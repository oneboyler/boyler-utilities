//! The menu's frame (Order 014, drawing menu-v22): the top row (17 tabs in ONE row that scrolls sideways - the drawing's
//! default top-row option 7 - with its soft edge fades, round glass chevrons, the shared name label and the accent dot),
//! the Windows caption buttons, the page area with the page switch animation and the glass scrollbar, and the popups of
//! the pages. The pages themselves are modules of `crate::pages` (the page API); their boxes are laid out and painted by
//! `ui::lay` from `ui::el` trees, their hover / press / transitions live in `ui::cx`, the shared pieces in `ui::pieces`.
//! All coordinates are window DIPs (CSS px) with the window's top-left at 0,0.

pub mod cx;
pub mod damage;
pub mod dragout;
pub mod el;
pub mod gallery;
pub mod lay;
pub mod picker;
pub mod pieces;

use std::collections::HashMap;

use skia_safe as sk;
use windows_numerics::Matrix3x2;

use crate::anim::{self, Tween};
use crate::audio::Audio;
use crate::gfx::{sh, Align, CssColor, Font, Gfx, Rgba};
use crate::icons::Icons;
use crate::pages::{self, Env, Page};
use crate::settings::GlassStyle;
use cx::{Cx, Ev, State};
use el::{Cursor, El, Key};
use lay::Laid;

pub const WIN_W: f32 = 600.0;
pub const WIN_H: f32 = 520.0;
pub const RADIUS: f32 = 14.0;
/// the page area: `.right` margin-top 52 + padding-top 4
pub const PAGE_TOP: f32 = 56.0;
pub const PAGE_H: f32 = WIN_H - PAGE_TOP;

// ------------------------------------------------------------------ colour tokens (#sw, menu-v22): the theme's palette
// Order 033: every token reads the current theme. Dark = the dark glass exactly as before; light = the drawing's
// `#sw.light{...}` values.
pub const fn c(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    Rgba::rgba(r, g, b, a)
}

/// One theme's colour tokens (the drawing's CSS variables).
pub struct Pal {
    pub acc: Rgba,
    pub fg: Rgba,
    pub fg2: Rgba,
    pub fg3: Rgba,
    pub grp: Rgba,
    pub grp_rim: Rgba,
    pub grp_top: Rgba,
    pub hair: Rgba,
    pub ctl: Rgba,
    pub ctl_h: Rgba,
    pub trk: Rgba,
    pub pill: Rgba,
    pub sel: Rgba,
    pub menu: Rgba,
    pub pop: Rgba,
    pub hov: Rgba,
    pub red: Rgba,
    pub green: Rgba,
    pub amber: Rgba,
    pub key: Rgba,
    pub ico: Rgba,
    pub ico_h: Rgba,
    pub ico_on: Rgba,
    pub acc_glow: Rgba,
    pub acc_s: Rgba,
    pub vz1: Rgba,
    pub vz2: Rgba,
    pub lvt: Rgba,
    pub well: Rgba,
    pub dash: Rgba,
    pub hl: Rgba,
    pub hl_v19: Rgba,
}

/// The dark glass (#sw + v19), unchanged.
pub const DARK: Pal = Pal {
    acc: c(10, 132, 255, 1.0),
    fg: c(245, 245, 247, 1.0),
    fg2: c(235, 235, 245, 0.6),
    fg3: c(235, 235, 245, 0.36),
    // the option groups ("bubbles"), the owner's final glass (Oct 7): white .08 + the rim of glass-compare-v6 `.w3.bx .grp`
    grp: c(255, 255, 255, 0.08),
    grp_rim: c(255, 255, 255, 0.18),
    grp_top: c(255, 255, 255, 0.26),
    hair: c(255, 255, 255, 0.085),
    ctl: c(255, 255, 255, 0.1),
    ctl_h: c(255, 255, 255, 0.15),
    trk: c(120, 120, 128, 0.36),
    pill: c(255, 255, 255, 0.22),
    sel: c(10, 132, 255, 0.28),
    menu: c(42, 42, 48, 0.74),
    pop: c(36, 37, 43, 0.94),
    hov: c(255, 255, 255, 0.06),
    red: c(255, 69, 58, 1.0),
    green: c(48, 209, 88, 1.0),
    amber: c(255, 214, 10, 1.0),
    key: c(255, 255, 255, 0.16),
    ico: c(225, 227, 236, 0.66),
    ico_h: c(255, 255, 255, 1.0),
    ico_on: c(61, 155, 255, 1.0),
    acc_glow: c(10, 132, 255, 0.55),
    acc_s: c(10, 132, 255, 0.45),
    vz1: c(10, 132, 255, 1.0),
    vz2: c(47, 214, 196, 1.0),
    lvt: c(255, 255, 255, 0.07),
    well: c(0, 0, 0, 0.16),
    dash: c(255, 255, 255, 0.2),
    hl: c(255, 255, 255, 0.12),
    // v19: the dark glass's `--hl` (popups' inner ring, the name label)
    hl_v19: c(255, 255, 255, 0.22),
};

/// The light glass: `#sw.light{...}` (menu-v22). Its groups have no bright rim - `.grp{box-shadow:inset 0 0 0 .5px
/// var(--hair)}` - so grp_rim = --hair and grp_top is clear; --hl is .75 white for every glass style.
pub const LIGHT: Pal = Pal {
    acc: c(0, 122, 255, 1.0),
    fg: c(29, 29, 31, 1.0),
    fg2: c(60, 60, 67, 0.62),
    fg3: c(60, 60, 67, 0.4),
    grp: c(255, 255, 255, 0.62),
    grp_rim: c(60, 60, 67, 0.12),
    grp_top: c(255, 255, 255, 0.0),
    hair: c(60, 60, 67, 0.12),
    ctl: c(0, 0, 0, 0.055),
    ctl_h: c(0, 0, 0, 0.085),
    trk: c(120, 120, 128, 0.26),
    pill: c(255, 255, 255, 1.0),
    sel: c(0, 122, 255, 0.16),
    menu: c(250, 250, 252, 0.8),
    pop: c(250, 250, 252, 0.95),
    hov: c(0, 0, 0, 0.045),
    red: c(255, 59, 48, 1.0),
    green: c(52, 199, 89, 1.0),
    amber: c(255, 184, 0, 1.0),
    key: c(255, 255, 255, 1.0),
    ico: c(60, 60, 67, 0.6),
    ico_h: c(29, 29, 31, 1.0),
    ico_on: c(0, 122, 255, 1.0),
    acc_glow: c(0, 122, 255, 0.32),
    acc_s: c(0, 122, 255, 0.35),
    vz1: c(0, 122, 255, 1.0),
    vz2: c(0, 179, 161, 1.0),
    lvt: c(0, 0, 0, 0.07),
    well: c(0, 0, 0, 0.035),
    dash: c(60, 60, 67, 0.28),
    hl: c(255, 255, 255, 0.75),
    hl_v19: c(255, 255, 255, 0.75),
};

thread_local! {
    /// Order 047: the tab the next `Ui::new` opens first (`set_first_tab`)
    static FIRST_TAB: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// The next menu opens showing this tab (its page id) - the only page it opens (main.rs, before `Menu::new`).
pub fn set_first_tab(id: Option<String>) {
    FIRST_TAB.with(|f| *f.borrow_mut() = id);
}

#[cfg(not(test))]
static LIGHT_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
// unit tests run in parallel threads: each test thread has its own theme
#[cfg(test)]
thread_local! {
    static LIGHT_ON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Is the light glass showing? (set by the frame from Settings › Theme / Windows' app theme)
pub fn is_light() -> bool {
    #[cfg(not(test))]
    return LIGHT_ON.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(test)]
    return LIGHT_ON.with(|l| l.get());
}

/// Switch the palette (the frame repaints everything after it).
pub fn set_light(on: bool) {
    #[cfg(not(test))]
    LIGHT_ON.store(on, std::sync::atomic::Ordering::Relaxed);
    #[cfg(test)]
    LIGHT_ON.with(|l| l.set(on));
}

static WIN_LIGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// the test hook stood a value in (`winlight:`): a later WM_SETTINGCHANGE does not read the registry over it
static WIN_LIGHT_HOOK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Read Windows' app theme (`HKCU\...\Themes\Personalize\AppsUseLightTheme`, what "Match Windows" follows) - at start and on
/// every WM_SETTINGCHANGE. A test copy can stand a value in with `BU_WINLIGHT` / the hook's `winlight:<0|1>`.
pub fn read_windows_theme() {
    if WIN_LIGHT_HOOK.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let light = match crate::testmode::env("BU_WINLIGHT") {
        Some(v) => v == "1",
        None => crate::tray::apps_light(),
    };
    WIN_LIGHT.store(light, std::sync::atomic::Ordering::Relaxed);
}

/// Windows' app theme as last read (`read_windows_theme`).
pub fn windows_light() -> bool {
    WIN_LIGHT.load(std::sync::atomic::Ordering::Relaxed)
}

/// Test hook only: Windows' app theme as if it had just changed.
pub fn set_windows_light(on: bool) {
    WIN_LIGHT_HOOK.store(true, std::sync::atomic::Ordering::Relaxed);
    WIN_LIGHT.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// The palette the saved Theme choice wants now (Match Windows = Windows' app theme as last read), switched to. Returns
/// whether that is the light glass.
pub fn sync_theme() -> bool {
    let t = crate::services::with(|s| s.theme()).unwrap_or_default();
    let light = t.light(WIN_LIGHT.load(std::sync::atomic::Ordering::Relaxed));
    set_light(light);
    light
}

/// The current theme's palette.
pub fn pal() -> &'static Pal {
    if is_light() {
        &LIGHT
    } else {
        &DARK
    }
}

macro_rules! tokens {
    ($($name:ident $field:ident),* $(,)?) => {$(
        #[allow(non_snake_case)]
        #[inline]
        pub fn $name() -> Rgba {
            pal().$field
        }
    )*};
}
tokens!(ACC acc, FG fg, FG2 fg2, FG3 fg3, GRP grp, GRP_RIM grp_rim, GRP_TOP grp_top, HAIR hair, CTL ctl, CTL_H ctl_h, TRK trk,
    PILL pill, SEL sel, MENU menu, POP pop, HOV hov, RED red, GREEN green, AMBER amber, KEY key, ICO ico, ICO_H ico_h,
    ICO_ON ico_on, ACC_GLOW acc_glow, ACC_S acc_s, VZ1 vz1, VZ2 vz2, LVT lvt, WELL well, DASH dash, HL hl, HL_V19 hl_v19);

/// Plain white in both themes (text on the accent, the drawing's `#fff`).
pub const WHITE: Rgba = c(255, 255, 255, 1.0);
// the window glass, the owner's final numbers (Oct 7): rgba(20,20,26,.22) (blur / colour boost: see comp.rs)
pub const TINT: (u8, u8, u8, f32) = (20, 20, 26, 0.22);

/// CSS colour transitions mix in premultiplied space.
pub fn cmix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let al = a.3 + (b.3 - a.3) * t;
    if al <= 0.0 {
        return Rgba(0.0, 0.0, 0.0, 0.0);
    }
    let p = |x: f32, y: f32| (x * a.3 + (y * b.3 - x * a.3) * t) / al;
    Rgba(p(a.0, b.0), p(a.1, b.1), p(a.2, b.2), al)
}

// ------------------------------------------------------------------ fonts + line boxes
pub const F13: Font = Font::new(13.0, 400);
// text inside a <button> of the drawing: `font:inherit` but the browser's own `letter-spacing: normal`
pub const F13BTN: Font = Font::new(13.0, 400).ls(0);
pub const F11: Font = Font::new(11.0, 400);
pub const F11M: Font = Font::new(11.0, 500);
pub const F125: Font = Font::new(12.5, 400).tnum();
pub const F20: Font = Font::display(20.0, 600);
pub const F115B: Font = Font::new(11.5, 600);
pub const F105M: Font = Font::new(10.5, 500);
pub const F12B: Font = Font::new(12.0, 600);
pub const FTAG: Font = Font::new(10.0, 600).ls(200);
pub fn lu(v: f32) -> f32 {
    (v * 64.0).floor() / 64.0
}
pub const LH13: f32 = 17.546875; // 13 x 1.35 in LayoutUnits
pub const LH11: f32 = 14.84375; // 11 x 1.35
pub const LH11F: f32 = 15.390625; // 11 x 1.4 (group footers)
pub const LH125: f32 = 16.875; // 12.5 x 1.35

// ------------------------------------------------------------------ the top row (menu-v22 option 7 = SCROLL)
/// `.dscroll`: left 10, right 94 -> 496 px of the 600 px window
const DS_X: f32 = 10.0;
const DS_W: f32 = WIN_W - 10.0 - 94.0;
/// `.dock` padding 0 6px, `.dt` 32 x 34, gap 3
const DT_W: f32 = 32.0;
const DT_GAP: f32 = 3.0;
const DOCK_PAD: f32 = 6.0;
const DOCK_Y: f32 = 9.0;
/// SCR_FADE: the edge fade width (and how far the active icon stays clear of it)
const SCR_FADE: f32 = 30.0;

/// An icon's left edge inside `.dock` (scroll 0).
fn dock_x(i: usize) -> f32 {
    DOCK_PAD + i as f32 * (DT_W + DT_GAP)
}

// ------------------------------------------------------------------ hit targets
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    None,
    Dock(usize),
    /// 0 = left chevron, 1 = right
    Chev(usize),
    CapMin,
    CapClose,
    SbUp,
    SbDown,
    SbThumb,
    SbTrack,
    /// an element of the page (its innermost key) and the cursor it asks for
    Page(Option<Key>, Cursor),
    /// an element of a page's popup
    Popup(Option<Key>, Cursor),
    /// the hand-painted Audio page: (hand, text) cursor
    Legacy(bool, bool),
}

impl Hit {
    pub fn cursor_hand(&self) -> bool {
        match self {
            Hit::Dock(_) | Hit::Chev(_) | Hit::SbUp | Hit::SbDown | Hit::SbTrack => true,
            // (Windows has no open / closed hand cursor of its own: grab / grabbing show the hand - a gap, Chromium draws its own)
            Hit::Page(_, c) | Hit::Popup(_, c) => matches!(c, Cursor::Hand | Cursor::Grab | Cursor::Grabbing),
            Hit::Legacy(h, _) => *h,
            _ => false,
        }
    }
    pub fn cursor_text(&self) -> bool {
        match self {
            Hit::Page(_, c) | Hit::Popup(_, c) => *c == Cursor::Text,
            Hit::Legacy(_, t) => *t,
            _ => false,
        }
    }
}

/// What a click on the menu asks the window to do.
pub enum Action {
    None,
    Close,
}

/// A hand-painted page (only Audio: test A's code, Orders 001 / 003) plugged into the frame.
pub trait LegacyPage {
    fn set_scroll(&mut self, sy: f32);
    fn content_height(&self) -> f32;
    fn update(&mut self, now: f64);
    /// anything but its live meters moving
    fn busy(&self, now: f64) -> bool;
    fn take_dirty(&mut self) -> bool;
    fn draw(&self, f: &Frame, stag: &dyn Fn(usize) -> (f32, f32));
    fn mouse_move(&mut self, x: f32, y: f32, now: f64);
    fn mouse_leave(&mut self, now: f64);
    fn mouse_down(&mut self, x: f32, y: f32, now: f64) -> Action;
    fn mouse_up(&mut self, x: f32, y: f32, now: f64) -> Action;
    fn key(&mut self, vk: u16, now: f64) -> (bool, Action);
    fn char_input(&mut self, ch: char, now: f64);
    fn is_editing(&self) -> bool;
    fn end_edit(&mut self, save: bool);
    /// (hand, text) cursor of what is under the mouse
    fn cursor(&self) -> (bool, bool);
    fn popup_open(&self) -> bool;
    fn draw_popup(&mut self, f: &Frame, base: &sk::Image);
    /// the page scrolled: close its popup
    fn on_scroll(&mut self, now: f64);
    fn target_point(&self, name: &str) -> Option<(f32, f32)>;
    fn set_style(&mut self, frozen: bool, caret_ms: f64, sel: Rgba);
    fn device_names(&self) -> Vec<String>;
}

pub struct Frame<'a> {
    pub g: &'a Gfx,
    pub icons: &'a Icons,
    pub now: f64,
}

fn tw(v: f64) -> Tween {
    Tween::new(v)
}

/// The page tree of one tab, laid out (page coordinates: the drawing's `.pg` box, top-left 0,0).
struct PageLayer {
    laid: Laid,
    built_at: f64,
    /// an event changed the page: build it again at the next paint - but keep these boxes until then, so a release
    /// that comes before that paint (a fast click, the hook's click) still finds what was pressed
    stale: bool,
    /// the page has sticky boxes, placed for this scroll offset
    sticky_at: Option<f32>,
}

pub struct Ui {
    pub rm: bool,
    pub tab: usize,
    switch: Option<(usize, f64, f32)>,
    pub open_t: f64,
    pub close_t: Option<f64>,
    pages: Vec<Box<dyn Page>>,
    env: Env,
    // ---- top row
    mag: Vec<Tween>,
    bounce: Vec<f64>,
    ico_h: Vec<Tween>,
    on_t: Vec<Tween>,
    press_s: Vec<Tween>,
    /// when an add-on's icon joined the row (its pop-in, Order 037)
    appear: Vec<f64>,
    /// `crate::addons::gen()` when the row was last matched to the add-ons
    addon_gen: u64,
    dot: Tween,
    /// the row's sideways scroll: target and smoothly followed position (the drawing's scrGo: 1 - e^(-dt/55))
    ds_t: f32,
    ds: f32,
    ds_last: f64,
    /// pointer x over the top row (chevrons show within 36 px of a faded edge)
    dock_px: Option<f32>,
    /// --fl / --fr (.22 s ease) and --cl / --cr (.2 s ease)
    fades: [Tween; 4],
    chev_on: [Tween; 2],
    chev_h: [Tween; 2],
    chev_p: [Tween; 2],
    /// the shared name label: which icon, since when hovered
    lbl_i: Option<usize>,
    lbl_since: f64,
    lbl_t: Tween,
    lbl_shown: usize,
    // ---- caption buttons
    cap_h: [Tween; 2],
    cap_p: [Tween; 2],
    // ---- page scroll + the glass scrollbar
    scroll: Vec<Tween>,
    sb_drag: Option<(f32, f32)>,
    sb_h: Tween,
    sb_on: Tween,
    sb_hold: Option<(f32, f64)>,
    // ---- page boxes
    st: State,
    layers: HashMap<usize, PageLayer>,
    popup: Option<Laid>,
    press_key: Option<(Key, bool)>,
    pub hover: Hit,
    press: Hit,
    mouse: (f32, f32),
    mouse_in: bool,
    pub dirty: bool,
    /// the page's live parts changed (`Page::tick` = true): rebuild its boxes for the live pass, no static repaint
    pub live_dirty: bool,
    /// the shown page's last `tick` said its live parts move (frames keep coming)
    ticking: bool,
    /// Order 041: what changed in the shown page's boxes since the frame last took it (`take_page_damage`)
    page_damage: Option<damage::Damage>,
    /// Order 041: each top-row icon's last raster (its look as a number, the picture) - `draw_dock` places it again
    dock_icons: HashMap<usize, (u64, sk::Image)>,
    /// Order 041: the shown page is painted this far down (DIPs, = a whole number of device pixels from the frame's
    /// scroll); None = PAGE_TOP - the scroll
    pub page_dy: Option<f32>,
    pub caret_ms: f64,
    pub sel_color: Rgba,
    pub frozen: bool,
    last_step: f64,
    /// the reset review a page opened (Req::Reset): the lines, where, which tab, when
    review: Option<ReviewState>,
    /// Order 047: the review's reads / resets running off the menu's thread (`review_poll`)
    review_bg: ReviewBg,
    /// the toast a page asked for (Req::Toast): text, since
    toast: Option<(String, f64)>,
    /// a page asked to scroll (Req::ScrollTo / ScrollY): done once its boxes are built again
    want_scroll: Option<WantScroll>,
    /// a page asked for another tab (Req::ShowTab): its id and the target for its `jump`
    want_tab: Option<(String, Option<String>)>,
    /// requests a page made outside `event` (in `build` / `popup` / `overlay`): applied at the next `update` (they are
    /// safe there) - REVIEW_014_item1c HOLD 1
    pending: Vec<cx::Req>,
    /// a dismissable popup is up (the page's `popup` or the reset review): a click beside it / Esc closes it
    modal: bool,
    /// page events the frame delivered (test hook `state`): clicks, context (right button), drops, page-level keys
    evs: [u32; 4],
    /// until this time the shown tab may hold the page switch / the open motion while it is not `ready` (its values not
    /// in yet): the old page stays, then the new one comes in with its real values
    hold_until: f64,
    /// how long the open motion's page part was held for a page that was not ready (ms, added to its timing)
    open_hold: f64,
    /// the shared tooltip (`update_tips`): its state, whether it needs frames, whether a bubble shows
    tips: pieces::tip::Tips,
    tips_busy: bool,
    tip_shown: bool,
    /// Order 045: the focus came from the keyboard (Tab): the focused control shows its ring (`:focus-visible`); a mouse
    /// press ends it
    focus_ring: bool,
    /// Order 045: the element files from Explorer are dragged over (`drag_over`)
    drag_over: Option<Key>,
    /// Order 047 item 10: the tab whose page was opened ahead because the pointer rests on its icon (a click shows it at
    /// once: its reads already ran) and when it is let go if not clicked
    pre: Option<usize>,
    pre_close_at: Option<f64>,
    /// Order 047 (measuring): a tab was clicked at this time and its page is not shown yet (`switch_shown`)
    switch_wait: Option<f64>,
    /// Order 047: the clicked tab's page is in this frame - (click time, its values were in = not the HOLD_MS give-up)
    pub switch_shown: Option<(f64, bool)>,
}

