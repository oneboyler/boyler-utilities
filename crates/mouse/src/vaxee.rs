//! VAXEE - 63-byte feature reports on report id 0x0e, ported from OpenMouse (`crate::family`: AGPL-3.0-or-later,
//! @openmouse/protocol 0.40.1), DPI / polling / battery only:
//!
//! - the codec and device ids: `vaxee/index.js` (VAXEE Control Center v3.6 protocol: vaxeeRequest, vaxeeReply, vaxeeDpi,
//!   the polling codes, the product ids and mouse names);
//! - the client: `drivers/vaxee/hid.js` (VaxeeHidClient: isSupported, readStatus, setDpi, setPollingRate, exchange); the
//!   model list is `supported-devices-data.ts` (status "supported", 13 VAXEE mice, pids 0x1003 - 0x1015).
//!
//! Packet (63 bytes = OpenMouse's feature report payload; Windows puts the report id 0x0e in front, so OpenMouse's byte i is
//! our byte i + 1; the collection's feature length is 64): request `[0]` 0xa5 · `[1]` command · `[2]` 1 = read, 2 = write ·
//! `[3]` data length · `[4..]` values; answer `[0]` 0xa5 · `[1]` command echoed · `[2]` 3 · `[3]` 1 · `[4]` data length ·
//! `[5..]` data. Every request is HidD_SetFeature, a 100 ms pause (the vendor panel's own wait), HidD_GetFeature
//! ([`HidTransfer::Feature`]); there is no retry - an answer that is not the command's is an error.
//!
//! Commands used: 0x0a mouse product id (read, 2 bytes), 0x02 DPI stage (1 - 4), 0x04 DPI value of a stage (`[stage]` read,
//! `[stage, lo, hi]` write; the answer repeats stage, lo, hi), 0x07 polling (code 1 = 500 Hz, 2 = 1000), 0x0b battery (steps
//! of 5 %, at most 20), 0x10 charging. OpenMouse's setDpi changes the value of the ACTIVE stage only (nothing else is
//! rewritten); setPollingRate writes the one code, and both read the value back and compare.
//!
//! Left out on purpose: lift-off (OpenMouse's LOD is Low / High with no millimetre figure anywhere in its code, so a
//! tenths-of-a-mm row would be made up), the tracking-mode read (OpenMouse needs it only to allow more than 1000 Hz, which
//! the listed mice can't be set to), firmware, debounce, DPI-stage enable flags, trajectory and profile commands (none is
//! DPI / polling / lift-off / battery). The receivers (0x0005, 0x1001, 0x1002, 0x2001, 0x2002) and the 4K / 8K polling rates
//! that go with them are not on OpenMouse's SUPPORTED list. OpenMouse sends no firmware-update, DFU or reset command to a
//! VAXEE and neither does this file. Nothing here ran on a real unit.

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Driver, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x3057;
pub const USAGE_PAGE: u16 = 0xff05;
pub const USAGE: u16 = 0x01;
pub const REPORT_ID: u8 = 0x0e;
/// VAXEE_REPORT_SIZE: the feature report's payload.
pub const PACKET_LEN: usize = 63;
/// Windows' feature report: report id 0x0e + the 63-byte payload.
pub const REPORT_LEN: usize = PACKET_LEN + 1;
/// The vendor panel waits 100 ms between send and receive.
pub const PAUSE_MS: u32 = 100;

pub const CMD_FIRMWARE: u8 = 0x01;
pub const CMD_DPI_STAGE: u8 = 0x02;
pub const CMD_DPI_VALUE: u8 = 0x04;
pub const CMD_POLLING: u8 = 0x07;
pub const CMD_MOUSE_PID: u8 = 0x0a;
pub const CMD_BATTERY: u8 = 0x0b;
pub const CMD_CHARGING: u8 = 0x10;

