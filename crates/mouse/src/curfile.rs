//! Reading a cursor FILE as a picture (Order 066): `.cur` (Windows cursor: one or several sizes, each a DIB with an AND mask
//! or a PNG) and `.ani` (animated: RIFF "ACON" holding one `.cur` per frame). The pickers show every set's REAL arrow with
//! this - never a stand-in drawing. Pure Rust, no Windows calls, and it never reads outside the bytes it is given: a damaged
//! file is `None`, not a panic.

/// One decoded cursor picture: straight (not premultiplied) RGBA, top row first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorImage {
    pub w: u32,
    pub h: u32,
    /// the click point inside the picture
    pub hot: (u32, u32),
    pub rgba: Vec<u8>,
}

/// Biggest picture side read (a damaged header can't ask for gigabytes).
const MAX_SIDE: u32 = 512;

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    b.get(i..i.checked_add(2)?).map(|s| u16::from_le_bytes([s[0], s[1]]))
}
fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    b.get(i..i.checked_add(4)?).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn i32_at(b: &[u8], i: usize) -> Option<i32> {
    u32_at(b, i).map(|v| v as i32)
}

/// The picture of a cursor file's bytes: a `.cur` (the size nearest `want` px, preferring the next one up), or a `.ani`
/// (its first frame). `None` = not a cursor / damaged.
pub fn decode(bytes: &[u8], want: u32) -> Option<CursorImage> {
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"ACON" {
        return decode_ani(bytes, want);
    }
    decode_cur(bytes, want)
}

/// The sizes a `.cur` holds, as (width, height) px.
pub fn sizes(bytes: &[u8]) -> Vec<(u32, u32)> {
    entries(bytes).into_iter().map(|e| (e.w, e.h)).collect()
}

struct Entry {
    w: u32,
    h: u32,
    bpp: u32,
    hot: (u32, u32),
    off: usize,
    len: usize,
}

fn entries(b: &[u8]) -> Vec<Entry> {
    let mut out = Vec::new();
    if !(u16_at(b, 0) == Some(0) && matches!(u16_at(b, 2), Some(1) | Some(2))) {
        return out;
    }
    let is_cur = u16_at(b, 2) == Some(2);
    let n = u16_at(b, 4).unwrap_or(0) as usize;
    for i in 0..n.min(64) {
        let p = 6 + i * 16;
        let (Some(&w), Some(&h)) = (b.get(p), b.get(p + 1)) else { break };
        let (Some(x), Some(y), Some(len), Some(off)) = (u16_at(b, p + 4), u16_at(b, p + 6), u32_at(b, p + 8), u32_at(b, p + 12)) else { break };
        let side = |v: u8| if v == 0 { 256 } else { v as u32 };
        // for a .cur, planes / bit count hold the hot spot; for an icon they are the colour depth
        let (hot, bpp) = if is_cur { ((x as u32, y as u32), 0) } else { ((0, 0), y as u32) };
        out.push(Entry { w: side(w), h: side(h), bpp, hot, off: off as usize, len: len as usize });
    }
    out
}

fn decode_cur(b: &[u8], want: u32) -> Option<CursorImage> {
    let es = entries(b);
    // the smallest picture that is at least `want`, else the biggest there is
    let pick = es.iter().filter(|e| e.w.max(e.h) >= want).min_by_key(|e| (e.w.max(e.h), std::cmp::Reverse(e.bpp))).or_else(|| es.iter().max_by_key(|e| (e.w.max(e.h), e.bpp)))?;
    let data = b.get(pick.off..pick.off.checked_add(pick.len)?)?;
    let mut img = if data.starts_with(&[0x89, b'P', b'N', b'G']) { decode_png(data)? } else { decode_dib(data)? };
    // the directory's hot spot is the one Windows uses (clamped into the picture)
    img.hot = (pick.hot.0.min(img.w.saturating_sub(1)), pick.hot.1.min(img.h.saturating_sub(1)));
    Some(img)
}

fn decode_png(data: &[u8]) -> Option<CursorImage> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(data));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().ok()?;
    let info = r.info();
    if info.width == 0 || info.height == 0 || info.width > MAX_SIDE || info.height > MAX_SIDE {
        return None;
    }
    let mut buf = vec![0u8; r.output_buffer_size()?];
    let f = r.next_frame(&mut buf).ok()?;
    let (w, h) = (f.width, f.height);
    let px = (w * h) as usize;
    let rgba = match f.color_type {
        png::ColorType::Rgba => buf[..px * 4].to_vec(),
        png::ColorType::Rgb => buf[..px * 3].as_chunks::<3>().0.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf[..px * 2].as_chunks::<2>().0.iter().flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => buf[..px].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some(CursorImage { w, h, hot: (0, 0), rgba })
}

