//! Notifications for OBS in the app: ClipPing's popup looks and status icon are pixel-for-pixel ClipPing's own pictures
//! (its test renders, C:\src\notifications-for-obs\tests\renders, read-only; skipped where missing), the Glass look
//! draws in both themes, and the pictures of every look are written for a person to look at (BU_OBS_PICS=<folder>).

use bu_obs::engine::{Color, Icon, PopMsg};
use bu_obs::settings::{ST_CARD, ST_FLOAT, ST_GLASS};

use super::gdi::{self, on_backdrop, Img};

const REF: &str = "C:/src/notifications-for-obs/tests/renders";
const NAMES: [&str; 6] = ["Card", "Pill", "Accent edge", "Timer bar", "Tile", "Floating text"];

fn samples() -> [PopMsg; 5] {
    [
        PopMsg::new(Color::Green, Icon::Check, "Monitor 1", "Clipped last 60 seconds", "Clipped", "60 s"),
        PopMsg::new(Color::Red, Icon::Cross, "Instant replay is off", "Nothing was saved", "Not saved", "replay off"),
        PopMsg::new(Color::Blue, Icon::Switch, "Monitor 2 · instant replay on", "Now clipping Second screen", "Now clipping", "Second screen"),
        PopMsg::new(Color::Amber, Icon::Drive, "About 40 minutes of recording left", "Storage almost full", "Storage almost full", "~40 min"),
        PopMsg::new(Color::Grey, Icon::Plug, "Monitor 1 · instant replay on", "OBS connected", "OBS connected", ""),
    ]
}

fn read_bmp(p: &str) -> Option<Img> {
    let b = std::fs::read(p).ok()?;
    let off = u32::from_le_bytes(b[10..14].try_into().ok()?) as usize;
    let w = i32::from_le_bytes(b[18..22].try_into().ok()?);
    let h = i32::from_le_bytes(b[22..26].try_into().ok()?);
    let (h, top_down) = (h.abs(), h < 0);
    let mut px = vec![0u32; (w * h) as usize];
    for y in 0..h {
        let sy = if top_down { y } else { h - 1 - y };
        for x in 0..w {
            let i = off + ((sy * w + x) * 4) as usize;
            px[(y * w + x) as usize] = u32::from_le_bytes(b[i..i + 4].try_into().ok()?);
        }
    }
    Some(Img { w, h, px })
}

fn diff(a: &Img, b: &Img) -> usize {
    if (a.w, a.h) != (b.w, b.h) {
        return usize::MAX;
    }
    a.px.iter().zip(&b.px).filter(|(x, y)| (**x & 0xFFFFFF) != (**y & 0xFFFFFF)).count()
}

