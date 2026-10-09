//! What the capture overlay shows on one monitor, as boxes in that monitor's DIPs (menu-v22 #cap CSS, numbers quoted on each
//! part). Two layers, like the drawing's stacking: `back` (the dim, the box with its edge + handles, the marks) and `front`
//! (the size tag + monitor buttons + Live, the toolbar, the emoji picker, the hint, ×, the crosshair, the text field, the
//! flash). The front's glass parts blur what the back drew (`backdrop-filter`); the window paints the back first, keeps
//! that picture, then the front (see `paint`).

use std::rc::Rc;

use skia_safe as sk;
use taffy::style::{AlignItems, JustifyContent};
use windows_numerics::Matrix3x2;

use super::emoji;
use super::ink;
use super::model::{Ann, Corner, Mode, Model, Pointer, Sel, Tool, COLORS};
use crate::anim::{self, Bezier, EASE, EASE_IN, EASE_OUT, EASE_OUT_CSS};
use crate::gfx::{sh, Align, CssColor, Font, Gfx, Rgba, Shadow};
use crate::icons::Icons;
use crate::ui::cx::Cx;
use crate::ui::el::{idx, key, sub, El, Key, RADIUS_PILL};
use crate::ui::lay::Laid;

// ------------------------------------------------------------------ keys
pub const K_CAP: Key = key("cap");
pub const K_SZ: Key = key("cap.sz");
pub const K_MON: Key = key("cap.mon");
pub const K_LIVE: Key = key("cap.live");
pub const K_SNAP: Key = key("cap.snap");
pub const K_X: Key = key("cap.x");
pub const K_TB: Key = key("cap.tb");
pub const K_TOOL: Key = key("cap.tool");
pub const K_DOT: Key = key("cap.dot");
pub const K_UNDO: Key = key("cap.undo");
pub const K_COPY: Key = key("cap.copy");
pub const K_SAVE: Key = key("cap.save");
pub const K_EMJ: Key = key("cap.emj");
pub const K_EQ: Key = key("cap.eq");
pub const K_MORE: Key = key("cap.more");
pub const K_BACK: Key = key("cap.back");
pub const K_SRCH: Key = key("cap.srch");
pub const K_ETAB: Key = key("cap.etab");
pub const K_EGRID: Key = key("cap.egrid");
pub const K_EB: Key = key("cap.eb");
pub const K_HD: Key = key("cap.hd");
pub const K_BAR: Key = key("cap.bar");

// ------------------------------------------------------------------ the drawing's colours
/// A CSS colour as Chromium keeps it: 8-bit channels, the alpha too (rgba(255,255,255,.88) = 224 / 255).
const fn c(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    Rgba::rgba(r, g, b, ((a * 255.0 + 0.5) as u32) as f32 / 255.0)
}
const WHITE: Rgba = c(255, 255, 255, 1.0);
/// #cap{--cdim:rgba(6,8,16,.2)}
const CDIM: Rgba = c(6, 8, 16, 0.2);
/// .csz,.cmb,.clv,.csnp{background:rgba(14,16,22,.78)} :hover rgba(34,38,50,.9)
const CHIP: Rgba = c(14, 16, 22, 0.78);
const CHIP_H: Rgba = c(34, 38, 50, 0.9);
const CHIP_FG: Rgba = c(255, 255, 255, 0.84);
/// #cap{--tbg:rgba(30,32,40,.74);--tfg:rgba(255,255,255,.86);--tfh:#fff;--thv:rgba(255,255,255,.1);--tsep:rgba(255,255,255,.14);
///   --tring:rgba(255,255,255,.92);--tfill:rgba(255,255,255,.08);--tin:rgba(0,0,0,.24)}
/// `#cap.light{--tbg:rgba(248,248,250,.8);--tfg:rgba(29,29,31,.78);--tfh:#1d1d1f;--thv:rgba(0,0,0,.06);--tsep:rgba(0,0,0,.12);
///   --tring:rgba(29,29,31,.8);--tfill:rgba(0,0,0,.05);--tin:rgba(0,0,0,.05)}` - the overlay takes the menu's theme (Order 033).
macro_rules! cap_tokens {
    ($($name:ident: $dark:expr, $light:expr;)*) => {$(
        #[allow(non_snake_case)]
        fn $name() -> Rgba {
            if crate::ui::is_light() {
                $light
            } else {
                $dark
            }
        }
    )*};
}
cap_tokens! {
    TBG: c(30, 32, 40, 0.74), c(248, 248, 250, 0.8);
    TFG: c(255, 255, 255, 0.86), c(29, 29, 31, 0.78);
    TFH: WHITE, c(29, 29, 31, 1.0);
    THV: c(255, 255, 255, 0.1), c(0, 0, 0, 0.06);
    TSEP: c(255, 255, 255, 0.14), c(0, 0, 0, 0.12);
    TRING: c(255, 255, 255, 0.92), c(29, 29, 31, 0.8);
    TFILL: c(255, 255, 255, 0.08), c(0, 0, 0, 0.05);
    TIN: c(0, 0, 0, 0.24), c(0, 0, 0, 0.05);
    // `.ctb,.cemj{box-shadow:inset 0 0 0 .5px rgba(255,255,255,.16),0 0 0 .5px rgba(0,0,0,.5),0 12px 32px rgba(0,0,0,.36)}`;
    // `#cap.light .ctb,#cap.light .cemj{box-shadow:inset 0 0 0 .5px rgba(255,255,255,.8),0 0 0 .5px rgba(0,0,0,.18),0 12px 32px rgba(0,0,0,.22)}`
    TRIM: c(255, 255, 255, 0.16), c(255, 255, 255, 0.8);
    TRING_O: c(0, 0, 0, 0.5), c(0, 0, 0, 0.18);
    TDROP: c(0, 0, 0, 0.36), c(0, 0, 0, 0.22);
}
/// .chint / .cxb glass: rgba(22,24,30,.62)
const PILL: Rgba = c(22, 24, 30, 0.62);
const ACC: Rgba = c(10, 132, 255, 1.0);
const ETAB_ON: Rgba = c(61, 155, 255, 1.0);

/// The overlay's text: `font-family: var(--font)` with the browser's own letter-spacing (no #sw -.006em here).
fn f(size: f32, w: u16) -> Font {
    Font::new(size, w).ls(0)
}
fn t(s: &str, size: f32, w: u16, col: Rgba, lh: f32) -> El {
    El::text(s, f(size, w), col, lh).none()
}
fn tn(s: &str, size: f32, w: u16, col: Rgba, lh: f32) -> El {
    El::text(s, f(size, w).tnum(), col, lh).none()
}
fn rgb_of(c: u32) -> Rgba {
    Rgba::rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// One glass part: its key, `backdrop-filter: blur(b) [saturate(s)]`, and its border radius.
#[derive(Clone, Copy, Debug)]
pub struct Glass {
    pub key: Key,
    pub blur: f32,
    pub sat: Option<f32>,
}

/// What one monitor's window shows.
pub struct Scene {
    /// the monitor's size in DIPs and its scale
    pub w: f32,
    pub h: f32,
    pub scale: f32,
    pub back: El,
    pub front: El,
    pub glass: Vec<Glass>,
    /// something is still moving (fades, the emoji pop-in, a flash)
    pub busy: bool,
}

/// Order 054: a decoration the pointer goes through, children included (`El::no_hit` is the box alone: the hit test still
/// looks inside it). The crosshair / ring / emoji drawn under the pointer took every press on the toolbar, so no button
/// could be clicked; the hint and the tips the same where they lie over something.
fn ghost(mut e: El) -> El {
    e.hit = false;
    e.children = e.children.into_iter().map(ghost).collect();
    e
}

/// A small SVG (path / circle / rect elements, `viewBox="0 0 n n"`) stroked in a `size` box, like Blink paints an inline
/// `<svg>`: pixel-snapped origin, `stroke: currentColor`, round caps / joins unless `butt`.
fn svg(src: &'static str, size: f32, stroke: f32, col: Rgba, butt: bool) -> El {
    El::paint(move |g, (x, y, _, _)| paint_svg(g, src, x, y, size, stroke, col, butt)).size(size, size).none()
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let pat = format!(" {}=\"", name);
    let i = tag.find(&pat)? + pat.len();
    let j = tag[i..].find('"')? + i;
    Some(&tag[i..j])
}

pub fn paint_svg(g: &Gfx, src: &str, x: f32, y: f32, size: f32, stroke: f32, col: Rgba, butt: bool) {
    let vb: f32 = attr(src, "viewBox").and_then(|v| v.split_whitespace().nth(2)).and_then(|v| v.parse().ok()).unwrap_or(20.0);
    let (x, y, _, _) = g.snap(x, y, size, size);
    let t0 = g.transform();
    let k = size / vb;
    // an <svg> is a replaced element: its own box clips what it draws (round caps at the ends are cut there)
    g.set_transform(&(Matrix3x2::translation(x, y) * t0));
    g.push_clip(0.0, 0.0, size, size);
    g.set_transform(&(Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: 0.0, M32: 0.0 } * Matrix3x2::translation(x, y) * t0));
    let num = |tag: &str, n: &str| attr(tag, n).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
    for part in src.split('<').skip(1) {
        let tag = part.split('>').next().unwrap_or("");
        let w = attr(tag, "stroke-width").and_then(|v| v.parse().ok()).unwrap_or(stroke);
        let op = attr(tag, "opacity").and_then(|v| v.parse().ok()).unwrap_or(1.0);
        let col = attr(tag, "stroke").and_then(css_rgba).unwrap_or(col);
        let geo = if tag.starts_with("path") {
            attr(tag, "d").map(|d| g.path(d))
        } else if tag.starts_with("circle") {
            let (cx, cy, r) = (num(tag, "cx"), num(tag, "cy"), num(tag, "r"));
            Some(sk::Path::oval(sk::Rect::new(cx - r, cy - r, cx + r, cy + r), None))
        } else if tag.starts_with("rect") {
            let (rx, ry, rw, rh, r) = (num(tag, "x"), num(tag, "y"), num(tag, "width"), num(tag, "height"), num(tag, "rx"));
            Some(sk::Path::rrect(sk::RRect::new_rect_xy(sk::Rect::from_xywh(rx, ry, rw, rh), r, r), None))
        } else {
            None
        };
        if let Some(p) = geo {
            g.stroke_geom_ex(&p, w, col, !butt, true, op);
        }
    }
    g.set_transform(&(Matrix3x2::translation(x, y) * t0));
    g.pop_clip();
    g.set_transform(&t0);
}

