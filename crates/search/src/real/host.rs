//! Everything the program: our own copy (`ours`: set up by one admin prompt), our hidden instance (started with the Search
//! tab, quit when the tab closes), and its settings.
//!
//! Our instance is a NAMED one (`-instance BoylerUtilities`) with its own settings file (no tray icon, no window, no update
//! check, not run at sign-in) and its own index file in `%LOCALAPPDATA%\BoylerUtilities\Everything`, so a copy of Everything
//! the user runs themselves is never touched (and is used instead when it runs). The switches (`-instance`, `-startup`,
//! `-config`, `-db`, `-exit`, `-reindex`) and the settings were run on Everything 1.4.1.1032 (Orders 043, 049, scratch
//! instances): a first build of five drives took 75 - 80 s, loading the saved index 9.3 s, saving it on quit 7.7 - 9.6 s.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use super::ours::{self, Drive};
use crate::error::{Result, SearchError};

/// Our instance's name (its IPC window is `EVERYTHING_TASKBAR_NOTIFICATION_(BoylerUtilities)`).
pub const INSTANCE: &str = "BoylerUtilities";
/// Storage's own instance (Order 069): started when Measure is pressed, quit when the measure is done.
pub const STORAGE_INSTANCE: &str = "BoylerUtilitiesStorage";

/// Our instance's settings (Everything.ini): everything that would show or stay is off; only the drives `ini` lists
/// (no drive is taken by itself).
///
/// Order 049 (RAM): only the indexes our query needs. It sorts by name (always kept) and asks for size + date modified
/// (`everything::query`), so those two stay indexed but without their own sorted copies (`fast_*_sort=0`); no list of recent
/// changes; and the big folders nobody searches for are left out (`EXCLUDE`). Measured on a PC with 10.1 M files on 5
/// drives: 830 MB -> 552 MB; the Windows drive alone (the default, item 6): 186 MB.
pub const INI: &str = "[Everything]\r\nrun_in_background=1\r\nshow_tray_icon=0\r\nrun_on_system_startup=0\r\ncheck_for_updates_on_startup=0\r\nshow_in_taskbar=0\r\nrun_as_admin=0\r\nindex_size=1\r\nindex_date_modified=1\r\nfast_size_sort=0\r\nfast_date_modified_sort=0\r\nfast_path_sort=0\r\nfast_extension_sort=0\r\nindex_recent_changes=0\r\nexclude_list_enabled=1\r\nauto_include_fixed_volumes=0\r\nauto_include_fixed_refs_volumes=0\r\nauto_include_removable_volumes=0\r\n";

/// The folders our index leaves out (Everything's wildcards, `*` = any text, any depth - checked Oct 9): Windows'
/// component store, every drive's recycle bin, and build output (node_modules, Rust's target\debug + target\release).
/// `{windir}` = the Windows folder.
pub const EXCLUDE: [&str; 5] = [r"{windir}\WinSxS", r"*:\$Recycle.Bin", r"*\node_modules", r"*\target\debug", r"*\target\release"];

/// Everything.ini's list form: each in quotes, `\` doubled, comma between.
fn ini_list<'a>(items: impl Iterator<Item = &'a str>) -> String {
    items.map(|s| format!("\"{}\"", s.replace('\\', "\\\\"))).collect::<Vec<_>>().join(",")
}

