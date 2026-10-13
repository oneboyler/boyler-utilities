//! VGN Dragonfly F2 Master+ (receiver 0xFB56, cable 0xFB57) - 16-byte reports on report id 8, ported from OpenMouse
//! (`crate::family`: AGPL-3.0-or-later): DPI, polling rate and battery.
//!
//! Ported from `vgn/index.js` (the codec: checksum, read / write payloads, battery, the profile decode, DPI and polling
//! encodings) and `drivers/vgn/hid.js` (VgnF2HidClient: readStatus / readProfile / setDpi / setPollingRate / writeScalar).
//!
//! Packet (OpenMouse sends `sendReport(8, 16 bytes)` and takes the first input report of id 8 whose byte 0 equals the command;
//! Windows puts the id 8 in front, so OpenMouse's byte i is our byte i + 1): `[0]` command (4 battery, 7 write flash, 8 read
//! flash) · `[1]` status in answers (0 = ok) · `[2..4]` flash address big-endian · `[4]` length (1-10) · `[5..]` data ·
//! `[15]` checksum = `(0x55 - (8 + sum of bytes 0..14)) & 0xFF`.
//! The settings live in the mouse's flash; the profile is read in 10-byte chunks. Scalars (polling rate) are stored as
//! `[value, 0x55 - value]`; a DPI stage is four bytes `[low, low, flags, 0x55 - low - low - flags]` with `low = dpi/50 - 1` (low
//! byte) and its two high bits in `flags` bits 2-3 and 6-7. The active stage's record is at `0x0C + 4 * active stage`.
//!
//! Differences from OpenMouse, deliberate: an unreadable DPI record or polling code is reported as unknown (OpenMouse shows
//! 800 DPI / 1000 Hz then); a DPI change is only written when re-encoding the stored value gives back the stored four bytes
//! (otherwise unknown flag bits would be wiped - refused, nothing written). The profile is OpenMouse's readProfile: flash
//! `0..0x2C` and `0xA8..0xB7`; the firmware and key-table reads are left out.
//!
//! Order 110 (A_110_02 scope): debounce (0-30 ms, setDebounceTime), Motion Sync, angle snapping, ripple control and the
//! performance mode (flash 0xB5, the Pulsar cMouse's turbo address: [`Extra::Turbo`]) - each OpenMouse's writeScalar: the
//! write, then its two bytes read back (value and parity).
//!
//! Left out on purpose: lift-off (Low / Medium / High = raw 3 / 1 / 2, no millimetre figure), sleep, button mapping, firmware
//! reads. OpenMouse sends no firmware or reset command to this family and neither does this file.

