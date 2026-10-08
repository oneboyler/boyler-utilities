//! Windows-layer parts that can be proven without changing anything: the undocumented DPI packet maths, the EDID
//! size parser, and the app watcher (source fallback, start + exit of a hidden throw-away `ping` we start; the test with the
//! real window-creation hook is #[ignore]d: tests install no real WinEvent hook).
#![cfg(windows)]

use bu_display::autoswitch::AppEvent;
use bu_display::win::{dpi, edid, watch};
use std::os::windows::process::CommandExt;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// Windows' own tools are started from `%SystemRoot%\System32` by full path, never by bare name (TECH_RULES).
fn system32(exe: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join("System32").join(exe)
}

#[test]
fn dpi_steps_decode_and_encode() {
    // Recommended 150 % (min is 2 steps down), current 175 %, max 225 %.
    let s = dpi::decode(-2, 1, 3).unwrap();
    assert_eq!((s.current_percent, s.recommended_percent), (175, 150));
    assert_eq!(s.allowed_percent, vec![100, 125, 150, 175, 200, 225]);
    assert_eq!(dpi::encode(&s, 100), Some(-2));
    assert_eq!(dpi::encode(&s, 225), Some(3));
    assert_eq!(dpi::encode(&s, 250), None);
    assert_eq!(dpi::encode(&s, 110), None);
    // Measured on a test PC: both monitors recommended 100 %.
    let z = dpi::decode(0, 0, 3).unwrap();
    assert_eq!(z.allowed_percent, vec![100, 125, 150, 175]);
    // Out-of-range packets (a changed Windows layout) are refused, never indexed blindly.
    assert!(dpi::decode(-20, 0, 3).is_none());
    assert!(dpi::decode(0, 0, 40).is_none());
    assert_eq!(dpi::decode(0, 9, 2).unwrap().current_percent, 150); // current clamped into range
}

#[test]
fn edid_diagonal() {
    let mut e = vec![0u8; 128];
    e[0..8].copy_from_slice(&[0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0]);
    // Detailed timing: pixel clock non-zero, 597 x 336 mm (a 27" 16:9 panel).
    e[54] = 0x01;
    e[55] = 0x02;
    e[66] = (597 & 0xFF) as u8;
    e[67] = (336 & 0xFF) as u8;
    e[68] = (((597 >> 8) as u8) << 4) | ((336 >> 8) as u8);
    assert_eq!(edid::diagonal_inches(&e), Some(27.0));
    // No detailed size → basic cm size (60 x 34 cm).
    e[54] = 0;
    e[55] = 0;
    e[21] = 60;
    e[22] = 34;
    assert_eq!(edid::diagonal_inches(&e), Some(27.2));
    assert_eq!(edid::diagonal_inches(&[0u8; 10]), None);
}

