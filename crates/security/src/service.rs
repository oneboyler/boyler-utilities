//! The commands of the Security tab (DESIGN.md §3.14): read the page, start / cancel a scan, update definitions, the
//! offline scan, Remove / Allow / Restore, undo. A scan is a "slow job": it starts ONLY from [`SecurityService::start_scan`]
//! (the user's button) and runs on a thread that exists only while the scan runs and waits on the process, never polls.

use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::error::{Result, SecurityError};
use crate::model::*;
use crate::os::{CancelToken, ScanExit, SecurityOs};

/// A user action, for asking "does this need admin?" before doing it (the shield + "Windows asks once").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    QuickScan,
    FullScan,
    /// Scan a dropped / picked file or folder.
    ScanPath,
    UpdateDefinitions,
    /// Restarts the PC (admin, WinRE).
    OfflineScan,
    /// Remove (→ Quarantine).
    RemoveThreat,
    /// Allow (asks first).
    AllowThreat,
    /// Restore from Quarantine (asks first). Also adds the Allow.
    RestoreQuarantined,
    /// Take an Allow away again (the reset line "Allowed in Defender").
    RemoveAllow,
    /// Delete from Quarantine - Windows has no supported command for it (the page opens Windows Security instead, see
    /// `SecurityService::open_protection_history`).
    DeleteQuarantined,
}

/// Whether an action needs admin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminNeed {
    No,
    Yes,
    /// Microsoft does not say (scans and definition updates): the action is tried and, if Windows answers
    /// "access denied", the error is [`SecurityError::NeedsAdmin`]. Never proven on the real PC: tests start no scan.
    Unknown,
}

impl Action {
    pub fn admin_need(self) -> AdminNeed {
        match self {
            Action::QuickScan | Action::FullScan | Action::ScanPath | Action::UpdateDefinitions => AdminNeed::Unknown,
            _ => AdminNeed::Yes,
        }
    }
}

/// How a scan ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanOutcome {
    Finished,
    Cancelled,
    Failed(SecurityError),
}

/// What the scan left behind: the page shows "Quick scan done · no threats found" or what was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanReport {
    pub kind: ScanKind,
    pub outcome: ScanOutcome,
    pub finished: Stamp,
    /// EVERY detection that did not exist before the scan, with its state. Defender handles High / Severe threats on its
    /// own (quarantine / remove / clean), so a fresh row may already be `Quarantined`, `Removed` or `Handled`: the scan
    /// still found something. Only the `NeedsChoice` ones wait for Remove / Allow (see [`ScanReport::waiting`]).
    pub new_threats: Vec<ThreatRow>,
}

impl ScanReport {
    /// "<title> done · no threats found": the scan finished and found NOTHING (not even something Defender dealt with itself).
    pub fn is_clean(&self) -> bool {
        self.outcome == ScanOutcome::Finished && self.new_threats.is_empty()
    }
    /// The found threats that still wait for the user's choice (the drop zone shows "Threat found: <name>" for the first).
    pub fn waiting(&self) -> Vec<&ThreatRow> {
        self.new_threats.iter().filter(|r| r.state == ThreatState::NeedsChoice).collect()
    }
    /// The found threats Defender already dealt with on its own (quarantined, removed, cleaned, blocked).
    pub fn handled_by_defender(&self) -> Vec<&ThreatRow> {
        self.new_threats.iter().filter(|r| matches!(r.state, ThreatState::Quarantined | ThreatState::Removed | ThreatState::Handled)).collect()
    }
}

/// The scan right now. This crate reports only the elapsed time (the bar is indeterminate): `Start-MpScan` and `MpCmdRun -Scan`
/// print no progress, so the page's "28 % · 13,119 files · about 4 min left", the current path and "87 of 214 files" have no
/// source here. The report names the one documented route that might give them (the MpClient API); it is not built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanState {
    Idle,
    Running { kind: ScanKind, elapsed: Duration },
}

struct Slot {
    running: Option<Running>,
    last: Option<ScanReport>,
}

struct Running {
    kind: ScanKind,
    started: Instant,
    cancel: CancelToken,
}

struct Shared {
    slot: Mutex<Slot>,
    cv: Condvar,
}

