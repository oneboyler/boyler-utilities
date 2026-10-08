//! Boyler Utilities — the Controller tab's features. No UI.
//!
//! - **Steam Input layouts** per Steam game: find Steam, the account, the games with a layout, read the active layout and
//!   write any setting the page shows ([`settings::Change`]) keeping every other byte of the file identical
//!   ([`vdf::Doc`]); a backup of the original before the first write, one undo step per write, "Steam's setting for this",
//!   "Steam's layout" and "Back to how your PC was" ([`ControllerService`]). Steam re-reads a game's layout when the game
//!   window gets focus (measured live Oct 8), so no Steam restart is needed — and the app never starts, closes or
//!   restarts Steam.
//! - **This controller · all games**: Steam's `preferences_<serial>.vdf` ([`prefs`]).
//! - **Controllers**: the connected ones, their type and battery ([`os::PadOs`]).
//! - **Live view**: sticks, buttons, triggers of the selected controller while the page is open ([`live::LiveView`]).
//!
//! The OS layer sits behind [`os::SteamOs`] / [`os::PadOs`]: the real Windows code ([`real`]) and a fake for tests
//! ([`fake`]). Nothing here ever sends anything TO a controller.

pub mod binding;
pub mod binvdf;
pub mod error;
pub mod fake;
pub mod layout;
pub mod live;
pub mod os;
pub mod parts;
pub mod prefs;
#[cfg(windows)]
pub mod real;
pub mod service;
pub mod settings;
pub mod steam;
pub mod vdf;

pub use binding::{Action, MouseButton, PadButton, SteamAction};
pub use error::{Error, Result};
pub use layout::{ActionSet, Layout, Press};
pub use live::{LiveState, LiveView};
pub use os::{Battery, BatteryLevel, Connection, PadInfo, PadOs, PadSource, SteamOs};
pub use parts::{ButtonId, PadKind, Part, Side};
pub use prefs::{PrefSetting, Prefs};
pub use service::ControllerService;
pub use settings::{Change, GyroMode, GyroSetting, PadView, PressSetting, StickMode, StickSetting, TouchMode, TouchSetting, TriggerSetting};
pub use steam::{Game, LayoutSource, SteamPaths};
