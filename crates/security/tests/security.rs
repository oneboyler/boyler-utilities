//! Every row / action of DESIGN §3.14 against the fake Windows: read, scans, offline scan, Remove / Allow / Restore / Delete,
//! undo, the admin path, other antivirus, errors. Nothing here touches the real Defender.

use bu_security::fake::{detection, sample_status, threat};
use bu_security::*;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(20);
const KMS: &str = r"C:\Users\x\Downloads\kms_activator.exe";
const FREE: &str = r"C:\Users\x\Downloads\free_skins_unlocker.zip";

fn svc(os: &FakeOs) -> SecurityService {
    SecurityService::new(Arc::new(os.clone()))
}

fn stamp(d: u8, h: u8, m: u8) -> Stamp {
    Stamp::new(2026, 10, d, h, m)
}

/// A fake with one quarantined threat from 3 Oct (the drawing's example) and one waiting for a choice.
fn with_threats() -> FakeOs {
    let os = FakeOs::protected();
    os.add_threat(
        threat(10, "HackTool:Win32/AutoKMS", Severity::High, false),
        detection("d-quar", 10, 3, stamp(3, 14, 2), r"C:\Users\x\Downloads\kms_activator.exe"),
    );
    os.add_threat(
        threat(20, "Trojan:Win32/Wacatac.B!ml", Severity::Severe, true),
        detection("d-wait", 20, 1, stamp(8, 9, 30), r"C:\Users\x\Downloads\free_skins_unlocker.zip"),
    );
    os
}

// ------------------------------------------------------------------------------------------------ the page

#[test]
fn protected_page_has_the_drawings_lines() {
    let os = FakeOs::protected();
    let s = svc(&os);
    let p = s.page().unwrap();
    let now = s.now();
    assert_eq!(p.banner, Banner::Protected);
    assert_eq!(p.status.definitions_version, "1.421.733.0");
    let (t, k) = p.last_scan.clone().unwrap();
    assert_eq!(k, ScanKind::Quick);
    assert_eq!(t.label(&now), "Today 09:12"); // "Last scan: Today 09:12 · Quick scan"
    assert_eq!(p.status.definitions_updated.unwrap().label(&now), "Today 06:40");
    assert!(p.threats.is_empty() && p.quarantine.is_empty());
}

#[test]
fn last_scan_is_the_newer_of_quick_and_full() {
    let mut st = sample_status();
    st.full_scan_end = Some(stamp(8, 9, 50));
    assert_eq!(last_scan_of(&st).unwrap().1, ScanKind::Full);
    st.full_scan_end = Some(stamp(7, 9, 50));
    assert_eq!(last_scan_of(&st).unwrap().1, ScanKind::Quick);
    st.quick_scan_end = None;
    assert_eq!(last_scan_of(&st).unwrap().1, ScanKind::Full);
    st.full_scan_end = None;
    assert!(last_scan_of(&st).is_none());
}

#[test]
fn threats_and_quarantine_rows() {
    let os = with_threats();
    let s = svc(&os);
    let p = s.page().unwrap();
    let now = s.now();
    assert_eq!(p.banner, Banner::NeedsAttention { threats: 1 });
    assert_eq!(p.threats.len(), 1);
    let t = &p.threats[0];
    assert_eq!((t.name.as_str(), t.severity, t.state), ("Trojan:Win32/Wacatac.B!ml", Severity::Severe, ThreatState::NeedsChoice));
    assert_eq!((t.file.as_str(), t.folder_name()), ("free_skins_unlocker.zip", "Downloads"));
    assert_eq!(t.found.unwrap().short(&now), "09:30"); // "found HH:MM"
    assert!(t.severity.is_red());
    assert_eq!(p.quarantine.len(), 1);
    let q = &p.quarantine[0];
    assert_eq!((q.name.as_str(), q.file.as_str()), ("HackTool:Win32/AutoKMS", "kms_activator.exe"));
    assert_eq!(q.found.unwrap().short(&now), "3 Oct"); // "quarantined 3 Oct"
    assert_eq!(q.path(), r"C:\Users\x\Downloads\kms_activator.exe");
    assert!(p.quarantine_from_history);
}

#[test]
fn severity_tags_red_and_amber() {
    assert!(Severity::from_id(5).is_red() && Severity::from_id(4).is_red());
    assert!(!Severity::from_id(2).is_red() && !Severity::from_id(1).is_red());
    assert_eq!(Severity::from_id(2).label(), "Moderate");
    assert_eq!(Severity::from_id(0), Severity::Unknown);
}

#[test]
fn threat_status_ids_follow_microsofts_table() {
    use ThreatState::*;
    for (id, want) in [(1, NeedsChoice), (102, NeedsChoice), (103, NeedsChoice), (104, NeedsChoice), (105, NeedsChoice), (107, NeedsChoice), (2, Handled), (6, Handled), (3, Quarantined), (4, Removed), (5, Allowed), (0, Unknown), (77, Unknown)] {
        assert_eq!(ThreatState::from_status_id(id), want, "status {id}");
    }
}

#[test]
fn inactive_old_detections_do_not_ask_for_a_choice() {
    let os = FakeOs::protected();
    // "abandoned" (105) forever, but the threat is no longer active: not a row in Threats found
    os.add_threat(threat(5, "PUA:Win32/Old", Severity::Low, false), detection("d", 5, 105, stamp(1, 8, 0), r"C:\a\old.exe"));
    let p = svc(&os).page().unwrap();
    assert!(p.threats.is_empty());
    assert_eq!(p.banner, Banner::Protected);
}

