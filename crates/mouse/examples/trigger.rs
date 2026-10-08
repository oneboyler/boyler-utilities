//! `cargo run -p bu-mouse --example mouse-trigger` — proves the per-app trigger on the real PC without touching anything:
//! watches for `ping.exe`, starts ONE hidden `ping -n 3 127.0.0.1` itself, and prints the start / stop events and which
//! source is listening. A hidden ping makes no window, so `bu_procwatch::rescan()` stands in for the window a game would
//! create (the same snapshot path). Nothing is written anywhere; Raw Accel is not involved.

use bu_mouse::win::watch::AppWatcher;
use std::os::windows::process::CommandExt;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn main() {
    let (tx, rx) = mpsc::channel();
    let t0 = Instant::now();
    let w = match AppWatcher::start(vec!["PING.EXE".into()], move |e| {
        let _ = tx.send((t0.elapsed(), e));
    }) {
        Ok(w) => w,
        Err(e) => {
            println!("watcher refused: {e}");
            return;
        }
    };
    println!("watcher source: {:?}", w.active_source());
    std::thread::sleep(Duration::from_millis(300)); // its first snapshot (what already runs) is taken
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let started = Instant::now();
    // by full path (TECH_RULES, Order 011: system tools are started from %SystemRoot%\System32, never by bare name)
    let ping = std::path::PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join(r"System32\PING.EXE");
    let child = std::process::Command::new(ping).args(["-n", "3", "127.0.0.1"]).creation_flags(CREATE_NO_WINDOW).stdout(std::process::Stdio::null()).spawn();
    let pid = match child {
        Ok(mut c) => {
            let pid = c.id();
            std::thread::spawn(move || {
                let _ = c.wait();
            });
            pid
        }
        Err(e) => {
            println!("could not start ping: {e}");
            return;
        }
    };
    println!("started hidden ping, pid {pid}, at {:?}", started.duration_since(t0));
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut got_stop = false;
    let mut got_start = false;
    while Instant::now() < deadline {
        if !got_start {
            bu_procwatch::rescan(); // stands in for the window a game would create
        }
        let Ok((at, e)) = rx.recv_timeout(Duration::from_millis(200)) else { continue };
        println!("event at {at:?}: {e:?}");
        match e {
            bu_mouse::accel::switch::AppEvent::Started { pid: p, .. } if p == pid => got_start = true,
            bu_mouse::accel::switch::AppEvent::Stopped { pid: p } if p == pid => {
                got_stop = true;
                break;
            }
            _ => {}
        }
    }
    println!("{}", if got_stop { "start + stop seen" } else { "no stop seen within 20 s" });
}