use crate::error::{Error, Result};
use crate::family::{check_extra, Caps, Change, Changed, DpiCaps, Driver, Extra, ExtraRow, Family, Kind, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x3554;
/// The 2.4 GHz receiver ("wireless path").
pub const PID_RECEIVER: u16 = 0xFB56;
pub const PID_WIRED: u16 = 0xFB57;
pub const MODEL: &str = "VGN Dragonfly F2 Master+";
pub const REPORT_ID: u8 = 0x08;
pub const PAYLOAD_LEN: usize = 16;
/// Windows: the report id + the payload.
pub const REPORT_LEN: usize = PAYLOAD_LEN + 1;
pub const USAGE_PAGE: u16 = 0xFF02;

pub const CMD_BATTERY: u8 = 0x04;
pub const CMD_WRITE: u8 = 0x07;
pub const CMD_READ: u8 = 0x08;
/// VGN_ADDRESS.
pub const ADDR_POLLING: u16 = 0x00;
pub const ADDR_STAGE_COUNT: u16 = 0x02;
pub const ADDR_ACTIVE_STAGE: u16 = 0x04;
pub const ADDR_DPI_STAGES: u16 = 0x0C;
pub const ADDR_DEBOUNCE: u16 = 0xA9;
pub const ADDR_MOTION_SYNC: u16 = 0xAB;
pub const ADDR_ANGLE_SNAP: u16 = 0xAF;
pub const ADDR_RIPPLE: u16 = 0xB1;
/// performanceMode (the Pulsar cMouse's turbo address)
pub const ADDR_PERFORMANCE: u16 = 0xB5;
/// setDebounceTime: 0-30 ms.
pub const DEBOUNCE_MAX_MS: i32 = 30;
pub const DPI_MIN: u32 = 50;
pub const DPI_MAX: u32 = 26000;
pub const DPI_STEP: u32 = 50;
/// SUPPORTED_POLLING_RATES.
pub const RATES_HZ: [u32; 7] = [125, 250, 500, 1000, 2000, 4000, 8000];
const TIMEOUT_MS: u32 = 700;
/// readProfile's chunks: (address, length) - `0..0x2C` and `0xA8..0xB7`.
pub const PROFILE_CHUNKS: [(u16, u8); 7] = [(0x00, 10), (0x0A, 10), (0x14, 10), (0x1E, 10), (0x28, 4), (0xA8, 10), (0xB2, 5)];
/// The DPI part of the profile (decode_profile needs at least this much).
const PROFILE_LEN: usize = 0x2C;
/// What readProfile fills (`new Uint8Array(0xb7)`).
const PROFILE_FULL_LEN: usize = 0xB7;

// ---------------------------------------------------------------- packets

pub type Payload = [u8; PAYLOAD_LEN];

/// vgnReportChecksum: `(0x55 - (8 + sum of the first 15 bytes)) & 0xFF`.
pub fn checksum(p: &Payload) -> u8 {
    let sum: u32 = REPORT_ID as u32 + p[..15].iter().map(|b| *b as u32).sum::<u32>();
    0x55u8.wrapping_sub((sum & 0xFF) as u8)
}

pub fn checksum_is_valid(p: &[u8]) -> bool {
    p.len() == PAYLOAD_LEN && (REPORT_ID as u32 + p.iter().map(|b| *b as u32).sum::<u32>()) & 0xFF == 0x55
}

fn finish(mut p: Payload) -> Payload {
    p[15] = checksum(&p);
    p
}

/// vgnBuildSimplePayload(command).
pub fn simple(command: u8) -> Payload {
    let mut p = [0u8; PAYLOAD_LEN];
    p[0] = command;
    finish(p)
}

/// vgnBuildReadPayload(address, length).
pub fn read_payload(address: u16, length: u8) -> Payload {
    let mut p = simple(CMD_READ);
    p[2] = (address >> 8) as u8;
    p[3] = address as u8;
    p[4] = length;
    finish(p)
}

/// vgnBuildWritePayload(address, data) (1-10 bytes).
pub fn write_payload(address: u16, data: &[u8]) -> Payload {
    let mut p = simple(CMD_WRITE);
    p[2] = (address >> 8) as u8;
    p[3] = address as u8;
    p[4] = data.len() as u8;
    p[5..5 + data.len()].copy_from_slice(data);
    finish(p)
}

/// vgnBuildWriteScalarPayload(address, value): the two bytes `[value, 0x55 - value]`.
pub fn write_scalar_payload(address: u16, value: u8) -> Payload {
    write_payload(address, &[value, 0x55u8.wrapping_sub(value)])
}

/// What Windows sends: the report id 8, the payload, zero-padded to the collection's output size.
pub fn wire(p: &Payload, len: usize) -> Vec<u8> {
    let mut v = vec![REPORT_ID];
    v.extend_from_slice(p);
    v.resize(len.max(REPORT_LEN), 0);
    v
}

// ---------------------------------------------------------------- answers

/// vgnParseReadResponse: the data behind the header when the answer is this read's.
pub fn parse_read(resp: &[u8], address: u16, length: u8) -> Option<Vec<u8>> {
    if !checksum_is_valid(resp) || resp[0] != CMD_READ || resp[1] != 0 {
        return None;
    }
    if resp[2] != (address >> 8) as u8 || resp[3] != address as u8 || resp[4] != length || resp.len() < 5 + length as usize {
        return None;
    }
    Some(resp[5..5 + length as usize].to_vec())
}

/// vgnParseBattery: (percent, charging). `None` = not a valid battery answer or over 100.
pub fn parse_battery(resp: &[u8]) -> Option<(u8, bool)> {
    if !checksum_is_valid(resp) || resp[0] != CMD_BATTERY || resp[1] != 0 || resp[5] > 100 {
        return None;
    }
    Some((resp[5], resp[6] == 1))
}

/// Is this a valid, successful answer to a write (vgnReportChecksumIsValid, command 7, status 0)?
pub fn write_ok(resp: &[u8]) -> bool {
    checksum_is_valid(resp) && resp[0] == CMD_WRITE && resp[1] == 0
}

/// vgnEncodePollingRate.
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

/// vgnDecodePollingRate.
pub fn decode_polling(raw: u8) -> Option<u32> {
    match raw {
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

/// vgnEncodeDpi: the stage record `[low, low, flags, 0x55 - low - low - flags]`.
pub fn encode_dpi(dpi: u32) -> Option<[u8; 4]> {
    if !(DPI_MIN..=DPI_MAX).contains(&dpi) || !dpi.is_multiple_of(DPI_STEP) {
        return None;
    }
    let enc = dpi / DPI_STEP - 1;
    let low = (enc & 0xFF) as u8;
    let high = ((enc >> 8) & 0x03) as u8;
    let flags = (high << 2) | (high << 6);
    Some([low, low, flags, 0x55u8.wrapping_sub(low).wrapping_sub(low).wrapping_sub(flags)])
}

/// vgnDecodeDpi: `None` when the two low bytes differ or the checksum is wrong.
pub fn decode_dpi(stage: &[u8]) -> Option<u32> {
    if stage.len() < 4 || stage[0] != stage[1] || stage[0].wrapping_add(stage[1]).wrapping_add(stage[2]).wrapping_add(stage[3]) != 0x55 {
        return None;
    }
    Some(((((stage[2] >> 2) & 0x03) as u32) << 8 | stage[0] as u32).wrapping_add(1) * 50)
}

/// vgnUnpackScalar.
pub fn unpack_scalar(value: u8, parity: u8) -> Option<u8> {
    (value.wrapping_add(parity) == 0x55).then_some(value)
}

/// What the profile (flash `0..0x2C`) says about DPI and polling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    pub active_stage: usize,
    pub stage_count: usize,
    /// the active stage's four stored bytes
    pub stage_bytes: [u8; 4],
    pub dpi: Option<u32>,
    pub polling_hz: Option<u32>,
    /// both the stage count and the active stage passed their parity check, the count is 1..=8 and the active stage is below it:
    /// `active_stage` is then the stage the mouse really uses (else it is OpenMouse's clamped / defaulted guess)
    pub stage_certain: bool,
}

/// vgnDecodeProfile for the fields this app uses.
pub fn decode_profile(profile: &[u8]) -> Option<Profile> {
    if profile.len() < PROFILE_LEN {
        return None;
    }
    let scalar = |a: u16| unpack_scalar(profile[a as usize], profile[a as usize + 1]);
    let (raw_count, raw_active) = (scalar(ADDR_STAGE_COUNT), scalar(ADDR_ACTIVE_STAGE));
    let count = (raw_count.unwrap_or(1) as usize).clamp(1, 8);
    let active = (raw_active.unwrap_or(0) as usize).min(count - 1);
    let stage_certain = matches!((raw_count, raw_active), (Some(c), Some(a)) if (1..=8).contains(&c) && a < c);
    let off = ADDR_DPI_STAGES as usize + active * 4;
    let mut stage_bytes = [0u8; 4];
    stage_bytes.copy_from_slice(&profile[off..off + 4]);
    Some(Profile { active_stage: active, stage_count: count, stage_bytes, dpi: decode_dpi(&stage_bytes), polling_hz: scalar(ADDR_POLLING).and_then(decode_polling), stage_certain })
}

/// vgnDecodeProfile's debounceMs and switches (each a scalar whose parity must hold, else no value; switches `!= 0`), in
/// the card's order.
pub fn extras_from_profile(profile: &[u8]) -> Vec<ExtraRow> {
    let scalar = |a: u16| profile.get(a as usize..a as usize + 2).and_then(|b| unpack_scalar(b[0], b[1]));
    let on = |a: u16| scalar(a).map(|v| v != 0);
    vec![
        ExtraRow::new(Extra::Debounce, Kind::Range { min: 0, max: DEBOUNCE_MAX_MS, step: 1, unit: "ms" }, scalar(ADDR_DEBOUNCE).map(i32::from)),
        ExtraRow::switch(Extra::MotionSync, on(ADDR_MOTION_SYNC)),
        ExtraRow::switch(Extra::AngleSnap, on(ADDR_ANGLE_SNAP)),
        ExtraRow::switch(Extra::RippleControl, on(ADDR_RIPPLE)),
        ExtraRow::switch(Extra::Turbo, on(ADDR_PERFORMANCE)),
    ]
}

fn extra_addr(e: Extra) -> u16 {
    match e {
        Extra::Debounce => ADDR_DEBOUNCE,
        Extra::MotionSync => ADDR_MOTION_SYNC,
        Extra::AngleSnap => ADDR_ANGLE_SNAP,
        Extra::RippleControl => ADDR_RIPPLE,
        Extra::Turbo => ADDR_PERFORMANCE,
    }
}

// ---------------------------------------------------------------- the device table + allow-list

/// OpenMouse's isSupported: Compx's vendor id, the receiver or cable id, and a collection on usage page 0xFF02 with a 16-byte
/// report 8 both ways (a collection that also carries longer reports is fine: report 8 is only 16 bytes of it).
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID || (pid != PID_RECEIVER && pid != PID_WIRED) {
        return None;
    }
    let c = own.iter().find(|h| h.usage_page == USAGE_PAGE && h.input_len as usize >= REPORT_LEN && h.output_len as usize >= REPORT_LEN)?;
    Some(Matched { family: Family::Vgn, model: Some(MODEL), wireless: Some(pid == PID_RECEIVER), config: (*c).clone(), variant: pid })
}

/// The allow-list for the read-only proof on a real mouse: report id 8 carrying exactly the battery query or a flash read
/// (command 8, 1-10 bytes, checksum right), zeros behind. Flash writes (command 7) and every other command are refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    if out.len() < REPORT_LEN || out[0] != REPORT_ID || out[REPORT_LEN..].iter().any(|b| *b != 0) {
        return false;
    }
    let p = &out[1..REPORT_LEN];
    match p[0] {
        CMD_BATTERY => p == simple(CMD_BATTERY),
        CMD_READ => (1..=10).contains(&p[4]) && p == read_payload(u16::from_be_bytes([p[2], p[3]]), p[4]),
        _ => false,
    }
}

