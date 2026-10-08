//! Small Windows helpers for the real OS layer: registry, version info, env, admin check, COM init.

use std::path::Path;

use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, WIN32_ERROR};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::os::{FileInfo, Hive, OsError, RegView};

pub fn os_err(code: u32) -> OsError {
    match code {
        5 => OsError::AccessDenied,
        2 | 3 | 1060 => OsError::NotFound, // file / path not found, service does not exist
        c => OsError::Other { code: c as i32, message: windows::core::HRESULT::from_win32(c).message() },
    }
}

pub fn hr_err(e: &windows::core::Error) -> OsError {
    let hr = e.code().0 as u32;
    // HRESULT_FROM_WIN32(x) = 0x8007xxxx
    if hr & 0xFFFF_0000 == 0x8007_0000 {
        return os_err(hr & 0xFFFF);
    }
    OsError::Other { code: e.code().0, message: e.message() }
}

fn check(r: WIN32_ERROR) -> Result<(), OsError> {
    if r.0 == 0 {
        Ok(())
    } else {
        Err(os_err(r.0))
    }
}

pub fn root(h: Hive) -> HKEY {
    match h {
        Hive::CurrentUser => HKEY_CURRENT_USER,
        Hive::LocalMachine => HKEY_LOCAL_MACHINE,
    }
}

pub fn view_flag(v: RegView) -> REG_SAM_FLAGS {
    match v {
        RegView::Bits64 => KEY_WOW64_64KEY,
        RegView::Bits32 => KEY_WOW64_32KEY,
    }
}

/// An open registry key, closed on drop.
pub struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

/// A value: (type, raw bytes).
pub type RawValue = (u32, Vec<u8>);

impl Key {
    /// Open for reading. A missing key = `Ok(None)`.
    pub fn open(root: HKEY, path: &str, view: RegView) -> Result<Option<Key>, OsError> {
        Self::open_with(root, path, KEY_READ | view_flag(view))
    }

    pub fn open_with(root: HKEY, path: &str, sam: REG_SAM_FLAGS) -> Result<Option<Key>, OsError> {
        let mut h = HKEY::default();
        let r = unsafe { RegOpenKeyExW(root, &HSTRING::from(path), None, sam, &mut h) };
        match r.0 {
            0 => Ok(Some(Key(h))),
            2 | 3 => Ok(None),
            c => Err(os_err(c)),
        }
    }

    /// Open (creating if needed) for writing, 64-bit view.
    pub fn create(root: HKEY, path: &str) -> Result<Key, OsError> {
        let mut h = HKEY::default();
        let r = unsafe {
            RegCreateKeyExW(
                root,
                &HSTRING::from(path),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_WRITE | KEY_WOW64_64KEY,
                None,
                &mut h,
                None,
            )
        };
        check(r)?;
        Ok(Key(h))
    }

    pub fn hkey(&self) -> HKEY {
        self.0
    }

    pub fn values(&self) -> Result<Vec<(String, RawValue)>, OsError> {
        let (mut count, mut max_name, mut max_data) = (0u32, 0u32, 0u32);
        check(unsafe {
            RegQueryInfoKeyW(
                self.0,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(&mut count),
                Some(&mut max_name),
                Some(&mut max_data),
                None,
                None,
            )
        })?;
        let mut out = Vec::new();
        let mut name = vec![0u16; max_name as usize + 2];
        let mut data = vec![0u8; max_data as usize + 4];
        for i in 0..count {
            let mut nlen = name.len() as u32;
            let mut dlen = data.len() as u32;
            let mut ty = 0u32;
            let r = unsafe {
                RegEnumValueW(
                    self.0,
                    i,
                    Some(PWSTR(name.as_mut_ptr())),
                    &mut nlen,
                    None,
                    Some(&mut ty),
                    Some(data.as_mut_ptr()),
                    Some(&mut dlen),
                )
            };
            if r.0 == 259 {
                break; // ERROR_NO_MORE_ITEMS (values changed while reading)
            }
            if r.0 != 0 {
                continue;
            }
            out.push((String::from_utf16_lossy(&name[..nlen as usize]), (ty, data[..dlen as usize].to_vec())));
        }
        Ok(out)
    }

