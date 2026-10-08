//! The counter. Fed with what is in front and with idle / lock / sleep changes (the real watcher gets them from Windows
//! events), it adds up time per app per local day and keeps it in a [`Store`].
//!
//! Counted = the switch is on, the PC is awake, unlocked and not idle, and an app is in front that isn't "Don't count".
//! Idle = no keyboard / mouse input for [`IDLE_AFTER_MS`] (5 min) — but never while a game is in front (a controller
//! doesn't count as input for Windows, and cutscenes have none). Idle time is cut at the last input.
//! Only the exe in front is recorded — never window titles, never what is on screen.

use crate::clock::{Date, Stamp};
use crate::games::{key_of, GameRules};
use crate::store::{DayData, Store, StoredSettings};
use crate::{ActivityError, Result};
use std::collections::{BTreeMap, BTreeSet};

/// No input for this long = away (guess: 5 min, like most time trackers; DESIGN names only `GetLastInputInfo`).
pub const IDLE_AFTER_MS: i64 = 5 * 60_000;
/// Save at most once a minute while counting (and always on lock, sleep, sign-out, switch off).
pub const SAVE_EVERY_MS: i64 = 60_000;
/// Days kept in memory (today and the 6 before: what the tab shows).
pub const DAYS_SHOWN: i64 = 7;

pub type Settings = StoredSettings;

/// The app in front.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FgApp {
    /// The exe's full path.
    pub path: String,
    pub name: String,
}

impl FgApp {
    pub fn new(path: &str, name: &str) -> Self {
        FgApp { path: path.into(), name: name.into() }
    }
}

struct Span {
    key: String,
    name: String,
    start: Stamp,
}

pub struct Activity<S: Store> {
    store: S,
    settings: StoredSettings,
    pub rules: GameRules,
    days: BTreeMap<i64, DayData>,
    dirty: BTreeSet<i64>,
    fg: Option<FgApp>,
    idle: bool,
    locked: bool,
    asleep: bool,
    cur: Option<Span>,
    last_save: Option<i64>,
    /// Days whose file was there but could not be read: NEVER written over. Their new time is kept in memory and added to
    /// the file once it reads again (a passing lock by a virus scanner / backup). A file that is broken for good never reads
    /// again: that day's NEW time is lost when the app closes (the broken file itself stays as it is).
    unreadable: BTreeSet<i64>,
    /// settings.tsv could not be read: it is never written over this run (changes stay in memory).
    settings_unreadable: bool,
    /// Problems with the data files (an unreadable file is left exactly as it is). Newest last, at most 20.
    pub problems: Vec<ActivityError>,
}

impl<S: Store> Activity<S> {
    /// Loads the settings and the last 7 days.
    pub fn new(mut store: S, auto_roots: Vec<String>, now: Stamp) -> Self {
        let mut problems = Vec::new();
        let mut settings_unreadable = false;
        let settings = store.load_settings().unwrap_or_else(|e| {
            problems.push(e);
            settings_unreadable = true;
            StoredSettings::default()
        });
        let mut rules = GameRules::new(auto_roots);
        rules.overrides = settings.game.clone();
        let mut a = Activity {
            store,
            settings,
            rules,
            days: BTreeMap::new(),
            dirty: BTreeSet::new(),
            fg: None,
            idle: false,
            locked: false,
            asleep: false,
            cur: None,
            last_save: None,
            unreadable: BTreeSet::new(),
            settings_unreadable,
            problems,
        };
        let today = now.day();
        for d in today - (DAYS_SHOWN - 1)..=today {
            a.day_mut(d);
        }
        a.dirty.clear();
        a
    }

    fn problem(&mut self, e: ActivityError) {
        // each problem once (an unreadable file would otherwise add one every minute and push the others out)
        if self.problems.contains(&e) {
            return;
        }
        self.problems.push(e);
        let n = self.problems.len();
        if n > 20 {
            self.problems.drain(..n - 20);
        }
    }

    fn day_mut(&mut self, day: i64) -> &mut DayData {
        if !self.days.contains_key(&day) {
            let d = match self.store.load_day(Date::from_day(day)) {
                Ok(d) => d.unwrap_or_default(),
                Err(e) => {
                    self.problem(e);
                    self.unreadable.insert(day);
                    DayData::default()
                }
            };
            self.days.insert(day, d);
        }
        self.days.entry(day).or_default()
    }

