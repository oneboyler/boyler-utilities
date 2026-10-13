//! WALLHACK M-001 (ids 0x1110 and 0x0807) - 63-byte reports on report id 4, ported from OpenMouse (`crate::family`:
//! AGPL-3.0-or-later): DPI, polling rate, lift-off and battery. The keyboard client (`keyboard-hid.js`, K-001) is not a mouse
//! and is not ported.
//!
//! Ported from `wallhack/index.js` (the codec: packet builders, the config "function area" map, polling ranks, lift-off codes,
//! battery decode) and `drivers/wallhack/mouse-hid.js` (WallhackMouseHidClient: readStatus / readByte / readDpi / setDpi /
//! setPollingRate / setLiftOffDistance / writeVerifiedByte).
//!
//! Packet (OpenMouse sends `sendReport(4, 63 bytes)`; Windows puts the id 4 in front, so OpenMouse's byte i is our byte i + 1):
//! simple command `[0, 0, cmd]` · function-area read `[0, 0, 0xA4, n, addrLo, addrHi, 0]` · write `[0, 0, 0xA5, n, addrLo,
//! addrHi, 0, data...]`; no checksum. The answer is the first input report whose byte 2 equals the command; for a read the
//! data starts at byte 7. Battery (command 0xBA): percent at byte 7, charging at byte 8 (OpenMouse's own comment: "best-effort
//! from the obfuscated bundle and wants hardware confirmation").
//! Function area: `10` wireless polling rank · `11` wired polling rank · `12` active DPI stage · `77` the stage records (9 bytes
//! `[enabled, 0, dpiLo, dpiHi, 0x90, 1, 0xFF, 0xFF, 0]` - OpenMouse reads and writes only the FIRST record) · `110` lift-off code
//! (0, 1, 2 = 0.7, 1, 2 mm per the codec's `WALLHACK_LOD_MM_BY_CODE`). Polling rank r: 125, 250, 500, 1000, then 1500 ... 8000
//! in steps of 500 (18 rates). A polling change writes BOTH rank bytes (10 and 11) - only when they read the same - then reads both back. A write has no
//! acknowledgement in OpenMouse (`sendReport` only); the value is confirmed by reading it again.
//!
//! Differences from OpenMouse, deliberate and safe: the DPI is read from the first stage record, so it is shown (and
//! changed) only while stage 1 is the active one (OpenMouse shows it whatever the active stage is); a DPI change is only
//! written when the stored record equals what OpenMouse would write apart from the DPI itself (else the stage's LED colour
//! bytes would be overwritten with OpenMouse's defaults - refused, nothing written). DPI is offered in steps of 50 (OpenMouse's
//! driver takes any whole number 50-26000; what the sensor stores is unclear).
//!
//! Unclear: the report id the answers arrive with (taken to be 4 like the request; if the real mouse answers on another id, every
//! read times out and the mouse shows as not answering).
//!
//! Order 110 (A_110_02 scope): debounce (area 109, 0-20 ms), angle snapping (111), ripple (112), Motion Sync (113) - OpenMouse's
//! writeVerifiedByte: the one-byte area write, then the byte read back; a read that is not answered leaves its row out.
//!
//! Left out: firmware versions, the settings OpenMouse also reads
//! (sleep, game mode, angle tuning, profile, dynamic sensitivity), button
//! bindings, macros, curves, pairing, `clearPairing`, `factoryReset` (never sent).