#[test]
fn newest_detection_of_a_file_decides_the_quarantine_row() {
    let os = FakeOs::protected();
    os.add_threat(threat(7, "PUA:Win32/X", Severity::Low, false), detection("old", 7, 3, stamp(1, 8, 0), r"C:\a\x.exe"));
    os.state().detections.push(detection("new", 7, 5, stamp(2, 8, 0), r"C:\a\x.exe")); // allowed later
    assert!(svc(&os).page().unwrap().quarantine.is_empty());
}

#[test]
fn resource_paths_are_cleaned() {
    assert_eq!(resource_path(r"file:_C:\Users\x\a.exe").as_deref(), Some(r"C:\Users\x\a.exe"));
    assert_eq!(resource_path(r"webfile:_C:\x\a.exe|https://e.com/a.exe|pid:4").as_deref(), Some(r"C:\x\a.exe"));
    assert_eq!(resource_path(r"containerfile:_C:\x\a.zip->inner.exe").as_deref(), Some(r"C:\x\a.zip"));
    assert_eq!(resource_path(r"file:_\\server\share\a.exe").as_deref(), Some(r"\\server\share\a.exe"));
    assert_eq!(resource_path(r"regkey:_HKLM\SOFTWARE\x"), None);
    assert_eq!(resource_path("process:_pid:1234"), None);
    assert_eq!(resource_path("nonsense"), None);
}

#[test]
fn one_detection_with_three_files_makes_three_rows() {
    let d = Detection {
        detection_id: "d".into(),
        threat_id: 1,
        status_id: 3,
        found: Some(stamp(7, 18, 7)),
        status_changed: None,
        resources: vec![r"file:_C:\D\a (1).exe".into(), r"file:_C:\D\a (2).exe".into(), r"file:_C:\D\a.exe".into()],
    };
    let rows = rows_from(&[d], &[ThreatInfo { threat_id: 1, name: "PUA:Win32/GameHack".into(), severity: Severity::Low, active: false }]);
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|r| r.name == "PUA:Win32/GameHack" && r.folder == r"C:\D"));
}

#[test]
fn cim_datetimes() {
    assert_eq!(parse_cim_datetime("20261007163037.000000+000"), Some((2026, 10, 7, 16, 30, 37, 0)));
    assert_eq!(parse_cim_datetime("20261007183037.000000+120"), Some((2026, 10, 7, 18, 30, 37, 120)));
    assert_eq!(parse_cim_datetime("20261007083037.000000-480"), Some((2026, 10, 7, 8, 30, 37, -480)));
    assert_eq!(parse_cim_datetime("16010101000000.000000+000"), None); // never
    assert_eq!(parse_cim_datetime(""), None);
    assert_eq!(parse_cim_datetime("garbage-garbage-garbage!"), None);
    // offsets are undone, across midnight and month ends
    assert_eq!(cim_to_utc((2026, 10, 7, 18, 30, 37, 120)), (2026, 10, 7, 16, 30, 37));
    assert_eq!(cim_to_utc((2026, 10, 1, 0, 30, 0, 120)), (2026, 9, 30, 22, 30, 0));
    assert_eq!(cim_to_utc((2026, 12, 31, 23, 30, 0, -120)), (2027, 1, 1, 1, 30, 0));
    assert_eq!(cim_to_utc((2028, 3, 1, 0, 10, 0, 60)), (2028, 2, 29, 23, 10, 0)); // leap year
}

#[test]
fn stamp_labels() {
    let now = stamp(8, 10, 0);
    assert_eq!(stamp(8, 9, 12).label(&now), "Today 09:12");
    assert_eq!(stamp(3, 9, 12).label(&now), "3 Oct");
    assert_eq!(stamp(3, 9, 12).short(&now), "3 Oct");
    assert_eq!(stamp(8, 0, 5).short(&now), "00:05");
    assert_eq!(Stamp::new(2025, 10, 8, 9, 0).label(&now), "8 Oct"); // another year is not "Today"
}

#[test]
fn security_center_product_state() {
    // 397568 = 0x61100: on, definitions up to date (a real value from this kind of PC)
    let d = AvProduct::from_product_state("Windows Defender", 397_568);
    assert!(d.on && d.up_to_date && d.is_defender());
    let off = AvProduct::from_product_state("Other AV", 0x0000);
    assert!(!off.on);
    let old = AvProduct::from_product_state("Other AV", 0x1010);
    assert!(old.on && !old.up_to_date);
}

#[test]
fn running_modes() {
    assert_eq!(RunningMode::parse("Normal"), RunningMode::Normal);
    assert_eq!(RunningMode::parse("Passive Mode"), RunningMode::Passive);
    assert_eq!(RunningMode::parse("Not running"), RunningMode::NotRunning);
    assert_eq!(RunningMode::parse(""), RunningMode::NotRunning);
    assert_eq!(RunningMode::parse("EDR Block Mode"), RunningMode::Other("EDR Block Mode".into()));
}

// ------------------------------------------------------------------------------------------------ other antivirus / off

#[test]
fn another_antivirus_active_defender_stands_by() {
    let os = FakeOs::protected();
    {
        let mut s = os.state();
        s.status.as_mut().unwrap().running_mode = RunningMode::Passive;
        s.products = vec![
            AvProduct { name: "Windows Defender".into(), on: false, up_to_date: true },
            AvProduct { name: "Contoso Shield".into(), on: true, up_to_date: true },
        ];
    }
    let s = svc(&os);
    assert_eq!(s.page().unwrap().banner, Banner::OtherAntivirus { name: "Contoso Shield".into() });
    assert_eq!(s.can_start_scan(), Err(SecurityError::OtherAntivirus("Contoso Shield".into())));
    assert!(matches!(s.start_scan(ScanKind::Quick, |_| {}), Err(SecurityError::OtherAntivirus(_))));
    assert_eq!(s.offline_scan(true), Err(SecurityError::OtherAntivirus("Contoso Shield".into())));
    assert!(os.log().is_empty(), "nothing reached Windows");
}

