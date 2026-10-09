//! The popup menu's richer rows (menu-v22, Order 025): icon items `.mitem.cxi` (+ `.mic`), the header line `.mhead`, the
//! separator `.msep`, the red danger item `.mitem.danger`, disabled `.dis`, the submenu chevron `.msub`, right-side notes
//! (`em`, `.mr`), leading tiles (`.gt` game tag, `.mi2` icon), section headings `.pmh`, the keyboard-chosen row `.mitem.kb`
//! - and the drawing's two placements: under a button (`placeMenu`) and right at the mouse (`menuAt`, the right-click menus
//! of Startup, Performance, Apps, Search, Screenshots, Security).
//!
//! The page keeps which menu is open and returns `mitems::menu(..)` from `Page::popup`. A click on row i = `Ev::Click(idx(key, i))`
//! (i = the row's index in the slice, headers and separators count too).

use taffy::style::AlignItems;

use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Align, Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{idx, lh, El, Key};
use crate::ui::{ACC, CTL, FG, FG3, HAIR, HL_V19, HOV, POP, RED, WHITE, WIN_H, WIN_W};

/// What sits before the label.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lead<'a> {
    /// `.mitem .gt{width:16px;height:16px;border-radius:4px;color:#fff;font:700 7px/1}` - a game tag with its colour
    Gt(&'a str, Rgba),
    /// `.mitem .mi2{width:16px;display:grid;place-items:center}` `svg{width:15px;height:15px;stroke-width:1.5}` - an icon
    Mi2(&'a str),
}

/// What sits after the label.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Right<'a> {
    /// `.mitem em{font-style:normal;color:var(--fg3);margin-left:6px}` (`:hover em{color:rgba(255,255,255,.75)}`) - DNS
    Em(&'a str),
    /// `.mitem .mr{margin-left:auto;padding-left:16px;font-size:11.5px;color:var(--fg3)}` (`:hover .mr` .8 white) - Controller
    Mr(&'a str),
    /// `.mitem .msub{margin-left:auto;padding-left:18px}` + `ICON.chevR` 6 x 10, stroke 1.5 - "Set priority ›"
    Sub,
}

/// One clickable row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct It<'a> {
    pub label: &'a str,
    /// `Some(checked)` = the radio lists' `.ck` tick column (14 px); None = no column
    pub tick: Option<bool>,
    /// an icon item `.mitem.cxi` (`.mic` 16 px box, 15 px icon) - the right-click menus
    pub icon: Option<&'a str>,
    pub lead: Option<Lead<'a>>,
    pub right: Option<Right<'a>>,
    /// `.mitem.danger` - red text, red row on hover (Uninstall, Delete)
    pub danger: bool,
    /// `.mitem.dis` - opacity .38, fg3, no clicks
    pub disabled: bool,
    /// `.mitem.kb` - the row the arrow keys chose (hov background)
    pub kb: bool,
}

impl<'a> It<'a> {
    /// A right-click menu item: icon + text.
    pub const fn icon(icon: &'a str, label: &'a str) -> It<'a> {
        It { label, tick: None, icon: Some(icon), lead: None, right: None, danger: false, disabled: false, kb: false }
    }
    /// A radio-list item: the tick column + text.
    pub const fn tick(label: &'a str, checked: bool) -> It<'a> {
        It { label, tick: Some(checked), icon: None, lead: None, right: None, danger: false, disabled: false, kb: false }
    }
    pub const fn danger(mut self) -> It<'a> {
        self.danger = true;
        self
    }
    pub const fn disabled(mut self, d: bool) -> It<'a> {
        self.disabled = d;
        self
    }
    pub const fn right(mut self, r: Right<'a>) -> It<'a> {
        self.right = Some(r);
        self
    }
    pub const fn lead(mut self, l: Lead<'a>) -> It<'a> {
        self.lead = Some(l);
        self
    }
    pub const fn kb(mut self, k: bool) -> It<'a> {
        self.kb = k;
        self
    }
}

