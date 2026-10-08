//! The lightbox's window (Order 019): a double-click on a shot opens the calm lightbox of menu-v22 `#lb` - it covers the
//! whole screen above the taskbar (the menu's monitor's work area), dims and blurs it (`.lbk`), and grows the picture out
//! of its thumbnail; a click anywhere or Esc closes it (160 ms fade). The boxes are `lightbox::lightbox`; this file is
//! only the window: one layered topmost window (overlay::window::Layer) painted with the app's painter, animated on a
//! timer only while something moves (idle = no frames). A layered window can't read the screen behind it, so the screen
//! under the work area is copied ONCE when it opens (GDI BitBlt) and blurred by the painter like Chromium's backdrop.
//! Never in a test copy (refused): tests never read or cover the screen.
//! It never takes the focus (no-activate): the menu closes when it stops being the active window, and the drawing keeps the
//! menu open under the lightbox. A click lands on the lightbox window itself; Esc reaches the menu, where the Screenshots page
//! holds an empty popup while the lightbox is up, so the frame's "Esc closes the popup" closes the lightbox (`close`).

use std::cell::RefCell;

use skia_safe as sk;
use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::lightbox;
use super::overlay::window::Layer;
use crate::gfx::Gfx;
use crate::icons::Icons;
use crate::ui::cx::{Cx, State};
use crate::ui::el::key;
use crate::ui::lay::Laid;

const TIMER: usize = 0x5C0B;

struct Lb {
    layer: Layer,
    g: Gfx,
    st: State,
    icons: Icons,
    /// the screen under the window (device px), copied when it opened
    bg: sk::Image,
    pic: sk::Image,
    size: (u32, u32),
    name: String,
    info: String,
    /// CSS px of the window, the scale
    css: (f32, f32),
    scale: f32,
    /// the thumbnail's box in the window's CSS px (the picture grows out of it)
    from: Option<(f32, f32, f32, f32)>,
    opened_at: f64,
    closing_at: Option<f64>,
    /// the menu window (the active window when it opened): told when the lightbox goes
    menu: HWND,
}

thread_local! {
    static LB: RefCell<Option<Lb>> = const { RefCell::new(None) };
}

fn now() -> f64 {
    crate::timing::now()
}

/// The lightbox is on screen.
pub fn is_open() -> bool {
    LB.with(|l| l.try_borrow().map(|l| l.is_some()).unwrap_or(true))
}

/// Opens the lightbox for a picture on the monitor of `near` (screen px, e.g. the menu window); `thumb` = the thumbnail's
/// box on screen (px). Refused in a test copy.
pub fn open(pic: sk::Image, size: (u32, u32), name: String, info: String, near: RECT, thumb: Option<RECT>) -> Result<(), String> {
    if crate::testmode::on() || cfg!(test) {
        return Err("a test copy never opens the lightbox".into());
    }
    if is_open() {
        return Ok(());
    }
    unsafe {
        let mon = MonitorFromRect(&near, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let scale = dx as f32 / 96.0;
        let r = mi.rcWork;
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let bg = grab(r).ok_or("couldn’t read the screen")?;
        register();
        let layer = Layer::with_class(w!("BoylerUtilities.Lightbox"), r.left, r.top, w, h, false).map_err(|e| e.to_string())?;
        let from = thumb.map(|t| ((t.left - r.left) as f32 / scale, (t.top - r.top) as f32 / scale, (t.right - t.left) as f32 / scale, (t.bottom - t.top) as f32 / scale));
        let lb = Lb {
            layer,
            g: Gfx::new(scale),
            st: State::default(),
            icons: Icons::new(),
            bg,
            pic,
            size,
            name,
            info,
            css: (w as f32 / scale, h as f32 / scale),
            scale,
            from,
            opened_at: now(),
            closing_at: None,
            // the double-click came from the menu, so it is the active window now
            menu: GetForegroundWindow(),
        };
        let hwnd = lb.layer.hwnd();
        // never activated (the menu stays the active window)
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE.0 as isize);
        LB.with(|l| *l.borrow_mut() = Some(lb));
        paint();
        let _ = ShowWindow(hwnd, SW_SHOWNA);
        SetTimer(Some(hwnd), TIMER, 15, None);
    }
    Ok(())
}

/// The screen inside `r` (device px) as a picture (GDI: what is on screen right now; alpha set opaque).
unsafe fn grab(r: RECT) -> Option<sk::Image> {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    let screen = GetDC(None);
    let mdc = CreateCompatibleDC(Some(screen));
    let bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
        ..Default::default()
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let hb = CreateDIBSection(Some(mdc), &bi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
    let old = SelectObject(mdc, hb.into());
    let ok = BitBlt(mdc, 0, 0, w, h, Some(screen), r.left, r.top, SRCCOPY).is_ok();
    let img = if ok {
        let len = (w * h * 4) as usize;
        let mut px = std::slice::from_raw_parts(bits as *const u8, len).to_vec();
        for a in px.iter_mut().skip(3).step_by(4) {
            *a = 255;
        }
        let ii = sk::ImageInfo::new((w, h), sk::ColorType::BGRA8888, sk::AlphaType::Premul, Some(sk::ColorSpace::new_srgb()));
        sk::images::raster_from_data(&ii, sk::Data::new_copy(&px), (w * 4) as usize)
    } else {
        None
    };
    SelectObject(mdc, old);
    let _ = DeleteObject(hb.into());
    let _ = DeleteDC(mdc);
    ReleaseDC(None, screen);
    img
}

fn register() {
    unsafe {
        let inst: HINSTANCE = GetModuleHandleW(None).map(|h| h.into()).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(proc),
            hInstance: inst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("BoylerUtilities.Lightbox"),
            ..Default::default()
        };
        RegisterClassExW(&wc);
    }
}