#[test]
fn defender_provider_missing_but_other_av_on_still_shows_a_page() {
    let os = FakeOs::protected();
    {
        let mut s = os.state();
        s.status = None;
        s.products = vec![AvProduct { name: "Contoso Shield".into(), on: true, up_to_date: true }];
    }
    let p = svc(&os).page().unwrap();
    assert_eq!(p.banner, Banner::OtherAntivirus { name: "Contoso Shield".into() });
}

#[test]
fn defender_gone_and_no_other_av_is_an_error() {
    let os = FakeOs::protected();
    os.state().status = None;
    assert_eq!(svc(&os).page().unwrap_err(), SecurityError::DefenderNotRunning);
    assert_eq!(svc(&os).can_start_scan(), Err(SecurityError::DefenderNotRunning));
}

#[test]
fn realtime_off_is_its_own_banner_and_defender_is_never_switched() {
    let os = FakeOs::protected();
    os.state().status.as_mut().unwrap().realtime_enabled = false;
    assert_eq!(svc(&os).page().unwrap().banner, Banner::ProtectionOff);
    // the service has no call that changes protection: nothing in the log after reading
    assert!(os.log().is_empty());
}

// ------------------------------------------------------------------------------------------------ scans

#[test]
fn scan_lifecycle_quick_scan_clean() {
    let os = FakeOs::protected();
    let s = svc(&os);
    assert_eq!(s.scan_state(), ScanState::Idle);
    let (tx, rx) = mpsc::channel();
    s.start_scan(ScanKind::Quick, move |r| tx.send(r).unwrap()).unwrap();
    assert!(os.wait_scan_started(WAIT));
    assert!(matches!(s.scan_state(), ScanState::Running { kind: ScanKind::Quick, .. }));
    assert_eq!(s.page().unwrap().banner, Banner::Scanning { title: "Quick scan".into() });
    assert_eq!(s.can_start_scan(), Err(SecurityError::ScanRunning)); // the other tiles are disabled
    assert!(matches!(s.start_scan(ScanKind::Full, |_| {}), Err(SecurityError::ScanRunning)));
    os.state().now = stamp(8, 10, 5);
    os.release_scan();
    let report = rx.recv_timeout(WAIT).unwrap();
    assert!(report.is_clean());
    assert_eq!(report.kind.title(), "Quick scan");
    assert_eq!(report.finished, stamp(8, 10, 5));
    assert!(s.wait_idle(WAIT));
    assert_eq!(s.scan_state(), ScanState::Idle);
    assert_eq!(s.last_report().unwrap(), report);
    // "Last scan: Today 10:05 · Quick scan"
    assert_eq!(s.page().unwrap().last_scan.unwrap().0, stamp(8, 10, 5));
    assert_eq!(os.log(), vec!["scan Quick scan".to_string()]);
}

#[test]
fn full_scan_updates_the_full_scan_time() {
    let os = FakeOs::protected();
    let s = svc(&os);
    let (tx, rx) = mpsc::channel();
    s.start_scan(ScanKind::Full, move |r| tx.send(r).unwrap()).unwrap();
    assert!(os.wait_scan_started(WAIT));
    os.release_scan();
    assert!(rx.recv_timeout(WAIT).unwrap().is_clean());
    let (_, k) = s.page().unwrap().last_scan.unwrap();
    assert_eq!(k, ScanKind::Full);
}

#[test]
fn scan_that_finds_a_threat_reports_it_and_the_status_flips() {
    let os = FakeOs::protected();
    os.state().scan_adds.push((
        detection("new", 20, 1, stamp(8, 10, 2), r"C:\Users\x\Downloads\free_skins_unlocker.zip"),
        threat(20, "Trojan:Win32/Wacatac.B!ml", Severity::Severe, true),
    ));
    let s = svc(&os);
    let (tx, rx) = mpsc::channel();
    s.start_scan(ScanKind::Quick, move |r| tx.send(r).unwrap()).unwrap();
    assert!(os.wait_scan_started(WAIT));
    os.release_scan();
    let report = rx.recv_timeout(WAIT).unwrap();
    assert!(!report.is_clean());
    assert_eq!(report.new_threats.len(), 1);
    assert_eq!(report.new_threats[0].name, "Trojan:Win32/Wacatac.B!ml"); // "Threat found: <name>"
    assert!(s.wait_idle(WAIT));
    assert_eq!(s.page().unwrap().banner, Banner::NeedsAttention { threats: 1 });
}

#[test]
fn old_detections_are_not_reported_as_new() {
    let os = with_threats();
    let s = svc(&os);
    let (tx, rx) = mpsc::channel();
    s.start_scan(ScanKind::Quick, move |r| tx.send(r).unwrap()).unwrap();
    assert!(os.wait_scan_started(WAIT));
    os.release_scan();
    assert!(rx.recv_timeout(WAIT).unwrap().new_threats.is_empty());
}

