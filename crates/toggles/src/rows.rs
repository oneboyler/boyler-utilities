//! The data-driven list of every Toggles row of DESIGN.md §3.6 (incl. the v18 rows). Each row says what it is (title, sub-line,
//! badges), how to read and change it (`Method`), what must happen after (`Broadcast`, Explorer badge), and where the method
//! came from (`source`). The service (`crate::service`) runs the methods; nothing here touches Windows.
//!
//! Not here on purpose:
//! - "Memory integrity" — NEVER (Vanguard needs it).
//! - Network rows (DNS, Wi-Fi…) — the Network tab (Order 008).

use crate::model::{Badge, Group, Kind};
use crate::os::{Hive, PowerSetting, RegValue, SpiItem};

/// `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced`
pub const ADV: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";
/// `HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager`
pub const CDM: &str = r"Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager";
/// `HKCU\Software\Microsoft\DirectX\UserGpuPreferences` — value `DirectXUserGlobalSettings` (a `key=value;` list)
pub const DXG_PATH: &str = r"Software\Microsoft\DirectX\UserGpuPreferences";
pub const DXG_VALUE: &str = "DirectXUserGlobalSettings";
/// `HKCU\Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers` — value name = exe path
pub const LAYERS_PATH: &str = r"Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers";
/// The classic right-click menu key (create = classic menu, delete = Windows 11 menu)
pub const CLASSIC_MENU_KEY: &str = r"Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}";
pub const CLASSIC_MENU_SUBKEY: &str = r"Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32";
/// `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power` `HiberbootEnabled` (Fast Startup)
pub const HIBERBOOT_PATH: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Power";
pub const HIBERBOOT_VALUE: &str = "HiberbootEnabled";
/// `HKLM\SYSTEM\CurrentControlSet\Control\GraphicsDrivers` `HwSchMode` (GPU scheduling: 2 = on, 1 = off)
pub const HAGS_PATH: &str = r"SYSTEM\CurrentControlSet\Control\GraphicsDrivers";
pub const HAGS_VALUE: &str = "HwSchMode";

const CAM_MIC: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
const CAM_MIC_NP: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone\NonPackaged";
const CAM_CAM: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\webcam";
const CAM_CAM_NP: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\webcam\NonPackaged";
const LANG_TOGGLE: &str = r"Keyboard Layout\Toggle";

/// Registry data as written by a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegData {
    D(u32),
    S(&'static str),
}

impl RegData {
    pub fn to_value(self) -> RegValue {
        match self {
            RegData::D(d) => RegValue::Dword(d),
            RegData::S(s) => RegValue::Sz(s.to_string()),
        }
    }
    pub fn matches(self, v: &RegValue) -> bool {
        match (self, v) {
            (RegData::D(a), RegValue::Dword(b)) => a == *b,
            (RegData::S(a), RegValue::Sz(b)) => a.eq_ignore_ascii_case(b.trim_end_matches('\0')),
            // a number stored as text ("1") — some tweak tools do that
            (RegData::D(a), RegValue::Sz(b)) => b.trim().parse::<u32>().ok() == Some(a),
            _ => false,
        }
    }
}

/// One registry value of a registry row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegSpec {
    pub hive: Hive,
    pub path: &'static str,
    pub name: &'static str,
    /// written for "on" (unless `on_deletes`)
    pub on: RegData,
    /// written for "off"
    pub off: RegData,
    /// the state when the value is missing (Windows' default)
    pub absent_on: bool,
    /// the state for any value that is neither `on` nor `off`
    pub other_on: bool,
    /// "on" deletes the value instead of writing `on` (policies: no policy = Windows' default)
    pub on_deletes: bool,
}

impl RegSpec {
    /// The state this value says, given what is stored.
    pub fn state(&self, v: Option<&RegValue>) -> bool {
        match v {
            None => self.absent_on,
            Some(v) if self.on.matches(v) => true,
            Some(v) if self.off.matches(v) => false,
            Some(_) => self.other_on,
        }
    }
}

