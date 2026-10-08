//! A fake OS for tests: an in-memory file tree, drives, recycle bin, processes and disk health.
//! Nothing here touches the real PC.

use crate::{
    DriveInfo, HealthRaw, KnownDirs, MediaKind, NvmeHealthLog, OsHealthStatus, PhysicalDisk, RawEntry, RecycleBinInfo, Result,
    SmartAttribute, StorageError, StorageOs,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone)]
struct FakeEntry {
    name: String,
    is_dir: bool,
    size: u64,
    reparse: bool,
    cloud: bool,
    in_use: bool,
}

#[derive(Debug, Default)]
struct State {
    dirs: BTreeMap<String, Vec<FakeEntry>>,
    unreadable: HashSet<String>,
    drives: Vec<DriveInfo>,
    recycle: RecycleBinInfo,
    /// Bytes / items the fake recycle bin keeps after "empty" (simulates files Windows won't let go).
    recycle_stuck: RecycleBinInfo,
    known: KnownDirs,
    running: Vec<String>,
    game_roots: Vec<PathBuf>,
    elevated: bool,
    disks: Vec<PhysicalDisk>,
    health: HashMap<u32, HealthRaw>,
    changes: Vec<String>,
}

/// The fake. Build it with the `with_*` / `add_*` methods, then pass `&fake` where a `StorageOs` is wanted.
#[derive(Debug, Default)]
pub struct FakeOs {
    s: Mutex<State>,
}

fn key(p: &Path) -> String {
    let mut s = p.to_string_lossy().replace('/', "\\").to_lowercase();
    while s.ends_with('\\') && s.len() > 3 {
        s.pop();
    }
    if s.len() == 2 && s.ends_with(':') {
        s.push('\\');
    }
    s
}

fn split(p: &Path) -> Option<(String, String)> {
    let parent = p.parent()?;
    let name = p.file_name()?.to_string_lossy().into_owned();
    Some((key(parent), name))
}

impl FakeOs {
    pub fn new() -> Self {
        Self::default()
    }
    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.s.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn ensure_dir(st: &mut State, p: &Path) {
        let k = key(p);
        if st.dirs.contains_key(&k) {
            return;
        }
        st.dirs.insert(k, Vec::new());
        if let Some((parent, name)) = split(p) {
            Self::ensure_dir(st, p.parent().unwrap_or(p));
            let list = st.dirs.entry(parent).or_default();
            if !list.iter().any(|e| e.name.eq_ignore_ascii_case(&name)) {
                list.push(FakeEntry { name, is_dir: true, size: 0, reparse: false, cloud: false, in_use: false });
            }
        }
    }

    fn add_entry(&self, p: &Path, size: u64, reparse: bool, cloud: bool, in_use: bool, is_dir: bool) {
        let mut st = self.st();
        if let Some(parent) = p.parent() {
            Self::ensure_dir(&mut st, parent);
        }
        if let Some((parent, name)) = split(p) {
            st.dirs.entry(parent).or_default().push(FakeEntry { name, is_dir, size, reparse, cloud, in_use });
        }
    }

