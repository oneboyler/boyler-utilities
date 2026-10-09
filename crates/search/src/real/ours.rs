//! Order 049 (the owner, Oct 9: "as for everything i dont think anyone wants to use it outside of the app, my friend said wtf
//! is the big magnifier on his pc now ... only usable inside the app? ... definitely not in the background"): Everything
//! ONLY inside Boyler Utilities.
//!
//! - Our own copy of voidtools' Everything.exe (MIT licence; the official portable zip, pinned SHA-256s) in
//!   `C:\Program Files\Boyler Utilities\Everything` - a folder only admins can write, because the SYSTEM service below runs
//!   that file (from the per-user app folder any program could swap it and become SYSTEM).
//! - Our own service `BoylerUtilitiesSearch` (`Everything.exe -svc` on its own pipe, `PIPE`): it reads the drives for our
//!   hidden Everything without admin. MANUAL start; its access list lets signed-in users start and stop it (`SDDL`), so
//!   the app starts it when the Search tab opens and stops it when the tab closes. No tray icon, no shortcut, no start at
//!   sign-in, no window. A user's own Everything (and its "Everything" service) is never touched.
//! - Set up by ONE admin prompt: the app's own exe started as `BoylerUtilities.exe --everything-helper install "<folder>"
//!   keep|tidy` (main.rs hands it over first). It checks the exe's SHA-256 with the file held against writes, writes
//!   THOSE bytes to the protected folder, makes / updates the service, and with `tidy` removes the Everything v1.0.0 put
//!   on the PC (`v100_rule`, A_049_02) - one fixed product code, nothing of the caller's choosing.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::error::{Result, SearchError};

/// voidtools' portable zip (the same release as the MSI v1.0.0 used) and its SHA-256.
pub const ZIP_HOST: &str = "www.voidtools.com";
pub const ZIP_PATH: &str = "/Everything-1.4.1.1032.x64.zip";
pub const ZIP_SHA256: &str = "698df475ec44e638f66f1b6a32d28fea613cec78d3b6310e6abe53431eeb940c";
/// `everything.exe` inside it (byte for byte the MSI's Everything.exe, checked Oct 9).
pub const EXE_SHA256: &str = "f191f756996a14a11e5445fa7103d302efd510cf2fbf920e6c0c8ed51d512e36";

/// Our service: its name, what Windows' Services list calls it, and its pipe (the full pipe path - a bare name makes no
/// pipe, tried Oct 9).
pub const SERVICE: &str = "BoylerUtilitiesSearch";
pub const SERVICE_DISPLAY: &str = "Boyler Utilities Search (Everything)";
pub const PIPE: &str = r"\\.\PIPE\BoylerUtilities Search";
/// Windows' default service access list + signed-in users (IU) may also start (RP) and stop (WP) it.
pub const SDDL: &str = "D:(A;;CCLCSWRPWPDTLOCRRC;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)(A;;CCLCSWRPWPLOCRRC;;;IU)(A;;CCLCSWLOCRRC;;;SU)";

/// The elevated helper's switch.
pub const HELPER_ARG: &str = "--everything-helper";
/// Setup's switch: the app (not elevated) downloads, checks and hands over to the helper, then ends.
pub const SETUP_ARG: &str = "--install-everything";

/// The Everything v1.0.0 installed for all of Windows (voidtools' MSI, 1.4.1.1032 x64) and the day v1.0.0 came out.
pub const V100_PRODUCT: &str = "{B5F500BB-4625-445D-A2E7-E7FE5E1E7E85}";
pub const V100_DAY: &str = "20261008";
/// Boyler Utilities' own "Installed apps" entry (Setup's AppId) and where the app keeps the day it was first installed.
pub const APP_UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{9C7C5E8C-178D-40CC-9BC5-54986753A645}_is1";
pub const APP_KEY: &str = r"Software\BoylerUtilities";
pub const FIRST_DAY_VALUE: &str = "FirstInstallDate";

/// The protected folder of our copy.
pub fn dir() -> Option<PathBuf> {
    // Windows' own answer, never an environment variable: the elevated helper's environment comes from the user's profile,
    // which any of their programs can write (review, Order 049)
    Some(program_files()?.join("Boyler Utilities").join("Everything"))
}

/// `C:\Program Files` (64-bit) from Windows' known folders.
fn program_files() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_ProgramFilesX64, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_ProgramFilesX64, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

