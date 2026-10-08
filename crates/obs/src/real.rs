//! The REAL Windows layer (obscfg.c / obsctl.c / obsstart.c / sound.c / app.c's other-copy parts). Used only by the
//! real app; tests use fake.rs. It never touches OBS unless the user asked for it (Apply, the start key, Connect).

use std::path::{Path, PathBuf};

use windows::core::{BOOL, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WAIT_OBJECT_0, WPARAM};
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, STGM_READ};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::Registry::{RegCloseKey, RegEnumValueW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_EXPAND_SZ, REG_SZ, REG_VALUE_TYPE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6432KEY, RRF_SUBKEY_WOW6464KEY};
use windows::Win32::System::Threading::{
    CreateProcessW, OpenMutexW, OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject, PROCESS_INFORMATION, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_CREATION_FLAGS, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE, STARTF_USESHOWWINDOW, STARTUPINFOW, SYNCHRONIZATION_SYNCHRONIZE, TerminateProcess,
};
use windows::Win32::UI::Shell::{FOLDERID_CommonStartup, FOLDERID_Startup, IShellLinkW, SHGetKnownFolderPath, ShellLink, KF_FLAG_DEFAULT};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowW, GetClassNameW, GetWindow, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, GW_OWNER, SW_SHOWMINNOACTIVE, WM_CLOSE, WM_COMMAND,
};

use crate::os::{ObsOs, Waiter};

/// ClipPing's own shortcut name + description: the shortcut it made is the feature's own one too (the same feature).
const LNK_NAME: &str = "OBS Studio (Notifications for OBS).lnk";
const LNK_DESC: &str = "Created by Notifications for OBS";
const OBS_EXE: &str = "obs64.exe";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
fn from_wide(b: &[u16]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf16_lossy(&b[..n])
}

/// "obs64" anywhere in a name or path
fn has_obs_name(s: &str) -> bool {
    s.to_ascii_lowercase().contains("obs64")
}

pub struct RealOs {
    /// keeps the two last sounds alive while they play (PlaySound reads the memory)
    play: [Option<Vec<u8>>; 2],
    flip: usize,
    com: bool,
}

impl Default for RealOs {
    fn default() -> Self {
        Self::new()
    }
}

impl RealOs {
    pub fn new() -> Self {
        RealOs { play: [None, None], flip: 0, com: false }
    }

