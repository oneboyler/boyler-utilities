//! The Controller page's link to `bu-controller` (Order 015): the real Steam files (normal runs), Steam read-only
//! (`--real-read` test copies) or an in-memory FAKE Steam (every other test copy) - the fake holds the drawing's sample data
//! (Rocket League with the R4 = F5 / light-bar-red / dead-zone edits, a community layout, a shortcut; a DualSense Edge at
//! 82 %), built from Lane L's Steam-format fixtures (`crates/controller/tests/fixtures`, made-up values - A_015_01).

use std::path::PathBuf;

use bu_controller::fake::{FakePads, FakeSteam};
use bu_controller::layout::ActionSet;
use bu_controller::os::{Battery, Connection, PadInfo, PadOs, PadSource};
use bu_controller::real::{RealPads, RealSteam};
use bu_controller::service::Opened;
use bu_controller::{Change, ControllerService, Game, Layout, PadKind, PadView, Part, PrefSetting, Prefs, Result};

use crate::undo::{DefaultItem, Val};

/// The tab's Steam service, shared by the page and its worker (Order 047): made on the worker (finding Steam reads the
/// registry), locked by the worker for each read / write, by the page only for the change log and in tests. None = not
/// made yet.
pub type Shared = std::sync::Arc<std::sync::Mutex<Option<std::result::Result<Svc, String>>>>;

/// The service over the real or the fake Steam.
#[allow(clippy::large_enum_variant)] // the fake Steam holds the test switches (Order 085); one value per tab
pub enum Svc {
    Fake(ControllerService<FakeSteam>),
    Real(ControllerService<RealSteam>),
}

macro_rules! with {
    ($s:expr, $v:ident => $e:expr) => {
        match $s {
            Svc::Fake($v) => $e,
            Svc::Real($v) => $e,
        }
    };
}

impl Svc {
    pub fn games(&self, k: PadKind) -> Result<Vec<Game>> {
        with!(self, s => s.games(k))
    }
    /// Order 047: one read of the game list serves the whole view (`open_game` + `steam_layout_of`).
    pub fn open_game(&self, g: Game, k: PadKind) -> Result<Opened> {
        with!(self, s => s.open_game(g, k))
    }
    pub fn steam_layout_of(&self, o: &Opened) -> Result<Layout> {
        with!(self, s => s.steam_layout_of(o))
    }
    /// The game names are read again at the next question (the tab opened, Steam started).
    pub fn forget_names(&self) {
        with!(self, s => s.forget_names())
    }
    /// Tests: the fake Steam answers this slowly (ms per read); the real one is never slowed.
    pub fn set_fake_delay(&self, ms: u64) {
        if let Svc::Fake(s) = self {
            s.os().delay_ms.store(ms, std::sync::atomic::Ordering::Relaxed);
        }
    }
    pub fn view(&self, key: &str, k: PadKind, set: u32) -> Result<PadView> {
        with!(self, s => s.view(key, k, set))
    }
    pub fn action_sets(&self, key: &str, k: PadKind) -> Result<Vec<ActionSet>> {
        with!(self, s => s.action_sets(key, k))
    }
    pub fn steam_layout(&self, key: &str, k: PadKind) -> Result<Layout> {
        with!(self, s => s.steam_layout(key, k))
    }
    pub fn changed_parts(&self, key: &str, k: PadKind, set: u32) -> Result<Vec<Part>> {
        with!(self, s => s.changed_parts(key, k, set))
    }
    pub fn apply_all(&mut self, key: &str, k: PadKind, set: u32, c: &[Change]) -> Result<()> {
        with!(self, s => s.apply_all(key, k, set, c))
    }
    pub fn add_action_set(&mut self, key: &str, k: PadKind, from: u32, title: &str) -> Result<u32> {
        with!(self, s => s.add_action_set(key, k, from, title))
    }
    pub fn part_to_steam(&mut self, key: &str, k: PadKind, set: u32, p: Part) -> Result<()> {
        with!(self, s => s.part_to_steam(key, k, set, p))
    }
    pub fn layout_to_steam(&mut self, key: &str, k: PadKind) -> Result<()> {
        with!(self, s => s.layout_to_steam(key, k))
    }
    pub fn back_to_original(&mut self, key: &str, k: PadKind) -> Result<()> {
        with!(self, s => s.back_to_original(key, k))
    }
    pub fn has_original(&self, key: &str, k: PadKind) -> bool {
        with!(self, s => s.has_original(key, k))
    }
    pub fn game(&self, key: &str, k: PadKind) -> Result<Game> {
        with!(self, s => s.game(key, k))
    }
    pub fn is_original(&self, key: &str, k: PadKind) -> Result<bool> {
        with!(self, s => s.is_original(key, k))
    }
    pub fn is_steam_layout(&self, key: &str, k: PadKind) -> Result<bool> {
        with!(self, s => s.is_steam_layout(key, k))
    }
    pub fn preferences_are_original(&self, serial: &str) -> Result<bool> {
        with!(self, s => s.preferences_are_original(serial))
    }
    pub fn has_preferences_original(&self, serial: &str) -> bool {
        with!(self, s => s.has_preferences_original(serial))
    }
    pub fn preferences_to_original(&mut self, serial: &str) -> Result<()> {
        with!(self, s => s.preferences_to_original(serial))
    }

