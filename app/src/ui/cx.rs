//! The state model of the pages (Order 014): pages own their VALUES; the `Cx` owns what CSS owns in the drawing -
//! hover (`:hover`), press (`:active`), focus, and every `transition` - keyed by element key, so pieces stay plain
//! builder functions. Events reach the page as `Ev` (clicks by key, drags with the element's box, keys, wheel).

use std::collections::{HashMap, HashSet};

use super::el::{El, Key};
use crate::anim::{Bezier, Tween};
use crate::gfx::Gfx;

/// What a page is told about input. Every event names the element (its key) it happened on.
#[derive(Clone, Debug, PartialEq)]
pub enum Ev {
    /// pressed and released on the same clickable element
    Click(Key),
    /// the button went down on an element (sliders start dragging here): pointer (x, y) and the element's box
    Press(Key, f32, f32, (f32, f32, f32, f32)),
    /// the pointer moved while the button is held after a `Press` on this element
    Drag(Key, f32, f32, (f32, f32, f32, f32)),
    /// the button came up after a `Press` on this element
    Release(Key),
    /// a key went down while the element has focus (virtual-key code); Esc / Enter / arrows / Backspace...
    Key(Key, u16),
    /// a typed character while the element has focus
    Char(Key, char),
    /// the element lost focus (a click elsewhere, Esc)
    Blur(Key),
    /// the right button was released on an element (the drawing's `contextmenu`): the element's key (the clickable one
    /// under the pointer, else the innermost keyed one) and the pointer (window DIPs) - open a menu there
    Context(Key, f32, f32),
    /// files / folders were dropped on an element (the drawing's drag and drop): their full paths
    Drop(Key, Vec<String>),
    /// Order 045: the mouse wheel over an element marked `El::wheel_steps` (a number field, the DPI box - the drawing's
    /// `wheel` listener with `preventDefault`): +1 = up a notch, -1 = down. The page does not scroll then.
    Wheel(Key, i32),
    /// Order 045: files are dragged over the window from Explorer (Windows' drag and drop): the element under the pointer
    /// (the innermost keyed one), None = they left it / were dropped / the drag ended - the drawing's `dragenter` /
    /// `dragleave` (`.sdz.over`)
    DragOver(Option<Key>),
}

/// The key of page-level keys: `Ev::Key(PAGE, vk)` = a key went down while NO element has focus (Ctrl+F, Ctrl+A, Delete,
/// Esc...). Esc still closes the menu when the page does not use it (`Cx::used`).
pub const PAGE: Key = super::el::key("page");

#[derive(Default)]
pub struct State {
    tw: HashMap<(Key, u32), Tween>,
    seen: HashSet<(Key, u32)>,
    /// keys of the element under the pointer and all its keyed ancestors (CSS :hover)
    pub hover: Vec<Key>,
    /// the same chain for the element the button went down on, while it is held (CSS :active)
    pub active: Vec<Key>,
    pub focus: Option<Key>,
    /// a transition is running: the page needs frames
    pub busy: bool,
    /// when each key started hovering (tooltips that show after a delay)
    pub hover_since: HashMap<Key, f64>,
    /// the scroll offset of every scrolling box (`Cx::scroll_box`) by its key; the frame's wheel changes it
    pub scroll_y: HashMap<Key, f32>,
    /// the pointer in PAGE coordinates (the page's own boxes: y from the page's top, scroll included), None = not over
    /// the page
    pub pointer: Option<(f32, f32)>,
    /// elements whose hover point the last build read (`Cx::hover_point`): the frame builds again on every move over them
    pub watch: Vec<Key>,
    /// until this time (ms) every transition jumps to its target: a tab just opened shows its real values at once,
    /// not a slide from the defaults it was built with before its service answered (the owner Oct 8: settings "snap to the
    /// new settings when the page is fully loaded")
    pub settle_until: f64,
    /// Order 045: key fields that were listening (since when) - the moment one takes its key its caps drop in
    /// (`pieces::keyfield`); then when that drop-in started
    pub kf_listen: HashMap<Key, f64>,
    pub kf_set: HashMap<Key, f64>,
    /// Order 047: the earliest moment (ms) the last build's picture changes by itself while nothing moves until then (a
    /// caret's next blink, a toast's end, a clock's next minute): the menu sleeps until then and builds again at it.
    /// `None` = nothing due. Cleared at each build of the shown page (`Cx::wake_at` sets it).
    pub wake: Option<f64>,
}

