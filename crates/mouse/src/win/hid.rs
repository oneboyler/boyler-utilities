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
    let mut all: Vec<HidInfo> = interface_paths()?.iter().filter_map(|p| info(p)).collect();
    let extra = raw_input_mice(&all);
    all.extend(extra);
    Ok(all)
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
        HidTransfer::FeatureOut => {
            let h = Handle(open_feature(path).map_err(gone)?);
            set_feature(&h, out)?;
            Ok(Vec::new())
        }
        HidTransfer::OutputOnly => {
            let h = Handle(open(path, (GENERIC_READ | GENERIC_WRITE).0, FILE_FLAG_OVERLAPPED).map_err(gone)?);
            let ev = Handle(unsafe { CreateEventW(None, true, false, None) }.map_err(|e| super::hr("CreateEvent", e))?);
            let mut ov = OVERLAPPED { hEvent: ev.0, ..Default::default() };
            let mut n = 0u32;
            if let Err(e) = unsafe { WriteFile(h.0, Some(out), None, Some(&mut ov)) } {
                if e.code() != ERROR_IO_PENDING.to_hresult() {
                    return Err(super::hr("WriteFile (mouse)", e));
                }
            }
            wait(&h, &ev, &mut ov, Instant::now() + Duration::from_millis(1000), &mut n, "WriteFile (mouse)")?;
            Ok(Vec::new())
        }
        HidTransfer::Feature { reply_id, reply_len, wait_ms } => {
            let h = Handle(open_feature(path).map_err(gone)?);
            set_feature(&h, out)?;
            if *wait_ms > 0 {
                std::thread::sleep(Duration::from_millis(*wait_ms as u64));
            }
            get_feature(&h, *reply_id, *reply_len)
        }
        HidTransfer::FeatureIn { reply_id, reply_len, wait_ms } => {
            // nothing is sent: `out` was sent by the transfer before this one
            let h = Handle(open_feature(path).map_err(gone)?);
            if *wait_ms > 0 {
                std::thread::sleep(Duration::from_millis(*wait_ms as u64));
            }
            get_feature(&h, *reply_id, *reply_len)
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
        HidTransfer::OutputThenMatch { reply_len, max_reads, timeout_ms, prefixes } => output_then_match(path, out, *reply_len, *max_reads, *timeout_ms, prefixes),
        HidTransfer::OutputThenStream { reply_len, max_reads, timeout_ms, idle_ms } => output_then_stream(path, out, *reply_len, *max_reads, *timeout_ms, *idle_ms),
    }
}

/// `HidTransfer::OutputThenMatch`: WriteFile(out), then ReadFile input reports on the same handle until one starts with one
/// of `prefixes` (other reports - notifications - are skipped).
fn output_then_match(path: &str, out: &[u8], reply_len: usize, max_reads: u32, timeout_ms: u32, prefixes: &[Vec<u8>]) -> Result<Vec<u8>> {
    let gone = |e: windows::core::Error| {
        if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() || e.code() == ERROR_DEVICE_NOT_CONNECTED.to_hresult() {
            Error::MouseGone(path.into())
        } else {
            super::hr("open the mouse", e)
        }
    };
    let h = Handle(open(path, (GENERIC_READ | GENERIC_WRITE).0, FILE_FLAG_OVERLAPPED).map_err(gone)?);
    let ev = Handle(unsafe { CreateEventW(None, true, false, None) }.map_err(|e| super::hr("CreateEvent", e))?);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
    let mut ov = OVERLAPPED { hEvent: ev.0, ..Default::default() };
    let mut n = 0u32;
    if let Err(e) = unsafe { WriteFile(h.0, Some(out), None, Some(&mut ov)) } {
        if e.code() != ERROR_IO_PENDING.to_hresult() {
            return Err(super::hr("WriteFile (mouse)", e));
        }
    }
    wait(&h, &ev, &mut ov, deadline, &mut n, "WriteFile (mouse)")?;
    for _ in 0..max_reads {
        let mut buf = vec![0u8; reply_len];
        let mut ov = OVERLAPPED { hEvent: ev.0, ..Default::default() };
        if let Err(e) = unsafe { ReadFile(h.0, Some(&mut buf), None, Some(&mut ov)) } {
            if e.code() != ERROR_IO_PENDING.to_hresult() {
                return Err(super::hr("ReadFile (mouse)", e));
            }
        }
        let mut got = 0u32;
        wait(&h, &ev, &mut ov, deadline, &mut got, "ReadFile (mouse)")?;
        buf.truncate(got as usize);
        if prefixes.iter().any(|p| buf.len() >= p.len() && buf[..p.len()] == p[..]) {
            return Ok(buf);
        }
    }
    Err(Error::BadAnswer(format!("no matching answer in {max_reads} reports")))
}

