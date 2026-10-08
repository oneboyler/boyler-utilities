//! The keep countdown as a real timer: one thread that blocks in ONE wait until the deadline (no polling, ~0 CPU),
//! then reverts — even if the flyout was closed meanwhile. Keep / Revert / a new Apply cancels it.

use crate::os::DisplayOs;
use crate::service::{DisplayService, Reverted};
use crate::error::Result;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub struct KeepTimer {
    cancel: Option<Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl KeepTimer {
    /// Calls `on_expire` once after `after`, unless cancelled first.
    pub fn start(after: Duration, on_expire: impl FnOnce() + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel::<()>();
        let handle = std::thread::Builder::new()
            .name("bu-display-keep".into())
            .spawn(move || {
                if let Err(RecvTimeoutError::Timeout) = rx.recv_timeout(after) {
                    on_expire();
                }
            })
            .ok();
        Self { cancel: Some(tx), handle }
    }

    /// Stops the countdown (Keep / Revert pressed, or a new Apply restarts it).
    pub fn cancel(mut self) {
        self.stop();
    }

    /// Never joins: the caller may hold the service lock that a just-fired timer is waiting for. A timer that fired
    /// anyway finds no pending change (Keep / Revert already cleared it) and does nothing.
    fn stop(&mut self) {
        if let Some(tx) = self.cancel.take() {
            let _ = tx.send(());
        }
        self.handle.take();
    }

    /// True once the timer thread has finished (fired or cancelled).
    pub fn is_finished(&self) -> bool {
        self.handle.as_ref().map(|h| h.is_finished()).unwrap_or(true)
    }
}

impl Drop for KeepTimer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Arms the countdown for a shared service: after the pending deadline it calls `tick`, and hands the result
/// (the revert, for the toast "Not kept, back to …") to `on_reverted`. Call it again after every Apply (drop/cancel
/// the old timer first).
pub fn arm<O>(svc: Arc<Mutex<DisplayService<O>>>, on_reverted: impl FnOnce(Result<Reverted>) + Send + 'static) -> Option<KeepTimer>
where
    O: DisplayOs + Send + 'static,
{
    let deadline = svc.lock().ok()?.pending()?.deadline;
    let after = deadline.saturating_duration_since(Instant::now());
    Some(KeepTimer::start(after, move || {
        let res = match svc.lock() {
            Ok(mut s) => s.tick(Instant::now().max(deadline)),
            Err(_) => None,
        };
        if let Some(r) = res {
            on_reverted(r);
        }
    }))
}
