//! The Mouse tab's buttons (Order 090, keyboard-v8.html): the BIG mouse picture first ("Buttons"; the five buttons of the
//! mice the app knows - a generic 5-button mouse otherwise), a click on a button = the shared button window (what it does +
//! its Sound, `pages::btnwin`), and the "Click sounds" card at the foot (switch, Sound: Same as keyboard / a click style,
//! Volume). Its settings live with the Keyboard tab's (`keyboard::prefs`, one engine for every sound).
//!
//! A mouse button's job is honest about what the app can do without a hook (boss A_090_01): the wheel click and the side
//! buttons can "Also press" a key / button, run an Action or a Macro - the button still does its own job too, and nothing
//! runs while a game is in front. Left and right click keep only their own job (a sound of their own is fine).

use super::*;
use crate::pages::btnwin::{self, HearOf, MOut, MacroEd, MacroFor, Out, SoundEd, BUTTON_WORDS};
use crate::pages::keyboard::{glue, prefs::Prefs};
use crate::ui::pieces::keyfield::{self, Show};
use crate::ui::pieces::{dialog, tinput, tip};
use bu_keysound::binds::{Bind, Preset};
use bu_keysound::synth::MouseButtonClass;
use bu_keysound::{ClickStyle, Dev, Pack, CLICK_STYLES};

pub(super) const K_MB: Key = key("mouse.btn");
const K_MBT: Key = key("mouse.btnt");
const K_BW: Key = key("mouse.bw");
const K_BMODE: Key = key("mouse.bmode");
const K_BACT: Key = key("mouse.bact");
const K_BATEXT: Key = key("mouse.batext");
const K_BABROWSE: Key = key("mouse.babrowse");
const K_BALSO: Key = key("mouse.balso");
const K_BALSOP: Key = key("mouse.balsop");
const K_BRESET: Key = key("mouse.breset");
const K_BSND: Key = key("mouse.bsnd");
const K_BMAC: Key = key("mouse.bmac");
const K_BMENU: Key = key("mouse.bmenu");
const K_CS: Key = key("mouse.cs");
const K_CSND: Key = key("mouse.csnd");
const K_CPLAY: Key = key("mouse.cplay");
const K_CVOL: Key = key("mouse.cvol");
const K_CTIP: Key = key("mouse.ctip");

/// The window's width (the key window's).
const BW_W: f32 = 420.0;

/// The buttons, by their number (= `bu_rawin::MOUSE_*`, the sound layers' and jobs' slot): name, class.
pub const BUTTONS: [(u16, &str, MouseButtonClass); 5] = [
    (0, "Left button", MouseButtonClass::Left),
    (1, "Right button", MouseButtonClass::Right),
    (2, "Wheel click", MouseButtonClass::Middle),
    (3, "Back (side)", MouseButtonClass::Side),
    (4, "Forward (side)", MouseButtonClass::Side),
];

pub fn button_name(slot: u16) -> &'static str {
    BUTTONS.iter().find(|b| b.0 == slot).map(|b| b.1).unwrap_or("Button")
}

fn class_of(slot: u16) -> Option<MouseButtonClass> {
    BUTTONS.iter().find(|b| b.0 == slot).map(|b| b.2)
}

/// What a button's window shows it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BMode {
    Normal,
    Also,
    Action,
    Macro,
}

const MODES: [(BMode, &str); 4] = [(BMode::Normal, "Normal"), (BMode::Also, "Also press"), (BMode::Action, "Action"), (BMode::Macro, "Macro")];

/// Which list is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BPop {
    Action,
    Also,
    Click,
}

/// The mouse picture = mouse-v9.html's SVG (viewBox 18 6 130 228): the body outline (it stops at the two side buttons), the L / R
/// halves (the left and right button), the wheel and the two slim side buttons. This file holds the whole picture (shapes, hit
/// boxes, painting) so the look is easy to swap.
const VB: (f32, f32, f32, f32) = (18.0, 6.0, 130.0, 228.0);
const BODY: &str = "M85 8 C 48 8 26 30 26 68 L 26 160 C 26 205 50 232 85 232 C 120 232 144 205 144 160 L 144 68 C 144 30 122 8 85 8 Z";
const HALF_L: &str = "M85 9 C 49 9 27 30 27 68 L 27 108 L 85 108 Z";
const HALF_R: &str = "M85 9 C 121 9 143 30 143 68 L 143 108 L 85 108 Z";
/// the cross line runs the body's width; the middle line stops above and below the wheel
const LINES: [(f32, f32, f32, f32); 3] = [(27.0, 108.0, 143.0, 108.0), (85.0, 8.5, 85.0, 30.0), (85.0, 68.0, 85.0, 108.0)];
/// the side buttons' places cut out of the body outline (the drawing's mask): x, y, w, h, radius
const CUTS: [(f32, f32, f32, f32, f32); 2] = [(22.5, 118.0, 7.0, 26.0, 3.5), (22.5, 149.0, 7.0, 26.0, 3.5)];
/// (slot, shape, hit rects (x0, y0, x1, y1) in the picture's units). The halves' hit boxes follow their rounded top; the side
/// buttons' are a little wider than they are drawn so a slim button is easy to click. Back (3) is the lower side button.
type Shape = (u16, &'static str, &'static [(f32, f32, f32, f32)]);
const SHAPES: [Shape; 5] = [
    (0, HALF_L, &[(55.0, 9.0, 85.0, 30.0), (36.0, 30.0, 85.0, 55.0), (27.0, 55.0, 85.0, 108.0)]),
    (1, HALF_R, &[(85.0, 9.0, 115.0, 30.0), (85.0, 30.0, 134.0, 55.0), (85.0, 55.0, 143.0, 108.0)]),
    (2, "M77 38a8 8 0 0 1 16 0V60a8 8 0 0 1-16 0Z", &[(77.0, 30.0, 93.0, 68.0)]),
    (4, "M22.5 121.5a3.5 3.5 0 0 1 7 0V140.5a3.5 3.5 0 0 1-7 0Z", &[(18.0, 115.0, 34.0, 146.0)]),
    (3, "M22.5 152.5a3.5 3.5 0 0 1 7 0V171.5a3.5 3.5 0 0 1-7 0Z", &[(18.0, 147.0, 34.0, 178.0)]),
];
/// The picture's column (the drawing's `.mz{flex:0 0 124px}`) and its height = the Your mouse card's.
pub(super) const PIC_COL_W: f32 = 124.0;
pub(super) const PIC_H: f32 = 184.0;