/// `rgba(r,g,b,a)` / `#fff` in an SVG attribute.
fn css_rgba(v: &str) -> Option<Rgba> {
    if let Some(h) = v.strip_prefix('#') {
        let n = u32::from_str_radix(h, 16).ok()?;
        return Some(if h.len() == 3 {
            Rgba::rgb((((n >> 8) & 15) * 17) as u8, (((n >> 4) & 15) * 17) as u8, ((n & 15) * 17) as u8)
        } else {
            Rgba::hex(n)
        });
    }
    let inner = v.strip_prefix("rgba(")?.strip_suffix(')')?;
    let p: Vec<f32> = inner.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    (p.len() == 4).then(|| Rgba::rgba(p[0] as u8, p[1] as u8, p[2] as u8, p[3]))
}

/// An emoji as text in a box (`font: <size>px/<lh>px "Segoe UI Emoji"; text-align: center`), opacity `op`.
pub fn emoji_el(e: &str, size: f32, lh: f32, w: f32, op: f32) -> El {
    let e = e.to_string();
    El::paint(move |g, (x, y, bw, _)| {
        let Some(ink::Shaped { blob: Some(blob), adv, asc, desc }) = ink::shaped(&e, "Segoe UI Emoji", 400, size) else { return };
        let base = y + crate::gfx::baseline_in_line(asc, desc, lh);
        let mut p = sk::Paint::default();
        p.set_anti_alias(true);
        p.set_alpha_f(op);
        g.cv().draw_text_blob(&blob, (x + (bw - adv) / 2.0, base), &p);
    })
    .size(w, lh)
    .none()
}

/// The Windows highlight colour (the focused text selection).
fn highlight() -> Rgba {
    #[cfg(windows)]
    unsafe {
        let v = windows::Win32::Graphics::Gdi::GetSysColor(windows::Win32::Graphics::Gdi::COLOR_HIGHLIGHT);
        return Rgba::rgb((v & 0xff) as u8, ((v >> 8) & 0xff) as u8, ((v >> 16) & 0xff) as u8);
    }
    #[allow(unreachable_code)]
    Rgba::rgb(0, 120, 215)
}

fn ease(b: Bezier, p: f64) -> f32 {
    b.ease(p) as f32
}

/// The shared chip look of the size tag, the monitor buttons, Live and Snap:
/// `height:22px;border-radius:6px;font:600 11.5px/22px;background:rgba(14,16,22,.78);backdrop-filter:blur(10px);
///  box-shadow:inset 0 0 0 .5px rgba(255,255,255,.18),0 2px 8px rgba(0,0,0,.25)` (+ hover rgba(34,38,50,.9), .12s ease).
fn chip(cx: &mut Cx, k: Key, glass: &mut Vec<Glass>, hover_bg: bool) -> El {
    let h = if hover_bg { cx.hover_t(k, 120.0, EASE) } else { 0.0 };
    glass.push(Glass { key: k, blur: 10.0, sat: None });
    El::row()
        .key(k)
        .h(22.0)
        .none()
        .center()
        .radius(6.0)
        .bg(CHIP.mix(CHIP_H, h))
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, c(255, 255, 255, 0.18))])
        .shadow(&[sh(0.0, 2.0, 8.0, 0.0, c(0, 0, 0, 0.25))])
}

/// The key cap (`.ckc`): `inline-flex; height:18px; padding:0 6px; margin-right:-4px; border-radius:4px; background:rgba(255,255,255,.16);
/// box-shadow:inset 0 0 0 .5px rgba(255,255,255,.14),0 1px 0 rgba(0,0,0,.3); font:600 11px/1`.
fn keycap(s: &str, h: f32, size: f32, pad: f32, mr: f32, bg: Rgba) -> El {
    El::row()
        .center()
        .h(h)
        .none()
        .pad(0.0, pad, 0.0, pad)
        .margin(0.0, mr, 0.0, 0.0)
        .radius(4.0)
        .bg(bg)
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, c(255, 255, 255, 0.14))])
        .shadow(&[sh(0.0, 1.0, 0.0, 0.0, c(0, 0, 0, 0.3))])
        .child(t(s, size, 600, WHITE, size))
}

/// MONI.all: two small screens side by side.
const MON_ALL: &str = r#"<svg viewBox="0 0 20 20"><rect x="1.5" y="4.5" width="7.8" height="6.5" rx="1.2"/><rect x="10.7" y="4.5" width="7.8" height="6.5" rx="1.2"/><path d="M5.4 11v3M3.6 14h3.6M14.6 11v3M12.8 14h3.6"/></svg>"#;
/// ICON.wcls with the overlay's round caps (`.cxb svg{stroke-linecap:round}`), cut by the svg's box.
const X_ICON: &str = r#"<svg viewBox="0 0 10 10"><path d="M.6.6l8.8 8.8M9.4.6L.6 9.4"/></svg>"#;
/// The crosshair (`.cmk`): a dark 3 px under-stroke, then the white 1 px cross (butt caps).
const CROSS: &str = r##"<svg viewBox="0 0 25 25"><path d="M12.5 1v8.5M12.5 15.5V24M1 12.5h8.5M15.5 12.5H24" stroke="rgba(0,0,0,.5)" stroke-width="3"/><path d="M12.5 1v8.5M12.5 15.5V24M1 12.5h8.5M15.5 12.5H24" stroke="#fff" stroke-width="1"/></svg>"##;

/// A box in desktop px -> this monitor's DIPs.
fn to_dip(m: &bu_screenshot::Monitor, s: f32, r: Sel) -> (f32, f32, f32, f32) {
    ((r.x - m.rect.x) as f32 / s, (r.y - m.rect.y) as f32 / s, r.w as f32 / s, r.h as f32 / s)
}

/// The monitor the selection's bars sit on: the one under the box's centre (else the home monitor).
pub fn bars_monitor(m: &Model) -> usize {
    match m.sel {
        Some(s) => m
            .monitors
            .iter()
            .position(|mo| mo.rect.contains(s.x + s.w / 2, s.y + s.h / 2))
            .or_else(|| m.monitors.iter().position(|mo| mo.rect.contains(s.x, s.y)))
            .unwrap_or(m.home),
        None => m.home,
    }
}

/// The width of a box when laid out on its own (offsetWidth).
fn measure(g: &Gfx, e: &El) -> (f32, f32) {
    let l = Laid::new(g, El::row().items(AlignItems::FLEX_START).child(e.clone()), 100_000.0, None);
    (l.nodes[1].rect.2, l.nodes[1].rect.3)
}

