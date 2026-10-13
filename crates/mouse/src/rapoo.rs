//! Rapoo VT9 Pro (receiver 0x1205, cable 0x4405) - BATTERY only, ported from OpenMouse (`crate::family`: AGPL-3.0-or-later).
//!
//! Ported from `rapoo/index.js` (ids, usage pages, `rapooStatusBatteryPercent`, the register map in its header comment) and
//! `drivers/rapoo/hid.js` (RapooHidClient: isSupported, readFeatureBattery, readRegisters / exchange, the "nothing is writable
//! yet" status note).
//!
//! What is ported: the battery percentage from the feature report 0x2A of the status collection (usage page 0xFF0B / usage
//! 0x0104) - the one Rapoo read a browser can do (`receiveFeatureReport`), answered while the mouse is idle. The report is
//! `[0x2A, ...]` with the percentage at byte 9 of the Windows buffer (OpenMouse: "relative to the report data", report byte 8
//! behind the id; example `2A 01 00 00 04 21 50 00 00 62` = 98 %); values above 100 are no reading. The charging state is NOT
//! in that report (OpenMouse takes it from the register channel), so it is not shown.
//!
//! What is held back, and why: OpenMouse reads DPI (register 0x888 / 0x898), polling rate (0x880), motion sync and the rest
//! through its OpenMouse Bridge only - the register channel answers through GET_REPORT(Input), which WebHID lacks - and OpenMouse
//! itself writes NOTHING to a Rapoo mouse ("This driver does not write to the mouse yet"; DPI / polling `pollingReadOnly`).
//! NEEDS TRANSFER (not built): "send the 31-byte output report 0xBA `[connection 0xA5 (receiver) / 0xFF (cable), 0xA4 (read), length,
//! address u32 little-endian, zeros]` on the 0xFF00 / 0x000E collection (WriteFile), then HidD_GetInputReport(0xBA) every ~2 ms
//! for up to 120 ms until a busy status (first byte != 1) was followed by an OK (first byte 1); the block starts at answer byte 4;
//! resend up to 4 times, 50 ms apart; the whole read up to 4 rounds, 120 ms apart; battery query command 0xAA: answer marker
//! byte 1 = 1 or 2 (2 = charging), percent at byte 2". With it the DPI stages, active stage and polling rate (register values
//! in the header of `rapoo/index.js`) could be shown, still read-only. Lift-off: the stored selector is read by OpenMouse but
//! "the scale it indexes is not established", so it is never shown.
//!
//! Safety: the only request this file sends is the feature-report read 0x2A.

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, Driver, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x24AE;
/// The 2.4 GHz receiver.
pub const PID_RECEIVER: u16 = 0x1205;
/// The same mouse on its cable.
pub const PID_WIRED: u16 = 0x4405;
pub const MODEL: &str = "Rapoo VT9 Pro";
/// The configuration collection OpenMouse's isSupported looks for.
pub const CONFIG_USAGE_PAGE: u16 = 0xFF00;
pub const CONFIG_USAGE: u16 = 0x000E;
/// The status collection that answers the feature report.
pub const STATUS_USAGE_PAGE: u16 = 0xFF0B;
pub const STATUS_USAGE: u16 = 0x0104;
pub const STATUS_REPORT_ID: u8 = 0x2A;
/// The shortest Windows buffer that holds the percentage (id + 9 bytes).
pub const STATUS_MIN_LEN: usize = 10;

/// rapooStatusBatteryPercent for a Windows buffer (the report id first).
pub fn battery_percent(buf: &[u8]) -> Option<u8> {
    if buf.first() != Some(&STATUS_REPORT_ID) || buf.len() < STATUS_MIN_LEN {
        return None;
    }
    (buf[9] <= 100).then_some(buf[9])
}

/// The request: HidD_GetFeature of report 0x2A (`[0x2A, zeros]` at the collection's feature size).
pub fn status_request(len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len.max(STATUS_MIN_LEN)];
    v[0] = STATUS_REPORT_ID;
    v
}

/// OpenMouse's isSupported (a Rapoo id and the 0xFF00 / 0x000E configuration collection) plus the status collection this port
/// reads through.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID || (pid != PID_RECEIVER && pid != PID_WIRED) {
        return None;
    }
    own.iter().find(|h| h.usage_page == CONFIG_USAGE_PAGE && h.usage == CONFIG_USAGE)?;
    let c = own.iter().find(|h| h.usage_page == STATUS_USAGE_PAGE && h.usage == STATUS_USAGE && h.feature_len as usize >= STATUS_MIN_LEN)?;
    Some(Matched { family: Family::Rapoo, model: Some(MODEL), wireless: Some(pid == PID_RECEIVER), config: (*c).clone(), variant: pid })
}

/// The allow-list for the read-only proof on a real mouse: NO frame passes. The one read (report 0x2A) is a HidD_GetFeature
/// ([`HidTransfer::FeatureIn`]: nothing is sent, and the transfer kind itself is let through); the blank `[0x2A, 0, ...]` that is
/// the `out` of that read must not pass as a SetFeature, which would write to the mouse. This family sends nothing else.
pub fn read_request_allowed(_out: &[u8]) -> bool {
    false
}

pub fn caps() -> Caps {
    Caps { dpi: None, polling: Vec::new(), lift_off: Vec::new(), battery: true }
}

/// The Rapoo driver.
pub struct Rapoo;

impl Driver for Rapoo {
    /// readFeatureBattery: no answer (or a value above 100) = no battery; the mouse is "online" when it answers.
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let mut r = OnMouse { wireless: Some(m.variant == PID_RECEIVER), caps: caps(), ..Default::default() };
        let len = (port.info().feature_len as usize).max(STATUS_MIN_LEN);
        let out = status_request(len);
        match port.exchange(&out, &HidTransfer::FeatureIn { reply_id: STATUS_REPORT_ID, reply_len: len, wait_ms: 0 }) {
            Ok(a) => {
                r.online = a.first() == Some(&STATUS_REPORT_ID);
                r.battery_percent = battery_percent(&a);
            }
            Err(Error::BadAnswer(_) | Error::Os { .. }) => {}
            Err(e) => return Err(e),
        }
        Ok(r)
    }

    /// Nothing is writable: OpenMouse's Rapoo driver writes nothing.
    fn write(&self, _m: &Matched, _port: &mut dyn Port, s: Setting) -> Result<Changed> {
        Err(Error::range(
            match s {
                Setting::Dpi(_) => "DPI",
                Setting::Polling(_) => "polling rate",
                Setting::LiftOff(_) => "lift-off distance",
            },
            "OpenMouse can't change this setting on a Rapoo mouse yet, so this app doesn't either".to_string(),
        ))
    }
}