/// getDpiOptions / setDpi: 100 - 26000 in steps of 50.
pub const DPI_MIN: u32 = 100;
pub const DPI_MAX: u32 = 26000;
pub const DPI_STEP: u32 = 50;
/// getSupportedPollingRates for every product id on the list (only the receivers 0x1002 / 0x2001 / 0x2002 get more).
pub const POLLING_RATES: [u32; 2] = [500, 1000];

// ---------------------------------------------------------------- the codec

/// vaxeeRequest: `[0xa5, command, write ? 2 : 1, length, values...]`, zero-padded to 63 bytes.
pub fn request(command: u8, length: u8, values: &[u8], write: bool) -> [u8; PACKET_LEN] {
    let mut b = [0u8; PACKET_LEN];
    b[..4].copy_from_slice(&[0xa5, command, if write { 2 } else { 1 }, length]);
    b[4..4 + values.len()].copy_from_slice(values);
    b
}

/// The request as Windows sends it: report id 0x0e, the 63-byte payload, zeros up to the collection's feature length.
pub fn report(command: u8, length: u8, values: &[u8], write: bool, feature_len: usize) -> Vec<u8> {
    let mut v = vec![0u8; feature_len.max(REPORT_LEN)];
    v[0] = REPORT_ID;
    v[1..REPORT_LEN].copy_from_slice(&request(command, length, values, write));
    v
}

/// vaxeeReply on an answer WITHOUT its report id: `a5`, the command, `03 01`, a data length of at least `min`; `None` = not
/// that command's answer (OpenMouse throws "Invalid VAXEE reply").
pub fn check_reply(bytes: &[u8], command: u8, min: usize) -> Option<&[u8]> {
    (bytes.len() >= 5 + min && bytes[0] == 0xa5 && bytes[1] == command && bytes[2] == 3 && bytes[3] == 1 && bytes[4] as usize >= min).then_some(bytes)
}

/// vaxeeDpi: the stage is byte 5, the DPI bytes 6 - 7, little-endian.
pub fn decode_dpi(reply: &[u8]) -> Option<u32> {
    check_reply(reply, CMD_DPI_VALUE, 3).map(|r| r[6] as u32 | (r[7] as u32) << 8)
}

/// vaxeePollingRate: the rate of a code (1 - 5), `None` for any other.
pub fn polling_hz(code: u8) -> Option<u32> {
    [0, 500, 1000, 2000, 4000, 8000].get(code as usize).copied().filter(|r| *r != 0)
}

/// vaxeePollingCode
pub fn polling_code(hz: u32) -> Option<u8> {
    match hz {
        500 => Some(1),
        1000 => Some(2),
        2000 => Some(3),
        4000 => Some(4),
        8000 => Some(5),
        _ => None,
    }
}

/// The battery in percent: the answer counts steps of 5 (at most 20); more is no reading.
pub fn battery_percent(steps: u8) -> Option<u8> {
    (steps <= 20).then(|| steps * 5)
}

/// The mouse's own name from the product id it reports (`VAXEE_MOUSE_NAMES`, shown as "VAXEE <name>").
pub fn mouse_name(pid: u16) -> Option<&'static str> {
    PRODUCTS.iter().find(|p| p.pid == pid).map(|p| p.model)
}

/// What the Mouse page offers: DPI 100 - 26000 in 50s, polling 500 / 1000 Hz, battery; no lift-off row (see the header).
pub fn caps() -> Caps {
    Caps { dpi: Some(DpiCaps::uniform(DPI_MIN, DPI_MAX, DPI_STEP)), polling: POLLING_RATES.to_vec(), lift_off: Vec::new(), battery: true }
}

/// The read requests this file sends (command, declared length, values) - the allow-list's table. DPI value: `[stage]`.
fn read_requests(stage: u8) -> [[u8; PACKET_LEN]; 6] {
    [
        request(CMD_MOUSE_PID, 2, &[], false),
        request(CMD_DPI_STAGE, 1, &[], false),
        request(CMD_DPI_VALUE, 3, &[stage], false),
        request(CMD_POLLING, 1, &[], false),
        request(CMD_BATTERY, 1, &[], false),
        request(CMD_CHARGING, 1, &[], false),
    ]
}

