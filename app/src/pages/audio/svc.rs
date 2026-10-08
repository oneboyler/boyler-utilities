//! The Audio page's link to crates/audio (bu-audio): its page worker (`AudioPage`: one thread that owns Core Audio and
//! keeps a snapshot the page reads without waiting) - REAL in normal runs, read-only in `--real-read` copies, the FAKE
//! (`SharedFake`, the drawing's setup) in every other test copy. Made when the page opens, dropped when it closes (the
//! thread ends). Changes go to the worker as commands; each returns a `Change` the crate keeps for undo.
//!
//! "Keep my devices" and "New apps volume" work while the menu is CLOSED: bu-audio's `Watcher` (one sleeping thread,
//! Windows' callbacks, no polling). It belongs to the app, not the page: started at app start (`Page::background`, with
//! the saved switches) and kept for the app's life.
//! Test copies never start it.
//!
//! Order 036 (the change log): the worker hands the default-device and device on/off changes back as [`Note`]s (old value
//! read by the crate before the change, names and the new value read right after it); the page writes them with
//! `undo::note`. The values' raw / shown forms and the reset's own Windows calls ([`apply_item`]) are here too.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bu_audio::fake::{dev, session};
use bu_audio::page::{AudioPage, PageSnapshot, Timing};
use bu_audio::service::AudioService;
use bu_audio::{AudioError, AudioOs, Change, Defaults, DeviceKind, DeviceState, FakeOs, Flow, Result, Role, SessionState, SharedFake, VolumeMute, ROLES};

use crate::undo::Val;

/// One change the page asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    /// a device picked in the list = Windows' default (all three roles)
    Default(Flow, String),
    DeviceVolume(String, f32),
    /// a device's own switch in the list
    DeviceOn(Flow, String, bool),
    /// (output device, app group, volume) - unmutes a muted app, as in Windows
    AppVolume(String, String, f32),
    AppMute(String, String, bool),
}

fn apply<O: AudioOs>(s: &mut AudioService<O>, c: Cmd) -> Result<Option<Change>> {
    match c {
        Cmd::Default(f, id) => s.select_default(f, &id).map(Some),
        Cmd::DeviceVolume(id, v) => s.set_device_volume(&id, v).map(Some),
        Cmd::DeviceOn(f, id, on) => s.set_device_on(f, &id, on).map(Some),
        Cmd::AppVolume(out, g, v) => s.set_app_volume(&out, &g, v).map(Some),
        Cmd::AppMute(out, g, m) => s.set_app_mute(&out, &g, m).map(Some),
    }
}

// ------------------------------------------------------------------ Order 036: the change log's items
/// One change-log entry: item id, label, old value, new value.
pub type Note = (String, String, Val, Val);
type Notes = Arc<Mutex<Vec<Note>>>;

pub const KEEP: &str = "keep";
pub const KEEP_LABEL: &str = "Keep my devices";
pub const NEWAPPS: &str = "newapps";
pub const NEWAPPS_LABEL: &str = "New apps volume";

/// Windows' default device of a flow (all three roles). Items sort "dev:" < "in.default" < "out.default": a reset puts a
/// switched-off device back on before it is made the default again.
pub fn def_item(flow: Flow) -> &'static str {
    if flow == Flow::Output {
        "out.default"
    } else {
        "in.default"
    }
}
pub fn def_label(flow: Flow) -> &'static str {
    if flow == Flow::Output {
        "Default output"
    } else {
        "Default input"
    }
}
/// A device's own switch (its endpoint id).
pub fn dev_item(id: &str) -> String {
    format!("dev:{id}")
}
/// An app's volume + mute (its group: the exe path, lower case; "system" = System sounds).
pub fn app_item(group: &str) -> String {
    format!("app:{group}")
}

pub fn on_val(on: bool) -> Val {
    if on {
        Val::new("on", "On")
    } else {
        Val::new("off", "Off")
    }
}

