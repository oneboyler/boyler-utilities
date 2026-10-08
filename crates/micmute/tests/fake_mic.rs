//! Mic mute against the FAKE audio stack and speaker. No real mic is muted, nothing is played.

use bu_micmute::fake::{FakeMicOs, FakeSoundOut};
use bu_micmute::{sound, MicChoice, MicError, MicMute, MicState, Sound, SoundSettings};
use std::sync::{Arc, Mutex};

const A: &str = "{0.0.1.00000000}.{aaaa}";
const B: &str = "{0.0.1.00000000}.{bbbb}";

struct Rig {
    os: FakeMicOs,
    out: FakeSoundOut,
    mm: MicMute,
}

fn rig() -> Rig {
    let os = FakeMicOs::new(&[(A, "Microphone (Shure MV7)"), (B, "Headset Microphone (Arctis)")]);
    let out = FakeSoundOut::default();
    let mm = MicMute::new(Arc::new(os.clone()), Arc::new(out.clone()));
    // the sound is OFF by default (Order 046); these tests are about what it plays when on
    mm.set_sound(SoundSettings { enabled: true, volume: 60, ..Default::default() });
    Rig { os, out, mm }
}

fn wav(s: Sound) -> Vec<u8> {
    sound::wav(s, 60).unwrap()
}

/// Collects change events.
fn recorder() -> (Arc<Mutex<Vec<MicState>>>, bu_micmute::ChangeFn) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s2 = seen.clone();
    (seen, Arc::new(move |st| s2.lock().unwrap().push(st)))
}

// ---------- read ----------

#[test]
fn lists_mics_and_default() {
    let r = rig();
    let d = r.mm.devices().unwrap();
    assert_eq!(d.len(), 2);
    assert!(d[0].is_default && !d[1].is_default);
    assert_eq!(d[1].name, "Headset Microphone (Arctis)");
    let s = r.mm.state().unwrap();
    assert_eq!(s.device.unwrap().id, A);
    assert!(!s.muted);
}

#[test]
fn no_mic_reads_as_no_device_and_actions_say_no_mic() {
    let os = FakeMicOs::new(&[]);
    let mm = MicMute::new(
        Arc::new(os),
        Arc::new(FakeSoundOut::default()),
    );
    assert_eq!(mm.state().unwrap(), MicState { device: None, muted: false });
    assert!(matches!(mm.toggle(), Err(MicError::NoMic(_))));
    assert!(matches!(mm.mute(), Err(MicError::NoMic(_))));
}

// ---------- apply: mute / unmute / toggle + sound ----------

#[test]
fn mute_unmute_toggle_on_the_default_mic_with_sounds() {
    let r = rig();
    assert!(r.mm.mute().unwrap().muted);
    assert!(r.os.muted(A) && !r.os.muted(B));
    assert!(!r.mm.unmute().unwrap().muted);
    assert!(r.mm.toggle().unwrap().muted);
    assert!(!r.mm.toggle().unwrap().muted);
    assert_eq!(r.os.sets(), vec![(A.into(), true), (A.into(), false), (A.into(), true), (A.into(), false)]);
    // Blip down on mute, Blip up on unmute (rig() switches the sound on at 60 %)
    assert_eq!(r.out.played(), vec![wav(Sound::BlipDown), wav(Sound::BlipUp), wav(Sound::BlipDown), wav(Sound::BlipUp)]);
}

#[test]
fn no_change_means_no_call_and_no_sound() {
    let r = rig();
    r.mm.unmute().unwrap(); // already live
    r.mm.mute().unwrap();
    r.mm.mute().unwrap(); // already muted (separate keys pressed twice)
    assert_eq!(r.os.sets(), vec![(A.into(), true)]);
    assert_eq!(r.out.played().len(), 1);
}

#[test]
fn sound_settings_switch_none_and_volume() {
    let r = rig();
    r.mm.set_sound(SoundSettings { enabled: false, ..Default::default() });
    r.mm.toggle().unwrap();
    assert!(r.out.played().is_empty(), "Sound switch off → silent");

    r.mm.set_sound(SoundSettings { on_mute: Sound::Chime, on_unmute: Sound::None, volume: 100, enabled: true });
    r.mm.toggle().unwrap(); // unmute → None → silent
    r.mm.toggle().unwrap(); // mute → Chime at 100 %
    assert_eq!(r.out.played(), vec![sound::wav(Sound::Chime, 100).unwrap()]);

    r.mm.set_sound(SoundSettings { enabled: true, volume: 0, ..Default::default() });
    r.mm.toggle().unwrap();
    assert_eq!(r.out.played().len(), 1, "volume 0 → silent");

    r.mm.set_sound(SoundSettings { volume: 250, ..Default::default() });
    assert_eq!(r.mm.sound().volume, 100, "volume is stored as 0–100");
}