#[derive(Default)]
pub(super) struct Btns {
    loaded: bool,
    test: bool,
    prefs: Prefs,
    /// The open window's button.
    sel: Option<u16>,
    opened: f64,
    /// A mode picked that has nothing set yet (Also press waiting for its key).
    want: Option<BMode>,
    listening: bool,
    pop: Option<(BPop, (f32, f32, f32, f32))>,
    press: Option<(Key, (f32, f32, f32, f32))>,
    snd: SoundEd,
    med: MacroEd,
    err: Option<String>,
    /// A slider being dragged (the volume), saved on release.
    dragging: bool,
}

impl Btns {
    pub fn load(&mut self, test: bool) {
        self.test = test;
        self.prefs = crate::services::with(|s| Prefs::load(&s.store)).unwrap_or_default();
        self.loaded = true;
    }

    fn ensure(&mut self, test: bool) {
        if !self.loaded {
            self.load(test);
        }
    }

    /// Writes the settings and brings the engine + the mouse buttons' jobs to them.
    fn save(&mut self) {
        let prefs = self.prefs.clone();
        let test = self.test;
        let errs = crate::services::with(|s| {
            prefs.save(&mut s.store);
            if test {
                return Vec::new();
            }
            glue::publish(&prefs.binds, &prefs.macros);
            glue::publish_mouse(&prefs.mouse_binds);
            let dir = glue::packs_dir(s.store.folder());
            let _ = glue::apply(&prefs, &dir);
            glue::sync_mouse(s, &prefs.mouse_binds)
        })
        .unwrap_or_default();
        self.err = errs.into_iter().next().map(|(_, e)| e);
    }

    fn mode_of(&self, slot: u16) -> BMode {
        match self.prefs.mouse_binds.get(slot) {
            Some(Bind::Also(_) | Bind::AlsoClick(_)) => BMode::Also,
            Some(Bind::Preset(_) | Bind::App(_)) => BMode::Action,
            Some(Bind::Macro(_)) => BMode::Macro,
            None => BMode::Normal,
        }
    }

    /// Plays buttons `ids` as the Click sounds would (works with the sounds off; silent in a test copy).
    fn hear(&self, ids: &[u16]) {
        if self.test {
            return;
        }
        let s = self.prefs.engine_settings();
        if !glue::engine().status().enabled {
            // (the engine's own copy of the settings names the click to play)
            glue::engine().update(s.clone());
        }
        let mouse = |slot: u16| class_of(slot);
        let pad = |_: u16| None;
        btnwin::hear(&HearOf { dev: Dev::Mouse, pack: Some(&s.pack), volume: s.mouse_volume, play_on: s.play_on, mouse: &mouse, pad: &pad }, &self.prefs.mouse, ids);
    }

    // ------------------------------------------------------------------ the picture

    /// The mouse picture (left of the Your mouse card) + the small line under it.
    pub fn picture(&mut self, cx: &mut Cx, test: bool) -> El {
        self.ensure(test);
        let s = PIC_H / VB.3;
        let ox = (PIC_COL_W - VB.2 * s) / 2.0;
        let mut looks: Vec<(u16, f32, bool)> = Vec::new();
        for (slot, _, _) in SHAPES {
            let on = cx.hovered(idx(K_MB, slot as usize)) || self.sel == Some(slot);
            let hv = cx.tr(idx(K_MBT, slot as usize), 1, if on { 1.0 } else { 0.0 }, 120.0, EASE);
            looks.push((slot, hv, self.prefs.mouse_binds.get(slot).is_some()));
        }
        let paint = El::paint(move |g, (x, y, _, _)| paint_mouse(g, x + ox, y, s, &looks)).abs(0.0, 0.0, f32::NAN, f32::NAN).size(PIC_COL_W, PIC_H).no_hit();
        let mut col = El::block().size(PIC_COL_W, PIC_H).none().child(paint);
        for (slot, _, rects) in SHAPES {
            for (x0, y0, x1, y1) in rects {
                col = col.child(
                    El::block()
                        .abs(ox + (x0 - VB.0) * s, (y0 - VB.1) * s, f32::NAN, f32::NAN)
                        .size((x1 - x0) * s, (y1 - y0) * s)
                        .on_click(idx(K_MB, slot as usize))
                        .cursor(Cursor::Hand)
                        .title(button_name(slot)),
                );
            }
        }
        // `.mz .hint{position:absolute;left:-10px;right:-10px;top:100%;margin-top:4px;font-size:10px;color:var(--fg3);text-align:center}`
        // - under the mouse, moves nothing
        col.child(
            El::row()
                .abs(-10.0, PIC_H + 4.0, f32::NAN, f32::NAN)
                .w(PIC_COL_W + 20.0)
                .justify(JustifyContent::CENTER)
                .no_hit()
                .child(El::text("Click a button to change it", Font::new(10.0, 400), FG3(), lh(10.0, 1.35)).none()),
        )
    }

