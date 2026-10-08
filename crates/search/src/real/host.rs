//! Everything the program: where it is, our own hidden instance (started with the Search tab, quit with the menu), and the
//! one-click install (the owner, Oct 8: "Install Everything" downloads the official MSI from voidtools.com, checks it, runs it
//! silently; Windows asks for admin once).
//!
//! Our instance is a NAMED one (`-instance BoylerUtilities`) with its own settings file (no tray icon, no window, no update
//! check, not run at sign-in) and its own index file in `%LOCALAPPDATA%\BoylerUtilities\Everything`, so a copy of Everything
//! the user runs themselves is never touched (and is used instead when it runs). The switches (`-instance`, `-startup`,
//! `-config`, `-db`, `-exit`) and the settings were run on Everything 1.4.1.1032 in Order 043 (a scratch instance):
//! a first build of five drives took 75 - 80 s, loading the saved index 9.3 s, saving it on quit 7.7 - 9.6 s.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use crate::error::{Result, SearchError};

/// Our instance's name (its IPC window is `EVERYTHING_TASKBAR_NOTIFICATION_(BoylerUtilities)`).
pub const INSTANCE: &str = "BoylerUtilities";

/// The installer the owner approved (the boss, Oct 8): its address and SHA-256.
pub const MSI_HOST: &str = "www.voidtools.com";
pub const MSI_PATH: &str = "/Everything-1.4.1.1032.x64.msi";
pub const MSI_SHA256: &str = "4e7a80885aab8566b750c56a2a2b3e7c3c1f4920bcc53777e07f5eeb9e1f7485";

/// Order 049: that MSI writes an all-users "start Everything at sign-in" Run value and has no property to turn it off (its
/// tables: no START_ON_STARTUP; the value is component StartOnStartup with no condition). This transform
/// (tools/installer/everything_mst.ps1) gives that component the condition 1=0, so msiexec skips it (dry-run costing:
/// requested 3 = installed without it, -1 = skipped with it). The Everything service stays: our instance reads every drive
/// through it.
pub const NO_STARTUP_MST: &[u8] = include_bytes!("../../assets/everything-no-startup.mst");

/// Our instance's settings (Everything.ini): everything that would show or stay is off. `service_pipe_name`: a NAMED
/// instance looks for a service pipe of its own name, finds none ("Open pipe failed 2" in Everything's debug log) and then
/// reads only the drives a normal user may open - on a test PC only one of its five drives, so the others were missing from every
/// search (measured, Order 043). It is pointed at the one pipe the Everything service makes.
///
/// Order 049 (RAM): only the indexes our query needs. It sorts by name (always kept) and asks for size + date modified
/// (`everything::query`), so those two stay indexed but without their own sorted copies (`fast_*_sort=0`); no list of recent
/// changes; and the big folders nobody searches for are left out (`EXCLUDE`).
pub const INI: &str = "[Everything]\r\nrun_in_background=1\r\nshow_tray_icon=0\r\nrun_on_system_startup=0\r\ncheck_for_updates_on_startup=0\r\nshow_in_taskbar=0\r\nrun_as_admin=0\r\nservice_pipe_name=\\\\.\\PIPE\\Everything Service\r\nindex_size=1\r\nindex_date_modified=1\r\nfast_size_sort=0\r\nfast_date_modified_sort=0\r\nfast_path_sort=0\r\nfast_extension_sort=0\r\nindex_recent_changes=0\r\nexclude_list_enabled=1\r\n";

/// The folders our index leaves out (Everything's wildcards, `*` = any text): Windows' component store, every drive's
/// recycle bin, and build output (node_modules, Rust's target\debug + target\release). `{windir}` = the Windows folder.
pub const EXCLUDE: [&str; 5] = [r"{windir}\WinSxS", r"*:\$Recycle.Bin", r"*\node_modules", r"*\target\debug", r"*\target\release"];

/// Our instance's whole settings file: `INI` + the left-out folders (Everything.ini's list form: each in quotes, `\`
/// doubled, comma between).
pub fn ini(windir: &str) -> String {
    let list: Vec<String> = EXCLUDE.iter().map(|f| format!("\"{}\"", f.replace("{windir}", windir.trim_end_matches('\\')).replace('\\', "\\\\"))).collect();
    format!("{INI}exclude_folders={}\r\n", list.join(","))
}

/// What `index.ok` holds when OUR index was made with today's settings and saved whole (Everything quit by itself after
/// its index was loaded). Another text (older settings) throws the index away.
pub const INDEX_MARK: &str = "BoylerUtilities index: service pipe, lean (049)";

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

/// Where Everything.exe is (the installer's folders), if it is on this PC.
pub fn exe() -> Option<PathBuf> {
    let mut c = Vec::new();
    for v in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Ok(p) = std::env::var(v) {
            c.push(Path::new(&p).join("Everything").join("Everything.exe"));
        }
    }
    if let Ok(p) = std::env::var("LOCALAPPDATA") {
        c.push(Path::new(&p).join("Programs").join("Everything").join("Everything.exe"));
    }
    c.into_iter().find(|p| p.is_file())
}

/// Our instance's folder (settings + index).
pub fn data_dir() -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA").ok().map(|p| Path::new(&p).join("BoylerUtilities").join("Everything"))
}

