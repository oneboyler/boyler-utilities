//! Order 068 - does every Tweaks row SHOW Windows' real state and DO what it says, on the REAL PC?
//!
//! - `toggles-verify raw`            prints every raw value each row reads (registry / SPI / power / DXG), read-only.
//! - `toggles-verify flip <out_dir>` ONLY while the user is away (no input for 2 min, checked again before every step): flips every
//!   harmless, restart-free row through the same `Toggles::set` the app calls, holds the flipped state until
//!   `<out_dir>\resume.flag` appears (meanwhile a script reads Windows' own Settings / Shell), puts every row back with
//!   `Toggles::undo`, and proves the raw dump is identical to the one before. Input, an error or a panic puts everything back at once.
//!
//! `cargo run -p bu-toggles --example toggles-verify -- raw`

#[cfg(windows)]
mod imp {
    use bu_toggles::model::Value;
    use bu_toggles::os::{Hive, PowerSetting, RegValue, SpiItem, TogglesOs};
    use bu_toggles::real::RealOs;
    use bu_toggles::rows::{self, Method, ROWS};
    use bu_toggles::Toggles;
    use std::collections::BTreeMap;

    fn rv(v: &Option<RegValue>) -> String {
        match v {
            None => "(absent)".into(),
            Some(RegValue::Dword(d)) => format!("dword {d}"),
            Some(RegValue::Sz(s)) => format!("sz {s:?}"),
            Some(RegValue::Other { kind, bytes }) => format!("type{kind} {bytes:?}"),
        }
    }

