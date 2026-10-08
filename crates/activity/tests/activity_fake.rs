//! Every Activity row against the fake clock + in-memory store (and one real file round-trip in the scratch folder).

use bu_activity::activity::{Activity, IDLE_AFTER_MS, SAVE_EVERY_MS};
use bu_activity::clock::{day_label, Date, Stamp, DAY_MS};
use bu_activity::games::GameRules;
use bu_activity::store::{day_from_text, day_to_text, settings_from_text, settings_to_text, DayData, StoredSettings};
use bu_activity::views::{fmt_hm, fmt_uptime, summary, PRIVACY_LINE};
use bu_activity::{FgApp, FileStore, MemStore, Store};

const MIN: i64 = 60_000;
const H: i64 = 60 * MIN;
const CHROME: &str = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
const VALO: &str = r"C:\Riot Games\VALORANT\live\VALORANT.exe";
const RIOT: &str = r"C:\Riot Games\Riot Client\RiotClientServices.exe";

/// Thu 8 Oct 2026 11:24 local, CEST (+120).
fn t0() -> Stamp {
    let day = Date { y: 2026, m: 10, d: 8 }.to_day();
    Stamp { unix_ms: day * DAY_MS + (11 * 60 + 24) * MIN - 120 * MIN, offset_min: 120 }
}

fn roots() -> Vec<String> {
    vec![r"C:\Riot Games\VALORANT".into(), r"D:\SteamLibrary\steamapps\common".into()]
}

fn on() -> Activity<MemStore> {
    let mut a = Activity::new(MemStore::default(), roots(), t0());
    a.set_on(true, t0()).unwrap();
    a
}

fn chrome() -> Option<FgApp> {
    Some(FgApp::new(CHROME, "Google Chrome"))
}
fn valo() -> Option<FgApp> {
    Some(FgApp::new(VALO, "VALORANT"))
}

#[test]
fn dates_weekdays_labels() {
    assert_eq!(Date::from_day(0), Date { y: 1970, m: 1, d: 1 });
    assert_eq!(Date { y: 2026, m: 10, d: 8 }.to_day(), 20_734);
    assert_eq!(Date::from_day(20_734).iso(), "2026-10-08");
    assert_eq!(day_label(20_734), "Thu 8 Oct");
    assert_eq!(day_label(Date { y: 2026, m: 9, d: 30 }.to_day()), "Wed 30 Sep", "the drawing's 'since Wed 30 Sep'");
    assert_eq!(t0().local_minute(), 11 * 60 + 24);
    for z in [-1000, 0, 59, 10_000, 20_734, 100_000] {
        assert_eq!(Date::from_day(z).to_day(), z);
    }
    assert_eq!(Date::parse_iso("2026-13-01"), None);
}

#[test]
fn off_by_default_counts_nothing() {
    let mut a = Activity::new(MemStore::default(), roots(), t0());
    assert!(!a.is_on());
    a.foreground(chrome(), t0());
    let s = summary(&mut a, t0().plus_ms(H), 0);
    assert_eq!(s.screen_today_ms, 0);
    assert_eq!(s.status, "Most-used apps, game time and uptime · off");
    assert!(PRIVACY_LINE.contains("nothing leaves your PC"));
}

#[test]
fn counts_the_app_in_front_and_switches() {
    let mut a = on();
    a.foreground(chrome(), t0());
    a.foreground(valo(), t0().plus_ms(30 * MIN));
    a.foreground(chrome(), t0().plus_ms(90 * MIN));
    let s = summary(&mut a, t0().plus_ms(100 * MIN), 0);
    assert_eq!(s.screen_today_ms, (100 * MIN) as u64);
    assert_eq!(s.screen_text, "1 h 40 m · today · since 11:24");
    assert_eq!(s.games_today_ms, (60 * MIN) as u64);
    assert_eq!(s.games_text, "1 h 0 m · today · 1 h 0 m in 7 days");
    assert_eq!(s.status, "Counting since Thu 8 Oct");
    assert_eq!(s.today.iter().map(|r| (r.name.as_str(), r.ms, r.game)).collect::<Vec<_>>(), vec![
        ("VALORANT", (60 * MIN) as u64, true),
        ("Google Chrome", (40 * MIN) as u64, false)
    ]);
}

