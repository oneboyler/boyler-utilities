//! The Controller page's own boxes (classes only this page has in the drawing: `.prw`, `.asr` in `.pdx`, `.psec`,
//! `.ach`, `.pg2`, `.pdcp`, `.cpx`, `.lsw`, `.pwl`, `.pdtr`, `.pnote`, `.pdlive`, `.pdset`), built from `El` with the
//! drawing's CSS quoted on each.

use std::cell::RefCell;
use std::rc::Rc;

use bu_controller::Side;
use taffy::style::{AlignItems, JustifyContent};

use crate::anim::EASE;
use crate::gfx::{sh, Align, Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{lh, sub, Cursor, El, Key, RADIUS_PILL};
use crate::ui::pieces::{self, ibtn, slider};
use crate::ui::{cmix, ACC, CTL, CTL_H, FG, FG2, FG3, GREEN, HAIR, KEY, SEL, WELL, WHITE};

/// The pad glyphs `PG` (12 x 12 viewBox, stroked) as path data (circles / rects written as paths).
pub fn pg_path(name: &str) -> Option<&'static str> {
    Some(match name {
        "x" => "M3 3l6 6M9 3L3 9",
        "o" => "M9.7 6A3.7 3.7 0 1 1 2.3 6A3.7 3.7 0 1 1 9.7 6Z",
        "sq" => "M3.2 2.6H8.8Q9.4 2.6 9.4 3.2V8.8Q9.4 9.4 8.8 9.4H3.2Q2.6 9.4 2.6 8.8V3.2Q2.6 2.6 3.2 2.6Z",
        "tri" => "M6 2.2l4 7H2z",
        "du" => "M6 9.5v-7M3.3 5.2L6 2.5l2.7 2.7",
        "dd" => "M6 2.5v7M3.3 6.8L6 9.5l2.7-2.7",
        "dl" => "M9.5 6h-7M5.2 3.3L2.5 6l2.7 2.7",
        "dr" => "M2.5 6h7M6.8 3.3L9.5 6 6.8 8.7",
        "create" => "M3.5 2.8v6.4M6 2.8v6.4M8.5 2.8v6.4",
        "options" => "M2.6 3.6h6.8M2.6 6h6.8M2.6 8.4h6.8",
        "mic" => "M6 1.6A1.6 1.6 0 0 1 7.6 3.2V5.4A1.6 1.6 0 0 1 4.4 5.4V3.2A1.6 1.6 0 0 1 6 1.6ZM2.8 5.8a3.2 3.2 0 0 0 6.4 0M6 9v1.5",
        _ => return None,
    })
}

