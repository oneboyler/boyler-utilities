//! The app start / stop signal for the per-app acceleration switch — bu-display's approach (Order 004 + A_004_01 /
//! A_004_02), with the shared `bu_procwatch` watcher since Order 048 (one watcher for Display and Mouse). Event-driven,
//! nothing injected, no WMI and no polling on a normal start.
//!
//! START = the app's PROCESS starting, before its window is on screen. Sources, tried in order:
//! 1. `Win32_ProcessStartTrace` (WMI over the kernel trace) — exact, at creation, needs ADMIN; only tried when this
//!    process runs elevated (checked on the token), so a normal start never touches WMI at all.
//! 2. `WindowCreationStarts` — bu-procwatch: a new top-level window anywhere makes it take one process snapshot and
//!    report every new process with a watched exe name. A game's first window is created before it is shown or goes
//!    fullscreen, so this still lands in time. It replaced WMI `__InstanceCreationEvent WITHIN 1`, which made WMI
//!    re-read the process list every second (~1.3 % of a core in WmiPrvSE).
//!    A source that ends later (late refusal) is replaced by the next one automatically.
//!
//! Only exe names from the user's rows are listened for. With no rows there is no subscription at all.
//!
//! ALREADY RUNNING (Order 077): a listed game that runs when it is added / when the app starts / when the switch goes on is not
//! "too late" any more - `Mouse::adopt_running_games` finds it in a process snapshot and the engine gives it an exit wait
//! with [`AppWatcher::watch_exit`] (still no handle to the game).
//!
//! STOP = `bu_procwatch::wait_exit_no_handle` (Order 063: NO handle to the game at all - not even SYNCHRONIZE; an anti-cheat
//! protected game such as VALORANT is never opened): the exit is found by process snapshots - at every snapshot, when one of
//! its windows is destroyed, and by a bounded re-check timer. Already gone when its start is handled → Stopped at once.
//! (The Display tab's watcher still uses `wait_exit`: `OpenProcess(SYNCHRONIZE)` ONLY, A_004_01.)

use crate::accel::switch::AppEvent;
use crate::error::{Error, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use super::wmi;

/// Called by a source for every matching process start: (pid, exe path or file name).
pub type StartSink = Arc<dyn Fn(u32, String) + Send + Sync>;
/// Called by a source when it ends after it was running (e.g. WMI refused it late).
pub type DeadSink = Arc<dyn Fn(String) + Send + Sync>;

/// One way of hearing about process starts.
pub trait ProcessStartSource: Send + Sync {
    fn name(&self) -> &'static str;
    /// Starts listening for these exe file names; dropping the returned value stops it.
    fn start(&self, names: &[String], on_start: StartSink, on_dead: DeadSink) -> Result<Box<dyn Send>>;
}

/// Source 1: kernel process-start trace through WMI. Needs admin.
pub struct WmiStartTrace;
/// Source 2: a new top-level window anywhere + one process snapshot (bu-procwatch, shared with the Display tab). No admin.
pub struct WindowCreationStarts;

fn name_filter(field: &str, names: &[String]) -> String {
    names.iter().map(|n| format!("{field} = {}", wmi::wql_str(n))).collect::<Vec<_>>().join(" OR ")
}

impl ProcessStartSource for WmiStartTrace {
    fn name(&self) -> &'static str {
        "Win32_ProcessStartTrace (admin)"
    }
    fn start(&self, names: &[String], on_start: StartSink, on_dead: DeadSink) -> Result<Box<dyn Send>> {
        let q = format!("SELECT ProcessID, ProcessName FROM Win32_ProcessStartTrace WHERE {}", name_filter("ProcessName", names));
        let sub = wmi::subscribe(
            q,
            Arc::new(move |o| {
                if let (Some(pid), Some(name)) = (wmi::get_int(o, "ProcessID"), wmi::get_str(o, "ProcessName")) {
                    on_start(pid, name);
                }
            }),
            on_dead,
        )?;
        Ok(Box::new(sub))
    }
}

