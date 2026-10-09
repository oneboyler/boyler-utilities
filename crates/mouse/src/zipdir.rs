//! A small zip reader that works on PARTS of an archive (Order 066): the end record + central directory from the file's tail,
//! then any one entry from the bytes at its local header. "Get more cursors" shows each pack's real arrow by asking the
//! pack's release page for those few kilobytes only (an HTTP range request) - never the whole 11 MB - and installs from the
//! whole file with the same code. Stored and deflated entries (miniz_oxide), CRC-32 checked; no zip64, no encryption.

/// One entry of the central directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub method: u16,
    pub csize: u32,
    pub size: u32,
    pub crc: u32,
    /// where its local header starts in the archive
    pub local: u32,
}

/// Where the central directory is (from the end record).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tail {
    pub cd_off: u64,
    pub cd_size: u64,
    pub count: usize,
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    b.get(i..i.checked_add(2)?).map(|s| u16::from_le_bytes([s[0], s[1]]))
}
fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    b.get(i..i.checked_add(4)?).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// CRC-32 (the zip polynomial).
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

/// The end record in the last bytes of an archive (the final 64 KB + 22 are enough).
pub fn find_end(tail: &[u8]) -> Option<Tail> {
    if tail.len() < 22 {
        return None;
    }
    let at = (0..=tail.len() - 22).rev().find(|&i| tail[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])?;
    let count = u16_at(tail, at + 10)? as usize;
    let cd_size = u32_at(tail, at + 12)? as u64;
    let cd_off = u32_at(tail, at + 16)? as u64;
    // 0xFFFF / 0xFFFFFFFF mean zip64 (not read)
    if count == 0xFFFF || cd_off == 0xFFFF_FFFF || cd_size == 0xFFFF_FFFF {
        return None;
    }
    Some(Tail { cd_off, cd_size, count })
}

/// The entries of a central directory (`cd` = exactly its bytes).
pub fn parse_directory(cd: &[u8], count: usize) -> Option<Vec<Item>> {
    let mut out = Vec::with_capacity(count.min(4096));
    let mut p = 0usize;
    for _ in 0..count {
        if u32_at(cd, p)? != 0x0201_4b50 {
            return None;
        }
        let flags = u16_at(cd, p + 8)?;
        let method = u16_at(cd, p + 10)?;
        let crc = u32_at(cd, p + 16)?;
        let csize = u32_at(cd, p + 20)?;
        let size = u32_at(cd, p + 24)?;
        let nlen = u16_at(cd, p + 28)? as usize;
        let elen = u16_at(cd, p + 30)? as usize;
        let clen = u16_at(cd, p + 32)? as usize;
        let local = u32_at(cd, p + 42)?;
        let name = String::from_utf8_lossy(cd.get(p + 46..p.checked_add(46 + nlen)?)?).into_owned();
        p = p.checked_add(46 + nlen + elen + clen)?;
        // encrypted or zip64 entries are left out (nothing here needs them)
        if flags & 1 != 0 || csize == u32::MAX || size == u32::MAX || local == u32::MAX {
            continue;
        }
        out.push(Item { name, method, csize, size, crc, local });
    }
    Some(out)
}

/// The directory of a whole archive in memory.
pub fn read_all(zip: &[u8]) -> Option<Vec<Item>> {
    let from = zip.len().saturating_sub(65_535 + 22);
    let t = find_end(&zip[from..])?;
    let cd = zip.get(t.cd_off as usize..(t.cd_off.checked_add(t.cd_size)?) as usize)?;
    parse_directory(cd, t.count)
}

/// How many bytes from the entry's local header on contain its header and data (the local header repeats the name and may
/// carry an extra field of its own length: `slack` covers it).
pub fn span(item: &Item) -> (u64, usize) {
    (item.local as u64, 30 + item.name.len() + 256 + item.csize as usize)
}

/// The entry's bytes, unpacked, from the archive bytes starting at its local header. `max` caps the unpacked size.
pub fn unpack(item: &Item, from_local: &[u8], max: usize) -> Option<Vec<u8>> {
    if u32_at(from_local, 0)? != 0x0403_4b50 || item.size as usize > max {
        return None;
    }
    let start = 30 + u16_at(from_local, 26)? as usize + u16_at(from_local, 28)? as usize;
    let raw = from_local.get(start..start.checked_add(item.csize as usize)?)?;
    let data = match item.method {
        0 => raw.to_vec(),
        8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, item.size as usize).ok()?,
        _ => return None,
    };
    (data.len() == item.size as usize && crc32(&data) == item.crc).then_some(data)
}

/// One entry out of a whole archive in memory.
pub fn unpack_from(zip: &[u8], item: &Item, max: usize) -> Option<Vec<u8>> {
    unpack(item, zip.get(item.local as usize..)?, max)
}

/// An archive of (name, data) pairs, each stored or deflated (for tests: the packs a fake release page serves).
pub fn build(files: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cd = Vec::new();
    for (name, data) in files {
        let local = out.len() as u32;
        let (method, body) = if deflate { (8u16, miniz_oxide::deflate::compress_to_vec(data, 6)) } else { (0u16, data.to_vec()) };
        let crc = crc32(data);
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0]);
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&body);
        cd.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        cd.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
        cd.extend_from_slice(&method.to_le_bytes());
        cd.extend_from_slice(&[0, 0, 0, 0]);
        cd.extend_from_slice(&crc.to_le_bytes());
        cd.extend_from_slice(&(body.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(data.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(name.len() as u16).to_le_bytes());
        cd.extend_from_slice(&[0u8; 12]);
        cd.extend_from_slice(&local.to_le_bytes());
        cd.extend_from_slice(name.as_bytes());
    }
    let cd_off = out.len() as u32;
    out.extend_from_slice(&cd);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_directory_and_each_entry_from_its_own_bytes() {
        let big = vec![7u8; 5000];
        for deflate in [false, true] {
            let z = build(&[("a/one.cur", b"hello cursor"), ("a/two.ani", &big)], deflate);
            let items = read_all(&z).unwrap();
            assert_eq!(items.len(), 2);
            assert_eq!(items[0].name, "a/one.cur");
            // as a range request would give it: only the tail, then only the entry's bytes
            let tail = &z[z.len().saturating_sub(100)..];
            let t = find_end(tail).unwrap();
            let cd = &z[t.cd_off as usize..(t.cd_off + t.cd_size) as usize];
            let items2 = parse_directory(cd, t.count).unwrap();
            assert_eq!(items, items2);
            let (from, len) = span(&items[1]);
            let part = &z[from as usize..(from as usize + len).min(z.len())];
            assert_eq!(unpack(&items[1], part, 1 << 20).unwrap(), big);
            assert_eq!(unpack_from(&z, &items[0], 1 << 20).unwrap(), b"hello cursor");
        }
    }

    #[test]
    fn a_damaged_entry_or_a_too_big_one_is_none() {
        let mut z = build(&[("x.cur", b"abcdef")], false);
        let items = read_all(&z).unwrap();
        assert!(unpack_from(&z, &items[0], 3).is_none(), "bigger than the cap");
        z[35] ^= 0xFF; // a data byte
        assert!(unpack_from(&z, &items[0], 100).is_none(), "CRC");
        assert!(read_all(b"not a zip at all, not even close").is_none());
        assert!(find_end(&[]).is_none());
    }
}