/// `.pg2{display:inline-grid;place-items:center;width:14px;height:14px;color:var(--fg2)}`
/// `.pg2 svg{width:12px;height:12px;fill:none;stroke:currentColor;stroke-width:1.4;round caps / joins}`
/// The glyph is the shared `El::icon_svg` (12 px, stroke 1.4) centred in the `box_size` square.
pub fn pg2(name: &str, box_size: f32, color: Rgba) -> El {
    let d = pg_path(name).unwrap_or("");
    let src = format!(r#"<svg viewBox="0 0 12 12"><path d="{d}"/></svg>"#);
    El::block().size(box_size, box_size).none().place_center().no_hit().child(El::icon_svg(&src, 12.0, 1.4, color).no_hit())
}

/// How an action shows (`acNodes`): keys as a keycap, pad buttons with their glyph, nothing = a quiet "Nothing".
#[derive(Clone, Debug, PartialEq)]
pub enum ActShow {
    Nothing,
    Key(String),
    Pad(Option<&'static str>, String),
    Text(String),
}

/// `.ach>span{display:flex;align-items:center;gap:5px}` + its parts (`.none{color:var(--fg3)}`); `same` = `.ach.same`
/// (`>span{color:var(--fg2)}` `.pg2{opacity:.8}`).
pub fn act_nodes(a: &ActShow, size: f32, same: bool) -> Vec<El> {
    let col = if same { FG2() } else { FG() };
    let f = pieces::btn_font(size, 400);
    match a {
        ActShow::Nothing => vec![El::text("Nothing", f, FG3(), lh(size, 1.35)).none()],
        // `.ach .kc`: the small keycap in a button chip (letter-spacing normal)
        ActShow::Key(k) => vec![ibtn::keycap(k, &ibtn::CAP_SM_BTN, false)],
        ActShow::Pad(g, t) => {
            let mut v = Vec::new();
            if let Some(g) = g {
                v.push(pg2(g, 14.0, FG2()).opacity(if same { 0.8 } else { 1.0 }));
            }
            v.push(El::text(t.clone(), f, col, lh(size, 1.35)).none());
            v
        }
        ActShow::Text(t) => vec![El::text(t.clone(), f, col, lh(size, 1.35)).ellipsis()],
    }
}

/// The "does" chip: `#sw .ach{display:inline-flex;align-items:center;gap:6px;height:24px;max-width:100%;padding:0 6px 0 9px;
/// border-radius:6px;background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair),0 .5px 1px rgba(0,0,0,.12);font-size:12.5px;
/// transition:background .15s,transform .12s ease}` `:hover{background:var(--ctl-h)}` `:active{transform:scale(.97)}`
/// `.ach>i svg{width:9px;height:14px;stroke:var(--fg2);stroke-width:1.6}`.
pub fn ach(cx: &mut Cx, key: Key, a: &ActShow, same: bool, disabled: bool) -> El {
    let hv = cx.hover_t(key, 150.0, EASE);
    let pr = cx.active_t(key, 120.0, EASE);
    let mut b = El::row()
        .center()
        .gap(6.0)
        .h(24.0)
        .none()
        .pad(0.0, 6.0, 0.0, 9.0)
        .radius(6.0)
        .bg(cmix(CTL(), CTL_H(), hv))
        .shadow(&[sh(0.0, 0.5, 1.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.12))])
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
        .scale(1.0 - 0.03 * pr)
        .child(El::row().center().gap(5.0).shrink(1.0).clip().children(act_nodes(a, 12.5, same)))
        .child(El::icon("chev", 9.0, 1.6, FG2()).h(14.0).no_hit());
    if disabled {
        b = b.opacity(0.38).key(key);
    } else {
        b = b.on_click(key).cursor(Cursor::Hand);
    }
    b
}

/// `.prw{display:flex;flex-wrap:wrap;align-items:center;column-gap:10px;row-gap:2px;min-height:30px;padding:2px 0}`
/// `.prw>span:first-child{width:92px;flex:none;font-size:12px;color:var(--fg2);white-space:nowrap}` (the panel's
/// `.pdl4.sel .cpn .prw>span:first-child{width:112px}`, Controller settings `.pdvd ... {width:150px}`)
/// `.prw>.ctl2{flex:1 0 auto;max-width:100%;display:flex;align-items:center;gap:8px}` `.prw.dim{opacity:.4;pointer-events:none}`
pub fn prw(label: &str, lw: f32, ctl: Vec<El>, dim: bool, tail: Option<El>) -> El {
    let mut r = El::row()
        .wrap()
        .center()
        .gap2(2.0, 10.0)
        .min_h(30.0)
        .pad(2.0, 0.0, 2.0, 0.0)
        .child(El::text(label, Font::new(12.0, 400), FG2(), lh(12.0, 1.35)).w(lw).none())
        .child(El::row().center().gap(8.0).grow(1.0).shrink(0.0).children(ctl))
        .children(tail);
    if dim {
        r = r.opacity(0.4);
    }
    r
}

/// One slider row `.asr` in `.pdx`: `.asr{display:flex;align-items:center;gap:10px}` `.pdx .asr{height:30px}` label like `.prw`,
/// `.asr .rng{flex:1;min-width:0}` (the slider piece at the row's width), `.pdx .asr .sv{min-width:44px}`.
/// `step01` = one step of the setting as a part of the slider (an arrow key moves it one step: `<input type=range step>`).
#[allow(clippy::too_many_arguments)]
pub fn asr(cx: &mut Cx, key: Key, label: &str, lw: f32, row_w: f32, v01: f32, step01: f32, value: &str, dim: bool, tail: Option<El>) -> El {
    let svw = cx.g.text_width(value, Font::new(12.0, 400).tnum()).max(44.0);
    let tw = if tail.is_some() { TAIL_W + 10.0 } else { 0.0 };
    let rw = (row_w - lw - 20.0 - svw - tw).max(20.0);
    let mut r = El::row()
        .center()
        .gap(10.0)
        .h(30.0)
        .child(El::text(label, Font::new(12.0, 400), FG2(), lh(12.0, 1.35)).w(lw).none())
        .child({
            let mut s = slider::slider(cx, key, v01, rw, 20.0, slider::default()).on_click(key);
            s.range = Some((v01, step01));
            s
        })
        .child(slider::value_label(value).min_w(44.0))
        .children(tail);
    if dim {
        r = r.opacity(0.4);
    }
    r
}

/// The width of a row's end slot (Order 042 item 9): the "Undo" link + the reset.
pub const TAIL_W: f32 = 50.0;

/// A row's end slot: the "Undo" link (`undo` = its key; the row holds the last change) and the small reset (`back` = its
/// key + tip; the value differs from Steam's), each at a fixed place - the link left of the reset's 16 px square - so
/// neither moves when the other comes or goes. The reset: `.dlx`-like (transparent, `--ctl-h` on hover), the drawing's
/// `treset` glyph 12 px in `--fg3` (`--fg` on hover).
pub fn tail(cx: &mut Cx, undo: Option<Key>, back: Option<(Key, &str)>) -> El {
    let mut t = El::row().center().justify(JustifyContent::FLEX_END).gap(4.0).w(TAIL_W).h(24.0).none();
    if let Some(k) = undo {
        t = t.child(pieces::link::link(cx, k, "Undo", 11.5));
    }
    match back {
        Some((k, tip)) => {
            let hv = cx.hover_t(k, 150.0, EASE);
            t = t.child(
                El::block()
                    .size(16.0, 16.0)
                    .none()
                    .place_center()
                    .radius(4.0)
                    .bg(CTL_H().mul_a(hv))
                    .on_click(k)
                    .cursor(Cursor::Hand)
                    .tip(tip)
                    .child(El::icon("treset", 12.0, 1.4, cmix(FG3(), FG(), hv)).no_hit()),
            );
        }
        None => t = t.child(El::block().size(16.0, 16.0).none()),
    }
    t
}

/// `.psec` + `.psh{margin:2px 0 3px;font-size:11px;font-weight:600;color:var(--fg3);letter-spacing:.02em}`;
/// `.psec+.psec,.pfold+.psec{margin-top:10px}` (`mt`).
pub fn psec(title: &str, mt: f32, kids: Vec<El>) -> El {
    El::col()
        .items(AlignItems::STRETCH)
        // the heading's 2 px top margin collapses with the section's own (block flow): max(mt, 2)
        .margin(mt.max(2.0), 0.0, 0.0, 0.0)
        .child(El::text(title, Font::new(11.0, 600).ls(220), FG3(), lh(11.0, 1.35)).margin(0.0, 0.0, 3.0, 0.0))
        .children(kids)
}

/// The panel's first section: its heading's 2 px top margin collapses into the panel head's 6 px bottom margin (block flow).
pub fn psec_first(title: &str, kids: Vec<El>) -> El {
    El::col()
        .items(AlignItems::STRETCH)
        .margin(-2.0, 0.0, 0.0, 0.0)
        .child(El::text(title, Font::new(11.0, 600).ls(220), FG3(), lh(11.0, 1.35)).margin(2.0, 0.0, 3.0, 0.0))
        .children(kids)
}

/// `.pnote{display:flex;gap:7px;align-items:flex-start;margin-top:8px;font-size:11px;line-height:15px;color:var(--fg3)}`
/// `.pnote svg{width:13px;height:13px;margin-top:1px;stroke:var(--fg3);stroke-width:1.4}`
pub fn pnote(text: &str) -> El {
    El::row()
        .items(AlignItems::FLEX_START)
        .gap(7.0)
        .margin(8.0, 0.0, 0.0, 0.0)
        .child(El::icon("info", 13.0, 1.4, FG3()).margin(1.0, 0.0, 0.0, 0.0))
        .child(El::text(text, Font::new(11.0, 400), FG3(), 15.0).wrapping().flex1())
}

/// The Gyro chip: `#sw .pdcp{display:inline-flex;align-items:center;gap:6px;height:26px;padding:0 11px 0 8px;border-radius:13px;
/// background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair);font-size:12px;color:var(--fg)}` `:hover{background:var(--ctl-h)}`
/// `:active{transform:scale(.97)}` `.on{background:var(--sel);box-shadow:inset 0 0 0 1.5px var(--acc)}`
/// `.pdcp svg{width:14px;stroke:var(--fg2);stroke-width:1.4}` (`.on` --acc) `.pdcp small{font-size:12px;color:var(--fg2)}`
pub fn pdcp(cx: &mut Cx, key: Key, icon: &str, label: &str, small: &str, on: bool) -> El {
    let hv = cx.hover_t(key, 150.0, EASE);
    let pr = cx.active_t(key, 120.0, EASE);
    let o = cx.tr(key, 1, if on { 1.0 } else { 0.0 }, 150.0, EASE);
    let bg = cmix(cmix(CTL(), CTL_H(), hv), SEL(), o);
    let ring = if o > 0.5 { sh(0.0, 0.0, 0.0, 1.5, ACC()) } else { sh(0.0, 0.0, 0.0, 0.5, HAIR()) };
    El::row()
        .center()
        .gap(6.0)
        .h(26.0)
        .none()
        .pad(0.0, 11.0, 0.0, 8.0)
        .radius(13.0)
        .bg(bg)
        .inset(&[ring])
        .scale(1.0 - 0.03 * pr)
        .on_click(key)
        .cursor(Cursor::Hand)
        .child(El::icon(icon, 14.0, 1.4, cmix(FG2(), ACC(), o)).no_hit())
        .child(El::text(label, pieces::btn_font(12.0, 400), FG(), lh(12.0, 1.35)).none())
        .child(El::text(small, pieces::btn_font(12.0, 400), FG2(), lh(12.0, 1.35)).none())
}

/// The light colours (`LCOL`): Steam blue, red, green, purple, white, off.
pub const LCOL: [(&str, Option<(u8, u8, u8)>); 6] =
    [("Steam blue", Some((59, 130, 255))), ("Red", Some((255, 69, 58))), ("Green", Some((48, 209, 88))), ("Purple", Some((164, 99, 255))), ("White", Some((242, 242, 247))), ("Off", None)];

/// The colour swatches `.lsw{display:flex;align-items:center;gap:8px}` `#sw .lsw button{width:20px;height:20px;border-radius:50%;
/// box-shadow:inset 0 0 0 .5px rgba(0,0,0,.3);transition:transform .12s ease,box-shadow .15s ease}` `:hover{transform:scale(1.1)}`
/// `.on{box-shadow:inset 0 0 0 .5px rgba(0,0,0,.3),0 0 0 2px rgba(0,0,0,.35),0 0 0 3.5px var(--acc)}`
/// `.off{background:transparent;box-shadow:inset 0 0 0 1px var(--fg3)}`. Swatch i = `Ev::Click(idx(key, i))`.
pub fn lsw(cx: &mut Cx, key: Key, on: Option<usize>) -> El {
    let mut r = El::row().center().gap(8.0).none();
    for (i, (name, c)) in LCOL.iter().enumerate() {
        let k = crate::ui::el::idx(key, i);
        let hv = cx.hover_t(k, 120.0, EASE);
        // Order 045: `'data-tip':c[1]` (the colour's name)
        let mut b = El::block().size(20.0, 20.0).none().radius(RADIUS_PILL).scale(1.0 + 0.1 * hv).on_click(k).tip(name).cursor(Cursor::Hand);
        let inner = match c {
            Some((r, g, bl)) => {
                b = b.bg(Rgba::rgb(*r, *g, *bl));
                sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, 0.3))
            }
            None => sh(0.0, 0.0, 0.0, 1.0, FG3()),
        };
        b = b.inset(&[inner]);
        if on == Some(i) {
            // light (Order 033): `#sw.light .lsw button.on{box-shadow:...,0 0 0 2px #fff,...}` (the "off" swatch keeps its dark ring:
            // its own rule comes later)
            let gap = if crate::ui::is_light() && c.is_some() { crate::ui::WHITE } else { Rgba(0.0, 0.0, 0.0, 0.35) };
            b = b.shadow(&[sh(0.0, 0.0, 0.0, 2.0, gap), sh(0.0, 0.0, 0.0, 3.5, ACC())]);
        }
        r = r.child(b);
    }
    r
}

