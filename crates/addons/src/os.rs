//! The OS layer behind a trait: [`crate::real::RealOs`] (Windows) and [`crate::fake::FakeOs`] (tests).

use crate::error::Result;
use std::path::Path;

/// What the elevated helper is asked to do (its command line: `--addon-helper <name> <folder>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperAction {
    /// Raw Accel's installer.exe in the folder (copies the driver, adds the mouse filter; restart needed).
    RawAccelInstall,
    /// Raw Accel's uninstaller.exe in the folder (removes the filter, the driver file goes at the restart).
    RawAccelUninstall,
}

impl HelperAction {
    pub fn name(self) -> &'static str {
        match self {
            HelperAction::RawAccelInstall => "rawaccel-install",
            HelperAction::RawAccelUninstall => "rawaccel-uninstall",
        }
    }
    pub fn parse(s: &str) -> Option<HelperAction> {
        match s {
            "rawaccel-install" => Some(HelperAction::RawAccelInstall),
            "rawaccel-uninstall" => Some(HelperAction::RawAccelUninstall),
            _ => None,
        }
    }
}

/// How the elevated helper ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Elevated {
    /// The tool said it worked ("Install complete" / "Removal complete" / "No installed driver found").
    Done,
    /// The admin prompt was answered No.
    Declined,
    /// It failed: the tool's own error line, or why the helper refused.
    Failed(String),
}

pub trait AddonOs: Send + Sync {
    /// Raw Accel's driver is set up: "rawaccel" is in the mouse class's UpperFilters (what its installer adds and its
    /// uninstaller removes). Read-only.
    fn rawaccel_filter_set(&self) -> bool;
    /// Raw Accel's driver runs now (its device `\\.\rawaccel` opens). Read-only.
    fn rawaccel_running(&self) -> bool;
    /// Windows checks the file's Authenticode signature (WinVerifyTrust). Ok = signed and trusted.
    fn verify_signature(&self, file: &Path) -> Result<()>;
    /// Run the helper with Windows' admin prompt and wait for it (`stop()` = the app quits: stop waiting).
    fn run_elevated(&self, action: HelperAction, folder: &Path, stop: &dyn Fn() -> bool) -> Elevated;
}
