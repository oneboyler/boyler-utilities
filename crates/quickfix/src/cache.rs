//! Row 3 — **Rebuild icon & thumbnail cache**: stop File Explorer, delete `iconcache_*.db` + `thumbcache_*.db` in
//! `%LocalAppData%\Microsoft\Windows\Explorer`, start Explorer again (DESIGN "How"). No admin (the files are the
//! user's own). The row asks first (UI): [`CONFIRM`].
//!
//! Explorer is always started again, also when a delete fails (a file another app holds open is skipped and counted).

use crate::os::FixOs;
use crate::Result;
use std::path::PathBuf;

/// The mini confirm's text (DESIGN).
pub const CONFIRM: &str = "File Explorer restarts: the taskbar blinks once and open Explorer windows close.";

/// One cache file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheFile {
    pub path: PathBuf,
    pub bytes: u64,
}

/// `iconcache_*.db` / `thumbcache_*.db` (any case) — the only files this row ever deletes.
pub fn is_cache_file(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    (n.starts_with("iconcache_") || n.starts_with("thumbcache_")) && n.ends_with(".db")
}

/// The cache files now (read-only).
pub fn cache_files(os: &dyn FixOs) -> Result<Vec<CacheFile>> {
    let dir = os.explorer_cache_dir()?;
    let mut out: Vec<CacheFile> = os
        .list_files(&dir)?
        .into_iter()
        .filter(|(n, _)| is_cache_file(n))
        .map(|(n, bytes)| CacheFile { path: dir.join(n), bytes })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// What the rebuild did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildReport {
    pub deleted: Vec<CacheFile>,
    /// Files that could not be deleted (in use by another app) — Windows rebuilds the rest; these stay.
    pub skipped: Vec<(CacheFile, String)>,
}

impl RebuildReport {
    pub fn bytes_freed(&self) -> u64 {
        self.deleted.iter().map(|f| f.bytes).sum()
    }
    /// DESIGN: "✓ Rebuilt just now · icons refill as you browse".
    pub fn line(&self) -> String {
        "✓ Rebuilt just now · icons refill as you browse".into()
    }
}

/// Stops Explorer, deletes the cache files, starts Explorer. Call only after the user confirmed ([`CONFIRM`]).
pub fn rebuild(os: &dyn FixOs) -> Result<RebuildReport> {
    let dir = os.explorer_cache_dir()?;
    let pause = os.stop_explorer()?;
    // the list is read with Explorer stopped (it may write new ones until then)
    let listed = os.list_files(&dir);
    let mut deleted = Vec::new();
    let mut skipped = Vec::new();
    if let Ok(files) = &listed {
        for (name, bytes) in files.iter().filter(|(n, _)| is_cache_file(n)) {
            let f = CacheFile { path: dir.join(name), bytes: *bytes };
            match os.delete_file(&f.path) {
                Ok(()) => deleted.push(f),
                Err(e) => skipped.push((f, e.to_string())),
            }
        }
    }
    // always bring Explorer back, whatever happened above
    let restarted = pause.restart();
    listed?;
    restarted?;
    Ok(RebuildReport { deleted, skipped })
}
