//! The REAL registry code, run against a scratch key `HKCU\Software\BoylerUtilities-test\toggles\<test>` that each test
//! deletes at the end. the owner's real settings are only READ (to prove they didn't change). In scratch mode every after-step
//! (broadcast, Explorer restart, layout-hotkey reload) is recorded, never done; SPI / power / Bluetooth / Copilot
//! changes are refused.
#![cfg(windows)]

use bu_toggles::model::{Kind, Value};
use bu_toggles::os::{Hive, RegValue, TogglesOs};
use bu_toggles::real::{remove_scratch, RealOs};
use bu_toggles::rows::{self, Method, ROWS};
use bu_toggles::{Error, Toggles};

/// One scratch test at a time: a test's cleanup removes the empty `toggles` parent, which would pull the key out from under
/// another test that is creating its keys (measured: ERROR_KEY_DELETED).
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Deletes the scratch key even when the test fails.
struct Scratch(String, #[allow(dead_code)] std::sync::MutexGuard<'static, ()>);
impl Scratch {
    fn new(name: &str) -> Self {
        let guard = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
        let base = format!(r"Software\BoylerUtilities-test\toggles\{name}-{}", std::process::id());
        let _ = remove_scratch(&base);
        Scratch(base, guard)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        remove_scratch(&self.0).expect("scratch key removed");
    }
}

fn is_registry_row(m: Method) -> bool {
    matches!(m, Method::Reg { .. } | Method::KeyExists { .. } | Method::Dxg { .. } | Method::LangHotkeys)
}

fn sw<O: TogglesOs>(t: &Toggles<O>, id: &str) -> bool {
    match t.read(id).unwrap().value {
        Value::Switch(b) => b,
        v => panic!("{v:?}"),
    }
}

/// Every value a registry row touches, read through the given OS layer.
fn values_of(os: &RealOs, id: &str) -> Vec<Option<RegValue>> {
    let row = rows::find(id).unwrap();
    match row.method {
        Method::Reg { values, .. } => values.iter().map(|v| os.reg_read(v.hive, v.path, v.name).unwrap()).collect(),
        Method::KeyExists { hive, key, .. } => vec![os.reg_read(hive, key, "").unwrap()],
        Method::Dxg { .. } => vec![os.reg_read(Hive::Hkcu, rows::DXG_PATH, rows::DXG_VALUE).unwrap()],
        Method::LangHotkeys => ["Hotkey", "Language Hotkey", "Layout Hotkey"]
            .iter()
            .map(|n| os.reg_read(Hive::Hkcu, r"Keyboard Layout\Toggle", n).unwrap())
            .collect(),
        _ => unreachable!(),
    }
}

#[test]
fn every_registry_row_flips_and_undoes_in_the_scratch_key() {
    let s = Scratch::new("rows");
    let real = RealOs::read_only();
    let reg_rows: Vec<_> = ROWS.iter().filter(|r| r.kind == Kind::Switch && is_registry_row(r.method)).collect();
    // (Order 045: + call_ducking)
    assert_eq!(reg_rows.len(), 37);
    // the owner's real values before (read-only)
    let real_before: Vec<_> = reg_rows.iter().map(|r| values_of(&real, r.id)).collect();

    let mut t = Toggles::new(RealOs::scratch(&s.0).unwrap());
    for row in &reg_rows {
        let before = sw(&t, row.id);
        let vals_before = values_of(t.os(), row.id);
        let a = t.set(row.id, !before).unwrap_or_else(|e| panic!("{}: {e}", row.id));
        assert_eq!(a.value, Value::Switch(!before), "{}", row.id);
        assert_eq!(sw(&t, row.id), !before, "{} read back from the real registry code", row.id);
        t.undo(row.id).unwrap_or_else(|e| panic!("{} undo: {e}", row.id));
        assert_eq!(sw(&t, row.id), before, "{} after undo", row.id);
        assert_eq!(values_of(t.os(), row.id), vals_before, "{}: undo puts back the exact old values", row.id);
        // flip once more so the scratch key holds a real write to look at
        t.set(row.id, !before).unwrap();
    }
    // after-steps were only recorded
    let rec = t.os().recorded();
    assert!(rec.iter().any(|r| r == "restart_explorer"));
    assert!(rec.iter().any(|r| r == "refresh_shell"));
    assert!(rec.iter().any(|r| r == "broadcast:ImmersiveColorSet"));
    assert!(rec.iter().any(|r| r == "reload_language_hotkeys"));

    // the scratch key really holds the writes (one spot check per kind)
    let k = |p: &str| format!(r"{}\HKCU\{p}", s.0);
    let scratch_view = RealOs::scratch(&s.0).unwrap();
    assert_eq!(scratch_view.reg_read(Hive::Hkcu, rows::ADV, "HideFileExt").unwrap(), Some(RegValue::Dword(0)));
    assert!(scratch_view.reg_key_exists(Hive::Hkcu, rows::CLASSIC_MENU_SUBKEY).unwrap());
    assert!(RealOs::read_only().reg_key_exists(Hive::Hkcu, &k(rows::CLASSIC_MENU_SUBKEY)).unwrap());
    // HKLM rows landed under the scratch key's HKLM branch, never in HKLM
    assert_eq!(
        scratch_view.reg_read(Hive::Hklm, r"SOFTWARE\Policies\Microsoft\Dsh", "AllowNewsAndInterests").unwrap(),
        Some(RegValue::Dword(0))
    );
    assert!(RealOs::read_only()
        .reg_read(Hive::Hkcu, &format!(r"{}\HKLM\SOFTWARE\Policies\Microsoft\Dsh", s.0), "AllowNewsAndInterests")
        .unwrap()
        .is_some());

    // the owner's real values after: unchanged
    let real_after: Vec<_> = reg_rows.iter().map(|r| values_of(&real, r.id)).collect();
    assert_eq!(real_before, real_after, "real registry must not change");
}

#[test]
fn admin_path_in_the_scratch_key() {
    let s = Scratch::new("admin");
    let mut t = Toggles::new(RealOs::scratch(&s.0).unwrap().with_elevated(false));
    let before = sw(&t, "widgets");
    assert_eq!(t.set("widgets", !before).unwrap_err(), Error::NeedsAdmin { row: "widgets".into() });
    assert_eq!(sw(&t, "widgets"), before);
    // a non-admin row still works
    let b = sw(&t, "clock_seconds");
    t.set("clock_seconds", !b).unwrap();
}

#[test]
fn fullscreen_optimizations_in_the_scratch_key() {
    let s = Scratch::new("fso");
    let mut t = Toggles::new(RealOs::scratch(&s.0).unwrap());
    let exe = r"C:\Games\Test Game\Game-Win64-Shipping.exe";
    let other = r"C:\Games\Old\old.exe";
    t.os_mut().reg_write(Hive::Hkcu, rows::LAYERS_PATH, other, &RegValue::Sz("~ RUNASADMIN".into())).unwrap();
    t.fso_set(exe, true).unwrap();
    t.fso_set(other, true).unwrap();
    assert_eq!(
        t.os().reg_read(Hive::Hkcu, rows::LAYERS_PATH, other).unwrap(),
        Some(RegValue::Sz("~ RUNASADMIN DISABLEDXMAXIMIZEDWINDOWEDMODE".into()))
    );
    let games = t.fso_games().unwrap();
    assert_eq!(games.len(), 2);
    t.fso_remove(exe).unwrap();
    t.fso_remove(other).unwrap();
    assert_eq!(t.os().reg_read(Hive::Hkcu, rows::LAYERS_PATH, exe).unwrap(), None);
    assert_eq!(t.os().reg_read(Hive::Hkcu, rows::LAYERS_PATH, other).unwrap(), Some(RegValue::Sz("~ RUNASADMIN".into())));
    t.fso_undo(exe).unwrap();
    assert!(t.fso_state(exe).unwrap());
}

#[test]
fn byte_exact_round_trip_of_other_value_types() {
    let s = Scratch::new("types");
    let mut os = RealOs::scratch(&s.0).unwrap();
    let path = r"Software\Test";
    let other = RegValue::Other { kind: 3, bytes: vec![1, 2, 3, 0, 255] }; // REG_BINARY
    os.reg_write(Hive::Hkcu, path, "bin", &other).unwrap();
    os.reg_write(Hive::Hkcu, path, "sz", &RegValue::Sz("héllo ~ ✓".into())).unwrap();
    os.reg_write(Hive::Hkcu, path, "dw", &RegValue::Dword(0xDEAD_BEEF)).unwrap();
    assert_eq!(os.reg_read(Hive::Hkcu, path, "bin").unwrap(), Some(other));
    assert_eq!(os.reg_read(Hive::Hkcu, path, "sz").unwrap(), Some(RegValue::Sz("héllo ~ ✓".into())));
    assert_eq!(os.reg_read(Hive::Hkcu, path, "dw").unwrap(), Some(RegValue::Dword(0xDEAD_BEEF)));
    assert_eq!(os.reg_read(Hive::Hkcu, path, "missing").unwrap(), None);
    assert_eq!(os.reg_values(Hive::Hkcu, path).unwrap().len(), 3);
    os.reg_delete_value(Hive::Hkcu, path, "dw").unwrap();
    os.reg_delete_value(Hive::Hkcu, path, "dw").unwrap(); // missing: fine
    assert_eq!(os.reg_read(Hive::Hkcu, path, "dw").unwrap(), None);
    os.reg_delete_tree(Hive::Hkcu, path).unwrap();
    assert!(!os.reg_key_exists(Hive::Hkcu, path).unwrap());
    os.reg_delete_tree(Hive::Hkcu, path).unwrap(); // missing: fine
}

#[test]
fn scratch_and_read_only_refuse_every_real_system_change() {
    let s = Scratch::new("refuse");
    let mut os = RealOs::scratch(&s.0).unwrap();
    use bu_toggles::os::{PowerSetting, PowerValues, SpiItem};
    assert!(matches!(os.spi_set(SpiItem::StickyKeysFlags, 0), Err(Error::ReadOnly(_))));
    assert!(matches!(os.power_write(PowerSetting::Sleep, PowerValues { ac: 0, dc: 0 }), Err(Error::ReadOnly(_))));
    assert!(matches!(os.set_bluetooth(false), Err(Error::ReadOnly(_))));
    assert!(matches!(os.remove_copilot(), Err(Error::ReadOnly(_))));
    os.open_uri("ms-settings:taskbar").unwrap();
    os.open_with_dialog(".png").unwrap();
    assert_eq!(os.recorded(), ["open_uri:ms-settings:taskbar", "open_with:.png"]);

    let mut ro = RealOs::read_only();
    assert!(matches!(ro.reg_write(Hive::Hkcu, rows::ADV, "HideFileExt", &RegValue::Dword(0)), Err(Error::ReadOnly(_))));
    assert!(matches!(ro.reg_delete_tree(Hive::Hkcu, rows::CLASSIC_MENU_KEY), Err(Error::ReadOnly(_))));
    assert!(matches!(ro.restart_explorer(), Err(Error::ReadOnly(_))));
    assert!(matches!(ro.refresh_shell(), Err(Error::ReadOnly(_))));
    assert!(matches!(ro.broadcast_setting_change(None), Err(Error::ReadOnly(_))));
    assert!(matches!(ro.spi_set(SpiItem::StickyKeysFlags, 0), Err(Error::ReadOnly(_))));
    // the service on a read-only layer: reading works, changing is refused
    let mut t = Toggles::new(RealOs::read_only());
    let b = sw(&t, "clock_seconds");
    assert!(matches!(t.set("clock_seconds", !b), Err(Error::ReadOnly(_))));
    assert_eq!(sw(&t, "clock_seconds"), b);
}

#[test]
fn remove_scratch_refuses_anything_else() {
    assert!(remove_scratch(r"Software\Microsoft").is_err());
    assert!(remove_scratch(r"Software\BoylerUtilities-test\..\Microsoft").is_err());
    assert!(remove_scratch("").is_err());
    assert!(remove_scratch(r"Software\BoylerUtilities-test\").is_err(), "the shared parent itself is refused");
    assert!(matches!(RealOs::scratch(r"Software\Microsoft\Windows"), Err(Error::ReadOnly(_))));
    assert!(matches!(RealOs::scratch(r"Software\BoylerUtilities-test\..\Classes"), Err(Error::ReadOnly(_))));
    assert!(matches!(RealOs::scratch(r"Software\BoylerUtilities-test"), Err(Error::ReadOnly(_))));
}
