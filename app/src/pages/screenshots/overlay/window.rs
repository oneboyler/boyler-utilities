//! The capture overlay on the real screen: one topmost layered window per monitor (physical pixels, each painted at its own
//! monitor's scale), the capture toast and its flying thumbnail, the whole-screen flash. They live on the app's UI thread
//! (its message loop dispatches to `wndproc`). The overlay windows are excluded from screen capture
//! (`WDA_EXCLUDEFROMCAPTURE`), so a Live snap never sees them.
//!
//! Order 048 (a user's whole desktop froze): the UI thread never waits on Windows here - grabbing the screen and Copy / Save
//! run on a worker (`worker.rs`; the overlay opens when the key's picture lands), and while the overlay is open a watchdog
//! (`watchdog.rs`) takes its windows off the screen if this thread ever stops answering for 450 ms.
//!
//! Nothing here runs in a test copy: `start` refuses when the app runs in test mode (tests drive `model` / `capture` against
//! bu-screenshot's fake and render `view` off-screen).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use skia_safe as sk;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use bu_screenshot::real::RealOs;
use bu_screenshot::{Monitor, Screenshots};

use super::capture::{Capture, Done, Step};
use super::watchdog;
use super::worker::{self, Job, Res, Tag};
use super::model::{Corner, Finish, Keep, Mode, Out, Pointer, Tool, COLORS};
use super::toast;
use super::view::{self, *};
use crate::gfx::Gfx;
use crate::icons::Icons;
use crate::ui::cx::{Cx, State};
use crate::ui::el::{idx, El, Key};
use crate::ui::lay::Laid;

/// What the app hands the overlay (the app wires these; Order 019 leaves the wiring to the keys manager / frame - see the
/// report).
pub struct Hooks {
    /// the open menu's window (screen px): the toast sits left of it
    pub menu_rect: Box<dyn Fn() -> Option<RECT>>,
    /// where the thumbnail flies: the gallery's first tile if the Screenshots page is showing (screen px) - else None
    pub gallery_target: Box<dyn Fn() -> Option<RECT>>,
    /// the tray icon (screen px): the thumbnail flies there when the gallery is not showing
    pub tray_rect: Box<dyn Fn() -> Option<RECT>>,
    /// a shot joined the gallery (the page puts it in front)
    pub on_shot: Box<dyn Fn(&bu_screenshot::Shot)>,
    /// the thumbnail landed in the tray: the icon bobs (`toast::tray_bob`)
    pub on_tray_landed: Box<dyn Fn()>,
    /// the capture failed (no picture): what went wrong, in plain words
    pub on_error: Box<dyn Fn(&str)>,
}

impl Default for Hooks {
    fn default() -> Self {
        Hooks {
            menu_rect: Box::new(|| None),
            gallery_target: Box::new(|| None),
            tray_rect: Box::new(|| None),
            on_shot: Box::new(|_| {}),
            on_tray_landed: Box::new(|| {}),
            on_error: Box::new(|_| {}),
        }
    }
}

const CLASS: PCWSTR = w!("BoylerUtilities.Capture");
const TIMER: usize = 0x5C01;
const WDA_EXCLUDEFROMCAPTURE: WINDOW_DISPLAY_AFFINITY = WINDOW_DISPLAY_AFFINITY(0x11);

/// One layered window and its pixels (a DIB that Skia draws into directly). Also the lightbox's host (lbhost.rs).
/// Order 051: or a window whose pixels Skia draws on the GPU (`gpu`), straight into a composition swap chain shown by
/// DirectComposition - the capture overlay's monitor windows.
pub(crate) struct Layer {
    hwnd: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    gpu: Option<GpuOut>,
}

/// Order 051: a monitor window's GPU output - the swap chain (paced by its frame-latency waitable: the compositor's clock of
/// that monitor) and the DirectComposition visual showing it (its opacity = the fade).
pub(crate) struct GpuOut {
    chain: crate::gpu::GpuChain,
    dev: IDCompositionDesktopDevice,
    _target: IDCompositionTarget,
    visual: IDCompositionVisual2,
    alpha: f32,
    /// the waitable fired and no frame used it yet (the compositor has room for one)
    slot: bool,
    /// something changed since the last frame (painted at the next slot)
    want: bool,
    /// when it was last painted (ms): a window whose waitable stays silent for 100 ms is painted anyway (`overdue`)
    painted_at: f64,
}

impl Layer {
    fn new(x: i32, y: i32, w: i32, h: i32, click_through: bool) -> Result<Layer> {
        register();
        Self::with_class(CLASS, x, y, w, h, click_through)
    }

