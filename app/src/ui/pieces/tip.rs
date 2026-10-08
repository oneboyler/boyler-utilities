//! Tooltips (menu-v22, Order 025): the ONE shared glass tip `.wtip` for any element carrying `data-tip` (shown 300 ms
//! after the pointer rests on it, above it - under it near the top row), the small tip icons `.rq` (admin shield, restart
//! arrow, info) that usually carry one, and the under-button tip `.htip` of Display's Identify button `.idb`.
//!
//! A page marks an element with `.tip("text")` (`El::tip`); the frame owns ONE `tip::Tips` (Lane K, item 1c) that shows
//! the bubble for the hovered element and hides it the drawing's ways. Until the frame has it, NO `data-tip` bubble shows
//! on any page (a page never knows its elements' window boxes).

use std::rc::Rc;

use crate::anim::EASE;
use crate::gfx::{sh, Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{Cursor, El, Key};
use crate::ui::lay::Laid;
use crate::ui::{cmix, CTL, CTL_H, FG, FG3, HL_V19, HOV, MENU, POP};


/// The drawing's hover delay before the shared tip shows (`setTimeout(…,300)`; `.htip` `transition-delay:.3s`).
pub const DELAY_MS: f64 = 300.0;

/// Order 045: the hover delay of a plain `title="…"` hover name (`El::title`): Windows' own tooltip delay (TTDT_INITIAL =
/// the double-click time, 500 ms by default) - "small delay, like Windows".
pub const TITLE_DELAY_MS: f64 = 500.0;

/// The tip's text box: `font-size:11.5px;font-weight:600;line-height:15px` (+ `#sw`'s inherited letter-spacing).
const TF: Font = Font::new(11.5, 600);

/// Where the shared tip goes for an element box `r` (window coordinates) in a window `win_w` wide: the drawing's
/// `x = clamp(r.left + r.width/2 - tw/2, 8, winW - tw - 8)`, `y = r.top - th - 7`, below (`y = r.bottom + 7`) when that
/// would be above y = 56 (the top row); both `Math.round`ed. `tw` / `th` = the bubble's `offsetWidth` / `offsetHeight`.
/// Returns (x, y, below).
pub fn place(r: (f32, f32, f32, f32), tw: f32, th: f32, win_w: f32) -> (f32, f32, bool) {
    let (rx, ry, rw, rh) = r;
    let v = rx + rw / 2.0 - tw / 2.0;
    // const clamp=(v,a,b)=>v<a?a:(v>b?b:v)
    let (a, b) = (8.0, win_w - tw - 8.0);
    let x = if v < a { a } else if v > b { b } else { v };
    let mut y = ry - th - 7.0;
    let below = y < 56.0;
    if below {
        y = ry + rh + 7.0;
    }
    (js_round(x), js_round(y), below)
}

/// JavaScript's Math.round (halves go up).
fn js_round(v: f32) -> f32 {
    (v + 0.5).floor()
}

/// The shared tip bubble `.wtip` at the place `place` gives for the anchor box `anchor` (window coordinates).
///
/// `.wtip{position:absolute;z-index:21;padding:4px 9px;border-radius:7px;white-space:nowrap;font-size:11.5px;font-weight:600;
///   line-height:15px;color:var(--fg);background:var(--pop);backdrop-filter:blur(30px) saturate(180%);
///   box-shadow:inset 0 0 0 .5px var(--hl),0 0 0 .5px rgba(0,0,0,.3),0 6px 18px rgba(0,0,0,.28);opacity:0;
///   transform:translateY(3px);transition:opacity .12s ease,transform .12s ease}` `.below{transform:translateY(-3px)}`
/// `.on{opacity:1;transform:none}`. `key` = the tipped element's key (its fade), `on` = the hover rested `DELAY_MS`.
pub fn bubble(cx: &mut Cx, key: Key, text: &str, anchor: (f32, f32, f32, f32), win_w: f32, on: bool) -> El {
    let (x, y, below) = place(anchor, bubble_w(cx.g, text), BUBBLE_H, win_w);
    let k = crate::ui::el::sub(key, "wtip");
    let op = cx.tr(k, 1, if on { 1.0 } else { 0.0 }, 120.0, EASE);
    let t = cx.tr(k, 2, if on { 0.0 } else { 1.0 }, 120.0, EASE);
    bubble_at(text, x, y, below, op, t)
}

/// The bubble's `offsetHeight` (4 + 15 + 4).
const BUBBLE_H: f32 = 23.0;

/// The bubble's `offsetWidth`: the border box rounded to whole px (the text's LayoutUnit width + 18 px padding).
fn bubble_w(g: &crate::gfx::Gfx, text: &str) -> f32 {
    js_round(g.text_box(text, TF, 0.0).width + 18.0)
}

/// The bubble at a fixed place: `op` = its opacity, `t` = 0 shown .. 1 hidden (the 3 px slide toward its side).
fn bubble_at(text: &str, x: f32, y: f32, below: bool, op: f32, t: f32) -> El {
    El::block()
        .abs(x, y, f32::NAN, f32::NAN)
        .pad(4.0, 9.0, 4.0, 9.0)
        .radius(7.0)
        .bg(POP())
        .backdrop(30.0, 1.8)
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, HL_V19())])
        .shadow(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, 0.3)), sh(0.0, 6.0, 18.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.28))])
        .opacity(op)
        .translate(0.0, t * if below { -3.0 } else { 3.0 })
        .z(21)
        .no_hit()
        .child(El::text(text, TF, FG(), 15.0))
}

