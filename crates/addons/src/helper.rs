//! The elevated helper: the app's own exe started with Windows' admin prompt as
//! `BoylerUtilities.exe --addon-helper rawaccel-install|rawaccel-uninstall "<folder>"` (main.rs hands it over before
//! anything else starts). It
//! 1. reads Raw Accel's tool (and for an install the driver) from the folder and checks their SHA-256 against the pinned
//!    official files, then writes THOSE checked bytes into a fresh folder only SYSTEM and Administrators can write to
//!    (`%SystemRoot%\Temp\BU-addon-<random>`, a protected access list) and runs the tool from there: nothing a normal
//!    program does can swap the tool after the check or plant a DLL next to it (the tool is unsigned and loads system
//!    DLLs by name - from the user-writable add-ons folder a planted one would run with admin rights); the folder is
//!    deleted afterwards;
//! 2. runs the tool hidden inside a pseudoconsole (ConPTY: no window, but the tool sees a real console, so its
//!    `std::cout` lines arrive at once and its closing `_getwch()` - "Press any key to close this window" - reads the
//!    Enter the helper types once that line shows);
//! 3. exits 0 when the tool said it worked, else one of the [`Code`]s. It never writes anything into the folder it was
//!    given (a path from its command line: an admin write there could be bent to any file - Order 037 end review).

use crate::os::HelperAction;
use crate::rawaccel::{OFFICIAL_UNINSTALLERS, PIN};
use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::FromRawHandle;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const ARG: &str = "--addon-helper";

/// The helper's exit codes (its only answer; the app turns them into a line with [`message`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    Done = 0,
    BadArgs = 2,
    NotOfficial = 3,
    Prepare = 4,
    ToolError = 5,
    NotFinished = 6,
    NoStart = 7,
}

/// The line the app shows for a helper's exit code.
pub fn message(action: HelperAction, code: u32) -> String {
    let tool = if action == HelperAction::RawAccelInstall { "installer" } else { "uninstaller" };
    match code {
        2 => "The admin helper was started wrongly".into(),
        3 => "Raw Accel\u{2019}s files were not the official ones - nothing was run".into(),
        4 => "Could not prepare Raw Accel\u{2019}s tool".into(),
        5 => format!("Raw Accel\u{2019}s {tool} reported an error"),
        6 => format!("Raw Accel\u{2019}s {tool} did not finish"),
        7 => format!("Raw Accel\u{2019}s {tool} did not start"),
        c => format!("The admin helper stopped (code {c})"),
    }
}
/// Raw Accel's tools finish in well under a second; this only stops a stuck one.
pub const TOOL_TIMEOUT: Duration = Duration::from_secs(120);

/// main.rs, first thing: `if let Some(code) = bu_addons::helper::run_if_requested(&args) { std::process::exit(code) }`.
pub fn run_if_requested(args: &[String]) -> Option<i32> {
    if args.get(1).map(String::as_str) != Some(ARG) {
        return None;
    }
    let (Some(action), Some(folder)) = (args.get(2).and_then(|a| HelperAction::parse(a)), args.get(3).map(PathBuf::from)) else {
        return Some(Code::BadArgs as i32);
    };
    Some(match run(action, &folder) {
        Ok(()) => Code::Done,
        Err((c, _)) => c,
    } as i32)
}

/// Read `file` (writes / deletes refused while it is read) and check its SHA-256 is one of `allowed`. Returns the checked
/// bytes.
pub fn read_checked(file: &Path, allowed: &[&str]) -> Result<Vec<u8>, String> {
    const FILE_SHARE_READ: u32 = 1;
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut f = std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(file).map_err(|e| format!("Could not open {name}: {e}"))?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).map_err(|e| format!("Could not read {name}: {e}"))?;
    if !allowed.contains(&crate::sha256_hex(&bytes).as_str()) {
        return Err(format!("{name} is not Raw Accel\u{2019}s official file - nothing was run"));
    }
    Ok(bytes)
}

/// SYSTEM + Administrators only, nothing inherited from the parent (`P` = protected).
pub const ADMIN_ONLY: &str = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";

