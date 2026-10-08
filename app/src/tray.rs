//! The tray icon ("Pane"): added with Shell_NotifyIcon, follows the taskbar's light / dark theme, tooltip hidden
//! while the menu is open, right-click shows a tiny native menu (Open · Quit).

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_TRAY: u32 = WM_APP + 1;
const ICON_ID: u32 = 1;
pub const IDM_OPEN: usize = 100;
pub const IDM_QUIT: usize = 101;

pub struct Tray {
    hwnd: HWND,
    icon: HICON,
    added: bool,
    open: bool,
    /// false in automated tests (`--offscreen` / `--no-tray`): no icon at all
    enabled: bool,
}

/// Are Windows' apps light? (`AppsUseLightTheme` - Settings › Theme › Match Windows follows it)
pub fn apps_light() -> bool {
    // (no value = Windows' own default: light)
    personalize_flag(w!("AppsUseLightTheme")).unwrap_or(true)
}

/// Is the Windows taskbar light? (`SystemUsesLightTheme`)
pub fn taskbar_light() -> bool {
    personalize_flag(w!("SystemUsesLightTheme")).unwrap_or(false)
}

fn personalize_flag(name: PCWSTR) -> Option<bool> {
    unsafe {
        let mut v = 0u32;
        let mut n = 4u32;
        let r = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut _ as *mut _),
            Some(&mut n),
        );
        r.is_ok().then_some(v != 0)
    }
}

fn load_icon(hwnd: HWND) -> HICON {
    unsafe {
        let dpi = GetDpiForWindow(hwnd).max(96) as f32;
        let px = 16.0 * dpi / 96.0;
        // the drawing's rule: 16 x scale >= 28 -> 32 px, >= 20 -> 24 px, else 16 px
        let size = if px >= 28.0 { 32 } else if px >= 20.0 { 24 } else { 16 };
        let id = if taskbar_light() { 2u16 } else { 1u16 };
        let inst = GetModuleHandleW(None).unwrap_or_default();
        LoadImageW(Some(inst.into()), PCWSTR(id as usize as *const u16), IMAGE_ICON, size, size, LR_DEFAULTCOLOR)
            .map(|h| HICON(h.0))
            .unwrap_or_default()
    }
}

