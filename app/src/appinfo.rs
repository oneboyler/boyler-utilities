//! What an app in the mixer looks like: its name (the exe's description, like Windows' own mixer), its real icon,
//! and one colour picked from that icon for its slider and level bar.

use std::sync::Arc;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::DeleteObject;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Shell::*;

use crate::audio::Tile;
use crate::gfx::Rgba;
use crate::png::Pixels;

pub fn system_sounds() -> (String, Tile, Rgba, Rgba) {
    (
        "System sounds".into(),
        Tile::Glyph { glyph: "abell", a: Rgba::hex(0xa2abbd), b: Rgba::hex(0x6c7487) },
        Rgba::hex(0x8d96a8),
        Rgba::hex(0xc6ccd8),
    )
}

fn generic(name: String) -> (String, Tile, Rgba, Rgba) {
    (name, Tile::Glyph { glyph: "appw", a: Rgba::hex(0xa2abbd), b: Rgba::hex(0x6c7487) }, Rgba::hex(0x8d96a8), Rgba::hex(0xc6ccd8))
}

/// names / icons / colours already worked out, by exe path (kept while the menu is closed: a few KB per app; the
/// audio worker that asks is a new thread on every open, so this is process-wide)
static CACHE: std::sync::Mutex<Option<std::collections::HashMap<String, (String, Tile, Rgba, Rgba)>>> = std::sync::Mutex::new(None);

/// Icons read on a helper thread (the shell's icon reading took ~1 s on a first open, measured — the menu must not
/// wait for it): jobs (key, name, exe path) in, (key, name, icon) out. The thread only lives while there is work.
struct Work {
    jobs: Vec<(String, String, String)>,
    done: Vec<(String, String, Option<Pixels>)>,
    pending: Vec<String>,
    running: bool,
}
static WORK: std::sync::Mutex<Work> = std::sync::Mutex::new(Work { jobs: Vec::new(), done: Vec::new(), pending: Vec::new(), running: false });

/// Name, tile and colours of an app; the bool is false while its icon is still being read (a plain tile meanwhile:
/// ask again on the next refresh).
pub fn describe_cached(pid: u32, system: bool, display: &str) -> ((String, Tile, Rgba, Rgba), bool) {
    if system {
        return (system_sounds(), true);
    }
    let path = exe_path(pid).unwrap_or_default();
    let key = format!("{}|{}", if path.is_empty() { format!("pid{}", pid) } else { path.clone() }, display);
    // take in what the helper has finished
    let done = std::mem::take(&mut WORK.lock().unwrap().done);
    for (k, n, px) in done {
        let v = match px {
            Some(px) => {
                let (c, c2) = icon_colour(&px);
                (n, Tile::Icon(Arc::new(px)), c, c2)
            }
            None => generic(n),
        };
        WORK.lock().unwrap().pending.retain(|x| *x != k);
        CACHE.lock().unwrap().get_or_insert_with(Default::default).insert(k, v);
    }
    if let Some(v) = CACHE.lock().unwrap().get_or_insert_with(Default::default).get(&key).cloned() {
        return (v, true);
    }
    let name = app_name(pid, &path, display);
    let mut w = WORK.lock().unwrap();
    if !w.pending.contains(&key) {
        w.pending.push(key.clone());
        w.jobs.push((key, name.clone(), path));
        if !w.running {
            w.running = true;
            std::thread::spawn(helper);
        }
    }
    (generic(name), false)
}

fn helper() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    loop {
        let job = {
            let mut w = WORK.lock().unwrap();
            match w.jobs.pop() {
                Some(j) => j,
                None => {
                    w.running = false;
                    break;
                }
            }
        };
        let px = icon_pixels(&job.2, 48);
        WORK.lock().unwrap().done.push((job.0, job.1, px));
    }
    unsafe {
        CoUninitialize();
    }
}

fn app_name(pid: u32, path: &str, display: &str) -> String {
    let stem = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut name = if !display.is_empty() && !display.starts_with('@') { display.to_string() } else { String::new() };
    if name.is_empty() {
        name = file_description(path).unwrap_or_default();
    }
    if name.is_empty() {
        name = if stem.is_empty() { format!("App {}", pid) } else { stem };
    }
    name
}

fn exe_path(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n);
        let _ = CloseHandle(h);
        r.ok()?;
        Some(String::from_utf16_lossy(&buf[..n as usize]))
    }
}