/// After a tab opens its transitions jump for this long (its real values arrive without a slide from the defaults).
const SETTLE_MS: f64 = 500.0;
/// The longest the frame waits for a tab that is not `ready` before showing it anyway.
const HOLD_MS: f64 = 400.0;
/// Order 047: a tab opened ahead on hover is closed this long after the pointer left the icons without a click
const PRE_KEEP_MS: f64 = 1500.0;
/// An add-on icon's pop-in (addons-v1: `cubic-bezier(.3,1.35,.5,1)`, 420 ms).
const APPEAR: crate::anim::Bezier = crate::anim::Bezier::new(0.3, 1.35, 0.5, 1.0);

enum WantScroll {
    Key(Key),
    Y(f32),
}

struct ReviewState {
    review: crate::undo::Review,
    x: f32,
    y: f32,
    tab: usize,
}

/// Order 047: the reset review's work on worker threads: a review being read (where it opens, on which tab's page) and
/// resets being put back (what was reset, how many lines: the toast's words).
#[derive(Default)]
struct ReviewBg {
    reading: Option<(crate::undo::ReviewJob, f32, f32, &'static str)>,
    applying: Vec<(crate::undo::ApplyJob, crate::undo::Kind, usize)>,
}

const K_REVIEW: Key = el::key("frame.review");
const K_TOAST: Key = el::key("frame.toast");

impl Ui {
    /// The menu opens on its first tab: the one `set_first_tab` named (the tab shown last / a key's tab), else Audio -
    /// opened like every tab (`Page::open` makes its service; `close` drops it).
    pub fn new(rm: bool, frozen: bool, now: f64) -> Ui {
        // an add-on's tab only while the add-on is on (Order 035)
        let pages: Vec<Box<dyn Page>> = pages::all().into_iter().filter(|p| crate::addons::tab_visible(p.id())).collect();
        let first = FIRST_TAB.with(|f| f.borrow_mut().take()).and_then(|id| pages.iter().position(|p| p.id() == id)).unwrap_or(0);
        Ui::with_pages_on(pages, first, rm, frozen, now)
    }

    fn with_pages(pages: Vec<Box<dyn Page>>, rm: bool, frozen: bool, now: f64) -> Ui {
        Ui::with_pages_on(pages, 0, rm, frozen, now)
    }

    fn with_pages_on(mut pages: Vec<Box<dyn Page>>, first: usize, rm: bool, frozen: bool, now: f64) -> Ui {
        let n = pages.len();
        let env = Env { test: crate::testmode::on(), real_read: crate::testmode::real_read(), frozen, rm, keep: crate::keep::app() };
        // the first tab, opened like every tab (it makes its own service; Order 014 item 1c for 018). Order 047: the tab
        // the menu shows first - it opened Audio every time and closed it again at once (its worker's first full read,
        // 100-400 ms, on every open of another tab)
        pages[first].open(&env, now);
        let mut ui = Ui {
            rm,
            tab: first,
            switch: None,
            open_t: now,
            close_t: None,
            pages,
            env,
            mag: vec![tw(1.0); n],
            bounce: vec![-1e9; n],
            ico_h: vec![tw(0.0); n],
            on_t: vec![tw(0.0); n],
            press_s: vec![tw(1.0); n],
            appear: vec![-1e9; n],
            addon_gen: crate::addons::gen(),
            dot: tw((dock_x(first) + DT_W / 2.0) as f64),
            ds_t: 0.0,
            ds: 0.0,
            ds_last: now,
            dock_px: None,
            fades: [tw(0.0), tw(0.0), tw(0.0), tw(0.0)],
            chev_on: [tw(0.0); 2],
            chev_h: [tw(0.0); 2],
            chev_p: [tw(0.0); 2],
            lbl_i: None,
            lbl_since: 0.0,
            lbl_t: tw(0.0),
            lbl_shown: 0,
            cap_h: [tw(0.0); 2],
            cap_p: [tw(0.0); 2],
            scroll: vec![tw(0.0); n],
            sb_drag: None,
            sb_h: tw(0.0),
            sb_on: tw(0.0),
            sb_hold: None,
            st: State::default(),
            layers: HashMap::new(),
            popup: None,
            press_key: None,
            hover: Hit::None,
            press: Hit::None,
            mouse: (-1.0, -1.0),
            mouse_in: false,
            dirty: true,
            live_dirty: false,
            ticking: false,
            page_damage: None,
            dock_icons: HashMap::new(),
            page_dy: None,
            caret_ms: 530.0,
            sel_color: Rgba::rgb(0, 120, 215),
            frozen,
            last_step: now,
            review: None,
            review_bg: ReviewBg::default(),
            toast: None,
            want_scroll: None,
            want_tab: None,
            pending: Vec::new(),
            modal: false,
            evs: [0; 4],
            hold_until: now + HOLD_MS,
            open_hold: 0.0,
            tips: pieces::tip::Tips::default(),
            tips_busy: false,
            tip_shown: false,
            focus_ring: false,
            drag_over: None,
            switch_wait: None,
            switch_shown: None,
            pre: None,
            pre_close_at: None,
        };
        ui.st.settle_until = now + SETTLE_MS;
        ui.on_t[first].jump(1.0);
        ui.reveal(first, false, now);
        ui.edges(now, true);
        ui
    }

    fn n(&self) -> usize {
        self.pages.len()
    }

    fn legacy(&mut self, tab: usize) -> Option<&mut dyn LegacyPage> {
        self.pages.get_mut(tab).and_then(|p| p.legacy())
    }

    // ============================================================== state
    /// One animation step: the row's smooth scroll, live pages, finished switches.
    pub fn update(&mut self, now: f64) {
        let dt = (now - self.last_step).clamp(0.0, 50.0);
        self.last_step = now;
        self.env.frozen = self.frozen;
        self.apply_pending(now);
        self.review_poll(now);
        self.sync_addon_tabs(now);
        // the top row follows its target (time-based ease, settles in ~160 ms)
        if (self.ds_t - self.ds).abs() > 0.0 {
            let d = self.ds_t - self.ds;
            if d.abs() < 0.3 || self.rm {
                self.ds = self.ds_t;
            } else {
                let k = 1.0 - (-(now - self.ds_last).clamp(0.0, 48.0) / 55.0).exp();
                self.ds += d * k as f32;
            }
            self.edges(now, false);
            self.dirty = true;
        }
        self.ds_last = now;
        // the shown tab's values are not in yet (its service reads on a worker thread): the switch / the open motion's
        // page part holds at its start - the old page stays - until it is ready, HOLD_MS at most; its transitions keep
        // jumping meanwhile (the owner Oct 8: no snapping from defaults to the real settings)
        if now < self.hold_until && self.close_t.is_none() && !self.pages[self.tab].ready() {
            if let Some((from, t0, dir)) = self.switch {
                self.switch = Some((from, t0 + dt, dir));
            } else if now - self.open_t < self.open_len() {
                self.open_hold += dt;
            }
            self.st.settle_until = self.st.settle_until.max(now + SETTLE_MS);
            self.dirty = true;
        } else if let Some(t0) = self.switch_wait.take() {
            // (measuring, Order 047) the clicked tab's page goes into this frame
            self.switch_shown = Some((t0, self.pages[self.tab].ready()));
        }
        // Order 047 item 10: the pointer rests on a tab's icon (its name label is due, 120 ms): that page opens now - its
        // service starts reading - so a click shows it at once; let go again if no click comes (PRE_KEEP_MS)
        if self.close_t.is_none() {
            if let Some(i) = self.lbl_i.filter(|&i| i != self.tab && now - self.lbl_since >= 120.0 && self.pre != Some(i)) {
                if self.pages[i].preopen() && self.switch.is_none_or(|s| s.0 != i) {
                    self.drop_pre();
                    let env = self.env.clone();
                    self.pages[i].open(&env, now);
                    self.pre = Some(i);
                }
            }
            if self.pre_close_at.is_some_and(|t| now >= t) {
                self.drop_pre();
            }
        }
        // a key, a key field or a job changed something (services.rs)
        if crate::services::take_dirty() {
            self.dirty = true;
            let tab = self.tab;
            self.mark_stale(tab);
        }
        if self.toast.as_ref().is_some_and(|t| now - t.1 > crate::ui::pieces::toast::SHOW_MS + 300.0) {
            self.toast = None;
            self.dirty = true;
        }
        // the old page of a finished switch lets go of its service
        if let Some((from, t0, _)) = self.switch {
            if now - t0 >= 300.0 {
                self.switch = None;
                if from != self.tab {
                    self.pages[from].close();
                    self.layers.remove(&from);
                }
            }
        }
        // the glass scrollbar's arrow held down: keeps going every 90 ms after 360 ms
        if let Some((dir, next)) = self.sb_hold {
            if now >= next {
                self.scroll_by(dir * 48.0, now);
                self.sb_hold = Some((dir, now + 90.0));
            }
        }
        let tab = self.tab;
        let sy = self.page_scroll(now);
        // sticky boxes follow the scroll: the page is built again when it moved
        if self.layers.get(&tab).and_then(|l| l.sticky_at).is_some_and(|at| (at - sy).abs() > 0.01) {
            self.mark_stale(tab);
            self.dirty = true;
        }
        let frozen = self.frozen;
        let (caret, sel) = (self.caret_ms, self.sel_color);
        if let Some(l) = self.legacy(tab) {
            l.set_style(frozen, caret, sel);
            l.set_scroll(sy);
            l.update(now);
            if l.take_dirty() {
                self.dirty = true;
            }
        } else {
            // live content moving (meters): the page's boxes are built again for the live pass only - the static page
            // layer is not painted again (that is `dirty`)
            self.ticking = self.close_t.is_none() && self.pages[tab].tick(now);
            // (Order 047: only the live boxes' own values moved - they read them when painted; the boxes stay)
            let live_only = self.ticking && self.pages[tab].live_only() && self.layers.get(&tab).is_some_and(|l| l.laid.has_live() && !l.stale);
            if self.ticking && !live_only {
                self.live_dirty = true;
                // a page whose moving content is NOT in live boxes (a stopwatch's digits, a clock: plain text) changes in
                // its static layer: paint that again too, every tick - else it moved only when the mouse did (feedback F4)
                if self.layers.get(&tab).is_none_or(|l| !l.laid.has_live()) {
                    self.dirty = true;
                }
            }
        }
        // the content got shorter (rows went away, a fold closed): the scroll stays inside it - measured on built boxes
        // only (no layer yet = height unknown, not 0: a press on a scrolled page jumped it to the top, Lane V 08:5x)
        let built = self.pages[tab].legacy_ref().is_some() || self.layers.get(&tab).is_some_and(|l| !l.stale);
        let m = self.scroll_max() as f64;
        if built && self.scroll[tab].target() > m {
            self.scroll[tab].jump(m);
            self.dirty = true;
        }
    }

    /// Does the menu need frames right now (anything moving)?
    pub fn animating(&self, now: f64) -> bool {
        if self.close_t.is_some() || now - self.open_t < self.open_len() {
            return true;
        }
        if self.pages[self.tab].legacy_ref().is_some() || self.ticking {
            return true; // live levels
        }
        self.static_busy(now)
    }

    /// How long the open motion runs (the last page child's fade-up ends).
    fn open_len(&self) -> f64 {
        self.stag_t0() + 10.0 * 26.0 + 300.0 + self.open_hold
    }

    /// Is anything but the live level meters moving (the cached page / top-row layers must be painted again)?
    pub fn static_busy(&self, now: f64) -> bool {
        if self.close_t.is_some() || now - self.open_t < self.open_len() {
            return true;
        }
        // a page asked for something in its build (a toast when a job is Done...): one more frame applies it
        // (REVIEW_014_item1+1c_08c5f74 HOLD 1)
        if !self.pending.is_empty() {
            return true;
        }
        if self.switch.map(|s| now - s.1 < 300.0).unwrap_or(false) || self.st.busy {
            return true;
        }
        let busy = |t: &[Tween]| t.iter().any(|x| x.busy(now));
        if busy(&self.mag) || busy(&self.ico_h) || busy(&self.on_t) || busy(&self.press_s) || busy(&self.cap_h) || busy(&self.cap_p) {
            return true;
        }
        if busy(&self.fades) || busy(&self.chev_on) || busy(&self.chev_h) || busy(&self.chev_p) || self.lbl_t.busy(now) || self.sb_h.busy(now) || self.sb_on.busy(now) {
            return true;
        }
        if self.lbl_i.is_some() && self.lbl_t.target() < 0.5 {
            return true; // the name label waits out its 120 ms
        }
        if self.dot.busy(now) || self.scroll[self.tab].busy(now) || self.bounce.iter().any(|b| now - b < 430.0) || self.appear.iter().any(|a| now - a < 430.0) || (self.ds_t - self.ds).abs() > 0.0 {
            return true;
        }
        // (a page's non-modal overlay alone - a selection bar - needs no frames)
        // (Order 047: an open popup / dialog, a toast at rest, a tip waiting out its delay need no frames: their moving
        // parts are transitions (`st.busy`) and their timed steps are wake-ups - `wake_at`)
        if self.sb_hold.is_some() || self.tips_busy {
            return true;
        }
        if let Some(l) = self.pages[self.tab].legacy_ref() {
            return l.busy(now);
        }
        false
    }

    /// Order 047: while nothing moves (`animating` false), the next moment the menu must wake by itself - the shown
    /// page's poll (`Page::wake_at`) or a picture that changes at a known time (`Cx::wake_at`, a toast's end, a tip's
    /// delay, the top row's name label). None = only input or a waker wakes it (zero CPU).
    pub fn wake_at(&self, now: f64) -> Option<f64> {
        let min = |a: Option<f64>, b: Option<f64>| match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if self.close_t.is_some() {
            return None;
        }
        let w = min(self.repaint_at(), self.pages[self.tab].wake_at(now));
        // a tab opened ahead on hover is let go (`update`)
        min(w, self.pre_close_at)
    }

    /// Order 047: close the page opened ahead on hover (unless it is shown now / still sliding out).
    fn drop_pre(&mut self) {
        self.pre_close_at = None;
        if let Some(j) = self.pre.take() {
            if j != self.tab && self.switch.is_none_or(|s| s.0 != j) {
                self.pages[j].close();
                self.layers.remove(&j);
            }
        }
    }

    /// The timed repaints (`wake_at` without the page's poll): the moment the picture changes by itself.
    fn repaint_at(&self) -> Option<f64> {
        let min = |a: Option<f64>, b: Option<f64>| match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let mut w = self.st.wake;
        if let Some((_, at)) = &self.toast {
            // it goes (`update`) after its fade-out (whose start the toast's own build asked for: `st.wake`)
            w = min(w, Some(at + crate::ui::pieces::toast::SHOW_MS + 300.0));
        }
        w = min(w, self.tips.due());
        if self.lbl_i.is_some() && self.lbl_t.target() < 0.5 {
            w = min(w, Some(self.lbl_since + 120.0));
        }
        w
    }

    /// Order 047: the menu woke by itself (`wake_at` came): a timed repaint builds the page again; the page's poll runs
    /// (`update` -> `tick`) and draws only if it says something changed.
    pub fn wake(&mut self, now: f64) {
        if self.repaint_at().is_some_and(|t| t <= now + 0.5) {
            self.dirty = true;
            let tab = self.tab;
            self.mark_stale(tab);
        }
        self.update(now);
    }

    // ============================================================== open / close / tabs
    /// Re-opened while the close motion ran: it stays on the tab it showed (feedback F3 - the menu comes back on the last
    /// tab), the page keeps what it had.
    pub fn open(&mut self, now: f64) {
        self.open_t = now;
        self.open_hold = 0.0;
        self.close_t = None;
        // a page switch still running: its old page goes now
        if let Some((from, _, _)) = self.switch.take() {
            if from != self.tab {
                self.pages[from].close();
                self.layers.remove(&from);
            }
        }
        let tab = self.tab;
        for i in 0..self.n() {
            self.on_t[i].jump(if i == tab { 1.0 } else { 0.0 });
        }
        self.dot.jump((dock_x(tab) + DT_W / 2.0) as f64);
        self.layers.clear();
        self.edges(now, true);
        self.dirty = true;
    }

    /// An add-on was got or removed (Order 037): its tab joins / leaves the top row now, the other tabs keep their pages and
    /// their state; the new icon pops in. The shown tab stays (if it was the one removed: Add-ons).
    fn sync_addon_tabs(&mut self, now: f64) {
        let g = crate::addons::gen();
        if g == self.addon_gen {
            return;
        }
        self.addon_gen = g;
        let mut fresh: Vec<Box<dyn Page>> = pages::all().into_iter().filter(|p| crate::addons::tab_visible(p.id())).collect();
        let want: Vec<&'static str> = fresh.iter().map(|p| p.id()).collect();
        let have: Vec<&'static str> = self.pages.iter().map(|p| p.id()).collect();
        if want == have {
            return;
        }
        let cur = have[self.tab];
        // (Order 047: a page opened ahead on hover is let go first - the indexes change)
        self.drop_pre();
        // a running switch: its old page closes now (and is not closed twice below if it is the one leaving)
        let mut closed: Option<&'static str> = None;
        if let Some((from, _, _)) = self.switch.take() {
            if from != self.tab {
                self.pages[from].close();
                closed = Some(have[from]);
            }
        }
        let mut old: Vec<Option<Box<dyn Page>>> = std::mem::take(&mut self.pages).into_iter().map(Some).collect();
        // new index -> old index (None = a tab that just joined)
        let map: Vec<Option<usize>> = want.iter().map(|id| have.iter().position(|h| h == id)).collect();
        let mut pages_new = Vec::with_capacity(want.len());
        for (id, m) in want.iter().zip(&map) {
            match m {
                Some(o) => pages_new.push(old[*o].take().expect("each old tab once")),
                None => {
                    let fi = fresh.iter().position(|p| p.id() == *id).expect("a wanted tab");
                    pages_new.push(fresh.remove(fi));
                }
            }
        }
        for mut p in old.into_iter().flatten() {
            if Some(p.id()) != closed {
                p.close();
            }
        }
        fn remap<T: Copy>(v: &[T], map: &[Option<usize>], new: T) -> Vec<T> {
            map.iter().map(|m| m.map(|o| v[o]).unwrap_or(new)).collect()
        }
        self.mag = remap(&self.mag, &map, tw(1.0));
        self.bounce = remap(&self.bounce, &map, -1e9);
        self.ico_h = remap(&self.ico_h, &map, tw(0.0));
        self.on_t = remap(&self.on_t, &map, tw(0.0));
        self.press_s = remap(&self.press_s, &map, tw(1.0));
        self.scroll = remap(&self.scroll, &map, tw(0.0));
        self.appear = remap(&self.appear, &map, if self.rm || self.close_t.is_some() { -1e9 } else { now });
        self.pages = pages_new;
        match want.iter().position(|id| *id == cur) {
            Some(t) => self.tab = t,
            None => {
                // the shown tab left: Add-ons shows, nothing of the old tab stays up
                self.tab = want.iter().position(|id| *id == "add").unwrap_or(0);
                let env = self.env.clone();
                self.pages[self.tab].open(&env, now);
                self.popup = None;
                self.want_scroll = None;
                self.modal = false;
            }
        }
        // a tab less: the row's scroll stays inside its new end
        let m = self.ds_max();
        if self.ds_t > m || self.ds > m {
            self.ds_t = self.ds_t.min(m);
            self.ds = self.ds.min(m);
        }
        // a tab joined (Get, Order 043): the row scrolls until it is fully in view (the shown tab stays)
        if let Some(j) = map.iter().position(|m| m.is_none()) {
            self.reveal(j, !self.rm, now);
        }
        let tab = self.tab;
        for i in 0..self.n() {
            self.on_t[i].jump(if i == tab { 1.0 } else { 0.0 });
        }
        self.dot.jump((dock_x(tab) + DT_W / 2.0) as f64);
        self.layers.clear();
        self.lbl_i = None;
        self.hover = Hit::None;
        self.press = Hit::None;
        self.review = None;
        self.edges(now, true);
        self.dirty = true;
    }

