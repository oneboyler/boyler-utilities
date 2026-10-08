//! Wi-Fi network name + signal through the WLAN API (WlanQueryInterface, current connection). Read-only.
//! On PCs without Wi-Fi (or with the WLAN service off) this simply returns nothing.
//! Note: on newer Windows 11 builds reading the SSID can need the location permission; then `ssid` stays None.

use std::collections::HashMap;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::WiFi::{
    wlan_interface_state_connected, wlan_intf_opcode_current_connection, WlanCloseHandle, WlanEnumInterfaces,
    WlanFreeMemory, WlanOpenHandle, WlanQueryInterface, WLAN_CONNECTION_ATTRIBUTES, WLAN_INTERFACE_INFO_LIST,
};

use super::util::guid_string;

#[derive(Debug, Clone, Default)]
pub struct WlanInfo {
    pub ssid: Option<String>,
    pub signal_pct: Option<u8>,
}

/// Connected Wi-Fi interfaces, keyed by adapter GUID.
pub fn connections() -> HashMap<String, WlanInfo> {
    let mut out = HashMap::new();
    unsafe {
        let mut ver = 0u32;
        let mut h = HANDLE::default();
        if WlanOpenHandle(2, None, &mut ver, &mut h) != 0 {
            return out;
        }
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        if WlanEnumInterfaces(h, None, &mut list) == 0 && !list.is_null() {
            let n = (*list).dwNumberOfItems as usize;
            let items = std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), n);
            for it in items {
                let mut info = WlanInfo::default();
                if it.isState == wlan_interface_state_connected {
                    let mut size = 0u32;
                    let mut data: *mut core::ffi::c_void = std::ptr::null_mut();
                    if WlanQueryInterface(
                        h,
                        &it.InterfaceGuid,
                        wlan_intf_opcode_current_connection,
                        None,
                        &mut size,
                        &mut data,
                        None,
                    ) == 0
                        && !data.is_null()
                    {
                        let c = &*(data as *const WLAN_CONNECTION_ATTRIBUTES);
                        let s = &c.wlanAssociationAttributes.dot11Ssid;
                        let len = (s.uSSIDLength as usize).min(32);
                        if len > 0 {
                            info.ssid = Some(String::from_utf8_lossy(&s.ucSSID[..len]).into_owned());
                        }
                        info.signal_pct = Some(c.wlanAssociationAttributes.wlanSignalQuality.min(100) as u8);
                        WlanFreeMemory(data);
                    }
                }
                out.insert(guid_string(&it.InterfaceGuid), info);
            }
            WlanFreeMemory(list as *const _);
        }
        WlanCloseHandle(h, None);
    }
    out
}

// ---- v21 Wi-Fi networks: the list Windows keeps, connect / disconnect / forget. Never called by tests (FakeNet).

use windows::core::{GUID, PCWSTR};
use windows::Win32::NetworkManagement::WiFi::{
    dot11_BSS_type_infrastructure, wlan_connection_mode_profile, WlanConnect, WlanDeleteProfile, WlanDisconnect,
    WlanGetAvailableNetworkList, WlanSetProfile, WLAN_AVAILABLE_NETWORK, WLAN_AVAILABLE_NETWORK_CONNECTED,
    WLAN_AVAILABLE_NETWORK_HAS_PROFILE, WLAN_AVAILABLE_NETWORK_LIST, WLAN_CONNECTION_PARAMETERS,
};

use super::util::{from_wide, os_err, wide};
use crate::error::{NetError, Result};
use crate::model::WifiNetwork;

/// An open WLAN client handle, closed on drop.
struct Client(HANDLE);

impl Drop for Client {
    fn drop(&mut self) {
        unsafe { WlanCloseHandle(self.0, None) };
    }
}

fn open() -> Result<Client> {
    let mut ver = 0u32;
    let mut h = HANDLE::default();
    match unsafe { WlanOpenHandle(2, None, &mut ver, &mut h) } {
        0 => Ok(Client(h)),
        // ERROR_SERVICE_NOT_ACTIVE (1062): the WLAN service is off = no Wi-Fi on this PC
        1062 => Err(NetError::NoWifiRadio),
        c => Err(os_err("WlanOpenHandle", c)),
    }
}

