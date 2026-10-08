//! The Controller tab (menu-v22 page `pad`, Order 020): Steam Input layouts per Steam game, wired to `bu-controller`
//! (Order 015). The v22 layout ("4 · Picture", the owner's v21 review): nothing picked = the controller BIG and centred with
//! "Click buttons to edit" between the handles; a click on a part = the picture glides LEFT and shrinks (FLIP, 520 ms) and
//! that part's settings slide in on the RIGHT (440 ms); a click on empty page space / the × = back. The Gyro chip under the
//! picture; the Light bar lives in the "Controller settings" popup (A_015_02: Steam keeps the light per controller, all
//! games). Every setting the panel shows writes the game's Steam layout through `bu-controller` (one write + one backup /
//! undo step per change, Steam re-reads it when the game window gets focus); the popup's settings write
//! `preferences_<serial>.vdf`. Live view (sticks, buttons, triggers) only while the page is open.
//!
//! Layouts 5 · Cards and 6 · Mix of the drawing are its drawing-only ideas (keys 5 / 6): not built (v22 picked 4).

mod data;
mod hist;
mod look;
mod panel;
mod pic;
mod work;

use std::collections::HashMap;

use bu_controller::binding::{key_label, key_token, KEYS};
use bu_controller::layout::{ActionSet, Layout, Press};
use bu_controller::live::{LiveState, LiveView};
use bu_controller::os::PadInfo;
use bu_controller::prefs::NOISE_STEPS;
use bu_controller::settings::{radius_to_pct, GyroView, StickView, TouchpadView, TriggerView};
use bu_controller::{
    Action, ButtonId, Change, Game, GyroMode, GyroSetting, MouseButton, PadButton, PadKind, PadView, Part, PrefSetting, Prefs, PressSetting, Side, SteamAction,
    StickMode, StickSetting, TouchMode, TouchSetting, TriggerSetting,
};
use taffy::style::{AlignItems, JustifyContent};

use crate::anim::{Bezier, EASE, EASE_OUT, EASE_OUT_CSS};
use crate::gfx::{sh, Font, Rgba};
use crate::pages::{Env, Page};
use crate::undo::Resettable;
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, lh, sub, Cursor, El, Key};
use crate::ui::pieces::ibtn;
use crate::ui::pieces::mbtn::{self, Mb};
use crate::ui::pieces::mitems::{self, It, Lead, Right, Row};
use crate::ui::pieces::{self, button, dialog, dropdown, link, reset, search, toast};
use crate::ui::{cmix, ACC, FG, FG2, FG3, HAIR, HOV, POP, WHITE, WIN_W};

use data::Svc;
use look::ActShow;
use work::{After, Job, Want, Wr};
use pic::{Pic, Pid};

const K_GAME: Key = key("pad.game");
const K_DEV: Key = key("pad.dev");
const K_SET: Key = key("pad.set");
const K_BODY: Key = key("pad.body");
const K_PANEL: Key = key("pad.panel");
const K_PIC: Key = key("pad.pic");
const K_PART: Key = key("pad.part");
const K_GYRO: Key = key("pad.gyro");
const K_STEAMSET: Key = key("pad.steamset");
const K_CTLSET: Key = key("pad.ctlset");
const K_OPENSTEAM: Key = key("pad.opensteam");
const K_RESET: Key = key("pad.reset");
const K_MENU: Key = key("pad.menu");
const K_ACT: Key = key("pad.act");
const K_ASRCH: Key = key("pad.act.search");
const K_DLG: Key = key("pad.dlg");
const K_REVIEW: Key = key("pad.review");
const K_TOAST: Key = key("pad.toast");
/// the "Please launch Steam" glass (it takes every click under it) and its button
const K_GATE: Key = key("pad.gate");
const K_LAUNCH: Key = key("pad.launch");
/// "Launch Steam" waits this long for Steam before it can be clicked again
const LAUNCH_WAIT_MS: f64 = 20000.0;
/// every panel / popup control: sub(K_C, "<id>")
const K_C: Key = key("pad.c");

/// The picture's widths (`.pdl4 .cpw{width:480px}` / `#sw:not(.tr2) .pdl4 .cpw{width:470px}`; `.pdl4.sel .cpw{width:188px}`).
const BIG_W: f32 = 470.0;
/// the owner (test build 1): a clicked part's settings open in a POPUP WINDOW (the shared readable `dialog`), big, scrolling,
/// titled with the part - the controller stays big and centred (it no longer shrinks to a 188 px side picture). The
/// window: 480 wide (the old side panel was 344), the dialog's 18 px padding each side = 444 inside, less the scroll
/// box's 9 px thumb lane (`SlimThumb::GLASS`; a classic scrollbar's lane comes out of the content) = 435.
const PANEL_W: f32 = 480.0;
const PANEL_IN: f32 = PANEL_W - 36.0 - 9.0;
/// the part window's content: its left edge in window coordinates (the dialog is centred in the 600 px window)
const PANEL_LEFT: f32 = (WIN_W - PANEL_W) / 2.0 + 18.0;
/// the panel's label column (`.pdl4.sel .cpn .prw>span:first-child{width:112px}`)
const LW: f32 = 112.0;
/// The Controller settings popup's width (`.dlg.pdvdlg{width:420px}` in the drawing; as wide as a part's window since its
/// rows end in the undo / reset slot, Order 042)
const DLG_W: f32 = 480.0;

/// Where a value is written.
#[derive(Clone, Copy, Debug, PartialEq)]
enum W {
    Btn(ButtonId, PressSetting),
    Stick(Side, StickSetting),
    StickMode(Side),
    Trig(Side, TriggerSetting),
    TrigAnalog(Side),
    Gyro(GyroSetting),
    GyroMode,
    Touch(TouchSetting),
    TouchMode,
    Pref(PrefSetting),
    Noise,
    Light,
}

/// Where an action is written.
#[derive(Clone, Copy, Debug, PartialEq)]
enum AW {
    Btn(ButtonId, Press),
    Ring(Side),
    Trig(Side, bool),
    TouchClick(Side),
}

/// How a slider's shown number becomes the file's value.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Conv {
    /// % shown, Steam's 0-32767 radius written
    Radius,
    /// the number itself
    Raw,
    /// shown / 100 (custom curve shape 1.92 = 192)
    Div100,
    /// 0..100 % shown, 0..1 written (light brightness)
    Unit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Fmt {
    Pct,
    Ms,
    Deg,
    Shape,
    FullPull,
}

/// What a control does (registered while building; looked up by its key on input).
#[derive(Clone, Debug)]
enum Ctl {
    Toggle { on: bool, w: W, on_v: Option<i64>, off_v: Option<i64> },
    /// a segment / swatch: `on` = it is the current one (a click on it writes nothing)
    Val { v: Option<i64>, w: W, on: bool },
    Slider { lo: f64, hi: f64, step: f64, def: f64, conv: Conv, fmt: Fmt, w: W, keep: (f64, f64) },
    Menu { items: Vec<(Option<i64>, String)>, cur: Option<i64>, w: W, width: f32 },
    Act { cur: Action, w: AW, title: String },
    /// the dead-zone circle of a stick: a drag moves the nearer ring (inner = dead zone, outer = full at)
    Well { inner: Key, outer: Key, at: (f64, f64) },
    /// shown, not changeable (dimmed, or a setting Steam's files don't have)
    Dead,
    /// a row's small reset: that value back to Steam's (Order 042 item 9)
    Back(Back),
    /// the "Undo" link in the row of the last changed control
    Undo,
    /// "Check stick drift" (start / again) of a stick
    DriftGo(Side),
    /// its "Use it": the suggested dead zone in %
    DriftUse(Side, f64),
}

/// What a row's reset writes: the values of Steam's layout for this game (one write), or Steam's defaults in the
/// controller's own file (dead zones "-1", anything else taken out).
#[derive(Clone, Debug, PartialEq)]
enum Back {
    Layout(Vec<Change>),
    Pref(Vec<(PrefSetting, Option<String>)>),
}

/// "Check stick drift" (Order 042 item 11): the stick is left alone for `DRIFT_MS`, its largest distance from the centre
/// is the drift; a dead zone just above it is suggested (nothing is written before "Use it").
#[derive(Clone, Copy, Debug, PartialEq)]
enum Drift {
    Run { side: Side, at: f64, max: f32 },
    Done { side: Side, max: f32, dz: f64 },
}

/// How long the stick is watched.
const DRIFT_MS: f64 = 5000.0;
/// The margin over the measured drift (whole %).
const DRIFT_MARGIN: f64 = 2.0;

/// The dead zone (whole %) suggested for a drift of `max` (0..1 of the stick's reach): just above it, inside the slider's
/// 1..60 % and under "full at" - 5.
fn drift_dz(max: f32, full_at: f64) -> f64 {
    let v = (max as f64 * 100.0 + DRIFT_MARGIN).ceil();
    v.clamp(1.0, 60.0f64.min(full_at - 5.0).max(1.0))
}

/// An open popup.
#[derive(Clone, Debug, PartialEq)]
enum Pop {
    Game,
    Dev,
    Set,
    Menu(Key),
    Act { at: Key, q: String, ki: usize },
    Review { win: bool, ticked: bool },
}

/// The live view's values (the drawing's `LV`): sticks -1..1, triggers 0..1, pressed parts.
#[derive(Clone, Debug, Default, PartialEq)]
struct Lv {
    ls: (f32, f32),
    rs: (f32, f32),
    l2: f32,
    r2: f32,
    dn: Vec<Pid>,
}

/// Everything the open page holds (dropped on close).
struct Open {
    /// the Steam service: made and used on the tab's worker (Order 047), locked here only by the change log and tests
    svc: data::Shared,
    /// a `--real-read` copy (the change log's service is made the same way)
    real_read: bool,
    /// Steam's folder was found (the worker's first answer; None = not known yet)
    svc_ok: Option<bool>,
    pads: Vec<PadInfo>,
    kind: PadKind,
    pic: Pic,
    games: Vec<Game>,
    gi: usize,
    set: u32,
    sets: Vec<ActionSet>,
    view: Option<PadView>,
    steam: Option<PadView>,
    /// the shown game's layout and Steam's own layout as last read: a change and another action set show at once from
    /// them (Order 047), the files' truth follows with the worker's answer
    lay: Option<Layout>,
    steam_lay: Option<Layout>,
    /// Steam's install folder (where `steam.exe` is), from the worker's first answer
    steam_dir: Option<std::path::PathBuf>,
    /// the worker's jobs and answers; the answers still out (jobs that read the view again / others)
    jobs: Option<std::sync::mpsc::Sender<Job>>,
    done: std::sync::mpsc::Receiver<work::Done>,
    out_state: usize,
    out_other: usize,
    /// its real values are in (the worker's first answer, or the state kept from the last opening)
    ready: bool,
    /// Ctrl+Z / Ctrl+Y (redo = true) pressed while a change was still being written: done when its answer is in, so the
    /// newest change is the one undone
    undo_q: Vec<bool>,
    prefs: Vec<Prefs>,
    note: Option<String>,
    sel: Option<Pid>,
    /// a part's window opened (its show motion)
    part_at: f64,
    /// another controller picked: the picture fades + scales in (opacity 0 -> 1, scale .97 -> 1, 280 ms, EASE_OUT)
    dev_at: Option<f64>,
    /// another part picked while the panel shows: its swap animation start
    swap_at: f64,
    pop: Option<Pop>,
    dlg: Option<f64>,
    anchors: HashMap<Key, (f32, f32, f32, f32)>,
    toast: Option<(String, f64)>,
    /// toasts not yet handed to the frame (`cx.toast`, in the next build)
    toast_q: Vec<String>,
    /// the live view's wake-up (a changed report repaints the open tab, the menu sleeps between them)
    waker: crate::services::Waker,
    /// the action list's first build after opening scrolls it to the current item
    act_fresh: bool,
    ctl: HashMap<Key, Ctl>,
    drag: Option<(Key, f64)>,
    /// a drag on a dead-zone circle: (the circle, the ring's slider)
    well_drag: Option<(Key, Key)>,
    pressed_part: Option<Pid>,
    /// the hover label hides after a click until the pointer moves to another part (`show(null)` on click)
    lab_off: Option<Pid>,
    live: Option<LiveView>,
    lv: Lv,
    t0: f64,
    fake: bool,
    frozen: bool,
    test: bool,
    /// the change log (Order 036): this event's changes on the PC (item, label, before, after), written by `event`
    rec_q: Vec<(String, String, crate::undo::Val, crate::undo::Val)>,
    /// the tab's undo / redo (Order 042 item 9), kept in `keep` while the tab is closed
    hist: hist::Hist,
    keep: crate::keep::Keep,
    /// the control (its row) + words of the change being made now (taken by `remember`)
    acting: Option<(Key, String)>,
    /// every control's row and label (built with the controls): control key -> (row key, "Left stick · Dead zone")
    rows: HashMap<Key, (Key, String)>,
    /// Steam's process (real copies; None = the fake's switch, or no Steam folder at all)
    steam_watch: Option<data::SteamWatch>,
    /// Steam runs: false = the "Please launch Steam" glass covers the tab (Order 042 item 5b)
    steam_up: bool,
    /// "Launch Steam" was clicked (its button waits for Steam)
    launch_at: Option<f64>,
    drift: Option<Drift>,
}

#[derive(Default)]
pub struct Controller {
    o: Option<Box<Open>>,
    /// a closed tab's Steam service for the change log (Settings › Reset, the uninstaller): made at its first use
    cold: data::Shared,
    /// a closed tab's worker answers still out (Order 047): the change-log entries of writes it was still making
    tail: Vec<std::sync::mpsc::Receiver<work::Done>>,
}

/// What the tab showed when it closed (Order 047: kept in the app's `Keep`, shown at once on the next opening while the
/// worker reads the files again).
#[derive(Clone)]
struct Snap {
    kind: PadKind,
    pads: Vec<PadInfo>,
    prefs: Vec<Prefs>,
    games: Vec<Game>,
    gi: usize,
    set: u32,
    sets: Vec<ActionSet>,
    lay: Option<Layout>,
    steam_lay: Option<Layout>,
    view: Option<PadView>,
    steam: Option<PadView>,
    note: Option<String>,
    steam_dir: Option<std::path::PathBuf>,
    steam_up: bool,
}

/// The `Keep` slot of `Snap`.
const SNAP: &str = "pad.snap";

// ------------------------------------------------------------------------------------------------ small helpers

fn sv<S: PartialEq + Copy>(list: &[(S, Option<i64>)], s: S) -> Option<i64> {
    list.iter().find(|(k, _)| *k == s).and_then(|(_, v)| *v)
}

