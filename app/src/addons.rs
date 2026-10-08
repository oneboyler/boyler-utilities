//! Add-ons (Orders 035 + 037; drawing addons-v1): only things you download (the owner Oct 8: "add ons page just provides
//! quicker way to see all thats downloaded add ons wise"). Two kinds:
//! - PAGE add-on = a whole tab, got only on the Add-ons page: "obs" = Notifications for OBS (its code is in the app, Order
//!   035: Get switches it on - saved in the settings store, app scope "addon.obs" - and its tab joins the top row; Remove
//!   switches it off, its settings kept);
//! - FEATURE add-on = part of a tab, also got inside that tab: "acc" = Mouse acceleration = Raw Accel's official driver
//!   (crate bu-addons: pinned download, checks, its own installer through one admin prompt; Remove = its own uninstaller).
//!
//! Get / Remove of "acc" is ONE app-wide job ("addon.acc", the job runner: it runs on with the tab or the menu closed);
//! its progress is kept here so the Add-ons page and the Mouse tab show the same thing. A test copy runs a FAKE job
//! (pretend download pace, fake OS) - it never downloads or installs anything.

use crate::settings::Scope;
use bu_addons::fake::FakeOs;
use bu_addons::rawaccel::{self, RaState, Step, PIN};
use bu_addons::{AddonError, AddonOs, HelperAction};
use std::path::PathBuf;
#[cfg(not(test))]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Every add-on id.
pub const ALL: [&str; 2] = ["obs", "acc"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// a whole tab, got only on the Add-ons page
    Page,
    /// part of a tab, also got inside it
    Feature,
}

/// One tile of the Add-ons page (the drawing's AD list, word for word).
pub struct Addon {
    pub id: &'static str,
    pub kind: Kind,
    /// the tab it belongs to
    pub page: &'static str,
    pub name: &'static str,
    pub icon: &'static str,
    pub line: &'static str,
    pub adds: &'static [&'static str],
    pub note: &'static str,
}

pub const CATALOGUE: [Addon; 2] = [
    Addon {
        id: "obs",
        kind: Kind::Page,
        page: crate::obs::PAGE,
        name: "Notifications for OBS",
        icon: "bell",
        line: "A sound and a glass popup when OBS saves a replay clip",
        adds: &["A sound and a glass popup when OBS saves a replay", "Replay buffer and recording on or off", "A key that switches OBS scenes", "A small status icon on screen"],
        note: "Adds its own page to the top row.",
    },
    Addon {
        id: "acc",
        kind: Kind::Feature,
        page: "cur",
        name: "Mouse acceleration",
        icon: "accel",
        line: "Fast flicks go further, slow aim stays the same",
        adds: &["Raw Accel\u{2019}s driver (free, open source)", "Curves and presets in the Mouse tab", "Each game can have its own curve"],
        note: "Also in Mouse. Its installer asks for admin once and needs one restart.",
    },
];

pub fn addon(id: &str) -> Option<&'static Addon> {
    CATALOGUE.iter().find(|a| a.id == id)
}

/// What Notifications for OBS really adds to the app, in bytes: its code in the release exe (bu-obs + the app's obs
/// module + its tab), MEASURED Oct 8 (Order 037): the release exe with it 25,415,680 B, built without it (a throwaway copy
/// with `mod obs` stubbed, its tab and bu-obs left out) 24,923,136 B. Its sounds are made in memory (no files); Get
/// downloads nothing.
pub const OBS_BYTES: u64 = 25_415_680 - 24_923_136;

/// The size the tile shows: what it really adds (OBS) / the official download (Raw Accel's release zip, pinned).
pub fn size(id: &str) -> u64 {
    match id {
        "obs" => OBS_BYTES,
        _ => PIN.size,
    }
}

/// "1.5 MB"
pub fn mb(bytes: u64) -> String {
    rawaccel::mb(bytes)
}

// ------------------------------------------------------------------ change counter (the frame re-reads the top row)
#[cfg(not(test))]
static GEN: AtomicU64 = AtomicU64::new(1);

