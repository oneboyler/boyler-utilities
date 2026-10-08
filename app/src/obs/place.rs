//! "Custom…" popup position (place.c): an overlay on the monitor where popups appear. Drag the sample to move, drag its
//! corner to resize (60-200 %), hold Shift to snap to the edges and centres, Enter saves, Esc cancels. Saved as fractions
//! of the work area + a size. This overlay may take focus: the user opened it on purpose.

use std::cell::RefCell;

use bu_obs::engine::{Color, Icon, PopMsg};
use bu_obs::monitors::{Mon, Rect};
use bu_obs::popup::{custom_xy, snap, to_fraction};
use bu_obs::settings::{Settings, ST_GLASS, ST_PILL};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{GetDC, ReleaseDC, AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE, VK_RETURN, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::gdi::{self, Dib, Img};

const HINT: &str = "Drag to move · Drag the corner to resize · Hold Shift to snap · Enter to save · Esc to cancel";

struct Pl {
    hwnd: HWND,
    dpi: i32,
    rc: Rect,
    work: Rect,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    scale: i32,
    drag: i32,
    mx: i32,
    my: i32,
    sx: i32,
    sy: i32,
    w0: i32,
    scale0: i32,
    last: (i32, i32),
    gx: Option<i32>,
    gy: Option<i32>,
    set: Settings,
}

thread_local! {
    static P: RefCell<Option<Pl>> = const { RefCell::new(None) };
}

pub fn is_open() -> bool {
    P.with(|p| p.borrow().is_some())
}

/// The sample in the current look at a size (Glass = its own drawing without the blur: an overlay can't show one).
fn sample(set: &Settings, scale: i32, dpi: i32) -> Img {
    let m = PopMsg::sample();
    if set.style == ST_GLASS {
        let s = dpi as f32 / 96.0 * scale as f32 / 100.0;
        let g = crate::gfx::Gfx::new(s);
        let (bw, bh) = super::glass::measure(&g, &m);
        let (w, h) = ((bw * s).round() as i32, (bh * s).round() as i32);
        let Some(mut surf) = crate::gfx::new_surface(w, h) else { return Img { w: 0, h: 0, px: vec![] } };
        g.begin(surf.canvas());
        let mut pal = super::glass::Palette::current();
        pal.tint = crate::gfx::Rgba(pal.tint.0, pal.tint.1, pal.tint.2, 0.85); // no blur on an overlay: the tint nearly solid
        super::glass::draw(&g, &m, 0.0, 0.0, &pal);
        g.end();
        let px = crate::png::from_surface(&mut surf);
        return Img { w, h, px: px.data.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect() };
    }
    gdi::popup_draw(&m, set.style, set.popup_bg(), scale, dpi, 1000)
}

fn blend(dst: &mut [u32], ww: i32, hh: i32, ox: i32, oy: i32, img: &Img) {
    for y in 0..img.h {
        for x in 0..img.w {
            let (xx, yy) = (ox + x, oy + y);
            if xx < 0 || yy < 0 || xx >= ww || yy >= hh {
                continue;
            }
            let s = img.px[(y * img.w + x) as usize];
            let a = s >> 24;
            let i = (yy * ww + xx) as usize;
            let d = dst[i];
            dst[i] = ((a + (d >> 24) * (255 - a) / 255) << 24)
                | ((((s >> 16) & 255) + ((d >> 16) & 255) * (255 - a) / 255) << 16)
                | ((((s >> 8) & 255) + ((d >> 8) & 255) * (255 - a) / 255) << 8)
                | ((s & 255) + (d & 255) * (255 - a) / 255);
        }
    }
}

fn rect_opaque(dst: &mut [u32], ww: i32, hh: i32, x0: i32, y0: i32, w: i32, h: i32, col: u32) {
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            if x >= 0 && y >= 0 && x < ww && y < hh {
                dst[(y * ww + x) as usize] = 0xFF00_0000 | col;
            }
        }
    }
}

