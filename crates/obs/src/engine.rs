//! The state machine (app.c, ported 1:1): OBS connection + obs-websocket v5 requests / events, the popups and sounds for
//! clip saved / instant replay on-off / recording start-stop / storage / connection, the Save clip key's "press again to
//! turn instant replay on", any key's "OBS isn't open - press again to start it", the Switch scene key (next scene of the
//! user's list, the monitor's resolution, instant replay restarted around a video change), "Keep instant replay on at all
//! times", "Turn off instant replay while recording", applying OBS-side settings (profile keys / clip length / FPS / folder,
//! OBS closed and reopened when needed), turning OBS's remote control on, "Start OBS with Windows".
//!
//! Single-threaded like ClipPing's UI thread: the driver (service.rs) feeds it `Input`s and fires its timers; everything it
//! does to the world goes through `Host`. Timers behave like Win32 SetTimer (they repeat until killed).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::cfg::ObsCfg;
use crate::ctl;
use crate::keys::{binding_json, KeyBind};
use crate::monitors::{self, Mon};
use crate::os::{find_exe, Found, ObsOs};
use crate::settings::Settings;
use crate::sound::Sound;
use crate::ws::WsEvent;

const DOT: &str = " \u{00B7} ";
const GB: u64 = 1024 * 1024 * 1024;
const OUT_STARTED: &str = "OBS_WEBSOCKET_OUTPUT_STARTED";
const OUT_STOPPED: &str = "OBS_WEBSOCKET_OUTPUT_STOPPED";

/// Popup colours (ClipPing C_*): green worked, red didn't, blue changed, amber warning, grey info.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Green,
    Red,
    Blue,
    Amber,
    Grey,
}

/// Popup icons (ClipPing I_*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Check,
    Cross,
    Bang,
    Play,
    Stop,
    Rec,
    Switch,
    Plug,
    PlugX,
    Gear,
    Drive,
}

/// One popup: colour, icon, the two normal lines, and the short title / detail used by the Pill and Tile looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PopMsg {
    pub color: Color,
    pub icon: Icon,
    pub top: String,
    pub main: String,
    pub title: String,
    pub detail: String,
}

impl PopMsg {
    pub fn new(color: Color, icon: Icon, top: &str, main: &str, title: &str, detail: &str) -> Self {
        PopMsg { color, icon, top: top.into(), main: main.into(), title: title.into(), detail: detail.into() }
    }
    /// The sample the settings preview and the test popup show.
    pub fn sample() -> Self {
        PopMsg::new(Color::Green, Icon::Check, "Monitor 1", "Clipped last 60 seconds", "Clipped", "60 s")
    }
}

/// Which of the feature's keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyWhich {
    Clip = 0,
    Replay = 1,
    Record = 2,
    Switch = 3,
}

/// The OBS-side settings the page changes (ClipPing's ObsChange). `fps` = whole frames per second.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ObsChange {
    pub keys_changed: bool,
    /// save clip, replay on/off, recording on/off (None = Not set)
    pub key: [Option<KeyBind>; 3],
    pub cliplen_changed: bool,
    pub cliplen: i32,
    pub fps_changed: bool,
    pub fps: i32,
    pub folder_changed: bool,
    pub folder: PathBuf,
    /// turn on OBS remote control (needs OBS closed)
    pub ws_enable: bool,
}

/// The keys the feature listens to right now (for the app's keys manager).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KeysView {
    /// OBS's Save clip bindings (or ClipPing's fallback key)
    pub clip: Vec<KeyBind>,
    pub replay: Option<KeyBind>,
    pub record: Option<KeyBind>,
    pub switch: Option<KeyBind>,
}

/// One scene as the page lists it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SceneView {
    pub name: String,
    /// the monitor number its display capture shows (0 = none)
    pub mon: i32,
}

/// What the page, the status icon and the tray tooltip show (published after every change).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct View {
    pub connected: bool,
    pub replay: bool,
    pub recording: bool,
    pub scene: String,
    /// the monitor being clipped (index into `mons`)
    pub clipped: Option<usize>,
    /// remote control is off: the amber "Turn on OBS remote control"
    pub amber: bool,
    /// ask "Connect to OBS" (remote control off while OBS is open)
    pub ask_connect: bool,
    pub tip: String,
    pub scenes: Vec<SceneView>,
    /// the user's scene list as in use now (may be the default one made from OBS's scenes)
    pub list: Vec<String>,
    pub cliplen: i32,
    /// hundredths (12000 = 120, 2997 = 29.97)
    pub fps: i32,
    pub folder: PathBuf,
    /// OBS's own keys as in its profile: save clip, instant replay on/off, recording on/off
    pub obs_keys: [Option<KeyBind>; 3],
    pub applying: bool,
    pub start_obs_shown: bool,
    /// the "Start OBS with Windows" setting itself (the tick may also show on because of the user's own shortcut)
    pub start_obs: bool,
    pub mons: Vec<Mon>,
    pub obs_running: bool,
}

/// What the engine does to the world.
pub trait Host {
    fn now_ms(&self) -> u64;
    fn os(&mut self) -> &mut dyn ObsOs;
    /// connect; returns the generation (0 = busy)
    fn ws_start(&mut self, port: u16) -> u32;
    fn ws_busy(&self) -> bool;
    fn ws_send(&mut self, text: &str);
    fn ws_abort(&mut self);
    /// show a popup (the monitor being clipped, for "Other monitor")
    fn popup(&mut self, m: &PopMsg, clipped: Option<usize>);
    fn sound(&mut self, ev: Sound, set: &Settings);
    /// the view changed
    fn publish(&mut self, v: &View);
    /// a message box (ClipPing's dialog_confirm with only OK)
    fn dialog(&mut self, text: &str);
    /// wait for OBS's process to end on another thread, then feed Input::ObsExited
    fn wait_exit(&mut self, pid: u32, timeout_ms: u32);
    /// watch these folders for changes (OBS's settings), feed Input::FilesChanged
    fn watch(&mut self, dirs: Vec<PathBuf>);
    /// the keys to listen to changed
    fn keys(&mut self, k: &KeysView);
    /// a log line (test hook / tests)
    fn log(&mut self, _line: &str) {}
}

