//! The real Windows layer, read-only: reads work, every change is refused before anything starts, and the pure helpers
//! (platform folder choice, access-denied wording) behave. No scan is started, nothing in Defender or Quarantine changes.
#![cfg(windows)]

use bu_security::real::{looks_like_access_denied, mpcmdrun_path, newest_platform};
use bu_security::*;

#[test]
fn newest_platform_folder_is_compared_number_by_number() {
    let names: Vec<String> = ["4.18.26080.3-0", "4.18.26080.4-0", "4.18.9000.1-0", "4.18.26080.10-0", "Updates", ""].iter().map(|s| s.to_string()).collect();
    assert_eq!(newest_platform(&names).as_deref(), Some("4.18.26080.10-0"));
    assert_eq!(newest_platform(&[]), None);
    assert_eq!(newest_platform(&["Updates".to_string()]), None);
}

#[test]
fn access_denied_wording() {
    assert!(looks_like_access_denied("You need administrator privilege to execute this command.\n[Failed][0x80070005] Access is denied."));
    assert!(looks_like_access_denied("Access is denied"));
    assert!(looks_like_access_denied("Operation failed with 0x80041003"));
    assert!(!looks_like_access_denied("The term 'Foo' is not recognized"));
}

#[test]
fn real_reads_work_and_start_nothing() {
    let os = RealOs::read_only();
    let status = os.defender_status();
    // Defender may be absent on a test PC; if it is there, the page reads in full
    if let Ok(s) = status {
        assert!(!s.definitions_version.is_empty());
        assert!(os.allowed_threat_ids().is_ok(), "the allow list reads (empty on this PC; unproven with entries)");
        let svc = SecurityService::new(std::sync::Arc::new(RealOs::read_only()));
        let page = svc.page().unwrap();
        assert!(matches!(svc.scan_state(), ScanState::Idle));
        let _ = page;
    }
    assert!(os.antivirus_products().is_ok());
    assert!(mpcmdrun_path().is_none_or(|p| p.is_file()));
}

/// Harmless targets only. The read-only guard itself is proven in the crate's unit tests (`real::tests`) without calling any
/// change. Here only ONE change method is touched, with a target nobody has: restoring a file that is not in quarantine does
/// nothing even if the guard were broken. Scans, Allow, Remove and the offline scan are never called on the real layer.
#[test]
fn read_only_layer_refuses_a_restore_before_anything_starts() {
    let os = RealOs::read_only();
    let refused = |r: Result<()>| matches!(r, Err(SecurityError::Os { code: 5, ref text, .. }) if text.contains("read-only"));
    assert!(refused(os.restore_quarantined(r"C:\BoylerUtilities-test-no-such-folder\no-such-file.exe")));
    // the allow list read works (it is a read) and starts nothing
    let _ = os.allowed_threat_ids();
}

/// Parse-only proof for the PowerShell bodies behind the changes: PowerShell's own parser reads each wrapped script (nothing
/// runs), the cmdlet exists, its parameters exist, the scan types exist as words, and the number 6 is "Allow". The scripts
/// themselves are never run here (that would scan, change or restart).
#[test]
fn the_powershell_bodies_parse_and_use_real_cmdlets_and_parameters() {
    let ps = std::path::Path::new(&std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let check = r#"
$e=$null; $t=$null
[void][System.Management.Automation.Language.Parser]::ParseInput($env:BU_SCRIPT,[ref]$t,[ref]$e)
if ($e.Count) { $e | ForEach-Object { $_.Message }; exit 1 }
$c = Get-Command $env:BU_CMD -ErrorAction Stop
foreach ($p in $env:BU_PARAMS.Split(',')) { if ($p -and -not $c.Parameters.ContainsKey($p)) { 'missing parameter ' + $p; exit 2 } }
if ($env:BU_CMD -eq 'Start-MpScan') {
  $names = [Enum]::GetNames($c.Parameters['ScanType'].ParameterType)
  foreach ($n in 'QuickScan','FullScan','CustomScan') { if ($names -notcontains $n) { 'no scan type ' + $n; exit 3 } }
}
if ($env:BU_CMD -like '*-MpPreference') {
  $el = $c.Parameters['ThreatIDDefaultAction_Actions'].ParameterType.GetElementType()
  if ([Enum]::ToObject($el, 6).ToString() -ne 'Allow') { 'six is not Allow: ' + [Enum]::ToObject($el, 6); exit 4 }
}
exit 0
"#;
    for (name, script, cmd, params) in bu_security::real::change_scripts() {
        let out = std::process::Command::new(&ps)
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", check])
            .env("BU_SCRIPT", &script)
            .env("BU_CMD", cmd)
            .env("BU_PARAMS", params.join(","))
            .output()
            .expect("powershell runs");
        assert!(out.status.success(), "{name}: {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    }
}