    /// A layered window of an already registered window class (its own window procedure).
    pub(crate) fn with_class(class: PCWSTR, x: i32, y: i32, w: i32, h: i32, click_through: bool) -> Result<Layer> {
        unsafe {
            let ex = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | if click_through { WS_EX_TRANSPARENT | WS_EX_NOACTIVATE } else { WINDOW_EX_STYLE(0) };
            let inst = GetModuleHandleW(None)?;
            let hwnd = CreateWindowExW(ex, class, w!("Boyler Utilities capture"), WS_POPUP, x, y, w, h, None, None, Some(inst.into()), None)?;
            let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen));
            ReleaseDC(None, screen);
            let mut l = Layer { hwnd, x, y, w: 0, h: 0, dc, bmp: HBITMAP::default(), old: HGDIOBJ::default(), bits: std::ptr::null_mut(), gpu: None };
            l.resize(w, h)?;
            Ok(l)
        }
    }

    /// Order 051: a monitor window drawn on the GPU - no layered window, no DIB: a composition swap chain the size of the
    /// window, shown by a DirectComposition visual (a normal window takes the clicks on every pixel, transparent or not).
    fn new_gpu(gpu: &Rc<crate::gpu::Gpu>, x: i32, y: i32, w: i32, h: i32) -> Result<Layer> {
        register();
        unsafe {
            let ex = WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW;
            let inst = GetModuleHandleW(None)?;
            let hwnd = CreateWindowExW(ex, CLASS, w!("Boyler Utilities capture"), WS_POPUP, x, y, w, h, None, None, Some(inst.into()), None)?;
            let made = (|| -> Result<GpuOut> {
                let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
                let chain = crate::gpu::GpuChain::new(gpu, w.max(1) as u32, h.max(1) as u32)?;
                let dev: IDCompositionDesktopDevice = DCompositionCreateDevice2(None)?;
                let target = dev.CreateTargetForHwnd(hwnd, true)?;
                let visual = dev.CreateVisual()?;
                visual.SetContent(&chain.swap)?;
                target.SetRoot(&visual)?;
                dev.Commit()?;
                Ok(GpuOut { chain, dev, _target: target, visual, alpha: 1.0, slot: true, want: true, painted_at: crate::timing::now() })
            })();
            match made {
                Ok(g) => Ok(Layer { hwnd, x, y, w: w.max(1), h: h.max(1), dc: HDC::default(), bmp: HBITMAP::default(), old: HGDIOBJ::default(), bits: std::ptr::null_mut(), gpu: Some(g) }),
                Err(e) => {
                    let _ = DestroyWindow(hwnd);
                    Err(e)
                }
            }
        }
    }

    /// Drawn on the GPU?
    pub(crate) fn is_gpu(&self) -> bool {
        self.gpu.is_some()
    }

    fn resize(&mut self, w: i32, h: i32) -> Result<()> {
        if self.gpu.is_some() || (w == self.w && h == self.h && !self.bits.is_null()) {
            return Ok(());
        }
        unsafe {
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w.max(1),
                    biHeight: -h.max(1),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let hb = CreateDIBSection(Some(self.dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            let old = SelectObject(self.dc, hb.into());
            if !self.bmp.is_invalid() {
                let _ = DeleteObject(self.bmp.into());
            } else {
                self.old = old;
            }
            self.bmp = hb;
            self.bits = bits as *mut u8;
            self.w = w.max(1);
            self.h = h.max(1);
        }
        Ok(())
    }

    pub(crate) fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Skia draws straight into the window's pixels.
    pub(crate) fn surface(&mut self) -> Option<sk::Surface> {
        if let Some(g) = &self.gpu {
            return Some(g.chain.back());
        }
        let info = sk::ImageInfo::new((self.w, self.h), sk::ColorType::BGRA8888, sk::AlphaType::Premul, None);
        let len = (self.w * self.h * 4) as usize;
        let px = unsafe { std::slice::from_raw_parts_mut(self.bits, len) };
        sk::surfaces::wrap_pixels(&info, px, (self.w * 4) as usize, Some(&crate::gfx::surface_props())).map(|s| unsafe { s.release() })
    }

    pub(crate) fn present(&mut self, alpha: f32) {
        if let Some(g) = &mut self.gpu {
            let a = alpha.clamp(0.0, 1.0);
            if a != g.alpha {
                g.alpha = a;
                unsafe {
                    if let Ok(v3) = g.visual.cast::<IDCompositionVisual3>() {
                        let _ = v3.SetOpacity2(a);
                    }
                    let _ = g.dev.Commit();
                }
            }
            if let Err(e) = g.chain.present(None) {
                crate::timing::note(&format!("overlay present failed {:08x}", e.code().0));
                if crate::gpu::is_lost_error(&e) || g.chain.gpu.lost() {
                    LOST.with(|l| l.set(true));
                }
            }
            return;
        }
        unsafe {
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8, AlphaFormat: AC_SRC_ALPHA as u8 };
            let pt = POINT { x: self.x, y: self.y };
            let size = SIZE { cx: self.w, cy: self.h };
            let src = POINT { x: 0, y: 0 };
            let _ = UpdateLayeredWindow(self.hwnd, None, Some(&pt), Some(&size), Some(self.dc), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
        }
    }
}

impl Drop for Layer {
    fn drop(&mut self) {
        // (the GPU output first: the GPU finishes with the swap chain before the window goes)
        self.gpu = None;
        unsafe {
            if !self.old.is_invalid() {
                SelectObject(self.dc, self.old);
            }
            if !self.bmp.is_invalid() {
                let _ = DeleteObject(self.bmp.into());
            }
            if !self.dc.is_invalid() {
                let _ = DeleteDC(self.dc);
            }
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// One monitor's overlay window.
struct MonWin {
    mi: usize,
    layer: Layer,
    g: Gfx,
    st: State,
    front: Option<Laid>,
    /// the frozen picture of this monitor (device px)
    bg: Option<sk::Image>,
    /// Order 045: the shared tip bubble (the drawing's `title` hover names: Live, Snap, ×, Copy / Save, the emojis), run like the
    /// menu's (`ui.rs` `update_tips` / `draw_popup`)
    tips: crate::ui::pieces::tip::Tips,
}

struct Session {
    cap: Capture<RealOs>,
    wins: Vec<MonWin>,
    icons: Icons,
    hooks: Rc<Hooks>,
    /// the key that went down on a bar / button (a click = down and up on the same key)
    pressed: Option<Key>,
    /// closing after Copy / Save / a click (ms): the overlay fades out
    closing: Option<(f64, Option<Done>)>,
    /// Order 051: the GPU the monitor windows are drawn on (None = the CPU path: layered windows)
    gpu: Option<Rc<crate::gpu::Gpu>>,
    /// Order 048: this capture's number (a worker's result for another one is dropped), its engine's folder, the jobs sent
    id: u64,
    data_dir: PathBuf,
    seq: u64,
    /// a new picture is on its way (the job's number; a Snap / Live off): only the newest one is used
    grab_pending: Option<u64>,
    /// Copy / Save is running (the job's number); the overlay fades meanwhile
    finish_pending: Option<u64>,
    /// Live was on when that picture was asked for: the overlay shows the screen through until it lands
    show_live: bool,
}

impl Session {
    fn next_tag(&mut self) -> Tag {
        self.seq += 1;
        Tag { session: self.id, seq: self.seq }
    }
}

/// The Screenshot key was pressed and the screen is being grabbed (Order 048: on a worker - the overlay opens when it lands).
struct Starting {
    id: u64,
    hooks: Rc<Hooks>,
    pointer: (f32, f32),
    data_dir: PathBuf,
}

/// A Copy / Save still running after its overlay closed: the toast comes when it is done.
struct LateShot {
    tag: Tag,
    hooks: Rc<Hooks>,
    home: Monitor,
}

/// A finished shot on screen: the toast, then its thumbnail flying off, then (for a click) the flash that came before it.
struct ToastWin {
    layer: Layer,
    fly: Option<Layer>,
    g: Gfx,
    scale: f32,
    img: Rc<sk::Image>,
    done: Done,
    at: f64,
    /// the toast's box on screen (px) and the thumbnail's box inside it (px)
    thumb_from: (f32, f32, f32, f32),
    target: Option<(RECT, bool)>,
    hooks: Rc<Hooks>,
    landed: bool,
}

struct FlashWin {
    layer: Layer,
    at: f64,
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    static TOAST: RefCell<Option<ToastWin>> = const { RefCell::new(None) };
    static FLASH: RefCell<Vec<FlashWin>> = const { RefCell::new(Vec::new()) };
    static KEEP: RefCell<Option<Keep>> = const { RefCell::new(None) };
    static REGISTERED: RefCell<bool> = const { RefCell::new(false) };
    /// Order 051: a GPU present failed with a lost device - the overlay goes on on the CPU path
    static LOST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static STARTING: RefCell<Option<Starting>> = const { RefCell::new(None) };
    static LATE: RefCell<Vec<LateShot>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: RefCell<u64> = const { RefCell::new(0) };
    /// a hidden message-only window of the overlay's class: the workers' results come to it (WM_JOB_DONE)
    static MSG_WIN: RefCell<HWND> = RefCell::new(HWND::default());
}

/// The overlay's message window (made once, never on screen).
fn msg_window() -> HWND {
    register();
    MSG_WIN.with(|m| {
        let mut h = m.borrow_mut();
        if h.is_invalid() {
            unsafe {
                if let Ok(inst) = GetModuleHandleW(None) {
                    *h = CreateWindowExW(WINDOW_EX_STYLE(0), CLASS, w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(inst.into()), None).unwrap_or_default();
                }
            }
        }
        *h
    })
}

fn register() {
    REGISTERED.with(|r| {
        if *r.borrow() {
            return;
        }
        unsafe {
            if let Ok(inst) = GetModuleHandleW(None) {
                RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: CLASS, ..Default::default() });
            }
        }
        *r.borrow_mut() = true;
    });
}

fn now() -> f64 {
    crate::timing::now()
}

/// Is the overlay open (or opening: the screen is being grabbed)?
pub fn is_open() -> bool {
    SESSION.with(|s| s.try_borrow().map(|s| s.is_some()).unwrap_or(true)) || STARTING.with(|s| s.try_borrow().map(|s| s.is_some()).unwrap_or(true))
}

/// The Screenshot key's action: freeze every monitor and open the overlay on them. Refused in a test copy (tests never
/// capture the screen) and while it is already open. Order 048: the grab runs on a worker (it can wait 1-2 s for Windows);
/// the overlay opens when the picture lands ([`opened`]); a failure is reported through `on_error`.
pub fn start(hooks: Hooks) -> std::result::Result<(), String> {
    if let Some(why) = refused(crate::testmode::on()) {
        return Err(why.into());
    }
    if is_open() {
        return Ok(());
    }
    let Some(dir) = bu_screenshot::real::default_data_dir() else { return Err("no LOCALAPPDATA folder".into()) };
    let notify = msg_window();
    if notify.is_invalid() {
        return Err("the capture overlay's window could not be made".into());
    }
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    let id = NEXT_ID.with(|n| {
        *n.borrow_mut() += 1;
        *n.borrow()
    });
    let st = Starting { id, hooks: Rc::new(hooks), pointer: (p.x as f32, p.y as f32), data_dir: dir.clone() };
    STARTING.with(|c| *c.borrow_mut() = Some(st));
    worker::spawn(Tag { session: id, seq: 0 }, dir, Job::Grab, notify);
    Ok(())
}

/// The key's grab landed: the overlay opens on it (or the failure is reported).
fn opened(tag: Tag, r: bu_screenshot::Result<bu_screenshot::Frozen>) {
    let st = STARTING.with(|c| {
        let mut b = c.borrow_mut();
        if b.as_ref().map(|s| s.id) == Some(tag.session) {
            b.take()
        } else {
            None
        }
    });
    let Some(st) = st else { return };
    let hooks = st.hooks;
    let fail = |msg: String| (hooks.on_error)(&msg);
    let frozen = match r {
        Ok(f) => f,
        Err(e) => return fail(format!("Screenshot failed: {e}")),
    };
    let svc = Screenshots::new(RealOs::new(), st.data_dir.clone());
    let keep = KEEP.with(|k| k.borrow().clone());
    let cap = Capture::with_frozen(svc, frozen, st.pointer, now(), keep);
    let p = POINT { x: st.pointer.0 as i32, y: st.pointer.1 as i32 };
    if let Err(e) = open_session(cap, hooks.clone(), p, false, st.id, st.data_dir) {
        fail(format!("Screenshot failed: {e}"));
    }
}

/// Order 051, test copies only: the overlay over a made-up picture on one made-up monitor (`gputest`) - never a grab of the
/// screen; the engine runs on the read-only OS layer (Copy / Save / Snap are refused) and the windows never take the focus.
pub fn start_test(x: i32, y: i32, w: u32, h: u32) -> std::result::Result<(), String> {
    if !crate::testmode::on() {
        return Err("test copies only".into());
    }
    if is_open() {
        return Ok(());
    }
    let frozen = super::gputest::frozen(x, y, w, h);
    let dir = std::env::temp_dir().join("BoylerUtilities-test").join("overlay");
    let svc = Screenshots::new(RealOs::read_only(), dir.clone());
    let p = POINT { x: x + w as i32 / 2, y: y + h as i32 / 2 };
    let model = super::model::Model::new(frozen.monitors.clone(), (p.x as f32, p.y as f32), now(), None);
    let id = NEXT_ID.with(|n| {
        *n.borrow_mut() += 1;
        *n.borrow()
    });
    open_session(Capture { svc, frozen, model }, Rc::new(Hooks::default()), p, true, id, dir)
}

/// Test copies: the first overlay window (the drag's target) and how it is drawn.
pub fn test_window() -> Option<(HWND, bool)> {
    SESSION.with(|c| c.try_borrow().ok().and_then(|b| b.as_ref().and_then(|s| s.wins.first().map(|w| (w.layer.hwnd, w.layer.is_gpu())))))
}

/// Test copies: close without a shot.
pub fn test_close() {
    close(None);
}

/// Test copies (`ovclick`, Order 054): a click on a button of the first window - the `i`-th of its shown buttons (× / Copy /
/// Save left out: they end the overlay), or the one named `x` / `copy` / `save` - made of mouse messages posted to the window
/// (the pointer slides onto it 1 px at a time first, as a hand does). The log says which button was aimed at; the window
/// logs what the press reached (`ovclicked` / `ovpress`).
pub fn test_click(which: &str) {
    if !crate::testmode::on() {
        return;
    }
    let aim = SESSION.with(|c| {
        let b = c.try_borrow().ok()?;
        let w = b.as_ref()?.wins.first()?;
        let all = view::buttons(w.front.as_ref()?);
        let ends = [K_X, K_COPY, K_SAVE];
        let named = match which {
            "x" => Some(K_X),
            "copy" => Some(K_COPY),
            "save" => Some(K_SAVE),
            _ => None,
        };
        let (k, c) = match named {
            Some(n) => *all.iter().find(|(k, _)| *k == n)?,
            None => {
                let list: Vec<_> = all.iter().filter(|(k, _)| !ends.contains(k)).copied().collect();
                *list.get(which.parse::<usize>().ok()? % list.len().max(1))?
            }
        };
        let sc = w.g.scale;
        Some((w.layer.hwnd, k, ((c.0 * sc).round() as i32, (c.1 * sc).round() as i32), all.len()))
    });
    match aim {
        Some((hwnd, k, p, n)) => {
            crate::timing::note(&format!("ovclick {which} key={k:x} at {},{} of {n} buttons", p.0, p.1));
            super::gputest::click(hwnd, p);
        }
        None => crate::timing::note(&format!("ovclick {which} no button")),
    }
}

/// Test copies (`ovlose`): the overlay's GPU device removed, as a driver crash would.
pub fn test_lose_gpu() {
    SESSION.with(|c| {
        if let Some(g) = c.borrow().as_ref().and_then(|s| s.gpu.clone()) {
            g.remove_for_test();
        }
    });
    paint_all();
}

/// The overlay's windows over the captured picture (`test`: never takes the focus).
/// Order 048: `id` / `data_dir` = the capture's number and its engine's folder (its workers' results find it by them).
fn open_session(cap: Capture<RealOs>, hooks: Rc<Hooks>, p: POINT, test: bool, id: u64, data_dir: PathBuf) -> std::result::Result<(), String> {
    let mut wins = Vec::new();
    // Order 051: on the GPU (the adapter driving the monitor under the pointer), else today's layered windows
    let gpu = crate::gpu::get(Some(unsafe { MonitorFromPoint(p, MONITOR_DEFAULTTOPRIMARY) }));
    for (mi, m) in cap.model.monitors.iter().enumerate() {
        let layer = match gpu.as_ref().map(|g| Layer::new_gpu(g, m.rect.x, m.rect.y, m.rect.w as i32, m.rect.h as i32)) {
            Some(Ok(l)) => l,
            r => {
                if let Some(Err(e)) = r {
                    crate::timing::note(&format!("overlay gpu window failed {:08x} {} - layered window", e.code().0, e.message()));
                }
                Layer::new(m.rect.x, m.rect.y, m.rect.w as i32, m.rect.h as i32, false).map_err(|e| e.to_string())?
            }
        };
        wins.push(MonWin { mi, layer, g: Gfx::new(m.dpi as f32 / 96.0), st: State::default(), front: None, bg: None, tips: Default::default() });
    }
    let mut s = Session {
        cap,
        wins,
        icons: Icons::new(),
        hooks,
        pressed: None,
        closing: None,
        gpu,
        id,
        data_dir,
        seq: 0,
        grab_pending: None,
        finish_pending: None,
        show_live: false,
    };
    // Order 045: Save's hover name says where it saves (`S.shotDir`, as the page shows the folder)
    if let Ok(d) = s.cap.svc.save_dir() {
        s.cap.model.save_dir = super::super::gallery::short_dir(&d, super::super::gallery::profile_dir(false).as_deref());
    }
    slice_frozen(&mut s);
    let home = s.cap.model.home;
    SESSION.with(|c| *c.borrow_mut() = Some(s));
    paint_all();
    let hwnds = SESSION.with(|c| {
        let b = c.borrow();
        let Some(s) = b.as_ref() else { return Vec::new() };
        unsafe {
            for w in &s.wins {
                let _ = ShowWindow(w.layer.hwnd, SW_SHOWNA);
            }
            if let Some(w) = s.wins.iter().find(|w| w.mi == home).filter(|_| !test) {
                let _ = SetForegroundWindow(w.layer.hwnd);
                let _ = SetFocus(Some(w.layer.hwnd));
            }
        }
        s.wins.iter().map(|w| w.layer.hwnd).collect()
    });
    // Order 048: from now on the windows are watched - if this thread ever stops answering, they leave the screen
    watchdog::open(&hwnds);
    tick_timer(true);
    Ok(())
}

/// Why the overlay may not open: a test copy never captures the screen.
pub fn refused(test_copy: bool) -> Option<&'static str> {
    test_copy.then_some("the capture overlay never opens in a test copy")
}

/// Each monitor's part of the frozen picture as a Skia image (exact device pixels).
fn slice_frozen(s: &mut Session) {
    let f = &s.cap.frozen;
    for w in &mut s.wins {
        let m: &Monitor = &s.cap.model.monitors[w.mi];
        w.bg = f.crop(&m.rect).ok().and_then(|img| super::compose::to_sk(&img));
        // Order 051: a GPU window gets the frozen picture as a texture (uploaded once, not every frame)
        if let (Some(g), true) = (&s.gpu, w.layer.is_gpu()) {
            w.bg = w.bg.take().map(|img| g.texture(&img).unwrap_or(img));
        }
    }
}

fn tick_timer(on: bool) {
    SESSION.with(|c| {
        if let Ok(b) = c.try_borrow() {
            if let Some(s) = b.as_ref() {
                if let Some(w) = s.wins.first() {
                    unsafe {
                        if on {
                            let _ = SetTimer(Some(w.layer.hwnd), TIMER, 15, None);
                        }
                    }
                }
            }
        }
    });
}

/// Paint every monitor's window; true while something moves. Order 051: a window drawn on the GPU is painted only when its
/// swap chain has room (its monitor's next refresh) - else it is marked and painted at that moment (`on_waitable`).
fn paint_all() -> bool {
    paint_wins(None)
}

/// `only` = the one GPU window whose waitable just fired (None = every window: an input or the timer).
fn paint_wins(only: Option<usize>) -> bool {
    let busy = SESSION.with(|c| {
        let Ok(mut b) = c.try_borrow_mut() else { return false };
        let Some(s) = b.as_mut() else { return false };
        let t = now();
        let mut busy = false;
        let fade = match &s.closing {
            Some((at, _)) => 1.0 - (((t - at) - 110.0) / 150.0).clamp(0.0, 1.0) as f32,
            None => 1.0,
        };
        // (Order 048: Live off / a Snap: the screen shows through until the new picture is in from the worker)
        let live = s.cap.model.live || s.show_live;
        let closing = s.closing.is_some();
        for (i, w) in s.wins.iter_mut().enumerate() {
            if only.is_some_and(|o| o != i) {
                continue;
            }
            if let Some(g) = &mut w.layer.gpu {
                // a removed device never fires its waitable again: the fallback must not wait for a failed present
                if g.chain.gpu.lost() {
                    LOST.with(|l| l.set(true));
                    continue;
                }
                if only.is_none() {
                    g.want = true;
                }
                if !g.want || !g.slot {
                    busy |= g.want;
                    continue;
                }
                g.slot = false;
            }
            let wbusy = paint_win(w, &s.icons, &s.cap.model, t, fade, live);
            if let Some(g) = &mut w.layer.gpu {
                g.want = wbusy || closing;
                g.painted_at = now();
            }
            busy |= wbusy;
        }
        busy || closing
    });
    if LOST.with(|l| l.replace(false)) {
        cpu_fallback();
    }
    busy
}

/// Order 051: the GPU device was lost while the overlay was open - every monitor window again as today's layered window,
/// painted before it is shown and before the GPU window goes (nothing blank, not even for a frame), the GPU released. If a
/// layered window can't be made the overlay closes (never a frozen window).
fn cpu_fallback() {
    thread_local! {
        static FALLING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if FALLING.with(|f| f.replace(true)) {
        return;
    }
    crate::gpu::mark_lost();
    let focus = unsafe { GetFocus() };
    let (mut old, mut shown, mut focus_to, mut failed) = (Vec::new(), Vec::new(), None, false);
    SESSION.with(|c| {
        let Ok(mut b) = c.try_borrow_mut() else { return };
        let Some(s) = b.as_mut() else { return };
        for w in &mut s.wins {
            if !w.layer.is_gpu() {
                continue;
            }
            match Layer::new(w.layer.x, w.layer.y, w.layer.w, w.layer.h, false) {
                Ok(l) => {
                    if focus == w.layer.hwnd {
                        focus_to = Some(l.hwnd);
                    }
                    shown.push(l.hwnd);
                    old.push(std::mem::replace(&mut w.layer, l));
                }
                Err(_) => failed = true,
            }
        }
        // the frozen pictures back on the CPU (the GPU textures are gone with the device)
        s.gpu = None;
        slice_frozen(s);
    });
    if failed {
        drop(old);
        close(None);
    } else {
        paint_all();
        unsafe {
            for h in &shown {
                let _ = ShowWindow(*h, SW_SHOWNA);
            }
            if let Some(h) = focus_to {
                let _ = SetForegroundWindow(h);
                let _ = SetFocus(Some(h));
            }
        }
        drop(old);
        tick_timer(true);
        // Order 048: the watchdog watches the new windows
        let hwnds: Vec<HWND> = SESSION.with(|c| c.borrow().as_ref().map(|s| s.wins.iter().map(|w| w.layer.hwnd).collect()).unwrap_or_default());
        watchdog::open(&hwnds);
    }
    FALLING.with(|f| f.set(false));
}

/// Order 051: GPU windows waiting for a frame whose waitable stayed silent for 100 ms (a hidden monitor, or other handles
/// firing all the time) are painted anyway; a lost device is noticed here too.
pub fn overdue() {
    let t = now();
    let idx: Vec<usize> = SESSION.with(|c| {
        let Ok(mut b) = c.try_borrow_mut() else { return Vec::new() };
        let Some(s) = b.as_mut() else { return Vec::new() };
        let mut v = Vec::new();
        for (i, w) in s.wins.iter_mut().enumerate() {
            if let Some(g) = &mut w.layer.gpu {
                if g.want && !g.slot && (t - g.painted_at > 100.0 || g.chain.gpu.lost()) {
                    g.slot = true;
                    v.push(i);
                }
            }
        }
        v
    });
    for i in idx {
        paint_wins(Some(i));
    }
}

/// Order 051: the swap chains' waitables of the GPU windows that wait for a frame (the main loop waits on them).
pub fn frame_waitables() -> Vec<HANDLE> {
    SESSION.with(|c| {
        let Ok(b) = c.try_borrow() else { return Vec::new() };
        let Some(s) = b.as_ref() else { return Vec::new() };
        s.wins.iter().filter_map(|w| w.layer.gpu.as_ref().filter(|g| g.want && !g.slot).map(|g| g.chain.waitable)).collect()
    })
}

/// Order 051: a GPU window's waitable fired (`h`; None = none fired for 100 ms - every waiting window is painted anyway, so a
/// hidden monitor never freezes the others): it is painted now if it waits for a frame.
pub fn on_waitable(h: Option<HANDLE>) {
    let idx: Vec<usize> = SESSION.with(|c| {
        let Ok(mut b) = c.try_borrow_mut() else { return Vec::new() };
        let Some(s) = b.as_mut() else { return Vec::new() };
        let mut v = Vec::new();
        for (i, w) in s.wins.iter_mut().enumerate() {
            if let Some(g) = &mut w.layer.gpu {
                if h.is_none_or(|h| h == g.chain.waitable) {
                    g.slot = true;
                    v.push(i);
                }
            }
        }
        v
    });
    for i in idx {
        paint_wins(Some(i));
    }
}

/// One monitor window painted and presented; true while something on it moves.
fn paint_win(w: &mut MonWin, icons: &Icons, model: &super::model::Model, t: f64, fade: f32, live: bool) -> bool {
    let c0 = if PROF.with(|p| *p) { crate::timing::thread_cpu_ms() } else { -1.0 };
    let busy = paint_win_inner(w, icons, model, t, fade, live);
    if c0 >= 0.0 {
        crate::timing::ov_frame(t, crate::timing::thread_cpu_ms() - c0, w.mi, w.layer.is_gpu());
    }
    busy
}

thread_local! {
    /// BU_PROF (test copies): every overlay frame's UI-thread CPU goes to the timing log
    static PROF: bool = crate::testmode::env("BU_PROF").is_some();
}

fn paint_win_inner(w: &mut MonWin, icons: &Icons, model: &super::model::Model, t: f64, fade: f32, live: bool) -> bool {
    let mut busy = false;
    {
        {
            let mut cx = Cx::new(t, false, &w.g, &mut w.st);
            let sc = view::scene(model, &mut cx, w.mi, t);
            busy |= sc.busy || w.st.busy;
            w.st.busy = false;
            w.st.sweep();
            let Some(mut surf) = w.layer.surface() else { return busy };
            let bg = if live { None } else { w.bg.as_ref() };
            let (_, front) = view::paint(&w.g, icons, &mut surf, &sc, bg);
            // Order 045: the shared tip on top (the menu's `update_tips` + `draw_popup`): the hovered chain, 500 ms for a title
            busy |= w.tips.update(&w.g, &[(&front, (0.0, 0.0))], &w.st.hover, t, sc.w);
            if let Some(b) = w.tips.el(t) {
                let base = surf.image_snapshot();
                w.g.begin(surf.canvas());
                Laid::new(&w.g, El::block().w(sc.w).h(sc.h).no_hit().child(b), sc.w, Some(sc.h)).paint(&w.g, icons, 0.0, 0.0, Some(&base));
                w.g.end();
            }
            if live {
                // Live: the real screen shows through; a 1/255 film keeps the clicks in the overlay (a layered window lets
                // fully transparent pixels click through)
                let cv = surf.canvas();
                let mut p = sk::Paint::default();
                p.set_color(sk::Color::from_argb(1, 0, 0, 0));
                p.set_blend_mode(sk::BlendMode::DstOver);
                cv.draw_paint(&p);
            }
            w.front = Some(front);
            w.layer.present(fade);
        }
    }
    busy
}

/// A window's pointer position -> desktop px and the window's DIPs.
fn points(s: &Session, hwnd: HWND, l: LPARAM) -> Option<(usize, (f32, f32), (f32, f32))> {
    let w = s.wins.iter().position(|w| w.layer.hwnd == hwnd)?;
    let (lx, ly) = ((l.0 & 0xffff) as i16 as i32, ((l.0 >> 16) & 0xffff) as i16 as i32);
    let win = &s.wins[w];
    let p = ((win.layer.x + lx) as f32, (win.layer.y + ly) as f32);
    let sc = win.g.scale;
    Some((w, p, (lx as f32 / sc, ly as f32 / sc)))
}

/// The front's box under a DIP point: (hit keys innermost first, the clickable key, on the toolbar / picker).
fn hit(win: &MonWin, d: (f32, f32)) -> (Vec<Key>, Option<Key>, bool) {
    let Some(front) = &win.front else { return (Vec::new(), None, false) };
    match front.hit(d.0, d.1) {
        Some((i, keys)) => {
            let click = front.clickable(i);
            let bars = keys.iter().any(|k| view::is_bars(*k));
            (keys, click, bars)
        }
        None => (Vec::new(), None, false),
    }
}

/// The page-wide position of a pointer on another monitor's window: which window owns the desktop point.
fn win_at(s: &Session, p: (f32, f32)) -> Option<usize> {
    s.wins.iter().position(|w| {
        let l = &w.layer;
        p.0 >= l.x as f32 && p.1 >= l.y as f32 && p.0 < (l.x + l.w) as f32 && p.1 < (l.y + l.h) as f32
    })
}

/// A click on a key of the bars / toolbar / picker.
fn click(s: &mut Session, k: Key, t: f64) -> Out {
    let m = &mut s.cap.model;
    if k == K_X {
        return Out::Close;
    }
    if k == K_SZ {
        m.size_start();
        return Out::Nothing;
    }
    if k == K_LIVE {
        let on = !m.live;
        return m.set_live(on);
    }
    if k == K_SNAP {
        return m.snap(t);
    }
    if k == K_UNDO {
        m.undo(false, t);
        return Out::Nothing;
    }
    if k == K_COPY {
        return m.finish(Finish::Copy, t);
    }
    if k == K_SAVE {
        return m.finish(Finish::Save, t);
    }
    if k == K_MORE {
        m.picker_big(true, t);
        return Out::Nothing;
    }
    if k == K_BACK {
        m.picker_big(false, t);
        return Out::Nothing;
    }
    for i in 0..m.presets().len() {
        if k == idx(K_MON, i) {
            m.pick_preset(i, t);
            return Out::Nothing;
        }
    }
    for (i, tool) in Tool::ALL.iter().enumerate() {
        if k == idx(K_TOOL, i) {
            m.pick_tool(*tool, t);
            return Out::Nothing;
        }
    }
    for (i, c) in COLORS.iter().enumerate() {
        if k == idx(K_DOT, i) {
            m.pick_color(*c);
            return Out::Nothing;
        }
    }
    for i in 0..m.recent.0.len() {
        if k == idx(K_EQ, i) {
            let e = m.recent.0[i].clone();
            m.pick_emoji(&e, false);
            return Out::Nothing;
        }
    }
    let tops = view::section_tops(m);
    for i in 0..tops.len().saturating_sub(1) {
        if k == idx(K_ETAB, i) {
            m.picker.scroll = tops[i];
            m.picker.tab = i;
            return Out::Nothing;
        }
    }
    let mut n = 0;
    for (_, list) in view::sections(m) {
        for e in list {
            if k == idx(K_EB, n) {
                m.pick_emoji(&e, true);
                return Out::Nothing;
            }
            n += 1;
        }
    }
    Out::Nothing
}

/// The lit tab follows the list's scroll (at the very bottom: the last section).
fn sync_tab(m: &mut super::model::Model) {
    let tops = view::section_tops(m);
    let content = *tops.last().unwrap_or(&0.0);
    let max = (content - 244.0).max(0.0);
    m.picker.scroll = m.picker.scroll.clamp(0.0, max);
    let mut tab = 0;
    for (i, t) in tops.iter().take(tops.len().saturating_sub(1)).enumerate() {
        if t - 6.0 <= m.picker.scroll {
            tab = i;
        }
    }
    if m.picker.scroll >= max - 2.0 && max > 0.0 {
        tab = tops.len().saturating_sub(2);
    }
    m.picker.tab = tab;
}

/// Carry out an `Out`; returns what to do after the borrow is released.
enum After {
    None,
    Close(Option<Done>),
    Error(String),
}

fn act(s: &mut Session, out: Out) -> After {
    let t = now();
    let was_live = s.cap.model.live;
    match out {
        // Order 048: a new picture (Live off / Snap) and Copy / Save run on a worker - the overlay keeps answering
        Out::Live(false) | Out::Snap => {
            let tag = s.next_tag();
            s.grab_pending = Some(tag.seq);
            s.show_live |= was_live;
            worker::spawn(tag, s.data_dir.clone(), Job::Grab, msg_window());
            return After::None;
        }
        Out::Finish(kind) => {
            // a picture still on its way: the finish takes this moment itself
            let Some(job) = s.cap.finish_job(kind, s.grab_pending.is_some()) else { return After::None };
            let tag = s.next_tag();
            s.grab_pending = None;
            s.finish_pending = Some(tag.seq);
            // the white flash over the box, then the overlay fades (coFinish: 110 ms, then 150 ms); the shot (toast) comes
            // when the worker is done
            s.closing = Some((t, None));
            worker::spawn(tag, s.data_dir.clone(), Job::Finish(job), msg_window());
            return After::None;
        }
        _ => {}
    }
    match s.cap.apply(out.clone()) {
        Ok(Step::Stay) => {
            if was_live && !s.cap.model.live {
                slice_frozen(s);
            }
            After::None
        }
        Ok(Step::Closed) => After::Close(None),
        Ok(Step::Shot(d)) => After::Close(Some(d)),
        Err(e) => After::Error(format!("Screenshot failed: {e}")),
    }
}

/// A worker's result came in (WM_JOB_DONE): the opening overlay, a new picture, a finished shot.
fn job_done() {
    for (tag, res) in worker::take() {
        match res {
            Res::Grab(r) if tag.seq == 0 => opened(tag, r),
            Res::Grab(r) => {
                let after = SESSION.with(|c| {
                    let mut b = c.borrow_mut();
                    let s = b.as_mut().filter(|s| s.id == tag.session && s.grab_pending == Some(tag.seq))?;
                    s.grab_pending = None;
                    s.show_live = false;
                    match r {
                        Ok(f) => {
                            s.cap.frozen = f;
                            slice_frozen(s);
                            None
                        }
                        Err(e) => Some(After::Error(format!("Screenshot failed: {e}"))),
                    }
                });
                match after {
                    Some(a) => finish_after(a),
                    None => {
                        if paint_all() {
                            tick_timer(true);
                        }
                    }
                }
            }
            Res::Finish(r) => {
                // still fading out: the shot waits for the fade's end (WM_TIMER); a failure closes now
                let open = SESSION.with(|c| c.borrow().as_ref().is_some_and(|s| s.id == tag.session && s.finish_pending == Some(tag.seq)));
                match open {
                    true => {
                        let a = SESSION.with(|c| {
                            let mut b = c.borrow_mut();
                            let Some(s) = b.as_mut() else { return After::None };
                            s.finish_pending = None;
                            match r {
                                Ok(d) => {
                                    if let Some(cl) = s.closing.as_mut() {
                                        cl.1 = Some(d);
                                    }
                                    After::None
                                }
                                Err(e) => After::Error(format!("Screenshot failed: {e}")),
                            }
                        });
                        finish_after(a);
                    }
                    false => {
                        // the overlay is already gone: the toast now
                        let late = LATE.with(|l| {
                            let mut l = l.borrow_mut();
                            l.iter().position(|x| x.tag == tag).map(|i| l.remove(i))
                        });
                        if let Some(late) = late {
                            match r {
                                Ok(d) => shot_done(d, &late.home, late.hooks),
                                Err(e) => (late.hooks.on_error)(&format!("Screenshot failed: {e}")),
                            }
                        }
                    }
                }
            }
        }
    }
}

fn finish_after(a: After) {
    match a {
        After::None => {}
        After::Close(done) => close(done),
        After::Error(msg) => {
            let hooks = SESSION.with(|c| c.borrow().as_ref().map(|s| s.hooks.clone()));
            close(None);
            if let Some(h) = hooks {
                (h.on_error)(&msg);
            }
        }
    }
}

/// Close the overlay (drop its windows), then show what the shot needs: the flash (a click) and the toast.
fn close(done: Option<Done>) {
    let s = SESSION.with(|c| c.borrow_mut().take());
    let Some(s) = s else { return };
    // nothing to watch any more (Order 048)
    watchdog::close();
    KEEP.with(|k| *k.borrow_mut() = Some(s.cap.model.keep()));
    let hooks = s.hooks.clone();
    let home = s.cap.model.monitors[view::bars_monitor(&s.cap.model)].clone();
    // a Copy / Save still running on its worker: its toast comes when it is done (`job_done`)
    if let (None, Some(seq)) = (&done, s.finish_pending) {
        LATE.with(|l| l.borrow_mut().push(LateShot { tag: Tag { session: s.id, seq }, hooks: hooks.clone(), home: home.clone() }));
    }
    drop(s);
    // Order 051: the menu may still draw on the same GPU device - the overlay's GPU memory goes now
    crate::gpu::trim_current();
    if let Some(d) = done {
        shot_done(d, &home, hooks);
    }
}

/// A shot is done: the gallery hears of it, the flash (a click) and the toast.
fn shot_done(d: Done, home: &Monitor, hooks: Rc<Hooks>) {
    if let Some(shot) = &d.shot {
        (hooks.on_shot)(shot);
    }
    if let Some(r) = d.flash {
        flash(r);
    }
    show_toast(d, home, hooks);
}

/// `#flash`: a soft white flash over the screen(s) taken (0 -> 50 % at 18 % -> 0, 440 ms, ease-out).
fn flash(r: bu_screenshot::Rect) {
    let Ok(layer) = Layer::new(r.x, r.y, r.w as i32, r.h as i32, true) else { return };
    unsafe {
        let _ = ShowWindow(layer.hwnd, SW_SHOWNA);
        let _ = SetTimer(Some(layer.hwnd), TIMER, 15, None);
    }
    FLASH.with(|f| f.borrow_mut().push(FlashWin { layer, at: now() }));
    paint_flash();
}

fn paint_flash() -> bool {
    FLASH.with(|f| {
        let Ok(mut v) = f.try_borrow_mut() else { return true };
        let t = now();
        v.retain_mut(|fw| {
            let k = ((t - fw.at) / 440.0).clamp(0.0, 1.0);
            if k >= 1.0 {
                return false;
            }
            let e = crate::anim::EASE_OUT_CSS.ease(k);
            let op = if e < 0.18 { 0.5 * e / 0.18 } else { 0.5 * (1.0 - (e - 0.18) / 0.82) };
            if let Some(mut surf) = fw.layer.surface() {
                surf.canvas().clear(sk::Color::WHITE);
            }
            fw.layer.present(op as f32);
            true
        });
        !v.is_empty()
    })
}

/// The capture toast: bottom-right of the work area of the monitor the shot was taken on (left of the menu when it is open).
fn show_toast(done: Done, mon: &Monitor, hooks: Rc<Hooks>) {
    TOAST.with(|t| *t.borrow_mut() = None);
    let Some(img) = super::compose::to_sk(&done.image).map(Rc::new) else { return };
    let s = mon.dpi as f32 / 96.0;
    let g = Gfx::new(s);
    // the toast's size (DIPs), with room for its shadow (0 14px 36px)
    let el = toast::toast(img.clone(), done.title, &done.sub, 500.0);
    let l = Laid::new(&g, El::row().items(taffy::style::AlignItems::FLEX_START).child(el), 10_000.0, None);
    let (tw, th) = (l.nodes[1].rect.2, l.nodes[1].rect.3);
    const M: f32 = 56.0;
    let work = unsafe {
        let hm = MonitorFromPoint(POINT { x: mon.rect.x + 1, y: mon.rect.y + 1 }, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(hm, &mut mi);
        mi.rcWork
    };
    let mut right = work.right - (12.0 * s).round() as i32;
    if let Some(m) = (hooks.menu_rect)() {
        if m.left < work.right && m.right > work.left && m.bottom > work.top {
            right = m.left - (12.0 * s).round() as i32;
        }
    }
    let bottom = work.bottom - (12.0 * s).round() as i32;
    let (pw, ph) = (((tw + 2.0 * M) * s).ceil() as i32, ((th + 2.0 * M) * s).ceil() as i32);
    let (px, py) = (right - (tw * s).round() as i32 - (M * s).round() as i32, bottom - (th * s).round() as i32 - (M * s).round() as i32);
    let Ok(layer) = Layer::new(px, py, pw, ph, true) else { return };
    // the thumbnail's box on screen: the toast's padding 10 px
    let thumb_from = (px as f32 + (M + 10.0) * s, py as f32 + (M + (th - 45.0) / 2.0) * s, 80.0 * s, 45.0 * s);
    let target = (hooks.gallery_target)().map(|r| (r, true)).or_else(|| (hooks.tray_rect)().map(|r| (r, false)));
    unsafe {
        let _ = ShowWindow(layer.hwnd, SW_SHOWNA);
        let _ = SetTimer(Some(layer.hwnd), TIMER, 15, None);
    }
    TOAST.with(|t| *t.borrow_mut() = Some(ToastWin { layer, fly: None, g, scale: s, img, done, at: now(), thumb_from, target, hooks, landed: false }));
    paint_toast();
}

fn paint_toast() -> bool {
    let landed = TOAST.with(|c| {
        let Ok(mut b) = c.try_borrow_mut() else { return None };
        let tw = b.as_mut()?;
        let t = now() - tw.at;
        const M: f32 = 56.0;
        let el = toast::toast(tw.img.clone(), tw.done.title, &tw.done.sub, t);
        let s = tw.scale;
        let (w, h) = (tw.layer.w as f32 / s, tw.layer.h as f32 / s);
        let front = El::block().size(w, h).child(el.abs(M, M, f32::NAN, f32::NAN));
        let sc = Scene { w, h, scale: s, back: El::block().size(w, h), front, glass: Vec::new(), busy: false };
        if let Some(mut surf) = tw.layer.surface() {
            // the toast's backdrop blur needs the screen behind it, which a layered window cannot read: the glass is
            // drawn without it (rgba(32,34,42,.8) over the screen) - see the report
            view::paint(&tw.g, &Icons::new(), &mut surf, &sc, None);
        }
        tw.layer.present(1.0);
        // the flying thumbnail
        if let Some((r, gallery)) = tw.target {
            let to = if gallery {
                (r.left as f32, r.top as f32, (r.right - r.left) as f32, (r.bottom - r.top) as f32)
            } else {
                toast::tray_box((r.left as f32, r.top as f32, (r.right - r.left) as f32, (r.bottom - r.top) as f32))
            };
            match toast::fly_at(t, tw.thumb_from, to, gallery) {
                Some(((x, y, fw, fh), op)) => {
                    let (iw, ih) = (fw.ceil().max(1.0) as i32 + 2, fh.ceil().max(1.0) as i32 + 2);
                    if tw.fly.is_none() {
                        tw.fly = Layer::new(x as i32, y as i32, iw, ih, true).ok();
                        if let Some(f) = &tw.fly {
                            unsafe {
                                let _ = ShowWindow(f.hwnd, SW_SHOWNA);
                            }
                        }
                    }
                    if let Some(f) = &mut tw.fly {
                        f.x = x.floor() as i32;
                        f.y = y.floor() as i32;
                        let _ = f.resize(iw, ih);
                        if let Some(mut surf) = f.surface() {
                            let g = Gfx::new(1.0);
                            g.begin(surf.canvas());
                            surf.canvas().clear(sk::Color::TRANSPARENT);
                            let (ox, oy) = (x - x.floor(), y - y.floor());
                            let r5 = 5.0 * fw / tw.thumb_from.2;
                            g.push_clip_rr4(ox, oy, fw, fh, [r5; 4]);
                            let th = toast::thumb(tw.img.clone(), fw, fh);
                            let l = Laid::new(&g, El::block().size(iw as f32, ih as f32).child(th.abs(ox, oy, f32::NAN, f32::NAN)), iw as f32, Some(ih as f32));
                            l.paint(&g, &Icons::new(), 0.0, 0.0, None);
                            g.pop_clip();
                            g.end();
                        }
                        f.present(op);
                    }
                }
                None => {
                    if t > toast::FLY_AT + toast::FLY_MS {
                        tw.fly = None;
                        if !tw.landed {
                            tw.landed = true;
                            if !gallery {
                                return Some(tw.hooks.clone());
                            }
                        }
                    }
                }
            }
        }
        None
    });
    if let Some(h) = landed {
        (h.on_tray_landed)();
    }
    TOAST.with(|c| {
        let Ok(mut b) = c.try_borrow_mut() else { return true };
        let done = b.as_ref().is_some_and(|tw| now() - tw.at > toast::FADE_AT + 260.0 && now() - tw.at > toast::FLY_AT + toast::FLY_MS + 20.0);
        if done {
            *b = None;
            return false;
        }
        b.is_some()
    })
}

/// The cursor for where the pointer is.
fn set_cursor(s: &Session, w: usize, d: (f32, f32)) {
    let m = &s.cap.model;
    let (_, click, _) = hit(&s.wins[w], d);
    let id = if let Some(c) = super::view::handle_at(m, s.wins[w].mi, d.0, d.1) {
        Some(if matches!(c, Corner::Tl | Corner::Br) { IDC_SIZENWSE } else { IDC_SIZENESW })
    } else if click == Some(K_SZ) && m.mode == Mode::Edit {
        Some(IDC_IBEAM)
    } else if click.is_some() {
        Some(IDC_HAND)
    } else {
        match m.pointer() {
            Pointer::Move => Some(IDC_SIZEALL),
            Pointer::Text => Some(IDC_IBEAM),
            Pointer::Sys => Some(IDC_ARROW),
            _ => None,
        }
    };
    unsafe {
        match id {
            Some(i) => {
                let _ = SetCursor(LoadCursorW(None, i).ok());
            }
            None => {
                let _ = SetCursor(None);
            }
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        // Order 048: this thread answers the watchdog; if the watchdog had to take the overlay off the screen (this thread
        // was stuck), the capture ends now that the thread runs again - the user has gone on meanwhile
        if msg == watchdog::WM_BEAT {
            watchdog::beat();
            return LRESULT(0);
        }
        // (the flag is only taken where the capture can be closed: inside a nested message the session is borrowed - the
        // next message closes it)
        let free = SESSION.with(|c| c.try_borrow_mut().is_ok());
        if free && watchdog::take_fired().is_some() {
            let hooks = SESSION.with(|c| c.borrow().as_ref().map(|s| s.hooks.clone()));
            if let Some(h) = hooks {
                close(None);
                (h.on_error)("The screenshot overlay stopped answering, so it was closed. Press the key again.");
            }
            if !IsWindow(Some(hwnd)).as_bool() {
                return LRESULT(0);
            }
        }
        if msg == worker::WM_JOB_DONE {
            job_done();
            return LRESULT(0);
        }
        if msg == WM_TIMER && wp.0 == TIMER {
            let a = paint_all();
            let b = paint_flash();
            let c = paint_toast();
            // finish a closing overlay
            let closing = SESSION.with(|c| c.try_borrow().ok().and_then(|b| b.as_ref().and_then(|s| s.closing.as_ref().map(|x| x.0))));
            if let Some(at) = closing {
                if now() - at > 260.0 {
                    let d = SESSION.with(|c| c.borrow_mut().as_mut().and_then(|s| s.closing.take().and_then(|x| x.1)));
                    close(d);
                }
            }
            if !(a || b || c) && closing.is_none() {
                let _ = KillTimer(Some(hwnd), TIMER);
            }
            return LRESULT(0);
        }
        let mut after = After::None;
        let mut handled = true;
        let mut repaint = true;
        SESSION.with(|c| {
            let Ok(mut b) = c.try_borrow_mut() else {
                handled = false;
                return;
            };
            let Some(s) = b.as_mut() else {
                handled = false;
                return;
            };
            if s.closing.is_some() {
                repaint = false;
                return;
            }
            let t = now();
            // Order 045: a press or the wheel hides the tip (the drawing's `tipHide()`); it comes back on another element
            if matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MOUSEWHEEL) {
                for win in &mut s.wins {
                    let hv = win.st.hover.clone();
                    win.tips.hide(t, &hv);
                }
            }
            match msg {
                WM_MOUSEMOVE => {
                    let Some((w, p, d)) = points(s, hwnd, lp) else { return };
                    // with the button held the pointer may be over another monitor's window
                    let ow = if GetCapture() == hwnd { win_at(s, p).unwrap_or(w) } else { w };
                    let od = {
                        let l = &s.wins[ow].layer;
                        let sc = s.wins[ow].g.scale;
                        ((p.0 - l.x as f32) / sc, (p.1 - l.y as f32) / sc)
                    };
                    let (keys, _, _) = hit(&s.wins[ow], od);
                    let over = !keys.is_empty() && keys.iter().any(|k| *k != K_HD);
                    s.cap.model.moved(p, over);
                    let omi = s.wins[ow].mi;
                    for win in &mut s.wins {
                        win.st.hover = if win.mi == omi { keys.clone() } else { Vec::new() };
                        for k in &win.st.hover {
                            win.st.hover_since.entry(*k).or_insert(t);
                        }
                        let hv = win.st.hover.clone();
                        win.st.hover_since.retain(|k, _| hv.contains(k));
                    }
                    set_cursor(s, w, d);
                    let _ = SetTimer(Some(hwnd), TIMER, 15, None);
                }
                WM_LBUTTONDOWN => {
                    let Some((w, p, d)) = points(s, hwnd, lp) else { return };
                    let _ = SetCapture(hwnd);
                    let (keys, click, _) = hit(&s.wins[w], d);
                    // a press left from a lost button-up must not take this press's release (the drag would stay stuck)
                    s.pressed = None;
                    if let Some(k) = click {
                        s.pressed = Some(k);
                        s.wins[w].st.active = keys;
                    } else if keys.iter().any(|k| *k == K_EGRID || *k == K_SRCH || view::is_bars(*k) || *k == K_BAR) {
                        // on a bar's empty part: nothing
                    } else {
                        if crate::testmode::on() {
                            crate::timing::note(&format!("ovpress model at {:.0},{:.0}", d.0, d.1));
                        }
                        let handle = view::handle_at(&s.cap.model, s.wins[w].mi, d.0, d.1);
                        let out = s.cap.model.press(p, handle, t);
                        after = act(s, out);
                    }
                }
                WM_LBUTTONUP => {
                    let _ = ReleaseCapture();
                    let Some((w, _, d)) = points(s, hwnd, lp) else { return };
                    for win in &mut s.wins {
                        win.st.active.clear();
                    }
                    if let Some(k) = s.pressed.take() {
                        let (_, click, _) = hit(&s.wins[w], d);
                        if click == Some(k) {
                            if crate::testmode::on() {
                                crate::timing::note(&format!("ovclicked key={k:x}"));
                            }
                            let out = click_key(s, k, t);
                            after = act(s, out);
                        }
                    } else {
                        let out = s.cap.model.release(t);
                        after = act(s, out);
                    }
                }
                WM_RBUTTONDOWN => {
                    let Some((w, _, d)) = points(s, hwnd, lp) else { return };
                    let (_, _, bars) = hit(&s.wins[w], d);
                    let out = s.cap.model.right_press(bars, t);
                    after = act(s, out);
                }
                WM_MOUSEWHEEL => {
                    let m = &mut s.cap.model;
                    if m.picker.open && m.picker.big {
                        let delta = ((wp.0 >> 16) & 0xffff) as i16 as f32;
                        m.picker.scroll -= delta / 120.0 * 100.0;
                        sync_tab(m);
                    }
                }
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    let vk = wp.0 as u16;
                    let scan = ((lp.0 >> 16) & 0xff) as u32;
                    let repeat = (lp.0 >> 30) & 1 == 1;
                    let ch = char::from_u32(MapVirtualKeyW(vk as u32, MAPVK_VK_TO_CHAR) & 0xffff).map(|c| c.to_ascii_lowercase());
                    let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                    let out = s.cap.model.key(vk, ch, ctrl, scan == 0x2C, repeat, t);
                    after = act(s, out);
                }
                WM_CHAR => {
                    if let Some(c) = char::from_u32(wp.0 as u32) {
                        s.cap.model.char_input(c);
                    }
                }
                WM_ACTIVATE => {
                    s.cap.model.focused = (wp.0 & 0xffff) != 0;
                }
                WM_SETCURSOR => {
                    repaint = false;
                    let mut p = POINT::default();
                    let _ = GetCursorPos(&mut p);
                    if let Some(w) = win_at(s, (p.x as f32, p.y as f32)) {
                        let l = &s.wins[w].layer;
                        let sc = s.wins[w].g.scale;
                        set_cursor(s, w, ((p.x - l.x) as f32 / sc, (p.y - l.y) as f32 / sc));
                    }
                }
                WM_DISPLAYCHANGE => {
                    // the screens changed under the overlay (the drawing closes on a resize)
                    after = After::Close(None);
                }
                _ => {
                    handled = false;
                    repaint = false;
                }
            }
        });
        if !handled {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let closing = !matches!(after, After::None);
        finish_after(after);
        if repaint && !closing && paint_all() {
            let _ = SetTimer(Some(hwnd), TIMER, 15, None);
        }
        if msg == WM_SETCURSOR {
            return LRESULT(1);
        }
        LRESULT(0)
    }
}

fn click_key(s: &mut Session, k: Key, t: f64) -> Out {
    click(s, k, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_copy_never_opens_the_overlay() {
        assert!(refused(true).is_some());
        assert!(refused(false).is_none());
    }
}
