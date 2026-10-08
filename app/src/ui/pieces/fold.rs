//! The fold card's head (menu-v22 `foldCard`, Order 025 batch 8): the clickable card head `.row.ch.ex` (Security's Threats
//! found / Quarantine, Network's Wi-Fi networks), its count badge `.fcnt` and the chevron button `.cx` that turns 180° open.
//! The card around it is Lane K's: `card::card(cx, key, fold::head(..), Some(body), open, width)` (the `.xp` drop-out).

use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{lh, Cursor, El, Key};
use crate::ui::{ACC, ACC_S, CTL, CTL_H, FG, FG2, HAIR, HOV, SEL};

const TURN: Bezier = Bezier::new(0.3, 0.7, 0.2, 1.0);

/// The chevron button: `open` = turned 180° (.3 s; at once under reduced motion), `gone` = faded out and not clickable
/// (`.cx.gone`, .2 s). Click = `Ev::Click(key)` (the page folds the card; in the drawing the whole head folds it).
///
/// `.cx{width:28px;height:28px;flex:none;margin:0 -6px 0 2px;border-radius:6px;background:transparent;display:grid;
///   place-items:center;color:var(--fg2);transition:opacity .2s ease,background-color .15s ease,color .15s ease}`
/// `.cx:hover{background:var(--ctl-h);color:var(--fg)}` `.cx svg{width:12px;height:12px;stroke-width:1.5;transition:transform
///   .3s cubic-bezier(.3,.7,.2,1)}` `.cx.open svg{transform:rotate(180deg)}` `.cx.gone{opacity:0;pointer-events:none}` (`ICON.chevDw`)
pub fn chev(cx: &mut Cx, key: Key, open: bool, gone: bool) -> El {
    let hv = if gone { 0.0 } else { cx.hover_t(key, 150.0, EASE) };
    let rot = if cx.rm { if open { 1.0 } else { 0.0 } } else { cx.tr(key, 1, if open { 1.0 } else { 0.0 }, 300.0, TURN) };
    let op = cx.tr(key, 2, if gone { 0.0 } else { 1.0 }, 200.0, EASE);
    let b = El::block()
        .size(28.0, 28.0)
        .none()
        .margin(0.0, -6.0, 0.0, 2.0)
        .radius(6.0)
        .bg(CTL_H().mul_a(hv))
        .place_center()
        .opacity(op)
        .child(El::icon("chevDw", 12.0, 1.5, FG()).rotate(180.0 * rot).no_hit());
    if gone {
        b.no_hit()
    } else {
        b.on_click(key).cursor(Cursor::Hand)
    }
}

/// The count badge: `red` = something needs attention (Threats found > 0).
///
/// `.fcnt{min-width:22px;height:20px;padding:0 7px;border-radius:10px;background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair);
///   color:var(--fg2);font-size:11.5px;font-weight:600;line-height:20px;text-align:center}`
/// `.fcnt.red{background:rgba(255,69,58,.18);color:#ff6b61;box-shadow:inset 0 0 0 .5px rgba(255,69,58,.4)}`
pub fn fcnt(text: &str, red: bool) -> El {
    let (bg, rim, col) = if red { (Rgba::rgba(255, 69, 58, 0.18), Rgba::rgba(255, 69, 58, 0.4), Rgba::rgb(255, 107, 97)) } else { (CTL(), HAIR(), FG2()) };
    El::row()
        .min_w(22.0)
        .h(20.0)
        .none()
        .pad(0.0, 7.0, 0.0, 7.0)
        .radius(10.0)
        .bg(bg)
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, rim)])
        .justify(taffy::style::JustifyContent::CENTER)
        .child(El::text(text, Font::new(11.5, 600), col, 20.0))
}

/// The card icon `.ci` (32 px, `--sel` with the accent icon); `bad` = `.card.secc.bad .ci` (red).
fn ci(icon: &str, bad: bool) -> El {
    let (bg, rim, col) = if bad { (Rgba::rgba(255, 69, 58, 0.16), Rgba::rgba(255, 69, 58, 0.4), Rgba::rgb(255, 107, 97)) } else { (SEL(), ACC_S(), ACC()) };
    El::block().size(32.0, 32.0).none().radius(8.0).bg(bg).inset(&[sh(0.0, 0.0, 0.0, 0.5, rim)]).place_center().child(El::icon(icon, 18.0, 1.5, col).no_hit())
}

/// The clickable head of a fold card: icon, title (+ `extra` after it in the title line, e.g. a `.rq` tip icon), the line
/// under it, then `right` (the `fcnt` badge and the `chev` button). `secc` = Security's cards (`.card.secc`: 58 px, title gap
/// 6), `bad` = `.card.secc.bad` (the red icon). A click anywhere on it = `Ev::Click(key)` (the drawing ignores clicks on its
/// `.rq` - give that its own key). Hover: `--hov` (.15 s).
///
/// `.row{display:flex;align-items:center;gap:12px;padding:7px 12px}` `.ch{min-height:60px;padding:10px 12px}` `.ch.ex{cursor:pointer;
///   transition:background-color .15s ease}` `.ch.ex:hover{background:var(--hov)}` `.ch .lbl.ap{display:flex;align-items:center;gap:12px}`
/// `.ct{display:flex;align-items:center;gap:7px;font-size:13px;font-weight:600}` `.lbl small{display:block;font-size:11px;
///   color:var(--fg2);margin-top:1px}` `.ctl{display:flex;align-items:center;gap:8px}` `.card.secc .ch{min-height:58px}`
///   `.card.secc .ct{gap:6px}`
#[allow(clippy::too_many_arguments)]
pub fn head(cx: &mut Cx, key: Key, icon: &str, title: &str, extra: Vec<El>, small: &str, right: Vec<El>, secc: bool, bad: bool) -> El {
    let hv = cx.hover_t(key, 150.0, EASE);
    let ct = El::row().center().gap(if secc { 6.0 } else { 7.0 }).child(El::text(title, Font::new(13.0, 600), FG(), lh(13.0, 1.35))).children(extra);
    let words = El::col().child(ct).child(El::text(small, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0));
    let lbl = El::row().center().gap(12.0).flex1().min_w(0.0).child(ci(icon, bad)).child(words);
    El::row()
        .center()
        .gap(12.0)
        .min_h(if secc { 58.0 } else { 60.0 })
        .pad(10.0, 12.0, 10.0, 12.0)
        .bg(HOV().mul_a(hv))
        .key(key)
        .on_click(key)
        .cursor(Cursor::Hand)
        .child(lbl)
        .child(El::row().center().gap(8.0).none().children(right))
}
