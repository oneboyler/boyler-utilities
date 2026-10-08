//! The real Windows layer. Reads are WMI (`root\Microsoft\Windows\Defender`, `root\SecurityCenter2`); changes run
//! Microsoft's own tools from fixed paths, with no window: `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`
//! (the Defender cmdlets) and Defender's `MpCmdRun.exe`. `RealOs::read_only()` refuses every change on its first line.

mod wmi;

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
use windows::Win32::System::Time::SystemTimeToTzSpecificLocalTime;

use crate::error::{Result, SecurityError};
use crate::model::*;
use crate::os::{CancelToken, ScanExit, SecurityOs};
use wmi::{Row, Wmi};

const DEFENDER_NS: &str = r"root\Microsoft\Windows\Defender";
const SECURITY_CENTER_NS: &str = r"root\SecurityCenter2";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct RealOs {
    read_only: bool,
}

impl RealOs {
    /// The app's layer: reads and changes.
    pub fn new() -> RealOs {
        RealOs { read_only: false }
    }
    /// What `security-show` and the tests run on: reads only; every change / scan is refused before anything starts.
    pub fn read_only() -> RealOs {
        RealOs { read_only: true }
    }

    fn refuse(&self, what: &str) -> Result<()> {
        if self.read_only {
            return Err(SecurityError::Os { call: what.to_string(), code: 5, text: "read-only layer: refused".into() });
        }
        Ok(())
    }
}

impl Default for RealOs {
    fn default() -> Self {
        RealOs::new()
    }
}

fn sys_dir() -> PathBuf {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join("System32")
}

fn powershell_exe() -> PathBuf {
    sys_dir().join(r"WindowsPowerShell\v1.0\powershell.exe")
}

/// The newest `<major>.<minor>.<build>-<n>` folder name (Defender's platform versions), compared number by number.
pub fn newest_platform(names: &[String]) -> Option<String> {
    fn key(s: &str) -> Vec<u64> {
        s.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).map(|p| p.parse().unwrap_or(0)).collect()
    }
    names.iter().filter(|n| n.chars().next().is_some_and(|c| c.is_ascii_digit())).max_by_key(|n| key(n)).cloned()
}

