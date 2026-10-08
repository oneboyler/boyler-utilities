//! The mic icon on your screen (menu-v22 `#mici`): a small glass pill "Live" / "Muted" (or a round icon, or a dot), shown
//! by the Mute settings' "Icon on screen": When it changes (1.5 s after each change), Always, or Off. While Mute settings
//! is open (Mic mute on) it shows so its look can be seen; "Move icon" shows it with a dashed frame and lets you drag it on
//! the screen itself (the drawing's snapping: the edges at 24 px, the middle, 12 px snap). Every appearance pops in with a
//! spring; it fades out gently.
//!
//! Real copies: one click-through, topmost, layered tool window per monitor "Show on" picks (Main, one other, or all;
//! no taskbar button, never takes focus), made only while the icon shows (or fades out), painted by the app's own painter into the window with `UpdateLayeredWindow`; a thread
//! timer repaints it only while it animates and hides it when a "When it changes" flash ends. TEST copies never make the
//! window or a timer: the icon is proven off-screen (`render`, the tests). The same window plumbing as the Timers page's
//! on-screen pills (pages/timers/overlay.rs).

use std::cell::{Cell, RefCell};

use skia_safe as sk;
use taffy::style::JustifyContent;

use crate::anim::Bezier;
use crate::gfx::{sh, CssColor, Font, Gfx, Rgba};
use crate::icons::Icons;
use crate::ui::el::{El, RADIUS_PILL};
use crate::ui::lay::Laid;
use crate::ui::WHITE;

/// The drawing's EDGE / SNAP.
pub const EDGE: f32 = 24.0;
pub const SNAP: f32 = 12.0;
/// `KS={s:.82,m:1,l:1.22}`
pub const KS: [f32; 3] = [0.82, 1.0, 1.22];

/// Where the icon sits: anchored to its nearest edge (`h` 'L' / 'R' / 'C' + the distance `dx`; `v` 'T' / 'B' + `dy`),
/// measured from the monitor's work area (the drawing measures from its window minus a 48 px taskbar).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub h: char,
    pub dx: f32,
    pub v: char,
    pub dy: f32,
}

/// The drawing's six quick spots (`SPOTS`): 24 px from the edges, the middle centred.
pub fn quick(i: usize) -> Spot {
    let (h, v) = [('L', 'T'), ('C', 'T'), ('R', 'T'), ('L', 'B'), ('C', 'B'), ('R', 'B')][i.min(5)];
    Spot { h, dx: if h == 'C' { 0.0 } else { EDGE }, v, dy: EDGE }
}

/// Which quick spot a spot is (the corner picker lights it), if any.
pub fn quick_of(s: Spot) -> Option<usize> {
    (0..6).find(|&i| quick(i) == s)
}

// ---------------------------------------------------------------- "Show on" (which monitors)

/// The saved "Show on" value of "All monitors" (the others are the list's own index: 0 = Main, 1 = Monitor 2 ...).
pub const MON_ALL: usize = 99;

/// One monitor: its work area in px (x, y, w, h), its scale (effective DPI / 96), and whether it is Windows' main one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mon {
    pub work: (i32, i32, i32, i32),
    pub scale: f32,
    pub primary: bool,
}

/// The monitors in the "Show on" list order: Main first, then the others left to right (top to bottom when level).
pub fn order_monitors(mut v: Vec<Mon>) -> Vec<Mon> {
    v.sort_by_key(|m| (!m.primary, m.work.0, m.work.1));
    v
}

/// The "Show on" list for `n` monitors: "Main", "Monitor 2" ..., then "All monitors" when there are several.
pub fn monitor_labels(n: usize) -> Vec<String> {
    let mut v = vec!["Main".to_string()];
    for i in 2..=n.max(1) {
        v.push(format!("Monitor {i}"));
    }
    if n > 1 {
        v.push("All monitors".into());
    }
    v
}

/// A value saved before Order 040 (the list row; "All monitors" was the row after the last monitor = the monitor count),
/// read once at app start: it means All when the count is still the same, else the monitor it named.
pub fn from_old_row(row: usize, n: usize) -> usize {
    if n > 1 && row == n {
        MON_ALL
    } else {
        row
    }
}

