//! [`MicMute`] — the Mic mute card's logic (DESIGN §3.5). Actions only: the app layer maps the user's key(s) to
//! [`MicMute::toggle`] (one key) or [`MicMute::mute`] / [`MicMute::unmute`] (separate keys) — no key is registered here.
//!
//! - The mic: Windows' default input device or a picked one ([`MicChoice`]); muted at the endpoint
//!   (`IAudioEndpointVolume::SetMute`) — a real mic kill for every app.
//! - State + a change event ([`MicMute::start_watching`]) that also reports another app muting / unmuting it.
//! - The mute / unmute sound ([`SoundSettings`]) — only for our own changes, never for silent ones.
//! - Undo: the last change remembers the old state ([`MicMute::undo`]).

use crate::os::{EventSink, MicDevice, MicEvent, MicOs, SoundOut, Watch};
use crate::sound::{self, Sound, SoundSettings};
use crate::{MicError, Result};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// Which mic the card works on. Default = DESIGN's shared Audio › Input choice starts on Windows' default.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MicChoice {
    /// Whatever Windows' default input device is — follows it when it changes.
    #[default]
    Default,
    /// One endpoint by id.
    Device(String),
}

/// What the Live / Muted pill and the on-screen icon show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicState {
    /// The mic in use; `None` = no mic (none plugged in / the picked one is gone).
    pub device: Option<MicDevice>,
    pub muted: bool,
}

/// Called on this crate's `bu-micmute-watch` worker thread (never on Windows' audio thread) when the state changes by
/// something other than our own action: another app muting / unmuting the mic, the default mic changing, the mic being
/// unplugged. Our own actions return the new state. Keep it short, and never call `stop_watching`, `turn_off`,
/// `start_watching` or `settle` from inside it (they wait for this same worker) — post to the app's thread instead.
pub type ChangeFn = Arc<dyn Fn(MicState) + Send + Sync>;

struct State {
    choice: MicChoice,
    sound: SoundSettings,
    /// (device id, the mute flag before our last change).
    undo: Option<(String, bool)>,
    /// Mics this crate muted and hasn't unmuted since — switching the card off unmutes every one still muted.
    muted_by_us: Vec<String>,
    /// The id the mute watch is on.
    watched: Option<String>,
    mute_watch: Option<Box<dyn Watch>>,
    dev_watch: Option<Box<dyn Watch>>,
    on_change: Option<ChangeFn>,
    worker: Option<(Sender<Job>, JoinHandle<()>)>,
}

enum Job {
    /// Re-resolve the mic (default changed / devices changed) and move the mute watch.
    Retarget,
    /// Another app changed the watched mic's mute flag: the change event (off Windows' audio thread).
    Mute,
    /// Reply once everything sent before it is handled.
    Flush(Sender<()>),
    Stop,
}

struct Inner {
    os: Arc<dyn MicOs>,
    out: Arc<dyn SoundOut>,
    st: Mutex<State>,
}