    // ------------------------------------------------------------------ Click sounds (the card at the foot)

    pub fn click_card(&mut self, cx: &mut Cx, test: bool) -> Vec<El> {
        self.ensure(test);
        let on = self.prefs.s.mouse_on;
        let info = tip::rq(cx, K_CTIP, tip::Rq::Info, 16.0, "Only which button went down or up is heard - never where the mouse is", false);
        let head = group::row(true, vec![group::lbl("Click sounds", Some("A soft click on every mouse button")), group::ctl(vec![info, toggle::toggle(cx, K_CS, on, false)])]);
        let name = match self.prefs.s.mouse_click {
            None => "Same as keyboard",
            Some(c) => c.name(),
        };
        let snd = dropdown::dropdown(cx, K_CSND, name, Some(190.0));
        let play = btnwin::play_btn(cx, K_CPLAY, "Play this sound");
        let v = self.prefs.s.mouse_volume;
        let vol = slider::slider(cx, K_CVOL, f32::from(v) / 100.0, 150.0, 20.0, slider::default());
        let rows = El::col()
            .items(AlignItems::STRETCH)
            .child(group::row(false, vec![group::lbl("Sound", None), group::ctl(vec![snd, play])]))
            .child(group::row(false, vec![group::lbl("Volume", None), group::ctl(vec![vol, slider::value_label(&format!("{v} %"))])]));
        let rows = if on { rows } else { rows.opacity(0.38).no_hit() };
        vec![group::gh("Click sounds"), group::grp(vec![head, rows])]
    }

    // ------------------------------------------------------------------ the button's window

    pub fn window(&mut self, cx: &mut Cx) -> Option<El> {
        let mut kids: Vec<El> = Vec::new();
        if let Some(slot) = self.sel {
            kids.push(self.button_window(cx, slot));
            let ids = [slot];
            if let Some(m) = self.snd.popup(cx, K_BSND, &self.prefs.mouse, &ids) {
                kids.push(m);
            }
            let cur = match self.prefs.mouse_binds.get(slot) {
                Some(Bind::Macro(id)) => Some(id.clone()),
                _ => None,
            };
            if let Some(m) = self.med.popup(cx, K_BMAC, &self.prefs.macros, cur.as_deref(), MacroFor::Mouse) {
                kids.push(m);
            }
        }
        if let Some((p, a)) = self.pop {
            let owned = self.list(p);
            let rows: Vec<Row> = owned.iter().map(|(l, head, on)| if *head { Row::Section(l.as_str()) } else { Row::Item(It::tick(l.as_str(), *on)) }).collect();
            let at = Place::Under(a.0, a.1, a.2, a.3);
            let menu = if rows.len() > 14 { mitems::menu_scroll(cx, K_BMENU, &rows, at, a.2.max(170.0)) } else { mitems::menu(cx, K_BMENU, &rows, at, a.2.max(170.0)) };
            kids.push(menu.z(40));
        }
        (!kids.is_empty()).then(|| El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(crate::ui::WIN_W, crate::ui::WIN_H).no_hit().children(kids))
    }

    /// The rows of an open list: (text, heading?, ticked).
    fn list(&self, p: BPop) -> Vec<(String, bool, bool)> {
        let mut v = Vec::new();
        match p {
            BPop::Action => {
                let cur = self.sel.and_then(|s| self.prefs.mouse_binds.get(s)).and_then(|b| if let Bind::Preset(p) = b { Some(p.id()) } else { None });
                let mut group = "";
                for p in Preset::SIMPLE.iter().cloned().chain([Preset::OpenApp(String::new()), Preset::OpenFolder(String::new()), Preset::OpenWeb(String::new())]) {
                    if p.group() != group {
                        group = p.group();
                        v.push((group.to_string(), true, false));
                    }
                    v.push((p.name().to_string(), false, cur == Some(p.id())));
                }
            }
            BPop::Also => {
                let cur = self.sel.and_then(|s| self.prefs.mouse_binds.get(s)).cloned();
                v.push(("Mouse".into(), true, false));
                for (n, name, _) in BUTTONS {
                    // (never a button that has a job of its own: two buttons could set each other off)
                    if Some(n) != self.sel && self.prefs.mouse_binds.get(n).is_none() {
                        v.push((name.into(), false, cur == Some(Bind::AlsoClick(n as u8))));
                    }
                }
                v.push(("Keys".into(), true, false));
                for (vk, name) in ALSO_KEYS {
                    v.push((name.to_string(), false, cur == Some(Bind::Also(vec![vk]))));
                }
            }
            BPop::Click => {
                v.push(("Same as keyboard".into(), false, self.prefs.s.mouse_click.is_none()));
                for c in CLICK_STYLES {
                    v.push((c.name().into(), false, self.prefs.s.mouse_click == Some(c)));
                }
            }
        }
        v
    }

