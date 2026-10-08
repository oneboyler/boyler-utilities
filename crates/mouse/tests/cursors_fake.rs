//! Cursors against the fake registry; imported packs are copied inside the lane's scratch folder
//! (`BoylerUtilities-board\scratch\lane-g\<test>-<pid>\`, removed by a guard). Source cursor files are COPIED from
//! C:\Windows\Cursors (read only) into the scratch folder first. No fallback elsewhere: no scratch parent → skipped.

use bu_mouse::cursors::*;
use bu_mouse::fake::FakeOs;
use bu_mouse::os::{Hive, RegValue};
use bu_mouse::{AppDirs, Error, Mouse};
use std::path::{Path, PathBuf};

const SCRATCH_PARENT: &str = r"C:\BoylerUtilities-scratch";

/// Removes its own folder (only inside scratch\lane-g) when the test ends, pass or fail.
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        if self.0.starts_with(Path::new(SCRATCH_PARENT).join("lane-g")) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

fn scratch(name: &str) -> Option<Scratch> {
    if !Path::new(SCRATCH_PARENT).is_dir() {
        eprintln!("SKIPPED: no scratch folder {SCRATCH_PARENT}");
        return None;
    }
    let d = Path::new(SCRATCH_PARENT).join("lane-g").join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).ok()?;
    Some(Scratch(d))
}

/// A "downloaded pack" folder in scratch: real Windows cursor files copied under pack-style names.
fn make_pack(src_dir: &Path, with_inf: bool) -> PathBuf {
    let pack = src_dir.join("Neon Pack");
    std::fs::create_dir_all(&pack).unwrap();
    let copy = |from: &str, to: &str| std::fs::copy(Path::new(r"C:\Windows\Cursors").join(from), pack.join(to)).unwrap();
    copy("aero_arrow.cur", "Normal Select.cur");
    copy("aero_link.cur", "Link Select.cur");
    copy("aero_busy.ani", "Busy.ani");
    copy("aero_working.ani", "Working in Background.ani");
    copy("aero_ns.cur", "Vertical Resize.cur");
    copy("aero_ew.cur", "Horizontal Resize.cur");
    copy("aero_nwse.cur", "Diagonal Resize 1.cur");
    copy("aero_nesw.cur", "Diagonal Resize 2.cur");
    copy("aero_move.cur", "Move.cur");
    std::fs::write(pack.join("readme.txt"), "not a cursor").unwrap();
    std::fs::write(pack.join("broken.cur"), b"nope").unwrap();
    if with_inf {
        std::fs::write(
            pack.join("install.inf"),
            "[Version]\nsignature=\"$CHICAGO$\"\n[Strings]\nCUR_DIR = \"Cursors\\Neon\"\npointer = \"Normal Select.cur\"\nlink = \"Link Select.cur\"\nbusy = \"Busy.ani\"\nwork = \"Working in Background.ani\"\nvert = \"Vertical Resize.cur\"\nhorz = \"Horizontal Resize.cur\"\ndgn1 = \"Diagonal Resize 1.cur\"\ndgn2 = \"Diagonal Resize 2.cur\"\nmove = \"Move.cur\"\nSCHEME_NAME = \"Neon\"\n",
        )
        .unwrap();
    }
    pack
}

fn reg(m: &Mouse<FakeOs>, name: &str) -> Option<RegValue> {
    m.os().reg_get(Hive::Hkcu, CURSORS_KEY, name).cloned()
}

#[test]
fn reads_scheme_roles_and_size() {
    let m = Mouse::new(FakeOs::new(), AppDirs::new("unused"));
    let c = m.cursors().unwrap();
    assert_eq!(c.scheme, "Windows Default");
    assert_eq!(c.scheme_source, Some(2));
    assert_eq!((c.size, c.size_px), (1, 32));
    assert_eq!(c.roles.len(), 7);
    assert!(c.roles.iter().all(|r| r.set == SetId::WindowsDefault));
    assert_eq!(c.roles[0].file, r"C:\Windows\cursors\aero_arrow.cur");
    let s = m.schemes().unwrap();
    assert!(s.iter().any(|s| s.name == "Windows Aero" && s.system && s.paths.len() == 17));
}

