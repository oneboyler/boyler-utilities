//! Order 043: Get takes over the original NotificationsForOBS (ClipPing) when it still runs on its own. Found through its
//! "Start with Windows" entry (HKCU Run value "NotificationsForOBS", or "ClipPing" from its first versions) -> the exe it
//! starts -> NotificationsForOBS.ini next to that exe (read-only, never written). Then its settings are imported, its
//! startup entry is switched off the way the Startup tab does it (the StartupApproved flag, never the Run value itself;
//! one line in the ONE change log, so Settings' "Back to how your PC was" switches it on again) and the running copy is
//! closed (its own tray Quit; only if it hangs, that one process - while it still runs from that exe - is ended). Then
//! the app's own feature starts.
//! No entry, an entry whose exe is gone, or an original that neither starts with Windows nor runs: nothing is done.

use std::path::{Path, PathBuf};

use bu_obs::os::ObsOs;
use bu_obs::Settings;
use bu_startup::os::{Hive, RegView, StartupOs};
use bu_startup::saved::{State, Target};
use bu_startup::{ApprovedKey, ApprovedSlot, Startup, RUN};

use crate::undo::Val;

/// The original's Run value names.
const NAMES: [&str; 2] = ["NotificationsForOBS", "ClipPing"];
/// How long its own Quit gets before that process is ended.
const QUIT_MS: u32 = 3000;

/// The original as found (read-only).
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// its Run value: name and command
    pub value_name: String,
    pub command: String,
    pub exe: PathBuf,
    /// the name the Startup tab shows for it (the exe's description, else the value name)
    pub label: String,
    /// its entry starts it with Windows now
    pub on: bool,
    /// its settings file and what it holds (None: no file next to the exe)
    pub ini: Option<PathBuf>,
    pub settings: Option<Settings>,
    /// a NotificationsForOBS running now: (pid, exe path)
    pub running: Option<(u32, PathBuf)>,
}

impl Found {
    /// Still in use on its own (starts with Windows or runs): only then it is taken over.
    pub fn active(&self) -> bool {
        self.on || self.running.is_some()
    }
}

/// What the take-over did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Done {
    pub imported: bool,
    pub turned_off: bool,
    pub closed: bool,
}

impl Done {
    /// The one line the page shows.
    pub fn line(&self) -> String {
        let mut parts = Vec::new();
        if self.imported {
            parts.push("settings imported");
        }
        if self.turned_off {
            parts.push("its startup turned off");
        }
        let mut t = "Took over NotificationsForOBS".to_string();
        if !parts.is_empty() {
            t = format!("{t}: {}", parts.join(", "));
        }
        if self.turned_off {
            t.push_str(" (undo in Settings)");
        }
        t
    }
}

/// Its entry's StartupApproved flag (where the Startup tab switches it).
pub fn target(value_name: &str) -> Target {
    Target::Flag(ApprovedSlot { hive: Hive::CurrentUser, key: ApprovedKey::Run, value_name: value_name.to_string() })
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
}

/// Look for the original (reads only: the Run key, its flag, the exe, the ini, the process list).
pub fn find<S: StartupOs>(st: &Startup<S>, obs: &dyn ObsOs) -> Option<Found> {
    let vals = st.os().reg_strings(Hive::CurrentUser, RegView::Bits64, RUN).ok()?;
    let v = NAMES.iter().find_map(|n| vals.iter().find(|v| v.name.eq_ignore_ascii_case(n)))?;
    let exe = bu_obs::settings::exe_of_command(&st.os().expand_env(&v.data))?;
    if !st.os().file_exists(&exe) {
        return None;
    }
    let label = st.os().file_info(&exe).description.map(|d| d.trim().to_string()).filter(|d| !d.is_empty()).unwrap_or_else(|| v.name.clone());
    let on = st.state_of(&target(&v.name)).map(State::is_on).unwrap_or(false);
    let ini = bu_obs::settings::find_clipping_ini(std::slice::from_ref(&exe));
    let settings = ini.as_deref().and_then(bu_obs::settings::import_file);
    Some(Found { value_name: v.name.clone(), command: v.data.clone(), exe, label, on, ini, settings, running: obs.other_app() })
}

