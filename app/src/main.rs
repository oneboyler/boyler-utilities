//! Boyler Utilities — the tray app (fully Rust, native UI; grown from bake-off test A).
//! A tray icon; double-click opens the glass menu.
//! While the menu is closed the app only waits for messages: no timers, no polling, no graphics objects.
//!
//! Entry points — the same one the tray double-click uses (`App::command`):
//!   "Boyler Utilities.exe" --open | --close | --toggle | --tab <name> | --quit
//! Test mode (testmode.rs): `--test` or any test switch (--offscreen, --no-tray, --fake-audio, --real-read, --frozen, --log,
//! --log-frames, --cmd, --lab, --make-icons) makes a TEST copy with its own mutex and window classes; only a test copy
//! takes the test-only commands: --cmd "click:<target>" | "snap:<png>" | "state:<txt>" ...

#![windows_subsystem = "windows"]

mod anim;
mod appicon;
mod appinfo;
mod audio;
mod capture;
mod clipboard;
mod d2dref;
mod comp;
mod effects;
mod droptarget;
mod gfx;
mod gpu;
mod shadercache;
mod guides;
mod icons;
mod icons_v22;
mod lab;
mod menu;
mod pages;
#[cfg(test)]
mod proof045;
mod services;
mod png;
mod present;
mod svg;
mod testmode;
mod textparams;
mod timing;
mod vsync;
mod dib;
mod tray;
mod ui;
mod settings;
mod keys;
mod undo;
mod jobs;
mod keep;
mod addons;
mod admin;
mod obs;
mod offui;
mod welcome;

use std::cell::RefCell;
use std::collections::VecDeque;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::*;
use windows::Win32::System::WinRT::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::WindowsAndMessaging::*;


// ------------------------------------------------------------------ messages -> events (no re-entrancy into App)
enum Ev {
    /// a command handed over by a second start (with its WM_COPYDATA magic)
    Cmd(String, usize),
    TrayDbl,
    TrayDown,
    TrayHover,
    WarmTimeout,
    TrayMenu(i32, i32),
    Move(f32, f32),
    Leave,
    Down(f32, f32),
    Up(f32, f32),
    /// the right button came up (a page's context menu)
    RUp(f32, f32),
    /// files / folders dropped on the menu at (x, y)
    Drop(f32, f32, Vec<String>),
    /// Order 045: files dragged over the menu at (x, y) / gone (Windows' drag and drop, droptarget.rs)
    DragOver(Option<(f32, f32)>),
    Wheel(f32),
    Key(u16),
    Char(char),
    Deactivate,
    /// the menu got the focus back (a click on it while it sat behind another app)
    Activate,
    /// a key, a key field or a job changed something (services.rs)
    Services,
    /// the desktop under the menu changed (BU_GLASS=own)
    Capture,
    TaskbarCreated,
    Theme,
    /// the desktop's resolution changed (WM_DISPLAYCHANGE): the open menu goes back to its corner (Order 042)
    Display,
    /// start the next page's background part (Order 048: one per wake-up, after the tray is up)
    StartBg,
}

thread_local! {
    /// a test copy's timed commands (`after:<ms>|<command>`): when, what
    static SCHED: RefCell<Vec<(f64, String)>> = const { RefCell::new(Vec::new()) };
}

thread_local! {
    /// queued events + the modifiers held when each one's message arrived
    static EVENTS: RefCell<VecDeque<(Ev, ui::cx::Mods)>> = const { RefCell::new(VecDeque::new()) };
    static CURSOR: RefCell<PCWSTR> = const { RefCell::new(IDC_ARROW) };
    static MENU_HWND: RefCell<HWND> = RefCell::new(HWND::default());
    static SCALE: RefCell<f32> = const { RefCell::new(1.0) };
    static OFFSCREEN: RefCell<bool> = const { RefCell::new(false) };
    static TASKBAR_CREATED: RefCell<u32> = const { RefCell::new(0) };
    static MSG_HWND: RefCell<HWND> = RefCell::new(HWND::default());
    static WAKE_POSTED: RefCell<bool> = const { RefCell::new(false) };
    /// a modal window of Windows' own runs over the menu (the file picker, a drag out): the menu losing the focus to it
    /// must not close the menu (REVIEW_014_item1c HOLD 2)
    static MODAL: RefCell<bool> = const { RefCell::new(false) };
}

/// The menu lost the focus to the process `to_pid` (0 = unknown): it closes like after a click outside unless Windows' own
/// modal UI runs over it or the focus went to one of the app's OWN windows (a page's capture overlay, a picker).
fn closes_on_focus_loss(to_pid: u32, own_pid: u32, modal: bool) -> bool {
    !modal && (to_pid == 0 || to_pid != own_pid)
}

/// Run Windows' own modal UI over the menu (`ui::picker`, `ui::dragout`): while it runs, the menu's deactivation is not
/// a click outside. After it, the focus is back on the menu (the picker's owner) - or, if it went to another app (a drop
/// target that activates itself, a click elsewhere meanwhile), the menu closes like after any click outside.
pub(crate) fn modal<R>(f: impl FnOnce() -> R) -> R {
    MODAL.with(|m| *m.borrow_mut() = true);
    let r = f();
    MODAL.with(|m| *m.borrow_mut() = false);
    let menu = MENU_HWND.with(|h| *h.borrow());
    if !menu.is_invalid() && unsafe { GetForegroundWindow() } != menu {
        push(Ev::Deactivate);
    }
    r
}

const WM_WAKE: u32 = WM_APP + 2;
/// the app's setting holding the tab shown when the menu last closed (feedback F3)
const LAST_TAB: &str = "last_tab";

/// This start is the restart after a self-update: the version it came from (Settings › Updates can say so).
pub(crate) static UPDATED_FROM: std::sync::OnceLock<String> = std::sync::OnceLock::new();
/// The last self-update did NOT go through: what its install step wrote (read once at start and deleted) - Settings ›
/// Updates shows `message` as a note (Order 017: a failed update is never silent).
pub(crate) static LAST_UPDATE_FAILED: std::sync::OnceLock<bu_updater::SwapResult> = std::sync::OnceLock::new();
/// one-shot: drop the graphics device warmed up by a tray hover if no open followed
const TIMER_WARM: usize = 7;
const WARM_MS: u32 = 4000;
/// one-shot (Order 042): the menu goes to its corner again a moment after a resolution change - Windows may still move
/// windows and the taskbar's work area right after WM_DISPLAYCHANGE
const TIMER_ANCHOR: usize = 8;
/// one-shot (Order 048): the next page's background part starts (spread out after the start, `App::start_next_bg`)
const TIMER_BG: usize = 9;
/// the gap between two pages' background parts at the start
const BG_GAP_MS: u32 = 40;

fn push(e: Ev) {
    EVENTS.with(|q| {
        let mut q = q.borrow_mut();
        // Order 055: a move right after a move not handled yet replaces it - only where the pointer is NOW matters (a
        // 1000-8000 Hz mouse sends many per frame; each one was a hit test, a hover check and the top row's magnification)
        if let (Ev::Move(..), Some((Ev::Move(..), _))) = (&e, q.back()) {
            q.pop_back();
        }
        q.push_back((e, ui::cx::Mods::read()));
    });
    // sent messages (WM_COPYDATA, tray callbacks) are handled inside GetMessage without it returning:
    // post a small wake-up so the loop sees the event right away
    if !WAKE_POSTED.with(|w| *w.borrow()) {
        let h = MSG_HWND.with(|h| *h.borrow());
        if !h.is_invalid() {
            unsafe {
                let _ = PostMessageW(Some(h), WM_WAKE, WPARAM(0), LPARAM(0));
            }
            WAKE_POSTED.with(|w| *w.borrow_mut() = true);
        }
    }
}

fn dip(l: LPARAM) -> (f32, f32) {
    let x = (l.0 & 0xffff) as i16 as f32;
    let y = ((l.0 >> 16) & 0xffff) as i16 as f32;
    let s = SCALE.with(|s| *s.borrow());
    (x / s, y / s)
}

extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        let is_menu = MENU_HWND.with(|x| *x.borrow()) == h;
        match m {
            tray::WM_TRAY => {
                let ev = (l.0 & 0xffff) as u32;
                match ev {
                    WM_LBUTTONDBLCLK => push(Ev::TrayDbl),
                    WM_LBUTTONDOWN | WM_LBUTTONUP => push(Ev::TrayDown),
                    WM_MOUSEMOVE => push(Ev::TrayHover),
                    WM_CONTEXTMENU => {
                        let x = (w.0 & 0xffff) as i16 as i32;
                        let y = ((w.0 >> 16) & 0xffff) as i16 as i32;
                        push(Ev::TrayMenu(x, y));
                    }
                    _ => {}
                }
                return LRESULT(0);
            }
            WM_TIMER if w.0 == TIMER_WARM => {
                let _ = KillTimer(Some(h), TIMER_WARM);
                push(Ev::WarmTimeout);
                return LRESULT(0);
            }
            // Order 050: Windows is signing out - the settings writer's newest file goes to disk first (it is off the UI
            // thread now; the process ends right after this message)
            WM_ENDSESSION if w.0 != 0 && !is_menu => {
                services::try_with(|s| s.store.wait_written(std::time::Duration::from_secs(2)));
                wait_broadcasts();
                return LRESULT(0);
            }
            WM_COPYDATA => {
                let cds = &*(l.0 as *const COPYDATASTRUCT);
                if !cds.lpData.is_null() {
                    let bytes = std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize);
                    push(Ev::Cmd(String::from_utf8_lossy(bytes).to_string(), cds.dwData));
                }
                return LRESULT(1);
            }
            WM_DISPLAYCHANGE if !is_menu => {
                push(Ev::Theme);
                push(Ev::Display);
                SetTimer(Some(h), TIMER_ANCHOR, 600, None);
            }
            WM_TIMER if w.0 == TIMER_BG => {
                let _ = KillTimer(Some(h), TIMER_BG);
                push(Ev::StartBg);
                return LRESULT(0);
            }
            WM_TIMER if w.0 == TIMER_ANCHOR => {
                let _ = KillTimer(Some(h), TIMER_ANCHOR);
                push(Ev::Display);
                return LRESULT(0);
            }
            WM_SETTINGCHANGE | WM_DPICHANGED | WM_DISPLAYCHANGE if !is_menu => push(Ev::Theme),
            // the keys manager's keys (services.rs): hotkeys, and Raw Input for mouse-button / modifier-only keys
            WM_HOTKEY if !is_menu => {
                services::hotkey(w.0);
                push(Ev::Services);
                return LRESULT(0);
            }
            // bu-rawin's thread reads Raw Input; it wakes this window only for packets that matter (keys, mouse
            // buttons / wheel - never a move), once per batch (Order 048)
            services::WM_RAWKEYS if !is_menu => {
                if services::raw_packets() {
                    push(Ev::Services);
                }
                return LRESULT(0);
            }
            // a job's progress or end (jobs/): the menu repaints
            services::WM_JOB => {
                push(Ev::Services);
                return LRESULT(0);
            }
            WM_MOUSEMOVE if is_menu => {
                let (x, y) = dip(l);
                push(Ev::Move(x, y));
                // (an off-screen test copy is never under the real pointer: Windows would answer every posted move with a
                // leave at once - Order 055's cost measurement posts moves into it)
                if !OFFSCREEN.with(|o| *o.borrow()) {
                    let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut t);
                }
                return LRESULT(0);
            }
            WM_MOUSELEAVE if is_menu => {
                push(Ev::Leave);
                return LRESULT(0);
            }
            WM_LBUTTONDOWN if is_menu => {
                let (x, y) = dip(l);
                SetCapture(h);
                push(Ev::Down(x, y));
                return LRESULT(0);
            }
            WM_LBUTTONUP if is_menu => {
                let (x, y) = dip(l);
                let _ = ReleaseCapture();
                push(Ev::Up(x, y));
                return LRESULT(0);
            }
            // Order 045: the menu window goes - its drop target too (Windows holds it until revoked)
            WM_DESTROY if is_menu => droptarget::revoke(h),
            WM_RBUTTONUP if is_menu => {
                let (x, y) = dip(l);
                push(Ev::RUp(x, y));
                return LRESULT(0);
            }
            WM_DROPFILES if is_menu => {
                let hd = windows::Win32::UI::Shell::HDROP(w.0 as *mut _);
                let mut pt = POINT::default();
                let _ = windows::Win32::UI::Shell::DragQueryPoint(hd, &mut pt);
                let n = windows::Win32::UI::Shell::DragQueryFileW(hd, u32::MAX, None);
                let mut paths = Vec::new();
                for i in 0..n {
                    let len = windows::Win32::UI::Shell::DragQueryFileW(hd, i, None) as usize;
                    let mut b = vec![0u16; len + 1];
                    let got = windows::Win32::UI::Shell::DragQueryFileW(hd, i, Some(&mut b)) as usize;
                    paths.push(String::from_utf16_lossy(&b[..got]));
                }
                windows::Win32::UI::Shell::DragFinish(hd);
                let (x, y) = dip(LPARAM(((pt.y as isize & 0xffff) << 16) | (pt.x as isize & 0xffff)));
                push(Ev::Drop(x, y, paths));
                return LRESULT(0);
            }
            WM_MOUSEWHEEL if is_menu => {
                let d = ((w.0 >> 16) & 0xffff) as i16 as f32;
                push(Ev::Wheel(d));
                return LRESULT(0);
            }
            // a key field listening for a key (services.rs) takes every key message, Alt combos and key-ups included
            WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP if is_menu && services::key_message(m == WM_KEYDOWN || m == WM_SYSKEYDOWN, w.0 as u16, l.0) => {
                push(Ev::Services);
                return LRESULT(0);
            }
            WM_MBUTTONDOWN | WM_XBUTTONDOWN if is_menu => {
                let b = if m == WM_MBUTTONDOWN { 3 } else if (w.0 >> 16) & 0xffff == 1 { 4 } else { 5 };
                if services::mouse_button(b) {
                    push(Ev::Services);
                }
                return LRESULT(0);
            }
            WM_KEYDOWN if is_menu => {
                push(Ev::Key(w.0 as u16));
                return LRESULT(0);
            }
            WM_CHAR if is_menu => {
                if let Some(c) = char::from_u32(w.0 as u32) {
                    push(Ev::Char(c));
                }
                return LRESULT(0);
            }
            capture::WM_CAPTURE if is_menu => {
                push(Ev::Capture);
                return LRESULT(0);
            }
            WM_ACTIVATE if is_menu => {
                // (lParam = the window being activated) one of the app's OWN windows taking the focus (a page's capture
                // overlay, a picker; owner = the menu, so the focus comes back to it when they close) is not a click
                // outside - PIECES_WANTED (Lane Q, REVIEW 019)
                let to = HWND(l.0 as *mut _);
                let mut pid = 0u32;
                if !to.is_invalid() {
                    GetWindowThreadProcessId(to, Some(&mut pid));
                }
                if (w.0 & 0xffff) as u32 == WA_INACTIVE {
                    if closes_on_focus_loss(pid, GetCurrentProcessId(), MODAL.with(|m| *m.borrow()) || admin::client::prompting()) {
                        push(Ev::Deactivate);
                    }
                } else {
                    push(Ev::Activate);
                }
                return LRESULT(0);
            }
            WM_ACTIVATEAPP if is_menu => {
                // the focus leaves the APP while one of its own windows had it (the menu got no WA_INACTIVE then): every
                // top-level window of the app hears it; lParam = a thread of the app being activated (REVIEW_014_item2 HOLD 1)
                if w.0 == 0 {
                    let mut pid = 0u32;
                    if let Ok(t) = OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, l.0 as u32) {
                        pid = GetProcessIdOfThread(t);
                        let _ = CloseHandle(t);
                    }
                    if closes_on_focus_loss(pid, GetCurrentProcessId(), MODAL.with(|m| *m.borrow()) || admin::client::prompting()) {
                        push(Ev::Deactivate);
                    }
                }
                return LRESULT(0);
            }
            WM_MOUSEACTIVATE if is_menu && OFFSCREEN.with(|o| *o.borrow()) => return LRESULT(MA_NOACTIVATE as isize),
            WM_SETCURSOR if is_menu => {
                if (l.0 & 0xffff) as u32 == HTCLIENT {
                    let c = CURSOR.with(|c| *c.borrow());
                    if let Ok(hc) = LoadCursorW(None, c) {
                        SetCursor(Some(hc));
                    }
                    return LRESULT(1);
                }
            }
            WM_NCHITTEST if !is_menu && h != MENU_HWND.with(|x| *x.borrow()) => {}
            _ => {}
        }
        let tc = TASKBAR_CREATED.with(|t| *t.borrow());
        if tc != 0 && m == tc {
            push(Ev::TaskbarCreated);
        }
        DefWindowProcW(h, m, w, l)
    }
}

// ------------------------------------------------------------------ options
#[derive(Default, Clone)]
struct Opts {
    /// a test copy (testmode.rs)
    test: bool,
    /// `--demo-names`: made-up names in the sample data (README pictures, testmode.rs)
    demo: bool,
    /// test copy reading the real services (measuring); refuses everything that would change something
    real_read: bool,
    offscreen: bool,
    /// test only: the menu at its real place on the screen, but it never takes focus (no tray icon, fake audio)
    screen_test: bool,
    no_tray: bool,
    fake: bool,
    frozen: bool,
    log: Option<String>,
    cmds: Vec<String>,
    make_icons: Option<String>,
    lab: Option<String>,
    /// test only (with --offscreen): this new copy runs its own --cmd commands (`after:` times them)
    run: bool,
}