    // ---- the change log (Order 036)

    /// An item's value on the PC now: the game's files as they were before the app's first change ("orig"), Steam's own
    /// layout ("steam"), or the user's edits ("edits"); the controller's file as it was or not.
    pub fn log_val(&self, item: &Item) -> Option<Val> {
        match item {
            Item::Layout(k, key) => {
                if self.is_original(key, *k).ok()? {
                    Some(orig_val(false))
                } else if self.is_steam_layout(key, *k).ok()? {
                    Some(steam_val())
                } else {
                    Some(Val::new(EDITS, "your edits"))
                }
            }
            Item::Prefs(serial) => Some(if self.preferences_are_original(serial).ok()? { orig_val(false) } else { Val::new(EDITS, "your settings") }),
        }
    }

    /// "Steam’s layout" (the tab's Windows defaults): every game the app changed (+ the one shown) that isn't Steam's own
    /// layout now. One line per layout file (the DualSense and the Edge share theirs).
    pub fn steam_defaults(&self, shown: Option<(PadKind, &Game)>) -> Vec<DefaultItem> {
        let mut out: Vec<DefaultItem> = Vec::new();
        let mut add = |k: PadKind, g: &Game| {
            let item = Item::Layout(k, g.key.clone()).id();
            if out.iter().any(|d| d.item == item) {
                return;
            }
            if let Ok(false) = self.is_steam_layout(&g.key, k) {
                let raw = self.log_val(&Item::Layout(k, g.key.clone())).map(|v| v.raw).unwrap_or_else(|| EDITS.into());
                out.push(DefaultItem { item, label: layout_label(&g.name, k), now: Val::new(&raw, "your layout"), default: steam_val() });
            }
        };
        if let Some((k, g)) = shown {
            add(k, g);
        }
        for k in [PadKind::DualSenseEdge, PadKind::DualShock4, PadKind::Xbox] {
            for g in self.games(k).unwrap_or_default() {
                if self.has_original(&g.key, k) {
                    add(k, &g);
                }
            }
        }
        out
    }

