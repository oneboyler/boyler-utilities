//! Notifications for OBS in the app (Order 035): ClipPing / NotificationsForOBS built in. The engine (crate bu-obs) runs on
//! its own thread while the feature is on; everything it shows comes here, to the app's UI thread, through a hidden window
//! of this module: the popups (popups.rs / gdi.rs / glass.rs), the status icon (status.rs), the tray tooltip line, the
//! page's view, and OBS's keys - kept in step with the keys manager (actions `obs.clip`, `obs.replay`, `obs.record`,
//! `obs.switch`, Raw Input only: like ClipPing it only listens, so OBS and the game still get every key).
//! The feature is OFF until the Add-ons page switches it on (`crate::addons::set("obs", true)`); off = no thread, no
//! window, no key, nothing runs.
//! Settings live in the settings store (page "ntf"); at the first switch-on ClipPing's own NotificationsForOBS.ini is
//! imported if found (read-only; the popup look stays Glass - A_035_01).

pub mod gdi;
pub mod glass;
pub mod mons;
pub mod place;
pub mod popups;
pub mod status;
pub mod takeover;
#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;

use bu_obs::engine::{Input, KeyWhich, KeysView, ObsChange, PopMsg, View};
use bu_obs::keys::{KeyBind, MOD_A, MOD_C, MOD_S, MOD_W};
use bu_obs::monitors::Mon;
use bu_obs::os::ObsOs;
use bu_obs::{Options, Service, Settings};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::keys::{Action, Combo, KeyState, Mods};
use crate::settings::Scope;

/// The page id (the settings store's page scope, the top row's tab).
pub const PAGE: &str = "ntf";
const SCOPE: Scope<'static> = Scope::Page(PAGE);
const WM_OBS: u32 = WM_APP + 0x35;

/// The keys manager's actions: (id, name, which).
pub const ACTIONS: [(&str, &str, KeyWhich); 4] = [
    ("obs.clip", "Save clip", KeyWhich::Clip),
    ("obs.replay", "Instant replay on/off", KeyWhich::Replay),
    ("obs.record", "Recording on/off", KeyWhich::Record),
    ("obs.switch", "Switch scene", KeyWhich::Switch),
];
/// OBS's further Save clip bindings (ClipPing listens to up to 4).
const MORE_CLIP: [&str; 3] = ["obs.clip2", "obs.clip3", "obs.clip4"];

enum Cmd {
    Popup(PopMsg, Option<usize>),
    Publish(View),
    Dialog(String),
    Keys(KeysView),
    Log(String),
}

static Q: Mutex<VecDeque<Cmd>> = Mutex::new(VecDeque::new());
static HW: AtomicIsize = AtomicIsize::new(0);

fn post() {
    let h = HW.load(Ordering::Acquire);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_OBS, WPARAM(0), LPARAM(0));
        }
    }
}

struct UiQ;
impl bu_obs::Ui for UiQ {
    fn popup(&mut self, m: &PopMsg, c: Option<usize>) {
        Q.lock().unwrap().push_back(Cmd::Popup(m.clone(), c));
        post();
    }
    fn publish(&mut self, v: &View) {
        Q.lock().unwrap().push_back(Cmd::Publish(v.clone()));
        post();
    }
    fn dialog(&mut self, t: &str) {
        Q.lock().unwrap().push_back(Cmd::Dialog(t.into()));
        post();
    }
    fn keys(&mut self, k: &KeysView) {
        Q.lock().unwrap().push_back(Cmd::Keys(k.clone()));
        post();
    }
    fn log(&mut self, l: &str) {
        if TESTLOG.load(Ordering::Relaxed) {
            Q.lock().unwrap().push_back(Cmd::Log(l.into()));
            post();
        }
    }
}

static TESTLOG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct Feature {
    svc: Service,
    hwnd: HWND,
    set: Settings,
    view: View,
    mons: Vec<Mon>,
    /// OBS's own keys as last read (save clip, replay, record): the keys manager differs = an edit waiting for Apply
    obs_keys: [Option<KeyBind>; 3],
    /// OBS's keys the keys manager refused (action id -> why)
    key_errors: Vec<(String, String)>,
    dialogs: Vec<String>,
    test: bool,
    log: Vec<String>,
    /// NotificationsForOBS.exe running on its own: (pid)
    other: Option<u32>,
    fake: Option<bu_obs::fake::FakeOs>,
    /// the keys manager follows OBS's keys since the start
    synced: bool,
    /// actions bound to a mouse button now
    mouse: Vec<String>,
    /// OBS's keys still to hand to the keys manager (it was busy)
    pending_keys: Option<KeysView>,
}