/// What a change did, so it can be undone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub id: u64,
    pub kind: ChangeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    /// Windows removed every active threat; `files` (full paths) are what was waiting and should be in Quarantine now (undo =
    /// restore them one by one; the files still to restore are kept when an undo fails halfway).
    Removed { file: String, files: Vec<String> },
    /// Defender leaves this threat alone from now on (undo = take the rule away).
    Allowed { file: String, threat_id: i64 },
}

impl Change {
    /// The toast after the change (DESIGN §3.14 wording).
    pub fn toast(&self) -> String {
        match &self.kind {
            ChangeKind::Removed { file, .. } => format!("{file} removed · in Quarantine now"),
            ChangeKind::Allowed { file, .. } => format!("{file} allowed"),
        }
    }
}

pub struct SecurityService {
    os: Arc<dyn SecurityOs>,
    shared: Arc<Shared>,
    changes: Mutex<(u64, Vec<Change>)>,
}

impl SecurityService {
    pub fn new(os: Arc<dyn SecurityOs>) -> SecurityService {
        SecurityService {
            os,
            shared: Arc::new(Shared { slot: Mutex::new(Slot { running: None, last: None }), cv: Condvar::new() }),
            changes: Mutex::new((0, Vec::new())),
        }
    }

    /// The service on the real PC.
    #[cfg(windows)]
    pub fn real() -> SecurityService {
        SecurityService::new(Arc::new(crate::real::RealOs::new()))
    }

    pub fn os(&self) -> Arc<dyn SecurityOs> {
        self.os.clone()
    }

    pub fn now(&self) -> Stamp {
        self.os.now()
    }

    // ---------------------------------------------------------------------------------------------- reading

