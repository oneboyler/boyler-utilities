//! A tiny read-only WMI client (COM `IWbemLocator` → `IWbemServices::ExecQuery`). Only plain property values.

use crate::{Result, StorageError};
use std::collections::HashMap;
use windows::core::{BSTR, PCWSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoSetProxyBlanket, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, EOAC_NONE,
    RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Variant::{VariantClear, VARIANT};
use windows::Win32::System::Wmi::{
    IEnumWbemClassObject, IWbemClassObject, IWbemLocator, IWbemServices, WbemLocator, WBEM_FLAG_FORWARD_ONLY,
    WBEM_FLAG_RETURN_IMMEDIATELY,
};

const RPC_C_AUTHN_WINNT: u32 = 10;
const RPC_C_AUTHZ_NONE: u32 = 0;
/// WBEM_E_ACCESS_DENIED
const ACCESS_DENIED: i32 = 0x8004_1003u32 as i32;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Str(String),
    Int(i64),
    Bool(bool),
    Float(f64),
    Other,
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }
    /// Numbers (WMI sends uint64 as a string, so strings that are numbers count too).
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Str(s) => s.trim().parse().ok(),
            Value::Float(f) => Some(*f as i64),
            Value::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }
}

pub type Row = HashMap<String, Value>;

/// One row of a query can take at most this long (a hung WMI service must not hang the menu).
const NEXT_TIMEOUT_MS: i32 = 10_000;
/// WBEM_S_TIMEDOUT: `Next` gave up waiting.
const WBEM_S_TIMEDOUT: i32 = 0x0004_0004;

pub struct Wmi {
    svc: Option<IWbemServices>,
    /// This value started COM on its thread and ends it again on drop.
    com: bool,
}

impl Drop for Wmi {
    fn drop(&mut self) {
        self.svc.take(); // release the COM object before COM goes down
        if self.com {
            unsafe { CoUninitialize() };
        }
    }
}

impl Wmi {
    /// Connect to a namespace, e.g. `root\cimv2` or `root\Microsoft\Windows\Storage`.
    pub fn connect(namespace: &str) -> Result<Wmi> {
        unsafe {
            // S_OK / S_FALSE must be balanced with CoUninitialize; RPC_E_CHANGED_MODE (COM already up in another
            // mode on this thread) must not be.
            let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            let mut me = Wmi { svc: None, com };
            let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).map_err(os_err("WMI locator"))?;
            let svc = locator
                .ConnectServer(&BSTR::from(namespace), &BSTR::new(), &BSTR::new(), &BSTR::new(), 0, &BSTR::new(), None)
                .map_err(os_err("WMI connect"))?;
            CoSetProxyBlanket(
                &svc,
                RPC_C_AUTHN_WINNT,
                RPC_C_AUTHZ_NONE,
                PCWSTR::null(),
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            )
            .map_err(os_err("WMI proxy blanket"))?;
            me.svc = Some(svc);
            Ok(me)
        }
    }

    /// Run a WQL query and read the named properties of every row. Access denied → `NeedsAdmin`.
    pub fn query(&self, wql: &str, props: &[&str]) -> Result<Vec<Row>> {
        unsafe {
            let Some(svc) = self.svc.as_ref() else { return Ok(Vec::new()) };
            let en: IEnumWbemClassObject = svc
                .ExecQuery(&BSTR::from("WQL"), &BSTR::from(wql), WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY, None)
                .map_err(|e| map(e, wql))?;
            let mut rows = Vec::new();
            loop {
                let mut objs = [None];
                let mut got = 0u32;
                let hr = en.Next(NEXT_TIMEOUT_MS, &mut objs, &mut got);
                if hr.is_err() {
                    return Err(map(windows::core::Error::from(hr), wql));
                }
                if hr.0 == WBEM_S_TIMEDOUT && got == 0 {
                    return Err(os_err("WMI timed out")(windows::core::Error::from_hresult(windows::core::HRESULT(0x8004_1032u32 as i32))));
                }
                if got == 0 {
                    break;
                }
                let Some(obj) = objs[0].take() else { break };
                rows.push(read_row(&obj, props));
            }
            Ok(rows)
        }
    }
}

fn map(e: windows::core::Error, what: &str) -> StorageError {
    if e.code().0 == ACCESS_DENIED || e.code().0 == 0x8007_0005u32 as i32 {
        StorageError::NeedsAdmin(format!("WMI: {what}"))
    } else {
        StorageError::Os { context: format!("WMI {what}"), code: e.code().0 as u32 }
    }
}

fn os_err(context: &'static str) -> impl Fn(windows::core::Error) -> StorageError {
    move |e| StorageError::Os { context: context.to_string(), code: e.code().0 as u32 }
}

unsafe fn read_row(obj: &IWbemClassObject, props: &[&str]) -> Row {
    let mut row = Row::new();
    for &p in props {
        let name: Vec<u16> = p.encode_utf16().chain(Some(0)).collect();
        let mut v = VARIANT::default();
        let value = if obj.Get(PCWSTR(name.as_ptr()), 0, &mut v, None, None).is_ok() { variant_value(&v) } else { Value::Null };
        let _ = VariantClear(&mut v);
        row.insert(p.to_string(), value);
    }
    row
}

unsafe fn variant_value(v: &VARIANT) -> Value {
    let inner = &v.Anonymous.Anonymous;
    let d = &inner.Anonymous;
    match inner.vt.0 {
        0 | 1 => Value::Null,
        8 => Value::Str(d.bstrVal.to_string()),
        2 => Value::Int(d.iVal as i64),
        3 | 22 => Value::Int(d.lVal as i64),
        16 => Value::Int(d.bVal as i8 as i64),
        17 => Value::Int(d.bVal as i64),
        18 => Value::Int(d.iVal as u16 as i64),
        19 | 23 => Value::Int(d.lVal as u32 as i64),
        20 | 21 => Value::Int(d.llVal),
        11 => Value::Bool(d.boolVal.0 != 0),
        4 => Value::Float(d.fltVal as f64),
        5 => Value::Float(d.dblVal),
        _ => Value::Other,
    }
}
