//! The REAL Windows implementation of `TogglesOs` (the `windows` crate).
//!
//! Three modes:
//! - [`RealOs::new`] — everything real (the menu).
//! - [`RealOs::read_only`] — reads are real, EVERY change is refused (`examples/show`).
//! - [`RealOs::scratch`] — registry reads/writes go under a scratch key in HKCU (`<base>\HKCU\…`, `<base>\HKLM\…`); every
//!   non-registry change is refused and every after-step (broadcast, Explorer restart, layout-hotkey reload, opening pages) is
//!   only recorded, never done. Used by the scratch-registry tests.

use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Registry::*;

use crate::error::{Error, Result};
use crate::os::*;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Mode {
    Full,
    ReadOnly,
    Scratch(String),
}

/// The real OS layer.
pub struct RealOs {
    mode: Mode,
    elevated_override: Option<bool>,
    /// scratch mode: the after-steps that were NOT done, in order
    recorded: Vec<String>,
    /// HKCU means `HKEY_USERS\<this SID>` (the app's elevated copy writes for the user who clicked - Order 039)
    user_sid: Option<String>,
}

const VIDEO_SUBGROUP: windows::core::GUID = windows::core::GUID::from_u128(0x7516b95f_f776_4464_8c53_06167f40cc99);
const VIDEOIDLE: windows::core::GUID = windows::core::GUID::from_u128(0x3c0bc021_c8a8_4e07_a973_6b14cbcb2b7e);
const SLEEP_SUBGROUP: windows::core::GUID = windows::core::GUID::from_u128(0x238c9fa8_0aad_41ed_83f4_97be242c8f20);
const STANDBYIDLE: windows::core::GUID = windows::core::GUID::from_u128(0x29f6c1db_86da_48c5_9fdb_f2b67b1f44da);
const USB_SUBGROUP: windows::core::GUID = windows::core::GUID::from_u128(0x2a737441_1930_4402_8d77_b2bebba308a3);
const USB_SELECTIVE_SUSPEND: windows::core::GUID = windows::core::GUID::from_u128(0x48e6b7a6_50f5_4782_a5d4_53bb8f07e226);

/// The Copilot app's package family (Microsoft Store "Microsoft Copilot").
pub const COPILOT_FAMILY: &str = "Microsoft.Copilot_8wekyb3d8bbwe";

fn power_guids(s: PowerSetting) -> (windows::core::GUID, windows::core::GUID) {
    match s {
        PowerSetting::ScreenOff => (VIDEO_SUBGROUP, VIDEOIDLE),
        PowerSetting::Sleep => (SLEEP_SUBGROUP, STANDBYIDLE),
        PowerSetting::UsbSelectiveSuspend => (USB_SUBGROUP, USB_SELECTIVE_SUSPEND),
    }
}

fn win32(op: &str, e: WIN32_ERROR) -> Result<()> {
    match e {
        ERROR_SUCCESS => Ok(()),
        ERROR_ACCESS_DENIED => Err(Error::NeedsAdmin { row: op.into() }),
        other => Err(Error::os(op, other.0 as i64)),
    }
}

fn hr(op: &str, e: windows::core::Error) -> Error {
    if e.code() == ERROR_ACCESS_DENIED.to_hresult() {
        Error::NeedsAdmin { row: op.into() }
    } else {
        Error::os(op, e.code().0 as i64)
    }
}

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|c| *c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

impl RealOs {
    /// Everything real — the menu uses this.
    pub fn new() -> Self {
        Self { mode: Mode::Full, elevated_override: None, recorded: Vec::new(), user_sid: None }
    }

    /// Real reads; every change refused with `Error::ReadOnly`.
    pub fn read_only() -> Self {
        Self { mode: Mode::ReadOnly, ..Self::new() }
    }

    /// Registry under `HKCU\<base>` (e.g. `Software\BoylerUtilities-test\toggles\run1`); nothing else real is changed.
    /// Counts as elevated (the scratch key is HKCU) unless [`RealOs::with_elevated`] says otherwise.
    /// Refused (`Error::ReadOnly`) unless `base` is under `Software\BoylerUtilities-test\`.
    pub fn scratch(base: &str) -> Result<Self> {
        let base = base.trim_matches('\\');
        if !is_scratch_path(base) {
            return Err(Error::ReadOnly(format!("not a scratch key: {base}")));
        }
        Ok(Self { mode: Mode::Scratch(base.to_string()), elevated_override: Some(true), ..Self::new() })
    }

    /// Everything real, with HKCU = `HKEY_USERS\<sid>`: the app's elevated copy (Order 039) writes the clicking user's
    /// own values even when another account typed the admin password. None unless `sid` looks like `S-1-…`.
    pub fn for_user(sid: &str) -> Option<Self> {
        let ok = sid.len() < 200 && sid.starts_with("S-1-") && sid[4..].split('-').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        ok.then(|| Self { user_sid: Some(sid.to_string()), ..Self::new() })
    }

    /// Overrides what `is_elevated` answers (tests of the admin path in scratch mode).
    pub fn with_elevated(mut self, elevated: bool) -> Self {
        self.elevated_override = Some(elevated);
        self
    }

