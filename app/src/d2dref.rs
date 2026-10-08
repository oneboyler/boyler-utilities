//! Order 013, option 3's pixel check (`gpudiff` test command): the very Direct2D effect chain the compositor runs for
//! `BU_GLASS=gpu` (crop to the window box, mirrored edges, Gaussian blur at full quality, the colour matrix), run
//! off-screen with Direct2D on a desktop picture and read back, to compare with Skia's backdrop recipe on the same
//! picture. (Windows' compositor runs these effects itself; whether it keeps 8-bit or higher precision between them is
//! **unclear**, so this is the closest off-screen stand-in, not a picture of the screen.)

use windows::core::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;

use crate::png::Pixels;

fn bytes<T>(v: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v as *const T as *const u8, std::mem::size_of::<T>()) }
}

/// The window's box (x, y, w, h, device pixels) of `desk`, blurred by Direct2D with sigma, then saturate x brightness.
pub fn blur_d2d(d3d: &ID3D11Device, desk: &Pixels, bx: (i32, i32, i32, i32), sigma: f32, sat: f32, bright: f32) -> Result<Pixels> {
    unsafe {
        let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let dxgi: IDXGIDevice = d3d.cast()?;
        let dev = factory.CreateDevice(&dxgi)?;
        let dc = dev.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
        let pf = D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED };
        let props = |o: D2D1_BITMAP_OPTIONS| D2D1_BITMAP_PROPERTIES1 { pixelFormat: pf, dpiX: 96.0, dpiY: 96.0, bitmapOptions: o, colorContext: std::mem::ManuallyDrop::new(None) };
        let src = dc.CreateBitmap(D2D_SIZE_U { width: desk.w, height: desk.h }, Some(desk.data.as_ptr() as *const _), desk.w * 4, &props(D2D1_BITMAP_OPTIONS_NONE))?;
        let (x, y, w, h) = bx;
        let crop = dc.CreateEffect(&CLSID_D2D1Crop)?;
        crop.SetInput(0, &src, true);
        let r = [x as f32, y as f32, (x + w) as f32, (y + h) as f32];
        crop.SetValue(0, D2D1_PROPERTY_TYPE_VECTOR4, bytes(&r))?;
        crop.SetValue(1, D2D1_PROPERTY_TYPE_ENUM, bytes(&1u32))?;
        let border = dc.CreateEffect(&CLSID_D2D1Border)?;
        border.SetInput(0, &crop.GetOutput()?, true);
        border.SetValue(0, D2D1_PROPERTY_TYPE_ENUM, bytes(&2u32))?;
        border.SetValue(1, D2D1_PROPERTY_TYPE_ENUM, bytes(&2u32))?;
        let blur = dc.CreateEffect(&CLSID_D2D1GaussianBlur)?;
        blur.SetInput(0, &border.GetOutput()?, true);
        blur.SetValue(0, D2D1_PROPERTY_TYPE_FLOAT, bytes(&sigma))?;
        blur.SetValue(1, D2D1_PROPERTY_TYPE_ENUM, bytes(&2u32))?;
        blur.SetValue(2, D2D1_PROPERTY_TYPE_ENUM, bytes(&1u32))?;
        let cm = dc.CreateEffect(&CLSID_D2D1ColorMatrix)?;
        cm.SetInput(0, &blur.GetOutput()?, true);
        let mut m = crate::effects::saturate_matrix(sat);
        for row in 0..3 {
            for col in 0..3 {
                m[row * 4 + col] *= bright;
            }
        }
        cm.SetValue(0, D2D1_PROPERTY_TYPE_MATRIX_5X4, bytes(&m))?;
        cm.SetValue(1, D2D1_PROPERTY_TYPE_ENUM, bytes(&1u32))?;
        cm.SetValue(2, D2D1_PROPERTY_TYPE_BOOL, bytes(&1u32))?;
        let target = dc.CreateBitmap(D2D_SIZE_U { width: w as u32, height: h as u32 }, None, 0, &props(D2D1_BITMAP_OPTIONS_TARGET))?;
        dc.SetTarget(&target);
        dc.BeginDraw();
        dc.Clear(None);
        let out = cm.GetOutput()?;
        dc.DrawImage(&out, Some(&windows_numerics::Vector2 { X: -(x as f32), Y: -(y as f32) }), None, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_COMPOSITE_MODE_SOURCE_COPY);
        dc.EndDraw(None, None)?;
        let cpu = dc.CreateBitmap(D2D_SIZE_U { width: w as u32, height: h as u32 }, None, 0, &props(D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW))?;
        cpu.CopyFromBitmap(None, &target, None)?;
        let map = cpu.Map(D2D1_MAP_OPTIONS_READ)?;
        let mut data = vec![0u8; (w * h * 4) as usize];
        for row in 0..h as usize {
            let s = std::slice::from_raw_parts(map.bits.add(row * map.pitch as usize), w as usize * 4);
            data[row * w as usize * 4..(row + 1) * w as usize * 4].copy_from_slice(s);
        }
        cpu.Unmap()?;
        Ok(Pixels { w: w as u32, h: h as u32, data })
    }
}