fn parse_args() -> Opts {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut o = Opts::default();
    let mut i = 0;
    while i < a.len() {
        let next = |i: usize| a.get(i + 1).cloned().unwrap_or_default();
        // the self-update's restart (`--bu-updated <event> <old version>`, Order 017): a NORMAL start
        if a[i] == bu_updater::UPDATED_ARG {
            i += 3;
            continue;
        }
        if !matches!(a[i].as_str(), "--open" | "--close" | "--toggle" | "--quit" | "--tab") {
            // every other switch is a test switch: this is a test copy (or a test hook start)
            o.test = true;
        }
        match a[i].as_str() {
            "--test" => {}
            "--real-read" => o.real_read = true,
            // tests: the menu opens at x = -20000 and never takes focus; no tray icon either
            "--offscreen" => {
                o.offscreen = true;
                o.no_tray = true;
            }
            "--screen-test" => {
                o.screen_test = true;
                o.no_tray = true;
                o.fake = true;
            }
            "--no-tray" => o.no_tray = true,
            "--run" => o.run = true,
            "--fake-audio" => o.fake = true,
            // timing log: every presented frame, not only those of a page switch
            "--log-frames" => timing::frames_always(),
            "--frozen" => o.frozen = true,
            "--demo-names" => o.demo = true,
            "--log" => {
                o.log = Some(next(i));
                i += 1;
            }
            "--open" => o.cmds.push("open".into()),
            "--close" => o.cmds.push("close".into()),
            "--toggle" => o.cmds.push("toggle".into()),
            "--quit" => o.cmds.push("quit".into()),
            "--tab" => {
                o.cmds.push(format!("tab:{}", next(i)));
                i += 1;
            }
            "--cmd" => {
                o.cmds.push(next(i));
                i += 1;
            }
            // developer tools (they write files and leave; only with an explicit --test)
            "--lab" => {
                o.lab = Some(next(i));
                i += 1;
            }
            "--make-icons" => {
                o.make_icons = Some(next(i));
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    if o.test {
        // a test copy never shows a tray icon and never uses the real services, except to read them (--real-read)
        o.no_tray = true;
        o.fake = !o.real_read;
    }
    o
}

/// Hand commands to the running copy of the same mode (WM_COPYDATA to its message window). Returns false if none is running.
fn send_to_running(cmds: &[String]) -> bool {
    unsafe {
        let Ok(h) = FindWindowW(testmode::msg_class(), None) else { return false };
        for c in cmds {
            let bytes = c.as_bytes();
            let cds = COPYDATASTRUCT { dwData: testmode::magic(), cbData: bytes.len() as u32, lpData: bytes.as_ptr() as *mut _ };
            let mut res = 0usize;
            let _ = SendMessageTimeoutW(h, WM_COPYDATA, WPARAM(0), LPARAM(&cds as *const _ as isize), SMTO_ABORTIFHUNG, 5000, Some(&mut res));
        }
        true
    }
}

// ------------------------------------------------------------------ the app
struct App {
    msg: HWND,
    tray: tray::Tray,
    menu: Option<menu::Menu>,
    opts: Opts,
    dq: Option<windows::System::DispatcherQueueController>,
    quit: bool,
    /// the pages' background parts (Page::background), alive while the app runs
    bg: Vec<(&'static str, Box<dyn pages::Background>)>,
    /// the next open shows this tab first (+ its jump target): `services::show_menu` with the menu closed
    open_on: Option<(String, Option<String>)>,
    /// the menu is open but another app was clicked: it sits behind (not topmost) until raised (feedback F1)
    behind: bool,
    /// the pages whose background part has not started yet (Order 048: started one at a time after the tray is up -
    /// the very first start after Setup ran them all on the UI thread before it answered anything) + what they get
    bg_todo: Vec<Box<dyn pages::Page>>,
    env: pages::Env,
}

fn reduced_motion() -> bool {
    unsafe {
        let mut on = BOOL(1);
        let _ = SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        !on.as_bool()
    }
}

impl App {
    /// Where the menu opens: the work area of the monitor with the tray icon (else the main monitor), and its scale.
    fn monitor(&self) -> (RECT, f32) {
        unsafe {
            let mon = match self.tray.rect() {
                Some(r) => MonitorFromRect(&r, MONITOR_DEFAULTTOPRIMARY),
                None => MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY),
            };
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(mon, &mut mi);
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            // test only (off-screen pictures at other display scales): BU_SCALE=1.25 / 1.5 / 1.75
            let scale = testmode::env("BU_SCALE").and_then(|v| v.parse::<f32>().ok()).filter(|_| self.opts.offscreen).unwrap_or(dx as f32 / 96.0);
            (mi.rcWork, scale)
        }
    }

    fn open(&mut self) {
        let t = timing::now();
        timing::open_request(t);
        // the first-run bubble pointing at the tray icon goes once the menu opens
        welcome::dismiss();
        if let Some(m) = &mut self.menu {
            if m.ui.close_t.is_some() {
                m.ui.open(t); // re-open while it was closing
                if !self.opts.offscreen && !self.opts.screen_test {
                    unsafe {
                        let _ = SetForegroundWindow(m.hwnd);
                    }
                }
            } else if self.behind {
                // open while it sits behind another app (a key that shows a tab): it comes to the front
                self.raise();
            }
            return;
        }
        if self.dq.is_none() {
            self.dq = unsafe {
                CreateDispatcherQueueController(DispatcherQueueOptions {
                    dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
                    threadType: DQTYPE_THREAD_CURRENT,
                    apartmentType: DQTAT_COM_NONE,
                })
                .ok()
            };
        }
        // (Audio makes its own service when it opens, like every page - Order 014 item 1c)
        // Order 047 item 11: the graphics device is made on a worker meanwhile (started on tray hover, else now); the menu
        // waits for that one when it gets to its swap chain
        present::prepare_d3d();
        let (work, scale) = self.monitor();
        SCALE.with(|s| *s.borrow_mut() = scale);
        // Order 047: the menu makes only the page it shows first (a key's tab, else the tab shown last)
        let first = self.open_on.as_ref().map(|(id, _)| id.clone()).or_else(|| services::with(|s| s.store.get_str(settings::Scope::App, LAST_TAB).map(str::to_string)).flatten());
        ui::set_first_tab(first);
        match menu::Menu::new(work, scale, self.opts.offscreen, self.opts.offscreen || self.opts.screen_test, reduced_motion(), self.opts.frozen, t) {
            Ok(mut m) => {
                MENU_HWND.with(|h| *h.borrow_mut() = m.hwnd);
                admin::client::set_owner(m.hwnd.0 as isize);
                // opened for a tab (a key with the menu closed): that tab is the first one shown
                if let Some((id, target)) = self.open_on.take() {
                    m.ui.start_on(&id, target.as_deref(), t);
                } else if let Some(id) = services::with(|s| s.store.get_str(settings::Scope::App, LAST_TAB).map(str::to_string)).flatten() {
                    // feedback F3: it opens on the tab that was open last (kept in the settings file, so also after a restart)
                    m.ui.start_on(&id, None, t);
                }
                self.behind = false;
                // files / folders may be dropped on a page (Ev::Drop); Order 045: and their drag hover is seen (Ev::DragOver,
                // Security's drop zone) - Windows' drop target; if it refuses, drops still come as WM_DROPFILES
                // (Order 047, A_047_01: a test copy registers no drop target - its drops come from the `drop` test command)
                let registered = self.opts.test || droptarget::register(m.hwnd, move |e| {
                    let s = SCALE.with(|s| *s.borrow()).max(0.01);
                    match e {
                        droptarget::DropEv::Over(x, y) => push(Ev::DragOver(Some((x as f32 / s, y as f32 / s)))),
                        droptarget::DropEv::Leave => push(Ev::DragOver(None)),
                        droptarget::DropEv::Drop(x, y, paths) => push(Ev::Drop(x as f32 / s, y as f32 / s, paths)),
                    }
                });
                if !registered {
                    unsafe { windows::Win32::UI::Shell::DragAcceptFiles(m.hwnd, true) };
                }
                m.show();
                unsafe {
                    let _ = WaitForSingleObjectEx(m.waitable(), 1000, true);
                }
                timing::note("open_step waitable");
                let _ = m.frame(timing::now());
                // the menu holds the device now; it goes away with the menu
                present::KEPT_D3D.with(|k| *k.borrow_mut() = None);
                unsafe {
                    let _ = KillTimer(Some(self.msg), TIMER_WARM);
                }
                self.menu = Some(m);
                self.tray.set_open(true);
            }
            Err(e) => {
                timing::mark(&format!("open_error_{:08x}", e.code().0), timing::now());
                timing::flush();
            }
        }
    }

    /// The menu on the tab `id` (+ `target`): opened on it if closed, shown in it if open (`services::show_menu`).
    fn show_menu(&mut self, id: &str, target: Option<&str>) {
        if self.menu.is_none() {
            self.open_on = Some((id.to_string(), target.map(str::to_string)));
            self.open();
            self.open_on = None;
            return;
        }
        self.open(); // (re-opens it if it was closing)
        if let Some(m) = &mut self.menu {
            m.ui.go_to(id, target, timing::now());
        }
    }

    fn close(&mut self) {
        if let Some(m) = &mut self.menu {
            m.ui.close(timing::now());
        }
    }

    /// The mouse is on the tray icon: make the graphics device now (the slow part of opening, 130-300 ms measured), so a
    /// double-click opens at once. Dropped again WARM_MS after the last hover if the menu wasn't opened (one-shot timer;
    /// nothing runs while the icon isn't touched).
    fn warm_up(&mut self) {
        if self.menu.is_some() {
            return;
        }
        // Order 051: the GPU device + Skia's GPU context (the menu draws on it); Order 047 item 11: the CPU path's Direct3D 11
        // device (only when the GPU can't be used) is made on a worker thread - the open takes it
        let (work, _) = self.monitor();
        let mon = unsafe { MonitorFromPoint(POINT { x: (work.left + work.right) / 2, y: (work.top + work.bottom) / 2 }, MONITOR_DEFAULTTOPRIMARY) };
        let t = timing::now();
        gpu::warm(Some(mon));
        if gpu::alive() {
            timing::note(&format!("warm_up gpu {:.1} ms", timing::now() - t));
        } else if present::prepare_d3d() {
            timing::note("warm_up d3d worker started");
        }
        if menu::KEPT_GFX.with(|k| k.borrow().is_none()) {
            let t = timing::now();
            let (_, scale) = self.monitor();
            let g = gfx::Gfx::new(scale);
            ui::prime(&g);
            menu::KEPT_GFX.with(|k| *k.borrow_mut() = Some(g));
            timing::note(&format!("warm_up gfx {:.1} ms", timing::now() - t));
        }
        unsafe {
            SetTimer(Some(self.msg), TIMER_WARM, WARM_MS, None);
        }
    }

    fn cool_down(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.msg), TIMER_WARM);
        }
        let gfx_dropped = menu::KEPT_GFX.with(|k| k.borrow_mut().take()).is_some() | gpu::cool_down();
        // Order 047 item 11: no K32EmptyWorkingSet any more (the boss's order: the next open and its tab switches faulted
        // the pages back in); the kept device (or its worker) goes
        if (present::drop_d3d() || gfx_dropped) && self.menu.is_none() {
            timing::note("cool_down");
        }
    }

    /// The menu sat behind another app: on top and focused again (tray double-click).
    fn raise(&mut self) {
        self.behind = false;
        if let Some(m) = &mut self.menu {
            m.ui.dirty = true;
            m.set_topmost(true);
            if !self.opts.offscreen && !self.opts.screen_test {
                unsafe {
                    let _ = SetForegroundWindow(m.hwnd);
                }
            }
        }
    }

    fn destroy_menu(&mut self) {
        if let Some(m) = &self.menu {
            // feedback F3: the next open shows this tab again
            let id = m.ui.tab_id();
            services::with(|s| {
                let _ = s.store.set_str(settings::Scope::App, LAST_TAB, id);
                s.stop_listening();
            });
        }
        self.behind = false;
        if self.menu.take().is_some() {
            // Order 051: the overlay may still draw on the same GPU device - the menu's GPU memory goes now
            gpu::trim_current();
            MENU_HWND.with(|h| *h.borrow_mut() = HWND::default());
            admin::client::set_owner(0);
            self.tray.set_open(false);
            timing::flush();
            // (Order 047 item 11: the pages the menu used are no longer handed back to Windows here - K32EmptyWorkingSet
            // made the next open and its tab switches fault them back in)
        }
    }

    /// A command from a second start (`magic` = its WM_COPYDATA magic) or this copy's own command line (None): only
    /// what this copy accepts (testmode::accepts) gets through.
    fn command_from(&mut self, cmd: &str, magic: Option<usize>) {
        if testmode::accepts(self.opts.test, self.opts.real_read, magic, cmd) {
            self.command(cmd);
        } else {
            timing::note(&format!("refused {}", cmd.split_once(':').map(|(k, _)| k).unwrap_or(cmd)));
        }
    }

    /// THE entry point: the tray double-click and the test hook both come here.
    /// The palette may have changed (Windows' app theme): switched now; the open menu repaints (glass, shadow, colours) on its
    /// next frame (behind another app too: it is still on the screen).
    fn theme_changed(&mut self) {
        ui::sync_theme();
        if let Some(m) = self.menu.as_mut() {
            m.ui.dirty = true;
        }
    }

    fn command(&mut self, cmd: &str) {
        let (k, arg) = cmd.split_once(':').unwrap_or((cmd, ""));
        match k {
            "open" => self.open(),
            "close" => self.close(),
            "toggle" => {
                let open = self.menu.as_ref().map(|m| m.ui.close_t.is_none()).unwrap_or(false);
                if open {
                    self.close();
                } else {
                    self.open();
                }
            }
            "tab" => {
                if let (Some(m), Some(i)) = (&mut self.menu, ui::tab_index(arg)) {
                    let t = timing::now();
                    if i != m.ui.tab {
                        timing::mark("switch", t);
                    }
                    m.ui.show_tab(i, t);
                }
            }
            "quit" => self.quit = true,
            // test-only: what the mouse resting on the tray icon does
            "trayhover" => self.warm_up(),
            // test-only: what a key handler's `services::show_menu(id, target)` does (showmenu:<page id>[|<target>]) -
            // through the same request the main loop takes at its next wake-up
            "showmenu" => {
                let (id, target) = match arg.split_once('|') {
                    Some((i, t)) => (i, Some(t)),
                    None => (arg, None),
                };
                services::show_menu(id, target);
            }
            // ---- test-only (they drive the menu's own model; never the real mouse / keyboard / screen)
            "click" => {
                if let Some(m) = &mut self.menu {
                    if let Some((x, y)) = m.ui.target_point(arg) {
                        let t = timing::now();
                        m.ui.mouse_move(x, y, t);
                        let _ = m.ui.mouse_down(x, y, t);
                        if let ui::Action::Close = m.ui.mouse_up(x, y, t) {
                            m.ui.close(t);
                        }
                    }
                }
            }
            // test-only: the right button at a target (Ev::Context)
            "rclick" => {
                if let Some(m) = &mut self.menu {
                    if let Some((x, y)) = m.ui.target_point(arg) {
                        let t = timing::now();
                        m.ui.mouse_move(x, y, t);
                        m.ui.context(x, y, t);
                    }
                }
            }
            // test-only: drop:<target>|<path>[|<path>...] = files dropped on a target (Ev::Drop)
            "drop" => {
                if let Some(m) = &mut self.menu {
                    let mut it = arg.split('|');
                    if let Some((x, y)) = it.next().and_then(|t| m.ui.target_point(t)) {
                        m.ui.drop_files(x, y, it.map(|s| s.to_string()).collect(), timing::now());
                    }
                }
            }
            // test-only: key:<virtual-key code> = a key down in the menu (focused element, else the page)
            "key" => {
                if let (Some(m), Ok(vk)) = (&mut self.menu, arg.parse::<u16>()) {
                    if let (_, ui::Action::Close) = m.ui.key(vk, timing::now()) {
                        m.ui.close(timing::now());
                    }
                }
            }
            // test-only: scroll:<px> = the page's scroll offset at once (proofs below the first screen)
            "scroll" => {
                if let (Some(m), Ok(y)) = (&mut self.menu, arg.parse::<f32>()) {
                    m.ui.scroll_now(y, timing::now());
                }
            }
            // test-only: after:<ms>|<command> = that command <ms> from now (a test copy drives itself: the frame-rate proof
            // runs with no hook, no second start)
            "after" => {
                if let Some((ms, c)) = arg.split_once('|').and_then(|(ms, c)| ms.parse::<f64>().ok().map(|ms| (ms, c))) {
                    SCHED.with(|s| s.borrow_mut().push((timing::now() + ms, c.to_string())));
                }
            }
            // test-only: the frame painted incrementally and from scratch at the same moment (Menu::verify_incremental)
            "verify" => {
                if let Some(m) = &mut self.menu {
                    if let Ok(((n, mx, first), (n2, mx2, first2))) = m.verify_incremental(timing::now()) {
                        timing::note(&format!("verify {} diff {} max {} first {:?} | vs whole-page raster: diff {} max {} first {:?}", arg, n, mx, first, n2, mx2, first2));
                    }
                }
            }
            // test-only (Order 051): which path draws the menu (timing log), and a lost GPU device (ID3D12Device5::RemoveDevice)
            "gpustate" => {
                let st = self.menu.as_ref().map(|m| m.gpu_state()).unwrap_or_else(|| format!("menu closed device_alive={}", gpu::alive()));
                let (hit, compiled) = shadercache::STATS.with(|s| s.get());
                // (Order 052 item 0: + what the worker got / compiled / still has)
                let (d, w) = (shadercache::DEFERRED.load(std::sync::atomic::Ordering::Relaxed), shadercache::WORKER_COMPILED.load(std::sync::atomic::Ordering::Relaxed));
                timing::note(&format!("gpustate {} {} shaders_cached {} shaders_compiled {} deferred {} worker_compiled {} pending {}", arg, st, hit, compiled, d, w, shadercache::pending()));
            }
            // test-only (Order 051): the capture overlay over a made-up picture - ovtest:<x>,<y>,<w>,<h> (one made-up monitor),
            // ovdrag:<x0>,<y0>,<x1>,<y1>,<ms>[,<hz>] (a box dragged with posted mouse messages), ovstate, ovlose, ovclose
            "ovtest" => {
                let v: Vec<i32> = arg.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                let (x, y, w, h) = if v.len() == 4 { (v[0], v[1], v[2].max(1) as u32, v[3].max(1) as u32) } else { (-20000, 0, 1920, 1080) };
                let r = pages::screenshots::overlay::window::start_test(x, y, w, h);
                timing::note(&format!("ovtest {:?}", r));
            }
            "ovdrag" => {
                let v: Vec<i32> = arg.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if let (Some((hwnd, _)), true) = (pages::screenshots::overlay::window::test_window(), v.len() >= 5) {
                    let hz = v.get(5).copied().unwrap_or(1000).max(1) as u32;
                    pages::screenshots::overlay::gputest::drag(hwnd, (v[0], v[1]), (v[2], v[3]), v[4].max(1) as u32, hz);
                }
            }
            "ovstate" => {
                let st = pages::screenshots::overlay::window::test_window().map(|(_, g)| if g { "gpu" } else { "cpu" });
                let (hit, compiled) = shadercache::STATS.with(|s| s.get());
                timing::note(&format!("ovstate {} window={:?} device_alive={} shaders_cached {} shaders_compiled {}", arg, st, gpu::alive(), hit, compiled));
            }
            // test-only (Order 054): ovclick:<n> | x | copy | save = a click posted on a button of the overlay (see test_click)
            "ovclick" => pages::screenshots::overlay::window::test_click(arg),
            "ovlose" => pages::screenshots::overlay::window::test_lose_gpu(),
            "ovclose" => pages::screenshots::overlay::window::test_close(),
            "gpulose" => {
                if let Some(m) = &self.menu {
                    m.lose_gpu_for_test();
                    m_dirty(&mut self.menu);
                }
            }
            // test-only: wheel:<delta> = one mouse-wheel message over the page (120 = one notch up; the frame-rate proof)
            "wheel" => {
                if let (Some(m), Ok(d)) = (&mut self.menu, arg.parse::<f32>()) {
                    // over the middle of the page (not the top row)
                    m.ui.mouse_move(300.0, 300.0, timing::now());
                    m.ui.wheel(d, timing::now());
                }
            }
            "hover" => {
                if let Some(m) = &mut self.menu {
                    if let Some((x, y)) = m.ui.target_point(arg) {
                        m.ui.mouse_move(x, y, timing::now());
                    }
                }
            }
            "snap" => {
                if let Some(m) = &mut self.menu {
                    let _ = m.frame(timing::now());
                    if let Ok(p) = m.snapshot() {
                        let _ = png::save_png(&p, arg);
                    }
                }
            }
            "type" => {
                // type:<text> — characters into the % field being edited; "type:\r" = Enter, "type:\x1b" = Esc
                if let Some(m) = &mut self.menu {
                    let t = timing::now();
                    for ch in arg.chars() {
                        match ch {
                            '\r' | '\n' => {
                                let _ = m.ui.key(0x0D, t);
                            }
                            '\x1b' => {
                                let _ = m.ui.key(0x1B, t);
                            }
                            c => m.ui.char_input(c, t),
                        }
                    }
                }
            }
            "enter" => {
                if let Some(m) = &mut self.menu {
                    let _ = m.ui.key(0x0D, timing::now());
                }
            }
            "esc" => {
                if let Some(m) = &mut self.menu {
                    if let (_, ui::Action::Close) = m.ui.key(0x1B, timing::now()) {
                        m.ui.close(timing::now());
                    }
                }
            }
            "addon" => {
                // addon:<id>|<0|1> - the Add-ons page's switch (Order 035, test copies); its tab joins / leaves the open
                // menu's top row live (Order 037, `Ui::sync_addon_tabs`), as from the page's Get
                let (id, on) = arg.split_once('|').unwrap_or((arg, "1"));
                addons::set_for(id, on == "1", self.opts.test);
            }
            "addonacc" => {
                // addonacc:<absent|installed|restart> - test copies: Raw Accel's state on the fake OS (Order 037)
                addons::test_set_acc(arg);
            }
            "addonsim" => {
                // addonsim:acc|<none|dl:<share>|have> - test pictures: a frozen add-on state (the drawing's keys 1 / 2 / 3)
                let (_, st) = arg.split_once('|').unwrap_or(("acc", arg));
                addons::test_sim_acc(st);
            }
            "obssample" => {
                // test copies: Notifications for OBS with sample settings (scenes, a switch key) for pictures / checks;
                // obssample:other = NotificationsForOBS.exe pretends to run on its own
                if arg == "other" {
                    obs::set_other_for_test(Some(4242));
                }
                if let Some(mut s) = obs::settings() {
                    s.scenes = vec!["16:9".into(), "21:9".into()];
                    s.scenes_init = true;
                    s.switch_key = Some(bu_obs::keys::KeyBind::new(0x13, 0));
                    obs::set_settings(s);
                }
            }
            "winlight" => {
                // winlight:<0|1> - Windows' app theme as if it had just changed (Match Windows, Order 033; the real one is never touched)
                ui::set_windows_light(arg == "1");
                self.theme_changed();
            }
            "snapover" => {
                // snapover:<desk.png>|<out.png> — the comparison picture (see Menu::snapshot_over)
                if let (Some(m), Some((desk, out))) = (&mut self.menu, arg.split_once('|')) {
                    let _ = m.frame(timing::now());
                    if let Ok(p) = m.snapshot_over(desk) {
                        let _ = png::save_png(&p, out);
                    }
                }
            }
            "snapscreen" => {
                // snapscreen:<desk.png>|<out.png> — the on-screen pieces stacked over a desktop picture (Order 013)
                if let (Some(m), Some((desk, out))) = (&mut self.menu, arg.split_once('|')) {
                    let _ = m.frame(timing::now());
                    if let Ok(p) = m.snapshot_screen(desk) {
                        let _ = png::save_png(&p, out);
                    }
                }
            }
            "capstats" => {
                // capstats:<out.txt> — the desktop capture's counters (BU_GLASS=own): frames delivered, copies, unchanged copies, pictures
                if let Some((f, c, sm, q, ha, hm)) = self.menu.as_ref().and_then(|m| m.capture_stats()) {
                    let _ = std::fs::write(arg, format!("frames={} copies={} unchanged={} pictures={} gpu_copy_present_ms_avg={:.2} max={:.2}
", f, c, sm, q, ha, hm));
                }
            }
            "gpudiff" => {
                // gpudiff:<desk.png>|<out.png> — option 3's blur (Direct2D's effect chain) vs Skia's recipe on the same
                // desktop picture: <out.png> = Skia | Direct2D | difference, <out.png>.txt = the counts
                if let (Some(m), Some((desk, out))) = (&mut self.menu, arg.split_once('|')) {
                    match m.gpu_diff(desk, out) {
                        Ok(t) => {
                            let _ = std::fs::write(format!("{}.txt", out), t);
                        }
                        Err(e) => {
                            let _ = std::fs::write(format!("{}.txt", out), format!("error {:08x} {}", e.code().0, e.message()));
                        }
                    }
                }
            }
            "snapmask" => {
                // snapmask:<out.png> — the window's edge mask (alpha = Skia's coverage of the rounded shape)
                if let Some(m) = &self.menu {
                    let _ = png::save_png(&m.mask_pixels(), arg);
                }
            }
            // test-only (Order 052 item 0, tools/gpucache/make.ps1): the open and the close motion drawn at every 1 ms step - the
            // content's blur changes size every frame and Skia makes one shader per blur size, so the pack gets them all
            "blursweep" => {
                if let Some(m) = &mut self.menu {
                    let t0 = timing::now();
                    m.ui.open(t0);
                    for k in 0..=900 {
                        let _ = m.frame(t0 + k as f64);
                    }
                    let t1 = t0 + 1000.0;
                    m.ui.close(t1);
                    for k in 0..=200 {
                        let _ = m.frame(t1 + k as f64);
                    }
                    m.ui.open(timing::now());
                    timing::note("blursweep done");
                }
            }
            "switchshots" => {
                // switchshots:<tab>|<dir> — the page switch drawn at fixed moments (0, 20, 40 … 320 ms after the click),
                // each saved as <dir>\<ms>.png: pictures of the animation without the screen
                if let (Some(m), Some((tab, dir))) = (&mut self.menu, arg.split_once('|')) {
                    if let Some(i) = ui::tab_index(tab) {
                        let t = timing::now();
                        m.ui.show_tab(i, t);
                        for k in (0..=320).step_by(20) {
                            let _ = m.frame(t + k as f64);
                            if let Ok(p) = m.snapshot() {
                                let _ = png::save_png(&p, &format!("{}\\{:03}.png", dir, k));
                            }
                        }
                    }
                }
            }
            "where" => {
                // where:<target>|<file> — the target's centre in window pixels (tests post window messages there)
                if let (Some(m), Some((name, out))) = (&self.menu, arg.split_once('|')) {
                    if let Some((x, y)) = m.ui.target_point(name) {
                        let _ = std::fs::write(out, format!("{} {}", (x * m.scale).round() as i32, (y * m.scale).round() as i32));
                    }
                }
            }
            // test-only: the pieces gallery painted off-screen (ui/gallery.rs) -> gallery:<png>[|<scale>]
            "gallery" => {
                let mut it = arg.split('|');
                let out = it.next().unwrap_or_default();
                let sc = it.next().and_then(|s| s.parse().ok()).unwrap_or(1.0);
                let scene = it.next().and_then(|s| s.parse().ok()).unwrap_or(1);
                if let Some(p) = ui::gallery::render(sc, scene) {
                    let _ = png::save_png(&p, out);
                }
            }
            "state" => {
                // (a test reads every background part: the ones still waiting start now)
                while !self.bg_todo.is_empty() {
                    self.start_next_bg();
                }
                let mut s = match &self.menu {
                    // (Order 047: + the frames presented since the menu opened - the idle measurement counts them)
                    Some(m) => format!("{}\nframes {}", m.ui.describe(), m.frames),
                    None => "closed".to_string(),
                };
                // the pages' background parts (Page::background), one line each
                for (id, b) in &self.bg {
                    s.push_str(&format!("\nbg {}: {}", id, b.describe()));
                }
                // what a page put on the (test copy's stand-in) clipboard: `Cx::copy_text`
                if let Some(c) = clipboard::test_clip() {
                    s.push_str(&format!("\nclip {}", c.replace('\n', "\\n")));
                }
                let _ = std::fs::write(arg, s);
            }
            _ => {}
        }
    }

    fn handle(&mut self, e: Ev) {
        let now = timing::now();
        match e {
            Ev::Cmd(c, magic) => self.command_from(&c, Some(magic)),
            Ev::TrayDbl => {
                // open but behind another app (feedback F1: it no longer closes on a click outside): the double-click
                // brings it to the front instead of closing it
                if self.behind && self.menu.as_ref().is_some_and(|m| m.ui.close_t.is_none()) {
                    self.raise();
                } else {
                    self.command("toggle");
                }
            }
            Ev::TrayHover => self.warm_up(),
            Ev::WarmTimeout => {
                if self.menu.is_none() {
                    self.cool_down();
                }
            }
            Ev::TrayDown => {
                // a click on the tray icon deactivated the menu but must not close it: take the focus back (not while it
                // sits behind another app - then the double-click raises it)
                if let Some(m) = &self.menu {
                    if m.ui.close_t.is_none() && !self.behind && !self.opts.offscreen && !self.opts.screen_test {
                        unsafe {
                            let _ = SetForegroundWindow(m.hwnd);
                        }
                    }
                }
            }
            Ev::TrayMenu(x, y) => match self.tray.context_menu(x, y) {
                tray::IDM_OPEN => self.command("open"),
                tray::IDM_QUIT => self.command("quit"),
                _ => {}
            },
            Ev::Capture => {
                if let Some(m) = &mut self.menu {
                    m.backdrop_dirty = true;
                }
            }
            Ev::TaskbarCreated => self.tray.add(),
            Ev::Theme => {
                // Order 050: Windows broadcasts WM_SETTINGCHANGE for every change (the app's own too) - only a real theme /
                // DPI change does anything (the icon reloads, the menu repaints)
                self.tray.refresh();
                // Match Windows follows Windows' app theme live (Order 033): the open menu switches on its next frame
                let was = ui::windows_light();
                ui::read_windows_theme();
                if ui::windows_light() != was {
                    self.theme_changed();
                }
            }
            Ev::StartBg => self.start_next_bg(),
            Ev::Display => {
                let (work, _) = self.monitor();
                if let Some(m) = &mut self.menu {
                    m.re_anchor(work);
                }
            }
            Ev::Services => {
                // (behind another app too: it is still on the screen - Order 041)
                if let Some(m) = self.menu.as_mut() {
                    m.ui.dirty = true;
                }
            }
            Ev::Deactivate => {
                if self.opts.offscreen || self.opts.screen_test {
                    return;
                }
                // the owner Oct 8 (feedback F1): the menu closes ONLY on its X or minimize button. A click on another app
                // leaves it open; it just stops being on top, like any window, so the app clicked comes over it - the
                // tray double-click brings it back. (A click on the tray icon itself is not "another app".)
                let mut pt = POINT::default();
                unsafe {
                    let _ = GetCursorPos(&mut pt);
                }
                let on_icon = self.tray.rect().map(|r| pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom).unwrap_or(false);
                if let (Some(m), false) = (&mut self.menu, on_icon) {
                    if m.ui.close_t.is_none() {
                        self.behind = true;
                        m.set_topmost(false);
                        // behind another app it is still on the screen: timers, a stopwatch, progress bars and live pages
                        // keep moving (the owner Oct 8 test 2: "they should still be showing normally as long as the app is open
                        // not minimized"); only closing / minimizing (= closing) stops the frames
                        // a key field that listened stops: the keys must work in the app clicked (not for a mouse side
                        // button: it may be the very key being bound)
                        let side = unsafe { (GetAsyncKeyState(VK_XBUTTON1.0 as i32) as u16 & 0x8000) != 0 || (GetAsyncKeyState(VK_XBUTTON2.0 as i32) as u16 & 0x8000) != 0 };
                        if !side {
                            services::try_with(|s| s.stop_listening());
                        }
                    }
                }
            }
            Ev::Activate => {
                if let Some(m) = &mut self.menu {
                    self.behind = false;
                    m.set_topmost(true);
                    m.ui.dirty = true;
                }
            }
            ev => {
                let Some(m) = &mut self.menu else { return };
                if m.ui.close_t.is_some() {
                    return;
                }
                // a real-read test copy only looks: no clicks, keys or wheel reach the menu
                if self.opts.real_read && !matches!(ev, Ev::Move(..) | Ev::Leave) {
                    return;
                }
                let act = match ev {
                    Ev::Move(x, y) => {
                        m.ui.mouse_move(x, y, now);
                        ui::Action::None
                    }
                    Ev::Leave => {
                        m.ui.mouse_leave(now);
                        ui::Action::None
                    }
                    Ev::Down(x, y) => m.ui.mouse_down(x, y, now),
                    Ev::Up(x, y) => m.ui.mouse_up(x, y, now),
                    Ev::RUp(x, y) => {
                        m.ui.context(x, y, now);
                        ui::Action::None
                    }
                    Ev::Drop(x, y, paths) => {
                        m.ui.drop_files(x, y, paths, now);
                        ui::Action::None
                    }
                    Ev::DragOver(at) => {
                        m.ui.drag_over(at, now);
                        ui::Action::None
                    }
                    Ev::Wheel(d) => {
                        m.ui.wheel(d, now);
                        ui::Action::None
                    }
                    Ev::Key(vk) => m.ui.key(vk, now).1,
                    Ev::Char(c) => {
                        m.ui.char_input(c, now);
                        ui::Action::None
                    }
                    _ => ui::Action::None,
                };
                let hv = m.ui.hover;
                let cur = if hv.cursor_text() && !m.ui.is_editing() {
                    IDC_IBEAM
                } else if hv.cursor_hand() {
                    IDC_HAND
                } else {
                    IDC_ARROW
                };
                CURSOR.with(|c| *c.borrow_mut() = cur);
                if let ui::Action::Close = act {
                    m.ui.close(now);
                }
            }
        }
    }

    /// The next page's background part (the mute key's mic, the audio watcher, Activity's counter, Notifications for OBS):
    /// one per wake-up, BG_GAP_MS apart, so the tray answers from the first moment (Order 048 item 5).
    fn start_next_bg(&mut self) {
        // pages without one return at once: go on to the next that has one
        while let Some(p) = self.bg_todo.pop() {
            let t = timing::now();
            let b = p.background(&self.env);
            let ms = timing::now() - t;
            if ms >= 1.0 {
                timing::note(&format!("start bg {} {:.1} ms", p.id(), ms));
            }
            if let Some(b) = b {
                self.bg.push((p.id(), b));
            }
            if ms >= 1.0 {
                break;
            }
        }
        if !self.bg_todo.is_empty() {
            unsafe {
                SetTimer(Some(self.msg), TIMER_BG, BG_GAP_MS, None);
            }
        } else {
            timing::note(&format!("start bg all {}", self.bg.iter().map(|b| b.0).collect::<Vec<_>>().join(" ")));
        }
    }

    fn drain(&mut self) {
        WAKE_POSTED.with(|w| *w.borrow_mut() = false);
        loop {
            let e = EVENTS.with(|q| q.borrow_mut().pop_front());
            match e {
                Some((e, mods)) => {
                    ui::cx::Mods::set_event(Some(mods));
                    self.handle(e);
                    ui::cx::Mods::set_event(None);
                }
                None => break,
            }
        }
    }
}

fn run(opts: Opts) -> Result<()> {
    unsafe {
        let inst = GetModuleHandleW(None)?;
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: testmode::msg_class(), ..Default::default() });
        menu::register_classes(Some(wndproc))?;
        // a hidden top-level window (not message-only: it must hear "TaskbarCreated" after Explorer restarts)
        let msg = CreateWindowExW(WS_EX_TOOLWINDOW, testmode::msg_class(), w!("Boyler Utilities"), WS_POPUP, 0, 0, 0, 0, None, None, Some(inst.into()), None)?;
        TASKBAR_CREATED.with(|t| *t.borrow_mut() = RegisterWindowMessageW(w!("TaskbarCreated")));
        MSG_HWND.with(|h| *h.borrow_mut() = msg);
        OFFSCREEN.with(|o| *o.borrow_mut() = opts.offscreen);
        // the long-lived services: settings, keys, jobs (a test copy: scratch settings + the fake keys layer)
        services::init(msg, opts.test);
        // Settings › Theme: Windows' app theme (Match Windows) and the palette from the first paint on (Order 033)
        ui::read_windows_theme();
        ui::sync_theme();
        let tray = tray::Tray::new(msg, !opts.no_tray);
        // the very first start (Setup starts the app when it finishes): a bubble above the tray icon says where it is
        // (Order 041; never in a test copy)
        if !opts.test && !opts.no_tray {
            welcome::first_run(tray.rect());
        }
        // the app is up (window + tray): a restart after a self-update tells its install step, which then deletes the
        // old version (without it the install step rolls back) - Order 017
        if !opts.test {
            let args: Vec<String> = std::env::args().collect();
            if let Some(u) = bu_updater::confirm_started(&args) {
                let _ = UPDATED_FROM.set(u.previous_version);
            }
            // (FEATURES.md "Updates": the other two start calls) what a failed update wrote, and what an interrupted one
            // left: half downloads / staged / old copies next to the exe, the install step's %TEMP% folders. Normal copy
            // only - a test copy never reads or deletes anything of a real update.
            if let Ok(exe) = std::env::current_exe() {
                let cfg = bu_updater::UpdaterConfig::new("", env!("CARGO_PKG_VERSION"), exe);
                if let Some(r) = bu_updater::take_last_result(&cfg) {
                    let _ = LAST_UPDATE_FAILED.set(r);
                }
                bu_updater::cleanup_leftovers(&cfg, &args);
            }
        }
        timing::note(&format!("start test={} real_read={} offscreen={} tray={}", opts.test, opts.real_read, opts.offscreen, !opts.no_tray));
        // the pages' background parts, once for the app's life (the menu may never open) - Order 048: not here any more,
        // but one at a time once the loop runs (Ev::StartBg), in the pages' order (Audio's mute key first)
        let env = pages::Env { test: opts.test, real_read: opts.real_read, frozen: opts.frozen, rm: reduced_motion(), keep: keep::app() };
        let mut bg_todo = pages::all();
        bg_todo.reverse();
        let mut app = App { msg, tray, menu: None, opts: opts.clone(), dq: None, quit: false, bg: Vec::new(), open_on: None, behind: false, bg_todo, env };
        push(Ev::StartBg);
        for c in &opts.cmds {
            app.command_from(c, None);
        }
        let mut msgbuf = MSG::default();
        loop {
            // pick how to wait: frames while the menu moves, otherwise only messages (zero CPU)
            let want = app.menu.as_ref().map(|m| m.ui.dirty || m.backdrop_dirty || m.ui.animating(timing::now())).unwrap_or(false);
            // Order 051: the capture overlay's GPU windows that wait for their monitor's next refresh
            let ov = pages::screenshots::overlay::window::frame_waitables();
            if want || !ov.is_empty() {
                // wait for room in the swap chain (its waitable) or a message; a fired waitable is kept in `slot` until a
                // frame uses it, so a wake-up that draws nothing never loses it (lost, it froze the menu after a page switch)
                let menu_slot = app.menu.as_ref().map(|m| m.slot).unwrap_or(false);
                let menu_h = want && !menu_slot;
                let mut h = Vec::with_capacity(1 + ov.len());
                if menu_h {
                    h.push(app.menu.as_ref().unwrap().waitable());
                }
                h.extend_from_slice(&ov);
                let r = if want && menu_slot {
                    MsgWaitForMultipleObjectsEx(if ov.is_empty() { None } else { Some(&ov) }, 0, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
                } else {
                    MsgWaitForMultipleObjectsEx(Some(&h), 100, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
                };
                let timed_out = (!want || !menu_slot) && r == WAIT_TIMEOUT;
                let fired = (r.0.wrapping_sub(WAIT_OBJECT_0.0)) as usize;
                // (with the menu's slot already set, the wait was on `ov` alone)
                let hs: &[HANDLE] = if want && menu_slot { &ov } else { &h };
                if fired < hs.len() {
                    if menu_h && fired == 0 {
                        app.menu.as_mut().unwrap().slot = true;
                    } else {
                        pages::screenshots::overlay::window::on_waitable(Some(hs[fired]));
                    }
                }
                if !ov.is_empty() {
                    // a window whose refresh stayed away for 100 ms (a hidden monitor, a lost device): painted anyway
                    pages::screenshots::overlay::window::overdue();
                }
                // the same for the menu while the overlay's handles keep the wait busy
                let timed_out = timed_out || (want && !menu_slot && app.menu.as_ref().is_some_and(|m| timing::now() - m.last_frame > 100.0));
                while PeekMessageW(&mut msgbuf, None, 0, 0, PM_REMOVE).as_bool() {
                    if msgbuf.message == WM_QUIT {
                        app.quit = true;
                    }
                    let _ = TranslateMessage(&msgbuf);
                    DispatchMessageW(&msgbuf);
                }
                app.drain();
                // Order 055: the refresh after a frame that showed nothing (no present, so the swap chain stays silent)
                if menu::take_vblank() {
                    if let Some(m) = &mut app.menu {
                        m.slot = true;
                    }
                }
                if let Some(m) = &mut app.menu {
                    let now = timing::now();
                    let need =m.ui.switching(now) || m.ui.close_t.is_some() || m.ui.dirty || m.ui.animating(now);
                    // timed out: the compositor took no frame for 100 ms (window hidden) - draw anyway so nothing freezes
                    if want && need && (m.slot || timed_out) {
                        m.slot = false;
                        let _ = m.frame(now);
                        if timed_out {
                            timing::note_if_frames("frame_after_timeout");
                        }
                    }
                    if !m.ui.switching(timing::now()) {
                        timing::frames_off();
                    }
                }
            } else if let Some((due, menu_due)) = {
                // Order 047: nothing moves - sleep until the open menu's next timed step (a page's poll, a caret blink, a
                // toast's end) or a test copy's own timed command (`after:`), whichever comes first; a message wakes sooner
                let now = timing::now();
                let menu = app.menu.as_ref().and_then(|m| m.ui.wake_at(now));
                let sched = SCHED.with(|s| s.borrow().iter().map(|e| e.0).reduce(f64::min));
                let due = match (menu, sched) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                due.map(|d| (d, menu))
            } {
                let ms = (due - timing::now()).clamp(0.0, 1000.0).ceil() as u32;
                let _ = MsgWaitForMultipleObjectsEx(None, ms, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                let now = timing::now();
                // the menu's own step came due (its time was asked BEFORE the sleep: a page answers "now + 16 ms", so asking
                // again now would always be in the future and the page would never be polled - Opus review HIGH 1)
                if let Some(m) = app.menu.as_mut().filter(|_| menu_due.is_some_and(|t| t <= now + 0.5)) {
                    m.ui.wake(now);
                }
                while PeekMessageW(&mut msgbuf, None, 0, 0, PM_REMOVE).as_bool() {
                    if msgbuf.message == WM_QUIT {
                        app.quit = true;
                    }
                    let _ = TranslateMessage(&msgbuf);
                    DispatchMessageW(&msgbuf);
                }
                app.drain();
            } else {
                let ok = GetMessageW(&mut msgbuf, None, 0, 0);
                if !ok.as_bool() {
                    break;
                }
                let _ = TranslateMessage(&msgbuf);
                DispatchMessageW(&msgbuf);
                app.drain();
            }
            // a test copy's timed commands that are due
            let now = timing::now();
            let due: Vec<String> = SCHED.with(|s| {
                let mut s = s.borrow_mut();
                let (d, keep): (Vec<_>, Vec<_>) = s.drain(..).partition(|e| e.0 <= now);
                *s = keep;
                d.into_iter().map(|e| e.1).collect()
            });
            for c in due {
                app.command(&c);
            }
            if let Some(m) = &app.menu {
                if m.ui.closed(timing::now()) {
                    app.destroy_menu();
                }
            }
            // Order 047: Audio's switches put back by a Reset on its worker are saved here (menu open or not)
            pages::audio::drain_pending_rules();
            // changes noted from other threads (`undo::note`) go into the change log
            if undo::pending() {
                services::with(|s| undo::flush(&mut s.store));
            }
            // a key handler (or a page's thread) asked for the menu on a tab (`services::show_menu`)
            if let Some((id, target)) = services::take_show_menu() {
                app.show_menu(&id, target.as_deref());
            }
            // a page asked the app to end for the self-update's install step (`Cx::exit_for_update`, from its build or
            // event; the request posts the wake-up that gets here, menu open or not)
            if services::exit_requested() {
                app.quit = true;
            }
            if app.quit {
                break;
            }
        }
        app.destroy_menu();
        app.tray.remove();
        // Order 047 (Opus review): the pages' last work runs on their own threads now (Audio's close, a take-over): wait for
        // it (2 s at most), then their change-log notes go in before the store closes
        offui::wait_idle(2000);
        pages::audio::drain_pending_rules();
        if undo::pending() {
            services::with(|s| undo::flush(&mut s.store));
        }
        timing::flush();
        let _ = DestroyWindow(app.msg);
    }
    Ok(())
}

/// The open menu paints its next frame.
fn m_dirty(m: &mut Option<menu::Menu>) {
    if let Some(m) = m {
        m.ui.dirty = true;
    }
}

fn main() {
    // FIRST: started as the self-update's install step (Order 017) -> it swaps the files and ends; nothing opens
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = bu_updater::run_helper_if_requested(&args) {
        std::process::exit(code);
    }
    // started with the admin prompt as the add-ons' helper (Order 037): Raw Accel's own installer / uninstaller, checked
    // and run hidden; nothing opens
    if let Some(code) = bu_addons::helper::run_if_requested(&args) {
        std::process::exit(code);
    }
    // started with the admin prompt to set up Search's own Everything (Order 049): voidtools' file checked, our manual
    // service, v1.0.0's Everything tidied up when asked; nothing opens
    if let Some(code) = bu_search::real::ours::run_if_requested(&args) {
        std::process::exit(code as i32);
    }
    // Order 049: keep the day Boyler Utilities was FIRST installed (Setup rewrites its own date each time) - the rule that
    // tells v1.0.0's Everything from a user's own (A_049_02)
    bu_search::real::ours::remember_first_day();
    // Setup's "Everything for Search" task: set up our own Everything (one admin prompt; v1.0.0's is tidied up). No window.
    // the uninstaller: our Everything (copy + service) goes with the app (one admin prompt when it is there). No window.
    if args.iter().any(|a| a == bu_search::real::ours::REMOVE_ARG) {
        std::process::exit(match bu_search::real::ours::uninstall() {
            Ok(()) => 0,
            Err(bu_search::SearchError::InstallCancelled) => 1602,
            Err(_) => 1,
        });
    }
    if args.iter().any(|a| a == bu_search::real::ours::SETUP_ARG) {
        let tidy = bu_search::real::ours::v100_present();
        std::process::exit(match bu_search::real::ours::install(tidy) {
            Ok(()) => 0,
            Err(bu_search::SearchError::InstallCancelled) => 1602,
            Err(_) => 1,
        });
    }
    // started with the admin prompt for ONE admin action (Order 039): only the fixed ops of admin/, checked; nothing opens
    if let Some(code) = admin::helper::run_if_requested(&args) {
        std::process::exit(code);
    }
    // the uninstaller's "Undo my Windows changes too?" (Order 036): no window, no tray
    let undo_apply = args.iter().any(|a| a == UNDO_ARG);
    if undo_apply || args.iter().any(|a| a == UNDO_COUNT_ARG) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        std::process::exit(undo_windows(undo_apply, args.iter().any(|a| a == "--test")));
    }
    let opts = parse_args();
    testmode::set(opts.test, opts.real_read);
    testmode::set_demo(opts.demo);
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let explicit_test = std::env::args().any(|a| a == "--test");
    if let Some(dir) = opts.lab.as_ref().filter(|_| explicit_test) {
        lab::run(dir);
        return;
    }
    if let Some(dir) = opts.make_icons.as_ref().filter(|_| explicit_test) {
        let _ = appicon::make_icons(dir);
        return;
    }
    if opts.lab.is_some() || opts.make_icons.is_some() {
        return;
    }
    // single instance, one per mode: a second start hands its commands to the running copy of its own mode and leaves
    let mutex = unsafe { CreateMutexW(None, false, testmode::mutex_name()) };
    let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if already {
        // a self-driving test copy (`--run`) never hands its commands to another lane's running test copy
        if opts.run {
            return;
        }
        let cmds = if opts.cmds.is_empty() { vec!["open".to_string()] } else { opts.cmds.clone() };
        // the first copy may still be starting: retry for a moment
        for _ in 0..50 {
            if send_to_running(&cmds) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        return;
    }
    // a hook command with no copy running: never start a fresh copy (it would open its own menu, maybe on screen) -
    // except an off-screen test copy told to drive itself (`--test --offscreen --run --cmd ...`, the frame-rate proof)
    if !opts.cmds.is_empty() && !(explicit_test && opts.offscreen && opts.run) {
        return;
    }
    if let Some(p) = &opts.log {
        timing::open_log(p);
    }
    let _ = run(opts);
    services::shutdown();
    wait_broadcasts();
    drop(mutex);
}

/// `--undo-windows`: put back every change the app made to Windows (all of Settings › "Back to how your PC was", all
/// ticked), no window. Exit code = the lines that failed (0 = all back), 251 = the app is running (the uninstaller closes it
/// first). `--undo-windows-count`: only count them - exit code = the number of changes (0 = nothing to offer).
const UNDO_ARG: &str = "--undo-windows";
const UNDO_COUNT_ARG: &str = "--undo-windows-count";
const UNDO_APP_RUNNING: i32 = 251;

fn undo_windows(apply: bool, test: bool) -> i32 {
    // a test start (`--test`) = fake pages + scratch settings, like every test copy
    testmode::set(test, false);
    // never while the app runs (it holds its own copy of the settings and the pages' state); holding the mutex also keeps
    // the app from starting meanwhile
    let _mutex = unsafe { CreateMutexW(None, false, testmode::mutex_name()) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return UNDO_APP_RUNNING;
    }
    undo::set_headless();
    services::init_headless(test);
    // what a normal start reads in `Page::background` (not run here: no watchers, no keys)
    pages::audio::load_for_undo();
    let mut all = pages::all();
    let (n, failed) = undo::undo_everything(&mut all, apply);
    wait_broadcasts();
    drop(all);
    services::shutdown();
    if apply { failed.min(250) as i32 } else { n.min(250) as i32 }
}

#[cfg(test)]
mod tests {
    use super::closes_on_focus_loss;

    #[test]
    fn focus_loss_closes_the_menu_unless_own_window_or_modal() {
        // another app (WM_ACTIVATE to its window, or WM_ACTIVATEAPP from our overlay to it) -> closes
        assert!(closes_on_focus_loss(4242, 100, false));
        // unknown target (NULL window / thread gone) -> closes, the safe default
        assert!(closes_on_focus_loss(0, 100, false));
        // one of the app's own windows (capture overlay, picker) -> stays open
        assert!(!closes_on_focus_loss(100, 100, false));
        // Windows' modal UI over the menu -> stays open, whoever takes the focus
        assert!(!closes_on_focus_loss(4242, 100, true));
        assert!(!closes_on_focus_loss(0, 100, true));
    }
}

/// Order 050: the WM_SETTINGCHANGE broadcasts of the last changes go out on workers - before the app ends, wait for them
/// (1 s at most) so other apps still hear of every change.
fn wait_broadcasts() {
    let max = std::time::Duration::from_secs(1);
    bu_toggles::real::wait_broadcasts(max);
    bu_mouse::win::wait_broadcasts(max);
}
