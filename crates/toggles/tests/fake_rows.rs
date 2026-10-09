//! Every behaviour against the FAKE OS layer: read, apply (+ read-back), after-steps, undo, the admin path, errors.

use bu_toggles::defaults::ChangeAction;
use bu_toggles::fake::FakeOs;
use bu_toggles::model::{Kind, Timeout, Value};
use bu_toggles::os::{Hive, PowerSetting, PowerValues, RegValue, SpiItem};
use bu_toggles::rows::{self, Method, ROWS};
use bu_toggles::{Error, Toggles};

const ADV: &str = rows::ADV;

fn admin() -> Toggles<FakeOs> {
    Toggles::new(FakeOs { elevated: true, ..FakeOs::default() })
}

fn user() -> Toggles<FakeOs> {
    Toggles::new(FakeOs::default())
}

fn sw(t: &Toggles<FakeOs>, id: &str) -> bool {
    match t.read(id).unwrap().value {
        Value::Switch(b) => b,
        v => panic!("{id}: not a switch: {v:?}"),
    }
}

/// The fake's whole observable state minus the log (empty registry keys don't count as values).
fn raw(os: &FakeOs) -> String {
    let reg: Vec<_> = os.reg.iter().filter(|(_, v)| !v.is_empty()).collect();
    let mut spi: Vec<_> = os.spi.iter().collect();
    spi.sort_by_key(|(k, _)| format!("{k:?}"));
    let mut power: Vec<_> = os.power.iter().collect();
    power.sort_by_key(|(k, _)| format!("{k:?}"));
    format!("{reg:?}|{spi:?}|{power:?}|{}|{:?}|{}", os.hibernate_on, os.bluetooth, os.copilot)
}

// ---------------------------------------------------------------- every switch row, generically

#[test]
fn every_switch_row_reads_flips_reads_back_and_undoes_exactly() {
    for row in ROWS.iter().filter(|r| r.kind == Kind::Switch) {
        let mut t = admin();
        let before = sw(&t, row.id);
        let raw_before = raw(t.os());
        let a = t.set(row.id, !before).unwrap_or_else(|e| panic!("{}: {e}", row.id));
        if row.method == Method::Copilot && before {
            // off = uninstall
            assert_eq!(a.value, Value::Switch(false));
        } else if row.method == Method::Copilot {
            unreachable!("fake starts with Copilot installed");
        } else {
            assert_eq!(a.value, Value::Switch(!before), "{}", row.id);
        }
        assert_eq!(sw(&t, row.id), !before, "{} read-back", row.id);
        assert_eq!(a.explorer_restarted, row.restarts_explorer(), "{}", row.id);
        assert_eq!(t.os().count("restart_explorer"), row.restarts_explorer() as usize, "{}", row.id);
        assert!(t.can_undo(row.id));

        let u = t.undo(row.id).unwrap_or_else(|e| panic!("{} undo: {e}", row.id));
        if row.method == Method::Copilot {
            // Windows can't reinstall silently: undo hands back the Store page
            assert_eq!(u.open, Some(ChangeAction::OpenUri(bu_toggles::service::COPILOT_STORE_URI.into())));
            continue;
        }
        assert_eq!(sw(&t, row.id), before, "{} after undo", row.id);
        assert_eq!(raw(t.os()), raw_before, "{}: undo must put back the exact old values", row.id);
        // undo again = redo
        t.undo(row.id).unwrap();
        assert_eq!(sw(&t, row.id), !before, "{} redo", row.id);
    }
}

#[test]
fn admin_rows_refuse_without_admin_and_change_nothing() {
    for row in ROWS.iter().filter(|r| r.kind == Kind::Switch && r.needs_admin()) {
        let mut t = user();
        let before = sw(&t, row.id);
        let raw_before = raw(t.os());
        let e = t.set(row.id, !before).unwrap_err();
        assert_eq!(e, Error::NeedsAdmin { row: row.id.into() }, "{}", row.id);
        assert_eq!(raw(t.os()), raw_before, "{}", row.id);
        assert!(t.os().log.is_empty(), "{}: {:?}", row.id, t.os().log);
        assert!(t.needs_admin(row.id).unwrap());
        assert!(!t.can_undo(row.id));
    }
}

#[test]
fn non_admin_rows_work_without_admin() {
    for row in ROWS.iter().filter(|r| r.kind == Kind::Switch && !r.needs_admin()) {
        let mut t = user();
        let before = sw(&t, row.id);
        t.set(row.id, !before).unwrap_or_else(|e| panic!("{}: {e}", row.id));
    }
}

