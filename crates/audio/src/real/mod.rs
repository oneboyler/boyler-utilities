//! The REAL Windows implementation of [`AudioOs`]: Core Audio (`IMMDeviceEnumerator`, `IAudioEndpointVolume`,
//! `IAudioMeterInformation`, `IAudioSessionManager2`, `ISimpleAudioVolume`) + the undocumented `IPolicyConfig`.
//!
//! * [`RealOs::new`] — everything real (the app).
//! * [`RealOs::read_only`] — real reads; EVERY change refused with `AudioError::ReadOnly` (examples / proof runs).
//!
//! COM: the constructor joins the calling thread to the multithreaded apartment (Core Audio's session notifications
//! need MTA — Microsoft's IAudioSessionNotification page) and leaves it when dropped.

mod appinfo;
pub(crate) mod policy;

use crate::model::*;
use crate::os::AudioOs;
use crate::{AudioError, Result};
use policy::{set_default_endpoint, set_endpoint_visibility, IPolicyConfig, CLSID_POLICY_CONFIG_CLIENT};
use std::collections::HashMap;
use windows::core::{Interface, PCWSTR, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, E_ACCESSDENIED, FILETIME, S_OK};
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::StructuredStorage::{PropVariantToStringAlloc, PropVariantToUInt32};
use windows::Win32::System::Com::*;
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// HRESULT_FROM_WIN32(ERROR_NOT_FOUND): no default device for that role.
const E_NOTFOUND: i32 = 0x8007_0490u32 as i32;

pub(crate) fn os_err(context: &str, e: windows::core::Error) -> AudioError {
    if e.code() == E_ACCESSDENIED {
        AudioError::NeedsAdmin(context.into())
    } else {
        AudioError::Os { context: context.into(), code: e.code().0 as u32 }
    }
}

pub(crate) fn take_pwstr(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: Core Audio returns CoTaskMemAlloc'ed, NUL-terminated strings; freed right after copying.
    let s = unsafe { p.to_string().unwrap_or_default() };
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    s
}

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub(crate) fn edf(f: Flow) -> EDataFlow {
    match f {
        Flow::Output => eRender,
        Flow::Input => eCapture,
    }
}

pub(crate) fn erole(r: Role) -> ERole {
    match r {
        Role::Console => eConsole,
        Role::Multimedia => eMultimedia,
        Role::Communications => eCommunications,
    }
}

pub(crate) fn role_of(r: ERole) -> Option<Role> {
    match r {
        x if x == eConsole => Some(Role::Console),
        x if x == eMultimedia => Some(Role::Multimedia),
        x if x == eCommunications => Some(Role::Communications),
        _ => None,
    }
}

pub(crate) fn flow_of(f: EDataFlow) -> Option<Flow> {
    match f {
        x if x == eRender => Some(Flow::Output),
        x if x == eCapture => Some(Flow::Input),
        _ => None,
    }
}

