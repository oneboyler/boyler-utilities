//! Real cursor pictures for the Mouse page (Order 066): a `.cur` / `.ani` file read with `bu_mouse::curfile` and painted as
//! it is - in the 7 bubbles, in every row of the role pickers and in "Get more cursors". (Before this the pickers drew the
//! drawing's stand-in arrow for every set, so every row looked the same.) Files are read when first painted and kept in a
//! small cache that is emptied when a picker closes; a file that can't be read is simply not drawn (the stand-in shows).

use crate::gfx::Gfx;
use bu_mouse::curfile::{self, CursorImage};
use skia_safe as sk;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// One decoded cursor, ready to paint.
pub struct Pic {
    img: sk::Image,
    /// the picture's whole size
    w: f32,
    h: f32,
    /// the part that has pixels (l, t, r, b)
    ink: (f32, f32, f32, f32),
}

thread_local! {
    static CACHE: RefCell<HashMap<String, Option<Rc<Pic>>>> = RefCell::new(HashMap::new());
}

/// The size read from a multi-size file: big enough to be sharp in a 44 px tile at any screen scale.
const WANT: u32 = 64;

/// Makes a [`Pic`] of a decoded cursor (`None` = nothing to see in it).
pub fn from_image(ci: &CursorImage) -> Option<Pic> {
    let (w, h) = (ci.w as i32, ci.h as i32);
    // the part with pixels, so the cursor is centred in its tile (a 32 px file has its arrow in a corner)
    let (mut l, mut t, mut r, mut b) = (w, h, -1i32, -1i32);
    for y in 0..h {
        for x in 0..w {
            if ci.rgba[((y * w + x) * 4 + 3) as usize] > 12 {
                l = l.min(x);
                t = t.min(y);
                r = r.max(x);
                b = b.max(y);
            }
        }
    }
    if r < 0 {
        return None;
    }
    let info = sk::ImageInfo::new((w, h), sk::ColorType::RGBA8888, sk::AlphaType::Unpremul, Some(sk::ColorSpace::new_srgb()));
    let img = sk::images::raster_from_data(&info, sk::Data::new_copy(&ci.rgba), (w * 4) as usize)?;
    Some(Pic { img, w: w as f32, h: h as f32, ink: (l as f32, t as f32, (r + 1) as f32, (b + 1) as f32) })
}

/// The picture of a cursor file (cached; `None` = unreadable, an empty path, the built-in cursor).
pub fn load(path: &str) -> Option<Rc<Pic>> {
    if path.is_empty() {
        return None;
    }
    CACHE.with(|c| {
        if let Some(p) = c.borrow().get(path) {
            return p.clone();
        }
        // never a network path (an offline share would block the UI thread for seconds) and never a huge file
        let local = !path.starts_with(r"\\") && std::fs::metadata(path).is_ok_and(|m| m.len() <= 4 * 1024 * 1024);
        let pic = if local { std::fs::read(path).ok().and_then(|b| curfile::decode(&b, WANT)).and_then(|ci| from_image(&ci)).map(Rc::new) } else { None };
        c.borrow_mut().insert(path.to_string(), pic.clone());
        pic
    })
}

/// The picture of a decoded image kept under `key` (the list of downloadable packs).
pub fn keyed(key: &str, ci: &CursorImage) -> Option<Rc<Pic>> {
    CACHE.with(|c| {
        if let Some(p) = c.borrow().get(key) {
            return p.clone();
        }
        let pic = from_image(ci).map(Rc::new);
        c.borrow_mut().insert(key.to_string(), pic.clone());
        pic
    })
}

/// Empties the cache (a picker closed, a pack deleted).
pub fn forget() {
    CACHE.with(|c| c.borrow_mut().clear());
}

/// Paints `p` in the `size` px box at (x, y), centred, `scale` about the centre. The cursor keeps its own proportions and is
/// drawn as big as the picture's canvas allows (a bigger cursor in its file looks bigger), with the soft shadow cursors have.
pub fn paint(g: &Gfx, p: &Pic, x: f32, y: f32, size: f32, scale: f32) {
    // the whole canvas fits the box; the ink is centred in it
    let k = size / p.w.max(p.h) * scale;
    let (iw, ih) = ((p.ink.2 - p.ink.0) * k, (p.ink.3 - p.ink.1) * k);
    // never bigger than the box (a canvas-filling cursor)
    let fit = (size / iw.max(ih)).min(1.0);
    let (iw, ih) = (iw * fit, ih * fit);
    let dst = sk::Rect::from_xywh(x + (size - iw) / 2.0, y + (size - ih) / 2.0, iw, ih);
    let src = sk::Rect::new(p.ink.0, p.ink.1, p.ink.2, p.ink.3);
    let mut paint = sk::Paint::default();
    paint.set_anti_alias(true);
    if let Some(f) = sk::image_filters::drop_shadow((0.0, 0.8), (0.8, 0.8), sk::Color::from_argb(90, 0, 0, 0), None, None, None) {
        paint.set_image_filter(f);
    }
    let so = sk::SamplingOptions::new(sk::FilterMode::Linear, sk::MipmapMode::Linear);
    g.cv().draw_image_rect_with_sampling_options(&p.img, Some((&src, sk::canvas::SrcRectConstraint::Fast)), dst, so, &paint);
}