#[test]
fn preview_plays_at_the_current_volume_and_none_plays_nothing() {
    let r = rig();
    r.mm.set_sound(SoundSettings { volume: 35, ..Default::default() });
    r.mm.preview(Sound::SoftClick).unwrap();
    r.mm.preview(Sound::None).unwrap();
    assert_eq!(r.out.played(), vec![sound::wav(Sound::SoftClick, 35).unwrap()]);
    assert!(!Sound::None.can_preview() && Sound::Chime.can_preview());
}

#[test]
fn a_broken_speaker_never_breaks_the_mute() {
    let r = rig();
    r.out.fail();
    assert!(r.mm.mute().unwrap().muted);
    assert!(r.os.muted(A));
}

#[test]
fn os_error_on_set_is_returned_and_nothing_else_happens() {
    let r = rig();
    r.os.fail_next_set(MicError::Os { context: "SetMute".into(), code: 0x8007_0005 });
    assert!(matches!(r.mm.mute(), Err(MicError::Os { code: 0x8007_0005, .. })));
    assert!(!r.os.muted(A));
    assert!(r.out.played().is_empty());
    assert_eq!(r.mm.undo().unwrap(), None, "a failed change leaves nothing to undo");
}

#[test]
fn read_only_layer_refuses_every_change() {
    let os = FakeMicOs::new(&[(A, "Mic")]).read_only();
    let mm = MicMute::new(
        Arc::new(os.clone()),
        Arc::new(FakeSoundOut::default()),
    );
    assert!(matches!(mm.toggle(), Err(MicError::ReadOnly(_))));
    assert!(os.sets().is_empty());
    assert!(!mm.state().unwrap().muted);
}

// ---------- the mic choice ----------

#[test]
fn a_picked_mic_is_the_one_muted() {
    let r = rig();
    r.mm.set_choice(MicChoice::Device(B.into())).unwrap();
    r.mm.toggle().unwrap();
    assert!(r.os.muted(B) && !r.os.muted(A));
    assert_eq!(r.mm.state().unwrap().device.unwrap().id, B);
}

#[test]
fn a_picked_mic_that_is_gone_is_no_mic_not_another_mic() {
    let r = rig();
    assert!(matches!(r.mm.set_choice(MicChoice::Device("{gone}".into())), Err(MicError::NoMic(_))));
    assert_eq!(r.mm.choice(), MicChoice::Default, "a refused pick changes nothing");
    r.mm.set_choice(MicChoice::Device(B.into())).unwrap();
    r.os.remove(B);
    assert!(matches!(r.mm.toggle(), Err(MicError::NoMic(_))));
    assert!(!r.os.muted(A), "never falls back to muting a different mic");
    assert_eq!(r.mm.state().unwrap().device, None);
}

#[test]
fn switching_mic_while_muted_carries_the_mute_silently() {
    let r = rig();
    r.mm.mute().unwrap();
    let sounds = r.out.played().len();
    r.mm.set_choice(MicChoice::Device(B.into())).unwrap();
    assert!(!r.os.muted(A), "old mic not left muted");
    assert!(r.os.muted(B), "new mic muted");
    assert_eq!(r.out.played().len(), sounds, "silent");
    // live → switch → stays live
    r.mm.unmute().unwrap();
    r.mm.set_choice(MicChoice::Default).unwrap();
    assert!(!r.os.muted(A) && !r.os.muted(B));
}

// ---------- undo ----------

#[test]
fn undo_restores_the_state_before_the_last_change() {
    let r = rig();
    r.mm.mute().unwrap();
    let st = r.mm.undo().unwrap().unwrap();
    assert!(!st.muted && !r.os.muted(A));
    assert_eq!(r.mm.undo().unwrap(), None, "one undo per change");
    r.os.external_mute(A, true);
    r.mm.unmute().unwrap();
    r.mm.undo().unwrap();
    assert!(r.os.muted(A), "undo goes back to muted when it was muted before");
    assert_eq!(r.out.played().len(), 2, "undo is silent");
}

// ---------- switching the card off ----------

#[test]
fn turning_off_silently_unmutes_and_stops_watching() {
    let r = rig();
    r.mm.start_watching(Arc::new(|_| {})).unwrap();
    r.mm.mute().unwrap();
    let sounds = r.out.played().len();
    let st = r.mm.turn_off().unwrap();
    assert!(!st.muted && !r.os.muted(A));
    assert_eq!(r.out.played().len(), sounds, "silent");
    assert!(!r.mm.is_watching());
    assert!(r.os.live_mute_watches().is_empty() && r.os.live_device_watches() == 0);
}

