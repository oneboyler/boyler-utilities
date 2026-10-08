//! The Display tab (menu-v22 page `dsp`, DESIGN.md §3.2; Order 019): ONE monitor selector (+ Identify) and under it, for
//! that monitor: resolution Width × Height × Hz + Apply with the 10 s keep / revert bar, Scaling, Main display; Picture
//! (the monitor's own brightness / contrast over DDC/CI, vibrance); Presets (one click applies); Switch automatically
//! (app → preset (+ vibrance), at the game's process start). No OBS setting (moved to the OBS tab later, the owner Oct 8).
//! The values live in the runtime (`rt.rs`, it outlives the page: the countdown and the rules run with the menu closed);
//! the page reads them on open and drops everything on close. Tests: the FAKE runtime with the drawing's sample.

mod apps;
pub mod identify;
mod reset;
pub mod rt;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use taffy::prelude::*;
use taffy::style::JustifyContent;

use bu_display::fields::{self as fl, COMMON_HEIGHTS, COMMON_WIDTHS};
use bu_display::presets::{Preset, PresetId};
use bu_display::{DdcState, GpuScaling, MonitorId, MonitorInfo, PictureState, RefreshRate, Vcp, VideoMode};

use crate::anim::{Bezier, EASE, EASE_OUT};
use crate::gfx::{sh, Align, Font, Gfx, Rgba};
use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, lh, sub, Cursor, El, Key};
use crate::ui::pieces::mitems::{self, It, Lead, Place, Row};
use crate::ui::pieces::segx::{self, Label};
use crate::ui::pieces::{self, bits, button, group, nbox, rowbits, seg, slider, tip, toast, toggle};
use crate::ui::{cmix, ACC, ACC_S, CTL, CTL_H, DASH, FG, FG2, FG3, GRP, HAIR, HL, HOV, ICO_ON, MENU, RED, SEL, TRK, WHITE};

use rt::Rt;

const K_MON: Key = key("dsp.mon");
const K_ID: Key = key("dsp.ident");
const K_W: Key = key("dsp.w");
const K_H: Key = key("dsp.h");
const K_HZ: Key = key("dsp.hz");
const K_HZF: Key = key("dsp.hzf");
const K_APPLY: Key = key("dsp.apply");
const K_SCALE: Key = key("dsp.scale");
const K_MAIN: Key = key("dsp.main");
const K_BRI: Key = key("dsp.bri");
const K_CON: Key = key("dsp.con");
const K_VIB: Key = key("dsp.vib");
const K_PST: Key = key("dsp.pst");
const K_PNEW: Key = key("dsp.pnew");
const K_RULE: Key = key("dsp.rule");
const K_ADD: Key = key("dsp.addapp");
const K_MENU: Key = key("dsp.menu");
const K_CFM: Key = key("dsp.cfm");
const K_KEEP: Key = key("dsp.keep");
const K_REVERT: Key = key("dsp.revert");
const K_TOAST: Key = key("dsp.toast");
const K_RESET: Key = key("dsp.reset");