// Unit tests run in parallel, each on its own thread: one counter per thread there, so a test's Get never re-reads
// another test's top row (the app has ONE counter, one UI thread).
#[cfg(test)]
thread_local! {
    static GEN_T: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

/// Bumped on every add-on change (the frame compares it to re-read which add-on tabs show).
pub fn gen() -> u64 {
    #[cfg(test)]
    return GEN_T.with(|g| g.get());
    #[cfg(not(test))]
    GEN.load(Ordering::SeqCst)
}

fn bump() {
    #[cfg(test)]
    GEN_T.with(|g| g.set(g.get() + 1));
    #[cfg(not(test))]
    GEN.fetch_add(1, Ordering::SeqCst);
    crate::services::Waker.wake();
}

// ------------------------------------------------------------------ Notifications for OBS (page add-on)
fn key(id: &str) -> String {
    format!("addon.{id}")
}

/// Is the add-on on (saved)? (the page add-on "obs")
pub fn is_on(id: &str) -> bool {
    crate::services::try_with(|s| s.store.bool_or(Scope::App, &key(id), false)).unwrap_or(false)
}

/// Switch a page add-on on or off: saved, and the feature starts / stops now. `test` = a test copy (fake layers).
/// Get of Notifications for OBS in a normal run first takes over the original NotificationsForOBS (Order 043).
/// Order 047: that take-over closes the original (its own Quit gets up to 3 s, then 1 s more after ending it), so it
/// runs off the menu's thread and the switch-on follows when it ends ([`poll_take_over`]); its line then comes as the
/// page's notice (`take_notice`) - this returns None for it.
pub fn set_for(id: &str, on: bool, test: bool) -> Option<String> {
    if id == "obs" && on && !test {
        if !taking_over() {
            begin_take_over(false, || take_over_work(&bu_startup::Startup::new(bu_startup::real::RealOs::new()), &mut bu_obs::real::RealOs::new()));
        }
        return None;
    }
    switch_on_off(id, on, test);
    None
}

/// Save the switch and start / stop the feature (the menu's thread: the store, the feature's window and keys live there).
fn switch_on_off(id: &str, on: bool, test: bool) {
    crate::services::try_with(|s| {
        let _ = s.store.set_bool(Scope::App, &key(id), on);
    });
    if id == "obs" {
        if on {
            crate::obs::start(test);
        } else {
            crate::obs::stop();
        }
    }
    bump();
}

/// The Add-ons page's switch (a normal run).
pub fn set(id: &str, on: bool) -> Option<String> {
    let test = crate::services::try_with(|s| s.test).unwrap_or(false);
    set_for(id, on, test)
}

/// The take-over before the feature starts (obs/takeover.rs): the original found and still in use on its own -> its
/// settings become the feature's, its startup entry goes off (a line in the change log), the running copy is closed.
/// None = nothing to take over.
pub fn take_over<S: bu_startup::StartupOs>(st: &bu_startup::Startup<S>, obs: &mut dyn bu_obs::os::ObsOs) -> Option<String> {
    let t = take_over_work(st, obs)?;
    save_taken(&t);
    Some(t.line)
}

/// Order 047: what a take-over found and did, made on a worker thread; [`save_taken`] writes it on the menu's thread.
pub struct Taken {
    /// the original's settings (to import)
    settings: Option<bu_obs::Settings>,
    /// its change log lines: item, label, old, new (unit tests only - the app notes them on the worker, `take_over_work`)
    notes: Vec<(String, String, crate::undo::Val, crate::undo::Val)>,
    /// the line the page shows
    line: String,
}

/// The take-over's slow part (any thread): find the original, switch its startup off, close it (waits for it to end).
pub fn take_over_work<S: bu_startup::StartupOs>(st: &bu_startup::Startup<S>, obs: &mut dyn bu_obs::os::ObsOs) -> Option<Taken> {
    let f = crate::obs::takeover::find(st, obs).filter(|f| f.active())?;
    #[cfg_attr(not(test), allow(unused_mut))]
    let mut notes = Vec::new();
    let d = crate::obs::takeover::take_over(st, obs, &f, &mut |item, label, old, new| {
        // the original's startup entry is off now: into the change log at once, from this thread (`note` takes any
        // thread - a quit before the menu's poll still has it). Unit tests keep one queue per thread, so there the line
        // goes back to the test's thread (`save_taken`).
        #[cfg(not(test))]
        crate::undo::note("sup", item, label, old, new);
        #[cfg(test)]
        notes.push((item.to_string(), label.to_string(), old.clone(), new.clone()));
    });
    Some(Taken { settings: f.settings.clone(), notes, line: d.line() })
}

/// The menu's thread: the imported settings into the store (or the running feature), the change log lines.
fn save_taken(t: &Taken) {
    if let Some(set) = &t.settings {
        crate::obs::save_imported(set);
    }
    for (item, label, old, new) in &t.notes {
        crate::undo::note("sup", item, label, old, new);
    }
}

/// A running take-over: its result once the worker has it (None inside = still running), and whether it is a test's.
type TakeSlot = std::sync::Arc<Mutex<Option<Option<Taken>>>>;

thread_local! {
    /// Order 047: the take-over of the Get that runs now (the menu's thread only: one per UI thread, so each unit test
    /// has its own)
    static TAKING: std::cell::RefCell<Option<(TakeSlot, bool)>> = const { std::cell::RefCell::new(None) };
}

/// A Get of Notifications for OBS waits for its take-over (the tile stays as it is; another Get click does nothing).
pub fn taking_over() -> bool {
    TAKING.with(|t| t.borrow().is_some())
}

/// Start the take-over off the menu's thread (`crate::offui::spawn`, which wakes the menu when it ends); `test` = the
/// feature then starts on its fake layers. The menu's thread finishes it in [`poll_take_over`]: the shown Add-ons page
/// asks every tick, and (a normal run) a thread timer asks every 50 ms - also with the tab left or the menu closed.
pub(crate) fn begin_take_over(test: bool, work: impl FnOnce() -> Option<Taken> + Send + 'static) {
    let slot: TakeSlot = std::sync::Arc::new(Mutex::new(None));
    let s2 = slot.clone();
    TAKING.with(|t| *t.borrow_mut() = Some((slot, test)));
    crate::offui::spawn("obs-takeover", move || {
        let r = work();
        if let Ok(mut g) = s2.lock() {
            *g = Some(r);
        }
    });
    #[cfg(not(test))]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::SetTimer;
        SetTimer(None, 0, 50, Some(take_over_timer));
    }
}

