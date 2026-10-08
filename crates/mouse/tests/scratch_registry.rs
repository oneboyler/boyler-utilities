//! Cursors on the REAL registry, but only inside the lane's scratch key `HKCU\Software\BoylerUtilities-test\G\<pid>`
//! (`RealOs::scratch`): every HKCU path is mapped under it, HKLM is only read (Windows' real schemes), and every other
//! Windows call (SPI_SETCURSORS, SetSystemCursor, SystemParametersInfo) is RECORDED, never made. The test checks that
//! the user's real `HKCU\Control Panel\Cursors` and mouse settings are byte-identical before and after, and deletes only
//! its own subtree (never the shared parent). One test function = scratch keys are never used in parallel here.

#![cfg(windows)]

use bu_mouse::cursors::{Role, SetId, CURSORS_KEY};
use bu_mouse::os::{Hive, MouseOs, RegValue};
use bu_mouse::win::RealOs;
use bu_mouse::{AppDirs, Mouse};
use windows::core::HSTRING;
use windows::Win32::System::Registry::{RegDeleteKeyW, RegDeleteTreeW, HKEY_CURRENT_USER};

struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        assert!(self.0.starts_with(r"Software\BoylerUtilities-test\G\"), "only our own subtree");
        let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(self.0.as_str())) };
        // our lane key too when it is empty (RegDeleteKey refuses a key with subkeys); never the shared parent
        let _ = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(r"Software\BoylerUtilities-test\G")) };
    }
}

fn real_cursor_key() -> Vec<(String, RegValue)> {
    let mut v = RealOs::read_only().reg_values(Hive::Hkcu, CURSORS_KEY).unwrap();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

#[test]
fn cursor_changes_land_only_in_the_scratch_key_and_undo_restores_them() {
    let base = format!(r"Software\BoylerUtilities-test\G\{}", std::process::id());
    let _guard = Cleanup(base.clone());
    assert!(RealOs::scratch(r"Software\BoylerUtilities-test").is_err(), "the shared parent itself is refused");
    assert!(RealOs::scratch(r"Software\Other").is_err());

    let real_before = real_cursor_key();
    let real_mouse_before = Mouse::new(RealOs::read_only(), AppDirs::new("unused")).windows_mouse().unwrap();

    let mut os = RealOs::scratch(&base).unwrap();
    // seed the scratch copy of the cursor key with Windows' own default (read from HKLM, read only)
    for (n, v) in os.reg_values(Hive::Hklm, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Control Panel\Cursors\Default").unwrap() {
        os.reg_write(CURSORS_KEY, &n, &v).unwrap();
    }
    os.reg_write(CURSORS_KEY, "CursorBaseSize", &RegValue::Dword(32)).unwrap();
    let mut m = Mouse::new(os, AppDirs::new(r"C:\BoylerUtilities-scratch\lane-g\unused"));
    let before = m.cursor_snapshot().unwrap();
    assert!(m.cursors().unwrap().roles.iter().all(|r| r.set == SetId::WindowsDefault));

    // a whole scheme from Windows' real list
    m.set_scheme("Windows Black").unwrap();
    let arrow = m.os().reg_read(Hive::Hkcu, CURSORS_KEY, "Arrow").unwrap();
    assert_eq!(arrow, Some(RegValue::ExpandSz(r"%SystemRoot%\cursors\arrow_r.cur".into())));
    assert_eq!(m.os().reg_read(Hive::Hkcu, CURSORS_KEY, "").unwrap(), Some(RegValue::Sz("Windows Black".into())));
    // one role back to Windows default, then the size
    m.set_role(Role::Normal, SetId::WindowsDefault).unwrap();
    m.set_cursor_size(2).unwrap();
    assert_eq!(m.os().reg_read(Hive::Hkcu, CURSORS_KEY, "CursorBaseSize").unwrap(), Some(RegValue::Dword(48)));
    assert_eq!(m.os().reg_read(Hive::Hkcu, r"Software\Microsoft\Accessibility", "CursorSize").unwrap(), Some(RegValue::Dword(2)));
    // undo steps back once (the size); the scheme switch is undone by the snapshot it kept
    m.undo_cursors().unwrap();
    assert_eq!(m.os().reg_read(Hive::Hkcu, CURSORS_KEY, "CursorBaseSize").unwrap(), Some(RegValue::Dword(32)));
    // a Windows setting change is only recorded
    m.set_pointer_speed(if real_mouse_before.pointer_speed == 20 { 19 } else { 20 }).unwrap();

    let rec = m.os().recorded().to_vec();
    assert!(rec.iter().any(|r| r == "SPI_SETCURSORS"), "{rec:?}");
    assert!(rec.iter().any(|r| r.starts_with("SetSystemCursor 32512 ")), "{rec:?}");
    assert!(rec.iter().any(|r| r.starts_with("win_set PointerSpeed")), "{rec:?}");

    // the scheme switch alone, undone, gives back the seeded state exactly
    let mut m2 = m;
    let before2 = m2.cursor_snapshot().unwrap();
    m2.set_scheme("Windows Aero").unwrap();
    assert_ne!(m2.cursor_snapshot().unwrap(), before2);
    m2.undo_cursors().unwrap();
    assert_eq!(m2.cursor_snapshot().unwrap(), before2, "undo puts every value back exactly");
    assert_eq!(before2.base_size, before.base_size);

    // the user's real cursor key and mouse settings: untouched
    assert_eq!(real_cursor_key(), real_before);
    assert_eq!(Mouse::new(RealOs::read_only(), AppDirs::new("unused")).windows_mouse().unwrap(), real_mouse_before);
}
