//! The real speaker for the mute sounds: `PlaySoundW` from memory, asynchronously, on Windows' default output device.

use crate::os::SoundOut;
use crate::{MicError, Result};
use std::sync::Mutex;
use windows::core::PCWSTR;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

/// Plays one WAV at a time. The bytes of the sound that is playing are kept here: with `SND_MEMORY | SND_ASYNC`
/// Windows reads the buffer while it plays, so it must stay alive until the next sound replaces it (or `stop`).
#[derive(Default)]
pub struct RealSoundOut {
    playing: Mutex<Option<Vec<u8>>>,
}

impl RealSoundOut {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SoundOut for RealSoundOut {
    fn play_wav(&self, wav: Vec<u8>) -> Result<()> {
        let mut slot = self.playing.lock().unwrap_or_else(|e| e.into_inner());
        // a new PlaySound call stops the sound that is playing first; only then is the old buffer dropped
        let ok = unsafe { PlaySoundW(PCWSTR(wav.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT) };
        if ok.as_bool() {
            *slot = Some(wav);
            Ok(())
        } else {
            Err(MicError::Os { context: "PlaySoundW".into(), code: 0 })
        }
    }

    fn stop(&self) {
        let mut slot = self.playing.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            let _ = PlaySoundW(PCWSTR::null(), None, Default::default());
        }
        *slot = None;
    }
}

impl Drop for RealSoundOut {
    fn drop(&mut self) {
        self.stop();
    }
}