/// The Scaling control's order (the drawing's `SCALES`).
const SCALES: [GpuScaling; 3] = [GpuScaling::Stretch, GpuScaling::BlackBars, GpuScaling::KeepAspect];
const POP: Bezier = Bezier::new(0.3, 1.35, 0.5, 1.0);
const BAR_IN: Bezier = Bezier::new(0.3, 1.2, 0.5, 1.0);
const BAR_OUT: Bezier = Bezier::new(0.4, 0.0, 1.0, 1.0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fld {
    W,
    H,
    Hz,
}

impl Fld {
    fn key(self) -> Key {
        match self {
            Fld::W => K_W,
            Fld::H => K_H,
            Fld::Hz => K_HZ,
        }
    }
}

/// What the three fields + Scaling hold until Apply (the drawing's `D.f`).
#[derive(Clone, Copy, PartialEq, Debug)]
struct Fields {
    w: u32,
    h: u32,
    hz: RefreshRate,
    sc: GpuScaling,
}

struct Edit {
    fld: Fld,
    buf: String,
    /// just focused: the whole value is selected (typing replaces it)
    all: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum MenuKind {
    Rates,
    App(usize),
    Preset(usize),
    Vib(usize),
}

struct Menu {
    kind: MenuKind,
    /// the box the list opens under (window coordinates; `mitems::Place::Under`); NaN = not placed yet
    r: (f32, f32, f32, f32),
}

/// The lists' min-width: the drawing's `placeMenu(btn, 150)`.
const MENU_MIN_W: f32 = 150.0;

/// One line of a popup list, owned (a `mitems::Row` only borrows its words).
enum MItem {
    /// `app` = the app tile before the label (the list of apps)
    Item { label: String, app: Option<&'static apps::App>, checked: bool },
    Sep,
}

/// A short CSS keyframe animation started at `t0` (ms).
#[derive(Clone, Copy)]
struct Kf {
    t0: f64,
    dir: f32,
}

/// Brightness / contrast / vibrance changes go to this worker (DDC/CI is slow): the newest value per slider wins.
#[derive(Default)]
struct PicJobs {
    /// (monitor, which slider, %, its change-log label), and whether the worker runs
    slot: Mutex<(Vec<(MonitorId, u8, u8, String)>, bool)>,
}

impl PicJobs {
    /// Runs the waiting jobs until none is left. One run = one change-log line per slider (the value before the run's
    /// first change → its last), noted when the queue is empty (a drag is one change, not fifty file writes).
    fn run(&self, rt: &Rt) {
        let mut log: Vec<(MonitorId, u8, String, crate::undo::Val, crate::undo::Val)> = Vec::new();
        loop {
            let (id, which, pct, label) = {
                let mut g = self.slot.lock().unwrap_or_else(|e| e.into_inner());
                if g.0.is_empty() {
                    g.1 = false;
                    break;
                }
                g.0.remove(0)
            };
            let Ok(mut s) = rt.svc.lock() else { continue };
            let changed = match which {
                0 | 1 => {
                    let vcp = if which == 0 { Vcp::Brightness } else { Vcp::Contrast };
                    s.set_ddc_percent_change(&id, vcp, pct).map(|c| c.map(|(a, b)| (reset::ddc_val(a), reset::ddc_val(b))))
                }
                _ => s.set_vibrance_percent_change(&id, pct).map(|c| c.map(|(a, b)| (reset::vib_val(&a), reset::vib_val(&b)))),
            };
            drop(s);
            if let Ok(Some((old, new))) = changed {
                match log.iter_mut().find(|l| l.0 == id && l.1 == which) {
                    Some(l) => l.4 = new,
                    None => log.push((id, which, label, old, new)),
                }
            }
        }
        for (id, which, label, old, new) in log {
            let it = match which {
                0 => reset::Item::Ddc(id, Vcp::Brightness),
                1 => reset::Item::Ddc(id, Vcp::Contrast),
                _ => reset::Item::Vib(id),
            };
            reset::rec(&it.id(), &label, &old, &new);
        }
    }
}

/// Order 047: the monitors as the page's worker read them (`read_now`): every monitor with its modes, and the change
/// waiting for Keep (its deadline, the first monitor it changed).
#[derive(Clone)]
struct Read {
    mons: Vec<MonitorInfo>,
    modes: Vec<Vec<VideoMode>>,
    pending: Option<(Instant, Option<MonitorId>)>,
}

/// What a read does to the page's fields when it lands.
#[derive(Clone, Copy, PartialEq, Debug)]
enum How {
    /// the tab's open read (a change waiting for Keep brings its bar back)
    Open,
    /// the fields go back to the monitor's mode (the old `reload`)
    Fields,
    /// the fields stay what the user set (the old `reload_keep_fields`)
    KeepFields,
    /// the countdown went back by itself (a note): the bar goes when no change waits any more
    AfterNote,
}

/// One change-log line made on the worker (written by the page when the answer lands, or by the worker itself when the
/// page has closed meanwhile).
type Log = (String, String, crate::undo::Val, crate::undo::Val);

/// Order 047: what the page's worker answers. Mode changes (SetDisplayConfig, 0.5 - 3 s), the monitors' read
/// (QueryDisplayConfig, EDID, DXGI mode lists) and every other wait on the shared service (which a DDC/CI write may hold
/// ~50 ms per value) run there - the menu's thread never waits for the service.
enum Ans {
    Read { how: How, read: Read },
    /// Apply: the error, else the countdown is armed
    Applied { r: Result<(), String>, read: Read },
    /// Keep: the toast's mode text, or the error; the kept modes' change-log lines
    Kept { r: Result<Option<String>, String>, logs: Vec<Log> },
    /// Revert: the restored mode's text, or the error
    Reverted { r: Result<Option<String>, String>, read: Read },
    /// Main display: moved (true) / was already (false), or the error
    Main { r: Result<bool, String>, logs: Vec<Log>, read: Read },
}

type Job = Box<dyn FnOnce(&Arc<Rt>) -> Ans + Send>;

/// A mode click (Apply, Keep, Revert, Main display) made while another mode change was on the worker.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Next {
    Apply,
    Keep,
    Revert,
    Main,
}

/// The last read the tab showed (`env.keep`): shown at once when the tab opens again, while the fresh one runs.
const KEEP_READ: &str = "dsp.read";

/// Monitors, modes and the waiting change, read now (worker).
fn read_now(rt: &Rt) -> Read {
    let s = rt.svc.lock().unwrap_or_else(|e| e.into_inner());
    let mons = s.monitors().unwrap_or_default();
    let modes = mons.iter().map(|m| s.modes(&m.id).unwrap_or_default()).collect();
    let pending = s.pending().map(|p| (p.deadline, p.originals.first().map(|o| o.0.clone())));
    Read { mons, modes, pending }
}

fn rec_logs(logs: &[Log]) {
    for (item, label, old, new) in logs {
        reset::rec(item, label, old, new);
    }
}

/// The page's worker: one thread for the open page, its jobs one after another in click order. It ends when the page
/// drops its sender (close); a change-log line the closed page can't write any more the worker writes itself.
fn start_worker(rt: Arc<Rt>, slow: u64) -> (std::sync::mpsc::Sender<Job>, std::sync::mpsc::Receiver<Ans>) {
    let (jtx, jrx) = std::sync::mpsc::channel::<Job>();
    let (atx, arx) = std::sync::mpsc::channel::<Ans>();
    let _ = std::thread::Builder::new().name("bu-display".into()).spawn(move || {
        for job in jrx {
            if slow > 0 {
                // test copies only: a stand-in for a slow mode change
                std::thread::sleep(std::time::Duration::from_millis(slow));
            }
            let a = job(&rt);
            if let Err(e) = atx.send(a) {
                if let Ans::Kept { logs, .. } | Ans::Main { logs, .. } = &e.0 {
                    rec_logs(logs);
                }
            }
            crate::services::Waker.wake();
        }
    });
    (jtx, arx)
}

#[derive(Default)]
pub struct Display {
    rt: Option<Arc<Rt>>,
    /// Order 047: the worker's job line and its answers; jobs sent and not answered yet
    work: Option<std::sync::mpsc::Sender<Job>>,
    ans: Option<std::sync::mpsc::Receiver<Ans>>,
    jobs: usize,
    /// a mode change (Apply / Keep / Revert / Main display) is on the worker: another one waits for it
    mode_busy: bool,
    /// the last mode click made while one was on the worker: sent when its answer lands
    next: Option<Next>,
    /// a change waits for Keep (as last read)
    pend: bool,
    /// the app's store of last results (`env.keep`)
    memo: crate::keep::Keep,
    /// test copies only: every worker job first sleeps this long (ms) - a stand-in for a slow mode change
    slow: u64,
    /// a test copy's page state, put once the open read is in
    state_due: Option<String>,
    frozen: bool,
    mons: Vec<MonitorInfo>,
    modes: Vec<Vec<VideoMode>>,
    /// the selected monitor (index into `mons`)
    sel: usize,
    f: Option<Fields>,
    edit: Option<Edit>,
    /// per field: the last step / snap animation
    step: [Option<Kf>; 3],
    snap: [Option<f64>; 3],
    pic: Vec<Option<PictureState>>,
    pic_rx: Option<std::sync::mpsc::Receiver<(usize, PictureState)>>,
    /// the monitor whose values `pic_rx` brings
    pic_at: usize,
    pic_jobs: Option<Arc<PicJobs>>,
    menu: Option<Menu>,
    /// the list the frame closed on this press (a press beside it) - read by the press that follows
    dismissed: Option<MenuKind>,
    toast: Option<(String, f64)>,
    /// the keep bar: shown since (ms) / hidden since
    cfm_on: Option<f64>,
    cfm_off: Option<f64>,
    /// the monitor selector's pill blinks after Apply ("like a mode change")
    blink: Option<f64>,
    /// nudges (a small "look here" pulse) per element
    nudges: Vec<(Key, f64)>,
    /// a preset chip just saved / being deleted, a rule just added / being removed
    pst_in: Option<(PresetId, f64)>,
    pst_out: Option<(PresetId, f64)>,
    rule_in: Option<(u64, f64)>,
    rule_out: Option<(u64, f64)>,
    /// the runtime's change count when last read
    epoch: u64,
    /// the keep bar's deadline as last read (used while the service is busy)
    bar_deadline: Option<Instant>,
    /// the runtime of a CLOSED page for the reset (Settings › Reset, the uninstaller): made on first need (reset.rs)
    lazy_rt: std::cell::OnceCell<Arc<Rt>>,
    /// where the reset links were pressed (their box: the frame's review opens under it)
    link_box: Option<(Key, (f32, f32, f32, f32))>,
    now: f64,
}

/// "1920 × 1080 · 165 Hz" for a preset (the drawing's `fmt(p)` with no monitor: the rate rounded).
fn preset_text(p: &Preset) -> String {
    format!("{} × {} · {} Hz", p.width, p.height, p.refresh.hz().round() as i64)
}

fn sc_index(sc: GpuScaling) -> usize {
    SCALES.iter().position(|s| *s == sc).unwrap_or(2)
}

fn ease_t(now: f64, t0: f64, dur: f64, e: Bezier) -> f32 {
    e.ease(((now - t0) / dur).clamp(0.0, 1.0)) as f32
}

impl Display {
    fn rt(&self) -> Option<&Arc<Rt>> {
        self.rt.as_ref()
    }

    fn sel_id(&self) -> Option<MonitorId> {
        self.mons.get(self.sel).map(|m| m.id.clone())
    }

    /// The selected monitor's rates (every rate it reports at any size), fastest first.
    fn rates(&self) -> Vec<RefreshRate> {
        let mut v = self.modes.get(self.sel).map(|m| fl::all_rates(m)).unwrap_or_default();
        v.sort();
        v.dedup();
        v.reverse();
        v
    }

    fn hz_label(&self, r: RefreshRate) -> String {
        fl::rate_label(r, &self.rates())
    }

    /// Order 047: hand a job to the page's worker (its answer lands in `tick`). False = no worker (a closed page).
    fn run(&mut self, job: impl FnOnce(&Arc<Rt>) -> Ans + Send + 'static) -> bool {
        let sent = self.work.as_ref().is_some_and(|w| w.send(Box::new(job)).is_ok());
        if sent {
            self.jobs += 1;
        }
        sent
    }

    /// Reads monitors + modes again (QueryDisplayConfig, EDID, every monitor's DXGI mode list: 30 - 150 ms - Order 047: on
    /// the worker); `how` = what the fields do when it lands.
    fn read_again(&mut self, how: How) {
        self.run(move |rt| Ans::Read { how, read: read_now(rt) });
    }

    /// A read landed: the monitors and their modes, the selection kept by monitor; the fields back to the selected
    /// monitor's mode, or (`keep_fields`) what the user set.
    fn take_read(&mut self, r: Read, keep_fields: bool) {
        let f = self.f;
        let keep_id = self.sel_id();
        self.mons = r.mons;
        self.modes = r.modes;
        self.sel = keep_id.and_then(|id| self.mons.iter().position(|m| m.id == id)).unwrap_or(0);
        if self.pic.len() != self.mons.len() {
            self.pic = vec![None; self.mons.len()];
        }
        self.fields_from_monitor();
        if keep_fields && f.is_some() {
            self.f = f;
        }
        self.pend = r.pending.is_some();
        self.bar_deadline = r.pending.as_ref().map(|p| p.0);
        // the next open shows these at once (never the waiting change: its bar comes from the fresh read)
        self.memo.put(KEEP_READ, Read { mons: self.mons.clone(), modes: self.modes.clone(), pending: None });
    }

    /// A worker's answer: the page shows it and says how it went (as it did when the call ran here).
    fn landed(&mut self, a: Ans) {
        match a {
            Ans::Read { how, read } => match how {
                How::Open => {
                    let first = read.pending.as_ref().map(|p| p.1.clone());
                    self.take_read(read, false);
                    // a change still waiting for Keep (made before the menu was closed): its bar is up again
                    if let Some(first) = first {
                        if self.cfm_on.is_none() {
                            self.cfm_on = Some(self.now - 400.0);
                            self.cfm_off = None;
                        }
                        if let Some(i) = first.and_then(|id| self.mons.iter().position(|m| m.id == id)) {
                            self.sel = i;
                            self.fields_from_monitor();
                        }
                    }
                    self.read_picture(self.sel);
                    if let Some(s) = self.state_due.take() {
                        self.test_state(&s);
                    }
                }
                How::Fields => self.take_read(read, false),
                How::KeepFields => self.take_read(read, true),
                How::AfterNote => {
                    let gone = read.pending.is_none();
                    if self.cfm_on.is_some() && gone {
                        self.hide_bar();
                        self.take_read(read, false);
                    } else {
                        self.take_read(read, true);
                    }
                }
            },
            Ans::Applied { r, read } => {
                self.mode_busy = false;
                match r {
                    Ok(()) => {
                        self.cfm_on = Some(self.now);
                        self.cfm_off = None;
                        self.blink = Some(self.now);
                        self.take_read(read, true);
                    }
                    Err(e) => {
                        self.take_read(read, true);
                        self.show_toast(format!("Couldn’t apply: {e}"));
                    }
                }
            }
            Ans::Kept { r, logs } => {
                self.mode_busy = false;
                match r {
                    Ok(t) => {
                        // a KEPT change goes into the change log (one the countdown / Revert took back never does)
                        rec_logs(&logs);
                        self.pend = false;
                        self.bar_deadline = None;
                        if let Some(t) = t {
                            self.show_toast(format!("Kept {t}"));
                        }
                        self.hide_bar();
                    }
                    Err(e) => {
                        self.show_toast(format!("Couldn’t keep: {e}"));
                        self.bar_back();
                    }
                }
            }
            Ans::Reverted { r, read } => {
                self.mode_busy = false;
                match r {
                    Ok(t) => {
                        self.hide_bar();
                        self.take_read(read, false);
                        self.show_toast(format!("Back to {}", t.unwrap_or_default()));
                    }
                    Err(e) => {
                        self.show_toast(format!("Couldn’t go back: {e}"));
                        self.bar_back();
                    }
                }
            }
            Ans::Main { r, logs, read } => {
                self.mode_busy = false;
                match r {
                    Ok(true) => {
                        // into the change log: the monitor that was main
                        rec_logs(&logs);
                        self.take_read(read, true);
                        if let Some(m) = self.mons.get(self.sel) {
                            self.show_toast(format!("Main display: {} · {}", m.number, m.name));
                        }
                    }
                    // one monitor is always the main one: it just stays on, with a small nudge
                    Ok(false) => self.nudge(K_MAIN),
                    Err(e) => self.show_toast(format!("Couldn’t change it: {e}")),
                }
            }
        }
        // Order 047: the mode click made meanwhile goes now (the last one wins)
        if !self.mode_busy {
            match self.next.take() {
                Some(Next::Apply) => self.apply(),
                Some(Next::Keep) => self.keep(),
                Some(Next::Revert) => self.revert(),
                Some(Next::Main) => self.set_main(),
                None => {}
            }
        }
    }

    /// Keep / Revert didn't go through: the bar (hidden by the click) is back while the change still waits.
    fn bar_back(&mut self) {
        if self.pend && self.cfm_on.is_none() {
            self.cfm_on = Some(self.now - 400.0);
            self.cfm_off = None;
        }
    }

    /// Test copies (and proof pictures): wait (5 s at most) until every worker job has answered and landed.
    #[cfg(test)]
    pub fn settle(&mut self, now: f64) {
        let t0 = std::time::Instant::now();
        while self.jobs > 0 && t0.elapsed().as_secs() < 5 {
            self.tick(now);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(self.jobs == 0, "a Display job never answered");
    }

    fn fields_from_monitor(&mut self) {
        self.f = self.mons.get(self.sel).map(|m| Fields { w: m.current.width, h: m.current.height, hz: m.current.refresh, sc: m.current.scaling });
    }

    /// The Picture group's values: read off the UI thread (DDC/CI answers take tens of ms per value).
    fn read_picture(&mut self, i: usize) {
        let (Some(rt), Some(m)) = (self.rt.clone(), self.mons.get(i)) else { return };
        if self.pic.get(i).map(|p| p.is_some()).unwrap_or(false) {
            return;
        }
        // (already on its way)
        if self.pic_rx.is_some() && self.pic_at == i {
            return;
        }
        let id = m.id.clone();
        if rt.fake {
            if let Ok(mut s) = rt.svc.lock() {
                if let Ok(p) = s.picture(&id) {
                    self.pic[i] = Some(p);
                }
            }
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.pic_rx = Some(rx);
        self.pic_at = i;
        std::thread::spawn(move || {
            let p = rt.svc.lock().ok().and_then(|mut s| s.picture(&id).ok());
            if let Some(p) = p {
                let _ = tx.send((i, p));
            }
            // Order 047: the menu draws the values when they are in (no frames while they are read)
            crate::services::Waker.wake();
        });
    }

    fn select_monitor(&mut self, i: usize) {
        if i == self.sel || i >= self.mons.len() {
            return;
        }
        self.commit_edit();
        self.sel = i;
        self.fields_from_monitor();
        for (j, s) in self.step.iter_mut().enumerate() {
            *s = Some(Kf { t0: self.now + 35.0 * j as f64, dir: 0.0 });
        }
        self.read_picture(i);
    }

    fn show_toast(&mut self, t: impl Into<String>) {
        self.toast = Some((t.into(), self.now));
    }

    fn nudge(&mut self, k: Key) {
        self.nudges.retain(|(n, _)| *n != k);
        self.nudges.push((k, self.now));
    }

    fn nudge_scale(&self, k: Key) -> f32 {
        // nudge: scale 1 → 1.14 (35 %) → 1, 340 ms, EASE_OUT over the whole keyframe list
        self.nudges.iter().find(|(n, _)| *n == k).map_or(1.0, |(_, t0)| {
            let p = ((self.now - t0) / 340.0).clamp(0.0, 1.0);
            let e = EASE_OUT.ease(p) as f32;
            if e < 0.35 {
                1.0 + 0.14 * e / 0.35
            } else {
                1.14 - 0.14 * (e - 0.35) / 0.65
            }
        })
    }

    // ---------------------------------------------------------------- the fields
    fn field_text(&self, fld: Fld) -> String {
        if let Some(e) = &self.edit {
            if e.fld == fld {
                return e.buf.clone();
            }
        }
        let Some(f) = self.f else { return String::new() };
        match fld {
            Fld::W => f.w.to_string(),
            Fld::H => f.h.to_string(),
            Fld::Hz => self.hz_label(f.hz),
        }
    }

    fn begin_edit(&mut self, fld: Fld) {
        if self.edit.as_ref().map(|e| e.fld) == Some(fld) {
            return;
        }
        self.commit_edit();
        self.edit = Some(Edit { fld, buf: self.field_text(fld), all: true });
    }

    /// Enter / a click away: the typed value becomes a real one (W/H clamped, Hz snapped to a rate the monitor reports).
    fn commit_edit(&mut self) {
        let Some(e) = self.edit.take() else { return };
        let rates = self.rates();
        let Some(f) = self.f.as_mut() else { return };
        match e.fld {
            Fld::W => {
                if let Some(v) = fl::parse_size_field(&e.buf, true) {
                    f.w = v;
                }
            }
            Fld::H => {
                if let Some(v) = fl::parse_size_field(&e.buf, false) {
                    f.h = v;
                }
            }
            Fld::Hz => {
                if let Some(v) = fl::parse_hz_field(&e.buf) {
                    if let Some(s) = fl::snap_hz(v, &rates) {
                        f.hz = s;
                        if fl::rate_label(s, &rates) != e.buf.trim() {
                            self.snap[2] = Some(self.now);
                        }
                    }
                }
                if matches!(self.menu.as_ref().map(|m| m.kind), Some(MenuKind::Rates)) {
                    self.menu = None;
                }
            }
        }
    }

    fn step_field(&mut self, fld: Fld, up: bool) {
        let rates = self.rates();
        let Some(f) = self.f.as_mut() else { return };
        let before = *f;
        match fld {
            Fld::W => f.w = fl::step_through(f.w, &COMMON_WIDTHS, up),
            Fld::H => f.h = fl::step_through(f.h, &COMMON_HEIGHTS, up),
            Fld::Hz => f.hz = fl::step_hz(f.hz, &rates, up),
        }
        if *f != before {
            let i = fld as usize;
            self.step[i] = Some(Kf { t0: self.now, dir: if up { 1.0 } else { -1.0 } });
            if let Some(e) = self.edit.as_mut() {
                if e.fld == fld {
                    e.all = false;
                }
            }
            let t = self.field_text(fld);
            if let Some(e) = self.edit.as_mut() {
                if e.fld == fld {
                    e.buf = t;
                }
            }
            if matches!(self.menu.as_ref().map(|m| m.kind), Some(MenuKind::Rates)) {
                self.menu = None;
            }
        }
    }

    // ---------------------------------------------------------------- apply / keep / revert
    /// Order 047: the mode change (SetDisplayConfig, 0.5 - 3 s) runs on the worker; the fields keep the user's values
    /// meanwhile, the keep bar comes up (and its 10 s countdown starts) when Windows has answered.
    fn apply(&mut self) {
        self.commit_edit();
        if self.mode_busy {
            // Order 047: a second Apply (a preset) during a mode change is remembered and sent when its answer lands
            self.next = Some(Next::Apply);
            return;
        }
        let (Some(id), Some(f)) = (self.sel_id(), self.f) else { return };
        if self.run(move |rt| {
            let res = rt.svc.lock().map(|mut s| s.apply_fields(&id, f.w, f.h, f.hz.hz(), f.sc, Instant::now()));
            let r = match res {
                Ok(Ok(_)) => {
                    rt.arm_keep();
                    Ok(())
                }
                Ok(Err(e)) => Err(e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            Ans::Applied { r, read: read_now(rt) }
        }) {
            self.mode_busy = true;
        }
    }

    fn keep(&mut self) {
        if self.mode_busy {
            self.next = Some(Next::Keep);
            return;
        }
        if self.rt.is_none() {
            return;
        }
        // the monitors' names for the change log
        let names: Vec<(MonitorId, String)> = self.mons.iter().map(|m| (m.id.clone(), reset::mon_name(m))).collect();
        // Order 047: Windows' saved setting is written on the worker; the bar goes at once (the click), the toast comes
        // with the answer
        if self.run(move |rt| {
            rt.disarm_keep();
            // (what each monitor showed before the first Apply: the change log's old value)
            let mut s = rt.svc.lock().unwrap_or_else(|e| e.into_inner());
            let before = s.pending().map(|p| p.originals.clone()).unwrap_or_default();
            let list = match s.keep() {
                Ok(l) => l,
                Err(e) => return Ans::Kept { r: Err(e.to_string()), logs: Vec::new() },
            };
            let mut logs = Vec::new();
            let mut kept = Vec::new();
            for (id, m) in &list {
                let modes = s.modes(id).unwrap_or_default();
                if let Some((_, old)) = before.iter().find(|(b, _)| b == id) {
                    let name = names.iter().find(|(x, _)| x == id).map(|(_, n)| n.clone()).unwrap_or_else(|| "Monitor".into());
                    let it = reset::Item::Mode(id.clone());
                    logs.push((it.id(), it.label(&name), reset::mode_val(&s, id, old), reset::mode_val(&s, id, m)));
                }
                kept.push((id.clone(), *m, modes));
            }
            let t = list.first().map(|(id, m)| rt::mode_text(&s, id, m));
            drop(s);
            // (the store after the service: it is never held while waiting for the service)
            if let Ok(mut st) = rt.store.lock() {
                for (id, m, modes) in &kept {
                    st.presets.note_kept(id, m, modes);
                }
            }
            rt.save();
            Ans::Kept { r: Ok(t), logs }
        }) {
            self.mode_busy = true;
            self.hide_bar();
        }
    }

    fn revert(&mut self) {
        if self.mode_busy {
            self.next = Some(Next::Revert);
            return;
        }
        if self.rt.is_none() {
            return;
        }
        // Order 047: the mode change back on the worker; the bar goes at once (the click)
        if self.run(move |rt| {
            rt.disarm_keep();
            let r = {
                let mut s = rt.svc.lock().unwrap_or_else(|e| e.into_inner());
                s.revert().map(|rv| rv.restored.first().map(|(id, m)| rt::mode_text(&s, id, m))).map_err(|e| e.to_string())
            };
            Ans::Reverted { r, read: read_now(rt) }
        }) {
            self.mode_busy = true;
            self.hide_bar();
        }
    }

    /// Main display: Windows moves it on the worker (Order 047).
    fn set_main(&mut self) {
        if self.mode_busy {
            // (sent when the mode change on its way has answered)
            self.next = Some(Next::Main);
            return;
        }
        let Some(id) = self.sel_id() else { return };
        let was = self.mons.iter().find(|m| m.is_main).map(|m| m.id.clone());
        if self.run(move |rt| {
            let r = rt.svc.lock().unwrap_or_else(|e| e.into_inner()).set_main(&id).map_err(|e| e.to_string());
            let read = read_now(rt);
            let mut logs = Vec::new();
            if let (Ok(true), Some(was)) = (&r, was) {
                logs.push((reset::MAIN.to_string(), "Main display".to_string(), reset::main_val(&read.mons, &was), reset::main_val(&read.mons, &id)));
            }
            Ans::Main { r, logs, read }
        }) {
            self.mode_busy = true;
        }
    }

    fn hide_bar(&mut self) {
        if self.cfm_on.take().is_some() {
            self.cfm_off = Some(self.now);
        }
    }

    // ---------------------------------------------------------------- presets
    fn presets(&self) -> Vec<Preset> {
        self.rt().and_then(|r| r.store.lock().ok().map(|s| s.presets.items().to_vec())).unwrap_or_default()
    }

    fn current_preset(&self) -> Option<PresetId> {
        let rt = self.rt()?;
        let m = self.mons.get(self.sel)?;
        let st = rt.store.lock().ok()?;
        st.presets.matching(&m.current, self.modes.get(self.sel)?)
    }

    fn apply_preset(&mut self, id: PresetId) {
        // (a mode change on its way: the preset's Apply is sent when its answer lands - Order 047)
        let Some(p) = self.presets().into_iter().find(|p| p.id == id) else { return };
        let modes = self.modes.get(self.sel).cloned().unwrap_or_default();
        let hz = fl::snap_hz(p.refresh.hz(), &fl::rates_for(&modes, p.width, p.height)).unwrap_or(p.refresh);
        self.f = Some(Fields { w: p.width, h: p.height, hz, sc: p.scaling });
        self.apply();
    }

    fn save_preset(&mut self) {
        self.commit_edit();
        let (Some(rt), Some(f)) = (self.rt.clone(), self.f) else { return };
        let modes = self.modes.get(self.sel).cloned().unwrap_or_default();
        let r = rt.store.lock().map(|mut s| s.presets.add(f.w, f.h, f.hz.hz(), f.sc, &modes));
        match r {
            Ok(Ok(id)) => {
                self.pst_in = Some((id, self.now));
                rt.save();
            }
            Ok(Err(bu_display::DisplayError::DuplicatePreset(i))) => {
                if let Some(p) = self.presets().get(i) {
                    self.nudge(idx(K_PST, p.id.0 as usize));
                }
                self.show_toast("Already a preset");
            }
            _ => {}
        }
    }

    fn delete_preset(&mut self, id: PresetId) {
        let Some(rt) = self.rt.clone() else { return };
        let before = self.rules_val();
        if let Ok(mut s) = rt.store.lock() {
            let _ = s.presets.remove(id);
            s.rules.forget_preset(id);
        }
        rt.save();
        rt.sync_watcher_soon();
        self.rules_logged(before);
    }

    // ---------------------------------------------------------------- rules
    fn rules(&self) -> Vec<bu_display::autoswitch::AppRule> {
        self.rt().and_then(|r| r.store.lock().ok().map(|s| s.rules.rules().to_vec())).unwrap_or_default()
    }

    fn change_rule(&mut self, i: usize, f: impl FnOnce(&mut bu_display::autoswitch::AppRule)) {
        let Some(rt) = self.rt.clone() else { return };
        let before = self.rules_val();
        let id = self.rules().get(i).map(|r| r.id);
        if let (Some(id), Ok(mut s)) = (id, rt.store.lock()) {
            if let Some(r) = s.rules.rule_mut(id) {
                f(r);
            }
        }
        rt.save();
        rt.sync_watcher_soon();
        self.rules_logged(before);
    }

    fn add_rule(&mut self) {
        let Some(rt) = self.rt.clone() else { return };
        let before = self.rules_val();
        // a new row starts on the first Stretch preset (else the first one) and goes straight on to picking the app
        let ps = self.presets();
        let pre = ps.iter().find(|p| p.scaling == GpuScaling::Stretch).or(ps.first()).map(|p| p.id);
        let n = rt.store.lock().map(|mut s| {
            let id = s.rules.add_rule("", pre);
            (s.rules.rules().len() - 1, id.0)
        });
        if let Ok((i, id)) = n {
            self.rule_in = Some((id, self.now));
            // the app list opens under the new row's app button once it is laid out (anchor: the add row's place)
            self.menu = Some(Menu { kind: MenuKind::App(i), r: (f32::NAN, f32::NAN, 0.0, 0.0) });
        }
        rt.save();
        self.rules_logged(before);
    }

    fn remove_rule(&mut self, i: usize) {
        let Some(rt) = self.rt.clone() else { return };
        let before = self.rules_val();
        let id = self.rules().get(i).map(|r| r.id);
        if let (Some(id), Ok(mut s)) = (id, rt.store.lock()) {
            s.rules.remove_rule(id);
        }
        rt.save();
        rt.sync_watcher_soon();
        self.rules_logged(before);
    }

    /// The rules as the change log keeps them (reset.rs).
    fn rules_val(&self) -> Option<crate::undo::Val> {
        self.rt().and_then(|r| r.store.lock().ok().map(|s| reset::rules_val(&s)))
    }

    /// The rules changed (`before` = as they were): one line in the change log, "Switch automatically".
    fn rules_logged(&self, before: Option<crate::undo::Val>) {
        if let (Some(b), Some(a)) = (before, self.rules_val()) {
            if b.raw != a.raw {
                reset::rec(reset::RULES, reset::RULES_LABEL, &b, &a);
            }
        }
    }

    fn menu_items(&self, kind: MenuKind) -> Vec<MItem> {
        match kind {
            MenuKind::Rates => {
                let cur = self.f.map(|f| f.hz);
                self.rates().iter().map(|r| MItem::Item { label: format!("{} Hz", self.hz_label(*r)), app: None, checked: Some(*r) == cur }).collect()
            }
            MenuKind::App(i) => {
                let cur = self.rules().get(i).map(|r| r.exe.clone()).unwrap_or_default();
                let mut v: Vec<MItem> = apps::KNOWN
                    .iter()
                    .map(|a| MItem::Item { label: a.name.to_string(), app: Some(a), checked: bu_display::autoswitch::exe_matches(a.exe, &cur) && !cur.is_empty() })
                    .collect();
                v.push(MItem::Sep);
                v.push(MItem::Item { label: "Browse for an app…".into(), app: None, checked: false });
                v
            }
            MenuKind::Preset(i) => {
                let cur = self.rules().get(i).and_then(|r| r.preset);
                self.presets()
                    .iter()
                    .map(|p| MItem::Item { label: format!("{} · {}", preset_text(p), p.scaling.label()), app: None, checked: Some(p.id) == cur })
                    .collect()
            }
            MenuKind::Vib(i) => {
                let cur = self.rules().get(i).and_then(|r| r.vibrance);
                let mut v = vec![MItem::Item { label: "No change".into(), app: None, checked: cur.is_none() }];
                v.extend(bu_display::autoswitch::VIBRANCE_CHOICES.iter().map(|p| MItem::Item { label: format!("Vibrance {p} %"), app: None, checked: cur == Some(*p) }));
                v
            }
        }
    }

    /// The open list as `mitems::menu` rows' box, under `r`. A click on row j = `Ev::Click(idx(K_MENU, j))`, separators
    /// counted (`row_item` maps it back to the item).
    fn menu_box(&self, cx: &mut Cx, kind: MenuKind, r: (f32, f32, f32, f32)) -> El {
        let items = self.menu_items(kind);
        let rows: Vec<Row> = items
            .iter()
            .map(|m| match m {
                MItem::Sep => Row::Sep,
                // an app tile is 18 px wide: `Lead::Gt` holds its place (16 px) until the real tile replaces it below
                MItem::Item { label, app, checked } => {
                    let it = It::tick(label, *checked);
                    Row::Item(if app.is_some() { it.lead(Lead::Gt("", Rgba(0.0, 0.0, 0.0, 0.0))) } else { it })
                }
            })
            .collect();
        let mut menu = mitems::menu(cx, K_MENU, &rows, Place::Under(r.0, r.1, r.2, r.3), MENU_MIN_W);
        // `.mitem .at{margin-right:2px}`: the item's app tile (bits::at) in place of the placeholder (child 0 = the tick)
        for (m, row) in items.iter().zip(menu.children.iter_mut()) {
            if let (MItem::Item { app: Some(a), .. }, Some(slot)) = (m, row.children.get_mut(1)) {
                *slot = app_tile(a).margin(0.0, 2.0, 0.0, 0.0);
            }
        }
        menu
    }

    /// The item a clicked row j of a list is (separators are rows, not items): None for a separator.
    fn row_item(&self, kind: MenuKind, j: usize) -> Option<usize> {
        let items = self.menu_items(kind);
        match items.get(j)? {
            MItem::Sep => None,
            MItem::Item { .. } => Some(j - items[..j].iter().filter(|m| matches!(m, MItem::Sep)).count()),
        }
    }

    fn pick(&mut self, kind: MenuKind, i: usize, cx: &mut Cx) {
        self.menu = None;
        match kind {
            MenuKind::Rates => {
                let rates = self.rates();
                if let (Some(r), Some(f)) = (rates.get(i), self.f.as_mut()) {
                    f.hz = *r;
                }
                self.edit = None;
            }
            MenuKind::App(r) => {
                if let Some(a) = apps::KNOWN.get(i) {
                    self.change_rule(r, |rule| rule.exe = a.exe.to_string());
                } else if let Some(exe) = cx.pick_file("Choose an app", &[("Programs", "*.exe")]) {
                    // "Browse for an app…": Windows' file picker (the frame's, during this click); the path goes straight
                    // into the rule. A unit test is never inside a click, a test copy gets BU_PICK.
                    self.change_rule(r, |rule| rule.exe = exe);
                }
            }
            MenuKind::Preset(r) => {
                if let Some(p) = self.presets().get(i) {
                    let id = p.id;
                    self.change_rule(r, |rule| rule.preset = Some(id));
                }
            }
            MenuKind::Vib(r) => {
                let v = if i == 0 { None } else { bu_display::autoswitch::VIBRANCE_CHOICES.get(i - 1).copied() };
                self.change_rule(r, |rule| rule.vibrance = v);
            }
        }
    }

    // ---------------------------------------------------------------- picture
    fn set_picture(&mut self, which: u8, pct: u8) {
        let (Some(rt), Some(id)) = (self.rt.clone(), self.sel_id()) else { return };
        if let Some(Some(p)) = self.pic.get_mut(self.sel) {
            match which {
                0 => p.brightness = p.brightness.map(|v| bu_display::VcpValue { current: bu_display::picture::ddc_value_for_percent(v.max, pct), ..v }),
                1 => p.contrast = p.contrast.map(|v| bu_display::VcpValue { current: bu_display::picture::ddc_value_for_percent(v.max, pct), ..v }),
                _ => p.vibrance_percent = Some(pct),
            }
        }
        let name = self.mons.get(self.sel).map(reset::mon_name).unwrap_or_else(|| "Monitor".into());
        let label = match which {
            0 => reset::Item::Ddc(id.clone(), Vcp::Brightness),
            1 => reset::Item::Ddc(id.clone(), Vcp::Contrast),
            _ => reset::Item::Vib(id.clone()),
        }
        .label(&name);
        let jobs = self.pic_jobs.get_or_insert_with(|| Arc::new(PicJobs::default())).clone();
        let start = {
            let mut g = jobs.slot.lock().unwrap_or_else(|e| e.into_inner());
            g.0.retain(|(m, w, _, _)| !(m == &id && *w == which));
            g.0.push((id, which, pct, label));
            let idle = !g.1;
            g.1 = true;
            idle
        };
        if start {
            if rt.fake {
                // the fake answers at once: run here (a test sees the change and its change-log line right away)
                jobs.run(&rt);
            } else {
                std::thread::spawn(move || jobs.run(&rt));
            }
        }
    }

    // ---------------------------------------------------------------- building
    fn monbar(&self, cx: &mut Cx) -> El {
        let mut sorted: Vec<(usize, &MonitorInfo)> = self.mons.iter().enumerate().collect();
        sorted.sort_by_key(|(_, m)| m.number);
        let items: Vec<(String, String)> = sorted
            .iter()
            .map(|(_, m)| {
                let brand = m.name.split_whitespace().next().unwrap_or("Monitor").to_string();
                let inch = m.diagonal_inches.map(|d| format!(" {}″", d.round() as u32)).unwrap_or_default();
                (m.number.to_string(), format!("{brand}{inch}"))
            })
            .collect();
        let on = sorted.iter().position(|(i, _)| *i == self.sel).unwrap_or(0);
        let labels: Vec<Label> = items.iter().map(|(n, name)| Label::Mon(n, name)).collect();
        let mut ms = segx::seg_ex(cx, K_MON, &labels, Some(on), &segx::MONSEG);
        // Order 045: `const t=m.name+' · '+fmt(m.cur,m);monBtns[i].title=t` (fmt = "1920 × 1080 · 165 Hz", hzLab of that monitor's rates)
        for (j, (i, m)) in sorted.iter().enumerate() {
            let rates = self.modes.get(*i).map(|md| fl::all_rates(md)).unwrap_or_default();
            let t = format!("{} \u{b7} {} \u{d7} {} \u{b7} {} Hz", m.name, m.current.width, m.current.height, fl::rate_label(m.current.refresh, &rates));
            if let Some(b) = ms.children.iter_mut().find(|c| c.key == Some(idx(K_MON, j))) {
                *b = std::mem::take(b).title(&t);
            }
        }
        // Apply: the picked monitor "blinks" like a mode change (pill opacity 1 → .25 at 30 % → 1, 420 ms ease-out)
        if let Some(t0) = self.blink {
            let p = ((self.now - t0) / 420.0).clamp(0.0, 1.0);
            if p < 1.0 {
                let e = crate::anim::EASE_OUT_CSS.ease(p) as f32;
                let o = if e < 0.3 { 1.0 - 0.75 * e / 0.3 } else { 0.25 + 0.75 * (e - 0.3) / 0.7 };
                // the pill sits in the first child (seg_ex's strip, or the pill itself)
                if let Some(pill) = ms.children.first_mut() {
                    pill.opacity = o;
                }
                cx.st.busy = true;
            }
        }
        // .monbar{position:relative;z-index:2;display:flex;align-items:center;justify-content:center;gap:8px;margin:6px 0 14px}
        El::row()
            .center()
            .justify(JustifyContent::CENTER)
            .gap(8.0)
            .margin(6.0, 0.0, 14.0, 0.0)
            .z(2)
            .child(ms)
            .child(tip::idb(cx, K_ID, "ident", "Identify monitors"))
    }

    fn nf(&self, cx: &mut Cx, fld: Fld, label: &str) -> El {
        let i = fld as usize;
        let k = fld.key();
        let editing = self.edit.as_ref().map(|e| e.fld == fld).unwrap_or(false);
        let all = self.edit.as_ref().map(|e| e.all).unwrap_or(false);
        let text = self.field_text(fld);
        // the shared nbox owns the snap cue (accent colour 650 ms + the 2 px nudge), the arrow-key step cue (180 ms) and the
        // select-on-focus look; it reads its focus ring from the frame's focus
        let step = self.step[i].filter(|kf| kf.dir != 0.0).map(|kf| (kf.t0, kf.dir as i8));
        let cue = nbox::Cue { snap_at: self.snap[i], step, selected: all && editing };
        let opts = if fld == Fld::Hz { &nbox::HZ } else { &nbox::NUM };
        // Order 045: the wheel over the box steps the value (`box.addEventListener('wheel',e=>{e.preventDefault();
        // F.step(e.deltaY<0?1:-1);})`, L3730)
        let mut b = nbox::nbox(cx, k, &text, "", opts, &cue).wheel_steps();
        // a monitor switch: the value comes in (opacity 0 → 1, translateY(4px) → 0, 220 ms, 35 ms apart)
        if let Some(kf) = self.step[i].filter(|kf| kf.dir == 0.0) {
            if self.now < kf.t0 + 220.0 {
                let e = if self.now < kf.t0 { 0.0 } else { ease_t(self.now, kf.t0, 220.0, EASE_OUT) };
                // nbox -> its input box -> its clipped content (the text, the selection, the caret)
                if let Some(content) = b.children.first_mut().and_then(|inp| inp.children.first_mut()) {
                    for c in content.children.iter_mut() {
                        c.opacity = e;
                        c.translate = (0.0, 4.0 * (1.0 - e));
                    }
                }
                cx.st.busy = true;
            }
        }
        // .nf{display:flex;flex-direction:column;align-items:center;gap:5px} .nf>span{font-size:11px;line-height:13px;color:var(--fg3)}
        let mut lab = El::block().child(El::text(label, Font::new(11.0, 400), FG3(), 13.0));
        if fld == Fld::Hz {
            // .nf>span em{position:absolute;left:calc(100% + 5px);top:0;color:var(--fg2);tabular-nums;opacity:0;
            //   transform:translateY(2px);transition:opacity .15s ease,transform .15s ease} shown on hover / focus after .12 s
            let show = cx.hovered(K_HZF) || editing;
            let t = cx.tr_delayed(sub(K_HZF, "em"), 1, if show { 1.0 } else { 0.0 }, 150.0, if show { 120.0 } else { 0.0 }, EASE);
            let ex = self.f.map(|f| format!("{:.2}", f.hz.hz())).unwrap_or_default();
            let lw = cx.g.text_width(label, Font::new(11.0, 400));
            lab = lab.child(El::text(ex, Font::new(11.0, 400).tnum(), FG2(), 13.0).abs(lw + 5.0, 0.0, f32::NAN, f32::NAN).opacity(t).translate(0.0, 2.0 * (1.0 - t)).no_hit());
        }
        let mut nf = El::col().center().gap(5.0).none().child(lab).child(b);
        if fld == Fld::Hz {
            nf = nf.key(K_HZF);
        }
        nf
    }

    fn mong(&self, cx: &mut Cx) -> El {
        let Some(f) = self.f else { return group::grp(vec![]) };
        // .rx{width:22px;height:32px;line-height:32px;text-align:center;font-size:13px;color:var(--fg3)}
        let rx = || El::text("×", Font::new(13.0, 400), FG3(), 32.0).size(22.0, 32.0).none().align(Align::Center);
        // #sw .apb{margin-left:18px;height:32px;padding:0 20px;border-radius:8px;background:var(--acc);color:#fff;font-size:13px;
        //   font-weight:600;box-shadow:inset 0 0 0 .5px rgba(255,255,255,.2),inset 0 1px 0 rgba(255,255,255,.14),0 1px 3px rgba(0,0,0,.22)}
        //   :hover{filter:brightness(1.08)} :active{transform:scale(.97)}
        let hv = cx.hover_t(K_APPLY, 150.0, EASE);
        let pr = cx.active_t(K_APPLY, 120.0, EASE);
        let br = |c: Rgba| Rgba((c.0 * (1.0 + 0.08 * hv)).min(1.0), (c.1 * (1.0 + 0.08 * hv)).min(1.0), (c.2 * (1.0 + 0.08 * hv)).min(1.0), c.3);
        let apb = El::row()
            .center()
            .h(32.0)
            .none()
            .margin(0.0, 0.0, 0.0, 18.0)
            .pad(0.0, 20.0, 0.0, 20.0)
            .radius(8.0)
            .bg(br(ACC()))
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.2)), sh(0.0, 1.0, 0.0, 0.0, Rgba(1.0, 1.0, 1.0, 0.14))])
            .shadow(&[sh(0.0, 1.0, 3.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.22))])
            .scale(1.0 - 0.03 * pr)
            .on_click(K_APPLY)
            .cursor(Cursor::Hand)
            .child(El::text("Apply", Font::new(13.0, 600).ls(0), WHITE, lh(13.0, 1.35)));
        // .row.resrow{justify-content:center;align-items:flex-end;gap:0;padding:12px 12px 14px} + .grp.mong .row{padding-left/right:14px}
        let res = group::row_ex(true, 14.0, vec![self.nf(cx, Fld::W, "Width"), rx(), self.nf(cx, Fld::H, "Height"), rx(), self.nf(cx, Fld::Hz, "Hz"), apb])
            .pad(12.0, 14.0, 14.0, 14.0)
            .gap(0.0)
            .justify(JustifyContent::CENTER)
            .items(AlignItems::FLEX_END);
        let scale = group::row_ex(
            false,
            14.0,
            vec![group::lbl("Scaling", Some(f.sc.sub_line())), group::ctl(vec![seg::seg(cx, K_SCALE, &["Stretch", "Black bars", "Keep aspect"], sc_index(f.sc), true)])],
        )
        .pad(7.0, 14.0, 7.0, 14.0);
        let is_main = self.mons.get(self.sel).map(|m| m.is_main).unwrap_or(false);
        // Order 045: `mainTg.title=on?'One monitor is always the main one: …':'Make this the main display'`
        let tg = toggle::toggle(cx, K_MAIN, is_main, false).title(if is_main {
            "One monitor is always the main one: turn it on for the other monitor to move it there"
        } else {
            "Make this the main display"
        });
        let s = self.nudge_scale(K_MAIN);
        let main = group::row_ex(false, 14.0, vec![group::lbl("Main display", None), group::ctl(vec![tg.scale(s)])]).pad(7.0, 14.0, 7.0, 14.0).min_h(46.0);
        group::grp(vec![res, scale, main])
    }

    fn picture(&self, cx: &mut Cx) -> Vec<El> {
        let p = self.pic.get(self.sel).copied().flatten();
        let ok = p.map(|p| p.ddc == DdcState::Answers).unwrap_or(true);
        // a monitor that doesn't answer has no value to show: its greyed-out sliders rest in the middle
        let pct = |v: Option<bu_display::VcpValue>| v.map(|v| v.percent()).unwrap_or(50);
        let (b, c) = (p.map(|p| pct(p.brightness)).unwrap_or(50), p.map(|p| pct(p.contrast)).unwrap_or(50));
        let vib = p.and_then(|p| p.vibrance_percent);
        // .msl{display:flex;align-items:center;gap:10px} .msl .rng{width:168px} .msl .sv{min-width:50px}
        // .row.dim{opacity:.4;pointer-events:none} + .row.dim .msl .rng track{background:var(--trk)}
        let msl = |cx: &mut Cx, k: Key, v: u8, dim: bool| {
            let look = if dim { slider::Look { fill: TRK(), ..slider::default() } } else { slider::default() };
            El::row().center().gap(10.0).none().child(slider::slider(cx, k, v as f32 / 100.0, 168.0, 20.0, look)).child(slider::value_label(&format!("{v} %")).min_w(50.0))
        };
        let mut rows = Vec::new();
        for (i, (k, name, v)) in [(K_BRI, "Brightness", b), (K_CON, "Contrast", c)].into_iter().enumerate() {
            let mut r = group::row_ex(i == 0, 14.0, vec![group::lbl(name, None), group::ctl(vec![msl(cx, k, v, !ok)])]).pad(7.0, 14.0, 7.0, 14.0);
            if !ok {
                r = r.opacity(0.4).no_hit();
            }
            rows.push(r);
        }
        if vib.is_some() || p.is_none() {
            let vendor = match p.and_then(|p| p.vibrance_vendor) {
                Some(bu_display::GpuVendor::Amd) => "AMD saturation · 50 % is normal",
                _ => "NVIDIA digital vibrance · 50 % is normal",
            };
            rows.push(group::row_ex(false, 14.0, vec![group::lbl("Vibrance", Some(vendor)), group::ctl(vec![msl(cx, K_VIB, vib.unwrap_or(50), false)])]).pad(7.0, 14.0, 7.0, 14.0));
        }
        let mut out = vec![group::gh("Picture"), group::grp(rows)];
        if !ok {
            if let Some(m) = self.mons.get(self.sel) {
                let brand = m.name.split_whitespace().next().unwrap_or("Monitor");
                let inch = m.diagonal_inches.map(|d| format!(" {}″", d.round() as u32)).unwrap_or_default();
                out.push(group::gf(&format!("{brand}{inch} doesn’t answer · turn on DDC/CI in the monitor’s own menu.")));
            }
        }
        out
    }

    fn preset_card(&self, cx: &mut Cx, p: &Preset, on: bool) -> El {
        let k = idx(K_PST, p.id.0 as usize);
        let hv = cx.hover_t(k, 150.0, EASE);
        let pr = cx.active_t(k, 120.0, EASE);
        let ont = cx.tr(k, 3, if on { 1.0 } else { 0.0 }, 200.0, EASE);
        let hovered = cx.hovered(k);
        // .pck: opacity / scale(.6) → 1 (.18s ease / .26s cubic-bezier(.3,1.35,.5,1)); hidden while the card is hovered
        let ck_on = on && !hovered;
        let ck_o = cx.tr(sub(k, "ck"), 1, if ck_on { 1.0 } else { 0.0 }, 180.0, EASE);
        let ck_s = cx.tr(sub(k, "ck"), 2, if ck_on { 1.0 } else { 0.6 }, 260.0, POP);
        // #sw .pdel: opacity .14s ease, shown on card hover after .06 s
        let del_k = sub(k, "del");
        let del_o = cx.tr_delayed(del_k, 1, if hovered { 1.0 } else { 0.0 }, 140.0, if hovered { 60.0 } else { 0.0 }, EASE);
        let del_h = cx.hover_t(del_k, 120.0, EASE);
        let gl = glyph(p, if on { ICO_ON() } else { FG3() }, ont);
        let font = Font::new(12.0, 600).tnum();
        let pt = El::row()
            .flex1()
            .clip()
            .child(El::text(preset_text(p), font, FG(), 16.0).none())
            .child(El::text(format!(" · {}", p.scaling.label()), Font::new(12.0, 400).tnum(), FG2(), 16.0).ellipsis());
        let mut pse = El::block()
            .size(16.0, 16.0)
            .none()
            .child(El::icon("dcheck", 16.0, 1.8, ICO_ON()).abs(0.0, 0.0, f32::NAN, f32::NAN).opacity(ck_o).scale(ck_s).no_hit());
        let mut del = El::block()
            .abs(-2.0, -2.0, f32::NAN, f32::NAN)
            .size(20.0, 20.0)
            .radius(5.0)
            .bg(Rgba::rgba(255, 69, 58, 0.14 * del_h))
            .place_center()
            .opacity(del_o)
            // Order 045: `h('button',{class:'pdel',title:'Delete',…})`
            .title("Delete")
            .child(El::icon("x", 8.0, 1.5, cmix(FG2(), RED(), del_h)).no_hit());
        if hovered || del_o > 0.001 {
            del = del.on_click(del_k).cursor(Cursor::Hand);
        } else {
            del = del.no_hit();
        }
        pse = pse.child(del);
        // .pst::before (hover --hov; .on: --sel at .5 / .72 hovered)
        let before = if on { SEL().mul_a(0.5 + 0.22 * hv) } else { HOV().mul_a(hv) };
        let mut c = El::row()
            .center()
            .gap(9.0)
            .h(36.0)
            .min_w(0.0)
            .pad(0.0, 8.0, 0.0, 11.0)
            .radius(9.0)
            .bg(GRP())
            .inset(&[sh(0.0, 0.0, 0.0, 0.5 + 0.5 * ont, cmix(HAIR(), ACC_S(), ont))])
            .scale((1.0 - 0.03 * pr) * self.nudge_scale(k))
            .on_click(k)
            .cursor(Cursor::Hand)
            .child(El::block().abs(0.0, 0.0, 0.0, 0.0).radius(9.0).bg(before).no_hit())
            .child(gl)
            .child(pt)
            .child(pse);
        // a new chip: opacity 0 / scale(.9) → 1, 280 ms; a deleted one: → 0 / .94, 160 ms, then the list closes up
        if let Some((id, t0)) = self.pst_in {
            if id == p.id && self.now - t0 < 280.0 {
                let e = ease_t(self.now, t0, 280.0, EASE_OUT);
                c = c.opacity(e).scale(0.9 + 0.1 * e);
                cx.st.busy = true;
            }
        }
        if let Some((id, t0)) = self.pst_out {
            if id == p.id {
                let e = ease_t(self.now, t0, 160.0, BAR_OUT);
                c = c.opacity(1.0 - e).scale(1.0 - 0.06 * e).no_hit();
                // Order 047: frames only while it fades (its end is `wake_at`)
                if e < 1.0 {
                    cx.st.busy = true;
                }
            }
        }
        c
    }

    fn presets_grid(&self, cx: &mut Cx) -> El {
        let cur = self.current_preset();
        let ps = self.presets();
        // .pgrid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px}
        let mut g = El::grid().cols(2).gap(8.0);
        // the other chips glide to their new places (300 ms EASE_OUT): each chip's slot animates
        for (i, p) in ps.iter().enumerate() {
            let k = idx(K_PST, p.id.0 as usize);
            let col = (i % 2) as f32;
            let row = (i / 2) as f32;
            let w = (548.0 - 8.0) / 2.0;
            let x = cx.tr(sub(k, "slot"), 1, col * (w + 8.0), 300.0, EASE_OUT);
            let y = cx.tr(sub(k, "slot"), 2, row * 44.0, 300.0, EASE_OUT);
            let card = self.preset_card(cx, p, Some(p.id) == cur);
            g = g.child(card.translate(x - col * (w + 8.0), y - row * 44.0));
        }
        // .pst.pnew{padding:0;justify-content:center;background:transparent;box-shadow:none} ::after{border:1px dashed var(--dash)}
        //   :hover::after{border-color:var(--acc-s)} .pnew svg{width:14px;height:14px;stroke:var(--ico-on);stroke-width:1.6}
        let hv = cx.hover_t(K_PNEW, 200.0, EASE);
        let pr = cx.active_t(K_PNEW, 120.0, EASE);
        // .pst::before (the hover wash) also covers the dashed chip: --hov, .15 s
        let on = cx.hovered(K_PNEW);
        let hvb = cx.tr(K_PNEW, 7, if on { 1.0 } else { 0.0 }, 150.0, EASE);
        let dash = cmix(DASH(), ACC_S(), hv);
        let n = ps.len();
        let (col, row) = ((n % 2) as f32, (n / 2) as f32);
        let w = (548.0 - 8.0) / 2.0;
        let x = cx.tr(sub(K_PNEW, "slot"), 1, col * (w + 8.0), 300.0, EASE_OUT);
        let y = cx.tr(sub(K_PNEW, "slot"), 2, row * 44.0, 300.0, EASE_OUT);
        // Order 045: `addCard.title='Save as preset · '+pLab(D.f,selMon())` (pLab = fmt + ' · ' + the scaling's name)
        let save_t = self
            .f
            .map(|f| format!("Save as preset \u{b7} {} \u{d7} {} \u{b7} {} Hz \u{b7} {}", f.w, f.h, self.hz_label(f.hz), f.sc.label()))
            .unwrap_or_else(|| "Save as preset".into());
        let add = El::row()
            .center()
            .justify(JustifyContent::CENTER)
            .h(36.0)
            .radius(9.0)
            .scale(1.0 - 0.03 * pr)
            .translate(x - col * (w + 8.0), y - row * 44.0)
            .on_click(K_PNEW)
            .title(&save_t)
            .cursor(Cursor::Hand)
            .child(El::block().abs(0.0, 0.0, 0.0, 0.0).radius(9.0).bg(HOV().mul_a(hvb)).no_hit())
            .child(El::paint(move |g, (x, y, w, h)| dashed_rr(g, x, y, w, h, 9.0, dash)).abs(0.0, 0.0, 0.0, 0.0).no_hit())
            .child(El::icon("dplus", 14.0, 1.6, ICO_ON()).no_hit());
        g.child(add)
    }

    fn rule_row(&self, cx: &mut Cx, i: usize, r: &bu_display::autoswitch::AppRule, first: bool) -> El {
        let k = idx(K_RULE, i);
        let hovered = cx.hovered(k);
        let off = cx.tr(sub(k, "off"), 1, if r.enabled { 0.0 } else { 1.0 }, 200.0, EASE);
        let op = 1.0 - 0.55 * off;
        // the app button: .pu.app{width:144px;max-width:none;padding-left:4px} = tile slot + name (+ the chevron)
        let a = if r.exe.is_empty() { None } else { Some(r.exe.as_str()) };
        let mut kids = Vec::new();
        if let Some(exe) = a {
            kids.push(match apps::known(exe) {
                Some(app) => app_tile(app),
                None => other_tile(),
            });
        }
        let (name, col) = match a {
            Some(exe) => (apps::name_of(exe), FG()),
            None => ("Choose an app".to_string(), FG3()),
        };
        // Order 045: `vibB.dataset.tip=r.vib==null?'Vibrance: no change · click to set':'Vibrance '+r.vib+' % while '+(a?a[1]:'it')+' runs'`
        let vib_tip = match r.vibrance {
            None => "Vibrance: no change \u{b7} click to set".to_string(),
            Some(v) => format!("Vibrance {v} % while {} runs", if a.is_some() { name.as_str() } else { "it" }),
        };
        kids.push(El::text(name, pieces::btn_font(13.0, 400), col, lh(13.0, 1.35)).ellipsis().flex1_auto());
        let app = pieces::dropdown::dropdown_with(cx, sub(k, "app"), kids, 4.0, 6.0).w(144.0).opacity(op);
        // .rar{width:16px;height:16px;color:var(--fg3)} svg{stroke-width:1.4}
        let rar = El::icon("darrow", 16.0, 1.4, FG3()).opacity(op);
        let pre_lab = r.preset.and_then(|id| self.presets().into_iter().find(|p| p.id == id)).map(|p| format!("{} · {}", preset_text(&p), p.scaling.label()));
        let (pl, pc) = match pre_lab {
            Some(t) => (t, FG()),
            None => ("Choose a preset".to_string(), FG3()),
        };
        // .pu.pre{flex:1;min-width:0;max-width:none}
        // Order 045: `preB.title=preLab.textContent`
        let pre = pieces::dropdown::dropdown_with(cx, sub(k, "pre"), vec![El::text(pl.clone(), pieces::btn_font(13.0, 400), pc, lh(13.0, 1.35)).ellipsis().flex1_auto()], 10.0, 6.0)
            .title(&pl)
            .flex1()
            .style(|s| s.flex_shrink = 1.0)
            .opacity(op);
        // #sw .vch{display:inline-flex;align-items:center;justify-content:center;gap:4px;width:56px;height:24px;padding:0 4px;
        //   border-radius:6px;background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair),0 .5px 1px rgba(0,0,0,.12);font-size:12px;
        //   tabular-nums} :hover{--ctl-h} .none{color:var(--fg3)} .vch i{11px circle, conic rainbow, inset .5px rgba(0,0,0,.25)}
        let vk = sub(k, "vib");
        let vh = cx.hover_t(vk, 150.0, EASE);
        let vp = cx.active_t(vk, 120.0, EASE);
        let none = r.vibrance.is_none();
        let vl = r.vibrance.map(|v| format!("{v} %")).unwrap_or_else(|| "—".into());
        let vch = El::row()
            .center()
            .justify(JustifyContent::CENTER)
            .gap(4.0)
            .size(56.0, 24.0)
            .none()
            .pad(0.0, 4.0, 0.0, 4.0)
            .radius(6.0)
            .bg(cmix(CTL(), CTL_H(), vh))
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
            .shadow(&[sh(0.0, 0.5, 1.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.12))])
            .scale(1.0 - 0.04 * vp)
            .opacity(op)
            .on_click(vk)
            .tip(&vib_tip)
            .cursor(Cursor::Hand)
            .child(El::paint(move |g, (x, y, w, _)| rainbow_dot(g, x + w / 2.0, y + 5.5, none)).size(11.0, 11.0).none().no_hit())
            .child(El::text(vl, pieces::btn_font(12.0, 400).tnum(), if none { FG3() } else { FG() }, lh(12.0, 1.35)).none().no_hit());
        // #sw .rdel: shown while the rule row is hovered
        // Order 045: `h('button',{class:'rdel',title:'Remove',…})`
        let del = rowbits::rdel(cx, sub(k, "del"), hovered).title("Remove");
        let tg = toggle::toggle(cx, sub(k, "tg"), r.enabled, false);
        // .row.rule{gap:6px;min-height:46px}
        let mut row = group::row(first, vec![app, rar, pre, vch, del, tg]).gap(6.0).min_h(46.0).key(k);
        if let Some((id, t0)) = self.rule_in {
            if id == r.id.0 && self.now - t0 < 260.0 {
                let e = ease_t(self.now, t0, 260.0, EASE_OUT);
                row = row.opacity(e).translate(0.0, -6.0 * (1.0 - e));
                cx.st.busy = true;
            }
        }
        if let Some((id, t0)) = self.rule_out {
            if id == r.id.0 {
                let e = ease_t(self.now, t0, 220.0, EASE_OUT);
                row = row.opacity(1.0 - e).clip().style(move |s| {
                    s.size.height = Dimension::length(46.0 * (1.0 - e));
                    s.min_size.height = LengthPercentageAuto::length(0.0);
                });
                if e < 1.0 {
                    cx.st.busy = true;
                }
            }
        }
        row
    }

    fn rules_grp(&self, cx: &mut Cx) -> El {
        let rules = self.rules();
        let mut rows: Vec<El> = rules.iter().enumerate().map(|(i, r)| self.rule_row(cx, i, r, i == 0)).collect();
        // .row.addr{min-height:40px}
        let addb = bits::addb(cx, K_ADD, "Add app", false);
        rows.push(group::row(rules.is_empty(), vec![addb]).min_h(40.0));
        group::grp(rows)
    }

    /// The keep bar (`.cfm`, at the window's bottom): window coordinates.
    fn keep_bar(&mut self, cx: &mut Cx) -> Option<El> {
        let rt = self.rt.clone()?;
        let (on, t0) = match (self.cfm_on, self.cfm_off) {
            (Some(t), _) => (true, t),
            (None, Some(t)) if self.now - t < 200.0 => (false, t),
            _ => return None,
        };
        // built every frame while the bar shows: never wait for the service (a DDC/CI worker may hold it for ~100 ms) -
        // when it is busy, the deadline read last frame is used
        let total = bu_display::service::KEEP_SECONDS as f64 * 1000.0;
        let deadline = match rt.svc.try_lock() {
            Ok(s) => {
                let d = s.pending().map(|p| p.deadline);
                self.bar_deadline = d;
                d
            }
            Err(_) => self.bar_deadline,
        };
        let left_ms = deadline.map(|d| d.saturating_duration_since(Instant::now()).as_millis() as f64);
        let left_ms = if self.frozen { Some(total) } else { left_ms.or(if on { None } else { Some(0.0) }) };
        let left_ms = left_ms?;
        let secs = (left_ms / 1000.0).ceil().max(0.0) as u64;
        let drained = (1.0 - left_ms / total).clamp(0.0, 1.0) as f32;
        // .cfm{opacity:0;transform:translateY(12px) scale(.985);transition:opacity .16s ease,transform .2s cubic-bezier(.4,0,1,1)}
        // .cfm.on{opacity:1;transform:none;transition:opacity .2s ease,transform .38s cubic-bezier(.3,1.2,.5,1)}
        let (o, m) = if on { (ease_t(self.now, t0, 200.0, EASE), ease_t(self.now, t0, 380.0, BAR_IN)) } else { (1.0 - ease_t(self.now, t0, 160.0, EASE), 1.0 - ease_t(self.now, t0, 200.0, BAR_OUT)) };
        cx.st.busy = true;
        // .ring{32px} svg rotate(-90deg); circle r13 stroke-width 2.5; .rb{stroke:var(--trk)} .rf{stroke:var(--acc);linecap round};
        // b{font:600 12px/1;tabular-nums}
        let ring = El::block()
            .size(32.0, 32.0)
            .none()
            .child(El::paint(move |g, (x, y, _, _)| ring(g, x + 16.0, y + 16.0, drained)).abs(0.0, 0.0, 0.0, 0.0))
            .child(El::text(secs.to_string(), Font::new(12.0, 600).tnum(), FG(), 32.0).w(32.0).align(Align::Center));
        let lbl = El::col()
            .flex1()
            .child(El::text("Keep these settings?", Font::new(13.0, 600), FG(), lh(13.0, 1.35)))
            .child(El::text(format!("Reverting in {secs} s"), Font::new(11.0, 400).tnum(), FG2(), lh(11.0, 1.35)).margin(1.0, 0.0, 0.0, 0.0));
        let bar = El::row()
            .abs(14.0, crate::ui::WIN_H - 14.0 - 58.0, f32::NAN, f32::NAN)
            .w(crate::ui::WIN_W - 28.0)
            .center()
            .gap(12.0)
            .h(58.0)
            .pad(0.0, 12.0, 0.0, 13.0)
            .radius(12.0)
            .bg(MENU())
            .backdrop(30.0, 1.8)
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, HL())])
            .shadow(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, 0.35)), sh(0.0, 16.0, 40.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.42))])
            .opacity(o)
            .translate(0.0, 12.0 * (1.0 - m))
            .scale(0.985 + 0.015 * m)
            .z(7)
            .key(K_CFM)
            .child(ring)
            .child(lbl)
            .child(button::cbtn(cx, K_REVERT, "Revert", button::Kind::Ghost, false, false, 0.0))
            .child(button::cbtn(cx, K_KEEP, "Keep", button::Kind::Primary, false, false, 0.0));
        Some(if on { bar } else { bar.no_hit() })
    }
}

