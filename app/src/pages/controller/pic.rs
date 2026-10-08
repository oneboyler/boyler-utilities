//! The controller pictures (menu-v22 `PADTYPES`, v22 "every outline and every button's place + size is MEASURED from a real
//! reference"): DualSense Edge / DualSense = a DualSense photo (x = 240 + (px - 367) * .66, y = 38 + (py - 88) * .66),
//! DualShock 4 = Wikimedia "Dualshock 4 Layout.svg" (x = 240 + (px - 480) * .497), Xbox Series = Wikimedia "Xbox Series
//! Controller Carbon Black.jpg" (halves averaged, height corrected). Picture = 480 x 326, mirrored at x 240. Every number below
//! is the drawing's own (its `smooth` / `mirrorBody` / `dpadParts` / `faceParts` / `pTab` / `pPill` / `pBack` / `pStick`), turned
//! into the same SVG path text (`f1` = JS `toFixed(1)`), so the shapes are the drawing's by construction.
//!
//! Painting follows the drawing's CSS (`.cps .bd`, `.cps .pp>.s/.g/.t/.cap`, `.bk`, `.lb`, `:hover`, `.sel`, `.dn`, `.tf`).

use bu_controller::{ButtonId, PadKind, Side};

use crate::gfx::{Gfx, Rgba};
use crate::ui::{ACC, CTL, CTL_H, FG2, FG3, SEL, WHITE};

/// One part of the picture (what a click picks).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pid {
    B(ButtonId),
    /// The Edge's Fn buttons (no Steam slot of their own - see the report)
    Fn(Side),
    Stick(Side),
    Trig(Side),
    Touch,
    Light,
    Gyro,
}

impl Pid {
    /// A short stable name (keys, the test hook).
    pub fn name(self) -> String {
        match self {
            Pid::B(b) => format!("{b:?}").to_lowercase(),
            Pid::Fn(Side::Left) => "fnl".into(),
            Pid::Fn(Side::Right) => "fnr".into(),
            Pid::Stick(Side::Left) => "ls".into(),
            Pid::Stick(Side::Right) => "rs".into(),
            Pid::Trig(Side::Left) => "l2".into(),
            Pid::Trig(Side::Right) => "r2".into(),
            Pid::Touch => "tp".into(),
            Pid::Light => "lt".into(),
            Pid::Gyro => "gy".into(),
        }
    }
}

/// One SVG shape in picture coordinates.
#[derive(Clone, Debug)]
pub enum Sh {
    Rect { x: f32, y: f32, w: f32, h: f32, r: f32 },
    Circle { cx: f32, cy: f32, r: f32 },
    Path(String),
}

impl Sh {
    fn bbox(&self, g: &Gfx) -> (f32, f32, f32, f32) {
        match self {
            Sh::Rect { x, y, w, h, .. } => (*x, *y, *x + *w, *y + *h),
            Sh::Circle { cx, cy, r } => (cx - r, cy - r, cx + r, cy + r),
            Sh::Path(d) => {
                let b = g.path(d).bounds().to_owned();
                (b.left, b.top, b.right, b.bottom)
            }
        }
    }
}

/// `.cps .pp>.t`: text-anchor middle at (x, baseline y); `xl` = the Xbox letters (inline fill + font-size, weight 700).
#[derive(Clone, Debug)]
pub struct Txt {
    pub x: f32,
    pub y: f32,
    pub s: String,
    pub size: f32,
    pub weight: u16,
    pub fill: Option<Rgba>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cls {
    Plain,
    /// `.pp.bk` (back buttons: dashed)
    Back,
    /// `.pp.lb` (the light bar: a coloured line)
    Light,
}

#[derive(Clone, Debug)]
pub struct Part {
    pub id: Pid,
    pub cls: Cls,
    /// `.s` shapes
    pub s: Vec<Sh>,
    /// `.g` glyphs (stroked)
    pub g: Vec<Sh>,
    pub t: Option<Txt>,
    /// `.cap` (stick caps): centre + radius
    pub cap: Option<(f32, f32, f32)>,
    /// a trigger (`.tf` fill, `.trg`)
    pub trig: bool,
}

impl Part {
    /// The part's box in picture coordinates (its shapes + cap), like `getBoundingClientRect` of its `<g>` (text aside).
    pub fn bbox(&self, g: &Gfx) -> (f32, f32, f32, f32) {
        let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for s in self.s.iter().chain(self.g.iter()) {
            let (l, t, r, bo) = s.bbox(g);
            b = (b.0.min(l), b.1.min(t), b.2.max(r), b.3.max(bo));
        }
        if self.cls == Cls::Light {
            // the 12 px wide invisible hit stroke
            b = (b.0 - 6.0, b.1 - 6.0, b.2 + 6.0, b.3 + 6.0);
        }
        b
    }

    /// Hover / click boxes in picture coordinates (the light bar = one box per strip, not the whole width).
    pub fn hit_boxes(&self, g: &Gfx) -> Vec<(f32, f32, f32, f32)> {
        if self.cls == Cls::Light {
            if let Some(Sh::Path(d)) = self.s.first() {
                // one sub-path per strip: "M..L..M..L.."
                return d
                    .split('M')
                    .filter(|p| !p.trim().is_empty())
                    .map(|p| {
                        let b = g.path(&format!("M{p}")).bounds().to_owned();
                        (b.left - 6.0, b.top - 6.0, b.right + 6.0, b.bottom + 6.0)
                    })
                    .collect();
            }
        }
        vec![self.bbox(g)]
    }