/// Defender's own command line tool: the newest copy under `%ProgramData%\...\Platform`, else `%ProgramFiles%\Windows Defender`.
pub fn mpcmdrun_path() -> Option<PathBuf> {
    if let Some(pd) = std::env::var_os("ProgramData") {
        let platform = PathBuf::from(pd).join(r"Microsoft\Windows Defender\Platform");
        if let Ok(rd) = std::fs::read_dir(&platform) {
            let names: Vec<String> = rd.flatten().filter(|e| e.path().is_dir()).filter_map(|e| e.file_name().into_string().ok()).collect();
            if let Some(best) = newest_platform(&names) {
                let p = platform.join(best).join("MpCmdRun.exe");
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    let pf = std::env::var_os("ProgramFiles").map(PathBuf::from)?;
    let p = pf.join(r"Windows Defender\MpCmdRun.exe");
    p.is_file().then_some(p)
}

/// The ids whose action is Allow (6): `ids[i]` goes with `actions[i]`. Entries that do not parse are skipped.
pub fn allowed_ids(ids: &[String], actions: &[String]) -> Vec<i64> {
    ids.iter().zip(actions).filter(|(_, a)| a.trim() == "6").filter_map(|(i, _)| i.trim().parse().ok()).collect()
}

/// Does a failure text mean "needs admin"?
pub fn looks_like_access_denied(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("access is denied") || t.contains("administrator privilege") || t.contains("0x80070005") || t.contains("0x80041003") || t.contains("access denied")
}

fn failure(call: &str, code: i32, text: &str) -> SecurityError {
    if looks_like_access_denied(text) {
        return SecurityError::NeedsAdmin;
    }
    let text: String = text.trim().chars().take(300).collect();
    SecurityError::Os { call: call.to_string(), code: code as u32, text }
}

/// The PowerShell wrapper every change uses: a failure becomes its message on stderr and exit code 1.
/// Arguments never go into the script text: they come in as environment variables (`BU_ID`, `BU_PATH`, `BU_TYPE`).
/// The Defender module is imported first, from `<SystemRoot>\System32\WindowsPowerShell\v1.0\Modules\Defender` by its full
/// path (never found through PSModulePath, whose first folder is the user's own).
fn ps_script(body: &str) -> String {
    format!(
        "$ErrorActionPreference='Stop'; try {{ Import-Module -Name (Join-Path $env:SystemRoot 'System32\\WindowsPowerShell\\v1.0\\Modules\\Defender'); {body}; exit 0 }} catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 1 }}"
    )
}

/// Every PowerShell body this crate can run, wrapped as it is run: (name, script, the cmdlet and the parameters it uses).
/// A test parses them (parse only, nothing runs) and checks the cmdlets and parameters exist.
pub fn change_scripts() -> Vec<(&'static str, String, &'static str, &'static [&'static str])> {
    vec![
        ("quick scan", ps_script(QUICK), "Start-MpScan", &["ScanType"]),
        ("full scan", ps_script(FULL), "Start-MpScan", &["ScanType"]),
        ("custom scan", ps_script(CUSTOM), "Start-MpScan", &["ScanType", "ScanPath"]),
        ("update definitions", ps_script(UPDATE), "Update-MpSignature", &[]),
        ("offline scan", ps_script(OFFLINE), "Start-MpWDOScan", &[]),
        ("remove threats", ps_script(REMOVE), "Remove-MpThreat", &[]),
        ("allow", ps_script(ALLOW), "Add-MpPreference", &["ThreatIDDefaultAction_Ids", "ThreatIDDefaultAction_Actions"]),
        ("disallow", ps_script(DISALLOW), "Remove-MpPreference", &["ThreatIDDefaultAction_Ids", "ThreatIDDefaultAction_Actions"]),
    ]
}

// Every cmdlet by its module-qualified name, after the Defender module is imported from Windows' own folder by its full path
// (ps_script): a module of the same name in the user's own module folder can never answer instead (Order 039 review - the
// changes run in the app's elevated copy).
const QUICK: &str = r"Defender\Start-MpScan -ScanType QuickScan";
const FULL: &str = r"Defender\Start-MpScan -ScanType FullScan";
const CUSTOM: &str = r"Defender\Start-MpScan -ScanType CustomScan -ScanPath $env:BU_PATH";
const UPDATE: &str = r"Defender\Update-MpSignature";
const OFFLINE: &str = r"Defender\Start-MpWDOScan";
const REMOVE: &str = r"Defender\Remove-MpThreat";
const ALLOW: &str = r"Defender\Add-MpPreference -ThreatIDDefaultAction_Ids ([int64]$env:BU_ID) -ThreatIDDefaultAction_Actions 6";
const DISALLOW: &str = r"Defender\Remove-MpPreference -ThreatIDDefaultAction_Ids ([int64]$env:BU_ID) -ThreatIDDefaultAction_Actions 6";

fn ps_command(body: &str, env: &[(&str, String)]) -> Command {
    let mut c = Command::new(powershell_exe());
    c.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", &ps_script(body)]);
    for (k, v) in env {
        c.env(k, v);
    }
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).creation_flags(CREATE_NO_WINDOW);
    c
}

fn run_ps(call: &str, body: &str, env: &[(&str, String)]) -> Result<()> {
    let out = ps_command(body, env)
        .spawn()
        .and_then(|c| c.wait_with_output())
        .map_err(|e| SecurityError::Os { call: call.to_string(), code: e.raw_os_error().unwrap_or(0) as u32, text: e.to_string() })?;
    if out.status.success() {
        Ok(())
    } else {
        Err(failure(call, out.status.code().unwrap_or(-1), &String::from_utf8_lossy(&out.stderr)))
    }
}

fn kill_pid(pid: u32) {
    unsafe {
        if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
            let _ = TerminateProcess(h, 1);
            let _ = CloseHandle(h);
        }
    }
}