    fn com(&mut self) {
        if !self.com {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
            }
            self.com = true;
        }
    }

    fn processes(f: &mut dyn FnMut(u32, &str) -> bool) {
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return };
            let mut pe = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            if Process32FirstW(snap, &mut pe).is_ok() {
                loop {
                    if f(pe.th32ProcessID, &from_wide(&pe.szExeFile)) {
                        break;
                    }
                    if Process32NextW(snap, &mut pe).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
        }
    }

    fn image_path(pid: u32) -> PathBuf {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return PathBuf::new() };
            let mut buf = [0u16; 1024];
            let mut n = buf.len() as u32;
            let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n);
            let _ = CloseHandle(h);
            if r.is_ok() {
                PathBuf::from(String::from_utf16_lossy(&buf[..n as usize]))
            } else {
                PathBuf::new()
            }
        }
    }

    fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
        unsafe {
            let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const _));
            s.map(PathBuf::from)
        }
    }

    fn startup_dir(&self) -> Option<PathBuf> {
        Self::known_folder(&FOLDERID_Startup).or_else(|| std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Microsoft\\Windows\\Start Menu\\Programs\\Startup")))
    }

    /// a shortcut's (target, description); never changes it
    fn lnk_read(&mut self, path: &Path) -> Option<(String, String)> {
        self.com();
        unsafe {
            let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
            let pf: IPersistFile = windows_core::Interface::cast(&sl).ok()?;
            pf.Load(&HSTRING::from(path.as_os_str()), STGM_READ).ok()?;
            let mut t = [0u16; 520];
            let mut d = [0u16; 520];
            let _ = sl.GetPath(&mut t, std::ptr::null_mut(), 0);
            let _ = sl.GetDescription(&mut d);
            Some((from_wide(&t), from_wide(&d)))
        }
    }

    fn is_ours(&mut self, dir: &Path, name: &str) -> bool {
        name.eq_ignore_ascii_case(LNK_NAME) && self.lnk_read(&dir.join(name)).is_some_and(|(_, d)| d == LNK_DESC)
    }

    /// a Startup folder that starts OBS through something the user made (not the feature's own shortcut)
    fn folder_starts_obs(&mut self, dir: &Path) -> bool {
        let Ok(rd) = std::fs::read_dir(dir) else { return false };
        for e in rd.flatten() {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(true) {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if self.is_ours(dir, &name) {
                continue;
            }
            if has_obs_name(&name) {
                return true;
            }
            if name.to_ascii_lowercase().ends_with(".lnk") && self.lnk_read(&e.path()).is_some_and(|(t, _)| has_obs_name(&t)) {
                return true;
            }
        }
        false
    }

    fn reg_sz(root: HKEY, key: &str, value: Option<&str>, flags: windows::Win32::System::Registry::REG_ROUTINE_FLAGS) -> Option<String> {
        unsafe {
            let mut buf = [0u16; 1024];
            let mut sz = (buf.len() * 2) as u32;
            let k = wide(key);
            let v = value.map(wide);
            let r = RegGetValueW(
                root,
                PCWSTR(k.as_ptr()),
                v.as_ref().map(|v| PCWSTR(v.as_ptr())).unwrap_or(PCWSTR::null()),
                RRF_RT_REG_SZ | flags,
                None,
                Some(buf.as_mut_ptr() as *mut _),
                Some(&mut sz),
            );
            r.is_ok().then(|| from_wide(&buf))
        }
    }

    /// every Run value (name, data) under HKCU
    fn run_values() -> Vec<(String, String)> {
        let mut out = Vec::new();
        unsafe {
            let mut k = HKEY::default();
            let path = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
            if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), None, KEY_READ, &mut k).is_err() {
                return out;
            }
            for i in 0.. {
                let mut name = [0u16; 256];
                let mut nl = name.len() as u32;
                let mut val = [0u16; 2048];
                let mut vl = (val.len() * 2 - 2) as u32;
                let mut ty = REG_VALUE_TYPE::default();
                if RegEnumValueW(k, i, Some(PWSTR(name.as_mut_ptr())), &mut nl, None, Some(&mut ty as *mut REG_VALUE_TYPE as *mut u32), Some(val.as_mut_ptr() as *mut u8), Some(&mut vl)).is_err() {
                    break;
                }
                if ty == REG_SZ || ty == REG_EXPAND_SZ {
                    out.push((from_wide(&name[..nl as usize]), from_wide(&val[..(vl as usize / 2).min(val.len())])));
                }
            }
            let _ = RegCloseKey(k);
        }
        out
    }

    fn steam_lib(lib: &str) -> Option<PathBuf> {
        let p = PathBuf::from(lib.replace('/', "\\")).join("steamapps\\common\\OBS Studio\\bin\\64bit\\obs64.exe");
        p.is_file().then_some(p)
    }
}

/// the "path" values of Steam's libraryfolders.vdf
pub fn vdf_paths(t: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = t;
    while let Some(i) = rest.find("\"path\"") {
        rest = &rest[i + 6..];
        let r = rest.trim_start_matches([' ', '\t']);
        let Some(r) = r.strip_prefix('"') else { continue };
        let mut v = String::new();
        let mut it = r.chars().peekable();
        while let Some(c) = it.next() {
            if c == '"' {
                break;
            }
            if c == '\\' && matches!(it.peek(), Some('\\') | Some('"')) {
                v.push(it.next().unwrap_or('\\'));
                continue;
            }
            v.push(c);
        }
        out.push(v);
    }
    out
}

struct FindWin {
    pid: u32,
    best: HWND,
    score: i32,
}

