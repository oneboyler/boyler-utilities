//! The always-on part of the Noise tab: the process's ONE noise player. Making it opens nothing and starts nothing; its worker
//! thread and its stream exist only while the noise plays (a press of Play), and the noise keeps playing with the menu closed.
//! The tray's "Stop noise" item and the app's exit reach it from here.

use std::sync::OnceLock;

static ENGINE: OnceLock<bu_noise::Noise> = OnceLock::new();

/// The player (made on first use).
pub fn engine() -> &'static bu_noise::Noise {
    ENGINE.get_or_init(bu_noise::Noise::new)
}

/// The noise is on right now (playing or fading out) - the tray menu offers "Stop noise" then. Never makes the player.
pub fn playing() -> bool {
    ENGINE.get().is_some_and(|e| e.status().playing)
}

/// The tray's "Stop noise": fades out and closes everything.
pub fn stop() {
    if let Some(e) = ENGINE.get() {
        e.stop();
    }
}

/// The app is ending: the noise ends at once.
pub fn shutdown() {
    if let Some(e) = ENGINE.get() {
        e.shutdown();
    }
}

/// Held by the app for its whole life (`Page::background`): dropping it at exit ends the noise.
pub struct Bg;

impl crate::pages::Background for Bg {
    fn describe(&self) -> String {
        format!("noise playing={}", playing())
    }
}

impl Drop for Bg {
    fn drop(&mut self) {
        shutdown();
    }
}

/// App start: when the sleep timer or the output ends the noise by itself, the open menu is woken to show it.
pub fn start() {
    engine().set_on_change(std::sync::Arc::new(|| crate::services::Waker.wake()));
}
