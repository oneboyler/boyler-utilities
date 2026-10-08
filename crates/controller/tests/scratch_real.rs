//! The REAL file layer on a Steam-shaped tree inside the lane's scratch folder (never the real Steam folder): write, the rest
//! byte-identical on disk, backup, undo, the write guard. The folder is made and removed by the test.
#![cfg(windows)]

mod common;

use bu_controller::real::RealSteam;
use bu_controller::settings::{Change, StickSetting};
use bu_controller::steam::SteamPaths;
use bu_controller::{ControllerService, Error, PadKind, Side, SteamOs};
use common::*;
use std::path::{Path, PathBuf};

const SCRATCH: &str = r"C:\BoylerUtilities-scratch\L";

/// A fresh folder inside the scratch folder; `None` (test skipped) when the scratch folder is not on this PC.
fn scratch_dir(name: &str) -> Option<PathBuf> {
    let root = Path::new(SCRATCH);
    if !root.is_dir() {
        eprintln!("scratch folder {SCRATCH} missing: skipped");
        return None;
    }
    let d = root.join(format!("test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Some(d)
}

/// Copy the fake Steam tree (C:\Steam\… → <dir>\steam\…, D:\… → <dir>\lib2\…) onto disk.
fn build_tree(dir: &Path) -> PathBuf {
    let steam = dir.join("steam");
    for (p, bytes) in files() {
        let s = p.to_string_lossy().to_string();
        let target = if let Some(rest) = s.strip_prefix(r"C:\Steam\") { steam.join(rest) } else { dir.join("lib2").join(&s[3..]) };
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, bytes).unwrap();
    }
    // the library list points INTO the scratch tree (never at a real D:\SteamLibrary)
    let lf = steam.join(r"steamapps\libraryfolders.vdf");
    let esc = |p: &Path| p.to_string_lossy().replace('\\', r"\\");
    let t = std::fs::read_to_string(&lf).unwrap().replace(r"C:\\Steam", &esc(&steam)).replace(r"D:\\SteamLibrary", &esc(&dir.join(r"lib2\SteamLibrary")));
    assert!(!t.contains(r"D:\\"), "{t}");
    std::fs::write(&lf, t).unwrap();
    steam
}

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn real_files_in_scratch_write_backup_undo() {
    let Some(dir) = scratch_dir("files") else { return };
    let _c = Cleanup(dir.clone());
    let steam = build_tree(&dir);
    let os = RealSteam::scratch(&steam, Path::new(SCRATCH)).unwrap();
    let paths = SteamPaths { dir: steam.clone(), account: ACCOUNT.into() };
    let backups = dir.join("backups");
    let mut s = ControllerService::with_paths(os, paths, &backups);
    let names: Vec<String> = s.games(PadKind::DualSenseEdge).unwrap().into_iter().map(|g| g.name).collect();
    assert!(names.contains(&"Yakuza 0".to_string()), "found in the second library INSIDE the scratch tree: {names:?}");
    let rl = steam.join(r"steamapps\common\Steam Controller Configs").join(ACCOUNT).join(r"config\252950\controller_ps5.vdf");
    let before = std::fs::read(&rl).unwrap();

    s.apply("252950", PadKind::DualSenseEdge, 0, &Change::StickSetting { side: Side::Left, setting: StickSetting::DeadZone, value: Some(1000) }).unwrap();
    let after = std::fs::read_to_string(&rl).unwrap();
    let (rem, add) = line_diff(&String::from_utf8(before.clone()).unwrap(), &after);
    assert_eq!(rem, vec!["\t\t\t\"deadzone_inner_radius\"\t\t\"3357\"\n".to_string()]);
    assert_eq!(add, vec!["\t\t\t\"deadzone_inner_radius\"\t\t\"1000\"\n".to_string()]);
    assert!(!rl.with_file_name("controller_ps5.vdf.bu-tmp").exists(), "no temp file left behind");
    let orig = backups.join(ACCOUNT).join(r"steamapps\common\Steam Controller Configs").join(ACCOUNT).join(r"config\252950\controller_ps5.vdf.original");
    assert_eq!(std::fs::read(&orig).unwrap(), before, "backup on disk before the first write");

    s.undo().unwrap();
    assert_eq!(std::fs::read(&rl).unwrap(), before, "undo = byte-identical on disk");

    // the community layout's own copy is created on disk, and undo removes it again
    s.apply("638970", PadKind::DualSenseEdge, 0, &Change::StickSetting { side: Side::Right, setting: StickSetting::Sensitivity, value: Some(200) }).unwrap();
    let own = rl.parent().unwrap().parent().unwrap().join(r"638970\controller_ps5.vdf");
    assert!(own.exists());
    s.undo().unwrap();
    assert!(!own.exists());
}

#[test]
fn the_write_guard_refuses_outside_the_allowed_folder() {
    let Some(dir) = scratch_dir("guard") else { return };
    let _c = Cleanup(dir.clone());
    // the guard's root is an INNER folder; the "outside" targets are missing paths still inside scratch\L, so even a
    // broken guard could not write anywhere real
    let inner = dir.join("inner");
    std::fs::create_dir_all(&inner).unwrap();
    let os = RealSteam::scratch(inner.join("steam"), &inner).unwrap();
    let outside = dir.join("outside-missing").join("x.vdf");
    assert!(matches!(os.write(&outside, b"x"), Err(Error::OutsideScratch(_))));
    assert!(!outside.exists());
    assert!(matches!(os.write(&inner.join(r"..\outside-missing\escape.vdf"), b"x"), Err(Error::OutsideScratch(_))));
    let dir = inner;
    assert!(matches!(RealSteam::scratch(r"C:\Program Files (x86)\Steam", Path::new(SCRATCH)), Err(Error::OutsideScratch(_))), "never the user's real Steam");
    os.write(&dir.join("ok.vdf"), b"x").unwrap();
    assert_eq!(std::fs::read(dir.join("ok.vdf")).unwrap(), b"x");
    os.remove(&dir.join("ok.vdf")).unwrap();
}
