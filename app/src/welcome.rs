//! Order 041 item 1 (the owner Oct 8, test 2: "it should show some sort of pop up at the bottom right where to find it"): the
//! first time the app runs (after Setup starts it), a small glass bubble bottom-right just above the tray icon says
//! "Boyler Utilities is here - double-click this icon", its arrow pointing at the icon. It goes by itself after a few
//! seconds, on a click on it, or when the menu opens. Shown once: the settings store remembers it (app scope "welcomed",
//! so "Reset the app's own settings" shows it again). The look is the capture toast's (`#ctoast`): rgba(32,34,42,.8), the
//! .5 px rims, the 0 14px 36px shadow, radius 12. Never in a test copy (a test only paints the bubble off-screen).

use std::cell::RefCell;

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::anim::{self, Bezier, EASE};
use crate::gfx::{sh, Font, Gfx, Rgba};
use crate::icons::Icons;
use crate::pages::screenshots::overlay::window::Layer;
use crate::settings::Scope;
use crate::ui::el::El;
use crate::ui::lay::Laid;

/// The settings key (app scope): the bubble was shown.
pub const SETTING: &str = "welcomed";
/// The words (the order's text, word for word).
pub const TEXT: &str = "Boyler Utilities is here - double-click this icon";
/// How long it stays (ms) before it fades by itself.
const STAY_MS: f64 = 8000.0;
const FADE_IN: f64 = 200.0;
const FADE_OUT: f64 = 260.0;
/// Room around the bubble for its shadow (DIPs), and the arrow under it.
const M: f32 = 40.0;
const ARROW: f32 = 7.0;
const TIMER: usize = 0x5E01;

const fn c(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    Rgba::rgba(r, g, b, ((a * 255.0 + 0.5) as u32) as f32 / 255.0)
}
const BG: Rgba = c(32, 34, 42, 0.8);

struct Bubble {
    layer: Layer,
    g: Gfx,
    scale: f32,
    /// the window's size in DIPs and where the arrow's tip points (x, DIPs inside the window)
    w: f32,
    h: f32,
    tip_x: f32,
    shown_at: f64,
    closing_at: Option<f64>,
}

thread_local! {
    static B: RefCell<Option<Bubble>> = const { RefCell::new(None) };
}

fn now() -> f64 {
    crate::timing::now()
}

/// The bubble's box: one line of text (`#ctoast b`: 600 13px/17px white), padding 10 / 16.
pub fn bubble() -> El {
    El::row()
        .center()
        .pad(10.0, 16.0, 10.0, 16.0)
        .radius(12.0)
        .bg(BG)
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, c(255, 255, 255, 0.16))])
        .shadow(&[sh(0.0, 0.0, 0.0, 0.5, c(0, 0, 0, 0.5)), sh(0.0, 14.0, 36.0, 0.0, c(0, 0, 0, 0.4))])
        .child(El::text(TEXT, Font::new(13.0, 600).ls(0), Rgba::rgb(255, 255, 255), 17.0).none())
}

/// The bubble's size (DIPs) at this scale.
pub fn bubble_size(g: &Gfx) -> (f32, f32) {
    let l = Laid::new(g, El::row().items(taffy::style::AlignItems::FLEX_START).child(bubble()), 10_000.0, None);
    (l.nodes[1].rect.2, l.nodes[1].rect.3)
}

/// Paint the whole window (`w` x `h` DIPs): the bubble at (M, M + dy), its arrow under it pointing down at `tip_x`.
pub fn paint_into(g: &Gfx, canvas: &skia_safe::Canvas, w: f32, h: f32, tip_x: f32, dy: f32) {
    let (bw, bh) = bubble_size(g);
    g.begin(canvas);
    canvas.clear(skia_safe::Color::TRANSPARENT);
    let root = El::block().size(w, h).child(bubble().abs(M, M + dy, f32::NAN, f32::NAN));
    Laid::new(g, root, w, Some(h)).paint(g, &Icons::new(), 0.0, 0.0, None);
    // the arrow: a small triangle of the bubble's glass under its bottom edge, pointing at the icon
    let ax = tip_x.clamp(M + 14.0 + ARROW, M + bw - 14.0 - ARROW);
    let ay = M + dy + bh;
    let mut p = skia_safe::PathBuilder::new();
    p.move_to((ax - ARROW, ay - 0.5));
    p.line_to((ax + ARROW, ay - 0.5));
    p.line_to((ax, ay + ARROW));
    p.close();
    let mut paint = skia_safe::Paint::default();
    paint.set_anti_alias(true);
    paint.set_color4f(BG.c4(), None);
    g.cv().draw_path(&p.detach(), &paint);
    g.end();
}

