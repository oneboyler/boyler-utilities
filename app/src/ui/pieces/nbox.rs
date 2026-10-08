//! The text fields (Order 025): `.nbox` + its `<input>` (menu-v22: Display's number fields, Mouse's DPI fields, the Wi-Fi
//! password field) and the well field `.dnsf input` (Network's Custom DNS form, with its red `.bad` error ring). The page
//! keeps the text; typing reaches it as `Ev::Char(key, c)` / `Ev::Key(key, vk)` while the field has focus (a click on it) -
//! `type_char` / `edit_key` apply them.
//!
//! Other text inputs of the drawing are NOT these pieces (each has its own CSS): Timers' name / time `.thn` `.thd`, Mouse's
//! preset name `.apin`, Keys' text `.kdin`, Screenshots' `.ctxt` / size field - PIECES_WANTED lines of their own.

use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Align, Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{Cursor, El, Key};
use crate::ui::{cmix, ACC, ACC_S, CTL, CTL_H, FG, FG3, HAIR, ICO_ON, RED, WELL};

/// The drawing's EASE_OUT (`cubic-bezier(.2,.8,.2,1)`), used by the snap nudge and the step cue.
const EASE_OUT: Bezier = crate::anim::EASE_OUT;

/// Chromium's own active-selection colours (the drawing has no `::selection` rule), measured off-screen with a real
/// `focus()` + `select()`: an opaque rgb(10,62,172) box, white text.
pub const SEL_BG: Rgba = Rgba::rgb(10, 62, 172);
/// The browser's placeholder colour for an input with no `::placeholder` rule (`.dnsf input`): #757575, measured.
pub const PLACEHOLDER: Rgba = Rgba::rgb(117, 117, 117);

/// Which `.nbox` size.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Size {
    /// `.nbox` - 32 px high, radius 8, `font:600 15px/32px` (Display's width / height / rate fields)
    Md,
    /// `.nbox.sm` - 26 px high, radius 7, `font:600 12.5px/26px` (Mouse "Custom" DPI, the Wi-Fi password)
    Sm,
    /// `.nbox.xs` - 20 px high, radius 5, `font:600 11px/20px` (Mouse acceleration "at … DPI")
    Xs,
}

/// A text field's look. Use the drawing's presets (`NUM`, `HZ`, `SM`, `XS`, `FORM`) - each is one class combination of
/// the drawing that a v22 page uses.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Opts {
    pub size: Size,
    /// the `<input>`'s width (border-box, `*{box-sizing:border-box}`)
    pub input_w: f32,
    /// `.wfx .nbox input{text-align:left;padding:0 9px;font-size:12.5px}` - else `text-align:center;padding:0`
    pub left: bool,
    /// `.wfx .nbox{flex:1;max-width:220px}` - the box grows in a row up to this width (0 = the input's width)
    pub grow_max: f32,
    /// `type=password`: every character shows as a bullet
    pub password: bool,
}

/// `.nbox input{width:66px}` (Display's width / height fields)
pub const NUM: Opts = Opts { size: Size::Md, input_w: 66.0, left: false, grow_max: 0.0, password: false };
/// `.nbox.hz input{width:58px}` (Display's refresh rate; it takes digits and '.': `Filter::Decimal`)
pub const HZ: Opts = Opts { size: Size::Md, input_w: 58.0, left: false, grow_max: 0.0, password: false };
/// `.nbox.sm input{width:64px}` (Mouse "Custom" DPI)
pub const SM: Opts = Opts { size: Size::Sm, input_w: 64.0, left: false, grow_max: 0.0, password: false };
/// `.nbox.sm.tbdu input{width:56px}` - a rule among Timers' list rules (`.tblist .tbr`); no v22 element uses it
pub const SM_TBDU: Opts = Opts { size: Size::Sm, input_w: 56.0, left: false, grow_max: 0.0, password: false };
/// `.nbox.xs input{width:44px}` (Mouse acceleration's DPI)
pub const XS: Opts = Opts { size: Size::Xs, input_w: 44.0, left: false, grow_max: 0.0, password: false };
/// `.wfx .nbox.sm` + `type=password` (Network's Wi-Fi password): the box grows to 220 px; the input keeps
/// `.nbox.sm input{width:64px}` (same specificity as `.wfx .nbox input{width:100%}`, later in the sheet - Chromium's
/// computed width is 64 px) with `text-align:left;padding:0 9px`.
pub const FORM: Opts = Opts { size: Size::Sm, input_w: 64.0, left: true, grow_max: 220.0, password: true };

/// How long the snap cue lasts (the drawing's `setTimeout(…,650)`).
pub const SNAP_MS: f64 = 650.0;
/// The step cue (arrow keys / wheel on Display's fields): 180 ms.
pub const STEP_MS: f64 = 180.0;

