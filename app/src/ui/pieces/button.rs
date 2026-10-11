//! Buttons (menu-v22): the dialog / page buttons `.cbtn` (ghost, primary `.acc`, red `.red`, red text `.redt`, small
//! `.sm`), the small icon + text button `.btn`, and the icon-only button (`.dlx` / `.cx`: 28 x 28, transparent).

use taffy::style::JustifyContent;

use crate::anim::EASE;
use crate::gfx::{sh, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{lh, Cursor, El, IconPaint, Key};
use crate::ui::{cmix, ACC, CTL, CTL_H, FG, FG2, HAIR, RED, WHITE};

use super::btn_font;

/// Which `.cbtn`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// `.cbtn` - the plain (ghost) button
    Ghost,
    /// `.cbtn.acc` - the blue main action
    Primary,
    /// `.cbtn.red` - a destructive main action
    Red,
    /// `.cbtn.redt` - red text on the plain button
    RedText,
    /// `.cbtn.qt` (addons-v1 tiles: Remove) - quiet grey text, red on hover: `#sw .adft .cbtn.qt{color:var(--fg2);
    /// font-weight:400}` `:hover{color:var(--red);background:rgba(255,69,58,.14)}`
    Quiet,
}

/// `#sw .cbtn{height:28px;padding:0 16px;border-radius:7px;background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair);
///   font-size:13px;transition:background-color .15s ease,filter .15s ease,transform .12s ease}` `:hover{background:var(--ctl-h)}`
/// `.acc{background:var(--acc);color:#fff;font-weight:600;box-shadow:inset 0 0 0 .5px rgba(255,255,255,.2),0 1px 3px rgba(0,0,0,.22)}`
/// `.acc:hover{filter:brightness(1.08)}` `.red{background:var(--red);...}` `.redt{color:var(--red);font-weight:600}`
/// `.redt:hover{background:rgba(255,69,58,.14)}` `.sm{height:26px;padding:0 12px;font-size:12px}` `:active{transform:scale(.97)}`
/// `:disabled{opacity:.45}`. `min_w` = e.g. the dialog footer's 76 px.
pub fn cbtn(cx: &mut Cx, key: Key, label: &str, kind: Kind, small: bool, disabled: bool, min_w: f32) -> El {
    let (h, px, fs) = if small { (26.0, 12.0, 12.0) } else { (28.0, 16.0, 13.0) };
    cbtn_sized(cx, key, label, kind, (h, px, fs), disabled, min_w)
}

/// The popup lists' own buttons: `#sw .mcfb .cbtn{height:26px;padding:0 13px;font-size:12px}`.
pub const MCFB: (f32, f32, f32) = (26.0, 13.0, 12.0);
/// The dialog footer's: `#sw .dft .cbtn{min-width:76px;height:30px}` (padding 0 16px, 13 px).
pub const DFT: (f32, f32, f32) = (30.0, 16.0, 13.0);

/// A `.cbtn` of a given (height, horizontal padding, font size).
pub fn cbtn_sized(cx: &mut Cx, key: Key, label: &str, kind: Kind, (h, px, fs): (f32, f32, f32), disabled: bool, min_w: f32) -> El {
    let hv = if disabled { 0.0 } else { cx.hover_t(key, 150.0, EASE) };
    let pr = if disabled { 0.0 } else { cx.active_t(key, 120.0, EASE) };
    // `.acc:hover` / `.red:hover{filter:brightness(1.08)}`: the whole button (rim, shadow, label), not only its fill
    let filter = if matches!(kind, Kind::Primary | Kind::Red) { 1.0 + 0.08 * hv } else { 1.0 };
    let (bg, fg, weight, insets, shadows): (Rgba, Rgba, u16, Vec<_>, Vec<_>) = match kind {
        Kind::Ghost => (cmix(CTL(), CTL_H(), hv), FG(), 400, vec![sh(0.0, 0.0, 0.0, 0.5, HAIR())], vec![]),
        Kind::RedText => (cmix(CTL(), Rgba::rgba(255, 69, 58, 0.14), hv), RED(), 600, vec![sh(0.0, 0.0, 0.0, 0.5, HAIR())], vec![]),
        Kind::Quiet => (cmix(CTL(), Rgba::rgba(255, 69, 58, 0.14), hv), cmix(FG2(), RED(), hv), 400, vec![sh(0.0, 0.0, 0.0, 0.5, HAIR())], vec![]),
        Kind::Primary | Kind::Red => {
            let base = if kind == Kind::Primary { ACC() } else { RED() };
            (base, WHITE, 600, vec![sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.2))], vec![sh(0.0, 1.0, 3.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.22))])
        }
    };
    let mut b = El::row()
        .center()
        .justify(JustifyContent::CENTER)
        .h(h)
        .min_w(min_w)
        .pad(0.0, px, 0.0, px)
        .radius(7.0)
        .bg(bg)
        .inset(&insets)
        .shadow(&shadows)
        .none()
        .scale(1.0 - 0.03 * pr)
        .brightness(filter)
        .child(El::text(label, btn_font(fs, weight), fg, lh(fs, 1.35)));
    if disabled {
        b = b.opacity(0.45).key(key);
    } else {
        b = b.on_click(key).cursor(Cursor::Hand);
    }
    b
}