/// One shown bubble: placed ONCE when it shows (the drawing sets left / top once), fading in, or out after a hide.
struct Shown {
    key: Key,
    text: Rc<str>,
    x: f32,
    y: f32,
    below: bool,
    on_at: f64,
    off_at: Option<f64>,
}

/// The shared tip's state - ONE per window, owned by the frame (the drawing's `wtip` / `wtipEl` / `wtipT`, 4728-4748).
///
/// The frame (Lane K, item 1c) calls `update` whenever it builds a frame (hover changes, the 300 ms timer, fades),
/// `hide` on everything the drawing calls `tipHide()` for (the page scrolls, a menu / popup / dialog opens, the tab
/// changes, the pointer leaves the window), and draws `el` above everything (z 21, above menus).
#[derive(Default)]
pub struct Tips {
    /// the tipped element the pointer is on (`wtipEl`) and since when (the 300 ms timer)
    armed: Option<(Key, f64)>,
    /// the armed element's delay (`DELAY_MS`, or `TITLE_DELAY_MS` for a plain hover name)
    delay: f64,
    shown: Option<Shown>,
    /// after `hide`: no tip until the hover chain changes (the drawing re-arms on the next mouseover)
    blocked: Option<Vec<Key>>,
}

impl Tips {
    /// `layers` = the laid layers the pointer can be over, topmost first (the open popup, then the page), each with its
    /// offset in the window; `hover` = the hover chain (`State::hover`, innermost first). The innermost tipped element
    /// wins (`closest('[data-tip]')`). Returns true while it needs frames (the timer runs or a fade).
    pub fn update(&mut self, g: &crate::gfx::Gfx, layers: &[(&Laid, (f32, f32))], hover: &[Key], now: f64, win_w: f32) -> bool {
        self.drop_faded(now);
        if let Some(b) = &self.blocked {
            if b.as_slice() == hover {
                return self.busy(now);
            }
            self.blocked = None;
        }
        let found = layers.iter().find_map(|(l, (dx, dy))| l.tip_in(hover).map(|(k, t, (x, y, w, h), title)| (k, t, (x + dx, y + dy, w, h), title)));
        match &found {
            Some((k, _, _, _)) if self.armed.map(|a| a.0) == Some(*k) => {}
            Some((k, _, _, title)) => {
                // another tipped element: the old bubble fades, the timer restarts
                self.fade_out(now);
                self.armed = Some((*k, now));
                self.delay = if *title { TITLE_DELAY_MS } else { DELAY_MS };
            }
            None => {
                self.armed = None;
                self.fade_out(now);
            }
        }
        if let (Some((k, t0)), Some((_, text, r, _))) = (self.armed, &found) {
            let showing = self.shown.as_ref().is_some_and(|s| s.key == k && s.off_at.is_none());
            if !showing && now - t0 >= self.delay {
                let (x, y, below) = place(*r, bubble_w(g, text), BUBBLE_H, win_w);
                self.shown = Some(Shown { key: k, text: text.clone(), x, y, below, on_at: now, off_at: None });
            }
        }
        self.drop_faded(now);
        self.busy(now)
    }

