//! "This controller · all games": Steam's per-controller file `preferences_<serial>.vdf` (KeyValues text).
//!
//! Keys measured in the local files (2026-10-08): `name`, `stick_left_deadzone` / `stick_right_deadzone` (-1 = Steam's
//! default; else Steam's 0–32767 radius, e.g. 4000 ≈ 12 %), `antidrift_enabled_sw` (0/1), `gyro_stationary_noise_tolerance`
//! (0.5 everywhere = Steam's default), `rumble` (-1 / 1), `color_red` / `color_green` / `color_blue` (0–255) +
//! `color_saturation`, `guide_brightness` (0–1). Edited like the layouts: only the changed value's bytes move.

use crate::vdf::{Doc, EditError, ParseError};

/// One per-controller setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrefSetting {
    LeftStickDeadZone,
    RightStickDeadZone,
    AntiDrift,
    GyroNoiseFilter,
    Rumble,
    LedRed,
    LedGreen,
    LedBlue,
    LedBrightness,
}

impl PrefSetting {
    pub const ALL: [PrefSetting; 9] = [
        PrefSetting::LeftStickDeadZone,
        PrefSetting::RightStickDeadZone,
        PrefSetting::AntiDrift,
        PrefSetting::GyroNoiseFilter,
        PrefSetting::Rumble,
        PrefSetting::LedRed,
        PrefSetting::LedGreen,
        PrefSetting::LedBlue,
        PrefSetting::LedBrightness,
    ];
    pub fn key(self) -> &'static str {
        match self {
            PrefSetting::LeftStickDeadZone => "stick_left_deadzone",
            PrefSetting::RightStickDeadZone => "stick_right_deadzone",
            PrefSetting::AntiDrift => "antidrift_enabled_sw",
            PrefSetting::GyroNoiseFilter => "gyro_stationary_noise_tolerance",
            PrefSetting::Rumble => "rumble", // -1 = Steam's default, 1 on (measured); 0 = off is a GUESS
            PrefSetting::LedRed => "color_red",
            PrefSetting::LedGreen => "color_green",
            PrefSetting::LedBlue => "color_blue",
            PrefSetting::LedBrightness => "guide_brightness", // GUESS: Steam's name says "guide" (Steam Controller); used for the light
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            PrefSetting::LeftStickDeadZone => "Left stick dead zone",
            PrefSetting::RightStickDeadZone => "Right stick dead zone",
            PrefSetting::AntiDrift => "Anti-drift",
            PrefSetting::GyroNoiseFilter => "Gyro noise filter",
            PrefSetting::Rumble => "Rumble",
            PrefSetting::LedRed => "Light colour (red)",
            PrefSetting::LedGreen => "Light colour (green)",
            PrefSetting::LedBlue => "Light colour (blue)",
            PrefSetting::LedBrightness => "Light brightness",
        }
    }
}

/// One controller's file, read.
#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    pub serial: String,
    /// What Steam calls the controller (e.g. "DualSense Edge Wireless Controller").
    pub name: String,
    /// Each setting's raw text (`None` = not in the file); numbers stay text so `0.5` survives.
    pub values: Vec<(PrefSetting, Option<String>)>,
}

impl Prefs {
    pub fn parse(serial: &str, text: &str) -> Result<Prefs, ParseError> {
        let doc = Doc::parse(text)?;
        let top = doc.top();
        let get = |k: &str| top.and_then(|t| t.child_value(k)).map(str::to_string);
        Ok(Prefs {
            serial: serial.to_string(),
            name: get("name").unwrap_or_default(),
            values: PrefSetting::ALL.into_iter().map(|s| (s, get(s.key()))).collect(),
        })
    }

    pub fn get(&self, s: PrefSetting) -> Option<&str> {
        self.values.iter().find(|(k, _)| *k == s).and_then(|(_, v)| v.as_deref())
    }

    /// A stick dead zone in % (`None` = Steam's default, -1).
    pub fn stick_deadzone_pct(&self, s: PrefSetting) -> Option<f64> {
        let v: i64 = self.get(s)?.trim().parse().ok()?;
        (v >= 0).then(|| crate::settings::radius_to_pct(v))
    }

    /// The light colour (r, g, b).
    pub fn led(&self) -> Option<(u8, u8, u8)> {
        let c = |s| self.get(s).and_then(|v| v.trim().parse::<u8>().ok());
        Some((c(PrefSetting::LedRed)?, c(PrefSetting::LedGreen)?, c(PrefSetting::LedBlue)?))
    }
}

/// Set (Some) or remove (None) one value in a preferences file's text; every other byte is kept.
pub fn set(text: &str, s: PrefSetting, value: Option<&str>) -> Result<String, EditError> {
    let mut doc = Doc::parse(text)?;
    match value {
        Some(v) => doc.upsert(&[0], s.key(), v)?,
        None => {
            if let Some(a) = doc.find(&[0], s.key()) {
                doc.remove(&a)?;
            }
        }
    }
    Ok(doc.into_text())
}

/// The Gyro noise filter as the drawing shows it (Low / Medium / High). Steam stores a number (0.5 on all measured pads =
/// Steam's default); which numbers Steam's own Low / High use is UNCLEAR — these steps are a guess (half / double).
pub const NOISE_STEPS: &[(&str, &str)] = &[("Low", "0.25"), ("Medium", "0.5"), ("High", "1")];

#[cfg(test)]
mod tests {
    use super::*;
    const P: &str = "\"ControllerPersonalization\"\n{\n\t\"name\"\t\t\"DualSense Wireless Controller\"\n\t\"guide_brightness\"\t\t\"0.5\"\n\t\"antidrift_enabled_sw\"\t\t\"0\"\n\t\"rumble\"\t\t\"1\"\n\t\"color_red\"\t\t\"10\"\n\t\"color_green\"\t\t\"20\"\n\t\"color_blue\"\t\t\"30\"\n\t\"stick_left_deadzone\"\t\t\"4000\"\n\t\"stick_right_deadzone\"\t\t\"-1\"\n}\n";

    #[test]
    fn reads_and_writes_one_value() {
        let p = Prefs::parse("DS000000000002", P).unwrap();
        assert_eq!(p.name, "DualSense Wireless Controller");
        assert_eq!(p.led(), Some((10, 20, 30)));
        assert!((p.stick_deadzone_pct(PrefSetting::LeftStickDeadZone).unwrap() - 12.207).abs() < 0.01);
        assert_eq!(p.stick_deadzone_pct(PrefSetting::RightStickDeadZone), None);
        let t = set(P, PrefSetting::AntiDrift, Some("1")).unwrap();
        assert_eq!(t, P.replace("\"antidrift_enabled_sw\"\t\t\"0\"", "\"antidrift_enabled_sw\"\t\t\"1\""));
        let t2 = set(P, PrefSetting::GyroNoiseFilter, Some("0.5")).unwrap();
        assert!(t2.ends_with("\t\"gyro_stationary_noise_tolerance\"\t\t\"0.5\"\n}\n"));
        assert_eq!(set(&t2, PrefSetting::GyroNoiseFilter, None).unwrap(), P);
    }
}
