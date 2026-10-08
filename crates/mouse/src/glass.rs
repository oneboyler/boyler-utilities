//! The app's own "Glass" cursor set (Order 040): 17 files, one per Windows cursor role, drawn as SVG and rendered by
//! `tools/cursorgen` into `app/assets/cursors/glass/` (every size Windows uses: 32 / 48 / 64 / 96 / 128 px). They are built
//! into the exe and written into `AppDirs::glass()` (`<data>\cursors\glass\`) when the Mouse tab opens - only the files
//! that are missing or differ (an update replaces older ones) - where [`crate::Mouse::glass_set`] finds them.
//!
//! File names are `WinRole::reg_name()` lower-case: `.ani` for the two animated roles (Busy, Working), `.cur` for the rest.

use crate::error::{Error, Result};
use crate::os::MouseOs;
use crate::service::Mouse;

macro_rules! glass {
    ($($name:literal),* $(,)?) => {
        [$(($name, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../app/assets/cursors/glass/", $name)) as &[u8])),*]
    };
}

/// (file name, bytes) of every Glass cursor, in `WinRole::ALL` order.
pub const FILES: [(&str, &[u8]); 17] = glass![
    "arrow.cur",
    "help.cur",
    "appstarting.ani",
    "wait.ani",
    "crosshair.cur",
    "ibeam.cur",
    "nwpen.cur",
    "no.cur",
    "sizens.cur",
    "sizewe.cur",
    "sizenwse.cur",
    "sizenesw.cur",
    "sizeall.cur",
    "uparrow.cur",
    "hand.cur",
    "pin.cur",
    "person.cur",
];

impl<O: MouseOs> Mouse<O> {
    /// Puts the Glass set into `AppDirs::glass()`: writes every file that is missing or differs from the built-in one
    /// (through a temporary file, then a rename, so a cursor Windows reads is never half written). Returns how many files
    /// were written (0 = all there already). Files of other names in the folder are left alone.
    pub fn install_glass(&self) -> Result<usize> {
        let dir = self.dirs.glass();
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(format!("create {}", dir.display()), e))?;
        let mut written = 0;
        let mut first_err = None;
        for (name, bytes) in FILES {
            let path = dir.join(name);
            // the same size first: a different one needs no read
            let same = std::fs::metadata(&path).map(|m| m.len() == bytes.len() as u64).unwrap_or(false)
                && std::fs::read(&path).map(|b| b == bytes).unwrap_or(false);
            if same {
                continue;
            }
            let tmp = dir.join(format!("{name}.new"));
            let r = std::fs::write(&tmp, bytes).map_err(|e| Error::io(format!("write {}", tmp.display()), e)).and_then(|_| {
                std::fs::rename(&tmp, &path).map_err(|e| {
                    let _ = std::fs::remove_file(&tmp);
                    Error::io(format!("replace {}", path.display()), e)
                })
            });
            // a file that fails doesn't stop the others; the first error is returned after all were tried
            match r {
                Ok(()) => written += 1,
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(written),
        }
    }
}
