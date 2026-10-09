//! The fast whole-drive listing through Everything (Order 069, the owner: "What's using <drive>" uses Everything when it is
//! installed - near-instant; the own walk stays as the fallback).
//!
//! Only OUR copy of Everything (the one the Search tab set up, `bu_search::real::ours`) and only while measuring: it is
//! started when Measure is pressed - as its own named instance `BoylerUtilitiesStorage`, hidden, below-normal priority, on
//! the ONE drive being measured, with its own settings and index file in `%LOCALAPPDATA%\BoylerUtilities\EverythingStorage\<drive>`,
//! the drive's folders and files are read (name, folder, size, attributes), and it is quit again the moment the listing is
//! dropped (the measure is done / stopped). Nothing of it stays running: not the instance, not our service (the Search tab's
//! instance, if it runs, keeps the service until it ends too - `ours::service_stop`).
//!
//! Measured on a real PC (C:, 2.2 million files + 475,000 folders, Everything 1.4.1.1032): index ready 6.0 s on a first start
//! (built from the drive), 0.4 s from the saved index; the whole list in pages of 100,000 (25 MB each) at 66-105 ms a page from
//! Everything's side; Everything's own RAM 174 MB (181 MB after the lists) - 178 / 184 MB with folder sizes indexed too. Asking
//! for attributes that are NOT indexed makes Everything read them from the disk (8.5 s a page): `index_attributes=1` is set.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use bu_search::real::{everything, host, ours};
use windows::Win32::Foundation::HWND;

use crate::scan::ScanControl;
use crate::{DriveListing, ListEntry, Result, StorageError};

/// Our instance's settings (Everything.ini). Everything that would show or stay is off; only the drive's volume is read;
/// nothing is left out (the Search tab's lean index leaves out WinSxS and build folders - Storage wants every byte); the
/// size and the attributes are indexed (an attribute that is not indexed is read from the disk, page after page).
fn ini(windir: &str, pipe: &str, drive: &ours::Drive) -> String {
    let _ = windir;
    format!(
        "[Everything]\r\nrun_in_background=1\r\nshow_tray_icon=0\r\nrun_on_system_startup=0\r\ncheck_for_updates_on_startup=0\r\nshow_in_taskbar=0\r\nrun_as_admin=0\r\n\
         index_size=1\r\nindex_date_modified=0\r\nindex_attributes=1\r\nfast_size_sort=0\r\nfast_date_modified_sort=0\r\nfast_path_sort=0\r\nfast_extension_sort=0\r\nfast_attributes_sort=0\r\n\
         index_recent_changes=0\r\nexclude_list_enabled=0\r\nauto_include_fixed_volumes=0\r\nauto_include_fixed_refs_volumes=0\r\nauto_include_removable_volumes=0\r\n\
         service_pipe_name={pipe}\r\nntfs_volume_guids=\"{}\"\r\nntfs_volume_paths=\"{}:\"\r\nntfs_volume_roots=\"\"\r\nntfs_volume_includes=1\r\nntfs_volume_load_recent_changes=0\r\n",
        drive.guid.replace('\\', "\\\\"),
        drive.letter
    )
}

/// What `index.ok` holds when the saved index was made with today's settings and saved whole.
const MARK: &str = "BoylerUtilities storage index: one drive, size + attributes (069)";

/// How long Everything may take to have the drive's index (a first build of a big hard drive can take minutes).
const READY_WAIT: Duration = Duration::from_secs(900);

/// One instance at a time: it is ONE named instance with one drive's settings, so a second drive measured meanwhile waits
/// its turn (the page may measure two drives at once).
static SESSION: Mutex<()> = Mutex::new(());

/// The listing of one drive through our Everything: a started instance and the next page of each of the two lists.
pub struct EverythingListing {
    window: HWND,
    letter: char,
    at: [u32; 2],
    instance: Option<host::Ours>,
    exe: PathBuf,
    dir: PathBuf,
    /// held until the instance is quit (dropped after `Drop::drop`)
    _turn: MutexGuard<'static, ()>,
}

/// The listing of `letter`, or `None` when there is none to give (our Everything is not set up, or the drive is not NTFS -
/// the walk is used). `Some(Err)` = it was tried and failed (the walk is used).
pub fn open(letter: char, ctl: &ScanControl) -> Option<Result<Box<dyn DriveListing>>> {
    if !ours::installed() {
        return None;
    }
    let exe = ours::exe()?;
    let letter = letter.to_ascii_uppercase();
    let drive = ours::ntfs_drives().into_iter().find(|d| d.letter == letter)?;
    let base = std::env::var("LOCALAPPDATA").ok()?;
    let dir = std::path::Path::new(&base).join("BoylerUtilities").join("EverythingStorage").join(letter.to_string());
    Some(start(exe, dir, drive, ctl).map(|l| Box::new(l) as Box<dyn DriveListing>))
}

fn err(what: &str) -> StorageError {
    StorageError::Unsupported(format!("Everything ({what})"))
}

