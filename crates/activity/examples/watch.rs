//! `cargo run -p bu-activity --example activity-watch -- <seconds> <scratch folder>` — the REAL watcher for a few
//! minutes, for the idle-cost measurement. It reads which app is in front (read-only) and writes its day file ONLY into
//! the scratch folder given (refused unless it is inside `BoylerUtilities-board\scratch\lane-j\`, no `..`). Prints the CPU time and memory
//! this process used while watching, how often it woke, and the day file's size.

use bu_activity::activity::Activity;
use bu_activity::views::summary;
use bu_activity::watch::Watcher;
use bu_activity::{ActivityOs, Clock, FileStore, RealOs, SystemClock};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

fn cpu_ms() -> f64 {
    let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    // SAFETY: reads our own process times.
    unsafe {
        let _ = GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
    }
    let t = |f: FILETIME| (((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64) as f64 / 10_000.0;
    t(k) + t(u)
}

fn cycles() -> u64 {
    let mut c = 0u64;
    // SAFETY: reads our own process's cycle count (exact, unlike the 15.6 ms-tick CPU times).
    unsafe {
        let _ = windows::Win32::System::WindowsProgramming::QueryProcessCycleTime(GetCurrentProcess(), &mut c);
    }
    c
}

fn mem() -> (usize, usize) {
    let mut m = PROCESS_MEMORY_COUNTERS { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
    // SAFETY: reads our own memory counters.
    unsafe {
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut m, m.cb);
    }
    (m.WorkingSetSize, m.PeakWorkingSetSize)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let secs: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(60);
    let Some(dir) = args.next() else {
        println!("usage: activity-watch <seconds> <folder inside BoylerUtilities-board\\scratch\\lane-j\\>");
        return;
    };
    // only inside this lane's scratch folder (no `..` tricks): FileStore::in_scratch refuses anything else
    let root = std::path::Path::new(r"C:\BoylerUtilities-scratch\lane-j");
    let store = match FileStore::in_scratch(&dir, root) {
        Ok(s) => s,
        Err(e) => {
            println!("{e}");
            return;
        }
    };
    let mut os = RealOs;
    let now = SystemClock.now();
    let mut a = Activity::new(store, os.game_roots(), now);
    if let Err(e) = a.set_on(true, now) {
        println!("can't write the scratch folder: {e}");
        return;
    }
    let w = match Watcher::start(a) {
        Ok(w) => w,
        Err(e) => {
            println!("watcher failed: {e}");
            return;
        }
    };
    // settle (start-up work: reading launcher folders, first app name), then measure the watching only
    std::thread::sleep(Duration::from_secs(2));
    let (c0, y0, t0) = (cpu_ms(), cycles(), Instant::now());
    std::thread::sleep(Duration::from_secs(secs));
    let (cpu, cyc, wall) = (cpu_ms() - c0, cycles() - y0, t0.elapsed().as_secs_f64() * 1000.0);
    println!("CPU cycles used while watching: {cyc}");
    let (ws, peak) = mem();
    let s = &w.stats;
    println!(
        "watched {:.0} s: CPU {:.1} ms = {:.4} % of one core; working set {:.1} MB (peak {:.1} MB)",
        wall / 1000.0,
        cpu,
        cpu / wall * 100.0,
        ws as f64 / 1_048_576.0,
        peak as f64 / 1_048_576.0
    );
    println!(
        "wakes: {} foreground changes, {} idle checks, {} input-after-idle, {} lock/power",
        s.foreground_events.load(Ordering::Relaxed),
        s.idle_timer_wakes.load(Ordering::Relaxed),
        s.input_wakes.load(Ordering::Relaxed),
        s.other_messages.load(Ordering::Relaxed)
    );
    let up = os.uptime_ms();
    let sum = w.with(|a| summary(a, SystemClock.now(), up));
    println!("screen: {} | games: {} | uptime: {}", sum.screen_text, sum.games_text, sum.uptime_text);
    for r in sum.today.iter().take(6) {
        println!("  {:<32} {:>8}{}", r.name, r.text, if r.game { "  [Game]" } else { "" });
    }
    drop(w); // saves
    for (d, size) in FileStore::new(&dir).days_on_disk() {
        println!("day file {}.tsv: {size} bytes", d.iso());
    }
}