/// Build what monitor `mi` shows at `now`.
pub fn scene(m: &Model, cx: &mut Cx, mi: usize, now: f64) -> Scene {
    let mo = m.monitors[mi].clone();
    let s = mo.dpi as f32 / 96.0;
    let (w, h) = (mo.rect.w as f32 / s, mo.rect.h as f32 / s);
    let mut busy = false;
    let mut glass = Vec::new();
    let opened = now - m.opened_at;
    busy |= opened < 400.0;

    // ================================================================ back: dim, the box, the marks
    let mut back = El::block().size(w, h);
    let gone = m.sel.is_some() || m.mode != Mode::Idle;
    // #cap .cdim{position:absolute;inset:0;background:var(--cdim)} .cdim.gone{opacity:0}; fades in 140 ms ease-out at the start
    // and after a reset
    let dim_in = ease(EASE_OUT_CSS, anim::prog(now, m.opened_at.max(m.reset_at), 0.0, 140.0));
    if !gone && dim_in > 0.0 {
        back = back.child(El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(w, h).bg(CDIM).opacity(dim_in));
    }
    // the marks (#cap .ccv), clipped to the box
    if let Some(sel) = m.sel {
        if !m.anns.is_empty() {
            let anns: Rc<Vec<Ann>> = Rc::new(m.anns.clone());
            let (ox, oy) = (mo.rect.x as f32, mo.rect.y as f32);
            busy |= anns.iter().any(|a| matches!(a, Ann::Emo { born, .. } if now - born < 260.0));
            back = back.child(
                El::paint(move |g, _| {
                    let cv = g.cv();
                    cv.save();
                    // DIPs -> desktop px: the marks are kept in desktop px
                    cv.scale((1.0 / s, 1.0 / s));
                    cv.translate((-ox, -oy));
                    ink::draw_all(cv, &anns, sk::Rect::from_xywh(sel.x as f32, sel.y as f32, sel.w as f32, sel.h as f32), Some(now));
                    cv.restore();
                })
                .abs(0.0, 0.0, f32::NAN, f32::NAN)
                .size(w, h)
                .no_hit(),
            );
        }
        // .csel{box-shadow:0 0 0 9000px var(--cdim)} + ::after{inset:-1px;border:1px solid #fff;box-shadow:0 0 0 1px rgba(0,0,0,.38)}
        let (x, y, bw, bh) = to_dip(&mo, s, sel);
        let ed = m.mode == Mode::Edit;
        let mut csel = El::block()
            .abs(x, y, f32::NAN, f32::NAN)
            .size(bw, bh)
            .no_hit()
            .shadow(&[sh(0.0, 0.0, 0.0, 9000.0, CDIM)])
            .child(El::block().abs(-1.0, -1.0, f32::NAN, f32::NAN).size(bw + 2.0, bh + 2.0).border(1.0, WHITE).shadow(&[sh(0.0, 0.0, 0.0, 1.0, c(0, 0, 0, 0.38))]).no_hit());
        // the 4 handles: .hd{width:8px;height:8px;margin:-4px 0 0 -4px;border-radius:50%;background:#fff;box-shadow:0 0 0 1px rgba(0,0,0,.42),
        //   0 1px 3px rgba(0,0,0,.3);opacity:0;transform:scale(.3);transition:opacity .12s ease,transform .2s cubic-bezier(.3,1.4,.5,1)}
        //   .csel.ed .hd{opacity:1;transform:none}; tl/tr/bl/br at -.5px / calc(100% + .5px)
        let op = cx.tr(K_HD, 1, if ed { 1.0 } else { 0.0 }, 120.0, EASE);
        let sc = cx.tr(K_HD, 2, if ed { 1.0 } else { 0.3 }, 200.0, Bezier::new(0.3, 1.4, 0.5, 1.0));
        for (i, (hx, hy)) in [(-0.5, -0.5), (bw + 0.5, -0.5), (-0.5, bh + 0.5), (bw + 0.5, bh + 0.5)].into_iter().enumerate() {
            csel = csel.child(
                El::block()
                    .key(idx(K_HD, i))
                    .abs(hx - 4.0, hy - 4.0, f32::NAN, f32::NAN)
                    .size(8.0, 8.0)
                    .radius(RADIUS_PILL)
                    .bg(WHITE)
                    .shadow(&[sh(0.0, 0.0, 0.0, 1.0, c(0, 0, 0, 0.42)), sh(0.0, 1.0, 3.0, 0.0, c(0, 0, 0, 0.3))])
                    .opacity(op)
                    .scale(sc)
                    .z(1),
            );
        }
        back = back.child(csel);
    }

    // ================================================================ front
    let mut front = El::block().size(w, h);
    let bars_on_this = bars_monitor(m) == mi;
    let home = m.home == mi;
    let ed = m.mode == Mode::Edit;
    let top = m.sel.is_none();

    // ---- .cbar: the size tag, the monitor buttons, Live, Snap
    if bars_on_this && (top || m.mode != Mode::Idle) {
        let mut kids = Vec::new();
        if !top {
            // .csz{display:block;margin-right:2px;padding:0 8px} .csz i{color:rgba(255,255,255,.5);margin:0 3px}
            let sel = m.sel.unwrap_or_default();
            let mut sz = chip(cx, K_SZ, &mut glass, ed && m.size_edit.is_none()).margin(0.0, 2.0, 0.0, 0.0).cursor(crate::ui::el::Cursor::Text);
            if let Some(text) = &m.size_edit {
                // .csz.ed{background:rgba(14,16,22,.9);box-shadow:0 0 0 2px rgba(10,132,255,.45),inset 0 0 0 1px #0a84ff,0 2px 8px rgba(0,0,0,.25)}
                // .csz input{width:84px;height:22px;text-align:center} - the text is selected when the field opens
                // the selected text: Chromium's selection colours made translucent (Color::BlendWithWhite): focused = the
                // Windows highlight (white text), unfocused = #c8c8c8 -> rgba(163,163,163,.6) with #323232 text
                let sel_all = m.size_selected;
                let (sbg, sfg) = if m.focused { (highlight().a(0.8), WHITE) } else { (c(163, 163, 163, 0.6), Rgba::rgb(50, 50, 50)) };
                let inner = if sel_all {
                    El::row().h(22.0).center().child(El::row().h(22.0).center().bg(sbg).child(tn(text, 11.5, 600, sfg, 22.0)))
                } else {
                    El::row().h(22.0).center().child(tn(text, 11.5, 600, WHITE, 22.0))
                };
                sz = sz
                    .bg(c(14, 16, 22, 0.9))
                    .inset(&[sh(0.0, 0.0, 0.0, 1.0, ACC)])
                    .shadow(&[sh(0.0, 0.0, 0.0, 2.0, c(10, 132, 255, 0.45)), sh(0.0, 2.0, 8.0, 0.0, c(0, 0, 0, 0.25))])
                    .pad(0.0, 8.0, 0.0, 8.0)
                    .child(El::row().w(84.0).h(22.0).justify(JustifyContent::CENTER).child(inner));
            } else {
                sz = sz.pad(0.0, 8.0, 0.0, 8.0).on_click(K_SZ).children([
                    tn(&sel.w.to_string(), 11.5, 600, WHITE, 22.0),
                    tn("×", 11.5, 600, c(255, 255, 255, 0.5), 22.0).margin(0.0, 3.0, 0.0, 3.0),
                    tn(&sel.h.to_string(), 11.5, 600, WHITE, 22.0),
                ]);
            }
            kids.push(sz);
        }
        if ed || top {
            let lit = m.lit();
            for (i, (label, _)) in m.presets().iter().enumerate() {
                // .cmb{display:inline-flex;align-items:center;gap:4px;padding:0 8px 0 6px;color:rgba(255,255,255,.84)} svg 14 px stroke 1.4
                // .cmb.on{background:rgba(10,132,255,.88);color:#fff;box-shadow:inset 0 0 0 .5px rgba(255,255,255,.3),0 2px 8px rgba(0,0,0,.25)}
                let k = idx(K_MON, i);
                let on = lit == Some(i);
                let hv = cx.hover_t(k, 120.0, EASE);
                let col = CHIP_FG.mix(WHITE, if on { 1.0 } else { hv });
                let all = label == "All";
                let icon = if all { svg(MON_ALL, 14.0, 1.4, col, false) } else { El::icon("mon", 14.0, 1.4, col) };
                let mut b = chip(cx, k, &mut glass, !on).on_click(k).gap(4.0).pad(0.0, 8.0, 0.0, 6.0).cursor(crate::ui::el::Cursor::Hand);
                if on {
                    b = b
                        .bg(c(10, 132, 255, 0.88))
                        .inset(&[sh(0.0, 0.0, 0.0, 0.5, c(255, 255, 255, 0.3))])
                        .shadow(&[sh(0.0, 2.0, 8.0, 0.0, c(0, 0, 0, 0.25))]);
                }
                let pr = cx.active_t(k, 120.0, EASE);
                kids.push(b.scale(1.0 - 0.06 * pr).child(icon).child(tn(label, 11.5, 600, col, 22.0)));
            }
            // .cbs{width:1px;height:14px;margin:0 3px;background:rgba(255,255,255,.24)}
            kids.push(El::block().size(1.0, 14.0).none().margin(0.0, 3.0, 0.0, 3.0).bg(c(255, 255, 255, 0.24)));
            // .clv{padding:0 9px 0 7px;gap:6px} .lsw{20x12;border-radius:6px;background:rgba(255,255,255,.24)} ::after{left:2px;top:2px;8x8;#fff}
            //   .clv.on .lsw{background:#ff453a} ::after translateX(8px) (.22s cubic-bezier(.3,.7,.2,1)); bg .2s ease
            let on = m.live;
            let hv = cx.hover_t(K_LIVE, 120.0, EASE);
            let col = CHIP_FG.mix(WHITE, if on { 1.0 } else { hv });
            let kx = cx.tr(K_LIVE, 1, if on { 8.0 } else { 0.0 }, 220.0, Bezier::new(0.3, 0.7, 0.2, 1.0));
            let tr = cx.tr(K_LIVE, 2, if on { 1.0 } else { 0.0 }, 200.0, EASE);
            let sw = El::block()
                .size(20.0, 12.0)
                .none()
                .radius(6.0)
                .bg(c(255, 255, 255, 0.24).mix(Rgba::hex(0xff453a), tr))
                .child(El::block().abs(2.0, 2.0, f32::NAN, f32::NAN).size(8.0, 8.0).radius(RADIUS_PILL).bg(WHITE).translate(kx, 0.0));
            let pr = cx.active_t(K_LIVE, 120.0, EASE);
            // Order 045: `title:'Live: the picture keeps moving until you snap'`
            kids.push(chip(cx, K_LIVE, &mut glass, true).on_click(K_LIVE).title("Live: the picture keeps moving until you snap").gap(6.0).pad(0.0, 9.0, 0.0, 7.0).scale(1.0 - 0.05 * pr).cursor(crate::ui::el::Cursor::Hand).child(sw).child(tn("Live", 11.5, 600, col, 22.0)));
            if m.live {
                // #cap .csnp{padding:0 4px 0 10px;background:rgba(10,132,255,.92)} :hover #2a94ff; .csnp .ckc{height:16px;margin:0;padding:0 5px;
                //   font-size:10px;background:rgba(255,255,255,.22)}
                let hv = cx.hover_t(K_SNAP, 120.0, EASE);
                let pr = cx.active_t(K_SNAP, 120.0, EASE);
                kids.push(
                    chip(cx, K_SNAP, &mut glass, false)
                        .on_click(K_SNAP)
                        // Order 045: `title:'Snap this moment'`
                        .title("Snap this moment")
                        .gap(6.0)
                        .pad(0.0, 4.0, 0.0, 10.0)
                        .bg(c(10, 132, 255, 0.92).mix(Rgba::hex(0x2a94ff), hv))
                        .scale(1.0 - 0.05 * pr)
                        .cursor(crate::ui::el::Cursor::Hand)
                        .child(tn("Snap", 11.5, 600, WHITE, 22.0))
                        .child(keycap("Space", 16.0, 10.0, 5.0, 0.0, c(255, 255, 255, 0.22))),
                );
            }
        }
        // .cbar{display:flex;align-items:center;gap:4px}; barPlace(): just above the box's top-left, inside it when there is no room,
        // always on screen, never under the ×; no box: centred at y 62
        let bar = El::row().key(K_BAR).center().gap(4.0).children(kids);
        let bw = measure(cx.g, &bar).0.round(); // offsetWidth (whole px)
        let (bx, by) = match m.sel {
            None => (((w - bw) / 2.0).round(), 62.0),
            Some(sel) => {
                let (sx, sy, _, _) = to_dip(&mo, s, sel);
                let (mut x, mut y) = (sx - 1.0, sy - 8.0 - 22.0);
                if y < 6.0 {
                    x = sx + 7.0;
                    y = sy + 7.0;
                }
                x = x.clamp(8.0, (w - bw - 8.0).max(8.0));
                y = y.clamp(8.0, h - 30.0);
                if y < 58.0 && x + bw > w - 60.0 {
                    x = (w - 60.0 - bw).max(8.0);
                }
                (x.round(), y.round())
            }
        };
        // the bar fades in 240 ms after 110 ms when the overlay opens
        let op = ease(EASE_OUT_CSS, anim::prog(now, m.opened_at, 110.0, 240.0));
        front = front.child(bar.abs(bx, by, f32::NAN, f32::NAN).opacity(op).z(2));
    }

    // ---- .ctb: the toolbar under the box
    let mut tb_rect = None;
    if bars_on_this && ed {
        if let Some(sel) = m.sel {
            let mut kids = Vec::new();
            for (i, tool) in Tool::ALL.iter().enumerate() {
                kids.push(tool_button(cx, idx(K_TOOL, i), tool.icon(), tool.name(), m.tool == Some(*tool), false));
            }
            kids.push(tsep());
            for (i, col) in COLORS.iter().enumerate() {
                // .cdot{22x22;border-radius:50%} i{12x12;border-radius:50%;box-shadow:inset 0 0 0 .5px rgba(0,0,0,.35)}
                //   .cdot.on{box-shadow:inset 0 0 0 1.5px var(--tring)} .cdot:hover i{transform:scale(1.15)}
                let k = idx(K_DOT, i);
                let hv = cx.hover_t(k, 150.0, EASE);
                let on = cx.tr(k, 1, if m.color == *col { 1.0 } else { 0.0 }, 150.0, EASE);
                let mut d = El::block().on_click(k).size(22.0, 22.0).none().radius(RADIUS_PILL).place_center().cursor(crate::ui::el::Cursor::Hand);
                if on > 0.0 {
                    d = d.inset(&[sh(0.0, 0.0, 0.0, 1.5, TRING().mul_a(on))]);
                }
                kids.push(d.child(El::block().size(12.0, 12.0).radius(RADIUS_PILL).bg(rgb_of(*col)).inset(&[sh(0.0, 0.0, 0.0, 0.5, c(0, 0, 0, 0.35))]).scale(1.0 + 0.15 * hv)));
            }
            kids.push(tsep());
            kids.push(tool_button(cx, K_UNDO, "undo", "Undo", now - m.undo_at < 280.0, !m.can_undo()));
            busy |= now - m.undo_at < 280.0;
            kids.push(tsep());
            // Order 045: `tbt(..,'Copy the picture and close (Ctrl+C)')`, `coSaveB.title='Save to '+S.shotDir+' and close (Ctrl+S)'`
            kids.push(fin_button(cx, K_COPY, "copy", "Copy", 0.0).title("Copy the picture and close (Ctrl+C)"));
            let save = fin_button(cx, K_SAVE, "save", "Save", 4.0);
            kids.push(if m.save_dir.is_empty() { save } else { save.title(&format!("Save to {} and close (Ctrl+S)", m.save_dir)) });
            glass.push(Glass { key: K_TB, blur: 28.0, sat: Some(1.7) });
            // .ctb{display:flex;align-items:center;gap:2px;height:42px;padding:0 5px;border-radius:12px} + the glass
            let tb = El::row()
                .key(K_TB)
                .center()
                .gap(2.0)
                .h(42.0)
                .pad(0.0, 5.0, 0.0, 5.0)
                .radius(12.0)
                .bg(TBG())
                .inset(&[sh(0.0, 0.0, 0.0, 0.5, TRIM())])
                .shadow(&[sh(0.0, 0.0, 0.0, 0.5, TRING_O()), sh(0.0, 12.0, 32.0, 0.0, TDROP())])
                .children(kids);
            let tw = measure(cx.g, &tb).0.round(); // offsetWidth
            // tbPlace(): under the box, right-aligned; above it when there is no room; inside as a last resort
            let (sx, sy, sw, sh_) = to_dip(&mo, s, sel);
            let (mut x, mut y, mut below) = (sx + sw - tw, sy + sh_ + 12.0, true);
            if y + 42.0 > h - 8.0 {
                y = sy - 12.0 - 42.0;
                below = false;
                if y < 8.0 {
                    y = sy + sh_ - 42.0 - 10.0;
                }
            }
            x = x.clamp(8.0, w - tw - 8.0);
            y = y.clamp(8.0, h - 42.0 - 8.0);
            let (x, y) = (x.round(), y.round());
            // tbShow(): opacity 0 -> 1, translateY(-6 / 6 px) -> 0, 200 ms EASE_OUT
            let p = ease(EASE_OUT, anim::prog(now, m.tb_at, 0.0, 200.0));
            busy |= p < 1.0;
            front = front.child(tb.abs(x, y, f32::NAN, f32::NAN).opacity(p).translate(0.0, (1.0 - p) * if below { -6.0 } else { 6.0 }));
            tb_rect = Some((x, y, tw, below));
        }
    }

    // ---- .cemj: the emoji picker next to the Emoji button
    if let (true, Some((tx, ty, _, below))) = (m.picker.open, tb_rect) {
        let pk = picker(m, cx, now, &mut busy);
        glass.push(Glass { key: K_EMJ, blur: 28.0, sat: Some(1.7) });
        let (pw, ph) = measure(cx.g, &pk);
        let (pw, ph) = (pw.round(), ph.round()); // offsetWidth / offsetHeight
        // emjPlace(): centred on the Emoji button, the side with room, always on screen
        let ex = tx + 5.0 + 5.0 * 34.0 + 16.0;
        let mut y = if below { ty + 42.0 + 8.0 } else { ty - ph - 8.0 };
        if y + ph > h - 6.0 || y < 6.0 {
            y = if below { ty - ph - 8.0 } else { ty + 42.0 + 8.0 };
        }
        let y = y.clamp(8.0, (h - ph - 8.0).max(8.0)).round();
        let x = (ex - pw / 2.0).clamp(8.0, w - pw - 8.0).round();
        let (from_op, from_sc, dur) = if m.picker.big { (0.4, 0.97, 180.0) } else { (0.0, 0.96, 160.0) };
        let p = ease(EASE_OUT, anim::prog(now, m.picker.opened_at, 0.0, dur));
        busy |= p < 1.0;
        front = front.child(pk.abs(x, y, f32::NAN, f32::NAN).opacity(from_op + (1.0 - from_op) * p).scale(from_sc + (1.0 - from_sc) * p));
    }

    // ---- .chint (home monitor, until something is picked)
    if home {
        // .chint{left:50%;top:18px;transform:translateX(-50%);gap:10px;height:34px;padding:0 15px;border-radius:17px;font:12px/1;
        //   color:rgba(255,255,255,.92);background:rgba(22,24,30,.62);backdrop-filter:blur(20px) saturate(1.5);
        //   box-shadow:inset 0 0 0 .5px rgba(255,255,255,.16),0 8px 24px rgba(0,0,0,.28)} .chint.hide{opacity:0;transform:translate(-50%,-6px)}
        let hide = m.sel.is_some() || m.mode != Mode::Idle;
        let sep = || El::block().size(1.0, 14.0).none().bg(c(255, 255, 255, 0.18));
        let fg = c(255, 255, 255, 0.92);
        let hint = El::row()
            .key(key("cap.hint"))
            .center()
            .gap(10.0)
            .h(34.0)
            .pad(0.0, 15.0, 0.0, 15.0)
            .radius(17.0)
            .bg(PILL)
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, c(255, 255, 255, 0.16))])
            .shadow(&[sh(0.0, 8.0, 24.0, 0.0, c(0, 0, 0, 0.28))])
            .no_hit()
            .children([
                t("Drag to capture an area", 12.0, 600, WHITE, 12.0),
                sep(),
                t("Click for a whole screen", 12.0, 400, fg, 12.0),
                sep(),
                keycap("Esc", 18.0, 11.0, 6.0, -4.0, c(255, 255, 255, 0.16)),
                t("or right-click to close", 12.0, 400, fg, 12.0),
            ]);
        let (hw, _) = measure(cx.g, &hint);
        // the start: opacity 0 -> 1, translateY(-8 px) -> 0, 260 ms after 60 ms (EASE_OUT); after a reset 220 ms; hiding: .16s / .2s ease
        let op_t = cx.tr(key("cap.hint"), 1, if hide { 0.0 } else { 1.0 }, 160.0, EASE);
        let ty_t = cx.tr(key("cap.hint"), 2, if hide { -6.0 } else { 0.0 }, 200.0, EASE);
        let (sp, sdelay, sdur) = if m.reset_at > m.opened_at { (m.reset_at, 0.0, 220.0) } else { (m.opened_at, 60.0, 260.0) };
        let p = ease(EASE_OUT, anim::prog(now, sp, sdelay, sdur));
        let from = if m.reset_at > m.opened_at { -6.0 } else { -8.0 };
        let (op, ty) = if p < 1.0 && !hide { (p, from * (1.0 - p)) } else { (op_t, ty_t) };
        if op > 0.0 {
            glass.push(Glass { key: key("cap.hint"), blur: 20.0, sat: Some(1.5) });
            // left:50% then translateX(-50%): the box is snapped at left 50 % and the transform moves it by a fraction (like Blink)
            front = front.child(ghost(hint).abs(w / 2.0, 18.0, f32::NAN, f32::NAN).opacity(op).translate(-hw / 2.0, ty));
        }
        // .cxb{right:18px;top:18px;34x34;border-radius:50%;color:rgba(255,255,255,.88);background:rgba(22,24,30,.62);
        //   backdrop-filter:blur(20px) saturate(1.5);box-shadow:inset 0 0 0 .5px rgba(255,255,255,.16),0 8px 24px rgba(0,0,0,.28)}
        //   :hover{background:rgba(196,43,28,.92);color:#fff} svg 11 px stroke 1.5 round caps; the start: scale .8 -> 1, 260 ms after 80 ms
        let hv = cx.hover_t(K_X, 120.0, EASE);
        let pr = cx.active_t(K_X, 120.0, EASE);
        let p = ease(EASE_OUT, anim::prog(now, m.opened_at, 80.0, 260.0));
        glass.push(Glass { key: K_X, blur: 20.0, sat: Some(1.5) });
        let col = c(255, 255, 255, 0.88).mix(WHITE, hv);
        front = front.child(
            El::block()
                .on_click(K_X)
                // Order 045: `title:'Close (Esc)'`
                .title("Close (Esc)")
                .abs(w - 18.0 - 34.0, 18.0, f32::NAN, f32::NAN)
                .size(34.0, 34.0)
                .radius(RADIUS_PILL)
                .place_center()
                .bg(PILL.mix(c(196, 43, 28, 0.92), hv))
                .inset(&[sh(0.0, 0.0, 0.0, 0.5, c(255, 255, 255, 0.16))])
                .shadow(&[sh(0.0, 8.0, 24.0, 0.0, c(0, 0, 0, 0.28))])
                .opacity(p)
                .scale((0.8 + 0.2 * p) * (1.0 - 0.08 * pr))
                .z(3)
                .child(svg(X_ICON, 11.0, 1.5, col, false)),
        );
    }

    // ---- the text being typed (.ctxt)
    if let Some(tf) = &m.text {
        if mo.rect.contains(tf.x as i32, tf.y as i32) {
            // .ctxt{height:30px;min-width:24px;padding:0 4px;border:1px dashed rgba(255,255,255,.8);border-radius:4px;background:rgba(0,0,0,.14);
            //   font:600 20px/28px;text-shadow:0 1px 3px rgba(0,0,0,.5)} width = the text's width + 12 (fitTxt)
            let (x, y) = ((tf.x - mo.rect.x as f32) / s, (tf.y - mo.rect.y as f32) / s);
            let shown = if tf.text.is_empty() { "M" } else { tf.text.as_str() };
            let tw = (cx.g.text_width(shown, f(20.0, 600)) + 12.0).ceil().max(24.0);
            let col = rgb_of(tf.c);
            let text = tf.text.clone();
            // the caret blinks (530 ms) only while the window has the keyboard
            let caret_on = m.focused && ((now - m.opened_at) / 530.0) as i64 % 2 == 0;
            busy = true;
            front = front.child(
                El::paint(move |g, (bx, by, bw, bh)| {
                    g.fill_rr(bx, by, bw, bh, 4.0, c(0, 0, 0, 0.14));
                    dashed_border(g, bx, by, bw, bh, c(255, 255, 255, 0.8));
                    let ft = f(20.0, 600);
                    let tw = g.text_width(&text, ft);
                    // Chromium centres an input's line a pixel lower than its line box here (measured on the drawing)
                    // text-shadow:0 1px 3px rgba(0,0,0,.5): the glyphs blurred (sigma 1.5) under the text
                    if let Some(ink::Shaped { blob: Some(blob), asc, desc, .. }) = ink::shaped(&text, "Segoe UI Variable Text", 600, 20.0 * g.scale) {
                        let base = by + 2.0 + crate::gfx::baseline_in_line(asc / g.scale, desc / g.scale, 28.0);
                        let mut p = sk::Paint::new(c(0, 0, 0, 0.5).c4(), None);
                        p.set_anti_alias(true);
                        p.set_mask_filter(sk::MaskFilter::blur(sk::BlurStyle::Normal, 1.5, false));
                        let cv = g.cv();
                        cv.save();
                        cv.scale((1.0 / g.scale, 1.0 / g.scale));
                        cv.draw_text_blob(&blob, ((bx + 5.0) * g.scale, (base + 1.0) * g.scale), &p);
                        cv.restore();
                    }
                    g.text(&text, ft, bx + 5.0, by + 2.0, 28.0, col, Align::Left, 0.0);
                    if caret_on {
                        g.fill_rect(bx + 5.0 + tw, by + 4.0, 1.0, 22.0, col);
                    }
                })
                .abs(x, y, f32::NAN, f32::NAN)
                .size(tw, 30.0),
            );
        }
    }

    // ---- .cfl: the white flash over the box (Snap / Copy / Save)
    if let Some(fl) = m.flash {
        let p = anim::prog(now, fl.at, 0.0, fl.ms);
        if p < 1.0 {
            busy = true;
            let (x, y, bw, bh) = to_dip(&mo, s, fl.r);
            let op = fl.peak * (1.0 - ease(EASE_OUT_CSS, p));
            front = front.child(El::block().abs(x, y, f32::NAN, f32::NAN).size(bw, bh).bg(WHITE).opacity(op).no_hit());
        }
    }

    // ---- the crosshair (.cxh) where the pointer is
    if mo.rect.contains(m.last.0.floor() as i32, m.last.1.floor() as i32) {
        let (px, py) = (((m.last.0 - mo.rect.x as f32) / s).round(), ((m.last.1 - mo.rect.y as f32) / s).round());
        let op = ease(EASE_OUT_CSS, anim::prog(now, m.opened_at, 0.0, 120.0));
        let mut xh = El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(w, h).no_hit().opacity(op);
        match m.pointer() {
            Pointer::Full | Pointer::Cross => {
                if m.pointer() == Pointer::Full {
                    // .cxl{background:rgba(255,255,255,.2)} .v{left:0;width:1px} .h{top:0;height:1px} (through the pointer)
                    xh = xh
                        .child(El::block().abs(px, 0.0, f32::NAN, f32::NAN).size(1.0, h).bg(c(255, 255, 255, 0.2)))
                        .child(El::block().abs(0.0, py, f32::NAN, f32::NAN).size(w, 1.0).bg(c(255, 255, 255, 0.2)));
                }
                // .cmk{left:-12px;top:-12px;width:25px;height:25px}
                xh = xh.child(svg(CROSS, 25.0, 1.0, WHITE, true).abs(px - 12.0, py - 12.0, f32::NAN, f32::NAN));
            }
            Pointer::Ring => {
                // .cring{border-radius:50%;border:1.5px solid <colour>;box-shadow:0 0 0 1px rgba(0,0,0,.35),inset 0 0 0 1px rgba(0,0,0,.2)}
                let r = if m.tool == Some(Tool::Hl) { 18.0 } else { 7.0 };
                let col = rgb_of(m.color);
                xh = xh.child(
                    El::block()
                        .abs(px - r / 2.0, py - r / 2.0, f32::NAN, f32::NAN)
                        .size(r, r)
                        .radius(RADIUS_PILL)
                        .border((1.5 * s).floor() / s, col)
                        .shadow(&[sh(0.0, 0.0, 0.0, 1.0, c(0, 0, 0, 0.35))])
                        .inset(&[sh(0.0, 0.0, 0.0, 1.0, c(0, 0, 0, 0.2))]),
                );
            }
            Pointer::Emo => {
                // .cemo{transform:translate(-50%,-50%);font:36px/1 "Segoe UI Emoji";opacity:.55}
                xh = xh.child(emoji_el(&m.emoji, 36.0, 36.0, 48.0, 0.55).abs(px - 24.0, py - 18.0, f32::NAN, f32::NAN));
            }
            _ => {}
        }
        front = front.child(ghost(xh).z(4));
    }

    // ---- the hover tips (`[data-n]::after`, after .35 s): the tools + Undo (10 px under the toolbar, above it when the toolbar is
    // above the box), the monitor buttons ("Monitor 1 · 1920×1080", 8 px under the bar, above when the bar is low), the emoji tabs
    // (6 px above)
    let mut tips: Vec<(Key, String, f32, bool, f32)> = Vec::new();
    if let Some((_, _, _, below)) = tb_rect {
        for (i, tool) in Tool::ALL.iter().enumerate() {
            tips.push((idx(K_TOOL, i), tool.name().to_string(), 10.0, below, 0.88));
        }
        tips.push((K_UNDO, "Undo".to_string(), 10.0, below, 0.88));
    }
    for (i, (label, r)) in m.presets().iter().enumerate() {
        let n = if label == "All" { "All".to_string() } else { format!("Monitor {label}") };
        tips.push((idx(K_MON, i), format!("{n} · {}×{}", r.w, r.h), 8.0, true, 0.88));
    }
    for (i, name) in emoji::tab_names().iter().enumerate() {
        tips.push((idx(K_ETAB, i), name.to_string(), 6.0, false, 0.92));
    }
    let mut laid: Option<Laid> = None;
    for (k, name, gap, below, a) in tips {
        let on = cx.hovered(k) && cx.hover_age(k) >= 350.0;
        let op = cx.tr(k, 7, if on { 1.0 } else { 0.0 }, 120.0, EASE);
        if on && op < 1.0 {
            busy = true;
        }
        if op <= 0.0 {
            continue;
        }
        let l = laid.get_or_insert_with(|| Laid::new(cx.g, front.clone(), w, Some(h)));
        let Some(n) = l.nodes.iter().find(|n| n.el.key == Some(k)) else { continue };
        let (rx, ry, rw, rh) = n.rect;
        let (dx, dy) = front_translate(l, n);
        let (rx, ry) = (rx + dx, ry + dy);
        // the monitor buttons' tips go above when the bar sits low on the screen (.cbar.lo)
        let below = if gap == 8.0 { ry <= h - 80.0 } else { below };
        let tipb = El::row()
            .pad(3.0, 8.0, 3.0, 8.0)
            .radius(6.0)
            .bg(c(18, 20, 26, a))
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, c(255, 255, 255, 0.14))])
            .shadow(&[sh(0.0, 4.0, 12.0, 0.0, c(0, 0, 0, 0.25))])
            .no_hit()
            .child(t(&name, 11.0, 600, WHITE, 15.0));
        let (tw, th) = measure(cx.g, &tipb);
        // left:50% + translate(-50%, 3px -> 0): snapped at the button's centre, then moved by the fraction
        let y = if below { ry + rh + gap } else { ry - gap - th };
        front = front.child(ghost(tipb).abs(rx + rw / 2.0, y, f32::NAN, f32::NAN).opacity(op).translate(-tw / 2.0, 3.0 * (1.0 - op)).z(5));
    }

    Scene { w, h, scale: s, back, front, glass, busy }
}

