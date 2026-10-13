//! Teevolution Terra Pro (ids 0xF520, 0xF523, 0xF5BB, 0xF522) - 16-byte reports on report id 8, ported from OpenMouse
//! (`crate::family`: AGPL-3.0-or-later): DPI, polling rate and battery.
//!
//! Ported from `teevolution/index.js` (the codec: packet checksum, read / write / online payloads, flash map, DPI and polling
//! encodings, the Terra Pro profile, the type -> maximum polling table) and `drivers/teevolution/hid.js` (TeevolutionHidClient:
//! readDeviceInfo, readStatus, setDpi, setPollingRate, withDeviceControl / setDeviceOnline, readFlash / writeFlash).
//!
//! Packet (OpenMouse sends `sendReport(8, 16 bytes)` and takes the first input report of id 8 whose byte 0 equals the command;
//! Windows puts the id 8 in front, so OpenMouse's byte i is our byte i + 1): `[0]` command (1 identify, 3 host control,
//! 4 battery, 7 write flash, 8 read flash) · `[1]` status in answers (0 = ok) · `[2..4]` flash address big-endian · `[4]` length
//! (1-10) · `[5..]` data · `[15]` checksum = `(0x55 - (8 + sum of bytes 0..14)) & 0xFF`.
//!
//! Flow (per OpenMouse): identify (command 1 with an 8-byte challenge whose first four bytes OpenMouse takes from the browser's
//! random generator; here the system clock - the answer's cid / mid / type are what is used: cid 14 = Terra Pro, any other cid
//! stops everything; type 2 / 3 = cable, else wireless, with the maximum polling rate 1000 / 4000 / 1000 / 8000 / 2000 / 8000
//! by type 0..5), then everything else inside "host control": command 3 with byte 5 = 1 (answer byte 5 = 1: the mouse is
//! online; answer byte 9 = 1: the receiver is busy - up to 20 tries, 10 ms apart) and, always at the end, command 3 with
//! byte 5 = 0. Flash map: `0` polling (`[value, 0x55 - value]`: 1000 / rate for rates up to 1000, rate / 2000 * 16 above),
//! `2` enabled DPI stages, `4` active stage, `12 + 4 * stage` the stage records. A DPI record is `[low, low, flags, 0x55 - low -
//! low - flags]`: `raw = dpi/50 - 1` (below 30100 DPI) or `dpi/100 - 1` with dpiEx 0x11 (from 30100 DPI, steps of 100, up to
//! 42000), low byte in `low`, the two high bits in `flags` bits 2-3 and 6-7, dpiEx in the low two bits of each nibble.
//!
//! Safety: a DPI change replaces the active stage's four bytes only, and only when re-encoding the stored value gives back
//! the stored bytes (else unknown flag bits would be wiped - refused, nothing written).
//!
//! Read-only proof: reading needs the host-control ENTRY (command 3, byte 5 = 1), which [`read_request_allowed`] refuses (as
//! pulsar.rs refuses host control entered); only the exit, the identify, the battery query and flash reads pass.
//!
//! Left out on purpose: lift-off (Low / Medium / High = raw 3 / 1 / 2, no millimetre figure), debounce, sleep, motion sync,
//! angle snapping, ripple, performance mode, sensor mode, DPI light, profile switch (command 15), button mapping, the LCD
//! clock of the RapidSync dongle (a different collection, report 10), firmware / RSSI / dongle-version reads, the DPI stage
//! count and active-stage editors. OpenMouse sends no firmware or reset command here and neither does this file.

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Driver, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x3554;
/// TEEVOLUTION_PRODUCT_IDS (the list names them all "Terra Pro").
pub const PRODUCT_IDS: [u16; 4] = [0xF520, 0xF523, 0xF5BB, 0xF522];
pub const MODEL: &str = "Teevolution Terra Pro";
pub const REPORT_ID: u8 = 0x08;
pub const PAYLOAD_LEN: usize = 16;
pub const REPORT_LEN: usize = PAYLOAD_LEN + 1;

