//! Every Display change against the FAKE OS layer: read, apply, keep / revert, undo, the admin path, errors.

use bu_display::fake::{FakeCall, FakeDisplayOs};
use bu_display::picture::{vibrance_level, vibrance_percent, DdcCrashGuard};
use bu_display::presets::PresetList;
use bu_display::service::{UndoEntry, KEEP_SECONDS};
use bu_display::*;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

fn dell() -> MonitorId {
    MonitorId("fake-dell".into())
}
fn lg() -> MonitorId {
    MonitorId("fake-lg".into())
}
fn r(m: u32) -> RefreshRate {
    RefreshRate::new(m, 1000)
}
fn svc() -> DisplayService<FakeDisplayOs> {
    DisplayService::new(FakeDisplayOs::two_monitors())
}

// ---------- 1. monitors ----------

#[test]
fn monitors_listed_in_number_order_with_main_position_dpi_hdr() {
    let s = svc();
    let m = s.monitors().unwrap();
    assert_eq!(m.len(), 2);
    assert_eq!((m[0].number, m[0].is_main, m[0].rect.x), (1, true, 0));
    assert_eq!((m[1].number, m[1].is_main, m[1].rect.x), (2, false, 2560));
    assert_eq!(m[0].dpi.as_ref().unwrap().current_percent, 100);
    assert_eq!(m[0].hdr, Some(HdrInfo { supported: true, enabled: false }));
    assert_eq!(DisplayService::<FakeDisplayOs>::selector_label(&m[0]), "1 · DELL 27″");
    assert_eq!(DisplayService::<FakeDisplayOs>::selector_label(&m[1]), "2 · LG 24″");
    assert_eq!(s.selector_tooltip(&m[1]), "LG 24GL600F · 1920 × 1080 · 144 Hz");
    assert_eq!(s.selector_tooltip(&m[0]), "DELL S2721DGF · 2560 × 1440 · 165 Hz");
    assert!(matches!(s.monitor(&MonitorId("nope".into())), Err(DisplayError::MonitorNotFound(_))));
}

// ---------- 2. modes ----------

#[test]
fn modes_are_the_monitors_own_sorted_biggest_fastest_first() {
    let s = svc();
    let v = s.modes(&lg()).unwrap();
    assert_eq!((v[0].width, v[0].height, v[0].refresh), (1920, 1080, r(143_981)));
    assert_eq!(v.len(), 12);
    let rates = fields::rates_for(&v, 1920, 1080);
    assert_eq!(rates, vec![r(60_000), r(119_982), r(143_981)]);
}

// ---------- 3. apply + keep / revert ----------

#[test]
fn apply_snaps_hz_then_keep_makes_it_undoable() {
    let mut s = svc();
    let t0 = Instant::now();
    let a = s.apply_fields(&lg(), 1280, 960, 240.0, GpuScaling::BlackBars, t0).unwrap();
    assert_eq!(a.mode, Mode { width: 1280, height: 960, refresh: r(143_981), scaling: GpuScaling::BlackBars });
    assert_eq!(a.revert_to.width, 1920);
    assert_eq!(s.keep_seconds_left(t0), Some(KEEP_SECONDS));
    assert_eq!(s.keep_seconds_left(t0 + Duration::from_millis(9_001)), Some(1));
    s.keep().unwrap();
    assert!(s.pending().is_none());
    assert_eq!(s.os().current(&lg()).width, 1280);
    assert!(matches!(s.undo_stack().last(), Some(UndoEntry::Mode { .. })));
    s.undo().unwrap();
    assert_eq!(s.os().current(&lg()).width, 1920);
    assert_eq!(s.undo(), Err(DisplayError::NothingToUndo));
}

#[test]
fn revert_button_goes_back() {
    let mut s = svc();
    let before = s.os().current(&dell());
    s.apply_fields(&dell(), 1920, 1080, 144.0, GpuScaling::Stretch, Instant::now()).unwrap();
    let r = s.revert().unwrap();
    assert!(!r.timed_out);
    assert_eq!(r.restored, vec![(dell(), before)]);
    assert_eq!(s.os().current(&dell()), before);
    assert_eq!(s.revert(), Err(DisplayError::NoPendingChange));
    assert_eq!(s.keep(), Err(DisplayError::NoPendingChange));
}