/// `.pdlive{margin-left:auto;display:inline-flex;align-items:center;gap:5px;height:20px;padding:0 8px;border-radius:10px;
/// background:rgba(48,209,88,.14);box-shadow:inset 0 0 0 .5px rgba(48,209,88,.3);color:#4cd964;font-size:11px;font-weight:600}`
/// `.pdlive i{width:6px;height:6px;border-radius:50%;background:#30d158;animation:vdot 1.4s}` `.off{background:var(--ctl);
/// box-shadow:inset 0 0 0 .5px var(--hair);color:var(--fg3)}` `.off i{background:var(--fg3);animation:none}`
pub fn pdlive(on: bool) -> El {
    let (bg, ring, col, dot, t) = if on {
        (Rgba::rgba(48, 209, 88, 0.14), Rgba::rgba(48, 209, 88, 0.3), Rgba::rgb(76, 217, 100), GREEN(), "Live")
    } else {
        (CTL(), HAIR(), FG3(), FG3(), "Not connected")
    };
    El::row()
        .center()
        .gap(5.0)
        .h(20.0)
        .none()
        .ml_auto()
        .pad(0.0, 8.0, 0.0, 8.0)
        .radius(10.0)
        .bg(bg)
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, ring)])
        // Order 045: `'data-tip':'Read from your controller while this tab is open · nothing is sent to it'` (the drawing keeps it
        // on `.pdlive.off` too)
        .key(crate::ui::el::key("pad.live"))
        .tip("Read from your controller while this tab is open \u{b7} nothing is sent to it")
        .child(El::block().size(6.0, 6.0).none().radius(RADIUS_PILL).bg(dot))
        .child(El::text(t, Font::new(11.0, 600).ls(0), col, lh(11.0, 1.35)).none())
}

