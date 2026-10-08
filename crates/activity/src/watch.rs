//! The event-driven watcher (Windows). One thread with a message loop and one hidden, never-shown window; it sleeps in
//! `GetMessageW` and wakes only when Windows has something:
//! * the front window changed — `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT)` (an accessibility
//!   event, not an input hook; nothing is injected anywhere);
//! * lock / unlock — `WTSRegisterSessionNotification` → `WM_WTSSESSION_CHANGE`;
//! * sleep / wake — `WM_POWERBROADCAST`; sign-out / shutdown — `WM_QUERYENDSESSION` / `WM_ENDSESSION` (both save).
//!
//! **Idle** has no Windows event, so it is the one timed wake: a one-shot timer aimed at "last input + 5 min"
//! (`GetLastInputInfo`); when it fires and there was input meanwhile it re-aims — at most one wake per 5 minutes of
//! activity, none while a game is in front. Once idle, the thread asks for raw keyboard / mouse input
//! (`RegisterRawInputDevices`, RIDEV_INPUTSINK — read-only notifications) only until the first input arrives, then
//! drops it again; and while away it re-checks `GetLastInputInfo` once every 5 min (and on any other wake), so idle
//! ends even if raw input couldn't be registered or was taken over. Wakes: ≤ 1 per 5 min while active or away.

