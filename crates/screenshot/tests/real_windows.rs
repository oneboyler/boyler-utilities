//! Real Windows, without ever capturing the screen:
//! - the read-only / proof layers refuse file changes (on harmless targets only);
//! - monitors are read (numbers, sizes) — read only;
//! - the real GPU → memory path (staging copy, row pitch, HDR conversion, rotation, content crop) on textures this test makes
//!   itself with known pixels;
//! - Save / gallery / thumbnails / prune with real files, only inside `BoylerUtilities-board\scratch\lane-h\` (skipped if the
//!   board's scratch folder is missing — never a fallback elsewhere);
//! - the drag-out data object carries exactly the files (CF_HDROP) and "Preferred DropEffect" = copy.
//! - (ignored by default) delete moves a scratch file to the real Recycle Bin: `cargo test -p bu-screenshot -- --ignored`.
//!
//! Not run for real anywhere (they would change the user's clipboard or open windows on the screen): clipboard writes,
//! show in folder, open folder, folder picker. Those are tested against the fake.
#![cfg(windows)]

use std::path::{Path, PathBuf};

use bu_screenshot::encode;
use bu_screenshot::real::gpu::{self, Slot};
use bu_screenshot::real::{drag_data_object, RealOs};
use bu_screenshot::{ColorPath, Error, Image, Rotation, ScreenshotOs, Screenshots};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
};

const SCRATCH_PARENT: &str = r"C:\BoylerUtilities-scratch";

