//! bu-obs - Notifications for OBS, the engine (Order 035): a port of NotificationsForOBS (plain C,
//! C:\src\notifications-for-obs\src) with the same behaviour. No UI: the app draws the popups, the status icon and the
//! page, and routes the keys through its keys manager.
//!
//! - `ws` obs-websocket transport, `auth` its v5 authentication, `engine` the state machine (app.c), `service` its thread;
//! - `cfg` OBS's settings files (read-only), `ctl` the safe edits of OBS's profile / WebSocket config, `ini` the text edits;
//! - `keys` OBS's hotkey names, `monitors` monitor numbering + OBS display-capture matching, `settings` the feature's own
//!   settings + ClipPing's file import, `sound` the built-in sounds;
//! - `os` the OS trait: `real` (Windows) / `fake` (tests).

pub mod auth;
pub mod cfg;
pub mod ctl;
pub mod engine;
pub mod fake;
pub mod ini;
pub mod keys;
pub mod monitors;
pub mod os;
pub mod popup;
#[cfg(windows)]
pub mod real;
pub mod service;
pub mod settings;
pub mod sha256;
pub mod sound;
#[cfg(windows)]
pub mod watch;
pub mod ws;

pub use engine::{Color, Icon, Input, KeyWhich, KeysView, ObsChange, PopMsg, View};
pub use service::{Options, Service, Ui};
pub use settings::Settings;