/// The list row a saved value shows as (a monitor that is gone falls back to Main).
pub fn list_index(mon: usize, n: usize) -> usize {
    if n > 1 && mon == MON_ALL {
        n
    } else if mon < n {
        mon
    } else {
        0
    }
}

/// The value saved for a picked list row.
pub fn stored(row: usize, n: usize) -> usize {
    if n > 1 && row == n {
        MON_ALL
    } else {
        row
    }
}

/// Which monitors (indexes into [`order_monitors`]' order) get the icon.
pub fn targets(mon: usize, n: usize) -> Vec<usize> {
    if n == 0 {
        return Vec::new();
    }
    let i = list_index(mon, n);
    if n > 1 && i == n {
        (0..n).collect()
    } else {
        vec![i]
    }
}

/// The icon's top-left in px on one monitor for a spot (`size` in DIPs; the spot is kept in DIPs of that monitor).
pub fn place_on(m: &Mon, spot: Spot, size: (f32, f32)) -> (i32, i32) {
    let s = m.scale;
    let (wx, wy, ww, wh) = m.work;
    let (x, y) = place(spot, (wx as f32 / s, wy as f32 / s, ww as f32 / s, wh as f32 / s), size);
    ((x * s).round() as i32, (y * s).round() as i32)
}

thread_local! {
    /// the saved "Show on" value the windows were made for
    static MON: Cell<usize> = const { Cell::new(0) };
    /// the monitors changed (or "Show on" did): the windows are made again on the next frame
    static REBUILD: Cell<bool> = const { Cell::new(false) };
    /// Windows said the monitors may have changed: the next frame compares them with the windows (made again only if
    /// different - a window moved onto a monitor with another scale also gets WM_DPICHANGED)
    static CHECK: Cell<bool> = const { Cell::new(false) };
}

/// "Show on" from the settings (before each `update`): a different choice moves the icon to the new monitor(s).
pub fn set_monitor(mon: usize) {
    if MON.with(|m| m.replace(mon)) != mon {
        REBUILD.with(|r| r.set(true));
    }
}

/// How many monitors Windows has now.
pub fn monitor_count() -> usize {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CMONITORS};
        unsafe { GetSystemMetrics(SM_CMONITORS) }.max(1) as usize
    }
    #[cfg(not(windows))]
    1
}

/// How the icon looks now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub muted: bool,
    /// 0 pill with text, 1 icon only, 2 dot
    pub style: usize,
    /// 0 S, 1 M, 2 L
    pub size: usize,
    /// 0.3 .. 1
    pub op: f32,
    pub moving: bool,
}

/// The icon (`.mv` > `.in`): `.in{display:flex;align-items:center;justify-content:center;gap:calc(7px*k);height:calc(34px*k);
///   padding:0 calc(14px*k) 0 calc(11px*k);border-radius:calc(17px*k);overflow:hidden;color:#fff;font-size:calc(12.5px*k);
///   font-weight:600;letter-spacing:-.005em;opacity:var(--op);background:rgba(22,24,30,.62);backdrop-filter:blur(20px)
///   saturate(1.5);box-shadow:inset 0 0 0 1px rgba(255,255,255,.13),0 6px 18px rgba(0,0,0,.26)}`
/// `svg{width:calc(18px*k);stroke:#b4f3c6;stroke-width:1.7}` `.m svg{stroke:#ff5a52}` `.sl{opacity:0}` `.m .sl{opacity:1}`
/// `.ico .in{width:calc(34px*k);padding:0}` `.dot .in{width:calc(14px*k);height:calc(14px*k);border-radius:50%;
///   background:#30d158;box-shadow:inset 0 0 0 1px rgba(255,255,255,.3),0 0 0 calc(3px*k) rgba(22,24,30,.5),0 2px 8px rgba(0,0,0,.3)}`
/// `.dot.m .in{background:#ff453a}`. `m` = 0..1 (live -> muted, the .2 s colour transition).
pub fn icon(l: &Look, m: f32) -> El {
    let k = KS[l.size.min(2)];
    let col = crate::ui::cmix(Rgba::hex(0xb4f3c6), Rgba::hex(0xff5a52), m);
    if l.style == 2 {
        return El::block()
            .size(14.0 * k, 14.0 * k)
            .none()
            .radius(RADIUS_PILL)
            .bg(crate::ui::cmix(Rgba::hex(0x30d158), Rgba::hex(0xff453a), m))
            .shadow(&[sh(0.0, 0.0, 0.0, 3.0 * k, Rgba::rgba(22, 24, 30, 0.5)), sh(0.0, 2.0, 8.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.3))])
            .inset(&[sh(0.0, 0.0, 0.0, 1.0, Rgba(1.0, 1.0, 1.0, 0.3))])
            .opacity(l.op);
    }
    let mut e = El::row()
        .center()
        .justify(JustifyContent::CENTER)
        .gap(7.0 * k)
        .h(34.0 * k)
        .none()
        .radius(17.0 * k)
        .clip()
        .bg(Rgba::rgba(22, 24, 30, 0.62))
        .shadow(&[sh(0.0, 6.0, 18.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.26))])
        .inset(&[sh(0.0, 0.0, 0.0, 1.0, Rgba(1.0, 1.0, 1.0, 0.13))])
        .opacity(l.op)
        .child(El::icon("micS", 18.0 * k, 1.7, col).class_op("sl", m));
    if l.style == 1 {
        e = e.w(34.0 * k);
    } else {
        // #mici sits outside the menu's #sw: the label's own letter-spacing (-.005em)
        let f = Font::new(12.5 * k, 600).ls((-0.005 * 12.5 * k * 1000.0) as i32);
        e = e.pad(0.0, 14.0 * k, 0.0, 11.0 * k).child(El::text(if l.muted { "Muted" } else { "Live" }, f, WHITE, 12.5 * k * 1.35).none());
    }
    e
}