use crate::error::{Error, Result};
use crate::family::{check_extra, Caps, Change, Changed, DpiCaps, Driver, Extra, ExtraRow, Family, Kind, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x3879;
/// M-001: the real config id and the in-app demo id.
pub const PRODUCT_IDS: [u16; 2] = [0x1110, 0x0807];
pub const MODEL: &str = "WALLHACK M-001";
pub const REPORT_ID: u8 = 4;
pub const PAYLOAD_LEN: usize = 63;
/// Windows: the report id + the payload.
pub const REPORT_LEN: usize = PAYLOAD_LEN + 1;
pub const USAGE_PAGE: u16 = 0xFF1C;

/// Order 110: WALLHACK_FLASH keyDebounceTime / angleSnapEnable / rippleControlEnable / motionSyncEnable.
pub const AREA_DEBOUNCE: u16 = 109;
pub const AREA_ANGLE_SNAP: u16 = 111;
pub const AREA_RIPPLE: u16 = 112;
pub const AREA_MOTION_SYNC: u16 = 113;
/// setDebounceTime: 0-20 ms.
pub const DEBOUNCE_MAX_MS: i32 = 20;

pub const CMD_READ_AREA: u8 = 0xA4;
pub const CMD_WRITE_AREA: u8 = 0xA5;
pub const CMD_BATTERY: u8 = 0xBA;
/// WALLHACK_FLASH.
pub const AREA_REPORT_ESB: u16 = 10;
pub const AREA_REPORT_USB: u16 = 11;
pub const AREA_DPI_RANK: u16 = 12;
pub const AREA_DPI_BLOCK: u16 = 77;
pub const AREA_SILENT_HEIGHT: u16 = 110;
pub const DPI_MIN: u32 = 50;
pub const DPI_MAX: u32 = 26000;
pub const DPI_STEP: u32 = 50;
/// Lift-off codes 0, 1, 2 in tenths of a millimetre (0.7 / 1 / 2 mm).
pub const LIFT_OFF: [u32; 3] = [7, 10, 20];
const RESPONSE_TIMEOUT_MS: u32 = 1000;

pub type Payload = [u8; PAYLOAD_LEN];

// ---------------------------------------------------------------- packets

/// wallhackBuildSimple(command).
pub fn simple(command: u8) -> Payload {
    let mut p = [0u8; PAYLOAD_LEN];
    p[2] = command;
    p
}

fn area(command: u8, address: u16, n: u8, data: &[u8]) -> Payload {
    let mut p = simple(command);
    p[3] = n;
    p[4] = address as u8;
    p[5] = (address >> 8) as u8;
    p[7..7 + data.len()].copy_from_slice(data);
    p
}

/// wallhackBuildRead(address, count).
pub fn read_request(address: u16, count: u8) -> Payload {
    area(CMD_READ_AREA, address, count, &[])
}

/// wallhackBuildWrite(address, payload).
pub fn write_request(address: u16, data: &[u8]) -> Payload {
    area(CMD_WRITE_AREA, address, data.len() as u8, data)
}

/// wallhackBuildSetDpiStage(dpi): the first stage record `[1, 0, lo, hi, 0x90, 1, 0xFF, 0xFF, 0]`.
pub fn set_dpi_request(dpi: u32) -> Payload {
    write_request(AREA_DPI_BLOCK, &[1, 0, dpi as u8, (dpi >> 8) as u8, 0x90, 1, 0xFF, 0xFF, 0])
}

/// What Windows sends: the report id 4, the payload, zero-padded to the collection's output size.
pub fn wire(p: &Payload, len: usize) -> Vec<u8> {
    let mut v = vec![REPORT_ID];
    v.extend_from_slice(p);
    v.resize(len.max(REPORT_LEN), 0);
    v
}

// ---------------------------------------------------------------- answers

/// wallhackReadByte: byte 7 of the answer.
pub fn read_byte(resp: &[u8]) -> Option<u8> {
    resp.get(7).copied()
}

/// wallhackReadDpi: bytes 9-10 of a 9-byte block read.
pub fn read_dpi(resp: &[u8]) -> Option<u32> {
    (resp.len() >= 11).then(|| u16::from_le_bytes([resp[9], resp[10]]) as u32)
}

/// The 9 stage-record bytes of a block read (answer bytes 7..16).
pub fn block_bytes(resp: &[u8]) -> Option<[u8; 9]> {
    resp.get(7..16)?.try_into().ok()
}

/// wallhackDecodeBattery: (percent or unknown, charging); needs 9 bytes.
pub fn decode_battery(resp: &[u8]) -> Option<(Option<u8>, bool)> {
    (resp.len() >= 9).then(|| ((resp[7] <= 100).then_some(resp[7]), resp[8] == 1))
}

/// WALLHACK_POLLING_BY_RANK: 0 → 125, 1 → 250, 2 → 500, 3 → 1000, then 500 Hz steps up to rank 17 = 8000.
pub fn rank_to_hz(rank: u8) -> Option<u32> {
    match rank {
        0 => Some(125),
        1 => Some(250),
        2 => Some(500),
        3..=17 => Some(1000 + (rank as u32 - 3) * 500),
        _ => None,
    }
}

pub fn hz_to_rank(hz: u32) -> Option<u8> {
    (0..=17u8).find(|r| rank_to_hz(*r) == Some(hz))
}

/// Every polling rate (jm), ascending.
pub fn rates() -> Vec<u32> {
    (0..=17u8).filter_map(rank_to_hz).collect()
}

/// wallhackLodFromCode in tenths of a millimetre.
pub fn lift_off_tenths(code: u8) -> Option<u32> {
    LIFT_OFF.get(code as usize).copied()
}

pub fn lift_off_code(tenths: u32) -> Option<u8> {
    LIFT_OFF.iter().position(|t| *t == tenths).map(|i| i as u8)
}

pub fn caps() -> Caps {
    Caps { dpi: Some(DpiCaps::uniform(DPI_MIN, DPI_MAX, DPI_STEP)), polling: rates(), lift_off: LIFT_OFF.to_vec(), battery: true }
}

// ---------------------------------------------------------------- device table + allow-list

/// OpenMouse's isSupported: WALLHACK's vendor id, an M-001 product id, and a collection on usage page 0xFF1C (any usage - the
/// app itself matches on the page alone) that carries 63-byte reports both ways.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID || !PRODUCT_IDS.contains(&pid) {
        return None;
    }
    let c = own.iter().find(|h| h.usage_page == USAGE_PAGE && h.input_len as usize >= REPORT_LEN && h.output_len as usize >= REPORT_LEN)?;
    Some(Matched { family: Family::Wallhack, model: Some(MODEL), wireless: None, config: (*c).clone(), variant: pid })
}