#[test]
fn games_from_launcher_folders_not_the_launcher_and_right_click_fix() {
    let mut a = on();
    assert!(a.rules.is_game(VALO));
    assert!(!a.rules.is_game(RIOT), "Riot Client is not inside a game folder");
    assert!(a.rules.is_game(r"d:\steamlibrary\steamapps\common\Rocket League\RocketLeague.exe"), "case and drive don't matter");
    assert!(!a.rules.is_game(r"D:\SteamLibrary\steamapps\commonX\x.exe"), "only inside the folder");
    assert_eq!(a.set_game(CHROME, Some(true)).unwrap(), None);
    assert!(a.rules.is_game(CHROME), "Count as a game");
    assert_eq!(a.set_game(VALO, Some(false)).unwrap(), None);
    assert!(!a.rules.is_game(VALO), "Not a game");
    assert_eq!(a.set_game(VALO, None).unwrap(), Some(false), "undo: back to automatic");
    assert!(a.rules.is_game(VALO));
    // the choice is retroactive in the views (stored per app, not per day)
    a.foreground(chrome(), t0());
    let s = summary(&mut a, t0().plus_ms(10 * MIN), 0);
    assert!(s.today[0].game);
    assert!(a.settings().game.get(&CHROME.to_lowercase()) == Some(&true), "stored");
}

#[test]
fn dont_count_this_app_and_undo() {
    let mut a = on();
    a.foreground(chrome(), t0());
    a.set_counted(CHROME, false, t0().plus_ms(10 * MIN)).unwrap();
    let s = summary(&mut a, t0().plus_ms(60 * MIN), 0);
    assert_eq!(s.screen_today_ms, 0, "hidden, and not counted from then on");
    a.set_counted(CHROME, true, t0().plus_ms(60 * MIN)).unwrap();
    let s = summary(&mut a, t0().plus_ms(70 * MIN), 0);
    assert_eq!(s.screen_today_ms, (20 * MIN) as u64, "10 min before + 10 min after; the 50 min skipped stay uncounted");
}

#[test]
fn idle_is_cut_at_the_last_input_and_never_while_a_game_is_in_front() {
    let mut a = on();
    a.foreground(chrome(), t0());
    // last input at +20 min, noticed at +25
    a.went_idle(t0().plus_ms(20 * MIN), t0().plus_ms(20 * MIN + IDLE_AFTER_MS));
    assert!(a.is_idle());
    a.back_from_idle(t0().plus_ms(60 * MIN));
    let s = summary(&mut a, t0().plus_ms(70 * MIN), 0);
    assert_eq!(s.screen_today_ms, (30 * MIN) as u64, "20 min + 10 min; the 40 min away not counted");
    // a game: the idle rule doesn't apply (controller / cutscenes)
    a.foreground(valo(), t0().plus_ms(70 * MIN));
    assert!(!a.idle_applies());
    a.went_idle(t0().plus_ms(70 * MIN), t0().plus_ms(80 * MIN));
    assert!(!a.is_idle());
}

#[test]
fn a_game_coming_to_the_front_ends_idle() {
    let mut a = on();
    a.foreground(chrome(), t0());
    a.went_idle(t0().plus_ms(MIN), t0().plus_ms(6 * MIN));
    a.foreground(valo(), t0().plus_ms(10 * MIN));
    assert!(!a.is_idle());
    let s = summary(&mut a, t0().plus_ms(20 * MIN), 0);
    assert_eq!(s.games_today_ms, (10 * MIN) as u64);
}

#[test]
fn lock_and_sleep_are_not_counted_and_save() {
    let mut a = on();
    a.foreground(chrome(), t0());
    a.locked(true, t0().plus_ms(10 * MIN));
    let writes = a.store().writes;
    assert!(writes >= 1, "lock saves");
    a.locked(false, t0().plus_ms(30 * MIN));
    a.asleep(true, t0().plus_ms(40 * MIN));
    assert!(a.store().writes > writes, "sleep saves");
    a.asleep(false, t0().plus_ms(8 * H));
    let s = summary(&mut a, t0().plus_ms(8 * H + 5 * MIN), 0);
    assert_eq!(s.screen_today_ms, (25 * MIN) as u64);
}

#[test]
fn midnight_splits_the_span_into_two_days() {
    // 23:30 local, chrome until 00:45 the next day
    let start = Stamp { unix_ms: t0().unix_ms + (12 * 60 + 6) * MIN, offset_min: 120 };
    assert_eq!(start.local_minute(), 23 * 60 + 30);
    let mut a = Activity::new(MemStore::default(), roots(), start);
    a.set_on(true, start).unwrap();
    a.foreground(chrome(), start);
    let end = start.plus_ms(75 * MIN);
    let s = summary(&mut a, end, 0);
    assert_eq!(s.screen_today_ms, (45 * MIN) as u64, "today = after midnight");
    assert_eq!(s.screen_text, "45 m · today · since 00:00");
    let cols = &s.last7;
    assert_eq!(cols.len(), 7);
    assert_eq!(cols[5].total_ms, (30 * MIN) as u64, "yesterday got 30 min");
    assert!(cols[6].today && !cols[5].today);
    assert_eq!(cols[5].label, "Thu 8 Oct");
    assert_eq!(cols[5].hover, "Thu 8 Oct · 30 m · games 0 m");
}

