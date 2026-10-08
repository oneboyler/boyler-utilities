//! Per app (DESIGN §3.4 "Per app"): rows `[app] → [preset / Off]` + "Everywhere else → [Off / a preset]". The PROCESS
//! starting / stopping switches Raw Accel (fed by `crate::win::watch::AppWatcher`, event-driven — bu-display's approach).
//! This module is pure planning: events in, "what should Raw Accel run now" out. No timers, no I/O.
//!
//! Calls made for what DESIGN left unclear (also in the report):
//! - **Two listed apps at once:** the one started last wins; when it closes, the one still running takes over again;
//!   when the last one closes, "Everywhere else" applies.
//! - **Never mid-game:** an app that already shows a window when its start is noticed (the watcher's `has_window`) is
//!   not switched to; a note says "it will be next launch". Its stop is then ignored as well.

use serde::{Deserialize, Serialize};

/// What the app watcher reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppEvent {
    /// A watched process started. `exe` = its full path if known, else its file name. `has_window` = it already showed
    /// a window when the start was noticed (too late to switch).
    Started { pid: u32, exe: String, has_window: bool },
    Stopped { pid: u32 },
}

/// A preset id (stable across renames).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PresetId(pub u64);

/// The right side of a row / of "Everywhere else".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    /// plain 1:1 (see `crate::accel::panel::off_args`)
    Off,
    Preset(PresetId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RowId(pub u64);

/// One row: app → preset / Off.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRow {
    pub id: RowId,
    /// full path of the .exe ("Browse for an app…") or just its file name (recent games list)
    pub exe: String,
    /// the name the header line shows ("VALORANT"); from the app picker, else the exe name without ".exe"
    pub label: String,
    pub target: Target,
}

/// Notes for the menu (a quiet toast later; never a prompt, never in a game).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwitchNote {
    /// The app already had a window when its start was noticed: nothing switched (it will be next launch).
    TooLate { row: RowId, exe: String },
}

fn file_name(p: &str) -> &str {
    p.rsplit(['\\', '/']).next().unwrap_or(p)
}

/// Row matching: a full path in the row must equal the process path (case-insensitive) when the process path is known;
/// otherwise the file names must match (case-insensitive).
pub fn exe_matches(row_exe: &str, process_exe: &str) -> bool {
    if row_exe.contains(['\\', '/']) && process_exe.contains(['\\', '/']) {
        row_exe.eq_ignore_ascii_case(process_exe)
    } else {
        file_name(row_exe).eq_ignore_ascii_case(file_name(process_exe))
    }
}

/// The rows + "Everywhere else" (saved with the app's settings) and which listed apps run now (not saved).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PerApp {
    rows: Vec<AppRow>,
    next_id: u64,
    /// "Everywhere else", default Off
    everywhere_else: Option<PresetId>,
    #[serde(skip)]
    active: Vec<(u32, RowId)>,
    #[serde(skip)]
    notes: Vec<SwitchNote>,
}

impl PerApp {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rows(&self) -> &[AppRow] {
        &self.rows
    }

    pub fn everywhere_else(&self) -> Target {
        self.everywhere_else.map(Target::Preset).unwrap_or(Target::Off)
    }

    pub fn set_everywhere_else(&mut self, t: Target) {
        self.everywhere_else = match t {
            Target::Off => None,
            Target::Preset(p) => Some(p),
        };
    }

    /// "+ Add app": a row with the current preset (the caller passes it).
    pub fn add_row(&mut self, exe: impl Into<String>, target: Target) -> RowId {
        self.next_id += 1;
        let id = RowId(self.next_id);
        let exe = exe.into();
        let label = file_name(&exe).trim_end_matches(".exe").trim_end_matches(".EXE").to_string();
        self.rows.push(AppRow { id, exe, label, target });
        id
    }

    pub fn set_row_target(&mut self, id: RowId, t: Target) -> bool {
        match self.rows.iter_mut().find(|r| r.id == id) {
            Some(r) => {
                r.target = t;
                true
            }
            None => false,
        }
    }

    pub fn set_row_exe(&mut self, id: RowId, exe: impl Into<String>) -> bool {
        match self.rows.iter_mut().find(|r| r.id == id) {
            Some(r) => {
                r.exe = exe.into();
                true
            }
            None => false,
        }
    }

