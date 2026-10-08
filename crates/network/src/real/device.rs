//! The adapters' Device Manager entries (SetupAPI, network class): which are switched off, and the wired switch
//! itself (DIF_PROPERTYCHANGE enable / disable = what Device Manager's "Disable device" does; ADMIN).
//! The adapter <-> device link is the driver key's `NetCfgInstanceId` (= the adapter GUID).

use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_Status, SetupDiCallClassInstaller, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo,
    SetupDiGetClassDevsW, SetupDiGetDeviceRegistryPropertyW, SetupDiOpenDevRegKey, SetupDiSetClassInstallParamsW,
    CM_DEVNODE_STATUS_FLAGS, CM_PROB, CM_PROB_DISABLED, CR_SUCCESS, DICS_DISABLE, DICS_ENABLE, DICS_FLAG_GLOBAL,
    DIF_PROPERTYCHANGE, DIGCF_PRESENT, DIREG_DRV, DN_HAS_PROBLEM, GUID_DEVCLASS_NET, HDEVINFO, SETUP_DI_REGISTRY_PROPERTY,
    SPDRP_DEVICEDESC, SPDRP_ENUMERATOR_NAME, SPDRP_FRIENDLYNAME, SP_CLASSINSTALL_HEADER, SP_DEVINFO_DATA,
    SP_PROPCHANGE_PARAMS,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_VALUE_TYPE,
};

use super::util::{from_wide, wide};
use crate::error::{NetError, Result};

#[derive(Debug, Clone)]
pub struct NetDevice {
    /// Adapter GUID (NetCfgInstanceId), upper case with braces.
    pub guid: String,
    pub description: String,
    /// "Ethernet 2" - the connection name from the network control key.
    pub connection_name: Option<String>,
    pub disabled: bool,
    /// Real hardware (PCI / USB / SD / ...), not ROOT / SWD software adapters.
    pub physical: bool,
    /// `*IfType` from the driver key (6 Ethernet, 71 Wi-Fi).
    pub if_type: Option<u32>,
}

