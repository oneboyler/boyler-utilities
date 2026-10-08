//! Plain data the menu shows: groups, badges, row state, the result of a change.

use crate::defaults::ChangeAction;

/// The eight groups of DESIGN §3.6 "Plain switches", in page order. (Network moved to its own tab; Default apps is
/// `crate::defaults`.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    FilesExplorer,
    TaskbarStart,
    Gaming,
    Input,
    DevicesPower,
    Sound,
    PrivacyAds,
    Look,
}

impl Group {
    pub const ALL: [Group; 8] = [
        Group::FilesExplorer,
        Group::TaskbarStart,
        Group::Gaming,
        Group::Input,
        Group::DevicesPower,
        Group::Sound,
        Group::PrivacyAds,
        Group::Look,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Group::FilesExplorer => "Files & Explorer",
            Group::TaskbarStart => "Taskbar & Start",
            Group::Gaming => "Gaming",
            Group::Input => "Input",
            Group::DevicesPower => "Devices & power",
            Group::Sound => "Sound",
            Group::PrivacyAds => "Privacy & ads",
            Group::Look => "Look",
        }
    }
}

/// The tiny grey icons after a row's title (DESIGN §3.6 "Shared row bits").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Badge {
    /// shield
    Admin,
    /// restart arrow — Explorer restarts
    Explorer,
    /// restart arrow — sign out
    SignOut,
    /// restart arrow — PC restart
    Restart,
    /// restart arrow — next game start
    NextGame,
}

impl Badge {
    /// The tip text, word for word from DESIGN §3.6.
    pub fn tip(self) -> &'static str {
        match self {
            Badge::Admin => "Needs admin — Windows asks once",
            Badge::Explorer => "Restarts Explorer — the taskbar blinks once",
            Badge::SignOut => "Takes effect after you sign out",
            Badge::Restart => "Takes effect after a restart",
            Badge::NextGame => "Takes effect the next time a game starts",
        }
    }

    /// The toast a badge row shows after a flip (DESIGN: "a sign-out / restart / next-game row toasts its badge text. An Explorer
    /// row toasts 'Explorer restarts — the taskbar blinks once'"). Admin has its own pre-flip toast in the menu.
    pub fn toast(self) -> Option<&'static str> {
        match self {
            Badge::Admin => None,
            Badge::Explorer => Some("Explorer restarts — the taskbar blinks once"),
            other => Some(other.tip()),
        }
    }
}

/// What kind of control a row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// on / off switch
    Switch,
    /// a dropdown of timeouts (Screen off after, Sleep after)
    Timeout,
    /// the per-game Fullscreen-optimizations list
    GameList,
}

/// A power timeout. Windows stores seconds; 0 = never.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timeout {
    Never,
    Seconds(u32),
}

impl Timeout {
    pub fn from_seconds(s: u32) -> Self {
        if s == 0 { Timeout::Never } else { Timeout::Seconds(s) }
    }
    pub fn seconds(self) -> u32 {
        match self {
            Timeout::Never => 0,
            Timeout::Seconds(s) => s,
        }
    }
    /// "1 min", "30 min", "2 h", "Never" — the dropdown text of DESIGN §3.6.
    pub fn label(self) -> String {
        match self {
            Timeout::Never => "Never".into(),
            Timeout::Seconds(s) if s % 3600 == 0 => format!("{} h", s / 3600),
            Timeout::Seconds(s) if s % 60 == 0 => format!("{} min", s / 60),
            Timeout::Seconds(s) => format!("{s} s"),
        }
    }
}

/// The dropdown choices: 1, 2, 5, 10, 15, 30 min, 1, 2, 5 h, Never (DESIGN §3.6 v18).
pub const TIMEOUT_CHOICES: [Timeout; 10] = [
    Timeout::Seconds(60),
    Timeout::Seconds(120),
    Timeout::Seconds(300),
    Timeout::Seconds(600),
    Timeout::Seconds(900),
    Timeout::Seconds(1800),
    Timeout::Seconds(3600),
    Timeout::Seconds(7200),
    Timeout::Seconds(18000),
    Timeout::Never,
];

/// One game in the Fullscreen-optimizations list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsoGame {
    /// full .exe path as Windows stores it
    pub exe: String,
    /// true = fullscreen optimizations are OFF for this game (our flag is set)
    pub fso_off: bool,
}

/// A row's current value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Switch(bool),
    Timeout(Timeout),
    Games(Vec<FsoGame>),
}

/// What Windows is set to NOW for one row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowState {
    pub id: &'static str,
    pub value: Value,
    /// false = the switch is greyed (e.g. Fast Startup while Hibernate is off in Windows, Sleep after while Sleep is off)
    pub enabled: bool,
    /// the grey line shown when `enabled` is false
    pub disabled_reason: Option<&'static str>,
}

/// What a change did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub id: &'static str,
    /// the value read back from Windows after the change
    pub value: Value,
    /// the toast to show (badge text or the row's own toast), if any
    pub toast: Option<String>,
    /// Explorer was restarted as part of this change
    pub explorer_restarted: bool,
    /// something the menu must open for the user (e.g. Copilot's Store page) — Windows can't do it silently
    pub open: Option<ChangeAction>,
}
