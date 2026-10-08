use super::scratch::Scratch;
use super::*;

#[test]
fn missing_file_gives_defaults_and_writes_nothing() {
    let s = Scratch::new("missing");
    let store = SettingsStore::open(s.dir());
    assert_eq!(store.load_note(), None);
    assert_eq!(store.glass(), GlassStyle::Liquid);
    assert_eq!(store.get_bool(Scope::App, "x"), None);
    assert!(store.bool_or(Scope::App, "x", true));
    assert!(!s.dir().exists(), "opening must not create anything");
}

#[test]
fn every_type_round_trips_through_the_file() {
    let s = Scratch::new("roundtrip");
    let list = vec!["a,b".to_string(), String::new(), "tab\there\\ \n end".to_string(), "Čćžšđ".to_string()];
    {
        let mut st = SettingsStore::open(s.dir());
        assert!(st.set_bool(Scope::App, "start_with_windows", true).unwrap());
        assert!(st.set_i64(Scope::Page("audio"), "step", -42).unwrap());
        assert!(st.set_f64(Scope::Page("audio"), "gain", 0.1).unwrap());
        assert!(st.set_f64(Scope::Page("audio"), "nan", f64::NAN).unwrap());
        assert!(st.set_str(Scope::Page("display"), "pre\tset", "1920×1080 · 144 Hz,\r\nx").unwrap());
        assert!(st.set_list(Scope::Page("timers"), "places", &list).unwrap());
        assert!(st.set_list(Scope::Page("timers"), "empty", &[]).unwrap());
        assert!(st.set_list(Scope::Page("timers"), "one_empty", &[String::new()]).unwrap());
        assert!(st.set_glass(GlassStyle::Frosted).unwrap());
    }
    let st = SettingsStore::open(s.dir());
    assert_eq!(st.load_note(), None);
    assert_eq!(st.get_bool(Scope::App, "start_with_windows"), Some(true));
    assert_eq!(st.get_i64(Scope::Page("audio"), "step"), Some(-42));
    assert_eq!(st.get_f64(Scope::Page("audio"), "gain"), Some(0.1));
    assert!(st.get_f64(Scope::Page("audio"), "nan").unwrap().is_nan());
    assert_eq!(st.get_str(Scope::Page("display"), "pre\tset"), Some("1920×1080 · 144 Hz,\r\nx"));
    assert_eq!(st.get_list(Scope::Page("timers"), "places"), Some(&list[..]));
    assert_eq!(st.get_list(Scope::Page("timers"), "empty"), Some(&[][..]));
    assert_eq!(st.get_list(Scope::Page("timers"), "one_empty"), Some(&[String::new()][..]));
    assert_eq!(st.glass(), GlassStyle::Frosted);
}

#[test]
fn pages_are_namespaced_and_types_dont_mix() {
    let s = Scratch::new("ns");
    let mut st = SettingsStore::open(s.dir());
    st.set_bool(Scope::Page("audio"), "on", true).unwrap();
    st.set_bool(Scope::Page("mouse"), "on", false).unwrap();
    assert_eq!(st.get_bool(Scope::Page("audio"), "on"), Some(true));
    assert_eq!(st.get_bool(Scope::Page("mouse"), "on"), Some(false));
    assert_eq!(st.get_bool(Scope::App, "on"), None);
    // a value of another type reads as "not set"
    assert_eq!(st.get_i64(Scope::Page("audio"), "on"), None);
    assert_eq!(st.i64_or(Scope::Page("audio"), "on", 7), 7);
    assert_eq!(st.keys(Scope::Page("audio")), vec!["on"]);
    assert!(st.clear_page("audio").unwrap());
    assert_eq!(st.get_bool(Scope::Page("audio"), "on"), None);
    assert_eq!(st.get_bool(Scope::Page("mouse"), "on"), Some(false));
    assert!(!st.clear_page("audio").unwrap());
}

#[test]
fn writes_only_on_change() {
    let s = Scratch::new("onchange");
    let mut st = SettingsStore::open(s.dir());
    assert!(st.set_i64(Scope::App, "n", 5).unwrap());
    assert!(st.set_f64(Scope::App, "f", f64::NAN).unwrap());
    let p = st.path();
    // remove the file: an unchanged set must not bring it back
    std::fs::remove_file(&p).unwrap();
    assert!(!st.set_i64(Scope::App, "n", 5).unwrap());
    assert!(!st.set_f64(Scope::App, "f", f64::NAN).unwrap());
    assert!(!st.remove(Scope::App, "absent").unwrap());
    assert!(!p.exists(), "an unchanged value must not write the file");
    assert!(st.set_i64(Scope::App, "n", 6).unwrap());
    assert!(p.exists());
}

#[test]
fn write_is_atomic_no_temp_left() {
    let s = Scratch::new("atomic");
    let mut st = SettingsStore::open(s.dir());
    for i in 0..20 {
        st.set_i64(Scope::App, "n", i).unwrap();
    }
    let names: Vec<String> =
        std::fs::read_dir(s.dir()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec![FILE_NAME.to_string()]);
    assert_eq!(SettingsStore::open(s.dir()).get_i64(Scope::App, "n"), Some(19));
}

