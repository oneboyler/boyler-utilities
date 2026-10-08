//! The popups' maths (popup.c / place.c), pure so it is tested without a window: the slide / fade animation, the corner a
//! position setting means, the custom spot, the slide direction, the stack, and the placement overlay's snapping.

use crate::monitors::{Mon, Rect};
use crate::settings::{Settings, AN_NONE, AN_SLIDE, P_BR, P_CUSTOM, P_NEAR, P_TR};

pub const HOLD_MS: i32 = 2500;
pub const IN_MS: i32 = 220;
pub const OUT_MS: i32 = 400;

/// One animation frame: (dx, dy, alpha 0..255, Timer bar fill 0..1000), None = over. dir: 0 from left, 1 from right, 2 from
/// top, 3 from bottom; `slide` = how far it slides (device px).
pub fn anim_frame(anim: i32, dir: i32, slide: i32, t: i32) -> Option<(i32, i32, i32, i32)> {
    let inn = if anim == AN_NONE { 0 } else { IN_MS };
    let out = if anim == AN_NONE { 0 } else { OUT_MS };
    let (mut alpha, mut barq, mut off) = (255, 1000, 0);
    if t < inn {
        // ease-out cubic: fast start, gentle stop
        let p = t * 1000 / inn;
        let q = (1000 - p) as i64;
        let e = 1000 - (q * q * q / 1_000_000) as i32;
        alpha = 255 * p / 1000;
        if anim == AN_SLIDE {
            off = slide * (1000 - e) / 1000;
        }
    } else if t < inn + HOLD_MS {
        barq = 1000 - (t - inn) * 1000 / HOLD_MS;
    } else if t < inn + HOLD_MS + out {
        barq = 0;
        alpha = 255 - 255 * (t - inn - HOLD_MS) / out;
    } else {
        return None;
    }
    let (dx, dy) = match dir {
        0 => (-off, 0),
        1 => (off, 0),
        2 => (0, -off),
        _ => (0, off),
    };
    Some((dx, dy, alpha, barq))
}

/// The position setting as a corner / edge (P_TL..P_BR) or P_CUSTOM ("Nearest to game" resolved).
pub fn pick_corner(set: &Settings, mons: &[Mon], mon: Option<usize>, clipped: Option<usize>) -> i32 {
    if set.pos == P_CUSTOM {
        return if set.cx >= 0 { P_CUSTOM } else { P_BR };
    }
    if set.pos != P_NEAR {
        return set.pos;
    }
    match (clipped, mon) {
        (Some(c), Some(m)) if c != m && c < mons.len() && m < mons.len() => {
            if mons[c].rc.left + mons[c].rc.right < mons[m].rc.left + mons[m].rc.right {
                3 // P_BL: the game is to the left
            } else {
                P_BR
            }
        }
        _ => P_BR,
    }
}

/// Custom position (the centre as fractions x10000 of the work area) -> top-left, kept on screen.
pub fn custom_xy(wk: &Rect, w: i32, h: i32, fx: i32, fy: i32) -> (i32, i32) {
    let (ww, hh) = (wk.w() as i64, wk.h() as i64);
    let mut x = wk.left + ((fx as i64 * ww + 5000) / 10000) as i32 - w / 2;
    let mut y = wk.top + ((fy as i64 * hh + 5000) / 10000) as i32 - h / 2;
    if x > wk.right - w {
        x = wk.right - w;
    }
    if y > wk.bottom - h {
        y = wk.bottom - h;
    }
    if x < wk.left {
        x = wk.left;
    }
    if y < wk.top {
        y = wk.top;
    }
    (x, y)
}

/// The popup's centre as fractions x10000 of the work area (rounded both ways, so saving and loading give back the same
/// pixel).
pub fn to_fraction(wk: &Rect, x: i32, y: i32, w: i32, h: i32) -> (i32, i32) {
    let (ww, hh) = (wk.w() as i64, wk.h() as i64);
    let fx = if ww != 0 { (((x + w / 2 - wk.left) as i64 * 10000 + ww / 2) / ww) as i32 } else { 5000 };
    let fy = if hh != 0 { (((y + h / 2 - wk.top) as i64 * 10000 + hh / 2) / hh) as i32 } else { 5000 };
    (fx, fy)
}

/// Slide in from the nearest screen edge.
pub fn slide_dir(corner: i32, wk: &Rect, x: i32, y: i32, w: i32, h: i32) -> i32 {
    if corner == P_CUSTOM {
        let (cx, cy) = (x + w / 2, y + h / 2);
        let (dl, dr, dt, db) = (cx - wk.left, wk.right - cx, cy - wk.top, wk.bottom - cy);
        let (mut m, mut d) = (dl, 0);
        if dr < m {
            m = dr;
            d = 1;
        }
        if dt < m {
            m = dt;
            d = 2;
        }
        if db < m {
            d = 3;
        }
        return d;
    }
    match corner % 3 {
        0 => 0,
        2 => 1,
        _ if corner <= P_TR => 2,
        _ => 3,
    }
}