/// Joins this thread to COM's multithreaded apartment; leaves on drop (only if the join worked).
pub(crate) struct Com(bool);
impl Com {
    pub(crate) fn mta() -> Com {
        // SAFETY: plain COM initialisation; balanced in Drop.
        Com(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

struct Endpoint {
    vol: Option<IAudioEndpointVolume>,
    meter: Option<IAudioMeterInformation>,
}

struct Sess {
    vol: ISimpleAudioVolume,
    meter: Option<IAudioMeterInformation>,
    /// The output device it was listed on ("" = handed over by the watcher).
    device: String,
}

pub struct RealOs {
    en: IMMDeviceEnumerator,
    read_only: bool,
    eps: HashMap<String, Endpoint>,
    mgrs: HashMap<String, IAudioSessionManager2>,
    sess: HashMap<String, Sess>,
    /// last field: COM is left after every interface above is released
    _com: Com,
}

impl RealOs {
    /// Everything real — the app uses this.
    pub fn new() -> Result<Self> {
        let com = Com::mta();
        // SAFETY: COM is initialised on this thread.
        let en: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.map_err(|e| os_err("MMDeviceEnumerator", e))?;
        Ok(RealOs { en, read_only: false, eps: HashMap::new(), mgrs: HashMap::new(), sess: HashMap::new(), _com: com })
    }

    /// Real reads; every change refused (`AudioError::ReadOnly`).
    pub fn read_only() -> Result<Self> {
        Ok(RealOs { read_only: true, ..Self::new()? })
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn refuse(&self, what: &str) -> Result<()> {
        if self.read_only {
            Err(AudioError::ReadOnly(what.into()))
        } else {
            Ok(())
        }
    }

    pub(crate) fn enumerator(&self) -> &IMMDeviceEnumerator {
        &self.en
    }

    fn device(&self, id: &str) -> Result<IMMDevice> {
        let w = wide(id);
        unsafe { self.en.GetDevice(PCWSTR(w.as_ptr())) }.map_err(|_| AudioError::NotFound(id.into()))
    }

    fn endpoint(&mut self, id: &str) -> Result<&Endpoint> {
        if !self.eps.contains_key(id) {
            let d = self.device(id)?;
            let ep = unsafe {
                Endpoint {
                    vol: d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok(),
                    meter: d.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None).ok(),
                }
            };
            self.eps.insert(id.to_string(), ep);
        }
        self.eps.get(id).ok_or_else(|| AudioError::NotFound(id.into()))
    }

    fn policy(&self) -> Result<IPolicyConfig> {
        unsafe { CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) }.map_err(|e| os_err("PolicyConfig", e))
    }

    /// The watcher hands over a brand-new session's volume control (the session enumerator may not know it yet —
    /// Microsoft's IAudioSessionNotification page).
    pub(crate) fn remember_session(&mut self, key: &str, vol: ISimpleAudioVolume, meter: Option<IAudioMeterInformation>) {
        self.sess.insert(key.to_string(), Sess { vol, meter, device: String::new() });
    }

    /// The watcher is done with a new session: drop its handles (the always-on part keeps nothing per session).
    pub(crate) fn forget_session(&mut self, key: &str) {
        self.sess.remove(key);
    }

    /// A cached device object failed (unplugged / re-plugged → AUDCLNT_E_DEVICE_INVALIDATED): forget it, so the next
    /// call opens the device again.
    fn evict<T>(&mut self, id: &str, r: Result<T>) -> Result<T> {
        if r.is_err() {
            self.eps.remove(id);
            self.mgrs.remove(id);
        }
        r
    }
}

/// Exe path + start time (FILETIME) of a process; ("", 0) when it can't be opened (elevated / protected / gone).
pub(crate) fn process_info(pid: u32) -> (String, u64) {
    if pid == 0 {
        return (String::new(), 0);
    }
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return (String::new(), 0) };
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        let path = if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok() {
            String::from_utf16_lossy(&buf[..n as usize])
        } else {
            String::new()
        };
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let start = if GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u).is_ok() {
            ((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64
        } else {
            0
        };
        let _ = CloseHandle(h);
        (path, start)
    }
}

/// Everything about one session control.
pub(crate) fn session_info(c: &IAudioSessionControl) -> Option<(SessionInfo, ISimpleAudioVolume, Option<IAudioMeterInformation>)> {
    unsafe {
        let c2: IAudioSessionControl2 = c.cast().ok()?;
        let vol: ISimpleAudioVolume = c.cast().ok()?;
        let meter = c.cast::<IAudioMeterInformation>().ok();
        let state = match c.GetState().unwrap_or(AudioSessionStateInactive) {
            s if s == AudioSessionStateActive => SessionState::Active,
            s if s == AudioSessionStateExpired => SessionState::Expired,
            _ => SessionState::Inactive,
        };
        let pid = c2.GetProcessId().unwrap_or(0);
        let (exe_path, process_started) = process_info(pid);
        let info = SessionInfo {
            key: take_pwstr(c2.GetSessionInstanceIdentifier().unwrap_or(PWSTR::null())),
            pid,
            process_started,
            exe_path,
            display_name: take_pwstr(c.GetDisplayName().unwrap_or(PWSTR::null())),
            system: c2.IsSystemSoundsSession() == S_OK,
            state,
            volume: vol.GetMasterVolume().unwrap_or(1.0),
            muted: vol.GetMute().map(|b| b.as_bool()).unwrap_or(false),
        };
        Some((info, vol, meter))
    }
}

impl AudioOs for RealOs {
    fn devices(&mut self, flow: Flow) -> Result<Vec<Device>> {
        let mut out = Vec::new();
        unsafe {
            let col = self
                .en
                .EnumAudioEndpoints(edf(flow), DEVICE_STATE(DEVICE_STATE_ACTIVE.0 | DEVICE_STATE_DISABLED.0 | DEVICE_STATE_UNPLUGGED.0))
                .map_err(|e| os_err("EnumAudioEndpoints", e))?;
            let n = col.GetCount().unwrap_or(0);
            for i in 0..n {
                let Ok(d) = col.Item(i) else { continue };
                let id = take_pwstr(d.GetId().unwrap_or(PWSTR::null()));
                let state = match d.GetState() {
                    Ok(s) if s == DEVICE_STATE_ACTIVE => DeviceState::On,
                    Ok(s) if s == DEVICE_STATE_DISABLED => DeviceState::Off,
                    _ => DeviceState::Unplugged,
                };
                let (mut name, mut form) = (String::new(), 0u32);
                if let Ok(ps) = d.OpenPropertyStore(STGM_READ) {
                    if let Ok(v) = ps.GetValue(&PKEY_Device_FriendlyName) {
                        name = PropVariantToStringAlloc(&v).map(take_pwstr).unwrap_or_default();
                    }
                    if let Ok(v) = ps.GetValue(&PKEY_AudioEndpoint_FormFactor) {
                        form = PropVariantToUInt32(&v).unwrap_or(0);
                    }
                }
                let kind = DeviceKind::classify(&name, form, flow);
                out.push(Device { id, name, kind, flow, state });
            }
        }
        Ok(out)
    }

