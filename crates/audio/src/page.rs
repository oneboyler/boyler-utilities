//! The Audio page while it is open — Lane A's worker-thread design (bakeoff-native `audio.rs` `Threaded`): Windows'
//! audio service sometimes takes 100–220 ms to answer even a volume question, and 126–1076 ms on a first open (both
//! measured by Lane A, Oct 7/8), so the menu never calls it directly. One worker thread owns the OS layer, keeps the
//! latest answers in a [`PageSnapshot`] the UI reads without waiting, and takes changes as commands.
//!
//! It lives only while the menu is open on Audio: [`AudioPage::start`] when the page opens, drop it on close — the thread
//! ends, nothing runs. While it lives it re-reads (the meters have no change events, so this part is a timed loop —
//! only while the page is open):
//! * levels every [`Timing::levels`] (16 ms = one 60 Hz frame) — only while levels are asked for ([`AudioPage::set_levels`]);
//! * defaults, volumes, mutes, the app list every [`Timing::fast`] (250 ms);
//! * the device lists every [`Timing::slow`] (1 s).
//!
//! A command wakes the worker at once; its result (and the change for undo) is kept for the UI.

use crate::model::*;
use crate::os::AudioOs;
use crate::service::{AudioService, Change};
use crate::{AudioError, Result};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub levels: Duration,
    pub fast: Duration,
    pub slow: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing { levels: Duration::from_millis(16), fast: Duration::from_millis(250), slow: Duration::from_secs(1) }
    }
}

/// The latest answers.
#[derive(Debug, Clone, Default)]
pub struct PageSnapshot {
    /// The first full read is in.
    pub ready: bool,
    pub outputs: Vec<DeviceRow>,
    pub inputs: Vec<DeviceRow>,
    /// The Output / Input rows' device and its volume.
    pub output: Option<(String, VolumeMute)>,
    pub input: Option<(String, VolumeMute)>,
    /// Windows' defaults (all three roles) of Output / Input (the reset's "now").
    pub output_defaults: Defaults,
    pub input_defaults: Defaults,
    pub apps: Vec<AppRow>,
    /// Device levels 0..1 (raw; the UI smooths: attack .28 / release .06 and the peak tick).
    pub output_level: f32,
    pub input_level: f32,
    /// Per app (group → loudest session's raw peak).
    pub app_levels: Vec<(String, f32)>,
    /// Number of level reads done (tests prove levels stop when not asked).
    pub level_reads: u64,
    /// The last command's error.
    pub last_error: Option<AudioError>,
    /// Changes done through this page, oldest first ([`AudioPage::undo_last`]).
    pub undo: Vec<Change>,
}

type Job<O> = Box<dyn FnOnce(&mut AudioService<O>) -> Result<Option<Change>> + Send>;

enum Cmd<O: AudioOs> {
    Run(Job<O>),
    Undo,
    Levels(bool),
    Quit,
}

pub struct AudioPage<O: AudioOs + 'static> {
    snap: Arc<Mutex<PageSnapshot>>,
    tx: Sender<Cmd<O>>,
    join: Option<JoinHandle<()>>,
}

