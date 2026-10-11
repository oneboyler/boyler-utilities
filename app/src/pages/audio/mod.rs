//! The Audio tab (menu-v22 page `aud`), Order 018: rebuilt from the shared pieces on the page API (test A's hand-painted
//! page was its start: its meter painting and level smoothing are carried over number for number) and wired to
//! crates/audio (bu-audio) + crates/micmute (bu-micmute).
//!
//! Devices: Output and Input - each a live level pill that IS the volume slider, the % you can type, the device list
//! (every device with its own switch); the Input row's mic icon mutes / unmutes, "Mute settings" under "Input" opens the
//! small popup (mute.rs). "Keep my devices". Apps: one row per app making sound (tile, name, slider in the app's colour
//! with its live level under it, %, mute). "New apps volume". The reset line.
//! v21 review: the app NAMES sit where v20 had them; the tiles are centred on their names (`.mxr .ait{transform:
//! translateY(.5px)}`, measured in the drawing per row - the report has the per-row numbers of both).

mod meter;
mod micicon;
mod mute;
mod svc;

use std::collections::HashMap;

use taffy::style::AlignItems;

use bu_audio::page::PageSnapshot;
use bu_audio::{AudioError, DeviceRow, Flow};
use bu_micmute::MicMute;

use crate::anim::EASE;
use crate::gfx::{sh, Align, Font, Rgba};
use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{key, lh, sub, Cursor, El, Key, RADIUS_PILL};
use crate::ui::pieces::{self, dropdown, group, link, reset, slider, toast, toggle};
use crate::ui::{cmix, ACC, ACC_S, CTL, CTL_H, F125, FG, FG2, FG3, HAIR, HOV, ICO, RED, SEL, TRK, WHITE, WIN_H, WIN_W};
use crate::undo::{DefaultItem, Kind, Resettable, Val};

use meter::{Face, FakeSound, Level, PeakFade};
use svc::{Cmd, Note, Svc};

const K_OUT_VOL: Key = key("aud.outvol");
const K_IN_VOL: Key = key("aud.invol");
const K_OUT_PCT: Key = key("aud.outpct");
const K_IN_PCT: Key = key("aud.inpct");
const K_OUT_PICK: Key = key("aud.outpick");
const K_IN_PICK: Key = key("aud.inpick");
const K_MIC: Key = key("aud.mic");
const K_SPK: Key = key("aud.spk");
const K_MML: Key = key("aud.mml");
const K_KEEP: Key = key("aud.keep");
const K_NEW_VOL: Key = key("aud.newvol");
const K_NEW_PCT: Key = key("aud.newpct");
const K_NEW_ON: Key = key("aud.newon");
const K_RESET: Key = key("aud.reset");
const K_REVIEW: Key = key("aud.review");
const K_RV_CANCEL: Key = key("aud.review.cancel");
const K_RV_GO: Key = key("aud.review.go");
const K_DEVM: Key = key("aud.devm");
const K_TOAST: Key = key("aud.toast");

fn k_vol(i: usize) -> Key {
    key(&format!("aud.vol.{i}"))
}
fn k_pct(i: usize) -> Key {
    key(&format!("aud.pct.{i}"))
}
fn k_mute(i: usize) -> Key {
    key(&format!("aud.mute.{i}"))
}
fn k_dev(i: usize) -> Key {
    key(&format!("aud.dev.{i}"))
}
fn k_devsw(i: usize) -> Key {
    key(&format!("aud.devsw.{i}"))
}

/// The selection colour of a typed % (Windows' highlight, as Chromium paints a selected input text).
const SEL_TEXT: Rgba = Rgba(0.0, 120.0 / 255.0, 215.0 / 255.0, 1.0);
/// A muted app's thumb: `.mxr.mu .rng::-webkit-slider-thumb{background:#c8cad2}`
const THUMB_MU: Rgba = Rgba(200.0 / 255.0, 202.0 / 255.0, 210.0 / 255.0, 1.0);

thread_local! {
    /// Order 047: the real worker's last full snapshot of this run - the page shows it at once when it opens again
    static LAST_SNAP: std::cell::RefCell<Option<std::rc::Rc<PageSnapshot>>> = const { std::cell::RefCell::new(None) };
}

/// Order 055: the meters are DATA, not motion - they step at their own rate, never at the screen's: at most every 33 ms
/// (~30 Hz) while any meter shows a level, and the worker is looked at again every 100 ms while all is silent (a sound
/// starting then waits at most that long). Between two steps nothing is asked for: no frame, no work.
const STEP_MS: f64 = 33.0;
const SILENT_MS: f64 = 100.0;

/// Order 047: the meters' levels for the live pass (devices; apps by group: level + peak, and the peak dot's opacity).
#[derive(Default)]
struct LiveLv {
    vo: Level,
    vi: Level,
    apps: HashMap<String, (Level, f32)>,
}

/// One mixer row.
struct App {
    group: String,
    name: String,
    face: Face,
    c: Rgba,
    c2: Rgba,
    /// the drawing's fake sound for the sample apps (test copies)
    mode: &'static str,
    vol: f32,
    muted: bool,
    lv: Level,
    last_sound: f64,
    /// the held peak's dot (Order 055: stepped with the levels, painted by the live pass)
    pk: PeakFade,
}

/// A % being typed.
struct Edit {
    key: Key,
    text: String,
    sel_all: bool,
    start: f64,
}

/// The device list (`.menu.devm`) under its button.
struct DevMenu {
    flow: Flow,
    x: f32,
    y: f32,
    w: f32,
    nudge: HashMap<usize, f64>,
}

/// The reset review (`.menu.rsm`) under its link.
struct Review {
    win: bool,
    /// the link that opened it
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    lines: Vec<reset::Line>,
}

struct St {
    svc: Svc,
    frozen: bool,
    outs: Vec<DeviceRow>,
    ins: Vec<DeviceRow>,
    /// (device id, volume) of the Output / Input rows
    out: Option<(String, f32)>,
    inp: Option<(String, f32)>,
    apps: Vec<App>,
    vo: Level,
    vi: Level,
    /// Order 047: the levels as the meters paint them (the live pass reads these when it paints: a moving meter repaints
    /// without the page being built again)
    live: std::rc::Rc<std::cell::RefCell<LiveLv>>,
    /// the last tick moved only the meters (the live pass repaints them; no build)
    live_only: bool,
    /// Order 055: when the worker is looked at next (a step of the meters while there is sound, else the silent rate)
    next_poll: f64,
    /// the worker's write count at the last look: the same = nothing new to copy
    seen_gen: Option<u64>,
    /// the last look found a value held back (a command's answer awaited, a slider dragged, a % typed): look again
    resync: bool,
    /// the worker's first full read has been seen (until then the page looks at it every step: it is waited for)
    answered: bool,
    /// when the page opened (the wait for the first read is not kept up for ever if the worker never answers)
    born: f64,
    /// Order 047: the last snapshot copied and its write count
    snap_cache: Option<(u64, std::rc::Rc<PageSnapshot>)>,
    /// Order 047: the last snapshot of this run, shown until the worker's first read is in
    seed: Option<std::rc::Rc<PageSnapshot>>,
    /// the slider being dragged and the pointer's offset from its thumb centre
    drag: Option<(Key, f32)>,
    /// after a change the page keeps its own values until the worker has answered
    hold_until: f64,
    edit: Option<Edit>,
    menu: Option<DevMenu>,
    /// the list just closed by a click on its own button: that click must not open it again
    just_closed: Option<Flow>,
    mic: MicMute,
    /// Order 047: the mic's mute state read off the menu's thread after a change (`tick` takes it)
    mic_read: std::sync::Arc<std::sync::Mutex<Option<bool>>>,
    muted: bool,
    /// Order 081: the default output device is muted (the Output row's speaker icon, like the mic icon of the Input row)
    out_muted: bool,
    mute: Option<mute::MuteUi>,
    review: Option<Review>,
    toast: Option<(String, f64)>,
    expect_err: bool,
    rects: HashMap<Key, (f32, f32, f32, f32)>,
    last: f64,
    fake_sound: Option<FakeSound>,
    /// Order 036: change-log entries made by this event (written with `cx.record` when the event ends)
    recs: Vec<Note>,
    /// a slider's entry (item, label, value) when its drag started: one entry per drag, written on release
    drag_before: Option<(String, String, Val)>,
    /// a reset link was clicked: the frame's shared review opens under its box
    reset_req: Option<(Kind, (f32, f32, f32, f32))>,
    /// Windows' text selection colour and caret blink time (the frame keeps them for test A's page only)
    sel: Rgba,
    caret_ms: f64,
    /// Order 047: which app rows showed as quiet (no sound for 1.5 s) at the last step: a change repaints (a .25 s fade of
    /// the row - motion - so it is a build, and 1.5 s of silence is its own hysteresis)
    quiet: Vec<bool>,
}

pub struct Audio {
    rm: bool,
    st: Option<St>,
    /// Order 036: a closed page in a test copy resets this fake (made on first use; never the PC)
    closed_fake: std::cell::OnceCell<bu_audio::SharedFake>,
}

impl Default for Audio {
    fn default() -> Self {
        Audio::new()
    }
}

/// The frame drops the pages with the menu (it doesn't always call `close` first): the mic watch must not outlive them.
impl Drop for Audio {
    fn drop(&mut self) {
        self.close();
    }
}

impl Audio {
    pub fn new() -> Audio {
        Audio { rm: false, st: None, closed_fake: std::cell::OnceCell::new() }
    }

    fn st(&mut self) -> Option<&mut St> {
        self.st.as_mut()
    }

    /// Test copies only: clicks by element name, each pressed at its laid-out box's centre (the page's own events), so a
    /// picture can show an open list / popup the way a click opens it.
    fn replay(&mut self, list: &str, now: f64) {
        let g = crate::gfx::Gfx::new(1.0);
        for name in list.split(';').filter(|n| !n.is_empty()) {
            // "aud.reset/pc" = the piece's own part sub(key("aud.reset"), "pc")
            let k = match name.split_once('/') {
                Some((a, b)) => sub(key(a), b),
                None => key(name),
            };
            let mut st = crate::ui::cx::State::default();
            let r = {
                let mut cx = Cx::new(now, self.rm, &g, &mut st);
                let pop = self.popup(&mut cx).map(|p| crate::ui::lay::Laid::new(&g, El::block().w(WIN_W).h(WIN_H).child(p), WIN_W, Some(WIN_H)));
                let kids = self.build(&mut cx);
                let page = crate::ui::lay::Laid::new(&g, El::block().w(WIN_W).pad(2.0, 26.0, 18.0, 26.0).children(kids), WIN_W, None);
                pop.and_then(|p| p.rect_of(k)).or_else(|| page.rect_of(k).map(|r| (r.0, r.1 + crate::ui::PAGE_TOP, r.2, r.3)))
            };
            let Some(r) = r else { continue };
            let mut cx = Cx::new(now, self.rm, &g, &mut st);
            self.event(&Ev::Press(k, r.0 + r.2 / 2.0, r.1 + r.3 / 2.0, r), &mut cx);
            self.event(&Ev::Release(k), &mut cx);
            self.event(&Ev::Click(k), &mut cx);
        }
    }
}