thread_local! {
    static F: RefCell<Option<Feature>> = const { RefCell::new(None) };
}

/// Is the feature running?
pub fn running() -> bool {
    F.with(|f| f.borrow().is_some())
}

fn with<R>(f: impl FnOnce(&mut Feature) -> R) -> Option<R> {
    F.with(|c| c.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

// ---------------------------------------------------------------- settings

fn load_settings(s: &crate::services::Services) -> Option<Settings> {
    let pairs = s.store.get_list(SCOPE, "settings")?;
    let scenes = s.store.get_list(SCOPE, "scenes").map(|v| v.to_vec());
    let lookup = |k: &str| pairs.iter().find_map(|p| p.split_once('=').filter(|(a, _)| *a == k).map(|(_, v)| v.to_string()));
    Some(Settings::from_lookup(lookup, scenes))
}

fn save_settings_store(s: &mut crate::services::Services, set: &Settings) {
    let pairs: Vec<String> = set.to_pairs().into_iter().map(|(k, v)| format!("{k}={v}")).collect();
    let _ = s.store.set_list(SCOPE, "settings", &pairs);
    if set.scenes_init {
        let _ = s.store.set_list(SCOPE, "scenes", &set.scenes);
    }
}

/// Settings imported from ClipPing's file (the take-over, Order 043): saved, so the start (or the running feature) uses
/// them.
pub fn save_imported(set: &Settings) {
    if running() {
        set_settings(set.clone());
    } else {
        crate::services::try_with(|s| save_settings_store(s, set));
    }
}

/// ClipPing's settings file on this PC (next to its running exe, or the exe its Start-with-Windows entry starts).
pub fn clipping_ini(os: &dyn ObsOs) -> Option<PathBuf> {
    let mut exes = Vec::new();
    if let Some((_, p)) = os.other_app() {
        exes.push(p);
    }
    if let Some(p) = os.other_app_run_entry() {
        exes.push(p);
    }
    bu_obs::settings::find_clipping_ini(&exes)
}

// ---------------------------------------------------------------- keys

fn combo_of(k: KeyBind) -> Combo {
    let mut m = Mods::NONE;
    if k.mods & MOD_C != 0 {
        m = m.with(Mods::CTRL);
    }
    if k.mods & MOD_A != 0 {
        m = m.with(Mods::ALT);
    }
    if k.mods & MOD_S != 0 {
        m = m.with(Mods::SHIFT);
    }
    if k.mods & MOD_W != 0 {
        m = m.with(Mods::WIN);
    }
    Combo::new(m, k.vk)
}

/// A keys-manager combo as ClipPing's KeyBind.
pub fn bind_of(c: Combo) -> KeyBind {
    let mut m = 0;
    if c.mods.contains(Mods::CTRL) {
        m |= MOD_C;
    }
    if c.mods.contains(Mods::ALT) {
        m |= MOD_A;
    }
    if c.mods.contains(Mods::SHIFT) {
        m |= MOD_S;
    }
    if c.mods.contains(Mods::WIN) {
        m |= MOD_W;
    }
    KeyBind::new(c.vk, m)
}

/// The key an action has in the keys manager now.
pub fn current_key(id: &str) -> Option<KeyBind> {
    crate::services::try_with(|s| match s.keys.state(id) {
        KeyState::Working(c) | KeyState::NotWorking(c, _) => Some(bind_of(c)),
        KeyState::Unbound => None,
    })
    .flatten()
}

fn add_actions(test: bool) {
    let _ = test;
    crate::services::try_with(|s| {
        for (id, name, which) in ACTIONS {
            if s.has_action(id) {
                continue;
            }
            s.add_action(Action::new(id, name, PAGE).with_release().with_extra_mods(), handler(id, which));
        }
    });
}

fn remove_actions() {
    crate::services::try_with(|s| {
        for id in ACTIONS.iter().map(|a| a.0).chain(MORE_CLIP) {
            if s.has_action(id) {
                // the key's saved binding stays out of the way of other features: unbind + remove
                s.remove_action(id);
            }
        }
    });
}

/// Bind `id` to `k` (or unbind) unless it already is; the refusal (if any) is returned.
fn bind(s: &mut crate::services::Services, id: &str, k: Option<KeyBind>) -> Option<String> {
    let cur = match s.keys.state(id) {
        KeyState::Working(c) | KeyState::NotWorking(c, _) => Some(c),
        KeyState::Unbound => None,
    };
    let want = k.map(combo_of);
    if cur == want {
        return None;
    }
    match want {
        None => {
            let _ = s.keys.unbind(&mut s.store, id);
            None
        }
        Some(c) => s.keys.bind(&mut s.store, id, c).err().map(|e| e.message()),
    }
}

/// OBS's keys changed (or the switch key): the keys manager follows. An OBS key the user is editing (keys manager !=
/// OBS's old value) keeps the edit until Apply / Undo.
fn sync_keys(f: &mut Feature, k: &KeysView) {
    let new = [k.clip.first().copied(), k.replay, k.record];
    let old = f.obs_keys;
    let first = !f.synced;
    let mut errs = Vec::new();
    let mut mouse = Vec::new();
    let ran = crate::services::try_with(|s| {
        // a key the keys manager refuses is left unbound (never an old key that would look like an edit)
        let put = |s: &mut crate::services::Services, id: &str, b: Option<KeyBind>, errs: &mut Vec<(String, String)>| {
            if let Some(e) = bind(s, id, b) {
                errs.push((id.to_string(), e));
                let _ = s.keys.unbind(&mut s.store, id);
            }
        };
        for i in 0..3 {
            let id = ACTIONS[i].0;
            let cur = match s.keys.state(id) {
                KeyState::Working(c) | KeyState::NotWorking(c, _) => Some(bind_of(c)),
                KeyState::Unbound => None,
            };
            if first || cur == old[i] || cur.is_none() {
                put(s, id, new[i], &mut errs);
            }
        }
        put(s, "obs.switch", k.switch, &mut errs);
        // ClipPing listens to every Save clip binding of OBS (up to 4)
        for (j, id) in MORE_CLIP.iter().enumerate() {
            let b = k.clip.get(j + 1).copied();
            if b.is_some() && !s.has_action(id) {
                let name = format!("Save clip (OBS's key {})", j + 2);
                s.add_action(Action::new(id, &name, PAGE).with_release().with_extra_mods(), handler(id, KeyWhich::Clip));
            }
            if s.has_action(id) {
                put(s, id, b, &mut errs);
                if b.is_none() {
                    s.remove_action(id);
                }
            }
        }
        for id in ACTIONS.iter().map(|a| a.0).chain(MORE_CLIP) {
            if let KeyState::Working(c) | KeyState::NotWorking(c, _) = s.keys.state(id) {
                if bu_obs::keys::is_mouse(c.vk) {
                    mouse.push(id.to_string());
                }
            }
        }
    })
    .is_some();
    if !ran {
        // the services are busy (a modal picker): again in a moment
        f.pending_keys = Some(k.clone());
        unsafe {
            SetTimer(Some(f.hwnd), 1, 250, None);
        }
        return;
    }
    f.pending_keys = None;
    f.synced = true;
    f.obs_keys = new;
    f.key_errors = errs;
    f.mouse = mouse;
}

/// What a key of the feature does: the press / release to the engine (`mouse` = a mouse button: no release comes).
fn handler(id: &'static str, which: KeyWhich) -> impl FnMut(bool) + 'static {
    move |down| {
        with(|f| {
            let mouse = f.mouse.iter().any(|m| m == id);
            f.svc.send(Input::Key { which, down, mouse });
        });
    }
}

// ---------------------------------------------------------------- the UI-thread window

unsafe extern "system" fn proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_OBS => {
            drain();
            LRESULT(0)
        }
        WM_TIMER => {
            let _ = KillTimer(Some(h), 1);
            with(|f| {
                if let Some(k) = f.pending_keys.take() {
                    sync_keys(f, &k);
                }
            });
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            on_display_change();
            LRESULT(0)
        }
        WM_SETTINGCHANGE if wp.0 == SPI_SETWORKAREA.0 as usize => {
            on_display_change();
            LRESULT(0)
        }
        _ => DefWindowProcW(h, m, wp, lp),
    }
}

fn on_display_change() {
    let test = with(|f| f.test).unwrap_or(true);
    if test {
        return;
    }
    let m = mons::list();
    popups::clear();
    with(|f| {
        f.mons = m.clone();
        f.svc.send(Input::Monitors(m));
    });
}

fn drain() {
    loop {
        let c = Q.lock().unwrap().pop_front();
        let Some(c) = c else { break };
        match c {
            Cmd::Popup(m, clipped) => {
                let st = with(|f| (f.set.clone(), f.mons.clone()));
                if let Some((set, mons)) = st {
                    popups::show(&m, clipped, &set, &mons, false);
                }
            }
            Cmd::Publish(v) => {
                let st = with(|f| {
                    // the engine made the default scene list: the page shows it (not saved until changed)
                    if !f.set.scenes_init {
                        f.set.scenes = v.list.clone();
                    }
                    // "Start OBS with Windows" was ticked in the engine: saved here
                    let changed = f.set.start_obs != v.start_obs;
                    f.set.start_obs = v.start_obs;
                    f.view = v.clone();
                    (f.set.clone(), f.mons.clone(), f.test, changed)
                });
                if let Some((set, mons, test, changed)) = st {
                    if changed {
                        crate::services::try_with(|s| save_settings_store(s, &set));
                    }
                    status::update(&set, &mons, v.connected, v.replay, v.recording, v.clipped, test);
                    crate::tray::set_tip_extra(Some(&v.tip));
                }
                crate::services::Waker.wake();
            }
            Cmd::Dialog(t) => {
                with(|f| f.dialogs.push(t));
                crate::services::Waker.wake();
            }
            Cmd::Keys(k) => {
                with(|f| sync_keys(f, &k));
                crate::services::Waker.wake();
            }
            Cmd::Log(l) => {
                with(|f| f.log.push(l));
            }
        }
    }
}

fn make_window() -> Option<HWND> {
    unsafe {
        static REG: std::sync::Once = std::sync::Once::new();
        REG.call_once(|| {
            let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: GetModuleHandleW(None).unwrap_or_default().into(), lpszClassName: w!("BoylerObsHub"), ..Default::default() };
            RegisterClassW(&wc);
        });
        // a hidden top-level window (not message-only: it must get WM_DISPLAYCHANGE broadcasts)
        CreateWindowExW(WS_EX_TOOLWINDOW, w!("BoylerObsHub"), w!(""), WS_POPUP, 0, 0, 0, 0, None, None, Some(GetModuleHandleW(None).ok()?.into()), None).ok()
    }
}

