//! The real mic: Windows Core Audio (`IMMDeviceEnumerator`, `IAudioEndpointVolume`, `IMMNotificationClient`).

use super::{com_err, Com, ReadMode};
use crate::os::{EventSink, MicDevice, MicEvent, MicOs, Watch};
use crate::{MicError, Result};
use windows::core::{GUID, HSTRING, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    eCapture, eConsole, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl,
    MMDeviceEnumerator, AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{
    CoCreateInstance, CoDecrementMTAUsage, CoIncrementMTAUsage, CoTaskMemFree, CLSCTX_ALL, CO_MTA_USAGE_COOKIE, STGM_READ,
};

/// Our event context for `SetMute`: Windows hands it back in the change notification, so our own changes are told
/// apart from other apps' (fixed for this app).
pub const OUR_CONTEXT: GUID = GUID::from_u128(0x6b1f0c2e_5a3d_4c8b_9e71_b0b1e5a4d011);

/// The real Windows audio stack. [`RealMicOs::read_only`] reads and watches, every mute change refused.
pub struct RealMicOs {
    mode: ReadMode,
}

impl RealMicOs {
    pub fn new() -> Self {
        RealMicOs { mode: ReadMode::ReadWrite }
    }
    /// Real reads + watches; `set_muted` refused with `MicError::ReadOnly` (examples/show).
    pub fn read_only() -> Self {
        RealMicOs { mode: ReadMode::ReadOnly }
    }
}

impl Default for RealMicOs {
    fn default() -> Self {
        Self::new()
    }
}

fn enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(com_err("MMDeviceEnumerator")) }
}

unsafe fn device_id(d: &IMMDevice) -> Result<String> {
    let p = d.GetId().map_err(com_err("IMMDevice::GetId"))?;
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as *const _));
    Ok(s)
}

unsafe fn friendly_name(d: &IMMDevice) -> String {
    let read = || -> Option<String> {
        let store = d.OpenPropertyStore(STGM_READ).ok()?;
        let key: PROPERTYKEY = PKEY_Device_FriendlyName;
        let mut v = store.GetValue(&key).ok()?;
        let s = PropVariantToStringAlloc(&v).ok();
        let _ = PropVariantClear(&mut v);
        let s = s?;
        let out = s.to_string().ok();
        CoTaskMemFree(Some(s.0 as *const _));
        out
    };
    read().unwrap_or_else(|| "Microphone".into())
}

unsafe fn endpoint_volume(id: &str) -> Result<IAudioEndpointVolume> {
    let dev = enumerator()?.GetDevice(&HSTRING::from(id)).map_err(|e| no_mic(e, id))?;
    dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).map_err(|e| no_mic(e, id))
}

/// Gone / unplugged endpoint → `NoMic`, anything else → `Os`.
fn no_mic(e: windows::core::Error, id: &str) -> MicError {
    const E_NOTFOUND: u32 = 0x8007_0490; // HRESULT_FROM_WIN32(ERROR_NOT_FOUND)
    const DEVICE_INVALIDATED: u32 = 0x8889_0004; // AUDCLNT_E_DEVICE_INVALIDATED
    match e.code().0 as u32 {
        E_NOTFOUND | DEVICE_INVALIDATED => MicError::NoMic(format!("microphone {id} is gone")),
        c => MicError::Os { context: format!("open microphone {id}"), code: c },
    }
}