/// "console|multimedia|communications" (ids, empty = none); shown as the device's name, "· calls: <name>" when Windows'
/// communications device is another one.
pub fn defaults_val(d: &Defaults, names: &HashMap<String, String>) -> Val {
    let id = |r: Role| d.get(r).cloned().unwrap_or_default();
    let raw = format!("{}|{}|{}", id(Role::Console), id(Role::Multimedia), id(Role::Communications));
    let name = |s: String| if s.is_empty() { "None".to_string() } else { names.get(&s).cloned().unwrap_or(s) };
    let mut text = name(id(Role::Console));
    if id(Role::Communications) != id(Role::Console) {
        text = format!("{text} \u{00b7} calls: {}", name(id(Role::Communications)));
    }
    Val::new(&raw, &text)
}

pub fn parse_defaults(raw: &str) -> Option<Defaults> {
    let p: Vec<&str> = raw.split('|').collect();
    let [c, m, k] = p.as_slice() else { return None };
    let o = |s: &&str| (!s.is_empty()).then(|| s.to_string());
    let mut d = Defaults::default();
    d.set(Role::Console, o(c));
    d.set(Role::Multimedia, o(m));
    d.set(Role::Communications, o(k));
    Some(d)
}

/// "0.80|0" (volume, muted); shown "80 %" or "Muted".
pub fn app_val(vol: f32, muted: bool) -> Val {
    let raw = format!("{:.2}|{}", vol, u8::from(muted));
    if muted {
        Val::new(&raw, "Muted")
    } else {
        Val::new(&raw, &format!("{} %", (vol * 100.0).round()))
    }
}

pub fn parse_app(raw: &str) -> Option<(f32, bool)> {
    let (v, m) = raw.split_once('|')?;
    Some((v.parse::<f32>().ok()?.clamp(0.0, 1.0), m == "1"))
}

/// New apps volume: "on|0.50" / "off" (the volume of a switched-off rule doesn't matter).
pub fn newapps_val(r: Rules) -> Val {
    if r.new_on {
        Val::new(&format!("on|{:.2}", r.new_vol), &format!("On \u{00b7} {} %", (r.new_vol * 100.0).round()))
    } else {
        Val::new("off", "Off")
    }
}

/// Write one Audio entry into the change log ([`crate::undo::note`]) - only in a running app (it has the services; they
/// may be busy further up the stack, then the note waits for the main loop). A unit test without services writes nothing
/// (its note would wait in the process-wide queue for another test's store).
pub fn log(item: &str, label: &str, old: &Val, new: &Val) {
    if crate::services::in_use() || crate::services::with(|_| ()).is_some() {
        crate::undo::note("aud", item, label, old, new);
    }
}

/// The error the reset shows (the page's own toast words).
pub fn err_text(e: &AudioError) -> String {
    match e {
        AudioError::NeedsAdmin(_) => crate::admin::NOT_CHANGED.to_string(),
        AudioError::LastDeviceOn => "One device always stays on".to_string(),
        AudioError::DeviceOff(n) => format!("{n} is switched off"),
        AudioError::NotFound(_) => "It isn\u{2019}t there now (unplugged, or the app is closed)".to_string(),
        e => e.to_string(),
    }
}

fn names<O: AudioOs>(s: &mut AudioService<O>, flow: Flow) -> HashMap<String, String> {
    s.os_mut().devices(flow).unwrap_or_default().into_iter().map(|d| (d.id, d.name)).collect()
}