/// One row of a menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Row<'a> {
    /// `.mhead{max-width:300px;margin:0 2px 4px;padding:3px 6px 7px;border-bottom:1px solid var(--hair);font-size:11px;
    /// line-height:14px;color:var(--fg3);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}`
    Head(&'a str),
    /// Order 045: a `.mhead` that also has the drawing's plain `title` hover name (`h('div',{class:'mhead',text:a.path,
    /// title:a.path})`): (text, title)
    HeadTitled(&'a str, &'a str),
    /// `.msep{height:1px;margin:4px 6px;background:var(--hair)}`
    Sep,
    /// `.pmh{padding:7px 8px 3px;font-size:10.5px;font-weight:600;color:var(--fg3);letter-spacing:.02em}` (`:first-child`
    /// padding-top 2px)
    Section(&'a str),
    Item(It<'a>),
}

const F13: Font = Font::new(13.0, 400);

fn row_h(r: &Row, first: bool) -> f32 {
    match r {
        Row::Head(_) | Row::HeadTitled(..) => 3.0 + 14.0 + 7.0 + 1.0 + 4.0,
        Row::Sep => 1.0 + 8.0,
        Row::Section(_) => (if first { 2.0 } else { 7.0 }) + lh(10.5, 1.35) + 3.0,
        Row::Item(_) => 26.0,
    }
}

/// The rows' boxes. Row i reacts as `idx(key, i)`.
pub fn rows(cx: &mut Cx, key: Key, rows: &[Row]) -> Vec<El> {
    let mut out = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let k = idx(key, i);
        out.push(match r {
            Row::Head(t) => El::block()
                .max_w(300.0)
                .margin(0.0, 2.0, 4.0, 2.0)
                // padding 3 6 7 + the 1 px bottom border (drawn as the last px of the box)
                .pad(3.0, 6.0, 8.0, 6.0)
                .child(El::text(*t, Font::new(11.0, 400), FG3(), 14.0).ellipsis())
                .child(El::block().abs(0.0, f32::NAN, 0.0, 0.0).h(1.0).bg(HAIR()).no_hit()),
            // the same head, keyed (row i) so its hover name shows
            Row::HeadTitled(t, title) => El::block()
                .max_w(300.0)
                .margin(0.0, 2.0, 4.0, 2.0)
                .pad(3.0, 6.0, 8.0, 6.0)
                .key(k)
                .title(title)
                .child(El::text(*t, Font::new(11.0, 400), FG3(), 14.0).ellipsis())
                .child(El::block().abs(0.0, f32::NAN, 0.0, 0.0).h(1.0).bg(HAIR()).no_hit()),
            Row::Sep => El::block().h(1.0).margin(4.0, 6.0, 4.0, 6.0).bg(HAIR()),
            Row::Section(t) => El::block()
                .pad(if i == 0 { 2.0 } else { 7.0 }, 8.0, 3.0, 8.0)
                .child(El::text(*t, Font::new(10.5, 600).ls(210), FG3(), lh(10.5, 1.35))),
            Row::Item(it) => item(cx, k, it),
        });
    }
    out
}

fn item(cx: &mut Cx, k: Key, it: &It) -> El {
    let hv = !it.disabled && cx.hovered(k);
    // `.mitem` has no transition: the hover colours are instant
    let base = if it.disabled { FG3() } else if it.danger { RED() } else { FG() };
    let col = if hv { WHITE } else { base };
    let cxi = it.icon.is_some();
    let mut r = El::row().center().gap(if cxi { 9.0 } else { 6.0 }).h(26.0).pad(0.0, if cxi { 16.0 } else { 14.0 }, 0.0, if cxi { 8.0 } else { 6.0 }).radius(5.0);
    if hv {
        r = r.bg(if it.danger { RED() } else { ACC() });
    } else if it.kb {
        r = r.bg(HOV());
    }
    if let Some(c) = it.tick {
        r = r.child(El::text(if c { "\u{2713}" } else { "" }, Font::new(12.0, 400), col, lh(12.0, 1.35)).w(14.0).none().align(Align::Center));
    }
    if let Some(ic) = it.icon {
        // .mitem .mic{width:16px;height:16px;display:grid;place-items:center} svg 15 px, stroke currentColor 1.5
        r = r.child(El::block().size(16.0, 16.0).none().place_center().no_hit().child(El::icon(ic, 15.0, 1.5, col)));
    }
    match it.lead {
        Some(Lead::Gt(t, bg)) => {
            // `.mitem.dis .gt{background:var(--ctl)!important;color:var(--fg3)}`
            let (b, c) = if it.disabled { (CTL(), FG3()) } else { (bg, WHITE) };
            r = r.child(El::block().size(16.0, 16.0).none().radius(4.0).bg(b).place_center().no_hit().child(El::text(t, Font::new(7.0, 700), c, 7.0)));
        }
        Some(Lead::Mi2(ic)) => {
            r = r.child(El::block().size(16.0, 16.0).none().place_center().no_hit().child(El::icon(ic, 15.0, 1.5, col)));
        }
        None => {}
    }
    r = r.child(El::text(it.label, F13, col, lh(13.0, 1.35)).none());
    match it.right {
        Some(Right::Em(t)) => {
            let c = if hv { Rgba(1.0, 1.0, 1.0, 0.75) } else { FG3() };
            r = r.child(El::text(t, F13, c, lh(13.0, 1.35)).none().margin(0.0, 0.0, 0.0, 6.0));
        }
        Some(Right::Mr(t)) => {
            let c = if hv { Rgba(1.0, 1.0, 1.0, 0.8) } else { FG3() };
            r = r.child(El::text(t, Font::new(11.5, 400), c, lh(11.5, 1.35)).none().ml_auto().pad(0.0, 0.0, 0.0, 16.0));
        }
        Some(Right::Sub) => {
            r = r.child(El::block().ml_auto().pad(0.0, 0.0, 0.0, 18.0).none().no_hit().child(El::icon_fit("chevR", 6.0, 10.0, 1.5, col)));
        }
        None => {}
    }
    if it.disabled {
        // `#sw .mitem.dis{opacity:.38;pointer-events:none}`
        r.key(k).opacity(0.38).no_hit()
    } else {
        r.on_click(k)
    }
}

/// Where the menu opens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    /// `placeMenu(btn, minW)`: under the button's box (window coordinates) + 4 px, at least as wide as the button; to the
    /// left (its right edge on the button's) / above (4 px) when it would leave the window (8 px margin)
    Under(f32, f32, f32, f32),
    /// `menuAt(el, x, y, minW)`: at the mouse + 2 px; to the left / above the point when it would leave the window; grows
    /// from the corner next to the point; kept 8 px inside
    At(f32, f32),
}

