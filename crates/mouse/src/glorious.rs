//! Glorious - ported from OpenMouse (`crate::family`: AGPL-3.0-or-later, @openmouse/protocol 0.40.1): polling rate,
//! lift-off distance and (receiver ids) battery. Only OpenMouse's CLASSIC client is used here:
//!
//! - the codec: `glorious-classic/index.js` (the pre-Pixart Model O / D protocol, reverse-engineered from glorious-ctl and
//!   mxw): one 64-byte unnumbered feature report per command;
//! - the client: `drivers/glorious/classic-hid.js` (GloriousClassicHidClient). All five models on OpenMouse's SUPPORTED list
//!   (Model O 2 PRO, Model O, Model D Wireless, Model D, Model O Pro) are in its product table
//!   (`GLORIOUS_CLASSIC_PRODUCTS` in `drivers/vendors.js`, generation "core1") on vendor id 0x258a, so the other two Glorious
//!   clients are not used: `drivers/glorious/hid.js` (Pixart Model O 2 / I 2 family, vendor 0x093a, "write-only") and
//!   `core2-hid.js` (Model O2 / D2 PRO 4K/8K, ids 0x201b / 0x201c / 0x2035 / 0x2036, status "pr" = not supported yet).
//!   The registry notes 0x258a is shared with Redragon's M690 PRO (disjoint by product id).
//!
//! Packet (Windows' buffer = the 65-byte `bfr` of the source tools: byte 0 is the report id 0, then OpenMouse's 64-byte body, so
//! OpenMouse's `bfr(body, n, v)` is our byte n):
//! - polling rate: `[3]=0x02 [4]=0x01 [5]=0x01 [7]=interval in ms` (1, 2, 4, 8 ms = 1000, 500, 250, 125 Hz), no answer;
//! - lift-off distance: `[3]=0x02 [4]=0x01 [5]=0x01 [6]=0x07 [7]=mm - 1` (1 mm = 0, 2 mm = 1; mxw: "only those two are valid"),
//!   no answer;
//! - battery (receiver ids only, as OpenMouse): `[3]=0x02 [4]=0x02 [6]=0x83`, then 60 ms, then the answer:
//!   `[1]` status (0xa1 normal, 0xa4 asleep, 0xa0 waking, 0xa2 / 0xa3 unknown) · `[6]` echo 0x83 · `[7]` charging (1) · `[8]` percent.
//!
//! Held back on purpose:
//! - DPI. OpenMouse's setDpi rewrites the WHOLE 4-stage table (`02 12 01 01 profile count stages...`, then the active stage) from
//!   its own browser-side cache (default 800/1600/3200/6400) - the protocol cannot read the stages, so setting one DPI would
//!   overwrite the user's other three saved stages with guesses. No DPI row for these mice.
//! - Reading DPI / polling / lift-off back: the protocol is write-only for them (OpenMouse keeps a cache; this file has no
//!   cache), so `read` returns only the battery, and a write returns `before: None` and is not read back.
//! - RGB, debounce, active-stage, profile commands (not DPI / polling / lift-off / battery).
//! - Writes on a unit whose config feature report is not 64 bytes (OpenMouse's isConfirmedReportLength): such a unit is not
//!   matched at all.
//!
//! Unclear: whether a unit's config report is numbered (OpenMouse reads the id from the descriptor; Windows' HidInfo has only the
//! length, 65 = id + 64, which this file takes as the unnumbered id 0 report).
//! Proven only against OpenMouse's bytes (tests); never run on a real unit.

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

/// Sinowealth's vendor id, where every SUPPORTED Glorious model lives (`VENDOR_ID.gloriousClassic`).
pub const VENDOR_ID: u16 = 0x258A;
/// report id 0 + the 64-byte body
pub const REPORT_LEN: usize = 65;
/// the config channel's usage pages (CLASSIC_USAGE_PAGES)
pub const CONFIG_PAGES: [u16; 3] = [0xFF01, 0xFF00, 0xFFFF];
/// BATTERY_RESPONSE_DELAY_MS
pub const BATTERY_DELAY_MS: u32 = 60;

/// One id of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Product {
    pub pid: u16,
    /// name on OpenMouse's SUPPORTED list
    pub model: &'static str,
    /// a 2.4 GHz receiver id (OpenMouse reads the battery only for these)
    pub receiver: bool,
}

