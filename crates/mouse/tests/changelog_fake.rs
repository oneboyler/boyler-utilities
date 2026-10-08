//! Order 036 (the app's change log): every value the Mouse tab records as text, read from the FAKE PC, changed, and put
//! back from that text alone (a fresh service, no earlier state) — Windows' mouse settings, the cursors' look, the cursor
//! size, Raw Accel's driver state (kept as a file: in the fake's memory, never on the disk).

use bu_mouse::accel::args::{AccelMode, DriverConfig};
use bu_mouse::accel::bytes;
use bu_mouse::accel::service::driver_state_on;
use bu_mouse::cursors::{SetId, WinRole, ACCESSIBILITY_KEY, CURSORS_KEY};
use bu_mouse::fake::FakeOs;
use bu_mouse::os::{DriverVersion, Hive, MouseOs, RegValue, WinRaw, WinSetting};
use bu_mouse::{AppDirs, Mouse};
use std::path::PathBuf;

fn fake() -> Mouse<FakeOs> {
    Mouse::new(FakeOs::new(), AppDirs::new(r"C:\fake-app-data\mouse"))
}

#[test]
fn windows_values_go_to_text_and_back() {
    for (s, v) in [
        (WinSetting::PointerSpeed, WinRaw::Num(8)),
        (WinSetting::Precision, WinRaw::Mouse([6, 10, 1])),
        (WinSetting::Precision, WinRaw::Mouse([0, 0, 0])),
        (WinSetting::ScrollLines, WinRaw::Num(u32::MAX)),
        (WinSetting::DoubleClick, WinRaw::Num(450)),
        (WinSetting::SwapButtons, WinRaw::Bool(true)),
        (WinSetting::SwapButtons, WinRaw::Bool(false)),
    ] {
        assert_eq!(WinRaw::from_text(s, &v.to_text()), Some(v), "{s:?}");
    }
    assert_eq!(WinRaw::from_text(WinSetting::Precision, "6,10"), None);
    assert_eq!(WinRaw::from_text(WinSetting::SwapButtons, "1"), None);
}

#[test]
fn a_windows_setting_is_put_back_from_its_text_on_a_fresh_service() {
    let mut m = fake();
    let was = m.os().win_get(WinSetting::PointerSpeed).unwrap().to_text();
    m.set_pointer_speed(14).unwrap();
    // a fresh process: only the text
    let mut os = std::mem::take(m.os_mut());
    os.log.clear();
    let mut m2 = Mouse::new(os, AppDirs::new(r"C:\fake-app-data\mouse"));
    m2.restore_windows(WinSetting::PointerSpeed, WinRaw::from_text(WinSetting::PointerSpeed, &was).unwrap()).unwrap();
    assert_eq!(m2.windows_mouse().unwrap().pointer_speed, 10);
    // already there: nothing written
    m2.os_mut().log.clear();
    m2.restore_windows(WinSetting::PointerSpeed, WinRaw::Num(10)).unwrap();
    assert!(m2.os().log.is_empty(), "{:?}", m2.os().log);
}

#[test]
fn the_cursors_look_is_put_back_from_its_text() {
    let mut m = fake();
    let was = m.cursor_look_text().unwrap();
    let list: Vec<String> = serde_json::from_str(&was).unwrap();
    assert_eq!(list.len(), 2 + WinRole::ALL.len());
    assert_eq!((list[0].as_str(), list[1].as_str()), ("Windows Default", "2"));
    m.set_scheme("Windows Black").unwrap();
    assert_ne!(m.cursor_look_text().unwrap(), was);
    let size = m.cursor_size_text().unwrap();
    m.restore_cursor_look(&was).unwrap();
    assert_eq!(m.cursor_look_text().unwrap(), was, "every role, the name and the source back");
    assert_eq!(m.cursor_size_text().unwrap(), size, "the size is not touched");
    assert!(m.os().log.iter().any(|l| l == "reload_cursors"));
    assert!(m.restore_cursor_look("not a look").is_err());
}

