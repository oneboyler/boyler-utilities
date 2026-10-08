//! An app's name, icon and colour for the mixer (Lane A's method, bakeoff-native `appinfo.rs`): the name is the exe's
//! FileDescription (like Windows' own mixer), else the session's own name, else the exe name; the icon is the shell's
//! own 48 px image (matches Explorer); the colour comes from the icon ([`crate::colour::icon_colour`]).
//!
//! Reading an icon took ~1 s on a first open (Lane A, measured), so icons are read on a helper thread that lives only
//! while there is work: the first call returns the name with a plain grey tile, a later call has the icon.

use crate::colour::icon_colour;
use crate::model::{AppLook, Icon, SessionInfo};
use crate::os::GREY;
use std::collections::HashMap;
use std::sync::Mutex;
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{DeleteObject, HPALETTE};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY};

struct Work {
    names: HashMap<String, String>,
    /// exe path → icon (None = no icon / failed)
    icons: HashMap<String, Option<Icon>>,
    jobs: Vec<String>,
    running: bool,
}

static WORK: Mutex<Option<Work>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Work) -> R) -> R {
    let mut g = WORK.lock().unwrap_or_else(|p| p.into_inner());
    f(g.get_or_insert_with(|| Work { names: HashMap::new(), icons: HashMap::new(), jobs: Vec::new(), running: false }))
}

pub fn look(s: &SessionInfo) -> AppLook {
    if s.system {
        return AppLook { name: "System sounds".into(), icon: None, colour: GREY.0, colour2: GREY.1 };
    }
    let path = s.exe_path.clone();
    let name = {
        let cached = with(|w| w.names.get(&path).cloned());
        match cached {
            Some(n) if !path.is_empty() => n,
            _ => {
                let n = name_of(s);
                if !path.is_empty() {
                    with(|w| w.names.insert(path.clone(), n.clone()));
                }
                n
            }
        }
    };
    if path.is_empty() {
        return AppLook { name, icon: None, colour: GREY.0, colour2: GREY.1 };
    }
    let icon = with(|w| match w.icons.get(&path) {
        Some(i) => Some(i.clone()),
        None => {
            if !w.jobs.contains(&path) {
                w.jobs.push(path.clone());
                if !w.running {
                    w.running = true;
                    if std::thread::Builder::new().name("bu-audio-icons".into()).spawn(helper).is_err() {
                        w.running = false;
                    }
                }
            }
            None
        }
    });
    match icon.flatten() {
        Some(i) => {
            let (c, c2) = icon_colour(&i);
            AppLook { name, icon: Some(i), colour: c, colour2: c2 }
        }
        None => AppLook { name, icon: None, colour: GREY.0, colour2: GREY.1 },
    }
}

fn helper() {
    // the shell's image factory wants a single-threaded apartment
    let ok = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    loop {
        let job = with(|w| match w.jobs.pop() {
            Some(j) => Some(j),
            None => {
                w.running = false;
                None
            }
        });
        let Some(path) = job else { break };
        let px = icon_pixels(&path, 48);
        with(|w| w.icons.insert(path, px));
    }
    if ok {
        unsafe { CoUninitialize() };
    }
}

fn name_of(s: &SessionInfo) -> String {
    if let Some(d) = file_description(&s.exe_path) {
        return d;
    }
    if !s.display_name.is_empty() && !s.display_name.starts_with('@') {
        return s.display_name.clone();
    }
    std::path::Path::new(&s.exe_path)
        .file_stem()
        .map(|x| x.to_string_lossy().to_string())
        .filter(|x| !x.is_empty())
        .unwrap_or_else(|| format!("App {}", s.pid))
}

fn file_description(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    unsafe {
        let w = super::wide(path);
        let size = GetFileVersionInfoSizeW(PCWSTR(w.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(w.as_ptr()), None, size, data.as_mut_ptr() as *mut _).ok()?;
        let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        let q = super::wide("\\VarFileInfo\\Translation");
        let mut tr = (0x0409u16, 0x04b0u16);
        if VerQueryValueW(data.as_ptr() as *const _, PCWSTR(q.as_ptr()), &mut p, &mut len).as_bool() && len >= 4 && !p.is_null() {
            let a = p as *const u16;
            tr = (*a, *a.add(1));
        }
        let k = super::wide(&format!("\\StringFileInfo\\{:04x}{:04x}\\FileDescription", tr.0, tr.1));
        if VerQueryValueW(data.as_ptr() as *const _, PCWSTR(k.as_ptr()), &mut p, &mut len).as_bool() && len > 1 && !p.is_null() {
            let s = std::slice::from_raw_parts(p as *const u16, len as usize);
            let s = String::from_utf16_lossy(s).trim_end_matches('\0').trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
        None
    }
}

/// The exe's icon as premultiplied BGRA (the shell's own image).
fn icon_pixels(path: &str, size: i32) -> Option<Icon> {
    unsafe {
        let w = super::wide(path);
        let item: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None).ok()?;
        let hb = item.GetImage(SIZE { cx: size, cy: size }, SIIGBF_ICONONLY).ok()?;
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let bmp = wic.CreateBitmapFromHBITMAP(hb, HPALETTE::default(), WICBitmapUsePremultipliedAlpha).ok();
        let _ = DeleteObject(hb.into());
        let bmp = bmp?;
        let conv = wic.CreateFormatConverter().ok()?;
        conv.Initialize(&bmp.cast::<IWICBitmapSource>().ok()?, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
            .ok()?;
        let (mut bw, mut bh) = (0, 0);
        conv.GetSize(&mut bw, &mut bh).ok()?;
        let mut data = vec![0u8; (bw * bh * 4) as usize];
        conv.CopyPixels(std::ptr::null(), bw * 4, &mut data).ok()?;
        if data.chunks(4).all(|p| p[3] == 0) {
            return None;
        }
        Some(Icon { w: bw, h: bh, bgra: data })
    }
}