    /// Is the picture point inside the part's painted shape (SVG hit: fill, or the light bar's 12 px stroke)?
    pub fn contains(&self, g: &Gfx, x: f32, y: f32) -> bool {
        if self.cls == Cls::Light {
            if let Some(Sh::Path(d)) = self.s.first() {
                let pts = line_points(d);
                return pts.chunks(2).any(|s| s.len() == 2 && seg_dist((x, y), s[0], s[1]) <= 6.0);
            }
        }
        self.s.iter().any(|s| match s {
            Sh::Rect { x: rx, y: ry, w, h, .. } => x >= *rx && x <= rx + w && y >= *ry && y <= ry + h,
            Sh::Circle { cx, cy, r } => (x - cx).hypot(y - cy) <= *r + 0.6,
            Sh::Path(d) => g.path(d).contains((x, y)),
        })
    }
}

fn line_points(d: &str) -> Vec<(f32, f32)> {
    let nums: Vec<f32> = d.replace(['M', 'L'], " ").split_whitespace().filter_map(|v| v.parse().ok()).collect();
    nums.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0], c[1])).collect()
}

fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
}

/// One controller's picture.
#[derive(Clone, Debug)]
pub struct Pic {
    pub body: String,
    pub lines: String,
    /// `.bl` circles painted right after the body (DS4 rings, the Xbox d-pad bowl)
    pub deco: Vec<(f32, f32, f32)>,
    /// the parts in the drawing's order; the first four (the shoulder tabs) are painted before the body
    pub parts: Vec<Part>,
    /// where "Click buttons to edit" sits (picture y)
    pub hint: f32,
    /// the picture's height in picture px (480 wide): the drawing's 326, more when the shoulder buttons needed room (`lift`)
    pub h: f32,
}

pub const PIC_W: f32 = 480.0;
pub const PIC_H: f32 = 326.0;

/// JS `(+v).toFixed(1)` (ties away from zero like toFixed's "larger n"; -0 kept as JS prints it).
pub fn f1(v: f32) -> String {
    let v = v as f64;
    let n = (v * 10.0).abs();
    let r = if (n - n.floor() - 0.5).abs() < 1e-9 { n.floor() + 1.0 } else { n.round() };
    let r = r / 10.0 * v.signum();
    let s = format!("{r:.1}");
    if s == "0.0" && v < 0.0 {
        "-0.0".into()
    } else {
        s
    }
}

fn n1(v: f32) -> f32 {
    f1(v).parse().unwrap_or(v)
}

/// The drawing's `smooth(pts, closed)`: a smooth line through measured points (Catmull-Rom -> cubic Bezier).
pub fn smooth(pts: &[(f32, f32)], closed: bool) -> String {
    let n = pts.len() as i32;
    let p = |i: i32| -> (f32, f32) {
        if closed {
            pts[(((i % n) + n) % n) as usize]
        } else {
            pts[i.clamp(0, n - 1) as usize]
        }
    };
    let mut d = format!("M{} {}", f1(pts[0].0), f1(pts[0].1));
    let end = if closed { n } else { n - 1 };
    for i in 0..end {
        let (p0, p1, p2, p3) = (p(i - 1), p(i), p(i + 1), p(i + 2));
        d += &format!(
            "C{} {} {} {} {} {}",
            f1(p1.0 + (p2.0 - p0.0) / 6.0),
            f1(p1.1 + (p2.1 - p0.1) / 6.0),
            f1(p2.0 - (p3.0 - p1.0) / 6.0),
            f1(p2.1 - (p3.1 - p1.1) / 6.0),
            f1(p2.0),
            f1(p2.1)
        );
    }
    if closed {
        d.push('Z');
    }
    d
}

/// `mirrorBody(half)`: the left half (top centre -> bottom centre) + its mirror, one closed smooth line.
fn mirror_body(half: &[(f32, f32)]) -> String {
    let mut pts = half.to_vec();
    let mid: Vec<(f32, f32)> = half[1..half.len() - 1].iter().rev().map(|p| (PIC_W - p.0, p.1)).collect();
    pts.extend(mid);
    smooth(&pts, true)
}

/// `mirrorLine(pts)`: an open line and its mirror.
fn mirror_line(pts: &[(f32, f32)]) -> String {
    let m: Vec<(f32, f32)> = pts.iter().map(|p| (PIC_W - p.0, p.1)).collect();
    smooth(pts, false) + &smooth(&m, false)
}

fn rot(x: f32, y: f32, cx: f32, cy: f32, deg: f32) -> (f32, f32) {
    let a = (deg as f64).to_radians();
    let (c, s) = (a.cos() as f32, a.sin() as f32);
    (cx + (x - cx) * c - (y - cy) * s, cy + (x - cx) * s + (y - cy) * c)
}