/// The uninstaller's undo with no window (Order 036): the saved switches, as `Page::background` reads them at a normal
/// start (else "Keep my devices" / "New apps volume" would be judged by the app's defaults).
pub fn load_for_undo() {
    svc::load_rules();
}

/// Settings › "Reset the app's own settings" (after the store's reset): Audio's live choices back to how the app came -
/// Mic mute's settings / sounds and keys, Keep my devices and New apps volume (the always-on watcher follows). Order 036:
/// the two switches change how Windows behaves, so a change of theirs goes into the change log (Mic mute's settings are
/// the app's own).
pub fn reset_app_settings(fake: bool) {
    mute::reset_to_defaults(fake);
    let old = svc::rules();
    svc::set_rules(svc::Rules::default());
    svc::note_rules(old, svc::Rules::default());
}

/// Mic mute is switched on (Settings › All shortcuts shows its keys dimmed / "Off" otherwise).
pub fn mic_mute_on() -> bool {
    mute::settings().on
}

/// Settings › All shortcuts lists Mic mute's one key or its two separate keys - the ones of the chosen way.
pub fn mic_action_shown(id: &str) -> bool {
    let sep = mute::settings().sep;
    match id {
        mute::A_ONE => !sep,
        mute::A_MUTE | mute::A_UNMUTE => sep,
        _ => true,
    }
}

/// Drop per-device caches (the menu closed).
pub fn reset_caches() {
    meter::reset_caches();
}

/// (test A's frame asks the page to measure device names once at open; the shared layout measures its own text now)
pub fn remember_widths(_g: &crate::gfx::Gfx, _names: &[String]) {}

/// Is the worker's Output / Input device (id, volume) what the page shows? (No copy made to ask.)
fn same_dev(w: &Option<(String, bu_audio::VolumeMute)>, p: &Option<(String, f32)>) -> bool {
    match (w, p) {
        (Some((id, v)), Some((pid, pv))) => id == pid && v.volume == *pv,
        (None, None) => true,
        _ => false,
    }
}

/// One mixer row follows the worker's row: the icon that arrived (the real layer reads icons on a helper thread), and the
/// volume / mute unless the user holds them (`free` false). True = something on the page changed.
fn follow_row(r: &mut App, a: &bu_audio::AppRow, free: bool, real: bool) -> bool {
    let mut changed = false;
    if let (Face::Glyph { .. }, Some(i)) = (&r.face, &a.look.icon) {
        if real {
            r.face = Face::Icon(meter::pixels(i));
            r.c = Rgba::hex(a.look.colour);
            r.c2 = Rgba::hex(a.look.colour2);
            changed = true;
        }
    }
    if free && (r.vol != a.volume || r.muted != a.muted) {
        r.vol = a.volume;
        r.muted = a.muted;
        changed = true;
    }
    changed
}

impl St {
    /// Mute settings opens: the fake reads in place as before; a real mic (Order 047) opens on the last known state and
    /// is read off the menu's thread.
    fn mute_ui(&self, now: f64) -> mute::MuteUi {
        if matches!(self.svc, Svc::Fake(..)) {
            mute::MuteUi::open(self.mic.clone(), now, self.frozen)
        } else {
            mute::MuteUi::open_real(self.mic.clone(), now, self.frozen, self.muted)
        }
    }

    // ============================================================== state from the worker
    fn poll(&mut self, now: f64) -> bool {
        // Order 036: the worker's change-log entries (a new default device, a device switched on / off)
        for (item, label, old, new) in self.svc.take_notes() {
            svc::log(&item, &label, &old, &new);
        }
        // Order 047: the worker's snapshot is copied only when it wrote something new (its levels every 16 ms) - not on
        // every frame of a 360 Hz screen
        let g = self.svc.gen();
        let s: std::rc::Rc<PageSnapshot> = match self.snap_cache.take() {
            Some((cg, c)) if cg == g => c,
            _ => std::rc::Rc::new(self.svc.snapshot()),
        };
        self.snap_cache = Some((g, s.clone()));
        let s = if s.ready {
            self.answered = true;
            self.seed = None;
            if !matches!(self.svc, Svc::Fake(..)) {
                LAST_SNAP.with(|l| *l.borrow_mut() = Some(s.clone()));
            }
            s
        } else if let Some(seed) = self.seed.take() {
            // (the worker's first read is not in yet: the last one of this run shows meanwhile - once)
            seed
        } else {
            return false;
        };
        let mut changed = false;
        if self.expect_err {
            if let Some(e) = &s.last_error {
                self.expect_err = false;
                let t = match e {
                    AudioError::NeedsAdmin(_) => crate::admin::NOT_CHANGED.to_string(),
                    AudioError::LastDeviceOn => "One device always stays on".to_string(),
                    e => e.to_string(),
                };
                self.toast = Some((t, now));
                changed = true;
            }
        }
        if s.outputs != self.outs || s.inputs != self.ins {
            self.outs = s.outputs.clone();
            self.ins = s.inputs.clone();
            changed = true;
        }
        let holding = now < self.hold_until;
        let (drag, edit) = (self.drag, self.edit.as_ref().map(|e| e.key));
        // (a slider being dragged / a % being typed keeps its own value; the keys are made only while one is)
        let dragging = |k: Key| drag.map(|d| d.0 == k).unwrap_or(false);
        let editing = |k: Key| edit == Some(k);
        let user = drag.is_some() || edit.is_some();
        // a value held back now is looked at again at the next step (the worker may not write anything new meanwhile)
        self.resync = holding || user;
        if !holding && !dragging(K_OUT_VOL) && !editing(K_OUT_PCT) && !same_dev(&s.output, &self.out) {
            self.out = s.output.as_ref().map(|(id, v)| (id.clone(), v.volume));
            changed = true;
        }
        let out_muted = s.output.as_ref().is_some_and(|o| o.1.muted);
        if !holding && out_muted != self.out_muted {
            self.out_muted = out_muted;
            changed = true;
        }
        if !holding && !dragging(K_IN_VOL) && !editing(K_IN_PCT) && !same_dev(&s.input, &self.inp) {
            self.inp = s.input.as_ref().map(|(id, v)| (id.clone(), v.volume));
            changed = true;
        }
        let real = !(self.frozen || matches!(self.svc, Svc::Fake(..)));
        // keep rows (and their smoothing) by app, in the worker's order
        if s.apps.len() == self.apps.len() && s.apps.iter().zip(&self.apps).all(|(a, r)| a.group == r.group) {
            // (the same apps in the same order - nearly every look: the rows follow in place, nothing is moved or made)
            for (i, (a, r)) in s.apps.iter().zip(self.apps.iter_mut()).enumerate() {
                let free = !holding && (!user || (!dragging(k_vol(i)) && !editing(k_pct(i))));
                changed |= follow_row(r, a, free, real);
            }
        } else {
            let mut rows = Vec::with_capacity(s.apps.len());
            for a in &s.apps {
                let pos = self.apps.iter().position(|r| r.group == a.group);
                let mut r = match pos {
                    Some(i) => self.apps.remove(i),
                    None => {
                        changed = true;
                        let sample = svc::sample_look(&a.look.name);
                        let (face, c, c2, mode) = match (&sample, &a.look.icon) {
                            (Some(l), _) if self.frozen || matches!(self.svc, Svc::Fake(..)) => (
                                Face::Glyph { glyph: l.glyph, a: Rgba::hex(l.a), b: Rgba::hex(l.b) },
                                Rgba::hex(l.c),
                                Rgba::hex(l.c2),
                                l.mode,
                            ),
                            (_, Some(i)) => (Face::Icon(meter::pixels(i)), Rgba::hex(a.look.colour), Rgba::hex(a.look.colour2), ""),
                            _ => {
                                // no icon (yet): the drawing's grey tile with the app's first letter-free glyph
                                let g = if a.system { "abell" } else { "apps" };
                                (Face::Glyph { glyph: g, a: Rgba::hex(0xa2abbd), b: Rgba::hex(0x6c7487) }, Rgba::hex(a.look.colour), Rgba::hex(a.look.colour2), "")
                            }
                        };
                        App { group: a.group.clone(), name: a.look.name.clone(), face, c, c2, mode, vol: a.volume, muted: a.muted, lv: Level::default(), last_sound: -1e9, pk: PeakFade::default() }
                    }
                };
                let i = rows.len();
                let free = !holding && !dragging(k_vol(i)) && !editing(k_pct(i));
                changed |= follow_row(&mut r, a, free, real);
                rows.push(r);
            }
            if rows.len() != self.apps.len() {
                changed = true;
            }
            self.apps = rows;
        }
        // levels
        let dt = (now - self.last).clamp(0.0, 50.0);
        self.last = now;
        if self.frozen {
            // the drawing's frozen sample (dom_dump's freeze): devices .70 / .62 with their peaks, apps .55 .12 .30 0 0
            self.vo = Level { l: if self.out_muted { 0.0 } else { 0.70 }, pk: if self.out_muted { 0.0 } else { 0.82 }, pt: now + 1e9 };
            self.vi = Level { l: if self.muted { 0.0 } else { 0.62 }, pk: if self.muted { 0.0 } else { 0.74 }, pt: now + 1e9 };
            const LV: [f32; 5] = [0.55, 0.12, 0.30, 0.0, 0.0];
            const PK: [f32; 5] = [0.06, 0.25, 0.04, 0.0, 0.0];
            // test pictures against the drawing with its meter loop stopped (freeze_preload.js): every level 0
            if crate::testmode::env("BU_AUD_LEVELS").as_deref() == Some("0") {
                self.vo = Level::default();
                self.vi = Level::default();
                for r in self.apps.iter_mut() {
                    r.lv = Level::default();
                }
                return changed;
            }
            for (i, r) in self.apps.iter_mut().enumerate() {
                let l = if r.muted { 0.0 } else { LV.get(i).copied().unwrap_or(0.0) };
                r.lv = Level { l, pk: if l > 0.0 { (l + PK.get(i).copied().unwrap_or(0.0)).min(1.0) } else { 0.0 }, pt: now + 1e9 };
            }
            return changed;
        }
        if let Some(fs) = &mut self.fake_sound {
            let mut sum = 0.0;
            for r in self.apps.iter_mut() {
                let t = fs.app(&r.name, r.mode, now);
                let t = if r.muted { 0.0 } else { t * r.vol.powf(0.7) };
                sum += t;
                r.lv.app(t, now, dt);
            }
            let o = fs.output(sum, now);
            self.vo.device(if self.out_muted { 0.0 } else { o }, now, dt);
            let i = if self.muted { 0.0 } else { fs.input(now) };
            self.vi.device(i, now, dt);
        } else {
            self.vo.device(if self.out_muted { 0.0 } else { s.output_level }, now, dt);
            self.vi.device(if self.muted { 0.0 } else { s.input_level }, now, dt);
            for r in self.apps.iter_mut() {
                let raw = s.app_levels.iter().find(|x| x.0 == r.group).map(|x| x.1).unwrap_or(0.0);
                r.lv.app(if r.muted { 0.0 } else { raw }, now, dt);
            }
        }
        for r in self.apps.iter_mut() {
            if r.lv.l > 0.01 {
                r.last_sound = now;
            }
        }
        changed
    }