// ---------------------------------------------------------------- on / off

/// Switch the feature on: settings (imported from ClipPing's file the first time, if found), the engine, the keys.
/// `test` = a test copy: the fake OS (no real OBS, no sound), fake monitors, popups never shown.
pub fn start(test: bool) {
    if running() {
        return;
    }
    let real: Box<dyn ObsOs> = if test {
        let d = std::env::temp_dir().join("Boyler Utilities test").join(format!("obs-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        Box::new(bu_obs::fake::FakeOs::new(&d))
    } else {
        Box::new(bu_obs::real::RealOs::new())
    };
    let fake = if test { Some(bu_obs::fake::FakeOs::new(&real.obs_dir())) } else { None };
    let os: Box<dyn ObsOs> = match &fake {
        Some(f) => Box::new(f.clone()),
        None => real,
    };
    let set = crate::services::try_with(|s| match load_settings(s) {
        Some(set) => set,
        None => {
            // first switch-on: ClipPing's own settings, if it is on this PC (read-only)
            let imported = if test { None } else { clipping_ini(&*os).and_then(|p| bu_obs::settings::import_file(&p)) };
            let set = imported.unwrap_or_default();
            save_settings_store(s, &set);
            set
        }
    });
    // the services must be there (the saved settings, the keys): never start on defaults that a later change would save
    let Some(set) = set else { return };
    // the Glass popups' composition needs a DispatcherQueue on this thread (the menu makes one only when it first opens)
    ensure_dispatcher_queue();
    let mons = if test { bu_obs::monitors::fake("A,1920,1080,0,0,1 B,1920,1080,1920,0,0") } else { mons::list() };
    let other = if test { None } else { os.other_app().map(|o| o.0) };
    popups::set_hidden(test);
    let Some(hwnd) = make_window() else { return };
    HW.store(hwnd.0 as isize, Ordering::Release);
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let svc = Service::start(os, set.clone(), mons.clone(), Box::new(UiQ), Options { watch: !test, exe_dir });
    F.with(|c| {
        *c.borrow_mut() = Some(Feature {
            svc,
            hwnd,
            set,
            view: View::default(),
            mons,
            obs_keys: [None; 3],
            key_errors: vec![],
            dialogs: vec![],
            test,
            log: vec![],
            other,
            fake,
            synced: false,
            mouse: vec![],
            pending_keys: None,
        })
    });
    add_actions(test);
}

/// Switch it off: the engine and every thread / window / key of the feature go.
pub fn stop() {
    let f = F.with(|c| c.borrow_mut().take());
    if let Some(f) = f {
        remove_actions();
        drop(f.svc); // joins the engine thread (and its WebSocket / watcher threads)
        popups::clear();
        status::hide();
        place::close();
        HW.store(0, Ordering::Release);
        unsafe {
            let _ = DestroyWindow(f.hwnd);
        }
        Q.lock().unwrap().clear();
        crate::tray::set_tip_extra(None);
    }
}

// ---------------------------------------------------------------- what the page uses

pub fn view() -> Option<View> {
    with(|f| f.view.clone())
}
pub fn settings() -> Option<Settings> {
    with(|f| f.set.clone())
}
pub fn monitors() -> Vec<Mon> {
    with(|f| f.mons.clone()).unwrap_or_default()
}

/// A setting changed in the page: saved, handed to the engine (popups cleared, as ClipPing does).
pub fn set_settings(set: Settings) {
    let Some(()) = with(|f| {
        f.set = set.clone();
        f.svc.send(Input::Settings(set.clone()));
    }) else {
        return;
    };
    crate::services::try_with(|s| save_settings_store(s, &set));
    popups::clear();
    let st = with(|f| (f.set.clone(), f.mons.clone(), f.view.clone(), f.test));
    if let Some((set, mons, v, test)) = st {
        status::update(&set, &mons, v.connected, v.replay, v.recording, v.clipped, test);
    }
}

/// A Popups setting changed: save it, then ONE silent test popup exactly as it will look.
pub fn popups_changed(set: Settings) {
    set_settings(set);
    if let Some((set, mons, clipped)) = with(|f| (f.set.clone(), f.mons.clone(), f.view.clipped)) {
        popups::test(clipped, &set, &mons);
    }
}

pub fn send(i: Input) {
    with(|f| f.svc.send(i));
}

/// Apply OBS-side changes (the page's Apply bar).
pub fn apply(c: ObsChange) {
    send(Input::Apply(c));
}

/// The next message box the engine asked for.
pub fn take_dialog() -> Option<String> {
    with(|f| if f.dialogs.is_empty() { None } else { Some(f.dialogs.remove(0)) }).flatten()
}

/// OBS's keys the keys manager refused: (action id, why).
pub fn key_errors() -> Vec<(String, String)> {
    with(|f| f.key_errors.clone()).unwrap_or_default()
}

/// OBS's own keys as OBS has them (save clip, replay, record).
pub fn obs_keys() -> [Option<KeyBind>; 3] {
    with(|f| f.obs_keys).unwrap_or_default()
}

/// The "Custom…" placement overlay.
pub fn place_open() {
    if let Some((set, mons, clipped, test)) = with(|f| (f.set.clone(), f.mons.clone(), f.view.clipped, f.test)) {
        if test {
            return;
        }
        let mon = bu_obs::monitors::pick_monitor(&mons, set.where_, clipped);
        place::open(&set, &mons, mon);
    }
}

/// The overlay closed: saved (fx, fy, scale) or cancelled.
pub fn on_place(saved: Option<(i32, i32, i32)>) {
    let Some(mut set) = settings() else { return };
    if let Some((fx, fy, scale)) = saved {
        set.cx = fx;
        set.cy = fy;
        set.scale = scale;
        set.pos = bu_obs::settings::P_CUSTOM;
        popups_changed(set);
    }
    crate::services::Waker.wake();
}

/// NotificationsForOBS.exe running on its own (pid): the page offers to close it (never two apps sounding twice).
pub fn other_app() -> Option<u32> {
    with(|f| f.other).flatten()
}

/// Look again whether it runs (the page opens).
pub fn refresh_other_app() {
    let test = with(|f| f.test).unwrap_or(true);
    if test {
        return;
    }
    let o = bu_obs::real::RealOs::new().other_app().map(|o| o.0);
    with(|f| f.other = o);
}

/// Close it like its own tray menu's Quit.
pub fn close_other_app() {
    let Some(pid) = other_app() else { return };
    let test = with(|f| f.test).unwrap_or(true);
    if !test {
        let mut os = bu_obs::real::RealOs::new();
        os.close_other_app(pid);
    }
    with(|f| f.other = None);
}

/// Test copies: what the engine logged / the popups showed.
pub fn test_log() -> Vec<String> {
    with(|f| f.log.clone()).unwrap_or_default()
}
pub fn fake_os() -> Option<bu_obs::fake::FakeOs> {
    with(|f| f.fake.clone()).flatten()
}
pub fn set_test_log(on: bool) {
    TESTLOG.store(on, Ordering::Relaxed);
}

/// A setting changing continuously (the volume slider while dragged): the engine gets it, the store only at the end.
pub fn set_settings_quiet(set: Settings) {
    with(|f| {
        f.set = set.clone();
        f.svc.send(Input::Settings(set));
    });
}

thread_local! {
    /// the play buttons' sounds (PlaySound reads the memory while it plays)
    static PREVIEW: RefCell<Option<bu_obs::real::RealOs>> = const { RefCell::new(None) };
}

/// A Sounds row's play button (settings.c `sound_preview`): the chosen sound at the chosen volume. Test copies: silent.
pub fn preview_sound(e: usize) {
    let Some((set, test)) = with(|f| (f.set.clone(), f.test)) else { return };
    let ev = [bu_obs::sound::Sound::Saved, bu_obs::sound::Sound::Failed, bu_obs::sound::Sound::Changed, bu_obs::sound::Sound::Warning][e.min(3)];
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let Some((wav, src)) = bu_obs::sound::build(&set, ev, exe_dir.as_deref()) else { return };
    if test {
        with(|f| f.log.push(format!("PREVIEW {e} {src:?} vol={}", set.vol)));
        return;
    }
    PREVIEW.with(|p| {
        let mut p = p.borrow_mut();
        p.get_or_insert_with(bu_obs::real::RealOs::new).play(wav);
    });
}

/// Explorer on a folder ("Open folder"). Order 047: the shell's open runs off the menu's thread (it can take seconds
/// while Explorer starts; the menu kept painting nothing meanwhile).
pub fn open_folder(p: &std::path::Path) {
    crate::offui::shell_open(&p.to_string_lossy());
}

/// Windows' colour picker (Background › Custom…), owned by the menu. Test copies never open it.
/// (Order 047: a modal dialog - it runs its own message loop on the menu's thread while it is up, so it stays here.)
pub fn pick_color(initial: u32, test: bool) -> Option<u32> {
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::UI::Controls::Dialogs::{ChooseColorW, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW};
    if test {
        return None;
    }
    thread_local! {
        static CUSTOM: RefCell<[COLORREF; 16]> = const { RefCell::new([COLORREF(0); 16]) };
    }
    let owner = crate::MENU_HWND.with(|h| *h.borrow());
    CUSTOM.with(|c| {
        let mut c = c.borrow_mut();
        let rgb = COLORREF(((initial >> 16) & 255) | (initial & 0xFF00) | ((initial & 255) << 16));
        let mut cc = CHOOSECOLORW { lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32, hwndOwner: owner, lpCustColors: c.as_mut_ptr(), rgbResult: rgb, Flags: CC_FULLOPEN | CC_RGBINIT, ..Default::default() };
        unsafe {
            if !ChooseColorW(&mut cc).as_bool() {
                return None;
            }
        }
        let r = cc.rgbResult.0;
        Some(((r & 255) << 16) | (r & 0xFF00) | ((r >> 16) & 255))
    })
}

/// Test copies: pretend NotificationsForOBS.exe runs on its own (the page's "close it?" line).
pub fn set_other_for_test(pid: Option<u32>) {
    with(|f| {
        if f.test {
            f.other = pid;
        }
    });
}

thread_local! {
    /// the DispatcherQueue made for the Glass popups, kept for the app's life (the menu uses the thread's queue too)
    static DQ: RefCell<Option<windows::System::DispatcherQueueController>> = const { RefCell::new(None) };
}

/// A DispatcherQueue on the UI thread (Windows.UI.Composition needs one), made once if the thread has none yet.
fn ensure_dispatcher_queue() {
    use windows::Win32::System::WinRT::{CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT};
    if DQ.with(|d| d.borrow().is_some()) {
        return;
    }
    // (fails when the thread has a queue already - the menu's: then that one serves)
    let c = unsafe {
        CreateDispatcherQueueController(DispatcherQueueOptions { dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32, threadType: DQTYPE_THREAD_CURRENT, apartmentType: DQTAT_COM_NONE }).ok()
    };
    DQ.with(|d| *d.borrow_mut() = c);
}