/// The drawing's open / close: `.menu{opacity:0;transform:scale(.97);transition:opacity .12s ease,transform .12s ease}`.
const OPEN: Bezier = EASE;

/// Where the menu box goes and which corner it grows from: (left, top, origin_right, origin_bottom).
pub fn place(p: Place, mw: f32, mh: f32) -> (f32, f32, bool, bool) {
    let r = |v: f32| (v + 0.5).floor();
    match p {
        Place::Under(bx, by, bw, bh) => {
            let (mut l, mut t) = (bx, by + bh + 4.0);
            if l + mw > WIN_W - 8.0 {
                l = (bx + bw - mw).max(8.0);
            }
            if t + mh > WIN_H - 8.0 {
                t = (by - mh - 4.0).max(8.0);
            }
            (r(l), r(t), false, false)
        }
        Place::At(x, y) => {
            let (mut l, mut t, mut orr, mut ob) = (x + 2.0, y + 2.0, false, false);
            if l + mw > WIN_W - 8.0 {
                l = x - mw - 2.0;
                orr = true;
            }
            if t + mh > WIN_H - 8.0 {
                t = y - mh - 2.0;
                ob = true;
            }
            // clamp(v, 8, max) of the drawing (v < a -> a, v > b -> b)
            let cl = |v: f32, b: f32| if v < 8.0 { 8.0 } else if v > b { b } else { v };
            (r(cl(l, WIN_W - mw - 8.0)), r(cl(t, WIN_H - mh - 8.0)), orr, ob)
        }
    }
}

/// The whole menu: `.menu` with `rows` at `p`, `min_w` = the drawing's minW (150 lists, 190-240 right-click menus).
///
/// `.menu{position:absolute;z-index:20;min-width:150px;max-height:300px;overflow-y:auto;padding:5px;border-radius:10px;
///   background:var(--pop);backdrop-filter:blur(30px) saturate(180%);box-shadow:inset 0 0 0 .5px var(--hl),
///   0 0 0 .5px rgba(0,0,0,.35),0 12px 32px rgba(0,0,0,.35)}` (+ the open transition from its origin corner).
pub fn menu(cx: &mut Cx, key: Key, list: &[Row], p: Place, min_w: f32) -> El {
    let kids = rows(cx, key, list);
    let (mw, mh, min_w) = menu_size(cx, list, p, min_w);
    menu_box(cx, key, kids, p, mw, mh, min_w)
}