#[test]
fn cancel_stops_the_scan_and_the_report_says_cancelled() {
    let os = FakeOs::protected();
    let s = svc(&os);
    let (tx, rx) = mpsc::channel();
    assert_eq!(s.cancel_scan(), Err(SecurityError::NoScanRunning));
    s.start_scan(ScanKind::Full, move |r| tx.send(r).unwrap()).unwrap();
    assert!(os.wait_scan_started(WAIT));
    s.cancel_scan().unwrap();
    let report = rx.recv_timeout(WAIT).unwrap();
    assert_eq!(report.outcome, ScanOutcome::Cancelled);
    assert!(!report.is_clean(), "a cancelled scan is not 'no threats found'");
    assert!(s.wait_idle(WAIT));
    // the last-scan time did not move
    assert_eq!(s.page().unwrap().last_scan.unwrap().1, ScanKind::Quick);
    // a new scan can start afterwards
    let (tx2, rx2) = mpsc::channel();
    s.start_scan(ScanKind::Quick, move |r| tx2.send(r).unwrap()).unwrap();
    s.cancel_scan().unwrap();
    assert_eq!(rx2.recv_timeout(WAIT).unwrap().outcome, ScanOutcome::Cancelled);
}

#[test]
fn a_failing_scan_is_reported_not_swallowed() {
    let os = FakeOs::protected();
    os.state().scan_error = Some(SecurityError::NeedsAdmin);
    let s = svc(&os);
    let (tx, rx) = mpsc::channel();
    s.start_scan(ScanKind::Quick, move |r| tx.send(r).unwrap()).unwrap();
    assert!(os.wait_scan_started(WAIT));
    os.release_scan();
    let r = rx.recv_timeout(WAIT).unwrap();
    assert_eq!(r.outcome, ScanOutcome::Failed(SecurityError::NeedsAdmin));
    assert!(!r.is_clean());
    assert!(s.wait_idle(WAIT));
}

#[test]
fn drop_zone_scan_of_a_file_and_a_folder() {
    let os = FakeOs::protected();
    os.state().existing.insert(r"C:\Users\x\Downloads\OBS.exe".into());
    os.state().existing.insert(r"C:\Users\x\Downloads".into());
    let s = svc(&os);
    assert_eq!(
        s.start_scan(ScanKind::Path(r"C:\nope\missing.exe".into()), |_| {}),
        Err(SecurityError::PathMissing(r"C:\nope\missing.exe".into()))
    );
    assert!(os.log().is_empty(), "a missing path never starts a scan");
    for p in [r"C:\Users\x\Downloads\OBS.exe", r"C:\Users\x\Downloads"] {
        let (tx, rx) = mpsc::channel();
        let before = os.state().scans_started;
        s.start_scan(ScanKind::Path(p.into()), move |r| tx.send(r).unwrap()).unwrap();
        while os.state().scans_started == before {
            assert!(os.wait_scan_started(WAIT));
            std::thread::yield_now();
        }
        os.release_scan();
        let r = rx.recv_timeout(WAIT).unwrap();
        assert!(r.is_clean(), "{p}");
        assert!(s.wait_idle(WAIT));
    }
    assert_eq!(ScanKind::Path(r"C:\Users\x\Downloads\OBS.exe".into()).title(), "OBS.exe");
}

#[test]
fn reading_the_page_starts_nothing() {
    let os = with_threats();
    let s = svc(&os);
    for _ in 0..3 {
        s.page().unwrap();
    }
    assert_eq!(os.state().scans_started, 0);
    assert!(os.log().is_empty());
    assert_eq!(s.scan_state(), ScanState::Idle);
}

#[test]
fn scans_need_no_admin_check_up_front() {
    // admin need of scans / updates is "unknown" (Microsoft does not say): they are tried, not blocked
    let os = FakeOs::protected();
    os.set_elevated(false);
    let s = svc(&os);
    assert_eq!(s.needs_admin(Action::QuickScan), AdminNeed::Unknown);
    let (tx, rx) = mpsc::channel();
    s.start_scan(ScanKind::Quick, move |r| tx.send(r).unwrap()).unwrap();
    assert!(os.wait_scan_started(WAIT));
    os.release_scan();
    assert!(rx.recv_timeout(WAIT).unwrap().is_clean());
}

#[test]
fn update_definitions() {
    let os = FakeOs::protected();
    os.state().now = stamp(8, 11, 0);
    let s = svc(&os);
    s.update_definitions().unwrap();
    assert_eq!(s.page().unwrap().status.definitions_updated, Some(stamp(8, 11, 0)));
    assert_eq!(os.log(), vec!["update definitions".to_string()]);
    os.state().fail.push(("update", SecurityError::NeedsAdmin));
    assert_eq!(s.update_definitions(), Err(SecurityError::NeedsAdmin));
}

// ------------------------------------------------------------------------------------------------ offline scan

#[test]
fn offline_scan_asks_first_then_needs_admin() {
    let os = FakeOs::protected();
    os.set_elevated(false);
    let s = svc(&os);
    assert_eq!(s.needs_admin(Action::OfflineScan), AdminNeed::Yes);
    assert_eq!(s.offline_scan(false), Err(SecurityError::RestartNotConfirmed));
    assert_eq!(s.offline_scan(true), Err(SecurityError::NeedsAdmin));
    assert!(os.log().is_empty(), "the PC was never asked to restart");
    os.set_elevated(true);
    assert_eq!(s.offline_scan(false), Err(SecurityError::RestartNotConfirmed));
    assert!(os.log().is_empty());
    s.offline_scan(true).unwrap();
    assert_eq!(os.log(), vec!["offline scan (restart)".to_string()]);
}

#[test]
fn offline_scan_not_while_a_scan_runs() {
    let os = FakeOs::protected();
    let s = svc(&os);
    s.start_scan(ScanKind::Quick, |_| {}).unwrap();
    assert!(os.wait_scan_started(WAIT));
    assert_eq!(s.offline_scan(true), Err(SecurityError::ScanRunning));
    s.cancel_scan().unwrap();
    assert!(s.wait_idle(WAIT));
}

