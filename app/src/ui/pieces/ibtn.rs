//! Keycaps in every size of the drawing (`.kc` and its variants) and the dialog button with an icon (`.dft .cbtn.ic`) +
//! Network's game-server Start / Stop button (`.cbtn.acc.sm.ic.gsgo`) - menu-v22, Order 025.

use taffy::style::JustifyContent;

use crate::anim::EASE;
use crate::gfx::{sh, CssColor, Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{lh, Cursor, El, IconPaint, Key};
use crate::ui::{cmix, ACC, CTL, CTL_H, FG, HAIR, KEY, RED, WHITE};

use super::btn_font;

/// One keycap size.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Cap {
    pub h: f32,
    pub fs: f32,
    pub px: f32,
    pub r: f32,
    /// inside a `<button>` (Controller's action chip `.ach`): the button's `letter-spacing: normal`
    pub in_button: bool,
}

/// `.kc{height:18px;padding:0 6px;border-radius:4px;font:600 11px/1}` (key fields, shortcut lists)
pub const CAP: Cap = Cap { h: 18.0, fs: 11.0, px: 6.0, r: 4.0, in_button: false };
/// `.mitem .kc` / `.pdlb .kc` / `.chr .kc{height:16px;font-size:10.5px}` (Controller)
pub const CAP_SM: Cap = Cap { h: 16.0, fs: 10.5, px: 6.0, r: 4.0, in_button: false };
/// `.ach .kc` - the same in a button chip (letter-spacing 0)
pub const CAP_SM_BTN: Cap = Cap { h: 16.0, fs: 10.5, px: 6.0, r: 4.0, in_button: true };
/// `.kf.kbig .kc{height:28px;padding:0 10px;border-radius:6px;font-size:14px}` (Keys' big "press the key" field)
pub const CAP_BIG: Cap = Cap { h: 28.0, fs: 14.0, px: 10.0, r: 6.0, in_button: false };
/// `.pdcards .ch small .kc{height:15px;font-size:10px;padding:0 4px}`
pub const CAP_XS: Cap = Cap { h: 15.0, fs: 10.0, px: 4.0, r: 4.0, in_button: false };

/// A keycap: `.kc{display:inline-flex;align-items:center;background:var(--key);box-shadow:inset 0 0 0 .5px var(--hair),
/// 0 1px 0 rgba(0,0,0,.22);font:600 11px/1 var(--font);color:var(--fg)}` in a size; `dim` = `.kc.dim{opacity:.55}`.
/// (Lane K's key field draws its own `.kc` - this is for the other places.)
pub fn keycap(text: &str, c: &Cap, dim: bool) -> El {
    let f = if c.in_button { Font::new(c.fs, 600).ls(0) } else { Font::new(c.fs, 600) };
    let mut k = El::row()
        .center()
        .h(c.h)
        .none()
        .pad(0.0, c.px, 0.0, c.px)
        .radius(c.r)
        .bg(KEY())
        .shadow(&[sh(0.0, 1.0, 0.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.22))])
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
        .child(El::text(text, f, FG(), c.fs));
    if dim {
        k = k.opacity(0.55);
    }
    k
}

/// The dialog footer's button with an icon: `#sw .dft .cbtn{min-width:76px;height:30px}` `#sw .dft .cbtn.ic{display:inline-flex;
/// align-items:center;gap:7px;padding:0 14px 0 11px}` `.cbtn.ic svg{width:15px;height:15px;stroke-width:1.5}` + the `.cbtn`
/// states (hover, `.acc` brightness 1.08, pressed .97). The content starts at the LEFT (no justify rule). `primary` =
/// `.cbtn.acc` (Change, Add game), else the plain one (Open). Click = `Ev::Click(key)`. (The drawing has no disabled `.cbtn.ic`;
/// Add game's native `title` "Pick the game's .exe" is not built - the app has no native title tips.)
pub fn icbtn(cx: &mut Cx, key: Key, icon: &str, label: &str, primary: bool) -> El {
    let hv = cx.hover_t(key, 150.0, EASE);
    let pr = if cx.rm { 0.0 } else { cx.active_t(key, 120.0, EASE) };
    let (bg, fg, w, insets, shadows) = if primary {
        (ACC(), WHITE, 600, vec![sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.2))], vec![sh(0.0, 1.0, 3.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.22))])
    } else {
        (cmix(CTL(), CTL_H(), hv), FG(), 400, vec![sh(0.0, 0.0, 0.0, 0.5, HAIR())], vec![])
    };
    acc_hover(El::row(), primary, hv)
        .center()
        .gap(7.0)
        .h(30.0)
        .min_w(76.0)
        .none()
        .pad(0.0, 14.0, 0.0, 11.0)
        .radius(7.0)
        .bg(bg)
        .inset(&insets)
        .shadow(&shadows)
        .scale(1.0 - 0.03 * pr)
        .on_click(key)
        .cursor(Cursor::Hand)
        .child(El::icon(icon, 15.0, 1.5, fg).no_hit())
        .child(El::text(label, btn_font(13.0, w), fg, lh(13.0, 1.35)).none())
}

