//! Order 072: the open menu stays in front of normal windows, but steps back for a fullscreen window (a game).
//! `is_fullscreen` says whether a window is one; `Hook` is the event-driven "the front window changed" notice
//! (`SetWinEventHook(EVENT_SYSTEM_FOREGROUND)`, out of context: an accessibility event - nothing injected, no input hook).
//! It exists only while the menu is open and costs nothing between two changes of the front window.

use std::cell::Cell;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONULL};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::*;

/// A window is "fullscreen" when it covers its whole monitor (taskbar included) and has no title bar (a maximized
/// window keeps its caption and sits inside the work area or just over its edges). The desktop is not one.
pub fn covers_monitor(win: RECT, mon: RECT, has_caption: bool) -> bool {
    !has_caption && win.left <= mon.left && win.top <= mon.top && win.right >= mon.right && win.bottom >= mon.bottom
}

/// Whether `hwnd` is a visible, not minimized, fullscreen window of some other app than the desktop / shell.
pub fn is_fullscreen(hwnd: HWND) -> bool {
    if hwnd.is_invalid() {
        return false;
    }
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() || hwnd == GetShellWindow() || hwnd == GetDesktopWindow() {
            return false;
        }
        // the desktop's own windows (wallpaper host) cover the monitor too
        let mut class = [0u16; 32];
        let n = GetClassNameW(hwnd, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..n.min(32)]);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
            return false;
        }
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
        if mon.is_invalid() {
            return false;
        }
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let mut r = RECT::default();
        if !GetMonitorInfoW(mon, &mut mi).as_bool() || GetWindowRect(hwnd, &mut r).is_err() {
            return false;
        }
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        covers_monitor(r, mi.rcMonitor, style & WS_CAPTION.0 == WS_CAPTION.0)
    }
}

thread_local! {
    static NOTIFY: Cell<Option<fn()>> = const { Cell::new(None) };
}

unsafe extern "system" fn on_foreground(_h: HWINEVENTHOOK, _ev: u32, _hwnd: HWND, id_object: i32, _child: i32, _thread: u32, _time: u32) {
    if id_object == OBJID_WINDOW.0 {
        if let Some(f) = NOTIFY.with(|n| n.get()) {
            f();
        }
    }
}

/// The installed notice; dropping it removes the hook. Must live on (and is called back on) the thread with the message loop.
pub struct Hook(HWINEVENTHOOK);

impl Hook {
    /// `notify` runs on this thread whenever another app's window comes to the front (the app's own windows are skipped).
    pub fn install(notify: fn()) -> Option<Hook> {
        NOTIFY.with(|n| n.set(Some(notify)));
        let h = unsafe { SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS) };
        (!h.is_invalid()).then_some(Hook(h))
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWinEvent(self.0);
        }
        NOTIFY.with(|n| n.set(None));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(l: i32, t: i32, rr: i32, b: i32) -> RECT {
        RECT { left: l, top: t, right: rr, bottom: b }
    }

    #[test]
    fn borderless_game_covering_the_monitor_is_fullscreen() {
        assert!(covers_monitor(r(0, 0, 1920, 1080), r(0, 0, 1920, 1080), false));
        assert!(covers_monitor(r(-1920, 0, 0, 1080), r(-1920, 0, 0, 1080), false));
    }

    #[test]
    fn maximized_window_with_a_title_bar_is_not() {
        assert!(!covers_monitor(r(-8, -8, 1928, 1088), r(0, 0, 1920, 1080), true));
    }

    #[test]
    fn window_inside_the_monitor_or_over_the_work_area_only_is_not() {
        assert!(!covers_monitor(r(0, 0, 1920, 1040), r(0, 0, 1920, 1080), false));
        assert!(!covers_monitor(r(100, 100, 900, 700), r(0, 0, 1920, 1080), false));
    }
}