use crate::activity::{Activity, FgApp, IDLE_AFTER_MS};
use crate::clock::{Clock, SystemClock};
use crate::real::{exe_of_window, file_description};
use crate::store::FileStore;
use crate::{ActivityError, Result};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::{WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Input::{RegisterRawInputDevices, RAWINPUTDEVICE, RIDEV_INPUTSINK, RIDEV_REMOVE};
use windows::Win32::UI::WindowsAndMessaging::*;

pub type Shared = Arc<Mutex<Activity<FileStore>>>;

/// How often the watcher woke (proof of "event-driven").
#[derive(Debug, Default)]
pub struct Stats {
    pub foreground_events: AtomicU64,
    pub idle_timer_wakes: AtomicU64,
    pub input_wakes: AtomicU64,
    pub other_messages: AtomicU64,
}

pub struct Watcher {
    thread: u32,
    join: Option<JoinHandle<()>>,
    shared: Shared,
    pub stats: Arc<Stats>,
}

struct Ctx {
    shared: Shared,
    hwnd: HWND,
    names: HashMap<String, String>,
    raw_on: bool,
    stats: Arc<Stats>,
}

thread_local! {
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

const IDLE_TIMER: usize = 1;
const WM_POKE: u32 = WM_APP + 1;
const WTS_SESSION_LOCK: usize = 7;
const WTS_SESSION_UNLOCK: usize = 8;
const PBT_APMSUSPEND: usize = 4;
const PBT_APMRESUMESUSPEND: usize = 7;
const PBT_APMRESUMEAUTOMATIC: usize = 0x12;

fn lock(s: &Shared) -> std::sync::MutexGuard<'_, Activity<FileStore>> {
    s.lock().unwrap_or_else(|p| p.into_inner())
}

impl Watcher {
    /// Starts watching; the counter is shared with the UI ([`Watcher::with`]). The app starts the watcher only while
    /// "Count my activity" is on.
    pub fn start(activity: Activity<FileStore>) -> Result<Watcher> {
        let shared: Shared = Arc::new(Mutex::new(activity));
        let stats = Arc::new(Stats::default());
        let (tx, rx) = std::sync::mpsc::channel::<std::result::Result<u32, ActivityError>>();
        let (s2, st2) = (shared.clone(), stats.clone());
        let join = std::thread::Builder::new()
            .name("bu-activity-watch".into())
            .spawn(move || run(s2, st2, tx))
            .map_err(|e| ActivityError::Os { context: format!("start watcher thread: {e}"), code: 0 })?;
        match rx.recv() {
            Ok(Ok(thread)) => Ok(Watcher { thread, join: Some(join), shared, stats }),
            Ok(Err(e)) => {
                let _ = join.join();
                Err(e)
            }
            Err(_) => Err(ActivityError::Os { context: "watcher thread ended".into(), code: 0 }),
        }
    }

    /// Read or change the counter (views, the switch, right-click choices). Then call [`Watcher::poke`] after a change.
    pub fn with<R>(&self, f: impl FnOnce(&mut Activity<FileStore>) -> R) -> R {
        f(&mut lock(&self.shared))
    }

    /// Re-reads the front window and re-aims the idle check (after the switch or a choice changed).
    pub fn poke(&self) {
        // SAFETY: posting to our own thread's queue.
        let _ = unsafe { PostThreadMessageW(self.thread, WM_POKE, WPARAM(0), LPARAM(0)) };
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // SAFETY: as above; the loop ends on WM_QUIT and saves.
        let _ = unsafe { PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn run(shared: Shared, stats: Arc<Stats>, ready: std::sync::mpsc::Sender<std::result::Result<u32, ActivityError>>) {
    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let class = w!("BoylerUtilitiesActivityWatch");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: class, ..Default::default() };
        RegisterClassW(&wc);
        // a top-level window (broadcasts like WM_POWERBROADCAST only reach top-level windows), never shown
        let hwnd = match CreateWindowExW(WS_EX_TOOLWINDOW, class, PCWSTR::null(), WS_POPUP, 0, 0, 0, 0, None, None, Some(inst.into()), None) {
            Ok(h) => h,
            Err(e) => {
                let _ = ready.send(Err(ActivityError::Os { context: "CreateWindowExW".into(), code: e.code().0 as u32 }));
                return;
            }
        };
        CTX.with(|c| *c.borrow_mut() = Some(Ctx { shared, hwnd, names: HashMap::new(), raw_on: false, stats }));
        let hook = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
        if hook.is_invalid() {
            // without the foreground event nothing could be counted: fail the start instead of counting nothing
            let _ = ready.send(Err(ActivityError::Os { context: "SetWinEventHook(EVENT_SYSTEM_FOREGROUND)".into(), code: 0 }));
            CTX.with(|c| *c.borrow_mut() = None);
            let _ = DestroyWindow(hwnd);
            return;
        }
        let wts = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION).is_ok();
        let _ = ready.send(Ok(GetCurrentThreadId()));
        refresh();
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.hwnd.is_invalid() && msg.message == WM_POKE {
                refresh();
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if !hook.is_invalid() {
            let _ = UnhookWinEvent(hook);
        }
        if wts {
            let _ = WTSUnRegisterSessionNotification(hwnd);
        }
        with_ctx(|c| {
            set_raw(c, false);
            let now = SystemClock.now();
            let _ = lock(&c.shared).save(now);
        });
        let _ = DestroyWindow(hwnd);
        CTX.with(|c| *c.borrow_mut() = None);
    }
}

/// Runs `f` on the watcher state; skipped (never a panic) if a Windows callback ever re-entered while it is in use.
fn with_ctx<R>(f: impl FnOnce(&mut Ctx) -> R) -> Option<R> {
    CTX.with(|c| match c.try_borrow_mut() {
        Ok(mut b) => b.as_mut().map(f),
        Err(_) => None,
    })
}

/// What is in front now.
fn front(c: &mut Ctx, hwnd: HWND) -> Option<FgApp> {
    if hwnd.is_invalid() {
        return None;
    }
    let path = exe_of_window(hwnd);
    if path.is_empty() {
        return None;
    }
    let name = c.names.entry(path.clone()).or_insert_with(|| file_description(&path).unwrap_or_else(|| crate::os::exe_stem(&path))).clone();
    Some(FgApp { path, name })
}

/// Re-read the front window and re-aim the idle check.
fn refresh() {
    with_ctx(|c| {
        let h = unsafe { GetForegroundWindow() };
        let app = front(c, h);
        lock(&c.shared).foreground(app, SystemClock.now());
        arm_idle(c);
    });
}

unsafe extern "system" fn on_foreground(_h: HWINEVENTHOOK, _ev: u32, hwnd: HWND, id_object: i32, _child: i32, _thread: u32, _time: u32) {
    if id_object != OBJID_WINDOW.0 {
        return;
    }
    with_ctx(|c| {
        c.stats.foreground_events.fetch_add(1, Ordering::Relaxed);
        let app = front(c, hwnd);
        lock(&c.shared).foreground(app, SystemClock.now());
        arm_idle(c);
    });
}

fn ms_since_input() -> u32 {
    let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    // SAFETY: plain reads.
    unsafe {
        if !GetLastInputInfo(&mut li).as_bool() {
            return 0;
        }
        GetTickCount().wrapping_sub(li.dwTime)
    }
}

/// Aims the one-shot idle check, or goes idle now.
fn arm_idle(c: &mut Ctx) {
    unsafe {
        let _ = KillTimer(Some(c.hwnd), IDLE_TIMER);
    }
    let (on, applies, idle) = {
        let a = lock(&c.shared);
        (a.is_on(), a.idle_applies(), a.is_idle())
    };
    if !on {
        return;
    }
    let since = ms_since_input() as i64;
    if idle {
        if since < IDLE_AFTER_MS {
            // input came back without a WM_INPUT (raw input couldn't be registered, or another part of the process took
            // it over): any wake ends idle — so idle can never stick
            set_raw(c, false);
            lock(&c.shared).back_from_idle(SystemClock.now());
        } else {
            // still away: one more check in 5 min (the fallback if no WM_INPUT ever arrives)
            unsafe {
                SetTimer(Some(c.hwnd), IDLE_TIMER, IDLE_AFTER_MS as u32, None);
            }
            return;
        }
    }
    if !applies {
        return;
    }
    if since >= IDLE_AFTER_MS {
        let now = SystemClock.now();
        lock(&c.shared).went_idle(now.plus_ms(-since), now);
        set_raw(c, true);
        // the fallback if no WM_INPUT ever arrives (raw input failed / taken over): re-check in 5 min
        unsafe {
            SetTimer(Some(c.hwnd), IDLE_TIMER, IDLE_AFTER_MS as u32, None);
        }
    } else {
        unsafe {
            SetTimer(Some(c.hwnd), IDLE_TIMER, (IDLE_AFTER_MS - since) as u32 + 50, None);
        }
    }
}

/// Raw keyboard + mouse notifications on (only while idle) / off.
fn set_raw(c: &mut Ctx, on: bool) {
    if c.raw_on == on {
        return;
    }
    let (flags, target) = if on { (RIDEV_INPUTSINK, c.hwnd) } else { (RIDEV_REMOVE, HWND::default()) };
    let devs = [
        RAWINPUTDEVICE { usUsagePage: 1, usUsage: 2, dwFlags: flags, hwndTarget: target },
        RAWINPUTDEVICE { usUsagePage: 1, usUsage: 6, dwFlags: flags, hwndTarget: target },
    ];
    // SAFETY: a plain registration of two devices for our own window.
    if unsafe { RegisterRawInputDevices(&devs, std::mem::size_of::<RAWINPUTDEVICE>() as u32) }.is_ok() {
        c.raw_on = on;
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wp.0 == IDLE_TIMER => {
            with_ctx(|c| {
                c.stats.idle_timer_wakes.fetch_add(1, Ordering::Relaxed);
                arm_idle(c);
                lock(&c.shared).maybe_save(SystemClock.now());
            });
            LRESULT(0)
        }
        WM_INPUT => {
            with_ctx(|c| {
                c.stats.input_wakes.fetch_add(1, Ordering::Relaxed);
                set_raw(c, false);
                lock(&c.shared).back_from_idle(SystemClock.now());
                arm_idle(c);
            });
            unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
        }
        WM_WTSSESSION_CHANGE => {
            with_ctx(|c| {
                c.stats.other_messages.fetch_add(1, Ordering::Relaxed);
                let now = SystemClock.now();
                match wp.0 {
                    WTS_SESSION_LOCK => lock(&c.shared).locked(true, now),
                    WTS_SESSION_UNLOCK => lock(&c.shared).locked(false, now),
                    _ => {}
                }
            });
            if wp.0 == WTS_SESSION_UNLOCK {
                refresh();
            }
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            with_ctx(|c| {
                c.stats.other_messages.fetch_add(1, Ordering::Relaxed);
                let now = SystemClock.now();
                match wp.0 {
                    PBT_APMSUSPEND => lock(&c.shared).asleep(true, now),
                    PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => lock(&c.shared).asleep(false, now),
                    _ => {}
                }
            });
            if matches!(wp.0, PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND) {
                refresh();
            }
            LRESULT(1)
        }
        WM_QUERYENDSESSION => {
            with_ctx(|c| {
                let _ = lock(&c.shared).save(SystemClock.now());
            });
            LRESULT(1)
        }
        WM_ENDSESSION => {
            if wp.0 != 0 {
                with_ctx(|c| lock(&c.shared).asleep(true, SystemClock.now()));
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}
