//! Monitors and rectangles in desktop pixels (physical pixels, the coordinates Windows' Display settings uses), plus the small
//! pieces of overlay logic DESIGN §3.3 describes that are pure math: the monitor under the mouse, "lit while the box equals it
//! exactly", typing a size ("the first two numbers with any separator", clamped 4 px … the whole desktop, top-left stays, slides back).

/// A rectangle in desktop pixels. `x`/`y` can be negative (a monitor left of / above the main one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> i64 {
        self.x as i64 + self.w as i64
    }
    pub fn bottom(&self) -> i64 {
        self.y as i64 + self.h as i64
    }
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }
    pub fn contains(&self, px: i32, py: i32) -> bool {
        (px as i64) >= self.x as i64 && (px as i64) < self.right() && (py as i64) >= self.y as i64 && (py as i64) < self.bottom()
    }
    /// The overlap of two rectangles (empty → `None`).
    pub fn intersect(&self, o: &Rect) -> Option<Rect> {
        let l = (self.x as i64).max(o.x as i64);
        let t = (self.y as i64).max(o.y as i64);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        (r > l && b > t).then(|| Rect::new(l as i32, t as i32, (r - l) as u32, (b - t) as u32))
    }
    /// The smallest rectangle holding both.
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let l = (self.x as i64).min(o.x as i64);
        let t = (self.y as i64).min(o.y as i64);
        let r = self.right().max(o.right());
        let b = self.bottom().max(o.bottom());
        Rect::new(l as i32, t as i32, (r - l) as u32, (b - t) as u32)
    }
}

/// How the monitor is turned in Windows (Display settings › Display orientation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rotation {
    #[default]
    None,
    Cw90,
    Cw180,
    Cw270,
}

/// One monitor as the engine sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    /// 1, 2, 3 … — numbered left to right (then top to bottom) by position, which is what the overlay's [1] [2] buttons follow.
    pub number: usize,
    /// Windows' device name, e.g. `\\.\DISPLAY1`.
    pub device: String,
    /// Where it sits on the desktop, in physical pixels; `w`×`h` is its exact resolution.
    pub rect: Rect,
    pub primary: bool,
    /// The scaling Windows uses on it (96 = 100 %, 144 = 150 %).
    pub dpi: u32,
    pub rotation: Rotation,
    /// HDR (Windows HD Color) is on for this monitor.
    pub hdr: bool,
    /// Windows' "SDR content brightness" white on this monitor, in nits (80 = scRGB 1.0). Only used for HDR monitors.
    pub sdr_white_nits: f32,
    /// Opaque handle the real OS layer uses to find the monitor again (HMONITOR as an integer). 0 in the fake.
    pub handle: isize,
}

/// Numbers monitors 1, 2, 3 … left to right, then top to bottom (stable for a given layout).
pub fn number_monitors(monitors: &mut [Monitor]) {
    monitors.sort_by_key(|m| (m.rect.x, m.rect.y));
    for (i, m) in monitors.iter_mut().enumerate() {
        m.number = i + 1;
    }
}

/// The bounding box of every monitor = the "All" picture. Gaps between monitors of different sizes are part of it (black).
pub fn desktop_bounds(monitors: &[Monitor]) -> Rect {
    monitors.iter().fold(Rect::default(), |acc, m| acc.union(&m.rect))
}

/// The monitor under a desktop point (DESIGN: "a click = the whole monitor … the one under the mouse").
pub fn monitor_at(monitors: &[Monitor], x: i32, y: i32) -> Option<&Monitor> {
    monitors.iter().find(|m| m.rect.contains(x, y))
}

/// Which preset button is "lit": `Some(n)` when the box equals monitor n exactly, `Some(0)` when it equals All.
pub fn preset_matching(monitors: &[Monitor], b: &Rect) -> Option<usize> {
    if let Some(m) = monitors.iter().find(|m| m.rect == *b) {
        return Some(m.number);
    }
    (monitors.len() > 1 && desktop_bounds(monitors) == *b).then_some(0)
}

/// The smallest box side the overlay accepts (DESIGN: "A box under 4 px reverts", typed sizes clamp at 4 px).
pub const MIN_SIDE: u32 = 4;

/// Reads a typed size: the first two whole numbers in the text, any separator ("1920x1080", "1920 × 1080", "1920,1080").
pub fn parse_size(text: &str) -> Option<(u32, u32)> {
    let mut nums = Vec::with_capacity(2);
    let mut cur: Option<u64> = None;
    for c in text.chars().chain(std::iter::once(' ')) {
        if let Some(d) = c.to_digit(10) {
            cur = Some(cur.unwrap_or(0).saturating_mul(10).saturating_add(d as u64));
        } else if let Some(n) = cur.take() {
            nums.push(n.min(u32::MAX as u64) as u32);
            if nums.len() == 2 {
                break;
            }
        }
    }
    (nums.len() == 2).then(|| (nums[0], nums[1]))
}

/// Applies a typed size to a box (DESIGN §3.3 "Size tag"): width and height clamp to 4 px … the whole desktop, the top-left
/// stays put, and the box slides back if it would run off the desktop.
pub fn fit_typed_size(current: &Rect, w: u32, h: u32, desktop: &Rect) -> Rect {
    let w = w.clamp(MIN_SIDE.min(desktop.w), desktop.w);
    let h = h.clamp(MIN_SIDE.min(desktop.h), desktop.h);
    let max_x = desktop.right() - w as i64;
    let max_y = desktop.bottom() - h as i64;
    let x = (current.x as i64).clamp(desktop.x as i64, max_x);
    let y = (current.y as i64).clamp(desktop.y as i64, max_y);
    Rect::new(x as i32, y as i32, w, h)
}

/// Clamps a dragged box to the desktop (whole pixels). `None` if what is left is under 4 px on a side (the box reverts).
pub fn clamp_box(b: &Rect, desktop: &Rect) -> Option<Rect> {
    b.intersect(desktop).filter(|r| r.w >= MIN_SIDE && r.h >= MIN_SIDE)
}
