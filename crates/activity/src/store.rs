//! Kept on this PC only: one small text file per day + one settings file, in `%LOCALAPPDATA%\BoylerUtilities\activity\`
//! (the app's own data folder; tests use a scratch folder or memory). Nothing is sent anywhere.
//!
//! Day file `2026-10-08.tsv` (UTF-8, tab-separated — ⇥ below; about 60–120 bytes per app, so ~1–3 KB for a normal day):
//! ```text
//! bu-activity day 1
//! first ⇥ 11:24
//! app ⇥ <milliseconds> ⇥ <name> ⇥ <exe path>
//! ```
//! Settings file `settings.tsv`: `on 0/1`, `since <date>`, `game <path> 0/1` (the user's right-click fix), `skip <path>`.
//! A file that can't be read is left exactly as it is: never deleted, never written over (the new time waits in memory
//! and is added once the file reads again — see `Activity::save`).

use crate::clock::Date;
use crate::{ActivityError, Result};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// One app on one day.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppDay {
    pub name: String,
    pub ms: u64,
}

/// One local day.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DayData {
    /// Minutes after midnight of the first counted moment ("since 11:24").
    pub first_minute: Option<u32>,
    /// exe path (lower case) → time
    pub apps: BTreeMap<String, AppDay>,
}

/// The stored settings.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StoredSettings {
    pub on: bool,
    /// The day counting was first switched on ("Counting since Wed 30 Sep").
    pub since: Option<Date>,
    /// exe path (lower case) → counts as a game (true) / not a game (false).
    pub game: BTreeMap<String, bool>,
    /// "Don't count this app".
    pub skip: Vec<String>,
}

pub trait Store {
    fn load_day(&mut self, date: Date) -> Result<Option<DayData>>;
    fn save_day(&mut self, date: Date, day: &DayData) -> Result<()>;
    fn load_settings(&mut self) -> Result<StoredSettings>;
    fn save_settings(&mut self, s: &StoredSettings) -> Result<()>;
}

/// In memory (tests).
#[derive(Debug, Clone, Default)]
pub struct MemStore {
    pub days: HashMap<Date, DayData>,
    pub settings: StoredSettings,
    pub writes: u32,
}

impl Store for MemStore {
    fn load_day(&mut self, date: Date) -> Result<Option<DayData>> {
        Ok(self.days.get(&date).cloned())
    }
    fn save_day(&mut self, date: Date, day: &DayData) -> Result<()> {
        self.writes += 1;
        self.days.insert(date, day.clone());
        Ok(())
    }
    fn load_settings(&mut self) -> Result<StoredSettings> {
        Ok(self.settings.clone())
    }
    fn save_settings(&mut self, s: &StoredSettings) -> Result<()> {
        self.writes += 1;
        self.settings = s.clone();
        Ok(())
    }
}

/// The files in one folder.
#[derive(Debug, Clone)]
pub struct FileStore {
    dir: PathBuf,
}

const DAY_HEADER: &str = "bu-activity day 1";
const SETTINGS_HEADER: &str = "bu-activity settings 1";

fn ferr(p: &Path, e: impl std::fmt::Display) -> ActivityError {
    ActivityError::File { path: p.display().to_string(), msg: e.to_string() }
}

impl FileStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        FileStore { dir: dir.into() }
    }

    /// A store that may only live inside `root` (proof runs: the lane's scratch folder). Refused (`ActivityError::Refused`)
    /// when `dir` has a `..` part or is not under `root` (compared without case, `/` = ``). `root` itself must exist.
    pub fn in_scratch(dir: impl Into<PathBuf>, root: &Path) -> Result<Self> {
        let dir: PathBuf = dir.into();
        let refuse = |why: &str| ActivityError::Refused(format!("{}: {why}", dir.display()));
        if dir.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(refuse("has a .. part"));
        }
        let root = std::fs::canonicalize(root).map_err(|e| refuse(&format!("scratch root missing ({e})")))?;
        // canonicalize gives a `\\?\C:\...` path; compare plain lower-case paths
        let norm = |p: &Path| p.display().to_string().replace('/', "\\").trim_start_matches(r"\\?\").to_lowercase();
        let (d, r) = (norm(&dir), norm(&root));
        if !d.starts_with(&format!("{}\\", r.trim_end_matches('\\'))) {
            return Err(refuse("not inside the scratch folder"));
        }
        Ok(FileStore { dir })
    }
    pub fn dir(&self) -> &Path {
        &self.dir
    }
    pub fn day_path(&self, date: Date) -> PathBuf {
        self.dir.join(format!("{}.tsv", date.iso()))
    }

    /// Writes next to the target, then renames over it: a crash never leaves half a file.
    fn write(&self, path: &Path, text: &str) -> Result<()> {
        std::fs::create_dir_all(&self.dir).map_err(|e| ferr(&self.dir, e))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, text).map_err(|e| ferr(&tmp, e))?;
        std::fs::rename(&tmp, path).map_err(|e| ferr(path, e))
    }

    fn read(&self, path: &Path) -> Result<Option<String>> {
        match std::fs::read_to_string(path) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(ferr(path, e)),
        }
    }

    /// Every day file in the folder (for "Show all" / the size check).
    pub fn days_on_disk(&self) -> Vec<(Date, u64)> {
        let mut v: Vec<(Date, u64)> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                let d = Date::parse_iso(n.strip_suffix(".tsv")?)?;
                Some((d, e.metadata().ok()?.len()))
            })
            .collect();
        v.sort();
        v
    }
}

