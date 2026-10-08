//! The app start / stop signal for per-app switching — event-driven, no admin needed, nothing injected, no WMI and no
//! polling on a normal start.
//!
//! START = the game's PROCESS starting (NOTE_004_01: the mode must be set BEFORE the game's window exists / goes
//! fullscreen; switching mid-game makes it glitch). Sources, behind one trait, tried in order:
//! 1. `Win32_ProcessStartTrace` (WMI over the kernel trace) — exact, at creation, needs ADMIN; only tried when this
//!    process runs elevated (checked on the token), so a normal start never touches WMI at all.
//! 2. `WindowCreationStarts` — the shared bu-procwatch watcher (one thread for Display and Mouse): a new top-level window
//!    anywhere makes it take one process snapshot and report every new process with a watched exe name. A game's first
//!    window is created before it is shown or goes fullscreen, so this still lands in time. It replaced WMI
//!    `__InstanceCreationEvent WITHIN 1`, which made WMI re-read the process list every second (~1.3 % of a core in
//!    WmiPrvSE, Order 048).
//!
//! Only exe names from the user's rules are listened for. With no rules there is no subscription at all.
//!
//! LATE = when the start is reported, the process already has a visible top-level window (found with `EnumWindows` —
//! no handle to the game). Reported as `has_window: true`; the switcher then does NOT switch (Addendum 2).
//!
//! STOP = `bu_procwatch::wait_exit`: `OpenProcess(SYNCHRONIZE)` only (A_004_01: approved), waited on by the Windows
//! thread pool (no thread of ours per game). Process already gone when its start is handled → Stopped at once. If it
//! refuses even that handle: process snapshots decide (at every snapshot and when one of its windows is destroyed).
//!
//! MONITOR = the main display (the game has no window yet when it starts; games open on the main display).

use crate::autoswitch::AppEvent;
use crate::error::{DisplayError, Result};
use crate::types::MonitorId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible};

use super::wmi;

/// Called by a source for every matching process start: (pid, exe path or file name).
pub type StartSink = Arc<dyn Fn(u32, String) + Send + Sync>;
/// Called by a source that was running and then stopped working (e.g. WMI refused it late), with the reason.
pub type LostSink = Arc<dyn Fn(String) + Send + Sync>;

/// One way of hearing about process starts.
pub trait ProcessStartSource: Send + Sync {
    fn name(&self) -> &'static str;
    /// Starts listening for these exe file names; dropping the returned value stops it. If it stops working later it
    /// calls `on_lost` once; the watcher then moves on to the next source.
    fn start(&self, names: &[String], on_start: StartSink, on_lost: LostSink) -> Result<Box<dyn Send>>;
}

/// Source 1: kernel process-start trace through WMI. Needs admin.
pub struct WmiStartTrace;
/// Source 2: a new top-level window anywhere + one process snapshot (bu-procwatch, shared with the Mouse tab). No admin.
pub struct WindowCreationStarts;

fn name_filter(field: &str, names: &[String]) -> String {
    names.iter().map(|n| format!("{field} = {}", wmi::wql_str(n))).collect::<Vec<_>>().join(" OR ")
}

impl ProcessStartSource for WmiStartTrace {
    fn name(&self) -> &'static str {
        "Win32_ProcessStartTrace (admin)"
    }
    fn start(&self, names: &[String], on_start: StartSink, on_lost: LostSink) -> Result<Box<dyn Send>> {
        let q = format!("SELECT ProcessID, ProcessName FROM Win32_ProcessStartTrace WHERE {}", name_filter("ProcessName", names));
        let sub = wmi::subscribe(
            q,
            Arc::new(move |o| {
                if let (Some(pid), Some(name)) = (wmi::get_int(o, "ProcessID"), wmi::get_str(o, "ProcessName")) {
                    on_start(pid, name);
                }
            }),
            Some(on_lost),
        )?;
        Ok(Box::new(sub))
    }
}

impl ProcessStartSource for WindowCreationStarts {
    fn name(&self) -> &'static str {
        "window creation + process snapshot (no admin)"
    }
    fn start(&self, names: &[String], on_start: StartSink, _on_lost: LostSink) -> Result<Box<dyn Send>> {
        Ok(Box::new(bu_procwatch::subscribe(names.to_vec(), on_start)))
    }
}

/// The default order: the exact admin trace only when we run elevated, then the window-creation watcher.
pub fn default_sources() -> Vec<Arc<dyn ProcessStartSource>> {
    let mut v: Vec<Arc<dyn ProcessStartSource>> = Vec::new();
    if bu_procwatch::is_elevated() {
        v.push(Arc::new(WmiStartTrace));
    }
    v.push(Arc::new(WindowCreationStarts));
    v
}

