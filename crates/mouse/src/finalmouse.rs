//! Finalmouse (UltralightX / Starlight-12 dongle, 0x361d:0x0100) - ported from OpenMouse (`crate::family`: AGPL-3.0-or-later,
//! @openmouse/protocol 0.40.1): DPI, polling rate, lift-off distance and battery.
//!
//! - the codec: `finalmouse/index.js` (xpanel's framing: [`request`], [`decode_report`], the battery curve
//!   [`battery_percent_from_mv`]);
//! - the client: `drivers/finalmouse/hid.js` (FinalmouseHidClient), the ULX branch only. Its Starlight X branch (dongle ids
//!   0x0300 / 0x0301, report 0x01 / 0x02, PAW lift-off, click mode, profiles) is not for any model on OpenMouse's SUPPORTED
//!   list ("Starlight-12 / ULX" 0x100 is the only Finalmouse entry; "Starlight Pro Tenz" and "UltraX" are "pending").
//!
//! Frames (Windows' buffer: byte 0 is the report id; OpenMouse's `sendReport(id, body)` keeps the id apart; a body is 63 bytes):
//! - request, output report 0x04 (0x02 for the dongle query): `[1]=2 + payload length [2]=0x80 | command [3]=payload length
//!   [4..]=payload`, zero padded to the collection's output length;
//! - commands we send: 16 = DPI (u16 LE), 17 = polling rate in Hz (u16 LE), 21 = lift-off in mm (1 byte; with no payload it
//!   is the QUERY OpenMouse's readStatus sends), 96 = wake / report everything, 10 = dongle info (output report 0x02);
//! - answers are separate input reports (id 0x05 from the mouse, 0x03 from the dongle): `[id, inner length, command, payload
//!   length, payload..]`: command 3 = DPI (u16 LE), 4 = polling Hz (u16 LE), 5 = battery millivolts (u16 LE), 21 = lift-off mm,
//!   37 = charging state (1 = charging), 38 = `[state of charge %, mV lo, mV hi]`.
//!
//! OpenMouse's readStatus sends three requests (21 query, 96, dongle info), then collects the burst of answers for up to 1 s plus
//! 75 ms and reads DPI / polling / battery out of it. It sets DPI, polling and lift-off with one frame each and reads nothing back.
//!
//! NEEDS TRANSFER: the burst. `HidTransfer` has no "write, then collect every input report for N ms" (and no read-only input
//! transfer); every exchange is one write and ONE matching report ([`HidTransfer::OutputThenMatch`]). This file therefore asks
//! once per value - the same frames OpenMouse sends, each exchange waiting for the one answer it wants (prefix
//! `[0x05, n, command, length]`) - and treats "no answer to any of the three requests" as offline. Which of the three
//! requests the mouse answers a value with is unclear (OpenMouse does not say), so the first value (DPI) is asked with all three in
//! OpenMouse's order and the one that worked is used for the rest. Unclear: whether the mouse's inner-length byte always equals 2 +
//! payload length (it is matched for every length from there up to 62).
//!
//! Left out on purpose: motion sync, dongle LED, tournament scroll, every Starlight X command, profile commands (not DPI /
//! polling / lift-off / battery). OpenMouse sends no firmware, DFU or reset command and neither does this file.
//! Proven only against OpenMouse's bytes (tests); never run on a real unit.

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x361D;
/// the ULX dongle ("Starlight-12 / ULX" on the SUPPORTED list)
pub const ULX_DONGLE: u16 = 0x0100;
pub const MODEL: &str = "Finalmouse Starlight-12 / UltralightX";
/// FINALMOUSE_REPORT: output reports 0x04 (main) and 0x02 (dongle), input reports 0x05 (main) and 0x03 (dongle)
pub const MAIN_OUT: u8 = 0x04;
pub const MAIN_IN: u8 = 0x05;
pub const DONGLE_OUT: u8 = 0x02;
pub const DONGLE_IN: u8 = 0x03;
/// OUTPUT_REPORT_LENGTH 63 + the report id
pub const OUT_LEN: usize = 64;

/// FINALMOUSE_COMMAND
pub const CMD_DPI: u8 = 16;
pub const CMD_POLLING: u8 = 17;
pub const CMD_LIFT_OFF: u8 = 21;
pub const CMD_WAKE_ALL: u8 = 96;
pub const CMD_DONGLE_INFO: u8 = 10;

/// DPI_MIN / DPI_MAX (getDpiOptions: every integer between)
pub const DPI_MIN: u32 = 50;
pub const DPI_MAX: u32 = 26_000;
/// the client's POLLING_RATES
pub const POLLING: [u32; 5] = [500, 1000, 2000, 4000, 8000];
/// the legacy (ULX) lift-off stops, mm (setLiftOffDistance: "Medium" = 1, "High" = 2)
pub const LIFT_OFF_MM: [u32; 2] = [1, 2];

