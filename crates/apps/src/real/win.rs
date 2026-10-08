//! Small Windows registry helpers for the real OS layer (same shape as bu-startup's; each feature crate stands alone).

use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::Registry::*;

use crate::os::{Hive, OsError, RegValue, RegView};

pub fn os_err(code: u32) -> OsError {
    match code {
        5 => OsError::AccessDenied,
        2 | 3 => OsError::NotFound,
        1223 => OsError::Cancelled, // ERROR_CANCELLED: No at the admin prompt
        c => OsError::Other { code: c as i32, message: windows::core::HRESULT::from_win32(c).message() },
    }
}

pub fn hr_err(e: &windows::core::Error) -> OsError {
    let hr = e.code().0 as u32;
    if hr & 0xFFFF_0000 == 0x8007_0000 {
        return os_err(hr & 0xFFFF);
    }
    OsError::Other { code: e.code().0, message: e.message() }
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

pub struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

impl Key {
    pub fn open(root: HKEY, path: &str, view: RegView) -> Result<Option<Key>, OsError> {
        let mut h = HKEY::default();
        let r = unsafe { RegOpenKeyExW(root, &HSTRING::from(path), None, KEY_READ | view_flag(view), &mut h) };
        match r.0 {
            0 => Ok(Some(Key(h))),
            2 | 3 => Ok(None),
            c => Err(os_err(c)),
        }
    }

    pub fn hkey(&self) -> HKEY {
        self.0
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

    /// String and DWORD values (names lower-case).
    pub fn values(&self) -> Result<Vec<(String, RegValue)>, OsError> {
        let (mut count, mut max_name, mut max_data) = (0u32, 0u32, 0u32);
        let r = unsafe {
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
        };
        if r.0 != 0 {
            return Err(os_err(r.0));
        }
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
                break;
            }
            if r.0 != 0 {
                continue;
            }
            let n = String::from_utf16_lossy(&name[..nlen as usize]).to_lowercase();
            let bytes = &data[..dlen as usize];
            match ty {
                1 | 2 => {
                    let u: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
                    let end = u.iter().position(|&c| c == 0).unwrap_or(u.len());
                    out.push((n, RegValue::Str(String::from_utf16_lossy(&u[..end]))));
                }
                4 if bytes.len() >= 4 => out.push((n, RegValue::Dword(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])))),
                _ => {}
            }
        }
        Ok(out)
    }

    pub fn last_write(&self) -> Option<u64> {
        let mut ft = FILETIME::default();
        let r = unsafe { RegQueryInfoKeyW(self.0, None, None, None, None, None, None, None, None, None, None, Some(&mut ft)) };
        (r.0 == 0).then_some(((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64)
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
