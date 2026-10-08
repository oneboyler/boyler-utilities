//! The timers on your screen (menu-v22 `#tbars`): a small glass pill per timer / place whose "On screen" is on - name ·
//! time, a countdown's thin line that drains and turns red for its last 3 seconds. A countdown / stopwatch shows while it
//! runs (a stopwatch while it holds a time), a place always; while the Timers page is open they all show (so the look can
//! be seen); "Move" lets you drag them on the screen itself (the drawing's snapping: edges at 24 px, the middle, 12 px snap).
//!
//! Real copies: one click-through, topmost, layered tool window (no taskbar button, never takes focus), made only when a
//! pill must show and destroyed when none does; painted by the app's own painter (Skia) into the window with
//! `UpdateLayeredWindow` from pixels it keeps (`crate::dib`). Order 049: the app's vblank-paced animator (`crate::vsync`)
//! wakes it only when a pill will look different (a time's next second, a countdown's line moving a quarter pixel, a
//! place's next minute), on the screen's refresh, and it repaints only when the picture really changed (before: a full
//! repaint into new buffers every 100 ms while a timer ran). A thread timer (`SetTimer` with a TIMERPROC) still wakes the
//! app when the next countdown ends (also with the menu closed). TEST copies never make the window, an ask or a timer:
//! the pills are proven off-screen (`render`, the tests).

use std::cell::{Cell, RefCell};

use skia_safe as sk;
use taffy::style::JustifyContent;

use super::model::{self, Pill, Spot};
use crate::gfx::{sh, CssColor, Font, Gfx, Rgba};
use crate::icons::Icons;
use crate::ui::el::El;
use crate::ui::lay::Laid;
use crate::ui::WHITE;

/// `#tbars .mv{gap:calc(6px*var(--k))}`, `.tbx{width:196px;height:32px}` at size M (k = 1).
pub const GAP: f32 = 6.0;
pub const PILL_W: f32 = 196.0;
pub const PILL_H: f32 = 32.0;
/// The drawing's EDGE / SNAP (the mic icon's snapping, shared by the timers).
pub const EDGE: f32 = 24.0;
pub const SNAP: f32 = 12.0;

/// One pill: `.tbx{display:flex;align-items:center;gap:10px;width:196px;height:32px;padding:0 12px 3px;border-radius:10px;
///   overflow:hidden;color:#fff;font:600 12.5px/1 "Segoe UI Variable Text";background:rgba(22,24,30,.62);
///   backdrop-filter:blur(20px) saturate(1.5);box-shadow:inset 0 0 0 1px rgba(255,255,255,.13),0 6px 18px rgba(0,0,0,.26)}`
/// `.tbn2{flex:1;min-width:0;text-overflow:ellipsis}` `.tbtm{flex:none;font-weight:600;font-variant-numeric:tabular-nums;
///   color:rgba(255,255,255,.88)}` `.tbx.low .tbtm{color:#ff6b62}` `.tbf{position:absolute;left:10px;right:10px;bottom:5px;
///   height:3px;border-radius:2px;background:rgba(255,255,255,.14);overflow:hidden}` `.tbf i{background:var(--c)}`
/// `.tbx.low .tbf i{background:#ff453a}` `.tbx.nl .tbf{display:none}` `.tbx.nl{padding-bottom:0}`. #tbars sits outside the
/// menu's `#sw`: no letter-spacing.
pub fn pill(p: &Pill) -> El {
    let font = Font::new(12.5, 600).ls(0);
    let tm_col = if p.low { Rgba::hex(0xff6b62) } else { Rgba(1.0, 1.0, 1.0, 0.88) };
    let mut e = El::row()
        .center()
        .gap(10.0)
        .size(PILL_W, PILL_H)
        .none()
        .pad(0.0, 12.0, if p.line.is_some() { 3.0 } else { 0.0 }, 12.0)
        .radius(10.0)
        .clip()
        .bg(Rgba::rgba(22, 24, 30, 0.62))
        .shadow(&[sh(0.0, 6.0, 18.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.26))])
        .inset(&[sh(0.0, 0.0, 0.0, 1.0, Rgba(1.0, 1.0, 1.0, 0.13))])
        .child(El::text(p.name.clone(), font, WHITE, 12.5).ellipsis().flex1())
        .child(El::text(p.time.clone(), font.tnum(), tm_col, 12.5).none());
    if let Some(l) = p.line {
        let c = if p.low { Rgba::hex(0xff453a) } else { p.colour };
        e = e.child(
            El::block()
                .abs(10.0, f32::NAN, 10.0, 5.0)
                .h(3.0)
                .radius(2.0)
                .clip()
                .bg(Rgba(1.0, 1.0, 1.0, 0.14))
                .child(El::block().h(3.0).w_pct(l * 100.0).radius(2.0).bg(c)),
        );
    }
    e
}

