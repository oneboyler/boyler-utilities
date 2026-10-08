//! Per-app automatic switching (DESIGN §3.2.5) against the fake: app starts → preset (+ vibrance) on its monitor;
//! app stops → everything back.

use bu_display::autoswitch::{exe_matches, AppEvent, AutoSwitcher, SwitchAction, SwitchNote};
use bu_display::fake::FakeDisplayOs;
use bu_display::presets::PresetList;
use bu_display::*;

fn dell() -> MonitorId {
    MonitorId("fake-dell".into())
}
fn lg() -> MonitorId {
    MonitorId("fake-lg".into())
}
const VAL: &str = r"C:\Riot Games\VALORANT\live\ShooterGame\Binaries\Win64\VALORANT-Win64-Shipping.exe";

struct Setup {
    svc: DisplayService<FakeDisplayOs>,
    presets: PresetList,
    sw: AutoSwitcher,
    stretch: presets::PresetId,
    black: presets::PresetId,
}

fn setup() -> Setup {
    let svc = DisplayService::new(FakeDisplayOs::two_monitors());
    let mut presets = PresetList::new();
    let dm = svc.modes(&dell()).unwrap();
    let stretch = presets.add(1440, 1080, 165.0, GpuScaling::Stretch, &dm).unwrap();
    let black = presets.add(1280, 960, 144.0, GpuScaling::BlackBars, &dm).unwrap();
    let mut s = Setup { svc, presets, sw: AutoSwitcher::new(), stretch, black };
    // The real flow: each preset applied by hand and KEPT once on each monitor (then put back), so rules may use it.
    for m in [dell(), lg()] {
        for p in [stretch, black] {
            keep_once(&mut s, &m, p);
        }
    }
    s
}

fn keep_once(s: &mut Setup, m: &MonitorId, p: presets::PresetId) {
    let preset = s.presets.get(p).unwrap().clone();
    s.svc.apply_preset(m, &preset, std::time::Instant::now()).unwrap();
    for (mon, mode) in s.svc.keep().unwrap() {
        let modes = s.svc.modes(&mon).unwrap();
        assert_eq!(s.presets.note_kept(&mon, &mode, &modes), 1);
    }
    s.svc.undo().unwrap();
    s.svc.os_mut().calls.clear();
}

fn started(pid: u32, exe: &str, m: MonitorId) -> AppEvent {
    AppEvent::Started { pid, exe: exe.into(), monitor: m, has_window: false }
}

#[test]
fn app_start_applies_preset_and_vibrance_stop_restores_both() {
    let mut s = setup();
    let id = s.sw.add_rule("VALORANT-Win64-Shipping.exe", Some(s.stretch));
    s.sw.rule_mut(id).unwrap().vibrance = Some(80);
    let before = s.svc.os().current(&dell());

    let acts = s.sw.run(&started(100, VAL, dell()), &s.presets, &mut s.svc).unwrap();
    assert_eq!(acts.len(), 2);
    let now = s.svc.os().current(&dell());
    assert_eq!((now.width, now.height, now.scaling), (1440, 1080, GpuScaling::Stretch));
    assert_eq!(s.svc.picture(&dell()).unwrap().vibrance_percent, Some(80));
    assert!(s.svc.pending().is_none(), "automatic switches have no keep bar");
    assert!(!s.svc.can_undo(), "automatic switches are not undo steps");
    // The other monitor is untouched.
    assert_eq!(s.svc.os().current(&lg()).width, 1920);

    s.sw.run(&AppEvent::Stopped { pid: 100 }, &s.presets, &mut s.svc).unwrap();
    assert_eq!(s.svc.os().current(&dell()), before);
    assert_eq!(s.svc.picture(&dell()).unwrap().vibrance_percent, Some(50));
}

#[test]
fn rule_without_vibrance_leaves_vibrance_alone() {
    let mut s = setup();
    s.sw.add_rule("cs2.exe", Some(s.black));
    let acts = s.sw.run(&started(7, "cs2.exe", lg()), &s.presets, &mut s.svc).unwrap();
    assert_eq!(acts, vec![SwitchAction::ApplyPreset { monitor: lg(), preset: s.black }]);
    let back = s.sw.run(&AppEvent::Stopped { pid: 7 }, &s.presets, &mut s.svc).unwrap();
    assert!(matches!(back.as_slice(), [SwitchAction::RestoreMode { .. }]));
    assert_eq!(s.svc.os().current(&lg()).width, 1920);
}

