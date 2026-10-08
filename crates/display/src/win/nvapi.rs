//! NVIDIA digital vibrance through NVAPI's DVC calls (as vibranceGUI does). Not in NVIDIA's public docs: the
//! function ids come from NvAPIWrapper (github.com/falahati/NvAPIWrapper, Native/Helpers/FunctionId.cs):
//!   NvAPI_Initialize 0x0150E828 · NvAPI_GetAssociatedNvidiaDisplayHandle 0x35C29134
//!   NvAPI_GetDVCInfoEx 0x0E45002D · NvAPI_SetDVCLevelEx 0x4A82C2B1 · NvAPI_GetDVCInfo 0x4085DE45 · NvAPI_SetDVCLevel 0x172409B4
//! Struct layouts: NvAPIWrapper PrivateDisplayDVCInfo(Ex).cs (version = size | 1 << 16).
//! The "Ex" pair carries min / max / default (range below the normal level); the old pair is the fallback (0..63, 0 = normal).
//! nvapi64.dll ships with every NVIDIA driver; no admin. FACEIT lists VibranceGUI as allowed (research big-C §0).
//! Known gap (research): not on NVIDIA laptops (Optimus — the panel is driven by the Intel/AMD iGPU).

use crate::error::{DisplayError, Result};
use crate::types::{GpuVendor, VibranceRaw};
use std::ffi::{c_void, CString};
use std::sync::OnceLock;
use windows::core::{s, w};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};

type QueryInterface = unsafe extern "C" fn(u32) -> *const c_void;
type Initialize = unsafe extern "C" fn() -> i32;
type GetHandle = unsafe extern "C" fn(*const i8, *mut *mut c_void) -> i32;
type GetDvcEx = unsafe extern "C" fn(*mut c_void, u32, *mut DvcInfoEx) -> i32;
type SetDvcEx = unsafe extern "C" fn(*mut c_void, u32, *mut DvcInfoEx) -> i32;
type GetDvc = unsafe extern "C" fn(*mut c_void, u32, *mut DvcInfo) -> i32;
type SetDvc = unsafe extern "C" fn(*mut c_void, u32, i32) -> i32;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct DvcInfo {
    version: u32,
    current: i32,
    min: i32,
    max: i32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct DvcInfoEx {
    version: u32,
    current: i32,
    min: i32,
    max: i32,
    default: i32,
}

const fn ver<T>() -> u32 {
    size_of::<T>() as u32 | (1 << 16)
}

struct Api {
    get_handle: GetHandle,
    get_ex: Option<GetDvcEx>,
    set_ex: Option<SetDvcEx>,
    get: Option<GetDvc>,
    set: Option<SetDvc>,
}

// Function pointers into a DLL that stays loaded for the life of the process.
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        // The driver installs it into System32; never searched in the exe folder / current folder / PATH (TECH_RULES).
        let lib = LoadLibraryExW(w!("nvapi64.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
        let qi: QueryInterface = std::mem::transmute(GetProcAddress(lib, s!("nvapi_QueryInterface"))?);
        let f = |id: u32| {
            let p = qi(id);
            (!p.is_null()).then_some(p)
        };
        let init: Initialize = std::mem::transmute(f(0x0150_E828)?);
        if init() != 0 {
            return None;
        }
        Some(Api {
            get_handle: std::mem::transmute::<*const c_void, GetHandle>(f(0x35C2_9134)?),
            get_ex: f(0x0E45_002D).map(|p| std::mem::transmute::<*const c_void, GetDvcEx>(p)),
            set_ex: f(0x4A82_C2B1).map(|p| std::mem::transmute::<*const c_void, SetDvcEx>(p)),
            get: f(0x4085_DE45).map(|p| std::mem::transmute::<*const c_void, GetDvc>(p)),
            set: f(0x1724_09B4).map(|p| std::mem::transmute::<*const c_void, SetDvc>(p)),
        })
    })
    .as_ref()
}

fn handle(api: &Api, gdi_name: &str) -> Result<*mut c_void> {
    let name = CString::new(gdi_name).map_err(|_| DisplayError::VibranceUnsupported(Some(GpuVendor::Nvidia)))?;
    let mut h: *mut c_void = std::ptr::null_mut();
    let r = unsafe { (api.get_handle)(name.as_ptr(), &mut h) };
    if r != 0 || h.is_null() {
        return Err(DisplayError::os("NvAPI_GetAssociatedNvidiaDisplayHandle", format!("status {r}")));
    }
    Ok(h)
}

pub(crate) fn get(gdi_name: &str) -> Result<VibranceRaw> {
    let api = api().ok_or(DisplayError::VibranceUnsupported(Some(GpuVendor::Nvidia)))?;
    let h = handle(api, gdi_name)?;
    if let Some(get_ex) = api.get_ex {
        let mut i = DvcInfoEx { version: ver::<DvcInfoEx>(), ..Default::default() };
        if unsafe { get_ex(h, 0, &mut i) } == 0 && !(i.min == 0 && i.max == 0 && i.current == 0) {
            return Ok(VibranceRaw { vendor: GpuVendor::Nvidia, current: i.current, min: i.min, max: i.max, default: i.default });
        }
    }
    if let Some(get) = api.get {
        let mut i = DvcInfo { version: ver::<DvcInfo>(), ..Default::default() };
        let r = unsafe { get(h, 0, &mut i) };
        if r == 0 {
            return Ok(VibranceRaw { vendor: GpuVendor::Nvidia, current: i.current, min: i.min, max: i.max, default: 0 });
        }
        return Err(DisplayError::os("NvAPI_GetDVCInfo", format!("status {r}")));
    }
    Err(DisplayError::VibranceUnsupported(Some(GpuVendor::Nvidia)))
}

pub(crate) fn set(gdi_name: &str, level: i32) -> Result<()> {
    let api = api().ok_or(DisplayError::VibranceUnsupported(Some(GpuVendor::Nvidia)))?;
    let h = handle(api, gdi_name)?;
    if let (Some(get_ex), Some(set_ex)) = (api.get_ex, api.set_ex) {
        let mut i = DvcInfoEx { version: ver::<DvcInfoEx>(), ..Default::default() };
        if unsafe { get_ex(h, 0, &mut i) } == 0 && !(i.min == 0 && i.max == 0 && i.current == 0) {
            i.current = crate::picture::clamp_any(level, i.min, i.max);
            let r = unsafe { set_ex(h, 0, &mut i) };
            return if r == 0 { Ok(()) } else { Err(DisplayError::os("NvAPI_SetDVCLevelEx", format!("status {r}"))) };
        }
    }
    let set = api.set.ok_or(DisplayError::VibranceUnsupported(Some(GpuVendor::Nvidia)))?;
    let r = unsafe { set(h, 0, level) };
    if r == 0 { Ok(()) } else { Err(DisplayError::os("NvAPI_SetDVCLevel", format!("status {r}"))) }
}
