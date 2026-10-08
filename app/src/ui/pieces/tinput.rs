//! The other text inputs of menu-v22 (Order 025 batch 7) - each has its own CSS, so each is its own function: Keys' text /
//! website field `.kdin`, Timers' name `.thn` and big time `.thd`, Mouse's preset rename `.apin`. Typing works as for
//! `nbox`: the page keeps the text and applies `Ev::Char` / `Ev::Key` with `nbox::type_char` / `nbox::edit_key`; `selected`
//! = the whole value is selected (the drawing's `inp.select()` on focus) and the first typed key replaces it.
//!
//! Every `<input>` here has the browser's `letter-spacing: normal` (no `#sw input{font:inherit}` rule exists) and no outline.
//! (Screenshots' `.ctxt` and size field live in Lane Q's capture overlay - single use, built there; `input.tmi` has no v22
//! element.)

use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Align, Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{Cursor, El, Key};
use crate::ui::{cmix, ACC, ACC_S, CTL, FG, FG2, FG3, HAIR, HOV, WELL};

/// Chromium's active selection is TRANSLUCENT: measured over two fills - (10,62,172) over `--ctl` (52,54,69), (4,57,166) over
/// `--well` (24,26,40) - rgba(0,65,198,.8) gives both (Blink makes the opaque highlight colour translucent at alpha .8; this
/// is the drawing as rendered here). (`nbox::SEL_BG` = the same over `--ctl`, opaque.)
const SEL: Rgba = Rgba(0.0, 65.0 / 255.0, 198.0 / 255.0, 0.8);

/// CSS `ease-in-out` (the end blink's keyframe easing).
const EASE_IN_OUT: Bezier = Bezier::new(0.42, 0.0, 0.58, 1.0);

/// Where the text goes inside the input's content box.
#[derive(Clone, Copy, PartialEq)]
enum At {
    /// `text-align:center`: Blink's line offset = (available - shaped width) / 2, floored to 1/64 px
    Centre,
    /// left, scrolled so the caret stays in view while focused (Blink scrolls its inner editor)
    Left,
}

/// The input's content: the selection box, the text (or the placeholder) and the caret, clipped at the content box (Blink's
/// inner editor). `inner` x `h` = the content box. The caret blinks 530 ms on / off after the text (its height is a guess,
/// as in `nbox`).
#[allow(clippy::too_many_arguments)]
fn content(cx: &mut Cx, text: &str, placeholder: &str, font: Font, ph: (Font, Rgba), col: Rgba, inner: f32, h: f32, at: At, focused: bool, selected: bool, full_line: bool) -> El {
    let selected = selected && focused && !text.is_empty();
    let (shown, f, c) = if text.is_empty() { (placeholder, ph.0, ph.1) } else if selected { (text, font, Rgba::rgb(255, 255, 255)) } else { (text, font, col) };
    let tb = cx.g.text_box(shown, f, 0.0);
    let tx = match at {
        At::Centre => (((inner - tb.raw) / 2.0) * 64.0).floor() / 64.0,
        At::Left => {
            if focused && !text.is_empty() {
                -(tb.width + 1.0 - inner).max(0.0)
            } else {
                0.0
            }
        }
    };
    let mut b = El::block().w(inner).h(h).none().clip().no_hit();
    if selected {
        // Blink's inner editor keeps the line box when the input's fixed height is NOT taller than its line-height (`.thn` 24 =
        // 24, `.thd` 66 = 66, `.kdin` 34 = 34: the selection spans the whole line) and resets line-height to normal otherwise
        // (`.apin` 20 px, `.nbox` height:100%: the font's ascent + descent around the baseline) - `full_line` says which
        let (a, d) = cx.g.asc_desc(font);
        let (top, sh_) = if full_line { (0.0, h) } else { (cx.g.baseline(font, h) - a, a + d) };
        let (x0, x1) = (tx.floor(), (tx + tb.width).ceil());
        b = b.child(El::block().abs(x0, top, f32::NAN, f32::NAN).size(x1 - x0, sh_).bg(SEL));
    }
    if !shown.is_empty() {
        let mut t = El::text(shown, f, c, h).abs(tx, 0.0, f32::NAN, f32::NAN);
        if at == At::Left {
            t = t.align(Align::Left);
        }
        b = b.child(t);
    }
    if focused && !selected {
        cx.wake_every(530.0, 0.0);
        if ((cx.now / 530.0) as i64) % 2 == 0 {
            let tw = if text.is_empty() { 0.0 } else { tb.width };
            let x = if text.is_empty() && at == At::Centre { inner / 2.0 } else { (tx + tw).min(inner - 1.0) };
            let ch = (font.size() * 1.12).round();
            b = b.child(El::block().abs(x.round(), ((h - ch) / 2.0).round(), f32::NAN, f32::NAN).size(1.0, ch).bg(col).no_hit());
        }
    }
    b
}

