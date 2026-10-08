//! Monitors from DXGI (every GPU's outputs that are part of the desktop).
//!
//! DPI-correct: the enumeration runs with the thread set to per-monitor DPI awareness v2, so every rectangle is in physical
//! pixels — the real resolution, never a scaled "virtual" size (a 3440×1440 monitor at 150 % stays 3440×1440). The proof
//! (examples/show) prints each monitor's DXGI rectangle next to its current display mode (EnumDisplaySettings) and its
//! GetMonitorInfo rectangle so they can be compared.
//!
//! HDR: IDXGIOutput6::GetDesc1 — colour space `RGB_FULL_G2084_NONE_P2020` = Windows HD Color is on. The monitor's "SDR content
//! brightness" comes from DisplayConfigGetDeviceInfo(GET_SDR_WHITE_LEVEL) (1000 = 80 nits).

use std::collections::HashMap;

use windows::core::Interface;
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig, DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SDR_WHITE_LEVEL, DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, RECT};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, DXGI_MODE_ROTATION_ROTATE180, DXGI_MODE_ROTATION_ROTATE270,
    DXGI_MODE_ROTATION_ROTATE90,
};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, IDXGIOutput6, DXGI_ERROR_NOT_FOUND};
use windows::Win32::Graphics::Gdi::{
    EnumDisplaySettingsW, GetMonitorInfoW, DEVMODEW, ENUM_CURRENT_SETTINGS, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{
    GetDpiForMonitor, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    MDT_EFFECTIVE_DPI,
};
use windows::core::PCWSTR;

use super::Ctx;
use crate::error::Result;
use crate::geom::{self, Monitor, Rect, Rotation};

/// One DXGI output: what Desktop Duplication needs (its GPU and output) plus the engine's view of it.
pub struct Output {
    pub adapter: IDXGIAdapter1,
    pub output: IDXGIOutput,
    pub monitor: Monitor,
}

/// Sets the thread to per-monitor DPI awareness v2 for its lifetime, then restores the old setting.
pub(crate) struct DpiScope {
    old: DPI_AWARENESS_CONTEXT,
}

impl DpiScope {
    pub(crate) fn per_monitor() -> Self {
        DpiScope { old: unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } }
    }
}

impl Drop for DpiScope {
    fn drop(&mut self) {
        if !self.old.0.is_null() {
            unsafe { SetThreadDpiAwarenessContext(self.old) };
        }
    }
}

fn wide_to_string(w: &[u16]) -> String {
    let n = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..n])
}

fn rect_of(r: &RECT) -> Rect {
    Rect::new(r.left, r.top, (r.right - r.left).max(0) as u32, (r.bottom - r.top).max(0) as u32)
}

