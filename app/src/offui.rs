//! Order 047 (the owner's test 3: "i clicked launch steam ... the entire bottom right of the screen gets a big black box, and
//! then steam opens and it unfreezes"): work that can wait on Windows - starting a program or opening a link through the
//! shell, an admin prompt, a COM / registry / WMI read, a device read - never runs on the menu's thread. The menu's
//! thread paints; while it waits on anything the window stops painting and Windows shows it black.
//!
//! `spawn` runs one such call on its own short-lived thread (COM ready in a single-threaded apartment, as the shell's
//! calls want) and wakes the menu when it ends. `shell_open` is the shell's "open" (a program, a folder, a link).
//!
//! The proof (`tests`): every call made through here returns to the menu's thread within one frame (16 ms) even when the
//! work itself takes seconds - a test copy can make every call take `set_test_delay` ms longer to show it.

use std::sync::atomic::{AtomicU64, Ordering};

thread_local! {
    /// test-only: every call `spawn`ed FROM THIS THREAD first sleeps this long (ms) - a stand-in for a slow shell / admin
    /// prompt / device (per thread: tests running side by side don't slow each other)
    static TEST_DELAY_MS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
/// how many `spawn`ed calls are running now (the quit waits for them, bounded: `wait_idle`)
static RUNNING: AtomicU64 = AtomicU64::new(0);
/// how many `spawn`ed calls have finished (tests wait on it)
static DONE: AtomicU64 = AtomicU64::new(0);

/// Run `f` off the menu's thread: its own thread, COM initialised (apartment-threaded), the menu woken when it ends.
/// Returns at once.
pub fn spawn(label: &'static str, f: impl FnOnce() + Send + 'static) {
    let d = TEST_DELAY_MS.with(|c| c.get());
    RUNNING.fetch_add(1, Ordering::AcqRel);
    let r = std::thread::Builder::new().name(format!("bu-offui-{label}")).spawn(move || {
        if d > 0 {
            std::thread::sleep(std::time::Duration::from_millis(d));
        }
        #[cfg(windows)]
        let com = unsafe {
            windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED | windows::Win32::System::Com::COINIT_DISABLE_OLE1DDE).is_ok()
        };
        f();
        #[cfg(windows)]
        if com {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
        DONE.fetch_add(1, Ordering::AcqRel);
        RUNNING.fetch_sub(1, Ordering::AcqRel);
        crate::services::Waker.wake();
    });
    if let Err(e) = r {
        RUNNING.fetch_sub(1, Ordering::AcqRel);
        crate::timing::note(&format!("offui spawn {label} failed: {e}"));
    }
}

/// The app is quitting: wait (at most `max_ms`) for the calls still running - their change-log notes must be in before
/// the store closes (Opus review: Audio's close and the take-over note from their threads).
pub fn wait_idle(max_ms: u64) {
    let t0 = std::time::Instant::now();
    while RUNNING.load(Ordering::Acquire) > 0 && t0.elapsed().as_millis() < max_ms as u128 {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Windows' shell "open" of a program, a folder or a link (`steam.exe`, `steam://...`, `https://...`), off the menu's
/// thread. Returns at once; the shell's answer is not waited for.
pub fn shell_open(target: &str) {
    let t = target.to_string();
    spawn("shell-open", move || {
        #[cfg(windows)]
        unsafe {
            use windows::core::HSTRING;
            use windows::Win32::UI::Shell::ShellExecuteW;
            use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
            let _ = ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(t.as_str()), None, None, SW_SHOWNORMAL);
        }
    });
}

/// test-only: make every `spawn`ed call take `ms` longer (0 = off).
#[cfg(test)]
pub fn set_test_delay(ms: u64) {
    TEST_DELAY_MS.with(|c| c.set(ms));
}

/// How many `spawn`ed calls have finished so far.
#[cfg(test)]
pub fn done() -> u64 {
    DONE.load(Ordering::Acquire)
}

/// The longest the menu's thread may be held by one call (one frame at 60 Hz; the order's limit).
#[cfg(test)]
pub const UI_BUDGET_MS: f64 = 16.0;

/// test-only: run `f` (a page's click, an action) and fail if it held the calling thread longer than `UI_BUDGET_MS`.
#[cfg(test)]
pub fn assert_quick<R>(what: &str, f: impl FnOnce() -> R) -> R {
    let t0 = std::time::Instant::now();
    let r = f();
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    assert!(ms <= UI_BUDGET_MS, "{what} held the menu's thread {ms:.1} ms (limit {UI_BUDGET_MS} ms) - its slow part must run off the menu's thread");
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A slow call (500 ms) made through `spawn` hands the thread back within one frame, and still runs.
    #[test]
    fn a_slow_call_never_holds_the_menu() {
        let ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let r2 = ran.clone();
        assert_quick("spawn of a 500 ms call", || {
            spawn("test", move || {
                std::thread::sleep(std::time::Duration::from_millis(500));
                r2.store(true, Ordering::Release);
            })
        });
        let t0 = std::time::Instant::now();
        while !ran.load(Ordering::Acquire) && t0.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(ran.load(Ordering::Acquire), "the call ran on its own thread");
    }

    /// The check itself fails a call that blocks (so a call left on the menu's thread is caught).
    #[test]
    #[should_panic(expected = "held the menu's thread")]
    fn the_check_catches_a_blocking_call() {
        assert_quick("a blocking call", || std::thread::sleep(std::time::Duration::from_millis(40)));
    }
}
