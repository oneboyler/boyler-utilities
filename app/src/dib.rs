//! Order 049: a small layered window's pixels kept between paints (the timers' pills, the mic icon): one memory DC + one
//! DIB, made again only when the size changes, that Skia draws into directly - before, every paint made a new Skia
//! surface, copied it out, made a new DIB + DC, copied again and deleted them.

use skia_safe as sk;
use windows::Win32::Foundation::{HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

pub struct Dib {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    pub w: i32,
    pub h: i32,
}

impl Dib {
    pub fn new() -> Dib {
        let dc = unsafe {
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen));
            ReleaseDC(None, screen);
            dc
        };
        Dib { dc, bmp: HBITMAP::default(), old: HGDIOBJ::default(), bits: std::ptr::null_mut(), w: 0, h: 0 }
    }

    /// A surface over the pixels (`w` x `h`, cleared to transparent), the same format as `gfx::new_surface`. The DIB is
    /// made again only when the size changed.
    pub fn surface(&mut self, w: i32, h: i32) -> Option<sk::Surface> {
        let (w, h) = (w.max(1), h.max(1));
        if (w, h) != (self.w, self.h) || self.bits.is_null() {
            unsafe {
                let bi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..Default::default() },
                    ..Default::default()
                };
                let mut bits = std::ptr::null_mut();
                let hb = CreateDIBSection(Some(self.dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
                let old = SelectObject(self.dc, hb.into());
                if self.bmp.is_invalid() {
                    self.old = old;
                } else {
                    let _ = DeleteObject(self.bmp.into());
                }
                self.bmp = hb;
                self.bits = bits as *mut u8;
                self.w = w;
                self.h = h;
            }
        }
        let info = sk::ImageInfo::new((w, h), sk::ColorType::BGRA8888, sk::AlphaType::Premul, Some(sk::ColorSpace::new_srgb()));
        let px = unsafe { std::slice::from_raw_parts_mut(self.bits, (w * h * 4) as usize) };
        let mut s = sk::surfaces::wrap_pixels(&info, px, (w * 4) as usize, Some(&crate::gfx::surface_props())).map(|s| unsafe { s.release() })?;
        s.canvas().clear(sk::Color::TRANSPARENT);
        Some(s)
    }

    /// Hand the pixels to the window at `pos` (screen px).
    pub fn show(&self, hwnd: HWND, pos: POINT) {
        unsafe {
            let sz = SIZE { cx: self.w, cy: self.h };
            let src = POINT::default();
            let blend = BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: 1 };
            let _ = UpdateLayeredWindow(hwnd, None, Some(&pos), Some(&sz), Some(self.dc), Some(&src), Default::default(), Some(&blend), ULW_ALPHA);
        }
    }
}

impl Default for Dib {
    fn default() -> Self {
        Dib::new()
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            if !self.old.is_invalid() {
                SelectObject(self.dc, self.old);
            }
            if !self.bmp.is_invalid() {
                let _ = DeleteObject(self.bmp.into());
            }
            let _ = DeleteDC(self.dc);
        }
    }
}
