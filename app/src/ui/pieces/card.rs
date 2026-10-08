//! The card (menu-v22 `.card`: a group box with a head - icon tile, title, a line under it, its switch - whose settings
//! drop out under it) and the fold card (`.fgrp` + `.fold`: a group whose first row opens / closes the rest).

use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Font};
use crate::ui::cx::Cx;
use crate::ui::el::{lh, sub, Cursor, El, Key};
use crate::ui::lay::Laid;
use crate::ui::{ACC, ACC_S, FG, FG2, HOV, SEL};

use super::group::grp;

const FOLD: Bezier = Bezier::new(0.3, 0.7, 0.2, 1.0);

/// `.ci{width:32px;height:32px;border-radius:8px;background:var(--sel);box-shadow:inset 0 0 0 .5px var(--acc-s)}`
/// `.ci svg{width:18px;height:18px;stroke:var(--acc);stroke-width:1.5}`
pub fn card_icon(icon: &str) -> El {
    El::block().size(32.0, 32.0).none().radius(8.0).bg(SEL()).inset(&[sh(0.0, 0.0, 0.0, 0.5, ACC_S())]).place_center().child(El::icon(icon, 18.0, 1.5, ACC()))
}

/// The card's head: `.ch{min-height:60px;padding:10px 12px}` (a `.row`: flex, centre, gap 12) with the icon, the title
/// (`.ct{font-size:13px;font-weight:600}`) + its line (`small`, 11 px --fg2), and the right side (its switch, `.cx`).
pub fn card_head(icon: &str, title: &str, small: Option<&str>, right: Vec<El>) -> El {
    let mut l = El::col().flex1().child(El::text(title, Font::new(13.0, 600), FG(), lh(13.0, 1.35)).ellipsis());
    if let Some(s) = small {
        l = l.child(El::text(s, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0));
    }
    El::row().center().gap(12.0).min_h(60.0).pad(10.0, 12.0, 10.0, 12.0).child(card_icon(icon)).child(l).children(right)
}

/// How far a fold is open (0..1) for this frame: `.xp` grid-template-rows 0fr -> 1fr (.36 s opening, .32 s closing,
/// cubic-bezier(.3,.7,.2,1)) with its opacity (.26 s after .06 s opening, .18 s closing).
pub fn fold_t(cx: &mut Cx, key: Key, open: bool) -> (f32, f32) {
    let h = cx.tr(key, 10, if open { 1.0 } else { 0.0 }, if open { 360.0 } else { 320.0 }, FOLD);
    let o = if open { cx.tr_delayed(key, 11, 1.0, 260.0, 60.0, EASE) } else { cx.tr(key, 11, 0.0, 180.0, EASE) };
    (h, o)
}

/// The part that drops out (`.xp > .xin{min-height:0;overflow:hidden}`): `body` laid out at `width` to know its height,
/// shown at (height x h, opacity o).
pub fn drop_out(cx: &mut Cx, body: El, width: f32, (h, o): (f32, f32)) -> El {
    if h <= 0.0005 {
        return El::block().h(0.0);
    }
    let full = Laid::new(cx.g, body.clone(), width, None).height;
    El::block().h(full * h).clip().opacity(o).child(body)
}

/// A card: the group box (`overflow:hidden`) with its head, and its settings dropping out under it while `open`.
/// `.pg>.card{margin-top:12px}` (`.ph+.card` 4) is up to the page.
pub fn card(cx: &mut Cx, key: Key, head: El, settings: Option<El>, open: bool, width: f32) -> El {
    let ft = fold_t(cx, key, open);
    let mut kids = vec![head];
    if let Some(s) = settings {
        kids.push(drop_out(cx, s, width, ft));
    }
    grp(kids).clip()
}

/// The fold card: `.fgrp{margin-top:20px;overflow:hidden}` whose first row is `.fold` (`cursor:pointer;
/// transition:background-color .12s ease` `:hover{background:var(--hov)}`) with the chevron `.fchv{width:20px;height:20px}
/// svg{12px;stroke:var(--fg2);stroke-width:1.5;transition:transform .28s cubic-bezier(.3,.7,.2,1)}` turned 180° open.
/// A click on the fold row = `Ev::Click(sub(key, "fold"))`. `row_kids` = the fold row's label etc. (the chevron is added).
pub fn fold_card(cx: &mut Cx, key: Key, row_kids: Vec<El>, body: El, open: bool, width: f32) -> El {
    let fk = sub(key, "fold");
    let hv = cx.hover_t(fk, 120.0, EASE);
    let rot = cx.tr(key, 12, if open { 1.0 } else { 0.0 }, 280.0, FOLD);
    let chev = El::block().size(20.0, 20.0).none().place_center().no_hit().child(El::icon("chevD", 12.0, 1.5, FG2()).rotate(180.0 * rot));
    let fold = super::group::row(true, row_kids).child(chev).bg(HOV().mul_a(hv)).on_click(fk).cursor(Cursor::Hand);
    let ft = fold_t(cx, key, open);
    grp(vec![fold, drop_out(cx, body, width, ft)]).clip().margin(20.0, 0.0, 0.0, 0.0)
}
