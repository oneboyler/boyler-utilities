//! The OS layer: everything this crate asks Windows, behind one trait.

use crate::Result;
use std::io;
use std::path::{Path, PathBuf};

/// What `GetDriveTypeW` says a drive is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveKind {
    Fixed,
    /// USB sticks, SD cards.
    Removable,
    Network,
    CdRom,
    RamDisk,
    Unknown,
}

/// SSD or hard drive (from `StorageDeviceSeekPenaltyProperty`: a seek penalty = spinning disk).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Ssd,
    Hdd,
    Unknown,
}

/// Can the volume be read?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeState {
    Ready,
    /// BitLocker-locked.
    Locked,
    /// No media (empty card reader) or the volume refused.
    NotReady,
}

/// One drive letter, as the OS reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct DriveInfo {
    pub letter: char,
    pub kind: DriveKind,
    pub state: VolumeState,
    /// Volume label ("" when it has none).
    pub label: String,
    pub file_system: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    /// The drive Windows runs from.
    pub is_system: bool,
    /// Physical disk model (e.g. "Samsung SSD 970 EVO 1TB"), from `IOCTL_STORAGE_QUERY_PROPERTY`.
    pub model: Option<String>,
    pub media: MediaKind,
    /// Bus ("NVMe", "SATA", "USB" …).
    pub bus: Option<String>,
    /// `\\.\PhysicalDriveN` this volume lives on (first extent).
    pub disk_number: Option<u32>,
}

/// One entry of a directory listing (what `FindFirstFileExW` returns).
#[derive(Debug, Clone, PartialEq)]
pub struct RawEntry {
    pub name: String,
    pub is_dir: bool,
    /// Logical file size in bytes (0 for folders).
    pub size: u64,
    /// A junction / symlink / mount point. The walk never follows these (no loops, no double counting).
    pub is_reparse: bool,
    /// A cloud placeholder that is not on the disk (OneDrive "online-only"): counted as 0 bytes.
    pub is_cloud_only: bool,
}

/// The folders this crate needs to know (from environment / registry).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KnownDirs {
    /// `%TEMP%` of the user.
    pub user_temp: Option<PathBuf>,
    /// `C:\Windows\Temp` (needs admin to list / clean).
    pub windows_temp: Option<PathBuf>,
    /// `%LocalAppData%`.
    pub local_appdata: Option<PathBuf>,
    /// `%UserProfile%\AppData\LocalLow`.
    pub local_low: Option<PathBuf>,
    /// `%ProgramData%`.
    pub program_data: Option<PathBuf>,
    /// `C:\Windows`.
    pub windows: Option<PathBuf>,
    /// `C:\Program Files`, `C:\Program Files (x86)`.
    pub program_files: Vec<PathBuf>,
    /// Steam's install folder (HKCU\Software\Valve\Steam\SteamPath).
    pub steam: Option<PathBuf>,
}

/// Recycle bin size (all drives), from `SHQueryRecycleBinW`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecycleBinInfo {
    pub bytes: u64,
    pub items: u64,
}

/// A physical disk (`\\.\PhysicalDriveN`).
#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalDisk {
    pub number: u32,
    pub model: String,
    pub media: MediaKind,
    pub bus: Option<String>,
    pub size_bytes: u64,
}

/// Windows' own one-word health verdict (`MSFT_PhysicalDisk.HealthStatus`, readable without admin).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsHealthStatus {
    Healthy,
    Warning,
    Unhealthy,
    Unknown,
}

/// One SATA SMART attribute (READ DATA, command 0xD0 — a read; this crate never sends a SMART command that writes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmartAttribute {
    pub id: u8,
    /// Normalised value (100 / 200 / 253 … = new, falls with wear).
    pub value: u8,
    pub worst: u8,
    /// The 6 raw bytes as a little-endian number.
    pub raw: u64,
    /// The drive's failure threshold for this attribute (READ THRESHOLDS, 0xD1), when read.
    pub threshold: Option<u8>,
}

/// The NVMe SMART / health log page (log 0x02) fields this crate uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NvmeHealthLog {
    /// Bit field: 0 spare below threshold, 1 temperature, 2 reliability degraded, 3 read-only, 4 volatile backup failed.
    pub critical_warning: u8,
    pub temperature_kelvin: u16,
    pub available_spare_pct: u8,
    pub available_spare_threshold_pct: u8,
    /// "Percentage used" of rated life (can go over 100).
    pub percentage_used: u8,
    pub power_on_hours: u64,
    pub media_errors: u64,
    pub unsafe_shutdowns: u64,
}

/// What the OS layer could read about one disk's health. Every field is optional: drives report different things.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HealthRaw {
    pub os_status: Option<OsHealthStatus>,
    pub nvme: Option<NvmeHealthLog>,
    /// From `StorageDeviceTemperatureProperty` (may work without admin even when SMART does not).
    pub temperature_c: Option<i32>,
    pub smart: Vec<SmartAttribute>,
    /// `MSFT_StorageReliabilityCounter` (WMI): temperature, wear %, power-on hours. Admin only.
    pub reliability: Option<ReliabilityCounter>,
    /// What could not be read because we are not admin (plain words, e.g. "SATA SMART attributes").
    pub needs_admin: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReliabilityCounter {
    pub temperature_c: Option<u32>,
    pub wear_pct: Option<u32>,
    pub power_on_hours: Option<u64>,
}

