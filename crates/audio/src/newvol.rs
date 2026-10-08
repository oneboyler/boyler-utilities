//! "New apps volume" — "Apps you open from now on" start at a set volume (A_012_01, option 1: each app start).
//! Pure logic, fed with new sessions (the real watcher gets them from `IAudioSessionNotification::OnSessionCreated`:
//! no polling).
//!
//! The rule: the FIRST session of a process that STARTED after the switch was turned on gets the volume. More sessions
//! of that same running process are left alone (never fight a slider the user moved); apps that were already running are
//! never touched; System sounds never. A process run is (pid, start time) — pids are reused by Windows.

use crate::model::SessionInfo;
use std::collections::HashSet;

/// The drawing's defaults (A_012_01 b): on, 50 %.
pub const DEFAULT_ON: bool = true;
pub const DEFAULT_VOLUME: f32 = 0.5;

#[derive(Debug, Clone)]
pub struct NewAppsVolume {
    on: bool,
    volume: f32,
    /// Processes that started before this (FILETIME, 100 ns since 1601) are "already running".
    since: u64,
    seen: HashSet<(u32, u64)>,
    own_pid: u32,
}

/// Now as a Windows FILETIME (100 ns since 1601-01-01 UTC), the unit of a process's start time.
pub fn filetime_now() -> u64 {
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (unix.as_nanos() / 100) as u64 + 116_444_736_000_000_000
}

impl NewAppsVolume {
    /// `since` = now (FILETIME): only apps started from here on count.
    pub fn new(on: bool, volume: f32, since: u64) -> Self {
        NewAppsVolume { on, volume: volume.clamp(0.0, 1.0), since, seen: HashSet::new(), own_pid: std::process::id() }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }
    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// The switch. Turning it on starts "from now on" again at `now` (FILETIME).
    pub fn set_on(&mut self, on: bool, now: u64) {
        if on && !self.on {
            self.since = now;
            self.seen.clear();
        }
        self.on = on;
    }

    /// The slider (0.0 … 1.0).
    pub fn set_volume(&mut self, v: f32) {
        self.volume = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    }

    /// A new session: the volume to give it, or `None` to leave it alone.
    pub fn on_session_created(&mut self, s: &SessionInfo) -> Option<f32> {
        if !self.on || s.system || s.pid == 0 || s.pid == self.own_pid {
            return None;
        }
        if s.process_started == 0 || s.process_started < self.since {
            // already running (or its start time can't be read): not an app opened "from now on"
            return None;
        }
        self.seen.insert((s.pid, s.process_started)).then_some(self.volume)
    }

    /// How many process runs are remembered (16 bytes each; cleared when the switch goes off → on).
    pub fn remembered(&self) -> usize {
        self.seen.len()
    }
}