    /// The shown tab's page id (the app remembers it: the next open shows it again - feedback F3).
    pub fn tab_id(&self) -> &'static str {
        self.pages[self.tab].id()
    }

    pub fn close(&mut self, now: f64) {
        if self.close_t.is_none() {
            self.close_t = Some(now);
            self.drop_pre();
            self.dismiss_popup();
            crate::services::try_with(|s| s.stop_listening());
            self.lbl_i = None;
            let tab = self.tab;
            if let Some(l) = self.legacy(tab) {
                l.on_scroll(now);
                l.end_edit(false);
            }
        }
    }

    /// The close motion has finished (the window can go).
    pub fn closed(&self, now: f64) -> bool {
        self.close_t.map(|t| now - t >= if self.rm { 120.0 } else { 160.0 }).unwrap_or(false)
    }

    pub fn show_tab(&mut self, i: usize, now: f64) {
        if i == self.tab || i >= self.n() {
            return;
        }
        self.dismiss_popup();
        self.hide_tip(now);
        // a key field listening on the page left stops (else every key stays paused and the next key typed anywhere binds)
        crate::services::try_with(|s| s.stop_listening());
        // what the old page asked for in its last build is not for the new one (REVIEW_014_item1+1c_08c5f74 remark 1)
        self.pending.clear();
        let tab = self.tab;
        if let Some(l) = self.legacy(tab) {
            l.on_scroll(now);
            l.end_edit(true);
        }
        if let Some((from, _, _)) = self.switch {
            // a second switch before the first ended: the first page goes now
            if from != tab && from != i {
                self.pages[from].close();
                self.layers.remove(&from);
            }
        }
        let dir = if i > self.tab { 1.0 } else { -1.0 };
        self.switch = Some((self.tab, now, dir));
        for k in 0..self.n() {
            self.on_t[k].set(now, if k == i { 1.0 } else { 0.0 }, 300.0, anim::EASE);
        }
        self.tab = i;
        self.st.clear();
        self.press_key = None;
        let env = self.env.clone();
        // (Order 047: a page opened ahead on hover is open already - its reads ran meanwhile)
        if self.pre == Some(i) {
            self.pre = None;
            self.pre_close_at = None;
        } else {
            self.pages[i].open(&env, now);
        }
        self.st.settle_until = now + SETTLE_MS;
        self.hold_until = now + HOLD_MS;
        self.switch_wait = Some(now);
        self.scroll[i].jump(0.0);
        self.layers.remove(&i);
        let x = (dock_x(i) + DT_W / 2.0) as f64;
        if self.rm {
            self.dot.jump(x);
        } else {
            self.dot.set(now, x, 400.0, anim::DOT_GLIDE);
        }
        self.reveal(i, !self.rm, now);
        self.dirty = true;
    }

    /// The menu was opened FOR a tab (`services::show_menu`, a key with the menu closed): it opens showing that tab, no
    /// switch animation (the first page shown is that one), then hands it `target` (`Page::jump`).
    pub fn start_on(&mut self, id: &str, target: Option<&str>, now: f64) {
        let Some(i) = self.pages.iter().position(|p| p.id() == id) else { return };
        self.drop_pre();
        if i != self.tab {
            let old = self.tab;
            self.pages[old].close();
            self.layers.remove(&old);
            self.tab = i;
            for k in 0..self.n() {
                self.on_t[k].jump(if k == i { 1.0 } else { 0.0 });
            }
            let env = self.env.clone();
            self.pages[i].open(&env, now);
            self.st.settle_until = now + SETTLE_MS;
            self.hold_until = now + HOLD_MS;
            self.scroll[i].jump(0.0);
            self.dot.jump((dock_x(i) + DT_W / 2.0) as f64);
            self.reveal(i, false, now);
            self.dirty = true;
        }
        if let Some(t) = target {
            self.pages[i].jump(t);
        }
    }

    /// Show the tab `id` (animated, like a click on it) and hand it `target` - `services::show_menu` with the menu open.
    pub fn go_to(&mut self, id: &str, target: Option<&str>, now: f64) {
        self.want_tab = Some((id.to_string(), target.map(str::to_string)));
        self.go_to_wanted_tab(now);
    }

    pub fn switching(&self, now: f64) -> bool {
        self.switch.map(|s| now - s.1 < if self.rm { 240.0 } else { 280.0 }).unwrap_or(false)
    }

    // ============================================================== the top row's sideways scroll
    fn ds_max(&self) -> f32 {
        let content = dock_x(self.n() - 1) + DT_W + DOCK_PAD;
        (content - DS_W).max(0.0)
    }
    fn ds_go(&mut self, to: f32, smooth: bool, now: f64) {
        self.ds_t = to.clamp(0.0, self.ds_max());
        if !smooth || self.rm {
            self.ds = self.ds_t;
        }
        self.ds_last = now;
        self.lbl_i = None;
        self.edges(now, false);
        self.dirty = true;
    }
    /// The active icon always in view, clear of the faded edges (the drawing's dockReveal).
    fn reveal(&mut self, i: usize, smooth: bool, now: f64) {
        let l = dock_x(i) - SCR_FADE - 6.0;
        let r = dock_x(i) + DT_W + SCR_FADE + 6.0;
        if l < self.ds_t {
            self.ds_go(l, smooth, now);
        } else if r > self.ds_t + DS_W {
            self.ds_go(r - DS_W, smooth, now);
        }
    }
    /// The soft fades and the chevrons (the drawing's dockEdges + dockChev), as CSS transitions.
    fn edges(&mut self, now: f64, jump: bool) {
        let m = self.ds_max();
        let x = self.ds;
        let fl = x > 1.0;
        let fr = x < m - 1.0;
        let px = self.dock_px;
        let nl = px.map(|p| p < DS_X + 36.0).unwrap_or(false) && fl;
        let nr = px.map(|p| p > DS_X + DS_W - 36.0).unwrap_or(false) && fr;
        // .fl --fl 30 (22 while its chevron shows: .cl --cl 30 --fl 22)
        let tfl = if nl { 22.0 } else if fl { 30.0 } else { 0.0 };
        let tfr = if nr { 22.0 } else if fr { 30.0 } else { 0.0 };
        let t = [(tfl, 220.0), (tfr, 220.0), (if nl { 30.0 } else { 0.0 }, 200.0), (if nr { 30.0 } else { 0.0 }, 200.0)];
        for (k, (v, d)) in t.iter().enumerate() {
            if jump {
                self.fades[k].jump(*v);
            } else if (self.fades[k].target() - *v).abs() > 1e-6 {
                self.fades[k].set(now, *v, *d, anim::EASE);
            }
        }
        for (k, on) in [nl, nr].iter().enumerate() {
            let v = if *on { 1.0 } else { 0.0 };
            if jump {
                self.chev_on[k].jump(v);
            } else if (self.chev_on[k].target() - v).abs() > 1e-6 {
                self.chev_on[k].set(now, v, 180.0, anim::EASE);
            }
        }
    }

    // ============================================================== page scroll
    fn content_height(&mut self, tab: usize) -> f32 {
        if let Some(l) = self.pages[tab].legacy_ref() {
            return l.content_height();
        }
        self.layers.get(&tab).map(|l| l.laid.height).unwrap_or(0.0)
    }
    fn scroll_max(&mut self) -> f32 {
        let t = self.tab;
        (self.content_height(t) - PAGE_H).max(0.0)
    }
    fn scroll_by(&mut self, d: f32, now: f64) {
        let m = self.scroll_max() as f64;
        let t = (self.scroll[self.tab].target() + d as f64).clamp(0.0, m);
        self.scroll[self.tab].set(now, t, 160.0, anim::EASE_OUT_CSS);
        self.after_scroll(now);
    }
    fn scroll_to(&mut self, v: f32, now: f64) {
        let m = self.scroll_max();
        self.scroll[self.tab].jump(v.clamp(0.0, m) as f64);
        self.after_scroll(now);
    }
    fn after_scroll(&mut self, now: f64) {
        let tab = self.tab;
        if let Some(l) = self.legacy(tab) {
            l.on_scroll(now);
        }
        self.dismiss_popup();
        self.dirty = true;
    }

    /// How far the current page is scrolled (DIPs).
    pub fn page_scroll(&self, now: f64) -> f32 {
        self.scroll[self.tab].value(now) as f32
    }

    /// The glass scrollbar's track (.gsb: right 5, top 8 / bottom 14 inside .right, 7 wide; arrows 13, gap 4).
    fn sb_geom(&mut self, now: f64) -> Option<(f32, f32, f32, f32)> {
        let t = self.tab;
        let sh = self.content_height(t);
        let ch = PAGE_H;
        let max = sh - ch;
        if max <= 1.0 {
            return None;
        }
        let top = 52.0 + 8.0;
        let bottom = WIN_H - 14.0;
        let tr_top = top + 13.0 + 4.0;
        let tr_h = bottom - 13.0 - 4.0 - tr_top;
        let th = (tr_h * ch / sh).round().max(24.0);
        let y = (tr_h - th) * (self.page_scroll(now) / max).clamp(0.0, 1.0);
        let y = (y * 10.0).round() / 10.0; // y.toFixed(1)
        Some((tr_top, tr_h, y, th))
    }

    // ============================================================== hit test
    /// The thing under a point (topmost first).
    pub fn hit(&mut self, x: f32, y: f32, now: f64) -> Hit {
        if let Some(p) = &self.popup {
            if let Some((i, keys)) = p.hit(x, y) {
                return Hit::Popup(keys.first().copied(), p.cursor_at(i));
            }
        }
        let tab = self.tab;
        if y < 52.0 {
            if x >= 508.0 && x < 554.0 {
                return Hit::CapMin;
            }
            if x >= 554.0 {
                return Hit::CapClose;
            }
            // chevrons (24 x 24 round, top 14; left 13 / right 97), only while shown
            for k in 0..2 {
                let cx = if k == 0 { 13.0 } else { WIN_W - 97.0 - 24.0 };
                if self.chev_on[k].target() > 0.5 && x >= cx && x < cx + 24.0 && y >= 14.0 && y < 38.0 {
                    return Hit::Chev(k);
                }
            }
            if x >= DS_X && x < DS_X + DS_W {
                let lx = x - DS_X + self.ds;
                for i in 0..self.n() {
                    let bx = dock_x(i);
                    if lx >= bx && lx < bx + DT_W && y >= DOCK_Y && y < DOCK_Y + 34.0 {
                        return Hit::Dock(i);
                    }
                }
            }
            return Hit::None;
        }
        // the glass scrollbar (+ its invisible wider lane: left -5, right -4)
        if let Some((tt, th_, ty, thh)) = self.sb_geom(now) {
            if x >= 588.0 - 5.0 && x < 595.0 + 4.0 && y >= 60.0 && y < WIN_H - 14.0 {
                if y < tt - 4.0 {
                    return Hit::SbUp;
                }
                if y >= tt + th_ + 4.0 {
                    return Hit::SbDown;
                }
                if y >= tt + ty && y < tt + ty + thh {
                    return Hit::SbThumb;
                }
                return Hit::SbTrack;
            }
        }
        if y < PAGE_TOP {
            return Hit::None;
        }
        if let Some(l) = self.pages[tab].legacy_ref() {
            let (h, t) = l.cursor();
            return Hit::Legacy(h, t);
        }
        let sy = self.page_scroll(now);
        if let Some(pl) = self.layers.get(&tab) {
            if let Some((i, keys)) = pl.laid.hit(x, y - PAGE_TOP + sy) {
                return Hit::Page(keys.first().copied(), pl.laid.cursor_at(i));
            }
        }
        Hit::Page(None, Cursor::Default)
    }

    /// Keys under a point: (popup?, chain of keys innermost first, clickable key, box of the innermost key).
    fn keys_at(&self, x: f32, y: f32, now: f64) -> Option<(bool, Vec<Key>, Option<Key>, (f32, f32, f32, f32))> {
        if let Some(p) = &self.popup {
            if let Some((i, keys)) = p.hit(x, y) {
                let r = keys.first().and_then(|k| p.rect_of(*k)).unwrap_or((0.0, 0.0, 0.0, 0.0));
                return Some((true, keys, p.clickable(i), r));
            }
        }
        if y < PAGE_TOP {
            return None;
        }
        let sy = self.page_scroll(now);
        let pl = self.layers.get(&self.tab)?;
        let (i, keys) = pl.laid.hit(x, y - PAGE_TOP + sy)?;
        let r = keys.first().and_then(|k| pl.laid.rect_of(*k)).map(|(a, b, w, h)| (a, b + PAGE_TOP - sy, w, h)).unwrap_or((0.0, 0.0, 0.0, 0.0));
        Some((false, keys, pl.laid.clickable(i), r))
    }

    // ============================================================== input
    pub fn mouse_move(&mut self, x: f32, y: f32, now: f64) {
        self.mouse = (x, y);
        self.mouse_in = true;
        if let Some((y0, s0)) = self.sb_drag {
            if let Some((_, tr_h, _, th)) = self.sb_geom(now) {
                let k = self.scroll_max() / (tr_h - th).max(1.0);
                self.scroll_to(s0 + (y - y0) * k, now);
            }
            return;
        }
        // a drag on a page element (sliders)
        if let Some((k, popup)) = self.press_key {
            let r = self.rect_of_key(k, popup, now).unwrap_or((0.0, 0.0, 0.0, 0.0));
            self.page_event(Ev::Drag(k, x, y, r), now);
        }
        let h = self.hit(x, y, now);
        self.set_hover(h, x, y, now);
        // the pointer over the top row: chevrons near a faded edge, magnification
        self.dock_px = if y < 52.0 { Some(x) } else { None };
        self.edges(now, false);
        if !self.rm {
            let in_dock = y < 52.0 && x >= DS_X && x < DS_X + DS_W;
            for i in 0..self.n() {
                let target = if in_dock {
                    let cxi = DS_X - self.ds + dock_x(i) + DT_W / 2.0;
                    let d = (x - cxi).abs();
                    1.0 + 0.18 * (1.0 - d / 68.0).max(0.0)
                } else {
                    1.0
                };
                self.mag[i].set(now, ((target * 1000.0).round() / 1000.0) as f64, 90.0, anim::EASE_OUT_CSS);
            }
        }
        let tab = self.tab;
        if let Some(l) = self.legacy(tab) {
            if y >= PAGE_TOP || l.popup_open() {
                l.mouse_move(x, y, now);
            } else {
                l.mouse_leave(now);
            }
        }
    }

    pub fn mouse_leave(&mut self, now: f64) {
        self.hide_tip(now);
        self.mouse_in = false;
        if self.sb_drag.is_none() && self.press_key.is_none() {
            self.set_hover(Hit::None, -1.0, -1.0, now);
        }
        self.dock_px = None;
        self.edges(now, false);
        for i in 0..self.n() {
            self.mag[i].set(now, 1.0, 90.0, anim::EASE_OUT_CSS);
        }
        let tab = self.tab;
        if let Some(l) = self.legacy(tab) {
            l.mouse_leave(now);
        }
    }

    fn set_hover(&mut self, h: Hit, x: f32, y: f32, now: f64) {
        // the page's :hover chain
        let at = self.keys_at(x, y, now);
        // the pointer in page coordinates (Cx::hover_point); a page reading it is built again on every move over it
        self.st.pointer = if y >= PAGE_TOP && at.as_ref().is_some_and(|a| !a.0) { Some((x, y - PAGE_TOP + self.page_scroll(now))) } else { None };
        let keys = at.map(|k| k.1).unwrap_or_default();
        if keys.iter().any(|k| self.st.watch.contains(k)) {
            self.dirty = true;
        }
        if keys != self.st.hover {
            for k in &keys {
                if !self.st.hover.contains(k) {
                    self.st.hover_since.insert(*k, now);
                }
            }
            self.st.hover_since.retain(|k, _| keys.contains(k));
            self.st.hover = keys;
            self.dirty = true;
        }
        if h == self.hover {
            return;
        }
        let old = self.hover;
        self.hover = h;
        self.dirty = true;
        let dock_h = |x: Hit| if let Hit::Dock(i) = x { Some(i) } else { None };
        if let Some(i) = dock_h(old) {
            self.ico_h[i].set(now, 0.0, 180.0, anim::EASE);
        }
        if let Some(i) = dock_h(h) {
            self.ico_h[i].set(now, 1.0, 180.0, anim::EASE);
        }
        // the shared name label: shows 120 ms after the pointer rests on an icon, hides at once when it leaves
        if dock_h(h) != self.lbl_i {
            self.lbl_i = dock_h(h);
            self.lbl_since = now;
            self.lbl_t.set(now, 0.0, 120.0, anim::EASE);
        }
        // Order 047: a tab opened ahead on hover (`update`) is let go a while after the pointer leaves the icons
        if dock_h(h).is_some() {
            self.pre_close_at = None;
        } else if self.pre.is_some() && self.pre_close_at.is_none() {
            self.pre_close_at = Some(now + PRE_KEEP_MS);
        }
        let cap = |x: Hit| match x {
            Hit::CapMin => Some(0),
            Hit::CapClose => Some(1),
            _ => None,
        };
        if let Some(i) = cap(old) {
            self.cap_h[i].set(now, 0.0, 120.0, anim::EASE);
        }
        if let Some(i) = cap(h) {
            self.cap_h[i].set(now, 1.0, 120.0, anim::EASE);
        }
        let chev = |x: Hit| if let Hit::Chev(k) = x { Some(k) } else { None };
        if let Some(k) = chev(old) {
            self.chev_h[k].set(now, 0.0, 140.0, anim::EASE);
        }
        if let Some(k) = chev(h) {
            self.chev_h[k].set(now, 1.0, 140.0, anim::EASE);
        }
        let sb = |x: Hit| matches!(x, Hit::SbUp | Hit::SbDown | Hit::SbThumb | Hit::SbTrack);
        if sb(old) != sb(h) {
            self.sb_h.set(now, if sb(h) { 1.0 } else { 0.0 }, 150.0, anim::EASE);
        }
    }

    fn rect_of_key(&self, k: Key, popup: bool, now: f64) -> Option<(f32, f32, f32, f32)> {
        if popup {
            return self.popup.as_ref()?.rect_of(k);
        }
        let sy = self.page_scroll(now);
        self.layers.get(&self.tab)?.laid.rect_of(k).map(|(a, b, w, h)| (a, b + PAGE_TOP - sy, w, h))
    }

    /// Hand an event to the shown page (with the modifiers held now); returns whether the page used it (`cx.used`).
    fn page_event(&mut self, ev: Ev, now: f64) -> bool {
        let tab = self.tab;
        // the page handles it with its own Cx (the painter is only needed for text widths)
        let g = DUMMY_G.with(|g| g.clone());
        // the frame's own popup (the reset review) takes its clicks first
        if self.review_event(&ev, now) {
            self.dirty = true;
            self.mark_stale(tab);
            return true;
        }
        let page = self.pages[tab].id();
        let mut cx = Cx::new(now, self.rm, &g, &mut self.st).for_page(page);
        cx.in_click = matches!(ev, Ev::Click(_) | Ev::Drop(..));
        cx.mods = cx::Mods::now();
        // (counted for the test hook's `state`: the frame delivered them, whatever the page does)
        match &ev {
            Ev::Click(_) => self.evs[0] += 1,
            Ev::Context(..) => self.evs[1] += 1,
            Ev::Drop(..) => self.evs[2] += 1,
            Ev::Key(k, _) if *k == cx::PAGE => self.evs[3] += 1,
            _ => {}
        }
        self.pages[tab].event(&ev, &mut cx);
        let used = cx.used;
        let reqs = std::mem::take(&mut cx.reqs);
        drop(cx);
        let drag = self.apply_reqs(reqs, now);
        self.dirty = true;
        self.mark_stale(tab);
        // files dragged out (Windows' drag is modal: the button comes up inside it) - the press ends here, no click
        if let Some(paths) = drag {
            if !crate::testmode::on() {
                dragout::drag_out(&paths);
            }
            if let Some((k, _)) = self.press_key.take() {
                self.st.active.clear();
                self.press = Hit::None;
                self.page_event(Ev::Release(k), now);
            }
        }
        self.go_to_wanted_tab(now);
        used
    }