/// The day file's text.
pub fn day_to_text(day: &DayData) -> String {
    let mut s = String::from(DAY_HEADER);
    s.push('\n');
    if let Some(m) = day.first_minute {
        s.push_str(&format!("first\t{:02}:{:02}\n", m / 60, m % 60));
    }
    for (path, a) in &day.apps {
        s.push_str(&format!("app\t{}\t{}\t{}\n", a.ms, clean(&a.name), clean(path)));
    }
    s
}

fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

pub fn day_from_text(text: &str) -> std::result::Result<DayData, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(DAY_HEADER) {
        return Err("not a bu-activity day file".into());
    }
    let mut day = DayData::default();
    for l in lines {
        let f: Vec<&str> = l.split('\t').collect();
        match f.as_slice() {
            ["first", hm] => {
                let (h, m) = hm.split_once(':').ok_or("bad first line")?;
                let (h, m): (u32, u32) = (h.parse().map_err(|_| "bad hour")?, m.parse().map_err(|_| "bad minute")?);
                day.first_minute = Some(h * 60 + m);
            }
            ["app", ms, name, path] => {
                let ms: u64 = ms.parse().map_err(|_| format!("bad time in {l:?}"))?;
                let e = day.apps.entry(path.to_string()).or_default();
                e.ms += ms;
                e.name = name.to_string();
            }
            [""] => {}
            _ => return Err(format!("unknown line {l:?}")),
        }
    }
    Ok(day)
}

pub fn settings_to_text(s: &StoredSettings) -> String {
    let mut t = format!("{SETTINGS_HEADER}\non\t{}\n", s.on as u8);
    if let Some(d) = s.since {
        t.push_str(&format!("since\t{}\n", d.iso()));
    }
    for (p, g) in &s.game {
        t.push_str(&format!("game\t{}\t{}\n", clean(p), *g as u8));
    }
    for p in &s.skip {
        t.push_str(&format!("skip\t{}\n", clean(p)));
    }
    t
}

pub fn settings_from_text(text: &str) -> std::result::Result<StoredSettings, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(SETTINGS_HEADER) {
        return Err("not a bu-activity settings file".into());
    }
    let mut s = StoredSettings::default();
    for l in lines {
        let f: Vec<&str> = l.split('\t').collect();
        match f.as_slice() {
            ["on", v] => s.on = *v == "1",
            ["since", d] => s.since = Date::parse_iso(d),
            ["game", p, v] => {
                s.game.insert(p.to_string(), *v == "1");
            }
            ["skip", p] => s.skip.push(p.to_string()),
            [""] => {}
            _ => return Err(format!("unknown line {l:?}")),
        }
    }
    Ok(s)
}

impl Store for FileStore {
    fn load_day(&mut self, date: Date) -> Result<Option<DayData>> {
        let p = self.day_path(date);
        match self.read(&p)? {
            None => Ok(None),
            Some(t) => day_from_text(&t).map(Some).map_err(|msg| ActivityError::BadData { path: p.display().to_string(), msg }),
        }
    }
    fn save_day(&mut self, date: Date, day: &DayData) -> Result<()> {
        let p = self.day_path(date);
        self.write(&p, &day_to_text(day))
    }
    fn load_settings(&mut self) -> Result<StoredSettings> {
        let p = self.dir.join("settings.tsv");
        match self.read(&p)? {
            None => Ok(StoredSettings::default()),
            Some(t) => settings_from_text(&t).map_err(|msg| ActivityError::BadData { path: p.display().to_string(), msg }),
        }
    }
    fn save_settings(&mut self, s: &StoredSettings) -> Result<()> {
        let p = self.dir.join("settings.tsv");
        self.write(&p, &settings_to_text(s))
    }
}
