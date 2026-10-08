//! HID: listing every HID interface (to identify the mouse) and one vendor request/answer exchange.
//! Listing opens each interface with NO read/write access (dwDesiredAccess = 0) — enough for its attributes, usage and
//! strings; nothing is sent to the device.

use crate::error::{Error, Result};
use crate::os::{HidInfo, HidTransfer};
use std::time::{Duration, Instant};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW, CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CR_BUFFER_SMALL, CR_SUCCESS,
};
use windows::Win32::Devices::HumanInterfaceDevice::*;
use windows::Win32::Foundation::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::System::IO::{CancelIo, GetOverlappedResult, OVERLAPPED};

fn interface_paths() -> Result<Vec<String>> {
    let guid = unsafe { HidD_GetHidGuid() };
    for _ in 0..4 {
        let mut len = 0u32;
        let r = unsafe { CM_Get_Device_Interface_List_SizeW(&mut len, &guid, PCWSTR::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT) };
        if r != CR_SUCCESS {
            return Err(Error::os("CM_Get_Device_Interface_List_Size", r.0 as i64));
        }
        let mut buf = vec![0u16; len as usize + 1];
        let r = unsafe { CM_Get_Device_Interface_ListW(&guid, PCWSTR::null(), &mut buf, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) };
        if r == CR_BUFFER_SMALL {
            continue; // a device arrived between the two calls
        }
        if r != CR_SUCCESS {
            return Err(Error::os("CM_Get_Device_Interface_List", r.0 as i64));
        }
        return Ok(buf.split(|c| *c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect());
    }
    Err(Error::os("CM_Get_Device_Interface_List (list kept changing)", 0))
}

/// `mi_02` in an interface path → 2.
pub fn interface_number(path: &str) -> Option<u8> {
    let l = path.to_ascii_lowercase();
    let i = l.find("&mi_")?;
    u8::from_str_radix(l.get(i + 4..i + 6)?, 16).ok()
}

fn hid_string(h: HANDLE, f: unsafe fn(HANDLE, *mut core::ffi::c_void, u32) -> bool) -> Option<String> {
    let mut buf = [0u16; 128];
    let ok = unsafe { f(h, buf.as_mut_ptr() as *mut _, (buf.len() * 2) as u32) };
    if !ok {
        return None;
    }
    let s = super::wide_to_string(&buf).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn open(path: &str, access: u32, flags: FILE_FLAGS_AND_ATTRIBUTES) -> windows::core::Result<HANDLE> {
    unsafe { CreateFileW(&HSTRING::from(path), access, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, flags, None) }
}

fn info(path: &str) -> Option<HidInfo> {
    let h = open(path, 0, FILE_FLAGS_AND_ATTRIBUTES(0)).ok()?;
    let mut a = HIDD_ATTRIBUTES { Size: size_of::<HIDD_ATTRIBUTES>() as u32, ..Default::default() };
    let got = unsafe { HidD_GetAttributes(h, &mut a) };
    let mut caps = HIDP_CAPS::default();
    let mut pp = PHIDP_PREPARSED_DATA::default();
    if unsafe { HidD_GetPreparsedData(h, &mut pp) } {
        let _ = unsafe { HidP_GetCaps(pp, &mut caps) };
        let _ = unsafe { HidD_FreePreparsedData(pp) };
    }
    let product = hid_string(h, HidD_GetProductString);
    let manufacturer = hid_string(h, HidD_GetManufacturerString);
    let _ = unsafe { CloseHandle(h) };
    if !got {
        return None;
    }
    Some(HidInfo {
        path: path.to_string(),
        vid: a.VendorID,
        pid: a.ProductID,
        version: a.VersionNumber,
        usage_page: caps.UsagePage,
        usage: caps.Usage,
        input_len: caps.InputReportByteLength,
        output_len: caps.OutputReportByteLength,
        feature_len: caps.FeatureReportByteLength,
        interface: interface_number(path),
        product,
        manufacturer,
    })
}

/// Every HID interface present (read-only).
pub fn list() -> Result<Vec<HidInfo>> {
    Ok(interface_paths()?.iter().filter_map(|p| info(p)).collect())
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// One vendor request → its answer. Only the full (menu) OS layer calls this.
pub fn exchange(path: &str, out: &[u8], how: &HidTransfer) -> Result<Vec<u8>> {
    let gone = |e: windows::core::Error| {
        if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() || e.code() == ERROR_DEVICE_NOT_CONNECTED.to_hresult() {
            Error::MouseGone(path.into())
        } else {
            super::hr("open the mouse", e)
        }
    };
    match how {
        HidTransfer::Feature { reply_id, reply_len } => {
            let h = Handle(open(path, (GENERIC_READ | GENERIC_WRITE).0, FILE_FLAGS_AND_ATTRIBUTES(0)).map_err(gone)?);
            if !unsafe { HidD_SetFeature(h.0, out.as_ptr() as *const _, out.len() as u32) } {
                return Err(Error::os("HidD_SetFeature", unsafe { GetLastError() }.0 as i64));
            }
            let mut buf = vec![0u8; *reply_len];
            if let Some(b) = buf.first_mut() {
                *b = *reply_id;
            }
            if !unsafe { HidD_GetFeature(h.0, buf.as_mut_ptr() as *mut _, buf.len() as u32) } {
                return Err(Error::os("HidD_GetFeature", unsafe { GetLastError() }.0 as i64));
            }
            Ok(buf)
        }
        HidTransfer::OutputThenInput { reply_len, max_reads, timeout_ms, echo } => {
            let h = Handle(open(path, (GENERIC_READ | GENERIC_WRITE).0, FILE_FLAG_OVERLAPPED).map_err(gone)?);
            let ev = Handle(unsafe { CreateEventW(None, true, false, None) }.map_err(|e| super::hr("CreateEvent", e))?);
            let deadline = Instant::now() + Duration::from_millis(*timeout_ms as u64);
            // write the request
            let mut ov = OVERLAPPED { hEvent: ev.0, ..Default::default() };
            let mut n = 0u32;
            let w = unsafe { WriteFile(h.0, Some(out), None, Some(&mut ov)) };
            if let Err(e) = w {
                if e.code() != ERROR_IO_PENDING.to_hresult() {
                    return Err(super::hr("WriteFile (mouse)", e));
                }
            }
            wait(&h, &ev, &mut ov, deadline, &mut n, "WriteFile (mouse)")?;
            // read input reports until one with the same report id (byte 0) as the request arrives
            for _ in 0..*max_reads {
                let mut buf = vec![0u8; *reply_len];
                let mut ov = OVERLAPPED { hEvent: ev.0, ..Default::default() };
                let r = unsafe { ReadFile(h.0, Some(&mut buf), None, Some(&mut ov)) };
                if let Err(e) = r {
                    if e.code() != ERROR_IO_PENDING.to_hresult() {
                        return Err(super::hr("ReadFile (mouse)", e));
                    }
                }
                let mut got = 0u32;
                wait(&h, &ev, &mut ov, deadline, &mut got, "ReadFile (mouse)")?;
                buf.truncate(got as usize);
                if buf.first() == out.first() && echo.map(|(i, v)| buf.get(i) == Some(&v)).unwrap_or(true) {
                    return Ok(buf);
                }
            }
            Err(Error::BadAnswer(format!("no answer with report id {:?} in {max_reads} reports", out.first())))
        }
    }
}

fn wait(h: &Handle, ev: &Handle, ov: &mut OVERLAPPED, deadline: Instant, n: &mut u32, op: &str) -> Result<()> {
    let left = deadline.saturating_duration_since(Instant::now()).as_millis() as u32;
    let r = unsafe { WaitForSingleObject(ev.0, left) };
    if r != WAIT_OBJECT_0 {
        let _ = unsafe { CancelIo(h.0) };
        // wait for the cancel to finish before the buffers go away
        let _ = unsafe { GetOverlappedResult(h.0, ov, n, true) };
        return Err(Error::BadAnswer(format!("{op}: the mouse did not answer in time")));
    }
    unsafe { GetOverlappedResult(h.0, ov, n, false) }.map_err(|e| super::hr(op, e))?;
    let _ = unsafe { windows::Win32::System::Threading::ResetEvent(ev.0) };
    Ok(())
}
