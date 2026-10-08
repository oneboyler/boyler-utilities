//! WMI event subscriptions (ROOT\CIMV2) for the per-app trigger: one thread per subscription that sets up an ASYNC
//! notification query and then just blocks until it is told to stop — WMI calls our sink when an event happens, so
//! our side never wakes up on a timer.
//! - `Win32_ProcessStartTrace` (kernel trace, exact, at process creation) — needs ADMIN (the later elevated helper).
//!   Used only when this process runs elevated (Order 048). The no-admin WMI creation / deletion events (WMI re-read
//!   the process list every second, ~1.3 % of a core in WmiPrvSE) were replaced by `bu_procwatch` (window creation +
//!   process snapshot; SYNCHRONIZE exit waits).

use crate::error::{DisplayError, Result};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use windows::core::{implement, Ref, BSTR, HRESULT, PCWSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoSetProxyBlanket, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, EOAC_NONE,
    RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::Variant::{VariantClear, VARIANT, VT_BSTR, VT_I4, VT_UI4};
use windows::Win32::System::Wmi::{
    IWbemClassObject, IWbemLocator, IWbemObjectSink, IWbemObjectSink_Impl, IWbemServices, WbemLocator, WBEM_FLAG_SEND_STATUS,
    WBEM_STATUS_COMPLETE,
};

pub(crate) type OnObject = Arc<dyn Fn(&IWbemClassObject) + Send + Sync>;

/// Called once if a running subscription is ended by WMI (a late refusal, WMI restarted…), with the reason.
pub(crate) type OnLost = Arc<dyn Fn(String) + Send + Sync>;

/// What the subscription thread waits for: our stop, or WMI ending the query (its final status).
enum Msg {
    Stop,
    Status(HRESULT),
}

#[implement(IWbemObjectSink)]
struct Sink {
    on_object: OnObject,
    status: mpsc::Sender<Msg>,
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
            let _ = self.status.send(Msg::Status(hr));
        }
        Ok(())
    }
}

/// A running subscription; `stop()` (or drop) cancels it.
pub(crate) struct Subscription {
    stop: Option<mpsc::Sender<Msg>>,
    join: Option<JoinHandle<()>>,
}

impl Subscription {
    fn shutdown(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(Msg::Stop);
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.shutdown();
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

/// How long `subscribe` waits for WMI to refuse the query (e.g. access denied for the admin-only trace) before it
/// counts the subscription as running (`display-trigger` prints how long the refusal really took). A refusal that comes
/// later is not lost: it ends the subscription and calls `on_lost`, so the watcher moves on to its next source.
const REFUSE_WAIT: Duration = Duration::from_millis(1500);

/// Starts an async WQL notification query. Fails when WMI refuses it (e.g. `Win32_ProcessStartTrace` without admin).
/// If WMI ends it later, `on_lost` is called once (from the subscription's own thread) and the thread ends.
pub(crate) fn subscribe(query: String, on_object: OnObject, on_lost: Option<OnLost>) -> Result<Subscription> {
    let (ready_tx, ready_rx) = mpsc::channel::<std::result::Result<(), String>>();
    let (stop_tx, stop_rx) = mpsc::channel::<Msg>();
    let status_tx = stop_tx.clone();
    let join = std::thread::Builder::new()
        .name("bu-display-wmi".into())
        .spawn(move || {
            if let Err(e) = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok() {
                let _ = ready_tx.send(Err(format!("CoInitializeEx: {e}")));
                return;
            }
            let run = || -> std::result::Result<(), String> {
                let svc = connect().map_err(|e| format!("WMI connect: {e}"))?;
                let sink: IWbemObjectSink = Sink { on_object, status: status_tx }.into();
                unsafe { svc.ExecNotificationQueryAsync(&BSTR::from("WQL"), &BSTR::from(query.as_str()), WBEM_FLAG_SEND_STATUS, None, &sink) }
                    .map_err(|e| format!("ExecNotificationQueryAsync: {e}"))?;
                // A refused query ends quickly with a final status; a live one sends nothing until it is cancelled.
                // (The caller can't have sent Stop yet: it is still waiting for `ready`.)
                match stop_rx.recv_timeout(REFUSE_WAIT) {
                    Ok(Msg::Status(hr)) => return Err(ended(hr)),
                    Ok(Msg::Stop) => return Ok(()),
                    Err(_) => {}
                }
                let _ = ready_tx.send(Ok(()));
                // Block until our stop OR WMI ending the query. No wake-ups meanwhile.
                match stop_rx.recv() {
                    Ok(Msg::Status(hr)) => {
                        if let Some(lost) = on_lost {
                            lost(ended(hr));
                        }
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
        .map_err(|e| DisplayError::Watcher(e.to_string()))?;
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(Subscription { stop: Some(stop_tx), join: Some(join) }),
        Ok(Err(e)) => {
            let _ = join.join();
            Err(DisplayError::Watcher(e))
        }
        Err(_) => Err(DisplayError::Watcher("WMI thread ended".into())),
    }
}

fn ended(hr: HRESULT) -> String {
    if hr.is_err() { format!("WMI refused / ended the query: {}", windows::core::Error::from(hr)) } else { "WMI ended the query".into() }
}

/// Reads a property; `None` if missing / null. Integers come as VT_I4 or VT_UI4, strings as VT_BSTR.
pub(crate) enum Prop {
    Int(u32),
    Str(String),
}

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

/// WQL string literal: quotes and backslashes escaped.
pub(crate) fn wql_str(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}
