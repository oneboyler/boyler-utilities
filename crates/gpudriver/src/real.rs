//! The real Windows layers: the display adapters Windows lists as present (SetupAPI, read-only), Windows' Authenticode
//! check with the signer's name (WinVerifyTrust), and the elevated copy's work folder + process run.

use crate::detect::{Adapter, GpuOs};
use crate::download::Signatures;
use crate::error::{DriverError, Result};
use crate::install::InstallOs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceRegistryPropertyW, SetupDiOpenDevRegKey, DICS_FLAG_GLOBAL,
    DIGCF_PRESENT, DIREG_DRV, GUID_DEVCLASS_DISPLAY, HDEVINFO, SETUP_DI_REGISTRY_PROPERTY, SPDRP_DEVICEDESC, SPDRP_HARDWAREID, SP_DEVINFO_DATA,
};
use windows::Win32::Foundation::{HWND, LocalFree, HLOCAL};
use windows::Win32::System::Registry::{RegCloseKey, RegGetValueW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_SZ};

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain([0]).collect()
}

fn os_err(what: &str, e: windows::core::Error) -> DriverError {
    DriverError::Os(format!("{what}: {} (0x{:08X})", e.message().trim(), e.code().0 as u32))
}

/// Windows' display adapters (read-only).
pub struct RealGpuOs;

struct DevList(HDEVINFO);

