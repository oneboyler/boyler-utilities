//! "What is in front" for the key sounds (Windows only): the program's exe name (for the per-app rules) and whether a
//! full-screen app / game is in front (for "off while a game is in front"). No hook, no timer, no thread: it is read at the
//! moment of a press, and the answer is kept while the SAME window stays in front (re-read at most every 1.5 s, so a window
//! that goes full-screen later is noticed on a later press). Windows' own `SHQueryUserNotificationState` is the full-screen
//! test (the one Focus Assist's "when I'm playing a game" uses): D3D full screen, a full-screen "busy" app, presentation mode.

use std::time::{Duration, Instant};
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

const KEEP: Duration = Duration::from_millis(1500);

pub struct Front {
    hwnd: isize,
    exe: String,
    game: bool,
    at: Option<Instant>,
}

impl Default for Front {
    fn default() -> Self {
        Self::new()
    }
}

impl Front {
    pub const fn new() -> Front {
        Front { hwnd: 0, exe: String::new(), game: false, at: None }
    }

    /// (a full-screen app / game is in front, the front program's exe file name in lower case; "" when unknown).
    pub fn now(&mut self) -> (bool, &str) {
        let hwnd = unsafe { GetForegroundWindow() };
        let h = hwnd.0 as isize;
        let fresh = self.at.is_some_and(|t| t.elapsed() < KEEP);
        if !(fresh && h == self.hwnd) {
            if h != self.hwnd || self.at.is_none() {
                self.exe = exe_of(hwnd);
            }
            self.game = full_screen_in_front();
            self.hwnd = h;
            self.at = Some(Instant::now());
        }
        (self.game, &self.exe)
    }
}

pub(crate) fn full_screen_in_front() -> bool {
    match unsafe { SHQueryUserNotificationState() } {
        Ok(s) => s == QUNS_BUSY || s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE,
        Err(_) => false,
    }
}

fn exe_of(hwnd: HWND) -> String {
    if hwnd.0.is_null() {
        return String::new();
    }
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return String::new();
        }
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/']).next().unwrap_or("").to_ascii_lowercase()
    }
}
