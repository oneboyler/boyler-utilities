//! The search field (menu-v22 `.tsrch`, Tweaks / Apps headers; `.tsrch.psr` the small one). The page keeps the text;
//! typing reaches it as `Ev::Char(key, c)` / `Ev::Key(key, vk)` while the field has focus (a click on it).

use crate::anim::EASE;
use crate::gfx::{sh, Font};
use crate::ui::cx::Cx;
use crate::ui::el::{sub, Cursor, El, Key};
use crate::ui::{cmix, ACC, ACC_S, CTL, CTL_H, FG, FG3, HAIR};

/// `.tsrch{display:flex;align-items:center;width:196px;height:28px;border-radius:8px;background:var(--ctl);
///   box-shadow:inset 0 0 0 .5px var(--hair);transition:background-color .15s ease}` `:hover{background:var(--ctl-h)}`
/// `::after{box-shadow:0 0 0 3px var(--acc-s),inset 0 0 0 1px var(--acc);opacity:0;transition:opacity .15s ease}`
/// `:focus-within::after{opacity:1}` `>i{width:28px;height:28px;color:var(--fg3)} svg{13px;stroke-width:1.5}`
/// `.tsq{flex:1;height:28px;padding:0 4px 0 0;font:12.5px/28px}` `::placeholder{color:var(--fg3)}`
/// `.tsx{width:20px;height:20px;margin-right:4px;border-radius:50%;color:var(--fg3)}` (shown when there is text;
/// `:hover{background:var(--ctl-h);color:var(--fg)}`, svg 8 px) - a click on it = `Ev::Click(sub(key, "x"))`.
/// `small` = `.tsrch.psr` (168 x 24, 12 px text).
pub fn search(cx: &mut Cx, key: Key, text: &str, placeholder: &str, small: bool) -> El {
    let (w, h, iw, fs) = if small { (168.0, 24.0, 26.0, 12.0) } else { (196.0, 28.0, 28.0, 12.5) };
    let hv = cx.hover_t(key, 150.0, EASE);
    let focused = cx.focused(key);
    let ring = cx.tr(key, 3, if focused { 1.0 } else { 0.0 }, 150.0, EASE);
    let font = Font::new(fs, 400).ls(0);
    let shown = if text.is_empty() { placeholder } else { text };
    let col = if text.is_empty() { FG3() } else { FG() };
    // the input box: the text (or placeholder) and, while focused, the caret beside it - a SIBLING of the text, never its
    // child (a text box with a child is no longer measured as text: the text fell below the field, REVIEW_020 HOLD 1)
    let mut q = El::row().center().flex1().h(h).child(El::text(shown, font, col, h).ellipsis().flex1().pad(0.0, 4.0, 0.0, 0.0));
    if focused {
        // the caret after the text (blinking: 530 ms on / off)
        let caret_on = ((cx.now / 530.0) as i64) % 2 == 0;
        cx.st.busy = true;
        let tw = if text.is_empty() { 0.0 } else { cx.g.text_width(text, font) };
        if caret_on {
            q = q.child(El::block().abs(tw.round(), (h - 14.0) / 2.0, f32::NAN, f32::NAN).size(1.0, 14.0).bg(FG()).no_hit());
        }
    }
    let mut f = El::row()
        .center()
        .w(w)
        .h(h)
        .none()
        .radius(8.0)
        .bg(cmix(CTL(), CTL_H(), hv))
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
        .key(key)
        .cursor(Cursor::Text)
        .child(El::block().size(iw, h).none().place_center().no_hit().child(El::icon("search", 13.0, 1.5, FG3())))
        .child(q);
    if !text.is_empty() {
        let xk = sub(key, "x");
        let xh = cx.hover_t(xk, 150.0, EASE);
        f = f.child(
            El::block()
                .size(20.0, 20.0)
                .none()
                .margin(0.0, 4.0, 0.0, 0.0)
                .radius(10.0)
                .bg(CTL_H().mul_a(xh))
                .place_center()
                .on_click(xk)
                .cursor(Cursor::Hand)
                // every `.tsx`: `title:'Clear'`
                .title("Clear")
                .child(El::icon("x", 8.0, 1.5, cmix(FG3(), FG(), xh)).no_hit()),
        );
    }
    if ring > 0.001 {
        f = f.child(
            El::block()
                .abs(0.0, 0.0, 0.0, 0.0)
                .radius(8.0)
                .shadow(&[sh(0.0, 0.0, 0.0, 3.0, ACC_S())])
                .inset(&[sh(0.0, 0.0, 0.0, 1.0, ACC())])
                .opacity(ring)
                .no_hit(),
        );
    }
    f
}

/// Apply a typed key to a search text: Backspace deletes the last character, Esc clears (the page also drops focus).
pub fn edit_key(text: &mut String, vk: u16) {
    match vk {
        0x08 => {
            text.pop();
        }
        0x1B => text.clear(),
        _ => {}
    }
}

/// Apply a typed character to a search text (control characters are ignored).
pub fn edit_char(text: &mut String, c: char) {
    if !c.is_control() {
        text.push(c);
    }
}
