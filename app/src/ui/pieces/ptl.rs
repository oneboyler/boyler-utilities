//! The tiles (menu-v22 `.ptl`, Order 025): Performance's live tiles, Storage's drive tiles (`.ptl.dtl`, clickable, the
//! one you look at accent-ringed), Activity's three tiles (`.ptl.atl`), in the 6-column tile grid `.pcg`.
//! Not a backdrop glass: `background:var(--grp)` + a .5 px hairline (the `.grp` rim does not apply to tiles).

use taffy::style::AlignItems;

use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Font};
use crate::ui::cx::Cx;
use crate::ui::el::{lh, Cursor, El, Key};
use crate::ui::{cmix, ACC, AMBER, FG, FG2, FG3, GRP, HAIR, HOV, SEL, TRK};

/// The drive bar's width transition `cubic-bezier(.3,.7,.2,1)`.
const BAR_EASE: Bezier = Bezier::new(0.3, 0.7, 0.2, 1.0);

/// A tile's width in the 6-column grid: `.ptl{grid-column:span 2}` `.w` span 3, `.g4` span 4, `.f` the whole row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Span {
    Two,
    Three,
    Four,
    Full,
}

/// The tile grid: `.pcg{display:grid;grid-template-columns:repeat(6,minmax(0,1fr));gap:8px;margin-top:2px}`.
pub fn grid(tiles: Vec<El>) -> El {
    El::grid().cols(6).gap(8.0).margin(2.0, 0.0, 0.0, 0.0).children(tiles)
}

/// What a tile shows: the header line `.pch` (label, name, right part), the value line `.pcm` (big value + unit, extra),
/// then anything under it (a sparkline `El::paint`, the drive bar `dbar`, Activity's `.pcx` line).
pub struct Tile<'a> {
    /// `.pcl` - "CPU", "C:", "Screen time"
    pub label: &'a str,
    /// `.pcn` - "Ryzen 7 7800X3D" (ellipsis)
    pub name: Option<&'a str>,
    /// the right end of the header: `pcq(..)`, `tip::rq(cx, .., 16.0, ..)` (as `.pch>.rq`), `dtk(..)`
    pub right: Option<El>,
    /// `.pcv` + its `small` unit: ("1.2", Some("TB")); None = `.ptl.none` (the value line keeps 26 px)
    pub value: Option<(&'a str, Option<&'a str>)>,
    /// `.pcx` after the value: (text, bold part shown first) - e.g. ("free of 2 TB", None)
    pub extra: Option<El>,
    /// under the value line
    pub below: Vec<El>,
}

fn head(t: &mut Tile, in_button: bool) -> El {
    // .pch{display:flex;align-items:baseline;gap:8px;white-space:nowrap} - the two texts share one font size, so their
    // baselines line up at the top; the right part is `align-self:center`
    // .pcl{flex:none;font-size:11px;font-weight:600;color:var(--fg2);letter-spacing:.02em}
    // .pcn{min-width:0;overflow:hidden;text-overflow:ellipsis;font-size:11px;color:var(--fg3)}
    let mut h = El::row().gap(8.0).items(AlignItems::FLEX_START).child(El::text(t.label, Font::new(11.0, 600).ls(220), FG2(), lh(11.0, 1.35)).none());
    if let Some(n) = t.name {
        // in a drive tile (a <button>) the browser's `letter-spacing: normal`
        let f = Font::new(11.0, 400).ls(if in_button { 0 } else { -78 });
        h = h.child(El::text(n, f, FG3(), lh(11.0, 1.35)).ellipsis().shrink(1.0).min_w(0.0));
    }
    if let Some(r) = t.right.take() {
        h = h.child(r.ml_auto().style(|s| s.align_self = Some(AlignItems::CENTER)));
    }
    h
}

/// The value line `.pcm{display:flex;align-items:center;gap:8px;margin-top:3px;white-space:nowrap;min-width:0}`
/// with `.pcv{flex:none;font:600 20px/26px "Segoe UI Variable Display";letter-spacing:-.01em;tabular-nums}`
/// `.pcv small{font-size:13px;font-weight:600;color:var(--fg2);margin-left:2px}` (the unit sits on the value's baseline;
/// a line box holding both is as tall as Blink's max-ascent + max-descent rule makes it).
fn value_line(cx: &Cx, t: &mut Tile, mt: f32) -> El {
    let mut m = El::row().center().gap(8.0).margin(mt, 0.0, 0.0, 0.0).min_w(0.0);
    match t.value {
        None => m = m.min_h(26.0), // .ptl.none .pcv{display:none} .ptl.none .pcm{min-height:26px}
        Some((v, unit)) => m = m.child(pcv(cx, v, unit)),
    }
    if let Some(x) = t.extra.take() {
        m = m.child(x);
    }
    m
}