unsafe extern "system" fn find_cb(h: HWND, lp: LPARAM) -> BOOL {
    let f = &mut *(lp.0 as *mut FindWin);
    let mut pid = 0u32;
    GetWindowThreadProcessId(h, Some(&mut pid));
    if pid != f.pid || GetWindow(h, GW_OWNER).is_ok_and(|o| !o.is_invalid()) {
        return true.into();
    }
    let mut title = [0u16; 128];
    let mut cls = [0u16; 64];
    GetWindowTextW(h, &mut title);
    GetClassNameW(h, &mut cls);
    let t = from_wide(&title);
    if !t.starts_with("OBS") {
        return true.into();
    }
    let c = from_wide(&cls);
    let score = 1 + c.starts_with("Qt") as i32 + IsWindowVisible(h).as_bool() as i32;
    if score > f.score {
        f.best = h;
        f.score = score;
    }
    true.into()
}

impl ObsOs for RealOs {
    fn obs_dir(&self) -> PathBuf {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("obs-studio")).unwrap_or_default()
    }

    fn obs_running(&self) -> bool {
        unsafe {
            match OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &HSTRING::from("OBSStudioCore")) {
                Ok(h) => {
                    let _ = CloseHandle(h);
                    true
                }
                Err(_) => false,
            }
        }
    }

    fn find_obs(&self) -> Option<(u32, PathBuf)> {
        let mut pid = 0;
        Self::processes(&mut |p, name| {
            if name.eq_ignore_ascii_case(OBS_EXE) {
                pid = p;
                true
            } else {
                false
            }
        });
        (pid != 0).then(|| (pid, Self::image_path(pid)))
    }

    fn close_obs(&mut self, pid: u32) -> bool {
        let mut f = FindWin { pid, best: HWND::default(), score: 0 };
        unsafe {
            let _ = EnumWindows(Some(find_cb), LPARAM(&mut f as *mut FindWin as isize));
            if f.best.is_invalid() {
                return false;
            }
            PostMessageW(Some(f.best), WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok()
        }
    }

    fn wait_exit(&self, pid: u32, timeout_ms: u32) -> bool {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else { return true }; // already gone
            let r = WaitForSingleObject(h, timeout_ms);
            let _ = CloseHandle(h);
            r == WAIT_OBJECT_0
        }
    }

    fn waiter(&self) -> Waiter {
        Box::new(|p, t| RealOs::new().wait_exit(p, t))
    }

    fn start_obs(&mut self, path: &Path, args: Option<&str>, quiet: bool) -> bool {
        if !path.is_file() {
            return false;
        }
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut cmd = format!("\"{}\"", path.display());
        if let Some(a) = args {
            cmd.push(' ');
            cmd.push_str(a);
        }
        let mut cmdw = wide(&cmd);
        let app = wide(&path.to_string_lossy());
        let dirw = wide(&dir.to_string_lossy());
        let mut si = STARTUPINFOW { cb: std::mem::size_of::<STARTUPINFOW>() as u32, ..Default::default() };
        if quiet {
            si.dwFlags = STARTF_USESHOWWINDOW;
            si.wShowWindow = SW_SHOWMINNOACTIVE.0 as u16;
        }
        let mut pi = PROCESS_INFORMATION::default();
        unsafe {
            let ok = CreateProcessW(
                PCWSTR(app.as_ptr()),
                Some(PWSTR(cmdw.as_mut_ptr())),
                None,
                None,
                false,
                PROCESS_CREATION_FLAGS(0),
                None,
                PCWSTR(dirw.as_ptr()),
                &si,
                &mut pi,
            )
            .is_ok();
            if ok {
                let _ = CloseHandle(pi.hThread);
                let _ = CloseHandle(pi.hProcess);
            }
            ok
        }
    }

    fn default_path(&self) -> PathBuf {
        for f in [RRF_SUBKEY_WOW6464KEY, RRF_SUBKEY_WOW6432KEY] {
            if let Some(v) = Self::reg_sz(HKEY_LOCAL_MACHINE, "SOFTWARE\\OBS Studio", None, f) {
                let p = PathBuf::from(v).join("bin\\64bit\\obs64.exe");
                if p.is_file() {
                    return p;
                }
            }
        }
        PathBuf::from("C:\\Program Files\\obs-studio\\bin\\64bit\\obs64.exe")
    }

    fn program_files_path(&self) -> Option<PathBuf> {
        std::env::var_os("ProgramFiles").map(|p| PathBuf::from(p).join("obs-studio\\bin\\64bit\\obs64.exe"))
    }

    fn steam_obs(&self) -> Option<PathBuf> {
        let steam = Self::reg_sz(HKEY_CURRENT_USER, "Software\\Valve\\Steam", Some("SteamPath"), Default::default())
            .or_else(|| Self::reg_sz(HKEY_LOCAL_MACHINE, "SOFTWARE\\Valve\\Steam", Some("InstallPath"), RRF_SUBKEY_WOW6432KEY))?;
        if let Some(p) = Self::steam_lib(&steam) {
            return Some(p);
        }
        let t = std::fs::read(PathBuf::from(steam.replace('/', "\\")).join("steamapps\\libraryfolders.vdf")).ok()?;
        vdf_paths(&String::from_utf8_lossy(&t)).iter().find_map(|l| Self::steam_lib(l))
    }

    fn file_exists(&self, p: &Path) -> bool {
        p.is_file()
    }

    fn free_bytes(&self, dir: &Path) -> Option<u64> {
        let get = |d: &str| unsafe {
            let mut a = 0u64;
            GetDiskFreeSpaceExW(&HSTRING::from(d), Some(&mut a), None, None).ok().map(|_| a)
        };
        let s = dir.to_string_lossy().into_owned();
        get(&s).or_else(|| {
            let b = s.as_bytes();
            (b.len() >= 2 && b[1] == b':').then(|| format!("{}:\\", &s[..1])).and_then(|r| get(&r))
        })
    }

    fn user_autostart(&self) -> bool {
        let mut me = RealOs::new();
        if let Some(d) = self.startup_dir() {
            if me.folder_starts_obs(&d) {
                return true;
            }
        }
        if let Some(d) = Self::known_folder(&FOLDERID_CommonStartup) {
            if me.folder_starts_obs(&d) {
                return true;
            }
        }
        Self::run_values().iter().any(|(n, v)| !n.eq_ignore_ascii_case("NotificationsForOBS") && !n.eq_ignore_ascii_case("Boyler Utilities") && has_obs_name(v))
    }

    fn shortcut_exists(&self) -> bool {
        let Some(d) = self.startup_dir() else { return false };
        d.join(LNK_NAME).is_file() && RealOs::new().is_ours(&d, LNK_NAME)
    }

    fn shortcut_create(&mut self, obs: &Path) -> bool {
        let Some(d) = self.startup_dir() else { return false };
        let p = d.join(LNK_NAME);
        if !obs.is_file() {
            return false;
        }
        if p.exists() && !self.is_ours(&d, LNK_NAME) {
            return false; // a file with the shortcut's name that isn't the feature's: left alone
        }
        self.com();
        let dir = obs.parent().map(Path::to_path_buf).unwrap_or_default();
        unsafe {
            let Ok(sl) = CoCreateInstance::<_, IShellLinkW>(&ShellLink, None, CLSCTX_INPROC_SERVER) else { return false };
            let _ = sl.SetPath(&HSTRING::from(obs.as_os_str()));
            let _ = sl.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()));
            let _ = sl.SetArguments(&HSTRING::from(""));
            let _ = sl.SetDescription(&HSTRING::from(LNK_DESC));
            let Ok(pf) = windows_core::Interface::cast::<IPersistFile>(&sl) else { return false };
            pf.Save(&HSTRING::from(p.as_os_str()), true).is_ok()
        }
    }

    fn shortcut_delete(&mut self) -> bool {
        let Some(d) = self.startup_dir() else { return true };
        let p = d.join(LNK_NAME);
        if !p.exists() {
            return true;
        }
        if !self.is_ours(&d, LNK_NAME) {
            return true; // not the feature's: left alone
        }
        std::fs::remove_file(&p).is_ok()
    }

    fn random_password(&self, n: usize) -> Option<String> {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        let mut out = String::with_capacity(n);
        while out.len() < n {
            let mut r = [0u8; 64];
            unsafe {
                if BCryptGenRandom(None, &mut r, BCRYPT_USE_SYSTEM_PREFERRED_RNG).is_err() {
                    return None;
                }
            }
            for b in r {
                if out.len() < n && b < 248 {
                    out.push(A[(b % 62) as usize] as char); // 248 = 4 x 62: no bias
                }
            }
        }
        Some(out)
    }

    fn write_file(&mut self, p: &Path, text: &str) -> bool {
        let mut tmp = p.as_os_str().to_owned();
        tmp.push(".notifications-tmp");
        let tmp = PathBuf::from(tmp);
        let ok = (|| -> std::io::Result<()> {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()
        })()
        .is_ok();
        if !ok {
            let _ = std::fs::remove_file(&tmp);
            return false;
        }
        let moved = unsafe { MoveFileExW(&HSTRING::from(tmp.as_os_str()), &HSTRING::from(p.as_os_str()), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH).is_ok() };
        if !moved {
            let _ = std::fs::remove_file(&tmp);
        }
        moved
    }

    fn read_file(&self, p: &Path) -> Option<String> {
        std::fs::read(p).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    fn play(&mut self, wav: Vec<u8>) {
        // keep the previous buffer alive until this one replaces it
        self.flip ^= 1;
        self.play[self.flip] = Some(wav);
        if let Some(w) = &self.play[self.flip] {
            unsafe {
                let _ = PlaySoundW(PCWSTR(w.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
            }
        }
    }

    fn other_app(&self) -> Option<(u32, PathBuf)> {
        let mut pid = 0;
        Self::processes(&mut |p, name| {
            let n = name.to_ascii_lowercase();
            if (n.starts_with("notificationsforobs") || n == "clipping.exe") && n.ends_with(".exe") {
                pid = p;
                true
            } else {
                false
            }
        });
        (pid != 0).then(|| (pid, Self::image_path(pid)))
    }

    fn close_other_app(&mut self, pid: u32) -> bool {
        // closed exactly like its own tray menu's Quit (IDM_QUIT = 103), never forced
        for cls in ["NotificationsForOBSMain", "ClipPingMain"] {
            unsafe {
                let Ok(w) = FindWindowW(&HSTRING::from(cls), PCWSTR::null()) else { continue };
                let mut p = 0u32;
                GetWindowThreadProcessId(w, Some(&mut p));
                if p == pid {
                    return PostMessageW(Some(w), WM_COMMAND, WPARAM(103), LPARAM(0)).is_ok();
                }
            }
        }
        false
    }

    fn end_other_app(&mut self, pid: u32, exe: &Path) -> bool {
        // only that process, and only while it still runs from that exe (a pid can be reused)
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
            let mut buf = [0u16; 1024];
            let mut n = buf.len() as u32;
            let same = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok()
                && String::from_utf16_lossy(&buf[..n as usize]).eq_ignore_ascii_case(&exe.to_string_lossy());
            let ok = same && TerminateProcess(h, 0).is_ok();
            let _ = CloseHandle(h);
            ok
        }
    }

    fn other_app_run_entry(&self) -> Option<PathBuf> {
        let vals = Self::run_values();
        ["NotificationsForOBS", "ClipPing"].iter().find_map(|n| vals.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).and_then(|(_, v)| crate::settings::exe_of_command(v)))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn steam_library_paths() {
        let t = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}";
        assert_eq!(super::vdf_paths(t), vec!["C:\\Program Files (x86)\\Steam".to_string(), "D:\\SteamLibrary".to_string()]);
    }
}

impl Drop for RealOs {
    /// A sound still playing reads the memory this layer keeps: stop it before that memory goes.
    fn drop(&mut self) {
        if self.play.iter().any(|p| p.is_some()) {
            unsafe {
                let _ = PlaySoundW(PCWSTR::null(), None, windows::Win32::Media::Audio::SND_FLAGS(0));
            }
        }
    }
}