/// The whole overlay, monitor-sized.
fn draw(p: &Pl) -> Img {
    let (ww, hh) = (p.rc.w(), p.rc.h());
    let mut px = vec![150u32 << 24; (ww * hh).max(0) as usize];
    let lw = gdi::muldiv(2, p.dpi, 96);
    let hs = gdi::muldiv(12, p.dpi, 96);
    if let Some(gx) = p.gx {
        rect_opaque(&mut px, ww, hh, gx - p.rc.left - lw / 2, p.work.top - p.rc.top, lw, p.work.h(), 0x85B7EB);
    }
    if let Some(gy) = p.gy {
        rect_opaque(&mut px, ww, hh, p.work.left - p.rc.left, gy - p.rc.top - lw / 2, p.work.w(), lw, 0x85B7EB);
    }
    let img = sample(&p.set, p.scale, p.dpi);
    blend(&mut px, ww, hh, p.x - p.rc.left, p.y - p.rc.top, &img);
    rect_opaque(&mut px, ww, hh, p.x + p.w - p.rc.left - hs / 2 - 1, p.y + p.h - p.rc.top - hs / 2 - 1, hs + 2, hs + 2, 0xF1EFE8);
    rect_opaque(&mut px, ww, hh, p.x + p.w - p.rc.left - hs / 2, p.y + p.h - p.rc.top - hs / 2, hs, hs, 0x85B7EB);
    let hm = PopMsg::new(Color::Grey, Icon::Gear, "", "", HINT, "");
    let hint = gdi::popup_draw(&hm, ST_PILL, 0x2C2C2A, 100, p.dpi, 1000);
    blend(&mut px, ww, hh, (ww - hint.w) / 2, gdi::muldiv(24, p.dpi, 96), &hint);
    Img { w: ww, h: hh, px }
}

fn measure(p: &mut Pl) {
    let i = sample(&p.set, p.scale, p.dpi);
    p.w = i.w;
    p.h = i.h;
}

fn clamp_inside(p: &mut Pl) {
    p.x = p.x.min(p.work.right - p.w).max(p.work.left);
    p.y = p.y.min(p.work.bottom - p.h).max(p.work.top);
}

fn redraw(p: &Pl) {
    let img = draw(p);
    let Some(mut dib) = Dib::new(img.w, img.h) else { return };
    dib.px().copy_from_slice(&img.px);
    let bf = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
    unsafe {
        let sdc = GetDC(None);
        let _ = UpdateLayeredWindow(p.hwnd, Some(sdc), Some(&POINT { x: p.rc.left, y: p.rc.top }), Some(&SIZE { cx: img.w, cy: img.h }), Some(dib.dc), Some(&POINT::default()), COLORREF(0), Some(&bf), ULW_ALPHA);
        ReleaseDC(None, sdc);
    }
}

fn on_handle(p: &Pl, x: i32, y: i32) -> bool {
    let hs = gdi::muldiv(12, p.dpi, 96);
    x >= p.x + p.w - hs && x <= p.x + p.w + hs && y >= p.y + p.h - hs && y <= p.y + p.h + hs
}

fn mv(p: &mut Pl, x: i32, y: i32, shift: bool) {
    p.last = (x, y);
    p.gx = None;
    p.gy = None;
    if p.drag == 1 {
        p.x = p.sx + (x - p.mx);
        p.y = p.sy + (y - p.my);
        clamp_inside(p);
        if shift {
            let (nx, ny, gx, gy) = snap(&p.work, p.w, p.h, p.dpi, p.x, p.y);
            p.x = nx;
            p.y = ny;
            p.gx = gx;
            p.gy = gy;
        }
    } else if p.drag == 2 {
        // the size follows the corner smoothly; 60 % .. 200 %
        let nw = p.w0 + (x - p.mx);
        let s = if p.w0 > 0 { p.scale0 * nw / p.w0 } else { p.scale0 };
        p.scale = s.clamp(60, 200);
        measure(p);
        clamp_inside(p);
    }
    redraw(p);
}

fn finish(save: bool) {
    let p = P.with(|c| c.borrow_mut().take());
    let Some(p) = p else { return };
    unsafe {
        let _ = DestroyWindow(p.hwnd);
    }
    if save {
        let (fx, fy) = to_fraction(&p.work, p.x, p.y, p.w, p.h);
        super::on_place(Some((fx, fy, p.scale)));
    } else {
        super::on_place(None);
    }
}

