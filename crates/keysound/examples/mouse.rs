//! Order 064's proof on the real PC: what listening to the mouse buttons costs. MUTED: the engine's own audio session is muted
//! (`set_test_mute`), so a click that plays can't be heard. The mouse (and the keyboard, for the key sink `enable` also
//! registers) is only LISTENED to, as the real app does: nothing is blocked, changed or stored; only counters are read.
//!
//! `cargo run -p bu-keysound --example keysound-mouse -- <seconds> [--play]` prints, for the window: this process's CPU cycles (exact)
//! and CPU time, how many raw packets the raw-input thread read (mouse moves included) in how many batches, how many sounds
//! the engine started (muted), and the cost per packet. The numbers mean something only if the mouse is used during the window.

#[cfg(windows)]
fn main() {
    use bu_keysound::{KeySounds, Settings};
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
    fn ram_mb() -> f64 {
        let mut m = PROCESS_MEMORY_COUNTERS { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
        // SAFETY: reads our own memory counters.
        unsafe {
            let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut m, m.cb);
        }
        m.WorkingSetSize as f64 / 1_048_576.0
    }

    let secs: u64 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(20);
    // default: both volumes 0, so every click and key is heard and DROPPED (no stream is ever opened): the cost of listening alone.
    // `--play`: the sounds really play (muted), so the stream and the mixer run as well.
    let play = std::env::args().any(|a| a == "--play");
    let ks = KeySounds::new();
    ks.set_test_mute(true);
    let s = Settings { mouse_on: true, volume: if play { 5 } else { 0 }, mouse_volume: if play { 5 } else { 0 }, ..Settings::default() };
    if let Err(e) = ks.enable(s) {
        println!("could not start: {e}");
        return;
    }
    println!("listening to the mouse for {secs} s (muted; mouse listening = {}) ...", ks.status().mouse_listening);
    std::thread::sleep(Duration::from_millis(500));
    let (c0, t0, r0, p0, w0) = (cycles(), cpu_ms(), bu_rawin::stats(), ks.status().plays, Instant::now());
    std::thread::sleep(Duration::from_secs(secs));
    let (c1, t1, r1, p1, w1) = (cycles(), cpu_ms(), bu_rawin::stats(), ks.status().plays, Instant::now());
    let wall = w1.duration_since(w0).as_secs_f64();
    let packets = r1.packets_seen - r0.packets_seen;
    let batches = r1.batches - r0.batches;
    println!("window {wall:.1} s: {packets} raw packets read in {batches} batches, {} sounds started (muted)", p1 - p0);
    println!("process CPU: {:.2} ms ({:.4} % of one core), {} M cycles; RAM {:.1} MB", t1 - t0, (t1 - t0) / (wall * 10.0), (c1 - c0) / 1_000_000, ram_mb());
    if packets > 0 {
        println!("per packet: {:.0} cycles", (c1 - c0) as f64 / packets as f64);
    } else {
        println!("no packet came in the window (the mouse wasn't used): cost of listening while nothing happens = {} M cycles", (c1 - c0) / 1_000_000);
    }
    ks.disable();
    println!("released: mouse listening = {}", ks.status().mouse_listening);
}

#[cfg(not(windows))]
fn main() {}
