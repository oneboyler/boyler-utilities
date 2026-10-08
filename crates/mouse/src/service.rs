//! The one service type the menu talks to: `Mouse<O>` owns the OS layer and the undo memory. Its methods are spread over
//! the feature modules (`settings`, `cursors`, `device`, `accel::service`), one `impl` block each.

use crate::cursors::CursorSnapshot;
use crate::os::{MouseOs, WinRaw, WinSetting};
use std::collections::HashMap;
use std::path::PathBuf;

/// What an undo entry is for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum UndoKey {
    Windows(WinSetting),
    /// the whole cursor state (17 role paths + scheme name/source + size), one entry
    Cursors,
    /// a setting on the mouse itself (DPI / polling / lift-off), by name
    OnMouse(&'static str),
}

/// The old value an undo puts back — exactly as it was read.
#[derive(Clone, Debug, PartialEq)]
pub enum UndoValue {
    Windows(WinRaw),
    Cursors(Box<CursorSnapshot>),
    OnMouse(u32),
}

/// Where the app keeps its own files (imported cursor packs, the settings file it hands to Raw Accel's writer).
/// Tests point it at the lane's scratch folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppDirs {
    pub data: PathBuf,
}

impl AppDirs {
    pub fn new(data: impl Into<PathBuf>) -> Self {
        Self { data: data.into() }
    }
    /// `<data>\cursors\packs\<pack>\`
    pub fn packs(&self) -> PathBuf {
        self.data.join("cursors").join("packs")
    }
    /// `<data>\cursors\own\` — single files picked with "Choose your own file…"
    pub fn own_cursors(&self) -> PathBuf {
        self.data.join("cursors").join("own")
    }
    /// `<data>\cursors\glass\` — the app's own Glass set
    pub fn glass(&self) -> PathBuf {
        self.data.join("cursors").join("glass")
    }
    /// The JSON the app hands to Raw Accel's writer.exe (never Raw Accel's own settings.json).
    pub fn rawaccel_settings(&self) -> PathBuf {
        self.data.join("rawaccel").join("settings.json")
    }
    /// `<data>\rawaccel\before\` — copies of what Raw Accel's driver ran before the app changed it (the change log's
    /// "Back to how your PC was"; kept across restarts, one file per different state).
    pub fn rawaccel_before(&self) -> PathBuf {
        self.data.join("rawaccel").join("before")
    }
}

/// The Mouse tab's service.
pub struct Mouse<O: MouseOs> {
    pub(crate) os: O,
    pub(crate) dirs: AppDirs,
    undo: HashMap<UndoKey, UndoValue>,
    /// "the set you last picked for another role" (cursor suggestion): (role, set) newest last
    pub(crate) cursor_picks: Vec<(crate::cursors::Role, crate::cursors::SetId)>,
    pub(crate) accel: crate::accel::service::AccelState,
}

impl<O: MouseOs> Mouse<O> {
    pub fn new(os: O, dirs: AppDirs) -> Self {
        Self { os, dirs, undo: HashMap::new(), cursor_picks: Vec::new(), accel: Default::default() }
    }

    pub fn os(&self) -> &O {
        &self.os
    }
    pub fn os_mut(&mut self) -> &mut O {
        &mut self.os
    }
    pub fn dirs(&self) -> &AppDirs {
        &self.dirs
    }

    /// Remembers the value before a change (a second change keeps the newest "before", so undo steps back once).
    pub(crate) fn remember(&mut self, key: UndoKey, old: UndoValue) {
        self.undo.insert(key, old);
    }

    pub(crate) fn take_undo(&mut self, key: &UndoKey) -> Option<UndoValue> {
        self.undo.remove(key)
    }

    /// Every change made here that can be put back, with the value from before it (the tab's "Back to how your PC was"
    /// review), in a fixed order: Windows' mouse settings (pointer speed, precision, scroll lines, double-click, swap),
    /// the cursors, then the mouse's own settings (DPI, polling, lift-off).
    pub fn undo_entries(&self) -> Vec<(UndoKey, UndoValue)> {
        let rank = |k: &UndoKey| match k {
            UndoKey::Windows(s) => *s as u32,
            UndoKey::Cursors => 10,
            UndoKey::OnMouse(w) => 20 + ["dpi", "polling", "lift_off"].iter().position(|x| x == w).unwrap_or(9) as u32,
        };
        let mut v: Vec<(UndoKey, UndoValue)> = self.undo.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        v.sort_by_key(|(k, _)| rank(k));
        v
    }

    /// Is there something to undo for this key?
    pub fn can_undo(&self, key: &UndoKey) -> bool {
        self.undo.contains_key(key)
    }
}