/// A known app's tile (`bits::at`: its 135° gradient and glyph); inside a click target, so it takes no clicks itself.
fn app_tile(a: &apps::App) -> El {
    bits::at(a.glyph, a.grad.0, a.grad.1, false).no_hit()
}

/// A browsed app (not in the list): the same tile in a neutral grey with the app glyph (the drawing has no example).
fn other_tile() -> El {
    bits::at("app", Rgba::rgb(142, 142, 154), Rgba::rgb(92, 92, 104), false).no_hit()
}

/// The preset glyph (`.pgl`, 20 × 15; the drawing's `presetGlyph`, viewBox 30 × 22): a tiny screen and the picture in it -
/// filling it (Stretch), keeping its shape (Keep aspect) or real-size in black bars. `.scr{stroke:var(--fg3);
/// stroke-width:1.2}` `.img{fill:var(--fg3);fill-opacity:.5}` (both `--ico-on` on the chosen chip, .2 s).
fn glyph(p: &Preset, col: Rgba, _on: f32) -> El {
    let (pw, ph, sc) = (p.width as f32, p.height as f32, p.scaling);
    El::paint(move |g, (x, y, _, _)| {
        // viewBox 30 x 22 in 20 x 15 (xMidYMid meet): scale 2/3, centred vertically
        let s = 20.0 / 30.0;
        let oy = (15.0 - 22.0 * s) / 2.0;
        let tx = |v: f32| x + v * s;
        let ty = |v: f32| y + oy + v * s;
        let (xx, yy, ww, hh) = (3.5f32, 4.5f32, 23.0f32, 11.2f32);
        let (mut w, mut h) = (ww, hh);
        match sc {
            GpuScaling::KeepAspect => {
                let a = pw / ph;
                if a < ww / hh {
                    w = hh * a;
                } else {
                    h = ww / a;
                }
            }
            GpuScaling::BlackBars => {
                w = ww * (pw / 1920.0).min(1.0);
                h = hh * (ph / 1080.0).min(1.0);
            }
            _ => {}
        }
        let r2 = |v: f32| (v * 100.0).round() / 100.0;
        g.fill_rr(tx(r2(xx + (ww - w) / 2.0)), ty(r2(yy + (hh - h) / 2.0)), r2(w) * s, r2(h) * s, 1.0 * s, col.mul_a(0.5));
        g.stroke_rr(tx(1.5), ty(2.5), 27.0 * s, 15.2 * s, 2.4 * s, 1.2 * s, col);
        g.line(tx(11.0), ty(20.5), tx(19.0), ty(20.5), 1.2 * s, col, true);
        g.line(tx(15.0), ty(17.7), tx(15.0), ty(20.5), 1.2 * s, col, true);
    })
    .size(20.0, 15.0)
    .none()
    .no_hit()
}

