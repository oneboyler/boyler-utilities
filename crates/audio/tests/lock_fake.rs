//! Order 118 - the input volume lock: the engine acting on the FAKE (never a real device). The notification itself
//! (Windows -> `Event::InputVolume`) is covered in the `watch` unit tests.

use bu_audio::engine::{Did, Engine, Event};
use bu_audio::keep::DeviceEvent;
use bu_audio::newvol::NewAppsVolume;
use bu_audio::*;
use std::time::Duration;

fn engine() -> Engine<FakeOs> {
    Engine::new(FakeOs::drawing(), true, NewAppsVolume::new(false, 0.5, 0)).unwrap()
}

fn vol(e: &mut Engine<FakeOs>, id: &str) -> f32 {
    e.os.volumes[id].volume
}

fn changed(e: &mut Engine<FakeOs>, id: &str, v: f32) -> Vec<Did> {
    // what Windows / an app did: the device is moved, then the notification arrives
    e.os.volumes.get_mut(id).unwrap().volume = v;
    e.handle(Event::InputVolume { id: id.into(), volume: v }, Duration::ZERO)
}

#[test]
fn locking_puts_the_default_input_at_the_value_at_once() {
    let mut e = engine();
    assert_eq!(vol(&mut e, "mv7"), 0.90);
    let did = e.set_lock(Some(0.62));
    assert_eq!(did, vec![Did::LockPutBack { id: "mv7".into(), from: 0.90, to: 0.62 }]);
    assert_eq!(vol(&mut e, "mv7"), 0.62);
    assert_eq!(e.os.log, vec!["set_volume mv7 62".to_string()]);
}

#[test]
fn locking_at_the_value_it_already_has_changes_nothing() {
    let mut e = engine();
    assert!(e.set_lock(Some(0.90)).is_empty());
    assert!(e.os.log.is_empty());
}

#[test]
fn windows_lowering_and_raising_it_is_put_back_every_time() {
    let mut e = engine();
    e.set_lock(Some(0.62));
    for wrong in [0.10, 1.0, 0.0, 0.63, 0.31] {
        let did = changed(&mut e, "mv7", wrong);
        assert_eq!(did, vec![Did::LockPutBack { id: "mv7".into(), from: wrong, to: 0.62 }], "after {wrong}");
        assert_eq!(vol(&mut e, "mv7"), 0.62);
    }
}

#[test]
fn the_echo_of_our_own_put_back_is_not_put_back_again() {
    let mut e = engine();
    e.set_lock(Some(0.62));
    let before = e.os.log.len();
    assert!(e.handle(Event::InputVolume { id: "mv7".into(), volume: 0.62 }, Duration::ZERO).is_empty());
    assert_eq!(e.os.log.len(), before);
}

#[test]
fn unlocked_nothing_is_put_back() {
    let mut e = engine();
    assert!(changed(&mut e, "mv7", 0.2).is_empty());
    e.set_lock(Some(0.5));
    assert!(e.set_lock(None).is_empty());
    assert!(changed(&mut e, "mv7", 0.2).is_empty());
    assert_eq!(vol(&mut e, "mv7"), 0.2);
}

#[test]
fn only_the_input_lock_not_the_output() {
    let mut e = engine();
    e.set_lock(Some(0.5));
    // an output device's change is no input event; and a notification from a non-default input is ignored
    assert!(changed(&mut e, "arctis", 0.1).is_empty());
    assert!(changed(&mut e, "c920", 0.1).is_empty());
    assert_eq!(vol(&mut e, "arctis"), 0.1);
    assert_eq!(vol(&mut e, "c920"), 0.1);
}

#[test]
fn the_lock_follows_a_change_of_default_input_device() {
    let mut e = engine();
    e.set_lock(Some(0.62));
    // the user picks the webcam mic (50 %): the lock now holds on it
    let did = e.handle(
        Event::Device(DeviceEvent::DefaultChanged { flow: Flow::Input, role: Role::Console, id: Some("c920".into()) }),
        Duration::from_secs(100),
    );
    assert!(did.contains(&Did::LockPutBack { id: "c920".into(), from: 0.5, to: 0.62 }), "{did:?}");
    assert_eq!(vol(&mut e, "c920"), 0.62);
    // the old default is let go; the new one is held
    assert!(changed(&mut e, "mv7", 0.2).is_empty());
    assert_eq!(changed(&mut e, "c920", 0.2).len(), 1);
    assert_eq!(vol(&mut e, "c920"), 0.62);
}

#[test]
fn another_role_or_the_output_default_does_not_move_the_lock() {
    let mut e = engine();
    e.set_lock(Some(0.62));
    for (flow, role, id) in [(Flow::Input, Role::Communications, "c920"), (Flow::Output, Role::Console, "spk")] {
        e.handle(Event::Device(DeviceEvent::DefaultChanged { flow, role, id: Some(id.into()) }), Duration::from_secs(100));
    }
    assert_eq!(e.lock.watching(), Some("mv7"));
}

#[test]
fn a_device_with_coarse_steps_is_not_fought_in_a_loop() {
    let mut e = engine();
    e.set_lock(Some(0.62));
    // the device can only hold 61 %: after the put-back it reads 61
    e.os.volumes.get_mut("mv7").unwrap().volume = 0.30;
    let did = e.handle(Event::InputVolume { id: "mv7".into(), volume: 0.30 }, Duration::ZERO);
    assert_eq!(did.len(), 1);
    e.os.volumes.get_mut("mv7").unwrap().volume = 0.61;
    e.lock.note_settled(0.61);
    let n = e.os.log.len();
    assert!(e.handle(Event::InputVolume { id: "mv7".into(), volume: 0.61 }, Duration::ZERO).is_empty());
    assert_eq!(e.os.log.len(), n);
}

#[test]
fn a_refused_put_back_is_reported_not_swallowed() {
    let mut e = engine();
    e.set_lock(Some(0.62));
    e.os.volumes.get_mut("mv7").unwrap().volume = 0.2;
    e.os.fail_next = Some(AudioError::NeedsAdmin("test".into()));
    let did = e.handle(Event::InputVolume { id: "mv7".into(), volume: 0.2 }, Duration::ZERO);
    assert!(matches!(did.as_slice(), [Did::Failed(_)]), "{did:?}");
}