/// One entry of a whole-drive listing (Order 069): a file or a folder as a fast source (Everything's index) lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    /// The folder it is in, e.g. `C:\Users\You` (a drive's top folder is `C:`); empty = the drive itself.
    pub dir: String,
    pub name: String,
    /// bytes (0 for a folder)
    pub size: u64,
    /// An online-only cloud file: it takes no room on the drive (counted as 0, like the walk does).
    pub cloud_only: bool,
}

/// A drive's folders and files as a fast source reads them. Dropping it ends the source (Everything is quit then).
pub trait DriveListing {
    /// The next page of the drive's FOLDERS (`folders`) or FILES, at most `max`; an empty page = the end of that list. The
    /// two lists are read one after the other.
    fn page(&mut self, folders: bool, max: usize) -> Result<Vec<ListEntry>>;
}

/// Everything the Storage features ask Windows. `RealOs` is the Windows one; `FakeOs` is for tests.
pub trait StorageOs: Send + Sync {
    /// Order 069: a fast listing of the whole drive (Everything's index) for the "What's using" measure; `None` = there is
    /// none (the walk is used), `Some(Err)` = it was tried and failed (the walk is used). `ctl` can stop the wait for it.
    fn open_listing(&self, _letter: char, _ctl: &crate::scan::ScanControl) -> Option<Result<Box<dyn DriveListing>>> {
        None
    }

    /// All drive letters with their sizes (network drives too; [`crate::drives`] filters).
    fn drives(&self) -> Result<Vec<DriveInfo>>;
    /// One directory's entries (no `.` / `..`). `PermissionDenied` = "Can't be read".
    fn read_dir(&self, path: &Path) -> io::Result<Vec<RawEntry>>;
    /// Delete one file. In use → `io::ErrorKind` from Windows (sharing violation / access denied).
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    /// Delete one empty folder.
    fn remove_dir(&self, path: &Path) -> io::Result<()>;
    /// Move one file to the Recycle Bin (Order 069: Files view, Delete) - not a permanent delete.
    fn recycle_file(&self, path: &Path) -> io::Result<()>;
    fn recycle_bin(&self) -> Result<RecycleBinInfo>;
    /// Empty the recycle bin of every drive, no confirm / progress / sound.
    fn empty_recycle_bin(&self) -> Result<()>;
    fn known_dirs(&self) -> KnownDirs;
    /// Lower-case exe names of running processes ("steam.exe").
    fn running_exe_names(&self) -> Vec<String>;
    /// Folders that hold installed games (launcher libraries + game folders).
    fn game_roots(&self) -> Vec<PathBuf>;
    fn is_elevated(&self) -> bool;
    fn physical_disks(&self) -> Result<Vec<PhysicalDisk>>;
    /// Read-only health of one physical disk.
    fn disk_health(&self, disk_number: u32) -> Result<HealthRaw>;
}

/// Lets `Arc<RealOs>` / `&FakeOs` be passed where a `StorageOs` is wanted.
impl<T: StorageOs + ?Sized> StorageOs for &T {
    fn drives(&self) -> Result<Vec<DriveInfo>> {
        (**self).drives()
    }
    fn read_dir(&self, path: &Path) -> io::Result<Vec<RawEntry>> {
        (**self).read_dir(path)
    }
    fn open_listing(&self, letter: char, ctl: &crate::scan::ScanControl) -> Option<Result<Box<dyn DriveListing>>> {
        (**self).open_listing(letter, ctl)
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        (**self).remove_file(path)
    }
    fn remove_dir(&self, path: &Path) -> io::Result<()> {
        (**self).remove_dir(path)
    }
    fn recycle_file(&self, path: &Path) -> io::Result<()> {
        (**self).recycle_file(path)
    }
    fn recycle_bin(&self) -> Result<RecycleBinInfo> {
        (**self).recycle_bin()
    }
    fn empty_recycle_bin(&self) -> Result<()> {
        (**self).empty_recycle_bin()
    }
    fn known_dirs(&self) -> KnownDirs {
        (**self).known_dirs()
    }
    fn running_exe_names(&self) -> Vec<String> {
        (**self).running_exe_names()
    }
    fn game_roots(&self) -> Vec<PathBuf> {
        (**self).game_roots()
    }
    fn is_elevated(&self) -> bool {
        (**self).is_elevated()
    }
    fn physical_disks(&self) -> Result<Vec<PhysicalDisk>> {
        (**self).physical_disks()
    }
    fn disk_health(&self, disk_number: u32) -> Result<HealthRaw> {
        (**self).disk_health(disk_number)
    }
}