    /// Scratch mode: the after-steps that were recorded instead of done.
    pub fn recorded(&self) -> &[String] {
        &self.recorded
    }

    fn map(&self, hive: Hive, path: &str) -> (HKEY, String) {
        match &self.mode {
            Mode::Scratch(base) => {
                let h = match hive {
                    Hive::Hkcu => "HKCU",
                    Hive::Hklm => "HKLM",
                };
                (HKEY_CURRENT_USER, format!("{base}\\{h}\\{}", path.trim_matches('\\')))
            }
            _ => match (hive, &self.user_sid) {
                (Hive::Hkcu, Some(sid)) => (HKEY_USERS, format!("{sid}\\{}", path.trim_matches('\\'))),
                (Hive::Hkcu, None) => (HKEY_CURRENT_USER, path.trim_matches('\\').to_string()),
                (Hive::Hklm, _) => (HKEY_LOCAL_MACHINE, path.trim_matches('\\').to_string()),
            },
        }
    }

    fn deny_write(&self, op: &str) -> Result<()> {
        if self.mode == Mode::ReadOnly {
            return Err(Error::ReadOnly(op.into()));
        }
        Ok(())
    }

    /// Changes that can't go to a scratch key (SPI, power, Bluetooth, Copilot).
    fn deny_system_change(&self, op: &str) -> Result<()> {
        match self.mode {
            Mode::Full => Ok(()),
            _ => Err(Error::ReadOnly(op.into())),
        }
    }

    /// After-steps in scratch mode are recorded, never done. Returns true when the caller must do the real thing.
    fn after_step(&mut self, what: String) -> Result<bool> {
        match self.mode {
            Mode::Full => Ok(true),
            Mode::ReadOnly => Err(Error::ReadOnly(what)),
            Mode::Scratch(_) => {
                self.recorded.push(what);
                Ok(false)
            }
        }
    }

    fn open(&self, hive: Hive, path: &str, sam: REG_SAM_FLAGS) -> Result<Option<HKEY>> {
        let (root, p) = self.map(hive, path);
        let mut hk = HKEY::default();
        let e = unsafe { RegOpenKeyExW(root, &HSTRING::from(p.as_str()), None, sam | KEY_WOW64_64KEY, &mut hk) };
        match e {
            ERROR_SUCCESS => Ok(Some(hk)),
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(None),
            other => win32(&format!("RegOpenKeyEx {p}"), other).map(|_| None),
        }
    }

    fn parse_value(kind: REG_VALUE_TYPE, bytes: Vec<u8>) -> RegValue {
        match kind {
            REG_DWORD if bytes.len() >= 4 => RegValue::Dword(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
            REG_SZ => {
                let w: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
                RegValue::Sz(wide_to_string(&w))
            }
            other => RegValue::Other { kind: other.0, bytes },
        }
    }
}

impl Default for RealOs {
    fn default() -> Self {
        Self::new()
    }
}

impl TogglesOs for RealOs {
    fn reg_read(&self, hive: Hive, path: &str, name: &str) -> Result<Option<RegValue>> {
        let Some(hk) = self.open(hive, path, KEY_READ)? else { return Ok(None) };
        let hname = HSTRING::from(name);
        let result = (|| unsafe {
            let mut kind = REG_VALUE_TYPE::default();
            let mut size = 0u32;
            match RegQueryValueExW(hk, &hname, None, Some(&mut kind), None, Some(&mut size)) {
                ERROR_SUCCESS => {}
                ERROR_FILE_NOT_FOUND => return Ok(None),
                e => return win32(&format!("RegQueryValueEx {name}"), e).map(|_| None),
            }
            let mut buf = vec![0u8; size as usize];
            let e = RegQueryValueExW(hk, &hname, None, Some(&mut kind), Some(buf.as_mut_ptr()), Some(&mut size));
            win32(&format!("RegQueryValueEx {name}"), e)?;
            buf.truncate(size as usize);
            Ok(Some(Self::parse_value(kind, buf)))
        })();
        unsafe {
            let _ = RegCloseKey(hk);
        }
        result
    }