    /// What a page asked for through its Cx (the shown page's requests); returns the files to drag out (only an event's).
    fn apply_reqs(&mut self, reqs: Vec<cx::Req>, now: f64) -> Option<Vec<String>> {
        let tab = self.tab;
        let mut drag = None;
        for r in reqs {
            match r {
                cx::Req::Toast(t) => self.toast = Some((t, now)),
                // (resolved after the next build: the element may be new)
                cx::Req::ScrollTo(k) => self.want_scroll = Some(WantScroll::Key(k)),
                cx::Req::ScrollY(y) => self.want_scroll = Some(WantScroll::Y(y)),
                cx::Req::ShowTab(id, target) => self.want_tab = Some((id, target)),
                cx::Req::DragOut(paths) => drag = Some(paths),
                // (the page's `resettable()` runs first: it may build its service; then the store, queued notes written first)
                // Order 047: a page that hands a detached copy is read on a worker thread - the review opens when its lines
                // are in (`review_poll`); the others are read here as before
                cx::Req::Reset(kind, (x, y, _w, h)) => {
                    let mut rs: Vec<&mut dyn crate::undo::Resettable> = self.pages[tab].resettable().into_iter().collect();
                    let opened = if rs.is_empty() {
                        None
                    } else {
                        crate::services::with(|s| {
                            crate::undo::flush(&mut s.store);
                            crate::undo::Review::open(kind, false, &mut rs, &s.store)
                        })
                    };
                    drop(rs);
                    self.review_opened(opened, x, y + h + 4.0, tab, now);
                }
                // Settings › Reset: every tab's lines, in the top row's order, grouped by tab
                cx::Req::ResetAll(kind, (x, y, _w, h)) => {
                    let mut rs: Vec<&mut dyn crate::undo::Resettable> = self.pages.iter_mut().filter_map(|p| p.resettable()).collect();
                    let opened = crate::services::with(|s| {
                        crate::undo::flush(&mut s.store);
                        crate::undo::Review::open(kind, true, &mut rs, &s.store)
                    });
                    drop(rs);
                    self.review_opened(opened, x, y + h + 4.0, tab, now);
                }
            }
        }
        drag
    }

    /// Another tab asked for (after the page's event / build: the switch closes this page).
    fn go_to_wanted_tab(&mut self, now: f64) {
        if let Some((id, target)) = self.want_tab.take() {
            if let Some(i) = self.pages.iter().position(|p| p.id() == id) {
                self.show_tab(i, now);
                if let Some(t) = target {
                    self.pages[i].jump(&t);
                }
            }
        }
    }

    /// Requests made in `build` / `popup` / `overlay` (kept by `ensure_layer`), applied here before the next build. A drag
    /// out needs a pressed button, so only an event may ask for one; from a build it is dropped.
    fn apply_pending(&mut self, now: f64) {
        if self.pending.is_empty() {
            return;
        }
        let reqs = std::mem::take(&mut self.pending);
        let _ = self.apply_reqs(reqs, now);
        self.dirty = true;
        let tab = self.tab;
        self.mark_stale(tab);
        self.go_to_wanted_tab(now);
    }

    /// A page asked to scroll (`cx.scroll_to` / `cx.scroll_y`), once its boxes are built: the element to the nearest edge
    /// of the view (the drawing's `scrollIntoView({block:'nearest'})`), or the offset; animated like a wheel notch.
    fn resolve_scroll(&mut self, now: f64) {
        let Some(w) = self.want_scroll.take() else { return };
        let cur = self.scroll[self.tab].target() as f32;
        let to = match w {
            WantScroll::Y(y) => y,
            WantScroll::Key(k) => {
                let Some((_, y, _, h)) = self.layers.get(&self.tab).and_then(|l| l.laid.rect_of(k)) else { return };
                if y < cur {
                    y
                } else if y + h > cur + PAGE_H {
                    y + h - PAGE_H
                } else {
                    cur
                }
            }
        };
        // already there: no scroll, so nothing is dismissed (a page scroll closes an open popup / the reset review)
        if (to.clamp(0.0, self.scroll_max()) - cur).abs() < 0.01 {
            return;
        }
        self.scroll_by(to - cur, now);
    }

    /// Test hook `scroll:<px>`: the page's scroll offset at once (no animation), to prove a page below its first screen.
    pub fn scroll_now(&mut self, y: f32, now: f64) {
        self.scroll_to(y, now);
        self.dirty = true;
    }

    /// The right button came up at (x, y): `Ev::Context` for the element under it (the clickable one, else the innermost
    /// keyed one).
    pub fn context(&mut self, x: f32, y: f32, now: f64) {
        self.mouse = (x, y);
        let tab = self.tab;
        if self.legacy(tab).is_some() {
            return;
        }
        // the right button outside an open popup closes it, as the left one does (menu-v22: any pointerdown outside the
        // menu closes it; REVIEW_014_item1c remark 5). A drop has no pointerdown in the drawing: it closes nothing.
        if self.modal && !matches!(self.hit(x, y, now), Hit::Popup(..)) {
            self.dismiss_popup();
        }
        if let Some((_, keys, click, _)) = self.keys_at(x, y, now) {
            if let Some(k) = click.or_else(|| keys.first().copied()) {
                self.page_event(Ev::Context(k, x, y), now);
            }
        }
    }

    /// Order 045: files dragged over the window from Explorer at (x, y) (None = they left / were dropped): the page hears
    /// `Ev::DragOver` with the element under them (found as for a drop) each time that element changes.
    pub fn drag_over(&mut self, at: Option<(f32, f32)>, now: f64) {
        let tab = self.tab;
        if self.legacy(tab).is_some() {
            return;
        }
        let k = at.and_then(|(x, y)| self.keys_at(x, y, now)).and_then(|(_, keys, click, _)| click.or_else(|| keys.first().copied()));
        if k != self.drag_over {
            self.drag_over = k;
            self.page_event(Ev::DragOver(k), now);
        }
    }

    /// Files / folders dropped on the window at (x, y): `Ev::Drop` for the element under them (as `context`).
    pub fn drop_files(&mut self, x: f32, y: f32, paths: Vec<String>, now: f64) {
        let tab = self.tab;
        if self.legacy(tab).is_some() || paths.is_empty() {
            return;
        }
        if let Some((_, keys, click, _)) = self.keys_at(x, y, now) {
            if let Some(k) = click.or_else(|| keys.first().copied()) {
                self.page_event(Ev::Drop(k, paths), now);
            }
        }
    }

    /// The page changed: built again at the next paint; its old boxes stay for hit tests until then (a fast click's
    /// release arrives before that paint - dropping them lost every Ev::Click, Lane V / Lane R 08:2x).
    fn mark_stale(&mut self, tab: usize) {
        if let Some(l) = self.layers.get_mut(&tab) {
            l.stale = true;
        }
    }

    /// Clicks on the reset review: a line ticks / unticks, Cancel closes, Reset applies the ticked lines through the
    /// page (undo::Review::apply) and says how it went.
    fn review_event(&mut self, ev: &Ev, now: f64) -> bool {
        let Some(rs) = self.review.as_mut() else { return false };
        let Ev::Click(k) = ev else { return matches!(ev, Ev::Press(..) | Ev::Release(..)) && self.modal };
        if let Some(i) = (0..rs.review.lines.len()).find(|&i| el::idx(K_REVIEW, i) == *k) {
            rs.review.toggle(i);
            return true;
        }
        if *k == el::sub(K_REVIEW, "no") {
            self.review = None;
            return true;
        }
        if *k == el::sub(K_REVIEW, "go") {
            let rs = self.review.take().unwrap();
            let n = rs.review.ticked();
            let (all, tab) = (rs.review.all, rs.tab);
            // the pages put their items back while the frame does NOT hold the services (a page may save its own
            // settings meanwhile); each ok line is recorded (queued notes first)
            let mut rp: Vec<&mut dyn crate::undo::Resettable> =
                if all { self.pages.iter_mut().filter_map(|p| p.resettable()).collect() } else { self.pages[tab].resettable().into_iter().collect() };
            // Order 047: a page that hands a detached copy is put back on a worker thread (an admin prompt, a display mode
            // change wait there); the toast comes when it has ended (`review_poll`)
            let started = rs.review.start_apply(&mut rp);
            drop(rp);
            match started {
                crate::undo::Applied::Done(results) => self.toast = Some((crate::undo::reset_toast(rs.review.kind, n, &results), now)),
                crate::undo::Applied::Running(job) => self.review_bg.applying.push((job, rs.review.kind, n)),
            }
            return true;
        }
        false
    }

    /// Order 047: what `Review::open` gave: the review shows now, or when its worker has read the detached pages.
    fn review_opened(&mut self, opened: Option<crate::undo::Opened>, x: f32, y: f32, tab: usize, now: f64) {
        match opened {
            Some(crate::undo::Opened::Ready(r)) if !r.is_empty() => self.review = Some(ReviewState { review: r, x, y, tab }),
            Some(crate::undo::Opened::Ready(_)) => self.toast = Some(("Nothing to reset".to_string(), now)),
            Some(crate::undo::Opened::Reading(job)) => self.review_bg.reading = Some((job, x, y, self.pages[tab].id())),
            None => {}
        }
    }

    /// Order 047: the reset review's work off the menu's thread came back (the worker woke the menu; called every step):
    /// the review shows (still on its tab), a finished reset says how it went and its pages read their new state.
    fn review_poll(&mut self, now: f64) {
        if let Some((job, x, y, id)) = self.review_bg.reading.as_mut() {
            if let Some(r) = job.take() {
                let (x, y, id) = (*x, *y, *id);
                self.review_bg.reading = None;
                if self.pages[self.tab].id() == id {
                    let tab = self.tab;
                    if r.is_empty() {
                        self.toast = Some(("Nothing to reset".to_string(), now));
                    } else {
                        self.review = Some(ReviewState { review: r, x, y, tab });
                    }
                    self.dirty = true;
                    self.mark_stale(tab);
                }
            }
        }
        let mut i = 0;
        while i < self.review_bg.applying.len() {
            let Some(results) = self.review_bg.applying[i].0.take() else {
                i += 1;
                continue;
            };
            let (job, kind, n) = self.review_bg.applying.remove(i);
            for p in self.pages.iter_mut() {
                if let Some(r) = p.resettable() {
                    if job.away.iter().any(|a| a == r.page_id()) {
                        r.reset_done();
                    }
                }
            }
            self.toast = Some((crate::undo::reset_toast(kind, n, &results), now));
            self.dirty = true;
            let tab = self.tab;
            self.mark_stale(tab);
        }
    }

    fn dismiss_popup(&mut self) {
        self.review = None;
        // (Order 047: a review still being read is dropped too - it never pops up after the user moved on)
        self.review_bg.reading = None;
        self.modal = false;
        if self.popup.take().is_some() {
            let tab = self.tab;
            self.pages[tab].popup_dismiss();
            self.dirty = true;
        }
    }

    /// Left button down. Returns what the window should do.
    pub fn mouse_down(&mut self, x: f32, y: f32, now: f64) -> Action {
        self.hide_tip(now);
        self.focus_ring = false;
        self.mouse = (x, y);
        let h = self.hit(x, y, now);
        self.press = h;
        self.dirty = true;
        match h {
            Hit::CapMin => self.cap_p[0].set(now, 1.0, 120.0, anim::EASE),
            Hit::CapClose => self.cap_p[1].set(now, 1.0, 120.0, anim::EASE),
            Hit::Dock(i) => {
                if !self.rm {
                    self.press_s[i].set(now, 0.92, 120.0, anim::EASE);
                }
            }
            Hit::Chev(k) => self.chev_p[k].set(now, 1.0, 120.0, anim::EASE),
            Hit::SbUp | Hit::SbDown => {
                let dir = if h == Hit::SbUp { -1.0 } else { 1.0 };
                self.scroll_by(dir * 48.0, now);
                self.sb_hold = Some((dir, now + 360.0));
            }
            Hit::SbThumb => {
                self.sb_drag = Some((y, self.page_scroll(now)));
            }
            Hit::SbTrack => {
                if let Some((tt, _, ty, _)) = self.sb_geom(now) {
                    let dir = if y < tt + ty { -1.0 } else { 1.0 };
                    self.scroll_by(dir * PAGE_H * 0.85, now);
                }
            }
            _ => {}
        }
        // a click outside a page popup closes it (and still goes to what was clicked)
        if self.modal && !matches!(h, Hit::Popup(..)) {
            self.dismiss_popup();
        }
        if let Hit::Legacy(..) = h {
            let tab = self.tab;
            if let Some(l) = self.legacy(tab) {
                return l.mouse_down(x, y, now);
            }
        }
        let tab = self.tab;
        if let Some(l) = self.legacy(tab) {
            if l.popup_open() {
                return l.mouse_down(x, y, now);
            }
        }
        if let Some((popup, mut keys, _click, mut r)) = self.keys_at(x, y, now) {
            // Order 045: a key that is there only for a tip (a lock icon, a name, a shield inside a button) does not take the
            // press - the control around it does (its box anchors the page's popup)
            let laid = if popup { self.popup.as_ref() } else { self.layers.get(&self.tab).map(|l| &l.laid) };
            let skip = laid.map(|l| keys.iter().take_while(|k| l.tip_only(**k)).count()).unwrap_or(0);
            if skip > 0 && skip < keys.len() {
                keys.drain(..skip);
                r = self.rect_of_key(keys[0], popup, now).unwrap_or(r);
            }
            self.st.active = keys.clone();
            if let Some(&k) = keys.first() {
                if self.st.focus != Some(k) {
                    if let Some(old) = self.st.focus.take() {
                        self.page_event(Ev::Blur(old), now);
                    }
                    self.st.focus = Some(k);
                }
                self.press_key = Some((k, popup));
                self.page_event(Ev::Press(k, x, y, r), now);
            } else if let Some(old) = self.st.focus.take() {
                // a click on a key-less part of the page (a title) blurs the focused field too
                self.page_event(Ev::Blur(old), now);
            }
        } else if let Some(old) = self.st.focus.take() {
            self.page_event(Ev::Blur(old), now);
        }
        Action::None
    }

    pub fn mouse_up(&mut self, x: f32, y: f32, now: f64) -> Action {
        let pressed = std::mem::replace(&mut self.press, Hit::None);
        self.dirty = true;
        self.cap_p[0].set(now, 0.0, 120.0, anim::EASE);
        self.cap_p[1].set(now, 0.0, 120.0, anim::EASE);
        self.chev_p[0].set(now, 0.0, 120.0, anim::EASE);
        self.chev_p[1].set(now, 0.0, 120.0, anim::EASE);
        for i in 0..self.n() {
            self.press_s[i].set(now, 1.0, 120.0, anim::EASE);
        }
        self.sb_hold = None;
        if self.sb_drag.take().is_some() {
            return Action::None;
        }
        let tab = self.tab;
        if let Some(l) = self.legacy(tab) {
            if matches!(pressed, Hit::Legacy(..)) || l.popup_open() {
                return l.mouse_up(x, y, now);
            }
        }
        let down_click = self.press_key.map(|(k, _)| k);
        // what is under the release, taken BEFORE the page hears the Release (its event may change the boxes / popup)
        let h = self.hit(x, y, now);
        let up_keys = self.keys_at(x, y, now);
        if let Some((k, _)) = self.press_key.take() {
            self.st.active.clear();
            self.page_event(Ev::Release(k), now);
        }
        if h != pressed && !matches!((h, pressed), (Hit::Page(..), Hit::Page(..)) | (Hit::Popup(..), Hit::Popup(..))) {
            return Action::None;
        }
        match h {
            Hit::CapMin | Hit::CapClose => return Action::Close,
            Hit::Dock(i) => {
                if !self.rm {
                    self.bounce[i] = now;
                }
                self.show_tab(i, now);
            }
            Hit::Chev(k) => {
                // a click = 3 icons
                let to = self.ds_t + if k == 0 { -105.0 } else { 105.0 };
                self.ds_go(to, true, now);
            }
            Hit::Page(..) | Hit::Popup(..) => {
                if let (Some((_, chain, Some(u), _)), Some(d)) = (up_keys, down_click) {
                    // the clickable element under the release must be the one pressed (or hold it)
                    if chain.contains(&d) || u == d {
                        self.page_event(Ev::Click(u), now);
                    }
                }
            }
            _ => {}
        }
        Action::None
    }

    /// A click on a target by name (the test hook's "click").
    pub fn click(&mut self, name: &str, now: f64) -> Action {
        match self.target_point(name) {
            Some((x, y)) => {
                self.mouse_move(x, y, now);
                let _ = self.mouse_down(x, y, now);
                self.mouse_up(x, y, now)
            }
            None => Action::None,
        }
    }

    pub fn wheel(&mut self, delta: f32, now: f64) {
        self.hide_tip(now);
        let (x, y) = self.mouse;
        if y < 52.0 {
            // the top row scrolls sideways: one notch = 45 px
            if x >= DS_X && x < DS_X + DS_W && self.ds_max() > 0.0 {
                let notch = if delta.abs() >= 50.0 { -delta / 120.0 } else { 0.0 };
                let to = self.ds_t + if notch != 0.0 { notch * 45.0 } else { -delta };
                self.ds_go(to, true, now);
            }
            return;
        }
        // Order 045: a field that steps with the wheel (Display's Width / Height / Hz, Mouse's DPI box) takes it; the page
        // stays where it is (the drawing's `preventDefault`)
        if delta != 0.0 {
            if let Some((popup, keys, _, _)) = self.keys_at(x, y, now) {
                let found = if popup { self.popup.as_ref().and_then(|p| p.wheel_in(&keys)) } else { self.layers.get(&self.tab).and_then(|l| l.laid.wheel_in(&keys)) };
                if let Some(k) = found {
                    self.page_event(Ev::Wheel(k, if delta > 0.0 { 1 } else { -1 }), now);
                    return;
                }
            }
        }
        // a scrolling box under the pointer (a popup list, a tall dialog's body) scrolls first; at its end the page does
        // (CSS scroll chaining)
        let step = -delta / 120.0 * 100.0;
        if self.wheel_box(x, y, step, now) {
            return;
        }
        // over a modal popup (a dialog and its scrim): the page under it does not move - and the popup is not dismissed
        // by a scroll (a notch at the end of a dialog's body closed it, Order 029 review)
        if self.modal && self.keys_at(x, y, now).is_some_and(|k| k.0) {
            return;
        }
        // Chromium: 100 px per notch (3 lines), animated
        self.scroll_by(step, now);
    }

    /// The wheel over a `Cx::scroll_box`: move its content by `step` px (clamped); false = none there or already at
    /// that end.
    fn wheel_box(&mut self, x: f32, y: f32, step: f32, now: f64) -> bool {
        let Some((popup, keys, _, _)) = self.keys_at(x, y, now) else { return false };
        let found = if popup { self.popup.as_ref().and_then(|p| p.scroll_box_in(&keys)) } else { self.layers.get(&self.tab).and_then(|l| l.laid.scroll_box_in(&keys)) };
        let Some((k, max)) = found else { return false };
        let cur = self.st.scroll_y.get(&k).copied().unwrap_or(0.0);
        let to = (cur + step).clamp(0.0, max);
        if (to - cur).abs() < 0.01 {
            return false;
        }
        self.st.scroll_y.insert(k, to);
        let tab = self.tab;
        self.mark_stale(tab);
        self.dirty = true;
        true
    }

    pub fn end_edit(&mut self, save: bool) {
        let tab = self.tab;
        if let Some(l) = self.legacy(tab) {
            l.end_edit(save);
        }
    }
    pub fn is_editing(&self) -> bool {
        self.pages[self.tab].legacy_ref().map(|l| l.is_editing()).unwrap_or(false) || self.st.focus.is_some()
    }
    pub fn char_input(&mut self, ch: char, now: f64) {
        let tab = self.tab;
        if let Some(l) = self.legacy(tab) {
            l.char_input(ch, now);
            self.dirty = true;
            return;
        }
        if let Some(k) = self.st.focus {
            self.page_event(Ev::Char(k, ch), now);
        }
    }
    /// Order 045: the keyboard's tab order of what is in front (the open popup, else the page), by key.
    fn focus_layer(&self) -> (bool, Vec<Key>) {
        if let Some(p) = &self.popup {
            return (true, p.focusables());
        }
        (false, self.layers.get(&self.tab).map(|l| l.laid.focusables()).unwrap_or_default())
    }

    /// Order 045: the focused element is a text field (`Cursor::Text`) of the popup or the page.
    fn focused_is_text(&self, k: Key) -> bool {
        let info = match &self.popup {
            Some(p) if p.rect_of(k).is_some() => p.focus_info(k),
            _ => self.layers.get(&self.tab).and_then(|l| l.laid.focus_info(k)),
        };
        info.is_some_and(|i| i.3)
    }

