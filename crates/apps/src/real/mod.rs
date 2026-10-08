//! The REAL Windows OS layer. Reads are safe anywhere. `run_uninstaller` / `remove_package` are only ever called by the menu after
//! the user confirms — the tests never call them on a real app (Order 006: "NEVER uninstall anything real").
//!
//! Windows APIs used: registry (Uninstall keys), Windows.Management.Deployment.PackageManager (Store apps: list, find, remove),
//! ShellExecuteExW (the app's own uninstaller with its own window; Windows shows the admin prompt when the uninstaller asks for
//! it), WaitForSingleObject + a Toolhelp32 process snapshot after each exit (to also wait for the copies uninstallers start).

pub mod win;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use windows::core::{HSTRING, PCWSTR};
use windows::Management::Deployment::PackageManager;
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
use windows::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessId, GetProcessTimes, OpenProcess, WaitForSingleObject, INFINITE, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE,
};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};

use crate::os::*;
use crate::UNINSTALL;
use win::{hr_err, Key};

#[derive(Clone, Copy, Debug, Default)]
pub struct RealOs;

/// ShellExecuteEx "open" on a program (+ arguments) or a URI, not waiting. COM is set up on the calling thread for the call
/// (ShellExecute needs it; the page calls this from its own thread).
fn shell_open(file: &str, args: Option<&str>) -> Result<(), OsError> {
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok();
    let file_w = HSTRING::from(file);
    let args_w = HSTRING::from(args.unwrap_or(""));
    let verb = HSTRING::from("open");
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file_w.as_ptr()),
        lpParameters: if args.is_some() { PCWSTR(args_w.as_ptr()) } else { PCWSTR::null() },
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    let r = unsafe { ShellExecuteExW(&mut info) }.map_err(|e| hr_err(&e));
    if com {
        unsafe { CoUninitialize() };
    }
    r
}

impl RealOs {
    pub fn new() -> Self {
        RealOs
    }
}

/// `C:\Windows` (GetWindowsDirectoryW) or `C:\Windows\System32` (GetSystemDirectoryW).
fn os_dir(system: bool) -> Option<PathBuf> {
    use windows::Win32::System::SystemInformation::{GetSystemDirectoryW, GetWindowsDirectoryW};
    let mut buf = [0u16; 260];
    let n = unsafe { if system { GetSystemDirectoryW(Some(&mut buf)) } else { GetWindowsDirectoryW(Some(&mut buf)) } } as usize;
    (n > 0 && n < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
}

/// `%LOCALAPPDATA%\Microsoft\WindowsApps` from the known-folder id (where Windows keeps app execution aliases, e.g. winget).
fn windows_apps_dir() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    let p = unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None) }.ok()?;
    let s = unsafe { p.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    s.map(|s| PathBuf::from(s).join(r"Microsoft\WindowsApps"))
}

/// A bare program name as a full path in a FIXED folder — never left to the search order, which also looks in our own folder
/// and the current folder: System32, then the Windows folder (`MsiExec.exe`, as Windows writes it into MSI UninstallStrings;
/// `RunDll32`, `msiexec` without ".exe" also get ".exe" / ".com" tried there, as ShellExecute would), then the app execution
/// aliases folder (`winget uninstall …` entries; an alias is a reparse point that only `symlink_metadata` can see). A bare name
/// found in none of them is refused. A program given with a folder is kept as it is.
fn full_program(prog: &str) -> Result<String, OsError> {
    if prog.contains(['\\', '/', ':']) {
        return Ok(prog.to_string());
    }
    let names: Vec<String> =
        if Path::new(prog).extension().is_some() { vec![prog.to_string()] } else { vec![prog.to_string(), format!("{prog}.exe"), format!("{prog}.com")] };
    let in_dir = |d: Option<PathBuf>, alias: bool| {
        let d = d?;
        names.iter().map(|n| d.join(n)).find(|p| if alias { std::fs::symlink_metadata(p).is_ok_and(|m| !m.is_dir()) } else { p.is_file() })
    };
    in_dir(os_dir(true), false)
        .or_else(|| in_dir(os_dir(false), false))
        .or_else(|| in_dir(windows_apps_dir(), true))
        .map(|p| p.display().to_string())
        .ok_or(OsError::NotFound)
}

