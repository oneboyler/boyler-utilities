//! Every Quick fix against the FAKE Windows: no key chord is sent, no DISM / sfc / pnputil runs, Explorer is never
//! stopped, no file is deleted, no restore point is made. Time is the fake's clock (no wall-clock bounds).

use bu_quickfix::fake::{stamp, utc, FakeFixOs, Script};
use bu_quickfix::repair::{self, DismResult, Phase, Progress, RepairOutcome, RepairRun, SfcResult};
use bu_quickfix::restore::{self, RestoreOutcome};
use bu_quickfix::{cache, gfx, DisplayAdapter, Fix, FixError, FixOs, LocalTime};
use std::sync::{Arc, Mutex};

// ======================================================================= the card

#[test]
fn admin_shields_match_the_design() {
    let adm: Vec<Fix> = Fix::ALL.into_iter().filter(|f| f.needs_admin()).collect();
    assert_eq!(adm, vec![Fix::RepairWindowsFiles, Fix::MakeRestorePoint]);
}

// ======================================================================= 1 reset graphics driver

#[test]
fn chord_is_sent_only_while_our_menu_is_in_front() {
    let os = FakeFixOs::new();
    gfx::reset_with_chord(&os).unwrap();
    assert_eq!(os.chords_sent(), 1);
    os.set_foreground_ours(false); // a game / another app in front
    assert!(matches!(gfx::reset_with_chord(&os), Err(FixError::Refused(_))));
    assert_eq!(os.chords_sent(), 1, "never sent into another app");
}

#[test]
fn chord_on_a_read_only_layer_is_refused() {
    let os = FakeFixOs::new().read_only();
    assert!(matches!(gfx::reset_with_chord(&os), Err(FixError::Refused(_))));
    assert_eq!(os.chords_sent(), 0);
}

#[test]
fn adapter_restart_needs_admin() {
    let os = FakeFixOs::new();
    assert!(matches!(gfx::restart_adapters(&os), Err(FixError::NeedsAdmin(_))));
    assert!(os.spawned().is_empty());
}

#[test]
fn adapter_restart_runs_pnputil_per_adapter_and_reads_its_answer() {
    let os = FakeFixOs::new().elevated();
    os.set_adapters(vec![
        DisplayAdapter { name: "NVIDIA GeForce RTX 4090".into(), instance_id: r"PCI\VEN_10DE&DEV_2684\A".into() },
        DisplayAdapter { name: "AMD Radeon(TM) Graphics".into(), instance_id: r"PCI\VEN_1002&DEV_164E\B".into() },
    ]);
    os.script("pnputil.exe", Script::new(vec![b"Microsoft PnP Utility\r\n\r\nRestarting device:   PCI\\VEN_10DE\r\nDevice restarted successfully.\r\n".to_vec()], 0));
    os.script("pnputil.exe", Script::new(vec![b"Restarting device: B\r\nSystem reboot is needed to complete the restart.\r\n".to_vec()], 3010));
    let r = gfx::restart_adapters(&os).unwrap();
    assert_eq!(os.spawned(), vec![
        ("pnputil.exe".to_string(), vec!["/restart-device".to_string(), r"PCI\VEN_10DE&DEV_2684\A".to_string()]),
        ("pnputil.exe".to_string(), vec!["/restart-device".to_string(), r"PCI\VEN_1002&DEV_164E\B".to_string()]),
    ]);
    assert!(r[0].ok() && !r[0].needs_reboot());
    assert_eq!(r[0].message, "Device restarted successfully.");
    assert!(r[1].ok() && r[1].needs_reboot());
}

#[test]
fn adapter_restart_without_adapters_is_unavailable() {
    let os = FakeFixOs::new().elevated();
    os.set_adapters(vec![]);
    assert!(matches!(gfx::restart_adapters(&os), Err(FixError::Unavailable(_))));
}

// ======================================================================= 2 repair Windows files — parsing

#[test]
fn utf16_output_without_bom_is_decoded_line_by_line_even_split_mid_character() {
    let raw = bu_quickfix::fake::utf16("Beginning system scan.  This process will take some time.\r\n\r\nVerification 34% complete.\rVerification 35% complete.\r");
    let mut d = repair::LineDecoder::default();
    let mut lines = Vec::new();
    for chunk in raw.chunks(7) {
        lines.extend(d.push(chunk)); // odd chunk size splits UTF-16 units
    }
    lines.extend(d.finish());
    assert_eq!(lines, vec![
        "Beginning system scan.  This process will take some time.",
        "Verification 34% complete.",
        "Verification 35% complete.",
    ]);
}