// ------------------------------------------------------------------------------------------------ Remove / Allow / Restore / Delete

#[test]
fn remove_moves_the_threat_to_quarantine_and_undo_restores_it() {
    let os = with_threats();
    let s = svc(&os);
    let c = s.remove_threat(20).unwrap();
    assert_eq!(c.toast(), "free_skins_unlocker.zip removed · in Quarantine now");
    let p = s.page().unwrap();
    assert!(p.threats.is_empty());
    assert_eq!(p.banner, Banner::Protected);
    assert_eq!(p.quarantine.len(), 2);
    assert!(p.quarantine.iter().any(|r| r.name == "Trojan:Win32/Wacatac.B!ml"));
    // undo = restore what the remove quarantined
    s.undo(c.id).unwrap();
    assert!(os.log().contains(&r"restore C:\Users\x\Downloads\free_skins_unlocker.zip".to_string()));
    assert_eq!(s.undo(c.id), Err(SecurityError::NothingToUndo));
}

#[test]
fn remove_unknown_threat() {
    let os = with_threats();
    assert_eq!(svc(&os).remove_threat(999), Err(SecurityError::NoSuchThreat(999)));
    assert!(os.log().is_empty());
    // a quarantined one is not "waiting for a choice"
    assert_eq!(svc(&os).remove_threat(10), Err(SecurityError::NoSuchThreat(10)));
}

#[test]
fn allow_and_undo() {
    let os = with_threats();
    let s = svc(&os);
    let c = s.allow_threat(20).unwrap();
    assert_eq!(c.toast(), "free_skins_unlocker.zip allowed");
    let p = s.page().unwrap();
    assert!(p.threats.is_empty());
    assert_eq!(p.banner, Banner::Protected);
    s.undo(c.id).unwrap();
    assert_eq!(s.page().unwrap().threats.len(), 1, "the threat is back waiting for a choice");
    assert_eq!(os.log(), vec!["allow 20".to_string(), "disallow 20".to_string()]);
}

#[test]
fn restore_from_quarantine() {
    let os = with_threats();
    let s = svc(&os);
    let toast = s.restore_quarantined(10, KMS).unwrap();
    assert_eq!(toast, "kms_activator.exe restored to Downloads");
    assert!(s.page().unwrap().quarantine.is_empty());
    assert_eq!(os.log(), vec!["allow 10".to_string(), format!("restore {KMS}")]);
    assert_eq!(s.allowed_in_defender().unwrap().iter().map(|a| a.threat_id).collect::<Vec<_>>(), vec![10]);
    // gone from the list: nothing to restore any more
    assert_eq!(s.restore_quarantined(10, KMS), Err(SecurityError::NoSuchThreat(10)));
    // a threat that is not in quarantine cannot be "restored"
    assert_eq!(s.restore_quarantined(20, FREE), Err(SecurityError::NoSuchThreat(20)));
}

#[test]
fn delete_from_quarantine_is_not_available_and_says_why() {
    let os = with_threats();
    let s = svc(&os);
    let err = s.delete_quarantined(10).unwrap_err();
    assert!(matches!(err, SecurityError::Unsupported(ref why) if why.contains("quarantine")), "{err}");
    assert_eq!(s.page().unwrap().quarantine.len(), 1);
    assert!(os.log().is_empty());
}

#[test]
fn the_four_threat_actions_need_admin_and_stop_before_windows() {
    let os = with_threats();
    os.set_elevated(false);
    let s = svc(&os);
    for a in [Action::RemoveThreat, Action::AllowThreat, Action::RestoreQuarantined, Action::OfflineScan] {
        assert_eq!(s.needs_admin(a), AdminNeed::Yes, "{a:?}");
    }
    assert_eq!(s.remove_threat(20), Err(SecurityError::NeedsAdmin));
    assert_eq!(s.allow_threat(20), Err(SecurityError::NeedsAdmin));
    assert_eq!(s.restore_quarantined(10, KMS), Err(SecurityError::NeedsAdmin));
    assert!(os.log().is_empty(), "refused calls never reached the OS");
    let p = s.page().unwrap();
    assert_eq!((p.threats.len(), p.quarantine.len()), (1, 1), "nothing changed");
}

#[test]
fn windows_saying_access_denied_becomes_needs_admin_and_nothing_is_recorded() {
    let os = with_threats();
    let s = svc(&os);
    os.state().fail.push(("remove", SecurityError::NeedsAdmin));
    assert_eq!(s.remove_threat(20), Err(SecurityError::NeedsAdmin));
    // the failed remove left no change behind to undo
    assert_eq!(s.undo(1), Err(SecurityError::NothingToUndo));
    os.state().fail.push(("restore", SecurityError::Os { call: "MpCmdRun -Restore".into(), code: 0x8050_8014, text: "restore failed".into() }));
    assert!(matches!(s.restore_quarantined(10, KMS), Err(SecurityError::Os { code: 0x8050_8014, .. })));
    assert_eq!(s.page().unwrap().quarantine.len(), 1);
}

#[test]
fn undo_that_needs_admin_stays_available() {
    let os = with_threats();
    let s = svc(&os);
    let c = s.allow_threat(20).unwrap();
    os.set_elevated(false);
    assert_eq!(s.undo(c.id), Err(SecurityError::NeedsAdmin));
    os.set_elevated(true);
    s.undo(c.id).unwrap();
}

#[test]
fn nothing_to_undo() {
    let os = FakeOs::protected();
    assert_eq!(svc(&os).undo(42), Err(SecurityError::NothingToUndo));
}

