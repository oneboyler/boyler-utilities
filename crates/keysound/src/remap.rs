//! Key remap (Order 058): Windows' OWN Scancode Map — `HKLM\SYSTEM\CurrentControlSet\Control\Keyboard Layout`, value
//! `Scancode Map` (REG_BINARY). Nothing of ours runs afterwards: Windows applies the map for every app and game at the
//! next sign-in / restart. Reading it needs no admin; writing it needs one admin Yes (the app's admin path; the elevated
//! copy calls [`real::write`] / [`real::clear`]). "Restart to apply" is the page's note.
//!
//! The value: 8 bytes of zero (version, flags) · a count (the mappings + 1, little-endian u32) · one 4-byte entry per
//! mapping — the NEW scancode (u16) then the ORIGINAL scancode (u16), both little-endian; an extended key has the 0xE0
//! prefix in the high byte (right Ctrl = 0xE01D) · four zero bytes. A new scancode of 0 turns the key off.
//! The pure codec ([`encode`] / [`decode`] / [`check`]) is unit-tested; [`real`] is the registry shell.

/// A scancode: the make code, with 0xE000 added for an extended key (right Ctrl 0xE01D, Delete 0xE053 …).
pub type Code = u16;

/// "This key does nothing."
pub const DISABLED: Code = 0;
/// More than this makes no sense for a keyboard (and keeps the value small).
pub const MAX_MAPPINGS: usize = 64;

pub const KEY_PATH: &str = r"SYSTEM\CurrentControlSet\Control\Keyboard Layout";
pub const VALUE_NAME: &str = "Scancode Map";

/// "Pressing `from` gives `to`".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mapping {
    pub from: Code,
    pub to: Code,
}

/// The scancode of a raw key packet (`bu_rawin` gives the make code and the E0 flag).
pub fn code_of(make: u16, extended: bool) -> Code {
    (make & 0x7F) | if extended { 0xE000 } else { 0 }
}

/// What the registry value holds for `maps`; None for no mappings (then the value is deleted, not written empty).
pub fn encode(maps: &[Mapping]) -> Option<Vec<u8>> {
    if maps.is_empty() {
        return None;
    }
    let mut v = Vec::with_capacity(12 + maps.len() * 4 + 4);
    v.extend_from_slice(&[0u8; 8]);
    v.extend_from_slice(&(maps.len() as u32 + 1).to_le_bytes());
    for m in maps {
        v.extend_from_slice(&m.to.to_le_bytes());
        v.extend_from_slice(&m.from.to_le_bytes());
    }
    v.extend_from_slice(&[0u8; 4]);
    Some(v)
}

/// The mappings in a registry value. Err for anything that isn't a well-formed map (it is then never rewritten blindly).
pub fn decode(b: &[u8]) -> Result<Vec<Mapping>, String> {
    if b.len() < 16 {
        return Err(format!("the Scancode Map is {} bytes (at least 16)", b.len()));
    }
    if b[..8] != [0u8; 8] {
        return Err("the Scancode Map has an unknown version".into());
    }
    let count = u32::from_le_bytes([b[8], b[9], b[10], b[11]]) as usize;
    if count == 0 || count - 1 > MAX_MAPPINGS * 4 || b.len() != 12 + count * 4 {
        return Err(format!("the Scancode Map says {count} entries but holds {} bytes", b.len()));
    }
    if b[b.len() - 4..] != [0u8; 4] {
        return Err("the Scancode Map doesn't end with its null entry".into());
    }
    Ok(b[12..b.len() - 4].as_chunks::<4>().0.iter().map(|c| Mapping { to: u16::from_le_bytes([c[0], c[1]]), from: u16::from_le_bytes([c[2], c[3]]) }).collect())
}

