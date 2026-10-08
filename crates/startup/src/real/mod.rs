//! The REAL Windows OS layer. Reads are safe to run anywhere; the write methods are only ever called by the menu on the user's
//! click (tests use the fake).
//!
//! Windows APIs used: registry (RegOpenKeyExW / RegEnumValueW / RegSetValueExW), SHGetKnownFolderPath + IShellLinkW (Startup
//! folders), Task Scheduler 2.0 COM (ITaskService), the Service Control Manager (EnumServicesStatusExW / QueryServiceConfigW /
//! ChangeServiceConfigW / ChangeServiceConfig2W), PackageManager (Store app names), GetFileVersionInfoW (publisher).

pub mod win;

use std::path::{Path, PathBuf};

use windows::core::{Interface, BSTR, HSTRING, PCWSTR, PWSTR};
use windows::Management::Deployment::PackageManager;
use windows::Win32::Foundation::{LocalFree, HANDLE, HLOCAL, VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ};
use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, REG_BINARY, REG_DWORD};
use windows::Win32::System::Services::*;
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
use windows::Win32::System::TaskScheduler::*;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Shell::{
    FOLDERID_CommonStartup, FOLDERID_Startup, IShellLinkW, SHGetKnownFolderPath, SHLoadIndirectString, ShellLink, KF_FLAG_DEFAULT,
};

use crate::os::*;
use win::{hr_err, reg_dword, reg_text, Com, Key};

const STORE_STATE_KEY: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\SystemAppData";
/// The app's own memory of services it turned from Automatic to Manual (so they stay listed). Written only on the user's click.
pub const MEMORY_KEY: &str = r"Software\BoylerUtilities\Startup\ServicesTurnedOff";

/// The real OS layer. `RealOs::new()` reads and writes the real places.
#[derive(Clone, Debug, Default)]
pub struct RealOs {
    scratch: Option<String>,
}

impl RealOs {
    pub fn new() -> Self {
        RealOs { scratch: None }
    }

    /// TESTS ONLY (boss answer A_006_01): every registry read and write goes to `HKCU\<prefix>\HKCU|HKLM\<the real path>`
    /// instead of the real place, so the real write code can be proven without touching the user's settings. The prefix must
    /// lie under [`win::SCRATCH_ROOT`] (else `Err`). Tasks, services and folders are still READ for real; task and service
    /// WRITES are refused in scratch mode, so a scratch `RealOs` can never change a real task or service.
    pub fn scratch(prefix: &str) -> Result<Self, OsError> {
        let p = prefix.trim_matches('\\');
        win::check_scratch_path(p)?;
        Ok(RealOs { scratch: Some(p.to_string()) })
    }

    fn refuse_in_scratch(&self) -> Result<(), OsError> {
        match self.scratch {
            Some(_) => Err(OsError::Other { code: 0, message: "scratch mode: real task / service writes are refused".into() }),
            None => Ok(()),
        }
    }

    /// Where a registry path really is: (root, path).
    fn reg(&self, hive: Hive, path: &str) -> (HKEY, String) {
        match &self.scratch {
            None => (win::root(hive), path.to_string()),
            Some(p) => {
                let h = if hive == Hive::CurrentUser { "HKCU" } else { "HKLM" };
                (HKEY_CURRENT_USER, format!(r"{p}\{h}\{path}"))
            }
        }
    }
}

fn var_i4(i: i32) -> VARIANT {
    let mut v = VARIANT::default();
    unsafe {
        let inner = &mut *v.Anonymous.Anonymous;
        inner.vt = VT_I4;
        inner.Anonymous.lVal = i;
    }
    v
}

fn task_service() -> Result<ITaskService, OsError> {
    unsafe {
        let svc: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER).map_err(|e| hr_err(&e))?;
        let empty = VARIANT::default();
        svc.Connect(&empty, &empty, &empty, &empty).map_err(|e| hr_err(&e))?;
        Ok(svc)
    }
}

