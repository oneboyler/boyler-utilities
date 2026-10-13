//! Ryunix KYU Pro MX1 (cable 0x026E, receiver 0x026F) - HELD BACK: the codec is ported, the driver does nothing yet.
//!
//! Ported from `ryunix/kyu-pro-mx1.js` (ids, usage page / usage, report ids, the telemetry decoder) and
//! `drivers/ryunix/kyu-pro-mx1-hid.js` (KyuProMx1Client: isSupported, onInputReport, readStatus).
//!
//! What OpenMouse does with this mouse: nothing but LISTEN. It writes nothing, asks nothing, and offers no DPI or polling
//! control (`getDpiOptions()` / `getPollingRateOptions()` are empty, `pollingReadOnly`, `settingsReady: false`). The only data
//! is the mouse's unsolicited INPUT report 0x04 (six bytes): `[active 0|1, dpi stage, polling code, battery 0..100, charging
//! 0|1, LED mode]` with the polling code 0x08 = 125 Hz, 0x04 = 250, 0x02 = 500, 0x01 = 1000 (other codes, a battery above 100 or
//! flags other than 0 / 1 make the whole report invalid). OpenMouse shows its battery, charging state and polling rate (its DPI
//! is a fixed 0 - "dpiStage" is only a stage number).
//!
//! Why the driver is held back: this app's [`HidTransfer`] has no "read the next input report without sending anything"
//! transfer (every input read is the second half of a write). Sending something to get one would be a request OpenMouse never
//! sends. NEEDS TRANSFER (not built): "ReadFile (overlapped, up to ~3 s) on the interface path of the collection with usage
//! page 0x0A / usage 0xC7 (its feature report 0x05 is the control channel OpenMouse's isSupported requires), keep reading input
//! reports until one with report id 0x04 and at least 6 bytes arrives; no write at all". It is unclear whether the telemetry
//! report arrives on that same collection (OpenMouse listens on the whole device) and how often the mouse sends it. With that
//! transfer, `matches` could return the table below and `read` could give battery, charging and polling rate - and nothing else
//! (no DPI, no lift-off, no writes).
//!
//! Until then [`matches`] returns `None` for every id, so the app never claims this mouse and never sends it anything.

use crate::error::{Error, Result};
use crate::family::{Changed, Driver, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::HidInfo;

pub const VENDOR_ID: u16 = 0x04F3;
pub const PID_WIRED: u16 = 0x026E;
pub const PID_RECEIVER: u16 = 0x026F;
pub const MODEL: &str = "Ryunix KYU Pro MX1";
pub const USAGE_PAGE: u16 = 0x000A;
pub const USAGE: u16 = 0x00C7;
pub const TELEMETRY_REPORT_ID: u8 = 0x04;
pub const CONFIG_REPORT_ID: u8 = 0x05;

/// What the telemetry report says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Telemetry {
    pub active: bool,
    pub dpi_stage: u8,
    pub polling_hz: u32,
    pub battery_percent: u8,
    pub charging: bool,
    /// the LED mode code (0 = off ... 7)
    pub led_mode: u8,
}

/// decodeKyuProMx1Telemetry: the six report bytes (behind the report id) and the report's id → the readings, or `None` for any
/// report that fails one of OpenMouse's checks.
pub fn decode_telemetry(report_id: u8, data: &[u8]) -> Option<Telemetry> {
    if report_id != TELEMETRY_REPORT_ID || data.len() < 6 {
        return None;
    }
    let polling_hz = match data[2] {
        0x08 => 125,
        0x04 => 250,
        0x02 => 500,
        0x01 => 1000,
        _ => return None,
    };
    if data[0] > 1 || data[3] > 100 || data[4] > 1 {
        return None;
    }
    Some(Telemetry { active: data[0] == 1, dpi_stage: data[1], polling_hz, battery_percent: data[3], charging: data[4] == 1, led_mode: data[5] })
}

/// Held back (see the header): no mouse is claimed.
pub fn matches(_own: &[&HidInfo], _vid: u16, _pid: u16) -> Option<Matched> {
    None
}

/// The driver sends nothing, so no request is a read request.
pub fn read_request_allowed(_out: &[u8]) -> bool {
    false
}

/// The Ryunix driver (held back).
pub struct Ryunix;

impl Driver for Ryunix {
    fn read(&self, _m: &Matched, _port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        Err(Error::UnsupportedMouse("Ryunix KYU Pro MX1 (needs a transfer that only listens for input reports)".into()))
    }
    fn write(&self, _m: &Matched, _port: &mut dyn Port, _s: Setting) -> Result<Changed> {
        Err(Error::UnsupportedMouse("Ryunix KYU Pro MX1 (OpenMouse changes nothing on it either)".into()))
    }
}