/// `.pcv` (+ `small`): the big tile value with its unit.
pub fn pcv(cx: &Cx, v: &str, unit: Option<&str>) -> El {
    let fv = Font::display(20.0, 600).ls(-200).tnum();
    let Some(u) = unit else {
        return El::text(v, fv, FG(), 26.0).none();
    };
    let fu = Font::display(13.0, 600).ls(-200).tnum();
    // both inline boxes are 26 px (line-height 26px is inherited by the small); their baselines meet
    let (bv, bu) = (cx.g.baseline(fv, 26.0), cx.g.baseline(fu, 26.0));
    let a = bv.max(bu);
    let hgt = a + (26.0 - bv).max(26.0 - bu);
    let vw = cx.g.text_box(v, fv, 0.0).width;
    El::block()
        .h(hgt)
        .none()
        .child(El::text(v, fv, FG(), 26.0).abs(0.0, a - bv, f32::NAN, f32::NAN))
        .child(El::text(u, fu, FG2(), 26.0).abs(vw + 2.0, a - bu, f32::NAN, f32::NAN))
        .w(vw + 2.0 + cx.g.text_box(u, fu, 0.0).width)
}

/// `.pcx{min-width:0;overflow:hidden;text-overflow:ellipsis;font-size:11.5px;color:var(--fg2);tabular-nums}`
/// `.pcx b{font-weight:600;color:var(--fg)}`: `parts` = the runs in order, `true` = a `<b>` run - e.g.
/// `&[("VRAM ", false), ("7.1", true), (" / 12 GB", false)]`. `in_button` = inside a drive tile (a `<button>`: the
/// browser's `letter-spacing: normal` instead of #sw's).
pub fn pcx(cx: &Cx, parts: &[(&str, bool)], in_button: bool) -> El {
    let ls = if in_button { 0 } else { -78 };
    let f = Font::new(11.5, 400).tnum().ls(ls);
    let fb = Font::new(11.5, 600).tnum().ls(ls);
    let l = lh(11.5, 1.35);
    if let [(t, false)] = parts {
        return El::text(*t, f, FG2(), l).ellipsis().shrink(1.0).min_w(0.0);
    }
    let mut r = El::row().shrink(1.0).min_w(0.0).clip();
    for (i, (t, b)) in parts.iter().enumerate() {
        let (font, c) = if *b { (fb, FG()) } else { (f, FG2()) };
        let e = El::text(*t, font, c, l);
        r = r.child(if i + 1 == parts.len() { e.ellipsis().shrink(1.0).min_w(0.0) } else { e.none().w(cx.g.text_box(t, font, 0.0).width) });
    }
    r
}

/// The small note at a tile header's right end: `.pcq{display:flex;align-items:center;gap:4px;font-size:10.5px;
/// line-height:14px;color:var(--fg3)}` `.pcq svg{width:11px;height:11px;stroke-width:1.4}`.
pub fn pcq(icon: &str, text: &str) -> El {
    El::row().center().gap(4.0).none().child(El::icon(icon, 11.0, 1.4, FG3()).no_hit()).child(El::text(text, Font::new(10.5, 400), FG3(), 14.0).none())
}

/// A drive tile's disk icon: `.dtk{margin-left:auto;display:grid;place-items:center;width:16px;height:16px;align-self:center;
/// color:var(--fg3)}` `.dtk svg{width:15px;height:15px;stroke-width:1.3}` (centred both ways).
pub fn dtk(icon: &str) -> El {
    El::block().size(16.0, 16.0).none().no_hit().place_center().child(El::icon(icon, 15.0, 1.3, FG3()))
}

