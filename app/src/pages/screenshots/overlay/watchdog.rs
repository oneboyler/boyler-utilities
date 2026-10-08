//! The capture overlay's watchdog (Order 048: a user's whole PC looked frozen). The overlay is one topmost full-screen window
//! per monitor that takes the mouse and the keys, owned by the app's UI thread. If that thread stops answering (a slow
//! Windows call, a bug), Windows keeps those frozen windows over everything and the whole desktop looks locked.
//!
//! While the overlay is open a small thread posts a heartbeat to it every [`TICK_MS`]; the UI thread answers it
//! ([`beat`]) when it handles the message. No answer for [`LIMIT_MS`] = the UI thread is stuck: the watchdog takes the
//! overlay windows off the screen itself (it never waits for the stuck thread), so the desktop is usable again at once.
//! When the UI thread answers again it sees [`take_fired`] and closes the capture (window.rs).
//! Nothing runs while the overlay is closed.
//!
//! HOW it hides (measured on a stuck test window, Order 048): `ShowWindowAsync` only POSTS the hide to the window's own
//! thread - nothing happens until that thread runs again. Cloaking the window (`DWMWA_CLOAK`) works at once (0.8 ms) from
//! any thread: the window is no longer drawn and clicks go through it to the apps below. So the watchdog cloaks, and also
//! queues the normal hide for when the thread comes back.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, ShowWindowAsync, SW_HIDE, WM_APP};

/// The heartbeat the watchdog posts to the overlay (its window procedure calls [`beat`]).
pub const WM_BEAT: u32 = WM_APP + 0x31;
/// How often the heartbeat goes out.
pub const TICK_MS: u64 = 25;
/// No answer for this long = stuck. With the tick the overlay is gone at most LIMIT + TICK (475 ms) after the UI thread's
/// last answer - within the half second the order asks.
pub const LIMIT_MS: u64 = 450;

/// When the UI thread last answered (ms since [`epoch`]).
static ANSWERED: AtomicU64 = AtomicU64::new(0);
/// The watchdog hid the overlay (the UI thread reads and clears it when it answers again).
static FIRED: AtomicBool = AtomicBool::new(false);
/// When the windows were hidden (ms since [`epoch`]).
static FIRED_AT: AtomicU64 = AtomicU64::new(0);

/// The running watchdog: (stop flag + its wake-up, the thread).
type Running = (Arc<(Mutex<bool>, Condvar)>, std::thread::JoinHandle<()>);
static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

fn epoch() -> Instant {
    static E: OnceLock<Instant> = OnceLock::new();
    *E.get_or_init(Instant::now)
}

/// ms since the watchdog's epoch (the same clock on every thread).
pub fn now_ms() -> u64 {
    epoch().elapsed().as_millis() as u64
}

/// The decision: hide when the UI thread's last answer is older than the limit (and not already hidden).
pub fn should_hide(now: u64, answered: u64, hidden: bool) -> bool {
    !hidden && now.saturating_sub(answered) > LIMIT_MS
}

/// The overlay opened with these windows: watch them until [`close`]. The heartbeat goes to the first one.
pub fn open(hwnds: &[HWND]) {
    close();
    let hs: Vec<isize> = hwnds.iter().map(|h| h.0 as isize).collect();
    if hs.is_empty() {
        return;
    }
    FIRED.store(false, Ordering::Release);
    ANSWERED.store(now_ms(), Ordering::Release);
    let stop = Arc::new((Mutex::new(false), Condvar::new()));
    let st = stop.clone();
    let spawned = std::thread::Builder::new().name("bu-overlay-watchdog".into()).spawn(move || run(hs, st));
    if let Ok(j) = spawned {
        *RUNNING.lock().unwrap_or_else(|e| e.into_inner()) = Some((stop, j));
    }
}

/// The overlay closed: the watchdog ends (waits for its thread - it never blocks for long).
pub fn close() {
    let r = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some((stop, j)) = r {
        *stop.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
        stop.1.notify_all();
        let _ = j.join();
    }
}

/// The UI thread handled a heartbeat.
pub fn beat() {
    ANSWERED.store(now_ms(), Ordering::Release);
}

/// The watchdog hid the overlay since the last call: when (ms, [`now_ms`]'s clock) - read once.
pub fn take_fired() -> Option<u64> {
    FIRED.swap(false, Ordering::AcqRel).then(|| FIRED_AT.load(Ordering::Acquire))
}

