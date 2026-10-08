//! Chromium's text contrast / gamma on Windows (ui/gfx/font_util_win.cc): contrast 1.0 and gamma 0 (= the sRGB curve) —
//! unless the ClearType Tuner's registry key for the primary display exists
//! (HKCU\SOFTWARE\Microsoft\Avalon.Graphics\<display>); then DirectWrite's rendering params decide:
//! enhanced contrast clamped to [0, 1], gamma clamped to [0, 4). Read once per process, like Chromium.

use std::sync::OnceLock;
use windows::core::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Gdi::{EnumDisplayDevicesW, DISPLAY_DEVICEW, DISPLAY_DEVICE_PRIMARY_DEVICE};
use windows::Win32::System::Registry::*;

static CG: OnceLock<(f32, f32, bool)> = OnceLock::new();

/// (contrast, gamma) for Skia's text tables.
pub fn contrast_gamma() -> (f32, f32) {
    let v = CG.get_or_init(read);
    (v.0, v.1)
}

/// True when the ClearType Tuner's values are in use (for the timing log).
pub fn from_registry() -> bool {
    CG.get_or_init(read).2
}

fn primary_display() -> Option<String> {
    unsafe {
        let mut i = 0;
        loop {
            let mut d = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
            if !EnumDisplayDevicesW(None, i, &mut d, 0).as_bool() {
                return None;
            }
            if d.StateFlags.0 & DISPLAY_DEVICE_PRIMARY_DEVICE.0 != 0 {
                let n = String::from_utf16_lossy(&d.DeviceName[..d.DeviceName.iter().position(|&c| c == 0).unwrap_or(32)]);
                // "\\.\DISPLAY1" -> "DISPLAY1"
                return Some(n.trim_start_matches(['\\', '.']).to_string());
            }
            i += 1;
        }
    }
}

fn read() -> (f32, f32, bool) {
    let def = (1.0, 0.0, false);
    let Some(name) = primary_display() else { return def };
    let path: Vec<u16> = format!("SOFTWARE\\Microsoft\\Avalon.Graphics\\{}", name).encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut k = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), Some(0), KEY_READ, &mut k).is_err() {
            return def;
        }
        let _ = RegCloseKey(k);
        let Ok(f) = DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) else { return def };
        let Ok(p) = f.CreateRenderingParams() else { return def };
        let c = p.GetEnhancedContrast().clamp(0.0, 1.0);
        let g = p.GetGamma().clamp(0.0, 3.999);
        (c, g, true)
    }
}