/// Is this list fine to write? A key may be mapped once, not onto itself, and `from` can't be 0.
pub fn check(maps: &[Mapping]) -> Result<(), String> {
    if maps.len() > MAX_MAPPINGS {
        return Err(format!("at most {MAX_MAPPINGS} remaps"));
    }
    for (i, m) in maps.iter().enumerate() {
        if m.from == 0 {
            return Err("a remap needs the key to change".into());
        }
        if m.from == m.to {
            return Err(format!("{} can't become itself", name(m.from)));
        }
        if maps[..i].iter().any(|o| o.from == m.from) {
            return Err(format!("{} is remapped twice", name(m.from)));
        }
    }
    Ok(())
}

/// A fixed English name for the keys that don't print a character (the page asks Windows for the layout's own names of the
/// rest; this is the fallback and what the tests read). Unknown → `Key 0x1E`.
pub fn name(c: Code) -> String {
    let s = match c {
        0x0001 => "Esc",
        0x000E => "Backspace",
        0x000F => "Tab",
        0x001C => "Enter",
        0x001D => "Left Ctrl",
        0xE01D => "Right Ctrl",
        0x002A => "Left Shift",
        0x0036 => "Right Shift",
        0x0038 => "Left Alt",
        0xE038 => "Right Alt",
        0x0039 => "Space",
        0x003A => "Caps Lock",
        0x0045 => "Num Lock",
        0x0046 => "Scroll Lock",
        0xE05B => "Left Win",
        0xE05C => "Right Win",
        0xE05D => "Menu",
        0xE037 => "Print Screen",
        0xE052 => "Insert",
        0xE053 => "Delete",
        0xE047 => "Home",
        0xE04F => "End",
        0xE049 => "Page Up",
        0xE051 => "Page Down",
        0xE048 => "Up",
        0xE050 => "Down",
        0xE04B => "Left",
        0xE04D => "Right",
        0x003B..=0x0044 => return format!("F{}", c - 0x3A),
        0x0057 => "F11",
        0x0058 => "F12",
        _ => return format!("Key 0x{c:02X}"),
    };
    s.to_string()
}

#[cfg(windows)]
pub mod real {
    //! The registry shell. `read` needs no admin; `write` / `clear` need it (the app's elevated copy calls them).

