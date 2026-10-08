//! The elevated copy's entry point: `BoylerUtilities.exe --bu-admin <purpose> <pipe id>` (main.rs hands it over before
//! anything else starts; nothing opens, no tray, no window). It connects to the app's two pipes (wire.rs checks they
//! are the app's own), then answers ops until the app closes the line, waits for a running program's output to end,
//! and exits. Any command line that is not exactly that form exits with code 2 and does nothing.
//!
//! Requests: `op <seq> <op fields…>`. Answers: `ok <seq> <fields…>` / `err <seq> refused|denied|notfound|failed <text>`;
//! program output: `out <stream> <bytes>` … `end <stream> <exit code>`.

use std::sync::Arc;

use super::exec::{self, Streams, Sys};
use super::wire::{self, Rx, Tx};
use super::{AdminError, Op, Purpose, ARG};

pub const CODE_BAD_ARGS: i32 = 2;
pub const CODE_NO_PIPE: i32 = 3;

/// main.rs, first thing: `if let Some(code) = admin::helper::run_if_requested(&args) { exit(code) }`.
pub fn run_if_requested(args: &[String]) -> Option<i32> {
    if args.get(1).map(String::as_str) != Some(ARG) {
        return None;
    }
    let Some((purpose, id)) = parse_args(args) else { return Some(CODE_BAD_ARGS) };
    // COM for the real layers (WMI, Task Scheduler, the audio policy, the restore point), on this one thread
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
    }
    // before anything finds a program or a folder: an environment made from Windows itself, not the user's
    let Some(temp) = clean_env() else { return Some(CODE_NO_ENV) };
    let Ok((mut rx, tx, sid)) = wire::connect(&id) else {
        let _ = std::fs::remove_dir_all(&temp);
        return Some(CODE_NO_PIPE);
    };
    let mut sys = exec::RealSys::new(&sid);
    serve(purpose, &mut rx, Arc::new(tx), &mut sys);
    let _ = std::fs::remove_dir_all(&temp);
    Some(0)
}

pub const CODE_NO_ENV: i32 = 4;

/// The variables the copy keeps from the environment it was started with (harmless facts about the PC); everything else
/// is dropped.
const KEEP_ENV: [&str; 10] =
    ["USERNAME", "USERDOMAIN", "COMPUTERNAME", "NUMBER_OF_PROCESSORS", "PROCESSOR_ARCHITECTURE", "PROCESSOR_IDENTIFIER", "PROCESSOR_LEVEL", "PROCESSOR_REVISION", "OS", "PATHEXT"];

/// The elevated copy's environment, rebuilt before anything runs (Order 039 review): the environment an elevated process
/// starts with is the user's (`HKCU\Environment` - any program of the user can write it), and the crates find Windows'
/// programs through it (`%SystemRoot%\System32\…\powershell.exe`, DISM / sfc, `%ProgramData%\…\MpCmdRun.exe`) and their
/// children inherit it (PATH for DLLs, TEMP where DISM unpacks its helpers, .NET profiler variables, PSModulePath). So:
/// every variable is dropped except [`KEEP_ENV`]; Windows' folders come from Windows (`GetSystemWindowsDirectoryW`, the
/// known-folder API); PATH = Windows' own folders only; TEMP / TMP = a fresh folder only SYSTEM and Administrators can
/// write (removed when the copy ends). Returns that folder; None = Windows' folders can't be read (the copy does nothing).
fn clean_env() -> Option<std::path::PathBuf> {
    use windows::core::GUID;
    use windows::Win32::UI::Shell::{
        FOLDERID_LocalAppData, FOLDERID_Profile, FOLDERID_ProgramData, FOLDERID_ProgramFiles, FOLDERID_ProgramFilesCommon, FOLDERID_ProgramFilesCommonX86, FOLDERID_ProgramFilesX86,
        FOLDERID_RoamingAppData, SHGetKnownFolderPath, KF_FLAG_DEFAULT,
    };
    let win = bu_addons::helper::windows_dir()?;
    let w = win.to_str()?.to_string();
    let temp_root = win.join("Temp");
    if bu_addons::helper::is_reparse(&win) || bu_addons::helper::is_reparse(&temp_root) {
        return None;
    }
    let known = |id: &GUID| -> Option<String> {
        // SAFETY: the returned string is freed with CoTaskMemFree after it is copied.
        unsafe {
            let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
            let s = p.to_string().ok();
            windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
            s
        }
    };
    let temp = bu_addons::helper::private_dir(&temp_root, bu_addons::helper::ADMIN_ONLY).ok()?;
    let kept: Vec<(std::ffi::OsString, std::ffi::OsString)> =
        std::env::vars_os().filter(|(k, _)| KEEP_ENV.iter().any(|x| k.to_string_lossy().eq_ignore_ascii_case(x))).collect();
    let all: Vec<std::ffi::OsString> = std::env::vars_os().map(|(k, _)| k).collect();
    // (one thread runs here: nothing else reads the environment yet)
    for k in all {
        std::env::remove_var(k);
    }
    for (k, v) in kept {
        std::env::set_var(k, v);
    }
    let sys32 = format!(r"{w}\System32");
    let mut set: Vec<(&str, String)> = vec![
        ("SystemRoot", w.clone()),
        ("windir", w.clone()),
        ("SystemDrive", w.chars().take(2).collect()),
        ("ComSpec", format!(r"{sys32}\cmd.exe")),
        ("PATH", format!(r"{sys32};{w};{sys32}\Wbem;{sys32}\WindowsPowerShell\v1.0")),
        ("PSModulePath", format!(r"{sys32}\WindowsPowerShell\v1.0\Modules")),
        ("TEMP", temp.to_str()?.to_string()),
        ("TMP", temp.to_str()?.to_string()),
    ];
    for (name, id) in [
        ("ProgramData", &FOLDERID_ProgramData),
        ("ALLUSERSPROFILE", &FOLDERID_ProgramData),
        ("ProgramFiles", &FOLDERID_ProgramFiles),
        ("ProgramW6432", &FOLDERID_ProgramFiles),
        ("ProgramFiles(x86)", &FOLDERID_ProgramFilesX86),
        ("CommonProgramFiles", &FOLDERID_ProgramFilesCommon),
        ("CommonProgramW6432", &FOLDERID_ProgramFilesCommon),
        ("CommonProgramFiles(x86)", &FOLDERID_ProgramFilesCommonX86),
        ("USERPROFILE", &FOLDERID_Profile),
        ("APPDATA", &FOLDERID_RoamingAppData),
        ("LOCALAPPDATA", &FOLDERID_LocalAppData),
    ] {
        if let Some(p) = known(id) {
            set.push((name, p));
        }
    }
    for (k, v) in set {
        std::env::set_var(k, v);
    }
    Some(temp)
}

