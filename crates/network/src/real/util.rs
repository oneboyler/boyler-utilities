//! Small helpers for the real layer: wide strings, GUIDs, socket addresses, elevation.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use windows::core::GUID;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WIN32_ERROR};
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6, SOCKADDR, SOCKADDR_IN, SOCKADDR_IN6};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::error::NetError;

pub fn os_err(call: &'static str, code: u32) -> NetError {
    match code {
        5 => NetError::AccessDenied(call.into()),
        _ => NetError::Os { call, code },
    }
}

pub fn win32(call: &'static str, r: WIN32_ERROR) -> Result<(), NetError> {
    if r.0 == 0 {
        Ok(())
    } else {
        Err(os_err(call, r.0))
    }
}

pub fn from_wide(s: &[u16]) -> String {
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..end])
}

/// Reads a NUL-terminated wide string from a pointer.
///
/// # Safety
/// `p` must be null or point to a NUL-terminated UTF-16 string.
pub unsafe fn pwstr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// "{4D36E972-E325-11CE-BFC1-08002BE10318}" (Windows' own adapter-name spelling).
pub fn guid_string(g: &GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g.data1, g.data2, g.data3, g.data4[0], g.data4[1], g.data4[2], g.data4[3], g.data4[4], g.data4[5], g.data4[6], g.data4[7]
    )
}

pub fn parse_guid(s: &str) -> Option<GUID> {
    let h: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if h.len() != 32 {
        return None;
    }
    u128::from_str_radix(&h, 16).ok().map(GUID::from_u128)
}

/// sockaddr -> IP address (IPv4 / IPv6 only).
///
/// # Safety
/// `sa` must be null or point to a valid SOCKADDR of its family's size.
pub unsafe fn sockaddr_ip(sa: *const SOCKADDR) -> Option<IpAddr> {
    if sa.is_null() {
        return None;
    }
    match (*sa).sa_family {
        f if f == AF_INET => {
            let b = (*(sa as *const SOCKADDR_IN)).sin_addr.S_un.S_un_b;
            Some(IpAddr::V4(Ipv4Addr::new(b.s_b1, b.s_b2, b.s_b3, b.s_b4)))
        }
        f if f == AF_INET6 => {
            let a = &*(sa as *const SOCKADDR_IN6);
            Some(IpAddr::V6(Ipv6Addr::from(a.sin6_addr.u.Byte)))
        }
        _ => None,
    }
}

pub fn is_elevated() -> bool {
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

/// Reads a NUL-terminated ANSI string (adapter GUID names are plain ASCII).
///
/// # Safety
/// `p` must be null or point to a NUL-terminated string.
pub unsafe fn pstr_or_empty(p: *const u8) -> String {
    if p.is_null() {
        return String::new();
    }
    std::ffi::CStr::from_ptr(p as *const std::ffi::c_char).to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_round_trip() {
        let s = "{12345678-9ABC-4DEF-8123-456789ABCDEF}";
        assert_eq!(guid_string(&parse_guid(s).unwrap()), s);
        assert!(parse_guid("{nope}").is_none());
    }
}
