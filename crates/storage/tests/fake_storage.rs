//! Every Storage row against the fake OS (nothing real is touched).

use bu_storage::classify::{by_extension, parse_epic_manifest, parse_steam_libraries, ClassRules, FileType};
use bu_storage::cleanup::{self, is_safe_target, CleanKind, CleanPlan, PartState};
use bu_storage::health::{self, temp_level, TempLevel};
use bu_storage::scan::{self, RowKind, ScanControl, ScanOptions};
use bu_storage::*;
use std::path::{Path, PathBuf};

const GB: u64 = 1 << 30;
const MB: u64 = 1 << 20;

fn drive(letter: char, kind: DriveKind, total: u64, free: u64) -> DriveInfo {
    DriveInfo {
        letter,
        kind,
        state: VolumeState::Ready,
        label: String::new(),
        file_system: "NTFS".into(),
        total_bytes: total,
        free_bytes: free,
        is_system: letter == 'C',
        model: Some(format!("Model {letter}")),
        media: MediaKind::Ssd,
        bus: Some("NVMe".into()),
        disk_number: Some(letter as u32 - 'C' as u32),
    }
}

fn known() -> KnownDirs {
    KnownDirs {
        user_temp: Some(PathBuf::from(r"C:\Users\J\AppData\Local\Temp")),
        windows_temp: Some(PathBuf::from(r"C:\Windows\Temp")),
        local_appdata: Some(PathBuf::from(r"C:\Users\J\AppData\Local")),
        local_low: Some(PathBuf::from(r"C:\Users\J\AppData\LocalLow")),
        program_data: Some(PathBuf::from(r"C:\ProgramData")),
        windows: Some(PathBuf::from(r"C:\Windows")),
        program_files: vec![PathBuf::from(r"C:\Program Files"), PathBuf::from(r"C:\Program Files (x86)")],
        steam: Some(PathBuf::from(r"C:\Program Files (x86)\Steam")),
    }
}

// ---------------------------------------------------------------- drives

#[test]
fn drive_tiles_list_local_drives_only_with_low_space_and_default_c() {
    let os = FakeOs::new();
    let mut usb = drive('F', DriveKind::Removable, 32 * GB, 30 * GB);
    usb.label = "STICK".into();
    let mut locked = drive('H', DriveKind::Fixed, 0, 0);
    locked.state = VolumeState::Locked;
    os.with_drives(vec![
        drive('D', DriveKind::Fixed, 1000 * GB, 50 * GB), // 5 % free → amber
        drive('C', DriveKind::Fixed, 2000 * GB, 612 * GB),
        drive('Z', DriveKind::Network, 100 * GB, 10 * GB),
        drive('X', DriveKind::CdRom, 0, 0),
        usb,
        locked,
    ]);
    let tiles = drives::list(&os).unwrap();
    let letters: Vec<char> = tiles.iter().map(|t| t.info.letter).collect();
    assert_eq!(letters, vec!['C', 'D', 'F', 'H'], "network + CD drives are not listed");
    assert!(!tiles[0].low_space);
    assert!(tiles[1].low_space);
    assert!(tiles[2].removable);
    assert_eq!(tiles[3].info.state, VolumeState::Locked);
    assert_eq!(tiles[3].used_fraction, 0.0);
    assert_eq!(drives::default_choice(&tiles), Some('C'));
    assert_eq!(tiles[0].title(), "C:  Local Disk");
    assert_eq!(tiles[2].title(), "F:  STICK");
    assert_eq!(tiles[0].hover(), "Model C · NVMe SSD");
    assert_eq!(drives::format_bytes(612 * GB), "612 GB");
    assert_eq!(drives::format_bytes(2_000_000_000_000), "1.82 TB");
    assert_eq!(drives::format_bytes(205 * MB), "205 MB");
    assert_eq!(drives::format_bytes(512), "512 B");
}

// ---------------------------------------------------------------- classify