/// The icon with, while moving, its dashed frame (`.mv::after{inset:-6px;border-radius:calc(17px*k + 6px);border:1px dashed
/// rgba(10,132,255,.95)}`, round for the dot) and the "Drag to place" tag 14 px under it (over it when it sits low).
pub fn mv(l: &Look, m: f32, low: bool) -> El {
    let k = KS[l.size.min(2)];
    let inner = icon(l, m);
    // `#mici{display:flex}` > `.mv{position:relative;flex:none}`: as wide as the icon
    let mut mv = El::row().none().child(inner);
    if l.moving {
        let r = if l.style == 2 { 1e6 } else { 17.0 * k + 6.0 - 0.5 };
        mv = mv.child(
            El::paint(move |g: &Gfx, (x, y, w, h)| {
                let mut pt = sk::Paint::new(Rgba::rgba(10, 132, 255, 0.95).c4(), None);
                pt.set_anti_alias(true);
                pt.set_style(sk::PaintStyle::Stroke);
                pt.set_stroke_width(1.0);
                pt.set_path_effect(sk::PathEffect::dash(&[3.0, 3.0], 0.0));
                let rr = r.min(w / 2.0).min(h / 2.0);
                g.cv().draw_rrect(sk::RRect::new_rect_xy(sk::Rect::from_xywh(x + 0.5, y + 0.5, w - 1.0, h - 1.0), rr, rr), &pt);
            })
            .abs(-6.0, -6.0, -6.0, -6.0)
            .no_hit(),
        );
        let tag = El::row()
            .pad(5.0, 9.0, 5.0, 9.0)
            .radius(9.0)
            .bg(Rgba::rgba(10, 132, 255, 0.92))
            .shadow(&[sh(0.0, 4.0, 12.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.25))])
            .child(El::text("Drag to place", Font::new(11.0, 600).ls(0), WHITE, 11.0));
        let h = if l.style == 2 { 14.0 * k } else { 34.0 * k };
        let wrap = El::row().justify(JustifyContent::CENTER).no_hit().child(tag);
        mv = mv.child(if low { wrap.abs(-60.0, f32::NAN, -60.0, h + 14.0) } else { wrap.abs(-60.0, h + 14.0, -60.0, f32::NAN) });
    }
    mv
}

/// The icon's size (DIPs), laid out once.
pub fn size_of(g: &Gfx, l: &Look) -> (f32, f32) {
    let laid = Laid::new(g, El::row().w(400.0).child(icon(l, if l.muted { 1.0 } else { 0.0 })), 400.0, None);
    laid.nodes.get(1).map(|n| (n.rect.2, n.rect.3)).unwrap_or((96.0, 34.0))
}