#[test]
fn scheme_switch_sets_all_17_and_undo_restores_exactly() {
    let mut m = Mouse::new(FakeOs::new(), AppDirs::new("unused"));
    let before = m.cursor_snapshot().unwrap();
    m.set_scheme("Windows Black").unwrap();
    assert_eq!(reg(&m, ""), Some(RegValue::Sz("Windows Black".into())));
    assert_eq!(reg(&m, "Scheme Source"), Some(RegValue::Dword(2)));
    assert_eq!(reg(&m, "Arrow"), Some(RegValue::ExpandSz(r"%SystemRoot%\cursors\arrow_r.cur".into())));
    assert!(m.os().log.iter().any(|l| l == "reload_cursors"));
    assert!(m.os().log.iter().any(|l| l.starts_with("set_system_cursor 32512 C:\\Windows\\cursors\\arrow_r.cur")), "re-push, %vars% expanded");
    m.undo_cursors().unwrap();
    assert_eq!(m.cursor_snapshot().unwrap(), before);
    assert!(matches!(m.set_scheme("Nope"), Err(Error::NotFound(_))));
}

#[test]
fn size_writes_both_values_keeps_the_set_and_undoes() {
    let mut m = Mouse::new(FakeOs::new(), AppDirs::new("unused"));
    let before = m.cursor_snapshot().unwrap();
    m.set_cursor_size(3).unwrap();
    assert_eq!(reg(&m, "CursorBaseSize"), Some(RegValue::Dword(64)));
    assert_eq!(m.os().reg_get(Hive::Hkcu, ACCESSIBILITY_KEY, "CursorSize"), Some(&RegValue::Dword(3)));
    assert_eq!(reg(&m, "Arrow"), before.roles[0].1, "the role paths are re-written as they were");
    assert_eq!(m.cursors().unwrap().size, 3);
    m.undo_cursors().unwrap();
    assert_eq!(m.cursor_snapshot().unwrap(), before);
    assert!(matches!(m.set_cursor_size(16), Err(Error::OutOfRange { .. })));
    assert_eq!((size_px(1), size_px(15)), (32, 256));
    assert_eq!(size_step(48), 2);
}

#[test]
fn import_pack_with_inf_does_not_apply_then_role_pick_and_suggestion_then_delete() {
    let Some(s) = scratch("import-inf") else { return };
    let src = make_pack(&s.0.join("downloads"), true);
    let mut m = Mouse::new(FakeOs::new(), AppDirs::new(s.0.join("appdata")));
    let before = m.cursor_snapshot().unwrap();
    let (pack, toast, skipped) = m.import_cursors(&[src]).unwrap();
    assert_eq!(pack.name, "Neon Pack");
    assert_eq!(toast, "Imported Neon Pack · pick it in any cursor's list");
    assert_eq!(skipped, vec!["broken.cur".to_string()]);
    assert_eq!(pack.files.len(), 9);
    assert_eq!(pack.roles.get(&WinRole::Arrow).map(String::as_str), Some("Normal Select.cur"));
    assert_eq!(pack.roles.get(&WinRole::SizeNESW).map(String::as_str), Some("Diagonal Resize 2.cur"));
    assert_eq!(m.cursor_snapshot().unwrap(), before, "importing applies nothing");
    assert!(s.0.join(r"appdata\cursors\packs\Neon Pack\pack.json").is_file());

    // nothing matches yet → the "Matches your other cursors" row is hidden
    assert_eq!(m.suggestion(Role::Link).unwrap(), None);
    m.set_role(Role::Normal, SetId::Pack("Neon Pack".into())).unwrap();
    let st = m.cursors().unwrap();
    assert_eq!(st.roles[0].set, SetId::Pack("Neon Pack".into()));
    assert_eq!(reg(&m, "Scheme Source"), Some(RegValue::Dword(0)), "a mix is no named scheme");
    // last picked for another role → suggested for Link (the pack has a Link cursor)
    assert_eq!(m.suggestion(Role::Link).unwrap(), Some(SetId::Pack("Neon Pack".into())));
    // Text: the pack has no Text cursor → hidden
    assert_eq!(m.suggestion(Role::Text).unwrap(), None);
    // Resize sets all four Windows resize roles
    m.set_role(Role::Resize, SetId::Pack("Neon Pack".into())).unwrap();
    for r in ["SizeNS", "SizeWE", "SizeNWSE", "SizeNESW"] {
        assert!(reg(&m, r).and_then(|v| v.as_str().map(|s| s.contains("Neon Pack"))).unwrap_or(false), "{r}");
    }
    // × on the pack: its roles go back to Windows default, folder gone
    let back = m.delete_pack("Neon Pack").unwrap();
    assert_eq!(back, vec![Role::Normal, Role::Resize]);
    assert_eq!(reg(&m, "Arrow"), Some(RegValue::ExpandSz(r"C:\Windows\cursors\aero_arrow.cur".into())));
    assert!(!s.0.join(r"appdata\cursors\packs\Neon Pack").exists());
    assert_eq!(m.suggestion(Role::Link).unwrap(), None);
}