/// Our instance's whole settings file: `INI` + the service pipe it reads the drives through (OUR service's) + the drives
/// it covers (only these, Order 049 item 6: the volume GUID is what makes Everything take a drive - a letter alone indexed
/// nothing, tried Oct 9) + the left-out folders.
pub fn ini(windir: &str, pipe: &str, drives: &[Drive]) -> String {
    let w = windir.trim_end_matches('\\');
    let ex: Vec<String> = EXCLUDE.iter().map(|f| f.replace("{windir}", w)).collect();
    let letters: Vec<String> = drives.iter().map(|d| format!("{}:", d.letter)).collect();
    let ones = vec!["1"; drives.len()].join(",");
    let zeros = vec!["0"; drives.len()].join(",");
    format!(
        "{INI}service_pipe_name={pipe}\r\nntfs_volume_guids={}\r\nntfs_volume_paths={}\r\nntfs_volume_roots={}\r\nntfs_volume_includes={ones}\r\nntfs_volume_load_recent_changes={zeros}\r\nexclude_folders={}\r\n",
        ini_list(drives.iter().map(|d| d.guid.as_str())),
        ini_list(letters.iter().map(|s| s.as_str())),
        ini_list(drives.iter().map(|_| "")),
        ini_list(ex.iter().map(|s| s.as_str())),
    )
}

/// What `index.ok` holds when OUR index was made with today's settings and saved whole (Everything quit by itself after
/// its index was loaded). Another text (older settings) throws the index away.
pub const INDEX_MARK: &str = "BoylerUtilities index: our service, lean, picked drives (049)";

/// The mark file next to our index.
pub fn mark_file(dir: &Path) -> PathBuf {
    dir.join("index.ok")
}

/// Before ours starts: an index without today's mark (made with older settings, or its save was cut short) is thrown
/// away - only OUR folder's. The mark goes too, until this run saves cleanly again. True = no index: Everything builds its
/// file list (the page says "building").
pub fn prepare_index(dir: &Path) -> bool {
    let db = db_file(dir);
    let mark = mark_file(dir);
    let ok = std::fs::read_to_string(&mark).map(|s| s == INDEX_MARK).unwrap_or(false);
    if !ok {
        let _ = std::fs::remove_file(&db);
    }
    let _ = std::fs::remove_file(&mark);
    !db.is_file()
}

/// Ours quit by itself after its index was loaded: Everything saved it whole.
pub fn mark_index(dir: &Path) {
    let _ = std::fs::write(mark_file(dir), INDEX_MARK);
}

/// How long a quitting instance may take to save a loaded index (measured on a test PC: 7.7 - 9.6 s for a large index).
pub const SAVE_WAIT: Duration = Duration::from_secs(30);

/// Our Everything.exe (Order 049: only our own copy, never the user's).
pub fn exe() -> Option<PathBuf> {
    ours::exe()
}

/// Our instance's folder (settings + index).
pub fn data_dir() -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA").ok().map(|p| Path::new(&p).join("BoylerUtilities").join("Everything"))
}

/// Our instance's index file in its folder (Everything saves it when it quits).
pub fn db_file(dir: &Path) -> PathBuf {
    dir.join("Everything.db")
}

/// Start our service, then our instance hidden at below-normal priority (its index build never competes with a game or
/// the desktop), covering `drives`. Writes its settings file first (each time: the user never edits ours).
pub fn start_ours(exe: &Path, dir: &Path, drives: &[Drive]) -> Result<Child> {
    ours::service_start(Duration::from_secs(10))?;
    // the service never stays running without our instance (review): any failure below stops it again
    let r = start_client(exe, dir, drives);
    if r.is_err() {
        ours::service_stop();
    }
    r
}

pub fn start_client(exe: &Path, dir: &Path, drives: &[Drive]) -> Result<Child> {
    use std::os::windows::process::CommandExt;
    const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
    std::fs::create_dir_all(dir).map_err(|e| SearchError::Everything(format!("its folder: {e}")))?;
    let ini = dir.join("Everything.ini");
    let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    std::fs::write(&ini, self::ini(&windir, ours::PIPE, drives)).map_err(|e| SearchError::Everything(format!("its settings: {e}")))?;
    let db = db_file(dir);
    Command::new(exe)
        .creation_flags(BELOW_NORMAL_PRIORITY_CLASS)
        .arg("-instance")
        .arg(INSTANCE)
        .arg("-startup")
        .arg("-config")
        .arg(&ini)
        .arg("-db")
        .arg(&db)
        .spawn()
        .map_err(|e| SearchError::Everything(format!("could not start it: {e}")))
}