/// Where the icon goes on a work area (x, y, w, h) for a spot (`placeIcon`).
pub fn place(spot: Spot, work: (f32, f32, f32, f32), size: (f32, f32)) -> (f32, f32) {
    let (wx, wy, ww, wh) = work;
    let (w, h) = size;
    let dx = spot.dx.clamp(0.0, (ww - w).max(0.0));
    let dy = spot.dy.clamp(0.0, (wh - h).max(0.0));
    let x = match spot.h {
        'L' => wx + dx,
        'R' => wx + ww - dx - w,
        _ => wx + (ww - w) / 2.0,
    };
    let y = if spot.v == 'B' { wy + wh - dy - h } else { wy + dy };
    (x.round(), y.round())
}

/// A drag (`pointermove` while moving): the new spot, snapped to the edges (EDGE), the middle, within SNAP. `l`, `t` = the
/// icon's wanted top-left inside the work area (w x h).
pub fn snap_spot(l: f32, t: f32, size: (f32, f32), area: (f32, f32)) -> Spot {
    let (w, hh) = size;
    let (aw, ah) = area;
    let (mut l, mut t) = (l, t);
    let xs = [(EDGE, 'L'), (aw - EDGE - w, 'R'), (aw / 2.0 - w / 2.0, 'C')];
    let ys = [(EDGE, 'T'), (ah - EDGE - hh, 'B'), (ah / 2.0 - hh / 2.0, 'C')];
    let near = |v: f32, list: &[(f32, char)]| list.iter().filter(|c| (v - c.0).abs() <= SNAP).min_by(|a, b| (v - a.0).abs().total_cmp(&(v - b.0).abs())).copied();
    let sx = near(l, &xs);
    let sy = near(t, &ys);
    if let Some(s) = sx {
        l = s.0;
    }
    if let Some(s) = sy {
        t = s.0;
    }
    l = l.clamp(0.0, (aw - w).max(0.0));
    t = t.clamp(0.0, (ah - hh).max(0.0));
    let hs = if l + w / 2.0 > aw / 2.0 { 'R' } else { 'L' };
    let vs = if t + hh / 2.0 > ah / 2.0 { 'B' } else { 'T' };
    let mid = sx.map(|s| s.1 == 'C').unwrap_or(false);
    Spot { h: if mid { 'C' } else { hs }, dx: if mid { 0.0 } else if hs == 'R' { aw - (l + w) } else { l }, v: vs, dy: if vs == 'B' { ah - (t + hh) } else { t } }
}

/// The pop-in spring: scale .85 -> 1.04 (58 %) -> 1 over 380 ms; a pop while shown: 1 -> 1.07 (40 %) -> 1 over 300 ms.
pub fn pop_scale(age: f64, was_shown: bool) -> f32 {
    let (dur, mid_at, a, mid) = if was_shown { (300.0, 0.4, 1.0, 1.07) } else { (380.0, 0.58, 0.85, 1.04) };
    if age >= dur {
        return 1.0;
    }
    let p = (age / dur).clamp(0.0, 1.0);
    if p < mid_at {
        let e = Bezier::new(0.2, 0.8, 0.3, 1.0).ease(p / mid_at) as f32;
        a + (mid - a) * e
    } else {
        let e = Bezier::new(0.45, 0.0, 0.4, 1.0).ease((p - mid_at) / (1.0 - mid_at)) as f32;
        mid + (1.0 - mid) * e
    }
}

/// The off-screen picture of the icon over a desktop picture (`desk`, 1 px = 1 DIP x scale), placed by `spot` on a work
/// area = the desk minus a 48 px taskbar (the drawing's TB). Tests / the proof only.
pub fn render(look: &Look, spot: Spot, desk: &crate::png::Pixels, scale: f32) -> Option<crate::png::Pixels> {
    let mut s = crate::gfx::new_surface(desk.w as i32, desk.h as i32)?;
    let g = Gfx::new(scale);
    let icons = Icons::new();
    let img = crate::png::to_image(desk)?;
    g.begin(s.canvas());
    g.cv().save();
    g.cv().reset_matrix();
    g.cv().draw_image(&img, (0.0, 0.0), None);
    g.cv().restore();
    let (dw, dh) = (desk.w as f32 / scale, desk.h as f32 / scale);
    let size = size_of(&g, look);
    let (x, y) = place(spot, (0.0, 0.0, dw, dh - 48.0), size);
    paint(&g, &icons, look, if look.muted { 1.0 } else { 0.0 }, spot.v == 'B', x, y, 1.0, 1.0, Some(&img));
    g.end();
    Some(crate::png::from_surface(&mut s))
}

