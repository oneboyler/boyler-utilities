//! The Tweaks rows in the drawing's order with the drawing's words (menu-v22 `TGL`: id, `t`, `s`, `k`, sample `on` / `v`),
//! each tied to its bu-toggles row (`crate_id`: how it is read and changed, its badges). Generated from menu-v22.html
//! (Order 021); "Calls turn other sounds down" (`duck`) since Order 045 (the owner said yes, Oct 8).

use super::svc::Sample;

pub struct R {
    pub id: &'static str,
    pub crate_id: &'static str,
    /// index into GROUPS
    pub group: usize,
    pub title: &'static str,
    pub sub: &'static str,
    /// extra search words
    pub k: &'static str,
    pub sample: Sample,
}

/// The switch groups (`TGL[].g`); "Quick fixes" and "Default apps" follow them on the page.
pub const GROUPS: [&str; 8] = ["Files & Explorer", "Taskbar & Start", "Gaming", "Input", "Devices & power", "Sound", "Privacy & ads", "Look"];

pub static ROWS: &[R] = &[
    R { id: "ext", crate_id: "show_file_extensions", group: 0, title: "Show file extensions", sub: "photo.jpg instead of photo", k: "extension type name", sample: Sample::On(true) },
    R { id: "hid", crate_id: "show_hidden_files", group: 0, title: "Show hidden files", sub: "Files and folders Windows hides", k: "hidden folders", sample: Sample::On(false) },
    R { id: "ctx", crate_id: "classic_context_menu", group: 0, title: "Classic right-click menu", sub: "The full menu, no “Show more options”", k: "context menu windows 10 old", sample: Sample::On(false) },
    R { id: "thispc", crate_id: "explorer_opens_this_pc", group: 0, title: "Explorer opens on This PC", sub: "Instead of Home", k: "file explorer drives start", sample: Sample::On(false) },
    R { id: "copy", crate_id: "copy_window_details", group: 0, title: "Copy window shows details", sub: "The speed graph when copying files", k: "copy move transfer speed", sample: Sample::On(false) },
    R { id: "odads", crate_id: "onedrive_ads_explorer", group: 0, title: "OneDrive ads in File Explorer", sub: "“Try Microsoft 365” bars at the top", k: "onedrive sync provider notifications ads explorer", sample: Sample::On(true) },
    R { id: "endtask", crate_id: "end_task", group: 1, title: "End task on right-click", sub: "Close a stuck app from the taskbar", k: "kill frozen not responding task manager", sample: Sample::On(false) },
    R { id: "secs", crate_id: "clock_seconds", group: 1, title: "Seconds on the clock", sub: "", k: "time clock", sample: Sample::On(false) },
    R { id: "left", crate_id: "start_on_left", group: 1, title: "Start on the left", sub: "Taskbar icons on the left, like Windows 10", k: "align centre center", sample: Sample::On(false) },
    R { id: "tview", crate_id: "task_view_button", group: 1, title: "Task View button", sub: "", k: "desktops windows", sample: Sample::On(true) },
    R { id: "recs", crate_id: "start_recommendations", group: 1, title: "Recommendations in Start", sub: "Tips and new-app ads", k: "ads suggestions start menu", sample: Sample::On(true) },
    R { id: "recsec", crate_id: "start_recommended_section", group: 1, title: "“Recommended” section in Start", sub: "Hides the whole section · recent files and new apps", k: "recommended start menu section recent files hide", sample: Sample::On(true) },
    R { id: "flash", crate_id: "taskbar_flashing", group: 1, title: "Taskbar flashing", sub: "Apps blink orange for attention", k: "flash blink orange attention distracting game", sample: Sample::On(true) },
    R { id: "edgetab", crate_id: "alt_tab_edge_tabs", group: 1, title: "Alt + Tab shows Edge tabs", sub: "Off = only real windows", k: "alt tab edge browser tabs multitasking switch", sample: Sample::On(true) },
    R { id: "gmode", crate_id: "game_mode", group: 2, title: "Game Mode", sub: "Windows puts the game first", k: "fps performance", sample: Sample::On(true) },
    R { id: "dvr", crate_id: "xbox_background_recording", group: 2, title: "Xbox background recording", sub: "Always records gameplay · costs FPS", k: "game bar dvr capture clips fps", sample: Sample::On(true) },
    R { id: "xbtn", crate_id: "xbox_button_game_bar", group: 2, title: "Xbox button opens Game Bar", sub: "On a controller", k: "controller pad guide", sample: Sample::On(true) },
    R { id: "winopt", crate_id: "windowed_game_optimizations", group: 2, title: "Optimizations for windowed games", sub: "Lower latency in windowed and borderless games", k: "windowed borderless latency flip model swap effect directx fps", sample: Sample::On(false) },
    R { id: "ahdr", crate_id: "auto_hdr", group: 2, title: "Auto HDR", sub: "HDR colours in older games · HDR screens only", k: "hdr colour color brightness directx", sample: Sample::On(false) },
    R { id: "vrr", crate_id: "variable_refresh_rate", group: 2, title: "Variable refresh rate", sub: "Smoother DirectX 11 games · G-Sync / FreeSync screens", k: "vrr gsync freesync adaptive sync tearing hz", sample: Sample::On(true) },
    R { id: "hags", crate_id: "gpu_scheduling", group: 2, title: "Hardware-accelerated GPU scheduling", sub: "The graphics card plans its own work · can lower latency", k: "hags gpu scheduling latency graphics nvidia", sample: Sample::On(true) },
    R { id: "fso", crate_id: "fullscreen_optimizations_off", group: 2, title: "Fullscreen optimizations off · per game", sub: "For games that stutter or feel laggy in fullscreen", k: "fullscreen optimizations fso exe compatibility game per game", sample: Sample::Games },
    R { id: "altsh", crate_id: "stop_layout_hotkeys", group: 3, title: "Stop Alt + Shift switching your keyboard layout", sub: "Ctrl + Shift too · your layouts stay, only the shortcut goes", k: "alt shift ctrl keyboard layout language switch input hotkey croatian english", sample: Sample::On(false) },
    R { id: "sticky", crate_id: "sticky_keys_popup", group: 3, title: "Sticky Keys pop-up", sub: "Shift pressed 5 times", k: "shift accessibility keyboard", sample: Sample::On(true) },
    R { id: "filter", crate_id: "filter_keys_popup", group: 3, title: "Filter Keys pop-up", sub: "Right Shift held for 8 seconds", k: "shift accessibility keyboard", sample: Sample::On(true) },
    R { id: "togk", crate_id: "toggle_keys_popup", group: 3, title: "Toggle Keys pop-up", sub: "Num Lock held for 5 seconds", k: "num lock caps accessibility keyboard beep", sample: Sample::On(true) },
    R { id: "clip", crate_id: "clipboard_history", group: 3, title: "Clipboard history", sub: "Win + V shows what you copied", k: "copy paste", sample: Sample::On(false) },
    R { id: "scroll", crate_id: "scroll_inactive_windows", group: 3, title: "Scroll inactive windows", sub: "Scroll whatever is under the mouse", k: "mouse wheel", sample: Sample::On(true) },
    R { id: "prtsnip", crate_id: "print_screen_snipping", group: 3, title: "Print Screen opens Snipping Tool", sub: "Off = your Screenshot key gets Print Screen", k: "prtsc print screen snipping tool screenshot key capture", sample: Sample::On(false) },
    R { id: "bt", crate_id: "bluetooth", group: 4, title: "Bluetooth", sub: "Off drops wireless headsets and controllers", k: "bluetooth wireless radio headset controller pad", sample: Sample::On(true) },
    R { id: "usbss", crate_id: "usb_power_saving", group: 4, title: "USB power saving", sub: "Off stops mice and keyboards dropping out", k: "usb selective suspend mouse keyboard headset disconnect drop out power", sample: Sample::On(true) },
    R { id: "scroff", crate_id: "screen_off_after", group: 4, title: "Screen off after", sub: "When you don’t touch anything", k: "screen display monitor off timeout power", sample: Sample::Time(600) },
    R { id: "sleep", crate_id: "sleep", group: 4, title: "Sleep", sub: "Off = the PC never sleeps by itself · for long downloads", k: "sleep standby afk download power never", sample: Sample::On(true) },
    R { id: "sleepafter", crate_id: "sleep_after", group: 4, title: "Sleep after", sub: "When you don’t touch anything", k: "sleep standby timeout power", sample: Sample::Time(1800) },
    R { id: "fast", crate_id: "fast_startup", group: 4, title: "Fast Startup", sub: "Shut down saves part of Windows for a quicker start", k: "fast startup boot shutdown hiberboot", sample: Sample::On(true) },
    R { id: "duck", crate_id: "call_ducking", group: 5, title: "Calls turn other sounds down", sub: "Discord or Teams calls lower your game", k: "ducking communications volume discord teams call game quiet audio", sample: Sample::On(true) },
    R { id: "mono", crate_id: "mono_audio", group: 5, title: "Mono audio", sub: "Both ears hear everything · for one earbud", k: "mono stereo earbud accessibility audio", sample: Sample::On(false) },
    R { id: "bootsnd", crate_id: "startup_sound", group: 5, title: "Windows startup sound", sub: "", k: "boot startup chime sound audio", sample: Sample::On(true) },
    R { id: "micacc", crate_id: "microphone_access", group: 6, title: "Microphone access", sub: "Off = no app can use the microphone", k: "microphone mic privacy kill switch permission", sample: Sample::On(true) },
    R { id: "camacc", crate_id: "camera_access", group: 6, title: "Camera access", sub: "Off = no app can use the camera", k: "camera webcam privacy kill switch permission", sample: Sample::On(true) },
    R { id: "bing", crate_id: "web_results_in_search", group: 6, title: "Web results in Start search", sub: "Bing results when you search", k: "bing internet ads", sample: Sample::On(true) },
    R { id: "widgets", crate_id: "widgets", group: 6, title: "Widgets", sub: "The news and weather board on the taskbar", k: "widgets news weather msn board taskbar ads", sample: Sample::On(true) },
    R { id: "copilot", crate_id: "copilot", group: 6, title: "Copilot", sub: "The Copilot app and the Copilot key", k: "copilot ai assistant chat microsoft key", sample: Sample::On(true) },
    R { id: "lock", crate_id: "lock_screen_tips", group: 6, title: "Lock-screen tips", sub: "Fun facts and ads on the lock screen", k: "spotlight ads", sample: Sample::On(true) },
    R { id: "setads", crate_id: "ads_in_settings", group: 6, title: "Ads in Settings", sub: "Suggested content in the Settings app", k: "suggestions", sample: Sample::On(true) },
    R { id: "tips", crate_id: "tips_notifications", group: 6, title: "Tips notifications", sub: "“Tips and suggestions” pop-ups", k: "notification ads", sample: Sample::On(true) },
    R { id: "adid", crate_id: "advertising_id", group: 6, title: "Advertising ID", sub: "Lets apps show you personal ads", k: "tracking privacy", sample: Sample::On(true) },
    R { id: "transp", crate_id: "transparency", group: 7, title: "Transparency", sub: "See-through taskbar and windows", k: "glass blur acrylic", sample: Sample::On(true) },
    R { id: "dark", crate_id: "dark_mode", group: 7, title: "Dark mode", sub: "Apps and taskbar", k: "theme light black", sample: Sample::On(true) },
    R { id: "anim", crate_id: "animations", group: 7, title: "Animations", sub: "Off = snappier windows", k: "motion effects speed", sample: Sample::On(true) },
];