/// `#tbars .mv` (the stack) with, while moving, `.mv::after` (the dashed outline, inset -6px, radius 16) and `.mv::before`
/// ("Drag to place" 14 px under it; above it when the stack sits at the bottom).
pub fn stack(pills: &[Pill], moving: bool, low_spot: bool) -> El {
    let n = pills.len() as f32;
    let h = n * PILL_H + (n - 1.0).max(0.0) * GAP;
    let mut mv = El::col().gap(GAP).w(PILL_W).h(h).none().children(pills.iter().map(pill));
    if moving {
        // `.mv::after{inset:-6px;border-radius:16px;border:1px dashed rgba(10,132,255,.95)}`: 3 px dashes, 3 px gaps (Blink's
        // dash = 3 x the width for thin borders)
        mv = mv.child(
            El::paint(|g: &Gfx, (x, y, w, h)| {
                let c = Rgba::rgba(10, 132, 255, 0.95);
                let mut pt = sk::Paint::new(c.c4(), None);
                pt.set_anti_alias(true);
                pt.set_style(sk::PaintStyle::Stroke);
                pt.set_stroke_width(1.0);
                pt.set_path_effect(sk::PathEffect::dash(&[3.0, 3.0], 0.0));
                let r = sk::RRect::new_rect_xy(sk::Rect::from_xywh(x + 0.5, y + 0.5, w - 1.0, h - 1.0), 15.5, 15.5);
                g.cv().draw_rrect(r, &pt);
            })
            .abs(-6.0, -6.0, -6.0, -6.0)
            .no_hit(),
        );
        // `content:'Drag to place';padding:5px 9px;border-radius:9px;font:600 11px/1;background:rgba(10,132,255,.92);
        //  box-shadow:0 4px 12px rgba(0,0,0,.25)` centred under (or over) the stack
        let tag = El::row()
            .pad(5.0, 9.0, 5.0, 9.0)
            .radius(9.0)
            .bg(Rgba::rgba(10, 132, 255, 0.92))
            .shadow(&[sh(0.0, 4.0, 12.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.25))])
            .child(El::text("Drag to place", Font::new(11.0, 600).ls(0), WHITE, 11.0));
        let wrap = El::row().justify(JustifyContent::CENTER).no_hit().child(tag);
        mv = mv.child(if low_spot { wrap.abs(0.0, f32::NAN, 0.0, h + 14.0) } else { wrap.abs(0.0, h + 14.0, 0.0, f32::NAN) });
    }
    mv
}

/// Where the stack goes on a work area (x, y, w, h) for a spot (`placeBars`): left / right edge distance, or centred.
pub fn place(spot: Spot, work: (f32, f32, f32, f32), size: (f32, f32)) -> (f32, f32) {
    let (wx, wy, ww, wh) = work;
    let (w, h) = size;
    let dx = spot.dx.clamp(0.0, (ww - w).max(0.0));
    let dy = spot.dy.clamp(0.0, (wh - h).max(0.0));
    let x = match spot.h {
        'L' => wx + dx,
        'R' => wx + ww - dx - w,
        _ => wx + (ww - w) / 2.0,
    };
    let y = if spot.v == 'B' { wy + wh - dy - h } else { wy + dy };
    (x.round(), y.round())
}