    fn button_window(&mut self, cx: &mut Cx, slot: u16) -> El {
        let jobs = glue::MOUSE_JOB_SLOTS.contains(&slot);
        let mode = self.want.unwrap_or_else(|| self.mode_of(slot));
        let labels: Vec<&str> = MODES.iter().map(|m| m.1).collect();
        let on = MODES.iter().position(|m| m.0 == mode).unwrap_or(0);
        let seg_el = seg::seg(cx, K_BMODE, &labels, on, true);
        let seg_el = if jobs { seg_el } else { seg_el.opacity(0.38).no_hit() };
        let sub_t = if mode == BMode::Normal { "as Windows made it" } else { "changed" };
        let state = El::row().center().gap(8.0).child(seg_el).child(El::block().flex1()).child(El::text(sub_t, Font::new(11.5, 400), FG3(), lh(11.5, 1.35)).none());
        let mut kids = vec![state.margin(0.0, 0.0, 10.0, 0.0)];
        let passthrough = "The button still does its own job too \u{b7} not while a game is in front";
        if !jobs {
            kids.push(group::gf("Left and right click always keep their own job \u{b7} a sound of their own is fine"));
        } else {
            match mode {
                BMode::Normal => kids.push(group::gf("Pick what this button should do")),
                BMode::Also => {
                    let cur = match self.prefs.mouse_binds.get(slot) {
                        Some(Bind::Also(k)) => Some(btnwin::combo_text(k)),
                        Some(Bind::AlsoClick(b)) => Some(button_name(u16::from(*b)).to_string()),
                        _ => None,
                    };
                    let show = if self.listening { Show::Listening(None) } else if let Some(t) = cur.as_deref() { Show::Set(t) } else { Show::Empty };
                    let field = keyfield::keyfield(cx, K_BALSO, show, self.opened, true).w(160.0);
                    let pick = dropdown::dropdown(cx, K_BALSOP, "Pick from a list", None);
                    kids.push(group::grp(vec![group::row(true, vec![group::lbl("Presses", None), group::ctl(vec![field, pick])])]));
                    kids.push(group::gf("Press a key, or pick a button \u{b7} the button still does its own job too \u{b7} not while a game is in front"));
                }
                BMode::Action => {
                    let cur = match self.prefs.mouse_binds.get(slot) {
                        Some(Bind::Preset(p)) => p.name().to_string(),
                        _ => "Choose an action".into(),
                    };
                    let mut rows = vec![group::row(true, vec![group::lbl("Does", None), group::ctl(vec![dropdown::dropdown(cx, K_BACT, &cur, Some(220.0))])])];
                    if let Some(Bind::Preset(p)) = self.prefs.mouse_binds.get(slot) {
                        if let Some(t) = p.target() {
                            let (what, ph) = match p {
                                Preset::OpenWeb(_) => ("Website", "https://\u{2026}"),
                                Preset::OpenFolder(_) => ("Folder", "C:\\Users\\\u{2026}\\Documents"),
                                _ => ("App or file", "C:\\Program Files\\\u{2026}\\app.exe"),
                            };
                            let field = tinput::kdin(cx, K_BATEXT, t, ph, 200.0);
                            let browse = if matches!(p, Preset::OpenWeb(_)) { El::block() } else { link::link(cx, K_BABROWSE, "Browse\u{2026}", 12.0) };
                            rows.push(group::row(false, vec![group::lbl(what, None), group::ctl(vec![field, browse])]));
                        }
                    }
                    kids.push(group::grp(rows));
                    kids.push(group::gf(passthrough));
                }
                BMode::Macro => {
                    let cur = match self.prefs.mouse_binds.get(slot) {
                        Some(Bind::Macro(id)) => Some(id.clone()),
                        _ => None,
                    };
                    let also: Vec<String> = cur.as_deref().map(|id| self.uses_of(id, slot)).unwrap_or_default();
                    let macros = self.prefs.macros.clone();
                    kids.extend(self.med.view(cx, K_BMAC, &macros, cur.as_deref(), MacroFor::Mouse, true, &also));
                    kids.push(group::gf("The button still does its own job too"));
                }
            }
            if let Some(e) = &self.err {
                kids.push(keyfield::error_line(e));
            }
            if mode != BMode::Normal {
                let mut r = El::row().center().margin(10.0, 0.0, 0.0, 0.0);
                if mode == BMode::Macro && matches!(self.prefs.mouse_binds.get(slot), Some(Bind::Macro(_))) {
                    r = r.child(MacroEd::delete_link(cx, K_BMAC));
                }
                kids.push(r.child(El::block().flex1()).child(link::link(cx, K_BRESET, "Reset this button", 12.0)));
            }
        }
        kids.push(El::block().h(1.0).bg(crate::ui::HAIR()).margin(14.0, 0.0, 0.0, 0.0));
        let names = |f: &str| f.to_string();
        kids.push(self.snd.view(cx, K_BSND, &self.prefs.mouse, &[slot], &BUTTON_WORDS, &names));
        let body = cx.scroll_box(sub(K_BW, "body"), kids).items(AlignItems::STRETCH).style(|s| {
            s.flex_shrink = 1.0;
            s.min_size.height = taffy::style::LengthPercentageAuto::length(0.0);
        });
        dialog::dialog(cx, K_BW, BW_W, button_name(slot), vec![body], vec![], true, self.opened)
    }

    /// The other buttons / keys that run macro `id` ("Also on F11").
    fn uses_of(&self, id: &str, slot: u16) -> Vec<String> {
        let mut v: Vec<String> = self.prefs.mouse_binds.iter().filter(|(s, b)| *s != slot && matches!(b, Bind::Macro(m) if m == id)).map(|(s, _)| button_name(s).to_string()).collect();
        let keys = self.prefs.binds.keys_of_macro(id).len();
        if keys > 0 {
            v.push(if keys == 1 { "1 key".into() } else { format!("{keys} keys") });
        }
        v
    }

    // ------------------------------------------------------------------ input

    fn open_button(&mut self, slot: u16, now: f64) {
        self.sel = Some(slot);
        self.opened = now;
        self.want = None;
        self.listening = false;
        self.pop = None;
        self.err = None;
        self.snd.reset();
        self.med.reset();
        self.med.opened(now);
    }

    fn close(&mut self) {
        self.sel = None;
        self.want = None;
        self.listening = false;
        self.pop = None;
        self.snd.reset();
        self.med.reset();
    }

    /// Is a window or a list of this part open?
    pub fn open(&self) -> bool {
        self.sel.is_some() || self.pop.is_some()
    }