const fn dw(hive: Hive, path: &'static str, name: &'static str, on: u32, off: u32, absent_on: bool) -> RegSpec {
    RegSpec { hive, path, name, on: RegData::D(on), off: RegData::D(off), absent_on, other_on: true, on_deletes: false }
}
const fn cu(path: &'static str, name: &'static str, on: u32, off: u32, absent_on: bool) -> RegSpec {
    dw(Hive::Hkcu, path, name, on, off, absent_on)
}
/// A policy value: off = write `off`, on = delete the value (Windows' default).
const fn policy(hive: Hive, path: &'static str, name: &'static str, off: u32) -> RegSpec {
    RegSpec { hive, path, name, on: RegData::D(if off == 0 { 1 } else { 0 }), off: RegData::D(off), absent_on: true, other_on: true, on_deletes: true }
}
const fn sz(hive: Hive, path: &'static str, name: &'static str, on: &'static str, off: &'static str, absent_on: bool) -> RegSpec {
    RegSpec { hive, path, name, on: RegData::S(on), off: RegData::S(off), absent_on, other_on: true, on_deletes: false }
}

/// How several registry values make one switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combine {
    /// on when ANY value is on ("on always means the thing is on" — even partly)
    Any,
    /// on only when ALL values are on
    All,
}

/// How a row reads and changes its setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// One or more registry values.
    Reg { values: &'static [RegSpec], combine: Combine },
    /// The setting is "a key exists" (Classic right-click menu): on = create `key` with an empty default value, off = delete
    /// `delete` (the whole CLSID key).
    KeyExists { hive: Hive, key: &'static str, delete: &'static str },
    /// One `key=value;` entry inside DXG `DirectXUserGlobalSettings`; on = `key=1;`, off = `key=0;`. Other entries kept.
    Dxg { key: &'static str, absent_on: bool },
    /// One bit of a SystemParametersInfo flags value; on = bit set.
    SpiFlag { item: SpiItem, bit: u32 },
    /// Animations: SPI client-area animation + minimize/maximize animation; on when either is on.
    Animations,
    /// Stop Alt+Shift / Ctrl+Shift layout switching: the registry values, then SPI_SETLANGTOGGLE.
    LangHotkeys,
    /// A power setting used as a switch (USB selective suspend: 1 = on).
    PowerSwitch(PowerSetting),
    /// A power timeout dropdown (Screen off after, Sleep after).
    PowerTimeout(PowerSetting),
    /// Sleep on/off: off = sleep timeout 0; on = the last timeout back (or 30 min).
    SleepSwitch,
    /// GPU scheduling: HKLM `GraphicsDrivers` `HwSchMode` 2 = on / 1 = off (what Settings sets; applies after a restart); when it
    /// is not set, what the driver says is running now (D3DKMT WDDM 2.7 caps). Greyed when the graphics card doesn't support it.
    Hags,
    /// Fast Startup: HKLM HiberbootEnabled; greyed while Hibernate is off.
    FastStartup,
    /// Bluetooth radio (Windows.Devices.Radios).
    Bluetooth,
    /// The Copilot app (installed = on; off uninstalls it for this user; on = open its Store page).
    Copilot,
    /// The per-game Fullscreen optimizations list (AppCompatFlags Layers).
    FsoGames,
}

/// The WM_SETTINGCHANGE broadcast after a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Broadcast {
    None,
    /// lParam = NULL
    Plain,
    /// lParam = this string ("ImmersiveColorSet", "Policy", "TraySettings", …)
    Area(&'static str),
}

/// When a change shows up (for the report and the docs; the menu shows `badges`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applies {
    Live,
    Explorer,
    NextWindow,
    SignOut,
    Restart,
    NextGame,
    NextShutdown,
    /// research marked the timing "(?)" — not confirmed by a source
    Unconfirmed,
}

/// One Toggles row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: &'static str,
    pub group: Group,
    pub title: &'static str,
    pub sub: &'static str,
    pub badges: &'static [Badge],
    pub kind: Kind,
    pub method: Method,
    pub broadcast: Broadcast,
    pub applies: Applies,
    /// Settings page to open when Windows blocks the write (UCPD) — research v2 §8: "read each value back … fall back to an
    /// 'Open in Settings' link".
    pub settings_uri: Option<&'static str>,
    /// hidden search words (besides title, sub-line, group name)
    pub keywords: &'static [&'static str],
    /// after the write, Explorer re-reads its folder settings without a restart (`TogglesOs::refresh_shell`, Order 043)
    pub refresh_shell: bool,
    /// the row's own toasts (on, off) from DESIGN §3.6 "Own toasts"
    pub toast_on: Option<&'static str>,
    pub toast_off: Option<&'static str>,
    /// where the method came from
    pub source: &'static str,
}