/// `dpadParts(cx, cy, R, w, inn)`: four arms (a rounded bar pointing to the middle) + their arrow glyphs; the drawing rotates
/// the up arm with `transform="rotate(a cx cy)"` - done here on the (rounded) points, which is the same shape.
fn dpad(cx: f32, cy: f32, big_r: f32, w: f32, inn: f32) -> Vec<Part> {
    let q = (w / 4.0).min(4.0);
    let (x0, x1, top, base, gy) = (cx - w / 2.0, cx + w / 2.0, cy - big_r, cy - inn - w / 2.0, cy - big_r * 0.62);
    // the arm's points in drawing order (M, Q c, Q end, H, Q c, Q end, V, L, L) - each rounded like f1
    let arm = |a: f32| -> String {
        let r = |x: f32, y: f32| {
            let (px, py) = rot(n1(x), n1(y), n1(cx), n1(cy), a);
            format!("{} {}", px, py)
        };
        format!(
            "M{}Q{} {}H{}Q{} {}V{}L{}L{}Z",
            r(x0, top + q),
            r(x0, top),
            r(x0 + q, top),
            "", // replaced below: H/V are not rotatable, so the arm is written with L
            r(x1, top),
            r(x1, top + q),
            "",
            r(cx, cy - inn),
            r(x0, base)
        )
    };
    let _ = arm; // (kept for the reading: the arm as the drawing writes it)
    let arm_path = |a: f32| -> String {
        let r = |x: f32, y: f32| {
            let (px, py) = rot(n1(x), n1(y), n1(cx), n1(cy), a);
            format!("{px} {py}")
        };
        format!(
            "M{}Q{} {}L{}Q{} {}L{}L{}L{}Z",
            r(x0, top + q),
            r(x0, top),
            r(x0 + q, top),
            r(x1 - q, top),
            r(x1, top),
            r(x1, top + q),
            r(x1, base),
            r(cx, cy - inn),
            r(x0, base)
        )
    };
    let glyph = |a: f32| -> String {
        let p0 = (n1(cx - 2.7), n1(gy + 1.4));
        let p1 = (p0.0 + 2.7, p0.1 - 2.8);
        let p2 = (p1.0 + 2.7, p1.1 + 2.8);
        let r = |p: (f32, f32)| {
            let (x, y) = rot(p.0, p.1, n1(cx), n1(cy), a);
            format!("{x} {y}")
        };
        format!("M{}L{}L{}", r(p0), r(p1), r(p2))
    };
    [(ButtonId::DpadUp, 0.0), (ButtonId::DpadRight, 90.0), (ButtonId::DpadDown, 180.0), (ButtonId::DpadLeft, 270.0)]
        .into_iter()
        .map(|(id, a)| Part { id: Pid::B(id), cls: Cls::Plain, s: vec![Sh::Path(arm_path(a))], g: vec![Sh::Path(glyph(a))], t: None, cap: None, trig: false })
        .collect()
}

/// `faceParts(cx, cy, off, r, xb)`: Triangle (top), Circle (right), Cross (bottom), Square (left); PlayStation glyphs or the
/// Xbox letters in their colours.
fn face(cx: f32, cy: f32, off: f32, r: f32, xb: bool) -> Vec<Part> {
    let b = |id: ButtonId, x: f32, y: f32, g: Vec<Sh>, t: Option<Txt>| Part {
        id: Pid::B(id),
        cls: Cls::Plain,
        s: vec![Sh::Circle { cx: n1(x), cy: n1(y), r: n1(r) }],
        g,
        t,
        cap: None,
        trig: false,
    };
    if xb {
        let t = |x: f32, y: f32, l: &str, c: u32| {
            Some(Txt { x: n1(x), y: n1(y + r * 0.33), s: l.into(), size: n1(r * 0.95), weight: 700, fill: Some(Rgba::hex(c)) })
        };
        return vec![
            b(ButtonId::Triangle, cx, cy - off, vec![], t(cx, cy - off, "Y", 0xe8c33a)),
            b(ButtonId::Circle, cx + off, cy, vec![], t(cx + off, cy, "B", 0xff6b5e)),
            b(ButtonId::Cross, cx, cy + off, vec![], t(cx, cy + off, "A", 0x52d17c)),
            b(ButtonId::Square, cx - off, cy, vec![], t(cx - off, cy, "X", 0x4fa3ff)),
        ];
    }
    let k = r / 11.5;
    let tri = format!("M{} {}l{} {}h{}z", f1(cx), f1(cy - off - 5.4 * k), f1(5.0 * k), f1(8.6 * k), f1(-10.0 * k));
    let crs = format!(
        "M{} {}l{} {}M{} {}l{} {}",
        f1(cx - 4.2 * k),
        f1(cy + off - 4.2 * k),
        f1(8.4 * k),
        f1(8.4 * k),
        f1(cx + 4.2 * k),
        f1(cy + off - 4.2 * k),
        f1(-8.4 * k),
        f1(8.4 * k)
    );
    vec![
        b(ButtonId::Triangle, cx, cy - off, vec![Sh::Path(tri)], None),
        b(ButtonId::Circle, cx + off, cy, vec![Sh::Circle { cx: n1(cx + off), cy: n1(cy), r: n1(5.0 * k) }], None),
        b(ButtonId::Cross, cx, cy + off, vec![Sh::Path(crs)], None),
        b(ButtonId::Square, cx - off, cy, vec![Sh::Rect { x: n1(cx - off - 4.3 * k), y: n1(cy - 4.3 * k), w: n1(8.6 * k), h: n1(8.6 * k), r: 0.6 }], None),
    ]
}

/// `pStick(id, cx, cy, r, cap)`
fn stick(side: Side, cx: f32, cy: f32, r: f32, cap: f32) -> Part {
    Part { id: Pid::Stick(side), cls: Cls::Plain, s: vec![Sh::Circle { cx, cy, r }], g: vec![], t: None, cap: Some((cx, cy, cap)), trig: false }
}

fn txt(x: f32, y: f32, s: &str, size: f32) -> Option<Txt> {
    Some(Txt { x, y, s: s.into(), size, weight: 600, fill: None })
}

/// `pTab(id, x, y, w, h, lab, trig)`: L2 / R2 (fill as they are pulled) and L1 / R1 (the same tab, a button).
fn tab(id: Pid, x: f32, y: f32, lab: &str, trig: bool) -> Part {
    let (w, h) = (58.0, 20.0);
    Part { id, cls: Cls::Plain, s: vec![Sh::Rect { x, y, w, h, r: 8.0 }], g: vec![], t: txt(n1(x + w / 2.0), n1(y + h / 2.0 + 3.0), lab, 8.5), cap: None, trig }
}

