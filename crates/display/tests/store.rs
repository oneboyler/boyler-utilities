//! Order 019: the Display tab's presets + rules file - written and read back in the lane's scratch folder only.

use bu_display::fake::FakeDisplayOs;
use bu_display::store::Store;
use bu_display::{DisplayOs, GpuScaling, MonitorId};
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    let d = PathBuf::from(r"C:\BoylerUtilities-scratch\Q\store-test").join(name);
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn presets_and_rules_survive_a_restart() {
    let dir = scratch("roundtrip");
    let path = dir.join("display.json");
    let os = FakeDisplayOs::drawing_sample();
    let modes = os.modes(&MonitorId("fake-dell".into())).unwrap();
    let mut s = Store::default();
    let p = s.presets.add(1440, 1080, 165.0, GpuScaling::Stretch, &modes).unwrap();
    let r = s.rules.add_rule("VALORANT-Win64-Shipping.exe", Some(p));
    s.rules.rule_mut(r).unwrap().vibrance = Some(80);
    s.save(&path).unwrap();
    assert!(!dir.join("display.tmp").exists(), "the temp file must be renamed over");
    let back = Store::load(&path);
    assert_eq!(back.presets, s.presets);
    assert_eq!(back.rules.rules(), s.rules.rules());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_file_is_empty_and_a_broken_one_is_kept_aside() {
    let dir = scratch("broken");
    let path = dir.join("display.json");
    let s = Store::load(&path);
    assert!(s.presets.items().is_empty() && s.rules.rules().is_empty());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&path, "{ not json").unwrap();
    let s = Store::load(&path);
    assert!(s.presets.items().is_empty());
    assert!(dir.join("display.bad").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_drawing_sample_is_the_drawings_monitors() {
    let os = FakeDisplayOs::drawing_sample();
    let m = os.monitors().unwrap();
    assert_eq!(m.len(), 2);
    assert_eq!((m[0].number, m[0].name.as_str(), m[0].is_main), (1, "DELL S2721DGF", true));
    assert_eq!((m[1].number, m[1].name.as_str(), m[1].diagonal_inches), (2, "LG 24GL600F", Some(24.0)));
    assert_eq!((m[0].current.width, m[0].current.height, m[0].current.refresh.hz()), (1920, 1080, 164.95));
}

/// Order 124: "Lock brightness" is saved per monitor (the page's monitor id) and survives a restart; a file from before
/// the lock existed loads with no locks.
#[test]
fn brightness_locks_survive_a_restart() {
    let dir = scratch("brightlock");
    let path = dir.join("display.json");
    let mut s = Store::default();
    let id = MonitorId(r"\?\DISPLAY#DELD1A8#5&abc&0&UID1#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}".into());
    s.lock_brightness(&id, Some(70));
    s.save(&path).unwrap();
    let back = Store::load(&path);
    assert_eq!(back.brightness_lock(&id), Some(70));
    assert_eq!(back.brightness_lock(&MonitorId("fake-lg".into())), None);
    assert_eq!(back.brightness_locks(), vec![(id.clone(), 70)]);
    let mut back = back;
    back.lock_brightness(&id, None);
    assert!(back.brightness_locks().is_empty());
    std::fs::write(&path, "{}").unwrap();
    assert!(Store::load(&path).brightness_locks().is_empty(), "an old file: no locks");
    assert!(!dir.join("display.bad").exists(), "an old file is not broken");
    let _ = std::fs::remove_dir_all(&dir);
}