/// A 1 px dashed rounded border (the dashed "+" chip's `::after{border:1px dashed}`), drawn the way Chromium's raster shows
/// it (measured on the drawing): the border box snapped to whole pixels, ONE dash pattern along the whole rounded outline
/// (the stroke's centre line), 3 px dashes with the gap closest to 2 px that fits the outline evenly, starting where the top
/// edge leaves the top-left corner. Also the Mouse cursor picker's "Choose your own file" tile (`.cmi.own .cpv`).
pub(crate) fn dashed_rr(g: &crate::gfx::Gfx, x: f32, y: f32, w: f32, h: f32, r: f32, c: Rgba) {
    use skia_safe as sk;
    let (x0, y0, x1, y1) = (x.round(), y.round(), (x + w).round(), (y + h).round());
    let (l, t, rr, b) = (x0 + 0.5, y0 + 0.5, x1 - 0.5, y1 - 0.5);
    let k = (r - 0.5).max(0.0);
    // corners as SVG arcs (radius k, clockwise), like the outline Blink strokes
    use sk::{path_builder::ArcSize, PathBuilder, PathDirection};
    let mut pb = PathBuilder::new();
    pb.move_to((l + k, t));
    pb.line_to((rr - k, t));
    pb.arc_to_radius((k, k), 0.0, ArcSize::Small, PathDirection::CW, (rr, t + k));
    pb.line_to((rr, b - k));
    pb.arc_to_radius((k, k), 0.0, ArcSize::Small, PathDirection::CW, (rr - k, b));
    pb.line_to((l + k, b));
    pb.arc_to_radius((k, k), 0.0, ArcSize::Small, PathDirection::CW, (l, b - k));
    pb.line_to((l, t + k));
    pb.arc_to_radius((k, k), 0.0, ArcSize::Small, PathDirection::CW, (l + k, t));
    pb.close();
    let path = pb.detach();
    let len = 2.0 * (rr - l - 2.0 * k) + 2.0 * (b - t - 2.0 * k) + 2.0 * std::f32::consts::PI * k;
    let (dash, want_gap) = (3.0f32, 2.0f32);
    let n0 = (len / (dash + want_gap)).floor().max(1.0);
    let g0 = (len - n0 * dash) / n0;
    let g1 = (len - (n0 + 1.0) * dash) / (n0 + 1.0);
    let gap = if g1 <= 0.0 || (g0 - want_gap).abs() < (g1 - want_gap).abs() { g0 } else { g1 };
    // the dashes as geometry (stroked by the painter, which knows its static / live pass)
    let mut dashes = PathBuilder::new();
    if let Some(m) = sk::ContourMeasureIter::new(&path, false, None).next() {
        let total = m.length();
        let mut d = 0.0;
        while d < total - 0.01 {
            let _ = m.segment(d, (d + dash).min(total), &mut dashes, true);
            d += dash + gap;
        }

    }
    g.stroke_geom_ex(&dashes.detach(), 1.0, c, false, true, 1.0);
}