impl Tray {
    pub fn new(hwnd: HWND, enabled: bool) -> Tray {
        let mut t = Tray { hwnd, icon: HICON::default(), added: false, open: false, enabled };
        if enabled {
            TRAY_HWND.store(hwnd.0 as isize, std::sync::atomic::Ordering::Release);
        }
        t.add();
        t
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut d = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | if self.open { NOTIFY_ICON_DATA_FLAGS(0) } else { NIF_SHOWTIP },
            uCallbackMessage: WM_TRAY,
            hIcon: self.icon,
            ..Default::default()
        };
        // + one line of an add-on (Notifications for OBS: "Clipping monitor 1 · instant replay on"), at most 127 characters
        let extra = TIP_EXTRA.lock().map(|e| e.clone()).unwrap_or_default();
        let text = if extra.is_empty() { "Boyler Utilities\nDouble-click to open".to_string() } else { format!("Boyler Utilities\nDouble-click to open\n{extra}") };
        let tip: Vec<u16> = text.encode_utf16().take(127).collect();
        d.szTip[..tip.len()].copy_from_slice(&tip);
        d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        d
    }

    /// Add (again, e.g. after Explorer restarted).
    pub fn add(&mut self) {
        if !self.enabled {
            return;
        }
        unsafe {
            self.icon = load_icon(self.hwnd);
            if let Ok(mut k) = LAST_ICON_KEY.lock() {
                *k = Some((taskbar_light(), GetDpiForWindow(self.hwnd)));
            }
            let d = self.data();
            let _ = Shell_NotifyIconW(NIM_DELETE, &d);
            self.added = Shell_NotifyIconW(NIM_ADD, &d).as_bool();
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &d);
        }
        badge_show();
    }

    /// Re-pick the icon (taskbar theme / DPI changed).
    pub fn refresh(&mut self) {
        if !self.added {
            return;
        }
        // Order 050: Windows broadcasts "a setting changed" for every change (the app's own included); the icon only
        // depends on the taskbar's theme and the DPI - nothing else reloads it (Shell_NotifyIcon waits for Explorer,
        // which is busy with that same broadcast)
        let key = (taskbar_light(), unsafe { GetDpiForWindow(self.hwnd) });
        if LAST_ICON_KEY.lock().ok().is_some_and(|mut k| k.replace(key) == Some(key)) {
            return;
        }
        unsafe {
            let old = self.icon;
            self.icon = load_icon(self.hwnd);
            let d = self.data();
            let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
            if !old.is_invalid() {
                let _ = DestroyIcon(old);
            }
        }
        badge_show();
    }

    /// While the menu is open the hover tooltip is hidden.
    pub fn set_open(&mut self, open: bool) {
        MENU_OPEN.store(open, std::sync::atomic::Ordering::Relaxed);
        if self.open != open {
            self.open = open;
            if !self.added {
                return;
            }
            unsafe {
                let d = self.data();
                let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
            }
            // (Order 045: that modify put the plain icon back - the mute badge goes on again)
            badge_show();
        }
    }

    /// The icon's rectangle on screen (to tell a click on it from a click elsewhere).
    pub fn rect(&self) -> Option<RECT> {
        if !self.added {
            return None;
        }
        unsafe {
            let id = NOTIFYICONIDENTIFIER { cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32, hWnd: self.hwnd, uID: ICON_ID, ..Default::default() };
            Shell_NotifyIconGetRect(&id).ok()
        }
    }

    /// The tiny right-click menu. Returns the picked command id (0 = none).
    pub fn context_menu(&self, x: i32, y: i32) -> usize {
        unsafe {
            let Ok(m) = CreatePopupMenu() else { return 0 };
            let _ = AppendMenuW(m, MF_STRING, IDM_OPEN, w!("Open"));
            let _ = AppendMenuW(m, MF_STRING, IDM_QUIT, w!("Quit"));
            let _ = SetMenuDefaultItem(m, IDM_OPEN as u32, 0);
            let _ = SetForegroundWindow(self.hwnd);
            let r = TrackPopupMenu(m, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, x, y, None, self.hwnd, None);
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(m);
            r.0 as usize
        }
    }

    pub fn remove(&mut self) {
        if self.added {
            unsafe {
                let d = self.data();
                let _ = Shell_NotifyIconW(NIM_DELETE, &d);
            }
            self.added = false;
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
        unsafe {
            if !self.icon.is_invalid() {
                let _ = DestroyIcon(self.icon);
            }
        }
    }
}

/// The tooltip's add-on line (Order 035, A_035_01: Notifications for OBS's "Clipping monitor 1 · instant replay on").
static TIP_EXTRA: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
/// The tray icon's window (set by `Tray::new` when the icon is shown).
static TRAY_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
/// Order 050: what the icon was last loaded for (taskbar light, DPI) - `Tray::refresh` reloads only when it changed.
static LAST_ICON_KEY: std::sync::Mutex<Option<(bool, u32)>> = std::sync::Mutex::new(None);

/// The menu is open (the tooltip is hidden meanwhile).
static MENU_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set (or clear) the tooltip's add-on line; the icon's tooltip changes at once.
pub fn set_tip_extra(line: Option<&str>) {
    let s = line.unwrap_or("").to_string();
    if let Ok(mut e) = TIP_EXTRA.lock() {
        if *e == s {
            return;
        }
        *e = s;
    }
    let h = TRAY_HWND.load(std::sync::atomic::Ordering::Acquire);
    if h == 0 {
        return;
    }
    let t = Tray { hwnd: HWND(h as *mut _), icon: HICON::default(), added: true, open: false, enabled: true };
    let mut d = t.data();
    // while the menu is open the hover tooltip stays hidden (as `Tray::set_open` keeps it)
    d.uFlags = if MENU_OPEN.load(std::sync::atomic::Ordering::Relaxed) { NIF_TIP } else { NIF_TIP | NIF_SHOWTIP };
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
    }
    std::mem::forget(t);
}

