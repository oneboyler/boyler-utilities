//! A list row with an app icon (menu-v22 `.ait` tile + name, as in the Audio mixer, Startup and Apps lists): the drawing's
//! gradient tile with a white line glyph, or the app's real icon.

use std::sync::Arc;

use crate::gfx::{sh, Font, Rgba, Shadow};
use crate::png::Pixels;
use crate::ui::el::{lh, El};
use crate::ui::{FG, FG2, WHITE};

use super::group::row;

/// The tile's shadows: `inset 0 0 0 .5px rgba(255,255,255,.24), inset 0 1px 0 rgba(255,255,255,.18), 0 1px 2px rgba(0,0,0,.22)`.
pub const TILE_SH: [Shadow; 1] = [sh(0.0, 1.0, 2.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.22))];

/// What a tile shows.
#[derive(Clone)]
pub enum Tile {
    /// the drawing's tile: a 135° gradient a -> b with a white ICON glyph
    Glyph { glyph: &'static str, a: Rgba, b: Rgba },
    /// the app's own icon (pixels from the exe)
    Icon(Arc<Pixels>),
}

/// `.ait{width:24px;height:24px;border-radius:6px;display:grid;place-items:center;box-shadow:...}`
/// `.ait svg{width:14px;height:14px;stroke:#fff;stroke-width:1.6}`. `size` = 24 (Audio), 26 (Apps, radius 7),
/// 20 (`.prn`, radius 5).
pub fn tile(t: &Tile, size: f32) -> El {
    let (r, gs) = match size as i32 {
        26 => (7.0, 15.0),
        20 => (5.0, 12.0),
        _ => (6.0, 14.0),
    };
    match t {
        Tile::Glyph { glyph, a, b } => El::block()
            .size(size, size)
            .none()
            .radius(r)
            .bg_linear(135.0, &[(0.0, *a), (1.0, *b)])
            .shadow(&TILE_SH)
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.24)), sh(0.0, 1.0, 0.0, 0.0, Rgba(1.0, 1.0, 1.0, 0.18))])
            .clip()
            .place_center()
            .child(El::icon(glyph, gs, 1.6, WHITE)),
        Tile::Icon(px) => {
            let px = px.clone();
            let id = Arc::as_ptr(&px) as usize;
            El::paint(move |g, (x, y, w, h)| {
                if let Some(img) = crate::png::to_image(&px) {
                    g.draw_image_rect(&img, x, y, w, h);
                }
            })
            .sig(id)
            .size(size, size)
            .none()
        }
    }
}

/// A list row: the tile, the name (13 px, ellipsis) with an optional second line (11 px --fg2), then `right` (values,
/// buttons, a switch).
pub fn list_row(first: bool, t: &Tile, name: &str, sub: Option<&str>, right: Vec<El>) -> El {
    let mut l = El::col().flex1().child(El::text(name, Font::new(13.0, 400), FG(), lh(13.0, 1.35)).ellipsis());
    if let Some(s) = sub {
        l = l.child(El::text(s, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0));
    }
    let lbl = El::row().center().gap(10.0).flex1().child(tile(t, 24.0)).child(l);
    row(first, vec![lbl]).children(right)
}
