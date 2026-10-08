//! The menu window (exists only while the menu is open): a composition window for the glass + content, a
//! click-through layered window above it for the flyout's soft shadow, and the per-frame drawing pipeline:
//! Skia paints the frame with the drawing's own layer structure (`Menu::compose`) - on the GPU straight into the composition
//! swap chain's back buffer (Order 051, gpu.rs), or on the CPU with one copy into the swap chain (present.rs) when the GPU
//! can't be used.

use skia_safe as sk;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::audio::Audio;
use crate::comp::Glass;
use crate::gfx::*;
use crate::icons::Icons;
use crate::gpu::{self, Gpu, GpuChain};
use crate::present::{Chain, Swap};
use std::rc::Rc;
use crate::timing;
use crate::ui::{self, Frame, Ui, PAGE_H, PAGE_TOP, RADIUS, WIN_H, WIN_W};


// shadow margins around the glass (DIPs): 0 30px 80px reaches 120 px out, shifted 30 px down
const SH_L: f32 = 124.0;
const SH_T: f32 = 94.0;
const SH_R: f32 = 124.0;
const SH_B: f32 = 154.0;

pub struct Menu {
    pub hwnd: HWND,
    pub shadow: HWND,
    g: Gfx,
    /// Order 051: the GPU the frame is drawn on (None = the CPU path)
    gpu: Option<Rc<Gpu>>,
    /// the CPU path after a lost device: when to try the GPU again
    retry_at: Option<f64>,
    /// the monitor the menu is on (the GPU adapter that drives it is used)
    mon: HMONITOR,
    chain: Swap,
    /// Skia's coverage of the rounded window shape, the glass's mask (Order 013)
    mask: Swap,
    /// option 3 (BU_GLASS=gpu): the swap chain the desktop capture is copied into
    capchain: Option<Chain>,
    glass: Glass,
    /// the glass style the glass brush + the window's tint are painted with (Settings › Glass style, live)
    glass_style: crate::settings::GlassStyle,
    /// the light glass is showing (Settings › Theme, live; Order 033)
    light: bool,
    mode: crate::comp::GlassMode,
    /// BU_GLASS=own: the desktop capture, the blurred glass made from it, the frame with the glass under it
    cap: Option<crate::capture::Capture>,
    glass_px: Option<sk::Surface>,
    glass_key: (u64, i32, i32, bool, crate::settings::GlassStyle),
    /// test only: where the capture looks, relative to the window's place (BU_CAPTURE_AT)
    cap_origin: (i32, i32),
    out: Option<sk::Surface>,
    /// a new desktop picture arrived: draw a frame
    pub backdrop_dirty: bool,
    icons: Icons,
    ly: Layers,
    /// the finished frame
    lc: sk::Surface,
    last_now: f64,
    pub ui: Ui,
    pub scale: f32,
    pub x: i32,
    pub y: i32,
    w: i32,
    h: i32,
    sh_dc: HDC,
    sh_bmp: HBITMAP,
    sh_old: HGDIOBJ,
    sh_w: i32,
    sh_h: i32,
    /// the shadow window's pixels (test pictures: `snapscreen`)
    sh_px: Option<crate::png::Pixels>,
    last_pos: (i32, i32),
    last_alpha: u8,
    pub offscreen: bool,
    /// never takes focus (tests: off-screen, or `--screen-test` on the real screen)
    pub noact: bool,
    pub frames: u64,
    pub presented_once: bool,
    /// when the last frame was presented (ms): a waitable that stays silent for 100 ms is not waited for (main loop)
    pub last_frame: f64,
    /// the swap chain's waitable fired and no frame has used it yet (waiting on it again would lose it)
    pub slot: bool,
}

pub fn register_classes(wndproc: WNDPROC) -> Result<()> {
    unsafe {
        let inst = GetModuleHandleW(None)?;
        let cur = LoadCursorW(None, IDC_ARROW)?;
        RegisterClassW(&WNDCLASSW { lpfnWndProc: wndproc, hInstance: inst.into(), lpszClassName: crate::testmode::menu_class(), hCursor: cur, ..Default::default() });
        RegisterClassW(&WNDCLASSW { lpfnWndProc: wndproc, hInstance: inst.into(), lpszClassName: crate::testmode::shadow_class(), hCursor: cur, ..Default::default() });
    }
    Ok(())
}

/// Where the flyout goes: 12 px from the right of the work area and 12 px above the taskbar, on the given monitor.
pub fn place(work: RECT, scale: f32) -> (i32, i32, i32, i32) {
    let w = (WIN_W * scale).round() as i32;
    let h = (WIN_H * scale).round() as i32;
    let m = (12.0 * scale).round() as i32;
    // never larger than the screen: max width = screen - 24, max height = work area - 24
    let w = w.min(work.right - work.left - 2 * m);
    let h = h.min(work.bottom - work.top - 2 * m);
    (work.right - m - w, work.bottom - m - h, w, h)
}

