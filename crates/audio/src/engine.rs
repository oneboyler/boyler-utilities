//! The always-on part (menu closed): Keep my devices + New apps volume + the input volume lock acting on Windows' events.
//! The real watcher ([`crate::watch`]) only turns COM callbacks into [`Event`]s; everything it does is decided here, so
//! the fake proves it.

use crate::keep::{DeviceEvent, KeepDevices};
use crate::lock::MicLock;
use crate::model::*;
use crate::newvol::NewAppsVolume;
use crate::os::AudioOs;
use crate::Result;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Device(DeviceEvent),
    SessionCreated(SessionInfo),
    /// Order 118: a volume notification of the default input device (the lock listens to it).
    InputVolume { id: String, volume: f32 },
}

/// What the engine did for one event (for the app's log / tests).
#[derive(Debug, Clone, PartialEq)]
pub enum Did {
    PutBack { id: String, role: Role },
    NewAppVolume { key: String, pid: u32, volume: f32 },
    /// Order 118: the locked input volume put back (`from` = what Windows / an app had made it).
    LockPutBack { id: String, from: f32, to: f32 },
    Failed(String),
}

pub struct Engine<O: AudioOs> {
    pub os: O,
    pub keep: KeepDevices,
    pub new_apps: NewAppsVolume,
    /// Order 118: the locked input volume (off until [`Engine::set_lock`]).
    pub lock: MicLock,
}

impl<O: AudioOs> Engine<O> {
    /// Reads what is plugged in and the defaults now.
    pub fn new(mut os: O, keep_on: bool, new_apps: NewAppsVolume) -> Result<Self> {
        let mut present = Vec::new();
        let mut defaults = Vec::new();
        let mut input = None;
        for f in FLOWS {
            present.extend(os.devices(f)?.into_iter().filter(|d| d.state == DeviceState::On).map(|d| d.id));
            let d = os.defaults(f)?;
            for r in ROLES {
                if let Some(id) = d.get(r) {
                    defaults.push(((f, r), id.clone()));
                    if f == Flow::Input && r == Role::Console {
                        input = Some(id.clone());
                    }
                }
            }
        }
        Ok(Engine { os, keep: KeepDevices::new(keep_on, present, defaults), new_apps, lock: MicLock::new(None, input) })
    }

    /// The lock switch (None = unlocked): a lock is enforced at once on the default input device.
    pub fn set_lock(&mut self, locked: Option<f32>) -> Vec<Did> {
        self.lock.set(locked);
        self.enforce_lock()
    }

    /// Reads the device the lock holds on and puts the locked value back if it differs.
    pub fn enforce_lock(&mut self) -> Vec<Did> {
        let Some(id) = self.lock.watching().map(str::to_string) else { return Vec::new() };
        match self.os.volume(&id) {
            Ok(v) if self.lock.must_put(v.volume) => self.put_lock_back(id, v.volume),
            Ok(_) => Vec::new(),
            Err(e) => vec![Did::Failed(format!("read input volume {id}: {e}"))],
        }
    }

    fn put_lock_back(&mut self, id: String, from: f32) -> Vec<Did> {
        let Some(to) = self.lock.locked() else { return Vec::new() };
        if let Err(e) = self.os.set_volume(&id, to) {
            return vec![Did::Failed(format!("lock put back {id}: {e}"))];
        }
        if let Ok(v) = self.os.volume(&id) {
            self.lock.note_settled(v.volume);
        }
        vec![Did::LockPutBack { id, from, to }]
    }

    pub fn handle(&mut self, ev: Event, now: Duration) -> Vec<Did> {
        match ev {
            Event::Device(d) => {
                let input_default = match &d {
                    DeviceEvent::DefaultChanged { flow: Flow::Input, role: Role::Console, id } => Some(id.clone()),
                    _ => None,
                };
                let mut did: Vec<Did> = self
                    .keep
                    .on_event(d, now)
                    .into_iter()
                    .map(|p| match self.os.set_default(&p.id, p.role) {
                        Ok(()) => Did::PutBack { id: p.id, role: p.role },
                        Err(e) => Did::Failed(format!("put back {} {:?}: {e}", p.id, p.role)),
                    })
                    .collect();
                if let Some(id) = input_default {
                    // the lock follows the default input device (a put-back by Keep my devices reports itself again and
                    // lands here with the device then in use)
                    if self.lock.set_input(id) {
                        did.extend(self.enforce_lock());
                    }
                }
                did
            }
            Event::SessionCreated(s) => match self.new_apps.on_session_created(&s) {
                None => Vec::new(),
                Some(v) => vec![match self.os.set_session_volume(&s.key, v) {
                    Ok(()) => Did::NewAppVolume { key: s.key, pid: s.pid, volume: v },
                    Err(e) => Did::Failed(format!("new app volume {}: {e}", s.key)),
                }],
            },
            Event::InputVolume { id, volume } => match self.lock.on_volume(&id, volume) {
                Some(_) => self.put_lock_back(id, volume),
                None => Vec::new(),
            },
        }
    }
}