    /// A bubble whose .12 s fade-out has ended is gone.
    fn drop_faded(&mut self, now: f64) {
        if self.shown.as_ref().is_some_and(|s| s.off_at.is_some_and(|t| now - t >= 120.0)) {
            self.shown = None;
        }
    }

    /// The drawing's `tipHide()`: the bubble fades out and nothing shows again until the pointer moves to another element.
    pub fn hide(&mut self, now: f64, hover: &[Key]) {
        self.armed = None;
        self.fade_out(now);
        self.blocked = Some(hover.to_vec());
    }

    fn fade_out(&mut self, now: f64) {
        if let Some(s) = &mut self.shown {
            if s.off_at.is_none() {
                s.off_at = Some(now);
            }
        }
    }

    /// Frames only for a fade (Order 047: the delay is waited out asleep - `due`).
    fn busy(&self, now: f64) -> bool {
        let timer = self.armed.is_some_and(|(k, t)| now - t >= self.delay && self.shown.as_ref().map(|s| s.key) != Some(k));
        let fade = self.shown.as_ref().is_some_and(|s| now - s.on_at < 120.0 || s.off_at.is_some());
        timer || fade
    }

    /// Order 047: when the armed tip's delay ends (the menu wakes then and shows it), None = no timer running.
    pub fn due(&self) -> Option<f64> {
        let (k, t) = self.armed?;
        let showing = self.shown.as_ref().is_some_and(|s| s.key == k && s.off_at.is_none());
        (!showing).then_some(t + self.delay)
    }

    /// The bubble to draw (window coordinates), fading in / out over the drawing's .12 s ease.
    pub fn el(&self, now: f64) -> Option<El> {
        let s = self.shown.as_ref()?;
        let p_in = EASE.ease(((now - s.on_at) / 120.0).clamp(0.0, 1.0)) as f32;
        let p_out = s.off_at.map(|t| EASE.ease(((now - t) / 120.0).clamp(0.0, 1.0)) as f32).unwrap_or(0.0);
        let op = p_in * (1.0 - p_out);
        Some(bubble_at(&s.text, s.x, s.y, s.below, op, 1.0 - op))
    }
}

/// Which tip icon (`tipIc(k, tip)` -> `ICON.shield` for 'adm', `ICON.rst` otherwise; `.rq.rqi` = `ICON.info`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rq {
    /// needs admin (`TIP.adm` "Needs admin — Windows asks once")
    Adm,
    /// applies after Explorer restarts / sign-out / restart / next game (`TIP.exp|out|boot|game`)
    Rst,
    /// an info dot (`.rq.rqi`, Storage clean-up rows; Activity's uptime)
    Info,
}

/// The texts of the drawing's `TIP` table.
pub mod texts {
    pub const ADM: &str = "Needs admin \u{2014} Windows asks once";
    pub const EXP: &str = "Restarts Explorer \u{2014} the taskbar blinks once";
    pub const OUT: &str = "Takes effect after you sign out";
    pub const BOOT: &str = "Takes effect after a restart";
    pub const GAME: &str = "Takes effect the next time a game starts";
}

