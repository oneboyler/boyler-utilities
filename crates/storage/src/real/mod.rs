//! The real Windows implementation of [`StorageOs`]. Reads are safe to run on any PC; the three calls that change
//! something (`remove_file`, `remove_dir`, `empty_recycle_bin`) are only reached through [`crate::cleanup`].

mod disk;
mod fs;
pub mod wmi;

pub use disk::{parse_nvme_health_log, parse_nvme_protocol_data, parse_smart, parse_temperature_descriptor};

use crate::{DriveInfo, HealthRaw, KnownDirs, PhysicalDisk, RawEntry, RecycleBinInfo, Result, StorageError, StorageOs};
use disk::{from_wide, wide};
use std::io;
use std::path::{Path, PathBuf};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
    RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::{
    SHEmptyRecycleBinW, SHQueryRecycleBinW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHQUERYRBINFO,
};

/// The real OS. Cheap to create.
/// - [`RealOs::new`] — the app's one: reads and deletes for real.
/// - [`RealOs::read_only`] — reads are real, EVERY change is refused (deletes, emptying the bin, opening a disk
///   read+write for SATA SMART) — `examples/show`.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealOs {
    read_only: bool,
}

impl RealOs {
    pub fn new() -> Self {
        RealOs { read_only: false }
    }
    pub fn read_only() -> Self {
        RealOs { read_only: true }
    }
    fn refuse_io(&self, what: &str) -> io::Result<()> {
        if self.read_only {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("read-only OS layer refuses {what}")));
        }
        Ok(())
    }
}

/// A `REG_SZ` value, or `None`.
pub(crate) fn reg_string(root: HKEY, key: &str, value: &str) -> Option<String> {
    let k = wide(key);
    let v = wide(value);
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(root, PCWSTR(k.as_ptr()), PCWSTR(v.as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len))
    };
    r.is_ok().then(|| from_wide(&buf)).filter(|s| !s.is_empty())
}

fn reg_subkeys(root: HKEY, key: &str) -> Vec<String> {
    let k = wide(key);
    let mut h = HKEY::default();
    if unsafe { RegOpenKeyExW(root, PCWSTR(k.as_ptr()), None, KEY_READ, &mut h) }.is_err() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for i in 0.. {
        let mut name = [0u16; 256];
        let mut len = name.len() as u32;
        if unsafe { RegEnumKeyExW(h, i, Some(PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None) }.is_err() {
            break;
        }
        out.push(String::from_utf16_lossy(&name[..len as usize]));
    }
    unsafe {
        let _ = RegCloseKey(h);
    }
    out
}

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute())
}

pub(crate) fn elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut e = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut e as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && e.TokenIsElevated != 0
    }
}

