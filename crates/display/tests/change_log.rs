//! Order 036: the small helpers the app's change log uses (what a change was before / after, a mode and the rules as text
//! and back) - against the FAKE OS layer only.

use bu_display::autoswitch::AutoSwitcher;
use bu_display::fake::{FakeCall, FakeDisplayOs};
use bu_display::presets::PresetList;
use bu_display::*;

fn dell() -> MonitorId {
    MonitorId("fake-dell".into())
}
fn lg() -> MonitorId {
    MonitorId("fake-lg".into())
}

#[test]
fn a_ddc_change_says_its_before_and_after_and_none_when_already_there() {
    let mut s = DisplayService::new(FakeDisplayOs::two_monitors());
    let (before, after) = s.set_ddc_percent_change(&dell(), Vcp::Brightness, 90).unwrap().expect("a change");
    assert_eq!((before.current, after.current, after.max), (70, 90, before.max));
    assert_eq!(s.os().calls.last(), Some(&FakeCall::DdcSet(dell(), Vcp::Brightness, 90)));
    assert_eq!(s.set_ddc_percent_change(&dell(), Vcp::Brightness, 90).unwrap(), None);
    assert_eq!(s.set_ddc_percent_change(&lg(), Vcp::Brightness, 50), Err(DisplayError::DdcNoAnswer));
}

#[test]
fn a_vibrance_change_says_its_levels_and_none_when_already_there() {
    let mut s = DisplayService::new(FakeDisplayOs::two_monitors());
    let (before, after) = s.set_vibrance_percent_change(&lg(), 80).unwrap().expect("a change");
    assert_eq!((before.current, after.current), (0, 38));
    assert_eq!((after.min, after.max, after.default), (before.min, before.max, before.default));
    assert_eq!(s.set_vibrance_percent_change(&lg(), 80).unwrap(), None);
}

#[test]
fn a_mode_goes_to_text_and_back() {
    let m = Mode { width: 1440, height: 1080, refresh: RefreshRate::new(164_950, 1000), scaling: GpuScaling::Stretch };
    let raw = m.to_raw();
    assert_eq!(Mode::from_raw(&raw), Some(m));
    assert_eq!(Mode::from_raw("not a mode"), None);
}

#[test]
fn the_rules_go_to_text_and_back_and_a_gone_preset_is_dropped() {
    let svc = DisplayService::new(FakeDisplayOs::two_monitors());
    let dm = svc.modes(&dell()).unwrap();
    let mut presets = PresetList::new();
    let p1 = presets.add(1440, 1080, 165.0, GpuScaling::Stretch, &dm).unwrap();
    let p2 = presets.add(1280, 960, 144.0, GpuScaling::BlackBars, &dm).unwrap();
    let mut sw = AutoSwitcher::new();
    assert_eq!(sw.rules_raw(), "[]");
    sw.add_rule("cs2.exe", Some(p1));
    sw.add_rule("r5apex.exe", Some(p2));
    let raw = sw.rules_raw();
    let mut back = AutoSwitcher::new();
    back.set_rules_raw(&raw, &presets).unwrap();
    assert_eq!(back.rules(), sw.rules());
    // a new row after a restore gets a fresh id
    let id = back.add_rule("x.exe", None);
    assert!(back.rules().iter().filter(|r| r.id == id).count() == 1 && id.0 > 2);
    // the preset of a restored row was deleted meanwhile: "Choose a preset"
    presets.remove(p2).unwrap();
    let mut b2 = AutoSwitcher::new();
    b2.set_rules_raw(&raw, &presets).unwrap();
    assert_eq!(b2.rules()[1].preset, None);
    assert_eq!(b2.rules()[0].preset, Some(p1));
    // no rows
    b2.set_rules_raw("[]", &presets).unwrap();
    assert!(b2.rules().is_empty());
    assert!(b2.set_rules_raw("garbage", &presets).is_err());
}
