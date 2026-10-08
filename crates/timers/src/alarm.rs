//! The wake-up for countdowns and timer bars: one thread that SLEEPS until the next deadline and then calls the app
//! once. Nothing polls: with no deadline it blocks on its channel; with one it sleeps exactly until it
//! (`recv_timeout`), and a new deadline wakes it to re-aim. The app then calls `Countdown::check` / `TimerBars::screen`.

use crate::{Result, TimerError};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Instant;

enum Msg {
    Aim(Option<Instant>),
    Quit,
}

pub struct Alarm {
    tx: Sender<Msg>,
    join: Option<JoinHandle<()>>,
}

impl Alarm {
    /// Starts the sleeping thread. `on_ring` runs on that thread when a deadline is reached (post to the app's loop).
    /// Fails only if Windows can't start a thread.
    pub fn start(on_ring: impl Fn() + Send + 'static) -> Result<Alarm> {
        let (tx, rx) = channel::<Msg>();
        let join = std::thread::Builder::new()
            .name("bu-timers-alarm".into())
            .spawn(move || {
                let mut at: Option<Instant> = None;
                loop {
                    let msg = match at {
                        None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                        Some(t) => rx.recv_timeout(t.saturating_duration_since(Instant::now())),
                    };
                    match msg {
                        Ok(Msg::Aim(t)) => at = t,
                        Ok(Msg::Quit) | Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => {
                            // recv_timeout may return a hair early on some clocks: never ring before the deadline
                            if let Some(t) = at {
                                if Instant::now() < t {
                                    continue;
                                }
                            }
                            at = None;
                            on_ring();
                        }
                    }
                }
            })
            .map_err(|e| TimerError::Os { context: format!("start alarm thread: {e}"), code: e.raw_os_error().unwrap_or(0) as u32 })?;
        Ok(Alarm { tx, join: Some(join) })
    }

    /// Aims the alarm at a moment (`None` = disarm). It rings once.
    pub fn aim(&self, at: Option<Instant>) {
        let _ = self.tx.send(Msg::Aim(at));
    }
}

impl Drop for Alarm {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Quit);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}
