//! Watching OBS's settings folders (app.c `watch_setup` + its MsgWaitForMultipleObjects): a thread blocks on the change
//! notifications (0 % CPU while nothing changes) and reports each change; OBS writes through a temp file + rename, so the
//! engine waits 400 ms before re-reading.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects, INFINITE};

pub struct Watcher {
    stop: HANDLE,
    done: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

// the event handle is only signalled / closed by the owner
unsafe impl Send for Watcher {}

struct Handles(Vec<HANDLE>);
unsafe impl Send for Handles {}

impl Watcher {
    /// Watch `dirs[0]` (not its subfolders) and every other dir with its subfolders; `on_change` runs on the watcher thread.
    pub fn start(dirs: Vec<PathBuf>, on_change: impl Fn() + Send + 'static) -> Option<Watcher> {
        let stop = unsafe { CreateEventW(None, true, false, None).ok()? };
        let mut hs = vec![stop];
        for (i, d) in dirs.iter().enumerate() {
            let h = unsafe { FindFirstChangeNotificationW(&HSTRING::from(d.as_os_str()), i > 0, FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME) };
            if let Ok(h) = h {
                hs.push(h);
            }
        }
        let done = Arc::new(AtomicBool::new(false));
        let d2 = done.clone();
        let handles = Handles(hs);
        let thread = std::thread::Builder::new()
            .name("obs-watch".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                let hs = handles;
                loop {
                    let r = unsafe { WaitForMultipleObjects(&hs.0, false, INFINITE) };
                    let i = r.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
                    if i == 0 || i >= hs.0.len() || d2.load(Ordering::Acquire) {
                        break;
                    }
                    unsafe {
                        let _ = FindNextChangeNotification(hs.0[i]);
                    }
                    on_change();
                }
                for h in &hs.0[1..] {
                    unsafe {
                        let _ = FindCloseChangeNotification(*h);
                    }
                }
            })
            .ok()?;
        Some(Watcher { stop, done, thread: Some(thread) })
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        unsafe {
            let _ = SetEvent(self.stop);
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        unsafe {
            let _ = CloseHandle(self.stop);
        }
    }
}
