//! `bu-audio` — the Audio tab's features (DESIGN.md §3.1), no UI.
//!
//! * [`service`] — devices (output / input, names, type glyph), the default device (all three roles), device volume +
//!   mute, device on/off (the last one stays on; switching off the one in use moves Windows first), the app mixer
//!   (sessions grouped per app: volume / mute, name / icon / colour), undo of every change.
//! * [`page`]    — the page while open: one worker thread (Windows' audio service can take 100–1076 ms to answer), a
//!   snapshot the UI reads without waiting, levels only while asked. Its thread ends when it is dropped (an icon read
//!   already started on the helper thread may still finish; the read names / icons stay cached, a few KB per app).
//! * [`keep`] + [`newvol`] + [`engine`] — "Keep my devices" and "New apps volume": pure rules fed with Windows' events.
//! * [`watch`]   — (Windows) the event listener for those two: `IMMNotificationClient` + `IAudioSessionNotification`
//!   on one sleeping thread — no polling while the menu is closed.
//!
//! Every Windows call goes through [`AudioOs`]: [`RealOs`] (Core Audio; [`RealOs::read_only`] refuses every change)
//! and [`FakeOs`] (tests). Tests never change real devices, volumes, mutes or defaults.

pub mod colour;
pub mod engine;
mod error;
pub mod fake;
pub mod keep;
pub mod model;
pub mod newvol;
mod os;
pub mod page;
#[cfg(windows)]
pub mod real;
pub mod service;
#[cfg(windows)]
pub mod watch;

pub use error::{AudioError, Result};
pub use fake::{FakeOs, SharedFake};
pub use model::*;
pub use os::{AudioOs, GREY};
#[cfg(windows)]
pub use real::RealOs;
pub use service::{AudioService, Change};
