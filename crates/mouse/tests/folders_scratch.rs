//! Folder rules in the lane's scratch folder: the Raw Accel search looks only into folders NAMED RawAccel*, never into
//! any other folder of a root (other folders are never touched); pack names can't be paths.

use bu_mouse::accel::service::{find_rawaccel_dir, is_rawaccel_name};
use bu_mouse::cursors::is_plain_folder_name;
use bu_mouse::fake::FakeOs;
use bu_mouse::{AppDirs, Error, Mouse};
use std::path::{Path, PathBuf};

const SCRATCH_PARENT: &str = r"C:\BoylerUtilities-scratch";

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

fn fake_ra(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("rawaccel.exe"), "x").unwrap();
    std::fs::write(dir.join("writer.exe"), "x").unwrap();
}

#[test]
fn raw_accel_is_found_only_in_rawaccel_named_folders() {
    let Some(s) = scratch("find-ra") else { return };
    let root = s.0.join("Desktop");
    // a Raw Accel copy inside some OTHER project folder must never be found (= that folder is never looked into)
    fake_ra(&root.join("SomeProject"));
    assert_eq!(find_rawaccel_dir(std::slice::from_ref(&root)), None);
    fake_ra(&root.join("RawAccel-1.7.0"));
    assert_eq!(find_rawaccel_dir(std::slice::from_ref(&root)), Some(root.join("RawAccel-1.7.0")));
    // the root itself can be the folder
    assert_eq!(find_rawaccel_dir(&[root.join("RawAccel-1.7.0")]), Some(root.join("RawAccel-1.7.0")));
    assert!(is_rawaccel_name("RawAccel") && is_rawaccel_name("rawaccel_v1.7") && is_rawaccel_name("Raw Accel"));
    assert!(!is_rawaccel_name("MyProject") && !is_rawaccel_name("MyRawAccel") && !is_rawaccel_name("Raw"));
}

#[test]
fn pack_names_cannot_be_paths() {
    for bad in ["..", ".", "...", ". .", "x.", " x", "x ", "", "  ", r"..\x", "a/b", "C:", "x?"] {
        assert!(!is_plain_folder_name(bad), "{bad:?}");
    }
    assert!(is_plain_folder_name("Neon Pack 2"));
    let mut m = Mouse::new(FakeOs::new(), AppDirs::new(r"C:\BoylerUtilities-scratch\lane-g\never"));
    assert!(matches!(m.delete_pack(".."), Err(Error::BadName { .. })));
    assert!(matches!(m.delete_pack(r"..\cursors"), Err(Error::BadName { .. })));
}