impl Drop for DevList {
    fn drop(&mut self) {
        // SAFETY: the list came from SetupDiGetClassDevsW and is destroyed once, here.
        let _ = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

fn utf16_z(b: &[u8]) -> Vec<u16> {
    b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect()
}

/// A device's text property (the first string of a multi-string).
fn dev_text(list: &DevList, dev: &SP_DEVINFO_DATA, prop: SETUP_DI_REGISTRY_PROPERTY) -> String {
    let mut buf = vec![0u8; 2048];
    let mut need = 0u32;
    // SAFETY: buf outlives the call and its length is passed.
    if unsafe { SetupDiGetDeviceRegistryPropertyW(list.0, dev, prop, None, Some(&mut buf), Some(&mut need)) }.is_err() {
        return String::new();
    }
    let w = utf16_z(&buf[..(need as usize).min(buf.len())]);
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

fn reg_text(key: HKEY, sub: Option<&str>, name: &str) -> Option<String> {
    let sub_w = sub.map(|s| wide(std::ffi::OsStr::new(s)));
    let name_w = wide(std::ffi::OsStr::new(name));
    let mut buf = vec![0u16; 256];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: the strings are NUL-terminated and live across the call; buf's size in bytes is passed.
    let r = unsafe {
        RegGetValueW(
            key,
            sub_w.as_ref().map_or(PCWSTR::null(), |s| PCWSTR(s.as_ptr())),
            PCWSTR(name_w.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if r.is_err() {
        return None;
    }
    let n = (len as usize / 2).min(buf.len());
    let end = buf[..n].iter().position(|&c| c == 0).unwrap_or(n);
    Some(String::from_utf16_lossy(&buf[..end]))
}

impl GpuOs for RealGpuOs {
    fn adapters(&self) -> Result<Vec<Adapter>> {
        // SAFETY: plain SetupAPI enumeration; every buffer is owned here and the list is freed by DevList.
        let list = DevList(unsafe { SetupDiGetClassDevsW(Some(&GUID_DEVCLASS_DISPLAY), PCWSTR::null(), None, DIGCF_PRESENT) }.map_err(|e| os_err("listing display adapters", e))?);
        let mut out = Vec::new();
        for i in 0.. {
            let mut dev = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            // SAFETY: dev is a properly sized SP_DEVINFO_DATA.
            if unsafe { SetupDiEnumDeviceInfo(list.0, i, &mut dev) }.is_err() {
                break;
            }
            // SAFETY: opens the device's driver key read-only; closed right after.
            let driver_version = match unsafe { SetupDiOpenDevRegKey(list.0, &dev, DICS_FLAG_GLOBAL.0, 0, DIREG_DRV, KEY_READ.0) } {
                Ok(k) => {
                    let v = reg_text(k, None, "DriverVersion").unwrap_or_default();
                    // SAFETY: k came from SetupDiOpenDevRegKey.
                    let _ = unsafe { RegCloseKey(k) };
                    v
                }
                Err(_) => String::new(),
            };
            out.push(Adapter { name: dev_text(&list, &dev, SPDRP_DEVICEDESC), hardware_id: dev_text(&list, &dev, SPDRP_HARDWAREID), driver_version });
        }
        Ok(out)
    }

    fn os_build(&self) -> u32 {
        reg_text(HKEY_LOCAL_MACHINE, Some(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"), "CurrentBuildNumber").and_then(|s| s.trim().parse().ok()).unwrap_or(0)
    }

    fn is_laptop(&self) -> bool {
        use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        let mut s = SYSTEM_POWER_STATUS::default();
        // SAFETY: s is a valid out struct. BatteryFlag 128 = no system battery, 255 = unknown.
        unsafe { GetSystemPowerStatus(&mut s) }.is_ok() && s.BatteryFlag != 128 && s.BatteryFlag != 255
    }
}

/// Windows' Authenticode check (WinVerifyTrust, no UI, no network revocation check) + the signer's name.
pub struct RealSignatures;

impl Signatures for RealSignatures {
    fn signer(&self, file: &Path) -> Result<String> {
        use windows::Win32::Security::Cryptography::{CertGetNameStringW, CERT_NAME_SIMPLE_DISPLAY_TYPE};
        use windows::Win32::Security::WinTrust::{
            WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0,
            WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
        };
        let path = wide(file.as_os_str());
        let mut fi = WINTRUST_FILE_INFO { cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32, pcwszFilePath: PCWSTR(path.as_ptr()), ..Default::default() };
        let mut wd = WINTRUST_DATA {
            cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
            dwUIChoice: WTD_UI_NONE,
            fdwRevocationChecks: WTD_REVOKE_NONE,
            dwUnionChoice: WTD_CHOICE_FILE,
            Anonymous: WINTRUST_DATA_0 { pFile: &mut fi },
            dwStateAction: WTD_STATEACTION_VERIFY,
            ..Default::default()
        };
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        // SAFETY: wd / fi / path live across both calls; the CLOSE call frees the state VERIFY made, after the signer's
        // name was copied out of it.
        let r = unsafe { WinVerifyTrust(HWND::default(), &mut action, (&mut wd as *mut WINTRUST_DATA).cast()) };
        let name = if r == 0 {
            unsafe {
                let prov = WTHelperProvDataFromStateData(wd.hWVTStateData);
                let sgnr = if prov.is_null() { std::ptr::null_mut() } else { WTHelperGetProvSignerFromChain(prov, 0, false, 0) };
                if sgnr.is_null() || (*sgnr).csCertChain == 0 || (*sgnr).pasCertChain.is_null() || (*(*sgnr).pasCertChain).pCert.is_null() {
                    None
                } else if !root_in_machine_store((*sgnr).pChainContext) {
                    // end review: a root the user (not the machine) trusts could be anyone's
                    Some(String::from("a root certificate only this user trusts"))
                } else {
                    let cert = (*(*sgnr).pasCertChain).pCert;
                    let mut buf = [0u16; 512];
                    let n = CertGetNameStringW(cert, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, Some(&mut buf)) as usize;
                    Some(String::from_utf16_lossy(&buf[..n.saturating_sub(1).min(buf.len())]))
                }
            }
        } else {
            None
        };
        wd.dwStateAction = WTD_STATEACTION_CLOSE;
        let _ = unsafe { WinVerifyTrust(HWND::default(), &mut action, (&mut wd as *mut WINTRUST_DATA).cast()) };
        match (r, name) {
            (0, Some(n)) if !n.is_empty() => Ok(n),
            (0, _) => Err(DriverError::NotSigned("no signer name".into())),
            (code, _) => Err(DriverError::NotSigned(format!("Windows does not trust the signature (0x{:08X})", code as u32))),
        }
    }
}

/// The signature chain's root is one the MACHINE trusts (LocalMachine "Root" or "AuthRoot"), not only the user's own
/// store - the elevated copy runs nothing on a root a same-user program could have added.
///
/// # Safety
/// `chain` is the signer's chain from WinVerifyTrust's state (alive until its CLOSE call) or null.
unsafe fn root_in_machine_store(chain: *mut windows::Win32::Security::Cryptography::CERT_CHAIN_CONTEXT) -> bool {
    use windows::Win32::Security::Cryptography::{
        CertCloseStore, CertFindCertificateInStore, CertFreeCertificateContext, CertOpenStore, CERT_FIND_EXISTING, CERT_OPEN_STORE_FLAGS,
        CERT_STORE_OPEN_EXISTING_FLAG, CERT_STORE_PROV_SYSTEM_W, CERT_STORE_READONLY_FLAG, CERT_SYSTEM_STORE_LOCAL_MACHINE, PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
    };
    unsafe {
        if chain.is_null() || (*chain).cChain == 0 || (*chain).rgpChain.is_null() {
            return false;
        }
        let simple = *(*chain).rgpChain;
        if simple.is_null() || (*simple).cElement == 0 || (*simple).rgpElement.is_null() {
            return false;
        }
        let last = *(*simple).rgpElement.add((*simple).cElement as usize - 1);
        if last.is_null() || (*last).pCertContext.is_null() {
            return false;
        }
        let root = (*last).pCertContext;
        let flags = CERT_OPEN_STORE_FLAGS(CERT_SYSTEM_STORE_LOCAL_MACHINE) | CERT_STORE_READONLY_FLAG | CERT_STORE_OPEN_EXISTING_FLAG;
        for name in ["Root", "AuthRoot"] {
            let w = wide(std::ffi::OsStr::new(name));
            let Ok(store) = CertOpenStore(CERT_STORE_PROV_SYSTEM_W, X509_ASN_ENCODING | PKCS_7_ASN_ENCODING, None, flags, Some(w.as_ptr().cast())) else { continue };
            let found = CertFindCertificateInStore(store, X509_ASN_ENCODING | PKCS_7_ASN_ENCODING, 0, CERT_FIND_EXISTING, Some(root.cast()), None);
            if !found.is_null() {
                let _ = CertFreeCertificateContext(Some(found));
            }
            let _ = CertCloseStore(Some(store), 0);
            if !found.is_null() {
                return true;
            }
        }
        false
    }
}

/// The elevated copy's installer side.
pub struct RealInstallOs;

/// SYSTEM + Administrators full control, nothing inherited, nobody else.
const ADMIN_ONLY: &str = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";

impl InstallOs for RealInstallOs {
    fn make_work_dir(&self) -> Result<PathBuf> {
        use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
        use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
        use windows::Win32::Storage::FileSystem::CreateDirectoryW;
        use windows::Win32::System::SystemInformation::GetSystemWindowsDirectoryW;
        let mut buf = [0u16; 260];
        // SAFETY: buf's length is passed.
        let n = unsafe { GetSystemWindowsDirectoryW(Some(&mut buf)) } as usize;
        if n == 0 || n >= buf.len() {
            return Err(DriverError::Os("Windows' folder not found".into()));
        }
        let temp = PathBuf::from(String::from_utf16_lossy(&buf[..n])).join("Temp");
        let sddl = wide(std::ffi::OsStr::new(ADMIN_ONLY));
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: sddl is NUL-terminated; sd is freed with LocalFree below.
        unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()), SDDL_REVISION_1, &mut sd, None) }.map_err(|e| os_err("making the folder's permissions", e))?;
        let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd.0, bInheritHandle: false.into() };
        let mut made = Err(DriverError::Os("no free folder name".into()));
        for k in 0..16u32 {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            let dir = temp.join(format!("BoylerUtilities-gpudriver-{:x}{:08x}", std::process::id(), (t as u32) ^ k.wrapping_mul(0x9E37_79B9)));
            let w = wide(dir.as_os_str());
            // SAFETY: w is NUL-terminated; sa / sd live across the call. CreateDirectory fails on an existing name, so
            // the folder is always a new one with our permissions.
            if unsafe { CreateDirectoryW(PCWSTR(w.as_ptr()), Some(&sa)) }.is_ok() {
                made = Ok(dir);
                break;
            }
        }
        // SAFETY: sd came from ConvertStringSecurityDescriptorToSecurityDescriptorW.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(sd.0)));
        }
        made
    }

    fn run(&self, program: &Path, args: &[&str], cwd: &Path) -> Result<u32> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let st = std::process::Command::new(program)
            .args(args)
            .current_dir(cwd)
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| DriverError::Os(format!("starting NVIDIA's installer: {e}")))?;
        Ok(st.code().map(|c| c as u32).unwrap_or(u32::MAX))
    }
}
