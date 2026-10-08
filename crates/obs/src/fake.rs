//! The FAKE OS layer for tests: OBS, its process, the Startup folder, the disk and the speakers are all pretend. Files
//! are only written inside the fake's own OBS folder (a scratch folder the test made).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::os::{ObsOs, Waiter};

#[derive(Debug, Default)]
pub struct FakeState {
    pub obs_dir: PathBuf,
    pub running: bool,
    pub pid: u32,
    pub exe: PathBuf,
    /// what close_obs did (pids); a close makes `running` false when `close_works`
    pub closed: Vec<u32>,
    pub close_works: bool,
    pub started: Vec<(PathBuf, Option<String>, bool)>,
    /// start_obs makes OBS "run"
    pub start_works: bool,
    pub exists: HashSet<PathBuf>,
    pub free: Option<u64>,
    pub user_autostart: bool,
    pub shortcut: Option<PathBuf>,
    pub default_path: PathBuf,
    pub steam: Option<PathBuf>,
    pub played: Vec<Vec<u8>>,
    pub other: Option<(u32, PathBuf)>,
    pub other_closed: Vec<u32>,
    /// its Quit does nothing (it hangs): only `end_other_app` ends it
    pub other_stuck: bool,
    pub other_ended: Vec<u32>,
    pub run_entry: Option<PathBuf>,
    pub writes: Vec<PathBuf>,
    pub refused_writes: Vec<PathBuf>,
}

#[derive(Clone, Default)]
pub struct FakeOs {
    pub st: Arc<Mutex<FakeState>>,
}

impl FakeOs {
    pub fn new(obs_dir: &Path) -> Self {
        let st = FakeState {
            obs_dir: obs_dir.to_path_buf(),
            pid: 4242,
            exe: PathBuf::from("C:\\fake\\obs-studio\\bin\\64bit\\obs64.exe"),
            close_works: true,
            start_works: true,
            default_path: PathBuf::from("C:\\fake\\obs-studio\\bin\\64bit\\obs64.exe"),
            ..Default::default()
        };
        FakeOs { st: Arc::new(Mutex::new(st)) }
    }
    pub fn with<R>(&self, f: impl FnOnce(&mut FakeState) -> R) -> R {
        f(&mut self.st.lock().unwrap())
    }
}

impl ObsOs for FakeOs {
    fn obs_dir(&self) -> PathBuf {
        self.with(|s| s.obs_dir.clone())
    }
    fn obs_running(&self) -> bool {
        self.with(|s| s.running)
    }
    fn find_obs(&self) -> Option<(u32, PathBuf)> {
        self.with(|s| s.running.then(|| (s.pid, s.exe.clone())))
    }
    fn close_obs(&mut self, pid: u32) -> bool {
        self.with(|s| {
            s.closed.push(pid);
            if s.close_works {
                s.running = false;
            }
            true
        })
    }
    fn wait_exit(&self, pid: u32, _timeout_ms: u32) -> bool {
        self.with(|s| match &s.other {
            // NotificationsForOBS still runs
            Some((p, _)) if *p == pid => false,
            _ => !s.running,
        })
    }
    fn waiter(&self) -> Waiter {
        let me = self.clone();
        Box::new(move |p, t| me.wait_exit(p, t))
    }
    fn start_obs(&mut self, path: &Path, args: Option<&str>, quiet: bool) -> bool {
        self.with(|s| {
            s.started.push((path.to_path_buf(), args.map(String::from), quiet));
            if s.start_works {
                s.running = true;
            }
            s.start_works
        })
    }
    fn default_path(&self) -> PathBuf {
        self.with(|s| s.default_path.clone())
    }
    fn program_files_path(&self) -> Option<PathBuf> {
        None
    }
    fn steam_obs(&self) -> Option<PathBuf> {
        self.with(|s| s.steam.clone())
    }
    fn file_exists(&self, p: &Path) -> bool {
        self.with(|s| s.exists.contains(p) || (s.running && p == s.exe))
    }
    fn free_bytes(&self, _dir: &Path) -> Option<u64> {
        self.with(|s| s.free)
    }
    fn user_autostart(&self) -> bool {
        self.with(|s| s.user_autostart)
    }
    fn shortcut_exists(&self) -> bool {
        self.with(|s| s.shortcut.is_some())
    }
    fn shortcut_create(&mut self, obs: &Path) -> bool {
        self.with(|s| {
            s.shortcut = Some(obs.to_path_buf());
            true
        })
    }
    fn shortcut_delete(&mut self) -> bool {
        self.with(|s| {
            s.shortcut = None;
            true
        })
    }
    fn random_password(&self, n: usize) -> Option<String> {
        Some("Ab3".repeat(n).chars().take(n).collect())
    }
    fn write_file(&mut self, p: &Path, text: &str) -> bool {
        let inside = self.with(|s| p.starts_with(&s.obs_dir) && !s.obs_dir.as_os_str().is_empty());
        if !inside {
            self.with(|s| s.refused_writes.push(p.to_path_buf()));
            return false;
        }
        self.with(|s| s.writes.push(p.to_path_buf()));
        std::fs::write(p, text).is_ok()
    }
    fn read_file(&self, p: &Path) -> Option<String> {
        std::fs::read(p).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
    }
    fn play(&mut self, wav: Vec<u8>) {
        self.with(|s| s.played.push(wav));
    }
    fn other_app(&self) -> Option<(u32, PathBuf)> {
        self.with(|s| s.other.clone())
    }
    fn close_other_app(&mut self, pid: u32) -> bool {
        self.with(|s| {
            s.other_closed.push(pid);
            if !s.other_stuck {
                s.other = None;
            }
            true
        })
    }
    fn end_other_app(&mut self, pid: u32, exe: &Path) -> bool {
        self.with(|s| {
            if !matches!(&s.other, Some((p, e)) if *p == pid && e == exe) {
                return false;
            }
            s.other_ended.push(pid);
            s.other = None;
            true
        })
    }
    fn other_app_run_entry(&self) -> Option<PathBuf> {
        self.with(|s| s.run_entry.clone())
    }
}