pub fn caps() -> Caps {
    Caps { dpi: Some(DpiCaps::uniform(DPI_MIN, DPI_MAX, DPI_STEP)), polling: RATES_HZ.to_vec(), lift_off: Vec::new(), battery: true }
}

// ---------------------------------------------------------------- the exchange

/// transact: one payload → the first input report of id 8 whose byte 0 is the command (the OS layer skips the others), 16 bytes.
fn transact(port: &mut dyn Port, p: &Payload) -> Result<Vec<u8>> {
    let out_len = port.info().output_len as usize;
    let in_len = (port.info().input_len as usize).max(REPORT_LEN);
    let how = HidTransfer::OutputThenInput { reply_len: in_len, max_reads: 16, timeout_ms: TIMEOUT_MS, echo: Some((1, p[0])) };
    let a = port.exchange(&wire(p, out_len), &how)?;
    a.get(1..REPORT_LEN).map(<[u8]>::to_vec).ok_or_else(|| Error::BadAnswer("a VGN answer shorter than 16 bytes".into()))
}

fn optional<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::BadAnswer(_) | Error::Os { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

fn read_flash(port: &mut dyn Port, address: u16, length: u8) -> Result<Vec<u8>> {
    let r = transact(port, &read_payload(address, length))?;
    parse_read(&r, address, length).ok_or_else(|| Error::BadAnswer(format!("VGN flash read failed at {address:#x}")))
}

/// readProfile (the used range), the receiver paced 10 ms between reads like OpenMouse's readPaced.
fn read_profile(port: &mut dyn Port, wireless: bool) -> Result<Vec<u8>> {
    let mut profile = vec![0u8; PROFILE_FULL_LEN];
    for (address, length) in PROFILE_CHUNKS {
        let d = read_flash(port, address, length)?;
        profile[address as usize..address as usize + length as usize].copy_from_slice(&d);
        if wireless {
            port.pause_ms(10);
        }
    }
    Ok(profile)
}

fn write_flash(port: &mut dyn Port, address: u16, data: &[u8]) -> Result<()> {
    let r = transact(port, &write_payload(address, data))?;
    if write_ok(&r) {
        Ok(())
    } else {
        Err(Error::BadAnswer(format!("VGN flash write failed at {address:#x}")))
    }
}

// ---------------------------------------------------------------- the driver

/// The VGN driver (`Matched.variant` is the product id; the receiver is the wireless path).
pub struct Vgn;

impl Driver for Vgn {
    /// readStatus' battery query (no answer = offline) and the profile read.
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let wireless = m.variant == PID_RECEIVER;
        let mut r = OnMouse { wireless: Some(wireless), caps: caps(), ..Default::default() };
        let Some(b) = optional(transact(port, &simple(CMD_BATTERY)))? else { return Ok(r) };
        r.online = true;
        if let Some((pct, charging)) = parse_battery(&b) {
            r.battery_percent = Some(pct);
            r.charging = Some(charging);
        }
        let profile = read_profile(port, wireless)?;
        let p = decode_profile(&profile).ok_or_else(|| Error::BadAnswer("the VGN profile was too short".into()))?;
        r.dpi = p.dpi;
        r.polling_hz = p.polling_hz;
        r.extras = extras_from_profile(&profile);
        Ok(r)
    }

    /// Order 110: setDebounceTime / setMotionSync / setAngleSnapping / setRippleControl / setPerformanceMode - checked before a
    /// byte leaves, then writeScalar: the write, its two bytes read back (value, parity).
    fn change(&self, _m: &Matched, port: &mut dyn Port, c: Change) -> Result<()> {
        let Change::Extra(e, v) = c;
        check_extra(&extras_from_profile(&[]), e, v)?;
        let addr = extra_addr(e);
        let r = transact(port, &write_scalar_payload(addr, v as u8))?;
        if !write_ok(&r) {
            return Err(Error::BadAnswer(format!("VGN flash write failed at {addr:#x}")));
        }
        let back = read_flash(port, addr, 2)?;
        if back[0] as i32 != v || back[0].wrapping_add(back[1]) != 0x55 {
            return Err(Error::BadAnswer(format!("the VGN mouse did not retain the value written at {addr:#x} ({})", e.label())));
        }
        Ok(())
    }

    /// setDpi / setPollingRate: the profile read, the flash write, the read-back.
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let wireless = m.variant == PID_RECEIVER;
        fn cur(port: &mut dyn Port, wireless: bool) -> Result<Profile> {
            decode_profile(&read_profile(port, wireless)?).ok_or_else(|| Error::BadAnswer("the VGN profile was too short".into()))
        }
        match s {
            Setting::Dpi(d) => {
                let rec = encode_dpi(d).ok_or_else(|| Error::range("DPI", format!("{d} DPI is not one this VGN mouse can store (50 - 26000 in steps of 50)")))?;
                let p = cur(port, wireless)?;
                // OpenMouse clamps a bad stage count / active stage (and defaults to stage 0 when a parity byte is wrong); the
                // write would then change a stage the mouse is not on and still read back fine
                if !p.stage_certain {
                    return Err(Error::BadAnswer("the VGN mouse's active DPI stage is unreadable or out of range - nothing was written".into()));
                }
                // the stored record must be exactly what OpenMouse's encoding gives for its value, or the write would wipe bits
                let stored = p.dpi.and_then(encode_dpi);
                if stored != Some(p.stage_bytes) {
                    return Err(Error::BadAnswer("the VGN mouse's DPI stage holds bytes OpenMouse's way of saving would change - nothing was written".into()));
                }
                write_flash(port, ADDR_DPI_STAGES + p.active_stage as u16 * 4, &rec)?;
                let now = cur(port, wireless)?.dpi;
                if now != Some(d) {
                    return Err(Error::BadAnswer(format!("the VGN mouse kept {now:?} DPI instead of {d}")));
                }
                Ok(Changed { now: d, before: p.dpi })
            }
            Setting::Polling(hz) => {
                let raw = encode_polling(hz).ok_or_else(|| Error::range("polling rate", format!("{hz} Hz is not a VGN polling rate")))?;
                let before = cur(port, wireless)?.polling_hz;
                // writeScalar: the write, then its two bytes read back
                let r = transact(port, &write_scalar_payload(ADDR_POLLING, raw))?;
                if !write_ok(&r) {
                    return Err(Error::BadAnswer("VGN flash write failed at 0".into()));
                }
                let c = read_flash(port, ADDR_POLLING, 2)?;
                if c[0] != raw || c[0].wrapping_add(c[1]) != 0x55 {
                    return Err(Error::BadAnswer("the VGN mouse did not retain the polling rate".into()));
                }
                let now = cur(port, wireless)?.polling_hz;
                if now != Some(hz) {
                    return Err(Error::BadAnswer(format!("the VGN mouse kept {now:?} Hz instead of {hz} Hz")));
                }
                Ok(Changed { now: hz, before })
            }
            Setting::LiftOff(_) => Err(Error::range("lift-off distance", "this app can't set a VGN mouse's lift-off (its levels have no millimetre figure)".to_string())),
        }
    }
}