fn lock(m: &Mutex<PageSnapshot>) -> std::sync::MutexGuard<'_, PageSnapshot> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl<O: AudioOs + 'static> AudioPage<O> {
    /// Starts the worker; `make` builds the OS layer ON the worker thread (COM objects stay on the thread that made them).
    /// Returns at once — the UI starts from an empty snapshot (`ready` false) and the worker fills it in milliseconds.
    pub fn start(make: impl FnOnce() -> Result<O> + Send + 'static, timing: Timing) -> AudioPage<O> {
        let snap = Arc::new(Mutex::new(PageSnapshot::default()));
        let (tx, rx) = channel::<Cmd<O>>();
        let s2 = snap.clone();
        let join = std::thread::Builder::new()
            .name("bu-audio-page".into())
            .spawn(move || {
                let os = match make() {
                    Ok(os) => os,
                    Err(e) => {
                        lock(&s2).last_error = Some(e);
                        return;
                    }
                };
                let mut svc = AudioService::new(os);
                let mut levels = true;
                let (mut last_fast, mut last_slow) = (None::<Instant>, None::<Instant>);
                loop {
                    let now = Instant::now();
                    let slow = last_slow.is_none_or(|t| now - t >= timing.slow);
                    let fast = slow || last_fast.is_none_or(|t| now - t >= timing.fast);
                    if slow {
                        last_slow = Some(now);
                        let (o, i) = (svc.device_rows(Flow::Output), svc.device_rows(Flow::Input));
                        let mut s = lock(&s2);
                        s.outputs = o.unwrap_or_default();
                        s.inputs = i.unwrap_or_default();
                    }
                    if fast {
                        last_fast = Some(now);
                        refresh_fast(&mut svc, &s2);
                    }
                    if levels {
                        read_levels(&mut svc, &s2);
                    }
                    lock(&s2).ready = true;
                    let wait = if levels { timing.levels } else { timing.fast };
                    let first = match rx.recv_timeout(wait) {
                        Ok(c) => Some(c),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => break,
                    };
                    let mut changed = false;
                    let mut next = first;
                    while let Some(cmd) = next.take() {
                        match cmd {
                            Cmd::Quit => return,
                            Cmd::Levels(on) => levels = on,
                            Cmd::Run(job) => {
                                let r = job(&mut svc);
                                let mut s = lock(&s2);
                                match r {
                                    Ok(Some(c)) => {
                                        s.undo.push(c);
                                        s.last_error = None;
                                    }
                                    Ok(None) => s.last_error = None,
                                    Err(e) => s.last_error = Some(e),
                                }
                                changed = true;
                            }
                            Cmd::Undo => {
                                let c = lock(&s2).undo.pop();
                                if let Some(c) = c {
                                    let r = svc.undo(&c);
                                    lock(&s2).last_error = r.err();
                                    changed = true;
                                }
                            }
                        }
                        next = rx.try_recv().ok();
                    }
                    if changed {
                        // show the result at once (and re-read the device lists: on/off changes them)
                        last_slow = None;
                    }
                }
            })
            .map_err(|e| lock(&snap).last_error = Some(AudioError::Unavailable(format!("audio page thread did not start: {e}"))))
            .ok();
        AudioPage { snap, tx, join }
    }

    /// The latest answers (never waits for Windows).
    pub fn snapshot(&self) -> PageSnapshot {
        lock(&self.snap).clone()
    }

    /// Any change, run on the worker: e.g. `page.run(|s| s.set_device_volume(&id, 0.5).map(Some))`.
    pub fn run(&self, job: impl FnOnce(&mut AudioService<O>) -> Result<Option<Change>> + Send + 'static) {
        let _ = self.tx.send(Cmd::Run(Box::new(job)));
    }

    /// Undo the newest change made through this page.
    pub fn undo_last(&self) {
        let _ = self.tx.send(Cmd::Undo);
    }

    /// Levels only while asked (the meters animate only while the page shows).
    pub fn set_levels(&self, on: bool) {
        let _ = self.tx.send(Cmd::Levels(on));
    }
}

impl<O: AudioOs + 'static> Drop for AudioPage<O> {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Quit);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn refresh_fast<O: AudioOs>(svc: &mut AudioService<O>, snap: &Mutex<PageSnapshot>) {
    let (od, idf) = (svc.defaults(Flow::Output).unwrap_or_default(), svc.defaults(Flow::Input).unwrap_or_default());
    let (out, inp) = (od.console.clone(), idf.console.clone());
    let ov = out.as_ref().and_then(|id| svc.device_volume(id).ok().map(|v| (id.clone(), v)));
    let iv = inp.as_ref().and_then(|id| svc.device_volume(id).ok().map(|v| (id.clone(), v)));
    let apps = out.as_ref().and_then(|id| svc.apps(id).ok()).unwrap_or_default();
    let mut s = lock(snap);
    s.output = ov;
    s.input = iv;
    s.output_defaults = od;
    s.input_defaults = idf;
    s.apps = apps;
}

fn read_levels<O: AudioOs>(svc: &mut AudioService<O>, snap: &Mutex<PageSnapshot>) {
    let (out, inp, apps) = {
        let s = lock(snap);
        (s.output.as_ref().map(|o| o.0.clone()), s.input.as_ref().map(|i| i.0.clone()), s.apps.clone())
    };
    let ol = out.and_then(|id| svc.os_mut().peak(&id).ok()).unwrap_or(0.0);
    let il = inp.and_then(|id| svc.os_mut().peak(&id).ok()).unwrap_or(0.0);
    let al: Vec<(String, f32)> = apps
        .iter()
        .map(|a| (a.group.clone(), a.sessions.iter().filter_map(|k| svc.os_mut().session_peak(k).ok()).fold(0.0, f32::max)))
        .collect();
    let mut s = lock(snap);
    s.output_level = ol;
    s.input_level = il;
    s.app_levels = al;
    s.level_reads += 1;
}
