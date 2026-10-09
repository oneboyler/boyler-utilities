//! Order 056: where the open menu goes back to after a live resolution change. The monitor is the one whose mode the
//! Display tab changed (not the tray icon's: its rect is from before the change, and when the primary shrinks that old
//! point lies on the second monitor), measured fresh after the change.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::*;

/// The Keep / Revert bar's 10 s plus the time Windows needs: after this the change no longer counts.
const FRESH: Duration = Duration::from_secs(60);

static CHANGED: Mutex<Option<(String, Instant)>> = Mutex::new(None);

/// The Display tab changed a mode on this monitor (its GDI name; names are only compared).
pub fn changed(gdi_name: &str) {
    if let Ok(mut c) = CHANGED.lock() {
        *c = Some((gdi_name.to_string(), Instant::now()));
    }
}

/// One monitor as Windows lists it right now.
#[derive(Clone, Debug)]
pub struct Mon {
    pub name: String,
    pub work: RECT,
}

/// The work area of the monitor named `name`.
pub fn pick(mons: &[Mon], name: &str) -> Option<RECT> {
    mons.iter().find(|m| m.name == name).map(|m| m.work)
}

/// The menu's top-left in `work`: 12 px from the right and above the taskbar, its size unchanged (a work area smaller than
/// the menu: the top-left stays on the screen).
pub fn corner(work: RECT, w: i32, h: i32, scale: f32) -> (i32, i32) {
    let m = (12.0 * scale).round() as i32;
    ((work.right - m - w).max(work.left + m), (work.bottom - m - h).max(work.top + m))
}

/// The work area of the monitor the user changed a mode on (None: nothing changed lately, or it is gone).
pub fn changed_work() -> Option<RECT> {
    let name = CHANGED.lock().ok()?.as_ref().filter(|(_, t)| t.elapsed() < FRESH).map(|(n, _)| n.clone())?;
    pick(&list(), &name)
}

fn list() -> Vec<Mon> {
    unsafe extern "system" fn each(h: HMONITOR, _: HDC, _: *mut RECT, l: LPARAM) -> BOOL {
        let v = &mut *(l.0 as *mut Vec<Mon>);
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(h, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
            let n = mi.szDevice.iter().position(|&c| c == 0).unwrap_or(mi.szDevice.len());
            v.push(Mon { name: String::from_utf16_lossy(&mi.szDevice[..n]), work: mi.monitorInfo.rcWork });
        }
        BOOL(1)
    }
    let mut v: Vec<Mon> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut v as *mut Vec<Mon> as isize));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;


    fn rc(l: i32, t: i32, r: i32, b: i32) -> RECT {
        RECT { left: l, top: t, right: r, bottom: b }
    }

    /// Primary 1920x1080 -> 1440x1080 (stretched), the 3440x1440 second monitor to its right or its left.
    fn layouts() -> Vec<(&'static str, Vec<Mon>, RECT)> {
        let main = |l: i32| Mon { name: "DISPLAY1".into(), work: rc(l, 0, l + 1440, 1080 - 48) };
        let wide = |l: i32| Mon { name: "DISPLAY2".into(), work: rc(l, 0, l + 3440, 1440 - 48) };
        vec![
            ("second right", vec![wide(1440), main(0)], rc(0, 0, 1440, 1032)),
            ("second left", vec![wide(-3440), main(0)], rc(0, 0, 1440, 1032)),
        ]
    }

    #[test]
    fn the_menu_lands_on_the_monitor_whose_mode_changed() {
        for (what, mons, want) in layouts() {
            let work = pick(&mons, "DISPLAY1").unwrap_or_else(|| panic!("{what}: monitor not found"));
            assert_eq!((work.left, work.top, work.right, work.bottom), (want.left, want.top, want.right, want.bottom), "{what}");
            let (w, h) = (1000, 700);
            let (x, y) = corner(work, w, h, 1.0);
            assert_eq!((x, y), (1440 - 12 - w, 1032 - 12 - h), "{what}: bottom-right of the changed monitor");
            // the whole menu sits on that monitor, not on the other one
            assert!(x >= work.left && x + w <= work.right && y >= work.top && y + h <= work.bottom, "{what}");
        }
    }

    #[test]
    fn the_other_monitor_changing_moves_the_menu_there() {
        for (what, mons, _) in layouts() {
            let work = pick(&mons, "DISPLAY2").unwrap();
            assert_eq!(work.right - work.left, 3440, "{what}");
        }
        assert!(pick(&layouts()[0].1, "DISPLAY9").is_none());
    }

    #[test]
    fn a_work_area_smaller_than_the_menu_keeps_the_top_left_on_screen() {
        assert_eq!(corner(rc(0, 0, 500, 400), 1000, 700, 1.0), (12, 12));
    }
}