fn collect_tasks(folder: &ITaskFolder, out: &mut Vec<RawTask>) {
    unsafe {
        if let Ok(tasks) = folder.GetTasks(TASK_ENUM_HIDDEN.0) {
            let n = tasks.Count().unwrap_or(0);
            for i in 1..=n {
                let Ok(t) = tasks.get_Item(&var_i4(i)) else { continue };
                if let Some(raw) = read_task(&t) {
                    out.push(raw);
                }
            }
        }
        if let Ok(subs) = folder.GetFolders(0) {
            let n = subs.Count().unwrap_or(0);
            for i in 1..=n {
                if let Ok(f) = subs.get_Item(&var_i4(i)) {
                    collect_tasks(&f, out);
                }
            }
        }
    }
}

fn read_task(t: &IRegisteredTask) -> Option<RawTask> {
    unsafe {
        let def = t.Definition().ok()?;
        let trig = def.Triggers().ok()?;
        let mut count = 0i32;
        trig.Count(&mut count).ok()?;
        let mut triggers = Vec::new();
        for i in 1..=count {
            let Ok(tr) = trig.get_Item(i) else { continue };
            let mut ty = TASK_TRIGGER_TYPE2::default();
            if tr.Type(&mut ty).is_ok() {
                if ty == TASK_TRIGGER_LOGON {
                    triggers.push(TaskTrigger::Logon);
                } else if ty == TASK_TRIGGER_BOOT {
                    triggers.push(TaskTrigger::Boot);
                }
            }
        }
        if triggers.is_empty() {
            return None;
        }
        let (mut command, mut arguments) = (None, None);
        if let Ok(actions) = def.Actions() {
            let mut n = 0i32;
            let _ = actions.Count(&mut n);
            for i in 1..=n {
                let Ok(a) = actions.get_Item(i) else { continue };
                let mut ty = TASK_ACTION_TYPE::default();
                if a.Type(&mut ty).is_ok() && ty == TASK_ACTION_EXEC {
                    if let Ok(ex) = a.cast::<IExecAction>() {
                        let mut p = BSTR::new();
                        let mut g = BSTR::new();
                        let _ = ex.Path(&mut p);
                        let _ = ex.Arguments(&mut g);
                        command = Some(p.to_string()).filter(|s| !s.is_empty());
                        arguments = Some(g.to_string()).filter(|s| !s.is_empty());
                        break;
                    }
                }
            }
        }
        let author = def.RegistrationInfo().ok().and_then(|r| {
            let mut a = BSTR::new();
            r.Author(&mut a).ok()?;
            Some(indirect(&a.to_string())).filter(|s| !s.is_empty())
        });
        Some(RawTask {
            path: t.Path().map(|b| b.to_string()).unwrap_or_default(),
            name: t.Name().map(|b| b.to_string()).unwrap_or_default(),
            enabled: t.Enabled().map(|b| b.as_bool()).unwrap_or(false),
            triggers,
            command,
            arguments,
            author,
        })
    }
}

/// Task authors are sometimes resource references like `$(@%SystemRoot%\system32\Autopilot.dll,-600)`: load the real text
/// (SHLoadIndirectString); if that fails, no author.
fn indirect(s: &str) -> String {
    let Some(inner) = s.strip_prefix("$(").and_then(|r| r.strip_suffix(')')) else { return s.to_string() };
    let mut buf = [0u16; 512];
    match unsafe { SHLoadIndirectString(&HSTRING::from(inner), &mut buf, None) } {
        Ok(()) => {
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            String::from_utf16_lossy(&buf[..end])
        }
        Err(_) => String::new(),
    }
}

struct Sc(SC_HANDLE);
impl Drop for Sc {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

fn open_scm() -> Result<Sc, OsError> {
    unsafe {
        OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE)
            .map(Sc)
            .map_err(|e| hr_err(&e))
    }
}

fn open_service(scm: &Sc, name: &str, access: u32) -> Result<Sc, OsError> {
    unsafe { OpenServiceW(scm.0, &HSTRING::from(name), access).map(Sc).map_err(|e| hr_err(&e)) }
}