#[test]
fn undo_of_an_admin_row_without_admin_is_refused() {
    // an admin row changed while elevated, then undone without admin -> NeedsAdmin with the row id, nothing changed
    let mut a = admin();
    a.set("widgets", false).unwrap();
    a.os_mut().elevated = false;
    assert_eq!(a.undo("widgets").unwrap_err(), Error::NeedsAdmin { row: "widgets".into() });
    assert!(!sw(&a, "widgets"));
}

#[test]
fn setting_the_current_state_writes_nothing() {
    let mut t = admin();
    let a = t.set("show_file_extensions", false).unwrap(); // default: hidden extensions = off
    assert_eq!(a.value, Value::Switch(false));
    assert!(!a.explorer_restarted);
    assert!(t.os().log.is_empty());
    assert!(t.os().reg.is_empty());
    assert!(!t.can_undo("show_file_extensions"));
}

// ---------------------------------------------------------------- registry rows

#[test]
fn file_extensions_explorer_row() {
    let mut t = user();
    assert!(!sw(&t, "show_file_extensions"));
    let a = t.set("show_file_extensions", true).unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, ADV, "HideFileExt"), Some(RegValue::Dword(0)));
    // Order 043: the shell's refresh, never an Explorer restart (it blacked out the screen for ~30 s)
    assert!(!a.explorer_restarted);
    assert_eq!(a.toast, None);
    assert_eq!(t.os().log, ["refresh_shell"]);
    t.undo("show_file_extensions").unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, ADV, "HideFileExt"), None, "undo deletes a value that wasn't there");
    assert_eq!(t.os().count("refresh_shell"), 2, "undo refreshes too");
    assert_eq!(t.os().count("restart_explorer"), 0);
}

#[test]
fn hidden_files_uses_1_and_2() {
    let mut t = user();
    t.set("show_hidden_files", true).unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, ADV, "Hidden"), Some(RegValue::Dword(1)));
    t.set("show_hidden_files", false).unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, ADV, "Hidden"), Some(RegValue::Dword(2)));
    assert_eq!(t.os().count("refresh_shell"), 2);
    assert_eq!(t.os().count("restart_explorer"), 0);
}

#[test]
fn classic_context_menu_creates_and_deletes_the_key() {
    let mut t = user();
    assert!(!sw(&t, "classic_context_menu"));
    t.set("classic_context_menu", true).unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, rows::CLASSIC_MENU_SUBKEY, ""), Some(RegValue::Sz(String::new())));
    assert!(sw(&t, "classic_context_menu"));
    t.set("classic_context_menu", false).unwrap();
    assert!(!t.os().reg.keys().any(|(_, k)| k.contains("86ca1aa0")), "whole CLSID key removed");
    assert_eq!(t.os().count("restart_explorer"), 2);
}

#[test]
fn explorer_opens_this_pc_other_values_are_off() {
    let mut t = user();
    t.os_mut().put(Hive::Hkcu, ADV, "LaunchTo", RegValue::Dword(3)); // Downloads
    assert!(!sw(&t, "explorer_opens_this_pc"));
    t.set("explorer_opens_this_pc", true).unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, ADV, "LaunchTo"), Some(RegValue::Dword(1)));
    t.undo("explorer_opens_this_pc").unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, ADV, "LaunchTo"), Some(RegValue::Dword(3)));
}

#[test]
fn alt_tab_edge_tabs_other_values_count_as_on_and_undo_restores_them() {
    let mut t = user();
    t.os_mut().put(Hive::Hkcu, ADV, "MultiTaskingAltTabFilter", RegValue::Dword(1)); // 5 recent tabs
    assert!(sw(&t, "alt_tab_edge_tabs"));
    t.set("alt_tab_edge_tabs", false).unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, ADV, "MultiTaskingAltTabFilter"), Some(RegValue::Dword(3)));
    t.undo("alt_tab_edge_tabs").unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, ADV, "MultiTaskingAltTabFilter"), Some(RegValue::Dword(1)));
}

#[test]
fn start_on_left_and_seconds() {
    let mut t = user();
    t.set("start_on_left", true).unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, ADV, "TaskbarAl"), Some(RegValue::Dword(0)));
    t.set("clock_seconds", true).unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, ADV, "ShowSecondsInSystemClock"), Some(RegValue::Dword(1)));
    assert!(t.os().log.contains(&"broadcast:TraySettings".to_string()));
}

