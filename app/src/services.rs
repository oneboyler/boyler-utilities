//! The app's long-lived services (Order 014 item 2), alive from start to quit - the menu window comes and goes, these
//! stay (a key must work with the menu closed):
//! - the settings store (`settings/`): %APPDATA%\Boyler Utilities\settings.cfg; a test copy uses a scratch folder under
//!   %TEMP% that is deleted at quit;
//! - the keys manager (`keys/`): the REAL layer (RegisterHotKey on the message window + Raw Input through bu-rawin) in
//!   normal runs, the FAKE one in every test copy (a test never registers a real key); its key-field capture;
//! - the job runner (`jobs/`): it owns the process's one `Clicks`, so a job starts only from a click being delivered;
//! - the actions' handlers (what a key does), registered by the pages at start (`Page::start`).
//!
//! All on the UI thread (one thread-local); pages reach it through their `Cx` (ui/cx.rs), never directly.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::jobs::{Clicks, JobError, JobId, JobRunner, JobView, StartError};
use crate::keys::{fake::FakeKeysOs, real::RealKeysOs, Action, BindError, Capture, Combo, KeyEvent, KeyState, KeysManager, KeysOs, Mods, Step};
use crate::settings::{GlassStyle, Scope, SettingsStore};

/// The message a finished / progressing job posts to the message window (the menu repaints).
pub const WM_JOB: u32 = WM_APP + 7;
/// bu-rawin (the Raw Input thread) has key packets / mouse buttons for the keys manager (one per batch; Order 048).
pub const WM_RAWKEYS: u32 = WM_APP + 8;

/// A job or a key changed something the menu shows (read and cleared by the menu each frame).
static DIRTY: AtomicBool = AtomicBool::new(false);
/// the message window (the wake-ups go there)
static MSG_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// A page asked the app to end for the self-update's install step (`Cx::exit_for_update`) - an app-level flag, so it works
/// from a page's `build` / `event` / `popup` alike and with the menu closed meanwhile (REVIEW_014_item1c HOLD 1).
static EXIT: AtomicBool = AtomicBool::new(false);

/// End the app (main.rs's loop checks it after every wake-up; the message posted here is that wake-up).
pub fn request_exit() {
    EXIT.store(true, Ordering::Release);
    let h = MSG_HWND.load(Ordering::Acquire);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_JOB, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn exit_requested() -> bool {
    EXIT.load(Ordering::Acquire)
}

/// A key handler (or any thread) asked for the menu on a tab: (page id, target for its `Page::jump`).
static SHOW_MENU: std::sync::Mutex<Option<(String, Option<String>)>> = std::sync::Mutex::new(None);

/// Open the menu on the tab `page_id` and hand it `target` (`Page::jump`) - e.g. the Voice to text key with the menu
/// closed: `services::show_menu("vtt", Some("listen"))` from its `add_action` handler. Open already: the tab is shown
/// (the drawing's `jump`). Callable from any thread; the main loop does it at its next wake-up (posted here).
pub fn show_menu(page_id: &str, target: Option<&str>) {
    if let Ok(mut m) = SHOW_MENU.lock() {
        *m = Some((page_id.to_string(), target.map(str::to_string)));
    }
    let h = MSG_HWND.load(Ordering::Acquire);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_JOB, WPARAM(0), LPARAM(0));
        }
    }
}

/// main.rs: the menu someone asked for (once).
pub fn take_show_menu() -> Option<(String, Option<String>)> {
    SHOW_MENU.lock().ok().and_then(|mut m| m.take())
}

/// One keys-manager action as Settings › All shortcuts lists it (`Cx::actions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionInfo {
    pub id: String,
    /// what the user sees ("Mic mute")
    pub name: String,
    /// the owner page's id ("aud")
    pub page: String,
    /// the key as the key field writes it ("Ctrl + Shift + M"); None = no key
    pub key: Option<String>,
    /// set, but Windows refused it at start-up (another app holds it): the reason
    pub not_working: Option<String>,
}