    /// Order 045: Tab / Shift+Tab: the next / previous control gets the focus (wrapping), shown with the ring and
    /// scrolled into view (`scrollIntoView({block:'nearest'})`, like the browser).
    fn tab_focus(&mut self, back: bool, now: f64) {
        let (popup, list) = self.focus_layer();
        if list.is_empty() {
            return;
        }
        let at = self.st.focus.and_then(|k| list.iter().position(|&x| x == k));
        let n = list.len();
        let i = match (at, back) {
            (Some(i), false) => (i + 1) % n,
            (Some(i), true) => (i + n - 1) % n,
            (None, false) => 0,
            (None, true) => n - 1,
        };
        let k = list[i];
        if let Some(old) = self.st.focus.take() {
            if old != k {
                self.page_event(Ev::Blur(old), now);
            }
        }
        self.st.focus = Some(k);
        self.focus_ring = true;
        if !popup {
            self.want_scroll = Some(WantScroll::Key(k));
        }
        self.dirty = true;
        // a text field takes the focus the way a click gives it (the page starts editing / selects its text, as the
        // browser's focus does)
        let text = if popup { self.popup.as_ref().and_then(|p| p.focus_info(k)) } else { self.layers.get(&self.tab).and_then(|l| l.laid.focus_info(k)) };
        if text.is_some_and(|i| i.3) {
            if let Some(r) = self.rect_of_key(k, popup, now) {
                let (x, y) = (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
                self.page_event(Ev::Press(k, x, y, r), now);
                self.page_event(Ev::Release(k), now);
                self.page_event(Ev::Click(k), now);
                self.st.focus = Some(k);
            }
        }
    }

    /// Order 045: a key on the control reached with Tab that its page did not use: Enter / Space = a click (buttons,
    /// switches, rows: the drawing's `keydown Enter|' '` -> the click), the arrows = a slider step (Left / Down lower).
    fn focus_key(&mut self, k: Key, vk: u16, now: f64) {
        let popup = self.popup.as_ref().is_some_and(|p| p.rect_of(k).is_some());
        let info = if popup { self.popup.as_ref().and_then(|p| p.focus_info(k)) } else { self.layers.get(&self.tab).and_then(|l| l.laid.focus_info(k)) };
        let Some((_, range, click, text)) = info else { return };
        if text {
            return;
        }
        match (vk, range) {
            // (a slider is no button: Enter / Space do nothing on a range input)
            (0x0D | 0x20, None) if click => {
                self.page_event(Ev::Click(k), now);
            }
            (0x25..=0x28, Some((v, step))) => {
                let d = if vk == 0x25 || vk == 0x28 { -step } else { step };
                let Some(r) = self.rect_of_key(k, popup, now) else { return };
                // the page sets a slider from the pointer (`slider::value_at`): press its thumb where it is (a press on the
                // thumb grabs it without a jump), drag it one step, let go - like a short drag by hand
                let at = |v: f32| r.0 + 8.0 + (r.2 - 16.0) * v.clamp(0.0, 1.0);
                let y = r.1 + r.3 / 2.0;
                self.page_event(Ev::Press(k, at(v), y, r), now);
                self.page_event(Ev::Drag(k, at(v + d), y, r), now);
                self.page_event(Ev::Release(k), now);
            }
            _ => {}
        }
    }

    /// Order 045: the focus ring of the control reached with Tab (window coordinates): the drawing's
    /// `box-shadow:0 0 0 3px var(--acc-s)` around the control in its own radius (a slider: around its thumb,
    /// `.rng:focus-visible::-webkit-slider-thumb`). The page's ring is clipped to the page area (under the top row).
    fn focus_ring_el(&self, now: f64) -> Option<El> {
        if !self.focus_ring || self.close_t.is_some() || self.switch.is_some() {
            return None;
        }
        let k = self.st.focus?;
        let popup = self.popup.as_ref().is_some_and(|p| p.rect_of(k).is_some());
        let (radius, range, _, _) = if popup { self.popup.as_ref()?.focus_info(k)? } else { self.layers.get(&self.tab)?.laid.focus_info(k)? };
        let (x, y, w, h) = self.rect_of_key(k, popup, now)?;
        let top = if popup { 0.0 } else { PAGE_TOP };
        let el = focus_ring((x, y - top, w, h), radius, range);
        let mut area = El::block().abs(0.0, top, f32::NAN, f32::NAN).size(WIN_W, WIN_H - top).no_hit().child(el);
        if !popup {
            area = area.clip();
        }
        Some(area)
    }

    /// Keys: returns true when the key was used.
    pub fn key(&mut self, vk: u16, now: f64) -> (bool, Action) {
        const VK_ESCAPE: u16 = 0x1B;
        const VK_TAB: u16 = 0x09;
        self.dirty = true;
        let tab = self.tab;
        if let Some(l) = self.legacy(tab) {
            return l.key(vk, now);
        }
        if vk == VK_ESCAPE && self.modal {
            self.dismiss_popup();
            return (true, Action::None);
        }
        // Order 045: Tab / Shift+Tab walk the controls (the browser's tab order), with the focus ring
        let mods = cx::Mods::now();
        if vk == VK_TAB && !mods.ctrl && !mods.alt {
            // a focused text field with its own Tab order (Network's DNS fields wrap 1 -> 4 -> 1) hears it first
            if let Some(k) = self.st.focus.filter(|k| self.focused_is_text(*k)) {
                if self.page_event(Ev::Key(k, vk), now) {
                    return (true, Action::None);
                }
            }
            self.tab_focus(mods.shift, now);
            return (true, Action::None);
        }
        if let Some(k) = self.st.focus {
            let used = self.page_event(Ev::Key(k, vk), now);
            // Order 045: a control reached with Tab: Enter / Space press it, the arrows move a slider (what the page did
            // not use itself; never in a text field - there the keys type)
            if !used && self.focus_ring && self.st.focus == Some(k) {
                self.focus_key(k, vk, now);
            }
            if vk == VK_ESCAPE {
                self.st.focus = None;
                self.page_event(Ev::Blur(k), now);
            }
            return (true, Action::None);
        }
        // no element has focus: the page itself hears the key (Ctrl+F, Ctrl+A, Delete...); Esc closes the menu unless
        // the page used it (`cx.used = true`)
        let used = self.page_event(Ev::Key(cx::PAGE, vk), now);
        if vk == VK_ESCAPE && !used {
            return (true, Action::Close);
        }
        (used, Action::None)
    }

    /// Test hook: what the menu shows right now, as plain text.
    pub fn describe(&self) -> String {
        let p = &self.pages[self.tab];
        let e = &self.evs;
        // tabs= the top row's tabs (an add-on's joins / leaves it, Order 037)
        let tabs: Vec<&str> = self.pages.iter().map(|p| p.id()).collect();
        format!("tab={} tabs={} open={} {} evs click={} context={} drop={} pagekey={}", p.id(), tabs.join(","), self.close_t.is_none(), p.describe(), e[0], e[1], e[2], e[3])
    }

    /// Test hook: the centre of a named target in window coordinates.
    pub fn target_point(&self, name: &str) -> Option<(f32, f32)> {
        let (k, arg) = name.split_once(':').unwrap_or((name, ""));
        let i: usize = arg.parse().unwrap_or(0);
        match k {
            "dock" => return Some((DS_X - self.ds + dock_x(i) + DT_W / 2.0, DOCK_Y + 17.0)),
            "close" => return Some((577.0, 26.0)),
            "min" => return Some((531.0, 26.0)),
            "chev" => return Some((if i == 0 { 25.0 } else { WIN_W - 97.0 - 12.0 }, 26.0)),
            // a plain point in window DIPs (tests: the pointer resting somewhere, e.g. near the top row's faded edge)
            "xy" => {
                let (x, y) = arg.split_once(',')?;
                return Some((x.parse().ok()?, y.parse().ok()?));
            }
            // el:<key name> or el:<key name>/<part> (= el::sub(key, part); a list's item i = "<name>/<i>")
            "el" => {
                let key = match arg.split_once('/') {
                    Some((n, part)) => el::sub(el::key(n), part),
                    None => el::key(arg),
                };
                let r = self.rect_of_key(key, true, self.last_step).or_else(|| self.rect_of_key(key, false, self.last_step))?;
                return Some((r.0 + r.2 / 2.0, r.1 + r.3 / 2.0));
            }
            _ => {}
        }
        self.pages[self.tab].legacy_ref().and_then(|l| l.target_point(name))
    }
}

thread_local! {
    /// a painter for text widths while handling input (layout-only; same fonts and shaping as the window's painter)
    static DUMMY_G: std::rc::Rc<Gfx> = std::rc::Rc::new(Gfx::new(1.0));
}

/// The menu closed (the Ui goes with the menu window): the shown page gets its `close` (pages/mod.rs: "when the tab is
/// left or the menu closes") before it is dropped, so a page can stop what it started (a sampler) in one place.
impl Drop for Ui {
    fn drop(&mut self) {
        let tab = self.tab;
        // (a page switch still running: its old page is open too)
        if let Some((from, _, _)) = self.switch.take() {
            if from != tab {
                self.pages[from].close();
            }
        }
        if let Some(p) = self.pages.get_mut(tab) {
            p.close();
        }
    }
}

// =================================================================================================== drawing
impl Ui {
    fn open_age(&self, now: f64) -> f64 {
        now - self.open_t
    }

    /// When the page's children start fading up after an open: after the last top-row icon popped in.
    fn stag_t0(&self) -> f64 {
        50.0 + self.n() as f64 * 25.0 + 20.0
    }

    /// The content blur (CSS `filter: blur()` on the top row, page and caption buttons) for this frame.
    pub fn content_blur(&self, now: f64) -> f32 {
        if self.rm {
            return 0.0;
        }
        if let Some(t) = self.close_t {
            let p = ((now - t) / 160.0).clamp(0.0, 1.0);
            return (2.0 * anim::EASE_IN.ease(p)) as f32;
        }
        let p = (self.open_age(now) / 280.0).clamp(0.0, 1.0);
        (6.0 * (1.0 - anim::EASE_OUT_CSS.ease(p))) as f32
    }

    /// The flyout's own motion: (translateY, opacity).
    pub fn window_motion(&self, now: f64) -> (f32, f32) {
        if let Some(t) = self.close_t {
            let d = if self.rm { 120.0 } else { 160.0 };
            let p = ((now - t) / d).clamp(0.0, 1.0);
            if self.rm {
                return (0.0, 1.0 - anim::EASE.ease(p) as f32);
            }
            let e = anim::ACCEL.ease(p) as f32;
            return (8.0 * e, 1.0 - e);
        }
        let a = self.open_age(now);
        if self.rm {
            return (0.0, anim::EASE.ease((a / 160.0).clamp(0.0, 1.0)) as f32);
        }
        (anim::open_rise(a) as f32, anim::EASE_OUT_CSS.ease((a / 190.0).clamp(0.0, 1.0)) as f32)
    }

    /// While a page switch runs: the two pages as (tab, translateX, opacity) - like the drawing, whose pages move as
    /// composited layers, so their content is painted once and only moved / faded per frame.
    pub fn page_motion(&self, now: f64) -> Option<[(usize, f32, f32); 2]> {
        let (from, t0, dir) = self.switch.filter(|s| now - s.1 < 300.0)?;
        let age = now - t0;
        let p = (age / 130.0).clamp(0.0, 1.0);
        let e = anim::ACCEL.ease(p) as f32;
        let out = (from, if self.rm { 0.0 } else { -10.0 * dir * e }, 1.0 - e);
        let delay = if self.rm { 0.0 } else { 40.0 };
        let q = ((age - delay) / 240.0).clamp(0.0, 1.0);
        let e = anim::EASE_OUT.ease(q) as f32;
        let inn = (self.tab, if self.rm { 0.0 } else { 14.0 * dir * (1.0 - e) }, if q > 0.0 { e } else { 0.0 });
        Some([out, inn])
    }

    /// The open motion of the page's i-th child: (opacity, translateY).
    fn stag(&self, i: usize, now: f64, current: bool) -> (f32, f32) {
        if self.rm || self.close_t.is_some() || !current {
            return (1.0, 0.0);
        }
        let p = ((self.open_age(now) - self.open_hold - self.stag_t0() - i as f64 * 26.0) / 300.0).clamp(0.0, 1.0);
        let e = anim::EASE_OUT.ease(p) as f32;
        (e, 6.0 * (1.0 - e))
    }

    /// Build (if needed) and lay out a page's boxes.
    fn ensure_layer(&mut self, g: &Gfx, tab: usize, now: f64) {
        if self.pages[tab].legacy_ref().is_some() {
            return;
        }
        let opening = now - self.open_t < self.open_len() && self.close_t.is_none() && tab == self.tab;
        let live = self.live_dirty && tab == self.tab;
        let stale = self.dirty || live || self.st.busy || opening || self.layers.get(&tab).is_none_or(|l| l.stale);
        // built already in this frame (the frame asks before it rasters, then its passes ask again): the same boxes
        // (Order 041: two or three builds a frame before)
        if !stale || self.layers.get(&tab).is_some_and(|l| l.built_at == now && !l.stale && !live) {
            return;
        }
        let tb = crate::timing::now();
        if tab == self.tab {
            self.live_dirty = false;
        }
        if tab == self.tab {
            self.st.busy = false;
            self.st.wake = None;
        }
        let page = self.pages[tab].id();
        let mut cx = Cx::new(now, self.rm, g, &mut self.st).for_page(page);
        if tab == self.tab {
            cx.st.watch.clear();
        }
        let kids = self.pages[tab].build(&mut cx);
        let current = tab == self.tab;
        // its requests (a toast when a job is Done, a scroll, a tab) wait for the next update; a page that is only being
        // painted in a switch asks nothing
        if current {
            self.pending.append(&mut cx.reqs);
        }
        let kids: Vec<El> = kids
            .into_iter()
            .enumerate()
            .map(|(i, e)| {
                let (op, dy) = self.stag(i, now, current);
                if op >= 0.999 && dy == 0.0 {
                    e
                } else {
                    e.opacity(op).translate(0.0, dy)
                }
            })
            .collect();
        // the drawing's .pg: 600 wide, padding 2px 26px 18px 26px, block layout
        let root = El::block().w(WIN_W).pad(2.0, 26.0, 18.0, 26.0).children(kids);
        let mut laid = Laid::new(g, root, WIN_W, None);
        // sticky boxes (El::sticky) placed for the scroll they are painted at; update() builds again when it moves
        let sticky_at = if laid.has_sticky() {
            let sy = self.scroll[tab].value(now) as f32;
            laid.apply_sticky(sy);
            Some(sy)
        } else {
            None
        };
        // what changed against the boxes painted so far (the frame rasters only those tiles - Order 041)
        if current {
            let d = match self.layers.get(&tab) {
                Some(old) => damage::diff(&old.laid, &laid, WIN_W),
                None => damage::Damage::Full,
            };
            damage::Damage::add(&mut self.page_damage, d);
        }
        self.layers.insert(tab, PageLayer { laid, built_at: now, stale: false, sticky_at });
        // the content got shorter in this build: the scroll goes back inside it now (and one more frame paints it there),
        // not only at the next input (REVIEW_014_item1_333f84f remark 2)
        if current {
            let m = self.scroll_max() as f64;
            if self.scroll[tab].target() > m {
                self.scroll[tab].jump(m);
                self.dirty = true;
            }
        }
        // the page's popup (window coordinates)
        if tab == self.tab {
            let mut cx = Cx::new(now, self.rm, g, &mut self.st).for_page(page);
            // the page's non-modal overlay first (under its popups), window-fixed, never dismissed by a click beside it
            let mut over: Vec<El> = self.pages[tab].overlay(&mut cx).into_iter().collect();
            let pop = self.pages[tab].popup(&mut cx);
            let was_modal = self.modal;
            self.modal = pop.is_some() || self.review.as_ref().is_some_and(|r| r.tab == tab);
            // a popup / menu opened: the tip goes (the drawing's tipHide on every menu open)
            if self.modal && !was_modal {
                let hover = cx.st.hover.clone();
                self.tips.hide(now, &hover);
            }
            over.extend(pop);
            if let Some(rs) = self.review.as_ref().filter(|r| r.tab == tab) {
                let r = &rs.review;
                // (Settings › Reset: each tab's lines under its title)
                let lines: Vec<pieces::reset::Line> = r
                    .lines
                    .iter()
                    .enumerate()
                    .map(|(i, l)| {
                        let first = i == 0 || r.lines[i - 1].page_title != l.page_title;
                        let heading = (r.all && first).then(|| l.page_title.clone());
                        pieces::reset::Line { title: l.label.clone(), from: l.from.text.clone(), to: l.to.text.clone(), ticked: l.ticked, heading }
                    })
                    .collect();
                let n = r.ticked();
                let buttons = vec![
                    pieces::button::cbtn_sized(&mut cx, el::sub(K_REVIEW, "no"), "Cancel", pieces::button::Kind::Ghost, pieces::button::MCFB, false, 0.0),
                    pieces::button::cbtn_sized(&mut cx, el::sub(K_REVIEW, "go"), &r.button_text(), pieces::button::Kind::Red, pieces::button::MCFB, n == 0, 0.0),
                ];
                over.push(pieces::reset::review_popup(&mut cx, K_REVIEW, rs.x, rs.y, &r.title(), r.subline(), &lines, buttons));
            }
            if let Some((t, at)) = &self.toast {
                over.push(pieces::toast::toast(&mut cx, K_TOAST, t, *at, self.pages[tab].bar_shown()));
            }
            self.pending.append(&mut cx.reqs);
            self.popup = if over.is_empty() { None } else { Some(Laid::new(g, El::block().w(WIN_W).h(WIN_H).no_hit().children(over), WIN_W, Some(WIN_H))) };
            self.st.sweep();
            self.resolve_scroll(now);
        }
        let _ = self.layers.get(&tab).map(|l| l.built_at);
        BUILD_MS.with(|b| b.set(b.get() + crate::timing::now() - tb));
    }

    /// One page at rest (clipped to the page's scroll box) - the switch animation's layer content.
    pub fn draw_one_page(&mut self, f: &Frame, tab: usize) {
        let g = f.g;
        self.ensure_layer(g, tab, f.now);
        g.push_clip(0.0, PAGE_TOP, WIN_W, PAGE_H);
        self.draw_page(f, tab, tab == self.tab);
        g.pop_clip();
    }

    /// The page layer: the current page (clipped to the page's scroll box), or both pages of a running switch.
    pub fn draw_pages(&mut self, f: &Frame) {
        let g = f.g;
        let now = f.now;
        let tab = self.tab;
        self.ensure_layer(g, tab, now);
        g.push_clip(0.0, PAGE_TOP, WIN_W, PAGE_H);
        if let Some(m) = self.page_motion(now) {
            for (t, dx, op) in m {
                if op <= 0.001 {
                    continue;
                }
                self.ensure_layer(g, t, now);
                let t0 = g.transform();
                g.set_transform(&(Matrix3x2::translation(dx, 0.0) * t0));
                g.push_layer(op, None);
                self.draw_page(f, t, t == tab);
                g.pop_layer();
                g.set_transform(&t0);
            }
        } else {
            self.draw_page(f, tab, true);
        }
        g.pop_clip();
    }

    fn draw_page(&self, f: &Frame, tab: usize, current: bool) {
        let now = f.now;
        if let Some(l) = self.pages[tab].legacy_ref() {
            l.draw(f, &|i| self.stag(i, now, current));
            return;
        }
        if let Some(pl) = self.layers.get(&tab) {
            let dy = match (tab == self.tab, self.page_dy) {
                (true, Some(dy)) => dy,
                (true, None) => PAGE_TOP - self.page_scroll(now),
                _ => PAGE_TOP,
            };
            pl.laid.paint(f.g, f.icons, 0.0, dy, None);
        }
    }

