//! The segmented switch's other sizes (menu-v22, Order 025) - Lane K's `seg::seg` is the base `.seg` (24 px, 13 px); these
//! are the drawing's variants: `.seg.sm` (Activity's "Most used" range), `.prw .seg` (Controller panel rows), `.seg.acct`
//! (Mouse acceleration's Input / Output / Both) and Display's monitor selector `.seg.monseg` ("1 · DELL 27″").
//!
//! Cascade note: `#sw button{font:inherit}` beats every `.seg… button` font rule, so the segments take their PARENT's font
//! (`SegOpts::font`): the 11.5 / 11 px in the `.sm` / `.acct` CSS never apply; `.monseg`'s letter-spacing does.

use taffy::prelude::*;

use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{idx, lh, Cursor, El, Key};
use crate::ui::{CTL, FG, FG2, FG3, PILL};

const GLIDE: Bezier = Bezier::new(0.3, 0.7, 0.2, 1.0);

/// One variant's numbers.
#[derive(Clone, Copy, Debug)]
pub struct SegOpts {
    /// the container's padding (= the pill's inset) and radius, the pill's radius
    pub pad: f32,
    pub radius: f32,
    pub pill_r: f32,
    /// a segment's height and horizontal padding
    pub h: f32,
    pub px: f32,
    /// the parent's font (with the button's letter-spacing: normal = 0, or `.monseg`'s -.005em)
    pub font: Font,
    /// `.seg.fit` - each segment as wide as its words (else equal columns)
    pub fit: bool,
    /// `.seg.acct{flex:1;min-width:0}` - fills its row
    pub grow: bool,
    /// cSeg's `--sw` min-width (0 = none)
    pub min_w: f32,
    /// the words' colour = the parent's (`#sw button{color:inherit}`): `FG2` in a group header's `.ghr`, else `FG`
    pub color: fn() -> Rgba,
}

/// `.seg.sm button{height:20px;padding:0 9px}` in Activity's header (`.gh .ghr`: 11 px, weight 400, fg2), fit.
pub const SM: SegOpts = SegOpts { pad: 2.0, radius: 8.0, pill_r: 6.0, h: 20.0, px: 9.0, font: Font::new(11.0, 400).ls(0), fit: true, grow: false, min_w: 0.0, color: FG2 };
/// A plain `.seg.fit` (24 px, padding 0 11px) inside a group header's `.ghr` (11 px, weight 400, fg2): Storage's "What's
/// using" [File types | Folders] (`h('span',{class:'ghr'},[useSeg])`, `cSeg(.., 0, true)` - not `.sm`).
pub const GHR: SegOpts = SegOpts { pad: 2.0, radius: 8.0, pill_r: 6.0, h: 24.0, px: 11.0, font: Font::new(11.0, 400).ls(0), fit: true, grow: false, min_w: 0.0, color: FG2 };
/// The base `.seg` (24 px, padding 0 11px, equal columns) in a 13 px row - Lane K's `seg::seg` numbers, for the variants that
/// piece has no option for: Mouse's DPI chips with nothing picked (`seg_ex(.., None, &BASE)` = `.seg.nopick`).
pub const BASE: SegOpts = SegOpts { pad: 2.0, radius: 8.0, pill_r: 6.0, h: 24.0, px: 11.0, font: Font::new(13.0, 400).ls(0), fit: false, grow: false, min_w: 0.0, color: FG };
/// `.prw .seg button{padding:0 9px}` (Controller panel rows, `.seg.fit`; the panel's 13 px).
pub const PRW: SegOpts = SegOpts { pad: 2.0, radius: 8.0, pill_r: 6.0, h: 24.0, px: 9.0, font: Font::new(13.0, 400).ls(0), fit: true, grow: false, min_w: 0.0, color: FG };
/// `.seg.acct{flex:1;min-width:0}` `.seg.acct button{height:20px;padding:0 4px}` (Mouse acceleration; equal columns).
pub const ACCT: SegOpts = SegOpts { pad: 2.0, radius: 8.0, pill_r: 6.0, h: 20.0, px: 4.0, font: Font::new(13.0, 400).ls(0), fit: false, grow: true, min_w: 0.0, color: FG };
/// `.seg.monseg{padding:3px;border-radius:10px}` `.pill{top:3px;bottom:3px;left:3px;border-radius:7px}`
/// `.seg.monseg button{height:30px;padding:0 16px;letter-spacing:-.005em}` (13 px; equal columns; its tabular-nums loses
/// to `font:inherit`).
pub const MONSEG: SegOpts = SegOpts { pad: 3.0, radius: 10.0, pill_r: 7.0, h: 30.0, px: 16.0, font: Font::new(13.0, 400).ls(-65), fit: false, grow: false, min_w: 0.0, color: FG };

/// A segment's words: plain, or Display's monitor label `"1" + <span>·</span> + "DELL 27″"` (`.monseg button span{color:var(--fg2);
/// margin:0 5px}`, `.on span{color:var(--fg3)}`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Label<'a> {
    Text(&'a str),
    Mon(&'a str, &'a str),
}