    /// Order 047: the levels into `live`, where the meters' paint reads them (each step, and each build). Order 055: the
    /// app peak dots' fade is stepped here (it was a transition of the built page), and `live` is written in place - nothing
    /// is made or cloned unless an app came or went.
    fn sync_live(&mut self, now: f64) {
        for a in self.apps.iter_mut() {
            a.pk.set(a.lv.pk - a.lv.l > 0.02, now);
        }
        let mut lv = self.live.borrow_mut();
        lv.vo = self.vo;
        lv.vi = self.vi;
        let mut missing = lv.apps.len() != self.apps.len();
        for a in &self.apps {
            match lv.apps.get_mut(&a.group) {
                Some(e) => *e = (a.lv, a.pk.at(now)),
                None => missing = true,
            }
        }
        if missing {
            lv.apps.clear();
            for a in &self.apps {
                lv.apps.insert(a.group.clone(), (a.lv, a.pk.at(now)));
            }
        }
    }

    /// Order 047: is any meter (devices, apps) still showing a level, a peak tick or a fading peak dot? (Then the meters
    /// keep stepping - Order 055: every `STEP_MS`.)
    fn meters_moving(&self, now: f64) -> bool {
        self.vo.moving() || self.vi.moving() || self.apps.iter().any(|r| r.lv.moving() || r.pk.busy(now))
    }

    /// Order 055: which app rows are quiet now (no sound for 1.5 s) - kept in place, nothing made per call. True = a row
    /// changed (its .25 s fade is a build).
    fn quiet_flip(&mut self, now: f64) -> bool {
        let mut flip = self.quiet.len() != self.apps.len();
        self.quiet.resize(self.apps.len(), false);
        for (q, a) in self.quiet.iter_mut().zip(&self.apps) {
            let n = now - a.last_sound > 1500.0;
            if *q != n {
                *q = n;
                flip = true;
            }
        }
        flip && !self.frozen
    }

    fn run(&mut self, c: Cmd, now: f64) {
        self.svc.run(c);
        self.hold_until = now + 600.0;
        // (the worker's answer is looked for at the next step, not at the silent rate)
        self.next_poll = self.next_poll.min(now + STEP_MS);
    }

    fn value_of(&self, k: Key) -> f32 {
        if k == K_OUT_VOL || k == K_OUT_PCT {
            return self.out.as_ref().map(|o| o.1).unwrap_or(0.0);
        }
        if k == K_IN_VOL || k == K_IN_PCT {
            return self.inp.as_ref().map(|o| o.1).unwrap_or(0.0);
        }
        if k == K_NEW_VOL || k == K_NEW_PCT {
            return svc::rules().new_vol;
        }
        (0..self.apps.len()).find(|&i| k_vol(i) == k || k_pct(i) == k).map(|i| self.apps[i].vol).unwrap_or(0.0)
    }

    /// A new value for a slider / typed %.
    fn set_value(&mut self, k: Key, v: f32, now: f64) {
        let v = (v * 100.0).round().clamp(0.0, 100.0) / 100.0;
        if k == K_OUT_VOL || k == K_OUT_PCT || k == K_IN_VOL || k == K_IN_PCT {
            let out = k == K_OUT_VOL || k == K_OUT_PCT;
            let dev = if out { &mut self.out } else { &mut self.inp };
            if let Some((id, vol)) = dev {
                if (*vol - v).abs() > 1e-4 {
                    *vol = v;
                    let id = id.clone();
                    self.run(Cmd::DeviceVolume(id, v), now);
                }
            }
            return;
        }
        if k == K_NEW_VOL || k == K_NEW_PCT {
            let mut r = svc::rules();
            r.new_vol = v;
            svc::set_rules(r);
            return;
        }
        let Some(i) = (0..self.apps.len()).find(|&i| k_vol(i) == k || k_pct(i) == k) else { return };
        let out = self.out.as_ref().map(|o| o.0.clone()).unwrap_or_default();
        let a = &mut self.apps[i];
        // moving the slider or typing a % unmutes, as in Windows
        if (a.vol - v).abs() > 1e-4 || a.muted {
            a.vol = v;
            a.muted = false;
            let g = a.group.clone();
            self.run(Cmd::AppVolume(out, g, v), now);
        }
    }

    fn end_edit(&mut self, save: bool, now: f64) {
        if let Some(e) = self.edit.take() {
            if save {
                if let Ok(v) = e.text.parse::<i32>() {
                    let b = self.before(e.key);
                    self.set_value(e.key, v.clamp(0, 100) as f32 / 100.0, now);
                    self.track(b);
                }
            }
        }
    }

    fn open_menu(&mut self, flow: Flow) {
        let k = if flow == Flow::Output { K_OUT_PICK } else { K_IN_PICK };
        let (bx, by, bw, bh) = self.rects.get(&k).copied().unwrap_or((366.0, 140.0, 196.0, 24.0));
        let list = if flow == Flow::Output { &self.outs } else { &self.ins };
        let n = list.len();
        // `placeMenu(btn, 300)`: min-width max(300, the button), as wide as its widest row
        let names = list.iter().map(|d| d.device.name.clone()).collect::<Vec<_>>();
        let w = names.iter().fold(300.0f32.max(bw), |w, s| w.max(10.0 + 6.0 + 12.0 + 8.0 + 16.0 + 8.0 + text_w(s) + 10.0 + 8.0 + 44.0 + 6.0));
        // (a long list scrolls inside the box's 300 px - dropdown::menu_box)
        let h = (5.0 + 32.0 * n as f32 + 4.0 + 25.84375 + 4.0).min(300.0);
        let mut x = bx;
        let mut y = by + bh + 4.0;
        if x + w > WIN_W - 8.0 {
            x = (bx + bw - w).max(8.0);
        }
        if y + h > WIN_H - 8.0 {
            y = (by - h - 4.0).max(8.0);
        }
        // the drawing's placeMenu: Math.round(left), Math.round(top)
        self.menu = Some(DevMenu { flow, x: x.round(), y: y.round(), w, nudge: HashMap::new() });
    }

    /// Test pictures only (a frozen copy): the drawing's made-up change log (RS.aud) in the page's own review. Every other
    /// copy opens the frame's shared review over the change log (`Resettable` below).
    fn review_lines(&self, win: bool) -> Vec<reset::Line> {
        let line = |t: &str, f: &str, to: &str| reset::Line { title: t.into(), from: f.into(), to: to.into(), ticked: true, heading: None };
        if win {
            vec![line("Keep my devices", "On", "Off"), line("App volumes", "5 changed", "100 %"), line("New apps volume", "On \u{00b7} 50 %", "Off")]
        } else {
            vec![line("Default output", "Headphones (Arctis Nova)", "Speakers (Realtek)"), line("Keep my devices", "On", "Off"), line("Discord volume", "80 %", "100 %")]
        }
    }

    // ============================================================== Order 036: the change log
    /// The change-log item behind a control: "keep", "newapps", "app:<group>" (an app whose exe can't be read - "pid:" -
    /// has no item: its id would not survive a restart).
    fn item_of(&self, k: Key) -> Option<String> {
        if k == K_KEEP {
            return Some(svc::KEEP.into());
        }
        if k == K_NEW_ON || k == K_NEW_VOL || k == K_NEW_PCT {
            return Some(svc::NEWAPPS.into());
        }
        let i = (0..self.apps.len()).find(|&i| k_vol(i) == k || k_pct(i) == k || k_mute(i) == k)?;
        let g = &self.apps[i].group;
        (!g.starts_with("pid:")).then(|| svc::app_item(g))
    }

    /// An item's label and value as the page knows them now.
    fn entry(&self, item: &str) -> Option<(String, Val)> {
        let r = svc::rules();
        match item {
            svc::KEEP => Some((svc::KEEP_LABEL.into(), svc::on_val(r.keep))),
            svc::NEWAPPS => Some((svc::NEWAPPS_LABEL.into(), svc::newapps_val(r))),
            _ => {
                let g = item.strip_prefix("app:")?;
                let a = self.apps.iter().find(|a| a.group == g)?;
                Some((format!("{} volume", a.name), svc::app_val(a.vol, a.muted)))
            }
        }
    }

    /// The item behind a control with its label and value, before a change.
    fn before(&self, k: Key) -> Option<(String, String, Val)> {
        let item = self.item_of(k)?;
        let (label, v) = self.entry(&item)?;
        Some((item, label, v))
    }

    /// After a change: its entry (old → new) when the value moved.
    fn track(&mut self, before: Option<(String, String, Val)>) {
        let Some((item, label, old)) = before else { return };
        if let Some((_, new)) = self.entry(&item) {
            if new.raw != old.raw {
                self.recs.push((item, label, old, new));
            }
        }
    }

    /// The live value of an item while the page is open (the reset line's "now"; a line already back disappears).
    fn current(&self, item: &str) -> Option<Val> {
        let s = self.svc.snapshot();
        if !s.ready {
            return None;
        }
        let names = |rows: &[DeviceRow]| rows.iter().map(|d| (d.device.id.clone(), d.device.name.clone())).collect::<HashMap<_, _>>();
        match item {
            "out.default" => Some(svc::defaults_val(&s.output_defaults, &names(&s.outputs))),
            "in.default" => Some(svc::defaults_val(&s.input_defaults, &names(&s.inputs))),
            _ => {
                if let Some(id) = item.strip_prefix("dev:") {
                    return s.outputs.iter().chain(s.inputs.iter()).find(|d| d.device.id == id).map(|d| svc::on_val(d.on));
                }
                self.entry(item).map(|e| e.1)
            }
        }
    }
}

/// Windows' selection colour (COLOR_HIGHLIGHT), as Chromium paints a selected input's text.
fn sys_highlight() -> Rgba {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Graphics::Gdi::{GetSysColor, COLOR_HIGHLIGHT};
        let c = GetSysColor(COLOR_HIGHLIGHT);
        Rgba::rgb((c & 0xff) as u8, ((c >> 8) & 0xff) as u8, ((c >> 16) & 0xff) as u8)
    }
    #[cfg(not(windows))]
    SEL_TEXT
}

/// Windows' caret blink time (ms).
fn caret_blink() -> f64 {
    #[cfg(windows)]
    unsafe {
        let t = windows::Win32::UI::WindowsAndMessaging::GetCaretBlinkTime();
        if t > 0 && t < 10_000 {
            return t as f64;
        }
    }
    530.0
}

/// Text width of a 13 px name (the device list's widest row).
fn text_w(s: &str) -> f32 {
    thread_local! {
        static G: crate::gfx::Gfx = crate::gfx::Gfx::new(1.0);
    }
    G.with(|g| g.text_width(s, Font::new(13.0, 400)))
}

