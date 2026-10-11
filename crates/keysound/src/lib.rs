//! `bu-keysound` — the Keyboard tab's engine (Order 058). No UI.
//!
//! **Key sounds** (off by default; while off nothing listens): a soft sound on every key down / up.
//! * The key is used ONLY to pick the sound and is FORGOTTEN at once: it is never stored, logged, sent or kept in a buffer.
//!   The only listener is the process's one Raw Input owner (`bu-rawin`), which hands this crate a [`bu_rawin::SoundEvent`]
//!   — a class (Space / Enter / Backspace / other), up / down and (Order 090) the key's scan code, so a key's own sound and a
//!   pack made from one sound can play — straight from its own thread. No text, no time, no window ever reaches it.
//! * Order 090: every key / mouse button / controller button plays two LAYERS ([`layers`]): the pack's sound (switchable per
//!   button) and "your sound" on top (a file of your own, pitch + loudness). Packs can be made from one sound
//!   ([`layers::Made`]). A file someone else made is decoded in a helper process ([`safe`]): a broken one can't crash the app.
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
//! **Mouse clicks too** (Order 064, off by default, own volume 5 %): the same engine, the same one stream, the same game switch,
//! per-app rules and "ignore repeats" window. `bu-rawin` hands over a [`bu_rawin::MouseSoundEvent`] (Left / Right / Middle /
//! Side + up or down; no position, no device). Side buttons (X1 / X2) play the chosen pack's own key sound; left, right and the
//! wheel click play a click of our own synth ([`synth::ClickStyle`]: silent switch, optical, micro-switch, deep click, or a tick of
//! a satisfying pack) that suits the pack. While the switch is off the mouse is not even registered with Windows.
//!
//! **Controller too** (Order 081, off by default, at the keys' volume): `bu-rawin` hands over a [`bu_rawin::PadSoundEvent`] (button /
//! left trigger / right trigger + up or down; no button number, no device). A button (face, bumper, D-pad, stick click) plays the
//! chosen pack's key sound, a trigger the left / right click. While the switch is off no controller is registered with Windows.
//!
//! **Key remap** ([`remap`]): Windows' own Scancode Map (HKLM, one admin Yes, a restart), listed + "Reset all".
//!
//! The pure parts (synth, mixer, rules, the scancode map codec) are unit-tested; `engine` / `front` / `stream` /
//! `remap::real` are the thin Windows shells.

pub mod binds;
pub mod gallery;
pub mod import;
pub mod kind;
pub mod layers;
pub mod layout;
pub mod macros;
pub mod mixer;
pub mod remap;
pub mod rules;
pub mod safe;
pub mod synth;

#[cfg(windows)]
pub mod engine;
#[cfg(windows)]
mod front;
#[cfg(windows)]
mod mf;
#[cfg(windows)]
pub mod guard;
#[cfg(windows)]
pub mod send;
#[cfg(windows)]
mod stream;
#[cfg(windows)]
pub mod watch;

pub use kind::{kind_of, Kind, KINDS};
pub use layers::{Layer, Layers, Made, Release, Vary};
pub use rules::{choose, choose_mouse, choose_pad, click_from_key, click_key, gain, Pack, PlayOn, Rule, Settings, CLICK_STYLES, DEFAULT_VOLUME, MAX_REPEAT_MS};
pub use synth::{render_clicks, ClickSet, ClickStyle, PackId, SoundSet};

#[cfg(windows)]
pub use engine::{Dev, Hear, KeySounds, MadeSet, Status};
