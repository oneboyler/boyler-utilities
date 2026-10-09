//! `bu-keysound` — the Keyboard tab's engine (Order 058). No UI.
//!
//! **Key sounds** (off by default; while off nothing listens): a soft sound on every key down / up.
//! * The key is used ONLY to pick the sound (Space / Enter / Backspace / any other) and is FORGOTTEN at once: it is never
//!   stored, logged, sent or kept in a buffer. The only listener is the process's one Raw Input owner (`bu-rawin`), which
//!   hands this crate a [`bu_rawin::SoundEvent`] — a class and up / down, two bytes, no key — straight from its own
//!   thread. This crate never sees a key code at all (`tests::the_engine_never_sees_a_key`).
//! * The sounds are OUR OWN, made by a small synth ([`synth`]) when the stream opens — no recordings, no licences. Nine
//!   packs: Linear, Tactile, Clicky, Typewriter (clean keyboard) and Bubble, Glass tap, Water drop, Wood block, Marble
//!   (satisfying); each with key down, key up and Space / Enter / Backspace variants. A pack the user imports (a Mechvibes
//!   pack, [`import`]) is decoded into the same kind of set.
//! * Played from memory by [`mixer::Mixer`] (8 voices, a slight random pitch per press so it never sounds robotic) through ONE
//!   low-latency WASAPI shared stream ([`engine`]): a single render thread, no new thread per press. The stream runs only
//!   while sounds are playing and stops a few seconds after the last one: 0 CPU between presses.
//! * "Off while a game is in front": Windows' own "a full-screen app is in front" answer ([`front`]), read at the moment of a
//!   press (cached while the same window stays in front) — no hook, no timer, no thread.
//!
//! **Key remap** ([`remap`]): Windows' own Scancode Map (HKLM, one admin Yes, a restart), listed + "Reset all".
//!
//! The pure parts (synth, mixer, rules, the scancode map codec) are unit-tested; `engine` / `front` / `stream` /
//! `remap::real` are the thin Windows shells.

pub mod binds;
pub mod import;
pub mod kind;
pub mod layout;
pub mod macros;
pub mod mixer;
pub mod remap;
pub mod rules;
pub mod synth;

#[cfg(windows)]
pub mod engine;
#[cfg(windows)]
mod front;
#[cfg(windows)]
pub mod guard;
#[cfg(windows)]
pub mod send;
#[cfg(windows)]
mod stream;
#[cfg(windows)]
pub mod watch;

pub use kind::{kind_of, Kind, KINDS};
pub use rules::{choose, gain, Pack, Rule, Settings, DEFAULT_VOLUME};
pub use synth::{PackId, SoundSet};

#[cfg(windows)]
pub use engine::{KeySounds, Status};