fn pid_part(p: Pid) -> Option<Part> {
    Some(match p {
        Pid::B(b) => Part::Button(b),
        Pid::Stick(s) => Part::Stick(s),
        Pid::Trig(s) => Part::Trigger(s),
        Pid::Touch => Part::Touchpad,
        Pid::Gyro => Part::Gyro,
        Pid::Light | Pid::Fn(_) => return None,
    })
}

/// The glyph a pad button shows (`PADV`), PlayStation pads only.
fn pad_glyph(b: PadButton) -> Option<&'static str> {
    Some(match b {
        PadButton::Cross => "x",
        PadButton::Circle => "o",
        PadButton::Square => "sq",
        PadButton::Triangle => "tri",
        PadButton::DpadUp => "du",
        PadButton::DpadDown => "dd",
        PadButton::DpadLeft => "dl",
        PadButton::DpadRight => "dr",
        _ => return None,
    })
}

fn show(a: &Action, xbox: bool) -> ActShow {
    match a {
        Action::Nothing => ActShow::Nothing,
        Action::Key(k) => ActShow::Key(key_label(k)),
        Action::Pad(b) => ActShow::Pad(if xbox { None } else { pad_glyph(*b) }, a.label(xbox)),
        other => ActShow::Text(other.label(xbox)),
    }
}

/// The drawing's action list (`ACTS`): Controller, Mouse, Keyboard, Steam.
fn act_sections() -> Vec<(&'static str, Vec<Action>)> {
    use PadButton::*;
    let pad = [Cross, Circle, Square, Triangle, L1, R1, L2, R2, L3, R3, DpadUp, DpadDown, DpadLeft, DpadRight, Create, Options].into_iter().map(Action::Pad).collect();
    let mouse = vec![
        Action::Mouse(MouseButton::Left),
        Action::Mouse(MouseButton::Right),
        Action::Mouse(MouseButton::Middle),
        Action::WheelUp,
        Action::WheelDown,
        Action::Mouse(MouseButton::Back),
        Action::Mouse(MouseButton::Forward),
    ];
    let order = [
        "Space", "Enter", "Esc", "Tab", "Shift", "Ctrl", "Alt", "Backspace", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "Up", "Down",
        "Left", "Right", "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z", "1", "2", "3",
        "4", "5", "6", "7", "8", "9", "0", "Home", "End", "Page Up", "Page Down", "Insert", "Del",
    ];
    let keys = order.iter().filter_map(|l| key_token(l)).map(Action::Key).collect();
    let _ = KEYS;
    let steam = vec![Action::Steam(SteamAction::Screenshot), Action::Steam(SteamAction::ShowKeyboard), Action::light_bar_red()];
    vec![("Controller", pad), ("Mouse", mouse), ("Keyboard", keys), ("Steam", steam)]
}

/// The search words of an action (`ASRCH` + " key").
fn act_words(a: &Action) -> String {
    let extra = match a {
        Action::Pad(PadButton::Cross) => " x",
        Action::Pad(PadButton::Circle) => " o",
        Action::Pad(PadButton::Square) => " □",
        Action::Pad(PadButton::Triangle) => " △",
        Action::Mouse(MouseButton::Left) => " mouse 1 lmb",
        Action::Mouse(MouseButton::Right) => " mouse 2 rmb",
        Action::Mouse(MouseButton::Middle) => " mouse 3 wheel click",
        _ => "",
    };
    let k = if matches!(a, Action::Key(_)) { " key" } else { "" };
    format!("{}{}{}", a.label(false), extra, k).to_lowercase()
}

/// The game tile (`.gt`): the drawing's colours for its three games, else a calm colour from the name.
fn game_tile(name: &str) -> (String, Rgba, Rgba) {
    let init: String = {
        let w: Vec<&str> = name.split_whitespace().collect();
        let s = if w.len() >= 2 { format!("{}{}", w[0].chars().next().unwrap_or(' '), w[1].chars().next().unwrap_or(' ')) } else { name.chars().take(2).collect() };
        s.to_uppercase()
    };
    match name {
        "Rocket League" => ("RL".into(), Rgba::hex(0x2f8cff), Rgba::hex(0xff8a2a)),
        "Yakuza 0" => ("Y0".into(), Rgba::hex(0xd43c3c), Rgba::hex(0x5a1010)),
        "Epic Games Launcher" => ("EG".into(), Rgba::hex(0x4b4f5c), Rgba::hex(0x1f2128)),
        _ => {
            const PAL: [(u32, u32); 5] = [(0x5ab4ff, 0x2a5fd6), (0x9f8cff, 0x5a3fe0), (0x46d989, 0x1c9a5a), (0xffb86b, 0xe0661c), (0x6f8fb8, 0x2b3f5c)];
            let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32)) as usize % PAL.len();
            (init, Rgba::hex(PAL[h].0), Rgba::hex(PAL[h].1))
        }
    }
}

/// `.gt` 18 x 18 (header) / 16 x 16 (menu).
fn gt(init: &str, c1: Rgba, c2: Rgba, size: f32) -> El {
    let (r, fs) = if size >= 18.0 { (5.0, 8.0) } else { (4.0, 7.0) };
    El::block()
        .size(size, size)
        .none()
        .radius(r)
        .bg_linear(135.0, &[(0.0, c1), (1.0, c2)])
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.25))])
        .place_center()
        .child(El::text(init, Font::new(fs, 700).ls(-160), WHITE, fs))
}

fn pct(v: f64) -> String {
    format!("{} %", v.round() as i64)
}

impl Open {
    fn new(env: &Env, now: f64) -> Open {
        // Order 047: nothing slow here - the tab's worker makes the Steam service (the registry), lists the controllers
        // (one report per PlayStation pad for its battery), reads the files and starts the live view
        let svc: data::Shared = Default::default();
        let (jobs, done) = work::start(svc.clone(), env.fake(), env.waker());
        let kind = PadKind::DualSense;
        let mut o = Open {
            svc,
            real_read: env.real_read,
            svc_ok: None,
            pads: vec![],
            kind,
            pic: pic::pic(kind),
            games: vec![],
            gi: 0,
            set: 0,
            sets: vec![],
            view: None,
            steam: None,
            lay: None,
            steam_lay: None,
            steam_dir: None,
            jobs: Some(jobs),
            done,
            out_state: 0,
            out_other: 0,
            ready: false,
            undo_q: Vec::new(),
            prefs: vec![],
            note: None,
            sel: None,
            part_at: -1e9,
            swap_at: -1e9,
            dev_at: None,
            pop: None,
            dlg: None,
            anchors: HashMap::new(),
            toast: None,
            toast_q: Vec::new(),
            waker: env.waker(),
            act_fresh: false,
            ctl: HashMap::new(),
            drag: None,
            well_drag: None,
            pressed_part: None,
            lab_off: None,
            live: None,
            lv: Lv::default(),
            t0: now,
            fake: env.fake(),
            frozen: env.frozen,
            test: env.test,
            rec_q: Vec::new(),
            hist: env.keep.get::<hist::Hist>(hist::KEEP).unwrap_or_default(),
            keep: env.keep.clone(),
            acting: None,
            rows: HashMap::new(),
            steam_watch: None,
            steam_up: true,
            launch_at: None,
            drift: None,
        };
        // the last opening's state at once; the worker's answer fills in
        if let Some(s) = env.keep.get::<Snap>(SNAP) {
            o.restore(s);
        }
        // is Steam running: the real one watched while the tab is open (the watch's own thread looks at the process list;
        // no Steam folder at all: the note says so, no glass - the worker's first answer stops the watch); the fake's
        // switch comes with the worker's answers
        if !o.fake {
            o.steam_watch = Some(data::SteamWatch::start(o.waker, o.steam_up));
        }
        o.send(Job::Open { real_read: env.real_read, slow: data::test_slow() });
        if o.frozen {
            // the drawing without motion (RM): LV.ls = [.32, -.12]
            o.lv.ls = (0.32, -0.12);
        }
        o
    }

    /// The state kept from the last opening, shown at once (Order 047). The tab opens on the first game's first action set
    /// of the controller plugged in (as it always did): a kept state of another game / set / controller type shows only
    /// its lists, the view waits for the worker.
    fn restore(&mut self, s: Snap) {
        let kind = work::open_kind(&s.pads, &s.prefs);
        let first = s.sets.first().map(|x| x.id).unwrap_or(0);
        self.kind = kind;
        self.pic = pic::pic(kind);
        self.pads = s.pads;
        self.prefs = s.prefs;
        self.steam_dir = s.steam_dir;
        self.steam_up = s.steam_up;
        if s.kind != kind {
            return;
        }
        self.games = s.games;
        if s.gi == 0 && s.set == first {
            self.set = s.set;
            self.sets = s.sets;
            self.lay = s.lay;
            self.steam_lay = s.steam_lay;
            self.view = s.view;
            self.steam = s.steam;
            self.note = s.note;
            self.ready = true;
        }
    }

    /// What the tab shows now (kept when it closes).
    fn snap(&self) -> Snap {
        Snap {
            kind: self.kind,
            pads: self.pads.clone(),
            prefs: self.prefs.clone(),
            games: self.games.clone(),
            gi: self.gi,
            set: self.set,
            sets: self.sets.clone(),
            lay: self.lay.clone(),
            steam_lay: self.steam_lay.clone(),
            view: self.view.clone(),
            steam: self.steam.clone(),
            note: self.note.clone(),
            steam_dir: self.steam_dir.clone(),
            steam_up: self.steam_up,
        }
    }

    fn xbox(&self) -> bool {
        self.kind.is_xbox()
    }

    fn game(&self) -> Option<&Game> {
        self.games.get(self.gi)
    }

    // ---- the worker (Order 047): every Steam read / write and the controllers off the menu's thread

    /// Hand a job to the tab's worker (its answer comes back through `drain`, in the order asked).
    fn send(&mut self, j: Job) {
        let state = j.is_state();
        let Some(tx) = &self.jobs else { return };
        if tx.send(j).is_ok() {
            if state {
                self.out_state += 1;
            } else {
                self.out_other += 1;
            }
        }
    }

    /// Nothing asked of the worker is still out.
    fn idle(&self) -> bool {
        self.out_state == 0 && self.out_other == 0 && self.undo_q.is_empty()
    }

    /// The game / controller type / action set shown.
    fn want(&self) -> Want {
        Want { kind: self.kind, key: self.game().map(|g| g.key.clone()), set: Some(self.set), fresh: false }
    }

    /// Read the shown game's files again (on the worker).
    fn reload(&mut self) {
        let w = self.want();
        self.send(Job::Load(w));
    }

    /// The worker's answers that are in (nothing waiting = nothing done): true = something to show.
    fn drain(&mut self, now: f64) -> bool {
        let mut any = false;
        while let Ok(d) = self.done.try_recv() {
            any = true;
            if d.is_state() {
                self.out_state = self.out_state.saturating_sub(1);
            } else {
                self.out_other = self.out_other.saturating_sub(1);
            }
            self.take(d, now);
        }
        // an undo / redo pressed meanwhile: now that every change is written and its step kept (one at a time - each
        // one's write is waited for too)
        while self.out_state == 0 && !self.undo_q.is_empty() {
            let redo = self.undo_q.remove(0);
            self.undo(redo, now);
            any = true;
        }
        any
    }

    fn take(&mut self, d: work::Done, now: f64) {
        // a read is shown only when it is the newest one asked for: an older one, under a change still being written,
        // would show that change undone for a moment
        let last = self.out_state == 0;
        match d {
            work::Done::Opened { pads, steam_dir, svc_ok, running, loaded, live } => {
                self.pads = pads;
                self.steam_dir = steam_dir;
                self.svc_ok = Some(svc_ok);
                if !svc_ok {
                    // no Steam folder at all: the note says so, no glass
                    self.steam_watch = None;
                    self.steam_up = true;
                } else if self.fake {
                    self.steam_up = running;
                }
                self.live = live;
                if last {
                    if loaded.kind != self.kind {
                        // the outline = the controller plugged in
                        self.kind = loaded.kind;
                        self.pic = pic::pic(loaded.kind);
                        self.sel = None;
                    }
                    self.apply(loaded);
                }
                self.ready = true;
            }
            work::Done::Loaded(l) => {
                if last {
                    self.apply(l);
                }
            }
            work::Done::Wrote { r, recs, loaded, after } => {
                // the change log: written by the next build (`cx.record`)
                self.rec_q.extend(recs);
                match r {
                    Ok(id) => self.wrote(after, id, &loaded, now),
                    Err(e) => self.failed(after, e, now),
                }
                if last {
                    self.apply(loaded);
                }
            }
            work::Done::Live(l) => self.live = l,
            work::Done::Said(t) => {
                if let Some(t) = t {
                    self.say(t, now);
                }
            }
        }
    }

    /// Show what the worker read (the files' truth).
    fn apply(&mut self, l: work::Loaded) {
        if l.kind != self.kind {
            return; // read for a controller type no longer shown (a newer read is on its way)
        }
        self.games = l.games;
        // installed games first, the order Steam's index has them otherwise; the picked one kept
        self.gi = l.gi;
        self.set = l.set;
        if let Some(s) = l.sets {
            self.sets = s;
        }
        self.lay = l.lay;
        self.steam_lay = l.steam_lay;
        self.view = l.view;
        self.steam = l.steam;
        if let Some(p) = l.prefs {
            self.prefs = p;
        }
        self.note = l.note;
    }

