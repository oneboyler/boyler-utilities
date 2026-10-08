//! `cargo run -p bu-audio --example audio-listen -- <seconds>` — the Keep my devices / New apps volume listener on
//! the REAL PC in READ-ONLY mode (it decides, but the OS layer refuses every change): prints what Windows reported, what
//! it would have done, and the CPU cycles this process used while listening (for the idle-cost measurement). Default 60 s.

use bu_audio::watch::{WatchConfig, Watcher};
use std::time::{Duration, Instant};

fn cycles() -> u64 {
    let mut c = 0u64;
    // SAFETY: reads our own process's cycle count.
    unsafe {
        let _ = windows::Win32::System::WindowsProgramming::QueryProcessCycleTime(windows::Win32::System::Threading::GetCurrentProcess(), &mut c);
    }
    c
}

fn main() {
    let secs: u64 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(60);
    let w = Watcher::start(WatchConfig { read_only: true, ..WatchConfig::default() });
    println!("listening {secs} s (read-only), pid {}", std::process::id());
    // settle (start-up registration), then measure the listening only
    std::thread::sleep(Duration::from_secs(2));
    let (c0, t0) = (cycles(), Instant::now());
    std::thread::sleep(Duration::from_secs(secs));
    println!("listened {:.0} s: {} CPU cycles used by this process", t0.elapsed().as_secs_f64(), cycles() - c0);
    for l in w.log() {
        println!("{l}");
    }
}