/// A page's waker (`Env::waker()`): a background thread of the page (a ping, a speed test, a live controller view) calls
/// `wake()` when it has something new; the menu rebuilds the page once - no frames while nothing changes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Waker;

impl Waker {
    pub fn wake(&self) {
        DIRTY.store(true, Ordering::Release);
        let h = MSG_HWND.load(Ordering::Acquire);
        if h != 0 {
            unsafe {
                let _ = PostMessageW(Some(HWND(h as *mut _)), WM_JOB, WPARAM(0), LPARAM(0));
            }
        }
    }
}

impl KeysOs for Box<dyn KeysOs> {
    fn register(&mut self, slot: i32, combo: Combo) -> Result<(), String> {
        (**self).register(slot, combo)
    }
    fn unregister(&mut self, slot: i32) {
        (**self).unregister(slot)
    }
    fn raw_devices(&mut self, keyboard: bool, mouse: bool) -> Result<(), String> {
        (**self).raw_devices(keyboard, mouse)
    }
    fn mods_now(&self) -> Option<Mods> {
        (**self).mods_now()
    }
    fn key_name(&self, vk: u16) -> String {
        (**self).key_name(vk)
    }
}

/// A key field listening for a key: which action, since when (the ring's breathing), what the user holds, the last refusal.
pub struct Listening {
    pub action: String,
    pub since: f64,
    pub capture: Capture,
    pub error: Option<String>,
}

pub struct Services {
    pub store: SettingsStore,
    pub keys: KeysManager<Box<dyn KeysOs>>,
    pub jobs: JobRunner,
    clicks: Clicks,
    handlers: HashMap<String, Box<dyn FnMut(bool)>>,
    pub listening: Option<Listening>,
    /// the last refusal of a key field, per action (shown under the field until the next try)
    pub key_errors: HashMap<String, String>,
    /// Order 045: when a key field last took a key / last refused one (`timing::now`) - the field's keycap drop-in and its
    /// shake (only one field listens at a time)
    pub key_bound_at: Option<f64>,
    pub key_refused_at: Option<f64>,
    scratch: Option<PathBuf>,
    pub test: bool,
}

thread_local! {
    static S: RefCell<Option<Services>> = const { RefCell::new(None) };
}

/// Start the services (once, at app start). `hwnd` = the message window (WM_HOTKEY, WM_RAWKEYS, job wake-ups).
pub fn init(hwnd: HWND, test: bool) {
    init_with(Some(hwnd), test);
}

/// The services with no window (the uninstaller's `--undo-windows`, Order 036): the settings store - the fake keys layer
/// (no key is registered), no page's keys or handlers.
pub fn init_headless(test: bool) {
    init_with(None, test);
}

fn init_with(hwnd: Option<HWND>, test: bool) {
    let (folder, scratch) = if test {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join("Boyler Utilities test").join(format!("{}-{}", std::process::id(), n));
        (d.clone(), Some(d))
    } else {
        (SettingsStore::default_folder().unwrap_or_else(|| std::env::temp_dir().join("Boyler Utilities")), None)
    };
    let mut store = SettingsStore::open(folder);
    // Order 050: the app's writes go to one background writer - a change never waits for the disk on the UI thread (unit
    // tests keep writing at once: they read the file back right after)
    if !cfg!(test) {
        store.write_in_background();
    }
    let os: Box<dyn KeysOs> = match hwnd {
        Some(hwnd) if !test => Box::new(RealKeysOs::new(hwnd)),
        _ => Box::new(FakeKeysOs::new()),
    };
    let keys = KeysManager::new(os, &store);
    let h = hwnd.map(|h| h.0 as isize).unwrap_or(0);
    if hwnd.is_some() {
        MSG_HWND.store(h, Ordering::Release);
    }
    let jobs = JobRunner::new(move || {
        DIRTY.store(true, Ordering::Release);
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_JOB, WPARAM(0), LPARAM(0));
        }
    });
    let clicks = take_clicks();
    let mut s = Services { store, keys, jobs, clicks, handlers: HashMap::new(), listening: None, key_errors: HashMap::new(), key_bound_at: None, key_refused_at: None, scratch, test };
    // every page registers its keys and what they do (they work with the menu closed)
    // (none with no window: the uninstaller's undo needs no keys)
    if hwnd.is_some() {
        for p in crate::pages::all() {
            p.start(&mut s);
        }
    }
    S.with(|c| *c.borrow_mut() = Some(s));
}