/// `#sw .cbtn.acc:hover{filter:brightness(1.08)}` (`transition: filter .15s ease`): the WHOLE button - fill, ring, shadow,
/// icon and text edges - brightened in one layer; no layer at rest.
fn acc_hover(b: El, acc: bool, hv: f32) -> El {
    if acc && hv > 0.0 {
        b.color_filter(CssColor::Brightness(1.0 + 0.08 * hv))
    } else {
        b
    }
}

/// The stop square of the running game-server button (`<svg viewBox="0 0 8 8"><rect … rx="1.4"/></svg>`).
const STOP: &str = r#"<svg viewBox="0 0 8 8"><rect x=".5" y=".5" width="7" height="7" rx="1.4"/></svg>"#;

/// Network's game-server Start / Stop: `cbtn acc sm ic gsgo` - `#sw .gsgo{display:inline-flex;align-items:center;gap:7px;
/// min-width:78px;padding:0 13px 0 11px;justify-content:center}` `#sw .gsgo svg{width:8px;height:9px;fill:currentColor;stroke:none}`
/// (`.sm`: 26 px, 12 px). `running` = `.gsstop` (no `.acc`): the plain button, text --fg, a red filled stop square. Start <->
/// Stop: the background glides (`.cbtn{transition:background-color .15s}`), the rest switches at once. (The native `title`
/// tips "Ping every … server region" / "Stop pinging" are not built - the app has no native title tips.)
pub fn gsgo(cx: &mut Cx, key: Key, running: bool) -> El {
    let hv = cx.hover_t(key, 150.0, EASE);
    let pr = if cx.rm { 0.0 } else { cx.active_t(key, 120.0, EASE) };
    let run = cx.tr(key, 5, if running { 1.0 } else { 0.0 }, 150.0, EASE);
    let plain = cmix(CTL(), CTL_H(), hv);
    let (bg, fg, w, insets, shadows) = if !running {
        (cmix(ACC(), plain, run), WHITE, 600, vec![sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.2))], vec![sh(0.0, 1.0, 3.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.22))])
    } else {
        (cmix(ACC(), plain, run), FG(), 400, vec![sh(0.0, 0.0, 0.0, 0.5, HAIR())], vec![])
    };
    let fill = IconPaint { fill_all: true, classes: vec![] };
    // the svg is 8 x 9 in both states (the stop square's 8 x 8 viewBox sits .5 px down in it, xMidYMid meet): the svg box is
    // snapped, the half pixel is the SVG's own transform - a translate after the snap, not a laid-out offset
    let icon = if running {
        El::block().size(8.0, 9.0).none().child(El::icon(STOP, 8.0, 1.0, RED()).icon_paint(fill).translate(0.0, 0.5))
    } else {
        El::icon("play", 8.0, 1.0, fg).h(9.0).icon_paint(fill)
    };
    acc_hover(El::row(), !running, hv)
        .center()
        .justify(JustifyContent::CENTER)
        .gap(7.0)
        .h(26.0)
        .min_w(78.0)
        .none()
        // `#sw .cbtn.sm{padding:0 12px}` (1,2,0) beats `#sw .gsgo{padding:0 13px 0 11px}` (1,1,0) - measured 12 / 12
        .pad(0.0, 12.0, 0.0, 12.0)
        .radius(7.0)
        .bg(bg)
        .inset(&insets)
        .shadow(&shadows)
        .scale(1.0 - 0.03 * pr)
        .on_click(key)
        .cursor(Cursor::Hand)
        .child(icon.no_hit())
        .child(El::text(if running { "Stop" } else { "Start" }, btn_font(12.0, w), fg, lh(12.0, 1.35)).none())
}
