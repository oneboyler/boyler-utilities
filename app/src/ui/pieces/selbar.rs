//! The selection bar (menu-v22 `.selbar`, Order 025): with things selected, a slim glass bar floats at the window's bottom
//! centre - "N selected" (+ Apps' total size) · actions · ×. Screenshots (two or more selected) and Apps (one or more).
//! In WINDOW coordinates: the page returns it from `Page::overlay` (Lane K, 014 item 1c: window-fixed, non-modal), built
//! EVERY frame with `on` (false = hidden) so its show / hide motion plays.

use taffy::style::JustifyContent;

use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{idx, lh, Cursor, El, Key};
use crate::ui::{cmix, CTL, FG, FG2, HAIR, HL_V19, MENU, RED, WIN_H, WIN_W};

use super::btn_font;

/// One action button `.sbb`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sbb<'a> {
    pub icon: &'a str,
    pub label: &'a str,
    /// `.sbb.danger` (Delete, Uninstall)
    pub danger: bool,
}

/// The show / hide motion: `.selbar.on{transition:opacity .2s ease,transform .34s cubic-bezier(.3,1.2,.5,1)}` (the
/// overshoot), back with `transition:opacity .14s ease,transform .18s cubic-bezier(.4,0,1,1)`.
const SHOW: Bezier = Bezier::new(0.3, 1.2, 0.5, 1.0);
const HIDE: Bezier = Bezier::new(0.4, 0.0, 1.0, 1.0);

/// The bar. `count` = "3 selected", `size` = Apps' `.sbg` total ("4.2 GB", `.apbar>b{min-width:0}`), `btns` = the actions;
/// the clear × comes last. Clicks: action i = `Ev::Click(idx(key, i))`, the × = `Ev::Click(idx(key, btns.len()))`.
/// `on` = shown (Screenshots: 2+ selected; Apps: 1+ and no dialog / job running).
///
/// `.selbar{position:absolute;z-index:7;left:50%;bottom:14px;display:flex;align-items:center;gap:2px;height:40px;
///   padding:0 5px 0 14px;border-radius:12px;background:var(--menu);backdrop-filter:blur(30px) saturate(180%);
///   box-shadow:inset 0 0 0 .5px var(--hl),0 0 0 .5px rgba(0,0,0,.35),0 14px 36px rgba(0,0,0,.4);opacity:0;
///   transform:translate(-50%,10px) scale(.985)}` `.on{opacity:1;transform:translate(-50%,0)}`
/// `.selbar>b{min-width:74px;font-size:13px;font-weight:600;tabular-nums}` `.sbs{width:1px;height:18px;margin:0 4px;
///   background:var(--hair)}` `.apbar .sbg{margin:0 6px 0 2px;font-size:12px;color:var(--fg2);tabular-nums}`
pub fn selbar(cx: &mut Cx, key: Key, count: &str, size: Option<&str>, btns: &[Sbb], on: bool) -> El {
    selbar_x(cx, key, count, size, btns, on, None)
}

/// `selbar` with the ×'s hover name: `sbBtn('x','',..,'sbx',lab)` = `title:lab` (Apps: "Clear selection"; Screenshots:
/// "Clear selection (or click empty space)"); the action buttons have none (`title:lab||null`).
pub fn selbar_x(cx: &mut Cx, key: Key, count: &str, size: Option<&str>, btns: &[Sbb], on: bool, x_title: Option<&str>) -> El {
    let (opd, opb, trd, trb) = if on { (200.0, EASE, 340.0, SHOW) } else { (140.0, EASE, 180.0, HIDE) };
    let op = cx.tr(key, 1, if on { 1.0 } else { 0.0 }, opd, opb);
    let t = cx.tr(key, 2, if on { 0.0 } else { 1.0 }, trd, trb);
    let mut bar = El::row()
        .center()
        .gap(2.0)
        .h(40.0)
        .none()
        .pad(0.0, 5.0, 0.0, 14.0)
        .radius(12.0)
        .bg(MENU())
        .backdrop(30.0, 1.8)
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, HL_V19())])
        .shadow(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, 0.35)), sh(0.0, 14.0, 36.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.4))])
        .opacity(op)
        .translate(0.0, 10.0 * t)
        .scale(1.0 - 0.015 * t)
        .key(key);
    // (texts and separators never take the pointer themselves: the bar body does while it shows, nothing while it hides)
    let b = El::text(count, Font::new(13.0, 600).tnum(), FG(), lh(13.0, 1.35)).none().no_hit();
    bar = bar.child(if size.is_some() { b } else { b.min_w(74.0) });
    if let Some(s) = size {
        bar = bar.child(El::text(s, Font::new(12.0, 400).tnum(), FG2(), lh(12.0, 1.35)).none().no_hit().margin(0.0, 6.0, 0.0, 2.0));
    }
    let sep = || El::block().size(1.0, 18.0).none().no_hit().margin(0.0, 4.0, 0.0, 4.0).bg(HAIR());
    bar = bar.child(sep());
    for (i, s) in btns.iter().enumerate() {
        bar = bar.child(sbb(cx, idx(key, i), Some(s.label), s.icon, s.danger, false, on));
    }
    let x = sbb(cx, idx(key, btns.len()), None, "x", false, true, on);
    bar = bar.child(sep()).child(match x_title {
        Some(t) => x.title(t),
        None => x,
    });
    // left:50% + translate(-50%), bottom 14 px: laid out at the window's middle (its parts snapped there, a whole pixel)
    // and then moved by half its width - Blink rasters the bar as a layer at x 300 and composites it at 300 - w/2
    // (x .156 for "3 selected"), so its separators / icons sit on fractional pixels like the drawing's
    bar = bar.abs(WIN_W / 2.0, WIN_H - 14.0 - 40.0, f32::NAN, f32::NAN).translate_pct(-0.5, 0.0);
    if !on {
        // hiding: `.selbar{pointer-events:none}` at once - the whole bar lets the pointer through during its fade
        bar = bar.no_hit();
    }
    El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(WIN_W, WIN_H).z(7).no_hit().child(bar)
}

