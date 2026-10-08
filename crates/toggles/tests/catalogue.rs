//! The row list matches DESIGN.md §3.6 (every row of its table, incl. v18), minus what must not be there.

use bu_toggles::model::{Badge, Group, Kind};
use bu_toggles::rows::{find, ROWS};
use bu_toggles::search;

/// (id, group, title) in page order — DESIGN §3.6 table, without "Calls turn other sounds down" (A_005_01: left out).
const EXPECTED: &[(&str, Group, &str)] = &[
    ("show_file_extensions", Group::FilesExplorer, "Show file extensions"),
    ("show_hidden_files", Group::FilesExplorer, "Show hidden files"),
    ("classic_context_menu", Group::FilesExplorer, "Classic right-click menu"),
    ("explorer_opens_this_pc", Group::FilesExplorer, "Explorer opens on This PC"),
    ("copy_window_details", Group::FilesExplorer, "Copy window shows details"),
    ("onedrive_ads_explorer", Group::FilesExplorer, "OneDrive ads in File Explorer"),
    ("end_task", Group::TaskbarStart, "End task on right-click"),
    ("clock_seconds", Group::TaskbarStart, "Seconds on the clock"),
    ("start_on_left", Group::TaskbarStart, "Start on the left"),
    ("task_view_button", Group::TaskbarStart, "Task View button"),
    ("start_recommendations", Group::TaskbarStart, "Recommendations in Start"),
    ("taskbar_flashing", Group::TaskbarStart, "Taskbar flashing"),
    ("alt_tab_edge_tabs", Group::TaskbarStart, "Alt + Tab shows Edge tabs"),
    ("start_recommended_section", Group::TaskbarStart, "“Recommended” section in Start"),
    ("game_mode", Group::Gaming, "Game Mode"),
    ("xbox_background_recording", Group::Gaming, "Xbox background recording"),
    ("xbox_button_game_bar", Group::Gaming, "Xbox button opens Game Bar"),
    ("windowed_game_optimizations", Group::Gaming, "Optimizations for windowed games"),
    ("auto_hdr", Group::Gaming, "Auto HDR"),
    ("variable_refresh_rate", Group::Gaming, "Variable refresh rate"),
    ("gpu_scheduling", Group::Gaming, "Hardware-accelerated GPU scheduling"),
    ("fullscreen_optimizations_off", Group::Gaming, "Fullscreen optimizations off"),
    ("stop_layout_hotkeys", Group::Input, "Stop Alt + Shift switching your keyboard layout"),
    ("sticky_keys_popup", Group::Input, "Sticky Keys pop-up"),
    ("filter_keys_popup", Group::Input, "Filter Keys pop-up"),
    ("toggle_keys_popup", Group::Input, "Toggle Keys pop-up"),
    ("clipboard_history", Group::Input, "Clipboard history"),
    ("scroll_inactive_windows", Group::Input, "Scroll inactive windows"),
    ("print_screen_snipping", Group::Input, "Print Screen opens Snipping Tool"),
    ("bluetooth", Group::DevicesPower, "Bluetooth"),
    ("usb_power_saving", Group::DevicesPower, "USB power saving"),
    ("screen_off_after", Group::DevicesPower, "Screen off after"),
    ("sleep", Group::DevicesPower, "Sleep"),
    ("sleep_after", Group::DevicesPower, "Sleep after"),
    ("fast_startup", Group::DevicesPower, "Fast Startup"),
    ("call_ducking", Group::Sound, "Calls turn other sounds down"),
    ("mono_audio", Group::Sound, "Mono audio"),
    ("startup_sound", Group::Sound, "Windows startup sound"),
    ("microphone_access", Group::PrivacyAds, "Microphone access"),
    ("camera_access", Group::PrivacyAds, "Camera access"),
    ("web_results_in_search", Group::PrivacyAds, "Web results in Start search"),
    ("widgets", Group::PrivacyAds, "Widgets"),
    ("copilot", Group::PrivacyAds, "Copilot"),
    ("lock_screen_tips", Group::PrivacyAds, "Lock-screen tips"),
    ("ads_in_settings", Group::PrivacyAds, "Ads in Settings"),
    ("tips_notifications", Group::PrivacyAds, "Tips notifications"),
    ("advertising_id", Group::PrivacyAds, "Advertising ID"),
    ("transparency", Group::Look, "Transparency"),
    ("dark_mode", Group::Look, "Dark mode"),
    ("animations", Group::Look, "Animations"),
];

#[test]
fn every_design_row_is_there_in_page_order() {
    let got: Vec<(&str, Group, &str)> = ROWS.iter().map(|r| (r.id, r.group, r.title)).collect();
    assert_eq!(got, EXPECTED);
    // (Order 045: + "Calls turn other sounds down")
    assert_eq!(ROWS.len(), 50);
}