    use super::*;
    use std::ffi::c_void;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_LOCAL_MACHINE, REG_BINARY, RRF_RT_REG_BINARY};

    fn path() -> Vec<u16> {
        KEY_PATH.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The mappings Windows has now (an empty list when there is no map). Err: a map that isn't well-formed or can't be read.
    pub fn read() -> Result<Vec<Mapping>, String> {
        let p = path();
        let name = w!("Scancode Map");
        let mut len = 0u32;
        // SAFETY: sizing call, no buffer.
        let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, PCWSTR(p.as_ptr()), name, RRF_RT_REG_BINARY, None, None, Some(&mut len)) };
        if r == ERROR_FILE_NOT_FOUND {
            return Ok(Vec::new());
        }
        if r != ERROR_SUCCESS {
            return Err(format!("reading the Scancode Map: error {}", r.0));
        }
        let mut buf = vec![0u8; len as usize];
        // SAFETY: `buf` holds `len` bytes.
        let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, PCWSTR(p.as_ptr()), name, RRF_RT_REG_BINARY, None, Some(buf.as_mut_ptr() as *mut c_void), Some(&mut len)) };
        if r != ERROR_SUCCESS {
            return Err(format!("reading the Scancode Map: error {}", r.0));
        }
        buf.truncate(len as usize);
        decode(&buf)
    }

    /// Writes the map (admin). An empty list removes the value.
    pub fn write(maps: &[Mapping]) -> Result<(), String> {
        check(maps)?;
        let Some(bytes) = encode(maps) else { return clear() };
        let p = path();
        // SAFETY: `bytes` is a whole value of `bytes.len()` bytes.
        let r = unsafe { RegSetKeyValueW(HKEY_LOCAL_MACHINE, PCWSTR(p.as_ptr()), w!("Scancode Map"), REG_BINARY.0, Some(bytes.as_ptr() as *const c_void), bytes.len() as u32) };
        if r == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("writing the Scancode Map: error {}", r.0))
        }
    }

    /// Removes the map (admin): every key is as the keyboard made it again after the restart.
    pub fn clear() -> Result<(), String> {
        let p = path();
        // SAFETY: plain value delete.
        let r = unsafe { RegDeleteKeyValueW(HKEY_LOCAL_MACHINE, PCWSTR(p.as_ptr()), w!("Scancode Map")) };
        if r == ERROR_SUCCESS || r == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!("removing the Scancode Map: error {}", r.0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(from: Code, to: Code) -> Mapping {
        Mapping { from, to }
    }

    #[test]
    fn caps_lock_to_escape_is_windows_exact_bytes() {
        // the well-known example: Caps Lock (0x3A) -> Esc (0x01)
        let b = encode(&[m(0x3A, 0x01)]).unwrap();
        assert_eq!(b, vec![0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0x01, 0x00, 0x3A, 0x00, 0, 0, 0, 0]);
        assert_eq!(decode(&b).unwrap(), vec![m(0x3A, 0x01)]);
    }

    #[test]
    fn extended_keys_and_disabled_keys_round_trip() {
        let maps = vec![m(0xE038, 0xE01D), m(0xE05B, DISABLED), m(0x3A, 0x1D)];
        let b = encode(&maps).unwrap();
        assert_eq!(b.len(), 8 + 4 + 3 * 4 + 4);
        assert_eq!(u32::from_le_bytes([b[8], b[9], b[10], b[11]]), 4, "the count includes the null entry");
        assert_eq!(&b[12..16], &[0x1D, 0xE0, 0x38, 0xE0], "new scancode first, then the original (little-endian)");
        assert_eq!(decode(&b).unwrap(), maps);
    }

    #[test]
    fn no_mappings_means_no_value() {
        assert_eq!(encode(&[]), None);
    }

    #[test]
    fn a_damaged_value_is_refused_not_guessed() {
        let good = encode(&[m(0x3A, 0x01)]).unwrap();
        assert!(decode(&good[..10]).is_err(), "too short");
        let mut bad = good.clone();
        bad[0] = 1;
        assert!(decode(&bad).is_err(), "unknown version");
        let mut bad = good.clone();
        bad[8] = 5;
        assert!(decode(&bad).is_err(), "count and size disagree");
        let mut bad = good.clone();
        let n = bad.len();
        bad[n - 1] = 1;
        assert!(decode(&bad).is_err(), "no null entry");
        assert!(decode(&[]).is_err());
    }

    #[test]
    fn check_refuses_nonsense() {
        assert!(check(&[m(0x3A, 0x01), m(0xE038, 0xE01D)]).is_ok());
        assert!(check(&[m(0x3A, 0x3A)]).unwrap_err().contains("itself"));
        assert!(check(&[m(0x3A, 0x01), m(0x3A, 0x02)]).unwrap_err().contains("twice"));
        assert!(check(&[m(0, 0x01)]).is_err());
        let many: Vec<_> = (1..=(MAX_MAPPINGS as u16 + 1)).map(|i| m(i, 0x200 + i)).collect();
        assert!(check(&many).is_err());
    }

    #[test]
    fn codes_and_names() {
        assert_eq!(code_of(0x1D, true), 0xE01D);
        assert_eq!(code_of(0x1D, false), 0x1D);
        assert_eq!(code_of(0x9D, false), 0x1D, "the break bit isn't part of the code");
        assert_eq!(name(0x3A), "Caps Lock");
        assert_eq!(name(0xE038), "Right Alt");
        assert_eq!(name(0x3B), "F1");
        assert_eq!(name(0x44), "F10");
        assert_eq!(name(0x1E), "Key 0x1E");
    }
}