fn interfaces(c: &Client) -> Result<Vec<GUID>> {
    let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
    let r = unsafe { WlanEnumInterfaces(c.0, None, &mut list) };
    if r != 0 || list.is_null() {
        return Err(os_err("WlanEnumInterfaces", r));
    }
    let out = unsafe {
        let n = (*list).dwNumberOfItems as usize;
        let v = std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), n).iter().map(|i| i.InterfaceGuid).collect();
        WlanFreeMemory(list as *const _);
        v
    };
    Ok(out)
}

/// One raw entry of Windows' list.
struct Avail {
    iface: GUID,
    ssid: Vec<u8>,
    profile: String,
    signal: u32,
    secured: bool,
    auth: i32,
    cipher: i32,
    flags: u32,
}

fn available(c: &Client) -> Result<Vec<Avail>> {
    let mut out = Vec::new();
    for g in interfaces(c)? {
        let mut list: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
        if unsafe { WlanGetAvailableNetworkList(c.0, &g, 0, None, &mut list) } != 0 || list.is_null() {
            continue;
        }
        unsafe {
            let n = (*list).dwNumberOfItems as usize;
            let items: &[WLAN_AVAILABLE_NETWORK] = std::slice::from_raw_parts((*list).Network.as_ptr(), n);
            for a in items {
                if a.dot11BssType != dot11_BSS_type_infrastructure {
                    continue;
                }
                let len = (a.dot11Ssid.uSSIDLength as usize).min(32);
                if len == 0 {
                    continue; // a hidden network: no name to show
                }
                out.push(Avail {
                    iface: g,
                    ssid: a.dot11Ssid.ucSSID[..len].to_vec(),
                    profile: from_wide(&a.strProfileName),
                    signal: a.wlanSignalQuality.min(100),
                    secured: a.bSecurityEnabled.as_bool(),
                    auth: a.dot11DefaultAuthAlgorithm.0,
                    cipher: a.dot11DefaultCipherAlgorithm.0,
                    flags: a.dwFlags,
                });
            }
            WlanFreeMemory(list as *const _);
        }
    }
    Ok(out)
}