#[test]
fn utf16_with_bom_and_plain_bytes_both_work() {
    let mut bom = vec![0xFF, 0xFE];
    bom.extend(bu_quickfix::fake::utf16("Line one\r\nLine two"));
    assert_eq!(repair::decode_all(&bom), "Line one\nLine two");
    assert_eq!(repair::decode_all(b"[==   18.3%   ]\r[====  24.0%  ]\r\nThe operation completed successfully.\r\n"),
        "[==   18.3%   ]\n[====  24.0%  ]\nThe operation completed successfully.");
    // a UTF-8 character split across two chunks
    let mut d = repair::LineDecoder::default();
    let s = "Fehlerfrei · fertig\n".as_bytes();
    let mut out = d.push(&s[..12]);
    out.extend(d.push(&s[12..]));
    assert_eq!(out, vec!["Fehlerfrei · fertig"]);
}

#[test]
fn percentages_are_read_from_both_programs() {
    assert_eq!(repair::percent("Verification 34% complete."), Some(34.0));
    assert_eq!(repair::percent("[=========                  18.3%                          ]"), Some(18.3));
    assert_eq!(repair::percent("[==========================100.0%==========================]"), Some(100.0));
    assert_eq!(repair::percent("Provjera 71 % dovršena."), Some(71.0));
    assert_eq!(repair::percent("[===   18,3%   ]"), Some(18.3), "decimal comma");
    assert_eq!(repair::percent("The operation completed successfully."), None);
    assert_eq!(repair::percent("150% nonsense"), None);
}

#[test]
fn progress_text_is_the_designs() {
    assert_eq!(Progress { phase: Phase::Dism, percent: Some(34.9) }.text(), "DISM · checking the Windows image · 34 %");
    assert_eq!(Progress { phase: Phase::Sfc, percent: Some(71.0) }.text(), "sfc · checking system files · 71 %");
    assert_eq!(Progress { phase: Phase::Sfc, percent: None }.text(), "sfc · checking system files");
}

#[test]
fn dism_and_sfc_results_are_classified() {
    assert_eq!(repair::classify_dism(0, "The restore operation completed successfully.\nThe operation completed successfully."), DismResult::Healthy);
    assert_eq!(repair::classify_dism(0, "The restore operation completed successfully.\nThe component store corruption was repaired.\nThe operation completed successfully."), DismResult::Repaired);
    assert_eq!(repair::classify_dism(740, "Error: 740\n\nElevated permissions are required to run DISM."), DismResult::NotAdmin);
    assert_eq!(
        repair::classify_dism(0x800F_081F, "Error: 0x800f081f\n\nThe source files could not be found."),
        DismResult::Failed { exit_code: 0x800F_081F, line: "Error: 0x800f081f".into() }
    );
    let wrp = "Windows Resource Protection";
    assert_eq!(repair::classify_sfc(&format!("{wrp} did not find any integrity violations.")), SfcResult::NoViolations);
    assert_eq!(repair::classify_sfc(&format!("{wrp} found corrupt files and successfully repaired them.")), SfcResult::Repaired);
    assert_eq!(repair::classify_sfc(&format!("{wrp} found corrupt files but was unable to fix some of them.")), SfcResult::NotAllFixed);
    assert_eq!(repair::classify_sfc(&format!("{wrp} could not perform the requested operation.")), SfcResult::CouldNotRun);
    assert_eq!(repair::classify_sfc("There is a system repair pending which requires reboot to complete.  Restart Windows and run SFC again."), SfcResult::RepairPending);
    assert_eq!(repair::classify_sfc("You must be an administrator running a console session in order to use the sfc utility."), SfcResult::NotAdmin);
    assert_eq!(repair::classify_sfc("Verification 100% complete.\nZaštita resursa sustava Windows nije pronašla kršenja integriteta."),
        SfcResult::Unknown("Zaštita resursa sustava Windows nije pronašla kršenja integriteta.".into()));
}

