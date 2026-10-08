//! Boyler Utilities - the Network tab's features (DESIGN.md §3.11), no UI.
//!
//! * Connection: Wi-Fi / Ethernet adapters, which one is in use, link speed, their switches ([`NetworkService`]).
//! * The ping pill ([`ping::PingSampler`]), Flush DNS, the DNS switcher (Automatic / Cloudflare / Google).
//! * Speed test with progress events ([`speedtest`]).
//! * Game-server ping ([`gameservers`]).
//!
//! Everything that touches Windows goes through [`os::NetworkOs`]: [`real::WindowsNet`] on the PC, [`fake::FakeNet`]
//! in tests. Nothing here runs in the background: the samplers exist only between `start` and `stop`.

pub mod error;
pub mod fake;
pub mod gameregions;
pub mod gameservers;
mod gamelist;
pub mod model;
pub mod os;
pub mod ping;
pub mod service;
pub mod speedtest;

#[cfg(windows)]
pub mod real;

pub use error::{NetError, Result};
pub use model::*;
pub use os::NetworkOs;
pub use service::{Action, Change, ChangeKind, NetworkService};
