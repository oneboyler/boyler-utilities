//! The dropdown button (menu-v22 `.pu`) and its popup list (`.menu` + `.mitem`): the page keeps which list is open and
//! returns `menu(...)` from `Page::popup`, anchored under the button (`Laid::rect_of` / the button's `Ev::Press` box).

use taffy::style::AlignItems;

use crate::anim::EASE;
use crate::gfx::{sh, Align, Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{idx, lh, Cursor, El, Key};
use crate::ui::{cmix, ACC, CTL, CTL_H, FG, FG2, HAIR, HL_V19, POP, WHITE, WIN_H, WIN_W};

use super::btn_font;

/// `.pu{display:flex;align-items:center;gap:6px;height:24px;max-width:240px;padding:0 6px 0 10px;border-radius:6px;
///   background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair),0 .5px 1px rgba(0,0,0,.12);font-size:13px;
///   transition:background .15s,transform .12s ease}` `.pu:hover{background:var(--ctl-h)}` `.pu:active{transform:scale(.97)}`
/// `.pu span{flex:1;min-width:0;text-align:left;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}`
/// `.pu svg{width:9px;height:14px;stroke:var(--fg2);stroke-width:1.6}`. `width` = a fixed width (`.pu.w` 128, `.pu.dev` 196)
/// or None (as wide as its text, at most 240).
pub fn dropdown(cx: &mut Cx, key: Key, label: &str, width: Option<f32>) -> El {
    let hv = cx.hover_t(key, 150.0, EASE);
    let pr = cx.active_t(key, 120.0, EASE);
    let mut b = El::row()
        .center()
        .gap(6.0)
        .h(24.0)
        .max_w(width.unwrap_or(240.0))
        .pad(0.0, 6.0, 0.0, 10.0)
        .radius(6.0)
        .bg(cmix(CTL(), CTL_H(), hv))
        .shadow(&[sh(0.0, 0.5, 1.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.12))])
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
        .scale(1.0 - 0.03 * pr)
        .on_click(key)
        .cursor(Cursor::Hand)
        .child(El::text(label, btn_font(13.0, 400), FG(), lh(13.0, 1.35)).ellipsis().flex1_auto())
        .child(El::icon("chev", 9.0, 1.6, FG2()).h(14.0).no_hit());
    if let Some(w) = width {
        b = b.w(w).none();
    }
    b
}

/// One line of a popup list.
#[derive(Clone, Debug)]
pub struct Item {
    pub label: String,
    /// the tick (✓) in front
    pub checked: bool,
    pub disabled: bool,
}

/// The popup list at (x, y) in window coordinates, opened `open_t` 0..1 (`opacity:0;transform:scale(.97)` -> 1, .12 s
/// ease, from its top-left). `.menu{min-width:150px;max-height:300px;padding:5px;border-radius:10px;background:var(--pop);
/// backdrop-filter:blur(30px) saturate(180%);box-shadow:inset 0 0 0 .5px var(--hl),0 0 0 .5px rgba(0,0,0,.35),
/// 0 12px 32px rgba(0,0,0,.35)}` `.mitem{display:flex;align-items:center;gap:6px;height:26px;padding:0 14px 0 6px;
/// border-radius:5px;font-size:13px}` `.mitem .ck{width:14px;font-size:12px;text-align:center}`
/// `.mitem:hover{background:var(--acc);color:#fff}`. A click on item i = `Ev::Click(idx(key, i))`.
pub fn menu(cx: &mut Cx, key: Key, items: &[Item], x: f32, y: f32, min_w: f32) -> El {
    let rows: Vec<El> = items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let k = idx(key, i);
            let on = !it.disabled && cx.hovered(k);
            let col = if it.disabled { crate::ui::FG3() } else if on { WHITE } else { FG() };
            let ck = El::text(if it.checked { "\u{2713}" } else { "" }, Font::new(12.0, 400), col, lh(12.0, 1.35)).w(14.0).none().align(Align::Center);
            let mut r = El::row()
                .center()
                .gap(6.0)
                .h(26.0)
                .pad(0.0, 14.0, 0.0, 6.0)
                .radius(5.0)
                .child(ck)
                .child(El::text(it.label.clone(), Font::new(13.0, 400), col, lh(13.0, 1.35)));
            if on {
                r = r.bg(ACC());
            }
            if !it.disabled {
                r = r.on_click(k);
            } else {
                // `#sw .mitem.dis{opacity:.38;pointer-events:none}` + `.mitem.dis{color:var(--fg3)}`
                r = r.key(k).opacity(0.38).no_hit();
            }
            r
        })
        .collect();
    // kept inside the window (the drawing's placeMenu: 8 px from the edges)
    let est_h = 10.0 + 26.0 * items.len() as f32;
    menu_box(cx, key, x, y, min_w, est_h, 300.0, rows)
}