/// The places an Uninstall entry can live: HKLM 64, HKLM 32 (`WOW6432Node`), HKCU (both views; the same key on current
/// Windows, duplicates are dropped by key name).
const PLACES: [(Hive, RegView); 4] =
    [(Hive::LocalMachine, RegView::Bits64), (Hive::LocalMachine, RegView::Bits32), (Hive::CurrentUser, RegView::Bits64), (Hive::CurrentUser, RegView::Bits32)];

fn filetime_u64(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

/// Program + arguments of a command line (quoted first part, or the shortest run of words that names an existing file — how
/// CreateProcessW resolves an unquoted path with spaces, Microsoft Learn; our own addition: a word ending in .exe ends it).
pub fn split_command(cmd: &str) -> Option<(String, String)> {
    let cmd = win::expand_env(cmd.trim());
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return None;
    }
    if let Some(rest) = cmd.strip_prefix('"') {
        let end = rest.find('"')?;
        let prog = &rest[..end];
        return (!prog.is_empty()).then(|| (prog.to_string(), rest[end + 1..].trim().to_string()));
    }
    let words: Vec<&str> = cmd.split(' ').collect();
    for i in 1..=words.len() {
        let cand = words[..i].join(" ");
        let args = words[i..].join(" ").trim().to_string();
        if Path::new(&cand).is_file() || cand.to_ascii_lowercase().ends_with(".exe") {
            return Some((cand, args));
        }
    }
    Some((words[0].to_string(), words[1..].join(" ").trim().to_string()))
}

/// Start `command` (window shown with `show`) and wait until it and every process it started have ended. Returns the first
/// process's exit code. Waiting blocks on process handles (no timer); after each exit a process snapshot finds the children
/// (e.g. the temp copy NSIS / Inno uninstallers start before the first process exits).
/// Known gaps: a child started by a process that itself ended between two of our wake-ups is not seen; a hand-over to an
/// already running program (e.g. `steam.exe steam://uninstall/<id>`) returns at once. Long-lived programs in [`DONT_WAIT_FOR`]
/// (browsers, Explorer) are not waited for.
pub fn run_and_wait(command: &str, show: i32) -> Result<u32, OsError> {
    let (prog, args) = split_command(command).ok_or(OsError::NotFound)?;
    let prog = full_program(&prog)?;
    let prog_w = HSTRING::from(prog.as_str());
    let args_w = HSTRING::from(args.as_str());
    let verb = HSTRING::from("open");
    let started = filetime_u64(unsafe { GetSystemTimeAsFileTime() });
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(prog_w.as_ptr()),
        lpParameters: if args.is_empty() { PCWSTR::null() } else { PCWSTR(args_w.as_ptr()) },
        nShow: show,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| hr_err(&e))?;
    let root = info.hProcess;
    if root.is_invalid() {
        // Handed to an already running process (no handle): nothing to wait for.
        return Ok(0);
    }
    let root_pid = unsafe { GetProcessId(root) };
    unsafe { WaitForSingleObject(root, INFINITE) };
    let mut code = 0u32;
    let _ = unsafe { GetExitCodeProcess(root, &mut code) };
    let _ = unsafe { CloseHandle(root) };

    let mut known: HashSet<u32> = HashSet::from([root_pid]);
    loop {
        let children = children_of(&known, started);
        if children.is_empty() {
            break;
        }
        for (pid, h) in children {
            known.insert(pid);
            if let Some(h) = h {
                unsafe { WaitForSingleObject(h, INFINITE) };
                let _ = unsafe { CloseHandle(h) };
            }
        }
    }
    Ok(code)
}