/// A tip icon `.rq` (`box_size` 18 = `.rq`, 16 = `.rq.rqi` / `.pch>.rq`, 14 = `.scb b .rq`; its svg is 12 px, 11 px in the
/// 14 px box). Carries its tip text: the shared bubble shows it.
///
/// `.rq{display:grid;place-items:center;width:18px;height:18px;flex:none;border-radius:5px;color:var(--fg3);cursor:help;
///   transition:color .12s ease,background-color .12s ease}` `:hover{color:var(--fg);background:var(--hov)}`
/// `.rq svg{width:12px;height:12px;stroke:currentColor;stroke-width:1.4}` `.rq.adm+.rq{margin-left:-3px}` (pass `after_adm`).
/// The svg is centred both ways (Chromium: at x + 3, y + 3 in the 18 px box). Inside `.mbtn` the drawing's
/// `#sw .mbtn i{display:block}` wins instead (the DNS button's shield sits at the top) - `mbtn` draws that one itself.
pub fn rq(cx: &mut Cx, key: Key, which: Rq, box_size: f32, tip: &str, after_adm: bool) -> El {
    let hv = cx.hover_t(key, 120.0, EASE);
    let s = if box_size <= 14.0 { 11.0 } else { 12.0 };
    let icon = match which {
        Rq::Adm => "shield",
        Rq::Rst => "rst",
        Rq::Info => "info",
    };
    El::block()
        .size(box_size, box_size)
        .none()
        .radius(5.0)
        .bg(HOV().mul_a(hv))
        .margin(0.0, 0.0, 0.0, if after_adm { -3.0 } else { 0.0 })
        .key(key)
        .tip(tip)
        .place_center()
        .child(El::icon(icon, s, 1.4, cmix(FG3(), FG(), hv)).no_hit())
}

/// Display's Identify button with its under-button tip: `.btn.idb` + `.htip`.
///
/// `.btn{display:inline-flex;align-items:center;background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair),
///   0 .5px 1px rgba(0,0,0,.12)}` `:hover{background:var(--ctl-h)}` `:active{transform:scale(.97)}`
/// `#sw .idb{position:relative;width:36px;height:36px;padding:0;justify-content:center;border-radius:10px}`
/// `#sw .idb svg{width:16px;height:16px}` (`.btn svg{stroke-width:1.5}`)
/// `.htip{position:absolute;left:50%;top:calc(100% + 8px);z-index:9;transform:translate(-50%,3px);opacity:0;white-space:nowrap;
///   padding:4px 9px;border-radius:7px;font-size:11.5px;font-weight:600;line-height:15px;color:var(--fg);background:var(--menu);
///   backdrop-filter:blur(30px) saturate(180%);box-shadow:inset 0 0 0 .5px var(--hl),0 0 0 .5px rgba(0,0,0,.3),
///   0 6px 18px rgba(0,0,0,.28);transition:opacity .12s ease,transform .12s ease}`
/// `.idb:hover .htip{opacity:1;transform:translate(-50%,0);transition-delay:.3s}`.
pub fn idb(cx: &mut Cx, key: Key, icon: &str, tip: &str) -> El {
    let hv = cx.hover_t(key, 150.0, EASE);
    // reduced motion: `.btn:active{transform:none}` (the drawing's RM rules)
    let pr = if cx.rm { 0.0 } else { cx.active_t(key, 120.0, EASE) };
    let hovered = cx.hovered(key);
    let tk = crate::ui::el::sub(key, "htip");
    let show = cx.tr_delayed(tk, 1, if hovered { 1.0 } else { 0.0 }, 120.0, if hovered { DELAY_MS } else { 0.0 }, EASE);
    // inside a <button>: the browser's own `letter-spacing: normal` (not #sw's -.006em)
    let tf = TF.ls(0);
    let tw = cx.g.text_box(tip, tf, 0.0).width + 18.0;
    let mut b = El::row()
        .center()
        .justify(taffy::style::JustifyContent::CENTER)
        .size(36.0, 36.0)
        .none()
        .radius(10.0)
        .bg(cmix(CTL(), CTL_H(), hv))
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, crate::ui::HAIR())])
        .shadow(&[sh(0.0, 0.5, 1.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.12))])
        .scale(1.0 - 0.03 * pr)
        .on_click(key)
        .cursor(Cursor::Hand)
        .child(El::icon(icon, 16.0, 1.5, FG()).no_hit());
    if show > 0.001 {
        b = b.child(
            El::block()
                // left:50% + translate(-50%): the centre of the 36 px button
                .abs(18.0, 36.0 + 8.0, f32::NAN, f32::NAN)
                .translate(-tw / 2.0, 3.0 * (1.0 - show))
                .pad(4.0, 9.0, 4.0, 9.0)
                .radius(7.0)
                .bg(MENU())
                .backdrop(30.0, 1.8)
                .inset(&[sh(0.0, 0.0, 0.0, 0.5, HL_V19())])
                .shadow(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, 0.3)), sh(0.0, 6.0, 18.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.28))])
                .opacity(show)
                .z(9)
                .no_hit()
                .child(El::text(tip, tf, FG(), 15.0)),
        );
    }
    b
}