#[test]
fn admin_gate_comes_before_anything_else_but_scans_are_not_gated() {
    assert_eq!(Action::QuickScan.admin_need(), AdminNeed::Unknown);
    assert_eq!(Action::FullScan.admin_need(), AdminNeed::Unknown);
    assert_eq!(Action::ScanPath.admin_need(), AdminNeed::Unknown);
    assert_eq!(Action::UpdateDefinitions.admin_need(), AdminNeed::Unknown);
    assert_eq!(Action::DeleteQuarantined.admin_need(), AdminNeed::Yes);
}

// ------------------------------------------------------------------------------------------------ review 016 item 1 fixes

/// Defender handles High / Severe threats on its own: the scan still FOUND something, so it is never "no threats found".
#[test]
fn a_scan_that_defender_already_dealt_with_is_not_clean() {
    for (status, state) in [(3, ThreatState::Quarantined), (2, ThreatState::Handled), (4, ThreatState::Removed), (6, ThreatState::Handled)] {
        let os = FakeOs::protected();
        os.state().scan_adds.push((
            detection("auto", 30, status, stamp(8, 10, 2), r"C:\Users\x\Downloads\crack.exe"),
            threat(30, "Trojan:Win32/Wacatac.B!ml", Severity::Severe, false),
        ));
        let s = svc(&os);
        let (tx, rx) = mpsc::channel();
        s.start_scan(ScanKind::Quick, move |r| tx.send(r).unwrap()).unwrap();
        assert!(os.wait_scan_started(WAIT));
        os.release_scan();
        let report = rx.recv_timeout(WAIT).unwrap();
        assert!(!report.is_clean(), "status {status}");
        assert_eq!(report.new_threats.len(), 1);
        assert_eq!(report.new_threats[0].state, state);
        assert!(report.waiting().is_empty(), "nothing waits for a choice");
        assert_eq!(report.handled_by_defender().len(), 1);
        assert!(s.wait_idle(WAIT));
    }
}

#[test]
fn a_clean_scan_has_no_found_rows_at_all() {
    let os = FakeOs::protected();
    let s = svc(&os);
    let (tx, rx) = mpsc::channel();
    s.start_scan(ScanKind::Quick, move |r| tx.send(r).unwrap()).unwrap();
    assert!(os.wait_scan_started(WAIT));
    os.release_scan();
    let r = rx.recv_timeout(WAIT).unwrap();
    assert!(r.is_clean() && r.new_threats.is_empty() && r.waiting().is_empty());
}

/// One detection that holds three files (the three ReShade installers on a real PC): restoring the 2nd must restore exactly it.
fn three_files_one_detection() -> FakeOs {
    let os = FakeOs::protected();
    let mut d = detection("d3", 40, 3, stamp(7, 16, 0), r"C:\Users\x\Downloads\Addon (1).exe");
    d.resources.push(r"file:_C:\Users\x\Downloads\Addon (2).exe".into());
    d.resources.push(r"file:_C:\Users\x\Downloads\Addon.exe".into());
    os.add_threat(threat(40, "PUA:Win32/GameHack", Severity::Low, false), d);
    os
}

#[test]
fn restore_the_second_of_three_files_of_one_detection() {
    let os = three_files_one_detection();
    let s = svc(&os);
    let p2 = r"C:\Users\x\Downloads\Addon (2).exe";
    assert_eq!(s.page().unwrap().quarantine.len(), 3);
    let toast = s.restore_quarantined(40, p2).unwrap();
    assert_eq!(toast, "Addon (2).exe restored to Downloads");
    assert_eq!(os.log(), vec!["allow 40".to_string(), format!("restore {p2}")], "exactly that path, the Allow first");
    let left: Vec<String> = s.page().unwrap().quarantine.iter().map(|r| r.file.clone()).collect();
    assert_eq!(left.len(), 2, "the other two are still in quarantine and stay listed: {left:?}");
    assert!(!left.contains(&"Addon (2).exe".to_string()));
}

#[test]
fn restore_the_second_of_two_detections_with_the_same_threat_id() {
    let os = FakeOs::protected();
    os.add_threat(threat(10, "HackTool:Win32/AutoKMS", Severity::High, false), detection("a", 10, 3, stamp(3, 14, 2), KMS));
    os.state().detections.push(detection("b", 10, 3, stamp(3, 14, 5), r"C:\Users\x\Documents\other.exe"));
    let s = svc(&os);
    assert_eq!(s.page().unwrap().quarantine.len(), 2);
    // the newest row is other.exe; restoring kms_activator.exe must not touch it
    s.restore_quarantined(10, KMS).unwrap();
    assert_eq!(os.log(), vec!["allow 10".to_string(), format!("restore {KMS}")]);
    let q = s.page().unwrap().quarantine;
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].file, "other.exe", "its copy is still in quarantine, although its threat id is allowed now");
    // a path that is not in the list is refused, nothing reaches the OS
    assert_eq!(s.restore_quarantined(10, r"C:\Users\x\nothing.exe"), Err(SecurityError::NoSuchThreat(10)));
    assert_eq!(os.log().len(), 2);
}

#[test]
fn allowing_a_threat_does_not_hide_its_older_quarantined_copies() {
    let os = with_threats();
    os.state().detections.push(detection("old", 20, 3, stamp(2, 9, 0), r"C:\Users\x\Downloads\older_copy.zip"));
    let s = svc(&os);
    s.allow_threat(20).unwrap();
    let p = s.page().unwrap();
    assert!(p.threats.is_empty());
    assert!(p.quarantine.iter().any(|r| r.file == "older_copy.zip"), "the older copy is still in quarantine");
}

