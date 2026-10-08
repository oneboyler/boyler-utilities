//! The Windows layer. Reads are read-only (registry, the driver's device, a file's signature); the only change is
//! `run_elevated`, which starts the app's own exe as the helper with Windows' admin prompt.

use crate::error::{AddonError, Result};
use crate::os::{AddonOs, Elevated, HelperAction};
use std::path::{Path, PathBuf};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, HANDLE, HWND, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_MULTI_SZ};

/// The mouse device class (GUID_DEVCLASS_MOUSE) - its UpperFilters list gets "rawaccel" from Raw Accel's installer.
const MOUSE_CLASS: &str = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e96f-e325-11ce-bfc1-08002be10318}";
const DEVICE: &str = r"\\.\rawaccel";

pub struct RealOs {
    /// the app's exe, started as the elevated helper
    pub exe: PathBuf,
}

impl RealOs {
    /// The helper is the running exe itself.
    pub fn new() -> Self {
        RealOs { exe: std::env::current_exe().unwrap_or_default() }
    }
}

impl Default for RealOs {
    fn default() -> Self {
        Self::new()
    }
}

/// The mouse class's UpperFilters (a REG_MULTI_SZ), read-only.
pub fn mouse_upper_filters() -> Vec<String> {
    let k: Vec<u16> = MOUSE_CLASS.encode_utf16().chain([0]).collect();
    let v: Vec<u16> = "UpperFilters".encode_utf16().chain([0]).collect();
    // the size first (a long list must not read as "no filters")
    let mut len = 0u32;
    // SAFETY: a size query (no buffer).
    let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, PCWSTR(k.as_ptr()), PCWSTR(v.as_ptr()), RRF_RT_REG_MULTI_SZ, None, None, Some(&mut len)) };
    if r.is_err() {
        return Vec::new();
    }
    let mut buf = vec![0u16; len as usize / 2 + 2];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: RegGetValueW writes at most `len` bytes into `buf`.
    let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, PCWSTR(k.as_ptr()), PCWSTR(v.as_ptr()), RRF_RT_REG_MULTI_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut len)) };
    if r.is_err() {
        return Vec::new();
    }
    let n = (len as usize / 2).min(buf.len());
    buf[..n].split(|&c| c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
}

impl AddonOs for RealOs {
    fn rawaccel_filter_set(&self) -> bool {
        mouse_upper_filters().iter().any(|f| f.eq_ignore_ascii_case("rawaccel"))
    }

    fn rawaccel_running(&self) -> bool {
        // Raw Accel's own way to open its device (access 0, no admin)
        match unsafe { CreateFileW(&HSTRING::from(DEVICE), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) } {
            Ok(h) => {
                let _ = unsafe { CloseHandle(h) };
                true
            }
            Err(_) => false,
        }
    }

    fn verify_signature(&self, file: &Path) -> Result<()> {
        verify_signature(file)
    }

    fn run_elevated(&self, action: HelperAction, folder: &Path, stop: &dyn Fn() -> bool) -> Elevated {
        run_elevated(&self.exe, action, folder, stop)
    }
}

/// Windows' Authenticode check of one file (WinVerifyTrust, generic verify v2, no UI, no network revocation check).
pub fn verify_signature(file: &Path) -> Result<()> {
    use windows::Win32::Security::WinTrust::{
        WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
        WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    };
    let path: Vec<u16> = file.as_os_str().encode_wide_z();
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
    // SAFETY: wd / fi / path live across both calls; the second call frees the state the first one made.
    let r = unsafe { WinVerifyTrust(HWND::default(), &mut action, (&mut wd as *mut WINTRUST_DATA).cast()) };
    wd.dwStateAction = WTD_STATEACTION_CLOSE;
    let _ = unsafe { WinVerifyTrust(HWND::default(), &mut action, (&mut wd as *mut WINTRUST_DATA).cast()) };
    if r == 0 {
        Ok(())
    } else {
        Err(AddonError::Verify(format!("Windows did not trust the driver\u{2019}s signature (0x{:08X})", r as u32)))
    }
}

trait WideZ {
    fn encode_wide_z(&self) -> Vec<u16>;
}
impl WideZ for std::ffi::OsStr {
    fn encode_wide_z(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().chain([0]).collect()
    }
}

/// Start `<exe> --addon-helper <action> "<folder>"` with Windows' admin prompt (hidden) and wait for it; its exit code is
/// its answer (`helper::Code`). `stop()` (the app quits) ends the wait - the prompt / the helper go on by themselves.
pub fn run_elevated(exe: &Path, action: HelperAction, folder: &Path, stop: &dyn Fn() -> bool) -> Elevated {
    // the prompt blocks ShellExecuteEx: it runs on its own thread, this one waits and can give up (app quit)
    let (tx, rx) = std::sync::mpsc::channel();
    let (exe, folder) = (exe.to_path_buf(), folder.to_path_buf());
    std::thread::spawn(move || {
        let _ = tx.send(launch_and_wait(&exe, action, &folder));
    });
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(r) => return r,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Elevated::Failed("The admin helper did not finish".into()),
            Err(_) if stop() => return Elevated::Failed("Stopped waiting for Windows' admin prompt".into()),
            Err(_) => {}
        }
    }
}

fn launch_and_wait(exe: &Path, action: HelperAction, folder: &Path) -> Elevated {
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    let verb: Vec<u16> = "runas".encode_utf16().chain([0]).collect();
    let file = exe.as_os_str().encode_wide_z();
    let params: Vec<u16> = format!("{} {} \"{}\"", crate::helper::ARG, action.name(), folder.display()).encode_utf16().chain([0]).collect();
    // ShellExecuteEx wants COM on its thread (a job's worker thread)
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok();
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    let started = unsafe { ShellExecuteExW(&mut sei) };
    if com {
        unsafe { CoUninitialize() };
    }
    if let Err(e) = started {
        if e.code() == ERROR_CANCELLED.to_hresult() {
            return Elevated::Declined;
        }
        return Elevated::Failed(format!("The admin helper did not start: {}", e.message()));
    }
    let h: HANDLE = sei.hProcess;
    if h.is_invalid() {
        return Elevated::Failed("The admin helper did not start".into());
    }
    let w = unsafe { WaitForSingleObject(h, INFINITE) };
    let mut code = 1u32;
    let _ = unsafe { GetExitCodeProcess(h, &mut code) };
    let _ = unsafe { CloseHandle(h) };
    if w != WAIT_OBJECT_0 {
        return Elevated::Failed("The admin helper did not finish".into());
    }
    if code == 0 {
        return Elevated::Done;
    }
    Elevated::Failed(crate::helper::message(action, code))
}