impl Menu {
    /// `frozen` = test pictures (live values at the drawing's samples); the pages make their own services (Audio too).
    #[allow(clippy::too_many_arguments)]
    pub fn new(work: RECT, scale: f32, offscreen: bool, noact: bool, rm: bool, frozen: bool, now: f64) -> Result<Menu> {
        unsafe {
            let (mut x, y, w, h) = place(work, scale);
            if offscreen {
                x = -20000;
            }
            let inst = GetModuleHandleW(None)?;
            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | if noact { WS_EX_NOACTIVATE } else { WINDOW_EX_STYLE(0) },
                crate::testmode::menu_class(),
                w!("Boyler Utilities"),
                WS_POPUP,
                x,
                y,
                w,
                h,
                None,
                None,
                Some(inst.into()),
                None,
            )?;
            log_rect("create", hwnd);
            // the real desktop behind the window (see comp.rs); no DWM rounding / border (we draw our own 14 px corners)
            let mode = crate::comp::glass_mode();
            let pref = DWMWCP_DONOTROUND;
            let r1 = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const _ as *const _, 4);
            let none: u32 = 0xFFFFFFFE; // DWMWA_COLOR_NONE: no system border
            let r2 = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &none as *const _ as *const _, 4);

            // the painter made on tray hover (fonts already loaded), or a new one
            let g = match KEPT_GFX.with(|k| k.borrow_mut().take()) {
                Some(g) if g.scale == scale => g,
                _ => Gfx::new(scale),
            };
            crate::timing::note("open_step gfx");
            // Order 051: the GPU (the adapter driving the menu's monitor), unless a test glass mode needs the CPU frame
            // (BU_GLASS=own / gpu read and copy it on the CPU)
            let mon = MonitorFromPoint(POINT { x: (work.left + work.right) / 2, y: (work.top + work.bottom) / 2 }, MONITOR_DEFAULTTOPRIMARY);
            let gpu = match mode {
                crate::comp::GlassMode::Host | crate::comp::GlassMode::BlurBehind => gpu::get(Some(mon)),
                _ => None,
            };
            let (gpu, chain, mask) = make_swaps(gpu, &g, w, h)?;
            // the layers on the same device; a GPU that can't give them (out of memory, a crash just now) = the CPU path
            let (gpu, chain, mask, ly, lc) = match (Layers::new(gpu.clone(), w, h), surf_on(gpu.as_ref(), w, h)) {
                (Ok(ly), Ok(lc)) => (gpu, chain, mask, ly, lc),
                (a, b) if gpu.is_some() => {
                    let e = a.err().or(b.err()).map(|e| format!("{:08x}", e.code().0)).unwrap_or_default();
                    timing::note(&format!("gpu path=cpu reason=layers {e}"));
                    gpu::mark_lost();
                    drop((chain, mask, gpu));
                    let (gpu, chain, mask) = make_swaps(None, &g, w, h)?;
                    (gpu, chain, mask, Layers::new(None, w, h)?, surf_on(None, w, h)?)
                }
                (a, b) => {
                    a?;
                    b?;
                    unreachable!()
                }
            };
            crate::timing::note("open_step chain");
            let glass_ok = match mode {
                crate::comp::GlassMode::Own | crate::comp::GlassMode::Gpu => test_flag("noexclude") || crate::capture::exclude_from_capture(hwnd),
                crate::comp::GlassMode::Host => {
                    let on: BOOL = TRUE;
                    DwmSetWindowAttribute(hwnd, DWMWA_USE_HOSTBACKDROPBRUSH, &on as *const _ as *const _, 4).is_ok()
                }
                crate::comp::GlassMode::BlurBehind => {
                    // test only (the Order 001-003 way): Windows' blur, meant to be cut by a 1-bit rounded region
                    let d = (2.0 * RADIUS * scale).round() as i32;
                    let rg = test_flag("noregion") || SetWindowRgn(hwnd, Some(CreateRoundRectRgn(0, 0, w + 1, h + 1, d, d)), false) != 0;
                    test_flag("noaccent") || (blur_behind(hwnd) && rg)
                }
            };
            crate::timing::note(&format!(
                "glass mode={:?} ok={} corner_pref={} border_none={} class_background=none no_redirection_bitmap=true edge=mask",
                mode,
                glass_ok,
                r1.is_ok(),
                r2.is_ok()
            ));
            crate::timing::note("open_step mask");
            // the tint is painted by Skia with the rest of the window's own paint (ui::draw_rim), not by the composition
            // option 3 (BU_GLASS=gpu): the swap chain the captured desktop goes into (the window + the motion below it)
            // on its OWN device: the worker waits inside the duplication's AcquireNextFrame, and on a shared
            // (multithread-protected) device that wait held the device's lock - the menu's own presents then took
            // 100-380 ms (measured, Order 013)
            let capchain = if mode == crate::comp::GlassMode::Gpu {
                Some(Chain::new_alpha(crate::present::new_d3d()?, w as u32, h as u32, windows::Win32::Graphics::Dxgi::Common::DXGI_ALPHA_MODE_IGNORE)?)
            } else {
                None
            };
            // the theme first: the glass brush, the shadow and every colour follow it (Order 033)
            let light = ui::sync_theme();
            let (cw, ch) = chain.size();
            let glass = Glass::new(hwnd, (chain.swap(), cw, ch), mask.swap(), capchain.as_ref().map(|c| (c, scale)), w as f32, h as f32, RADIUS * scale, mode)?;
            crate::timing::note("open_step glass");

            // the shadow window: layered + click-through, owned by the menu so it always sits right above it
            let sw = ((WIN_W + SH_L + SH_R) * scale).ceil() as i32;
            let shh = ((WIN_H + SH_T + SH_B) * scale).ceil() as i32;
            let shadow = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                crate::testmode::shadow_class(),
                w!(""),
                WS_POPUP,
                x - (SH_L * scale).round() as i32,
                y - (SH_T * scale).round() as i32,
                sw,
                shh,
                Some(hwnd),
                None,
                Some(inst.into()),
                None,
            )?;
            crate::timing::note("open_step shadow_window");
            let mut ui = Ui::new(rm, frozen, now);
            crate::timing::note("open_step ui");
            ui.sel_color = sys_highlight();
            ui.caret_ms = GetCaretBlinkTime() as f64;
            let mut m = Menu {
                glass_style: crate::services::with(|s| s.glass()).unwrap_or_default(),
                light,
                hwnd,
                shadow,
                g,
                gpu,
                retry_at: None,
                mon,
                chain,
                mask,
                capchain,
                glass,
                mode,
                cap: None,
                glass_px: None,
                glass_key: (0, i32::MIN, i32::MIN, false, crate::settings::GlassStyle::Liquid),
                cap_origin: (0, 0),
                out: None,
                backdrop_dirty: false,
                icons: Icons::new(),
                ly,
                lc,
                last_now: now,
                ui,
                scale,
                x,
                y,
                w,
                h,
                sh_dc: HDC::default(),
                sh_bmp: HBITMAP::default(),
                sh_old: HGDIOBJ::default(),
                sh_w: sw,
                sh_h: shh,
                sh_px: None,
                last_pos: (i32::MIN, i32::MIN),
                last_alpha: 255,
                offscreen,
                noact,
                frames: 0,
                presented_once: false,
                last_frame: now,
                slot: false,
            };
            ui::remember_widths(&m.g, &m.ui.device_names());
            crate::timing::note("open_step widths");
            let at = (m.x - (SH_L * m.scale).round() as i32, m.y - (SH_T * m.scale).round() as i32, 0);
            m.render_shadow(at)?;
            crate::timing::note("open_step shadow");
            if mode == crate::comp::GlassMode::Own || mode == crate::comp::GlassMode::Gpu {
                if !test_flag("noexclude") {
                    crate::capture::exclude_from_capture(m.shadow);
                }
                // the window's rectangle on the screen + the open / close motion below it (8 px + the rise); test only
                // (cost measurements without showing anything): BU_CAPTURE_AT=x,y watches that place while off-screen
                if let Some((cx, cy)) = crate::testmode::env("BU_CAPTURE_AT").filter(|_| offscreen).and_then(|v| v.split_once(',').map(|(a, b)| (a.parse::<i32>().unwrap_or(0), b.parse::<i32>().unwrap_or(0)))) {
                    m.cap_origin = (cx - x, cy - y);
                }
                let (x, y) = (x + m.cap_origin.0, y + m.cap_origin.1);
                // option 3: exactly the window's box (its swap chain's size); option 2: + the motion below it
                let below = if mode == crate::comp::GlassMode::Gpu { 0 } else { (24.0 * scale).ceil() as i32 };
                let r = RECT { left: x, top: y, right: x + w, bottom: y + h + below };
                let gt = m.capchain.as_ref().map(|c| crate::capture::GpuTarget { dev: c.d3d.clone(), swap: c.swap.clone() });
                m.cap = Some(crate::capture::Capture::start(hwnd, r, gt));
                if mode == crate::comp::GlassMode::Own {
                    m.glass_px = Some(layer(w, h)?);
                    m.out = Some(layer(w, h)?);
                }
                // the first picture of the desktop before the first frame (else the glass would pop in)
                let t0 = crate::timing::now();
                while crate::timing::now() - t0 < 250.0 {
                    if m.cap.as_ref().map(|c| c.shot.lock().unwrap().seq > 0).unwrap_or(true) {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                crate::timing::note(&format!("open_step capture_first {:.1} ms", crate::timing::now() - t0));
            }
            Ok(m)
        }
    }

    /// Paint the window's own layer once - the CSS outer shadow, the tint and the rim, as the drawing's `#sw` layer -
    /// then split it at the window rectangle (Order 013): the inside goes to the menu's frame (the rim layer), the
    /// outside into the layered shadow window, which keeps a HOLE the size of the whole window rectangle. Every pixel
    /// is painted exactly once, so at the rounded edge nothing is doubled.
    /// Painted like Chromium rasters that layer: on a surface with Blink's layer bounds (the window's ink rect: each
    /// outer shadow grown by ceil(1.5 x blur) + spread and moved by its offset, rounded out), in its 256 px tiles.
    /// Skia's shadow pixels depend on where the shape sits in the surface / tile it is drawn into (measured: a
    /// window-sized surface made the .5 px ring at the arcs lighter by up to 26 levels, the shadow-window-sized one left
    /// 2 pixels 4 levels off at the left end of the top arc).
    fn render_shadow(&mut self, at: (i32, i32, u8)) -> Result<()> {
        unsafe {
            let s = self.scale;
            let (w, h) = (self.w as f32, self.h as f32);
            let (mut l, mut t, mut r, mut b) = (0.0f32, 0.0f32, w, h);
            for sh in shadows().iter() {
                let e = (1.5 * sh.blur * s).ceil() + sh.spread * s;
                l = l.min(sh.dx * s - e);
                t = t.min(sh.dy * s - e);
                r = r.max(w + sh.dx * s + e);
                b = b.max(h + sh.dy * s + e);
            }
            let (l, t, r, b) = (l.floor(), t.floor(), r.ceil(), b.ceil());
            // test only: move the layer's origin (sweep, Order 013)
            let (l, t) = match crate::testmode::env("BU_OWNSHIFT").and_then(|v| v.split_once(',').map(|(a, b)| (a.parse::<f32>().unwrap_or(0.0), b.parse::<f32>().unwrap_or(0.0)))) {
                Some((sx, sy)) => (l - sx, t - sy),
                None => (l, t),
            };
            let mut own = layer((r - l) as i32, (b - t) as i32)?;
            {
                let g = &self.g;
                let ui = &self.ui;
                raster_tiled(g, &mut own, (0, 0), || {
                    g.cv().translate((-l / s, -t / s));
                    g.box_shadows(0.0, 0.0, WIN_W, WIN_H, RADIUS, &shadows(), false);
                    ui.draw_rim(g);
                });
            }
            let win = sk::IRect::from_xywh(-l as i32, -t as i32, self.w, self.h);
            if let Some(inside) = own.image_snapshot_with_bounds(win) {
                let c = self.ly.rim.canvas();
                c.clear(sk::Color::TRANSPARENT);
                c.draw_image(&inside, (0, 0), None);
                self.ly.rim_valid = true;
            }
            // the window sits at whole pixels (SH_L, SH_T rounded) inside the shadow window
            let (ox, oy) = ((SH_L * s).round() as i32, (SH_T * s).round() as i32);
            let mut surf = layer(self.sh_w, self.sh_h)?;
            surf.canvas().draw_image(own.image_snapshot(), (ox + l as i32, oy + t as i32), None);
            // the hole = the whole window rectangle (the menu window shows all of it)
            {
                let c = surf.canvas();
                c.save();
                c.reset_matrix();
                c.clip_irect(sk::IRect::from_xywh(ox, oy, self.w, self.h), sk::ClipOp::Intersect);
                c.clear(sk::Color::TRANSPARENT);
                c.restore();
            }
            let px = crate::png::from_surface(&mut surf);
            self.sh_px = Some(crate::png::Pixels { w: px.w, h: px.h, data: px.data.clone() });
            // into a DIB for UpdateLayeredWindow (premultiplied BGRA, top-down)
            let screen = GetDC(None);
            let mdc = CreateCompatibleDC(Some(screen));
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: px.w as i32,
                    biHeight: -(px.h as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let hb = match CreateDIBSection(Some(mdc), &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(hb) => hb,
                Err(e) => {
                    ReleaseDC(None, screen);
                    let _ = DeleteDC(mdc);
                    return Err(e);
                }
            };
            std::ptr::copy_nonoverlapping(px.data.as_ptr(), bits as *mut u8, px.data.len());
            let old = SelectObject(mdc, hb.into());
            ReleaseDC(None, screen);
            self.sh_dc = mdc;
            self.sh_bmp = hb;
            self.sh_old = old;
            self.update_shadow(at.0, at.1, at.2, true);
            Ok(())
        }
    }

    /// The theme changed while the menu is open: the shadow window's pixels (and the rim picture) again, shown at once where
    /// and as strong as the shadow was (Order 033).
    fn restyle_shadow(&mut self) {
        unsafe {
            if !self.sh_dc.is_invalid() {
                SelectObject(self.sh_dc, self.sh_old);
                let _ = DeleteObject(self.sh_bmp.into());
                let _ = DeleteDC(self.sh_dc);
                self.sh_dc = HDC::default();
            }
        }
        // pushed once, where and as strong as the shadow is now (no blink to alpha 0 in between)
        let s = self.scale;
        let (pos, alpha) = if self.last_pos.0 == i32::MIN { ((self.x, self.y), 0) } else { (self.last_pos, self.last_alpha) };
        let _ = self.render_shadow((pos.0 - (SH_L * s).round() as i32, pos.1 - (SH_T * s).round() as i32, alpha));
    }

    fn update_shadow(&mut self, x: i32, y: i32, alpha: u8, full: bool) {
        unsafe {
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: alpha, AlphaFormat: AC_SRC_ALPHA as u8 };
            let pt = POINT { x, y };
            if full {
                let size = SIZE { cx: self.sh_w, cy: self.sh_h };
                let src = POINT { x: 0, y: 0 };
                let _ = UpdateLayeredWindow(self.shadow, None, Some(&pt), Some(&size), Some(self.sh_dc), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
            } else {
                let _ = UpdateLayeredWindow(self.shadow, None, Some(&pt), None, None, None, COLORREF(0), Some(&blend), ULW_ALPHA);
            }
        }
    }

    /// On top of every window (open, focused) or a normal window under the app clicked meanwhile (feedback F1: a click
    /// outside no longer closes the menu). The owned shadow window follows its owner (SetWindowPos: a window made
    /// topmost / non-topmost takes its owned windows along).
    pub fn set_topmost(&self, on: bool) {
        if self.noact {
            return;
        }
        unsafe {
            let after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
            let _ = SetWindowPos(self.hwnd, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    /// Show both windows (no activation in test mode).
    pub fn show(&mut self) {
        unsafe {
            let cmd = if self.noact { SW_SHOWNOACTIVATE } else { SW_SHOW };
            let _ = ShowWindow(self.hwnd, cmd);
            if !test_flag("noshadow") {
                let _ = ShowWindow(self.shadow, SW_SHOWNOACTIVATE);
            }
            if !self.noact {
                let _ = SetForegroundWindow(self.hwnd);
            }
            log_rect("show", self.hwnd);
            log_rect("show_shadow", self.shadow);
        }
    }

    /// One frame: move the flyout, draw, present.
    pub fn frame(&mut self, now: f64) -> Result<()> {
        let p0 = timing::now();
        let c0 = if crate::testmode::env("BU_PROF").is_some() { timing::thread_cpu_ms() } else { 0.0 };
        self.ui.update(now);
        // Settings › Glass style changed (a click on the Settings page): the glass brush and the window's tint / rim /
        // bubbles follow at once
        // Settings › Theme / Windows' app theme (Match Windows) changed: the same, plus the outer shadow (Order 033)
        let gs = crate::services::with(|s| s.glass()).unwrap_or_default();
        let light = ui::sync_theme();
        if gs != self.glass_style || light != self.light {
            self.glass_style = gs;
            self.light = light;
            let _ = self.glass.restyle(ui::glass_numbers());
            // the tint is part of the shadow window's picture and of the rim picture (with the outer shadow's pixels inside the
            // window rectangle): both again (Order 033 review: only `draw_rim` lost the corners' shadow ring)
            self.ly.rim_valid = false;
            self.ly.after_valid = false;
            self.restyle_shadow();
            self.ly.valid = false;
            self.ui.dirty = true;
        }
        let p1 = timing::now();
        let (dy, op) = self.ui.window_motion(now);
        // the flyout's own motion: whole pixels by moving the windows, the rest inside the composition
        let dyp = dy * self.scale;
        let iy = dyp.floor() as i32;
        let frac = dyp - iy as f32;
        let pos = (self.x, self.y + iy);
        if pos != self.last_pos {
            unsafe {
                let _ = SetWindowPos(self.hwnd, None, pos.0, pos.1, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW);
            }
        }
        let alpha = (op.clamp(0.0, 1.0) * 255.0).round() as u8;
        if pos != self.last_pos || alpha != self.last_alpha {
            let sx = pos.0 - (SH_L * self.scale).round() as i32;
            let sy = pos.1 - (SH_T * self.scale).round() as i32;
            self.update_shadow(sx, sy, alpha, false);
            self.last_pos = pos;
            self.last_alpha = alpha;
        }
        self.glass.set_opacity(op);
        self.glass.set_offset_y(frac);
        let p2 = timing::now();
        // Order 051: back on the GPU after a lost device (RETRY_MS later), or onto the CPU path when it is lost now
        if self.gpu.is_none() && self.retry_at.is_some_and(|t| now >= t) {
            self.retry_at = None;
            if let Some(g) = gpu::get(Some(self.mon)) {
                if let Err(e) = self.rebuild(Some(g)) {
                    timing::note(&format!("gpu retry failed {:08x}", e.code().0));
                    let _ = self.rebuild(None);
                }
            }
        }
        if self.gpu.as_ref().is_some_and(|g| g.lost()) {
            self.on_lost()?;
            self.glass.set_opacity(op);
            self.glass.set_offset_y(frac);
        }
        self.draw(now)?;
        let p3 = timing::now();
        if let Err(e) = self.present() {
            if self.gpu.is_some() && (gpu::is_lost_error(&e) || self.gpu.as_ref().is_some_and(|g| g.lost())) {
                // the frame is drawn again on the CPU at once: no blank window
                self.on_lost()?;
                self.glass.set_opacity(op);
                self.glass.set_offset_y(frac);
                self.draw(now)?;
                self.present()?;
            } else {
                return Err(e);
            }
        }
        let p4 = timing::now();
        if p4 - p0 > 8.0 {
            timing::note(&format!("slow_frame update {:.1} window {:.1} draw {:.1} present {:.1}", p1 - p0, p2 - p1, p3 - p2, p4 - p3));
        }
        self.frames += 1;
        self.last_frame = timing::now();
        if c0 > 0.0 {
            timing::frame_cpu(now, timing::thread_cpu_ms() - c0);
        } else {
            timing::frame(now);
        }
        if !self.presented_once {
            self.presented_once = true;
            timing::first_present(now);
        }
        self.ui.dirty = false;
        Ok(())
    }

    /// Order 042 (the owner's test 2): after a resolution change Windows moves the open menu along with the old desktop,
    /// where Display's Keep / Revert can be out of reach. It goes back to its corner of the NEW work area (12 px from
    /// the right and above the taskbar, its size unchanged) at once, painted or not.
    pub fn re_anchor(&mut self, work: RECT) {
        if self.offscreen {
            return;
        }
        let m = (12.0 * self.scale).round() as i32;
        // (a work area smaller than the menu: its top-left stays on the screen, like `place`)
        self.x = (work.right - m - self.w).max(work.left + m);
        self.y = (work.bottom - m - self.h).max(work.top + m);
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, self.x, self.y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
        let sx = self.x - (SH_L * self.scale).round() as i32;
        let sy = self.y - (SH_T * self.scale).round() as i32;
        self.update_shadow(sx, sy, self.last_alpha, false);
        self.last_pos = (self.x, self.y);
        self.ui.dirty = true;
    }

    /// Order 051: the GPU device was lost (driver update / crash, `gpulose`): everything again on the CPU path at once, the
    /// GPU tried again `gpu::RETRY_MS` later.
    fn on_lost(&mut self) -> Result<()> {
        gpu::mark_lost();
        self.retry_at = Some(timing::now() + gpu::RETRY_MS + 1.0);
        self.rebuild(None)
    }

    /// The menu's swap chains, glass and layers again on `gpu` (None = the CPU path); the next frame paints everything.
    fn rebuild(&mut self, gpu: Option<Rc<Gpu>>) -> Result<()> {
        let t = timing::now();
        self.glass.close();
        drop_tiles();
        ui::reset_caches();
        self.ui.clear_dock_cache();
        let (gpu, chain, mask) = make_swaps(gpu, &self.g, self.w, self.h)?;
        let (cw, ch) = chain.size();
        self.glass = Glass::new(self.hwnd, (chain.swap(), cw, ch), mask.swap(), None, self.w as f32, self.h as f32, RADIUS * self.scale, self.mode)?;
        self.chain = chain;
        self.mask = mask;
        self.ly = Layers::new(gpu.clone(), self.w, self.h)?;
        self.lc = surf_on(gpu.as_ref(), self.w, self.h)?;
        self.gpu = gpu;
        // the rim picture (and the shadow window) again
        self.restyle_shadow();
        self.glass.set_opacity(self.last_alpha as f32 / 255.0);
        self.slot = true;
        self.ui.dirty = true;
        timing::note(&format!("gpu rebuild path={} {:.1} ms", if self.gpu.is_some() { "gpu" } else { "cpu" }, timing::now() - t));
        Ok(())
    }

    /// Which path draws the menu now (test command `gpustate`).
    pub fn gpu_state(&self) -> String {
        format!("path={} chain_gpu={} device_alive={}", if self.gpu.is_some() { "gpu" } else { "cpu" }, self.chain.is_gpu(), gpu::alive())
    }

    /// Test only (`gpulose`): the device removed as a driver crash would.
    pub fn lose_gpu_for_test(&self) {
        if let Some(g) = &self.gpu {
            g.remove_for_test();
        }
    }

    fn draw(&mut self, now: f64) -> Result<()> {
        self.last_now = now;
        // Order 051: on the GPU the frame is painted straight into the swap chain's back buffer
        if let Swap::Gpu(c) = &self.chain {
            self.lc = c.back();
        }
        let Menu { g, icons, ly, lc, ui, .. } = self;
        lc.canvas().clear(sk::Color::TRANSPARENT);
        Self::compose(g, icons, ui, ly, lc, 0.0, 0.0, false, now);
        if self.mode == crate::comp::GlassMode::Own {
            self.draw_own_glass();
        }
        Ok(())
    }

    /// BU_GLASS=own: the drawing's backdrop recipe on the captured desktop (made again only when the desktop under
    /// the menu or the window's place changed), then the frame over it - like Chromium: the filtered backdrop cut by
    /// the rounded box, the element's layer blended once on top.
    fn draw_own_glass(&mut self) {
        self.backdrop_dirty = false;
        let (Some(cap), Some(gs), Some(out)) = (&self.cap, &mut self.glass_px, &mut self.out) else { return };
        let pos = if self.last_pos.0 == i32::MIN { (self.x, self.y) } else { self.last_pos };
        let pos = (pos.0 + self.cap_origin.0, pos.1 + self.cap_origin.1);
        let t0 = crate::timing::now();
        let c0 = thread_cpu_ms();
        let mut lag = None;
        {
            let shot = cap.shot.lock().unwrap();
            let tl = crate::timing::now();
            // (the glass numbers too: a live theme / glass style switch makes it again - Order 033)
            let key = (shot.seq, pos.0, pos.1, ui::is_light(), self.glass_style);
            if key != self.glass_key && !shot.px.is_empty() {
                let (rw, rh) = (shot.rect.right - shot.rect.left, shot.rect.bottom - shot.rect.top);
                let ii = sk::ImageInfo::new((rw, rh), sk::ColorType::BGRA8888, sk::AlphaType::Premul, Some(sk::ColorSpace::new_srgb()));
                if let Some(img) = sk::images::raster_from_data(&ii, sk::Data::new_copy(&shot.px), (rw * 4) as usize) {
                    // the window's own place inside the captured rectangle; whatever is outside the screen stays empty
                    let mut base = layer(self.w, self.h).ok();
                    if let Some(b) = &mut base {
                        b.canvas().draw_image(&img, ((shot.rect.left - pos.0) as f32, (shot.rect.top - pos.1) as f32), None);
                    }
                    let tb = crate::timing::now();
                    if let Some(mut b) = base {
                        let bimg = b.image_snapshot();
                        gs.canvas().clear(sk::Color::TRANSPARENT);
                        self.g.begin(gs.canvas());
                        self.g.backdrop(&bimg, 0.0, 0.0, WIN_W, WIN_H, RADIUS, 13.0, &[CssColor::Saturate(crate::comp::SATURATE), CssColor::Brightness(crate::comp::BRIGHTNESS)]);
                        self.g.end();
                    }
                    if crate::testmode::env("BU_PROF").is_some() {
                        crate::timing::note(&format!("own_prof lock {:.2} base {:.2} filter {:.2}", tl - t0, tb - tl, crate::timing::now() - tb));
                    }
                }
                if key.0 != self.glass_key.0 {
                    lag = Some(shot.t_present);
                }
                self.glass_key = key;
            }
        }
        let t1 = crate::timing::now();
        let c1 = thread_cpu_ms();
        let c = out.canvas();
        c.clear(sk::Color::TRANSPARENT);
        c.draw_image(gs.image_snapshot(), (0, 0), None);
        c.draw_image(self.lc.image_snapshot(), (0, 0), None);
        if let Some(tp) = lag {
            crate::timing::note(&format!("own_glass blur {:.2} ms compose {:.2} ms since_capture {:.1} ms blur_cpu {:.2} ms", t1 - t0, crate::timing::now() - t1, crate::timing::now() - tp, c1 - c0));
        }
    }

    /// The capture's cost counters (BU_GLASS=own): frames the duplication delivered, copies made, copies unchanged.
    pub fn capture_stats(&self) -> Option<(u64, u64, u64, u64, f64, f64)> {
        self.cap.as_ref().map(|c| {
            let s = c.shot.lock().unwrap();
            (s.frames, s.copies, s.same, s.seq, if s.hand_n > 0 { s.hand_sum / s.hand_n as f64 } else { 0.0 }, s.hand_max)
        })
    }

    /// Paint one frame into `t` with the window's top-left at (`ox`, `oy`) DIPs, layer by layer the way Chromium
    /// composites the drawing (its own layer list, read with tools/ref/cdp_dump.js): the window's own layer (the outer
    /// shadows when `shadows`, the tint, the rim, the caption buttons), the page layer, the live meter layers, the
    /// `::after` layer (the sheen and the accent dot), the top row's icon layers, the popup. A composited layer is
    /// rastered on its own (transparent) and blended once, like Chromium's tiles - the rounding of every blended pixel
    /// depends on it. Like Chromium, a layer is rastered again only when its content changed: the page, `::after` and
    /// top-row layers when anything but the meters moves, the meters every frame, the rim once.
    #[allow(clippy::too_many_arguments)]
    fn compose(g: &Gfx, icons: &Icons, ui: &mut Ui, ly: &mut Layers, t: &mut sk::Surface, ox: f32, oy: f32, shadows: bool, now: f64) {
        let f = Frame { g, icons, now };
        let s = g.scale;
        // test only (Order 013 edge proof): the window's own layer + the sheen, no content (BU_TEST=bare)
        let bare = test_flag("bare");
        // CSS filter blur(N) on the top row, the page and the caption buttons while opening / closing: sigma = N
        let blur = ui.content_blur(now);
        let blur_paint = || {
            let mut p = sk::Paint::default();
            if blur > 0.01 {
                if let Some(fx) = sk::image_filters::blur((blur * s, blur * s), None, None, None) {
                    p.set_image_filter(fx);
                }
            }
            p
        };
        let t0 = crate::timing::now();
        // the snapped page offset is this frame's (set below for the band); a switch's pages are painted without it (the
        // new tab must not take the old tab's scroll)
        ui.page_dy = None;
        let fresh = !ly.valid || shadows || ui.dirty || ui.static_busy(now);
        // a page switch: both pages painted once (at rest), then only moved and faded each frame, like the drawing's
        // composited page layers
        let motion = ui.page_motion(now);
        if let Some(m) = motion {
            let key = (m[0].0, m[1].0, ui.dirty);
            if ly.switch_key != Some((key.0, key.1)) || key.2 {
                let page_top = ((56.0 - ui.page_scroll(now)) * s).round() as i32;
                g.set_pass(Pass::Static);
                raster_tiled(g, &mut ly.page, (0, page_top), || ui.draw_one_page(&f, m[0].0));
                raster_tiled(g, &mut ly.page2, (0, page_top), || ui.draw_one_page(&f, m[1].0));
                g.set_pass(Pass::All);
                ly.switch_key = Some((key.0, key.1));
                ly.valid = false;
            }
        } else if ly.switch_key.is_some() {
            ly.switch_key = None;
            ly.valid = false;
        }
        let invalid = !ly.valid;
        let fresh = fresh || invalid;
        // Order 041: a page of boxes on the screen goes through the tile cache (the band); a legacy page, a switch's two
        // pages and the comparison pictures (`shadows`) are rastered whole as before
        ui.prepare_page(g, now);
        let damage = ui.take_page_damage();
        let band_on = !shadows && motion.is_none() && ui.laid_page() && !ly.no_band;
        if band_on {
            // the content's top on a whole device pixel (Chromium snaps a composited scroll offset the same way)
            let ct = ((PAGE_TOP - ui.page_scroll(now)) * s).round() as i32;
            ui.page_dy = Some(ct as f32 / s);
            ly.band_ct = ct;
            band_frame(g, &f, ui, ly, ct, damage, invalid);
        } else {
            ui.page_dy = None;
            if let Some(b) = &mut ly.band {
                b.ok = false;
            }
        }
        if fresh && motion.is_none() && !band_on {
            // the page layer (static part), rastered alone in Chromium's tiles, which start at the top of the scrolling
            // page content (window y 56 - scroll)
            let page_top = ((56.0 - ui.page_scroll(now)) * s).round() as i32;
            g.set_pass(Pass::Static);
            raster_tiled(g, &mut ly.page, (0, page_top), || ui.draw_pages(&f));
            g.set_pass(Pass::All);
        }
        let tp = crate::timing::now();
        // the ::after layer (the window's box: sheen + bright rim): it depends only on the theme and the glass style, so
        // it is rastered once (and again on a restyle) - not every frame something else moves (Order 041: 3-5 ms a frame)
        if !ly.after_valid {
            raster_tiled(g, &mut ly.after, (0, 0), || {
                ui.draw_edge(g);
            });
            ly.after_valid = true;
        }
        let t1 = crate::timing::now();
        // the live layers (level meters), every frame - on a page that has any (Order 041: an empty full-window layer was
        // cleared and blended every frame)
        let live_on = shadows || motion.is_some() || ui.page_has_live();
        if live_on {
            ly.live.canvas().clear(sk::Color::TRANSPARENT);
            g.begin(ly.live.canvas());
            g.set_pass(Pass::Live);
            ui.draw_pages(&f);
            g.set_pass(Pass::All);
            g.end();
        }
        // the window's own layer: its outer shadows (only in the comparison picture; on screen they are the shadow
        // window), the tint and rim (one picture, made once), the caption buttons
        if !shadows && !ly.rim_valid {
            ly.rim.canvas().clear(sk::Color::TRANSPARENT);
            g.begin(ly.rim.canvas());
            // (normally painted already by render_shadow, with the outer shadow's pixels inside the window rectangle)
            ui.draw_rim(g);
            g.end();
            ly.rim_valid = true;
        }
        let t2 = crate::timing::now();
        let page = ly.page.image_snapshot();
        let live = ly.live.image_snapshot();
        let after = ly.after.image_snapshot();
        let (dx, dy) = ((ox * s).round(), (oy * s).round());
        g.begin(t.canvas());
        g.cv().translate((ox, oy));
        if shadows {
            g.box_shadows(0.0, 0.0, WIN_W, WIN_H, RADIUS, &self::shadows(), false);
            ui.draw_rim(g);
        } else {
            // blended onto the cleared frame = the same pixels as painting it there
            let rim = ly.rim.image_snapshot();
            let c = g.cv();
            c.save();
            c.reset_matrix();
            c.draw_image(&rim, (dx, dy), None);
            c.restore();
        }
        if blur > 0.01 {
            if let Some(fx) = sk::image_filters::blur((blur, blur), None, None, None) {
                g.push_filter(fx);
                if !bare {
                    ui.draw_caps(&f);
                }
                g.pop_filter();
            }
        } else if !bare {
            ui.draw_caps(&f);
        }
        {
            let c = g.cv();
            c.save();
            c.reset_matrix();
            let p = blur_paint();
            match motion {
                Some(m) => {
                    // the two pages' layers, moved (bilinear, like the compositor) and faded - only inside the page's
                    // view, where their pixels are (whole device rows around it: nothing of them is cut)
                    let page2 = ly.page2.image_snapshot();
                    c.save();
                    c.clip_rect(sk::Rect::from_ltrb(dx, dy + (PAGE_TOP * s).floor() - 1.0, dx + (WIN_W * s).ceil(), dy + ((PAGE_TOP + PAGE_H) * s).ceil() + 1.0), sk::ClipOp::Intersect, false);
                    for (img, (_, mx, mo)) in [(&page, m[0]), (&page2, m[1])] {
                        if mo <= 0.001 {
                            continue;
                        }
                        let mut pp = p.clone();
                        pp.set_alpha_f(mo.clamp(0.0, 1.0));
                        let x = dx + mx * s;
                        let (xi, fr) = (x.floor(), x - x.floor());
                        if blur > 0.01 {
                            let so = sk::SamplingOptions::new(sk::FilterMode::Linear, sk::MipmapMode::None);
                            c.draw_image_with_sampling_options(img, (x, dy), so, Some(&pp));
                        } else {
                            // the same bilinear move as one sideways step: the picture at the whole pixel and at the next one,
                            // weighted (1 - f) and f, added in a layer that is then faded - two plain copies instead of a
                            // filtered draw (Order 041: ~1 ms less per page and frame)
                            let a2 = (fr * 255.0).round() as u8;
                            c.save_layer(&sk::canvas::SaveLayerRec::default().paint(&pp));
                            let mut a = sk::Paint::default();
                            a.set_alpha(255 - a2);
                            a.set_blend_mode(sk::BlendMode::Src);
                            c.draw_image(img, (xi, dy), Some(&a));
                            if a2 > 0 {
                                let mut b = sk::Paint::default();
                                b.set_alpha(a2);
                                b.set_blend_mode(sk::BlendMode::Plus);
                                c.draw_image(img, (xi + 1.0, dy), Some(&b));
                            }
                            c.restore();
                        }
                    }
                    c.restore();
                }
                None if bare => {}
                None if band_on => {
                    // the band, moved to the scroll and cut to the page's view (as the page layer was clipped)
                    let ct = ly.band_ct;
                    if let Some(b) = &mut ly.band {
                        let img = b.surf.image_snapshot();
                        let y = dy + (ct + b.b0) as f32;
                        let clip = sk::Rect::from_xywh(dx, dy + PAGE_TOP * s, WIN_W * s, PAGE_H * s);
                        if blur > 0.01 {
                            // blurred while opening / closing: the cut page is blurred, as the clipped layer was
                            c.save_layer(&sk::canvas::SaveLayerRec::default().paint(&p));
                            c.clip_rect(clip, sk::ClipOp::Intersect, true);
                            c.draw_image(&img, (dx, y), None);
                            c.restore();
                        } else {
                            c.save();
                            c.clip_rect(clip, sk::ClipOp::Intersect, true);
                            c.draw_image(&img, (dx, y), None);
                            c.restore();
                        }
                    }
                }
                None => {
                    c.draw_image(&page, (dx, dy), Some(&p));
                }
            }
            if !bare && live_on {
                c.draw_image(&live, (dx, dy), Some(&p));
            }
            c.draw_image(&after, (dx, dy), None);
            c.restore();
        }
        g.end();
        let t3 = crate::timing::now();
        // the top row's layers (they never overlap each other, so one picture of them blends the same). Painted again only
        // when something in it changed (Order 041: 1-2.5 ms a frame)
        let dsig = ui.dock_sig(now);
        let mut dock_drawn = 0;
        if invalid || shadows || dsig.is_none() || dsig != ly.dock_sig {
            ly.dock_sig = dsig;
            dock_drawn = 1;
            ly.dock.canvas().clear(sk::Color::TRANSPARENT);
            g.begin(ly.dock.canvas());
            ui.draw_dock(&f);
            g.end();
        }
        let dock = ly.dock.image_snapshot();
        let tsb = crate::timing::now();
        // the top row's hover names and chevrons, and the page's glass scrollbar: every frame they show, into their own
        // layer, frosting the frame so far (the page, its meters, the ::after layer - not the top row: Chromium's
        // backdrop root is the window's content)
        let sb = !bare && ui.scrollbar_shown(now);
        let ovl = !bare && ui.dock_overlays_shown(now);
        // where they are (window device px): the top band (chevrons, names) and the scrollbar's column
        // (the name label's shadow reaches ~110 DIPs down)
        let band_r = sk::IRect::from_ltrb(0, 0, ly.over.width(), (120.0 * s).ceil() as i32);
        // (the scrollbar at x 588 frosts 8 px blur = up to 3 sigma = 24 px to its left)
        let col_r = sk::IRect::from_ltrb((556.0 * s).floor() as i32, band_r.bottom, ly.over.width(), ly.over.height());
        if sb || ovl {
            let base = t.image_snapshot_with_bounds(sk::IRect::from_xywh(dx as i32, dy as i32, ly.over.width(), ly.over.height())).unwrap_or_else(|| page.clone());
            OV_PROF.with(|o| o.set((crate::timing::now() - tsb, 0.0, 0.0)));
            // the same state over the same pixels: the same picture - kept (a hover name over the Audio meters' page was
            // painted again every frame: its 30 px frost alone ~1.5 ms)
            let key = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (ui.overlay_sig(now), sb, ovl).hash(&mut h);
                if let Some(pm) = base.peek_pixels() {
                    let rb = pm.row_bytes();
                    if let Some(px) = pm.bytes() {
                        for r in [band_r, col_r] {
                            for y in r.top..r.bottom.min(pm.height()) {
                                let o = y as usize * rb;
                                px[o + r.left as usize * 4..o + (r.right.min(pm.width()) as usize) * 4].hash(&mut h);
                            }
                        }
                    }
                }
                h.finish()
            };
            let th = crate::timing::now();
            // (on the GPU there are no CPU pixels to compare: painted every frame they show - cheap there)
            if invalid || shadows || ly.gpu.is_some() || ly.over_key != Some(key) {
                ly.over_key = Some(key);
                ly.over.canvas().clear(sk::Color::TRANSPARENT);
                g.begin(ly.over.canvas());
                if ovl {
                    ui.draw_dock_overlays(&f, &base);
                }
                if sb {
                    ui.draw_page_scrollbar(&f, &base);
                }
                g.end();
            }
            OV_PROF.with(|o| {
                let v = o.get();
                o.set((v.0, th - tsb - v.0, crate::timing::now() - th));
            });
        }
        {
            let c = t.canvas();
            c.save();
            c.reset_matrix();
            // each picture only where it has pixels (their layers are window-sized and clear elsewhere)
            let clip = |c: &sk::Canvas, r: sk::IRect| {
                c.save();
                c.clip_rect(sk::Rect::from(r.with_offset((dx as i32, dy as i32))), sk::ClipOp::Intersect, false);
            };
            if !bare {
                clip(c, sk::IRect::from_ltrb(0, 0, ly.dock.width(), (64.0 * s).ceil() as i32));
                c.draw_image(&dock, (dx, dy), Some(&blur_paint()));
                c.restore();
            }
            if sb || ovl {
                let over = ly.over.image_snapshot();
                for r in [band_r, col_r] {
                    clip(c, r);
                    c.draw_image(&over, (dx, dy), Some(&blur_paint()));
                    c.restore();
                }
            }
            c.restore();
        }
        let t4 = crate::timing::now();
        // the popup, above everything (+ the shared tooltip)
        ui.update_tips(g, now);
        if ui.popup_open() {
            // the WHOLE target as it is (device pixels): `backdrop` draws its base at device 0,0, so a crop at (dx, dy) put
            // the wrong part behind the popup in the comparison picture (the window at dx, dy != 0) - Lane R 09:01
            let base = t.image_snapshot();
            g.begin(t.canvas());
            g.cv().translate((ox, oy));
            ui.draw_popup(&f, &base);
            g.end();
        }
        let t5 = crate::timing::now();
        if crate::testmode::env("BU_PROF").is_some() {
            crate::timing::note(&format!(
                "prof fresh {} static {:.2} live {:.2} composite {:.2} dock {:.2} blur {:.1} page {:.2} after {:.2} build {:.2} popup {:.2} sbar {:.2} tiles {} dockpaint {} ovsnap {:.2} ovhash {:.2} ovdraw {:.2}",
                fresh,
                t1 - t0,
                t2 - t1,
                t3 - t2,
                t4 - t3,
                blur,
                tp - t0,
                t1 - tp,
                ui::take_build_ms(),
                t5 - t4,
                t4 - tsb,
                BAND_TILES.with(|b| b.replace(0)),
                dock_drawn,
                OV_PROF.with(|o| o.get().0),
                OV_PROF.with(|o| o.get().1),
                OV_PROF.with(|o| o.replace((0.0, 0.0, 0.0)).2)
            ));
        }
        let _ = t5;
        ly.valid = true;
    }

    /// Hand the finished frame to the swap chain.
    fn present(&mut self) -> Result<()> {
        let chain = match &mut self.chain {
            Swap::Gpu(c) => return c.present(),
            Swap::Cpu(c) => c,
        };
        let src = match &mut self.out {
            Some(o) if self.mode == crate::comp::GlassMode::Own => o,
            _ => &mut self.lc,
        };
        let pm = src.peek_pixels().ok_or_else(|| Error::from(E_FAIL))?;
        let rb = pm.row_bytes();
        let bytes = pm.bytes().ok_or_else(|| Error::from(E_FAIL))?;
        chain.present(bytes, rb as u32)
    }

    /// Order 041 self-check (test command `verify`): this frame painted the incremental way (tile cache, kept top row,
    /// kept overlays) and again from scratch at the same moment - how many pixels differ and by how much at most. 0 =
    /// the caches never show a stale or different pixel.
    pub fn verify_incremental(&mut self, now: f64) -> Result<((usize, u8, Option<(u32, u32)>), (usize, u8, Option<(u32, u32)>))> {
        self.ui.update(now);
        self.draw(now)?;
        let a = crate::png::from_surface(&mut self.lc);
        self.ly.valid = false;
        self.ly.band = None;
        self.ly.dock_sig = None;
        self.ly.over_key = None;
        self.ui.clear_dock_cache();
        self.draw(now)?;
        let b = crate::png::from_surface(&mut self.lc);
        // and the way before Order 041 (the whole page rastered, no tile cache)
        self.ly.valid = false;
        self.ly.no_band = true;
        let c = self.draw(now).map(|_| crate::png::from_surface(&mut self.lc));
        self.ly.no_band = false;
        self.ly.valid = false;
        let c = c?;
        let cmp = |a: &crate::png::Pixels, b: &crate::png::Pixels| {
            let (mut n, mut mx, mut first) = (0usize, 0u8, None);
            for (i, (p, q)) in a.data.chunks(4).zip(b.data.chunks(4)).enumerate() {
                let d = p.iter().zip(q).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
                if d > 0 {
                    n += 1;
                    mx = mx.max(d);
                    first.get_or_insert(((i as u32) % a.w, (i as u32) / a.w));
                }
            }
            (n, mx, first)
        };
        Ok((cmp(&a, &b), cmp(&b, &c)))
    }

    /// The last finished frame as pixels - test pictures without the screen.
    pub fn snapshot(&mut self) -> Result<crate::png::Pixels> {
        Ok(crate::png::from_surface(&mut self.lc))
    }

    /// A comparison picture without the screen: the drawing's desktop picture (`desk`, device pixels) with the glass
    /// recipe applied the way the drawing's CSS does it (backdrop-filter blur 13 px + saturate 170 % + brightness 1.04
    /// inside the rounded window, edges mirrored; the outer shadow), then this menu's real frame (layer C) on top.
    /// Only the backdrop is simulated here - on the PC it comes from Windows.
    pub fn snapshot_over(&mut self, desk: &str) -> Result<crate::png::Pixels> {
        let dp = crate::png::load_png(desk)?;
        let dimg = crate::png::to_image(&dp).ok_or_else(|| Error::from(E_FAIL))?;
        let (sw, sh) = (dp.w as i32, dp.h as i32);
        let mut out = layer(sw, sh)?;
        let s = self.scale;
        // the flyout's place in the drawing's picture (taskbar 48 px): right 12, bottom 48 + 12
        let wx = sw as f32 / s - 12.0 - WIN_W;
        let wy = sh as f32 / s - 48.0 - 12.0 - WIN_H;
        out.canvas().draw_image(&dimg, (0, 0), None);
        // what Chromium's compositor does for an element with backdrop-filter: first the filtered backdrop inside the
        // rounded box, then the element's own render surface (all its layers, outer shadows included) blended once
        let g = &self.g;
        g.begin(out.canvas());
        let gn = ui::glass_numbers();
        let mut fx = vec![CssColor::Saturate(gn.saturate), CssColor::Brightness(gn.brightness)];
        // Order 041: the adaptive cap / floor the compositor applies on screen
        fx.extend(crate::comp::adapt_filter(crate::comp::adapt(&gn, ui::is_light())));
        g.backdrop(&dimg, wx, wy, WIN_W, WIN_H, RADIUS, gn.blur_px, &fx);
        g.end();
        // the window's own layer on a transparent surface of its own, as on the screen: the top row's frosted parts
        // (hover chevrons, names) read only the window's content (Chromium: #sw is their backdrop root), never the
        // desktop under it (Order 014: hover chevron 674 px before)
        let mut win = surf_on(self.gpu.as_ref(), sw, sh)?;
        let Menu { g, icons, ly, ui, last_now, .. } = self;
        Self::compose(g, icons, ui, ly, &mut win, wx, wy, true, *last_now);
        out.canvas().draw_image(win.image_snapshot(), (0, 0), None);
        Ok(crate::png::from_surface(&mut out))
    }

    /// The on-screen picture rebuilt from the very pieces the screen gets (Order 013 edge proof), over a desktop
    /// picture `desk`: the glass = the drawing's backdrop recipe on `desk` (standing in for Windows' backdrop) times
    /// the window's MASK pixels, then the menu window's real frame, then the shadow window's real pixels - stacked the
    /// way Windows composites the two windows and the menu's two visuals. Same place as `snapshot_over`.
    pub fn snapshot_screen(&mut self, desk: &str) -> Result<crate::png::Pixels> {
        let dp = crate::png::load_png(desk)?;
        let dimg = crate::png::to_image(&dp).ok_or_else(|| Error::from(E_FAIL))?;
        let (sw, sh) = (dp.w as i32, dp.h as i32);
        let s = self.scale;
        let wx = sw as f32 / s - 12.0 - WIN_W;
        let wy = sh as f32 / s - 48.0 - 12.0 - WIN_H;
        let (dx, dy) = ((wx * s).round() as i32, (wy * s).round() as i32);
        // the glass visual: backdrop (the whole window box) x mask alpha
        let mut gl = layer(sw, sh)?;
        {
            let g = &self.g;
            g.begin(gl.canvas());
            let mut fx = vec![CssColor::Saturate(crate::comp::SATURATE), CssColor::Brightness(crate::comp::BRIGHTNESS)];
            fx.extend(crate::comp::adapt_filter(crate::comp::adapt(&ui::glass_numbers(), ui::is_light())));
            g.backdrop_in(&dimg, wx, wy, WIN_W, WIN_H, None, 13.0, &[], &fx);
            g.end();
        }
        // backdrop x mask and the blend onto the desktop in ONE draw (one rounding), as the compositor does it on the GPU
        let mimg = crate::png::to_image(&self.mask_pixels()).ok_or_else(|| Error::from(E_FAIL))?;
        let glimg = gl.image_snapshot();
        let so = sk::SamplingOptions::default();
        let gsh = glimg.to_shader((sk::TileMode::Decal, sk::TileMode::Decal), so, None).ok_or_else(|| Error::from(E_FAIL))?;
        let msh = mimg.to_shader((sk::TileMode::Decal, sk::TileMode::Decal), so, &sk::Matrix::translate((dx as f32, dy as f32))).ok_or_else(|| Error::from(E_FAIL))?;
        let mut gp = sk::Paint::default();
        gp.set_shader(sk::shaders::blend(sk::BlendMode::DstIn, gsh, msh));
        let frame = self.lc.image_snapshot();
        let mut out = surf_on(self.gpu.as_ref(), sw, sh)?;
        let c = out.canvas();
        c.draw_image(&dimg, (0, 0), None);
        c.draw_rect(sk::Rect::from_xywh(dx as f32, dy as f32, self.w as f32, self.h as f32), &gp);
        c.draw_image(&frame, (dx, dy), None);
        if let Some(sp) = &self.sh_px {
            let simg = crate::png::to_image(sp).ok_or_else(|| Error::from(E_FAIL))?;
            c.draw_image(&simg, (dx - (SH_L * s).round() as i32, dy - (SH_T * s).round() as i32), None);
        }
        Ok(crate::png::from_surface(&mut out))
    }

    /// Option 3's pixel check (Order 013): on a desktop picture `desk`, the drawing's backdrop recipe by Skia (exact,
    /// matches Chromium) vs the same recipe by Direct2D's effects (what the compositor runs in `BU_GLASS=gpu`), over the
    /// whole window box (no rounded cut). Saves Skia | Direct2D | difference (x8) to `out`; returns the counts.
    pub fn gpu_diff(&mut self, desk: &str, out: &str) -> Result<String> {
        let dp = crate::png::load_png(desk)?;
        let dimg = crate::png::to_image(&dp).ok_or_else(|| Error::from(E_FAIL))?;
        let (sw, sh) = (dp.w as i32, dp.h as i32);
        let s = self.scale;
        let wx = sw as f32 / s - 12.0 - WIN_W;
        let wy = sh as f32 / s - 48.0 - 12.0 - WIN_H;
        let (dx, dy) = ((wx * s).round() as i32, (wy * s).round() as i32);
        let mut sl = layer(sw, sh)?;
        self.g.begin(sl.canvas());
        self.g.backdrop_in(&dimg, wx, wy, WIN_W, WIN_H, None, 13.0, &[], &[CssColor::Saturate(crate::comp::SATURATE), CssColor::Brightness(crate::comp::BRIGHTNESS)]);
        self.g.end();
        let full = crate::png::from_surface(&mut sl);
        let t0 = crate::timing::now();
        let d3d = match &self.chain {
            Swap::Cpu(c) => c.d3d.clone(),
            Swap::Gpu(_) => crate::present::new_d3d()?,
        };
        let d2 = crate::d2dref::blur_d2d(&d3d, &dp, (dx, dy, self.w, self.h), 13.0 * s, crate::comp::SATURATE, crate::comp::BRIGHTNESS)?;
        let t1 = crate::timing::now();
        let (w, h) = (self.w as usize, self.h as usize);
        let mut side = crate::png::Pixels { w: (w * 3) as u32, h: h as u32, data: vec![0; w * 3 * h * 4] };
        let (mut n, mut max, mut sum) = (0u64, 0i32, 0u64);
        let mut hist = [0u64; 6];
        for y in 0..h {
            for x in 0..w {
                let a = &full.data[(((y as i32 + dy) as usize) * sw as usize + (x as i32 + dx) as usize) * 4..][..4];
                let b = &d2.data[(y * w + x) * 4..][..4];
                let d = (0..3).map(|c| (a[c] as i32 - b[c] as i32).abs()).max().unwrap_or(0);
                if d > 0 {
                    n += 1;
                }
                max = max.max(d);
                sum += d as u64;
                hist[(d as usize).min(5)] += 1;
                let row = y * w * 3;
                side.data[(row + x) * 4..][..4].copy_from_slice(&[a[0], a[1], a[2], 255]);
                side.data[(row + w + x) * 4..][..4].copy_from_slice(&[b[0], b[1], b[2], 255]);
                let v = (d * 8).min(255) as u8;
                side.data[(row + 2 * w + x) * 4..][..4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        crate::png::save_png(&side, out)?;
        Ok(format!(
            "window box {}x{} px: {} differ ({:.2} %), max {} levels, mean {:.3} levels; pixels by difference 0/1/2/3/4/5+: {:?}; Direct2D took {:.1} ms (CPU side, incl. device setup)
",
            w,
            h,
            n,
            100.0 * n as f64 / (w * h) as f64,
            max,
            sum as f64 / (w * h) as f64,
            hist,
            t1 - t0
        ))
    }

    /// The window's edge mask pixels (white, alpha = coverage).
    pub fn mask_pixels(&self) -> crate::png::Pixels {
        mask_raster(&self.g, self.w, self.h).unwrap_or(crate::png::Pixels { w: 0, h: 0, data: Vec::new() })
    }

    pub fn waitable(&self) -> HANDLE {
        self.chain.waitable()
    }

    pub fn device_lost_hint(&self) -> bool {
        false
    }
}

impl Drop for Menu {
    fn drop(&mut self) {
        unsafe {
            ui::reset_caches();
            self.glass.close();
            self.chain.release();
            self.mask.release();
            // (the GPU tile goes too: nothing of ours may keep the device alive while the menu is closed - Order 051)
            drop_tiles();
            // (the capture chain is the worker's: not touched here, it goes when the worker lets go of it)
            let _ = DestroyWindow(self.shadow);
            let _ = DestroyWindow(self.hwnd);
            if !self.sh_dc.is_invalid() {
                SelectObject(self.sh_dc, self.sh_old);
                let _ = DeleteObject(self.sh_bmp.into());
                let _ = DeleteDC(self.sh_dc);
            }
        }
    }
}

/// CPU time this thread has used (user + kernel), ms - for the cost measurements.
fn thread_cpu_ms() -> f64 {
    unsafe {
        let (mut a, mut b, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let _ = windows::Win32::System::Threading::GetThreadTimes(windows::Win32::System::Threading::GetCurrentThread(), &mut a, &mut b, &mut k, &mut u);
        let f = |t: FILETIME| ((t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64) as f64 / 10_000.0;
        f(k) + f(u)
    }
}

/// Test-only experiment switches for real-screen proof pictures (Order 013): BU_TEST=noshadow,noregion,noaccent
pub fn test_flag(name: &str) -> bool {
    crate::testmode::env("BU_TEST").map(|v| v.split(',').any(|f| f == name)).unwrap_or(false)
}

/// Proof for the test runs: where the window really is (screen pixels) and whether it is on any monitor.
pub fn log_rect(what: &str, hwnd: HWND) {
    unsafe {
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        let on_screen = !MonitorFromRect(&r, MONITOR_DEFAULTTONULL).is_invalid();
        crate::timing::note(&format!("rect {} {} {} {} {} on_screen={}", what, r.left, r.top, r.right, r.bottom, on_screen));
    }
}

/// Windows' own blur of the desktop behind the window (SetWindowCompositionAttribute, accent "blur behind"; user32,
/// undocumented but stable since Windows 10). No colour from Windows: our tint is drawn in the composition.
fn blur_behind(hwnd: HWND) -> bool {
    #[repr(C)]
    struct AccentPolicy {
        state: u32,
        flags: u32,
        color: u32,
        animation: u32,
    }
    #[repr(C)]
    struct Data {
        attrib: u32,
        data: *mut core::ffi::c_void,
        size: usize,
    }
    type Swca = unsafe extern "system" fn(HWND, *mut Data) -> BOOL;
    unsafe {
        let Ok(user) = windows::Win32::System::LibraryLoader::GetModuleHandleW(w!("user32.dll")) else { return false };
        let Some(f) = windows::Win32::System::LibraryLoader::GetProcAddress(user, s!("SetWindowCompositionAttribute")) else { return false };
        let f: Swca = std::mem::transmute(f);
        let mut ap = AccentPolicy { state: 3, flags: 0, color: 0, animation: 0 }; // ACCENT_ENABLE_BLURBEHIND
        let mut d = Data { attrib: 19, data: &mut ap as *mut _ as *mut _, size: std::mem::size_of::<AccentPolicy>() }; // WCA_ACCENT_POLICY
        f(hwnd, &mut d).as_bool()
    }
}

thread_local! {
    /// A painter made and primed on tray hover (see App::warm_up), handed to the next open; None = make one per open.
    pub static KEPT_GFX: std::cell::RefCell<Option<Gfx>> = const { std::cell::RefCell::new(None) };
}

/// Chromium's raster tiles (cc TilingData): 256 x 256 tiles with a 1 px border shared between neighbours, so tile k's
/// texture starts at 254 k and it owns the pixels from 254 k + 1 (tile 0 from 0) up to 254 (k + 1). Skia's gradient
/// dither is a pattern on the pixels of the canvas it draws into, so it restarts in every tile - measured in the Order 003
/// sweep: the sheen's dither moves by 2 px at x = 255 and y = 255 and by 4 px at x = 509.
const TILE: i32 = 256;
const TILE_STEP: i32 = TILE - 2;

thread_local! {
    /// the tile surfaces: [CPU, GPU] (a tile is rastered on the same kind of surface as the layer it goes into)
    static TILE_SURF: std::cell::RefCell<[Option<sk::Surface>; 2]> = const { std::cell::RefCell::new([None, None]) };
}

/// Let go of the tile surfaces (the GPU one holds the device).
fn drop_tiles() {
    TILE_SURF.with(|t| *t.borrow_mut() = [None, None]);
}

/// A tile surface of the same kind as `like` (GPU or CPU).
fn with_tile<R>(like: &mut sk::Surface, f: impl FnOnce(&mut sk::Surface, &mut sk::Surface) -> R) -> R {
    let k = like.recording_context().is_some() as usize;
    TILE_SURF.with(|ts| {
        let mut ts = ts.borrow_mut();
        if ts[k].is_none() {
            ts[k] = if k == 1 { like.new_surface_with_dimensions((TILE, TILE)) } else { None }.or_else(|| new_surface(TILE, TILE));
        }
        f(ts[k].as_mut().expect("tile surface"), like)
    })
}

/// Raster one composited layer into `surf` (window-sized, device pixels) tile by tile, the layer's tile grid starting at
/// `grid` (device pixels in `surf`), with `draw` painting the layer's content in window DIPs. Like Chromium, the
/// content is recorded once (an SkPicture with a bounding-box hierarchy) and played back into each tile.
fn raster_tiled(g: &Gfx, surf: &mut sk::Surface, grid: (i32, i32), mut draw: impl FnMut()) {
    let (w, h) = (surf.width(), surf.height());
    surf.canvas().clear(sk::Color::TRANSPARENT);
    let mut rec = sk::PictureRecorder::new();
    let rc = rec.begin_recording(sk::Rect::from_iwh(w, h), true);
    g.begin(rc);
    draw();
    g.end();
    let Some(pic) = rec.finish_recording_as_picture(None) else { return };
    with_tile(surf, |tile, surf| {
        // owned span of tile k along one axis, in layer pixels
        let span = |k: i32| (if k == 0 { 0 } else { TILE_STEP * k + 1 }, TILE_STEP * (k + 1) + 1);
        let range = |o: i32, n: i32| (((-o) - 1).div_euclid(TILE_STEP).max(0), (n - o).div_euclid(TILE_STEP));
        let (kx0, kx1) = range(grid.0, w);
        let (ky0, ky1) = range(grid.1, h);
        for ky in ky0..=ky1 {
            for kx in kx0..=kx1 {
                let (ox, oy) = (grid.0 + TILE_STEP * kx, grid.1 + TILE_STEP * ky);
                let (ax, bx) = span(kx);
                let (ay, by) = span(ky);
                let own = sk::IRect::new((grid.0 + ax).max(0), (grid.1 + ay).max(0), (grid.0 + bx).min(w), (grid.1 + by).min(h));
                if own.is_empty() {
                    continue;
                }
                let c = tile.canvas();
                c.clear(sk::Color::TRANSPARENT);
                c.save();
                // like cc: the whole tile texture (border included) is rastered, only the owned part is used
                c.clip_irect(sk::IRect::from_wh(TILE, TILE), sk::ClipOp::Intersect);
                c.translate((-ox as f32, -oy as f32));
                c.draw_picture(&pic, None, None);
                c.restore();
                let img = tile.image_snapshot();
                let mut p = sk::Paint::default();
                p.set_blend_mode(sk::BlendMode::Src);
                let src = sk::Rect::from(own.with_offset((-ox, -oy)));
                surf.canvas().draw_image_rect(img, Some((&src, sk::canvas::SrcRectConstraint::Strict)), sk::Rect::from(own), &p);
            }
        }
    });
}

/// The separately rastered layers of the menu (window-sized, device pixels).
struct Layers {
    /// Order 051: the GPU the layers live on (None = CPU surfaces)
    gpu: Option<Rc<Gpu>>,
    /// the page layer (the drawing's scrolling `.pg` layer + its scrollbar layer), static part
    page: sk::Surface,
    /// the second page while a page switch runs (the new one; `page` holds the old one then)
    page2: sk::Surface,
    switch_key: Option<(usize, usize)>,
    /// the `::after` layer (the sheen + the bright rim)
    after: sk::Surface,
    /// the level meters (the drawing's canvases, range inputs and `.lvl i` layers)
    live: sk::Surface,
    /// the top row's icon layers and hover names
    dock: sk::Surface,
    /// the window's own background: tint + rim
    rim: sk::Surface,
    /// Order 041: the small things painted every frame they show (the top row's names and chevrons, the page's
    /// scrollbar), over the top row
    over: sk::Surface,
    /// what `over` was painted from (state + the pixels under it)
    over_key: Option<u64>,
    /// test only (`verify`): the page rastered whole, the way before the tile cache
    no_band: bool,
    valid: bool,
    rim_valid: bool,
    /// the `::after` layer is painted (it changes only with the theme / glass style)
    after_valid: bool,
    /// Order 041: the shown page's tile cache (scrolling moves it, a change rasters only its tiles) and where the frame
    /// puts it (the content's top, window device px)
    band: Option<Band>,
    band_ct: i32,
    /// what the top row's picture was painted from (`Ui::dock_sig`): the same = it is not painted again
    dock_sig: Option<u64>,
}

impl Layers {
    fn new(gpu: Option<Rc<Gpu>>, w: i32, h: i32) -> Result<Layers> {
        let g = gpu.as_ref();
        Ok(Layers {
            page: surf_on(g, w, h)?,
            page2: surf_on(g, w, h)?,
            switch_key: None,
            after: surf_on(g, w, h)?,
            live: surf_on(g, w, h)?,
            dock: surf_on(g, w, h)?,
            rim: surf_on(g, w, h)?,
            over: surf_on(g, w, h)?,
            over_key: None,
            no_band: false,
            valid: false,
            rim_valid: false,
            after_valid: false,
            band: None,
            band_ct: 0,
            dock_sig: None,
            gpu,
        })
    }
}

/// Order 041 (smoothness, the owner Oct 8: "like its 60hz"): the shown page's content rastered in Chromium's tiles (256 px,
/// anchored at the content's top, exactly as `raster_tiled` does at a whole-pixel scroll) into a band a little taller
/// than the view. Scrolling only moves the band (and rasters the tile rows that come into view); a change of the page
/// rasters only the tiles its boxes touch (`ui::damage`). Before, every frame with anything moving rastered the whole
/// page (10-30 ms on the CPU at 1x - 40-60 frames a second at most).
struct Band {
    surf: sk::Surface,
    /// the content row (device px) at the band's row 0
    b0: i32,
    tab: usize,
    /// tile rows rastered and still right
    rows: std::collections::BTreeSet<i32>,
    ok: bool,
}

/// The tile (row or column) a content pixel belongs to (its owned span, `raster_tiled`).
fn tile_of(r: i32) -> i32 {
    if r <= TILE_STEP {
        0
    } else {
        (r - 1) / TILE_STEP
    }
}

/// The pixels tile k owns along one axis (content px): [start, end).
fn tile_span(k: i32) -> (i32, i32) {
    (if k == 0 { 0 } else { TILE_STEP * k + 1 }, TILE_STEP * (k + 1) + 1)
}

/// Bring the band up to date for a frame whose content top sits at window device row `ct`: re-centre it when the view
/// leaves it, raster the tile rows of the view it lacks and the tiles `damage` touches. True = it rastered something.
fn band_frame(g: &Gfx, f: &Frame, ui: &mut Ui, ly: &mut Layers, ct: i32, damage: Option<ui::damage::Damage>, reset: bool) -> bool {
    use ui::damage::Damage;
    let s = g.scale;
    let w = ly.page.width();
    let hb = (PAGE_H * s).ceil() as i32 + 2 + 4 * TILE;
    if ly.band.as_ref().is_none_or(|b| b.surf.width() != w || b.surf.height() != hb) {
        let Ok(surf) = surf_on(ly.gpu.as_ref(), w, hb) else { return false };
        ly.band = Some(Band { surf, b0: i32::MIN, tab: usize::MAX, rows: Default::default(), ok: false });
    }
    let b = ly.band.as_mut().unwrap();
    let tab = ui.tab;
    if reset || !b.ok || b.tab != tab || damage == Some(Damage::Full) {
        b.rows.clear();
        b.ok = true;
        b.tab = tab;
        b.b0 = i32::MIN;
        b.surf.canvas().clear(sk::Color::TRANSPARENT);
    }
    // the view's content rows and the tile rows over them
    let vt = ((PAGE_TOP * s).floor() as i32 - ct).max(0);
    let vb = (((PAGE_TOP + PAGE_H) * s).ceil() as i32 - ct).max(vt + 1);
    let (k0, k1) = (tile_of(vt), tile_of(vb - 1));
    let (r0, r1) = (tile_span(k0).0, tile_span(k1).1);
    if b.b0 == i32::MIN || r0 < b.b0 || r1 > b.b0 + hb {
        let nb0 = (r0 - (hb - (r1 - r0)) / 2).max(0);
        if b.b0 != i32::MIN {
            // what is still inside moves with it (whole pixels: the same pixels as rastering it there)
            let img = b.surf.image_snapshot();
            let c = b.surf.canvas();
            c.clear(sk::Color::TRANSPARENT);
            let mut p = sk::Paint::default();
            p.set_blend_mode(sk::BlendMode::Src);
            c.draw_image(&img, (0, b.b0 - nb0), Some(&p));
            b.rows.retain(|&k| {
                let (a, e) = tile_span(k);
                a >= nb0 && e <= nb0 + hb
            });
        }
        b.b0 = nb0;
    }
    // the tiles to raster: rows of the view not there yet; the changed tiles of rows that are (a changed row outside the
    // view is only forgotten - rastered when it comes into view)
    let cols = tile_of(w - 1);
    let mut todo: std::collections::BTreeSet<(i32, i32)> = Default::default();
    for ky in k0..=k1 {
        if !b.rows.contains(&ky) {
            for kx in 0..=cols {
                todo.insert((ky, kx));
            }
        }
    }
    if let Some(Damage::Rects(rs)) = &damage {
        for &(x, y, dw, dh) in rs {
            // (inside the band: a box of an absurd size never loops over millions of tile rows)
            let (x0, y0) = ((x * s).floor() as i32, ((y * s).floor() as i32).max(b.b0));
            let (x1, y1) = (((x + dw) * s).ceil() as i32, (((y + dh) * s).ceil() as i32).min(b.b0 + hb));
            if x1 <= x0 || y1 <= y0 || y1 <= 0 || x1 <= 0 || x0 >= w {
                continue;
            }
            for ky in tile_of(y0.max(0))..=tile_of(y1 - 1) {
                if !b.rows.contains(&ky) {
                    continue;
                }
                if ky < k0 || ky > k1 {
                    b.rows.remove(&ky);
                    continue;
                }
                for kx in tile_of(x0.max(0))..=tile_of((x1 - 1).min(w - 1)) {
                    todo.insert((ky, kx));
                }
            }
        }
    }
    if todo.is_empty() {
        return false;
    }
    let ky_lo = todo.iter().map(|t| t.0).min().unwrap_or(0);
    let ky_hi = todo.iter().map(|t| t.0).max().unwrap_or(0);
    let mut rec = sk::PictureRecorder::new();
    let rc = rec.begin_recording(sk::Rect::from_ltrb(0.0, (TILE_STEP * ky_lo) as f32, w as f32, (TILE_STEP * ky_hi + TILE) as f32), true);
    g.begin(rc);
    g.set_pass(Pass::Static);
    ui.paint_page_content(f);
    g.set_pass(Pass::All);
    g.end();
    let Some(pic) = rec.finish_recording_as_picture(None) else { return false };
    let b0 = b.b0;
    let bs = &mut b.surf;
    with_tile(bs, |tile, bs| {
        for &(ky, kx) in &todo {
            let (ox, oy) = (TILE_STEP * kx, TILE_STEP * ky);
            let (ax, bx) = tile_span(kx);
            let (ay, by) = tile_span(ky);
            let own = sk::IRect::new(ax, ay.max(b0), bx.min(w), by.min(b0 + hb));
            if own.is_empty() {
                continue;
            }
            let c = tile.canvas();
            c.clear(sk::Color::TRANSPARENT);
            c.save();
            // like cc: the whole tile texture (border included) is rastered, only the owned part is used
            c.clip_irect(sk::IRect::from_wh(TILE, TILE), sk::ClipOp::Intersect);
            c.translate((-ox as f32, -oy as f32));
            c.draw_picture(&pic, None, None);
            c.restore();
            let img = tile.image_snapshot();
            let mut p = sk::Paint::default();
            p.set_blend_mode(sk::BlendMode::Src);
            let src = sk::Rect::from(own.with_offset((-ox, -oy)));
            bs.canvas().draw_image_rect(img, Some((&src, sk::canvas::SrcRectConstraint::Strict)), sk::Rect::from(own.with_offset((0, -b0))), &p);
        }
    });
    for ky in k0..=k1 {
        b.rows.insert(ky);
    }
    BAND_TILES.with(|n| n.set(n.get() + todo.len()));
    true
}

thread_local! {
    /// BU_PROF: the overlays' snapshot / key / paint times of this frame
    static OV_PROF: std::cell::Cell<(f64, f64, f64)> = const { std::cell::Cell::new((0.0, 0.0, 0.0)) };
    /// BU_PROF: tiles the band rastered since the last profile line
    static BAND_TILES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The flyout's outer box-shadow: 0 0 0 .5px rgba(0,0,0,.6), 0 30px 80px rgba(0,0,0,.45), 0 8px 24px rgba(0,0,0,.28).
/// The window's outer shadow: `#sw{box-shadow:...}`; light (Order 033): `#sw.light{box-shadow:0 0 0 .5px rgba(0,0,0,.2),0 30px 80px
/// rgba(0,0,0,.28),0 8px 24px rgba(0,0,0,.16)}`.
fn shadows() -> [Shadow; 3] {
    if ui::is_light() {
        return [
            Shadow { dx: 0.0, dy: 0.0, blur: 0.0, spread: 0.5, c: Rgba(0.0, 0.0, 0.0, 0.2) },
            Shadow { dx: 0.0, dy: 30.0, blur: 80.0, spread: 0.0, c: Rgba(0.0, 0.0, 0.0, 0.28) },
            Shadow { dx: 0.0, dy: 8.0, blur: 24.0, spread: 0.0, c: Rgba(0.0, 0.0, 0.0, 0.16) },
        ];
    }
    [
    Shadow { dx: 0.0, dy: 0.0, blur: 0.0, spread: 0.5, c: Rgba(0.0, 0.0, 0.0, 0.6) },
    Shadow { dx: 0.0, dy: 30.0, blur: 80.0, spread: 0.0, c: Rgba(0.0, 0.0, 0.0, 0.45) },
    Shadow { dx: 0.0, dy: 8.0, blur: 24.0, spread: 0.0, c: Rgba(0.0, 0.0, 0.0, 0.28) },
    ]
}

/// A CPU surface (the shadow window's picture, test pictures, the CPU-only glass test modes).
fn layer(w: i32, h: i32) -> Result<sk::Surface> {
    new_surface(w, h).ok_or_else(|| Error::from(E_OUTOFMEMORY))
}

/// A surface on the GPU when there is one (Order 051), else on the CPU.
fn surf_on(gpu: Option<&Rc<Gpu>>, w: i32, h: i32) -> Result<sk::Surface> {
    match gpu {
        Some(g) => g.surface(w, h).ok_or_else(|| Error::new(E_OUTOFMEMORY, "no GPU surface")),
        None => layer(w, h),
    }
}

/// The window's edge (Order 013): Skia's anti-aliased coverage of the 14 px rounded shape - the same clip the drawing's
/// backdrop is cut with - is the glass's mask. No window region: the menu window shows its whole rectangle exactly as the
/// drawing's layer has it there (the outer shadow round the corners, and the page's scrollbar, which reaches 1 px past the
/// curve at the bottom-right corner, measured).
fn paint_mask(g: &Gfx, s: &mut sk::Surface) {
    s.canvas().clear(sk::Color::TRANSPARENT);
    g.begin(s.canvas());
    g.clip_coverage(0.0, 0.0, WIN_W, WIN_H, RADIUS);
    g.end();
}

/// The edge mask as CPU pixels (the CPU path's mask chain; test pictures).
fn mask_raster(g: &Gfx, w: i32, h: i32) -> Result<crate::png::Pixels> {
    let mut m = layer(w, h)?;
    paint_mask(g, &mut m);
    Ok(crate::png::from_surface(&mut m))
}

/// The menu's two swap chains (its frame and its edge mask, painted once): on the GPU when `gpu` is given and they can be
/// made there, else on the CPU path (logged). Returns the GPU actually used.
fn make_swaps(gpu: Option<Rc<Gpu>>, g: &Gfx, w: i32, h: i32) -> Result<(Option<Rc<Gpu>>, Swap, Swap)> {
    if let Some(gp) = gpu {
        let made = (|| -> Result<(Swap, Swap)> {
            let chain = GpuChain::new(&gp, w as u32, h as u32)?;
            let mut mask = GpuChain::new(&gp, w as u32, h as u32)?;
            let mut ms = mask.back();
            paint_mask(g, &mut ms);
            mask.present()?;
            Ok((Swap::Gpu(chain), Swap::Gpu(mask)))
        })();
        match made {
            Ok((c, m)) => return Ok((Some(gp), c, m)),
            Err(e) => {
                if gp.lost() || gpu::is_lost_error(&e) {
                    gpu::mark_lost();
                }
                timing::note(&format!("gpu path=cpu reason=swap chain {:08x} {}", e.code().0, e.message()));
            }
        }
    }
    let chain = Chain::new(w as u32, h as u32)?;
    let mask = Chain::new_on(chain.d3d.clone(), w as u32, h as u32)?;
    let px = mask_raster(g, w, h)?;
    mask.present(&px.data, (w * 4) as u32)?;
    Ok((None, Swap::Cpu(chain), Swap::Cpu(mask)))
}

fn sys_highlight() -> Rgba {
    unsafe {
        let c = GetSysColor(COLOR_HIGHLIGHT);
        Rgba::rgb((c & 0xff) as u8, ((c >> 8) & 0xff) as u8, ((c >> 16) & 0xff) as u8)
    }
}