/// A new folder `BU-addon-<random>` in `parent` with the access list `sddl` (CreateDirectoryW makes it with that list, so
/// it is never open to anyone else, not even for a moment). A name that exists already is never reused.
pub fn private_dir(parent: &Path, sddl: &str) -> Result<PathBuf, String> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::Storage::FileSystem::CreateDirectoryW;
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: the descriptor is freed below with LocalFree, after its last use.
    unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(&HSTRING::from(sddl), SDDL_REVISION_1, &mut sd, None) }
        .map_err(|e| format!("Could not make a private folder ({})", e.message()))?;
    let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd.0, bInheritHandle: false.into() };
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0) ^ ((std::process::id() as u64) << 32);
    let mut made = Err("Could not make a private folder".to_string());
    for i in 0..16u64 {
        let p = parent.join(format!("BU-addon-{:016x}", seed.wrapping_add(i).wrapping_mul(0x9E37_79B9_7F4A_7C15)));
        // SAFETY: sa and its descriptor are alive for the call.
        if unsafe { CreateDirectoryW(&HSTRING::from(p.as_os_str()), Some(&sa)) }.is_ok() {
            made = Ok(p);
            break;
        }
    }
    // SAFETY: made by ConvertStringSecurityDescriptorToSecurityDescriptorW above.
    unsafe { LocalFree(Some(HLOCAL(sd.0))) };
    made
}

fn run(action: HelperAction, folder: &Path) -> Result<(), (Code, String)> {
    let nof = |e: String| (Code::NotOfficial, e);
    let prep = |e: String| (Code::Prepare, e);
    // the checked bytes, by the name each must have where the tool runs (the installer copies driver\rawaccel.sys)
    let files: Vec<(&str, Vec<u8>)> = match action {
        HelperAction::RawAccelInstall => vec![
            ("installer.exe", read_checked(&folder.join("installer.exe"), &[PIN.installer_sha256]).map_err(nof)?),
            ("driver\\rawaccel.sys", read_checked(&folder.join("driver").join("rawaccel.sys"), &[PIN.driver_sha256]).map_err(nof)?),
        ],
        HelperAction::RawAccelUninstall => vec![("uninstaller.exe", read_checked(&folder.join("uninstaller.exe"), &OFFICIAL_UNINSTALLERS).map_err(nof)?)],
    };
    // Windows' own folder from Windows (never %SystemRoot%: a user can set that for their processes) and its Temp only when
    // it is a real folder (not a link elsewhere)
    let win = windows_dir().ok_or_else(|| prep("no Windows folder".into()))?;
    let root = win.join("Temp");
    if is_reparse(&win) || is_reparse(&root) {
        return Err(prep("Windows' Temp folder is redirected".into()));
    }
    let work = private_dir(&root, ADMIN_ONLY).map_err(prep)?;
    let out = (|| {
        for (name, bytes) in &files {
            let p = work.join(name);
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d).map_err(|e| format!("Could not prepare Raw Accel\u{2019}s tool: {e}"))?;
            }
            std::fs::write(&p, bytes).map_err(|e| format!("Could not prepare Raw Accel\u{2019}s tool: {e}"))?;
        }
        Ok(())
    })()
    .map_err(prep)
    .and_then(|()| run_tool_env(&work.join(files[0].0), &[], TOOL_TIMEOUT, Some(&tool_env(&win, &work))).map_err(|e| (if e.contains("in time") { Code::NotFinished } else { Code::NoStart }, e)));
    let _ = std::fs::remove_dir_all(&work);
    let out = out?;
    judge(action, &out).map_err(|e| (if e.contains("Error:") { Code::ToolError } else { Code::NotFinished }, e))
}

/// Windows' folder as Windows says (GetSystemWindowsDirectoryW).
pub fn windows_dir() -> Option<PathBuf> {
    use windows::Win32::System::SystemInformation::GetSystemWindowsDirectoryW;
    let mut buf = [0u16; 260];
    // SAFETY: the buffer's length is passed with it.
    let n = unsafe { GetSystemWindowsDirectoryW(Some(&mut buf)) } as usize;
    (n > 0 && n < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
}

/// The path is a junction / symbolic link (or can't be read).
pub fn is_reparse(p: &Path) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, FILE_ATTRIBUTE_REPARSE_POINT, INVALID_FILE_ATTRIBUTES};
    // SAFETY: a plain attribute read.
    let a = unsafe { GetFileAttributesW(&HSTRING::from(p.as_os_str())) };
    a == INVALID_FILE_ATTRIBUTES || a & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
}

/// The tool's own small environment (not the user's: no user folders on its PATH to load a DLL from): Windows' folders,
/// its temp = the private run folder.
pub fn tool_env(win: &Path, work: &Path) -> String {
    let w = win.display();
    format!("SystemRoot={w}\0windir={w}\0PATH={w}\\System32;{w}\0TEMP={t}\0TMP={t}\0\0", t = work.display())
}

/// Did the tool's output say it worked? (Raw Accel's own lines, installer.cpp / uninstaller.cpp v1.7.1)
pub fn judge(action: HelperAction, out: &str) -> Result<(), String> {
    let ok = match action {
        HelperAction::RawAccelInstall => out.contains("Install complete"),
        HelperAction::RawAccelUninstall => out.contains("Removal complete") || out.contains("No installed driver found"),
    };
    if ok {
        return Ok(());
    }
    let tool = if action == HelperAction::RawAccelInstall { "installer" } else { "uninstaller" };
    match out.lines().map(str::trim).find(|l| l.starts_with("Error:")) {
        Some(l) => Err(format!("Raw Accel\u{2019}s {tool}: {l}")),
        None => Err(format!("Raw Accel\u{2019}s {tool} did not finish")),
    }
}