#[test]
fn two_matching_apps_last_started_wins_then_hands_back() {
    let mut s = setup();
    s.sw.add_rule("a.exe", Some(s.stretch));
    let b = s.sw.add_rule("b.exe", Some(s.black));
    s.sw.rule_mut(b).unwrap().vibrance = Some(100);
    let original = s.svc.os().current(&dell());
    s.sw.run(&started(1, "a.exe", dell()), &s.presets, &mut s.svc).unwrap();
    s.sw.run(&started(2, "b.exe", dell()), &s.presets, &mut s.svc).unwrap();
    assert_eq!(s.svc.os().current(&dell()).width, 1280);
    assert_eq!(s.svc.picture(&dell()).unwrap().vibrance_percent, Some(100));
    // b closes → a's preset again, vibrance back to the saved one (a changes none).
    s.sw.run(&AppEvent::Stopped { pid: 2 }, &s.presets, &mut s.svc).unwrap();
    assert_eq!(s.svc.os().current(&dell()).width, 1440);
    assert_eq!(s.svc.picture(&dell()).unwrap().vibrance_percent, Some(50));
    // a closes → the mode from before the first app.
    s.sw.run(&AppEvent::Stopped { pid: 1 }, &s.presets, &mut s.svc).unwrap();
    assert_eq!(s.svc.os().current(&dell()), original);
}

#[test]
fn first_app_closing_while_second_runs_changes_nothing() {
    let mut s = setup();
    s.sw.add_rule("a.exe", Some(s.stretch));
    s.sw.add_rule("b.exe", Some(s.black));
    let original = s.svc.os().current(&dell());
    s.sw.run(&started(1, "a.exe", dell()), &s.presets, &mut s.svc).unwrap();
    s.sw.run(&started(2, "b.exe", dell()), &s.presets, &mut s.svc).unwrap();
    let acts = s.sw.run(&AppEvent::Stopped { pid: 1 }, &s.presets, &mut s.svc).unwrap();
    assert!(acts.is_empty());
    assert_eq!(s.svc.os().current(&dell()).width, 1280);
    s.sw.run(&AppEvent::Stopped { pid: 2 }, &s.presets, &mut s.svc).unwrap();
    assert_eq!(s.svc.os().current(&dell()), original);
}

#[test]
fn switched_off_rule_deleted_preset_and_unknown_apps_do_nothing() {
    let mut s = setup();
    let id = s.sw.add_rule("a.exe", Some(s.stretch));
    s.sw.rule_mut(id).unwrap().enabled = false;
    assert!(s.sw.run(&started(1, "a.exe", dell()), &s.presets, &mut s.svc).unwrap().is_empty());
    s.sw.rule_mut(id).unwrap().enabled = true;
    // Deleted preset → the row falls back to "Choose a preset" and does nothing.
    s.presets.remove(s.stretch).unwrap();
    s.sw.forget_preset(s.stretch);
    assert_eq!(s.sw.rules()[0].preset, None);
    assert!(s.sw.run(&started(1, "a.exe", dell()), &s.presets, &mut s.svc).unwrap().is_empty());
    assert!(s.sw.run(&started(2, "other.exe", dell()), &s.presets, &mut s.svc).unwrap().is_empty());
    assert!(s.sw.run(&AppEvent::Stopped { pid: 99 }, &s.presets, &mut s.svc).unwrap().is_empty());
    assert!(s.svc.os().calls.is_empty());
}

#[test]
fn same_pid_twice_is_one_start() {
    let mut s = setup();
    s.sw.add_rule("a.exe", Some(s.stretch));
    s.sw.run(&started(1, "a.exe", dell()), &s.presets, &mut s.svc).unwrap();
    assert!(s.sw.run(&started(1, "a.exe", dell()), &s.presets, &mut s.svc).unwrap().is_empty());
}