/// A 1 px dashed border like Blink paints one side at a time (StrokeData::SetupPaintDashPathEffect): dashes of 3 px, the gap
/// stretched so each side starts and ends with a dash.
fn dashed_border(g: &Gfx, x: f32, y: f32, w: f32, h: f32, col: Rgba) {
    let side = |x0: f32, y0: f32, x1: f32, y1: f32| {
        let len = (x1 - x0).abs() + (y1 - y0).abs();
        let (dash, gap0) = (3.0f32, 3.0f32);
        let min_n = ((len + gap0) / (dash + gap0)).floor();
        let max_n = min_n + 1.0;
        let min_gap = if min_n > 1.0 { (len - min_n * dash) / (min_n - 1.0) } else { gap0 };
        let max_gap = (len - max_n * dash) / (max_n - 1.0);
        let gap = if max_gap <= 0.0 || (min_gap - gap0).abs() < (max_gap - gap0).abs() { min_gap } else { max_gap };
        let mut p = sk::Paint::new(col.c4(), None);
        p.set_anti_alias(true).set_style(sk::PaintStyle::Stroke).set_stroke_width(1.0);
        p.set_path_effect(sk::PathEffect::dash(&[dash, gap.max(0.1)], 0.0));
        g.cv().draw_line((x0, y0), (x1, y1), &p);
    };
    side(x, y + 0.5, x + w, y + 0.5);
    side(x, y + h - 0.5, x + w, y + h - 0.5);
    side(x + 0.5, y, x + 0.5, y + h);
    side(x + w - 0.5, y, x + w - 0.5, y + h);
}

