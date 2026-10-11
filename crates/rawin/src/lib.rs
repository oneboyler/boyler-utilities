//! `bu-rawin` — the process's ONE Raw Input owner (Order 048). No UI.
//!
//! Windows allows one target window per device kind (keyboard / mouse) per process, so two parts of the app that each
//! call `RegisterRawInputDevices` take the devices away from each other. Here one owner registers for everyone:
//! * one thread "bu-rawin" (started on first use) with a message-only window; it sleeps in `GetMessageW` (0 CPU) while
//!   nothing is registered. Every `RegisterRawInputDevices` call happens on that thread — the other threads ask it with
//!   a sent message (`SendMessageTimeoutW`, SMTO_BLOCK), so they get Windows' answer (and its error text) back at once;
//! * what is registered = what the clients need together ([`hub::Hub::wanted`]): keyboard / mouse for the keys manager
//!   ([`set_keys`]), both while the activity watcher waits for the first input after idle ([`notify_on_input`]).
//!   RIDEV_INPUTSINK (listen only, nothing blocked, no hook); RIDEV_REMOVE only when nobody needs that device any more;
//! * reading: WM_INPUT's own packet (`GetRawInputData`) + everything else already queued in batches
//!   (`GetRawInputBuffer`), so a fast mouse (8000 packets a second) costs one wake-up of this thread per batch;
//! * the keys client is woken (one `PostMessageW` per batch) only for packets that matter: key packets, and mouse
//!   packets with button / wheel flags. A pure mouse move never leaves this thread — it is only counted ([`stats`]);
//! * the key sounds (Order 058, `bu-keysound`) listen with [`set_key_sound`]: every key down / up is handed straight to its
//!   sink on this thread as a class-only [`SoundEvent`] (Space / Enter / Backspace / other + up or down; no key), a held key's
//!   auto-repeat is dropped by a 32-byte "held now" bit map that a release clears. Nothing is queued or kept; off = unregistered;
//! * the mouse sounds (Order 064, `bu-keysound`) listen with [`set_mouse_sound`]: every button down / up is handed straight to
//!   its sink on this thread as a class-only [`MouseSoundEvent`] (Left / Right / Middle / Side + up or down). Moves and wheel
//!   turns are only counted. Same rules as the key sounds: nothing queued or kept, off = unregistered;
//! * the activity client gets one message on the first packet of any kind (moves too), then is disarmed.
//!
//! [`hub`] is the pure part (who needs what, which packet goes where, one wake-up per batch) and is unit-tested; `win`
//! is the thin Windows shell around it.

pub mod hub;
#[cfg(windows)]
pub mod pad;
#[cfg(windows)]
mod win;

pub use hub::pad as padbtn;
pub use hub::{MOUSE_LEFT, MOUSE_MIDDLE, MOUSE_RIGHT, MOUSE_X1, MOUSE_X2};
pub use hub::{MouseButtonClass, MouseSoundEvent, PadSoundClass, PadSoundEvent, PadStats, RawPacket, SoundClass, SoundEvent, Stats, Target, MAX_CHATTER_MS, QUEUE_CAP};
#[cfg(windows)]
pub use win::{list_pads, notify_on_input, pad_stats, set_key_sound, set_keys, set_mouse_sound, set_pad_sound, set_sound_chatter, stats, take_packets, MouseSink, PadSink, SoundSink};