fn start(exe: PathBuf, dir: PathBuf, drive: ours::Drive, ctl: &ScanControl) -> Result<EverythingListing> {
    use std::os::windows::process::CommandExt;
    const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
    // wait for our turn (a Stop ends the wait)
    let turn = loop {
        match SESSION.try_lock() {
            Ok(g) => break g,
            Err(std::sync::TryLockError::Poisoned(p)) => break p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                if ctl.is_cancelled() {
                    return Err(StorageError::Cancelled);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    };
    std::fs::create_dir_all(&dir)?;
    // an index without today's mark (older settings, or a save that was cut short) is thrown away
    let (db, mark) = (dir.join("Everything.db"), dir.join("index.ok"));
    if !std::fs::read_to_string(&mark).map(|s| s == MARK).unwrap_or(false) {
        let _ = std::fs::remove_file(&db);
    }
    let _ = std::fs::remove_file(&mark);
    let ini_path = dir.join("Everything.ini");
    let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    std::fs::write(&ini_path, ini(&windir, ours::PIPE, &drive))?;
    if let Err(e) = ours::service_start(Duration::from_secs(10)) {
        // it may have started before the failure (a timeout): never leave it running
        ours::service_stop();
        return Err(err(&e.to_string()));
    }
    let spawned = std::process::Command::new(&exe)
        .creation_flags(BELOW_NORMAL_PRIORITY_CLASS)
        .arg("-instance")
        .arg(host::STORAGE_INSTANCE)
        .arg("-startup")
        .arg("-config")
        .arg(&ini_path)
        .arg("-db")
        .arg(&db)
        .spawn();
    let child = match spawned {
        Ok(c) => c,
        Err(e) => {
            ours::service_stop();
            return Err(err(&format!("could not start it: {e}")));
        }
    };
    let mut listing = EverythingListing { window: HWND::default(), letter: drive.letter, at: [0, 0], instance: Some(host::Ours::new(child)), exe, dir, _turn: turn };
    // from here a failure (or a Stop) drops `listing`, which quits Everything and our service again
    let class = everything::class_of(Some(host::STORAGE_INSTANCE));
    let began = Instant::now();
    let until = began + READY_WAIT;
    let mut loaded_at: Option<Instant> = None;
    let mut service_checked = began;
    loop {
        if ctl.is_cancelled() {
            return Err(StorageError::Cancelled);
        }
        if let Some(h) = everything::find(&class) {
            listing.window = h;
            if everything::db_loaded(h) {
                if everything::ntfs_drives(h).contains(drive.letter) {
                    break;
                }
                // loaded but the drive never shows up (no service, another volume name): the walk is used after 15 s
                if loaded_at.get_or_insert_with(Instant::now).elapsed() > Duration::from_secs(15) {
                    return Err(err("it does not hold the drive"));
                }
            }
        }
        // the Search tab closing at the same moment may have stopped the service just before our instance showed up
        if service_checked.elapsed() > Duration::from_secs(1) {
            service_checked = Instant::now();
            if ours::service_state() == Some(1) {
                let _ = ours::service_start(Duration::from_secs(5));
            }
        }
        if Instant::now() > until {
            return Err(err("its index was not ready in time"));
        }
        if let Some(inst) = listing.instance.as_mut() {
            if matches!(inst.child.try_wait(), Ok(Some(_))) {
                return Err(err("it ended by itself"));
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(listing)
}

impl DriveListing for EverythingListing {
    fn page(&mut self, folders: bool, max: usize) -> Result<Vec<ListEntry>> {
        let text = format!("{}:\\ {}", self.letter, if folders { "folder:" } else { "file:" });
        let at = &mut self.at[if folders { 0 } else { 1 }];
        let page = everything::list_page(self.window, &text, *at, max as u32, Duration::from_secs(120)).map_err(|e| err(&e.to_string()))?;
        *at += page.items.len() as u32;
        Ok(page
            .items
            .into_iter()
            .map(|i| ListEntry {
                dir: i.dir,
                name: i.name,
                size: if i.is_folder || i.size == u64::MAX { 0 } else { i.size },
                // OFFLINE, RECALL_ON_DATA_ACCESS: online-only cloud files (they take no room on the drive) - the same two the walk uses (RECALL_ON_OPEN is NOT one: the Xbox app's game files carry it and are all there)
                cloud_only: !i.is_folder && i.attrs & (0x1000 | 0x0040_0000) != 0,
            })
            .collect())
    }
}

impl Drop for EverythingListing {
    /// Quit Everything right away (a loaded index is saved on the way out, and marked whole when it was), then our service.
    fn drop(&mut self) {
        if let Some(inst) = self.instance.take() {
            let loaded = !self.window.is_invalid() && everything::db_loaded(self.window);
            // one still building its index (Stop pressed early) would finish the build first - minutes on a big drive: it is
            // ended after 1 s and leaves no index; a loaded one gets the time to save its index
            let wait = if loaded { host::SAVE_WAIT } else { Duration::from_secs(1) };
            let clean = host::stop_instance(Some(&self.exe), host::STORAGE_INSTANCE, inst, wait);
            if clean && loaded {
                let _ = std::fs::write(self.dir.join("index.ok"), MARK);
            }
        }
    }
}
