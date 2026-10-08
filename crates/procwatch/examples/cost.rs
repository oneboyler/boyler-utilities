//! Order 048 proof (item 4): the shared window-creation watcher's own cost - it listens for a made-up exe name (no game
//! is ever matched or switched) for N seconds and prints this process's CPU per thread. WmiPrvSE is measured from outside
//! (it must stay at its baseline: this watcher never asks WMI anything).
//!
//! `cargo run --release -p bu-procwatch --example procwatch-cost -- [seconds] [exe name]`

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

fn main() {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(60);
    let name = std::env::args().nth(2).unwrap_or_else(|| "bu-048-test.exe".into());
    let hits = Arc::new(AtomicU64::new(0));
    let h2 = hits.clone();
    // two subscribers, as the display and the mouse rules are
    let a = bu_procwatch::subscribe(vec![name.clone()], Arc::new(move |pid, exe| {
        h2.fetch_add(1, Ordering::Relaxed);
        println!("start: {pid} {exe}");
    }));
    let b = bu_procwatch::subscribe(vec![name.clone()], Arc::new(|_, _| {}));
    let cpu = || {
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        unsafe {
            let _ = GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
        }
        let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 10_000.0;
        ft(k) + ft(u)
    };
    std::thread::sleep(Duration::from_millis(500));
    let c0 = cpu();
    std::thread::sleep(Duration::from_secs(secs));
    let c1 = cpu();
    println!(
        "procwatch: listening for {name} {secs} s: {:.1} ms CPU = {:.4} % of one core; starts reported {}; problem: {:?}",
        c1 - c0,
        (c1 - c0) / (secs as f64 * 10.0),
        hits.load(Ordering::Relaxed),
        bu_procwatch::problem()
    );
    drop((a, b));
}