/// Paint the icon at (x, y) DIPs: `fade` = the wrapper's opacity (`#mici` .18 s in / .45 s out), `pop` = its spring scale.
#[allow(clippy::too_many_arguments)]
fn paint(g: &Gfx, icons: &Icons, look: &Look, m: f32, low: bool, x: f32, y: f32, fade: f32, pop: f32, base: Option<&sk::Image>) {
    let root = mv(look, m, low);
    let laid = Laid::new(g, El::row().w(400.0).items(taffy::style::AlignItems::FLEX_START).child(root), 400.0, None);
    let (w, h) = laid.nodes.get(2).map(|n| (n.rect.2, n.rect.3)).unwrap_or((96.0, 34.0));
    let t0 = g.transform();
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    g.set_transform(&(windows_numerics::Matrix3x2 { M11: pop, M12: 0.0, M21: 0.0, M22: pop, M31: cx * (1.0 - pop), M32: cy * (1.0 - pop) } * t0));
    if fade < 0.999 {
        g.push_layer(fade, None);
    }
    if let (Some(b), true) = (base, look.style != 2) {
        // backdrop-filter: blur(20px) saturate(1.5) under the glass
        let k = KS[look.size.min(2)];
        g.backdrop(b, x, y, w, h, 17.0 * k, 20.0, &[CssColor::Saturate(1.5)]);
    }
    laid.paint(g, icons, x, y, None);
    if fade < 0.999 {
        g.pop_layer();
    }
    g.set_transform(&t0);
}

// ====================================================================== what shows, and the real window

/// What decides whether the icon shows (`iconRefresh`).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Want {
    /// Mic mute on and a key set (`ready()`), and "Icon on screen" not Off
    pub active: bool,
    /// "Always"
    pub always: bool,
    /// the Mute settings popup is open with Mic mute on (preview)
    pub preview: bool,
    pub moving: bool,
}

struct State {
    want: Want,
    look: Look,
    spot: Spot,
    /// a "When it changes" flash shows until (ms, the app clock)
    flash_until: f64,
    shown: bool,
    shown_at: f64,
    hid_at: f64,
    pop_at: f64,
    pop_shown: bool,
    m: f32,
}

thread_local! {
    /// the page runs on a fake (a test copy, a unit test): never a window, never a timer
    static FAKE: Cell<bool> = const { Cell::new(false) };
    static ST: RefCell<Option<State>> = const { RefCell::new(None) };
    /// one window per monitor the icon shows on ("Show on")
    #[cfg(windows)]
    static WIN: RefCell<Vec<real::Win>> = const { RefCell::new(Vec::new()) };
    static TIMER: Cell<usize> = const { Cell::new(0) };
}

fn now() -> f64 {
    crate::timing::now()
}

/// Is the on-screen window there (tests: never in a test copy).
pub fn window_exists() -> bool {
    #[cfg(windows)]
    return WIN.with(|w| !w.borrow().is_empty());
    #[cfg(not(windows))]
    false
}

/// Is a Windows timer armed for it (tests: never in a test copy).
pub fn timer_armed() -> bool {
    TIMER.with(|t| t.get() != 0)
}

/// The page's services are fakes (set at every page open): the icon is then only a model, never on the screen.
pub fn set_fake(fake: bool) {
    FAKE.with(|f| f.set(fake));
}

/// Is the icon showing now (the model; test copies too).
pub fn showing() -> bool {
    ST.with(|s| s.borrow().as_ref().map(|s| s.shown).unwrap_or(false))
}

/// New settings / mic state from the page. `changed` = the mic was just muted / unmuted (a "When it changes" flash).
pub fn update(want: Want, look: Look, spot: Spot, changed: bool) {
    let t = now();
    ST.with(|s| {
        let mut s = s.borrow_mut();
        let st = s.get_or_insert(State {
            want,
            look,
            spot,
            flash_until: 0.0,
            shown: false,
            shown_at: -1e9,
            hid_at: -1e9,
            pop_at: -1e9,
            pop_shown: false,
            m: if look.muted { 1.0 } else { 0.0 },
        });
        let look_changed = st.look.style != look.style || st.look.size != look.size;
        st.want = want;
        st.look = look;
        st.spot = spot;
        if changed && want.active && !want.always {
            st.flash_until = t + 1500.0;
        }
        let show = want.moving || want.preview || (want.active && (want.always || t < st.flash_until));
        if show && !st.shown {
            st.shown_at = t;
            st.pop_at = t;
            st.pop_shown = false;
        } else if show && (changed || look_changed) {
            st.pop_at = t;
            st.pop_shown = true;
        } else if !show && st.shown {
            st.hid_at = t;
        }
        st.shown = show;
    });
    frame();
}