/// `buildFinalmouseReport(command, payload)` as Windows sends it on `report_id`: `[id, 2 + n, 0x80 | command, n, payload..]`,
/// zero padded to `len` bytes.
pub fn request(report_id: u8, command: u8, payload: &[u8], len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len.max(OUT_LEN)];
    v[0] = report_id;
    v[1] = 2 + payload.len() as u8;
    v[2] = 0x80 | command;
    v[3] = payload.len() as u8;
    v[4..4 + payload.len()].copy_from_slice(payload);
    v
}

/// The three requests of OpenMouse's readStatus (ULX), in its order: lift-off query, wake-all, dongle info.
pub fn read_requests(len: usize) -> [Vec<u8>; 3] {
    [request(MAIN_OUT, CMD_LIFT_OFF, &[], len), request(MAIN_OUT, CMD_WAKE_ALL, &[], len), request(DONGLE_OUT, CMD_DONGLE_INFO, &[], len)]
}

pub fn dpi_request(dpi: u32, len: usize) -> Option<Vec<u8>> {
    (DPI_MIN..=DPI_MAX).contains(&dpi).then(|| request(MAIN_OUT, CMD_DPI, &(dpi as u16).to_le_bytes(), len))
}

pub fn polling_request(hz: u32, len: usize) -> Option<Vec<u8>> {
    POLLING.contains(&hz).then(|| request(MAIN_OUT, CMD_POLLING, &(hz as u16).to_le_bytes(), len))
}

pub fn lift_off_request(mm: u32, len: usize) -> Option<Vec<u8>> {
    LIFT_OFF_MM.contains(&mm).then(|| request(MAIN_OUT, CMD_LIFT_OFF, &[mm as u8], len))
}

/// What one answer report says (the fields of `decodeFinalmouseReport` this file uses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Update {
    Dpi(u32),
    Polling(u32),
    BatteryMv(u32),
    LiftOffMm(u32),
    /// command 37: 1 = charging
    Charging(u8),
    /// command 38: state of charge in percent
    BatterySoc(u8),
}

/// `decodeFinalmouseReport(reportId, data)` for the mouse's input report (0x05). `report` is Windows' buffer: the id first.
pub fn decode_report(report: &[u8]) -> Option<Update> {
    if report.first() != Some(&MAIN_IN) || report.len() < 4 {
        return None;
    }
    let data = &report[1..];
    let inner = (data[0] as usize).min(data.len() - 1);
    let body = &data[1..inner + 1];
    if body.len() < 2 {
        return None;
    }
    let command = body[0];
    let n = (body[1] as usize).min(body.len().saturating_sub(2));
    let payload = &body[2..2 + n];
    let u16le = |p: &[u8]| (p.len() >= 2).then(|| p[0] as u32 | ((p[1] as u32) << 8));
    match command {
        3 => u16le(payload).map(Update::Dpi),
        4 => u16le(payload).map(Update::Polling),
        5 => u16le(payload).map(Update::BatteryMv),
        21 => payload.first().map(|b| Update::LiftOffMm(*b as u32)),
        37 => payload.first().map(|b| Update::Charging(*b)),
        38 if payload.len() >= 3 => Some(Update::BatterySoc(payload[0])),
        _ => None,
    }
}

/// `finalmouseBatteryPercent`: xpanel's ULX voltage curve.
pub fn battery_percent_from_mv(mv: u32) -> u8 {
    if mv == 0 {
        return 0;
    }
    let v = mv as f64 / 1000.0;
    const VOLTS: [f64; 7] = [3.0, 3.62, 3.66, 3.74, 3.88, 4.17, 4.38];
    const PCT: [f64; 7] = [0.2, 5.0, 10.0, 25.0, 50.0, 75.0, 100.0];
    if v >= VOLTS[6] {
        return 100;
    }
    if v <= VOLTS[0] {
        return 0;
    }
    for i in 0..6 {
        let (lo, hi) = (VOLTS[i], VOLTS[i + 1]);
        if v < lo || v > hi {
            continue;
        }
        let f = (v - lo) / (hi - lo);
        return (PCT[i] + (PCT[i + 1] - PCT[i]) * f).round() as u8;
    }
    0
}

/// What the ULX can do here.
pub fn caps() -> Caps {
    Caps { dpi: Some(DpiCaps::uniform(DPI_MIN, DPI_MAX, 1)), polling: POLLING.to_vec(), lift_off: LIFT_OFF_MM.iter().map(|m| m * 10).collect(), battery: true }
}

/// OpenMouse's isSupported for the ULX dongle: the vendor id, product 0x0100 and a collection on usage page 0xFF00, usage 1
/// (the one the reports 0x02 / 0x04 / 0x03 / 0x05 belong to). A collection too short for the 64-byte frames is not used.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID || pid != ULX_DONGLE {
        return None;
    }
    let config = own.iter().find(|h| h.usage_page == 0xFF00 && h.usage == 0x0001 && h.output_len as usize >= OUT_LEN && h.input_len > 3)?;
    Some(Matched { family: Family::Finalmouse, model: Some(MODEL), wireless: Some(true), config: (*config).clone(), variant: pid })
}

