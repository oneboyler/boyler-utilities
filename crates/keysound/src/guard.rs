//! "May input go out now?" (Windows only; Order 058): macros and "open" actions never send anything while a game / full-screen
//! window is in front (no input into games: his Vanguard) nor while the window in front is an administrator window (never type
//! into those; Windows would drop the input anyway). Two cheap queries made at the moment of use — no hook, no timer, no thread.

use crate::front::full_screen_in_front;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

/// Why input must NOT go to the window in front right now; None = fine.
pub fn input_blocked() -> Option<&'static str> {
    if full_screen_in_front() {
        return Some("a game or full-screen window is in front");
    }
    if admin_window_in_front() {
        return Some("an administrator window is in front");
    }
    None
}

/// Is a full-screen app / game in front? (The same Windows answer the key sounds use.)
pub fn game_in_front() -> bool {
    full_screen_in_front()
}

/// Is this process (or token) elevated? None = couldn't tell.
fn elevated(process: HANDLE) -> Option<bool> {
    // SAFETY: a plain token query on a process handle we hold; the token is closed again.
    unsafe {
        let mut tok = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut tok).ok()?;
        let mut e = TOKEN_ELEVATION::default();
        let mut ret = 0u32;
        let ok = GetTokenInformation(tok, TokenElevation, Some(&mut e as *mut TOKEN_ELEVATION as *mut std::ffi::c_void), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut ret).is_ok();
        let _ = CloseHandle(tok);
        ok.then_some(e.TokenIsElevated != 0)
    }
}

fn admin_window_in_front() -> bool {
    static ME: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    // SAFETY: the pseudo handle of this process.
    let me = *ME.get_or_init(|| elevated(unsafe { GetCurrentProcess() }).unwrap_or(false));
    if me {
        return false; // an elevated app can send to elevated windows
    }
    // SAFETY: plain window / process queries; the handle is closed again.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 || pid == std::process::id() {
            return false;
        }
        // a process we can't even look at (a protected / system one) counts as off limits
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return true };
        let r = elevated(h);
        let _ = CloseHandle(h);
        r.unwrap_or(true)
    }
}