    fn reg_write(&mut self, hive: Hive, path: &str, name: &str, value: &RegValue) -> Result<()> {
        self.deny_write("reg_write")?;
        let (root, p) = self.map(hive, path);
        let (kind, bytes) = match value {
            RegValue::Dword(d) => (REG_DWORD, d.to_le_bytes().to_vec()),
            RegValue::Sz(s) => (REG_SZ, s.encode_utf16().chain(std::iter::once(0)).flat_map(|c| c.to_le_bytes()).collect()),
            RegValue::Other { kind, bytes } => (REG_VALUE_TYPE(*kind), bytes.clone()),
        };
        unsafe {
            let mut hk = HKEY::default();
            let e = RegCreateKeyExW(
                root,
                &HSTRING::from(p.as_str()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE | KEY_WOW64_64KEY,
                None,
                &mut hk,
                None,
            );
            win32(&format!("RegCreateKeyEx {p}"), e)?;
            let e = RegSetValueExW(hk, &HSTRING::from(name), None, kind, Some(&bytes));
            let _ = RegCloseKey(hk);
            win32(&format!("RegSetValueEx {name}"), e)
        }
    }

    fn reg_delete_value(&mut self, hive: Hive, path: &str, name: &str) -> Result<()> {
        self.deny_write("reg_delete_value")?;
        let Some(hk) = self.open(hive, path, KEY_SET_VALUE)? else { return Ok(()) };
        let e = unsafe { RegDeleteValueW(hk, &HSTRING::from(name)) };
        unsafe {
            let _ = RegCloseKey(hk);
        }
        match e {
            ERROR_FILE_NOT_FOUND => Ok(()),
            other => win32(&format!("RegDeleteValue {name}"), other),
        }
    }

    fn reg_key_exists(&self, hive: Hive, path: &str) -> Result<bool> {
        Ok(match self.open(hive, path, KEY_READ)? {
            Some(hk) => {
                unsafe {
                    let _ = RegCloseKey(hk);
                }
                true
            }
            None => false,
        })
    }

    fn reg_create_key(&mut self, hive: Hive, path: &str) -> Result<()> {
        self.deny_write("reg_create_key")?;
        let (root, p) = self.map(hive, path);
        unsafe {
            let mut hk = HKEY::default();
            let e = RegCreateKeyExW(
                root,
                &HSTRING::from(p.as_str()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_WOW64_64KEY,
                None,
                &mut hk,
                None,
            );
            win32(&format!("RegCreateKeyEx {p}"), e)?;
            let _ = RegCloseKey(hk);
        }
        Ok(())
    }

    fn reg_delete_tree(&mut self, hive: Hive, path: &str) -> Result<()> {
        self.deny_write("reg_delete_tree")?;
        let (root, p) = self.map(hive, path);
        let hp = HSTRING::from(p.as_str());
        unsafe {
            match RegDeleteTreeW(root, &hp) {
                ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => {}
                e => return win32(&format!("RegDeleteTree {p}"), e),
            }
            match RegDeleteKeyExW(root, &hp, KEY_WOW64_64KEY.0, None) {
                ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
                e => win32(&format!("RegDeleteKeyEx {p}"), e),
            }
        }
    }

    fn reg_values(&self, hive: Hive, path: &str) -> Result<Vec<(String, RegValue)>> {
        let Some(hk) = self.open(hive, path, KEY_READ)? else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        let mut index = 0u32;
        let result = loop {
            let mut name = vec![0u16; 16384];
            let mut name_len = name.len() as u32;
            let mut kind = 0u32;
            let mut data = vec![0u8; 4096];
            let mut data_len = data.len() as u32;
            let mut e = unsafe {
                RegEnumValueW(hk, index, Some(PWSTR(name.as_mut_ptr())), &mut name_len, None, Some(&mut kind),
                    Some(data.as_mut_ptr()), Some(&mut data_len))
            };
            if e == ERROR_MORE_DATA {
                data = vec![0u8; data_len as usize];
                name_len = name.len() as u32;
                e = unsafe {
                    RegEnumValueW(hk, index, Some(PWSTR(name.as_mut_ptr())), &mut name_len, None, Some(&mut kind),
                        Some(data.as_mut_ptr()), Some(&mut data_len))
                };
            }
            match e {
                ERROR_SUCCESS => {
                    data.truncate(data_len as usize);
                    let n = String::from_utf16_lossy(&name[..name_len as usize]);
                    out.push((n, Self::parse_value(REG_VALUE_TYPE(kind), data)));
                    index += 1;
                }
                ERROR_NO_MORE_ITEMS => break Ok(out),
                other => break win32("RegEnumValue", other).map(|_| Vec::new()),
            }
        };
        unsafe {
            let _ = RegCloseKey(hk);
        }
        result
    }

    fn is_elevated(&self) -> bool {
        if let Some(e) = self.elevated_override {
            return e;
        }
        use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }
            let mut elev = TOKEN_ELEVATION::default();
            let mut len = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elev as *mut _ as *mut _),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            )
            .is_ok();
            let _ = CloseHandle(token);
            ok && elev.TokenIsElevated != 0
        }
    }

    fn spi_get(&self, item: SpiItem) -> Result<u32> {
        use windows::Win32::UI::Accessibility::{FILTERKEYS, STICKYKEYS, TOGGLEKEYS};
        use windows::Win32::UI::WindowsAndMessaging::*;
        let none = SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0);
        unsafe {
            match item {
                SpiItem::StickyKeysFlags => {
                    let mut s = STICKYKEYS { cbSize: std::mem::size_of::<STICKYKEYS>() as u32, ..Default::default() };
                    SystemParametersInfoW(SPI_GETSTICKYKEYS, s.cbSize, Some(&mut s as *mut _ as *mut _), none)
                        .map_err(|e| hr("SPI_GETSTICKYKEYS", e))?;
                    Ok(s.dwFlags.0)
                }
                SpiItem::FilterKeysFlags => {
                    let mut s = FILTERKEYS { cbSize: std::mem::size_of::<FILTERKEYS>() as u32, ..Default::default() };
                    SystemParametersInfoW(SPI_GETFILTERKEYS, s.cbSize, Some(&mut s as *mut _ as *mut _), none)
                        .map_err(|e| hr("SPI_GETFILTERKEYS", e))?;
                    Ok(s.dwFlags)
                }
                SpiItem::ToggleKeysFlags => {
                    let mut s = TOGGLEKEYS { cbSize: std::mem::size_of::<TOGGLEKEYS>() as u32, ..Default::default() };
                    SystemParametersInfoW(SPI_GETTOGGLEKEYS, s.cbSize, Some(&mut s as *mut _ as *mut _), none)
                        .map_err(|e| hr("SPI_GETTOGGLEKEYS", e))?;
                    Ok(s.dwFlags)
                }
                SpiItem::ClientAreaAnimation => {
                    let mut b = windows::core::BOOL(0);
                    SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut b as *mut _ as *mut _), none)
                        .map_err(|e| hr("SPI_GETCLIENTAREAANIMATION", e))?;
                    Ok(b.as_bool() as u32)
                }
                SpiItem::MinimizeAnimation => {
                    let mut a = ANIMATIONINFO { cbSize: std::mem::size_of::<ANIMATIONINFO>() as u32, iMinAnimate: 0 };
                    SystemParametersInfoW(SPI_GETANIMATION, a.cbSize, Some(&mut a as *mut _ as *mut _), none)
                        .map_err(|e| hr("SPI_GETANIMATION", e))?;
                    Ok((a.iMinAnimate != 0) as u32)
                }
            }
        }
    }

    fn spi_set(&mut self, item: SpiItem, value: u32) -> Result<()> {
        self.deny_system_change("spi_set")?;
        use windows::Win32::UI::Accessibility::{FILTERKEYS, STICKYKEYS, STICKYKEYS_FLAGS, TOGGLEKEYS};
        use windows::Win32::UI::WindowsAndMessaging::*;
        let none = SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0);
        let save = SPIF_UPDATEINIFILE | SPIF_SENDCHANGE;
        unsafe {
            match item {
                SpiItem::StickyKeysFlags => {
                    let mut s = STICKYKEYS { cbSize: std::mem::size_of::<STICKYKEYS>() as u32, ..Default::default() };
                    SystemParametersInfoW(SPI_GETSTICKYKEYS, s.cbSize, Some(&mut s as *mut _ as *mut _), none)
                        .map_err(|e| hr("SPI_GETSTICKYKEYS", e))?;
                    s.dwFlags = STICKYKEYS_FLAGS(value);
                    SystemParametersInfoW(SPI_SETSTICKYKEYS, s.cbSize, Some(&mut s as *mut _ as *mut _), save)
                        .map_err(|e| hr("SPI_SETSTICKYKEYS", e))
                }
                SpiItem::FilterKeysFlags => {
                    // read the whole struct first so the timing fields stay as they are
                    let mut s = FILTERKEYS { cbSize: std::mem::size_of::<FILTERKEYS>() as u32, ..Default::default() };
                    SystemParametersInfoW(SPI_GETFILTERKEYS, s.cbSize, Some(&mut s as *mut _ as *mut _), none)
                        .map_err(|e| hr("SPI_GETFILTERKEYS", e))?;
                    s.dwFlags = value;
                    SystemParametersInfoW(SPI_SETFILTERKEYS, s.cbSize, Some(&mut s as *mut _ as *mut _), save)
                        .map_err(|e| hr("SPI_SETFILTERKEYS", e))
                }
                SpiItem::ToggleKeysFlags => {
                    let mut s = TOGGLEKEYS { cbSize: std::mem::size_of::<TOGGLEKEYS>() as u32, dwFlags: value };
                    SystemParametersInfoW(SPI_SETTOGGLEKEYS, s.cbSize, Some(&mut s as *mut _ as *mut _), save)
                        .map_err(|e| hr("SPI_SETTOGGLEKEYS", e))
                }
                SpiItem::ClientAreaAnimation => {
                    // the BOOL goes in pvParam itself
                    SystemParametersInfoW(SPI_SETCLIENTAREAANIMATION, 0, Some((value != 0) as usize as *mut _), save)
                        .map_err(|e| hr("SPI_SETCLIENTAREAANIMATION", e))
                }
                SpiItem::MinimizeAnimation => {
                    let mut a = ANIMATIONINFO { cbSize: std::mem::size_of::<ANIMATIONINFO>() as u32, iMinAnimate: (value != 0) as i32 };
                    SystemParametersInfoW(SPI_SETANIMATION, a.cbSize, Some(&mut a as *mut _ as *mut _), save)
                        .map_err(|e| hr("SPI_SETANIMATION", e))
                }
            }
        }
    }

    fn reload_language_hotkeys(&mut self) -> Result<()> {
        if !self.after_step("reload_language_hotkeys".into())? {
            return Ok(());
        }
        use windows::Win32::UI::WindowsAndMessaging::*;
        unsafe {
            SystemParametersInfoW(SPI_SETLANGTOGGLE, 0, None, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0))
                .map_err(|e| hr("SPI_SETLANGTOGGLE", e))
        }
    }

    fn power_read(&self, setting: PowerSetting) -> Result<PowerValues> {
        use windows::Win32::System::Power::*;
        let (sub, set) = power_guids(setting);
        unsafe {
            let mut scheme: *mut windows::core::GUID = std::ptr::null_mut();
            win32("PowerGetActiveScheme", PowerGetActiveScheme(None, &mut scheme))?;
            let mut ac = 0u32;
            let mut dc = 0u32;
            let e1 = PowerReadACValueIndex(None, Some(scheme), Some(&sub), Some(&set), &mut ac);
            let e2 = PowerReadDCValueIndex(None, Some(scheme), Some(&sub), Some(&set), &mut dc);
            let _ = LocalFree(Some(HLOCAL(scheme as *mut _)));
            win32("PowerReadACValueIndex", e1)?;
            win32("PowerReadDCValueIndex", WIN32_ERROR(e2))?;
            Ok(PowerValues { ac, dc })
        }
    }

    fn power_write(&mut self, setting: PowerSetting, values: PowerValues) -> Result<()> {
        self.deny_system_change("power_write")?;
        use windows::Win32::System::Power::*;
        let (sub, set) = power_guids(setting);
        unsafe {
            let mut scheme: *mut windows::core::GUID = std::ptr::null_mut();
            win32("PowerGetActiveScheme", PowerGetActiveScheme(None, &mut scheme))?;
            let r = (|| {
                win32("PowerWriteACValueIndex", PowerWriteACValueIndex(None, scheme, Some(&sub), Some(&set), values.ac))?;
                win32("PowerWriteDCValueIndex", WIN32_ERROR(PowerWriteDCValueIndex(None, scheme, Some(&sub), Some(&set), values.dc)))?;
                win32("PowerSetActiveScheme", PowerSetActiveScheme(None, Some(scheme)))
            })();
            let _ = LocalFree(Some(HLOCAL(scheme as *mut _)));
            r
        }
    }

    fn has_battery(&self) -> bool {
        use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        let mut s = SYSTEM_POWER_STATUS::default();
        // BatteryFlag 128 = no system battery, 255 = unknown
        unsafe { GetSystemPowerStatus(&mut s).is_ok() && s.BatteryFlag != 128 && s.BatteryFlag != 255 }
    }

    fn hibernate_on(&self) -> Result<bool> {
        use windows::Win32::System::Power::{GetPwrCapabilities, SYSTEM_POWER_CAPABILITIES};
        let mut caps = SYSTEM_POWER_CAPABILITIES::default();
        if !unsafe { GetPwrCapabilities(&mut caps) } {
            return Err(Error::os("GetPwrCapabilities", unsafe { GetLastError().0 } as i64));
        }
        Ok(caps.SystemS4 && caps.HiberFilePresent)
    }

    fn gpu_scheduling(&self) -> Result<GpuScheduling> {
        use windows::Wdk::Graphics::Direct3D::*;
        unsafe {
            let mut e = D3DKMT_ENUMADAPTERS2::default();
            let st = D3DKMTEnumAdapters2(&mut e);
            if st.0 != 0 {
                return Err(Error::os("D3DKMTEnumAdapters2", st.0 as u32 as i64));
            }
            let mut adapters = vec![D3DKMT_ADAPTERINFO::default(); e.NumAdapters as usize];
            e.pAdapters = adapters.as_mut_ptr();
            let st = D3DKMTEnumAdapters2(&mut e);
            if st.0 != 0 {
                return Err(Error::os("D3DKMTEnumAdapters2", st.0 as u32 as i64));
            }
            adapters.truncate(e.NumAdapters as usize);
            let mut found = GpuScheduling::default();
            for a in &adapters {
                let mut caps = D3DKMT_WDDM_2_7_CAPS::default();
                let mut q = D3DKMT_QUERYADAPTERINFO {
                    hAdapter: a.hAdapter,
                    Type: KMTQAITYPE_WDDM_2_7_CAPS,
                    pPrivateDriverData: &mut caps as *mut _ as *mut _,
                    PrivateDriverDataSize: std::mem::size_of::<D3DKMT_WDDM_2_7_CAPS>() as u32,
                };
                // bit 0 HwSchSupported, bit 1 HwSchEnabled, bit 2 HwSchEnabledByDefault (d3dkmthk.h)
                if D3DKMTQueryAdapterInfo(&mut q).0 == 0 && !found.supported {
                    let bits = caps.Anonymous.Value;
                    if bits & 1 != 0 {
                        found = GpuScheduling { supported: true, enabled_now: bits & 2 != 0, enabled_by_default: bits & 4 != 0 };
                    }
                }
                let close = D3DKMT_CLOSEADAPTER { hAdapter: a.hAdapter };
                let _ = D3DKMTCloseAdapter(&close);
            }
            Ok(found)
        }
    }

    fn bluetooth(&self) -> Result<Option<bool>> {
        use windows::Devices::Radios::{Radio, RadioKind, RadioState};
        let radios = Radio::GetRadiosAsync().and_then(|op| op.join()).map_err(|e| hr("Radio.GetRadiosAsync", e))?;
        for r in radios {
            if r.Kind().map_err(|e| hr("Radio.Kind", e))? == RadioKind::Bluetooth {
                return Ok(Some(r.State().map_err(|e| hr("Radio.State", e))? == RadioState::On));
            }
        }
        Ok(None)
    }

    fn set_bluetooth(&mut self, on: bool) -> Result<()> {
        self.deny_system_change("set_bluetooth")?;
        use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState};
        let access = Radio::RequestAccessAsync().and_then(|op| op.join()).map_err(|e| hr("Radio.RequestAccessAsync", e))?;
        if access != RadioAccessStatus::Allowed {
            return Err(Error::os("Radio.RequestAccessAsync (not allowed)", access.0 as i64));
        }
        let radios = Radio::GetRadiosAsync().and_then(|op| op.join()).map_err(|e| hr("Radio.GetRadiosAsync", e))?;
        for r in radios {
            if r.Kind().map_err(|e| hr("Radio.Kind", e))? == RadioKind::Bluetooth {
                let st = if on { RadioState::On } else { RadioState::Off };
                let res = r.SetStateAsync(st).and_then(|op| op.join()).map_err(|e| hr("Radio.SetStateAsync", e))?;
                return if res == RadioAccessStatus::Allowed {
                    Ok(())
                } else {
                    Err(Error::os("Radio.SetStateAsync (not allowed)", res.0 as i64))
                };
            }
        }
        Err(Error::os("no Bluetooth radio", 0x8007_0490u32 as i64))
    }

    fn copilot_installed(&self) -> Result<bool> {
        use windows::Management::Deployment::PackageManager;
        let pm = PackageManager::new().map_err(|e| hr("PackageManager", e))?;
        let pkgs = pm
            .FindPackagesByUserSecurityIdPackageFamilyName(&HSTRING::new(), &HSTRING::from(COPILOT_FAMILY))
            .map_err(|e| hr("FindPackagesForUser", e))?;
        let it = pkgs.First().map_err(|e| hr("IIterable.First", e))?;
        it.HasCurrent().map_err(|e| hr("HasCurrent", e))
    }

    fn remove_copilot(&mut self) -> Result<()> {
        self.deny_system_change("remove_copilot")?;
        use windows::Management::Deployment::PackageManager;
        let pm = PackageManager::new().map_err(|e| hr("PackageManager", e))?;
        let pkgs = pm
            .FindPackagesByUserSecurityIdPackageFamilyName(&HSTRING::new(), &HSTRING::from(COPILOT_FAMILY))
            .map_err(|e| hr("FindPackagesForUser", e))?;
        for p in pkgs {
            let full = p.Id().and_then(|id| id.FullName()).map_err(|e| hr("Package.Id.FullName", e))?;
            pm.RemovePackageAsync(&full).and_then(|op| op.join()).map_err(|e| hr("RemovePackageAsync", e))?;
        }
        Ok(())
    }

    fn broadcast_setting_change(&mut self, area: Option<&str>) -> Result<()> {
        if !self.after_step(format!("broadcast:{}", area.unwrap_or("")))? {
            return Ok(());
        }
        use windows::Win32::UI::WindowsAndMessaging::*;
        let h = area.map(HSTRING::from);
        let lp = h.as_ref().map(|h| LPARAM(h.as_ptr() as isize)).unwrap_or(LPARAM(0));
        unsafe {
            SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, WPARAM(0), lp, SMTO_ABORTIFHUNG, 1000, None);
        }
        Ok(())
    }

    fn restart_explorer(&mut self) -> Result<()> {
        if !self.after_step("restart_explorer".into())? {
            return Ok(());
        }
        restart_explorer_now()
    }

    fn refresh_shell(&mut self) -> Result<()> {
        if !self.after_step("refresh_shell".into())? {
            return Ok(());
        }
        // the values just written (an undo may have deleted one)
        let (ext, hid) = shell_flags(self.reg_read(Hive::Hkcu, crate::rows::ADV, "HideFileExt")?, self.reg_read(Hive::Hkcu, crate::rows::ADV, "Hidden")?);
        // off the caller's thread: the shell's own broadcast may wait on a slow window - the menu never does
        let _ = std::thread::Builder::new().name("bu-shell-refresh".into()).spawn(move || refresh_shell_now(ext, hid));
        Ok(())
    }

    fn registered_browsers(&self) -> Result<Vec<RegisteredBrowser>> {
        let mut out = Vec::new();
        for (hive, machine) in [(Hive::Hklm, true), (Hive::Hkcu, false)] {
            for (reg_name, v) in self.reg_values(hive, r"SOFTWARE\RegisteredApplications")? {
                let RegValue::Sz(cap) = v else { continue };
                let https = match self.reg_read(hive, &format!(r"{cap}\URLAssociations"), "https")? {
                    Some(RegValue::Sz(p)) => p,
                    _ => continue,
                };
                let display_name = match self.reg_read(hive, &cap, "ApplicationName")? {
                    Some(RegValue::Sz(n)) => resolve_indirect(&n).unwrap_or_else(|| reg_name.clone()),
                    _ => reg_name.clone(),
                };
                out.push(RegisteredBrowser { reg_name, display_name, machine, https_progid: Some(https) });
            }
        }
        Ok(out)
    }

    fn default_browser_progid(&self) -> Result<Option<String>> {
        // READ only. Newer builds keep it in UserChoiceLatest; older in UserChoice.
        for key in [
            r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoiceLatest",
            r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice",
        ] {
            if let Some(RegValue::Sz(p)) = self.reg_read(Hive::Hkcu, key, "ProgId")? {
                return Ok(Some(p));
            }
        }
        Ok(None)
    }

    fn assoc_app(&self, what: &str) -> Result<Option<AssocApp>> {
        use windows::Win32::UI::Shell::*;
        let flags = if what.starts_with('.') { ASSOCF_NOTRUNCATE } else { ASSOCF_NOTRUNCATE | ASSOCF_IS_PROTOCOL };
        let query = |s: ASSOCSTR| -> Option<String> {
            let mut buf = vec![0u16; 1024];
            let mut len = buf.len() as u32;
            let r = unsafe { AssocQueryStringW(flags, s, &HSTRING::from(what), PCWSTR::null(), Some(PWSTR(buf.as_mut_ptr())), &mut len) };
            r.is_ok().then(|| wide_to_string(&buf)).filter(|s| !s.is_empty())
        };
        // raw answer; "Pick an app" (OpenWith.exe) is turned into "no default" by `defaults::read` (tested)
        Ok(query(ASSOCSTR_FRIENDLYAPPNAME).map(|name| AssocApp { name, exe: query(ASSOCSTR_EXECUTABLE) }))
    }

    fn open_uri(&mut self, uri: &str) -> Result<()> {
        if !self.after_step(format!("open_uri:{uri}"))? {
            return Ok(());
        }
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let r = unsafe {
            ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(uri), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL)
        };
        if r.0 as isize > 32 {
            Ok(())
        } else {
            Err(Error::os("ShellExecute", r.0 as isize as i64))
        }
    }

    fn open_with_dialog(&mut self, ext: &str) -> Result<()> {
        if !self.after_step(format!("open_with:{ext}"))? {
            return Ok(());
        }
        use windows::Win32::UI::Shell::*;
        // a file name with this extension (it doesn't have to exist: registration only, no "open")
        let file = HSTRING::from(std::env::temp_dir().join(format!("BoylerUtilities-sample{ext}")).to_string_lossy().as_ref());
        let info = OPENASINFO {
            pcszFile: PCWSTR(file.as_ptr()),
            pcszClass: PCWSTR::null(),
            oaifInFlags: OAIF_ALLOW_REGISTRATION | OAIF_REGISTER_EXT,
        };
        unsafe { SHOpenWithDialog(None, &info) }.map_err(|e| hr("SHOpenWithDialog", e))
    }
}