/// `HidTransfer::OutputThenStream`: WriteFile(out), then ReadFile input reports on the same handle, all kept, until `max_reads`
/// came, `timeout_ms` passed or `idle_ms` went by with no new report after the first.
fn output_then_stream(path: &str, out: &[u8], reply_len: usize, max_reads: u32, timeout_ms: u32, idle_ms: u32) -> Result<Vec<u8>> {
    let gone = |e: windows::core::Error| {
        if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() || e.code() == ERROR_DEVICE_NOT_CONNECTED.to_hresult() {
            Error::MouseGone(path.into())
        } else {
            super::hr("open the mouse", e)
        }
    };
    let h = Handle(open(path, (GENERIC_READ | GENERIC_WRITE).0, FILE_FLAG_OVERLAPPED).map_err(gone)?);
    let ev = Handle(unsafe { CreateEventW(None, true, false, None) }.map_err(|e| super::hr("CreateEvent", e))?);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
    let mut ov = OVERLAPPED { hEvent: ev.0, ..Default::default() };
    let mut n = 0u32;
    if let Err(e) = unsafe { WriteFile(h.0, Some(out), None, Some(&mut ov)) } {
        if e.code() != ERROR_IO_PENDING.to_hresult() {
            return Err(super::hr("WriteFile (mouse)", e));
        }
    }
    wait(&h, &ev, &mut ov, deadline, &mut n, "WriteFile (mouse)")?;
    if max_reads == 0 {
        return Err(Error::BadAnswer("OutputThenStream: max_reads is 0".into()));
    }
    let mut all = Vec::new();
    let mut count = 0u32;
    while count < max_reads {
        let mut buf = vec![0u8; reply_len];
        let mut ov = OVERLAPPED { hEvent: ev.0, ..Default::default() };
        if let Err(e) = unsafe { ReadFile(h.0, Some(&mut buf), None, Some(&mut ov)) } {
            if e.code() != ERROR_IO_PENDING.to_hresult() {
                return Err(super::hr("ReadFile (mouse)", e));
            }
        }
        // after the first report only `idle_ms` of silence is waited for
        let limit = if count == 0 { deadline } else { deadline.min(Instant::now() + Duration::from_millis(idle_ms as u64)) };
        let mut got = 0u32;
        if let Err(e) = wait(&h, &ev, &mut ov, limit, &mut got, "ReadFile (mouse)") {
            // silence after the first report ends the burst; a real read error (device gone ...) is an error
            if count == 0 || !matches!(e, Error::BadAnswer(_)) {
                return Err(e);
            }
            break;
        }
        buf.truncate(got as usize);
        all.extend(buf);
        count += 1;
    }
    Ok(all)
}

/// A feature-report handle (overlapped, so the reply fetch can time out). Windows keeps a mouse's own collections (usage
/// page 1 / usage 2 - where Razer talks) for itself and refuses read/write access there; feature reports still work on a
/// handle opened with no access rights (hidapi's fallback), so that is the second try.
fn open_feature(path: &str) -> windows::core::Result<HANDLE> {
    open(path, (GENERIC_READ | GENERIC_WRITE).0, FILE_FLAG_OVERLAPPED).or_else(|e| if e.code() == ERROR_ACCESS_DENIED.to_hresult() { open(path, 0, FILE_FLAG_OVERLAPPED) } else { Err(e) })
}

fn set_feature(h: &Handle, out: &[u8]) -> Result<()> {
    // (hidapi sends feature reports the same way, on its overlapped handle)
    if !unsafe { HidD_SetFeature(h.0, out.as_ptr() as *const _, out.len() as u32) } {
        return Err(Error::os("HidD_SetFeature", unsafe { GetLastError() }.0 as i64));
    }
    Ok(())
}

/// IOCTL_HID_GET_FEATURE (hidclass.h: HID_OUT_CTL_CODE(100)) - what HidD_GetFeature sends, here overlapped.
const IOCTL_HID_GET_FEATURE: u32 = 0x000B_0192;
/// The longest a mouse may take to hand over a feature report.
const FEATURE_TIMEOUT_MS: u64 = 1000;

/// HidD_GetFeature with a time limit (hidapi's hid_get_feature_report: the buffer goes in and out, byte 0 = report id): a
/// mouse that stalls the request can't hold the worker thread - it is cancelled after [`FEATURE_TIMEOUT_MS`].
fn get_feature(h: &Handle, reply_id: u8, reply_len: usize) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; reply_len];
    if let Some(b) = buf.first_mut() {
        *b = reply_id;
    }
    let ev = Handle(unsafe { CreateEventW(None, true, false, None) }.map_err(|e| super::hr("CreateEvent", e))?);
    let mut ov = OVERLAPPED { hEvent: ev.0, ..Default::default() };
    let mut n = 0u32;
    let p = buf.as_mut_ptr() as *mut core::ffi::c_void;
    let len = buf.len() as u32;
    if let Err(e) = unsafe { windows::Win32::System::IO::DeviceIoControl(h.0, IOCTL_HID_GET_FEATURE, Some(p as *const _), len, Some(p), len, None, Some(&mut ov)) } {
        if e.code() != ERROR_IO_PENDING.to_hresult() {
            return Err(super::hr("HidD_GetFeature", e));
        }
    }
    wait(h, &ev, &mut ov, Instant::now() + Duration::from_millis(FEATURE_TIMEOUT_MS), &mut n, "HidD_GetFeature (mouse)")?;
    Ok(buf)
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

