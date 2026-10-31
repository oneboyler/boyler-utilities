//! The Display tab's runtime (Order 019): the one `DisplayService` + the presets / rules + the 10 s keep countdown + the
//! app watcher for "Switch automatically". It is NOT the page: the page shows it and sends it changes, and is dropped when
//! the tab is left, while this lives on, because
//! - a resolution change waiting for Keep must still go back after 10 s when the menu is closed meanwhile, and
//! - the rules switch at a game's PROCESS START (never mid-game, no prompt; OWNER_DECISIONS Oct 8) with the menu closed.
//! Real copy: one shared runtime per app run (`shared`), made the first time it is needed. Test copies: a FAKE runtime per
//! page (`fake_sample`) - nothing on the PC is read or changed, no watcher, no file.
//! Lock order everywhere: `store` before `svc` (the watcher's thread takes both).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use std::time::Duration;

use bu_display::autoswitch::AppEvent;
use bu_display::brightlock::{LockWatch, Restore, WakeEvent, RETRY_AT};
use bu_display::fake::FakeDisplayOs;
use bu_display::keep::{self, KeepTimer};
use bu_display::store::{Store, FILE_NAME};
use bu_display::{ChangeKind, DisplayOs, DisplayService, GpuScaling, Mode, MonitorId, MonitorInfo, Result, Vcp, VcpValue, VibranceRaw, VideoMode};

/// The OS layer behind the service: Windows, or the fake (tests / pixel comparisons).
pub enum AnyOs {
    #[cfg(windows)]
    Win(bu_display::win::WinDisplayOs),
    Fake(FakeDisplayOs),
}

macro_rules! each {
    ($s:expr, $o:ident => $e:expr) => {
        match $s {
            #[cfg(windows)]
            AnyOs::Win($o) => $e,
            AnyOs::Fake($o) => $e,
        }
    };
}

impl DisplayOs for AnyOs {
    fn monitors(&self) -> Result<Vec<MonitorInfo>> {
        each!(self, o => o.monitors())
    }
    fn modes(&self, id: &MonitorId) -> Result<Vec<VideoMode>> {
        each!(self, o => o.modes(id))
    }
    fn apply_mode(&mut self, id: &MonitorId, mode: &Mode, save: bool) -> Result<()> {
        each!(self, o => o.apply_mode(id, mode, save))
    }
    fn save_current(&mut self, id: &MonitorId, mode: &Mode) -> Result<()> {
        each!(self, o => o.save_current(id, mode))
    }
    fn set_main(&mut self, id: &MonitorId) -> Result<()> {
        each!(self, o => o.set_main(id))
    }
    fn set_dpi_percent(&mut self, id: &MonitorId, percent: u32) -> Result<()> {
        each!(self, o => o.set_dpi_percent(id, percent))
    }
    fn ddc_get(&mut self, id: &MonitorId, vcp: Vcp) -> Result<VcpValue> {
        each!(self, o => o.ddc_get(id, vcp))
    }
    fn ddc_set(&mut self, id: &MonitorId, vcp: Vcp, value: u32) -> Result<()> {
        each!(self, o => o.ddc_set(id, vcp, value))
    }
    fn vibrance_get(&mut self, id: &MonitorId) -> Result<VibranceRaw> {
        each!(self, o => o.vibrance_get(id))
    }
    fn vibrance_set(&mut self, id: &MonitorId, level: i32) -> Result<()> {
        each!(self, o => o.vibrance_set(id, level))
    }
    fn needs_admin(&self, kind: ChangeKind) -> bool {
        each!(self, o => o.needs_admin(kind))
    }
}

pub type Svc = DisplayService<AnyOs>;

pub struct Rt {
    pub svc: Arc<Mutex<Svc>>,
    pub store: Arc<Mutex<Store>>,
    /// the settings file (None = never saved: fake runtimes)
    path: Option<PathBuf>,
    keep: Mutex<Option<KeepTimer>>,
    /// short notes made while the page may be closed (the countdown went back, a rule couldn't switch): shown as a
    /// toast the next time the page is built
    notes: Arc<Mutex<Vec<String>>>,
    #[cfg(windows)]
    watcher: Mutex<Option<bu_display::win::watch::AppWatcher>>,
    /// Order 124: "Lock brightness" - the one sleeping thread that puts a locked brightness back after a wake event (only
    /// while a lock is on), and (real runs) the hidden window that hears those events from Windows
    lock: Mutex<Option<LockWatch>>,
    #[cfg(windows)]
    power: Mutex<Option<bu_display::win::power::PowerEvents>>,
    /// when the lock's tries run after an event (fake runtimes: short)
    lock_retry: Vec<Duration>,
    /// bumped by every brightness change of the user's (after the locked value moved, before the monitor is written): a
    /// try of the lock thread running meanwhile writes nothing (end review)
    moved: Arc<std::sync::atomic::AtomicU64>,
    pub fake: bool,
    /// bumped by every change made off the page (a picked exe, a rule switching): the page reads again
    epoch: std::sync::atomic::AtomicU64,
}

