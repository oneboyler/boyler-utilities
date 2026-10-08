//! Test-only scratch folders: `C:\BoylerUtilities-scratch\K\settings-test\<unique>\`, removed
//! when the [`Scratch`] is dropped. Tests never touch %APPDATA%.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

const ROOT: &str = r"C:\BoylerUtilities-scratch\K\settings-test";

pub struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    /// A new, empty, unique folder (not created yet — the store creates it on its first write).
    pub fn new(tag: &str) -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = PathBuf::from(ROOT).join(format!(
            "{tag}-{}-{}-{nanos}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        Scratch { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