#[test]
fn groups_follow_design_order() {
    let mut seen: Vec<Group> = Vec::new();
    for r in ROWS {
        if seen.last() != Some(&r.group) {
            assert!(!seen.contains(&r.group), "group {:?} split", r.group);
            seen.push(r.group);
        }
    }
    assert_eq!(seen, Group::ALL.to_vec());
    assert_eq!(Group::ALL.map(|g| g.title()), [
        "Files & Explorer", "Taskbar & Start", "Gaming", "Input", "Devices & power", "Sound", "Privacy & ads", "Look"
    ]);
}

#[test]
fn ids_unique_and_rows_complete() {
    for (i, r) in ROWS.iter().enumerate() {
        assert!(ROWS.iter().skip(i + 1).all(|o| o.id != r.id), "duplicate id {}", r.id);
        assert!(!r.title.is_empty() && !r.source.is_empty(), "{}", r.id);
        assert_eq!(find(r.id).map(|f| f.id), Some(r.id));
    }
    assert!(find("nope").is_none());
}

#[test]
fn never_rows_are_absent() {
    for r in ROWS {
        let t = format!("{} {} {}", r.id, r.title, r.sub).to_lowercase();
        assert!(!t.contains("memory integrity") && !t.contains("hvci"), "NEVER row: {}", r.id);

        assert!(!t.contains("dns") && !t.contains("wi-fi"), "network rows belong to the Network tab: {}", r.id);
    }
}

#[test]
fn admin_and_explorer_badges_match_design() {
    let admin: Vec<&str> = ROWS.iter().filter(|r| r.needs_admin()).map(|r| r.id).collect();
    assert_eq!(admin, [
        "start_recommended_section", "gpu_scheduling", "usb_power_saving", "fast_startup", "startup_sound",
        "web_results_in_search", "widgets",
    ]);
    let explorer: Vec<&str> = ROWS.iter().filter(|r| r.restarts_explorer()).map(|r| r.id).collect();
    assert_eq!(explorer, [
        "classic_context_menu", "start_recommendations", "start_recommended_section", "web_results_in_search", "widgets",
    ]);
    // Order 043: these two refresh the shell instead (no restart)
    let refresh: Vec<&str> = ROWS.iter().filter(|r| r.refresh_shell).map(|r| r.id).collect();
    assert_eq!(refresh, ["show_file_extensions", "show_hidden_files"]);
    let badge = |id: &str| find(id).unwrap().badges.to_vec();
    assert_eq!(badge("xbox_background_recording"), [Badge::Restart]);
    assert_eq!(badge("gpu_scheduling"), [Badge::Admin, Badge::Restart]);
    assert_eq!(badge("startup_sound"), [Badge::Admin, Badge::Restart]);
    assert_eq!(badge("scroll_inactive_windows"), [Badge::SignOut]);
    for id in ["windowed_game_optimizations", "auto_hdr", "variable_refresh_rate"] {
        assert_eq!(badge(id), [Badge::NextGame]);
    }
    // Copilot: no admin (A_005_02 — PackageManager per-user removal)
    assert!(!find("copilot").unwrap().needs_admin());
}

#[test]
fn kinds() {
    assert_eq!(find("screen_off_after").unwrap().kind, Kind::Timeout);
    assert_eq!(find("sleep_after").unwrap().kind, Kind::Timeout);
    assert_eq!(find("fullscreen_optimizations_off").unwrap().kind, Kind::GameList);
    assert_eq!(ROWS.iter().filter(|r| r.kind == Kind::Switch).count(), 47);
}

#[test]
fn badge_tips_are_design_words() {
    assert_eq!(Badge::Admin.tip(), "Needs admin — Windows asks once");
    assert_eq!(Badge::Explorer.tip(), "Restarts Explorer — the taskbar blinks once");
    assert_eq!(Badge::SignOut.tip(), "Takes effect after you sign out");
    assert_eq!(Badge::Restart.tip(), "Takes effect after a restart");
    assert_eq!(Badge::NextGame.tip(), "Takes effect the next time a game starts");
    assert_eq!(Badge::Explorer.toast(), Some("Explorer restarts — the taskbar blinks once"));
    assert_eq!(Badge::Admin.toast(), None);
}

#[test]
fn search_matches_every_word_against_title_sub_keywords_group() {
    assert_eq!(search::search("edge"), ["alt_tab_edge_tabs"]);
    // "taskbar" hits the group name, "left" the title
    assert!(search::search("taskbar left").contains(&"start_on_left"));
    // sub-line
    assert_eq!(search::search("photo.jpg"), ["show_file_extensions"]);
    // hidden keyword, any case
    assert_eq!(search::search("HAGS"), ["gpu_scheduling"]);
    // every word must match
    assert!(search::search("edge zzz").is_empty());
    assert_eq!(search::search("").len(), ROWS.len());
    assert_eq!(search::search("   ").len(), ROWS.len());
    let r = find("sticky_keys_popup").unwrap();
    assert_eq!(search::title_hits(r, "key sticky"), vec![(0, 6), (7, 10)]);
    assert_eq!(search::title_hits(r, "zz"), vec![]);
}