/// The thread timer of a normal run (dispatched by the app's message loop on the menu's thread): ends with the take-over.
#[cfg(not(test))]
unsafe extern "system" fn take_over_timer(_: windows::Win32::Foundation::HWND, _: u32, id: usize, _: u32) {
    // (a modal loop inside a services call dispatches it too: then the next round)
    if crate::services::in_use() {
        return;
    }
    poll_take_over();
    if !taking_over() {
        let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::KillTimer(None, id) };
    }
}

/// The menu's thread: the take-over has ended -> its settings and change log lines saved, the feature switched on, its
/// line (or the usual "… is in the top row") as the page's notice. True = it ended now.
pub fn poll_take_over() -> bool {
    let done = TAKING.with(|t| {
        let mut t = t.borrow_mut();
        let r = t.as_ref().and_then(|(slot, test)| slot.lock().ok().and_then(|mut g| g.take()).map(|r| (r, *test)));
        if r.is_some() {
            *t = None;
        }
        r
    });
    let Some((taken, test)) = done else { return false };
    if let Some(t) = &taken {
        save_taken(t);
    }
    switch_on_off("obs", true, test);
    let line = taken.map(|t| t.line).unwrap_or_else(|| format!("{} is in the top row", CATALOGUE[0].name));
    *NOTICE.lock().unwrap() = Some((line, Instant::now()));
    crate::services::Waker.wake();
    true
}

