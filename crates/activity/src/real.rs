//! The REAL reads (all read-only): uptime, game launcher install folders, app names, the data folder, and which exe
//! owns a window.

use crate::os::{exe_stem, ActivityOs};
use std::path::PathBuf;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetFileVersionInfoSizeW, GetFileVersionInfoW, GetLogicalDrives, VerQueryValueW};
use windows::Win32::System::Registry::*;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, GetWindowThreadProcessId};

/// DRIVE_FIXED
const DRIVE_FIXED: u32 = 3;

#[derive(Debug, Default)]
pub struct RealOs;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A REG_SZ value, read-only.
fn reg_str(root: HKEY, key: &str, value: &str) -> Option<String> {
    let (k, v) = (wide(key), wide(value));
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: RegGetValueW writes at most `len` bytes into `buf`.
    let r = unsafe {
        RegGetValueW(root, PCWSTR(k.as_ptr()), PCWSTR(v.as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len))
    };
    if r.is_err() {
        return None;
    }
    let n = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..n.min(buf.len())]))
}

/// Sub-key names of a key, read-only.
fn reg_subkeys(root: HKEY, key: &str) -> Vec<String> {
    let k = wide(key);
    let mut h = HKEY::default();
    // SAFETY: KEY_READ only; closed below.
    if unsafe { RegOpenKeyExW(root, PCWSTR(k.as_ptr()), None, KEY_READ, &mut h) }.is_err() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for i in 0.. {
        let mut name = [0u16; 256];
        let mut n = name.len() as u32;
        if unsafe { RegEnumKeyExW(h, i, Some(PWSTR(name.as_mut_ptr())), &mut n, None, None, None, None) }.is_err() {
            break;
        }
        out.push(String::from_utf16_lossy(&name[..n as usize]));
    }
    let _ = unsafe { RegCloseKey(h) };
    out
}

fn fixed_drives() -> Vec<String> {
    // SAFETY: plain reads.
    let mask = unsafe { GetLogicalDrives() };
    (0..26u32)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| format!("{}:\\", (b'A' + i as u8) as char))
        .filter(|d| unsafe { GetDriveTypeW(PCWSTR(wide(d).as_ptr())) } == DRIVE_FIXED)
        .collect()
}

fn subdirs(dir: &str) -> Vec<String> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path().display().to_string())
        .collect()
}

/// `"path"  "D:\\SteamLibrary"` lines of Steam's libraryfolders.vdf.
pub fn steam_library_paths(vdf: &str) -> Vec<String> {
    vdf.lines()
        .filter_map(|l| {
            let t = l.trim();
            let rest = t.strip_prefix("\"path\"")?.trim();
            Some(rest.trim_matches('"').replace("\\\\", "\\"))
        })
        .collect()
}

/// An Epic manifest of something you play: `"bIsApplication": true` (Fortnite, Rocket League). Unreal Engine installs,
/// Fab / Quixel plugins and other engine content say `false` (seen in the manifests on the build PC) — never games.
pub fn epic_is_application(item: &str) -> bool {
    item.split("\"bIsApplication\"").nth(1).is_some_and(|r| r.trim_start().trim_start_matches(':').trim_start().starts_with("true"))
}

/// `"InstallLocation": "C:\\Program Files\\Epic Games\\Fortnite",` in an Epic manifest.
pub fn epic_install_location(item: &str) -> Option<String> {
    let i = item.find("\"InstallLocation\"")?;
    let rest = &item[i + "\"InstallLocation\"".len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].replace("\\\\", "\\"))
}

impl ActivityOs for RealOs {
    fn uptime_ms(&mut self) -> u64 {
        // SAFETY: plain read.
        unsafe { GetTickCount64() }
    }