// ------------------------------------------------------------------ the mute badge (Order 045)
/// The red mute badge on the tray icon while the mic is muted (menu-v22 `#btb.bdg`, L61-63 / L3600):
/// `.bdg{right:-3px;bottom:-2px;width:7px;height:7px;border-radius:50%;background:#ff453a;box-shadow:0 0 0 1.5px #23252d;
///   transform:scale(0);transition:transform .22s cubic-bezier(.3,.7,.2,1)}` `.bdg.on{transform:scale(1)}`.
/// A tray icon cannot draw outside its own 16 px (the drawing's dot hangs 3 px / 2 px past its glyph), so the dot sits in
/// the icon's bottom-right corner with its 1.5 px ring inside the icon; the ring (the drawing's taskbar colour) is cut out
/// of the tile, so the real taskbar shows through it on a dark and on a light taskbar alike. Drawn per pixel size.
struct Badge {
    on: bool,
    /// the scale shown now and the transition running (from, start)
    cur: f32,
    anim: Option<(f32, std::time::Instant)>,
    /// the tray shows a badged icon now
    shown: bool,
}

static BADGE: std::sync::Mutex<Badge> = std::sync::Mutex::new(Badge { on: false, cur: 0.0, anim: None, shown: false });
const BADGE_MS: f32 = 220.0;

/// The mic is muted / unmuted (the Audio tab's mute state, `mute::sync_icon`): the badge scales in / out.
pub fn set_muted(muted: bool) {
    let Ok(mut b) = BADGE.lock() else { return };
    if b.on == muted {
        return;
    }
    b.on = muted;
    let h = TRAY_HWND.load(std::sync::atomic::Ordering::Acquire);
    if h == 0 {
        // no icon yet (app start): it is added with the badge already in place
        b.cur = if muted { 1.0 } else { 0.0 };
        b.anim = None;
        return;
    }
    let from = b.cur;
    b.anim = Some((from, std::time::Instant::now()));
    drop(b);
    // a few icon changes only while the .22 s scale runs - on their own short thread (Order 050: each is a wait for
    // Explorer, ~14 of them per mute; they used to run on the UI thread's timer)
    if BADGE_RUNNING.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new().name("bu-tray-badge".into()).spawn(|| loop {
        badge_show();
        if BADGE.lock().map(|b| b.anim.is_none()).unwrap_or(true) {
            BADGE_RUNNING.store(false, std::sync::atomic::Ordering::Release);
            // a new transition that started just now keeps going here
            if BADGE.lock().map(|b| b.anim.is_none()).unwrap_or(true) || BADGE_RUNNING.swap(true, std::sync::atomic::Ordering::AcqRel) {
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    });
    if spawned.is_err() {
        BADGE_RUNNING.store(false, std::sync::atomic::Ordering::Release);
        badge_show();
    }
}

/// Order 050: the badge's animation thread runs.
static BADGE_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The badge's scale `ms` ms into the transition from `from` toward on / off: `cubic-bezier(.3,.7,.2,1)` over .22 s.
fn badge_scale(from: f32, on: bool, ms: f32) -> f32 {
    let p = crate::anim::Bezier::new(0.3, 0.7, 0.2, 1.0).ease((ms / BADGE_MS).clamp(0.0, 1.0) as f64) as f32;
    let to = if on { 1.0 } else { 0.0 };
    from + (to - from) * p
}

/// Put the icon with the badge at its current scale on the tray (the plain icon at scale 0).
fn badge_show() {
    let h = TRAY_HWND.load(std::sync::atomic::Ordering::Acquire);
    if h == 0 {
        return;
    }
    let hwnd = HWND(h as *mut _);
    let Ok(mut b) = BADGE.lock() else { return };
    if let Some((from, t0)) = b.anim {
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        b.cur = badge_scale(from, b.on, ms);
        if ms >= BADGE_MS {
            b.anim = None;
        }
    }
    if b.cur <= 0.0 && !b.shown {
        return;
    }
    let base = load_icon(hwnd);
    let icon = if b.cur > 0.0 { badged(base, b.cur) } else { None };
    let t = Tray { hwnd, icon: icon.unwrap_or(base), added: true, open: MENU_OPEN.load(std::sync::atomic::Ordering::Relaxed), enabled: true };
    let mut d = t.data();
    d.uFlags = NIF_ICON;
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
    }
    std::mem::forget(t);
    // the shell keeps its own copy of the icon it was given
    unsafe {
        if let Some(i) = icon {
            let _ = DestroyIcon(i);
        }
        let _ = DestroyIcon(base);
    }
    b.shown = b.cur > 0.0;
}

/// An icon's straight-alpha BGRA pixels (top-down), width, height.
fn icon_pixels(icon: HICON) -> Option<(Vec<u8>, i32, i32)> {
    use windows::Win32::Graphics::Gdi::*;
    unsafe {
        let mut ii = ICONINFO::default();
        GetIconInfo(icon, &mut ii).ok()?;
        let mut bm = BITMAP::default();
        GetObjectW(ii.hbmColor.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _));
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        let mut px = vec![0u8; (w.max(0) * h.max(0) * 4) as usize];
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..Default::default() },
            ..Default::default()
        };
        let dc = GetDC(None);
        let got = if px.is_empty() { 0 } else { GetDIBits(dc, ii.hbmColor, 0, h as u32, Some(px.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS) };
        ReleaseDC(None, dc);
        let _ = DeleteObject(ii.hbmColor.into());
        let _ = DeleteObject(ii.hbmMask.into());
        (got != 0).then_some((px, w, h))
    }
}