    /// Every raw value the rows read, as `row | source = value` lines (sorted, so two dumps can be compared).
    pub fn raw(t: &Toggles<RealOs>) -> BTreeMap<String, String> {
        let os = t.os();
        let mut m = BTreeMap::new();
        let mut put = |row: &str, src: String, val: String| {
            m.insert(format!("{row} | {src}"), val);
        };
        let reg = |hive: Hive, path: &str, name: &str| os.reg_read(hive, path, name).map(|v| rv(&v)).unwrap_or_else(|e| format!("ERR {e}"));
        for r in ROWS.iter() {
            match r.method {
                Method::Reg { values, .. } => {
                    for v in values {
                        let h = if v.hive == Hive::Hkcu { "HKCU" } else { "HKLM" };
                        put(r.id, format!("{h}\\{}\\{}", v.path, v.name), reg(v.hive, v.path, v.name));
                    }
                }
                Method::KeyExists { hive, key, .. } => {
                    put(r.id, format!("key {key}"), format!("{:?}", os.reg_key_exists(hive, key)));
                }
                Method::Dxg { .. } => put(r.id, "DirectXUserGlobalSettings".into(), reg(Hive::Hkcu, rows::DXG_PATH, rows::DXG_VALUE)),
                Method::SpiFlag { item, .. } => put(r.id, format!("SPI {item:?}"), format!("{:?}", os.spi_get(item))),
                Method::Animations => {
                    for item in [SpiItem::ClientAreaAnimation, SpiItem::MinimizeAnimation] {
                        put(r.id, format!("SPI {item:?}"), format!("{:?}", os.spi_get(item)));
                    }
                }
                Method::LangHotkeys => {
                    for (n, _) in [("Hotkey", 0), ("Language Hotkey", 0), ("Layout Hotkey", 0)] {
                        put(r.id, format!("HKCU\\Keyboard Layout\\Toggle\\{n}"), reg(Hive::Hkcu, r"Keyboard Layout\Toggle", n));
                    }
                }
                Method::PowerSwitch(s) | Method::PowerTimeout(s) => put(r.id, format!("power {s:?}"), format!("{:?}", os.power_read(s))),
                Method::SleepSwitch => put(r.id, "power Sleep".into(), format!("{:?}", os.power_read(PowerSetting::Sleep))),
                Method::Hags => {
                    put(r.id, "HwSchMode".into(), reg(Hive::Hklm, rows::HAGS_PATH, rows::HAGS_VALUE));
                    put(r.id, "driver caps".into(), format!("{:?}", os.gpu_scheduling()));
                }
                Method::FastStartup => {
                    put(r.id, "HiberbootEnabled".into(), reg(Hive::Hklm, rows::HIBERBOOT_PATH, rows::HIBERBOOT_VALUE));
                    put(r.id, "hibernate".into(), format!("{:?}", os.hibernate_on()));
                }
                Method::Bluetooth => put(r.id, "radio".into(), format!("{:?}", os.bluetooth())),
                Method::Copilot => put(r.id, "package".into(), format!("{:?}", os.copilot_installed())),
                Method::FsoGames => {
                    let mut l: Vec<String> = os
                        .reg_values(Hive::Hkcu, rows::LAYERS_PATH)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(n, v)| format!("{n} = {}", rv(&Some(v))))
                        .collect();
                    l.sort();
                    put(r.id, "Layers".into(), l.join(" || "));
                }
            }
        }
        m
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetLastInputInfo(p: *mut [u32; 2]) -> i32;
        fn GetTickCount() -> u32;
    }
    /// Seconds since the last keyboard / mouse input.
    pub fn idle_secs() -> u32 {
        let mut l = [8u32, 0];
        unsafe {
            GetLastInputInfo(&mut l);
            GetTickCount().wrapping_sub(l[1]) / 1000
        }
    }

    /// Rows flipped for real: harmless, no restart, nothing his games / stream / network / power plans rely on.
    const FLIP: &[&str] = &[
        "show_file_extensions", "show_hidden_files", "explorer_opens_this_pc", "copy_window_details", "onedrive_ads_explorer",
        "end_task", "clock_seconds", "start_on_left", "task_view_button", "taskbar_flashing", "alt_tab_edge_tabs",
        "game_mode", "xbox_button_game_bar", "windowed_game_optimizations", "auto_hdr", "variable_refresh_rate",
        "stop_layout_hotkeys", "sticky_keys_popup", "filter_keys_popup", "toggle_keys_popup", "clipboard_history",
        "scroll_inactive_windows", "print_screen_snipping", "call_ducking", "mono_audio",
        "lock_screen_tips", "ads_in_settings", "tips_notifications", "transparency", "dark_mode", "animations",
    ];

    struct Guard<'a> {
        t: &'a mut Toggles<RealOs>,
        done: Vec<&'static str>,
    }
    impl Guard<'_> {
        fn put_back(&mut self) -> Vec<String> {
            let mut errs = Vec::new();
            while let Some(id) = self.done.pop() {
                if let Err(e) = self.t.undo(id) {
                    errs.push(format!("UNDO {id}: {e}"));
                }
            }
            errs
        }
    }
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            let _ = self.put_back();
        }
    }

    pub fn flip(dir: &str) -> i32 {
        let say = |s: String| println!("{s}");
        if idle_secs() < 120 {
            say(format!("NOT STARTED: input {} s ago (need 120)", idle_secs()));
            return 2;
        }
        let mut t = Toggles::new(RealOs::new());
        let before = raw(&t);
        let _ = std::fs::write(format!("{dir}\\raw_before.txt"), before.iter().map(|(k, v)| format!("{k} = {v}\n")).collect::<String>());
        let mut g = Guard { t: &mut t, done: Vec::new() };
        let mut table: Vec<String> = Vec::new();
        let mut aborted = false;
        for id in FLIP {
            if idle_secs() < 100 {
                say("INPUT SEEN - stopping and putting everything back".into());
                aborted = true;
                break;
            }
            let was = match g.t.read(id).map(|s| s.value) {
                Ok(Value::Switch(b)) => b,
                other => {
                    table.push(format!("{id} | SKIP read {other:?}"));
                    continue;
                }
            };
            match g.t.set(id, !was) {
                Ok(_) => {
                    g.done.push(id);
                    let now = g.t.read(id).map(|s| s.value);
                    let ok = now == Ok(Value::Switch(!was));
                    table.push(format!("{id} | {was} -> {now:?} | read-back {}", if ok { "OK" } else { "MISMATCH" }));
                }
                Err(e) => table.push(format!("{id} | set({}) refused: {e}", !was)),
            }
        }
        // the flipped state: everything the app now shows, plus the raw values, for the Windows-side reads
        if !aborted {
            let flipped = raw(g.t);
            let _ = std::fs::write(
                format!("{dir}\\raw_flipped.txt"),
                flipped.iter().map(|(k, v)| format!("{k} = {v}\n")).collect::<String>(),
            );
            let _ = std::fs::write(
                format!("{dir}\\app_flipped.txt"),
                g.t.read_all().iter().map(|(id, st)| format!("{id} = {:?}\n", st.as_ref().map(|s| &s.value))).collect::<String>(),
            );
            // hold the flipped state while the caller reads Windows' own views: it writes resume.flag when done; input from
            // the user, or 6 minutes, ends the hold too
            let _ = std::fs::remove_file(format!("{dir}\\resume.flag"));
            let _ = std::fs::write(format!("{dir}\\flipped.flag"), "flipped");
            say("FLIPPED - holding until resume.flag".into());
            let t0 = std::time::Instant::now();
            while !std::path::Path::new(&format!("{dir}\\resume.flag")).exists() && t0.elapsed().as_secs() < 360 {
                if idle_secs() < 20 {
                    say("INPUT SEEN while holding - putting everything back".into());
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            let _ = std::fs::remove_file(format!("{dir}\\flipped.flag"));
        }
        let errs = g.put_back();
        drop(g);
        let after = raw(&t);
        let mut diff = 0;
        for (k, v) in &before {
            if after.get(k) != Some(v) {
                diff += 1;
                say(format!("DIFF {k}: before {v} / after {:?}", after.get(k)));
            }
        }
        let _ = std::fs::write(format!("{dir}\\raw_after.txt"), after.iter().map(|(k, v)| format!("{k} = {v}\n")).collect::<String>());
        let _ = std::fs::write(format!("{dir}\\flip_table.txt"), table.join("\n"));
        for l in &table {
            say(l.clone());
        }
        for e in &errs {
            say(e.clone());
        }
        say(format!("PUT BACK: {} diff(s) between raw before and after, {} undo error(s)", diff, errs.len()));
        i32::from(diff != 0 || !errs.is_empty())
    }
}

#[cfg(windows)]
fn main() {
    use bu_toggles::real::RealOs;
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("raw") => {
            let t = bu_toggles::Toggles::new(RealOs::read_only());
            for (k, v) in imp::raw(&t) {
                println!("{k} = {v}");
            }
        }
        Some("flip") if a.len() > 2 => std::process::exit(imp::flip(&a[2])),
        _ => eprintln!("usage: toggles-verify raw | flip <out_dir>"),
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows only");
}
