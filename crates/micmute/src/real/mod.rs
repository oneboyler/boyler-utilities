//! The REAL Windows implementations of the OS traits.
//!
//! - [`RealMicOs`] — Core Audio. `RealMicOs::read_only()` reads and watches but refuses every mute change.
//! - [`RealSoundOut`] — `PlaySoundW(SND_MEMORY | SND_ASYNC)` on the default output device.

mod audio;
mod sound;

pub use audio::{RealMicOs, OUR_CONTEXT};
pub use sound::RealSoundOut;

use crate::MicError;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadMode {
    ReadWrite,
    ReadOnly,
}

/// COM on this thread for one call: balanced with `CoUninitialize` only when this call started it (S_OK / S_FALSE);
/// `RPC_E_CHANGED_MODE` (the thread already runs COM in another mode) is left alone.
pub(crate) struct Com(bool);

impl Com {
    pub(crate) fn init() -> Com {
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

pub(crate) fn com_err(context: &'static str) -> impl Fn(windows::core::Error) -> MicError {
    move |e| MicError::Os { context: context.to_string(), code: e.code().0 as u32 }
}