/// Keys › "Add a key" › Website / Text: the 34 px well field, as wide as its parent. Focused on open, caret at the end (no
/// select); Enter with nothing typed uses the placeholder (the page's).
///
/// `.kdin{display:block;width:100%;height:34px;padding:0 11px;border:0;border-radius:8px;background:var(--well);
///   box-shadow:inset 0 0 0 .5px var(--hair);color:var(--fg);font:13px/34px "Segoe UI Variable Text";outline:none;cursor:text;
///   transition:box-shadow .15s ease}` `.kdin:focus{box-shadow:0 0 0 3px var(--acc-s),inset 0 0 0 1px var(--acc)}`
/// `.kdin::placeholder{color:var(--fg3)}`. (The two shadow lists differ in `inset` at the same place: not interpolable, so
/// Chromium switches at once - no visible transition.)
pub fn kdin(cx: &mut Cx, key: Key, text: &str, placeholder: &str, width: f32) -> El {
    let focused = cx.focused(key);
    let f = Font::new(13.0, 400).ls(0);
    let (insets, shadows) = if focused { (vec![sh(0.0, 0.0, 0.0, 1.0, ACC())], vec![sh(0.0, 0.0, 0.0, 3.0, ACC_S())]) } else { (vec![sh(0.0, 0.0, 0.0, 0.5, HAIR())], vec![]) };
    let inner = width - 22.0;
    El::block()
        .w(width)
        .h(34.0)
        .none()
        .pad(0.0, 11.0, 0.0, 11.0)
        .radius(8.0)
        .bg(WELL())
        .inset(&insets)
        .shadow(&shadows)
        .key(key)
        .cursor(Cursor::Text)
        .child(content(cx, text, placeholder, f, (f, FG3()), FG(), inner, 34.0, At::Left, focused, false, false))
}

/// Timers' hero name `.thn` (240 x 24, centred): rest fg2 on nothing, hover `--hov` + fg, focused `--ctl` + fg + a 2 px soft
/// ring; focus selects the name (`selected`). `clock` = the world-clock mode (`.thero.clk .thn{pointer-events:none}`).
///
/// `#sw .thn{width:240px;height:24px;padding:0 8px;border-radius:6px;background:transparent;color:var(--fg2);text-align:center;
///   font:600 12.5px/24px "Segoe UI Variable Text";transition:background-color .12s ease,color .12s ease}`
/// `:hover{background:var(--hov);color:var(--fg)}` `:focus{background:var(--ctl);color:var(--fg);box-shadow:0 0 0 2px var(--acc-s)}`
pub fn thn(cx: &mut Cx, key: Key, text: &str, selected: bool, clock: bool) -> El {
    let focused = !clock && cx.focused(key);
    let hovered = !clock && cx.hovered(key);
    // background: transparent -> hov (hover) -> ctl (focus); colour fg2 -> fg - both 120 ms
    let to_hov = cx.tr(key, 1, if hovered && !focused { 1.0 } else { 0.0 }, 120.0, EASE);
    let to_ctl = cx.tr(key, 2, if focused { 1.0 } else { 0.0 }, 120.0, EASE);
    let to_fg = cx.tr(key, 3, if hovered || focused { 1.0 } else { 0.0 }, 120.0, EASE);
    let bg = cmix(cmix(Rgba(HOV().0, HOV().1, HOV().2, 0.0), HOV(), to_hov), CTL(), to_ctl);
    let col = cmix(FG2(), FG(), to_fg);
    let f = Font::new(12.5, 600).ls(0);
    let mut b = El::block()
        .w(240.0)
        .h(24.0)
        .none()
        .pad(0.0, 8.0, 0.0, 8.0)
        .radius(6.0)
        .bg(bg)
        .child(content(cx, text, "", f, (f, col), col, 224.0, 24.0, At::Centre, focused, selected, true));
    if focused {
        b = b.shadow(&[sh(0.0, 0.0, 0.0, 2.0, ACC_S())]);
    }
    if clock {
        b.no_hit()
    } else {
        b.key(key).cursor(Cursor::Text)
    }
}

