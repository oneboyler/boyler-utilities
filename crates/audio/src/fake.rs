//! The fake OS layer for tests: devices, defaults, volumes, sessions in memory; every change logged. Never touches
//! Windows.

use crate::model::*;
use crate::os::{AudioOs, GREY};
use crate::{AudioError, Result};
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct FakeOs {
    pub devices: Vec<Device>,
    pub defaults: HashMap<(Flow, Role), String>,
    pub volumes: HashMap<String, VolumeMute>,
    pub peaks: HashMap<String, f32>,
    /// (device id, session)
    pub sessions: Vec<(String, SessionInfo)>,
    pub session_peaks: HashMap<String, f32>,
    /// Every change, in order ("set_default <id> Console", "set_volume <id> 50" …).
    pub log: Vec<String>,
    /// Device on/off answers "access denied" (→ needs admin).
    pub enable_needs_admin: bool,
    /// The next change fails with this error.
    pub fail_next: Option<AudioError>,
    /// Counts reads (to prove a stopped sampler reads nothing).
    pub reads: u64,
}

pub fn dev(id: &str, name: &str, kind: DeviceKind, flow: Flow) -> Device {
    Device { id: id.into(), name: name.into(), kind, flow, state: DeviceState::On }
}

pub fn session(key: &str, pid: u32, exe: &str, state: SessionState, volume: f32) -> SessionInfo {
    SessionInfo {
        key: key.into(),
        pid,
        process_started: 1000 + pid as u64,
        exe_path: exe.into(),
        display_name: String::new(),
        system: false,
        state,
        volume,
        muted: false,
    }
}

impl FakeOs {
    /// The drawing's setup: 4 outputs (Arctis default 74 %), 4 inputs (MV7 default 90 %), a few apps.
    pub fn drawing() -> Self {
        let mut f = FakeOs { devices: vec![
            dev("spk", "Speakers (Realtek)", DeviceKind::Speakers, Flow::Output),
            dev("arctis", "Headphones (Arctis Nova)", DeviceKind::Headphones, Flow::Output),
            dev("nv", "Monitor (NVIDIA HD Audio)", DeviceKind::Monitor, Flow::Output),
            dev("ds", "Wireless Controller (DualSense)", DeviceKind::Controller, Flow::Output),
            dev("mv7", "Microphone (Shure MV7)", DeviceKind::Microphone, Flow::Input),
            dev("arctis-mic", "Headset Microphone (Arctis Nova)", DeviceKind::Headphones, Flow::Input),
            dev("c920", "Webcam Microphone (C920)", DeviceKind::Webcam, Flow::Input),
            dev("ds-mic", "Wireless Controller (DualSense)", DeviceKind::Controller, Flow::Input),
        ], ..Default::default() };
        for r in ROLES {
            f.defaults.insert((Flow::Output, r), "arctis".into());
            f.defaults.insert((Flow::Input, r), "mv7".into());
        }
        for d in &f.devices {
            f.volumes.insert(d.id.clone(), VolumeMute { volume: 0.5, muted: false });
        }
        f.volumes.insert("arctis".into(), VolumeMute { volume: 0.74, muted: false });
        f.volumes.insert("mv7".into(), VolumeMute { volume: 0.90, muted: false });
        let mut sys = session("sys", 0, "", SessionState::Inactive, 0.5);
        sys.system = true;
        f.sessions = vec![
            ("arctis".into(), session("spotify-1", 100, r"C:\Apps\Spotify.exe", SessionState::Active, 0.64)),
            ("arctis".into(), session("discord-1", 200, r"C:\Apps\Discord\Discord.exe", SessionState::Active, 0.8)),
            ("arctis".into(), session("discord-2", 201, r"C:\Apps\Discord\Discord.exe", SessionState::Active, 0.6)),
            ("arctis".into(), session("chrome-1", 300, r"C:\Apps\Chrome\chrome.exe", SessionState::Inactive, 1.0)),
            ("arctis".into(), sys),
            ("spk".into(), session("game-1", 400, r"C:\Games\Game.exe", SessionState::Active, 0.72)),
        ];
        f
    }

    fn change(&mut self, line: String) -> Result<()> {
        if let Some(e) = self.fail_next.take() {
            return Err(e);
        }
        self.log.push(line);
        Ok(())
    }

    fn device_mut(&mut self, id: &str) -> Result<&mut Device> {
        self.devices.iter_mut().find(|d| d.id == id).ok_or_else(|| AudioError::NotFound(id.into()))
    }

    fn session_mut(&mut self, key: &str) -> Result<&mut SessionInfo> {
        self.sessions.iter_mut().map(|(_, s)| s).find(|s| s.key == key).ok_or_else(|| AudioError::NotFound(key.into()))
    }

    /// Test helper: plug a device in (or back in).
    pub fn plug(&mut self, d: Device) {
        self.devices.retain(|x| x.id != d.id);
        self.volumes.entry(d.id.clone()).or_insert(VolumeMute { volume: 1.0, muted: false });
        self.devices.push(d);
    }
    /// Test helper: Windows switches the default by itself (as it does for a new device).
    pub fn windows_sets_default(&mut self, flow: Flow, id: &str) {
        for r in ROLES {
            self.defaults.insert((flow, r), id.into());
        }
    }
}

