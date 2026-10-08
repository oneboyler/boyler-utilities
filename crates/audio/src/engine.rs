//! The always-on part (menu closed): Keep my devices + New apps volume acting on Windows' events. The real watcher
//! ([`crate::watch`]) only turns COM callbacks into [`Event`]s; everything it does is decided here, so the fake proves it.

use crate::keep::{DeviceEvent, KeepDevices};
use crate::model::*;
use crate::newvol::NewAppsVolume;
use crate::os::AudioOs;
use crate::Result;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Device(DeviceEvent),
    SessionCreated(SessionInfo),
}

/// What the engine did for one event (for the app's log / tests).
#[derive(Debug, Clone, PartialEq)]
pub enum Did {
    PutBack { id: String, role: Role },
    NewAppVolume { key: String, pid: u32, volume: f32 },
    Failed(String),
}

pub struct Engine<O: AudioOs> {
    pub os: O,
    pub keep: KeepDevices,
    pub new_apps: NewAppsVolume,
}

impl<O: AudioOs> Engine<O> {
    /// Reads what is plugged in and the defaults now.
    pub fn new(mut os: O, keep_on: bool, new_apps: NewAppsVolume) -> Result<Self> {
        let mut present = Vec::new();
        let mut defaults = Vec::new();
        for f in FLOWS {
            present.extend(os.devices(f)?.into_iter().filter(|d| d.state == DeviceState::On).map(|d| d.id));
            let d = os.defaults(f)?;
            for r in ROLES {
                if let Some(id) = d.get(r) {
                    defaults.push(((f, r), id.clone()));
                }
            }
        }
        Ok(Engine { os, keep: KeepDevices::new(keep_on, present, defaults), new_apps })
    }

    pub fn handle(&mut self, ev: Event, now: Duration) -> Vec<Did> {
        match ev {
            Event::Device(d) => self
                .keep
                .on_event(d, now)
                .into_iter()
                .map(|p| match self.os.set_default(&p.id, p.role) {
                    Ok(()) => Did::PutBack { id: p.id, role: p.role },
                    Err(e) => Did::Failed(format!("put back {} {:?}: {e}", p.id, p.role)),
                })
                .collect(),
            Event::SessionCreated(s) => match self.new_apps.on_session_created(&s) {
                None => Vec::new(),
                Some(v) => vec![match self.os.set_session_volume(&s.key, v) {
                    Ok(()) => Did::NewAppVolume { key: s.key, pid: s.pid, volume: v },
                    Err(e) => Did::Failed(format!("new app volume {}: {e}", s.key)),
                }],
            },
        }
    }
}
