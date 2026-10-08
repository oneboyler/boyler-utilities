//! The real monitors for Notifications for OBS (monitors.c `mons_refresh`): rectangle, work area, the mode's pixel size and
//! the device interface path OBS's display capture stores; numbered by bu_obs::monitors::number.

use bu_obs::monitors::{number, Mon, Rect};
use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayDevicesW, EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, DEVMODEW, DISPLAY_DEVICEW, ENUM_CURRENT_SETTINGS, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::core::PCWSTR;

const EDD_GET_DEVICE_INTERFACE_NAME: u32 = 1;

fn from_wide(b: &[u16]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf16_lossy(&b[..n])
}

fn rect(r: RECT) -> Rect {
    Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
}

unsafe extern "system" fn cb(h: HMONITOR, _dc: HDC, _r: *mut RECT, lp: LPARAM) -> BOOL {
    let v = &mut *(lp.0 as *mut Vec<Mon>);
    if v.len() >= 8 {
        return false.into();
    }
    let mut mi = MONITORINFOEXW { monitorInfo: MONITORINFO { cbSize: std::mem::size_of::<MONITORINFOEXW>() as u32, ..Default::default() }, ..Default::default() };
    if !GetMonitorInfoW(h, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
        return true.into();
    }
    let dev = from_wide(&mi.szDevice);
    let mut m = Mon {
        dev: dev.clone(),
        rc: rect(mi.monitorInfo.rcMonitor),
        work: rect(mi.monitorInfo.rcWork),
        primary: mi.monitorInfo.dwFlags & 1 != 0,
        ..Default::default()
    };
    let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
    if EnumDisplaySettingsW(PCWSTR(mi.szDevice.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm).as_bool() {
        m.w = dm.dmPelsWidth as i32;
        m.h = dm.dmPelsHeight as i32;
    } else {
        m.w = m.rc.w();
        m.h = m.rc.h();
    }
    let mut dd = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
    if EnumDisplayDevicesW(PCWSTR(mi.szDevice.as_ptr()), 0, &mut dd, EDD_GET_DEVICE_INTERFACE_NAME).as_bool() {
        m.iface = from_wide(&dd.DeviceID);
    }
    v.push(m);
    true.into()
}

/// Every monitor now, numbered (Monitor 1 = the main display).
pub fn list() -> Vec<Mon> {
    let mut v: Vec<Mon> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut v as *mut Vec<Mon> as isize));
    }
    number(&mut v);
    v
}