#[test]
fn repaired_files_are_counted_from_cbs_log_since_the_run_started() {
    let log = "\
2026-10-07 21:30:01, Info                  CSI    00000001 [SR] Repairing corrupted file [ml:520{260},l:36{18}]\"\\??\\C:\\Windows\\inf\"[l:18{9}]\"old.inf\" from store
2026-10-07 21:37:12, Info                  CSI    00000002 [SR] Verifying 100 components
2026-10-07 21:41:55, Info                  CSI    00000003 [SR] Repairing corrupted file [ml:520{260},l:36{18}]\"\\??\\C:\\Windows\\inf\"[l:18{9}]\"netnb.inf\" from store
2026-10-07 21:41:56, Info                  CSI    00000004 Repaired file \\SystemRoot\\WinSxS\\x\\\"netnb.inf\" by copying from backup
2026-10-07 21:42:03, Info                  CSI    00000005 [SR] Repairing corrupted file [ml:520{260},l:36{18}]\"\\??\\C:\\Windows\\System32\"[l:18{9}]\"a.dll\" from store
2026-10-07 21:50:00, Info                  CSI    00000006 [SR] Verify complete";
    let since = LocalTime { year: 2026, month: 10, day: 7, hour: 21, minute: 37 };
    assert_eq!(repair::count_sr_repairs(log, since), 2);
}

#[test]
fn outcomes_combine_and_read_like_the_design() {
    use RepairOutcome as O;
    assert_eq!(repair::combine(&DismResult::Healthy, Some(&SfcResult::NoViolations), None), O::NoProblems);
    assert_eq!(repair::combine(&DismResult::Repaired, Some(&SfcResult::NoViolations), None), O::Repaired { files: None });
    assert_eq!(repair::combine(&DismResult::Healthy, Some(&SfcResult::Repaired), Some(3)), O::Repaired { files: Some(3) });
    assert_eq!(repair::combine(&DismResult::Healthy, Some(&SfcResult::Repaired), Some(0)), O::Repaired { files: None });
    assert_eq!(repair::combine(&DismResult::Healthy, Some(&SfcResult::RepairPending), None), O::RestartFirst);
    assert_eq!(repair::combine(&DismResult::NotAdmin, None, None), O::NeedsAdmin);
    assert_eq!(repair::combine(&DismResult::Cancelled, None, None), O::Cancelled);
    assert!(matches!(
        repair::combine(&DismResult::Failed { exit_code: 0x800F_081F, line: "Error: 0x800f081f".into() }, Some(&SfcResult::NoViolations), None),
        O::Failed { phase: Phase::Dism, .. }
    ));
    assert_eq!(O::NoProblems.line(), "✓ No problems found");
    assert_eq!(O::Repaired { files: Some(4) }.line(), "Repaired 4 files · restart to finish");
    assert_eq!(O::Repaired { files: Some(1) }.line(), "Repaired 1 file · restart to finish");
}

// ======================================================================= 2 repair Windows files — the run

fn progress_log() -> (Arc<Mutex<Vec<Progress>>>, impl Fn(Progress) + Send + 'static) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let l2 = log.clone();
    (log, move |p| l2.lock().unwrap().push(p))
}

fn dism_ok() -> Script {
    Script::new(
        vec![
            b"Deployment Image Servicing and Management tool\r\nVersion: 10.0.26100.1\r\n\r\n".to_vec(),
            b"[=                          2.0%                           ]\r".to_vec(),
            b"[=========                  34.4%                          ]\r[==========                 34.9%    ".to_vec(),
            b"                      ]\r[==========================100.0%==========================] \r\n".to_vec(),
            b"The restore operation completed successfully.\r\nThe operation completed successfully.\r\n".to_vec(),
        ],
        0,
    )
}

fn sfc_with(result: &str) -> Script {
    Script::new(
        vec![
            bu_quickfix::fake::utf16("\r\nBeginning system scan.  This process will take some time.\r\n\r\nBeginning verification phase of system scan.\r\n"),
            bu_quickfix::fake::utf16("Verification 5% complete.\rVerification 71% complete.\rVerification 100% complete.\r\n\r\n"),
            bu_quickfix::fake::utf16(&format!("{result}\r\n")),
        ],
        0,
    )
}

#[test]
fn repair_needs_admin_and_starts_nothing_without_it() {
    let os = FakeFixOs::new();
    assert!(matches!(RepairRun::start(Arc::new(os.clone()), |_| {}), Err(FixError::NeedsAdmin(_))));
    assert!(os.spawned().is_empty());
}

