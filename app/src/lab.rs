//! Order 003 lab (dev command `--lab <dir>`): small scenes drawn with the app's Skia in several ways, to compare
//! against the same scene rendered by Chromium (tools/ref/lab_shot.js) and find Chromium's exact recipe.
//! Scene "grad" = tools/ref lab.html: background #203048; #a = a 512 x 64 white .16 -> 0 gradient in its own
//! composited layer (will-change: transform); #b = the same gradient painted straight onto the background at y 96.

use skia_safe as sk;

use crate::gfx::{new_surface, Gfx, Rgba};
use crate::png;

fn grad_paint(g: &Gfx, variant: &str) -> sk::Paint {
    let w0 = Rgba(1.0, 1.0, 1.0, 0.0);
    let sh = if variant == "legacy" {
        let cols = [sk::Color::from_argb(41, 255, 255, 255), sk::Color::from_argb(0, 255, 255, 255)];
        #[allow(deprecated)]
        sk::gradient_shader::linear(((0.0, 0.0), (512.0, 0.0)), &cols[..], None, sk::TileMode::Clamp, None, None).unwrap()
    } else {
        g.hgrad(0.0, 0.0, 512.0, 0.0, &[(0.0, Rgba(1.0, 1.0, 1.0, 0.16)), (1.0, w0)])
    };
    let mut p = sk::Paint::default();
    p.set_anti_alias(true);
    p.set_dither(variant != "nodither");
    p.set_shader(sh);
    p
}

pub fn run(dir: &str) {
    let g = Gfx::new(1.0);
    track(&g, dir);
    for variant in ["base", "legacy", "nodither", "nocs_layer"] {
        let mut s = new_surface(600, 200).unwrap();
        let c = s.canvas();
        c.clear(sk::Color::from_rgb(0x20, 0x30, 0x48));
        // #a in its own layer
        let mut l = if variant == "nocs_layer" {
            let ii = sk::ImageInfo::new((512, 64), sk::ColorType::BGRA8888, sk::AlphaType::Premul, None);
            sk::surfaces::raster(&ii, None, None).unwrap()
        } else {
            new_surface(512, 64).unwrap()
        };
        l.canvas().clear(sk::Color::TRANSPARENT);
        l.canvas().draw_rect(sk::Rect::from_xywh(0.0, 0.0, 512.0, 64.0), &grad_paint(&g, variant));
        let img = l.image_snapshot();
        s.canvas().draw_image(&img, (0, 0), None);
        // #b straight
        s.canvas().save();
        s.canvas().translate((0.0, 96.0));
        s.canvas().draw_rect(sk::Rect::from_xywh(0.0, 0.0, 512.0, 64.0), &grad_paint(&g, variant));
        s.canvas().restore();
        let px = png::from_surface(&mut s);
        let _ = png::save_png(&px, &format!("{}\\lab_{}.png", dir, variant));
    }
}

/// Scene "track" = tools lab track.html: 278 x 4 tracks (radius 2) inside opacity .42 wrappers on #203048; #a / #c a hard
/// stop at 100 %, #b at 50 %. Variants of the stop list handed to Skia.
fn track(g: &Gfx, dir: &str) {
    let f = Rgba::rgba(235, 235, 245, 0.6);
    let t = Rgba::rgba(120, 120, 128, 0.36);
    for variant in ["blink", "two", "solid"] {
        let mut s = new_surface(600, 200).unwrap();
        s.canvas().clear(sk::Color::from_rgb(0x20, 0x30, 0x48));
        g.begin(s.canvas());
        for (x, y, p) in [(164.0f32, 20.0f32, 1.0f32), (164.0, 60.0, 0.5), (5.0, 100.0, 1.0)] {
            g.push_layer_in(0.42, sk::Rect::from_xywh(x, y, 278.0, 4.0));
            let mut stops = vec![(p, f), (p, t)];
            if p > 0.0 {
                stops.insert(0, (0.0, f));
            }
            if p < 1.0 {
                stops.push((1.0, t));
            }
            if variant == "two" && p >= 1.0 {
                stops.truncate(2);
            }
            if variant == "solid" && p >= 1.0 {
                g.fill_rr(x, y, 278.0, 4.0, 2.0, f);
            } else {
                let sh = g.hgrad(x, 0.0, x + 278.0, 0.0, &stops);
                g.fill_rr_shader(x, y, 278.0, 4.0, 2.0, &sh, 1.0);
            }
            g.pop_layer();
        }
        g.end();
        let px = png::from_surface(&mut s);
        let _ = png::save_png(&px, &format!("{}\\track_{}.png", dir, variant));
    }
}