/// `.tsep{width:1px;height:20px;margin:0 5px;background:var(--tsep)}`
fn tsep() -> El {
    El::block().size(1.0, 20.0).none().margin(0.0, 5.0, 0.0, 5.0).bg(TSEP())
}

/// `.tbb{32x32;border-radius:8px;display:grid;place-items:center}` svg 18 px stroke 1.5; :hover{background:var(--thv);color:#fff}
/// .tbb.on{background:rgba(10,132,255,.28);color:#fff;box-shadow:inset 0 0 0 1px rgba(10,132,255,.62)} :disabled{opacity:.35}
fn tool_button(cx: &mut Cx, k: Key, icon: &str, _name: &str, on: bool, disabled: bool) -> El {
    let hv = if disabled { 0.0 } else { cx.hover_t(k, 120.0, EASE) };
    let pr = if disabled { 0.0 } else { cx.active_t(k, 120.0, EASE) };
    let col = TFG().mix(TFH(), if on { 1.0 } else { hv });
    let mut b = El::block().on_click(k).size(32.0, 32.0).none().radius(8.0).place_center().cursor(crate::ui::el::Cursor::Hand);
    if on {
        b = b.bg(c(10, 132, 255, 0.28)).inset(&[sh(0.0, 0.0, 0.0, 1.0, c(10, 132, 255, 0.62))]);
    } else if hv > 0.0 {
        b = b.bg(THV().mul_a(hv));
    }
    if disabled {
        b = b.opacity(0.35);
    }
    b.scale(1.0 - 0.08 * pr).child(El::icon(icon, 18.0, 1.5, col))
}