/// Exactly `<exe> --bu-admin <purpose> <32 hex>`.
pub fn parse_args(args: &[String]) -> Option<(Purpose, String)> {
    if args.len() != 4 || args[1] != ARG {
        return None;
    }
    let p = Purpose::parse(&args[2])?;
    wire::check_id(&args[3]).then(|| (p, args[3].clone()))
}

fn err_fields(e: &AdminError) -> (&'static str, String) {
    match e {
        AdminError::Refused(s) => ("refused", s.clone()),
        AdminError::Denied(s) => ("denied", s.clone()),
        AdminError::NotFound(s) => ("notfound", s.clone()),
        AdminError::Failed(s) => ("failed", s.clone()),
        AdminError::Declined => ("failed", "declined".into()),
    }
}

/// Answer ops of `purpose` until the line closes (or a frame is malformed - then stop: the line can't be trusted).
pub fn serve(purpose: Purpose, rx: &mut dyn Rx, tx: Arc<dyn Tx>, sys: &mut dyn Sys) {
    let streams = Streams::default();
    let t2 = tx.clone();
    let out: Arc<dyn Fn(&[&[u8]]) + Send + Sync> = Arc::new(move |f: &[&[u8]]| {
        let _ = t2.send(f);
    });
    while let Ok(Some(frame)) = rx.recv() {
        let Some(f) = wire::texts(&frame) else { break };
        if f.len() < 2 || f[0] != "op" || f[1].is_empty() || f[1].len() > 20 || !f[1].bytes().all(|b| b.is_ascii_digit()) {
            break;
        }
        let seq = f[1].clone();
        let reply = match Op::parse(&f[2..]) {
            Err(e) => Err(AdminError::Refused(e)),
            Ok(op) if !purpose.allows(&op) => Err(AdminError::Refused(format!("{} is not part of {}", f[2], purpose.name()))),
            Ok(op) => exec::run(&op, sys, &streams, out.clone()),
        };
        let sent = match reply {
            Ok(fields) => {
                let mut v: Vec<&[u8]> = vec![b"ok", seq.as_bytes()];
                v.extend(fields.iter().map(|s| s.as_bytes()));
                tx.send(&v)
            }
            Err(e) => {
                let (k, t) = err_fields(&e);
                tx.send(&[b"err", seq.as_bytes(), k.as_bytes(), t.as_bytes()])
            }
        };
        if sent.is_err() {
            break;
        }
    }
    // the line closed: a running program goes on to its end (a killed DISM may leave Windows' component store half
    // way); its output has nowhere to go
    streams.wait_all();
}