/// The device paths of every mouse Windows' Raw Input lists (`RIM_TYPEMOUSE`): the second source for "which mice are
/// connected", for a mouse whose HID mouse collection did not show in the HID list (Order 061: a friend's Pulsar X2 V2 was
/// not found at all). Reads Windows' list only; nothing is sent to any device.
fn raw_input_mouse_paths() -> Vec<String> {
    use windows::Win32::UI::Input::{GetRawInputDeviceInfoW, GetRawInputDeviceList, RAWINPUTDEVICELIST, RIDI_DEVICENAME, RIM_TYPEMOUSE};
    let sz = size_of::<RAWINPUTDEVICELIST>() as u32;
    let mut n = 0u32;
    if unsafe { GetRawInputDeviceList(None, &mut n, sz) } == u32::MAX || n == 0 {
        return Vec::new();
    }
    let mut list = vec![RAWINPUTDEVICELIST::default(); n as usize + 8];
    let mut n2 = list.len() as u32;
    let got = unsafe { GetRawInputDeviceList(Some(list.as_mut_ptr()), &mut n2, sz) };
    if got == u32::MAX {
        return Vec::new();
    }
    let mut out = Vec::new();
    for d in list.iter().take(got as usize).filter(|d| d.dwType == RIM_TYPEMOUSE) {
        let mut len = 0u32;
        if unsafe { GetRawInputDeviceInfoW(Some(d.hDevice), RIDI_DEVICENAME, None, &mut len) } == u32::MAX || len == 0 {
            continue;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let r = unsafe { GetRawInputDeviceInfoW(Some(d.hDevice), RIDI_DEVICENAME, Some(buf.as_mut_ptr() as *mut _), &mut len) };
        if r == u32::MAX {
            continue;
        }
        out.push(super::wide_to_string(&buf));
    }
    out
}

/// `VID_3710&PID_5406` (USB) or `VID&0002046d_PID&b037` (Bluetooth: 4-digit source + vendor id) in a device path → (vid, pid).
pub fn ids_from_path(path: &str) -> Option<(u16, u16)> {
    let l = path.to_ascii_lowercase();
    let hex = |s: &str, from: usize, n: usize| -> Option<u16> { u16::from_str_radix(s.get(from..from + n)?, 16).ok() };
    if let (Some(v), Some(p)) = (l.find("vid_"), l.find("pid_")) {
        return Some((hex(&l, v + 4, 4)?, hex(&l, p + 4, 4)?));
    }
    if let (Some(v), Some(p)) = (l.find("vid&"), l.find("pid&")) {
        // the Bluetooth form: 2 hex digits of vendor-id source + 4 of vendor id ("0002046d"), 4 of product id
        return Some((hex(&l, v + 8, 4)?, hex(&l, p + 4, 4)?));
    }
    None
}

/// Mice that Windows (Raw Input) knows but the HID list has no mouse collection for, as `HidInfo` entries with the mouse's
/// usage (1 / 2). A mouse with no USB id at all (touchpad, PS/2, a remote session) comes as vid 0 / pid 0 - one entry.
/// Virtual devices (remote desktop, root-enumerated) are left out.
pub fn raw_input_mice(known: &[HidInfo]) -> Vec<HidInfo> {
    let mut out: Vec<HidInfo> = Vec::new();
    for path in raw_input_mouse_paths() {
        let l = path.to_ascii_lowercase();
        if l.contains("rdp_mou") || l.contains("#root#") || l.contains("\root#") || l.contains("root#") {
            continue;
        }
        let (vid, pid) = ids_from_path(&path).unwrap_or((0, 0));
        let seen = |h: &HidInfo| h.vid == vid && h.pid == pid && h.usage_page == 0x01 && h.usage == 0x02;
        if known.iter().any(seen) || out.iter().any(seen) {
            continue;
        }
        let mut h = info(&path).unwrap_or(HidInfo {
            path: path.clone(),
            vid,
            pid,
            version: 0,
            usage_page: 0,
            usage: 0,
            input_len: 0,
            output_len: 0,
            feature_len: 0,
            interface: interface_number(&path),
            product: None,
            manufacturer: None,
        });
        // the ids of the path win when the device would not open (they are what Windows says it is)
        h.vid = vid;
        h.pid = pid;
        h.usage_page = 0x01;
        h.usage = 0x02;
        out.push(h);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_come_out_of_usb_and_bluetooth_paths() {
        assert_eq!(ids_from_path(r"\?\HID#VID_3710&PID_5406&MI_00#7&1a2b#{378de44c-56ef-11d1-bc8c-00a0c91405dd}"), Some((0x3710, 0x5406)));
        assert_eq!(ids_from_path(r"\?\HID#{00001124-0000-1000-8000-00805f9b34fb}_VID&0002046d_PID&b037#9&2&0000"), Some((0x046D, 0xB037)));
        assert_eq!(ids_from_path(r"\?\ACPI#PNP0F13#4&1#{378de44c}"), None);
    }
}
