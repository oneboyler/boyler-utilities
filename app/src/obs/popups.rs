//! The popup windows (popup.c's window part), on the app's UI thread: topmost, click-through, never take focus, no
//! taskbar button, left out of screen capture (recordings), on the monitor the "Show on" setting picks (ClipPing's
//! rule: "Other monitor" = not the one being clipped), stacked at the chosen corner or custom spot, sliding / fading in
//! and out. A popup's timer runs only while it moves or fades (or the Timer bar runs) - none while nothing shows.
//! ClipPing's six looks are layered windows with GDI pictures (gdi.rs); the Glass look is a composition window with the
//! menu's glass (glass.rs + comp.rs).

use std::cell::RefCell;
use std::time::Instant;

use bu_obs::engine::PopMsg;
use bu_obs::monitors::{self, Mon};
use bu_obs::popup::{anim_frame, pick_corner, slide_dir, stack, HOLD_MS, IN_MS};
use bu_obs::settings::{Settings, AN_NONE, P_CUSTOM, ST_GLASS, ST_TIMER, W_NONE};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_USE_HOSTBACKDROPBRUSH, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND};
use windows::Win32::Graphics::Gdi::{GetDC, ReleaseDC, AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::gdi::{self, Dib};

const MAX_POP: usize = 6;

enum Kind {
    Gdi(Dib),
    Glass(GlassWin),
}

struct GlassWin {
    _chain: crate::present::Chain,
    _mask: crate::present::Chain,
    glass: crate::comp::Glass,
    /// the box's offset inside the window (shadow room), device px
    ml: i32,
    mt: i32,
}

struct Pop {
    hwnd: HWND,
    kind: Kind,
    /// the box (device px)
    w: i32,
    h: i32,
    dpi: i32,
    bx: i32,
    by: i32,
    dir: i32,
    style: i32,
    barq: i32,
    t0: Instant,
    msg: PopMsg,
    bg: u32,
    scale: i32,
    anim: i32,
    test: bool,
}

#[derive(Default)]
struct State {
    pops: Vec<Pop>,
    target: Option<usize>,
    corner: i32,
    cx: i32,
    cy: i32,
    work: Option<bu_obs::monitors::Rect>,
    /// what a test copy would have shown (the test hook / unit tests read it)
    shown: Vec<String>,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State::default());
}

/// test copies never show a window: they only note what would be shown
pub fn set_hidden(on: bool) {
    HIDDEN.with(|h| *h.borrow_mut() = on);
}
thread_local! {
    static HIDDEN: RefCell<bool> = const { RefCell::new(false) };
}
fn hidden() -> bool {
    HIDDEN.with(|h| *h.borrow())
}

/// What the popups showed in a hidden (test) run, oldest first.
pub fn shown() -> Vec<String> {
    S.with(|s| s.borrow().shown.clone())
}

fn class() -> PCWSTR {
    static REG: std::sync::Once = std::sync::Once::new();
    REG.call_once(|| unsafe {
        let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: GetModuleHandleW(None).unwrap_or_default().into(), lpszClassName: w!("BoylerObsPopup"), ..Default::default() };
        RegisterClassW(&wc);
    });
    w!("BoylerObsPopup")
}

unsafe extern "system" fn proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_TIMER => {
            tick(h);
            LRESULT(0)
        }
        _ => DefWindowProcW(h, m, wp, lp),
    }
}

/// keep a window out of screen capture (OBS's display capture too); Windows 10 2004+
pub fn no_capture(h: HWND) {
    unsafe {
        let _ = SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE);
    }
}

fn create(x: i32, y: i32, glass: bool) -> Option<HWND> {
    let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | if glass { WS_EX_NOREDIRECTIONBITMAP } else { WINDOW_EX_STYLE(0) };
    unsafe {
        let h = CreateWindowExW(ex, class(), w!(""), WS_POPUP, x, y, 1, 1, None, None, Some(GetModuleHandleW(None).ok()?.into()), None).ok()?;
        no_capture(h);
        if glass {
            // composition content on a layered, click-through window: the layer itself fully opaque
            let _ = SetLayeredWindowAttributes(h, COLORREF(0), 255, LWA_ALPHA);
            let pref = DWMWCP_DONOTROUND;
            let _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const _ as *const _, 4);
            let none: u32 = 0xFFFF_FFFE;
            let _ = DwmSetWindowAttribute(h, DWMWA_BORDER_COLOR, &none as *const _ as *const _, 4);
            let on: i32 = 1;
            let _ = DwmSetWindowAttribute(h, DWMWA_USE_HOSTBACKDROPBRUSH, &on as *const _ as *const _, 4);
        }
        Some(h)
    }
}

fn dpi_of(h: HWND) -> i32 {
    (unsafe { GetDpiForWindow(h) } as i32).max(96)
}