/// `tabs(xL, y1, y2, n1, n2)`: L2, R2, L1, R1
fn tabs(xl: f32, y1: f32, y2: f32, n1s: [&str; 2], n2s: [&str; 2]) -> Vec<Part> {
    vec![
        tab(Pid::Trig(Side::Left), xl, y2, n2s[0], true),
        tab(Pid::Trig(Side::Right), PIC_W - xl - 58.0, y2, n2s[1], true),
        tab(Pid::B(ButtonId::L1), xl, y1, n1s[0], false),
        tab(Pid::B(ButtonId::R1), PIC_W - xl - 58.0, y1, n1s[1], false),
    ]
}

/// `pPill(id, x, y, w, h)`: rx = min(w, h) / 2
fn pill(id: Pid, x: f32, y: f32, w: f32, h: f32) -> Part {
    Part { id, cls: Cls::Plain, s: vec![Sh::Rect { x: n1(x), y: n1(y), w: n1(w), h: n1(h), r: n1(w.min(h) / 2.0) }], g: vec![], t: None, cap: None, trig: false }
}

/// `pBack(id, x, y, lab)`: 31 x 14, rx 7, dashed
fn back(id: ButtonId, x: f32, y: f32, lab: &str) -> Part {
    Part { id: Pid::B(id), cls: Cls::Back, s: vec![Sh::Rect { x, y, w: 31.0, h: 14.0, r: 7.0 }], g: vec![], t: txt(x + 15.5, y + 10.0, lab, 8.5), cap: None, trig: false }
}

fn ps_button(cy: f32, r: f32) -> Part {
    Part { id: Pid::B(ButtonId::Home), cls: Cls::Plain, s: vec![Sh::Circle { cx: 240.0, cy, r }], g: vec![], t: txt(240.0, n1(cy + 3.0), "PS", 7.0), cap: None, trig: false }
}

fn ps5_shell(edge: bool) -> Pic {
    // DualSense (photo -> picture: x = 240 + (px - 367) * .66, y = 38 + (py - 88) * .66)
    // the owner (test build 1): "dualsense looks weird, the outline of it". Re-traced from the same photo by its pixels (Order
    // 029): the body against the pure-white background (sides), the first body pixel under the black bumpers (shoulders),
    // the black centre / grip panel's edge seen from the middle (the arch between the grips); the photo's own symmetry
    // axis is px 359.25. The old points bulged the upper sides out by ~6 px, dipped 6 px beside the touchpad's corners and
    // put the arch 6.6 px high with a knee. The touchpad's top line and the grips' bottoms (blurred by the photo's shadow)
    // keep their measured points.
    let body = mirror_body(&[
        (240.0, 38.0),
        (195.8, 39.3),
        (149.6, 41.3),
        (133.6, 43.3),
        (109.8, 46.6),
        (86.1, 51.2),
        (74.2, 53.8),
        (66.3, 60.4),
        (58.3, 77.6),
        (50.4, 96.7),
        (42.5, 121.2),
        (34.6, 150.2),
        (29.3, 177.9),
        (26.0, 204.3),
        (24.0, 230.7),
        (22.7, 257.1),
        (22.0, 283.5),
        (22.7, 296.7),
        (30.8, 310.0),
        (55.2, 318.5),
        (79.0, 318.5),
        (88.2, 310.0),
        (95.3, 288.8),
        (101.9, 273.0),
        (109.8, 259.5),
        (117.7, 249.2),
        (125.7, 239.3),
        (133.6, 233.4),
        (145.5, 227.4),
        (165.3, 226.5),
        (200.9, 226.5),
        (240.0, 225.4),
    ]);
    let lines = mirror_line(&[(160.5, 118.0), (150.0, 134.0), (139.7, 144.9), (123.2, 177.9), (103.4, 210.9), (83.6, 244.0), (71.7, 277.0), (63.8, 318.5)]);
    let mut parts = tabs(76.0, 28.0, 4.0, ["L1", "R1"], ["L2", "R2"]);
    parts.push(Part {
        id: Pid::Touch,
        cls: Cls::Plain,
        s: vec![Sh::Path("M151.5 39.6H328.5L318.6 116Q317 126.8 306 126.8H174Q163 126.8 161.4 116Z".into())],
        g: vec![],
        t: None,
        cap: None,
        trig: false,
    });
    parts.push(Part { id: Pid::Light, cls: Cls::Light, s: vec![Sh::Path("M152.5 42L161 112M327.5 42L319 112".into())], g: vec![], t: None, cap: None, trig: false });
    parts.push(pill(Pid::B(ButtonId::Create), 131.9, 57.8, 11.0, 20.5));
    parts.push(pill(Pid::B(ButtonId::Options), 337.1, 57.8, 11.0, 20.5));
    parts.extend(dpad(103.0, 115.2, 36.0, 27.0, 5.0));
    parts.extend(face(377.0, 115.2, 32.0, 14.5, false));
    parts.push(stick(Side::Left, 170.0, 177.3, 28.0, 24.0));
    parts.push(stick(Side::Right, 310.0, 177.3, 28.0, 24.0));
    parts.push(ps_button(171.3, 9.5));
    parts.push(pill(Pid::B(ButtonId::Mute), 230.0, 195.6, 20.0, 5.6));
    if edge {
        parts.push(pill(Pid::Fn(Side::Left), 163.0, 212.5, 16.0, 6.0));
        parts.push(pill(Pid::Fn(Side::Right), 301.0, 212.5, 16.0, 6.0));
        parts.push(back(ButtonId::BackLeftUpper, 34.0, 232.0, "L4"));
        parts.push(back(ButtonId::BackLeftLower, 44.0, 258.0, "L5"));
        parts.push(back(ButtonId::BackRightUpper, 415.0, 232.0, "R4"));
        parts.push(back(ButtonId::BackRightLower, 405.0, 258.0, "R5"));
    }
    Pic { body, lines, deco: vec![], parts, hint: 282.0, h: PIC_H }
}