/// The popup list's box (`.menu`) at (x, y) with its content, kept 8 px inside the window; `est_h` = its height (to
/// keep it inside), `max_h` = 300 for lists (none for `.menu.rsm`). Opens with opacity / scale .97 -> 1 (.12 s ease).
#[allow(clippy::too_many_arguments)]
pub fn menu_box(cx: &mut Cx, key: Key, x: f32, y: f32, min_w: f32, est_h: f32, max_h: f32, kids: Vec<El>) -> El {
    let t = cx.tr(key, 1, 1.0, 120.0, EASE);
    // a list taller than max_h scrolls inside it (`.menu{max-height:300px;overflow-y:auto}`): its rows keep their height
    // - as flex items of the capped box they shrank instead (the owner Oct 8: the Output list "looks crazy weird ... make it
    // scrollable instead of making it so tiny")
    let est_h = est_h.min(max_h);
    let y = if y + est_h > WIN_H - 8.0 { (WIN_H - 8.0 - est_h).max(8.0) } else { y };
    let kids: Vec<El> = kids.into_iter().map(|k| k.none()).collect();
    let list = cx
        .scroll_box(crate::ui::el::sub(key, "scr"), kids)
        .items(AlignItems::STRETCH)
        .slim_thumb(crate::ui::el::SlimThumb::GLASS)
        .style(|s| {
            s.flex_shrink = 1.0;
            s.min_size.height = taffy::style::LengthPercentageAuto::length(0.0);
        });
    El::col()
        .abs(x.min(WIN_W - 8.0 - min_w).max(8.0), y, f32::NAN, f32::NAN)
        .min_w(min_w.max(150.0))
        .max_h(max_h)
        .pad_all(5.0)
        .radius(10.0)
        .bg(POP())
        .backdrop(30.0, 1.8)
        .shadow(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, 0.35)), sh(0.0, 12.0, 32.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.35))])
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, crate::ui::hl())])
        .items(AlignItems::STRETCH)
        .opacity(t)
        .scale(0.97 + 0.03 * t)
        .key(key)
        .child(list)
}

/// A `.pu` with its own content (the Controller header's game / controller pickers: `.pu.pdg{max-width:none;gap:7px;
/// padding-left:4px}`, `.pu.pdc{max-width:none;gap:7px}`): `kids` then the chevron; `pad_left` / `gap` from the variant.
pub fn dropdown_with(cx: &mut Cx, key: Key, kids: Vec<El>, pad_left: f32, gap: f32) -> El {
    let hv = cx.hover_t(key, 150.0, EASE);
    let pr = cx.active_t(key, 120.0, EASE);
    El::row()
        .center()
        .gap(gap)
        .h(24.0)
        .none()
        .pad(0.0, 6.0, 0.0, pad_left)
        .radius(6.0)
        .bg(cmix(CTL(), CTL_H(), hv))
        .shadow(&[sh(0.0, 0.5, 1.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.12))])
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
        .scale(1.0 - 0.03 * pr)
        .on_click(key)
        .cursor(Cursor::Hand)
        .children(kids)
        .child(El::icon("chev", 9.0, 1.6, FG2()).h(14.0).no_hit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::Gfx;
    use crate::ui::cx::State;
    use crate::ui::el::{key, sub};
    use crate::ui::lay::Laid;

    /// the owner Oct 8 (the Output list with many devices "looks crazy weird ... make it scrollable instead of making it so
    /// tiny"): a list taller than the box's 300 px keeps every row's height and scrolls inside the box.
    #[test]
    fn a_long_list_keeps_its_rows_and_scrolls() {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        let rows: Vec<El> = (0..12).map(|i| El::block().h(32.0).key(crate::ui::el::idx(key("t.row"), i))).collect();
        let m = menu_box(&mut cx, key("t.menu"), 20.0, 20.0, 300.0, 5.0 + 12.0 * 32.0, 300.0, rows);
        let l = Laid::new(&g, El::block().w(WIN_W).h(WIN_H).child(m), WIN_W, Some(WIN_H));
        for i in 0..12 {
            let r = l.rect_of(crate::ui::el::idx(key("t.row"), i)).unwrap();
            assert!((r.3 - 32.0).abs() < 0.01, "row {i} squeezed to {}", r.3);
        }
        let b = l.rect_of(key("t.menu")).unwrap();
        assert!(b.3 <= 300.01, "the box stays 300 tall: {}", b.3);
        let (k, max) = l.scroll_box_in(&[sub(key("t.menu"), "scr")]).expect("it scrolls");
        assert_eq!(k, sub(key("t.menu"), "scr"));
        assert!(max > 80.0, "the rows below can be scrolled to: {max}");
    }
}