pub const CMD_IDENTIFY: u8 = 0x01;
pub const CMD_ONLINE: u8 = 0x03;
pub const CMD_BATTERY: u8 = 0x04;
pub const CMD_WRITE: u8 = 0x07;
pub const CMD_READ: u8 = 0x08;
/// The Terra Pro's cid in the identification answer.
pub const CID_TERRA_PRO: u8 = 14;

/// TEEVOLUTION_FLASH.
pub const FLASH_REPORT_RATE: u16 = 0;
pub const FLASH_MAX_DPI_STAGE: u16 = 2;
pub const FLASH_CURRENT_DPI: u16 = 4;
pub const FLASH_DPI_VALUES: u16 = 12;
/// Terra Pro: four stage records.
pub const STAGE_COUNT: usize = 4;
pub const POLLING_RATES: [u32; 7] = [125, 250, 500, 1000, 2000, 4000, 8000];
/// teevolutionDpiOptions: 50..30000 in 50s, then 30100..42000 in 100s.
pub const DPI_BANDS: [(u32, u32, u32); 2] = [(50, 30000, 50), (30100, 42000, 100)];

const RESPONSE_TIMEOUT_MS: u32 = 1200;
const ONLINE_TRIES: u32 = 20;

pub type Payload = [u8; PAYLOAD_LEN];

// ---------------------------------------------------------------- packets

/// teevolutionPacketChecksum: `(0x55 - (sum of the first 15 bytes) - 8) & 0xFF`.
pub fn checksum(p: &Payload) -> u8 {
    let sum: u32 = p[..15].iter().map(|b| *b as u32).sum();
    0x55u8.wrapping_sub((sum & 0xFF) as u8).wrapping_sub(REPORT_ID)
}

pub fn checksum_is_valid(p: &[u8]) -> bool {
    p.len() == PAYLOAD_LEN && p[15] == checksum(p.try_into().expect("16 bytes"))
}

fn finish(mut p: Payload) -> Payload {
    p[15] = checksum(&p);
    p
}

/// teevolutionBuildSimplePayload.
pub fn simple(command: u8) -> Payload {
    let mut p = [0u8; PAYLOAD_LEN];
    p[0] = command;
    finish(p)
}

/// readDeviceInfo's query: command 1, length 8, the challenge (four bytes, then zeros) from byte 5.
pub fn identify_request(challenge: [u8; 4]) -> Payload {
    let mut p = simple(CMD_IDENTIFY);
    p[4] = 8;
    p[5..9].copy_from_slice(&challenge);
    finish(p)
}

/// teevolutionBuildOnlinePayload(enabled): command 3, byte 5 = 1 / 0.
pub fn online_request(enabled: bool) -> Payload {
    let mut p = simple(CMD_ONLINE);
    p[5] = enabled as u8;
    finish(p)
}

/// teevolutionBuildReadPayload(address, length).
pub fn read_request(address: u16, length: u8) -> Payload {
    let mut p = simple(CMD_READ);
    p[2] = (address >> 8) as u8;
    p[3] = address as u8;
    p[4] = length;
    finish(p)
}

/// teevolutionBuildWritePayload(address, data) (1-10 bytes).
pub fn write_request(address: u16, data: &[u8]) -> Payload {
    let mut p = simple(CMD_WRITE);
    p[2] = (address >> 8) as u8;
    p[3] = address as u8;
    p[4] = data.len() as u8;
    p[5..5 + data.len()].copy_from_slice(data);
    finish(p)
}

/// writeCheckedByte: `[value, 0x55 - value]` at `address`.
pub fn write_byte_request(address: u16, value: u8) -> Payload {
    write_request(address, &[value, 0x55u8.wrapping_sub(value)])
}

/// What Windows sends: the report id 8, the payload, zero-padded to the collection's output size.
pub fn wire(p: &Payload, len: usize) -> Vec<u8> {
    let mut v = vec![REPORT_ID];
    v.extend_from_slice(p);
    v.resize(len.max(REPORT_LEN), 0);
    v
}

// ---------------------------------------------------------------- codec