/// First start: show the bubble once (`tray` = the tray icon's place on screen, None = unknown: bottom-right corner).
pub fn first_run(tray: Option<RECT>) {
    if crate::testmode::on() || cfg!(test) {
        return;
    }
    let seen = crate::services::with(|s| s.store.bool_or(Scope::App, SETTING, false)).unwrap_or(true);
    if seen {
        return;
    }
    if show(tray) {
        crate::services::with(|s| {
            let _ = s.store.set_bool(Scope::App, SETTING, true);
        });
    }
}

fn show(tray: Option<RECT>) -> bool {
    unsafe {
        // the monitor of the tray icon (else the main one) and its work area (the screen above the taskbar)
        let at = tray.map(|r| POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 }).unwrap_or_default();
        let mon = MonitorFromPoint(at, if tray.is_some() { MONITOR_DEFAULTTONEAREST } else { MONITOR_DEFAULTTOPRIMARY });
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let s = dx as f32 / 96.0;
        let g = Gfx::new(s);
        let (bw, bh) = bubble_size(&g);
        let (w, h) = (bw + 2.0 * M, bh + 2.0 * M + ARROW);
        let (pw, ph) = ((w * s).ceil() as i32, (h * s).ceil() as i32);
        let work = mi.rcWork;
        // the icon is in the taskbar (not in the hidden-icons flyout): the arrow points at it; else the bubble sits in the
        // corner with its arrow at its right end
        let icon = tray.filter(|r| r.left >= mi.rcMonitor.left && r.right <= mi.rcMonitor.right && (r.top >= work.bottom - 4 || r.bottom <= work.top + 4));
        let gap = (8.0 * s).round() as i32;
        let tip = icon.map(|r| (r.left + r.right) / 2).unwrap_or(work.right - (40.0 * s) as i32);
        // the arrow's tip just above the work area's bottom (the taskbar at the bottom; elsewhere: the corner)
        let tip_y = work.bottom - gap;
        let mut px = tip - ((M + bw - 14.0 - ARROW) * s).round() as i32;
        px = px.clamp(work.left, work.right - pw + (M * s) as i32 - gap);
        let py = tip_y - ph + ((M - ARROW) * s).round() as i32;
        let tip_x = (tip - px) as f32 / s;
        register();
        let Ok(layer) = Layer::with_class(w!("BoylerUtilities.Welcome"), px, py, pw, ph, false) else { return false };
        let hwnd = layer.hwnd();
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE.0 as isize);
        B.with(|b| *b.borrow_mut() = Some(Bubble { layer, g, scale: s, w, h, tip_x, shown_at: now(), closing_at: None }));
        paint();
        let _ = ShowWindow(hwnd, SW_SHOWNA);
        SetTimer(Some(hwnd), TIMER, 15, None);
    }
    true
}

/// It goes (fades): a click on it, the menu opening.
pub fn dismiss() {
    B.with(|b| {
        if let Ok(mut b) = b.try_borrow_mut() {
            if let Some(bb) = b.as_mut() {
                if bb.closing_at.is_none() {
                    bb.closing_at = Some(now());
                    unsafe {
                        SetTimer(Some(bb.layer.hwnd()), TIMER, 15, None);
                    }
                }
            }
        }
    });
}

/// The opacity and slide (DIPs) `t` ms after it showed, `closing` ms after it started going (None = not going).
pub fn motion(t: f64, closing: Option<f64>) -> (f32, f32) {
    let op_in = EASE.ease(anim::prog(t, 0.0, 0.0, FADE_IN)) as f32;
    let dy = 8.0 * (1.0 - Bezier::new(0.3, 0.7, 0.2, 1.0).ease(anim::prog(t, 0.0, 0.0, 320.0)) as f32);
    let out_t = closing.unwrap_or(t - STAY_MS);
    let op_out = 1.0 - EASE.ease(anim::prog(out_t, 0.0, 0.0, FADE_OUT)) as f32;
    (op_in.min(op_out), dy)
}