/// `#sw .sbb{display:inline-flex;align-items:center;gap:6px;height:30px;padding:0 11px 0 9px;border-radius:8px;background:transparent;
///   font-size:13px;transition:background-color .12s ease,color .12s ease,transform .12s ease}` `:hover{background:var(--ctl)}`
/// `.sbb:active{transform:scale(.96)}` `.sbb svg{width:15px;height:15px;stroke-width:1.5}` `#sw .sbb.danger{color:var(--red)}`
/// `:hover{background:rgba(255,69,58,.14)}` `#sw .sbb.sbx{width:30px;padding:0;justify-content:center;color:var(--fg2)}`
/// `:hover{color:var(--fg)}` `.sbx svg{width:9px;height:9px}`.
/// `on` = false: hiding (the drawing switches `pointer-events` off at once) - no clicks.
pub(crate) fn sbb(cx: &mut Cx, k: Key, label: Option<&str>, icon: &str, danger: bool, sbx: bool, on: bool) -> El {
    let hv = cx.hover_t(k, 120.0, EASE);
    // `.sbb:active{transform:scale(.96)}` is not in the drawing's reduced-motion list: kept
    let pr = cx.active_t(k, 120.0, EASE);
    let hover_bg = if danger { Rgba::rgba(255, 69, 58, 0.14) } else { CTL() };
    let col = if danger { RED() } else if sbx { cmix(FG2(), FG(), hv) } else { FG() };
    let mut b = El::row()
        .center()
        .gap(6.0)
        .h(30.0)
        .none()
        .radius(8.0)
        .bg(hover_bg.mul_a(hv))
        .scale(1.0 - 0.04 * pr);
    b = if on { b.on_click(k).cursor(Cursor::Hand) } else { b.no_hit() };
    b = if sbx { b.w(30.0).justify(JustifyContent::CENTER) } else { b.pad(0.0, 11.0, 0.0, 9.0) };
    let s = if sbx { 9.0 } else { 15.0 };
    b = b.child(El::icon(icon, s, 1.5, col).no_hit());
    if let Some(l) = label {
        // `#sw button{font:inherit}` + `.sbb{font-size:13px}`: 13 px, the button's letter-spacing: normal
        b = b.child(El::text(l, btn_font(13.0, 400), col, lh(13.0, 1.35)).none().no_hit());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::{selbar, Sbb};
    use crate::gfx::Gfx;
    use crate::ui::cx::{Cx, State};
    use crate::ui::el::{idx, key};
    use crate::ui::lay::Laid;
    use crate::ui::{WIN_H, WIN_W};

    #[test]
    fn buttons_hit_where_the_centred_bar_is_painted_and_not_while_hiding() {
        let g = Gfx::new(1.0);
        let k = key("t.sb");
        let btns = [Sbb { icon: "copy", label: "Copy", danger: false }, Sbb { icon: "trash", label: "Delete", danger: true }];
        let hit_at = |on: bool, x: f32| {
            let mut st = State::default();
            let mut cx = Cx::new(0.0, false, &g, &mut st);
            let laid = Laid::new(&g, selbar(&mut cx, k, "3 selected", None, &btns, on), WIN_W, Some(WIN_H));
            laid.hit(x, WIN_H - 14.0 - 20.0).map(|(_, keys)| keys).unwrap_or_default()
        };
        // laid out at x 300, moved left by half its width (~150): "Copy" sits at ~251-322 in the window, the x at ~415-445
        assert!(hit_at(true, 260.0).contains(&idx(k, 0)));
        assert!(hit_at(true, 430.0).contains(&idx(k, 2)));
        // where the bar would be without the move: nothing of it
        assert!(!hit_at(true, 470.0).contains(&idx(k, 1)));
        // hiding: the drawing turns pointer-events off at once
        assert!(hit_at(false, 260.0).is_empty());
    }

    #[test]
    fn rect_of_follows_the_move_and_nothing_hits_during_the_hide_fade() {
        let g = Gfx::new(1.0);
        let k = key("t.sb2");
        let btns = [Sbb { icon: "copy", label: "Copy", danger: false }, Sbb { icon: "trash", label: "Delete", danger: true }];
        let mut st = State::default();
        let mut build = |now: f64, on: bool| {
            let mut cx = Cx::new(now, false, &g, &mut st);
            let laid = Laid::new(&g, selbar(&mut cx, k, "3 selected", None, &btns, on), WIN_W, Some(WIN_H));
            st.sweep();
            laid
        };
        // the box the frame gives a button (test hook click:el:, Press / Drag boxes) is where it is painted and hit
        let laid = build(0.0, true);
        let boxes: Vec<_> = (0..3).map(|i| laid.rect_of(idx(k, i)).expect("button")).collect();
        for (i, (x, y, w, h)) in boxes.iter().enumerate() {
            assert!(laid.hit(x + w / 2.0, y + h / 2.0).map(|(_, keys)| keys).unwrap_or_default().contains(&idx(k, i)), "button {i}");
        }
        // 50 ms into the hide fade (opacity still ~.6): no part of the bar takes the pointer - not the body, a label, a button
        build(1000.0, true);
        let laid = build(1050.0, false);
        for (x, y, w, h) in &boxes {
            assert!(laid.hit(x + w / 2.0, y + h / 2.0).is_none());
        }
        assert!(laid.hit(160.0, WIN_H - 34.0).is_none());
    }
}