impl Row {
    pub fn needs_admin(&self) -> bool {
        self.badges.contains(&Badge::Admin)
    }
    pub fn restarts_explorer(&self) -> bool {
        self.badges.contains(&Badge::Explorer)
    }
}

const E: &[Badge] = &[Badge::Explorer];
const NONE: &[Badge] = &[];
const ADMIN: &[Badge] = &[Badge::Admin];
const ADMIN_EXPLORER: &[Badge] = &[Badge::Admin, Badge::Explorer];
const ADMIN_RESTART: &[Badge] = &[Badge::Admin, Badge::Restart];
const RESTART: &[Badge] = &[Badge::Restart];
const NEXT_GAME: &[Badge] = &[Badge::NextGame];
const SIGN_OUT: &[Badge] = &[Badge::SignOut];

const V2: &str = "research ideas-v2.md §8";
const V3: &str = "research ideas-v3.md §5";

#[allow(clippy::too_many_arguments)]
const fn row(
    id: &'static str,
    group: Group,
    title: &'static str,
    sub: &'static str,
    badges: &'static [Badge],
    method: Method,
    broadcast: Broadcast,
    applies: Applies,
    settings_uri: Option<&'static str>,
    keywords: &'static [&'static str],
    source: &'static str,
) -> Row {
    Row {
        id,
        group,
        title,
        sub,
        badges,
        kind: Kind::Switch,
        method,
        broadcast,
        applies,
        settings_uri,
        keywords,
        refresh_shell: false,
        toast_on: None,
        toast_off: None,
        source,
    }
}

const fn reg(values: &'static [RegSpec]) -> Method {
    Method::Reg { values, combine: Combine::Any }
}