/// `.btn{display:inline-flex;align-items:center;gap:6px;height:26px;padding:0 10px 0 8px;border-radius:6px;background:var(--ctl);
///   box-shadow:inset 0 0 0 .5px var(--hair),0 .5px 1px rgba(0,0,0,.12);font-size:12.5px;white-space:nowrap}`
/// `.btn svg{width:14px;height:14px;stroke-width:1.5}` `.btn:hover{background:var(--ctl-h)}` `:active{transform:scale(.97)}`
/// `#sw .btn.on{background:var(--acc);color:#fff;box-shadow:none}`. `icon` = an ICON name or "".
pub fn btn(cx: &mut Cx, key: Key, icon: &str, label: &str, on: bool) -> El {
    let hv = cx.hover_t(key, 150.0, EASE);
    let pr = cx.active_t(key, 120.0, EASE);
    let onv = cx.tr(key, 3, if on { 1.0 } else { 0.0 }, 150.0, EASE);
    let fg = cmix(FG(), WHITE, onv);
    let mut b = El::row()
        .center()
        .gap(6.0)
        .h(26.0)
        .pad(0.0, 10.0, 0.0, 8.0)
        .radius(6.0)
        .bg(cmix(cmix(CTL(), CTL_H(), hv), ACC(), onv))
        .none()
        .scale(1.0 - 0.03 * pr)
        .on_click(key)
        .cursor(Cursor::Hand);
    if onv < 0.999 {
        let k = 1.0 - onv;
        b = b.inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR().mul_a(k))]).shadow(&[sh(0.0, 0.5, 1.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.12 * k))]);
    }
    if !icon.is_empty() {
        b = b.child(El::icon(icon, 14.0, 1.5, fg).no_hit());
    }
    // `#sw button{font:inherit}` (an id rule) wins over `.btn{font-size:12.5px}`: 13 px (Chromium's computed style)
    b.child(El::text(label, btn_font(13.0, 400), fg, lh(13.0, 1.35)))
}

/// Order 098 (the owner: "This icon is ugly"): every play button. A small FILLED triangle in the text colour (white on the dark
/// glass), no circle at rest, centred optically (a triangle's weight sits left of its box, so it is nudged 1 px right); on hover
/// the same soft highlight as the window's × (28 x 28, radius 7, `--ctl-h`).
pub fn play_icon_btn(cx: &mut Cx, key: Key) -> El {
    let hv = cx.hover_t(key, 120.0, EASE);
    let tri = El::icon("play", 9.0, 1.0, FG()).h(10.0).icon_paint(IconPaint { fill_all: true, classes: vec![] }).translate(1.0, 0.0);
    El::block().size(28.0, 28.0).none().radius(7.0).bg(CTL_H().mul_a(hv)).place_center().on_click(key).cursor(Cursor::Hand).child(tri.no_hit())
}

/// The icon-only button: `#sw .dlx{width:28px;height:28px;border-radius:7px;background:transparent;color:var(--fg2);
///   transition:background-color .12s ease,color .12s ease}` `:hover{background:var(--ctl-h);color:var(--fg)}`;
/// `icon_size` = its svg (9 for the dialog's ×, 12 for `.cx`'s chevron), `stroke` its stroke width.
pub fn icon_btn(cx: &mut Cx, key: Key, icon: &str, icon_size: f32, stroke: f32) -> El {
    let hv = cx.hover_t(key, 120.0, EASE);
    El::block()
        .size(28.0, 28.0)
        .none()
        .radius(7.0)
        .bg(CTL_H().mul_a(hv))
        .place_center()
        .on_click(key)
        .cursor(Cursor::Hand)
        .child(El::icon(icon, icon_size, stroke, cmix(FG2(), FG(), hv)).no_hit())
}
