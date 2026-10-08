//! bu-display — the Windows features behind Boyler Utilities' Display tab (DESIGN.md §3.2). No UI.
//!
//! - Read: monitors (name, main, position, Windows scaling %, HDR read-only), each monitor's real modes with exact
//!   rates, the applied mode, brightness / contrast (DDC/CI) and vibrance.
//! - Change (all undoable): apply W × H × Hz + scaling with a 10 s keep / revert, main display, Windows scaling %,
//!   brightness / contrast, vibrance.
//! - Presets and per-app automatic switching (event-driven app watcher, no polling).
//!
//! The OS sits behind [`DisplayOs`]: [`win::WinDisplayOs`] is the real one, [`fake::FakeDisplayOs`] the test one.

pub mod autoswitch;
pub mod error;
pub mod fake;
pub mod fields;
pub mod keep;
pub mod os;
pub mod picture;
pub mod presets;
pub mod service;
pub mod store;
pub mod types;
#[cfg(windows)]
pub mod win;

pub use error::{DisplayError, Result};
pub use os::DisplayOs;
pub use service::DisplayService;
pub use types::*;