#[test]
fn import_without_inf_guesses_roles_from_file_names_and_names_clash_get_a_number() {
    let Some(s) = scratch("import-guess") else { return };
    let src = make_pack(&s.0.join("downloads"), false);
    let mut m = Mouse::new(FakeOs::new(), AppDirs::new(s.0.join("appdata")));
    let (p1, _, _) = m.import_cursors(std::slice::from_ref(&src)).unwrap();
    assert_eq!(p1.roles.get(&WinRole::Arrow).map(String::as_str), Some("Normal Select.cur"));
    assert_eq!(p1.roles.get(&WinRole::Hand).map(String::as_str), Some("Link Select.cur"));
    assert_eq!(p1.roles.get(&WinRole::Wait).map(String::as_str), Some("Busy.ani"));
    assert_eq!(p1.roles.get(&WinRole::AppStarting).map(String::as_str), Some("Working in Background.ani"));
    assert_eq!(p1.roles.get(&WinRole::SizeAll).map(String::as_str), Some("Move.cur"));
    let (p2, _, _) = m.import_cursors(&[src]).unwrap();
    assert_eq!(p2.name, "Neon Pack 2");
    assert_eq!(m.packs().unwrap().len(), 2);
}

#[test]
fn own_file_for_one_role_and_bad_files() {
    let Some(s) = scratch("own-file") else { return };
    let src = make_pack(&s.0.join("downloads"), false);
    let mut m = Mouse::new(FakeOs::new(), AppDirs::new(s.0.join("appdata")));
    let dst = m.set_role_file(Role::Text, &src.join("Move.cur")).unwrap();
    assert!(dst.starts_with(s.0.join("appdata")));
    assert_eq!(m.cursors().unwrap().roles[2].set, SetId::Own);
    assert!(matches!(m.set_role_file(Role::Text, &src.join("broken.cur")), Err(Error::NotACursor(_))));
    assert!(matches!(m.set_role_file(Role::Text, &src.join("readme.txt")), Err(Error::NotACursor(_))));
    assert!(matches!(m.import_cursors(&[src.join("readme.txt")]), Err(Error::NotACursor(_))));
}

#[test]
fn suggestion_rules() {
    let st = |sets: [SetId; 7]| -> Vec<RoleState> { Role::ALL.iter().zip(sets).map(|(r, s)| RoleState { role: *r, file: String::new(), set: s }).collect() };
    let a = SetId::Pack("A".into());
    let b = SetId::Pack("B".into());
    let d = SetId::WindowsDefault;
    let has_all = |_: &SetId, _: Role| true;
    // majority of the other roles
    let cur = st([a.clone(), a.clone(), b.clone(), d.clone(), d.clone(), d.clone(), d.clone()]);
    assert_eq!(suggest_match(Role::Move, &cur, &[], &has_all), Some(a.clone()));
    // last picked for ANOTHER role beats the majority
    assert_eq!(suggest_match(Role::Move, &cur, &[(Role::Text, b.clone())], &has_all), Some(b.clone()));
    // a pick for the same role does not count
    assert_eq!(suggest_match(Role::Move, &cur, &[(Role::Move, b.clone())], &has_all), Some(a.clone()));
    // never Windows default
    let all_default = st([d.clone(), d.clone(), d.clone(), d.clone(), d.clone(), d.clone(), d.clone()]);
    assert_eq!(suggest_match(Role::Move, &all_default, &[(Role::Text, d.clone())], &has_all), None);
    // tie → the most recently picked
    let tie = st([a.clone(), b.clone(), d.clone(), d.clone(), d.clone(), d.clone(), d.clone()]);
    assert_eq!(suggest_match(Role::Move, &tie, &[(Role::Move, b.clone()), (Role::Move, a.clone())], &has_all), Some(a.clone()));
    // the set must have this role's cursor
    let only_a = |s: &SetId, _: Role| *s == SetId::Pack("A".into());
    assert_eq!(suggest_match(Role::Move, &cur, &[(Role::Text, b)], &only_a), Some(a));
}