// =================================================================================================== building
impl Audio {
    fn build_page(&mut self, cx: &mut Cx) -> Vec<El> {
        let header = pieces::header("Audio", None);
        let Some(st) = self.st.as_mut() else { return vec![header] };
        st.sync_live(cx.now);
        // (what this build shows as quiet - a later change of it is a build again: `quiet_flip`)
        st.quiet.clear();
        st.quiet.extend(st.apps.iter().map(|a| cx.now - a.last_sound > 1500.0));
        // ---- Devices
        let out_row = dev_row(cx, st, Flow::Output);
        let in_row = dev_row(cx, st, Flow::Input);
        let r = svc::rules();
        // "Keep my devices": `.lbl.wide{flex:1;width:auto}`
        let keep = group::row(
            false,
            vec![
                dvi("plug"),
                group::lbl("Keep my devices", Some("Stop Windows switching to newly plugged-in devices (like a controller)")),
                group::ctl(vec![toggle::toggle(cx, K_KEEP, r.keep, false)]),
            ],
        )
        .min_h(44.0);
        let devices = El::block().child(group::gh("Devices")).child(group::grp(vec![out_row, in_row, keep]));
        // ---- Apps (v21: the obvious footer line is gone)
        let mut rows = Vec::new();
        for i in 0..st.apps.len() {
            rows.push(app_row(cx, st, i));
        }
        let apps = El::block().child(group::gh("Apps")).child(group::grp(rows));
        // ---- New apps volume: `.grp.nag{margin-top:14px}`
        let nag = group::grp(vec![new_row(cx, st)]).margin(14.0, 0.0, 0.0, 0.0);
        let rs = reset::reset_line(cx, K_RESET, Some("Windows defaults"));
        // (the caret's blink wakes the menu at each flip - `Cx::wake_every` in the % field; Order 047)
        vec![header, devices, apps, nag, rs]
    }
}

/// `.dvi` (22 px line icon, stroke --ico 1.5).
fn dvi(icon: &str) -> El {
    El::block().size(22.0, 22.0).none().place_center().child(El::icon(icon, 22.0, 1.5, ICO()))
}

/// A device row (`.row.dvr`: gap 12, min-height 44): icon, label (46 px), the level pill / volume slider, the %, the list.
fn dev_row(cx: &mut Cx, st: &mut St, flow: Flow) -> El {
    let out = flow == Flow::Output;
    let (kv, kp, kk) = if out { (K_OUT_VOL, K_OUT_PCT, K_OUT_PICK) } else { (K_IN_VOL, K_IN_PCT, K_IN_PICK) };
    let dev = if out { st.out.clone() } else { st.inp.clone() };
    let vol = dev.as_ref().map(|d| d.1).unwrap_or(0.0);
    let list = if out { &st.outs } else { &st.ins };
    let name = dev.as_ref().and_then(|d| list.iter().find(|x| x.device.id == d.0)).map(|d| d.device.name.clone()).unwrap_or_default();
    // v21: the Input row's mic icon IS the mute button (Order 081: the Output row's speaker icon is the same button for the
    // output device). `.dvi.dmb{width:30px;height:30px;margin:-4px;border-radius:50%;
    // transition:background .15s ease,transform .12s ease}` `:hover{background:var(--ctl-h)}` `:hover svg{stroke:var(--fg)}`
    // `:active{transform:scale(.9)}` `.m{background:rgba(255,69,58,.16)}` `.m svg{stroke:var(--red)}` `.sl{opacity:0}` `.m .sl{opacity:1}`
    let (kb, is_muted, ico, tip) = if out {
        (K_SPK, st.out_muted, "spkS", if st.out_muted { "Unmute speakers" } else { "Mute speakers" })
    } else {
        (K_MIC, st.muted, "micS", if st.muted { "Unmute mic" } else { "Mute mic" })
    };
    let icon = {
        let hv = cx.hover_t(kb, 150.0, EASE);
        let pr = cx.active_t(kb, 120.0, EASE);
        let m = cx.tr(kb, 5, if is_muted { 1.0 } else { 0.0 }, 150.0, EASE);
        let bg = cmix(CTL_H().mul_a(hv), Rgba::rgba(255, 69, 58, 0.16), m);
        let stroke = cmix(cmix(ICO(), FG(), hv), RED(), m);
        El::grid()
            .size(30.0, 30.0)
            .none()
            .margin(-4.0, -4.0, -4.0, -4.0)
            .radius(RADIUS_PILL)
            .bg(bg)
            .place_center()
            .scale(1.0 - 0.1 * pr)
            .on_click(kb)
            .cursor(Cursor::Hand)
            // `micB.title=S.muted?'Unmute mic':'Mute mic'`
            .title(tip)
            .child(El::icon(ico, 22.0, 1.5, stroke).class_op("sl", m).no_hit())
    };
    // `.dvr .lbl{flex:none;width:46px}`
    let mut lbl = El::col().w(46.0).none().child(El::text(if out { "Output" } else { "Input" }, Font::new(13.0, 400), FG(), lh(13.0, 1.35)).ellipsis());
    if !out {
        // the owner Oct 8: the "Mute settings" link "should be way smaller, the input text itself should move up so they both
        // fit and center vertically on the icon so no new space is needed": "Input" + a 9.5 px link under it (57 px wide: it ends before the slider at 58), the pair
        // centred in the row like the mic icon (17.55 + 12 = 29.55 px in the row's 30 px line), the row as tall as Output
        // `title:'Mute key, sound, icon on screen'`
        let mut l = link::link(cx, K_MML, "Mute settings", 9.5).title("Mute key, sound, icon on screen");
        if let crate::ui::el::Content::Text(t) = &mut l.content {
            t.lh = 12.0;
        }
        // its own width (wider than the 46 px column: a narrower box centred the text 6 px out to the left)
        let lw = cx.g.text_width("Mute settings", Font::new(9.5, 400)).ceil();
        lbl = lbl.child(El::block().w(lw).h(12.0).none().child(l.none().w(lw)));
    }
    // `.dvs{position:relative;flex:1;min-width:0;height:30px}`: the canvas (live) and the range over it (8 px transparent track)
    // (Order 047: the level is read when the live pass paints it - `St::live`, written by `tick`)
    let src = st.live.clone();
    // `.vis.mut{opacity:.35}` (transition .3 s) - the Input's pill while the mic is muted
    let vis_op = cx.tr(kv, 9, if is_muted { 0.35 } else { 1.0 }, 300.0, EASE);
    let vis = El::paint(move |g, r| {
        let lv = {
            let s = src.borrow();
            if out {
                s.vo
            } else {
                s.vi
            }
        };
        meter::level_pill(g, r, vol, lv)
    })
    .abs(0.0, 0.0, 0.0, 0.0)
    .opacity(vis_op)
    .live()
    .no_hit();
    let look = slider::Look { track_h: 8.0, fill: Rgba(0.0, 0.0, 0.0, 0.0), track: Rgba(0.0, 0.0, 0.0, 0.0), thumb: WHITE };
    let rng = slider::slider(cx, kv, vol, 0.0, 30.0, look).w_pct(100.0).abs(0.0, 0.0, f32::NAN, f32::NAN);
    let dvs = El::block().flex1().h(30.0).child(vis).child(rng);
    // `.dvr .sv{flex:none;font-size:12.5px;margin-left:-2px}`
    let pct = pct(cx, st, kp, vol, FG2()).margin(0.0, 0.0, 0.0, -2.0);
    let pu = dropdown::dropdown(cx, kk, &name, Some(196.0));
    group::row(out, vec![icon, lbl, dvs, pct, pu]).min_h(44.0)
}

/// The % after a slider (`.sv.pct`): click it to type (Enter / clicking away applies, Esc cancels, clamped 0-100).
/// `.pct{display:inline-flex;align-items:center;justify-content:center;gap:3px;width:52px;height:22px;border-radius:6px;
///   font-variant-numeric:tabular-nums;transition:background-color .12s ease,color .12s ease}` `:hover{background:var(--hov);color:var(--fg)}`
/// `.pcti{width:30px;height:20px;border-radius:4px;background:var(--ctl);box-shadow:0 0 0 2px var(--acc-s),inset 0 0 0 1px var(--acc)}`
fn pct(cx: &mut Cx, st: &St, k: Key, v: f32, col: Rgba) -> El {
    let hv = cx.hover_t(k, 120.0, EASE);
    let font = F125;
    let lhv = lh(12.5, 1.35);
    // `title:'Click to type'` (on the span: it stays while the input is in it)
    let b = El::row().center().justify(taffy::style::JustifyContent::CENTER).gap(3.0).w(52.0).h(22.0).none().radius(6.0).on_click(k).cursor(Cursor::Text).title("Click to type");
    if let Some(e) = st.edit.as_ref().filter(|e| e.key == k) {
        let tw = cx.g.text_width(&e.text, font);
        let blink = ((cx.now - e.start) / st.caret_ms) as i64 % 2 == 0;
        cx.wake_every(st.caret_ms, e.start);
        let sel = st.sel;
        let (text, sel_all) = (e.text.clone(), e.sel_all);
        let isig = (text.clone(), sel_all, blink, tw.to_bits(), [sel.0, sel.1, sel.2, sel.3].map(f32::to_bits));
        let input = El::paint(move |g, (x, y, w, h)| {
            let tx = x + w / 2.0 - tw / 2.0;
            let ly = y + (h - lhv) / 2.0;
            if sel_all && !text.is_empty() {
                // Chromium's selection box = the text's line (measured in the drawing: 2 px under the input's top, 16 tall)
                g.fill_rect(tx, y + 2.0, tw, 16.0, sel);
                g.text(&text, font, tx, ly, lhv, WHITE, Align::Left, 0.0);
            } else {
                g.text(&text, font, tx, ly, lhv, FG(), Align::Left, 0.0);
                if blink {
                    g.fill_rect((tx + tw).round(), y + 3.0, 1.0, 14.0, FG());
                }
            }
        })
        .sig(isig)
        .size(30.0, 20.0)
        .none()
        .radius(4.0)
        .bg(CTL())
        .shadow(&[sh(0.0, 0.0, 0.0, 2.0, ACC_S())])
        .inset(&[sh(0.0, 0.0, 0.0, 1.0, ACC())]);
        return b.child(input).child(El::text("%", font, FG2(), lhv).none());
    }
    let c = cmix(col, FG(), hv);
    b.bg(HOV().mul_a(hv)).child(El::text(format!("{}", (v * 100.0).round() as i32), font, c, lhv).none()).child(El::text("%", font, c, lhv).none())
}