impl MicOs for RealMicOs {
    fn capture_devices(&self) -> Result<Vec<MicDevice>> {
        let _com = Com::init();
        unsafe {
            let en = enumerator()?;
            let default = en.GetDefaultAudioEndpoint(eCapture, eConsole).ok().and_then(|d| device_id(&d).ok());
            let coll = en.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE).map_err(com_err("EnumAudioEndpoints"))?;
            let n = coll.GetCount().map_err(com_err("IMMDeviceCollection::GetCount"))?;
            let mut out = Vec::with_capacity(n as usize);
            for i in 0..n {
                let d = coll.Item(i).map_err(com_err("IMMDeviceCollection::Item"))?;
                let id = device_id(&d)?;
                out.push(MicDevice { name: friendly_name(&d), is_default: default.as_ref() == Some(&id), id });
            }
            Ok(out)
        }
    }

    fn default_capture(&self) -> Result<Option<String>> {
        let _com = Com::init();
        unsafe {
            match enumerator()?.GetDefaultAudioEndpoint(eCapture, eConsole) {
                Ok(d) => device_id(&d).map(Some),
                Err(e) if e.code().0 as u32 == 0x8007_0490 => Ok(None), // E_NOTFOUND: no input device at all
                Err(e) => Err(com_err("GetDefaultAudioEndpoint")(e)),
            }
        }
    }

    fn is_muted(&self, id: &str) -> Result<bool> {
        let _com = Com::init();
        unsafe { Ok(endpoint_volume(id)?.GetMute().map_err(com_err("IAudioEndpointVolume::GetMute"))?.as_bool()) }
    }

    fn set_muted(&self, id: &str, muted: bool) -> Result<()> {
        if self.mode == ReadMode::ReadOnly {
            return Err(MicError::ReadOnly(format!("set_muted({muted})")));
        }
        let _com = Com::init();
        unsafe { endpoint_volume(id)?.SetMute(muted, &OUR_CONTEXT).map_err(com_err("IAudioEndpointVolume::SetMute")) }
    }

    fn watch_mute(&self, id: &str, sink: EventSink) -> Result<Box<dyn Watch>> {
        let _com = Com::init();
        unsafe {
            // keeps the multithreaded apartment alive for as long as the watch lives, whatever thread made it
            let mta = CoIncrementMTAUsage().map_err(com_err("CoIncrementMTAUsage"))?;
            let r = (|| {
                let vol = endpoint_volume(id)?;
                let cb: IAudioEndpointVolumeCallback = VolumeCallback { id: id.to_string(), sink }.into();
                vol.RegisterControlChangeNotify(&cb).map_err(com_err("RegisterControlChangeNotify"))?;
                Ok((vol, cb))
            })();
            match r {
                Ok((vol, cb)) => Ok(Box::new(MuteWatch { vol: Some(vol), cb: Some(cb), mta })),
                Err(e) => {
                    let _ = CoDecrementMTAUsage(mta);
                    Err(e)
                }
            }
        }
    }

    fn watch_devices(&self, sink: EventSink) -> Result<Box<dyn Watch>> {
        let _com = Com::init();
        unsafe {
            let mta = CoIncrementMTAUsage().map_err(com_err("CoIncrementMTAUsage"))?;
            let r = (|| {
                let en = enumerator()?;
                let cb: IMMNotificationClient = DeviceCallback { sink }.into();
                en.RegisterEndpointNotificationCallback(&cb).map_err(com_err("RegisterEndpointNotificationCallback"))?;
                Ok((en, cb))
            })();
            match r {
                Ok((en, cb)) => Ok(Box::new(DeviceWatch { en: Some(en), cb: Some(cb), mta })),
                Err(e) => {
                    let _ = CoDecrementMTAUsage(mta);
                    Err(e)
                }
            }
        }
    }
}

#[windows_core::implement(IAudioEndpointVolumeCallback)]
struct VolumeCallback {
    id: String,
    sink: EventSink,
}

impl IAudioEndpointVolumeCallback_Impl for VolumeCallback_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        // Windows' audio thread: read the data, hand it on, return quickly (no blocking, no re-registering here)
        if let Some(d) = unsafe { data.as_ref() } {
            (self.sink)(MicEvent::Mute {
                device_id: self.id.clone(),
                muted: d.bMuted.as_bool(),
                by_us: d.guidEventContext == OUR_CONTEXT,
            });
        }
        Ok(())
    }
}

#[windows_core::implement(IMMNotificationClient)]
struct DeviceCallback {
    sink: EventSink,
}

impl IMMNotificationClient_Impl for DeviceCallback_Impl {
    fn OnDeviceStateChanged(&self, _id: &PCWSTR, _state: DEVICE_STATE) -> windows::core::Result<()> {
        (self.sink)(MicEvent::DevicesChanged);
        Ok(())
    }
    fn OnDeviceAdded(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        (self.sink)(MicEvent::DevicesChanged);
        Ok(())
    }
    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        (self.sink)(MicEvent::DevicesChanged);
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, id: &PCWSTR) -> windows::core::Result<()> {
        if flow == eCapture && role == eConsole {
            let device_id = if id.is_null() { None } else { unsafe { id.to_string().ok() } };
            (self.sink)(MicEvent::DefaultChanged { device_id });
        }
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _id: &PCWSTR, _key: &PROPERTYKEY) -> windows::core::Result<()> {
        Ok(()) // fires often (levels, formats) — not interesting here
    }
}

struct MuteWatch {
    vol: Option<IAudioEndpointVolume>,
    cb: Option<IAudioEndpointVolumeCallback>,
    mta: CO_MTA_USAGE_COOKIE,
}
// SAFETY: both objects live in the multithreaded apartment (kept alive by `mta`); MTA interface pointers may be used
// from any MTA thread, and `Drop` only unregisters + releases.
unsafe impl Send for MuteWatch {}
impl Watch for MuteWatch {}
impl Drop for MuteWatch {
    fn drop(&mut self) {
        let _com = Com::init();
        unsafe {
            if let (Some(vol), Some(cb)) = (self.vol.take(), self.cb.take()) {
                let _ = vol.UnregisterControlChangeNotify(&cb);
            }
            let _ = CoDecrementMTAUsage(self.mta);
        }
    }
}

struct DeviceWatch {
    en: Option<IMMDeviceEnumerator>,
    cb: Option<IMMNotificationClient>,
    mta: CO_MTA_USAGE_COOKIE,
}
// SAFETY: as `MuteWatch`.
unsafe impl Send for DeviceWatch {}
impl Watch for DeviceWatch {}
impl Drop for DeviceWatch {
    fn drop(&mut self) {
        let _com = Com::init();
        unsafe {
            if let (Some(en), Some(cb)) = (self.en.take(), self.cb.take()) {
                let _ = en.UnregisterEndpointNotificationCallback(&cb);
            }
            let _ = CoDecrementMTAUsage(self.mta);
        }
    }
}