    /// The view a read holds of this game on this controller type (the undo step of a write: before / after).
    fn read_view<'a>(l: &'a work::Loaded, game: &str, kind: PadKind) -> Option<&'a PadView> {
        if l.kind == kind && l.games.get(l.gi).is_some_and(|g| g.key == game) {
            l.view.as_ref()
        } else {
            None
        }
    }

    /// A write went through: its undo step (the view before it against the files read back after it) and its words.
    fn wrote(&mut self, after: After, id: u32, l: &work::Loaded, now: f64) {
        match after {
            After::Layout { before, parts, game, kind, set, acting, fallback, say } => {
                if let (Some(b), Some(a)) = (before, Self::read_view(l, &game, kind)) {
                    let (bc, ac) = hist::layout_diff(&b, a, &parts, kind);
                    self.acting = acting;
                    self.remember(hist::What::Layout { game, kind, set, before: bc, after: ac }, &fallback);
                }
                self.acting = None;
                self.say(say, now);
            }
            After::PartToSteam { before, part, game, kind, set, name } => {
                if let (Some(b), Some(a)) = (before, Self::read_view(l, &game, kind)) {
                    let (bc, ac) = hist::layout_diff(&b, a, &[part], kind);
                    self.acting = Some((K_STEAMSET, format!("{name} \u{b7} Steam\u{2019}s setting")));
                    self.remember(hist::What::Layout { game, kind, set, before: bc, after: ac }, &name);
                }
                self.say(format!("{name} · back to Steam\u{2019}s setting"), now);
            }
            After::NewSet { game, kind, from, title } => {
                self.acting = Some((K_SET, "New action set".into()));
                self.remember(hist::What::NewSet { game, kind, from, title: title.clone(), id }, "New action set");
                let first = l.sets.as_ref().and_then(|s| s.first()).map(|s| s.title.clone()).unwrap_or_else(|| "Default".into());
                self.say(format!("{title} made \u{b7} a copy of {first}"), now);
            }
            After::Prefs { before, label, acting } => {
                if let Some(a) = l.prefs.as_ref().and_then(|p| p.iter().find(|p| p.serial == before.serial)) {
                    let (b, a) = hist::prefs_diff(&before, a);
                    self.acting = acting;
                    self.remember(hist::What::Prefs { serial: before.serial.clone(), before: b, after: a }, &format!("Controller settings \u{b7} {label}"));
                }
                self.acting = None;
                self.say("Saved · this controller, every game", now);
            }
            After::Replay { mut step, redo } => {
                // a new action set made again gets a new id
                if redo {
                    if let hist::What::NewSet { id: sid, .. } = &mut step.what {
                        *sid = id;
                    }
                }
                self.say(format!("{} \u{b7} {}", if redo { "Redone" } else { "Undone" }, step.label), now);
                if redo {
                    self.hist.undo.push(step);
                } else {
                    self.hist.redo.push(step);
                }
            }
        }
    }

    /// A write did not go through: its words; an undo / redo step goes back to its list (the files read again show what
    /// is there).
    fn failed(&mut self, after: After, e: String, now: f64) {
        if let After::Replay { step, redo } = after {
            if redo {
                self.hist.redo.push(step);
            } else {
                self.hist.undo.push(step);
            }
        }
        self.acting = None;
        self.say(e, now);
    }

    /// Order 047: a change shows at once - made to the tab's own copy of the layout (the same edit bu-controller makes to
    /// the file on the worker); the files read back after the write follow. A change that doesn't fit changes nothing here.
    fn local(&mut self, changes: &[Change]) {
        let Some(l) = &self.lay else { return };
        let mut n = l.clone();
        for c in changes {
            if !c.fits(self.kind) || n.apply(self.set, c).is_err() {
                return;
            }
        }
        self.view = Some(n.pad_view(self.set, self.kind));
        self.sets = n.action_sets();
        self.lay = Some(n);
    }

    /// The same for the controller's own file (the values as the file will hold them; None = taken out).
    fn local_prefs(&mut self, serial: &str, vals: &[(PrefSetting, Option<String>)]) {
        if let Some(p) = self.prefs.iter_mut().find(|p| p.serial == serial) {
            for (s, v) in vals {
                if let Some(slot) = p.values.iter_mut().find(|x| x.0 == *s) {
                    slot.1 = v.clone();
                }
            }
        }
    }

    /// A new action set shown at once: the copy of `from` made in the tab's own copy of the layout.
    fn local_new_set(&mut self, from: u32, title: &str) {
        let Some(l) = &self.lay else { return };
        let mut n = l.clone();
        if let Ok(id) = n.add_action_set(from, title) {
            self.set = id;
            self.sets = n.action_sets();
            self.view = Some(n.pad_view(id, self.kind));
            self.steam = self.steam_lay.as_ref().map(|s| work::steam_view(s, id, self.kind));
            self.lay = Some(n);
        }
    }

    /// The tab's Steam service on this thread (the change log, tests): waits while the worker uses it; made here when the
    /// worker has not made it yet.
    fn svc_do<R>(&self, f: impl FnOnce(&mut Svc) -> R) -> Result<R, String> {
        let mut g = self.svc.lock().unwrap_or_else(|p| p.into_inner());
        if g.is_none() {
            *g = Some(data::open_steam(self.fake, self.real_read));
        }
        match &mut *g {
            Some(Ok(s)) => Ok(f(s)),
            Some(Err(e)) => Err(e.clone()),
            None => Err("No Steam".into()),
        }
    }

    /// Another controller type: its game list read again (on the worker), the picked game kept when it has a layout there.
    fn load_games(&mut self) {
        self.set = 0;
        self.clear_view();
        self.reload();
    }

    /// The shown view goes (another game / controller type is read): the panels wait for the worker's answer.
    fn clear_view(&mut self) {
        self.view = None;
        self.steam = None;
        self.lay = None;
        self.steam_lay = None;
    }

    /// Another action set: shown at once from the layouts read before; the files read again behind it.
    fn load_view(&mut self) {
        self.show_set();
        self.reload();
    }

    fn show_set(&mut self) {
        if let Some(l) = &self.lay {
            self.view = Some(l.pad_view(self.set, self.kind));
            self.steam = self.steam_lay.as_ref().map(|s| work::steam_view(s, self.set, self.kind));
        }
    }

    /// The live view of the shown controller type: started on the worker (opening the device can wait), the old one
    /// stopped there too.
    fn start_live(&mut self) {
        let old = self.live.take();
        if self.fake {
            return; // the drawing's made-up 6-second loop (tick)
        }
        let pad = self.pads.iter().find(|p| p.kind == self.kind).cloned();
        self.send(Job::Live { pad, old });
    }

    fn connected(&self) -> Option<&PadInfo> {
        self.pads.iter().find(|p| p.kind == self.kind)
    }

    /// The per-controller file of the shown controller (by Steam's name for it).
    fn pref(&self) -> Option<&Prefs> {
        let want = match self.kind {
            PadKind::DualSenseEdge => "Edge",
            PadKind::DualSense => "DualSense",
            PadKind::DualShock4 => "PlayStation 4",
            PadKind::Xbox => "Xbox",
        };
        self.prefs.iter().find(|p| p.name.contains(want) && (self.kind != PadKind::DualSense || !p.name.contains("Edge"))).or_else(|| self.prefs.first())
    }

    fn say(&mut self, t: impl Into<String>, now: f64) {
        let t = t.into();
        self.toast_q.push(t.clone());
        self.toast = Some((t, now));
    }

    /// Write changes to the game's layout (one write, one undo step of the crate and of the tab): shown at once, written
    /// and read back on the worker (Order 047); the undo step and "Saved" come with its answer.
    fn write(&mut self, changes: Vec<Change>, _now: f64) {
        let Some(g) = self.game().cloned() else { return };
        let (before, kind, set) = (self.view.clone(), self.kind, self.set);
        let fallback = self.sel.map(|p| self.pname(p)).unwrap_or_else(|| "Controller".into());
        let after = After::Layout {
            before,
            parts: hist::parts_of(&changes),
            game: g.key.clone(),
            kind,
            set,
            acting: self.acting.take(),
            fallback,
            say: format!("Saved · {} gets it when you click back into the game", g.name),
        };
        self.local(&changes);
        let want = self.want();
        self.send(Job::Write { wr: Wr::Layout { key: g.key, kind, set, changes, gone: false }, want, after });
    }

    /// Write values of the shown controller's own file (one write, one undo step of the crate and of the tab).
    fn write_pref(&mut self, vals: &[(PrefSetting, Option<String>)], label: &str, now: f64) {
        let Some(before) = self.pref().cloned() else {
            self.acting = None;
            self.say("Steam has no settings file for this controller yet", now);
            return;
        };
        let serial = before.serial.clone();
        let after = After::Prefs { before, label: label.to_string(), acting: self.acting.take() };
        self.local_prefs(&serial, vals);
        let want = self.want();
        self.send(Job::Write { wr: Wr::Prefs { serial, vals: vals.to_vec(), label: label.to_string() }, want, after });
    }

    /// The layout change a value of a control makes (None: not a layout value, or a stick mode the list doesn't have -
    /// kept as Steam wrote it, never rewritten).
    fn layout_change(w: W, v: Option<i64>) -> Option<Change> {
        Some(match w {
            W::Btn(b, s) => Change::ButtonSetting { button: b, setting: s, value: v },
            W::Stick(side, s) => Change::StickSetting { side, setting: s, value: v },
            W::StickMode(side) => Change::StickMode { side, mode: v.and_then(|i| StickMode::LISTED.get(i as usize)).cloned()? },
            W::Trig(side, s) => Change::TriggerSetting { side, setting: s, value: v },
            W::TrigAnalog(side) => Change::TriggerAnalog { side, analog: v == Some(0) },
            W::Gyro(s) => Change::GyroSetting { setting: s, value: v },
            W::GyroMode => Change::GyroMode {
                mode: match v {
                    Some(1) => GyroMode::Mouse,
                    Some(2) => GyroMode::Joystick,
                    Some(3) => GyroMode::Camera,
                    _ => GyroMode::Off,
                },
            },
            W::Touch(s) => Change::TouchSetting { setting: s, value: v },
            W::TouchMode => Change::TouchMode {
                mode: match v {
                    Some(1) => TouchMode::Mouse,
                    Some(2) => TouchMode::Scroll,
                    _ => TouchMode::Nothing,
                },
            },
            W::Pref(_) | W::Noise | W::Light => return None,
        })
    }

    /// One value from a control.
    fn put(&mut self, w: W, v: Option<i64>, now: f64) {
        match w {
            W::Pref(p) => {
                let text = match p {
                    PrefSetting::LeftStickDeadZone | PrefSetting::RightStickDeadZone => v.map(|r| r.to_string()).or(Some("-1".into())),
                    PrefSetting::LedBrightness => v.map(|x| format!("{}", x as f64 / 100.0)),
                    _ => v.map(|x| x.to_string()),
                };
                self.write_pref(&[(p, text)], p.label(), now);
            }
            W::Noise => {
                let s = NOISE_STEPS.get(v.unwrap_or(1) as usize).map(|(_, n)| n.to_string());
                self.write_pref(&[(PrefSetting::GyroNoiseFilter, s)], "Gyro noise filter", now);
            }
            W::Light => {
                // the light colour: `color_red/green/blue` of the controller's file (what bu-controller's set_light_bar writes)
                let (r, g, b) = look::LCOL.get(v.unwrap_or(0) as usize).and_then(|c| c.1).unwrap_or((0, 0, 0));
                let vals = [(PrefSetting::LedRed, Some(r.to_string())), (PrefSetting::LedGreen, Some(g.to_string())), (PrefSetting::LedBlue, Some(b.to_string()))];
                self.write_pref(&vals, "light bar colour", now);
            }
            _ => match Self::layout_change(w, v) {
                Some(c) => self.write(vec![c], now),
                None => self.acting = None,
            },
        }
    }

    fn act_change(w: AW, a: Action) -> Change {
        match w {
            AW::Btn(b, p) => Change::ButtonAction { button: b, press: p, action: a },
            AW::Ring(s) => Change::StickRing { side: s, action: a },
            AW::Trig(s, soft) => Change::TriggerAction { side: s, soft, action: a },
            AW::TouchClick(s) => Change::TouchClick { half: s, action: a },
        }
    }

    fn put_action(&mut self, w: AW, a: Action, now: f64) {
        self.write(vec![Self::act_change(w, a)], now);
    }

    /// Steam's own value of a control, as the control reads it (`Some(None)` = not in Steam's file = its default; None =
    /// unknown - no Steam layout here, or a value without a Steam default such as the light colour). For the controller's
    /// own file Steam's default is "not in the file".
    fn steam_raw(&self, w: W) -> Option<Option<i64>> {
        match w {
            W::Pref(_) => Some(None),
            // nothing in the file = Steam's 0.5 = Medium, the list's item 1
            W::Noise => Some(Some(1)),
            W::Light => None,
            _ => raw_in(self.steam.as_ref()?, w),
        }
    }

    /// What a row's reset writes for Steam's value `v`.
    fn back_of(w: W, v: Option<i64>) -> Option<Back> {
        match w {
            W::Pref(p @ (PrefSetting::LeftStickDeadZone | PrefSetting::RightStickDeadZone)) => Some(Back::Pref(vec![(p, Some("-1".into()))])),
            W::Pref(p) => Some(Back::Pref(vec![(p, None)])),
            W::Noise => Some(Back::Pref(vec![(PrefSetting::GyroNoiseFilter, None)])),
            W::Light => None,
            _ => Self::layout_change(w, v).map(|c| Back::Layout(vec![c])),
        }
    }

    /// Steam's own action of a "does" chip (None = no Steam layout here).
    fn steam_act(&self, w: AW) -> Option<Action> {
        let v = self.steam.as_ref()?;
        Some(match w {
            AW::Btn(b, p) => v.buttons.iter().find(|x| x.id == b)?.presses.iter().find(|(q, _)| *q == p).map(|(_, a)| a.clone()).unwrap_or(Action::Nothing),
            AW::Ring(s) => v.sticks.iter().find(|x| x.side == s)?.ring_action.clone(),
            AW::Trig(s, soft) => {
                let t = v.triggers.iter().find(|x| x.side == s)?;
                if soft {
                    t.soft_pull.clone()
                } else {
                    t.click.clone()
                }
            }
            AW::TouchClick(s) => {
                let t = v.touchpad.as_ref()?;
                if s == Side::Left {
                    t.left_click.clone()
                } else {
                    t.right_click.clone()
                }
            }
        })
    }

    fn pick(&mut self, id: Option<Pid>, now: f64) {
        let was = self.sel;
        self.pop = None;
        if id == Some(Pid::Light) {
            // A_015_02: the light bar's colour is per controller = the Controller settings popup (only while it is plugged in)
            if self.connected().is_some() {
                self.dlg = Some(now);
            } else {
                self.say(NO_PAD_TIP, now);
            }
            return;
        }
        self.sel = id;
        if id != was {
            // a drift check belongs to its stick's window
            self.drift = None;
        }
        if id.is_some() && was.is_some() && id != was {
            self.swap_at = now;
        }
        if id.is_some() && was.is_none() {
            // the part's window opens (the dialog's own show motion)
            self.part_at = now;
        }
    }

    /// Every frame step while the tab shows: the worker's answers, the toast, Steam started / ended, the live view, the
    /// drift check. True only when something shown changed (Order 047: nothing moving = no frames).
    fn tick(&mut self, now: f64) -> bool {
        let answers = self.drain(now);
        if let Some((_, at)) = &self.toast {
            if now - at >= toast::SHOW_MS + 400.0 {
                self.toast = None;
            }
        }
        let steam = self.steam_tick();
        let live = self.live_tick(now);
        let drift = self.drift_tick(now);
        // "Starting Steam…" (at most LAUNCH_WAIT_MS after the click): its end repaints (Order 042 review) - the menu sleeps
        // until then (`wake_at`; Order 047: no frames while it waits)
        let launching = match self.launch_at {
            Some(a) if now - a >= LAUNCH_WAIT_MS => {
                self.launch_at = None;
                true
            }
            _ => false,
        };
        answers || steam || live || drift || launching
    }

    /// Steam started or ended (the watch's thread woke the menu; the fake's switch in tests): the glass comes / goes.
    /// Nothing is read here: the watch keeps its answer, the fake's switch is looked at only while the worker doesn't hold
    /// the service.
    fn steam_tick(&mut self) -> bool {
        // not known before the worker's first answer (no Steam folder = no glass)
        if self.svc_ok != Some(true) {
            return false;
        }
        let up = match &self.steam_watch {
            Some(w) => w.up(),
            None if self.fake => match self.fake_running() {
                Some(up) => up,
                None => return false,
            },
            None => return false,
        };
        if up == self.steam_up {
            return false;
        }
        self.steam_up = up;
        if up {
            // Steam may have written its files while it started: read them again - on the worker (Order 047: this read on
            // the menu's thread was the black box after "Launch Steam"), the names too
            self.launch_at = None;
            let mut w = self.want();
            w.fresh = true;
            self.send(Job::Load(w));
        } else {
            // nothing stays open under the glass
            self.sel = None;
            self.pop = None;
            self.dlg = None;
            self.drift = None;
            self.drag = None;
            self.well_drag = None;
        }
        true
    }

    /// The fake Steam's process switch (test copies), looked at only while the worker doesn't hold the service (None).
    fn fake_running(&self) -> Option<bool> {
        let g = self.svc.try_lock().ok()?;
        match &*g {
            Some(Ok(s)) => Some(s.steam_running()),
            Some(Err(_)) => Some(true),
            None => None,
        }
    }

    /// "Launch Steam" (the glass's button): Steam's own `steam.exe` through Windows' shell, only on this click; a test
    /// copy starts nothing.
    fn launch_steam(&mut self, now: f64) {
        if self.launch_at.is_some_and(|a| now - a < LAUNCH_WAIT_MS) {
            return;
        }
        self.launch_at = Some(now);
        // Steam's folder: from the worker's first answer (none = no Steam)
        let Some(dir) = &self.steam_dir else { return };
        let exe = dir.join("steam.exe");
        if self.test {
            self.say("Starts Steam (a test copy starts nothing)", now);
        } else {
            shell_open(&exe.to_string_lossy());
        }
    }

    /// The drift check: the picked stick's largest distance from the centre while it runs; after `DRIFT_MS` the result.
    fn drift_tick(&mut self, now: f64) -> bool {
        let Some(Drift::Run { side, at, max }) = self.drift else { return false };
        let p = if side == Side::Left { self.lv.ls } else { self.lv.rs };
        let max = max.max(p.0.hypot(p.1));
        self.drift = Some(if now - at >= DRIFT_MS {
            let full = self.view.as_ref().and_then(|v| v.sticks.iter().find(|s| s.side == side)).map(|s| stick_dz(s).1).unwrap_or(100.0);
            Drift::Done { side, max, dz: drift_dz(max, full) }
        } else {
            Drift::Run { side, at, max }
        });
        true
    }

    /// The drawing's live loop (a made-up 6 s of hands) for the fake controller; the real controller's state otherwise.
    fn live_tick(&mut self, now: f64) -> bool {
        if self.frozen {
            return false;
        }
        if self.fake {
            if self.kind != PadKind::DualSenseEdge {
                self.lv = Lv::default();
                return false;
            }
            let a = (now - self.t0) / 1000.0;
            let s = a % 6.0;
            let m = 0.55 + 0.42 * (a * 0.9).sin();
            let mut ls = (((a * 1.15).cos() * m) as f32, ((a * 1.15).sin() * m * 0.9) as f32);
            let n = if s > 3.9 && s < 5.2 { 0.75 } else { 0.12 };
            let mut rs = (((a * 2.1).sin() * n) as f32, ((a * 1.6).cos() * n * 0.6) as f32);
            // the drift check on the fake: the stick is "left alone" and wobbles a little off the centre (at most ~2.9 %)
            if let Some(Drift::Run { side, .. }) = self.drift {
                let d = (((a * 2.7).sin() * 0.017 + 0.006) as f32, ((a * 1.9).cos() * 0.014 - 0.004) as f32);
                if side == Side::Left {
                    ls = d;
                } else {
                    rs = d;
                }
            }
            let r2 = if s < 1.0 {
                0.0
            } else if s < 1.35 {
                (s - 1.0) / 0.35
            } else if s < 2.0 {
                1.0
            } else if s < 2.3 {
                1.0 - (s - 2.0) / 0.3
            } else {
                0.0
            };
            let l2 = if s > 4.1 && s < 4.7 { ((s - 4.1) / 0.6 * std::f64::consts::PI).sin() * 0.6 } else { 0.0 };
            let presses = [(Pid::B(ButtonId::Cross), 2.55, 2.8), (Pid::B(ButtonId::Square), 3.35, 3.55), (Pid::B(ButtonId::R1), 4.95, 5.2), (Pid::B(ButtonId::DpadUp), 5.55, 5.75), (Pid::B(ButtonId::L3), 1.2, 1.4)];
            let dn = presses.iter().filter(|p| s >= p.1 && s < p.2).map(|p| p.0).collect();
            self.lv = Lv { ls, rs, l2: l2 as f32, r2: r2 as f32, dn };
            return true;
        }
        let Some(v) = &self.live else { return false };
        if v.is_gone() {
            self.live = None;
            self.lv = Lv::default();
            return true;
        }
        // a changed report woke the menu (env.waker): repaint the live parts once; nothing changed = no frame
        match v.latest().map(|st| lv_of(&st)) {
            Some(lv) if lv != self.lv => {
                self.lv = lv;
                true
            }
            _ => false,
        }
    }
}

