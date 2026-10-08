//! The OS reads Activity needs (all read-only): uptime, launcher install folders, app names, the data folder.

use std::collections::HashMap;
use std::path::PathBuf;

pub trait ActivityOs {
    /// Milliseconds since Windows started (`GetTickCount64`: counts sleep too, so it is "since the last full restart";
    /// with Fast Startup a Shut down doesn't reset it — the ⓘ text).
    fn uptime_ms(&mut self) -> u64;
    /// Every game launcher install folder on this PC.
    fn game_roots(&mut self) -> Vec<String>;
    /// The app's name for the list: the exe's FileDescription, else the exe name.
    fn app_name(&mut self, exe_path: &str) -> String;
    /// `%LOCALAPPDATA%\BoylerUtilities\activity`.
    fn data_dir(&mut self) -> Option<PathBuf>;
}

#[derive(Debug, Clone, Default)]
pub struct FakeOs {
    pub uptime_ms: u64,
    pub roots: Vec<String>,
    pub names: HashMap<String, String>,
    pub dir: Option<PathBuf>,
}

impl ActivityOs for FakeOs {
    fn uptime_ms(&mut self) -> u64 {
        self.uptime_ms
    }
    fn game_roots(&mut self) -> Vec<String> {
        self.roots.clone()
    }
    fn app_name(&mut self, exe_path: &str) -> String {
        self.names.get(exe_path).cloned().unwrap_or_else(|| exe_stem(exe_path))
    }
    fn data_dir(&mut self) -> Option<PathBuf> {
        self.dir.clone()
    }
}

/// "chrome" from "C:\...\chrome.exe".
pub fn exe_stem(path: &str) -> String {
    let f = path.rsplit(['\\', '/']).next().unwrap_or(path);
    f.strip_suffix(".exe").or_else(|| f.strip_suffix(".EXE")).unwrap_or(f).to_string()
}
