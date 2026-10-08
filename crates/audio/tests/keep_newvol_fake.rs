//! Keep my devices + New apps volume — the rules and the engine acting on the fake. Times are passed in by hand
//! (no real clock, no load dependence).

use bu_audio::engine::{Did, Engine, Event};
use bu_audio::fake::{dev, session};
use bu_audio::keep::{DeviceEvent, KeepDevices, PutBack, ARRIVAL_WINDOW};
use bu_audio::newvol::{NewAppsVolume, DEFAULT_ON, DEFAULT_VOLUME};
use bu_audio::*;
use std::time::Duration;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn keep() -> KeepDevices {
    let present = ["arctis", "spk", "mv7"].map(String::from);
    let mut defs = Vec::new();
    for r in ROLES {
        defs.push(((Flow::Output, r), "arctis".to_string()));
        defs.push(((Flow::Input, r), "mv7".to_string()));
    }
    KeepDevices::new(true, present, defs)
}

fn plug(id: &str, flow: Flow) -> DeviceEvent {
    DeviceEvent::Present { id: id.into(), flow }
}
fn def(flow: Flow, role: Role, id: &str) -> DeviceEvent {
    DeviceEvent::DefaultChanged { flow, role, id: Some(id.into()) }
}
fn gone(id: &str) -> DeviceEvent {
    DeviceEvent::Gone { id: id.into() }
}

#[test]
fn ps5_controller_plugged_in_gets_undone_all_roles() {
    let mut k = keep();
    assert!(k.on_event(plug("ds", Flow::Output), ms(1000)).is_empty());
    for (i, r) in ROLES.iter().enumerate() {
        let back = k.on_event(def(Flow::Output, *r, "ds"), ms(1400 + i as u64));
        assert_eq!(back, vec![PutBack { id: "arctis".into(), role: *r }]);
    }
    // Windows then reports our own put-back: nothing more to do
    for r in ROLES {
        assert!(k.on_event(def(Flow::Output, r, "arctis"), ms(1500)).is_empty());
    }
    assert_eq!(k.chosen(Flow::Output, Role::Console).map(String::as_str), Some("arctis"));
}

#[test]
fn input_is_kept_too() {
    let mut k = keep();
    k.on_event(plug("ds-mic", Flow::Input), ms(0));
    assert_eq!(k.on_event(def(Flow::Input, Role::Communications, "ds-mic"), ms(300)), vec![PutBack { id: "mv7".into(), role: Role::Communications }]);
}

#[test]
fn default_event_before_the_plug_event_counts_as_arrival() {
    let mut k = keep();
    assert_eq!(k.on_event(def(Flow::Output, Role::Console, "ds"), ms(50)), vec![PutBack { id: "arctis".into(), role: Role::Console }]);
    assert!(k.on_event(plug("ds", Flow::Output), ms(60)).is_empty());
}

#[test]
fn the_users_own_pick_is_kept_and_then_protected() {
    let mut k = keep();
    assert!(k.on_event(def(Flow::Output, Role::Console, "spk"), ms(0)).is_empty(), "no plug-in: the user's own choice");
    assert_eq!(k.chosen(Flow::Output, Role::Console).map(String::as_str), Some("spk"));
    k.on_event(plug("ds", Flow::Output), ms(10_000));
    assert_eq!(k.on_event(def(Flow::Output, Role::Console, "ds"), ms(10_500)), vec![PutBack { id: "spk".into(), role: Role::Console }]);
}

#[test]
fn a_switch_long_after_the_plug_in_is_the_users_choice() {
    let mut k = keep();
    k.on_event(plug("ds", Flow::Output), ms(0));
    let late = ARRIVAL_WINDOW + ms(1);
    assert!(k.on_event(def(Flow::Output, Role::Console, "ds"), late).is_empty());
    assert_eq!(k.chosen(Flow::Output, Role::Console).map(String::as_str), Some("ds"));
}

#[test]
fn headset_unplugged_fallback_accepted_and_its_return_kept() {
    let mut k = keep();
    k.on_event(gone("arctis"), ms(0));
    assert!(k.on_event(def(Flow::Output, Role::Console, "spk"), ms(200)).is_empty(), "Windows' fallback");
    assert_eq!(k.chosen(Flow::Output, Role::Console).map(String::as_str), Some("arctis"), "still the chosen one");
    // the controller is plugged in while the headset is away: back to the fallback (the chosen one isn't there)
    k.on_event(plug("ds", Flow::Output), ms(30_000));
    assert_eq!(k.on_event(def(Flow::Output, Role::Console, "ds"), ms(30_300)), vec![PutBack { id: "spk".into(), role: Role::Console }]);
    assert!(k.on_event(def(Flow::Output, Role::Console, "spk"), ms(30_400)).is_empty(), "our put-back reported");
    assert_eq!(k.chosen(Flow::Output, Role::Console).map(String::as_str), Some("arctis"), "the put-back is not a choice");
    // the headset comes back and Windows switches to it: kept
    k.on_event(plug("arctis", Flow::Output), ms(60_000));
    assert!(k.on_event(def(Flow::Output, Role::Console, "arctis"), ms(60_300)).is_empty());
}

#[test]
fn nothing_to_put_back_when_nothing_else_is_plugged_in() {
    let mut k = KeepDevices::new(true, Vec::<String>::new(), Vec::new());
    k.on_event(plug("ds", Flow::Output), ms(0));
    assert!(k.on_event(def(Flow::Output, Role::Console, "ds"), ms(100)).is_empty());
}

