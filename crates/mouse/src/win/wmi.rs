//! WMI event subscriptions (ROOT\CIMV2) for the per-app acceleration switch — the SAME approach as bu-display's per-app
//! trigger (Order 004, `crates/display/src/win/wmi.rs`), re-written here because this crate must not depend on bu-display.
//! One thread per subscription sets up an ASYNC notification query and then blocks until it is told to stop — WMI calls
//! our sink when an event happens, so our side never wakes up on a timer.
//! - `Win32_ProcessStartTrace` (kernel trace, exact, at process creation) — needs ADMIN (the later elevated helper).
//! - `__InstanceCreationEvent WITHIN 1 … Win32_Process` — no admin; WMI itself re-checks the process list every second
//!   (bu-display measured the cost: ~1.3 % of one core in WmiPrvSE while a rule is on, A_004_02), so the event lands
//!   0–1 s after the start.
//! - `__InstanceDeletionEvent WITHIN 1 … ProcessId = n` — exit fallback when a process refuses a SYNCHRONIZE handle.
//!
//! Difference to bu-display (its review REVIEW_004_done_4c59204 found it): a refusal that arrives AFTER the first
//! 1.5 s wait is not lost — the subscription thread keeps listening for WMI's final status and reports a late failure
//! through `on_dead`, so the watcher can fall back to the next source instead of staying silently dead.

