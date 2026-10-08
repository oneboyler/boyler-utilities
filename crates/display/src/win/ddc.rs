//! Brightness / contrast over DDC/CI through the public Monitor Configuration API (dxva2), the way Twinkle Tray,
//! Monitorian and PowerToys do it. Only the low-level VCP get/set of 0x10 (brightness) and 0x12 (contrast) — we
//! NEVER read the capabilities string (`GetCapabilitiesStringLength` / `CapabilitiesRequestAndCapabilitiesReply`), the
//! call that blue-screens some monitors (PowerToys Power Display docs, research big-B §1). Every call runs inside the
//! crash guard (picture.rs).

use super::hmonitor_for;
use crate::error::{DisplayError, Result};
use crate::types::{Vcp, VcpValue};
use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitors, GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR, GetVCPFeatureAndVCPFeatureReply,
    SetVCPFeature, PHYSICAL_MONITOR,
};

/// Runs `f` with the first physical monitor behind a GDI display (clone mode puts several behind one; the first is used).
fn with_physical<T>(gdi_name: &str, f: impl FnOnce(&PHYSICAL_MONITOR) -> Result<T>) -> Result<T> {
    let hmon = hmonitor_for(gdi_name).ok_or(DisplayError::DdcNoAnswer)?;
    let mut n = 0u32;
    unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(hmon, &mut n) }.map_err(|_| DisplayError::DdcNoAnswer)?;
    if n == 0 {
        return Err(DisplayError::DdcNoAnswer);
    }
    let mut pm = vec![PHYSICAL_MONITOR::default(); n as usize];
    unsafe { GetPhysicalMonitorsFromHMONITOR(hmon, &mut pm) }.map_err(|_| DisplayError::DdcNoAnswer)?;
    let r = f(&pm[0]);
    let _ = unsafe { DestroyPhysicalMonitors(&pm) };
    r
}

pub(crate) fn get(gdi_name: &str, vcp: Vcp) -> Result<VcpValue> {
    with_physical(gdi_name, |pm| {
        let (mut cur, mut max) = (0u32, 0u32);
        let ok = unsafe { GetVCPFeatureAndVCPFeatureReply(pm.hPhysicalMonitor, vcp.code(), None, &mut cur, Some(&mut max)) };
        if ok == 0 || max == 0 {
            return Err(DisplayError::DdcNoAnswer);
        }
        Ok(VcpValue { current: cur, max })
    })
}

pub(crate) fn set(gdi_name: &str, vcp: Vcp, value: u32) -> Result<()> {
    with_physical(gdi_name, |pm| {
        let ok = unsafe { SetVCPFeature(pm.hPhysicalMonitor, vcp.code(), value) };
        if ok == 0 { Err(DisplayError::DdcNoAnswer) } else { Ok(()) }
    })
}