/// Order 041 (adaptive glass): every channel of `px` through Direct2D's own Blend effect against a flat grey (Flood) -
/// the same effects, CLSIDs and property values the compositor gets in comp.rs (`effects::clamp_level`).
#[cfg(test)]
pub fn clamp_d2d(d3d: &ID3D11Device, px: &Pixels, mode: u32, level: f32) -> Result<Pixels> {
    unsafe {
        let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let dxgi: IDXGIDevice = d3d.cast()?;
        let dev = factory.CreateDevice(&dxgi)?;
        let dc = dev.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
        let pf = D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED };
        let props = |o: D2D1_BITMAP_OPTIONS| D2D1_BITMAP_PROPERTIES1 { pixelFormat: pf, dpiX: 96.0, dpiY: 96.0, bitmapOptions: o, colorContext: std::mem::ManuallyDrop::new(None) };
        let (w, h) = (px.w, px.h);
        let src = dc.CreateBitmap(D2D_SIZE_U { width: w, height: h }, Some(px.data.as_ptr() as *const _), w * 4, &props(D2D1_BITMAP_OPTIONS_NONE))?;
        let flood = dc.CreateEffect(&crate::effects::CLSID_FLOOD)?;
        flood.SetValue(0, D2D1_PROPERTY_TYPE_VECTOR4, bytes(&[level, level, level, 1.0f32]))?;
        let blend = dc.CreateEffect(&crate::effects::CLSID_BLEND)?;
        blend.SetInput(0, &src, true);
        blend.SetInput(1, &flood.GetOutput()?, true);
        blend.SetValue(0, D2D1_PROPERTY_TYPE_ENUM, bytes(&mode))?;
        let target = dc.CreateBitmap(D2D_SIZE_U { width: w, height: h }, None, 0, &props(D2D1_BITMAP_OPTIONS_TARGET))?;
        dc.SetTarget(&target);
        dc.BeginDraw();
        dc.Clear(None);
        dc.DrawImage(&blend.GetOutput()?, None, Some(&D2D_RECT_F { left: 0.0, top: 0.0, right: w as f32, bottom: h as f32 }), D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_COMPOSITE_MODE_SOURCE_COPY);
        dc.EndDraw(None, None)?;
        let cpu = dc.CreateBitmap(D2D_SIZE_U { width: w, height: h }, None, 0, &props(D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW))?;
        cpu.CopyFromBitmap(None, &target, None)?;
        let map = cpu.Map(D2D1_MAP_OPTIONS_READ)?;
        let mut data = vec![0u8; (w * h * 4) as usize];
        for row in 0..h as usize {
            let s = std::slice::from_raw_parts(map.bits.add(row * map.pitch as usize), w as usize * 4);
            data[row * w as usize * 4..(row + 1) * w as usize * 4].copy_from_slice(s);
        }
        cpu.Unmap()?;
        Ok(Pixels { w, h, data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The compositor's adaptive-glass step really is "at most" / "at least" the level per channel: Direct2D's Blend effect
    /// run with `effects::BLEND_DARKEN` / `BLEND_LIGHTEN` on every 8-bit value (off-screen: nothing is shown).
    #[test]
    fn the_glass_cap_is_a_per_channel_min_and_the_floor_a_max() {
        let d3d = crate::present::new_d3d().expect("a Direct3D device");
        // 256 pixels: blue = i, green = 255 - i, red = i / 2, opaque
        let data: Vec<u8> = (0..256u32).flat_map(|i| [i as u8, (255 - i) as u8, (i / 2) as u8, 255]).collect();
        let px = Pixels { w: 256, h: 1, data };
        let level = 0.5f32;
        let l8 = (level * 255.0).round() as i32;
        for (mode, f) in [(crate::effects::BLEND_DARKEN, i32::min as fn(i32, i32) -> i32), (crate::effects::BLEND_LIGHTEN, i32::max)] {
            let out = clamp_d2d(&d3d, &px, mode, level).expect("Direct2D blend");
            for (i, (a, b)) in px.data.chunks(4).zip(out.data.chunks(4)).enumerate() {
                for c in 0..3 {
                    let want = f(a[c] as i32, l8);
                    assert!((b[c] as i32 - want).abs() <= 1, "mode {mode} pixel {i} channel {c}: {} -> {} (want {want})", a[c], b[c]);
                }
                assert_eq!(b[3], 255);
            }
        }
    }
}