/// A mixer row (`.row.mxr`: gap 0, min-height 39, padding 4 12 9): tile + name (122 px), the slider in the app's colour
/// with its live level under it, the %, the mute button.
fn app_row(cx: &mut Cx, st: &mut St, i: usize) -> El {
    let now = cx.now;
    let a = &st.apps[i];
    let (kv, kp, km) = (k_vol(i), k_pct(i), k_mute(i));
    let rk = key(&format!("aud.row.{}", a.group));
    // muted: .25 s on the tile / slider / level / colours
    let mu = cx.tr(rk, 1, if a.muted { 1.0 } else { 0.0 }, 250.0, EASE);
    // `.mxr.quiet{opacity:.74}` `:hover{opacity:.85}` (silent apps), `.mxr.mu{opacity:1}`; transition .25 s
    let quiet = if st.frozen { matches!(a.mode, "none" | "blip") } else { now - a.last_sound > 1500.0 };
    let q = cx.tr(rk, 2, if quiet && !a.muted { 1.0 } else { 0.0 }, 250.0, EASE);
    let hov = cx.hovered(rk);
    let row_op = 1.0 - (1.0 - if hov { 0.85 } else { 0.74 }) * q;
    // tile + name: `.mxr .lbl.ap{flex:none;width:122px;gap:10px}` `.mxr .ait{transform:translateY(.5px)}` (v22, measured)
    let face = a.face.clone();
    let fsig = match &face {
        meter::Face::Glyph { glyph, a, b } => (glyph.as_ptr() as usize, [a.0, a.1, a.2, a.3, b.0, b.1, b.2, b.3].map(f32::to_bits)),
        meter::Face::Icon(px) => (std::sync::Arc::as_ptr(px) as usize, [0; 8]),
    };
    let mut tile = El::paint(move |g, (x, y, _, _)| meter::tile(g, &face, (x, y))).sig(fsig).size(24.0, 24.0).none().translate(0.0, 0.5);
    if mu > 0.001 {
        tile = tile.color_filter(crate::gfx::CssColor::Grayscale(mu)).opacity(1.0 - 0.58 * mu);
    }
    let name = El::text(a.name.clone(), Font::new(13.0, 400), cmix(FG(), FG3(), mu), lh(13.0, 1.35)).ellipsis().flex1_auto();
    // the owner Oct 8: "the icons inside of apps ... still look too high up ... move both the text and it down to center properly
    // for its own pill": the row is 39 tall with padding 4 / 9 (the level bar under the slider), so its content - and the
    // label in it - is centred 2.5 px above the row's middle (4 + 26 / 2 = 17 vs 39 / 2 = 19.5): the label moves down 2.5
    let lbl = El::row().center().gap(10.0).w(122.0).none().translate(0.0, 2.5).child(tile).child(name);
    // `.mxc{position:relative;flex:1;min-width:0;height:22px;margin-left:4px}` opacity .42 muted; 4 px track in --ac
    let look = slider::Look { track_h: 4.0, fill: cmix(a.c, FG2(), mu), track: TRK(), thumb: cmix(WHITE, THUMB_MU, mu) };
    let rng = slider::slider(cx, kv, a.vol, 0.0, 22.0, look).w_pct(100.0);
    // `.lvl{position:absolute;left:0;right:0;top:25px;height:3px}` (gone while muted)
    // (Order 047: level + peak read when the live pass paints it - `St::live`, written by `tick`. Order 055: the peak dot's
    // fade too - it is stepped with the levels, so a peak showing / hiding builds nothing)
    let (src, group) = (st.live.clone(), a.group.clone());
    let now_lv = move || src.borrow().apps.get(&group).map(|(l, p)| (l.l, l.pk, *p)).unwrap_or((0.0, 0.0, 0.0));
    let lvl = meter::level_bar_live(now_lv, a.c, a.c2).w_pct(100.0).abs(0.0, 25.0, f32::NAN, f32::NAN).opacity(1.0 - mu).no_hit();
    let mxc = El::block().flex1().h(22.0).margin(0.0, 0.0, 0.0, 4.0).opacity(1.0 - 0.58 * mu).child(rng).child(lvl);
    // `.mxr .sv{margin:0 12px}`
    let pct = pct(cx, st, kp, st.apps[i].vol, cmix(FG2(), FG3(), mu)).margin(0.0, 12.0, 0.0, 12.0);
    // `.amb{width:26px;height:26px;border-radius:50%}` `:hover{background:var(--ctl-h)}` `:active{transform:scale(.9)}`
    // `.m{background:rgba(255,69,58,.14)}`; `#sw button{color:inherit}` keeps the glyph --fg; `.mxr .amb{margin:0 18px 0 0}`
    let ah = cx.hover_t(km, 150.0, EASE);
    let ap = cx.active_t(km, 120.0, EASE);
    let amb = El::grid()
        .size(26.0, 26.0)
        .none()
        .margin(0.0, 18.0, 0.0, 0.0)
        .radius(RADIUS_PILL)
        .bg(cmix(CTL_H().mul_a(ah), Rgba::rgba(255, 69, 58, 0.14), mu))
        .place_center()
        .scale(1.0 - 0.1 * ap)
        .on_click(km)
        .cursor(Cursor::Hand)
        // `const t=(a.muted?'Unmute ':'Mute ')+a.name;mb.title=t;`
        .title(&format!("{}{}", if st.apps[i].muted { "Unmute " } else { "Mute " }, st.apps[i].name))
        .child(El::icon("spkM", 16.0, 1.5, FG()).class_op("w", 1.0 - mu).class_op("x", mu).no_hit());
    let mut r = El::row().center().min_h(39.0).pad(4.0, 12.0, 9.0, 12.0).key(rk).opacity(row_op);
    if i > 0 {
        r = r.child(El::block().abs(12.0, 0.0, 0.0, f32::NAN).h(1.0).bg(HAIR()).no_hit());
    }
    r.child(lbl).child(mxc).child(pct).child(amb)
}

/// "New apps volume" (`.row.mxr.nar`: min-height 46, padding 7): the + tile, its two lines, the slider, %, its switch.
fn new_row(cx: &mut Cx, st: &St) -> El {
    let r = svc::rules();
    // `.nai{width:24px;height:24px;border-radius:6px;box-shadow:inset 0 0 0 1px var(--hair);background:var(--hov)}` svg 12 px --fg2 1.6
    let nai = El::block().size(24.0, 24.0).none().radius(6.0).bg(HOV()).inset(&[sh(0.0, 0.0, 0.0, 1.0, HAIR())]).place_center().child(El::icon("plus12", 12.0, 1.6, FG2()));
    // `.nar .lbl.ap{width:auto;margin-right:16px}`
    let lbl = El::row().center().gap(10.0).none().margin(0.0, 16.0, 0.0, 0.0).child(nai).child(
        El::col()
            .none()
            .child(El::text("New apps volume", Font::new(13.0, 400), FG(), lh(13.0, 1.35)))
            .child(El::text("Apps you open from now on", Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).margin(1.0, 0.0, 0.0, 0.0)),
    );
    // `.nar.offv .mxc,.nar.offv .sv{opacity:0;pointer-events:none}` (.22 s)
    let fade = cx.tr(K_NEW_ON, 9, if r.new_on { 1.0 } else { 0.0 }, 220.0, EASE);
    let look = slider::Look { track_h: 4.0, fill: ACC(), track: TRK(), thumb: WHITE };
    let mut mxc = El::block().flex1().h(22.0).margin(0.0, 0.0, 0.0, 4.0).opacity(fade).child(slider::slider(cx, K_NEW_VOL, r.new_vol, 0.0, 22.0, look).w_pct(100.0));
    let mut p = pct(cx, st, K_NEW_PCT, r.new_vol, FG2()).margin(0.0, 12.0, 0.0, 12.0).opacity(fade);
    if !r.new_on {
        mxc = mxc.no_hit();
        p = p.no_hit();
    }
    El::row().center().min_h(46.0).pad(7.0, 12.0, 7.0, 12.0).child(lbl).child(mxc).child(p).child(toggle::toggle(cx, K_NEW_ON, r.new_on, false))
}

/// The device list (`.menu.devm`): every device with its own switch, the current one ticked, a switched-off one greyed;
/// the footer line. `.ditem{display:flex;align-items:center;gap:8px;height:32px;padding:0 6px;border-radius:6px;font-size:13px;
/// transition:background-color .12s ease}` `:hover{background:var(--sel)}` `.dis:hover{background:var(--hov)}`
/// `.ck{width:12px;font-size:12px;color:var(--acc);font-weight:600}` `.dg{16 x 16} svg{stroke:var(--fg2);stroke-width:1.4}`
/// `.dn{flex:1;min-width:0;ellipsis;padding-right:10px;transition:color .15s ease}` `.dis .dn{color:var(--fg3)}` `.dis .dg{opacity:.45}`
/// `.mfoot{margin:4px 2px 0;padding:7px 6px 3px;border-top:1px solid var(--hair);font-size:11px;line-height:1.35;color:var(--fg3)}`
fn dev_menu(cx: &mut Cx, st: &St, m: &DevMenu) -> El {
    let list = if m.flow == Flow::Output { &st.outs } else { &st.ins };
    let on_count = list.iter().filter(|d| d.on).count();
    let mut rows = Vec::new();
    for (i, d) in list.iter().enumerate() {
        let k = k_dev(i);
        let hv = cx.hover_t(k, 120.0, EASE);
        let last = d.on && on_count == 1;
        let ck = El::text(if d.current { "\u{2713}" } else { "" }, Font::new(12.0, 600), ACC(), lh(12.0, 1.35)).w(12.0).none().align(Align::Center);
        let dg = El::grid().size(16.0, 16.0).none().place_center().opacity(if d.on { 1.0 } else { 0.45 }).child(El::icon(d.device.kind.glyph(), 16.0, 1.4, FG2()));
        let nc = cx.tr(k, 4, if d.on { 1.0 } else { 0.0 }, 150.0, EASE);
        let dn = El::text(d.device.name.clone(), Font::new(13.0, 400), cmix(FG3(), FG(), nc), lh(13.0, 1.35)).ellipsis().flex1().pad(0.0, 10.0, 0.0, 0.0);
        // a switched-off device can't be picked: a click nudges its switch (the drawing's nudge: 1.14 at 35 %, 340 ms)
        let nud = m.nudge.get(&i).map(|t| cx.now - t).filter(|a| *a < 340.0).map(|a| {
            cx.st.busy = true;
            let p = a / 340.0;
            let s = if p < 0.35 { crate::anim::EASE_OUT.ease(p / 0.35) } else { 1.0 - crate::anim::EASE_OUT.ease((p - 0.35) / 0.65) };
            1.0 + 0.14 * s as f32
        });
        // `title:last?'One device always stays on':(d.on?'Switch off: Windows never uses it':'Switch on')`
        let sw_t = if last { "One device always stays on" } else if d.on { "Switch off: Windows never uses it" } else { "Switch on" };
        // (the disabled last switch keeps its key: `.tg:disabled` still takes the pointer, so its name shows)
        let sw = toggle::toggle(cx, k_devsw(i), d.on, last).scale(nud.unwrap_or(1.0)).key(k_devsw(i)).title(sw_t);
        rows.push(El::row().center().gap(8.0).h(32.0).pad(0.0, 6.0, 0.0, 6.0).radius(6.0).bg(if d.on { SEL() } else { HOV() }.mul_a(hv)).on_click(k).child(ck).child(dg).child(dn).child(sw));
    }
    rows.push(
        El::text("Switched off = Windows never uses it.", Font::new(11.0, 400), FG3(), lh(11.0, 1.35))
            .wrapping()
            .margin(4.0, 2.0, 0.0, 2.0)
            .pad(7.0, 6.0, 3.0, 6.0)
            .border_top(1.0, HAIR()),
    );
    let h = 5.0 + 32.0 * list.len() as f32 + 4.0 + 25.84375 + 4.0;
    // `.menu.devm{padding:5px 5px 4px}`
    // as wide as its widest row (`.dn` grows to its name: max-content), at least 300
    dropdown::menu_box(cx, K_DEVM, m.x, m.y, m.w, h, 300.0, rows).pad(5.0, 5.0, 4.0, 5.0).w(m.w)
}