/// The icon `base` with the badge at scale `s` (a new icon; `base` stays).
fn badged(base: HICON, s: f32) -> Option<HICON> {
    use windows::Win32::Graphics::Gdi::*;
    let (mut px, w, h) = icon_pixels(base)?;
    paint_badge(&mut px, w as usize, h as usize, s);
    unsafe {
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..Default::default() },
            ..Default::default()
        };
        let dc = GetDC(None);
        let color = CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, std::ptr::null_mut(), None, 0).ok();
        ReleaseDC(None, dc);
        let color = color?;
        SetDIBits(None, color, 0, h as u32, px.as_ptr() as *const _, &bi, DIB_RGB_COLORS);
        // an all-zero mask: the colour bitmap's alpha decides (rows of whole 16-bit words)
        let zeros = vec![0u8; (((w + 15) / 16 * 2) * h).max(0) as usize];
        let mask = CreateBitmap(w, h, 1, 1, Some(zeros.as_ptr() as *const _));
        let ii2 = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let r = CreateIconIndirect(&ii2).ok();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        r
    }
}

/// The badge into straight-alpha BGRA pixels `w` x `h` (top-down) at scale `s`: the 1.5 px ring cut out of the icon, then
/// the 7 px #ff453a dot (16 px icon units, scaled to the pixel size), both scaled about the dot's centre; edges
/// anti-aliased by 4 x 4 sampling.
fn paint_badge(px: &mut [u8], w: usize, h: usize, s: f32) {
    let u = w as f32 / 16.0;
    let (r_dot, r_cut) = (3.5 * u * s, 5.0 * u * s);
    // at full size the ring touches the icon's right and bottom edges
    let (cx, cy) = (w as f32 - 5.0 * u, h as f32 - 5.0 * u);
    let cover = |x: usize, y: usize, r: f32| -> f32 {
        let mut n = 0;
        for j in 0..4 {
            for i in 0..4 {
                let dx = x as f32 + (i as f32 + 0.5) / 4.0 - cx;
                let dy = y as f32 + (j as f32 + 0.5) / 4.0 - cy;
                if dx * dx + dy * dy <= r * r {
                    n += 1;
                }
            }
        }
        n as f32 / 16.0
    };
    for y in 0..h {
        for x in 0..w {
            let cut = cover(x, y, r_cut);
            if cut <= 0.0 {
                continue;
            }
            let d = cover(x, y, r_dot);
            let o = (y * w + x) * 4;
            let a = px[o + 3] as f32 / 255.0 * (1.0 - cut);
            let out_a = d + a * (1.0 - d);
            // #ff453a over what is left of the tile (B, G, R order)
            for (c, dot) in [(0usize, 58.0f32), (1, 69.0), (2, 255.0)] {
                let v = if out_a > 0.0 { (dot * d + px[o + c] as f32 * a * (1.0 - d)) / out_a } else { 0.0 };
                px[o + c] = v.round().clamp(0.0, 255.0) as u8;
            }
            px[o + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

#[cfg(test)]
mod badge_tests {
    use super::{badge_scale, paint_badge};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, LoadImageW, HICON, IMAGE_ICON, LR_LOADFROMFILE};

    #[test]
    fn the_badge_scales_in_over_220_ms() {
        assert_eq!(badge_scale(0.0, true, 0.0), 0.0);
        assert_eq!(badge_scale(0.0, true, 220.0), 1.0);
        assert!(badge_scale(0.0, true, 110.0) > 0.5, "an ease-out curve is past half at half time");
        assert_eq!(badge_scale(1.0, false, 220.0), 0.0);
    }

    #[test]
    fn the_dot_is_red_the_ring_is_cut_and_the_tile_stays() {
        // a 16 px half-transparent grey tile
        let mut px = vec![0x80u8; 16 * 16 * 4];
        paint_badge(&mut px, 16, 16, 1.0);
        let at = |x: usize, y: usize| px[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4].to_vec();
        // the dot's centre (11, 11): #ff453a, opaque
        assert_eq!(at(11, 11), vec![0x3a, 0x45, 0xff, 0xff]);
        // in the ring (4.5 px left of the centre): cut out
        assert_eq!(at(6, 11)[3], 0);
        // far from the badge: the tile, untouched
        assert_eq!(at(2, 2), vec![0x80, 0x80, 0x80, 0x80]);
    }

    /// Order 045 proof picture (`BU_PIC_OUT=<folder> cargo test -p bu-app proof_045 -- --ignored`): the real tray icons
    /// (dark / light taskbar) at 16 / 24 / 32 px, plain and with the badge, on their taskbar's colour, 8 x enlarged.
    #[test]
    #[ignore]
    fn proof_045_tray_badge() {
        use windows::core::HSTRING;
        let Ok(dir) = std::env::var("BU_PIC_OUT") else { return };
        let mut rows: Vec<Vec<u8>> = Vec::new();
        let (z, gap) = (8usize, 16usize);
        let width = (16 + 24 + 32) * 2 * z + 7 * gap;
        for (file, bg) in [("pane_dark.ico", [0x2d, 0x25, 0x23]), ("pane_light.ico", [0xf3, 0xf3, 0xf3])] {
            let path = format!("{}/assets/{file}", env!("CARGO_MANIFEST_DIR"));
            let hgt = 32 * z + 2 * gap;
            let mut img = vec![0u8; width * hgt * 4];
            for p in img.chunks_mut(4) {
                p.copy_from_slice(&[bg[0], bg[1], bg[2], 255]);
            }
            let mut x0 = gap;
            for size in [16i32, 24, 32] {
                let h = unsafe { LoadImageW(None, &HSTRING::from(path.as_str()), IMAGE_ICON, size, size, LR_LOADFROMFILE) }.expect("icon");
                let (px, w, hh) = super::icon_pixels(HICON(h.0)).expect("pixels");
                for badge in [false, true] {
                    let mut p = px.clone();
                    if badge {
                        paint_badge(&mut p, w as usize, hh as usize, 1.0);
                    }
                    for y in 0..hh as usize * z {
                        for x in 0..w as usize * z {
                            let s = &p[((y / z) * w as usize + x / z) * 4..][..4];
                            let a = s[3] as u32;
                            let o = ((gap + y) * width + x0 + x) * 4;
                            for c in 0..3 {
                                img[o + c] = ((s[c] as u32 * a + img[o + c] as u32 * (255 - a)) / 255) as u8;
                            }
                        }
                    }
                    x0 += w as usize * z + gap;
                }
                unsafe {
                    let _ = DestroyIcon(HICON(h.0));
                }
            }
            rows.push(img);
        }
        let data: Vec<u8> = rows.concat();
        let h = data.len() / 4 / width;
        let _ = std::fs::create_dir_all(&dir);
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let px = crate::png::Pixels { w: width as u32, h: h as u32, data };
        crate::png::save_png(&px, &format!("{dir}/07_tray_badge.png")).expect("png");
    }
}
