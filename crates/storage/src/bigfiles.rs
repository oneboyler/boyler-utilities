//! "Files" (Order 069, the owner OK): the biggest single files of a drive, out of the same walk as the folder tree - no extra
//! scan. [`crate::scan`] keeps the biggest [`BIGGEST_FILES`] of the whole walk next to the tree (a few KB), so they survive
//! the tree being cut down when the menu closes. Delete goes to the Recycle Bin ([`recycle`]), never straight away.

use crate::scan::is_windows_own;
use crate::{Result, StorageError, StorageOs};
use std::path::{Path, PathBuf};

/// How many files the Files list holds.
pub const BIGGEST_FILES: usize = 20;

/// One of the biggest files of a drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BigFile {
    pub name: String,
    /// The folder it is in.
    pub dir: PathBuf,
    pub bytes: u64,
}

impl BigFile {
    pub fn path(&self) -> PathBuf {
        self.dir.join(&self.name)
    }
    /// Delete is offered for it ([`can_recycle`]).
    pub fn can_recycle(&self) -> bool {
        can_recycle(&self.path())
    }
}

/// Windows' own places and the three files Windows keeps at the top of a drive are never deleted from here: anything under
/// `X:\Windows`, `X:\$Recycle.Bin`, `X:\System Volume Information`, `X:\Program Files\WindowsApps`, and `pagefile.sys` /
/// `hiberfil.sys` / `swapfile.sys` at a drive's top (Windows manages them; they are the biggest "files" of most drives).
pub fn can_recycle(path: &Path) -> bool {
    if !path.is_absolute() || path.file_name().is_none() {
        return false;
    }
    if path.ancestors().any(is_windows_own) {
        return false;
    }
    let at_top = path.parent().is_some_and(|p| p.parent().is_none());
    if at_top {
        let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        if matches!(name.as_str(), "pagefile.sys" | "hiberfil.sys" | "swapfile.sys") {
            return false;
        }
    }
    true
}

/// Move one file to the Recycle Bin (it can be restored from there). Refuses what [`can_recycle`] refuses.
pub fn recycle(os: &dyn StorageOs, path: &Path) -> Result<()> {
    if !can_recycle(path) {
        return Err(StorageError::UnsafePath(path.to_path_buf()));
    }
    os.recycle_file(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => StorageError::NotFound(path.display().to_string()),
        _ => StorageError::Io(e),
    })
}