fn stamp_from_cim(s: &str) -> Option<Stamp> {
    let parts = parse_cim_datetime(s)?;
    let (y, mo, d, h, mi, sec) = cim_to_utc(parts);
    let utc = windows::Win32::Foundation::SYSTEMTIME {
        wYear: y,
        wMonth: mo as u16,
        wDay: d as u16,
        wHour: h as u16,
        wMinute: mi as u16,
        wSecond: sec as u16,
        ..Default::default()
    };
    let mut local = windows::Win32::Foundation::SYSTEMTIME::default();
    unsafe { SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()? };
    Some(Stamp::new(local.wYear, local.wMonth as u8, local.wDay as u8, local.wHour as u8, local.wMinute as u8))
}

fn stamp(row: &Row, key: &str) -> Option<Stamp> {
    row.get(key).and_then(|v| stamp_from_cim(&v.str()))
}

fn flag(row: &Row, key: &str) -> bool {
    row.get(key).is_some_and(|v| v.bool())
}

impl SecurityOs for RealOs {
    fn is_elevated(&self) -> bool {
        use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }
            let mut elev = TOKEN_ELEVATION::default();
            let mut len = 0u32;
            let ok = GetTokenInformation(token, TokenElevation, Some(&mut elev as *mut _ as *mut _), size_of::<TOKEN_ELEVATION>() as u32, &mut len).is_ok();
            let _ = CloseHandle(token);
            ok && elev.TokenIsElevated != 0
        }
    }

    fn now(&self) -> Stamp {
        let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        Stamp::new(t.wYear, t.wMonth as u8, t.wDay as u8, t.wHour as u8, t.wMinute as u8)
    }

    fn defender_status(&self) -> Result<DefenderStatus> {
        let props = [
            "AMServiceEnabled", "AntivirusEnabled", "RealTimeProtectionEnabled", "IsTamperProtected", "AMRunningMode",
            "AntivirusSignatureVersion", "AntivirusSignatureLastUpdated", "QuickScanEndTime", "FullScanEndTime", "RebootRequired",
        ];
        let wmi = Wmi::connect(DEFENDER_NS)?;
        let rows = wmi.query("SELECT * FROM MSFT_MpComputerStatus", &props)?;
        let Some(r) = rows.first() else { return Err(SecurityError::DefenderNotRunning) };
        Ok(DefenderStatus {
            service_enabled: flag(r, "AMServiceEnabled"),
            antivirus_enabled: flag(r, "AntivirusEnabled"),
            realtime_enabled: flag(r, "RealTimeProtectionEnabled"),
            tamper_protected: flag(r, "IsTamperProtected"),
            running_mode: RunningMode::parse(&r["AMRunningMode"].str()),
            definitions_version: r["AntivirusSignatureVersion"].str(),
            definitions_updated: stamp(r, "AntivirusSignatureLastUpdated"),
            quick_scan_end: stamp(r, "QuickScanEndTime"),
            full_scan_end: stamp(r, "FullScanEndTime"),
            reboot_required: flag(r, "RebootRequired"),
        })
    }

    fn antivirus_products(&self) -> Result<Vec<AvProduct>> {
        let wmi = Wmi::connect(SECURITY_CENTER_NS)?;
        let rows = wmi.query("SELECT displayName, productState FROM AntiVirusProduct", &["displayName", "productState"])?;
        Ok(rows.iter().map(|r| AvProduct::from_product_state(&r["displayName"].str(), r["productState"].int().unwrap_or(0) as u32)).collect())
    }

    fn detections(&self) -> Result<Vec<Detection>> {
        let props = ["DetectionID", "ThreatID", "ThreatStatusID", "InitialDetectionTime", "LastThreatStatusChangeTime", "Resources"];
        let wmi = Wmi::connect(DEFENDER_NS)?;
        let rows = wmi.query("SELECT * FROM MSFT_MpThreatDetection", &props)?;
        Ok(rows
            .iter()
            .map(|r| Detection {
                detection_id: r["DetectionID"].str(),
                threat_id: r["ThreatID"].int().unwrap_or(0),
                status_id: r["ThreatStatusID"].int().unwrap_or(0),
                found: stamp(r, "InitialDetectionTime"),
                status_changed: stamp(r, "LastThreatStatusChangeTime"),
                resources: r["Resources"].list(),
            })
            .collect())
    }

    fn threats(&self) -> Result<Vec<ThreatInfo>> {
        let wmi = Wmi::connect(DEFENDER_NS)?;
        let rows = wmi.query("SELECT * FROM MSFT_MpThreat", &["ThreatID", "ThreatName", "SeverityID", "IsActive"])?;
        Ok(rows
            .iter()
            .map(|r| ThreatInfo {
                threat_id: r["ThreatID"].int().unwrap_or(0),
                name: r["ThreatName"].str(),
                severity: Severity::from_id(r["SeverityID"].int().unwrap_or(0)),
                active: flag(r, "IsActive"),
            })
            .collect())
    }

    fn path_exists(&self, path: &str) -> bool {
        std::path::Path::new(path).exists()
    }

    fn allowed_threat_ids(&self) -> Result<Vec<i64>> {
        let wmi = Wmi::connect(DEFENDER_NS)?;
        let rows = wmi.query("SELECT * FROM MSFT_MpPreference", &["ThreatIDDefaultAction_Ids", "ThreatIDDefaultAction_Actions"])?;
        let Some(r) = rows.first() else { return Ok(Vec::new()) };
        let ids = r["ThreatIDDefaultAction_Ids"].list();
        let actions = r["ThreatIDDefaultAction_Actions"].list();
        Ok(allowed_ids(&ids, &actions))
    }

    fn run_scan(&self, kind: &ScanKind, cancel: &CancelToken) -> Result<ScanExit> {
        self.refuse("scan")?;
        let (body, env): (&str, Vec<(&str, String)>) = match kind {
            ScanKind::Quick => (QUICK, vec![]),
            ScanKind::Full => (FULL, vec![]),
            ScanKind::Path(p) => (CUSTOM, vec![("BU_PATH", p.clone())]),
        };
        let child = ps_command(body, &env)
            .spawn()
            .map_err(|e| SecurityError::Os { call: "Start-MpScan".into(), code: e.raw_os_error().unwrap_or(0) as u32, text: e.to_string() })?;
        let pid = child.id();
        let ended = Arc::new(AtomicBool::new(false));
        let ended_hook = ended.clone();
        // Cancel kills our PowerShell and, for quick / full scans, tells Defender to stop (`MpCmdRun -Scan -Cancel`). That call
        // stops ANY running scan, also one the user started in Windows Security. For a file / folder scan only PowerShell is killed,
        // so Defender may keep scanning that path while the report says Cancelled.
        let stop_service_scan = !matches!(kind, ScanKind::Path(_)); // `-Scan -Cancel` is documented for quick / full scans only
        cancel.on_cancel(move || {
            if !ended_hook.load(Ordering::SeqCst) {
                kill_pid(pid);
            }
            if stop_service_scan {
                if let Some(mp) = mpcmdrun_path() {
                    // spawn, do not wait: the Cancel link must not block the caller (MpCmdRun ends on its own)
                    let _ = Command::new(mp).args(["-Scan", "-Cancel"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).creation_flags(CREATE_NO_WINDOW).spawn();
                }
            }
        });
        let out = child.wait_with_output();
        ended.store(true, Ordering::SeqCst);
        let out = out.map_err(|e| SecurityError::Os { call: "Start-MpScan".into(), code: e.raw_os_error().unwrap_or(0) as u32, text: e.to_string() })?;
        if cancel.is_cancelled() {
            return Ok(ScanExit::Cancelled);
        }
        if out.status.success() {
            Ok(ScanExit::Finished)
        } else {
            Err(failure("Start-MpScan", out.status.code().unwrap_or(-1), &String::from_utf8_lossy(&out.stderr)))
        }
    }

    fn update_definitions(&self) -> Result<()> {
        self.refuse("Update-MpSignature")?;
        run_ps("Update-MpSignature", UPDATE, &[])
    }

    fn start_offline_scan(&self) -> Result<()> {
        self.refuse("Start-MpWDOScan")?;
        run_ps("Start-MpWDOScan", OFFLINE, &[])
    }

    fn remove_active_threats(&self) -> Result<()> {
        self.refuse("Remove-MpThreat")?;
        run_ps("Remove-MpThreat", REMOVE, &[])
    }

    fn allow_threat(&self, threat_id: i64) -> Result<()> {
        self.refuse("Add-MpPreference")?;
        run_ps("Add-MpPreference", ALLOW, &[("BU_ID", threat_id.to_string())])
    }

    fn disallow_threat(&self, threat_id: i64) -> Result<()> {
        self.refuse("Remove-MpPreference")?;
        run_ps("Remove-MpPreference", DISALLOW, &[("BU_ID", threat_id.to_string())])
    }

    fn open_protection_history(&self) -> Result<()> {
        self.refuse("open Protection history")?; // a window on the screen: never from the read-only layer
        let w: Vec<u16> = "windowsdefender://history".encode_utf16().chain(Some(0)).collect();
        // ShellExecute asks the caller to have COM started (apartment, no DDE); a thread that already has COM is left alone.
        use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
        let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok();
        let h = unsafe {
            windows::Win32::UI::Shell::ShellExecuteW(
                None,
                windows::core::w!("open"),
                windows::core::PCWSTR(w.as_ptr()),
                windows::core::PCWSTR::null(),
                windows::core::PCWSTR::null(),
                windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
            )
        };
        if com {
            unsafe { CoUninitialize() };
        }
        // ShellExecute reports success with a value above 32
        if (h.0 as isize) > 32 {
            Ok(())
        } else {
            Err(SecurityError::Os { call: "ShellExecute windowsdefender://history".into(), code: h.0 as isize as u32, text: "could not open Windows Security".into() })
        }
    }

    fn restore_quarantined(&self, file_path: &str) -> Result<()> {
        self.refuse("MpCmdRun -Restore")?;
        let mp = mpcmdrun_path().ok_or(SecurityError::DefenderNotRunning)?;
        let out = Command::new(mp)
            .args(["-Restore", "-FilePath", file_path])
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| SecurityError::Os { call: "MpCmdRun -Restore".into(), code: e.raw_os_error().unwrap_or(0) as u32, text: e.to_string() })?;
        if out.status.success() {
            Ok(())
        } else {
            let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            Err(failure("MpCmdRun -Restore", out.status.code().unwrap_or(-1), &text))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The read-only guard is proven WITHOUT calling any real change: `refuse` itself, and the source order of every change
    /// method (the guard is its first statement, before anything could start). A broken guard therefore cannot be "tested"
    /// by really changing Defender.
    #[test]
    fn refuse_blocks_only_the_read_only_layer() {
        assert!(matches!(RealOs::read_only().refuse("x"), Err(SecurityError::Os { code: 5, ref text, .. }) if text.contains("read-only")));
        assert!(RealOs::new().refuse("x").is_ok());
    }

    #[test]
    fn every_change_method_starts_with_the_guard() {
        let src = include_str!("mod.rs");
        for name in ["run_scan", "update_definitions", "start_offline_scan", "remove_active_threats", "allow_threat", "disallow_threat", "restore_quarantined", "open_protection_history"] {
            let needle = format!("    fn {name}(");
            let at = src.find(&needle).unwrap_or_else(|| panic!("{name} not found"));
            let body = src[at..].lines().nth(1).unwrap_or("");
            assert!(body.trim_start().starts_with("self.refuse("), "{name}: first line is `{body}`");
        }
    }

    #[test]
    fn allow_list_pairs_ids_with_actions() {
        let ids: Vec<String> = ["2147735503", "5", "-1", "x"].iter().map(|s| s.to_string()).collect();
        let actions: Vec<String> = ["6", "2", "6", "6"].iter().map(|s| s.to_string()).collect();
        assert_eq!(allowed_ids(&ids, &actions), vec![2147735503, -1]);
        assert!(allowed_ids(&[], &[]).is_empty());
    }
}