/// "1920 × 1080 · 165 Hz" (the drawing's `fmt`: the rate rounded unless two of the monitor's rates round the same).
pub fn mode_text(svc: &Svc, id: &MonitorId, m: &Mode) -> String {
    let rates = svc.modes(id).map(|v| bu_display::fields::all_rates(&v)).unwrap_or_default();
    bu_display::fields::mode_text(m, &rates)
}

impl Rt {
    fn new(os: AnyOs, store: Store, path: Option<PathBuf>, fake: bool) -> Rt {
        Rt {
            svc: Arc::new(Mutex::new(DisplayService::new(os))),
            store: Arc::new(Mutex::new(store)),
            path,
            keep: Mutex::new(None),
            notes: Arc::new(Mutex::new(Vec::new())),
            #[cfg(windows)]
            watcher: Mutex::new(None),
            lock: Mutex::new(None),
            #[cfg(windows)]
            power: Mutex::new(None),
            lock_retry: if fake { vec![Duration::from_millis(20), Duration::from_millis(60), Duration::from_millis(120)] } else { RETRY_AT.to_vec() },
            moved: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            fake,
            epoch: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// A fake runtime with the drawing's sample: two monitors (bu-display `drawing_sample`), its three presets and its
    /// three rules (VALORANT → 1440 × 1080 Stretch + vibrance 80 %, CS2 → the same + 70 %, Fortnite → 1920 × 1080, off).
    pub fn fake_sample() -> Arc<Rt> {
        Self::fake_sample_keep(std::time::Duration::from_secs(bu_display::service::KEEP_SECONDS))
    }

    /// The same with another countdown length (tests).
    pub fn fake_sample_keep(keep_for: std::time::Duration) -> Arc<Rt> {
        let os = FakeDisplayOs::drawing_sample();
        let dell = MonitorId("fake-dell".into());
        let modes = os.modes(&dell).unwrap_or_default();
        let mut st = Store::default();
        let p1 = st.presets.add(1920, 1080, 165.0, GpuScaling::KeepAspect, &modes).ok();
        let p2 = st.presets.add(1440, 1080, 165.0, GpuScaling::Stretch, &modes).ok();
        let _p3 = st.presets.add(1280, 960, 144.0, GpuScaling::BlackBars, &modes).ok();
        for (exe, p, on, vib) in [(super::apps::KNOWN[0].exe, p2, true, Some(80)), (super::apps::KNOWN[1].exe, p2, true, Some(70)), (super::apps::KNOWN[2].exe, p1, false, None)] {
            let id = st.rules.add_rule(exe, p);
            if let Some(r) = st.rules.rule_mut(id) {
                r.enabled = on;
                r.vibrance = vib;
            }
        }
        let rt = Rt::new(AnyOs::Fake(os), st, None, true);
        if let Ok(mut svc) = rt.svc.lock() {
            let fresh = std::mem::replace(&mut *svc, DisplayService::new(AnyOs::Fake(FakeDisplayOs::default())));
            *svc = fresh.with_keep_duration(keep_for);
        }
        Arc::new(rt)
    }

    /// The real runtime of this app run (made once; kept until the app exits). `real_read` = a measuring test copy:
    /// real reads, every change refused by the OS layer itself, no watcher.
    #[cfg(windows)]
    pub fn shared(real_read: bool) -> Arc<Rt> {
        static RT: OnceLock<Arc<Rt>> = OnceLock::new();
        RT.get_or_init(|| {
            let dir = std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Boyler Utilities"));
            let os = if real_read {
                bu_display::win::WinDisplayOs::read_only(dir.clone())
            } else {
                bu_display::win::WinDisplayOs::new(dir.clone())
            };
            let path = if real_read { None } else { dir.map(|d| d.join(FILE_NAME)) };
            let store = path.as_deref().map(Store::load).unwrap_or_default();
            let rt = Arc::new(Rt::new(AnyOs::Win(os), store, path, false));
            if !real_read {
                rt.sync_watcher_soon();
                rt.sync_lock();
            }
            rt
        })
        .clone()
    }

    /// The runtime of this app run is made once; the per-game rules must work with the menu closed, so the watcher starts at
    /// app start - not only when the tab was opened (Order 063: "VALORANT 1440x1080 does nothing until I open Display").
    /// No rules and no brightness lock saved = nothing is made (no runtime, no watcher, no thread). Real runs only.
    /// (Order 124: a saved "Lock brightness" works from the app's start too.)
    #[cfg(windows)]
    pub fn start_background() {
        let Some(dir) = std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Boyler Utilities")) else { return };
        let st = Store::load(&dir.join(FILE_NAME));
        if st.rules.watched_names().is_empty() && st.bright_locks.is_empty() {
            return;
        }
        let _ = Rt::shared(false);
    }

    /// Writes presets + rules (nothing for a fake runtime). A failed write becomes a note.
    pub fn save(&self) {
        let Some(p) = &self.path else { return };
        let res = self.store.lock().map(|s| s.save(p));
        if let Ok(Err(e)) = res {
            self.note(format!("Couldn’t save: {e}"));
        }
    }

    pub fn bump(&self) {
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn epoch(&self) -> u64 {
        self.epoch.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn note(&self, t: String) {
        if let Ok(mut n) = self.notes.lock() {
            n.push(t);
        }
    }

    pub fn take_notes(&self) -> Vec<String> {
        self.notes.lock().map(|mut n| std::mem::take(&mut *n)).unwrap_or_default()
    }

    /// After every Apply: (re)starts the 10 s countdown. When it runs out the service goes back by itself (menu open or
    /// not) and leaves the note "Not kept, back to …".
    pub fn arm_keep(&self) {
        let mut k = self.keep.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = k.take() {
            old.cancel();
        }
        let notes = self.notes.clone();
        let svc = self.svc.clone();
        *k = keep::arm(self.svc.clone(), move |r| {
            let text = match r {
                Ok(rv) => match (rv.restored.first(), svc.lock()) {
                    (Some((id, m)), Ok(s)) => format!("Not kept, back to {}", mode_text(&s, id, m)),
                    _ => "Not kept".to_string(),
                },
                Err(e) => format!("Couldn’t go back: {e}"),
            };
            if let Ok(mut n) = notes.lock() {
                n.push(text);
            }
        });
    }

    /// Keep / Revert pressed: the countdown is not needed any more.
    pub fn disarm_keep(&self) {
        if let Some(old) = self.keep.lock().unwrap_or_else(|e| e.into_inner()).take() {
            old.cancel();
        }
    }

    /// Starts, updates or stops the app watcher to match the rules (no rules on = no watcher at all: 0 CPU). Never in the
    /// uninstaller's undo with no window (Order 036: it would switch for a game and exit without switching back).
    #[cfg(windows)]
    pub fn sync_watcher(self: &Arc<Self>) {
        if self.fake || self.path.is_none() || crate::undo::headless() {
            return;
        }
        let names = self.store.lock().map(|s| s.rules.watched_names()).unwrap_or_default();
        let mut w = self.watcher.lock().unwrap_or_else(|e| e.into_inner());
        if names.is_empty() {
            *w = None;
            return;
        }
        if let Some(cur) = w.as_ref() {
            if cur.set_names(names.clone()).is_ok() {
                return;
            }
        }
        let weak = Arc::downgrade(self);
        *w = bu_display::win::watch::AppWatcher::start(names, move |ev| {
            if let Some(rt) = weak.upgrade() {
                rt.on_app_event(&ev);
            }
        })
        .ok();
    }

    #[cfg(not(windows))]
    pub fn sync_watcher(self: &Arc<Self>) {}

    /// Order 047: `sync_watcher` off the caller's thread - starting the watcher waits for WMI's answer (0.1 - 1.5 s), which
    /// held the menu when a rule was added / changed / removed. One at a time, each reading the rules as they are when it
    /// runs, so the last one leaves the watcher matching the last change.
    pub fn sync_watcher_soon(self: &Arc<Self>) {
        static ONE: Mutex<()> = Mutex::new(());
        let rt = self.clone();
        crate::offui::spawn("display-watcher", move || {
            let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
            rt.sync_watcher();
        });
    }

    /// Order 124: the monitor's locked brightness %, if it is locked.
    pub fn brightness_lock(&self, id: &MonitorId) -> Option<u8> {
        self.store.lock().ok().and_then(|s| s.brightness_lock(id))
    }

    /// Order 124: every locked monitor with its brightness %.
    /// Order 124 (end review): the user sets a brightness (slider, change-log put-back). Call it BEFORE the monitor is
    /// written: a locked monitor's locked value moves to `pct`, and the lock thread hears of it, so a try running
    /// meanwhile never puts the old value back. True when the locked value changed (the caller saves).
    pub fn user_brightness(&self, id: &MonitorId, pct: u8) -> bool {
        let changed = self.store.lock().is_ok_and(|mut st| {
            let move_it = st.brightness_lock(id).is_some_and(|l| l != pct);
            if move_it {
                st.lock_brightness(id, Some(pct));
            }
            move_it
        });
        self.moved.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        changed
    }

    /// How many brightness changes of the user's the lock thread was told of (tests).
    pub fn brightness_moves(&self) -> u64 {
        self.moved.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn brightness_locks(&self) -> Vec<(MonitorId, u8)> {
        self.store.lock().map(|s| s.brightness_locks()).unwrap_or_default()
    }

    /// Order 124: "Lock brightness" on (at `pct`; a new value replaces the old one) / off (`None`): saved (it holds after
    /// a restart) and the lock thread follows (started with the first lock, ended with the last).
    pub fn set_brightness_lock(self: &Arc<Self>, id: &MonitorId, pct: Option<u8>) {
        let changed = self.store.lock().map(|mut s| {
            let before = s.brightness_lock(id);
            s.lock_brightness(id, pct);
            before != s.brightness_lock(id)
        });
        if changed.unwrap_or(false) {
            self.save();
            self.sync_lock();
        }
    }

    /// Order 124: a lock on = the lock thread runs (+ in real runs the hidden window for Windows' wake events); no lock =
    /// neither exists (0 CPU, no thread). Never in the uninstaller's undo with no window, never in a --real-read copy.
    pub fn sync_lock(self: &Arc<Self>) {
        let any = self.store.lock().map(|s| !s.bright_locks.is_empty()).unwrap_or(false);
        if !any || (!self.fake && (self.path.is_none() || crate::undo::headless())) {
            // the window first, dropped outside every lock: its thread may be in `wake` (which takes `lock`) and is joined
            #[cfg(windows)]
            {
                let p = self.power.lock().unwrap_or_else(|e| e.into_inner()).take();
                drop(p);
            }
            let l = self.lock.lock().unwrap_or_else(|e| e.into_inner()).take();
            drop(l);
            return;
        }
        let mut lock = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if lock.is_none() {
            let store = self.store.clone();
            let weak = Arc::downgrade(self);
            *lock = Some(LockWatch::start(
                self.svc.clone(),
                move || store.lock().map(|s| s.brightness_locks()).unwrap_or_default(),
                self.moved.clone(),
                self.lock_retry.clone(),
                move |r: &[(MonitorId, Restore)]| {
                    // an open page shows the value put back
                    if r.iter().any(|(_, x)| matches!(x, Restore::Written { .. })) {
                        if let Some(rt) = weak.upgrade() {
                            rt.bump();
                        }
                    }
                },
            ));
        }
        drop(lock);
        #[cfg(windows)]
        if !self.fake {
            let mut p = self.power.lock().unwrap_or_else(|e| e.into_inner());
            if p.is_none() {
                let weak = Arc::downgrade(self);
                *p = bu_display::win::power::PowerEvents::start(Arc::new(move |ev| {
                    if let Some(rt) = weak.upgrade() {
                        rt.wake(ev);
                    }
                }))
                .ok();
            }
        }
    }

    /// Order 124: Windows said a monitor may have lost its brightness (the power window; tests call it the same way).
    pub fn wake(&self, ev: WakeEvent) {
        if let Some(l) = self.lock.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            l.wake(ev);
        }
    }

    /// A watched game started / stopped: the rules switch (no prompt, no keep bar); their notes wait for the page.
    pub fn on_app_event(&self, ev: &AppEvent) {
        let Ok(mut st) = self.store.lock() else { return };
        let Ok(mut svc) = self.svc.lock() else { return };
        let Store { presets, rules, .. } = &mut *st;
        let _ = rules.run(ev, presets, &mut svc);
        let notes = rules.take_notes();
        drop(svc);
        drop(st);
        self.bump();
        for n in notes {
            use bu_display::autoswitch::SwitchNote;
            let t = match n {
                SwitchNote::TooLate { exe, .. } => format!("{} was already open · it switches next launch", super::apps::name_of(&exe)),
                SwitchNote::PresetNotKept { .. } => "Apply and keep that preset once to use it automatically".to_string(),
                SwitchNote::SwitchFailed { detail, .. } => format!("Couldn’t switch: {detail}"),
            };
            self.note(t);
        }
    }
}
