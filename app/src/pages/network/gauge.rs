//! The speed test's glass gauge (menu-v22 `.gau`): a 270° arc on the usual speed-test scale 0 · 5 · 10 · 50 · 100 · 250 · 500 ·
//! 1000 Mb/s, painted from the drawing's SVG numbers (`GTK`, `GCX = 113`, `GCY = 100`, `GRR = 86`, viewBox 226 x 184).

use skia_safe as sk;

use crate::gfx::{Font, Gfx, Rgba};
use crate::ui::{FG2, FG3, TRK, VZ1, VZ2};

/// `const GTK=[0,5,10,50,100,250,500,1000]`
pub const GTK: [f64; 8] = [0.0, 5.0, 10.0, 50.0, 100.0, 250.0, 500.0, 1000.0];
const GCX: f32 = 113.0;
const GCY: f32 = 100.0;
const GRR: f32 = 86.0;

/// `gFrac(v)`: where a value sits on the arc (each tick step is 1/7 of it, linear inside a step).
pub fn frac(v: f64) -> f64 {
    if v <= 0.0 {
        return 0.0;
    }
    if v >= 1000.0 {
        return 1.0;
    }
    for i in 0..GTK.len() - 1 {
        if v < GTK[i + 1] {
            return (i as f64 + (v - GTK[i]) / (GTK[i + 1] - GTK[i])) / 7.0;
        }
    }
    1.0
}

/// `gPt(f, r)`: the point at share `f` of the arc, radius `r` (angle 135° + 270° f, y down).
pub fn pt(f: f32, r: f32) -> (f32, f32) {
    let a = (135.0 + 270.0 * f).to_radians();
    (GCX + a.cos() * r, GCY + a.sin() * r)
}

/// What the gauge shows this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// the value on the arc (Mb/s)
    pub value: f64,
    /// upload colours (`url(#nwGu)`) instead of download
    pub up: bool,
    /// the ping phase: track + ticks faded out (`.gau.png .gtk,.gau.png .gtrk{opacity:0}`, .25 s) - 0..1
    pub png: f32,
}

/// The arc as a path from share 0 to share `f` (Blink flattens the SVG arc the same way: one circle segment).
fn arc(f: f32) -> sk::Path {
    let mut b = sk::PathBuilder::new();
    let oval = sk::Rect::from_xywh(GCX - GRR, GCY - GRR, GRR * 2.0, GRR * 2.0);
    b.add_arc(oval, 135.0, 270.0 * f);
    b.detach()
}

thread_local! {
    static CACHE: std::cell::RefCell<Vec<(u64, sk::Image)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Drops the cached pictures (the page closed: closed = small RAM). Called on the UI thread, like `cached`.
pub fn clear() {
    CACHE.with(|c| *c.borrow_mut() = Vec::new());
}

/// How many pictures are cached on this thread (tests).
#[cfg(test)]
pub fn cached_count() -> usize {
    CACHE.with(|c| c.borrow().len())
}

/// Own canvas drawing (shaders the painter has no call for) as a picture at the device scale, cached by `key`, drawn
/// through the painter (so it obeys the painter's static / live passes like every other box). Keeps the last 4.
pub fn cached(g: &Gfx, key: u64, x: f32, y: f32, w: f32, h: f32, draw: impl FnOnce(&sk::Canvas)) {
    let s = g.scale;
    // (the theme is part of the key: its colours are in the picture - Order 033)
    let k = (key ^ (s.to_bits() as u64).rotate_left(32)) ^ (crate::ui::is_light() as u64).rotate_left(63);
    let img = CACHE.with(|c| c.borrow().iter().find(|e| e.0 == k).map(|e| e.1.clone()));
    let img = match img {
        Some(i) => i,
        None => {
            let Some(mut surf) = crate::gfx::new_surface((w * s).ceil() as i32, (h * s).ceil() as i32) else { return };
            let cv = surf.canvas();
            cv.clear(sk::Color::TRANSPARENT);
            cv.scale((s, s));
            draw(cv);
            let i = surf.image_snapshot();
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                c.push((k, i.clone()));
                if c.len() > 4 {
                    c.remove(0);
                }
            });
            i
        }
    };
    g.draw_image_rect(&img, x, y, (w * s).ceil() / s, (h * s).ceil() / s);
}