/// Is this tab shown in the top row? (a tab of an add-on only while it is on)
pub fn tab_visible(page_id: &str) -> bool {
    match page_id {
        crate::obs::PAGE => is_on("obs"),
        _ => true,
    }
}

// ------------------------------------------------------------------ Mouse acceleration (feature add-on)
/// Get or Remove.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Get,
    Remove,
}

/// What the "addon.acc" job is doing now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// downloading: bytes so far, all, seconds left (None until a rate is known)
    Download { got: u64, total: u64, left_s: Option<u32> },
    /// size, SHA-256, unpacking, the driver's signature
    Checking,
    /// Windows' admin prompt + Raw Accel's installer
    Installing,
    /// Windows' admin prompt + Raw Accel's uninstaller (or the download it needs first)
    Removing,
}

/// The tile's / card's state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// on this PC (shows "On this PC" + Remove)
    pub got: bool,
    /// a restart finishes the last Get / Remove
    pub restart: bool,
    /// a Get / Remove is running
    pub busy: Option<Phase>,
}

pub const JOB: &str = "addon.acc";

struct Acc {
    state: Option<RaState>,
    busy: Option<Phase>,
    /// test pictures: a frozen display state (`addonsim:` hook), no job
    sim: bool,
}

static ACC: Mutex<Acc> = Mutex::new(Acc { state: None, busy: None, sim: false });
/// a note the shown page puts up as a toast (a job ended), and when - a note older than NOTICE_S is dropped (the menu was
/// closed or on another tab: no stale toast later)
static NOTICE: Mutex<Option<(String, Instant)>> = Mutex::new(None);
const NOTICE_S: f64 = 8.0;

fn test_copy() -> bool {
    crate::testmode::on() && !crate::testmode::real_read()
}

/// The fake OS of a test copy: Raw Accel absent unless `BU_ADDON_ACC` = installed / restart.
pub fn fake_os() -> &'static FakeOs {
    static F: OnceLock<FakeOs> = OnceLock::new();
    F.get_or_init(|| {
        let f = FakeOs::new();
        match crate::testmode::env("BU_ADDON_ACC").as_deref() {
            Some("installed") => f.set_installed(true, true),
            Some("restart") => f.set_installed(true, false),
            _ => {}
        }
        f
    })
}

fn read_state() -> RaState {
    if test_copy() {
        rawaccel::state(fake_os())
    } else {
        rawaccel::state(&bu_addons::real::RealOs::new())
    }
}

/// Read Raw Accel's state again (a registry value + opening its device: cheap, read-only). The pages call it on open.
pub fn refresh() {
    let s = read_state();
    let mut a = ACC.lock().unwrap();
    if a.state != Some(s) {
        a.state = Some(s);
        drop(a);
        bump();
    }
}

/// The tile / card state of an add-on.
pub fn view(id: &str) -> View {
    if id == "obs" {
        return View { got: is_on("obs"), restart: false, busy: None };
    }
    let mut a = ACC.lock().unwrap();
    if a.state.is_none() {
        a.state = Some(read_state());
    }
    let s = a.state.unwrap_or(RaState::Absent);
    View { got: s.got(), restart: matches!(s, RaState::InstallRestart | RaState::RemoveRestart), busy: a.busy }
}

/// A finished job's note, once.
pub fn take_notice() -> Option<String> {
    NOTICE.lock().unwrap().take().filter(|(_, at)| at.elapsed().as_secs_f64() < NOTICE_S).map(|(n, _)| n)
}

fn set_busy(p: Option<Phase>) {
    ACC.lock().unwrap().busy = p;
    crate::services::Waker.wake();
}

/// `%LOCALAPPDATA%\Boyler Utilities\Add-ons` (Raw Accel's files live in `RawAccel` inside it); a test copy: a scratch
/// folder.
pub fn dir() -> PathBuf {
    if crate::testmode::on() {
        return std::env::temp_dir().join(format!("BoylerUtilities-test-addons-{}", std::process::id()));
    }
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Boyler Utilities").join("Add-ons")
}