/// The read-only proof's allow-list: report id 0x0e and a packet that is exactly one of the READ requests (byte 2 = 1; the DPI
/// value read for stage 1 - 4), zeros after it. Every write (byte 2 = 2) is refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    if out.len() < REPORT_LEN || out[0] != REPORT_ID || out[REPORT_LEN..].iter().any(|b| *b != 0) {
        return false;
    }
    let p = &out[1..REPORT_LEN];
    (1..=4).any(|stage| read_requests(stage).iter().any(|r| r[..] == *p))
}

// ---------------------------------------------------------------- the device table

/// One VAXEE product id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Product {
    pub pid: u16,
    /// "VAXEE <name>" as OpenMouse shows it
    pub model: &'static str,
}

const fn p(pid: u16, model: &'static str) -> Product {
    Product { pid, model }
}

/// OpenMouse's SUPPORTED list (supported-devices-data.ts, status "supported"): the 13 mice, names from `VAXEE_MOUSE_NAMES`.
/// The ids are the mouse itself; OpenMouse shows these as "Wired USB" (the receivers have their own ids).
pub const PRODUCTS: [Product; 13] = [
    p(0x1003, "VAXEE XE Wireless"),
    p(0x1004, "VAXEE NP-01S Wireless"),
    p(0x1005, "VAXEE AX Wireless"),
    p(0x1006, "VAXEE NP-01 Wireless"),
    p(0x1007, "VAXEE XE-S Wireless"),
    p(0x1008, "VAXEE XE-S-L Wireless"),
    p(0x1009, "VAXEE x Ninjutso Sora"),
    p(0x1010, "VAXEE E1 Wireless"),
    p(0x1011, "VAXEE NP-01S V2 Wireless"),
    p(0x1012, "VAXEE XE V2 Wireless"),
    p(0x1013, "VAXEE NP-01S Ergo Wireless"),
    p(0x1014, "VAXEE NP-01S V3 Wireless"),
    p(0x1015, "VAXEE NP-01 Ergo Wireless"),
];

pub fn product(pid: u16) -> Option<&'static Product> {
    PRODUCTS.iter().find(|p| p.pid == pid)
}

/// hasControlCollection: usage page 0xff05, usage 1, with a feature report on id 0x0e (Windows: a feature length of 64).
pub fn config_interface<'a>(own: &[&'a HidInfo]) -> Option<&'a HidInfo> {
    own.iter().find(|h| h.usage_page == USAGE_PAGE && h.usage == USAGE && h.feature_len as usize >= REPORT_LEN).copied()
}

/// OpenMouse's VaxeeHidClient.isSupported: the vendor id, a listed product id, the control collection.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID {
        return None;
    }
    let p = product(pid)?;
    let config = config_interface(own)?;
    Some(Matched { family: Family::Vaxee, model: Some(p.model), wireless: Some(false), config: config.clone(), variant: pid })
}

// ---------------------------------------------------------------- the exchange

/// One request -> the answer's bytes (without the report id), checked like vaxeeReply. `min` = the least data bytes.
fn exchange(port: &mut dyn Port, command: u8, length: u8, values: &[u8], min: usize, write: bool) -> Result<Vec<u8>> {
    let len = (port.info().feature_len as usize).max(REPORT_LEN);
    let out = report(command, length, values, write, len);
    let a = port.exchange(&out, &HidTransfer::Feature { reply_id: REPORT_ID, reply_len: len, wait_ms: PAUSE_MS })?;
    let data = a.get(1..).unwrap_or(&[]);
    check_reply(data, command, min).map(|d| d.to_vec()).ok_or_else(|| Error::BadAnswer(format!("an invalid VAXEE reply for command {command:#04x}")))
}

fn read_stage(port: &mut dyn Port) -> Result<u8> {
    let stage = exchange(port, CMD_DPI_STAGE, 1, &[], 1, false)?[5];
    if (1..=4).contains(&stage) {
        Ok(stage)
    } else {
        Err(Error::BadAnswer(format!("an invalid VAXEE DPI stage {stage}")))
    }
}