/// Paints the gauge's SVG into its 226 x 184 box at (x, y).
pub fn paint(g: &Gfx, x: f32, y: f32, l: Look) {
    let key = (l.value.to_bits() ^ ((l.up as u64) << 63)).wrapping_mul(31) ^ (l.png.to_bits() as u64) ^ 0x6761_7567;
    cached(g, key, x, y, 226.0, 184.0, |cv| arcs(cv, l));
    ticks(g, x, y, l);
}

fn arcs(cv: &sk::Canvas, l: Look) {
    let mut p = sk::Paint::default();
    p.set_anti_alias(true);
    p.set_style(sk::PaintStyle::Stroke);
    p.set_stroke_width(10.0);
    p.set_stroke_cap(sk::paint::Cap::Round);
    // .gau .gtrk{stroke:var(--trk);stroke-width:10;opacity:.7}
    let trk_op = 0.7 * (1.0 - l.png);
    if trk_op > 0.001 {
        p.set_color4f(TRK().mul_a(trk_op).c4(), None);
        cv.draw_path(&arc(1.0), &p);
    }
    // the fill: stroke-dasharray f*100 of pathLength 100, gradient x1=0 y1=1 x2=1 y2=0 over the arc's bounding box
    let f = frac(l.value) as f32;
    if f >= 0.004 {
        let (c1, c2) = if l.up { (VZ2(), Rgba::hex(0x8ff0e2)) } else { (VZ1(), Rgba::hex(0x64c8ff)) };
        let full = arc(1.0);
        let bb = full.bounds();
        let m = sk::Matrix::new_all(bb.width(), 0.0, bb.left, 0.0, bb.height(), bb.top, 0.0, 0.0, 1.0);
        let colors = [c1.c4(), c2.c4()];
        let sh = sk::Shader::linear_gradient(
            ((0.0, 1.0), (1.0, 0.0)),
            sk::gradient_shader::GradientShaderColors::ColorsInSpace(&colors, None),
            None,
            sk::TileMode::Clamp,
            None,
            Some(&m),
        );
        let mut fp = p.clone();
        fp.set_color4f(sk::Color4f::new(1.0, 1.0, 1.0, 1.0), None);
        fp.set_shader(sh);
        cv.draw_path(&arc(f), &fp);
        // .ghd{r:4;fill:#fff;stroke:rgba(0,0,0,.12);stroke-width:.5}
        let (hx, hy) = pt(f, GRR);
        let mut hp = sk::Paint::default();
        hp.set_anti_alias(true);
        hp.set_color4f(sk::Color4f::new(1.0, 1.0, 1.0, 1.0), None);
        cv.draw_circle((hx, hy), 4.0, &hp);
        hp.set_style(sk::PaintStyle::Stroke);
        hp.set_stroke_width(0.5);
        hp.set_color4f(Rgba(0.0, 0.0, 0.0, 0.12).c4(), None);
        cv.draw_circle((hx, hy), 4.0, &hp);
    }
}

fn ticks(g: &Gfx, x: f32, y: f32, l: Look) {
    // ticks: .gtk{fill:var(--fg3);font:600 9.5px/1;tabular-nums;text-anchor:middle;dominant-baseline:central} .lit{fill:var(--fg2)}
    let tick_op = 1.0 - l.png;
    if tick_op > 0.001 {
        let font = Font::new(9.5, 600).tnum();
        for (i, t) in GTK.iter().enumerate() {
            let (tx, ty) = pt(i as f32 / 7.0, GRR - 21.0);
            let s = format!("{}", *t as u32);
            let lit = i > 0 && l.value >= *t;
            let c = if lit { FG2() } else { FG3() }.mul_a(tick_op);
            let w = g.text_width(&s, font);
            // `font: ... /1` = line-height 1: a 9.5 px line box, its middle on the point (central baseline)
            g.text(&s, font, x + tx - w / 2.0, y + ty - 9.5 / 2.0, 9.5, c, crate::gfx::Align::Left, 1000.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scale_is_the_drawings() {
        assert_eq!(frac(0.0), 0.0);
        assert!((frac(5.0) - 1.0 / 7.0).abs() < 1e-9);
        assert!((frac(75.0) - 3.5 / 7.0).abs() < 1e-9);
        assert!((frac(920.0) - (6.0 + 0.84) / 7.0).abs() < 1e-9);
        assert_eq!(frac(2000.0), 1.0);
        let (x, y) = pt(0.0, 86.0);
        assert!((x - (113.0 - 60.811)).abs() < 0.01 && (y - (100.0 + 60.811)).abs() < 0.01);
    }
}
