//! The pretend Windows every test runs on. It keeps Defender's lists in memory, logs every call that would change
//! something (`log()`), and holds a scan open until the test lets it end (`release_scan`) or the user cancels.

use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use crate::error::{Result, SecurityError};
use crate::model::*;
use crate::os::{CancelToken, ScanExit, SecurityOs};

pub struct FakeState {
    pub elevated: bool,
    pub now: Stamp,
    /// `None` = Defender's WMI provider is missing.
    pub status: Option<DefenderStatus>,
    pub products: Vec<AvProduct>,
    pub detections: Vec<Detection>,
    pub threats: Vec<ThreatInfo>,
    /// Paths that "exist" for a path scan.
    pub existing: HashSet<String>,
    /// Detections that appear when the next scan ends.
    pub scan_adds: Vec<(Detection, ThreatInfo)>,
    /// The next scan fails with this.
    pub scan_error: Option<SecurityError>,
    /// The next call of a change fails with this (key = the log word: "update", "offline", "remove", "allow", "restore", "read allowed", "read detections").
    pub fail: Vec<(&'static str, SecurityError)>,
    /// Every call that reached the OS and would change something, in order.
    pub log: Vec<String>,
    pub scans_started: usize,
    /// The threat ids on Defender's allow list.
    pub allowed: Vec<i64>,
    /// A restore of exactly this file path fails once (to prove an undo that stops halfway).
    pub restore_fails_for: Option<String>,
    scan_release: bool,
    scan_cancelled: bool,
}

struct Shared {
    st: Mutex<FakeState>,
    cv: Condvar,
}

#[derive(Clone)]
pub struct FakeOs(Arc<Shared>);

pub fn sample_status() -> DefenderStatus {
    DefenderStatus {
        service_enabled: true,
        antivirus_enabled: true,
        realtime_enabled: true,
        tamper_protected: true,
        running_mode: RunningMode::Normal,
        definitions_version: "1.421.733.0".into(),
        definitions_updated: Some(Stamp::new(2026, 10, 8, 6, 40)),
        quick_scan_end: Some(Stamp::new(2026, 10, 8, 9, 12)),
        full_scan_end: None,
        reboot_required: false,
    }
}

impl FakeOs {
    /// A protected PC, elevated, now = 8 Oct 2026 10:00, nothing found.
    pub fn protected() -> FakeOs {
        FakeOs(Arc::new(Shared {
            st: Mutex::new(FakeState {
                elevated: true,
                now: Stamp::new(2026, 10, 8, 10, 0),
                status: Some(sample_status()),
                products: vec![AvProduct { name: "Windows Defender".into(), on: true, up_to_date: true }],
                detections: Vec::new(),
                threats: Vec::new(),
                existing: HashSet::new(),
                scan_adds: Vec::new(),
                scan_error: None,
                fail: Vec::new(),
                log: Vec::new(),
                scans_started: 0,
                allowed: Vec::new(),
                restore_fails_for: None,
                scan_release: false,
                scan_cancelled: false,
            }),
            cv: Condvar::new(),
        }))
    }

    pub fn state(&self) -> MutexGuard<'_, FakeState> {
        self.0.st.lock().unwrap()
    }
    pub fn log(&self) -> Vec<String> {
        self.state().log.clone()
    }
    pub fn set_elevated(&self, v: bool) {
        self.state().elevated = v;
    }
    pub fn add_threat(&self, info: ThreatInfo, detection: Detection) {
        let mut s = self.state();
        s.threats.push(info);
        s.detections.push(detection);
    }
    /// Let a held scan end normally.
    pub fn release_scan(&self) {
        self.state().scan_release = true;
        self.0.cv.notify_all();
    }
    /// Wait until `run_scan` is blocked inside the fake (a scan thread started).
    pub fn wait_scan_started(&self, timeout: Duration) -> bool {
        let g = self.state();
        let (g, _) = self.0.cv.wait_timeout_while(g, timeout, |s| s.scans_started == 0).unwrap();
        g.scans_started > 0
    }

    fn take_fail(&self, key: &'static str) -> Result<()> {
        let mut s = self.state();
        if let Some(i) = s.fail.iter().position(|(k, _)| *k == key) {
            return Err(s.fail.remove(i).1);
        }
        Ok(())
    }
}

/// A detection waiting for a choice (status 1, "Detected"), with its threat.
pub fn detection(id: &str, threat_id: i64, status_id: i64, found: Stamp, path: &str) -> Detection {
    Detection {
        detection_id: id.to_string(),
        threat_id,
        status_id,
        found: Some(found),
        status_changed: Some(found),
        resources: vec![format!("file:_{path}")],
    }
}

pub fn threat(threat_id: i64, name: &str, severity: Severity, active: bool) -> ThreatInfo {
    ThreatInfo { threat_id, name: name.to_string(), severity, active }
}