fn label_w(cx: &Cx, l: &Label, f: Font) -> f32 {
    match l {
        Label::Text(t) => cx.g.text_width(t, f),
        Label::Mon(n, name) => cx.g.text_width(n, f) + 5.0 + cx.g.text_width("\u{b7}", f) + 5.0 + cx.g.text_width(name, f),
    }
}

/// The segmented switch with a variant's numbers (`SM`, `PRW`, `ACCT`, `MONSEG`). Segment i = `Ev::Click(idx(key, i))`;
/// `on = None` = `.seg.nopick` (no segment picked: the pill hidden - Mouse's DPI chips when a custom DPI is typed).
/// States as `seg::seg`: rest .78, hover .95, the chosen one 1; the pill glides .26 s; pressed .96.
pub fn seg_ex(cx: &mut Cx, key: Key, labels: &[Label], on: Option<usize>, o: &SegOpts) -> El {
    let n = labels.len().max(1);
    let f = o.font;
    let ws: Vec<f32> = labels.iter().map(|l| label_w(cx, l, f) + 2.0 * o.px).collect();
    let sel = on.unwrap_or(0).min(n - 1);
    let pill = || {
        El::block()
            .radius(o.pill_r)
            .bg(PILL())
            .shadow(&[sh(0.0, 1.0, 3.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.2))])
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.14))])
            .no_hit()
    };
    let mut g = El::grid().pad_all(o.pad).radius(o.radius).bg(CTL());
    // the pill's tweens run while nothing is picked too (at segment 0, the drawing's `--i` / `--x` then): a pick after
    // `.nopick` glides from there; `.seg.nopick .pill{opacity:0}` hides it at once (no opacity transition)
    if o.fit {
        // .seg.fit: --x = offsetLeft - pad, --w = offsetWidth (whole pixels, like every offset* value)
        let x_of = |i: usize| -> f32 { (o.pad + ws[..i].iter().sum::<f32>()).round() - o.pad };
        let px = cx.tr(key, 1, x_of(sel), 260.0, GLIDE);
        let pw = cx.tr(key, 2, ws[sel].round(), 260.0, GLIDE);
        if on.is_some() {
            g = g.child(pill().abs(o.pad, o.pad, f32::NAN, o.pad).w(pw).translate(px, 0.0));
        }
    } else {
        // equal columns: `width:calc((100% - 2 x pad) / n)` (a flex share of the inner width, so it also fills a `.seg.acct`
        // stretched across its row) + `translateX(100% * i)`: snapped at the first column, then moved by i widths (Blink's
        // transform: a 1/3 px width puts the moved pill's edges on fractional pixels); the index glides
        let fi = cx.tr(key, 1, sel as f32, 260.0, GLIDE);
        if on.is_some() {
            let strip = El::row()
                .abs(o.pad, o.pad, o.pad, o.pad)
                .no_hit()
                .child(pill().grow(1.0).shrink(0.0).translate_pct(fi, 0.0))
                .child(El::block().grow((n - 1) as f32).shrink(0.0));
            g = g.child(strip);
        }
    }
    let (fit, grow, mw) = (o.fit, o.grow, o.min_w);
    g = g.style(move |s| {
        s.grid_auto_flow = GridAutoFlow::Column;
        s.grid_auto_columns = vec![if fit { auto() } else { fr(1.0) }];
        if mw > 0.0 {
            s.min_size.width = length(mw);
        }
    });
    g = if grow { g.flex1().min_w(0.0) } else { g.none() };
    let lhv = lh(f.size(), 1.35);
    for (i, l) in labels.iter().enumerate() {
        let k = idx(key, i);
        let hv = cx.hover_t(k, 150.0, EASE);
        let pr = if cx.rm { 0.0 } else { cx.active_t(k, 120.0, EASE) };
        let chosen = on == Some(i);
        let op = if chosen { 1.0 } else { 0.78 + (0.95 - 0.78) * hv };
        let mut b = El::row().center().justify(JustifyContent::CENTER).h(o.h).pad(0.0, o.px, 0.0, o.px).opacity(op).scale(1.0 - 0.04 * pr).on_click(k).cursor(Cursor::Hand);
        match l {
            Label::Text(t) => b = b.child(El::text(*t, f, (o.color)(), lhv).none()),
            Label::Mon(num, name) => {
                let dot = cx.tr(k, 7, if chosen { 1.0 } else { 0.0 }, 150.0, EASE);
                b = b
                    .child(El::text(*num, f, (o.color)(), lhv).none())
                    .child(El::text("\u{b7}", f, crate::ui::cmix(FG2(), FG3(), dot), lhv).none().margin(0.0, 5.0, 0.0, 5.0))
                    .child(El::text(*name, f, (o.color)(), lhv).none());
            }
        }
        g = g.child(b);
    }
    g
}