    // ---- Order 041: the frame's tile cache of the shown page (menu.rs `Band`)
    /// The shown page is made of boxes (not a legacy page drawn by its own code) and they are built.
    pub fn laid_page(&self) -> bool {
        self.pages[self.tab].legacy_ref().is_none() && self.layers.contains_key(&self.tab)
    }
    /// Forget the top row's kept icon pictures (the frame's self-check paints everything from scratch).
    pub fn clear_dock_cache(&mut self) {
        self.dock_icons.clear();
    }
    /// Does the shown page paint anything in the live pass (meters, a legacy page)?
    pub fn page_has_live(&self) -> bool {
        self.pages[self.tab].legacy_ref().is_some() || self.layers.get(&self.tab).is_some_and(|l| l.laid.has_live())
    }
    /// Build the shown page's boxes now if they are due (before the frame decides what to raster).
    pub fn prepare_page(&mut self, g: &Gfx, now: f64) {
        if self.pages[self.tab].legacy_ref().is_none() {
            self.ensure_layer(g, self.tab, now);
        }
    }
    /// What changed in the shown page since the last call (None = nothing).
    pub fn take_page_damage(&mut self) -> Option<damage::Damage> {
        self.page_damage.take()
    }
    /// The shown page's content at the content's own origin (0, 0), unclipped - the tile cache's raster.
    pub fn paint_page_content(&self, f: &Frame) {
        if let Some(pl) = self.layers.get(&self.tab) {
            pl.laid.paint(f.g, f.icons, 0.0, 0.0, None);
        }
    }
    /// Everything the top row's picture is made of right now (`draw_dock`), or None when it must be painted anyway
    /// (opening): the same value = the same picture.
    pub fn dock_sig(&self, now: f64) -> Option<u64> {
        use std::hash::Hasher;
        let age = self.open_age(now);
        let opening = self.close_t.is_none() && age < 700.0 + self.n() as f64 * 25.0 && !self.rm;
        if opening {
            return None;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let mut f = |v: f64| h.write_u64(v.to_bits());
        f(self.ds as f64);
        for t in &self.fades {
            f(t.value(now));
        }
        f(self.dot.value(now));
        for i in 0..self.n() {
            f(self.mag[i].value(now));
            f(self.press_s[i].value(now));
            f(self.on_t[i].value(now));
            f(self.ico_h[i].value(now));
            let ap = now - self.appear[i];
            f(if ap < 420.0 { ap } else { -1.0 });
            let b = now - self.bounce[i];
            f(if b < 420.0 { b } else { -1.0 });
        }
        for p in &self.pages {
            h.write(p.icon().as_bytes());
            h.write(p.name().as_bytes());
        }
        h.write_u8(is_light() as u8);
        Some(h.finish())
    }

    pub fn popup_open(&self) -> bool {
        self.popup.is_some() || self.tip_shown || (self.focus_ring && self.st.focus.is_some()) || self.pages[self.tab].legacy_ref().map(|l| l.popup_open()).unwrap_or(false)
    }

    /// The ONE shared tooltip (`.wtip`, ui/pieces/tip.rs) for the element under the pointer that carries `El::tip` - in
    /// the open popup first, then the page (the owner Oct 8: "a lot of tooltips ... just don't work" - the frame never ran
    /// them). Called once per frame before the popup is drawn.
    pub fn update_tips(&mut self, g: &Gfx, now: f64) {
        let sy = self.page_scroll(now);
        let mut hover: Vec<Key> = if self.mouse_in && self.close_t.is_none() { self.st.hover.clone() } else { Vec::new() };
        // Order 045: the caption buttons' hover names (`capBtn(..)`: `title:label` - "Minimize" / "Close"); the frame paints
        // them, so their boxes are given here
        const K_CMIN: Key = el::key("cap.min");
        const K_CCLS: Key = el::key("cap.cls");
        if self.mouse_in && self.close_t.is_none() {
            let (mx, my) = self.mouse;
            match self.hit(mx, my, now) {
                Hit::CapMin => hover.insert(0, K_CMIN),
                Hit::CapClose => hover.insert(0, K_CCLS),
                _ => {}
            }
        }
        let caps = Laid::new(
            g,
            El::block()
                .w(WIN_W)
                .h(52.0)
                .child(El::block().abs(508.0, 0.0, f32::NAN, f32::NAN).size(46.0, 52.0).key(K_CMIN).title("Minimize"))
                .child(El::block().abs(554.0, 0.0, f32::NAN, f32::NAN).size(46.0, 52.0).key(K_CCLS).title("Close")),
            WIN_W,
            Some(52.0),
        );
        let mut ls: Vec<(&Laid, (f32, f32))> = vec![(&caps, (0.0, 0.0))];
        if let Some(p) = &self.popup {
            ls.push((p, (0.0, 0.0)));
        }
        if self.switch.is_none() {
            if let Some(pl) = self.layers.get(&self.tab) {
                ls.push((&pl.laid, (0.0, PAGE_TOP - sy)));
            }
        }
        self.tips_busy = self.tips.update(g, &ls, &hover, now, WIN_W);
        self.tip_shown = self.tips.el(now).is_some();
    }

    /// The drawing's `tipHide()` (a scroll, a click, a popup, a tab switch, the pointer leaving the window).
    fn hide_tip(&mut self, now: f64) {
        let hover = self.st.hover.clone();
        self.tips.hide(now, &hover);
    }

    /// The caption buttons (painted in the window's own layer in the drawing).
    pub fn draw_caps(&self, f: &Frame) {
        let g = f.g;
        let now = f.now;
        let hm = self.cap_h[0].value(now) as f32;
        let hc = self.cap_h[1].value(now) as f32;
        let pm = self.cap_p[0].value(now) as f32;
        let pc = self.cap_p[1].value(now) as f32;
        // the caption group clips to the window's top-right corner (overflow hidden, border-top-right-radius 14)
        g.push_clip_rr4(508.0, 0.0, 92.0, 52.0, [0.0, RADIUS, 0.0, 0.0]);
        // minimize: hover = hov, pressed = ctl; close: hover #c42b1c, pressed #b22a1b
        let bg = cmix(HOV().mul_a(hm), CTL(), pm);
        g.fill_rect(508.0, 0.0, 46.0, 52.0, bg);
        let cbg = cmix(Rgba::hex(0xc42b1c).mul_a(hc), Rgba::hex(0xb22a1b), pc);
        g.fill_rect(554.0, 0.0, 46.0, 52.0, cbg);
        // glyphs: 10 px, 1 px stroke (butt caps); minimize is crisp (`shape-rendering: crispEdges`)
        f.icons.draw(g, "wmin", 508.0 + 18.0, 21.0, 10.0, 1.0, FG(), &|_| 1.0);
        let col = cmix(FG(), WHITE, hc);
        f.icons.draw(g, "wcls", 554.0 + 18.0, 21.0, 10.0, 1.0, col, &|_| 1.0);
        g.pop_clip();
    }

    /// The top row: the icons in their sideways-scrolling strip (each icon its own composited layer, `.mag`
    /// will-change: transform) under the strip's edge mask and the accent dot. The chevrons and the name label are
    /// `draw_dock_overlays`, the page's glass scrollbar `draw_page_scrollbar` (both frost the frame so far).
    pub fn draw_dock(&mut self, f: &Frame) {
        let g = f.g;
        let now = f.now;
        let age = self.open_age(now);
        let opening = self.close_t.is_none() && age < 700.0 + self.n() as f64 * 25.0 && !self.rm;
        let n = self.n();
        let ds = self.ds;
        // the strip: overflow hidden + mask-image (transparent cl, ramp fl ... ramp fr, transparent cr)
        let (fl, fr, cl, cr) = (self.fades[0].value(now) as f32, self.fades[1].value(now) as f32, self.fades[2].value(now) as f32, self.fades[3].value(now) as f32);
        let strip = sk::Rect::from_xywh(DS_X, 0.0, DS_W, 52.0);
        g.cv().save_layer(&sk::canvas::SaveLayerRec::default().bounds(&strip));
        g.push_clip(DS_X, 0.0, DS_W, 52.0);
        for i in 0..n {
            let (mut op, mut sc) = (1.0f32, 1.0f32);
            if opening {
                let p = ((age - 50.0 - i as f64 * 25.0) / 300.0).clamp(0.0, 1.0);
                // opacity 0 -> 1 and scale .9 -> 1 on the same springy curve (opacity clamps at 1); before its delay: hidden
                let e = anim::SPRING_POP.ease(p) as f32;
                op = if p <= 0.0 { 0.0 } else { e.clamp(0.0, 1.0) };
                sc = 0.9 + 0.1 * e;
            }
            // an add-on's icon that just joined: opacity 0 -> 1 and scale .6 -> 1, 420 ms cubic-bezier(.3,1.35,.5,1)
            // (addons-v1 adDone; opacity clamps at 1)
            let ap = now - self.appear[i];
            if ap < 420.0 {
                let e = APPEAR.ease((ap / 420.0).clamp(0.0, 1.0)) as f32;
                op *= e.clamp(0.0, 1.0);
                sc *= 0.6 + 0.4 * e;
            }
            if op <= 0.0 {
                continue;
            }
            let bx = DS_X - ds + dock_x(i);
            if bx + DT_W + 12.0 < DS_X || bx - 12.0 > DS_X + DS_W {
                continue;
            }
            let m = self.mag[i].value(now) as f32 * self.press_s[i].value(now) as f32;
            let b = if now - self.bounce[i] < 420.0 { anim::bounce(now - self.bounce[i]) as f32 } else { 0.0 };
            // a composited layer sits on a whole pixel
            let bxr = (bx * g.scale).round() / g.scale;
            // the icon is painted at its own origin (x 0 = its left edge) and placed at bxr - a whole device pixel - so
            // a sideways move of the strip only places the same picture elsewhere
            let (cx, cy) = (DT_W / 2.0, DOCK_Y + 17.0);
            let s = sc * m;
            let t0 = g.transform();
            let tr = Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: cx - cx * s, M32: cy - cy * s + b * sc * self.mag[i].value(now) as f32 };
            let on = self.on_t[i].value(now) as f32;
            let lb = if on > 0.0 { sk::Rect::from_xywh(-10.0, DOCK_Y - 9.0, 52.0, 52.0) } else { sk::Rect::from_xywh(0.0, DOCK_Y, DT_W, 34.0) };
            let hv = self.ico_h[i].value(now) as f32;
            let col = cmix(cmix(ICO(), ICO_H(), hv), ICO_ON(), on);
            let glow_c = ACC_GLOW().mul_a(on);
            // .tile 32 x 34, svg 22 x 22 centred
            let ix = 5.0;
            let iy = DOCK_Y + 6.0;
            let icon_name = self.pages[i].icon();
            let paint_icon = |g: &Gfx| {
                g.push_isolated_in(op, lb);
                // the active icon: filter drop-shadow(0 0 5px acc-glow) (Blink keeps a drop-shadow blur in sigma form)
                let glow = on > 0.0;
                if glow {
                    if let Some(fx) = sk::image_filters::drop_shadow((0.0, 0.0), (5.0, 5.0), glow_c.c4(), None, None, None) {
                        g.push_filter_in(fx, sk::Rect::from_xywh(ix, iy, 22.0, 22.0));
                    }
                }
                f.icons.draw_ex(g, icon_name, ix, iy, 22.0, 1.5, col, &|_| 1.0, false);
                if glow {
                    g.pop_filter();
                }
                g.pop_layer();
            };
            // Order 041: each icon is its own layer (like Chromium's `.mag` layers): rastered once per look and kept, then
            // only placed - a tab switch, a hover or the strip sliding rasters the one or two icons whose look changes, not
            // all 17 every frame
            let gs = g.scale;
            let ox = (bxr * gs).round() as i32;
            let (bl, bt, br, bb) = {
                let q = |x: f32, y: f32| (x * tr.M11 + y * tr.M21 + tr.M31, x * tr.M12 + y * tr.M22 + tr.M32);
                let (pad, r) = (18.0, lb);
                let p = [q(r.left - pad, r.top - pad), q(r.right + pad, r.top - pad), q(r.left - pad, r.bottom + pad), q(r.right + pad, r.bottom + pad)];
                let l = p.iter().map(|v| v.0).fold(f32::MAX, f32::min);
                let t = p.iter().map(|v| v.1).fold(f32::MAX, f32::min);
                let rr = p.iter().map(|v| v.0).fold(f32::MIN, f32::max);
                let bm = p.iter().map(|v| v.1).fold(f32::MIN, f32::max);
                ((l * gs).floor() as i32, (t * gs).floor() as i32, (rr * gs).ceil() as i32, (bm * gs).ceil() as i32)
            };
            let sig = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                let fl = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<u32>>();
                fl(&[op, tr.M11, tr.M12, tr.M21, tr.M22, tr.M31, tr.M32, on, gs, col.0, col.1, col.2, col.3, glow_c.0, glow_c.1, glow_c.2, glow_c.3]).hash(&mut h);
                fl(&[lb.left, lb.top, lb.right, lb.bottom]).hash(&mut h);
                (bl, bt, br, bb, icon_name).hash(&mut h);
                h.finish()
            };
            let cached = self.dock_icons.get(&i).filter(|c| c.0 == sig).map(|c| c.1.clone());
            let img = cached.or_else(|| {
                let mut surf = g.surface_like((br - bl).max(1), (bb - bt).max(1))?;
                surf.canvas().clear(sk::Color::TRANSPARENT);
                g.begin(surf.canvas());
                g.set_transform(&(tr * Matrix3x2::translation(-bl as f32 / gs, -bt as f32 / gs)));
                paint_icon(g);
                g.end();
                let img = surf.image_snapshot();
                self.dock_icons.insert(i, (sig, img.clone()));
                Some(img)
            });
            match img {
                Some(img) => {
                    let c = g.cv();
                    c.save();
                    c.reset_matrix();
                    c.draw_image(&img, ((bl + ox) as f32, bt as f32), None);
                    c.restore();
                }
                None => {
                    g.set_transform(&(tr * Matrix3x2::translation(bxr, 0.0) * t0));
                    paint_icon(g);
                    g.set_transform(&t0);
                }
            }
        }
        // the accent dot: inside .dock (it scrolls and fades with the strip), 3 px under the active icon
        {
            let mut dop = 1.0f32;
            if opening {
                let p = ((age - 50.0 - n as f64 * 25.0) / 260.0).clamp(0.0, 1.0);
                dop = anim::EASE_OUT_CSS.ease(p) as f32;
            }
            let dx = DS_X - ds + self.dot.value(now) as f32;
            g.fill_circle(dx, DOCK_Y + 34.0 + 3.0 + 2.0, 2.0, ACC().mul_a(dop));
        }
        g.pop_clip();
        // the mask: alpha 0 -> 1 over the fades (CSS mask-image: linear-gradient(90deg, ...))
        if fl + fr + cl + cr > 0.0 {
            let w = DS_W;
            let stops = [
                (cl / w, Rgba(0.0, 0.0, 0.0, 0.0)),
                ((cl + fl) / w, Rgba(0.0, 0.0, 0.0, 1.0)),
                ((w - cr - fr) / w, Rgba(0.0, 0.0, 0.0, 1.0)),
                ((w - cr) / w, Rgba(0.0, 0.0, 0.0, 0.0)),
            ];
            let mut st: Vec<(f32, Rgba)> = stops.iter().map(|(p, c)| (p.clamp(0.0, 1.0), *c)).collect();
            st.insert(0, (0.0, Rgba(0.0, 0.0, 0.0, 0.0)));
            st.push((1.0, Rgba(0.0, 0.0, 0.0, 0.0)));
            let shd = g.hgrad(DS_X, 0.0, DS_X + w, 0.0, &st);
            let mut p = sk::Paint::default();
            p.set_shader(shd);
            p.set_blend_mode(sk::BlendMode::DstIn);
            g.cv().draw_rect(strip, &p);
        }
        g.cv().restore();
    }

    /// The top row's chevrons and the shared name label (they frost `base`, the frame so far): painted every frame they
    /// show, on their own, so the top row's picture itself is kept (Order 041).
    pub fn dock_overlays_shown(&self, now: f64) -> bool {
        self.chev_on.iter().any(|t| t.value(now) > 0.001) || self.lbl_t.value(now) > 0.001 || self.lbl_i.is_some()
    }