fn file_description(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    unsafe {
        let w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let size = GetFileVersionInfoSizeW(PCWSTR(w.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(w.as_ptr()), None, size, data.as_mut_ptr() as *mut _).ok()?;
        let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        let q: Vec<u16> = "\\VarFileInfo\\Translation\0".encode_utf16().collect();
        let mut tr = (0x0409u16, 0x04b0u16);
        if VerQueryValueW(data.as_ptr() as *const _, PCWSTR(q.as_ptr()), &mut p, &mut len).as_bool() && len >= 4 {
            let a = p as *const u16;
            tr = (*a, *a.add(1));
        }
        let key = format!("\\StringFileInfo\\{:04x}{:04x}\\FileDescription\0", tr.0, tr.1);
        let k: Vec<u16> = key.encode_utf16().collect();
        if VerQueryValueW(data.as_ptr() as *const _, PCWSTR(k.as_ptr()), &mut p, &mut len).as_bool() && len > 1 {
            let s = std::slice::from_raw_parts(p as *const u16, len as usize);
            let s = String::from_utf16_lossy(s).trim_end_matches('\0').trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
        None
    }
}

/// The exe's icon as premultiplied BGRA (the shell's own image, so it matches Explorer).
/// Order 025: `pub` - pages show real app icons with it (Apps' DisplayIcon, Search's app results); it asks the shell,
/// so call it off the UI thread (the job runner), never while building a page.
pub fn icon_pixels(path: &str, size: i32) -> Option<Pixels> {
    if path.is_empty() {
        return None;
    }
    unsafe {
        let w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let item: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None).ok()?;
        let hb = item.GetImage(SIZE { cx: size, cy: size }, SIIGBF_ICONONLY).ok()?;
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let bmp = wic.CreateBitmapFromHBITMAP(hb, windows::Win32::Graphics::Gdi::HPALETTE::default(), WICBitmapUsePremultipliedAlpha).ok();
        let _ = DeleteObject(hb.into());
        let bmp = bmp?;
        let conv = wic.CreateFormatConverter().ok()?;
        conv.Initialize(&bmp, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).ok()?;
        let (mut bw, mut bh) = (0, 0);
        conv.GetSize(&mut bw, &mut bh).ok()?;
        let mut data = vec![0u8; (bw * bh * 4) as usize];
        conv.CopyPixels(std::ptr::null(), bw * 4, &mut data).ok()?;
        if data.chunks(4).all(|p| p[3] == 0) {
            return None;
        }
        Some(Pixels { w: bw, h: bh, data })
    }
}

/// One colour from the icon: the most common saturated hue (weighted by how vivid and opaque each pixel is),
/// and a lighter tint of it for the end of the level bar. A grey icon gets the drawing's grey.
pub fn icon_colour(p: &Pixels) -> (Rgba, Rgba) {
    let mut bins = [(0f64, 0f64, 0f64, 0f64); 36];
    for px in p.data.chunks(4) {
        let a = px[3] as f64 / 255.0;
        if a < 0.5 {
            continue;
        }
        let (b, g, r) = (px[0] as f64 / 255.0 / a, px[1] as f64 / 255.0 / a, px[2] as f64 / 255.0 / a);
        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        let s = if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
        if s < 0.25 || mx < 0.25 {
            continue;
        }
        let h = if mx == r {
            ((g - b) / (mx - mn)).rem_euclid(6.0)
        } else if mx == g {
            (b - r) / (mx - mn) + 2.0
        } else {
            (r - g) / (mx - mn) + 4.0
        } * 60.0;
        let wgt = s * a * mx;
        let i = ((h / 10.0) as usize).min(35);
        bins[i].0 += r * wgt;
        bins[i].1 += g * wgt;
        bins[i].2 += b * wgt;
        bins[i].3 += wgt;
    }
    // the strongest hue, merged with its two neighbours
    let score = |i: usize| bins[i].3 + 0.5 * (bins[(i + 35) % 36].3 + bins[(i + 1) % 36].3);
    let best = (0..36).max_by(|a, b| score(*a).partial_cmp(&score(*b)).unwrap()).unwrap_or(0);
    let mut acc = (0.0, 0.0, 0.0, 0.0);
    for j in [(best + 35) % 36, best, (best + 1) % 36] {
        acc.0 += bins[j].0;
        acc.1 += bins[j].1;
        acc.2 += bins[j].2;
        acc.3 += bins[j].3;
    }
    if acc.3 < 1.0 {
        return (Rgba::hex(0x8d96a8), Rgba::hex(0xc6ccd8));
    }
    let c = Rgba((acc.0 / acc.3) as f32, (acc.1 / acc.3) as f32, (acc.2 / acc.3) as f32, 1.0);
    // keep it readable on dark glass: lift very dark colours a little
    let l = 0.2126 * c.0 + 0.7152 * c.1 + 0.0722 * c.2;
    let c = if l < 0.28 { c.mix(Rgba(1.0, 1.0, 1.0, 1.0), (0.28 - l) / 0.72) } else { c };
    let c2 = c.mix(Rgba(1.0, 1.0, 1.0, 1.0), 0.45);
    (c, c2)
}
