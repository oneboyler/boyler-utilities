//! "Open" a screenshot (double-click in the gallery): the calm lightbox that grows out of the thumbnail (menu-v22 `#lb`).
//! It covers the whole SCREEN above the taskbar (`position:fixed`), not the menu, so it needs a full-screen window to live
//! in - that host is the capture overlay's full-screen window (overlay.rs) / the frame (014 item 1c). This file builds
//! its boxes for a screen of a given size; the page keeps which shot is open (`Screenshots::lb`).

use skia_safe as sk;

use crate::anim::{Bezier, EASE_OUT};
use crate::gfx::{sh, Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{El, Key};
use crate::ui::WHITE;

/// cubic-bezier(.2,.8,.2,1)
const GROW: Bezier = Bezier::new(0.2, 0.8, 0.2, 1.0);

/// The picture's size on screen: `s = min(innerWidth*.7/w, (innerHeight-48)*.72/h, 1)` (the drawing's openLB; 48 = its
/// taskbar), rounded. `screen` = the area above the taskbar (= innerWidth x (innerHeight - 48)), so its height is used as is.
pub fn fit(screen: (f32, f32), pic: (u32, u32)) -> (f32, f32) {
    let (w, h) = (pic.0 as f32, pic.1 as f32);
    let s = (screen.0 * 0.7 / w).min(screen.1 * 0.72 / h).min(1.0);
    ((w * s).round(), (h * s).round())
}

/// The lightbox over a screen `screen` (CSS px; its bottom = the taskbar's top), `opened_at` = when it opened, `from` = the
/// thumbnail's box on that screen (the picture grows out of it: 360 ms cubic-bezier(.2,.8,.2,1)), or None (it fades and
/// scales from .96 in 240 ms EASE_OUT). Closing = the page drops it after the 160 ms fade (`closing_at`).
/// `#lb .lbk{background:rgba(6,8,14,.5);backdrop-filter:blur(8px)}` (fades in 220 ms ease-out)
/// `#lb .lbf{display:flex;flex-direction:column;align-items:center;gap:14px}`
/// `#lb canvas{border-radius:8px;box-shadow:0 0 0 .5px rgba(255,255,255,.16),0 30px 80px rgba(0,0,0,.55)}`
/// `#lb .lbc{display:flex;align-items:center;gap:12px;padding:7px 13px;border-radius:10px;font:12px/16px;color:#fff;
///   background:rgba(22,24,30,.7);backdrop-filter:blur(16px);box-shadow:inset 0 0 0 .5px rgba(255,255,255,.14)}`
/// `#lb .lbc b{font-weight:600}` `#lb .lbc span{color:rgba(255,255,255,.55)}` (the caption rises 6 px, 240 ms, 140 ms delay)
#[allow(clippy::too_many_arguments)]
pub fn lightbox(cx: &mut Cx, key: Key, img: &sk::Image, pic: (u32, u32), name: &str, info: &str, screen: (f32, f32), opened_at: f64, from: Option<(f32, f32, f32, f32)>, closing_at: Option<f64>, bg: Option<sk::Image>) -> El {
    let age = cx.now - opened_at;
    let (w, h) = fit(screen, pic);
    let back = crate::anim::EASE_OUT_CSS.ease((age / 220.0).clamp(0.0, 1.0)) as f32;
    let close = closing_at.map(|t| 1.0 - crate::anim::EASE_IN.ease(((cx.now - t) / 160.0).clamp(0.0, 1.0)) as f32).unwrap_or(1.0);
    if age < 400.0 || closing_at.is_some() {
        cx.st.busy = true;
    }
    // where the picture sits at rest: centred in the column (picture + 14 + caption 30)
    let cap_h = 30.0;
    let col_h = h + 14.0 + cap_h;
    let (px, py) = ((screen.0 - w) / 2.0, (screen.1 - col_h) / 2.0);
    let mut picture_tf = (0.0f32, 0.0f32, 1.0f32, 1.0f32); // dx, dy, scale, opacity
    match from {
        Some((fx, fy, fw, _)) if fw > 0.0 => {
            let p = GROW.ease((age / 360.0).clamp(0.0, 1.0)) as f32;
            let s0 = fw / w;
            picture_tf = ((fx - px) * (1.0 - p), (fy - py) * (1.0 - p), s0 + (1.0 - s0) * p, 1.0);
        }
        _ => {
            let p = EASE_OUT.ease((age / 240.0).clamp(0.0, 1.0)) as f32;
            picture_tf.2 = 0.96 + 0.04 * p;
            picture_tf.3 = p;
        }
    }
    let im = img.clone();
    // a <canvas> (replaced element) is painted on its pixel-snapped box
    let canvas = El::paint(move |g, (x, y, w, h)| {
        let (x, y, w, h) = g.snap(x, y, w, h);
        g.draw_image_rect(&im, x, y, w, h)
    })
        .size(w, h)
        .radius(8.0)
        .clip()
        .shadow(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.16)), sh(0.0, 30.0, 80.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.55))])
        .translate(picture_tf.0, picture_tf.1)
        .scale(picture_tf.2)
        .opacity(picture_tf.3);
    let cp = EASE_OUT.ease(((age - 140.0) / 240.0).clamp(0.0, 1.0)) as f32;
    // #lb sits outside #sw: letter-spacing normal (0), not the menu's -.006em
    let f = Font::new(12.0, 400).ls(0);
    let cap = El::row()
        .center()
        .gap(12.0)
        .pad(7.0, 13.0, 7.0, 13.0)
        .radius(10.0)
        .opacity(cp)
        .translate(0.0, 6.0 * (1.0 - cp))
        // behind the caption: its blurred backdrop, then its tint + ring (painted as children so they stay under the text)
        .children(bg.clone().map(|b| backdrop(b, 10.0, 16.0)))
        .child(El::block().abs(0.0, 0.0, 0.0, 0.0).radius(10.0).bg(Rgba::rgba(22, 24, 30, 0.7)).inset(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.14))]).no_hit())
        .child(El::text(name, Font::new(12.0, 600).ls(0), WHITE, 16.0).none())
        .child(El::text(info, f.tnum(), Rgba(1.0, 1.0, 1.0, 0.55), 16.0).none())
        .child(El::text("Esc or click to close", f.tnum(), Rgba(1.0, 1.0, 1.0, 0.55), 16.0).none());
    El::block()
        .size(screen.0, screen.1)
        .opacity(close)
        .on_click(key)
        .child(
            El::block()
                .abs(0.0, 0.0, 0.0, 0.0)
                .children(bg.map(|b| backdrop(b, 0.0, 8.0)))
                .child(El::block().abs(0.0, 0.0, 0.0, 0.0).bg(Rgba::rgba(6, 8, 14, 0.5)))
                .opacity(back)
                .no_hit(),
        )
        .child(El::col().abs(0.0, 0.0, 0.0, 0.0).center().justify(taffy::style::JustifyContent::CENTER).gap(14.0).no_hit().child(canvas).child(cap))
}

/// `backdrop-filter: blur(<px>)` alone (the frame's El backdrop adds the popups' saturate(180%), which `.lbk` / `.lbc` don't
/// have): the screen behind (`bg`, device px) blurred, under the box's own background.
fn backdrop(bg: sk::Image, r: f32, blur: f32) -> El {
    El::paint(move |g, (x, y, w, h)| g.backdrop(&bg, x, y, w, h, r, blur, &[])).abs(0.0, 0.0, 0.0, 0.0).no_hit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picture_size_is_the_drawings() {
        // 1920 x 1080 screen: a full-HD shot = min(1344/1920, 743.04/1080, 1) = .688 -> 1321 x 743
        // (the screen above its 48 px taskbar: 1920 x 1032)
        assert_eq!(fit((1920.0, 1032.0), (1920, 1080)), (1321.0, 743.0));
        // a small crop is never blown up
        assert_eq!(fit((1920.0, 1032.0), (640, 300)), (640.0, 300.0));
        // the two-monitor shot is limited by the width
        assert_eq!(fit((1920.0, 1032.0), (3840, 1080)), (1344.0, 378.0));
    }
}