/// teevolutionDataChecksum: `(0x55 - sum) & 0xFF`.
fn data_checksum(d: &[u8]) -> u8 {
    0x55u8.wrapping_sub((d.iter().map(|b| *b as u32).sum::<u32>() & 0xFF) as u8)
}

/// teevolutionEncodePollingRate.
pub fn encode_polling(hz: u32) -> Option<u8> {
    match hz {
        125 => Some(8),
        250 => Some(4),
        500 => Some(2),
        1000 => Some(1),
        2000 => Some(16),
        4000 => Some(32),
        8000 => Some(64),
        _ => None,
    }
}

/// teevolutionDecodePollingRate (JS divides; only the codes the encoder makes are whole rates).
pub fn decode_polling(code: u8) -> Option<u32> {
    match code {
        8 => Some(125),
        4 => Some(250),
        2 => Some(500),
        1 => Some(1000),
        16 => Some(2000),
        32 => Some(4000),
        64 => Some(8000),
        _ => None,
    }
}

/// teevolutionEncodeDpi: the four-byte stage record.
pub fn encode_dpi(dpi: u32) -> Option<[u8; 4]> {
    let (ex, scaled) = if dpi >= 30100 { (0x11u32, dpi / 2) } else { (0, dpi) };
    if scaled < 50 || !scaled.is_multiple_of(50) {
        return None;
    }
    let raw = scaled / 50 - 1;
    if raw > 0x3FF {
        return None;
    }
    let high = raw >> 8;
    let flags = ((high << 2) | (high << 6) | ex | (ex << 4)) as u8;
    let low = raw as u8;
    Some([low, low, flags, data_checksum(&[low, low, flags])])
}

/// teevolutionDecodeDpi (no validation of the checksum, like OpenMouse).
pub fn decode_dpi(d: &[u8]) -> Option<u32> {
    if d.len() < 4 {
        return None;
    }
    let flags = d[2] as u32;
    let raw = d[0] as u32 + (((flags & 0x0C) >> 2) << 8);
    let ex = flags & 0x03;
    let mut dpi = (raw + 1) * 50;
    if ex & 1 != 0 {
        dpi *= 2;
    }
    if ex & 2 != 0 {
        dpi *= 2;
    }
    Some(dpi)
}

/// teevolutionParseReadResponse: the data behind the header when the answer is this read's.
pub fn parse_read(resp: &[u8], address: u16, length: u8) -> Option<Vec<u8>> {
    if !checksum_is_valid(resp) || resp[0] != CMD_READ || resp[1] != 0 {
        return None;
    }
    if resp[2] != (address >> 8) as u8 || resp[3] != address as u8 || resp[4] != length || resp.len() < 5 + length as usize {
        return None;
    }
    Some(resp[5..5 + length as usize].to_vec())
}

/// teevolutionParseBattery: (percent or unknown, charging); `None` = not a valid battery answer.
pub fn parse_battery(resp: &[u8]) -> Option<(Option<u8>, bool)> {
    if !checksum_is_valid(resp) || resp[0] != CMD_BATTERY || resp[1] != 0 {
        return None;
    }
    Some(((resp[5] <= 100).then_some(resp[5]), resp[6] == 1))
}

/// TEEVOLUTION_TYPE_MAX_POLL (a type outside the table: 1000).
pub fn max_polling(kind: u8) -> u32 {
    match kind {
        1 => 4000,
        3 | 5 => 8000,
        4 => 2000,
        _ => 1000,
    }
}

/// What readDeviceInfo takes from the identification answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub cid: u8,
    pub mid: u8,
    pub kind: u8,
    pub dongle_type: u8,
    pub wired: bool,
    pub max_polling_hz: u32,
}

