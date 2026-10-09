//! "Get more cursors" (Order 066) against the fake registry: a pack zip (made here, like a release page's) is installed into a
//! scratch folder, registered as a scheme, used, and deleted again. Nothing real is touched.

use bu_mouse::cursors::*;
use bu_mouse::fake::FakeOs;
use bu_mouse::os::{Hive, RegValue};
use bu_mouse::{store, zipdir, AppDirs, Mouse};
use std::path::{Path, PathBuf};

const SCRATCH_PARENT: &str = r"C:\BoylerUtilities-scratch";

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        if self.0.starts_with(Path::new(SCRATCH_PARENT).join("CU66")) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

fn scratch(name: &str) -> Option<Scratch> {
    if !Path::new(SCRATCH_PARENT).is_dir() {
        eprintln!("SKIPPED: no scratch folder {SCRATCH_PARENT}");
        return None;
    }
    let d = Path::new(SCRATCH_PARENT).join("CU66").join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).ok()?;
    Some(Scratch(d))
}

fn cur() -> Vec<u8> {
    let mut c = vec![0, 0, 2, 0, 1, 0, 16, 16, 0, 0, 1, 0, 1, 0, 4, 0, 0, 0, 22, 0, 0, 0];
    c.extend_from_slice(&[1, 2, 3, 4]);
    c
}

const INF: &str = "[Scheme.Reg]\nHKCU,\"Control Panel\\Cursors\\Schemes\",\"%SCHEME_NAME%\",,\"%10%\\%CUR_DIR%\\%pointer%,%10%\\%CUR_DIR%\\%help%,%10%\\%CUR_DIR%\\%work%,%10%\\%CUR_DIR%\\%busy%,%10%\\%CUR_DIR%\\%cross%,%10%\\%CUR_DIR%\\%text%\"\n[Strings]\nCUR_DIR = \"Cursors\\X\"\nSCHEME_NAME = \"X\"\npointer = \"Pointer.cur\"\nhelp = \"Help.cur\"\nwork = \"Work.ani\"\nbusy = \"Busy.ani\"\ncross = \"Cross.cur\"\ntext = \"Text.cur\"\n";

fn ani() -> Vec<u8> {
    let mut v = b"RIFF".to_vec();
    v.extend_from_slice(&4u32.to_le_bytes());
    v.extend_from_slice(b"ACON");
    v
}

fn zip() -> Vec<u8> {
    let (c, a) = (cur(), ani());
    zipdir::build(
        &[
            ("Pack-Small/Pointer.cur", &c),
            ("Pack-Regular/Pointer.cur", &c),
            ("Pack-Regular/Help.cur", &c),
            ("Pack-Regular/Text.cur", &c),
            ("Pack-Regular/Cross.cur", &c),
            ("Pack-Regular/Link.cur", &c),
            ("Pack-Regular/Work.ani", &a),
            ("Pack-Regular/Busy.ani", &a),
            ("Pack-Regular/Pan.cur", &c),
            ("Pack-Regular/install.inf", INF.as_bytes()),
            ("Pack-Regular/uninstall.bat", b"@echo off"),
        ],
        true,
    )
}

fn mouse(dir: &Path) -> Mouse<FakeOs> {
    Mouse::new(FakeOs::new(), AppDirs::new(dir))
}

fn scheme_value(m: &Mouse<FakeOs>, name: &str) -> Option<RegValue> {
    m.os().reg_get(Hive::Hkcu, USER_SCHEMES_KEY, name).cloned()
}