fn lv_of(s: &LiveState) -> Lv {
    let mut dn: Vec<Pid> = s.pressed.iter().map(|b| Pid::B(*b)).collect();
    if s.touchpad {
        dn.push(Pid::Touch);
    }
    Lv { ls: s.left, rs: s.right, l2: s.l2, r2: s.r2, dn }
}

impl Page for Controller {
    fn id(&self) -> &'static str {
        "pad"
    }
    fn name(&self) -> &'static str {
        "Controller"
    }
    fn icon(&self) -> &'static str {
        "pad"
    }
    fn open(&mut self, env: &Env, now: f64) {
        // opened again while still open (a tab still sliding out shown again): closed first, so its writes' entries and
        // its undo list are kept
        if self.o.is_some() {
            self.close();
        }
        self.drain_tail();
        self.o = Some(Box::new(Open::new(env, now)));
    }
    fn close(&mut self) {
        // drops the services, the live view's thread + device, the Steam watch's thread, every cached value; the undo /
        // redo lists and what the tab showed wait in the app's keep for the next opening; the worker ends after the
        // writes still queued (Order 047: their change-log entries are taken at the next opening / reset)
        if let Some(o) = self.o.take() {
            o.keep.put(hist::KEEP, o.hist.clone());
            // (a tab closed before its first answer has nothing to show next time)
            if o.ready {
                o.keep.put(SNAP, o.snap());
            }
            if !o.idle() {
                let o = *o;
                self.tail.push(o.done);
            }
        }
    }
    fn ready(&self) -> bool {
        self.o.as_ref().is_none_or(|o| o.ready)
    }
    fn tick(&mut self, now: f64) -> bool {
        self.o.as_mut().map(|o| o.tick(now)).unwrap_or(false)
    }
    fn wake_at(&self, _now: f64) -> Option<f64> {
        let o = self.o.as_ref()?;
        // "Starting Steam…" ends; the page's toast goes (the worker's answers wake the menu themselves)
        let launch = o.launch_at.map(|a| a + LAUNCH_WAIT_MS);
        let toast = o.toast.as_ref().map(|(_, at)| at + toast::SHOW_MS + 400.0);
        match (launch, toast) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        if self.o.is_none() {
            // built before open (layout tests): the drawing's header only
            self.o = Some(Box::new(Open::new(&Env { test: true, ..Env::default() }, cx.now)));
        }
        let o = self.o.as_mut().unwrap();
        o.ctl.clear();
        o.rows.clear();
        o.build(cx)
    }
    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        if let Some(o) = self.o.as_mut() {
            o.event(ev, cx);
            // the change log: what this event changed on the PC
            for (i, l, old, new) in std::mem::take(&mut o.rec_q) {
                cx.record(&i, &l, &old, &new);
            }
        }
    }
    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        self.o.as_mut().and_then(|o| o.popup(cx))
    }
    fn popup_dismiss(&mut self) {
        if let Some(o) = self.o.as_mut() {
            if o.pop.is_some() {
                o.pop = None;
            } else if o.dlg.is_some() {
                o.dlg = None;
            } else if o.sel.is_some() {
                o.sel = None;
            } else {
                o.toast = None;
            }
        }
    }
    fn describe(&self) -> String {
        let Some(o) = &self.o else { return String::new() };
        format!(
            "game={} dev={} set={} sel={} pop={} dlg={} writes={}",
            o.game().map(|g| g.name.as_str()).unwrap_or("-").replace(' ', "_"),
            o.kind.name().replace(' ', "_"),
            o.set,
            o.sel.map(|p| p.name()).unwrap_or_else(|| "-".into()),
            o.pop.as_ref().map(|p| format!("{p:?}").split([' ', '(', '{']).next().unwrap_or("").to_string()).unwrap_or_else(|| "-".into()),
            o.dlg.is_some(),
            o.svc_do(|s| s.fake_writes().len()).unwrap_or(0)
        )
    }
    fn resettable(&mut self) -> Option<&mut dyn crate::undo::Resettable> {
        self.drain_tail();
        Some(self)
    }
}

impl Controller {
    /// The tab's reset as a copy that works on any thread (Order 047: the Reset review reads and puts back off the menu's
    /// thread): the Steam service the change log works through - the open tab's own (shared with its worker; the open tab
    /// reads its view again when the reset ended, `reset_done`), else a closed tab's made at its first use (the fake in
    /// test copies and unit tests, read-only in a `--real-read` copy) - and the game the open tab shows. Reads nothing.
    fn away(&self) -> Away {
        match &self.o {
            Some(o) => Away {
                svc: o.svc.clone(),
                fake: o.fake,
                real_read: o.real_read,
                shown: o.game().cloned().map(|g| (o.kind, g)),
                read_only: crate::testmode::real_read() || (o.test && !o.fake),
            },
            None => Away {
                svc: self.cold.clone(),
                fake: crate::testmode::on() || cfg!(test),
                real_read: crate::testmode::real_read(),
                shown: None,
                read_only: crate::testmode::real_read(),
            },
        }
    }

    /// A closed tab's worker answers that came in since (Order 047): the change-log entries of the writes it was still
    /// making when the tab closed.
    fn drain_tail(&mut self) {
        let mut recs = Vec::new();
        // every closed opening's worker, oldest first (one that has ended and was read to its end is let go)
        self.tail.retain(|rx| loop {
            match rx.try_recv() {
                Ok(work::Done::Wrote { recs: r, .. }) => recs.extend(r),
                Ok(_) => {}
                Err(std::sync::mpsc::TryRecvError::Empty) => break true,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break false,
            }
        });
        for (i, l, old, new) in recs {
            crate::services::try_with(|s| crate::undo::record(&mut s.store, data::PAGE, &i, &l, &old, &new));
        }
    }
}

/// The change log's side of the Controller tab (Order 036): every game's layout file the app wrote ("Back to how your PC
/// was" = the bytes from before the app's first change, kept by bu-controller in the app's folder), every controller's
/// settings file; "Steam’s layout" = the tab's Windows defaults. Order 047: the same answers from `Away` - here on the
/// caller's thread, or on the review's worker through `detach`.
impl crate::undo::Resettable for Controller {
    fn page_id(&self) -> &str {
        data::PAGE
    }
    fn page_title(&self) -> &str {
        "Controller"
    }
    fn current(&self, item: &str) -> Option<crate::undo::Val> {
        self.away().current(item)
    }
    fn defaults_name(&self) -> Option<&str> {
        Some("Steam\u{2019}s layout")
    }
    fn windows_defaults(&self) -> Vec<crate::undo::DefaultItem> {
        self.away().windows_defaults()
    }
    fn apply(&mut self, item: &str, to: &crate::undo::Val) -> Result<(), String> {
        let r = self.away().apply(item, to);
        // the open tab shows the files as they are now (read again on its worker)
        self.reset_done();
        r
    }
    fn detach(&mut self) -> Option<crate::undo::Detached> {
        Some(Box::new(self.away()))
    }
    fn reset_done(&mut self) {
        if let Some(o) = self.o.as_mut() {
            o.reload();
        }
    }
}

/// The Controller's reset as a Send copy (`Controller::away`): its Steam service handle, the shown game.
struct Away {
    svc: data::Shared,
    fake: bool,
    real_read: bool,
    shown: Option<(PadKind, Game)>,
    read_only: bool,
}

impl Away {
    /// The service (made here at its first use; waits while the open tab's worker uses it).
    fn with<R>(&self, f: impl FnOnce(&mut Svc) -> R) -> Result<R, String> {
        let mut g = self.svc.lock().unwrap_or_else(|p| p.into_inner());
        if g.is_none() {
            *g = Some(data::open_steam(self.fake, self.real_read));
        }
        match &mut *g {
            Some(Ok(s)) => Ok(f(s)),
            Some(Err(e)) => Err(e.clone()),
            None => Err("No Steam".into()),
        }
    }
}

impl crate::undo::Resettable for Away {
    fn page_id(&self) -> &str {
        data::PAGE
    }
    fn page_title(&self) -> &str {
        "Controller"
    }
    fn current(&self, item: &str) -> Option<crate::undo::Val> {
        let it = data::Item::parse(item)?;
        self.with(|s| s.log_val(&it)).ok().flatten()
    }
    fn defaults_name(&self) -> Option<&str> {
        Some("Steam\u{2019}s layout")
    }
    fn windows_defaults(&self) -> Vec<crate::undo::DefaultItem> {
        let shown = self.shown.as_ref().map(|(k, g)| (*k, g));
        self.with(|s| s.steam_defaults(shown)).unwrap_or_default()
    }
    fn apply(&mut self, item: &str, to: &crate::undo::Val) -> Result<(), String> {
        if self.read_only {
            return Err("A read-only test copy changes nothing".into());
        }
        let it = data::Item::parse(item).ok_or("Unknown setting")?;
        self.with(|s| s.restore(&it, &to.raw)).and_then(|r| r)
    }
}