struct DevSet(HDEVINFO);
impl Drop for DevSet {
    fn drop(&mut self) {
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn reg_value(key: HKEY, name: &str) -> Option<(REG_VALUE_TYPE, Vec<u8>)> {
    let n = wide(name);
    let mut ty = REG_VALUE_TYPE::default();
    let mut size = 0u32;
    unsafe {
        if RegQueryValueExW(key, PCWSTR(n.as_ptr()), None, Some(&mut ty), None, Some(&mut size)).is_err() {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if RegQueryValueExW(key, PCWSTR(n.as_ptr()), None, Some(&mut ty), Some(buf.as_mut_ptr()), Some(&mut size))
            .is_err()
        {
            return None;
        }
        buf.truncate(size as usize);
        Some((ty, buf))
    }
}

fn reg_string(key: HKEY, name: &str) -> Option<String> {
    let (_, b) = reg_value(key, name)?;
    let w: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    Some(from_wide(&w))
}

fn reg_dword(key: HKEY, name: &str) -> Option<u32> {
    let (_, b) = reg_value(key, name)?;
    (b.len() >= 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn prop_string(set: HDEVINFO, data: &SP_DEVINFO_DATA, prop: SETUP_DI_REGISTRY_PROPERTY) -> Option<String> {
    let mut buf = vec![0u8; 1024];
    let mut need = 0u32;
    unsafe { SetupDiGetDeviceRegistryPropertyW(set, data, prop, None, Some(&mut buf), Some(&mut need)).ok()? };
    let w: Vec<u16> = buf.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    Some(from_wide(&w))
}

fn connection_name(guid: &str) -> Option<String> {
    let path = wide(&format!(
        "SYSTEM\\CurrentControlSet\\Control\\Network\\{{4D36E972-E325-11CE-BFC1-08002BE10318}}\\{guid}\\Connection"
    ));
    let mut k = HKEY::default();
    unsafe {
        RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(path.as_ptr()), None, KEY_READ, &mut k).ok().ok()?;
    }
    let k = Key(k);
    reg_string(k.0, "Name")
}

/// Walks every present network-class device; `f` gets the set + entry + parsed info and may stop the walk.
fn walk(mut f: impl FnMut(HDEVINFO, &SP_DEVINFO_DATA, NetDevice) -> bool) -> Result<()> {
    let set = unsafe { SetupDiGetClassDevsW(Some(&GUID_DEVCLASS_NET), PCWSTR::null(), None, DIGCF_PRESENT) }
        .map_err(|e| NetError::Os { call: "SetupDiGetClassDevsW", code: e.code().0 as u32 })?;
    let set = DevSet(set);
    let mut i = 0u32;
    loop {
        let mut data = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
        if unsafe { SetupDiEnumDeviceInfo(set.0, i, &mut data) }.is_err() {
            break;
        }
        i += 1;
        let key = match unsafe { SetupDiOpenDevRegKey(set.0, &data, DICS_FLAG_GLOBAL.0, 0, DIREG_DRV, KEY_READ.0) } {
            Ok(k) => Key(k),
            Err(_) => continue,
        };
        let Some(guid) = reg_string(key.0, "NetCfgInstanceId") else { continue };
        let guid = guid.to_ascii_uppercase();
        let if_type = reg_dword(key.0, "*IfType");
        let mut status = CM_DEVNODE_STATUS_FLAGS(0);
        let mut problem = CM_PROB(0);
        let disabled = unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, data.DevInst, 0) } == CR_SUCCESS
            && (status.0 & DN_HAS_PROBLEM.0) != 0
            && problem == CM_PROB_DISABLED;
        let enumerator = prop_string(set.0, &data, SPDRP_ENUMERATOR_NAME).unwrap_or_default().to_ascii_uppercase();
        let physical = !enumerator.is_empty() && !matches!(enumerator.as_str(), "ROOT" | "SWD" | "SW");
        let description = prop_string(set.0, &data, SPDRP_FRIENDLYNAME)
            .or_else(|| prop_string(set.0, &data, SPDRP_DEVICEDESC))
            .unwrap_or_default();
        let dev = NetDevice {
            connection_name: connection_name(&guid),
            guid,
            description,
            disabled,
            physical,
            if_type,
        };
        if !f(set.0, &data, dev) {
            break;
        }
    }
    Ok(())
}

pub fn list_net_devices() -> Result<Vec<NetDevice>> {
    let mut v = Vec::new();
    walk(|_, _, d| {
        v.push(d);
        true
    })?;
    Ok(v)
}

/// Read-only: (device description, disabled?) of the adapter's device - None if not found.
pub fn describe(guid: &str) -> Result<Option<(String, bool)>> {
    let want = guid.to_ascii_uppercase();
    let mut found = None;
    walk(|_, _, d| {
        if d.guid == want {
            found = Some((d.description, d.disabled));
            false
        } else {
            true
        }
    })?;
    Ok(found)
}

/// ADMIN. Enables / disables the adapter's device (Device Manager's switch). Never called by tests.
pub fn set_enabled(guid: &str, on: bool) -> Result<()> {
    let want = guid.to_ascii_uppercase();
    let mut result: Option<Result<()>> = None;
    walk(|set, data, d| {
        if d.guid != want {
            return true;
        }
        let params = SP_PROPCHANGE_PARAMS {
            ClassInstallHeader: SP_CLASSINSTALL_HEADER {
                cbSize: std::mem::size_of::<SP_CLASSINSTALL_HEADER>() as u32,
                InstallFunction: DIF_PROPERTYCHANGE,
            },
            StateChange: if on { DICS_ENABLE } else { DICS_DISABLE },
            Scope: DICS_FLAG_GLOBAL,
            HwProfile: 0,
        };
        let r = unsafe {
            SetupDiSetClassInstallParamsW(
                set,
                Some(data),
                Some(&params.ClassInstallHeader),
                std::mem::size_of::<SP_PROPCHANGE_PARAMS>() as u32,
            )
            .and_then(|_| SetupDiCallClassInstaller(DIF_PROPERTYCHANGE, set, Some(data)))
        };
        result = Some(r.map_err(|e| match e.code().0 as u32 & 0xFFFF {
            5 => NetError::NeedsAdmin,
            c => NetError::Os { call: "SetupDiCallClassInstaller", code: c },
        }));
        false
    })?;
    result.unwrap_or_else(|| Err(NetError::NoSuchAdapter(guid.into())))
}