    pub fn settings(&self) -> &StoredSettings {
        &self.settings
    }
    pub fn is_on(&self) -> bool {
        self.settings.on
    }
    pub fn store(&self) -> &S {
        &self.store
    }
    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    /// The "Count my activity" switch (off by default). On: counting starts now ("Counting since" = the first day it was
    /// switched on). Off: what was counted is saved and counting stops; the data stays.
    pub fn set_on(&mut self, on: bool, now: Stamp) -> Result<()> {
        if on == self.settings.on {
            return Ok(());
        }
        if on {
            self.settings.on = true;
            self.settings.since.get_or_insert(Date::from_day(now.day()));
            self.sync(now);
        } else {
            self.close(now);
            self.settings.on = false;
            self.save(now)?;
        }
        self.save_settings()
    }

    fn save_settings(&mut self) -> Result<()> {
        if self.settings_unreadable {
            let e = ActivityError::BadData { path: "settings.tsv".into(), msg: "could not be read at start: not written over".into() };
            self.problem(e.clone());
            return Err(e);
        }
        let s = self.settings.clone();
        self.store.save_settings(&s)
    }

    fn counting(&self) -> bool {
        self.settings.on && !self.idle && !self.locked && !self.asleep
    }

    fn wanted(&self) -> Option<&FgApp> {
        if !self.counting() {
            return None;
        }
        self.fg.as_ref().filter(|f| !f.path.is_empty() && !self.settings.skip.contains(&key_of(&f.path)))
    }

    fn sync(&mut self, now: Stamp) {
        let want = self.wanted().map(|f| (key_of(&f.path), f.name.clone()));
        if self.cur.as_ref().map(|c| &c.key) != want.as_ref().map(|w| &w.0) {
            self.close(now);
            if let Some((key, name)) = want {
                self.cur = Some(Span { key, name, start: now });
            }
        }
    }

    /// Ends the running span at `at` (never before its start) and adds it to its day(s), split at local midnight.
    fn close(&mut self, at: Stamp) {
        let Some(c) = self.cur.take() else { return };
        let mut s = c.start;
        let end = at.unix_ms;
        while s.unix_ms < end {
            let next = end.min(s.next_midnight_ms());
            let day = s.day();
            let minute = s.local_minute();
            let d = self.day_mut(day);
            let e = d.apps.entry(c.key.clone()).or_default();
            e.ms += (next - s.unix_ms) as u64;
            e.name = c.name.clone();
            d.first_minute = Some(d.first_minute.map_or(minute, |m| m.min(minute)));
            self.dirty.insert(day);
            s = Stamp { unix_ms: next, offset_min: s.offset_min };
        }
    }

    /// Counts the running span up to `now` without ending it (for views and saves).
    fn flush(&mut self, now: Stamp) {
        if let Some(c) = &self.cur {
            let (key, name) = (c.key.clone(), c.name.clone());
            self.close(now);
            self.cur = Some(Span { key, name, start: now });
        }
    }

    /// The front window changed (`None` = nothing usable in front: the desktop shell, the lock screen).
    pub fn foreground(&mut self, app: Option<FgApp>, now: Stamp) {
        if let Some(a) = &app {
            if self.idle && self.rules.is_game(&a.path) {
                // a game came to the front: it never idles
                self.idle = false;
            }
        }
        self.fg = app;
        self.sync(now);
        self.maybe_save(now);
    }

    /// Whether the idle rule applies now (no idle while a game is in front).
    pub fn idle_applies(&self) -> bool {
        self.fg.as_ref().is_none_or(|f| !self.rules.is_game(&f.path))
    }

    pub fn is_idle(&self) -> bool {
        self.idle
    }

    /// No input since `last_input`: away from then on (time after the last input isn't counted).
    pub fn went_idle(&mut self, last_input: Stamp, now: Stamp) {
        if !self.idle_applies() || self.idle {
            return;
        }
        let at = match &self.cur {
            Some(c) if last_input < c.start => c.start,
            _ => last_input.min(now),
        };
        self.close(at);
        self.idle = true;
        self.sync(now);
        self.maybe_save(now);
    }

    /// Input again after idle.
    pub fn back_from_idle(&mut self, now: Stamp) {
        self.idle = false;
        self.sync(now);
    }