#[test]
fn removed_rule_still_switches_back() {
    let mut s = setup();
    let id = s.sw.add_rule("a.exe", Some(s.stretch));
    let original = s.svc.os().current(&dell());
    s.sw.run(&started(1, "a.exe", dell()), &s.presets, &mut s.svc).unwrap();
    s.sw.remove_rule(id).unwrap();
    s.sw.run(&AppEvent::Stopped { pid: 1 }, &s.presets, &mut s.svc).unwrap();
    assert_eq!(s.svc.os().current(&dell()), original);
}

#[test]
fn preset_hz_snaps_to_the_apps_monitor() {
    let mut s = setup();
    s.sw.add_rule("a.exe", Some(s.stretch)); // saved at 164.95 on the DELL
    s.sw.run(&started(1, "a.exe", lg()), &s.presets, &mut s.svc).unwrap();
    assert_eq!(s.svc.os().current(&lg()).refresh, RefreshRate::new(143_981, 1000));
}

#[test]
fn exe_matching_rules() {
    assert!(exe_matches("VALORANT-Win64-Shipping.exe", VAL));
    assert!(exe_matches(VAL, &VAL.to_lowercase()));
    assert!(exe_matches(VAL, "valorant-win64-shipping.exe"));
    assert!(!exe_matches(r"D:\Games\Other\VALORANT-Win64-Shipping.exe", VAL));
    assert!(!exe_matches("cs2.exe", VAL));
}

#[test]
fn watched_names_only_live_rules() {
    let mut s = setup();
    s.sw.add_rule(VAL, Some(s.stretch));
    let off = s.sw.add_rule("off.exe", Some(s.stretch));
    s.sw.rule_mut(off).unwrap().enabled = false;
    s.sw.add_rule("nopreset.exe", None);
    assert_eq!(s.sw.watched_names(), vec!["valorant-win64-shipping.exe".to_string()]);
}

#[test]
fn vibrance_chip_choices() {
    assert_eq!(autoswitch::VIBRANCE_CHOICES, [60, 70, 80, 90, 100]);
}

// ---------- NOTE_004_01 rules ----------

#[test]
fn late_start_with_a_window_is_not_switched_and_leaves_a_note() {
    let mut s = setup();
    let id = s.sw.add_rule("VALORANT-Win64-Shipping.exe", Some(s.stretch));
    let before = s.svc.os().current(&dell());
    let ev = AppEvent::Started { pid: 5, exe: VAL.into(), monitor: dell(), has_window: true };
    assert!(s.sw.run(&ev, &s.presets, &mut s.svc).unwrap().is_empty());
    assert_eq!(s.svc.os().current(&dell()), before);
    assert!(s.svc.os().calls.is_empty());
    assert_eq!(s.sw.take_notes(), vec![SwitchNote::TooLate { rule: id, exe: "VALORANT-Win64-Shipping.exe".into() }]);
    // Its exit changes nothing either (it never switched).
    assert!(s.sw.run(&AppEvent::Stopped { pid: 5 }, &s.presets, &mut s.svc).unwrap().is_empty());
    assert!(s.sw.take_notes().is_empty());
}

#[test]
fn preset_never_kept_on_that_monitor_is_not_used_automatically() {
    let mut s = setup();
    let dm = s.svc.modes(&dell()).unwrap();
    let fresh = s.presets.add(1920, 1080, 144.0, GpuScaling::KeepAspect, &dm).unwrap();
    let id = s.sw.add_rule("a.exe", Some(fresh));
    assert!(s.sw.run(&started(1, "a.exe", dell()), &s.presets, &mut s.svc).unwrap().is_empty());
    assert!(s.svc.os().calls.is_empty());
    assert_eq!(s.sw.take_notes(), vec![SwitchNote::PresetNotKept { rule: id, preset: fresh, monitor: dell() }]);
    // Kept once by hand → now it is used.
    keep_once(&mut s, &dell(), fresh);
    assert_eq!(s.sw.run(&started(2, "a.exe", dell()), &s.presets, &mut s.svc).unwrap().len(), 1);
    assert_eq!(s.svc.os().current(&dell()).width, 1920);
    // Kept on the DELL only: not on the LG.
    assert!(!s.presets.is_kept_on(fresh, &lg()));
}