/// (start type, delayed, display name, binary path)
fn service_config(svc: &Sc) -> Result<(ServiceStart, bool, String, Option<String>), OsError> {
    unsafe {
        let mut need = 0u32;
        let _ = QueryServiceConfigW(svc.0, None, 0, &mut need);
        let mut buf = vec![0u64; (need as usize).div_ceil(8).max(8)];
        let cfg = buf.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW;
        QueryServiceConfigW(svc.0, Some(cfg), (buf.len() * 8) as u32, &mut need).map_err(|e| hr_err(&e))?;
        let c = &*cfg;
        let start = match c.dwStartType.0 {
            0 => ServiceStart::Boot,
            1 => ServiceStart::System,
            2 => ServiceStart::Automatic,
            3 => ServiceStart::Manual,
            _ => ServiceStart::Disabled,
        };
        let display = win::pwstr_to_string(c.lpDisplayName);
        let image = Some(win::pwstr_to_string(c.lpBinaryPathName)).filter(|s| !s.is_empty());
        let mut delayed = false;
        let mut dbuf = [0u8; 16];
        let mut dneed = 0u32;
        if QueryServiceConfig2W(svc.0, SERVICE_CONFIG_DELAYED_AUTO_START_INFO, Some(&mut dbuf), &mut dneed).is_ok() {
            delayed = i32::from_le_bytes([dbuf[0], dbuf[1], dbuf[2], dbuf[3]]) != 0;
        }
        Ok((start, delayed, display, image))
    }
}

fn current_user_sid() -> Option<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut need = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut need);
        let mut buf = vec![0u64; (need as usize).div_ceil(8)];
        let ok = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr() as *mut _), (buf.len() * 8) as u32, &mut need).is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(token);
        if !ok {
            return None;
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut s).ok()?;
        let out = win::pwstr_to_string(s);
        let _ = LocalFree(Some(HLOCAL(s.0 as *mut _)));
        Some(out)
    }
}

/// Text of an XML file that may be UTF-16 (BOM FF FE) or UTF-8.
fn decode_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let u: Vec<u16> = bytes[2..].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        String::from_utf16_lossy(&u)
    } else {
        String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)).into_owned()
    }
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

impl StartupOs for RealOs {
    fn is_admin(&self) -> bool {
        win::is_admin()
    }

    fn reg_strings(&self, hive: Hive, view: RegView, path: &str) -> Result<Vec<RegString>, OsError> {
        let (root, path) = self.reg(hive, path);
        let Some(k) = Key::open(root, &path, view)? else { return Ok(Vec::new()) };
        Ok(k.values()?
            .into_iter()
            .filter_map(|(name, v)| match v.0 {
                1 | 2 => Some(RegString { name, data: reg_text(&v).unwrap_or_default() }),
                _ => None,
            })
            .collect())
    }

    fn reg_binary(&self, hive: Hive, path: &str, name: &str) -> Result<Option<Vec<u8>>, OsError> {
        let (root, path) = self.reg(hive, path);
        let Some(k) = Key::open(root, &path, RegView::Bits64)? else { return Ok(None) };
        Ok(k.get(name)?.map(|(_, d)| d))
    }

    fn reg_set_binary(&self, hive: Hive, path: &str, name: &str, data: &[u8]) -> Result<(), OsError> {
        let (root, path) = self.reg(hive, path);
        Key::create(root, &path)?.set(name, REG_BINARY, data)
    }

    fn reg_delete_value(&self, hive: Hive, path: &str, name: &str) -> Result<(), OsError> {
        use windows::Win32::System::Registry::{KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY};
        let (root, path) = self.reg(hive, path);
        match Key::open_with(root, &path, KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY)? {
            Some(k) => k.delete(name),
            None => Ok(()),
        }
    }