    /// Puts one item to a value from the change log: "orig" = the bytes from before the app's first change (its backups,
    /// kept in the app's folder across restarts), "steam" = Steam's own layout.
    pub fn restore(&mut self, item: &Item, raw: &str) -> std::result::Result<(), String> {
        let r = match (item, raw) {
            (Item::Layout(k, key), ORIG) => self.back_to_original(key, *k),
            (Item::Layout(k, key), STEAM_LAYOUT) => self.layout_to_steam(key, *k),
            (Item::Prefs(serial), ORIG) => self.preferences_to_original(serial),
            _ => return Err("This value can\u{2019}t be put back".into()),
        };
        match r {
            // the app never changed it (no backup): it is as it was
            Ok(()) | Err(bu_controller::Error::NoBackup) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
    pub fn preferences(&self) -> Result<Vec<Prefs>> {
        with!(self, s => s.preferences())
    }
    pub fn set_preferences(&mut self, serial: &str, v: &[(PrefSetting, Option<&str>)], label: &str) -> Result<()> {
        with!(self, s => s.set_preferences(serial, v, label))
    }
    pub fn set_light_bar(&mut self, serial: &str, rgb: (u8, u8, u8)) -> Result<()> {
        with!(self, s => s.set_light_bar(serial, rgb))
    }
    pub fn open_in_steam_link(&self, g: &Game) -> Result<String> {
        with!(self, s => s.open_in_steam_link(g))
    }
    pub fn can_undo(&self) -> bool {
        with!(self, s => s.can_undo())
    }
    pub fn undo(&mut self) -> Result<String> {
        with!(self, s => s.undo())
    }
    /// What bu-controller's own last step was ("<game> · new action set").
    pub fn undo_label(&self) -> Option<String> {
        with!(self, s => s.undo_label().map(str::to_string))
    }
    /// Is Steam running (the fake: its switch; the real one: the process list).
    pub fn steam_running(&self) -> bool {
        with!(self, s => s.steam_running())
    }
    // ---- "Restart Steam to apply" (Order 085), in the order the worker runs them
    pub fn restart_check(&self) -> Result<()> {
        with!(self, s => s.restart_check())
    }
    pub fn light_values(&self, serial: &str) -> Result<Vec<(PrefSetting, Option<String>)>> {
        with!(self, s => s.light_values(serial))
    }
    pub fn steam_shutdown(&self) -> Result<()> {
        with!(self, s => s.steam_shutdown())
    }
    pub fn steam_closed(&self) -> bool {
        with!(self, s => s.steam_closed())
    }
    pub fn keep_light(&mut self, serial: &str, wanted: &[(PrefSetting, Option<String>)]) -> Result<bool> {
        with!(self, s => s.keep_light(serial, wanted))
    }
    pub fn steam_start_minimised(&self) -> Result<()> {
        with!(self, s => s.steam_start_minimised())
    }
    /// Steam's install folder (where `steam.exe` is).
    pub fn steam_dir(&self) -> PathBuf {
        with!(self, s => s.steam().dir.clone())
    }
    /// The fake's write log (tests).
    pub fn fake_writes(&self) -> Vec<String> {
        match self {
            Svc::Fake(s) => s.os().writes(),
            Svc::Real(_) => vec![],
        }
    }
    /// The fake's `steam.exe` calls ("-shutdown" / "-silent"), in order (tests).
    pub fn fake_procs(&self) -> Vec<String> {
        match self {
            Svc::Fake(s) => s.os().procs.lock().unwrap().clone(),
            Svc::Real(_) => vec![],
        }
    }
    /// A game runs through the fake Steam (tests).
    pub fn fake_game(&self, on: bool) {
        if let Svc::Fake(s) = self {
            s.os().game.store(on, std::sync::atomic::Ordering::Relaxed);
        }
    }
    /// The fake Steam writes this over a file while it closes (tests: Steam keeps its own copy and writes it back).
    pub fn fake_write_on_exit(&self, path: PathBuf, bytes: Vec<u8>) {
        if let Svc::Fake(s) = self {
            *s.os().write_on_exit.lock().unwrap() = Some((path, bytes));
        }
    }
    pub fn fake_text(&self, path: &std::path::Path) -> Option<String> {
        match self {
            Svc::Fake(s) => s.os().text(path),
            Svc::Real(_) => None,
        }
    }
}

// ------------------------------------------------------------------------------------------------ the change log (Order 036)

/// The page id the change log keeps the Controller tab's items under.
pub const PAGE: &str = "pad";
/// "as before the app's first change" (the crate's backups), Steam's own layout, the user's edits.
pub const ORIG: &str = "orig";
pub const STEAM_LAYOUT: &str = "steam";
pub const EDITS: &str = "edits";

/// One item of the change log: a game's layout file (per layout type: `ps5` / `ps4` / `xboxone`) or a controller's own
/// settings file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Layout(PadKind, String),
    Prefs(String),
}

impl Item {
    /// Its stable id: `layout:<layout type>:<game key>` / `prefs:<serial>`.
    pub fn id(&self) -> String {
        match self {
            Item::Layout(k, key) => format!("layout:{}:{key}", k.layout_type()),
            Item::Prefs(serial) => format!("prefs:{serial}"),
        }
    }

    pub fn parse(id: &str) -> Option<Item> {
        if let Some(rest) = id.strip_prefix("layout:") {
            let (t, key) = rest.split_once(':')?;
            let k = PadKind::ALL.into_iter().find(|k| k.layout_type() == t)?;
            return Some(Item::Layout(k, key.to_string()));
        }
        id.strip_prefix("prefs:").map(|s| Item::Prefs(s.to_string()))
    }
}

/// "Rocket League · DualSense Edge" (the drawing's words).
pub fn layout_label(game: &str, k: PadKind) -> String {
    format!("{game} \u{b7} {}", k.name())
}

/// "Controller settings · DualSense Edge Wireless Controller".
pub fn prefs_label(name: &str) -> String {
    format!("Controller settings \u{b7} {name}")
}

/// The value "as before the app's first change": `fresh` = the backup is made by this change ("as on 8 Oct (backup)",
/// the drawing's words).
pub fn orig_val(fresh: bool) -> Val {
    if fresh {
        Val::new(ORIG, &format!("as on {} (backup)", today()))
    } else {
        Val::new(ORIG, "as before (backup)")
    }
}

pub fn steam_val() -> Val {
    Val::new(STEAM_LAYOUT, "Steam\u{2019}s layout")
}

/// Today as "8 Oct" (UTC).
fn today() -> String {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86400).unwrap_or(0) as i64;
    // days since 1970-01-01 -> civil date (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{d} {}", M[(m - 1) as usize])
}