/// Programs an uninstaller may start that live on long after it (a "sorry to see you go" web page, a restarted Explorer):
/// never waited for, nor their children. Exe names, no case.
pub const DONT_WAIT_FOR: &[&str] = &[
    "explorer.exe",
    "msedge.exe",
    "msedgewebview2.exe",
    "chrome.exe",
    "firefox.exe",
    "opera.exe",
    "brave.exe",
    "vivaldi.exe",
    "iexplore.exe",
];

/// Processes whose parent is in `known`, started after `started`, not yet known: (pid, a wait handle if it could be opened).
fn children_of(known: &HashSet<u32>, started: u64) -> Vec<(u32, Option<HANDLE>)> {
    let mut out = Vec::new();
    let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else { return out };
    let mut all: Vec<(u32, u32)> = Vec::new(); // (pid, parent pid) — long-lived programs (DONT_WAIT_FOR) left out
    let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut ok = unsafe { Process32FirstW(snap, &mut e) }.is_ok();
    while ok {
        let end = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
        let exe = String::from_utf16_lossy(&e.szExeFile[..end]);
        if !DONT_WAIT_FOR.iter().any(|d| d.eq_ignore_ascii_case(&exe)) {
            all.push((e.th32ProcessID, e.th32ParentProcessID));
        }
        ok = unsafe { Process32NextW(snap, &mut e) }.is_ok();
    }
    let _ = unsafe { CloseHandle(snap) };
    // Children, grandchildren … in this snapshot.
    let mut family = known.clone();
    loop {
        let mut grew = false;
        for &(pid, parent) in &all {
            if family.contains(&parent) && !family.contains(&pid) {
                let h = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok();
                // A reused parent PID would make an older process look like a child: keep only processes born after the start.
                let born_after = h.is_some_and(|h| {
                    let (mut c, mut x, mut k, mut u) = Default::default();
                    unsafe { GetProcessTimes(h, &mut c, &mut x, &mut k, &mut u) }.is_ok() && filetime_u64(c) >= started
                });
                match h {
                    Some(h) if born_after => {
                        family.insert(pid);
                        out.push((pid, Some(h)));
                        grew = true;
                    }
                    Some(h) => {
                        let _ = unsafe { CloseHandle(h) };
                    }
                    None => {} // gone already, or not openable
                }
            }
        }
        if !grew {
            break;
        }
    }
    out
}

fn folder_size(path: &Path) -> Option<u64> {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    let mut first = true;
    while let Some(dir) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) if first => return None,
            Err(_) => continue,
        };
        first = false;
        for ent in rd.flatten() {
            let Ok(ft) = ent.file_type() else { continue };
            if ft.is_symlink() {
                continue; // junctions / links: not this app's own bytes, and could loop
            }
            if ft.is_dir() {
                stack.push(ent.path());
            } else if let Ok(m) = ent.metadata() {
                total += m.len();
            }
        }
    }
    Some(total)
}

fn read_package(p: &windows::ApplicationModel::Package) -> Option<RawPackage> {
    let id = p.Id().ok()?;
    let v = id.Version().ok()?;
    let sig = match p.SignatureKind().map(|s| s.0).unwrap_or(0) {
        1 => Signature::Developer,
        2 => Signature::Enterprise,
        3 => Signature::Store,
        4 => Signature::System,
        _ => Signature::None,
    };
    Some(RawPackage {
        full_name: id.FullName().ok()?.to_string(),
        family_name: id.FamilyName().ok()?.to_string(),
        name: id.Name().ok()?.to_string(),
        display_name: p.DisplayName().ok().map(|s| s.to_string()),
        publisher: p.PublisherDisplayName().ok().map(|s| s.to_string()).filter(|s| !s.is_empty()),
        version: format!("{}.{}.{}.{}", v.Major, v.Minor, v.Build, v.Revision),
        // DateTime.UniversalTime = 100 ns since 1601 (the FILETIME scale).
        installed: p.InstalledDate().ok().map(|d| d.UniversalTime).filter(|t| *t > 0).map(|t| t as u64),
        logo: p.Logo().ok().and_then(|u| u.AbsoluteUri().ok()).and_then(|s| file_uri_to_path(&s.to_string())).map(resolve_logo),
        installed_path: p.InstalledPath().ok().map(|s| PathBuf::from(s.to_string())).filter(|p| !p.as_os_str().is_empty()),
        is_framework: p.IsFramework().unwrap_or(false),
        is_resource: p.IsResourcePackage().unwrap_or(false),
        is_bundle: p.IsBundle().unwrap_or(false),
        is_optional: p.IsOptional().unwrap_or(false),
        signature: sig,
    })
}