const fn p(pid: u16, model: &'static str, receiver: bool) -> Product {
    Product { pid, model, receiver }
}

/// OpenMouse's SUPPORTED list (supported-devices-data.ts, status "supported"), ids from `GLORIOUS_CLASSIC_PRODUCTS`:
/// Model O 2 PRO 0x2019 / 0x2033, Model O 0x2011 / 0x2022, Model D Wireless 0x2012 / 0x2023, Model D 0x2012, Model O Pro
/// 0x2015 / 0x2027. The table has 0x2012 as "Model D Wireless" (wired id) and the list names it for both Model D and Model D
/// Wireless, so it carries both names.
pub const PRODUCTS: [Product; 8] = [
    p(0x2019, "Glorious Model O 2 PRO", false),
    p(0x2033, "Glorious Model O 2 PRO", true),
    p(0x2011, "Glorious Model O", false),
    p(0x2022, "Glorious Model O", true),
    p(0x2012, "Glorious Model D / Model D Wireless", false),
    p(0x2023, "Glorious Model D Wireless", true),
    p(0x2015, "Glorious Model O Pro", false),
    p(0x2027, "Glorious Model O Pro", true),
];

pub fn product(pid: u16) -> Option<&'static Product> {
    PRODUCTS.iter().find(|p| p.pid == pid)
}

/// The polling rates OpenMouse can set on this protocol (`GLORIOUS_CLASSIC_POLLING_RATES`): interval ms -> Hz.
pub const POLLING: [(u8, u32); 4] = [(1, 1000), (2, 500), (4, 250), (8, 125)];
/// GLORIOUS_CLASSIC_LOD_MEDIUM_MM / HIGH_MM
pub const LIFT_OFF_MM: [u32; 2] = [1, 2];

fn bfr(v: &mut [u8], index: usize, value: u8) {
    v[index] = value;
}

fn packet() -> Vec<u8> {
    vec![0u8; REPORT_LEN]
}

/// `gloriousClassicEncodePollingRate`: Hz -> the interval in ms.
pub fn polling_interval(hz: u32) -> Option<u8> {
    POLLING.iter().find(|(_, h)| *h == hz).map(|(ms, _)| *ms)
}

/// `buildGloriousClassicPollingRatePayload(intervalMs)` as Windows sends it.
pub fn polling_request(interval_ms: u8) -> Vec<u8> {
    let mut v = packet();
    bfr(&mut v, 3, 0x02);
    bfr(&mut v, 4, 0x01);
    bfr(&mut v, 5, 0x01);
    bfr(&mut v, 7, interval_ms);
    v
}

/// `buildGloriousClassicLiftOffPayload(millimetres)` as Windows sends it.
pub fn lift_off_request(mm: u32) -> Vec<u8> {
    let mut v = packet();
    bfr(&mut v, 3, 0x02);
    bfr(&mut v, 4, 0x01);
    bfr(&mut v, 5, 0x01);
    bfr(&mut v, 6, 0x07);
    bfr(&mut v, 7, (mm - 1) as u8);
    v
}

/// `buildGloriousClassicBatteryRequestPayload()` as Windows sends it.
pub fn battery_request() -> Vec<u8> {
    let mut v = packet();
    bfr(&mut v, 3, 0x02);
    bfr(&mut v, 4, 0x02);
    bfr(&mut v, 6, 0x83);
    v
}

/// What the receiver says about the mouse (the status byte).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    Normal,
    Asleep,
    WakingUp,
    Unknown,
}

/// A decoded battery answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    pub state: LinkState,
    pub percent: Option<u8>,
    pub charging: bool,
}

/// `parseGloriousClassicBatteryResponse`. `reply` is Windows' buffer (report id 0 first); OpenMouse's body is `reply[1..]`.
pub fn decode_battery(reply: &[u8]) -> Battery {
    let body = reply.get(1..).unwrap_or(&[]);
    let at = |i: usize| body.get(i).copied().unwrap_or(0);
    if at(5) != 0x83 {
        return Battery { state: LinkState::Unknown, percent: None, charging: false };
    }
    let state = match at(0) {
        0xA1 => LinkState::Normal,
        0xA4 => LinkState::Asleep,
        0xA0 => LinkState::WakingUp,
        _ => LinkState::Unknown,
    };
    if state != LinkState::Normal {
        return Battery { state, percent: None, charging: false };
    }
    let raw = at(7);
    let percent = if raw == 0 { 1 } else { raw };
    Battery { state, percent: (1..=100).contains(&percent).then_some(percent), charging: at(6) == 1 }
}

