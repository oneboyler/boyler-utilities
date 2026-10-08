//! QueryPerformanceCounter time and the optional timing log (`--log <file>`) the measuring script reads:
//! when an open request arrives, when its first frame is presented, every presented frame, every page switch.

use std::cell::RefCell;
use std::io::Write;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

thread_local! {
    static LOG: RefCell<Option<std::io::BufWriter<std::fs::File>>> = const { RefCell::new(None) };
    static FREQ: f64 = {
        let mut f = 0i64;
        unsafe { let _ = QueryPerformanceFrequency(&mut f); }
        f as f64
    };
    static FRAMES_ON: RefCell<bool> = const { RefCell::new(false) };
    static FRAMES_ALWAYS: RefCell<bool> = const { RefCell::new(false) };
}

/// Milliseconds from the performance counter.
pub fn now() -> f64 {
    let mut c = 0i64;
    unsafe {
        let _ = QueryPerformanceCounter(&mut c);
    }
    FREQ.with(|f| c as f64 * 1000.0 / *f)
}

pub fn open_log(path: &str) {
    if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        LOG.with(|l| *l.borrow_mut() = Some(std::io::BufWriter::new(f)));
    }
}

fn line(s: String) {
    LOG.with(|l| {
        if let Some(w) = l.borrow_mut().as_mut() {
            let _ = writeln!(w, "{}", s);
        }
    });
}

pub fn flush() {
    LOG.with(|l| {
        if let Some(w) = l.borrow_mut().as_mut() {
            let _ = w.flush();
        }
    });
}

pub fn open_request(t: f64) {
    line(format!("open_req {:.3}", t));
}
pub fn first_present(t: f64) {
    line(format!("first_present {:.3} done {:.3}", t, now()));
    flush();
}
pub fn frames_always() {
    FRAMES_ALWAYS.with(|f| *f.borrow_mut() = true);
}
fn frames_logged() -> bool {
    FRAMES_ON.with(|f| *f.borrow()) || FRAMES_ALWAYS.with(|f| *f.borrow())
}
/// A note only while frames are logged.
pub fn note_if_frames(s: &str) {
    if frames_logged() {
        line(format!("note {:.3} {}", now(), s));
    }
}
pub fn frame(t: f64) {
    if frames_logged() {
        line(format!("frame {:.3} done {:.3}", t, now()));
    }
}

/// Order 051: one capture-overlay window's frame (test copies, BU_PROF): when, its UI-thread CPU, which window, GPU or CPU path.
pub fn ov_frame(t: f64, cpu: f64, win: usize, gpu: bool) {
    line(format!("ovframe {:.3} done {:.3} cpu {:.3} win {} gpu {}", t, now(), cpu, win, gpu as u8));
}

/// A frame with the CPU time this thread spent on it (ms, `thread_cpu_ms`): what the frame costs, whatever else the PC
/// is doing (Order 041: other jobs' builds made the clock time useless).
pub fn frame_cpu(t: f64, cpu: f64) {
    if frames_logged() {
        line(format!("frame {:.3} done {:.3} cpu {:.3}", t, now(), cpu));
    }
}

/// This thread's CPU time (ms) from its cycle count (QueryThreadCycleTime: exact, not the 15.6 ms scheduler tick of
/// GetThreadTimes), converted with the time-stamp counter's rate measured once against the performance counter.
pub fn thread_cpu_ms() -> f64 {
    // (kernel32's own declaration: no extra `windows` feature for one call)
    #[link(name = "kernel32")]
    extern "system" {
        fn QueryThreadCycleTime(thread: *mut core::ffi::c_void, cycles: *mut u64) -> i32;
    }
    let mut c = 0u64;
    unsafe {
        let _ = QueryThreadCycleTime(windows::Win32::System::Threading::GetCurrentThread().0, &mut c);
    }
    c as f64 / tsc_per_ms()
}

#[cfg(target_arch = "x86_64")]
fn tsc_per_ms() -> f64 {
    static R: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *R.get_or_init(|| {
        let (q0, c0) = (now(), unsafe { core::arch::x86_64::_rdtsc() });
        std::thread::sleep(std::time::Duration::from_millis(60));
        let (q1, c1) = (now(), unsafe { core::arch::x86_64::_rdtsc() });
        (c1 - c0) as f64 / (q1 - q0)
    })
}
#[cfg(not(target_arch = "x86_64"))]
fn tsc_per_ms() -> f64 {
    1e6
}
pub fn mark(name: &str, t: f64) {
    line(format!("{} {:.3}", name, t));
    if name == "switch" {
        FRAMES_ON.with(|f| *f.borrow_mut() = true);
    }
}
pub fn frames_off() {
    FRAMES_ON.with(|f| *f.borrow_mut() = false);
    flush();
}
/// A free-form line (window rectangles, glass set-up results) — the proof the test windows stayed off-screen.
pub fn note(s: &str) {
    line(format!("note {:.3} {}", now(), s));
    flush();
}
