//! The Mouse page's own painting (things that are not boxes): the cursor pictures in the bubbles and the role picker
//! (the drawing's CSH shapes on a 32 px grid, each cursor set's CLOOK / CWIN look), the battery level, the double-click
//! test folder and Raw Accel's graph. Every number is the drawing's (menu-v22 MOUSE PAGE); SVG is drawn the way Blink
//! paints it (objectBoundingBox gradients, feDropShadow as a blur layer, dash arcs with round caps).

use skia_safe as sk;
use skia_safe::{gradient_shader, Paint, PaintStyle};

use crate::gfx::{Font, Gfx, Rgba};

/// One cursor set's look (`CLOOK` / `CWIN`): fill gradient top / bottom, fill opacity, the solid colour (I-beam), the rim and
/// its width / join, the drop shadow [dx, dy, blur (stdDeviation), colour, opacity], the spinner's track / arc, size k.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub fill: (Rgba, Rgba),
    pub op: f32,
    pub solid: Rgba,
    pub rim: Rgba,
    pub rw: f32,
    pub round: bool,
    pub sh: (f32, f32, f32, Rgba),
    pub track: Rgba,
    pub arc: Rgba,
    pub k: f32,
    /// Windows default: the spinner has no rim ring
    pub win: bool,
}

const fn hex(h: u32) -> Rgba {
    Rgba::rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

/// `CWIN={fill:['#ffffff','#ffffff'],op:1,solid:'#ffffff',rim:'#000000',rw:1,join:'miter',sh:[0,.8,.8,'#000',.3],
///   track:'rgba(42,140,255,.18)',arc:'#2a8cff',win:true}`
pub const WIN: Look = Look {
    fill: (hex(0xffffff), hex(0xffffff)),
    op: 1.0,
    solid: hex(0xffffff),
    rim: hex(0x000000),
    rw: 1.0,
    round: false,
    sh: (0.0, 0.8, 0.8, Rgba(0.0, 0.0, 0.0, 0.3)),
    track: Rgba::rgba(42, 140, 255, 0.18),
    arc: hex(0x2a8cff),
    k: 1.0,
    win: true,
};
/// `glass:{fill:['#ffffff','#dfe6f1'],op:.94,solid:'#f6f8fc',rim:'rgba(16,20,30,.55)',rw:1,join:'round',sh:[0,1.2,1.3,'#000',.36],
///   track:'#9eaabd',arc:'#ffffff'}`
pub const GLASS: Look = Look {
    fill: (hex(0xffffff), hex(0xdfe6f1)),
    op: 0.94,
    solid: hex(0xf6f8fc),
    rim: Rgba::rgba(16, 20, 30, 0.55),
    rw: 1.0,
    round: true,
    sh: (0.0, 1.2, 1.3, Rgba(0.0, 0.0, 0.0, 0.36)),
    track: hex(0x9eaabd),
    arc: hex(0xffffff),
    k: 1.0,
    win: false,
};
/// `neon:{fill:['#18263f','#0a1120'],op:.86,solid:'#0d1626',rim:'#3d9bff',rw:1.3,join:'round',sh:[0,0,1.5,'#3d9bff',.85],
///   track:'#0e1626',arc:'#7cc0ff'}` - the look an imported pack is drawn with until its real files are read (see the report).
pub const NEON: Look = Look {
    fill: (hex(0x18263f), hex(0x0a1120)),
    op: 0.86,
    solid: hex(0x0d1626),
    rim: hex(0x3d9bff),
    rw: 1.3,
    round: true,
    sh: (0.0, 0.0, 1.5, Rgba::rgba(61, 155, 255, 0.85)),
    track: hex(0x0e1626),
    arc: hex(0x7cc0ff),
    k: 1.0,
    win: false,
};

/// `CSH`: the shapes on a 32 px grid.
pub const ARROW: &str = "M6.5 3.5v20.2l4.9-4.6 3.3 7.5 3.4-1.5-3.3-7.4h6.8z";
pub const HAND: &str = "M12.2 4.4c0-1.1.8-1.9 1.8-1.9s1.8.8 1.8 1.9v8.1c.3-.8 1-1.3 1.9-1.3 1 0 1.8.8 1.8 1.8v.5c.3-.6 1-1 1.7-1 1 0 1.8.8 1.8 1.8v6.6c0 4.1-3 7.1-6.8 7.1h-1.1c-2.2 0-4.1-1-5.3-2.8l-3.6-5.6c-.6-.9-.4-2.1.5-2.6.8-.5 1.9-.3 2.5.4l2.8 3.1z";
pub const IBEAM: &str = "M12.4 4.6c1.6 0 2.8.5 3.6 1.5.8-1 2-1.5 3.6-1.5M12.4 27.4c1.6 0 2.8-.5 3.6-1.5.8 1 2 1.5 3.6 1.5M16 6.1v19.8M13.6 16h4.8";
pub const MOVE: &str = "M16 3l5 5h-3.5v6.5H24V11l5 5-5 5v-3.5h-6.5V24H21l-5 5-5-5h3.5v-6.5H8V21l-5-5 5-5v3.5h6.5V8H11z";
pub const DBL: &str = "M3 16l6-5.5v3.9h14v-3.9l6 5.5-6 5.5v-3.9H9v3.9z";
pub const FINGERS: &str = "M15.8 12.6v3.3M19.5 13.6v2.7";

/// The roles (the drawing's CROLES order) and their hotspots (`CHOT`, for the size k).
pub const HOT: [(f32, f32); 7] = [(6.5, 3.5), (14.0, 2.5), (16.0, 16.0), (16.0, 16.0), (7.0, 4.0), (16.0, 16.0), (16.0, 16.0)];

fn affine(g: &Gfx, m: &sk::Matrix) {
    g.cv().concat(m);
}

fn body(g: &Gfx, d: &str, l: &Look) {
    let p = g.path(d);
    // fill="url(#cg)" (objectBoundingBox, top -> bottom) fill-opacity = op
    let b = p.compute_tight_bounds();
    let mut f = Paint::default();
    f.set_anti_alias(true);
    if let Some(s) = gradient_shader::linear(
        ((b.left, b.top), (b.left, b.bottom)),
        sk::gradient_shader::GradientShaderColors::ColorsInSpace(&[l.fill.0.c4(), l.fill.1.c4()], None),
        None,
        sk::TileMode::Clamp,
        None,
        None,
    ) {
        f.set_shader(s);
    }
    f.set_alpha_f(l.op);
    g.cv().draw_path(&p, &f);
    let mut s = Paint::new(l.rim.c4(), None);
    s.set_anti_alias(true);
    s.set_style(PaintStyle::Stroke).set_stroke_width(l.rw).set_stroke_miter(10.0);
    s.set_stroke_join(if l.round { sk::paint::Join::Round } else { sk::paint::Join::Miter });
    g.cv().draw_path(&p, &s);
}

fn stroke(g: &Gfx, d: &str, w: f32, c: Rgba, op: f32) {
    let mut s = Paint::new(c.c4(), None);
    s.set_anti_alias(true);
    s.set_style(PaintStyle::Stroke).set_stroke_width(w).set_stroke_cap(sk::paint::Cap::Round).set_stroke_join(sk::paint::Join::Round);
    s.set_alpha_f(s.alpha_f() * op);
    g.cv().draw_path(&g.path(d), &s);
}

fn circle(g: &Gfx, cx: f32, cy: f32, r: f32, w: f32, c: Rgba) {
    let mut s = Paint::new(c.c4(), None);
    s.set_anti_alias(true);
    s.set_style(PaintStyle::Stroke).set_stroke_width(w);
    g.cv().draw_circle((cx, cy), r, &s);
}

/// `curSpin`: a ring (+ the rim ring unless Windows default), its track, and the arc (dasharray .3 C, round caps) from 12
/// o'clock turned by `spin` degrees (`.cpv .spin{animation:cspin 1.1s linear infinite}`).
fn spinner(g: &Gfx, l: &Look, cx: f32, cy: f32, r: f32, w: f32, spin: f32) {
    if !l.win {
        circle(g, cx, cy, r, w + 2.0 * l.rw, l.rim);
    }
    circle(g, cx, cy, r, w, l.track);
    let mut s = Paint::new(l.arc.c4(), None);
    s.set_anti_alias(true);
    s.set_style(PaintStyle::Stroke).set_stroke_width(w).set_stroke_cap(sk::paint::Cap::Round);
    let oval = sk::Rect::from_ltrb(cx - r, cy - r, cx + r, cy + r);
    g.cv().draw_arc(oval, -90.0 + spin, 0.3 * 360.0, false, &s);
}

/// One cursor picture into a `size` px box at (x, y) (the drawing's `curSvg`: viewBox 0 0 32 32), scaled `scale` about
/// the box centre (`.cpv svg{transform:scale(var(--ck))}`), the spinners turned `spin` degrees.
#[allow(clippy::too_many_arguments)]
pub fn cursor(g: &Gfx, l: &Look, role: usize, x: f32, y: f32, size: f32, scale: f32, spin: f32) {
    let cv = g.cv();
    cv.save();
    let k = size / 32.0;
    let m = sk::Matrix::translate((x + size / 2.0, y + size / 2.0)) * sk::Matrix::scale((scale, scale)) * sk::Matrix::translate((-size / 2.0, -size / 2.0)) * sk::Matrix::scale((k, k));
    affine(g, &m);
    // <g filter="url(#cf)">: feDropShadow dx dy stdDeviation flood-color flood-opacity
    let (dx, dy, sd, c) = l.sh;
    let shadow = sk::image_filters::drop_shadow((dx, dy), (sd, sd), c.c4().to_color(), None, None, None);
    let mut lp = Paint::default();
    if let Some(f) = shadow {
        lp.set_image_filter(f);
    }
    cv.save_layer(&sk::canvas::SaveLayerRec::default().paint(&lp));
    if l.k != 1.0 {
        let (hx, hy) = HOT[role];
        affine(g, &(sk::Matrix::translate((hx, hy)) * sk::Matrix::scale((l.k, l.k)) * sk::Matrix::translate((-hx, -hy))));
    }
    match role {
        0 => body(g, ARROW, l),
        1 => {
            body(g, HAND, l);
            stroke(g, FINGERS, l.rw * 0.9, l.rim, 0.6);
        }
        2 => {
            stroke(g, IBEAM, 1.7 + 2.0 * l.rw, l.rim, 1.0);
            stroke(g, IBEAM, 1.7, l.solid, 1.0);
        }
        3 => spinner(g, l, 16.0, 16.0, 8.6, 3.6, spin),
        4 => {
            cv.save();
            affine(g, &(sk::Matrix::translate((1.2, 0.9)) * sk::Matrix::scale((0.86, 0.86))));
            body(g, ARROW, l);
            cv.restore();
            spinner(g, l, 23.4, 23.4, 4.6, 2.4, spin);
        }
        5 => body(g, MOVE, l),
        _ => {
            cv.save();
            affine(g, &(sk::Matrix::translate((16.0, 16.0)) * sk::Matrix::rotate_deg(45.0) * sk::Matrix::translate((-16.0, -16.0))));
            body(g, DBL, l);
            cv.restore();
        }
    }
    cv.restore();
    cv.restore();
}

/// `ICON.plus12` in the picker's "Choose your own file…" tile (`.cmi.own .cpv svg{width:13px;height:13px;stroke:var(--ico-on);
/// stroke-width:1.6}`) is an ordinary icon - drawn by the box tree.
/// The battery level (`.bat .lv{fill:var(--green)}`, `width = 10.6 x percent`), over the `bat` icon at (x, y) (21 x 12).
pub fn battery_level(g: &Gfx, x: f32, y: f32, pct: f32) {
    let w = 10.6 * (pct / 100.0).clamp(0.0, 1.0);
    // an <svg> is painted at its pixel-snapped position (as icons.rs does)
    let (x, y, _, _) = g.snap(x, y, 21.0, 12.0);
    if w > 0.0 {
        let p = rect_path(x + 2.4, y + 3.4, w, 5.2, 1.1);
        g.fill_geom(&p, crate::ui::GREEN());
    }
}

/// Blink's path for an SVG `<rect rx>` (as `icons.rs` builds it).
fn rect_path(x: f32, y: f32, w: f32, h: f32, rx: f32) -> sk::Path {
    let r = rx.min(w / 2.0).min(h / 2.0);
    let k = std::f32::consts::FRAC_1_SQRT_2;
    let mut b = sk::PathBuilder::new();
    b.move_to((x + r, y))
        .line_to((x + w - r, y))
        .conic_to((x + w, y), (x + w, y + r), k)
        .line_to((x + w, y + h - r))
        .conic_to((x + w, y + h), (x + w - r, y + h), k)
        .line_to((x + r, y + h))
        .conic_to((x, y + h), (x, y + h - r), k)
        .line_to((x, y + r))
        .conic_to((x, y), (x + r, y), k)
        .close();
    b.detach()
}

/// The double-click test folder `DCF[open]` (20 x 20, `.dct svg{width:20px;height:20px}`).
pub fn folder(g: &Gfx, x: f32, y: f32, open: bool) {
    let cv = g.cv();
    cv.save();
    cv.translate((x, y));
    if !open {
        g.fill_geom(&g.path("M2.5 5.6a1.4 1.4 0 0 1 1.4-1.4h3.7l1.6 1.6h6.9a1.4 1.4 0 0 1 1.4 1.4v1H2.5z"), hex(0xe8a33a));
        g.fill_geom(&g.path("M2.5 7.6h15v7.6a1.4 1.4 0 0 1-1.4 1.4H3.9a1.4 1.4 0 0 1-1.4-1.4z"), hex(0xffc83d));
    } else {
        g.fill_geom(&g.path("M2.5 5.6a1.4 1.4 0 0 1 1.4-1.4h3.7l1.6 1.6h6.9a1.4 1.4 0 0 1 1.4 1.4v1.5H2.5z"), hex(0xe8a33a));
        g.fill_geom(&rect_path(4.5, 6.6, 11.0, 6.0, 0.6), Rgba(1.0, 1.0, 1.0, 0.92));
        g.fill_geom(&g.path("M4.4 9.6h14.3l-2.2 6.1a1.4 1.4 0 0 1-1.3.9H3.9a1.4 1.4 0 0 1-1.4-1.4V11a1.4 1.4 0 0 1 1.9-1.4z"), hex(0xffc83d));
    }
    cv.restore();
}

/// The graph's box (`GW=214,GH=178,GL=30,GR=8,GT=8,GB=19`).
pub const GW: f32 = 214.0;
pub const GH: f32 = 178.0;
const GL: f32 = 30.0;
const GR: f32 = 8.0;
const GT: f32 = 8.0;
const GB: f32 = 19.0;
const AX: f64 = 120.0;

thread_local! {
    /// (ascent, descent) of the graph's 10 px / 9 px label font (Segoe UI Variable Text), read from the font itself
    static AXM: std::cell::RefCell<std::collections::HashMap<u32, (f32, f32)>> = Default::default();
}

fn metrics(size: f32) -> (f32, f32) {
    AXM.with(|m| {
        *m.borrow_mut().entry((size * 100.0) as u32).or_insert_with(|| {
            let fm = sk::FontMgr::default();
            match fm.match_family_style("Segoe UI Variable Text", sk::FontStyle::normal()).or_else(|| fm.match_family_style("Segoe UI", sk::FontStyle::normal())) {
                Some(tf) => {
                    let (_, m) = sk::Font::from_typeface(tf, size).metrics();
                    (-m.ascent, m.descent)
                }
                None => (size * 0.92, size * 0.22),
            }
        })
    })
}

/// An SVG `<text>` at baseline (x, y) with `text-anchor` (0 start, 1 middle, 2 end), via the painter's own shaping.
fn svg_text(g: &Gfx, s: &str, size: f32, x: f32, y: f32, anchor: u8, c: Rgba) {
    let f = Font::new(size, 400).ls(0);
    let (a, d) = metrics(size);
    // g.text puts the baseline at top + half-leading + round(ascent); with lh = round(a) + round(d) the half-leading is 0
    let lh = a.round() + d.round();
    let top = y - a.round();
    let align = match anchor {
        1 => crate::gfx::Align::Center,
        2 => crate::gfx::Align::Right,
        _ => crate::gfx::Align::Left,
    };
    g.text(s, f, x, top, lh, c, align, 1e4);
}

/// Raw Accel's graph (`aDraw`): `pts` = (speed 0..120, sensitivity) samples; the box at (x, y). Grid lines every 20 speed
/// (dotted), a line per sensitivity step (the 1.0 line dashed), the area under the curve (accent .26 -> 0), the curve
/// (accent, 2 px), the axis numbers (10 px --fg3) and the tiny "speed →". `hover` = the readout point (speed).
pub fn graph(g: &Gfx, x: f32, y: f32, pts: &[(f64, f64)], hover: Option<(f64, f64)>) {
    use crate::ui::{ACC, DASH, FG2, FG3, HAIR};
    if pts.is_empty() {
        return;
    }
    let (mut mn, mut mx) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in pts {
        mn = mn.min(p.1);
        mx = mx.max(p.1);
    }
    let span = (mx - mn).max(0.4);
    let lo = (mn - span * 0.12).max(0.0);
    let hi = mx + span * 0.14;
    let px = |v: f64| x + GL + (v / AX) as f32 * (GW - GL - GR);
    let py = |v: f64| y + GT + (1.0 - ((v - lo) / (hi - lo)) as f32) * (GH - GT - GB);
    let bot = y + GH - GB;
    let st = [0.05, 0.1, 0.2, 0.25, 0.5, 1.0, 2.0, 5.0].into_iter().find(|s| (hi - lo) / s <= 4.4).unwrap_or(10.0);
    let r1 = |v: f32| (v * 10.0).round() / 10.0; // the drawing's toFixed(1)
    let cv = g.cv();
    let line = |x1: f32, y1: f32, x2: f32, y2: f32, c: Rgba, dash: Option<[f32; 2]>| {
        let mut p = Paint::new(c.c4(), None);
        p.set_anti_alias(true);
        p.set_style(PaintStyle::Stroke).set_stroke_width(1.0);
        if let Some(d) = dash {
            p.set_path_effect(sk::PathEffect::dash(&d, 0.0));
        }
        cv.draw_line((x1, y1), (x2, y2), &p);
    };
    // .gv{stroke:var(--hair);stroke-dasharray:1 3}
    let mut sx = 20.0;
    while sx < AX {
        let gx = r1(px(sx) - x) + x;
        line(gx, y + GT, gx, bot, HAIR(), Some([1.0, 3.0]));
        sx += 20.0;
    }
    let mut t = (lo / st - 1e-9).ceil() * st;
    let mut labels = Vec::new();
    while t <= hi + 1e-9 {
        let ty = py(t);
        if ty <= bot - 3.0 {
            let gy = r1(ty - y) + y;
            if (t - 1.0).abs() < 1e-9 {
                // .gb{stroke:var(--fg3);stroke-dasharray:2 3}
                line(x + GL, gy, x + GW - GR, gy, FG3(), Some([2.0, 3.0]));
            } else {
                // .gl{stroke:var(--hair)}
                line(x + GL, gy, x + GW - GR, gy, HAIR(), None);
            }
            let v = (t * 100.0).round() / 100.0;
            labels.push((format!("{}", v), x + GL - 5.0, r1(ty + 3.5 - y) + y, 2u8));
        }
        t += st;
    }
    // .gx{stroke:var(--dash)}
    line(x + GL, bot, x + GW - GR, bot, DASH(), None);
    for v in [0.0, 40.0, 80.0, 120.0] {
        let (lx, an) = if v == 0.0 { (r1(px(v) - x) + x, 0) } else if v == AX { (x + GW - GR, 2) } else { (r1(px(v) - x) + x, 1) };
        labels.push((format!("{}", v as i32), lx, y + GH - 5.0, an));
    }
    // the curve: M x y L ... (each point toFixed(1))
    let mut pb = sk::PathBuilder::new();
    let mut ab = sk::PathBuilder::new();
    for (i, p) in pts.iter().enumerate() {
        let (qx, qy) = (r1(px(p.0) - x) + x, r1(py(p.1) - y) + y);
        if i == 0 {
            pb.move_to((qx, qy));
            ab.move_to((qx, qy));
        } else {
            pb.line_to((qx, qy));
            ab.line_to((qx, qy));
        }
    }
    let curve = pb.detach();
    ab.line_to((r1(px(AX) - x) + x, bot)).line_to((r1(px(0.0) - x) + x, bot)).close();
    let area = ab.detach();
    // fill="url(#acfill)": objectBoundingBox top (accent .26) -> bottom (accent 0)
    let b = area.compute_tight_bounds();
    let mut fp = Paint::default();
    fp.set_anti_alias(true);
    if let Some(s) = gradient_shader::linear(
        ((b.left, b.top), (b.left, b.bottom)),
        sk::gradient_shader::GradientShaderColors::ColorsInSpace(&[ACC().a(0.26).c4(), ACC().a(0.0).c4()], None),
        None,
        sk::TileMode::Clamp,
        None,
        None,
    ) {
        fp.set_shader(s);
    }
    cv.draw_path(&area, &fp);
    // .cl{fill:none;stroke:var(--acc);stroke-width:2;stroke-linejoin:round;stroke-linecap:round}
    let mut cp = Paint::new(ACC().c4(), None);
    cp.set_anti_alias(true);
    cp.set_style(PaintStyle::Stroke).set_stroke_width(2.0).set_stroke_join(sk::paint::Join::Round).set_stroke_cap(sk::paint::Cap::Round);
    cv.draw_path(&curve, &cp);
    // .ax{font:10px/1;fill:var(--fg3)}
    for (s, lx, ly, an) in labels {
        svg_text(g, &s, 10.0, lx, ly, an, FG3());
    }
    // .ax.sp{font-size:9px;opacity:.8}
    svg_text(g, "speed \u{2192}", 9.0, x + GW - GR - 3.0, bot - 4.0, 2, FG3().mul_a(0.8));
    if let Some((hx, hy)) = hover {
        // .hl{stroke:var(--fg2);stroke-dasharray:2 2} .hdot{fill:var(--acc);stroke:#fff;stroke-width:1.5} r 3.5
        let lx = r1(px(hx) - x) + x;
        line(lx, y + GT, lx, bot, FG2(), Some([2.0, 2.0]));
        let cy = r1(py(hy) - y) + y;
        g.fill_circle(lx, cy, 3.5, ACC());
        let mut s = Paint::new(crate::ui::WHITE.c4(), None);
        s.set_anti_alias(true);
        s.set_style(PaintStyle::Stroke).set_stroke_width(1.5);
        cv.draw_circle((lx, cy), 3.5, &s);
    }
}
