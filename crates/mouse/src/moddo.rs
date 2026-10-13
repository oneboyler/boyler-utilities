//! moddoMOUSE - one packed feature report, ported from OpenMouse (`crate::family`: AGPL-3.0-or-later): DPI, polling rate,
//! lift-off and battery.
//!
//! Ported from `moddo/index.js` (the codec: decodeModdoConfig / encodeModdoConfig / decodeModdoBattery / decodeModdoFirmware)
//! and `drivers/moddo/hid.js` (ModdoHidClient: readStatus / setDpi / setPollingRate / setLiftOffDistance / confirmConfig).
//!
//! Reports (OpenMouse counts the payload WITHOUT the report id; Windows puts the id in front, so its byte i is our i + 1):
//! feature report 0x02 = the settings, 11 bytes `[polling Hz u16le | lift u8 | dpiX u16le | dpiY u16le | invert/swap u8 |
//! angle i16le | deep sleep u8]` (read with HidD_GetFeature, written with HidD_SetFeature); 0x03 = firmware (bytes 0..4 the
//! dongle's version, 4..8 the mouse's - a non-zero dongle version means the answer came over the 2.4 GHz receiver); 0x04 =
//! battery (`[percent, -, status, charger flags]`, flag bit 0 = a cell is present; status 0x44 charging, 0x45 discharging,
//! 0x46 full, 0x47 empty). A change is read-modify-write of the whole settings report; the three "preserved" fields are
//! copied back from the read, and the change is read back (up to five reads, 60 ms apart - the firmware applies it a moment
//! late).
//!
//! Lift-off: raw 1 = 1 mm, raw 2 = 2 mm (OpenMouse names them Medium / High but its own error text says "1 mm (Medium) or
//! 2 mm (High)"). DPI 50-26000 in 50s, polling 125 / 250 / 500 / 1000.
//!
//! The device table has no product id: OpenMouse's isSupported is the vendor id 0x2FE3 plus a collection on usage page 0xFF
//! with usage 1 (or the legacy usage 2). 0x2FE3 is also the generic id of the Zephyr RTOS, so this port ALSO requires the
//! product or manufacturer string to say "moddo" (case-insensitive) - stricter than OpenMouse. A change rewrites the 11 known
//! bytes and zeros behind them, so it is refused when the report is shorter than 11 bytes or holds a non-zero byte behind them,
//! and a DPI change is refused when X and Y differ (both would be set). The write is confirmed on the changed field only.
//! Left out: firmware flashing, pairing, button remapping (OpenMouse has none for this family either).

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Driver, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x2FE3;
pub const MODEL: &str = "moddoMOUSE";
pub const USAGE_PAGE: u16 = 0x00FF;
pub const USAGE_CONFIG: u16 = 0x01;
pub const USAGE_CONFIG_LEGACY: u16 = 0x02;
pub const REPORT_CONFIG: u8 = 0x02;
pub const REPORT_FIRMWARE: u8 = 0x03;
pub const REPORT_BATTERY: u8 = 0x04;
pub const CONFIG_LEN: usize = 11;
/// The report id + the settings.
pub const CONFIG_REPORT_LEN: usize = CONFIG_LEN + 1;
pub const RATES_HZ: [u32; 4] = [125, 250, 500, 1000];
pub const DPI_MIN: u32 = 50;
pub const DPI_MAX: u32 = 26000;
pub const DPI_STEP: u32 = 50;
/// The lift-off distances in tenths of a millimetre (raw 1 and 2).
pub const LIFT_OFF: [u32; 2] = [10, 20];

const WRITE_SETTLE_MS: u64 = 60;
const WRITE_CONFIRM_ATTEMPTS: u32 = 5;

// ---------------------------------------------------------------- the codec

/// The settings report (decodeModdoConfig).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub polling: u16,
    pub lift: u8,
    pub dpi_x: u16,
    pub dpi_y: u16,
    pub invert_swap: u8,
    pub angle: i16,
    pub deep_sleep: u8,
}

/// decodeModdoConfig: the payload (behind the report id) → the settings, or `None` when the mouse is not ready (a truncated
/// report, or the zeroed one a sleeping wireless mouse answers with).
pub fn decode_config(p: &[u8]) -> Option<Config> {
    if p.len() < 7 {
        return None;
    }
    let u8_at = |o: usize| p.get(o).copied().unwrap_or(0);
    let c = Config {
        polling: u16::from_le_bytes([p[0], p[1]]),
        lift: p[2],
        dpi_x: u16::from_le_bytes([p[3], p[4]]),
        dpi_y: u16::from_le_bytes([p[5], p[6]]),
        invert_swap: u8_at(7),
        angle: if p.len() >= 10 { i16::from_le_bytes([p[8], p[9]]) } else { 0 },
        deep_sleep: u8_at(10),
    };
    (c.polling > 0 && (DPI_MIN..=DPI_MAX).contains(&(c.dpi_x as u32))).then_some(c)
}

