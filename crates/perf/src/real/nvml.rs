//! NVIDIA's NVML (`nvml.dll`, installed with the NVIDIA driver) for fan % (and temperature as a fallback).
//! Loaded from System32 only (no DLL search of the current folder), only while the live sampler runs.

use std::ffi::c_void;
use windows::core::{s, PCWSTR};
use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};

type Dev = *mut c_void;
type FnVoid = unsafe extern "C" fn() -> i32;
type FnCount = unsafe extern "C" fn(*mut u32) -> i32;
type FnHandle = unsafe extern "C" fn(u32, *mut Dev) -> i32;
type FnName = unsafe extern "C" fn(Dev, *mut u8, u32) -> i32;
type FnU32 = unsafe extern "C" fn(Dev, *mut u32) -> i32;
type FnTemp = unsafe extern "C" fn(Dev, u32, *mut u32) -> i32;

pub struct Nvml {
    lib: HMODULE,
    shutdown: FnVoid,
    devices: Vec<Dev>,
    names: Vec<String>,
    fan: Option<FnU32>,
    temp: Option<FnTemp>,
}

// NVML handles are used from the sampler thread only.
unsafe impl Send for Nvml {}

impl Nvml {
    pub fn load() -> Option<Nvml> {
        unsafe {
            let name: Vec<u16> = "nvml.dll".encode_utf16().chain(Some(0)).collect();
            let lib = LoadLibraryExW(PCWSTR(name.as_ptr()), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
            let get = |n: windows::core::PCSTR| GetProcAddress(lib, n);
            let init: FnVoid = std::mem::transmute(get(s!("nvmlInit_v2"))?);
            let shutdown: FnVoid = std::mem::transmute(get(s!("nvmlShutdown"))?);
            let count: FnCount = std::mem::transmute(get(s!("nvmlDeviceGetCount_v2"))?);
            let handle: FnHandle = std::mem::transmute(get(s!("nvmlDeviceGetHandleByIndex_v2"))?);
            let dev_name: FnName = std::mem::transmute(get(s!("nvmlDeviceGetName"))?);
            let fan: Option<FnU32> = get(s!("nvmlDeviceGetFanSpeed")).map(|f| std::mem::transmute(f));
            let temp: Option<FnTemp> = get(s!("nvmlDeviceGetTemperature")).map(|f| std::mem::transmute(f));
            if init() != 0 {
                let _ = FreeLibrary(lib);
                return None;
            }
            let mut n = 0u32;
            let _ = count(&mut n);
            let mut devices = Vec::new();
            let mut names = Vec::new();
            for i in 0..n.min(16) {
                let mut d: Dev = std::ptr::null_mut();
                if handle(i, &mut d) != 0 {
                    continue;
                }
                let mut buf = [0u8; 96];
                let nm = if dev_name(d, buf.as_mut_ptr(), buf.len() as u32) == 0 {
                    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                    String::from_utf8_lossy(&buf[..end]).into_owned()
                } else {
                    String::new()
                };
                devices.push(d);
                names.push(nm);
            }
            Some(Nvml { lib, shutdown, devices, names, fan, temp })
        }
    }

    /// Device names in NVML order ("NVIDIA GeForce RTX 4090").
    pub fn names(&self) -> Vec<String> {
        self.names.clone()
    }

    pub fn fan_pct(&self, i: u32) -> Option<u32> {
        let (f, d) = (self.fan?, *self.devices.get(i as usize)?);
        let mut v = 0u32;
        (unsafe { f(d, &mut v) } == 0).then_some(v)
    }

    pub fn temperature(&self, i: u32) -> Option<u32> {
        let (f, d) = (self.temp?, *self.devices.get(i as usize)?);
        let mut v = 0u32;
        (unsafe { f(d, 0, &mut v) } == 0).then_some(v) // NVML_TEMPERATURE_GPU = 0
    }
}

impl Drop for Nvml {
    fn drop(&mut self) {
        unsafe {
            (self.shutdown)();
            let _ = FreeLibrary(self.lib);
        }
    }
}