/// Windows' System32 folder (GetSystemDirectoryW, never `%SystemRoot%`).
fn system32() -> Option<PathBuf> {
    use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
    let mut buf = [0u16; 260];
    let n = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
    (n > 0 && n < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
}

/// Our Everything.exe, when it is there.
pub fn exe() -> Option<PathBuf> {
    dir().map(|d| d.join("Everything.exe")).filter(|p| p.is_file())
}

/// The service's command line.
pub fn service_command(exe: &Path) -> String {
    format!("\"{}\" -svc -svc-pipe-name \"{}\"", exe.display(), PIPE)
}

/// Set up: our copy AND our service are there.
pub fn installed() -> bool {
    exe().is_some() && service_state().is_some()
}

// ------------------------------------------------------------------------------------------------- drives

/// One drive Search can cover: an NTFS fixed drive (Everything reads those through the service).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drive {
    pub letter: char,
    /// `\\?\Volume{...}` (no trailing backslash): how Everything.ini names a volume
    pub guid: String,
}

/// The Windows drive's letter (`%SystemDrive%`).
pub fn windows_drive() -> char {
    std::env::var("SystemDrive").ok().and_then(|s| s.chars().next()).map(|c| c.to_ascii_uppercase()).unwrap_or('C')
}

/// This PC's NTFS fixed drives, A to Z.
pub fn ntfs_drives() -> Vec<Drive> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW, GetVolumeNameForVolumeMountPointW};
    const DRIVE_FIXED: u32 = 3;
    let mask = unsafe { GetLogicalDrives() };
    let mut v = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = super::wide(&format!("{letter}:\\"));
        if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } != DRIVE_FIXED {
            continue;
        }
        let mut fs = [0u16; 32];
        if unsafe { GetVolumeInformationW(PCWSTR(root.as_ptr()), None, None, None, None, Some(&mut fs)) }.is_err() {
            continue;
        }
        let fs = String::from_utf16_lossy(&fs[..fs.iter().position(|&c| c == 0).unwrap_or(fs.len())]);
        if !fs.eq_ignore_ascii_case("NTFS") {
            continue;
        }
        let mut name = [0u16; 64];
        if unsafe { GetVolumeNameForVolumeMountPointW(PCWSTR(root.as_ptr()), &mut name) }.is_err() {
            continue;
        }
        let guid = String::from_utf16_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())]);
        v.push(Drive { letter, guid: guid.trim_end_matches('\\').to_string() });
    }
    v
}

// ------------------------------------------------------------------------------------------------- the service

/// Our service's state (Windows' SERVICE_* number: 1 stopped, 4 running ...), None = not there.
pub fn service_state() -> Option<u32> {
    use windows::Win32::System::Services::*;
    unsafe {
        let scm = OpenSCManagerW(None, None, SC_MANAGER_CONNECT).ok()?;
        let name = super::wide(SERVICE);
        let r = OpenServiceW(scm, windows::core::PCWSTR(name.as_ptr()), SERVICE_QUERY_STATUS).ok().and_then(|s| {
            let mut st = SERVICE_STATUS::default();
            let ok = QueryServiceStatus(s, &mut st).is_ok();
            let _ = CloseServiceHandle(s);
            ok.then_some(st.dwCurrentState.0)
        });
        let _ = CloseServiceHandle(scm);
        r
    }
}

