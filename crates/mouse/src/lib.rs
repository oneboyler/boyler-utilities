//! bu-mouse — the Mouse tab's Windows features (DESIGN.md §3.4). No UI: the menu only shows the state this crate reads and
//! sends the changes this crate makes. Keys are never registered here (wave-2 rule: the app layer maps keys to actions).
//!
//! - [`device`] + [`pulsar`]: "Your mouse" — identify the connected mouse; DPI / polling / lift-off / battery for supported
//!   mice (Pulsar X2 CrazyLight, cMouse protocol); a brand web-settings link for the rest.
//! - [`accel`]: "Mouse acceleration" = Raw Accel — its curve maths (exactly Raw Accel's), its settings.json, presets,
//!   per-app switching, handing settings to its driver.
//! - [`settings`]: Windows' own mouse settings (speed, Enhance pointer precision, scroll lines, double-click, swap).
//! - [`cursors`]: scheme, roles, size, imported packs, the "Matches your other cursors" suggestion.
//! - [`glass`]: the app's own Glass cursor set (built-in files, written into the app's folder when the tab opens).
//! - [`os::MouseOs`]: the OS layer — `win::RealOs` (Windows) and [`fake::FakeOs`] (tests).

pub mod accel;
pub mod cursors;
pub mod curfile;
pub mod store;
pub mod zipdir;
pub mod device;
pub mod error;
pub mod fake;
pub mod glass;
pub mod models;
pub mod os;
pub mod pulsar;
pub mod service;
pub mod settings;
#[cfg(windows)]
pub mod win;

pub use error::{Error, Result};
pub use os::MouseOs;
pub use service::{AppDirs, Mouse, UndoKey};