impl Open {
    // ============================================================================================ build
    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        for t in std::mem::take(&mut self.toast_q) {
            cx.toast(&t);
        }
        // the change log of the writes the worker made since (Order 047: they land in `tick`, after their click)
        for (i, l, old, new) in std::mem::take(&mut self.rec_q) {
            cx.record(&i, &l, &old, &new);
        }
        let head = self.header(cx);
        let rest = vec![self.pdq(cx), self.body(cx), reset::reset_line(cx, K_RESET, Some("Steam\u{2019}s layout"))];
        if self.steam_up {
            let mut v = vec![head];
            v.extend(rest);
            return v;
        }
        // Steam closed: the glass lies over everything under the header
        vec![head, El::block().child(El::col().items(AlignItems::STRETCH).children(rest)).child(self.steam_gate(cx))]
    }

    /// "Please launch Steam" (Order 042 item 5b): a glass over the controller area that takes every click under it, with
    /// the app's popup-window card in its middle; it goes away by itself when Steam runs (`steam_tick`).
    fn steam_gate(&mut self, cx: &mut Cx) -> El {
        let waiting = self.launch_at.is_some_and(|a| cx.now - a < LAUNCH_WAIT_MS);
        let title = El::text("Please launch Steam", Font::display(15.0, 600).ls(-150), FG(), 20.0).none();
        let text = El::text("The controller settings are Steam\u{2019}s own: they can be changed while Steam runs.", Font::new(12.0, 400), FG2(), 16.0)
            .wrapping()
            .align(crate::gfx::Align::Center)
            .margin(6.0, 0.0, 16.0, 0.0);
        let label = if waiting { "Starting Steam\u{2026}" } else { "Launch Steam" };
        let btn = button::cbtn_sized(cx, K_LAUNCH, label, button::Kind::Primary, button::DFT, waiting, 0.0);
        // the popup window's look (`dialog`): its fill, blur, rim and shadow
        let card = El::col()
            .items(AlignItems::CENTER)
            .w(300.0)
            .pad(20.0, 22.0, 18.0, 22.0)
            .radius(14.0)
            .bg(dialog::dlg_bg())
            .backdrop(30.0, 1.8)
            .shadow(&dialog::dlg_shadow())
            .inset(&dialog::dlg_inset())
            .key(sub(K_GATE, "card"))
            .on_click(sub(K_GATE, "card"))
            .child(El::icon("pad", 26.0, 1.4, FG2()).margin(0.0, 0.0, 10.0, 0.0))
            .child(title)
            .child(text)
            .child(btn);
        // the glass: the page under it stepped back (the popup window's dim), every click taken
        El::grid()
            .abs(-8.0, -6.0, -8.0, -6.0)
            .place_center()
            .radius(12.0)
            .bg(dialog::scrim(0.45))
            .z(5)
            .key(K_GATE)
            .on_click(K_GATE)
            .child(card)
    }

    /// `.ph.pdh` + `.pdpk`: the game picker and the controller picker (with its battery).
    fn header(&mut self, cx: &mut Cx) -> El {
        let (name, tile) = match self.game() {
            Some(g) => (g.name.clone(), game_tile(&g.name)),
            None => ("No Steam game".to_string(), ("–".to_string(), Rgba::hex(0x4b4f5c), Rgba::hex(0x1f2128))),
        };
        // .pu.pdg .gt{width:18px;height:18px;border-radius:5px;font:700 8px/1;letter-spacing:-.02em;box-shadow:inset 0 0 0 .5px rgba(255,255,255,.25)}
        let gt_el = gt(&tile.0, tile.1, tile.2, 18.0);
        let name_el = El::text(name, pieces::btn_font(13.0, 400), FG(), lh(13.0, 1.35)).none();
        let game = dropdown::dropdown_with(cx, K_GAME, vec![gt_el, name_el], 4.0, 7.0);
        // .pu.pdc .bat{gap:4px;font-size:12px;color:var(--fg2)} .bat svg{width:19px;height:11px} - only while it is plugged in
        let mut dev_kids = vec![El::text(self.kind.name(), pieces::btn_font(13.0, 400), FG(), lh(13.0, 1.35)).none()];
        if let Some((b, bt)) = self.connected().and_then(|p| p.battery).and_then(|bt| bt.percent.map(|b| (b, bt))) {
            // Order 045: `'data-tip':'Battery 82 % · charging over USB'` (the "charging over USB" part only while it charges on the cable)
            let tip = if bt.charging && bt.wired { format!("Battery {b} % \u{b7} charging over USB") } else { format!("Battery {b} %") };
            dev_kids.push(
                El::row()
                    .center()
                    .gap(4.0)
                    .none()
                    .key(sub(K_DEV, "bat"))
                    .tip(&tip)
                    .child(El::icon("bat", 19.0, 1.2, FG2()).h(11.0))
                    .child(El::text(format!("{b} %"), pieces::btn_font(12.0, 400).tnum(), FG2(), lh(12.0, 1.35)).none()),
            );
        }
        let dev = dropdown::dropdown_with(cx, K_DEV, dev_kids, 10.0, 7.0);
        // .pdpk{display:flex;align-items:center;gap:8px}
        let mut pk = El::row().center().gap(8.0).none().child(game).child(dev);
        if !self.steam_up {
            // Steam closed: the pickers wait too (dimmed; a cover takes their clicks)
            pk = El::block().none().child(pk.opacity(0.4)).child(El::block().abs(0.0, 0.0, 0.0, 0.0).key(K_GATE).on_click(K_GATE).z(1));
        }
        pieces::header_mb("Controller", Some(pk), 2.0)
    }

    /// `.pdq{display:flex;align-items:center;gap:8px;margin:0 0 6px}`: the action set, one line, Live.
    fn pdq(&mut self, cx: &mut Cx) -> El {
        let set_name = self.sets.iter().find(|s| s.id == self.set).map(|s| s.title.clone()).unwrap_or_else(|| "Default".into());
        let text = match (&self.note, self.game()) {
            (Some(n), _) => n.clone(),
            (None, Some(g)) if g.is_community() => "A community layout · your first change makes it your own copy".into(),
            (None, Some(g)) if g.shortcut => "Not a Steam game · works because it is added to Steam".into(),
            _ => "Changes apply when you click back into the game".into(),
        };
        // .pdqt{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:11.5px;color:var(--fg3)}
        // the last change made a new action set: its "Undo" next to the set's button
        let undo = (self.undo_at() == Some(K_SET)).then(|| {
            let k = sub(K_SET, "undo");
            self.reg(k, Ctl::Undo);
            link::link(cx, k, "Undo", 11.5)
        });
        El::row()
            .center()
            .gap(8.0)
            .margin(0.0, 0.0, 6.0, 0.0)
            // Order 045: `h('button',{class:'mbtn pdset','data-tip':'Action set · Steam can switch between sets with a button'})`
            .child(mbtn::mbtn(cx, K_SET, Mb::Set(&set_name), false).tip("Action set \u{b7} Steam can switch between sets with a button"))
            .children(undo)
            .child(El::text(text, Font::new(11.5, 400), FG3(), lh(11.5, 1.35)).ellipsis().shrink(1.0).min_w(0.0))
            .child(look::pdlive(self.connected().is_some()))
    }

    /// One-shot keyframe progress (delay, duration, easing) since `t`.
    fn kf(cx: &mut Cx, t: f64, delay: f64, dur: f64, ease: Bezier) -> f32 {
        if cx.rm {
            return 1.0;
        }
        let p = ((cx.now - t - delay) / dur).clamp(0.0, 1.0);
        if p < 1.0 {
            cx.st.busy = true;
        }
        ease.ease(p) as f32
    }

    /// `.pdb` > `.pdl4`: the picture (+ chips), always big and centred; a picked part's settings are a popup window
    /// (`part_window`).
    fn body(&mut self, cx: &mut Cx) -> El {
        let mut picture = self.picture(cx, BIG_W);
        if let Some(t) = self.dev_at {
            let e = Self::kf(cx, t, 0.0, 280.0, EASE_OUT);
            if e >= 1.0 {
                self.dev_at = None;
            } else {
                picture = picture.opacity(e).scale(0.97 + 0.03 * e);
            }
        }
        // .pdl4 .cpl{flex:1 1 auto;min-width:0;display:flex;flex-direction:column;align-items:center} (the Gyro chip and
        // the links sit inside the picture, between the grips)
        let cpl = El::col().items(AlignItems::CENTER).child(picture).flex1();
        // a click on empty space on the page = nothing picked (`.pdb` is clickable; the picture takes its own clicks)
        El::block().key(K_BODY).on_click(K_BODY).child(El::row().items(AlignItems::FLEX_START).child(cpl))
    }

    /// `.cpw` + `.cps` + `.pdlb` + `.pdhint`
    fn picture(&mut self, cx: &mut Cx, w: f32) -> El {
        let h = w * self.pic.h / pic::PIC_W;
        let s = w / pic::PIC_W;
        let st = foot_stack(s, self.pic.h, self.kind.has_gyro());
        let foot_el = self.foot(cx);
        let chip_el = st.gyro.map(|_| {
            let mode = self.view.as_ref().and_then(|v| v.gyro.as_ref()).map(|g| gyro_label(&g.mode)).unwrap_or("Off");
            look::pdcp(cx, K_GYRO, "pgyr", "Gyro", mode, self.sel == Some(Pid::Gyro))
        });
        let parts = &self.pic.parts;
        let mut looks = Vec::with_capacity(parts.len());
        let mut hovered = None;
        for p in parts {
            let nb = p.hit_boxes(cx.g).len();
            let on = (0..nb).any(|i| cx.hovered(Self::box_key(p.id, i)));
            let hv = cx.tr(sub(K_PART, &p.id.name()), 1, if on { 1.0 } else { 0.0 }, 150.0, EASE);
            if on {
                hovered = Some(p.id);
            }
            let side_cap = match p.id {
                Pid::Stick(Side::Left) => self.lv.ls,
                Pid::Stick(Side::Right) => self.lv.rs,
                _ => (0.0, 0.0),
            };
            let pull = match p.id {
                Pid::Trig(Side::Left) => self.lv.l2,
                Pid::Trig(Side::Right) => self.lv.r2,
                _ => 0.0,
            };
            let dn = match p.id {
                Pid::Trig(_) => pull > 0.08,
                Pid::Stick(Side::Left) => self.lv.dn.contains(&Pid::B(ButtonId::L3)),
                Pid::Stick(Side::Right) => self.lv.dn.contains(&Pid::B(ButtonId::R3)),
                id => self.lv.dn.contains(&id),
            };
            // the drawing without motion never moves the caps (`live()` runs only in its loop): frozen pictures keep them centred
            let cap_off = if self.frozen { (0.0, 0.0) } else { (side_cap.0 * 7.0, side_cap.1 * 7.0) };
            looks.push(pic::Look { hv, sel: self.sel == Some(p.id), dn, pull, cap_off });
        }
        let lc = if self.kind.has_light_bar() {
            self.pref().and_then(|p| p.led()).map(|(r, g, b)| if (r, g, b) == (0, 0, 0) { FG3() } else { Rgba::rgb(r, g, b) })
        } else {
            None
        };
        let pc = self.pic.clone();
        let live = !self.frozen && (self.fake || self.live.is_some());
        let mut paint = El::paint(move |g, (x, y, w, _)| pic::paint(g, &pc, (x, y, w), &|i| looks[i], lc)).abs(0.0, 0.0, f32::NAN, f32::NAN).size(w, h).no_hit();
        if live {
            paint = paint.live();
        }
        let mut cpw = El::block().size(w, h).none().key(K_PIC).on_click(K_PIC).child(paint);
        // the parts' hover / click boxes (in the drawing the SVG shapes themselves; here their boxes - a press is checked
        // against the exact shape)
        for p in parts {
            for (i, (l, t, r, b)) in p.hit_boxes(cx.g).into_iter().enumerate() {
                let k = Self::box_key(p.id, i);
                cpw = cpw.child(El::block().abs(l * s, t * s, f32::NAN, f32::NAN).size((r - l) * s, (b - t) * s).on_click(k).cursor(Cursor::Hand));
            }
        }
        // the owner (test build 1): no scrolling just for the bottom links. The empty space between the grips (under the
        // arch) holds, bottom up: "Controller settings · Open in Steam" 10 picture px above the picture's bottom, the
        // Gyro chip 8 px above them, "Click buttons to edit" 8 px above that (page px; placed by `foot_stack`)
        cpw = cpw.child(foot_el.abs(0.0, st.links, f32::NAN, f32::NAN).w(w));
        if let (Some(gy), Some(chip)) = (st.gyro, chip_el) {
            cpw = cpw.child(El::row().justify(JustifyContent::CENTER).abs(0.0, gy, f32::NAN, f32::NAN).w(w).no_hit().child(chip));
        }
        // "Click buttons to edit": only while nothing is picked
        if self.sel.is_none() {
            cpw = cpw.child(look::pdhint(cx, w, st.hint, 1.0));
        }
        // the hover label (.pdlb): name + what it does now, above the part
        if hovered != self.lab_off {
            self.lab_off = None;
        }
        if let Some(id) = hovered.filter(|h| Some(*h) != self.lab_off) {
            if let Some(p) = parts.iter().find(|p| p.id == id) {
                let (l, t, r, _) = p.bbox(cx.g);
                let lab = self.part_label(cx, id);
                let lw = Self::measure(cx, &lab);
                let x = ((l + r) / 2.0 * s).round();
                let y = (t * s - 6.0).round();
                cpw = cpw.child(lab.abs(x - lw / 2.0, y - 23.0, f32::NAN, f32::NAN).z(3));
            }
        }
        cpw
    }

    fn measure(cx: &mut Cx, el: &El) -> f32 {
        crate::ui::lay::Laid::new(cx.g, El::row().child(el.clone().none()), 2000.0, None).nodes.get(1).map(|n| n.rect.2).unwrap_or(0.0)
    }

    /// `.pdlb{padding:4px 9px;border-radius:7px;display:flex;align-items:center;gap:6px;font-size:11.5px;line-height:15px;
    /// color:var(--fg2);background:var(--menu);backdrop-filter:blur(30px) saturate(180%);box-shadow:inset 0 0 0 .5px var(--hl),
    /// 0 0 0 .5px rgba(0,0,0,.3),0 6px 18px rgba(0,0,0,.28)}` `.pdlb b{font-weight:600;color:var(--fg)}`
    fn part_label(&self, _cx: &mut Cx, id: Pid) -> El {
        let mut kids = vec![El::text(self.pname(id), Font::new(11.5, 600), FG(), 15.0).none()];
        let f = Font::new(11.5, 400);
        let txt = |t: String| El::text(t, f, FG2(), 15.0).none();
        if let Some(v) = &self.view {
            match id {
                Pid::B(b) => {
                    if let Some(bv) = v.buttons.iter().find(|x| x.id == b) {
                        if bv.fixed {
                            kids.push(txt("Opens Steam".into()));
                        } else if let Some((_, a)) = bv.presses.first() {
                            kids.extend(look::act_nodes(&show(a, self.xbox()), 11.5, false));
                        }
                    }
                }
                Pid::Stick(side) => {
                    if let Some(st) = v.sticks.iter().find(|s| s.side == side) {
                        let (i, o) = stick_dz(st);
                        let curve = curve_name(sv(&st.settings, StickSetting::Curve));
                        kids.push(txt(format!("dead zone {} · full at {} · {} curve", pct(i), pct(o), curve)));
                    }
                }
                Pid::Trig(side) => {
                    if let Some(t) = v.triggers.iter().find(|t| t.side == side) {
                        let at = sv(&t.settings, TriggerSetting::ClicksAt).map(radius_to_pct).unwrap_or(100.0);
                        kids.push(txt(if t.analog {
                            if at < 100.0 {
                                format!("Analog · clicks at {}", pct(at))
                            } else {
                                "Analog".into()
                            }
                        } else {
                            format!("Click only, at {}", pct(at))
                        }));
                    }
                }
                Pid::Touch => {
                    if let Some(t) = &v.touchpad {
                        let a = |x: &Action| if matches!(x, Action::Key(_)) { format!("{} key", x.label(false)) } else { x.label(false) };
                        kids.push(txt(format!("click: {} · {}{}", a(&t.left_click), a(&t.right_click), if t.touch == TouchMode::Mouse { " · touch = mouse" } else { "" })));
                    }
                }
                Pid::Light => kids.push(txt("this controller, every game".into())),
                _ => {}
            }
        }
        El::row()
            .center()
            .gap(6.0)
            .pad(4.0, 9.0, 4.0, 9.0)
            .radius(7.0)
            .bg(crate::ui::MENU())
            .backdrop(30.0, 1.8)
            .shadow(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, 0.3)), sh(0.0, 6.0, 18.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.28))])
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, crate::ui::HL_V19())])
            .no_hit()
            .children(kids)
    }

    /// `PNAME`: the part's name on this pad.
    fn pname(&self, id: Pid) -> String {
        match id {
            Pid::B(b) => b.name(self.kind).into(),
            Pid::Fn(Side::Left) => "Fn left".into(),
            Pid::Fn(Side::Right) => "Fn right".into(),
            Pid::Stick(Side::Left) => "Left stick".into(),
            Pid::Stick(Side::Right) => "Right stick".into(),
            Pid::Trig(Side::Left) => (if self.xbox() { "LT trigger" } else { "L2 trigger" }).into(),
            Pid::Trig(Side::Right) => (if self.xbox() { "RT trigger" } else { "R2 trigger" }).into(),
            Pid::Touch => "Touchpad".into(),
            Pid::Light => "Light bar".into(),
            Pid::Gyro => "Gyro".into(),
        }
    }

    /// The picked part's settings as a popup window (the shared readable `dialog`): titled with the part, a line with the
    /// back-button tag / "Steam's setting for this", then the settings, scrolling inside the window when they are taller.
    fn part_window(&mut self, cx: &mut Cx, id: Pid) -> El {
        // a swap (another part): the line and the settings fade up 6 px (240 ms, the settings 30 ms later)
        let sw = |cx: &mut Cx, i: f64| Self::kf(cx, self.swap_at, i * 30.0, 240.0, EASE_OUT);
        let (h_e, b_e) = (sw(cx, 0.0), sw(cx, 1.0));
        let back = matches!(id, Pid::B(ButtonId::BackLeftUpper | ButtonId::BackLeftLower | ButtonId::BackRightUpper | ButtonId::BackRightLower));
        let steam = pid_part(id).is_some() && self.steam.is_some() && id != Pid::B(ButtonId::Home);
        let mut kids = Vec::new();
        // the last change was "Steam's setting for this": its "Undo" sits next to that link
        let undo = (self.undo_at() == Some(K_STEAMSET)).then(|| sub(K_STEAMSET, "undo"));
        if back || steam {
            let mut line = El::row().center().gap(8.0).min_h(18.0).margin(0.0, 0.0, 8.0, 0.0);
            if back {
                line = line.child(look::back_tag());
            }
            line = line.child(El::block().flex1());
            if let Some(k) = undo {
                self.reg(k, Ctl::Undo);
                line = line.child(link::link(cx, k, "Undo", 12.0)).child(El::block().size(3.0, 3.0).none().radius(crate::ui::el::RADIUS_PILL).bg(FG3()).opacity(0.7));
            }
            if steam {
                line = line.child(link::link(cx, K_STEAMSET, "Steam\u{2019}s setting for this", 12.0));
            }
            kids.push(line.opacity(h_e).translate(0.0, 6.0 * (1.0 - h_e)));
        }
        kids.push(self.panel_body(cx, id).opacity(b_e).translate(0.0, 6.0 * (1.0 - b_e)));
        // `.mdb{min-height:0;overflow-y:auto}` with the inner popups' slim glass thumb
        let body = cx.scroll_box(K_PANEL, kids).items(AlignItems::STRETCH).min_h(0.0).slim_thumb(crate::ui::el::SlimThumb::GLASS).style(|s| s.flex_shrink = 1.0);
        dialog::dialog(cx, K_PANEL, PANEL_W, &self.pname(id), vec![body], vec![], true, self.part_at)
    }

    /// The page's foot: `.pdf{display:flex;align-items:center;gap:12px;margin:12px 4px 0;font-size:12px}` `#sw .pdf{justify-content:center;
    /// margin-top:10px}` - "Controller settings · Open in Steam".
    /// "Controller settings" (the light bar, rumble... of the controller itself): the owner (test build 1) "why can i change
    /// them when i don't have a controller plugged in" - without the shown controller plugged in it is dimmed text with a
    /// tip, and does nothing.
    fn ctlset_link(&mut self, cx: &mut Cx) -> El {
        if self.connected().is_some() {
            link::link(cx, K_CTLSET, "Controller settings", 12.0)
        } else {
            El::text("Controller settings", Font::new(12.0, 400), FG3(), 16.0).none().key(K_CTLSET).tip(NO_PAD_TIP)
        }
    }

    fn foot(&mut self, cx: &mut Cx) -> El {
        El::row()
            .center()
            .justify(JustifyContent::CENTER)
            .gap(12.0)
            .child(self.ctlset_link(cx))
            .child(El::text("\u{b7}", Font::new(12.0, 400), FG3(), 16.0).none())
            .child(link::link(cx, K_OPENSTEAM, "Open in Steam", 12.0))
    }

    // ============================================================================================ events
    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        let now = cx.now;
        // Steam closed: the glass covers the tab - only its "Launch Steam" does something (the owner, test build 2: "i
        // shouldn't be able to change settings unless steam is launched")
        if !self.steam_up {
            if matches!(ev, Ev::Click(k) if *k == K_LAUNCH) {
                self.launch_steam(now);
            }
            return;
        }
        match ev {
            // Ctrl+Z = undo, Ctrl+Y / Ctrl+Shift+Z = redo (the tab's own list; the key the keyboard layout calls Z / Y)
            Ev::Key(k, vk @ (0x5A | 0x59)) if *k == crate::ui::cx::PAGE && cx.mods.ctrl && !cx.mods.alt => {
                self.undo(*vk == 0x59 || cx.mods.shift, now);
                cx.used = true;
            }
            Ev::Press(k, x, y, r) => {
                self.anchors.insert(*k, *r);
                if *k == K_PIC || self.box_of_key(cx, *k).is_some() {
                    self.pressed_part = self.exact_part(cx, *k, *x, *y, *r);
                }
                if let Some(Ctl::Slider { lo, hi, step, keep, .. }) = self.ctl.get(k).cloned() {
                    let v = pieces::slider::value_at(*r, *x) as f64;
                    self.drag = Some((*k, keep_in(snap(lo + (hi - lo) * v, step), (lo, hi), keep)));
                }
                if let Some(Ctl::Well { inner, outer, at }) = self.ctl.get(k).cloned() {
                    // `drag = |v - inner| <= |v - outer| ? 'in' : 'out'`
                    let v = well_at(*r, *x, *y);
                    let ring = if (v - at.0).abs() <= (v - at.1).abs() { inner } else { outer };
                    self.well_drag = Some((*k, ring));
                    self.well_move(*r, *x, *y);
                }
            }
            Ev::Drag(k, x, y, r) if self.well_drag.map(|w| w.0) == Some(*k) => self.well_move(*r, *x, *y),
            Ev::Drag(k, x, _, r) => {
                if let Some(Ctl::Slider { lo, hi, step, keep, .. }) = self.ctl.get(k).cloned() {
                    let v = pieces::slider::value_at(*r, *x) as f64;
                    self.drag = Some((*k, keep_in(snap(lo + (hi - lo) * v, step), (lo, hi), keep)));
                }
            }
            Ev::Release(k) => {
                let target = match self.well_drag.take() {
                    Some((wk, ring)) if wk == *k => ring,
                    _ => *k,
                };
                if let Some((dk, v)) = self.drag.take() {
                    if dk == target {
                        let k = &target;
                        if let Some(Ctl::Slider { def, conv, w, step, .. }) = self.ctl.get(k).cloned() {
                            let raw = if (v - def).abs() < step / 2.0 { None } else { Some(to_raw(v, conv)) };
                            self.acting = self.rows.get(k).cloned();
                            self.put(w, raw, now);
                        }
                    }
                }
            }
            Ev::Click(k) => self.click(*k, cx),
            Ev::Char(k, c) if *k == K_ASRCH => {
                if let Some(Pop::Act { q, ki, .. }) = &mut self.pop {
                    search::edit_char(q, *c);
                    *ki = 0;
                }
            }
            Ev::Key(k, vk) if *k == K_ASRCH => self.act_key(*vk, cx),
            // `keydown Escape` while a part is picked (no popup / dialog open): back to the big controller
            Ev::Key(k, 0x1B) if *k == crate::ui::cx::PAGE && self.sel.is_some() && self.pop.is_none() && self.dlg.is_none() => {
                self.pick(None, cx.now);
                cx.used = true;
            }
            _ => {}
        }
    }

    /// Move the dragged ring (`move`: inner = clamp(v, 0, outer - 5), outer = clamp(v, inner + 5, 100), within the sliders' ranges).
    fn well_move(&mut self, r: (f32, f32, f32, f32), x: f32, y: f32) {
        let Some((wk, ring)) = self.well_drag else { return };
        let Some(Ctl::Well { inner, at, .. }) = self.ctl.get(&wk).cloned() else { return };
        let Some(Ctl::Slider { lo, hi, .. }) = self.ctl.get(&ring).cloned() else { return };
        let v = well_at(r, x, y).round();
        let keep = if ring == inner { (0.0, at.1 - 5.0) } else { (at.0 + 5.0, 100.0) };
        self.drag = Some((ring, keep_in(v, (lo, hi), keep)));
    }

    fn part_of_key(&self, k: Key) -> Option<Pid> {
        self.pic.parts.iter().find(|p| sub(K_PART, &p.id.name()) == k).map(|p| p.id)
    }

    /// The key of a part's hover / click box i (the light bar has one per strip).
    fn box_key(id: Pid, i: usize) -> Key {
        // the test hook's `el:pad.part.<name>` (`.<i>` for the second light strip)
        if i == 0 {
            key(&format!("pad.part.{}", id.name()))
        } else {
            key(&format!("pad.part.{}.{i}", id.name()))
        }
    }

    /// (part, box index) of a box key.
    fn box_of_key(&self, cx: &mut Cx, k: Key) -> Option<(Pid, usize)> {
        for p in &self.pic.parts {
            for i in 0..p.hit_boxes(cx.g).len() {
                if Self::box_key(p.id, i) == k {
                    return Some((p.id, i));
                }
            }
        }
        None
    }

    /// The part whose painted shape is under (x, y) (the SVG hit test: the boxes only bring the press here).
    fn exact_part(&self, cx: &mut Cx, k: Key, x: f32, y: f32, r: (f32, f32, f32, f32)) -> Option<Pid> {
        let s = BIG_W / pic::PIC_W;
        let origin = if k == K_PIC {
            (r.0, r.1)
        } else {
            let (id, i) = self.box_of_key(cx, k)?;
            let p = self.pic.parts.iter().find(|p| p.id == id)?;
            let b = p.hit_boxes(cx.g)[i];
            (r.0 - b.0 * s, r.1 - b.1 * s)
        };
        let (px, py) = ((x - origin.0) / s, (y - origin.1) / s);
        let exact = self.pic.parts.iter().rev().find(|p| p.contains(cx.g, px, py)).map(|p| p.id);
        exact.or_else(|| if k == K_PIC { None } else { self.box_of_key(cx, k).map(|b| b.0) })
    }

    fn click(&mut self, k: Key, cx: &mut Cx) {
        let now = cx.now;
        // ---- the picture
        if k == K_PIC || self.box_of_key(cx, k).is_some() {
            if let Some(id) = self.pressed_part.take() {
                self.lab_off = Some(id);
                self.pick(Some(id), now);
            }
            return;
        }
        match k {
            K_BODY => {
                if self.sel.is_some() {
                    self.pick(None, now);
                }
                return;
            }
            // the part's window: its × or a click beside it = back to the controller; a click on its blank parts closes
            // a list open in it
            _ if k == sub(K_PANEL, "x") || k == sub(K_PANEL, "out") => return self.pick(None, now),
            _ if k == K_PANEL || k == sub(K_PANEL, "win") => {
                if matches!(self.pop, Some(Pop::Menu(_) | Pop::Act { .. })) {
                    self.pop = None;
                }
                return;
            }
            K_GYRO => return self.pick(Some(Pid::Gyro), now),
            K_GAME => return self.toggle_pop(Pop::Game),
            K_DEV => return self.toggle_pop(Pop::Dev),
            K_SET => return self.toggle_pop(Pop::Set),
            K_CTLSET => {
                if self.connected().is_some() {
                    self.pop = None;
                    self.dlg = Some(now);
                }
                return;
            }
            K_OPENSTEAM => return self.open_in_steam(now),
            K_STEAMSET => return self.part_to_steam(now),
            _ => {}
        }
        if k == sub(K_RESET, "pc") || k == sub(K_RESET, "win") {
            let win = k == sub(K_RESET, "win");
            if self.frozen {
                // test pictures: the drawing's sample review (nothing on the PC changes)
                self.pop = Some(Pop::Review { win, ticked: true });
            } else {
                // the frame's review over the change log, under the link
                self.pop = None;
                cx.open_reset(if win { crate::undo::Kind::WindowsDefaults } else { crate::undo::Kind::HowItWas }, self.anchor(k));
            }
            return;
        }
        if k == sub(K_DLG, "x") || k == sub(K_DLG, "out") {
            self.dlg = None;
            self.pop = None;
            return;
        }
        if self.review_click(k) || self.menu_click(k, now) || self.act_click(k, cx) {
            return;
        }
        // ---- a control. A list open inside the part's window closes on a click on anything else in that window (the
        // window belongs to the popup layer, so the frame's click-beside-a-popup dismiss never sees those clicks)
        let c = self.ctl.get(&k).cloned();
        if !matches!(c, Some(Ctl::Menu { .. } | Ctl::Act { .. })) && matches!(self.pop, Some(Pop::Menu(_) | Pop::Act { .. })) {
            self.pop = None;
        }
        let Some(c) = c else { return };
        // the row (and words) of this change, for the undo list and its "Undo" link
        self.acting = self.rows.get(&k).cloned();
        match c {
            Ctl::Toggle { on, w, on_v, off_v } => self.put(w, if on { off_v } else { on_v }, now),
            Ctl::Val { on: true, .. } => self.acting = None,
            Ctl::Val { v, w, .. } => self.put(w, v, now),
            Ctl::Back(Back::Layout(c)) => self.write(c, now),
            Ctl::Back(Back::Pref(v)) => self.write_pref(&v, "Steam\u{2019}s default", now),
            Ctl::Undo => self.undo(false, now),
            Ctl::DriftGo(side) => self.drift = Some(Drift::Run { side, at: now, max: 0.0 }),
            Ctl::DriftUse(side, dz) => {
                self.drift = None;
                self.acting = self.rows.get(&Self::k(&format!("{}.dz", stick_id(side)))).cloned();
                // like the slider: the default (8 %) = no value in the file
                let raw = if (dz - 8.0).abs() < 0.5 { None } else { Some(bu_controller::settings::pct_to_radius(dz)) };
                self.put(W::Stick(side, StickSetting::DeadZone), raw, now);
            }
            Ctl::Menu { .. } => {
                self.acting = None;
                self.toggle_pop(Pop::Menu(k));
            }
            Ctl::Act { .. } => {
                self.acting = None;
                self.pop = Some(Pop::Act { at: k, q: String::new(), ki: 0 });
                self.act_fresh = true;
                cx.focus(Some(K_ASRCH));
            }
            Ctl::Slider { .. } | Ctl::Well { .. } | Ctl::Dead => self.acting = None,
        }
    }

    fn toggle_pop(&mut self, p: Pop) {
        self.pop = if self.pop.as_ref() == Some(&p) { None } else { Some(p) };
    }

    /// "Open in Steam": the link (Steam's process list) and the shell's open on the worker (Order 047); its words come
    /// with the answer.
    fn open_in_steam(&mut self, _now: f64) {
        let Some(g) = self.game().cloned() else { return };
        let test = self.test;
        self.send(Job::OpenInSteam { game: g, test });
    }

    /// "Steam's setting for this": the part's values of Steam's own layout - shown at once, written on the worker.
    fn part_to_steam(&mut self, _now: f64) {
        let (Some(id), Some(g)) = (self.sel, self.game().cloned()) else { return };
        let Some(part) = pid_part(id) else { return };
        let (before, kind, set) = (self.view.clone(), self.kind, self.set);
        let steams: Option<Vec<Change>> = self.steam.as_ref().map(|st| st.part_changes(part).into_iter().filter(|c| c.fits(kind)).collect());
        if let Some(c) = steams {
            self.local(&c);
        }
        let after = After::PartToSteam { before, part, game: g.key.clone(), kind, set, name: self.pname(id) };
        let want = self.want();
        self.send(Job::Write { wr: Wr::PartToSteam { key: g.key, kind, set, part }, want, after });
    }

    // ============================================================================================ popups
    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let mut over: Vec<El> = Vec::new();
        if let Some(id) = self.sel.filter(|p| *p != Pid::Light) {
            over.push(self.part_window(cx, id));
        }
        if let Some(at) = self.dlg {
            over.push(self.settings_dialog(cx, at));
        }
        if let Some(p) = self.pop.clone() {
            if let Some(e) = self.pop_el(cx, &p) {
                // above the part window / Controller settings (the dialog's layer is z 9): a list opened from a control
                // inside them was drawn - and hit - UNDER the window, so "Acts as" & co. could not be changed (the owner,
                // test build 2: "the, acts as joystick option on top ... it can't be changed")
                over.push(e.z(10));
            }
        }
        if over.is_empty() {
            return None;
        }
        Some(El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(WIN_W, crate::ui::WIN_H).no_hit().children(over))
    }

    /// The anchor box of a button (window coordinates, from its last press).
    fn anchor(&self, k: Key) -> (f32, f32, f32, f32) {
        self.anchors.get(&k).copied().unwrap_or((300.0, 100.0, 0.0, 24.0))
    }

    fn pop_el(&mut self, cx: &mut Cx, p: &Pop) -> Option<El> {
        match p {
            Pop::Game => Some(self.game_menu(cx)),
            Pop::Dev => Some(self.dev_menu(cx)),
            Pop::Set => {
                let mut items: Vec<dropdown::Item> = self.sets.iter().map(|s| dropdown::Item { label: s.title.clone(), checked: s.id == self.set, disabled: false }).collect();
                if items.is_empty() {
                    items.push(dropdown::Item { label: "Default".into(), checked: true, disabled: false });
                }
                items.push(dropdown::Item { label: "New action set\u{2026}".into(), checked: false, disabled: self.game().is_none() });
                let a = self.anchor(K_SET);
                Some(dropdown::menu(cx, K_MENU, &items, a.0, a.1 + a.3 + 4.0, a.2.max(150.0)))
            }
            Pop::Menu(at) => {
                let Some(Ctl::Menu { items, cur, width, .. }) = self.ctl.get(at).cloned() else { return None };
                let list: Vec<dropdown::Item> = items.iter().map(|(v, l)| dropdown::Item { label: l.clone(), checked: *v == cur, disabled: false }).collect();
                let a = self.anchor(*at);
                Some(dropdown::menu(cx, K_MENU, &list, a.0, a.1 + a.3 + 4.0, a.2.max(width).max(150.0)))
            }
            Pop::Act { at, q, ki } => Some(self.act_menu(cx, *at, q, *ki)),
            Pop::Review { win, ticked } => {
                let g = self.game().map(|g| g.name.clone()).unwrap_or_default();
                let what = format!("{g} \u{b7} {}", self.kind.name());
                let (title, text, line) = if *win {
                    ("Controller \u{b7} back to Steam\u{2019}s layout?", "The layout goes back to the one Steam made for this game.", reset::Line { title: what, from: "your layout".into(), to: "Steam\u{2019}s layout".into(), ticked: *ticked, heading: None })
                } else {
                    ("Controller \u{b7} back to how it was?", "Each one goes back to the value it had before this app changed it.", reset::Line { title: what, from: "your edits".into(), to: "as on 7 Oct (backup)".into(), ticked: *ticked, heading: None })
                };
                let n = if *ticked { 1 } else { 0 };
                let buttons = vec![
                    button::cbtn_sized(cx, sub(K_REVIEW, "no"), "Cancel", button::Kind::Ghost, button::MCFB, false, 0.0),
                    button::cbtn_sized(cx, sub(K_REVIEW, "go"), if n > 0 { "Reset 1" } else { "Reset" }, button::Kind::Red, button::MCFB, n == 0, 0.0),
                ];
                let a = self.anchor(if *win { sub(K_RESET, "win") } else { sub(K_RESET, "pc") });
                // placeMenu(btn, 316): under the link, or above it when there is no room below
                let est = 80.0 + 40.0;
                let y = if a.1 + a.3 + 4.0 + est > crate::ui::WIN_H - 8.0 { (a.1 - est - 4.0).max(8.0) } else { a.1 + a.3 + 4.0 };
                Some(reset::review_popup(cx, K_REVIEW, a.0, y, title, text, &[line], buttons))
            }
        }
    }

    /// The drawing's sample review (test pictures only): tick / untick, Cancel, Reset (closes it - a picture changes
    /// nothing; every other copy uses the frame's review over the change log).
    fn review_click(&mut self, k: Key) -> bool {
        let Some(Pop::Review { win, ticked }) = self.pop.clone() else { return false };
        if k == idx(K_REVIEW, 0) {
            self.pop = Some(Pop::Review { win, ticked: !ticked });
            return true;
        }
        if k == sub(K_REVIEW, "no") || k == sub(K_REVIEW, "go") {
            self.pop = None;
            return true;
        }
        false
    }

    /// The menu lists (game, controller, action set, a popup button's choices): item i = idx(K_MENU, i).
    fn menu_click(&mut self, k: Key, now: f64) -> bool {
        let Some(p) = self.pop.clone() else { return false };
        let n = match &p {
            Pop::Game => self.games.len() + 2,
            Pop::Dev => PadKind::ALL.len(),
            Pop::Set => self.sets.len().max(1) + 1,
            Pop::Menu(at) => match self.ctl.get(at) {
                Some(Ctl::Menu { items, .. }) => items.len(),
                _ => 0,
            },
            _ => return false,
        };
        let Some(i) = (0..n).find(|i| idx(K_MENU, *i) == k) else { return false };
        self.pop = None;
        match p {
            Pop::Game => {
                if i < self.games.len() && i != self.gi {
                    // its files are read on the worker (Order 047): the panels wait for them
                    self.gi = i;
                    self.set = 0;
                    self.sel = None;
                    self.clear_view();
                    self.sets.clear();
                    self.reload();
                }
            }
            Pop::Dev => {
                let kd = PadKind::ALL[i];
                if kd != self.kind {
                    self.dev_at = Some(now);
                    self.kind = kd;
                    self.pic = pic::pic(kd);
                    self.sel = None;
                    self.load_games();
                    self.start_live();
                }
            }
            Pop::Set => {
                let ns = self.sets.len().max(1);
                if i < self.sets.len() {
                    self.set = self.sets[i].id;
                    self.load_view();
                } else if i == ns {
                    if let Some(g) = self.game().cloned() {
                        let title = format!("Set {}", self.sets.len() + 1);
                        let (kind, from) = (self.kind, self.set);
                        // shown at once (the same copy bu-controller makes in the file, on the worker)
                        self.local_new_set(from, &title);
                        let want = Want { kind, key: Some(g.key.clone()), set: None, fresh: false };
                        self.send(Job::Write { wr: Wr::NewSet { key: g.key.clone(), kind, from, title: title.clone() }, want, after: After::NewSet { game: g.key, kind, from, title } });
                    }
                }
            }
            Pop::Menu(at) => {
                if let Some(Ctl::Menu { items, w, cur, .. }) = self.ctl.get(&at).cloned() {
                    // the item that is already set: nothing to write (no backup, no undo step, no own copy of a community layout)
                    if let Some((v, _)) = items.get(i).filter(|(v, _)| *v != cur) {
                        self.acting = self.rows.get(&at).cloned();
                        self.put(w, *v, now);
                    }
                }
            }
            _ => {}
        }
        true
    }

    /// `openGameMenu`: narrow (196 px), opens to the right of its button; the games, a line, the Desktop (greyed).
    fn game_menu(&mut self, cx: &mut Cx) -> El {
        // row i = the game i (`Ev::Click(idx(K_MENU, i))`), then the line (a row of its own), then the Desktop (n + 1)
        let none = Rgba(0.0, 0.0, 0.0, 0.0);
        let mut list: Vec<Row> = self.games.iter().enumerate().map(|(i, g)| Row::Item(It::tick(&g.name, i == self.gi).lead(Lead::Gt("", none)))).collect();
        // .msep{height:1px;margin:4px 6px;background:var(--hair)}
        list.push(Row::Sep);
        // .mitem.dis .gt{background:var(--ctl)!important;color:var(--fg3)} - Steam keeps the Desktop layout inside Steam
        list.push(Row::Item(It::tick("Desktop", false).lead(Lead::Gt("D", none)).disabled(true)));
        let mut rows = mitems::rows(cx, K_MENU, &list);
        // the game tiles: the page's gradient `.gt` in the 16 px slot `Lead::Gt` holds (child 0 = the tick column, 1 = the lead)
        for (g, row) in self.games.iter().zip(rows.iter_mut()) {
            let (init, c1, c2) = game_tile(&g.name);
            if let Some(slot) = row.children.get_mut(1) {
                *slot = gt(&init, c1, c2, 16.0).no_hit();
            }
        }
        let a = self.anchor(K_GAME);
        let est = 10.0 + 26.0 * (self.games.len() + 1) as f32 + 9.0;
        // menu.style.width = 196px; left = min(button left, window - 196 - 8); top = button bottom + 4
        dropdown::menu_box(cx, K_MENU, a.0.min(WIN_W - 196.0 - 8.0), a.1 + a.3 + 4.0, 196.0, est, 300.0, rows).w(196.0)
    }

    /// The controller popup (`placeMenu(pdDev, 240)`): ✓, the pad icon, the name, its state on the right.
    fn dev_menu(&mut self, cx: &mut Cx) -> El {
        let states: Vec<String> = PadKind::ALL
            .iter()
            .map(|kd| match self.pads.iter().find(|p| p.kind == *kd) {
                Some(p) => p.battery.and_then(|b| b.percent).map(|b| format!("{b} %")).unwrap_or_else(|| "connected".into()),
                None => "not connected".into(),
            })
            .collect();
        // .mitem .mi2{width:16px;display:grid;place-items:center} .mi2 svg{width:15px;stroke-width:1.5}; .mr = the state
        let list: Vec<Row> = PadKind::ALL
            .iter()
            .zip(&states)
            .map(|(kd, st)| Row::Item(It::tick(kd.name(), *kd == self.kind).lead(Lead::Mi2("pad")).right(Right::Mr(st))))
            .collect();
        let rows = mitems::rows(cx, K_MENU, &list);
        let a = self.anchor(K_DEV);
        // the list is as wide as its widest row (shrink-to-fit), at least 240 (placeMenu(pdDev, 240)) + the menu's padding
        let widest = rows.iter().map(|r| Self::measure(cx, r)).fold(0.0f32, f32::max);
        let w = 240.0f32.max(a.2).max(widest + 10.0);
        let x = if a.0 + w > WIN_W - 8.0 { (a.0 + a.2 - w).max(8.0) } else { a.0 };
        dropdown::menu_box(cx, K_MENU, x, a.1 + a.3 + 4.0, w, 10.0 + 26.0 * 4.0, 300.0, rows).w(w)
    }

    // ---- the "does" popup (`.menu.padm`): search on top, sections below
    fn act_items(&self, q: &str) -> Vec<(Option<&'static str>, Action)> {
        let s = q.trim().to_lowercase();
        let mut out = Vec::new();
        if s.is_empty() {
            out.push((None, Action::Nothing));
        }
        for (sec, list) in act_sections() {
            let mut first = true;
            for a in list {
                let hit = s.is_empty() || act_words(&a).contains(&s) || matches!(&a, Action::Key(k) if key_label(k).to_lowercase() == s);
                if hit {
                    out.push((if first { Some(sec) } else { None }, a));
                    first = false;
                }
            }
        }
        out
    }

    fn act_menu(&mut self, cx: &mut Cx, at: Key, q: &str, ki: usize) -> El {
        let cur = match self.ctl.get(&at) {
            Some(Ctl::Act { cur, .. }) => cur.clone(),
            _ => Action::Nothing,
        };
        let items = self.act_items(q);
        let xb = self.xbox();
        let mut rows: Vec<(f32, El)> = Vec::new();
        let mut cur_y = None;
        let mut y = 0.0f32;
        let mut first_head = true;
        for (i, (head, a)) in items.iter().enumerate() {
            if let Some(h) = head {
                // .pmh{padding:7px 8px 3px;font-size:10.5px;font-weight:600;color:var(--fg3);letter-spacing:.02em} .pmh:first-child{padding-top:2px}
                let top = if first_head && i == 0 { 2.0 } else { 7.0 };
                first_head = false;
                let lhh = lh(10.5, 1.35);
                rows.push((top + 3.0 + lhh, El::text(*h, Font::new(10.5, 600).ls(210), FG3(), lhh).pad(top, 8.0, 3.0, 8.0)));
                y += top + 3.0 + lhh;
            }
            let k = idx(K_ACT, i);
            let hv = cx.hovered(k);
            let kb = !q.trim().is_empty() && i == ki;
            let col = if hv { WHITE } else { FG() };
            let ck = El::text(if *a == cur { "\u{2713}" } else { "" }, Font::new(12.0, 400), col, lh(12.0, 1.35)).w(14.0).none().align(crate::gfx::Align::Center);
            let mut r = El::row().center().gap(6.0).h(26.0).pad(0.0, 14.0, 0.0, 6.0).radius(5.0).on_click(k).child(ck);
            r = match a {
                Action::Key(t) => r.child(ibtn::keycap(&key_label(t), &ibtn::CAP_SM, false)),
                Action::Pad(b) if !xb && pad_glyph(*b).is_some() => {
                    // .mitem .pg2{width:16px;height:16px} .mitem:hover .pg2{color:#fff}
                    r.child(look::pg2(pad_glyph(*b).unwrap_or(""), 16.0, if hv { WHITE } else { FG2() })).child(El::text(a.label(false), Font::new(13.0, 400), col, lh(13.0, 1.35)))
                }
                _ => r.child(El::text(a.label(xb), Font::new(13.0, 400), col, lh(13.0, 1.35))),
            };
            // .mitem:hover{background:var(--acc)} .mitem.kb{background:var(--hov)}
            if hv {
                r = r.bg(ACC());
            } else if kb {
                r = r.bg(HOV());
            }
            if *a == cur {
                cur_y = Some(y);
            }
            if kb {
                cur_y = Some(y);
            }
            rows.push((26.0, r));
            y += 26.0;
        }
        if items.is_empty() {
            // .pme{padding:10px 8px;font-size:12px;color:var(--fg3)}
            rows.push((36.0, El::text(format!("Nothing matches \u{201c}{}\u{201d}", q.trim()), Font::new(12.0, 400), FG3(), lh(12.0, 1.35)).pad(10.0, 8.0, 10.0, 8.0)));
            y += 36.0;
        }
        // .pml{max-height:250px;overflow-y:auto}: a scrolling box (the wheel moves it); opened at the current item
        // (`list.scrollTop = cur.offsetTop - list.clientHeight / 2 + 13`), the keyboard row kept in view (ArrowUp / Down)
        let total = y;
        let lk = sub(K_ACT, "list");
        let max = (total - 250.0).max(0.0);
        let now_off = cx.st.scroll_y.get(&lk).copied().unwrap_or(0.0);
        let want = match cur_y {
            Some(cy) if self.act_fresh => (cy - 250.0 / 2.0 + 13.0).clamp(0.0, max),
            Some(cy) if !q.trim().is_empty() && (cy < now_off || cy + 26.0 > now_off + 250.0) => (if cy < now_off { cy } else { cy + 26.0 - 250.0 }).clamp(0.0, max),
            _ => now_off.min(max),
        };
        self.act_fresh = false;
        cx.st.scroll_y.insert(lk, want);
        let clip = cx.scroll_box(lk, rows.into_iter().map(|(_, e)| e).collect()).items(AlignItems::STRETCH).max_h(250.0).min_h(0.0);
        // .menu.padm .tsrch{width:100%;height:28px;margin-bottom:5px} (the search piece is 196 wide: `.w(240)` overrides it)
        let s = search::search(cx, K_ASRCH, q, "Search a button, key or click", false).w(240.0).margin(0.0, 0.0, 5.0, 0.0);
        let a = self.anchor(at);
        // .menu.padm{width:252px;padding:6px}
        let est = 12.0 + 33.0 + total.min(250.0);
        let x = if a.0 + 252.0 > WIN_W - 8.0 { (a.0 + a.2 - 252.0).max(8.0) } else { a.0 };
        let y0 = a.1 + a.3 + 4.0;
        let y0 = if y0 + est > crate::ui::WIN_H - 8.0 { (a.1 - est - 4.0).max(8.0) } else { y0 };
        dropdown::menu_box(cx, sub(K_ACT, "box"), x, y0, 252.0, est, 10000.0, vec![s, clip]).w(252.0).pad_all(6.0)
    }

    fn act_click(&mut self, k: Key, cx: &mut Cx) -> bool {
        let Some(Pop::Act { at, q, .. }) = self.pop.clone() else { return false };
        if k == sub(K_ASRCH, "x") {
            if let Some(Pop::Act { q, .. }) = &mut self.pop {
                q.clear();
            }
            return true;
        }
        if k == K_ASRCH || k == sub(K_ACT, "box") {
            return true;
        }
        let items = self.act_items(&q);
        let Some(i) = (0..items.len()).find(|i| idx(K_ACT, *i) == k) else { return false };
        self.choose_act(at, items[i].1.clone(), cx.now);
        cx.focus(None);
        true
    }

    fn choose_act(&mut self, at: Key, a: Action, now: f64) {
        self.pop = None;
        if let Some(Ctl::Act { cur, w, .. }) = self.ctl.get(&at).cloned() {
            if cur != a {
                self.acting = self.rows.get(&at).cloned();
                self.put_action(w, a, now);
            }
        }
    }

    fn act_key(&mut self, vk: u16, cx: &mut Cx) {
        let Some(Pop::Act { at, q, ki }) = self.pop.clone() else { return };
        const ESC: u16 = 0x1B;
        const ENTER: u16 = 0x0D;
        const UP: u16 = 0x26;
        const DOWN: u16 = 0x28;
        let n = self.act_items(&q).len();
        match vk {
            ESC => {
                self.pop = None;
                cx.focus(None);
            }
            ENTER => {
                if let Some((_, a)) = self.act_items(&q).get(ki).cloned() {
                    self.choose_act(at, a, cx.now);
                    cx.focus(None);
                }
            }
            UP | DOWN if n > 0 => {
                let ni = if vk == UP { ki.saturating_sub(1) } else { (ki + 1).min(n - 1) };
                self.pop = Some(Pop::Act { at, q, ki: ni });
            }
            _ => {
                if let Some(Pop::Act { q, ki, .. }) = &mut self.pop {
                    search::edit_key(q, vk);
                    *ki = 0;
                }
            }
        }
    }

    /// "Controller settings" (`miniDlg('Controller settings', ...)`, `.dlg.pdvdlg{width:420px}`): one line why it is different,
    /// then the per-controller settings (`.grp.pdvd{padding:2px 12px 6px}`, labels 150 px) + the Light bar (A_015_02).
    fn settings_dialog(&mut self, cx: &mut Cx, at: f64) -> El {
        let why = El::text("These apply in every game \u{b7} each game\u{2019}s own settings: click the controller", Font::new(12.0, 400), FG2(), 16.0)
            .wrapping()
            .margin(0.0, 0.0, 12.0, 0.0);
        let rows = panel::prefs_rows(self, cx);
        let grp = pieces::group::grp(vec![El::col().items(AlignItems::STRETCH).children(rows)]).pad(2.0, 12.0, 6.0, 12.0);
        dialog::dialog(cx, K_DLG, DLG_W, "Controller settings", vec![why, grp], vec![], true, at)
    }

    // ============================================================================================ registry helpers (panel.rs)
    fn reg(&mut self, k: Key, c: Ctl) {
        self.ctl.insert(k, c);
    }
}

