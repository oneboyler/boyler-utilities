//! Group header, group box ("bubble"), group footer and the row (menu-v22 `.gh`, `.grp`, `.gf`, `.row`, `.lbl`, `.ctl`).

use crate::gfx::{sh, Font};
use crate::settings::GlassStyle;
use crate::ui::el::{lh, El};
use crate::ui::{FG, FG2, FG3, GRP, GRP_RIM, GRP_TOP, HAIR};

/// `.gh{display:flex;align-items:center;gap:6px;font-size:11px;font-weight:500;color:var(--fg2);margin:20px 12px 7px}`
pub fn gh(title: &str) -> El {
    El::row().center().gap(6.0).margin(20.0, 12.0, 7.0, 12.0).child(El::text(title, Font::new(11.0, 500), FG2(), lh(11.0, 1.35)))
}

/// The group box: `.grp{background:var(--grp);border-radius:10px}` with the glass style's rim - Liquid (v19):
/// `inset 0 0 0 1px rgba(255,255,255,.18), inset 0 1px 0 rgba(255,255,255,.26)`; Frosted: `inset 0 0 0 .5px .13, inset 0 1px 0 .12`;
/// Windows look: `inset 0 0 0 .5px .1`. `rows` = its `.row`s (the first one without the hairline above it).
pub fn grp(rows: Vec<El>) -> El {
    let (bg, rim) = glass();
    El::block().bg(bg).radius(10.0).inset(&rim).children(rows)
}

/// The group box's fill and rim for the glass showing. Light (Order 033): `.grp{background:var(--grp);box-shadow:inset 0 0 0
/// .5px var(--hair)}` (the bright rims are `:not(.light)`).
pub fn glass() -> (crate::gfx::Rgba, Vec<crate::gfx::Shadow>) {
    if crate::ui::is_light() {
        return (GRP(), vec![sh(0.0, 0.0, 0.0, 0.5, HAIR())]);
    }
    let w = |a: f32| crate::gfx::Rgba(1.0, 1.0, 1.0, a);
    match crate::ui::glass() {
        GlassStyle::Liquid => (GRP(), vec![sh(0.0, 0.0, 0.0, 1.0, GRP_RIM()), sh(0.0, 1.0, 0.0, 0.0, GRP_TOP())]),
        GlassStyle::Frosted => (w(0.07), vec![sh(0.0, 0.0, 0.0, 0.5, w(0.13)), sh(0.0, 1.0, 0.0, 0.0, w(0.12))]),
        GlassStyle::WindowsLook => (w(0.06), vec![sh(0.0, 0.0, 0.0, 0.5, w(0.1))]),
    }
}

/// `.gf{font-size:11px;color:var(--fg3);margin:7px 12px 0;line-height:1.4}`
pub fn gf(text: &str) -> El {
    El::text(text, Font::new(11.0, 400), FG3(), lh(11.0, 1.4)).wrapping().margin(7.0, 12.0, 0.0, 12.0)
}

/// A row's label: `.lbl{flex:1;min-width:0;font-size:13px}` + optional `small{display:block;font-size:11px;color:var(--fg2);
/// margin-top:1px}`.
pub fn lbl(title: &str, small: Option<&str>) -> El {
    let mut l = El::col().flex1().child(El::text(title, Font::new(13.0, 400), FG(), lh(13.0, 1.35)).ellipsis());
    if let Some(s) = small {
        l = l.child(El::text(s, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0));
    }
    l
}

/// `.ctl{flex:none;display:flex;align-items:center;gap:8px}`
pub fn ctl(kids: Vec<El>) -> El {
    El::row().none().center().gap(8.0).children(kids)
}

/// A row: `.row{position:relative;display:flex;align-items:center;gap:12px;min-height:42px;padding:7px 12px}` with
/// `.row::before{left:12px;right:0;top:0;height:1px;background:var(--hair)}` (not on `.row.first`).
/// `kids` = usually [optional icon, lbl(...), ctl(...)].
pub fn row(first: bool, kids: Vec<El>) -> El {
    row_ex(first, 12.0, kids)
}

/// A row whose hairline starts at `line_left` (cards: 56).
pub fn row_ex(first: bool, line_left: f32, kids: Vec<El>) -> El {
    let mut r = El::row().center().gap(12.0).min_h(42.0).pad(7.0, 12.0, 7.0, 12.0);
    if !first {
        r = r.child(El::block().abs(line_left, 0.0, 0.0, f32::NAN).h(1.0).bg(HAIR()).no_hit());
    }
    r.children(kids)
}

/// A row's line icon (the Audio device rows' `.dvi`): 22 px, stroke --ico 1.5.
pub fn row_icon(name: &str) -> El {
    El::icon(name, 22.0, 1.5, crate::ui::ICO())
}

/// `.fixed{font-size:13px;color:var(--fg2)}` - a value shown as text in a row.
pub fn fixed(text: &str) -> El {
    El::text(text, Font::new(13.0, 400), FG2(), lh(13.0, 1.35))
}
