//! Order 116: the privacy eye - a small eye button at the right end of a page's header (Notes, Screenshots). Click = the
//! page's private content blurs / shows again; right-click = one menu row "Blur every time I open this tab" (a tick, saved
//! per page in that page's own settings). The page decides WHAT blurs (`El::blur(eye::BLUR)` on it while `eye.on()`).
//!
//! Use in a page (the page keeps one `Eye` and gives it ONE key):
//! ```ignore
//! const K_EYE: Key = key("shot.eye");
//! // close():          self.eye.reset();                       (the next open reads "blur every time" again)
//! // build():          self.eye.load(cx);                      (first build after open: on = "blur every time")
//! //                   header right: self.eye.button(cx, K_EYE)
//! //                   private boxes: if self.eye.on() { el.blur(eye::BLUR) } else { el }
//! // event():          if self.eye.event(ev, cx, K_EYE) { return; }
//! // popup():          if let Some(m) = self.eye.popup(cx, K_EYE) { return Some(m); }
//! // popup_dismiss():  self.eye.dismiss();
//! ```
//! Look: the icon-only header button `.dlx` (28 x 28, radius 7, `--ctl-h` on hover, `--fg2` -> `--fg`); while the blur is on
//! the icon is the struck-through eye in the accent colour (`--ico-on`). The menu is `mitems::menu` at the mouse.

use super::mitems::{self, It, Place, Row};
use crate::anim::EASE;
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, sub, Cursor, El, Key};
use crate::ui::{cmix, CTL_H, FG, FG2, ICO_ON};

/// "Blurred, let's say 40 %" (the owner, Oct 23): the blur's strength, as CSS `filter: blur(8px)` = 40 % of a 20 px blur
/// (20 px leaves nothing but colour). Text and pictures become unreadable shapes; their colours and layout still show.
pub const BLUR: f32 = 8.0;

/// The page setting (its own section of settings.cfg) behind "Blur every time I open this tab".
pub const ALWAYS: &str = "eye.always";

/// The menu's one row.
pub const ROW: &str = "Blur every time I open this tab";

/// One page's eye: whether its private content is blurred now, the saved "every time" switch, its right-click menu.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Eye {
    loaded: bool,
    on: bool,
    always: bool,
    /// the right-click menu is open at (x, y) - window DIPs
    menu: Option<(f32, f32)>,
}

impl Eye {
    /// The first build after the tab opened: read "blur every time" - when it is on, the page opens blurred.
    pub fn load(&mut self, cx: &Cx) {
        if !self.loaded {
            let always = cx.get_bool(ALWAYS, false);
            self.load_with(always);
        }
    }

    /// `load` with the setting given (tests, a page that read it already).
    pub fn load_with(&mut self, always: bool) {
        if !self.loaded {
            self.loaded = true;
            self.always = always;
            self.on = always;
        }
    }

    /// The tab was left / the menu closed: forget everything (the next open reads the setting again).
    pub fn reset(&mut self) {
        *self = Eye::default();
    }

    /// Is the page's private content blurred now?
    pub fn on(&self) -> bool {
        self.on
    }

    /// The saved "Blur every time I open this tab".
    pub fn always(&self) -> bool {
        self.always
    }

    /// The right-click menu is open.
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// The button for the page header's right end.
    pub fn button(&self, cx: &mut Cx, key: Key) -> El {
        let hv = cx.hover_t(key, 120.0, EASE);
        let on = cx.tr(key, 7, if self.on { 1.0 } else { 0.0 }, 150.0, EASE);
        let col = cmix(cmix(FG2(), FG(), hv), ICO_ON(), on);
        El::block()
            .size(28.0, 28.0)
            .none()
            .radius(7.0)
            .bg(CTL_H().mul_a(hv))
            .place_center()
            .on_click(key)
            .cursor(Cursor::Hand)
            .title(if self.on { "Show" } else { "Blur" })
            .child(El::icon(if self.on { "eyeoff" } else { "eye" }, 18.0, 1.5, col).no_hit())
    }

    /// Its input; true = it was the eye's (the page does nothing more with it).
    pub fn event(&mut self, ev: &Ev, cx: &mut Cx, key: Key) -> bool {
        match ev {
            Ev::Click(k) if *k == key => {
                self.on = !self.on;
                self.menu = None;
            }
            Ev::Context(k, x, y) if *k == key => self.menu = Some((*x, *y)),
            Ev::Click(k) if *k == idx(sub(key, "m"), 0) => {
                self.always = !self.always;
                cx.set_bool(ALWAYS, self.always);
                self.menu = None;
            }
            _ => return false,
        }
        cx.dirty = true;
        true
    }

    /// The right-click menu (window coordinates), while open.
    pub fn popup(&self, cx: &mut Cx, key: Key) -> Option<El> {
        let (x, y) = self.menu?;
        Some(mitems::menu(cx, sub(key, "m"), &[Row::Item(It::tick(ROW, self.always))], Place::At(x, y), 150.0))
    }