    pub fn add_dir(&self, p: impl AsRef<Path>) -> &Self {
        Self::ensure_dir(&mut self.st(), p.as_ref());
        self
    }
    pub fn add_file(&self, p: impl AsRef<Path>, size: u64) -> &Self {
        self.add_entry(p.as_ref(), size, false, false, false, false);
        self
    }
    /// A file Windows won't let go (delete fails like a sharing violation).
    pub fn add_file_in_use(&self, p: impl AsRef<Path>, size: u64) -> &Self {
        self.add_entry(p.as_ref(), size, false, false, true, false);
        self
    }
    /// An online-only cloud file (OneDrive placeholder).
    pub fn add_cloud_file(&self, p: impl AsRef<Path>, size: u64) -> &Self {
        self.add_entry(p.as_ref(), size, false, true, false, false);
        self
    }
    /// A junction pointing somewhere else (must never be followed or deleted).
    pub fn add_junction(&self, p: impl AsRef<Path>) -> &Self {
        self.add_entry(p.as_ref(), 0, true, false, false, true);
        self
    }
    /// A folder whose listing is "access denied".
    pub fn set_unreadable(&self, p: impl AsRef<Path>) -> &Self {
        self.add_dir(p.as_ref());
        self.st().unreadable.insert(key(p.as_ref()));
        self
    }
    pub fn with_drives(&self, d: Vec<DriveInfo>) -> &Self {
        self.st().drives = d;
        self
    }
    pub fn with_recycle_bin(&self, bytes: u64, items: u64) -> &Self {
        self.st().recycle = RecycleBinInfo { bytes, items };
        self
    }
    pub fn with_recycle_stuck(&self, bytes: u64, items: u64) -> &Self {
        self.st().recycle_stuck = RecycleBinInfo { bytes, items };
        self
    }
    pub fn with_known(&self, k: KnownDirs) -> &Self {
        self.st().known = k;
        self
    }
    pub fn with_running(&self, exes: &[&str]) -> &Self {
        self.st().running = exes.iter().map(|s| s.to_lowercase()).collect();
        self
    }
    pub fn with_game_roots(&self, roots: Vec<PathBuf>) -> &Self {
        self.st().game_roots = roots;
        self
    }
    pub fn with_elevated(&self, e: bool) -> &Self {
        self.st().elevated = e;
        self
    }
    pub fn add_disk(&self, d: PhysicalDisk, h: HealthRaw) -> &Self {
        let mut st = self.st();
        st.health.insert(d.number, h);
        st.disks.push(d);
        self
    }

    /// Does this path exist in the fake tree?
    pub fn exists(&self, p: impl AsRef<Path>) -> bool {
        let p = p.as_ref();
        let st = self.st();
        if st.dirs.contains_key(&key(p)) {
            return true;
        }
        split(p).and_then(|(parent, name)| st.dirs.get(&parent).map(|l| l.iter().any(|e| e.name.eq_ignore_ascii_case(&name)))).unwrap_or(false)
    }
    /// Every change the code asked for, in order ("remove_file c:\…", "empty_recycle_bin").
    pub fn changes(&self) -> Vec<String> {
        self.st().changes.clone()
    }
}

