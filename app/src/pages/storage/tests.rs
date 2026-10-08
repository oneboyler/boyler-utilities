//! The Storage page against the FAKE PC (`FakeOs::drawing`): nothing measured on open, Measure with progress + Stop, the
//! views, Clean up only after measuring, Clean only the ticked rows, health.

use std::time::Duration;

use super::*;
use crate::gfx::Gfx;
use crate::ui::cx::State;
use crate::ui::lay::Laid;

fn opened() -> Storage {
    let mut s = Storage::default();
    s.open(&Env { test: true, frozen: true, ..Env::default() }, 0.0);
    s
}

fn build(s: &mut Storage, st: &mut State, now: f64) -> Laid {
    let g = Gfx::new(1.0);
    let mut cx = Cx::new(now, false, &g, st);
    let kids = s.build(&mut cx);
    Laid::new(&g, El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids), 600.0, None)
}

fn click(s: &mut Storage, st: &mut State, k: Key) {
    let g = Gfx::new(1.0);
    let mut cx = Cx::new(0.0, false, &g, st);
    s.event(&Ev::Click(k), &mut cx);
}

fn wait_for(s: &mut Storage, ms: u64, f: impl Fn(&Storage) -> bool) -> bool {
    let t0 = std::time::Instant::now();
    while t0.elapsed().as_millis() < ms as u128 {
        // the page's clock runs like the frame's (the staged sizes / the countdown follow it)
        s.tick(t0.elapsed().as_millis() as f64);
        if f(s) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn open_shows_drives_and_health_and_measures_nothing() {
    let mut s = opened();
    assert_eq!(s.drives.len(), 3);
    assert_eq!(s.drv, 'C');
    assert_eq!(s.health.len(), 3);
    assert_eq!(s.health[2].warnings, ["E: 8 sectors were moved to spares · back up what matters."]);
    assert!(s.scans.is_empty() && s.plan.is_none() && !s.measuring);
    assert!(s.fake.as_ref().unwrap().changes().is_empty());
    let mut st = State::default();
    let l = build(&mut s, &mut st, 0.0);
    assert!(l.rect_of(K_MEASURE).is_some(), "the idle state shows Measure");
    assert!(l.rect_of(K_CLN).is_some());
    assert_eq!(gbf(s.drives[0].info.free_bytes), "612 GB");
    assert_eq!(gbf(s.drives[0].info.total_bytes), "1.82 TB");
}

#[test]
fn the_drawings_size_format() {
    assert_eq!(gbf((6.4 * 1_073_741_824.0) as u64), "6.4 GB");
    assert_eq!(gbf((14.4 * 1_073_741_824.0) as u64), "14.4 GB");
    assert_eq!(gbf(3726 * 1_073_741_824), "3.64 TB");
    assert_eq!(gbf(205 * 1_048_576), "205 MB");
    assert_eq!(gbf(0), "0 B");
}

#[test]
fn measure_runs_only_on_the_button_shows_progress_and_ends_in_the_views() {
    let mut s = opened();
    let mut st = State::default();
    click(&mut s, &mut st, K_MEASURE);
    assert!(matches!(s.scan_of('C'), Scan::Running { .. }));
    let _ = build(&mut s, &mut st, 10.0);
    assert!(wait_for(&mut s, 10_000, |s| matches!(s.scan_of('C'), Scan::Done { .. })));
    let l = build(&mut s, &mut st, 20.0);
    assert!(l.rect_of(idx(K_TYR, 0)).is_some(), "file types listed");
    // Folders: the biggest first; a folder with folders inside goes deeper, the path goes back
    click(&mut s, &mut st, idx(K_SEG, 1));
    let _ = build(&mut s, &mut st, 30.0);
    let Scan::Done { result, .. } = s.scan_of('C') else { panic!() };
    let rows = result.tree.rows(result.tree.root()).unwrap();
    assert_eq!(rows[0].name, "Users");
    click(&mut s, &mut st, idx(K_FDR, 0));
    assert_eq!(s.path.len(), 1);
    click(&mut s, &mut st, K_BACK);
    assert!(s.path.is_empty());
    // Windows' own folder can't be opened
    let wi = rows.iter().position(|r| r.name == "Windows").unwrap();
    click(&mut s, &mut st, idx(K_FDR, wi));
    assert!(s.path.is_empty());
    // Open in Explorer in a test copy only says what it would do
    click(&mut s, &mut st, idx(K_FOP, 0));
    assert!(s.toast.as_ref().unwrap().0.starts_with("Opens "));
    // nothing on the PC changed
    assert!(s.fake.as_ref().unwrap().changes().is_empty());
}

#[test]
fn stop_ends_the_walk() {
    let mut s = opened();
    let mut st = State::default();
    click(&mut s, &mut st, K_MEASURE);
    click(&mut s, &mut st, K_STOP);
    assert!(matches!(s.scan_of('C'), Scan::Idle));
    std::thread::sleep(Duration::from_millis(300));
    s.tick(0.0);
    // a late result of the stopped walk is not shown
    assert!(matches!(s.scan_of('C'), Scan::Idle));
}

/// REVIEW 022 HOLD 3: Measure, Stop, Measure again - the first walk's late answer (Cancelled, or even a finished result) must
/// not end the second walk; the second stays Running with its own Stop, and closing the tab cancels it.
#[test]
fn a_stopped_walks_late_answer_does_not_end_the_new_walk() {
    let mut s = opened();
    let mut st = State::default();
    click(&mut s, &mut st, K_MEASURE);
    let Scan::Running { ctl: first, .. } = s.scan_of('C') else { panic!("running") };
    let first = first.clone();
    click(&mut s, &mut st, K_STOP);
    assert!(first.is_cancelled());
    click(&mut s, &mut st, K_MEASURE);
    let Scan::Running { ctl: second, .. } = s.scan_of('C') else { panic!("running again") };
    let second = second.clone();
    assert!(!Arc::ptr_eq(&first, &second));
    // the old walk answers late - as Cancelled, and as a result
    s.apply(Msg::Scanned('C', first.clone(), Err(StorageError::Cancelled), "21:37".into(), std::time::SystemTime::now()));
    assert!(matches!(s.scan_of('C'), Scan::Running { ctl, .. } if Arc::ptr_eq(ctl, &second)), "still the second walk");
    let old = scan::scan_drive(s.os.clone().unwrap().as_ref(), 'C', &ScanControl::new()).unwrap();
    s.apply(Msg::Scanned('C', first, Ok(Box::new(old)), "21:37".into(), std::time::SystemTime::now()));
    assert!(matches!(s.scan_of('C'), Scan::Running { ctl, .. } if Arc::ptr_eq(ctl, &second)), "a stale result is dropped");
    // closing the tab does NOT stop the walk (F2): it goes on and its result is kept
    s.close();
    assert!(!second.is_cancelled(), "close() keeps the walk that runs now");
    second.cancel();
}

#[test]
fn another_drive_is_measured_separately() {
    let mut s = opened();
    let mut st = State::default();
    click(&mut s, &mut st, idx(K_DRV, 2));
    assert_eq!(s.drv, 'E');
    assert!(matches!(s.scan_of('E'), Scan::Idle));
    let l = build(&mut s, &mut st, 0.0);
    assert!(l.rect_of(K_MEASURE).is_some());
}

#[test]
fn clean_up_measures_first_then_cleans_only_the_ticked_rows() {
    let mut s = opened();
    let mut st = State::default();
    // before measuring, a row can't be ticked and the button measures
    click(&mut s, &mut st, idx(K_CN, 0));
    assert_eq!(s.ticked.len(), 4, "every row ticked at rest; a click before measuring changes nothing");
    click(&mut s, &mut st, K_CLN);
    assert!(s.measuring);
    assert!(wait_for(&mut s, 5000, |s| s.plan.is_some()));
    let gbs: Vec<String> = CleanKind::ALL.iter().map(|k| gbf(s.plan.as_ref().unwrap().row(*k).unwrap().bytes)).collect();
    assert_eq!(gbs, ["6.4 GB", "3.3 GB", "2.8 GB", "1.9 GB"]);
    // every row ticked: "Clean 14.4 GB"
    let l = build(&mut s, &mut st, 0.0);
    assert!(l.rect_of(K_CLN).is_some());
    assert_eq!(s.ticked.len(), 4);
    assert!(s.fake.as_ref().unwrap().changes().is_empty(), "measuring deletes nothing");
    // untick the recycle bin, then Clean
    click(&mut s, &mut st, idx(K_CN, 0));
    assert_eq!(s.ticked.len(), 3);
    click(&mut s, &mut st, K_CLN);
    assert!(wait_for(&mut s, 5000, |s| s.report.is_some()));
    let ch = s.fake.as_ref().unwrap().changes();
    assert!(!ch.iter().any(|c| c.contains("recycle")), "{ch:?}");
    assert!(ch.iter().any(|c| c.contains("temp")), "{ch:?}");
    let t = &s.toast.as_ref().unwrap().0;
    assert!(t.starts_with("Freed 7.8 GB") && t.contains("205 MB of temp files are in use and stay"), "{t}");
}

/// REVIEW 022 HOLD 7 - a copy that is not a picture copy (`frozen` off) shows the real detail lines worked out from what was
/// measured, incl. the needs-admin note (the fake PC is not elevated: Windows' temp folder is left out and said so).
#[test]
fn measured_detail_lines_and_the_admin_note() {
    let mut s = Storage::default();
    s.open(&Env { test: true, frozen: false, ..Env::default() }, 0.0);
    let mut st = State::default();
    // before measuring: the crate's own line per row
    assert_eq!(view::detail(&s, CleanKind::TempFiles), CleanKind::TempFiles.detail());
    // Windows' temp folder holds something (reading / cleaning it needs admin; the fake PC is not elevated)
    s.fake.as_ref().unwrap().add_file(r"C:\Windows\Temp\setup.log", 4_000_000);
    click(&mut s, &mut st, K_CLN);
    assert!(wait_for(&mut s, 5000, |s| s.plan.is_some()));
    let temp = view::detail(&s, CleanKind::TempFiles);
    assert!(temp.contains("needs admin"), "{temp}");
    let launchers = view::detail(&s, CleanKind::LauncherCaches);
    assert!(launchers.contains("Steam") && launchers.contains(" GB"), "{launchers}");
    let bin = view::detail(&s, CleanKind::RecycleBin);
    assert!(bin.ends_with("files") || bin.contains("files ·"), "{bin}");
    // a picture copy keeps the drawing's sample lines
    let mut p = opened();
    click(&mut p, &mut st, K_CLN);
    assert!(wait_for(&mut p, 5000, |p| p.plan.is_some()));
    assert_eq!(view::detail(&p, CleanKind::LauncherCaches), "Steam 1.2 GB · Epic 0.5 GB · Riot 0.2 GB");
}

/// REVIEW 022 HOLD 7 - errors: unreadable drives, a failed measure, a failed clean, a failed walk each say so in a toast and
/// leave the page usable.
#[test]
fn errors_say_so() {
    let mut s = opened();
    s.apply(Msg::Drives(Err(StorageError::Unsupported("no volumes".into()))));
    assert!(s.toast.as_ref().unwrap().0.starts_with("Can't read the drives · "));
    s.measuring = true;
    s.apply(Msg::Measured(Err(StorageError::Unsupported("x".into()))));
    assert!(!s.measuring && s.plan.is_none());
    assert!(s.toast.as_ref().unwrap().0.starts_with("Couldn't measure · "));
    s.cleaning = true;
    s.apply(Msg::Cleaned(Err(StorageError::Unsupported("x".into()))));
    assert!(!s.cleaning && s.report.is_none());
    assert!(s.toast.as_ref().unwrap().0.starts_with("Couldn't clean · "));
    let mut st = State::default();
    click(&mut s, &mut st, K_MEASURE);
    let Scan::Running { ctl, .. } = s.scan_of('C') else { panic!() };
    let ctl = ctl.clone();
    s.apply(Msg::Scanned('C', ctl, Err(StorageError::Unsupported("x".into())), "21:37".into(), std::time::SystemTime::now()));
    assert!(matches!(s.scan_of('C'), Scan::Idle), "Measure shows again");
    assert!(s.toast.as_ref().unwrap().0.starts_with("Couldn't measure C: · "));
    let _ = build(&mut s, &mut st, 0.0);
}

fn texts(l: &Laid) -> Vec<String> {
    l.nodes.iter().filter_map(|n| if let crate::ui::el::Content::Text(t) = &n.el.content { Some(t.s.clone()) } else { None }).collect()
}

/// REVIEW 022 HOLD 5: the drawing's Clean up timing - after Measure each size arrives at 300 + i*180 ms and the rows become
/// measured at 1000 ms; Clean counts each ticked row down (520 ms, rows 380 ms apart), then ✓, then the toast.
#[test]
fn clean_up_sizes_arrive_one_by_one_and_cleaned_rows_count_down() {
    let mut s = opened();
    let mut st = State::default();
    click(&mut s, &mut st, K_CLN); // at 0 ms
    let t0 = std::time::Instant::now();
    while s.staged.is_none() && t0.elapsed().as_secs() < 5 {
        s.tick(0.0);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(s.staged.is_some() && s.plan.is_none() && s.measuring);
    let t = texts(&build(&mut s, &mut st, 350.0));
    assert!(t.contains(&"6.4 GB".to_string()), "the first size is there at 350 ms");
    assert!(!t.contains(&"3.3 GB".to_string()), "the second arrives at 480 ms");
    let t = texts(&build(&mut s, &mut st, 900.0));
    assert!(t.contains(&"1.9 GB".to_string()) && t.contains(&"Measuring…".to_string()), "all four shown, still measuring");
    s.tick(999.0);
    assert!(s.plan.is_none());
    s.tick(1000.0);
    assert!(s.plan.is_some() && !s.measuring, "measured at 1000 ms");
    // Clean (all four ticked)
    click(&mut s, &mut st, K_CLN);
    while s.countdown.is_none() && t0.elapsed().as_secs() < 10 {
        s.tick(1000.0);
        std::thread::sleep(Duration::from_millis(5));
    }
    let c = s.countdown.as_ref().expect("counting down");
    assert_eq!(c.t0, 1000.0);
    assert_eq!(c.progress(CleanKind::RecycleBin, 1260.0, false), Some(0.5));
    assert_eq!(c.progress(CleanKind::TempFiles, 1260.0, false), Some(0.0), "the second starts 380 ms later");
    let t = texts(&build(&mut s, &mut st, 1260.0));
    assert!(t.contains(&"3.2 GB".to_string()), "6.4 GB half way down: {t:?}");
    assert!(s.cleaning && s.report.is_none() && s.toast.is_none());
    // the last of 4 rows ends at 3 * 380 + 520 ms
    s.tick(1000.0 + 3.0 * 380.0 + 519.0);
    assert!(s.report.is_none());
    s.tick(1000.0 + 3.0 * 380.0 + 520.0);
    assert!(s.report.is_some() && !s.cleaning);
    assert!(s.toast.as_ref().unwrap().0.starts_with("Freed "));
}

/// REVIEW 022 HOLD 5: the list slides in from the side it comes from (deeper +10 px, back -10 px), 220 ms.
#[test]
fn the_list_slides_in() {
    let mut s = opened();
    let mut st = State::default();
    click(&mut s, &mut st, K_MEASURE);
    assert!(wait_for(&mut s, 10_000, |s| matches!(s.scan_of('C'), Scan::Done { .. })));
    s.now = 0.0;
    click(&mut s, &mut st, idx(K_SEG, 1));
    assert_eq!(s.list_anim, Some((0.0, 0.0)));
    click(&mut s, &mut st, idx(K_FDR, 0));
    assert_eq!(s.list_anim.map(|a| a.1), Some(1.0));
    click(&mut s, &mut st, K_BACK);
    assert_eq!(s.list_anim.map(|a| a.1), Some(-1.0));
    // built mid-way and after: it builds; after 220 ms nothing moves any more
    let _ = build(&mut s, &mut st, 110.0);
    let mut st2 = State::default();
    let _ = build(&mut s, &mut st2, 400.0);
    assert!(!st2.busy, "no frames after the slide");
}

/// F2 (the owner Oct 8): closing the window / leaving the tab keeps the measure going and its result: the tab opens again on it
/// ("Measured ... ago"), nothing is measured again on its own.
#[test]
fn close_keeps_the_walk_and_its_result_for_the_next_open() {
    let e = Env { test: true, frozen: true, ..Env::default() };
    let mut s = Storage::default();
    s.open(&e, 0.0);
    let mut st = State::default();
    click(&mut s, &mut st, K_MEASURE);
    s.close();
    assert!(s.os.is_none() && s.drives.is_empty() && s.scans.is_empty() && s.health.is_empty(), "the page itself lets go");
    // the walk finishes while the tab is away; the next open shows it
    let mut s = Storage::default();
    s.open(&e, 10.0);
    assert!(matches!(s.scan_of('C'), Scan::Running { .. } | Scan::Done { .. }));
    assert!(wait_for(&mut s, 10_000, |s| matches!(s.scan_of('C'), Scan::Done { .. })));
    s.close();
    // the menu window closes: its page objects go
    drop(s);
    let mut s = Storage::default();
    s.open(&e, 20.0);
    assert!(matches!(s.scan_of('C'), Scan::Done { .. }), "kept, not measured again");
    // the menu closed in between (the page objects were dropped): only the top rows are kept; opening a folder scans just it
    let Scan::Done { result, .. } = s.scan_of('C').clone() else { panic!() };
    let rows = result.tree.rows(result.tree.root()).unwrap();
    let i = rows
        .iter()
        .position(|r| matches!(r.kind, scan::RowKind::Folder { id, has_subfolders: true, windows_own: false, .. } if result.tree.is_pruned(id)))
        .expect("a pruned folder");
    click(&mut s, &mut st, idx(K_FDR, i));
    assert!(s.sub.is_some() && s.path.is_empty(), "scanning that folder first");
    assert!(wait_for(&mut s, 10_000, |s| !s.path.is_empty()), "then in");
    let Scan::Done { result, .. } = s.scan_of('C').clone() else { panic!() };
    assert!(!result.tree.rows(s.path[0]).unwrap().is_empty());
    // a fresh app (another store) starts empty
    let mut s = opened();
    assert!(matches!(s.scan_of('C'), Scan::Idle));
    s.close();
}

/// Debug aid for the pixel proof: `BU_DUMP=<y0>,<y1> cargo test -p bu-app <page>::tests::dump_layout -- --nocapture` prints every
/// box whose top (window coordinates) is in [y0, y1) with its text - to compare with tools/ref/dom_dump.js. Does nothing otherwise.
#[test]
fn dump_layout() {
    let Ok(r) = std::env::var("BU_DUMP") else { return };
    let (y0, y1) = r.split_once(',').map(|(a, b)| (a.parse::<f32>().unwrap(), b.parse::<f32>().unwrap())).unwrap();
    let mut p = opened();
    let mut st = State::default();
    let l = build(&mut p, &mut st, 0.0);
    eprintln!("page height {}", l.height);
    for n in &l.nodes {
        let (x, y, w, h) = n.rect;
        let y = y + 56.0;
        if y >= y0 && y < y1 {
            let t = match &n.el.content {
                crate::ui::el::Content::Text(t) => format!("{:?}", t.s),
                crate::ui::el::Content::Icon(i) => format!("icon {}", i.name),
                _ => String::new(),
            };
            eprintln!("[{x:.2}, {y:.2}, {w:.2}, {h:.2}] {t}");
        }
    }
}

// ------------------------------------------------------------------ Order 039: Windows' Temp folder and Drive health through the elevated copy

/// Clean with the temp row ticked: Windows' own Temp folder (needs admin) is emptied by the elevated copy in the same press
/// - one prompt; its numbers join the temp row. A "No" leaves it and the toast says so.
#[test]
fn windows_temp_goes_through_the_elevated_copy() {
    for decline in [false, true] {
        let w = crate::admin::tests::world();
        let (hub, n) = crate::admin::tests::hub(&w, decline);
        crate::admin::client::set_for_test(hub);
        let mut s = Storage::default();
        s.open(&Env { test: true, frozen: false, ..Env::default() }, 0.0);
        s.fake.as_ref().unwrap().add_file(r"C:\Windows\Temp\setup.log", 4_000_000);
        let mut st = State::default();
        click(&mut s, &mut st, K_CLN);
        assert!(wait_for(&mut s, 5000, |s| s.plan.is_some()));
        assert!(s.plan.as_ref().unwrap().row(CleanKind::TempFiles).unwrap().parts.iter().any(|p| p.state == PartState::NeedsAdmin), "the fake PC is not elevated");
        click(&mut s, &mut st, K_CLN);
        assert!(wait_for(&mut s, 8000, |s| s.report.is_some()));
        let row = s.report.as_ref().unwrap().rows.iter().find(|r| r.kind == Some(CleanKind::TempFiles)).unwrap().clone();
        let t = s.toast.as_ref().unwrap().0.clone();
        assert_eq!(n.load(std::sync::atomic::Ordering::SeqCst), 1, "one prompt");
        if decline {
            assert!(row.skipped.iter().any(|(_, st)| *st == PartState::NeedsAdmin));
            assert!(t.contains("Windows temp folder: needs admin, not changed"), "{t}");
            assert_eq!(w.temp_cleans.load(std::sync::atomic::Ordering::SeqCst), 0);
        } else {
            assert!(!row.skipped.iter().any(|(_, st)| *st == PartState::NeedsAdmin), "{row:?}");
            assert!(row.in_use_files >= 1 && row.freed_files >= 3, "the copy's numbers joined the row: {row:?}");
            assert!(!t.contains("not changed"), "{t}");
            assert_eq!(w.temp_cleans.load(std::sync::atomic::Ordering::SeqCst), 1);
        }
    }
}

/// A_039_01: "Read with admin" shows only while a drive gave less than an admin read would; the click reads those
/// drives in the elevated copy (one prompt, read-only); the rows stay filled; a "No" changes nothing.
#[test]
fn drive_health_reads_with_admin_on_the_click() {
    for decline in [true, false] {
        let w = crate::admin::tests::world();
        let (hub, n) = crate::admin::tests::hub(&w, decline);
        crate::admin::client::set_for_test(hub);
        let mut s = opened();
        let mut st = State::default();
        assert!(!s.health_link(), "the drawing's drives are all read");
        assert!(build(&mut s, &mut st, 0.0).rect_of(K_HADM).is_none());
        // this PC: one SATA drive (disk 1) whose SMART needs admin
        let f = bu_storage::FakeOs::new();
        f.add_disk(
            bu_storage::PhysicalDisk { number: 1, model: "SATA SSD".into(), media: bu_storage::MediaKind::Ssd, bus: Some("SATA".into()), size_bytes: 500 << 30 },
            bu_storage::HealthRaw { os_status: Some(bu_storage::OsHealthStatus::Healthy), needs_admin: vec!["SMART attributes".into()], ..Default::default() },
        );
        let os: Arc<dyn StorageOs> = Arc::new(f);
        s.health = health::read_all(os.as_ref()).unwrap();
        s.os = Some(os);
        assert!(s.health_link());
        assert!(build(&mut s, &mut st, 0.0).rect_of(K_HADM).is_some(), "the link shows");
        assert_eq!(s.health[0].power_on_hours, None);
        click(&mut s, &mut st, K_HADM);
        assert!(wait_for(&mut s, 5000, |s| !s.health_reading));
        assert_eq!(n.load(std::sync::atomic::Ordering::SeqCst), 1, "one prompt");
        if decline {
            assert_eq!(s.toast.as_ref().unwrap().0, crate::admin::NOT_CHANGED);
            assert!(s.health_link(), "nothing changed, the link stays");
        } else {
            assert_eq!(s.health[0].power_on_hours, Some(12345), "the copy's read");
            assert!(!s.health_link());
            assert!(build(&mut s, &mut st, 0.0).rect_of(K_HADM).is_none(), "the rows stay filled; no link");
        }
    }
}

/// Order 039 picture check (only with `BU_RENDER_DIR` set): the Storage page with a drive read without admin - Drive
/// health's last row "Some drive details can only be read with admin" + the shield + "Read with admin", painted off-screen
/// with the app's own painter (no window, no screen), dark and light.
#[test]
fn picture_of_the_read_with_admin_link() {
    let Ok(dir) = std::env::var("BU_RENDER_DIR") else { return };
    for light in [false, true] {
        crate::ui::set_light(light);
        let mut s = opened();
        for h in s.health.iter_mut().filter(|h| h.media == bu_storage::MediaKind::Hdd) {
            h.temperature_c = None;
            h.power_on_hours = None;
            h.admin_would_add = vec!["SMART attributes".into()];
        }
        let g = Gfx::new(1.0);
        let icons = crate::icons::Icons::new();
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        let kids = s.build(&mut cx);
        let laid = Laid::new(&g, El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids), 600.0, None);
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let h = laid.height.ceil() as i32;
        let mut sf = crate::gfx::new_surface(600, h).unwrap();
        g.begin(sf.canvas());
        let bg = if light { crate::gfx::Rgba::rgb(236, 239, 245) } else { crate::gfx::Rgba::rgb(20, 24, 40) };
        g.fill_rect(0.0, 0.0, 600.0, h as f32, bg);
        laid.paint(&g, &icons, 0.0, 0.0, None);
        g.end();
        let px = crate::png::from_surface(&mut sf);
        crate::png::save_png(&px, &format!("{dir}/storage_read_with_admin_{}.png", if light { "light" } else { "dark" })).expect("save");
    }
    crate::ui::set_light(false);
}
