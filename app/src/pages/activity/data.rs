//! Where the Activity tab's numbers come from (crates/activity, `bu-activity`):
//! * `Fake` (every test copy, unit tests): the crate's counter over an in-memory store holding the drawing's made-up
//!   week (Wed 30 Sep - Tue 6 Oct 2026, the drawing's ACTA + WEEK tables), a fixed "now" (Tue 6 Oct 21:37) and uptime.
//! * `Real`: the crate's event-driven watcher (`bu_activity::watch::Watcher`) counting into
//!   `%LOCALAPPDATA%\BoylerUtilities\activity\`. The watcher is process-wide (it keeps counting while the menu is closed:
//!   the switch means "count my activity", not "while this tab is open"); the page itself only reads from it.
//!   It runs only while the switch is on: switching off saves and ends its thread.
//! * `ReadOnly` (a `--real-read` test copy): the real files read, nothing written, no watcher, the switch does nothing.

use std::sync::Mutex;

use bu_activity::clock::{Date, Stamp, DAY_MS};
use bu_activity::store::{DayData, StoredSettings};
use bu_activity::views::{summary, Summary};
use bu_activity::{Activity as Counter, ActivityOs, Clock, FileStore, MemStore, Result, Store};

/// What the page can ask of its source.
pub trait Src {
    fn is_on(&self) -> bool;
    /// The switch. Errors are shown in the card's line.
    fn set_on(&mut self, on: bool) -> Result<()>;
    /// Everything the tab shows (only asked while the switch is on).
    fn summary(&mut self) -> Summary;
    /// Right-click "Count as a game" / "Not a game".
    fn set_game(&mut self, path: &str, game: bool) -> Result<()>;
    /// Right-click "Don't count this app".
    fn dont_count(&mut self, path: &str) -> Result<()>;
    /// test hook text: is the counter running (a watcher / a fake that counts)
    fn counting(&self) -> bool;
}

// ------------------------------------------------------------------------------------------------ the drawing's week

const MIN: u64 = 60_000;

/// One app of the drawing's ACTA table: (exe path, name, minutes today).
pub struct DemoApp {
    pub path: &'static str,
    pub name: &'static str,
    pub today: u64,
    /// minutes on Wed 30 Sep .. Mon 5 Oct (today comes from `today`)
    pub days: [u64; 6],
}

/// The drawing's ACTA (minutes today, minutes in 7 days) spread over the drawing's WEEK columns (total, games per day):
/// every column total, every games total and every app's 7-day total is the drawing's number.
pub const DEMO: [DemoApp; 11] = [
    DemoApp { path: r"C:\Riot Games\VALORANT\live\VALORANT.exe", name: "VALORANT", today: 161, days: [110, 140, 185, 44, 165, 40] },
    DemoApp { path: r"C:\Program Files\Google\Chrome\Application\chrome.exe", name: "Google Chrome", today: 112, days: [73, 85, 110, 128, 104, 88] },
    DemoApp { path: r"C:\Users\Public\AppData\Local\Discord\app-1.0.9163\Discord.exe", name: "Discord", today: 47, days: [33, 38, 50, 58, 46, 40] },
    DemoApp { path: r"C:\Program Files\obs-studio\bin\64bit\obs64.exe", name: "OBS Studio", today: 22, days: [13, 16, 20, 24, 19, 16] },
    DemoApp { path: r"C:\Program Files\Epic Games\rocketleague\Binaries\Win64\RocketLeague.exe", name: "Rocket League", today: 0, days: [0, 0, 0, 206, 0, 0] },
    DemoApp { path: r"C:\Program Files (x86)\Steam\steam.exe", name: "Steam", today: 12, days: [6, 8, 10, 12, 9, 8] },
    DemoApp { path: r"C:\Windows\explorer.exe", name: "File Explorer", today: 9, days: [6, 7, 9, 11, 9, 7] },
    DemoApp { path: r"C:\Users\Public\AppData\Roaming\Spotify\Spotify.exe", name: "Spotify", today: 6, days: [4, 5, 7, 8, 6, 5] },
    DemoApp { path: r"C:\Windows\ImmersiveControlPanel\SystemSettings.exe", name: "Settings", today: 4, days: [2, 3, 4, 4, 3, 3] },
    DemoApp { path: r"C:\Windows\System32\notepad.exe", name: "Notepad", today: 3, days: [2, 2, 3, 3, 2, 2] },
    DemoApp { path: r"C:\Program Files\Boyler Utilities\BoylerUtilities.exe", name: "Boyler Utilities", today: 2, days: [1, 1, 2, 2, 2, 1] },
];

