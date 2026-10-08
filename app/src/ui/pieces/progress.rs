//! Progress (menu-v22): the progress line `.updbar` (a share, or indeterminate = a gliding part), the thin scan bar
//! `.scan`, the "busy" spinning icon of a button (`.abb.busy svg{animation:spin .8s linear infinite}`), and the
//! Updating window's status line `.updst`.

use crate::anim::Bezier;
use crate::gfx::{Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{lh, El};
use crate::ui::{ACC, FG2, FG3, TRK};

const SCAN: Bezier = Bezier::new(0.45, 0.0, 0.55, 1.0);

/// `.updbar{height:6px;border-radius:3px;background:var(--trk);overflow:hidden}` `i{border-radius:inherit;
///   background:linear-gradient(90deg,var(--acc),#64c8ff);transition:width .25s linear}` `.ind i{width:38%;
///   animation:scan 1.1s cubic-bezier(.45,0,.55,1) infinite}` (`scan`: translateX(-100%) -> translateX(250%)).
/// `share` = 0..1, or None = indeterminate.
pub fn bar(cx: &mut Cx, key: crate::ui::el::Key, share: Option<f32>) -> El {
    let fill = |w: f32| El::block().abs(0.0, 0.0, f32::NAN, 0.0).w(w).radius(3.0).bg_linear(90.0, &[(0.0, ACC()), (1.0, Rgba::hex(0x64c8ff))]);
    let mut b = El::block().h(6.0).radius(3.0).bg(TRK()).clip();
    match share {
        Some(s) => {
            let p = cx.tr(key, 1, s.clamp(0.0, 1.0), 250.0, Bezier::new(0.0, 0.0, 1.0, 1.0));
            b = b.child(El::paint(move |g, (x, y, w, h)| {
                g.fill_rr_shader(x, y, w * p, h, 3.0, &g.hgrad(x, 0.0, x + w * p, 0.0, &[(0.0, ACC()), (1.0, Rgba::hex(0x64c8ff))]), 1.0);
            })
            .sig(p.to_bits())
            .abs(0.0, 0.0, 0.0, 0.0));
            let _ = fill;
        }
        None => {
            cx.st.busy = true;
            let t = SCAN.ease((cx.now % 1100.0) / 1100.0) as f32;
            b = b.child(El::paint(move |g, (x, y, w, h)| {
                // Blink: the part's box is pixel-snapped where it sits (left 0, 38 % wide), then `translateX` moves the
                // painted part by a sub-pixel amount (not snapped)
                let pw = w * 0.38;
                let t0 = g.transform();
                g.set_transform(&(windows_numerics::Matrix3x2::translation(pw * (-1.0 + 3.5 * t), 0.0) * t0));
                g.fill_rr_shader(x, y, pw, h, 3.0, &g.hgrad(x, 0.0, x + pw, 0.0, &[(0.0, ACC()), (1.0, Rgba::hex(0x64c8ff))]), 1.0);
                g.set_transform(&t0);
            })
            .sig(t.to_bits())
            .abs(0.0, 0.0, 0.0, 0.0));
        }
    }
    b
}

/// `.updst{display:flex;justify-content:space-between;gap:10px;margin-top:8px;font-size:11.5px;color:var(--fg2);
///   font-variant-numeric:tabular-nums}` `span:last-child{color:var(--fg3)}` - what it is doing now | how much.
pub fn status(left: &str, right: &str) -> El {
    let f = Font::new(11.5, 400).tnum();
    El::row()
        .justify(taffy::style::JustifyContent::SPACE_BETWEEN)
        .gap(10.0)
        .margin(8.0, 0.0, 0.0, 0.0)
        .child(El::text(left, f, FG2(), lh(11.5, 1.35)).ellipsis())
        .child(El::text(right, f, FG3(), lh(11.5, 1.35)).none())
}

/// A spinning icon (`animation: spin .8s linear infinite`) for a busy button, e.g. "Checking…".
pub fn spinner(cx: &mut Cx, icon: &str, size: f32, color: Rgba) -> El {
    cx.st.busy = true;
    let deg = ((cx.now % 800.0) / 800.0 * 360.0) as f32;
    El::icon(icon, size, 1.5, color).rotate(deg)
}
