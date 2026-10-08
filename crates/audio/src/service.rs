//! The Audio tab's rules on top of an [`AudioOs`]: read state, apply a change (it returns a [`Change`] that remembers
//! the old value), [`AudioService::undo`] it.

use crate::model::*;
use crate::os::AudioOs;
use crate::{AudioError, Result};
use std::collections::HashMap;

/// One change, with everything needed to put it back.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// A new default device (all three roles); `before` = the old defaults.
    Default { flow: Flow, before: Defaults },
    DeviceVolume { id: String, before: f32 },
    DeviceMute { id: String, before: bool },
    /// Device on/off; `moved` = the defaults before Windows was moved off the switched-off device.
    DeviceOn { id: String, flow: Flow, before_on: bool, moved: Option<Defaults> },
    /// An app's volume / mute: every session's old (volume, muted).
    App { group: String, before: Vec<(String, f32, bool)> },
}

pub struct AudioService<O: AudioOs> {
    os: O,
    looks: HashMap<String, AppLook>,
}

/// An app's group key: its exe path (lower case), else its pid, else "system".
pub fn group_of(s: &SessionInfo) -> String {
    if s.system {
        "system".into()
    } else if !s.exe_path.is_empty() {
        s.exe_path.to_lowercase()
    } else {
        format!("pid:{}", s.pid)
    }
}

impl<O: AudioOs> AudioService<O> {
    pub fn new(os: O) -> Self {
        AudioService { os, looks: HashMap::new() }
    }
    pub fn os(&self) -> &O {
        &self.os
    }
    pub fn os_mut(&mut self) -> &mut O {
        &mut self.os
    }

    // ---------------------------------------------------------------------------------------------- devices
    /// The popup's rows: devices that are on or switched off (unplugged ones are hidden), the current one ✓, and the
    /// last device still on locked.
    pub fn device_rows(&mut self, flow: Flow) -> Result<Vec<DeviceRow>> {
        let devs: Vec<Device> = self.os.devices(flow)?.into_iter().filter(|d| d.state != DeviceState::Unplugged).collect();
        let cur = self.os.defaults(flow)?.console;
        let on_count = devs.iter().filter(|d| d.state == DeviceState::On).count();
        Ok(devs
            .into_iter()
            .map(|d| {
                let on = d.state == DeviceState::On;
                DeviceRow { current: cur.as_deref() == Some(d.id.as_str()), on, switch_locked: on && on_count == 1, device: d }
            })
            .collect())
    }

    /// The current default (Console role) — the Output / Input row's device.
    pub fn current(&mut self, flow: Flow) -> Result<Option<Device>> {
        let cur = self.os.defaults(flow)?.console;
        Ok(cur.and_then(|id| self.os.devices(flow).ok()?.into_iter().find(|d| d.id == id)))
    }

    pub fn defaults(&mut self, flow: Flow) -> Result<Defaults> {
        self.os.defaults(flow)
    }

    fn device(&mut self, flow: Flow, id: &str) -> Result<Device> {
        self.os.devices(flow)?.into_iter().find(|d| d.id == id).ok_or_else(|| AudioError::NotFound(id.into()))
    }

    /// Picking a device in the popup: it becomes Windows' default for all three roles. A switched-off device can't be
    /// picked (`DeviceOff`).
    pub fn select_default(&mut self, flow: Flow, id: &str) -> Result<Change> {
        let d = self.device(flow, id)?;
        if d.state != DeviceState::On {
            return Err(AudioError::DeviceOff(d.name));
        }
        let before = self.os.defaults(flow)?;
        for r in ROLES {
            if before.get(r).map(String::as_str) != Some(id) {
                self.os.set_default(id, r)?;
            }
        }
        Ok(Change::Default { flow, before })
    }

    pub fn device_volume(&mut self, id: &str) -> Result<VolumeMute> {
        self.os.volume(id)
    }

    /// The level pill's slider. 0.0 … 1.0 (clamped).
    pub fn set_device_volume(&mut self, id: &str, volume: f32) -> Result<Change> {
        let before = self.os.volume(id)?.volume;
        self.os.set_volume(id, clamp01(volume))?;
        Ok(Change::DeviceVolume { id: id.into(), before })
    }

    pub fn set_device_mute(&mut self, id: &str, muted: bool) -> Result<Change> {
        let before = self.os.volume(id)?.muted;
        self.os.set_mute(id, muted)?;
        Ok(Change::DeviceMute { id: id.into(), before })
    }

    /// A device's own switch. Off: the last device still on is refused (`LastDeviceOn`); switching off the device in use
    /// first moves Windows (each role that used it) to the first device still on. Needs admin → `NeedsAdmin` from the OS.
    pub fn set_device_on(&mut self, flow: Flow, id: &str, on: bool) -> Result<Change> {
        let d = self.device(flow, id)?;
        let before_on = d.state == DeviceState::On;
        if before_on == on {
            return Ok(Change::DeviceOn { id: id.into(), flow, before_on, moved: None });
        }
        let mut moved = None;
        if !on {
            let others: Vec<Device> =
                self.os.devices(flow)?.into_iter().filter(|x| x.id != id && x.state == DeviceState::On).collect();
            let Some(first) = others.first() else { return Err(AudioError::LastDeviceOn) };
            let defs = self.os.defaults(flow)?;
            if ROLES.iter().any(|r| defs.get(*r).map(String::as_str) == Some(id)) {
                for r in ROLES {
                    if defs.get(r).map(String::as_str) == Some(id) {
                        self.os.set_default(&first.id, r)?;
                    }
                }
                moved = Some(defs);
            }
        }
        if let Err(e) = self.os.set_enabled(id, on) {
            // put the defaults back if the switch itself failed (e.g. needs admin)
            if let Some(defs) = &moved {
                let _ = self.restore_defaults(defs);
            }
            return Err(e);
        }
        Ok(Change::DeviceOn { id: id.into(), flow, before_on, moved })
    }