    fn defaults(&mut self, flow: Flow) -> Result<Defaults> {
        let mut d = Defaults::default();
        for r in ROLES {
            match unsafe { self.en.GetDefaultAudioEndpoint(edf(flow), erole(r)) } {
                Ok(dev) => d.set(r, Some(take_pwstr(unsafe { dev.GetId() }.unwrap_or(PWSTR::null())))),
                Err(e) if e.code().0 == E_NOTFOUND => d.set(r, None),
                Err(e) => return Err(os_err("GetDefaultAudioEndpoint", e)),
            }
        }
        Ok(d)
    }

    fn set_default(&mut self, id: &str, role: Role) -> Result<()> {
        self.refuse("set default device")?;
        let pc = self.policy()?;
        let w = wide(id);
        set_default_endpoint(&pc, PCWSTR(w.as_ptr()), erole(role).0).ok().map_err(|e| os_err("SetDefaultEndpoint", e))
    }

    fn volume(&mut self, id: &str) -> Result<VolumeMute> {
        let v = self.endpoint(id)?.vol.clone().ok_or_else(|| AudioError::Unavailable(format!("volume of {id}")))?;
        let r = unsafe {
            v.GetMasterVolumeLevelScalar()
                .and_then(|volume| Ok(VolumeMute { volume, muted: v.GetMute()?.as_bool() }))
                .map_err(|e| os_err("GetMasterVolumeLevelScalar / GetMute", e))
        };
        self.evict(id, r)
    }

    fn set_volume(&mut self, id: &str, volume: f32) -> Result<()> {
        self.refuse("set device volume")?;
        let v = self.endpoint(id)?.vol.clone().ok_or_else(|| AudioError::Unavailable(format!("volume of {id}")))?;
        let r = unsafe { v.SetMasterVolumeLevelScalar(volume.clamp(0.0, 1.0), std::ptr::null()) }.map_err(|e| os_err("SetMasterVolumeLevelScalar", e));
        self.evict(id, r)
    }