/// Our instance: the process + the job object that ties it to the app.
pub struct Ours {
    pub child: Child,
    job: Option<windows::Win32::Foundation::HANDLE>,
}

// the job handle is only closed (once, in Drop)
unsafe impl Send for Ours {}

impl Ours {
    /// Tie the started process to this app with a kill-on-close job object: if the app ends without quitting it (a
    /// crash, Task Manager), Windows ends it too - it can never stay behind using RAM.
    pub fn new(child: Child) -> Ours {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::System::JobObjects::*;
        let job = unsafe {
            CreateJobObjectW(None, windows::core::PCWSTR::null()).ok().and_then(|j| {
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let set = SetInformationJobObject(j, JobObjectExtendedLimitInformation, &info as *const _ as *const _, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32);
                let tied = set.is_ok() && AssignProcessToJobObject(j, HANDLE(child.as_raw_handle())).is_ok();
                if tied {
                    Some(j)
                } else {
                    let _ = CloseHandle(j);
                    None
                }
            })
        };
        Ours { child, job }
    }
}

impl Drop for Ours {
    fn drop(&mut self) {
        if let Some(j) = self.job.take() {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(j);
            }
        }
    }
}

/// Quit our instance: Everything's own `-exit` for it, then (if it is still there after `wait`) the process we started.
/// A loaded index is saved on `-exit` (give it `SAVE_WAIT`); one still being built is not - Everything would finish the
/// build first (minutes), so it is ended after 3 s and leaves no index file (measured). True = it quit by itself.
pub fn stop_ours(exe: Option<&Path>, ours: Ours, wait: Duration) -> bool {
    stop_instance(exe, INSTANCE, ours, wait)
}