/// The trigger's "where it clicks" bar: `.pdtr{height:10px;margin:4px 0 6px;border-radius:5px;background:var(--well);
/// box-shadow:inset 0 0 0 .5px var(--hair);overflow:hidden}` `i{background:linear-gradient(90deg,rgba(10,132,255,.06),var(--acc-s))}`
/// (width = clicks at) `b{width:2px;margin-left:-1px;background:var(--acc)}` (at it) `em{top:3px;bottom:3px;border-radius:3px;
/// background:#30d158;opacity:.75}` (the live pull: Order 055, read from the shared readings when the bar is painted).
pub fn pdtr(at: f32, src: Rc<RefCell<super::Lv>>, side: Side, live: bool) -> El {
    let e = El::paint(move |g, (x, y, w, h)| {
        let pull = {
            let lv = src.borrow();
            if side == Side::Left {
                lv.l2
            } else {
                lv.r2
            }
        };
        g.fill_rr(x, y, w, h, 5.0, WELL());
        g.push_clip(x, y, w, h);
        let fw = w * at;
        if fw > 0.0 {
            let shd = g.hgrad(x, 0.0, x + fw, 0.0, &[(0.0, Rgba::rgba(10, 132, 255, 0.06)), (1.0, crate::ui::ACC_S())]);
            g.fill_rr_shader(x, y, fw, h, 0.0, &shd, 1.0);
        }
        g.fill_rect(x + fw - 1.0, y, 2.0, h, ACC());
        if pull > 0.0 {
            g.fill_rr(x, y + 3.0, w * pull, h - 6.0, 3.0, Rgba::rgba(48, 209, 88, 0.75));
        }
        g.pop_clip();
        g.inset_shadows(x, y, w, h, 5.0, &[sh(0.0, 0.0, 0.0, 0.5, HAIR())]);
    })
    .h(10.0)
    .margin(4.0, 0.0, 6.0, 0.0);
    // repainted every frame only while a live view runs (frozen pictures / no pad: painted with the page)
    if live {
        e.live()
    } else {
        e
    }
}

/// The panel head's "Back" tag (`.tag`).
pub fn back_tag() -> El {
    pieces::badge::tag("Back", pieces::badge::Tone::Plain)
}

/// White text for an accent row.
pub const ON_ACC: Rgba = WHITE;

/// `.pdhint{position:absolute;left:50%;transform:translate(-50%,-50%);font-size:12.5px;font-weight:500;color:var(--fg3);
/// white-space:nowrap}` at `top` (px in the picture box).
pub fn pdhint(cx: &mut Cx, w: f32, top: f32, op: f32) -> El {
    let f = Font::new(12.5, 500);
    let tw = cx.g.text_width("Click buttons to edit", f);
    let line = lh(12.5, 1.35);
    El::text("Click buttons to edit", f, FG3(), line)
        .abs((w - tw) / 2.0, top - line / 2.0, f32::NAN, f32::NAN)
        .align(Align::Left)
        .opacity(op)
        .no_hit()
}

pub fn center_row(kids: Vec<El>) -> El {
    El::row().center().justify(JustifyContent::CENTER).children(kids)
}

/// A child key for a control id text.
pub fn ck(parent: Key, id: &str) -> Key {
    sub(parent, id)
}