/// The user's own Raw Accel folder (Desktop / Downloads / Documents, names only - as the Mouse tab finds it).
fn user_rawaccel_dir() -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE").ok()?;
    let roots: Vec<PathBuf> = ["Desktop", "Downloads", "Documents"].iter().map(|d| PathBuf::from(&home).join(d)).collect();
    bu_mouse::accel::service::find_rawaccel_dir(&roots)
}

/// Turns the crate's steps into the shown phase (+ the job's own progress), with a seconds-left from the pace so far.
struct Pace {
    t0: Option<Instant>,
}

impl Pace {
    fn phase(&mut self, s: Step, ctx: &crate::jobs::JobCtx) -> Phase {
        match s {
            Step::Download { got, total } => {
                let t0 = *self.t0.get_or_insert_with(Instant::now);
                let secs = t0.elapsed().as_secs_f64();
                let left_s = (got > 0 && secs > 0.3).then(|| ((total.saturating_sub(got)) as f64 / (got as f64 / secs)).round().max(1.0) as u32);
                ctx.progress(if total > 0 { got as f32 / total as f32 } else { 0.0 });
                Phase::Download { got, total, left_s }
            }
            Step::Checking => {
                ctx.busy();
                Phase::Checking
            }
            Step::Installing => Phase::Installing,
            Step::Removing => Phase::Removing,
        }
    }
}

/// The real work (a normal run): WinHTTP + the Windows layer.
fn real_run(op: Op, ctx: &crate::jobs::JobCtx) -> Result<(), AddonError> {
    let http = bu_updater::WinHttp::new("BoylerUtilities");
    let os = bu_addons::real::RealOs::new();
    let root = dir();
    let mut pace = Pace { t0: None };
    let mut step = |s: Step| set_busy(Some(pace.phase(s, ctx)));
    // Cancel (and the app quitting) - also ends the wait on Windows' admin prompt
    let stop = || ctx.stopped();
    match op {
        Op::Get => rawaccel::get(&http, &os, &root, &mut step, &stop).map(|_| ()),
        Op::Remove => rawaccel::remove(&http, &os, &root, user_rawaccel_dir().as_deref(), &mut step, &stop),
    }
}

/// A test copy's work: the download at a pretend pace (`BU_ADDON_RATE` = share per second, default the drawing's .24),
/// cancel between steps, then the fake OS's install / uninstall. Nothing is downloaded, unpacked or run.
fn fake_run(op: Op, ctx: &crate::jobs::JobCtx) -> Result<(), AddonError> {
    let os = fake_os();
    let mut pace = Pace { t0: None };
    let mut step = |s: Step| set_busy(Some(pace.phase(s, ctx)));
    let wait = |ms: u64| std::thread::sleep(std::time::Duration::from_millis(ms));
    let r = match op {
        Op::Get => {
            let rate: f64 = crate::testmode::env("BU_ADDON_RATE").and_then(|v| v.parse().ok()).unwrap_or(0.24);
            let t0 = Instant::now();
            loop {
                if ctx.stopped() {
                    return Err(AddonError::Cancelled);
                }
                let k = (t0.elapsed().as_secs_f64() * rate).min(1.0);
                step(Step::Download { got: (k * PIN.size as f64) as u64, total: PIN.size });
                if k >= 1.0 {
                    break;
                }
                wait(40);
            }
            step(Step::Checking);
            wait(150);
            step(Step::Installing);
            wait(300);
            os.run_elevated(HelperAction::RawAccelInstall, std::path::Path::new("fake"), &|| ctx.stopped())
        }
        Op::Remove => {
            step(Step::Removing);
            wait(300);
            os.run_elevated(HelperAction::RawAccelUninstall, std::path::Path::new("fake"), &|| ctx.stopped())
        }
    };
    match r {
        bu_addons::Elevated::Done => Ok(()),
        bu_addons::Elevated::Declined => Err(AddonError::Declined),
        bu_addons::Elevated::Failed(s) => Err(AddonError::Tool(s)),
    }
}