/// The console text without its VT control sequences (CSI `ESC [ … final`, OSC `ESC ] … BEL / ESC \`).
pub fn strip_vt(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('[') => {
                for d in it.by_ref() {
                    if ('@'..='~').contains(&d) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(d) = it.next() {
                    if d == '\u{7}' || (d == '\u{1b}' && it.peek() == Some(&'\\')) {
                        if d == '\u{1b}' {
                            it.next();
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Run `tool` (in its own folder) inside a hidden pseudoconsole; once its output shows "Press any key", type Enter.
/// Returns its console text (VT sequences stripped). A tool still running after `timeout` is ended.
pub fn run_tool(tool: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    run_tool_env(tool, args, timeout, None)
}

/// [`run_tool`] with its own environment block (`NAME=value\0…\0\0`), or the caller's (None).
pub fn run_tool_env(tool: &Path, args: &[&str], timeout: Duration, env: Option<&str>) -> Result<String, String> {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Console::{ClosePseudoConsole, CreatePseudoConsole, COORD};
    use windows::Win32::System::Pipes::CreatePipe;
    use windows::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, CREATE_UNICODE_ENVIRONMENT, InitializeProcThreadAttributeList, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject, EXTENDED_STARTUPINFO_PRESENT,
        LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    };
    let fail = |what: &str, e: windows::core::Error| format!("Could not start Raw Accel\u{2019}s tool ({what}: {})", e.message());
    let dir = tool.parent().ok_or("no folder")?;
    unsafe {
        // pipes: we write the console's input, read its output
        let (mut in_r, mut in_w, mut out_r, mut out_w) = (HANDLE::default(), HANDLE::default(), HANDLE::default(), HANDLE::default());
        CreatePipe(&mut in_r, &mut in_w, None, 0).map_err(|e| fail("pipe", e))?;
        CreatePipe(&mut out_r, &mut out_w, None, 0).map_err(|e| fail("pipe", e))?;
        let hpc = CreatePseudoConsole(COORD { X: 120, Y: 40 }, in_r, out_w, 0);
        // the console holds its own copies now
        let _ = CloseHandle(in_r);
        let _ = CloseHandle(out_w);
        let hpc = hpc.map_err(|e| fail("console", e))?;
        let input = Arc::new(Mutex::new(File::from_raw_handle(in_w.0)));
        let output = File::from_raw_handle(out_r.0);

        // the process, attached to the pseudoconsole
        let mut size = 0usize;
        let _ = InitializeProcThreadAttributeList(None, 1, None, &mut size);
        let mut attr_buf = vec![0u8; size];
        let attrs = LPPROC_THREAD_ATTRIBUTE_LIST(attr_buf.as_mut_ptr().cast());
        let started = (|| {
            InitializeProcThreadAttributeList(Some(attrs), 1, None, &mut size).map_err(|e| fail("attributes", e))?;
            UpdateProcThreadAttribute(attrs, 0, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize, Some(hpc.0 as *const _), std::mem::size_of_val(&hpc), None, None)
                .map_err(|e| fail("attributes", e))?;
            let mut si = STARTUPINFOEXW::default();
            si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
            // no standard handles of ours (a redirected parent's would be handed down instead of the pseudoconsole's:
            // microsoft/terminal#11276) - the child gets the console's own
            si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            si.lpAttributeList = attrs;
            let app: Vec<u16> = tool.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
            let mut cmd = format!("\"{}\"", tool.display());
            for a in args {
                cmd.push(' ');
                cmd.push_str(a);
            }
            let mut cmd: Vec<u16> = cmd.encode_utf16().chain([0]).collect();
            let cwd: Vec<u16> = dir.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
            let mut pi = PROCESS_INFORMATION::default();
            let envw: Option<Vec<u16>> = env.map(|e| e.encode_utf16().collect());
            let flags = if envw.is_some() { EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT } else { EXTENDED_STARTUPINFO_PRESENT };
            CreateProcessW(
                PCWSTR(app.as_ptr()),
                Some(PWSTR(cmd.as_mut_ptr())),
                None,
                None,
                false,
                flags,
                envw.as_ref().map(|e| e.as_ptr() as *const std::ffi::c_void),
                PCWSTR(cwd.as_ptr()),
                &si.StartupInfo,
                &mut pi,
            )
            .map_err(|e| fail("process", e))?;
            Ok::<_, String>(pi)
        })();
        let pi = match started {
            Ok(pi) => pi,
            Err(e) => {
                DeleteProcThreadAttributeList(attrs);
                ClosePseudoConsole(hpc);
                return Err(e);
            }
        };

        // read everything it prints; answer "Press any key" once
        let text = Arc::new(Mutex::new(String::new()));
        let reader = {
            let text = text.clone();
            let input = input.clone();
            std::thread::spawn(move || {
                let mut output = output;
                let mut buf = [0u8; 4096];
                let mut answered = false;
                while let Ok(n) = output.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let mut t = text.lock().unwrap();
                    t.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if !answered && strip_vt(&t).contains("Press any key") {
                        answered = true;
                        let _ = input.lock().unwrap().write_all(b"\r");
                    }
                }
            })
        };

        let deadline = Instant::now() + timeout;
        let mut timed_out = false;
        loop {
            if WaitForSingleObject(pi.hProcess, 100) == WAIT_OBJECT_0 {
                break;
            }
            if Instant::now() >= deadline {
                let _ = TerminateProcess(pi.hProcess, 1);
                let _ = WaitForSingleObject(pi.hProcess, 5000);
                timed_out = true;
                break;
            }
        }
        let _ = CloseHandle(pi.hThread);
        let _ = CloseHandle(pi.hProcess);
        // closing the console ends its output pipe: the reader sees the end and stops
        ClosePseudoConsole(hpc);
        let _ = reader.join();
        DeleteProcThreadAttributeList(attrs);
        drop(attr_buf);
        let out = strip_vt(&text.lock().unwrap());
        if timed_out {
            return Err("Raw Accel\u{2019}s tool did not finish in time".into());
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judges_the_official_lines() {
        let i = HelperAction::RawAccelInstall;
        let u = HelperAction::RawAccelUninstall;
        assert!(judge(i, "Install complete, change will take effect after restart.\r\nPress any key to close this window . . .").is_ok());
        assert!(judge(u, "Removal complete, change will take effect after restart.").is_ok());
        assert!(judge(u, "No installed driver found.").is_ok());
        assert_eq!(judge(i, "Error: copy_file failed: Access is denied. system:5\r\nPress any key").unwrap_err(), "Raw Accel\u{2019}s installer: Error: copy_file failed: Access is denied. system:5");
        assert!(judge(i, "").unwrap_err().contains("did not finish"));
        // the uninstaller's success line never passes for an install
        assert!(judge(i, "Removal complete").is_err());
    }

    #[test]
    fn strips_console_sequences() {
        assert_eq!(strip_vt("\u{1b}[?25l\u{1b}[2J\u{1b}[HInstall complete\u{1b}[K\r\n\u{1b}]0;title\u{7}Press"), "Install complete\r\nPress");
    }

    #[test]
    fn only_official_files_are_read() {
        let dir = std::env::temp_dir().join(format!("bu-addons-hold-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("uninstaller.exe");
        std::fs::write(&f, b"not the official one").unwrap();
        assert!(read_checked(&f, &OFFICIAL_UNINSTALLERS).unwrap_err().contains("not Raw Accel"));
        let sha = crate::sha256_hex(b"not the official one");
        assert_eq!(read_checked(&f, &[sha.as_str()]).unwrap(), b"not the official one");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The run folder: made with its own access list. With the owner added (a test runs without admin) it works as a
    /// folder; with the admin-only list a normal (not elevated) program cannot put a file into it.
    #[test]
    fn the_run_folder_is_private() {
        let parent = std::env::temp_dir();
        let d = private_dir(&parent, "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;OW)").unwrap();
        std::fs::create_dir_all(d.join("driver")).unwrap();
        std::fs::write(d.join("driver").join("x.sys"), b"1").unwrap();
        std::fs::remove_dir_all(&d).unwrap();
        let d = private_dir(&parent, ADMIN_ONLY).unwrap();
        assert!(d.file_name().unwrap().to_string_lossy().starts_with("BU-addon-"));
        let wrote = std::fs::write(d.join("planted.dll"), b"x").is_ok();
        if wrote {
            let _ = std::fs::remove_file(d.join("planted.dll"));
        }
        // (elevated, an administrator may write there - that is the point)
        assert!(!wrote || is_elevated(), "a normal program wrote into the admin-only folder");
        // empty: the parent (the user's own temp folder) lets its owner remove it
        let _ = std::fs::remove_dir(&d);
    }

    fn is_elevated() -> bool {
        std::process::Command::new("net").arg("session").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
    }

    #[test]
    fn a_bad_command_line_is_refused() {
        assert_eq!(run_if_requested(&["app".into()]), None);
        assert_eq!(run_if_requested(&["app".into(), ARG.into(), "format-c".into(), "x".into()]), Some(2));
        assert_eq!(run_if_requested(&["app".into(), ARG.into()]), Some(2));
    }
}
