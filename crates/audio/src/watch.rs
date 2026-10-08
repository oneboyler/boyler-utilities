//! The always-on listener for "Keep my devices" and "New apps volume" (menu closed). One thread that SLEEPS on a
//! channel: Windows' callbacks (`IMMNotificationClient` for device plug / unplug / default changes,
//! `IAudioSessionNotification` for new sessions) only post a message — Microsoft: callbacks must not block and must not
//! (un)register — and the thread wakes, lets the [`Engine`] decide, and acts. No timer, no polling.
//!
//! Session notifications are registered on every output device that is on (new sessions are created on the device an
//! app plays to), and re-synced when devices come and go. Only while New apps volume is on.

#![allow(non_snake_case)]

use crate::engine::{Did, Engine, Event};
use crate::keep::DeviceEvent;
use crate::model::*;
use crate::newvol::{filetime_now, NewAppsVolume};
use crate::os::AudioOs;
use crate::real::{flow_of, role_of, session_info, Com, RealOs};
use crate::Result;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;
use windows::core::{implement, Interface, Ref, PCWSTR};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::CLSCTX_ALL;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WatchConfig {
    pub keep_devices: bool,
    pub new_apps: bool,
    pub new_apps_volume: f32,
    /// Decide everything but change nothing (the OS layer refuses) — proof / measuring runs.
    pub read_only: bool,
}

impl Default for WatchConfig {
    /// The drawing's defaults (A_012_01): Keep my devices on; New apps volume on, 50 %.
    fn default() -> Self {
        WatchConfig { keep_devices: true, new_apps: crate::newvol::DEFAULT_ON, new_apps_volume: crate::newvol::DEFAULT_VOLUME, read_only: false }
    }
}

/// A session control handed from Windows' callback thread to the watcher thread. Both are in the multithreaded
/// apartment, where a COM pointer may be used from any thread.
struct Ctl(IAudioSessionControl);
// SAFETY: see above — the watcher thread joins the MTA before it receives anything.
unsafe impl Send for Ctl {}

enum Msg {
    Added(String),
    State(String, bool),
    Removed(String),
    Default(Flow, Role, Option<String>),
    Session(Ctl),
    Keep(bool),
    NewApps(bool, f32),
    Quit,
}

#[implement(IMMNotificationClient)]
struct DeviceClient {
    tx: Sender<Msg>,
}

impl DeviceClient {
    fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
    }
}

fn s(p: &PCWSTR) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { p.to_string().unwrap_or_default() }
    }
}

impl IMMNotificationClient_Impl for DeviceClient_Impl {
    fn OnDeviceStateChanged(&self, id: &PCWSTR, state: DEVICE_STATE) -> windows::core::Result<()> {
        self.send(Msg::State(s(id), state == DEVICE_STATE_ACTIVE));
        Ok(())
    }
    fn OnDeviceAdded(&self, id: &PCWSTR) -> windows::core::Result<()> {
        self.send(Msg::Added(s(id)));
        Ok(())
    }
    fn OnDeviceRemoved(&self, id: &PCWSTR) -> windows::core::Result<()> {
        self.send(Msg::Removed(s(id)));
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, id: &PCWSTR) -> windows::core::Result<()> {
        if let (Some(f), Some(r)) = (flow_of(flow), role_of(role)) {
            let id = s(id);
            self.send(Msg::Default(f, r, (!id.is_empty()).then_some(id)));
        }
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _id: &PCWSTR, _key: &PROPERTYKEY) -> windows::core::Result<()> {
        Ok(())
    }
}

#[implement(IAudioSessionNotification)]
struct SessionClient {
    tx: Sender<Msg>,
}

impl IAudioSessionNotification_Impl for SessionClient_Impl {
    fn OnSessionCreated(&self, newsession: Ref<IAudioSessionControl>) -> windows::core::Result<()> {
        if let Ok(c) = newsession.ok() {
            let _ = self.tx.send(Msg::Session(Ctl(c.clone())));
        }
        Ok(())
    }
}