/// One frame; false = it is gone (the window closed).
fn paint() -> bool {
    let alive = B.with(|b| {
        let Ok(mut b) = b.try_borrow_mut() else { return true };
        let Some(bb) = b.as_mut() else { return false };
        let t = now();
        let (op, dy) = motion(t - bb.shown_at, bb.closing_at.map(|c| t - c));
        let gone = bb.closing_at.is_some_and(|c| t - c > FADE_OUT) || t - bb.shown_at > STAY_MS + FADE_OUT;
        if gone {
            *b = None;
            return false;
        }
        let _ = bb.scale;
        if let Some(mut surf) = bb.layer.surface() {
            paint_into(&bb.g, surf.canvas(), bb.w, bb.h, bb.tip_x, dy);
        }
        bb.layer.present(op);
        true
    });
    alive
}

fn register() {
    unsafe {
        let inst: HINSTANCE = GetModuleHandleW(None).map(|h| h.into()).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(proc),
            hInstance: inst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("BoylerUtilities.Welcome"),
            ..Default::default()
        };
        RegisterClassExW(&wc);
    }
}

unsafe extern "system" fn proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            dismiss();
            LRESULT(0)
        }
        WM_TIMER if wp.0 == TIMER => {
            // fading in / out: every 15 ms; resting: once a second is enough to notice the end of its stay
            let moving = B.with(|b| {
                b.try_borrow().ok().and_then(|b| b.as_ref().map(|bb| {
                    let t = now();
                    t - bb.shown_at < 400.0 || bb.closing_at.is_some() || t - bb.shown_at > STAY_MS - 20.0
                }))
            });
            if !paint() {
                let _ = KillTimer(Some(h), TIMER);
            } else if moving == Some(false) {
                SetTimer(Some(h), TIMER, 250, None);
            } else {
                SetTimer(Some(h), TIMER, 15, None);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(h, m, wp, lp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_fades_in_stays_and_goes() {
        assert_eq!(motion(0.0, None).0, 0.0);
        assert!((motion(500.0, None).0 - 1.0).abs() < 1e-6 && motion(500.0, None).1.abs() < 1e-3);
        assert!((motion(STAY_MS - 1.0, None).0 - 1.0).abs() < 1e-6, "still there before its stay ends");
        assert!(motion(STAY_MS + FADE_OUT + 1.0, None).0 < 1e-6, "gone by itself");
        assert!(motion(1000.0, Some(FADE_OUT + 1.0)).0 < 1e-6, "a click: gone after the fade");
    }

    /// The bubble painted off-screen (BU_PROOF_DIR set: written as welcome.png there, to look at).
    #[test]
    fn the_bubble_paints() {
        let g = Gfx::new(1.0);
        let (bw, bh) = bubble_size(&g);
        assert!(bw > 200.0 && bh > 30.0 && bh < 45.0, "{bw} x {bh}");
        let (w, h) = (bw + 2.0 * M, bh + 2.0 * M + ARROW);
        let mut surf = crate::gfx::new_surface(w as i32, h as i32).unwrap();
        // a mid-grey desktop under it, like the screen
        paint_into(&g, surf.canvas(), w, h, M + bw - 30.0, 0.0);
        let mut out = crate::gfx::new_surface(w as i32, h as i32).unwrap();
        out.canvas().clear(skia_safe::Color::from_rgb(128, 128, 128));
        out.canvas().draw_image(surf.image_snapshot(), (0, 0), None);
        let px = crate::png::from_surface(&mut out);
        // the bubble's middle is the dark glass, the arrow is under it
        let at = |x: f32, y: f32| {
            let i = ((y as usize) * px.w as usize + x as usize) * 4;
            (px.data[i], px.data[i + 1], px.data[i + 2])
        };
        let mid = at(M + 6.0, M + bh / 2.0);
        assert!(mid.0 < 110 && mid.1 < 110 && mid.2 < 110, "{mid:?}");
        let arrow = at(M + bw - 30.0, M + bh + 2.0);
        assert!(arrow.0 < 110, "the arrow under the bubble: {arrow:?}");
        if let Ok(dir) = std::env::var("BU_PROOF_DIR") {
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
            }
            crate::png::save_png(&px, &format!("{dir}\\welcome.png")).expect("save");
        }
    }
}
