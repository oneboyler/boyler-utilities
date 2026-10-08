//! The capture toast (menu-v22 `#ctoast`) and the thumbnail that flies off it (`.thfly`): the finished shot's picture, a green
//! tick, "Screenshot copied" / "Screenshot saved" and its size (+ folder). Shown bottom-right above the taskbar (left of the
//! menu when it is open); at 1.25 s the thumbnail flies into the gallery (if the Screenshots page shows) or into the tray icon.

use std::rc::Rc;

use skia_safe as sk;

use crate::anim::{self, Bezier, EASE};
use crate::gfx::{sh, Font, Rgba};
use crate::ui::el::{key, El, Key};

pub const K_TOAST: Key = key("cap.toast");

const fn c(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    Rgba::rgba(r, g, b, ((a * 255.0 + 0.5) as u32) as f32 / 255.0)
}

/// The toast's timeline (ms after it shows): the thumbnail flies at 1250, the toast fades at 1330 (.2 s), the flight takes 560.
pub const FLY_AT: f64 = 1250.0;
pub const FADE_AT: f64 = 1330.0;
pub const FLY_MS: f64 = 560.0;
/// `cubic-bezier(.55,0,.2,1)` (the flight)
pub const FLY_EASE: Bezier = Bezier::new(0.55, 0.0, 0.2, 1.0);

/// The thumbnail painted like the drawing's paintImg: `#0d0f15` letterbox, the picture "contain"-fitted and centred.
pub fn thumb(img: Rc<sk::Image>, w: f32, h: f32) -> El {
    El::paint(move |g, (x, y, bw, bh)| {
        g.fill_rect(x, y, bw, bh, Rgba::hex(0x0d0f15));
        let (iw, ih) = (img.width() as f32, img.height() as f32);
        let s = (bw / iw).min(bh / ih);
        let (dw, dh) = (iw * s, ih * s);
        g.draw_image_rect(&img, x + (bw - dw) / 2.0, y + (bh - dh) / 2.0, dw, dh);
    })
    .size(w, h)
    .none()
}

/// `#ctoast` at `t` ms after it showed: `display:flex;align-items:center;gap:12px;padding:10px 18px 10px 10px;border-radius:12px;
/// color:#fff;background:rgba(32,34,42,.8);backdrop-filter:blur(30px) saturate(1.6);box-shadow:inset 0 0 0 .5px rgba(255,255,255,.16),
/// 0 0 0 .5px rgba(0,0,0,.5),0 14px 36px rgba(0,0,0,.4)`; opacity 0 / translateY(10px) -> shown (.2s ease / .32s cubic-bezier(.3,.7,.2,1)).
pub fn toast(img: Rc<sk::Image>, title: &str, sub: &str, t: f64) -> El {
    // .ctth{80x45;border-radius:5px;overflow:hidden;background:#0d0f15;box-shadow:0 0 0 .5px rgba(255,255,255,.2)}; flown: canvas fades .2s
    let flown = ease(EASE, anim::prog(t, 0.0, FLY_AT, 200.0));
    let th = El::block()
        .size(80.0, 45.0)
        .none()
        .radius(5.0)
        .clip()
        .bg(Rgba::hex(0x0d0f15))
        .shadow(&[sh(0.0, 0.0, 0.0, 0.5, c(255, 255, 255, 0.2))])
        .child(thumb(img, 80.0, 45.0).opacity(1.0 - flown));
    // b{display:flex;align-items:center;gap:6px;font:600 13px/17px} svg check 13 px stroke #30d158 1.8;
    // span{display:block;margin-top:1px;font:12px/16px;color:rgba(255,255,255,.62);font-variant-numeric:tabular-nums}
    let b = El::row()
        .center()
        .gap(6.0)
        .child(El::icon("check", 13.0, 1.8, Rgba::hex(0x30d158)))
        .child(El::text(title, Font::new(13.0, 600).ls(0), Rgba::rgb(255, 255, 255), 17.0).none());
    let s = El::text(sub, Font::new(12.0, 400).ls(0).tnum(), c(255, 255, 255, 0.62), 16.0).none().margin(1.0, 0.0, 0.0, 0.0);
    let on = toast_shown(t);
    El::row()
        .key(K_TOAST)
        .center()
        .gap(12.0)
        .pad(10.0, 18.0, 10.0, 10.0)
        .radius(12.0)
        .bg(c(32, 34, 42, 0.8))
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, c(255, 255, 255, 0.16))])
        .shadow(&[sh(0.0, 0.0, 0.0, 0.5, c(0, 0, 0, 0.5)), sh(0.0, 14.0, 36.0, 0.0, c(0, 0, 0, 0.4))])
        .opacity(on.0)
        .translate(0.0, on.1)
        .child(th)
        .child(El::col().child(b).child(s))
}