/// A DIB as `.cur` / `.ico` store it: a BITMAPINFOHEADER, the palette (up to 8 bit), the colour rows (bottom up), then the
/// 1-bit AND mask rows. The header's height counts both bitmaps.
fn decode_dib(d: &[u8]) -> Option<CursorImage> {
    let hdr = u32_at(d, 0)? as usize;
    if hdr < 40 || d.len() < hdr {
        return None;
    }
    let w = i32_at(d, 4)?;
    let h2 = i32_at(d, 8)?;
    let bpp = u16_at(d, 14)? as u32;
    let compression = u32_at(d, 16)?;
    let used = u32_at(d, 32)? as usize;
    // (RLE-compressed pictures - 1 and 2 - are not read)
    if w <= 0 || h2 == 0 || compression == 1 || compression == 2 || compression > 3 || !matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32) {
        return None;
    }
    let (w, h) = (w as u32, (h2.unsigned_abs() / 2).max(1));
    if w > MAX_SIDE || h > MAX_SIDE {
        return None;
    }
    let top_down = h2 < 0;
    let mut p = hdr;
    // BI_BITFIELDS with the plain 40-byte header keeps its three masks right after it (we only need the common layouts)
    let mut masks = None;
    if compression == 3 {
        if hdr == 40 {
            masks = Some((u32_at(d, p)?, u32_at(d, p + 4)?, u32_at(d, p + 8)?));
            p += 12;
        } else {
            masks = Some((u32_at(d, 40)?, u32_at(d, 44)?, u32_at(d, 48)?));
        }
    }
    let colors = if bpp <= 8 { if used == 0 { 1usize << bpp } else { used.min(1usize << bpp) } } else { 0 };
    let pal: Vec<[u8; 3]> = (0..colors).map(|i| d.get(p + i * 4..p + i * 4 + 3).map(|c| [c[2], c[1], c[0]]).unwrap_or([0, 0, 0])).collect();
    p += colors * 4;
    let row = ((w * bpp).div_ceil(32) * 4) as usize;
    let xor_len = row * h as usize;
    let xor = d.get(p..p.checked_add(xor_len)?)?;
    let mrow = (w.div_ceil(32) * 4) as usize;
    let mask = d.get(p + xor_len..p + xor_len + mrow * h as usize);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let mut any_alpha = false;
    for y in 0..h as usize {
        let sy = if top_down { y } else { h as usize - 1 - y };
        let r = &xor[sy * row..sy * row + row];
        for x in 0..w as usize {
            let (rgb, a) = match bpp {
                32 => {
                    let c = &r[x * 4..x * 4 + 4];
                    any_alpha |= c[3] != 0;
                    ([c[2], c[1], c[0]], c[3])
                }
                24 => {
                    let c = &r[x * 3..x * 3 + 3];
                    ([c[2], c[1], c[0]], 255)
                }
                16 => {
                    let v = u16::from_le_bytes([r[x * 2], r[x * 2 + 1]]) as u32;
                    let e = |s: u32, m: u32| ((((v >> s) & m) * 255) / m) as u8;
                    ([e(10, 31), e(5, 31), e(0, 31)], 255)
                }
                8 => (pal.get(r[x] as usize).copied().unwrap_or([0; 3]), 255),
                4 => (pal.get(((r[x / 2] >> (if x % 2 == 0 { 4 } else { 0 })) & 15) as usize).copied().unwrap_or([0; 3]), 255),
                _ => (pal.get(((r[x / 8] >> (7 - x % 8)) & 1) as usize).copied().unwrap_or([0; 3]), 255),
            };
            let o = (y * w as usize + x) * 4;
            rgba[o..o + 3].copy_from_slice(&rgb);
            rgba[o + 3] = a;
        }
    }
    let _ = masks; // (16 / 32 bit masks only matter for odd layouts; the usual 5-5-5 / 8-8-8-8 are read above)
    // a 32-bit picture with an alpha channel is done; anything else takes its transparency from the AND mask
    if !(bpp == 32 && any_alpha) {
        for y in 0..h as usize {
            let sy = if top_down { y } else { h as usize - 1 - y };
            for x in 0..w as usize {
                let set = mask.map(|m| (m[sy * mrow + x / 8] >> (7 - x % 8)) & 1 == 1).unwrap_or(false);
                let o = (y * w as usize + x) * 4;
                if !set {
                    rgba[o + 3] = 255;
                } else if rgba[o..o + 3].iter().map(|&c| c as u32).sum::<u32>() > 3 * 127 {
                    // mask 1 + a light picture pixel = Windows INVERTS what is under it (the old I-beam / cross): drawn white
                    rgba[o..o + 3].copy_from_slice(&[255, 255, 255]);
                    rgba[o + 3] = 255;
                } else {
                    rgba[o + 3] = 0;
                }
            }
        }
    }
    Some(CursorImage { w, h, hot: (0, 0), rgba })
}