impl AudioOs for FakeOs {
    fn devices(&mut self, flow: Flow) -> Result<Vec<Device>> {
        self.reads += 1;
        Ok(self.devices.iter().filter(|d| d.flow == flow).cloned().collect())
    }
    fn defaults(&mut self, flow: Flow) -> Result<Defaults> {
        self.reads += 1;
        let mut d = Defaults::default();
        for r in ROLES {
            d.set(r, self.defaults.get(&(flow, r)).cloned());
        }
        Ok(d)
    }
    fn set_default(&mut self, id: &str, role: Role) -> Result<()> {
        let flow = self.device_mut(id)?.flow;
        self.change(format!("set_default {id} {role:?}"))?;
        self.defaults.insert((flow, role), id.into());
        Ok(())
    }
    fn volume(&mut self, id: &str) -> Result<VolumeMute> {
        self.reads += 1;
        self.volumes.get(id).copied().ok_or_else(|| AudioError::NotFound(id.into()))
    }
    fn set_volume(&mut self, id: &str, volume: f32) -> Result<()> {
        self.device_mut(id)?;
        self.change(format!("set_volume {id} {}", (volume * 100.0).round()))?;
        self.volumes.entry(id.into()).or_insert(VolumeMute { volume, muted: false }).volume = volume;
        Ok(())
    }
    fn set_mute(&mut self, id: &str, muted: bool) -> Result<()> {
        self.device_mut(id)?;
        self.change(format!("set_mute {id} {muted}"))?;
        self.volumes.entry(id.into()).or_insert(VolumeMute { volume: 1.0, muted }).muted = muted;
        Ok(())
    }
    fn peak(&mut self, id: &str) -> Result<f32> {
        self.reads += 1;
        Ok(self.peaks.get(id).copied().unwrap_or(0.0))
    }
    fn set_enabled(&mut self, id: &str, on: bool) -> Result<()> {
        self.device_mut(id)?;
        if self.enable_needs_admin {
            return Err(AudioError::NeedsAdmin(format!("switch {id} {}", if on { "on" } else { "off" })));
        }
        self.change(format!("set_enabled {id} {on}"))?;
        let d = self.device_mut(id)?;
        d.state = if on { DeviceState::On } else { DeviceState::Off };
        Ok(())
    }
    fn sessions(&mut self, device_id: &str) -> Result<Vec<SessionInfo>> {
        self.reads += 1;
        Ok(self.sessions.iter().filter(|(d, s)| d == device_id && s.state != SessionState::Expired).map(|(_, s)| s.clone()).collect())
    }
    fn set_session_volume(&mut self, key: &str, volume: f32) -> Result<()> {
        self.session_mut(key)?;
        self.change(format!("set_session_volume {key} {}", (volume * 100.0).round()))?;
        self.session_mut(key)?.volume = volume;
        Ok(())
    }
    fn set_session_mute(&mut self, key: &str, muted: bool) -> Result<()> {
        self.session_mut(key)?;
        self.change(format!("set_session_mute {key} {muted}"))?;
        self.session_mut(key)?.muted = muted;
        Ok(())
    }
    fn session_peak(&mut self, key: &str) -> Result<f32> {
        self.reads += 1;
        Ok(self.session_peaks.get(key).copied().unwrap_or(0.0))
    }
    fn app_look(&mut self, s: &SessionInfo) -> AppLook {
        let name = if s.system {
            "System sounds".to_string()
        } else {
            std::path::Path::new(&s.exe_path).file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| format!("App {}", s.pid))
        };
        AppLook { name, icon: None, colour: GREY.0, colour2: GREY.1 }
    }
}

/// A [`FakeOs`] shared between a test and a worker thread ([`crate::page::AudioPage`], the engine).
#[derive(Debug, Clone, Default)]
pub struct SharedFake(pub std::sync::Arc<std::sync::Mutex<FakeOs>>);

impl SharedFake {
    pub fn new(f: FakeOs) -> Self {
        SharedFake(std::sync::Arc::new(std::sync::Mutex::new(f)))
    }
    pub fn with<R>(&self, f: impl FnOnce(&mut FakeOs) -> R) -> R {
        f(&mut self.0.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

impl AudioOs for SharedFake {
    fn devices(&mut self, flow: Flow) -> Result<Vec<Device>> {
        self.with(|f| f.devices(flow))
    }
    fn defaults(&mut self, flow: Flow) -> Result<Defaults> {
        self.with(|f| f.defaults(flow))
    }
    fn set_default(&mut self, id: &str, role: Role) -> Result<()> {
        self.with(|f| f.set_default(id, role))
    }
    fn volume(&mut self, id: &str) -> Result<VolumeMute> {
        self.with(|f| f.volume(id))
    }
    fn set_volume(&mut self, id: &str, volume: f32) -> Result<()> {
        self.with(|f| f.set_volume(id, volume))
    }
    fn set_mute(&mut self, id: &str, muted: bool) -> Result<()> {
        self.with(|f| f.set_mute(id, muted))
    }
    fn peak(&mut self, id: &str) -> Result<f32> {
        self.with(|f| f.peak(id))
    }
    fn set_enabled(&mut self, id: &str, on: bool) -> Result<()> {
        self.with(|f| f.set_enabled(id, on))
    }
    fn sessions(&mut self, device_id: &str) -> Result<Vec<SessionInfo>> {
        self.with(|f| f.sessions(device_id))
    }
    fn set_session_volume(&mut self, key: &str, volume: f32) -> Result<()> {
        self.with(|f| f.set_session_volume(key, volume))
    }
    fn set_session_mute(&mut self, key: &str, muted: bool) -> Result<()> {
        self.with(|f| f.set_session_mute(key, muted))
    }
    fn session_peak(&mut self, key: &str) -> Result<f32> {
        self.with(|f| f.session_peak(key))
    }
    fn app_look(&mut self, s: &SessionInfo) -> AppLook {
        self.with(|f| f.app_look(s))
    }
}