/// The allow-list for the real mouse: the three read requests of readStatus - the lift-off query (no payload), wake-all and the
/// dongle info - as 64-byte-or-longer output reports, zero padded. Every write has a payload and never passes.
pub fn read_request_allowed(out: &[u8]) -> bool {
    out.len() >= OUT_LEN && read_requests(out.len()).iter().any(|r| r == out)
}

/// The Finalmouse driver (ULX).
pub struct Finalmouse;

/// the wait for one answer (OpenMouse waits up to 1000 ms for the first report, then 75 ms for the rest of the burst)
const FIRST_MS: u32 = 400;
const NEXT_MS: u32 = 250;
const MAX_READS: u32 = 24;

/// `[0x05, n, command, length]` for every inner length `n` a frame of this payload length can have (2 + length ..= 62).
fn prefixes(command: u8, plen: u8) -> Vec<Vec<u8>> {
    (2 + plen..=62).map(|n| vec![MAIN_IN, n, command, plen]).collect()
}

/// One request and the one answer wanted from the burst; `None` = no such answer in time.
fn ask(port: &mut dyn Port, out: &[u8], command: u8, plen: u8, timeout_ms: u32) -> Result<Option<Update>> {
    let how = HidTransfer::OutputThenMatch { reply_len: (port.info().input_len as usize).max(8), max_reads: MAX_READS, timeout_ms, prefixes: prefixes(command, plen) };
    match port.exchange(out, &how) {
        Ok(a) => Ok(decode_report(&a)),
        Err(Error::BadAnswer(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

fn out_len(port: &dyn Port) -> usize {
    (port.info().output_len as usize).max(OUT_LEN)
}

impl crate::family::Driver for Finalmouse {
    /// OpenMouse's readStatus (see the header for how the burst is asked). No answer to any request = asleep / out of range.
    fn read(&self, _m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let len = out_len(port);
        let reqs = read_requests(len);
        let mut r = OnMouse { online: true, wireless: Some(true), caps: caps(), ..Default::default() };
        // DPI first, with each of the three requests until one is answered
        let mut used = None;
        for (i, q) in reqs.iter().enumerate() {
            if let Some(Update::Dpi(d)) = ask(port, q, 3, 2, FIRST_MS)? {
                r.dpi = Some(d);
                used = Some(i);
                break;
            }
        }
        let Some(i) = used else {
            r.online = false;
            return Ok(r);
        };
        let q = &reqs[i];
        if let Some(Update::Polling(hz)) = ask(port, q, 4, 2, NEXT_MS)? {
            r.polling_hz = Some(hz);
        }
        let soc = ask(port, q, 38, 3, NEXT_MS)?;
        r.battery_percent = match soc {
            // a state of charge above 100 % (0xFF = unknown) is no reading: the voltage curve answers instead
            Some(Update::BatterySoc(p)) if p <= 100 => Some(p),
            _ => match ask(port, q, 5, 2, NEXT_MS)? {
                Some(Update::BatteryMv(mv)) => Some(battery_percent_from_mv(mv)),
                _ => None,
            },
        };
        if let Some(Update::Charging(s)) = ask(port, q, 37, 1, NEXT_MS)? {
            r.charging = Some(s == 1);
        }
        // the lift-off query names its own answer (command 21)
        if let Some(Update::LiftOffMm(mm)) = ask(port, &reqs[0], 21, 1, NEXT_MS)? {
            if LIFT_OFF_MM.contains(&mm) {
                r.lift_off = Some(mm * 10);
            }
        }
        Ok(r)
    }

    /// OpenMouse's setDpi / setPollingRate / setLiftOffDistance: one frame, nothing read back (so `before` is `None`).
    fn write(&self, _m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let len = out_len(port);
        let req = match s {
            Setting::Dpi(d) => dpi_request(d, len).ok_or_else(|| Error::range("DPI", "Finalmouse UltralightX DPI must be between 50 and 26,000"))?,
            Setting::Polling(hz) => polling_request(hz, len).ok_or_else(|| Error::range("polling rate", format!("{hz} Hz is not a Finalmouse UltralightX polling rate")))?,
            Setting::LiftOff(t) => {
                let mm = t.is_multiple_of(10).then_some(t / 10);
                mm.and_then(|mm| lift_off_request(mm, len)).ok_or_else(|| Error::range("lift-off distance", "Finalmouse UltralightX lift-off distance must be 1 mm or 2 mm"))?
            }
        };
        port.exchange(&req, &HidTransfer::OutputOnly)?;
        Ok(Changed { now: s.value(), before: None })
    }
}