#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Ws(WsEvent),
    /// a key went down / up (vk for the release wait; `mouse` = no release will come)
    Key { which: KeyWhich, down: bool, mouse: bool },
    ObsExited(bool),
    FilesChanged,
    Monitors(Vec<Mon>),
    Settings(Settings),
    Apply(ObsChange),
    StartObsToggle,
    /// "Connect to OBS": true = Connect
    Connect(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum T {
    Retry,
    Clip,
    Rescan,
    Cfg,
    Minute,
    Switch,
    Hello,
    SwRetry,
    Oa,
    OaRetry,
    Rbe,
    LaWait,
    RaWait,
    RaStart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum R {
    None,
    Version,
    RecDir,
    RbStat,
    RecStat,
    Mode,
    RbTime,
    CurScene,
    Scenes,
    Items,
    InSet,
    Minute,
    SwRec,
    SwVget,
    SwRb,
    SwStop,
    SwScene,
    SwVideo,
    OaRec,
    OaStop,
    OaSet,
    OaRbStop,
    OaFpsRec,
    OaFps,
    Video,
    RbeStart,
    RbeStat,
}

struct Req {
    kind: R,
    scan: u32,
    arg: String,
}

#[derive(Debug, Clone, Default)]
struct Scene {
    name: String,
    input: String,
    mon_id: String,
    ui: i64,
}

#[derive(Debug, Clone, Default)]
struct Input2 {
    name: String,
    mon_id: String,
}

#[derive(Default)]
struct St {
    gen: u32,
    open: bool,
    ident: bool,
    greeted: bool,
    have_rbstat: bool,
    have_scene: bool,
    scanned: bool,
    rb: bool,
    rec: bool,
    rb_start: u64,
    rec_start: u64,
    rb_sec_ws: i32,
    have_rb_ws: bool,
    rec_dir: PathBuf,
    scene: String,
    fails: i32,
    amber: bool,
    last_saved: u64,
    clip_pending: bool,
    low_warned: bool,
    meas_kbps: i32,
    quiet_stop: bool,
    quiet_start: bool,
    rb_before_rec: bool,
    rec_paused: bool,
    fps_num: i32,
    fps_den: i32,
    quiet_until: u64,
    seq: u64,
    scan_id: u32,
    scan_left: i32,
    sc: Vec<Scene>,
    tsc: Vec<Scene>,
    inputs: Vec<Input2>,
}

#[derive(Default)]
struct Sw {
    active: bool,
    step: Option<R>,
    was_rb: bool,
    reset: bool,
    rb_stopped: bool,
    scene_set: bool,
    w: i32,
    h: i32,
    video_t0: u64,
    orig: String,
    target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Oa {
    #[default]
    Idle,
    RecCheck,
    Stop,
    WaitExit,
    RbStop,
    RbStart,
    FpsRec,
    Fps,
}

#[derive(Default)]
struct OaS {
    step: Oa,
    was_rb: bool,
    fps_t0: u64,
    c: ObsChange,
    pid: u32,
    path: PathBuf,
    last_code: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Rbe {
    #[default]
    None,
    Switch,
    Oa,
    Restore,
    Connect,
    Launch,
    ClipKey,
}

#[derive(Default)]
struct RbeS {
    active: bool,
    purpose: Rbe,
    tries: i32,
    sent: bool,
    t0: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum La {
    #[default]
    Idle,
    Armed,
    Starting,
}

#[derive(Default)]
struct RaS {
    armed: bool,
    waiting: bool,
    t: u64,
    guard_until: u64,
}

const RBE_LIMIT_MS: u64 = 40000;
const LA_AGAIN_MS: u64 = 5000;
const RA_AGAIN_MS: u64 = 5000;
const RA_RELEASE_MS: u64 = 300;
const RA_HOLD_CAP_MS: u64 = 5000;
const RA_GUARD_MS: u64 = 2000;
const MAX_REQ: usize = 96;

pub struct Engine {
    pub cfg: ObsCfg,
    pub set: Settings,
    pub mons: Vec<Mon>,
    s: St,
    sw: Sw,
    oa: OaS,
    rbe: RbeS,
    la: La,
    la_t: u64,
    ra: RaS,
    reqs: HashMap<u64, Req>,
    timers: HashMap<T, (u64, u64)>,
    /// where the running OBS was seen (kept in memory only)
    obs_seen: PathBuf,
    /// OBS starts with Windows through the user's own shortcut / entry (None = not checked)
    obs_own: Option<bool>,
    fast_retry_until: u64,
    restore_rb: bool,
    /// "press again": how long OBS may take to connect
    pub launch_wait: u64,
    watching: bool,
    default_made: bool,
    ask_connect: bool,
    clip_down: bool,
    clip_mouse: bool,
    last_keys: Option<KeysView>,
}

fn jb(v: Option<&Value>) -> bool {
    v.map(|x| x.as_bool().unwrap_or_else(|| x.as_f64().is_some_and(|n| n != 0.0))).unwrap_or(false)
}
fn jn(v: Option<&Value>, def: f64) -> f64 {
    v.and_then(|x| x.as_f64()).unwrap_or(def)
}
fn js<'a>(v: Option<&'a Value>, def: &'a str) -> &'a str {
    v.and_then(|x| x.as_str()).unwrap_or(def)
}

fn fmt_dur(ms: u64) -> String {
    let s = (ms + 500) / 1000;
    let (h, m, sec) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

fn slashes(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

impl Engine {
    pub fn new(set: Settings, mons: Vec<Mon>) -> Self {
        Engine {
            cfg: ObsCfg::default(),
            set,
            mons,
            s: St::default(),
            sw: Sw::default(),
            oa: OaS::default(),
            rbe: RbeS::default(),
            la: La::Idle,
            la_t: 0,
            ra: RaS::default(),
            reqs: HashMap::new(),
            timers: HashMap::new(),
            obs_seen: PathBuf::new(),
            obs_own: None,
            fast_retry_until: 0,
            restore_rb: false,
            launch_wait: 60000,
            watching: false,
            default_made: false,
            ask_connect: false,
            clip_down: false,
            clip_mouse: false,
            last_keys: None,
        }
    }

    // ------------------------------------------------------------------ timers

    fn set_timer(&mut self, h: &dyn Host, t: T, ms: u64) {
        self.timers.insert(t, (h.now_ms() + ms, ms));
    }
    fn kill(&mut self, t: T) {
        self.timers.remove(&t);
    }
    pub fn timer_on(&self, t: T) -> bool {
        self.timers.contains_key(&t)
    }
    /// When the next timer is due (None = none: sleep until an input).
    pub fn next_due(&self) -> Option<u64> {
        self.timers.values().map(|(d, _)| *d).min()
    }
    /// Fire every due timer (repeating ones are re-armed first, like SetTimer).
    pub fn fire_timers(&mut self, h: &mut dyn Host) {
        loop {
            let now = h.now_ms();
            let due = self.timers.iter().filter(|(_, (d, _))| *d <= now).min_by_key(|(_, (d, _))| *d).map(|(t, _)| *t);
            let Some(t) = due else { break };
            if let Some((_, p)) = self.timers.get(&t).copied() {
                self.timers.insert(t, (now + p.max(1), p));
            }
            self.on_timer(h, t);
        }
    }

    // ------------------------------------------------------------------ start

    /// app.c `run`, the OBS part: read OBS's settings, watch them, publish, "Start OBS with Windows" check, the
    /// remote-control check, then try to connect (and every 5 s while OBS is closed - one cheap mutex check).
    pub fn start(&mut self, h: &mut dyn Host) {
        self.cfg = ObsCfg::read(&h.os().obs_dir());
        self.keys_changed(h);
        self.watch_setup(h);
        self.state_changed(h);
        self.startobs_sync(h);
        self.ws_startup_check(h);
        self.try_connect(h);
        self.set_timer(h, T::Retry, 5000);
    }

    /// The feature is being switched off: an "apply OBS settings" that closed OBS and waits for it finishes first (the
    /// profile is written and OBS started again) - else OBS would stay closed.
    pub fn finish_on_stop(&mut self, h: &mut dyn Host) {
        if self.oa.step == Oa::WaitExit {
            let pid = self.oa.pid;
            let ok = h.os().wait_exit(pid, 30000);
            self.oa_obs_exited(h, ok);
        }
    }

    /// One input.
    pub fn input(&mut self, h: &mut dyn Host, i: Input) {
        match i {
            Input::Ws(WsEvent::Open(g)) => {
                if g == self.s.gen {
                    self.s.open = true;
                    self.set_timer(h, T::Hello, 10000);
                }
            }
            Input::Ws(WsEvent::Msg(g, m)) => {
                if g == self.s.gen {
                    self.on_ws_msg(h, &m);
                }
            }
            Input::Ws(WsEvent::Closed(g, code)) => {
                if g == self.s.gen {
                    self.on_ws_closed(h, code);
                }
            }
            Input::Key { which, down, mouse } => {
                if which == KeyWhich::Clip {
                    self.clip_down = down && !mouse;
                    self.clip_mouse = mouse;
                }
                if down {
                    self.handle_key(h, which);
                }
                self.ra_check_release(h);
            }
            Input::ObsExited(ok) => self.oa_obs_exited(h, ok),
            Input::FilesChanged => self.set_timer(h, T::Cfg, 400),
            Input::Monitors(m) => {
                self.mons = m;
                self.state_changed(h);
            }
            Input::Settings(mut s) => {
                // the default scene list stays in use (not saved) until the user makes the list their own
                if self.default_made && s.scenes.is_empty() && !s.scenes_init {
                    s.scenes = self.set.scenes.clone();
                }
                // "Start OBS with Windows" is the engine's own (its tick runs here): a page copy made before the tick's
                // publish arrived must not undo it
                s.start_obs = self.set.start_obs;
                self.set = s;
                self.keys_changed(h);
                self.state_changed(h);
            }
            Input::Apply(c) => self.apply_obs(h, c),
            Input::StartObsToggle => self.startobs_toggle(h),
            Input::Connect(yes) => {
                self.ask_connect = false;
                if yes {
                    self.apply_obs(h, ObsChange { ws_enable: true, ..Default::default() });
                } else {
                    self.amber_once(h);
                }
                self.state_changed(h);
            }
        }
    }

    // ------------------------------------------------------------------ scenes / monitors

    fn scene_find(&self, name: &str) -> Option<&Scene> {
        self.s.sc.iter().find(|s| s.name == name)
    }
    fn scene_mon(&self, s: Option<&Scene>) -> Option<usize> {
        s.filter(|s| !s.mon_id.is_empty()).and_then(|s| monitors::find_obs(&self.mons, &s.mon_id))
    }
    fn cur_mon(&self) -> Option<usize> {
        if self.s.ident {
            self.scene_mon(self.scene_find(&self.s.scene))
        } else {
            None
        }
    }
    fn mon_label(&self) -> String {
        match self.cur_mon() {
            Some(m) => format!("Monitor {}", self.mons[m].num),
            None if !self.s.scene.is_empty() => self.s.scene.clone(),
            None => "OBS".into(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn pop(&mut self, h: &mut dyn Host, color: Color, icon: Icon, top: &str, main: &str, title: &str, detail: &str) {
        let m = PopMsg::new(color, icon, top, main, title, detail);
        h.log(&format!("POPUP {:?} {:?} | {} | {}", color, icon, top, main));
        let cm = self.cur_mon();
        h.popup(&m, cm);
        let ev = match color {
            Color::Green => Some(Sound::Saved),
            Color::Red => Some(Sound::Failed),
            Color::Blue => Some(Sound::Changed),
            Color::Amber => Some(Sound::Warning),
            Color::Grey => None,
        };
        if let Some(e) = ev {
            h.sound(e, &self.set);
        }
    }

    /// Everything the page / status icon / tray show.
    pub fn view(&self, h: &mut dyn Host) -> View {
        let m = self.cur_mon();
        let on = self.s.ident && self.s.rb;
        let tip = if !self.s.ident {
            if self.s.amber {
                format!("Notifications for OBS{DOT}turn on OBS remote control")
            } else {
                format!("Notifications for OBS{DOT}OBS isn't connected")
            }
        } else if let Some(m) = m {
            format!("Clipping monitor {}{DOT}instant replay {}", self.mons[m].num, if on { "on" } else { "off" })
        } else {
            format!("Not clipping a monitor{DOT}instant replay {}", if on { "on" } else { "off" })
        };
        let obs_running = h.os().obs_running();
        View {
            connected: self.s.ident,
            replay: self.s.ident && self.s.rb,
            recording: self.s.ident && self.s.rec,
            scene: self.s.scene.clone(),
            clipped: m,
            amber: self.s.amber,
            ask_connect: self.ask_connect && self.cfg.ws_json && !self.cfg.ws_enabled,
            tip,
            scenes: if self.s.scanned {
                self.s.sc.iter().map(|s| SceneView { name: s.name.clone(), mon: self.scene_mon(Some(s)).map(|i| self.mons[i].num).unwrap_or(0) }).collect()
            } else {
                Vec::new()
            },
            list: self.set.scenes.clone(),
            cliplen: self.clip_max_sec(),
            fps: self.fps(),
            folder: self.folder(),
            obs_keys: [if self.cfg.clip_known { self.cfg.clip.first().copied() } else { None }, self.cfg.rbkey, self.cfg.reckey],
            applying: self.oa.step != Oa::Idle,
            start_obs_shown: self.set.start_obs || self.obs_own == Some(true),
            start_obs: self.set.start_obs,
            mons: self.mons.clone(),
            obs_running,
        }
    }

    fn state_changed(&mut self, h: &mut dyn Host) {
        let v = self.view(h);
        h.publish(&v);
    }

    // ------------------------------------------------------------------ storage

    fn free_bytes(&self, h: &mut dyn Host) -> Option<u64> {
        let dir = if !self.s.rec_dir.as_os_str().is_empty() { self.s.rec_dir.clone() } else { self.cfg.rec_dir.clone() };
        if dir.as_os_str().is_empty() {
            return None;
        }
        h.os().free_bytes(&dir)
    }
    fn rec_kbps(&self) -> i32 {
        if self.s.meas_kbps > 0 {
            self.s.meas_kbps
        } else {
            self.cfg.kbps
        }
    }
    fn minutes_left(&self, h: &mut dyn Host) -> Option<i64> {
        let f = self.free_bytes(h)?;
        let k = self.rec_kbps();
        if k <= 0 {
            return None;
        }
        Some(f as i64 / (k as i64 * 125) / 60) // kbit/s -> bytes/s = *125
    }
    fn storage_low(&self, h: &mut dyn Host) -> bool {
        match self.minutes_left(h) {
            Some(m) => m < 30,
            None => self.free_bytes(h).is_some_and(|f| f < 2 * GB),
        }
    }
    fn storage_full(&self, h: &mut dyn Host) -> bool {
        self.free_bytes(h).is_some_and(|f| f < 512 * 1024 * 1024)
    }
    fn storage_check(&mut self, h: &mut dyn Host) {
        let Some(m) = self.minutes_left(h) else { return };
        if m >= 30 {
            self.s.low_warned = false;
            return;
        }
        if self.s.low_warned {
            return;
        }
        self.s.low_warned = true;
        let t = if m == 1 { format!("About {m} minute of recording left") } else { format!("About {m} minutes of recording left") };
        let d = format!("~{m} min");
        self.pop(h, Color::Amber, Icon::Drive, &t, "Storage almost full", "Storage almost full", &d);
    }

    // ------------------------------------------------------------------ requests

    fn send_req(&mut self, h: &mut dyn Host, kind: R, ty: &str, data: Option<Value>, arg: &str) {
        self.s.seq += 1;
        let id = self.s.seq;
        if self.reqs.len() >= MAX_REQ {
            // table full: the oldest request's answer is dropped
            if let Some(old) = self.reqs.keys().min().copied() {
                self.reqs.remove(&old);
            }
        }
        self.reqs.insert(id, Req { kind, scan: self.s.scan_id, arg: arg.to_string() });
        let mut d = json!({"requestType": ty, "requestId": id.to_string()});
        if let Some(data) = data {
            d["requestData"] = data;
        }
        let msg = json!({"op": 6, "d": d}).to_string();
        h.log(&format!("REQ {ty}"));
        h.ws_send(&msg);
    }
    fn send_param(&mut self, h: &mut dyn Host, kind: R, cat: &str, name: &str) {
        self.send_req(h, kind, "GetProfileParameter", Some(json!({"parameterCategory": cat, "parameterName": name})), "");
    }

    // ------------------------------------------------------------------ clip length / fps / folder

    fn clip_max_sec(&self) -> i32 {
        let s = if self.s.have_rb_ws { self.s.rb_sec_ws } else { self.cfg.rb_sec };
        if s > 0 {
            s
        } else {
            20
        }
    }
    fn clip_real_sec(&self, h: &dyn Host) -> i32 {
        let m = self.clip_max_sec();
        if self.s.rb_start != 0 {
            let el = ((h.now_ms() - self.s.rb_start + 500) / 1000) as i32;
            if el < m {
                return el;
            }
        }
        m
    }
    /// OBS's FPS in hundredths.
    pub fn fps(&self) -> i32 {
        let live = self.s.ident && self.s.fps_num != 0;
        let n = if live { self.s.fps_num } else { self.cfg.fps_num };
        let d = if live { self.s.fps_den } else { self.cfg.fps_den }.max(1);
        if n > 0 {
            (n as i64 * 100 / d as i64) as i32
        } else {
            0
        }
    }
    pub fn folder(&self) -> PathBuf {
        let f = if !self.s.rec_dir.as_os_str().is_empty() { &self.s.rec_dir } else { &self.cfg.rec_dir };
        PathBuf::from(f.to_string_lossy().replace('/', "\\"))
    }

    // ------------------------------------------------------------------ scene scan

    fn scan_start(&mut self, h: &mut dyn Host) {
        self.s.scan_id += 1;
        self.s.tsc.clear();
        self.s.inputs.clear();
        self.send_req(h, R::Scenes, "GetSceneList", None, "");
    }

    fn maybe_greet(&mut self, h: &mut dyn Host) {
        if self.s.greeted || !self.s.ident || !self.s.have_rbstat || !self.s.have_scene || !self.s.scanned {
            return;
        }
        self.s.greeted = true;
        let top = format!("{}{DOT}instant replay {}", self.mon_label(), if self.s.rb { "on" } else { "off" });
        self.pop(h, Color::Grey, Icon::Plug, &top, "OBS connected", "OBS connected", "");
        self.state_changed(h);
    }

    /// First run: one scene per monitor (Monitor 1 first); in use, not saved until the user changes the list.
    fn default_scene_list(&mut self, h: &mut dyn Host) {
        if self.default_made || self.set.scenes_init || !self.set.scenes.is_empty() {
            return;
        }
        let mut list = Vec::new();
        for num in 1..=self.mons.len() as i32 {
            let Some(m) = monitors::by_num(&self.mons, num) else { continue };
            if let Some(s) = self.s.sc.iter().find(|s| self.scene_mon(Some(s)) == Some(m)) {
                list.push(s.name.clone());
            }
        }
        if list.is_empty() {
            return;
        }
        self.default_made = true;
        self.set.scenes = list;
        h.log("SCENES default list (saved only when changed)");
        self.state_changed(h);
    }

    fn scan_finish(&mut self, h: &mut dyn Host) {
        let inputs = self.s.inputs.clone();
        for s in &mut self.s.tsc {
            s.mon_id.clear();
            for i in &inputs {
                if !s.input.is_empty() && s.input == i.name {
                    s.mon_id = i.mon_id.clone();
                }
            }
        }
        // UI order: OBS lists the top scene with the highest sceneIndex (a stable insertion sort, as ClipPing's)
        let mut v = std::mem::take(&mut self.s.tsc);
        for i in 1..v.len() {
            let t = v[i].clone();
            let mut j = i;
            while j > 0 && v[j - 1].ui < t.ui {
                v[j] = v[j - 1].clone();
                j -= 1;
            }
            v[j] = t;
        }
        self.s.sc = v;
        self.s.scanned = true;
        self.default_scene_list(h);
        self.state_changed(h);
        self.maybe_greet(h);
    }

    // ------------------------------------------------------------------ making sure instant replay is running
    // OBS can silently ignore a "start instant replay" request (e.g. while it rebuilds its outputs after a video change):
    // ask, check the real status every second, ask again, for up to 40 s.

    fn rbe_send(&mut self, h: &mut dyn Host) {
        self.rbe.tries += 1;
        self.rbe.sent = true;
        self.send_req(h, R::RbeStart, "StartReplayBuffer", None, "");
    }
    fn rbe_done(&mut self, h: &mut dyn Host, ok: bool) {
        if !self.rbe.active {
            return;
        }
        let p = self.rbe.purpose;
        self.kill(T::Rbe);
        self.rbe.active = false;
        if !ok {
            self.s.quiet_start = false;
        }
        self.rbe_finished(h, p, ok);
    }
    fn rbe_begin(&mut self, h: &mut dyn Host, purpose: Rbe, settle_ms: u64) {
        self.rbe = RbeS { active: true, purpose, tries: 0, sent: false, t0: h.now_ms() };
        if purpose != Rbe::Connect {
            self.s.quiet_start = true;
            self.s.quiet_until = h.now_ms() + RBE_LIMIT_MS + 5000;
        }
        if self.s.rb {
            self.rbe_done(h, true);
            return;
        }
        if settle_ms > 0 {
            self.set_timer(h, T::Rbe, settle_ms);
        } else {
            self.rbe_send(h);
            self.set_timer(h, T::Rbe, 1000);
        }
    }
    fn rbe_tick(&mut self, h: &mut dyn Host) {
        if !self.rbe.active {
            self.kill(T::Rbe);
            return;
        }
        if !self.rbe.sent {
            self.rbe_send(h);
            self.set_timer(h, T::Rbe, 1000);
            return;
        }
        self.send_req(h, R::RbeStat, "GetReplayBufferStatus", None, "");
    }
    fn rbe_status(&mut self, h: &mut dyn Host, ok: bool, r: Option<&Value>) {
        if !self.rbe.active {
            return;
        }
        if ok && jb(r.and_then(|r| r.get("outputActive"))) {
            self.s.rb = true;
            if self.s.rb_start == 0 {
                self.s.rb_start = h.now_ms();
            }
            self.rbe_done(h, true);
        } else if h.now_ms() - self.rbe.t0 >= RBE_LIMIT_MS {
            self.rbe_done(h, false);
        } else {
            self.rbe_send(h);
        }
    }

    fn rbe_finished(&mut self, h: &mut dyn Host, purpose: Rbe, ok: bool) {
        match purpose {
            Rbe::Switch => {
                if ok {
                    self.sw_success(h)
                } else {
                    self.sw_fail(h, "instant replay never started")
                }
            }
            Rbe::Oa if self.oa.step == Oa::RbStart => {
                if ok {
                    self.oa_done(h, "instant replay restarted");
                } else {
                    self.oa.step = Oa::Idle;
                    h.dialog("Instant replay didn't come back on. Please turn it on in OBS.");
                }
            }
            Rbe::ClipKey => {
                if ok {
                    self.ra.guard_until = h.now_ms() + RA_GUARD_MS;
                    self.pop_replay_on(h);
                } else {
                    self.pop(h, Color::Red, Icon::Bang, "OBS didn't start it", "Instant replay still off", "Replay still off", "OBS didn't start it");
                }
            }
            Rbe::Launch => {
                self.la = La::Idle;
                self.kill(T::LaWait);
                if ok {
                    self.pop(h, Color::Blue, Icon::Play, "Instant replay on", "OBS started", "OBS started", "replay on");
                } else {
                    h.log("RBE gave up: instant replay did not start");
                    self.s.greeted = false;
                    self.maybe_greet(h);
                }
            }
            _ => {
                if !ok {
                    h.log("RBE gave up: instant replay did not start");
                }
            }
        }
        self.state_changed(h);
    }

    // ------------------------------------------------------------------ switching

    /// next scene in the user's list after the current one (wraps)
    fn sw_pick_target(&mut self) -> bool {
        let list = &self.set.scenes;
        let n = list.len();
        let cur = list.iter().rposition(|s| *s == self.s.scene);
        for k in 1..=n {
            let i = match cur {
                Some(c) => (c + k) % n,
                None => k - 1,
            };
            if self.scene_find(&list[i]).is_some() && list[i] != self.s.scene {
                self.sw.target = list[i].clone();
                return true;
            }
        }
        false
    }

    fn sw_fail(&mut self, h: &mut dyn Host, why: &str) {
        h.log(&format!("SWITCH failed: {why}"));
        self.kill(T::Switch);
        self.kill(T::SwRetry);
        if self.sw.step == Some(R::SwVideo) && self.sw.scene_set {
            // the resolution couldn't be applied: back to the scene we came from
            let orig = self.sw.orig.clone();
            self.send_req(h, R::None, "SetCurrentProgramScene", Some(json!({"sceneName": orig})), &orig);
            self.s.scene = orig;
        }
        if (self.sw.was_rb || self.set.keep_rb) && !self.s.rb && self.sw.step != Some(R::RbeStart) {
            self.rbe_begin(h, Rbe::Restore, 0);
        }
        self.sw.active = false;
        let top = match self.cur_mon() {
            Some(m) => format!("Still clipping monitor {}", self.mons[m].num),
            None => format!("Still on {}", if self.s.scene.is_empty() { "the same scene" } else { &self.s.scene }),
        };
        self.pop(h, Color::Red, Icon::Bang, &top, "Couldn't switch", "Couldn't switch", "");
        self.state_changed(h);
    }

    fn sw_success(&mut self, h: &mut dyn Host) {
        self.kill(T::Switch);
        self.kill(T::SwRetry);
        self.sw.active = false;
        self.s.scene = self.sw.target.clone();
        let on = self.s.rb;
        let top = format!("{}{DOT}instant replay {}", self.mon_label(), if on { "on" } else { "off" });
        let main = format!("Now clipping {}", self.sw.target);
        let target = self.sw.target.clone();
        h.log("SWITCH ok");
        self.pop(h, Color::Blue, Icon::Switch, &top, &main, "Now clipping", &target);
        self.state_changed(h);
    }

    fn sw_step(&mut self, h: &dyn Host, step: R, timeout_ms: u64) {
        self.sw.step = Some(step);
        self.set_timer(h, T::Switch, timeout_ms);
    }

    fn sw_set_scene(&mut self, h: &mut dyn Host) {
        self.sw_step(h, R::SwScene, 15000);
        let t = self.sw.target.clone();
        self.send_req(h, R::SwScene, "SetCurrentProgramScene", Some(json!({"sceneName": t})), &t);
    }

    fn sw_set_video(&mut self, h: &mut dyn Host) {
        // OBS can freeze for several seconds while it resets video
        self.sw_step(h, R::SwVideo, 40000);
        let (w, hh) = (self.sw.w, self.sw.h);
        self.send_req(h, R::SwVideo, "SetVideoSettings", Some(json!({"baseWidth": w, "baseHeight": hh, "outputWidth": w, "outputHeight": hh})), "");
    }

    fn sw_start_rb(&mut self, h: &mut dyn Host) {
        self.sw.step = Some(R::RbeStart); // ClipPing's R_SW_START
        self.set_timer(h, T::Switch, RBE_LIMIT_MS + 10000);
        let settle = if self.sw.reset { 500 } else { 0 }; // after a video change, give OBS a moment first
        self.rbe_begin(h, Rbe::Switch, settle);
    }

    fn sw_key(&mut self, h: &mut dyn Host) {
        h.log("KEY switch");
        if self.sw.active {
            return;
        }
        if !self.s.ident {
            self.pop(h, Color::Red, Icon::Bang, "OBS isn't open", "Couldn't switch", "Couldn't switch", "OBS isn't open");
            return;
        }
        self.sw = Sw { active: true, orig: self.s.scene.clone(), ..Default::default() };
        self.sw_step(h, R::SwRec, 10000);
        self.send_req(h, R::SwRec, "GetRecordStatus", None, "");
    }

    fn sw_response(&mut self, h: &mut dyn Host, kind: R, ok: bool, r: Option<&Value>, code: i64) {
        if !self.sw.active || Some(kind) != self.sw.step {
            return;
        }
        let g = |k: &str| r.and_then(|r| r.get(k));
        match kind {
            R::SwRec => {
                if ok && jb(g("outputActive")) {
                    self.kill(T::Switch);
                    self.sw.active = false;
                    self.pop(h, Color::Red, Icon::Cross, "Stop recording first", "Can't switch while recording", "Can't switch", "recording");
                    return;
                }
                if !self.sw_pick_target() {
                    self.sw_fail(h, "no other scene in the list");
                    return;
                }
                let m = self.scene_mon(self.scene_find(&self.sw.target));
                self.sw.w = m.map(|m| self.mons[m].w).unwrap_or(0);
                self.sw.h = m.map(|m| self.mons[m].h).unwrap_or(0);
                if self.sw.w == 0 {
                    self.sw_set_scene(h); // not a monitor scene: the resolution stays
                    return;
                }
                self.sw_step(h, R::SwVget, 10000);
                self.send_req(h, R::SwVget, "GetVideoSettings", None, "");
            }
            R::SwVget => {
                let n = |k: &str| jn(g(k), 0.0) as i32;
                self.sw.reset = !ok || n("baseWidth") != self.sw.w || n("baseHeight") != self.sw.h || n("outputWidth") != self.sw.w || n("outputHeight") != self.sw.h;
                if !self.sw.reset {
                    self.sw_set_scene(h); // same size already: no replay restart needed
                    return;
                }
                self.sw_step(h, R::SwRb, 10000);
                self.send_req(h, R::SwRb, "GetReplayBufferStatus", None, "");
            }
            R::SwRb => {
                self.sw.was_rb = ok && jb(g("outputActive"));
                if self.sw.was_rb {
                    self.sw_step(h, R::SwStop, 20000);
                    self.send_req(h, R::SwStop, "StopReplayBuffer", None, "");
                } else {
                    self.sw_set_scene(h);
                }
            }
            R::SwStop => {
                if !ok {
                    self.sw_fail(h, "StopReplayBuffer refused");
                    return;
                }
                if self.sw.rb_stopped || !self.s.rb {
                    self.sw_set_scene(h); // else wait for OBS's STOPPED event
                }
            }
            R::SwScene => {
                if !ok {
                    self.sw_fail(h, "SetCurrentProgramScene refused");
                    return;
                }
                self.sw.scene_set = true;
                self.s.scene = self.sw.target.clone();
                if !self.sw.reset {
                    // keep instant replay on: also after a switch that didn't need a restart
                    if self.set.keep_rb && !self.s.rb {
                        self.sw.was_rb = true;
                        self.sw_start_rb(h);
                        return;
                    }
                    self.sw_success(h);
                    return;
                }
                self.sw.video_t0 = h.now_ms();
                self.sw_set_video(h);
            }
            R::SwVideo => {
                if !ok {
                    // "output still active" right after a stop: try again shortly (only for up to 10 s)
                    if code == 500 && h.now_ms() - self.sw.video_t0 < 10000 {
                        self.set_timer(h, T::SwRetry, 250);
                        return;
                    }
                    self.sw_fail(h, "SetVideoSettings refused");
                    return;
                }
                if self.sw.was_rb || self.set.keep_rb {
                    self.sw_start_rb(h); // keep on at all times: even if it was off before
                } else {
                    self.sw_success(h);
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ OBS events

    fn on_saved(&mut self, h: &mut dyn Host, path: &str) {
        self.s.last_saved = h.now_ms();
        if (self.rbe.active && self.rbe.purpose == Rbe::ClipKey) || h.now_ms() < self.ra.guard_until {
            // OBS saved right as the second Save clip press turned instant replay on: not a clip the user asked for. No
            // popup; the file is left alone.
            self.s.clip_pending = false;
            self.kill(T::Clip);
            h.log("SAVED right after instant replay was turned on by the second press: no popup (file kept)");
            return;
        }
        self.s.clip_pending = false;
        self.kill(T::Clip);
        let ok = !path.is_empty() && h.os().file_exists(Path::new(path));
        if !ok {
            self.pop(h, Color::Red, Icon::Bang, "OBS couldn't save the file", "Clip failed", "Clip failed", "");
            return;
        }
        let n = self.clip_real_sec(h);
        let lab = self.mon_label();
        let main = if n == 1 { format!("Clipped last {n} second") } else { format!("Clipped last {n} seconds") };
        self.pop(h, Color::Green, Icon::Check, &lab, &main, "Clipped", &format!("{n} s"));
    }

    /// blue "Instant replay on" (top: "Monitor N · 60 seconds")
    fn pop_replay_on(&mut self, h: &mut dyn Host) {
        let c = self.clip_max_sec();
        let top = format!("{}{DOT}{c} seconds", self.mon_label());
        self.pop(h, Color::Blue, Icon::Play, &top, "Instant replay on", "Replay on", &format!("{c} s"));
    }

    fn on_event(&mut self, h: &mut dyn Host, ty: &str, e: Option<&Value>) {
        h.log(&format!("EVENT {ty}"));
        let g = |k: &str| e.and_then(|e| e.get(k));
        match ty {
            "ReplayBufferStateChanged" => {
                let st = js(g("outputState"), "");
                let quiet = h.now_ms() < self.s.quiet_until;
                if st == OUT_STARTED {
                    let q = quiet && self.s.quiet_start;
                    let purpose = if self.rbe.active { self.rbe.purpose } else { Rbe::None };
                    self.s.rb = true;
                    self.s.rb_start = h.now_ms();
                    if q {
                        self.s.quiet_start = false;
                    }
                    if self.rbe.active {
                        self.rbe_done(h, true);
                    }
                    if self.sw.active || q || self.la == La::Starting || (purpose != Rbe::None && purpose != Rbe::Connect) {
                        // the app's own restart: its own popup says it
                    } else {
                        self.pop_replay_on(h);
                    }
                    self.oa_rb_event(h, true);
                    self.storage_check(h);
                } else if st == OUT_STOPPED {
                    self.s.rb = false;
                    self.s.rb_start = 0;
                    self.s.clip_pending = false;
                    self.kill(T::Clip);
                    if self.sw.active {
                        self.sw.rb_stopped = true;
                        if self.sw.step == Some(R::SwStop) {
                            self.sw_set_scene(h);
                        }
                    } else if quiet && self.s.quiet_stop {
                        self.s.quiet_stop = false;
                    } else {
                        let lab = self.mon_label();
                        self.pop(h, Color::Blue, Icon::Stop, &lab, "Instant replay off", "Replay off", "");
                    }
                    self.oa_rb_event(h, false);
                }
                self.state_changed(h);
            }
            "ReplayBufferSaved" => {
                let p = js(g("savedReplayPath"), "").to_string();
                self.on_saved(h, &p);
            }
            "RecordStateChanged" => {
                let st = js(g("outputState"), "");
                if st == OUT_STARTED {
                    self.s.rec = true;
                    self.s.rec_start = h.now_ms();
                    self.s.meas_kbps = 0;
                    self.s.rb_before_rec = self.s.rb;
                    self.s.rec_paused = false;
                    let lab = self.mon_label();
                    if self.set.off_rec && (self.s.rb || self.set.keep_rb) {
                        // "Turn off instant replay while recording": one popup, no separate "Instant replay off"
                        self.s.rec_paused = true;
                        if self.s.rb {
                            self.s.quiet_stop = true;
                            self.s.quiet_until = h.now_ms() + 30000;
                            self.send_req(h, R::None, "StopReplayBuffer", None, "");
                        }
                        let top = format!("{lab}{DOT}instant replay paused");
                        self.pop(h, Color::Blue, Icon::Rec, &top, "Recording", "Recording", "");
                    } else {
                        self.pop(h, Color::Blue, Icon::Rec, &lab, "Recording", "Recording", "");
                    }
                    self.storage_check(h);
                    self.set_timer(h, T::Minute, 60000);
                } else if st == OUT_STOPPED {
                    let len = if self.s.rec_start != 0 { h.now_ms() - self.s.rec_start } else { 0 };
                    let restore = self.s.rec_paused && !self.s.rb;
                    self.s.rec = false;
                    self.s.rec_start = 0;
                    self.s.rec_paused = false;
                    self.kill(T::Minute);
                    if restore {
                        self.rbe_begin(h, Rbe::Restore, 0); // recording over: instant replay back on (also after an error)
                    }
                    if self.storage_full(h) {
                        self.pop(h, Color::Red, Icon::Bang, "Your storage is full", "Recording stopped", "Recording stopped", "storage full");
                    } else {
                        let lab = self.mon_label();
                        let d = fmt_dur(len);
                        let top = if restore { format!("{lab}{DOT}{d}{DOT}instant replay on") } else { format!("{lab}{DOT}{d}") };
                        self.pop(h, Color::Green, Icon::Check, &top, "Recording saved", "Recording saved", &d);
                    }
                    self.s.meas_kbps = 0;
                }
                self.state_changed(h);
            }
            "CurrentProgramSceneChanged" => {
                self.s.scene = js(g("sceneName"), "").to_string();
                self.state_changed(h);
            }
            "SceneListChanged" | "SceneCreated" | "SceneRemoved" | "SceneNameChanged" | "SceneItemCreated" | "SceneItemRemoved"
            | "SceneItemEnableStateChanged" | "InputSettingsChanged" | "InputCreated" | "InputRemoved" | "InputNameChanged"
            | "CurrentSceneCollectionChanged" => self.set_timer(h, T::Rescan, 500),
            "CurrentProfileChanged" => self.set_timer(h, T::Cfg, 400),
            _ => {}
        }
    }

    // ------------------------------------------------------------------ applying OBS settings

    /// write the changed values into OBS's files (only while OBS is closed)
    fn oa_write_file(&mut self, h: &mut dyn Host) -> bool {
        if self.oa.c.ws_enable && ctl::enable_ws(h.os()).is_err() {
            self.cfg_reload(h);
            return false;
        }
        self.cfg = ObsCfg::read(&h.os().obs_dir());
        let c = self.oa.c.clone();
        if !c.keys_changed && !c.cliplen_changed && !c.fps_changed && !c.folder_changed {
            self.cfg_reload(h);
            return true;
        }
        if self.cfg.profile_dir.as_os_str().is_empty() {
            h.log("OA no profile folder");
            return false;
        }
        let mut vals: Vec<(String, String, String)> = Vec::new();
        let put = |v: &mut Vec<(String, String, String)>, s: &str, k: &str, val: String| v.push((s.into(), k.into(), val));
        if c.keys_changed {
            let one = |k: Option<KeyBind>| k.and_then(binding_json).unwrap_or_default();
            put(&mut vals, "Hotkeys", "ReplayBuffer", format!("{{\"ReplayBuffer.Save\":[{}]}}", one(c.key[0])));
            // the same key on Start and Stop: OBS uses it as an on/off toggle
            for (k, pair) in [(1usize, ["OBSBasic.StartReplayBuffer", "OBSBasic.StopReplayBuffer"]), (2, ["OBSBasic.StartRecording", "OBSBasic.StopRecording"])] {
                let b = format!("{{\"bindings\":[{}]}}", one(c.key[k]));
                for p in pair {
                    put(&mut vals, "Hotkeys", p, b.clone());
                }
            }
        }
        let out = if self.cfg.adv { "AdvOut" } else { "SimpleOutput" };
        if c.cliplen_changed {
            put(&mut vals, out, "RecRBTime", c.cliplen.to_string());
        }
        if c.fps_changed {
            put(&mut vals, "Video", "FPSType", "1".into());
            put(&mut vals, "Video", "FPSInt", c.fps.to_string());
        }
        if c.folder_changed {
            let key = if self.cfg.adv { if self.cfg.ffmpeg_rec { "FFFilePath" } else { "RecFilePath" } } else { "FilePath" };
            put(&mut vals, out, key, slashes(&c.folder));
        }
        let pd = self.cfg.profile_dir.clone();
        let ok = ctl::write_profile(h.os(), &pd, &vals);
        self.cfg_reload(h);
        ok
    }

    fn oa_done(&mut self, h: &mut dyn Host, how: &str) {
        h.log(&format!("OBSAPPLY done: {how}"));
        self.kill(T::Oa);
        self.oa.step = Oa::Idle;
        self.state_changed(h);
    }

    fn oa_fail(&mut self, h: &mut dyn Host, msg: &str) {
        h.log(&format!("OBSAPPLY failed: {msg}"));
        self.kill(T::Oa);
        if self.oa.was_rb && self.s.ident && !self.s.rb && !self.rbe.active {
            self.rbe_begin(h, Rbe::Restore, 0);
        }
        self.oa.step = Oa::Idle;
        h.dialog(msg);
        self.state_changed(h);
    }

    fn oa_close_obs(&mut self, h: &mut dyn Host) {
        let found = h.os().find_obs();
        let Some((pid, path)) = found else {
            // OBS already gone: just write
            self.oa_write_file(h);
            self.oa_done(h, "OBS was closed, file written");
            return;
        };
        self.oa.pid = pid;
        self.oa.path = path.clone();
        if !path.as_os_str().is_empty() {
            self.obs_seen = path;
        }
        if !h.os().close_obs(pid) {
            self.oa_fail(h, "Couldn't close OBS, so nothing was changed.");
            return;
        }
        self.oa.step = Oa::WaitExit;
        self.kill(T::Oa);
        h.wait_exit(pid, 30000);
    }

    fn oa_obs_exited(&mut self, h: &mut dyn Host, ok: bool) {
        if self.oa.step != Oa::WaitExit {
            return;
        }
        if !ok {
            self.oa_fail(h, "OBS didn't close, so nothing was changed.");
            return;
        }
        if !self.oa_write_file(h) {
            h.log("OBSAPPLY write failed");
        }
        if self.oa.path.as_os_str().is_empty() {
            self.oa.path = h.os().default_path();
        }
        let p = self.oa.path.clone();
        if h.os().start_obs(&p, None, false) {
            self.restore_rb = self.oa.was_rb;
            self.fast_retry_until = h.now_ms() + 60000;
            self.set_timer(h, T::Retry, 1000);
            self.oa_done(h, "OBS restarted");
        } else {
            self.oa_done(h, "OBS could not be started again");
            let m = if self.oa.c.ws_enable && !self.oa.c.keys_changed {
                "OBS remote control was turned on, but OBS didn't start again. Please open OBS."
            } else {
                "The new keys were saved, but OBS didn't start again. Please open OBS."
            };
            h.dialog(m);
        }
    }

    /// FPS: a video reset like a resolution change (OBS may freeze ~8 s), so wait generously
    fn oa_set_fps(&mut self, h: &mut dyn Host) {
        self.oa.step = Oa::Fps;
        if self.oa.fps_t0 == 0 {
            self.oa.fps_t0 = h.now_ms();
        }
        self.set_timer(h, T::Oa, 40000);
        let f = self.oa.c.fps;
        self.send_req(h, R::OaFps, "SetVideoSettings", Some(json!({"fpsNumerator": f, "fpsDenominator": 1})), "");
    }
    fn oa_start_rb(&mut self, h: &mut dyn Host) {
        self.oa.step = Oa::RbStart;
        self.kill(T::Oa); // the restart routine has its own 40 s limit
        let settle = if self.oa.c.fps_changed { 500 } else { 0 };
        self.rbe_begin(h, Rbe::Oa, settle);
    }

    /// replay-buffer events while applying clip length / folder / FPS
    fn oa_rb_event(&mut self, h: &mut dyn Host, started: bool) {
        if self.oa.step == Oa::Stop && !started {
            self.oa_close_obs(h);
            return;
        }
        if self.oa.step == Oa::RbStop && !started {
            if self.oa.c.fps_changed {
                self.oa_set_fps(h);
                return;
            }
            self.oa_start_rb(h);
        }
    }

    fn oa_apply_live(&mut self, h: &mut dyn Host) {
        let c = self.oa.c.clone();
        let out = if self.cfg.adv { "AdvOut" } else { "SimpleOutput" };
        if c.cliplen_changed {
            self.send_req(
                h,
                R::OaSet,
                "SetProfileParameter",
                Some(json!({"parameterCategory": out, "parameterName": "RecRBTime", "parameterValue": c.cliplen.to_string()})),
                "",
            );
            self.s.rb_sec_ws = c.cliplen;
            self.s.have_rb_ws = true;
        }
        if c.folder_changed {
            self.send_req(h, R::OaSet, "SetRecordDirectory", Some(json!({"recordDirectory": slashes(&c.folder)})), "");
            self.s.rec_dir = c.folder.clone();
        }
        if self.s.rb {
            // OBS reads clip length and folder when instant replay starts: restart it
            self.oa.step = Oa::RbStop;
            self.s.quiet_stop = true;
            self.s.quiet_until = h.now_ms() + 30000;
            self.send_req(h, R::OaRbStop, "StopReplayBuffer", None, "");
            self.set_timer(h, T::Oa, 20000);
        } else if c.fps_changed {
            self.oa_set_fps(h);
        } else {
            self.oa_done(h, "applied live");
        }
    }

    fn apply_obs(&mut self, h: &mut dyn Host, c: ObsChange) {
        if self.oa.step != Oa::Idle {
            return;
        }
        self.oa.was_rb = self.s.ident && self.s.rb;
        self.oa.fps_t0 = 0;
        h.log(&format!(
            "OBSAPPLY keys={} cliplen={} folder={} fps={} ws={}",
            c.keys_changed as i32, c.cliplen_changed as i32, c.folder_changed as i32, c.fps_changed as i32, c.ws_enable as i32
        ));
        let ws = c.ws_enable;
        let keys = c.keys_changed;
        let fps = c.fps_changed;
        self.oa.c = c;
        if !h.os().obs_running() {
            // OBS is closed: it reads the file next time it starts
            self.oa_write_file(h);
            self.oa_done(h, "file written (OBS closed)");
            return;
        }
        if ws {
            // remote control is off, so OBS can't be asked to stop anything first: just close it (if something is running,
            // OBS asks "exit anyway?" itself; after 30 s the app gives up)
            self.oa_close_obs(h);
            self.state_changed(h);
            return;
        }
        if keys {
            if self.s.ident {
                self.oa.step = Oa::RecCheck;
                self.set_timer(h, T::Oa, 10000);
                self.send_req(h, R::OaRec, "GetRecordStatus", None, "");
            } else {
                self.oa_close_obs(h);
            }
            self.state_changed(h);
            return;
        }
        if !self.s.ident {
            self.oa_fail(h, "Can't reach OBS right now, so this wasn't changed.");
            return;
        }
        if fps {
            // a new FPS ends a running recording: ask OBS first
            self.oa.step = Oa::FpsRec;
            self.set_timer(h, T::Oa, 10000);
            self.send_req(h, R::OaFpsRec, "GetRecordStatus", None, "");
            self.state_changed(h);
            return;
        }
        self.oa_apply_live(h);
    }

    fn oa_response(&mut self, h: &mut dyn Host, kind: R, ok: bool, r: Option<&Value>) {
        let active = jb(r.and_then(|r| r.get("outputActive")));
        match kind {
            R::OaRec if self.oa.step == Oa::RecCheck => {
                if ok && active {
                    self.oa.step = Oa::Idle;
                    self.kill(T::Oa);
                    h.dialog("Stop recording first \u{2014} changing keys restarts OBS");
                    self.state_changed(h);
                    return;
                }
                if self.s.rb {
                    // stop instant replay first so OBS doesn't ask "outputs are active, exit anyway?"
                    self.oa.step = Oa::Stop;
                    self.set_timer(h, T::Oa, 20000);
                    self.send_req(h, R::OaStop, "StopReplayBuffer", None, "");
                } else {
                    self.oa_close_obs(h);
                }
            }
            R::OaStop if !ok && self.oa.step == Oa::Stop => self.oa_fail(h, "OBS didn't stop instant replay, so nothing was changed."),
            R::OaSet if !ok => h.log("OBSAPPLY a setting was refused by OBS"),
            R::OaFpsRec if self.oa.step == Oa::FpsRec => {
                if ok && active {
                    self.oa.step = Oa::Idle;
                    self.kill(T::Oa);
                    h.dialog("Stop recording first \u{2014} changing FPS would end your recording");
                    self.state_changed(h);
                    return;
                }
                self.oa_apply_live(h);
            }
            R::OaFps if self.oa.step == Oa::Fps => {
                if !ok {
                    // "output still active" right after the stop: retry shortly, for up to 10 s
                    if self.oa.last_code == 500 && h.now_ms() - self.oa.fps_t0 < 10000 {
                        self.set_timer(h, T::OaRetry, 250);
                        return;
                    }
                    self.oa_fail(h, "OBS didn't accept the new FPS, so it wasn't changed.");
                    return;
                }
                self.s.fps_num = self.oa.c.fps;
                self.s.fps_den = 1;
                // back on if it was on, or always with "Keep instant replay on at all times"
                if self.oa.was_rb || self.set.keep_rb {
                    self.oa_start_rb(h);
                } else {
                    self.oa_done(h, "fps set");
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ OBS responses

    fn on_response(&mut self, h: &mut dyn Host, d: &Value) {
        let id: u64 = js(d.get("requestId"), "0").parse().unwrap_or(0);
        let Some(req) = self.reqs.remove(&id).filter(|_| id != 0) else { return };
        let (kind, scan, arg) = (req.kind, req.scan, req.arg);
        let st = d.get("requestStatus");
        let ok = jb(st.and_then(|s| s.get("result")));
        let code = jn(st.and_then(|s| s.get("code")), 0.0) as i64;
        if !ok {
            h.log(&format!("RESP {} failed code={code}", js(d.get("requestType"), "?")));
        }
        let r = d.get("responseData");
        let g = |k: &str| r.and_then(|r| r.get(k));
        match kind {
            R::RbeStat => self.rbe_status(h, ok, r),
            R::RbeStart => {} // the status check decides
            R::Video => {
                if ok {
                    self.s.fps_num = jn(g("fpsNumerator"), 0.0) as i32;
                    self.s.fps_den = jn(g("fpsDenominator"), 1.0) as i32;
                }
                self.state_changed(h);
            }
            R::Version => h.log(&format!("OBS version={} websocket={}", js(g("obsVersion"), "?"), js(g("obsWebSocketVersion"), "?"))),
            R::RecDir => {
                if ok {
                    self.s.rec_dir = PathBuf::from(js(g("recordDirectory"), ""));
                }
                self.state_changed(h);
            }
            R::Mode => {
                let v = g("parameterValue").and_then(|v| v.as_str()).unwrap_or_else(|| js(g("defaultParameterValue"), "Simple")).to_string();
                let cat = if v.eq_ignore_ascii_case("Advanced") { "AdvOut" } else { "SimpleOutput" };
                self.send_param(h, R::RbTime, cat, "RecRBTime");
            }
            R::RbTime => {
                let v = g("parameterValue").and_then(|v| v.as_str()).or_else(|| g("defaultParameterValue").and_then(|v| v.as_str()));
                if let Some(v) = v.filter(|v| ok && crate::ini::atoi(v) > 0) {
                    self.s.rb_sec_ws = crate::ini::atoi(v) as i32;
                    self.s.have_rb_ws = true;
                }
                self.state_changed(h);
            }
            R::RbStat => {
                self.s.rb = ok && jb(g("outputActive"));
                self.s.rb_start = 0; // already running: since when is unknown, assume the full length
                self.s.have_rbstat = true;
                if self.la == La::Starting {
                    h.log("LAUNCHKEY connected: making sure instant replay is on");
                    self.s.greeted = true; // the "OBS started" popup replaces "OBS connected"
                    self.set_timer(h, T::LaWait, RBE_LIMIT_MS + 10000);
                    self.rbe_begin(h, Rbe::Launch, 0);
                } else if !self.s.rb && (self.restore_rb || self.set.keep_rb) {
                    // "Keep instant replay on at all times" (or the app restarted OBS while it was on)
                    h.log("AUTOREPLAY starting instant replay");
                    self.rbe_begin(h, Rbe::Connect, 0);
                }
                self.restore_rb = false;
                self.maybe_greet(h);
                self.state_changed(h);
            }
            R::RecStat => {
                if ok && jb(g("outputActive")) {
                    self.s.rec = true;
                    self.s.rec_start = h.now_ms().saturating_sub(jn(g("outputDuration"), 0.0) as u64);
                    self.set_timer(h, T::Minute, 60000);
                    self.state_changed(h);
                }
            }
            R::Minute => {
                if ok && jb(g("outputActive")) {
                    let bytes = jn(g("outputBytes"), 0.0);
                    let dur = jn(g("outputDuration"), 0.0);
                    if dur > 20000.0 && bytes > 0.0 {
                        self.s.meas_kbps = (bytes * 8.0 / dur) as i32;
                    }
                    self.storage_check(h);
                }
            }
            R::CurScene => {
                let n = g("sceneName").and_then(|v| v.as_str()).unwrap_or_else(|| js(g("currentProgramSceneName"), ""));
                self.s.scene = n.to_string();
                self.s.have_scene = true;
                self.maybe_greet(h);
            }
            R::Scenes => {
                if scan != self.s.scan_id {
                    return;
                }
                if let Some(arr) = g("scenes").and_then(|v| v.as_array()).filter(|_| ok) {
                    for c in arr.iter().take(48) {
                        self.s.tsc.push(Scene { name: js(c.get("sceneName"), "").into(), ui: jn(c.get("sceneIndex"), 0.0) as i64, ..Default::default() });
                    }
                }
                self.s.scan_left = self.s.tsc.len() as i32;
                if self.s.tsc.is_empty() {
                    self.scan_finish(h);
                    return;
                }
                let names: Vec<String> = self.s.tsc.iter().map(|s| s.name.clone()).collect();
                for n in names {
                    self.send_req(h, R::Items, "GetSceneItemList", Some(json!({"sceneName": n})), &n);
                }
            }
            R::Items => {
                if scan != self.s.scan_id {
                    return;
                }
                if let Some(items) = g("sceneItems").and_then(|v| v.as_array()).filter(|_| ok) {
                    if let Some(si) = self.s.tsc.iter().rposition(|s| s.name == arg) {
                        for c in items {
                            // last visible display capture in the list = the top-most one
                            if js(c.get("inputKind"), "") != "monitor_capture" || !c.get("sceneItemEnabled").map(|v| jb(Some(v))).unwrap_or(true) {
                                continue;
                            }
                            self.s.tsc[si].input = js(c.get("sourceName"), "").to_string();
                        }
                        let inp = self.s.tsc[si].input.clone();
                        if !inp.is_empty() && !self.s.inputs.iter().any(|i| i.name == inp) && self.s.inputs.len() < 24 {
                            self.s.inputs.push(Input2 { name: inp, mon_id: String::new() });
                        }
                    }
                }
                self.s.scan_left -= 1;
                if self.s.scan_left > 0 {
                    return;
                }
                if self.s.inputs.is_empty() {
                    self.scan_finish(h);
                    return;
                }
                self.s.scan_left = self.s.inputs.len() as i32;
                let names: Vec<String> = self.s.inputs.iter().map(|i| i.name.clone()).collect();
                for n in names {
                    self.send_req(h, R::InSet, "GetInputSettings", Some(json!({"inputName": n})), &n);
                }
            }
            R::InSet => {
                if scan != self.s.scan_id {
                    return;
                }
                if ok {
                    let id = r.and_then(|r| r.get("inputSettings")).and_then(|s| s.get("monitor_id")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    for i in &mut self.s.inputs {
                        if i.name == arg {
                            i.mon_id = id.clone();
                        }
                    }
                }
                self.s.scan_left -= 1;
                if self.s.scan_left <= 0 {
                    self.scan_finish(h);
                }
            }
            R::OaRec | R::OaStop | R::OaSet | R::OaRbStop | R::OaFpsRec | R::OaFps => {
                self.oa.last_code = code;
                self.oa_response(h, kind, ok, r);
            }
            _ => self.sw_response(h, kind, ok, r, code),
        }
    }

    // ------------------------------------------------------------------ connection

    fn amber_once(&mut self, h: &mut dyn Host) {
        if self.s.amber {
            return;
        }
        self.s.amber = true;
        self.pop(h, Color::Amber, Icon::Gear, "OBS \u{2192} Tools \u{2192} WebSocket Server Settings", "Turn on OBS remote control", "Turn on OBS remote control", "");
        self.state_changed(h);
    }

    fn remember_obs_path(&mut self, h: &mut dyn Host) {
        if let Some((_, p)) = h.os().find_obs() {
            if !p.as_os_str().is_empty() {
                self.obs_seen = p;
            }
        }
    }

    fn known_obs_path(&self) -> PathBuf {
        if !self.obs_seen.as_os_str().is_empty() {
            self.obs_seen.clone()
        } else {
            PathBuf::from(&self.set.obs_path)
        }
    }

    // ---- "Start OBS with Windows" = the app's shortcut to OBS in the user's Startup folder; if OBS already starts with
    // Windows through the user's own shortcut / entry, the app adds nothing and the tick shows on.

    /// On but the app's shortcut is missing: add it as soon as OBS's path is known (a running OBS, or where it was seen).
    fn startobs_sync(&mut self, h: &mut dyn Host) {
        if !self.set.start_obs {
            return;
        }
        let own = h.os().user_autostart();
        self.obs_own = Some(own);
        if own {
            // the user's own shortcut starts OBS: the app's shortcut would open it twice
            if h.os().shortcut_exists() {
                h.os().shortcut_delete();
            }
            return;
        }
        if h.os().shortcut_exists() {
            return;
        }
        let k = self.known_obs_path();
        match find_exe(h.os(), Some(&k)) {
            Some((p, Found::Running | Found::Known)) => {
                h.os().shortcut_create(&p);
            }
            _ => h.log("OBSSTART shortcut missing: added once OBS connects"),
        }
    }

    /// The user clicked the tick.
    fn startobs_toggle(&mut self, h: &mut dyn Host) {
        let own = h.os().user_autostart();
        self.obs_own = Some(own);
        if own {
            h.dialog("OBS starts with Windows through a shortcut you made. Remove it from your Startup folder to turn this off.");
            return;
        }
        if self.set.start_obs {
            if !h.os().shortcut_delete() {
                h.dialog("Couldn't remove OBS from your Startup folder.");
                return;
            }
            self.set.start_obs = false;
        } else {
            let k = self.known_obs_path();
            let Some((p, _)) = find_exe(h.os(), Some(&k)) else {
                h.dialog("Open OBS once, then tick this again.");
                return;
            };
            if !h.os().shortcut_create(&p) {
                h.dialog("Couldn't add OBS to your Startup folder.");
                return;
            }
            self.set.start_obs = true;
        }
        self.state_changed(h);
    }

    fn on_identified(&mut self, h: &mut dyn Host) {
        h.log("CONNECTED");
        self.s.ident = true;
        self.s.fails = 0;
        self.s.amber = false;
        self.kill(T::Hello);
        self.kill(T::Retry);
        self.fast_retry_until = 0;
        self.send_req(h, R::Version, "GetVersion", None, "");
        self.send_req(h, R::RecDir, "GetRecordDirectory", None, "");
        self.send_param(h, R::Mode, "Output", "Mode");
        self.send_req(h, R::RbStat, "GetReplayBufferStatus", None, "");
        self.send_req(h, R::RecStat, "GetRecordStatus", None, "");
        self.send_req(h, R::CurScene, "GetCurrentProgramScene", None, "");
        self.send_req(h, R::Video, "GetVideoSettings", None, "");
        self.scan_start(h);
        self.remember_obs_path(h);
        self.startobs_sync(h);
    }

    fn on_ws_msg(&mut self, h: &mut dyn Host, m: &str) {
        let Ok(v) = serde_json::from_str::<Value>(m) else { return };
        let op = jn(v.get("op"), -1.0) as i64;
        let d = v.get("d");
        match op {
            0 => {
                // Hello -> Identify. Events: General | Config | Scenes | Inputs | Outputs | SceneItems (no high-volume ones)
                let mut id = json!({"rpcVersion": 1, "eventSubscriptions": 207});
                let au = d.and_then(|d| d.get("authentication"));
                if let Some(au) = au {
                    let a = crate::auth::obs_auth(&self.cfg.ws_password, js(au.get("salt"), ""), js(au.get("challenge"), ""));
                    id["authentication"] = json!(a);
                }
                h.log(&format!("HELLO auth={}", au.is_some() as i32));
                h.ws_send(&json!({"op": 1, "d": id}).to_string());
            }
            2 => self.on_identified(h),
            5 => {
                if let Some(d) = d {
                    let ty = js(d.get("eventType"), "").to_string();
                    self.on_event(h, &ty, d.get("eventData"));
                }
            }
            7 => {
                if let Some(d) = d {
                    self.on_response(h, d);
                }
            }
            _ => {}
        }
    }

    fn cfg_reload(&mut self, h: &mut dyn Host) {
        let old = std::mem::take(&mut self.cfg);
        self.cfg = ObsCfg::read(&h.os().obs_dir());
        self.keys_changed(h);
        self.state_changed(h);
        if old.profile_dir != self.cfg.profile_dir {
            self.watch_setup(h);
        }
    }

    fn try_connect(&mut self, h: &mut dyn Host) {
        if self.s.open || self.s.ident || h.ws_busy() {
            return;
        }
        if !h.os().obs_running() {
            // OBS not open: stay idle, just this cheap check every 5 s
            if self.s.amber {
                self.s.amber = false;
                self.state_changed(h);
            }
            self.s.fails = 0;
            return;
        }
        self.cfg_reload(h);
        if !self.watching {
            self.watch_setup(h);
        }
        if self.cfg.ws_known && !self.cfg.ws_enabled {
            if self.oa.step == Oa::Idle {
                self.amber_once(h);
            }
            return;
        }
        let port = if self.cfg.ws_known { self.cfg.ws_port } else { 4455 };
        self.s.gen = h.ws_start(port);
    }

    fn on_ws_closed(&mut self, h: &mut dyn Host, code: u16) {
        let (was, rec, mut amber, mut fails) = (self.s.ident, self.s.rec, self.s.amber, self.s.fails);
        h.log(&format!("DISCONNECTED code={code}"));
        if code == 0 {
            fails += 1;
            if fails >= 2 && h.os().obs_running() && h.now_ms() > self.fast_retry_until {
                self.amber_once(h);
                amber = self.s.amber;
            }
        } else if code == 4009 {
            h.log("AUTH failed (password in OBS config doesn't match)");
        }
        if was && rec {
            self.pop(h, Color::Red, Icon::Bang, "OBS closed", "Recording stopped", "Recording stopped", "OBS closed");
        }
        let (seq, scan) = (self.s.seq, self.s.scan_id);
        self.s = St { seq, scan_id: scan, amber, fails, ..Default::default() };
        self.sw = Sw::default();
        self.rbe = RbeS::default();
        self.ra = RaS::default();
        self.reqs.clear();
        for t in [T::RaWait, T::RaStart, T::Rbe, T::Clip, T::Minute, T::Switch, T::SwRetry, T::Hello, T::Rescan] {
            self.kill(t);
        }
        self.state_changed(h);
        let ms = if h.now_ms() < self.fast_retry_until { 1000 } else { 5000 };
        self.set_timer(h, T::Retry, ms);
    }

    /// OBS's remote control off at start (A_035_01): OBS closed = turned on now (password protected), used when OBS opens;
    /// OBS open = the page asks "Connect to OBS" (the menu), and the amber popup says it meanwhile.
    fn ws_startup_check(&mut self, h: &mut dyn Host) {
        if !self.cfg.ws_json || self.cfg.ws_enabled {
            return;
        }
        if !h.os().obs_running() {
            if ctl::enable_ws(h.os()).is_ok() {
                self.cfg_reload(h);
                self.pop(h, Color::Grey, Icon::Plug, "Password protected \u{00B7} takes effect when OBS opens", "OBS remote control turned on", "Remote control on", "");
            }
            return;
        }
        h.log("WSCONNECT asking");
        self.ask_connect = true;
        self.amber_once(h);
        self.state_changed(h);
    }

    // ------------------------------------------------------------------ keys

    fn clip_binds(&self) -> Vec<KeyBind> {
        if self.cfg.clip_known {
            return self.cfg.clip.clone();
        }
        self.set.clip_fallback.into_iter().collect()
    }

    fn keys_changed(&mut self, h: &mut dyn Host) {
        let k = KeysView { clip: self.clip_binds(), replay: self.cfg.rbkey, record: self.cfg.reckey, switch: self.set.switch_key };
        if self.last_keys.as_ref() != Some(&k) {
            self.last_keys = Some(k.clone());
            h.keys(&k);
        }
    }

    /// the second press's key went up (or was held too long): turn instant replay on shortly after
    fn ra_start(&mut self, h: &mut dyn Host) {
        self.kill(T::RaStart);
        if !self.ra.waiting {
            return;
        }
        self.ra.waiting = false;
        if !self.s.ident {
            return;
        }
        h.log(&format!("CLIPKEY turning instant replay on ({} ms after the second press)", h.now_ms() - self.ra.t));
        self.rbe_begin(h, Rbe::ClipKey, 0);
    }
    fn ra_check_release(&mut self, h: &mut dyn Host) {
        if !self.ra.waiting || self.clip_down {
            return;
        }
        self.kill(T::RaWait);
        h.log("CLIPKEY key let go");
        self.set_timer(h, T::RaStart, RA_RELEASE_MS);
    }

    fn clip_key(&mut self, h: &mut dyn Host) {
        h.log("KEY clip");
        if !self.s.ident {
            if h.os().obs_running() {
                // OBS is open but we can't talk to it, so we can't confirm anything
                self.pop(h, Color::Amber, Icon::Gear, "OBS \u{2192} Tools \u{2192} WebSocket Server Settings", "Turn on OBS remote control", "Turn on OBS remote control", "");
            } else {
                self.pop(h, Color::Red, Icon::PlugX, "OBS isn't open", "Nothing was saved", "Not saved", "OBS isn't open");
            }
            return;
        }
        if !self.s.rb && self.s.rec && (self.s.rec_paused || self.set.off_rec) {
            // recording with "Turn off instant replay while recording": everything is recorded anyway (grey, no sound)
            self.pop(h, Color::Grey, Icon::Rec, "You're recording", "It's all in the recording", "It's all in the recording", "");
            return;
        }
        if !self.s.rb {
            if self.ra.waiting || (self.rbe.active && self.rbe.purpose == Rbe::ClipKey) {
                h.log("CLIPKEY ignored: instant replay is being turned on");
                return;
            }
            if self.ra.armed && h.now_ms() - self.ra.t < RA_AGAIN_MS {
                // second press: turn instant replay on (confirmed); nothing is saved. Not yet: first wait until the key is
                // let go (OBS handles the same key itself and would save a ~1 s clip)
                self.ra.armed = false;
                if self.rbe.active {
                    h.log("CLIPKEY second press: instant replay is already being turned on");
                    return;
                }
                self.ra.waiting = true;
                self.ra.t = h.now_ms();
                h.log("CLIPKEY second press: waiting until the key is let go");
                self.set_timer(h, T::RaWait, RA_HOLD_CAP_MS);
                return; // the release check runs right after this key input
            }
            self.ra.armed = true;
            self.ra.t = h.now_ms();
            self.pop(h, Color::Red, Icon::Cross, "Instant replay is off \u{00B7} press again to turn it on", "Nothing was saved", "Not saved", "press again for replay");
            return;
        }
        self.ra.armed = false;
        if h.now_ms() - self.s.last_saved < 400 {
            return; // OBS already announced this save
        }
        self.s.clip_pending = true;
        self.set_timer(h, T::Clip, 2000);
    }

    fn la_start(&mut self, h: &mut dyn Host, p: &Path) {
        self.la = La::Starting;
        self.la_t = h.now_ms();
        h.log(&format!("LAUNCHKEY starting OBS: {}", p.display()));
        self.cfg_reload(h);
        // remote control off: turn it on now, while OBS is still closed (always password protected)
        if self.cfg.ws_json && !self.cfg.ws_enabled && ctl::enable_ws(h.os()).is_ok() {
            self.cfg_reload(h);
        }
        if !h.os().start_obs(p, Some("--minimize-to-tray --startreplaybuffer"), true) {
            self.la = La::Idle;
            self.pop(h, Color::Red, Icon::PlugX, "It may still be opening", "Couldn't start OBS", "Couldn't start OBS", "may still be opening");
            return;
        }
        let w = self.launch_wait;
        self.set_timer(h, T::LaWait, w);
        self.fast_retry_until = h.now_ms() + w;
        self.set_timer(h, T::Retry, 1000);
    }

    /// true = the press was used here (OBS not running, or being started)
    fn la_key(&mut self, h: &mut dyn Host, which: KeyWhich) -> bool {
        if self.la == La::Starting {
            h.log("LAUNCHKEY ignored: OBS is being started");
            return true;
        }
        if self.s.ident || h.os().obs_running() {
            self.la = La::Idle;
            return false;
        }
        let k = self.known_obs_path();
        let Some((p, _)) = find_exe(h.os(), Some(&k)) else {
            h.log("LAUNCHKEY no OBS exe found");
            self.la = La::Idle;
            // save clip and switch show their usual "OBS isn't open"
            return matches!(which, KeyWhich::Replay | KeyWhich::Record);
        };
        if self.la == La::Armed && h.now_ms() - self.la_t < LA_AGAIN_MS {
            self.la_start(h, &p);
            return true;
        }
        self.la = La::Armed;
        self.la_t = h.now_ms();
        if which == KeyWhich::Clip {
            self.pop(h, Color::Red, Icon::PlugX, "OBS isn't open \u{00B7} press again to start it", "Nothing was saved", "Not saved", "press again to start OBS");
        } else {
            self.pop(h, Color::Red, Icon::PlugX, "Press again to start it", "OBS isn't open", "OBS isn't open", "press again");
        }
        true
    }

    fn handle_key(&mut self, h: &mut dyn Host, which: KeyWhich) {
        if matches!(which, KeyWhich::Replay | KeyWhich::Record) {
            h.log(if which == KeyWhich::Replay { "KEY replay" } else { "KEY record" });
        }
        if self.la_key(h, which) {
            return;
        }
        match which {
            KeyWhich::Clip => self.clip_key(h),
            KeyWhich::Switch => self.sw_key(h),
            _ => {} // instant replay / recording keys: OBS handles those itself
        }
    }

    // ------------------------------------------------------------------ files

    fn watch_setup(&mut self, h: &mut dyn Host) {
        // user.ini / global.ini (active profile) + every profile's basic.ini (the hotkeys live there)
        let mut dirs = vec![h.os().obs_dir()];
        if let Some(root) = self.cfg.profiles_root() {
            dirs.push(root);
        }
        self.watching = true;
        h.watch(dirs);
    }

    // ------------------------------------------------------------------ timers

    fn on_timer(&mut self, h: &mut dyn Host, t: T) {
        match t {
            T::Retry => {
                if self.fast_retry_until != 0 && h.now_ms() > self.fast_retry_until {
                    self.fast_retry_until = 0;
                    self.set_timer(h, T::Retry, 5000);
                }
                self.try_connect(h);
            }
            T::Hello => {
                self.kill(T::Hello);
                if !self.s.ident {
                    h.ws_abort();
                }
            }
            T::Rescan => {
                self.kill(T::Rescan);
                if self.s.ident {
                    self.scan_start(h);
                }
            }
            T::Cfg => {
                self.kill(T::Cfg);
                self.cfg_reload(h);
                if self.s.ident {
                    self.send_param(h, R::Mode, "Output", "Mode");
                }
            }
            T::Minute => {
                if self.s.ident && self.s.rec {
                    self.send_req(h, R::Minute, "GetRecordStatus", None, "");
                }
            }
            T::Switch => {
                self.kill(T::Switch);
                if self.sw.active {
                    self.sw_fail(h, "timeout");
                }
            }
            T::SwRetry => {
                self.kill(T::SwRetry);
                if self.sw.active && self.sw.step == Some(R::SwVideo) {
                    self.sw_set_video(h);
                }
            }
            T::Rbe => self.rbe_tick(h),
            T::OaRetry => {
                self.kill(T::OaRetry);
                if self.oa.step == Oa::Fps {
                    self.oa_set_fps(h);
                }
            }
            T::Oa => {
                self.kill(T::Oa);
                if self.oa.step == Oa::RbStart || self.oa.step == Oa::RbStop {
                    self.oa_done(h, "instant replay restart not confirmed");
                } else if self.oa.step != Oa::Idle && self.oa.step != Oa::WaitExit {
                    self.oa_fail(h, "OBS didn't answer, so nothing was changed.");
                }
            }
            T::RaWait => {
                self.kill(T::RaWait);
                if self.ra.waiting {
                    h.log("CLIPKEY key still held after 5 s: going ahead");
                    self.ra_start(h);
                }
            }
            T::RaStart => self.ra_start(h),
            T::LaWait => {
                self.kill(T::LaWait);
                if self.la == La::Starting {
                    self.la = La::Idle;
                    if !self.s.ident {
                        h.log("LAUNCHKEY OBS didn't connect in time");
                        self.pop(h, Color::Red, Icon::PlugX, "It may still be opening", "Couldn't start OBS", "Couldn't start OBS", "may still be opening");
                    }
                }
            }
            T::Clip => {
                self.kill(T::Clip);
                if !self.s.clip_pending {
                    return;
                }
                self.s.clip_pending = false;
                if self.storage_low(h) {
                    self.pop(h, Color::Red, Icon::Bang, "Your storage is low", "Clip failed", "Clip failed", "storage low");
                } else {
                    self.pop(h, Color::Red, Icon::Bang, "OBS couldn't save the file", "Clip failed", "Clip failed", "");
                }
            }
        }
    }
}

/// What a set of OBS-side changes needs confirming (app.c `app_obs_needs`): (OBS restarts, instant replay restarts).
pub fn needs(c: &ObsChange, obs_running: bool, connected: bool, replay_on: bool) -> (bool, bool) {
    let restart_obs = c.keys_changed && obs_running;
    let restart_rb = !restart_obs && (c.cliplen_changed || c.folder_changed || c.fps_changed) && connected && replay_on;
    (restart_obs, restart_rb)
}
