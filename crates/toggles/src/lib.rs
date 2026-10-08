//! bu-toggles — the Toggles tab's Windows features (DESIGN.md §3.6, incl. the v18 rows). No UI: the menu only shows the state
//! this crate reads and sends the changes this crate makes.
//!
//! - [`rows`]: the data-driven list of every row (title, sub-line, badges, how to read / change it, after-step, source).
//! - [`service::Toggles`]: read, apply (with read-back), undo; per-game fullscreen optimizations.
//! - [`defaults`]: Default apps (read; "Change" = Windows' own window).
//! - [`os::TogglesOs`]: the OS layer — [`real::RealOs`] (Windows) and [`fake::FakeOs`] (tests).

pub mod defaults;
pub mod dxg;
pub mod error;
pub mod fake;
pub mod fso;
pub mod model;
pub mod os;
#[cfg(windows)]
pub mod real;
pub mod rows;
pub mod search;
pub mod service;

pub use error::{Error, Result};
pub use model::{Applied, Badge, FsoGame, Group, Kind, RowState, Timeout, Value, TIMEOUT_CHOICES};
pub use os::TogglesOs;
pub use service::Toggles;