/// Start Get / Remove of "acc" (only from a click: the job runner refuses anything else). One at a time.
pub fn start_acc(cx: &mut crate::ui::cx::Cx, op: Op) -> Result<(), String> {
    if ACC.lock().unwrap().busy.is_some() {
        return Err("already running".into());
    }
    set_busy(Some(if op == Op::Get { Phase::Download { got: 0, total: PIN.size, left_s: None } } else { Phase::Removing }));
    // every test copy - the read-only one too (it reads the real state but changes nothing) - runs the fake work
    let fake = crate::testmode::on();
    let r = cx.start_job(JOB, move |ctx| {
        // whatever happens in the work (a panic too), the tile and the Mouse card stop showing "Getting…"
        struct Clear;
        impl Drop for Clear {
            fn drop(&mut self) {
                if let Ok(mut a) = ACC.lock() {
                    if a.busy.is_some() {
                        a.busy = None;
                        a.state = Some(read_state());
                    }
                }
                bump();
            }
        }
        let _clear = Clear;
        let r = if fake { fake_run(op, ctx) } else { real_run(op, ctx) };
        let note = match (&r, op) {
            (Ok(()), Op::Get) => "Mouse acceleration is installed \u{b7} restart your PC to finish".to_string(),
            (Ok(()), Op::Remove) => "Removed Mouse acceleration \u{b7} restart your PC to finish".to_string(),
            (Err(e), _) => e.to_string(),
        };
        *NOTICE.lock().unwrap() = Some((note, Instant::now()));
        match r {
            Ok(()) => Ok(String::new()),
            Err(AddonError::Cancelled) => Err(crate::jobs::JobError::Stopped),
            Err(e) => Err(crate::jobs::JobError::Failed(e.to_string())),
        }
    });
    if r.is_err() {
        set_busy(None);
    }
    r
}

/// Cancel a running Get (works until the install step; the admin prompt can't be taken back).
pub fn cancel_acc(cx: &mut crate::ui::cx::Cx) {
    if matches!(ACC.lock().unwrap().busy, Some(Phase::Download { .. })) {
        cx.stop_job(JOB);
    }
}

/// Test hook `addonacc:<absent|installed|restart>` (fake OS state) and `addonsim:acc|<none|dl:<share>|have>` (a frozen
/// display state for pictures, the drawing's keys 1 / 2 / 3). Test copies only.
pub fn test_set_acc(state: &str) {
    if !test_copy() {
        return;
    }
    let f = fake_os();
    match state {
        "installed" => f.set_installed(true, true),
        "restart" => f.set_installed(true, false),
        _ => f.set_installed(false, false),
    }
    let mut a = ACC.lock().unwrap();
    a.state = Some(rawaccel::state(f));
    a.sim = false;
    a.busy = None;
    drop(a);
    bump();
}

pub fn test_sim_acc(state: &str) {
    if !test_copy() {
        return;
    }
    let mut a = ACC.lock().unwrap();
    a.sim = true;
    if let Some(k) = state.strip_prefix("dl:").and_then(|v| v.parse::<f64>().ok()) {
        let got = (k * PIN.size as f64) as u64;
        // the drawing's line at key 2: rate .012 / s -> round((1 - k) / .012) s left (44 % = 47 s)
        a.busy = Some(Phase::Download { got, total: PIN.size, left_s: Some(((1.0 - k) / 0.012).round().max(1.0) as u32) });
    } else {
        a.busy = None;
        a.state = Some(if state == "have" { RaState::Installed } else { RaState::Absent });
    }
    drop(a);
    bump();
}

/// Test hook text: `addons obs=<0|1> acc=<state> busy=<phase>`.
pub fn describe() -> String {
    let v = view("acc");
    let a = ACC.lock().unwrap();
    format!("addons obs={} acc={:?} got={} restart={} busy={:?} sim={}", is_on("obs") as u8, a.state, v.got, v.restart, a.busy, a.sim)
}