// =================================================================================================== the page
impl Page for Audio {
    fn id(&self) -> &'static str {
        "aud"
    }
    fn name(&self) -> &'static str {
        "Audio"
    }
    fn icon(&self) -> &'static str {
        "spk"
    }
    fn open(&mut self, env: &Env, now: f64) {
        if self.st.is_some() {
            return;
        }
        let fake = env.fake();
        let svc = Svc::start(fake, env.real_read);
        svc.set_levels(!env.frozen);
        svc::ensure_watcher(env.test, env.real_read);
        micicon::set_fake(fake || env.test);
        let mic = mute::service(fake, env.frozen, env.real_read);
        // Order 047: a real mic's read and its watch (Core Audio, 10-50 ms) run on the key worker: the page opens on the
        // last known state, the read follows (`mic_read`)
        let muted = if fake {
            let m = mute::mic_muted(&mic);
            mute::watch(&mic);
            m
        } else {
            mute::watch_off(&mic, true);
            mute::last_muted()
        };
        let mut st = St {
            svc,
            frozen: env.frozen,
            outs: Vec::new(),
            ins: Vec::new(),
            out: None,
            inp: None,
            apps: Vec::new(),
            vo: Level::default(),
            vi: Level::default(),
            live: Default::default(),
            next_poll: now,
            seen_gen: None,
            resync: false,
            answered: false,
            born: now,
            live_only: false,
            snap_cache: None,
            seed: None,
            drag: None,
            hold_until: 0.0,
            edit: None,
            menu: None,
            just_closed: None,
            mic,
            mic_read: Default::default(),
            muted,
            out_muted: false,
            mute: None,
            review: None,
            toast: None,
            expect_err: false,
            rects: HashMap::new(),
            last: now,
            fake_sound: if fake && !env.frozen { Some(FakeSound::default()) } else { None },
            recs: Vec::new(),
            drag_before: None,
            reset_req: None,
            sel: if env.frozen { SEL_TEXT } else { sys_highlight() },
            caret_ms: if env.frozen { 530.0 } else { caret_blink() },
            quiet: Vec::new(),
        };
        // the fake answers at once: its first full read shows right away. Order 047: the real worker is never waited for
        // (it spun up to 100 ms here) - the page shows what it showed last time (this run) until the worker's first read
        if matches!(st.svc, Svc::Fake(..)) {
            for _ in 0..50 {
                if st.svc.snapshot().ready {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        } else {
            st.seed = LAST_SNAP.with(|l| l.borrow().clone());
        }
        if !fake {
            mute::read_state(&st.mic, st.mic_read.clone());
        }
        st.poll(now);
        self.st = Some(st);
        // test pictures of the open states (the frame's test hook can't click a page yet): BU_AUD_CLICKS="aud.mml;..."
        if let Some(list) = crate::testmode::env("BU_AUD_CLICKS") {
            self.replay(&list, now);
        }
    }
    /// The mic mute keys (they work with the menu closed).
    fn start(&self, s: &mut crate::services::Services) {
        mute::register(s);
    }
    /// At app start: the saved switches + Mute settings, and the always-on watcher ("Keep my devices" / "New apps volume"
    /// work with the menu closed - normal runs only).
    fn background(&self, env: &Env) -> Option<Box<dyn crate::pages::Background>> {
        svc::load_rules();
        let m = mute::service(env.fake(), env.frozen, env.real_read);
        mute::load(&m);
        // Order 045: a mic muted before the app started shows the tray badge at once
        crate::tray::set_muted(mute::mic_muted(&m));
        // only the keys of the chosen way, none while Mic mute is off (also with nothing saved yet)
        mute::sync_keys();
        svc::ensure_watcher(env.test, env.real_read);
        None
    }
    /// The first full read of the devices is in (no Output / Input row shows empty, then snaps).
    fn ready(&self) -> bool {
        self.st.as_ref().is_some_and(|s| s.out.is_some() || s.answered || s.svc.snapshot().ready)
    }
    /// Another page sent the menu here: "mute" opens Mute settings (Voice to text's "Mic mute" link, Settings' shortcuts).
    fn jump(&mut self, target: &str) {
        if let Some(st) = self.st.as_mut() {
            if (matches!(target, "mute" | "mm" | "Mic mute") || target.starts_with("mic.")) && st.mute.is_none() {
                st.mute = Some(st.mute_ui(st.last));
            }
        }
    }
    fn close(&mut self) {
        if let Some(st) = self.st.take() {
            if matches!(st.svc, Svc::Fake(..)) {
                mute::unwatch(&st.mic);
            } else {
                // (on the key worker, after its start: stopping waits for the watch's thread)
                mute::watch_off(&st.mic, false);
            }
            // the preview / Move end with the page
            if st.mute.is_some() {
                mute::sync_icon(st.muted, false, false, false);
            }
            // Order 036: the worker ends with the page (a change already asked for is still made); its entries are written.
            // Order 047: the real worker may be inside a Core Audio call (its first full read: 100-400 ms) - it is waited
            // for off the menu's thread (the change log takes notes from any thread); the fake (tests) ends here
            if matches!(st.svc, Svc::Fake(..)) {
                for (item, label, old, new) in st.svc.finish() {
                    svc::log(&item, &label, &old, &new);
                }
            } else {
                let s = st.svc;
                crate::offui::spawn("audio-close", move || {
                    for (item, label, old, new) in s.finish() {
                        crate::undo::note("aud", &item, &label, &old, &new);
                    }
                });
            }
        }
    }
    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        self.build_page(cx)
    }
    fn tick(&mut self, now: f64) -> bool {
        let Some(st) = self.st() else { return false };
        // Order 036: the worker's change-log entries (nothing is made when there are none)
        for (item, label, old, new) in st.svc.take_notes() {
            svc::log(&item, &label, &old, &new);
        }
        let mut changed = false;
        if let Some(m) = &mut st.mute {
            // (Order 047: reads only after a change, off the menu's thread for a real mic; true = the state shown changed)
            changed |= m.refresh();
            st.muted = m.muted();
        } else if mute::take_changed() {
            if matches!(st.svc, Svc::Fake(..)) {
                let was = st.muted;
                st.muted = mute::mic_muted(&st.mic);
                mute::sync_icon(st.muted, false, false, was != st.muted);
            } else {
                // Order 047: the real mic's state is a Core Audio read (10-50 ms) - off the menu's thread, on the key
                // worker (in order: an older read never lands last); the next tick shows it
                mute::read_state(&st.mic, st.mic_read.clone());
            }
        }
        let read = st.mic_read.lock().ok().and_then(|mut s| s.take());
        if let Some(v) = read {
            let was = st.muted;
            st.muted = v;
            mute::sync_icon(st.muted, false, false, was != st.muted);
            changed |= was != st.muted;
        }
        // Order 055: the meters and the worker's data are DATA - they are looked at when `next_poll` comes (a step every
        // 33 ms while there is sound, every 100 ms in silence), never per frame: the frames of a 360 Hz screen (a hover, a
        // scroll) pass here at once. `wake_at` asks for the step, so nothing else is needed to keep them going.
        if now < st.next_poll - 1.0 {
            st.live_only = false;
            return changed;
        }
        // (the worker writes its levels every 16 ms; a write count that did not move, no meter still decaying and no value
        // held back = nothing to copy or compare)
        let g = st.svc.gen();
        if st.seen_gen != Some(g) || st.resync || st.fake_sound.is_some() || st.meters_moving(now) {
            changed |= st.poll(now);
            st.seen_gen = Some(g);
        }
        // frames only while a meter moves (or something changed); in silence the menu sleeps and `wake_at` looks for sound
        // again at the silent rate
        let went_quiet = st.quiet_flip(now);
        // the meters paint from `live` in the live pass - the peak dots too (their fade is stepped there), so a peak showing
        // or hiding is not a build; a row going quiet (a .25 s fade of the row) is
        st.sync_live(now);
        let moving = !st.frozen && st.meters_moving(now);
        st.next_poll = now + if moving || (!st.answered && now - st.born < 5000.0) { STEP_MS } else { SILENT_MS };
        st.live_only = !(changed || went_quiet);
        changed || went_quiet || moving
    }
    fn live_only(&self) -> bool {
        self.st.as_ref().is_some_and(|s| s.live_only)
    }
    /// Order 055: the next step of the meters / the next look for sound - the page asks for it, the menu sleeps until then
    /// (no frames, no CPU). Frozen test pictures never.
    fn wake_at(&self, now: f64) -> Option<f64> {
        let st = self.st.as_ref()?;
        (!st.frozen).then_some(st.next_poll.max(now + 4.0))
    }
    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        let now = cx.now;
        let Some(st) = self.st.as_mut() else { return };
        // the Mute settings popup is modal (its dim layer covers the window): every event is its own
        if let Some(m) = &mut st.mute {
            if !m.event(ev, cx) {
                st.mute = None;
                mute::sync_icon(st.muted, false, false, false);
            }
            st.muted = st.mute.as_ref().map(|m| m.muted()).unwrap_or(st.muted);
            return;
        }
        match ev {
            Ev::Press(k, x, _, r) => {
                st.rects.insert(*k, *r);
                if is_slider(st, *k) {
                    let v = st.value_of(*k);
                    let thumb = r.0 + 8.0 + (r.2 - 16.0) * v;
                    let off = if (x - thumb).abs() <= 8.0 { x - thumb } else { 0.0 };
                    st.drag = Some((*k, off));
                    st.drag_before = st.before(*k);
                    st.set_value(*k, slider::value_at(*r, x - off), now);
                }
                if st.edit.as_ref().map(|e| e.key != *k).unwrap_or(false) {
                    st.end_edit(true, now);
                }
            }
            Ev::Drag(k, x, _, r) => {
                if let Some((dk, off)) = st.drag {
                    if dk == *k {
                        st.set_value(*k, slider::value_at(*r, x - off), now);
                    }
                }
            }
            Ev::Release(k) => {
                if st.drag.map(|d| d.0 == *k).unwrap_or(false) {
                    st.drag = None;
                    st.hold_until = now + 600.0;
                    let b = st.drag_before.take();
                    st.track(b);
                }
            }
            Ev::Char(k, c) => {
                if let Some(e) = st.edit.as_mut().filter(|e| e.key == *k) {
                    if c.is_ascii_digit() {
                        if e.sel_all {
                            e.text.clear();
                            e.sel_all = false;
                        }
                        if e.text.len() < 3 {
                            e.text.push(*c);
                        }
                    }
                    e.start = now;
                }
            }
            Ev::Key(k, vk) => {
                const VK_BACK: u16 = 0x08;
                const VK_RETURN: u16 = 0x0D;
                const VK_ESCAPE: u16 = 0x1B;
                if let Some(e) = st.edit.as_mut().filter(|e| e.key == *k) {
                    match *vk {
                        VK_RETURN => st.end_edit(true, now),
                        VK_ESCAPE => st.end_edit(false, now),
                        VK_BACK => {
                            if e.sel_all {
                                e.text.clear();
                                e.sel_all = false;
                            } else {
                                e.text.pop();
                            }
                            e.start = now;
                        }
                        0x25 | 0x27 => {
                            e.sel_all = false;
                            e.start = now;
                        }
                        _ => {}
                    }
                } else if is_slider(st, *k) && matches!(*vk, 0x25..=0x28) {
                    // arrows on a focused slider: 1 % a step
                    let step = if matches!(*vk, 0x26 | 0x27) { 0.01 } else { -0.01 };
                    let v = st.value_of(*k) + step;
                    let b = st.before(*k);
                    st.set_value(*k, v, now);
                    st.track(b);
                    // (Order 045: the frame does not step it a second time)
                    cx.used = true;
                }
            }
            Ev::Blur(k) => {
                if st.edit.as_ref().map(|e| e.key == *k).unwrap_or(false) {
                    st.end_edit(true, now);
                }
            }
            Ev::Click(k) => click(st, *k, now),
            Ev::Context(..) | Ev::Drop(..) | Ev::Wheel(..) | Ev::DragOver(..) => {}
        }
        // Order 036: this event's changes into the change log; a reset link opens the frame's shared review
        for (item, label, old, new) in std::mem::take(&mut st.recs) {
            cx.record(&item, &label, &old, &new);
        }
        if let Some((kind, r)) = st.reset_req.take() {
            cx.open_reset(kind, r);
        }
    }
    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let st = self.st.as_mut()?;
        let toast_el = st.toast.as_ref().filter(|t| cx.now - t.1 < toast::SHOW_MS + 300.0).map(|t| toast::toast(cx, K_TOAST, &t.0.clone(), t.1, false));
        if toast_el.is_none() {
            st.toast = None;
        }
        let main = if let Some(m) = st.mute.as_mut() {
            Some(m.popup(cx))
        } else if let Some(m) = st.menu.take() {
            let e = dev_menu(cx, st, &m);
            st.menu = Some(m);
            Some(e)
        } else if let Some(r) = st.review.as_ref() {
            let (title, text) = if r.win {
                ("Audio \u{00b7} Windows defaults?", "Each one goes to Windows\u{2019} own value.")
            } else {
                ("Audio \u{00b7} back to how it was?", "Each one goes back to the value it had before this app changed it.")
            };
            let lines = r.lines.clone();
            // the drawing's placeMenu(btn, 316): under the link (left-aligned), above it when there is no room, right-aligned
            // when too wide; Math.round of both. The list's real size = its laid-out content + the box's 5 px padding.
            let mk = |cx: &mut Cx, x: f32, y: f32| {
                let n = lines.iter().filter(|l| l.ticked).count();
                let go_label = if n > 0 { format!("Reset {n}") } else { "Reset".to_string() };
                let buttons = vec![
                    pieces::button::cbtn_sized(cx, K_RV_CANCEL, "Cancel", pieces::button::Kind::Ghost, pieces::button::MCFB, false, 0.0),
                    pieces::button::cbtn_sized(cx, K_RV_GO, &go_label, pieces::button::Kind::Red, pieces::button::MCFB, n == 0, 0.0),
                ];
                // = reset::review_popup, placed exactly (its own clamp pulls the list up into the window; the drawing's list,
                // opened from the page's last line, reaches below the window's edge and is clipped there)
                let body = reset::review(cx, K_REVIEW, title, text, &lines, buttons);
                dropdown::menu_box(cx, sub(K_REVIEW, "box"), x, y, 316.0, 0.0, 10000.0, vec![body])
            };
            // its real size: the list box laid out once
            let probe = mk(cx, 0.0, 0.0);
            let pl = crate::ui::lay::Laid::new(cx.g, El::block().w(WIN_W).h(WIN_H).child(probe), WIN_W, Some(WIN_H));
            let (mw, mh) = pl.nodes.get(1).map(|n| (n.rect.2, n.rect.3)).unwrap_or((326.0, 233.0));
            let mut x = r.x;
            let mut y = r.y + r.h + 4.0;
            if x + mw > WIN_W - 8.0 {
                x = (r.x + r.w - mw).max(8.0);
            }
            if y + mh > WIN_H - 8.0 {
                y = (r.y - mh - 4.0).max(8.0);
            }
            let (x, y) = (x.round(), y.round());
            Some(mk(cx, x, y))
        } else {
            None
        };
        match (main, toast_el) {
            (Some(m), Some(t)) => Some(El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(WIN_W, WIN_H).no_hit().child(m).child(t)),
            (Some(m), None) => Some(m),
            (None, Some(t)) => Some(t),
            (None, None) => None,
        }
    }
    fn popup_dismiss(&mut self) {
        let Some(st) = self.st.as_mut() else { return };
        if let Some(m) = &mut st.mute {
            if !m.dismiss() {
                st.mute = None;
                // a key field that was listening stops with its window
                crate::services::try_with(|s| s.stop_listening());
                mute::sync_icon(st.muted, false, false, false);
            }
            return;
        }
        if let Some(m) = st.menu.take() {
            st.just_closed = Some(m.flow);
        }
        st.review = None;
    }
    fn describe(&self) -> String {
        let Some(st) = &self.st else { return String::new() };
        let id = |d: &Option<(String, f32)>| d.as_ref().map(|d| d.0.clone()).unwrap_or_default();
        let vol = |d: &Option<(String, f32)>| d.as_ref().map(|d| (d.1 * 100.0).round()).unwrap_or(0.0);
        let r = svc::rules();
        let mut s = format!(
            "out={} in={} outvol={} invol={} keep={} newapps={} newvol={} micmuted={} menu={} mute={} review={} outmuted={}",
            id(&st.out),
            id(&st.inp),
            vol(&st.out),
            vol(&st.inp),
            r.keep,
            r.new_on,
            (r.new_vol * 100.0).round(),
            st.muted,
            st.menu.as_ref().map(|m| if m.flow == Flow::Output { "out" } else { "in" }).unwrap_or("none"),
            st.mute.is_some(),
            st.review.as_ref().map(|r| if r.win { "win" } else { "pc" }).unwrap_or("none"),
            st.out_muted
        );
        for a in &st.apps {
            s += &format!("\napp {} vol={} muted={}", a.name, (a.vol * 100.0).round(), a.muted);
        }
        if let Some(m) = &st.mute {
            s += &format!("\n{}", m.describe());
        }
        for l in st.svc.log() {
            s += &format!("\ncall {l}");
        }
        s
    }
    /// Order 036: cheap (nothing is opened here); a closed page builds what a reset needs on its first call.
    fn resettable(&mut self) -> Option<&mut dyn Resettable> {
        Some(self)
    }
}