/// A drag of the stack (`pointermove` while moving): the new spot, snapped to the edges (EDGE), the middle, within SNAP.
/// `l`, `t` = the stack's wanted top-left inside the work area (w x h).
pub fn snap_spot(l: f32, t: f32, size: (f32, f32), area: (f32, f32)) -> Spot {
    let (w, hh) = size;
    let (aw, ah) = area;
    let (mut l, mut t) = (l, t);
    let xs = [(EDGE, 'L'), (aw - EDGE - w, 'R'), (aw / 2.0 - w / 2.0, 'C')];
    let ys = [(EDGE, 'T'), (ah - EDGE - hh, 'B'), (ah / 2.0 - hh / 2.0, 'C')];
    let near = |v: f32, list: &[(f32, char)]| list.iter().filter(|c| (v - c.0).abs() <= SNAP).min_by(|a, b| (v - a.0).abs().total_cmp(&(v - b.0).abs())).copied();
    let sx = near(l, &xs);
    let sy = near(t, &ys);
    if let Some(s) = sx {
        l = s.0;
    }
    if let Some(s) = sy {
        t = s.0;
    }
    l = l.clamp(0.0, (aw - w).max(0.0));
    t = t.clamp(0.0, (ah - hh).max(0.0));
    let hs = if l + w / 2.0 > aw / 2.0 { 'R' } else { 'L' };
    let vs = if t + hh / 2.0 > ah / 2.0 { 'B' } else { 'T' };
    let mid = sx.map(|s| s.1 == 'C').unwrap_or(false);
    Spot { h: if mid { 'C' } else { hs }, dx: if mid { 0.0 } else if hs == 'R' { aw - (l + w) } else { l }, v: vs, dy: if vs == 'B' { ah - (t + hh) } else { t } }
}

/// Paint the stack at (x, y) DIPs on the current canvas. `base` = the picture under it (the frosted glass); without one
/// (the real screen) the pills keep only their tint.
pub fn paint_stack(g: &Gfx, icons: &Icons, pills: &[Pill], moving: bool, low_spot: bool, x: f32, y: f32, base: Option<&sk::Image>) {
    let root = stack(pills, moving, low_spot);
    let laid = Laid::new(g, El::block().w(PILL_W).child(root), PILL_W, None);
    if let Some(b) = base {
        // backdrop-filter: blur(20px) saturate(1.5) under each pill (the shared painter's popups use 1.8)
        for i in 0..pills.len() {
            let py = y + i as f32 * (PILL_H + GAP);
            g.backdrop(b, x, py, PILL_W, PILL_H, 10.0, 20.0, &[CssColor::Saturate(1.5)]);
        }
    }
    laid.paint(g, icons, x, y, None);
}

/// The off-screen picture of the pills over a desktop picture (`desk`, 1 px = 1 DIP x scale), the stack placed by `spot`
/// on a work area of the desk size minus a 48 px taskbar (the drawing's TB_H). Tests / the proof only.
pub fn render(pills: &[Pill], moving: bool, spot: Spot, desk: &crate::png::Pixels, scale: f32) -> Option<crate::png::Pixels> {
    let mut s = crate::gfx::new_surface(desk.w as i32, desk.h as i32)?;
    let g = Gfx::new(scale);
    let icons = Icons::new();
    let img = crate::png::to_image(desk)?;
    g.begin(s.canvas());
    let (dw, dh) = (desk.w as f32 / scale, desk.h as f32 / scale);
    g.cv().save();
    g.cv().reset_matrix();
    g.cv().draw_image(&img, (0.0, 0.0), None);
    g.cv().restore();
    let n = pills.len() as f32;
    let size = (PILL_W, n * PILL_H + (n - 1.0).max(0.0) * GAP);
    let (x, y) = place(spot, (0.0, 0.0, dw, dh - 48.0), size);
    paint_stack(&g, &icons, pills, moving, spot.v == 'B', x, y, Some(&img));
    g.end();
    Some(crate::png::from_surface(&mut s))
}

// ====================================================================== the real window

thread_local! {
    /// the Timers page is open (preview: every "On screen" pill shows)
    static PREVIEW: Cell<bool> = const { Cell::new(false) };
    static WIN: RefCell<Option<Win>> = const { RefCell::new(None) };
    /// the thread timer that wakes the app when the next countdown ends (its id)
    static ALARM: Cell<usize> = const { Cell::new(0) };
    /// when it fires (ms, the app clock)
    static ALARM_AT: Cell<f64> = const { Cell::new(0.0) };
}