/// Gone from the screen at once, whatever its thread is doing: cloaked (not drawn, clicks go through), and the normal hide
/// queued for when its thread runs again.
pub fn hide_now(h: HWND) {
    let on = BOOL(1);
    unsafe {
        let _ = DwmSetWindowAttribute(h, DWMWA_CLOAK, &on as *const _ as *const _, std::mem::size_of::<BOOL>() as u32);
        let _ = ShowWindowAsync(h, SW_HIDE);
    }
}

fn run(hwnds: Vec<isize>, stop: Arc<(Mutex<bool>, Condvar)>) {
    let mut hidden = false;
    loop {
        {
            let g = stop.0.lock().unwrap_or_else(|e| e.into_inner());
            let (g, _) = stop.1.wait_timeout_while(g, Duration::from_millis(TICK_MS), |s| !*s).unwrap_or_else(|e| e.into_inner());
            if *g {
                return;
            }
        }
        let now = now_ms();
        if should_hide(now, ANSWERED.load(Ordering::Acquire), hidden) {
            for h in &hwnds {
                hide_now(HWND(*h as *mut _));
            }
            hidden = true;
            FIRED_AT.store(now, Ordering::Release);
            FIRED.store(true, Ordering::Release);
        }
        if !hidden {
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnds[0] as *mut _)), WM_BEAT, WPARAM(0), LPARAM(0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_only_after_the_limit_and_once() {
        assert!(!should_hide(1000, 1000, false));
        assert!(!should_hide(1000 + LIMIT_MS, 1000, false), "exactly the limit: still waiting");
        assert!(should_hide(1001 + LIMIT_MS, 1000, false));
        assert!(!should_hide(5000, 1000, true), "already hidden: nothing more");
        const { assert!(LIMIT_MS + TICK_MS < 500, "gone within half a second of the last answer") };
    }

    /// The real thing on an OFF-SCREEN window (x = -20000, never on the monitors): its thread stops answering on purpose;
    /// the watchdog must take the window off the screen (cloaked, clicks go through) within 0.5 s, while that thread is
    /// still stuck. (Order 048 item 6.)
    #[test]
    fn a_stuck_ui_thread_loses_its_overlay_within_half_a_second() {
        use windows::core::w;
        use windows::Win32::Foundation::{LRESULT, POINT};
        use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::*;

        extern "system" fn proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
            if m == WM_BEAT {
                beat();
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(h, m, w, l) }
        }
        const AT: i32 = -20000;
        let (tx, rx) = std::sync::mpsc::channel::<(isize, u64)>();
        let t = std::thread::spawn(move || unsafe {
            let inst = GetModuleHandleW(None).unwrap();
            let class = w!("BoylerUtilities.WatchdogTest");
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(proc), hInstance: inst.into(), lpszClassName: class, ..Default::default() });
            let h = CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, class, w!(""), WS_POPUP, AT, AT, 64, 64, None, None, Some(inst.into()), None).unwrap();
            let _ = ShowWindow(h, SW_SHOWNA);
            open(&[h]);
            // answer heartbeats for 300 ms (a healthy UI thread)
            let until = now_ms() + 300;
            let mut msg = MSG::default();
            while now_ms() < until {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    DispatchMessageW(&msg);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(take_fired().is_none(), "a healthy thread keeps its overlay");
            // now stuck for 1.5 s (no messages handled); the test thread watches the window meanwhile
            tx.send((h.0 as isize, now_ms())).unwrap();
            std::thread::sleep(Duration::from_millis(1500));
            close();
            let _ = DestroyWindow(h);
        });
        let (h, stuck) = rx.recv().unwrap();
        let h = HWND(h as *mut _);
        let cloaked = || {
            let mut v = 0u32;
            let _ = unsafe { DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut v as *mut _ as *mut _, 4) };
            v != 0
        };
        let hit = || unsafe { WindowFromPoint(POINT { x: AT + 32, y: AT + 32 }) } == h;
        let hit_before = hit();
        let mut gone = None;
        while now_ms() < stuck + 1400 {
            if cloaked() {
                gone = Some(now_ms() - stuck);
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let clicks_through = !hit();
        t.join().unwrap();
        assert!(hit_before, "the window took the clicks at its place before");
        let gone = gone.expect("the overlay stayed while its thread was stuck");
        println!("watchdog: overlay off the screen {gone} ms after the UI thread got stuck");
        assert!(gone <= 500, "hidden after {gone} ms");
        assert!(clicks_through, "a click there reaches what is below (not the stuck window)");
        assert!(take_fired().is_some());
    }
}