#[test]
fn full_run_dism_then_sfc_with_progress_and_no_problems() {
    let os = FakeFixOs::new().elevated();
    os.script("dism.exe", dism_ok());
    os.script("sfc.exe", sfc_with("Windows Resource Protection did not find any integrity violations."));
    let (log, cb) = progress_log();
    let run = RepairRun::start(Arc::new(os.clone()), cb).unwrap();
    let rep = run.wait();
    assert_eq!(os.spawned(), vec![
        ("dism.exe".into(), vec!["/Online".into(), "/Cleanup-Image".into(), "/RestoreHealth".into()]),
        ("sfc.exe".into(), vec!["/scannow".into()]),
    ]);
    assert_eq!(rep.dism, DismResult::Healthy);
    assert_eq!(rep.sfc, Some(SfcResult::NoViolations));
    assert_eq!(rep.outcome, RepairOutcome::NoProblems);
    assert_eq!(rep.started, LocalTime { year: 2026, month: 10, day: 7, hour: 21, minute: 37 });
    assert_eq!(rep.line(utc(os.now())), "✓ No problems found · today, 21:37");
    let texts: Vec<String> = log.lock().unwrap().iter().map(|p| p.text()).collect();
    assert_eq!(texts, vec![
        "DISM · checking the Windows image",
        "DISM · checking the Windows image · 2 %",
        "DISM · checking the Windows image · 34 %",
        "DISM · checking the Windows image · 100 %",
        "sfc · checking system files",
        "sfc · checking system files · 5 %",
        "sfc · checking system files · 71 %",
        "sfc · checking system files · 100 %",
    ], "34.4 → 34.9 is the same whole percent: one update");
}

#[test]
fn repaired_run_counts_files_from_cbs_log() {
    let os = FakeFixOs::new().elevated();
    os.script("dism.exe", dism_ok());
    os.script("sfc.exe", sfc_with("Windows Resource Protection found corrupt files and successfully repaired them."));
    os.set_cbs_log("2026-10-07 21:40:00, Info CSI 00000001 [SR] Repairing corrupted file x from store\n2026-10-07 21:40:01, Info CSI 00000002 [SR] Repairing corrupted file y from store");
    let rep = RepairRun::start(Arc::new(os.clone()), |_| {}).unwrap().wait();
    assert_eq!(rep.outcome, RepairOutcome::Repaired { files: Some(2) });
    assert_eq!(rep.outcome.line(), "Repaired 2 files · restart to finish");
}

#[test]
fn sfc_still_runs_when_dism_fails_and_the_dism_failure_is_reported() {
    let os = FakeFixOs::new().elevated();
    os.script("dism.exe", Script::new(vec![b"Error: 0x800f081f\r\n\r\nThe source files could not be found.\r\n".to_vec()], 0x800F_081F));
    os.script("sfc.exe", sfc_with("Windows Resource Protection did not find any integrity violations."));
    let rep = RepairRun::start(Arc::new(os.clone()), |_| {}).unwrap().wait();
    assert_eq!(os.spawned().len(), 2);
    assert!(matches!(rep.dism, DismResult::Failed { exit_code: 0x800F_081F, .. }));
    assert!(rep.outcome.line().starts_with("DISM failed"), "{}", rep.outcome.line());
}

#[test]
fn cancel_during_dism_kills_it_and_never_starts_sfc() {
    let os = FakeFixOs::new().elevated();
    os.script("dism.exe", Script::new(vec![b"[====    12.0%    ]\r".to_vec()], 0).hanging());
    os.script("sfc.exe", sfc_with("Windows Resource Protection did not find any integrity violations."));
    let run = RepairRun::start(Arc::new(os.clone()), |_| {}).unwrap();
    os.wait_running(1);
    assert_eq!(run.progress(), Progress { phase: Phase::Dism, percent: Some(12.0) });
    assert!(!run.is_finished());
    run.cancel();
    let rep = run.wait();
    assert_eq!(rep.outcome, RepairOutcome::Cancelled);
    assert_eq!(rep.sfc, None);
    assert_eq!(os.killed(), 1);
    assert_eq!(os.spawned().len(), 1, "sfc never started");
}

#[test]
fn cancel_during_sfc_kills_it() {
    let os = FakeFixOs::new().elevated();
    os.script("dism.exe", dism_ok());
    os.script("sfc.exe", Script::new(vec![bu_quickfix::fake::utf16("Verification 40% complete.\r")], 0).hanging());
    let run = RepairRun::start(Arc::new(os.clone()), |_| {}).unwrap();
    os.wait_running(1);
    assert_eq!(run.progress().phase, Phase::Sfc);
    run.cancel();
    let rep = run.wait();
    assert_eq!(rep.dism, DismResult::Healthy);
    assert_eq!(rep.sfc, Some(SfcResult::Cancelled));
    assert_eq!(rep.outcome, RepairOutcome::Cancelled);
}