/// Why Controller settings can't open.
const NO_PAD_TIP: &str = "Plug in the controller to change its own settings";

/// Where the parts under the arch sit, in page px from the picture's top (see `Open::picture`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct FootStack {
    /// the top of "Controller settings · Open in Steam" (a 16 px line)
    links: f32,
    /// the top of the Gyro chip (26 px), when the controller has gyro
    gyro: Option<f32>,
    /// the middle of "Click buttons to edit"
    hint: f32,
}

/// `s` = page px per picture px, `pic_h` = the picture's height (picture px).
fn foot_stack(s: f32, pic_h: f32, gyro: bool) -> FootStack {
    let links = (pic_h - 10.0) * s - 16.0;
    let gyro = gyro.then_some(links - 8.0 - 26.0);
    let hint = gyro.unwrap_or(links) - 8.0 - lh(12.5, 1.35) / 2.0;
    FootStack { links, gyro, hint }
}

fn gyro_label(m: &GyroMode) -> &'static str {
    match m {
        GyroMode::Off => "Off",
        GyroMode::Mouse => "As mouse",
        GyroMode::Joystick => "As joystick",
        GyroMode::Camera => "As joystick \u{b7} camera",
        GyroMode::Other(_) => "Other",
    }
}

/// The id part of a stick's controls (`ls.dz`, `rs.dz` …).
fn stick_id(side: Side) -> &'static str {
    if side == Side::Left {
        "ls"
    } else {
        "rs"
    }
}