/// `.tbt.fin{height:32px;padding:0 12px 0 10px;border-radius:8px;background:var(--tfill);gap:6px;font:600 12.5px/1}` svg 15 px stroke 1.6;
/// `.tbt.fin+.tbt.fin{margin-left:4px}`; :hover{background:var(--thv);color:#fff}
fn fin_button(cx: &mut Cx, k: Key, icon: &str, label: &str, ml: f32) -> El {
    let hv = cx.hover_t(k, 120.0, EASE);
    let pr = cx.active_t(k, 120.0, EASE);
    let col = TFG().mix(TFH(), hv);
    El::row()
        .on_click(k)
        .h(32.0)
        .none()
        .center()
        .gap(6.0)
        .pad(0.0, 12.0, 0.0, 10.0)
        .margin(0.0, 0.0, 0.0, ml)
        .radius(8.0)
        .bg(TFILL().mix(THV(), hv))
        .scale(1.0 - 0.05 * pr)
        .cursor(crate::ui::el::Cursor::Hand)
        .child(El::icon(icon, 15.0, 1.6, col))
        .child(t(label, 12.5, 600, col, 12.5))
}

/// The emoji picker's boxes (`.cemj`, quick row or More).
fn picker(m: &Model, cx: &mut Cx, now: f64, busy: &mut bool) -> El {
    let _ = (now, &busy);
    // .cemj{display:flex;flex-direction:column;padding:6px;border-radius:12px} + the toolbar's glass
    let root = El::col()
        .key(K_EMJ)
        .pad(6.0, 6.0, 6.0, 6.0)
        .radius(12.0)
        .bg(TBG())
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, TRIM())])
        .shadow(&[sh(0.0, 0.0, 0.0, 0.5, TRING_O()), sh(0.0, 12.0, 32.0, 0.0, TDROP())]);
    if !m.picker.big {
        // .eqr{display:flex;align-items:center;gap:2px}: the quick row, .esep, More
        let mut row = El::row().center().gap(2.0);
        for (i, e) in m.recent.0.iter().enumerate() {
            row = row.child(eb(cx, idx(K_EQ, i), e, m.emoji == *e, true));
        }
        // .esep{width:1px;height:20px;margin:0 5px 0 4px} .emore{height:30px;padding:0 8px 0 11px;border-radius:8px;background:var(--tfill);
        //   gap:4px;font:600 12px/1} svg 11 px stroke 1.6 rotate(-90deg)
        let hv = cx.hover_t(K_MORE, 120.0, EASE);
        let col = TFG().mix(TFH(), hv);
        row = row.child(El::block().size(1.0, 20.0).none().margin(0.0, 5.0, 0.0, 4.0).bg(TSEP())).child(
            El::row()
                .on_click(K_MORE)
                // Order 045: `title:'All emojis'`
                .title("All emojis")
                .h(30.0)
                .none()
                .center()
                .gap(4.0)
                .pad(0.0, 8.0, 0.0, 11.0)
                .radius(8.0)
                .bg(TFILL().mix(THV(), hv))
                .cursor(crate::ui::el::Cursor::Hand)
                .child(t("More", 12.0, 600, col, 12.0))
                .child(El::icon("chevDw", 11.0, 1.6, col).rotate(-90.0)),
        );
        return root.child(row);
    }
    // ---- More: .ebig{flex-direction:column;width:338px}
    // .ehd{display:flex;align-items:center;gap:6px;padding:0 0 6px}: back + the search field
    let hv = cx.hover_t(K_BACK, 120.0, EASE);
    let back = El::block()
        .on_click(K_BACK)
        // Order 045: `title:'Back to the most used'`
        .title("Back to the most used")
        .size(30.0, 30.0)
        .none()
        .radius(8.0)
        .place_center()
        .bg(THV().mul_a(hv))
        .cursor(crate::ui::el::Cursor::Hand)
        .child(svg(emoji::tab_icon("back"), 12.0, 1.6, TFG().mix(TFH(), hv), false));
    // .esrch{flex:1;height:30px;border-radius:8px;background:var(--tin);box-shadow:inset 0 0 0 .5px var(--tsep)} :focus-within::after
    //   {box-shadow:0 0 0 2px rgba(10,132,255,.4),inset 0 0 0 1px #0a84ff} svg{left:9px;top:8px;14px;stroke 1.5;opacity:.55}
    //   .esq{padding:0 10px 0 30px;font:12.5px/30px;color:#fff} ::placeholder{color:var(--tfg);opacity:.6}
    let q = &m.picker.search;
    let shown = if q.is_empty() { t("Search emoji", 12.5, 400, TFG().mul_a(0.6), 30.0) } else { t(q, 12.5, 400, TFH(), 30.0) };
    let srch = El::block()
        .key(K_SRCH)
        .flex1()
        .h(30.0)
        .radius(8.0)
        .bg(TIN())
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, TSEP())])
        .child(svg(emoji::tab_icon("search"), 14.0, 1.5, TFG(), false).opacity(0.55).abs(9.0, 8.0, f32::NAN, f32::NAN))
        .child(El::row().abs(30.0, 0.0, 10.0, f32::NAN).h(30.0).child(shown))
        .child(El::block().abs(0.0, 0.0, 0.0, 0.0).radius(8.0).inset(&[sh(0.0, 0.0, 0.0, 1.0, ACC)]).shadow(&[sh(0.0, 0.0, 0.0, 2.0, c(10, 132, 255, 0.4))]).no_hit());
    let ehd = El::row().center().gap(6.0).pad(0.0, 0.0, 6.0, 0.0).child(back).child(srch);
    // .etabs{display:flex;justify-content:space-between;padding:0 1px 5px;margin-bottom:2px;box-shadow:inset 0 -1px 0 var(--tsep)}
    // .etab{32x28;border-radius:7px;color:var(--tfg);opacity:.72} .etab.on{color:#3d9bff;opacity:1} svg 16 px stroke 1.4
    // .etl{left:0;bottom:-1px;width:20px;height:2px;margin-left:-10px;border-radius:1px;background:#3d9bff} (.26s glide)
    let searching = !q.trim().is_empty();
    let ids = emoji::tab_ids();
    let mut tabs = El::row().justify(JustifyContent::SPACE_BETWEEN).pad(0.0, 1.0, 5.0, 1.0).margin(0.0, 0.0, 2.0, 0.0).inset(&[sh(0.0, -1.0, 0.0, 0.0, TSEP())]);
    for (i, id) in ids.iter().enumerate() {
        let k = idx(K_ETAB, i);
        let on = m.picker.tab == i;
        let hv = if searching { 0.0 } else { cx.hover_t(k, 120.0, EASE) };
        let col = if on && !searching { ETAB_ON } else { TFG() };
        let op = if searching { 0.3 } else if on { 1.0 } else { 0.72 + 0.28 * hv };
        tabs = tabs.child(El::block().on_click(k).size(32.0, 28.0).none().radius(7.0).place_center().bg(THV().mul_a(hv)).opacity(op).child(svg(emoji::tab_icon(id), 16.0, 1.4, col, false)));
    }
    if !searching {
        let lx = cx.tr(K_ETAB, 9, 1.0 + m.picker.tab as f32 * 38.0 + 16.0, 260.0, Bezier::new(0.3, 0.7, 0.2, 1.0));
        tabs = tabs.child(El::block().abs(lx - 10.0, f32::NAN, f32::NAN, -1.0).size(20.0, 2.0).radius(1.0).bg(ETAB_ON));
    }
    // .egrid{height:244px;overflow-y:auto;margin-right:-4px;padding-right:4px}: every section in one scroll (or the results)
    let grid = egrid(m, cx);
    root.child(El::col().w(338.0).child(ehd).child(tabs).child(grid))
}

