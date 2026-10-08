//! The capture overlay of the Screenshots tab (menu-v22 "CAPTURE OVERLAY", DESIGN.md §3.3) - Order 019.
//!
//! - `model`: the overlay's state and rules (selection, marks, tools, Live / Snap, the size tag, the emoji picker, keys) -
//!   pure, tested.
//! - `emoji`: the quick row, the More list and its search.
//! - `view`: what each monitor's window shows (boxes from the drawing's CSS) and how it is painted.
//! - `ink`: the marks, drawn like the drawing's canvas. `compose`: the finished picture (box + marks).

pub mod capture;
pub mod compose;
pub mod emoji;
mod emoji_data;
pub mod ink;
pub mod model;
#[cfg(test)]
mod proof;
pub mod toast;
pub mod view;
#[cfg(windows)]
pub mod window;