/// encodeModdoConfig: the 11-byte payload.
pub fn encode_config(c: &Config) -> [u8; CONFIG_LEN] {
    let mut b = [0u8; CONFIG_LEN];
    b[0..2].copy_from_slice(&c.polling.to_le_bytes());
    b[2] = c.lift;
    b[3..5].copy_from_slice(&c.dpi_x.to_le_bytes());
    b[5..7].copy_from_slice(&c.dpi_y.to_le_bytes());
    b[7] = c.invert_swap;
    b[8..10].copy_from_slice(&c.angle.to_le_bytes());
    b[10] = c.deep_sleep;
    b
}

/// decodeModdoLift in tenths of a millimetre: 1 → 10, 2 → 20, else unknown.
pub fn lift_off_tenths(raw: u8) -> Option<u32> {
    match raw {
        1 => Some(10),
        2 => Some(20),
        _ => None,
    }
}

/// encodeModdoLift: 10 (1 mm, "Medium") → 1, 20 → 2.
pub fn lift_off_raw(tenths: u32) -> Option<u8> {
    match tenths {
        10 => Some(1),
        20 => Some(2),
        _ => None,
    }
}

/// decodeModdoBattery: (percent, charging) - `None` when there is no cell (a wired mouse) or the report is too short.
pub fn decode_battery(p: &[u8]) -> Option<(Option<u8>, bool)> {
    if p.len() < 4 || p[3] & 1 == 0 {
        return None;
    }
    Some(((p[0] <= 100).then_some(p[0]), p[2] == 0x44))
}

/// decodeModdoFirmware's `dongleConnected`: a non-zero dongle version (bytes 0..4) means the receiver answered.
pub fn dongle_connected(p: &[u8]) -> Option<bool> {
    (p.len() >= 8).then(|| p[0..4].iter().any(|b| *b != 0))
}

// ---------------------------------------------------------------- device table + allow-list

/// OpenMouse's isSupported: the vendor id and a vendor config collection (usage page 0xFF, usage 1 or the legacy 2) with a
/// feature report big enough for the settings. Usage 1 is preferred. Deliberately stricter than OpenMouse: 0x2FE3 is the
/// generic vendor id of the Zephyr RTOS, so the device's product or manufacturer string must also say "moddo".
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID {
        return None;
    }
    let says_moddo = |s: &Option<String>| s.as_deref().is_some_and(|s| s.to_ascii_lowercase().contains("moddo"));
    if !own.iter().any(|h| says_moddo(&h.product) || says_moddo(&h.manufacturer)) {
        return None;
    }
    let fits = |h: &&&HidInfo| h.usage_page == USAGE_PAGE && (h.usage == USAGE_CONFIG || h.usage == USAGE_CONFIG_LEGACY) && h.feature_len as usize >= CONFIG_REPORT_LEN;
    let c = own.iter().filter(fits).min_by_key(|h| h.usage)?;
    Some(Matched { family: Family::Moddo, model: Some(MODEL), wireless: None, config: (*c).clone(), variant: pid })
}

/// The allow-list for the read-only proof on a real mouse: NO frame passes. Every moddoMOUSE read is a HidD_GetFeature
/// (`FeatureIn`, which sends nothing and is passed by the transfer kind itself); the only frame this family ever sends is the
/// settings WRITE (a SetFeature of report 0x02). A blank frame such as `[0x02, 0, ...]` is only the `out` of a `FeatureIn`;
/// sent as a SetFeature it would store polling 0 / DPI 0, so it must not pass here.
pub fn read_request_allowed(_out: &[u8]) -> bool {
    false
}

pub fn caps() -> Caps {
    Caps { dpi: Some(DpiCaps::uniform(DPI_MIN, DPI_MAX, DPI_STEP)), polling: RATES_HZ.to_vec(), lift_off: LIFT_OFF.to_vec(), battery: true }
}

// ---------------------------------------------------------------- the exchange

fn report_len(port: &dyn Port) -> usize {
    (port.info().feature_len as usize).max(CONFIG_REPORT_LEN)
}

/// receiveFeatureReport(id): HidD_GetFeature only; the payload behind the report id.
fn get(port: &mut dyn Port, id: u8) -> Result<Vec<u8>> {
    let len = report_len(port);
    let mut out = vec![0u8; len];
    out[0] = id;
    let a = port.exchange(&out, &HidTransfer::FeatureIn { reply_id: id, reply_len: len, wait_ms: 0 })?;
    a.get(1..).map(<[u8]>::to_vec).ok_or_else(|| Error::BadAnswer("an empty feature report".into()))
}