#[test]
fn a_program_that_fails_to_start_is_a_failure_not_a_panic() {
    let os = FakeFixOs::new().elevated(); // no scripts: spawn fails
    let rep = RepairRun::start(Arc::new(os), |_| {}).unwrap().wait();
    assert!(matches!(rep.outcome, RepairOutcome::Failed { phase: Phase::Dism, .. }));
}

#[test]
fn the_run_keeps_going_when_the_handle_is_dropped() {
    // "Keeps running when the menu closes": dropping RepairRun doesn't stop the programs
    let os = FakeFixOs::new().elevated();
    os.script("dism.exe", dism_ok());
    os.script("sfc.exe", Script::new(vec![bu_quickfix::fake::utf16("Verification 1% complete.\r")], 0).hanging());
    drop(RepairRun::start(Arc::new(os.clone()), |_| {}).unwrap());
    os.wait_running(1); // sfc got started after the handle was gone
    assert_eq!(os.spawned().len(), 2);
    assert_eq!(os.killed(), 0);
}

// ======================================================================= 3 icon & thumbnail cache

#[test]
fn only_icon_and_thumbnail_cache_files_qualify() {
    for yes in ["iconcache_16.db", "IconCache_idx.db", "thumbcache_1280.db", "thumbcache_idx.db"] {
        assert!(cache::is_cache_file(yes), "{yes}");
    }
    for no in ["ExplorerStartupLog.etl", "iconcache_16.db.bak", "thumbcache.txt", "IconCache.db.old", "desktop.ini"] {
        assert!(!cache::is_cache_file(no), "{no}");
    }
}

#[test]
fn rebuild_stops_explorer_deletes_the_cache_and_starts_it_again() {
    let os = FakeFixOs::new();
    for (n, b) in [("iconcache_16.db", 1_048_576), ("iconcache_idx.db", 4096), ("thumbcache_256.db", 9_437_184), ("ExplorerStartupLog.etl", 10)] {
        os.add_file(n, b);
    }
    assert_eq!(cache::cache_files(&os).unwrap().len(), 3, "read-only listing");
    let r = cache::rebuild(&os).unwrap();
    assert_eq!(os.events(), vec![
        "stop explorer",
        "delete iconcache_16.db",
        "delete iconcache_idx.db",
        "delete thumbcache_256.db",
        "restart explorer",
    ]);
    assert_eq!(os.files(), vec!["ExplorerStartupLog.etl"]);
    assert_eq!(r.bytes_freed(), 1_048_576 + 4096 + 9_437_184);
    assert!(r.skipped.is_empty() && os.explorer_running());
    assert_eq!(r.line(), "✓ Rebuilt just now · icons refill as you browse");
    assert!(cache::CONFIRM.starts_with("File Explorer restarts"));
}

#[test]
fn a_file_in_use_is_skipped_and_explorer_still_comes_back() {
    let os = FakeFixOs::new();
    os.add_file("thumbcache_96.db", 100);
    os.add_file("thumbcache_idx.db", 50);
    os.lock_file("thumbcache_96.db"); // e.g. a photo app holds it open
    let r = cache::rebuild(&os).unwrap();
    assert_eq!(r.deleted.len(), 1);
    assert_eq!(r.skipped.len(), 1);
    assert!(os.explorer_running());
    assert_eq!(os.events().last().map(String::as_str), Some("restart explorer"));
}

#[test]
fn a_failed_restart_is_reported_and_the_shell_is_back() {
    let os = FakeFixOs::new();
    os.add_file("iconcache_32.db", 1);
    os.fail_restart();
    assert!(matches!(cache::rebuild(&os), Err(FixError::Os { .. })));
    assert!(os.explorer_running());
}

#[test]
fn read_only_rebuild_never_stops_explorer() {
    let os = FakeFixOs::new().read_only();
    os.add_file("iconcache_32.db", 1);
    assert!(matches!(cache::rebuild(&os), Err(FixError::Refused(_))));
    assert!(os.events().is_empty());
    assert_eq!(os.files(), vec!["iconcache_32.db"]);
}

// ======================================================================= 4 restore point

