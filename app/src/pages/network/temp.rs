//! The per-path-coloured inline SVG `svg` (the speed test's small arrows, the Start disc's play mark, Storage's folder icons).
//! The page-local stand-ins of Order 022 are gone: the pages use the shared pieces in ui/pieces.

use windows_numerics::Matrix3x2;

use crate::gfx::{Gfx, Rgba};
use crate::ui::el::El;

/// One shape of an inline SVG: path data, fill, stroke (width, colour).
#[derive(Clone)]
pub struct Shape {
    pub d: &'static str,
    pub fill: Option<Rgba>,
    pub stroke: Option<(f32, Rgba)>,
}

/// An inline SVG (viewBox 0 0 vbw vbh) drawn into a w x h box with each shape's own fill / stroke (round caps and joins),
/// through the painter. Kept for the speed test's arrows and the Start disc's white play mark (not part of this swap).
pub fn svg(shapes: Vec<Shape>, vbw: f32, vbh: f32, w: f32, h: f32) -> El {
    El::paint(move |g: &Gfx, (x, y, bw, bh)| {
        // an <svg> is a replaced element: Blink paints it at its pixel-snapped origin; inside, SVG's default
        // preserveAspectRatio (xMidYMid meet): one scale, centred
        let (x, y, _, _) = g.snap(x, y, bw, bh);
        let k = (bw / vbw).min(bh / vbh);
        let (ox, oy) = (x + (bw - vbw * k) / 2.0, y + (bh - vbh * k) / 2.0);
        let t0 = g.transform();
        g.set_transform(&(Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: ox, M32: oy } * t0));
        for s in &shapes {
            let p = g.path(s.d);
            if let Some(c) = s.fill {
                g.fill_geom(&p, c);
            }
            if let Some((sw, c)) = s.stroke {
                g.stroke_geom(&p, sw, c);
            }
        }
        g.set_transform(&t0);
    })
    .size(w, h)
    .none()
    .no_hit()
}
