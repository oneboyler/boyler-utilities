//! The one door to Windows. [`crate::real::RealOs`] talks to Windows; [`crate::FakeOs`] is what every test uses.

use std::sync::{Arc, Mutex};

use crate::error::Result;
use crate::model::{AvProduct, DefenderStatus, Detection, ScanKind, Stamp, ThreatInfo};

/// How a blocking scan ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanExit {
    /// Defender finished the scan (threats, if any, are in its detection list afterwards).
    Finished,
    /// The user cancelled it.
    Cancelled,
}

/// Lets the user's Cancel reach a scan that is blocked inside [`SecurityOs::run_scan`]. The scan registers one hook
/// (kill the child, tell Defender to stop); `cancel` runs it. No polling anywhere.
#[derive(Clone, Default)]
pub struct CancelToken(Arc<Mutex<CancelInner>>);

#[derive(Default)]
struct CancelInner {
    cancelled: bool,
    hook: Option<Box<dyn FnOnce() + Send>>,
}

impl CancelToken {
    pub fn new() -> CancelToken {
        CancelToken::default()
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.lock().unwrap().cancelled
    }
    /// Ask the scan to stop. Safe to call twice.
    pub fn cancel(&self) {
        let hook = {
            let mut g = self.0.lock().unwrap();
            g.cancelled = true;
            g.hook.take()
        };
        if let Some(h) = hook {
            h();
        }
    }
    /// Register what to do on cancel; runs at once if cancel already happened.
    pub fn on_cancel(&self, f: impl FnOnce() + Send + 'static) {
        let mut g = self.0.lock().unwrap();
        if g.cancelled {
            drop(g); // never hold the lock while a hook runs
            f();
        } else {
            g.hook = Some(Box::new(f));
        }
    }
}

/// Everything the Security features need from Windows. The changes are the methods below `run_scan`. Every method may block (WMI, child programs): the menu calls them off its UI thread.
pub trait SecurityOs: Send + Sync {
    fn is_elevated(&self) -> bool;
    /// The local time now (the page labels "Today 09:12" against it).
    fn now(&self) -> Stamp;
    /// `Get-MpComputerStatus`. Fails when Defender's WMI provider is not there.
    fn defender_status(&self) -> Result<DefenderStatus>;
    /// Security Center's antivirus list (who protects this PC).
    fn antivirus_products(&self) -> Result<Vec<AvProduct>>;
    /// `Get-MpThreatDetection`: active and past detections (works without admin: shown by security-show from a non-elevated shell).
    fn detections(&self) -> Result<Vec<Detection>>;
    /// `Get-MpThreat`: name + severity per threat id.
    fn threats(&self) -> Result<Vec<ThreatInfo>>;
    /// Does this file / folder exist (a dropped path is checked before a scan starts).
    fn path_exists(&self, path: &str) -> bool;

    /// Run one scan and BLOCK until it ends (the service runs this on its own thread, only after the user's button).
    fn run_scan(&self, kind: &ScanKind, cancel: &CancelToken) -> Result<ScanExit>;
    /// `Update-MpSignature`.
    fn update_definitions(&self) -> Result<()>;
    /// `Start-MpWDOScan`: restarts the PC into Defender Offline.
    fn start_offline_scan(&self) -> Result<()>;
    /// `Remove-MpThreat`: Windows removes ALL active threats (it has no per-threat form).
    fn remove_active_threats(&self) -> Result<()>;
    /// `Add-MpPreference -ThreatIDDefaultAction_Ids <id> -ThreatIDDefaultAction_Actions Allow`.
    fn allow_threat(&self, threat_id: i64) -> Result<()>;
    /// The inverse: `Remove-MpPreference -ThreatIDDefaultAction_Ids <id>`.
    fn disallow_threat(&self, threat_id: i64) -> Result<()>;
    /// `MpCmdRun -Restore -FilePath <file>`: puts back the quarantined copy of ONE file (not every file of that threat name).
    fn restore_quarantined(&self, file_path: &str) -> Result<()>;
    /// Opens Windows Security's Protection history (`windowsdefender://history`) on the screen: the only place where Windows' own
    /// "Remove" for one quarantined item exists (no supported command does it). Answer A_016_01 (boss).
    fn open_protection_history(&self) -> Result<()>;
    /// The threat ids Defender is told to allow (`ThreatIDDefaultAction_Ids` whose `_Actions` entry is 6 = Allow).
    /// Whether this read works without admin is **not proven** (Get-MpPreference hides the exclusion lists without admin).
    fn allowed_threat_ids(&self) -> Result<Vec<i64>>;
}
