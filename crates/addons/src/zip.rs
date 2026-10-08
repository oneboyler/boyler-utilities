//! A small zip reader for the one archive we unpack (Raw Accel's release, already SHA-256 checked): the central directory,
//! stored and deflated entries (miniz_oxide), CRC-32 checked per file. No zip64, no encryption (refused). Paths are
//! taken apart by `/`; an entry with an empty, `.`, `..`, drive or backslash part is refused, so nothing can land outside
//! the destination folder.

use crate::error::{AddonError, Result};
use std::path::{Path, PathBuf};

/// One file (or folder, `dir`) of the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub data: Vec<u8>,
}

fn bad(s: &str) -> AddonError {
    AddonError::Files(format!("the archive is damaged ({s})"))
}

fn u16_at(b: &[u8], i: usize) -> Result<u16> {
    b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| bad("cut short"))
}
fn u32_at(b: &[u8], i: usize) -> Result<u32> {
    b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| bad("cut short"))
}

/// CRC-32 (the zip / PNG polynomial).
pub fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *t = c;
    }
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// Every entry of the archive, unpacked. `max_total` caps the unpacked bytes (a damaged size can't fill the disk).
pub fn read(zip: &[u8], max_total: usize) -> Result<Vec<Entry>> {
    // end of central directory: the last "PK\5\6" within the final 64 KB + 22
    let from = zip.len().saturating_sub(65_535 + 22);
    let eocd = (from..zip.len().saturating_sub(21)).rev().find(|&i| zip[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]).ok_or_else(|| bad("no directory"))?;
    let count = u16_at(zip, eocd + 10)? as usize;
    let mut p = u32_at(zip, eocd + 16)? as usize;
    let mut out = Vec::with_capacity(count);
    let mut total = 0usize;
    for _ in 0..count {
        if u32_at(zip, p)? != 0x0201_4b50 {
            return Err(bad("directory entry"));
        }
        let flags = u16_at(zip, p + 8)?;
        let method = u16_at(zip, p + 10)?;
        let crc = u32_at(zip, p + 16)?;
        let csize = u32_at(zip, p + 20)? as usize;
        let usize_ = u32_at(zip, p + 24)? as usize;
        let nlen = u16_at(zip, p + 28)? as usize;
        let elen = u16_at(zip, p + 30)? as usize;
        let clen = u16_at(zip, p + 32)? as usize;
        let local = u32_at(zip, p + 42)? as usize;
        let name = String::from_utf8_lossy(zip.get(p.saturating_add(46)..p.saturating_add(46 + nlen)).ok_or_else(|| bad("name"))?).into_owned();
        p = p.checked_add(46 + nlen + elen + clen).ok_or_else(|| bad("directory entry"))?;
        if flags & 1 != 0 {
            return Err(bad("encrypted"));
        }
        if csize == u32::MAX as usize || usize_ == u32::MAX as usize {
            return Err(bad("zip64"));
        }
        total = total.saturating_add(usize_);
        if total > max_total {
            return Err(bad("too big"));
        }
        if u32_at(zip, local)? != 0x0403_4b50 {
            return Err(bad("file header"));
        }
        let start = local.checked_add(30 + u16_at(zip, local + 26)? as usize + u16_at(zip, local + 28)? as usize).ok_or_else(|| bad("file data"))?;
        let raw = zip.get(start..start.checked_add(csize).ok_or_else(|| bad("file data"))?).ok_or_else(|| bad("file data"))?;
        let dir = name.ends_with('/');
        let data = match method {
            0 => raw.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, usize_).map_err(|_| bad("deflate"))?,
            _ => return Err(bad("compression method")),
        };
        if data.len() != usize_ || crc32(&data) != crc {
            return Err(bad("CRC"));
        }
        out.push(Entry { name, dir, data });
    }
    Ok(out)
}

/// The parts of a zip path under `prefix` (e.g. "RawAccel/"), or None when the entry is not under it or a part is not a
/// plain name.
pub fn safe_parts<'a>(name: &'a str, prefix: &str) -> Option<Vec<&'a str>> {
    let rest = name.strip_prefix(prefix)?;
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.is_empty() {
        return Some(Vec::new());
    }
    let parts: Vec<&str> = rest.split('/').collect();
    let plain = |s: &&str| !s.is_empty() && *s != "." && *s != ".." && !s.contains(['\\', ':']) && !s.ends_with('.') && !s.ends_with(' ');
    parts.iter().all(plain).then_some(parts)
}