#[test]
fn cim_datetimes_parse_to_utc() {
    assert_eq!(restore::parse_cim_datetime("20261002180400.000000+000"), Some(stamp(2026, 10, 2, 18, 4)));
    // 18:04 at UTC+2 (CEST, +120) is 16:04 UTC
    assert_eq!(restore::parse_cim_datetime("20261002180400.000000+120"), Some(stamp(2026, 10, 2, 16, 4)));
    assert_eq!(restore::parse_cim_datetime("20261002180400.000000-060"), Some(stamp(2026, 10, 2, 19, 4)));
    assert_eq!(restore::parse_cim_datetime("garbage"), None);
    assert_eq!(restore::parse_cim_datetime("20261302180400.000000+000"), None);
    assert_eq!(utc(stamp(2024, 2, 29, 23, 59)), LocalTime { year: 2024, month: 2, day: 29, hour: 23, minute: 59 });
}

#[test]
fn texts_read_like_the_design() {
    let t = LocalTime { year: 2026, month: 10, day: 2, hour: 18, minute: 4 };
    assert_eq!(restore::format_when(t), "2 Oct 2026, 18:04");
    assert_eq!(restore::description(t), "Boyler Utilities · 2 Oct 2026");
    let os = FakeFixOs::new();
    os.add_point(stamp(2026, 10, 2, 18, 4), "Windows Update");
    let st = os.restore_status().unwrap();
    assert_eq!(restore::status_line(&os, &st), "Only when you press it · last one 2 Oct 2026, 18:04");
}

#[test]
fn restore_point_needs_admin() {
    let os = FakeFixOs::new();
    assert!(matches!(restore::make_restore_point(&os), Err(FixError::NeedsAdmin(_))));
    assert!(os.create_calls().is_empty());
}

#[test]
fn makes_a_point_when_allowed() {
    let os = FakeFixOs::new().elevated(); // now = 7 Oct 2026 21:37
    os.add_point(stamp(2026, 10, 2, 18, 4), "Windows Update");
    let o = restore::make_restore_point(&os).unwrap();
    assert_eq!(os.create_calls(), vec!["Boyler Utilities · 7 Oct 2026"]);
    let now = utc(os.now());
    assert_eq!(o, RestoreOutcome::Made { at: now, description: "Boyler Utilities · 7 Oct 2026".into() });
    assert_eq!(o.line(now), "✓ Made today, 21:37 · \u{201c}Boyler Utilities · 7 Oct 2026\u{201d}");
}

#[test]
fn too_soon_says_windows_24h_rule_and_does_not_call() {
    let os = FakeFixOs::new().elevated();
    os.add_point(stamp(2026, 10, 7, 9, 15), "Windows Update");
    let o = restore::make_restore_point(&os).unwrap();
    assert!(os.create_calls().is_empty(), "no call — Windows would say yes and make nothing");
    assert_eq!(
        o.line(utc(os.now())),
        "Windows allows one restore point every 24 hours · the last one is from 7 Oct 2026, 09:15 · the next one from 8 Oct 2026, 09:15"
    );
}

#[test]
fn silent_refusal_by_windows_is_detected_after_the_call() {
    // the newest point can't be read before the call → we call; Windows says yes but makes nothing; the read after says so
    let os = FakeFixOs::new().elevated();
    os.add_point(stamp(2026, 10, 7, 20, 0), "Installed something");
    os.unreadable_until_create();
    let o = restore::make_restore_point(&os).unwrap();
    assert_eq!(os.create_calls().len(), 1);
    assert_eq!(os.points().len(), 1, "Windows made nothing");
    assert!(matches!(o, RestoreOutcome::TooSoon { .. }), "{o:?}");
}

#[test]
fn frequency_zero_means_no_limit_and_other_values_are_said_plainly() {
    let os = FakeFixOs::new().elevated();
    os.add_point(stamp(2026, 10, 7, 21, 0), "x");
    os.set_frequency(0);
    assert!(matches!(restore::make_restore_point(&os).unwrap(), RestoreOutcome::Made { .. }));
    let os = FakeFixOs::new().elevated();
    os.add_point(stamp(2026, 10, 7, 21, 0), "x");
    os.set_frequency(120);
    let o = restore::make_restore_point(&os).unwrap();
    assert!(o.line(utc(os.now())).starts_with("Windows allows one restore point every 2 hours"));
}

#[test]
fn protection_off_is_reported() {
    let os = FakeFixOs::new().elevated();
    os.set_protection_off();
    assert_eq!(restore::make_restore_point(&os).unwrap(), RestoreOutcome::ProtectionOff);
    assert!(os.points().is_empty());
}

#[test]
fn read_only_layer_never_makes_a_point() {
    let os = FakeFixOs::new().elevated().read_only();
    assert!(matches!(restore::make_restore_point(&os), Err(FixError::Refused(_))));
    assert!(os.points().is_empty());
}
