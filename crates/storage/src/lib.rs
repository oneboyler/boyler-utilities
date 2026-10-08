//! `bu-storage` — the Storage tab's features (DESIGN.md §3.12), no UI.
//!
//! * [`drives`]  — the drive tiles: every local drive with used / free, model, SSD / HDD.
//! * [`scan`]    — "What's using C:": a walk of one drive into a folder tree (biggest first, drill down) and a
//!   file-type breakdown ([`classify`] says how a file is classed).
//! * [`cleanup`] — Clean up with sizes first: recycle bin, temp files, shader caches, launcher caches. Only ticked
//!   rows are deleted, and only after they were measured.
//! * [`health`]  — drive health (SMART), read-only.
//!
//! Every Windows call goes through the [`StorageOs`] trait: [`RealOs`] (Windows) and [`FakeOs`] (tests).

pub mod classify;
pub mod cleanup;
pub mod drives;
mod error;
pub mod fake;
pub mod health;
mod os;
#[cfg(windows)]
pub mod real;
pub mod scan;

pub use error::{Result, StorageError};
pub use fake::FakeOs;
pub use os::*;
#[cfg(windows)]
pub use real::RealOs;