#[test]
fn failed_switch_reverts_silently_and_leaves_a_note() {
    let mut s = setup();
    let id = s.sw.add_rule("a.exe", Some(s.stretch));
    s.sw.rule_mut(id).unwrap().vibrance = Some(90);
    let before = s.svc.os().current(&dell());
    s.svc.os_mut().fail_next_change = Some("the driver refused the mode".into());
    let done = s.sw.run(&started(1, "a.exe", dell()), &s.presets, &mut s.svc).unwrap();
    assert!(done.is_empty());
    assert_eq!(s.svc.os().current(&dell()), before);
    assert_eq!(s.svc.picture(&dell()).unwrap().vibrance_percent, Some(50));
    assert!(s.svc.pending().is_none(), "never a keep bar / prompt");
    match s.sw.take_notes().as_slice() {
        [SwitchNote::SwitchFailed { rule, monitor, detail }] => {
            assert_eq!((*rule, monitor), (id, &dell()));
            assert!(detail.contains("refused"));
        }
        other => panic!("{other:?}"),
    }
    // The app is no longer tracked: its exit does nothing.
    assert!(s.sw.run(&AppEvent::Stopped { pid: 1 }, &s.presets, &mut s.svc).unwrap().is_empty());
}

#[test]
fn vibrance_failure_after_mode_switch_puts_the_mode_back_too() {
    let mut s = setup();
    let id = s.sw.add_rule("a.exe", Some(s.stretch));
    s.sw.rule_mut(id).unwrap().vibrance = Some(90);
    let before = s.svc.os().current(&dell());
    s.svc.os_mut().admin_required.insert(ChangeKind::Vibrance); // vibrance call fails, mode call works
    s.sw.run(&started(1, "a.exe", dell()), &s.presets, &mut s.svc).unwrap();
    assert_eq!(s.svc.os().current(&dell()), before);
    assert!(matches!(s.sw.take_notes().as_slice(), [SwitchNote::SwitchFailed { .. }]));
}

#[test]
fn automatic_switch_never_starts_the_keep_bar() {
    let mut s = setup();
    s.sw.add_rule("a.exe", Some(s.black));
    s.sw.run(&started(1, "a.exe", dell()), &s.presets, &mut s.svc).unwrap();
    assert!(s.svc.pending().is_none());
    assert_eq!(s.svc.keep_seconds_left(std::time::Instant::now()), None);
}

#[test]
fn automatic_switch_is_never_stored_as_windows_saved_setting() {
    let mut s = setup();
    let saved = s.svc.os().saved_mode(&dell());
    s.sw.add_rule("a.exe", Some(s.black));
    s.sw.run(&started(1, "a.exe", dell()), &s.presets, &mut s.svc).unwrap();
    assert_eq!(s.svc.os().current(&dell()).width, 1280);
    // A crash / reboot during the game would come back to the user's own mode.
    assert_eq!(s.svc.os().saved_mode(&dell()), saved);
    s.sw.run(&AppEvent::Stopped { pid: 1 }, &s.presets, &mut s.svc).unwrap();
    assert_eq!(s.svc.os().saved_mode(&dell()), saved);
}

#[test]
fn second_app_failing_hands_the_monitor_back_to_the_first_apps_preset() {
    let mut s = setup();
    let before = s.svc.os().current(&dell());
    s.sw.add_rule("a.exe", Some(s.black));
    let b = s.sw.add_rule("b.exe", Some(s.stretch));
    s.sw.rule_mut(b).unwrap().vibrance = Some(80);
    s.sw.run(&started(1, "a.exe", dell()), &s.presets, &mut s.svc).unwrap();
    let a_mode = s.svc.os().current(&dell());
    assert_eq!((a_mode.width, a_mode.scaling), (1280, GpuScaling::BlackBars));
    // b's preset applies, then its vibrance fails → b is dropped and the monitor goes back to a's preset (not left on b's).
    s.svc.os_mut().admin_required.insert(ChangeKind::Vibrance);
    s.sw.run(&started(2, "b.exe", dell()), &s.presets, &mut s.svc).unwrap();
    assert_eq!(s.svc.os().current(&dell()), a_mode);
    assert!(matches!(s.sw.take_notes().as_slice(), [SwitchNote::SwitchFailed { .. }]));
    // a still owns it; when a ends, the original mode comes back.
    s.svc.os_mut().admin_required.clear();
    s.sw.run(&AppEvent::Stopped { pid: 1 }, &s.presets, &mut s.svc).unwrap();
    assert_eq!(s.svc.os().current(&dell()), before);
}