#[test]
fn seven_days_columns_and_most_used_week() {
    let mut store = MemStore::default();
    let today = t0().day();
    for back in 1..=6 {
        let mut d = DayData { first_minute: Some(600), ..Default::default() };
        d.apps.insert(VALO.to_lowercase(), bu_activity::store::AppDay { name: "VALORANT".into(), ms: (2 * H) as u64 });
        d.apps.insert(CHROME.to_lowercase(), bu_activity::store::AppDay { name: "Google Chrome".into(), ms: H as u64 });
        store.days.insert(Date::from_day(today - back), d);
    }
    // an old day beyond 7 days is not shown
    store.days.insert(Date::from_day(today - 9), DayData::default());
    let mut a = Activity::new(store, roots(), t0());
    a.set_on(true, t0()).unwrap();
    a.foreground(chrome(), t0());
    let s = summary(&mut a, t0().plus_ms(30 * MIN), 0);
    assert_eq!(s.last7.iter().map(|c| c.total_ms / MIN as u64).collect::<Vec<_>>(), vec![180, 180, 180, 180, 180, 180, 30]);
    assert_eq!(s.last7[0].games_ms, (2 * H) as u64);
    assert_eq!(s.games_week_ms, (12 * H) as u64);
    assert_eq!(s.games_text, "0 m · today · 12 h 0 m in 7 days");
    assert_eq!(s.week[0].name, "VALORANT");
    assert_eq!(s.week[1].ms, (6 * H + 30 * MIN) as u64);
}

#[test]
fn saves_at_most_once_a_minute_while_switching() {
    let mut a = on();
    let w0 = a.store().writes;
    a.foreground(chrome(), t0());
    for i in 1..30 {
        a.foreground(if i % 2 == 0 { chrome() } else { valo() }, t0().plus_ms(i * 1000));
    }
    let w1 = a.store().writes;
    assert!(w1 - w0 <= 2, "30 switches in 30 s: {} saves", w1 - w0);
    a.foreground(chrome(), t0().plus_ms(SAVE_EVERY_MS + 1000));
    assert!(a.store().writes > w1, "a minute later it saves");
}

#[test]
fn switching_off_saves_and_keeps_since() {
    let mut a = on();
    a.foreground(chrome(), t0());
    a.set_on(false, t0().plus_ms(10 * MIN)).unwrap();
    let day = a.store().days.get(&Date::from_day(t0().day())).cloned().unwrap();
    assert_eq!(day.apps[&CHROME.to_lowercase()].ms, (10 * MIN) as u64);
    a.foreground(valo(), t0().plus_ms(20 * MIN));
    a.set_on(true, t0().plus_ms(3 * DAY_MS)).unwrap();
    assert_eq!(a.settings().since, Some(Date { y: 2026, m: 10, d: 8 }), "Counting since = first day");
}

#[test]
fn uptime_tile() {
    assert_eq!(fmt_uptime((27 * H) as u64), "1 d 3 h");
    assert_eq!(fmt_uptime((5 * H + 12 * MIN) as u64), "5 h 12 m");
    assert_eq!(fmt_hm(59_999), "0 m");
    let mut a = on();
    // booted Mon 18:02 (3 days + 17 h 22 m before Thu 11:24)
    let up = (3 * 24 * H + 17 * H + 22 * MIN) as u64;
    let s = summary(&mut a, t0(), up);
    assert_eq!(s.uptime_text, "3 d 17 h · since Sun 18:02");
    let s = summary(&mut a, t0(), (10 * 24 * H) as u64);
    assert_eq!(s.uptime_text, "10 d 0 h · since 28 Sep 11:24", "older than a week: the date");
}

#[test]
fn file_formats_round_trip_and_bad_files_are_errors() {
    let mut d = DayData { first_minute: Some(11 * 60 + 24), ..Default::default() };
    d.apps.insert(CHROME.to_lowercase(), bu_activity::store::AppDay { name: "Google\tChrome".into(), ms: 123_456 });
    let t = day_to_text(&d);
    assert!(t.starts_with("bu-activity day 1\nfirst\t11:24\napp\t123456\tGoogle Chrome\t"));
    let back = day_from_text(&t).unwrap();
    assert_eq!(back.apps.values().next().unwrap().ms, 123_456);
    assert!(day_from_text("hello").is_err());
    assert!(day_from_text("bu-activity day 1\napp\tx\ty\tz").is_err());
    let s = StoredSettings { on: true, since: Some(Date { y: 2026, m: 9, d: 30 }), skip: vec!["c:\\x.exe".into()], ..Default::default() };
    assert_eq!(settings_from_text(&settings_to_text(&s)).unwrap(), s);
}