/// Timers' big time `.thd` (56 px Display digits, as wide as its parent, 66 px). `editable` = a countdown that is not
/// running (`.thero.cd`): it takes focus (selects all: `selected`, the drawing's `hTime.select()`); a stopwatch / running
/// countdown is read-only (no focus). The cursor is the arrow either way (`#sw .thd{cursor:default}` (1,1,0) beats
/// `.thero.cd .thd{cursor:text}` (0,3,0) - same trap as the colour). The digits stay `--fg` while focused: the drawing's `.thero.cd .thd:focus{color:var(--acc)}` (0,4,0) LOSES
/// to `#sw .thd{color:var(--fg)}` (1,1,0) - measured white in Chromium. `end_since` = when the countdown ended: three accent
/// blinks (`cdend .55s ease-in-out 3` - an animation beats both rules; also under reduced motion, whose rule names
/// `input.tmi`, not `.thd`).
///
/// `#sw .thd{display:block;width:100%;height:66px;padding:0;background:transparent;color:var(--fg);text-align:center;
///   cursor:default;font:600 56px/66px "Segoe UI Variable Display";letter-spacing:-.025em;font-variant-numeric:tabular-nums}`
/// `.thero.cd .thd{cursor:text}` `.thero.cd .thd:focus{color:var(--acc)}` `@keyframes cdend{50%{color:var(--acc)}}`
pub fn thd(cx: &mut Cx, key: Key, text: &str, width: f32, editable: bool, selected: bool, end_since: Option<f64>) -> El {
    let focused = editable && cx.focused(key);
    let mut col = FG();
    if let Some(t0) = end_since {
        let a = cx.now - t0;
        if (0.0..3.0 * 550.0).contains(&a) {
            // each 550 ms: fg -> acc (first half) -> fg (second half), ease-in-out per keyframe interval
            let p = (a % 550.0) / 275.0;
            let k = if p < 1.0 { EASE_IN_OUT.ease(p) } else { 1.0 - EASE_IN_OUT.ease(p - 1.0) } as f32;
            col = cmix(col, ACC(), k);
            cx.st.busy = true;
        }
    }
    let f = Font::display(56.0, 600).tnum().ls(-1400);
    let b = El::block().w(width).h(66.0).none().child(content(cx, text, "", f, (f, col), col, width, 66.0, At::Centre, focused, selected, true));
    if editable {
        // the arrow even when editable: `#sw .thd{cursor:default}` (1,1,0) beats `.thero.cd .thd{cursor:text}` (0,3,0)
        b.key(key)
    } else {
        b.no_hit()
    }
}

/// Mouse › Acceleration: renaming a preset chip in place (`.apin`, 20 px well inside the `.apc` chip, 12 px / 600). Focus
/// selects the name (`selected`); Enter / blur = save, Esc = cancel (the page's). Its width follows the text like the
/// drawing's `fit()`: `max(3, length × .62 + 1.2)` em of 12 px, padding included (border-box, measured); `margin:0 -4px`
/// keeps the chip's size.
///
/// `.apin{height:20px;margin:0 -4px;padding:0 4px;border:0;border-radius:4px;background:var(--well);color:var(--fg);
///   font:inherit;font-weight:600;outline:none}` (the chip's font: 12 px)
pub fn apin(cx: &mut Cx, key: Key, text: &str, selected: bool) -> El {
    let focused = cx.focused(key);
    // the input is border-box (the padding inside the width), the width floored to 1/64 px - measured: "Quake" 51.594 px
    let w = ((text.chars().count() as f32 * 0.62 + 1.2).max(3.0) * 12.0 * 64.0).floor() / 64.0;
    let em = w - 8.0;
    let f = Font::new(12.0, 600).ls(0);
    El::block()
        .w(w)
        .h(20.0)
        .none()
        .margin(0.0, -4.0, 0.0, -4.0)
        .pad(0.0, 4.0, 0.0, 4.0)
        .radius(4.0)
        .bg(WELL())
        .key(key)
        .cursor(Cursor::Text)
        .child(content(cx, text, "", f, (f, FG()), FG(), em, 20.0, At::Left, focused, selected, false))
}