/// Start our service (the Search tab opened) and wait until it runs (at most `wait`). No admin: its access list allows it.
pub fn service_start(wait: Duration) -> Result<()> {
    use windows::Win32::Foundation::ERROR_SERVICE_ALREADY_RUNNING;
    use windows::Win32::System::Services::*;
    let err = |what: &str, e: windows::core::Error| SearchError::Everything(format!("its service ({what}): {}", e.message()));
    unsafe {
        let scm = OpenSCManagerW(None, None, SC_MANAGER_CONNECT).map_err(|e| err("open", e))?;
        let name = super::wide(SERVICE);
        let s = match OpenServiceW(scm, windows::core::PCWSTR(name.as_ptr()), SERVICE_START | SERVICE_QUERY_STATUS) {
            Ok(s) => s,
            Err(e) => {
                let _ = CloseServiceHandle(scm);
                return Err(err("open", e));
            }
        };
        // (re)start it whenever it is stopped - a quick reopen may find it still stopping from the last close - until it runs
        // AND its pipe is really its own (a demand-start service's pipe name is free most of the time: any program could
        // make one of that name first - review, Order 049)
        let until = Instant::now() + wait;
        let r = loop {
            let mut st = SERVICE_STATUS::default();
            let state = if QueryServiceStatus(s, &mut st).is_ok() { st.dwCurrentState } else { SERVICE_STOPPED };
            if state == SERVICE_STOPPED {
                if let Err(e) = StartServiceW(s, None) {
                    if e.code() != ERROR_SERVICE_ALREADY_RUNNING.to_hresult() {
                        break Err(err("start", e));
                    }
                }
            } else if state == SERVICE_RUNNING {
                match (service_pid(s), pipe_server_pid(PIPE)) {
                    (Some(a), Some(b)) if a == b => break Ok(()),
                    (Some(_), Some(_)) => break Err(SearchError::Everything("another program holds its pipe".into())),
                    _ => {}
                }
            }
            if Instant::now() > until {
                break Err(SearchError::Everything("its service did not start in time".into()));
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let _ = CloseServiceHandle(s);
        let _ = CloseServiceHandle(scm);
        r
    }
}

/// A running service's process id.
fn service_pid(s: windows::Win32::System::Services::SC_HANDLE) -> Option<u32> {
    use windows::Win32::System::Services::*;
    let mut p = SERVICE_STATUS_PROCESS::default();
    let mut need = 0u32;
    let buf = unsafe { std::slice::from_raw_parts_mut(&mut p as *mut _ as *mut u8, std::mem::size_of::<SERVICE_STATUS_PROCESS>()) };
    unsafe { QueryServiceStatusEx(s, SC_STATUS_PROCESS_INFO, Some(buf), &mut need) }.ok()?;
    (p.dwProcessId != 0).then_some(p.dwProcessId)
}

/// The process that serves a named pipe (connects as a client for a moment). None = no such pipe (yet).
pub fn pipe_server_pid(pipe: &str) -> Option<u32> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE};
    use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_NONE, OPEN_EXISTING};
    use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
    let name = super::wide(pipe);
    unsafe {
        let h = CreateFileW(PCWSTR(name.as_ptr()), (GENERIC_READ | GENERIC_WRITE).0, FILE_SHARE_NONE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None).ok()?;
        let mut pid = 0u32;
        let ok = GetNamedPipeServerProcessId(h, &mut pid).is_ok();
        let _ = CloseHandle(h);
        (ok && pid != 0).then_some(pid)
    }
}

/// Is a hidden Everything of ours running in ANOTHER user's session (one service serves the whole PC: it must not be
/// stopped under them - review, Order 049)? Windows' process list with sessions needs no admin.
fn others_use_it() -> bool {
    use windows::Win32::System::RemoteDesktop::{WTSEnumerateProcessesW, WTSFreeMemory, WTS_CURRENT_SERVER_HANDLE, WTS_PROCESS_INFOW};
    use windows::Win32::System::Threading::GetCurrentProcessId;
    let mut mine = 0u32;
    unsafe {
        let _ = windows::Win32::System::RemoteDesktop::ProcessIdToSessionId(GetCurrentProcessId(), &mut mine);
        let mut list: *mut WTS_PROCESS_INFOW = std::ptr::null_mut();
        let mut n = 0u32;
        if WTSEnumerateProcessesW(Some(WTS_CURRENT_SERVER_HANDLE), 0, 1, &mut list, &mut n).is_err() || list.is_null() {
            return false;
        }
        let items = std::slice::from_raw_parts(list, n as usize);
        let found = items.iter().any(|p| p.SessionId != 0 && p.SessionId != mine && p.pProcessName.to_string().map(|s| s.eq_ignore_ascii_case("Everything.exe")).unwrap_or(false));
        WTSFreeMemory(list as *mut _);
        found
    }
}

/// Stop our service (the Search tab closed / the app quits) - unless another signed-in user's Search uses it. Quiet when it
/// is not there or already stopped.
pub fn service_stop() {
    if others_use_it() {
        return;
    }
    force_stop();
}

/// Stop it, whoever uses it (the elevated helper replacing / removing our copy).
fn force_stop() {
    use windows::Win32::System::Services::*;
    unsafe {
        let Ok(scm) = OpenSCManagerW(None, None, SC_MANAGER_CONNECT) else { return };
        let name = super::wide(SERVICE);
        if let Ok(s) = OpenServiceW(scm, windows::core::PCWSTR(name.as_ptr()), SERVICE_STOP | SERVICE_QUERY_STATUS) {
            let mut st = SERVICE_STATUS::default();
            let _ = ControlService(s, SERVICE_CONTROL_STOP, &mut st);
            let _ = CloseServiceHandle(s);
        }
        let _ = CloseServiceHandle(scm);
    }
}

