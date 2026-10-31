//! What the Display tab keeps between runs (Order 019): the presets and the "Switch automatically" rules (and, Order
//! 124, each monitor's "Lock brightness"), in one small
//! JSON file in the app's settings folder. Written whole to a `.tmp` file and renamed over the old one, so a crash in the
//! middle never leaves half a file. A missing file = no presets, no rules; an unreadable one is kept as `.bad` and the tab
//! starts empty (nothing on the PC depends on it).
//! (The app's shared settings store comes with Order 014 item 2; this file moves into it then.)

use crate::autoswitch::AutoSwitcher;
use crate::error::{DisplayError, Result};
use crate::presets::PresetList;
use crate::types::MonitorId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// The file name inside the settings folder.
pub const FILE_NAME: &str = "display.json";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub presets: PresetList,
    #[serde(default)]
    pub rules: AutoSwitcher,
    /// Order 124: "Lock brightness" per monitor (the page's monitor id) → the locked brightness %; missing = not locked.
    #[serde(default)]
    pub bright_locks: BTreeMap<MonitorId, u8>,
}

impl Store {
    /// Reads the file; missing = empty. An unreadable file is renamed to `<name>.bad` (kept for a look) and an empty
    /// store is returned.
    pub fn load(path: &Path) -> Store {
        let Ok(text) = fs::read_to_string(path) else { return Store::default() };
        match serde_json::from_str(&text) {
            Ok(s) => s,
            Err(_) => {
                let _ = fs::rename(path, path.with_extension("bad"));
                Store::default()
            }
        }
    }

    /// Order 124: the monitor's locked brightness %, if it is locked.
    pub fn brightness_lock(&self, id: &MonitorId) -> Option<u8> {
        self.bright_locks.get(id).copied()
    }

    /// Order 124: locks a monitor's brightness at `pct` (a new value replaces the old one) / unlocks it (`None`).
    pub fn lock_brightness(&mut self, id: &MonitorId, pct: Option<u8>) {
        match pct {
            Some(p) => {
                self.bright_locks.insert(id.clone(), p.min(100));
            }
            None => {
                self.bright_locks.remove(id);
            }
        }
    }

    /// Order 124: every locked monitor with its brightness %.
    pub fn brightness_locks(&self) -> Vec<(MonitorId, u8)> {
        self.bright_locks.iter().map(|(id, p)| (id.clone(), *p)).collect()
    }

    /// Writes the whole store (tmp file + rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| DisplayError::os("create settings folder", e))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| DisplayError::os("write display.json", e))?;
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, text).map_err(|e| DisplayError::os("write display.json", e))?;
        fs::rename(&tmp, path).map_err(|e| DisplayError::os("write display.json", e))
    }
}