/// Activity's line under the value: `.atl>.pcx{display:block;margin-top:1px;font-size:11px;color:var(--fg3);white-space:nowrap}`.
pub fn atl_line(text: &str) -> El {
    El::text(text, Font::new(11.0, 400).tnum(), FG3(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0)
}

/// Storage's used-space bar: `.dbar{display:block;height:5px;margin-top:9px;border-radius:3px;background:var(--trk);overflow:hidden}`
/// `.dbar i{background:var(--acc);transition:width .6s cubic-bezier(.3,.7,.2,1)}` `.dtl.dlow .dbar i{background:var(--amber)}`.
pub fn dbar(cx: &mut Cx, key: Key, used: f32, low: bool) -> El {
    let k = crate::ui::el::sub(key, "dbar");
    // the drawing's `(used/tot*100).toFixed(1)+'%'`: the width in tenths of a percent
    let share = cx.tr(k, 1, (used.clamp(0.0, 1.0) * 1000.0).round() / 1000.0, 600.0, BAR_EASE);
    El::block()
        .h(5.0)
        .margin(9.0, 0.0, 0.0, 0.0)
        .radius(3.0)
        .bg(TRK())
        .clip()
        .no_hit()
        .child(El::block().abs(0.0, 0.0, f32::NAN, 0.0).w_pct(share * 100.0).radius(3.0).bg(if low { AMBER() } else { ACC() }))
}

fn span_style(e: El, sp: Span) -> El {
    e.style(move |s| {
        use taffy::prelude::{line, GridPlacement, Line};
        s.grid_column = match sp {
            Span::Two => Line { start: GridPlacement::Span(2), end: GridPlacement::Auto },
            Span::Three => Line { start: GridPlacement::Span(3), end: GridPlacement::Auto },
            Span::Four => Line { start: GridPlacement::Span(4), end: GridPlacement::Auto },
            Span::Full => Line { start: line(1), end: line(-1) },
        };
    })
}

/// A static tile (Performance, Activity `atl = true`).
///
/// `.ptl{grid-column:span 2;position:relative;min-width:0;padding:10px 12px 8px;border-radius:10px;background:var(--grp);
///   box-shadow:inset 0 0 0 .5px var(--hair);overflow:hidden}` `.atl .pcm{margin-top:2px}`.
pub fn ptl(cx: &Cx, span: Span, mut t: Tile, atl: bool) -> El {
    let h = head(&mut t, false);
    let m = value_line(cx, &mut t, if atl { 2.0 } else { 3.0 });
    let e = El::block().min_w(0.0).pad(10.0, 12.0, 8.0, 12.0).radius(10.0).bg(GRP()).inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())]).clip().child(h).child(m).children(t.below);
    span_style(e, span)
}

/// A drive tile (Storage): a button. `#sw .dtl{display:block;width:100%;text-align:left;cursor:pointer;padding-bottom:11px;
///   transition:background-color .15s ease,box-shadow .2s ease,transform .12s ease}` `:hover{background:var(--hov)}`
/// `.dtl:active{transform:scale(.985)}` `#sw .dtl.on{box-shadow:inset 0 0 0 1.5px var(--acc);background:var(--sel)}`
/// (`.on` stays on hover: same specificity, later). `on` = the drive you look at. Click = `Ev::Click(key)`.
pub fn dtl(cx: &mut Cx, key: Key, span: Span, mut t: Tile, on: bool) -> El {
    let hv = cx.hover_t(key, 150.0, EASE);
    let pr = cx.active_t(key, 120.0, EASE);
    let onv = cx.tr(key, 3, if on { 1.0 } else { 0.0 }, 150.0, EASE);
    let ring = cx.tr(key, 4, if on { 1.0 } else { 0.0 }, 200.0, EASE);
    let h = head(&mut t, true);
    let m = value_line(cx, &mut t, 3.0);
    let bg = cmix(cmix(GRP(), HOV(), hv), SEL(), onv);
    let mut insets = Vec::new();
    if ring < 0.999 {
        insets.push(sh(0.0, 0.0, 0.0, 0.5, HAIR().mul_a(1.0 - ring)));
    }
    if ring > 0.001 {
        insets.push(sh(0.0, 0.0, 0.0, 1.5, ACC().mul_a(ring)));
    }
    let e = El::block()
        .min_w(0.0)
        .w_pct(100.0)
        .pad(10.0, 12.0, 11.0, 12.0)
        .radius(10.0)
        .bg(bg)
        .inset(&insets)
        .clip()
        .scale(1.0 - 0.015 * if cx.rm { 0.0 } else { pr })
        .on_click(key)
        .cursor(Cursor::Hand)
        .child(h)
        .child(m)
        .children(t.below);
    span_style(e, span)
}
