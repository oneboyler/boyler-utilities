//! The app start / stop signal for per-app switching — event-driven on our side, no admin needed, nothing injected.
//!
//! START = the game's PROCESS starting (NOTE_004_01: the mode must be set BEFORE the game's window exists / goes
//! fullscreen; switching mid-game makes it glitch). Sources, behind one trait, tried in order:
//! 1. `Win32_ProcessStartTrace` (WMI over the kernel trace) — exact, at creation, needs ADMIN (the later elevated helper;
//!    also used if the app itself runs elevated).
//! 2. WMI `__InstanceCreationEvent WITHIN 1` on `Win32_Process` — no admin; lands 0–1 s after the start (WMI re-checks
//!    the process list once a second; its CPU cost is measured in the report).
//!
//! Only exe names from the user's rules are in the query. With no rules there is no subscription at all.
//!
//! LATE = when the start is reported, the process already has a visible top-level window (found with `EnumWindows` —
//! no handle to the game). Reported as `has_window: true`; the switcher then does NOT switch (Addendum 2).
//!
//! STOP = `OpenProcess(SYNCHRONIZE)` only (A_004_01: approved) + `WaitForMultipleObjects` — Windows wakes us when it
//! ends. Process already gone when its start is handled → Stopped at once. If it refuses even that handle: WMI
//! `__InstanceDeletionEvent WITHIN 1` for that process id.
//!
//! MONITOR = the main display (the game has no window yet when it starts; games open on the main display).

use crate::autoswitch::AppEvent;
use crate::error::{DisplayError, Result};
use crate::types::MonitorId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WAIT_OBJECT_0};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::Threading::{
    CreateEventW, OpenProcess, SetEvent, WaitForMultipleObjects, INFINITE, PROCESS_SYNCHRONIZE,
};
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
/// Source 2: WMI instance-creation events (WITHIN 1). No admin.
pub struct WmiCreationEvents;

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

impl ProcessStartSource for WmiCreationEvents {
    fn name(&self) -> &'static str {
        "__InstanceCreationEvent WITHIN 1 (no admin)"
    }
    fn start(&self, names: &[String], on_start: StartSink, on_lost: LostSink) -> Result<Box<dyn Send>> {
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
            Some(on_lost),
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
    cancel: Ev,
    sources: Vec<Arc<dyn ProcessStartSource>>,
    running: Mutex<Option<(Box<dyn Send>, &'static str)>>,
    /// The exe names listened for now (lower case).
    names: Mutex<Vec<String>>,
    /// The last time a running source stopped working: what happened (for the menu's diagnostics).
    problem: Mutex<Option<String>>,
    /// Exit fallbacks (WMI deletion events) for processes that refused a SYNCHRONIZE handle.
    exit_subs: Mutex<HashMap<u32, Box<dyn Send>>>,
}

/// The running watcher. Drop or `stop()` ends it (subscriptions cancelled, waiting threads released).
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
        let has_window = has_visible_window(pid);
        let Some(monitor) = main_monitor() else { return };
        // SYNCHRONIZE only (A_004_01): no read, no query, nothing else. The exe path comes from the WMI event.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) };
        (self.sink)(AppEvent::Started { pid, exe, monitor, has_window });
        match handle {
            Ok(h) => self.wait_exit(pid, h),
            // Already gone (a short-lived exe, or noticed late): say so now, or Stopped would never come.
            Err(_) if exe_name_of(pid).is_none() => (self.sink)(AppEvent::Stopped { pid }),
            Err(_) => self.exit_fallback(pid),
        }
    }

    fn wait_exit(&self, pid: u32, h: HANDLE) {
        let cancel = self.cancel;
        let sink = self.sink.clone();
        let hp = Ev(h);
        std::thread::Builder::new()
            .name("bu-display-exit".into())
            .spawn(move || {
                let (hp, cancel) = (hp, cancel);
                // Cancel FIRST: when both are signalled Windows reports the lowest index, so a stopped watcher never
                // sends a late Stopped.
                let r = unsafe { WaitForMultipleObjects(&[cancel.0, hp.0], false, INFINITE) };
                let _ = unsafe { CloseHandle(hp.0) };
                if r.0 == WAIT_OBJECT_0.0 + 1 {
                    sink(AppEvent::Stopped { pid });
                }
            })
            .ok();
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
                    let gone = me.exit_subs.lock().unwrap_or_else(|e| e.into_inner()).remove(&pid);
                    std::thread::spawn(move || drop(gone));
                }
            }),
            None,
        );
        if let Ok(s) = sub {
            self.exit_subs.lock().unwrap_or_else(|e| e.into_inner()).insert(pid, Box::new(s));
        }
    }
}

impl AppWatcher {
    /// Starts watching for these exe file names (e.g. "VALORANT-Win64-Shipping.exe") with the default sources.
    pub fn start(names: Vec<String>, sink: impl Fn(AppEvent) + Send + Sync + 'static) -> Result<Self> {
        Self::start_with(default_sources(), names, sink)
    }

    /// Same with explicit sources (tests; the elevated helper later).
    pub fn start_with(sources: Vec<Arc<dyn ProcessStartSource>>, names: Vec<String>, sink: impl Fn(AppEvent) + Send + Sync + 'static) -> Result<Self> {
        let cancel = unsafe { CreateEventW(None, true, false, None) }.map_err(|e| DisplayError::Watcher(e.to_string()))?;
        let w = Self {
            inner: Arc::new(Inner {
                sink: Arc::new(sink),
                cancel: Ev(cancel),
                sources,
                running: Mutex::new(None),
                names: Mutex::new(Vec::new()),
                problem: Mutex::new(None),
                exit_subs: Mutex::new(HashMap::new()),
            }),
        };
        w.set_names(names)?;
        Ok(w)
    }

    /// The rule list changed: listen for these names instead (no names = no subscription at all).
    /// Tries each source in order; the first that WMI accepts is used.
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
        // Take them out first, drop them outside the locks (a WMI callback may be waiting for a lock meanwhile).
        let running = self.inner.running.lock().unwrap_or_else(|e| e.into_inner()).take();
        let subs = std::mem::take(&mut *self.inner.exit_subs.lock().unwrap_or_else(|e| e.into_inner()));
        drop(running);
        drop(subs);
        let _ = unsafe { SetEvent(self.inner.cancel.0) };
    }
}
