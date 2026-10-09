//! The one WASAPI shared-mode output stream the key sounds play through (Windows only).
//!
//! Low latency: `IAudioClient3::InitializeSharedAudioStream` at the engine's SMALLEST period (about 3 ms at 48 kHz on most
//! devices, 10 ms on the others), event driven. A device whose driver can't do that (some Bluetooth / old drivers) gets the
//! plain shared-mode stream at its default period (10 ms, event driven): a little later, the same sounds. The stream is
//! opened on the default render device in the engine's own mix format (float, the device's rate and channels), so Windows does
//! no format conversion. It is opened when a sound has to play and closed again a few seconds after the last one ends (the
//! engine decides): between presses nothing runs.

use crate::mixer::Mixer;
use std::ffi::c_void;
use windows::core::Interface;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};

pub struct Stream {
    client: IAudioClient,
    render: IAudioRenderClient,
    pub rate: u32,
    pub channels: usize,
    pub buffer_frames: u32,
    pub period_frames: u32,
    /// What Windows says the stream adds before a sample is heard (ms), plus one period.
    pub latency_ms: f32,
    /// Opened with the low-latency (IAudioClient3) path.
    pub low_latency: bool,
}

fn os(ctx: &str, e: windows::core::Error) -> String {
    format!("{ctx}: {} (0x{:08X})", e.message().trim(), e.code().0 as u32)
}

/// (channels, rate) of the mix format when it is 32-bit float (plain or "extensible"), which the shared engine always is.
unsafe fn check_format(fmt: *mut WAVEFORMATEX) -> Result<(usize, u32), String> {
    // SAFETY: `fmt` is the mix format Windows returned.
    let (tag, channels, rate, bits) = unsafe { ((*fmt).wFormatTag, (*fmt).nChannels as usize, (*fmt).nSamplesPerSec, (*fmt).wBitsPerSample) };
    if bits != 32 || !(tag == 3 || tag == 0xFFFE) || channels == 0 {
        return Err(format!("the output's mix format isn't 32-bit float (tag {tag}, {bits} bit, {channels} ch)"));
    }
    Ok((channels, rate))
}

impl Stream {
    /// Opens the default output device's stream. `event` is signalled by Windows once per period. `mute` mutes this
    /// stream's own audio session (tests: everything runs through the real device but nothing is heard).
    pub fn open(event: HANDLE, mute: bool) -> Result<Stream, String> {
        // SAFETY: Core Audio calls on this thread (COM is initialised by the caller); every pointer is Windows' own.
        unsafe {
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| os("MMDeviceEnumerator", e))?;
            let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).map_err(|e| os("no default output device", e))?;
            match Self::open_low_latency(&dev, event, mute) {
                Ok(s) => Ok(s),
                Err(first) => Self::open_standard(&dev, event, mute).map_err(|e| format!("{e} (the low-latency stream said: {first})")),
            }
        }
    }

    unsafe fn open_low_latency(dev: &IMMDevice, event: HANDLE, mute: bool) -> Result<Stream, String> {
        // SAFETY: see `open`.
        unsafe {
            let client: IAudioClient3 = dev.Activate(CLSCTX_ALL, None).map_err(|e| os("IAudioClient3", e))?;
            let fmt = client.GetMixFormat().map_err(|e| os("GetMixFormat", e))?;
            let r = (|| {
                let (channels, rate) = check_format(fmt)?;
                let (mut def, mut fund, mut min, mut max) = (0u32, 0u32, 0u32, 0u32);
                client.GetSharedModeEnginePeriod(fmt, &mut def, &mut fund, &mut min, &mut max).map_err(|e| os("GetSharedModeEnginePeriod", e))?;
                client.InitializeSharedAudioStream(AUDCLNT_STREAMFLAGS_EVENTCALLBACK, min, fmt, None).map_err(|e| os("InitializeSharedAudioStream", e))?;
                let base: IAudioClient = client.cast().map_err(|e| os("IAudioClient", e))?;
                Self::finish(base, event, mute, channels, rate, min, true)
            })();
            CoTaskMemFree(Some(fmt as *const c_void));
            r
        }
    }

    unsafe fn open_standard(dev: &IMMDevice, event: HANDLE, mute: bool) -> Result<Stream, String> {
        // SAFETY: see `open`.
        unsafe {
            let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).map_err(|e| os("IAudioClient", e))?;
            let fmt = client.GetMixFormat().map_err(|e| os("GetMixFormat", e))?;
            let r = (|| {
                let (channels, rate) = check_format(fmt)?;
                let (mut def, mut min) = (0i64, 0i64);
                client.GetDevicePeriod(Some(&mut def), Some(&mut min)).map_err(|e| os("GetDevicePeriod", e))?;
                client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, def, 0, fmt, None).map_err(|e| os("Initialize", e))?;
                let period = (def as f64 * f64::from(rate) / 1.0e7).round() as u32;
                Self::finish(client.clone(), event, mute, channels, rate, period, false)
            })();
            CoTaskMemFree(Some(fmt as *const c_void));
            r
        }
    }

    /// What both ways share once the stream is initialised.
    unsafe fn finish(client: IAudioClient, event: HANDLE, mute: bool, channels: usize, rate: u32, period_frames: u32, low_latency: bool) -> Result<Stream, String> {
        // SAFETY: see `open`.
        unsafe {
            client.SetEventHandle(event).map_err(|e| os("SetEventHandle", e))?;
            let buffer_frames = client.GetBufferSize().map_err(|e| os("GetBufferSize", e))?;
            let render: IAudioRenderClient = client.GetService().map_err(|e| os("IAudioRenderClient", e))?;
            let latency = client.GetStreamLatency().unwrap_or(0);
            if mute {
                if let Ok(v) = client.GetService::<ISimpleAudioVolume>() {
                    let _ = v.SetMute(true, std::ptr::null());
                }
            }
            Ok(Stream {
                client,
                render,
                rate,
                channels,
                buffer_frames,
                period_frames,
                latency_ms: (latency as f32 / 10_000.0) + (period_frames as f32 * 1000.0 / rate as f32),
                low_latency,
            })
        }
    }

    /// Writes every free frame of the buffer from the mixer (silence where nothing plays). Err = the device went away.
    pub fn fill(&self, mixer: &mut Mixer) -> Result<u32, String> {
        // SAFETY: the buffer Windows hands out holds `free` frames of `channels` floats; it is released right after.
        unsafe {
            let padding = self.client.GetCurrentPadding().map_err(|e| os("GetCurrentPadding", e))?;
            let free = self.buffer_frames.saturating_sub(padding);
            if free == 0 {
                return Ok(0);
            }
            let p = self.render.GetBuffer(free).map_err(|e| os("GetBuffer", e))?;
            let out = std::slice::from_raw_parts_mut(p as *mut f32, free as usize * self.channels);
            mixer.render(out, self.channels);
            self.render.ReleaseBuffer(free, 0).map_err(|e| os("ReleaseBuffer", e))?;
            Ok(free)
        }
    }

    pub fn start(&self) -> Result<(), String> {
        // SAFETY: a plain call on our initialised client.
        unsafe { self.client.Start() }.map_err(|e| os("Start", e))
    }

    pub fn stop(&self) {
        // SAFETY: plain calls on our client.
        unsafe {
            let _ = self.client.Stop();
            let _ = self.client.Reset();
        }
    }
}