fn curve_name(v: Option<i64>) -> &'static str {
    bu_controller::settings::CURVES.iter().find(|(k, _)| Some(*k) == v).map(|(_, l)| *l).unwrap_or("Linear")
}

/// The gyro mode as its menu's value (`W::GyroMode`); a mode the menu doesn't list = None.
fn gyro_mode_v(m: &GyroMode) -> Option<i64> {
    match m {
        GyroMode::Off => Some(0),
        GyroMode::Mouse => Some(1),
        GyroMode::Joystick => Some(2),
        GyroMode::Camera => Some(3),
        GyroMode::Other(_) => None,
    }
}

/// The touch mode as its segment's value (`W::TouchMode`); a touch menu, d-pad, radial menu … = None.
fn touch_mode_v(m: &TouchMode) -> Option<i64> {
    match m {
        TouchMode::Nothing => Some(0),
        TouchMode::Mouse => Some(1),
        TouchMode::Scroll => Some(2),
        TouchMode::Other(_) => None,
    }
}

/// A control's value in a layout's view, as the control reads it (`Some(None)` = not in the file); None = the view has no
/// such part (or it is not a layout value).
fn raw_in(v: &PadView, w: W) -> Option<Option<i64>> {
    Some(match w {
        W::Btn(b, s) => sv(&v.buttons.iter().find(|x| x.id == b)?.settings, s),
        W::Stick(side, s) => sv(&v.sticks.iter().find(|x| x.side == side)?.settings, s),
        W::StickMode(side) => {
            let m = &v.sticks.iter().find(|x| x.side == side)?.mode;
            StickMode::LISTED.iter().position(|x| x == m).map(|i| i as i64)
        }
        W::Trig(side, s) => sv(&v.triggers.iter().find(|x| x.side == side)?.settings, s),
        W::TrigAnalog(side) => Some(if v.triggers.iter().find(|x| x.side == side)?.analog { 0 } else { 1 }),
        W::Gyro(s) => sv(&v.gyro.as_ref()?.settings, s),
        W::GyroMode => gyro_mode_v(&v.gyro.as_ref()?.mode),
        W::Touch(s) => sv(&v.touchpad.as_ref()?.settings, s),
        W::TouchMode => touch_mode_v(&v.touchpad.as_ref()?.touch),
        W::Pref(_) | W::Noise | W::Light => return None,
    })
}

