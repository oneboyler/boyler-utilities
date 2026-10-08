//! A captured picture in memory (BGRA, 8 bit, alpha always 255) and the pixel work the engine does on it: crop, place monitors
//! side by side, undo a monitor's rotation, the HDR → SDR conversion and thumbnails. All exact: no resampling except thumbnails.

use crate::geom::{Rect, Rotation};

/// Top-down rows, 4 bytes per pixel in B, G, R, A order (what Windows' desktop surfaces use), no row padding.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl std::fmt::Debug for Image {
    // Never print pixel data (it may be a picture of the user's screen).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image({}x{})", self.width, self.height)
    }
}

impl Image {
    /// A black, opaque picture.
    pub fn black(width: u32, height: u32) -> Self {
        let mut bgra = vec![0u8; width as usize * height as usize * 4];
        for px in bgra.as_chunks_mut::<4>().0.iter_mut() {
            px[3] = 255;
        }
        Image { width, height, bgra }
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [self.bgra[i], self.bgra[i + 1], self.bgra[i + 2], self.bgra[i + 3]]
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, p: [u8; 4]) {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        self.bgra[i..i + 4].copy_from_slice(&p);
    }

    /// Copies a part out. `r` is in this picture's own pixels and must lie inside it (else `None`).
    pub fn crop(&self, r: &Rect) -> Option<Image> {
        if r.is_empty() || r.x < 0 || r.y < 0 || r.right() > self.width as i64 || r.bottom() > self.height as i64 {
            return None;
        }
        let (w, row) = (r.w as usize, self.width as usize * 4);
        let mut out = Vec::with_capacity(w * r.h as usize * 4);
        for y in r.y as usize..r.y as usize + r.h as usize {
            let s = y * row + r.x as usize * 4;
            out.extend_from_slice(&self.bgra[s..s + w * 4]);
        }
        Some(Image { width: r.w, height: r.h, bgra: out })
    }

    /// Pastes `src` with its top-left at (`x`, `y`) in this picture; parts outside are cut off.
    pub fn blit(&mut self, src: &Image, x: i64, y: i64) {
        let dst = Rect::new(0, 0, self.width, self.height);
        let placed = Rect::new(x as i32, y as i32, src.width, src.height);
        let Some(r) = dst.intersect(&placed) else { return };
        let (sx, sy) = ((r.x as i64 - x) as usize, (r.y as i64 - y) as usize);
        let n = r.w as usize * 4;
        for row in 0..r.h as usize {
            let s = ((sy + row) * src.width as usize + sx) * 4;
            let d = ((r.y as usize + row) * self.width as usize + r.x as usize) * 4;
            self.bgra[d..d + n].copy_from_slice(&src.bgra[s..s + n]);
        }
    }

    /// Turns the picture by the monitor's rotation so it shows the desktop the way the user sees it. Desktop Duplication hands
    /// over the scan-out image (not turned); Windows.Graphics.Capture already turns it, so it never calls this. The direction
    /// follows Microsoft's DXGIDesktopDuplication sample (DisplayManager.cpp SetDirtyVert: ROTATE90 maps texture (x, y) to desktop
    /// (W - y, x), i.e. turn clockwise; ROTATE270 maps it to (y, H - x)). Not yet seen on a real rotated monitor.
    pub fn rotated(&self, rot: Rotation) -> Image {
        let (w, h) = (self.width as usize, self.height as usize);
        match rot {
            Rotation::None => self.clone(),
            Rotation::Cw180 => {
                let mut out = Vec::with_capacity(self.bgra.len());
                for px in self.bgra.as_chunks::<4>().0.iter().rev() {
                    out.extend_from_slice(px);
                }
                Image { width: self.width, height: self.height, bgra: out }
            }
            Rotation::Cw90 | Rotation::Cw270 => {
                // New picture is h wide, w tall.
                let mut out = vec![0u8; self.bgra.len()];
                for ny in 0..w {
                    for nx in 0..h {
                        // 90° clockwise: new(nx, ny) = old(ny, h-1-nx). 270°: new(nx, ny) = old(w-1-ny, nx).
                        let (ox, oy) = if rot == Rotation::Cw90 { (ny, h - 1 - nx) } else { (w - 1 - ny, nx) };
                        let s = (oy * w + ox) * 4;
                        let d = (ny * h + nx) * 4;
                        out[d..d + 4].copy_from_slice(&self.bgra[s..s + 4]);
                    }
                }
                Image { width: self.height, height: self.width, bgra: out }
            }
        }
    }