    pub fn subkeys(&self) -> Result<Vec<String>, OsError> {
        let mut out = Vec::new();
        let mut i = 0;
        loop {
            let mut name = [0u16; 512];
            let mut len = name.len() as u32;
            let r = unsafe { RegEnumKeyExW(self.0, i, Some(PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None) };
            match r.0 {
                0 => out.push(String::from_utf16_lossy(&name[..len as usize])),
                259 => break,
                c => return Err(os_err(c)),
            }
            i += 1;
        }
        Ok(out)
    }

    pub fn get(&self, name: &str) -> Result<Option<RawValue>, OsError> {
        let h = HSTRING::from(name);
        let mut ty = REG_VALUE_TYPE::default();
        let mut len = 0u32;
        let r = unsafe { RegQueryValueExW(self.0, &h, None, Some(&mut ty), None, Some(&mut len)) };
        match r.0 {
            0 => {}
            2 => return Ok(None),
            c => return Err(os_err(c)),
        }
        let mut data = vec![0u8; len as usize];
        let r = unsafe { RegQueryValueExW(self.0, &h, None, Some(&mut ty), Some(data.as_mut_ptr()), Some(&mut len)) };
        match r.0 {
            0 => {
                data.truncate(len as usize);
                Ok(Some((ty.0, data)))
            }
            2 => Ok(None),
            c => Err(os_err(c)),
        }
    }

    pub fn set(&self, name: &str, ty: REG_VALUE_TYPE, data: &[u8]) -> Result<(), OsError> {
        check(unsafe { RegSetValueExW(self.0, &HSTRING::from(name), None, ty, Some(data)) })
    }

    pub fn delete(&self, name: &str) -> Result<(), OsError> {
        let r = unsafe { RegDeleteValueW(self.0, &HSTRING::from(name)) };
        match r.0 {
            0 | 2 => Ok(()),
            c => Err(os_err(c)),
        }
    }

    /// The key's last-write time as FILETIME ticks.
    pub fn last_write(&self) -> Option<u64> {
        let mut ft = FILETIME::default();
        let r = unsafe { RegQueryInfoKeyW(self.0, None, None, None, None, None, None, None, None, None, None, Some(&mut ft)) };
        (r.0 == 0).then_some(((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64)
    }
}

/// REG_SZ / REG_EXPAND_SZ / REG_MULTI_SZ (first string) bytes → text.
pub fn reg_text(v: &RawValue) -> Option<String> {
    match v.0 {
        1 | 2 | 7 => {
            let u: Vec<u16> = v.1.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
            let end = u.iter().position(|&c| c == 0).unwrap_or(u.len());
            Some(String::from_utf16_lossy(&u[..end]))
        }
        _ => None,
    }
}

pub fn reg_dword(v: &RawValue) -> Option<u32> {
    match (v.0, v.1.len()) {
        (4, 4..) => Some(u32::from_le_bytes([v.1[0], v.1[1], v.1[2], v.1[3]])),
        _ => None,
    }
}

pub fn expand_env(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let src = HSTRING::from(s);
    let n = unsafe { ExpandEnvironmentStringsW(&src, None) };
    if n == 0 {
        return s.to_string();
    }
    let mut buf = vec![0u16; n as usize];
    let n = unsafe { ExpandEnvironmentStringsW(&src, Some(&mut buf)) };
    if n == 0 || n as usize > buf.len() {
        return s.to_string();
    }
    String::from_utf16_lossy(&buf[..(n as usize).saturating_sub(1)])
}

pub fn is_admin() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut el = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut el as *mut _ as *mut core::ffi::c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && el.TokenIsElevated != 0
    }
}

/// CompanyName + FileDescription from an exe's version resource (first translation, then en-US fallbacks).
pub fn file_info(path: &Path) -> FileInfo {
    let p = HSTRING::from(path.as_os_str());
    unsafe {
        let size = GetFileVersionInfoSizeW(&p, None);
        if size == 0 {
            return FileInfo::default();
        }
        let mut buf = vec![0u8; size as usize];
        if GetFileVersionInfoW(&p, None, size, buf.as_mut_ptr() as *mut _).is_err() {
            return FileInfo::default();
        }
        let mut langs: Vec<String> = Vec::new();
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        if VerQueryValueW(buf.as_ptr() as *const _, &HSTRING::from(r"\VarFileInfo\Translation"), &mut ptr, &mut len).as_bool()
            && len >= 4
        {
            let words = std::slice::from_raw_parts(ptr as *const u16, (len / 2) as usize);
            for pair in words.as_chunks::<2>().0 {
                langs.push(format!("{:04x}{:04x}", pair[0], pair[1]));
            }
        }
        langs.extend(["040904b0".to_string(), "040904e4".to_string(), "04090000".to_string()]);
        let query = |field: &str| -> Option<String> {
            for l in &langs {
                let mut ptr = std::ptr::null_mut();
                let mut len = 0u32;
                let q = HSTRING::from(format!(r"\StringFileInfo\{l}\{field}"));
                if VerQueryValueW(buf.as_ptr() as *const _, &q, &mut ptr, &mut len).as_bool() && len > 0 {
                    let s = std::slice::from_raw_parts(ptr as *const u16, len as usize);
                    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
                    let t = String::from_utf16_lossy(&s[..end]).trim().to_string();
                    if !t.is_empty() {
                        return Some(t);
                    }
                }
            }
            None
        };
        FileInfo { company: query("CompanyName"), description: query("FileDescription") }
    }
}

/// COM for this thread while alive (multithreaded; if the thread already chose another mode, COM still works and we don't uninit).
pub struct Com(bool);

impl Com {
    pub fn init() -> Com {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        Com(hr.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() }
        }
    }
}