#[test]
fn policies_write_off_and_delete_for_on() {
    let mut t = admin();
    assert!(sw(&t, "widgets"));
    let a = t.set("widgets", false).unwrap();
    assert_eq!(t.os().get(Hive::Hklm, r"SOFTWARE\Policies\Microsoft\Dsh", "AllowNewsAndInterests"), Some(RegValue::Dword(0)));
    assert!(a.explorer_restarted);
    assert!(t.os().log.contains(&"broadcast:Policy".to_string()));
    t.set("widgets", true).unwrap();
    assert_eq!(t.os().get(Hive::Hklm, r"SOFTWARE\Policies\Microsoft\Dsh", "AllowNewsAndInterests"), None);

    t.set("web_results_in_search", false).unwrap();
    assert_eq!(
        t.os().get(Hive::Hkcu, r"Software\Policies\Microsoft\Windows\Explorer", "DisableSearchBoxSuggestions"),
        Some(RegValue::Dword(1))
    );
    t.set("start_recommended_section", false).unwrap();
    assert_eq!(
        t.os().get(Hive::Hklm, r"SOFTWARE\Policies\Microsoft\Windows\Explorer", "HideRecommendedSection"),
        Some(RegValue::Dword(1))
    );
}

#[test]
fn hags_is_hklm_2_on_1_off_restart_toast() {
    let mut t = admin();
    assert!(!sw(&t, "gpu_scheduling"));
    let a = t.set("gpu_scheduling", true).unwrap();
    assert_eq!(t.os().get(Hive::Hklm, r"SYSTEM\CurrentControlSet\Control\GraphicsDrivers", "HwSchMode"), Some(RegValue::Dword(2)));
    assert_eq!(a.toast.as_deref(), Some("Takes effect after a restart"));
    t.set("gpu_scheduling", false).unwrap();
    assert_eq!(t.os().get(Hive::Hklm, r"SYSTEM\CurrentControlSet\Control\GraphicsDrivers", "HwSchMode"), Some(RegValue::Dword(1)));
}

#[test]
fn hags_not_set_follows_the_driver_default_and_unsupported_greys_it() {
    use bu_toggles::os::GpuScheduling;
    let mut t = admin();
    // a real PC (measured): no HwSchMode, running = on, driver default bit = off -> the switch shows ON
    t.os_mut().gpu = GpuScheduling { supported: true, enabled_now: true, enabled_by_default: false };
    assert!(sw(&t, "gpu_scheduling"), "HwSchMode missing -> what is running now (on)");
    t.set("gpu_scheduling", false).unwrap();
    assert_eq!(t.os().get(Hive::Hklm, rows::HAGS_PATH, rows::HAGS_VALUE), Some(RegValue::Dword(1)));
    assert!(!sw(&t, "gpu_scheduling"), "the set value wins (Settings shows it; takes effect after a restart)");
    t.undo("gpu_scheduling").unwrap();
    assert_eq!(t.os().get(Hive::Hklm, rows::HAGS_PATH, rows::HAGS_VALUE), None);

    t.os_mut().gpu = GpuScheduling::default();
    let st = t.read("gpu_scheduling").unwrap();
    assert!(!st.enabled);
    assert_eq!(st.disabled_reason, Some("Your graphics card doesn't support it"));
    assert_eq!(st.value, Value::Switch(false));
    assert!(matches!(t.set("gpu_scheduling", true), Err(Error::NotAvailable { .. })));
}

#[test]
fn multi_value_rows_any_value_on_means_on() {
    let mut t = user();
    let cdm = rows::CDM;
    t.set("xbox_background_recording", false).unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, r"System\GameConfigStore", "GameDVR_Enabled"), Some(RegValue::Dword(0)));
    assert_eq!(t.os().get(Hive::Hkcu, r"Software\Microsoft\Windows\CurrentVersion\GameDVR", "AppCaptureEnabled"), Some(RegValue::Dword(0)));

    t.set("ads_in_settings", false).unwrap();
    for n in ["SubscribedContent-338393Enabled", "SubscribedContent-353694Enabled", "SubscribedContent-353696Enabled"] {
        assert_eq!(t.os().get(Hive::Hkcu, cdm, n), Some(RegValue::Dword(0)), "{n}");
    }
}