/// readDeviceInfo's decode: status 0 required; `None` for the Terra Pro's cid only.
pub fn decode_identity(resp: &[u8]) -> Result<Info> {
    if resp.len() < 13 {
        return Err(Error::BadAnswer("a Teevolution identification answer shorter than 13 bytes".into()));
    }
    if resp[1] != 0 {
        return Err(Error::BadAnswer(format!("The Teevolution receiver rejected the identification (status {}).", resp[1])));
    }
    if resp[9] != CID_TERRA_PRO {
        return Err(Error::UnsupportedMouse(format!("Teevolution model (CID {:#04x}) - no settings were read or written", resp[9])));
    }
    let kind = resp[11];
    Ok(Info { cid: resp[9], mid: resp[10], kind, dongle_type: resp[12], wired: kind == 2 || kind == 3, max_polling_hz: max_polling(kind) })
}

pub fn caps(max_polling_hz: u32) -> Caps {
    Caps { dpi: Some(DpiCaps { bands: DPI_BANDS.to_vec() }), polling: POLLING_RATES.iter().copied().filter(|r| *r <= max_polling_hz).collect(), lift_off: Vec::new(), battery: true }
}

// ---------------------------------------------------------------- device table + allow-list

/// OpenMouse's isSupported: Compx's vendor id, a Terra Pro product id, and a collection whose only reports are 16-byte
/// report 8 in and out (Windows: 17-byte input and output reports). A vendor-defined collection is preferred.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID || !PRODUCT_IDS.contains(&pid) {
        return None;
    }
    let c = own.iter().filter(|h| h.input_len as usize == REPORT_LEN && h.output_len as usize == REPORT_LEN).min_by_key(|h| h.usage_page < 0xFF00)?;
    Some(Matched { family: Family::Teevolution, model: Some(MODEL), wireless: None, config: (*c).clone(), variant: pid })
}

/// The allow-list for the read-only proof on a real mouse: report id 8 carrying exactly the identify query, the battery
/// query, a flash read (1-10 bytes) or the host-control EXIT (command 3, byte 5 = 0), checksum right, zeros behind. The
/// host-control entry, flash writes and every other command are refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    if out.len() < REPORT_LEN || out[0] != REPORT_ID || out[REPORT_LEN..].iter().any(|b| *b != 0) {
        return false;
    }
    let p = &out[1..REPORT_LEN];
    match p[0] {
        CMD_IDENTIFY => p[4] == 8 && p[9..15].iter().all(|b| *b == 0) && p == identify_request([p[5], p[6], p[7], p[8]]),
        CMD_BATTERY => p == simple(CMD_BATTERY),
        CMD_ONLINE => p == online_request(false),
        CMD_READ => (1..=10).contains(&p[4]) && p == read_request(u16::from_be_bytes([p[2], p[3]]), p[4]),
        _ => false,
    }
}

// ---------------------------------------------------------------- the exchange

/// exchange: one payload → the first input report of id 8 whose byte 0 is the command (the OS layer skips the others).
fn transact(port: &mut dyn Port, p: &Payload) -> Result<Vec<u8>> {
    let out_len = port.info().output_len as usize;
    let in_len = (port.info().input_len as usize).max(REPORT_LEN);
    let how = HidTransfer::OutputThenInput { reply_len: in_len, max_reads: 16, timeout_ms: RESPONSE_TIMEOUT_MS, echo: Some((1, p[0])) };
    let a = port.exchange(&wire(p, out_len), &how)?;
    a.get(1..REPORT_LEN).map(<[u8]>::to_vec).ok_or_else(|| Error::BadAnswer("a Teevolution answer shorter than 16 bytes".into()))
}

fn optional<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::BadAnswer(_) | Error::Os { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// A challenge for the identification: four bytes from the clock (OpenMouse uses a random generator; the answer does not
/// depend on it for the fields used here).
fn challenge() -> [u8; 4] {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() ^ (d.as_secs() as u32)).unwrap_or(0x5A5A_A5A5);
    n.to_le_bytes()
}

fn identify(port: &mut dyn Port) -> Result<Info> {
    decode_identity(&transact(port, &identify_request(challenge()))?)
}

