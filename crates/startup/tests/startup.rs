//! Every §3.7 row against the fake OS layer. Nothing real is touched.

use bu_startup::fake::FakeOs;
use bu_startup::os::{RawService, RawTask, StoreStartupTask};
use bu_startup::*;

const HKCU: Hive = Hive::CurrentUser;
const HKLM: Hive = Hive::LocalMachine;
const B64: RegView = RegView::Bits64;
const B32: RegView = RegView::Bits32;
const APPROVED_RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const APPROVED_RUN32: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run32";
const APPROVED_FOLDER: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder";
const FT: u64 = 0x01DB_05E7_1B17_890A;

fn task(path: &str, enabled: bool, cmd: &str) -> RawTask {
    RawTask {
        path: path.into(),
        name: path.rsplit('\\').next().unwrap().into(),
        enabled,
        triggers: vec![TaskTrigger::Logon],
        command: Some(cmd.into()),
        arguments: None,
        author: Some("Author Co".into()),
    }
}

fn service(name: &str, start: ServiceStart, delayed: bool, image: &str) -> RawService {
    RawService { name: name.into(), display_name: format!("{name} display"), start, delayed, image_path: Some(image.into()) }
}

/// A PC with one of everything.
fn pc() -> FakeOs {
    let mut f = FakeOs::new()
        .env("windir", r"C:\Windows")
        .file(r"C:\Program Files (x86)\Steam\steam.exe", "Valve Corporation", "Steam")
        .file(r"C:\Riot Games\Riot Client\RiotClientServices.exe", "Riot Games, Inc.", "Riot Client")
        .file(r"C:\Windows\system32\SecurityHealthSystray.exe", "Microsoft Corporation", "Windows Security notification icon")
        .file(r"C:\Program Files (x86)\Old\old32.exe", "Old Co", "Old 32-bit app")
        .file(r"C:\Program Files\OBS\obs64.exe", "OBS", "OBS Studio")
        .file(r"C:\Program Files\Updater\upd.exe", "Upd Co", "Updater")
        .file(r"C:\Windows\System32\svchost.exe", "Microsoft Corporation", "Host Process")
        .file(r"C:\Program Files\Vendor\vendorsvc.exe", "Vendor Inc.", "Vendor service")
        .file(r"C:\Program Files\Riot Vanguard\vgc.exe", "Riot Games, Inc.", "Vanguard")
        .file(r"C:\Program Files\Delayed\d.exe", "Delayed Inc.", "Delayed service")
        .file(r"C:\Users\u\AppData\Local\Discord\Update.exe", "GitHub", "Update")
        .file(r"C:\Users\u\AppData\Local\Discord\app-1.0.9\Discord.exe", "Discord Inc.", "Discord")
        .file(r"C:\Users\u\AppData\Local\Discord\app-1.0.10\Discord.exe", "Discord Inc.", "Discord")
        // Run keys
        .run(HKCU, B64, RUN, "Steam", r#""C:\Program Files (x86)\Steam\steam.exe" -silent"#)
        .run(HKCU, B32, RUN, "Steam", r#""C:\Program Files (x86)\Steam\steam.exe" -silent"#) // same key seen through the 32-bit view
        .run(HKCU, B64, RUN, "RiotClient", r"C:\Riot Games\Riot Client\RiotClientServices.exe --launch-background-mode")
        .run(HKCU, B64, RUN, "Discord", r#""C:\Users\u\AppData\Local\Discord\Update.exe" --processStart Discord.exe"#)
        .run(HKLM, B64, RUN, "SecurityHealth", r"%windir%\system32\SecurityHealthSystray.exe")
        .run(HKLM, B32, RUN, "Old32", r#""C:\Program Files (x86)\Old\old32.exe""#)
        .run(HKCU, B64, RUN_ONCE, "FinishSetup", r#""C:\Program Files\Updater\upd.exe" /finish"#)
        // Steam is off (Task Manager wrote 03 + a time), Riot on (02), Discord has no flag (= on), HKLM 32 off with 07.
        .binary(HKCU, APPROVED_RUN, "Steam", &[3, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8])
        .binary(HKCU, APPROVED_RUN, "RiotClient", &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
        .binary(HKLM, APPROVED_RUN32, "Old32", &[7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
        // Startup folders
        .folder_item(false, r"C:\Users\u\Startup", "obs64.exe.lnk", Some(r"C:\Program Files\OBS\obs64.exe"), Some("--minimize-to-tray"))
        .folder_item(false, r"C:\Users\u\Startup", "desktop.ini", None, None)
        .folder_item(true, r"C:\ProgramData\Startup", "Updater.lnk", Some(r"C:\Program Files\Updater\upd.exe"), None)
        .binary(HKLM, APPROVED_FOLDER, "Updater.lnk", &[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
        // Store app startup tasks
        .store(StoreStartupTask {
            package_family: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0".into(),
            task_id: "Spotify".into(),
            state: 1,
            display_name: Some("Spotify".into()),
            publisher: Some("Spotify AB".into()),
            logo: Some(r"C:\Program Files\WindowsApps\Spotify\Logo.png".into()),
        })
        .store(StoreStartupTask {
            package_family: "Microsoft.WindowsTerminal_8wekyb3d8bbwe".into(),
            task_id: "StartTerminalOnLoginTask".into(),
            state: 2,
            display_name: None,
            publisher: Some("Microsoft Corporation".into()),
            logo: None,
        })
        // Tasks
        .task(task(r"\Microsoft\Windows\Foo\WinTask", true, r"%windir%\system32\SecurityHealthSystray.exe"))
        .task(task(r"\Vendor Updater", true, r"C:\Program Files\Updater\upd.exe"))
        .task(task(r"\Locked Task", true, r"C:\Program Files\Updater\upd.exe"))
        // Services
        .service(service("VendorSvc", ServiceStart::Automatic, false, r#""C:\Program Files\Vendor\vendorsvc.exe""#))
        .service(service("DelayedSvc", ServiceStart::Automatic, true, r"C:\Program Files\Delayed\d.exe -run"))
        .service(service("Dnscache", ServiceStart::Automatic, false, r"C:\Windows\System32\svchost.exe -k NetworkService -p"))
        .service(service("vgc", ServiceStart::Automatic, false, r#""C:\Program Files\Riot Vanguard\vgc.exe""#))
        .service(service("ManualSvc", ServiceStart::Manual, false, r"C:\Program Files\Vendor\vendorsvc.exe"));
    f = f.impact(Ok(vec![report()]));
    f.state().now = FT;
    f.state().refused.insert(r"\Locked Task".into());
    f
}

fn report() -> String {
    r#"<StartupData><Startup>
<Process Name="C:\Program Files (x86)\Steam\steam.exe" PID="1" StartedInTraceSec="1"><DiskUsage Units="bytes">5000000</DiskUsage><CpuUsage Units="us">10</CpuUsage></Process>
<Process Name="C:\Riot Games\Riot Client\RiotClientServices.exe" PID="2" StartedInTraceSec="1"><DiskUsage Units="bytes">400000</DiskUsage><CpuUsage Units="us">10</CpuUsage></Process>
<Process Name="C:\Windows\system32\SecurityHealthSystray.exe" PID="3" StartedInTraceSec="1"><DiskUsage Units="bytes">1000</DiskUsage><CpuUsage Units="us">1000</CpuUsage></Process>
</Startup></StartupData>"#
        .into()
}

fn find<'a>(l: &'a StartupList, id: &str) -> &'a StartupEntry {
    l.get(id).unwrap_or_else(|| panic!("no entry {id}; have {:?}", l.entries.iter().map(|e| &e.id).collect::<Vec<_>>()))
}

// ---------- reading ----------

#[test]
fn run_keys_all_views_and_runonce() {
    let s = Startup::new(pc());
    let l = s.list();
    // HKCU Steam seen through both views is listed once.
    assert_eq!(l.entries.iter().filter(|e| e.key_name == "Steam").count(), 1);

    let steam = find(&l, r"run|HKCU|Bits64|Run|Steam");
    assert_eq!(steam.name, "Steam");
    assert_eq!(steam.publisher.as_deref(), Some("Valve Corporation"));
    assert_eq!(steam.path.as_deref(), Some(std::path::Path::new(r"C:\Program Files (x86)\Steam\steam.exe")));
    assert_eq!(steam.icon_path, steam.path);
    assert_eq!(steam.location, r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run");
    assert!(!steam.enabled, "03 = off");
    assert_eq!(steam.kind, Kind::Normal);
    assert_eq!(steam.switch, Switch::Free);
    assert_eq!(steam.approved.as_ref().unwrap().key, ApprovedKey::Run);

    let riot = find(&l, r"run|HKCU|Bits64|Run|RiotClient");
    assert!(riot.enabled, "02 = on");
    assert_eq!(riot.name, "Riot Client", "unquoted path with spaces found");

    let sec = find(&l, r"run|HKLM|Bits64|Run|SecurityHealth");
    assert!(sec.enabled, "no flag = on");
    assert!(sec.windows_own);
    assert!(sec.needs_admin());
    assert_eq!(sec.path.as_deref(), Some(std::path::Path::new(r"C:\Windows\system32\SecurityHealthSystray.exe")), "%windir% expanded");

    let old = find(&l, r"run|HKLM|Bits32|Run|Old32");
    assert!(!old.enabled, "07 = off (odd byte)");
    assert_eq!(old.approved.as_ref().unwrap().key, ApprovedKey::Run32);
    assert_eq!(old.location, r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run");

    let once = find(&l, r"run|HKCU|Bits64|RunOnce|FinishSetup");
    assert_eq!(once.switch, Switch::Locked(LockReason::RunOnce));
    assert!(once.approved.is_none());
    assert!(once.enabled);
}

#[test]
fn squirrel_launcher_resolves_to_the_newest_app() {
    let l = Startup::new(pc()).list();
    let d = find(&l, r"run|HKCU|Bits64|Run|Discord");
    assert_eq!(d.name, "Discord");
    assert_eq!(d.publisher.as_deref(), Some("Discord Inc."), "not Update.exe's 'GitHub'");
    assert_eq!(
        d.path.as_deref(),
        Some(std::path::Path::new(r"c:\users\u\appdata\local\discord\app-1.0.10\Discord.exe")),
        "1.0.10 is newer than 1.0.9"
    );
    assert!(d.command.contains("Update.exe"), "the stored command is shown as is");
}

#[test]
fn startup_folders() {
    let l = Startup::new(pc()).list();
    assert!(l.entries.iter().all(|e| e.key_name != "desktop.ini"));
    let obs = find(&l, "folder|user|obs64.exe.lnk");
    assert_eq!(obs.name, "obs64.exe");
    assert_eq!(obs.publisher.as_deref(), Some("OBS"));
    assert_eq!(obs.command, r#""C:\Program Files\OBS\obs64.exe" --minimize-to-tray"#);
    assert_eq!(obs.location, r"C:\Users\u\Startup");
    assert!(obs.enabled);
    assert_eq!(obs.switch, Switch::Free);
    let upd = find(&l, "folder|all|Updater.lnk");
    assert!(!upd.enabled);
    assert_eq!(upd.switch, Switch::NeedsAdmin);
    assert_eq!(upd.approved.as_ref().unwrap().hive, HKLM);
}

#[test]
fn store_apps_go_through_settings() {
    let s = Startup::new(pc().admin(true));
    let l = s.list();
    let sp = find(&l, "store|SpotifyAB.SpotifyMusic_zpdnekdrzrea0|Spotify");
    assert!(!sp.enabled, "1 = DisabledByUser");
    assert_eq!(sp.publisher.as_deref(), Some("Spotify AB"));
    assert_eq!(sp.icon_path.as_deref(), Some(std::path::Path::new(r"C:\Program Files\WindowsApps\Spotify\Logo.png")));
    assert_eq!(sp.switch, Switch::Settings("ms-settings:startupapps"));
    let term = find(&l, "store|Microsoft.WindowsTerminal_8wekyb3d8bbwe|StartTerminalOnLoginTask");
    assert!(term.enabled, "2 = Enabled");
    assert_eq!(term.name, "StartTerminalOnLoginTask", "no display name → the task id");
    assert_eq!(s.set_enabled(sp, true), Err(StartupError::UseSettings("ms-settings:startupapps")));
}

#[test]
fn tasks_and_services_are_hidden_rows_in_order() {
    let l = Startup::new(pc()).list();
    let kinds: Vec<Kind> = l.entries.iter().map(|e| e.kind).collect();
    let mut sorted = kinds.clone();
    sorted.sort();
    assert_eq!(kinds, sorted, "normal rows, then tasks, then services");

    let tasks: Vec<&str> = l.entries.iter().filter(|e| e.kind == Kind::HiddenTask).map(|e| e.key_name.as_str()).collect();
    assert_eq!(tasks, vec![r"\Locked Task", r"\Vendor Updater", r"\Microsoft\Windows\Foo\WinTask"], "third-party first");
    let wt = find(&l, r"task|\Microsoft\Windows\Foo\WinTask");
    assert!(wt.windows_own);
    assert_eq!(wt.switch, Switch::Locked(LockReason::WindowsOwnTask), "default policy (A_006_01): Windows' own tasks locked");
    let vt = find(&l, r"task|\Vendor Updater");
    assert_eq!(vt.publisher.as_deref(), Some("Upd Co"));
    assert_eq!(vt.location, r"Task Scheduler \Vendor Updater");

    let svcs: Vec<&str> = l.entries.iter().filter(|e| e.kind == Kind::HiddenService).map(|e| e.key_name.as_str()).collect();
    assert_eq!(svcs, vec!["DelayedSvc", "VendorSvc", "vgc", "Dnscache"], "Automatic only, third-party first, Windows' own last");
    assert_eq!(find(&l, "service|vgc").switch, Switch::Locked(LockReason::AntiCheat));
    assert_eq!(find(&l, "service|Dnscache").switch, Switch::Locked(LockReason::WindowsOwnService));
    assert!(find(&l, "service|Dnscache").windows_own);
    assert_eq!(find(&l, "service|VendorSvc").switch, Switch::NeedsAdmin);
    assert_eq!(find(&l, "service|VendorSvc").name, "VendorSvc display");
}

#[test]
fn views_counts_and_unique_ids() {
    let l = Startup::new(pc()).list();
    let (on, shown) = l.counts(View::All);
    assert_eq!(shown, l.entries.len());
    assert_eq!(on, l.entries.iter().filter(|e| e.enabled).count());
    assert_eq!(l.counts(View::Normal).1 + l.counts(View::Hidden).1, shown);
    assert!(l.shown(View::Hidden).iter().all(|e| e.kind != Kind::Normal));
    assert!(l.shown(View::Normal).iter().all(|e| e.kind == Kind::Normal));
    let mut ids: Vec<&String> = l.entries.iter().map(|e| &e.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), l.entries.len());
}

#[test]
fn impact_from_windows_boot_report() {
    let l = Startup::new(pc()).list();
    assert_eq!(l.impact_source, ImpactSource::Report);
    assert!(matches!(find(&l, r"run|HKCU|Bits64|Run|Steam").impact, ImpactState::Measured(Impact::High, _)));
    assert!(matches!(find(&l, r"run|HKCU|Bits64|Run|RiotClient").impact, ImpactState::Measured(Impact::Medium, _)));
    assert!(matches!(find(&l, r"run|HKLM|Bits64|Run|SecurityHealth").impact, ImpactState::Measured(Impact::Low, _)));
    assert_eq!(find(&l, "folder|user|obs64.exe.lnk").impact, ImpactState::NotMeasured);
}

#[test]
fn impact_unknown_without_admin_or_report() {
    let f = pc().impact(Err(OsError::AccessDenied));
    let l = Startup::new(f).list();
    assert_eq!(l.impact_source, ImpactSource::NeedsAdmin);
    assert!(l.entries.iter().all(|e| e.impact == ImpactState::Unknown));

    let l = Startup::new(pc().impact(Ok(vec![]))).list();
    assert_eq!(l.impact_source, ImpactSource::NoReport);
    assert!(l.entries.iter().all(|e| e.impact == ImpactState::Unknown));
}

#[test]
fn a_failing_source_keeps_the_rest() {
    let f = pc();
    f.state().fail_read.insert("tasks", OsError::AccessDenied);
    let l = Startup::new(f).list();
    assert_eq!(l.problems, vec![("Scheduled tasks".to_string(), OsError::AccessDenied)]);
    assert!(l.entries.iter().all(|e| e.kind != Kind::HiddenTask));
    assert!(l.entries.iter().any(|e| e.kind == Kind::HiddenService));
}

// ---------- switching + undo ----------

#[test]
fn hkcu_flag_off_on_like_task_manager_and_undo() {
    let s = Startup::new(pc());
    let l = s.list();
    let riot = find(&l, r"run|HKCU|Bits64|Run|RiotClient").clone();

    let off = s.set_enabled(&riot, false).unwrap();
    let mut want = vec![3, 0, 0, 0];
    want.extend_from_slice(&FT.to_le_bytes());
    assert_eq!(s.os().get_binary(HKCU, APPROVED_RUN, "RiotClient"), Some(want), "03 00 00 00 + FILETIME");
    assert!(!find(&s.list(), &riot.id).enabled);
    assert_eq!(off.undo, Undo::Flag { slot: riot.approved.clone().unwrap(), old: Some(vec![2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]) });

    s.undo(&off).unwrap();
    assert_eq!(s.os().get_binary(HKCU, APPROVED_RUN, "RiotClient"), Some(vec![2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), "old bytes back exactly");

    let steam = find(&l, r"run|HKCU|Bits64|Run|Steam").clone();
    s.set_enabled(&steam, true).unwrap();
    assert_eq!(s.os().get_binary(HKCU, APPROVED_RUN, "Steam"), Some(vec![2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), "02 + zeros");
    assert!(find(&s.list(), &steam.id).enabled);
}

#[test]
fn undo_of_a_flag_that_was_missing_removes_it_again() {
    let s = Startup::new(pc());
    let discord = find(&s.list(), r"run|HKCU|Bits64|Run|Discord").clone();
    let c = s.set_enabled(&discord, false).unwrap();
    assert!(s.os().get_binary(HKCU, APPROVED_RUN, "Discord").is_some());
    s.undo(&c).unwrap();
    assert_eq!(s.os().get_binary(HKCU, APPROVED_RUN, "Discord"), None, "back to no value, exactly like before");
    // The Run value itself was never touched (never delete).
    assert!(s.os().state().writes.iter().all(|w| !w.contains(r"CurrentVersion\Run\")));
}

#[test]
fn hklm_and_all_users_need_admin() {
    let s = Startup::new(pc());
    let l = s.list();
    let sec = find(&l, r"run|HKLM|Bits64|Run|SecurityHealth").clone();
    assert_eq!(s.set_enabled(&sec, false), Err(StartupError::NeedsAdmin));
    let upd = find(&l, "folder|all|Updater.lnk").clone();
    assert_eq!(s.set_enabled(&upd, true), Err(StartupError::NeedsAdmin));
    assert!(s.os().state().writes.is_empty(), "nothing written without admin");

    let s = Startup::new(pc().admin(true));
    let c = s.set_enabled(&upd, true).unwrap();
    assert!(find(&s.list(), "folder|all|Updater.lnk").enabled);
    let old = find(&s.list(), r"run|HKLM|Bits32|Run|Old32").clone();
    s.set_enabled(&old, true).unwrap();
    assert_eq!(s.os().get_binary(HKLM, APPROVED_RUN32, "Old32"), Some(vec![2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), "32-bit HKLM → Run32");
    s.undo(&c).unwrap();
    assert!(!find(&s.list(), "folder|all|Updater.lnk").enabled);
}

#[test]
fn hkcu_startup_folder_flag() {
    let s = Startup::new(pc());
    let obs = find(&s.list(), "folder|user|obs64.exe.lnk").clone();
    let c = s.set_enabled(&obs, false).unwrap();
    assert_eq!(s.os().get_binary(HKCU, APPROVED_FOLDER, "obs64.exe.lnk").map(|v| v[0]), Some(3));
    s.undo(&c).unwrap();
    assert_eq!(s.os().get_binary(HKCU, APPROVED_FOLDER, "obs64.exe.lnk"), None);
}

#[test]
fn a_silently_blocked_write_is_caught_by_the_read_back() {
    let f = pc();
    f.state().ignored.insert("RiotClient".into());
    let s = Startup::new(f);
    let riot = find(&s.list(), r"run|HKCU|Bits64|Run|RiotClient").clone();
    assert_eq!(s.set_enabled(&riot, false), Err(StartupError::WriteBlocked));
}

#[test]
fn runonce_is_locked() {
    let s = Startup::new(pc().admin(true));
    let once = find(&s.list(), r"run|HKCU|Bits64|RunOnce|FinishSetup").clone();
    assert_eq!(s.set_enabled(&once, false), Err(StartupError::Locked(LockReason::RunOnce)));
}

#[test]
fn tasks_disable_enable_undo_and_admin() {
    let s = Startup::new(pc());
    let vt = find(&s.list(), r"task|\Vendor Updater").clone();
    assert_eq!(s.set_enabled(&vt, false), Err(StartupError::NeedsAdmin));

    let s = Startup::new(pc().admin(true));
    let c = s.set_enabled(&vt, false).unwrap();
    let after = find(&s.list(), r"task|\Vendor Updater").clone();
    assert!(!after.enabled, "disabled, still listed (never deleted)");
    assert_eq!(c.undo, Undo::Task { path: r"\Vendor Updater".into(), was_enabled: true });
    s.undo(&c).unwrap();
    assert!(find(&s.list(), r"task|\Vendor Updater").enabled);
    // Windows' own task: locked under the default policy, nothing written.
    let wt = find(&s.list(), r"task|\Microsoft\Windows\Foo\WinTask").clone();
    assert_eq!(s.set_enabled(&wt, false), Err(StartupError::Locked(LockReason::WindowsOwnTask)));
    assert!(find(&s.list(), &wt.id).enabled);
}

#[test]
fn a_task_windows_locks_even_for_admin() {
    let s = Startup::new(pc().admin(true));
    let lt = find(&s.list(), r"task|\Locked Task").clone();
    assert_eq!(s.set_enabled(&lt, false), Err(StartupError::Refused));
    assert!(find(&s.list(), &lt.id).enabled);
}

#[test]
fn services_automatic_to_manual_stays_listed_and_back() {
    let s = Startup::new(pc().admin(true));
    let v = find(&s.list(), "service|VendorSvc").clone();
    let c = s.set_enabled(&v, false).unwrap();
    assert_eq!(s.os().service_start("VendorSvc").unwrap(), (ServiceStart::Manual, false));
    let after = find(&s.list(), "service|VendorSvc").clone();
    assert!(!after.enabled, "Manual = off, and the row stays listed");
    s.undo(&c).unwrap();
    assert_eq!(s.os().service_start("VendorSvc").unwrap(), (ServiceStart::Automatic, false));
    assert!(s.os().remembered_services().unwrap().is_empty(), "memory restored too");
}

#[test]
fn delayed_start_comes_back_exactly() {
    let s = Startup::new(pc().admin(true));
    let d = find(&s.list(), "service|DelayedSvc").clone();
    s.set_enabled(&d, false).unwrap();
    assert_eq!(s.os().remembered_services().unwrap(), vec![("DelayedSvc".to_string(), true)]);
    let off = find(&s.list(), "service|DelayedSvc").clone();
    s.set_enabled(&off, true).unwrap();
    assert_eq!(s.os().service_start("DelayedSvc").unwrap(), (ServiceStart::Automatic, true), "Automatic (Delayed Start) again");
    assert!(s.os().remembered_services().unwrap().is_empty());
    // A service that was never Automatic is not listed.
    assert!(s.list().get("service|ManualSvc").is_none());
}

#[test]
fn services_need_admin_and_locks_hold() {
    let s = Startup::new(pc());
    let v = find(&s.list(), "service|VendorSvc").clone();
    assert_eq!(s.set_enabled(&v, false), Err(StartupError::NeedsAdmin));

    let s = Startup::new(pc().admin(true));
    let l = s.list();
    assert_eq!(s.set_enabled(find(&l, "service|vgc"), false), Err(StartupError::Locked(LockReason::AntiCheat)));
    assert_eq!(s.set_enabled(find(&l, "service|Dnscache"), false), Err(StartupError::Locked(LockReason::WindowsOwnService)));
    assert!(s.os().state().writes.is_empty());
}

#[test]
fn policy_can_open_windows_services_but_never_anti_cheat() {
    let policy = Policy { windows_services_switchable: true, windows_tasks_switchable: false };
    let s = Startup::with_policy(pc().admin(true), policy);
    let l = s.list();
    assert_eq!(find(&l, "service|Dnscache").switch, Switch::NeedsAdmin);
    assert_eq!(find(&l, "service|vgc").switch, Switch::Locked(LockReason::AntiCheat));
    assert_eq!(find(&l, r"task|\Microsoft\Windows\Foo\WinTask").switch, Switch::Locked(LockReason::WindowsOwnTask));
    let c = s.set_enabled(find(&l, "service|Dnscache"), false).unwrap();
    s.undo(&c).unwrap();
    assert_eq!(s.os().service_start("Dnscache").unwrap(), (ServiceStart::Automatic, false));

    // Opening Windows' own tasks too.
    let all = Policy { windows_services_switchable: true, windows_tasks_switchable: true };
    let s = Startup::with_policy(pc().admin(true), all);
    let wt = find(&s.list(), r"task|\Microsoft\Windows\Foo\WinTask").clone();
    assert_eq!(wt.switch, Switch::NeedsAdmin);
    let c = s.set_enabled(&wt, false).unwrap();
    assert!(!find(&s.list(), &wt.id).enabled);
    s.undo(&c).unwrap();
    assert!(find(&s.list(), &wt.id).enabled);
}

#[test]
fn undo_needs_admin_where_the_change_did() {
    let f = pc().admin(true);
    let s = Startup::new(f);
    let v = find(&s.list(), "service|VendorSvc").clone();
    let c = s.set_enabled(&v, false).unwrap();
    s.os().state().admin = false;
    assert_eq!(s.undo(&c), Err(StartupError::NeedsAdmin));
    s.os().state().admin = true;
    s.undo(&c).unwrap();
}

#[test]
fn a_vanished_entry_is_gone() {
    let s = Startup::new(pc().admin(true));
    let v = find(&s.list(), "service|VendorSvc").clone();
    s.os().state().services.retain(|x| x.name != "VendorSvc");
    assert_eq!(s.set_enabled(&v, false), Err(StartupError::Gone));
}

#[test]
fn approved_byte_rules() {
    assert!(approved_enabled(None));
    assert!(approved_enabled(Some(&[2])));
    assert!(approved_enabled(Some(&[6])));
    assert!(!approved_enabled(Some(&[3])));
    assert!(!approved_enabled(Some(&[7])));
    assert!(approved_enabled(Some(&[])), "empty value treated as no flag");
    assert_eq!(approved_bytes(true, 99).len(), 12);
    assert_eq!(&approved_bytes(false, 0x0102030405060708)[4..], &[8, 7, 6, 5, 4, 3, 2, 1]);
    assert!(is_anti_cheat("EASYANTICHEAT"));
    assert!(!is_anti_cheat("Steam Client Service"));
}

#[test]
fn boot_trigger_tasks_are_listed_too() {
    let mut t = task(r"\Vendor Boot Helper", true, r"C:\Program Files\Updater\upd.exe");
    t.triggers = vec![TaskTrigger::Boot];
    let s = Startup::new(pc().task(t).admin(true));
    let e = find(&s.list(), r"task|\Vendor Boot Helper").clone();
    assert_eq!(e.kind, Kind::HiddenTask);
    assert_eq!(e.source, Source::Task { path: r"\Vendor Boot Helper".into(), triggers: vec![TaskTrigger::Boot] });
    let c = s.set_enabled(&e, false).unwrap();
    assert!(!find(&s.list(), &e.id).enabled);
    s.undo(&c).unwrap();
    assert!(find(&s.list(), &e.id).enabled);
}

#[test]
fn shortcut_icon_location_is_used() {
    let f = pc();
    f.state().user_folder[0].icon = Some(r#""%windir%\system32\shell32.dll",-21"#.into());
    let l = Startup::new(f).list();
    let obs = find(&l, "folder|user|obs64.exe.lnk");
    assert_eq!(obs.icon_path.as_deref(), Some(std::path::Path::new(r"C:\Windows\system32\shell32.dll")));
    assert_eq!(obs.icon_index, -21);
}

#[test]
fn windows_own_fails_closed_without_version_info() {
    let f = pc()
        .env("SystemRoot", r"C:\Windows")
        .file(r"C:\Windows\System32\mystery.exe", "", "")
        .file(r"C:\Program Files\NoInfo\noinfo.exe", "", "")
        .service(service("MysterySvc", ServiceStart::Automatic, false, r"C:\Windows\System32\mystery.exe"))
        .service(service("NoInfoSvc", ServiceStart::Automatic, false, r"C:\Program Files\NoInfo\noinfo.exe"));
    let l = Startup::new(f).list();
    let m = find(&l, "service|MysterySvc");
    assert!(m.windows_own, "no company name, under the Windows folder → Windows' own");
    assert_eq!(m.switch, Switch::Locked(LockReason::WindowsOwnService));
    let n = find(&l, "service|NoInfoSvc");
    assert!(!n.windows_own);
    assert_eq!(n.switch, Switch::NeedsAdmin);
}

#[test]
fn hklm_64_and_32_bit_entries_with_the_same_name_both_stay() {
    let f = pc()
        .run(HKLM, B64, RUN, "Twin", r"C:\Program Files\Updater\upd.exe")
        .run(HKLM, B32, RUN, "Twin", r"C:\Program Files\Updater\upd.exe");
    let l = Startup::new(f).list();
    assert!(l.get(r"run|HKLM|Bits64|Run|Twin").is_some());
    assert!(l.get(r"run|HKLM|Bits32|Run|Twin").is_some(), "different keys (Run vs Run32 flag) — not merged");
}

#[test]
fn fake_env_value_holding_its_own_name_does_not_loop() {
    let f = FakeOs::new().env("A", "%A%x");
    assert_eq!(f.expand_env("%A% and %a%"), "%A%x and %A%x");
}

/// Order 021 (the v21/v22 drawing): only PARTS OF WINDOWS get the Windows badge (and their tasks / services stay locked);
/// Microsoft apps outside the Windows folder (OneDrive, Edge's updater) are ordinary rows.
#[test]
fn microsoft_apps_are_not_windows_own_only_parts_of_windows_are() {
    let f = FakeOs::new()
        .env("windir", r"C:\Windows")
        .file(r"C:\Program Files\Microsoft OneDrive\OneDrive.exe", "Microsoft Corporation", "Microsoft OneDrive")
        .file(r"C:\Program Files (x86)\Microsoft\EdgeUpdate\MicrosoftEdgeUpdate.exe", "Microsoft Corporation", "Microsoft Edge Update")
        .file(r"C:\ProgramData\Microsoft\Windows Defender\Platform\4.18.25080.5-0\MsMpEng.exe", "Microsoft Corporation", "Antimalware Service Executable")
        .file(r"C:\Windows\system32\SecurityHealthSystray.exe", "Microsoft Corporation", "Windows Security notification icon")
        .run(HKCU, B64, RUN, "OneDrive", r#""C:\Program Files\Microsoft OneDrive\OneDrive.exe" /background"#)
        .run(HKLM, B64, RUN, "SecurityHealth", r"%windir%\system32\SecurityHealthSystray.exe")
        .task(task(r"\MicrosoftEdgeUpdateTaskMachineCore", true, r"C:\Program Files (x86)\Microsoft\EdgeUpdate\MicrosoftEdgeUpdate.exe"))
        .service(service("WinDefend", ServiceStart::Automatic, false, r#""C:\ProgramData\Microsoft\Windows Defender\Platform\4.18.25080.5-0\MsMpEng.exe""#));
    let s = Startup::new(f);
    let l = s.list();
    let od = l.entries.iter().find(|e| e.key_name == "OneDrive").unwrap();
    assert!(!od.windows_own && od.can_switch());
    let edge = find(&l, r"task|\MicrosoftEdgeUpdateTaskMachineCore");
    assert!(!edge.windows_own && edge.switch == Switch::NeedsAdmin, "{:?}", edge.switch);
    let sec = l.entries.iter().find(|e| e.key_name == "SecurityHealth").unwrap();
    assert!(sec.windows_own && sec.can_switch(), "a Run-key part of Windows: badge + warning, still switchable");
    let def = find(&l, "service|WinDefend");
    assert!(def.windows_own && def.switch == Switch::Locked(LockReason::WindowsOwnService));
}

// ---------- the app's change log (Order 036): target + state as text, read now, put back on a fresh service ----------

use bu_startup::saved::{State, Target};

#[test]
fn saved_flag_is_text_and_goes_back_on_a_fresh_service() {
    let s = Startup::new(pc());
    let riot = find(&s.list(), r"run|HKCU|Bits64|Run|RiotClient").clone();
    let t = Target::of(&riot).unwrap();
    assert_eq!(t.to_text(), "flag|HKCU|Run|RiotClient");
    assert_eq!(Target::from_text(&t.to_text()), Some(t.clone()));
    assert!(t.is_normal());
    let c = s.set_enabled(&riot, false).unwrap();
    assert_eq!(Target::of_change(&c), t);
    let was = State::before(&c);
    assert_eq!(was.to_text(), "on");
    assert_eq!(s.state_of(&t).unwrap(), State::Enabled(false));
    // what a fresh process gets: only the two texts
    let (t2, was2) = (Target::from_text("flag|HKCU|Run|RiotClient").unwrap(), State::from_text("on").unwrap());
    s.put_back(&t2, was2).unwrap();
    assert_eq!(s.state_of(&t).unwrap(), State::Enabled(true));
    assert_eq!(s.os().get_binary(HKCU, APPROVED_RUN, "RiotClient"), Some(vec![2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), "Task Manager's on bytes");
    // already so: nothing written
    let n = s.os().state().writes.len();
    s.put_back(&t2, was2).unwrap();
    assert_eq!(s.os().state().writes.len(), n);
    // a value name with the separator in it survives
    let odd = Target::Flag(ApprovedSlot { hive: HKLM, key: ApprovedKey::Run32, value_name: "A|B".into() });
    assert_eq!(Target::from_text(&odd.to_text()), Some(odd));
}

#[test]
fn saved_hklm_flag_needs_admin_to_go_back() {
    let s = Startup::new(pc());
    let t = Target::from_text("flag|HKLM|StartupFolder|Updater.lnk").unwrap();
    assert_eq!(s.state_of(&t).unwrap(), State::Enabled(false));
    assert_eq!(s.put_back(&t, State::Enabled(true)), Err(StartupError::NeedsAdmin));
    let s = Startup::new(pc().admin(true));
    s.put_back(&t, State::Enabled(true)).unwrap();
    assert!(s.state_of(&t).unwrap().is_on());
}

#[test]
fn saved_task_and_service_go_back() {
    let s = Startup::new(pc().admin(true));
    let l = s.list();
    let task = find(&l, r"task|\Vendor Updater").clone();
    let c = s.set_enabled(&task, false).unwrap();
    let t = Target::of_change(&c);
    assert_eq!(t.to_text(), r"task|\Vendor Updater");
    assert!(!t.is_normal());
    s.put_back(&t, State::before(&c)).unwrap();
    assert_eq!(s.state_of(&t).unwrap(), State::Enabled(true));

    let d = find(&l, "service|DelayedSvc").clone();
    let c = s.set_enabled(&d, false).unwrap();
    let t = Target::of(&d).unwrap();
    let was = State::before(&c);
    assert_eq!(was.to_text(), "auto-delayed");
    assert_eq!(s.state_of(&t).unwrap().to_text(), "manual");
    s.put_back(&Target::from_text(&t.to_text()).unwrap(), State::from_text("auto-delayed").unwrap()).unwrap();
    assert_eq!(s.os().service_start("DelayedSvc").unwrap(), (ServiceStart::Automatic, true));
    assert!(s.os().remembered_services().unwrap().is_empty(), "back on: forgotten, as set_enabled does");
    // back to Manual keeps it listed (remembered as Delayed Start)
    s.put_back(&t, State::from_text("manual").unwrap()).unwrap();
    assert_eq!(s.os().remembered_services().unwrap(), vec![("DelayedSvc".to_string(), true)]);
    assert!(s.list().get("service|DelayedSvc").is_some());
    // without admin: says so
    let s2 = Startup::new(pc());
    assert_eq!(s2.put_back(&t, State::from_text("auto").unwrap()), Err(StartupError::NeedsAdmin));
    for x in ["on", "off", "auto", "auto-delayed", "manual", "disabled", "boot", "system"] {
        assert_eq!(State::from_text(x).unwrap().to_text(), x);
    }
    assert!(State::from_text("maybe").is_none() && Target::from_text("flag|HKXX|Run|a").is_none());
}

fn store(family: &str, task: &str, publisher: &str) -> StoreStartupTask {
    StoreStartupTask {
        package_family: family.into(),
        task_id: task.into(),
        state: 2,
        display_name: Some(task.into()),
        publisher: Some(publisher.into()),
        logo: None,
    }
}

/// Order 074 (the owner: "why is spotify windows only"): a Store app is "Windows" only when it is a part of Windows - every other
/// publisher, and Microsoft's own apps that are not Windows (Xbox, Phone Link, Windows Terminal), are ordinary rows, whatever the
/// startup kind.
#[test]
fn only_parts_of_windows_are_windows_store_apps() {
    let f = FakeOs::new()
        .store(store("SpotifyAB.SpotifyMusic_zpdnekdrzrea0", "SpotifyLauncher", "Spotify AB"))
        .store(store("Claude_pzs8sxrjxfjjc", "ClaudeStartup", "Anthropic, PBC"))
        .store(store("Microsoft.GamingApp_8wekyb3d8bbwe", "Xbox", "Microsoft Corporation"))
        .store(store("Microsoft.YourPhone_8wekyb3d8bbwe", "YourPhone.Start", "Microsoft Corporation"))
        .store(store("Microsoft.WindowsTerminal_8wekyb3d8bbwe", "Term", "Microsoft Corporation"))
        .store(store("Microsoft.StartExperiencesApp_8wekyb3d8bbwe", "Feed", "Microsoft Corporation"))
        .store(store("MicrosoftWindows.CrossDevice_cw5n1h2txyewy", "CrossDevice.Start", "Microsoft Windows"))
        .store(store("Windows.Fake_abc", "Spoof", "Some Other Company"));
    let l = Startup::new(f).list();
    let own = |id: &str| l.entries.iter().find(|e| e.id.starts_with(id)).unwrap().windows_own;
    assert!(!own("store|SpotifyAB"));
    assert!(!own("store|Claude_"));
    assert!(!own("store|Microsoft.GamingApp"));
    assert!(!own("store|Microsoft.YourPhone"));
    assert!(!own("store|Microsoft.WindowsTerminal"));
    assert!(own("store|Microsoft.StartExperiencesApp"));
    assert!(own("store|MicrosoftWindows.CrossDevice"));
    assert!(!own("store|Windows.Fake"), "a package must also be published by Microsoft");
}

/// Order 074: a task in \Microsoft\Windows\ is Windows' own - but one whose program names another company is an ordinary task,
/// and a Microsoft task outside \Microsoft\Windows\ (Office, Edge) is not a part of Windows.
#[test]
fn a_task_is_windows_only_when_microsoft_made_it_in_the_windows_folder() {
    let f = pc()
        .file(r"C:\Program Files\Vendor\vtask.exe", "Vendor Inc.", "Vendor task")
        .task(task(r"\Microsoft\Windows\Vendor\VTask", true, r"C:\Program Files\Vendor\vtask.exe"))
        .task(task(r"\Microsoft\Office\Sync", true, r"C:\Program Files\Updater\upd.exe"));
    let l = Startup::new(f).list();
    let own = |p: &str| find(&l, p).windows_own;
    assert!(own(r"task|\Microsoft\Windows\Foo\WinTask"));
    assert!(!own(r"task|\Microsoft\Windows\Vendor\VTask"));
    assert!(!own(r"task|\Microsoft\Office\Sync"));
}
