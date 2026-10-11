//! Order 081's check: can the app see a game controller's presses on this PC (read-only, event-driven)?
//!
//! `cargo run -p bu-rawin --example rawin-padcheck -- [seconds]` lists the controllers Windows shows to Raw Input, listens
//! for `seconds` (default 10) through the SAME listener the Keyboard tab uses (listen-only, nothing sent to the pad, no window on
//! screen), and prints what it saw: reports read, how many it could decode, which classes of button / trigger went down or up
//! (counts only - never which button, never saved), and the process's CPU time while listening.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bu_rawin::{PadSoundClass, PadSoundEvent};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

fn cpu_100ns() -> u64 {
    let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    // SAFETY: our own process handle and four FILETIMEs.
    unsafe {
        let _ = GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
    }
    let t = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
    t(k) + t(u)
}

fn main() {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let pads = bu_rawin::list_pads();
    println!("controllers Windows shows to Raw Input: {}", pads.len());
    for p in &pads {
        println!("  {:04X}:{:04X}  {}", p.vendor, p.product, p.path);
    }
    let counts = Arc::new([AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)]);
    let c = counts.clone();
    let sink: bu_rawin::PadSink = Arc::new(move |e: PadSoundEvent| {
        let i = match e.class {
            PadSoundClass::Button => 0,
            PadSoundClass::TriggerLeft => 2,
            PadSoundClass::TriggerRight => 4,
        } + usize::from(!e.down);
        c[i].fetch_add(1, Ordering::Relaxed);
    });
    if let Err(e) = bu_rawin::set_pad_sound(Some(sink)) {
        println!("Windows refused the registration: {e}");
        return;
    }
    println!("listening for {secs} s (press buttons / pull triggers on the pad if you like) ...");
    let (cpu0, t0) = (cpu_100ns(), Instant::now());
    let mut last = Default::default();
    while t0.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_millis(250));
        let s = bu_rawin::pad_stats();
        if s != last {
            println!("  {:>4.1} s  reports {:>6}  decoded {:>6}  controllers {}  events {}", t0.elapsed().as_secs_f32(), s.packets, s.decoded, s.devices, s.events);
            last = s;
        }
    }
    let (cpu, wall) = (cpu_100ns() - cpu0, t0.elapsed());
    let s = bu_rawin::pad_stats();
    let n = |i: usize| counts[i].load(Ordering::Relaxed);
    println!("reports read {} (decoded {}), controllers that sent reports {}", s.packets, s.decoded, s.devices);
    println!("buttons down {} up {} | left trigger down {} up {} | right trigger down {} up {}", n(0), n(1), n(2), n(3), n(4), n(5));
    println!(
        "process CPU while listening: {:.3} % of one core ({} ms of CPU in {:.1} s)  [reports per second: {:.0}]",
        cpu as f64 / 1e7 / wall.as_secs_f64() * 100.0,
        cpu / 10_000,
        wall.as_secs_f64(),
        s.packets as f64 / wall.as_secs_f64()
    );
    let _ = bu_rawin::set_pad_sound(None);
}