/// setDeviceOnline(enabled): the host-control command until the receiver is not busy. `Ok(true)` = the mouse is online
/// (answer byte 5 = 1; only asked for on entry).
fn set_online(port: &mut dyn Port, enabled: bool, tries: u32) -> Result<bool> {
    let mut resp = Vec::new();
    for _ in 0..tries.max(1) {
        resp = transact(port, &online_request(enabled))?;
        if resp[1] != 0 {
            return Err(Error::BadAnswer(format!("The Teevolution receiver rejected the host-control {} (status {}).", if enabled { "entry" } else { "exit" }, resp[1])));
        }
        if resp[9] != 1 {
            break;
        }
        port.pause_ms(10);
    }
    if resp[9] == 1 {
        return Err(Error::BadAnswer("The Teevolution receiver stayed busy.".into()));
    }
    Ok(resp[5] == 1)
}

/// withDeviceControl: enter host control, run `f`, always leave it again (a failed exit is ignored). `Ok(None)` = the
/// mouse is not online (OpenMouse throws "The Teevolution mouse is offline").
fn with_control<T>(port: &mut dyn Port, tries: u32, f: impl FnOnce(&mut dyn Port) -> Result<T>) -> Result<Option<T>> {
    if !set_online(port, true, tries)? {
        return Ok(None);
    }
    let r = f(port);
    let _ = set_online(port, false, tries);
    r.map(Some)
}

/// readFlash(address, length) for a length of at most 10 (one chunk).
fn read_flash(port: &mut dyn Port, address: u16, length: u8) -> Result<Vec<u8>> {
    let r = transact(port, &read_request(address, length))?;
    parse_read(&r, address, length).ok_or_else(|| {
        if r[1] != 0 {
            Error::BadAnswer(format!("The Teevolution receiver rejected the configuration read (status {}).", r[1]))
        } else {
            Error::BadAnswer("The Teevolution configuration read response was corrupt.".into())
        }
    })
}

/// writeFlash for one chunk (at most 10 bytes): the answer's status must be 0.
fn write_flash(port: &mut dyn Port, address: u16, data: &[u8]) -> Result<()> {
    let r = transact(port, &write_request(address, data))?;
    if r[1] != 0 {
        return Err(Error::BadAnswer(format!("The Teevolution receiver rejected the configuration write (status {}).", r[1])));
    }
    Ok(())
}

fn offline() -> Error {
    Error::BadAnswer("The Teevolution mouse is offline. Move it or click a button, then retry.".into())
}

/// The part of the flash this app reads: bytes `0..30` in three chunks of ten (OpenMouse reads `0..187` in chunks of ten from 0).
fn read_settings(port: &mut dyn Port) -> Result<Vec<u8>> {
    let mut f = Vec::new();
    for a in [0u16, 10, 20] {
        f.extend(read_flash(port, a, 10)?);
    }
    Ok(f)
}

/// The enabled stage count and the active stage of a flash image (decodeDpiStageCount; `min(currentDpi, count - 1)`).
fn stages(flash: &[u8]) -> (usize, usize) {
    let count = (flash[FLASH_MAX_DPI_STAGE as usize] as usize).clamp(1, STAGE_COUNT);
    (count, (flash[FLASH_CURRENT_DPI as usize] as usize).min(count - 1))
}

/// Inside host control: the flash image and the battery answer (decoded; no answer = no battery).
/// (percent or unknown, charging)
type BatteryReading = (Option<u8>, bool);

fn read_status_data(port: &mut dyn Port) -> Result<(Vec<u8>, Option<BatteryReading>)> {
    let flash = read_settings(port)?;
    let battery = optional(transact(port, &simple(CMD_BATTERY)))?.and_then(|b| parse_battery(&b));
    Ok((flash, battery))
}

// ---------------------------------------------------------------- the driver

/// The Teevolution driver (`Matched.variant` is the product id).
pub struct Teevolution;