    /// Everything the page shows. Starts nothing. Takes 1-2 s on a real PC (WMI): call it off the UI thread.
    ///
    /// Every action below that calls `defender_ready()` (two WMI connects) or `page()` blocks the caller the same way: the UI
    /// calls them off its own thread. Measured with security-show: `page()` took about 0.1 s on the author's PC.
    pub fn page(&self) -> Result<SecurityPage> {
        let antivirus = self.os.antivirus_products().unwrap_or_default();
        let status = match self.os.defender_status() {
            Ok(s) => s,
            // Defender's WMI provider is gone while another antivirus runs: say so instead of failing.
            Err(e) => {
                if antivirus.iter().any(|p| p.on && !p.is_defender()) {
                    unavailable_status()
                } else {
                    return Err(e);
                }
            }
        };
        // A detection list that cannot be read (access denied, the 10 s WMI timeout while Defender is busy) must never look like
        // "nothing found": the error stays on the page and the banner is never Protected while it is set.
        let (detections, unreadable) = match self.os.detections() {
            Ok(d) => (d, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let threats = self.os.threats().unwrap_or_default();
        let rows = rows_from(&detections, &threats);

        let allowed_ids: HashSet<i64> = self.os.allowed_threat_ids().unwrap_or_default().into_iter().collect();

        // The newest detection of each file decides (rows are newest first), so one file shows once. Threats found: only the
        // ones still waiting for a choice and not on the allow list (whether Windows flips a current detection at once when an
        // Allow is added is unverified, so the page does not rely on it). Quarantine: only "still quarantined" ones, and not
        // the ones whose threat is allowed and whose file is back at its original path (see below).
        let active_ids: HashSet<i64> = threats.iter().filter(|t| t.active).map(|t| t.threat_id).collect();
        let mut seen: HashSet<(i64, String)> = HashSet::new();
        let mut waiting: Vec<ThreatRow> = Vec::new();
        let mut quarantine = Vec::new();
        for r in &rows {
            if !seen.insert((r.threat_id, r.path().to_ascii_lowercase())) {
                continue;
            }
            let allowed = allowed_ids.contains(&r.threat_id);
            if r.state == ThreatState::NeedsChoice && active_ids.contains(&r.threat_id) && !allowed {
                waiting.push(r.clone());
            }
            // Only the file that came back is hidden, not every file of the threat id: an allowed threat id AND its original
            // path existing again = restored. (Whether the history still says "Quarantined" after a restore is unverified.)
            if r.state == ThreatState::Quarantined && !(allowed && self.os.path_exists(&r.path())) {
                quarantine.push(r.clone());
            }
        }
        let allowed = allowed_list(&allowed_ids, &rows, &threats);

        let scanning = self.scan_state();
        let title = match &scanning {
            ScanState::Running { kind, .. } => Some(kind.title()),
            ScanState::Idle => None,
        };
        let mut banner = banner_for(&status, &antivirus, waiting.len(), title.as_deref());
        if unreadable.is_some() && banner == Banner::Protected {
            banner = Banner::CannotRead;
        }
        Ok(SecurityPage {
            banner,
            last_scan: last_scan_of(&status),
            status,
            antivirus,
            threats: waiting,
            quarantine,
            quarantine_from_history: true,
            allowed,
            unreadable,
        })
    }

    pub fn needs_admin(&self, action: Action) -> AdminNeed {
        action.admin_need()
    }

    /// Can the action be started right now (for greying buttons)? `Err` says why not.
    pub fn can_start_scan(&self) -> Result<()> {
        if self.is_scanning() {
            return Err(SecurityError::ScanRunning);
        }
        self.defender_ready()
    }

    fn defender_ready(&self) -> Result<()> {
        let products = self.os.antivirus_products().unwrap_or_default();
        let status = self.os.defender_status().map_err(|_| SecurityError::DefenderNotRunning)?;
        if let Some(name) = other_antivirus(&status, &products) {
            return Err(SecurityError::OtherAntivirus(name));
        }
        if status.running_mode == RunningMode::NotRunning || !status.service_enabled {
            return Err(SecurityError::DefenderNotRunning);
        }
        Ok(())
    }

    // ---------------------------------------------------------------------------------------------- scans

    pub fn scan_state(&self) -> ScanState {
        let g = self.shared.slot.lock().unwrap();
        match &g.running {
            Some(r) => ScanState::Running { kind: r.kind.clone(), elapsed: r.started.elapsed() },
            None => ScanState::Idle,
        }
    }

    pub fn is_scanning(&self) -> bool {
        self.shared.slot.lock().unwrap().running.is_some()
    }

    /// The last finished scan's report (until the next scan starts).
    pub fn last_report(&self) -> Option<ScanReport> {
        self.shared.slot.lock().unwrap().last.clone()
    }

    /// Start a scan. Call this ONLY from the user's button (or a drop). `on_done` runs on the scan's thread when it ends.
    /// Quick scan, Full scan and a file / folder; one at a time.
    pub fn start_scan(&self, kind: ScanKind, on_done: impl FnOnce(ScanReport) + Send + 'static) -> Result<()> {
        if self.is_scanning() {
            return Err(SecurityError::ScanRunning);
        }
        if let ScanKind::Path(p) = &kind {
            if !self.os.path_exists(p) {
                return Err(SecurityError::PathMissing(p.clone()));
            }
        }
        self.defender_ready()?;

        let cancel = CancelToken::new();
        {
            let mut g = self.shared.slot.lock().unwrap();
            if g.running.is_some() {
                return Err(SecurityError::ScanRunning);
            }
            g.running = Some(Running { kind: kind.clone(), started: Instant::now(), cancel: cancel.clone() });
        }
        let os = self.os.clone();
        let shared = self.shared.clone();
        let spawned = std::thread::Builder::new().name("bu-security-scan".into()).spawn(move || {
            // A detection list that cannot be read must never turn into "no threats found" (before: every old detection would
            // look new; after: the result would be unknown), so a failed read makes the scan Failed.
            let (outcome, new_threats) = match os.detections() {
                Err(e) => (ScanOutcome::Failed(e), Vec::new()),
                Ok(old) => {
                    let before: HashSet<String> = old.into_iter().map(|d| d.detection_id).collect();
                    match os.run_scan(&kind, &cancel) {
                        Ok(ScanExit::Cancelled) => (ScanOutcome::Cancelled, new_threat_rows(os.as_ref(), &before).unwrap_or_default()),
                        Err(e) => (ScanOutcome::Failed(e), Vec::new()),
                        Ok(ScanExit::Finished) => match new_threat_rows(os.as_ref(), &before) {
                            Ok(rows) => (ScanOutcome::Finished, rows),
                            Err(e) => (ScanOutcome::Failed(e), Vec::new()),
                        },
                    }
                }
            };
            let report = ScanReport { kind: kind.clone(), outcome, finished: os.now(), new_threats };
            {
                let mut g = shared.slot.lock().unwrap();
                g.running = None;
                g.last = Some(report.clone());
            }
            shared.cv.notify_all();
            on_done(report);
        });
        if let Err(e) = spawned {
            self.shared.slot.lock().unwrap().running = None;
            return Err(SecurityError::Os { call: "start scan thread".into(), code: e.raw_os_error().unwrap_or(0) as u32, text: e.to_string() });
        }
        Ok(())
    }

    /// Cancel the running scan (the Cancel link). Does not wait for the scan to end (the real hook only kills our PowerShell and spawns
    /// `MpCmdRun -Scan -Cancel`); the scan's `on_done` reports `Cancelled`. See the real layer for what Cancel does to a path scan.
    pub fn cancel_scan(&self) -> Result<()> {
        let token = {
            let g = self.shared.slot.lock().unwrap();
            g.running.as_ref().map(|r| r.cancel.clone())
        };
        match token {
            Some(t) => {
                t.cancel();
                Ok(())
            }
            None => Err(SecurityError::NoScanRunning),
        }
    }

    /// Wait until no scan runs (tests / app shutdown). `true` = idle now.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let g = self.shared.slot.lock().unwrap();
        let (g, _) = self.shared.cv.wait_timeout_while(g, timeout, |s| s.running.is_some()).unwrap();
        g.running.is_none()
    }

    /// Check for new definitions now (the page's button, if it has one). Admin need unknown (see [`AdminNeed::Unknown`]).
    pub fn update_definitions(&self) -> Result<()> {
        self.defender_ready()?;
        self.os.update_definitions()
    }

    /// The offline scan: the PC restarts into Defender Offline (about 15 minutes). `restart_confirmed` must be true,
    /// i.e. the page showed "Restart and scan?" and the user pressed the button. Needs admin (the page says so).
    pub fn offline_scan(&self, restart_confirmed: bool) -> Result<()> {
        if !restart_confirmed {
            return Err(SecurityError::RestartNotConfirmed);
        }
        self.gate(Action::OfflineScan)?;
        if self.is_scanning() {
            return Err(SecurityError::ScanRunning);
        }
        self.defender_ready()?;
        self.os.start_offline_scan()
    }

    // ---------------------------------------------------------------------------------------------- threats

    fn gate(&self, action: Action) -> Result<()> {
        if action.admin_need() == AdminNeed::Yes && !self.os.is_elevated() {
            return Err(SecurityError::NeedsAdmin);
        }
        Ok(())
    }

    fn record(&self, kind: ChangeKind) -> Change {
        let mut g = self.changes.lock().unwrap();
        g.0 += 1;
        let c = Change { id: g.0, kind };
        g.1.push(c.clone());
        c
    }

    fn waiting_threats(&self) -> Result<Vec<ThreatRow>> {
        let page = self.page()?;
        match page.unreadable {
            Some(e) => Err(e), // the real read error, not "no such threat"
            None => Ok(page.threats),
        }
    }

    /// "Remove". Windows has no per-threat remove: it removes every active threat, and they all go to Quarantine.
    /// `threat_id` must be one of the rows waiting for a choice.
    pub fn remove_threat(&self, threat_id: i64) -> Result<Change> {
        self.gate(Action::RemoveThreat)?;
        self.defender_ready()?;
        let waiting = self.waiting_threats()?;
        let Some(row) = waiting.iter().find(|r| r.threat_id == threat_id) else { return Err(SecurityError::NoSuchThreat(threat_id)) };
        let file = row.file.clone();
        let mut files: Vec<String> = waiting.iter().map(|r| r.path()).collect();
        files.sort();
        files.dedup();
        // What `Remove-MpThreat` does to the current detections (quarantine, delete or clean) is not proven: the page's toast
        // says "in Quarantine now" as the drawing does, and the undo restores by file, which only works for quarantined ones.
        self.os.remove_active_threats()?;
        Ok(self.record(ChangeKind::Removed { file, files }))
    }

    /// "Allow" (the page asks first). Defender stops acting on this threat id.
    pub fn allow_threat(&self, threat_id: i64) -> Result<Change> {
        self.gate(Action::AllowThreat)?;
        self.defender_ready()?;
        let waiting = self.waiting_threats()?;
        let Some(row) = waiting.iter().find(|r| r.threat_id == threat_id) else { return Err(SecurityError::NoSuchThreat(threat_id)) };
        let file = row.file.clone();
        self.os.allow_threat(threat_id)?;
        Ok(self.record(ChangeKind::Allowed { file, threat_id }))
    }

    /// "Restore" from Quarantine (the page asks first and says "Defender allows it from now on"): puts back THIS file
    /// (`MpCmdRun -Restore -FilePath`, not every file with that threat name) and then adds the Allow for its threat id, so a
    /// later scan does not quarantine it again. The row is picked by threat id AND file path (one threat id often covers
    /// several files). The Allow goes first and the file second (real-time protection might quarantine a restored file again
    /// before a late Allow lands - unverified); if the restore fails, the Allow is taken away again. The Allow can also be taken
    /// away later with [`SecurityService::remove_allow`]; the restore itself is not undone. No test runs this against the real
    /// quarantine: the real effect of both calls is unverified. Returns the toast text "<file> restored to <folder>".
    pub fn restore_quarantined(&self, threat_id: i64, file_path: &str) -> Result<String> {
        self.gate(Action::RestoreQuarantined)?;
        self.defender_ready()?;
        let page = self.page()?;
        if let Some(e) = page.unreadable {
            return Err(e);
        }
        let Some(row) = page.quarantine.iter().find(|r| r.threat_id == threat_id && r.path().eq_ignore_ascii_case(file_path)) else {
            return Err(SecurityError::NoSuchThreat(threat_id));
        };
        // `None` = the allow list could not be read: then the Allow is not rolled back (it might have been the user's own).
        let already_allowed = self.os.allowed_threat_ids().ok().map(|ids| ids.contains(&threat_id));
        self.os.allow_threat(threat_id)?;
        if let Err(e) = self.os.restore_quarantined(&row.path()) {
            if already_allowed == Some(false) && self.os.disallow_threat(threat_id).is_err() {
                return Err(SecurityError::Os {
                    call: "MpCmdRun -Restore".into(),
                    code: 0,
                    text: format!("{e}; and the Allow added for it could not be taken away again (remove it with remove_allow)"),
                });
            }
            return Err(e);
        }
        Ok(format!("{} restored to {}", row.file, row.folder_name()))
    }

    /// The allow list as the reset line shows it ("Allowed in Defender · 1 file → none"). Read from Defender itself, so it also
    /// holds what an earlier run of the app (or Windows Security) allowed. Whether it can be read without admin is unproven.
    pub fn allowed_in_defender(&self) -> Result<Vec<AllowedThreat>> {
        let ids: HashSet<i64> = self.os.allowed_threat_ids()?.into_iter().collect();
        let threats = self.os.threats().unwrap_or_default();
        let rows = rows_from(&self.os.detections().unwrap_or_default(), &threats);
        Ok(allowed_list(&ids, &rows, &threats))
    }

    /// Take ONE Allow away (admin). Works from the list Defender holds, so it needs no change id (those live in memory only).
    pub fn remove_allow(&self, threat_id: i64) -> Result<()> {
        self.gate(Action::RemoveAllow)?;
        if !self.os.allowed_threat_ids()?.contains(&threat_id) {
            return Err(SecurityError::NoSuchThreat(threat_id));
        }
        self.os.disallow_threat(threat_id)
    }

    /// Put ONE Allow (back) by threat id - the reset's "how your PC was" for an Allow the app took away (admin). Already
    /// allowed = nothing to do.
    pub fn add_allow(&self, threat_id: i64) -> Result<()> {
        self.gate(Action::AllowThreat)?;
        if self.os.allowed_threat_ids()?.contains(&threat_id) {
            return Ok(());
        }
        self.os.allow_threat(threat_id)
    }

    /// The Security reset "Windows defaults" (nothing allowed): take EVERY Allow away, also ones made in Windows Security before
    /// the app. Returns how many were removed. Stops at the first failure (the ones already removed stay removed). For "Back to
    /// how your PC was", the reset framework should call [`SecurityService::remove_allow`] only for the ids the app added.
    pub fn reset_allowed(&self) -> Result<usize> {
        self.gate(Action::RemoveAllow)?;
        let ids = self.os.allowed_threat_ids()?;
        for id in &ids {
            self.os.disallow_threat(*id)?;
        }
        Ok(ids.len())
    }

    /// "Delete" from Quarantine: **not available.** Windows documents no command that deletes one quarantined item
    /// (`MpCmdRun -Restore` only restores / lists; the Windows Security app uses an internal call). Defender purges
    /// quarantined items itself after `QuarantinePurgeItemsAfterDelay` (90 days by default).
    pub fn delete_quarantined(&self, _threat_id: i64) -> Result<()> {
        Err(SecurityError::Unsupported(
            "Windows has no supported command to delete one quarantined item; Defender empties its quarantine on its own".into(),
        ))
    }

    /// The page's "Delete" on a Quarantine row: opens Windows Security's Protection history, where Windows' own Remove exists
    /// ([`SecurityService::delete_quarantined`] stays unsupported). A call nobody approved explicitly: the boss chose it
    /// (A_016_01). No admin (unlike [`Action::DeleteQuarantined`], which the drawing marks as admin: the page lane picks which to
    /// show); opens a window, so call it only from the user's click. The URI is from A_016_01 and was never run (unverified).
    pub fn open_protection_history(&self) -> Result<()> {
        self.os.open_protection_history()
    }

    /// Put an Allow back / restore what a Remove quarantined.
    pub fn undo(&self, change_id: u64) -> Result<Change> {
        let c = {
            let mut g = self.changes.lock().unwrap();
            let Some(i) = g.1.iter().position(|c| c.id == change_id) else { return Err(SecurityError::NothingToUndo) };
            g.1.remove(i)
        };
        let mut c = c;
        let done = match c.kind.clone() {
            ChangeKind::Allowed { threat_id, .. } => self.gate(Action::AllowThreat).and_then(|_| self.os.disallow_threat(threat_id)),
            ChangeKind::Removed { file, files } => self.gate(Action::RestoreQuarantined).and_then(|_| {
                // The files already back are dropped from the change, so a retry after a halfway failure does not restore them twice.
                let mut left = files.clone();
                while let Some(f) = left.first() {
                    if let Err(e) = self.os.restore_quarantined(f) {
                        c.kind = ChangeKind::Removed { file: file.clone(), files: left };
                        return Err(e);
                    }
                    left.remove(0);
                }
                Ok(())
            }),
        };
        match done {
            Ok(()) => Ok(c),
            Err(e) => {
                self.changes.lock().unwrap().1.push(c); // still undoable later (e.g. after elevating)
                Err(e)
            }
        }
    }
}

fn unavailable_status() -> DefenderStatus {
    DefenderStatus {
        service_enabled: false,
        antivirus_enabled: false,
        realtime_enabled: false,
        tamper_protected: false,
        running_mode: RunningMode::NotRunning,
        definitions_version: String::new(),
        definitions_updated: None,
        quick_scan_end: None,
        full_scan_end: None,
        reboot_required: false,
    }
}

/// Every detection that did not exist before the scan, whatever happened to it (Defender may have dealt with it already).
fn new_threat_rows(os: &dyn SecurityOs, before: &HashSet<String>) -> Result<Vec<ThreatRow>> {
    let detections = os.detections()?;
    let threats = os.threats().unwrap_or_default();
    let fresh: Vec<Detection> = detections.into_iter().filter(|d| !before.contains(&d.detection_id)).collect();
    Ok(rows_from(&fresh, &threats))
}

fn allowed_list(ids: &HashSet<i64>, rows: &[ThreatRow], threats: &[ThreatInfo]) -> Vec<AllowedThreat> {
    let mut sorted: Vec<i64> = ids.iter().copied().collect();
    sorted.sort_unstable();
    sorted
        .into_iter()
        .map(|id| {
            let name = threats.iter().find(|t| t.threat_id == id).map(|t| t.name.clone()).unwrap_or_else(|| format!("Threat {id}"));
            let mut files: Vec<String> = rows.iter().filter(|r| r.threat_id == id && !r.file.is_empty()).map(|r| r.path()).collect();
            files.sort();
            files.dedup();
            AllowedThreat { threat_id: id, name, files }
        })
        .collect()
}