/// Store logos are named without their scale (`Assets\StoreLogo.png`) while the file on disk carries one
/// (`StoreLogo.scale-100.png`). Returns the path as is when it exists, else a scaled file: normal (not high-contrast) first,
/// scale-100 first, then by name (so scale-125 before scale-200).
fn resolve_logo(p: PathBuf) -> PathBuf {
    if p.is_file() {
        return p;
    }
    let (Some(dir), Some(stem), Some(ext)) = (p.parent(), p.file_stem(), p.extension()) else { return p };
    let (stem, ext) = (stem.to_string_lossy().to_lowercase(), ext.to_string_lossy().to_lowercase());
    let Ok(rd) = std::fs::read_dir(dir) else { return p };
    let mut found: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|f| {
            let n = f.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            n.starts_with(&format!("{stem}.")) && n.ends_with(&format!(".{ext}"))
        })
        .collect();
    found.sort_by_key(|f| {
        let n = f.to_string_lossy().to_lowercase();
        (n.contains("contrast-"), !n.contains(".scale-100."), n)
    });
    found.into_iter().next().unwrap_or(p)
}
fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let b = uri.strip_prefix("file:///")?.as_bytes();
    let mut i = 0;
    let mut bytes = Vec::new();
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                bytes.push(v);
                i += 3;
                continue;
            }
        }
        bytes.push(b[i]);
        i += 1;
    }
    Some(PathBuf::from(String::from_utf8_lossy(&bytes).replace('/', "\\")))
}

impl AppsOs for RealOs {
    fn uninstall_entries(&self) -> Result<Vec<RawEntry>, OsError> {
        let mut out: Vec<RawEntry> = Vec::new();
        for (hive, view) in PLACES {
            let Some(k) = Key::open(win::root(hive), UNINSTALL, view)? else { continue };
            for name in k.subkeys()? {
                if hive == Hive::CurrentUser && out.iter().any(|e| e.hive == hive && e.key_name.eq_ignore_ascii_case(&name)) {
                    continue;
                }
                let Ok(Some(sub)) = Key::open(k.hkey(), &name, view) else { continue };
                let Ok(values) = sub.values() else { continue };
                out.push(RawEntry { hive, view, key_name: name, values: values.into_iter().collect(), last_write: sub.last_write() });
            }
        }
        Ok(out)
    }

    fn store_packages(&self) -> Result<Vec<RawPackage>, OsError> {
        let pm = PackageManager::new().map_err(|e| hr_err(&e))?;
        let pkgs = pm.FindPackagesByUserSecurityId(&HSTRING::new()).map_err(|e| hr_err(&e))?;
        Ok(pkgs.into_iter().filter_map(|p| read_package(&p)).collect())
    }

    fn entry_exists(&self, hive: Hive, view: RegView, key_name: &str) -> bool {
        matches!(Key::open(win::root(hive), &format!(r"{UNINSTALL}\{key_name}"), view), Ok(Some(_)))
    }

    fn package_installed(&self, full_name: &str) -> bool {
        let Ok(pm) = PackageManager::new() else { return true };
        pm.FindPackageByUserSecurityIdPackageFullName(&HSTRING::new(), &HSTRING::from(full_name)).is_ok_and(|p| p.Id().is_ok())
    }

    fn run_uninstaller(&self, command: &str) -> Result<u32, OsError> {
        run_and_wait(command, SW_SHOWNORMAL.0)
    }