#[test]
fn file_types_by_extension() {
    assert_eq!(by_extension("clip.MP4"), FileType::Videos);
    assert_eq!(by_extension("a.png"), FileType::Pictures);
    assert_eq!(by_extension("tax.pdf"), FileType::Documents);
    assert_eq!(by_extension("setup.exe"), FileType::Apps);
    assert_eq!(by_extension("pagefile.sys"), FileType::WindowsOther);
    assert_eq!(by_extension("song.mp3"), FileType::WindowsOther);
    assert_eq!(by_extension("noext"), FileType::WindowsOther);
}

#[test]
fn steam_and_epic_parsers() {
    let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}";
    assert_eq!(
        parse_steam_libraries(vdf),
        vec![PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps"), PathBuf::from(r"D:\SteamLibrary\steamapps")]
    );
    let item = r#"{ "DisplayName": "Fortnite", "InstallLocation": "C:\\Program Files\\Epic Games\\Fortnite", "x": 1 }"#;
    assert_eq!(parse_epic_manifest(item), Some(PathBuf::from(r"C:\Program Files\Epic Games\Fortnite")));
    assert_eq!(parse_epic_manifest("{}"), None);
}

// ---------------------------------------------------------------- scan

fn scan_fixture() -> FakeOs {
    let os = FakeOs::new();
    os.with_drives(vec![drive('C', DriveKind::Fixed, 1000 * GB, 400 * GB)]);
    os.with_known(known());
    os.with_game_roots(vec![PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps")]);
    os.add_file(r"C:\pagefile.sys", 16 * GB)
        .add_file(r"C:\Windows\System32\ntoskrnl.exe", 10 * GB)
        .add_file(r"C:\Program Files\App\app.exe", 2 * GB)
        .add_file(r"C:\Program Files\App\intro.mp4", GB) // inside Apps → Apps, not Videos
        .add_file(r"C:\Program Files (x86)\Steam\steamapps\common\Game\data.pak", 100 * GB)
        .add_file(r"C:\Program Files (x86)\Steam\steam.exe", MB) // Program Files → Apps
        .add_file(r"C:\Users\J\Videos\a.mp4", 50 * GB)
        .add_file(r"C:\Users\J\Pictures\p.jpg", 5 * GB)
        .add_file(r"C:\Users\J\Documents\d.pdf", 3 * GB)
        .add_file(r"C:\Users\J\Downloads\setup.exe", GB)
        .add_file(r"C:\Users\J\Downloads\song.mp3", GB)
        .add_cloud_file(r"C:\Users\J\OneDrive\big.mkv", 70 * GB) // online-only → 0
        .add_junction(r"C:\Users\J\Link") // never followed
        .set_unreadable(r"C:\System Volume Information")
        .add_file(r"C:\$Recycle.Bin\S-1\$R1.mp4", 4 * GB);
    for i in 0..30 {
        os.add_file(format!(r"C:\Users\J\Small\f{i:02}.txt"), (i + 1) as u64 * 1000);
    }
    os
}

#[test]
fn scan_types_folders_locks_and_links() {
    let os = scan_fixture();
    let ctl = ScanControl::new();
    let r = scan::scan_drive(&os, 'c', &ctl).unwrap();
    let w = |t: FileType| r.types.walked[t.index()];
    assert_eq!(w(FileType::Games), 100 * GB);
    assert_eq!(w(FileType::Videos), 50 * GB + 4 * GB); // the recycle-bin .mp4 counts as a video
    assert_eq!(w(FileType::Apps), 2 * GB + GB + MB + GB);
    assert_eq!(w(FileType::Pictures), 5 * GB);
    assert_eq!(w(FileType::Documents), 3 * GB + (1..=30).map(|i| i * 1000).sum::<u64>());
    assert_eq!(w(FileType::WindowsOther), 16 * GB + 10 * GB + GB);
    assert_eq!(r.stats.links_skipped, 1);
    assert_eq!(r.stats.unreadable_folders, 1);
    assert_eq!(ctl.progress().files, r.stats.files);

    // The bar adds up to "used" exactly: Windows & other = used − the five named types.
    let rows = r.types.rows();
    assert_eq!(rows.last().unwrap().ty, FileType::WindowsOther);
    let total: u64 = rows.iter().map(|x| x.bytes).sum();
    assert_eq!(total, 600 * GB);
    assert_eq!(rows[0].ty, FileType::Games);
    assert!(r.types.unseen_bytes().unwrap() > 0);

    // Folders: biggest first, Windows' own places locked, unreadable marked.
    let tree = &r.tree;
    let top = tree.rows(tree.root()).unwrap();
    assert_eq!(top[0].name, "Program Files (x86)");
    let names: Vec<&str> = top.iter().map(|x| x.name.as_str()).collect();
    assert!(names.contains(&"pagefile.sys"));
    let win = top.iter().find(|x| x.name == "Windows").unwrap();
    let RowKind::Folder { id: win_id, windows_own, .. } = win.kind else { panic!() };
    assert!(windows_own);
    assert!(matches!(tree.rows(win_id), Err(StorageError::WindowsOwn)));
    let svi = top.iter().find(|x| x.name == "System Volume Information").unwrap();
    assert!(matches!(svi.kind, RowKind::Folder { unreadable: true, windows_own: true, .. }));
    assert!(top.windows(2).all(|p| p[0].bytes >= p[1].bytes), "biggest first");

    // Drill down, breadcrumb, path, back up.
    let users = tree.find(Path::new(r"C:\users\j")).unwrap();
    let crumb = tree.breadcrumb(users).unwrap();
    assert_eq!(crumb.iter().map(|c| c.1.as_str()).collect::<Vec<_>>(), vec!["C:", "Users", "J"]);
    assert_eq!(tree.path(users).unwrap(), PathBuf::from(r"C:\Users\J"));
    assert_eq!(tree.size(users).unwrap(), 50 * GB + 5 * GB + 3 * GB + 2 * GB + (1..=30).map(|i| i * 1000).sum::<u64>());
    let j_rows = tree.rows(users).unwrap();
    assert_eq!(j_rows[0].name, "Videos");
    assert!(matches!(j_rows[0].kind, RowKind::Folder { has_subfolders: false, .. }));
    assert!((j_rows[0].share - (50 * GB) as f64 / tree.size(users).unwrap() as f64).abs() < 1e-9);
    assert!(!j_rows.iter().any(|x| x.name == "Link"), "junctions are not listed");
    assert_eq!(tree.parent(users).unwrap(), tree.find(Path::new(r"C:\Users")));
    assert_eq!(tree.parent(tree.root()).unwrap(), None);
}

#[test]
fn scan_groups_small_files_into_other_files_row() {
    let os = scan_fixture();
    let rules = ClassRules::from_os(&os);
    let opts = ScanOptions { threads: 3, files_per_folder: 5 };
    let r = scan::scan_folder(&os, Path::new(r"C:\Users\J\Small"), &rules, opts, &ScanControl::new()).unwrap();
    let rows = r.tree.rows(0).unwrap();
    assert_eq!(rows.len(), 6);
    assert_eq!(rows[0].name, "f29.txt");
    assert_eq!(rows[5].kind, RowKind::OtherFiles { count: 25 });
    assert_eq!(rows[5].bytes, (1..=25).map(|i| i * 1000).sum::<u64>());
    assert_eq!(r.types.used_bytes, None);
    assert_eq!(r.tree.name(0).unwrap(), "Small");
}

#[test]
fn scan_refuses_locked_drive_and_honours_cancel() {
    let os = scan_fixture();
    let mut locked = drive('E', DriveKind::Fixed, 0, 0);
    locked.state = VolumeState::Locked;
    os.with_drives(vec![drive('C', DriveKind::Fixed, 1000 * GB, 400 * GB), locked]);
    assert!(matches!(scan::scan_drive(&os, 'E', &ScanControl::new()), Err(StorageError::Locked('E'))));
    assert!(matches!(scan::scan_drive(&os, 'Q', &ScanControl::new()), Err(StorageError::NotFound(_))));
    let ctl = ScanControl::new();
    ctl.cancel();
    assert!(matches!(scan::scan_drive(&os, 'C', &ctl), Err(StorageError::Cancelled)));
}

#[test]
fn windows_own_places() {
    assert!(scan::is_windows_own(Path::new(r"C:\Windows")));
    assert!(scan::is_windows_own(Path::new(r"d:\$RECYCLE.BIN\")));
    assert!(scan::is_windows_own(Path::new(r"C:\Program Files\WindowsApps")));
    assert!(scan::is_windows_own(Path::new(r"E:\System Volume Information")));
    assert!(!scan::is_windows_own(Path::new(r"C:\Windows\Temp")));
    assert!(!scan::is_windows_own(Path::new(r"C:\Users")));
}

// ---------------------------------------------------------------- cleanup

fn clean_fixture() -> FakeOs {
    let os = FakeOs::new();
    os.with_known(known()).with_recycle_bin(3 * GB, 12);
    let l = r"C:\Users\J\AppData\Local";
    os.add_file(format!(r"{l}\Temp\a.tmp"), 100 * MB)
        .add_file(format!(r"{l}\Temp\sub\b.tmp"), 50 * MB)
        .add_file_in_use(format!(r"{l}\Temp\sub\locked.tmp"), 205 * MB)
        .add_junction(format!(r"{l}\Temp\link-to-docs"))
        .add_file(r"C:\Users\J\Documents\precious.docx", MB)
        .add_file(r"C:\Windows\Temp\w.tmp", 70 * MB)
        .add_file(format!(r"{l}\D3DSCache\x\1.bin"), 10 * MB)
        .add_file(format!(r"{l}\NVIDIA\DXCache\2.bin"), 20 * MB)
        .add_file(r"C:\Users\J\AppData\LocalLow\NVIDIA\PerDriverVersion\DXCache\3.bin", 30 * MB)
        .add_file(format!(r"{l}\AMD\VkCache\4.bin"), 5 * MB)
        .add_file(r"C:\Program Files (x86)\Steam\appcache\appinfo.vdf", 40 * MB)
        .add_file(format!(r"{l}\Steam\htmlcache\Cache\c1"), 60 * MB)
        .add_file(format!(r"{l}\EpicGamesLauncher\Saved\webcache_4430\e1"), 8 * MB)
        .add_file(format!(r"{l}\EpicGamesLauncher\Saved\Config\keep.ini"), 1000)
        .add_file(format!(r"{l}\Riot Games\Riot Client\HttpCache\r1"), 9 * MB);
    os.set_unreadable(r"C:\Windows\Temp");
    os
}

#[test]
fn cleanup_measures_sizes_first_with_blocked_parts() {
    let os = clean_fixture();
    os.with_running(&["Steam.exe", "explorer.exe"]);
    let plan = cleanup::measure(&os).unwrap();
    assert!(os.changes().is_empty(), "measuring changes nothing");
    let rb = plan.row(CleanKind::RecycleBin).unwrap();
    assert_eq!((rb.bytes, rb.items), (3 * GB, 12));
    let temp = plan.row(CleanKind::TempFiles).unwrap();
    assert_eq!(temp.bytes, 100 * MB + 50 * MB + 205 * MB);
    let win_temp = temp.parts.iter().find(|p| p.name == "Windows temp folder").unwrap();
    assert_eq!(win_temp.state, PartState::NeedsAdmin);
    assert!(temp.notes().contains(&"Windows temp folder needs admin".to_string()));
    let shader = plan.row(CleanKind::ShaderCaches).unwrap();
    assert_eq!(shader.bytes, 65 * MB);
    assert!(shader.parts.iter().any(|p| p.name == "AMD DirectX cache" && p.state == PartState::Missing));
    let launch = plan.row(CleanKind::LauncherCaches).unwrap();
    assert_eq!(launch.bytes, 8 * MB + 9 * MB, "Steam runs → its caches are blocked");
    assert_eq!(launch.blocked_bytes, 100 * MB);
    assert_eq!(launch.notes(), vec!["Close Steam first".to_string()]);
    assert_eq!(plan.default_ticked(), CleanKind::ALL.to_vec());
    assert_eq!(plan.ticked_bytes(&[CleanKind::TempFiles, CleanKind::ShaderCaches]), 355 * MB + 65 * MB);
    assert!(CleanKind::ALL.iter().all(|k| !k.undoable()));
}

#[test]
fn cleanup_cleans_only_ticked_rows_keeps_in_use_and_never_follows_links() {
    let os = clean_fixture();
    let plan = cleanup::measure(&os).unwrap();
    let report = plan.clean(&os, &[CleanKind::TempFiles, CleanKind::LauncherCaches]).unwrap();
    let l = r"C:\Users\J\AppData\Local";
    // Temp: emptied except the in-use file; its folder stays because it is not empty; the temp folder itself stays.
    assert!(!os.exists(format!(r"{l}\Temp\a.tmp")));
    assert!(!os.exists(format!(r"{l}\Temp\sub\b.tmp")));
    assert!(os.exists(format!(r"{l}\Temp\sub\locked.tmp")));
    assert!(os.exists(format!(r"{l}\Temp\sub")));
    assert!(os.exists(format!(r"{l}\Temp")));
    assert!(os.exists(format!(r"{l}\Temp\link-to-docs")), "a junction is never deleted or followed");
    assert!(os.exists(r"C:\Users\J\Documents\precious.docx"));
    let t = &report.rows[0];
    assert_eq!(t.kind, Some(CleanKind::TempFiles));
    assert_eq!((t.freed_bytes, t.in_use_bytes, t.in_use_files), (150 * MB, 205 * MB, 1));
    assert_eq!(t.skipped, vec![("Windows temp folder".to_string(), PartState::NeedsAdmin)]);
    // Not ticked → untouched.
    assert!(os.exists(format!(r"{l}\D3DSCache\x\1.bin")));
    assert!(!os.changes().contains(&"empty_recycle_bin".to_string()));
    // Launchers: all four cleaned (nothing running); Epic's Config stays (only webcache*).
    assert!(!os.exists(r"C:\Program Files (x86)\Steam\appcache\appinfo.vdf"));
    assert!(os.exists(r"C:\Program Files (x86)\Steam\appcache"));
    assert!(!os.exists(format!(r"{l}\Steam\htmlcache\Cache")));
    assert!(os.exists(format!(r"{l}\EpicGamesLauncher\Saved\Config\keep.ini")));
    assert_eq!(report.rows[1].freed_bytes, (40 + 60 + 8 + 9) * MB);
    assert_eq!(report.freed_bytes(), 150 * MB + 117 * MB);
    assert_eq!(report.in_use_bytes(), 205 * MB);
    // Every change stayed inside a target folder.
    for c in os.changes() {
        let p = c.split_once(' ').unwrap().1;
        assert!(
            ["\\temp\\", "\\appcache\\", "\\htmlcache\\", "\\webcache_4430\\", "\\httpcache\\"].iter().any(|t| p.contains(t)),
            "change outside a target: {c}"
        );
    }
}

#[test]
fn cleanup_skips_a_launcher_started_after_measuring_and_empties_recycle_bin() {
    let os = clean_fixture();
    os.with_recycle_stuck(10 * MB, 1);
    let plan = cleanup::measure(&os).unwrap();
    os.with_running(&["riotclientservices.exe"]);
    let report = plan.clean(&os, &[CleanKind::LauncherCaches, CleanKind::RecycleBin, CleanKind::ShaderCaches]).unwrap();
    assert!(os.exists(r"C:\Users\J\AppData\Local\Riot Games\Riot Client\HttpCache\r1"));
    let launch = report.rows.iter().find(|r| r.kind == Some(CleanKind::LauncherCaches)).unwrap();
    assert!(launch.skipped.contains(&("Riot Client cache".to_string(), PartState::LauncherRunning("Riot Client"))));
    let rb = report.rows.iter().find(|r| r.kind == Some(CleanKind::RecycleBin)).unwrap();
    assert_eq!((rb.freed_bytes, rb.in_use_bytes), (3 * GB - 10 * MB, 10 * MB));
    assert!(os.changes().contains(&"empty_recycle_bin".to_string()));
    let sh = report.rows.iter().find(|r| r.kind == Some(CleanKind::ShaderCaches)).unwrap();
    assert_eq!(sh.freed_bytes, 65 * MB);
}

#[test]
fn cleanup_admin_path_cleans_windows_temp_when_elevated() {
    let os = FakeOs::new();
    os.with_known(known()).with_elevated(true);
    os.add_file(r"C:\Windows\Temp\w.tmp", 70 * MB);
    let plan = cleanup::measure(&os).unwrap();
    let temp = plan.row(CleanKind::TempFiles).unwrap();
    assert_eq!(temp.bytes, 70 * MB);
    plan.clean(&os, &[CleanKind::TempFiles]).unwrap();
    assert!(!os.exists(r"C:\Windows\Temp\w.tmp"));
    assert!(os.exists(r"C:\Windows\Temp"));
}

#[test]
fn cleanup_refuses_unsafe_targets_and_unmeasured_rows() {
    let k = known();
    assert!(!is_safe_target(Path::new(r"C:\"), &k));
    assert!(!is_safe_target(Path::new(r"C:\Windows"), &k));
    assert!(!is_safe_target(Path::new(r"C:\Users\J\AppData\Local"), &k));
    assert!(!is_safe_target(Path::new(r"C:\Users\J\Documents"), &k));
    assert!(!is_safe_target(Path::new(r"Temp"), &k));
    assert!(!is_safe_target(Path::new(r"C:\Temp"), &k), "too shallow");
    assert!(is_safe_target(Path::new(r"C:\Windows\Temp"), &k));
    assert!(is_safe_target(Path::new(r"C:\Users\J\AppData\Local\NVIDIA\DXCache"), &k));

    // A known folder pointing somewhere unsafe is refused at measure time and never cleaned.
    let os = FakeOs::new();
    let mut bad = known();
    bad.user_temp = Some(PathBuf::from(r"C:\Users\J\Documents"));
    os.with_known(bad).add_file(r"C:\Users\J\Documents\precious.docx", MB);
    let plan = cleanup::measure(&os).unwrap();
    let t = plan.row(CleanKind::TempFiles).unwrap();
    assert_eq!(t.parts[0].state, PartState::Refused);
    plan.clean(&os, &[CleanKind::TempFiles]).unwrap();
    assert!(os.exists(r"C:\Users\J\Documents\precious.docx"));

    let partial = CleanPlan { rows: vec![] };
    assert!(matches!(partial.clean(&os, &[CleanKind::TempFiles]), Err(StorageError::NotMeasured)));
}

// ---------------------------------------------------------------- health

fn disk(number: u32, media: MediaKind, bus: &str) -> PhysicalDisk {
    PhysicalDisk { number, model: format!("Disk {number}"), media, bus: Some(bus.into()), size_bytes: 1000 * GB }
}

#[test]
fn health_nvme_sata_hdd_and_admin_notes() {
    let drives = vec![drive('C', DriveKind::Fixed, 1, 0), drive('E', DriveKind::Fixed, 1, 0)];
    // NVMe, healthy: 46 °C, 3 % used, 12 000 h.
    let nvme = HealthRaw {
        os_status: Some(OsHealthStatus::Healthy),
        nvme: Some(NvmeHealthLog {
            temperature_kelvin: 319,
            available_spare_pct: 100,
            available_spare_threshold_pct: 10,
            percentage_used: 3,
            power_on_hours: 12_000,
            ..Default::default()
        }),
        ..Default::default()
    };
    let row = health::build_row(&disk(0, MediaKind::Ssd, "NVMe"), &nvme, &drives);
    assert_eq!((row.temperature_c, row.life_left_pct, row.power_on_hours), (Some(46), Some(97), Some(12_000)));
    assert_eq!(row.letters, vec!['C']);
    assert_eq!(row.status_text(), "● Healthy");
    assert_eq!(row.temp_level(), Some(TempLevel::Ok));

    // NVMe with critical warning + media errors.
    let mut bad = nvme.clone();
    if let Some(n) = bad.nvme.as_mut() {
        n.critical_warning = 0x01 | 0x04;
        n.available_spare_pct = 5;
        n.media_errors = 2;
        n.temperature_kelvin = 273 + 88;
    }
    let row = health::build_row(&disk(0, MediaKind::Ssd, "NVMe"), &bad, &drives);
    assert_eq!(row.warnings.len(), 3);
    assert_eq!(row.status_text(), "● 3 warnings");
    assert_eq!(row.temp_level(), Some(TempLevel::Hot));

    // SATA HDD with 8 moved sectors: life "—", one warning with the design's wording.
    let sata = HealthRaw {
        smart: vec![
            SmartAttribute { id: 5, value: 100, worst: 100, raw: 8, threshold: Some(10) },
            SmartAttribute { id: 9, value: 90, worst: 90, raw: 0x0001_0000_2710, threshold: Some(0) },
            SmartAttribute { id: 194, value: 40, worst: 30, raw: 0x0014_0000_0029, threshold: Some(0) },
        ],
        ..Default::default()
    };
    let row = health::build_row(&disk(2, MediaKind::Hdd, "SATA"), &sata, &[drive('E', DriveKind::Fixed, 1, 0)]);
    assert_eq!(row.life_left_pct, None);
    assert_eq!(row.temperature_c, Some(41));
    assert_eq!(row.power_on_hours, Some(10_000));
    assert_eq!(row.warnings, vec!["E: 8 sectors were moved to spares · back up what matters.".to_string()]);
    assert_eq!(row.status_text(), "● 1 warning");

    // Not admin: only Windows' verdict; the menu can say what admin would add.
    let limited = HealthRaw {
        os_status: Some(OsHealthStatus::Warning),
        needs_admin: vec!["SMART attributes (temperature, power-on hours, moved sectors)".into()],
        ..Default::default()
    };
    let row = health::build_row(&disk(3, MediaKind::Hdd, "SATA"), &limited, &[]);
    assert_eq!((row.temperature_c, row.power_on_hours), (None, None));
    assert_eq!(row.warnings.len(), 1);
    assert_eq!(row.admin_would_add.len(), 1);

    // Nothing at all (USB stick): "—".
    let row = health::build_row(&disk(4, MediaKind::Unknown, "USB"), &HealthRaw::default(), &[]);
    assert_eq!(row.status_text(), "—");

    // Reliability counter (admin WMI) fills gaps.
    let rel = HealthRaw {
        reliability: Some(ReliabilityCounter { temperature_c: Some(77), wear_pct: Some(12), power_on_hours: Some(5) }),
        ..Default::default()
    };
    let row = health::build_row(&disk(5, MediaKind::Ssd, "SATA"), &rel, &[]);
    assert_eq!((row.temperature_c, row.life_left_pct, row.power_on_hours), (Some(77), Some(88), Some(5)));
    assert_eq!(row.temp_level(), Some(TempLevel::Warm));
    assert_eq!(temp_level(74), TempLevel::Ok);
    assert_eq!(temp_level(75), TempLevel::Warm);
    assert_eq!(temp_level(85), TempLevel::Hot);
}

#[test]
fn health_read_all_maps_letters_to_disks() {
    let os = FakeOs::new();
    os.with_drives(vec![drive('C', DriveKind::Fixed, 1, 0), drive('D', DriveKind::Fixed, 1, 0)]);
    os.add_disk(disk(0, MediaKind::Ssd, "NVMe"), HealthRaw { os_status: Some(OsHealthStatus::Healthy), ..Default::default() });
    os.add_disk(disk(1, MediaKind::Hdd, "SATA"), HealthRaw::default());
    let rows = health::read_all(&os).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].letters, vec!['C']);
    assert_eq!(rows[1].letters, vec!['D']);
}

#[test]
fn scan_gives_the_same_tree_with_1_or_16_threads() {
    // The wake-up change (notify_one per new folder) is a speed change: prove the result doesn't depend on threads.
    let os = FakeOs::new();
    os.with_known(known());
    for a in 0..12 {
        for b in 0..12 {
            for c in 0..6 {
                os.add_file(format!(r"C:\T\d{a}\e{b}\f{c}\x.mp4"), (a * 1000 + b * 10 + c + 1) as u64);
                os.add_file(format!(r"C:\T\d{a}\e{b}\f{c}\y.png"), 7);
            }
        }
    }
    let rules = ClassRules::from_os(&os);
    let one = scan::scan_folder(&os, Path::new(r"C:\T"), &rules, ScanOptions { threads: 1, files_per_folder: 24 }, &ScanControl::new()).unwrap();
    for _ in 0..5 {
        let many = scan::scan_folder(&os, Path::new(r"C:\T"), &rules, ScanOptions { threads: 16, files_per_folder: 24 }, &ScanControl::new()).unwrap();
        assert_eq!(many.types.walked, one.types.walked);
        assert_eq!((many.stats.files, many.stats.folders), (one.stats.files, one.stats.folders));
        assert_eq!(many.tree.size(0).unwrap(), one.tree.size(0).unwrap());
        let a: Vec<(String, u64)> = one.tree.rows(0).unwrap().into_iter().map(|r| (r.name, r.bytes)).collect();
        let b: Vec<(String, u64)> = many.tree.rows(0).unwrap().into_iter().map(|r| (r.name, r.bytes)).collect();
        assert_eq!(a, b);
    }
    assert_eq!(one.stats.files, 12 * 12 * 6 * 2);
}

/// the owner Oct 8 (boss call): with the menu closed only what the page shows first is kept - the root's rows; a folder whose
/// inside was cut away is scanned again on its own and grafted back, giving the same rows as the full walk.
#[test]
fn pruned_tree_keeps_the_top_rows_and_a_rescanned_folder_grafts_back() {
    let os = scan_fixture();
    let full = scan::scan_drive(&os, 'C', &ScanControl::new()).unwrap().tree;
    let mut p = full.pruned();
    let strip = |rows: Vec<scan::FolderRow>| -> Vec<(String, u64)> { rows.into_iter().map(|r| (r.name, r.bytes)).collect() };
    assert_eq!(strip(p.rows(p.root()).unwrap()), strip(full.rows(full.root()).unwrap()), "the top rows are the same");
    assert!(p.heap_bytes() < full.heap_bytes());
    let users = p.rows(p.root()).unwrap().into_iter().find(|r| r.name == "Users").unwrap();
    let RowKind::Folder { id, has_subfolders, .. } = users.kind else { panic!() };
    assert!(has_subfolders && p.is_pruned(id));
    let sub = scan::scan_subfolder(&os, 'C', &p.path(id).unwrap(), &ScanControl::new()).unwrap();
    p.graft(id, &sub.tree).unwrap();
    assert!(!p.is_pruned(id));
    let fid = full.find(Path::new(r"C:\Users")).unwrap();
    assert_eq!(strip(p.rows(id).unwrap()), strip(full.rows(fid).unwrap()));
    // one level deeper works too (the grafted folders carry their own insides)
    let j = p.find(Path::new(r"C:\Users\J")).unwrap();
    assert_eq!(strip(p.rows(j).unwrap()), strip(full.rows(full.find(Path::new(r"C:\Users\J")).unwrap()).unwrap()));
    assert_eq!(p.breadcrumb(j).unwrap().len(), 3);
}