/// Every row, in page order.
pub static ROWS: &[Row] = &[
    // ---------------- Files & Explorer ----------------
    // Order 043: no Explorer restart — the shell's own refresh (what Folder Options' Apply does) shows it in open windows
    Row {
        refresh_shell: true,
        ..row("show_file_extensions", Group::FilesExplorer, "Show file extensions", "photo.jpg instead of photo", NONE,
            reg(&[cu(ADV, "HideFileExt", 0, 1, false)]), Broadcast::None, Applies::Live,
            Some("ms-settings:developers"), &["extension", "ext", "file type", "HideFileExt"], V2)
    },
    Row {
        refresh_shell: true,
        ..row("show_hidden_files", Group::FilesExplorer, "Show hidden files", "Files and folders Windows hides", NONE,
            reg(&[RegSpec { other_on: false, ..cu(ADV, "Hidden", 1, 2, false) }]), Broadcast::None, Applies::Live,
            Some("ms-settings:developers"), &["hidden", "invisible", "dot files"], V2)
    },
    row("classic_context_menu", Group::FilesExplorer, "Classic right-click menu", "The full menu, no “Show more options”", E,
        Method::KeyExists { hive: Hive::Hkcu, key: CLASSIC_MENU_SUBKEY, delete: CLASSIC_MENU_KEY },
        Broadcast::None, Applies::Explorer, None, &["context menu", "show more options", "windows 10 menu"], V2),
    row("explorer_opens_this_pc", Group::FilesExplorer, "Explorer opens on This PC", "Instead of Home", NONE,
        reg(&[RegSpec { other_on: false, ..cu(ADV, "LaunchTo", 1, 2, false) }]), Broadcast::None, Applies::NextWindow,
        None, &["this pc", "home", "quick access", "launch"], V2),
    row("copy_window_details", Group::FilesExplorer, "Copy window shows details", "The speed graph when copying files", NONE,
        reg(&[cu(r"Software\Microsoft\Windows\CurrentVersion\Explorer\OperationStatusManager", "EnthusiastMode", 1, 0, false)]),
        Broadcast::None, Applies::Unconfirmed, None, &["copy", "speed", "transfer", "progress"], V2),
    row("onedrive_ads_explorer", Group::FilesExplorer, "OneDrive ads in File Explorer", "“Try Microsoft 365” bars at the top", NONE,
        reg(&[cu(ADV, "ShowSyncProviderNotifications", 1, 0, true)]), Broadcast::None, Applies::NextWindow,
        None, &["onedrive", "microsoft 365", "sync provider", "ads"], V3),
    // ---------------- Taskbar & Start ----------------
    row("end_task", Group::TaskbarStart, "End task on right-click", "Close a stuck app from the taskbar", NONE,
        reg(&[cu(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced\TaskbarDeveloperSettings", "TaskbarEndTask", 1, 0, false)]),
        Broadcast::Area("TraySettings"), Applies::Unconfirmed, Some("ms-settings:developers"), &["kill", "frozen", "not responding"], V2),
    row("clock_seconds", Group::TaskbarStart, "Seconds on the clock", "", NONE,
        reg(&[cu(ADV, "ShowSecondsInSystemClock", 1, 0, false)]), Broadcast::Area("TraySettings"), Applies::Live,
        Some("ms-settings:taskbar"), &["time", "clock", "seconds"], V2),
    // Order 043: the Windows 11 taskbar watches its Advanced key — it moves at once, no Explorer restart
    row("start_on_left", Group::TaskbarStart, "Start on the left", "Taskbar icons on the left, like Windows 10", NONE,
        reg(&[RegSpec { other_on: false, ..cu(ADV, "TaskbarAl", 0, 1, false) }]), Broadcast::Area("TraySettings"), Applies::Live,
        Some("ms-settings:taskbar"), &["alignment", "center", "centre", "left", "windows 10"],
        "research ideas-v2.md §8 + pureinfotech.com/align-taskbar-icons-left-windows-11 (\"the Taskbar updates immediately\")"),
    row("task_view_button", Group::TaskbarStart, "Task View button", "", NONE,
        reg(&[cu(ADV, "ShowTaskViewButton", 1, 0, true)]), Broadcast::Area("TraySettings"), Applies::Unconfirmed,
        Some("ms-settings:taskbar"), &["task view", "desktops", "virtual desktop"], V2),
    row("start_recommendations", Group::TaskbarStart, "Recommendations in Start", "Tips and new-app ads", E,
        reg(&[cu(ADV, "Start_IrisRecommendations", 1, 0, true)]), Broadcast::None, Applies::Explorer,
        Some("ms-settings:personalization-start"), &["tips", "ads", "iris", "suggestions"], V2),
    row("taskbar_flashing", Group::TaskbarStart, "Taskbar flashing", "Apps blink orange for attention", NONE,
        reg(&[cu(ADV, "TaskbarFlashing", 1, 0, true)]), Broadcast::Area("TraySettings"), Applies::Unconfirmed,
        Some("ms-settings:taskbar"), &["flash", "blink", "orange", "attention"], V3),
    // Order 043: Alt + Tab reads it each time it opens — no Explorer restart
    row("alt_tab_edge_tabs", Group::TaskbarStart, "Alt + Tab shows Edge tabs", "Off = only real windows", NONE,
        // 3 = open windows only; on writes 2 (windows + 3 most recent tabs) unless undo has the old value
        reg(&[cu(ADV, "MultiTaskingAltTabFilter", 2, 3, true)]), Broadcast::None, Applies::Live,
        Some("ms-settings:multitasking"), &["alt tab", "edge", "browser tabs", "multitasking"],
        "research ideas-v3.md §5 + winaero.com/how-to-disable-microsoft-edge-tabs-in-alttab-on-windows-11 (\"effective immediately\")"),
    row("start_recommended_section", Group::TaskbarStart, "“Recommended” section in Start",
        "Hides the whole section · recent files and new apps", ADMIN_EXPLORER,
        reg(&[policy(Hive::Hklm, r"SOFTWARE\Policies\Microsoft\Windows\Explorer", "HideRecommendedSection", 1)]),
        Broadcast::Area("Policy"), Applies::Explorer, Some("ms-settings:personalization-start"),
        &["recommended", "recent files", "start menu"], "DESIGN §3.6 v18 (guess) + ntlite.com / whysogeek.com (Pro honours it from 25H2)"),
    // ---------------- Gaming ----------------
    row("game_mode", Group::Gaming, "Game Mode", "Windows puts the game first", NONE,
        reg(&[cu(r"Software\Microsoft\GameBar", "AutoGameModeEnabled", 1, 0, true)]), Broadcast::None, Applies::Unconfirmed,
        Some("ms-settings:gaming-gamemode"), &["game mode", "fps"], V2),
    row("xbox_background_recording", Group::Gaming, "Xbox background recording", "Always records gameplay · costs FPS", RESTART,
        reg(&[
            cu(r"Software\Microsoft\Windows\CurrentVersion\GameDVR", "AppCaptureEnabled", 1, 0, true),
            cu(r"System\GameConfigStore", "GameDVR_Enabled", 1, 0, true),
        ]),
        Broadcast::None, Applies::Restart, Some("ms-settings:gaming-gamedvr"), &["game dvr", "capture", "record", "xbox", "fps"], V2),
    row("xbox_button_game_bar", Group::Gaming, "Xbox button opens Game Bar", "On a controller", NONE,
        reg(&[cu(r"Software\Microsoft\GameBar", "UseNexusForGameBarEnabled", 1, 0, true)]), Broadcast::None, Applies::Unconfirmed,
        Some("ms-settings:gaming-gamebar"), &["controller", "guide button", "nexus", "game bar"], V2),
    row("windowed_game_optimizations", Group::Gaming, "Optimizations for windowed games",
        "Lower latency in windowed and borderless games", NEXT_GAME,
        Method::Dxg { key: "SwapEffectUpgradeEnable", absent_on: false }, Broadcast::None, Applies::NextGame,
        Some("ms-settings:display-advancedgraphics"), &["borderless", "flip model", "latency", "swap effect"], V3),
    row("auto_hdr", Group::Gaming, "Auto HDR", "HDR colours in older games · HDR screens only", NEXT_GAME,
        Method::Dxg { key: "AutoHDREnable", absent_on: false }, Broadcast::None, Applies::NextGame,
        Some("ms-settings:display-advancedgraphics"), &["hdr", "colour", "color"], V3),
    row("variable_refresh_rate", Group::Gaming, "Variable refresh rate", "Smoother DirectX 11 games · G-Sync / FreeSync screens",
        NEXT_GAME, Method::Dxg { key: "VRROptimizeEnable", absent_on: false }, Broadcast::None, Applies::NextGame,
        Some("ms-settings:display-advancedgraphics"), &["vrr", "gsync", "g-sync", "freesync", "adaptive sync"], V3),
    row("gpu_scheduling", Group::Gaming, "Hardware-accelerated GPU scheduling", "", ADMIN_RESTART,
        Method::Hags, Broadcast::None, Applies::Restart, Some("ms-settings:display-advancedgraphics"), &["hags", "gpu", "scheduling", "latency"],
        V2),
    Row {
        kind: Kind::GameList,
        ..row("fullscreen_optimizations_off", Group::Gaming, "Fullscreen optimizations off",
            "Set it before the game starts — Windows reads it at launch.", NONE,
            Method::FsoGames, Broadcast::None, Applies::NextGame, None,
            &["fullscreen", "fso", "exclusive", "compatibility", "per game"],
            "DESIGN §3.6 v18 (guess: AppCompatFlags Layers ~ DISABLEDXMAXIMIZEDWINDOWEDMODE)")
    },
    // ---------------- Input ----------------
    row("stop_layout_hotkeys", Group::Input, "Stop Alt + Shift switching your keyboard layout",
        "Ctrl + Shift too · your layouts stay, only the shortcut goes", NONE,
        Method::LangHotkeys, Broadcast::None, Applies::Live, Some("ms-settings:typing"),
        &["language", "keyboard layout", "alt shift", "ctrl shift", "input language"],
        "DESIGN §3.6 v18 (guess) + Microsoft: Keyboard Layout\\Toggle values 1/2/3 (3 = not assigned)"),
    row("sticky_keys_popup", Group::Input, "Sticky Keys pop-up", "Shift pressed 5 times", NONE,
        Method::SpiFlag { item: SpiItem::StickyKeysFlags, bit: crate::os::HOTKEYACTIVE }, Broadcast::None, Applies::Live,
        Some("ms-settings:easeofaccess-keyboard"), &["sticky", "shift", "accessibility"], V2),
    row("filter_keys_popup", Group::Input, "Filter Keys pop-up", "Right Shift held for 8 seconds", NONE,
        Method::SpiFlag { item: SpiItem::FilterKeysFlags, bit: crate::os::HOTKEYACTIVE }, Broadcast::None, Applies::Live,
        Some("ms-settings:easeofaccess-keyboard"), &["filter", "shift", "accessibility"], V2),
    row("toggle_keys_popup", Group::Input, "Toggle Keys pop-up", "Num Lock held for 5 seconds", NONE,
        Method::SpiFlag { item: SpiItem::ToggleKeysFlags, bit: crate::os::HOTKEYACTIVE }, Broadcast::None, Applies::Live,
        Some("ms-settings:easeofaccess-keyboard"), &["toggle keys", "num lock", "caps lock", "beep", "accessibility"], V3),
    row("clipboard_history", Group::Input, "Clipboard history", "Win + V shows what you copied", NONE,
        reg(&[cu(r"Software\Microsoft\Clipboard", "EnableClipboardHistory", 1, 0, false)]), Broadcast::None, Applies::Unconfirmed,
        Some("ms-settings:clipboard"), &["clipboard", "win v", "copy", "paste"], V2),
    row("scroll_inactive_windows", Group::Input, "Scroll inactive windows", "Scroll whatever is under the mouse", SIGN_OUT,
        reg(&[cu(r"Control Panel\Desktop", "MouseWheelRouting", 2, 0, true)]), Broadcast::Plain, Applies::SignOut,
        Some("ms-settings:mousetouchpad"), &["mouse wheel", "scroll", "hover"], V2),
    Row {
        toast_on: Some("Print Screen opens Snipping Tool again · your Screenshot key loses it"),
        toast_off: Some("Print Screen is yours: the Screenshot key gets it"),
        ..row("print_screen_snipping", Group::Input, "Print Screen opens Snipping Tool",
            "Off = your Screenshot key gets Print Screen", NONE,
            reg(&[cu(r"Control Panel\Keyboard", "PrintScreenKeyForSnippingEnabled", 1, 0, true)]), Broadcast::Plain,
            Applies::Unconfirmed, Some("ms-settings:easeofaccess-keyboard"), &["prtsc", "print screen", "snipping", "screenshot"], V3)
    },
    // ---------------- Devices & power ----------------
    Row {
        toast_on: Some("Bluetooth on"),
        toast_off: Some("Bluetooth off · wireless headsets and controllers disconnect"),
        ..row("bluetooth", Group::DevicesPower, "Bluetooth", "Off drops wireless headsets and controllers", NONE,
            Method::Bluetooth, Broadcast::None, Applies::Live, Some("ms-settings:bluetooth"), &["bt", "wireless", "radio"], V3)
    },
    Row {
        toast_on: Some("USB power saving on"),
        toast_off: Some("USB power saving off · mice and keyboards stay awake"),
        ..row("usb_power_saving", Group::DevicesPower, "USB power saving", "Off stops mice and keyboards dropping out", ADMIN,
            Method::PowerSwitch(PowerSetting::UsbSelectiveSuspend), Broadcast::None, Applies::Live, Some("ms-settings:powersleep"),
            &["usb", "selective suspend", "disconnect", "mouse drops"], V3)
    },
    Row {
        kind: Kind::Timeout,
        ..row("screen_off_after", Group::DevicesPower, "Screen off after", "When you don't touch anything", NONE,
            Method::PowerTimeout(PowerSetting::ScreenOff), Broadcast::None, Applies::Live, Some("ms-settings:powersleep"),
            &["monitor", "display", "timeout", "turn off"], "DESIGN §3.6 v18 (powercfg monitor-timeout = PowerWriteACValueIndex VIDEOIDLE)")
    },
    row("sleep", Group::DevicesPower, "Sleep", "Off = the PC never sleeps by itself · for long downloads", NONE,
        Method::SleepSwitch, Broadcast::None, Applies::Live, Some("ms-settings:powersleep"), &["sleep", "standby", "awake", "downloads"],
        "DESIGN §3.6 v18 + research ideas-v3.md §5 (standby-timeout 0)"),
    Row {
        kind: Kind::Timeout,
        ..row("sleep_after", Group::DevicesPower, "Sleep after", "When you don't touch anything", NONE,
            Method::PowerTimeout(PowerSetting::Sleep), Broadcast::None, Applies::Live, Some("ms-settings:powersleep"),
            &["sleep", "standby", "timeout"], "DESIGN §3.6 v18 (powercfg standby-timeout = PowerWriteACValueIndex STANDBYIDLE)")
    },
    row("fast_startup", Group::DevicesPower, "Fast Startup", "Shut down saves part of Windows for a quicker start", ADMIN,
        Method::FastStartup, Broadcast::None, Applies::NextShutdown, None, &["fast boot", "hiberboot", "shutdown"],
        "DESIGN §3.6 v18 (HKLM Session Manager\\Power HiberbootEnabled)"),
    // ---------------- Sound ----------------
    // Order 045 item 15 (the owner Oct 8: "i mean it can be a setting i guess in tweaks"; menu-v22 TGL `duck`, L4840): Windows'
    // own Sound > Communications choice - 0 mute other sounds, 1 reduce them by 80 % (Windows' default: no value), 2 reduce by
    // 50 %, 3 do nothing. On = any of 0 / 1 / 2 (on writes Windows' default 1), off writes 3.
    row("call_ducking", Group::Sound, "Calls turn other sounds down", "Discord or Teams calls lower your game", NONE,
        reg(&[cu(r"Software\Microsoft\Multimedia\Audio", "UserDuckingPreference", 1, 3, true)]), Broadcast::None,
        Applies::Unconfirmed, None, &["ducking", "communications", "call"], "menu-v22 L4840; Windows Sound panel, Communications tab (UserDuckingPreference)"),
    row("mono_audio", Group::Sound, "Mono audio", "Both ears hear everything · for one earbud", NONE,
        reg(&[cu(r"Software\Microsoft\Multimedia\Audio", "AccessibilityMonoMixState", 1, 0, false)]), Broadcast::None,
        Applies::Unconfirmed, Some("ms-settings:easeofaccess-audio"), &["mono", "earbud", "stereo"], V3),
    row("startup_sound", Group::Sound, "Windows startup sound", "", ADMIN_RESTART,
        reg(&[dw(Hive::Hklm, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\LogonUI\BootAnimation", "DisableStartupSound", 0, 1, true)]),
        Broadcast::None, Applies::Restart, None, &["boot sound", "startup sound", "chime"], V3),
    // ---------------- Privacy & ads ----------------
    Row {
        toast_on: Some("Apps can use your microphone again"),
        toast_off: Some("No app can use your microphone now · Discord too"),
        ..row("microphone_access", Group::PrivacyAds, "Microphone access", "Off = no app can use the microphone", NONE,
            reg(&[sz(Hive::Hkcu, CAM_MIC, "Value", "Allow", "Deny", true), sz(Hive::Hkcu, CAM_MIC_NP, "Value", "Allow", "Deny", true)]),
            Broadcast::None, Applies::Live, Some("ms-settings:privacy-microphone"), &["mic", "privacy", "consent"], V3)
    },
    Row {
        toast_on: Some("Apps can use your camera again"),
        toast_off: Some("No app can use your camera now"),
        ..row("camera_access", Group::PrivacyAds, "Camera access", "Off = no app can use the camera", NONE,
            reg(&[sz(Hive::Hkcu, CAM_CAM, "Value", "Allow", "Deny", true), sz(Hive::Hkcu, CAM_CAM_NP, "Value", "Allow", "Deny", true)]),
            Broadcast::None, Applies::Live, Some("ms-settings:privacy-webcam"), &["webcam", "camera", "privacy"], V3)
    },
    row("web_results_in_search", Group::PrivacyAds, "Web results in Start search", "Bing results when you search", ADMIN_EXPLORER,
        reg(&[policy(Hive::Hkcu, r"Software\Policies\Microsoft\Windows\Explorer", "DisableSearchBoxSuggestions", 1)]),
        Broadcast::Area("Policy"), Applies::Explorer, None, &["bing", "web search", "start search"], V2),
    row("widgets", Group::PrivacyAds, "Widgets", "", ADMIN_EXPLORER,
        reg(&[policy(Hive::Hklm, r"SOFTWARE\Policies\Microsoft\Dsh", "AllowNewsAndInterests", 0)]),
        Broadcast::Area("Policy"), Applies::Explorer, Some("ms-settings:taskbar"), &["news", "weather", "widgets board"],
        "research ideas-v2.md §8 (TaskbarDa is blocked by UCPD; the Dsh policy works)"),
    row("copilot", Group::PrivacyAds, "Copilot", "", NONE, Method::Copilot, Broadcast::None, Applies::Live, None,
        &["ai", "copilot"], "DESIGN §3.6 v18 (guess) + Q_005_02 pick: uninstall/reinstall the Store app (PackageManager)"),
    row("lock_screen_tips", Group::PrivacyAds, "Lock-screen tips", "Fun facts and ads on the lock screen", NONE,
        reg(&[cu(CDM, "RotatingLockScreenOverlayEnabled", 1, 0, true), cu(CDM, "SubscribedContent-338387Enabled", 1, 0, true)]),
        Broadcast::None, Applies::Unconfirmed, Some("ms-settings:lockscreen"), &["spotlight", "fun facts", "ads", "lock screen"], V2),
    row("ads_in_settings", Group::PrivacyAds, "Ads in Settings", "Suggested content in the Settings app", NONE,
        reg(&[
            cu(CDM, "SubscribedContent-338393Enabled", 1, 0, true),
            cu(CDM, "SubscribedContent-353694Enabled", 1, 0, true),
            cu(CDM, "SubscribedContent-353696Enabled", 1, 0, true),
        ]),
        Broadcast::None, Applies::Unconfirmed, Some("ms-settings:privacy-general"), &["suggested content", "ads", "settings app"], V2),
    row("tips_notifications", Group::PrivacyAds, "Tips notifications", "“Tips and suggestions” pop-ups", NONE,
        reg(&[cu(CDM, "SubscribedContent-338389Enabled", 1, 0, true)]), Broadcast::None, Applies::Unconfirmed,
        Some("ms-settings:notifications"), &["tips", "suggestions", "notifications", "ads"], V2),
    row("advertising_id", Group::PrivacyAds, "Advertising ID", "Lets apps show you personal ads", NONE,
        reg(&[cu(r"Software\Microsoft\Windows\CurrentVersion\AdvertisingInfo", "Enabled", 1, 0, true)]), Broadcast::None,
        Applies::Unconfirmed, Some("ms-settings:privacy-general"), &["ads", "tracking", "personalised ads"], V2),
    // ---------------- Look ----------------
    row("transparency", Group::Look, "Transparency", "See-through taskbar and windows", NONE,
        reg(&[cu(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "EnableTransparency", 1, 0, true)]),
        Broadcast::Area("ImmersiveColorSet"), Applies::Live, Some("ms-settings:colors"), &["glass", "acrylic", "mica", "see through"], V2),
    row("dark_mode", Group::Look, "Dark mode", "Apps and taskbar", NONE,
        reg(&[
            cu(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "AppsUseLightTheme", 0, 1, false),
            cu(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "SystemUsesLightTheme", 0, 1, false),
        ]),
        Broadcast::Area("ImmersiveColorSet"), Applies::Live, Some("ms-settings:colors"), &["theme", "light", "dark", "black"], V2),
    row("animations", Group::Look, "Animations", "Off = snappier windows", NONE, Method::Animations, Broadcast::None, Applies::Live,
        Some("ms-settings:easeofaccess-visualeffects"), &["animation", "effects", "snappy", "motion"], V2),
];

/// The row with this id.
pub fn find(id: &str) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.id == id)
}

/// The layout-hotkey values written by "Stop Alt + Shift switching": on = all "3" (not assigned).
pub(crate) const LANG_VALUES: [(&str, &str); 3] = [("Hotkey", "1"), ("Language Hotkey", "1"), ("Layout Hotkey", "2")];
pub(crate) const LANG_PATH: &str = LANG_TOGGLE;