/// The drawing's game launcher folders (VALORANT and Rocket League are found as games by their folder).
pub const DEMO_ROOTS: [&str; 2] = [r"C:\Riot Games\VALORANT", r"C:\Program Files\Epic Games\rocketleague"];

/// The drawing's "now": Tue 6 Oct 2026 21:37, CEST (+120).
pub fn demo_now() -> Stamp {
    let day = Date { y: 2026, m: 10, d: 6 }.to_day();
    Stamp { unix_ms: day * DAY_MS + (21 * 60 + 37) as i64 * 60_000 - 120 * 60_000, offset_min: 120 }
}

/// "1 d 3 h · since Mon 18:02": the PC started Mon 5 Oct 18:02.
pub const DEMO_UPTIME_MS: u64 = (27 * 60 + 35) * MIN;

/// The drawing's store: the week + settings (off; first switched on Wed 30 Sep).
pub fn demo_store() -> MemStore {
    let mut st = MemStore::default();
    let today = demo_now().day();
    for back in 0..7i64 {
        let day = today - 6 + back;
        let mut d = DayData { first_minute: Some(if back == 6 { 11 * 60 + 24 } else { 9 * 60 }), ..DayData::default() };
        for a in &DEMO {
            let m = if back == 6 { a.today } else { a.days[back as usize] };
            if m > 0 {
                d.apps.insert(bu_activity::games::key_of(a.path), bu_activity::store::AppDay { name: a.name.into(), ms: m * MIN });
            }
        }
        st.days.insert(Date::from_day(day), d);
    }
    st.settings = StoredSettings { on: false, since: Some(Date { y: 2026, m: 9, d: 30 }), ..StoredSettings::default() };
    st
}

// ------------------------------------------------------------------------------------------------ fake

pub struct Fake {
    pub a: Counter<MemStore>,
    pub now: Stamp,
    pub uptime_ms: u64,
}

impl Fake {
    pub fn demo() -> Fake {
        let roots = DEMO_ROOTS.iter().map(|r| r.to_string()).collect();
        Fake { a: Counter::new(demo_store(), roots, demo_now()), now: demo_now(), uptime_ms: DEMO_UPTIME_MS }
    }
    /// tests: an app comes to the front / time passes
    #[cfg(test)]
    pub fn front(&mut self, path: &str, name: &str) {
        self.a.foreground(Some(bu_activity::FgApp::new(path, name)), self.now);
    }
    #[cfg(test)]
    pub fn advance_min(&mut self, m: i64) {
        self.now = self.now.plus_ms(m * 60_000);
    }
}

impl Src for Fake {
    fn is_on(&self) -> bool {
        self.a.is_on()
    }
    fn set_on(&mut self, on: bool) -> Result<()> {
        self.a.set_on(on, self.now)
    }
    fn summary(&mut self) -> Summary {
        summary(&mut self.a, self.now, self.uptime_ms)
    }
    fn set_game(&mut self, path: &str, game: bool) -> Result<()> {
        self.a.set_game(path, Some(game)).map(|_| ())
    }
    fn dont_count(&mut self, path: &str) -> Result<()> {
        self.a.set_counted(path, false, self.now)
    }
    fn counting(&self) -> bool {
        self.a.is_on()
    }
}

// ------------------------------------------------------------------------------------------------ real

/// The one watcher of the app (process-wide): Some while "Count my activity" is on.
static WATCHER: Mutex<Option<bu_activity::watch::Watcher>> = Mutex::new(None);

fn watcher() -> std::sync::MutexGuard<'static, Option<bu_activity::watch::Watcher>> {
    WATCHER.lock().unwrap_or_else(|p| p.into_inner())
}

fn now() -> Stamp {
    bu_activity::SystemClock.now()
}

fn data_dir() -> Result<std::path::PathBuf> {
    bu_activity::RealOs
        .data_dir()
        .ok_or_else(|| bu_activity::ActivityError::Os { context: "no %LOCALAPPDATA% folder".into(), code: 0 })
}