// ------------------------------------------------------------------------------------------------- v1.0.0's Everything

/// A_049_02 (option 1, the same-day rule): the Everything MSI is v1.0.0's when it was installed on or after v1.0.0's day
/// AND on the day Boyler Utilities was first installed. Dates are Windows' `YYYYMMDD`.
pub fn v100_rule(msi_day: Option<&str>, app_first_day: Option<&str>) -> bool {
    let ok = |d: &str| d.len() == 8 && d.bytes().all(|b| b.is_ascii_digit());
    match (msi_day, app_first_day) {
        (Some(m), Some(a)) if ok(m) && ok(a) => m >= V100_DAY && m == a,
        _ => false,
    }
}

fn reg_string(root: windows::Win32::System::Registry::HKEY, key: &str, value: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::*;
    let (k, v) = (super::wide(key), super::wide(value));
    let mut buf = [0u16; 64];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(root, PCWSTR(k.as_ptr()), PCWSTR(v.as_ptr()), RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) };
    if r.is_err() {
        return None;
    }
    let n = (len as usize / 2).saturating_sub(1).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]).trim_end_matches('\0').to_string())
}

/// The day the Everything MSI was installed (None = it is not installed).
pub fn v100_msi_day() -> Option<String> {
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
    reg_string(HKEY_LOCAL_MACHINE, &format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{V100_PRODUCT}"), "InstallDate")
}

/// The day Boyler Utilities was FIRST installed: kept by the app (`remember_first_day`) - Setup rewrites its own entry's
/// date on every install.
pub fn app_first_day() -> Option<String> {
    use windows::Win32::System::Registry::HKEY_CURRENT_USER;
    reg_string(HKEY_CURRENT_USER, APP_KEY, FIRST_DAY_VALUE).or_else(|| reg_string(HKEY_CURRENT_USER, APP_UNINSTALL_KEY, "InstallDate"))
}

/// At app start: keep the install day of the first install (taken from Setup's entry the first time; never changed again).
pub fn remember_first_day() {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::*;
    if reg_string(HKEY_CURRENT_USER, APP_KEY, FIRST_DAY_VALUE).is_some() {
        return;
    }
    let Some(day) = reg_string(HKEY_CURRENT_USER, APP_UNINSTALL_KEY, "InstallDate") else { return };
    let (k, v, d) = (super::wide(APP_KEY), super::wide(FIRST_DAY_VALUE), super::wide(&day));
    unsafe {
        let mut h = HKEY::default();
        if RegCreateKeyExW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), None, None, REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, None, &mut h, None).is_ok() {
            let bytes = std::slice::from_raw_parts(d.as_ptr() as *const u8, d.len() * 2);
            let _ = RegSetValueExW(h, PCWSTR(v.as_ptr()), None, REG_SZ, Some(bytes));
            let _ = RegCloseKey(h);
        }
    }
}

/// Is v1.0.0's Everything still on this PC (the tidy-up is offered)?
pub fn v100_present() -> bool {
    v100_rule(v100_msi_day().as_deref(), app_first_day().as_deref())
}

// ------------------------------------------------------------------------------------------------- install (normal side)

