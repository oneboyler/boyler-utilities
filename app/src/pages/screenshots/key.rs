//! The Screenshot key's action (Order 019): what the app's keys manager calls when the Screenshot key is pressed (the
//! keys manager is Order 014 item 2 - until it is merged nothing calls this, see the report). It opens the capture
//! overlay (`overlay::window::start`, refused in a test copy) and connects it to the rest of the app:
//! - a saved shot = `gallery_changed()` (an open Screenshots page shows it at the front);
//! - the thumbnail flies into the gallery's first tile when the Screenshots page is showing, else towards the tray;
//! - the capture toast sits left of the open menu.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{HWND, RECT};

use super::overlay::window::{self, Hooks};

/// The Screenshots page is the tab on screen (set by its open / close).
pub static SHOWN: AtomicBool = AtomicBool::new(false);

/// The open menu window (screen px) and its scale, if it is on screen.
pub fn menu_window() -> Option<(RECT, f32)> {
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowRect, IsWindowVisible};
    unsafe {
        let h: HWND = FindWindowW(crate::testmode::menu_class(), None).ok()?;
        if !IsWindowVisible(h).as_bool() {
            return None;
        }
        let mut r = RECT::default();
        GetWindowRect(h, &mut r).ok()?;
        let dpi = GetDpiForWindow(h);
        Some((r, if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 }))
    }
}

/// The gallery's first tile on screen: window (26, 98), 131 × 74 CSS px (the page unscrolled), when the page shows.
fn first_tile() -> Option<RECT> {
    if !SHOWN.load(Ordering::Relaxed) {
        return None;
    }
    let (r, s) = menu_window()?;
    let (x, y) = (r.left + (26.0 * s).round() as i32, r.top + (98.0 * s).round() as i32);
    Some(RECT { left: x, top: y, right: x + (131.0 * s).round() as i32, bottom: y + (74.0 * s).round() as i32 })
}

/// The Screenshot key was pressed: freeze every monitor and open the capture overlay.
pub fn screenshot_key() -> Result<(), String> {
    window::start(Hooks {
        menu_rect: Box::new(|| menu_window().map(|m| m.0)),
        gallery_target: Box::new(first_tile),
        on_shot: Box::new(|_| super::gallery_changed()),
        ..Hooks::default()
    })
}