fn ds4() -> Pic {
    // DualShock 4 (svg -> picture: x = 240 + (px - 480) * .497, y = 50 + (py - 72) * .497)
    let body = mirror_body(&[
        (240.0, 50.0),
        (169.2, 50.0),
        (120.0, 50.8),
        (76.1, 51.2),
        (51.3, 96.2),
        (38.9, 138.4),
        (32.4, 173.2),
        (26.5, 212.9),
        (22.5, 242.7),
        (22.0, 262.5),
        (33.9, 287.4),
        (51.3, 302.3),
        (71.2, 308.2),
        (101.0, 297.3),
        (115.9, 273.7),
        (125.8, 245.2),
        (135.7, 216.6),
        (143.2, 211.6),
        (170.5, 221.6),
        (200.3, 210.4),
        (240.0, 210.4),
    ]);
    let lines = mirror_line(&[(22.7, 242.7), (50.7, 272.5), (88.5, 279.9), (115.8, 273.7)]);
    let mut parts = tabs(78.0, 29.0, 5.0, ["L1", "R1"], ["L2", "R2"]);
    parts.push(Part { id: Pid::Light, cls: Cls::Light, s: vec![Sh::Path("M184 45.5H296".replace("H296", "L296 45.5"))], g: vec![], t: None, cap: None, trig: false });
    parts.push(Part {
        id: Pid::Touch,
        cls: Cls::Plain,
        s: vec![Sh::Path("M169.2 50.5H310.8V122Q310.8 129.5 303.3 129.5H176.7Q169.2 129.5 169.2 122Z".into())],
        g: vec![],
        t: None,
        cap: None,
        trig: false,
    });
    parts.push(pill(Pid::B(ButtonId::Create), 144.8, 59.5, 13.7, 24.3));
    parts.push(pill(Pid::B(ButtonId::Options), 321.5, 59.5, 13.7, 24.3));
    parts.extend(dpad(103.9, 113.6, 36.0, 24.3, 6.2));
    parts.extend(face(376.1, 113.6, 32.8, 13.7, false));
    parts.push(stick(Side::Left, 170.5, 178.1, 30.0, 22.0));
    parts.push(stick(Side::Right, 309.5, 178.1, 30.0, 22.0));
    parts.push(ps_button(176.1, 9.0));
    Pic { body, lines, deco: vec![(103.9, 113.6, 50.0), (376.1, 113.6, 50.0), (170.5, 178.1, 42.0), (309.5, 178.1, 42.0)], parts, hint: 272.0, h: PIC_H }
}

fn xbox() -> Pic {
    // Xbox Series (photo -> picture: x = 240 - (521 - px) * .531, y = 44 + (py - 128) * .529)
    let body = mirror_body(&[
        (240.0, 44.0),
        (202.3, 43.5),
        (165.2, 42.4),
        (122.7, 46.1),
        (98.8, 55.6),
        (77.6, 87.4),
        (59.0, 135.0),
        (45.7, 182.6),
        (32.5, 246.1),
        (32.5, 283.1),
        (45.7, 309.6),
        (69.6, 322.8),
        (96.1, 301.6),
        (128.0, 261.9),
        (154.5, 248.7),
        (240.0, 248.7),
    ]);
    let mut parts = tabs(106.0, 25.0, 1.0, ["LB", "RB"], ["LT", "RT"]);
    parts.push(Part {
        id: Pid::B(ButtonId::Home),
        cls: Cls::Plain,
        s: vec![Sh::Circle { cx: 240.0, cy: 72.8, r: 15.0 }],
        g: vec![Sh::Path("M234 66.8l12 12M246 66.8l-12 12".into())],
        t: None,
        cap: None,
        trig: false,
    });
    let circ = |id: ButtonId, cx: f32| Part { id: Pid::B(id), cls: Cls::Plain, s: vec![Sh::Circle { cx, cy: 115.2, r: 8.5 }], g: vec![], t: None, cap: None, trig: false };
    parts.push(circ(ButtonId::Create, 206.4));
    parts.push(circ(ButtonId::Options, 273.6));
    parts.push(pill(Pid::B(ButtonId::Mute), 229.0, 132.4, 22.0, 13.0));
    parts.push(stick(Side::Left, 120.0, 112.5, 31.0, 20.0));
    parts.extend(dpad(175.0, 189.9, 34.7, 23.5, 0.0));
    parts.extend(face(360.0, 116.7, 28.0, 14.0, true));
    parts.push(stick(Side::Right, 305.0, 182.6, 31.0, 20.0));
    Pic { body, lines: String::new(), deco: vec![(175.0, 189.9, 37.0)], parts, hint: 282.0, h: PIC_H }
}

/// The picture of a controller type.
pub fn pic(kind: PadKind) -> Pic {
    lift(match kind {
        PadKind::DualSenseEdge => ps5_shell(true),
        PadKind::DualSense => ps5_shell(false),
        PadKind::DualShock4 => ds4(),
        PadKind::Xbox => xbox(),
    })
}

/// The clear space between the body's outline and L1 / R1, and between L1 / R1 and L2 / R2 - edge to edge, strokes
/// included (the drawing's own L2 -> L1 space: L2 ends at y 24, L1 starts at 28).
pub const SHOULDER_GAP: f32 = 4.0;
/// `.cps .bd{stroke-width:1.3}` and `.cps .pp>.s{stroke-width:1.2}`
const BODY_STROKE: f32 = 1.3;
const PART_STROKE: f32 = 1.2;

