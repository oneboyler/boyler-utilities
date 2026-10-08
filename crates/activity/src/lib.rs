//! `bu-activity` — the Activity tab's features (DESIGN.md §3.18), no UI.
//!
//! * [`activity`] — the counter: which app is in front (time per app per day), idle / lock / sleep not counted, game
//!   time, "Count as a game / Not a game", "Don't count this app", the switch (off by default). Pure logic.
//! * [`views`]    — what the tab shows: Screen time · Games · Uptime tiles, Most used [Today | 7 days], Last 7 days.
//! * [`store`]    — kept on this PC only, one small text file per day (`%LOCALAPPDATA%\BoylerUtilities\activity\`);
//!   nothing is ever sent anywhere (this crate has no network code at all).
//! * [`games`]    — which exes count as games: everything inside a game launcher's install folders (Steam, Epic, Riot,
//!   Ubisoft, GOG, Xbox / Game Pass) + the user's right-click fix.
//! * [`watch`]    — (Windows) the event-driven watcher: `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` wakes it only when the
//!   front window changes; lock / sleep / sign-out arrive as window messages. No polling (one idle check — see there).
//!
//! The OS reads (uptime, launcher folders, app names, the data folder) go through [`ActivityOs`]: [`RealOs`] and
//! [`FakeOs`].

pub mod activity;
pub mod clock;
mod error;
pub mod games;
mod os;
#[cfg(windows)]
pub mod real;
pub mod store;
pub mod views;
#[cfg(windows)]
pub mod watch;

pub use activity::{Activity, FgApp, Settings};
pub use clock::{Clock, FakeClock, Stamp, SystemClock};
pub use error::{ActivityError, Result};
pub use os::{ActivityOs, FakeOs};
#[cfg(windows)]
pub use real::RealOs;
pub use store::{FileStore, MemStore, Store};