use crate::error::{Error, Result};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use windows::core::{implement, Interface, Ref, BSTR, HRESULT, PCWSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoSetProxyBlanket, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, EOAC_NONE,
    RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::Variant::{VariantClear, VARIANT, VT_BSTR, VT_I4, VT_UI4, VT_UNKNOWN};
use windows::Win32::System::Wmi::{
    IWbemClassObject, IWbemLocator, IWbemObjectSink, IWbemObjectSink_Impl, IWbemServices, WbemLocator, WBEM_FLAG_SEND_STATUS,
    WBEM_STATUS_COMPLETE,
};

pub(crate) type OnObject = Arc<dyn Fn(&IWbemClassObject) + Send + Sync>;
/// Called once if WMI ends a running subscription with a failure (e.g. a late "access denied").
pub(crate) type OnDead = Arc<dyn Fn(String) + Send + Sync>;

enum Msg {
    Status(HRESULT),
    Stop,
}

#[implement(IWbemObjectSink)]
struct Sink {
    on_object: OnObject,
    msgs: mpsc::Sender<Msg>,
}

impl IWbemObjectSink_Impl for Sink_Impl {
    fn Indicate(&self, count: i32, objs: *const Option<IWbemClassObject>) -> windows::core::Result<()> {
        if !objs.is_null() && count > 0 {
            for o in unsafe { std::slice::from_raw_parts(objs, count as usize) }.iter().flatten() {
                (self.on_object)(o);
            }
        }
        Ok(())
    }

    fn SetStatus(&self, flags: i32, hr: HRESULT, _param: &BSTR, _obj: Ref<IWbemClassObject>) -> windows::core::Result<()> {
        if flags == WBEM_STATUS_COMPLETE.0 {
            let _ = self.msgs.send(Msg::Status(hr));
        }
        Ok(())
    }
}

/// A running subscription; drop cancels it.
pub(crate) struct Subscription {
    stop: Option<mpsc::Sender<Msg>>,
    join: Option<JoinHandle<()>>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(Msg::Stop);
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn connect() -> windows::core::Result<IWbemServices> {
    let locator: IWbemLocator = unsafe { CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)? };
    let svc = unsafe { locator.ConnectServer(&BSTR::from(r"ROOT\CIMV2"), &BSTR::new(), &BSTR::new(), &BSTR::new(), 0, &BSTR::new(), None)? };
    unsafe {
        CoSetProxyBlanket(&svc, RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE, PCWSTR::null(), RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE)?;
    }
    Ok(svc)
}

/// How long `subscribe` waits for WMI to refuse the query before it counts the subscription as running. A later
/// refusal still arrives through `on_dead`.
const REFUSE_WAIT: Duration = Duration::from_millis(1500);

/// How long `subscribe` waits in total for WMI to connect and accept or refuse the query (TECH_RULES: WMI calls carry a
/// timeout).
const START_WAIT: Duration = Duration::from_secs(10);

/// Starts an async WQL notification query. Fails when WMI refuses it at once (e.g. `Win32_ProcessStartTrace` without admin).
pub(crate) fn subscribe(query: String, on_object: OnObject, on_dead: OnDead) -> Result<Subscription> {
    let (ready_tx, ready_rx) = mpsc::channel::<std::result::Result<(), String>>();
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    let stop_tx = msg_tx.clone();
    let join = std::thread::Builder::new()
        .name("bu-mouse-wmi".into())
        .spawn(move || {
            if let Err(e) = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok() {
                let _ = ready_tx.send(Err(format!("CoInitializeEx: {e}")));
                return;
            }
            let run = || -> std::result::Result<(), String> {
                let svc = connect().map_err(|e| format!("WMI connect: {e}"))?;
                let sink: IWbemObjectSink = Sink { on_object, msgs: msg_tx }.into();
                unsafe { svc.ExecNotificationQueryAsync(&BSTR::from("WQL"), &BSTR::from(query.as_str()), WBEM_FLAG_SEND_STATUS, None, &sink) }
                    .map_err(|e| format!("ExecNotificationQueryAsync: {e}"))?;
                // A refused query ends at once with a failing status; a live one sends nothing until it is cancelled.
                match msg_rx.recv_timeout(REFUSE_WAIT) {
                    Ok(Msg::Status(hr)) if hr.is_err() => return Err(format!("WMI refused the query: {}", windows::core::Error::from(hr))),
                    Ok(Msg::Status(_)) => return Err("WMI ended the query".into()),
                    Ok(Msg::Stop) => {
                        let _ = unsafe { svc.CancelAsyncCall(&sink) };
                        return Err("stopped before it started".into());
                    }
                    Err(_) => {}
                }
                let _ = ready_tx.send(Ok(()));
                // Block until stop or until WMI ends the query (late refusal). No wake-ups meanwhile.
                match msg_rx.recv() {
                    Ok(Msg::Status(hr)) => {
                        let why = if hr.is_err() { windows::core::Error::from(hr).to_string() } else { "WMI ended the query".into() };
                        on_dead(why);
                    }
                    Ok(Msg::Stop) | Err(_) => {
                        let _ = unsafe { svc.CancelAsyncCall(&sink) };
                    }
                }
                Ok(())
            };
            if let Err(e) = run() {
                let _ = ready_tx.send(Err(e));
            }
            unsafe { CoUninitialize() };
        })
        .map_err(|e| Error::Watcher(e.to_string()))?;
    // WMI connect can hang on a broken WMI service: give up after START_WAIT. The thread is told to stop (it cancels the
    // query the moment it gets that far) and is left to end on its own — never joined here, so the caller never hangs.
    match ready_rx.recv_timeout(START_WAIT) {
        Ok(Ok(())) => Ok(Subscription { stop: Some(stop_tx), join: Some(join) }),
        Ok(Err(e)) => {
            let _ = join.join();
            Err(Error::Watcher(e))
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let _ = stop_tx.send(Msg::Stop);
            Err(Error::Watcher(format!("WMI did not answer within {} s", START_WAIT.as_secs())))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(Error::Watcher("WMI thread ended".into())),
    }
}

pub(crate) enum Prop {
    Int(u32),
    Str(String),
    Obj(IWbemClassObject),
}

/// Reads a property; `None` if missing / null. Integers come as VT_I4 or VT_UI4, strings as VT_BSTR, objects as VT_UNKNOWN.
pub(crate) fn get(obj: &IWbemClassObject, name: &str) -> Option<Prop> {
    let wname: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut v = VARIANT::default();
    unsafe { obj.Get(PCWSTR(wname.as_ptr()), 0, &mut v, None, None) }.ok()?;
    let out = unsafe {
        let inner = &v.Anonymous.Anonymous;
        match inner.vt {
            VT_I4 => Some(Prop::Int(inner.Anonymous.lVal as u32)),
            VT_UI4 => Some(Prop::Int(inner.Anonymous.ulVal)),
            VT_BSTR => Some(Prop::Str(inner.Anonymous.bstrVal.to_string())),
            VT_UNKNOWN => inner.Anonymous.punkVal.as_ref().and_then(|u| u.cast::<IWbemClassObject>().ok()).map(Prop::Obj),
            _ => None,
        }
    };
    let _ = unsafe { VariantClear(&mut v) };
    out
}

pub(crate) fn get_int(obj: &IWbemClassObject, name: &str) -> Option<u32> {
    match get(obj, name)? {
        Prop::Int(i) => Some(i),
        _ => None,
    }
}

pub(crate) fn get_str(obj: &IWbemClassObject, name: &str) -> Option<String> {
    match get(obj, name)? {
        Prop::Str(s) => Some(s),
        _ => None,
    }
}

pub(crate) fn get_obj(obj: &IWbemClassObject, name: &str) -> Option<IWbemClassObject> {
    match get(obj, name)? {
        Prop::Obj(o) => Some(o),
        _ => None,
    }
}

/// WQL string literal: quotes and backslashes escaped.
pub(crate) fn wql_str(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}