#[test]
fn exe_name_from_snapshot_without_a_handle() {
    let me = watch::exe_name_of(std::process::id()).unwrap();
    assert!(me.starts_with("win_parts") && me.ends_with(".exe"), "{me}");
    assert_eq!(watch::exe_name_of(u32::MAX - 3), None);
    assert!(!watch::has_visible_window(u32::MAX - 3));
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn hidden_ping(count: u32) -> std::process::Child {
    std::process::Command::new(system32("PING.EXE"))
        .args(["-n", &count.to_string(), "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .unwrap()
}

#[test]
fn no_rules_means_no_subscription() {
    let w = watch::AppWatcher::start(vec![], |_| {}).unwrap();
    assert_eq!(w.active_source(), None);
}

/// A fake source that is refused and one that works: the watcher must fall back to the second.
struct Refuses;
impl watch::ProcessStartSource for Refuses {
    fn name(&self) -> &'static str {
        "refuses"
    }
    fn start(&self, _: &[String], _: watch::StartSink, _: watch::LostSink) -> bu_display::Result<Box<dyn Send>> {
        Err(bu_display::DisplayError::Watcher("access denied (fake)".into()))
    }
}
struct Works(Arc<Mutex<Option<watch::StartSink>>>);
impl watch::ProcessStartSource for Works {
    fn name(&self) -> &'static str {
        "works"
    }
    fn start(&self, _: &[String], s: watch::StartSink, _: watch::LostSink) -> bu_display::Result<Box<dyn Send>> {
        *self.0.lock().unwrap() = Some(s);
        Ok(Box::new(()))
    }
}

#[test]
fn watcher_falls_back_to_the_next_source_and_reports_start_then_exit() {
    let slot = Arc::new(Mutex::new(None));
    let (tx, rx) = mpsc::channel();
    let w = watch::AppWatcher::start_with(vec![Arc::new(Refuses), Arc::new(Works(slot.clone()))], vec!["ping.exe".into()], move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    assert_eq!(w.active_source(), Some("works"));
    let mut child = hidden_ping(2);
    let pid = child.id();
    (slot.lock().unwrap().as_ref().unwrap())(pid, "ping.exe".into());
    match rx.recv_timeout(Duration::from_secs(60)).unwrap() {
        AppEvent::Started { pid: p, exe, has_window, .. } => {
            assert_eq!(p, pid);
            assert!(exe.to_ascii_lowercase().ends_with("ping.exe"), "{exe}");
            assert!(!has_window);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(rx.recv_timeout(Duration::from_secs(60)).unwrap(), AppEvent::Stopped { pid });
    let _ = child.wait();
}

#[test]
fn all_sources_refusing_is_a_typed_error() {
    let r = watch::AppWatcher::start_with(vec![Arc::new(Refuses)], vec!["x.exe".into()], |_| {});
    assert!(matches!(r, Err(bu_display::DisplayError::Watcher(_))));
}

#[test]
#[ignore = "installs the real window-creation WinEvent hook (run by hand: cargo test -p bu-display --test win_parts -- --ignored)"]
fn real_watcher_reports_a_process_start_and_exit() {
    // REAL: the default sources (admin trace only if elevated, then the bu-procwatch window-creation watcher) for
    // "ping.exe"; a hidden ping we start. A hidden ping makes no window, so `rescan` stands in for the window a game
    // would create (the same snapshot path runs).
    let (tx, rx) = mpsc::channel();
    let w = watch::AppWatcher::start(vec!["PING.EXE".into()], move |e| {
        let _ = tx.send((e, Instant::now()));
    })
    .unwrap();
    let src = w.active_source().unwrap();
    println!("active source: {src}");
    std::thread::sleep(Duration::from_millis(300)); // the watcher's first snapshot (what already runs) is taken
    let t0 = Instant::now();
    let mut child = hidden_ping(3);
    let pid = child.id();
    // Other terminals may run ping too: only our pid counts.
    let deadline = Instant::now() + Duration::from_secs(60); // generous: only an upper bound, PC load can slow it
    let mut started_at = None;
    let mut stopped = false;
    while Instant::now() < deadline && !stopped {
        if started_at.is_none() {
            bu_procwatch::rescan();
        }
        if let Ok((e, at)) = rx.recv_timeout(Duration::from_millis(200)) {
            match e {
                AppEvent::Started { pid: p, has_window, .. } if p == pid => {
                    assert!(!has_window);
                    started_at = Some(at);
                }
                AppEvent::Stopped { pid: p } if p == pid => stopped = true,
                _ => {}
            }
        }
    }
    let _ = child.wait();
    let started_at = started_at.expect("start reported");
    println!("start reported {} ms after spawn", (started_at - t0).as_millis());
    assert!(stopped, "exit reported");
}

#[test]
fn watcher_drop_releases_exit_waits_without_events() {
    let slot = Arc::new(Mutex::new(None));
    let (tx, rx) = mpsc::channel();
    let w = watch::AppWatcher::start_with(vec![Arc::new(Works(slot.clone()))], vec!["ping.exe".into()], move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    // A long ping (~60 s) so it can never end before `drop(w)`, even under load; we kill our own child at the end.
    let mut child = hidden_ping(60);
    w.report_start(child.id(), "ping.exe".into());
    assert!(matches!(rx.recv_timeout(Duration::from_secs(60)).unwrap(), AppEvent::Started { .. }));
    drop(w); // the exit wait is unregistered: no Stopped is sent
    let _ = child.kill();
    let _ = child.wait();
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}

#[test]
fn dpi_packet_with_an_unknown_layout_is_refused_not_panicking() {
    // Windows reports min <= 0 <= max relative to "recommended"; anything else (or min > max) → None, never a panic.
    assert!(dpi::decode(1, 0, 3).is_none());
    assert!(dpi::decode(0, 0, -1).is_none());
    assert!(dpi::decode(2, 5, -3).is_none());
}

/// Starts fine, then stops working later (like WMI refusing the admin trace only after the refuse wait).
struct DiesLater(Arc<Mutex<Option<watch::LostSink>>>);
impl watch::ProcessStartSource for DiesLater {
    fn name(&self) -> &'static str {
        "dies-later"
    }
    fn start(&self, _: &[String], _: watch::StartSink, lost: watch::LostSink) -> bu_display::Result<Box<dyn Send>> {
        *self.0.lock().unwrap() = Some(lost);
        Ok(Box::new(()))
    }
}

#[test]
fn a_source_that_dies_later_hands_over_to_the_next_one() {
    let dies = Arc::new(Mutex::new(None));
    let slot = Arc::new(Mutex::new(None));
    let (tx, rx) = mpsc::channel();
    let w = watch::AppWatcher::start_with(vec![Arc::new(DiesLater(dies.clone())), Arc::new(Works(slot.clone()))], vec!["ping.exe".into()], move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    assert_eq!(w.active_source(), Some("dies-later"));
    assert!(slot.lock().unwrap().is_none(), "the second source is not started while the first runs");
    (dies.lock().unwrap().as_ref().unwrap())("access denied (late)".into());
    // The hand-over runs on its own thread: wait for it (upper bound only).
    let give_up = Instant::now() + Duration::from_secs(60);
    while w.active_source() != Some("works") && Instant::now() < give_up {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(w.active_source(), Some("works"));
    let p = w.last_problem().unwrap();
    assert!(p.contains("dies-later") && p.contains("access denied (late)") && p.contains("works"), "{p}");
    // The new source really feeds the watcher.
    let mut child = hidden_ping(2);
    (slot.lock().unwrap().as_ref().unwrap())(child.id(), "ping.exe".into());
    assert!(matches!(rx.recv_timeout(Duration::from_secs(60)).unwrap(), AppEvent::Started { .. }));
    let _ = child.wait();
}

#[test]
fn a_process_already_gone_is_reported_stopped_at_once() {
    let slot = Arc::new(Mutex::new(None));
    let (tx, rx) = mpsc::channel();
    let _w = watch::AppWatcher::start_with(vec![Arc::new(Works(slot.clone()))], vec!["ping.exe".into()], move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    let mut child = hidden_ping(1);
    let pid = child.id();
    let _ = child.wait(); // ended before its start is handled (and its handle is closed by `wait`'s Child drop below)
    drop(child);
    (slot.lock().unwrap().as_ref().unwrap())(pid, "ping.exe".into());
    assert!(matches!(rx.recv_timeout(Duration::from_secs(60)).unwrap(), AppEvent::Started { .. }));
    assert_eq!(rx.recv_timeout(Duration::from_secs(60)).unwrap(), AppEvent::Stopped { pid });
}