    /// A small copy that fits inside `max_w`×`max_h` keeping the aspect ratio ("contain"), never larger than the original.
    /// Each output pixel is the average of the source pixels it covers (box filter) — sharp enough for a 16:9 gallery tile.
    pub fn thumbnail(&self, max_w: u32, max_h: u32) -> Image {
        let scale = (max_w as f64 / self.width as f64).min(max_h as f64 / self.height as f64).min(1.0);
        let tw = ((self.width as f64 * scale).round() as u32).max(1);
        let th = ((self.height as f64 * scale).round() as u32).max(1);
        let mut out = Vec::with_capacity(tw as usize * th as usize * 4);
        for ty in 0..th {
            let y0 = (ty as u64 * self.height as u64 / th as u64) as u32;
            let y1 = (((ty + 1) as u64 * self.height as u64 / th as u64) as u32).max(y0 + 1);
            for tx in 0..tw {
                let x0 = (tx as u64 * self.width as u64 / tw as u64) as u32;
                let x1 = (((tx + 1) as u64 * self.width as u64 / tw as u64) as u32).max(x0 + 1);
                let mut acc = [0u64; 3];
                for y in y0..y1 {
                    let row = (y as usize * self.width as usize) * 4;
                    for x in x0..x1 {
                        let i = row + x as usize * 4;
                        acc[0] += self.bgra[i] as u64;
                        acc[1] += self.bgra[i + 1] as u64;
                        acc[2] += self.bgra[i + 2] as u64;
                    }
                }
                let n = ((y1 - y0) as u64) * ((x1 - x0) as u64);
                out.extend_from_slice(&[((acc[0] + n / 2) / n) as u8, ((acc[1] + n / 2) / n) as u8, ((acc[2] + n / 2) / n) as u8, 255]);
            }
        }
        Image { width: tw, height: th, bgra: out }
    }

    /// How many pixels are not pure black — the proof run reports this number (never the picture) to show a frame really
    /// arrived and was not an empty surface.
    pub fn non_black_pixels(&self) -> usize {
        self.bgra.as_chunks::<4>().0.iter().filter(|p| p[0] | p[1] | p[2] != 0).count()
    }
}

/// Copies rows of 8-bit BGRA from a mapped GPU surface (rows `pitch` bytes apart) and forces alpha to 255 (the desktop's alpha
/// byte is undefined).
pub fn from_bgra_rows(width: u32, height: u32, src: &[u8], pitch: usize) -> Image {
    let n = width as usize * 4;
    let mut bgra = Vec::with_capacity(n * height as usize);
    for y in 0..height as usize {
        bgra.extend_from_slice(&src[y * pitch..y * pitch + n]);
    }
    for px in bgra.as_chunks_mut::<4>().0.iter_mut() {
        px[3] = 255;
    }
    Image { width, height, bgra }
}

/// HDR monitors: Windows composes the desktop in scRGB (linear light, Rec.709 primaries, 1.0 = 80 nits, 16-bit floats).
/// This turns such a surface into normal 8-bit sRGB the way SDR content is shown on that monitor: divide by the monitor's
/// "SDR content brightness" white, keep 0…1 (HDR highlights brighter than SDR white clip to white; colours outside sRGB clip),
/// then the sRGB curve. Built as one 65 536-entry table per white level (indexed by the raw half-float bits), so a 4K frame is
/// one table look-up per channel.
pub struct HdrToSdr {
    lut: Vec<u8>,
}

impl HdrToSdr {
    pub fn new(sdr_white_nits: f32) -> Self {
        let white = (sdr_white_nits / 80.0).max(0.01);
        let lut = (0..=u16::MAX)
            .map(|h| {
                let v = half_to_f32(h) / white;
                let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
                (srgb_encode(v) * 255.0 + 0.5) as u8
            })
            .collect();
        HdrToSdr { lut }
    }

    /// One channel value (raw half-float bits) → 8-bit sRGB.
    pub fn channel(&self, half_bits: u16) -> u8 {
        self.lut[half_bits as usize]
    }

    /// Converts rows of RGBA half floats (8 bytes per pixel, rows `pitch` bytes apart) into a BGRA image.
    pub fn convert_rows(&self, width: u32, height: u32, src: &[u8], pitch: usize) -> Image {
        let mut bgra = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height as usize {
            let row = &src[y * pitch..y * pitch + width as usize * 8];
            for px in row.as_chunks::<8>().0.iter() {
                let r = u16::from_le_bytes([px[0], px[1]]);
                let g = u16::from_le_bytes([px[2], px[3]]);
                let b = u16::from_le_bytes([px[4], px[5]]);
                bgra.extend_from_slice(&[self.lut[b as usize], self.lut[g as usize], self.lut[r as usize], 255]);
            }
        }
        Image { width, height, bgra }
    }
}

/// IEEE 754 half → f32.
pub fn half_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) as u32) << 31;
    let exp = ((h >> 10) & 0x1f) as u32;
    let man = (h & 0x3ff) as u32;
    let bits = match (exp, man) {
        (0, 0) => sign,
        (0, m) => {
            // Subnormal: normalise.
            let mut e: i32 = -14;
            let mut m = m;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | (((e + 127) as u32) << 23) | ((m & 0x3ff) << 13)
        }
        (0x1f, 0) => sign | 0x7f80_0000,
        (0x1f, m) => sign | 0x7f80_0000 | (m << 13),
        (e, m) => sign | ((e + 112) << 23) | (m << 13),
    };
    f32::from_bits(bits)
}

/// The sRGB transfer curve (linear 0…1 → encoded 0…1).
pub fn srgb_encode(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Places monitor pictures into one desktop picture. `parts` are (where the monitor sits on the desktop, its picture);
/// `area` is the region of the desktop wanted. Desktop pixels with no monitor stay black.
pub fn compose(area: &Rect, parts: &[(Rect, &Image)]) -> Image {
    let mut out = Image::black(area.w, area.h);
    for (at, img) in parts {
        out.blit(img, at.x as i64 - area.x as i64, at.y as i64 - area.y as i64);
    }
    out
}