    fn startup_folder(&self, all_users: bool) -> Result<Vec<FolderItem>, OsError> {
        let dir = unsafe {
            let id = if all_users { &FOLDERID_CommonStartup } else { &FOLDERID_Startup };
            let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).map_err(|e| hr_err(&e))?;
            let s = win::pwstr_to_string(p);
            CoTaskMemFree(Some(p.0 as *const _));
            PathBuf::from(s)
        };
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return Err(OsError::AccessDenied),
            Err(e) => return Err(OsError::Other { code: e.raw_os_error().unwrap_or(0), message: e.to_string() }),
        };
        let _com = Com::init();
        let mut out = Vec::new();
        for ent in rd.flatten() {
            let path = ent.path();
            if path.is_dir() {
                continue;
            }
            let file_name = ent.file_name().to_string_lossy().into_owned();
            let (target, arguments, icon) = if file_name.to_lowercase().ends_with(".lnk") { resolve_link(&path) } else { (None, None, None) };
            out.push(FolderItem { file_name, path, target, arguments, icon });
        }
        out.sort_by_key(|a| a.file_name.to_lowercase());
        Ok(out)
    }

    fn store_startup_tasks(&self) -> Result<Vec<StoreStartupTask>, OsError> {
        let (hk, path) = self.reg(Hive::CurrentUser, STORE_STATE_KEY);
        let Some(root) = Key::open(hk, &path, RegView::Bits64)? else { return Ok(Vec::new()) };
        let pm = PackageManager::new().ok();
        let mut out = Vec::new();
        for pfn in root.subkeys()? {
            let Ok(Some(pk)) = Key::open(root.hkey(), &pfn, RegView::Bits64) else { continue };
            for task_id in pk.subkeys().unwrap_or_default() {
                let Ok(Some(tk)) = Key::open(pk.hkey(), &task_id, RegView::Bits64) else { continue };
                let Some(state) = tk.get("State").ok().flatten().as_ref().and_then(reg_dword) else { continue };
                // Name, publisher, logo from the installed package; a state left over from an uninstalled app is skipped.
                let pkg = pm.as_ref().and_then(|pm| {
                    pm.FindPackagesByUserSecurityIdPackageFamilyName(&HSTRING::new(), &HSTRING::from(pfn.as_str()))
                        .ok()?
                        .First()
                        .ok()?
                        .Current()
                        .ok()
                });
                let Some(pkg) = pkg else { continue };
                out.push(StoreStartupTask {
                    package_family: pfn.clone(),
                    task_id,
                    state,
                    display_name: pkg.DisplayName().ok().map(|s| s.to_string()).filter(|s| !s.is_empty()),
                    publisher: pkg.PublisherDisplayName().ok().map(|s| s.to_string()).filter(|s| !s.is_empty()),
                    logo: pkg.Logo().ok().and_then(|u| u.AbsoluteUri().ok()).and_then(|s| file_uri_to_path(&s.to_string())).map(resolve_logo),
                });
            }
        }
        Ok(out)
    }

    fn logon_tasks(&self) -> Result<Vec<RawTask>, OsError> {
        let _com = Com::init();
        let svc = task_service()?;
        let root = unsafe { svc.GetFolder(&BSTR::from("\\")) }.map_err(|e| hr_err(&e))?;
        let mut out = Vec::new();
        collect_tasks(&root, &mut out);
        Ok(out)
    }

    fn set_task_enabled(&self, path: &str, enabled: bool) -> Result<(), OsError> {
        self.refuse_in_scratch()?;
        let _com = Com::init();
        let svc = task_service()?;
        unsafe {
            let root = svc.GetFolder(&BSTR::from("\\")).map_err(|e| hr_err(&e))?;
            let t = root.GetTask(&BSTR::from(path)).map_err(|e| hr_err(&e))?;
            t.SetEnabled(if enabled { VARIANT_TRUE } else { VARIANT_FALSE }).map_err(|e| hr_err(&e))
        }
    }

    fn task_enabled(&self, path: &str) -> Result<bool, OsError> {
        let _com = Com::init();
        let svc = task_service()?;
        unsafe {
            let root = svc.GetFolder(&BSTR::from("\\")).map_err(|e| hr_err(&e))?;
            let t = root.GetTask(&BSTR::from(path)).map_err(|e| hr_err(&e))?;
            t.Enabled().map(|b| b.as_bool()).map_err(|e| hr_err(&e))
        }
    }

    fn services(&self, also: &[String]) -> Result<Vec<RawService>, OsError> {
        let scm = open_scm()?;
        let mut names: Vec<String> = Vec::new();
        // A u64 buffer keeps the ENUM_SERVICE_STATUS_PROCESSW records aligned. ERROR_MORE_DATA (234) = call again: the resume
        // handle continues where the last call stopped (or the buffer grows when not even one record fitted).
        let mut buf = vec![0u64; 8192];
        let mut resume = 0u32;
        loop {
            let mut need = 0u32;
            let mut count = 0u32;
            let r = unsafe {
                let bytes = std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, buf.len() * 8);
                EnumServicesStatusExW(
                    scm.0,
                    SC_ENUM_PROCESS_INFO,
                    SERVICE_WIN32,
                    SERVICE_STATE_ALL,
                    Some(bytes),
                    &mut need,
                    &mut count,
                    Some(&mut resume),
                    PCWSTR::null(),
                )
            };
            let items = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW, count as usize) };
            for it in items {
                names.push(win::pwstr_to_string(it.lpServiceName));
            }
            match r {
                Ok(()) => break,
                Err(e) if e.code() == windows::core::HRESULT::from_win32(234) => {
                    if count == 0 {
                        buf.resize((need as usize).div_ceil(8) + 1, 0);
                    }
                }
                Err(e) => return Err(hr_err(&e)),
            }
        }
        let mut out = Vec::new();
        for name in names {
            let Ok(svc) = open_service(&scm, &name, SERVICE_QUERY_CONFIG) else { continue };
            let Ok((start, delayed, display_name, image_path)) = service_config(&svc) else { continue };
            if start == ServiceStart::Automatic || also.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                out.push(RawService { name, display_name, start, delayed, image_path });
            }
        }
        Ok(out)
    }

    fn set_service_start(&self, name: &str, start: ServiceStart, delayed: bool) -> Result<(), OsError> {
        self.refuse_in_scratch()?;
        let scm = open_scm()?;
        let svc = open_service(&scm, name, SERVICE_QUERY_CONFIG | SERVICE_CHANGE_CONFIG)?;
        let st = match start {
            ServiceStart::Boot => 0,
            ServiceStart::System => 1,
            ServiceStart::Automatic => 2,
            ServiceStart::Manual => 3,
            ServiceStart::Disabled => 4,
        };
        unsafe {
            ChangeServiceConfigW(
                svc.0,
                ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
                SERVICE_START_TYPE(st),
                SERVICE_ERROR(SERVICE_NO_CHANGE),
                PCWSTR::null(),
                PCWSTR::null(),
                None,
                PCWSTR::null(),
                PCWSTR::null(),
                PCWSTR::null(),
                PCWSTR::null(),
            )
            .map_err(|e| hr_err(&e))?;
            if start == ServiceStart::Automatic {
                let info = SERVICE_DELAYED_AUTO_START_INFO { fDelayedAutostart: delayed.into() };
                ChangeServiceConfig2W(svc.0, SERVICE_CONFIG_DELAYED_AUTO_START_INFO, Some(&info as *const _ as *const _))
                    .map_err(|e| hr_err(&e))?;
            }
        }
        Ok(())
    }

    fn service_start(&self, name: &str) -> Result<(ServiceStart, bool), OsError> {
        let scm = open_scm()?;
        let svc = open_service(&scm, name, SERVICE_QUERY_CONFIG)?;
        let (s, d, _, _) = service_config(&svc)?;
        Ok((s, d))
    }

    fn remembered_services(&self) -> Result<Vec<(String, bool)>, OsError> {
        let (root, path) = self.reg(Hive::CurrentUser, MEMORY_KEY);
        let Some(k) = Key::open(root, &path, RegView::Bits64)? else { return Ok(Vec::new()) };
        Ok(k.values()?.into_iter().filter_map(|(n, v)| reg_dword(&v).map(|d| (n, d != 0))).collect())
    }

    fn remember_service(&self, name: &str, delayed: bool) -> Result<(), OsError> {
        let (root, path) = self.reg(Hive::CurrentUser, MEMORY_KEY);
        Key::create(root, &path)?.set(name, REG_DWORD, &(delayed as u32).to_le_bytes())
    }

    fn forget_service(&self, name: &str) -> Result<(), OsError> {
        self.reg_delete_value(Hive::CurrentUser, MEMORY_KEY, name)
    }

    fn file_info(&self, path: &Path) -> FileInfo {
        win::file_info(path)
    }

    fn expand_env(&self, s: &str) -> String {
        win::expand_env(s)
    }

    fn file_exists(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn subdirs(&self, dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
            .unwrap_or_default()
    }

    fn impact_reports(&self) -> Result<Vec<String>, OsError> {
        let dir = PathBuf::from(win::expand_env(r"%SystemRoot%\System32\wdi\LogFiles\StartupInfo"));
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return Err(OsError::AccessDenied),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(OsError::NotFound),
            Err(e) => return Err(OsError::Other { code: e.raw_os_error().unwrap_or(0), message: e.to_string() }),
        };
        let sid = current_user_sid().unwrap_or_default().to_lowercase();
        let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
            .flatten()
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_lowercase();
                !sid.is_empty() && n.starts_with(&format!("{sid}_startupinfo")) && n.ends_with(".xml")
            })
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        files.sort_by_key(|f| std::cmp::Reverse(f.0));
        let mut out = Vec::new();
        for (_, p) in files {
            match std::fs::read(&p) {
                Ok(b) => out.push(decode_text(&b)),
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return Err(OsError::AccessDenied),
                Err(_) => {}
            }
        }
        Ok(out)
    }

    fn now_filetime(&self) -> u64 {
        let ft = unsafe { GetSystemTimeAsFileTime() };
        ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
    }
}

