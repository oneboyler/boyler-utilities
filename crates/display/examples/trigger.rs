//! Real measurement of the per-app trigger, with hidden throw-away `ping` processes we start ourselves (never a game):
//! how long the watcher takes to start (incl. the admin trace being refused), which source it ends up on, and how
//! long after a process starts / ends the event lands. Changes nothing.
//!
//!   cargo run -p bu-display --example display-trigger -- [runs]

use bu_display::autoswitch::AppEvent;
use bu_display::win::watch::{AppWatcher, ProcessStartSource, WmiStartTrace};
use std::sync::Arc;
use std::os::windows::process::CommandExt;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Windows' own tools are started from `%SystemRoot%\System32` by full path, never by bare name (TECH_RULES).
fn system32(exe: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join("System32").join(exe)
}

fn main() {
    let runs: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    // How fast WMI refuses (or accepts) the admin-only trace on its own — the REFUSE_WAIT in win::wmi is 1.5 s.
    for i in 1..=3 {
        let t = Instant::now();
        let r = WmiStartTrace.start(&["ping.exe".into()], Arc::new(|_, _| {}), Arc::new(|_| {}));
        let ms = t.elapsed().as_millis();
        match r {
            Ok(_) => println!("admin trace try {i}: ACCEPTED (elevated?) after {ms} ms"),
            Err(e) => println!("admin trace try {i}: refused after {ms} ms: {e}"),
        }
    }
    let (tx, rx) = mpsc::channel();
    let t = Instant::now();
    let w = AppWatcher::start(vec!["ping.exe".into()], move |e| {
        let _ = tx.send((e, Instant::now()));
    })
    .expect("watcher");
    println!("watcher started in {} ms, source: {:?}", t.elapsed().as_millis(), w.active_source());
    let (mut starts, mut stops) = (vec![], vec![]);
    for i in 0..runs {
        let spawned = Instant::now();
        let mut child = std::process::Command::new(system32("PING.EXE"))
            .args(["-n", "3", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
            .expect("ping");
        let pid = child.id();
        let mut started = None;
        let mut ended_at = None;
        let mut stopped = None;
        let deadline = Instant::now() + Duration::from_secs(12);
        while Instant::now() < deadline && stopped.is_none() {
            if ended_at.is_none() {
                if let Ok(Some(_)) = child.try_wait() {
                    ended_at = Some(Instant::now());
                }
            }
            if let Ok((e, at)) = rx.recv_timeout(Duration::from_millis(5)) {
                match e {
                    AppEvent::Started { pid: p, has_window, .. } if p == pid => started = Some((at - spawned, has_window)),
                    AppEvent::Stopped { pid: p } if p == pid => stopped = Some(at),
                    _ => {}
                }
            }
        }
        let _ = child.wait();
        let ended_at = ended_at.unwrap_or_else(Instant::now);
        let s = started.map(|(d, _)| d.as_millis() as i64).unwrap_or(-1);
        let x = stopped.map(|at| at.saturating_duration_since(ended_at).as_millis() as i64).unwrap_or(-1);
        println!("run {:2}: start event after {:5} ms (has_window {:?}), stop event {:3} ms after the process ended", i + 1, s, started.map(|v| v.1), x);
        if s >= 0 {
            starts.push(s);
        }
        if x >= 0 {
            stops.push(x);
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    let stat = |v: &Vec<i64>| {
        let mut v = v.clone();
        v.sort();
        if v.is_empty() {
            return "none".to_string();
        }
        format!("n {} · min {} · median {} · max {} ms", v.len(), v[0], v[v.len() / 2], v[v.len() - 1])
    };
    println!("start delay: {}", stat(&starts));
    println!("stop delay : {}", stat(&stops));
}