/// The glass popup's window content: composition glass + Skia's pixels. Returns the window kind and the box size.
fn make_glass(h: HWND, m: &PopMsg, dpi: i32, scale: i32) -> Option<(Kind, i32, i32)> {
    // the theme as Settings has it now (the menu may never have opened since the app started)
    if !crate::services::in_use() {
        crate::ui::sync_theme();
    }
    let s = dpi as f32 / 96.0 * scale as f32 / 100.0;
    let g = crate::gfx::Gfx::new(s);
    let (bw, bh) = super::glass::measure(&g, m);
    let (ww, wh) = (((bw + super::glass::M_L + super::glass::M_R) * s).ceil() as i32, ((bh + super::glass::M_T + super::glass::M_B) * s).ceil() as i32);
    let (ml, mt) = ((super::glass::M_L * s).round() as i32, (super::glass::M_T * s).round() as i32);
    let ox = ml as f32 / s;
    let oy = mt as f32 / s;
    let d3d = crate::present::new_d3d().ok()?;
    let chain = crate::present::Chain::new_on(d3d.clone(), ww as u32, wh as u32).ok()?;
    let mask = crate::present::Chain::new_on(d3d, ww as u32, wh as u32).ok()?;
    let mut ms = crate::gfx::new_surface(ww, wh)?;
    g.begin(ms.canvas());
    g.clip_coverage(ox, oy, bw, bh, super::glass::RADIUS);
    g.end();
    let mp = crate::png::from_surface(&mut ms);
    mask.present(&mp.data, (ww * 4) as u32).ok()?;
    let mut cs = crate::gfx::new_surface(ww, wh)?;
    g.begin(cs.canvas());
    super::glass::draw(&g, m, ox, oy, &super::glass::Palette::current());
    g.end();
    let cp = crate::png::from_surface(&mut cs);
    chain.present(&cp.data, (ww * 4) as u32).ok()?;
    let glass = crate::comp::Glass::new(h, (&chain.swap, chain.w, chain.h), &mask.swap, None, ww as f32, wh as f32, super::glass::RADIUS * s, crate::comp::GlassMode::Host).ok()?;
    glass.set_opacity(0.0);
    unsafe {
        let _ = SetWindowPos(h, None, 0, 0, ww, wh, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
    Some((Kind::Glass(GlassWin { _chain: chain, _mask: mask, glass, ml, mt }), (bw * s).round() as i32, (bh * s).round() as i32))
}

fn place(p: &mut Pop) {
    let t = p.t0.elapsed().as_millis() as i32;
    let Some((dx, dy, al, bq)) = anim_frame(p.anim, p.dir, gdi::muldiv(60, p.dpi, 96), t) else { return };
    match &mut p.kind {
        Kind::Gdi(dib) => {
            if p.style == ST_TIMER && bq != p.barq {
                p.barq = bq;
                let img = gdi::popup_draw(&p.msg, p.style, p.bg, p.scale, p.dpi, p.barq);
                if img.w == dib.w && img.h == dib.h {
                    dib.px().copy_from_slice(&img.px);
                }
            }
            let pt = POINT { x: p.bx + dx, y: p.by + dy };
            let sz = SIZE { cx: dib.w, cy: dib.h };
            let bf = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: al as u8, AlphaFormat: AC_SRC_ALPHA as u8 };
            unsafe {
                let sdc = GetDC(None);
                let _ = UpdateLayeredWindow(p.hwnd, Some(sdc), Some(&pt), Some(&sz), Some(dib.dc), Some(&POINT::default()), COLORREF(0), Some(&bf), ULW_ALPHA);
                ReleaseDC(None, sdc);
            }
        }
        Kind::Glass(gw) => {
            gw.glass.set_opacity(al as f32 / 255.0);
            unsafe {
                let _ = SetWindowPos(p.hwnd, Some(HWND_TOPMOST), p.bx + dx - gw.ml, p.by + dy - gw.mt, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }
}

/// next wake-up: every frame while moving / fading (or the Timer bar runs), otherwise one wait until the fade-out
fn schedule(p: &Pop) {
    let t = p.t0.elapsed().as_millis() as i32;
    let inn = if p.anim == AN_NONE { 0 } else { IN_MS };
    let frame = t < inn || t >= inn + HOLD_MS || p.style == ST_TIMER;
    let ms = if frame { 16 } else { (inn + HOLD_MS - t).max(1) as u32 };
    unsafe {
        SetTimer(Some(p.hwnd), 1, ms, None);
    }
}

fn free(st: &mut State, i: usize) {
    let p = st.pops.remove(i);
    unsafe {
        let _ = KillTimer(Some(p.hwnd), 1);
    }
    if let Kind::Glass(gw) = &p.kind {
        gw.glass.close();
    }
    unsafe {
        let _ = DestroyWindow(p.hwnd);
    }
}

fn restack(st: &mut State) {
    let Some(wk) = st.work else { return };
    if st.pops.is_empty() {
        return;
    }
    let mg = gdi::muldiv(12, st.pops[st.pops.len() - 1].dpi, 96);
    let gap = gdi::muldiv(8, st.pops[st.pops.len() - 1].dpi, 96);
    let sizes: Vec<(i32, i32)> = st.pops.iter().map(|p| (p.w, p.h)).collect();
    let at = stack(&wk, st.corner, &sizes, mg, gap, (st.cx, st.cy));
    for (p, (x, y)) in st.pops.iter_mut().zip(at) {
        p.bx = x;
        p.by = y;
    }
    for p in st.pops.iter_mut().rev() {
        place(p);
    }
}

fn tick(h: HWND) {
    S.with(|s| {
        let Ok(mut st) = s.try_borrow_mut() else { return };
        let Some(i) = st.pops.iter().position(|p| p.hwnd == h) else {
            unsafe {
                let _ = KillTimer(Some(h), 1);
            }
            return;
        };
        let t = st.pops[i].t0.elapsed().as_millis() as i32;
        if anim_frame(st.pops[i].anim, st.pops[i].dir, 60, t).is_none() {
            free(&mut st, i);
            restack(&mut st);
            return;
        }
        place(&mut st.pops[i]);
        schedule(&st.pops[i]);
    });
}

/// Show one popup (popup.c `popup_show`). `clipped` = the monitor being clipped.
pub fn show(m: &PopMsg, clipped: Option<usize>, set: &Settings, mons: &[Mon], test: bool) {
    let t = monitors::pick_monitor(mons, set.where_, clipped);
    if set.where_ == W_NONE {
        return; // "Show on: None": no popup (the sound still plays)
    }
    let Some(t) = t else { return };
    let corner = pick_corner(set, mons, Some(t), clipped);
    if hidden() {
        S.with(|s| s.borrow_mut().shown.push(format!("{} | {} | monitor {}", m.main, m.top, mons[t].num)));
        return;
    }
    S.with(|s| {
        let Ok(mut st) = s.try_borrow_mut() else { return };
        if st.target != Some(t) || st.corner != corner || (corner == P_CUSTOM && (st.cx, st.cy) != (set.cx, set.cy)) {
            // monitor or position changed: drop the old stack
            while !st.pops.is_empty() {
                free(&mut st, 0);
            }
            st.target = Some(t);
            st.corner = corner;
        }
        st.cx = set.cx;
        st.cy = set.cy;
        st.work = Some(mons[t].work);
        if test {
            while let Some(i) = st.pops.iter().position(|p| p.test) {
                free(&mut st, i);
            }
        }
        if st.pops.len() == MAX_POP {
            free(&mut st, 0);
        }
        let glass = set.style == ST_GLASS;
        let Some(hw) = create(mons[t].work.right - 2, mons[t].work.bottom - 2, glass) else { return };
        let dpi = dpi_of(hw);
        let (kind, w, h) = if glass {
            match make_glass(hw, m, dpi, set.scale) {
                Some(k) => k,
                None => {
                    unsafe {
                        let _ = DestroyWindow(hw);
                    }
                    return;
                }
            }
        } else {
            let img = gdi::popup_draw(m, set.style, set.popup_bg(), set.scale, dpi, 1000);
            let Some(mut dib) = Dib::new(img.w, img.h) else {
                unsafe {
                    let _ = DestroyWindow(hw);
                }
                return;
            };
            dib.px().copy_from_slice(&img.px);
            (Kind::Gdi(dib), img.w, img.h)
        };
        st.pops.push(Pop {
            hwnd: hw,
            kind,
            w,
            h,
            dpi,
            bx: 0,
            by: 0,
            dir: 0,
            style: set.style,
            barq: 1000,
            t0: Instant::now(),
            msg: m.clone(),
            bg: set.popup_bg(),
            scale: set.scale,
            anim: set.anim,
            test,
        });
        restack(&mut st);
        let wk = mons[t].work;
        let n = st.pops.len() - 1;
        let p = &mut st.pops[n];
        p.dir = slide_dir(corner, &wk, p.bx, p.by, p.w, p.h);
        place(p);
        unsafe {
            let _ = ShowWindow(hw, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(hw, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
        schedule(&st.pops[n]);
    });
}

/// Every popup gone (settings changed, feature off).
pub fn clear() {
    S.with(|s| {
        if let Ok(mut st) = s.try_borrow_mut() {
            while !st.pops.is_empty() {
                free(&mut st, 0);
            }
        }
    });
}

/// After a popup setting changes: ONE silent popup exactly as it will look; a new one replaces the previous test popup.
pub fn test(clipped: Option<usize>, set: &Settings, mons: &[Mon]) {
    if set.where_ == W_NONE {
        return;
    }
    show(&PopMsg::sample(), clipped, set, mons, true);
}