    /// A press beside the window / list (the frame's dismiss).
    pub fn dismiss(&mut self) -> bool {
        if self.pop.take().is_some() {
            return true;
        }
        if self.snd.escape() || self.med.escape() {
            return true;
        }
        if self.sel.is_some() {
            self.close();
            return true;
        }
        false
    }

    fn set_bind(&mut self, slot: u16, b: Option<Bind>) {
        match b {
            Some(b) => {
                let _ = self.prefs.mouse_binds.set(slot, b);
            }
            None => {
                self.prefs.mouse_binds.remove(slot);
            }
        }
        self.want = None;
        self.save();
    }

    /// One event; true = it was this part's.
    pub fn event(&mut self, ev: &Ev, cx: &mut Cx) -> bool {
        // the window's parts first
        if let Some(slot) = self.sel {
            let ids = [slot];
            let o: Out = self.snd.event(ev, cx, K_BSND, &mut self.prefs.mouse, &ids, self.test, false);
            if o.changed || o.save {
                if o.save {
                    self.save();
                } else if !self.test {
                    glue::engine().set_layers(Dev::Mouse, self.prefs.mouse.clone());
                }
            }
            if o.play {
                self.hear(&ids);
            }
            if o.used {
                return true;
            }
            let cur = match self.prefs.mouse_binds.get(slot) {
                Some(Bind::Macro(id)) => Some(id.clone()),
                _ => None,
            };
            if self.want.unwrap_or_else(|| self.mode_of(slot)) == BMode::Macro {
                let mo: MOut = self.med.event(ev, cx, K_BMAC, &mut self.prefs.macros, cur.as_deref(), MacroFor::Mouse);
                if let Some(t) = &mo.toast {
                    cx.toast(t);
                }
                if let Some(id) = mo.chose {
                    if id.is_empty() {
                        // the macro was deleted: every button / key that ran it goes back to normal
                        let gone: Vec<u16> = self.prefs.mouse_binds.iter().filter(|(_, b)| matches!(b, Bind::Macro(m) if Some(m) == cur.as_ref())).map(|(s, _)| s).collect();
                        for s in gone {
                            self.prefs.mouse_binds.remove(s);
                        }
                        if let Some(c) = &cur {
                            self.prefs.binds.drop_macro(c);
                        }
                        self.want = None;
                        self.save();
                    } else {
                        self.set_bind(slot, Some(Bind::Macro(id)));
                    }
                } else if mo.save {
                    self.save();
                }
                if mo.used {
                    return true;
                }
            }
        }
        match ev {
            Ev::Press(k, x, _, r) => {
                self.press = Some((*k, *r));
                if *k == K_CVOL {
                    self.set_volume(slider::value_at(*r, *x));
                    self.dragging = true;
                    return true;
                }
                false
            }
            Ev::Drag(k, x, _, r) if *k == K_CVOL => {
                self.set_volume(slider::value_at(*r, *x));
                true
            }
            Ev::Release(k) if *k == K_CVOL => {
                self.dragging = false;
                self.save();
                true
            }
            Ev::Click(k) => self.clicked(*k, cx),
            Ev::Key(k, vk) if *k == K_BALSO && self.listening => {
                cx.used = true;
                if *vk == 0x1B {
                    self.listening = false;
                    return true;
                }
                if let (Some(slot), Some(c)) = (self.sel, crate::pages::keyboard::combo_of(*vk)) {
                    self.listening = false;
                    self.set_bind(slot, Some(Bind::Also(c)));
                }
                true
            }
            Ev::Blur(k) if *k == K_BALSO => {
                self.listening = false;
                true
            }
            Ev::Char(k, c) if *k == K_BATEXT => {
                self.edit_target(|t| nbox::type_char(t, &mut false, *c, bu_keysound::binds::MAX_TARGET, nbox::Filter::Any));
                true
            }
            Ev::Key(k, vk) if *k == K_BATEXT && *vk == 0x08 => {
                self.edit_target(|t| {
                    t.pop();
                });
                true
            }
            _ => false,
        }
    }

    fn edit_target(&mut self, f: impl FnOnce(&mut String)) {
        let Some(slot) = self.sel else { return };
        if let Some(Bind::Preset(p)) = self.prefs.mouse_binds.get(slot).cloned() {
            let mut t = p.target().unwrap_or("").to_string();
            f(&mut t);
            let np = match p {
                Preset::OpenApp(_) => Preset::OpenApp(t),
                Preset::OpenFolder(_) => Preset::OpenFolder(t),
                _ => Preset::OpenWeb(t),
            };
            let _ = self.prefs.mouse_binds.set(slot, Bind::Preset(np));
            self.save();
        }
    }

    fn set_volume(&mut self, t: f32) {
        self.prefs.s.mouse_volume = (t.clamp(0.0, 1.0) * 100.0).round() as u8;
        if !self.test && self.prefs.s.mouse_on {
            glue::engine().update(self.prefs.engine_settings());
        }
    }