/// [`menu`] for a LONG list (Order 059: the Keyboard tab's ready-made actions): the box keeps its 300 px and the rows scroll
/// inside it (wheel, slim glass thumb) instead of hanging out below. Row clicks are the same `Ev::Click(idx(key, i))`.
pub fn menu_scroll(cx: &mut Cx, key: Key, list: &[Row], p: Place, min_w: f32) -> El {
    let kids = rows(cx, key, list);
    let (mw, mh, min_w) = menu_size(cx, list, p, min_w);
    let body = cx
        .scroll_box(crate::ui::el::sub(key, "scroll"), kids)
        .items(AlignItems::STRETCH)
        .min_h(0.0)
        .max_h(290.0)
        .slim_thumb(crate::ui::el::SlimThumb::GLASS);
    menu_box(cx, key, vec![body], p, mw, mh, min_w)
}

/// The menu's measured size (Chromium measures offsetWidth / offsetHeight after filling it): the widest row's max-content + 10,
/// its rows' heights + 10 up to 300 - and the minimum width.
fn menu_size(cx: &mut Cx, list: &[Row], p: Place, min_w: f32) -> (f32, f32, f32) {
    let mut w: f32 = 0.0;
    for r in list {
        let rw = match r {
            Row::Head(t) | Row::HeadTitled(t, _) => (cx.g.text_box(t, Font::new(11.0, 400), 0.0).width + 16.0).min(304.0),
            Row::Sep => 0.0,
            Row::Section(t) => cx.g.text_box(t, Font::new(10.5, 600).ls(210), 0.0).width + 16.0,
            Row::Item(it) => item_w(cx, it),
        };
        w = w.max(rw);
    }
    let min_w = match p {
        Place::Under(_, _, bw, _) => min_w.max(bw),
        Place::At(..) => min_w,
    };
    let mw = (w + 10.0).max(min_w);
    let mh: f32 = (list.iter().enumerate().map(|(i, r)| row_h(r, i == 0)).sum::<f32>() + 10.0).min(300.0);
    (mw, mh, min_w)
}

/// The popup confirm `.mcf` (Startup's "This is part of Windows", Performance's "End … ?", Security's Offline scan / Allow /
/// Restore, Tweaks' Rebuild icon cache, Settings' "Reset the app's own settings?"): a short question INSIDE the menu box, opened
/// at a point (`Place::At`, minW 260) or under its button (`Place::Under`, minW 262). Buttons: `sub(key, "no")` (the plain one:
/// Cancel / Keep on) and `sub(key, "go")` (`go_kind` = `Kind::Red` for End task / Turn off / Settings' Reset, `Kind::Primary` for
/// Restart and scan / Allow / Restore / Rebuild). On open the page focuses `no` (Startup) or `go` (everywhere else), as the drawing does.
///
/// `.mcf{width:248px;padding:6px 7px 4px}` `.mcf b{display:block;font-size:13px;font-weight:600;line-height:17px;
///   white-space:nowrap;overflow:hidden;text-overflow:ellipsis}` `.mcf p{margin:4px 0 11px;font-size:11.5px;line-height:15px;
///   color:var(--fg2)}` `.mcfb{display:flex;justify-content:flex-end;gap:6px}` `#sw .mcfb .cbtn{height:26px;padding:0 13px;font-size:12px}`
#[allow(clippy::too_many_arguments)]
pub fn confirm(cx: &mut Cx, key: Key, title: &str, text: &str, no: &str, go: &str, go_kind: super::button::Kind, p: Place, min_w: f32) -> El {
    use super::button::{cbtn_sized, Kind, MCFB};
    use crate::ui::el::sub;
    let fp = Font::new(11.5, 400);
    let tf = crate::ui::el::Text { s: text.to_string(), font: fp, color: crate::ui::FG2(), lh: 15.0, wrap: crate::ui::el::Wrap::Wrap, align: Align::Left, underline: false };
    let lines = crate::ui::lay::wrap_lines(cx.g, &tf, 248.0 - 14.0).len().max(1) as f32;
    let body = El::col()
        .w(248.0)
        .pad(6.0, 7.0, 4.0, 7.0)
        .child(El::text(title, Font::new(13.0, 600), FG(), 17.0).ellipsis())
        .child(El::text(text, fp, crate::ui::FG2(), 15.0).wrapping().margin(4.0, 0.0, 11.0, 0.0))
        .child(
            El::row()
                .justify(taffy::style::JustifyContent::FLEX_END)
                .gap(6.0)
                .child(cbtn_sized(cx, sub(key, "no"), no, Kind::Ghost, MCFB, false, 0.0))
                .child(cbtn_sized(cx, sub(key, "go"), go, go_kind, MCFB, false, 0.0)),
        );
    let min_w = match p {
        Place::Under(_, _, bw, _) => min_w.max(bw),
        Place::At(..) => min_w,
    };
    let mw = (248.0f32 + 10.0).max(min_w);
    let mh = 10.0 + 6.0 + 17.0 + 4.0 + 15.0 * lines + 11.0 + 26.0 + 4.0;
    menu_box(cx, key, vec![body], p, mw, mh, min_w)
}

