//! Order 124: the brightness lock's wake events from Windows - no polling. One thread with a message loop and one
//! hidden, never-shown top-level window (broadcasts like WM_POWERBROADCAST and WM_DISPLAYCHANGE only reach top-level
//! windows); it sleeps in `GetMessageW` and wakes only when Windows sends:
//! * the display(s) powered back on: `RegisterPowerSettingNotification(GUID_CONSOLE_DISPLAY_STATE)` - off (0) then on
//!   (1); dimmed (2) is not off. Windows sends the current state once right after registering: that one is not an event;
//! * the PC woke: `WM_POWERBROADCAST` `PBT_APMRESUMEAUTOMATIC` / `PBT_APMRESUMESUSPEND`;
//! * the session was unlocked: `WTSRegisterSessionNotification` → `WM_WTSSESSION_CHANGE` `WTS_SESSION_UNLOCK`;
//! * the monitors changed: `WM_DISPLAYCHANGE`;
//! * a monitor arrived: `RegisterDeviceNotificationW(GUID_DEVINTERFACE_MONITOR)` → `WM_DEVICECHANGE` `DBT_DEVICEARRIVAL`.

use crate::brightlock::WakeEvent;
use crate::error::{DisplayError, Result};
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use windows::core::{w, PCWSTR};
use windows::Win32::Devices::Display::GUID_DEVINTERFACE_MONITOR;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{RegisterPowerSettingNotification, UnregisterPowerSettingNotification, POWERBROADCAST_SETTING};
use windows::Win32::System::RemoteDesktop::{WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION};
use windows::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::*;

pub type WakeSink = Arc<dyn Fn(WakeEvent) + Send + Sync>;

/// The running event window. Dropping it ends its thread (window and registrations removed).
pub struct PowerEvents {
    thread: u32,
    join: Option<JoinHandle<()>>,
    /// How many messages woke the thread (proof of "sleeps until Windows says something").
    pub wakes: Arc<AtomicU64>,
}

struct Ctx {
    sink: WakeSink,
    wakes: Arc<AtomicU64>,
    /// the display went off since it was last on (`GUID_CONSOLE_DISPLAY_STATE`); None = no state seen yet
    was_off: Option<bool>,
}

thread_local! {
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

impl PowerEvents {
    /// Starts the window thread; `sink` gets every event (on that thread: it must return quickly).
    pub fn start(sink: WakeSink) -> Result<PowerEvents> {
        let wakes = Arc::new(AtomicU64::new(0));
        let (tx, rx) = std::sync::mpsc::channel::<Result<u32>>();
        let w2 = wakes.clone();
        let join = std::thread::Builder::new()
            .name("bu-display-power".into())
            .spawn(move || run(sink, w2, tx))
            .map_err(|e| DisplayError::Watcher(format!("thread: {e}")))?;
        match rx.recv() {
            Ok(Ok(thread)) => Ok(PowerEvents { thread, join: Some(join), wakes }),
            Ok(Err(e)) => {
                let _ = join.join();
                Err(e)
            }
            Err(_) => Err(DisplayError::Watcher("power event thread ended".into())),
        }
    }
}

impl Drop for PowerEvents {
    fn drop(&mut self) {
        // SAFETY: posting to our own thread's queue; its loop ends on WM_QUIT and removes everything it registered.
        let _ = unsafe { PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn run(sink: WakeSink, wakes: Arc<AtomicU64>, ready: std::sync::mpsc::Sender<Result<u32>>) {
    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let class = w!("BoylerUtilitiesBrightnessLock");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: class, ..Default::default() };
        RegisterClassW(&wc);
        // a top-level window (broadcasts only reach those), never shown
        let hwnd = match CreateWindowExW(WS_EX_TOOLWINDOW, class, PCWSTR::null(), WS_POPUP, 0, 0, 0, 0, None, None, Some(inst.into()), None) {
            Ok(h) => h,
            Err(e) => {
                let _ = ready.send(Err(DisplayError::os("CreateWindowExW", e)));
                return;
            }
        };
        CTX.with(|c| *c.borrow_mut() = Some(Ctx { sink, wakes, was_off: None }));
        let recipient = HANDLE(hwnd.0);
        let power = RegisterPowerSettingNotification(recipient, &GUID_CONSOLE_DISPLAY_STATE, DEVICE_NOTIFY_WINDOW_HANDLE).ok();
        let wts = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION).is_ok();
        let filter = DEV_BROADCAST_DEVICEINTERFACE_W {
            dbcc_size: size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
            dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
            dbcc_classguid: GUID_DEVINTERFACE_MONITOR,
            ..Default::default()
        };
        let dev = RegisterDeviceNotificationW(recipient, &filter as *const _ as *const core::ffi::c_void, DEVICE_NOTIFY_WINDOW_HANDLE).ok();
        let _ = ready.send(Ok(GetCurrentThreadId()));
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if let Some(p) = power {
            let _ = UnregisterPowerSettingNotification(p);
        }
        if wts {
            let _ = WTSUnRegisterSessionNotification(hwnd);
        }
        if let Some(d) = dev {
            let _ = UnregisterDeviceNotification(d);
        }
        let _ = DestroyWindow(hwnd);
        CTX.with(|c| *c.borrow_mut() = None);
    }
}

/// Hands one event to the sink (skipped, never a panic, if a Windows callback ever re-entered).
fn send(ev: Option<WakeEvent>) {
    CTX.with(|c| {
        if let Ok(mut b) = c.try_borrow_mut() {
            if let Some(c) = b.as_mut() {
                c.wakes.fetch_add(1, Ordering::Relaxed);
                if let Some(ev) = ev {
                    (c.sink)(ev);
                }
            }
        }
    });
}

/// `GUID_CONSOLE_DISPLAY_STATE`: 0 off, 1 on, 2 dimmed. On after off = an event; the first state seen is not.
pub fn display_state_event(was_off: &mut Option<bool>, state: u32) -> Option<WakeEvent> {
    let before = *was_off;
    match state {
        0 => *was_off = Some(true),
        1 => *was_off = Some(false),
        _ => {
            was_off.get_or_insert(false);
        }
    }
    (state == 1 && before == Some(true)).then_some(WakeEvent::DisplayOn)
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_POWERBROADCAST => {
            match wp.0 as u32 {
                PBT_POWERSETTINGCHANGE if lp.0 != 0 => {
                    // SAFETY: Windows passes a POWERBROADCAST_SETTING with this message; its data is one DWORD here
                    let s = unsafe { &*(lp.0 as *const POWERBROADCAST_SETTING) };
                    if s.PowerSetting == GUID_CONSOLE_DISPLAY_STATE && s.DataLength >= 4 {
                        let state = unsafe { std::ptr::read_unaligned(s.Data.as_ptr() as *const u32) };
                        let ev = CTX.with(|c| c.try_borrow_mut().ok().and_then(|mut b| b.as_mut().and_then(|c| display_state_event(&mut c.was_off, state))));
                        send(ev);
                    }
                }
                PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => send(Some(WakeEvent::Resume)),
                _ => send(None),
            }
            LRESULT(1)
        }
        WM_WTSSESSION_CHANGE => {
            send((wp.0 as u32 == WTS_SESSION_UNLOCK).then_some(WakeEvent::Unlock));
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            send(Some(WakeEvent::DisplayChange));
            LRESULT(0)
        }
        WM_DEVICECHANGE => {
            send((wp.0 as u32 == DBT_DEVICEARRIVAL).then_some(WakeEvent::MonitorArrived));
            LRESULT(1)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}