/// The change-log entries of one change the worker made: a new default (all roles, before → after) and a device's
/// switch (+ the defaults Windows was moved off it). Volumes are not here: device volume is the everyday volume knob (not
/// logged), app volumes are logged by the page (one entry per drag, not per step).
pub fn notes_for<O: AudioOs>(s: &mut AudioService<O>, c: &Change) -> Vec<Note> {
    match c {
        Change::Default { flow, before } => {
            let now = s.defaults(*flow).unwrap_or_default();
            let n = names(s, *flow);
            vec![(def_item(*flow).into(), def_label(*flow).into(), defaults_val(before, &n), defaults_val(&now, &n))]
        }
        Change::DeviceOn { id, flow, before_on, moved } => {
            let n = names(s, *flow);
            let on_now = s
                .os_mut()
                .devices(*flow)
                .ok()
                .and_then(|d| d.into_iter().find(|d| &d.id == id))
                .map(|d| d.state == DeviceState::On)
                .unwrap_or(*before_on);
            let mut v = Vec::new();
            if on_now != *before_on {
                v.push((dev_item(id), n.get(id).cloned().unwrap_or_else(|| id.clone()), on_val(*before_on), on_val(on_now)));
            }
            if let Some(m) = moved {
                let now = s.defaults(*flow).unwrap_or_default();
                v.push((def_item(*flow).into(), def_label(*flow).into(), defaults_val(m, &n), defaults_val(&now, &n)));
            }
            v
        }
        _ => Vec::new(),
    }
}

/// The reset of one device-side item ("out.default" / "in.default" / "dev:<id>" / "app:<group>") through a service (the
/// fake's, or a real one made for it on its own thread).
pub fn apply_item<O: AudioOs>(s: &mut AudioService<O>, item: &str, to: &Val) -> std::result::Result<(), String> {
    let flow = match item {
        "out.default" => Some(Flow::Output),
        "in.default" => Some(Flow::Input),
        _ => None,
    };
    if let Some(flow) = flow {
        let d = parse_defaults(&to.raw).ok_or("Unknown value")?;
        return s.set_defaults(flow, &d).map_err(|e| err_text(&e));
    }
    if let Some(id) = item.strip_prefix("dev:") {
        let on = to.raw == "on";
        for flow in [Flow::Output, Flow::Input] {
            if s.os_mut().devices(flow).map_err(|e| err_text(&e))?.iter().any(|d| d.id == id) {
                return s.set_device_on(flow, id, on).map(|_| ()).map_err(|e| err_text(&e));
            }
        }
        return Err(err_text(&AudioError::NotFound(id.into())));
    }
    if let Some(group) = item.strip_prefix("app:") {
        let (vol, muted) = parse_app(&to.raw).ok_or("Unknown value")?;
        let out = s.defaults(Flow::Output).map_err(|e| err_text(&e))?.console.ok_or("No output device")?;
        // the volume (it unmutes, as in Windows), then the mute it had
        s.set_app_volume(&out, group, vol).map_err(|e| err_text(&e))?;
        if muted {
            s.set_app_mute(&out, group, true).map_err(|e| err_text(&e))?;
        }
        return Ok(());
    }
    Err("Unknown setting".into())
}

/// The apps on the current output device (the reset's "Windows defaults": every app at 100 %).
pub fn apps_now<O: AudioOs>(s: &mut AudioService<O>) -> Vec<bu_audio::AppRow> {
    let Some(out) = s.defaults(Flow::Output).ok().and_then(|d| d.console) else { return Vec::new() };
    s.apps(&out).unwrap_or_default()
}

/// A real service for one reset call, on its own short-lived thread (Core Audio wants its own COM apartment; the UI thread
/// has another).
#[cfg(windows)]
pub fn on_real<R: Send>(f: impl FnOnce(&mut AudioService<Real>) -> std::result::Result<R, String> + Send) -> std::result::Result<R, String> {
    std::thread::scope(|sc| {
        sc.spawn(|| {
            let os = bu_audio::RealOs::new().map_err(|e| err_text(&e))?;
            let os = crate::admin::proxy::AudioOs::new(os, crate::admin::client::admin());
            f(&mut AudioService::new(os))
        })
        .join()
        .unwrap_or_else(|_| Err("The audio service stopped".into()))
    })
}

