//! Order 049: one vblank-paced animator for the small on-screen windows (the timers' pills, the mic icon) instead of a
//! Windows timer each (`SetTimer` 16 ms = 64 Hz at most for the mic icon, 100 ms for the pills - both below a 144-360 Hz
//! screen, and both woke the app on every tick whether anything changed or not).
//!
//! A client asks for a frame at a time: now (it animates) or later (the next moment it looks different, e.g. a timer's next
//! second). A helper thread sleeps until the earliest asked time, then waits for the compositor's clock
//! (`DCompositionWaitForCompositorClock`, Windows 11: it ticks at the fastest screen's refresh) - before Windows 11 the
//! main screen's vblank (`IDXGIOutput::WaitForVBlank`) - and posts ONE message to a message-only window of the UI thread
//! (never a second while one waits). The UI thread runs the clients whose time has come; each asks again if it wants more.
//! Nothing asked = the helper waits on an event: no wake-ups at all.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject, INFINITE};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Who asks for frames (one slot each).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Client {
    Timers = 0,
    Mic = 1,
}
const N: usize = 2;
const WM_FRAME: u32 = WM_APP + 0x49;

thread_local! {
    /// what each client runs and from when (timing::now ms)
    static SLOTS: RefCell<[Option<(fn(), f64)>; N]> = const { RefCell::new([None; N]) };
    static HWND_: Cell<isize> = const { Cell::new(0) };
}

/// The earliest asked time over all clients (f64 bits; infinity = nothing asked).
static EARLIEST: AtomicU64 = AtomicU64::new(0x7ff0_0000_0000_0000);
/// A frame message is on its way to the UI thread.
static PENDING: AtomicBool = AtomicBool::new(false);
/// The helper thread's wake-up event and the UI thread's window (raw values: both live as long as the app).
static SHARED: OnceLock<(isize, isize)> = OnceLock::new();

/// Ask for `f` to run on the first screen refresh at or after `due` (ms, `timing::now`). A client has one slot: a new ask
/// replaces its old one. Off in test copies and unit tests (nothing goes on the screen there).
pub fn at(c: Client, due: f64, f: fn()) {
    if cfg!(test) || crate::testmode::on() {
        return;
    }
    ask(c, due, f);
}

fn ask(c: Client, due: f64, f: fn()) {
    if !ensure() {
        return;
    }
    SLOTS.with(|s| s.borrow_mut()[c as usize] = Some((f, due)));
    publish();
}

/// `at` now: the next refresh.
pub fn next(c: Client, f: fn()) {
    at(c, crate::timing::now(), f);
}

/// The client wants no more frames.
pub fn cancel(c: Client) {
    let had = SLOTS.with(|s| s.borrow_mut()[c as usize].take().is_some());
    if had {
        publish();
    }
}

/// Is a frame asked for (tests / proofs).
pub fn asked(c: Client) -> bool {
    SLOTS.with(|s| s.borrow()[c as usize].is_some())
}

fn publish() {
    let e = SLOTS.with(|s| s.borrow().iter().flatten().map(|x| x.1).fold(f64::INFINITY, f64::min));
    EARLIEST.store(e.to_bits(), Ordering::SeqCst);
    if let Some((ev, _)) = SHARED.get() {
        unsafe {
            let _ = SetEvent(HANDLE(*ev as *mut _));
        }
    }
}

/// The UI thread's frame: run every client whose time has come.
fn run_due() {
    let t = crate::timing::now();
    let due: Vec<fn()> = SLOTS.with(|s| {
        let mut s = s.borrow_mut();
        let mut v = Vec::new();
        for slot in s.iter_mut() {
            if let Some((f, d)) = *slot {
                if d <= t + 0.5 {
                    v.push(f);
                    *slot = None;
                }
            }
        }
        v
    });
    for f in due {
        f();
    }
    publish();
}

extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_FRAME {
        PENDING.store(false, Ordering::SeqCst);
        run_due();
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(h, msg, wp, lp) }
}

/// The message-only window (UI thread) and the helper thread, made on the first ask.
fn ensure() -> bool {
    if HWND_.with(|h| h.get()) != 0 {
        return true;
    }
    if SHARED.get().is_some() {
        // made by another thread: only the UI thread animates
        return false;
    }
    unsafe {
        let Ok(inst) = GetModuleHandleW(None) else { return false };
        let class = w!("BoylerUtilities.VSync");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: class, ..Default::default() });
        let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(inst.into()), None) else { return false };
        let Ok(ev) = CreateEventW(None, false, false, windows::core::PCWSTR::null()) else { return false };
        if SHARED.set((ev.0 as isize, hwnd.0 as isize)).is_err() {
            return false;
        }
        HWND_.with(|h| h.set(hwnd.0 as isize));
        let (e, h) = (ev.0 as isize, hwnd.0 as isize);
        let _ = std::thread::Builder::new().name("vsync".into()).spawn(move || helper(e, h));
    }
    true
}

type WaitClock = unsafe extern "system" fn(u32, *const HANDLE, u32) -> u32;