    /// The frame dismissed the popup (a click outside, Esc): true = the eye's menu was open.
    pub fn dismiss(&mut self) -> bool {
        self.menu.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::Gfx;
    use crate::ui::cx::State;
    use crate::ui::el::key;

    const K: Key = key("t.eye");

    fn ev(e: &mut Eye, g: &Gfx, st: &mut State, x: Ev) -> bool {
        let mut cx = Cx::new(0.0, false, g, st);
        e.event(&x, &mut cx, K)
    }

    /// `El::blur` really blurs what is inside the box: the text's sharpest pixel goes soft and its ink spreads past the
    /// text's own lines (a CPU raster of the same box, sharp and blurred).
    #[test]
    fn blur_softens_the_text_inside() {
        use crate::gfx::{Font, Rgba};
        let g = Gfx::new(1.0);
        let icons = crate::icons::Icons::new();
        let shot = |b: f32| {
            let mut t = El::block().w(200.0).pad(20.0, 20.0, 20.0, 20.0).child(El::text("Secret words", Font::new(13.0, 400), Rgba(1.0, 1.0, 1.0, 1.0), 18.0));
            if b > 0.0 {
                t = t.blur(b);
            }
            let laid = crate::ui::lay::Laid::new(&g, El::block().w(200.0).h(60.0).bg(Rgba(0.0, 0.0, 0.0, 1.0)).child(t), 200.0, Some(60.0));
            let mut s = crate::gfx::new_surface(200, 60).unwrap();
            g.begin(s.canvas());
            laid.paint(&g, &icons, 0.0, 0.0, None);
            g.end();
            crate::png::from_surface(&mut s)
        };
        let (sharp, soft) = (shot(0.0), shot(BLUR));
        let max = |p: &crate::png::Pixels, y0: u32, y1: u32| (y0..y1).flat_map(|y| (0..p.w).map(move |x| (y * p.w + x) as usize * 4)).map(|i| p.data[i + 1]).max().unwrap_or(0);
        assert!(max(&sharp, 20, 38) > 200, "the sharp text is bright");
        assert!(max(&soft, 20, 38) < 120, "blurred: no bright pixel left ({})", max(&soft, 20, 38));
        assert_eq!(max(&sharp, 0, 12), 0, "nothing above the sharp text");
        assert!(max(&soft, 0, 18) > 0, "the blur reaches above the text's line");
    }

    /// Review fix: a blurred box that also has a brightness filter closes its layers in order - what comes after it is sharp.
    #[test]
    fn what_follows_a_blurred_bright_box_is_not_blurred() {
        use crate::gfx::{Font, Rgba};
        let g = Gfx::new(1.0);
        let icons = crate::icons::Icons::new();
        let white = Rgba(1.0, 1.0, 1.0, 1.0);
        let root = El::block()
            .w(200.0)
            .h(80.0)
            .bg(Rgba(0.0, 0.0, 0.0, 1.0))
            .child(El::block().h(40.0).pad(10.0, 10.0, 10.0, 10.0).blur(BLUR).brightness(1.1).child(El::text("Hidden", Font::new(13.0, 400), white, 18.0)))
            .child(El::block().h(40.0).pad(10.0, 10.0, 10.0, 10.0).child(El::text("Sharp", Font::new(13.0, 400), white, 18.0)));
        let laid = crate::ui::lay::Laid::new(&g, root, 200.0, Some(80.0));
        let mut s = crate::gfx::new_surface(200, 80).unwrap();
        g.begin(s.canvas());
        laid.paint(&g, &icons, 0.0, 0.0, None);
        g.end();
        let p = crate::png::from_surface(&mut s);
        let max = (50..68u32).flat_map(|y| (0..p.w).map(move |x| (y * p.w + x) as usize * 4)).map(|i| p.data[i + 1]).max().unwrap_or(0);
        assert!(max > 200, "the box after it stays sharp ({max})");
    }

    #[test]
    fn opens_unblurred_unless_every_time_is_on() {
        let mut e = Eye::default();
        e.load_with(false);
        assert!(!e.on());
        let mut e = Eye::default();
        e.load_with(true);
        assert!(e.on() && e.always());
        // a second load in the same visit changes nothing (the page's click stays)
        e.on = false;
        e.load_with(true);
        assert!(!e.on());
        e.reset();
        e.load_with(true);
        assert!(e.on(), "the next visit reads it again");
    }

    #[test]
    fn a_click_blurs_and_shows_again() {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut e = Eye::default();
        e.load_with(false);
        assert!(ev(&mut e, &g, &mut st, Ev::Click(K)));
        assert!(e.on());
        assert!(ev(&mut e, &g, &mut st, Ev::Click(K)));
        assert!(!e.on());
        assert!(!ev(&mut e, &g, &mut st, Ev::Click(key("t.other"))), "other keys are the page's");
    }

    #[test]
    fn right_click_opens_the_one_row_menu_and_its_click_flips_every_time() {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut e = Eye::default();
        e.load_with(false);
        assert!(ev(&mut e, &g, &mut st, Ev::Context(K, 500.0, 70.0)));
        assert!(e.menu_open());
        {
            let mut cx = Cx::new(0.0, false, &g, &mut st);
            assert!(e.popup(&mut cx, K).is_some());
        }
        assert!(ev(&mut e, &g, &mut st, Ev::Click(idx(sub(K, "m"), 0))));
        assert!(e.always() && !e.menu_open());
        assert!(!e.on(), "the setting is for the next opens; the blur now stays as it is");
        assert!(ev(&mut e, &g, &mut st, Ev::Context(K, 500.0, 70.0)));
        assert!(e.dismiss() && !e.menu_open());
    }
}