#[test]
fn countdown_reverts_by_itself_at_zero_not_before() {
    let mut s = svc();
    let before = s.os().current(&dell());
    let t0 = Instant::now();
    s.apply_fields(&dell(), 1280, 720, 60.0, GpuScaling::KeepAspect, t0).unwrap();
    assert!(s.tick(t0 + Duration::from_millis(9_999)).is_none());
    let r = s.tick(t0 + Duration::from_secs(10)).unwrap().unwrap();
    assert!(r.timed_out);
    assert_eq!(s.os().current(&dell()), before);
    assert_eq!(s.keep_seconds_left(t0), None);
}

#[test]
fn applying_again_keeps_the_original_as_revert_target() {
    let mut s = svc();
    let original = s.os().current(&dell());
    let t0 = Instant::now();
    s.apply_fields(&dell(), 1920, 1080, 144.0, GpuScaling::Stretch, t0).unwrap();
    let a2 = s.apply_fields(&dell(), 1280, 960, 60.0, GpuScaling::BlackBars, t0 + Duration::from_secs(5)).unwrap();
    assert_eq!(a2.revert_to, original);
    // The countdown restarted at the second Apply.
    assert!(s.tick(t0 + Duration::from_secs(12)).is_none());
    s.revert().unwrap();
    assert_eq!(s.os().current(&dell()), original);
}

#[test]
fn unsupported_size_is_refused_with_nearest_and_nothing_changes() {
    let mut s = svc();
    let e = s.apply_fields(&lg(), 2560, 1440, 144.0, GpuScaling::Stretch, Instant::now()).unwrap_err();
    match e {
        DisplayError::ModeNotSupported { nearest: Some(n), .. } => assert_eq!((n.width, n.height), (1920, 1080)),
        other => panic!("{other:?}"),
    }
    assert!(s.pending().is_none());
    assert!(s.os().calls.is_empty());
}

#[test]
fn os_failure_on_apply_is_typed_and_leaves_no_bar() {
    let mut s = svc();
    s.os_mut().fail_next_change = Some("driver said no".into());
    let e = s.apply_fields(&lg(), 1280, 960, 60.0, GpuScaling::Stretch, Instant::now()).unwrap_err();
    assert!(matches!(e, DisplayError::Os { .. }));
    assert!(s.pending().is_none());
}

#[test]
fn failed_revert_keeps_the_bar_and_the_target() {
    let mut s = svc();
    s.apply_fields(&lg(), 1280, 960, 60.0, GpuScaling::Stretch, Instant::now()).unwrap();
    s.os_mut().fail_next_change = Some("busy".into());
    assert!(s.revert().is_err());
    assert!(s.pending().is_some());
    s.revert().unwrap();
    assert_eq!(s.os().current(&lg()).width, 1920);
}

#[test]
fn keep_timer_reverts_on_its_own_even_with_no_ui() {
    let s = Arc::new(Mutex::new(svc().with_keep_duration(Duration::from_millis(60))));
    s.lock().unwrap().apply_fields(&lg(), 1280, 960, 60.0, GpuScaling::Stretch, Instant::now()).unwrap();
    let (tx, rx) = mpsc::channel();
    let _t = keep::arm(s.clone(), move |r| {
        let _ = tx.send(r);
    })
    .unwrap();
    let r = rx.recv_timeout(Duration::from_secs(60)).expect("timer fired").unwrap();
    assert!(r.timed_out);
    assert_eq!(s.lock().unwrap().os().current(&lg()).width, 1920);
}

#[test]
fn keep_timer_cancelled_by_keep_does_nothing() {
    let s = Arc::new(Mutex::new(svc().with_keep_duration(Duration::from_secs(60))));
    s.lock().unwrap().apply_fields(&lg(), 1280, 960, 60.0, GpuScaling::Stretch, Instant::now()).unwrap();
    let (tx, rx) = mpsc::channel::<()>();
    let t = keep::arm(s.clone(), move |_| {
        let _ = tx.send(());
    })
    .unwrap();
    s.lock().unwrap().keep().unwrap();
    t.cancel();
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    assert_eq!(s.lock().unwrap().os().current(&lg()).width, 1280);
}

