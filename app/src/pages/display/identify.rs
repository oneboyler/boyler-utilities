//! Identify (the monitor icon beside the selector): a big clean number in the middle of each screen for ~2 s (menu-v22
//! `#ident` / `.idn`). It only ever runs in a REAL copy after a click: a test copy (fake runtime) only counts the
//! request, so nothing appears on the screen in a test.
//!
//! Each number is its own small topmost click-through window (layered, no activation, not in the taskbar) centred in the
//! monitor's work area (the drawing's `#ident{bottom:var(--tb)}` = above the taskbar), painted with the app's painter:
//! `.idn{width:176px;height:176px;border-radius:34px;background:rgba(24,26,34,.4);box-shadow:inset 0 0 0 1px
//! rgba(255,255,255,.2),inset 0 1px 0 rgba(255,255,255,.24),0 24px 64px rgba(0,0,0,.42)}` `.idn b{font:600 108px/1
//! "Segoe UI Variable Display";letter-spacing:-.04em;margin-top:-8px;text-shadow:0 2px 12px rgba(0,0,0,.25)}`;
//! in: opacity 0 / scale(.86) → 1, 340 ms cubic-bezier(.3,1.3,.5,1), 40 ms apart; at 1950 ms all fade out (320 ms ease-in).
//! Not drawn: the box's `backdrop-filter:blur(28px) saturate(1.6)` (a layered window cannot blur the screen behind it).

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use bu_display::MonitorInfo;

use crate::anim::{Bezier, EASE_IN};
use crate::gfx::{sh, Font, Gfx, Rgba};

static RUNNING: AtomicBool = AtomicBool::new(false);
static GEN: AtomicU64 = AtomicU64::new(0);
/// Test copies: how many times Identify was asked for (nothing is shown).
pub static FAKE_SHOWN: AtomicUsize = AtomicUsize::new(0);

const IN: Bezier = Bezier::new(0.3, 1.3, 0.5, 1.0);
const HOLD_MS: f64 = 1950.0;
const OUT_MS: f64 = 320.0;

/// An Identify run is on screen. (Order 047: the numbers are their own windows painted on their own thread - the menu
/// needs no frames for them; only tests ask.)
#[cfg(test)]
pub fn running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

/// Shows the numbers (a new click restarts them). `fake` = a test copy: counted, nothing shown.
pub fn show(mons: &[MonitorInfo], fake: bool) {
    if fake {
        FAKE_SHOWN.fetch_add(1, Ordering::Relaxed);
        return;
    }
    #[cfg(windows)]
    {
        let g = GEN.fetch_add(1, Ordering::SeqCst) + 1;
        let list: Vec<(u32, i32, i32, u32, u32)> = mons.iter().map(|m| (m.number, m.rect.x, m.rect.y, m.rect.width, m.rect.height)).collect();
        std::thread::Builder::new().name("bu-identify".into()).spawn(move || win::run(g, list)).ok();
    }
}

/// One number's look at time `t` ms after the click (its own delay `i` × 40 ms): (opacity, scale).
pub fn look_at(t: f64, i: usize) -> (f32, f32) {
    let d = t - 40.0 * i as f64;
    let (o_in, s_in) = if d <= 0.0 {
        (0.0, 0.86)
    } else {
        let e = IN.ease((d / 340.0).min(1.0)) as f32;
        (e.min(1.0), 0.86 + 0.14 * e)
    };
    let out = if t > HOLD_MS { 1.0 - EASE_IN.ease(((t - HOLD_MS) / OUT_MS).min(1.0)) as f32 } else { 1.0 };
    (o_in.clamp(0.0, 1.0) * out, s_in)
}

/// Paints one number box (176 CSS px, at `scale`) into a transparent surface of `size` physical px, centred.
pub fn paint(g: &Gfx, c: &skia_safe::Canvas, size: f32, scale: f32, n: u32, opacity: f32, box_scale: f32) {
    c.clear(skia_safe::Color::TRANSPARENT);
    g.begin(c);
    let css = size / scale;
    let (x, y) = ((css - 176.0) / 2.0, (css - 176.0) / 2.0);
    let (cxp, cyp) = (x + 88.0, y + 88.0);
    c.save();
    c.translate((cxp, cyp));
    c.scale((box_scale, box_scale));
    c.translate((-cxp, -cyp));
    g.push_layer(opacity, None);
    g.box_shadows(x, y, 176.0, 176.0, 34.0, &[sh(0.0, 24.0, 64.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.42))], false);
    g.fill_rr(x, y, 176.0, 176.0, 34.0, Rgba::rgba(24, 26, 34, 0.4));
    g.inset_shadows(x, y, 176.0, 176.0, 34.0, &[sh(0.0, 0.0, 0.0, 1.0, Rgba(1.0, 1.0, 1.0, 0.2)), sh(0.0, 1.0, 0.0, 0.0, Rgba(1.0, 1.0, 1.0, 0.24))]);
    // grid place-items:center with margin-top -8: the 108 px line box sits at 176/2 - 100/2 - 8
    let f = Font::display(108.0, 600).ls(-4320);
    let ty = y + (176.0 - 100.0) / 2.0 - 8.0;
    let shadow = skia_safe::image_filters::drop_shadow((0.0, 2.0), (6.0, 6.0), skia_safe::Color4f::new(0.0, 0.0, 0.0, 0.25), None, None, None);
    let pushed = shadow.map(|sf| g.push_filter(sf)).is_some();
    // Align::Center centres the line on the x it is given: the box's middle, not its left edge
    g.text(&n.to_string(), f, cxp, ty, 108.0, Rgba::rgb(255, 255, 255), crate::gfx::Align::Center, 176.0);
    if pushed {
        g.pop_filter();
    }
    g.pop_layer();
    c.restore();
    g.end();
}