#[test]
fn preview_sets_the_real_cursor_only_and_end_reloads() {
    let mut m = Mouse::new(FakeOs::new(), AppDirs::new("unused"));
    let before = m.cursor_snapshot().unwrap();
    m.preview_cursor(Role::Normal, &SetId::WindowsDefault).unwrap();
    m.end_preview().unwrap();
    assert_eq!(m.cursor_snapshot().unwrap(), before, "a preview writes nothing");
    assert_eq!(m.os().log, vec!["set_system_cursor 32512 C:\\Windows\\cursors\\aero_arrow.cur".to_string(), "reload_cursors".to_string()]);
}

#[test]
fn reapply_repushes_every_role() {
    let mut m = Mouse::new(FakeOs::new(), AppDirs::new("unused"));
    m.reapply_cursors().unwrap();
    let pushes = m.os().log.iter().filter(|l| l.starts_with("set_system_cursor")).count();
    assert_eq!(pushes, 15, "17 roles minus the two empty (built-in) ones");
}

#[test]
fn helpers() {
    assert!(is_cursor_bytes(&[0, 0, 2, 0, 1, 0]));
    assert!(!is_cursor_bytes(&[0, 0, 1, 0, 1, 0]), "an .ico is not a cursor");
    assert!(is_cursor_bytes(b"RIFF\0\0\0\0ACON"));
    let p = parse_scheme("a.cur,,b.cur,@main.cpl,-1020");
    assert_eq!(p.len(), 17);
    assert_eq!((p[0].as_str(), p[1].as_str(), p[2].as_str(), p[3].as_str()), ("a.cur", "", "b.cur", "@main.cpl"));
    let p = parse_scheme(&vec!["x.cur"; 19].join(","));
    assert_eq!(p.len(), 17, "extra items (display-name resource) ignored");
}

#[test]
fn admin_denied_write_is_typed_and_not_remembered() {
    let mut m = Mouse::new(FakeOs::new(), AppDirs::new("unused"));
    m.os_mut().deny_writes = 1;
    assert!(matches!(m.set_scheme("Windows Black"), Err(Error::NeedsAdmin { .. })));
    assert!(matches!(m.undo_cursors(), Err(Error::NothingToUndo(_))));
}

/// Order 042: the schemes Windows has installed are sets of their own - listed (all but the one that is just Windows'
/// default again), picked for one bubble, and recognised afterwards.
#[test]
fn installed_schemes_are_sets_for_the_pickers() {
    let mut m = Mouse::new(FakeOs::new(), AppDirs::new("unused"));
    let list = m.installed_schemes().unwrap();
    let names: Vec<&str> = list.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["Windows Black"], "Windows Aero = the default files again: not listed");
    assert_eq!(list[0].1, Role::ALL.to_vec());
    let black = SetId::Scheme("Windows Black".into());
    assert!(m.set_has(&black, Role::Normal));
    m.set_role(Role::Normal, black.clone()).unwrap();
    assert_eq!(reg(&m, "Arrow"), Some(RegValue::ExpandSz(r"%SystemRoot%\cursors\arrow_r.cur".into())));
    let st = m.cursors().unwrap();
    assert_eq!(st.roles.iter().find(|r| r.role == Role::Normal).map(|r| r.set.clone()), Some(black.clone()));
    assert_eq!(st.roles.iter().find(|r| r.role == Role::Text).map(|r| r.set.clone()), Some(SetId::WindowsDefault));
    // the whole scheme switched in Control Panel: every bubble shows it
    m.set_scheme("Windows Black").unwrap();
    assert!(m.cursors().unwrap().roles.iter().all(|r| r.set == black), "{:?}", m.cursors().unwrap().roles);
    assert!(matches!(m.set_role(Role::Normal, SetId::Scheme("Nope".into())), Err(Error::NotFound(_))));
}
