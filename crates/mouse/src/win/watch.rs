//! The app start / stop signal for the per-app acceleration switch — bu-display's approach (Order 004 + A_004_01 /
//! A_004_02), re-written here (no dependency on bu-display). Event-driven on our side, nothing injected.
//!
//! START = the app's PROCESS starting. Sources, tried in order:
//! 1. `Win32_ProcessStartTrace` (WMI over the kernel trace) — exact, at creation, needs ADMIN (the later elevated helper).
//! 2. WMI `__InstanceCreationEvent WITHIN 1` on `Win32_Process` — no admin; lands 0–1 s after the start.
//!    A source that WMI ends later (late refusal) is replaced by the next one automatically.
//!
//! Only exe names from the user's rows are in the query. With no rows there is no subscription at all.
//!
//! LATE = when the start is reported, the process already shows a visible top-level window (`EnumWindows`; no handle to
//! the app). Reported as `has_window: true`; the switcher then does NOT switch ("never mid-game").
//!
//! STOP = `OpenProcess(SYNCHRONIZE)` ONLY (what A_004_01 approved: no read, no write, no query) + `WaitForMultipleObjects`
//! — Windows wakes us when it ends. If even that is refused: WMI `__InstanceDeletionEvent WITHIN 1` for that process id.

use crate::accel::switch::AppEvent;
use crate::error::{Error, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateEventW, OpenProcess, SetEvent, WaitForMultipleObjects, INFINITE, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible};

use super::wmi;

/// Called by a source for every matching process start: (pid, exe path or file name).
pub type StartSink = Arc<dyn Fn(u32, String) + Send + Sync>;
/// Called by a source when WMI ends it after it was running.
pub type DeadSink = Arc<dyn Fn(String) + Send + Sync>;

/// One way of hearing about process starts.
pub trait ProcessStartSource: Send + Sync {
    fn name(&self) -> &'static str;
    /// Starts listening for these exe file names; dropping the returned value stops it.
    fn start(&self, names: &[String], on_start: StartSink, on_dead: DeadSink) -> Result<Box<dyn Send>>;
}

/// Source 1: kernel process-start trace through WMI. Needs admin.
pub struct WmiStartTrace;
/// Source 2: WMI instance-creation events (WITHIN 1). No admin.
pub struct WmiCreationEvents;

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

impl ProcessStartSource for WmiCreationEvents {
    fn name(&self) -> &'static str {
        "__InstanceCreationEvent WITHIN 1 (no admin)"
    }
    fn start(&self, names: &[String], on_start: StartSink, on_dead: DeadSink) -> Result<Box<dyn Send>> {
        let q = format!(
            "SELECT * FROM __InstanceCreationEvent WITHIN 1 WHERE TargetInstance ISA 'Win32_Process' AND ({})",
            name_filter("TargetInstance.Name", names)
        );
        let sub = wmi::subscribe(
            q,
            Arc::new(move |o| {
                let Some(ti) = wmi::get_obj(o, "TargetInstance") else { return };
                let Some(pid) = wmi::get_int(&ti, "ProcessId") else { return };
                let exe = wmi::get_str(&ti, "ExecutablePath").or_else(|| wmi::get_str(&ti, "Name")).unwrap_or_default();
                on_start(pid, exe);
            }),
            on_dead,
        )?;
        Ok(Box::new(sub))
    }
}

/// The default order: exact admin trace first, the no-admin events if that is refused.
pub fn default_sources() -> Vec<Arc<dyn ProcessStartSource>> {
    vec![Arc::new(WmiStartTrace), Arc::new(WmiCreationEvents)]
}

/// Sendable wrapper for kernel handles (event / process handles may be waited on from any thread).
#[derive(Clone, Copy)]
struct Ev(HANDLE);
unsafe impl Send for Ev {}
unsafe impl Sync for Ev {}

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

/// The live subscription, its source's index and name.
type Running = Option<(Box<dyn Send>, usize, &'static str)>;
type EventSink = Arc<dyn Fn(AppEvent) + Send + Sync>;

struct Inner {
    sink: EventSink,
    cancel: Ev,
    sources: Vec<Arc<dyn ProcessStartSource>>,
    /// (subscription, its source's index, its name)
    running: Mutex<Running>,
    names: Mutex<Vec<String>>,
    /// Exit fallbacks (WMI deletion events) for processes that refused a SYNCHRONIZE handle.
    exit_subs: Mutex<HashMap<u32, Box<dyn Send>>>,
}

/// The running watcher. Drop or `stop()` ends it (subscriptions cancelled, waiting threads released).
pub struct AppWatcher {
    inner: Arc<Inner>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Inner {
    /// A matching process started (from any source).
    fn on_start(self: &Arc<Self>, pid: u32, exe: String) {
        let has_window = has_visible_window(pid);
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) };
        (self.sink)(AppEvent::Started { pid, exe, has_window });
        match handle {
            Ok(h) => self.wait_exit(pid, h),
            Err(_) => self.exit_fallback(pid),
        }
    }

    fn wait_exit(&self, pid: u32, h: HANDLE) {
        let cancel = self.cancel;
        let sink = self.sink.clone();
        let hp = Ev(h);
        let spawned = std::thread::Builder::new().name("bu-mouse-exit".into()).spawn(move || {
            let (hp, cancel) = (hp, cancel);
            let r = unsafe { WaitForMultipleObjects(&[hp.0, cancel.0], false, INFINITE) };
            let _ = unsafe { CloseHandle(hp.0) };
            if r == WAIT_OBJECT_0 {
                sink(AppEvent::Stopped { pid });
            }
        });
        if spawned.is_err() {
            let _ = unsafe { CloseHandle(h) };
        }
    }

    fn exit_fallback(self: &Arc<Self>, pid: u32) {
        let me = Arc::downgrade(self);
        let q = format!("SELECT * FROM __InstanceDeletionEvent WITHIN 1 WHERE TargetInstance ISA 'Win32_Process' AND TargetInstance.ProcessId = {pid}");
        let sub = wmi::subscribe(
            q,
            Arc::new(move |_| {
                if let Some(me) = me.upgrade() {
                    (me.sink)(AppEvent::Stopped { pid });
                    // Cancel it off this WMI callback thread.
                    let gone = lock(&me.exit_subs).remove(&pid);
                    std::thread::spawn(move || drop(gone));
                }
            }),
            Arc::new(|_| {}),
        );
        if let Ok(s) = sub {
            lock(&self.exit_subs).insert(pid, Box::new(s));
        }
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
                // WMI ended this source late: move to the next one, off the WMI thread (it is being dropped).
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
        let cancel = unsafe { CreateEventW(None, true, false, None) }.map_err(|e| Error::Watcher(e.to_string()))?;
        let w = Self {
            inner: Arc::new(Inner {
                sink: Arc::new(sink),
                cancel: Ev(cancel),
                sources,
                running: Mutex::new(None),
                names: Mutex::new(Vec::new()),
                exit_subs: Mutex::new(HashMap::new()),
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

    /// Reports a start for this process as a source would (tests; also lets the menu hand over a process it knows).
    pub fn report_start(&self, pid: u32, exe: String) {
        self.inner.on_start(pid, exe);
    }

    pub fn stop(self) {}
}

impl Drop for AppWatcher {
    fn drop(&mut self) {
        // Take them out first, drop them outside the locks (a WMI callback may be waiting for a lock meanwhile).
        let running = lock(&self.inner.running).take();
        let subs = std::mem::take(&mut *lock(&self.inner.exit_subs));
        drop(running);
        drop(subs);
        let _ = unsafe { SetEvent(self.inner.cancel.0) };
    }
}
