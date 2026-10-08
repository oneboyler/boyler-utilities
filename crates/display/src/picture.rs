//! Picture group maths: vibrance % ↔ driver level, and the DDC/CI crash guard.
//!
//! Vibrance % follows the NVIDIA Control Panel: 50 % = the driver's normal level, 100 % = its maximum, 0 % = its minimum
//! (greyscale). Measured from vibranceGUI's own table (NvidiaVibranceValueWrapper.cs: levels 0..63 ↔ 50..100 %), which is
//! exactly `50 + round(level × 50 / 63)`; the same formula with the driver's min/default/max covers the "Ex" range
//! below 50 % and AMD's saturation (ADL: default 100 of 0..200 on typical drivers — read from the driver, not assumed).

use crate::types::{MonitorId, VibranceRaw};
use std::fs;
use std::path::{Path, PathBuf};

/// Driver level → % (0..=100, 50 = normal).
pub fn vibrance_percent(raw: &VibranceRaw) -> u8 {
    let (cur, min, max, def) = (raw.current as f64, raw.min as f64, raw.max as f64, raw.default as f64);
    let p = if cur >= def {
        if max > def { 50.0 + (cur - def) * 50.0 / (max - def) } else { 50.0 }
    } else if def > min {
        50.0 - (def - cur) * 50.0 / (def - min)
    } else {
        50.0
    };
    p.round().clamp(0.0, 100.0) as u8
}

/// % → driver level (rounded, clamped to what the driver allows). Below 50 % on a driver whose minimum is its normal
/// level (old NVIDIA DVC API: 0..63) gives the normal level.
pub fn vibrance_level(raw: &VibranceRaw, percent: u8) -> i32 {
    let p = percent.min(100) as f64;
    let (min, max, def) = (raw.min as f64, raw.max as f64, raw.default as f64);
    let lvl = if p >= 50.0 { def + ((p - 50.0) * (max - def) / 50.0).round() } else { def - ((50.0 - p) * (def - min) / 50.0).round() };
    clamp_any(lvl as i32, raw.min, raw.max)
}

/// Slider % → the monitor's own DDC value (0..max).
pub fn ddc_value_for_percent(max: u32, percent: u8) -> u32 {
    ((percent.min(100) as f64) * max as f64 / 100.0).round() as u32
}

/// DDC/CI crash guard. Research (big-B §1, PowerToys Power Display docs): some monitors blue-screen the PC when asked
/// over DDC/CI. Before every DDC call a marker file `<dir>\ddc-<monitor>.inflight` is written and removed after; if the
/// app (or Windows) died during a call, the marker is still there at the next start and DDC stays OFF for that monitor
/// until the user clears it. We also never read the monitor's capabilities string (the call the warning is about) —
/// only plain VCP get/set of 0x10 / 0x12.
#[derive(Clone, Debug)]
pub struct DdcCrashGuard {
    dir: Option<PathBuf>,
}

impl DdcCrashGuard {
    /// `dir = None` disables the guard (tests / no settings folder yet).
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self { dir }
    }

    fn marker(&self, monitor_key: &str) -> Option<PathBuf> {
        let safe: String = monitor_key.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        // Keep file names short: the tail of a device path holds the unique part.
        let tail: String = safe.chars().rev().take(80).collect::<Vec<_>>().into_iter().rev().collect();
        self.dir.as_ref().map(|d| d.join(format!("ddc-{tail}.inflight")))
    }

    /// True when a previous DDC call on this monitor never finished (DDC stays off for it).
    pub fn is_blocked(&self, monitor_key: &str) -> bool {
        self.marker(monitor_key).map(|p| p.exists()).unwrap_or(false)
    }

    /// Runs one DDC call between "in flight" marker write and removal.
    pub fn run<T>(&self, monitor_key: &str, f: impl FnOnce() -> T) -> T {
        let m = self.marker(monitor_key);
        if let Some(p) = &m {
            if let Some(parent) = p.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(p, b"ddc call in flight");
        }
        let out = f();
        if let Some(p) = &m {
            let _ = fs::remove_file(p);
        }
        out
    }

    /// The user turns DDC back on for a monitor after a crash.
    pub fn clear(&self, monitor_key: &str) {
        if let Some(p) = self.marker(monitor_key) {
            let _ = fs::remove_file(p);
        }
    }

    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }
}

/// `clamp` that never panics on driver / OS values: if `a > b` they are swapped first.
pub fn clamp_any(v: i32, a: i32, b: i32) -> i32 {
    v.clamp(a.min(b), a.max(b))
}

/// PowerToys Power Display's built-in DDC/CI exclusion list, copied (MIT, © Microsoft) from
/// `microsoft/PowerToys` `src/modules/powerdisplay/PowerDisplay.Models/BuiltInMonitorBlacklist.json` (commit 4edfcee,
/// 2026-05-22): monitors whose DDC/CI capabilities read blue-screened Windows (PowerToys issues #47556 LTM2C02,
/// #47968 GSM7714). We never read the capabilities string, but skip DDC/CI on these models completely.
pub const DDC_EXCLUDED_EDID_IDS: &[&str] = &["LTM2C02", "GSM7714"];

/// The EDID id (PnP maker + product code, e.g. "DELD1A8") from a monitor's device path
/// `\\?\DISPLAY#DELD1A8#5&abc&0&UID1#{…}` — the same rule as PowerToys' `MonitorIdentity.EdidIdFromMonitorId`.
pub fn edid_id(id: &MonitorId) -> Option<&str> {
    let mut parts = id.0.split('#');
    let _ = parts.next()?;
    let edid = parts.next()?;
    parts.next()?;
    if edid.is_empty() { None } else { Some(edid) }
}

/// True when DDC/CI must never be used on this monitor (it is on the exclusion list).
pub fn ddc_excluded(id: &MonitorId) -> bool {
    edid_id(id).is_some_and(|e| DDC_EXCLUDED_EDID_IDS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}
