//! Small shared bits of menu-v22 (Order 025 batch 8): the spinner `.uspin`, the add button `.addb`, the app tile `.at`, a link
//! with its parent's weight (`.lnk` + `#sw button{font:inherit}`) and the group header's grey run `.ghs`.

use skia_safe as sk;

use crate::anim::EASE;
use crate::gfx::{sh, Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{lh, Cursor, El, Key};
use crate::ui::{ACC, FG3, HOV, ICO_ON, TRK, WHITE};

use super::btn_font;

/// Apps' "Uninstalling…" spinner: an 11 px ring (1.6 px `--trk`) whose top quarter is `--acc`, turning once every .8 s
/// (3 s under reduced motion). `since` = when it started turning (the page's clock).
///
/// `.uspin{width:11px;height:11px;flex:none;border-radius:50%;border:1.6px solid var(--trk);border-top-color:var(--acc);
///   animation:cspin .8s linear infinite}` `@keyframes cspin{to{transform:rotate(360deg)}}` (reduced motion: `animation-duration:3s`)
pub fn uspin(cx: &mut Cx, since: f64) -> El {
    let period = if cx.rm { 3000.0 } else { 800.0 };
    let turn = (((cx.now - since) / period).rem_euclid(1.0) * 360.0) as f32;
    cx.st.busy = true;
    El::paint(|g, (x, y, w, h)| {
        // the border box's ring: its middle line at .8 px inside, 1.6 px wide; the top border = the quarter between the two
        // upper diagonals (a box's corner split runs from the outer to the inner corner: through the centre on a circle)
        let b = 1.6;
        let (rx, ry, rw, rh) = (x + b / 2.0, y + b / 2.0, w - b, h - b);
        g.stroke_rr(rx, ry, rw, rh, rw / 2.0, b, TRK());
        let mut pb = sk::PathBuilder::new();
        pb.add_arc(sk::Rect::from_xywh(rx, ry, rw, rh), 225.0, 90.0);
        let mut p = sk::Paint::new(ACC().c4(), None);
        p.set_anti_alias(true);
        p.set_style(sk::PaintStyle::Stroke);
        p.set_stroke_width(b);
        g.cv().draw_path(&pb.detach(), &p);
    })
    .sig(())
    .size(11.0, 11.0)
    .none()
    .rotate(turn)
    .no_hit()
}

/// The add button in a list's last row (`.row.addr`, 40 px - 36 in Mouse's acceleration card): "Add app", "Add a key"; `apsv`
/// = Mouse's "Save as preset" (`.addb.apsv`: no -6 px pull, 12 px). Click = `Ev::Click(key)`.
///
/// `#sw .addb{display:inline-flex;align-items:center;gap:6px;height:26px;margin-left:-6px;padding:0 10px 0 6px;border-radius:7px;
///   background:transparent;color:var(--ico-on);font-size:12.5px;font-weight:600;transition:background-color .12s ease,transform .12s ease}`
/// `:hover{background:var(--hov)}` `:active{transform:scale(.97)}` (not under reduced motion) `.addb svg{width:14px;height:14px;
///   stroke-width:1.6}` (`ICON.dplus`) `#sw .addb.apsv{height:26px;margin-left:0;font-size:12px}`
pub fn addb(cx: &mut Cx, key: Key, label: &str, apsv: bool) -> El {
    let hv = cx.hover_t(key, 120.0, EASE);
    let pr = if cx.rm { 0.0 } else { cx.active_t(key, 120.0, EASE) };
    let fs = if apsv { 12.0 } else { 12.5 };
    El::row()
        .center()
        .gap(6.0)
        .h(26.0)
        .none()
        .margin(0.0, 0.0, 0.0, if apsv { 0.0 } else { -6.0 })
        .pad(0.0, 10.0, 0.0, 6.0)
        .radius(7.0)
        .bg(HOV().mul_a(hv))
        .scale(1.0 - 0.03 * pr)
        .on_click(key)
        .cursor(Cursor::Hand)
        .child(El::icon("dplus", 14.0, 1.6, ICO_ON()).no_hit())
        .child(El::text(label, btn_font(fs, 600), ICO_ON(), lh(fs, 1.35)).none().no_hit())
}

/// The game / app tile: 18 px (20 with `big`), the app's 135° gradient `a` -> `b` and its white glyph (11 px, 1.7) - in a
/// picker button `.pu.app` (inside its `.ats` slot) and in menu rows (`.mitem .at{margin-right:2px}` - the caller's).
///
/// `.at{width:18px;height:18px;flex:none;border-radius:5px;display:grid;place-items:center;box-shadow:inset 0 0 0 .5px
///   rgba(255,255,255,.24),inset 0 1px 0 rgba(255,255,255,.16)}` `.pu .at svg,.mitem .at svg{width:11px;height:11px;stroke:#fff;
///   stroke-width:1.7}` `.mitem .at.big{width:20px;height:20px}`
pub fn at(glyph: &str, a: Rgba, b: Rgba, big: bool) -> El {
    let s = if big { 20.0 } else { 18.0 };
    El::block()
        .size(s, s)
        .none()
        .radius(5.0)
        .bg_linear(135.0, &[(0.0, a), (1.0, b)])
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.24)), sh(0.0, 1.0, 0.0, 0.0, Rgba(1.0, 1.0, 1.0, 0.16))])
        .place_center()
        .child(El::icon(glyph, 11.0, 1.7, WHITE).no_hit())
}

/// A text link that takes its PARENT's weight (`#sw button{font:inherit}`, `#sw .lnk` sets only size / line-height): e.g.
/// Mouse's "Your mouse" web settings link in a 600 line. Otherwise Lane K's `link::link` (weight 400).
/// `#sw .lnk{color:var(--acc);font-size:12px;line-height:16px}` `.lnk:hover{text-decoration:underline;text-underline-offset:2px}`
pub fn lnk(cx: &mut Cx, key: Key, label: &str, size: f32, weight: u16, line_h: f32) -> El {
    let hv = cx.hovered(key);
    El::text(label, Font::new(size, weight).ls(0), ACC(), line_h).align(crate::gfx::Align::Center).underline(hv).none().on_click(key).cursor(Cursor::Hand)
}

/// The group header's grey run after its title (`.gh` gap 6): "while an app is running", a count, "all drives".
/// `.ghs{font-weight:400;color:var(--fg3)}` (the `.gh`'s 11 px). Add it as a child of `group::gh(title)`.
pub fn ghs(text: &str) -> El {
    El::text(text, Font::new(11.0, 400), FG3(), lh(11.0, 1.35))
}
