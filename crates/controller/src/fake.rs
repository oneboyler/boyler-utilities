//! The FAKE OS layer for tests: an in-memory file system with Steam's folders, a log of every write, a read-only switch,
//! and fake controllers whose reports come from a script.

use crate::error::{Error, Result};
use crate::os::{Entry, LiveEvent, LiveSource, PadInfo, PadOs, SteamOs};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

fn key(p: &Path) -> String {
    p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase()
}

/// In-memory Steam.
#[derive(Debug, Default)]
pub struct FakeSteam {
    pub files: Mutex<BTreeMap<String, (PathBuf, Vec<u8>)>>,
    pub steam_dir: Option<PathBuf>,
    pub active: Option<u32>,
    pub running: bool,
    /// Refuse every write (like `RealOs::read_only()`).
    pub read_only: bool,
    /// Every write / remove, in order ("write <path>" / "remove <path>").
    pub log: Mutex<Vec<String>>,
    /// Tests: a write to a path ending with this text fails (like a locked file).
    pub fail_on: Mutex<Option<String>>,
    /// Tests: removing a path ending with this text fails.
    pub fail_remove: Mutex<Option<String>>,
}

impl FakeSteam {
    pub fn new(steam_dir: impl Into<PathBuf>) -> Self {
        FakeSteam { steam_dir: Some(steam_dir.into()), ..Default::default() }
    }
    pub fn put(&self, path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) {
        let p = path.as_ref().to_path_buf();
        self.files.lock().unwrap().insert(key(&p), (p, bytes.as_ref().to_vec()));
    }
    pub fn get(&self, path: impl AsRef<Path>) -> Option<Vec<u8>> {
        self.files.lock().unwrap().get(&key(path.as_ref())).map(|(_, b)| b.clone())
    }
    pub fn text(&self, path: impl AsRef<Path>) -> Option<String> {
        self.get(path).map(|b| String::from_utf8_lossy(&b).into_owned())
    }
    pub fn writes(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl SteamOs for FakeSteam {
    fn steam_dir(&self) -> Option<PathBuf> {
        self.steam_dir.clone()
    }
    fn active_account(&self) -> Option<u32> {
        self.active
    }
    fn read(&self, path: &Path) -> Result<Vec<u8>> {
        self.get(path).ok_or_else(|| Error::io(format!("read {}", path.display()), std::io::Error::from(std::io::ErrorKind::NotFound)))
    }
    fn exists(&self, path: &Path) -> bool {
        let k = key(path);
        let pre = format!("{k}\\");
        let f = self.files.lock().unwrap();
        f.contains_key(&k) || f.keys().any(|x| x.starts_with(&pre))
    }
    fn list(&self, dir: &Path) -> Result<Vec<Entry>> {
        let pre = format!("{}\\", key(dir));
        let f = self.files.lock().unwrap();
        let mut out: BTreeMap<String, bool> = BTreeMap::new();
        for (k, (p, _)) in f.iter() {
            if let Some(rest) = k.strip_prefix(&pre) {
                // keep the original spelling of the name
                let depth = rest.split('\\').count();
                let comps: Vec<String> = p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                let name = comps[comps.len() - depth].clone();
                let is_dir = depth > 1;
                let e = out.entry(name).or_insert(is_dir);
                *e |= is_dir;
            }
        }
        if out.is_empty() && !f.keys().any(|k| k.starts_with(&pre)) {
            return Err(Error::io(format!("list {}", dir.display()), std::io::Error::from(std::io::ErrorKind::NotFound)));
        }
        Ok(out.into_iter().map(|(name, is_dir)| Entry { name, is_dir }).collect())
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        if self.read_only {
            return Err(Error::ReadOnly(format!("write {}", path.display())));
        }
        if let Some(f) = self.fail_on.lock().unwrap().as_deref() {
            if path.to_string_lossy().ends_with(f) {
                return Err(Error::io(format!("write {}", path.display()), std::io::Error::from(std::io::ErrorKind::PermissionDenied)));
            }
        }
        self.log.lock().unwrap().push(format!("write {}", path.display()));
        self.put(path, bytes);
        Ok(())
    }
    fn create_dir_all(&self, dir: &Path) -> Result<()> {
        if self.read_only {
            return Err(Error::ReadOnly(format!("create {}", dir.display())));
        }
        Ok(())
    }
    fn remove(&self, path: &Path) -> Result<()> {
        if self.read_only {
            return Err(Error::ReadOnly(format!("remove {}", path.display())));
        }
        if let Some(f) = self.fail_remove.lock().unwrap().as_deref() {
            if path.to_string_lossy().ends_with(f) {
                return Err(Error::io(format!("remove {}", path.display()), std::io::Error::from(std::io::ErrorKind::PermissionDenied)));
            }
        }
        self.log.lock().unwrap().push(format!("remove {}", path.display()));
        self.files.lock().unwrap().remove(&key(path));
        Ok(())
    }
    fn steam_running(&self) -> bool {
        self.running
    }
}

/// Fake controllers: a list + per-controller scripted reports.
#[derive(Default)]
pub struct FakePads {
    pub pads: Vec<PadInfo>,
    /// Reports each opened live source hands out, then it blocks until stopped.
    pub script: Mutex<VecDeque<LiveEvent>>,
    /// How many live sources are open right now (the page-closed = nothing-open proof).
    pub open_now: Arc<Mutex<usize>>,
}

impl PadOs for FakePads {
    fn list_pads(&self) -> Result<Vec<PadInfo>> {
        Ok(self.pads.clone())
    }
    fn open_live(&self, pad: &PadInfo) -> Result<Box<dyn LiveSource>> {
        if !self.pads.contains(pad) {
            return Err(Error::PadGone(pad.name.clone()));
        }
        let (tx, rx) = channel();
        *self.open_now.lock().unwrap() += 1;
        Ok(Box::new(FakeLive { script: self.script.lock().unwrap().drain(..).collect(), stop_rx: rx, stop_tx: tx, open_now: self.open_now.clone() }))
    }
}

struct FakeLive {
    script: VecDeque<LiveEvent>,
    stop_rx: Receiver<()>,
    stop_tx: Sender<()>,
    open_now: Arc<Mutex<usize>>,
}

impl LiveSource for FakeLive {
    fn next(&mut self) -> Result<LiveEvent> {
        if let Some(e) = self.script.pop_front() {
            return Ok(e);
        }
        // like the real source: block (no timer) until stopped
        let _ = self.stop_rx.recv();
        Ok(LiveEvent::Stopped)
    }
    fn stopper(&self) -> Arc<dyn Fn() + Send + Sync> {
        let tx = Mutex::new(self.stop_tx.clone());
        Arc::new(move || {
            let _ = tx.lock().map(|t| t.send(()));
        })
    }
}

impl Drop for FakeLive {
    fn drop(&mut self) {
        *self.open_now.lock().unwrap() -= 1;
    }
}
