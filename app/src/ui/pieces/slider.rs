//! The slider (a range input as Chromium paints it, menu-v22 `.rng`), its value label (`.sv`) and the slim live level
//! meter that runs under a slider (the Audio app rows' `.lvl`).

use crate::anim::EASE;
use crate::gfx::{sh, Font, Gfx, Rgba, Shadow};
use crate::ui::cx::Cx;
use crate::ui::el::{lh, Cursor, El, Key};
use crate::ui::{ACC, FG2, LVT, TRK, WHITE};

/// The thumb's shadows: `0 1px 3px rgba(0,0,0,.35), 0 0 0 .5px rgba(0,0,0,.12)`.
pub const KNOB_SH: [Shadow; 2] = [sh(0.0, 1.0, 3.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.35)), sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, 0.12))];

/// How a slider looks: track height, fill / track colours, thumb colour.
#[derive(Clone, Copy)]
pub struct Look {
    pub track_h: f32,
    pub fill: Rgba,
    pub track: Rgba,
    pub thumb: Rgba,
}

/// `.rng::-webkit-slider-runnable-track{height:3px;border-radius:2px;background:linear-gradient(90deg,var(--acc) var(--p),var(--trk) var(--p))}`
pub fn default() -> Look {
    Look { track_h: 3.0, fill: ACC(), track: TRK(), thumb: WHITE }
}

/// A slider `w` x `h` (`.rng{width:150px;height:20px}` by default) at `value` 0..1. The thumb (16 px white) grows to 1.12
/// on hover and 1.18 while dragged (`transition: transform .14s ease`). Input: `Ev::Press` / `Ev::Drag` with the box -
/// `value_at(rect, x)` turns the pointer into a value.
pub fn slider(cx: &mut Cx, key: Key, value: f32, w: f32, h: f32, look: Look) -> El {
    let target = if cx.active(key) {
        1.18
    } else if cx.hovered(key) {
        1.12
    } else {
        1.0
    };
    let ks = cx.tr(key, 1, target, 140.0, EASE);
    let v = value.clamp(0.0, 1.0);
    let c = |r: Rgba| [r.0.to_bits(), r.1.to_bits(), r.2.to_bits(), r.3.to_bits()];
    let mut e = El::paint(move |g, (x, y, w, h)| paint_range(g, x, y, w, h, v, look, ks))
        .sig((v.to_bits(), ks.to_bits(), look.track_h.to_bits(), c(look.fill), c(look.track), c(look.thumb)))
        .size(w, h)
        .none()
        .key(key)
        .cursor(Cursor::Hand);
    // Order 045: Tab reaches it, the arrows move it (`.rng` is an <input type=range>)
    e.range = Some((v, 0.01));
    e
}

/// Order 045: a slider of `n` stops (pointer speed 1-20 = 20): an arrow key moves it one stop (`<input type=range step>`).
pub fn stops(mut e: El, n: u32) -> El {
    if let Some((v, _)) = e.range {
        e.range = Some((v, 1.0 / (n.max(2) - 1) as f32));
    }
    e
}

/// The value a pointer at window x means for a slider box (the thumb centre runs from 8 to w - 8), in whole percent.
pub fn value_at(rect: (f32, f32, f32, f32), x: f32) -> f32 {
    ((x - rect.0 - 8.0) / (rect.2 - 16.0) * 100.0).round().clamp(0.0, 100.0) / 100.0
}

/// One range input: the track filled up to the value with a hard colour stop (Blink adds a stop at 0 / 1 only where the
/// list doesn't reach the end), the thumb at 8 + (w - 16) v, centred on the track.
pub fn paint_range(g: &Gfx, x: f32, y: f32, w: f32, h: f32, v: f32, look: Look, ks: f32) {
    let cy = y + h / 2.0;
    let ty = cy - look.track_h / 2.0;
    let p = v.clamp(0.0, 1.0);
    let mut stops = vec![(p, look.fill), (p, look.track)];
    if p > 0.0 {
        stops.insert(0, (0.0, look.fill));
    }
    if p < 1.0 {
        stops.push((1.0, look.track));
    }
    let shd = g.hgrad(x, 0.0, x + w, 0.0, &stops);
    g.fill_rr_shader(x, ty, w, look.track_h, 2.0, &shd, 1.0);
    knob(g, x + 8.0 + (w - 16.0) * p, cy, ks, look.thumb);
}

/// The white thumb (16 px, its shadows), scaled about its centre.
pub fn knob(g: &Gfx, cx: f32, cy: f32, s: f32, col: Rgba) {
    let t0 = g.transform();
    if s != 1.0 {
        g.set_transform(&(windows_numerics::Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: cx * (1.0 - s), M32: cy * (1.0 - s) } * t0));
    }
    g.box_shadows(cx - 8.0, cy - 8.0, 16.0, 16.0, 8.0, &KNOB_SH, true);
    g.fill_circle(cx, cy, 8.0, col);
    g.set_transform(&t0);
}

/// `.sv{min-width:40px;text-align:right;font-size:12px;color:var(--fg2);font-variant-numeric:tabular-nums}`
pub fn value_label(text: &str) -> El {
    El::text(text, Font::new(12.0, 400).tnum(), FG2(), lh(12.0, 1.35)).min_w(40.0).align(crate::gfx::Align::Right)
}

/// The slim live level meter under a slider: a 3 px track (`--lvt`), the level filled with the row's colour gradient at
/// .45 + .55 min(1, 2.4 L), the held peak as a 3 px dot (`pk_op` = its fade). `w` wide; live (painted every frame).
pub fn level_bar(w: f32, level: f32, peak: f32, pk_op: f32, c1: Rgba, c2: Rgba) -> El {
    level_bar_with(w, move || (level, peak), pk_op, c1, c2)
}

/// Order 047: `level_bar` whose level and peak are read when it is painted (`now()` = (level, peak)) - a meter that moves
/// in the live pass without the page being built again.
pub fn level_bar_with(w: f32, now: impl Fn() -> (f32, f32) + 'static, pk_op: f32, c1: Rgba, c2: Rgba) -> El {
    El::paint(move |g, (x, y, w, _)| {
        let (level, peak) = now();
        g.fill_rr(x, y, w, 3.0, 1.5, LVT());
        let lvl = level.clamp(0.0, 1.0);
        if lvl > 0.0 {
            // `.lvl i{inset:0;border-radius:inherit;clip-path:inset(0 <1-L> 0 0 round 2px)}`: the whole-width rounded fill,
            // cut by a rounded clip - the left end is rounded twice (both anti-aliased), the right end by the clip only
            let br = g.hgrad(x, 0.0, x + w, 0.0, &[(0.0, c1), (1.0, c2)]);
            let o = 0.45 + 0.55 * (lvl * 2.4).min(1.0);
            g.push_layer(1.0, Some((x, y, w * lvl, 3.0, 1.5)));
            g.fill_rr_shader(x, y, w, 3.0, 1.5, &br, o);
            g.pop_layer();
        }
        if pk_op > 0.001 {
            g.fill_circle(x + w * peak.clamp(0.0, 1.0) - 1.5, y + 1.5, 1.5, c2.mul_a(pk_op));
        }
    })
    .size(w, 3.0)
    .live()
}