/// What the watcher saw and did (newest last, at most 200 lines) — for the app's log and the proof runs.
pub type WatchLog = Arc<Mutex<Vec<String>>>;

pub struct Watcher {
    tx: Sender<Msg>,
    join: Option<JoinHandle<()>>,
    log: WatchLog,
}

impl Watcher {
    /// Starts the listener thread. Errors (no Core Audio) end up in the log; the thread then ends.
    pub fn start(cfg: WatchConfig) -> Watcher {
        let (tx, rx) = channel::<Msg>();
        let log: WatchLog = Arc::new(Mutex::new(Vec::new()));
        let (tx2, log2) = (tx.clone(), log.clone());
        let join = std::thread::Builder::new()
            .name("bu-audio-watch".into())
            .spawn(move || {
                if let Err(e) = run(cfg, tx2, rx, &log2) {
                    push(&log2, format!("watcher stopped: {e}"));
                }
            })
            .map_err(|e| push(&log, format!("watcher thread did not start: {e}")))
            .ok();
        Watcher { tx, join, log }
    }

    pub fn set_keep_devices(&self, on: bool) {
        let _ = self.tx.send(Msg::Keep(on));
    }
    pub fn set_new_apps(&self, on: bool, volume: f32) {
        let _ = self.tx.send(Msg::NewApps(on, volume));
    }
    pub fn log(&self) -> Vec<String> {
        self.log.lock().map(|l| l.clone()).unwrap_or_default()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Quit);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn push(log: &WatchLog, line: String) {
    if let Ok(mut l) = log.lock() {
        l.push(line);
        let n = l.len();
        if n > 200 {
            l.drain(..n - 200);
        }
    }
}

fn run(cfg: WatchConfig, tx: Sender<Msg>, rx: Receiver<Msg>, log: &WatchLog) -> Result<()> {
    let _com = Com::mta();
    let os = if cfg.read_only { RealOs::read_only()? } else { RealOs::new()? };
    let en: IMMDeviceEnumerator = os.enumerator().clone();
    let mut eng = Engine::new(os, cfg.keep_devices, NewAppsVolume::new(cfg.new_apps, cfg.new_apps_volume, filetime_now()))?;
    let client: IMMNotificationClient = DeviceClient { tx: tx.clone() }.into();
    unsafe { en.RegisterEndpointNotificationCallback(&client) }.map_err(|e| crate::real::os_err("RegisterEndpointNotificationCallback", e))?;
    let notif: IAudioSessionNotification = SessionClient { tx }.into();
    let mut regs: HashMap<String, IAudioSessionManager2> = HashMap::new();
    if eng.new_apps.is_on() {
        sync_sessions(&mut eng, &notif, &mut regs, log);
    }
    push(log, format!("watching: keep devices {}, new apps volume {} {:.0} %", eng.keep.is_on(), eng.new_apps.is_on(), eng.new_apps.volume() * 100.0));
    let t0 = Instant::now();
    // blocks until Windows (or the app) has something — the thread sleeps in between
    while let Ok(m) = rx.recv() {
        let now = t0.elapsed();
        let mut resync = false;
        let ev = match m {
            Msg::Quit => break,
            Msg::Keep(on) => {
                eng.keep.set_on(on);
                None
            }
            Msg::NewApps(on, v) => {
                eng.new_apps.set_on(on, filetime_now());
                eng.new_apps.set_volume(v);
                resync = true;
                None
            }
            Msg::Added(id) | Msg::State(id, true) => {
                resync = true;
                match device_flow(&en, &id) {
                    Some((f, true)) => Some(Event::Device(DeviceEvent::Present { id, flow: f })),
                    _ => None,
                }
            }
            Msg::State(id, false) | Msg::Removed(id) => {
                resync = true;
                Some(Event::Device(DeviceEvent::Gone { id }))
            }
            Msg::Default(flow, role, id) => Some(Event::Device(DeviceEvent::DefaultChanged { flow, role, id })),
            Msg::Session(Ctl(c)) => match session_info(&c) {
                Some((info, vol, meter)) => {
                    eng.os.remember_session(&info.key, vol, meter);
                    Some(Event::SessionCreated(info))
                }
                None => None,
            },
        };
        if let Some(ev) = ev {
            let line = describe(&ev);
            let new_key = match &ev {
                Event::SessionCreated(s) => Some(s.key.clone()),
                _ => None,
            };
            let did = eng.handle(ev, now);
            if let Some(k) = new_key {
                eng.os.forget_session(&k);
            }
            push(log, format!("{:>9.3}s {line}{}", now.as_secs_f64(), did.iter().map(|d| format!(" -> {}", describe_did(d))).collect::<String>()));
        }
        if resync {
            if eng.new_apps.is_on() {
                sync_sessions(&mut eng, &notif, &mut regs, log);
            } else {
                for (_, m) in regs.drain() {
                    let _ = unsafe { m.UnregisterSessionNotification(&notif) };
                }
            }
        }
    }
    for (_, m) in regs.drain() {
        let _ = unsafe { m.UnregisterSessionNotification(&notif) };
    }
    let _ = unsafe { en.UnregisterEndpointNotificationCallback(&client) };
    Ok(())
}

/// The device's flow and whether it is on.
fn device_flow(en: &IMMDeviceEnumerator, id: &str) -> Option<(Flow, bool)> {
    let w = crate::real::wide(id);
    unsafe {
        let d = en.GetDevice(PCWSTR(w.as_ptr())).ok()?;
        let ep: IMMEndpoint = d.cast().ok()?;
        let f = flow_of(ep.GetDataFlow().ok()?)?;
        Some((f, d.GetState().ok()? == DEVICE_STATE_ACTIVE))
    }
}

/// Registers for new sessions on every output device that is on; drops devices that left.
fn sync_sessions(eng: &mut Engine<RealOs>, notif: &IAudioSessionNotification, regs: &mut HashMap<String, IAudioSessionManager2>, log: &WatchLog) {
    let on: Vec<String> = eng
        .os
        .devices(Flow::Output)
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d.state == DeviceState::On)
        .map(|d| d.id)
        .collect();
    regs.retain(|id, m| {
        let keep = on.contains(id);
        if !keep {
            let _ = unsafe { m.UnregisterSessionNotification(notif) };
        }
        keep
    });
    let en = eng.os.enumerator().clone();
    for id in on {
        if regs.contains_key(&id) {
            continue;
        }
        let w = crate::real::wide(&id);
        let r = unsafe {
            en.GetDevice(PCWSTR(w.as_ptr())).and_then(|d| d.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None)).and_then(|m| {
                m.RegisterSessionNotification(notif)?;
                // asked once after registering: developers report OnSessionCreated stays silent until the session list was
                // read (delphipraxis.net TAudioVolume thread) — not in Microsoft's docs; unverified, harmless
                let _ = m.GetSessionEnumerator()?;
                Ok(m)
            })
        };
        match r {
            Ok(m) => {
                regs.insert(id, m);
            }
            Err(e) => push(log, format!("session notifications on {id}: {e}")),
        }
    }
}

fn describe(ev: &Event) -> String {
    match ev {
        Event::Device(DeviceEvent::Present { flow, .. }) => format!("device plugged in / on ({flow:?})"),
        Event::Device(DeviceEvent::Gone { .. }) => "device unplugged / off".into(),
        Event::Device(DeviceEvent::DefaultChanged { flow, role, id }) => {
            format!("default {flow:?} {role:?} changed{}", if id.is_none() { " to none" } else { "" })
        }
        Event::SessionCreated(s) => format!(
            "new session pid {} ({})",
            s.pid,
            std::path::Path::new(&s.exe_path).file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default()
        ),
    }
}

fn describe_did(d: &Did) -> String {
    match d {
        Did::PutBack { role, .. } => format!("put the chosen device back ({role:?})"),
        Did::NewAppVolume { pid, volume, .. } => format!("pid {pid} set to {:.0} %", volume * 100.0),
        Did::Failed(e) => format!("not done: {e}"),
    }
}