impl StorageOs for RealOs {
    fn drives(&self) -> Result<Vec<DriveInfo>> {
        disk::drives()
    }
    fn read_dir(&self, path: &Path) -> io::Result<Vec<RawEntry>> {
        fs::read_dir(path)
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.refuse_io("a delete")?;
        fs::remove_file(path)
    }
    fn remove_dir(&self, path: &Path) -> io::Result<()> {
        self.refuse_io("a delete")?;
        fs::remove_dir(path)
    }
    fn recycle_bin(&self) -> Result<RecycleBinInfo> {
        let mut info = SHQUERYRBINFO { cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
        unsafe { SHQueryRecycleBinW(PCWSTR::null(), &mut info) }
            .map_err(|e| StorageError::Os { context: "SHQueryRecycleBinW".into(), code: e.code().0 as u32 })?;
        Ok(RecycleBinInfo { bytes: info.i64Size.max(0) as u64, items: info.i64NumItems.max(0) as u64 })
    }
    fn empty_recycle_bin(&self) -> Result<()> {
        self.refuse_io("emptying the recycle bin")?;
        match unsafe { SHEmptyRecycleBinW(None, PCWSTR::null(), SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND) } {
            Ok(()) => Ok(()),
            // E_UNEXPECTED is what an already-empty bin answers.
            Err(e) if e.code().0 as u32 == 0x8000_FFFF => Ok(()),
            Err(e) => Err(StorageError::Os { context: "SHEmptyRecycleBinW".into(), code: e.code().0 as u32 }),
        }
    }
    fn known_dirs(&self) -> KnownDirs {
        let windows = env_dir("SystemRoot").or_else(|| env_dir("windir"));
        let mut program_files = Vec::new();
        for v in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(p) = env_dir(v) {
                if !program_files.contains(&p) {
                    program_files.push(p);
                }
            }
        }
        KnownDirs {
            user_temp: Some(std::env::temp_dir()).filter(|p| p.is_absolute()).map(|p| {
                // GetTempPath ends with '\'; keep the plain folder.
                PathBuf::from(p.to_string_lossy().trim_end_matches('\\'))
            }),
            windows_temp: windows.as_ref().map(|w| w.join("Temp")),
            local_appdata: env_dir("LOCALAPPDATA"),
            local_low: env_dir("USERPROFILE").map(|u| u.join("AppData\\LocalLow")),
            program_data: env_dir("ProgramData"),
            windows,
            program_files,
            steam: reg_string(HKEY_CURRENT_USER, "Software\\Valve\\Steam", "SteamPath")
                .map(|s| PathBuf::from(s.replace('/', "\\"))),
        }
    }
    fn running_exe_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
            let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let mut ok = Process32FirstW(snap, &mut e).is_ok();
            while ok {
                out.push(from_wide(&e.szExeFile).to_lowercase());
                ok = Process32NextW(snap, &mut e).is_ok();
            }
            let _ = CloseHandle(snap);
        }
        out.sort();
        out.dedup();
        out
    }
    fn game_roots(&self) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        let k = self.known_dirs();
        // Steam: every library in libraryfolders.vdf (its whole steamapps folder).
        if let Some(steam) = &k.steam {
            let vdf = steam.join("steamapps\\libraryfolders.vdf");
            if let Ok(text) = std::fs::read_to_string(&vdf) {
                roots.extend(crate::classify::parse_steam_libraries(&text));
            } else {
                roots.push(steam.join("steamapps"));
            }
        }
        // Epic: InstallLocation of every manifest.
        if let Some(pd) = &k.program_data {
            let dir = pd.join("Epic\\EpicGamesLauncher\\Data\\Manifests");
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    if e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("item")) {
                        if let Some(p) = std::fs::read_to_string(e.path()).ok().and_then(|t| crate::classify::parse_epic_manifest(&t)) {
                            roots.push(p);
                        }
                    }
                }
            }
        }
        // GOG Galaxy: HKLM\SOFTWARE\WOW6432Node\GOG.com\Games\<id>\path.
        let gog = "SOFTWARE\\WOW6432Node\\GOG.com\\Games";
        for id in reg_subkeys(HKEY_LOCAL_MACHINE, gog) {
            if let Some(p) = reg_string(HKEY_LOCAL_MACHINE, &format!("{gog}\\{id}"), "path") {
                roots.push(PathBuf::from(p));
            }
        }
        // Riot Games and the Xbox app's XboxGames folder, at the root of every fixed drive.
        if let Ok(drives) = disk::drives() {
            for d in drives.iter().filter(|d| d.kind == crate::DriveKind::Fixed && d.state == crate::VolumeState::Ready) {
                for name in ["Riot Games", "XboxGames"] {
                    let p = PathBuf::from(format!("{}:\\{name}", d.letter));
                    if p.is_dir() {
                        roots.push(p);
                    }
                }
            }
        }
        roots.sort();
        roots.dedup();
        roots
    }
    fn is_elevated(&self) -> bool {
        elevated()
    }
    fn physical_disks(&self) -> Result<Vec<PhysicalDisk>> {
        Ok(disk::physical_disks())
    }
    fn disk_health(&self, disk_number: u32) -> Result<HealthRaw> {
        disk::disk_health(disk_number, elevated(), !self.read_only)
    }
}