/// This test's own folder under scratch\lane-h; removed (only it) when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Option<Self> {
        if !Path::new(SCRATCH_PARENT).is_dir() {
            eprintln!("SKIPPED: {SCRATCH_PARENT} missing (no fallback location is used)");
            return None;
        }
        let dir = Path::new(SCRATCH_PARENT).join("lane-h").join(format!("test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        Some(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn picture(w: u32, h: u32) -> Image {
    let mut img = Image::black(w, h);
    for y in 0..h {
        for x in 0..w {
            img.set_pixel(x, y, [(x * 7) as u8, (y * 5) as u8, (x + y) as u8, 255]);
        }
    }
    img
}

// ---------- refusals ----------

#[test]
fn guarded_layers_refuse_file_changes_on_harmless_targets() {
    // Only targets a broken guard could not turn into a real change: missing paths inside this lane's scratch folder.
    // (Clipboard / Explorer / picker / capture refusals are proven on the guards themselves: real::tests.)
    let Some(sc) = Scratch::new("refuse") else { return };
    let missing = sc.0.join("missing").join("x.png");
    let ro = RealOs::read_only();
    assert!(matches!(ro.write_file(&missing, b"x"), Err(Error::ReadOnly(_))));
    assert!(!missing.exists());
    assert!(matches!(ro.remove_file(&missing), Err(Error::ReadOnly(_))));
    assert!(matches!(ro.recycle(std::slice::from_ref(&missing)), Err(Error::ReadOnly(_))));
    assert!(matches!(ro.show_in_folder(std::slice::from_ref(&missing)), Err(Error::ReadOnly(_))));
    let proof = RealOs::capture_proof();
    assert!(matches!(proof.recycle(std::slice::from_ref(&missing)), Err(Error::ReadOnly(_))));
    assert!(matches!(proof.show_in_folder(std::slice::from_ref(&missing)), Err(Error::ReadOnly(_))));
}
// ---------- monitors (read only) ----------

#[test]
fn real_monitors_are_numbered_and_sized() {
    let mons = RealOs::read_only().monitors().unwrap();
    assert!(!mons.is_empty());
    for (i, m) in mons.iter().enumerate() {
        assert_eq!(m.number, i + 1);
        assert!(m.rect.w >= 640 && m.rect.h >= 480, "monitor {} {:?}", m.number, m.rect);
        assert!(m.device.starts_with(r"\\.\DISPLAY"), "{}", m.device);
        assert!(m.dpi >= 96);
    }
    assert_eq!(mons.iter().filter(|m| m.primary).count(), 1);
    let primary = mons.iter().find(|m| m.primary).unwrap();
    assert_eq!((primary.rect.x, primary.rect.y), (0, 0), "Windows puts the main monitor at 0,0");
}

#[test]
fn default_save_folder_is_found_without_creating_it() {
    let d = RealOs::read_only().default_save_dir().unwrap();
    assert!(d.ends_with("Screenshots"), "{}", d.display());
}

// ---------- the real GPU readback path, on our own textures ----------

fn texture(w: u32, h: u32, format: DXGI_FORMAT, bytes: &[u8], bpp: u32) -> (Slot, ID3D11Texture2D) {
    let (dev, ctx) = gpu::device(None).expect("a D3D11 GPU device");
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let init = D3D11_SUBRESOURCE_DATA { pSysMem: bytes.as_ptr() as *const _, SysMemPitch: w * bpp, SysMemSlicePitch: 0 };
    let mut tex = None;
    unsafe { dev.CreateTexture2D(&desc, Some(&init), Some(&mut tex)) }.expect("CreateTexture2D");
    (Slot::new(1, dev, ctx, Rotation::None, ColorPath::Sdr, None), tex.unwrap())
}

#[test]
fn gpu_readback_of_bgra_is_exact_with_odd_width() {
    // 37 px wide: the staging row pitch is padded, the copy must skip the padding exactly.
    let want = picture(37, 23);
    let mut src = want.bgra.clone();
    for px in src.chunks_mut(4) {
        px[3] = 0; // desktop alpha is undefined; the engine forces 255
    }
    let (mut slot, tex) = texture(37, 23, DXGI_FORMAT_B8G8R8A8_UNORM, &src, 4);
    slot.copy(&tex).unwrap();
    let f = slot.read().unwrap();
    assert_eq!(f.image, want);
    assert_eq!(f.monitor, 1);
}

#[test]
fn gpu_readback_of_rgba_swaps_channels() {
    let want = picture(16, 8);
    let mut src = want.bgra.clone();
    for px in src.chunks_mut(4) {
        px.swap(0, 2);
    }
    let (mut slot, tex) = texture(16, 8, DXGI_FORMAT_R8G8B8A8_UNORM, &src, 4);
    slot.copy(&tex).unwrap();
    assert_eq!(slot.read().unwrap().image, want);
}

#[test]
fn gpu_readback_turns_and_crops() {
    let want = picture(20, 10);
    let (mut slot, tex) = texture(20, 10, DXGI_FORMAT_B8G8R8A8_UNORM, &want.bgra, 4);
    slot.rotation = Rotation::Cw90;
    slot.copy(&tex).unwrap();
    assert_eq!(slot.read().unwrap().image, want.rotated(Rotation::Cw90));
    slot.rotation = Rotation::None;
    slot.content = Some((12, 6));
    assert_eq!(slot.read().unwrap().image, want.crop(&bu_screenshot::Rect::new(0, 0, 12, 6)).unwrap());
}

#[test]
fn gpu_readback_of_hdr_float_converts_with_sdr_white() {
    // 3×1: scRGB (1,1,1) = white at 80 nits, (0.5, 0, 2.0) = 188 / 0 / clipped, (0,0,0).
    let halfs: [[u16; 4]; 3] = [[0x3c00, 0x3c00, 0x3c00, 0x3c00], [0x3800, 0, 0x4000, 0x3c00], [0, 0, 0, 0x3c00]];
    let bytes: Vec<u8> = halfs.iter().flatten().flat_map(|h| h.to_le_bytes()).collect();
    let (mut slot, tex) = texture(3, 1, DXGI_FORMAT_R16G16B16A16_FLOAT, &bytes, 8);
    slot.hdr = Some(bu_screenshot::image::HdrToSdr::new(80.0));
    slot.copy(&tex).unwrap();
    let img = slot.read().unwrap().image;
    assert_eq!(img.pixel(0, 0), [255, 255, 255, 255]);
    assert_eq!(img.pixel(1, 0), [255, 0, 188, 255], "B = 2.0 clipped, G = 0, R = 0.5 -> 188");
    assert_eq!(img.pixel(2, 0), [0, 0, 0, 255]);
}

// ---------- real files in the scratch folder ----------

#[test]
fn save_gallery_thumbnail_and_prune_with_real_files() {
    let Some(sc) = Scratch::new("save") else { return };
    let shots_dir = sc.0.join("shots");
    std::fs::create_dir_all(&shots_dir).unwrap();
    let s = Screenshots::new(RealOs::new(), sc.0.join("data"));
    s.set_save_dir(&shots_dir).unwrap();
    assert_eq!(s.save_dir().unwrap(), shots_dir);

    let img = picture(300, 170);
    let a = s.save(&img).unwrap();
    let b = s.save(&img).unwrap();
    assert!(a.path.starts_with(&shots_dir) && b.path.starts_with(&shots_dir));
    assert_ne!(a.path, b.path);
    let name = a.path.file_name().unwrap().to_string_lossy().to_string();
    assert!(name.starts_with("Screenshot ") && name.ends_with(".png"), "{name}");
    assert_eq!(encode::decode_png(&std::fs::read(&a.path).unwrap()).unwrap(), img, "exact pixels on disk");
    assert_eq!(s.gallery().unwrap().iter().map(|x| x.id).collect::<Vec<_>>(), vec![b.id, a.id]);
    let t = s.thumbnail(a.id).unwrap();
    assert_eq!((t.width, t.height), (300, 170), "small shots are not enlarged");
    assert!(!std::fs::read_dir(&shots_dir).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().ends_with(".bu-tmp")),
        "no temp files left");

    std::fs::remove_file(&a.path).unwrap(); // "deleted in Explorer"
    assert_eq!(s.gallery().unwrap().iter().map(|x| x.id).collect::<Vec<_>>(), vec![b.id]);
    assert!(!sc.0.join("data").join("thumbs").join(format!("{}.png", a.id)).exists());
}

