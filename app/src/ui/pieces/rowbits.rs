//! Small row controls (menu-v22, Order 025): the row remove × `.rdel` (Display rules, Keys, Mouse acceleration apps,
//! Tweaks' fullscreen window, Timers), the sound preview play button `.pb` (Audio's Mute settings, Timers' end chime) and
//! the chip switch `.wps` + its mini toggle `.mtg` (Performance "Show Windows processes", Timers "On screen" / "Sound at
//! the end").

use taffy::style::JustifyContent;

use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{lh, sub, Cursor, El, IconPaint, Key};
use crate::ui::{cmix, ACC, CTL, CTL_H, FG, FG2, FG3, HAIR, RED, TRK, WHITE};

/// The row remove ×. `row_hovered` = the page's `cx.hovered(row_key)` (the drawing's `#sw .<row>:hover .rdel{opacity:1}`);
/// it stays clickable while invisible (opacity does not stop clicks). Click = `Ev::Click(key)`.
///
/// `#sw .rdel{width:22px;height:22px;border-radius:50%;background:transparent;display:grid;place-items:center;color:var(--fg3);
///   opacity:0;transition:opacity .15s ease,background-color .12s ease,color .12s ease}` `#sw .rdel:hover{background:
///   rgba(255,69,58,.14);color:var(--red)}` `.rdel svg{width:8px;height:8px;stroke-width:1.5}` (`ICON.x`, 8 x 8)
pub fn rdel(cx: &mut Cx, key: Key, row_hovered: bool) -> El {
    let shown = cx.tr(key, 1, if row_hovered { 1.0 } else { 0.0 }, 150.0, EASE);
    let hv = cx.hover_t(key, 120.0, EASE);
    El::block()
        .size(22.0, 22.0)
        .none()
        .radius(11.0)
        .bg(Rgba::rgba(255, 69, 58, 0.14).mul_a(hv))
        .opacity(shown)
        .place_center()
        .on_click(key)
        .cursor(Cursor::Hand)
        .child(El::icon("x", 8.0, 1.5, cmix(FG3(), RED(), hv)).no_hit())
}

/// How long the play button stays "playing" (`.play`): 260 ms for the mute sound (`dir < 0`), 300 ms otherwise.
pub fn play_ms(dir: i8) -> f64 {
    if dir < 0 {
        260.0
    } else {
        300.0
    }
}

/// The sound preview play button. `playing_since` = when the page started the sound (`.play` for `play_ms`), `disabled` =
/// the picked sound is "none". Click = `Ev::Click(key)`.
///
/// `.pb{width:24px;height:24px;border-radius:50%;background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair);display:grid;
///   place-items:center;transition:background .15s ease,color .15s ease,transform .12s ease}` `.pb svg{width:8px;height:9px;
///   fill:currentColor;margin-left:1.5px}` `.pb:hover{background:var(--ctl-h)}` `.pb:active{transform:scale(.92)}`
/// `.pb.play{background:var(--acc);color:#fff;box-shadow:none}` `.pb:disabled{opacity:.35}` (hover still changes the
/// background while disabled). The icon colour is the parent's (`#sw button{color:inherit}`): `FG`.
pub fn pb(cx: &mut Cx, key: Key, playing_since: Option<f64>, dir: i8, disabled: bool) -> El {
    let playing = playing_since.is_some_and(|t| (0.0..play_ms(dir)).contains(&(cx.now - t)));
    if playing {
        cx.st.busy = true;
    }
    let hv = cx.hover_t(key, 150.0, EASE);
    let pr = if disabled || cx.rm { 0.0 } else { cx.active_t(key, 120.0, EASE) };
    let pl = cx.tr(key, 3, if playing { 1.0 } else { 0.0 }, 150.0, EASE);
    let mut b = El::block()
        .size(24.0, 24.0)
        .none()
        .radius(12.0)
        .bg(cmix(cmix(CTL(), CTL_H(), hv), ACC(), pl))
        .scale(1.0 - 0.08 * pr)
        .key(key);
    if pl < 0.999 {
        b = b.inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR().mul_a(1.0 - pl))]);
    }
    // the grid centres the svg's margin box (8 + 1.5 wide): the svg at x (24 - 9.5) / 2 + 1.5 = 8.75, y 7.5
    // `.pb.play{color:#fff}` loses to `#sw button{color:inherit}` (1,0,1 > 0,2,0): the icon keeps the parent's colour
    let icon = El::icon("play", 8.0, 1.0, FG()).h(9.0).abs(8.75, 7.5, f32::NAN, f32::NAN).no_hit();
    b = b.child(icon.icon_paint(IconPaint { fill_all: true, classes: vec![] }));
    if disabled {
        b.opacity(0.35)
    } else {
        b.on_click(key).cursor(Cursor::Hand)
    }
}

