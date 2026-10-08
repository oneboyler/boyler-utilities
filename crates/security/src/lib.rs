//! `bu-security` — the Security tab's features (DESIGN.md §3.14), no UI: a simple front for **Microsoft Defender**.
//!
//! * [`SecurityService::page`] — what the page shows: status banner, last scan, definitions, threats found, quarantine.
//! * [`SecurityService::start_scan`] / [`SecurityService::cancel_scan`] — Quick, Full and file / folder scans (the drop
//!   zone). A scan is a slow job: it starts only from the user's button, on a thread that lives only while it runs.
//! * [`SecurityService::offline_scan`] — restarts the PC into Defender Offline, only after an explicit confirmation.
//! * [`SecurityService::remove_threat`] / `allow_threat` / `restore_quarantined` / `undo` — admin (the page says so).
//!
//! Every Windows call goes through the [`SecurityOs`] trait: [`RealOs`] (Windows) and [`FakeOs`] (tests).
//! Defender is never switched off or changed beyond the page's own actions.

mod error;
pub mod fake;
pub mod model;
mod os;
#[cfg(windows)]
pub mod real;
mod service;

pub use error::{Result, SecurityError};
pub use fake::FakeOs;
pub use model::*;
pub use os::{CancelToken, ScanExit, SecurityOs};
#[cfg(windows)]
pub use real::RealOs;
pub use service::*;