    /// Everything the overlays (`draw_dock_overlays` + `draw_page_scrollbar`) are painted from besides the pixels under
    /// them: the same value and the same pixels = the same picture (Order 041).
    pub fn overlay_sig(&mut self, now: f64) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let geom = self.sb_geom(now).map(|(a, b, c, d)| [a, b, c, d].map(f32::to_bits));
        geom.hash(&mut h);
        let mut v = vec![self.sb_on.value(now), self.sb_h.value(now), self.page_scroll(now) as f64, self.scroll_max() as f64, self.lbl_t.value(now), self.ds as f64];
        for k in 0..2 {
            v.extend([self.chev_on[k].value(now), self.chev_h[k].value(now), self.chev_p[k].value(now)]);
        }
        v.iter().map(|x| x.to_bits()).collect::<Vec<u64>>().hash(&mut h);
        (self.sb_drag.is_some(), self.lbl_shown, self.lbl_i, self.lbl_t.target().to_bits(), now - self.lbl_since >= 120.0, is_light(), self.tab).hash(&mut h);
        h.finish()
    }

    pub fn draw_dock_overlays(&mut self, f: &Frame, base: &sk::Image) {
        let g = f.g;
        let now = f.now;
        let n = self.n();
        let ds = self.ds;
        // chevrons: round glass buttons (24 px, top 14; left 13 / right 97), opacity .18 s, scale .86 -> 1 (.22 s)
        for k in 0..2 {
            let on = self.chev_on[k].value(now) as f32;
            if on <= 0.001 {
                continue;
            }
            let x = if k == 0 { 13.0 } else { WIN_W - 97.0 - 24.0 };
            let y = 14.0;
            let hv = self.chev_h[k].value(now) as f32;
            let pr = self.chev_p[k].value(now) as f32;
            let s = (0.86 + 0.14 * anim::EASE_OUT.ease(on as f64) as f32) * (1.0 - 0.08 * pr);
            let (cx, cy) = (x + 12.0, y + 12.0);
            let t0 = g.transform();
            g.set_transform(&(Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: cx - cx * s, M32: cy - cy * s } * t0));
            // light (Order 033): `#sw.light .dchev{color:rgba(30,30,36,.8);background:linear-gradient(180deg,rgba(255,255,255,.85),
            // rgba(255,255,255,.55));box-shadow:inset 0 .5px 0 #fff,0 0 0 .5px rgba(0,0,0,.14),0 3px 10px rgba(0,0,0,.14)}` - it beats
            // `.dchev:hover`, so no hover change
            let light = is_light();
            let sa = if light { 0.14 } else { 0.28 };
            let shs = [sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, sa)), sh(0.0, 3.0, 10.0, 0.0, Rgba(0.0, 0.0, 0.0, sa))];
            g.push_isolated_in(on, g.ink(&[(x, y, 24.0, 24.0, &shs[..])]));
            g.box_shadows(x, y, 24.0, 24.0, 12.0, &shs, false);
            g.backdrop(base, x, y, 24.0, 24.0, 12.0, 14.0, &[CssColor::Saturate(1.7)]);
            let (a0, a1) = if light { (0.85, 0.55) } else { (0.2 + 0.08 * hv, 0.07 + 0.04 * hv) };
            let shd = g.hgrad(0.0, y, 0.0, y + 24.0, &[(0.0, Rgba(1.0, 1.0, 1.0, a0)), (1.0, Rgba(1.0, 1.0, 1.0, a1))]);
            g.fill_rr_shader(x, y, 24.0, 24.0, 12.0, &shd, 1.0);
            if light {
                g.inset_shadows(x, y, 24.0, 24.0, 12.0, &[sh(0.0, 0.5, 0.0, 0.0, WHITE)]);
            } else {
                g.inset_shadows(x, y, 24.0, 24.0, 12.0, &[sh(0.0, 0.5, 0.0, 0.0, Rgba(1.0, 1.0, 1.0, 0.45)), sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.22))]);
            }
            let col = if light { Rgba::rgba(30, 30, 36, 0.8) } else { cmix(Rgba(1.0, 1.0, 1.0, 0.82), WHITE, hv) };
            f.icons.draw(g, if k == 0 { "dchevL" } else { "dchevR" }, x + 6.0, y + 6.0, 12.0, 1.7, col, &|_| 1.0);
            g.pop_layer();
            g.set_transform(&t0);
        }
        // the shared name label: under the icon, kept 8 px inside the window
        if let Some(i) = self.lbl_i {
            if now - self.lbl_since >= 120.0 && self.lbl_t.target() < 0.5 {
                self.lbl_t.set(now, 1.0, 120.0, anim::EASE);
                self.lbl_shown = i;
            }
        }
        let t = self.lbl_t.value(now) as f32;
        if t > 0.001 {
            let i = self.lbl_shown.min(n - 1);
            let name = self.pages[i].name();
            let tw1 = g.text_width(name, F115B);
            let w = tw1 + 18.0;
            let h = 23.0;
            let icx = DS_X - ds + dock_x(i) + DT_W / 2.0;
            let x = (icx - w / 2.0).clamp(8.0, WIN_W - w - 8.0).round();
            let y = 53.0 + 3.0 * (1.0 - t);
            let shs = [sh(0.0, 6.0, 18.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.28)), sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, 0.3))];
            g.push_isolated_in(t, g.ink(&[(x, y, w, h, &shs[..])]));
            g.box_shadows(x, y, w, h, 7.0, &shs, false);
            g.backdrop(base, x, y, w, h, 7.0, 30.0, &[CssColor::Saturate(1.8)]);
            g.fill_rr(x, y, w, h, 7.0, MENU());
            g.inset_ring(x, y, w, h, 7.0, 0.5, hl());
            g.text(name, F115B, x + 9.0, y + 4.0, 15.0, FG(), Align::Left, 0.0);
            g.pop_layer();
        }
    }

    /// The page's glass scrollbar, painted every frame on its own (Order 041: it moves with the scroll; the top row's
    /// picture does not) - frosting `base`, the frame so far.
    pub fn draw_page_scrollbar(&mut self, f: &Frame, base: &sk::Image) {
        self.draw_scrollbar(f, base);
    }

    /// Does the glass scrollbar show (or fade)?
    pub fn scrollbar_shown(&mut self, now: f64) -> bool {
        self.sb_on.value(now) > 0.001 || self.sb_geom(now).is_some()
    }

    /// The page's glass scrollbar (.gsb): a 7 px glass capsule thumb, its two arrows of the same glass, a faint lane.
    fn draw_scrollbar(&mut self, f: &Frame, base: &sk::Image) {
        let g = f.g;
        let now = f.now;
        let geom = self.sb_geom(now);
        let on = if geom.is_some() { 1.0 } else { 0.0 };
        if (self.sb_on.target() - on).abs() > 1e-6 {
            self.sb_on.set(now, on, 200.0, anim::EASE);
        }
        let op = self.sb_on.value(now) as f32;
        let Some((tt, th_, ty, thh)) = geom else { return };
        if op <= 0.001 {
            return;
        }
        let x = 588.0;
        let hv = self.sb_h.value(now) as f32 + if self.sb_drag.is_some() { 1.0 } else { 0.0 };
        let hv = hv.min(1.0);
        let sy = self.page_scroll(now);
        let max = (self.scroll_max()).max(0.0);
        // light (Order 033): `#sw.light .gsb .ga,#sw.light .gsb .gth{background:linear-gradient(90deg,rgba(60,60,72,.34),
        // rgba(60,60,72,.22) 55%,rgba(60,60,72,.3));box-shadow:inset 0 0 0 .5px rgba(255,255,255,.5),inset 0 .5px 0
        // rgba(255,255,255,.7),0 0 0 .5px rgba(0,0,0,.08)}` - it beats `.gsb:hover`, so no hover change
        let light = is_light();
        // its fade: one layer; shown (opacity 1) it is painted straight (the same pixels as a full-window layer at 1, without
        // allocating and blending one every frame - Order 041)
        let faded = op < 0.999;
        if faded {
            g.push_isolated(op);
        }
        let glass = |gy: f32, gh: f32, dis: bool| {
            let shs: Vec<crate::gfx::Shadow> = if light {
                vec![sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, 0.08))]
            } else {
                vec![sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, 0.18)), sh(0.0, 1.0, 4.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.18))]
            };
            let o = if dis { 0.4 } else { 1.0 };
            if o < 1.0 {
                g.push_layer(o, None);
            }
            g.box_shadows(x, gy, 7.0, gh, 3.5, &shs, false);
            g.backdrop(base, x, gy, 7.0, gh, 3.5, 8.0, &[CssColor::Saturate(1.7)]);
            let hv = if dis { 0.0 } else { hv };
            let a = |lo: f32, hi: f32| Rgba(1.0, 1.0, 1.0, lo + (hi - lo) * hv);
            let (stops, rim) = if light {
                let d = |al: f32| Rgba::rgba(60, 60, 72, al);
                ([(0.0, d(0.34)), (0.55, d(0.22)), (1.0, d(0.3))], [0.5, 0.7])
            } else {
                ([(0.0, a(0.36, 0.5)), (0.55, a(0.2, 0.3)), (1.0, a(0.27, 0.4))], [0.32, 0.5])
            };
            let shd = g.hgrad(x, 0.0, x + 7.0, 0.0, &stops);
            g.fill_rr_shader(x, gy, 7.0, gh, 3.5, &shd, 1.0);
            g.inset_shadows(x, gy, 7.0, gh, 3.5, &[sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, rim[0])), sh(0.0, 0.5, 0.0, 0.0, Rgba(1.0, 1.0, 1.0, rim[1]))]);
            if o < 1.0 {
                g.pop_layer();
            }
        };
        // arrows (13 px), disabled at the ends (.4)
        let up_dis = sy <= 0.0;
        let dn_dis = sy >= max - 1.0;
        glass(60.0, 13.0, up_dis);
        glass(tt + th_ + 4.0, 13.0, dn_dis);
        let arrow = |gy: f32, up: bool, dis: bool| {
            // `.gsb .ga{color:rgba(20,22,30,.62)}` loses to `#sw button{color:inherit}` (1,0,1 > 0,2,0): the arrow is
            // the window's text colour --fg (measured: the drawing's up arrow is light)
            // light: `#sw.light .gsb .ga{color:rgba(255,255,255,.95)}` (1,3,0) beats that
            let c = if light { Rgba(1.0, 1.0, 1.0, 0.95) } else { FG() }.mul_a(if dis { 0.4 } else { 1.0 });
            let (ax, ay) = (x + 1.0, gy + 4.5);
            let p = if up { g.path("M2.5 .4L4.8 3.6H.2z") } else { g.path("M2.5 3.6L4.8 .4H.2z") };
            let t0 = g.transform();
            g.set_transform(&(Matrix3x2::translation(ax, ay) * t0));
            g.fill_geom(&p, c);
            g.set_transform(&t0);
        };
        arrow(60.0, true, up_dis);
        arrow(tt + th_ + 4.0, false, dn_dis);
        // the lane
        // (light: `#sw.light .gsb .gtr{background:rgba(0,0,0,.035)}`)
        g.fill_rr(x, tt, 7.0, th_, 3.5, if light { Rgba(0.0, 0.0, 0.0, 0.035) } else { Rgba(1.0, 1.0, 1.0, 0.035) });
        g.inset_shadows(x, tt, 7.0, th_, 3.5, &[sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.05))]);
        // the thumb
        glass(tt + ty, thh, false);
        if faded {
            g.pop_layer();
        }
    }

    /// The popups (above everything), with their frosted backdrop.
    pub fn draw_popup(&mut self, f: &Frame, base: &sk::Image) {
        let tab = self.tab;
        if let Some(l) = self.legacy(tab) {
            if l.popup_open() {
                l.draw_popup(f, base);
            }
        }
        if let Some(p) = &self.popup {
            p.paint(f.g, f.icons, 0.0, 0.0, Some(base));
        }
        // Order 045: the keyboard focus ring (above the page and its popup, under the tip)
        if let Some(r) = self.focus_ring_el(f.now) {
            Laid::new(f.g, El::block().w(WIN_W).h(WIN_H).no_hit().child(r), WIN_W, Some(WIN_H)).paint(f.g, f.icons, 0.0, 0.0, Some(base));
        }
        // the shared tooltip above everything (z 21, above menus)
        if let Some(b) = self.tips.el(f.now) {
            Laid::new(f.g, El::block().w(WIN_W).h(WIN_H).no_hit().child(b), WIN_W, Some(WIN_H)).paint(f.g, f.icons, 0.0, 0.0, Some(base));
        }
    }

    /// The window's background under the content: the tint (`#sw{background:var(--tint)}` of the glass style chosen in
    /// Settings; Liquid = rgba(20,20,26,.22)).
    pub fn draw_rim(&self, g: &Gfx) {
        let n = glass_numbers();
        let [tr, tg, tb] = n.tint_rgb;
        g.fill_rr(0.0, 0.0, WIN_W, WIN_H, RADIUS, Rgba::rgba(tr, tg, tb, n.tint_alpha));
    }

    /// The window's `::after` layer, ABOVE the content (menu-v22, v19 dark glass): its background = the sheen
    /// linear-gradient(135deg, white .16 0%, 0 32%, 0 72%, .07 100%), then its inset shadows = the bright rim
    /// `inset 0 0 0 1px .22, inset 0 1px 0 .55, inset 0 -1px 0 .14, inset 0 0 24px .06` (Blink paints the last first).
    /// Frosted: `#sw.gl-fro::after{box-shadow:inset 0 0 0 1px .14, inset 0 1px 0 .3; background:linear-gradient(135deg, .07 0%,
    /// 0 40%)}`; Windows look: `#sw.gl-win::after{box-shadow:inset 0 0 0 1px .1; background:none}`.
    pub fn draw_edge(&self, g: &Gfx) {
        if is_light() {
            // the light glass: `#sw::after{box-shadow:inset 0 0 0 1px var(--hl)}` only (the sheen and the bright rim are
            // `#sw:not(.light)`)
            g.inset_shadows(0.0, 0.0, WIN_W, WIN_H, RADIUS, &[sh(0.0, 0.0, 0.0, 1.0, HL_V19())]);
            return;
        }
        let style = glass();
        // CSS 135deg over 600 x 520: the gradient line runs through the centre toward bottom-right, length (w + h) / sqrt 2
        let half = (WIN_W + WIN_H) / 2.0f32.sqrt() / 2.0;
        let k = half / 2.0f32.sqrt();
        let (cx, cy) = (WIN_W / 2.0, WIN_H / 2.0);
        let w0 = Rgba(1.0, 1.0, 1.0, 0.0);
        let w = |a: f32| Rgba(1.0, 1.0, 1.0, a);
        let stops: Option<Vec<(f32, Rgba)>> = match style {
            GlassStyle::Liquid => Some(vec![(0.0, w(0.16)), (0.32, w0), (0.72, w0), (1.0, w(0.07))]),
            GlassStyle::Frosted => Some(vec![(0.0, w(0.07)), (0.4, w0)]),
            GlassStyle::WindowsLook => None,
        };
        if let Some(st) = stops {
            let mut st = st;
            if st.last().map(|l| l.0 < 1.0).unwrap_or(false) {
                st.push((1.0, w0));
            }
            let br = g.hgrad(cx - k, cy - k, cx + k, cy + k, &st);
            g.fill_rr_shader(0.0, 0.0, WIN_W, WIN_H, RADIUS, &br, 1.0);
        }
        let rim: Vec<crate::gfx::Shadow> = match style {
            GlassStyle::Liquid => vec![sh(0.0, 0.0, 0.0, 1.0, w(0.22)), sh(0.0, 1.0, 0.0, 0.0, w(0.55)), sh(0.0, -1.0, 0.0, 0.0, w(0.14)), sh(0.0, 0.0, 24.0, 0.0, w(0.06))],
            GlassStyle::Frosted => vec![sh(0.0, 0.0, 0.0, 1.0, w(0.14)), sh(0.0, 1.0, 0.0, 0.0, w(0.3))],
            GlassStyle::WindowsLook => vec![sh(0.0, 0.0, 0.0, 1.0, w(0.1))],
        };
        g.inset_shadows(0.0, 0.0, WIN_W, WIN_H, RADIUS, &rim);
    }

    pub fn device_names(&self) -> Vec<String> {
        self.pages[0].legacy_ref().map(|l| l.device_names()).unwrap_or_default()
    }
}

thread_local! {
    /// BU_PROF: the time pages spent being built + laid out since the last `take_build_ms` (ms)
    static BUILD_MS: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
}

/// BU_PROF: page build + layout time since the last call (ms).
pub fn take_build_ms() -> f64 {
    BUILD_MS.with(|b| b.replace(0.0))
}

/// The numbers the window's glass is painted with: the chosen style's (dark), or the light glass's (one look for every style).
/// Order 045: the keyboard focus ring for a control's box `r` (x, y, w, h) with its radius: the drawing's `box-shadow:0 0 0
/// 3px var(--acc-s)`; a slider (`range` = its value): around its 16 px thumb.
pub fn focus_ring(r: (f32, f32, f32, f32), radius: f32, range: Option<(f32, f32)>) -> El {
    let (x, y, w, h) = r;
    let ring = |x: f32, y: f32, w: f32, h: f32, rr: f32| El::block().abs(x, y, f32::NAN, f32::NAN).size(w, h).radius(rr).shadow(&[sh(0.0, 0.0, 0.0, 3.0, ACC_S())]).no_hit();
    match range {
        Some((v, _)) => ring(x + (w - 16.0) * v.clamp(0.0, 1.0), y + h / 2.0 - 8.0, 16.0, 16.0, 8.0),
        // a control whose own box is square (a segment, a row's hit box: its look is drawn by a sibling) gets the
        // controls' small rounding, never a sharp box
        None => ring(x, y, w, h, if radius > 0.0 { radius } else { 6.0f32.min(h / 2.0) }),
    }
}

pub fn glass_numbers() -> crate::settings::GlassNumbers {
    if is_light() {
        crate::settings::GlassNumbers::LIGHT
    } else {
        glass().numbers()
    }
}

/// The glass style chosen in Settings (Liquid when the services are not running, e.g. in a unit test).
pub fn glass() -> GlassStyle {
    crate::services::with(|s| s.glass()).unwrap_or_default()
}

/// `--hl` of the glass style (the popups' and small windows' inner rim): Liquid .22, Frosted .14, Windows look .10; light .75.
pub fn hl() -> Rgba {
    Rgba(1.0, 1.0, 1.0, glass_numbers().highlight_alpha)
}

/// The tab with this id or name (the test hook's `--tab`), in the top row's order.
pub fn tab_index(name: &str) -> Option<usize> {
    let n = name.to_lowercase();
    pages::all().iter().filter(|p| crate::addons::tab_visible(p.id())).position(|p| p.id() == n || p.name().to_lowercase() == n)
}

/// Warm the painter up before an open (tray hover): every font of the menu loaded, shaped and drawn once off-screen,
/// so the typefaces, the glyph cache and Skia's code are in memory when the first frame is painted.
pub fn prime(g: &Gfx) {
    let Some(mut s) = crate::gfx::new_surface(64, 64) else { return };
    g.begin(s.canvas());
    for f in [F13, F13BTN, F11, F11M, F125, F20, F115B, F105M, F12B, FTAG] {
        g.text("Audio Output 74 % Microphone (Shure MV7)", f, 0.0, 0.0, 20.0, FG(), Align::Left, 0.0);
    }
    for p in pages::all().iter() {
        let _ = g.text_width(p.name(), F115B);
        let _ = g.text_width(p.name(), F20);
    }
    g.end();
}

/// Drop per-device caches (the menu's device is gone).
pub fn reset_caches() {
    pages::audio::reset_caches();
}