    /// Lock / unlock (Win+L, the lock screen).
    pub fn locked(&mut self, locked: bool, now: Stamp) {
        self.locked = locked;
        self.sync(now);
        if locked {
            let _ = self.save(now);
        }
    }

    /// Sleep / hibernate (suspend) and wake (resume). Suspend saves.
    pub fn asleep(&mut self, asleep: bool, now: Stamp) {
        self.asleep = asleep;
        self.sync(now);
        if asleep {
            let _ = self.save(now);
        }
    }

    /// Saves what changed (the running span is counted up to `now`).
    pub fn save(&mut self, now: Stamp) -> Result<()> {
        self.flush(now);
        let dirty: Vec<i64> = std::mem::take(&mut self.dirty).into_iter().collect();
        let mut first_err = None;
        for d in dirty {
            let mut data = self.days.get(&d).cloned().unwrap_or_default();
            if self.unreadable.contains(&d) {
                // its file couldn't be read: try again; only a file that reads now is merged and written
                match self.store.load_day(Date::from_day(d)) {
                    Ok(disk) => {
                        let mut merged = disk.unwrap_or_default();
                        for (k, a) in &data.apps {
                            let e = merged.apps.entry(k.clone()).or_default();
                            e.ms += a.ms;
                            e.name = a.name.clone();
                        }
                        merged.first_minute = match (merged.first_minute, data.first_minute) {
                            (Some(x), Some(y)) => Some(x.min(y)),
                            (x, y) => x.or(y),
                        };
                        self.unreadable.remove(&d);
                        self.days.insert(d, merged.clone());
                        data = merged;
                    }
                    Err(e) => {
                        self.dirty.insert(d);
                        self.problem(e.clone());
                        first_err.get_or_insert(e);
                        continue;
                    }
                }
            }
            if let Err(e) = self.store.save_day(Date::from_day(d), &data) {
                self.dirty.insert(d);
                self.problem(e.clone());
                first_err.get_or_insert(e);
            }
        }
        self.last_save = Some(now.unix_ms);
        // keep only the days the tab shows (and anything not saved yet)
        let keep_from = now.day() - (DAYS_SHOWN - 1);
        let dirty = self.dirty.clone();
        self.days.retain(|d, _| *d >= keep_from || dirty.contains(d));
        first_err.map_or(Ok(()), Err)
    }

    /// Saves if the last save is a minute old.
    pub fn maybe_save(&mut self, now: Stamp) {
        if self.dirty.is_empty() && self.cur.is_none() {
            return;
        }
        if self.last_save.is_none_or(|t| now.unix_ms - t >= SAVE_EVERY_MS) {
            let _ = self.save(now);
        }
    }

    /// Right-click "Count as a game" (`Some(true)`) / "Not a game" (`Some(false)`); `None` = back to automatic.
    /// Returns the old choice (undo).
    pub fn set_game(&mut self, path: &str, game: Option<bool>) -> Result<Option<bool>> {
        let k = key_of(path);
        let old = match game {
            Some(g) => self.settings.game.insert(k.clone(), g),
            None => self.settings.game.remove(&k),
        };
        self.rules.overrides = self.settings.game.clone();
        self.save_settings()?;
        Ok(old)
    }

    /// Right-click "Don't count this app" (`false`) / count it again (`true`, the undo). Not counted from now on and
    /// hidden from the lists; its old time stays in the files.
    pub fn set_counted(&mut self, path: &str, counted: bool, now: Stamp) -> Result<()> {
        let k = key_of(path);
        if counted {
            self.settings.skip.retain(|x| *x != k);
        } else if !self.settings.skip.contains(&k) {
            self.settings.skip.push(k);
        }
        self.sync(now);
        self.save_settings()
    }

    pub fn is_counted(&self, path: &str) -> bool {
        !self.settings.skip.contains(&key_of(path))
    }

    /// The days the tab shows, with the running span counted up to `now`: (day number, data), oldest first.
    pub fn days_until(&mut self, now: Stamp) -> Vec<(i64, DayData)> {
        self.flush(now);
        let today = now.day();
        (today - (DAYS_SHOWN - 1)..=today).map(|d| (d, self.day_mut(d).clone())).collect()
    }
}