    fn game_roots(&mut self) -> Vec<String> {
        let mut roots = Vec::new();
        // Steam: every library's steamapps\common
        if let Some(steam) = reg_str(HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath") {
            let steam = steam.replace('/', "\\");
            let mut libs = vec![steam.clone()];
            if let Ok(vdf) = std::fs::read_to_string(format!(r"{steam}\steamapps\libraryfolders.vdf")) {
                libs.extend(steam_library_paths(&vdf));
            }
            for l in libs {
                roots.push(format!(r"{l}\steamapps\common"));
            }
        }
        // Epic: each game's install folder from its manifest
        for e in std::fs::read_dir(r"C:\ProgramData\Epic\EpicGamesLauncher\Data\Manifests").into_iter().flatten().flatten() {
            if e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("item")) {
                if let Some(p) = std::fs::read_to_string(e.path()).ok().as_deref().filter(|t| epic_is_application(t)).and_then(epic_install_location) {
                    roots.push(p);
                }
            }
        }
        for d in fixed_drives() {
            // Riot: Riot Games\<game>, not the Riot Client itself
            for g in subdirs(&format!("{d}Riot Games")) {
                if !g.to_lowercase().ends_with("\\riot client") {
                    roots.push(g);
                }
            }
            // Xbox app / Game Pass
            roots.extend(subdirs(&format!("{d}XboxGames")));
        }
        // Ubisoft Connect + GOG Galaxy: install folders from their registry keys
        let ubi = r"SOFTWARE\WOW6432Node\Ubisoft\Launcher\Installs";
        for id in reg_subkeys(HKEY_LOCAL_MACHINE, ubi) {
            if let Some(p) = reg_str(HKEY_LOCAL_MACHINE, &format!(r"{ubi}\{id}"), "InstallDir") {
                roots.push(p);
            }
        }
        let gog = r"SOFTWARE\WOW6432Node\GOG.com\Games";
        for id in reg_subkeys(HKEY_LOCAL_MACHINE, gog) {
            if let Some(p) = reg_str(HKEY_LOCAL_MACHINE, &format!(r"{gog}\{id}"), "path") {
                roots.push(p);
            }
        }
        roots
    }

    fn app_name(&mut self, exe_path: &str) -> String {
        file_description(exe_path).unwrap_or_else(|| exe_stem(exe_path))
    }

    fn data_dir(&mut self) -> Option<PathBuf> {
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("BoylerUtilities").join("activity"))
    }
}

/// The exe's FileDescription ("Google Chrome"), like Windows' own lists.
pub fn file_description(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    unsafe {
        let w = wide(path);
        let size = GetFileVersionInfoSizeW(PCWSTR(w.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(w.as_ptr()), None, size, data.as_mut_ptr() as *mut _).ok()?;
        let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        let q = wide("\\VarFileInfo\\Translation");
        let mut tr = (0x0409u16, 0x04b0u16);
        if VerQueryValueW(data.as_ptr() as *const _, PCWSTR(q.as_ptr()), &mut p, &mut len).as_bool() && len >= 4 && !p.is_null() {
            let a = p as *const u16;
            tr = (*a, *a.add(1));
        }
        let k = wide(&format!("\\StringFileInfo\\{:04x}{:04x}\\FileDescription", tr.0, tr.1));
        if VerQueryValueW(data.as_ptr() as *const _, PCWSTR(k.as_ptr()), &mut p, &mut len).as_bool() && len > 1 && !p.is_null() {
            let s = std::slice::from_raw_parts(p as *const u16, len as usize);
            let s = String::from_utf16_lossy(s).trim_end_matches('\0').trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
        None
    }
}

/// The exe path of a process ("" when it can't be opened).
pub fn exe_of_pid(pid: u32) -> String {
    if pid == 0 {
        return String::new();
    }
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        let _ = CloseHandle(h);
        if ok {
            String::from_utf16_lossy(&buf[..n as usize])
        } else {
            String::new()
        }
    }
}

fn pid_of(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    // SAFETY: plain read.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

/// The exe that owns a window. Store apps run inside ApplicationFrameHost.exe: then the app's own child window names it.
pub fn exe_of_window(hwnd: HWND) -> String {
    let host = pid_of(hwnd);
    let path = exe_of_pid(host);
    if !path.to_lowercase().ends_with("\\applicationframehost.exe") {
        return path;
    }
    struct Find {
        host: u32,
        found: u32,
    }
    unsafe extern "system" fn each(child: HWND, l: LPARAM) -> windows::core::BOOL {
        // SAFETY: `l` is the &mut Find passed below, alive for the whole enumeration.
        let f = unsafe { &mut *(l.0 as *mut Find) };
        let p = pid_of(child);
        if p != 0 && p != f.host {
            f.found = p;
            return false.into();
        }
        true.into()
    }
    let mut f = Find { host, found: 0 };
    // SAFETY: the callback only reads window pids.
    let _ = unsafe { EnumChildWindows(Some(hwnd), Some(each), LPARAM(&mut f as *mut Find as isize)) };
    if f.found != 0 {
        exe_of_pid(f.found)
    } else {
        path
    }
}
