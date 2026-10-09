//! Order 080's proof on the real PC: your own mix on the REAL engine and the REAL output stream, MUTED (the stream's own audio
//! session is muted, nothing is heard), measured from this process: CPU while a mix plays, CPU while a slider is dragged (the
//! new loop is made again and again), memory at each step and after the stop.
//!
//! `cargo run --release -p bu-noise --example noise-proof-mix`

#[cfg(windows)]
fn main() {
    use bu_noise::{Mix, Noise};
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

    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;
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

    // play a mix with waves
    let m = Mix::new(35, 30, 60);
    let (g0, gt) = (cycles(), Instant::now());
    n.play(m, 10, None);
    let mut s = n.status();
    while s.bytes == 0 && gt.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(10));
        s = n.status();
    }
    if s.bytes == 0 {
        println!("NO SOUND DEVICE / stream did not open: {:?}", s.error);
        return;
    }
    println!("first loop ready after {:.0} ms ({:.0} ms of one core), stream at {} Hz", gt.elapsed().as_secs_f64() * 1000.0, (cycles() - g0) as f64 / hz * 1000.0, s.rate);
    std::thread::sleep(Duration::from_millis(3000));
    let playing_mb = private_mb();

    // steady: the mix just plays
    let (c1, t1) = (cycles(), Instant::now());
    std::thread::sleep(Duration::from_secs(10));
    let one = (cycles() - c1) as f64 / (hz * t1.elapsed().as_secs_f64()) * 100.0;
    println!("CPU while a mix plays (muted, 10 s): {:.3} % of one core = {:.4} % of all cores (measured, exact cycles); memory {playing_mb:.1} MB private", one, one / cores);

    // a slider dragged: 30 moves a second for 4 s (each a different mix)
    let (c2, t2) = (cycles(), Instant::now());
    let mut peak_mb = 0.0f64;
    let mut i = 0u32;
    let w0 = n.status().written;
    while t2.elapsed() < Duration::from_secs(4) {
        let tone = ((i * 3) % 101) as u8;
        n.set_sound(Mix::new(tone, 30, 60));
        i += 1;
        peak_mb = peak_mb.max(private_mb());
        std::thread::sleep(Duration::from_millis(33));
    }
    let drag_wall = t2.elapsed().as_secs_f64();
    let drag = (cycles() - c2) as f64 / (hz * drag_wall) * 100.0;
    let fed = (n.status().written - w0) as f64 / f64::from(s.rate);
    println!("CPU while a slider is dragged ({i} moves in {drag_wall:.1} s): {drag:.1} % of one core = {:.2} % of all cores (measured); peak memory {peak_mb:.1} MB private", drag / cores);
    println!("audio fed during the drag: {fed:.2} s of sound in {drag_wall:.2} s (must be about equal: no gap while the loops are made)");

    // it settles
    std::thread::sleep(Duration::from_millis(2500));
    let (c3, t3) = (cycles(), Instant::now());
    std::thread::sleep(Duration::from_secs(5));
    let after = (cycles() - c3) as f64 / (hz * t3.elapsed().as_secs_f64()) * 100.0;
    println!("CPU 2.5 s after the last move (5 s): {after:.3} % of one core; memory {:.1} MB private; sound now {:?}", private_mb(), n.status().sound);

    let ts = Instant::now();
    n.stop();
    while n.status().playing && ts.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(500));
    let (c4, t4) = (cycles(), Instant::now());
    std::thread::sleep(Duration::from_secs(3));
    let idle = (cycles() - c4) as f64 / (hz * t4.elapsed().as_secs_f64()) * 100.0;
    println!("stopped; CPU after stop (3 s): {idle:.4} % of one core; memory after stop {:.1} MB private (before play {before:.1} MB); playing={}", private_mb(), n.status().playing);
}

#[cfg(not(windows))]
fn main() {}
