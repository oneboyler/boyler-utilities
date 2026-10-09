//! Order 062's proof on the real PC: the REAL engine and the REAL output stream, MUTED (the stream's own audio session is
//! muted, nothing is heard), measured from this process: CPU while it plays, memory while it plays and after it stops.
//!
//! `cargo run -p bu-noise --example noise-proof [-- <seconds measured, default 10>]`
//!
//! CPU is read as CPU CYCLES of this process (exact, not the 15 ms tick sampling) and turned into "% of one core" with the
//! clock speed measured here by a one-second spin; "% of all cores" is Task Manager's number.

#[cfg(windows)]
fn main() {
    use bu_noise::{Kind, Noise};
    use std::time::{Duration, Instant};
    use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::GetCurrentProcess;
    use windows::Win32::System::WindowsProgramming::QueryProcessCycleTime;

    fn cycles() -> u64 {
        let mut c = 0u64;
        // SAFETY: plain query of our own process.
        unsafe {
            let _ = QueryProcessCycleTime(GetCurrentProcess(), &mut c);
        }
        c
    }
    fn private_mb() -> f64 {
        let mut m = PROCESS_MEMORY_COUNTERS_EX::default();
        // SAFETY: plain query of our own process into a correctly sized struct.
        unsafe {
            let _ = K32GetProcessMemoryInfo(GetCurrentProcess(), &mut m as *mut _ as *mut _, std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32);
        }
        m.PrivateUsage as f64 / 1048576.0
    }

    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;

    // the clock speed: cycles per second of one core running flat out
    let (c0, t0) = (cycles(), Instant::now());
    let mut x = 1u64;
    while t0.elapsed() < Duration::from_millis(1000) {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    }
    let hz = (cycles() - c0) as f64 / t0.elapsed().as_secs_f64();
    println!("clock (measured by a 1 s spin, x={}): {:.2} GHz, {} logical cores", x & 1, hz / 1e9, cores);

    let n = Noise::new();
    n.set_test_mute(true);
    let before = private_mb();
    println!("memory before play: {before:.1} MB private");

    // the loop, made on the worker thread: its cost
    let (g0, gt) = (cycles(), Instant::now());
    n.play(Kind::Brown, 10, None);
    let mut s = n.status();
    while s.bytes == 0 && gt.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(10));
        s = n.status();
    }
    println!("first loop ready after {:.0} ms ({} cycles = {:.0} ms of one core), stream open at {} Hz, error: {:?}", gt.elapsed().as_secs_f64() * 1000.0, cycles() - g0, (cycles() - g0) as f64 / hz * 1000.0, s.rate, s.error);
    if s.bytes == 0 {
        println!("NO SOUND DEVICE / stream did not open: {:?}", s.error);
        return;
    }
    // past the one-second fade-in
    std::thread::sleep(Duration::from_millis(3000));
    let during = private_mb();
    let w1 = n.status().written;
    let (c1, t1) = (cycles(), Instant::now());
    std::thread::sleep(Duration::from_secs(secs));
    let used = cycles() - c1;
    let wall = t1.elapsed().as_secs_f64();
    let played = (n.status().written - w1) as f64 / f64::from(s.rate);
    println!("audio fed to the stream in that time: {played:.2} s of sound in {wall:.2} s of wall time (it must be about equal: the device plays it in real time)");
    let one_core = used as f64 / (hz * wall) * 100.0;
    println!("CPU while playing (muted, {wall:.1} s, brown, 10 %): {:.3} % of one core = {:.4} % of all cores (measured, exact cycles)", one_core, one_core / cores);
    println!("memory while playing: {during:.1} MB private (loop {:.1} MB)", s.bytes as f64 / 1048576.0);

    // another noise while it plays: the cost of making it, then it settles
    for k in [Kind::Pink, Kind::Grey] {
        let (c, t) = (cycles(), Instant::now());
        n.set_kind(k);
        while n.status().bytes == 0 || n.status().kind != Some(k) {
            std::thread::sleep(Duration::from_millis(10));
            if t.elapsed() > Duration::from_secs(5) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(1500));
        println!("switched to {}: {} cycles = {:.0} ms of one core in 1.5 s+ (making the new loop, once)", k.name(), cycles() - c, (cycles() - c) as f64 / hz * 1000.0);
    }

    // stop: fades out, everything closes
    let ts = Instant::now();
    n.stop();
    while n.status().playing && ts.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    println!("stopped after {:.2} s (fade 1 s + the buffer's tail); playing={}", ts.elapsed().as_secs_f64(), n.status().playing);
    std::thread::sleep(Duration::from_millis(500));
    let (c2, t2) = (cycles(), Instant::now());
    std::thread::sleep(Duration::from_secs(3));
    let idle = (cycles() - c2) as f64 / (hz * t2.elapsed().as_secs_f64()) * 100.0;
    println!("CPU after stop (3 s): {idle:.4} % of one core; memory after stop: {:.1} MB private (before play {before:.1} MB)", private_mb());
}

#[cfg(not(windows))]
fn main() {}
