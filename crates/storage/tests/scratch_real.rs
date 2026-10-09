//! The real Windows file calls (list, delete) proven ONLY inside a scratch folder this test creates and removes:
//! `C:\BoylerUtilities-scratch\lane-e\<test>-<pid>\` — nowhere else. When the board's
//! scratch folder doesn't exist (another PC, board cleaned up) the tests are SKIPPED with a message; they never fall
//! back to another folder. A guard removes the folder (its junctions first) even when an assert fails.
//! A fake recycle bin / temp / caches tree, an in-use file and a junction to "precious" files. Nothing of the owner's.
#![cfg(windows)]

use bu_storage::classify::ClassRules;
use bu_storage::cleanup::{self, CleanKind};
use bu_storage::scan::{self, RowKind, ScanControl, ScanOptions};
use bu_storage::*;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const BOARD_SCRATCH: &str = r"C:\BoylerUtilities-scratch";

/// This test's own folder under `...\scratch\lane-e\`; removed on drop (junctions first, never followed).
struct Scratch {
    root: PathBuf,
    links: Vec<PathBuf>,
}

impl Scratch {
    fn new(name: &str) -> Option<Scratch> {
        let board = PathBuf::from(BOARD_SCRATCH);
        if !board.is_dir() {
            eprintln!("SKIPPED: {BOARD_SCRATCH} does not exist; this test only ever writes under {BOARD_SCRATCH}\\lane-e");
            return None;
        }
        let lane = board.join("lane-e");
        let root = lane.join(format!("{name}-{}", std::process::id()));
        assert!(root.starts_with(&lane));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Some(Scratch { root, links: Vec::new() })
    }