/// Exe file name (lower case) of a PID from a process snapshot. No handle to the process is opened.
pub fn exe_name_of(pid: u32) -> Option<String> {
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;
    let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut found = None;
    if unsafe { Process32FirstW(snap, &mut e) }.is_ok() {
        loop {
            if e.th32ProcessID == pid {
                found = Some(super::wide_to_string(&e.szExeFile).to_ascii_lowercase());
                break;
            }
            if unsafe { Process32NextW(snap, &mut e) }.is_err() {
                break;
            }
        }
    }
    let _ = unsafe { CloseHandle(snap) };
    found
}

/// True when the process already shows a top-level window (`EnumWindows`; no handle to the process).
pub fn has_visible_window(pid: u32) -> bool {
    struct F {
        pid: u32,
        found: bool,
    }
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> BOOL {
        let f = unsafe { &mut *(lp.0 as *mut F) };
        let mut p = 0u32;
        unsafe { GetWindowThreadProcessId(h, Some(&mut p)) };
        if p == f.pid && unsafe { IsWindowVisible(h) }.as_bool() {
            f.found = true;
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut f = F { pid, found: false };
    let _ = unsafe { EnumWindows(Some(cb), LPARAM(&mut f as *mut F as isize)) };
    f.found
}

/// The main display's id (the monitor a just-started game will open on).
pub fn main_monitor() -> Option<MonitorId> {
    let s = super::config::query().ok()?;
    super::config::views(&s).into_iter().find(|v| v.x == 0 && v.y == 0).map(|v| MonitorId(v.device_path))
}


type EventSink = Arc<dyn Fn(AppEvent) + Send + Sync>;

struct Inner {
    sink: EventSink,
    sources: Vec<Arc<dyn ProcessStartSource>>,
    running: Mutex<Option<(Box<dyn Send>, &'static str)>>,
    /// The exe names listened for now (lower case).
    names: Mutex<Vec<String>>,
    /// The last time a running source stopped working: what happened (for the menu's diagnostics).
    problem: Mutex<Option<String>>,
    /// The exit waits of the started processes still running (by wait id, not pid: a pid can be reused).
    exits: Mutex<HashMap<u64, bu_procwatch::ExitWait>>,
    next_wait: AtomicU64,
    /// The watcher was dropped: no Started goes out any more (a start being handled meanwhile is dropped).
    stopped: AtomicBool,
}

/// The running watcher. Drop or `stop()` ends it (subscription and exit waits cancelled; no event after that).
pub struct AppWatcher {
    inner: Arc<Inner>,
}

impl Inner {
    /// Tries the sources from index `first` on; the first one that starts is used (`running` is locked by the caller).
    fn start_sources(self: &Arc<Self>, running: &mut Option<(Box<dyn Send>, &'static str)>, names: &[String], first: usize) -> Result<()> {
        let mut errors = Vec::new();
        for s in self.sources.iter().skip(first) {
            let name = s.name();
            let weak = Arc::downgrade(self);
            let on_start: StartSink = Arc::new(move |pid, exe| {
                if let Some(inner) = weak.upgrade() {
                    inner.on_start(pid, exe);
                }
            });
            let weak = Arc::downgrade(self);
            let on_lost: LostSink = Arc::new(move |why| {
                if let Some(inner) = weak.upgrade() {
                    // Off the source's own thread: replacing the source joins that thread.
                    std::thread::spawn(move || inner.source_lost(name, why));
                }
            });
            match s.start(names, on_start, on_lost) {
                Ok(r) => {
                    *running = Some((r, name));
                    return Ok(());
                }
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }
        *running = None;
        Err(DisplayError::Watcher(errors.join("; ")))
    }

    /// A running source stopped working (e.g. WMI refused the admin trace only after the refuse wait): move on to the
    /// sources after it, so per-app switching doesn't go silently dead.
    fn source_lost(self: &Arc<Self>, name: &'static str, why: String) {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        if running.as_ref().map(|(_, n)| *n) != Some(name) {
            return; // already replaced, or the watcher was stopped
        }
        running.take();
        let names = self.names.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let first = self.sources.iter().position(|s| s.name() == name).map_or(self.sources.len(), |i| i + 1);
        let now = match self.start_sources(&mut running, &names, first) {
            Ok(()) => format!("now: {}", running.as_ref().map(|(_, n)| *n).unwrap_or("-")),
            Err(e) => format!("no other source started: {e}"),
        };
        *self.problem.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!("{name} stopped ({why}); {now}"));
    }

    /// A matching process started (from any source).
    fn on_start(self: &Arc<Self>, pid: u32, exe: String) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        let has_window = has_visible_window(pid);
        let Some(monitor) = main_monitor() else { return };
        (self.sink)(AppEvent::Started { pid, exe, monitor, has_window });
        // After Started: a process already gone is reported Stopped at once (from inside `wait_exit`), never before.
        self.wait_exit(pid);
    }

    /// Stopped comes from `bu_procwatch::wait_exit` (a SYNCHRONIZE handle only, A_004_01; snapshots if even that is
    /// refused). The wait is kept until it fires or the watcher is dropped.
    fn wait_exit(self: &Arc<Self>, pid: u32) {
        let id = self.next_wait.fetch_add(1, Ordering::Relaxed);
        let weak = Arc::downgrade(self);
        let wait = bu_procwatch::wait_exit(
            pid,
            Box::new(move || {
                if let Some(me) = weak.upgrade() {
                    // Our own finished wait (dropping it from inside its callback is allowed); dropped after the event.
                    let done = me.exits.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                    (me.sink)(AppEvent::Stopped { pid });
                    drop(done);
                }
            }),
        );
        // Checked under the lock the callback takes too, so a wait that fired meanwhile is never kept.
        let mut exits = self.exits.lock().unwrap_or_else(|e| e.into_inner());
        if !wait.ended() {
            exits.insert(id, wait);
            return;
        }
        drop(exits);
        drop(wait);
    }
}

impl AppWatcher {
    /// Starts watching for these exe file names (e.g. "VALORANT-Win64-Shipping.exe") with the default sources.
    pub fn start(names: Vec<String>, sink: impl Fn(AppEvent) + Send + Sync + 'static) -> Result<Self> {
        Self::start_with(default_sources(), names, sink)
    }

    /// Same with explicit sources (tests; the elevated helper later).
    pub fn start_with(sources: Vec<Arc<dyn ProcessStartSource>>, names: Vec<String>, sink: impl Fn(AppEvent) + Send + Sync + 'static) -> Result<Self> {
        let w = Self {
            inner: Arc::new(Inner {
                sink: Arc::new(sink),
                sources,
                running: Mutex::new(None),
                names: Mutex::new(Vec::new()),
                problem: Mutex::new(None),
                exits: Mutex::new(HashMap::new()),
                next_wait: AtomicU64::new(1),
                stopped: AtomicBool::new(false),
            }),
        };
        w.set_names(names)?;
        Ok(w)
    }

    /// The rule list changed: listen for these names instead (no names = no subscription at all).
    /// Tries each source in order; the first that starts is used.
    pub fn set_names(&self, names: Vec<String>) -> Result<()> {
        let mut running = self.inner.running.lock().unwrap_or_else(|e| e.into_inner());
        running.take();
        let mut names: Vec<String> = names.into_iter().map(|n| n.to_ascii_lowercase()).collect();
        names.sort();
        names.dedup();
        if names.is_empty() {
            self.inner.names.lock().unwrap_or_else(|e| e.into_inner()).clear();
            return Ok(());
        }
        *self.inner.names.lock().unwrap_or_else(|e| e.into_inner()) = names.clone();
        self.inner.start_sources(&mut running, &names, 0)
    }

    /// What happened the last time a running source stopped working (and where the watcher went), if ever.
    pub fn last_problem(&self) -> Option<String> {
        self.inner.problem.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Which source is listening now (for the report / the menu's diagnostics).
    pub fn active_source(&self) -> Option<&'static str> {
        self.inner.running.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(_, n)| *n)
    }

    /// Reports a start for this process as a source would (tests; also lets the menu hand over a process it knows).
    pub fn report_start(&self, pid: u32, exe: String) {
        self.inner.on_start(pid, exe);
    }

    pub fn stop(self) {}
}

impl Drop for AppWatcher {
    fn drop(&mut self) {
        // No new Started from here on. Take them out first, drop them outside the locks (an exit callback may be waiting
        // for a lock meanwhile). Order 048 review: dropping a wait waits for its running callback (a Stopped switching the
        // display back), and this runs on the UI thread - so the exit waits go on a short-lived thread; a late Stopped
        // finds no watcher (weak) and does nothing.
        self.inner.stopped.store(true, Ordering::Release);
        let running = self.inner.running.lock().unwrap_or_else(|e| e.into_inner()).take();
        let exits = std::mem::take(&mut *self.inner.exits.lock().unwrap_or_else(|e| e.into_inner()));
        drop(running);
        if !exits.is_empty() {
            let _ = std::thread::Builder::new().name("bu-display-exit-drop".into()).spawn(move || drop(exits));
        }
    }
}
