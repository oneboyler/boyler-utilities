//! bu-mouse — the Mouse tab's Windows features (DESIGN.md §3.4). No UI: the menu only shows the state this crate reads and
//! sends the changes this crate makes. Keys are never registered here (wave-2 rule: the app layer maps keys to actions).
//!
//! - [`device`] + [`family`]: "Your mouse" — identify the connected mouse; DPI / polling / lift-off / battery through the
//!   driver of its family ([`pulsar`], [`logitech`], [`razer`], [`steelseries`] - ported from OpenMouse, AGPL-3.0-or-later);
//!   a brand web-settings link for the rest.
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
pub mod family;
pub mod glass;
pub mod logitech;
pub mod models;
pub mod os;
pub mod pulsar;
pub mod razer;
pub mod service;
pub mod settings;
pub mod steelseries;
pub mod atk;
pub mod endgame;
pub mod lamzu;
pub mod wlmouse;
pub mod redragon;
pub mod attackshark;
pub mod gearhub;
pub mod vaxee;
pub mod corsair;
pub mod glorious;
pub mod ninjutso;
pub mod finalmouse;
pub mod zaunkoenig;
pub mod orbital;
pub mod rawm;
pub mod mchose;
pub mod delux;
pub mod dareu;
pub mod keychron;
pub mod incott;
pub mod ksnake;
pub mod microsoft;
pub mod moddo;
pub mod teevolution;
pub mod wallhack;
pub mod vgn;
pub mod asus;
pub mod ryunix;
pub mod rapoo;
#[cfg(windows)]
pub mod win;

pub use error::{Error, Result};
pub use os::MouseOs;
pub use service::{AppDirs, Mouse, UndoKey};
