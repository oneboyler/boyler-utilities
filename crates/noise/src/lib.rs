//! `bu-noise` - the Noise tab's engine (Order 062). No UI.
//!
//! (Order 080: and your own mix of Tone / Rumble / Waves, [`Mix`], made the same way and changed live.)
//!
//! Six noises that we make ourselves (no recordings, no licences): white, pink, brown, dark brown, grey, blue ([`Kind`]).
//! * [`synth`]: each is made ONCE, when first picked, as one loop of about 22 s (stereo, 16-bit, 4.2 MB at 48 kHz) by an
//!   inverse FFT - so the loop repeats exactly and has no seam. Nothing is generated while it plays.
//! * [`player::Core`]: the pure player (fade in / out, volume, switching noise, the sleep timer), tested without a device.
//! * [`engine::Noise`]: one worker thread and one WASAPI shared stream that exist ONLY while the noise plays; the thread wakes
//!   about 14 times a second to top up a 200 ms lead. Stopped: no thread, no stream, no buffer.
//!
//! The player starts only when asked (`Noise::play`): it is off until the user presses Play.

pub mod fft;
pub mod kind;
pub mod player;
pub mod sound;
pub mod synth;

#[cfg(windows)]
pub mod engine;
#[cfg(windows)]
mod stream;

pub use crate::player::{gain_of, Core, DEFAULT_VOLUME, FADE_SECS, MIX_FADE_SECS, SLEEP_CHOICES, SWITCH_FADE_SECS};
pub use kind::Kind;
pub use sound::{Mix, Sound};

#[cfg(windows)]
pub use engine::{Noise, Status};
