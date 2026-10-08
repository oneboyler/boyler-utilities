//! The Glass cursor set (Order 040): the built-in files, putting them into the app's folder (inside the lane's scratch
//! folder `BoylerUtilities-board\scratch\040\`, removed by a guard), picking Glass against the FAKE registry, and Windows
//! itself loading and drawing every file OFF SCREEN (LoadImageW + GetIconInfo + DrawIconEx into a memory DC, then
//! DestroyCursor). Nothing here ever calls SetSystemCursor or SPI_SETCURSORS: the owner's real cursors are never touched.

use bu_mouse::cursors::*;
use bu_mouse::fake::FakeOs;
use bu_mouse::glass::FILES;
use bu_mouse::os::{Hive, RegValue};
use bu_mouse::{AppDirs, Mouse};
use std::path::{Path, PathBuf};

const SCRATCH_PARENT: &str = r"C:\BoylerUtilities-scratch";

/// Removes its own folder (only inside scratch\040) when the test ends, pass or fail.
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        if self.0.starts_with(Path::new(SCRATCH_PARENT).join("040")) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

fn scratch(name: &str) -> Option<Scratch> {
    if !Path::new(SCRATCH_PARENT).is_dir() {
        eprintln!("SKIPPED: no scratch folder {SCRATCH_PARENT}");
        return None;
    }
    let d = Path::new(SCRATCH_PARENT).join("040").join(format!("test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).ok()?;
    Some(Scratch(d))
}

fn assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/assets/cursors/glass")
}

fn animated(r: WinRole) -> bool {
    matches!(r, WinRole::AppStarting | WinRole::Wait)
}

#[test]
fn one_built_in_file_per_role_and_it_is_the_repo_file() {
    assert_eq!(FILES.len(), WinRole::ALL.len());
    for (r, (name, bytes)) in WinRole::ALL.iter().zip(FILES) {
        let want = format!("{}.{}", r.reg_name().to_ascii_lowercase(), if animated(*r) { "ani" } else { "cur" });
        assert_eq!(name, want);
        assert!(is_cursor_bytes(bytes), "{name}");
        assert_eq!(std::fs::read(assets().join(name)).unwrap(), bytes, "{name}: the exe carries the repo's file");
    }
}

#[test]
fn install_writes_missing_and_changed_files_only() {
    let Some(s) = scratch("install") else { return };
    let m = Mouse::new(FakeOs::new(), AppDirs::new(s.0.join("appdata")));
    assert!(m.glass_set().is_empty());
    assert_eq!(m.install_glass().unwrap(), 17);
    let set = m.glass_set();
    assert_eq!(set.len(), 17);
    assert_eq!(set.get(&WinRole::Wait).map(String::as_str), Some("wait.ani"));
    assert_eq!(set.get(&WinRole::Arrow).map(String::as_str), Some("arrow.cur"));
    // all there: nothing written again
    assert_eq!(m.install_glass().unwrap(), 0);
    // an older / damaged file is replaced, a missing one put back, other files are left alone
    let dir = s.0.join(r"appdata\cursors\glass");
    std::fs::write(dir.join("hand.cur"), b"old").unwrap();
    std::fs::remove_file(dir.join("wait.ani")).unwrap();
    std::fs::write(dir.join("notes.txt"), b"mine").unwrap();
    assert_eq!(m.install_glass().unwrap(), 2);
    assert_eq!(std::fs::read(dir.join("hand.cur")).unwrap(), FILES[14].1);
    assert_eq!(std::fs::read(dir.join("wait.ani")).unwrap(), FILES[3].1);
    assert_eq!(std::fs::read(dir.join("notes.txt")).unwrap(), b"mine");
    assert!(!dir.join("hand.cur.new").exists());
}

#[test]
fn picking_glass_writes_the_glass_files_into_the_fake_registry() {
    let Some(s) = scratch("pick") else { return };
    let mut m = Mouse::new(FakeOs::new(), AppDirs::new(s.0.join("appdata")));
    // before the files exist, Glass has nothing for any role
    assert!(!m.set_has(&SetId::Glass, Role::Normal));
    m.install_glass().unwrap();
    let dir = s.0.join(r"appdata\cursors\glass");
    for role in Role::ALL {
        assert!(m.set_has(&SetId::Glass, role), "{role:?}");
    }
    let before = m.cursor_snapshot().unwrap();
    let reg = |m: &Mouse<FakeOs>, n: &str| m.os().reg_get(Hive::Hkcu, CURSORS_KEY, n).cloned();
    let path = |f: &str| dir.join(f).to_string_lossy().into_owned();

    m.set_role(Role::Normal, SetId::Glass).unwrap();
    assert_eq!(reg(&m, "Arrow"), Some(RegValue::ExpandSz(path("arrow.cur"))));
    assert!(m.os().log.contains(&format!("set_system_cursor 32512 {}", path("arrow.cur"))), "{:?}", m.os().log);
    m.set_role(Role::Busy, SetId::Glass).unwrap();
    m.set_role(Role::Working, SetId::Glass).unwrap();
    m.set_role(Role::Text, SetId::Glass).unwrap();
    m.set_role(Role::Link, SetId::Glass).unwrap();
    m.set_role(Role::Move, SetId::Glass).unwrap();
    m.set_role(Role::Resize, SetId::Glass).unwrap();
    for (n, f) in [
        ("Wait", "wait.ani"),
        ("AppStarting", "appstarting.ani"),
        ("IBeam", "ibeam.cur"),
        ("Hand", "hand.cur"),
        ("SizeAll", "sizeall.cur"),
        ("SizeNS", "sizens.cur"),
        ("SizeWE", "sizewe.cur"),
        ("SizeNWSE", "sizenwse.cur"),
        ("SizeNESW", "sizenesw.cur"),
    ] {
        assert_eq!(reg(&m, n), Some(RegValue::ExpandSz(path(f))), "{n}");
    }
    // the roles a bubble doesn't set keep Windows' cursors
    assert_eq!(reg(&m, "Help"), Some(RegValue::ExpandSz(r"C:\Windows\cursors\aero_helpsel.cur".into())));
    // the page reads every bubble back as Glass, and Glass is what "Matches your other cursors" offers now
    let st = m.cursors().unwrap();
    assert!(st.roles.iter().all(|r| r.set == SetId::Glass), "{:?}", st.roles);
    assert_eq!(m.suggestion(Role::Normal).unwrap(), Some(SetId::Glass));
    // hovering Glass in a picker only shows it (no registry write)
    let writes = m.os().log.iter().filter(|l| l.starts_with("reg_write")).count();
    m.preview_cursor(Role::Text, &SetId::Glass).unwrap();
    assert_eq!(m.os().log.iter().filter(|l| l.starts_with("reg_write")).count(), writes);
    assert_eq!(m.os().log.last().unwrap(), &format!("set_system_cursor 32513 {}", path("ibeam.cur")));
    // undo puts back exactly what was there before the last change; going back role by role ends at the start
    m.undo_cursors().unwrap();
    assert_eq!(reg(&m, "SizeNS"), before.roles.iter().find(|(r, _)| *r == WinRole::SizeNS).unwrap().1.clone());
    // the whole look as one text (the change log) puts the start back too
    let mut f = Mouse::new(FakeOs::new(), AppDirs::new(s.0.join("appdata")));
    let start = f.cursor_look_text().unwrap();
    f.set_role(Role::Busy, SetId::Glass).unwrap();
    f.restore_cursor_look(&start).unwrap();
    assert_eq!(f.cursor_snapshot().unwrap(), before);
}

// ------------------------------------------------------------------------------------------------ Windows loads them

/// The hotspot the SVG asks for (`data-hotspot`, 32-unit grid) at `px` - the pixel that holds that point.
fn svg_hotspot(stem: &str, px: u32) -> (u32, u32) {
    let svg = std::fs::read_to_string(assets().join("svg").join(format!("{stem}.svg"))).unwrap();
    let i = svg.find("data-hotspot=\"").unwrap() + 14;
    let v: Vec<f32> = svg[i..i + svg[i..].find('"').unwrap()].split_whitespace().map(|x| x.parse().unwrap()).collect();
    let k = px as f32 / 32.0;
    ((v[0] * k).floor() as u32, (v[1] * k).floor() as u32)
}

/// A .cur's image of `px` as straight RGBA (top row first) + its hotspot, decoded here from the file (BMP or PNG entry).
fn cur_image(cur: &[u8], px: u32) -> (Vec<[u8; 4]>, (u32, u32)) {
    let le16 = |o: usize| u16::from_le_bytes([cur[o], cur[o + 1]]) as u32;
    let le32 = |o: usize| u32::from_le_bytes([cur[o], cur[o + 1], cur[o + 2], cur[o + 3]]) as usize;
    let n = le16(4) as usize;
    let e = (0..n).map(|i| 6 + 16 * i).find(|&e| cur[e] as u32 == px).unwrap_or_else(|| panic!("no {px} px entry"));
    let hot = (le16(e + 4), le16(e + 6));
    let data = &cur[le32(e + 12)..le32(e + 12) + le32(e + 8)];
    if data.starts_with(b"\x89PNG") {
        let mut r = png::Decoder::new(std::io::Cursor::new(data)).read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height, info.color_type), (px, px, png::ColorType::Rgba));
        (buf.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect(), hot)
    } else {
        // BITMAPINFOHEADER (40) + BGRA rows bottom-up (+ the AND mask, not needed: the alpha says it all)
        let px_us = px as usize;
        let mut out = vec![[0u8; 4]; px_us * px_us];
        for y in 0..px_us {
            for x in 0..px_us {
                let o = 40 + ((px_us - 1 - y) * px_us + x) * 4;
                out[y * px_us + x] = [data[o + 2], data[o + 1], data[o], data[o + 3]];
            }
        }
        (out, hot)
    }
}