/// The Mic mute service. Cheap to clone (shared inside).
#[derive(Clone)]
pub struct MicMute {
    inner: Arc<Inner>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl MicMute {
    pub fn new(os: Arc<dyn MicOs>, out: Arc<dyn SoundOut>) -> Self {
        MicMute {
            inner: Arc::new(Inner {
                os,
                out,
                st: Mutex::new(State {
                    choice: MicChoice::Default,
                    sound: SoundSettings::default(),
                    undo: None,
                    muted_by_us: Vec::new(),
                    watched: None,
                    mute_watch: None,
                    dev_watch: None,
                    on_change: None,
                    worker: None,
                }),
            }),
        }
    }

    // ---------- the mic ----------

    /// The Microphone popup's list (active capture devices).
    pub fn devices(&self) -> Result<Vec<MicDevice>> {
        self.inner.os.capture_devices()
    }

    pub fn choice(&self) -> MicChoice {
        lock(&self.inner.st).choice.clone()
    }

    /// Picks the mic. A mute WE made stays on across the switch: the old mic is unmuted silently and the new one muted
    /// silently, so no mic is left muted behind the user's back. A mute another app made is left alone. Moves the watch
    /// when watching.
    pub fn set_choice(&self, choice: MicChoice) -> Result<MicState> {
        let old = self.resolve(&self.choice()).ok();
        let new = self.resolve(&choice)?;
        if let Some(old) = old {
            self.carry_mute(&old, &new)?;
        }
        lock(&self.inner.st).choice = choice;
        if lock(&self.inner.st).dev_watch.is_some() {
            self.retarget()?;
        }
        self.state()
    }

    /// A mute we made on `old` moves to `new`: `old` unmuted, `new` muted — both silently. Another app's mute is not ours to move.
    fn carry_mute(&self, old: &str, new: &str) -> Result<()> {
        let ours = lock(&self.inner.st).muted_by_us.iter().any(|d| d == old);
        if old != new && ours && self.inner.os.is_muted(old)? {
            self.os_set(old, false)?;
            if !self.inner.os.is_muted(new)? {
                self.os_set(new, true)?;
            }
        }
        Ok(())
    }

    /// The one place that changes a mute flag: the OS call + the "muted by us" list. No lock is held across the OS
    /// call (Windows may deliver the change event on another thread meanwhile).
    fn os_set(&self, id: &str, muted: bool) -> Result<()> {
        self.inner.os.set_muted(id, muted)?;
        let mut st = lock(&self.inner.st);
        st.muted_by_us.retain(|d| d != id);
        if muted {
            st.muted_by_us.push(id.to_string());
        }
        Ok(())
    }

    /// The device id the current choice points to.
    fn resolve(&self, choice: &MicChoice) -> Result<String> {
        match choice {
            MicChoice::Default => {
                self.inner.os.default_capture()?.ok_or_else(|| MicError::NoMic("no input device".into()))
            }
            MicChoice::Device(id) => {
                if self.inner.os.capture_devices()?.iter().any(|d| &d.id == id) {
                    Ok(id.clone())
                } else {
                    Err(MicError::NoMic("the picked microphone is not plugged in or is disabled".into()))
                }
            }
        }
    }

    /// The current state (device + muted). No mic → `device: None, muted: false`.
    pub fn state(&self) -> Result<MicState> {
        let id = match self.resolve(&self.choice()) {
            Ok(id) => id,
            Err(MicError::NoMic(_)) => return Ok(MicState { device: None, muted: false }),
            Err(e) => return Err(e),
        };
        let muted = self.inner.os.is_muted(&id)?;
        let device = self.inner.os.capture_devices()?.into_iter().find(|d| d.id == id);
        Ok(MicState { device, muted })
    }

    // ---------- actions (the app layer maps keys to these) ----------

    /// Mutes the chosen mic (+ sound). Already muted → nothing happens (no sound).
    pub fn mute(&self) -> Result<MicState> {
        self.set(true, true)
    }

    /// Unmutes the chosen mic (+ sound). Already live → nothing happens (no sound).
    pub fn unmute(&self) -> Result<MicState> {
        self.set(false, true)
    }

    /// One key: muted ↔ live.
    pub fn toggle(&self) -> Result<MicState> {
        let id = self.resolve(&self.choice())?;
        let muted = self.inner.os.is_muted(&id)?;
        self.set(!muted, true)
    }

    /// Switching the card off: stops watching and **silently unmutes if muted** (so
    /// the mic can't stay stuck muted) — the chosen mic, plus any other mic this crate muted (e.g. the old default)
    /// that is still plugged in and muted. No sound.
    pub fn turn_off(&self) -> Result<MicState> {
        self.stop_watching();
        let ours = lock(&self.inner.st).muted_by_us.clone();
        let present: Vec<String> = self.inner.os.capture_devices()?.into_iter().map(|d| d.id).collect();
        for id in ours.iter().filter(|id| present.contains(id)) {
            if self.inner.os.is_muted(id)? {
                self.os_set(id, false)?;
            }
        }
        match self.resolve(&self.choice()) {
            Ok(_) => self.set(false, false),
            Err(MicError::NoMic(_)) => Ok(MicState { device: None, muted: false }),
            Err(e) => Err(e),
        }
    }

    /// Puts the mute flag back to what it was before our last change (silently). Nothing to undo → `Ok(None)`.
    pub fn undo(&self) -> Result<Option<MicState>> {
        let Some((id, was)) = lock(&self.inner.st).undo.take() else { return Ok(None) };
        self.os_set(&id, was)?;
        self.state().map(Some)
    }

    fn set(&self, muted: bool, with_sound: bool) -> Result<MicState> {
        let id = self.resolve(&self.choice())?;
        let was = self.inner.os.is_muted(&id)?;
        if was != muted {
            self.os_set(&id, muted)?;
            let settings = {
                let mut st = lock(&self.inner.st);
                st.undo = Some((id.clone(), was));
                st.sound
            };
            if with_sound {
                if let Some(s) = settings.sound_for(muted) {
                    // a failed sound never fails the mute itself
                    let _ = sound::play(self.inner.out.as_ref(), s, settings.volume);
                }
            }
        }
        let device = self.inner.os.capture_devices()?.into_iter().find(|d| d.id == id);
        Ok(MicState { device, muted })
    }

    // ---------- sound ----------

    pub fn sound(&self) -> SoundSettings {
        lock(&self.inner.st).sound
    }

    /// Volume above 100 is stored as 100.
    pub fn set_sound(&self, mut s: SoundSettings) {
        s.volume = s.volume.min(100);
        lock(&self.inner.st).sound = s;
    }

    /// ▶ / picking a sound / letting go of the volume slider: plays `sound` at the current volume. None → nothing.
    pub fn preview(&self, s: Sound) -> Result<()> {
        sound::play(self.inner.out.as_ref(), s, self.sound().volume)
    }

    // ---------- watching ----------

    /// Starts the change event (another app muting the mic, the default mic changing, unplugging). Event-driven:
    /// Windows calls us; nothing polls. Our own actions don't call `on_change` (they return the state).
    pub fn start_watching(&self, on_change: ChangeFn) -> Result<()> {
        self.stop_watching();
        let weak = Arc::downgrade(&self.inner);
        // device changes are handed to a small worker: Windows forbids (un)registering inside its device callback
        let (tx, rx) = channel::<Job>();
        let w = weak.clone();
        let worker = std::thread::Builder::new()
            .name("bu-micmute-watch".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    match job {
                        Job::Retarget => {
                            if let Some(inner) = w.upgrade() {
                                let me = MicMute { inner };
                                let _ = me.retarget();
                                me.notify();
                            }
                        }
                        Job::Mute => {
                            if let Some(inner) = w.upgrade() {
                                MicMute { inner }.notify();
                            }
                        }
                        Job::Flush(done) => {
                            let _ = done.send(());
                        }
                        Job::Stop => break,
                    }
                }
            })
            .map_err(|e| MicError::Os { context: format!("watch thread: {e}"), code: 0 })?;
        let txc = tx.clone();
        let sink: EventSink = Arc::new(move |ev| {
            if matches!(ev, MicEvent::DefaultChanged { .. } | MicEvent::DevicesChanged) {
                let _ = txc.send(Job::Retarget);
            }
        });
        let dev_watch = match self.inner.os.watch_devices(sink) {
            Ok(w) => w,
            Err(e) => {
                let _ = tx.send(Job::Stop);
                let _ = worker.join();
                return Err(e);
            }
        };
        {
            let mut st = lock(&self.inner.st);
            st.on_change = Some(on_change);
            st.dev_watch = Some(dev_watch);
            st.worker = Some((tx, worker));
        }
        self.retarget()
    }

    /// Stops every watch (nothing of ours stays registered with Windows).
    pub fn stop_watching(&self) {
        let (mute_w, dev_w, worker) = {
            let mut st = lock(&self.inner.st);
            st.on_change = None;
            st.watched = None;
            (st.mute_watch.take(), st.dev_watch.take(), st.worker.take())
        };
        drop(mute_w);
        drop(dev_w);
        if let Some((tx, h)) = worker {
            let _ = tx.send(Job::Stop);
            if h.thread().id() != std::thread::current().id() {
                let _ = h.join();
            }
        }
    }

    pub fn is_watching(&self) -> bool {
        lock(&self.inner.st).dev_watch.is_some()
    }

    /// Waits until every event delivered so far is handled (tests; the app after a device change). Never call it from
    /// inside the change callback — the worker would wait on itself.
    pub fn settle(&self) {
        let tx = lock(&self.inner.st).worker.as_ref().map(|(tx, _)| tx.clone());
        if let Some(tx) = tx {
            let (done_tx, done_rx) = channel();
            if tx.send(Job::Flush(done_tx)).is_ok() {
                let _ = done_rx.recv();
            }
        }
    }

    /// Puts the mute watch on the mic the choice points to now.
    fn retarget(&self) -> Result<()> {
        let target = self.resolve(&self.choice()).ok();
        let (old_id, old_watch) = {
            let mut st = lock(&self.inner.st);
            if st.watched == target && st.mute_watch.is_some() == target.is_some() {
                return Ok(());
            }
            (st.watched.take(), st.mute_watch.take())
        };
        drop(old_watch);
        let Some(id) = target else { return Ok(()) };
        // the default mic changed while we had it muted → the new default is muted too (nothing gets through)
        if let Some(old) = old_id {
            if self.inner.os.capture_devices()?.iter().any(|d| d.id == old) {
                self.carry_mute(&old, &id)?; // moves only a mute we made
            }
        }
        // Windows' audio thread only posts the event; the worker does the reading (no COM work inside the callback)
        let Some(tx) = lock(&self.inner.st).worker.as_ref().map(|(tx, _)| tx.clone()) else { return Ok(()) };
        let sink: EventSink = Arc::new(move |ev| {
            if let MicEvent::Mute { by_us: false, .. } = ev {
                let _ = tx.send(Job::Mute);
            }
        });
        let w = self.inner.os.watch_mute(&id, sink)?;
        let mut st = lock(&self.inner.st);
        if st.dev_watch.is_none() {
            return Ok(()); // stopped meanwhile — drop the new watch
        }
        st.watched = Some(id);
        st.mute_watch = Some(w);
        Ok(())
    }

    fn notify(&self) {
        let cb = lock(&self.inner.st).on_change.clone();
        if let Some(cb) = cb {
            if let Ok(s) = self.state() {
                cb(s);
            }
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        let st = self.st.get_mut().unwrap_or_else(|e| e.into_inner());
        st.mute_watch.take();
        st.dev_watch.take();
        if let Some((tx, _h)) = st.worker.take() {
            let _ = tx.send(Job::Stop); // the worker holds only a Weak; it ends on its own
        }
    }
}