/// Order 068, real flip run: SubscribedContent-338387Enabled is Settings' Spotlight / Picture choice (1 turned his Picture lock screen
/// into Windows spotlight) - the row must never write it, either way, and undo puts the overlay back exactly.
#[test]
fn lock_screen_tips_only_touches_the_overlay_never_the_spotlight_choice() {
    let mut t = user();
    let cdm = rows::CDM;
    let (overlay, spotlight) = ("RotatingLockScreenOverlayEnabled", "SubscribedContent-338387Enabled");
    // nothing stored = Windows' default (on)
    assert!(sw(&t, "lock_screen_tips"));
    for (pic, stored) in [("Picture", 0), ("Spotlight", 1)] {
        t.os_mut().put(Hive::Hkcu, cdm, spotlight, RegValue::Dword(stored));
        t.os_mut().put(Hive::Hkcu, cdm, overlay, RegValue::Dword(0));
        assert!(!sw(&t, "lock_screen_tips"), "{pic}");
        t.set("lock_screen_tips", true).unwrap();
        assert_eq!(t.os().get(Hive::Hkcu, cdm, overlay), Some(RegValue::Dword(1)), "{pic}");
        assert!(sw(&t, "lock_screen_tips"), "{pic}");
        t.set("lock_screen_tips", false).unwrap();
        assert_eq!(t.os().get(Hive::Hkcu, cdm, overlay), Some(RegValue::Dword(0)), "{pic}");
        assert!(!sw(&t, "lock_screen_tips"), "{pic}");
        t.undo("lock_screen_tips").unwrap();
        assert_eq!(t.os().get(Hive::Hkcu, cdm, overlay), Some(RegValue::Dword(1)), "{pic}");
        // the Spotlight / Picture choice is exactly as it was after every step
        assert_eq!(t.os().get(Hive::Hkcu, cdm, spotlight), Some(RegValue::Dword(stored)), "{pic}");
    }
}

/// Order 068, real flip run: Mono audio's value was written but Settings still showed it OFF - it is badged "after a restart".
#[test]
fn mono_audio_says_it_takes_a_restart() {
    let r = rows::find("mono_audio").unwrap();
    assert_eq!(r.badges, [bu_toggles::model::Badge::Restart]);
    assert_eq!(r.applies, rows::Applies::Restart);
}

#[test]
fn dark_mode_and_transparency_broadcast_immersive_color_set() {
    let mut t = user();
    assert!(!sw(&t, "dark_mode"));
    t.set("dark_mode", true).unwrap();
    let p = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    assert_eq!(t.os().get(Hive::Hkcu, p, "AppsUseLightTheme"), Some(RegValue::Dword(0)));
    assert_eq!(t.os().get(Hive::Hkcu, p, "SystemUsesLightTheme"), Some(RegValue::Dword(0)));
    t.set("transparency", false).unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, p, "EnableTransparency"), Some(RegValue::Dword(0)));
    assert_eq!(t.os().count("broadcast:ImmersiveColorSet"), 2);
}

#[test]
fn microphone_and_camera_write_allow_deny_with_their_toasts() {
    let mut t = user();
    let a = t.set("microphone_access", false).unwrap();
    assert_eq!(a.toast.as_deref(), Some("No app can use your microphone now · Discord too"));
    let mic = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
    assert_eq!(t.os().get(Hive::Hkcu, mic, "Value"), Some(RegValue::Sz("Deny".into())));
    assert_eq!(t.os().get(Hive::Hkcu, &format!(r"{mic}\NonPackaged"), "Value"), Some(RegValue::Sz("Deny".into())));
    let a = t.set("microphone_access", true).unwrap();
    assert_eq!(a.toast.as_deref(), Some("Apps can use your microphone again"));
    assert_eq!(t.set("camera_access", false).unwrap().toast.as_deref(), Some("No app can use your camera now"));
    assert_eq!(t.set("camera_access", true).unwrap().toast.as_deref(), Some("Apps can use your camera again"));
}

#[test]
fn print_screen_scroll_inactive_toasts_and_broadcast() {
    let mut t = user();
    let a = t.set("print_screen_snipping", false).unwrap();
    assert_eq!(a.toast.as_deref(), Some("Print Screen is yours: the Screenshot key gets it"));
    assert_eq!(t.os().get(Hive::Hkcu, r"Control Panel\Keyboard", "PrintScreenKeyForSnippingEnabled"), Some(RegValue::Dword(0)));
    let a = t.set("print_screen_snipping", true).unwrap();
    assert_eq!(a.toast.as_deref(), Some("Print Screen opens Snipping Tool again · your Screenshot key loses it"));
    let a = t.set("scroll_inactive_windows", false).unwrap();
    assert_eq!(a.toast.as_deref(), Some("Takes effect after you sign out"));
    assert_eq!(t.os().get(Hive::Hkcu, r"Control Panel\Desktop", "MouseWheelRouting"), Some(RegValue::Dword(0)));
    assert!(t.os().log.contains(&"broadcast:".to_string()));
}

