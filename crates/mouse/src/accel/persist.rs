//! The acceleration card's state kept in the app's own file (`AppDirs::accel_file`), Order 063. Before it, the switch, the
//! presets and the per-game rows lived only in the open Mouse tab: every app start began from an empty card ("it is always
//! off again") and nothing was left to switch Raw Accel for a game.
//!
//! Only what the user set is kept (`Panel`, `PerApp` rows); which games run now is never saved. A file that can't be read
//! (damaged, or from a newer app) is left alone and the card starts empty — it is never overwritten by a failed load.

use super::panel::Panel;
use super::switch::PerApp;
use crate::error::{Error, Result};
use crate::os::MouseOs;
use crate::service::Mouse;
use serde::{Deserialize, Serialize};

/// The file format's version (a newer file than this is not read).
pub const FILE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    panel: Panel,
    per_app: PerApp,
}

impl<O: MouseOs> Mouse<O> {
    /// Loads the saved card into the service. `Ok(false)` = nothing saved yet (the first run: `mirror_rawaccel` decides).
    /// Nothing is written, and the driver is not touched.
    pub fn load_accel(&mut self) -> Result<bool> {
        let file = self.dirs.accel_file();
        let Some(bytes) = self.os.read_bytes(&file)? else {
            self.accel.load_failed = false;
            return Ok(false);
        };
        let saved: Saved = match serde_json::from_slice(&bytes) {
            Ok(s) => s,
            Err(e) => {
                self.accel.load_failed = true;
                return Err(Error::RawAccelSettings(format!("the saved acceleration settings {} can't be read ({e}); delete the file to start over", file.display())));
            }
        };
        if saved.version > FILE_VERSION {
            self.accel.load_failed = true;
            return Err(Error::RawAccelSettings(format!("the saved acceleration settings {} are from a newer version ({})", file.display(), saved.version)));
        }
        self.accel.load_failed = false;
        self.accel.panel = saved.panel;
        self.accel.per_app = saved.per_app;
        Ok(true)
    }

    /// Keeps the card (when it differs from the file). Returns true when it wrote.
    pub fn save_accel(&mut self) -> Result<bool> {
        if self.accel.load_failed {
            return Err(Error::RawAccelSettings(format!("the saved acceleration settings {} could not be read, so they are not overwritten", self.dirs.accel_file().display())));
        }
        let mut per_app = self.accel.per_app.clone();
        per_app.drop_blank_rows();
        let saved = Saved { version: FILE_VERSION, panel: self.accel.panel.clone(), per_app };
        let json = serde_json::to_vec_pretty(&saved).map_err(|e| Error::RawAccelSettings(format!("the acceleration card can't be saved: {e}")))?;
        let file = self.dirs.accel_file();
        if self.os.read_bytes(&file)?.as_deref() == Some(json.as_slice()) {
            return Ok(false);
        }
        self.os.write_bytes(&file, &json)?;
        Ok(true)
    }
}
