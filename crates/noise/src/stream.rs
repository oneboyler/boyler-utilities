//! The one WASAPI shared-mode output stream the noise plays through (Windows only).
//!
//! Not the low-latency kind: noise doesn't care when it is heard, so the stream is the cheapest one - a 500 ms buffer in the
//! engine's own mix format (float, the device's rate and channels, so Windows converts nothing), NOT event driven: the worker
//! wakes about 14 times a second, tops the buffer up to 200 ms ahead and sleeps. No per-period wake-ups, no timer thread.

use std::ffi::c_void;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};

/// The buffer Windows keeps for us: 100-nanosecond units (500 ms).
const BUFFER_HNS: i64 = 5_000_000;

pub struct Stream {
    client: IAudioClient,
    render: IAudioRenderClient,
    pub rate: u32,
    pub channels: usize,
    pub buffer_frames: u32,
    /// The endpoint it plays on (to notice the default device changing).
    pub device_id: String,
}

fn os(ctx: &str, e: windows::core::Error) -> String {
    format!("{ctx}: {} (0x{:08X})", e.message().trim(), e.code().0 as u32)
}

/// The id of the current default output device (None = there is none).
pub fn default_id() -> Option<String> {
    // SAFETY: Core Audio calls on this thread (COM is initialised by the caller).
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        id_of(&dev)
    }
}

unsafe fn id_of(dev: &IMMDevice) -> Option<String> {
    // SAFETY: the id string is Windows' own allocation, freed after the copy.
    unsafe {
        let p = dev.GetId().ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const c_void));
        s
    }
}

impl Stream {
    /// Opens the default output device's stream (not started). `mute` mutes this stream's own audio session (tests: it goes
    /// through the real device and nothing is heard).
    pub fn open(mute: bool) -> Result<Stream, String> {
        // SAFETY: Core Audio calls on this thread; every pointer is Windows' own.
        unsafe {
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| os("MMDeviceEnumerator", e))?;
            let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).map_err(|e| os("no default output device", e))?;
            let device_id = id_of(&dev).unwrap_or_default();
            let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).map_err(|e| os("IAudioClient", e))?;
            let fmt = client.GetMixFormat().map_err(|e| os("GetMixFormat", e))?;
            let r = (|| {
                let (tag, channels, rate, bits) = ((*fmt).wFormatTag, (*fmt).nChannels as usize, (*fmt).nSamplesPerSec, (*fmt).wBitsPerSample);
                // an "extensible" format must say IEEE float too (its SubFormat GUID starts with 3); offset 24 = after the 18-byte
                // WAVEFORMATEX, 2 bytes of samples and the 4-byte channel mask
                let float = tag == 3 || (tag == 0xFFFE && (*fmt).cbSize >= 22 && std::ptr::read_unaligned((fmt as *const u8).add(24) as *const u32) == 3);
                if bits != 32 || !float || channels == 0 {
                    return Err(format!("the output's mix format isn't 32-bit float (tag {tag}, {bits} bit, {channels} ch)"));
                }
                client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, BUFFER_HNS, 0, fmt, None).map_err(|e| os("Initialize", e))?;
                let buffer_frames = client.GetBufferSize().map_err(|e| os("GetBufferSize", e))?;
                let render: IAudioRenderClient = client.GetService().map_err(|e| os("IAudioRenderClient", e))?;
                if mute {
                    if let Ok(v) = client.GetService::<ISimpleAudioVolume>() {
                        let _ = v.SetMute(true, std::ptr::null());
                    }
                }
                Ok(Stream { client: client.clone(), render, rate, channels, buffer_frames, device_id })
            })();
            CoTaskMemFree(Some(fmt as *const c_void));
            r
        }
    }

    /// Frames in the buffer that haven't been played yet.
    pub fn padding(&self) -> Result<u32, String> {
        // SAFETY: a plain call on our initialised client.
        unsafe { self.client.GetCurrentPadding() }.map_err(|e| os("GetCurrentPadding", e))
    }

    /// How many frames to write now so that `ahead` frames are waiting (0 = the buffer is full enough). A Windows call: never
    /// made while the page-facing lock is held. Err = the device went away.
    pub fn wanted(&self, ahead: u32) -> Result<u32, String> {
        Ok(ahead.min(self.buffer_frames).saturating_sub(self.padding()?))
    }

    /// Copies `buf` (interleaved, `channels` per frame) into the stream's buffer. Err = the device went away.
    pub fn write(&self, buf: &[f32]) -> Result<(), String> {
        let n = (buf.len() / self.channels) as u32;
        if n == 0 {
            return Ok(());
        }
        // SAFETY: the buffer Windows hands out holds `n` frames of `channels` floats; it is released right after.
        unsafe {
            let p = self.render.GetBuffer(n).map_err(|e| os("GetBuffer", e))?;
            std::ptr::copy_nonoverlapping(buf.as_ptr(), p as *mut f32, n as usize * self.channels);
            self.render.ReleaseBuffer(n, 0).map_err(|e| os("ReleaseBuffer", e))?;
        }
        Ok(())
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
