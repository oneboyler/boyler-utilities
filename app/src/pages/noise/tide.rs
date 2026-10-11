//! Order 097 (pack-noise-v1, option 1 "Soft tide"): three faint wave lines rolling slowly along the bottom of the Noise tab
//! while noise plays, like breathing. Drawn only while noise plays AND the tab is on screen (about 20 frames a second, see
//! `Noise::tick`); the page holds no timer otherwise. `level` 0..1 is how far it has faded in (Stop settles it to 0).

use skia_safe::PathBuilder;

use crate::gfx::{Gfx, Rgba};

/// The picture's size (the drawing's canvas): the full window width, 330 px tall, its bottom on the window's bottom.
pub const W: f32 = 600.0;
pub const H: f32 = 330.0;

struct Layer {
    /// Fill alpha (the line is 1.6 x this).
    a: f32,
    amp: f32,
    /// Waves per px.
    k: f32,
    /// Speed (a quarter turn per 1/s seconds).
    s: f32,
    off: f32,
    c: (f32, f32, f32),
}

const LAYERS: [Layer; 3] = [
    Layer { a: 0.10, amp: 9.0, k: 0.011, s: 0.23, off: 0.0, c: (120.0, 150.0, 255.0) },
    Layer { a: 0.075, amp: 12.0, k: 0.008, s: -0.16, off: 16.0, c: (150.0, 130.0, 255.0) },
    Layer { a: 0.06, amp: 7.0, k: 0.015, s: 0.31, off: 30.0, c: (110.0, 170.0, 240.0) },
];

fn y_at(l: &Layer, x: f32, t: f64, level: f32) -> f32 {
    let base = H - 70.0;
    let breath = 1.0 + 0.25 * (t * 0.35 + f64::from(l.off)).sin() as f32;
    let a = f64::from(x * l.k) + t * f64::from(l.s) * std::f64::consts::TAU / 4.0;
    let b = f64::from(x * 0.004) - t * 0.2;
    base + l.off - l.amp * breath * level * a.sin() as f32 - 4.0 * level * b.sin() as f32
}

/// The tide at time `t` (seconds), `level` faded in, in the box `(x, y, w, h)` (its bottom edge = the picture's bottom).
pub fn paint(g: &Gfx, (x0, y0, _w, h): (f32, f32, f32, f32), t: f64, level: f32) {
    if level <= 0.0 {
        return;
    }
    let oy = y0 + h - H;
    for l in &LAYERS {
        let col = |a: f32| Rgba(l.c.0 / 255.0, l.c.1 / 255.0, l.c.2 / 255.0, a);
        let pts: Vec<(f32, f32)> = (0..=100).map(|i| (x0 + i as f32 * 6.0, oy + y_at(l, i as f32 * 6.0, t, level))).collect();
        let mut lb = PathBuilder::new();
        let mut fb = PathBuilder::new();
        for (i, p) in pts.iter().enumerate() {
            if i == 0 {
                lb.move_to(*p);
                fb.move_to(*p);
            } else {
                lb.line_to(*p);
                fb.line_to(*p);
            }
        }
        fb.line_to((x0 + W, oy + H)).line_to((x0, oy + H)).close();
        let (line, fill) = (lb.detach(), fb.detach());
        let top = oy + H - 70.0 - 20.0;
        let shader = g.hgrad(0.0, top, 0.0, oy + H, &[(0.0, col(l.a * level)), (1.0, col(0.0))]);
        g.fill_path_shader(&fill, &shader, 1.0);
        g.stroke_geom(&line, 1.0, col(l.a * 1.6 * level));
    }
}