/// Starts the watcher when the stored switch is on and it isn't running (the app was restarted). Reads settings.tsv
/// only (one small file); the counter itself (7 day files + the launcher folders) loads only when the switch is on.
/// Meant for the app's start too (Order 014 item 2's `Page::start`, not merged yet - until then: when the tab opens).
pub fn resume_if_on() -> Result<bool> {
    if watcher().is_some() {
        return Ok(true);
    }
    let mut store = FileStore::new(data_dir()?);
    if !store.load_settings()?.on {
        return Ok(false);
    }
    start()?;
    Ok(true)
}

fn start() -> Result<()> {
    let mut os = bu_activity::RealOs;
    let t = now();
    let a = Counter::new(FileStore::new(data_dir()?), os.game_roots(), t);
    let w = bu_activity::watch::Watcher::start(a)?;
    w.with(|a| a.set_on(true, t))?;
    w.poke();
    *watcher() = Some(w);
    Ok(())
}

pub struct Real {
    on: bool,
}

impl Real {
    /// The tab opened: is it on (and counting)?
    pub fn open() -> (Real, Option<String>) {
        match resume_if_on() {
            Ok(on) => (Real { on }, None),
            Err(e) => (Real { on: false }, Some(e.to_string())),
        }
    }
}

impl Src for Real {
    fn is_on(&self) -> bool {
        self.on
    }
    fn set_on(&mut self, on: bool) -> Result<()> {
        if on {
            if watcher().is_none() {
                start()?;
            }
        } else if let Some(w) = watcher().take() {
            // saves what was counted and stops; dropping it ends the thread (it saves once more on WM_QUIT)
            w.with(|a| a.set_on(false, now()))?;
        }
        self.on = on;
        Ok(())
    }
    fn summary(&mut self) -> Summary {
        let up = bu_activity::RealOs.uptime_ms();
        let g = watcher();
        match g.as_ref() {
            Some(w) => w.with(|a| summary(a, now(), up)),
            None => {
                drop(g);
                // not counting (should not be asked): what is on disk
                let mut a = Counter::new(FileStore::new(data_dir().unwrap_or_default()), Vec::new(), now());
                summary(&mut a, now(), up)
            }
        }
    }
    fn set_game(&mut self, path: &str, game: bool) -> Result<()> {
        let g = watcher();
        let w = g.as_ref().ok_or(bu_activity::ActivityError::Os { context: "not counting".into(), code: 0 })?;
        w.with(|a| a.set_game(path, Some(game)))?;
        w.poke();
        Ok(())
    }
    fn dont_count(&mut self, path: &str) -> Result<()> {
        let g = watcher();
        let w = g.as_ref().ok_or(bu_activity::ActivityError::Os { context: "not counting".into(), code: 0 })?;
        w.with(|a| a.set_counted(path, false, now()))?;
        w.poke();
        Ok(())
    }
    fn counting(&self) -> bool {
        watcher().is_some()
    }
}

// ------------------------------------------------------------------------------------------------ read-only

/// A `--real-read` test copy: the real files, nothing written, nothing started.
pub struct ReadOnly {
    a: Option<Counter<FileStore>>,
}

impl ReadOnly {
    pub fn open() -> ReadOnly {
        let a = data_dir().ok().and_then(|d| {
            let mut s = FileStore::new(d.clone());
            let on = s.load_settings().map(|x| x.on).unwrap_or(false);
            on.then(|| Counter::new(FileStore::new(d), bu_activity::RealOs.game_roots(), now()))
        });
        ReadOnly { a }
    }
}

impl Src for ReadOnly {
    fn is_on(&self) -> bool {
        self.a.is_some()
    }
    fn set_on(&mut self, _on: bool) -> Result<()> {
        Err(bu_activity::ActivityError::Refused("a read-only test copy changes nothing".into()))
    }
    fn summary(&mut self) -> Summary {
        let up = bu_activity::RealOs.uptime_ms();
        match &mut self.a {
            Some(a) => summary(a, now(), up),
            None => summary(&mut Counter::new(MemStore::default(), Vec::new(), now()), now(), up),
        }
    }
    fn set_game(&mut self, _p: &str, _g: bool) -> Result<()> {
        Err(bu_activity::ActivityError::Refused("a read-only test copy changes nothing".into()))
    }
    fn dont_count(&mut self, _p: &str) -> Result<()> {
        Err(bu_activity::ActivityError::Refused("a read-only test copy changes nothing".into()))
    }
    fn counting(&self) -> bool {
        false
    }
}