impl State {
    /// Forget transitions of elements that were not built in the last pass (a page's rows went away).
    pub fn sweep(&mut self) {
        let seen = std::mem::take(&mut self.seen);
        self.tw.retain(|k, _| seen.contains(k));
    }
    pub fn clear(&mut self) {
        *self = State::default();
    }
}

/// What a page asks the frame to do after its event (the frame owns these popups).
#[derive(Clone, Debug)]
pub enum Req {
    /// open the reset review ("Back to how your PC was" / "Windows defaults") under the link's box
    Reset(crate::undo::Kind, (f32, f32, f32, f32)),
    /// the same over EVERY tab, grouped by tab (Settings › Reset)
    ResetAll(crate::undo::Kind, (f32, f32, f32, f32)),
    /// a small note above the window's bottom edge (1.8 s)
    Toast(String),
    /// scroll the page so this element is in view (the drawing's `scrollIntoView({block:'nearest'})`)
    ScrollTo(Key),
    /// scroll the page to this offset (px from the top; the drawing's `pg.scrollTop = y`)
    ScrollY(f32),
    /// show another tab (its page id), then hand it `target` (`Page::jump`: a key row, a popup to open) - the drawing's
    /// `jump(id, el)`, e.g. Voice's "Mic mute" -> Audio's Mute settings
    ShowTab(String, Option<String>),
    /// drag these files out of the menu (Windows' own drag, run by the frame after the event; only from `Page::event`)
    DragOut(Vec<String>),
}

/// The keyboard modifiers held when the event happened (Ctrl / Shift / Alt).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

thread_local! {
    /// the modifiers of the queued input event being handled (taken when its message arrived)
    static EVENT_MODS: std::cell::Cell<Option<Mods>> = const { std::cell::Cell::new(None) };
}

impl Mods {
    /// What the keyboard holds at the message being handled now (GetKeyState follows the message queue: inside a window
    /// procedure it is that message's moment).
    pub fn read() -> Mods {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT};
        let down = |vk: u16| unsafe { GetKeyState(vk as i32) } < 0;
        Mods { ctrl: down(VK_CONTROL.0), shift: down(VK_SHIFT.0), alt: down(VK_MENU.0) }
    }
    /// The modifiers of the event the frame is delivering: those taken when its message arrived (main.rs queues them with
    /// the event - a quick Ctrl release before the queue is drained no longer loses a Ctrl+click), else `read()`.
    pub fn now() -> Mods {
        EVENT_MODS.with(|m| m.get()).unwrap_or_else(Mods::read)
    }
    /// main.rs: the modifiers of the event it is about to hand on (None after it).
    pub fn set_event(m: Option<Mods>) {
        EVENT_MODS.with(|c| c.set(m));
    }
}

/// What a page's `build` and `event` get: the clock, reduced motion, the painter (text widths), the CSS state, and the
/// app's services (settings, undo records, jobs, keys) for THIS page.
pub struct Cx<'a> {
    pub now: f64,
    /// prefers-reduced-motion (the drawing's RM): transitions .01 s
    pub rm: bool,
    pub g: &'a Gfx,
    pub st: &'a mut State,
    /// the page wants to be rebuilt (a value changed)
    pub dirty: bool,
    /// the page this Cx belongs to (its settings scope, its undo records)
    pub(crate) page: &'static str,
    /// a click is being delivered right now (the only time a job may start)
    pub(crate) in_click: bool,
    pub(crate) reqs: Vec<Req>,
    /// the modifiers held at this event (Ctrl+click / Shift+click ranges)
    pub mods: Mods,
    /// the page used this event (a page-level key: Esc then does not close the menu)
    pub used: bool,
}

/// A property id for `Cx::tr` (any small number unique within one element).
pub type Prop = u32;