#[test]
fn ucpd_blocked_write_is_reported_with_the_settings_page() {
    let mut t = user();
    t.os_mut().blocked_values.insert("taskbaral".into());
    let e = t.set("start_on_left", true).unwrap_err();
    assert_eq!(e, Error::BlockedByWindows { row: "start_on_left".into(), settings_uri: Some("ms-settings:taskbar") });
    assert_eq!(t.os().count("restart_explorer"), 0, "no Explorer restart for a change that didn't stick");
    assert!(!t.can_undo("start_on_left"));
}

// ---------------------------------------------------------------- DirectX global settings

#[test]
fn dxg_rows_edit_only_their_key() {
    let mut t = user();
    let orig = "VRROptimizeEnable=0;SwapEffectUpgradeEnable=1;";
    t.os_mut().put(Hive::Hkcu, rows::DXG_PATH, rows::DXG_VALUE, RegValue::Sz(orig.into()));
    assert!(sw(&t, "windowed_game_optimizations"));
    assert!(!sw(&t, "variable_refresh_rate"));
    assert!(!sw(&t, "auto_hdr"));
    let a = t.set("auto_hdr", true).unwrap();
    assert_eq!(a.toast.as_deref(), Some("Takes effect the next time a game starts"));
    assert_eq!(
        t.os().get(Hive::Hkcu, rows::DXG_PATH, rows::DXG_VALUE),
        Some(RegValue::Sz("VRROptimizeEnable=0;SwapEffectUpgradeEnable=1;AutoHDREnable=1;".into()))
    );
    t.set("variable_refresh_rate", true).unwrap();
    assert_eq!(
        t.os().get(Hive::Hkcu, rows::DXG_PATH, rows::DXG_VALUE),
        Some(RegValue::Sz("VRROptimizeEnable=1;SwapEffectUpgradeEnable=1;AutoHDREnable=1;".into()))
    );
    t.undo("variable_refresh_rate").unwrap();
    t.undo("auto_hdr").unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, rows::DXG_PATH, rows::DXG_VALUE), Some(RegValue::Sz(orig.into())));
}

// ---------------------------------------------------------------- keyboard

#[test]
fn stop_layout_hotkeys_writes_3_and_reloads() {
    let mut t = user();
    t.os_mut().put(Hive::Hkcu, r"Keyboard Layout\Toggle", "Language Hotkey", RegValue::Sz("2".into()));
    assert!(!sw(&t, "stop_layout_hotkeys"));
    t.set("stop_layout_hotkeys", true).unwrap();
    for n in ["Hotkey", "Language Hotkey", "Layout Hotkey"] {
        assert_eq!(t.os().get(Hive::Hkcu, r"Keyboard Layout\Toggle", n), Some(RegValue::Sz("3".into())), "{n}");
    }
    assert_eq!(t.os().count("reload_language_hotkeys"), 1);
    assert!(sw(&t, "stop_layout_hotkeys"));
    // undo: the user's own "2" back, the values that weren't there removed
    t.undo("stop_layout_hotkeys").unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, r"Keyboard Layout\Toggle", "Language Hotkey"), Some(RegValue::Sz("2".into())));
    assert_eq!(t.os().get(Hive::Hkcu, r"Keyboard Layout\Toggle", "Layout Hotkey"), None);
    assert_eq!(t.os().count("reload_language_hotkeys"), 2);
    // plain "off" writes Windows' defaults (Alt+Shift = 1, Ctrl+Shift = 2)
    t.set("stop_layout_hotkeys", true).unwrap();
    t.set("stop_layout_hotkeys", false).unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, r"Keyboard Layout\Toggle", "Language Hotkey"), Some(RegValue::Sz("1".into())));
    assert_eq!(t.os().get(Hive::Hkcu, r"Keyboard Layout\Toggle", "Layout Hotkey"), Some(RegValue::Sz("2".into())));
}