/// The only place scratch keys may live.
pub const SCRATCH_PREFIX: &str = r"Software\BoylerUtilities-test\";

/// Deletes a scratch key (`HKCU\<base>`, which must start with `Software\BoylerUtilities-test\`), then its parent keys while
/// they are empty — but never `Software\BoylerUtilities-test` itself: other lanes create their scratch keys under it at the
/// same time, and deleting it under them makes their key creation fail (measured: ERROR_KEY_DELETED 1018). Anything else is
/// refused.
fn is_scratch_path(base: &str) -> bool {
    let lower = base.to_lowercase();
    lower.starts_with(&SCRATCH_PREFIX.to_lowercase()) && lower.len() > SCRATCH_PREFIX.len() && !base.contains("..")
}

pub fn remove_scratch(base: &str) -> Result<()> {
    let base = base.trim_matches('\\');
    if !is_scratch_path(base) {
        return Err(Error::ReadOnly(format!("not a scratch key: {base}")));
    }
    unsafe {
        let hp = HSTRING::from(base);
        match RegDeleteTreeW(HKEY_CURRENT_USER, &hp) {
            ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => {}
            e => return win32("RegDeleteTree scratch", e),
        }
        let _ = RegDeleteKeyExW(HKEY_CURRENT_USER, &hp, KEY_WOW64_64KEY.0, None);
        // parents: deleted only when empty (RegDeleteKeyEx refuses a key that still has subkeys)
        let mut parent = base.to_string();
        while let Some(i) = parent.rfind('\\') {
            parent.truncate(i);
            if parent.eq_ignore_ascii_case(SCRATCH_PREFIX.trim_end_matches('\\')) || !parent.contains('\\') {
                break;
            }
            let _ = RegDeleteKeyExW(HKEY_CURRENT_USER, &HSTRING::from(parent.as_str()), KEY_WOW64_64KEY.0, None);
        }
    }
    Ok(())
}

