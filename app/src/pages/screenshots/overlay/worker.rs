//! The capture's slow Windows work on a worker thread (Order 048): grabbing the screen (Desktop Duplication / Graphics
//! Capture wait for a frame, up to 1-2 s), the PNG encode, the file and the clipboard. On the UI thread they froze the
//! overlay - topmost windows on every monitor - and with it the whole desktop. Each job runs on its own short-lived thread
//! with its own engine; its result is handed back with one message to the overlay's message window (window.rs).

use std::path::PathBuf;
use std::sync::Mutex;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use bu_screenshot::real::RealOs;
use bu_screenshot::{Frozen, Screenshots};

use super::capture::{run_finish, Done, FinishJob};

/// A job's result is waiting ([`take`]).
pub const WM_JOB_DONE: u32 = WM_APP + 0x32;

/// Which capture (and which of its jobs) a result belongs to: a result of a closed capture, or of an older Snap, is dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tag {
    pub session: u64,
    pub seq: u64,
}

pub enum Job {
    /// every monitor, this moment (the overlay's start, a Snap, Live off)
    Grab,
    Finish(FinishJob),
}

pub enum Res {
    Grab(bu_screenshot::Result<Frozen>),
    Finish(bu_screenshot::Result<Done>),
}

static DONE: Mutex<Vec<(Tag, Res)>> = Mutex::new(Vec::new());

/// Run `job` off the UI thread; `notify` (a window of the UI thread) hears WM_JOB_DONE when its result is in.
pub fn spawn(tag: Tag, data_dir: PathBuf, job: Job, notify: HWND) {
    let notify = notify.0 as isize;
    let finish = matches!(job, Job::Finish(_));
    let run = move || {
        // COM for the capture APIs (Graphics Capture is WinRT); the clipboard opens its own hidden owner window
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        let svc = Screenshots::new(RealOs::new(), data_dir);
        let res = match job {
            Job::Grab => Res::Grab(svc.capture_all()),
            Job::Finish(f) => Res::Finish(run_finish(&svc, f)),
        };
        drop(svc);
        if com {
            unsafe { CoUninitialize() };
        }
        DONE.lock().unwrap_or_else(|e| e.into_inner()).push((tag, res));
        unsafe {
            let _ = PostMessageW(Some(HWND(notify as *mut _)), WM_JOB_DONE, WPARAM(0), LPARAM(0));
        }
    };
    if let Err(e) = std::thread::Builder::new().name("bu-capture-job".into()).spawn(run) {
        // no thread: say so like a failed capture (never silent)
        let err = bu_screenshot::Error::BadData(format!("could not start the capture thread: {e}"));
        let res = if finish { Res::Finish(Err(err)) } else { Res::Grab(Err(err)) };
        DONE.lock().unwrap_or_else(|e| e.into_inner()).push((tag, res));
        unsafe {
            let _ = PostMessageW(Some(HWND(notify as *mut _)), WM_JOB_DONE, WPARAM(0), LPARAM(0));
        }
    }
}

/// The results that came in (the UI thread, on WM_JOB_DONE).
pub fn take() -> Vec<(Tag, Res)> {
    std::mem::take(&mut *DONE.lock().unwrap_or_else(|e| e.into_inner()))
}
