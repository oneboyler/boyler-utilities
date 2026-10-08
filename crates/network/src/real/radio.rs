//! The Wi-Fi radio switch through `Windows.Devices.Radios` (the same switch as Quick Settings' Wi-Fi button).
//! No admin. Reading is safe; `set_wifi` is never called by tests (it would cut a Wi-Fi connection).

use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState};

use crate::error::{NetError, Result};

fn os(call: &'static str) -> impl Fn(windows::core::Error) -> NetError {
    move |e| NetError::Os { call, code: e.code().0 as u32 }
}

fn wifi_radios() -> Result<Vec<Radio>> {
    let all = Radio::GetRadiosAsync().map_err(os("Radio.GetRadiosAsync"))?.join().map_err(os("Radio.GetRadiosAsync"))?;
    let mut out = Vec::new();
    for r in all {
        if r.Kind().map_err(os("Radio.Kind"))? == RadioKind::WiFi {
            out.push(r);
        }
    }
    Ok(out)
}

/// Some(on) for the Wi-Fi radio, None when the PC has none.
pub fn wifi_state() -> Result<Option<bool>> {
    let radios = wifi_radios()?;
    if radios.is_empty() {
        return Ok(None);
    }
    let mut any_on = false;
    for r in &radios {
        any_on |= r.State().map_err(os("Radio.State"))? == RadioState::On;
    }
    Ok(Some(any_on))
}

/// Turns every Wi-Fi radio on or off.
pub fn set_wifi(on: bool) -> Result<()> {
    let radios = wifi_radios()?;
    if radios.is_empty() {
        return Err(NetError::NoWifiRadio);
    }
    let access = Radio::RequestAccessAsync()
        .map_err(os("Radio.RequestAccessAsync"))?
        .join()
        .map_err(os("Radio.RequestAccessAsync"))?;
    if access != RadioAccessStatus::Allowed {
        return Err(NetError::AccessDenied(format!("radio access: {}", access.0)));
    }
    let want = if on { RadioState::On } else { RadioState::Off };
    for r in radios {
        let s = r.SetStateAsync(want).map_err(os("Radio.SetStateAsync"))?.join().map_err(os("Radio.SetStateAsync"))?;
        if s != RadioAccessStatus::Allowed {
            return Err(NetError::AccessDenied(format!("radio set: {}", s.0)));
        }
    }
    Ok(())
}
