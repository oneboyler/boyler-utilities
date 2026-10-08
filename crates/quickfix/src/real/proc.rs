//! Starts a console program from System32 with no window, stdout captured, inside a job object so Cancel ends the
//! program AND what it started (DISM runs its work in DismHost.exe).

use super::win_err;
use crate::os::{ProcCtl, Spawned};
use crate::{FixError, Result};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, TerminateJobObject};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

struct Job(HANDLE);
// SAFETY: a job handle is a kernel handle, usable from any thread.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}
impl Drop for Job {
    fn drop(&mut self) {
        // no KILL_ON_JOB_CLOSE: closing our handle never ends a repair that is still running
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct RealProc {
    child: Mutex<Child>,
    job: Option<Job>,
}

impl ProcCtl for RealProc {
    fn kill(&self) {
        if let Some(j) = &self.job {
            if unsafe { TerminateJobObject(j.0, 1) }.is_ok() {
                return;
            }
        }
        if let Ok(mut c) = self.child.try_lock() {
            let _ = c.kill();
        }
    }

    fn wait(&self) -> Result<u32> {
        let mut c = self.child.lock().unwrap_or_else(|e| e.into_inner());
        let st = c.wait().map_err(|e| FixError::Os { context: "wait".into(), code: e.raw_os_error().unwrap_or(0) as u32 })?;
        Ok(st.code().map(|c| c as u32).unwrap_or(1))
    }
}

pub fn spawn(path: &Path, args: &[&str]) -> Result<Spawned> {
    let mut child = Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| match e.raw_os_error() {
            Some(740) => FixError::NeedsAdmin(path.display().to_string()), // ERROR_ELEVATION_REQUIRED
            c => FixError::Os { context: format!("start {}", path.display()), code: c.unwrap_or(0) as u32 },
        })?;
    let output = child.stdout.take().ok_or_else(|| FixError::Os { context: "stdout pipe".into(), code: 0 })?;
    let job = unsafe {
        CreateJobObjectW(None, PCWSTR::null()).map_err(win_err("CreateJobObject")).ok().and_then(|h| {
            let j = Job(h);
            AssignProcessToJobObject(j.0, HANDLE(child.as_raw_handle())).is_ok().then_some(j)
        })
    };
    Ok(Spawned { output: Box::new(output), ctl: Arc::new(RealProc { child: Mutex::new(child), job }) })
}