/// Windows lists a network once per profile (and once without); one row per name: connected / saved / strongest wins.
pub fn networks() -> Result<Vec<WifiNetwork>> {
    let c = open()?;
    let mut rows: Vec<WifiNetwork> = Vec::new();
    for a in available(&c)? {
        let n = WifiNetwork {
            ssid: String::from_utf8_lossy(&a.ssid).into_owned(),
            signal_pct: a.signal as u8,
            secured: a.secured,
            saved: a.flags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE != 0,
            connected: a.flags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0,
        };
        match rows.iter_mut().find(|r| r.ssid == n.ssid) {
            Some(r) => {
                r.saved |= n.saved;
                r.connected |= n.connected;
                r.signal_pct = r.signal_pct.max(n.signal_pct);
            }
            None => rows.push(n),
        }
    }
    Ok(rows)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// The profile XML Windows wants for a new network (WLAN_profile schema). DOT11_AUTH_ALGORITHM: 1 open, 4 WPA-PSK,
/// 7 RSNA-PSK (WPA2), 9 / 10 WPA3-SAE; cipher 2 TKIP, 4 CCMP (AES). Pure: tested.
pub fn profile_xml(ssid: &[u8], auth: i32, cipher: i32, password: Option<&str>, auto: bool) -> Result<String> {
    let name = xml_escape(&String::from_utf8_lossy(ssid));
    let hex: String = ssid.iter().map(|b| format!("{b:02X}")).collect();
    let (auth_s, enc_s) = match auth {
        1 => ("open", "none"),
        4 => ("WPAPSK", if cipher == 2 { "TKIP" } else { "AES" }),
        7 => ("WPA2PSK", if cipher == 2 { "TKIP" } else { "AES" }),
        9 | 10 => ("WPA3SAE", "AES"),
        a => return Err(NetError::Unsupported(format!("Wi-Fi security type {a} (enterprise networks are set up in Windows)"))),
    };
    let key = match (auth, password) {
        (1, _) => String::new(),
        (_, Some(p)) => format!(
            "<sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>{}</keyMaterial></sharedKey>",
            xml_escape(p)
        ),
        (_, None) => return Err(NetError::PasswordNeeded),
    };
    let mode = if auto { "auto" } else { "manual" };
    Ok(format!(
        "<?xml version=\"1.0\"?><WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\"><name>{name}</name>\
         <SSIDConfig><SSID><hex>{hex}</hex><name>{name}</name></SSID></SSIDConfig><connectionType>ESS</connectionType>\
         <connectionMode>{mode}</connectionMode><MSM><security><authEncryption><authentication>{auth_s}</authentication>\
         <encryption>{enc_s}</encryption><useOneX>false</useOneX></authEncryption>{key}</security></MSM></WLANProfile>"
    ))
}

/// Connects (a saved network: its profile; a new one: a profile is written first). No admin.
pub fn connect(ssid: &str, password: Option<&str>, auto: bool) -> Result<()> {
    let c = open()?;
    let all = available(&c)?;
    let want = ssid.as_bytes();
    let pick = all
        .iter()
        .filter(|a| a.ssid == want)
        .max_by_key(|a| (!a.profile.is_empty(), a.signal))
        .ok_or_else(|| NetError::Unreachable(ssid.into()))?;
    let profile = if !pick.profile.is_empty() {
        pick.profile.clone()
    } else {
        let xml = profile_xml(&pick.ssid, pick.auth, pick.cipher, password, auto)?;
        let x = wide(&xml);
        let mut reason = 0u32;
        let r = unsafe { WlanSetProfile(c.0, &pick.iface, 0, PCWSTR(x.as_ptr()), PCWSTR::null(), true, None, &mut reason) };
        if r != 0 {
            return Err(os_err("WlanSetProfile", if reason != 0 { reason } else { r }));
        }
        String::from_utf8_lossy(&pick.ssid).into_owned()
    };
    let p = wide(&profile);
    let params = WLAN_CONNECTION_PARAMETERS {
        wlanConnectionMode: wlan_connection_mode_profile,
        strProfile: PCWSTR(p.as_ptr()),
        pDot11Ssid: std::ptr::null_mut(),
        pDesiredBssidList: std::ptr::null_mut(),
        dot11BssType: dot11_BSS_type_infrastructure,
        dwFlags: 0,
    };
    match unsafe { WlanConnect(c.0, &pick.iface, &params, None) } {
        0 => Ok(()),
        r => Err(os_err("WlanConnect", r)),
    }
}

pub fn disconnect() -> Result<()> {
    let c = open()?;
    for g in interfaces(&c)? {
        let r = unsafe { WlanDisconnect(c.0, &g, None) };
        if r != 0 {
            return Err(os_err("WlanDisconnect", r));
        }
    }
    Ok(())
}

/// Deletes every saved profile of `ssid` (on every Wi-Fi adapter).
pub fn forget(ssid: &str) -> Result<()> {
    let c = open()?;
    let mut done = false;
    for a in available(&c)?.iter().filter(|a| a.ssid == ssid.as_bytes() && !a.profile.is_empty()) {
        let p = wide(&a.profile);
        match unsafe { WlanDeleteProfile(c.0, &a.iface, PCWSTR(p.as_ptr()), None) } {
            0 | 1168 => done = true, // 1168 = ERROR_NOT_FOUND: already gone (it was listed on two adapters)
            r => return Err(os_err("WlanDeleteProfile", r)),
        }
    }
    if done {
        Ok(())
    } else {
        Err(NetError::Unreachable(ssid.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_xml_escapes_and_picks_the_security() {
        let x = profile_xml("Café & <Bar>".as_bytes(), 7, 4, Some("p&ss\"word"), true).unwrap();
        assert!(x.contains("<name>Café &amp; &lt;Bar&gt;</name>"));
        assert!(x.contains("<hex>436166C3A92026203C4261723E</hex>"));
        assert!(x.contains("<authentication>WPA2PSK</authentication><encryption>AES</encryption>"));
        assert!(x.contains("<keyMaterial>p&amp;ss&quot;word</keyMaterial>"));
        assert!(x.contains("<connectionMode>auto</connectionMode>"));
        let o = profile_xml(b"Guest", 1, 0, None, false).unwrap();
        assert!(o.contains("<authentication>open</authentication><encryption>none</encryption>") && !o.contains("sharedKey"));
        assert_eq!(profile_xml(b"x", 7, 4, None, true), Err(NetError::PasswordNeeded));
        assert!(matches!(profile_xml(b"x", 8, 4, Some("p"), true), Err(NetError::Unsupported(_))));
        assert!(profile_xml(b"x", 9, 4, Some("p"), true).unwrap().contains("WPA3SAE"));
    }
}