/// The highest point (smallest y) of a path's outline between x0 and x1 (the outline walked every 0.25 px).
pub fn top_between(d: &str, x0: f32, x1: f32) -> f32 {
    let path = crate::svg::to_path(&crate::svg::parse(d));
    let mut top = f32::MAX;
    for c in skia_safe::ContourMeasureIter::new(&path, false, None) {
        let len = c.length();
        let n = (len / 0.25).ceil().max(1.0) as usize;
        for i in 0..=n {
            if let Some((p, _)) = c.pos_tan(len * i as f32 / n as f32) {
                if p.x >= x0 && p.x <= x1 {
                    top = top.min(p.y);
                }
            }
        }
    }
    top
}

/// the owner (test build 1): "on all of them, the l1 r1 button sticks into the controller instead of being above it ... lock in
/// move them both up". L1 / R1 go [`SHOULDER_GAP`] above the body's outline (its highest point under the tab), L2 / R2 the
/// same gap above them. A picture whose L2 / R2 would then leave the top is moved down as a whole and grows by as much.
fn lift(mut p: Pic) -> Pic {
    // parts 0..4 = L2, R2, L1, R1 (`tabs`); the picture is mirrored, so the left tab decides for both
    let Some(Sh::Rect { x, w, h, .. }) = p.parts.get(2).and_then(|t| t.s.first()).cloned() else { return p };
    let body_top = top_between(&p.body, x, x + w) - BODY_STROKE / 2.0;
    // a tab's painted box: y - .6 .. y + h + .6
    let l1 = body_top - SHOULDER_GAP - PART_STROKE / 2.0 - h;
    let l2 = l1 - PART_STROKE / 2.0 - SHOULDER_GAP - PART_STROKE / 2.0 - h;
    for (i, part) in p.parts.iter_mut().take(4).enumerate() {
        let ny = if i < 2 { l2 } else { l1 };
        if let Some(Sh::Rect { y, h, .. }) = part.s.first_mut() {
            *y = ny;
            if let Some(t) = part.t.as_mut() {
                t.y = n1(ny + *h / 2.0 + 3.0);
            }
        }
    }
    // whole tenths: the path text keeps one decimal (`f1`), so the moved outline is the same shape, not a rounded one
    let dy = ((PART_STROKE / 2.0 - l2).max(0.0) * 10.0).ceil() / 10.0;
    if dy > 0.0 {
        shift(&mut p, dy);
        p.h += dy;
    }
    p
}

/// Moves a whole picture down by `dy`.
fn shift(p: &mut Pic, dy: f32) {
    let sh = |s: &mut Sh| match s {
        Sh::Rect { y, .. } => *y += dy,
        Sh::Circle { cy, .. } => *cy += dy,
        Sh::Path(d) => *d = shift_path(d, dy),
    };
    p.body = shift_path(&p.body, dy);
    p.lines = shift_path(&p.lines, dy);
    for d in &mut p.deco {
        d.1 += dy;
    }
    for part in &mut p.parts {
        part.s.iter_mut().for_each(sh);
        part.g.iter_mut().for_each(sh);
        if let Some(t) = part.t.as_mut() {
            t.y += dy;
        }
        if let Some(c) = part.cap.as_mut() {
            c.1 += dy;
        }
    }
    p.hint += dy;
}

/// SVG path text moved down by `dy`: the y of every absolute point (M L C S Q T: x y pairs, V: y); relative commands and
/// H are unchanged. (The pictures use M L H V C Q Z and relative l - no arcs.)
pub fn shift_path(d: &str, dy: f32) -> String {
    let mut out = String::with_capacity(d.len() + 16);
    let mut cmd = 'M';
    let mut n = 0usize;
    let mut num = String::new();
    let flush = |num: &mut String, out: &mut String, cmd: char, n: &mut usize| {
        if num.is_empty() {
            return;
        }
        let v: f32 = num.parse().unwrap_or(0.0);
        let is_y = match cmd {
            'M' | 'L' | 'C' | 'S' | 'Q' | 'T' => *n % 2 == 1,
            'V' => true,
            _ => false,
        };
        if !out.is_empty() && !out.ends_with(|c: char| c.is_ascii_alphabetic()) {
            out.push(' ');
        }
        out.push_str(&if is_y { f1(v + dy) } else { num.clone() });
        *n += 1;
        num.clear();
    };
    for c in d.chars() {
        if c.is_ascii_alphabetic() {
            flush(&mut num, &mut out, cmd, &mut n);
            cmd = c;
            n = 0;
            out.push(c);
        } else if c == '-' {
            flush(&mut num, &mut out, cmd, &mut n);
            num.push(c);
        } else if c.is_ascii_digit() || c == '.' {
            num.push(c);
        } else {
            flush(&mut num, &mut out, cmd, &mut n);
        }
    }
    flush(&mut num, &mut out, cmd, &mut n);
    out
}

/// How one part looks this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Look {
    /// `:hover` / `.hv` 0..1 (`transition: fill .15s ease, stroke .15s ease`)
    pub hv: f32,
    pub sel: bool,
    /// live: pressed (`.dn`)
    pub dn: bool,
    /// live: trigger pull 0..1 (`.tf` opacity = pull x .85)
    pub pull: f32,
    /// live: stick cap offset in picture px (`translate(LV * 7)`)
    pub cap_off: (f32, f32),
}

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    crate::ui::cmix(a, b, t)
}

thread_local! {
    static SVG_FONTS: std::cell::RefCell<std::collections::HashMap<(u32, u16), skia_safe::Font>> = Default::default();
}

