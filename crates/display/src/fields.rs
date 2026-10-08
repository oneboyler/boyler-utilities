//! The W × H × Hz field rules of DESIGN §3.2.2 as plain functions (the menu only shows the results):
//! clamping, Hz snapping, NVIDIA-style rate labels, Arrow/wheel stepping, and helpers over a monitor's mode list.

use crate::types::{GpuScaling, Mode, RefreshRate, VideoMode};
use std::collections::BTreeSet;

pub const MIN_WIDTH: u32 = 640;
pub const MAX_WIDTH: u32 = 7680;
pub const MIN_HEIGHT: u32 = 480;
pub const MAX_HEIGHT: u32 = 4320;

/// Arrow Up/Down and wheel stops for Width (DESIGN §3.2.2).
pub const COMMON_WIDTHS: [u32; 10] = [1024, 1152, 1280, 1440, 1600, 1680, 1920, 2560, 3440, 3840];
/// Arrow Up/Down and wheel stops for Height (DESIGN §3.2.2).
pub const COMMON_HEIGHTS: [u32; 11] = [720, 768, 864, 900, 960, 1024, 1050, 1080, 1200, 1440, 2160];

/// Width as typed (digits only, up to 4) → clamped to 640–7680.
pub fn clamp_width(w: u32) -> u32 {
    w.clamp(MIN_WIDTH, MAX_WIDTH)
}

/// Height as typed (digits only, up to 4) → clamped to 480–4320.
pub fn clamp_height(h: u32) -> u32 {
    h.clamp(MIN_HEIGHT, MAX_HEIGHT)
}

/// Parses what was typed into W or H: digits only, at most 4 (anything else is dropped), then clamped.
/// `None` when nothing usable was typed (the field restores its value).
pub fn parse_size_field(text: &str, is_width: bool) -> Option<u32> {
    let digits: String = text.chars().filter(|c| c.is_ascii_digit()).take(4).collect();
    let v: u32 = digits.parse().ok()?;
    Some(if is_width { clamp_width(v) } else { clamp_height(v) })
}

/// Parses what was typed into Hz: digits and ".", at most 6 characters.
pub fn parse_hz_field(text: &str) -> Option<f64> {
    let kept: String = text.chars().filter(|c| c.is_ascii_digit() || *c == '.').take(6).collect();
    let v: f64 = kept.parse().ok()?;
    if v.is_finite() { Some(v) } else { None }
}

/// The distinct rates in a mode list, slowest first.
pub fn all_rates(modes: &[VideoMode]) -> Vec<RefreshRate> {
    let set: BTreeSet<RefreshRate> = modes.iter().map(|m| m.refresh).collect();
    set.into_iter().collect()
}

/// The rates the monitor reports at W × H, slowest first. If that size is not in the list, every rate of the monitor.
pub fn rates_for(modes: &[VideoMode], width: u32, height: u32) -> Vec<RefreshRate> {
    let set: BTreeSet<RefreshRate> =
        modes.iter().filter(|m| m.width == width && m.height == height).map(|m| m.refresh).collect();
    if set.is_empty() { all_rates(modes) } else { set.into_iter().collect() }
}

/// Hz snapping: the nearest rate the monitor reports; anything at or above its fastest rate becomes the fastest
/// (e.g. 240 typed on a 144 Hz monitor → 143.98). Never a rate the monitor doesn't report. A tie goes to the faster rate.
pub fn snap_hz(typed: f64, rates: &[RefreshRate]) -> Option<RefreshRate> {
    let fastest = *rates.iter().max()?;
    if typed >= fastest.hz() {
        return Some(fastest);
    }
    let mut best: Option<(f64, RefreshRate)> = None;
    for r in rates {
        let d = (r.hz() - typed).abs();
        match best {
            None => best = Some((d, *r)),
            Some((bd, br)) => {
                if d < bd - 1e-9 || ((d - bd).abs() <= 1e-9 && *r > br) {
                    best = Some((d, *r));
                }
            }
        }
    }
    best.map(|(_, r)| r)
}

/// The label for one rate inside its monitor's list (DESIGN "Rate labels"): rounded to whole numbers; only when two
/// rates round to the same number does the odd one (the one further from that whole number) keep 2 decimals.
/// So [165, 144, 120, 119.88, 100, 60, 59.94] → "165","144","120","119.88","100","60","59.94" and a lone 239.76 → "240".
pub fn rate_label(rate: RefreshRate, list: &[RefreshRate]) -> String {
    let rounded = rate.hz().round();
    let my_dist = (rate.hz() - rounded).abs();
    let clash = list.iter().filter(|o| **o != rate && o.hz().round() == rounded);
    // Keep decimals if a sibling with the same whole number is closer to it (ties: the faster rate gets the whole number).
    let mut odd = false;
    for o in clash {
        let od = (o.hz() - rounded).abs();
        if od < my_dist - 1e-9 || ((od - my_dist).abs() <= 1e-9 && *o > rate) {
            odd = true;
        }
    }
    if !odd {
        return format!("{}", rounded as i64);
    }
    // 2 decimals; 3 when 2 would still read the same as a sibling or the whole number (measured on a real 360 Hz monitor:
    // 360, 359.999 and 359.998 in one list).
    let two = format!("{:.2}", rate.hz());
    let whole = format!("{}.00", rounded as i64);
    let clash2 = two == whole || list.iter().any(|o| *o != rate && format!("{:.2}", o.hz()) == two);
    if clash2 { format!("{:.3}", rate.hz()) } else { two }
}