    /// Row ×.
    pub fn remove_row(&mut self, id: RowId) -> Option<AppRow> {
        let i = self.rows.iter().position(|r| r.id == id)?;
        self.active.retain(|(_, r)| *r != id);
        Some(self.rows.remove(i))
    }

    /// A preset was deleted: rows using it turn Off, and "Everywhere else" too (DESIGN: "Apps that used the preset turn
    /// Off"). Returns the exe names of the rows that changed (for the toast) and whether "Everywhere else" changed.
    pub fn forget_preset(&mut self, p: PresetId) -> (Vec<String>, bool) {
        let mut apps = Vec::new();
        for r in &mut self.rows {
            if r.target == Target::Preset(p) {
                r.target = Target::Off;
                apps.push(file_name(&r.exe).to_string());
            }
        }
        let ee = self.everywhere_else == Some(p);
        if ee {
            self.everywhere_else = None;
        }
        (apps, ee)
    }

    /// The exe file names to watch (only these are looked at by the watcher). No rows → nothing is watched.
    pub fn watched_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.rows.iter().map(|r| file_name(&r.exe).to_ascii_lowercase()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// Notes for the menu since the last call.
    pub fn take_notes(&mut self) -> Vec<SwitchNote> {
        std::mem::take(&mut self.notes)
    }

    /// Feeds one event. Returns true when what should run (`current`) may have changed.
    pub fn on_event(&mut self, ev: &AppEvent) -> bool {
        match ev {
            AppEvent::Started { pid, exe, has_window } => {
                if self.active.iter().any(|(p, _)| p == pid) {
                    return false;
                }
                let Some(row) = self.rows.iter().find(|r| exe_matches(&r.exe, exe)) else { return false };
                if *has_window {
                    self.notes.push(SwitchNote::TooLate { row: row.id, exe: file_name(exe).to_string() });
                    return false;
                }
                self.active.push((*pid, row.id));
                true
            }
            AppEvent::Stopped { pid } => {
                let before = self.active.len();
                self.active.retain(|(p, _)| p != pid);
                self.active.len() != before
            }
        }
    }

    /// What should run now: the last-started listed app's row, else "Everywhere else".
    pub fn current(&self) -> Target {
        self.active
            .iter()
            .rev()
            .find_map(|(_, id)| self.rows.iter().find(|r| r.id == *id).map(|r| r.target))
            .unwrap_or_else(|| self.everywhere_else())
    }

    /// The listed app that decides `current` (for the header line), if any.
    pub fn deciding_row(&self) -> Option<&AppRow> {
        self.active.iter().rev().find_map(|(_, id)| self.rows.iter().find(|r| r.id == *id))
    }
}

impl PerApp {
    /// The picker's display name for a row ("VALORANT" instead of "VALORANT-Win64-Shipping").
    pub fn set_row_label(&mut self, id: RowId, label: impl Into<String>) -> bool {
        match self.rows.iter_mut().find(|r| r.id == id) {
            Some(r) => {
                r.label = label.into();
                true
            }
            None => false,
        }
    }
}

/// The settle delay the app uses unless told otherwise — a GUESS (DESIGN only says "about a second"; the driver adds its
/// own 1 s per write).
pub const DEFAULT_SETTLE: std::time::Duration = std::time::Duration::from_millis(500);

/// "Switch only at app start / exit, with a settle delay" (DESIGN): events within `delay` of each other become ONE
/// driver write, `delay` after the last one. Pure — the caller owns the clock and waits on its own channel with
/// `recv_timeout(until_due)` (no polling timer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settle {
    pub delay: std::time::Duration,
    due: Option<std::time::Instant>,
}

impl Settle {
    pub fn new(delay: std::time::Duration) -> Self {
        Self { delay, due: None }
    }
    /// An event happened at `now`: the write is (re)scheduled for `now + delay`.
    pub fn poke(&mut self, now: std::time::Instant) {
        self.due = Some(now + self.delay);
    }
    /// How long to wait from `now` (None = nothing pending).
    pub fn until_due(&self, now: std::time::Instant) -> Option<std::time::Duration> {
        self.due.map(|d| d.saturating_duration_since(now))
    }
    /// True once at/after the due time; clears it.
    pub fn take_due(&mut self, now: std::time::Instant) -> bool {
        match self.due {
            Some(d) if now >= d => {
                self.due = None;
                true
            }
            _ => false,
        }
    }
}