#[test]
fn a_store_pack_is_installed_registered_as_a_scheme_and_usable() {
    let Some(s) = scratch("install") else { return };
    let mut m = mouse(&s.0);
    let pack = m.install_store_zip("Test Pack", &zip()).unwrap();
    assert_eq!(pack.name, "Test Pack");
    // only the Regular folder, only the files that have a role
    let dir = s.0.join("cursors").join("packs").join("Test Pack");
    let mut files: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    files.sort();
    assert_eq!(files, vec!["Busy.ani", "Cross.cur", "Help.cur", "Link.cur", "Pointer.cur", "Text.cur", "Work.ani", "pack.json"]);
    assert_eq!(pack.roles[&WinRole::Arrow], "Pointer.cur");
    assert_eq!(pack.roles[&WinRole::Wait], "Busy.ani");
    assert_eq!(pack.roles[&WinRole::AppStarting], "Work.ani");
    assert_eq!(pack.roles[&WinRole::IBeam], "Text.cur");
    assert_eq!(pack.roles[&WinRole::Hand], "Link.cur", "not in the inf's scheme line: found by its name");
    // the scheme: 17 paths in Windows' role order, Windows' own files for the roles the pack has none for
    let Some(RegValue::ExpandSz(v)) = scheme_value(&m, "Test Pack") else { panic!("no scheme") };
    let paths: Vec<&str> = v.split(',').collect();
    assert_eq!(paths.len(), 17);
    assert!(paths[0].ends_with(r"Test Pack\Pointer.cur"), "{}", paths[0]);
    assert!(paths[2].ends_with(r"Test Pack\Work.ani"));
    assert!(paths[3].ends_with(r"Test Pack\Busy.ani"));
    assert!(paths[8].to_ascii_lowercase().contains(r"c:\windows\cursors"), "SizeNS comes from Windows: {}", paths[8]);
    assert!(m.schemes().unwrap().iter().any(|s| s.name == "Test Pack" && !s.system));
    // it is ONE row in the pickers (the pack), not the pack and the scheme
    assert!(!m.installed_schemes().unwrap().iter().any(|(n, _)| n == "Test Pack"));
    assert!(m.packs().unwrap().iter().any(|p| p.name == "Test Pack"));
    // and it is not applied by the install
    assert!(m.cursors().unwrap().roles.iter().all(|r| r.set == SetId::WindowsDefault));
    // using it
    m.set_role(Role::Normal, SetId::Pack("Test Pack".into())).unwrap();
    assert_eq!(m.cursors().unwrap().roles[0].set, SetId::Pack("Test Pack".into()));
    // the real file of that set for that bubble, for the pickers' pictures
    assert!(m.preview_path(&SetId::Pack("Test Pack".into()), Role::Normal).unwrap().ends_with(r"Test Pack\Pointer.cur"));
    let all = m.set_preview_files();
    let (_, f) = all.iter().find(|(id, _)| *id == SetId::Pack("Test Pack".into())).unwrap();
    assert_eq!(f.len(), 7);
    assert!(f[0].as_deref().unwrap().ends_with("Pointer.cur"));
    assert!(f[5].is_none(), "the pack has no Move cursor");
}

#[test]
fn installing_again_changes_nothing_and_a_bad_zip_installs_nothing() {
    let Some(s) = scratch("again") else { return };
    let mut m = mouse(&s.0);
    m.install_store_zip("Test Pack", &zip()).unwrap();
    m.install_store_zip("Test Pack", &zip()).unwrap();
    assert_eq!(m.packs().unwrap().len(), 1);
    assert!(m.install_store_zip("Other", b"not a zip").is_err());
    assert!(m.install_store_zip("No Cursors", &zipdir::build(&[("x/readme.txt", b"hi")], false)).is_err());
    assert!(m.install_store_zip("..", &zip()).is_err(), "a name is one plain folder name");
    assert_eq!(m.packs().unwrap().len(), 1);
    assert!(!s.0.join("cursors").join("packs").join("Other").exists() && !s.0.join("cursors").join("packs").join("Other.new").exists());
    assert!(scheme_value(&m, "Other").is_none());
}

#[test]
fn deleting_the_pack_takes_its_scheme_with_it_but_not_someone_elses() {
    let Some(s) = scratch("delete") else { return };
    let mut m = mouse(&s.0);
    m.install_store_zip("Test Pack", &zip()).unwrap();
    m.set_role(Role::Normal, SetId::Pack("Test Pack".into())).unwrap();
    let back = m.delete_pack("Test Pack").unwrap();
    assert_eq!(back, vec![Role::Normal], "the bubble that used it goes back to Windows default");
    assert!(scheme_value(&m, "Test Pack").is_none());
    assert!(!s.0.join("cursors").join("packs").join("Test Pack").exists());
    // a scheme of the same name that is the user's own (files elsewhere) stays
    let mut m = mouse(&s.0);
    m.install_store_zip("Mine", &zip()).unwrap();
    m.os_mut().reg.insert((Hive::Hkcu, USER_SCHEMES_KEY.into(), "Mine".into()), RegValue::ExpandSz(r"C:\Elsewhere\a.cur,,,,,,,,,,,,,,,,".into()));
    m.delete_pack("Mine").unwrap();
    assert!(scheme_value(&m, "Mine").is_some(), "not ours: left alone");
}

#[test]
fn files_picked_before_are_offered_again() {
    let Some(s) = scratch("own") else { return };
    let mut m = mouse(&s.0);
    let src = s.0.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("cross_r.cur"), cur()).unwrap();
    std::fs::write(src.join("beam_m.cur"), cur()).unwrap();
    m.set_role_file(Role::Normal, &src.join("cross_r.cur")).unwrap();
    m.set_role_file(Role::Text, &src.join("beam_m.cur")).unwrap();
    let own = m.own_files();
    assert_eq!(own.len(), 2, "{own:?}");
    assert!(own.iter().any(|p| p.ends_with("cross_r.cur")) && own.iter().any(|p| p.ends_with("beam_m.cur")));
    // a file picked for Normal goes onto Link
    let pick = own.iter().find(|p| p.ends_with("cross_r.cur")).unwrap().to_string_lossy().into_owned();
    let id = SetId::OwnFile(pick.clone());
    assert_eq!(id.label(), "cross_r.cur");
    assert!(m.set_has(&id, Role::Link));
    m.set_role(Role::Link, id.clone()).unwrap();
    let st = m.cursors().unwrap();
    assert_eq!(st.roles.iter().find(|r| r.role == Role::Link).unwrap().file, pick);
    assert_eq!(m.preview_path(&id, Role::Move).as_deref(), Some(pick.as_str()));
}

