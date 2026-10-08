//! The OLD "Pane" app icon (concept 1 of mockups\app-icon-v1.html), drawn per pixel size and written as .ico by the dev
//! command `--make-icons <dir>`. STALE since Order 038: the .ico files in app\assets now hold the new "Aqua" glass tile,
//! rendered off-screen from the drawing (mockups\app-icon-v2.html, variant 2), so do NOT run this over them.

use windows::core::*;

use crate::gfx::{new_surface, Gfx, Rgba};
use crate::png;

struct Theme {
    ink: Rgba,
    ac: Rgba,
    tile: Rgba,
    edge: Rgba,
    hi: Rgba,
    top: Rgba,
    bot: Rgba,
}

fn theme(light: bool) -> Theme {
    if light {
        Theme {
            ink: Rgba::hex(0x1b1c20),
            ac: Rgba::hex(0x0067d9),
            tile: Rgba::hex(0xffffff),
            edge: Rgba(0.0, 0.0, 0.0, 0.2),
            hi: Rgba(1.0, 1.0, 1.0, 1.0),
            top: Rgba::hex(0xffffff),
            bot: Rgba::hex(0xeef1f6),
        }
    } else {
        Theme {
            ink: Rgba::hex(0xffffff),
            ac: Rgba::hex(0x3395ff),
            tile: Rgba::hex(0x1d2434),
            edge: Rgba(1.0, 1.0, 1.0, 0.2),
            hi: Rgba(1.0, 1.0, 1.0, 0.3),
            top: Rgba::hex(0x27324a),
            bot: Rgba::hex(0x161c2a),
        }
    }
}

fn jsround(v: f32) -> f32 {
    (v + 0.5).floor()
}

/// Draw the icon at `size` px into the current target (DIPs = px).
pub fn draw(g: &Gfx, size: f32, light: bool) {
    let t = theme(light);
    let k = size / 16.0;
    let w = if size <= 16.0 {
        1.0
    } else if size <= 24.0 {
        1.5
    } else if size <= 32.0 {
        2.0
    } else if size <= 48.0 {
        3.0
    } else {
        size / 21.333
    };
    let l = |v: f32| jsround(v * k - w / 2.0) + w / 2.0;
    let i = |v: f32| jsround(v * k);
    let p = |v: f32| v * k;
    let small = size <= 16.0;
    // the tile
    let (x, y) = (i(1.0), i(1.0));
    let s = i(15.0) - i(1.0);
    let r = (3.6 * k).max(3.0);
    if size >= 48.0 {
        let br = g.hgrad(0.0, y, 0.0, y + s, &[(0.0, t.top), (1.0, t.bot)]);
        g.fill_rr_shader(x, y, s, s, r, &br, 1.0);
    } else {
        g.fill_rr(x, y, s, s, r, t.tile);
    }
    let e = if size <= 48.0 { 1.0 } else { 2.0 };
    g.stroke_rr(x + e / 2.0, y + e / 2.0, s - e, s - e, r - e / 2.0, e, t.edge);
    if size >= 48.0 {
        let hy = y + e + e / 2.0;
        g.line(x + r, hy, x + s - r, hy, e, t.hi.mul_a(0.28), true);
    }
    // two sliders
    let (y1, y2, x0, x1) = (l(6.0), l(10.25), p(4.0), p(12.0));
    let rr = if small { 1.5 } else { 1.75 * k };
    let (kx1, kx2) = if small { (9.5, 6.5) } else { (p(9.6), p(6.4)) };
    g.line(x0, y1, x1, y1, w, t.ink.mul_a(0.5), true);
    g.line(x0, y2, x1, y2, w, t.ink.mul_a(0.5), true);
    g.fill_circle(kx1, y1, rr, t.ac);
    g.fill_circle(kx2, y2, rr, t.ink);
}

/// Write `pane_dark.ico` and `pane_light.ico` (PNG-compressed entries) into `dir`.
pub fn make_icons(dir: &str) -> Result<()> {
    let g = Gfx::new(1.0);
    let sizes = [16u32, 20, 24, 32, 40, 48, 64, 128, 256];
    for light in [false, true] {
        let mut pngs: Vec<(u32, Vec<u8>)> = Vec::new();
        for &sz in &sizes {
            let mut surf = new_surface(sz as i32, sz as i32).ok_or_else(|| Error::from(windows::Win32::Foundation::E_OUTOFMEMORY))?;
            g.begin(surf.canvas());
            draw(&g, sz as f32, light);
            g.end();
            let px = png::from_surface(&mut surf);
            let tmp = format!("{}\\_tmp_{}.png", dir, sz);
            png::save_png(&px, &tmp)?;
            pngs.push((sz, std::fs::read(&tmp).unwrap_or_default()));
            let keep = format!("{}\\pane_{}_{}.png", dir, if light { "light" } else { "dark" }, sz);
            let _ = std::fs::rename(&tmp, &keep);
        }
        let mut ico: Vec<u8> = Vec::new();
        ico.extend_from_slice(&[0, 0, 1, 0]);
        ico.extend_from_slice(&(pngs.len() as u16).to_le_bytes());
        let mut off = 6 + 16 * pngs.len() as u32;
        for (sz, data) in &pngs {
            let b = if *sz >= 256 { 0u8 } else { *sz as u8 };
            ico.extend_from_slice(&[b, b, 0, 0]);
            ico.extend_from_slice(&1u16.to_le_bytes());
            ico.extend_from_slice(&32u16.to_le_bytes());
            ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
            ico.extend_from_slice(&off.to_le_bytes());
            off += data.len() as u32;
        }
        for (_, data) in &pngs {
            ico.extend_from_slice(data);
        }
        std::fs::write(format!("{}\\pane_{}.ico", dir, if light { "light" } else { "dark" }), ico).map_err(|_| Error::from(windows::Win32::Foundation::E_FAIL))?;
    }
    Ok(())
}
