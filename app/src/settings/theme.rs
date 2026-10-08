//! Settings › Theme (Order 033): Dark glass · Light glass · Match Windows (menu-v22: `S.theme` 'dark' / 'light' / 'auto',
//! `applyTheme`: light = 'light', or 'auto' while Windows' apps are light). Dark is the default.

/// The three theme choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    /// follows Windows' app theme (`AppsUseLightTheme`), live
    MatchWindows,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::Dark, Theme::Light, Theme::MatchWindows];

    /// The id written to the settings file (the drawing's values).
    pub fn id(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::MatchWindows => "auto",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }

    /// Light glass with Windows' apps light (`windows_light`) or dark?
    pub fn light(self, windows_light: bool) -> bool {
        match self {
            Theme::Dark => false,
            Theme::Light => true,
            Theme::MatchWindows => windows_light,
        }
    }
}