/// popup.c `popup_render_styles`: every look x every colour x {Dark grey, Light} at 100 %, 96 dpi, Timer bar at 650,
/// 8 px over a backdrop (Floating text over dark / light grey).
#[test]
fn clippings_six_looks_are_pixel_identical_to_its_own_renders() {
    if !std::path::Path::new(REF).is_dir() {
        eprintln!("ClipPing's renders not here: skipped");
        return;
    }
    let mut bad = Vec::new();
    for st in 0..6 {
        for (c, m) in samples().iter().enumerate() {
            for (b, (bg, back)) in [(0x2C2C2Au32, 0x6E6E6Eu32), (0xF1EFE8, 0xE8E8E8)].into_iter().enumerate() {
                let img = gdi::popup_draw(m, st, bg, 100, 96, 650);
                let mine = on_backdrop(&img, if st == ST_FLOAT { back } else { 0x6E6E6E }, 8);
                let name = format!("{st}_{}_{c}_{}", NAMES[st as usize], if b == 1 { "light" } else { "darkgrey" });
                let theirs = read_bmp(&format!("{REF}/styles/{name}.bmp")).unwrap();
                let d = diff(&mine, &theirs);
                if d != 0 {
                    bad.push(format!("{name}: {d} px ({}x{} vs {}x{})", mine.w, mine.h, theirs.w, theirs.h));
                }
            }
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// status.c `status_render_bmp`: the tiles zoomed 4x, 12 px around, over a grey checkerboard.
fn status_picture(replay: bool, rec: bool, dpi: i32) -> Img {
    let img = gdi::status_draw(replay, rec, dpi);
    let (z, pad) = (4, 12);
    let (w, h) = ((img.w + 2 * pad) * z, (img.h + 2 * pad) * z);
    let mut px = vec![0u32; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = (x / z - pad, y / z - pad);
            let bg: u32 = if ((x / (8 * z) + y / (8 * z)) & 1) != 0 { 0x9A9A9A } else { 0xC8C8C8 };
            let s = if sx >= 0 && sy >= 0 && sx < img.w && sy < img.h { img.px[(sy * img.w + sx) as usize] } else { 0 };
            let a = s >> 24;
            let r = (((s >> 16) & 255) + ((bg >> 16) & 255) * (255 - a) / 255).min(255);
            let g = (((s >> 8) & 255) + ((bg >> 8) & 255) * (255 - a) / 255).min(255);
            let bl = ((s & 255) + (bg & 255) * (255 - a) / 255).min(255);
            px[(y * w + x) as usize] = 0xFF00_0000 | r << 16 | g << 8 | bl;
        }
    }
    Img { w, h, px }
}

#[test]
fn the_status_icon_is_pixel_identical_to_clippings() {
    if !std::path::Path::new(REF).is_dir() {
        return;
    }
    for (name, replay, rec, dpi) in [("status_icon_both", true, true, 96), ("status_icon_replay", true, false, 96), ("status_icon_recording", false, true, 96), ("status_icon_both_150", true, true, 144)] {
        let theirs = read_bmp(&format!("{REF}/{name}.bmp")).unwrap();
        let mine = status_picture(replay, rec, dpi);
        assert_eq!(diff(&mine, &theirs), 0, "{name}");
    }
}

#[test]
fn the_glass_look_draws_in_both_themes() {
    for light in [false, true] {
        crate::ui::set_light(light);
        let p = super::glass::picture(&PopMsg::sample(), 1.0).unwrap();
        assert!(p.w > 200 && p.h > 90, "{}x{}", p.w, p.h);
    }
    crate::ui::set_light(false);
}

fn save(img: &Img, path: &std::path::Path) {
    let data: Vec<u8> = img.px.iter().flat_map(|p| p.to_le_bytes()).collect();
    let px = crate::png::Pixels { w: img.w as u32, h: img.h as u32, data };
    let _ = crate::png::save_png(&px, &path.to_string_lossy());
}

/// The pictures for a person: every look (Glass dark + light, ClipPing's six on Dark grey) with each colour. Only when
/// BU_OBS_PICS names a folder.
#[test]
fn pictures_of_every_look() {
    let Ok(dir) = std::env::var("BU_OBS_PICS") else { return };
    let dir = std::path::PathBuf::from(dir);
    let _ = std::fs::create_dir_all(&dir);
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
    }
    for (c, m) in samples().iter().enumerate() {
        for light in [false, true] {
            crate::ui::set_light(light);
            if let Some(p) = super::glass::picture(m, 1.0) {
                let img = Img { w: p.w as i32, h: p.h as i32, px: p.data.as_chunks::<4>().0.iter().map(|b| u32::from_le_bytes(*b)).collect() };
                save(&img, &dir.join(format!("glass_{}_{c}.png", if light { "light" } else { "dark" })));
            }
        }
        crate::ui::set_light(false);
        for st in ST_CARD..ST_GLASS {
            let img = on_backdrop(&gdi::popup_draw(m, st, 0x2C2C2A, 100, 96, 650), 0x6E6E6E, 8);
            save(&img, &dir.join(format!("{st}_{}_{c}.png", NAMES[st as usize])));
        }
    }
    save(&status_picture(true, true, 96), &dir.join("status_icon.png"));
}

/// The first switch-on finds ClipPing's file next to the exe its Start-with-Windows entry starts and imports it (here a
/// scratch copy in UTF-16 like ClipPing writes it; the fake OS stands in for the registry).
#[test]
fn clippings_file_is_found_next_to_its_exe_and_imported() {
    let d = std::env::temp_dir().join("bu-obs-test").join(format!("import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let text = "[NotificationsForOBS]\r\nPopupWhere=0x1\r\nVolume=0x1E\r\nPopupStyle=0x0\r\nStatusIcon=0x1\r\nSwitchKey=0x13\r\n[Scenes]\r\nCount=0x2\r\n1=16:9\r\n2=21:9\r\n";
    let mut b = vec![0xFFu8, 0xFE];
    for u in text.encode_utf16() {
        b.extend_from_slice(&u.to_le_bytes());
    }
    std::fs::write(d.join("NotificationsForOBS.ini"), b).unwrap();
    let os = bu_obs::fake::FakeOs::new(&d);
    os.with(|s| s.run_entry = Some(d.join("NotificationsForOBS (1).exe")));
    let p = super::clipping_ini(&os).expect("found");
    let s = bu_obs::settings::import_file(&p).unwrap();
    assert_eq!((s.where_, s.vol, s.status, s.style), (bu_obs::settings::W_SAME, 30, bu_obs::settings::SI_REC_MON, ST_GLASS));
    assert_eq!(s.scenes, vec!["16:9".to_string(), "21:9".to_string()]);
    assert_eq!(s.switch_key, Some(bu_obs::keys::KeyBind::new(0x13, 0)));
    // the file itself is untouched (read-only)
    assert_eq!(std::fs::read(d.join("NotificationsForOBS.ini")).unwrap().len(), 2 + text.encode_utf16().count() * 2);
    let _ = std::fs::remove_dir_all(&d);
}
