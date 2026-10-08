//! The Windows side of the install step: process waits, the "I am up" event, starting processes.
//! Everything blocks on Windows kernel waits - no timers, no polling.

use crate::swap::{Procs, StartOutcome};
use std::collections::HashMap;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CreateEventW, OpenEventW, OpenProcess, SetEvent, WaitForMultipleObjects, WaitForSingleObject, EVENT_MODIFY_STATE,
    PROCESS_SYNCHRONIZE,
};

const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn millis(d: Duration) -> u32 {
    d.as_millis().min(u32::MAX as u128 - 1) as u32
}

/// Starts a program detached from us (it keeps running when we exit), with no console and no inherited handles.
pub(crate) fn spawn_detached(exe: &Path, args: &[String]) -> io::Result<Child> {
    let mut cmd = Command::new(exe);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    cmd.spawn()
}

#[derive(Default)]
pub struct RealProcs {
    event: Option<HANDLE>,
    children: HashMap<u32, Child>,
}

impl RealProcs {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Drop for RealProcs {
    fn drop(&mut self) {
        if let Some(h) = self.event.take() {
            // SAFETY: the handle was created by CreateEventW in `create_start_event` and is closed once.
            unsafe {
                let _ = CloseHandle(h);
            }
        }
    }
}

impl Procs for RealProcs {
    fn wait_for_exit(&mut self, pid: u32, timeout: Duration) -> bool {
        // SAFETY: plain Win32 calls; the handle is closed before returning.
        unsafe {
            match OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
                // cannot be opened = it is already gone (or never existed)
                Err(_) => true,
                Ok(h) => {
                    let r = WaitForSingleObject(h, millis(timeout));
                    let _ = CloseHandle(h);
                    r == WAIT_OBJECT_0
                }
            }
        }
    }

    fn create_start_event(&mut self, name: &str) -> io::Result<()> {
        let n = wide(name);
        // SAFETY: `n` is NUL-terminated and outlives the call. Manual-reset, starts not signalled.
        let h = unsafe { CreateEventW(None, true, false, PCWSTR(n.as_ptr())) }.map_err(|e| io::Error::other(e.to_string()))?;
        self.event = Some(h);
        Ok(())
    }

    fn launch(&mut self, exe: &Path, args: &[String]) -> io::Result<u32> {
        let child = spawn_detached(exe, args)?;
        let id = child.id();
        self.children.insert(id, child);
        Ok(id)
    }

    fn wait_started(&mut self, child: u32, timeout: Duration) -> StartOutcome {
        let (Some(ev), Some(c)) = (self.event, self.children.get(&child)) else { return StartOutcome::TimedOut };
        let handles = [ev, HANDLE(c.as_raw_handle())];
        // SAFETY: both handles are alive for the whole call (owned by `self`).
        let r = unsafe { WaitForMultipleObjects(&handles, false, millis(timeout)) };
        if r == WAIT_OBJECT_0 {
            StartOutcome::Started
        } else if r.0 == WAIT_OBJECT_0.0 + 1 {
            // the process ended; it may have signalled just before - the event wins
            // SAFETY: as above.
            if unsafe { WaitForSingleObject(ev, 0) } == WAIT_OBJECT_0 {
                StartOutcome::Started
            } else {
                StartOutcome::Exited
            }
        } else {
            StartOutcome::TimedOut
        }
    }

    fn kill(&mut self, child: u32) {
        if let Some(mut c) = self.children.remove(&child) {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// The new app tells the install step it is up: opens the named event and sets it.
pub(crate) fn signal_started(event_name: &str) -> bool {
    let n = wide(event_name);
    // SAFETY: `n` is NUL-terminated and outlives the call; the handle is closed before returning.
    unsafe {
        match OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(n.as_ptr())) {
            Ok(h) => {
                let ok = SetEvent(h).is_ok();
                let _ = CloseHandle(h);
                ok
            }
            Err(_) => false,
        }
    }
}
