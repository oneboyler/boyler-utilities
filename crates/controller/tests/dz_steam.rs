//! Order 122: a stick's dead zone numbers = the numbers Steam's Deadzones panel shows. Steam's settings table
//! (steamclient64.dll, read Oct 24 read-only) declares a joystick's `deadzone_inner_radius` / `deadzone_outer_radius` 0–32000
//! and its panel shows `round(value / 32000 × 100)`; Steam's own values when the file has no line: 0 / 28000, Circle,
//! "this controller's own". Round trips run on scratch copies (his real Rocket League layout is only READ, then copied).
#![cfg(windows)]

mod common;

use bu_controller::layout::Layout;
use bu_controller::settings::*;
use bu_controller::{PadKind, Side};
use common::*;
use std::path::{Path, PathBuf};

const SCRATCH: &str = r"C:\BoylerUtilities-scratch\122";
const REAL_RL: &str = r"C:\Program Files (x86)\Steam\steamapps\common\Steam Controller Configs\890771142\config\252950\controller_ps5.vdf";

/// What Steam's panel shows for a file value (its compound slider: `Math.round((v - min) / (max - min) * 100)`, min 0).
fn steam_shows(raw: i64, max: i64) -> i64 {
    (raw as f64 / max as f64 * 100.0 + 0.5).floor() as i64
}

#[test]
fn the_scale_and_defaults_are_steams() {
    for m in [StickMode::Joystick, StickMode::Camera, StickMode::Mouse] {
        assert_eq!(stick_dz_max(&m), 32000);
        assert_eq!(stick_dz_default(&m), (0, 28000));
    }
    assert_eq!(stick_dz_max(&StickMode::FlickStick), 80);
    // the friend's screenshot: our 86 % was Steam's 88 % (one file value, two scales)
    let raw = pct_to_radius(86.0);
    assert_eq!(radius_to_pct(raw).round(), 86.0, "the old 0-32767 scale");
    assert_eq!(steam_shows(raw, 32000), 88, "what Steam showed");
    assert_eq!(dz_to_pct(raw, 32000).round(), 88.0, "what we show now");
    // Steam's own full at (no line): 28000 = 88 %
    assert_eq!(steam_shows(28000, 32000), 88);
    assert_eq!((DZ_SHAPE_DEFAULT, DZ_SOURCE_DEFAULT), (1, 1));
    // the controller's own dead zone when its file says -1 (Steam's per-type default)
    assert_eq!(own_dz_default(PadKind::DualShock4), 4096);
    for k in [PadKind::DualSense, PadKind::DualSenseEdge, PadKind::Xbox] {
        assert_eq!(own_dz_default(k), 10000);
    }
}

#[test]
fn every_percent_shows_the_same_in_steam() {
    for p in 0..=100 {
        let raw = pct_to_dz(p as f64, 32000);
        assert_eq!(steam_shows(raw, 32000), p, "{p} % -> {raw}");
        assert_eq!(dz_to_pct(raw, 32000).round() as i64, p);
    }
    // and every file value reads as the number Steam shows
    for raw in (0..=32000).step_by(7) {
        assert_eq!(dz_to_pct(raw, 32000).round() as i64, steam_shows(raw, 32000), "{raw}");
    }
}

fn scratch(name: &str) -> Option<PathBuf> {
    let root = Path::new(SCRATCH);
    if std::fs::create_dir_all(root).is_err() {
        eprintln!("scratch folder {SCRATCH} not possible: skipped");
        return None;
    }
    let d = root.join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Some(d)
}

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Write each value into a scratch copy of `text` (on disk), read the file back, and check Steam's panel number.
fn round_trip(text: &str, dir: &Path) {
    let file = dir.join("controller_ps5.vdf");
    for side in [Side::Left, Side::Right] {
        for shape in [1, 0, 2] {
            for p in [0.0, 8.0, 16.0, 50.0, 88.0, 100.0] {
                std::fs::write(&file, text).unwrap();
                let mut l = Layout::parse(std::fs::read_to_string(&file).unwrap()).unwrap();
                let max = l.pad_view(0, PadKind::DualSense).sticks.iter().find(|s| s.side == side).map(|s| stick_dz_max(&s.mode)).unwrap();
                assert_eq!(max, 32000, "{side:?} is a joystick in this layout");
                for (setting, value) in [
                    (StickSetting::DeadZone, Some(pct_to_dz(p, max))),
                    (StickSetting::FullAt, Some(pct_to_dz(100.0 - p / 2.0, max))),
                    (StickSetting::DeadZoneShape, Some(shape)),
                    (StickSetting::DeadZoneSource, Some(2)),
                ] {
                    l.apply(0, &Change::StickSetting { side, setting, value }).unwrap();
                }
                std::fs::write(&file, l.text()).unwrap();
                let back = Layout::parse(std::fs::read_to_string(&file).unwrap()).unwrap();
                let v = back.pad_view(0, PadKind::DualSense);
                let st = v.sticks.iter().find(|s| s.side == side).unwrap();
                let g = |k| st.settings.iter().find(|(x, _)| *x == k).and_then(|(_, v)| *v);
                assert_eq!(steam_shows(g(StickSetting::DeadZone).unwrap(), max), p as i64, "{side:?} shape {shape} dead zone {p}");
                assert_eq!(steam_shows(g(StickSetting::FullAt).unwrap(), max), (100.0 - p / 2.0) as i64, "{side:?} shape {shape} full at");
                assert_eq!(dz_to_pct(g(StickSetting::DeadZone).unwrap(), max).round(), p);
                assert_eq!(g(StickSetting::DeadZoneShape), Some(shape));
                assert_eq!(g(StickSetting::DeadZoneSource), Some(2));
            }
        }
    }
}

#[test]
fn round_trip_on_a_scratch_copy_of_the_test_layout() {
    let Some(d) = scratch("fixture") else { return };
    let _c = Cleanup(d.clone());
    round_trip(RL, &d);
}

#[test]
fn round_trip_on_a_scratch_copy_of_his_real_rocket_league_layout() {
    // READ only; every write goes to the scratch copy
    let Ok(real) = std::fs::read_to_string(REAL_RL) else {
        eprintln!("{REAL_RL} not on this PC: skipped");
        return;
    };
    let Some(d) = scratch("real") else { return };
    let _c = Cleanup(d.clone());
    // what Steam's panel shows for his file now (read, Oct 24: left 5898 / 25602 = 18 / 80 %, right 3395 = 11 %, full at
    // Steam's own 28000 = 88 %) - printed for the report, the file may have changed since
    let l = Layout::parse(real.clone()).unwrap();
    for st in &l.pad_view(0, PadKind::DualSense).sticks {
        let g = |k| st.settings.iter().find(|(x, _)| *x == k).and_then(|(_, v)| *v);
        let max = stick_dz_max(&st.mode);
        let (di, do_) = stick_dz_default(&st.mode);
        eprintln!(
            "{:?} {:?}: dead zone {:?} -> Steam {} %, full at {:?} -> Steam {} %, shape {:?}, source {:?}",
            st.side,
            st.mode,
            g(StickSetting::DeadZone),
            steam_shows(g(StickSetting::DeadZone).unwrap_or(di), max),
            g(StickSetting::FullAt),
            steam_shows(g(StickSetting::FullAt).unwrap_or(do_), max),
            g(StickSetting::DeadZoneShape),
            g(StickSetting::DeadZoneSource)
        );
    }
    round_trip(&real, &d);
    assert_eq!(std::fs::read_to_string(REAL_RL).unwrap(), real, "his real file is untouched");
}