/// An animated cursor: the first of its frames (each an `icon` chunk holding a whole `.cur`).
fn decode_ani(b: &[u8], want: u32) -> Option<CursorImage> {
    frames_of(b)?.first().and_then(|f| decode_cur(f, want))
}

/// How many frames an `.ani` holds and the byte slices of each.
fn frames_of(b: &[u8]) -> Option<Vec<&[u8]>> {
    let mut out = Vec::new();
    walk(b, 12, 0, &mut out);
    (!out.is_empty()).then_some(out)
}

/// Frames read at most, and how deep LIST chunks may nest (a damaged / hostile file can nest thousands of them).
const MAX_FRAMES: usize = 4096;
const MAX_DEPTH: u32 = 3;

fn walk<'a>(b: &'a [u8], mut p: usize, depth: u32, out: &mut Vec<&'a [u8]>) {
    if depth > MAX_DEPTH {
        return;
    }
    while p + 8 <= b.len() && out.len() < MAX_FRAMES {
        let id = &b[p..p + 4];
        let len = u32_at(b, p + 4).unwrap_or(0) as usize;
        let body = p + 8;
        let end = body.saturating_add(len).min(b.len());
        if id == b"LIST" {
            // LIST <type> <chunks>: only the frame list matters
            if b.get(body..body + 4) == Some(b"fram") {
                walk(&b[..end], body + 4, depth + 1, out);
            }
        } else if id == b"icon" {
            out.push(&b[body..end]);
        }
        // chunks are word aligned
        p = end.saturating_add(len & 1);
    }
}

/// How many frames the animated cursor has (0 = not an `.ani`).
pub fn ani_frames(b: &[u8]) -> usize {
    if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"ACON" {
        frames_of(b).map(|f| f.len()).unwrap_or(0)
    } else {
        0
    }
}

