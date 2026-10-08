//! `bu-micmute` — the Mic mute card's features (DESIGN.md §3.5), no UI, no keys.
//!
//! * [`MicMute`] — mute / unmute / toggle the chosen mic (Windows' default or a picked one) at the endpoint
//!   (`IAudioEndpointVolume::SetMute`), its state, a change event (another app muting it), undo, and "switching the card
//!   off silently unmutes". The app layer maps keys to these actions (one key = `toggle`, separate keys = `mute` / `unmute`).
//! * [`sound`] — the mute / unmute sounds (made in code as short WAVs) and their settings.
//! * The on-screen Live / Muted icon is UI (a later order): it reads [`MicState`].
//!
//! Every Windows call goes through the OS traits in [`os`]: real ones in `real` (Windows) and fakes in [`fake`] (tests).

mod error;
pub mod fake;
pub mod os;
#[cfg(windows)]
pub mod real;
mod service;
pub mod sound;

pub use error::{MicError, Result};
pub use os::{MicDevice, MicEvent, MicOs, SoundOut};
pub use service::{ChangeFn, MicChoice, MicMute, MicState};
pub use sound::{Sound, SoundSettings};
