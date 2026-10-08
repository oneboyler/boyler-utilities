//! The REAL registry write path, proven inside a scratch key only (boss answer A_006_01):
//! everything is under HKCU\Software\BoylerUtilities-test\D\<process id>, created and removed by this test.
//! Tasks, services and folders are only READ here (Startup::list), never written.
#![cfg(windows)]

use bu_startup::real::{win, RealOs};
use bu_startup::*;

const LANE: &str = r"Software\BoylerUtilities-test\D";
/// One key per test process, so two test runs at once (a lane and the integrator) never share it.
fn prefix() -> String {
    format!(r"{LANE}\{}", std::process::id())
}
const APPROVED_RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const NAME: &str = "BU-Lane-D-Test";

/// Removes the scratch key even if the test fails.
struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = win::hkcu_delete_tree(&self.0);
        let _ = win::hkcu_delete_key_if_empty(LANE); // only goes if no other run's key is inside
    }
}

#[test]
fn real_registry_writes_inside_the_scratch_key_only() {
    let real = RealOs::new();
    // The user's real flag for this name must not exist before or after.
    assert_eq!(real.reg_binary(Hive::CurrentUser, APPROVED_RUN, NAME).unwrap(), None);

    let prefix = prefix();
    let _guard = Cleanup(prefix.clone());
    win::hkcu_delete_tree(&prefix).unwrap();
    // Another run's Cleanup may delete the empty `D` key while we create ours below it (ERROR_KEY_DELETED): try again a few times.
    let run_key = format!(r"{prefix}\HKCU\Software\Microsoft\Windows\CurrentVersion\Run");
    let mut tries = 0;
    while let Err(e) = win::hkcu_set_string(&run_key, NAME, r#""C:\Windows\System32\notepad.exe" /bu-test"#) {
        tries += 1;
        assert!(tries < 5, "could not create the scratch Run value after {tries} tries: {e:?}");
    }

    let s = Startup::new(RealOs::scratch(&prefix).expect("prefix is inside the scratch key"));
    let l = s.list();
    let id = format!("run|HKCU|Bits64|Run|{NAME}");
    let e = l.get(&id).expect("scratch Run value is listed").clone();
    assert!(e.enabled, "no flag = on");
    assert_eq!(e.publisher.as_deref(), Some("Microsoft Corporation"), "real version info read");

    // Off: 03 00 00 00 + now as FILETIME, written by the REAL registry code, read back.
    let before = s.os().now_filetime();
    let off = s.set_enabled(&e, false).unwrap();
    let after = s.os().now_filetime();
    let v = s.os().reg_binary(Hive::CurrentUser, APPROVED_RUN, NAME).unwrap().expect("flag written");
    assert_eq!(v.len(), 12);
    assert_eq!(&v[..4], &[3, 0, 0, 0]);
    let ft = u64::from_le_bytes(v[4..12].try_into().unwrap());
    assert!(ft >= before && ft <= after, "FILETIME is the switch time");
    assert!(!s.list().get(&id).expect("scratch row still listed after off").enabled);

    // On: 02 + zeros; its undo puts the exact 03 bytes back.
    let e2 = s.list().get(&id).expect("scratch row still listed").clone();
    let on = s.set_enabled(&e2, true).unwrap();
    assert_eq!(s.os().reg_binary(Hive::CurrentUser, APPROVED_RUN, NAME).unwrap(), Some(vec![2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
    s.undo(&on).unwrap();
    assert_eq!(s.os().reg_binary(Hive::CurrentUser, APPROVED_RUN, NAME).unwrap(), Some(v.clone()));
    // Undo of the first change: the flag was missing, so it is removed again.
    s.undo(&off).unwrap();
    assert_eq!(s.os().reg_binary(Hive::CurrentUser, APPROVED_RUN, NAME).unwrap(), None);
    assert!(s.list().get(&id).expect("scratch row still listed after undo").enabled);

    // The services memory (the app's own key) through the same redirect.
    s.os().remember_service("BU-Fake-Service", true).unwrap();
    assert_eq!(s.os().remembered_services().unwrap(), vec![("BU-Fake-Service".to_string(), true)]);
    s.os().forget_service("BU-Fake-Service").unwrap();
    assert!(s.os().remembered_services().unwrap().is_empty());

    // HKLM rows still need admin in scratch mode (this test runs without admin).
    if !s.os().is_admin() {
        let lm = StartupEntry {
            switch: Switch::NeedsAdmin,
            approved: Some(ApprovedSlot { hive: Hive::LocalMachine, key: ApprovedKey::Run, value_name: NAME.into() }),
            ..e.clone()
        };
        assert_eq!(s.set_enabled(&lm, false), Err(StartupError::NeedsAdmin));
    }

    drop(_guard);
    assert!(!win::hkcu_key_exists(&prefix), "scratch key removed");
    assert_eq!(real.reg_binary(Hive::CurrentUser, APPROVED_RUN, NAME).unwrap(), None, "real StartupApproved untouched");
}

#[test]
fn scratch_guards() {
    // RealOs::scratch and the helpers refuse anything outside HKCU\Software\BoylerUtilities-test\D.
    assert!(RealOs::scratch(r"Software\Microsoft\Windows\CurrentVersion").is_err());
    assert!(RealOs::scratch(r"Software\BoylerUtilities-test\C\1").is_err());
    assert!(RealOs::scratch(r"Software\BoylerUtilities-test\D\..\C").is_err());
    // Real keys that matter are checked with the pure path check only — never by calling a write on them, so a broken guard
    // can't do damage before the assert fails (REVIEW_006_done_eaed99a).
    for p in [
        r"Software\BoylerUtilities-test",
        r"Software\BoylerUtilities-test\DX",
        r"Software\Microsoft",
        r"Software\Microsoft\Windows\CurrentVersion\Run",
    ] {
        assert!(win::check_scratch_path(p).is_err(), "{p} must be outside the scratch key");
    }
    // The helpers themselves are called only on a key that doesn't exist and lies outside the scratch key.
    assert!(win::hkcu_delete_tree(r"Software\BU-Lane-D-not-scratch\x").is_err());
    assert!(win::hkcu_delete_key_if_empty(r"Software\BU-Lane-D-not-scratch").is_err());
    assert!(win::hkcu_set_string(r"Software\BU-Lane-D-not-scratch\Run", "x", "y").is_err());
    assert!(!win::hkcu_key_exists(r"Software\BU-Lane-D-not-scratch"), "nothing was created outside");
    // In scratch mode real task / service writes are refused before Windows is called (nothing is changed: the names don't exist
    // either way, and the refusal comes first).
    let s = RealOs::scratch(&format!(r"{}\guard", win::SCRATCH_ROOT)).expect("inside");
    assert!(matches!(s.set_task_enabled(r"\BU-Lane-D-no-such-task", false), Err(OsError::Other { code: 0, .. })));
    assert!(matches!(s.set_service_start("BU-Lane-D-no-such-service", ServiceStart::Manual, false), Err(OsError::Other { code: 0, .. })));
    assert!(!win::hkcu_key_exists(&format!(r"{}\guard", win::SCRATCH_ROOT)), "the guard test wrote nothing");
}