fn read_dpi_of(port: &mut dyn Port, stage: u8) -> Result<u32> {
    let r = exchange(port, CMD_DPI_VALUE, 3, &[stage], 3, false)?;
    decode_dpi(&r).ok_or_else(|| Error::BadAnswer("a short VAXEE DPI answer".into()))
}

fn read_polling(port: &mut dyn Port) -> Result<u32> {
    let code = exchange(port, CMD_POLLING, 1, &[], 1, false)?[5];
    polling_hz(code).ok_or_else(|| Error::BadAnswer(format!("an unsupported VAXEE polling code {code}")))
}

/// `catch(() => null)` for one field: a bad answer leaves the value out; a mouse that is gone still fails.
fn optional<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::BadAnswer(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------- the driver

/// The VAXEE driver (VaxeeHidClient).
pub struct Vaxee;

impl Driver for Vaxee {
    /// READ requests only: OpenMouse's readStatus, cut to the product id (is it answering, and which mouse), DPI (stage, then
    /// that stage's value), polling, battery, charging. No answer to the first request = offline; one that fails later
    /// leaves only its own value out.
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let mut r = OnMouse { wireless: m.wireless, caps: caps(), ..Default::default() };
        let Some(id) = optional(exchange(port, CMD_MOUSE_PID, 2, &[], 2, false))? else {
            return Ok(r);
        };
        r.online = true;
        r.model = mouse_name(id[5] as u16 | (id[6] as u16) << 8).map(|s| s.to_string());
        if let Some(stage) = optional(read_stage(port))? {
            r.dpi = optional(read_dpi_of(port, stage))?;
        }
        r.polling_hz = optional(read_polling(port))?;
        r.battery_percent = optional(exchange(port, CMD_BATTERY, 1, &[], 1, false))?.and_then(|b| battery_percent(b[5]));
        r.charging = optional(exchange(port, CMD_CHARGING, 1, &[], 1, false))?.map(|b| b[5] != 0);
        Ok(r)
    }

    /// OpenMouse's setDpi / setPollingRate: the write, then the read-back that is compared with the request. Before the write,
    /// one read of the same value for undo ("before"); a failed one stops the change. Lift-off is not offered.
    fn write(&self, _m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        match s {
            Setting::Dpi(dpi) => {
                if !caps().dpi.is_some_and(|d| d.allows(dpi)) {
                    return Err(Error::range("DPI", format!("{dpi} DPI is not one this VAXEE can store (100 - 26000 in steps of 50)")));
                }
                let stage = read_stage(port)?;
                let before = read_dpi_of(port, stage)?;
                exchange(port, CMD_DPI_VALUE, 3, &[stage, (dpi & 255) as u8, (dpi >> 8) as u8], 1, true)?;
                let now = read_dpi_of(port, stage)?;
                if now != dpi {
                    return Err(Error::BadAnswer(format!("the VAXEE kept {now} DPI instead of {dpi}")));
                }
                Ok(Changed { now: dpi, before: Some(before) })
            }
            Setting::Polling(hz) => {
                let code = polling_code(hz).filter(|_| POLLING_RATES.contains(&hz));
                let code = code.ok_or_else(|| Error::range("polling rate", format!("{hz} Hz is not a polling rate of this VAXEE")))?;
                let before = read_polling(port)?;
                exchange(port, CMD_POLLING, 1, &[code], 1, true)?;
                let now = read_polling(port)?;
                if now != hz {
                    return Err(Error::BadAnswer(format!("the VAXEE kept {now} Hz instead of {hz} Hz")));
                }
                Ok(Changed { now: hz, before: Some(before) })
            }
            Setting::LiftOff(t) => Err(Error::range("lift-off distance", format!("{t} tenths of a mm: a VAXEE's lift-off is Low / High with no millimetre figure, so it is not set here"))),
        }
    }
}