    fn set_mute(&mut self, id: &str, muted: bool) -> Result<()> {
        self.refuse("set device mute")?;
        let v = self.endpoint(id)?.vol.clone().ok_or_else(|| AudioError::Unavailable(format!("volume of {id}")))?;
        let r = unsafe { v.SetMute(muted, std::ptr::null()) }.map_err(|e| os_err("SetMute", e));
        self.evict(id, r)
    }

    fn peak(&mut self, id: &str) -> Result<f32> {
        let m = self.endpoint(id)?.meter.clone().ok_or_else(|| AudioError::Unavailable(format!("level of {id}")))?;
        let r = unsafe { m.GetPeakValue() }.map_err(|e| os_err("GetPeakValue", e));
        self.evict(id, r)
    }

    fn set_enabled(&mut self, id: &str, on: bool) -> Result<()> {
        self.refuse("switch device on/off")?;
        let pc = self.policy()?;
        let w = wide(id);
        self.eps.remove(id);
        self.mgrs.remove(id);
        set_endpoint_visibility(&pc, PCWSTR(w.as_ptr()), on as i32).ok().map_err(|e| os_err("SetEndpointVisibility", e))
    }

    fn sessions(&mut self, device_id: &str) -> Result<Vec<SessionInfo>> {
        if !self.mgrs.contains_key(device_id) {
            let d = self.device(device_id)?;
            let m: IAudioSessionManager2 =
                unsafe { d.Activate(CLSCTX_ALL, None) }.map_err(|e| os_err("IAudioSessionManager2", e))?;
            self.mgrs.insert(device_id.to_string(), m);
        }
        let mgr = self.mgrs[device_id].clone();
        let r = unsafe { mgr.GetSessionEnumerator() }.map_err(|e| os_err("GetSessionEnumerator", e));
        let en = self.evict(device_id, r)?;
        let n = unsafe { en.GetCount() }.unwrap_or(0);
        // the list is read fresh: drop this device's old session handles (closed apps' handles don't pile up)
        self.sess.retain(|_, s| s.device != device_id);
        let mut out: Vec<SessionInfo> = Vec::new();
        for i in 0..n {
            let Ok(c) = (unsafe { en.GetSession(i) }) else { continue };
            let Some((info, vol, meter)) = session_info(&c) else { continue };
            if info.state == SessionState::Expired || out.iter().any(|s| s.key == info.key) {
                continue;
            }
            self.sess.insert(info.key.clone(), Sess { vol, meter, device: device_id.to_string() });
            out.push(info);
        }
        Ok(out)
    }

    fn set_session_volume(&mut self, key: &str, volume: f32) -> Result<()> {
        self.refuse("set app volume")?;
        let s = self.sess.get(key).ok_or_else(|| AudioError::NotFound(key.into()))?;
        unsafe { s.vol.SetMasterVolume(volume.clamp(0.0, 1.0), std::ptr::null()) }.map_err(|e| os_err("SetMasterVolume", e))
    }

    fn set_session_mute(&mut self, key: &str, muted: bool) -> Result<()> {
        self.refuse("set app mute")?;
        let s = self.sess.get(key).ok_or_else(|| AudioError::NotFound(key.into()))?;
        unsafe { s.vol.SetMute(muted, std::ptr::null()) }.map_err(|e| os_err("SetMute", e))
    }

    fn session_peak(&mut self, key: &str) -> Result<f32> {
        let s = self.sess.get(key).ok_or_else(|| AudioError::NotFound(key.into()))?;
        let m = s.meter.as_ref().ok_or_else(|| AudioError::Unavailable(format!("level of {key}")))?;
        unsafe { m.GetPeakValue() }.map_err(|e| os_err("GetPeakValue", e))
    }

    fn app_look(&mut self, s: &SessionInfo) -> AppLook {
        appinfo::look(s)
    }
}