#[test]
fn keep_timer_fired_after_keep_finds_nothing_to_revert() {
    // Race: the timer fires while Keep is being pressed. It must not revert a kept change.
    // Load-proof: if the PC was so busy that the timer fired before we even got the lock, the race wasn't set up —
    // try again with a longer countdown instead of failing.
    let mut keep_for = Duration::from_millis(40);
    for _ in 0..6 {
        let s = Arc::new(Mutex::new(svc().with_keep_duration(keep_for)));
        s.lock().unwrap().apply_fields(&lg(), 1280, 960, 60.0, GpuScaling::Stretch, Instant::now()).unwrap();
        let (tx, rx) = mpsc::channel::<()>();
        let t = keep::arm(s.clone(), move |_| {
            let _ = tx.send(());
        })
        .unwrap();
        {
            // Hold the service while the deadline passes (the timer fires and waits for the lock), then press Keep.
            let mut g = s.lock().unwrap();
            if g.pending().is_none() {
                keep_for *= 4;
                continue;
            }
            std::thread::sleep(keep_for * 3);
            g.keep().unwrap();
        }
        // The timer finishes after we let go (it finds nothing pending → no callback).
        let give_up = Instant::now() + Duration::from_secs(60);
        while !t.is_finished() && Instant::now() < give_up {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(t.is_finished(), "timer thread ended");
        assert!(rx.try_recv().is_err());
        assert_eq!(s.lock().unwrap().os().current(&lg()).width, 1280);
        return;
    }
    panic!("the PC was too busy to set up the race 6 times in a row");
}

#[test]
fn undo_while_bar_is_up_reverts() {
    let mut s = svc();
    s.apply_fields(&lg(), 1280, 960, 60.0, GpuScaling::Stretch, Instant::now()).unwrap();
    s.undo().unwrap();
    assert!(s.pending().is_none());
    assert_eq!(s.os().current(&lg()).width, 1920);
}

#[test]
fn preset_click_applies_with_keep_bar_and_snapped_hz() {
    let mut s = svc();
    let mut p = PresetList::new();
    let dell_modes = s.modes(&dell()).unwrap();
    let id = p.add(1920, 1080, 165.0, GpuScaling::KeepAspect, &dell_modes).unwrap();
    // Applied on the LG (fastest 143.98) → snapped.
    let a = s.apply_preset(&lg(), p.get(id).unwrap(), Instant::now()).unwrap();
    assert_eq!(a.mode.refresh, r(143_981));
    assert!(s.pending().is_some());
    assert_eq!(p.matching(&a.mode, &s.modes(&lg()).unwrap()), Some(id));
}

// ---------- 4. Windows scaling % ----------

#[test]
fn dpi_set_and_undo_and_refusals() {
    let mut s = svc();
    s.set_dpi_percent(&lg(), 125).unwrap();
    assert_eq!(s.monitor(&lg()).unwrap().dpi.unwrap().current_percent, 125);
    assert_eq!(s.set_dpi_percent(&lg(), 110), Err(DisplayError::DpiNotOffered(110)));
    assert_eq!(s.set_dpi_percent(&lg(), 250), Err(DisplayError::DpiNotOffered(250)));
    s.undo().unwrap();
    assert_eq!(s.monitor(&lg()).unwrap().dpi.unwrap().current_percent, 100);
    // Same value: no call, no undo entry.
    s.set_dpi_percent(&lg(), 100).unwrap();
    assert!(!s.can_undo());
}

// ---------- 6. main display ----------

#[test]
fn main_switch_moves_main_and_undo_moves_it_back() {
    let mut s = svc();
    assert!(s.set_main(&lg()).unwrap());
    let m = s.monitors().unwrap();
    assert!(m[1].is_main && !m[0].is_main);
    assert_eq!((m[1].rect.x, m[0].rect.x), (0, -2560)); // relative places kept
    assert_eq!(DisplayService::<FakeDisplayOs>::main_toast(&m[1]), "Main display: 2 · LG 24GL600F");
    // Clicking the main one's switch is not a change (it only nudges).
    assert!(!s.set_main(&lg()).unwrap());
    s.undo().unwrap();
    let m = s.monitors().unwrap();
    assert!(m[0].is_main && m[0].rect.x == 0);
}

// ---------- 7. picture ----------

#[test]
fn picture_reads_ddc_and_vibrance_live() {
    let mut s = svc();
    let p = s.picture(&dell()).unwrap();
    assert_eq!(p.ddc, DdcState::Answers);
    assert_eq!(p.brightness.unwrap().percent(), 70);
    assert_eq!(p.contrast.unwrap().percent(), 75);
    assert_eq!(p.vibrance_percent, Some(50));
    assert_eq!(p.vibrance_vendor, Some(GpuVendor::Nvidia));
    // The LG doesn't answer DDC/CI: brightness/contrast greyed, vibrance still works.
    let p = s.picture(&lg()).unwrap();
    assert_eq!(p.ddc, DdcState::NoAnswer);
    assert!(p.brightness.is_none() && p.contrast.is_none());
    assert_eq!(p.vibrance_percent, Some(50));
    assert_eq!(s.set_ddc_percent(&lg(), Vcp::Brightness, 50), Err(DisplayError::DdcNoAnswer));
}

#[test]
fn ddc_slider_drag_is_one_undo_step() {
    let mut s = svc();
    for v in [71, 75, 80, 90] {
        s.set_ddc_percent(&dell(), Vcp::Brightness, v).unwrap();
    }
    assert_eq!(s.picture(&dell()).unwrap().brightness.unwrap().current, 90);
    assert_eq!(s.undo_stack().len(), 1);
    s.undo().unwrap();
    assert_eq!(s.picture(&dell()).unwrap().brightness.unwrap().current, 70);
}

#[test]
fn vibrance_percent_maps_to_driver_levels_and_undoes() {
    let mut s = svc();
    s.set_vibrance_percent(&lg(), 80).unwrap();
    // Fake = NVIDIA Ex range -63..63, default 0: 80 % → round(30 × 63 / 50) = 38.
    assert_eq!(s.os().calls.last(), Some(&FakeCall::VibranceSet(lg(), 38)));
    assert_eq!(s.picture(&lg()).unwrap().vibrance_percent, Some(80));
    s.set_vibrance_percent(&lg(), 0).unwrap();
    assert_eq!(s.picture(&lg()).unwrap().vibrance_percent, Some(0));
    s.undo().unwrap();
    assert_eq!(s.picture(&lg()).unwrap().vibrance_percent, Some(50));
}

#[test]
fn vibrance_mapping_matches_vibranceguis_table() {
    // vibranceGUI NvidiaVibranceValueWrapper.cs: 50 %..100 % ↔ these legacy DVC levels (0..63).
    let table = [
        0, 1, 3, 4, 5, 6, 8, 9, 10, 11, 13, 14, 15, 16, 18, 19, 20, 21, 23, 24, 25, 26, 28, 29, 30, 32, 33, 34, 35, 37, 38, 39, 40, 42,
        43, 44, 45, 47, 48, 49, 50, 52, 53, 54, 55, 57, 58, 59, 60, 62, 63,
    ];
    let legacy = VibranceRaw { vendor: GpuVendor::Nvidia, current: 0, min: 0, max: 63, default: 0 };
    for (i, lvl) in table.iter().enumerate() {
        let pct = 50 + i as u8;
        assert_eq!(vibrance_level(&legacy, pct), *lvl, "percent {pct}");
        assert_eq!(vibrance_percent(&VibranceRaw { current: *lvl, ..legacy }), pct, "level {lvl}");
    }
    // The old API can't go below normal.
    assert_eq!(vibrance_level(&legacy, 20), 0);
    // Measured on an NVIDIA driver (Ex API): min 0, max 100, default 50 — levels are percents.
    let ex = VibranceRaw { vendor: GpuVendor::Nvidia, current: 50, min: 0, max: 100, default: 50 };
    for p in [0u8, 25, 50, 80, 100] {
        assert_eq!(vibrance_level(&ex, p), p as i32);
    }
    // AMD ADL saturation, typical 0..200 with 100 normal.
    let amd = VibranceRaw { vendor: GpuVendor::Amd, current: 100, min: 0, max: 200, default: 100 };
    assert_eq!(vibrance_percent(&amd), 50);
    assert_eq!(vibrance_level(&amd, 75), 150);
}

#[test]
fn no_vibrance_gpu_reports_none() {
    let mut os = FakeDisplayOs::two_monitors();
    os.monitors[0].vibrance = None;
    let mut s = DisplayService::new(os);
    assert_eq!(s.picture(&dell()).unwrap().vibrance_percent, None);
    assert!(matches!(s.set_vibrance_percent(&dell(), 70), Err(DisplayError::VibranceUnsupported(_))));
}

#[test]
fn ddc_crash_guard_blocks_after_a_died_call() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ddc-guard-test");
    let _ = std::fs::remove_dir_all(&dir);
    let g = DdcCrashGuard::new(Some(dir.clone()));
    let key = r"\\?\DISPLAY#DEL41B6#5&abc&0&UID4352#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";
    assert!(!g.is_blocked(key));
    // During the call the marker exists; after it, it's gone.
    let seen_inside = g.run(key, || g.is_blocked(key));
    assert!(seen_inside);
    assert!(!g.is_blocked(key));
    // Simulate a crash mid-call: marker left behind → blocked until cleared.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| g.run(key, || panic!("monitor hung"))));
    assert!(g.is_blocked(key));
    g.clear(key);
    assert!(!g.is_blocked(key));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ddc_blocked_state_reaches_the_picture() {
    // A fake OS layer that answers like the real one when the guard has blocked a monitor.
    struct Blocked(FakeDisplayOs);
    impl DisplayOs for Blocked {
        fn monitors(&self) -> Result<Vec<MonitorInfo>> { self.0.monitors() }
        fn modes(&self, id: &MonitorId) -> Result<Vec<VideoMode>> { self.0.modes(id) }
        fn apply_mode(&mut self, id: &MonitorId, m: &Mode, save: bool) -> Result<()> { self.0.apply_mode(id, m, save) }
        fn save_current(&mut self, id: &MonitorId, mode: &Mode) -> Result<()> { self.0.save_current(id, mode) }
        fn set_main(&mut self, id: &MonitorId) -> Result<()> { self.0.set_main(id) }
        fn set_dpi_percent(&mut self, id: &MonitorId, p: u32) -> Result<()> { self.0.set_dpi_percent(id, p) }
        fn ddc_get(&mut self, _: &MonitorId, _: Vcp) -> Result<VcpValue> { Err(DisplayError::DdcBlockedAfterCrash) }
        fn ddc_set(&mut self, _: &MonitorId, _: Vcp, _: u32) -> Result<()> { Err(DisplayError::DdcBlockedAfterCrash) }
        fn vibrance_get(&mut self, id: &MonitorId) -> Result<VibranceRaw> { self.0.vibrance_get(id) }
        fn vibrance_set(&mut self, id: &MonitorId, l: i32) -> Result<()> { self.0.vibrance_set(id, l) }
        fn needs_admin(&self, k: ChangeKind) -> bool { self.0.needs_admin(k) }
    }
    let mut s = DisplayService::new(Blocked(FakeDisplayOs::two_monitors()));
    assert_eq!(s.picture(&dell()).unwrap().ddc, DdcState::BlockedAfterCrash);
}

// ---------- admin path ----------

#[test]
fn admin_path_is_reported_and_never_elevated() {
    let mut os = FakeDisplayOs::two_monitors();
    os.admin_required.insert(ChangeKind::Mode);
    os.admin_required.insert(ChangeKind::MainDisplay);
    let mut s = DisplayService::new(os);
    assert!(s.needs_admin(ChangeKind::Mode));
    assert!(!s.needs_admin(ChangeKind::Ddc));
    assert_eq!(
        s.apply_fields(&lg(), 1280, 960, 60.0, GpuScaling::Stretch, Instant::now()),
        Err(DisplayError::NeedsAdmin(ChangeKind::Mode))
    );
    assert!(s.pending().is_none());
    assert_eq!(s.set_main(&lg()), Err(DisplayError::NeedsAdmin(ChangeKind::MainDisplay)));
    assert!(!s.can_undo());
}

#[test]
fn failed_undo_stays_on_the_stack() {
    let mut s = svc();
    s.set_main(&lg()).unwrap();
    s.os_mut().fail_next_change = Some("nope".into());
    assert!(s.undo().is_err());
    assert!(s.can_undo());
    s.undo().unwrap();
}

// ---------- Windows' saved display setting: only Keep stores a mode ----------

#[test]
fn only_keep_stores_the_mode_countdown_and_revert_never_do() {
    let mut s = svc();
    let start = s.os().current(&lg());
    let t0 = Instant::now();
    s.apply_fields(&lg(), 1280, 960, 144.0, GpuScaling::BlackBars, t0).unwrap();
    assert_eq!(s.os().saved_mode(&lg()), start, "a crash during the countdown comes back to the old mode");
    // Timed out → back, nothing stored.
    s.tick(t0 + Duration::from_secs(KEEP_SECONDS)).unwrap().unwrap();
    assert_eq!(s.os().saved_mode(&lg()), start);
    // Applied + kept → stored.
    s.apply_fields(&lg(), 1280, 960, 144.0, GpuScaling::BlackBars, t0).unwrap();
    s.keep().unwrap();
    assert!(s.os().calls.contains(&FakeCall::SaveCurrent(lg())));
    assert_eq!(s.os().saved_mode(&lg()).width, 1280);
    // Undo of a kept change is a manual change: stored too.
    s.undo().unwrap();
    assert_eq!(s.os().saved_mode(&lg()), start);
}

/// Order 042 (the owner's test 2: "only keep aspect ever stays on after you click confirm"): Windows' read-back of a path
/// doesn't carry the GPU scaling Apply set. Keep must store the mode Apply set (Stretch), not the read-back, and the
/// monitor is reported with the scaling that was set while it still shows that size and rate.
#[test]
fn keep_stores_the_scaling_that_was_applied_not_the_read_back() {
    struct LosesScaling(FakeDisplayOs);
    impl DisplayOs for LosesScaling {
        fn monitors(&self) -> Result<Vec<MonitorInfo>> {
            let mut v = self.0.monitors()?;
            for m in &mut v {
                m.current.scaling = GpuScaling::KeepAspect;
            }
            Ok(v)
        }
        fn modes(&self, id: &MonitorId) -> Result<Vec<VideoMode>> { self.0.modes(id) }
        fn apply_mode(&mut self, id: &MonitorId, m: &Mode, save: bool) -> Result<()> { self.0.apply_mode(id, m, save) }
        fn save_current(&mut self, id: &MonitorId, mode: &Mode) -> Result<()> { self.0.save_current(id, mode) }
        fn set_main(&mut self, id: &MonitorId) -> Result<()> { self.0.set_main(id) }
        fn set_dpi_percent(&mut self, id: &MonitorId, p: u32) -> Result<()> { self.0.set_dpi_percent(id, p) }
        fn ddc_get(&mut self, id: &MonitorId, v: Vcp) -> Result<VcpValue> { self.0.ddc_get(id, v) }
        fn ddc_set(&mut self, id: &MonitorId, v: Vcp, x: u32) -> Result<()> { self.0.ddc_set(id, v, x) }
        fn vibrance_get(&mut self, id: &MonitorId) -> Result<VibranceRaw> { self.0.vibrance_get(id) }
        fn vibrance_set(&mut self, id: &MonitorId, l: i32) -> Result<()> { self.0.vibrance_set(id, l) }
        fn needs_admin(&self, k: ChangeKind) -> bool { self.0.needs_admin(k) }
    }
    let mut s = DisplayService::new(LosesScaling(FakeDisplayOs::two_monitors()));
    let t0 = Instant::now();
    for sc in [GpuScaling::Stretch, GpuScaling::BlackBars] {
        let a = s.apply_fields(&lg(), 1280, 960, 144.0, sc, t0).unwrap();
        assert_eq!(a.mode.scaling, sc, "shown after Apply");
        let kept = s.keep().unwrap();
        assert_eq!(kept[0].1.scaling, sc, "reported as kept");
        assert_eq!(s.os().0.saved_mode(&lg()).scaling, sc, "Windows' saved setting");
        assert_eq!(s.monitor(&lg()).unwrap().current.scaling, sc, "read again after Keep");
    }
}

#[test]
fn keep_refused_by_windows_leaves_the_bar_up() {
    let mut s = svc();
    let t0 = Instant::now();
    s.apply_fields(&lg(), 1280, 960, 144.0, GpuScaling::BlackBars, t0).unwrap();
    s.os_mut().fail_next_change = Some("store refused".into());
    assert!(s.keep().is_err());
    assert!(s.pending().is_some(), "the bar stays; the countdown still reverts");
    assert!(!s.can_undo());
    s.keep().unwrap();
    assert!(s.pending().is_none());
}

// ---------- never panics on odd driver / OS ranges ----------

#[test]
fn reversed_driver_ranges_do_not_panic() {
    assert_eq!(bu_display::picture::clamp_any(150, 100, 0), 100);
    assert_eq!(bu_display::picture::clamp_any(-5, 100, 0), 0);
    let odd = VibranceRaw { vendor: GpuVendor::Nvidia, current: 0, min: 100, max: 0, default: 50 };
    for p in [0u8, 50, 100] {
        let l = vibrance_level(&odd, p);
        assert!((0..=100).contains(&l), "percent {p} → {l}");
    }
    let mut os = FakeDisplayOs::two_monitors();
    if let Some(v) = os.monitors.iter_mut().find(|m| m.info.id == dell()).and_then(|m| m.vibrance.as_mut()) {
        (v.min, v.max) = (v.max, v.min);
    }
    let mut s = DisplayService::new(os);
    s.set_vibrance_percent(&dell(), 90).unwrap();
}

// ---------- DDC/CI exclusion list (PowerToys' built-in list) ----------

#[test]
fn ddc_excluded_models_are_never_asked() {
    use bu_display::picture::{ddc_excluded, edid_id};
    let lg_bad = MonitorId(r"\\?\DISPLAY#GSM7714#5&1a2b&0&UID4352#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}".into());
    assert_eq!(edid_id(&lg_bad), Some("GSM7714"));
    assert!(ddc_excluded(&lg_bad));
    assert!(ddc_excluded(&MonitorId(r"\\?\DISPLAY#ltm2c02#x#{y}".into())));
    assert!(!ddc_excluded(&MonitorId(r"\\?\DISPLAY#DELD1A8#x#{y}".into())));
    assert!(!ddc_excluded(&MonitorId("fake-dell".into())));
    assert_eq!(edid_id(&MonitorId("no-hash".into())), None);

    let mut os = FakeDisplayOs::two_monitors();
    os.monitors[1].info.id = lg_bad.clone();
    let mut s = DisplayService::new(os);
    let p = s.picture(&lg_bad).unwrap();
    assert_eq!(p.ddc, DdcState::ExcludedModel);
    assert_eq!((p.brightness, p.contrast), (None, None));
    assert_eq!(p.vibrance_percent, Some(50), "vibrance is the GPU driver, not DDC: still there");
    assert_eq!(s.set_ddc_percent(&lg_bad, Vcp::Brightness, 70), Err(DisplayError::DdcExcludedModel));
    assert!(!s.os().calls.iter().any(|c| matches!(c, FakeCall::DdcSet(..))));
    assert!(!s.can_undo());
    // The other monitor is unaffected.
    assert_eq!(s.picture(&dell()).unwrap().ddc, DdcState::Answers);
}