impl<'a> Cx<'a> {
    pub fn new(now: f64, rm: bool, g: &'a Gfx, st: &'a mut State) -> Cx<'a> {
        Cx { now, rm, g, st, dirty: false, page: "", in_click: false, reqs: Vec::new(), mods: Mods::default(), used: false }
    }
    pub(crate) fn for_page(mut self, page: &'static str) -> Self {
        self.page = page;
        self
    }
    pub fn hovered(&self, k: Key) -> bool {
        self.st.hover.contains(&k)
    }
    pub fn active(&self, k: Key) -> bool {
        self.st.active.contains(&k)
    }
    pub fn focused(&self, k: Key) -> bool {
        self.st.focus == Some(k)
    }
    pub fn focus(&mut self, k: Option<Key>) {
        self.st.focus = k;
    }
    /// Where the pointer is while it hovers `k` (page coordinates, like the page's own boxes), e.g. a curve's "x -> y %"
    /// readout; the page is built again on each pointer move while it hovers `k`.
    pub fn hover_point(&mut self, k: Key) -> Option<(f32, f32)> {
        if !self.st.watch.contains(&k) {
            self.st.watch.push(k);
        }
        if self.hovered(k) {
            self.st.pointer
        } else {
            None
        }
    }
    /// How long the element has been hovered (ms), 0 if not.
    pub fn hover_age(&self, k: Key) -> f64 {
        self.st.hover_since.get(&k).map(|t| self.now - t).unwrap_or(0.0)
    }

    /// A CSS transition of property `p` of element `k` towards `target` (`dur` ms, `ease`): the value for this frame.
    /// The first time an element is built its value starts AT the target (CSS: no transition on the first style).
    pub fn tr(&mut self, k: Key, p: Prop, target: f32, dur: f64, ease: Bezier) -> f32 {
        self.tr_delayed(k, p, target, dur, 0.0, ease)
    }

    pub fn tr_delayed(&mut self, k: Key, p: Prop, target: f32, dur: f64, delay: f64, ease: Bezier) -> f32 {
        let id = (k, p);
        self.st.seen.insert(id);
        let now = self.now;
        let dur = if self.rm { 10.0 } else { dur };
        let t = self.st.tw.entry(id).or_insert_with(|| Tween::new(target as f64));
        if (t.target() - target as f64).abs() > 1e-6 {
            if now < self.st.settle_until {
                t.jump(target as f64);
            } else if delay > 0.0 {
                t.set_delayed(now, target as f64, dur, delay, ease);
            } else {
                t.set(now, target as f64, dur, ease);
            }
        }
        if t.busy(now) {
            self.st.busy = true;
        }
        t.value(now) as f32
    }

    /// Order 047: build the page again at `t` (ms) - for a picture that changes at a known moment while nothing moves
    /// before it (a caret's next blink, a countdown's next second, a toast's end). Unlike `st.busy` no frames run until
    /// then: the menu sleeps (no CPU) and wakes at `t`.
    pub fn wake_at(&mut self, t: f64) {
        self.st.wake = Some(self.st.wake.map_or(t, |w| w.min(t)));
    }

    /// Order 047: build again at the next step of a clock that flips every `period` ms counted from `from` (a caret
    /// blinking 530 ms on / off: `wake_every(530.0, 0.0)`).
    pub fn wake_every(&mut self, period: f64, from: f64) {
        let n = ((self.now - from) / period).floor() + 1.0;
        self.wake_at(from + n * period);
    }

    /// 0..1 hover transition (`:hover` with `transition: <dur> <ease>`).
    pub fn hover_t(&mut self, k: Key, dur: f64, ease: Bezier) -> f32 {
        let on = self.hovered(k);
        self.tr(k, 0xF001, if on { 1.0 } else { 0.0 }, dur, ease)
    }
    /// 0..1 press transition (`:active`).
    pub fn active_t(&mut self, k: Key, dur: f64, ease: Bezier) -> f32 {
        let on = self.active(k);
        self.tr(k, 0xF002, if on { 1.0 } else { 0.0 }, dur, ease)
    }
}

// ---------------------------------------------------------------- the app's services, for this page (Order 014 item 2)
use crate::jobs::{JobCtx, JobError, JobView};
use crate::settings::Scope;
use crate::undo::{Kind, Val};

