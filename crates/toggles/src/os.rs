//! The OS layer behind a trait: the real Windows implementation is `crate::real::RealOs`, the fake is `crate::fake::FakeOs`.
//! The service (`crate::service::Toggles`) holds all logic and talks only to this trait.

use crate::error::Result;

/// Which registry root a row lives under. The real OS layer can map both onto a scratch key (tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Hive {
    /// HKEY_CURRENT_USER
    Hkcu,
    /// HKEY_LOCAL_MACHINE (writes need admin)
    Hklm,
}

/// A registry value as read from Windows. `Other` keeps any other type byte-exact so undo can put it back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegValue {
    Dword(u32),
    Sz(String),
    Other { kind: u32, bytes: Vec<u8> },
}

/// A `SystemParametersInfo` setting the toggles use. Each is read/written as one `u32`:
/// the `dwFlags` field for the accessibility structs, 0/1 for the BOOL ones, `iMinAnimate` for ANIMATIONINFO.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpiItem {
    /// STICKYKEYS.dwFlags (SPI_GET/SETSTICKYKEYS)
    StickyKeysFlags,
    /// FILTERKEYS.dwFlags (SPI_GET/SETFILTERKEYS)
    FilterKeysFlags,
    /// TOGGLEKEYS.dwFlags (SPI_GET/SETTOGGLEKEYS)
    ToggleKeysFlags,
    /// SPI_GET/SETCLIENTAREAANIMATION (BOOL)
    ClientAreaAnimation,
    /// ANIMATIONINFO.iMinAnimate (SPI_GET/SETANIMATION)
    MinimizeAnimation,
}

/// The "hot key pop-up" bit in STICKYKEYS / FILTERKEYS / TOGGLEKEYS `dwFlags` (SKF_/FKF_/TKF_HOTKEYACTIVE, all 0x4).
pub const HOTKEYACTIVE: u32 = 0x0000_0004;

/// A power-plan setting of the active scheme (GUIDs in `crate::real`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerSetting {
    /// VIDEOIDLE — "Turn off display after", seconds (0 = never)
    ScreenOff,
    /// STANDBYIDLE — "Sleep after", seconds (0 = never)
    Sleep,
    /// USB selective suspend — 1 = enabled, 0 = disabled
    UsbSelectiveSuspend,
}

/// Plugged-in (AC) and battery (DC) values of one power setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerValues {
    pub ac: u32,
    pub dc: u32,
}

/// GPU scheduling (HAGS) as the graphics driver reports it (D3DKMT WDDM 2.7 caps of the first adapter that supports it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuScheduling {
    pub supported: bool,
    /// on right now (a change shows only after a restart)
    pub enabled_now: bool,
    /// the driver's default when `HwSchMode` is not set
    pub enabled_by_default: bool,
}

/// One browser Windows knows (from `RegisteredApplications`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisteredBrowser {
    /// the value name under RegisteredApplications — goes into the Settings link
    pub reg_name: String,
    /// display name (Capabilities `ApplicationName`, resolved), or `reg_name`
    pub display_name: String,
    /// registered under HKLM (`registeredAppMachine=`) or HKCU (`registeredAppUser=`)
    pub machine: bool,
    /// the ProgId the browser registers for https
    pub https_progid: Option<String>,
}

/// The current default app for one file type or protocol, read-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssocApp {
    /// friendly name ("Photos", "VLC media player")
    pub name: String,
    /// the program file, when Windows reports one (for the small tile)
    pub exe: Option<String>,
}

/// Everything the toggles need from Windows. All logic sits in the service; implementations only read and write.
pub trait TogglesOs {
    // ---- registry ----
    fn reg_read(&self, hive: Hive, path: &str, name: &str) -> Result<Option<RegValue>>;
    fn reg_write(&mut self, hive: Hive, path: &str, name: &str, value: &RegValue) -> Result<()>;
    /// Deleting a missing value is not an error.
    fn reg_delete_value(&mut self, hive: Hive, path: &str, name: &str) -> Result<()>;
    fn reg_key_exists(&self, hive: Hive, path: &str) -> Result<bool>;
    fn reg_create_key(&mut self, hive: Hive, path: &str) -> Result<()>;
    /// Deletes the key and everything under it. Deleting a missing key is not an error.
    fn reg_delete_tree(&mut self, hive: Hive, path: &str) -> Result<()>;
    /// All values of a key (name, value). A missing key gives an empty list.
    fn reg_values(&self, hive: Hive, path: &str) -> Result<Vec<(String, RegValue)>>;

    // ---- process ----
    /// Is this process elevated (admin)?
    fn is_elevated(&self) -> bool;

    // ---- SystemParametersInfo ----
    fn spi_get(&self, item: SpiItem) -> Result<u32>;
    /// Writes and persists (SPIF_UPDATEINIFILE | SPIF_SENDCHANGE).
    fn spi_set(&mut self, item: SpiItem, value: u32) -> Result<()>;
    /// SPI_SETLANGTOGGLE: Windows re-reads the layout hotkeys from `HKCU\Keyboard Layout\Toggle`.
    fn reload_language_hotkeys(&mut self) -> Result<()>;

    // ---- power ----
    fn power_read(&self, setting: PowerSetting) -> Result<PowerValues>;
    /// Writes into the active scheme and re-applies it.
    fn power_write(&mut self, setting: PowerSetting, values: PowerValues) -> Result<()>;
    /// The PC has a battery (laptop) — then battery (DC) values are written too.
    fn has_battery(&self) -> bool;
    /// Hibernate is on in Windows (read only: Fast Startup needs it; the app never turns Hibernate on or off).
    fn hibernate_on(&self) -> Result<bool>;

    /// What the graphics driver says about GPU scheduling (read-only).
    fn gpu_scheduling(&self) -> Result<GpuScheduling>;

    // ---- devices / apps ----
    /// `None` = no Bluetooth radio.
    fn bluetooth(&self) -> Result<Option<bool>>;
    fn set_bluetooth(&mut self, on: bool) -> Result<()>;
    /// The Copilot app is installed for this user.
    fn copilot_installed(&self) -> Result<bool>;
    /// Uninstalls the Copilot app for this user.
    fn remove_copilot(&mut self) -> Result<()>;

    // ---- after-steps ----
    /// WM_SETTINGCHANGE to every top window; `area` = lParam string (None = no string).
    fn broadcast_setting_change(&mut self, area: Option<&str>) -> Result<()>;
    /// Restarts Explorer (the taskbar blinks once).
    fn restart_explorer(&mut self) -> Result<()>;
    /// Makes Explorer re-read "Show file extensions" / "Show hidden files" without a restart (what Folder Options' Apply
    /// does): the shell's own settings call, a shell change notice, and a refresh of every open folder window + the desktop.
    fn refresh_shell(&mut self) -> Result<()>;

    // ---- default apps (read-only + open Windows' own UI) ----
    fn registered_browsers(&self) -> Result<Vec<RegisteredBrowser>>;
    /// The ProgId of the current https default (UserChoice, READ only — never written).
    fn default_browser_progid(&self) -> Result<Option<String>>;
    /// `what` is ".png" or a protocol like "https".
    fn assoc_app(&self, what: &str) -> Result<Option<AssocApp>>;
    fn open_uri(&mut self, uri: &str) -> Result<()>;
    /// Windows' "Open with" list for one extension (SHOpenWithDialog).
    fn open_with_dialog(&mut self, ext: &str) -> Result<()>;
}