    fn clicked(&mut self, k: Key, cx: &mut Cx) -> bool {
        let rect = self.press.filter(|(pk, _)| *pk == k).map(|(_, r)| r).unwrap_or((0.0, 0.0, 0.0, 0.0));
        // an open list owns the next click
        if let Some((p, _)) = self.pop {
            self.pop = None;
            let list = self.list(p);
            if let Some(i) = (0..list.len()).find(|i| k == idx(K_BMENU, *i)) {
                if list[i].1 {
                    return true;
                }
                self.chose(p, i, &list[i].0, cx);
            }
            return true;
        }
        for (slot, _, _) in BUTTONS {
            if k == idx(K_MB, slot as usize) {
                self.open_button(slot, cx.now);
                return true;
            }
        }
        if k == K_CS {
            self.prefs.s.mouse_on = !self.prefs.s.mouse_on;
            self.save();
            cx.toast(if self.prefs.s.mouse_on { "Click sounds on" } else { "Click sounds off \u{b7} the app stops listening to the mouse" });
            return true;
        }
        if k == K_CSND {
            self.pop = Some((BPop::Click, rect));
            return true;
        }
        if k == K_CPLAY {
            self.hear(&[0]);
            return true;
        }
        let Some(slot) = self.sel else { return false };
        if k == sub(K_BW, "x") || k == sub(K_BW, "out") {
            self.close();
            return true;
        }
        if k == sub(K_BW, "win") {
            return true;
        }
        for (i, (m, _)) in MODES.iter().enumerate() {
            if k == idx(K_BMODE, i) {
                if !glue::MOUSE_JOB_SLOTS.contains(&slot) {
                    return true;
                }
                self.err = None;
                self.listening = false;
                match m {
                    BMode::Normal => self.set_bind(slot, None),
                    BMode::Also => {
                        self.want = Some(BMode::Also);
                        if !matches!(self.prefs.mouse_binds.get(slot), Some(Bind::Also(_) | Bind::AlsoClick(_))) {
                            self.listening = true;
                            cx.focus(Some(K_BALSO));
                        }
                    }
                    BMode::Action => {
                        self.want = Some(BMode::Action);
                        if !matches!(self.prefs.mouse_binds.get(slot), Some(Bind::Preset(_))) {
                            self.set_bind(slot, Some(Bind::Preset(Preset::PlayPause)));
                        }
                    }
                    BMode::Macro => {
                        self.want = Some(BMode::Macro);
                        if !matches!(self.prefs.mouse_binds.get(slot), Some(Bind::Macro(_))) {
                            if let Some(id) = self.prefs.macros.first().map(|m| m.id.clone()) {
                                self.set_bind(slot, Some(Bind::Macro(id)));
                                self.want = Some(BMode::Macro);
                            }
                        }
                    }
                }
                return true;
            }
        }
        if k == K_BALSO {
            self.listening = true;
            cx.focus(Some(K_BALSO));
            return true;
        }
        if k == K_BALSOP {
            self.pop = Some((BPop::Also, rect));
            return true;
        }
        if k == K_BACT {
            self.pop = Some((BPop::Action, rect));
            return true;
        }
        if k == K_BABROWSE {
            if let Some(Bind::Preset(p)) = self.prefs.mouse_binds.get(slot).cloned() {
                let picked = if matches!(p, Preset::OpenFolder(_)) { cx.pick_folder("Pick the folder") } else { cx.pick_file("Pick an app or file", &[("Programs and files", "*.*")]) };
                if let Some(path) = picked {
                    let np = if matches!(p, Preset::OpenFolder(_)) { Preset::OpenFolder(path) } else { Preset::OpenApp(path) };
                    self.set_bind(slot, Some(Bind::Preset(np)));
                    self.want = Some(BMode::Action);
                }
            }
            return true;
        }
        if k == K_BRESET {
            self.set_bind(slot, None);
            self.med.reset();
            cx.toast(&format!("{} is back as Windows made it", button_name(slot)));
            return true;
        }
        false
    }

    fn chose(&mut self, p: BPop, _i: usize, label: &str, cx: &mut Cx) {
        match p {
            BPop::Click => {
                self.prefs.s.mouse_click = CLICK_STYLES.into_iter().find(|c| c.name() == label);
                self.save();
                self.hear(&[0]);
            }
            BPop::Action => {
                let Some(slot) = self.sel else { return };
                let p = Preset::SIMPLE.iter().cloned().chain([Preset::OpenApp(String::new()), Preset::OpenFolder(String::new()), Preset::OpenWeb(String::new())]).find(|p| p.name() == label);
                if let Some(p) = p {
                    self.set_bind(slot, Some(Bind::Preset(p)));
                    self.want = Some(BMode::Action);
                }
            }
            BPop::Also => {
                let Some(slot) = self.sel else { return };
                if let Some((n, _, _)) = BUTTONS.iter().find(|b| b.1 == label) {
                    self.set_bind(slot, Some(Bind::AlsoClick(*n as u8)));
                } else if let Some((vk, _)) = ALSO_KEYS.iter().find(|k| k.1 == label) {
                    self.set_bind(slot, Some(Bind::Also(vec![*vk])));
                }
                self.listening = false;
            }
        }
        let _ = cx;
    }

    pub fn describe(&self) -> String {
        format!("sel={:?} mouse_on={} click={:?} binds={} layers={}", self.sel, self.prefs.s.mouse_on, self.prefs.s.mouse_click.map(ClickStyle::name), self.prefs.mouse_binds.len(), self.prefs.mouse.len())
    }

    /// Tests: the settings as this part holds them.
    #[cfg(test)]
    pub fn prefs(&self) -> &Prefs {
        &self.prefs
    }
}

/// Keys the "Also press" list offers (the remap list's kind of keys): (virtual key, name).
const ALSO_KEYS: [(u16, &str); 16] = [
    (0x1B, "Esc"),
    (0x0D, "Enter"),
    (0x20, "Space"),
    (0x09, "Tab"),
    (0x08, "Backspace"),
    (0x2E, "Delete"),
    (0x24, "Home"),
    (0x23, "End"),
    (0x21, "Page Up"),
    (0x22, "Page Down"),
    (0x26, "Up arrow"),
    (0x28, "Down arrow"),
    (0x25, "Left arrow"),
    (0x27, "Right arrow"),
    (0x7C, "F13"),
    (0x7D, "F14"),
];

/// The drawing's line colour (`#cfd5e2`); on the light glass the page's grey.
fn line_col() -> Rgba {
    if crate::ui::is_light() {
        FG2()
    } else {
        Rgba::rgb(207, 213, 226)
    }
}