/// The `.menu` box with its content at `p` (size mw x mh for the placement), opening from its origin corner.
fn menu_box(cx: &mut Cx, key: Key, kids: Vec<El>, p: Place, mw: f32, mh: f32, min_w: f32) -> El {
    let (x, y, orr, ob) = place(p, mw, mh);
    let t = cx.tr(key, 1, 1.0, 120.0, OPEN);
    let s = 0.97 + 0.03 * t;
    // scale about the origin corner = scale about the centre + a shift
    let dx = (1.0 - s) * mw / 2.0 * if orr { 1.0 } else { -1.0 };
    let dy = (1.0 - s) * mh / 2.0 * if ob { 1.0 } else { -1.0 };
    El::col()
        .abs(x, y, f32::NAN, f32::NAN)
        .min_w(min_w)
        .max_h(300.0)
        .pad(5.0, 5.0, 5.0, 5.0)
        .radius(10.0)
        .bg(POP())
        .backdrop(30.0, 1.8)
        .shadow(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, 0.35)), sh(0.0, 12.0, 32.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.35))])
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, HL_V19())])
        .items(AlignItems::STRETCH)
        .opacity(t)
        .scale(s)
        .translate(dx, dy)
        .z(20)
        .key(key)
        .children(kids)
}

fn item_w(cx: &Cx, it: &It) -> f32 {
    let cxi = it.icon.is_some();
    let gap = if cxi { 9.0 } else { 6.0 };
    let mut parts: Vec<f32> = Vec::new();
    if it.tick.is_some() {
        parts.push(14.0);
    }
    if it.icon.is_some() || it.lead.is_some() {
        parts.push(16.0);
    }
    parts.push(cx.g.text_box(it.label, F13, 0.0).width);
    match it.right {
        Some(Right::Em(t)) => parts.push(cx.g.text_box(t, F13, 0.0).width),
        Some(Right::Mr(t)) => parts.push(16.0 + cx.g.text_box(t, Font::new(11.5, 400), 0.0).width),
        Some(Right::Sub) => parts.push(24.0),
        None => {}
    }
    let pad = if cxi { 24.0 } else { 20.0 };
    parts.iter().sum::<f32>() + gap * (parts.len() as f32 - 1.0) + pad
}

#[cfg(test)]
mod tests {
    use super::{place, Place};

    #[test]
    fn placement_follows_the_drawing() {
        // under a button, room below: left-aligned, 4 px under it
        assert_eq!(place(Place::Under(100.0, 100.0, 80.0, 24.0), 150.0, 100.0), (100.0, 128.0, false, false));
        // near the right edge: its right edge on the button's; near the bottom: above the button
        assert_eq!(place(Place::Under(500.0, 450.0, 80.0, 24.0), 150.0, 100.0), (430.0, 346.0, false, false));
        // at the mouse: +2 / +2, growing from the top-left
        assert_eq!(place(Place::At(200.0, 200.0), 214.0, 150.0), (202.0, 202.0, false, false));
        // at the mouse near the bottom-right: left of / above the point, growing from the bottom-right
        assert_eq!(place(Place::At(500.0, 480.0), 214.0, 150.0), (284.0, 328.0, true, true));
    }
}