/// The field's moment-to-moment state the page keeps (all off = `Cue::NONE`).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Cue {
    /// when the page snapped a typed value to a real one (`snapCue`: accent colour 650 ms + a 2 px nudge)
    pub snap_at: Option<f64>,
    /// when the value was stepped and which way (+1 up / -1 down): `F.step` - the new value fades in from .2 and slides
    /// from ±5 px, 180 ms EASE_OUT
    pub step: Option<(f64, i8)>,
    /// the whole value is selected (Display's fields and Mouse's Custom DPI select it on focus: `inp.select()`); the first
    /// typed key replaces it (`type_char`)
    pub selected: bool,
}

impl Cue {
    pub const NONE: Cue = Cue { snap_at: None, step: None, selected: false };
}

/// The `.nbox` text field.
///
/// `.nbox{position:relative;display:flex;align-items:center;height:32px;border-radius:8px;background:var(--ctl);
///   box-shadow:inset 0 0 0 .5px var(--hair);transition:background-color .15s ease}` `:hover{background:var(--ctl-h)}`
/// `::after{inset:0;border-radius:inherit;box-shadow:0 0 0 3px var(--acc-s),inset 0 0 0 1px var(--acc);opacity:0;
///   transition:opacity .15s ease}` `:focus-within::after{opacity:1}`
/// `.nbox input{width:66px;height:100%;padding:0;color:var(--fg);text-align:center;font:600 15px/32px "Segoe UI Variable Text";
///   font-variant-numeric:tabular-nums;letter-spacing:-.005em;cursor:text;transition:color .5s ease}`
/// `.nbox.sm{height:26px;border-radius:7px}` `.nbox.sm input{width:64px;height:26px;font:600 12.5px/26px}`
/// `.nbox.sm input::placeholder{color:var(--fg3);font-weight:400}`
/// `.nbox.xs{height:20px;border-radius:5px}` `.nbox.xs input{width:44px;height:20px;font:600 11px/20px}`
/// `.nbox.snap input{color:var(--ico-on);transition:color .06s ease}` + the nudge `translateX(0 → 2px @30% → 0)` 240 ms
/// EASE_OUT (`snapCue`); the step cue `{opacity:.2,translateY(±5px)} → {1, 0}` 180 ms EASE_OUT (`F.step`).
pub fn nbox(cx: &mut Cx, key: Key, text: &str, placeholder: &str, o: &Opts, cue: &Cue) -> El {
    let (h, r, fs): (f32, f32, f32) = match o.size {
        Size::Md => (32.0, 8.0, 15.0),
        Size::Sm => (26.0, 7.0, 12.5),
        Size::Xs => (20.0, 5.0, 11.0),
    };
    let fs = if o.left { 12.5 } else { fs };
    // letter-spacing:-.005em of the input's own font size, in 1/1000 px
    let ls = (-0.005 * fs * 1000.0).round() as i32;
    let hv = cx.hover_t(key, 150.0, EASE);
    let focused = cx.focused(key);
    let ring = cx.tr(key, 3, if focused { 1.0 } else { 0.0 }, 150.0, EASE);
    let age = cue.snap_at.map(|t| cx.now - t).unwrap_or(f64::INFINITY);
    let snapping = (0.0..SNAP_MS).contains(&age);
    let sv = cx.tr(key, 4, if snapping { 1.0 } else { 0.0 }, if snapping { 60.0 } else { 500.0 }, EASE);
    let mut dx = 0.0;
    if (0.0..240.0).contains(&age) && !cx.rm {
        let p = EASE_OUT.ease(age / 240.0) as f32;
        dx = if p < 0.3 { 2.0 * p / 0.3 } else { 2.0 * (1.0 - p) / 0.7 };
    }
    if snapping {
        cx.st.busy = true;
    }
    // the step cue (no motion under reduced motion, like the drawing's `if(!RM)`)
    let (mut step_op, mut step_dy) = (1.0, 0.0);
    if let Some((t, dir)) = cue.step {
        let a = cx.now - t;
        if (0.0..STEP_MS).contains(&a) && !cx.rm {
            let p = EASE_OUT.ease(a / STEP_MS) as f32;
            step_op = 0.2 + 0.8 * p;
            step_dy = if dir > 0 { 5.0 } else { -5.0 } * (1.0 - p);
            cx.st.busy = true;
        }
    }
    let shown: String = if text.is_empty() {
        placeholder.to_string()
    } else if o.password {
        "\u{2022}".repeat(text.chars().count())
    } else {
        text.to_string()
    };
    // `.nbox input{font:600 15px/32px …;font-variant-numeric:tabular-nums}` - but `.nbox.sm input` / `.nbox.xs input` set the
    // `font` SHORTHAND again, which resets font-variant-numeric to normal: only the 32 px field has tabular digits
    let tnum = o.size == Size::Md && !o.left;
    let mk = |w: u16| if tnum { Font::new(fs, w).tnum().ls(ls) } else { Font::new(fs, w).ls(ls) };
    let selected = focused && cue.selected && !text.is_empty();
    let (font, col) = if text.is_empty() {
        (mk(400), FG3())
    } else if selected {
        (mk(600), Rgba::rgb(255, 255, 255))
    } else {
        (mk(600), cmix(FG(), ICO_ON(), sv))
    };
    let pad = if o.left { 9.0 } else { 0.0 };
    let inner = o.input_w - 2.0 * pad;
    let tb = cx.g.text_box(&shown, font, 0.0);
    // where the text starts in the input: centred = Blink's line offset (available - the shaped width) / 2 floored to a
    // LayoutUnit (measured: 0 px differ); left = the padding, scrolled left while focused so the caret stays in view (Blink
    // scrolls its inner editor)
    let tx = if o.left {
        let scroll = if focused && !text.is_empty() { (tb.width + 1.0 - inner).max(0.0) } else { 0.0 };
        -scroll
    } else {
        (((inner - tb.raw) / 2.0) * 64.0).floor() / 64.0
    };
    // the text is clipped at the input's CONTENT box (Blink's inner editor), not its padding box
    let mut content = El::block().w(inner).h(h).none().clip().no_hit();
    if selected {
        // the selection box: from the snapped text start to its snapped end, the font's ascent + descent tall around the
        // baseline, its edges snapped outwards (measured: x 16..50, y 6..26 on "1920" in the 66 x 32 field)
        let (a, d) = cx.g.asc_desc(font);
        let top = cx.g.baseline(font, h) - a;
        let (x0, x1) = (tx.floor(), (tx + tb.width).ceil());
        content = content.child(El::block().abs(x0, top, f32::NAN, f32::NAN).size(x1 - x0, a + d).bg(SEL_BG));
    }
    if !shown.is_empty() {
        let mut t = El::text(shown.clone(), font, col, h).abs(tx, 0.0, f32::NAN, f32::NAN);
        if o.left {
            t = t.align(Align::Left);
        }
        if step_op < 1.0 {
            t = t.opacity(step_op).translate(0.0, step_dy);
        }
        content = content.child(t);
    }
    if focused && !selected {
        // the caret after the text (blinking 530 ms on / off, like the search field); its height is a guess (not proven)
        let caret_on = ((cx.now / 530.0) as i64) % 2 == 0;
        cx.st.busy = true;
        if caret_on {
            let tw = if text.is_empty() { 0.0 } else { tb.width };
            let x = if text.is_empty() && !o.left { inner / 2.0 } else { (tx + tw).min(inner - 1.0) };
            let ch = (fs * 1.12).round();
            content = content.child(El::block().abs(x.round(), ((h - ch) / 2.0).round(), f32::NAN, f32::NAN).size(1.0, ch).bg(FG()).no_hit());
        }
    }
    let inp = El::block().w(o.input_w).h(h).none().pad(0.0, pad, 0.0, pad).no_hit().child(content);
    let mut b = El::row()
        .center()
        .h(h)
        .radius(r)
        .bg(cmix(CTL(), CTL_H(), hv))
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
        .translate(dx, 0.0)
        .key(key)
        .cursor(Cursor::Text)
        .child(inp);
    if o.grow_max > 0.0 {
        b = b.grow(1.0).shrink(1.0).max_w(o.grow_max);
    } else {
        b = b.none();
    }
    if ring > 0.001 {
        b = b.child(
            El::block()
                .abs(0.0, 0.0, 0.0, 0.0)
                .radius(r)
                .shadow(&[sh(0.0, 0.0, 0.0, 3.0, ACC_S())])
                .inset(&[sh(0.0, 0.0, 0.0, 1.0, ACC())])
                .opacity(ring)
                .no_hit(),
        );
    }
    b
}