#[test]
fn broken_file_is_kept_aside_and_defaults_used() {
    for (i, bad) in [&b"garbage"[..], b"Boyler Utilities settings 1\napp\tx\tb\tmaybe\n", b"\xff\xfe", b""].iter().enumerate() {
        let s = Scratch::new(&format!("broken{i}"));
        std::fs::create_dir_all(s.dir()).unwrap();
        let p = s.dir().join(FILE_NAME);
        std::fs::write(&p, bad).unwrap();
        let mut st = SettingsStore::open(s.dir());
        let broken = s.dir().join("settings.cfg.broken");
        assert_eq!(st.load_note(), Some(&LoadNote::Broken { kept_as: broken.clone() }), "case {i}");
        assert_eq!(std::fs::read(&broken).unwrap(), *bad, "the broken file is kept as it was");
        assert!(!p.exists());
        assert_eq!(st.glass(), GlassStyle::Liquid);
        // the next write makes a fresh good file and leaves the broken copy alone
        st.set_bool(Scope::App, "a", true).unwrap();
        assert_eq!(SettingsStore::open(s.dir()).get_bool(Scope::App, "a"), Some(true));
        assert_eq!(std::fs::read(&broken).unwrap(), *bad);
    }
}

#[test]
fn crlf_file_still_reads() {
    let s = Scratch::new("crlf");
    std::fs::create_dir_all(s.dir()).unwrap();
    std::fs::write(s.dir().join(FILE_NAME), "Boyler Utilities settings 1\r\napp\tglass\ts\twindows\r\npage:a\tn\ti\t3\r\n").unwrap();
    let st = SettingsStore::open(s.dir());
    assert_eq!(st.load_note(), None);
    assert_eq!(st.glass(), GlassStyle::WindowsLook);
    assert_eq!(st.get_i64(Scope::Page("a"), "n"), Some(3));
}

#[test]
fn reset_app_settings_keeps_the_pc_records() {
    let s = Scratch::new("reset");
    let mut st = SettingsStore::open(s.dir());
    st.set_glass(GlassStyle::WindowsLook).unwrap();
    st.set_list(Scope::App, "keys", &["mic=2,77".to_string()]).unwrap();
    st.set_i64(Scope::Page("timers"), "preset", 5).unwrap();
    st.set_str(Scope::Pc, "mouse\u{1f}speed", "10").unwrap();
    assert!(st.reset_app_settings().unwrap());
    let st = SettingsStore::open(s.dir());
    assert_eq!(st.glass(), GlassStyle::Liquid);
    assert_eq!(st.get_list(Scope::App, "keys"), None);
    assert_eq!(st.get_i64(Scope::Page("timers"), "preset"), None);
    assert_eq!(st.get_str(Scope::Pc, "mouse\u{1f}speed"), Some("10"));
}

#[test]
fn glass_numbers_match_the_drawing() {
    let l = GlassStyle::Liquid.numbers();
    assert_eq!((l.tint_rgb, l.tint_alpha, l.blur_px, l.saturate, l.brightness, l.bubbles_alpha), ([20, 20, 26], 0.22, 13.0, 1.70, 1.04, 0.08));
    let f = GlassStyle::Frosted.numbers();
    assert_eq!((f.tint_rgb, f.tint_alpha, f.blur_px, f.saturate, f.brightness, f.bubbles_alpha), ([28, 28, 32], 0.40, 24.0, 1.50, 1.0, 0.07));
    let w = GlassStyle::WindowsLook.numbers();
    assert_eq!((w.tint_rgb, w.tint_alpha, w.blur_px, w.saturate, w.brightness, w.bubbles_alpha), ([44, 44, 46], 0.75, 31.0, 1.15, 1.0, 0.06));
    for g in GlassStyle::ALL {
        assert_eq!(GlassStyle::from_id(g.id()), Some(g));
    }
    assert_eq!(GlassStyle::from_id("nope"), None);
}

#[test]
fn unknown_glass_id_falls_back_to_liquid_and_default_is_not_stored() {
    let s = Scratch::new("glass");
    let mut st = SettingsStore::open(s.dir());
    st.set_str(Scope::App, "glass", "chrome").unwrap();
    assert_eq!(st.glass(), GlassStyle::Liquid);
    assert!(st.set_glass(GlassStyle::Liquid).unwrap(), "setting the default removes the stored value");
    assert_eq!(st.get(Scope::App, "glass"), None);
}

#[test]
fn scratch_folder_is_removed_after_the_test() {
    let dir;
    {
        let s = Scratch::new("cleanup");
        dir = s.dir().to_path_buf();
        SettingsStore::open(s.dir()).set_bool(Scope::App, "a", true).unwrap();
        assert!(dir.exists());
    }
    assert!(!dir.exists());
}

/// Order 033: Settings › Theme is saved (Dark = the default, not written), and Match Windows follows Windows' app theme.
#[test]
fn theme_round_trips_and_match_windows_follows_windows() {
    let s = Scratch::new("theme");
    {
        let mut st = SettingsStore::open(s.dir());
        assert_eq!(st.theme(), Theme::Dark);
        assert!(st.set_theme(Theme::MatchWindows).unwrap());
    }
    let mut st = SettingsStore::open(s.dir());
    assert_eq!(st.theme(), Theme::MatchWindows);
    assert_eq!(st.get_str(Scope::App, "theme"), Some("auto"));
    assert!(st.set_theme(Theme::Dark).unwrap());
    assert_eq!(st.get_str(Scope::App, "theme"), None, "the default is not written");
    assert!(!Theme::Dark.light(true) && Theme::Light.light(false));
    assert!(Theme::MatchWindows.light(true) && !Theme::MatchWindows.light(false));
}