/// The allow-list for the read-only proof on a real mouse: report id 4 carrying exactly the battery query or a function-area
/// READ (command 0xA4, 1-16 bytes), zeros behind. Writes (0xA5), pairing, factory reset and every other command are refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    if out.len() < REPORT_LEN || out[0] != REPORT_ID || out[REPORT_LEN..].iter().any(|b| *b != 0) {
        return false;
    }
    let p = &out[1..REPORT_LEN];
    match p[2] {
        CMD_BATTERY => p == simple(CMD_BATTERY),
        CMD_READ_AREA => (1..=16).contains(&p[3]) && p == read_request(u16::from_le_bytes([p[4], p[5]]), p[3]),
        _ => false,
    }
}

// ---------------------------------------------------------------- the exchange

/// exchange: one payload → the first input report whose byte 2 is `expect` (the OS layer skips the others), without the
/// report id.
fn exchange(port: &mut dyn Port, p: &Payload, expect: u8) -> Result<Vec<u8>> {
    let out_len = port.info().output_len as usize;
    let in_len = (port.info().input_len as usize).max(REPORT_LEN);
    let how = HidTransfer::OutputThenInput { reply_len: in_len, max_reads: 32, timeout_ms: RESPONSE_TIMEOUT_MS, echo: Some((3, expect)) };
    let a = port.exchange(&wire(p, out_len), &how)?;
    a.get(1..).map(<[u8]>::to_vec).ok_or_else(|| Error::BadAnswer("an empty WALLHACK answer".into()))
}

/// send: a request with no answer (OpenMouse's `send`).
fn send(port: &mut dyn Port, p: &Payload) -> Result<()> {
    let out_len = port.info().output_len as usize;
    port.exchange(&wire(p, out_len), &HidTransfer::OutputOnly).map(|_| ())
}