#[test]
fn accessibility_popups_clear_only_the_hotkey_bit() {
    let mut t = user();
    for (id, item, on, off) in [
        ("sticky_keys_popup", SpiItem::StickyKeysFlags, 510, 506),
        ("filter_keys_popup", SpiItem::FilterKeysFlags, 126, 122),
        ("toggle_keys_popup", SpiItem::ToggleKeysFlags, 62, 58),
    ] {
        assert!(sw(&t, id));
        t.set(id, false).unwrap();
        assert_eq!(t.os().spi[&item], off, "{id}");
        t.set(id, true).unwrap();
        assert_eq!(t.os().spi[&item], on, "{id}");
    }
}

#[test]
fn animations_both_settings() {
    let mut t = user();
    t.set("animations", false).unwrap();
    assert_eq!(t.os().spi[&SpiItem::ClientAreaAnimation], 0);
    assert_eq!(t.os().spi[&SpiItem::MinimizeAnimation], 0);
    t.os_mut().spi.insert(SpiItem::MinimizeAnimation, 1);
    assert!(sw(&t, "animations"), "either one on = on");
}

// ---------------------------------------------------------------- power

#[test]
fn usb_power_saving_desktop_keeps_battery_value_laptop_writes_both() {
    let mut t = admin();
    t.os_mut().power.insert(PowerSetting::UsbSelectiveSuspend, PowerValues { ac: 1, dc: 1 });
    let a = t.set("usb_power_saving", false).unwrap();
    assert_eq!(a.toast.as_deref(), Some("USB power saving off · mice and keyboards stay awake"));
    assert_eq!(t.os().power[&PowerSetting::UsbSelectiveSuspend], PowerValues { ac: 0, dc: 1 });
    t.os_mut().battery = true;
    t.set("usb_power_saving", true).unwrap();
    assert_eq!(t.os().power[&PowerSetting::UsbSelectiveSuspend], PowerValues { ac: 1, dc: 1 });
    t.set("usb_power_saving", false).unwrap();
    assert_eq!(t.os().power[&PowerSetting::UsbSelectiveSuspend], PowerValues { ac: 0, dc: 0 });
}

#[test]
fn sleep_switch_greys_sleep_after_and_comes_back_to_the_old_timeout() {
    let mut t = user();
    t.os_mut().power.insert(PowerSetting::Sleep, PowerValues { ac: 3600, dc: 900 });
    assert!(sw(&t, "sleep"));
    let st = t.read("sleep_after").unwrap();
    assert_eq!(st.value, Value::Timeout(Timeout::Seconds(3600)));
    assert!(st.enabled);

    t.set("sleep", false).unwrap();
    assert_eq!(t.os().power[&PowerSetting::Sleep], PowerValues { ac: 0, dc: 900 }, "desktop: battery value untouched");
    let st = t.read("sleep_after").unwrap();
    assert_eq!(st.value, Value::Timeout(Timeout::Never));
    assert!(!st.enabled);
    assert_eq!(st.disabled_reason, Some("Sleep is off"));
    assert!(matches!(t.set_timeout("sleep_after", Timeout::Seconds(600)), Err(Error::Disabled { .. })));

    t.set("sleep", true).unwrap();
    assert_eq!(t.os().power[&PowerSetting::Sleep], PowerValues { ac: 3600, dc: 900 }, "the old 1 h comes back");
}

#[test]
fn sleep_on_without_a_known_old_timeout_uses_30_min() {
    let mut t = user();
    t.os_mut().power.insert(PowerSetting::Sleep, PowerValues { ac: 0, dc: 0 });
    t.set("sleep", true).unwrap();
    assert_eq!(t.os().power[&PowerSetting::Sleep].ac, 1800);
}

#[test]
fn timeouts_only_from_the_list_and_undoable() {
    let mut t = user();
    let a = t.set_timeout("screen_off_after", Timeout::Seconds(300)).unwrap();
    assert_eq!(a.value, Value::Timeout(Timeout::Seconds(300)));
    assert_eq!(t.os().power[&PowerSetting::ScreenOff], PowerValues { ac: 300, dc: 300 });
    t.set_timeout("screen_off_after", Timeout::Never).unwrap();
    assert_eq!(t.os().power[&PowerSetting::ScreenOff].ac, 0);
    t.undo("screen_off_after").unwrap();
    assert_eq!(t.os().power[&PowerSetting::ScreenOff].ac, 300);
    assert!(matches!(t.set_timeout("screen_off_after", Timeout::Seconds(333)), Err(Error::WrongKind { .. })));
    assert!(matches!(t.set_timeout("clock_seconds", Timeout::Seconds(60)), Err(Error::WrongKind { .. })));
    assert!(matches!(t.set("screen_off_after", true), Err(Error::WrongKind { .. })));
    t.set_timeout("sleep_after", Timeout::Seconds(18000)).unwrap();
    assert_eq!(t.os().power[&PowerSetting::Sleep].ac, 18000);
    // labels as drawn
    let labels: Vec<String> = bu_toggles::TIMEOUT_CHOICES.iter().map(|t| t.label()).collect();
    assert_eq!(labels, ["1 min", "2 min", "5 min", "10 min", "15 min", "30 min", "1 h", "2 h", "5 h", "Never"]);
}