#[test]
fn bad_day_file_is_a_problem_not_a_crash() {
    struct Bad;
    impl Store for Bad {
        fn load_day(&mut self, _: Date) -> bu_activity::Result<Option<DayData>> {
            Err(bu_activity::ActivityError::BadData { path: "x".into(), msg: "broken".into() })
        }
        fn save_day(&mut self, _: Date, _: &DayData) -> bu_activity::Result<()> {
            Err(bu_activity::ActivityError::File { path: "x".into(), msg: "disk full".into() })
        }
        fn load_settings(&mut self) -> bu_activity::Result<StoredSettings> {
            Ok(StoredSettings::default())
        }
        fn save_settings(&mut self, _: &StoredSettings) -> bu_activity::Result<()> {
            Ok(())
        }
    }
    let mut a = Activity::new(Bad, roots(), t0());
    assert!(!a.problems.is_empty());
    a.set_on(true, t0()).unwrap();
    a.foreground(chrome(), t0());
    assert!(a.save(t0().plus_ms(MIN)).is_err(), "a failed save is reported");
    assert_eq!(summary(&mut a, t0().plus_ms(2 * MIN), 0).screen_today_ms, (2 * MIN) as u64, "kept in memory");
}

#[test]
fn games_rules_roots_cleanup() {
    let r = GameRules::new(vec![r"C:\XboxGames\".into(), "C:".into(), "".into()]);
    assert_eq!(r.roots, vec![r"c:\xboxgames\".to_string()], "too-short roots (a bare drive) are ignored");
}

/// The real files, in the scratch folder this test creates and removes.
#[test]
fn file_store_in_scratch_folder() {
    let dir = std::path::PathBuf::from(r"C:\BoylerUtilities-scratch\lane-j").join(format!("activity-{}", std::process::id()));
    if !dir.parent().is_some_and(|p| p.parent().is_some_and(|q| q.exists())) {
        eprintln!("scratch parent missing: skipped");
        return;
    }
    let mut a = Activity::new(FileStore::new(&dir), roots(), t0());
    a.set_on(true, t0()).unwrap();
    a.foreground(chrome(), t0());
    a.foreground(valo(), t0().plus_ms(10 * MIN));
    a.save(t0().plus_ms(70 * MIN)).unwrap();
    let files = FileStore::new(&dir).days_on_disk();
    assert_eq!(files.len(), 1);
    assert!(files[0].1 < 300, "a small file: {} bytes", files[0].1);
    // a fresh start reads it back
    let mut b = Activity::new(FileStore::new(&dir), roots(), t0().plus_ms(80 * MIN));
    assert!(b.is_on());
    let s = summary(&mut b, t0().plus_ms(80 * MIN), 0);
    assert_eq!(s.screen_today_ms, (70 * MIN) as u64);
    assert_eq!(s.games_today_ms, (60 * MIN) as u64);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!dir.exists());
}

#[cfg(windows)]
#[test]
fn launcher_file_parsers() {
    use bu_activity::real::{epic_install_location, epic_is_application, steam_library_paths};
    let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\Program Files (x86)\\Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\SteamLibrary\"\n\t}\n}";
    assert_eq!(steam_library_paths(vdf), vec![r"C:\Program Files (x86)\Steam".to_string(), r"D:\SteamLibrary".to_string()]);
    let game = "{\n\t\"bIsApplication\": true,\n\t\"InstallLocation\": \"C:\\Program Files\\Epic Games\\rocketleague\",\n}";
    let engine = "{\n\t\"bIsApplication\": false,\n\t\"InstallLocation\": \"C:\\Program Files\\Epic Games\\UE_5.8\",\n}";
    assert!(epic_is_application(game) && !epic_is_application(engine), "Unreal Engine installs are not games");
    assert_eq!(epic_install_location(game).as_deref(), Some(r"C:\Program Files\Epic Games\rocketleague"));
    assert!(!epic_is_application("{}"));
}

/// A store whose day files fail to load until `readable` is set (a passing virus-scanner / backup lock).
#[derive(Default)]
struct Flaky {
    inner: MemStore,
    readable: bool,
}
impl Store for Flaky {
    fn load_day(&mut self, d: Date) -> bu_activity::Result<Option<DayData>> {
        if !self.readable {
            return Err(bu_activity::ActivityError::File { path: d.iso(), msg: "locked".into() });
        }
        self.inner.load_day(d)
    }
    fn save_day(&mut self, d: Date, day: &DayData) -> bu_activity::Result<()> {
        self.inner.save_day(d, day)
    }
    fn load_settings(&mut self) -> bu_activity::Result<StoredSettings> {
        self.inner.load_settings()
    }
    fn save_settings(&mut self, s: &StoredSettings) -> bu_activity::Result<()> {
        self.inner.save_settings(s)
    }
}

#[test]
fn an_unreadable_day_is_never_written_over_and_merged_once_it_reads() {
    let mut store = Flaky::default();
    let mut old = DayData { first_minute: Some(8 * 60), ..Default::default() };
    old.apps.insert(CHROME.to_lowercase(), bu_activity::store::AppDay { name: "Google Chrome".into(), ms: (3 * H) as u64 });
    let date = Date::from_day(t0().day());
    store.inner.days.insert(date, old.clone());
    let mut a = Activity::new(store, roots(), t0());
    assert!(!a.problems.is_empty(), "the lock is reported");
    a.set_on(true, t0()).unwrap();
    a.foreground(chrome(), t0());
    assert!(a.save(t0().plus_ms(10 * MIN)).is_err(), "still locked: not saved, reported");
    assert_eq!(a.store().inner.days[&date], old, "the file was NOT written over");
    // the lock is gone: the next save reads the file and ADDS the new time to it
    a.store_mut().readable = true;
    a.save(t0().plus_ms(20 * MIN)).unwrap();
    let saved = &a.store().inner.days[&date];
    assert_eq!(saved.apps[&CHROME.to_lowercase()].ms, (3 * H + 20 * MIN) as u64, "old 3 h + the 20 new minutes");
    assert_eq!(saved.first_minute, Some(8 * 60));
}

#[test]
fn bad_files_stay_byte_identical_in_scratch() {
    let dir = std::path::PathBuf::from(r"C:\BoylerUtilities-scratch\lane-j").join(format!("bad-{}", std::process::id()));
    if !std::path::Path::new(r"C:\BoylerUtilities-scratch").exists() {
        eprintln!("scratch folder missing: skipped");
        return;
    }
    std::fs::create_dir_all(&dir).unwrap();
    // removed even when an assert below fails
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let day = dir.join(format!("{}.tsv", Date::from_day(t0().day()).iso()));
    let settings = dir.join("settings.tsv");
    std::fs::write(&day, b"not a day file \xff\x00").unwrap();
    std::fs::write(&settings, b"garbage settings").unwrap();
    let mut a = Activity::new(FileStore::new(&dir), roots(), t0());
    assert_eq!(a.problems.len(), 2, "{:?}", a.problems);
    let _ = a.set_on(true, t0()); // refused to write settings.tsv (it couldn't be read), counting still on in memory
    assert!(a.is_on());
    a.foreground(chrome(), t0());
    assert!(a.save(t0().plus_ms(30 * MIN)).is_err());
    assert!(a.set_game(CHROME, Some(true)).is_err(), "settings.tsv is not written over");
    assert_eq!(std::fs::read(&day).unwrap(), b"not a day file \xff\x00", "day file byte-identical");
    assert_eq!(std::fs::read(&settings).unwrap(), b"garbage settings", "settings byte-identical");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scratch_store_guard() {
    let root = std::path::Path::new(r"C:\BoylerUtilities-scratch\lane-j");
    // only this lane's own folder is made, and only inside an existing scratch folder
    if root.parent().is_some_and(|p| p.exists()) {
        std::fs::create_dir_all(root).ok();
    }
    if !root.exists() {
        eprintln!("scratch folder missing: skipped");
        return;
    }
    assert!(FileStore::in_scratch(root.join("x"), root).is_ok());
    assert!(FileStore::in_scratch(r"c:/boylerutilities-scratch/lane-j/y", root).is_ok(), "case + slashes");
    for bad in [
        r"C:\BoylerUtilities-scratch\lane-j\..\..\..\AppData\Local\x",
        r"C:\Users\user\AppData\Local\BoylerUtilities\activity",
        r"C:\BoylerUtilities-scratch\lane-jX\y",
        r"C:\BoylerUtilities-scratch\lane-j",
    ] {
        assert!(matches!(FileStore::in_scratch(bad, root), Err(bu_activity::ActivityError::Refused(_))), "{bad}");
    }
}
