//! AMD saturation ("vibrance") through ADL, the AMD Display Library's plain C API in atiadlxx.dll (ships with every
//! AMD driver), the way vibranceGUI does it (vibrance.GUI/AMD/vendor/AmdAdapter64.cs, adl64/*.cs):
//!   ADL_Main_Control_Create → ADL_Adapter_NumberOfAdapters_Get → ADL_Adapter_AdapterInfo_Get → (adapter whose
//!   strDisplayName is our `\\.\DISPLAYn`) → ADL_Display_DisplayInfo_Get → ADL_Display_Color_Get/Set(SATURATION = 1<<2).
//! Struct layouts from the same files (AdapterInfo 1572 bytes, ADLDisplayInfo 552 bytes on Windows).
//! The research names ADLX `SetSaturation` (the newer C++ SDK); ADL is the same driver feature through a C API we can
//! load without the SDK. UNTESTED HERE: the test PC has an NVIDIA GPU, so this path is compiled but never ran.

use crate::error::{DisplayError, Result};
use crate::types::{GpuVendor, VibranceRaw};
use std::ffi::c_void;
use std::sync::OnceLock;
use windows::core::{s, w};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};

const ADL_MAX_PATH: usize = 256;
const ADL_DISPLAY_COLOR_SATURATION: i32 = 1 << 2;

#[repr(C)]
struct AdapterInfo {
    size: i32,
    adapter_index: i32,
    udid: [u8; ADL_MAX_PATH],
    bus_number: i32,
    device_number: i32,
    function_number: i32,
    vendor_id: i32,
    adapter_name: [u8; ADL_MAX_PATH],
    display_name: [u8; ADL_MAX_PATH],
    present: i32,
    exist: i32,
    driver_path: [u8; ADL_MAX_PATH],
    driver_path_ext: [u8; ADL_MAX_PATH],
    pnp_string: [u8; ADL_MAX_PATH],
    os_display_index: i32,
}
const _: () = assert!(size_of::<AdapterInfo>() == 1572);