/// The keep bar's ring: the track circle and the accent arc that drains clockwise from the top over 10 s.
fn ring(g: &crate::gfx::Gfx, cx: f32, cy: f32, drained: f32) {
    use skia_safe as sk;
    let oval = sk::Rect::from_xywh(cx - 13.0, cy - 13.0, 26.0, 26.0);
    g.stroke_oval(oval, 2.5, TRK(), 1.0);
    let sweep = 360.0 * (1.0 - drained);
    if sweep > 0.01 {
        let mut pb = sk::PathBuilder::new();
        pb.arc_to(oval, -90.0, sweep.min(359.99), true);
        g.stroke_geom(&pb.detach(), 2.5, ACC());
    }
}

/// `.vch i`: an 11 px circle with `conic-gradient(#ff5a52,#ffd60a,#30d158,#2fd6c4,#0a84ff,#bf5af2,#ff5a52)`,
/// `box-shadow:inset 0 0 0 .5px rgba(0,0,0,.25)`; `.none i{filter:grayscale(1);opacity:.55}`.
fn rainbow_dot(g: &crate::gfx::Gfx, cx: f32, cy: f32, grey: bool) {
    use skia_safe as sk;
    let hexes = [0xff5a52u32, 0xffd60a, 0x30d158, 0x2fd6c4, 0x0a84ff, 0xbf5af2, 0xff5a52];
    let cols: Vec<sk::Color4f> = hexes
        .iter()
        .map(|h| {
            let c = Rgba::hex(*h);
            if grey { c.gray().mul_a(0.55).c4() } else { c.c4() }
        })
        .collect();
    let m = sk::Matrix::rotate_deg_pivot(-90.0, (cx, cy));
    let shader = sk::gradient_shader::sweep((cx, cy), (&cols[..], None), None, sk::TileMode::Clamp, None, None, Some(&m));
    if let Some(s) = shader {
        g.fill_rr_shader(cx - 5.5, cy - 5.5, 11.0, 11.0, 5.5, &s, 1.0);
    }
    g.stroke_oval(sk::Rect::from_xywh(cx - 5.25, cy - 5.25, 10.5, 10.5), 0.5, Rgba(0.0, 0.0, 0.0, 0.25), 1.0);
}