#[test]
fn switch_off_never_puts_back_and_on_takes_todays_defaults() {
    let mut k = keep();
    k.set_on(false);
    k.on_event(plug("ds", Flow::Output), ms(0));
    assert!(k.on_event(def(Flow::Output, Role::Console, "ds"), ms(100)).is_empty());
    k.set_on(true);
    assert_eq!(k.chosen(Flow::Output, Role::Console).map(String::as_str), Some("ds"));
    k.on_event(plug("nv", Flow::Output), ms(10_000));
    assert_eq!(k.on_event(def(Flow::Output, Role::Console, "nv"), ms(10_100)), vec![PutBack { id: "ds".into(), role: Role::Console }]);
}

#[test]
fn no_default_left_is_handled() {
    let mut k = keep();
    assert!(k.on_event(DeviceEvent::DefaultChanged { flow: Flow::Output, role: Role::Console, id: None }, ms(0)).is_empty());
}

// ------------------------------------------------------------------------------------------------ new apps volume
const SINCE: u64 = 5_000;

fn s(key: &str, pid: u32, started: u64) -> SessionInfo {
    let mut x = session(key, pid, r"C:\Apps\x.exe", SessionState::Active, 1.0);
    x.process_started = started;
    x
}

#[test]
fn new_apps_volume_first_session_of_each_new_app_start() {
    assert_eq!((DEFAULT_ON, DEFAULT_VOLUME), (true, 0.5), "the drawing's defaults (A_012_01)");
    let mut n = NewAppsVolume::new(true, 0.5, SINCE);
    assert_eq!(n.on_session_created(&s("a1", 10, SINCE + 1)), Some(0.5));
    assert_eq!(n.on_session_created(&s("a2", 10, SINCE + 1)), None, "second session of the same run: left alone");
    assert_eq!(n.on_session_created(&s("old", 11, SINCE - 1)), None, "already running before: never touched");
    assert_eq!(n.on_session_created(&s("u", 12, 0)), None, "start time unknown: left alone");
    assert_eq!(n.on_session_created(&s("r1", 10, SINCE + 99)), Some(0.5), "pid reused by a new run");
    let mut sys = s("sys", 13, SINCE + 1);
    sys.system = true;
    assert_eq!(n.on_session_created(&sys), None, "System sounds never");
    assert_eq!(n.on_session_created(&s("me", std::process::id(), SINCE + 1)), None, "not our own sounds");
    assert_eq!(n.remembered(), 2);
}

#[test]
fn new_apps_volume_switch_and_slider() {
    let mut n = NewAppsVolume::new(false, 0.5, SINCE);
    assert_eq!(n.on_session_created(&s("a", 10, SINCE + 1)), None, "off");
    n.set_on(true, SINCE + 10);
    assert_eq!(n.on_session_created(&s("a", 10, SINCE + 1)), None, "started before the switch went on");
    n.set_volume(1.4);
    assert_eq!(n.on_session_created(&s("b", 20, SINCE + 11)), Some(1.0));
    n.set_volume(f32::NAN);
    assert_eq!(n.volume(), 0.0);
}

// ------------------------------------------------------------------------------------------------ the engine on the fake
#[test]
fn engine_puts_back_through_the_os_and_sets_new_app_volume() {
    let mut f = FakeOs::drawing();
    f.plug(dev("ds2", "Wireless Controller", DeviceKind::Controller, Flow::Output));
    f.devices.retain(|d| d.id != "ds2"); // not plugged yet when the engine starts
    let mut e = Engine::new(f, true, NewAppsVolume::new(true, 0.5, SINCE)).unwrap();
    e.os.plug(dev("ds2", "Wireless Controller", DeviceKind::Controller, Flow::Output));
    e.os.windows_sets_default(Flow::Output, "ds2");
    assert!(e.handle(Event::Device(plug("ds2", Flow::Output)), ms(0)).is_empty());
    let did = e.handle(Event::Device(def(Flow::Output, Role::Console, "ds2")), ms(400));
    assert_eq!(did, vec![Did::PutBack { id: "arctis".into(), role: Role::Console }]);
    assert_eq!(e.os.defaults.get(&(Flow::Output, Role::Console)).map(String::as_str), Some("arctis"));
    // a new app
    let mut ns = session("spotify-1", 100, r"C:\Apps\Spotify.exe", SessionState::Active, 1.0);
    ns.process_started = SINCE + 1;
    assert_eq!(e.handle(Event::SessionCreated(ns), ms(500)), vec![Did::NewAppVolume { key: "spotify-1".into(), pid: 100, volume: 0.5 }]);
    assert_eq!(e.os.sessions[0].1.volume, 0.5);
    assert_eq!(e.os.log, vec!["set_default arctis Console", "set_session_volume spotify-1 50"]);
}

#[test]
fn engine_reports_failures() {
    let mut f = FakeOs::drawing();
    f.fail_next = Some(AudioError::ReadOnly("set default device".into()));
    let mut e = Engine::new(f, true, NewAppsVolume::new(true, 0.5, SINCE)).unwrap();
    e.handle(Event::Device(plug("ds-new", Flow::Output)), ms(0));
    let did = e.handle(Event::Device(def(Flow::Output, Role::Console, "ds-new")), ms(10));
    assert!(matches!(&did[0], Did::Failed(m) if m.contains("read-only")), "{did:?}");
    let mut gone_app = session("ghost", 999, r"C:\x.exe", SessionState::Active, 1.0);
    gone_app.process_started = SINCE + 5;
    assert!(matches!(&e.handle(Event::SessionCreated(gone_app), ms(20))[0], Did::Failed(_)), "a session Windows already closed");
}