/// Remember a device name's text width (for the popup's width).
pub fn remember_widths(g: &Gfx, names: &[String]) {
    pages::audio::remember_widths(g, names);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Order 033: the dark palette is the dark glass exactly as before the light theme (every token, byte for byte), and the
    /// light one is the drawing's `#sw.light`.
    #[test]
    fn dark_palette_is_unchanged_and_light_switches() {
        let d = &DARK;
        let want = [
            (d.acc, c(10, 132, 255, 1.0)), (d.fg, c(245, 245, 247, 1.0)), (d.fg2, c(235, 235, 245, 0.6)), (d.fg3, c(235, 235, 245, 0.36)),
            (d.grp, c(255, 255, 255, 0.08)), (d.grp_rim, c(255, 255, 255, 0.18)), (d.grp_top, c(255, 255, 255, 0.26)),
            (d.hair, c(255, 255, 255, 0.085)), (d.ctl, c(255, 255, 255, 0.1)), (d.ctl_h, c(255, 255, 255, 0.15)),
            (d.trk, c(120, 120, 128, 0.36)), (d.pill, c(255, 255, 255, 0.22)), (d.sel, c(10, 132, 255, 0.28)),
            (d.menu, c(42, 42, 48, 0.74)), (d.pop, c(36, 37, 43, 0.94)), (d.hov, c(255, 255, 255, 0.06)), (d.red, c(255, 69, 58, 1.0)),
            (d.green, c(48, 209, 88, 1.0)), (d.amber, c(255, 214, 10, 1.0)), (d.key, c(255, 255, 255, 0.16)),
            (d.ico, c(225, 227, 236, 0.66)), (d.ico_h, c(255, 255, 255, 1.0)), (d.ico_on, c(61, 155, 255, 1.0)),
            (d.acc_glow, c(10, 132, 255, 0.55)), (d.acc_s, c(10, 132, 255, 0.45)), (d.vz1, c(10, 132, 255, 1.0)),
            (d.vz2, c(47, 214, 196, 1.0)), (d.lvt, c(255, 255, 255, 0.07)), (d.well, c(0, 0, 0, 0.16)), (d.dash, c(255, 255, 255, 0.2)),
            (d.hl, c(255, 255, 255, 0.12)), (d.hl_v19, c(255, 255, 255, 0.22)),
        ];
        for (i, (got, w)) in want.iter().enumerate() {
            assert_eq!(got, w, "dark token {i}");
        }
        assert!(!is_light());
        assert_eq!(FG(), c(245, 245, 247, 1.0));
        set_light(true);
        assert_eq!((FG(), GRP(), ACC()), (c(29, 29, 31, 1.0), c(255, 255, 255, 0.62), c(0, 122, 255, 1.0)));
        set_light(false);
        assert_eq!(FG(), c(245, 245, 247, 1.0));
    }

    /// A page that asks for things from its BUILD (Settings asks for the exit there, when its update job is Done).
    struct Asker {
        id: &'static str,
        ask: bool,
    }
    impl Page for Asker {
        fn id(&self) -> &'static str {
            self.id
        }
        fn name(&self) -> &'static str {
            self.id
        }
        fn icon(&self) -> &'static str {
            ""
        }
        fn build(&mut self, cx: &mut Cx) -> Vec<El> {
            if std::mem::take(&mut self.ask) {
                cx.toast("Saved");
                cx.show_tab("two", None);
            }
            vec![El::block().h(40.0)]
        }
    }

    /// REVIEW_014_item1c HOLD 1: requests made in `build` were dropped with its Cx; now they wait for the next update.
    #[test]
    fn requests_made_in_build_are_applied() {
        let g = Gfx::new(1.0);
        let pages: Vec<Box<dyn Page>> = vec![Box::new(Asker { id: "one", ask: true }), Box::new(Asker { id: "two", ask: false })];
        let mut ui = Ui::with_pages(pages, true, true, 0.0);
        // (long after the open motion: nothing else keeps frames coming)
        let t = 10_000.0;
        ui.ensure_layer(&g, 0, t);
        assert!(ui.toast.is_none() && ui.tab == 0, "applied only at the next update");
        // the request itself keeps the loop drawing, so that next update comes without any input
        assert!(ui.static_busy(t) && ui.animating(t), "a pending request must ask for one more frame");
        ui.update(t + 16.0);
        assert_eq!(ui.toast.as_ref().map(|t| t.0.as_str()), Some("Saved"));
        assert_eq!(ui.tab, 1, "the tab asked for in build is shown");
        // asked once: nothing more is pending
        ui.ensure_layer(&g, 0, t + 32.0);
        assert!(ui.pending.is_empty());
    }

    /// `exit_for_update` works from a build too: an app-level flag the main loop checks after every wake-up.
    #[test]
    fn exit_for_update_from_build_reaches_the_app() {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st).for_page("set");
        cx.exit_for_update();
        assert!(crate::services::exit_requested());
    }

    /// A tab with a resettable setting and a jump log (Lane V's asks: Settings › Reset over every tab, the menu opened on
    /// a tab by a key).
    struct Tab {
        id: &'static str,
        value: String,
        log: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    }
    impl Page for Tab {
        fn id(&self) -> &'static str {
            self.id
        }
        fn name(&self) -> &'static str {
            self.id
        }
        fn icon(&self) -> &'static str {
            ""
        }
        fn build(&mut self, _cx: &mut Cx) -> Vec<El> {
            vec![El::block().h(40.0)]
        }
        fn jump(&mut self, target: &str) {
            self.log.borrow_mut().push(format!("{} jump {target}", self.id));
        }
        fn resettable(&mut self) -> Option<&mut dyn crate::undo::Resettable> {
            Some(self)
        }
    }
    impl crate::undo::Resettable for Tab {
        fn page_id(&self) -> &str {
            self.id
        }
        fn page_title(&self) -> &str {
            self.id
        }
        fn current(&self, _item: &str) -> Option<crate::undo::Val> {
            Some(crate::undo::Val::plain(&self.value))
        }
        fn apply(&mut self, item: &str, to: &crate::undo::Val) -> Result<(), String> {
            self.value = to.raw.clone();
            self.log.borrow_mut().push(format!("{} {item}={}", self.id, to.raw));
            Ok(())
        }
    }
    fn tabs(log: &std::rc::Rc<std::cell::RefCell<Vec<String>>>) -> Vec<Box<dyn Page>> {
        ["one", "two", "three"].into_iter().map(|id| Box::new(Tab { id, value: "new".into(), log: log.clone() }) as Box<dyn Page>).collect()
    }

    /// Settings › Reset: one review over every tab, grouped (a heading per tab), applied through each tab.
    #[test]
    fn reset_all_reviews_and_applies_every_tab() {
        crate::services::init(windows::Win32::Foundation::HWND::default(), true);
        for id in ["one", "three"] {
            crate::services::with(|s| crate::undo::record(&mut s.store, id, "hz", "Refresh rate", &crate::undo::Val::plain("old"), &crate::undo::Val::plain("new")).unwrap());
        }
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut ui = Ui::with_pages(tabs(&log), true, true, 0.0);
        let _ = ui.apply_reqs(vec![cx::Req::ResetAll(crate::undo::Kind::HowItWas, (10.0, 20.0, 50.0, 16.0))], 0.0);
        let r = &ui.review.as_ref().expect("the review is up").review;
        assert!(r.all);
        assert_eq!(r.groups().iter().map(|g| g.0.as_str()).collect::<Vec<_>>(), ["one", "three"]);
        assert!(ui.review_event(&Ev::Click(el::sub(K_REVIEW, "go")), 0.0));
        assert_eq!(*log.borrow(), ["one hz=old", "three hz=old"]);
        assert!(ui.toast.as_ref().is_some_and(|t| t.0.contains("2 settings reset")), "{:?}", ui.toast);
        crate::services::shutdown();
    }

    /// Order 036: a change noted from another thread lands in the change log; the uninstaller's undo with no window
    /// (`--undo-windows-count` / `--undo-windows`) counts, then puts back every tab's changes through the tabs.
    #[test]
    fn the_uninstallers_undo_counts_then_puts_every_tab_back() {
        use crate::undo::{note, pending, undo_everything, Val};
        crate::services::init(windows::Win32::Foundation::HWND::default(), true);
        // noted while the services are busy (a key handler runs inside them): it waits in the queue for the next flush
        // (unit tests keep one queue per thread - the app's one queue takes every thread's notes)
        crate::services::with(|_| note("two", "dns", "DNS", &Val::plain("old"), &Val::plain("new")));
        assert!(pending(), "a note made while the services are busy waits");
        crate::services::with(|s| crate::undo::record(&mut s.store, "one", "hz", "Refresh rate", &Val::plain("old"), &Val::plain("new")).unwrap());
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut pages = tabs(&log);
        assert_eq!(undo_everything(&mut pages, false), (2, 0), "count only");
        assert!(log.borrow().is_empty(), "counting changes nothing");
        assert_eq!(undo_everything(&mut pages, true), (2, 0));
        assert_eq!(*log.borrow(), ["one hz=old", "two dns=old"]);
        assert_eq!(undo_everything(&mut pages, false), (0, 0), "nothing left to offer");
        crate::services::shutdown();
    }

    /// `services::show_menu` with the menu closed: the menu opens ON that tab (no switch) and the tab gets its target;
    /// with the menu open: the tab is shown (a switch) and gets its target.
    /// A page for the frame's own behaviour: a tipped box at page (40, 20) 60 x 20, a ready flag, a tick.
    struct Probe {
        id: &'static str,
        ready: std::rc::Rc<std::cell::Cell<bool>>,
        ticks: bool,
    }
    impl Page for Probe {
        fn id(&self) -> &'static str {
            self.id
        }
        fn name(&self) -> &'static str {
            self.id
        }
        fn icon(&self) -> &'static str {
            ""
        }
        fn ready(&self) -> bool {
            self.ready.get()
        }
        fn tick(&mut self, _now: f64) -> bool {
            self.ticks
        }
        fn build(&mut self, _cx: &mut Cx) -> Vec<El> {
            vec![El::block().h(20.0), El::block().key(el::key("probe.i")).w(60.0).h(20.0).margin(0.0, 0.0, 0.0, 14.0).tip("Needs admin")]
        }
    }
    /// Order 045: a page with two buttons, a slider and a wheel-stepping field; it logs what it hears.
    struct Kbd {
        log: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
        v: f32,
    }
    const KA: Key = el::key("kbd.a");
    const KS: Key = el::key("kbd.s");
    const KB: Key = el::key("kbd.b");
    const KW: Key = el::key("kbd.w");
    impl Page for Kbd {
        fn id(&self) -> &'static str {
            "kbd"
        }
        fn name(&self) -> &'static str {
            "kbd"
        }
        fn icon(&self) -> &'static str {
            ""
        }
        fn build(&mut self, cx: &mut Cx) -> Vec<El> {
            vec![
                El::block().size(80.0, 30.0).on_click(KA),
                pieces::slider::slider(cx, KS, self.v, 150.0, 20.0, pieces::slider::default()),
                El::block().size(80.0, 30.0).on_click(KB),
                El::block().size(80.0, 30.0).key(KW).wheel_steps(),
            ]
        }
        fn event(&mut self, ev: &Ev, _cx: &mut Cx) {
            let name = |k: Key| match k {
                KA => "a",
                KS => "s",
                KB => "b",
                KW => "w",
                _ => "?",
            };
            match ev {
                Ev::Click(k) => self.log.borrow_mut().push(format!("click {}", name(*k))),
                Ev::Press(k, x, _, r) | Ev::Drag(k, x, _, r) if *k == KS => self.v = pieces::slider::value_at(*r, *x),
                Ev::Wheel(k, d) => self.log.borrow_mut().push(format!("wheel {} {d}", name(*k))),
                Ev::DragOver(k) => self.log.borrow_mut().push(format!("over {}", k.map(name).unwrap_or("-"))),
                _ => {}
            }
        }
        fn describe(&self) -> String {
            format!("v={:.2}", self.v)
        }
    }

    /// Order 045 item 12: Tab / Shift+Tab walk the controls in order (wrapping) with the ring; Enter / Space press the
    /// focused button; the arrows step a focused slider by 1 %; a mouse press ends the ring.
    #[test]
    fn tab_walks_the_controls_and_keys_press_them() {
        let g = Gfx::new(1.0);
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut ui = Ui::with_pages(vec![Box::new(Kbd { log: log.clone(), v: 0.5 })], true, true, 0.0);
        let t = 10_000.0;
        ui.ensure_layer(&g, 0, t);
        cx::Mods::set_event(Some(cx::Mods::default()));
        ui.key(0x09, t);
        assert_eq!(ui.st.focus, Some(KA));
        assert!(ui.focus_ring_el(t).is_some(), "the ring shows");
        ui.key(0x0D, t);
        assert_eq!(*log.borrow(), ["click a"]);
        ui.key(0x09, t);
        assert_eq!(ui.st.focus, Some(KS), "the slider is in the tab order");
        ui.key(0x27, t);
        ui.ensure_layer(&g, 0, t + 16.0);
        assert!(ui.pages[0].describe() == "v=0.51", "{}", ui.pages[0].describe());
        ui.key(0x25, t + 20.0);
        ui.ensure_layer(&g, 0, t + 24.0);
        ui.key(0x25, t + 28.0);
        ui.ensure_layer(&g, 0, t + 32.0);
        assert_eq!(ui.pages[0].describe(), "v=0.49");
        ui.key(0x09, t + 30.0);
        ui.key(0x20, t + 30.0);
        assert_eq!(*log.borrow(), ["click a", "click b"]);
        // past the last control (the wheel field is not one): back to the first; Shift+Tab goes back
        ui.key(0x09, t + 40.0);
        assert_eq!(ui.st.focus, Some(KA));
        cx::Mods::set_event(Some(cx::Mods { shift: true, ..Default::default() }));
        ui.key(0x09, t + 50.0);
        assert_eq!(ui.st.focus, Some(KB));
        cx::Mods::set_event(None);
        // a mouse press: no ring
        let _ = ui.mouse_down(5.0, 300.0, t + 60.0);
        assert!(ui.focus_ring_el(t + 60.0).is_none());
    }

    /// Order 045 items 4 + 6: the wheel over a wheel-stepping field reaches the page (the page does not scroll); files
    /// dragged over an element tell the page once per element, and None when they leave.
    #[test]
    fn the_wheel_steps_a_field_and_a_drag_hover_reaches_the_page() {
        let g = Gfx::new(1.0);
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut ui = Ui::with_pages(vec![Box::new(Kbd { log: log.clone(), v: 0.5 })], true, true, 0.0);
        let t = 10_000.0;
        ui.ensure_layer(&g, 0, t);
        let (x, y, w, h) = ui.rect_of_key(KW, false, t).expect("the field is laid");
        ui.mouse_move(x + w / 2.0, y + h / 2.0, t);
        ui.wheel(120.0, t);
        ui.wheel(-120.0, t);
        assert_eq!(*log.borrow(), ["wheel w 1", "wheel w -1"]);
        let (ax, ay, aw, ah) = ui.rect_of_key(KA, false, t).unwrap();
        ui.drag_over(Some((ax + aw / 2.0, ay + ah / 2.0)), t);
        ui.drag_over(Some((ax + aw / 2.0 + 1.0, ay + ah / 2.0)), t);
        ui.drag_over(None, t);
        assert_eq!(log.borrow()[2..], ["over a", "over -"]);
    }

    fn probes(ready: &std::rc::Rc<std::cell::Cell<bool>>, ticks: bool) -> Vec<Box<dyn Page>> {
        ["one", "two"].into_iter().map(|id| Box::new(Probe { id, ready: ready.clone(), ticks }) as Box<dyn Page>).collect()
    }

    /// the owner Oct 8: "tooltips ... just don't work" - the frame now runs the shared tip: 300 ms after the pointer rests on
    /// a tipped element its bubble shows; a scroll hides it.
    #[test]
    fn a_tipped_element_shows_its_tip_after_300_ms() {
        let g = Gfx::new(1.0);
        let ready = std::rc::Rc::new(std::cell::Cell::new(true));
        let mut ui = Ui::with_pages(probes(&ready, false), true, true, 0.0);
        let t = 10_000.0;
        ui.ensure_layer(&g, 0, t);
        // the box: page x 26 + 14 = 40, y 20 (+ the page top 56 + the .pg top padding 2)
        ui.mouse_move(60.0, PAGE_TOP + 2.0 + 30.0, t);
        ui.update_tips(&g, t);
        assert!(!ui.tip_shown, "not before 300 ms");
        // Order 047: the 300 ms are waited out asleep - no frames, a wake-up when they end
        assert!(!ui.static_busy(t), "no frames while the 300 ms timer runs");
        assert!(ui.wake_at(t).is_some_and(|w| (w - (t + 300.0)).abs() < 1.0), "the menu wakes when the 300 ms end: {:?}", ui.wake_at(t));
        ui.update_tips(&g, t + 310.0);
        assert!(ui.tip_shown && ui.popup_open(), "the bubble shows");
        ui.wheel(-120.0, t + 400.0);
        ui.update_tips(&g, t + 600.0);
        assert!(!ui.tip_shown, "a scroll hides it");
    }

    /// the owner Oct 8: settings "snap to the new settings when the page is fully loaded" - a tab that is not ready holds the
    /// switch at its start (the old page stays) for at most 0.4 s; its transitions jump meanwhile.
    #[test]
    fn a_tab_that_is_not_ready_holds_the_switch() {
        let ready = std::rc::Rc::new(std::cell::Cell::new(true));
        let mut ui = Ui::with_pages(probes(&ready, false), false, true, 0.0);
        let t = 10_000.0;
        ui.update(t);
        ready.set(false);
        ui.show_tab(1, t);
        for i in 1..=5 {
            ui.update(t + 16.0 * i as f64);
        }
        let t0 = ui.switch.unwrap().1;
        assert!(t0 >= t + 60.0, "held: the switch starts later ({t0})");
        assert!(ui.st.settle_until > t + 80.0, "transitions jump while held");
        ready.set(true);
        ui.update(t + 100.0);
        assert_eq!(ui.switch.unwrap().1, t0, "ready: it runs");
        // never ready: shown anyway after HOLD_MS
        ready.set(false);
        ui.show_tab(0, t + 1_000.0);
        let mut now = t + 1_000.0;
        while now < t + 1_000.0 + 2.0 * HOLD_MS {
            now += 16.0;
            ui.update(now);
        }
        assert!(ui.switch.is_none(), "the switch ended although the page never said ready");
    }

    /// Feedback F4: a ticking page with no live boxes (a stopwatch's digits) has its static layer painted again.
    #[test]
    fn a_ticking_page_without_live_boxes_repaints() {
        let g = Gfx::new(1.0);
        let ready = std::rc::Rc::new(std::cell::Cell::new(true));
        let mut ui = Ui::with_pages(probes(&ready, true), true, true, 0.0);
        let t = 10_000.0;
        ui.ensure_layer(&g, 0, t);
        ui.dirty = false;
        ui.update(t + 16.0);
        assert!(ui.dirty, "the static layer must be painted again");
    }

    /// The menu sits behind another app (F1: it no longer closes): it is still on the screen, so a ticking page keeps
    /// asking for frames (the owner Oct 8 test 2: timers / progress bars move while the app is not focused).
    #[test]
    fn behind_another_app_it_keeps_ticking() {
        let ready = std::rc::Rc::new(std::cell::Cell::new(true));
        let mut ui = Ui::with_pages(probes(&ready, true), true, true, 0.0);
        let t = 10_000.0;
        ui.update(t);
        assert!(ui.animating(t + 16.0), "the ticking page wants frames");
        ui.update(t + 32.0);
        assert!(ui.animating(t + 48.0), "and keeps wanting them");
    }

    /// Order 047 (~1.7 cores with the menu open): at rest - the open motion over, nothing hovered or
    /// moving, a page that does not tick - the menu asks for NO frames and has nothing to wake for.
    #[test]
    fn at_rest_the_menu_draws_nothing() {
        let g = Gfx::new(1.0);
        let ready = std::rc::Rc::new(std::cell::Cell::new(true));
        let mut ui = Ui::with_pages(probes(&ready, false), true, true, 0.0);
        let t = 10_000.0;
        ui.update(t);
        ui.ensure_layer(&g, 0, t);
        ui.dirty = false;
        ui.update(t + 16.0);
        // (`dirty` is not asserted: other tests' threads may wake the app-wide services flag meanwhile)
        assert!(!ui.animating(t + 16.0), "no frames at rest");
        assert_eq!(ui.wake_at(t + 16.0), None, "nothing to wake for");
    }

    /// Order 047: a toast at rest needs no frames - only its fade-out, at 1.8 s, wakes the menu (it rebuilds there).
    #[test]
    fn a_toast_at_rest_sleeps_until_its_fade() {
        let g = Gfx::new(1.0);
        let ready = std::rc::Rc::new(std::cell::Cell::new(true));
        let mut ui = Ui::with_pages(probes(&ready, false), true, true, 0.0);
        let t = 10_000.0;
        ui.update(t);
        ui.toast = Some(("Saved".into(), t));
        ui.ensure_layer(&g, 0, t);
        // its fade-in (transitions, reduced motion: 10 ms) runs out
        ui.ensure_layer(&g, 0, t + 50.0);
        ui.dirty = false;
        ui.update(t + 60.0);
        assert!(!ui.animating(t + 60.0), "no frames while it just shows");
        let end = t + crate::ui::pieces::toast::SHOW_MS;
        assert!(ui.wake_at(t + 60.0).is_some_and(|w| (w - end).abs() < 1.0), "wakes when it starts to fade: {:?}", ui.wake_at(t + 60.0));
        ui.wake(end + 1.0);
        assert!(ui.dirty, "the wake-up builds it again (its fade-out starts)");
    }

    /// Order 047: a page polled with `wake_at` that has nothing new asks for no frame when it is woken.
    #[test]
    fn a_woken_page_with_nothing_new_draws_nothing() {
        struct Poll(std::rc::Rc<std::cell::Cell<u32>>);
        impl Page for Poll {
            fn id(&self) -> &'static str {
                "poll"
            }
            fn name(&self) -> &'static str {
                "poll"
            }
            fn icon(&self) -> &'static str {
                ""
            }
            fn tick(&mut self, _now: f64) -> bool {
                self.0.set(self.0.get() + 1);
                false
            }
            fn wake_at(&self, now: f64) -> Option<f64> {
                Some(now + 16.0)
            }
            fn build(&mut self, _cx: &mut Cx) -> Vec<El> {
                vec![El::block().h(20.0)]
            }
        }
        let g = Gfx::new(1.0);
        let n = std::rc::Rc::new(std::cell::Cell::new(0));
        let mut ui = Ui::with_pages(vec![Box::new(Poll(n.clone()))], true, true, 0.0);
        let t = 10_000.0;
        ui.update(t);
        ui.ensure_layer(&g, 0, t);
        ui.dirty = false;
        let before = n.get();
        let w = ui.wake_at(t).expect("it polls");
        ui.wake(w);
        assert!(n.get() > before, "the wake-up ran its tick");
        assert!(!ui.animating(w), "and drew nothing");
        assert!(ui.wake_at(w).is_some_and(|x| x > w), "the next look is later");
    }

    /// Order 047: the menu makes only the page it shows first (it used to open Audio on every open and close it again).
    #[test]
    fn the_menu_opens_only_its_first_tab() {
        struct Count(&'static str, std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>);
        impl Page for Count {
            fn id(&self) -> &'static str {
                self.0
            }
            fn name(&self) -> &'static str {
                self.0
            }
            fn icon(&self) -> &'static str {
                ""
            }
            fn open(&mut self, _env: &Env, _now: f64) {
                self.1.borrow_mut().push(self.0);
            }
            fn build(&mut self, _cx: &mut Cx) -> Vec<El> {
                vec![]
            }
        }
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let pages: Vec<Box<dyn Page>> = vec![Box::new(Count("aud", log.clone())), Box::new(Count("pad", log.clone()))];
        let mut ui = Ui::with_pages_on(pages, 1, true, true, 0.0);
        ui.start_on("pad", None, 0.0);
        assert_eq!(*log.borrow(), vec!["pad"], "only the shown tab was opened");
        assert_eq!(ui.tab_id(), "pad");
    }

    /// Feedback F3: re-opened during the close motion it stays on its tab.
    #[test]
    fn a_reopen_stays_on_the_tab() {
        let ready = std::rc::Rc::new(std::cell::Cell::new(true));
        let mut ui = Ui::with_pages(probes(&ready, false), true, true, 0.0);
        ui.show_tab(1, 100.0);
        ui.close(1_000.0);
        ui.open(1_050.0);
        assert_eq!((ui.tab, ui.tab_id()), (1, "two"));
    }

    #[test]
    fn a_menu_opened_for_a_tab_starts_on_it() {
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut ui = Ui::with_pages(tabs(&log), true, true, 0.0);
        ui.start_on("three", Some("listen"), 0.0);
        assert_eq!(ui.tab, 2);
        assert!(ui.switch.is_none(), "opened on it, no switch animation");
        ui.go_to("two", Some("mute"), 10.0);
        assert_eq!(ui.tab, 1);
        assert!(ui.switch.is_some());
        assert_eq!(*log.borrow(), ["three jump listen", "two jump mute"]);
        // the request itself: taken once
        crate::services::show_menu("vtt", Some("listen"));
        assert_eq!(crate::services::take_show_menu(), Some(("vtt".to_string(), Some("listen".to_string()))));
        assert_eq!(crate::services::take_show_menu(), None);
    }

    /// Order 043 item 5: Get on Notifications for OBS - its new tab joins the top row past the right edge (place 17 of
    /// 19); the row scrolls (its own smooth scroll) until the bell is fully in view. The shown tab stays.
    #[test]
    fn a_tab_got_scrolls_into_the_rows_view() {
        crate::services::init(windows::Win32::Foundation::HWND::default(), true);
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        // stand-ins with the real tab ids (none of them opens anything), the row at its start
        let ids: Vec<&'static str> = pages::all().iter().map(|p| p.id()).filter(|id| *id != crate::obs::PAGE).collect();
        let pages: Vec<Box<dyn Page>> = ids.iter().map(|&id| Box::new(Tab { id, value: String::new(), log: log.clone() }) as Box<dyn Page>).collect();
        let mut ui = Ui::with_pages(pages, false, true, 0.0);
        let t = 10_000.0;
        ui.update(t);
        assert_eq!(ui.ds, 0.0);
        crate::addons::set_for("obs", true, true);
        for k in 1..60 {
            ui.update(t + 16.0 * k as f64);
        }
        let i = ui.pages.iter().position(|p| p.id() == crate::obs::PAGE).expect("the bell joined the row");
        let x = dock_x(i) - ui.ds;
        assert!(x >= 0.0 && x + DT_W <= DS_W, "the bell at {x} of the row's {DS_W} px (scroll {})", ui.ds);
        assert_eq!(ui.tab_id(), "aud", "the shown tab stays");
        crate::addons::set_for("obs", false, true);
        crate::services::shutdown();
    }
}