#[test]
fn set_save_dir_refuses_a_missing_folder() {
    let Some(sc) = Scratch::new("badpath") else { return };
    let s = Screenshots::new(RealOs::new(), sc.0.join("data"));
    assert!(matches!(s.set_save_dir(&sc.0.join("does-not-exist")), Err(Error::NotAFolder(_))));
}

#[test]
fn drag_data_object_carries_the_files_and_asks_for_copy() {
    let Some(sc) = Scratch::new("drag") else { return };
    let sub = sc.0.join("other folder");
    std::fs::create_dir_all(&sub).unwrap();
    let files = vec![sc.0.join("Screenshot A.png"), sub.join("Screenshot B.png")];
    for f in &files {
        std::fs::write(f, encode::png_bytes(&picture(4, 4), encode::PngLevel::Fast).unwrap()).unwrap();
    }
    let files2 = files.clone();
    // OLE needs a single-threaded apartment: run on a fresh thread.
    let (dropped, effect) = std::thread::spawn(move || unsafe {
        use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
        use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
        use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
        use windows::Win32::System::Ole::{OleInitialize, OleUninitialize, ReleaseStgMedium, CF_HDROP};
        use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
        OleInitialize(None).unwrap();
        let obj = drag_data_object(&files2).unwrap();
        let fe = |cf: u16| FORMATETC { cfFormat: cf, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
        let mut sm = obj.GetData(&fe(CF_HDROP.0)).unwrap();
        let hdrop = HDROP(sm.u.hGlobal.0);
        let n = DragQueryFileW(hdrop, u32::MAX, None);
        let mut dropped = Vec::new();
        for i in 0..n {
            let len = DragQueryFileW(hdrop, i, None) as usize;
            let mut buf = vec![0u16; len + 1];
            DragQueryFileW(hdrop, i, Some(&mut buf));
            dropped.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
        }
        ReleaseStgMedium(&mut sm);
        let cf = RegisterClipboardFormatW(windows::core::w!("Preferred DropEffect")) as u16;
        let mut sm = obj.GetData(&fe(cf)).unwrap();
        let p = GlobalLock(sm.u.hGlobal) as *const u32;
        let effect = *p;
        let _ = GlobalUnlock(sm.u.hGlobal);
        ReleaseStgMedium(&mut sm);
        drop(obj);
        OleUninitialize();
        (dropped, effect)
    })
    .join()
    .unwrap();
    assert_eq!(dropped, files, "exactly the dragged files, from two different folders");
    assert_eq!(effect, 1, "DROPEFFECT_COPY: Explorer copies, never moves the shot away");
}

#[test]
#[ignore = "moves one tiny scratch file to the user's real Recycle Bin; run on purpose with --ignored"]
fn delete_moves_a_scratch_file_to_the_recycle_bin() {
    let Some(sc) = Scratch::new("recycle") else { return };
    let shots_dir = sc.0.join("shots");
    std::fs::create_dir_all(&shots_dir).unwrap();
    let s = Screenshots::new(RealOs::new(), sc.0.join("data"));
    s.set_save_dir(&shots_dir).unwrap();
    let shot = s.save(&picture(8, 8)).unwrap();
    assert!(shot.path.exists());
    s.delete(&[shot.id]).unwrap();
    assert!(!shot.path.exists(), "gone from the folder");
    assert!(s.gallery().unwrap().is_empty());
}