/// Switch its startup entry off (`log` gets the change log line: item, label, old, new - the Startup tab's own form) and
/// close the running copy. The settings are the caller's to save (`f.settings`).
pub fn take_over<S: StartupOs>(st: &Startup<S>, obs: &mut dyn ObsOs, f: &Found, log: &mut dyn FnMut(&str, &str, &Val, &Val)) -> Done {
    let mut d = Done { imported: f.settings.is_some(), ..Done::default() };
    if f.on {
        let t = target(&f.value_name);
        if st.put_back(&t, State::Enabled(false)).is_ok() {
            d.turned_off = true;
            // (the Startup tab's values: pages/startup.rs `val`)
            log(&t.to_text(), &f.label, &Val::new(&State::Enabled(true).to_text(), "Starts with Windows"), &Val::new(&State::Enabled(false).to_text(), "Off"));
        }
    }
    if let Some((pid, p)) = &f.running {
        // its own tray Quit first; ended only if it hangs, and only that process from that exe
        let asked = obs.close_other_app(*pid);
        let mut gone = asked && obs.wait_exit(*pid, QUIT_MS);
        if !gone && (same_path(p, &f.exe) || p.as_os_str().is_empty()) {
            gone = obs.end_other_app(*pid, &f.exe) && obs.wait_exit(*pid, 1000);
        }
        d.closed = gone;
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Scope;
    use bu_startup::fake::FakeOs as StartFake;
    use bu_startup::APPROVED;

    const PID: u32 = 7788;
    const ITEM: &str = "flag|HKCU|Run|NotificationsForOBS";

    /// A scratch folder (removed on drop): the original's pretend exe folder.
    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn dir(tag: &str) -> Dir {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = PathBuf::from(r"C:\BoylerUtilities-scratch\043").join(format!("takeover-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }

    /// ClipPing's file as it writes it: UTF-16 LE with a BOM (a real file's shape, other values).
    fn write_ini(d: &Path) {
        let text = "\r\n[NotificationsForOBS]\r\nStartWithWindows=0x1\r\nPopupWhere=0x2\r\nPopupPos=0x3\r\nVolume=0x50\r\nStatusIcon=0x1\r\nSwitchKey=0x13\r\n[Scenes]\r\nCount=0x2\r\n1=16:9\r\n2=21:9\r\n";
        let mut b = vec![0xFF, 0xFE];
        for u in text.encode_utf16() {
            b.extend_from_slice(&u.to_le_bytes());
        }
        std::fs::write(d.join("NotificationsForOBS.ini"), b).unwrap();
    }

    /// The original on a fake PC: its Run value (+ its flag bytes), its exe (if `exe`), running (if `running`).
    fn pc(d: &Path, exe: bool, flag: Option<&[u8]>, running: bool) -> (Startup<StartFake>, bu_obs::fake::FakeOs, PathBuf) {
        let path = d.join("NotificationsForOBS (1).exe");
        let mut f = StartFake::new().run(Hive::CurrentUser, RegView::Bits64, RUN, "NotificationsForOBS", &format!("\"{}\" --autostart", path.display()));
        if exe {
            f = f.file(&path.to_string_lossy(), "", "Notifications for OBS");
        }
        if let Some(b) = flag {
            f = f.binary(Hive::CurrentUser, &format!(r"{APPROVED}\Run"), "NotificationsForOBS", b);
        }
        let obs = bu_obs::fake::FakeOs::new(d);
        if running {
            obs.with(|s| s.other = Some((PID, path.clone())));
        }
        (Startup::new(f), obs, path)
    }

    fn flag(st: &Startup<StartFake>) -> Option<Vec<u8>> {
        st.os().get_binary(Hive::CurrentUser, &format!(r"{APPROVED}\Run"), "NotificationsForOBS")
    }

    fn saved_settings() -> bool {
        crate::services::with(|s| s.store.get_list(Scope::Page(super::super::PAGE), "settings").is_some()).unwrap()
    }

    /// Get with the original starting with Windows and running: its settings imported, its entry off (one change log
    /// line), its copy closed through its own Quit, then ours runs on those settings. Undo puts the entry back on.
    #[test]
    fn get_takes_over_the_running_original() {
        let d = dir("run");
        write_ini(&d.0);
        let (st, mut obs, _) = pc(&d.0, true, Some(&[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), true);
        crate::services::init(windows::Win32::Foundation::HWND::default(), true);
        let line = crate::addons::take_over(&st, &mut obs);
        assert_eq!(line.as_deref(), Some("Took over NotificationsForOBS: settings imported, its startup turned off (undo in Settings)"));
        // its entry: off the way Task Manager writes it (03 + the time); the Run value itself is untouched
        assert_eq!(flag(&st).map(|b| b[0]), Some(3));
        assert!(st.os().reg_strings(Hive::CurrentUser, RegView::Bits64, RUN).unwrap().iter().any(|v| v.name == "NotificationsForOBS"));
        // its copy: asked to Quit (never ended)
        obs.with(|s| assert!(s.other_closed == [PID] && s.other_ended.is_empty() && s.other.is_none()));
        // the change log: the Startup tab's own line
        let r = crate::services::with(|s| crate::undo::read_record(&s.store, "sup", ITEM)).flatten().expect("a change log line");
        assert_eq!((r.label.as_str(), r.was.raw.as_str(), r.was.text.as_str(), r.now.raw.as_str()), ("Notifications for OBS", "on", "Starts with Windows", "off"));
        // ours starts, on the imported settings
        assert_eq!(crate::addons::set_for("obs", true, true), None, "a test copy takes nothing over itself");
        assert!(crate::obs::running());
        let set = crate::obs::settings().unwrap();
        assert_eq!((set.where_, set.pos, set.vol, set.status), (2, 3, 80, 1));
        assert_eq!(set.scenes, ["16:9", "21:9"]);
        assert_eq!(set.switch_key, Some(bu_obs::keys::KeyBind::new(0x13, 0)));
        crate::addons::set_for("obs", false, true);
        // undo (Settings › Back to how your PC was -> the Startup tab's `apply`: the item and the old value, parsed the same)
        let t = Target::from_text(ITEM).unwrap();
        st.put_back(&t, State::from_text(&r.was.raw).unwrap()).unwrap();
        assert_eq!(st.state_of(&t).unwrap(), State::Enabled(true));
        assert_eq!(flag(&st).map(|b| b[0]), Some(2));
        crate::services::shutdown();
    }

    /// No Run entry (never on this PC): nothing is taken over, ours starts on its defaults.
    #[test]
    fn get_without_the_original_starts_ours_on_defaults() {
        let d = dir("none");
        crate::services::init(windows::Win32::Foundation::HWND::default(), true);
        let st = Startup::new(StartFake::new());
        let mut obs = bu_obs::fake::FakeOs::new(&d.0);
        assert_eq!(crate::addons::take_over(&st, &mut obs), None);
        assert!(crate::services::with(|s| crate::undo::records(&s.store, None).is_empty()).unwrap());
        crate::addons::set_for("obs", true, true);
        assert_eq!(crate::obs::settings(), Some(Settings::default()));
        crate::addons::set_for("obs", false, true);
        crate::services::shutdown();
    }

    /// No settings file next to its exe: its entry still goes off and its copy closes; ours keeps its defaults.
    #[test]
    fn missing_ini_turns_it_off_and_keeps_our_defaults() {
        let d = dir("noini");
        let (st, mut obs, _) = pc(&d.0, true, None, true);
        crate::services::init(windows::Win32::Foundation::HWND::default(), true);
        let line = crate::addons::take_over(&st, &mut obs);
        assert_eq!(line.as_deref(), Some("Took over NotificationsForOBS: its startup turned off (undo in Settings)"));
        assert_eq!(flag(&st).map(|b| b[0]), Some(3), "a missing flag = on: switched off");
        assert!(obs.with(|s| s.other.is_none()));
        assert!(!saved_settings(), "nothing imported");
        crate::addons::set_for("obs", true, true);
        assert_eq!(crate::obs::settings(), Some(Settings::default()));
        crate::addons::set_for("obs", false, true);
        crate::services::shutdown();
    }

    /// The Run entry points to an exe that is gone: skipped (no change, no line); an original that neither starts with
    /// Windows nor runs is left alone too.
    #[test]
    fn a_gone_exe_or_an_unused_original_is_skipped() {
        let d = dir("gone");
        write_ini(&d.0);
        crate::services::init(windows::Win32::Foundation::HWND::default(), true);
        let (st, mut obs, _) = pc(&d.0, false, None, true);
        assert_eq!(crate::addons::take_over(&st, &mut obs), None);
        assert_eq!(flag(&st), None);
        assert!(obs.with(|s| s.other_closed.is_empty()));
        let (st, mut obs, _) = pc(&d.0, true, Some(&[3, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]), false);
        assert_eq!(crate::addons::take_over(&st, &mut obs), None);
        assert_eq!(flag(&st).map(|b| b[4]), Some(1), "its flag untouched");
        assert!(!saved_settings());
        crate::services::shutdown();
    }

    /// Its Quit hangs: that process is ended - only while it runs from the entry's exe; a copy from another folder only
    /// gets the Quit.
    #[test]
    fn a_hanging_original_is_ended_only_from_its_own_exe() {
        let d = dir("stuck");
        let (st, mut obs, path) = pc(&d.0, true, None, true);
        obs.with(|s| s.other_stuck = true);
        let f = find(&st, &obs).unwrap();
        let done = take_over(&st, &mut obs, &f, &mut |_, _, _, _| {});
        assert!(done.closed);
        obs.with(|s| assert!(s.other_closed == [PID] && s.other_ended == [PID] && s.other.is_none()));
        let (st, mut obs, _) = pc(&d.0, true, None, false);
        let other = d.0.join("elsewhere").join("NotificationsForOBS.exe");
        obs.with(|s| {
            s.other = Some((PID, other.clone()));
            s.other_stuck = true;
        });
        let f = find(&st, &obs).unwrap();
        assert_ne!(f.running.as_ref().map(|r| &r.1), Some(&path));
        let done = take_over(&st, &mut obs, &f, &mut |_, _, _, _| {});
        assert!(!done.closed);
        obs.with(|s| assert!(s.other_closed == [PID] && s.other_ended.is_empty() && s.other.is_some()));
    }

    /// The line fits the toast (one line, inside the window with room at the sides).
    #[test]
    fn the_line_fits_the_toast() {
        let g = crate::gfx::Gfx::new(1.0);
        let font = crate::gfx::Font::new(12.0, 600).ls(0);
        for imported in [false, true] {
            for turned_off in [false, true] {
                let t = Done { imported, turned_off, closed: true }.line();
                let w = g.text_width(&t, font) + 26.0;
                assert!(w <= crate::ui::WIN_W - 32.0, "{t}: {w} px");
            }
        }
    }

    /// READ-ONLY proof on this PC (run once by hand: `cargo test -p bu-app takeover::tests::dry_run -- --ignored
    /// --nocapture`): what Get WOULD take over - nothing is written, closed or switched.
    #[test]
    #[ignore]
    fn dry_run_on_this_pc() {
        let st = Startup::new(bu_startup::real::RealOs::new());
        let obs = bu_obs::real::RealOs::new();
        match find(&st, &obs) {
            None => println!("DRY RUN: no NotificationsForOBS Run entry (or its exe is gone): Get would take nothing over"),
            Some(f) => {
                println!("DRY RUN: Run value {:?} = {}", f.value_name, f.command);
                println!("DRY RUN: exe {} (Startup tab name {:?})", f.exe.display(), f.label);
                println!("DRY RUN: starts with Windows now: {} (would be switched off: {})", f.on, f.on);
                println!("DRY RUN: running: {:?} (would be closed: {})", f.running, f.running.is_some());
                println!("DRY RUN: ini {:?}", f.ini);
                println!("DRY RUN: settings parsed {:?}", f.settings);
                println!("DRY RUN: would take over: {}", f.active());
            }
        }
    }
}
