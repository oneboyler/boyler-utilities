//! The bytes that leave the engine: PNG files (and PNG on the clipboard) and the DIB (device-independent bitmap) the clipboard
//! also gets, so apps that only read bitmaps (Paint, older apps) paste it too.

use crate::error::{Error, Result};
use crate::image::Image;

/// PNG speed / size trade-off. The engine uses [`PngLevel::Fast`] (measured in the report).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngLevel {
    Fast,
    Balanced,
    High,
}

/// Encodes a picture as an 8-bit RGB PNG (no alpha channel: a screenshot is opaque, and it keeps the file smaller). Lossless:
/// decoding gives back exactly the same pixels.
pub fn png_bytes(img: &Image, level: PngLevel) -> Result<Vec<u8>> {
    let mut rgb = Vec::with_capacity(img.width as usize * img.height as usize * 3);
    for px in img.bgra.as_chunks::<4>().0.iter() {
        rgb.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width, img.height);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(match level {
            PngLevel::Fast => png::Compression::Fast,
            PngLevel::Balanced => png::Compression::Balanced,
            PngLevel::High => png::Compression::High,
        });
        let mut w = enc.write_header().map_err(|e| Error::BadData(format!("png header: {e}")))?;
        w.write_image_data(&rgb).map_err(|e| Error::BadData(format!("png data: {e}")))?;
        w.finish().map_err(|e| Error::BadData(format!("png finish: {e}")))?;
    }
    Ok(out)
}

/// Decodes a PNG (8-bit RGB or RGBA, as this engine writes them; others are refused) back into a picture.
pub fn decode_png(bytes: &[u8]) -> Result<Image> {
    let dec = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut r = dec.read_info().map_err(|e| Error::BadData(format!("png: {e}")))?;
    let size = r.output_buffer_size().ok_or_else(|| Error::BadData("png: too large".into()))?;
    let mut buf = vec![0u8; size];
    let info = r.next_frame(&mut buf).map_err(|e| Error::BadData(format!("png: {e}")))?;
    if info.bit_depth != png::BitDepth::Eight {
        return Err(Error::BadData("png: not 8-bit".into()));
    }
    let (w, h) = (info.width, info.height);
    let mut bgra = Vec::with_capacity(w as usize * h as usize * 4);
    match info.color_type {
        png::ColorType::Rgb => {
            for y in 0..h as usize {
                let row = &buf[y * info.line_size..y * info.line_size + w as usize * 3];
                for p in row.as_chunks::<3>().0.iter() {
                    bgra.extend_from_slice(&[p[2], p[1], p[0], 255]);
                }
            }
        }
        png::ColorType::Rgba => {
            for y in 0..h as usize {
                let row = &buf[y * info.line_size..y * info.line_size + w as usize * 4];
                for p in row.as_chunks::<4>().0.iter() {
                    bgra.extend_from_slice(&[p[2], p[1], p[0], 255]);
                }
            }
        }
        other => return Err(Error::BadData(format!("png: colour type {other:?}"))),
    }
    Ok(Image { width: w, height: h, bgra })
}

/// CF_HDROP bytes (a DROPFILES block): the 20-byte header (`pFiles` = 20, point 0/0, `fNC` 0, `fWide` 1) followed by each path
/// as UTF-16 with a NUL, and one more NUL at the end.
pub fn dropfiles_bytes(paths: &[std::path::PathBuf]) -> Vec<u8> {
    let mut out = Vec::new();
    for v in [20u32, 0, 0, 0, 1] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for p in paths {
        for u in p.to_string_lossy().encode_utf16().chain(std::iter::once(0)) {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// The clipboard's CF_DIB: a 40-byte BITMAPINFOHEADER followed by 32-bit BI_RGB pixels, bottom row first (the DIB rule),
/// alpha 255. Windows makes CF_BITMAP and CF_DIBV5 from it for apps that ask for those.
pub fn dib_bytes(img: &Image) -> Vec<u8> {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut out = Vec::with_capacity(40 + w * h * 4);
    out.extend_from_slice(&40u32.to_le_bytes()); // biSize
    out.extend_from_slice(&(img.width as i32).to_le_bytes()); // biWidth
    out.extend_from_slice(&(img.height as i32).to_le_bytes()); // biHeight > 0 = bottom-up
    out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    out.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    out.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    out.extend_from_slice(&((w * h * 4) as u32).to_le_bytes()); // biSizeImage
    out.extend_from_slice(&3780i32.to_le_bytes()); // biXPelsPerMeter (96 dpi)
    out.extend_from_slice(&3780i32.to_le_bytes()); // biYPelsPerMeter
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant
    for y in (0..h).rev() {
        out.extend_from_slice(&img.bgra[y * w * 4..(y + 1) * w * 4]);
    }
    out
}