#[cfg(windows)]
mod win {
    use super::*;
    use windows::core::w;
    use windows::Win32::Foundation::*;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::*;

    struct Num {
        hwnd: HWND,
        n: u32,
        x: i32,
        y: i32,
        px: i32,
        scale: f32,
    }

    pub fn run(g: u64, list: Vec<(u32, i32, i32, u32, u32)>) {
        RUNNING.store(true, Ordering::Relaxed);
        unsafe {
            let hinst: HINSTANCE = GetModuleHandleW(None).map(|h| h.into()).unwrap_or_default();
            unsafe extern "system" fn proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
                DefWindowProcW(h, m, w, l)
            }
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(proc),
                hInstance: hinst,
                lpszClassName: w!("BoylerUtilities.Identify"),
                ..Default::default()
            };
            RegisterClassExW(&wc);
            let mut nums = Vec::new();
            for (n, x, y, w, h) in list {
                let c = POINT { x: x + w as i32 / 2, y: y + h as i32 / 2 };
                let mon = MonitorFromPoint(c, MONITOR_DEFAULTTONEAREST);
                let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                let _ = GetMonitorInfoW(mon, &mut mi);
                let (mut dx, mut dy) = (96u32, 96u32);
                let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
                let scale = dx as f32 / 96.0;
                let r = mi.rcWork;
                // the box + room for its shadow (0 24px 64px) on every side
                let px = ((176.0 + 2.0 * 96.0) * scale).round() as i32;
                let wx = (r.left + r.right) / 2 - px / 2;
                let wy = (r.top + r.bottom) / 2 - px / 2;
                let hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    w!("BoylerUtilities.Identify"),
                    w!(""),
                    WS_POPUP,
                    wx,
                    wy,
                    px,
                    px,
                    None,
                    None,
                    Some(hinst),
                    None,
                );
                if let Ok(hwnd) = hwnd {
                    nums.push(Num { hwnd, n, x: wx, y: wy, px, scale });
                }
            }
            let t0 = std::time::Instant::now();
            let gfx: Vec<Gfx> = nums.iter().map(|n| Gfx::new(n.scale)).collect();
            let mut shown = false;
            loop {
                let t = t0.elapsed().as_secs_f64() * 1000.0;
                if GEN.load(Ordering::SeqCst) != g || t > HOLD_MS + OUT_MS {
                    break;
                }
                for (i, (num, g)) in nums.iter().zip(gfx.iter()).enumerate() {
                    let (o, s) = look_at(t, i);
                    if let Some(mut surf) = skia_safe::surfaces::raster_n32_premul((num.px, num.px)) {
                        paint(g, surf.canvas(), num.px as f32, num.scale, num.n, o, s);
                        let p = crate::png::from_surface(&mut surf);
                        blit(num, &p);
                    }
                }
                if !shown {
                    for num in &nums {
                        let _ = ShowWindow(num.hwnd, SW_SHOWNOACTIVATE);
                    }
                    shown = true;
                }
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
            for num in &nums {
                let _ = DestroyWindow(num.hwnd);
            }
        }
        if GEN.load(Ordering::SeqCst) == g {
            RUNNING.store(false, Ordering::Relaxed);
        }
    }

    unsafe fn blit(num: &Num, p: &crate::png::Pixels) {
        let screen = GetDC(None);
        let mdc = CreateCompatibleDC(Some(screen));
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: p.w as i32,
                biHeight: -(p.h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        if let Ok(hb) = CreateDIBSection(Some(mdc), &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
            std::ptr::copy_nonoverlapping(p.data.as_ptr(), bits as *mut u8, p.data.len());
            let old = SelectObject(mdc, hb.into());
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let pt = POINT { x: num.x, y: num.y };
            let size = SIZE { cx: num.px, cy: num.px };
            let src = POINT { x: 0, y: 0 };
            let _ = UpdateLayeredWindow(num.hwnd, None, Some(&pt), Some(&size), Some(mdc), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
            SelectObject(mdc, old);
            let _ = DeleteObject(hb.into());
        }
        let _ = DeleteDC(mdc);
        ReleaseDC(None, screen);
    }
}