/// Windows' audio layer; a device switch Windows answers "access denied" goes to the app's elevated copy (Order 039 -
/// switching a device needs no admin as far as Windows' Sound panel shows, so this is only the fallback).
#[cfg(windows)]
pub type Real = crate::admin::proxy::AudioOs<bu_audio::RealOs>;

pub enum Svc {
    #[cfg(windows)]
    Real(AudioPage<Real>, Notes),
    Fake(AudioPage<SharedFake>, SharedFake, Notes),
}

impl Svc {
    /// `fake` = a test copy (the drawing's setup), `read_only` = a `--real-read` copy (reads Windows, refuses changes).
    pub fn start(fake: bool, read_only: bool) -> Svc {
        #[cfg(windows)]
        if !fake {
            let admin = crate::admin::client::admin();
            let make = move || (if read_only { bu_audio::RealOs::read_only() } else { bu_audio::RealOs::new() }).map(|o| crate::admin::proxy::AudioOs::new(o, admin));
            return Svc::Real(AudioPage::start(make, Timing::default()), Notes::default());
        }
        let _ = read_only;
        let f = SharedFake::new(drawing_fake());
        let f2 = f.clone();
        Svc::Fake(AudioPage::start(move || Ok(f2), Timing::default()), f, Notes::default())
    }

    pub fn snapshot(&self) -> PageSnapshot {
        match self {
            #[cfg(windows)]
            Svc::Real(p, _) => p.snapshot(),
            Svc::Fake(p, ..) => p.snapshot(),
        }
    }

    /// Order 047: the snapshot's write count - the same = nothing new (no copy needed).
    pub fn gen(&self) -> u64 {
        match self {
            #[cfg(windows)]
            Svc::Real(p, _) => p.gen(),
            Svc::Fake(p, ..) => p.gen(),
        }
    }

    fn notes(&self) -> Notes {
        match self {
            #[cfg(windows)]
            Svc::Real(_, n) => n.clone(),
            Svc::Fake(_, _, n) => n.clone(),
        }
    }

    pub fn run(&self, c: Cmd) {
        let n = self.notes();
        match self {
            #[cfg(windows)]
            Svc::Real(p, _) => p.run(move |s| run_noted(s, c, &n)),
            Svc::Fake(p, ..) => p.run(move |s| run_noted(s, c, &n)),
        }
    }

    /// The change-log entries the worker found since the last call (the page writes them with `undo::note`).
    pub fn take_notes(&self) -> Vec<Note> {
        self.notes().lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
    }

    /// The page closes: the worker ends (its queued commands done first), then its last entries.
    pub fn finish(self) -> Vec<Note> {
        let n = self.notes();
        drop(self);
        let v = n.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        v
    }

    /// The fake behind the worker (a test copy).
    pub fn fake(&self) -> Option<SharedFake> {
        match self {
            Svc::Fake(_, f, _) => Some(f.clone()),
            #[cfg(windows)]
            Svc::Real(..) => None,
        }
    }

    /// Levels only while the page shows (the meters).
    pub fn set_levels(&self, on: bool) {
        match self {
            #[cfg(windows)]
            Svc::Real(p, _) => p.set_levels(on),
            Svc::Fake(p, ..) => p.set_levels(on),
        }
    }

    /// The fake's change log (test copies: what the page asked Windows to do).
    pub fn log(&self) -> Vec<String> {
        match self {
            Svc::Fake(_, f, _) => f.with(|f| f.log.clone()),
            #[cfg(windows)]
            Svc::Real(..) => Vec::new(),
        }
    }

    /// Test helper: change the fake behind the worker.
    pub fn with_fake<R>(&self, f: impl FnOnce(&mut FakeOs) -> R) -> Option<R> {
        match self {
            Svc::Fake(_, s, _) => Some(s.with(f)),
            #[cfg(windows)]
            Svc::Real(..) => None,
        }
    }
}

