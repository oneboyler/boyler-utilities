//! Idle-cost measurement of the parts that run while the menu is closed: the app watcher (the bu-procwatch window-creation
//! hook + one exit wait on the thread pool) and an armed keep countdown. Prints the process CPU time used over the window
//! and its memory. Our own CPU now includes the hook callbacks (one per new window anywhere); WmiPrvSE no longer re-reads the
//! process list for us.
//! Changes nothing (the watched exe names match nothing; the countdown has no pending change to revert).
//!
//!   cargo run -p bu-display --example display-idle -- <seconds> [no-rules]

use bu_display::fake::FakeDisplayOs;
use bu_display::win::watch::AppWatcher;
use bu_display::{keep::KeepTimer, DisplayService};
use std::os::windows::process::CommandExt;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

/// Windows' own tools are started from `%SystemRoot%\System32` by full path, never by bare name (TECH_RULES).
fn system32(exe: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join("System32").join(exe)
}

fn cpu_100ns() -> u64 {
    let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) }.expect("GetProcessTimes");
    let f = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
    f(k) + f(u)
}

fn main() {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(60);
    let _svc = DisplayService::new(FakeDisplayOs::two_monitors());
    let names: Vec<String> = if std::env::args().nth(2).as_deref() == Some("no-rules") { vec![] } else { vec!["nothing-matches-this.exe".into()] };
    let w = AppWatcher::start(names, |e| println!("event {e:?}")).expect("watcher");
    println!("active source: {:?}", w.active_source());
    // One exit wait on our own process (never ends during the run) = the state while a tracked game runs.
    w.report_start(std::process::id(), "idle.exe".into());
    let _keep = KeepTimer::start(Duration::from_secs(secs + 30), || {});
    std::thread::sleep(Duration::from_millis(500)); // let start-up settle
    let (c0, t0) = (cpu_100ns(), Instant::now());
    std::thread::sleep(Duration::from_secs(secs));
    let (c1, wall) = (cpu_100ns(), t0.elapsed());
    let cpu_ms = (c1 - c0) as f64 / 10_000.0;
    println!("window {:.1} s: CPU used {:.3} ms = {:.5} % of one core", wall.as_secs_f64(), cpu_ms, cpu_ms / wall.as_secs_f64() / 10.0);
    let out = std::process::Command::new(system32("tasklist.exe"))
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .args(["/FI", &format!("PID eq {}", std::process::id()), "/FO", "CSV", "/NH"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    println!("memory (tasklist): {}", out.trim());
    w.stop();
}
