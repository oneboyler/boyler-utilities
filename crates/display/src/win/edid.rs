//! Monitor size from its EDID (read-only registry read) for the "DELL 27″" selector label.
//! The monitor device path `\\?\DISPLAY#DEL41B6#5&2a6e…&UID4352#{guid}` names the registry key
//! `HKLM\SYSTEM\CurrentControlSet\Enum\DISPLAY\DEL41B6\5&2a6e…&UID4352\Device Parameters`, value `EDID`.

use windows::core::HSTRING;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY};

fn key_for(device_path: &str) -> Option<String> {
    let parts: Vec<&str> = device_path.split('#').collect();
    if parts.len() < 3 {
        return None;
    }
    Some(format!(r"SYSTEM\CurrentControlSet\Enum\DISPLAY\{}\{}\Device Parameters", parts[1], parts[2]))
}

pub(crate) fn read(device_path: &str) -> Option<Vec<u8>> {
    let key = HSTRING::from(key_for(device_path)?);
    let name = HSTRING::from("EDID");
    let mut len = 0u32;
    let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, &key, &name, RRF_RT_REG_BINARY, None, None, Some(&mut len)) };
    if r.is_err() || len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, &key, &name, RRF_RT_REG_BINARY, None, Some(buf.as_mut_ptr().cast()), Some(&mut len)) };
    if r.is_err() {
        return None;
    }
    buf.truncate(len as usize);
    Some(buf)
}

/// Diagonal in inches: from the first detailed timing descriptor (image size in mm), else the basic size in cm.
pub fn diagonal_inches(edid: &[u8]) -> Option<f32> {
    if edid.len() < 128 || edid[0..8] != [0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0] {
        return None;
    }
    let d = &edid[54..72];
    let pixel_clock = u16::from_le_bytes([d[0], d[1]]);
    let (w_mm, h_mm) = if pixel_clock != 0 {
        (((d[14] as u32 & 0xF0) << 4) | d[12] as u32, ((d[14] as u32 & 0x0F) << 8) | d[13] as u32)
    } else {
        (0, 0)
    };
    let (w, h) = if w_mm > 0 && h_mm > 0 { (w_mm as f32, h_mm as f32) } else { (edid[21] as f32 * 10.0, edid[22] as f32 * 10.0) };
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    Some(((w * w + h * h).sqrt() / 25.4 * 10.0).round() / 10.0)
}