/// Download the zip, check it and the exe in it, hand the exe to the elevated helper (ONE admin prompt), which sets up our
/// copy + service and, with `tidy`, removes v1.0.0's Everything. Blocks.
pub fn install(tidy: bool) -> Result<()> {
    let zip = super::host::download(ZIP_HOST, ZIP_PATH)?;
    if super::host::sha256_hex(&zip)? != ZIP_SHA256 {
        return Err(SearchError::Install("the download did not match voidtools' file - nothing was installed".into()));
    }
    let entries = bu_addons::zip::read(&zip, 16 * 1024 * 1024).map_err(|e| SearchError::Install(e.to_string()))?;
    let exe = entries.into_iter().find(|e| !e.dir && e.name.eq_ignore_ascii_case("everything.exe")).ok_or_else(|| SearchError::Install("Everything.exe was not in voidtools' file".into()))?;
    if super::host::sha256_hex(&exe.data)? != EXE_SHA256 {
        return Err(SearchError::Install("Everything.exe did not match voidtools' file - nothing was installed".into()));
    }
    // a fresh folder per install; the helper only READS from it (and checks the file again, held against writes)
    let stage = std::env::temp_dir().join("BoylerUtilities").join(format!("everything-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| SearchError::Install(format!("could not save it: {e}")))?;
    let r = (|| {
        std::fs::write(stage.join("Everything.exe"), &exe.data).map_err(|e| SearchError::Install(format!("could not save it: {e}")))?;
        let me = std::env::current_exe().map_err(|e| SearchError::Install(format!("the app's own file: {e}")))?;
        let params = format!("{HELPER_ARG} install \"{}\" {}", stage.display(), if tidy { "tidy" } else { "keep" });
        match super::host::run_elevated(&me, &params)? {
            0 => Ok(()),
            c => Err(SearchError::Install(helper_message(c))),
        }
    })();
    let _ = std::fs::remove_dir_all(&stage);
    r?;
    if !installed() {
        return Err(SearchError::Install("the set-up finished but Everything is not where it should be".into()));
    }
    Ok(())
}

/// The helper's exit codes.
pub mod code {
    pub const DONE: u32 = 0;
    pub const BAD_ARGS: u32 = 2;
    pub const NOT_OFFICIAL: u32 = 3;
    pub const COPY: u32 = 4;
    pub const SERVICE: u32 = 5;
    pub const TIDY: u32 = 6;
}

/// The line the page shows for a helper's exit code.
pub fn helper_message(c: u32) -> String {
    match c {
        code::BAD_ARGS => "the admin helper was started wrongly".into(),
        code::NOT_OFFICIAL => "Everything.exe was not voidtools' file - nothing was installed".into(),
        code::COPY => "could not copy Everything into Program Files".into(),
        code::SERVICE => "could not set up its service".into(),
        code::TIDY => "set up, but the old Everything could not be removed".into(),
        c => format!("the admin helper stopped (code {c})"),
    }
}

// ------------------------------------------------------------------------------------------------- the elevated helper

/// main.rs: `--everything-helper install "<folder>" keep|tidy` -> do it, return the exit code; anything else -> None.
pub fn run_if_requested(args: &[String]) -> Option<u32> {
    let i = args.iter().position(|a| a == HELPER_ARG)?;
    let rest = &args[i + 1..];
    Some(match rest {
        [what, folder, mode] if what == "install" && (mode == "keep" || mode == "tidy") => helper_install(Path::new(folder), mode == "tidy"),
        [what] if what == "remove" => helper_remove(),
        _ => code::BAD_ARGS,
    })
}

/// The app is uninstalled (Setup's uninstaller runs `--uninstall-everything`): our service and our copy go with it (one
/// admin prompt). Nothing to do = no prompt.
pub fn uninstall() -> Result<()> {
    if exe().is_none() && service_state().is_none() {
        return Ok(());
    }
    let me = std::env::current_exe().map_err(|e| SearchError::Install(format!("the app's own file: {e}")))?;
    match super::host::run_elevated(&me, &format!("{HELPER_ARG} remove"))? {
        0 => Ok(()),
        c => Err(SearchError::Install(helper_message(c))),
    }
}

/// Setup's uninstall switch.
pub const REMOVE_ARG: &str = "--uninstall-everything";

/// Stop and delete our service, delete our copy's folder (only ours: the fixed name and the fixed folder).
fn helper_remove() -> u32 {
    use windows::core::PCWSTR;
    use windows::Win32::System::Services::*;
    force_stop();
    unsafe {
        if let Ok(scm) = OpenSCManagerW(None, None, SC_MANAGER_CONNECT) {
            let name = super::wide(SERVICE);
            if let Ok(s) = OpenServiceW(scm, PCWSTR(name.as_ptr()), windows::Win32::System::Services::SERVICE_ALL_ACCESS) {
                let _ = DeleteService(s);
                let _ = CloseServiceHandle(s);
            }
            let _ = CloseServiceHandle(scm);
        }
    }
    // the service process may take a moment to let go of the file
    let until = Instant::now() + Duration::from_secs(5);
    if let Some(d) = dir() {
        while d.exists() && std::fs::remove_dir_all(&d).is_err() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(200));
        }
        if let Some(parent) = d.parent() {
            // "Boyler Utilities" in Program Files: only when nothing else is in it
            let _ = std::fs::remove_dir(parent);
        }
    }
    if service_state().is_some() {
        return code::SERVICE;
    }
    code::DONE
}