#[test]
fn turning_off_also_unmutes_an_old_default_we_muted() {
    let r = rig();
    r.mm.mute().unwrap(); // A muted by us
    // the user changes Windows' default without our watch running (card was not watching)
    r.os.set_default(Some(B));
    r.mm.turn_off().unwrap();
    assert!(!r.os.muted(A) && !r.os.muted(B));
}

#[test]
fn turning_off_leaves_a_mic_another_app_muted_alone_unless_it_is_the_chosen_one() {
    let r = rig();
    r.os.external_mute(B, true); // B muted by another app, B is not our mic
    r.mm.turn_off().unwrap();
    assert!(r.os.muted(B), "not our mic, not our mute: left alone");
    r.os.external_mute(A, true); // the CHOSEN mic muted by another app
    r.mm.turn_off().unwrap();
    assert!(!r.os.muted(A), "DESIGN: switching off silently unmutes the chosen mic if muted");
}

#[test]
fn switching_mic_never_moves_another_apps_mute() {
    let r = rig();
    r.os.external_mute(A, true); // Discord muted A
    r.mm.set_choice(MicChoice::Device(B.into())).unwrap();
    assert!(r.os.muted(A), "another app's mute stays");
    assert!(!r.os.muted(B), "and is not copied");
    assert!(r.os.sets().is_empty());
}

// ---------- the change event ----------

#[test]
fn another_app_muting_fires_the_event_our_own_change_does_not() {
    let r = rig();
    let (seen, cb) = recorder();
    r.mm.start_watching(cb).unwrap();
    assert_eq!(r.os.live_mute_watches(), vec![A.to_string()]);
    r.mm.mute().unwrap(); // ours: returned, not evented
    assert!(seen.lock().unwrap().is_empty());
    r.os.external_mute(A, false); // Discord unmutes it
    r.os.external_mute(B, true); // not our mic: no event
    r.mm.settle();
    let s = seen.lock().unwrap().clone();
    assert_eq!(s.len(), 1);
    assert!(!s[0].muted);
    assert_eq!(s[0].device.as_ref().unwrap().id, A);
}

#[test]
fn default_mic_change_moves_the_watch_and_carries_our_mute() {
    let r = rig();
    let (seen, cb) = recorder();
    r.mm.start_watching(cb).unwrap();
    r.mm.mute().unwrap();
    r.os.set_default(Some(B));
    r.mm.settle();
    assert_eq!(r.os.live_mute_watches(), vec![B.to_string()]);
    assert!(r.os.muted(B) && !r.os.muted(A), "nothing gets through after the default changed");
    let last = seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last.device.unwrap().id, B);
    assert!(last.muted);
    // B's own changes now arrive
    r.os.external_mute(B, false);
    r.mm.settle();
    assert!(!seen.lock().unwrap().last().unwrap().muted);
}

#[test]
fn default_change_does_not_move_a_picked_mic() {
    let r = rig();
    r.mm.set_choice(MicChoice::Device(B.into())).unwrap();
    r.mm.start_watching(Arc::new(|_| {})).unwrap();
    r.os.set_default(Some(B));
    r.os.set_default(Some(A));
    r.mm.settle();
    assert_eq!(r.os.live_mute_watches(), vec![B.to_string()]);
}

#[test]
fn unplugging_the_mic_reports_no_device() {
    let r = rig();
    let (seen, cb) = recorder();
    r.mm.set_choice(MicChoice::Device(B.into())).unwrap();
    r.mm.start_watching(cb).unwrap();
    r.os.remove(B);
    r.mm.settle();
    assert_eq!(seen.lock().unwrap().last().unwrap().device, None);
    assert!(r.os.live_mute_watches().is_empty());
}

#[test]
fn stop_watching_unregisters_everything_and_restart_works() {
    let r = rig();
    r.mm.start_watching(Arc::new(|_| {})).unwrap();
    r.mm.start_watching(Arc::new(|_| {})).unwrap(); // a second start replaces the first
    assert_eq!(r.os.live_device_watches(), 1);
    assert_eq!(r.os.live_mute_watches().len(), 1);
    r.mm.stop_watching();
    assert_eq!(r.os.live_device_watches(), 0);
    assert!(r.os.live_mute_watches().is_empty());
    r.mm.settle(); // no worker: returns at once
}