/// A stick's dead zone (inner, outer) in % (Steam's default when not in the file: the drawing's 8 / 100).
fn stick_dz(s: &StickView) -> (f64, f64) {
    (sv(&s.settings, StickSetting::DeadZone).map(radius_to_pct).unwrap_or(8.0), sv(&s.settings, StickSetting::FullAt).map(radius_to_pct).unwrap_or(100.0))
}

/// `at(e)`: the pointer's distance from the circle's centre in % of its radius (viewBox 136 x 118, centre 68 / 59, R 48).
fn well_at(r: (f32, f32, f32, f32), x: f32, y: f32) -> f64 {
    let px = (x - r.0) / r.2.max(1.0) * 136.0 - 68.0;
    let py = (y - r.1) / r.3.max(1.0) * 118.0 - 59.0;
    (px.hypot(py) / 48.0 * 100.0) as f64
}

/// `v` inside the slider's range and, where the range allows, inside `keep` (the other ring's limit). Never panics: the bounds are
/// put in order first (a layout may hold a dead zone over 95 % or a "full at" under 5 %: then the range wins).
fn keep_in(v: f64, (lo, hi): (f64, f64), keep: (f64, f64)) -> f64 {
    let a = keep.0.max(lo).min(hi);
    let b = keep.1.min(hi).max(a);
    v.max(a).min(b)
}

fn snap(v: f64, step: f64) -> f64 {
    (v / step).round() * step
}

fn to_raw(v: f64, c: Conv) -> i64 {
    match c {
        Conv::Radius => bu_controller::settings::pct_to_radius(v),
        Conv::Raw => v.round() as i64,
        Conv::Div100 => (v * 100.0).round() as i64,
        Conv::Unit => v.round() as i64,
    }
}

fn from_raw(r: i64, c: Conv) -> f64 {
    match c {
        Conv::Radius => radius_to_pct(r),
        Conv::Raw | Conv::Unit => r as f64,
        Conv::Div100 => r as f64 / 100.0,
    }
}

fn fmt(v: f64, f: Fmt) -> String {
    match f {
        Fmt::Pct => pct(v),
        Fmt::Ms => format!("{} ms", v.round() as i64),
        Fmt::Deg => format!("{}\u{b0}", v.round() as i64),
        Fmt::Shape => format!("{v:.2}"),
        Fmt::FullPull => {
            if v >= 100.0 {
                "Full pull".into()
            } else {
                pct(v)
            }
        }
    }
}

/// Open a `steam://` link (only from a click, only while Steam runs - the crate refuses otherwise) or start `steam.exe`
/// ("Launch Steam", only from its click) with Windows' shell - off the menu's thread (Order 047: Launch Steam froze the
/// menu black until Steam opened).
fn shell_open(url: &str) {
    crate::offui::shell_open(url);
}

#[cfg(test)]
mod tests;
