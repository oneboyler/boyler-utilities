//! `RealOs::read_only()` refuses EVERY change. Only made-up ids / keys are used, so even a broken guard could change
//! nothing real (it would answer NotFound instead of ReadOnly). No Core Audio on the PC → skipped.

#![cfg(windows)]

use bu_audio::*;

const DEV: &str = "{bu-audio-no-such-device}";
const KEY: &str = "bu-audio-no-such-session";

#[test]
fn read_only_refuses_every_change() {
    let Ok(mut os) = RealOs::read_only() else {
        eprintln!("no Core Audio: skipped");
        return;
    };
    assert!(os.is_read_only());
    let refused = |r: Result<()>, what: &str| assert!(matches!(r, Err(AudioError::ReadOnly(_))), "{what}: {r:?}");
    refused(os.set_default(DEV, Role::Console), "set_default");
    refused(os.set_volume(DEV, 0.5), "set_volume");
    refused(os.set_mute(DEV, true), "set_mute");
    refused(os.set_enabled(DEV, false), "set_enabled");
    refused(os.set_session_volume(KEY, 0.5), "set_session_volume");
    refused(os.set_session_mute(KEY, true), "set_session_mute");
}