/// One command on the worker, its change-log entries kept for the page.
fn run_noted<O: AudioOs>(s: &mut AudioService<O>, c: Cmd, notes: &Notes) -> Result<Option<Change>> {
    let r = apply(s, c)?;
    if let Some(ch) = &r {
        let v = notes_for(s, ch);
        if let Ok(mut q) = notes.lock() {
            q.extend(v);
        }
    }
    Ok(r)
}

/// The drawing's setup (menu-v22 OUTS / INS / APPS): Headphones (Arctis Nova) 74 % and Microphone (Shure MV7) 90 % in use,
/// the DualSense switched off in both lists; on the headphones Spotifast 64 %, Discord 80 %, VALORANT 72 %, Chrome 100 %
/// muted and System sounds 50 %. Session keys = the names the test hook checks ("set_session_mute spotifast true").
pub fn drawing_fake() -> FakeOs {
    let mut f = FakeOs {
        devices: vec![
            dev("spk", "Speakers (Realtek)", DeviceKind::Speakers, Flow::Output),
            dev("arctis", "Headphones (Arctis Nova)", DeviceKind::Headphones, Flow::Output),
            dev("nv", "Monitor (NVIDIA HD Audio)", DeviceKind::Monitor, Flow::Output),
            dev("ds", "Wireless Controller (DualSense)", DeviceKind::Controller, Flow::Output),
            dev("mv7", "Microphone (Shure MV7)", DeviceKind::Microphone, Flow::Input),
            dev("arctis-mic", "Headset Microphone (Arctis Nova)", DeviceKind::Headphones, Flow::Input),
            dev("c920", "Webcam Microphone (C920)", DeviceKind::Webcam, Flow::Input),
            dev("ds-mic", "Wireless Controller (DualSense)", DeviceKind::Controller, Flow::Input),
        ],
        ..Default::default()
    };
    for d in f.devices.iter_mut() {
        if d.id.starts_with("ds") {
            d.state = DeviceState::Off;
        }
    }
    for r in ROLES {
        f.defaults.insert((Flow::Output, r), "arctis".into());
        f.defaults.insert((Flow::Input, r), "mv7".into());
    }
    for d in &f.devices {
        f.volumes.insert(d.id.clone(), VolumeMute { volume: 0.5, muted: false });
    }
    f.volumes.insert("arctis".into(), VolumeMute { volume: 0.74, muted: false });
    f.volumes.insert("mv7".into(), VolumeMute { volume: 0.90, muted: false });
    let mut chrome = session("chrome", 300, r"C:\Apps\Chrome.exe", SessionState::Active, 1.0);
    chrome.muted = true;
    let mut sys = session("sys", 0, "", SessionState::Inactive, 0.5);
    sys.system = true;
    f.sessions = vec![
        ("arctis".into(), session("spotifast", 100, r"C:\Apps\Spotifast.exe", SessionState::Active, 0.64)),
        ("arctis".into(), session("discord", 200, r"C:\Apps\Discord.exe", SessionState::Active, 0.80)),
        ("arctis".into(), session("valorant", 400, r"C:\Games\VALORANT.exe", SessionState::Active, 0.72)),
        ("arctis".into(), chrome),
        ("arctis".into(), sys),
    ];
    f
}

/// How the drawing paints the five sample apps (APPS: glyph, tile gradient, slider colour + its tint, its fake sound).
pub struct SampleLook {
    pub glyph: &'static str,
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub c2: u32,
    pub mode: &'static str,
}

pub fn sample_look(name: &str) -> Option<SampleLook> {
    let l = |glyph, a, b, c, c2, mode| SampleLook { glyph, a, b, c, c2, mode };
    Some(match name {
        "Spotifast" => l("note", 0x46d989, 0x1c9a5a, 0x2fc46f, 0x86eeb2, "music"),
        "Discord" => l("chat", 0x8f95ff, 0x5a5fe0, 0x7277f6, 0xb4b8ff, "voice"),
        "VALORANT" => l("pad", 0xff7a76, 0xd83f4c, 0xee4f5a, 0xffa29d, "game"),
        "Chrome" => l("globe", 0x5ab4ff, 0x2a74e6, 0x3f95f2, 0x94d0ff, "none"),
        "System sounds" => l("abell", 0xa2abbd, 0x6c7487, 0x8d96a8, 0xc6ccd8, "blip"),
        _ => return None,
    })
}

