//! The segmented switch (menu-v22 `.seg`): equal segments, or `.seg.fit` (each as wide as its word); the white pill
//! glides to the chosen one.

use taffy::prelude::*;

use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{idx, lh, Cursor, El, Key};
use crate::ui::{CTL, FG, PILL};

use super::btn_font;

const GLIDE: Bezier = Bezier::new(0.3, 0.7, 0.2, 1.0);

/// `.seg{display:grid;grid-auto-flow:column;grid-auto-columns:1fr;padding:2px;border-radius:8px;background:var(--ctl)}`
/// `.seg .pill{top:2px;bottom:2px;left:2px;width:calc((100% - 4px) / n);border-radius:6px;background:var(--pill);
///   box-shadow:0 1px 3px rgba(0,0,0,.2),inset 0 0 0 .5px rgba(255,255,255,.14);transition:transform .26s cubic-bezier(.3,.7,.2,1)}`
/// `.seg button{height:24px;padding:0 11px;font-size:12px;opacity:.78;transition:opacity .15s,transform .12s ease}`
/// `:hover{opacity:.95}` `:active{transform:scale(.96)}` `.on{opacity:1;font-weight:600}` - but `#sw button{font:inherit}` (an id
/// rule) wins over the font-size and the weight: the drawing's segments are 13 px, weight 400 (Chromium's computed style).
/// `fit` = `.seg.fit` (`grid-auto-columns:auto`; the pill takes the chosen segment's width). Segment i = `Ev::Click(idx(key, i))`.
pub fn seg(cx: &mut Cx, key: Key, labels: &[&str], on: usize, fit: bool) -> El {
    let n = labels.len().max(1);
    // segment widths: text + 22 (padding); equal = the widest
    let ws: Vec<f32> = labels
        .iter()
        .map(|l| cx.g.text_width(l, btn_font(13.0, 400)) + 22.0)
        .collect();
    let eq = ws.iter().cloned().fold(0.0, f32::max);
    // .seg.fit: the drawing sets --x = offsetLeft - 2 and --w = offsetWidth (whole pixels, like every offset* value)
    let x_of = |i: usize| -> f32 { if fit { (2.0 + ws[..i].iter().sum::<f32>()).round() - 2.0 } else { eq * i as f32 } };
    let px = cx.tr(key, 1, x_of(on), 260.0, GLIDE);
    let pw = cx.tr(key, 2, if fit { ws[on.min(n - 1)].round() } else { eq }, 260.0, GLIDE);
    let pill = El::block()
        .abs(2.0, 2.0, f32::NAN, 2.0)
        .w(pw)
        .radius(6.0)
        .bg(PILL())
        .shadow(&[sh(0.0, 1.0, 3.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.2))])
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.14))])
        .translate(px, 0.0)
        .no_hit();
    let mut g = El::grid().pad_all(2.0).radius(8.0).bg(CTL()).none().child(pill).style(|s| {
        s.grid_auto_flow = GridAutoFlow::Column;
        s.grid_auto_columns = vec![if fit { auto() } else { fr(1.0) }];
    });
    for (i, l) in labels.iter().enumerate() {
        let k = idx(key, i);
        let hv = cx.hover_t(k, 150.0, EASE);
        let pr = cx.active_t(k, 120.0, EASE);
        let op = if i == on { 1.0 } else { 0.78 + (0.95 - 0.78) * hv };
        let b = El::row()
            .center()
            .justify(JustifyContent::CENTER)
            .h(24.0)
            .pad(0.0, 11.0, 0.0, 11.0)
            .opacity(op)
            .scale(1.0 - 0.04 * pr)
            .on_click(k)
            .cursor(Cursor::Hand)
            .child(El::text(*l, btn_font(13.0, 400), FG(), lh(13.0, 1.35)));
        g = g.child(b);
    }
    g
}

/// Order 097 (pack-noise-v1 `.seg{justify-content:space-between}` in a fixed column): a `.seg.fit` that is exactly `w` wide - every
/// segment is its word's width plus an equal share of what is left over (or minus an equal share of what is missing), the pill
/// glides to the chosen one. Segment i = `Ev::Click(idx(key, i))`.
pub fn seg_w(cx: &mut Cx, key: Key, labels: &[&str], on: usize, w: f32) -> El {
    let n = labels.len().max(1);
    let words: Vec<f32> = labels.iter().map(|l| cx.g.text_width(l, btn_font(13.0, 400)) + 22.0).collect();
    let share = (w - 4.0 - words.iter().sum::<f32>()) / n as f32;
    let ws: Vec<f32> = words.iter().map(|x| (x + share).max(24.0)).collect();
    let on = on.min(n - 1);
    let px = cx.tr(key, 1, ws[..on].iter().sum::<f32>(), 260.0, GLIDE);
    let pw = cx.tr(key, 2, ws[on], 260.0, GLIDE);
    let pill = El::block()
        .abs(2.0, 2.0, f32::NAN, 2.0)
        .w(pw)
        .radius(6.0)
        .bg(PILL())
        .shadow(&[sh(0.0, 1.0, 3.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.2))])
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.14))])
        .translate(px, 0.0)
        .no_hit();
    let tracks: Vec<_> = ws.iter().map(|x| length(*x)).collect();
    let mut g = El::grid().pad_all(2.0).radius(8.0).bg(CTL()).none().w(w).child(pill).style(move |s| {
        s.grid_auto_flow = GridAutoFlow::Column;
        s.grid_auto_columns = tracks.clone();
    });
    for (i, l) in labels.iter().enumerate() {
        let k = idx(key, i);
        let hv = cx.hover_t(k, 150.0, EASE);
        let pr = cx.active_t(k, 120.0, EASE);
        let op = if i == on { 1.0 } else { 0.78 + (0.95 - 0.78) * hv };
        let b = El::row()
            .center()
            .justify(JustifyContent::CENTER)
            .h(24.0)
            .opacity(op)
            .scale(1.0 - 0.04 * pr)
            .on_click(k)
            .cursor(Cursor::Hand)
            .child(El::text(*l, btn_font(13.0, 400), FG(), lh(13.0, 1.35)));
        g = g.child(b);
    }
    g
}
