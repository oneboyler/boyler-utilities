//! The small toast (menu-v22 `#toast`): a short note just above the window's bottom edge, 1.8 s.

use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{El, Key};
use crate::ui::{WHITE, WIN_H, WIN_W};

const RISE: Bezier = Bezier::new(0.3, 0.7, 0.2, 1.0);

/// How long a toast shows (the drawing's setTimeout 1800).
pub const SHOW_MS: f64 = 1800.0;

/// `#toast{padding:7px 13px;border-radius:9px;white-space:nowrap;color:#fff;font:600 12px/16px;background:rgba(36,38,46,.88);
///   backdrop-filter:blur(20px) saturate(1.5);box-shadow:inset 0 0 0 .5px rgba(255,255,255,.16),0 8px 22px rgba(0,0,0,.35);
///   opacity:0;transform:translate(-50%,6px);transition:opacity .16s ease,transform .22s cubic-bezier(.3,.7,.2,1)}`
/// `.on{opacity:1;transform:translate(-50%,0)}`, centred, its bottom 20 px above the window's bottom (66 over a
/// selection bar). `shown_at` = when it was asked for; it hides itself after 1.8 s.
pub fn toast(cx: &mut Cx, key: Key, text: &str, shown_at: f64, over_bar: bool) -> El {
    let on = cx.now - shown_at < SHOW_MS;
    let op = cx.tr(key, 1, if on { 1.0 } else { 0.0 }, 160.0, EASE);
    let dy = cx.tr(key, 2, if on { 0.0 } else { 6.0 }, 220.0, RISE);
    if on {
        cx.st.busy = true;
    }
    let font = Font::new(12.0, 600).ls(0);
    let w = cx.g.text_width(text, font) + 26.0;
    let bottom = if over_bar { 66.0 } else { 20.0 };
    El::block()
        .abs((WIN_W - w) / 2.0, WIN_H - bottom - 30.0, f32::NAN, f32::NAN)
        .pad(7.0, 13.0, 7.0, 13.0)
        .radius(9.0)
        .bg(Rgba::rgba(36, 38, 46, 0.88))
        .backdrop(20.0, 1.5)
        .shadow(&[sh(0.0, 8.0, 22.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.35))])
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.16))])
        .opacity(op)
        .translate(0.0, dy)
        .z(60)
        .no_hit()
        .child(El::text(text, font, WHITE, 16.0))
}
