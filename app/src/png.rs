//! Pixels in and out: Skia surfaces to premultiplied BGRA, PNGs saved / loaded with WIC (test pictures, snapshots,
//! icon generation), and pictures handed to Skia.

use skia_safe as sk;
use windows::core::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;

/// Premultiplied BGRA.
#[derive(Debug)]
pub struct Pixels {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

/// Copy a Skia surface's pixels (premultiplied BGRA, sRGB).
pub fn from_surface(s: &mut sk::Surface) -> Pixels {
    let (w, h) = (s.width() as u32, s.height() as u32);
    let ii = sk::ImageInfo::new((w as i32, h as i32), sk::ColorType::BGRA8888, sk::AlphaType::Premul, Some(sk::ColorSpace::new_srgb()));
    let mut data = vec![0u8; (w * h * 4) as usize];
    s.read_pixels(&ii, &mut data, (w * 4) as usize, (0, 0));
    Pixels { w, h, data }
}

/// Hand pixels to Skia as an image.
pub fn to_image(p: &Pixels) -> Option<sk::Image> {
    let ii = sk::ImageInfo::new((p.w as i32, p.h as i32), sk::ColorType::BGRA8888, sk::AlphaType::Premul, Some(sk::ColorSpace::new_srgb()));
    sk::images::raster_from_data(&ii, sk::Data::new_copy(&p.data), (p.w * 4) as usize)
}

pub fn save_png(p: &Pixels, path: &str) -> Result<()> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = wic.CreateStream()?;
        let wpath: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        stream.InitializeFromFilename(PCWSTR(wpath.as_ptr()), 0x40000000 /* GENERIC_WRITE */)?;
        let enc = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        enc.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        enc.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.unwrap();
        frame.Initialize(None)?;
        frame.SetSize(p.w, p.h)?;
        let mut fmt = GUID_WICPixelFormat32bppPBGRA;
        frame.SetPixelFormat(&mut fmt)?;
        if fmt == GUID_WICPixelFormat32bppPBGRA {
            frame.WritePixels(p.h, p.w * 4, &p.data)?;
        } else {
            // the encoder wants straight alpha: un-premultiply
            let mut s = p.data.clone();
            for px in s.chunks_mut(4) {
                let a = px[3] as u32;
                if a > 0 && a < 255 {
                    for c in 0..3 {
                        px[c] = ((px[c] as u32 * 255 + a / 2) / a).min(255) as u8;
                    }
                }
            }
            frame.WritePixels(p.h, p.w * 4, &s)?;
        }
        frame.Commit()?;
        enc.Commit()?;
        Ok(())
    }
}

/// Load a PNG as premultiplied BGRA.
pub fn load_png(path: &str) -> Result<Pixels> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let wpath: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let dec = wic.CreateDecoderFromFilename(PCWSTR(wpath.as_ptr()), None, windows::Win32::Foundation::GENERIC_READ, WICDecodeMetadataCacheOnDemand)?;
        let f = dec.GetFrame(0)?;
        let conv = wic.CreateFormatConverter()?;
        conv.Initialize(&f, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let (mut w, mut h) = (0, 0);
        conv.GetSize(&mut w, &mut h)?;
        let mut data = vec![0u8; (w * h * 4) as usize];
        conv.CopyPixels(std::ptr::null(), w * 4, &mut data)?;
        Ok(Pixels { w, h, data })
    }
}