// ------------------------------------------------------------------ Keep my devices / New apps volume (the app's part)
/// The two switches' values (the drawing's defaults: Keep on; New apps on, 50 %). Kept for the app's life; saved with
/// the settings store (`load_rules` / `set_rules`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rules {
    pub keep: bool,
    pub new_on: bool,
    pub new_vol: f32,
}

impl Default for Rules {
    fn default() -> Self {
        Rules { keep: true, new_on: true, new_vol: 0.5 }
    }
}

thread_local! {
    static RULES: RefCell<Rules> = RefCell::new(Rules::default());
    #[cfg(windows)]
    static WATCH: RefCell<Option<bu_audio::watch::Watcher>> = const { RefCell::new(None) };
}

pub fn rules() -> Rules {
    RULES.with(|r| *r.borrow())
}

/// The saved switches (settings.cfg, page "aud"), read once at app start (`Page::background`).
pub fn load_rules() {
    let r = crate::services::with(|s| {
        let p = crate::settings::Scope::Page("aud");
        let d = Rules::default();
        Rules { keep: s.store.bool_or(p, "keep", d.keep), new_on: s.store.bool_or(p, "new_on", d.new_on), new_vol: s.store.f64_or(p, "new_vol", d.new_vol as f64).clamp(0.0, 1.0) as f32 }
    });
    if let Some(r) = r {
        RULES.with(|x| *x.borrow_mut() = r);
    }
}

/// Tests: the values in memory only (as if the app just started, before `load_rules`).
#[cfg(test)]
pub fn set_rules_mem(r: Rules) {
    RULES.with(|x| *x.borrow_mut() = r);
}

/// New values for the two switches: kept (and saved - the owner's choice survives a restart), and handed to the running
/// watcher.
pub fn set_rules(r: Rules) {
    crate::services::try_with(|s| {
        let p = crate::settings::Scope::Page("aud");
        let _ = s.store.set_bool(p, "keep", r.keep);
        let _ = s.store.set_bool(p, "new_on", r.new_on);
        let _ = s.store.set_f64(p, "new_vol", r.new_vol as f64);
    });
    RULES.with(|x| *x.borrow_mut() = r);
    #[cfg(windows)]
    WATCH.with(|w| {
        if let Some(w) = w.borrow().as_ref() {
            w.set_keep_devices(r.keep);
            w.set_new_apps(r.new_on, r.new_vol);
        }
    });
}

/// Order 036: the two switches' change-log entries for a change made outside a page event (Settings › "Reset the app's
/// own settings"): old → new, only what differs.
pub fn note_rules(old: Rules, new: Rules) {
    if old.keep != new.keep {
        log(KEEP, KEEP_LABEL, &on_val(old.keep), &on_val(new.keep));
    }
    let (o, n) = (newapps_val(old), newapps_val(new));
    if o.raw != n.raw {
        log(NEWAPPS, NEWAPPS_LABEL, &o, &n);
    }
}

/// The watcher runs for the app's life once started (normal runs only; `read_only` = decide but change nothing).
pub fn ensure_watcher(test: bool, read_only: bool) {
    if test {
        return;
    }
    #[cfg(windows)]
    WATCH.with(|w| {
        let mut w = w.borrow_mut();
        if w.is_none() {
            let r = rules();
            *w = Some(bu_audio::watch::Watcher::start(bu_audio::watch::WatchConfig {
                keep_devices: r.keep,
                new_apps: r.new_on,
                new_apps_volume: r.new_vol,
                read_only,
            }));
        }
    });
}