fn resolve_link(path: &Path) -> (Option<String>, Option<String>, Option<String>) {
    unsafe {
        let Ok(link) = CoCreateInstance::<_, IShellLinkW>(&ShellLink, None, CLSCTX_INPROC_SERVER) else { return (None, None, None) };
        let Ok(pf) = link.cast::<IPersistFile>() else { return (None, None, None) };
        if pf.Load(&HSTRING::from(path.as_os_str()), STGM_READ).is_err() {
            return (None, None, None);
        }
        let text = |buf: &[u16]| {
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..end])).filter(|s| !s.is_empty())
        };
        let mut target = [0u16; 1024];
        let target = link.GetPath(&mut target, std::ptr::null_mut(), 0).ok().and_then(|_| text(&target));
        let mut args = [0u16; 2048];
        let args = link.GetArguments(&mut args).ok().and_then(|_| text(&args));
        let mut icon = [0u16; 1024];
        let mut idx = 0i32;
        let icon = link.GetIconLocation(&mut icon, &mut idx).ok().and_then(|_| text(&icon)).map(|p| format!("{p},{idx}"));
        (target, args, icon)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uri() {
        assert_eq!(
            file_uri_to_path("file:///C:/Program%20Files/WindowsApps/X/Assets/Logo.png"),
            Some(PathBuf::from(r"C:\Program Files\WindowsApps\X\Assets\Logo.png"))
        );
        assert_eq!(file_uri_to_path("https://x"), None);
    }

    #[test]
    fn utf16_and_utf8_text() {
        assert_eq!(decode_text(&[0xFF, 0xFE, b'a', 0, b'b', 0]), "ab");
        assert_eq!(decode_text(&[0xEF, 0xBB, 0xBF, b'a']), "a");
    }
}