/// The section list's boxes and their tops (for the tabs).
pub fn sections(m: &Model) -> Vec<(String, Vec<String>)> {
    let q = m.picker.search.trim();
    if !q.is_empty() {
        let hits = m.search_hits();
        if hits.is_empty() {
            return Vec::new();
        }
        let n = hits.len();
        return vec![(format!("{} {}", n, if n == 1 { "result" } else { "results" }), hits.iter().map(|e| e.to_string()).collect())];
    }
    let mut v = vec![("Most used".to_string(), m.recent.0.clone())];
    for c in emoji::categories() {
        v.push((c.name.to_string(), c.list.iter().map(|e| e.e.to_string()).collect()));
    }
    v
}

/// The top of each section inside the list (CSS px): .esh margin 8 3 3, 14 px tall; .egr rows 34 + 2 gap; .esec stacks.
pub fn section_tops(m: &Model) -> Vec<f32> {
    let mut y = 0.0;
    let mut tops = Vec::new();
    for (_, list) in sections(m) {
        tops.push(y);
        let rows = list.len().div_ceil(9) as f32;
        y += 8.0 + 14.0 + 3.0 + rows * 34.0 + (rows - 1.0).max(0.0) * 2.0;
    }
    tops.push(y);
    tops
}

fn egrid(m: &Model, cx: &mut Cx) -> El {
    const GH: f32 = 244.0;
    let secs = sections(m);
    let tops = section_tops(m);
    let content_h = *tops.last().unwrap_or(&0.0);
    let scroll = m.picker.scroll.clamp(0.0, (content_h - GH).max(0.0));
    // the scrollbar (8 px) only takes room when the list is taller than the box
    let inner_w = if content_h > GH { 330.0 } else { 338.0 };
    let mut inner = El::col().w(inner_w).translate(0.0, -scroll);
    if secs.is_empty() {
        // .enone{padding:56px 0;text-align:center;font:12.5px/18px;color:var(--tfg);opacity:.7}
        inner = inner.child(El::row().justify(JustifyContent::CENTER).pad(56.0, 0.0, 56.0, 0.0).child(t("No emoji found", 12.5, 400, TFG().mul_a(0.7), 18.0)));
    }
    let mut n = 0usize;
    for (si, (name, list)) in secs.iter().enumerate() {
        let top = tops[si];
        // .esh{margin:8px 3px 3px;font:600 11px/14px;color:var(--tfg);opacity:.62}
        let mut sec = El::col().child(t(name, 11.0, 600, TFG().mul_a(0.62), 14.0).margin(8.0, 3.0, 3.0, 3.0));
        // .egr{display:grid;grid-template-columns:repeat(9,34px);gap:2px}
        let mut gr = El::grid().gap(2.0).style(|s| s.grid_template_columns = vec![taffy::prelude::repeat(9, vec![taffy::prelude::length(34.0)])]);
        for (i, e) in list.iter().enumerate() {
            let ry = top + 25.0 + (i / 9) as f32 * 36.0;
            let visible = ry + 34.0 > scroll - 40.0 && ry < scroll + GH + 40.0;
            if visible {
                gr = gr.child(eb(cx, idx(K_EB, n), e, m.emoji == *e, true));
            } else {
                gr = gr.child(El::block().size(34.0, 34.0));
            }
            n += 1;
        }
        sec = sec.child(gr);
        inner = inner.child(sec);
    }
    // the scrollbar (::-webkit-scrollbar{width:8px} thumb rgba(128,128,128,.42), radius 4, a 2 px transparent border)
    let mut grid = El::block().key(K_EGRID).size(342.0, GH).margin(0.0, -4.0, 0.0, 0.0).clip().child(inner);
    if content_h > GH {
        let track = GH;
        let len = (track * GH / content_h).max(16.0).round();
        let ty = ((track - len) * scroll / (content_h - GH)).round();
        grid = grid.child(El::block().abs(342.0 - 8.0 + 2.0, ty + 2.0, f32::NAN, f32::NAN).size(4.0, len - 4.0).radius(2.0).bg(c(128, 128, 128, 0.42)).no_hit());
    }
    grid
}

/// One emoji button (`.cemj .eb{34x34;border-radius:8px;font:21px/34px "Segoe UI Emoji";text-align:center}` :hover{background:var(--thv);
/// transform:scale(1.14)} .eb.on{background:rgba(10,132,255,.26);box-shadow:inset 0 0 0 1px rgba(10,132,255,.55)}).
fn eb(cx: &mut Cx, k: Key, e: &str, on: bool, live_hover: bool) -> El {
    let hv = if live_hover { cx.hover_t(k, 140.0, EASE) } else { 0.0 };
    // Order 045: `title:(EKW[em]||'').split(' ').slice(0,2).join(' ')` (its first two words; none = no name)
    let name = emoji::words(e).split(' ').take(2).collect::<Vec<_>>().join(" ");
    let mut b = El::block().on_click(k).size(34.0, 34.0).none().radius(8.0).cursor(crate::ui::el::Cursor::Hand);
    if !name.is_empty() {
        b = b.title(&name);
    }
    if on {
        b = b.bg(c(10, 132, 255, 0.26)).inset(&[sh(0.0, 0.0, 0.0, 1.0, c(10, 132, 255, 0.55))]);
    } else if hv > 0.0 {
        b = b.bg(THV().mul_a(hv));
    }
    b.scale(1.0 + 0.14 * hv).child(emoji_el(e, 21.0, 34.0, 34.0, 1.0))
}