/// Every desktop monitor, numbered left to right, with the GPU objects to capture it.
pub fn outputs() -> Result<Vec<Output>> {
    let _dpi = DpiScope::per_monitor();
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.ctx("CreateDXGIFactory1")?;
    let whites = sdr_white_levels();
    let mut out: Vec<Output> = Vec::new();
    let mut ai = 0;
    loop {
        let adapter = match unsafe { factory.EnumAdapters1(ai) } {
            Ok(a) => a,
            Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(e) => return Err(crate::Error::os("EnumAdapters1", e.code().0)),
        };
        ai += 1;
        let mut oi = 0;
        loop {
            let output = match unsafe { adapter.EnumOutputs(oi) } {
                Ok(o) => o,
                Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(e) => return Err(crate::Error::os("EnumOutputs", e.code().0)),
            };
            oi += 1;
            let desc = unsafe { output.GetDesc() }.ctx("IDXGIOutput::GetDesc")?;
            if !desc.AttachedToDesktop.as_bool() {
                continue;
            }
            let device = wide_to_string(&desc.DeviceName);
            if out.iter().any(|o| o.monitor.device == device) {
                continue; // the same monitor listed under a second GPU (hybrid laptops)
            }
            let hdr = output
                .cast::<IDXGIOutput6>()
                .ok()
                .and_then(|o6| unsafe { o6.GetDesc1() }.ok())
                .map(|d| d.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020)
                .unwrap_or(false);
            let rotation = match desc.Rotation {
                r if r == DXGI_MODE_ROTATION_ROTATE90 => Rotation::Cw90,
                r if r == DXGI_MODE_ROTATION_ROTATE180 => Rotation::Cw180,
                r if r == DXGI_MODE_ROTATION_ROTATE270 => Rotation::Cw270,
                _ => Rotation::None,
            };
            let hmon = desc.Monitor;
            let monitor = Monitor {
                number: 0,
                rect: rect_of(&desc.DesktopCoordinates),
                primary: is_primary(hmon),
                dpi: dpi_of(hmon),
                rotation,
                hdr,
                sdr_white_nits: whites.get(&device).copied().unwrap_or(80.0),
                handle: hmon.0 as isize,
                device,
            };
            out.push(Output { adapter: adapter.clone(), output, monitor });
        }
    }
    let mut mons: Vec<Monitor> = out.iter().map(|o| o.monitor.clone()).collect();
    geom::number_monitors(&mut mons);
    for o in &mut out {
        if let Some(m) = mons.iter().find(|m| m.device == o.monitor.device) {
            o.monitor.number = m.number;
        }
    }
    out.sort_by_key(|o| o.monitor.number);
    Ok(out)
}

fn is_primary(h: HMONITOR) -> bool {
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe { GetMonitorInfoW(h, &mut mi) }.as_bool() && (mi.dwFlags & 1) != 0 // MONITORINFOF_PRIMARY
}

fn dpi_of(h: HMONITOR) -> u32 {
    let (mut x, mut y) = (0u32, 0u32);
    match unsafe { GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut x, &mut y) } {
        Ok(()) => x,
        Err(_) => 96,
    }
}

/// GDI device name → SDR white level in nits, for every active display path.
fn sdr_white_levels() -> HashMap<String, f32> {
    let mut map = HashMap::new();
    let (mut np, mut nm) = (0u32, 0u32);
    if unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) } != ERROR_SUCCESS {
        return map;
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
    if unsafe { QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None) } != ERROR_SUCCESS {
        return map;
    }
    for p in paths.iter().take(np as usize) {
        let mut src = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                size: std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                adapterId: p.sourceInfo.adapterId,
                id: p.sourceInfo.id,
            },
            ..Default::default()
        };
        if unsafe { DisplayConfigGetDeviceInfo(&mut src.header) } != 0 {
            continue;
        }
        let mut white = DISPLAYCONFIG_SDR_WHITE_LEVEL {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
                size: std::mem::size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32,
                adapterId: p.targetInfo.adapterId,
                id: p.targetInfo.id,
            },
            ..Default::default()
        };
        if unsafe { DisplayConfigGetDeviceInfo(&mut white.header) } != 0 {
            continue;
        }
        map.insert(wide_to_string(&src.viewGdiDeviceName), white.SDRWhiteLevel as f32 / 1000.0 * 80.0);
    }
    map
}

/// For the proof: a monitor's current display mode size (EnumDisplaySettings) and its GetMonitorInfo rectangle, both read with
/// per-monitor DPI awareness — they must equal the DXGI rectangle for the capture to be DPI-correct.
pub fn cross_check(m: &Monitor) -> (Option<(u32, u32)>, Option<Rect>) {
    let _dpi = DpiScope::per_monitor();
    let name: Vec<u16> = m.device.encode_utf16().chain(std::iter::once(0)).collect();
    let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
    let mode = unsafe { EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm) }
        .as_bool()
        .then_some((dm.dmPelsWidth, dm.dmPelsHeight));
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let info = unsafe { GetMonitorInfoW(HMONITOR(m.handle as *mut _), &mut mi.monitorInfo) }
        .as_bool()
        .then(|| rect_of(&mi.monitorInfo.rcMonitor));
    (mode, info)
}
