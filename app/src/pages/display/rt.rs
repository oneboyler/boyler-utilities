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

use bu_display::autoswitch::AppEvent;
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
            }
            rt
        })
        .clone()
    }

    /// The runtime of this app run is made once; the per-game rules must work with the menu closed, so the watcher starts at
    /// app start - not only when the tab was opened (Order 063: "VALORANT 1440x1080 does nothing until I open Display").
    /// No rules saved = nothing is made (no runtime, no watcher, no thread). Real runs only.
    #[cfg(windows)]
    pub fn start_background() {
        let Some(dir) = std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Boyler Utilities")) else { return };
        if Store::load(&dir.join(FILE_NAME)).rules.watched_names().is_empty() {
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

    /// A watched game started / stopped: the rules switch (no prompt, no keep bar); their notes wait for the page.
    pub fn on_app_event(&self, ev: &AppEvent) {
        let Ok(mut st) = self.store.lock() else { return };
        let Ok(mut svc) = self.svc.lock() else { return };
        let Store { presets, rules } = &mut *st;
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