#[test]
fn windows_default_look_is_windows_own_files() {
    let mut m = fake();
    m.set_scheme("Windows Black").unwrap();
    let def = m.windows_default_look_text().unwrap();
    m.restore_cursor_look(&def).unwrap();
    let st = m.cursors().unwrap();
    assert!(st.roles.iter().all(|r| r.set == SetId::WindowsDefault), "{:?}", st.roles);
    assert_eq!(m.os().reg_get(Hive::Hkcu, CURSORS_KEY, "Scheme Source"), Some(&RegValue::Dword(0)));
}

#[test]
fn the_cursor_size_is_put_back_from_its_text() {
    let mut m = fake();
    let was = m.cursor_size_text().unwrap();
    assert_eq!(was, "32,1");
    m.set_cursor_size(3).unwrap();
    assert_eq!(m.cursor_size_text().unwrap(), "64,3");
    let look = m.cursor_look_text().unwrap();
    m.restore_cursor_size(&was).unwrap();
    assert_eq!(m.cursor_size_text().unwrap(), "32,1");
    assert_eq!(m.os().reg_get(Hive::Hkcu, ACCESSIBILITY_KEY, "CursorSize"), Some(&RegValue::Dword(1)));
    assert_eq!(m.cursor_look_text().unwrap(), look, "the roles are kept");
    // missing values read as Windows' own (32 px, 1)
    m.os_mut().reg.remove(&(Hive::Hkcu, ACCESSIBILITY_KEY.to_string(), "CursorSize".to_string()));
    assert_eq!(m.cursor_size_text().unwrap(), "32,1");
    assert!(m.restore_cursor_size("big").is_err());
}

#[test]
fn raw_accels_driver_state_is_kept_in_a_file_and_put_back() {
    let mut m = fake();
    // no driver: nothing to keep
    assert_eq!(m.driver_state().unwrap(), None);
    m.os_mut().rawaccel_version = Some(DriverVersion { major: 1, minor: 7, patch: 0 });
    let his = bytes::to_bytes(&DriverConfig::default());
    m.os_mut().rawaccel_driver = his.clone();
    let before = m.driver_state().unwrap().unwrap();
    assert!(!driver_state_on(&before), "Raw Accel's defaults: no curve");
    let file = m.keep_driver_state(&before).unwrap();
    assert!(file.starts_with(PathBuf::from(r"C:\fake-app-data\mouse\rawaccel\before")));
    assert_eq!(m.os().byte_files.get(&file), Some(&before));
    // the same state again: the same file, not written again
    let writes = m.os().log.len();
    assert_eq!(m.keep_driver_state(&before).unwrap(), file);
    assert_eq!(m.os().log.len(), writes);
    // the app hands the driver a curve
    let mut cfg = DriverConfig::default();
    cfg.profiles[0].accel_x.mode = AccelMode::Classic;
    cfg.profiles[0].accel_x.acceleration = 0.005;
    m.os_mut().rawaccel_write(&bytes::to_bytes(&cfg)).unwrap();
    let after = m.driver_state().unwrap().unwrap();
    assert_ne!(after, before);
    assert!(driver_state_on(&after));
    assert_ne!(m.driver_state_file(&after), file);
    // a fresh service puts it back from the file alone
    let os = std::mem::take(m.os_mut());
    let mut m2 = Mouse::new(os, AppDirs::new(r"C:\fake-app-data\mouse"));
    m2.restore_driver_state(&file).unwrap();
    assert_eq!(m2.driver_state().unwrap().unwrap(), before);
    // already running it: no WRITE
    let n = m2.os().rawaccel_byte_writes;
    m2.restore_driver_state(&file).unwrap();
    assert_eq!(m2.os().rawaccel_byte_writes, n);
    // only the app's own copies
    assert!(m2.restore_driver_state(&PathBuf::from(r"C:\Windows\x.bin")).is_err());
    assert!(m2.restore_driver_state(&file.with_file_name("0000000000000000.bin")).is_err());
}