/// The service for the change log: the fake Steam in test copies (and unit tests), Steam read-only in a `--real-read`
/// copy, else the real Steam files. Opens nothing but Steam's folder.
pub fn open_steam(fake: bool, real_read: bool) -> std::result::Result<Svc, String> {
    if fake {
        return ControllerService::new(fake_steam(), BACKUPS).map(Svc::Fake).map_err(|e| e.to_string());
    }
    let backups = backups_dir();
    let os = if real_read { RealSteam::read_only() } else { RealSteam::new(backups.clone()) };
    ControllerService::new(os, backups).map(Svc::Real).map_err(|e| e.to_string())
}

/// The app's own folder for the original copies (kept across restarts: "Back to how your PC was" reads them).
fn backups_dir() -> PathBuf {
    crate::settings::SettingsStore::default_folder().map(|f| f.join("controller-backups")).unwrap_or_else(|| PathBuf::from("controller-backups"))
}

// ------------------------------------------------------------------------------------------------ the fake Steam

const FX: &str = "../../../../crates/controller/tests/fixtures/";
macro_rules! fx {
    ($f:literal) => {
        include_bytes!(concat!("../../../../crates/controller/tests/fixtures/", $f))
    };
}

pub const STEAM: &str = r"C:\Steam";
pub const ACCOUNT: &str = "10000001";
pub const SERIAL: &str = "DSE000000000001";
pub const BACKUPS: &str = r"C:\BU\backups";

pub fn config() -> PathBuf {
    PathBuf::from(STEAM).join(r"steamapps\common\Steam Controller Configs").join(ACCOUNT).join("config")
}

/// The drawing's per-controller sample (v22 `DV` + the light bar "Steam blue" #3b82ff): dead zones 8 %, anti-drift on, gyro
/// noise Medium, rumble on. Steam's own file format (tabs, LF) like Lane L's fixture.
const PREFS: &str = "\"ControllerPersonalization\"\n{\n\t\"name\"\t\t\"DualSense Edge Wireless Controller\"\n\t\"guide_brightness\"\t\t\"1\"\n\t\"antidrift_enabled_sw\"\t\t\"1\"\n\t\"gyro_stationary_noise_tolerance\"\t\t\"0.5\"\n\t\"rumble\"\t\t\"1\"\n\t\"color_red\"\t\t\"59\"\n\t\"color_green\"\t\t\"130\"\n\t\"color_blue\"\t\t\"255\"\n\t\"stick_left_deadzone\"\t\t\"2621\"\n\t\"stick_right_deadzone\"\t\t\"2621\"\n}\n";

/// A layout index with Rocket League only (Steam's format, like the fixture).
const ONLY_RL: &str = "\"controller_config\"
{
	\"252950\"
	{
		\"autosave\"		\"1\"
	}
}
";

/// Steam's binary shortcuts.vdf with one shortcut (the measured shape, as Lane L's test helper writes it).
fn shortcuts_bin(name: &str) -> Vec<u8> {
    let mut b = vec![0u8];
    b.extend(b"shortcuts\0");
    b.push(0);
    b.extend(b"0\0");
    b.push(2);
    b.extend(b"appid\0");
    b.extend([0x12, 0x34, 0x56, 0x78]);
    b.push(1);
    b.extend(b"AppName\0");
    b.extend(name.as_bytes());
    b.push(0);
    b.extend([8, 8, 8]);
    b
}