#[test]
fn restore_whose_allow_fails_restores_nothing() {
    let os = with_threats();
    let s = svc(&os);
    os.state().fail.push(("allow", SecurityError::NeedsAdmin));
    assert_eq!(s.restore_quarantined(10, KMS), Err(SecurityError::NeedsAdmin));
    assert!(os.log().is_empty(), "no Allow, so no restore either");
}

#[test]
fn restore_that_fails_takes_its_allow_back() {
    let os = with_threats();
    let s = svc(&os);
    os.state().fail.push(("restore", SecurityError::Os { call: "MpCmdRun -Restore".into(), code: 0x8050_8014, text: "restore failed".into() }));
    assert!(matches!(s.restore_quarantined(10, KMS), Err(SecurityError::Os { .. })));
    assert_eq!(os.log(), vec!["allow 10".to_string(), "disallow 10".to_string()]);
    assert!(os.state().allowed.is_empty());
    assert_eq!(s.page().unwrap().quarantine.len(), 1, "still in quarantine");
    // an Allow that was already there before is left alone
    os.state().allowed = vec![10];
    os.state().fail.push(("restore", SecurityError::NeedsAdmin));
    assert!(s.restore_quarantined(10, KMS).is_err());
    assert_eq!(os.state().allowed, vec![10]);
}

/// A detection list that cannot be read is never "no threats found".
#[test]
fn a_scan_whose_detections_cannot_be_read_is_failed_not_clean() {
    let os = FakeOs::protected();
    let s = svc(&os);
    os.state().fail.push(("read detections", SecurityError::NeedsAdmin));
    let (tx, rx) = mpsc::channel();
    s.start_scan(ScanKind::Quick, move |r| tx.send(r).unwrap()).unwrap();
    let r = rx.recv_timeout(WAIT).unwrap();
    assert_eq!(r.outcome, ScanOutcome::Failed(SecurityError::NeedsAdmin));
    assert!(!r.is_clean());
    assert!(s.wait_idle(WAIT));
    // read fails only AFTER the scan
    let os2 = FakeOs::protected();
    let s2 = svc(&os2);
    let (tx2, rx2) = mpsc::channel();
    s2.start_scan(ScanKind::Quick, move |r| tx2.send(r).unwrap()).unwrap();
    assert!(os2.wait_scan_started(WAIT));
    os2.state().fail.push(("read detections", SecurityError::NeedsAdmin));
    os2.release_scan();
    let r2 = rx2.recv_timeout(WAIT).unwrap();
    assert_eq!(r2.outcome, ScanOutcome::Failed(SecurityError::NeedsAdmin));
    assert!(!r2.is_clean());
}

#[test]
fn allowed_threats_leave_threats_found() {
    let os = with_threats();
    let s = svc(&os);
    let c = s.allow_threat(20).unwrap();
    let p = s.page().unwrap();
    assert!(p.threats.is_empty(), "an allowed threat is not waiting for a choice");
    assert_eq!(p.allowed.len(), 1);
    assert_eq!(p.allowed[0].name, "Trojan:Win32/Wacatac.B!ml");
    assert_eq!(p.allowed[0].files, vec![r"C:\Users\x\Downloads\free_skins_unlocker.zip".to_string()]);
    s.undo(c.id).unwrap();
    assert_eq!(s.page().unwrap().threats.len(), 1);
    assert!(s.page().unwrap().allowed.is_empty());
}

#[test]
fn the_reset_line_reads_the_allow_list_and_clears_it() {
    let os = with_threats();
    let s = svc(&os);
    assert_eq!(s.allowed_in_defender().unwrap(), vec![]);
    assert_eq!(s.reset_allowed(), Ok(0));
    // an Allow made in an earlier run (the change ids are gone): Defender's own list is the truth
    os.state().allowed = vec![20, 4242];
    let list = s.allowed_in_defender().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].name, "Trojan:Win32/Wacatac.B!ml");
    assert_eq!(list[1].name, "Threat 4242", "a threat Defender no longer lists");
    assert_eq!(s.reset_allowed(), Ok(2));
    assert!(s.allowed_in_defender().unwrap().is_empty());
    assert_eq!(os.log(), vec!["disallow 20".to_string(), "disallow 4242".to_string()]);
}

#[test]
fn remove_allow_one_checks_the_list_and_needs_admin() {
    let os = with_threats();
    os.state().allowed = vec![20];
    os.set_elevated(false);
    let s = svc(&os);
    assert_eq!(s.needs_admin(Action::RemoveAllow), AdminNeed::Yes);
    assert_eq!(s.remove_allow(20), Err(SecurityError::NeedsAdmin));
    assert_eq!(s.reset_allowed(), Err(SecurityError::NeedsAdmin));
    assert!(os.log().is_empty());
    os.set_elevated(true);
    assert_eq!(s.remove_allow(99), Err(SecurityError::NoSuchThreat(99)));
    s.remove_allow(20).unwrap();
    assert!(os.state().allowed.is_empty());
}

#[test]
fn reset_allowed_stops_at_the_first_failure_and_the_rest_stays() {
    let os = FakeOs::protected();
    os.state().allowed = vec![1, 2, 3];
    let s = svc(&os);
    os.state().fail.push(("allow", SecurityError::Os { call: "Remove-MpPreference".into(), code: 1, text: "boom".into() }));
    assert!(matches!(s.reset_allowed(), Err(SecurityError::Os { .. })));
    assert_eq!(os.state().allowed, vec![1, 2, 3]);
    assert_eq!(s.reset_allowed(), Ok(3));
    assert!(os.state().allowed.is_empty());
}