/// One frame of the real window (made / repainted / dropped) and the timer that keeps it going while anything moves.
fn frame() {
    // never on the screen from a test: a test copy, a unit test (cargo test), or a page on fakes
    if cfg!(test) || crate::testmode::on() || FAKE.with(|f| f.get()) {
        return;
    }
    let t = now();
    let Some((look, spot, fade, pop, m, busy, gone)) = ST.with(|s| {
        let mut s = s.borrow_mut();
        let st = s.as_mut()?;
        // the colour follows the mic in .2 s
        let target = if st.look.muted { 1.0 } else { 0.0 };
        st.m += (target - st.m) * 0.35;
        if (st.m - target).abs() < 0.01 {
            st.m = target;
        }
        if st.shown && st.want.active && !st.want.always && !st.want.preview && !st.want.moving && t >= st.flash_until {
            st.shown = false;
            st.hid_at = t;
        }
        let fade = if st.shown { ((t - st.shown_at) / 180.0).clamp(0.0, 1.0) as f32 } else { 1.0 - ((t - st.hid_at) / 450.0).clamp(0.0, 1.0) as f32 };
        let pop = if st.shown { pop_scale(t - st.pop_at, st.pop_shown) } else { 0.96 + 0.04 * fade };
        let busy = st.m != target || (st.shown && (t - st.shown_at < 200.0 || t - st.pop_at < 400.0)) || (!st.shown && fade > 0.0) || (st.shown && st.want.active && !st.want.always && !st.want.preview && !st.want.moving);
        Some((st.look, st.spot, fade, pop, st.m, busy, !st.shown && fade <= 0.0))
    }) else {
        return;
    };
    #[cfg(windows)]
    real::show(&look, spot, fade, pop, m, gone);
    #[cfg(not(windows))]
    let _ = (look, spot, fade, pop, m, gone);
    aim((busy && !gone) || CHECK.with(|c| c.get()));
}

fn aim(on: bool) {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};
        unsafe extern "system" fn tick(_h: windows::Win32::Foundation::HWND, _m: u32, _id: usize, _t: u32) {
            frame();
        }
        let old = TIMER.with(|t| t.get());
        if on == (old != 0) {
            return;
        }
        let id = if on {
            unsafe { SetTimer(None, 0, 16, Some(tick)) }
        } else {
            unsafe {
                let _ = KillTimer(None, old);
            }
            0
        };
        TIMER.with(|t| t.set(id));
    }
    #[cfg(not(windows))]
    let _ = on;
}

/// The page asks for the spot the user dragged the icon to (moving).
pub fn dragged_spot() -> Option<Spot> {
    ST.with(|s| s.borrow().as_ref().map(|s| s.spot))
}

#[cfg(windows)]
mod real {
    use super::*;
    use windows::core::{w, BOOL};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
    use windows::Win32::UI::WindowsAndMessaging::*;

    const CLASS: windows::core::PCWSTR = w!("BoylerUtilities.MicIcon");
    /// room around the icon for its shadow (0 6px 18px), the pop (1.07) and the "Drag to place" tag
    const PAD: f32 = 56.0;

    pub struct Win {
        hwnd: HWND,
        g: Gfx,
        icons: Icons,
        at: (i32, i32),
        size: (f32, f32),
        mon: Mon,
        drag: Option<(i32, i32, i32, i32)>,
    }