// ================================================================ painting
/// Paint a scene: `bg` (the frozen picture of this monitor, or nothing while Live) and the back, then the front with its glass
/// blurring what is under it. The surface is the monitor's size in device pixels.
pub fn paint(g: &Gfx, icons: &Icons, surf: &mut sk::Surface, sc: &Scene, bg: Option<&sk::Image>) -> (Laid, Laid) {
    let back = Laid::new(g, sc.back.clone(), sc.w, Some(sc.h));
    let front = Laid::new(g, sc.front.clone(), sc.w, Some(sc.h));
    g.begin(surf.canvas());
    surf.canvas().clear(sk::Color::TRANSPARENT);
    if let Some(img) = bg {
        // the frozen picture 1:1 in device pixels
        let cv = surf.canvas();
        cv.save();
        cv.reset_matrix();
        cv.draw_image(img, (0, 0), None);
        cv.restore();
    }
    back.paint(g, icons, 0.0, 0.0, None);
    g.end();
    let base = surf.image_snapshot();
    g.begin(surf.canvas());
    // every glass part's backdrop first (they never overlap each other), with its own blur and saturation
    for gl in &sc.glass {
        if let Some(n) = front.nodes.iter().find(|n| n.el.key == Some(gl.key)) {
            let e = &n.el;
            if e.opacity <= 0.0 {
                continue;
            }
            let (x, y, w, h) = n.rect;
            let r = if e.radius >= RADIUS_PILL { w.min(h) / 2.0 } else { e.radius };
            let fx: Vec<CssColor> = gl.sat.map(|s| vec![CssColor::Saturate(s)]).unwrap_or_default();
            let parent_op = front_opacity(&front, n);
            if parent_op < 1.0 {
                g.push_layer(parent_op, None);
            }
            let (tx, ty) = front_translate(&front, n);
            let t0 = g.transform();
            g.set_transform(&(Matrix3x2::translation(tx, ty) * t0));
            g.backdrop(&base, x, y, w, h, r, gl.blur, &fx);
            g.set_transform(&t0);
            if parent_op < 1.0 {
                g.pop_layer();
            }
        }
    }
    front.paint(g, icons, 0.0, 0.0, None);
    g.end();
    (back, front)
}

/// The opacity a node gets from itself and its ancestors.
fn front_opacity(l: &Laid, n: &crate::ui::lay::Node) -> f32 {
    let mut op = n.el.opacity;
    let mut p = n.parent;
    while let Some(i) = p {
        op *= l.nodes[i].el.opacity;
        p = l.nodes[i].parent;
    }
    op
}

fn front_translate(l: &Laid, n: &crate::ui::lay::Node) -> (f32, f32) {
    let (mut x, mut y) = n.el.translate;
    let mut p = n.parent;
    while let Some(i) = p {
        x += l.nodes[i].el.translate.0;
        y += l.nodes[i].el.translate.1;
        p = l.nodes[i].parent;
    }
    (x, y)
}

/// Order 054 (tests + test copies: `ovclick`): every clickable box of a laid-out front - its key and centre (DIPs) - that is
/// shown and not covered by another part (a scrolled-away emoji, the picker over the bar).
pub fn buttons(l: &Laid) -> Vec<(Key, (f32, f32))> {
    let mut v = Vec::new();
    for n in &l.nodes {
        let Some(k) = n.el.key.filter(|_| n.el.click) else { continue };
        let (rx, ry, rw, rh) = n.rect;
        if rw <= 0.0 || rh <= 0.0 || front_opacity(l, n) <= 0.0 {
            continue;
        }
        let (tx, ty) = front_translate(l, n);
        let c = (rx + tx + rw / 2.0, ry + ty + rh / 2.0);
        if l.hit(c.0, c.1).and_then(|(j, _)| l.clickable(j)) == Some(k) {
            v.push((k, c));
        }
    }
    v
}

/// Which corner handle is at a DIP point of this monitor (within its 8 px dot + 2 px).
pub fn handle_at(m: &Model, mi: usize, x: f32, y: f32) -> Option<Corner> {
    let sel = m.sel?;
    if m.mode != Mode::Edit {
        return None;
    }
    let mo = &m.monitors[mi];
    let s = mo.dpi as f32 / 96.0;
    let (sx, sy, sw, shh) = to_dip(mo, s, sel);
    [(Corner::Tl, sx - 0.5, sy - 0.5), (Corner::Tr, sx + sw + 0.5, sy - 0.5), (Corner::Bl, sx - 0.5, sy + shh + 0.5), (Corner::Br, sx + sw + 0.5, sy + shh + 0.5)]
        .into_iter()
        .find(|(_, hx, hy)| (x - hx).abs() <= 6.0 && (y - hy).abs() <= 6.0)
        .map(|(c, _, _)| c)
}

/// The keys of the front parts that eat the pointer (the bars, the toolbar, the picker, ×) - their boxes are "chrome".
pub fn is_bars(k: Key) -> bool {
    k == K_TB || k == K_EMJ
}

#[allow(dead_code)]
fn unused(_: Shadow, _: Rgba) {
    let _ = (EASE_IN, sub(K_CAP, ""));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::cx::State;
    use bu_screenshot::fake::mon;
    use bu_screenshot::geom;

    const T: f64 = 10_000.0;

    /// The front of monitor `mi` as the window lays it out (transitions settled).
    fn front_of(g: &Gfx, m: &Model, mi: usize) -> Laid {
        let mut st = State::default();
        for _ in 0..2 {
            let mut cx = Cx::new(T, false, g, &mut st);
            let _ = scene(m, &mut cx, mi, T);
        }
        let mut cx = Cx::new(T, false, g, &mut st);
        let sc = scene(m, &mut cx, mi, T);
        Laid::new(g, sc.front, sc.w, Some(sc.h))
    }

    fn states(dpi: u32) -> Vec<(String, Model)> {
        let mons = || {
            let mut v = vec![mon(0, 0, 1920, 1080, true, false), mon(1920, 0, 3440, 1440, false, false)];
            for m in &mut v {
                m.dpi = dpi;
            }
            geom::number_monitors(&mut v);
            v
        };
        // the drawing's states (100 %, 1920 x 1080 and 3440 x 1440)
        let mut out: Vec<(String, Model)> = Vec::new();
        if dpi == 96 {
            out.extend(super::super::proof::states().into_iter().map(|(n, m)| (n.to_string(), m)));
            out.extend(super::super::proof::states_3440().into_iter().map(|(n, m)| (format!("{n}@3440"), m)));
        }
        // Order 041: a click picks the whole monitor - the toolbar then lies inside the box, under the tool's own pointer
        for tool in [None, Some(Tool::Arrow), Some(Tool::Box), Some(Tool::Pen), Some(Tool::Hl), Some(Tool::Text), Some(Tool::Emoji)] {
            for mi in [0usize, 1] {
                let r = mons()[mi].rect;
                let p = (r.x as f32 + 400.0, r.y as f32 + 300.0);
                let mut m = Model::new(mons(), p, 0.0, None);
                m.press(p, None, 10.0);
                m.release(10.0);
                if let Some(t) = tool {
                    m.pick_tool(t, 20.0);
                }
                out.push((format!("whole{mi}-{tool:?}@{dpi}"), m));
            }
        }
        out
    }

    /// Order 054 (the owner: "i cant click on any of the buttons inside of screenshot"): with the pointer ON a button - and the
    /// crosshair / cross / ring / emoji drawn there, as the window draws it when the last move didn't count as "over the
    /// toolbar" - the press finds that button, on every button of every state, at 100 % and 150 %, on both monitors.
    #[test]
    fn every_button_takes_the_click_under_its_own_pointer() {
        let mut checked = 0;
        let mut seen = std::collections::HashSet::new();
        for dpi in [96, 144] {
            for (name, mut m) in states(dpi) {
                for mi in 0..m.monitors.len() {
                    let mo = m.monitors[mi].clone();
                    let g = Gfx::new(mo.dpi as f32 / 96.0);
                    let s = g.scale;
                    m.moved((-100_000.0, -100_000.0), false);
                    let away = front_of(&g, &m, mi);
                    for (k, c) in buttons(&away) {
                        // the pointer on the button, not known as "over the toolbar": its own pointer is drawn under it
                        m.moved((mo.rect.x as f32 + c.0 * s, mo.rect.y as f32 + c.1 * s), false);
                        let l = front_of(&g, &m, mi);
                        let got = l.hit(c.0, c.1).and_then(|(j, _)| l.clickable(j));
                        assert_eq!(got, Some(k), "{name} monitor {mi}: a press on button {k:#x} at {c:?} (pointer {:?}) went elsewhere", m.pointer());
                        checked += 1;
                        seen.insert(k);
                    }
                }
            }
        }
        for k in [K_X, K_SZ, K_LIVE, K_SNAP, K_UNDO, K_COPY, K_SAVE, K_MORE, K_BACK, idx(K_TOOL, 0), idx(K_TOOL, 5), idx(K_MON, 0), idx(K_ETAB, 1)] {
            assert!(seen.contains(&k), "button {k:#x} never checked");
        }
        assert!(checked > 300, "only {checked} presses checked");
    }
}
