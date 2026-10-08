//! The REAL Windows implementation of [`FixOs`]. [`RealOs::read_only`] reads (adapters, cache files, restore points,
//! the clock) and refuses EVERY change — no key chord, no program started, no file deleted, Explorer never stopped,
//! no restore point. Only `examples/show` uses the real layer; tests use the fake.

mod explorer;
mod proc;
mod wmi;

use crate::cache::is_cache_file;
use crate::os::{CreateCall, DisplayAdapter, ExplorerPause, FixOs, LocalTime, RestorePoint, RestoreStatus, Spawned, Stamp};
use crate::restore::parse_cim_datetime;
use crate::{FixError, Result};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use windows::core::{HSTRING, PCSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW,
    SetupDiGetDeviceRegistryPropertyW, DIGCF_PRESENT, GUID_DEVCLASS_DISPLAY, SETUP_DI_REGISTRY_PROPERTY, SPDRP_DEVICEDESC,
    SPDRP_FRIENDLYNAME, SP_DEVINFO_DATA,
};
use windows::Win32::Foundation::{CloseHandle, FreeLibrary, FILETIME, HANDLE, SYSTEMTIME};
use windows::Win32::System::Com::{
    CoInitializeEx, CoInitializeSecurity, CoUninitialize, COINIT_MULTITHREADED, EOAC_DYNAMIC_CLOAKING,
    RPC_C_AUTHN_LEVEL_PKT_PRIVACY, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD};
use windows::Win32::System::Restore::{
    BEGIN_SYSTEM_CHANGE, END_SYSTEM_CHANGE, MODIFY_SETTINGS, RESTOREPOINTINFOW, STATEMGRSTATUS,
};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    ReadWrite,
    ReadOnly,
}

/// The real Windows. `new()` can do everything (the app, elevated where needed); `read_only()` refuses every change.
pub struct RealOs {
    mode: Mode,
}

impl RealOs {
    pub fn new() -> Self {
        RealOs { mode: Mode::ReadWrite }
    }
    pub fn read_only() -> Self {
        RealOs { mode: Mode::ReadOnly }
    }
    fn change(&self, what: &str) -> Result<()> {
        match self.mode {
            Mode::ReadWrite => Ok(()),
            Mode::ReadOnly => Err(FixError::Refused(format!("read-only: {what}"))),
        }
    }
}

impl Default for RealOs {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn win_err(context: impl Into<String>) -> impl FnOnce(windows::core::Error) -> FixError {
    let context = context.into();
    move |e| FixError::Os { context, code: e.code().0 as u32 }
}

fn system32() -> PathBuf {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join("System32")
}

unsafe fn reg_string(set: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO, info: &SP_DEVINFO_DATA, p: SETUP_DI_REGISTRY_PROPERTY) -> Option<String> {
    let mut raw = [0u8; 1024];
    let mut need = 0u32;
    SetupDiGetDeviceRegistryPropertyW(set, info, p, None, Some(&mut raw), Some(&mut need)).ok()?;
    let w: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|&c| c != 0).collect();
    (!w.is_empty()).then(|| String::from_utf16_lossy(&w))
}