    /// Every monitor Windows has now, in the "Show on" order.
    fn monitors_now() -> Vec<Mon> {
        unsafe extern "system" fn each(h: HMONITOR, _dc: HDC, _r: *mut RECT, lp: LPARAM) -> BOOL {
            let v = unsafe { &mut *(lp.0 as *mut Vec<Mon>) };
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if unsafe { GetMonitorInfoW(h, &mut mi) }.as_bool() {
                let (mut dx, mut dy) = (96u32, 96u32);
                let _ = unsafe { GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
                let r = mi.rcWork;
                v.push(Mon { work: (r.left, r.top, r.right - r.left, r.bottom - r.top), scale: dx as f32 / 96.0, primary: mi.dwFlags & MONITORINFOF_PRIMARY != 0 });
            }
            true.into()
        }
        let mut v: Vec<Mon> = Vec::new();
        unsafe {
            let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut v as *mut Vec<Mon> as isize));
        }
        order_monitors(v)
    }

    fn ensure_class() {
        thread_local!(static DONE: Cell<bool> = const { Cell::new(false) });
        if DONE.with(|d| d.get()) {
            return;
        }
        unsafe {
            let inst = GetModuleHandleW(None).unwrap_or_default();
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: CLASS, hCursor: LoadCursorW(None, IDC_SIZEALL).unwrap_or_default(), ..Default::default() };
            RegisterClassW(&wc);
        }
        DONE.with(|d| d.set(true));
    }

    fn drop_all() {
        let wins: Vec<Win> = WIN.with(|w| w.borrow_mut().drain(..).collect());
        for win in wins {
            unsafe {
                let _ = DestroyWindow(win.hwnd);
            }
        }
    }

    pub(super) fn show(look: &Look, spot: Spot, fade: f32, pop: f32, m: f32, gone: bool) {
        if gone {
            drop_all();
            CHECK.with(|c| c.set(false));
            return;
        }
        if REBUILD.with(|r| r.replace(false)) {
            drop_all();
        }
        if CHECK.with(|c| c.replace(false)) && !WIN.with(|w| w.borrow().is_empty()) {
            let mons = monitors_now();
            let want: Vec<Mon> = targets(MON.with(|m| m.get()), mons.len()).into_iter().map(|i| mons[i]).collect();
            let have: Vec<Mon> = WIN.with(|w| w.borrow().iter().map(|x| x.mon).collect());
            if want != have {
                drop_all();
            }
        }
        if WIN.with(|w| w.borrow().is_empty()) {
            ensure_class();
            let mons = monitors_now();
            for i in targets(MON.with(|m| m.get()), mons.len()) {
                let ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT;
                let hwnd = unsafe { CreateWindowExW(ex, CLASS, w!("Mic"), WS_POPUP, mons[i].work.0, mons[i].work.1, 1, 1, None, None, GetModuleHandleW(None).ok().map(|h| h.into()), None) };
                let Ok(hwnd) = hwnd else { continue };
                let mon = mons[i];
                WIN.with(|w| w.borrow_mut().push(Win { hwnd, g: Gfx::new(mon.scale), icons: Icons::new(), at: (0, 0), size: (0.0, 0.0), mon, drag: None }));
                unsafe {
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                }
            }
        }
        WIN.with(|w| {
            for win in w.borrow_mut().iter_mut() {
                paint_win(win, look, spot, fade, pop, m);
                // while moving the icon takes the mouse; otherwise clicks go through it
                unsafe {
                    let ex = GetWindowLongW(win.hwnd, GWL_EXSTYLE);
                    let t = WS_EX_TRANSPARENT.0 as i32;
                    let want = if look.moving { ex & !t } else { ex | t };
                    if want != ex {
                        SetWindowLongW(win.hwnd, GWL_EXSTYLE, want);
                    }
                }
            }
        });
    }

    fn paint_win(win: &mut Win, look: &Look, spot: Spot, fade: f32, pop: f32, m: f32) {
        let size = size_of(&win.g, look);
        let s = win.mon.scale;
        win.at = if let Some((_, _, l, t)) = win.drag { (l, t) } else { place_on(&win.mon, spot, size) };
        win.size = size;
        let (bw, bh) = (((size.0 + 2.0 * PAD) * s).ceil() as i32, ((size.1 + 2.0 * PAD) * s).ceil() as i32);
        let Some(mut surf) = crate::gfx::new_surface(bw, bh) else { return };
        win.g.begin(surf.canvas());
        paint(&win.g, &win.icons, look, m, spot.v == 'B', PAD, PAD, fade, pop, None);
        win.g.end();
        let px = crate::png::from_surface(&mut surf);
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: bw, biHeight: -bh, biPlanes: 1, biBitCount: 32, ..Default::default() },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            if let Ok(bmp) = CreateDIBSection(Some(mem), &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
                std::ptr::copy_nonoverlapping(px.data.as_ptr(), bits as *mut u8, px.data.len());
                let old = SelectObject(mem, bmp.into());
                let pos = POINT { x: win.at.0 - (PAD * s).round() as i32, y: win.at.1 - (PAD * s).round() as i32 };
                let sz = SIZE { cx: bw, cy: bh };
                let src = POINT::default();
                let blend = BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: 1 };
                let _ = UpdateLayeredWindow(win.hwnd, Some(screen), Some(&pos), Some(&sz), Some(mem), Some(&src), Default::default(), Some(&blend), ULW_ALPHA);
                SelectObject(mem, old);
                let _ = DeleteObject(bmp.into());
            }
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
        }
    }

    unsafe extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        let pt = || {
            let mut p = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut p);
            }
            p
        };
        match msg {
            WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
            WM_LBUTTONDOWN => {
                let p = pt();
                WIN.with(|w| {
                    if let Some(win) = w.borrow_mut().iter_mut().find(|x| x.hwnd == h) {
                        win.drag = Some((p.x, p.y, win.at.0, win.at.1));
                        unsafe {
                            SetCapture(win.hwnd);
                        }
                    }
                });
                return LRESULT(0);
            }
            WM_MOUSEMOVE => {
                let p = pt();
                // the spot is measured on the monitor the icon is dragged on; every other copy follows it
                let spot = WIN.with(|w| {
                    let w = w.borrow();
                    let win = w.iter().find(|x| x.hwnd == h)?;
                    let (x0, y0, l0, t0) = win.drag?;
                    let s = win.mon.scale;
                    let (wx, wy, ww, wh) = win.mon.work;
                    let l = (l0 + p.x - x0 - wx) as f32 / s;
                    let t = (t0 + p.y - y0 - wy) as f32 / s;
                    // Order 045: the blue snap guides + the corner tag (`guide(gV..)`, `gTag`)
                    crate::guides::show_for(win.mon.work, s, l, t, win.size, EDGE, SNAP, ((p.x - wx) as f32 / s, (p.y - wy) as f32 / s));
                    Some(snap_spot(l, t, win.size, (ww as f32 / s, wh as f32 / s)))
                });
                if let Some(sp) = spot {
                    ST.with(|s| {
                        if let Some(st) = s.borrow_mut().as_mut() {
                            st.spot = sp;
                        }
                    });
                    let (look, m) = ST.with(|s| s.borrow().as_ref().map(|s| (s.look, s.m)))
                        .unwrap_or((Look { muted: false, style: 0, size: 1, op: 1.0, moving: true }, 0.0));
                    WIN.with(|w| {
                        for win in w.borrow_mut().iter_mut() {
                            let d = win.drag.take();
                            paint_win(win, &look, sp, 1.0, 1.0, m);
                            win.drag = d;
                        }
                    });
                    super::super::mute::set_spot(sp);
                }
                return LRESULT(0);
            }
            WM_LBUTTONUP | WM_CAPTURECHANGED => {
                WIN.with(|w| {
                    if let Some(win) = w.borrow_mut().iter_mut().find(|x| x.hwnd == h) {
                        win.drag = None;
                    }
                });
                // Order 045: `dragEnd`: the guides and the tag go
                crate::guides::hide();
                if msg == WM_LBUTTONUP {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                }
                return LRESULT(0);
            }
            // a monitor came or went, a resolution / scale changed, the taskbar moved: the next tick compares the
            // monitors with the windows and makes them again only when they differ (never inside this window's message)
            WM_DISPLAYCHANGE | WM_DPICHANGED => {
                CHECK.with(|c| c.set(true));
                aim(true);
            }
            WM_SETTINGCHANGE if wp.0 as u32 == SPI_SETWORKAREA.0 => {
                CHECK.with(|c| c.set(true));
                aim(true);
            }
            _ => {}
        }
        unsafe { DefWindowProcW(h, msg, wp, lp) }
    }
}

#[cfg(test)]
pub fn reset_for_tests() {
    ST.with(|s| *s.borrow_mut() = None);
    MON.with(|m| m.set(0));
    REBUILD.with(|r| r.set(false));
    CHECK.with(|c| c.set(false));
}