impl ProcessStartSource for WindowCreationStarts {
    fn name(&self) -> &'static str {
        "window creation + process snapshot (no admin)"
    }
    fn start(&self, names: &[String], on_start: StartSink, _on_dead: DeadSink) -> Result<Box<dyn Send>> {
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

/// The live subscription, its source's index and name.
type Running = Option<(Box<dyn Send>, usize, &'static str)>;
type EventSink = Arc<dyn Fn(AppEvent) + Send + Sync>;

struct Inner {
    sink: EventSink,
    sources: Vec<Arc<dyn ProcessStartSource>>,
    /// (subscription, its source's index, its name)
    running: Mutex<Running>,
    names: Mutex<Vec<String>>,
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

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Inner {
    /// A matching process started (from any source).
    fn on_start(self: &Arc<Self>, pid: u32, exe: String) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        (self.sink)(AppEvent::Started { pid, exe });
        // After Started: a process already gone is reported Stopped at once (from inside `wait_exit`), never before.
        self.wait_exit(pid);
    }

    /// Stopped comes from `bu_procwatch::wait_exit_no_handle` (no handle to the process at all; process snapshots decide, A_063).
    /// The wait is kept until it fires or the watcher is dropped.
    fn wait_exit(self: &Arc<Self>, pid: u32) {
        let id = self.next_wait.fetch_add(1, Ordering::Relaxed);
        let weak = Arc::downgrade(self);
        let wait = bu_procwatch::wait_exit_no_handle(
            pid,
            Box::new(move || {
                if let Some(me) = weak.upgrade() {
                    // Our own finished wait (dropping it from inside its callback is allowed); dropped after the event.
                    let done = lock(&me.exits).remove(&id);
                    (me.sink)(AppEvent::Stopped { pid });
                    drop(done);
                }
            }),
        );
        // Checked under the lock the callback takes too, so a wait that fired meanwhile is never kept.
        let mut exits = lock(&self.exits);
        if !wait.ended() {
            exits.insert(id, wait);
            return;
        }
        drop(exits);
        drop(wait);
    }

    /// (Re)subscribes, trying the sources from `first` on. Dropping the old subscription happens outside the lock.
    fn subscribe_from(self: &Arc<Self>, first: usize) -> Result<()> {
        let names = lock(&self.names).clone();
        let old = lock(&self.running).take();
        drop(old);
        if names.is_empty() {
            return Ok(());
        }
        let mut errors = Vec::new();
        for (i, s) in self.sources.iter().enumerate().skip(first) {
            let weak = Arc::downgrade(self);
            let on_start: StartSink = Arc::new(move |pid, exe| {
                if let Some(inner) = weak.upgrade() {
                    inner.on_start(pid, exe);
                }
            });
            let weak: Weak<Inner> = Arc::downgrade(self);
            let on_dead: DeadSink = Arc::new(move |_why| {
                // The source ended late: move to the next one, off its thread (it is being dropped).
                if let Some(inner) = weak.upgrade() {
                    std::thread::spawn(move || {
                        let _ = inner.subscribe_from(i + 1);
                    });
                }
            });
            match s.start(&names, on_start, on_dead) {
                Ok(r) => {
                    *lock(&self.running) = Some((r, i, s.name()));
                    return Ok(());
                }
                Err(e) => errors.push(format!("{}: {e}", s.name())),
            }
        }
        Err(Error::Watcher(errors.join("; ")))
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
                exits: Mutex::new(HashMap::new()),
                next_wait: AtomicU64::new(1),
                stopped: AtomicBool::new(false),
            }),
        };
        w.set_names(names)?;
        Ok(w)
    }

    /// The row list changed: listen for these names instead (no names = no subscription at all).
    pub fn set_names(&self, names: Vec<String>) -> Result<()> {
        let mut names: Vec<String> = names.into_iter().map(|n| n.to_ascii_lowercase()).collect();
        names.sort();
        names.dedup();
        *lock(&self.inner.names) = names;
        self.inner.subscribe_from(0)
    }

    /// Which source is listening now (for the report / the menu's diagnostics).
    pub fn active_source(&self) -> Option<&'static str> {
        lock(&self.inner.running).as_ref().map(|(_, _, n)| *n)
    }

    /// Waits for this process to end (no handle to it) and reports `Stopped` then - for a game found already running, whose
    /// start was fed in by the caller. A process already gone reports `Stopped` at once.
    pub fn watch_exit(&self, pid: u32) {
        self.inner.wait_exit(pid);
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
        // for a lock meanwhile). Order 048 review: dropping a wait waits for its running callback, and this may run on the
        // UI thread - so the exit waits go on a short-lived thread; a late Stopped finds no watcher (weak) and does nothing.
        self.inner.stopped.store(true, Ordering::Release);
        let running = lock(&self.inner.running).take();
        let exits = std::mem::take(&mut *lock(&self.inner.exits));
        drop(running);
        if !exits.is_empty() {
            let _ = std::thread::Builder::new().name("bu-mouse-exit-drop".into()).spawn(move || drop(exits));
        }
    }
}