/// Unpack every entry under `prefix` into `dest` (created). An entry outside `prefix` or with an unsafe path refuses the
/// whole archive. Returns the files written.
pub fn extract_under(entries: &[Entry], prefix: &str, dest: &Path) -> Result<Vec<PathBuf>> {
    let io = |e: std::io::Error| AddonError::Files(e.to_string());
    let mut plan = Vec::new();
    for e in entries {
        let parts = safe_parts(&e.name, prefix).ok_or_else(|| bad(&format!("unexpected entry {}", e.name)))?;
        let mut p = dest.to_path_buf();
        parts.iter().for_each(|s| p.push(s));
        plan.push((p, e));
    }
    std::fs::create_dir_all(dest).map_err(io)?;
    let mut files = Vec::new();
    for (p, e) in plan {
        if e.dir {
            std::fs::create_dir_all(&p).map_err(io)?;
        } else {
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).map_err(io)?;
            }
            std::fs::write(&p, &e.data).map_err(io)?;
            files.push(p);
        }
    }
    Ok(files)
}

/// Tests: a zip of (name, data) pairs, stored (or deflated when `deflate`).
pub fn build(files: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cd = Vec::new();
    for (name, data) in files {
        let crc = crc32(data);
        let (method, body) = if deflate && !name.ends_with('/') { (8u16, miniz_oxide::deflate::compress_to_vec(data, 6)) } else { (0u16, data.to_vec()) };
        let off = out.len() as u32;
        let head = |sig: u32, central: bool| {
            let mut h = Vec::new();
            h.extend_from_slice(&sig.to_le_bytes());
            if central {
                h.extend_from_slice(&20u16.to_le_bytes());
            }
            h.extend_from_slice(&20u16.to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            h.extend_from_slice(&method.to_le_bytes());
            h.extend_from_slice(&[0, 0, 0, 0]);
            h.extend_from_slice(&crc.to_le_bytes());
            h.extend_from_slice(&(body.len() as u32).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            if central {
                // comment length, disk, internal + external attributes
                h.extend_from_slice(&[0u8; 10]);
                h.extend_from_slice(&off.to_le_bytes());
            }
            h.extend_from_slice(name.as_bytes());
            h
        };
        out.extend_from_slice(&head(0x0403_4b50, false));
        out.extend_from_slice(&body);
        cd.extend_from_slice(&head(0x0201_4b50, true));
    }
    let cd_off = out.len() as u32;
    out.extend_from_slice(&cd);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_of_a_known_text() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn stored_and_deflated_round_trip() {
        for deflate in [false, true] {
            let z = build(&[("RawAccel/", b""), ("RawAccel/a.txt", b"hello hello hello"), ("RawAccel/driver/x.sys", &[7u8; 3000])], deflate);
            let e = read(&z, 1 << 20).unwrap();
            assert_eq!(e.len(), 3);
            assert!(e[0].dir);
            assert_eq!(e[1].data, b"hello hello hello");
            assert_eq!(e[2].data, vec![7u8; 3000]);
        }
    }

    #[test]
    fn a_damaged_byte_is_refused() {
        let mut z = build(&[("RawAccel/a.txt", b"hello")], false);
        let i = z.windows(5).position(|w| w == b"hello").unwrap();
        z[i] = b'j';
        assert!(matches!(read(&z, 1 << 20), Err(AddonError::Files(_))));
    }

    #[test]
    fn the_size_cap_holds() {
        let z = build(&[("RawAccel/a", &[0u8; 5000])], true);
        assert!(read(&z, 4000).is_err());
    }

    #[test]
    fn only_plain_names_under_the_prefix() {
        assert_eq!(safe_parts("RawAccel/driver/rawaccel.sys", "RawAccel/"), Some(vec!["driver", "rawaccel.sys"]));
        assert_eq!(safe_parts("RawAccel/", "RawAccel/"), Some(vec![]));
        for n in ["Other/x", "RawAccel/../x", "RawAccel/a/../../x", "RawAccel/C:/x", "RawAccel/a\\..\\x", "RawAccel//x", "RawAccel/x."] {
            assert_eq!(safe_parts(n, "RawAccel/"), None, "{n}");
        }
    }

    #[test]
    fn an_escaping_entry_refuses_the_whole_archive() {
        let dir = std::env::temp_dir().join(format!("bu-addons-zip-{}", std::process::id()));
        let z = build(&[("RawAccel/ok.txt", b"1"), ("RawAccel/../evil.txt", b"2")], false);
        let e = read(&z, 1 << 20).unwrap();
        assert!(extract_under(&e, "RawAccel/", &dir).is_err());
        assert!(!dir.exists(), "nothing written when one entry is unsafe");
        let z = build(&[("RawAccel/sub/ok.txt", b"1")], false);
        let files = extract_under(&read(&z, 1 << 20).unwrap(), "RawAccel/", &dir).unwrap();
        assert_eq!(std::fs::read(&files[0]).unwrap(), b"1");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