/// Paints the mouse (mouse-v9.html's SVG) with its viewBox origin at (x, y), scaled by `s`: the body outline (cut at the side
/// buttons), the middle lines, then each button - hover / open = a soft blue fill, blue = it does something.
/// `looks` = (slot, hover 0..1, does something).
fn paint_mouse(g: &crate::gfx::Gfx, x: f32, y: f32, s: f32, looks: &[(u16, f32, bool)]) {
    use skia_safe as sk;
    let t0 = g.transform();
    g.set_transform(&(windows_numerics::Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: x - VB.0 * s, M32: y - VB.1 * s } * t0));
    let ink = line_col();
    // the body outline, without the two side buttons' places (the drawing's mask)
    let cv = g.cv();
    cv.save();
    for (cx, cy, w, h, r) in CUTS {
        cv.clip_rrect(sk::RRect::new_rect_xy(sk::Rect::from_xywh(cx, cy, w, h), r, r), sk::ClipOp::Difference, true);
    }
    g.stroke_geom_ex(&g.path(BODY), 1.5, ink, false, true, 1.0);
    cv.restore();
    // the halves' hover fill (no outline of their own), then the lines over it
    for (slot, d, _) in SHAPES.iter().take(2) {
        let Some((_, hv, _)) = looks.iter().find(|l| l.0 == *slot).copied() else { continue };
        g.fill_geom(&g.path(d), ACC().mul_a(0.22 * hv));
    }
    for (x1, y1, x2, y2) in LINES {
        g.line(x1, y1, x2, y2, 1.5, ink, false);
    }
    for (slot, d, _) in SHAPES.iter().skip(2) {
        let Some((_, hv, blue)) = looks.iter().find(|l| l.0 == *slot).copied() else { continue };
        let p = g.path(d);
        let base = if blue { 0.12 } else { 0.0 };
        g.fill_geom(&p, ACC().mul_a(base + (0.22 - base) * hv));
        g.stroke_geom_ex(&p, if blue { 2.0 } else { 1.5 }, if blue { ACC() } else { ink }, false, true, 1.0);
    }
    g.set_transform(&t0);
}

#[cfg(test)]
mod tests {
    use super::super::Mouse;
    use super::*;
    use crate::gfx::Gfx;
    use crate::pages::Env;
    use crate::ui::cx::State;

    fn opened() -> Mouse {
        let mut m = Mouse::default();
        m.open(&Env { test: true, frozen: true, ..Env::default() }, 0.0);
        let rs = m.svc.as_mut().map(|s| s.settle(5000)).unwrap_or_default();
        for r in rs {
            m.take(r, 0.0);
        }
        m
    }

    fn with_cx<R>(f: impl FnOnce(&mut Cx) -> R) -> R {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(1000.0, false, &g, &mut st).for_page("cur");
        f(&mut cx)
    }

    fn click(m: &mut Mouse, k: Key) {
        with_cx(|cx| m.event(&Ev::Click(k), cx));
    }

    /// The page (and its popup layer) painted with the app's own painter, off-screen: `full` = the whole page, else the
    /// window's 600 x 520 with the popup on top.
    fn paint(m: &mut Mouse, path: &str, full: bool, scroll: f32) {
        paint_with(m, path, full, scroll, &[], None);
    }

    /// `paint` with the pointer over `hover` and the focus in `focus`.
    fn paint_with(m: &mut Mouse, path: &str, full: bool, scroll: f32, hover: &[Key], focus: Option<Key>) {
        let g = Gfx::new(1.0);
        let icons = crate::icons::Icons::new();
        let mut st = State::default();
        st.hover = hover.to_vec();
        st.focus = focus;
        let mut cx = Cx::new(5000.0, false, &g, &mut st).for_page("cur");
        let kids = m.build(&mut cx);
        let root = El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
        let laid = crate::ui::lay::Laid::new(&g, root, 600.0, None);
        let h = if full { laid.height.ceil() } else { crate::ui::WIN_H };
        let mut s = crate::gfx::new_surface(600, h as i32).unwrap();
        g.begin(s.canvas());
        g.fill_rect(0.0, 0.0, 600.0, h, Rgba::rgb(20, 24, 40));
        g.end();
        let base = s.image_snapshot();
        g.begin(s.canvas());
        laid.paint(&g, &icons, 0.0, -scroll, Some(&base));
        g.end();
        if !full {
            if let Some(pop) = m.popup(&mut cx) {
                let base = s.image_snapshot();
                let l2 = crate::ui::lay::Laid::new(&g, El::block().w(600.0).h(h).child(pop), 600.0, Some(h));
                g.begin(s.canvas());
                l2.paint(&g, &icons, 0.0, 0.0, Some(&base));
                g.end();
            }
        }
        // SAFETY: COM for the WIC encoder on this test thread.
        let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
        let px = crate::png::from_surface(&mut s);
        crate::png::save_png(&px, path).expect("save");
    }

    const OUT: &str = r"C:\BoylerUtilities-scratch/P098";