/// "@{…}" / "@file,-id" resource strings → text.
fn resolve_indirect(s: &str) -> Option<String> {
    if !s.starts_with('@') {
        return Some(s.to_string());
    }
    let mut buf = vec![0u16; 512];
    unsafe { windows::Win32::UI::Shell::SHLoadIndirectString(&HSTRING::from(s), &mut buf, None) }.ok()?;
    Some(wide_to_string(&buf)).filter(|s| !s.is_empty())
}

/// (show extensions, show hidden files) as the two rows read them: HideFileExt 0 = show (missing = hidden); Hidden 1 =
/// show (2 or missing = don't). `SHGetSetSettings` writes them back, so a missing value must stay "off" - else an undo that
/// deleted HideFileExt came back as HideFileExt = 0 (extensions on, the switch on again).
fn shell_flags(hide_ext: Option<RegValue>, hidden: Option<RegValue>) -> (bool, bool) {
    (matches!(hide_ext, Some(RegValue::Dword(0))), matches!(hidden, Some(RegValue::Dword(1))))
}

/// The shell's own refresh for "Show file extensions" / "Show hidden files" (Order 043: no Explorer restart, it blacked out
/// the screen for ~30 s on a test PC). Runs on its own thread (see `refresh_shell`).
/// 1. `SHGetSetSettings(SSF_SHOWEXTENSIONS | SSF_SHOWALLOBJECTS, set)` — the call Folder Options makes: updates the shell's
///    cached state (the registry values are already written, it writes the same ones) and tells Explorer.
/// 2. `SHChangeNotify(SHCNE_ASSOCCHANGED, FLUSHNOWAIT)` — every shell view re-reads its items.
/// 3. Refresh posted to every open folder window (`WM_COMMAND` 41504 = View › Refresh) and the desktop's view (28931).
fn refresh_shell_now(show_ext: bool, show_hidden: bool) {
    use windows::Win32::UI::Shell::*;
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, FindWindowExW, GetClassNameW, PostMessageW, WM_COMMAND};
    // SHELLSTATE's first bit field: bit 0 fShowAllObjects, bit 1 fShowExtensions
    let mut ss = SHELLSTATEA { _bitfield1: (show_hidden as i32) | ((show_ext as i32) << 1), ..Default::default() };
    unsafe {
        SHGetSetSettings(Some(&mut ss), SSF_SHOWEXTENSIONS | SSF_SHOWALLOBJECTS, true);
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST | SHCNF_FLUSHNOWAIT, None, None);
    }
    unsafe extern "system" fn each(w: HWND, _: LPARAM) -> windows::core::BOOL {
        let mut buf = [0u16; 64];
        let n = unsafe { GetClassNameW(w, &mut buf) } as usize;
        let class = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
        unsafe {
            match class.as_str() {
                "CabinetWClass" | "ExploreWClass" => {
                    let _ = PostMessageW(Some(w), WM_COMMAND, WPARAM(41504), LPARAM(0));
                }
                // the desktop's icons live in a SHELLDLL_DefView under Progman (or a WorkerW when a wallpaper slideshow runs)
                "Progman" | "WorkerW" => {
                    if let Ok(view) = FindWindowExW(Some(w), None, &HSTRING::from("SHELLDLL_DefView"), PCWSTR::null()) {
                        let _ = PostMessageW(Some(view), WM_COMMAND, WPARAM(28931), LPARAM(0));
                    }
                }
                _ => {}
            }
        }
        TRUE
    }
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(0));
    }
}