#[test]
fn an_unreadable_allow_list_is_an_error_for_the_reset_but_not_for_the_page() {
    let os = with_threats();
    let s = svc(&os);
    os.state().fail.push(("read allowed", SecurityError::NeedsAdmin));
    assert_eq!(s.allowed_in_defender(), Err(SecurityError::NeedsAdmin));
    os.state().fail.push(("read allowed", SecurityError::NeedsAdmin));
    assert!(s.page().unwrap().allowed.is_empty(), "the page still shows");
}

#[test]
fn undo_of_a_remove_failing_halfway_restores_each_file_once() {
    let os = FakeOs::protected();
    for (i, f) in ["a.exe", "b.exe"].iter().enumerate() {
        os.add_threat(threat(50 + i as i64, "Trojan:Win32/X", Severity::High, true), detection(&format!("w{i}"), 50 + i as i64, 1, stamp(8, 9, i as u8), &format!(r"C:\x\{f}")));
    }
    let s = svc(&os);
    let c = s.remove_threat(50).unwrap();
    os.state().restore_fails_for = Some(r"C:\x\b.exe".to_string());
    assert!(matches!(s.undo(c.id), Err(SecurityError::Os { .. })));
    // a.exe is back; the change stays and now holds only b.exe
    assert_eq!(s.undo(c.id).map(|c| c.id), Ok(c.id));
    let restores: Vec<String> = os.log().into_iter().filter(|l| l.starts_with("restore")).collect();
    assert_eq!(
        restores,
        vec![r"restore C:\x\a.exe".to_string(), r"restore C:\x\b.exe".to_string()],
        "a.exe only once (the failed try of b.exe is not logged)"
    );
}

#[test]
fn delete_on_a_quarantine_row_opens_windows_security_history() {
    let os = with_threats();
    os.set_elevated(false); // no admin needed
    let s = svc(&os);
    s.open_protection_history().unwrap();
    assert_eq!(os.log(), vec!["open protection history".to_string()]);
    // the crate itself still deletes nothing
    assert!(matches!(s.delete_quarantined(10), Err(SecurityError::Unsupported(_))));
    assert_eq!(s.page().unwrap().quarantine.len(), 1);
}

/// A detection list that cannot be read is never "protected / nothing found" on the page (REVIEW 64bdece).
#[test]
fn an_unreadable_detection_list_is_never_protected_and_actions_return_the_read_error() {
    let os = with_threats();
    let s = svc(&os);
    os.state().fail.push(("read detections", SecurityError::NeedsAdmin));
    let p = s.page().unwrap();
    assert_eq!(p.banner, Banner::CannotRead);
    assert_eq!(p.unreadable, Some(SecurityError::NeedsAdmin));
    assert!(p.threats.is_empty() && p.quarantine.is_empty(), "empty because unknown, not because clean");
    // the actions say why, not "no such threat"
    os.state().fail.push(("read detections", SecurityError::NeedsAdmin));
    assert_eq!(s.remove_threat(20), Err(SecurityError::NeedsAdmin));
    os.state().fail.push(("read detections", SecurityError::NeedsAdmin));
    assert_eq!(s.allow_threat(20), Err(SecurityError::NeedsAdmin));
    os.state().fail.push(("read detections", SecurityError::NeedsAdmin));
    assert_eq!(s.restore_quarantined(10, KMS), Err(SecurityError::NeedsAdmin));
    assert!(os.log().is_empty(), "nothing reached the OS");
    // readable again: the normal page
    let p = s.page().unwrap();
    assert_eq!(p.unreadable, None);
    assert_eq!(p.banner, Banner::NeedsAttention { threats: 1 });
}

#[test]
fn an_unreadable_list_while_scanning_still_says_scanning_and_other_banners_win() {
    let os = FakeOs::protected();
    let s = svc(&os);
    s.start_scan(ScanKind::Quick, |_| {}).unwrap();
    assert!(os.wait_scan_started(WAIT));
    os.state().fail.push(("read detections", SecurityError::NeedsAdmin));
    let p = s.page().unwrap();
    assert!(matches!(p.banner, Banner::Scanning { .. }));
    assert_eq!(p.unreadable, Some(SecurityError::NeedsAdmin));
    s.cancel_scan().unwrap();
    assert!(s.wait_idle(WAIT));
}

#[test]
fn a_failed_restore_after_an_unreadable_allow_list_does_not_take_a_maybe_old_allow_away() {
    let os = with_threats();
    let s = svc(&os);
    os.state().allowed = vec![10];
    os.state().fail.push(("read allowed", SecurityError::NeedsAdmin)); // consumed by page()
    os.state().fail.push(("read allowed", SecurityError::NeedsAdmin)); // consumed by the restore's own read
    os.state().fail.push(("restore", SecurityError::NeedsAdmin));
    assert!(s.restore_quarantined(10, KMS).is_err());
    assert_eq!(os.state().allowed, vec![10], "no rollback when it is unknown whether the Allow was the user's own");
    assert!(!os.log().contains(&"disallow 10".to_string()));
}

/// Order 036: the reset puts ONE Allow back by threat id (admin); an Allow already there is left alone.
#[test]
fn add_allow_puts_one_allow_back_and_needs_admin() {
    let os = with_threats();
    os.set_elevated(false);
    let s = svc(&os);
    assert_eq!(s.add_allow(20), Err(SecurityError::NeedsAdmin));
    assert!(os.log().is_empty());
    os.set_elevated(true);
    s.add_allow(20).unwrap();
    assert_eq!(os.state().allowed, vec![20]);
    let n = os.log().len();
    s.add_allow(20).unwrap();
    assert_eq!(os.log().len(), n, "already allowed: nothing sent");
}