    /// Order 090: pictures of the Mouse tab (run: cargo test -p bu-app mouse::btns -- --ignored).
    #[test]
    #[ignore]
    fn pictures() {
        let _ = std::fs::create_dir_all(OUT);
        let mut m = opened();
        with_cx(|cx| {
            let _ = m.build(cx);
        });
        m.btns.prefs.s.mouse_on = true;
        m.btns.prefs.s.mouse_volume = 30;
        m.btns.prefs.mouse.set(4, bu_keysound::Layer { press: Some("click.wav".into()), pitch: -2.0, ..Default::default() }).unwrap();
        m.btns.prefs.mouse_binds.set(4, Bind::Preset(Preset::PlayPause)).unwrap();
        paint(&mut m, &format!("{OUT}\\mouse_page.png"), true, 0.0);
        paint(&mut m, &format!("{OUT}\\mouse_top.png"), false, 0.0);
        click(&mut m, idx(K_MB, 4));
        paint(&mut m, &format!("{OUT}\\mouse_window_forward.png"), false, 0.0);
        click(&mut m, sub(K_BW, "x"));
        click(&mut m, idx(K_MB, 2));
        paint(&mut m, &format!("{OUT}\\mouse_window_wheel.png"), false, 0.0);
        click(&mut m, idx(K_BMODE, 1));
        paint(&mut m, &format!("{OUT}\\mouse_window_also.png"), false, 0.0);
        click(&mut m, sub(K_BW, "x"));
        click(&mut m, idx(K_MB, 0));
        paint(&mut m, &format!("{OUT}\\mouse_window_left.png"), false, 0.0);
    }

    const OUT094: &str = r"C:\BoylerUtilities-scratch\ML094";

    /// Order 094: the Mouse tab's top as the app paints it (run: cargo test -p bu-app picture_094 -- --ignored). The Forward
    /// side button does something (blue), like the drawing's.
    #[test]
    #[ignore]
    fn picture_094() {
        let _ = std::fs::create_dir_all(OUT094);
        let mut m = opened();
        with_cx(|cx| {
            let _ = m.build(cx);
        });
        m.btns.prefs.mouse_binds.set(4, Bind::Preset(Preset::PlayPause)).unwrap();
        paint(&mut m, &format!("{OUT094}/app_top.png"), false, 0.0);
        // a DPI that is none of the three: Custom lights up and shows it
        with_cx(|cx| {
            for c in "2400".chars() {
                m.event(&Ev::Char(K_DPIN, c), cx);
            }
            m.event(&Ev::Key(K_DPIN, 0x0D), cx);
        });
        let rs = m.svc.as_mut().map(|s| s.settle(5000)).unwrap_or_default();
        for r in rs {
            m.take(r, 0.0);
        }
        paint(&mut m, &format!("{OUT094}/app_top_custom.png"), false, 0.0);
        // the Custom box being typed in, and the pointer over the wheel
        m.dpi_text = "26".into();
        paint_with(&mut m, &format!("{OUT094}/app_top_typing.png"), false, 0.0, &[], Some(K_DPIN));
        paint_with(&mut m, &format!("{OUT094}/app_top_hover_wheel.png"), false, 0.0, &[idx(K_MB, 2)], None);
        paint_with(&mut m, &format!("{OUT094}/app_top_hover_left.png"), false, 0.0, &[idx(K_MB, 0)], None);
    }

    /// A button's window: the jobs, the sounds and their settings land in the shared settings; left / right keep their job.
    #[test]
    fn a_buttons_window_sets_its_job_and_its_sound() {
        let mut m = opened();
        with_cx(|cx| {
            let _ = m.build(cx);
        });
        // the forward button: Action -> Play / pause
        click(&mut m, idx(K_MB, 4));
        assert_eq!(m.btns.sel, Some(4));
        click(&mut m, idx(K_BMODE, 2));
        assert_eq!(m.btns.prefs.mouse_binds.get(4), Some(&Bind::Preset(Preset::PlayPause)));
        // Also press: a key pressed on the field
        click(&mut m, idx(K_BMODE, 1));
        assert!(m.btns.listening);
        with_cx(|cx| m.event(&Ev::Key(K_BALSO, 0x7C), cx));
        assert!(matches!(m.btns.prefs.mouse_binds.get(4), Some(Bind::Also(v)) if v.last() == Some(&0x7C)));
        // the pack's sound off for this button only
        click(&mut m, sub(K_BSND, "pack"));
        assert!(!m.btns.prefs.mouse.of(4).pack_on);
        // Reset this button: the job goes, the sound setting stays
        click(&mut m, K_BRESET);
        assert!(m.btns.prefs.mouse_binds.get(4).is_none());
        assert!(!m.btns.prefs.mouse.of(4).pack_on);
        // the window closes with its x
        click(&mut m, sub(K_BW, "x"));
        assert_eq!(m.btns.sel, None);
        // left click: no job can be set (the modes are greyed), a sound can
        click(&mut m, idx(K_MB, 0));
        click(&mut m, idx(K_BMODE, 2));
        assert!(m.btns.prefs.mouse_binds.get(0).is_none(), "left click keeps its own job");
        click(&mut m, sub(K_BSND, "pack"));
        assert!(!m.btns.prefs.mouse.of(0).pack_on);
    }

    #[test]
    fn click_sounds_switch_sound_and_volume() {
        let mut m = opened();
        with_cx(|cx| {
            let _ = m.build(cx);
        });
        assert!(!m.btns.prefs.s.mouse_on, "off by default");
        click(&mut m, K_CS);
        assert!(m.btns.prefs.s.mouse_on);
        m.btns.press = Some((K_CSND, (300.0, 400.0, 190.0, 28.0)));
        click(&mut m, K_CSND);
        assert!(m.btns.pop.is_some());
        // the list: Same as keyboard, Silent switch, Optical, Micro-switch, Deep click
        click(&mut m, idx(K_BMENU, 4));
        assert_eq!(m.btns.prefs.s.mouse_click, Some(ClickStyle::Deep));
        with_cx(|cx| {
            m.event(&Ev::Press(K_CVOL, 100.0, 10.0, (0.0, 0.0, 200.0, 20.0)), cx);
            m.event(&Ev::Release(K_CVOL), cx);
        });
        assert_eq!(m.btns.prefs.s.mouse_volume, 50);
    }
}