/// Restarts Explorer with the Restart Manager (Explorer re-opens its folder windows), then makes sure the taskbar is back.
fn restart_explorer_now() -> Result<()> {
    use windows::Win32::System::RestartManager::*;
    use windows::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId, SW_SHOWNORMAL};
    unsafe {
        let tray = FindWindowW(&HSTRING::from("Shell_TrayWnd"), PCWSTR::null()).map_err(|e| hr("FindWindow(Shell_TrayWnd)", e))?;
        let mut pid = 0u32;
        GetWindowThreadProcessId(tray, Some(&mut pid));
        let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).map_err(|e| hr("OpenProcess(explorer)", e))?;
        let (mut created, mut exit, mut kernel, mut user) = Default::default();
        let times = GetProcessTimes(proc, &mut created, &mut exit, &mut kernel, &mut user);
        let _ = CloseHandle(proc);
        times.map_err(|e| hr("GetProcessTimes", e))?;
        let mut session = 0u32;
        let mut key = [0u16; 64];
        win32("RmStartSession", RmStartSession(&mut session, None, PWSTR(key.as_mut_ptr())))?;
        let app = RM_UNIQUE_PROCESS { dwProcessId: pid, ProcessStartTime: created };
        let r = (|| {
            win32("RmRegisterResources", RmRegisterResources(session, None, Some(&[app]), None))?;
            win32("RmShutdown", RmShutdown(session, RmForceShutdown.0 as u32, None))?;
            win32("RmRestart", RmRestart(session, None, None))
        })();
        let _ = RmEndSession(session);
        r?;
        // one-shot wait (not a background poll): give the taskbar up to 5 s to come back, else start Explorer ourselves
        for _ in 0..50 {
            if FindWindowW(&HSTRING::from("Shell_TrayWnd"), PCWSTR::null()).is_ok() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from("explorer.exe"), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_flags_read_like_the_rows() {
        assert_eq!(shell_flags(Some(RegValue::Dword(0)), Some(RegValue::Dword(1))), (true, true));
        assert_eq!(shell_flags(Some(RegValue::Dword(1)), Some(RegValue::Dword(2))), (false, false));
        // missing (an undo deleted the value): off, as the rows read it - never written back as "on"
        assert_eq!(shell_flags(None, None), (false, false));
    }
}
