//! Stops File Explorer with the Restart Manager (every explorer.exe of this session), and brings it back: `RmRestart`
//! (Explorer re-opens its folder windows), then a one-shot wait of up to 10 s for the taskbar — if it isn't back and
//! NO explorer.exe runs in this session, explorer.exe is started directly. Can block up to 10 s: call it off the UI
//! thread. Same method as the Toggles crate's Explorer restart (Order 005).

use super::win_err;
use crate::os::ExplorerPause;
use crate::{FixError, Result};
use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, FILETIME, WIN32_ERROR};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::RestartManager::{
    RmEndSession, RmForceShutdown, RmRegisterResources, RmRestart, RmShutdown, RmStartSession, RM_UNIQUE_PROCESS,
};
use windows::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SW_SHOWNORMAL};

fn rm(context: &str, e: WIN32_ERROR) -> Result<()> {
    if e == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(FixError::Os { context: context.into(), code: e.0 })
    }
}

/// explorer.exe processes in our session, with their start times (what the Restart Manager needs).
unsafe fn explorers() -> Result<Vec<RM_UNIQUE_PROCESS>> {
    let mut mine = 0u32;
    ProcessIdToSessionId(std::process::id(), &mut mine).map_err(win_err("ProcessIdToSessionId"))?;
    let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).map_err(win_err("CreateToolhelp32Snapshot"))?;
    let mut out = Vec::new();
    let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut ok = Process32FirstW(snap, &mut e).is_ok();
    while ok {
        let name = String::from_utf16_lossy(&e.szExeFile[..e.szExeFile.iter().position(|&c| c == 0).unwrap_or(0)]);
        let mut session = u32::MAX;
        if name.eq_ignore_ascii_case("explorer.exe")
            && ProcessIdToSessionId(e.th32ProcessID, &mut session).is_ok()
            && session == mine
        {
            if let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, e.th32ProcessID) {
                let (mut created, mut exit, mut kernel, mut user) =
                    (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
                if GetProcessTimes(p, &mut created, &mut exit, &mut kernel, &mut user).is_ok() {
                    out.push(RM_UNIQUE_PROCESS { dwProcessId: e.th32ProcessID, ProcessStartTime: created });
                }
                let _ = CloseHandle(p);
            }
        }
        ok = Process32NextW(snap, &mut e).is_ok();
    }
    let _ = CloseHandle(snap);
    Ok(out)
}

pub fn stop() -> Result<Box<dyn ExplorerPause>> {
    unsafe {
        let procs = explorers()?;
        if procs.is_empty() {
            return Err(FixError::Unavailable("File Explorer is not running".into()));
        }
        let mut session = 0u32;
        let mut key = [0u16; 64];
        rm("RmStartSession", RmStartSession(&mut session, None, PWSTR(key.as_mut_ptr())))?;
        let r = rm("RmRegisterResources", RmRegisterResources(session, None, Some(&procs), None))
            .and_then(|_| rm("RmShutdown", RmShutdown(session, RmForceShutdown.0 as u32, None)));
        if let Err(e) = r {
            let _ = RmRestart(session, None, None);
            let _ = RmEndSession(session);
            ensure_shell();
            return Err(e);
        }
        Ok(Box::new(Pause { session: Some(session) }))
    }
}

struct Pause {
    session: Option<u32>,
}

impl Pause {
    fn bring_back(&mut self) -> Result<()> {
        let Some(s) = self.session.take() else { return Ok(()) };
        unsafe {
            let r = rm("RmRestart", RmRestart(s, None, None));
            let _ = RmEndSession(s);
            ensure_shell();
            r
        }
    }
}

impl ExplorerPause for Pause {
    fn restart(mut self: Box<Self>) -> Result<()> {
        self.bring_back()
    }
}

impl Drop for Pause {
    fn drop(&mut self) {
        let _ = self.bring_back();
    }
}

/// One-shot wait (not a background poll): up to 10 s for the taskbar. Only when NO explorer.exe runs in our session
/// after that is it started directly — a slow but running Explorer is never doubled (a second one opens a folder window).
fn ensure_shell() {
    unsafe {
        for _ in 0..100 {
            if FindWindowW(&HSTRING::from("Shell_TrayWnd"), PCWSTR::null()).is_ok() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if explorers().map(|p| !p.is_empty()).unwrap_or(true) {
            return;
        }
        ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from("explorer.exe"), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}