/// [`stop_ours`] for any of our instances (Order 069: Storage's own, `BoylerUtilitiesStorage`).
pub fn stop_instance(exe: Option<&Path>, instance: &str, ours: Ours, wait: Duration) -> bool {
    let mut ours = ours;
    let child = &mut ours.child;
    if let Some(exe) = exe {
        if let Ok(mut c) = Command::new(exe).arg("-instance").arg(instance).arg("-exit").spawn() {
            // the helper gets 3 s too (never a wait without end: this runs at app exit)
            let until = Instant::now() + Duration::from_secs(3);
            while !matches!(c.try_wait(), Ok(Some(_))) && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(50));
            }
            if !matches!(c.try_wait(), Ok(Some(_))) {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
    let until = Instant::now() + wait;
    let mut clean = false;
    while Instant::now() < until {
        if let Ok(Some(_)) = child.try_wait() {
            clean = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if !clean {
        let _ = child.kill();
        let _ = child.wait();
    }
    // Order 049: nothing of Everything stays running - our service stops with it
    ours::service_stop();
    clean
}

/// "Update search" (Order 049 item 8): our running instance makes its file list again (`-reindex`).
pub fn reindex_ours(exe: &Path) -> Result<()> {
    let mut c = Command::new(exe).arg("-instance").arg(INSTANCE).arg("-reindex").spawn().map_err(|e| SearchError::Everything(format!("could not ask it: {e}")))?;
    let until = Instant::now() + Duration::from_secs(5);
    while !matches!(c.try_wait(), Ok(Some(_))) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    if !matches!(c.try_wait(), Ok(Some(_))) {
        let _ = c.kill();
        let _ = c.wait();
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------------- install

/// SHA-256 of the bytes, lower-case hex (Windows' CNG).
pub fn sha256_hex(data: &[u8]) -> Result<String> {
    use windows::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
    let mut out = [0u8; 32];
    let st = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut out) };
    if st.0 != 0 {
        return Err(SearchError::Install(format!("could not check the download (0x{:08x})", st.0)));
    }
    Ok(out.iter().map(|b| format!("{b:02x}")).collect())
}

/// Download over HTTPS (WinHTTP, the system's proxy settings) into memory.
pub fn download(host: &str, path: &str) -> Result<Vec<u8>> {
    use windows::core::PCWSTR;
    use windows::Win32::Networking::WinHttp::*;
    let fail = |what: &str| SearchError::Install(format!("the download failed ({what})"));
    let w = |s: &str| super::wide(s);
    let (agent, h, p, get) = (w("BoylerUtilities"), w(host), w(path), w("GET"));
    unsafe {
        let s = WinHttpOpen(PCWSTR(agent.as_ptr()), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0);
        if s.is_null() {
            return Err(fail("no connection"));
        }
        struct H(*mut core::ffi::c_void);
        impl Drop for H {
            fn drop(&mut self) {
                if !self.0.is_null() {
                    unsafe {
                        let _ = WinHttpCloseHandle(self.0);
                    }
                }
            }
        }
        let s = H(s);
        let _ = WinHttpSetTimeouts(s.0, 10_000, 10_000, 30_000, 30_000);
        let c = H(WinHttpConnect(s.0, PCWSTR(h.as_ptr()), INTERNET_DEFAULT_HTTPS_PORT, 0));
        if c.0.is_null() {
            return Err(fail("no connection"));
        }
        let r = H(WinHttpOpenRequest(c.0, PCWSTR(get.as_ptr()), PCWSTR(p.as_ptr()), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE));
        if r.0.is_null() {
            return Err(fail("request"));
        }
        WinHttpSendRequest(r.0, None, None, 0, 0, 0).map_err(|_| fail("no answer"))?;
        WinHttpReceiveResponse(r.0, std::ptr::null_mut()).map_err(|_| fail("no answer"))?;
        let mut code = 0u32;
        let mut len = 4u32;
        WinHttpQueryHeaders(r.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some(&mut code as *mut u32 as *mut _), &mut len, std::ptr::null_mut())
            .map_err(|_| fail("no status"))?;
        if code != 200 {
            return Err(fail(&format!("HTTP {code}")));
        }
        let mut out = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut got = 0u32;
            WinHttpReadData(r.0, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut got).map_err(|_| fail("cut off"))?;
            if got == 0 {
                break;
            }
            out.extend_from_slice(&buf[..got as usize]);
            if out.len() > 64 * 1024 * 1024 {
                return Err(fail("too big"));
            }
        }
        Ok(out)
    }
}

/// Start `file params` with Windows' admin prompt (hidden) and wait for it: its exit code. A "No" on the prompt =
/// `InstallCancelled`.
pub fn run_elevated(file: &Path, params: &str) -> Result<u32> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    let verb = super::wide("runas");
    let file = super::wide(&file.display().to_string());
    // ShellExecuteEx wants COM on its thread
    let _com = super::Com::sta();
    let params = super::wide(params);
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        if let Err(e) = ShellExecuteExW(&mut sei) {
            if e.code() == ERROR_CANCELLED.to_hresult() {
                return Err(SearchError::InstallCancelled);
            }
            return Err(SearchError::Install(format!("the set-up did not start: {}", e.message())));
        }
        if sei.hProcess.is_invalid() {
            return Err(SearchError::Install("the set-up did not start".into()));
        }
        let w = WaitForSingleObject(sei.hProcess, INFINITE);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(sei.hProcess, &mut code);
        let _ = CloseHandle(sei.hProcess);
        if w != WAIT_OBJECT_0 {
            return Err(SearchError::Install("the set-up did not finish".into()));
        }
        Ok(code)
    }
}

/// The whole install (Order 049: our own copy + our service, `ours::install`); `tidy` also removes the Everything v1.0.0
/// put on the PC (A_049_02's rule).
pub fn install(tidy: bool) -> Result<()> {
    ours::install(tidy)
}