/// Our instance's index file in its folder (Everything saves it when it quits).
pub fn db_file(dir: &Path) -> PathBuf {
    dir.join("Everything.db")
}

/// Start our instance hidden, at below-normal priority (Order 049: its index build never competes with a game or the
/// desktop). Writes its settings file first (each time: the user never edits ours).
pub fn start_ours(exe: &Path, dir: &Path) -> Result<Child> {
    use std::os::windows::process::CommandExt;
    const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
    std::fs::create_dir_all(dir).map_err(|e| SearchError::Everything(format!("its folder: {e}")))?;
    let ini = dir.join("Everything.ini");
    let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    std::fs::write(&ini, self::ini(&windir)).map_err(|e| SearchError::Everything(format!("its settings: {e}")))?;
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
    let mut ours = ours;
    let child = &mut ours.child;
    if let Some(exe) = exe {
        if let Ok(mut c) = Command::new(exe).arg("-instance").arg(INSTANCE).arg("-exit").spawn() {
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
    while Instant::now() < until {
        if let Ok(Some(_)) = child.try_wait() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    false
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

/// Run the MSI silently, elevated (Windows' admin prompt), with the transform that leaves out its start-at-sign-in. Ok =
/// installed (exit 0, or 3010 "restart needed later").
pub fn run_msi(msi: &Path, mst: &Path) -> Result<()> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    let verb = super::wide("runas");
    // Windows' own msiexec by its full path (never one found by name in the current or the app's folder)
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let file = super::wide(&format!(r"{root}\System32\msiexec.exe"));
    // ShellExecuteEx wants COM on its thread
    let _com = super::Com::sta();
    let params = super::wide(&msi_params(msi, mst));
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
            return Err(SearchError::Install(format!("the installer did not start: {}", e.message())));
        }
        if sei.hProcess.is_invalid() {
            return Err(SearchError::Install("the installer did not start".into()));
        }
        let w = WaitForSingleObject(sei.hProcess, INFINITE);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(sei.hProcess, &mut code);
        let _ = CloseHandle(sei.hProcess);
        if w != WAIT_OBJECT_0 {
            return Err(SearchError::Install("the installer did not finish".into()));
        }
        match code {
            0 | 3010 => Ok(()),
            1602 => Err(SearchError::InstallCancelled),
            c => Err(SearchError::Install(format!("the installer stopped (code {c})"))),
        }
    }
}

/// msiexec's command line: install silently, with the transform, no restart.
pub fn msi_params(msi: &Path, mst: &Path) -> String {
    format!("/i \"{}\" TRANSFORMS=\"{}\" /qn /norestart", msi.display(), mst.display())
}

/// The whole install: download, check the SHA-256 (a mismatch never runs), run, delete the file.
pub fn install() -> Result<()> {
    let data = download(MSI_HOST, MSI_PATH)?;
    if sha256_hex(&data)? != MSI_SHA256 {
        return Err(SearchError::Install("the download did not match voidtools' file - nothing was installed".into()));
    }
    let dir = std::env::temp_dir().join("BoylerUtilities");
    std::fs::create_dir_all(&dir).map_err(|e| SearchError::Install(format!("could not save it: {e}")))?;
    let msi = dir.join("Everything-1.4.1.1032.x64.msi");
    std::fs::write(&msi, &data).map_err(|e| SearchError::Install(format!("could not save it: {e}")))?;
    // held open with read-only sharing while the installer runs, and checked again through that handle: nothing can swap
    // the file between the check and the elevated install
    let lock = {
        use std::io::Read;
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        let mut f = std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(&msi).map_err(|e| SearchError::Install(format!("could not check it: {e}")))?;
        let mut again = Vec::new();
        f.read_to_end(&mut again).map_err(|e| SearchError::Install(format!("could not check it: {e}")))?;
        if sha256_hex(&again)? != MSI_SHA256 {
            drop(f);
            let _ = std::fs::remove_file(&msi);
            return Err(SearchError::Install("the saved file did not match voidtools' file - nothing was installed".into()));
        }
        f
    };
    // the transform (ours, from inside the app): saved, held and checked the same way
    let mst = dir.join("everything-no-startup.mst");
    if let Err(e) = std::fs::write(&mst, NO_STARTUP_MST) {
        drop(lock);
        let _ = std::fs::remove_file(&mst);
        let _ = std::fs::remove_file(&msi);
        return Err(SearchError::Install(format!("could not save it: {e}")));
    }
    let mst_lock = {
        use std::io::Read;
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        let held = std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(&mst).and_then(|mut f| {
            let mut again = Vec::new();
            f.read_to_end(&mut again).map(|_| (f, again))
        });
        match held {
            Ok((f, again)) if again == NO_STARTUP_MST => f,
            _ => {
                drop(lock);
                let _ = std::fs::remove_file(&mst);
                let _ = std::fs::remove_file(&msi);
                return Err(SearchError::Install("could not check its settings file - nothing was installed".into()));
            }
        }
    };
    let r = run_msi(&msi, &mst);
    drop(lock);
    drop(mst_lock);
    let _ = std::fs::remove_file(&msi);
    let _ = std::fs::remove_file(&mst);
    r?;
    if exe().is_none() {
        return Err(SearchError::Install("the installer finished but Everything is not where it should be".into()));
    }
    Ok(())
}