/// `.catch(() => null)`: no answer = `None`; a mouse that is gone still fails.
fn optional<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::BadAnswer(_) | Error::Os { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// A function-area read whose answer must echo the asked count (byte 3) and address (bytes 4-5, little-endian), as OpenMouse's
/// readCurve checks; the first 0xA4 answer of another read is a bad answer, not data.
fn area_read(port: &mut dyn Port, address: u16, count: u8) -> Result<Vec<u8>> {
    let r = exchange(port, &read_request(address, count), CMD_READ_AREA)?;
    if r.len() < 7 || r[3] != count || r[4..6] != address.to_le_bytes() {
        return Err(Error::BadAnswer(format!("the WALLHACK mouse answered the read of area {address} with another area's data")));
    }
    Ok(r)
}

/// readByte(address): `None` = no answer (or one too short or for another address).
fn byte_at(port: &mut dyn Port, address: u16) -> Result<Option<u8>> {
    Ok(optional(area_read(port, address, 1))?.and_then(|r| read_byte(&r)))
}

/// The first stage record (9 bytes) as a block read returns it.
fn dpi_block(port: &mut dyn Port) -> Result<Option<Vec<u8>>> {
    optional(area_read(port, AREA_DPI_BLOCK, 9))
}

/// Order 110: the function-area byte of an extra.
pub fn extra_area(e: Extra) -> Option<u16> {
    Some(match e {
        Extra::Debounce => AREA_DEBOUNCE,
        Extra::MotionSync => AREA_MOTION_SYNC,
        Extra::AngleSnap => AREA_ANGLE_SNAP,
        Extra::RippleControl => AREA_RIPPLE,
        Extra::Turbo => return None,
    })
}

/// The rows the card can have, in its order (values none: the guard's rows).
pub fn extra_rows() -> Vec<ExtraRow> {
    vec![
        ExtraRow::new(Extra::Debounce, Kind::Range { min: 0, max: DEBOUNCE_MAX_MS, step: 1, unit: "ms" }, None),
        ExtraRow::switch(Extra::MotionSync, None),
        ExtraRow::switch(Extra::AngleSnap, None),
        ExtraRow::switch(Extra::RippleControl, None),
    ]
}

// ---------------------------------------------------------------- the driver

/// The WALLHACK M-001 driver.
pub struct Wallhack;

impl Driver for Wallhack {
    /// readStatus' reads for the four values this app shows: battery, active DPI stage + the stage record, polling rank,
    /// lift-off code. No answer to the first two = the mouse is not answering.
    fn read(&self, _m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let mut r = OnMouse { caps: caps(), ..Default::default() };
        let battery = optional(exchange(port, &simple(CMD_BATTERY), CMD_BATTERY))?.and_then(|b| decode_battery(&b));
        let rank = byte_at(port, AREA_DPI_RANK)?;
        if battery.is_none() && rank.is_none() {
            return Ok(r);
        }
        r.online = true;
        if let Some((pct, charging)) = battery {
            r.battery_percent = pct;
            r.charging = Some(charging);
        }
        // OpenMouse treats an unreadable stage as 0
        if rank.unwrap_or(0) == 0 {
            r.dpi = dpi_block(port)?.and_then(|b| read_dpi(&b));
        }
        r.polling_hz = byte_at(port, AREA_REPORT_USB)?.and_then(rank_to_hz);
        r.lift_off = byte_at(port, AREA_SILENT_HEIGHT)?.and_then(lift_off_tenths);
        // Order 110: readStatus' order - Motion Sync, angle snapping, ripple, debounce; rows in the card's order
        let mut got = Vec::new();
        for e in [Extra::MotionSync, Extra::AngleSnap, Extra::RippleControl, Extra::Debounce] {
            got.push((e, if let Some(a) = extra_area(e) { byte_at(port, a)? } else { None }));
        }
        r.extras = extra_rows()
            .into_iter()
            .filter_map(|mut row| {
                let raw = got.iter().find(|g| g.0 == row.extra).and_then(|g| g.1)?;
                row.value = Some(if row.extra == Extra::Debounce { raw as i32 } else { (raw == 1) as i32 });
                Some(row)
            })
            .collect();
        Ok(r)
    }

    /// Order 110: setDebounceTime / setMotionSync / setAngleSnapping / setRippleControl - checked before a byte leaves, then
    /// writeVerifiedByte: the one-byte write, the byte read back.
    fn change(&self, _m: &Matched, port: &mut dyn Port, c: Change) -> Result<()> {
        let Change::Extra(e, v) = c;
        check_extra(&extra_rows(), e, v)?;
        let area = extra_area(e).ok_or_else(|| crate::family::not_offered(c))?;
        send(port, &write_request(area, &[v as u8]))?;
        let now = byte_at(port, area)?;
        if now != Some(v as u8) {
            return Err(Error::BadAnswer(format!("The mouse did not confirm the requested {}.", e.label())));
        }
        Ok(())
    }

    fn write(&self, _m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        match s {
            Setting::Dpi(d) => {
                if !caps().dpi.is_some_and(|c| c.allows(d)) {
                    return Err(Error::range("DPI", format!("{d} DPI is not one this WALLHACK mouse can be set to (50 - 26000 in steps of 50)")));
                }
                // no answer is not "stage 1 is active": only a read of exactly 0 allows the write
                if byte_at(port, AREA_DPI_RANK)? != Some(0) {
                    return Err(Error::BadAnswer("the WALLHACK mouse is on another DPI stage than the first (or did not say which) - this app only changes the first stage's DPI - nothing was written".into()));
                }
                let stored = dpi_block(port)?.ok_or_else(|| Error::BadAnswer("the WALLHACK mouse did not answer the DPI read".into()))?;
                let (b, before) = (block_bytes(&stored), read_dpi(&stored));
                // OpenMouse's record: [1, 0, lo, hi, 0x90, 1, 0xFF, 0xFF, 0]; everything but the DPI bytes must already be so
                let fixed = |b: &[u8; 9]| b[0] == 1 && b[1] == 0 && b[4..] == [0x90, 1, 0xFF, 0xFF, 0];
                if !b.as_ref().is_some_and(fixed) {
                    return Err(Error::BadAnswer("the WALLHACK mouse's first DPI stage holds other bytes (colour, enabled flag) than OpenMouse writes - nothing was written".into()));
                }
                send(port, &set_dpi_request(d))?;
                let now = dpi_block(port)?.and_then(|b| read_dpi(&b));
                if now != Some(d) {
                    return Err(Error::BadAnswer(format!("The mouse kept {now:?} DPI instead of {d}.")));
                }
                Ok(Changed { now: d, before })
            }
            Setting::Polling(hz) => {
                let rank = hz_to_rank(hz).ok_or_else(|| Error::range("polling rate", format!("{hz} Hz is not a supported WALLHACK polling rate")))?;
                // wired and wireless polling live in separate bytes and OpenMouse writes both; the wireless one is never shown,
                // so it is only overwritten while it holds the same rate as the wired one (else a rate nobody saw would be lost)
                let (usb, esb) = (byte_at(port, AREA_REPORT_USB)?, byte_at(port, AREA_REPORT_ESB)?);
                if usb.is_none() || usb != esb {
                    return Err(Error::BadAnswer("the WALLHACK mouse's wired and wireless polling rates differ (or could not be read) - this app changes both together only when they match - nothing was written".into()));
                }
                let before = usb.and_then(rank_to_hz);
                send(port, &write_request(AREA_REPORT_USB, &[rank]))?;
                send(port, &write_request(AREA_REPORT_ESB, &[rank]))?;
                let (now_usb, now_esb) = (byte_at(port, AREA_REPORT_USB)?, byte_at(port, AREA_REPORT_ESB)?);
                if now_usb != Some(rank) || now_esb != Some(rank) {
                    return Err(Error::BadAnswer(format!("The mouse did not confirm {hz} Hz.")));
                }
                Ok(Changed { now: hz, before })
            }
            Setting::LiftOff(t) => {
                let code = lift_off_code(t).ok_or_else(|| Error::range("lift-off distance", format!("{t} tenths of a mm: the WALLHACK M-001 has 0.7 mm, 1 mm and 2 mm")))?;
                let before = byte_at(port, AREA_SILENT_HEIGHT)?.and_then(lift_off_tenths);
                send(port, &write_request(AREA_SILENT_HEIGHT, &[code]))?;
                let now = byte_at(port, AREA_SILENT_HEIGHT)?.and_then(lift_off_tenths);
                if now != Some(t) {
                    return Err(Error::BadAnswer("The mouse did not confirm the requested lift-off distance.".into()));
                }
                Ok(Changed { now: t, before })
            }
        }
    }
}
