//! Order 124: "Lock brightness" (E46). A locked monitor gets its brightness put back after Windows says it may have
//! lost it: the monitor powered back on after sleep / display-off, the PC woke, the session was unlocked, or the
//! monitor came back (display change / device arrival). The events come from Windows (`win::power`); this is the part
//! that acts on them, with any OS layer (the fake in tests).
//!
//! ONE thread that sleeps in a channel wait until an event arrives - no polling loop. After an event it tries a few
//! times over ~10 s ([`RETRY_AT`]), because a monitor that just woke answers DDC/CI late; each try reads first and
//! writes only when the monitor shows another value. Events in a burst (wake + unlock + display change land together)
//! start the tries again from the newest one. Nothing is ever written outside those tries: the lock never fights the
//! user live.

use crate::os::DisplayOs;
use crate::service::DisplayService;
use crate::types::{MonitorId, VcpValue};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// When the tries run, counted from the (newest) event: ~10 s in all (the order's ~10 s; the steps are guessed - how
/// late a given monitor answers after waking is unclear).
pub const RETRY_AT: [Duration; 6] = [
    Duration::from_millis(500),
    Duration::from_millis(1500),
    Duration::from_millis(3000),
    Duration::from_millis(5000),
    Duration::from_millis(7500),
    Duration::from_millis(10000),
];

/// What Windows said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WakeEvent {
    /// The display(s) powered back on (`GUID_CONSOLE_DISPLAY_STATE` off → on).
    DisplayOn,
    /// The PC woke from sleep (`PBT_APMRESUMEAUTOMATIC` / `PBT_APMRESUMESUSPEND`).
    Resume,
    /// The session was unlocked (`WTS_SESSION_UNLOCK`).
    Unlock,
    /// The desktop's monitors changed (`WM_DISPLAYCHANGE`).
    DisplayChange,
    /// A monitor was plugged in / switched on (`DBT_DEVICEARRIVAL` for the monitor device interface).
    MonitorArrived,
}


/// What one try did on one locked monitor.
#[derive(Clone, Debug, PartialEq)]
pub enum Restore {
    /// It already showed the locked value: nothing written.
    Same,
    /// It showed another value: the locked one was written.
    Written { before: VcpValue, after: VcpValue },
    /// It didn't answer DDC/CI (still waking, unplugged, DDC/CI off): tried again at the next try.
    NoAnswer,
    /// The user moved a brightness slider while this try ran: nothing written (the next try uses the new locked value).
    Moved,
}

/// The running lock thread. Dropping it ends the thread (after a try in progress).
pub struct LockWatch {
    tx: Sender<WakeEvent>,
}

impl LockWatch {
    /// Starts the thread. `locks` = the locked monitors and their brightness % (read at every try, so a slider moved
    /// while locked is the value used); `moved` = bumped by every brightness change of the user's, AFTER the new locked
    /// value is stored and BEFORE the monitor is written (end review: a try that read the old locked value must not put
    /// it back over the user's new one); `retry_at` = when the tries run after an event ([`RETRY_AT`]); `on_try` gets
    /// every try's results.
    pub fn start<O, L, F>(svc: Arc<Mutex<DisplayService<O>>>, locks: L, moved: Arc<AtomicU64>, retry_at: Vec<Duration>, on_try: F) -> LockWatch
    where
        O: DisplayOs + Send + 'static,
        L: Fn() -> Vec<(MonitorId, u8)> + Send + 'static,
        F: Fn(&[(MonitorId, Restore)]) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel::<WakeEvent>();
        let _ = std::thread::Builder::new().name("bu-display-brightlock".into()).spawn(move || {
            // sleeps here until Windows says something (or the watch is dropped)
            while rx.recv().is_ok() {
                let mut t0 = Instant::now();
                let mut i = 0;
                while i < retry_at.len() {
                    let wait = (t0 + retry_at[i]).saturating_duration_since(Instant::now());
                    match rx.recv_timeout(wait) {
                        Ok(_) => {
                            t0 = Instant::now();
                            i = 0;
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            on_try(&try_once(&svc, &locks, &moved));
                            i += 1;
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
        });
        LockWatch { tx }
    }

    /// Windows said something: the tries (re)start.
    pub fn wake(&self, ev: WakeEvent) {
        let _ = self.tx.send(ev);
    }
}

/// One try on every locked monitor: read, write the locked value only when it differs - unless the user moved a
/// brightness since the locked values were read (`moved` is read first, checked again while the service is held).
pub fn try_once<O: DisplayOs>(svc: &Mutex<DisplayService<O>>, locks: impl Fn() -> Vec<(MonitorId, u8)>, moved: &AtomicU64) -> Vec<(MonitorId, Restore)> {
    let gen = moved.load(Ordering::SeqCst);
    let mut out = Vec::new();
    for (id, pct) in locks() {
        let r = match svc.lock() {
            Ok(_) if moved.load(Ordering::SeqCst) != gen => Restore::Moved,
            Ok(mut s) => match s.restore_brightness(&id, pct) {
                Ok(None) => Restore::Same,
                Ok(Some((before, after))) => Restore::Written { before, after },
                Err(_) => Restore::NoAnswer,
            },
            Err(_) => Restore::NoAnswer,
        };
        out.push((id, r));
    }
    out
}