fn is_slider(st: &St, k: Key) -> bool {
    k == K_OUT_VOL || k == K_IN_VOL || k == K_NEW_VOL || (0..st.apps.len()).any(|i| k_vol(i) == k)
}

fn click(st: &mut St, k: Key, now: f64) {
    let just = st.just_closed.take();
    // the device list
    if let Some(m) = &st.menu {
        let flow = m.flow;
        let list: Vec<DeviceRow> = if flow == Flow::Output { st.outs.clone() } else { st.ins.clone() };
        if let Some(i) = (0..list.len()).find(|&i| k_devsw(i) == k) {
            let d = &list[i];
            let on_count = list.iter().filter(|x| x.on).count();
            if d.on && on_count == 1 {
                return; // one device always stays on (its switch is disabled)
            }
            st.expect_err = true;
            st.run(Cmd::DeviceOn(flow, d.device.id.clone(), !d.on), now);
            return;
        }
        if let Some(i) = (0..list.len()).find(|&i| k_dev(i) == k) {
            let d = &list[i];
            if !d.on {
                if let Some(m) = &mut st.menu {
                    m.nudge.insert(i, now);
                }
                return;
            }
            let id = d.device.id.clone();
            st.menu = None;
            if flow == Flow::Output {
                st.out = Some((id.clone(), st.out.as_ref().map(|o| o.1).unwrap_or(0.0)));
            } else {
                st.inp = Some((id.clone(), st.inp.as_ref().map(|o| o.1).unwrap_or(0.0)));
            }
            st.expect_err = true;
            st.run(Cmd::Default(flow, id), now);
            return;
        }
    }
    // the reset review
    if let Some(r) = &mut st.review {
        if let Some(i) = (0..r.lines.len()).find(|&i| crate::ui::el::idx(K_REVIEW, i) == k) {
            r.lines[i].ticked = !r.lines[i].ticked;
            return;
        }
        if k == K_RV_CANCEL {
            st.review = None;
            return;
        }
        if k == K_RV_GO {
            let r = st.review.take().unwrap_or(Review { win: false, x: 0.0, y: 0.0, w: 0.0, h: 0.0, lines: Vec::new() });
            reset_now(st, &r, now);
            return;
        }
    }
    match k {
        K_OUT_PCT | K_IN_PCT | K_NEW_PCT => start_edit(st, k, now),
        K_OUT_PICK | K_IN_PICK => {
            let flow = if k == K_OUT_PICK { Flow::Output } else { Flow::Input };
            if st.menu.as_ref().map(|m| m.flow == flow).unwrap_or(false) || just == Some(flow) {
                st.menu = None;
            } else {
                st.open_menu(flow);
            }
        }
        K_SPK => {
            // Order 081: the default output device's own mute, on the worker like the other device commands; the icon
            // shows the flip at once and then follows the worker's answer
            if let Some((id, _)) = st.out.clone() {
                st.out_muted = !st.out_muted;
                let m = st.out_muted;
                st.run(Cmd::DeviceMute(id, m), now);
            }
        }
        K_MIC => {
            // Order 047: a real mic is toggled on the key's worker (Core Audio, 10-50 ms): the button shows the flip at once,
            // the icon follows the worker's answer
            let fake = matches!(st.svc, Svc::Fake(..));
            if let Some(m) = mute::toggle_now(&st.mic, st.muted, fake, true) {
                st.muted = m;
                if fake {
                    mute::sync_icon(st.muted, false, false, true);
                }
            }
        }
        K_MML => st.mute = Some(st.mute_ui(now)),
        K_KEEP => {
            let b = st.before(k);
            let mut r = svc::rules();
            r.keep = !r.keep;
            svc::set_rules(r);
            st.track(b);
        }
        K_NEW_ON => {
            let b = st.before(k);
            let mut r = svc::rules();
            r.new_on = !r.new_on;
            svc::set_rules(r);
            st.track(b);
        }
        _ if k == sub(K_RESET, "pc") || k == sub(K_RESET, "win") => {
            let win = k == sub(K_RESET, "win");
            // the link's box (window coordinates); the list is placed when it is built (its real height)
            let (x, y, w, h) = st.rects.get(&k).copied().unwrap_or((150.0, 600.0, 120.0, 16.0));
            if st.frozen {
                // test pictures: the drawing's sample log in the page's own review (nothing on the PC changes)
                let lines = st.review_lines(win);
                st.review = Some(Review { win, x, y, w, h, lines });
            } else {
                // Order 036: the frame's ONE review over the change log, applied through `Resettable`
                st.reset_req = Some((if win { Kind::WindowsDefaults } else { Kind::HowItWas }, (x, y, w, h)));
            }
        }
        _ => {
            if let Some(i) = (0..st.apps.len()).find(|&i| k_mute(i) == k) {
                let b = st.before(k);
                let out = st.out.as_ref().map(|o| o.0.clone()).unwrap_or_default();
                let a = &mut st.apps[i];
                a.muted = !a.muted;
                let (g, m) = (a.group.clone(), a.muted);
                st.run(Cmd::AppMute(out, g, m), now);
                st.track(b);
            } else if let Some(i) = (0..st.apps.len()).find(|&i| k_pct(i) == k) {
                let _ = i;
                start_edit(st, k, now);
            }
        }
    }
}