    /// Puts a flow's defaults to `to` (every role it names; the reset's "how your PC was"): each device must still be there
    /// and on (`NotFound` / `DeviceOff`); roles already right are left alone.
    pub fn set_defaults(&mut self, flow: Flow, to: &Defaults) -> Result<()> {
        let devs = self.os.devices(flow)?;
        for r in ROLES {
            if let Some(id) = to.get(r) {
                let d = devs.iter().find(|d| &d.id == id).ok_or_else(|| AudioError::NotFound(id.clone()))?;
                if d.state != DeviceState::On {
                    return Err(AudioError::DeviceOff(d.name.clone()));
                }
            }
        }
        let now = self.os.defaults(flow)?;
        for r in ROLES {
            if let Some(id) = to.get(r) {
                if now.get(r) != Some(id) {
                    self.os.set_default(id, r)?;
                }
            }
        }
        Ok(())
    }

    fn restore_defaults(&mut self, before: &Defaults) -> Result<()> {
        for r in ROLES {
            if let Some(id) = before.get(r) {
                self.os.set_default(id, r)?;
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------------------------------------- mixer
    /// The mixer for one output device: one row per app that is making sound (an Active session), every session of that
    /// app grouped in it; System sounds always listed, last.
    pub fn apps(&mut self, output_id: &str) -> Result<Vec<AppRow>> {
        let sessions = self.os.sessions(output_id)?;
        let mut order: Vec<String> = Vec::new();
        let mut groups: HashMap<String, Vec<SessionInfo>> = HashMap::new();
        for s in sessions {
            let g = group_of(&s);
            if !groups.contains_key(&g) {
                order.push(g.clone());
            }
            groups.entry(g).or_default().push(s);
        }
        let mut rows = Vec::new();
        for g in order {
            let ss = &groups[&g];
            let system = ss.iter().any(|s| s.system);
            if !system && !ss.iter().any(|s| s.state == SessionState::Active) {
                continue;
            }
            let look = match self.looks.get(&g) {
                Some(l) if l.icon.is_some() || system => l.clone(),
                _ => {
                    // ask again until the icon is there (the real layer reads icons on a helper thread)
                    let l = self.os.app_look(&ss[0]);
                    self.looks.insert(g.clone(), l.clone());
                    l
                }
            };
            rows.push(AppRow {
                group: g,
                look,
                sessions: ss.iter().map(|s| s.key.clone()).collect(),
                pids: dedup(ss.iter().map(|s| s.pid).collect()),
                volume: ss.iter().map(|s| s.volume).fold(0.0, f32::max),
                muted: ss.iter().all(|s| s.muted),
                system,
            });
        }
        rows.sort_by_key(|r| r.system);
        Ok(rows)
    }

    fn group_sessions(&mut self, output_id: &str, group: &str) -> Result<Vec<SessionInfo>> {
        let ss: Vec<SessionInfo> = self.os.sessions(output_id)?.into_iter().filter(|s| group_of(s) == group).collect();
        if ss.is_empty() {
            return Err(AudioError::NotFound(group.into()));
        }
        Ok(ss)
    }

    /// An app's slider / typed %: every session of the app gets it. A muted app is unmuted (as in Windows).
    pub fn set_app_volume(&mut self, output_id: &str, group: &str, volume: f32) -> Result<Change> {
        let ss = self.group_sessions(output_id, group)?;
        let before = ss.iter().map(|s| (s.key.clone(), s.volume, s.muted)).collect();
        for s in &ss {
            self.os.set_session_volume(&s.key, clamp01(volume))?;
            if s.muted {
                self.os.set_session_mute(&s.key, false)?;
            }
        }
        Ok(Change::App { group: group.into(), before })
    }

    /// An app's mute button.
    pub fn set_app_mute(&mut self, output_id: &str, group: &str, muted: bool) -> Result<Change> {
        let ss = self.group_sessions(output_id, group)?;
        let before = ss.iter().map(|s| (s.key.clone(), s.volume, s.muted)).collect();
        for s in &ss {
            if s.muted != muted {
                self.os.set_session_mute(&s.key, muted)?;
            }
        }
        Ok(Change::App { group: group.into(), before })
    }

    // ---------------------------------------------------------------------------------------------- undo
    /// Puts a change back. Sessions that closed meanwhile are skipped.
    pub fn undo(&mut self, change: &Change) -> Result<()> {
        match change {
            Change::Default { before, .. } => self.restore_defaults(before),
            Change::DeviceVolume { id, before } => self.os.set_volume(id, *before),
            Change::DeviceMute { id, before } => self.os.set_mute(id, *before),
            Change::DeviceOn { id, before_on, moved, .. } => {
                self.os.set_enabled(id, *before_on)?;
                if let Some(defs) = moved {
                    self.restore_defaults(defs)?;
                }
                Ok(())
            }
            Change::App { before, .. } => {
                for (key, vol, muted) in before {
                    match self.os.set_session_volume(key, *vol) {
                        Err(AudioError::NotFound(_)) => continue,
                        r => r?,
                    }
                    self.os.set_session_mute(key, *muted)?;
                }
                Ok(())
            }
        }
    }
}

fn clamp01(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

fn dedup(mut v: Vec<u32>) -> Vec<u32> {
    v.sort_unstable();
    v.dedup();
    v
}