/// SVG `<text>` with `text-anchor: middle` at (x, baseline y): "Segoe UI Variable Text" at the size / weight, letter-spacing
/// inherited from `#sw` (-.006em of 13 px = -0.078 px).
fn svg_text(g: &Gfx, t: &Txt, c: Rgba) {
    use skia_safe as sk;
    let font = SVG_FONTS.with(|m| {
        m.borrow_mut()
            .entry(((t.size * 100.0) as u32, t.weight))
            .or_insert_with(|| {
                let fm = sk::FontMgr::default();
                let st = sk::FontStyle::new(sk::font_style::Weight::from(t.weight as i32), sk::font_style::Width::NORMAL, sk::font_style::Slant::Upright);
                let tf = fm.match_family_style("Segoe UI Variable Text", st).or_else(|| fm.match_family_style("Segoe UI", st)).expect("font");
                let mut f = sk::Font::from_typeface(tf, t.size);
                // the same font setup as the page's text (gfx.rs sk_font: Chromium's)
                f.set_subpixel(true).set_hinting(sk::FontHinting::Normal).set_edging(sk::font::Edging::SubpixelAntiAlias).set_embedded_bitmaps(true).set_linear_metrics(false);
                f
            })
            .clone()
    });
    let glyphs = font.str_to_glyphs_vec(&t.s);
    let mut w = vec![0.0f32; glyphs.len()];
    font.get_widths(&glyphs, &mut w);
    let ls = -0.078f32;
    let total: f32 = w.iter().sum::<f32>() + ls * glyphs.len() as f32;
    let mut x = t.x - total / 2.0;
    let mut xs = Vec::with_capacity(glyphs.len());
    for wi in &w {
        xs.push(x);
        x += wi + ls;
    }
    if let Some(blob) = sk::TextBlob::from_pos_text_h(&glyphs[..], &xs, t.y, &font) {
        let mut p = sk::Paint::new(c.c4(), None);
        p.set_anti_alias(true);
        g.cv().draw_text_blob(&blob, (0.0, 0.0), &p);
    }
}

fn fill_sh(g: &Gfx, s: &Sh, c: Rgba) {
    match s {
        Sh::Rect { x, y, w, h, r } => g.fill_rr(*x, *y, *w, *h, *r, c),
        Sh::Circle { cx, cy, r } => g.fill_circle(*cx, *cy, *r, c),
        Sh::Path(d) => g.fill_geom(&g.path(d), c),
    }
}

fn stroke_sh(g: &Gfx, s: &Sh, w: f32, c: Rgba, dash: bool) {
    let path = match s {
        Sh::Rect { x, y, w, h, r } => g.rr_path(*x, *y, *w, *h, *r),
        Sh::Circle { cx, cy, r } => skia_safe::Path::circle((*cx, *cy), *r, None),
        Sh::Path(d) => g.path(d),
    };
    if dash {
        // `stroke-dasharray: 3 2` (butt caps, miter joins like SVG's default)
        use skia_safe as sk;
        let mut p = sk::Paint::new(c.c4(), None);
        p.set_anti_alias(true).set_style(sk::PaintStyle::Stroke).set_stroke_width(w);
        p.set_path_effect(sk::PathEffect::dash(&[3.0, 2.0], 0.0));
        g.cv().draw_path(&path, &p);
    } else {
        // SVG default caps / joins (butt, miter) for `.s`
        g.stroke_geom_ex(&path, w, c, false, true, 1.0);
    }
}

/// Paint the picture into (x, y, w) (height = w x 326 / 480), the parts looking as `look(i)` says; `lc` = the light bar's
/// colour (`--lc`, the controller's light; None = `--fg3`).
pub fn paint(g: &Gfx, p: &Pic, (x, y, w): (f32, f32, f32), look: &dyn Fn(usize) -> Look, lc: Option<Rgba>) {
    let s = w / PIC_W;
    let t0 = g.transform();
    g.set_transform(&(windows_numerics::Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: x, M32: y } * t0));
    let body = |g: &Gfx| {
        // `.cps .bd{fill:rgba(255,255,255,.035);stroke:var(--fg2);stroke-width:1.3;stroke-linejoin:round}`
        let b = g.path(&p.body);
        // (light, Order 033: `#sw.light .cps .bd{fill:rgba(255,255,255,.5)}`)
        g.fill_geom(&b, Rgba(1.0, 1.0, 1.0, if crate::ui::is_light() { 0.5 } else { 0.035 }));
        g.stroke_geom_ex(&b, 1.3, FG2(), false, true, 1.0);
        // `.cps .bl{fill:none;stroke:var(--fg3);stroke-width:1;opacity:.55}`
        let bl = FG3().mul_a(0.55);
        if !p.lines.is_empty() {
            g.stroke_geom_ex(&g.path(&p.lines), 1.0, bl, false, true, 1.0);
        }
        for (cx, cy, r) in &p.deco {
            g.stroke_geom_ex(&skia_safe::Path::circle((*cx, *cy), *r, None), 1.0, bl, false, true, 1.0);
        }
    };
    for (i, part) in p.parts.iter().enumerate() {
        if i == 4 {
            body(g);
        }
        paint_part(g, part, look(i), lc);
    }
    if p.parts.len() <= 4 {
        body(g);
    }
    g.set_transform(&t0);
}