impl StorageOs for FakeOs {
    fn drives(&self) -> Result<Vec<DriveInfo>> {
        Ok(self.st().drives.clone())
    }
    fn read_dir(&self, path: &Path) -> io::Result<Vec<RawEntry>> {
        let st = self.st();
        let k = key(path);
        if st.unreadable.contains(&k) {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        let list = st.dirs.get(&k).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        Ok(list
            .iter()
            .map(|e| RawEntry { name: e.name.clone(), is_dir: e.is_dir, size: e.size, is_reparse: e.reparse, is_cloud_only: e.cloud })
            .collect())
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        let mut st = self.st();
        let (parent, name) = split(path).ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        let list = st.dirs.get_mut(&parent).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let i = list
            .iter()
            .position(|e| !e.is_dir && e.name.eq_ignore_ascii_case(&name))
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        if list[i].in_use {
            return Err(io::Error::from_raw_os_error(32)); // ERROR_SHARING_VIOLATION
        }
        list.remove(i);
        st.changes.push(format!("remove_file {}", key(path)));
        Ok(())
    }
    fn remove_dir(&self, path: &Path) -> io::Result<()> {
        let mut st = self.st();
        let k = key(path);
        match st.dirs.get(&k) {
            Some(l) if !l.is_empty() => return Err(io::Error::from_raw_os_error(145)), // ERROR_DIR_NOT_EMPTY
            None => return Err(io::Error::from(io::ErrorKind::NotFound)),
            _ => {}
        }
        st.dirs.remove(&k);
        if let Some((parent, name)) = split(path) {
            if let Some(list) = st.dirs.get_mut(&parent) {
                list.retain(|e| !(e.is_dir && e.name.eq_ignore_ascii_case(&name)));
            }
        }
        st.changes.push(format!("remove_dir {k}"));
        Ok(())
    }
    fn recycle_bin(&self) -> Result<RecycleBinInfo> {
        Ok(self.st().recycle)
    }
    fn empty_recycle_bin(&self) -> Result<()> {
        let mut st = self.st();
        st.recycle = st.recycle_stuck;
        st.changes.push("empty_recycle_bin".to_string());
        Ok(())
    }
    fn known_dirs(&self) -> KnownDirs {
        self.st().known.clone()
    }
    fn running_exe_names(&self) -> Vec<String> {
        self.st().running.clone()
    }
    fn game_roots(&self) -> Vec<PathBuf> {
        self.st().game_roots.clone()
    }
    fn is_elevated(&self) -> bool {
        self.st().elevated
    }
    fn physical_disks(&self) -> Result<Vec<PhysicalDisk>> {
        Ok(self.st().disks.clone())
    }
    fn disk_health(&self, disk_number: u32) -> Result<HealthRaw> {
        self.st().health.get(&disk_number).cloned().ok_or_else(|| StorageError::NotFound(format!("disk {disk_number}")))
    }
}

/// 1 GB as the drawing counts it (GBf: 1024³ bytes).
fn gb(v: f64) -> u64 {
    (v * 1_073_741_824.0) as u64
}

impl FakeOs {
    /// The drawing's PC (menu-v22 `DRIVES`, `CLN`, health): C: Windows (Samsung 990 PRO, NVMe, 1863 GB, 1251 used),
    /// D: Games (WD_BLACK SN850X, 1863 / 1404), E: Storage (Seagate BarraCuda, hard drive, 3726 / 2614, 8 sectors moved);
    /// C:'s folder tree with the drawing's sizes; recycle bin 6.4 GB (312 items), temp 3.3 GB (0.2 GB in use), shader
    /// caches 2.8 GB, launcher caches 1.9 GB. Every name is made up ("You" is the user folder). Not elevated.
    pub fn drawing() -> FakeOs {
        let f = FakeOs::new();
        let drive = |letter: char, label: &str, model: &str, media: MediaKind, bus: &str, tot: f64, used: f64, n: u32| DriveInfo {
            letter,
            kind: crate::DriveKind::Fixed,
            state: crate::VolumeState::Ready,
            label: label.into(),
            file_system: "NTFS".into(),
            total_bytes: gb(tot),
            free_bytes: gb(tot) - gb(used),
            is_system: letter == 'C',
            model: Some(model.into()),
            media,
            bus: Some(bus.into()),
            disk_number: Some(n),
        };
        f.with_drives(vec![
            drive('C', "Windows", "Samsung 990 PRO", MediaKind::Ssd, "NVMe", 1863.0, 1251.0, 0),
            drive('D', "Games", "WD_BLACK SN850X", MediaKind::Ssd, "NVMe", 1863.0, 1404.0, 1),
            drive('E', "Storage", "Seagate BarraCuda", MediaKind::Hdd, "SATA", 3726.0, 2614.0, 2),
        ]);
        let home = "C:\\Users\\You";
        let local = format!("{home}\\AppData\\Local");
        f.with_known(KnownDirs {
            user_temp: Some(PathBuf::from(format!("{local}\\Temp"))),
            windows_temp: Some(PathBuf::from("C:\\Windows\\Temp")),
            local_appdata: Some(PathBuf::from(&local)),
            local_low: Some(PathBuf::from(format!("{home}\\AppData\\LocalLow"))),
            program_data: Some(PathBuf::from("C:\\ProgramData")),
            windows: Some(PathBuf::from("C:\\Windows")),
            program_files: vec![PathBuf::from("C:\\Program Files"), PathBuf::from("C:\\Program Files (x86)")],
            steam: Some(PathBuf::from("C:\\Program Files (x86)\\Steam")),
        });
        f.with_game_roots(vec![
            PathBuf::from("C:\\Program Files (x86)\\Steam\\steamapps"),
            PathBuf::from("C:\\Program Files\\Epic Games"),
            PathBuf::from("C:\\Riot Games"),
            PathBuf::from("D:\\SteamLibrary\\steamapps"),
            PathBuf::from("D:\\Epic Games"),
            PathBuf::from("D:\\Battle.net"),
        ]);
        f.with_recycle_bin(gb(6.4), 312);
        // C: - each leaf folder holds one file of the drawing's size (the extension picks its file type)
        let leaves: &[(&str, f64, &str)] = &[
            ("Users\\You\\Videos\\Clips\\VALORANT", 121.0, "mp4"),
            ("Users\\You\\Videos\\Clips\\Rocket League", 63.0, "mp4"),
            ("Users\\You\\Videos\\Clips\\Desktop", 30.0, "mp4"),
            ("Users\\You\\Videos\\OBS", 54.0, "mkv"),
            ("Users\\You\\AppData\\Local\\Packages", 15.7, "dat"),
            ("Users\\You\\AppData\\Local\\Programs", 11.6, "exe"),
            ("Users\\You\\AppData\\Local\\NVIDIA", 7.7, "dat"),
            ("Users\\You\\AppData\\Local\\Google", 6.1, "dat"),
            ("Users\\You\\AppData\\Local\\Discord", 2.2, "dat"),
            ("Users\\You\\AppData\\Local\\Spotify", 1.9, "dat"),
            ("Users\\You\\AppData\\Roaming", 17.4, "dat"),
            ("Users\\You\\AppData\\LocalLow", 1.6, "dat"),
            ("Users\\You\\Downloads", 44.0, "zip"),
            ("Users\\You\\Pictures", 27.0, "jpg"),
            ("Users\\You\\Documents", 14.8, "pdf"),
            ("Users\\You\\Desktop", 6.4, "lnk"),
            ("Users\\Public", 4.1, "dat"),
            ("Program Files (x86)\\Steam\\steamapps\\common\\Call of Duty HQ", 236.0, "pak"),
            ("Program Files (x86)\\Steam\\steamapps\\common\\Apex Legends", 71.0, "pak"),
            ("Program Files (x86)\\Steam\\steamapps\\common\\Elden Ring", 47.0, "pak"),
            ("Program Files (x86)\\Steam\\steamapps\\common\\Counter-Strike 2", 39.0, "pak"),
            ("Program Files (x86)\\Steam\\steamapps\\common\\Rust", 3.4, "pak"),
            ("Program Files (x86)\\Steam\\steamapps\\shadercache", 4.6, "bin"),
            ("Program Files (x86)\\Microsoft", 5.1, "exe"),
            ("Program Files (x86)\\Battle.net", 1.9, "exe"),
            ("Program Files\\Epic Games\\rocketleague", 34.0, "pak"),
            ("Program Files\\Epic Games\\Fall Guys", 24.0, "pak"),
            ("Program Files\\Adobe", 31.0, "exe"),
            ("Program Files\\NVIDIA Corporation", 4.1, "exe"),
            ("Program Files\\obs-studio", 0.5, "exe"),
            ("Program Files\\Common Files", 6.4, "dll"),
            ("Program Files (x86)\\Steam\\htmlcache", 1.2, "dat"),
            ("Program Files (x86)\\Steam\\depotcache", 1.6, "dat"),
            ("Program Files\\WindowsApps", 41.0, "appx"),
            ("ProgramData", 57.0, "dat"),
            ("Riot Games\\VALORANT", 46.0, "pak"),
            ("Riot Games\\Riot Client", 6.0, "exe"),
            ("Windows", 46.0, "dll"),
            ("System Volume Information", 41.0, "dat"),
            ("$Recycle.Bin", 6.4, "dat"),
        ];
        for (dir, size, ext) in leaves {
            let name = dir.rsplit('\\').next().unwrap_or("data");
            f.add_file(format!("C:\\{dir}\\{name}.{ext}"), gb(*size));
        }
        f.add_file("C:\\pagefile.sys", gb(32.0));
        f.add_file("C:\\hiberfil.sys", gb(25.6));
        // Clean up: temp 3.1 GB + 0.2 GB in use; shader caches (DirectX 1.1, NVIDIA 1.7); launcher caches (Steam 1.2,
        // Epic 0.5, Riot 0.2)
        f.add_file(format!("{local}\\Temp\\setup_leftovers.tmp"), gb(3.1));
        f.add_file_in_use(format!("{local}\\Temp\\open_by_an_app.tmp"), gb(0.2));
        f.add_file(format!("{local}\\D3DSCache\\cache.bin"), gb(1.1));
        f.add_file(format!("{local}\\NVIDIA\\DXCache\\cache.bin"), gb(1.7));
        f.add_file("C:\\Program Files (x86)\\Steam\\appcache\\appinfo.vdf", gb(1.2));
        f.add_file(format!("{local}\\EpicGamesLauncher\\Saved\\webcache\\data_1"), gb(0.5));
        f.add_file(format!("{local}\\Riot Games\\Riot Client\\HttpCache\\data_1"), gb(0.2));
        // D: and E: (the drawing's top folders)
        for (p, size) in [
            ("D:\\SteamLibrary\\steamapps\\common\\Call of Duty\\data.pak", 312.0),
            ("D:\\SteamLibrary\\steamapps\\common\\Baldur's Gate 3\\data.pak", 149.0),
            ("D:\\SteamLibrary\\steamapps\\common\\Red Dead Redemption 2\\data.pak", 121.0),
            ("D:\\SteamLibrary\\steamapps\\common\\Cyberpunk 2077\\data.pak", 118.0),
            ("D:\\SteamLibrary\\steamapps\\common\\Others\\data.pak", 196.0),
            ("D:\\Epic Games\\Fortnite\\data.pak", 104.0),
            ("D:\\Epic Games\\ARK Survival Ascended\\data.pak", 110.0),
            ("D:\\Battle.net\\Overwatch\\data.pak", 66.0),
            ("D:\\Clips\\clip.mp4", 96.0),
            ("D:\\Installers\\setup.exe", 71.0),
            ("D:\\Backups\\backup.zip", 52.5),
            ("E:\\Recordings\\2026\\rec.mp4", 812.0),
            ("E:\\Recordings\\2025\\rec.mp4", 508.0),
            ("E:\\Movies\\movie.mkv", 320.0),
            ("E:\\Backups\\backup.zip", 474.0),
            ("E:\\Photos\\Phone\\img.jpg", 366.0),
            ("E:\\Photos\\Camera\\img.jpg", 46.0),
            ("E:\\Documents\\doc.pdf", 88.0),
        ] {
            f.add_file(p, gb(size));
        }
        // health: NVMe logs for the two SSDs, SATA SMART for the hard drive (8 reallocated sectors)
        let nvme = |t: u16, used: u8, hrs: u64| HealthRaw {
            os_status: Some(OsHealthStatus::Healthy),
            nvme: Some(NvmeHealthLog {
                critical_warning: 0,
                temperature_kelvin: t + 273,
                available_spare_pct: 100,
                available_spare_threshold_pct: 10,
                percentage_used: used,
                power_on_hours: hrs,
                media_errors: 0,
                unsafe_shutdowns: 12,
            }),
            ..HealthRaw::default()
        };
        let disk = |n: u32, model: &str, media: MediaKind, bus: &str, tot: f64| PhysicalDisk { number: n, model: model.into(), media, bus: Some(bus.into()), size_bytes: gb(tot) };
        f.add_disk(disk(0, "Samsung 990 PRO", MediaKind::Ssd, "NVMe", 1863.0), nvme(41, 4, 3412));
        f.add_disk(disk(1, "WD_BLACK SN850X", MediaKind::Ssd, "NVMe", 1863.0), nvme(46, 9, 5120));
        let a = |id: u8, raw: u64| SmartAttribute { id, value: 100, worst: 100, raw, threshold: Some(if id == 5 { 36 } else { 0 }) };
        f.add_disk(
            disk(2, "Seagate BarraCuda", MediaKind::Hdd, "SATA", 3726.0),
            HealthRaw { os_status: Some(OsHealthStatus::Healthy), smart: vec![a(194, 38), a(9, 21870), a(5, 8)], ..HealthRaw::default() },
        );
        f
    }
}