/// The well field: Network's Custom DNS form (`.dnsf input`), as wide as its parent.
///
/// `.dnsf input{width:100%;height:26px;border:0;border-radius:6px;padding:0 8px;background:var(--well);color:var(--fg);
///   font:12px/26px "Cascadia Mono","Consolas",monospace;box-shadow:inset 0 0 0 .5px var(--hair);outline:none}`
/// `.dnsf input:focus{box-shadow:inset 0 0 0 1px var(--acc),0 0 0 3px var(--acc-s)}` `.dnsf input.bad{box-shadow:inset 0 0 0
/// 1px var(--red)}` (`.bad` wins over `:focus`: same specificity, later). `bad` = the drawing's check failed (the page sets
/// it on Save and clears it on the next typed key). The placeholder has the browser's own colour (#757575, measured) and
/// the input the browser's `letter-spacing: normal`. No transitions (instant).
pub fn well(cx: &mut Cx, key: Key, text: &str, placeholder: &str, bad: bool) -> El {
    let focused = cx.focused(key);
    let font = Font::new(12.0, 400).ls(0).mono();
    let (shown, col) = if text.is_empty() { (placeholder, PLACEHOLDER) } else { (text, FG()) };
    let (insets, shadows) = if bad {
        (vec![sh(0.0, 0.0, 0.0, 1.0, RED())], vec![])
    } else if focused {
        (vec![sh(0.0, 0.0, 0.0, 1.0, ACC())], vec![sh(0.0, 0.0, 0.0, 3.0, ACC_S())])
    } else {
        (vec![sh(0.0, 0.0, 0.0, 0.5, HAIR())], vec![])
    };
    let mut content = El::block().flex1().h(26.0).clip().no_hit();
    let tw = if text.is_empty() { 0.0 } else { cx.g.text_box(text, font, 0.0).width };
    if !shown.is_empty() {
        content = content.child(El::text(shown, font, col, 26.0).abs(0.0, 0.0, f32::NAN, f32::NAN));
    }
    if focused && ((cx.now / 530.0) as i64) % 2 == 0 {
        cx.st.busy = true;
        content = content.child(El::block().abs(tw.round(), 6.0, f32::NAN, f32::NAN).size(1.0, 14.0).bg(FG()).no_hit());
    } else if focused {
        cx.st.busy = true;
    }
    El::row()
        .w_pct(100.0)
        .h(26.0)
        .radius(6.0)
        .pad(0.0, 8.0, 0.0, 8.0)
        .bg(WELL())
        .inset(&insets)
        .shadow(&shadows)
        .key(key)
        .cursor(Cursor::Text)
        .child(content)
}