#[repr(C)]
#[derive(Clone, Copy)]
struct DisplayId {
    logical_index: i32,
    physical_index: i32,
    logical_adapter_index: i32,
    physical_adapter_index: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DisplayInfo {
    id: DisplayId,
    controller_index: i32,
    name: [u8; ADL_MAX_PATH],
    manufacturer: [u8; ADL_MAX_PATH],
    display_type: i32,
    output_type: i32,
    connector: i32,
    info_mask: i32,
    info_value: i32,
}
const _: () = assert!(size_of::<DisplayInfo>() == 552);

type Proc = unsafe extern "system" fn() -> isize;
type MallocCb = unsafe extern "C" fn(i32) -> *mut c_void;
type ControlCreate = unsafe extern "C" fn(MallocCb, i32) -> i32;
type NumAdapters = unsafe extern "C" fn(*mut i32) -> i32;
type AdapterInfoGet = unsafe extern "C" fn(*mut AdapterInfo, i32) -> i32;
type DisplayInfoGet = unsafe extern "C" fn(i32, *mut i32, *mut *mut DisplayInfo, i32) -> i32;
type ColorGet = unsafe extern "C" fn(i32, i32, i32, *mut i32, *mut i32, *mut i32, *mut i32, *mut i32) -> i32;
type ColorSet = unsafe extern "C" fn(i32, i32, i32, i32) -> i32;

extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

unsafe extern "C" fn adl_malloc(size: i32) -> *mut c_void {
    unsafe { malloc(size.max(0) as usize) }
}

struct Api {
    num_adapters: NumAdapters,
    adapter_info: AdapterInfoGet,
    display_info: DisplayInfoGet,
    color_get: ColorGet,
    color_set: ColorSet,
}
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        // The driver installs it into System32; never searched in the exe folder / current folder / PATH (TECH_RULES).
        let lib = LoadLibraryExW(w!("atiadlxx.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
        let create: ControlCreate = std::mem::transmute(GetProcAddress(lib, s!("ADL_Main_Control_Create"))?);
        if create(adl_malloc, 1) != 0 {
            return None;
        }
        Some(Api {
            num_adapters: std::mem::transmute::<Proc, NumAdapters>(GetProcAddress(lib, s!("ADL_Adapter_NumberOfAdapters_Get"))?),
            adapter_info: std::mem::transmute::<Proc, AdapterInfoGet>(GetProcAddress(lib, s!("ADL_Adapter_AdapterInfo_Get"))?),
            display_info: std::mem::transmute::<Proc, DisplayInfoGet>(GetProcAddress(lib, s!("ADL_Display_DisplayInfo_Get"))?),
            color_get: std::mem::transmute::<Proc, ColorGet>(GetProcAddress(lib, s!("ADL_Display_Color_Get"))?),
            color_set: std::mem::transmute::<Proc, ColorSet>(GetProcAddress(lib, s!("ADL_Display_Color_Set"))?),
        })
    })
    .as_ref()
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// (adapter index, display logical index) for a GDI name.
fn locate(api: &Api, gdi_name: &str) -> Result<(i32, i32)> {
    let unsupported = || DisplayError::VibranceUnsupported(Some(GpuVendor::Amd));
    let mut n = 0;
    if unsafe { (api.num_adapters)(&mut n) } != 0 || n <= 0 {
        return Err(unsupported());
    }
    let mut infos: Vec<AdapterInfo> = (0..n).map(|_| unsafe { std::mem::zeroed() }).collect();
    let bytes = (size_of::<AdapterInfo>() * n as usize) as i32;
    if unsafe { (api.adapter_info)(infos.as_mut_ptr(), bytes) } != 0 {
        return Err(unsupported());
    }
    for a in infos.iter().filter(|a| cstr(&a.display_name).eq_ignore_ascii_case(gdi_name)) {
        let mut count = 0;
        let mut list: *mut DisplayInfo = std::ptr::null_mut();
        if unsafe { (api.display_info)(a.adapter_index, &mut count, &mut list, 0) } != 0 || list.is_null() {
            continue;
        }
        let found = unsafe { std::slice::from_raw_parts(list, count.max(0) as usize) }
            .iter()
            .find(|d| d.id.logical_adapter_index == a.adapter_index)
            .map(|d| (a.adapter_index, d.id.logical_index));
        unsafe { free(list.cast()) };
        if let Some(f) = found {
            return Ok(f);
        }
    }
    Err(unsupported())
}

pub(crate) fn get(gdi_name: &str) -> Result<VibranceRaw> {
    let api = api().ok_or(DisplayError::VibranceUnsupported(Some(GpuVendor::Amd)))?;
    let (ad, di) = locate(api, gdi_name)?;
    let (mut cur, mut def, mut min, mut max, mut step) = (0, 0, 0, 0, 0);
    let r = unsafe { (api.color_get)(ad, di, ADL_DISPLAY_COLOR_SATURATION, &mut cur, &mut def, &mut min, &mut max, &mut step) };
    if r != 0 {
        return Err(DisplayError::os("ADL_Display_Color_Get", format!("status {r}")));
    }
    Ok(VibranceRaw { vendor: GpuVendor::Amd, current: cur, min, max, default: def })
}

pub(crate) fn set(gdi_name: &str, level: i32) -> Result<()> {
    let api = api().ok_or(DisplayError::VibranceUnsupported(Some(GpuVendor::Amd)))?;
    let (ad, di) = locate(api, gdi_name)?;
    let r = unsafe { (api.color_set)(ad, di, ADL_DISPLAY_COLOR_SATURATION, level) };
    if r == 0 { Ok(()) } else { Err(DisplayError::os("ADL_Display_Color_Set", format!("status {r}"))) }
}
