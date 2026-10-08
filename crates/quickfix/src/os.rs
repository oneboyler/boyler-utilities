//! The OS layer: everything the Quick fixes ask Windows, behind one trait ([`FixOs`]). Real = `crate::real`,
//! fake = `crate::fake`. All decisions (admin checks, the 24-hour rule, progress parsing, ordering) live outside it.

use crate::Result;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A moment in time: seconds since 1970-01-01 UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Stamp(pub i64);

/// A local wall-clock time (what the row shows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LocalTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

/// One display adapter (graphics card / integrated GPU) as Device Manager lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayAdapter {
    pub name: String,
    /// `PCI\VEN_10DE&DEV_2684&…` — what `pnputil /restart-device` takes.
    pub instance_id: String,
}

/// Controls a started console program (DISM / sfc / pnputil). `kill` may be called from another thread while the
/// output is being read; it ends the program and everything it started.
pub trait ProcCtl: Send + Sync {
    fn kill(&self);
    /// Waits for the end and returns the exit code.
    fn wait(&self) -> Result<u32>;
}

/// A started program: its output (stdout; read until it ends) + its control.
pub struct Spawned {
    pub output: Box<dyn Read + Send>,
    pub ctl: Arc<dyn ProcCtl>,
}

/// File Explorer while it is stopped for the cache rebuild. `restart` brings it back (Restart Manager re-opens its
/// folder windows); dropping it without `restart` also brings it back — Explorer is never left stopped.
pub trait ExplorerPause: Send {
    fn restart(self: Box<Self>) -> Result<()>;
}

/// One restore point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestorePoint {
    pub created: Stamp,
    pub description: String,
    pub sequence: u32,
}

/// What Windows says about System Restore before we ask for a point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreStatus {
    /// Minutes Windows waits between two points (`SystemRestorePointCreationFrequency`; missing → 1440 = 24 h; 0 = no limit).
    pub frequency_minutes: u32,
    /// The newest restore point, `None` = none exists. `Err`-free: unreadable (no admin) shows as `newest_known = false`.
    pub newest: Option<RestorePoint>,
    /// `false` = the list could not be read (reading it needs admin) — `newest` is then unknown, not "none".
    pub newest_known: bool,
}

/// How `create_restore_point` ended at the Windows call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateCall {
    /// `SRSetRestorePointW` said yes (within Windows' 24-hour window it ALSO says yes but makes nothing — the service
    /// checks the newest point afterwards).
    Accepted,
    /// `ERROR_SERVICE_DISABLED` (1058): System Protection is off.
    ProtectionOff,
}

/// Everything the Quick fixes need from Windows.
pub trait FixOs: Send + Sync {
    /// The app runs elevated (admin).
    fn is_elevated(&self) -> bool;

    // ----- reset graphics driver -----
    /// The window in front belongs to this app (the menu) — the key chord is only ever sent then, never into a game.
    fn foreground_is_ours(&self) -> bool;
    /// Sends Win + Ctrl + Shift + B with `SendInput`.
    fn send_reset_chord(&self) -> Result<()>;
    /// The present display adapters (read-only).
    fn display_adapters(&self) -> Result<Vec<DisplayAdapter>>;

    // ----- console programs (DISM, sfc, pnputil) -----
    /// Starts `%SystemRoot%\System32\<program>` with `args`, no window, stdout captured.
    fn spawn(&self, program: &str, args: &[&str]) -> Result<Spawned>;
    /// The end of `%windir%\Logs\CBS\CBS.log` (sfc's detail log), at most a few MB.
    fn cbs_log_tail(&self) -> Result<String>;

    // ----- icon & thumbnail cache -----
    /// `%LocalAppData%\Microsoft\Windows\Explorer`.
    fn explorer_cache_dir(&self) -> Result<PathBuf>;
    /// Files directly in `dir`: (name, size in bytes).
    fn list_files(&self, dir: &Path) -> Result<Vec<(String, u64)>>;
    fn delete_file(&self, path: &Path) -> Result<()>;
    /// Stops File Explorer (Restart Manager) until the returned pause is restarted / dropped.
    fn stop_explorer(&self) -> Result<Box<dyn ExplorerPause>>;

    // ----- restore point -----
    fn restore_status(&self) -> Result<RestoreStatus>;
    /// `SRSetRestorePointW` BEGIN_SYSTEM_CHANGE / MODIFY_SETTINGS with `description`, then END_SYSTEM_CHANGE.
    fn create_restore_point(&self, description: &str) -> Result<CreateCall>;

    // ----- time -----
    fn now(&self) -> Stamp;
    fn local(&self, t: Stamp) -> LocalTime;
}