fn start_edit(st: &mut St, k: Key, now: f64) {
    if st.edit.as_ref().map(|e| e.key == k).unwrap_or(false) {
        return;
    }
    st.end_edit(true, now);
    let v = (st.value_of(k) * 100.0).round() as i32;
    st.edit = Some(Edit { key: k, text: v.to_string(), sel_all: true, start: now });
}

/// The sample review's Reset (test pictures only): says what it would do; nothing on the PC changes.
fn reset_now(st: &mut St, r: &Review, now: f64) {
    let n = r.lines.iter().filter(|l| l.ticked).count();
    let what = if r.win { "Windows defaults" } else { "Back to how it was" };
    let t = if n == 0 { "Nothing to reset".to_string() } else { format!("{what} \u{00b7} {n} {} reset", if n == 1 { "setting" } else { "settings" }) };
    st.toast = Some((t, now));
}

// =================================================================================================== Order 036: reset
/// Where a reset's device-side change goes: the open page's fake, a closed test copy's own fake, or Windows (a real
/// service made for the call on its own thread - the open page's worker sees the result on its next read).
#[derive(Clone)]
enum Target {
    Fake(bu_audio::SharedFake),
    Real,
}

impl Audio {
    fn target(&self) -> Target {
        match self.st.as_ref().map(|s| s.svc.fake()) {
            Some(Some(f)) => Target::Fake(f),
            Some(None) => Target::Real,
            // (a unit test never reaches Windows either)
            None if crate::testmode::on() || cfg!(test) => Target::Fake(self.closed_fake.get_or_init(|| bu_audio::SharedFake::new(svc::drawing_fake())).clone()),
            None => Target::Real,
        }
    }

    /// The open page's app rows for the reset (item, label, value), as `apps_now` gives them. None = closed.
    fn seen_apps(&self) -> Option<Vec<(String, String, Val)>> {
        let st = self.st.as_ref()?;
        Some(st.apps.iter().filter(|a| !a.group.starts_with("pid:")).map(|a| (svc::app_item(&a.group), format!("{} volume", a.name), svc::app_val(a.vol, a.muted))).collect())
    }

    /// Order 047: the open page's values of every device / app item (what `St::current` answers), taken here at once
    /// from its worker's snapshot (cheap) for a detached reset copy. None = closed (the last recorded values are used).
    fn seen_vals(&self) -> Option<HashMap<String, Val>> {
        let st = self.st.as_ref()?;
        let s = st.svc.snapshot();
        let mut items = vec!["out.default".to_string(), "in.default".to_string()];
        items.extend(s.outputs.iter().chain(s.inputs.iter()).map(|d| format!("dev:{}", d.device.id)));
        items.extend(st.apps.iter().map(|a| svc::app_item(&a.group)));
        Some(items.into_iter().filter_map(|i| st.current(&i).map(|v| (i, v))).collect())
    }
}

fn device_apply(t: &Target, item: &str, to: &Val) -> Result<(), String> {
    match t {
        Target::Fake(f) => svc::apply_item(&mut bu_audio::AudioService::new(f.clone()), item, to),
        #[cfg(windows)]
        Target::Real => svc::on_real(|s| svc::apply_item(s, item, to)),
        #[cfg(not(windows))]
        Target::Real => Err("Windows only".into()),
    }
}

/// The apps on the output device now: (item, label, value) - the open page's rows (`seen`), else read for the call.
fn apps_now(t: &Target, seen: Option<&Vec<(String, String, Val)>>) -> Vec<(String, String, Val)> {
    if let Some(v) = seen {
        return v.clone();
    }
    let rows = match t {
        Target::Fake(f) => svc::apps_now(&mut bu_audio::AudioService::new(f.clone())),
        #[cfg(windows)]
        Target::Real => svc::on_real(|s| Ok(svc::apps_now(s))).unwrap_or_default(),
        #[cfg(not(windows))]
        Target::Real => Vec::new(),
    };
    rows.into_iter()
        .filter(|a| !a.group.starts_with("pid:"))
        .map(|a| (svc::app_item(&a.group), format!("{} volume", a.look.name), svc::app_val(a.volume, a.muted)))
        .collect()
}

fn rules_current(r: svc::Rules, item: &str) -> Val {
    if item == svc::KEEP {
        svc::on_val(r.keep)
    } else {
        svc::newapps_val(r)
    }
}

/// "Windows defaults" (RS.aud): both switches off, every app at 100 %.
fn rs_defaults(r: svc::Rules, apps: Vec<(String, String, Val)>) -> Vec<DefaultItem> {
    let mut v = vec![
        DefaultItem { item: svc::KEEP.into(), label: svc::KEEP_LABEL.into(), now: svc::on_val(r.keep), default: svc::on_val(false) },
        DefaultItem {
            item: svc::NEWAPPS.into(),
            label: svc::NEWAPPS_LABEL.into(),
            now: svc::newapps_val(r),
            default: svc::newapps_val(svc::Rules { new_on: false, ..r }),
        },
    ];
    for (item, label, now) in apps {
        v.push(DefaultItem { item, label, now, default: svc::app_val(1.0, false) });
    }
    v
}

/// One of the two switches put to `to` (None = not one of them).
fn rules_apply(mut r: svc::Rules, item: &str, to: &Val) -> Option<Result<svc::Rules, String>> {
    match item {
        svc::KEEP => r.keep = to.raw == "on",
        svc::NEWAPPS => match to.raw.strip_prefix("on|") {
            Some(v) => {
                r.new_on = true;
                match v.parse::<f32>() {
                    Ok(x) => r.new_vol = x.clamp(0.0, 1.0),
                    Err(_) => return Some(Err("Unknown value".into())),
                }
            }
            None => r.new_on = false,
        },
        _ => return None,
    }
    Some(Ok(r))
}

/// Audio's items: the default output / input (all three roles), each device's switch, each app's volume + mute, and the
/// two switches "Keep my devices" / "New apps volume". Device volume is not an item (the everyday volume knob, like the
/// mic mute). "Windows defaults" (RS.aud): both switches off, every app at 100 % (Windows has no default device).
impl Resettable for Audio {
    fn page_id(&self) -> &str {
        "aud"
    }
    fn page_title(&self) -> &str {
        "Audio"
    }
    fn current(&self, item: &str) -> Option<Val> {
        match item {
            svc::KEEP | svc::NEWAPPS => Some(rules_current(svc::rules(), item)),
            // the devices / apps only while the page is open (its worker's snapshot; Windows' audio service can take a second)
            _ => self.st.as_ref().and_then(|st| st.current(item)),
        }
    }
    fn windows_defaults(&self) -> Vec<DefaultItem> {
        rs_defaults(svc::rules(), apps_now(&self.target(), self.seen_apps().as_ref()))
    }
    fn apply(&mut self, item: &str, to: &Val) -> Result<(), String> {
        if crate::testmode::real_read() {
            return Err("A read-only test copy changes nothing".into());
        }
        if let Some(r) = rules_apply(svc::rules(), item, to) {
            svc::set_rules(r?);
            return Ok(());
        }
        device_apply(&self.target(), item, to)?;
        if let Some(st) = self.st.as_mut() {
            // the open page shows Windows' answer, not its own values held after a change
            st.hold_until = 0.0;
        }
        Ok(())
    }
    /// Order 047: a closed page's apps (Windows' audio service, a second) and every put-back (`on_real`: a Core Audio
    /// service made for the call) on the review's worker thread. The two switches live on the menu's thread (the
    /// settings store): the copy hands each put-back line over (`PENDING_RULES`), the main loop saves it
    /// (`drain_pending_rules`, also with the menu closed).
    fn detach(&mut self) -> Option<crate::undo::Detached> {
        Some(Box::new(AudReset { rules: svc::rules(), vals: self.seen_vals(), apps: self.seen_apps(), target: self.target() }))
    }
    fn reset_done(&mut self) {
        drain_pending_rules();
        if let Some(st) = self.st.as_mut() {
            // the open page shows Windows' answer, not its own values held after a change
            st.hold_until = 0.0;
        }
    }
}

/// Order 047: the switch lines a detached reset put back on the review's worker thread, waiting for the menu's thread
/// (the switches and the settings store live there): (item, value).
static PENDING_RULES: std::sync::Mutex<Vec<(String, Val)>> = std::sync::Mutex::new(Vec::new());

/// Order 047: save the switch lines a detached reset put back (the main loop calls it after every wake-up, menu open or
/// closed; `reset_done` too). Each line onto the switches as they are NOW (a toggle made meanwhile stays).
pub fn drain_pending_rules() {
    let lines = match PENDING_RULES.lock() {
        Ok(mut q) if !q.is_empty() => std::mem::take(&mut *q),
        _ => return,
    };
    for (item, to) in lines {
        if let Some(Ok(r)) = rules_apply(svc::rules(), &item, &to) {
            svc::set_rules(r);
        }
    }
}

/// Order 047: the Audio page's reset as a copy for the review's worker thread.
struct AudReset {
    /// the two switches as they were, then as the copy put them (each line handed over through `PENDING_RULES`)
    rules: svc::Rules,
    /// the open page's device / app values (None = closed)
    vals: Option<HashMap<String, Val>>,
    apps: Option<Vec<(String, String, Val)>>,
    target: Target,
}

impl Resettable for AudReset {
    fn page_id(&self) -> &str {
        "aud"
    }
    fn page_title(&self) -> &str {
        "Audio"
    }
    fn current(&self, item: &str) -> Option<Val> {
        match item {
            svc::KEEP | svc::NEWAPPS => Some(rules_current(self.rules, item)),
            _ => self.vals.as_ref()?.get(item).cloned(),
        }
    }
    fn windows_defaults(&self) -> Vec<DefaultItem> {
        rs_defaults(self.rules, apps_now(&self.target, self.apps.as_ref()))
    }
    fn apply(&mut self, item: &str, to: &Val) -> Result<(), String> {
        if crate::testmode::real_read() {
            return Err("A read-only test copy changes nothing".into());
        }
        if let Some(r) = rules_apply(self.rules, item, to) {
            self.rules = r?;
            if let Ok(mut q) = PENDING_RULES.lock() {
                q.push((item.to_string(), to.clone()));
            }
            crate::services::Waker.wake();
            return Ok(());
        }
        device_apply(&self.target, item, to)
    }
}

#[cfg(test)]
mod tests;
