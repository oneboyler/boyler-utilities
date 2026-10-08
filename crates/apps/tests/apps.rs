//! Every §3.13 row against the fake OS layer. Nothing real is ever uninstalled.

use bu_apps::fake::{d, s, FakeOs};
use bu_apps::os::{RawPackage, Signature};
use bu_apps::*;

const HKLM: Hive = Hive::LocalMachine;
const HKCU: Hive = Hive::CurrentUser;
const B64: RegView = RegView::Bits64;
const B32: RegView = RegView::Bits32;
const MSI: &str = "{12345678-1234-1234-1234-1234567890AB}";
/// 2026-09-02 00:00 UTC as FILETIME.
const SEP2: u64 = 116_444_736_000_000_000 + 1_788_307_200 * 10_000_000;

fn pkg(name: &str, display: Option<&str>, sig: Signature) -> RawPackage {
    RawPackage {
        full_name: format!("{name}_1.0.0.0_x64__abc"),
        family_name: format!("{name}_abc"),
        name: name.into(),
        display_name: display.map(Into::into),
        publisher: Some("Pub".into()),
        version: "1.0.0.0".into(),
        installed: Some(SEP2),
        logo: Some(format!(r"C:\Program Files\WindowsApps\{name}\Logo.png").into()),
        installed_path: Some(format!(r"C:\Program Files\WindowsApps\{name}").into()),
        is_framework: false,
        is_resource: false,
        is_bundle: false,
        is_optional: false,
        signature: sig,
    }
}