#[test]
fn fast_startup_greyed_while_hibernate_is_off_in_windows() {
    let mut t = admin();
    assert!(sw(&t, "fast_startup"));
    t.os_mut().hibernate_on = false;
    let fs = t.read("fast_startup").unwrap();
    assert_eq!(fs.value, Value::Switch(false));
    assert!(!fs.enabled);
    assert_eq!(fs.disabled_reason, Some("Needs Hibernate · Hibernate is off in Windows"));
    assert_eq!(t.set("fast_startup", true).unwrap_err(), Error::Disabled {
        row: "fast_startup".into(),
        reason: "Needs Hibernate · Hibernate is off in Windows"
    });
    // Hibernate back on in Windows: Fast Startup is what it was (HiberbootEnabled untouched)
    t.os_mut().hibernate_on = true;
    assert!(sw(&t, "fast_startup"));
}

#[test]
fn hibernate_and_windows_key_lock_are_gone() {
    // the owner, Oct 8: "nothing that can dramatically change the pc should be in the app"
    let mut t = admin();
    for id in ["hibernate", "windows_key_lock"] {
        assert_eq!(t.read(id).unwrap_err(), Error::UnknownRow(id.into()));
        assert_eq!(t.set(id, false).unwrap_err(), Error::UnknownRow(id.into()));
    }
    assert!(t.read_all().iter().all(|(id, _)| *id != "hibernate" && *id != "windows_key_lock"));
}

// ---------------------------------------------------------------- devices / apps

#[test]
fn bluetooth_missing_radio_greys_the_row() {
    let mut t = user();
    t.os_mut().bluetooth = None;
    let st = t.read("bluetooth").unwrap();
    assert!(!st.enabled);
    assert_eq!(st.disabled_reason, Some("No Bluetooth on this PC"));
    assert!(matches!(t.set("bluetooth", true), Err(Error::NotAvailable { .. })));
}

#[test]
fn bluetooth_toasts() {
    let mut t = user();
    assert_eq!(t.set("bluetooth", false).unwrap().toast.as_deref(), Some("Bluetooth off · wireless headsets and controllers disconnect"));
    assert_eq!(t.os().bluetooth, Some(false));
    assert_eq!(t.set("bluetooth", true).unwrap().toast.as_deref(), Some("Bluetooth on"));
}

#[test]
fn copilot_off_uninstalls_on_opens_the_store() {
    let mut t = user();
    assert!(sw(&t, "copilot"));
    let a = t.set("copilot", false).unwrap();
    assert_eq!(a.value, Value::Switch(false));
    assert!(!t.os().copilot);
    assert_eq!(t.os().count("remove_copilot"), 1);
    let a = t.set("copilot", true).unwrap();
    assert_eq!(a.open, Some(ChangeAction::OpenUri("ms-windows-store://pdp/?ProductId=9NHT9RB2F4HD".into())));
    assert_eq!(a.value, Value::Switch(false), "honest: still not installed until the Store installs it");
}

// ---------------------------------------------------------------- fullscreen optimizations per game