/// The knob's motion `cubic-bezier(.3,1.3,.5,1)` (an overshoot).
const KNOB: Bezier = Bezier::new(0.3, 1.3, 0.5, 1.0);

/// Where the chip switch sits: in a group header's right part (Performance) or in Timers' options row
/// (`#sw .tho .wps{margin:0;padding:0 2px}`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WpsAt {
    Header,
    Timers,
}

/// The chip switch: a mini toggle + its words, the whole button switches. Click = `Ev::Click(key)`.
///
/// `#sw .wps{display:inline-flex;align-items:center;gap:7px;height:24px;margin-right:2px;padding:0 4px;border-radius:6px;
///   background:transparent;font-size:11.5px;color:var(--fg2);transition:color .12s ease}` `:hover{color:var(--fg)}`
/// `.mtg{width:26px;height:15px;border-radius:8px;background:var(--trk);transition:background-color .2s ease}`
/// `.mtg::after{left:2px;top:2px;width:11px;height:11px;border-radius:50%;background:#fff;box-shadow:0 1px 2px rgba(0,0,0,.3);
///   transition:transform .2s cubic-bezier(.3,1.3,.5,1)}` `.wps.on .mtg{background:var(--acc)}` `.wps.on .mtg::after{transform:translateX(11px)}`
/// The words: `#sw button{font:inherit}` - the header's 400 weight (`.ghr`) / Timers' 12 px row font is inherited, then
/// `.wps{font-size:11.5px}`; the button's `letter-spacing: normal`.
pub fn wps(cx: &mut Cx, key: Key, label: &str, on: bool, at: WpsAt) -> El {
    let hv = cx.hover_t(key, 120.0, EASE);
    let tr = cx.tr(key, 3, if on { 1.0 } else { 0.0 }, 200.0, EASE);
    let kx = cx.tr(sub(key, "k"), 1, if on { 11.0 } else { 0.0 }, 200.0, KNOB);
    let track = El::block()
        .size(26.0, 15.0)
        .none()
        .radius(8.0)
        .bg(cmix(TRK(), ACC(), tr))
        .no_hit()
        .child(El::block().abs(2.0 + kx, 2.0, f32::NAN, f32::NAN).size(11.0, 11.0).radius(5.5).bg(WHITE).shadow(&[sh(0.0, 1.0, 2.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.3))]));
    let (mr, px) = match at {
        WpsAt::Header => (2.0, 4.0),
        WpsAt::Timers => (0.0, 2.0),
    };
    El::row()
        .center()
        .justify(JustifyContent::FLEX_START)
        .gap(7.0)
        .h(24.0)
        .none()
        .margin(0.0, mr, 0.0, 0.0)
        .pad(0.0, px, 0.0, px)
        .radius(6.0)
        .on_click(key)
        .cursor(Cursor::Hand)
        .child(track)
        .child(El::text(label, Font::new(11.5, 400).ls(0), cmix(FG2(), FG(), hv), lh(11.5, 1.35)).none())
}