fn pc() -> FakeOs {
    let mut fw = pkg("Microsoft.VCLibs.140.00", Some("VCLibs"), Signature::Store);
    fw.is_framework = true;
    let mut res = pkg("Game.Lang.de", Some("German pack"), Signature::Store);
    res.is_resource = true;
    let mut opt = pkg("Game.DLC", Some("DLC"), Signature::Store);
    opt.is_optional = true;
    FakeOs::new()
        .entry(HKLM, B64, "Valorant", &[("DisplayName", s("VALORANT")), ("Publisher", s("Riot Games, Inc")), ("EstimatedSize", d(35_190_000)),
            ("InstallDate", s("20260929")), ("UninstallString", s(r#""C:\Riot\RiotClientServices.exe" --uninstall-product=valorant"#)),
            ("DisplayIcon", s(r#""C:\Riot\valorant.ico",0"#)), ("InstallLocation", s(r"C:\Riot Games\VALORANT"))], None)
        .entry(HKLM, B64, "Riot Vanguard", &[("DisplayName", s("Riot Vanguard")), ("UninstallString", s(r#""C:\Program Files\Riot Vanguard\uninstall.exe""#))], Some(SEP2))
        .entry(HKLM, B32, "Microsoft Edge", &[("DisplayName", s("Microsoft Edge")), ("EstimatedSize", d(2_000_000)),
            ("UninstallString", s(r#""C:\Edge\setup.exe" --uninstall"#))], None)
        .entry(HKLM, B64, MSI, &[("DisplayName", s("Some MSI App")), ("WindowsInstaller", d(1)), ("UninstallString", s(&format!("MsiExec.exe /I{MSI}"))),
            ("DisplayVersion", s("2.1")), ("EstimatedSize", d(1024))], None)
        .entry(HKCU, B64, "Discord", &[("DisplayName", s("Discord")), ("QuietUninstallString", s(r#""C:\Users\u\Discord\Update.exe" --uninstall -s"#)),
            ("UninstallString", s(r#""C:\Users\u\Discord\Update.exe" --uninstall"#)), ("EstimatedSize", d(100_000)), ("InstallDate", s("20261001"))], None)
        .entry(HKLM, B64, "NoRemoveApp", &[("DisplayName", s("Driver Pack")), ("NoRemove", d(1)), ("UninstallString", s("x.exe"))], None)
        .entry(HKLM, B64, "BrokenApp", &[("DisplayName", s("Broken App"))], None)
        // Hidden by Windows' own rules:
        .entry(HKLM, B64, "Sys", &[("DisplayName", s("System part")), ("SystemComponent", d(1)), ("UninstallString", s("x"))], None)
        .entry(HKLM, B64, "KB1", &[("DisplayName", s("Update for X")), ("ParentKeyName", s("X")), ("UninstallString", s("x"))], None)
        .entry(HKLM, B64, "KB2", &[("DisplayName", s("Security Update for X")), ("ReleaseType", s("Security Update")), ("UninstallString", s("x"))], None)
        .entry(HKLM, B64, "NoName", &[("UninstallString", s("x"))], None)
        // Same app listed in both views (identical) → once.
        .entry(HKLM, B32, "Dup", &[("DisplayName", s("Dup App")), ("UninstallString", s("dup.exe"))], None)
        .entry(HKLM, B64, "Dup", &[("DisplayName", s("Dup App")), ("UninstallString", s("dup.exe"))], None)
        // Store
        .package(pkg("SpotifyAB.SpotifyMusic", Some("Spotify"), Signature::Store))
        .package(pkg("Microsoft.WindowsStore", Some("Microsoft Store"), Signature::Store))
        .package(pkg("Microsoft.Windows.FileExplorer", Some("File Explorer"), Signature::System))
        .package(pkg("Microsoft.MicrosoftEdge.Stable", Some("Microsoft Edge"), Signature::Store))
        .package(pkg("Some.Plumbing", Some("ms-resource:AppName"), Signature::Store))
        .package(pkg("No.Name", None, Signature::Developer))
        .package(fw)
        .package(res)
        .package(opt)
        .folder(r"C:\Program Files\WindowsApps\SpotifyAB.SpotifyMusic", 300 * 1024 * 1024)
}

fn by_name<'a>(l: &'a AppList, name: &str) -> &'a InstalledApp {
    l.apps.iter().find(|a| a.name == name).unwrap_or_else(|| panic!("no {name}: {:?}", l.apps.iter().map(|a| &a.name).collect::<Vec<_>>()))
}

#[test]
fn desktop_apps_from_all_three_places_with_windows_hiding_rules() {
    let l = Apps::new(pc()).list();
    let names: Vec<&str> = l.apps.iter().filter(|a| a.kind == AppKind::Desktop).map(|a| a.name.as_str()).collect();
    for hidden in ["System part", "Update for X", "Security Update for X"] {
        assert!(!names.contains(&hidden), "{hidden} must be hidden");
    }
    assert_eq!(names.iter().filter(|n| **n == "Dup App").count(), 1);
    for shown in ["VALORANT", "Riot Vanguard", "Microsoft Edge", "Some MSI App", "Discord", "Driver Pack", "Broken App"] {
        assert!(names.contains(&shown), "{shown} listed");
    }
    assert!(by_name(&l, "Discord").id.starts_with("desktop|HKCU|"));
    assert!(by_name(&l, "Microsoft Edge").id.starts_with("desktop|HKLM|Bits32|"));
}

#[test]
fn desktop_fields() {
    let l = Apps::new(pc()).list();
    let v = by_name(&l, "VALORANT");
    assert_eq!(v.publisher.as_deref(), Some("Riot Games, Inc"));
    assert_eq!(v.size_bytes, Some(35_190_000 * 1024), "EstimatedSize is KB");
    assert_eq!(v.size_source, SizeSource::Estimated);
    assert_eq!(v.install_date, Some(Date { year: 2026, month: 9, day: 29 }));
    assert_eq!(v.date_source, Some(DateSource::Registry));
    assert_eq!(v.icon_path.as_deref(), Some(std::path::Path::new(r"C:\Riot\valorant.ico")));
    assert_eq!(v.uninstall_command.as_deref(), Some(r#""C:\Riot\RiotClientServices.exe" --uninstall-product=valorant"#));
    assert!(!v.quiet);
    assert!(v.admin_prompt_likely, "HKLM entry");
    assert_eq!(v.lock, None);
    assert_eq!(open_target(v).as_deref(), Some(r"C:\Riot Games\VALORANT"));

    let vg = by_name(&l, "Riot Vanguard");
    assert_eq!(vg.size_bytes, None);
    assert_eq!(vg.size_source, SizeSource::Unknown);
    assert_eq!(vg.install_date, Some(Date { year: 2026, month: 9, day: 2 }), "no InstallDate → the entry's write time");
    assert_eq!(vg.date_source, Some(DateSource::EntryWritten));
    assert_eq!(vg.warning(), Some("VALORANT won't start without it."));

    let dc = by_name(&l, "Discord");
    assert!(dc.quiet, "the app offers a quiet uninstall → used");
    assert_eq!(dc.uninstall_command.as_deref(), Some(r#""C:\Users\u\Discord\Update.exe" --uninstall -s"#));
    assert!(!dc.admin_prompt_likely);

    let msi = by_name(&l, "Some MSI App");
    assert_eq!(msi.uninstall_command, Some(format!("MsiExec.exe /X{MSI}")), "MSI: /X removes (/I would open the change dialog)");
    assert_eq!(msi.version.as_deref(), Some("2.1"));
    assert_eq!(msi.size_bytes, Some(1024 * 1024));
}

#[test]
fn locked_rows() {
    let l = Apps::new(pc()).list();
    assert_eq!(by_name(&l, "Microsoft Edge").lock, Some(LockReason::WindowsKeeps));
    assert_eq!(by_name(&l, "Driver Pack").lock, Some(LockReason::NoRemove));
    assert_eq!(by_name(&l, "Broken App").lock, Some(LockReason::Broken));
    assert!(!by_name(&l, "Broken App").can_uninstall());
    let store_edge = l.apps.iter().find(|a| a.name == "Microsoft Edge" && a.kind == AppKind::Store).unwrap();
    assert_eq!(store_edge.lock, Some(LockReason::WindowsKeeps), "Edge locked as a Store package too");
    assert_eq!(by_name(&l, "Microsoft Store").lock, Some(LockReason::WindowsKeeps));
    assert_eq!(by_name(&l, "Spotify").lock, None);
}

#[test]
fn store_apps() {
    let l = Apps::new(pc()).list();
    let names: Vec<&str> = l.apps.iter().filter(|a| a.kind == AppKind::Store).map(|a| a.name.as_str()).collect();
    for hidden in ["VCLibs", "German pack", "DLC", "File Explorer", "ms-resource:AppName"] {
        assert!(!names.contains(&hidden), "{hidden} hidden");
    }
    assert_eq!(names.len(), 3, "Spotify, Microsoft Store, Microsoft Edge: {names:?}");
    let sp = by_name(&l, "Spotify");
    assert_eq!(sp.size_bytes, None, "the package manager gives no size");
    assert_eq!(sp.size_source, SizeSource::Unknown);
    assert_eq!(sp.install_date, Some(Date { year: 2026, month: 9, day: 2 }));
    assert_eq!(sp.date_source, Some(DateSource::Package));
    assert_eq!(sp.version.as_deref(), Some("1.0.0.0"));
    assert_eq!(sp.icon_path.as_deref(), Some(std::path::Path::new(r"C:\Program Files\WindowsApps\SpotifyAB.SpotifyMusic\Logo.png")));
    assert_eq!(sp.settings_uri.as_deref(), Some("ms-settings:appsfeatures-app?SpotifyAB.SpotifyMusic_abc"), "that app's own Settings page");
    assert_eq!(open_target(sp).as_deref(), Some("ms-settings:appsfeatures-app?SpotifyAB.SpotifyMusic_abc"));
    assert_eq!(sp.uninstall_command, None);
}

#[test]
fn size_measured_on_demand() {
    let apps = Apps::new(pc());
    let l = apps.list();
    let m = apps.measure_size(by_name(&l, "Spotify"));
    assert_eq!(m.size_bytes, Some(300 * 1024 * 1024));
    assert_eq!(m.size_source, SizeSource::Measured);
    // Unreadable folder → unchanged.
    let v = by_name(&l, "Riot Vanguard");
    assert_eq!(apps.measure_size(v), v.clone());
}

#[test]
fn sorting_and_search() {
    let l = Apps::new(pc()).list();
    // Default: size, biggest first, unknown last (by name).
    let sizes: Vec<Option<u64>> = l.apps.iter().map(|a| a.size_bytes).collect();
    let first_unknown = sizes.iter().position(|s| s.is_none()).unwrap();
    assert!(sizes[..first_unknown].windows(2).all(|w| w[0] >= w[1]));
    assert!(sizes[first_unknown..].iter().all(|s| s.is_none()));
    assert_eq!(l.apps[0].name, "VALORANT");

    let mut v = l.apps.clone();
    sort(&mut v, SortKey::Size, false);
    assert_eq!(v.iter().position(|a| a.size_bytes.is_none()), Some(first_unknown), "unknown last in both directions");
    sort(&mut v, SortKey::Name, false);
    let names: Vec<String> = v.iter().map(|a| a.name.to_lowercase()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    sort(&mut v, SortKey::Installed, true);
    assert_eq!(v[0].name, "Discord", "newest install first");

    assert_eq!(search(&l.apps, "riot").len(), 2, "VALORANT (publisher) + Riot Vanguard (name)");
    assert_eq!(search(&l.apps, "").len(), l.apps.len());
    assert!(search(&l.apps, "zzz").is_empty());
}

#[test]
fn confirm_sheet_data() {
    let apps = Apps::new(pc());
    let l = apps.list();
    let v = by_name(&l, "VALORANT");
    let vg = by_name(&l, "Riot Vanguard");
    let edge = by_name(&l, "Microsoft Edge");
    let c = apps.confirm(&[v, vg, edge]);
    assert_eq!(c.apps.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(), vec!["VALORANT", "Riot Vanguard"]);
    assert_eq!(c.frees_bytes, 35_190_000 * 1024);
    assert_eq!(c.unknown_sizes, 1, "Vanguard's size is unknown → 'about'");
    assert!(!c.store_only);
    assert!(c.uninstaller_may_open);
    assert_eq!(c.warnings, vec!["Riot Vanguard: VALORANT won't start without it.".to_string()]);
    assert_eq!(c.skipped_locked, vec![("Microsoft Edge".to_string(), LockReason::WindowsKeeps)]);

    let sp = by_name(&l, "Spotify");
    let c = apps.confirm(&[sp]);
    assert!(c.store_only, "Store apps are removed straight away");
    assert!(!c.uninstaller_may_open);
}

#[test]
fn desktop_uninstall_outcomes() {
    let dc_cmd = r#""C:\Users\u\Discord\Update.exe" --uninstall -s"#;
    let vg_cmd = r#""C:\Program Files\Riot Vanguard\uninstall.exe""#;
    let msi_cmd = format!("MsiExec.exe /X{MSI}");
    let v_cmd = r#""C:\Riot\RiotClientServices.exe" --uninstall-product=valorant"#;

    // Success: the entry is gone.
    let apps = Apps::new(pc().script(dc_cmd, 0, true));
    let l = apps.list();
    assert_eq!(apps.uninstall(by_name(&l, "Discord")), Ok(Outcome::Uninstalled { freed: Some(100_000 * 1024) }));
    assert!(apps.list().apps.iter().all(|a| a.name != "Discord"), "row folds away");
    assert_eq!(apps.os().state().ran, vec![format!("run {dc_cmd}")]);

    // The user cancels the uninstaller (MSI 1602, NSIS-style 2, or 0 with the app still there): "Not uninstalled".
    let apps = Apps::new(pc().script(&msi_cmd, 1602, false).script(vg_cmd, 2, false).script(v_cmd, 0, false));
    let l = apps.list();
    assert_eq!(apps.uninstall(by_name(&l, "Some MSI App")), Ok(Outcome::NotUninstalled { exit_code: Some(1602) }));
    assert_eq!(apps.uninstall(by_name(&l, "Riot Vanguard")), Ok(Outcome::NotUninstalled { exit_code: Some(2) }));
    assert_eq!(apps.uninstall(by_name(&l, "VALORANT")), Ok(Outcome::NotUninstalled { exit_code: Some(0) }));

    // Restart needed.
    let apps = Apps::new(pc().script(&msi_cmd, 3010, false));
    assert_eq!(apps.uninstall(by_name(&apps.list(), "Some MSI App")), Ok(Outcome::NeedsRestart));
    let apps = Apps::new(pc().script(&msi_cmd, 1641, true));
    assert_eq!(apps.uninstall(by_name(&apps.list(), "Some MSI App")), Ok(Outcome::NeedsRestart));

    // No at Windows' admin prompt → nothing happened.
    let apps = Apps::new(pc().script_prompt_cancelled(vg_cmd));
    assert_eq!(apps.uninstall(by_name(&apps.list(), "Riot Vanguard")), Ok(Outcome::NotUninstalled { exit_code: None }));
}

#[test]
fn locked_and_broken_never_run() {
    let apps = Apps::new(pc());
    let l = apps.list();
    assert_eq!(apps.uninstall(by_name(&l, "Microsoft Edge")), Err(AppsError::Locked(LockReason::WindowsKeeps)));
    assert_eq!(apps.uninstall(by_name(&l, "Driver Pack")), Err(AppsError::Locked(LockReason::NoRemove)));
    assert_eq!(apps.uninstall(by_name(&l, "Broken App")), Err(AppsError::Locked(LockReason::Broken)));
    assert_eq!(apps.uninstall(by_name(&l, "Microsoft Store")), Err(AppsError::Locked(LockReason::WindowsKeeps)));
    assert!(apps.os().state().ran.is_empty(), "nothing ran");
}

#[test]
fn store_uninstall() {
    let apps = Apps::new(pc());
    let l = apps.list();
    let sp = apps.measure_size(by_name(&l, "Spotify"));
    assert_eq!(apps.uninstall(&sp), Ok(Outcome::Uninstalled { freed: Some(300 * 1024 * 1024) }));
    assert_eq!(apps.os().state().ran, vec!["remove SpotifyAB.SpotifyMusic_1.0.0.0_x64__abc".to_string()]);
    assert!(apps.list().apps.iter().all(|a| a.name != "Spotify"));

    let f = pc();
    f.state().package_errors.insert(
        "SpotifyAB.SpotifyMusic_1.0.0.0_x64__abc".into(),
        OsError::Other { code: 0x80073CFA_u32 as i32, message: "Removal failed".into() },
    );
    let apps = Apps::new(f);
    let l = apps.list();
    assert!(matches!(apps.uninstall(by_name(&l, "Spotify")), Err(AppsError::Os(OsError::Other { .. }))));
    assert!(apps.list().apps.iter().any(|a| a.name == "Spotify"), "row comes back");
}

#[test]
fn several_one_after_another_with_progress() {
    let dc_cmd = r#""C:\Users\u\Discord\Update.exe" --uninstall -s"#;
    let vg_cmd = r#""C:\Program Files\Riot Vanguard\uninstall.exe""#;
    let apps = Apps::new(pc().script(dc_cmd, 0, true).script(vg_cmd, 1602, false));
    let l = apps.list();
    let picked = [by_name(&l, "Discord"), by_name(&l, "Microsoft Edge"), by_name(&l, "Riot Vanguard"), by_name(&l, "Spotify")];
    let mut log = Vec::new();
    let out = apps.uninstall_many(&picked, |i, p| log.push((i, p)));
    assert_eq!(out.len(), 4);
    assert_eq!(out[0], Ok(Outcome::Uninstalled { freed: Some(100_000 * 1024) }));
    assert_eq!(out[1], Err(AppsError::Locked(LockReason::WindowsKeeps)));
    assert_eq!(out[2], Ok(Outcome::NotUninstalled { exit_code: Some(1602) }));
    assert_eq!(out[3], Ok(Outcome::Uninstalled { freed: None }));
    // All Waiting first, then each one in turn.
    assert_eq!(&log[..4], &[(0, Progress::Waiting), (1, Progress::Waiting), (2, Progress::Waiting), (3, Progress::Waiting)]);
    assert_eq!(
        &log[4..],
        &[
            (0, Progress::Uninstalling),
            (0, Progress::Done(Outcome::Uninstalled { freed: Some(100_000 * 1024) })),
            (1, Progress::Uninstalling),
            (1, Progress::Failed),
            (2, Progress::Uninstalling),
            (2, Progress::Done(Outcome::NotUninstalled { exit_code: Some(1602) })),
            (3, Progress::Uninstalling),
            (3, Progress::Done(Outcome::Uninstalled { freed: None })),
        ]
    );
    // One after another, never in parallel, in the picked order.
    assert_eq!(
        apps.os().state().ran,
        vec![format!("run {dc_cmd}"), format!("run {vg_cmd}"), "remove SpotifyAB.SpotifyMusic_1.0.0.0_x64__abc".to_string()]
    );
}

#[test]
fn access_denied_is_needs_admin() {
    struct Deny(FakeOs);
    impl AppsOs for Deny {
        fn uninstall_entries(&self) -> Result<Vec<os::RawEntry>, OsError> {
            self.0.uninstall_entries()
        }
        fn store_packages(&self) -> Result<Vec<RawPackage>, OsError> {
            self.0.store_packages()
        }
        fn entry_exists(&self, h: Hive, v: RegView, k: &str) -> bool {
            self.0.entry_exists(h, v, k)
        }
        fn package_installed(&self, f: &str) -> bool {
            self.0.package_installed(f)
        }
        fn run_uninstaller(&self, _: &str) -> Result<u32, OsError> {
            Err(OsError::AccessDenied)
        }
        fn remove_package(&self, _: &str) -> Result<(), OsError> {
            Err(OsError::AccessDenied)
        }
        fn folder_size(&self, p: &std::path::Path) -> Option<u64> {
            self.0.folder_size(p)
        }
        fn expand_env(&self, s: &str) -> String {
            self.0.expand_env(s)
        }
        fn run_setup(&self, _: &str) -> Result<u32, OsError> {
            Err(OsError::AccessDenied)
        }
        fn is_dir(&self, p: &std::path::Path) -> bool {
            self.0.is_dir(p)
        }
        fn open_folder(&self, p: &std::path::Path) -> Result<(), OsError> {
            self.0.open_folder(p)
        }
        fn open_settings(&self, u: &str) -> Result<(), OsError> {
            self.0.open_settings(u)
        }
    }
    let apps = Apps::new(Deny(pc()));
    let l = apps.list();
    assert_eq!(apps.uninstall(by_name(&l, "VALORANT")), Err(AppsError::NeedsAdmin));
}

#[test]
fn a_failing_source_keeps_the_rest() {
    let f = pc();
    f.state().fail_packages = Some(OsError::AccessDenied);
    let l = Apps::new(f).list();
    assert_eq!(l.problems, vec![("Store apps".to_string(), OsError::AccessDenied)]);
    assert!(l.apps.iter().all(|a| a.kind == AppKind::Desktop));
    assert!(!l.apps.is_empty());
}

#[test]
fn icon_locations() {
    assert_eq!(parse_icon_location(r#""C:\a b\x.exe",0"#), Some((r"C:\a b\x.exe".into(), 0)));
    assert_eq!(parse_icon_location(r"C:\x\y.dll,-12"), Some((r"C:\x\y.dll".into(), -12)));
    assert_eq!(parse_icon_location(r"C:\x\y.ico"), Some((r"C:\x\y.ico".into(), 0)));
    assert_eq!(parse_icon_location(r"C:\a,b\y.ico"), Some((r"C:\a,b\y.ico".into(), 0)), "a comma in the path is not an index");
    assert_eq!(parse_icon_location("  "), None);
    assert_eq!(parse_icon_location(r#""",0"#), None);
}

#[test]
fn desktop_size_measured_on_demand() {
    let apps = Apps::new(pc().folder(r"C:\Riot Games\VALORANT", 40 * 1024 * 1024 * 1024));
    let l = apps.list();
    let v = apps.measure_size(by_name(&l, "VALORANT"));
    assert_eq!(v.size_bytes, Some(40 * 1024 * 1024 * 1024), "the folder's real size replaces the estimate");
    assert_eq!(v.size_source, SizeSource::Measured);
}

#[test]
fn denied_store_removal_is_needs_admin() {
    let f = pc();
    f.state().package_errors.insert("SpotifyAB.SpotifyMusic_1.0.0.0_x64__abc".into(), OsError::AccessDenied);
    let apps = Apps::new(f);
    let l = apps.list();
    assert_eq!(apps.uninstall(by_name(&l, "Spotify")), Err(AppsError::NeedsAdmin));
}


// ------------------------------------------------------------------ Order 023: the v22 wrench (Modify / Repair) and Open install folder

fn fix_pc() -> FakeOs {
    FakeOs::new()
        // an MSI app that allows both
        .entry(HKLM, B64, MSI, &[("DisplayName", s("Both MSI")), ("WindowsInstaller", d(1)), ("UninstallString", s(&format!("MsiExec.exe /I{MSI}")))], None)
        // an MSI app that says NoModify (Epic Games Launcher in the drawing: Repair only)
        .entry(HKLM, B64, "{AAAAAAAA-1234-1234-1234-1234567890AB}", &[("DisplayName", s("Repair only")), ("WindowsInstaller", d(1)), ("NoModify", d(1)),
            ("UninstallString", s("MsiExec.exe /I{AAAAAAAA-1234-1234-1234-1234567890AB}"))], None)
        // a plain setup with its own ModifyPath and NoRepair (Chrome in the drawing: Modify only)
        .entry(HKLM, B64, "Chrome", &[("DisplayName", s("Modify only")), ("NoRepair", d(1)), ("ModifyPath", s(r#""C:\Chrome\setup.exe" --modify"#)),
            ("UninstallString", s(r#""C:\Chrome\setup.exe" --uninstall"#)), ("InstallLocation", s(r"C:\Chrome"))], None)
        // a WiX bundle (VC++ redistributable in the drawing: Modify + Repair)
        .entry(HKLM, B32, "{BBBBBBBB-1234-1234-1234-1234567890AB}", &[("DisplayName", s("Bundle")), ("BundleCachePath", s(r"C:\ProgramData\Package Cache\{x}\vc_redist.x64.exe")),
            ("ModifyPath", s(r#""C:\ProgramData\Package Cache\{x}\vc_redist.x64.exe"  /modify"#)),
            ("UninstallString", s(r#""C:\ProgramData\Package Cache\{x}\vc_redist.x64.exe"  /uninstall"#))], None)
        .dir(r"C:\Apps\Plain")
        // a plain app: nothing to fix; its folder comes from its icon
        .entry(HKCU, B64, "Plain", &[("DisplayName", s("Plain")), ("UninstallString", s(r#""C:\Apps\Plain\unins000.exe""#)),
            ("DisplayIcon", s(r#""C:\Apps\Plain\plain.exe",0"#))], None)
        // an app whose only folder hint is Windows' own installer cache: no folder
        .entry(HKCU, B64, "Cached", &[("DisplayName", s("Cached")), ("UninstallString", s("x.exe")), ("DisplayIcon", s(r"C:\Windows\Installer\{z}\icon.ico"))], None)
        // a locked one with a ModifyPath: locked apps have no wrench
        .entry(HKLM, B32, "Microsoft Edge", &[("DisplayName", s("Microsoft Edge")), ("ModifyPath", s(r#""C:\Edge\setup.exe" --modify"#)),
            ("UninstallString", s(r#""C:\Edge\setup.exe" --uninstall"#))], None)
        .package(pkg("SpotifyAB.SpotifyMusic", Some("Spotify"), Signature::Store))
}

#[test]
fn wrench_offers_what_the_entry_allows() {
    let apps = Apps::new(fix_pc());
    let l = apps.list();
    assert_eq!(by_name(&l, "Both MSI").fixes(), vec![Fix::Modify, Fix::Repair]);
    assert_eq!(by_name(&l, "Both MSI").modify_command.as_deref(), Some(format!("MsiExec.exe /I{MSI}").as_str()));
    assert_eq!(by_name(&l, "Both MSI").repair_command.as_deref(), Some(format!("MsiExec.exe /f{MSI}").as_str()));
    assert_eq!(by_name(&l, "Repair only").fixes(), vec![Fix::Repair]);
    assert_eq!(by_name(&l, "Modify only").fixes(), vec![Fix::Modify]);
    assert_eq!(by_name(&l, "Bundle").fixes(), vec![Fix::Modify, Fix::Repair]);
    assert_eq!(by_name(&l, "Bundle").repair_command.as_deref(), Some(r#""C:\ProgramData\Package Cache\{x}\vc_redist.x64.exe" /repair"#));
    assert!(by_name(&l, "Plain").fixes().is_empty());
    assert!(by_name(&l, "Microsoft Edge").fixes().is_empty(), "locked apps have no wrench");
    assert_eq!(by_name(&l, "Spotify").fixes(), vec![Fix::Repair, Fix::Reset]);
    assert_eq!(Fix::Modify.label(), "Modify");
}

#[test]
fn wrench_runs_the_apps_own_setup_in_the_fake_only() {
    let apps = Apps::new(fix_pc().script(r#""C:\Chrome\setup.exe" --modify"#, 0, false));
    let l = apps.list();
    assert_eq!(apps.fix(by_name(&l, "Modify only"), Fix::Modify), Ok(FixOutcome::SetupEnded { exit_code: 0 }));
    assert_eq!(apps.fix(by_name(&l, "Both MSI"), Fix::Repair), Ok(FixOutcome::SetupEnded { exit_code: 0 }));
    assert_eq!(apps.fix(by_name(&l, "Modify only"), Fix::Repair), Err(AppsError::NotOffered(Fix::Repair)));
    assert!(matches!(apps.fix(by_name(&l, "Microsoft Edge"), Fix::Modify), Err(AppsError::Locked(_))));
    assert_eq!(apps.fix(by_name(&l, "Spotify"), Fix::Reset), Ok(FixOutcome::OpenedSettings));
    let ran = apps.os().state().ran.clone();
    assert_eq!(
        ran,
        vec![
            r#"setup "C:\Chrome\setup.exe" --modify"#.to_string(),
            format!("setup MsiExec.exe /f{MSI}"),
            "settings ms-settings:appsfeatures-app?SpotifyAB.SpotifyMusic_abc".to_string(),
        ]
    );
    // the app is still listed: a wrench never uninstalls
    assert_eq!(apps.list().apps.len(), l.apps.len());
}

#[test]
fn wrench_cancelled_at_the_admin_prompt_and_access_denied() {
    let apps = Apps::new(fix_pc().script_prompt_cancelled(&format!("MsiExec.exe /f{MSI}")));
    let l = apps.list();
    assert_eq!(apps.fix(by_name(&l, "Both MSI"), Fix::Repair), Err(AppsError::Os(OsError::Cancelled)));
}

#[test]
fn open_install_folder() {
    let apps = Apps::new(fix_pc());
    let l = apps.list();
    assert_eq!(by_name(&l, "Modify only").folder(), Some(std::path::PathBuf::from(r"C:\Chrome")));
    assert_eq!(by_name(&l, "Plain").folder(), Some(std::path::PathBuf::from(r"C:\Apps\Plain")));
    assert_eq!(by_name(&l, "Cached").folder(), None, "Windows' own folders are never offered");
    assert_eq!(by_name(&l, "Spotify").folder(), None, "Store apps have no folder icon (v22)");
    assert_eq!(apps.open_folder(by_name(&l, "Plain")), Ok(()));
    assert_eq!(apps.open_folder(by_name(&l, "Cached")), Err(AppsError::NoFolder));
    assert_eq!(apps.os().state().ran, vec![r"explore C:\Apps\Plain".to_string()]);
}

/// REVIEW_023 HOLD 2: an InstallLocation that names a FILE (some installers write the exe there) is never opened - a
/// ShellExecute "open" on it would run the program.
#[test]
fn open_install_folder_never_starts_a_file() {
    let os = FakeOs::new()
        .entry(HKLM, B64, "ExeLoc", &[("DisplayName", s("Exe as location")), ("UninstallString", s("u.exe")),
            ("InstallLocation", s(r"C:\Apps\Tool\tool.exe"))], None)
        .dir(r"C:\Apps\Tool");
    let apps = Apps::new(os);
    let l = apps.list();
    let a = by_name(&l, "Exe as location");
    assert_eq!(a.folder(), Some(std::path::PathBuf::from(r"C:\Apps\Tool\tool.exe")));
    assert_eq!(apps.open_folder(a), Err(AppsError::NoFolder));
    assert!(apps.os().state().ran.is_empty(), "nothing was opened or run");
    // the fake's OS layer itself refuses a non-folder and a non-settings URI
    assert_eq!(apps.os().open_folder(std::path::Path::new(r"C:\Apps\Tool\tool.exe")), Err(OsError::NotFound));
    assert_eq!(apps.os().open_settings(r"C:\Apps\Tool\tool.exe"), Err(OsError::NotFound));
}
