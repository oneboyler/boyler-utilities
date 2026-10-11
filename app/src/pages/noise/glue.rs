//! The always-on part of the Noise tab: the process's ONE noise player. Making it opens nothing and starts nothing; its worker
//! thread and its stream exist only while the noise plays (a press of Play), and the noise keeps playing with the menu closed.
//! The tray's "Stop noise" item and the app's exit reach it from here.
//!
//! Order 092: the listening tracker (`bu_noise::listen`) is fed from here, so every way the noise starts and ends passes it:
//! Play, the Stop button, the tray's "Stop noise", the sleep timer / a lost player (the engine's change call), the app's exit.

use bu_noise::listen::{self, Tracker, Totals};
use std::sync::OnceLock;

static ENGINE: OnceLock<bu_noise::Noise> = OnceLock::new();
/// Counts the listening time. Without a file (`set_file` not called) it only counts in memory.
static LISTEN: Tracker = Tracker::new();

/// The player (made on first use).
pub fn engine() -> &'static bu_noise::Noise {
    ENGINE.get_or_init(bu_noise::Noise::new)
}

/// The noise is on right now (playing or fading out) - the tray menu offers "Stop noise" then. Never makes the player.
pub fn playing() -> bool {
    ENGINE.get().is_some_and(|e| e.status().playing)
}

/// Seconds of sound the player has written so far (0 before it was made).
fn played() -> f64 {
    ENGINE.get().map_or(0.0, |e| e.status().played)
}

/// Play: the listening stretch starts, then the player.
pub fn play(sound: bu_noise::Sound, volume: u8, sleep: Option<u32>) {
    LISTEN.begin(listen::local_now(), played());
    engine().play(sound, volume, sleep);
}

/// Stop (the page's button and the tray's item): the stretch ends now, the player fades out.
pub fn stop() {
    LISTEN.end(listen::local_now(), played());
    if let Some(e) = ENGINE.get() {
        e.stop();
    }
}

/// The app is ending: the noise ends at once.
pub fn shutdown() {
    LISTEN.end(listen::local_now(), played());
    if let Some(e) = ENGINE.get() {
        e.shutdown();
    }
}

/// The ledger's file (the settings folder); nothing is read or written until the first Stop / the first look.
pub fn set_listen_file(folder: &std::path::Path) {
    LISTEN.set_file(folder.join("noise-listening.txt"));
}

/// The five figures for the Noise tab (as of now).
pub fn listened() -> Totals {
    LISTEN.totals(listen::local_now(), played())
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

/// App start: when the sleep timer or the output ends the noise by itself, the open menu is woken to show it - and the
/// listening stretch ends (the engine says so from its worker thread when that thread leaves).
pub fn start() {
    engine().set_on_change(std::sync::Arc::new(|| {
        if LISTEN.is_open() && !playing() {
            LISTEN.end(listen::local_now(), played());
        }
        crate::services::Waker.wake()
    }));
}