pub fn pwstr_to_string(p: PWSTR) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { p.to_string().unwrap_or_default() }
    }
}

// ---- scratch-key helpers for the real-registry test (boss answer A_006_01: only under HKCU\Software\BoylerUtilities-test\D) ----

/// The only place the scratch helpers and `RealOs::scratch` may touch (boss answer A_006_01).
pub const SCRATCH_ROOT: &str = r"Software\BoylerUtilities-test\D";

/// `Ok` only for SCRATCH_ROOT itself or a key below it (no `..`).
pub fn check_scratch_path(path: &str) -> Result<(), OsError> {
    let p = path.trim_matches('\\').to_lowercase();
    let root = SCRATCH_ROOT.to_lowercase();
    let inside = p == root || p.starts_with(&format!("{root}\\"));
    if inside && !p.split('\\').any(|part| part == ".." || part == ".") {
        Ok(())
    } else {
        Err(OsError::Other { code: 0, message: format!("outside the scratch key {SCRATCH_ROOT}: {path}") })
    }
}

/// Write a REG_SZ value in the scratch key (creating it). Refuses any path outside SCRATCH_ROOT.
pub fn hkcu_set_string(path: &str, name: &str, value: &str) -> Result<(), OsError> {
    check_scratch_path(path)?;
    let mut bytes: Vec<u8> = value.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    bytes.extend_from_slice(&[0, 0]);
    Key::create(HKEY_CURRENT_USER, path)?.set(name, REG_SZ, &bytes)
}

/// Delete a whole key tree in the scratch key (missing = Ok). Refuses any path outside SCRATCH_ROOT.
pub fn hkcu_delete_tree(path: &str) -> Result<(), OsError> {
    check_scratch_path(path)?;
    let r = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(path)) };
    match r.0 {
        0 | 2 => {}
        c => return Err(os_err(c)),
    }
    let r = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(path)) };
    match r.0 {
        0 | 2 => Ok(()),
        c => Err(os_err(c)),
    }
}

/// Delete a scratch key only if it has no subkeys (Windows refuses otherwise). Missing = Ok. Refuses paths outside SCRATCH_ROOT.
pub fn hkcu_delete_key_if_empty(path: &str) -> Result<(), OsError> {
    check_scratch_path(path)?;
    let r = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(path)) };
    match r.0 {
        0 | 2 => Ok(()),
        c => Err(os_err(c)),
    }
}

pub fn hkcu_key_exists(path: &str) -> bool {
    matches!(Key::open(HKEY_CURRENT_USER, path, RegView::Bits64), Ok(Some(_)))
}