/// `DCompositionWaitForCompositorClock` (dcomp.dll, Windows 11), if this Windows has it.
fn compositor_clock() -> Option<WaitClock> {
    unsafe {
        // only Windows' own copy (System32), never one next to the app
        let lib = LoadLibraryExW(w!("dcomp.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
        let f = GetProcAddress(lib, windows::core::s!("DCompositionWaitForCompositorClock"))?;
        Some(std::mem::transmute::<unsafe extern "system" fn() -> isize, WaitClock>(f))
    }
}

/// The main screen's output, for `WaitForVBlank` (before Windows 11).
fn main_output() -> Option<windows::Win32::Graphics::Dxgi::IDXGIOutput> {
    use windows::Win32::Graphics::Dxgi::*;
    unsafe {
        let f: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let a = f.EnumAdapters1(0).ok()?;
        a.EnumOutputs(0).ok()
    }
}

/// The helper thread: sleep until the earliest asked time, wait for the next refresh, post one frame.
fn helper(ev: isize, hwnd: isize) {
    let ev = HANDLE(ev as *mut _);
    let hwnd = HWND(hwnd as *mut _);
    let clock = compositor_clock();
    let output = if clock.is_none() { main_output() } else { None };
    loop {
        let e = f64::from_bits(EARLIEST.load(Ordering::SeqCst));
        if e.is_infinite() {
            unsafe {
                WaitForSingleObject(ev, INFINITE);
            }
            continue;
        }
        // a frame still waits for the UI thread: sleep until it ran (its `publish` sets the event), no empty ticks
        if PENDING.load(Ordering::SeqCst) {
            unsafe {
                WaitForSingleObject(ev, 100);
            }
            continue;
        }
        let left = e - crate::timing::now();
        // far off: sleep (an ask meanwhile wakes us); the last 20 ms are waited out tick by tick (Windows' sleep is
        // only as fine as its 15.6 ms timer)
        if left > 20.0 {
            unsafe {
                WaitForSingleObject(ev, (left - 16.0) as u32);
            }
            continue;
        }
        let t0 = crate::timing::now();
        match (clock, &output) {
            (Some(wait), _) => unsafe {
                // the event too: a new ask is looked at at once
                let r = wait(1, &ev, 100);
                if r == WAIT_OBJECT_0.0 {
                    continue;
                }
            },
            (None, Some(o)) => unsafe {
                let _ = o.WaitForVBlank();
            },
            (None, None) => unsafe {
                let _ = windows::Win32::Graphics::Dwm::DwmFlush();
            },
        }
        // a wait that returned at once (no screen, a locked desktop) never spins: at most ~1000 posts a second
        if crate::timing::now() - t0 < 0.5 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if f64::from_bits(EARLIEST.load(Ordering::SeqCst)) <= crate::timing::now() + 0.5 && !PENDING.swap(true, Ordering::SeqCst) {
            unsafe {
                if PostMessageW(Some(hwnd), WM_FRAME, WPARAM(0), LPARAM(0)).is_err() {
                    PENDING.store(false, Ordering::SeqCst);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Order 049 proof (run by hand: `cargo test -p bu-app vsync -- --ignored --nocapture`): the compositor clock ticks
    /// at the screen's refresh. Nothing is opened or shown; it only waits.
    #[test]
    #[ignore]
    fn compositor_clock_ticks_at_the_screen_rate() {
        let Some(wait) = compositor_clock() else {
            println!("no DCompositionWaitForCompositorClock (before Windows 11)");
            return;
        };
        let t0 = crate::timing::now();
        let mut n = 0;
        while crate::timing::now() - t0 < 2000.0 {
            unsafe {
                wait(0, std::ptr::null(), 100);
            }
            n += 1;
        }
        println!("compositor clock: {} ticks in 2 s = {:.1} Hz", n, n as f64 / 2.0);
        assert!(n > 100, "it ticks");
    }

    thread_local!(static FRAMES: Cell<u32> = const { Cell::new(0) });

    fn again() {
        FRAMES.with(|f| f.set(f.get() + 1));
        ask(Client::Timers, crate::timing::now(), again);
    }

    fn once_later() {
        FRAMES.with(|f| f.set(f.get() + 1000));
    }

    /// The whole animator (helper thread + message-only window + one message per refresh), driven for 1 s by a client that
    /// asks every frame, then by one that asks once 300 ms later - and nothing at all after it. Run by hand (as above).
    #[test]
    #[ignore]
    fn the_animator_runs_one_frame_per_refresh_and_sleeps_when_nothing_is_asked() {
        let pump = |ms: f64| {
            let t0 = crate::timing::now();
            while crate::timing::now() - t0 < ms {
                unsafe {
                    let mut m = MSG::default();
                    while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                        DispatchMessageW(&m);
                    }
                    let _ = MsgWaitForMultipleObjects(None, false, 5, QS_ALLINPUT);
                }
            }
        };
        ask(Client::Timers, crate::timing::now(), again);
        pump(1000.0);
        let n = FRAMES.with(|f| f.replace(0));
        cancel(Client::Timers);
        pump(50.0);
        FRAMES.with(|f| f.set(0));
        println!("animator: {n} frames in 1 s");
        let t = crate::timing::now();
        ask(Client::Mic, t + 300.0, once_later);
        pump(250.0);
        assert_eq!(FRAMES.with(|f| f.get()), 0, "not before its time");
        pump(150.0);
        assert_eq!(FRAMES.with(|f| f.get()), 1000, "once, at its time");
        pump(300.0);
        assert_eq!(FRAMES.with(|f| f.get()), 1000, "nothing asked: no frames");
        assert!(n > 50, "{n}");
    }
}
