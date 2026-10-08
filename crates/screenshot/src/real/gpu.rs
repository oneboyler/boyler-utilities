//! D3D11 devices and the GPU → memory copy. A captured frame lives on the GPU; it is copied into a CPU-readable "staging"
//! texture of the same size and format, mapped, and turned into an 8-bit BGRA [`Image`] — 8-bit surfaces row by row as they
//! are, 16-bit float (HDR) surfaces through [`HdrToSdr`]. No scaling anywhere, so the pixels are exact.

use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_R16G16B16A16_FLOAT,
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::IDXGIAdapter1;

use super::Ctx;
use crate::error::{Error, Result};
use crate::geom::Rotation;
use crate::image::{self, HdrToSdr, Image};
use crate::os::{ColorPath, MonitorFrame};

/// A D3D11 device on `adapter` (the GPU that drives the monitor — Desktop Duplication requires that), or on the default
/// hardware GPU when `None`.
pub fn device(adapter: Option<&IDXGIAdapter1>) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut dev = None;
    let mut ctx = None;
    let driver = if adapter.is_some() { D3D_DRIVER_TYPE_UNKNOWN } else { D3D_DRIVER_TYPE_HARDWARE };
    unsafe {
        match adapter {
            Some(a) => D3D11CreateDevice(
                a,
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut dev),
                None,
                Some(&mut ctx),
            ),
            None => D3D11CreateDevice(
                None::<&windows::Win32::Graphics::Dxgi::IDXGIAdapter>,
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut dev),
                None,
                Some(&mut ctx),
            ),
        }
    }
    .ctx("D3D11CreateDevice")?;
    match (dev, ctx) {
        (Some(d), Some(c)) => Ok((d, c)),
        _ => Err(Error::os("D3D11CreateDevice (no device)", 0)),
    }
}

/// A CPU-readable copy slot for frames of one size and format.
pub struct Staging {
    pub tex: ID3D11Texture2D,
    pub width: u32,
    pub height: u32,
    pub format: DXGI_FORMAT,
}

fn desc_of(tex: &ID3D11Texture2D) -> D3D11_TEXTURE2D_DESC {
    let mut d = D3D11_TEXTURE2D_DESC::default();
    unsafe { tex.GetDesc(&mut d) };
    d
}

/// Copies `src` (a frame on the GPU) into `slot`, (re)making the slot if the size or format changed. The copy is queued on
/// the GPU; [`read`] waits for it.
pub fn copy_to_staging(dev: &ID3D11Device, ctx: &ID3D11DeviceContext, src: &ID3D11Texture2D, slot: &mut Option<Staging>) -> Result<()> {
    let d = desc_of(src);
    let fits = matches!(slot, Some(s) if s.width == d.Width && s.height == d.Height && s.format == d.Format);
    if !fits {
        let sd = D3D11_TEXTURE2D_DESC {
            Width: d.Width,
            Height: d.Height,
            MipLevels: 1,
            ArraySize: 1,
            Format: d.Format,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut t = None;
        unsafe { dev.CreateTexture2D(&sd, None, Some(&mut t)) }.ctx("CreateTexture2D (staging)")?;
        let tex = t.ok_or_else(|| Error::os("CreateTexture2D (none)", 0))?;
        *slot = Some(Staging { tex, width: d.Width, height: d.Height, format: d.Format });
    }
    if let Some(s) = slot {
        unsafe { ctx.CopyResource(&s.tex, src) };
    }
    Ok(())
}

/// Reads a staging slot into an 8-bit BGRA image: only the top-left `content` part (Windows.Graphics.Capture surfaces can be
/// larger than the monitor after a resolution change), turned by `rot`, HDR surfaces converted with `hdr`.
pub fn read(ctx: &ID3D11DeviceContext, s: &Staging, content: (u32, u32), rot: Rotation, hdr: Option<&HdrToSdr>) -> Result<Image> {
    let (w, h) = (content.0.min(s.width), content.1.min(s.height));
    let mut m = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe { ctx.Map(&s.tex, 0, D3D11_MAP_READ, 0, Some(&mut m)) }.ctx("Map (staging)")?;
    let pitch = m.RowPitch as usize;
    let len = pitch * (h.max(1) as usize - 1) + w as usize * if s.format == DXGI_FORMAT_R16G16B16A16_FLOAT { 8 } else { 4 };
    let bytes = unsafe { std::slice::from_raw_parts(m.pData as *const u8, len) };
    let img = match s.format {
        f if f == DXGI_FORMAT_B8G8R8A8_UNORM || f == DXGI_FORMAT_B8G8R8A8_UNORM_SRGB => Ok(image::from_bgra_rows(w, h, bytes, pitch)),
        f if f == DXGI_FORMAT_R8G8B8A8_UNORM || f == DXGI_FORMAT_R8G8B8A8_UNORM_SRGB => {
            let mut img = image::from_bgra_rows(w, h, bytes, pitch);
            for px in img.bgra.as_chunks_mut::<4>().0.iter_mut() {
                px.swap(0, 2);
            }
            Ok(img)
        }
        f if f == DXGI_FORMAT_R16G16B16A16_FLOAT => {
            let fallback;
            let conv = match hdr {
                Some(c) => c,
                None => {
                    fallback = HdrToSdr::new(80.0);
                    &fallback
                }
            };
            Ok(conv.convert_rows(w, h, bytes, pitch))
        }
        f => Err(Error::os("unsupported desktop surface format", f.0 as i64)),
    };
    unsafe { ctx.Unmap(&s.tex, 0) };
    Ok(img?.rotated(rot))
}

/// One monitor's GPU side: its device, the CPU-readable copy of the newest frame, and how to turn it into pixels.
pub struct Slot {
    pub number: usize,
    pub dev: ID3D11Device,
    pub ctx: ID3D11DeviceContext,
    pub rotation: Rotation,
    pub color: ColorPath,
    pub hdr: Option<HdrToSdr>,
    pub staging: Option<Staging>,
    /// The picture part of the surface (Windows.Graphics.Capture); `None` = the whole surface.
    pub content: Option<(u32, u32)>,
}

impl Slot {
    pub fn new(number: usize, dev: ID3D11Device, ctx: ID3D11DeviceContext, rotation: Rotation, color: ColorPath, hdr: Option<HdrToSdr>) -> Self {
        Slot { number, dev, ctx, rotation, color, hdr, staging: None, content: None }
    }

    pub fn copy(&mut self, tex: &ID3D11Texture2D) -> Result<()> {
        copy_to_staging(&self.dev, &self.ctx, tex, &mut self.staging)
    }

    pub fn read(&self) -> Result<MonitorFrame> {
        let s = self.staging.as_ref().ok_or(Error::NoFrame { monitor: self.number, ms: super::FRAME_TIMEOUT.as_millis() as u32 })?;
        let content = self.content.unwrap_or((s.width, s.height));
        let image = read(&self.ctx, s, content, self.rotation, self.hdr.as_ref())?;
        Ok(MonitorFrame { monitor: self.number, image, color: self.color })
    }
}

/// Lets GPU objects (not marked thread-safe by the bindings) cross to the thread that uses them. Every user keeps the rule:
/// a capture object is used by one thread only, a slot's device context only under the slot's mutex.
pub struct SendCell<T>(pub T);
unsafe impl<T> Send for SendCell<T> {}
unsafe impl<T> Sync for SendCell<T> {}