    fn run_setup(&self, command: &str) -> Result<u32, OsError> {
        run_and_wait(command, SW_SHOWNORMAL.0)
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn open_folder(&self, dir: &Path) -> Result<(), OsError> {
        // never ShellExecute "open" on the path itself: a file there would run. Explorer, with the folder as its argument.
        if !dir.is_dir() {
            return Err(OsError::NotFound);
        }
        let arg = format!("\"{}\"", dir.display().to_string().trim_end_matches('\\'));
        // Explorer by its full path (C:\Windows\explorer.exe), never by bare name: the search order would also try our own folder.
        let explorer = os_dir(false).map(|d| d.join("explorer.exe")).filter(|p| p.is_file()).ok_or(OsError::NotFound)?;
        shell_open(&explorer.display().to_string(), Some(&arg))
    }

    fn open_settings(&self, uri: &str) -> Result<(), OsError> {
        if !uri.starts_with("ms-settings:") {
            return Err(OsError::NotFound);
        }
        shell_open(uri, None)
    }

    fn remove_package(&self, full_name: &str) -> Result<(), OsError> {
        let pm = PackageManager::new().map_err(|e| hr_err(&e))?;
        let op = pm.RemovePackageAsync(&HSTRING::from(full_name)).map_err(|e| hr_err(&e))?;
        let result = op.join().map_err(|e| hr_err(&e))?;
        match result.ExtendedErrorCode() {
            Ok(hr) if hr.is_err() => Err(OsError::Other {
                code: hr.0,
                message: result.ErrorText().map(|t| t.to_string()).unwrap_or_else(|_| hr.message()),
            }),
            _ => Ok(()),
        }
    }

    fn folder_size(&self, path: &Path) -> Option<u64> {
        folder_size(path)
    }

    fn expand_env(&self, s: &str) -> String {
        win::expand_env(s)
    }
}

/// Hidden window, for the runner's own test only.
pub const HIDDEN: i32 = SW_HIDE.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split() {
        assert_eq!(split_command(r#""C:\a b\u.exe" /S"#), Some((r"C:\a b\u.exe".into(), "/S".into())));
        assert_eq!(split_command("MsiExec.exe /X{1}"), Some(("MsiExec.exe".into(), "/X{1}".into())));
        assert_eq!(split_command(r"C:\Windows\System32\cmd.exe /c exit"), Some((r"C:\Windows\System32\cmd.exe".into(), "/c exit".into())));
        assert_eq!(split_command(""), None);
    }

    #[test]
    fn bare_programs_get_their_full_windows_path() {
        // read-only: only looks the names up, starts nothing
        let sys = os_dir(true).unwrap();
        let win = os_dir(false).unwrap();
        assert_eq!(full_program("MsiExec.exe").unwrap().to_ascii_lowercase(), sys.join("MsiExec.exe").display().to_string().to_ascii_lowercase());
        assert_eq!(full_program("explorer.exe").unwrap().to_ascii_lowercase(), win.join("explorer.exe").display().to_string().to_ascii_lowercase());
        assert_eq!(full_program(r"C:\x\u.exe").unwrap(), r"C:\x\u.exe");
        // names written without ".exe" (InstallShield's `RunDll32 …`, a hand-written `msiexec /x{…}`) - REVIEW_023 da8c30a
        assert_eq!(full_program("RunDll32").unwrap().to_ascii_lowercase(), sys.join("rundll32.exe").display().to_string().to_ascii_lowercase());
        assert_eq!(full_program("msiexec").unwrap().to_ascii_lowercase(), sys.join("msiexec.exe").display().to_string().to_ascii_lowercase());
        // an app execution alias (winget) - only where this PC has one
        if let Some(wa) = windows_apps_dir().filter(|d| std::fs::symlink_metadata(d.join("winget.exe")).is_ok()) {
            assert_eq!(full_program("winget").unwrap(), wa.join("winget.exe").display().to_string());
        }
        assert!(matches!(full_program("no-such-program-bu-test.exe"), Err(OsError::NotFound)));
    }

    #[test]
    fn uri() {
        assert_eq!(file_uri_to_path("file:///C:/A%20B/x.png"), Some(PathBuf::from(r"C:\A B\x.png")));
    }
}
