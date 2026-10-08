//! The shared UI pieces (Order 014), built ONCE from the drawing's CSS (menu-v22.html), each its own module. A page never
//! draws its own button, toggle, slider, popup...: it calls these. Every number here is the drawing's CSS number; the
//! CSS rule each one copies is quoted on it. Options and use: app/PAGES.md.
//!
//! A piece is a plain function returning an `El` (boxes); the page owns the value, the `Cx` owns hover / press /
//! transitions. A piece that reacts to the mouse takes a `Key`; the page gets `Ev::Click(key)` (or `Press` / `Drag` for a
//! slider, `Char` / `Key` for a text field) and changes its value.

use taffy::style::JustifyContent;

use super::el::El;
use super::{F20, FG};
use crate::gfx::Font;

pub mod badge;
pub mod bits;
pub mod button;
pub mod card;
pub mod dialog;
pub mod dropdown;
pub mod fold;
pub mod group;
pub mod ibtn;
pub mod inote;
pub mod keyfield;
pub mod link;
pub mod listrow;
pub mod mbtn;
pub mod mitems;
pub mod nbox;
pub mod progress;
pub mod ptl;
pub mod reset;
pub mod rowbits;
pub mod selbar;
pub mod search;
pub mod seg;
pub mod segx;
pub mod slider;
pub mod tinput;
pub mod tip;
pub mod toast;
pub mod toggle;
pub mod udlg;

/// Text inside a `<button>` of the drawing: `font: inherit` (size from its rule) but the browser's own
/// `letter-spacing: normal`.
pub const fn btn_font(size: f32, weight: u16) -> Font {
    Font::new(size, weight).ls(0)
}

/// The page header `.ph`: the title (`h2`, 600 20px/1.2 Segoe UI Variable Display, -.018em) and, at the right end
/// (`justify-content: space-between`), the ONE header element the drawing appends for the page (if any).
/// `.ph{display:flex;align-items:center;justify-content:space-between;gap:12px;margin:0 2px 8px;min-height:32px}`
pub fn header(title: &str, right: Option<El>) -> El {
    let h2 = El::text(title, F20, FG(), 24.0).none();
    El::row().center().justify(JustifyContent::SPACE_BETWEEN).gap(12.0).margin(0.0, 2.0, 8.0, 2.0).min_h(32.0).child(h2).children(right)
}

/// The header with its bottom margin changed (`.ph.pdh{margin-bottom:2px}` on Controller).
pub fn header_mb(title: &str, right: Option<El>, mb: f32) -> El {
    let h2 = El::text(title, F20, FG(), 24.0).none();
    El::row().center().justify(JustifyContent::SPACE_BETWEEN).gap(12.0).margin(0.0, 2.0, mb, 2.0).min_h(32.0).child(h2).children(right)
}

/// The thin separator between header parts (`.gxs{width:1px;height:11px;margin:0 4px;background:var(--hair)}`; `.phr .gxs{margin:0}`).
pub fn separator(margin: f32) -> El {
    El::block().size(1.0, 11.0).none().margin(0.0, margin, 0.0, margin).bg(super::HAIR())
}