impl Cx<'_> {
    /// This page's own setting (`settings.cfg`, section page:<id>).
    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        let p = self.page;
        crate::services::with(|s| s.store.bool_or(Scope::Page(p), key, default)).unwrap_or(default)
    }
    pub fn set_bool(&mut self, key: &str, v: bool) {
        let p = self.page;
        crate::services::with(|s| s.store.set_bool(Scope::Page(p), key, v));
    }
    pub fn get_i64(&self, key: &str, default: i64) -> i64 {
        let p = self.page;
        crate::services::with(|s| s.store.i64_or(Scope::Page(p), key, default)).unwrap_or(default)
    }
    pub fn set_i64(&mut self, key: &str, v: i64) {
        let p = self.page;
        crate::services::with(|s| s.store.set_i64(Scope::Page(p), key, v));
    }
    pub fn get_f64(&self, key: &str, default: f64) -> f64 {
        let p = self.page;
        crate::services::with(|s| s.store.f64_or(Scope::Page(p), key, default)).unwrap_or(default)
    }
    pub fn set_f64(&mut self, key: &str, v: f64) {
        let p = self.page;
        crate::services::with(|s| s.store.set_f64(Scope::Page(p), key, v));
    }
    pub fn get_str(&self, key: &str, default: &str) -> String {
        let p = self.page;
        crate::services::with(|s| s.store.str_or(Scope::Page(p), key, default).to_string()).unwrap_or_else(|| default.to_string())
    }
    pub fn set_str(&mut self, key: &str, v: &str) {
        let p = self.page;
        crate::services::with(|s| s.store.set_str(Scope::Page(p), key, v));
    }

    /// Record a change this page just made on the PC (old -> new): the first old value ever recorded stays as "how your
    /// PC was" for the reset line. Call it for EVERY change made through the app.
    pub fn record(&mut self, item: &str, label: &str, old: &Val, new: &Val) {
        let p = self.page;
        crate::services::with(|s| crate::undo::record(&mut s.store, p, item, label, old, new));
    }

    /// Open the reset review for this page (the reset line's links): `kind` = how it was / Windows defaults, `anchor` =
    /// the link's box (from `Ev::Press`). The page must give its `Page::resettable`.
    pub fn open_reset(&mut self, kind: Kind, anchor: (f32, f32, f32, f32)) {
        self.reqs.push(Req::Reset(kind, anchor));
    }

    /// Settings › Reset: ONE review over every tab ("Back to how your PC was" / "Windows defaults"), grouped by tab, under
    /// the link's box; the frame applies the ticked lines through each page's `Page::resettable` - which is asked of CLOSED
    /// pages too (it must build what its `apply` needs on demand).
    pub fn open_reset_all(&mut self, kind: Kind, anchor: (f32, f32, f32, f32)) {
        self.reqs.push(Req::ResetAll(kind, anchor));
    }

    /// Every key of the app (Settings › All shortcuts): each keys-manager action, its page and its key.
    pub fn actions(&self) -> Vec<crate::services::ActionInfo> {
        crate::services::with(|s| s.action_list()).unwrap_or_default()
    }

    /// Put `text` on the clipboard ("Copy all"); Err = why not (another app held it). A test copy never touches the real
    /// clipboard: the test hook's `state` shows the text as a `clip ...` line.
    pub fn copy_text(&mut self, text: &str) -> Result<(), String> {
        crate::clipboard::set_text(text)
    }

    /// A small toast above the window's bottom edge.
    pub fn toast(&mut self, text: &str) {
        self.reqs.push(Req::Toast(text.to_string()));
    }

    /// A scrolling box (CSS `overflow-y:auto` - a popup list, a tall dialog's body `.mdb`): `kids` inside a clipped box
    /// whose wheel (pointer over it) moves them first; at its ends the wheel goes on to the page. Give the box its height
    /// limit (`.max_h(..)` / `.h(..)`, and `.min_h(0.0)` inside a flex column). Its offset: `self.st.scroll_y[&key]`.
    pub fn scroll_box(&mut self, key: Key, kids: Vec<El>) -> El {
        let off = self.st.scroll_y.get(&key).copied().unwrap_or(0.0);
        let mut b = El::col().key(key).clip().child(El::col().key(super::el::sub(key, "in")).translate(0.0, -off).children(kids));
        b.scrolls = true;
        b
    }

    /// Scroll the page so the element `k` is in view (nearest edge, animated like the wheel).
    pub fn scroll_to(&mut self, k: Key) {
        self.reqs.push(Req::ScrollTo(k));
    }
    /// Show the tab `page_id` ("aud", "set", ...) and hand it `target` (its `Page::jump`), e.g. `cx.show_tab("aud",
    /// Some("mute"))`.
    pub fn show_tab(&mut self, page_id: &str, target: Option<&str>) {
        self.reqs.push(Req::ShowTab(page_id.to_string(), target.map(str::to_string)));
    }
    /// Drag files out of the menu (to Explorer, Discord...): call it from the `Ev::Drag` of the element the button went down
    /// on, once the pointer moved a few px. The frame runs Windows' drag after the event (it is modal: the button comes up
    /// inside it); the page then gets its `Ev::Release`. A test copy never starts a real drag (`describe` can show the ask).
    pub fn drag_out(&mut self, paths: Vec<String>) {
        self.reqs.push(Req::DragOut(paths));
    }
    /// End the app for the self-update's install step (Settings › Updates, after `bu_updater::update` said Ok) - from
    /// `build` (where the page sees its `cx.job(..)` Done) as well as from `event`; it works at once, even if the menu has
    /// closed meanwhile. A test copy ends too (its fake updater never swaps anything).
    pub fn exit_for_update(&mut self) {
        crate::services::request_exit();
    }
    /// Scroll the page to `y` px from its top (clamped to the page).
    pub fn scroll_y(&mut self, y: f32) {
        self.reqs.push(Req::ScrollY(y));
    }

    /// Windows' folder picker (modal, over the menu) - only while a click is delivered. A test copy never opens it: it
    /// gets the test switch `BU_PICK` (a scratch path) or None. Returns the chosen folder's full path.
    pub fn pick_folder(&mut self, title: &str) -> Option<String> {
        if !self.in_click {
            return None;
        }
        if crate::testmode::on() {
            return crate::testmode::env("BU_PICK");
        }
        crate::ui::picker::pick(title, true, &[])
    }
    /// Windows' file picker: `filters` = (name, pattern) e.g. ("Programs", "*.exe"). Same rules as `pick_folder`.
    pub fn pick_file(&mut self, title: &str, filters: &[(&str, &str)]) -> Option<String> {
        if !self.in_click {
            return None;
        }
        if crate::testmode::on() {
            return crate::testmode::env("BU_PICK");
        }
        crate::ui::picker::pick(title, false, filters)
    }

    /// `pick_file` opening in the folder `start` (Order 042).
    pub fn pick_file_in(&mut self, title: &str, filters: &[(&str, &str)], start: &str) -> Option<String> {
        if !self.in_click {
            return None;
        }
        if crate::testmode::on() {
            return crate::testmode::env("BU_PICK");
        }
        crate::ui::picker::pick_in(title, false, filters, Some(start))
    }

    /// Start slow work on a worker thread - ONLY while a click is delivered (`Ev::Click`); anywhere else it is refused
    /// ("nothing heavy on its own"). `key` names the job (e.g. "net.speed") so the page finds it again.
    pub fn start_job<F>(&mut self, key: &str, work: F) -> Result<(), String>
    where
        F: FnOnce(&JobCtx) -> Result<String, JobError> + Send + 'static,
    {
        if !self.in_click {
            return Err("a job starts only from a button".into());
        }
        crate::services::with(|s| s.start_job(key, work).map(|_| ()).map_err(|e| format!("{e:?}"))).unwrap_or_else(|| Err("no services".into()))
    }
    /// A job's progress / end (None = never started).
    pub fn job(&self, key: &str) -> Option<JobView> {
        crate::services::with(|s| s.job(key)).flatten()
    }
    /// Stop a running job (it ends as Stopped at its next check).
    pub fn stop_job(&mut self, key: &str) {
        crate::services::with(|s| {
            if let Some(v) = s.jobs.view_key(key) {
                s.jobs.stop(v.id);
            }
        });
    }

    /// What a key field for a keys-manager action shows: the key text (None = no key), listening (held modifiers, since)
    /// and the last refusal ("Already used by Mic mute").
    pub fn key_field(&self, action: &str) -> (Option<String>, Option<(Option<String>, f64)>, Option<String>) {
        crate::services::with(|s| {
            let (set, l) = s.field(action);
            (set, l, s.key_errors.get(action).cloned())
        })
        .unwrap_or((None, None, None))
    }
    /// The field was clicked: listen for a new key (keys are paused meanwhile).
    pub fn listen_key(&mut self, action: &str) {
        let now = self.now;
        crate::services::with(|s| s.listen(action, now));
    }
    /// Esc / focus lost: stop listening, keep the old key.
    pub fn stop_listening(&mut self) {
        crate::services::with(|s| s.stop_listening());
    }
    /// The field's ×: no key.
    pub fn clear_key(&mut self, action: &str) {
        crate::services::with(|s| s.clear_key(action));
    }

    /// Settings › Glass style.
    pub fn glass(&self) -> crate::settings::GlassStyle {
        crate::services::with(|s| s.glass()).unwrap_or_default()
    }
    pub fn set_glass(&mut self, g: crate::settings::GlassStyle) {
        crate::services::with(|s| s.store.set_glass(g));
    }
    /// Settings › Theme: saved; the frame switches the palette, the glass and the shadow on its next frame (Order 033).
    pub fn set_theme(&mut self, t: crate::settings::Theme) {
        crate::services::with(|s| s.store.set_theme(t));
    }
    /// Settings › "Reset the app's own settings" (the PC is not touched).
    pub fn reset_app_settings(&mut self) {
        crate::services::with(|s| s.reset_app_settings());
        self.toast("The app\u{2019}s own settings are back to how it came");
    }
}