/// An .ani's first frame (the first `icon` chunk of LIST 'fram') - a whole .cur.
fn ani_first_frame(ani: &[u8]) -> &[u8] {
    let i = ani.windows(4).position(|w| w == b"fram").unwrap() + 4;
    assert_eq!(&ani[i..i + 4], b"icon");
    let len = u32::from_le_bytes([ani[i + 4], ani[i + 5], ani[i + 6], ani[i + 7]]) as usize;
    &ani[i + 8..i + 8 + len]
}

#[cfg(windows)]
mod win {
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    pub fn wide(p: &std::path::Path) -> Vec<u16> {
        p.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect()
    }

    /// LoadImageW(LR_LOADFROMFILE) at `px`: (hotspot, colour bitmap size, is a cursor), then the cursor drawn with
    /// DrawIconEx into a `px` x `px` memory DIB over black and over white (BGRA, top row first). DestroyCursor at the end.
    pub struct Loaded {
        pub hot: (u32, u32),
        pub bmp: (i32, i32),
        pub is_cursor: bool,
        pub on_black: Vec<[u8; 4]>,
        pub on_white: Vec<[u8; 4]>,
    }

    pub fn load(path: &std::path::Path, px: i32) -> Loaded {
        let w = wide(path);
        unsafe {
            let h = LoadImageW(None, PCWSTR(w.as_ptr()), IMAGE_CURSOR, px, px, LR_LOADFROMFILE).expect("LoadImageW");
            assert!(!h.is_invalid(), "{} at {px}: null handle", path.display());
            let cur = HCURSOR(h.0);
            let mut ii = ICONINFO::default();
            GetIconInfo(HICON(cur.0), &mut ii).expect("GetIconInfo");
            let mut bm = BITMAP::default();
            let got = GetObjectW(HGDIOBJ(ii.hbmColor.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _));
            assert!(got > 0, "colour bitmap");
            let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
            let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
            let draw = |bg: u8| -> Vec<[u8; 4]> {
                let dc = CreateCompatibleDC(None);
                let bi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: px,
                        biHeight: -px,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
                let dib = CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0).expect("CreateDIBSection");
                let old = SelectObject(dc, HGDIOBJ(dib.0));
                let n = (px * px) as usize;
                let buf = std::slice::from_raw_parts_mut(bits as *mut u8, n * 4);
                buf.fill(bg);
                DrawIconEx(dc, 0, 0, HICON(cur.0), px, px, 0, None, DI_NORMAL).expect("DrawIconEx");
                let _ = GdiFlush();
                let out = buf.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
                SelectObject(dc, old);
                let _ = DeleteObject(HGDIOBJ(dib.0));
                let _ = DeleteDC(dc);
                out
            };
            let on_black = draw(0);
            let on_white = draw(255);
            let l = Loaded { hot: (ii.xHotspot, ii.yHotspot), bmp: (bm.bmWidth, bm.bmHeight), is_cursor: !ii.fIcon.as_bool(), on_black, on_white };
            DestroyCursor(cur).expect("DestroyCursor");
            l
        }
    }

    /// LoadCursorFromFileW at Windows' own size: a handle comes back (then destroyed).
    pub fn load_default(path: &std::path::Path) -> bool {
        let w = wide(path);
        unsafe {
            match LoadCursorFromFileW(PCWSTR(w.as_ptr())) {
                Ok(c) if !c.is_invalid() => DestroyCursor(c).is_ok(),
                _ => false,
            }
        }
    }
}

