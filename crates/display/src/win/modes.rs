//! The monitor's real mode list with EXACT rates. DXGI `IDXGIOutput::GetDisplayModeList` returns every mode the
//! driver exposes for the output with the refresh rate as a fraction (e.g. 143981/1000), which is what makes "143.98"
//! vs "144" possible. `EnumDisplaySettingsExW` (whole Hz only) is the fallback when DXGI has no output for the monitor.
//! Also: which GPU vendor drives the monitor (for vibrance).

use super::wide_to_string;
use crate::error::{DisplayError, Result};
use crate::types::{GpuVendor, RefreshRate, VideoMode};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_DESC, DXGI_MODE_SCANLINE_ORDER_LOWER_FIELD_FIRST, DXGI_MODE_SCANLINE_ORDER_UPPER_FIELD_FIRST};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, IDXGIOutput, DXGI_ENUM_MODES};
use windows::Win32::Graphics::Gdi::{EnumDisplaySettingsExW, DEVMODEW, ENUM_DISPLAY_SETTINGS_FLAGS, ENUM_DISPLAY_SETTINGS_MODE};

fn vendor_of(id: u32) -> GpuVendor {
    match id {
        0x10DE => GpuVendor::Nvidia,
        0x1002 | 0x1022 => GpuVendor::Amd,
        0x8086 => GpuVendor::Intel,
        _ => GpuVendor::Other,
    }
}

/// The DXGI output for a GDI name (`\\.\DISPLAY1`) and its adapter's vendor.
fn output_for(gdi_name: &str) -> Option<(IDXGIOutput, GpuVendor)> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.ok()?;
    let mut ai = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(ai) } {
        ai += 1;
        let vendor = unsafe { adapter.GetDesc1() }.map(|d| vendor_of(d.VendorId)).unwrap_or(GpuVendor::Other);
        let mut oi = 0;
        while let Ok(out) = unsafe { adapter.EnumOutputs(oi) } {
            oi += 1;
            if let Ok(d) = unsafe { out.GetDesc() } {
                if wide_to_string(&d.DeviceName).eq_ignore_ascii_case(gdi_name) {
                    return Some((out, vendor));
                }
            }
        }
    }
    None
}

pub(crate) fn vendor(gdi_name: &str) -> Option<GpuVendor> {
    output_for(gdi_name).map(|(_, v)| v)
}

fn dxgi_modes(gdi_name: &str) -> Option<Vec<VideoMode>> {
    let (out, _) = output_for(gdi_name)?;
    let flags = DXGI_ENUM_MODES(0); // progressive modes, no extra scaled duplicates
    let mut n = 0u32;
    unsafe { out.GetDisplayModeList(DXGI_FORMAT_B8G8R8A8_UNORM, flags, &mut n, None) }.ok()?;
    let mut descs = vec![DXGI_MODE_DESC::default(); n as usize];
    unsafe { out.GetDisplayModeList(DXGI_FORMAT_B8G8R8A8_UNORM, flags, &mut n, Some(descs.as_mut_ptr())) }.ok()?;
    descs.truncate(n as usize);
    let mut v: Vec<VideoMode> = descs
        .iter()
        .filter(|d| d.ScanlineOrdering != DXGI_MODE_SCANLINE_ORDER_UPPER_FIELD_FIRST && d.ScanlineOrdering != DXGI_MODE_SCANLINE_ORDER_LOWER_FIELD_FIRST)
        .filter(|d| d.RefreshRate.Denominator != 0 && d.RefreshRate.Numerator != 0)
        .map(|d| VideoMode { width: d.Width, height: d.Height, refresh: RefreshRate::new(d.RefreshRate.Numerator, d.RefreshRate.Denominator) })
        .collect();
    v.sort_by_key(|m| (m.width, m.height, m.refresh));
    v.dedup();
    (!v.is_empty()).then_some(v)
}

fn gdi_modes(gdi_name: &str) -> Vec<VideoMode> {
    let name = HSTRING::from(gdi_name);
    let mut v = Vec::new();
    let mut i = 0u32;
    loop {
        let mut dm = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
        let ok = unsafe { EnumDisplaySettingsExW(PCWSTR(name.as_ptr()), ENUM_DISPLAY_SETTINGS_MODE(i), &mut dm, ENUM_DISPLAY_SETTINGS_FLAGS(0)) };
        if !ok.as_bool() {
            break;
        }
        i += 1;
        if dm.dmBitsPerPel == 32 && dm.dmDisplayFrequency > 1 {
            v.push(VideoMode { width: dm.dmPelsWidth, height: dm.dmPelsHeight, refresh: RefreshRate::whole(dm.dmDisplayFrequency) });
        }
    }
    v.sort_by_key(|m| (m.width, m.height, m.refresh));
    v.dedup();
    v
}

/// Every mode the monitor reports. Exact (DXGI) when possible, else whole-Hz (GDI).
pub(crate) fn list(gdi_name: &str) -> Result<Vec<VideoMode>> {
    if let Some(v) = dxgi_modes(gdi_name) {
        return Ok(v);
    }
    let v = gdi_modes(gdi_name);
    if v.is_empty() {
        return Err(DisplayError::NoModes);
    }
    Ok(v)
}

/// For the proof output: the whole-Hz GDI list too (to show DXGI's exact rates next to it).
pub fn list_gdi(gdi_name: &str) -> Vec<VideoMode> {
    gdi_modes(gdi_name)
}

/// For the proof output: the DXGI list only.
pub fn list_dxgi(gdi_name: &str) -> Option<Vec<VideoMode>> {
    dxgi_modes(gdi_name)
}