/// Which characters a field takes (the drawing's `value.replace(…)` on input).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Filter {
    /// `/\D/g` - Display's width / height, Mouse DPI
    Digits,
    /// `/[^\d.]/g` - Display's refresh rate
    Decimal,
    /// anything printable (the Wi-Fi password, DNS addresses)
    Any,
}

/// Apply a typed character, keeping at most `max_len` characters (the input's `maxlength`). While the whole value is
/// `selected` (select-on-focus) the first accepted key REPLACES it, as in the browser; `selected` is cleared.
pub fn type_char(text: &mut String, selected: &mut bool, c: char, max_len: usize, f: Filter) {
    let ok = match f {
        Filter::Digits => c.is_ascii_digit(),
        Filter::Decimal => c.is_ascii_digit() || c == '.',
        Filter::Any => !c.is_control(),
    };
    if !ok {
        return;
    }
    if *selected {
        text.clear();
        *selected = false;
    }
    if text.chars().count() < max_len {
        text.push(c);
    }
}

/// Apply a typed key: Backspace deletes the last character (all of it while `selected`). Enter / Esc / arrows are the
/// page's (commit / cancel / step).
pub fn edit_key(text: &mut String, selected: &mut bool, vk: u16) {
    if vk == 0x08 {
        if *selected {
            text.clear();
            *selected = false;
        } else {
            text.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_follows_maxlength_filters_and_selection() {
        // Display's width field: "1920" selected on focus, the first key replaces it
        let (mut t, mut sel) = (String::from("1920"), true);
        for c in "25a604".chars() {
            type_char(&mut t, &mut sel, c, 4, Filter::Digits);
        }
        assert_eq!(t, "2560");
        assert!(!sel);
        edit_key(&mut t, &mut sel, 0x08);
        assert_eq!(t, "256");
        // a full field without selection takes no more digits
        let (mut f, mut s2) = (String::from("1920"), false);
        type_char(&mut f, &mut s2, '5', 4, Filter::Digits);
        assert_eq!(f, "1920");
        // the refresh rate takes '.'
        let (mut hz, mut s3) = (String::from("144"), true);
        for c in "59.94".chars() {
            type_char(&mut hz, &mut s3, c, 6, Filter::Decimal);
        }
        assert_eq!(hz, "59.94");
        // Backspace on a selected value clears it
        let (mut b, mut s4) = (String::from("800"), true);
        edit_key(&mut b, &mut s4, 0x08);
        assert_eq!(b, "");
        let (mut p, mut s5) = (String::new(), false);
        type_char(&mut p, &mut s5, 'x', 64, Filter::Any);
        type_char(&mut p, &mut s5, '\n', 64, Filter::Any);
        assert_eq!(p, "x");
    }
}