/// What a model can do here (see the header: no DPI, no read-back).
pub fn caps(p: &Product) -> Caps {
    Caps { dpi: None, polling: vec![125, 250, 500, 1000], lift_off: LIFT_OFF_MM.iter().map(|m| m * 10).collect(), battery: p.receiver }
}

/// The config channel: the first collection on one of [`CONFIG_PAGES`] that has feature reports, of the 64-byte kind
/// ([`REPORT_LEN`]; OpenMouse's isConfirmedReportLength) - a unit with another length is not matched.
pub fn config_interface<'a>(own: &[&'a HidInfo]) -> Option<&'a HidInfo> {
    own.iter().find(|h| CONFIG_PAGES.contains(&h.usage_page) && h.feature_len as usize == REPORT_LEN).copied()
}

/// OpenMouse's GloriousClassicHidClient.isSupported (vendor id, a product of the table, a config report), for the ids of the
/// SUPPORTED list.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID {
        return None;
    }
    let p = product(pid)?;
    let config = config_interface(own)?;
    Some(Matched { family: Family::Glorious, model: Some(p.model), wireless: Some(p.receiver), config: config.clone(), variant: pid })
}

/// The allow-list for the real mouse: exactly the battery request (a 65-byte feature report, nothing else set). The polling
/// and lift-off writes never pass.
pub fn read_request_allowed(out: &[u8]) -> bool {
    out == battery_request()
}

/// The Glorious driver (classic protocol).
pub struct Glorious;

fn the_product(m: &Matched) -> Result<&'static Product> {
    product(m.variant).ok_or_else(|| Error::UnsupportedMouse(format!("Glorious variant {:#06x}", m.variant)))
}

impl crate::family::Driver for Glorious {
    /// OpenMouse's readStatus for the part the mouse can answer: the battery of a receiver id (a wired id sends nothing). The
    /// protocol cannot read DPI / polling / lift-off.
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let p = the_product(m)?;
        let mut r = OnMouse { online: true, wireless: Some(p.receiver), caps: caps(p), ..Default::default() };
        if !p.receiver {
            return Ok(r);
        }
        // readBattery(): SetFeature, 60 ms, GetFeature; a failure is "no battery" (readBattery().catch(() => null))
        let how = HidTransfer::Feature { reply_id: 0, reply_len: REPORT_LEN, wait_ms: BATTERY_DELAY_MS };
        match port.exchange(&battery_request(), &how) {
            Ok(a) => {
                let b = decode_battery(&a);
                if matches!(b.state, LinkState::Asleep | LinkState::WakingUp) {
                    r.online = false;
                }
                r.battery_percent = b.percent;
                r.charging = b.percent.map(|_| b.charging);
            }
            // readBattery().catch(() => null): a refused / failed battery read (an OS error too) is "no battery", not a failed read
            Err(Error::BadAnswer(_) | Error::Os { .. }) => r.online = false,
            Err(e) => return Err(e),
        }
        Ok(r)
    }

    /// OpenMouse's setPollingRate / setLiftOffDistance: the one command, nothing read back (the protocol has no read).
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        the_product(m)?;
        let req = match s {
            Setting::Polling(hz) => {
                polling_request(polling_interval(hz).ok_or_else(|| Error::range("polling rate", format!("This mouse does not support {hz} Hz")))?)
            }
            Setting::LiftOff(t) => {
                let mm = (t.is_multiple_of(10) && LIFT_OFF_MM.contains(&(t / 10))).then_some(t / 10);
                lift_off_request(mm.ok_or_else(|| Error::range("lift-off distance", format!("{t} tenths of a mm: this mouse takes 1 mm or 2 mm")))?)
            }
            Setting::Dpi(_) => {
                return Err(Error::range("DPI", "setting one DPI would overwrite this mouse's other saved DPI stages (it can't read them back) - use Glorious CORE".to_string()))
            }
        };
        port.exchange(&req, &HidTransfer::FeatureOut)?;
        Ok(Changed { now: s.value(), before: None })
    }
}