/// Frame `i` of an `.ani` (or the picture of a `.cur` for any `i`).
pub fn decode_frame(bytes: &[u8], i: usize, want: u32) -> Option<CursorImage> {
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"ACON" {
        let f = frames_of(bytes)?;
        return decode_cur(f.get(i % f.len())?, want);
    }
    decode_cur(bytes, want)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A .cur with one 32 bit DIB entry `s` x `s`: pixel (x, y) = [x, y, 7, alpha].
    pub fn dib_cur(s: u32, alpha: bool, hot: (u16, u16)) -> Vec<u8> {
        let mut dib = Vec::new();
        for v in [40u32, s, s * 2, 1 | (32 << 16), 0, 0, 0, 0, 0, 0] {
            dib.extend_from_slice(&v.to_le_bytes());
        }
        for y in (0..s).rev() {
            for x in 0..s {
                dib.extend_from_slice(&[7, y as u8, x as u8, if alpha { 255 } else { 0 }]);
            }
        }
        // AND mask: 1 = transparent; the first pixel of each row is transparent
        let mrow = s.div_ceil(32) * 4;
        for _ in 0..s {
            let mut r = vec![0u8; mrow as usize];
            r[0] = 0x80;
            dib.extend_from_slice(&r);
        }
        let mut out = vec![0, 0, 2, 0, 1, 0, s as u8, s as u8, 0, 0];
        out.extend_from_slice(&hot.0.to_le_bytes());
        out.extend_from_slice(&hot.1.to_le_bytes());
        out.extend_from_slice(&(dib.len() as u32).to_le_bytes());
        out.extend_from_slice(&22u32.to_le_bytes());
        out.extend_from_slice(&dib);
        out
    }

    #[test]
    fn a_32_bit_cur_with_alpha_reads_its_pixels_top_row_first() {
        let c = dib_cur(4, true, (1, 2));
        let i = decode(&c, 32).unwrap();
        assert_eq!((i.w, i.h, i.hot), (4, 4, (1, 2)));
        // pixel (x=2, y=1): R = x, G = y, B = 7
        let o = (4 + 2) * 4;
        assert_eq!(&i.rgba[o..o + 4], &[2, 1, 7, 255]);
    }

    #[test]
    fn a_picture_without_alpha_takes_its_transparency_from_the_and_mask() {
        let c = dib_cur(4, false, (0, 0));
        let i = decode(&c, 32).unwrap();
        assert_eq!(i.rgba[3], 0, "mask bit set = transparent");
        assert_eq!(i.rgba[7], 255, "mask bit clear = solid");
    }

    #[test]
    fn a_png_entry_is_decoded_too() {
        let mut png_bytes = Vec::new();
        {
            let mut e = png::Encoder::new(&mut png_bytes, 2, 2);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            let mut w = e.write_header().unwrap();
            w.write_image_data(&[1, 2, 3, 255, 4, 5, 6, 128, 7, 8, 9, 0, 10, 11, 12, 255]).unwrap();
        }
        let mut c = vec![0, 0, 2, 0, 1, 0, 2, 2, 0, 0, 1, 0, 1, 0];
        c.extend_from_slice(&(png_bytes.len() as u32).to_le_bytes());
        c.extend_from_slice(&22u32.to_le_bytes());
        c.extend_from_slice(&png_bytes);
        let i = decode(&c, 32).unwrap();
        assert_eq!((i.w, i.h, i.hot), (2, 2, (1, 1)));
        assert_eq!(&i.rgba[4..8], &[4, 5, 6, 128]);
    }

    #[test]
    fn the_size_nearest_the_wanted_one_is_chosen() {
        // two entries: 4 px and 8 px
        let a = dib_cur(4, true, (0, 0));
        let b = dib_cur(8, true, (0, 0));
        let (da, db) = (&a[22..], &b[22..]);
        let mut c = vec![0, 0, 2, 0, 2, 0];
        for (s, d, off) in [(4u8, da, 38), (8u8, db, 38 + da.len())] {
            c.extend_from_slice(&[s, s, 0, 0, 0, 0, 0, 0]);
            c.extend_from_slice(&(d.len() as u32).to_le_bytes());
            c.extend_from_slice(&(off as u32).to_le_bytes());
        }
        c.extend_from_slice(da);
        c.extend_from_slice(db);
        assert_eq!(sizes(&c), vec![(4, 4), (8, 8)]);
        assert_eq!(decode(&c, 4).unwrap().w, 4);
        assert_eq!(decode(&c, 6).unwrap().w, 8, "the next one up, not a smaller one");
        assert_eq!(decode(&c, 64).unwrap().w, 8, "bigger than any: the biggest");
    }

    #[test]
    fn an_ani_gives_its_frames() {
        let f1 = dib_cur(4, true, (0, 0));
        let f2 = dib_cur(6, true, (0, 0));
        let chunk = |c: &[u8]| {
            let mut v = b"icon".to_vec();
            v.extend_from_slice(&(c.len() as u32).to_le_bytes());
            v.extend_from_slice(c);
            if c.len() % 2 == 1 {
                v.push(0);
            }
            v
        };
        let mut list = b"fram".to_vec();
        list.extend(chunk(&f1));
        list.extend(chunk(&f2));
        let mut body = b"ACON".to_vec();
        body.extend_from_slice(b"anih");
        body.extend_from_slice(&36u32.to_le_bytes());
        body.extend_from_slice(&[0u8; 36]);
        body.extend_from_slice(b"LIST");
        body.extend_from_slice(&(list.len() as u32).to_le_bytes());
        body.extend(list);
        let mut ani = b"RIFF".to_vec();
        ani.extend_from_slice(&(body.len() as u32).to_le_bytes());
        ani.extend(body);
        assert_eq!(ani_frames(&ani), 2);
        assert_eq!(decode(&ani, 4).unwrap().w, 4);
        assert_eq!(decode_frame(&ani, 1, 6).unwrap().w, 6);
    }

    #[test]
    fn a_hostile_ani_with_thousands_of_nested_lists_does_not_overflow_the_stack() {
        let mut ani = b"RIFF\0\0\0\0ACON".to_vec();
        for _ in 0..50_000 {
            ani.extend_from_slice(b"LIST\xff\xff\xff\x7ffram");
        }
        let _ = decode(&ani, 32);
        assert!(ani_frames(&ani) <= 4096);
    }

    #[test]
    fn damaged_files_are_none_not_a_panic() {
        assert!(decode(&[], 32).is_none());
        assert!(decode(b"not a cursor at all", 32).is_none());
        let c = dib_cur(4, true, (0, 0));
        for cut in [3, 10, 30, 60, c.len() - 1] {
            let _ = decode(&c[..cut], 32);
        }
        // an entry pointing outside the file
        let mut c2 = c.clone();
        c2[18..22].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&c2, 32).is_none());
    }

    /// Every cursor Windows ships reads (skipped where the folder is missing).
    #[test]
    fn the_cursors_in_the_windows_folder_all_read() {
        let dir = std::path::PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join("Cursors");
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        let (mut ok, mut bad) = (0, Vec::new());
        for e in rd.flatten() {
            let p = e.path();
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if ext != "cur" && ext != "ani" {
                continue;
            }
            match std::fs::read(&p).ok().and_then(|b| decode(&b, 64)) {
                Some(i) if i.rgba.as_chunks::<4>().0.iter().any(|px| px[3] != 0) => ok += 1,
                _ => bad.push(p.file_name().unwrap().to_string_lossy().into_owned()),
            }
        }
        assert!(ok > 20, "only {ok} read");
        assert!(bad.is_empty(), "unreadable or empty: {bad:?}");
    }
}