unsafe extern "system" fn proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let pt = |lp: LPARAM| -> (i32, i32) {
        let (x, y) = ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32);
        P.with(|c| c.borrow().as_ref().map(|p| (x + p.rc.left, y + p.rc.top)).unwrap_or((x, y)))
    };
    match m {
        WM_LBUTTONDOWN => {
            SetCapture(h);
            let (x, y) = pt(lp);
            P.with(|c| {
                if let Some(p) = c.borrow_mut().as_mut() {
                    p.mx = x;
                    p.my = y;
                    p.sx = p.x;
                    p.sy = p.y;
                    p.w0 = p.w;
                    p.scale0 = p.scale;
                    p.drag = if on_handle(p, x, y) {
                        2
                    } else if x >= p.x && x < p.x + p.w && y >= p.y && y < p.y + p.h {
                        1
                    } else {
                        0
                    };
                }
            });
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = pt(lp);
            P.with(|c| {
                if let Some(p) = c.borrow_mut().as_mut() {
                    if p.drag != 0 {
                        mv(p, x, y, wp.0 & 4 != 0); // MK_SHIFT
                    }
                }
            });
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            P.with(|c| {
                if let Some(p) = c.borrow_mut().as_mut() {
                    p.drag = 0;
                    p.gx = None;
                    p.gy = None;
                    redraw(p);
                }
            });
            LRESULT(0)
        }
        WM_KEYDOWN | WM_KEYUP => {
            if m == WM_KEYDOWN && wp.0 == VK_RETURN.0 as usize {
                finish(true);
            } else if m == WM_KEYDOWN && wp.0 == VK_ESCAPE.0 as usize {
                finish(false);
            } else if wp.0 == VK_SHIFT.0 as usize {
                P.with(|c| {
                    if let Some(p) = c.borrow_mut().as_mut() {
                        if p.drag != 0 {
                            let (x, y) = p.last;
                            mv(p, x, y, m == WM_KEYDOWN);
                        }
                    }
                });
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let mut c = POINT::default();
            let _ = GetCursorPos(&mut c);
            let cur = P.with(|cc| {
                cc.borrow().as_ref().map(|p| {
                    if on_handle(p, c.x, c.y) {
                        IDC_SIZENWSE
                    } else if c.x >= p.x && c.x < p.x + p.w && c.y >= p.y && c.y < p.y + p.h {
                        IDC_SIZEALL
                    } else {
                        IDC_ARROW
                    }
                })
            });
            if let Ok(hc) = LoadCursorW(None, cur.unwrap_or(IDC_ARROW)) {
                SetCursor(Some(hc));
            }
            LRESULT(1)
        }
        WM_CLOSE => {
            finish(false);
            LRESULT(0)
        }
        _ => DefWindowProcW(h, m, wp, lp),
    }
}

/// Open placement on the monitor where the popup would appear now.
pub fn open(set: &Settings, mons: &[Mon], mon: Option<usize>) {
    if is_open() {
        return;
    }
    let Some(mon) = mon.or_else(|| bu_obs::monitors::primary(mons)) else { return };
    let m = &mons[mon];
    unsafe {
        static REG: std::sync::Once = std::sync::Once::new();
        REG.call_once(|| {
            let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: GetModuleHandleW(None).unwrap_or_default().into(), lpszClassName: w!("BoylerObsPlace"), ..Default::default() };
            RegisterClassW(&wc);
        });
        let Ok(h) = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("BoylerObsPlace"),
            w!("Notifications for OBS placement"),
            WS_POPUP,
            m.rc.left,
            m.rc.top,
            m.rc.w(),
            m.rc.h(),
            None,
            None,
            Some(GetModuleHandleW(None).unwrap_or_default().into()),
            None,
        ) else {
            return;
        };
        let dpi = (GetDpiForWindow(h) as i32).max(96);
        let mut p = Pl {
            hwnd: h,
            dpi,
            rc: m.rc,
            work: m.work,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            scale: set.scale,
            drag: 0,
            mx: 0,
            my: 0,
            sx: 0,
            sy: 0,
            w0: 0,
            scale0: 0,
            last: (0, 0),
            gx: None,
            gy: None,
            set: set.clone(),
        };
        measure(&mut p);
        let mg = gdi::muldiv(12, dpi, 96);
        if set.cx >= 0 {
            let (x, y) = custom_xy(&p.work, p.w, p.h, set.cx, set.cy);
            p.x = x;
            p.y = y;
        } else {
            p.x = p.work.right - mg - p.w;
            p.y = p.work.bottom - mg - p.h;
        }
        redraw(&p);
        P.with(|c| *c.borrow_mut() = Some(p));
        let _ = ShowWindow(h, SW_SHOW);
        let _ = SetForegroundWindow(h);
        let _ = SetFocus(Some(h));
    }
}

/// The overlay picture for a test (monitor-sized, over mid-grey), with the sample at the saved spot.
pub fn picture(set: &Settings, m: &Mon) -> Img {
    let mut p = Pl {
        hwnd: HWND::default(),
        dpi: 96,
        rc: m.rc,
        work: m.work,
        x: 0,
        y: 0,
        w: 0,
        h: 0,
        scale: set.scale,
        drag: 0,
        mx: 0,
        my: 0,
        sx: 0,
        sy: 0,
        w0: 0,
        scale0: 0,
        last: (0, 0),
        gx: None,
        gy: None,
        set: set.clone(),
    };
    measure(&mut p);
    p.x = p.work.right - 12 - p.w;
    p.y = p.work.bottom - 12 - p.h;
    gdi::on_backdrop(&draw(&p), 0x808080, 0)
}

/// Close the overlay without saving (the feature is switched off).
pub fn close() {
    let p = P.with(|c| c.borrow_mut().take());
    if let Some(p) = p {
        unsafe {
            let _ = DestroyWindow(p.hwnd);
        }
    }
}