#[test]
fn fso_per_game_keeps_other_flags() {
    let mut t = user();
    let rl = r"C:\Program Files\Epic Games\rocketleague\Binaries\Win64\RocketLeague.exe";
    let other = r"D:\Games\Old\game.exe";
    t.os_mut().put(Hive::Hkcu, rows::LAYERS_PATH, other, RegValue::Sz("~ RUNASADMIN".into()));
    assert_eq!(t.read("fullscreen_optimizations_off").unwrap().value, Value::Games(vec![]));

    t.fso_set(rl, true).unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, rows::LAYERS_PATH, rl), Some(RegValue::Sz("~ DISABLEDXMAXIMIZEDWINDOWEDMODE".into())));
    let a = t.fso_set(other, true).unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, rows::LAYERS_PATH, other), Some(RegValue::Sz("~ RUNASADMIN DISABLEDXMAXIMIZEDWINDOWEDMODE".into())));
    match a.value {
        Value::Games(g) => assert_eq!(g.len(), 2),
        v => panic!("{v:?}"),
    }
    assert!(t.fso_state(rl).unwrap());

    t.fso_remove(other).unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, rows::LAYERS_PATH, other), Some(RegValue::Sz("~ RUNASADMIN".into())), "user's flag stays");
    t.fso_set(rl, false).unwrap();
    assert_eq!(t.os().get(Hive::Hkcu, rows::LAYERS_PATH, rl), None, "only our flag -> value deleted");
    t.fso_undo(rl).unwrap();
    assert!(t.fso_state(rl).unwrap());
    // the row itself is a list, not a switch
    assert!(matches!(t.set("fullscreen_optimizations_off", true), Err(Error::WrongKind { .. })));
    assert!(matches!(t.fso_set("game.exe", true), Err(Error::WrongKind { .. })));
    assert!(matches!(t.fso_set(r"C:\Games\notes.txt", true), Err(Error::WrongKind { .. })));
    assert!(matches!(t.fso_undo(r"C:\never.exe"), Err(Error::NothingToUndo(_))));
}

// ---------------------------------------------------------------- errors

#[test]
fn errors_are_typed_never_panics() {
    let mut t = user();
    assert_eq!(t.read("nope").unwrap_err(), Error::UnknownRow("nope".into()));
    assert_eq!(t.set("nope", true).unwrap_err(), Error::UnknownRow("nope".into()));
    assert_eq!(t.undo("clock_seconds").unwrap_err(), Error::NothingToUndo("clock_seconds".into()));
    t.os_mut().failing_ops.insert("spi_set");
    assert!(matches!(t.set("sticky_keys_popup", false), Err(Error::Os { .. })));
    t.os_mut().failing_ops.insert("reg_read");
    assert!(matches!(t.read("clock_seconds"), Err(Error::Os { .. })));
    // one failing row doesn't hide the others
    let all = t.read_all();
    assert_eq!(all.len(), ROWS.len());
    assert!(all.iter().any(|(_, r)| r.is_ok()));
    assert!(all.iter().any(|(_, r)| r.is_err()));
}

#[test]
fn failed_explorer_restart_keeps_the_change() {
    let mut t = user();
    t.os_mut().failing_ops.insert("restart_explorer");
    let a = t.set("classic_context_menu", true).unwrap();
    assert!(!a.explorer_restarted);
    assert!(sw(&t, "classic_context_menu"));
}

#[test]
fn failed_shell_refresh_keeps_the_change() {
    let mut t = user();
    t.os_mut().failing_ops.insert("refresh_shell");
    t.set("show_hidden_files", true).unwrap();
    assert!(sw(&t, "show_hidden_files"));
}

#[test]
fn start_on_left_and_alt_tab_need_no_explorer_restart() {
    let mut t = user();
    let a = t.set("start_on_left", true).unwrap();
    assert!(!a.explorer_restarted);
    let a = t.set("alt_tab_edge_tabs", false).unwrap();
    assert!(!a.explorer_restarted);
    assert_eq!(t.os().count("restart_explorer"), 0);
}

#[test]
fn read_all_on_a_default_pc() {
    let t = user();
    for (id, r) in t.read_all() {
        let st = r.unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(st.id, id);
    }
}

/// Order 036: the app's reset puts back any old timeout, also one that isn't in the list (45 min).
#[test]
fn set_seconds_puts_back_any_timeout() {
    let mut t = user();
    let a = t.set_seconds("screen_off_after", 2700).unwrap();
    assert_eq!(a.value, Value::Timeout(Timeout::Seconds(2700)));
    assert_eq!(t.os().power[&PowerSetting::ScreenOff].ac, 2700);
    t.set_seconds("screen_off_after", 0).unwrap();
    assert_eq!(t.os().power[&PowerSetting::ScreenOff].ac, 0);
    assert!(matches!(t.set_seconds("clock_seconds", 60), Err(Error::WrongKind { .. })));
    // Sleep off: "Sleep after" still refuses (the Sleep switch puts it on first)
    t.set("sleep", false).unwrap();
    assert!(matches!(t.set_seconds("sleep_after", 2700), Err(Error::Disabled { .. })));
    t.set("sleep", true).unwrap();
    t.set_seconds("sleep_after", 2700).unwrap();
    assert_eq!(t.os().power[&PowerSetting::Sleep].ac, 2700);
}
