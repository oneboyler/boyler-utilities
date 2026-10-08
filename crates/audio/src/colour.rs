//! One colour from an app's icon for its slider and level bar (DESIGN: "its fill in the app icon's colour"). The same
//! method as Lane A's test app (bakeoff-native `appinfo.rs`): the most common saturated hue, weighted by how vivid and
//! opaque each pixel is, merged with its two neighbour hues; very dark colours lifted a little so they read on dark
//! glass; the second colour is that one mixed 45 % toward white. A grey icon gets the drawing's grey.

use crate::model::Icon;
use crate::os::GREY;

pub fn icon_colour(icon: &Icon) -> (u32, u32) {
    let mut bins = [(0f64, 0f64, 0f64, 0f64); 36];
    for px in icon.bgra.as_chunks::<4>().0 {
        let a = px[3] as f64 / 255.0;
        if a < 0.5 {
            continue;
        }
        // premultiplied → straight
        let (b, g, r) = (px[0] as f64 / 255.0 / a, px[1] as f64 / 255.0 / a, px[2] as f64 / 255.0 / a);
        let (r, g, b) = (r.min(1.0), g.min(1.0), b.min(1.0));
        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        let s = if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
        if s < 0.25 || mx < 0.25 {
            continue;
        }
        let h = if mx == r {
            ((g - b) / (mx - mn)).rem_euclid(6.0)
        } else if mx == g {
            (b - r) / (mx - mn) + 2.0
        } else {
            (r - g) / (mx - mn) + 4.0
        } * 60.0;
        let w = s * a * mx;
        let i = ((h / 10.0) as usize).min(35);
        bins[i].0 += r * w;
        bins[i].1 += g * w;
        bins[i].2 += b * w;
        bins[i].3 += w;
    }
    let score = |i: usize| bins[i].3 + 0.5 * (bins[(i + 35) % 36].3 + bins[(i + 1) % 36].3);
    let best = (0..36).max_by(|a, b| score(*a).total_cmp(&score(*b))).unwrap_or(0);
    let mut acc = (0.0, 0.0, 0.0, 0.0);
    for j in [(best + 35) % 36, best, (best + 1) % 36] {
        acc.0 += bins[j].0;
        acc.1 += bins[j].1;
        acc.2 += bins[j].2;
        acc.3 += bins[j].3;
    }
    if acc.3 < 1.0 {
        return GREY;
    }
    let mut c = (acc.0 / acc.3, acc.1 / acc.3, acc.2 / acc.3);
    let l = 0.2126 * c.0 + 0.7152 * c.1 + 0.0722 * c.2;
    if l < 0.28 {
        c = mix(c, (0.28 - l) / 0.72);
    }
    (rgb(c), rgb(mix(c, 0.45)))
}

fn mix(c: (f64, f64, f64), t: f64) -> (f64, f64, f64) {
    (c.0 + (1.0 - c.0) * t, c.1 + (1.0 - c.1) * t, c.2 + (1.0 - c.2) * t)
}

fn rgb(c: (f64, f64, f64)) -> u32 {
    let q = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    (q(c.0) << 16) | (q(c.1) << 8) | q(c.2)
}