/// Windows loads every Glass file at every size the files hold, reports the hotspot the SVG asks for and the right size,
/// and draws exactly the pixels the files carry (over black and over white = colour AND alpha checked; every entry is a
/// PNG - a .cur that mixes in a BMP entry makes Windows scale that one for every size, which this test catches). Off
/// screen only.
#[cfg(windows)]
#[test]
fn windows_loads_and_draws_every_glass_file() {
    let mut worst = 0u8;
    for (r, (name, bytes)) in WinRole::ALL.iter().zip(FILES) {
        let path = assets().join(name);
        assert!(win::load_default(&path), "{name}: LoadCursorFromFileW");
        let stem = name.split('.').next().unwrap();
        let cur = if animated(*r) { ani_first_frame(bytes) } else { bytes };
        for px in [32u32, 48, 64, 96, 128] {
            let l = win::load(&path, px as i32);
            assert!(l.is_cursor, "{name} {px}: a cursor, not an icon");
            assert_eq!(l.bmp, (px as i32, px as i32), "{name} {px}: Windows picked the {px} px image");
            assert_eq!(l.hot, svg_hotspot(stem, px), "{name} {px}: hotspot");
            let (want, hot) = cur_image(cur, px);
            assert_eq!(hot, l.hot, "{name} {px}: the file's hotspot");
            let mut opaque = 0;
            for (i, s) in want.iter().enumerate() {
                let a = s[3] as u32;
                for (c, bg) in [(l.on_black[i], 0u32), (l.on_white[i], 255u32)] {
                    for ch in 0..3 {
                        // BGRA from Windows vs straight RGBA from the file, blended the textbook way
                        let exp = (s[2 - ch] as u32 * a + bg * (255 - a) + 127) / 255;
                        let d = (c[ch] as i32 - exp as i32).unsigned_abs() as u8;
                        worst = worst.max(d);
                        assert!(d <= 3, "{name} {px}: pixel {i} ch {ch}: Windows {} vs file {exp} (alpha {a})", c[ch]);
                    }
                }
                opaque += (a > 200) as u32;
            }
            assert!(opaque as f32 > (px * px) as f32 * 0.02, "{name} {px}: real pixels drawn ({opaque})");
        }
    }
    eprintln!("Glass: 17 files x 5 sizes loaded and drawn by Windows; worst channel difference {worst}");
}