impl SecurityOs for FakeOs {
    fn is_elevated(&self) -> bool {
        self.state().elevated
    }
    fn now(&self) -> Stamp {
        self.state().now
    }
    fn defender_status(&self) -> Result<DefenderStatus> {
        self.state().status.clone().ok_or(SecurityError::DefenderNotRunning)
    }
    fn antivirus_products(&self) -> Result<Vec<AvProduct>> {
        Ok(self.state().products.clone())
    }
    fn detections(&self) -> Result<Vec<Detection>> {
        self.take_fail("read detections")?;
        Ok(self.state().detections.clone())
    }
    fn threats(&self) -> Result<Vec<ThreatInfo>> {
        Ok(self.state().threats.clone())
    }
    fn path_exists(&self, path: &str) -> bool {
        self.state().existing.contains(path)
    }

    fn run_scan(&self, kind: &ScanKind, cancel: &CancelToken) -> Result<ScanExit> {
        {
            let mut s = self.state();
            s.log.push(format!("scan {}", kind.title()));
            s.scans_started += 1;
            s.scan_release = false;
            s.scan_cancelled = false;
        }
        self.0.cv.notify_all();
        let shared = self.0.clone();
        cancel.on_cancel(move || {
            shared.st.lock().unwrap().scan_cancelled = true;
            shared.cv.notify_all();
        });
        let g = self.state();
        let mut g = self.0.cv.wait_while(g, |s| !s.scan_release && !s.scan_cancelled).unwrap();
        if g.scan_cancelled {
            g.log.push("scan cancelled".into());
            return Ok(ScanExit::Cancelled);
        }
        if let Some(e) = g.scan_error.take() {
            return Err(e);
        }
        let adds = std::mem::take(&mut g.scan_adds);
        for (d, t) in adds {
            g.detections.push(d);
            g.threats.push(t);
        }
        let now = g.now;
        if let Some(st) = g.status.as_mut() {
            match kind {
                ScanKind::Full => st.full_scan_end = Some(now),
                ScanKind::Quick => st.quick_scan_end = Some(now),
                ScanKind::Path(_) => {}
            }
        }
        Ok(ScanExit::Finished)
    }

    fn update_definitions(&self) -> Result<()> {
        self.take_fail("update")?;
        let mut s = self.state();
        s.log.push("update definitions".into());
        let now = s.now;
        if let Some(st) = s.status.as_mut() {
            st.definitions_updated = Some(now);
        }
        Ok(())
    }

    fn start_offline_scan(&self) -> Result<()> {
        self.take_fail("offline")?;
        self.state().log.push("offline scan (restart)".into());
        Ok(())
    }

    fn remove_active_threats(&self) -> Result<()> {
        self.take_fail("remove")?;
        let mut s = self.state();
        s.log.push("remove active threats".into());
        let active: Vec<i64> = s.threats.iter().filter(|t| t.active).map(|t| t.threat_id).collect();
        for d in s.detections.iter_mut().filter(|d| active.contains(&d.threat_id) && ThreatState::from_status_id(d.status_id) == ThreatState::NeedsChoice) {
            d.status_id = 3;
        }
        for t in s.threats.iter_mut() {
            t.active = false;
        }
        Ok(())
    }

    fn allow_threat(&self, threat_id: i64) -> Result<()> {
        self.take_fail("allow")?;
        let mut s = self.state();
        s.log.push(format!("allow {threat_id}"));
        // Only the allow list changes: what Windows does to a current detection when an Allow is added is unverified,
        // so the fake does not guess it (the service hides allowed threats itself).
        if !s.allowed.contains(&threat_id) {
            s.allowed.push(threat_id);
        }
        Ok(())
    }

    fn disallow_threat(&self, threat_id: i64) -> Result<()> {
        self.take_fail("allow")?;
        let mut s = self.state();
        s.log.push(format!("disallow {threat_id}"));
        s.allowed.retain(|id| *id != threat_id);
        Ok(())
    }

    fn restore_quarantined(&self, file_path: &str) -> Result<()> {
        self.take_fail("restore")?;
        {
            let mut s = self.state();
            if s.restore_fails_for.as_deref() == Some(file_path) {
                s.restore_fails_for = None;
                return Err(SecurityError::Os { call: "MpCmdRun -Restore".into(), code: 0x8050_8014, text: "restore failed".into() });
            }
        }
        let mut s = self.state();
        s.log.push(format!("restore {file_path}"));
        // The file is back at its original path; the detection history is left as it was (what the real history does after a
        // restore is unverified).
        s.existing.insert(file_path.to_string());
        Ok(())
    }

    fn open_protection_history(&self) -> Result<()> {
        self.state().log.push("open protection history".into());
        Ok(())
    }

    fn allowed_threat_ids(&self) -> Result<Vec<i64>> {
        self.take_fail("read allowed")?;
        Ok(self.state().allowed.clone())
    }
}