/// What a paint showed (Order 049: an equal one is not painted again): every pill with its line in quarter device pixels,
/// moving, the spot, where the stack sat.
#[derive(Clone, PartialEq)]
struct Shown {
    pills: Vec<(String, String, String, Rgba, bool, Option<i32>)>,
    moving: bool,
    spot: Spot,
    at: (i32, i32),
}

impl Shown {
    fn of(pills: &[Pill], moving: bool, spot: Spot, at: (i32, i32), scale: f32) -> Shown {
        let q = |l: f32| (l * line_px(scale) * 4.0).round() as i32;
        Shown { pills: pills.iter().map(|p| (p.key.clone(), p.name.clone(), p.time.clone(), p.colour, p.low, p.line.map(q))).collect(), moving, spot, at }
    }
}

/// Tests: would these pills paint the same picture as those (Order 049: the window skips an equal one)?
#[cfg(test)]
pub fn same_picture(a: &[Pill], b: &[Pill], scale: f32) -> bool {
    let s = Spot { h: 'C', dx: 0.0, v: 'T', dy: 24.0 };
    Shown::of(a, false, s, (0, 0), scale) == Shown::of(b, false, s, (0, 0), scale)
}

/// A countdown line's full length in device px (`.tbf{left:10px;right:10px}` of a 196 px pill).
pub fn line_px(scale: f32) -> f32 {
    (PILL_W - 20.0) * scale
}

struct Win {
    #[cfg(windows)]
    hwnd: windows::Win32::Foundation::HWND,
    g: Gfx,
    icons: Icons,
    /// the stack's place on the screen (physical px) and size (DIPs) - for the drag
    at: (i32, i32),
    size: (f32, f32),
    scale: f32,
    work: (i32, i32, i32, i32),
    drag: Option<(i32, i32, i32, i32)>,
    last: Vec<Pill>,
    /// the window's pixels, kept
    #[cfg(windows)]
    dib: crate::dib::Dib,
    /// what they show
    shown: Option<Shown>,
}

/// Is the on-screen window there (tests: never in a test copy).
pub fn window_exists() -> bool {
    WIN.with(|w| w.borrow().is_some())
}

/// Is a Windows timer or a frame of the animator asked for (tests: never in a test copy).
pub fn timer_armed() -> bool {
    ALARM.with(|t| t.get() != 0) || crate::vsync::asked(crate::vsync::Client::Timers)
}

/// The page is shown / left (or dropped with the menu).
pub fn set_preview(on: bool) {
    PREVIEW.with(|p| p.set(on));
    sync();
}

/// Bring the screen in line with the model: make / repaint / remove the window, aim the timers. Nothing in test copies.
pub fn sync() {
    let preview = PREVIEW.with(|p| p.get());
    // the line length of the window's monitor (its scale; 1 until the window is made)
    let scale = WIN.with(|w| w.borrow().as_ref().map(|w| w.scale).unwrap_or(1.0));
    let Some((test, pills, moving, spot, next_change, next_end)) =
        model::with_existing(|m| (m.test, m.pills(preview), m.moving, m.spot, m.next_change(preview, line_px(scale)), m.next_end()))
    else {
        return;
    };
    if test || crate::testmode::on() {
        return;
    }
    #[cfg(windows)]
    real::sync(pills, moving, spot, next_change, next_end);
    #[cfg(not(windows))]
    let _ = (pills, moving, spot, next_change, next_end);
}

#[cfg(windows)]
mod real {
    use super::*;
    use std::time::Duration;
    use windows::core::w;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
    use windows::Win32::UI::WindowsAndMessaging::*;

    const CLASS: windows::core::PCWSTR = w!("BoylerUtilities.Timers");
    /// room around the stack for its shadow (0 6px 18px) and the "Drag to place" tag
    const PAD: f32 = 48.0;

    fn work_area() -> ((i32, i32, i32, i32), f32) {
        unsafe {
            let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(mon, &mut mi);
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            let r = mi.rcWork;
            ((r.left, r.top, r.right - r.left, r.bottom - r.top), dx as f32 / 96.0)
        }
    }

