//! Order 072: the open menu stays in front of normal windows, but steps back for a fullscreen window (a game).
//! `is_fullscreen` says whether a window is one; `Hook` is the event-driven "the front window changed" notice
//! (`SetWinEventHook(EVENT_SYSTEM_FOREGROUND)`, out of context: an accessibility event - nothing injected, no input hook).
//! It exists only while the menu is open and costs nothing between two changes of the front window.

use std::cell::Cell;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONULL};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::*;

/// A window is "fullscreen" when it covers its whole monitor (taskbar included) and has no title bar (a maximized
/// window keeps its caption and sits inside the work area or just over its edges). The desktop is not one.
pub fn covers_monitor(win: RECT, mon: RECT, has_caption: bool) -> bool {
    !has_caption && win.left <= mon.left && win.top <= mon.top && win.right >= mon.right && win.bottom >= mon.bottom
}

/// Order 079: front windows that say nothing about a game: Windows' full-monitor hosts that are mostly see-through or
/// hidden (Alt+Tab / Task View, the emoji panel / Win+V / touch keyboard "Windows Input Experience", Start, Search:
/// CoreWindows of the shell - a Store app's own window is an ApplicationFrameWindow), a hidden (cloaked) window and a
/// click-through overlay. Counted as fullscreen they made the menu step back (and the next click on a normal window
/// covered it until it was on top again: a blink); counted as normal windows they would put the menu over a game still
/// on the screen (Alt+Tab over it). So the stay-in-front leaves the z-order as it is while one of them is in front.
pub fn ignored(class: &str, cloaked: bool, click_through: bool) -> bool {
    cloaked || click_through || matches!(class, "XamlExplorerHostIslandWindow" | "MultitaskingViewFrame" | "ForegroundStaging" | "Windows.UI.Core.CoreWindow")
}

/// What the front window means for the stay-in-front (Order 072 / 079).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Front {
    /// a fullscreen window of another app (a game): the menu steps back
    Full,
    /// any other window (the desktop too): the menu is on top
    Normal,
    /// `ignored`: nothing changes
    Ignored,
}

fn class_of(hwnd: HWND) -> String {
    let mut class = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, &mut class) } as usize;
    String::from_utf16_lossy(&class[..n.min(64)])
}

/// The front window `hwnd`, classed.
pub fn classify(hwnd: HWND) -> Front {
    if hwnd.is_invalid() {
        return Front::Normal;
    }
    let class = class_of(hwnd);
    let mut cloaked = 0u32;
    let ex = unsafe {
        let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4);
        GetWindowLongW(hwnd, GWL_EXSTYLE) as u32
    };
    if ignored(&class, cloaked != 0, ex & WS_EX_TRANSPARENT.0 != 0 && ex & WS_EX_LAYERED.0 != 0) {
        return Front::Ignored;
    }
    if is_fullscreen(hwnd) {
        Front::Full
    } else {
        Front::Normal
    }
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
        if matches!(class_of(hwnd).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
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

    #[test]
    fn shell_hosts_hidden_and_click_through_windows_are_ignored_at_the_front() {
        // Order 079: "Windows Input Experience" (TextInputHost, 1920x1080, no caption) on the owner's PC; Alt+Tab / Task View
        for c in ["Windows.UI.Core.CoreWindow", "XamlExplorerHostIslandWindow", "MultitaskingViewFrame", "ForegroundStaging"] {
            assert!(ignored(c, false, false), "{c}");
        }
        assert!(ignored("UnrealWindow", true, false));
        assert!(ignored("NVOverlay", false, true));
        // a game / a browser in F11 / a video player stay what covers_monitor says
        // (the desktop is a normal front window: a click on it means the game left the front)
        for c in ["UnrealWindow", "Chrome_WidgetWin_1", "UnityWndClass", "ApplicationFrameWindow", "Progman", "WorkerW"] {
            assert!(!ignored(c, false, false), "{c}");
        }
    }
}