fn ease(b: Bezier, p: f64) -> f32 {
    b.ease(p) as f32
}

/// The toast's opacity and translateY at `t` ms.
pub fn toast_shown(t: f64) -> (f32, f32) {
    let op_in = ease(EASE, anim::prog(t, 0.0, 0.0, 200.0));
    let ty = 10.0 * (1.0 - ease(Bezier::new(0.3, 0.7, 0.2, 1.0), anim::prog(t, 0.0, 0.0, 320.0)));
    let op_out = 1.0 - ease(EASE, anim::prog(t, 0.0, FADE_AT, 200.0));
    (op_in.min(op_out), ty)
}

/// Where the flying thumbnail is at `t` ms (screen px, from the toast's thumbnail box to the target box) and its opacity
/// (1 into the gallery, .45 into the tray). None before / after the flight.
pub fn fly_at(t: f64, from: (f32, f32, f32, f32), to: (f32, f32, f32, f32), to_gallery: bool) -> Option<((f32, f32, f32, f32), f32)> {
    if t < FLY_AT || t > FLY_AT + FLY_MS {
        return None;
    }
    let p = ease(FLY_EASE, anim::prog(t, 0.0, FLY_AT, FLY_MS));
    let l = |a: f32, b: f32| a + (b - a) * p;
    Some(((l(from.0, to.0), l(from.1, to.1), l(from.2, to.2), l(from.3, to.3)), l(1.0, if to_gallery { 1.0 } else { 0.45 })))
}

/// The tray icon's bob after the thumbnail landed in it: translateY 0 -> -3 px (35 %) -> 0 over 320 ms, ease-out.
pub fn tray_bob(t_after_landing: f64) -> f32 {
    let p = (t_after_landing / 320.0).clamp(0.0, 1.0) as f32;
    let e = crate::anim::EASE_OUT_CSS.ease(p as f64) as f32;
    if e < 0.35 {
        -3.0 * e / 0.35
    } else {
        -3.0 * (1.0 - (e - 0.35) / 0.65)
    }
}

/// The tray target box the drawing flies to: 18 x 10 at the icon's centre.
pub fn tray_box(icon: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    (icon.0 + icon.2 / 2.0 - 9.0, icon.1 + icon.3 / 2.0 - 5.0, 18.0, 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_matches_the_drawing() {
        assert_eq!(toast_shown(0.0).0, 0.0);
        assert!((toast_shown(400.0).0 - 1.0).abs() < 1e-6 && toast_shown(400.0).1.abs() < 1e-3);
        assert!(toast_shown(1600.0).0 < 0.01, "gone after the fade");
        assert!(fly_at(1000.0, (0.0, 0.0, 80.0, 45.0), (10.0, 10.0, 18.0, 10.0), false).is_none());
        let (r, op) = fly_at(FLY_AT + FLY_MS, (0.0, 0.0, 80.0, 45.0), (10.0, 10.0, 18.0, 10.0), false).unwrap();
        assert_eq!((r, op), ((10.0, 10.0, 18.0, 10.0), 0.45));
        assert_eq!(tray_box((100.0, 200.0, 24.0, 24.0)), (103.0, 207.0, 18.0, 10.0));
        assert!(tray_bob(112.0) < -2.0 && tray_bob(320.0).abs() < 1e-3);
    }
}