    fn junction(&mut self, link: &Path, target: &Path) {
        assert!(link.starts_with(&self.root) && target.starts_with(&self.root));
        let st = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(link).arg(target).output().unwrap();
        assert!(st.status.success(), "mklink failed: {}", String::from_utf8_lossy(&st.stderr));
        self.links.push(link.to_path_buf());
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        for l in &self.links {
            let _ = fs::remove_dir(l); // removes the junction itself only
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// RealOs for files; everything else points into the scratch folder.
struct ScratchOs {
    real: RealOs,
    root: PathBuf,
    running: Mutex<Vec<String>>,
}

impl ScratchOs {
    fn bin(&self) -> PathBuf {
        self.root.join("RecycleBin")
    }
}

impl StorageOs for ScratchOs {
    fn drives(&self) -> Result<Vec<DriveInfo>> {
        Ok(vec![])
    }
    fn read_dir(&self, path: &Path) -> io::Result<Vec<RawEntry>> {
        assert!(path.starts_with(&self.root), "listing outside scratch: {}", path.display());
        self.real.read_dir(path)
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        assert!(path.starts_with(&self.root), "delete outside scratch: {}", path.display());
        self.real.remove_file(path)
    }
    fn remove_dir(&self, path: &Path) -> io::Result<()> {
        assert!(path.starts_with(&self.root), "delete outside scratch: {}", path.display());
        self.real.remove_dir(path)
    }
    fn recycle_file(&self, path: &Path) -> io::Result<()> {
        // never the real shell here: the scratch "bin" is a folder
        assert!(path.starts_with(&self.root), "recycle outside scratch: {}", path.display());
        let to = self.bin().join(path.file_name().unwrap());
        fs::create_dir_all(self.bin())?;
        fs::rename(path, to)
    }
    fn recycle_bin(&self) -> Result<RecycleBinInfo> {
        let mut info = RecycleBinInfo::default();
        for e in self.real.read_dir(&self.bin())? {
            info.bytes += e.size;
            info.items += 1;
        }
        Ok(info)
    }
    fn empty_recycle_bin(&self) -> Result<()> {
        for e in self.real.read_dir(&self.bin())? {
            self.real.remove_file(&self.bin().join(e.name))?;
        }
        Ok(())
    }
    fn known_dirs(&self) -> KnownDirs {
        let r = &self.root;
        KnownDirs {
            user_temp: Some(r.join(r"Users\J\AppData\Local\Temp")),
            windows_temp: Some(r.join(r"Windows\Temp")),
            local_appdata: Some(r.join(r"Users\J\AppData\Local")),
            local_low: Some(r.join(r"Users\J\AppData\LocalLow")),
            program_data: Some(r.join("ProgramData")),
            windows: Some(r.join("Windows")),
            program_files: vec![r.join("Program Files")],
            steam: Some(r.join(r"Program Files\Steam")),
        }
    }
    fn running_exe_names(&self) -> Vec<String> {
        self.running.lock().unwrap().clone()
    }
    fn game_roots(&self) -> Vec<PathBuf> {
        vec![self.root.join(r"Program Files\Steam\steamapps")]
    }
    fn is_elevated(&self) -> bool {
        false
    }
    fn physical_disks(&self) -> Result<Vec<PhysicalDisk>> {
        Ok(vec![])
    }
    fn disk_health(&self, n: u32) -> Result<HealthRaw> {
        Err(StorageError::NotFound(format!("disk {n}")))
    }
}

fn write(p: &Path, bytes: usize) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![7u8; bytes]).unwrap();
}

#[test]
fn real_clean_in_scratch_only_ticked_in_use_stays_junction_not_followed() {
    use std::os::windows::fs::OpenOptionsExt;
    let Some(mut scratch) = Scratch::new("clean") else { return };
    let root = scratch.root.clone();
    let os = ScratchOs { real: RealOs::new(), root: root.clone(), running: Mutex::new(vec![]) };
    let local = root.join(r"Users\J\AppData\Local");
    write(&os.bin().join("$R1.txt"), 3000);
    write(&os.bin().join("$R2.txt"), 2000);
    write(&local.join(r"Temp\a.tmp"), 1000);
    write(&local.join(r"Temp\deep\b.tmp"), 500);
    write(&local.join(r"Temp\locked.tmp"), 700);
    write(&root.join(r"Precious\keep.docx"), 4242);
    write(&local.join(r"NVIDIA\DXCache\s.bin"), 800);
    write(&local.join(r"Steam\htmlcache\Default\Cache\c.bin"), 900);
    write(&root.join(r"Program Files\Steam\appcache\appinfo.vdf"), 600);
    // A junction inside Temp pointing at the precious folder (junctions need no admin).
    let link = local.join(r"Temp\link");
    scratch.junction(&link, &root.join("Precious"));
    // Hold locked.tmp open with no sharing, like an app using it.
    let held = fs::OpenOptions::new().read(true).share_mode(0).open(local.join(r"Temp\locked.tmp")).unwrap();
    *os.running.lock().unwrap() = vec!["steam.exe".into()];

    let plan = cleanup::measure(&os).unwrap();
    assert_eq!(plan.row(CleanKind::RecycleBin).unwrap().bytes, 5000);
    assert_eq!(plan.row(CleanKind::TempFiles).unwrap().bytes, 2200, "the junction's target is not counted");
    assert_eq!(plan.row(CleanKind::ShaderCaches).unwrap().bytes, 800);
    assert_eq!(plan.row(CleanKind::LauncherCaches).unwrap().bytes, 0, "Steam runs");
    assert_eq!(plan.row(CleanKind::LauncherCaches).unwrap().blocked_bytes, 1500);

    let report = plan.clean(&os, &[CleanKind::RecycleBin, CleanKind::TempFiles, CleanKind::LauncherCaches]).unwrap();
    drop(held);
    assert!(!os.bin().join("$R1.txt").exists());
    assert!(!local.join(r"Temp\a.tmp").exists());
    assert!(!local.join(r"Temp\deep").exists(), "emptied sub-folder removed");
    assert!(local.join(r"Temp\locked.tmp").exists(), "in-use file stays");
    assert!(local.join("Temp").is_dir(), "the temp folder itself stays");
    assert!(link.exists(), "junction left alone");
    assert_eq!(fs::read(root.join(r"Precious\keep.docx")).unwrap().len(), 4242, "junction target untouched");
    assert!(local.join(r"NVIDIA\DXCache\s.bin").exists(), "shader caches were not ticked");
    assert!(local.join(r"Steam\htmlcache\Default\Cache\c.bin").exists(), "Steam was running");
    let temp = report.rows.iter().find(|r| r.kind == Some(CleanKind::TempFiles)).unwrap();
    assert_eq!((temp.freed_bytes, temp.in_use_bytes, temp.in_use_files), (1500, 700, 1));
    assert_eq!(report.rows.iter().find(|r| r.kind == Some(CleanKind::RecycleBin)).unwrap().freed_bytes, 5000);

    // Steam closed → its caches go.
    os.running.lock().unwrap().clear();
    let plan = cleanup::measure(&os).unwrap();
    plan.clean(&os, &[CleanKind::LauncherCaches]).unwrap();
    assert!(!local.join(r"Steam\htmlcache\Default\Cache\c.bin").exists());
    assert!(!root.join(r"Program Files\Steam\appcache\appinfo.vdf").exists());

}

#[test]
fn real_walk_in_scratch_long_paths_links_and_types() {
    let Some(mut scratch) = Scratch::new("walk") else { return };
    let root = scratch.root.clone();
    let os = ScratchOs { real: RealOs::new(), root: root.clone(), running: Mutex::new(vec![]) };
    write(&root.join(r"Program Files\Steam\steamapps\common\G\data.pak"), 10_000);
    write(&root.join(r"Program Files\Tool\tool.exe"), 3000);
    write(&root.join(r"Users\J\Videos\v.mp4"), 5000);
    write(&root.join(r"Users\J\Pictures\p.png"), 2000);
    write(&root.join(r"Users\J\notes.txt"), 100);
    // A path well past 260 characters.
    let mut deep = root.join("Deep");
    for i in 0..12 {
        deep.push(format!("a-rather-long-folder-name-number-{i:02}"));
    }
    write(&deep.join("far.mkv"), 777);
    assert!(deep.to_string_lossy().len() > 300);
    let link = root.join("LoopLink");
    scratch.junction(&link, &root);

    let rules = ClassRules::from_os(&os);
    let ctl = ScanControl::new();
    let r = scan::scan_folder(&os, &root, &rules, ScanOptions { threads: 4, files_per_folder: 24 }, &ctl).unwrap();
    use bu_storage::classify::FileType::*;
    let w = |t: bu_storage::classify::FileType| r.types.walked[t.index()];
    assert_eq!(w(Games), 10_000);
    assert_eq!(w(Apps), 3000);
    assert_eq!(w(Videos), 5000 + 777);
    assert_eq!(w(Pictures), 2000);
    assert_eq!(w(Documents), 100);
    assert_eq!(r.stats.links_skipped, 1, "the loop junction was not followed");
    assert_eq!(r.tree.size(0).unwrap(), 10_000 + 3000 + 5000 + 2000 + 100 + 777);
    let top = r.tree.rows(0).unwrap();
    assert_eq!(top[0].name, "Program Files");
    let deep_id = r.tree.find(&deep).unwrap();
    let rows = r.tree.rows(deep_id).unwrap();
    assert_eq!((rows[0].name.as_str(), rows[0].bytes, rows[0].kind), ("far.mkv", 777, RowKind::File));

}

/// What keeping a walk's folder tree costs (boss call, Oct 8): a scratch tree of 2,000 folders x 5 files, walked by the real
/// code, its heap bytes full and pruned. Run on purpose: `cargo test -p bu-storage --test scratch_real tree_ram -- --ignored --nocapture`.
#[test]
#[ignore]
fn tree_ram_on_a_scratch_tree() {
    let Some(scratch) = Scratch::new("treeram") else { return };
    let root = scratch.root.clone();
    let os = ScratchOs { real: RealOs::new(), root: root.clone(), running: Mutex::new(vec![]) };
    for a in 0..40 {
        for b in 0..50 {
            let d = root.join(format!("folder-{a:02}")).join(format!("sub-folder-{b:02}"));
            fs::create_dir_all(&d).unwrap();
            for f in 0..5 {
                fs::write(d.join(format!("file-{f}.dat")), [0u8; 16]).unwrap();
            }
        }
    }
    let rules = ClassRules::from_os(&os);
    let r = scan::scan_folder(&os, &root, &rules, ScanOptions { threads: 4, files_per_folder: 24 }, &ScanControl::new()).unwrap();
    let (full, pruned) = (r.tree.heap_bytes(), r.tree.pruned().heap_bytes());
    println!("tree_ram: {} folders, {} files: full {} bytes ({:.1} per folder), pruned {} bytes", r.stats.folders, r.stats.files, full, full as f64 / r.stats.folders.max(1) as f64, pruned);
    assert!(pruned < full);
}