impl Driver for Teevolution {
    /// readStatus: identify (no answer = offline), host control (mouse not online = offline), the flash image, the battery.
    fn read(&self, _m: &Matched, port: &mut dyn Port, opt: ReadOptions) -> Result<OnMouse> {
        let mut r = OnMouse { caps: caps(1000), ..Default::default() };
        let Some(info) = optional(identify(port))? else { return Ok(r) };
        r.wireless = Some(!info.wired);
        r.caps = caps(info.max_polling_hz);
        let tries = opt.online_tries.min(ONLINE_TRIES);
        // host control: a receiver that does not answer, stays busy or reports the mouse offline = offline
        if !optional(set_online(port, true, tries))?.unwrap_or(false) {
            return Ok(r);
        }
        let got = read_status_data(port);
        let _ = set_online(port, false, tries);
        let (flash, battery) = got?;
        r.online = true;
        let (_, active) = stages(&flash);
        let o = FLASH_DPI_VALUES as usize + active * 4;
        r.dpi = decode_dpi(&flash[o..o + 4]);
        r.polling_hz = decode_polling(flash[FLASH_REPORT_RATE as usize]);
        if let Some((pct, charging)) = battery {
            r.battery_percent = pct;
            r.charging = Some(charging);
        }
        Ok(r)
    }

    /// setDpi / setPollingRate inside host control: read, write, read back.
    fn write(&self, _m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let dpi_caps = caps(1000).dpi.unwrap_or_default();
        match s {
            Setting::Dpi(d) if !dpi_caps.allows(d) => return Err(Error::range("DPI", format!("{d} DPI is not one this Terra Pro can store (50 - 30000 in steps of 50, 30100 - 42000 in steps of 100)"))),
            Setting::Polling(hz) if encode_polling(hz).is_none() => return Err(Error::range("polling rate", format!("{hz} Hz is not a Terra Pro polling rate"))),
            Setting::LiftOff(_) => return Err(Error::range("lift-off distance", "this app can't set a Teevolution mouse's lift-off (its levels have no millimetre figure)".to_string())),
            _ => {}
        }
        let info = identify(port)?;
        if let Setting::Polling(hz) = s {
            if hz > info.max_polling_hz {
                return Err(Error::range("polling rate", format!("This connection supports at most {} Hz.", info.max_polling_hz)));
            }
        }
        let r = with_control(port, ONLINE_TRIES, |port| match s {
            Setting::Dpi(d) => {
                let count = (read_flash(port, FLASH_MAX_DPI_STAGE, 2)?[0] as usize).clamp(1, STAGE_COUNT);
                let active = (read_flash(port, FLASH_CURRENT_DPI, 2)?[0] as usize).min(count - 1);
                let address = FLASH_DPI_VALUES + active as u16 * 4;
                let stored = read_flash(port, address, 4)?;
                let before = decode_dpi(&stored);
                if before.and_then(encode_dpi).as_ref().map(|r| &r[..]) != Some(&stored[..]) {
                    return Err(Error::BadAnswer("the Terra Pro's DPI stage holds bytes OpenMouse's way of saving would change - nothing was written".into()));
                }
                write_flash(port, address, &encode_dpi(d).ok_or_else(|| Error::range("DPI", format!("{d} DPI cannot be encoded")))?)?;
                let now = decode_dpi(&read_flash(port, address, 4)?);
                if now != Some(d) {
                    return Err(Error::BadAnswer(format!("The mouse kept {now:?} DPI instead of {d} DPI.")));
                }
                Ok(Changed { now: d, before })
            }
            Setting::Polling(hz) => {
                let before = decode_polling(read_flash(port, FLASH_REPORT_RATE, 2)?[0]);
                write_flash(port, FLASH_REPORT_RATE, &[encode_polling(hz).unwrap_or(0), 0x55u8.wrapping_sub(encode_polling(hz).unwrap_or(0))])?;
                let now = decode_polling(read_flash(port, FLASH_REPORT_RATE, 2)?[0]);
                if now != Some(hz) {
                    return Err(Error::BadAnswer(format!("The mouse kept {now:?} Hz instead of {hz} Hz.")));
                }
                Ok(Changed { now: hz, before })
            }
            Setting::LiftOff(_) => unreachable!("refused above"),
        })?;
        r.ok_or_else(offline)
    }
}