unsafe extern "system" fn proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        // a click never activates it (the menu would close)
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // the drawing: a press anywhere (pointerdown) or Esc closes it
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            close();
            LRESULT(0)
        }
        // (Order 045: Space / Enter too, L7961)
        WM_KEYDOWN if matches!(wp.0, 0x1B | 0x20 | 0x0D) => {
            close();
            LRESULT(0)
        }
        WM_TIMER if wp.0 == TIMER => {
            if !paint() {
                let _ = KillTimer(Some(h), TIMER);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(h, m, wp, lp),
    }
}

/// Starts the closing fade (Esc in the menu: the Screenshots page's popup dismiss).
pub fn close() {
    LB.with(|l| {
        if let Ok(mut b) = l.try_borrow_mut() {
            if let Some(lb) = b.as_mut() {
                if lb.closing_at.is_none() {
                    lb.closing_at = Some(now());
                    unsafe {
                        SetTimer(Some(lb.layer.hwnd()), TIMER, 15, None);
                    }
                }
            }
        }
    });
}

/// The lightbox went: the menu under the pointer hears a mouse move at once (what Windows sends it anyway when the window
/// above goes), so it draws a frame, its page sees the lightbox gone and a click right after reaches the page.
fn wake(menu: HWND) {
    unsafe {
        if menu.is_invalid() || !IsWindow(Some(menu)).as_bool() {
            return;
        }
        let mut p = POINT::default();
        if GetCursorPos(&mut p).is_err() || !ScreenToClient(menu, &mut p).as_bool() {
            return;
        }
        let lp = ((p.y as u16 as u32) << 16 | p.x as u16 as u32) as isize;
        let _ = PostMessageW(Some(menu), WM_MOUSEMOVE, WPARAM(0), LPARAM(lp));
    }
}

/// One frame; false = nothing moves (the timer stops). After the closing fade the window goes.
fn paint() -> bool {
    let done = LB.with(|l| {
        let Ok(mut b) = l.try_borrow_mut() else { return false };
        let Some(lb) = b.as_mut() else { return false };
        let t = now();
        if lb.closing_at.is_some_and(|c| t - c >= 160.0) {
            return true;
        }
        let mut cx = Cx::new(t, false, &lb.g, &mut lb.st);
        let el = lightbox::lightbox(&mut cx, key("shot.lb"), &lb.pic, lb.size, &lb.name, &lb.info, lb.css, lb.opened_at, lb.from, lb.closing_at, Some(lb.bg.clone()));
        let busy = cx.st.busy;
        lb.st.busy = false;
        let laid = Laid::new(&lb.g, el, lb.css.0, Some(lb.css.1));
        let Some(mut surf) = lb.layer.surface() else { return false };
        // the screen copy 1:1 (device px) = what is under the window; the boxes' backdrops blur it
        {
            let cv = surf.canvas();
            cv.clear(sk::Color::TRANSPARENT);
            cv.draw_image(&lb.bg, (0, 0), None);
        }
        let base = lb.bg.clone();
        lb.g.begin(surf.canvas());
        laid.paint(&lb.g, &lb.icons, 0.0, 0.0, Some(&base));
        lb.g.end();
        let _ = lb.scale;
        lb.layer.present(1.0);
        if !busy {
            unsafe {
                let _ = KillTimer(Some(lb.layer.hwnd()), TIMER);
            }
        }
        false
    });
    if done {
        let gone = LB.with(|l| l.try_borrow_mut().ok().and_then(|mut b| b.take()));
        if let Some(lb) = gone {
            let menu = lb.menu;
            drop(lb);
            wake(menu);
        }
        return false;
    }
    is_open()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REVIEW 019 HOLD 1: the lightbox never becomes the active window (the menu closes when it stops being active):
    /// a click on it answers "don't activate", and nothing in this file shows it activated or gives it the focus.
    #[test]
    fn the_lightbox_never_takes_the_focus() {
        let r = unsafe { proc(HWND::default(), WM_MOUSEACTIVATE, WPARAM(0), LPARAM(0)) };
        assert_eq!(r.0, MA_NOACTIVATE as isize);
        let src = include_str!("lbhost.rs");
        // the code above this test module (any line endings; the module's name is split so this line can't match it)
        let code = src.split(concat!("mod ", "tests {")).next().unwrap();
        assert!(code.len() < src.len() && code.contains("fn proc(") && !code.contains("fn the_lightbox_never_takes_the_focus"));
        for bad in [concat!("SetForeground", "Window("), concat!("SetFo", "cus("), concat!("SetActive", "Window("), concat!("SW_", "SHOW)")] {
            assert!(!code.contains(bad), "{bad} in lbhost.rs");
        }
        assert!(code.contains("SW_SHOWNA") && code.contains("WS_EX_NOACTIVATE"));
    }
}