/// `.catch(() => null)`: a failed or refused read = `None`; a mouse that is gone still fails.
fn optional<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::BadAnswer(_) | Error::Os { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// The decoded settings and the raw payload behind the report id.
fn read_config(port: &mut dyn Port) -> Result<Option<(Config, Vec<u8>)>> {
    Ok(optional(get(port, REPORT_CONFIG))?.and_then(|p| decode_config(&p).map(|c| (c, p))))
}

// ---------------------------------------------------------------- the driver

/// The moddoMOUSE driver.
pub struct Moddo;

impl Driver for Moddo {
    /// readStatus: the settings (not ready = offline), the battery, the firmware report (for the receiver flag).
    fn read(&self, _m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let mut r = OnMouse { caps: caps(), ..Default::default() };
        if let Some((c, _)) = read_config(port)? {
            r.online = true;
            r.dpi = Some(c.dpi_x as u32);
            r.polling_hz = Some(c.polling as u32);
            r.lift_off = lift_off_tenths(c.lift);
        }
        if let Some((pct, charging)) = optional(get(port, REPORT_BATTERY))?.and_then(|p| decode_battery(&p)) {
            r.battery_percent = pct;
            r.charging = Some(charging);
        }
        r.wireless = optional(get(port, REPORT_FIRMWARE))?.and_then(|p| dongle_connected(&p));
        Ok(r)
    }

    /// setDpi / setPollingRate / setLiftOffDistance: read the report, write it back with the one field changed, read it back
    /// until it shows the value.
    fn write(&self, _m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let lift_raw = match s {
            Setting::Dpi(d) if caps().dpi.is_some_and(|c| c.allows(d)) => None,
            Setting::Dpi(d) => return Err(Error::range("DPI", format!("{d} DPI is not one this moddoMOUSE can store (50 - 26000 in steps of 50)"))),
            Setting::Polling(hz) if RATES_HZ.contains(&hz) => None,
            Setting::Polling(hz) => return Err(Error::range("polling rate", format!("{hz} Hz is not one of 125, 250, 500 or 1000 Hz"))),
            Setting::LiftOff(t) => match lift_off_raw(t) {
                Some(raw) => Some(raw),
                None => return Err(Error::range("lift-off distance", format!("{t} tenths of a mm: the moddoMOUSE supports 1 mm or 2 mm"))),
            },
        };
        let Some((cur, raw)) = read_config(port)? else {
            return Err(Error::BadAnswer("the moddoMOUSE settings are not ready yet - wake the mouse and try again".into()));
        };
        // the write below carries the 11 bytes OpenMouse knows and zeros behind them: refuse a report that is shorter, or that
        // holds anything behind byte 11 (it would be wiped)
        if raw.len() < CONFIG_LEN || raw[CONFIG_LEN..].iter().any(|b| *b != 0) {
            return Err(Error::BadAnswer("the moddoMOUSE settings report holds bytes this app does not know (or is shorter than 11 bytes) - nothing was written".into()));
        }
        // a DPI change sets both axes; the "before" shows one number, so it is only changed while both are the same
        if matches!(s, Setting::Dpi(_)) && cur.dpi_x != cur.dpi_y {
            return Err(Error::BadAnswer(format!("the moddoMOUSE has different DPI for X ({}) and Y ({}) - this app sets both to one value, so nothing was written", cur.dpi_x, cur.dpi_y)));
        }
        let before = match s {
            Setting::Dpi(_) => Some(cur.dpi_x as u32),
            Setting::Polling(_) => Some(cur.polling as u32),
            Setting::LiftOff(_) => lift_off_tenths(cur.lift),
        };
        let mut next = cur;
        match s {
            Setting::Dpi(d) => {
                next.dpi_x = d as u16;
                next.dpi_y = d as u16;
            }
            Setting::Polling(h) => next.polling = h as u16,
            Setting::LiftOff(_) => next.lift = lift_raw.unwrap_or(next.lift),
        }
        let total = report_len(port);
        let mut out = vec![0u8; total];
        out[0] = REPORT_CONFIG;
        out[1..CONFIG_REPORT_LEN].copy_from_slice(&encode_config(&next));
        port.exchange(&out, &HidTransfer::FeatureOut)?;
        // confirmConfig: re-read until the CHANGED field shows the value (the firmware applies a write late; another field that
        // it normalises is not our change failing)
        let applied = |c: &Config| match s {
            Setting::Dpi(_) => c.dpi_x == next.dpi_x && c.dpi_y == next.dpi_y,
            Setting::Polling(_) => c.polling == next.polling,
            Setting::LiftOff(_) => c.lift == next.lift,
        };
        let mut seen = read_config(port)?;
        for _ in 1..WRITE_CONFIRM_ATTEMPTS {
            if seen.as_ref().is_some_and(|(c, _)| applied(c)) {
                break;
            }
            port.pause_ms(WRITE_SETTLE_MS);
            seen = read_config(port)?;
        }
        match seen {
            Some((c, _)) if applied(&c) => Ok(Changed { now: s.value(), before }),
            Some((c, _)) => Err(Error::BadAnswer(format!("the mouse kept {} DPI / {} Hz / lift-off code {} instead of the change", c.dpi_x, c.polling, c.lift))),
            None => Err(Error::BadAnswer("the moddoMOUSE settings were not readable after the change".into())),
        }
    }
}