impl FixOs for RealOs {
    fn is_elevated(&self) -> bool {
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

    fn foreground_is_ours(&self) -> bool {
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
        unsafe {
            let w = GetForegroundWindow();
            if w.is_invalid() {
                return false;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(w, Some(&mut pid));
            pid == std::process::id()
        }
    }

    fn send_reset_chord(&self) -> Result<()> {
        self.change("send_reset_chord")?;
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
            VIRTUAL_KEY, VK_LCONTROL, VK_LSHIFT, VK_LWIN,
        };
        let key = |vk: VIRTUAL_KEY, up: bool| {
            let mut flags = if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) };
            if vk == VK_LWIN {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
            }
        };
        let b = VIRTUAL_KEY(0x42);
        let inputs = [
            key(VK_LWIN, false),
            key(VK_LCONTROL, false),
            key(VK_LSHIFT, false),
            key(b, false),
            key(b, true),
            key(VK_LSHIFT, true),
            key(VK_LCONTROL, true),
            key(VK_LWIN, true),
        ];
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent as usize == inputs.len() {
            Ok(())
        } else {
            // UIPI: a higher-integrity window in front blocks injected input
            Err(FixError::Os { context: format!("SendInput ({sent} of {} keys)", inputs.len()), code: 0 })
        }
    }

    fn display_adapters(&self) -> Result<Vec<DisplayAdapter>> {
        unsafe {
            let set = SetupDiGetClassDevsW(Some(&GUID_DEVCLASS_DISPLAY), None, None, DIGCF_PRESENT)
                .map_err(win_err("SetupDiGetClassDevs(Display)"))?;
            let mut out = Vec::new();
            let mut i = 0;
            loop {
                let mut info = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
                if SetupDiEnumDeviceInfo(set, i, &mut info).is_err() {
                    break;
                }
                i += 1;
                let mut buf = [0u16; 512];
                if SetupDiGetDeviceInstanceIdW(set, &info, Some(&mut buf), None).is_err() {
                    continue;
                }
                let instance_id = String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(buf.len())]);
                let name = reg_string(set, &info, SPDRP_FRIENDLYNAME)
                    .or_else(|| reg_string(set, &info, SPDRP_DEVICEDESC))
                    .unwrap_or_else(|| "Display adapter".into());
                out.push(DisplayAdapter { name, instance_id });
            }
            let _ = SetupDiDestroyDeviceInfoList(set);
            Ok(out)
        }
    }

    fn spawn(&self, program: &str, args: &[&str]) -> Result<Spawned> {
        self.change(program)?;
        proc::spawn(&system32().join(program), args)
    }

    fn cbs_log_tail(&self) -> Result<String> {
        const MAX: u64 = 4 << 20;
        let root = std::env::var_os("windir").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let path = root.join(r"Logs\CBS\CBS.log");
        let mut f = std::fs::File::open(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::PermissionDenied => FixError::NeedsAdmin("CBS.log".into()),
            _ => FixError::Os { context: format!("open {}", path.display()), code: e.raw_os_error().unwrap_or(0) as u32 },
        })?;
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if len > MAX {
            let _ = f.seek(SeekFrom::Start(len - MAX));
        }
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes).map_err(|e| FixError::Os { context: "read CBS.log".into(), code: e.raw_os_error().unwrap_or(0) as u32 })?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn explorer_cache_dir(&self) -> Result<PathBuf> {
        use windows::Win32::System::Com::CoTaskMemFree;
        use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
        unsafe {
            let p = SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None).map_err(win_err("SHGetKnownFolderPath"))?;
            let s = p.to_string();
            CoTaskMemFree(Some(p.0 as *const _));
            let base = s.map_err(|_| FixError::Unavailable("LocalAppData path".into()))?;
            Ok(PathBuf::from(base).join(r"Microsoft\Windows\Explorer"))
        }
    }

    fn list_files(&self, dir: &Path) -> Result<Vec<(String, u64)>> {
        let rd = std::fs::read_dir(dir)
            .map_err(|e| FixError::Os { context: format!("list {}", dir.display()), code: e.raw_os_error().unwrap_or(0) as u32 })?;
        Ok(rd
            .flatten()
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                m.is_file().then(|| (e.file_name().to_string_lossy().into_owned(), m.len()))
            })
            .collect())
    }

    fn delete_file(&self, path: &Path) -> Result<()> {
        self.change("delete_file")?;
        // defence in depth: this layer deletes nothing but Explorer's cache files in Explorer's cache folder
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !is_cache_file(&name) || path.parent() != Some(self.explorer_cache_dir()?.as_path()) {
            return Err(FixError::Refused(format!("not an Explorer cache file: {}", path.display())));
        }
        std::fs::remove_file(path)
            .map_err(|e| FixError::Os { context: format!("delete {name}"), code: e.raw_os_error().unwrap_or(0) as u32 })
    }

    fn stop_explorer(&self) -> Result<Box<dyn ExplorerPause>> {
        self.change("stop_explorer")?;
        explorer::stop()
    }

    fn restore_status(&self) -> Result<RestoreStatus> {
        let frequency_minutes = unsafe {
            let mut v = 0u32;
            let mut n = std::mem::size_of::<u32>() as u32;
            let r = RegGetValueW(
                HKEY_LOCAL_MACHINE,
                &HSTRING::from(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\SystemRestore"),
                &HSTRING::from("SystemRestorePointCreationFrequency"),
                RRF_RT_REG_DWORD,
                None,
                Some(&mut v as *mut u32 as *mut _),
                Some(&mut n),
            );
            if r.is_ok() { v } else { 1440 } // missing = Windows' default 24 h
        };
        let rows = wmi::Wmi::connect(r"root\default")
            .and_then(|w| w.query("SELECT CreationTime, Description, SequenceNumber FROM SystemRestore", &["CreationTime", "Description", "SequenceNumber"]));
        match rows {
            Ok(rows) => {
                let newest = rows
                    .iter()
                    .filter_map(|r| {
                        Some(RestorePoint {
                            created: parse_cim_datetime(r.get("CreationTime")?.as_str()?)?,
                            description: r.get("Description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            sequence: r.get("SequenceNumber").and_then(|v| v.as_i64()).unwrap_or(0) as u32,
                        })
                    })
                    .max_by_key(|p| (p.created, p.sequence));
                Ok(RestoreStatus { frequency_minutes, newest, newest_known: true })
            }
            Err(FixError::NeedsAdmin(_)) => Ok(RestoreStatus { frequency_minutes, newest: None, newest_known: false }),
            Err(e) => Err(e),
        }
    }

    fn create_restore_point(&self, description: &str) -> Result<CreateCall> {
        self.change("create_restore_point")?;
        type SrSet = unsafe extern "system" fn(*const RESTOREPOINTINFOW, *mut STATEMGRSTATUS) -> windows::core::BOOL;
        const ERROR_SERVICE_DISABLED: u32 = 1058;
        unsafe {
            // COM + security first (Microsoft's System Restore sample); "already set" errors are fine
            let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            let _ = CoInitializeSecurity(
                None,
                -1,
                None,
                None,
                RPC_C_AUTHN_LEVEL_PKT_PRIVACY,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_DYNAMIC_CLOAKING,
                None,
            );
            let r = (|| {
                let lib = LoadLibraryExW(&HSTRING::from("srclient.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)
                    .map_err(|_| FixError::Unavailable("System Restore (srclient.dll) is not on this PC".into()))?;
                let r = (|| {
                    let f = GetProcAddress(lib, PCSTR(c"SRSetRestorePointW".as_ptr() as *const u8))
                        .ok_or_else(|| FixError::Unavailable("SRSetRestorePointW".into()))?;
                    let set: SrSet = std::mem::transmute::<unsafe extern "system" fn() -> isize, SrSet>(f);
                    // the struct is packed: fill the text first, then move it in whole
                    let mut text = [0u16; 256];
                    for (d, s) in text.iter_mut().zip(description.encode_utf16().take(255)) {
                        *d = s;
                    }
                    let mut info = RESTOREPOINTINFOW {
                        dwEventType: BEGIN_SYSTEM_CHANGE,
                        dwRestorePtType: MODIFY_SETTINGS,
                        llSequenceNumber: 0,
                        szDescription: text,
                    };
                    let mut st = STATEMGRSTATUS::default();
                    if !set(&info, &mut st).as_bool() {
                        let code = st.nStatus.0;
                        return if code == ERROR_SERVICE_DISABLED {
                            Ok(CreateCall::ProtectionOff)
                        } else {
                            Err(FixError::Os { context: "SRSetRestorePointW".into(), code })
                        };
                    }
                    info.dwEventType = END_SYSTEM_CHANGE;
                    info.llSequenceNumber = st.llSequenceNumber;
                    let mut st2 = STATEMGRSTATUS::default();
                    let _ = set(&info, &mut st2);
                    Ok(CreateCall::Accepted)
                })();
                let _ = FreeLibrary(lib);
                r
            })();
            if com {
                CoUninitialize();
            }
            r
        }
    }

    fn now(&self) -> Stamp {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Stamp(secs)
    }

    fn local(&self, t: Stamp) -> LocalTime {
        // Unix seconds → FILETIME (100 ns since 1601) → UTC SYSTEMTIME → local SYSTEMTIME (the PC's time zone rules)
        let ticks = (t.0 + 11_644_473_600) as u64 * 10_000_000;
        let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
        unsafe {
            let mut utc = SYSTEMTIME::default();
            let mut loc = SYSTEMTIME::default();
            if FileTimeToSystemTime(&ft, &mut utc).is_err() || SystemTimeToTzSpecificLocalTime(None, &utc, &mut loc).is_err() {
                return crate::fake::utc(t);
            }
            LocalTime { year: loc.wYear, month: loc.wMonth as u8, day: loc.wDay as u8, hour: loc.wHour as u8, minute: loc.wMinute as u8 }
        }
    }
}
