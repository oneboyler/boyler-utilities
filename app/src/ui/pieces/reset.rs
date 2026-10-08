//! The per-tab reset line at the bottom of a page ("Back to how your PC was · Windows defaults", menu-v22 `.rsl`) and its
//! review popup (`.menu.rsm` + `.rsf`: a title, a line, the ticked list `.rsi` of what will change, the buttons). The
//! undo / reset framework (Order 014 item 2) gives the lines; the page shows them with these.

use crate::anim::EASE;
use crate::gfx::Font;
use crate::ui::cx::Cx;
use crate::ui::el::{idx, lh, sub, Cursor, El, Key, RADIUS_PILL};
use crate::ui::{cmix, ACC, CTL, DASH, FG, FG2, FG3, HOV, WHITE};

use super::link::link;

/// `.rsl{display:flex;align-items:center;justify-content:center;gap:8px;margin:22px 0 2px;font-size:12px;color:var(--fg3);
///   white-space:nowrap}` `.rsl .lnk{font-size:12px}` `.rsl i{width:3px;height:3px;border-radius:50%;background:var(--fg3);opacity:.7}`
/// "Reset this page" + "Back to how your PC was" (= `Ev::Click(sub(key, "pc"))`) and, where the tab has them, the dot +
/// "Windows defaults" (or the tab's own name for it, e.g. Controller's) = `Ev::Click(sub(key, "win"))`.
pub fn reset_line(cx: &mut Cx, key: Key, win: Option<&str>) -> El {
    let mut r = El::row()
        .center()
        .justify(taffy::style::JustifyContent::CENTER)
        .gap(8.0)
        .margin(22.0, 0.0, 2.0, 0.0)
        .child(El::text("Reset this page", Font::new(12.0, 400), FG3(), lh(12.0, 1.35)).none())
        .child(link(cx, sub(key, "pc"), "Back to how your PC was", 12.0));
    if let Some(w) = win {
        r = r.child(El::block().size(3.0, 3.0).none().radius(RADIUS_PILL).bg(FG3()).opacity(0.7)).child(link(cx, sub(key, "win"), w, 12.0));
    }
    r
}

/// One line of the review list.
#[derive(Clone, Debug)]
pub struct Line {
    pub title: String,
    /// "from -> to" as the drawing writes it (`<em>` = the new value)
    pub from: String,
    pub to: String,
    pub ticked: bool,
    /// a section heading above this line (`.rsh`), e.g. "Changed by this app"
    pub heading: Option<String>,
}

/// The tick box `.tkb` (18 x 18 hit area, 15 x 15 box): `i{width:15px;height:15px;border-radius:4px;background:var(--ctl);
///   box-shadow:inset 0 0 0 1px var(--dash)}` `.on i{background:var(--acc);box-shadow:none}` `svg{10px;stroke:#fff;stroke-width:1.7;
///   opacity:0;transform:scale(.6)}` `.on svg{opacity:1;transform:scale(1)}`.
pub fn tick(cx: &mut Cx, key: Key, on: bool) -> El {
    let t = cx.tr(key, 1, if on { 1.0 } else { 0.0 }, 150.0, EASE);
    let mut b = El::block().size(15.0, 15.0).radius(4.0).bg(cmix(CTL(), ACC(), t)).place_center();
    if t < 0.999 {
        b = b.inset(&[crate::gfx::sh(0.0, 0.0, 0.0, 1.0, DASH().mul_a(1.0 - t))]);
    }
    b = b.child(El::icon("tkcheck", 10.0, 1.7, WHITE).opacity(t).scale(0.6 + 0.4 * t));
    El::block().size(18.0, 18.0).none().place_center().child(b)
}

/// The review popup's content (inside the shared popup list box `menu`, `.menu.rsm{max-height:none;overflow:visible}`):
/// `.rsf{width:316px;padding:5px 6px 4px}` `b{font-size:13px;font-weight:600;line-height:17px}`
/// `p{margin:3px 0 8px;font-size:11.5px;line-height:15px;color:var(--fg2)}` `.rsl2{max-height:196px;overflow-y:auto}`
/// `.rsh{margin:7px 0 3px;font-size:10.5px;font-weight:600;color:var(--fg3);letter-spacing:.02em}`
/// `.rsi{display:flex;align-items:flex-start;gap:8px;padding:5px 2px;border-radius:6px}` `:hover{background:var(--hov)}`
/// `.rsi div{font-size:12px;line-height:16px}` `small{font-size:11px;line-height:14px;color:var(--fg3)} em{color:var(--fg2)}`
/// `.rsi.off div{opacity:.5}`. Line i = `Ev::Click(idx(key, i))` (tick / untick); the buttons are the page's
/// (Cancel / Reset): `buttons`.
pub fn review(cx: &mut Cx, key: Key, title: &str, text: &str, lines: &[Line], buttons: Vec<El>) -> El {
    let mut list = El::col().max_h(196.0).clip().margin(0.0, -6.0, 0.0, -6.0).pad(0.0, 6.0, 0.0, 6.0);
    for (i, l) in lines.iter().enumerate() {
        if let Some(h) = &l.heading {
            list = list.child(El::text(h.clone(), Font::new(10.5, 600).ls(210), FG3(), lh(10.5, 1.35)).margin(7.0, 0.0, 3.0, 0.0));
        }
        let k = idx(key, i);
        let hv = cx.hover_t(k, 0.0, EASE);
        let small = El::row()
            .child(El::text(format!("{} \u{2192} ", l.from), Font::new(11.0, 400), FG3(), 14.0).none())
            .child(El::text(l.to.clone(), Font::new(11.0, 400), FG2(), 14.0).ellipsis());
        let body = El::col()
            .flex1()
            .opacity(if l.ticked { 1.0 } else { 0.5 })
            .child(El::text(l.title.clone(), Font::new(12.0, 400), FG(), 16.0).ellipsis())
            .child(small);
        list = list.child(
            El::row()
                .items(taffy::style::AlignItems::FLEX_START)
                .gap(8.0)
                .pad(5.0, 2.0, 5.0, 2.0)
                .radius(6.0)
                .bg(HOV().mul_a(hv))
                .on_click(k)
                .cursor(Cursor::Hand)
                .child(tick(cx, sub(k, "t"), l.ticked).margin(-1.0, 0.0, 0.0, 0.0))
                .child(body),
        );
    }
    El::col()
        .w(316.0)
        .pad(5.0, 6.0, 4.0, 6.0)
        .child(El::text(title, Font::new(13.0, 600), FG(), 17.0))
        .child(El::text(text, Font::new(11.5, 400), FG2(), 15.0).wrapping().margin(3.0, 0.0, 8.0, 0.0))
        .child(list)
        // `.mcfb{display:flex;justify-content:flex-end;gap:6px}` `.rsf .mcfb{margin-top:10px}`
        .child(El::row().justify(taffy::style::JustifyContent::FLEX_END).gap(6.0).margin(10.0, 0.0, 0.0, 0.0).children(buttons))
}

/// The review in its popup list box, at (x, y) (under the link that opened it: the drawing's placeMenu(btn, 316)).
#[allow(clippy::too_many_arguments)]
pub fn review_popup(cx: &mut Cx, key: Key, x: f32, y: f32, title: &str, text: &str, lines: &[Line], buttons: Vec<El>) -> El {
    let est = 80.0 + 40.0 * lines.len().min(5) as f32;
    let body = review(cx, key, title, text, lines, buttons);
    super::dropdown::menu_box(cx, sub(key, "box"), x, y, 316.0, est, 10000.0, vec![body])
}