    fn ensure_class() {
        thread_local!(static DONE: Cell<bool> = const { Cell::new(false) });
        if DONE.with(|d| d.get()) {
            return;
        }
        unsafe {
            let inst = GetModuleHandleW(None).unwrap_or_default();
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: CLASS, hCursor: LoadCursorW(None, IDC_SIZEALL).unwrap_or_default(), ..Default::default() };
            RegisterClassW(&wc);
        }
        DONE.with(|d| d.set(true));
    }

    pub(super) fn sync(pills: Vec<Pill>, moving: bool, spot: Spot, next_change: Option<Duration>, next_end: Option<Duration>) {
        aim_alarm(next_end);
        if pills.is_empty() {
            WIN.with(|w| {
                if let Some(win) = w.borrow_mut().take() {
                    unsafe {
                        let _ = DestroyWindow(win.hwnd);
                    }
                }
            });
            aim_repaint(None);
            return;
        }
        let made = WIN.with(|w| w.borrow().is_some());
        if !made {
            ensure_class();
            let ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT;
            let hwnd = unsafe { CreateWindowExW(ex, CLASS, w!("Timers"), WS_POPUP, 0, 0, 1, 1, None, None, GetModuleHandleW(None).ok().map(|h| h.into()), None) };
            let Ok(hwnd) = hwnd else { return };
            let (work, scale) = work_area();
            WIN.with(|w| {
                *w.borrow_mut() = Some(Win { hwnd, g: Gfx::new(scale), icons: Icons::new(), at: (0, 0), size: (0.0, 0.0), scale, work, drag: None, last: Vec::new(), dib: crate::dib::Dib::new(), shown: None })
            });
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
        }
        WIN.with(|w| {
            if let Some(win) = w.borrow_mut().as_mut() {
                paint(win, &pills, moving, spot);
                // while moving the stack takes the mouse; otherwise clicks go through it
                unsafe {
                    let ex = GetWindowLongW(win.hwnd, GWL_EXSTYLE);
                    let t = WS_EX_TRANSPARENT.0 as i32;
                    let want = if moving { ex & !t } else { ex | t };
                    if want != ex {
                        SetWindowLongW(win.hwnd, GWL_EXSTYLE, want);
                    }
                }
            }
        });
        aim_repaint(next_change);
    }

    fn paint(win: &mut Win, pills: &[Pill], moving: bool, spot: Spot) {
        let n = pills.len() as f32;
        let size = (PILL_W, n * PILL_H + (n - 1.0).max(0.0) * GAP);
        let s = win.scale;
        let (wx, wy, ww, wh) = win.work;
        let work = (wx as f32 / s, wy as f32 / s, ww as f32 / s, wh as f32 / s);
        let (x, y) = if let Some((_, _, l, t)) = win.drag { (l as f32 / s, t as f32 / s) } else { place(spot, work, size) };
        win.at = ((x * s).round() as i32, (y * s).round() as i32);
        win.size = size;
        win.last = pills.to_vec();
        // Order 049: the same picture is not painted again
        let shown = Shown::of(pills, moving, spot, win.at, s);
        if win.shown.as_ref() == Some(&shown) {
            return;
        }
        let (bw, bh) = (((size.0 + 2.0 * PAD) * s).ceil() as i32, ((size.1 + 2.0 * PAD) * s).ceil() as i32);
        let Some(mut surf) = win.dib.surface(bw, bh) else { return };
        win.g.begin(surf.canvas());
        paint_stack(&win.g, &win.icons, pills, moving, spot.v == 'B', PAD, PAD, None);
        win.g.end();
        drop(surf);
        win.dib.show(win.hwnd, POINT { x: win.at.0 - (PAD * s).round() as i32, y: win.at.1 - (PAD * s).round() as i32 });
        win.shown = Some(shown);
    }

    /// The animator's frame: the pills as they are now.
    fn frame() {
        super::sync();
    }

    unsafe extern "system" fn alarm(_h: HWND, _m: u32, id: usize, _t: u32) {
        let old = ALARM.with(|t| t.get());
        if id == old && old != 0 {
            unsafe {
                let _ = KillTimer(None, old);
            }
            ALARM.with(|t| t.set(0));
            // a countdown reached zero: it finishes (chime); a toast waits for the page if it is open
            model::with_existing(|m| {
                m.check();
                if !PREVIEW.with(|p| p.get()) {
                    m.ended.clear();
                }
            });
        }
        super::sync();
    }

    fn aim_alarm(next: Option<Duration>) {
        // Order 049 review: the pills' frames come every few ms now - an alarm aimed at the same end is left alone
        // (re-arming it each frame would never let it fire: SetTimer waits at least 10-16 ms)
        let want = next.map(|d| crate::timing::now() + d.as_secs_f64() * 1000.0);
        let (old, old_at) = (ALARM.with(|t| t.get()), ALARM_AT.with(|t| t.get()));
        if let (true, Some(w)) = (old != 0, want) {
            if (w - old_at).abs() < 5.0 {
                return;
            }
        }
        if old != 0 {
            unsafe {
                let _ = KillTimer(None, old);
            }
        }
        let id = match next {
            Some(d) => unsafe { SetTimer(None, 0, (d.as_millis() as u32).clamp(1, 0x7fff_ffff), Some(alarm)) },
            None => 0,
        };
        ALARM.with(|t| t.set(id));
        ALARM_AT.with(|t| t.set(want.unwrap_or(0.0)));
    }

    /// Order 049: the next frame when a pill will look different (on the screen's refresh), none when nothing moves.
    fn aim_repaint(next: Option<Duration>) {
        use crate::vsync::{at, cancel, Client};
        match next {
            Some(d) => at(Client::Timers, crate::timing::now() + d.as_secs_f64() * 1000.0, frame),
            None => cancel(Client::Timers),
        }
    }

    unsafe extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        let pt = || {
            let mut p = POINT::default();
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p);
            }
            p
        };
        match msg {
            WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
            WM_LBUTTONDOWN => {
                let p = pt();
                WIN.with(|w| {
                    if let Some(win) = w.borrow_mut().as_mut() {
                        win.drag = Some((p.x, p.y, win.at.0, win.at.1));
                        unsafe {
                            SetCapture(win.hwnd);
                        }
                    }
                });
                return LRESULT(0);
            }
            WM_MOUSEMOVE => {
                let p = pt();
                let spot = WIN.with(|w| {
                    let mut w = w.borrow_mut();
                    let win = w.as_mut()?;
                    let (x0, y0, l0, t0) = win.drag?;
                    let s = win.scale;
                    let (wx, wy, ww, wh) = win.work;
                    let l = (l0 + p.x - x0 - wx) as f32 / s;
                    let t = (t0 + p.y - y0 - wy) as f32 / s;
                    // Order 045: "the same snapping as the mic icon": the blue guides + the corner tag
                    crate::guides::show_for(win.work, s, l, t, win.size, EDGE, SNAP, ((p.x - wx) as f32 / s, (p.y - wy) as f32 / s));
                    Some(snap_spot(l, t, win.size, (ww as f32 / s, wh as f32 / s)))
                });
                if let Some(sp) = spot {
                    model::with_existing(|m| m.spot = sp);
                    WIN.with(|w| {
                        if let Some(win) = w.borrow_mut().as_mut() {
                            // follow the snapped spot (the drag offset is dropped, the spot places the stack)
                            let d = win.drag.take();
                            let pills = win.last.clone();
                            paint(win, &pills, true, sp);
                            win.drag = d;
                        }
                    });
                }
                return LRESULT(0);
            }
            WM_LBUTTONUP | WM_CAPTURECHANGED => {
                WIN.with(|w| {
                    if let Some(win) = w.borrow_mut().as_mut() {
                        win.drag = None;
                    }
                });
                // Order 045: `tbDragEnd`: the guides and the tag go
                crate::guides::hide();
                if msg == WM_LBUTTONUP {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                }
                return LRESULT(0);
            }
            _ => {}
        }
        unsafe { DefWindowProcW(h, msg, wp, lp) }
    }
}

#[cfg(test)]
pub fn reset_for_tests() {
    PREVIEW.with(|p| p.set(false));
}