#[cfg(test)]
mod tests {
    use super::{place, Tips};
    use crate::gfx::Gfx;
    use crate::ui::el::{key, El};
    use crate::ui::lay::Laid;

    #[test]
    fn tips_show_after_300_ms_and_hide_the_drawings_ways() {
        let g = Gfx::new(1.0);
        let (a, b) = (key("t.a"), key("t.b"));
        let page = El::block().size(600.0, 400.0).child(El::block().size(18.0, 18.0).key(a).tip("Needs admin")).child(El::block().size(18.0, 18.0).key(b));
        let laid = Laid::new(&g, page, 600.0, Some(400.0));
        let layers = [(&laid, (0.0, 100.0))];
        let mut t = Tips::default();
        // the pointer rests on the tipped element: nothing for 300 ms, then the bubble, placed once
        // (Order 047: the 300 ms are waited out asleep - no frames, `due` says when)
        assert!(!t.update(&g, &layers, &[a], 0.0, 600.0));
        assert_eq!(t.due(), Some(300.0));
        assert!(t.el(0.0).is_none());
        t.update(&g, &layers, &[a], 299.0, 600.0);
        assert!(t.el(299.0).is_none());
        t.update(&g, &layers, &[a], 300.0, 600.0);
        assert!(t.el(300.0).is_some());
        // tipHide (scroll / a menu opens): it fades for 120 ms, then is gone - and does not come back while the
        // pointer stays on the same element
        t.hide(350.0, &[a]);
        t.update(&g, &layers, &[a], 400.0, 600.0);
        assert!(t.el(400.0).is_some());
        t.update(&g, &layers, &[a], 480.0, 600.0);
        assert!(t.el(480.0).is_none());
        t.update(&g, &layers, &[a], 2000.0, 600.0);
        assert!(t.el(2000.0).is_none());
        // the pointer moves to an element without a tip and back: armed again, shows 300 ms later
        t.update(&g, &layers, &[b], 2100.0, 600.0);
        t.update(&g, &layers, &[a], 2200.0, 600.0);
        t.update(&g, &layers, &[a], 2500.0, 600.0);
        assert!(t.el(2500.0).is_some());
        // leaving it: a fade, not a jump
        assert!(t.update(&g, &layers, &[], 2600.0, 600.0));
        assert!(t.el(2650.0).is_some());
        t.update(&g, &layers, &[], 2730.0, 600.0);
        assert!(t.el(2730.0).is_none());
    }

    #[test]
    fn placement_follows_the_drawing() {
        // above, centred: an 18 px icon at (300, 200); bubble 100 x 23
        assert_eq!(place((300.0, 200.0, 18.0, 18.0), 100.0, 23.0, 600.0), (259.0, 170.0, false));
        // near the top row: below
        assert_eq!(place((300.0, 60.0, 18.0, 18.0), 100.0, 23.0, 600.0), (259.0, 85.0, true));
        // clamped 8 px inside both edges
        assert_eq!(place((0.0, 200.0, 18.0, 18.0), 100.0, 23.0, 600.0).0, 8.0);
        assert_eq!(place((590.0, 200.0, 18.0, 18.0), 100.0, 23.0, 600.0).0, 492.0);
        // wider than the window: the drawing's clamp gives the lower bound first (v < a -> a)
        assert_eq!(place((10.0, 200.0, 18.0, 18.0), 700.0, 23.0, 600.0).0, 8.0);
    }
}