#[test]
fn the_list_has_what_a_row_needs() {
    for l in store::LIST.iter() {
        assert!(!l.name.is_empty() && !l.about.is_empty() && !l.maker.is_empty() && l.licence == "GPL-3.0");
    }
}

#[test]
fn a_zip_cannot_write_outside_the_pack_folder() {
    let Some(s) = scratch("slip") else { return };
    let mut m = mouse(&s.0);
    let c = cur();
    let evil = zipdir::build(
        &[
            ("Pack-Regular/Pointer.cur", &c),
            ("Pack-Regular/..\\..\\..\\escaped_Default.cur", &c),
            ("C:\\Users\\x\\escaped2_Normal.cur", &c),
            ("Pack-Regular/sub/../../escaped3.cur", &c),
            ("Pack-Regular\\Text.cur", &c),
        ],
        false,
    );
    let pack = m.install_store_zip("Slip", &evil).unwrap();
    // only plain files of the chosen folder were written; the folder holds nothing else, and nothing escaped
    let packs = s.0.join("cursors").join("packs");
    let mut got: Vec<String> = std::fs::read_dir(packs.join("Slip")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    got.sort();
    assert!(got.iter().all(|f| !f.contains("escaped")), "{got:?}");
    assert!(pack.roles.values().all(|f| !f.contains('\\') && !f.contains('/') && !f.contains(':')), "{:?}", pack.roles);
    assert!(!s.0.join("cursors").join("escaped_Default.cur").exists() && !s.0.join("escaped_Default.cur").exists());
    assert!(!packs.join("escaped_Default.cur").exists());
    // a Windows-made zip with backslash separators installs like any other
    let win = zipdir::build(&[("P-Regular\\Pointer.cur", &c), ("P-Regular\\Text.cur", &c)], false);
    let p2 = m.install_store_zip("Backslashed", &win).unwrap();
    assert!(p2.roles.contains_key(&WinRole::Arrow));
}

#[test]
fn a_scheme_of_the_users_with_the_same_name_is_never_overwritten_or_deleted() {
    let Some(s) = scratch("foreign") else { return };
    let mut m = mouse(&s.0);
    let theirs = RegValue::ExpandSz(r"C:\Windows\Cursors\aero_arrow.cur,,,,,,,,,,,,,,,,".into());
    m.os_mut().reg.insert((Hive::Hkcu, USER_SCHEMES_KEY.into(), "Fuchsia".into()), theirs.clone());
    m.install_store_zip("Fuchsia", &zip()).unwrap();
    assert_eq!(scheme_value(&m, "Fuchsia"), Some(theirs.clone()), "installing leaves it alone");
    assert!(m.packs().unwrap().iter().any(|p| p.name == "Fuchsia"), "the pack itself is installed");
    m.delete_pack("Fuchsia").unwrap();
    assert_eq!(scheme_value(&m, "Fuchsia"), Some(theirs), "deleting the pack leaves it alone too");
}

#[test]
fn deleting_a_pack_puts_back_every_role_that_pointed_into_it() {
    let Some(s) = scratch("allroles") else { return };
    let mut m = mouse(&s.0);
    m.install_store_zip("Test Pack", &zip()).unwrap();
    // Windows' Mouse settings applying the registered scheme writes all 17 roles
    let Some(RegValue::ExpandSz(v)) = scheme_value(&m, "Test Pack") else { panic!() };
    for (r, p) in WinRole::ALL.iter().zip(v.split(',')) {
        m.os_mut().reg.insert((Hive::Hkcu, CURSORS_KEY.into(), r.reg_name().into()), RegValue::ExpandSz(p.to_string()));
    }
    m.delete_pack("Test Pack").unwrap();
    let dir = s.0.join("cursors").join("packs").join("Test Pack").to_string_lossy().to_ascii_lowercase();
    for r in WinRole::ALL {
        let Some(RegValue::ExpandSz(p)) = m.os().reg_get(Hive::Hkcu, CURSORS_KEY, r.reg_name()).cloned() else { continue };
        assert!(!p.to_ascii_lowercase().contains(&dir), "{} still points into the deleted pack: {p}", r.reg_name());
    }
}