/// Where each popup of a stack sits (newest last in `sizes`): returns the top-left of each. `mg` = 12 px at the popup's
/// DPI, `gap` = 8 px; at a custom spot the newest is there and older ones move away from the nearer screen edge.
pub fn stack(wk: &Rect, corner: i32, sizes: &[(i32, i32)], mg: i32, gap: i32, custom: (i32, i32)) -> Vec<(i32, i32)> {
    let n = sizes.len();
    let mut out = vec![(0, 0); n];
    if n == 0 {
        return out;
    }
    if corner == P_CUSTOM {
        let (nw, nh) = sizes[n - 1];
        let (bx, by) = custom_xy(wk, nw, nh, custom.0, custom.1);
        out[n - 1] = (bx, by);
        let cy = by + nh / 2;
        let top = cy < (wk.top + wk.bottom) / 2;
        let mut y = if top { by + nh + mg * 2 / 3 } else { by - mg * 2 / 3 };
        for i in (0..n - 1).rev() {
            let (w, h) = sizes[i];
            let x = bx + (nw - w) / 2;
            if top {
                out[i] = (x, y);
                y += h + mg * 2 / 3;
            } else {
                out[i] = (x, y - h);
                y -= h + mg * 2 / 3;
            }
        }
        return out;
    }
    let top = corner <= P_TR;
    let col = corner % 3;
    let mut y = if top { wk.top + mg } else { wk.bottom - mg };
    for i in (0..n).rev() {
        let (w, h) = sizes[i];
        if !top {
            y -= h;
        }
        let x = match col {
            0 => wk.left + mg,
            1 => (wk.left + wk.right - w) / 2,
            _ => wk.right - mg - w,
        };
        out[i] = (x, y);
        if top {
            y += h + gap;
        } else {
            y -= gap;
        }
    }
    out
}

/// Shift-snapping in the placement overlay: to the work area's edges (16 px in) and both centres when within 40 px.
/// Returns (x, y, guide x, guide y).
pub fn snap(wk: &Rect, w: i32, h: i32, dpi: i32, x: i32, y: i32) -> (i32, i32, Option<i32>, Option<i32>) {
    let m = 16 * dpi / 96;
    let thr = 40 * dpi / 96;
    let tx = [wk.left + m, (wk.left + wk.right - w) / 2, wk.right - m - w];
    let lx = [wk.left + m, (wk.left + wk.right) / 2, wk.right - m];
    let ty = [wk.top + m, (wk.top + wk.bottom - h) / 2, wk.bottom - m - h];
    let ly = [wk.top + m, (wk.top + wk.bottom) / 2, wk.bottom - m];
    let pick = |v: i32, t: &[i32; 3]| {
        let mut best: Option<usize> = None;
        let mut bd = thr + 1;
        for (i, tv) in t.iter().enumerate() {
            let d = (v - tv).abs();
            if d < bd {
                bd = d;
                best = Some(i);
            }
        }
        best
    };
    let (mut nx, mut ny, mut gx, mut gy) = (x, y, None, None);
    if let Some(i) = pick(x, &tx) {
        nx = tx[i];
        gx = Some(lx[i]);
    }
    if let Some(i) = pick(y, &ty) {
        ny = ty[i];
        gy = Some(ly[i]);
    }
    (nx, ny, gx, gy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AN_FADE;

    #[test]
    fn animation_curve_matches_clipping() {
        // ClipPing's animtest values for slide-from-right with a 60 px slide
        assert_eq!(anim_frame(AN_SLIDE, 1, 60, 0), Some((60, 0, 0, 1000)));
        assert_eq!(anim_frame(AN_SLIDE, 1, 60, 110), Some((7, 0, 127, 1000)));
        assert_eq!(anim_frame(AN_SLIDE, 1, 60, 220), Some((0, 0, 255, 1000)));
        assert_eq!(anim_frame(AN_SLIDE, 1, 60, 1000), Some((0, 0, 255, 688)));
        assert_eq!(anim_frame(AN_SLIDE, 1, 60, 2920), Some((0, 0, 128, 0)));
        assert_eq!(anim_frame(AN_SLIDE, 1, 60, 3120), None);
        assert_eq!(anim_frame(AN_FADE, 3, 60, 55), Some((0, 0, 63, 1000)));
        assert_eq!(anim_frame(AN_NONE, 0, 60, 0), Some((0, 0, 255, 1000)));
        assert_eq!(anim_frame(AN_NONE, 0, 60, 2500), None);
    }

    #[test]
    fn stacks_and_custom_spot() {
        let wk = Rect { left: 0, top: 0, right: 1920, bottom: 1040 };
        let s = stack(&wk, P_BR, &[(200, 50), (300, 60)], 12, 8, (0, 0));
        assert_eq!(s, vec![(1920 - 12 - 200, 1040 - 12 - 60 - 8 - 50), (1920 - 12 - 300, 1040 - 12 - 60)]);
        let (fx, fy) = to_fraction(&wk, 100, 200, 300, 60);
        assert_eq!(custom_xy(&wk, 300, 60, fx, fy), (100, 200));
        assert_eq!(slide_dir(P_BR, &wk, 0, 0, 1, 1), 1);
        assert_eq!(snap(&wk, 300, 60, 96, 30, 500), (16, 490, Some(16), Some(520)));
    }
}