fn helper_install(folder: &Path, tidy: bool) -> u32 {
    use std::io::Read;
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;
    // read the exe held against writes and check it: only THOSE bytes are written anywhere
    let mut data = Vec::new();
    let held = std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(folder.join("Everything.exe")).and_then(|mut f| f.read_to_end(&mut data));
    if held.is_err() || super::host::sha256_hex(&data).ok().as_deref() != Some(EXE_SHA256) {
        return code::NOT_OFFICIAL;
    }
    let Some(dest_dir) = dir() else { return code::COPY };
    let dest = dest_dir.join("Everything.exe");
    // our service may run the old copy: stop it first (the file can't be replaced while it runs)
    let same = std::fs::read(&dest).ok().and_then(|b| super::host::sha256_hex(&b).ok()).as_deref() == Some(EXE_SHA256);
    if !same {
        force_stop();
        let tmp = dest_dir.join("Everything.exe.new");
        let mut ok = std::fs::create_dir_all(&dest_dir).is_ok() && std::fs::write(&tmp, &data).is_ok();
        // the stopped service (or a hidden instance) may hold the old file a moment longer: up to 10 s
        let until = Instant::now() + Duration::from_secs(10);
        while ok && std::fs::rename(&tmp, &dest).is_err() {
            if Instant::now() > until {
                ok = false;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        if !ok {
            let _ = std::fs::remove_file(&tmp);
            return code::COPY;
        }
    }
    if make_service(&dest).is_err() {
        return code::SERVICE;
    }
    // the rule again here (the same user's registry; another admin account's has no first day: then v1.0.0's day only)
    let msi = v100_msi_day();
    let ours_rule = match app_first_day() {
        Some(_) => v100_present(),
        None => msi.as_deref().is_some_and(|d| d.len() == 8 && d >= V100_DAY),
    };
    if tidy && ours_rule && !uninstall_v100() {
        return code::TIDY;
    }
    code::DONE
}

/// Create (or update) our service: manual start, our copy's command line, the access list that lets users start/stop it.
fn make_service(exe: &Path) -> windows::core::Result<()> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    use windows::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};
    use windows::Win32::System::Services::*;
    let (name, display, cmd, sddl) = (super::wide(SERVICE), super::wide(SERVICE_DISPLAY), super::wide(&service_command(exe)), super::wide(SDDL));
    unsafe {
        let scm = OpenSCManagerW(None, None, SC_MANAGER_ALL_ACCESS)?;
        let svc = match OpenServiceW(scm, PCWSTR(name.as_ptr()), SERVICE_ALL_ACCESS) {
            Ok(s) => {
                let r = ChangeServiceConfigW(s, SERVICE_WIN32_OWN_PROCESS, SERVICE_DEMAND_START, SERVICE_ERROR_IGNORE, PCWSTR(cmd.as_ptr()), None, None, None, None, None, PCWSTR(display.as_ptr()));
                r.map(|_| s)
            }
            Err(_) => CreateServiceW(
                scm,
                PCWSTR(name.as_ptr()),
                PCWSTR(display.as_ptr()),
                SERVICE_ALL_ACCESS,
                SERVICE_WIN32_OWN_PROCESS,
                SERVICE_DEMAND_START,
                SERVICE_ERROR_IGNORE,
                PCWSTR(cmd.as_ptr()),
                None,
                None,
                None,
                None,
                None,
            ),
        };
        let svc = match svc {
            Ok(s) => s,
            Err(e) => {
                let _ = CloseServiceHandle(scm);
                return Err(e);
            }
        };
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let mut r = ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()), SDDL_REVISION_1, &mut sd, None);
        if r.is_ok() {
            r = SetServiceObjectSecurity(svc, DACL_SECURITY_INFORMATION, sd);
            let _ = LocalFree(Some(HLOCAL(sd.0)));
        }
        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);
        r
    }
}

/// Remove v1.0.0's Everything: Windows' own msiexec, the one fixed product code, silently. True = gone.
fn uninstall_v100() -> bool {
    let Some(sys) = system32() else { return false };
    let msiexec = sys.join("msiexec.exe");
    match std::process::Command::new(msiexec).args(["/x", V100_PRODUCT, "/qn", "/norestart"]).status() {
        Ok(s) => matches!(s.code(), Some(0) | Some(3010) | Some(1605)),
        Err(_) => false,
    }
}