impl Page for Display {
    fn id(&self) -> &'static str {
        "dsp"
    }
    fn name(&self) -> &'static str {
        "Display"
    }
    fn icon(&self) -> &'static str {
        "mon"
    }

    fn open(&mut self, env: &Env, now: f64) {
        #[cfg(windows)]
        let rt = if env.fake() { Rt::fake_sample() } else { Rt::shared(env.real_read) };
        #[cfg(not(windows))]
        let rt = Rt::fake_sample();
        self.open_rt(rt, env, now);
    }

    fn close(&mut self) {
        // Order 047: answers already in but not taken still write their change-log lines; a job still running writes its
        // own (its worker ends after it)
        if let Some(rx) = &self.ans {
            while let Ok(a) = rx.try_recv() {
                if let Ans::Kept { r: Ok(_), logs } | Ans::Main { r: Ok(true), logs, .. } = &a {
                    rec_logs(logs);
                }
            }
        }
        // the runtime lives on (countdown, rules); the page keeps nothing
        *self = Display::default();
    }

    /// The monitors are in (the last read or the fresh one) and the picked monitor's brightness / contrast / vibrance
    /// are read (DDC/CI, off the UI thread)
    fn ready(&self) -> bool {
        !self.mons.is_empty() && (self.pic_rx.is_none() || self.pic.get(self.sel).is_some_and(|p| p.is_some()))
    }
    /// Order 047: true only when something new came in or a nudge pulses; the keep bar, the chips' and rows' fades and
    /// the fields' cues ask for their own frames while they move (`st.busy` in the build), the timed ends are `wake_at`.
    /// (Identify's numbers are their own windows: the page needs no frames for them.)
    fn tick(&mut self, now: f64) -> bool {
        self.now = now;
        let mut busy = false;
        if let Some(rx) = &self.pic_rx {
            match rx.try_recv() {
                Ok((i, p)) => {
                    if let Some(slot) = self.pic.get_mut(i) {
                        *slot = Some(p);
                    }
                    self.pic_rx = None;
                    busy = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(_) => self.pic_rx = None,
            }
        }
        // the worker's answers (it woke the menu)
        while let Some(a) = self.ans.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.jobs = self.jobs.saturating_sub(1);
            self.landed(a);
            busy = true;
        }
        if let Some(rt) = self.rt.clone() {
            if rt.epoch() != self.epoch {
                self.epoch = rt.epoch();
                // (its answer draws)
                self.read_again(How::KeepFields);
            }
            let notes = rt.take_notes();
            if !notes.is_empty() {
                // the countdown went back by itself (or a rule left a note): read again, the bar goes when nothing waits
                if self.cfm_on.is_some() {
                    self.read_again(How::AfterNote);
                }
                if let Some(t) = notes.last() {
                    self.show_toast(t.clone());
                }
                busy = true;
            }
        }
        self.nudges.retain(|(_, t)| now - t < 340.0);
        if !self.nudges.is_empty() {
            busy = true;
        }
        if let Some((_, t0)) = self.pst_out {
            if now - t0 >= 160.0 {
                if let Some((id, _)) = self.pst_out.take() {
                    self.delete_preset(id);
                }
                busy = true;
            }
        }
        if let Some((_, t0)) = self.rule_out {
            if now - t0 >= 220.0 {
                if let Some((id, _)) = self.rule_out.take() {
                    if let Some(i) = self.rules().iter().position(|r| r.id.0 == id) {
                        self.remove_rule(i);
                    }
                }
                busy = true;
            }
        }
        busy
    }
    /// Order 047: a deleted chip / removed rule leaves the list when its fade has run (timed ends, no frames between).
    fn wake_at(&self, now: f64) -> Option<f64> {
        [self.pst_out.map(|(_, t)| t + 160.0), self.rule_out.map(|(_, t)| t + 220.0)].into_iter().flatten().map(|t| t.max(now + 1.0)).reduce(f64::min)
    }

    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        self.now = cx.now;
        // a dismiss is for the press that follows it at once (no build in between); one by Esc / a scroll is over here
        self.dismissed = None;
        let mut v = vec![pieces::header(self.name(), None)];
        if self.mons.is_empty() {
            return v;
        }
        v.push(self.monbar(cx));
        v.push(self.mong(cx));
        // .gwrap: the group header, its box, the footer
        v.push(El::block().children(self.picture(cx)));
        v.push(El::block().child(group::gh("Presets")).child(self.presets_grid(cx)));
        let gh = group::gh("Switch automatically").child(El::text("while an app is running", Font::new(11.0, 400), FG3(), lh(11.0, 1.35)));
        v.push(El::block().child(gh).child(self.rules_grp(cx)));
        v.push(pieces::reset::reset_line(cx, K_RESET, Some("Windows defaults")));
        v
    }

    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let mut layer = El::block().w(crate::ui::WIN_W).h(crate::ui::WIN_H).abs(0.0, 0.0, f32::NAN, f32::NAN).no_hit();
        let mut any = false;
        if let Some(bar) = self.keep_bar(cx) {
            layer = layer.child(bar);
            any = true;
        }
        if let Some(m) = &self.menu {
            if !m.r.0.is_nan() {
                let (kind, r) = (m.kind, m.r);
                layer = layer.child(self.menu_box(cx, kind, r));
                any = true;
            }
        }
        if let Some((t, at)) = self.toast.clone() {
            if cx.now - at < toast::SHOW_MS + 300.0 {
                layer = layer.child(toast::toast(cx, K_TOAST, &t, at, false));
                any = true;
            }
        }
        if any {
            Some(layer)
        } else {
            None
        }
    }

    fn popup_dismiss(&mut self) {
        self.dismissed = self.menu.take().map(|m| m.kind);
    }

    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        self.now = cx.now;
        cx.dirty = true;
        match ev {
            Ev::Press(k, _x, _y, r) => {
                for fld in [Fld::W, Fld::H, Fld::Hz] {
                    if *k == fld.key() {
                        self.begin_edit(fld);
                        if fld == Fld::Hz {
                            // the Hz field lists the monitor's rates when it gets focus (under the field: placeMenu, top + 4)
                            self.menu = Some(Menu { kind: MenuKind::Rates, r: *r });
                        } else {
                            self.menu = None;
                        }
                    }
                }
                for (which, sk) in [(0u8, K_BRI), (1, K_CON), (2, K_VIB)] {
                    if *k == sk {
                        self.set_picture(which, (slider::value_at(*r, *_x) * 100.0).round() as u8);
                    }
                }
                // a popup button: the list opens under it; a press on the button of the open list closes it (the frame
                // already dismissed it on this press - `dismissed` - so it must not open again)
                let open = self.menu.as_ref().map(|m| m.kind).or(self.dismissed.take());
                let rules = self.rules();
                for i in 0..rules.len() {
                    let rk = idx(K_RULE, i);
                    let anchor = |kind| Some(Menu { kind, r: *r });
                    if *k == sub(rk, "app") {
                        self.menu = if matches!(open, Some(MenuKind::App(j)) if j == i) { None } else { anchor(MenuKind::App(i)) };
                    } else if *k == sub(rk, "pre") {
                        self.menu = if matches!(open, Some(MenuKind::Preset(j)) if j == i) { None } else { anchor(MenuKind::Preset(i)) };
                    } else if *k == sub(rk, "vib") {
                        self.menu = if matches!(open, Some(MenuKind::Vib(j)) if j == i) { None } else { anchor(MenuKind::Vib(i)) };
                    }
                }
                if *k == sub(K_RESET, "pc") || *k == sub(K_RESET, "win") {
                    self.link_box = Some((*k, *r));
                }
                if *k == K_ADD {
                    // the new row's app list opens where its app button will be: under the add row's box
                    self.add_rule();
                    // the new row takes the add row's place: its app button (144 x 24) sits 6 px right of "Add app" (margin
                    // -6) and 4 px lower (a 46 px row instead of 40, both centred)
                    if let Some(MenuKind::App(i)) = self.menu.as_ref().map(|m| m.kind) {
                        self.menu = Some(Menu { kind: MenuKind::App(i), r: (r.0 + 6.0, r.1 + 4.0, 144.0, 24.0) });
                    }
                }
            }
            Ev::Drag(k, x, _y, r) => {
                for (which, sk) in [(0u8, K_BRI), (1, K_CON), (2, K_VIB)] {
                    if *k == sk {
                        self.set_picture(which, (slider::value_at(*r, *x) * 100.0).round() as u8);
                    }
                }
            }
            Ev::Char(k, c) => {
                if let Some(e) = self.edit.as_mut() {
                    if *k == e.fld.key() {
                        // width / height: up to 4 digits; the rate: up to 6 digits and '.'
                        let (max, filter) = if e.fld == Fld::Hz { (6, nbox::Filter::Decimal) } else { (4, nbox::Filter::Digits) };
                        nbox::type_char(&mut e.buf, &mut e.all, *c, max, filter);
                    }
                }
            }
            Ev::Key(k, vk) => {
                let fld = self.edit.as_ref().map(|e| e.fld);
                if let Some(fld) = fld {
                    if *k == fld.key() {
                        match *vk {
                            0x0D => {
                                self.commit_edit();
                                cx.focus(None);
                            }
                            0x1B => {
                                self.edit = None;
                                self.menu = None;
                                cx.focus(None);
                            }
                            0x26 | 0x28 => {
                                self.commit_edit();
                                self.step_field(fld, *vk == 0x26);
                                self.edit = Some(Edit { fld, buf: self.field_text(fld), all: false });
                            }
                            0x08 => {
                                if let Some(e) = self.edit.as_mut() {
                                    nbox::edit_key(&mut e.buf, &mut e.all, *vk);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Ev::Blur(k) => {
                if self.edit.as_ref().map(|e| e.fld.key() == *k).unwrap_or(false) {
                    self.commit_edit();
                }
            }
            Ev::Click(k) => self.click(*k, cx),
            // Order 045: the wheel over Width / Height / Refresh rate = `F.step(±1)` (the rate list closes, like
            // `if(menuBtn===box)closeMenu()`)
            Ev::Wheel(k, d) => {
                if let Some(fld) = [Fld::W, Fld::H, Fld::Hz].into_iter().find(|f| f.key() == *k) {
                    self.step_field(fld, *d > 0);
                }
            }
            _ => {}
        }
    }

    fn describe(&self) -> String {
        let f = self.f.map(|f| format!("{}x{}@{} {:?}", f.w, f.h, f.hz.hz(), f.sc)).unwrap_or_default();
        // (as last read: never waits for the service)
        let pending = self.pend;
        format!("mon={} fields={} keepbar={} presets={} rules={}", self.sel, f, pending, self.presets().len(), self.rules().len())
    }

    /// The reset line's items (reset.rs). Cheap: the runtime is made only when a line needs it.
    fn resettable(&mut self) -> Option<&mut dyn crate::undo::Resettable> {
        Some(self)
    }
}

impl Display {
    /// Shows the page on a runtime (tests hand it a fake one).
    fn open_rt(&mut self, rt: Arc<Rt>, env: &Env, now: f64) {
        self.now = now;
        self.frozen = env.frozen;
        self.rt = Some(rt.clone());
        self.memo = env.keep.clone();
        let (work, ans) = start_worker(rt.clone(), self.slow);
        self.work = Some(work);
        self.ans = Some(ans);
        // Order 047: the last read at once (QueryDisplayConfig, EDID, every monitor's mode list: 30 - 150 ms), the fresh
        // one from the worker - with a change still waiting for Keep (made before the menu was closed) its bar comes back
        if let Some(r) = env.keep.get::<Read>(KEEP_READ) {
            self.take_read(r, false);
            self.read_picture(self.sel);
        }
        self.read_again(How::Open);
        // rules made before the change log existed get their line ("none" before the app)
        reset::seed_rules(&rt);
        // test copies only: a state for the pixel proofs (the test hook's click can't reach El pages yet - PIECES_WANTED),
        // put once the open read is in
        if env.test {
            if let Some(s) = crate::testmode::env("BU_TEST_PAGE_STATE") {
                self.state_due = Some(s.strip_prefix("dsp:").unwrap_or("").to_string());
            }
        }
    }
}

impl Display {
    /// Test copies: put the page into one of the drawing's states (`keep` = after Apply, `mon2` = monitor 2 picked,
    /// `app<i>` / `pre<i>` / `vib<i>` = that rule's list open, `hz` = the Hz field focused with its list).
    fn test_state(&mut self, s: &str) {
        let sorted = |me: &Self| {
            let mut v: Vec<usize> = (0..me.mons.len()).collect();
            v.sort_by_key(|j| me.mons[*j].number);
            v
        };
        for part in s.split('+').filter(|p| !p.is_empty()) {
            self.test_state_one(part, &sorted);
        }
    }

    fn test_state_one(&mut self, s: &str, sorted: &dyn Fn(&Self) -> Vec<usize>) {
        match s {
            "keep" => self.apply(),
            "mon2" => {
                if let Some(i) = sorted(self).get(1).copied() {
                    self.select_monitor(i);
                    self.step = [None; 3];
                }
            }
            _ => {}
        }
    }

    fn click(&mut self, k: Key, cx: &mut Cx) {
        for i in 0..self.mons.len() {
            if k == idx(K_MON, i) {
                let mut sorted: Vec<usize> = (0..self.mons.len()).collect();
                sorted.sort_by_key(|j| self.mons[*j].number);
                self.select_monitor(sorted[i]);
                return;
            }
        }
        if k == K_ID {
            // nothing on the screen from any test copy (fake or --real-read) or unit test
            let test = self.rt().map(|r| r.fake).unwrap_or(true) || crate::testmode::on() || cfg!(test);
            identify::show(&self.mons, test);
            return;
        }
        if k == K_APPLY {
            cx.focus(None);
            self.apply();
            return;
        }
        if k == K_KEEP {
            self.keep();
            return;
        }
        if k == K_REVERT {
            self.revert();
            return;
        }
        for i in 0..3 {
            if k == idx(K_SCALE, i) {
                if let Some(f) = self.f.as_mut() {
                    f.sc = SCALES[i];
                }
                return;
            }
        }
        if k == K_MAIN {
            self.set_main();
            return;
        }
        if k == K_PNEW {
            self.save_preset();
            return;
        }
        if k == sub(K_RESET, "pc") || k == sub(K_RESET, "win") {
            // the frame's shared review over the app's change log (reset.rs), under the link
            let kind = if k == sub(K_RESET, "pc") { crate::undo::Kind::HowItWas } else { crate::undo::Kind::WindowsDefaults };
            let r = self.link_box.filter(|(lk, _)| *lk == k).map(|(_, r)| r).unwrap_or((150.0, 480.0, 150.0, 16.0));
            self.menu = None;
            cx.open_reset(kind, r);
            return;
        }
        for p in self.presets() {
            let pk = idx(K_PST, p.id.0 as usize);
            if k == sub(pk, "del") {
                self.pst_out = Some((p.id, self.now));
                return;
            }
            if k == pk {
                self.apply_preset(p.id);
                return;
            }
        }
        if let Some(m) = &self.menu {
            let kind = m.kind;
            // row j of the list (separators are rows too) -> its item
            for j in 0..self.menu_items(kind).len() {
                if k == idx(K_MENU, j) {
                    if let Some(i) = self.row_item(kind, j) {
                        if kind == MenuKind::Rates {
                            cx.focus(None);
                        }
                        self.pick(kind, i, cx);
                    }
                    return;
                }
            }
        }
        let rules = self.rules();
        for (i, r) in rules.iter().enumerate() {
            let rk = idx(K_RULE, i);
            if k == sub(rk, "tg") {
                let on = !r.enabled;
                self.change_rule(i, |x| x.enabled = on);
                return;
            }
            if k == sub(rk, "del") {
                self.rule_out = Some((r.id.0, self.now));
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests;