/// The exact rate shown small after the Hz label on hover / focus (e.g. "164.95"). Whole rates show without decimals.
pub fn exact_label(rate: RefreshRate) -> String {
    let s = format!("{:.2}", rate.hz());
    let t = s.trim_end_matches('0').trim_end_matches('.');
    t.to_string()
}

/// The Hz popup: this monitor's rates, fastest first, labelled, with a ✓ flag on the current one.
pub fn rate_menu(rates: &[RefreshRate], current: RefreshRate) -> Vec<(RefreshRate, String, bool)> {
    let mut v: Vec<RefreshRate> = rates.to_vec();
    v.sort();
    v.dedup();
    v.reverse();
    v.iter().map(|r| (*r, format!("{} Hz", rate_label(*r, rates)), *r == current)).collect()
}

/// Arrow Up (`up = true`) / Down and wheel stepping through a list of stops: to the next stop above / below the value.
/// Stepping stops at the ends (returns the same value).
pub fn step_through(value: u32, stops: &[u32], up: bool) -> u32 {
    if up {
        stops.iter().copied().filter(|s| *s > value).min().unwrap_or(value)
    } else {
        stops.iter().copied().filter(|s| *s < value).max().unwrap_or(value)
    }
}

/// Stepping Hz through the monitor's own rates; stops at the ends.
pub fn step_hz(current: RefreshRate, rates: &[RefreshRate], up: bool) -> RefreshRate {
    if up {
        rates.iter().copied().filter(|r| *r > current).min().unwrap_or(current)
    } else {
        rates.iter().copied().filter(|r| *r < current).max().unwrap_or(current)
    }
}

/// The closest size the monitor reports to W × H: same shape (aspect within 1 %) first, then by pixel distance,
/// ties to the larger size. E.g. 1600 × 900 on a monitor without it → 1920 × 1080, not 1280 × 960.
pub fn nearest_size(modes: &[VideoMode], width: u32, height: u32) -> Option<(u32, u32)> {
    let sizes: BTreeSet<(u32, u32)> = modes.iter().map(|m| (m.width, m.height)).collect();
    let want = width as f64 / height.max(1) as f64;
    sizes.into_iter().min_by_key(|(w, h)| {
        let other_shape = ((*w as f64 / (*h).max(1) as f64) / want - 1.0).abs() > 0.01;
        let dw = *w as i64 - width as i64;
        let dh = *h as i64 - height as i64;
        (other_shape, dw * dw + dh * dh, -((*w as i64) * (*h as i64)))
    })
}

/// True when the monitor reports W × H at some rate.
pub fn size_supported(modes: &[VideoMode], width: u32, height: u32) -> bool {
    modes.iter().any(|m| m.width == width && m.height == height)
}

/// Builds the exact mode Apply sets from the fields: W/H clamped, Hz snapped to a rate the monitor reports at that size.
/// A W × H the monitor doesn't report is refused with the nearest one it does (DESIGN left this unclear; decided here,
/// see the report: refuse + suggest, never guess for the user).
pub fn resolve_fields(
    modes: &[VideoMode],
    width: u32,
    height: u32,
    hz_typed: f64,
    scaling: GpuScaling,
) -> crate::Result<Mode> {
    if modes.is_empty() {
        return Err(crate::DisplayError::NoModes);
    }
    let (w, h) = (clamp_width(width), clamp_height(height));
    if !size_supported(modes, w, h) {
        let nearest = nearest_size(modes, w, h).and_then(|(nw, nh)| {
            let rates = rates_for(modes, nw, nh);
            snap_hz(hz_typed, &rates).map(|r| VideoMode { width: nw, height: nh, refresh: r })
        });
        return Err(crate::DisplayError::ModeNotSupported { width: w, height: h, nearest });
    }
    let rates = rates_for(modes, w, h);
    let refresh = snap_hz(hz_typed, &rates).ok_or(crate::DisplayError::NoModes)?;
    Ok(Mode { width: w, height: h, refresh, scaling })
}

/// "1920 × 1080 · 165 Hz" — the tooltip / toast text of a mode (rate labelled inside the monitor's rate list).
pub fn mode_text(mode: &Mode, rates: &[RefreshRate]) -> String {
    format!("{} × {} · {} Hz", mode.width, mode.height, rate_label(mode.refresh, rates))
}