/// The process's one `Clicks` (unit tests run several services, one per test thread).
#[cfg(not(test))]
fn take_clicks() -> Clicks {
    Clicks::take().expect("one Clicks per process")
}
#[cfg(test)]
fn take_clicks() -> Clicks {
    Clicks::for_test()
}

/// Stop: every job stopped and joined, every key unregistered, a test copy's scratch folder removed.
pub fn shutdown() {
    let s = S.with(|c| c.borrow_mut().take());
    if let Some(s) = s {
        let scratch = s.scratch.clone();
        drop(s);
        if let Some(d) = scratch {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

/// Use the services (None before `init` / after `shutdown`, e.g. in unit tests of pages).
pub fn with<R>(f: impl FnOnce(&mut Services) -> R) -> Option<R> {
    S.with(|c| c.borrow_mut().as_mut().map(f))
}

/// `with`, but None (instead of a panic) when the services are already in use further up the stack - e.g. inside a key
/// handler (`add_action`), which runs while the keys manager holds them.
pub fn try_with<R>(f: impl FnOnce(&mut Services) -> R) -> Option<R> {
    S.with(|c| c.try_borrow_mut().ok().and_then(|mut b| b.as_mut().map(f)))
}

/// The services are in use further up the stack right now (a key handler runs).
pub fn in_use() -> bool {
    S.with(|c| c.try_borrow_mut().is_err())
}

/// Did a job or a key change something the menu shows? (clears it)
pub fn take_dirty() -> bool {
    DIRTY.swap(false, Ordering::AcqRel)
}

impl Services {
    /// A page's key: its action and what it does (down = true on press, false on release for release-watched keys).
    /// Called from `Page::start`.
    pub fn add_action(&mut self, action: Action, handler: impl FnMut(bool) + 'static) {
        self.handlers.insert(action.id.clone(), Box::new(handler));
        self.keys.add_action(action);
    }

    /// An action added while the app runs (a timer's own key): is it there already?
    pub fn has_action(&self, id: &str) -> bool {
        self.handlers.contains_key(id)
    }

    /// The action goes (a timer was deleted): its key is unbound and freed, its handler dropped.
    pub fn remove_action(&mut self, id: &str) {
        if self.listening.as_ref().is_some_and(|l| l.action == id) {
            self.stop_listening();
        }
        let _ = self.keys.unbind(&mut self.store, id);
        self.keys.remove_action(id);
        self.handlers.remove(id);
        self.key_errors.remove(id);
    }

    /// Tests: what a key press does (the handler the page registered), without Windows.
    #[cfg(test)]
    pub fn fire_for_test(&mut self, id: &str, down: bool) {
        self.fire(id, down);
    }

    fn fire(&mut self, id: &str, down: bool) {
        if let Some(h) = self.handlers.get_mut(id) {
            h(down);
        }
        DIRTY.store(true, Ordering::Release);
    }

    /// Start a job - only with the token of the click being delivered (`Cx::start_job` passes it during a click).
    pub(crate) fn start_job<F>(&mut self, key: &str, work: F) -> Result<JobId, StartError>
    where
        F: FnOnce(&crate::jobs::JobCtx) -> Result<String, JobError> + Send + 'static,
    {
        let pressed = self.clicks.press();
        self.jobs.start(pressed, key, work)
    }

    pub fn job(&self, key: &str) -> Option<JobView> {
        self.jobs.view_key(key)
    }

    // ---- key fields
    /// Start listening for a new key for `action` (the field's click). Keys are paused meanwhile (a key being typed
    /// must not fire its old action).
    pub fn listen(&mut self, action: &str, now: f64) {
        self.keys.pause();
        self.key_errors.remove(action);
        self.listening = Some(Listening { action: action.to_string(), since: now, capture: Capture::new(), error: None });
    }

    pub fn stop_listening(&mut self) {
        if self.listening.take().is_some() {
            self.keys.resume();
        }
    }

    /// Clear an action's key (the field's ×).
    pub fn clear_key(&mut self, action: &str) {
        let _ = self.keys.unbind(&mut self.store, action);
    }

    fn step(&mut self, st: Step) {
        let Some(l) = self.listening.as_mut() else { return };
        match st {
            Step::Done(combo) => {
                let id = l.action.clone();
                match self.keys.check(&id, combo) {
                    Ok(()) => {
                        self.listening = None;
                        self.keys.resume();
                        if let Err(e) = self.keys.bind(&mut self.store, &id, combo) {
                            self.key_errors.insert(id, e.message());
                        } else {
                            self.key_bound_at = Some(crate::timing::now());
                        }
                    }
                    Err(e) => {
                        // refused (used elsewhere / types a character): the field keeps listening, as in the drawing
                        self.key_refused_at = Some(crate::timing::now());
                        l.error = Some(e.message());
                        self.key_errors.insert(id, e.message());
                        l.capture.restart();
                    }
                }
            }
            Step::Cancelled => self.stop_listening(),
            Step::Cleared => {
                let id = l.action.clone();
                self.stop_listening();
                let _ = self.keys.unbind(&mut self.store, &id);
            }
            Step::Refused(e) => {
                self.key_refused_at = Some(crate::timing::now());
                let m = refusal(&e);
                l.error = Some(m.clone());
                let id = l.action.clone();
                self.key_errors.insert(id, m);
            }
            Step::Listening { .. } | Step::Ignored => {}
        }
        DIRTY.store(true, Ordering::Release);
    }

    /// What a key field for `action` shows: (keys text or None, listening, the held modifiers' text, since).
    pub fn field(&self, action: &str) -> (Option<String>, Option<(Option<String>, f64)>) {
        let set = match self.keys.state(action) {
            KeyState::Working(c) | KeyState::NotWorking(c, _) => Some(self.keys.combo_text(c)),
            KeyState::Unbound => None,
        };
        let listening = self.listening.as_ref().filter(|l| l.action == action).map(|l| {
            let held = l.capture.held();
            ((!held.is_empty()).then(|| self.keys.held_text(held)), l.since)
        });
        (set, listening)
    }

    /// Every action the pages added (in the order they were added), with its key.
    pub fn action_list(&self) -> Vec<ActionInfo> {
        self.keys
            .actions()
            .map(|a| {
                let (key, not_working) = match self.keys.state(&a.id) {
                    KeyState::Working(c) => (Some(self.keys.combo_text(c)), None),
                    KeyState::NotWorking(c, why) => (Some(self.keys.combo_text(c)), Some(why)),
                    KeyState::Unbound => (None, None),
                };
                ActionInfo { id: a.id.clone(), name: a.name.clone(), page: a.page.clone(), key, not_working }
            })
            .collect()
    }

    /// The glass style (Settings › Glass style).
    pub fn glass(&self) -> GlassStyle {
        self.store.glass()
    }

    /// The theme choice (Settings › Theme).
    pub fn theme(&self) -> crate::settings::Theme {
        self.store.theme()
    }

    /// Settings › "Reset the app's own settings": the app's and the pages' own settings back to how the app came (the
    /// "how your PC was" records stay); keys reloaded.
    pub fn reset_app_settings(&mut self) {
        let _ = self.store.reset_app_settings();
        self.keys.reload(&self.store);
        DIRTY.store(true, Ordering::Release);
    }

    /// A page setting (the page's own scope).
    pub fn page_scope(page: &str) -> Scope<'_> {
        Scope::Page(page)
    }
}

fn refusal(e: &BindError) -> String {
    e.message()
}

// ------------------------------------------------------------------ window messages (main.rs calls these)
/// A key message to the menu window while a key field listens: true = taken (the menu does nothing else with it).
pub fn key_message(down: bool, vk: u16, lparam: isize) -> bool {
    with(|s| {
        if s.listening.is_none() {
            return false;
        }
        let mods = crate::keys::real::mods_now();
        let mut ev = KeyEvent::new(vk, mods);
        if lparam & (1 << 24) != 0 {
            ev = ev.extended();
        }
        if down && lparam & (1 << 30) != 0 {
            ev = ev.repeat();
        }
        let st = {
            let l = s.listening.as_mut().unwrap();
            if down {
                l.capture.key_down(ev)
            } else {
                l.capture.key_up(ev)
            }
        };
        s.step(st);
        true
    })
    .unwrap_or(false)
}

/// A mouse button 3 / 4 / 5 on the menu window while a key field listens.
pub fn mouse_button(button: u8) -> bool {
    with(|s| {
        if s.listening.is_none() {
            return false;
        }
        let mods = crate::keys::real::mods_now();
        let st = s.listening.as_mut().unwrap().capture.mouse_down(button, mods);
        s.step(st);
        true
    })
    .unwrap_or(false)
}

/// WM_HOTKEY on the message window.
pub fn hotkey(wparam: usize) {
    with(|s| {
        if let Some(id) = s.keys.action_for_slot(wparam as i32).map(|x| x.to_string()) {
            s.fire(&id, true);
        }
    });
}

/// WM_RAWKEYS on the message window (keys watched with Raw Input: mouse buttons, modifier-only keys, release keys): the
/// packets bu-rawin kept since the last one - key packets, mouse buttons / wheel; a mouse move never comes here.
/// true = something was fed to the keys manager.
pub fn raw_packets() -> bool {
    let packets = bu_rawin::take_packets();
    if packets.is_empty() {
        return false;
    }
    with(|s| {
        let mut fired: Vec<(String, bool)> = Vec::new();
        for p in packets {
            s.keys.on_raw(p.into(), |id, down| fired.push((id.to_string(), down)));
            for (id, down) in fired.drain(..) {
                s.fire(&id, down);
            }
        }
    })
    .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::Gfx;
    use crate::ui::cx::{Cx, State};
    use crate::undo::{Kind, Resettable, Review, Val};

    fn start() {
        init(HWND::default(), true);
    }

    #[test]
    fn a_test_copy_uses_a_scratch_folder_and_removes_it() {
        start();
        let dir = with(|s| s.store.folder().to_path_buf()).unwrap();
        assert!(dir.starts_with(std::env::temp_dir()), "{dir:?}");
        assert!(with(|s| s.test).unwrap());
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st).for_page("dsp");
        cx.set_bool("fast", true);
        cx.set_str("preset", "1920 x 1080");
        assert!(cx.get_bool("fast", false));
        assert_eq!(cx.get_str("preset", ""), "1920 x 1080");
        assert!(dir.join("settings.cfg").exists());
        drop(cx);
        shutdown();
        assert!(!dir.exists(), "scratch folder left behind");
    }

    struct FakePage {
        applied: Vec<(String, String)>,
    }
    impl Resettable for FakePage {
        fn page_id(&self) -> &str {
            "dsp"
        }
        fn page_title(&self) -> &str {
            "Display"
        }
        fn apply(&mut self, item: &str, to: &Val) -> Result<(), String> {
            self.applied.push((item.to_string(), to.raw.clone()));
            Ok(())
        }
    }

    #[test]
    fn a_recorded_change_comes_back_in_the_reset_review() {
        start();
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st).for_page("dsp");
        cx.record("hz", "Refresh rate", &Val::new("144", "144 Hz"), &Val::new("60", "60 Hz"));
        cx.record("hz", "Refresh rate", &Val::new("60", "60 Hz"), &Val::new("120", "120 Hz"));
        drop(cx);
        let mut page = FakePage { applied: Vec::new() };
        let review = with(|s| Review::for_page(Kind::HowItWas, &page, &s.store)).unwrap();
        assert_eq!(review.lines.len(), 1);
        assert_eq!(review.lines[0].to.raw, "144", "how the PC was = the first old value");
        let res = with(|s| review.apply(&mut s.store, &mut [&mut page])).unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(page.applied, vec![("hz".to_string(), "144".to_string())]);
        shutdown();
    }

    #[test]
    fn a_job_never_starts_outside_a_click() {
        start();
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st).for_page("net");
        // the open / build path: no click is being delivered
        assert!(cx.start_job("net.speed", |_| Ok("done".into())).is_err());
        assert!(cx.job("net.speed").is_none());
        // the click dispatch: the frame sets in_click while it delivers Ev::Click
        cx.in_click = true;
        assert!(cx.start_job("net.speed", |c| {
            c.progress(0.5);
            Ok("done".into())
        })
        .is_ok());
        let mut ended = false;
        for _ in 0..200 {
            if cx.job("net.speed").map(|v| v.end.is_some()).unwrap_or(false) {
                ended = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(ended, "the job did not finish");
        drop(cx);
        shutdown();
    }

    #[test]
    fn a_key_field_listens_binds_a_typing_key_and_esc_cancels() {
        start();
        with(|s| s.add_action(Action::new("test.mute", "Test mute", "aud"), |_| {}));
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st).for_page("aud");
        cx.listen_key("test.mute");
        let (set, listening, _) = cx.key_field("test.mute");
        assert!(set.is_none() && listening.is_some());
        // a key message with no modifier (whatever the real keyboard holds is read): a plain letter binds (the owner Oct 8)
        if crate::keys::real::mods_now().is_empty() {
            assert!(key_message(true, 0x4D, 0));
            let (set, listening, err) = cx.key_field("test.mute");
            assert!(set.is_some(), "a typing key was refused");
            assert!(listening.is_none() && err.is_none());
            cx.listen_key("test.mute");
        }
        // Esc: stop listening, the key stays as it was
        let before = cx.key_field("test.mute").0;
        assert!(key_message(true, 0x1B, 0));
        let (set, listening, _) = cx.key_field("test.mute");
        assert!(set == before && listening.is_none());
        // with no field listening, key messages are the menu's own
        assert!(!key_message(true, 0x4D, 0));
        drop(cx);
        shutdown();
    }

    /// Settings › All shortcuts: every action with its page and key text (Lane V, 024).
    #[test]
    fn the_action_list_has_every_key_of_the_app() {
        start();
        with(|s| {
            s.add_action(Action::new("mic.toggle", "Mic mute", "aud"), |_| {});
            s.add_action(Action::new("shot.take", "Screenshot", "shot"), |_| {});
            let combo = crate::keys::Combo::new(Mods::CTRL.with(Mods::SHIFT), 0x4D);
            s.keys.bind(&mut s.store, "mic.toggle", combo).unwrap();
        });
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let cx = Cx::new(0.0, false, &g, &mut st).for_page("set");
        let list = cx.actions();
        let mine: Vec<_> = list.iter().filter(|a| a.id == "mic.toggle" || a.id == "shot.take").collect();
        assert_eq!(mine.len(), 2);
        assert_eq!((mine[0].name.as_str(), mine[0].page.as_str()), ("Mic mute", "aud"));
        assert!(mine[0].key.as_deref().is_some_and(|k| k.contains('M')), "{:?}", mine[0].key);
        assert_eq!(mine[1].key, None);
        drop(cx);
        shutdown();
    }
}