/// An in-memory Steam with the drawing's three games for the DualSense (Edge) layouts.
pub fn fake_steam() -> FakeSteam {
    let _ = FX;
    let s = PathBuf::from(STEAM);
    let mut f = FakeSteam::new(STEAM);
    f.active = Some(ACCOUNT.parse().unwrap_or(0));
    f.running = true;
    let files: Vec<(PathBuf, Vec<u8>)> = vec![
        (s.join("steam.exe"), b"MZ".to_vec()),
        (config().join("configset_controller_ps5.vdf"), fx!("configset_ps5.vdf").to_vec()),
        (config().join(r"252950\controller_ps5.vdf"), fx!("rl_ps5.vdf").to_vec()),
        (config().join(r"epic games launcher\controller_ps5.vdf"), fx!("rl_official_legacy.bin").to_vec()),
        (config().join(format!("preferences_{SERIAL}.vdf")), PREFS.as_bytes().to_vec()),
        (s.join(r"steamapps\workshop\content\241100\1700935741\932716421548274678_legacy.bin"), fx!("rl_official_legacy.bin").to_vec()),
        (s.join(r"steamapps\workshop\content\241100\3275392801\2477623897035151804_legacy.bin"), fx!("community_legacy.bin").to_vec()),
        (s.join(r"steamapps\libraryfolders.vdf"), fx!("libraryfolders.vdf").to_vec()),
        (s.join(r"steamapps\appmanifest_252950.acf"), fx!("appmanifest_252950.vdf").to_vec()),
        (PathBuf::from(r"D:\SteamLibrary\steamapps\appmanifest_638970.acf"), fx!("appmanifest_638970.vdf").to_vec()),
        (s.join("userdata").join(ACCOUNT).join(r"config\localconfig.vdf"), fx!("localconfig.vdf").to_vec()),
        (s.join("userdata").join(ACCOUNT).join(r"config\shortcuts.vdf"), shortcuts_bin("Epic Games Launcher")),
        // the drawing shows Rocket League for every outline: the same layout for the DualShock 4 and Xbox types
        (config().join("configset_controller_ps4.vdf"), ONLY_RL.as_bytes().to_vec()),
        (config().join(r"252950\controller_ps4.vdf"), fx!("rl_ps5.vdf").to_vec()),
        (config().join("configset_controller_xboxone.vdf"), ONLY_RL.as_bytes().to_vec()),
        (config().join(r"252950\controller_xboxone.vdf"), fx!("rl_ps5.vdf").to_vec()),
    ];
    for (p, b) in files {
        f.put(p, b);
    }
    f
}

/// The drawing's controller: a DualSense Edge on USB at 82 %, charging.
pub fn fake_pad() -> PadInfo {
    PadInfo {
        kind: PadKind::DualSenseEdge,
        name: "DualSense Edge Wireless Controller".into(),
        connection: Connection::Usb,
        battery: Some(Battery { percent: Some(82), level: None, charging: true, wired: true }),
        source: PadSource::Hid(r"\\?\hid#fake-edge".into()),
    }
}

// ------------------------------------------------------------------------------------------------ is Steam running

/// Steam's process, watched only while the tab is open (real copies): a thread looks at the process list every 1.5 s and
/// wakes the menu when Steam starts or ends. Dropped with the tab: the thread sees its stop channel close and ends at
/// once - nothing runs while the tab / the menu is closed.
pub struct SteamWatch {
    up: std::sync::Arc<std::sync::atomic::AtomicBool>,
    _stop: std::sync::mpsc::Sender<()>,
}

/// How often the open tab looks for Steam's process.
pub const STEAM_POLL_MS: u64 = 1500;

impl SteamWatch {
    /// `last` = what the tab knew before (its kept state; true when it never knew): shown until the watch's first look,
    /// which its own thread makes at once (Order 047: the process list is never read on the menu's thread).
    pub fn start(waker: crate::services::Waker, last: bool) -> SteamWatch {
        use std::sync::atomic::Ordering;
        let running = || bu_controller::real::process_running("steam.exe");
        let up = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(last));
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let seen = up.clone();
        let _ = std::thread::Builder::new().name("pad-steam-watch".into()).spawn(move || loop {
            let now = running();
            if seen.swap(now, Ordering::AcqRel) != now {
                waker.wake();
            }
            match rx.recv_timeout(std::time::Duration::from_millis(STEAM_POLL_MS)) {
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                // the tab closed (its sender is gone)
                _ => return,
            }
        });
        SteamWatch { up, _stop: tx }
    }

    pub fn up(&self) -> bool {
        self.up.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// The page's controllers for this copy (made on the tab's worker, Order 047). `slow` (tests) = the fake lists this slowly.
pub fn pads(fake: bool, slow: u64) -> Box<dyn PadOs> {
    if fake {
        return Box::new(FakePads { pads: vec![fake_pad()], delay_ms: slow, ..Default::default() });
    }
    Box::new(RealPads::new())
}

#[cfg(test)]
thread_local! {
    /// test-only (Order 047): the fake Steam / controllers of a tab opened on this test's thread answer this slowly (ms)
    pub static TEST_SLOW_MS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How slowly the fakes of a tab opened now answer (tests; 0 otherwise).
#[cfg(test)]
pub fn test_slow() -> u64 {
    TEST_SLOW_MS.with(|c| c.get())
}

#[cfg(not(test))]
pub fn test_slow() -> u64 {
    0
}