fn paint_part(g: &Gfx, part: &Part, l: Look, lc: Option<Rgba>) {
    let on = if l.sel { 1.0 } else { l.hv };
    if part.cls == Cls::Light {
        // `.cps .pp.lb>.s{fill:none;stroke:var(--lc,var(--fg3));stroke-width:2.2;stroke-linecap:round;opacity:.9}`, hover / sel = --acc
        let c = mix(lc.unwrap_or(FG3()), ACC(), on).mul_a(0.9);
        for s in &part.s {
            if let Sh::Path(d) = s {
                g.stroke_geom_ex(&g.path(d), 2.2, c, true, true, 1.0);
            }
        }
        return;
    }
    // `.s{fill:var(--ctl);stroke:var(--fg3);stroke-width:1.2}` `:hover>.s{fill:var(--sel);stroke:var(--acc)}`
    // `.sel>.s{stroke-width:1.8}` `.bk>.s{stroke-dasharray:3 2;fill:rgba(255,255,255,.02)}`
    // `.dn>.s{fill:var(--acc);stroke:var(--acc)}` `.trg.dn>.s{fill:var(--ctl);stroke:var(--acc)}`
    let base_fill = if part.cls == Cls::Back { Rgba(1.0, 1.0, 1.0, 0.02) } else { CTL() };
    let mut fill = mix(base_fill, SEL(), on);
    let mut stroke = mix(FG3(), ACC(), on);
    let sw = if l.sel { 1.8 } else { 1.2 };
    if l.dn {
        if part.trig {
            fill = CTL();
        } else {
            fill = ACC();
        }
        stroke = ACC();
    }
    for s in &part.s {
        fill_sh(g, s, fill);
        stroke_sh(g, s, sw, stroke, part.cls == Cls::Back);
    }
    if part.trig && l.pull > 0.0 {
        // `.cps .tf{fill:var(--acc);opacity:0}` -> pull x .85
        for s in &part.s {
            fill_sh(g, s, ACC().mul_a((l.pull * 0.85).min(1.0)));
        }
    }
    // `.g{fill:none;stroke:var(--fg2);stroke-width:1.3;round caps / joins}` hover / sel = --acc, `.dn>.g` = #fff
    let gc = if l.dn { WHITE } else { mix(FG2(), ACC(), on) };
    for s in &part.g {
        let path = match s {
            Sh::Rect { x, y, w, h, r } => g.rr_path(*x, *y, *w, *h, *r),
            Sh::Circle { cx, cy, r } => skia_safe::Path::circle((*cx, *cy), *r, None),
            Sh::Path(d) => g.path(d),
        };
        g.stroke_geom(&path, 1.3, gc);
    }
    // `.cap{fill:var(--ctl-h);stroke:var(--fg3);stroke-width:1}` hover / sel stroke --acc; `.dn>.cap{fill:var(--acc);stroke:#fff}`
    if let Some((cx, cy, r)) = part.cap {
        let (cx, cy) = (cx + l.cap_off.0, cy + l.cap_off.1);
        let (cf, cs) = if l.dn { (ACC(), WHITE) } else { (CTL_H(), mix(FG3(), ACC(), on)) };
        g.fill_circle(cx, cy, r, cf);
        g.stroke_geom_ex(&skia_safe::Path::circle((cx, cy), r, None), 1.0, cs, false, true, 1.0);
    }
    // `.t{font:600 8.5px/1;fill:var(--fg3)}` hover / sel --acc, `.dn>.t` #fff; the Xbox letters keep their inline colour
    if let Some(t) = &part.t {
        let c = match t.fill {
            Some(c) => c,
            None if l.dn => WHITE,
            None => mix(FG3(), ACC(), on),
        };
        svg_text(g, t, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f1_is_js_to_fixed() {
        assert_eq!(f1(103.0), "103.0");
        assert_eq!(f1(-0.04), "-0.0");
        assert_eq!(f1(12.25), "12.3");
        assert_eq!(f1(-2.25), "-2.3");
        assert_eq!(f1(115.2), "115.2");
    }

    /// The smoothing gives the drawing's own path text (first segment of the DualSense body, computed by the drawing's JS).
    #[test]
    fn body_path_starts_like_the_drawing() {
        // (the shell as measured, before `lift` moves it down for its shoulder buttons)
        let p = ps5_shell(true);
        assert!(p.body.starts_with("M240.0 38.0C"), "{}", &p.body[..40]);
        assert!(p.body.ends_with('Z'));
        // 62 points (32 + 30 mirrored) = 62 curve segments
        assert_eq!(p.body.matches('C').count(), 62);
    }

    #[test]
    fn every_controller_has_its_parts() {
        let ids = |k: PadKind| pic(k).parts.iter().map(|p| p.id).collect::<Vec<_>>();
        let edge = ids(PadKind::DualSenseEdge);
        assert_eq!(edge.len(), 4 + 2 + 2 + 4 + 4 + 2 + 2 + 2 + 4);
        assert!(edge.contains(&Pid::Fn(Side::Left)) && edge.contains(&Pid::B(ButtonId::BackRightUpper)));
        let ds = ids(PadKind::DualSense);
        assert!(!ds.contains(&Pid::Fn(Side::Left)) && !ds.contains(&Pid::B(ButtonId::BackLeftLower)));
        let ds4 = ids(PadKind::DualShock4);
        assert!(!ds4.contains(&Pid::B(ButtonId::Mute)) && ds4.contains(&Pid::Light));
        let xb = ids(PadKind::Xbox);
        assert!(!xb.contains(&Pid::Touch) && !xb.contains(&Pid::Light));
        assert_eq!(super::ds4().hint, 272.0);
        let d4 = pic(PadKind::DualShock4);
        assert_eq!(d4.hint, 272.0 + (d4.h - PIC_H), "the hint moves with the picture");
    }

    #[test]
    fn exact_hits() {
        let g = Gfx::new(1.0);
        let p = pic(PadKind::DualSenseEdge);
        let at = |x: f32, y: f32| p.parts.iter().rev().find(|q| q.contains(&g, x, y)).map(|q| q.id);
        assert_eq!(at(377.0, 147.2), Some(Pid::B(ButtonId::Cross)));
        assert_eq!(at(103.0, 90.0), Some(Pid::B(ButtonId::DpadUp)));
        assert_eq!(at(130.0, 115.2), Some(Pid::B(ButtonId::DpadRight)));
        assert_eq!(at(170.0, 177.3), Some(Pid::Stick(Side::Left)));
        assert_eq!(at(240.0, 80.0), Some(Pid::Touch));
        assert_eq!(at(157.0, 77.0), Some(Pid::Light));
        assert_eq!(at(240.0, 260.0), None);
    }
}
