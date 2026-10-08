//! FAKE OS layers for tests: a pretend audio stack ([`FakeMicOs`]) and a speaker that only records ([`FakeSoundOut`]).
//! Nothing here touches a real mic or speaker.

use crate::os::{EventSink, MicDevice, MicEvent, MicOs, SoundOut, Watch};
use crate::{MicError, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct FakeWatch(Arc<AtomicBool>);
impl Watch for FakeWatch {}
impl Drop for FakeWatch {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct MicWorld {
    devices: Vec<(String, String)>,
    default: Option<String>,
    muted: HashMap<String, bool>,
    mute_watches: Vec<(String, EventSink, Arc<AtomicBool>)>,
    dev_watches: Vec<(EventSink, Arc<AtomicBool>)>,
    /// Every `set_muted` call: (id, muted).
    sets: Vec<(String, bool)>,
    fail_next_set: Option<MicError>,
    read_only: bool,
}

/// A pretend audio stack. Watches fire synchronously, like a Windows callback arriving on another thread would.
#[derive(Clone, Default)]
pub struct FakeMicOs {
    w: Arc<Mutex<MicWorld>>,
}

impl FakeMicOs {
    /// Mics as (id, name); the first one is Windows' default.
    pub fn new(mics: &[(&str, &str)]) -> Self {
        let f = FakeMicOs::default();
        {
            let mut w = lock(&f.w);
            for (id, name) in mics {
                w.devices.push((id.to_string(), name.to_string()));
                w.muted.insert(id.to_string(), false);
            }
            w.default = mics.first().map(|m| m.0.to_string());
        }
        f
    }

    /// Like `RealMicOs::read_only()`: every change refused.
    pub fn read_only(self) -> Self {
        lock(&self.w).read_only = true;
        self
    }

    /// The mute flag as the pretend Windows holds it.
    pub fn muted(&self, id: &str) -> bool {
        lock(&self.w).muted.get(id).copied().unwrap_or(false)
    }

    pub fn sets(&self) -> Vec<(String, bool)> {
        lock(&self.w).sets.clone()
    }

    pub fn fail_next_set(&self, e: MicError) {
        lock(&self.w).fail_next_set = Some(e);
    }

    /// Mute watches still registered.
    pub fn live_mute_watches(&self) -> Vec<String> {
        lock(&self.w).mute_watches.iter().filter(|w| w.2.load(Ordering::SeqCst)).map(|w| w.0.clone()).collect()
    }

    pub fn live_device_watches(&self) -> usize {
        lock(&self.w).dev_watches.iter().filter(|w| w.1.load(Ordering::SeqCst)).count()
    }

    /// Another app (Discord, Sound settings, a headset button) changes the mute flag.
    pub fn external_mute(&self, id: &str, muted: bool) {
        lock(&self.w).muted.insert(id.to_string(), muted);
        self.fire_mute(id, muted, false);
    }

    /// The user picks another default input device in Windows.
    pub fn set_default(&self, id: Option<&str>) {
        lock(&self.w).default = id.map(str::to_string);
        self.fire_dev(MicEvent::DefaultChanged { device_id: id.map(str::to_string) });
    }

    /// A mic is unplugged / disabled.
    pub fn remove(&self, id: &str) {
        {
            let mut w = lock(&self.w);
            w.devices.retain(|d| d.0 != id);
            if w.default.as_deref() == Some(id) {
                w.default = w.devices.first().map(|d| d.0.clone());
            }
        }
        self.fire_dev(MicEvent::DevicesChanged);
    }

    fn fire_mute(&self, id: &str, muted: bool, by_us: bool) {
        let sinks: Vec<EventSink> = lock(&self.w)
            .mute_watches
            .iter()
            .filter(|w| w.0 == id && w.2.load(Ordering::SeqCst))
            .map(|w| w.1.clone())
            .collect();
        for s in sinks {
            s(MicEvent::Mute { device_id: id.to_string(), muted, by_us });
        }
    }

    fn fire_dev(&self, ev: MicEvent) {
        let sinks: Vec<EventSink> =
            lock(&self.w).dev_watches.iter().filter(|w| w.1.load(Ordering::SeqCst)).map(|w| w.0.clone()).collect();
        for s in sinks {
            s(ev.clone());
        }
    }

    fn exists(&self, id: &str) -> Result<()> {
        if lock(&self.w).devices.iter().any(|d| d.0 == id) {
            Ok(())
        } else {
            Err(MicError::Os { context: format!("open endpoint {id}"), code: 0x8889_0004 }) // AUDCLNT_E_DEVICE_INVALIDATED
        }
    }
}

impl MicOs for FakeMicOs {
    fn capture_devices(&self) -> Result<Vec<MicDevice>> {
        let w = lock(&self.w);
        Ok(w.devices
            .iter()
            .map(|(id, name)| MicDevice { id: id.clone(), name: name.clone(), is_default: w.default.as_ref() == Some(id) })
            .collect())
    }

    fn default_capture(&self) -> Result<Option<String>> {
        Ok(lock(&self.w).default.clone())
    }

    fn is_muted(&self, id: &str) -> Result<bool> {
        self.exists(id)?;
        Ok(self.muted(id))
    }

    fn set_muted(&self, id: &str, muted: bool) -> Result<()> {
        self.exists(id)?;
        {
            let mut w = lock(&self.w);
            if w.read_only {
                return Err(MicError::ReadOnly(format!("set_muted({id}, {muted})")));
            }
            if let Some(e) = w.fail_next_set.take() {
                return Err(e);
            }
            w.sets.push((id.to_string(), muted));
            let old = w.muted.insert(id.to_string(), muted);
            if old == Some(muted) {
                return Ok(()); // Windows sends no notification when nothing changed
            }
        }
        self.fire_mute(id, muted, true);
        Ok(())
    }

    fn watch_mute(&self, id: &str, sink: EventSink) -> Result<Box<dyn Watch>> {
        self.exists(id)?;
        let alive = Arc::new(AtomicBool::new(true));
        lock(&self.w).mute_watches.push((id.to_string(), sink, alive.clone()));
        Ok(Box::new(FakeWatch(alive)))
    }

    fn watch_devices(&self, sink: EventSink) -> Result<Box<dyn Watch>> {
        let alive = Arc::new(AtomicBool::new(true));
        lock(&self.w).dev_watches.push((sink, alive.clone()));
        Ok(Box::new(FakeWatch(alive)))
    }
}

/// A speaker that only records what it was asked to play. Silent by construction.
#[derive(Clone, Default)]
pub struct FakeSoundOut {
    played: Arc<Mutex<Vec<Vec<u8>>>>,
    stops: Arc<Mutex<u32>>,
    fail: Arc<AtomicBool>,
}

impl FakeSoundOut {
    pub fn played(&self) -> Vec<Vec<u8>> {
        lock(&self.played).clone()
    }
    pub fn stops(&self) -> u32 {
        *lock(&self.stops)
    }
    /// Every play fails from now on (a broken speaker must not break muting).
    pub fn fail(&self) {
        self.fail.store(true, Ordering::SeqCst);
    }
}

impl SoundOut for FakeSoundOut {
    fn play_wav(&self, wav: Vec<u8>) -> Result<()> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(MicError::Os { context: "PlaySound".into(), code: 0 });
        }
        lock(&self.played).push(wav);
        Ok(())
    }
    fn stop(&self) {
        *lock(&self.stops) += 1;
    }
}
