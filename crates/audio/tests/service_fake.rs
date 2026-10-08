//! Devices, defaults, volumes, on/off, mixer, undo, admin + error paths — all against the fake. Never touches Windows.

use bu_audio::fake::{dev, session};
use bu_audio::*;

fn svc() -> AudioService<FakeOs> {
    AudioService::new(FakeOs::drawing())
}

fn log(s: &AudioService<FakeOs>) -> Vec<String> {
    s.os().log.clone()
}

// ------------------------------------------------------------------------------------------------ devices + defaults
#[test]
fn device_rows_names_glyphs_current_and_lock() {
    let mut s = svc();
    let rows = s.device_rows(Flow::Output).unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows.iter().filter(|r| r.current).map(|r| r.device.id.as_str()).collect::<Vec<_>>(), vec!["arctis"]);
    assert_eq!(rows.iter().map(|r| r.device.kind.glyph()).collect::<Vec<_>>(), vec!["spk", "hp", "mon", "pad"]);
    assert!(rows.iter().all(|r| r.on && !r.switch_locked));
    let ins = s.device_rows(Flow::Input).unwrap();
    assert_eq!(ins.iter().find(|r| r.current).unwrap().device.name, "Microphone (Shure MV7)");
    // unplugged devices are not listed
    s.os_mut().devices[0].state = DeviceState::Unplugged;
    assert_eq!(s.device_rows(Flow::Output).unwrap().len(), 3);
}

#[test]
fn select_default_sets_all_three_roles_and_undo() {
    let mut s = svc();
    let c = s.select_default(Flow::Output, "spk").unwrap();
    assert_eq!(log(&s), vec!["set_default spk Console", "set_default spk Multimedia", "set_default spk Communications"]);
    assert_eq!(s.current(Flow::Output).unwrap().unwrap().id, "spk");
    s.undo(&c).unwrap();
    assert_eq!(s.defaults(Flow::Output).unwrap().console.as_deref(), Some("arctis"));
    assert_eq!(s.defaults(Flow::Output).unwrap().communications.as_deref(), Some("arctis"));
}

#[test]
fn select_default_only_sets_roles_that_differ() {
    let mut s = svc();
    s.os_mut().defaults.insert((Flow::Output, Role::Communications), "spk".into());
    s.select_default(Flow::Output, "spk").unwrap();
    assert_eq!(log(&s), vec!["set_default spk Console", "set_default spk Multimedia"]);
}

#[test]
fn switched_off_or_unknown_device_cant_be_picked() {
    let mut s = svc();
    s.os_mut().devices[0].state = DeviceState::Off;
    assert_eq!(s.select_default(Flow::Output, "spk"), Err(AudioError::DeviceOff("Speakers (Realtek)".into())));
    assert_eq!(s.select_default(Flow::Output, "nope"), Err(AudioError::NotFound("nope".into())));
    assert_eq!(s.select_default(Flow::Input, "spk"), Err(AudioError::NotFound("spk".into())), "an output is not an input");
    assert!(log(&s).is_empty());
}

#[test]
fn device_volume_mute_clamp_and_undo() {
    let mut s = svc();
    assert_eq!(s.device_volume("arctis").unwrap(), VolumeMute { volume: 0.74, muted: false });
    let c = s.set_device_volume("arctis", 1.7).unwrap();
    assert_eq!(s.device_volume("arctis").unwrap().volume, 1.0);
    s.set_device_volume("arctis", f32::NAN).unwrap();
    assert_eq!(s.device_volume("arctis").unwrap().volume, 0.0);
    s.undo(&c).unwrap();
    assert_eq!(s.device_volume("arctis").unwrap().volume, 0.74);
    let m = s.set_device_mute("mv7", true).unwrap();
    assert!(s.device_volume("mv7").unwrap().muted);
    s.undo(&m).unwrap();
    assert!(!s.device_volume("mv7").unwrap().muted);
}

// ------------------------------------------------------------------------------------------------ device on/off
#[test]
fn switching_off_the_device_in_use_moves_windows_first_then_undo() {
    let mut s = svc();
    let c = s.set_device_on(Flow::Output, "arctis", false).unwrap();
    assert_eq!(
        log(&s),
        vec!["set_default spk Console", "set_default spk Multimedia", "set_default spk Communications", "set_enabled arctis false"]
    );
    let rows = s.device_rows(Flow::Output).unwrap();
    let a = rows.iter().find(|r| r.device.id == "arctis").unwrap();
    assert!(!a.on && !a.current);
    assert!(rows.iter().find(|r| r.device.id == "spk").unwrap().current, "the first device still on");
    s.undo(&c).unwrap();
    assert_eq!(s.current(Flow::Output).unwrap().unwrap().id, "arctis");
    assert_eq!(s.os().devices.iter().find(|d| d.id == "arctis").unwrap().state, DeviceState::On);
}

#[test]
fn switching_off_a_device_not_in_use_moves_nothing() {
    let mut s = svc();
    s.set_device_on(Flow::Output, "ds", false).unwrap();
    assert_eq!(log(&s), vec!["set_enabled ds false"]);
    let c = s.set_device_on(Flow::Output, "ds", false).unwrap();
    assert_eq!(log(&s).len(), 1, "already off: nothing to do");
    assert!(matches!(c, Change::DeviceOn { moved: None, .. }));
    s.set_device_on(Flow::Output, "ds", true).unwrap();
    assert_eq!(log(&s).last().unwrap(), "set_enabled ds true");
}

#[test]
fn one_device_always_stays_on() {
    let mut s = svc();
    for id in ["spk", "nv", "ds"] {
        s.set_device_on(Flow::Output, id, false).unwrap();
    }
    let rows = s.device_rows(Flow::Output).unwrap();
    assert!(rows.iter().find(|r| r.device.id == "arctis").unwrap().switch_locked, "tip: One device always stays on");
    let n = log(&s).len();
    assert_eq!(s.set_device_on(Flow::Output, "arctis", false), Err(AudioError::LastDeviceOn));
    assert_eq!(log(&s).len(), n, "nothing changed");
}

#[test]
fn device_on_off_needs_admin_puts_defaults_back() {
    let mut s = svc();
    s.os_mut().enable_needs_admin = true;
    let e = s.set_device_on(Flow::Output, "arctis", false).unwrap_err();
    assert!(matches!(e, AudioError::NeedsAdmin(_)), "{e:?}");
    // moved to spk first, then back to arctis because the switch itself was refused
    assert_eq!(s.current(Flow::Output).unwrap().unwrap().id, "arctis");
    assert_eq!(log(&s).last().unwrap(), "set_default arctis Communications");
}

#[test]
fn failing_os_call_is_an_error_not_a_panic() {
    let mut s = svc();
    s.os_mut().fail_next = Some(AudioError::Os { context: "SetMasterVolumeLevelScalar".into(), code: 0x8889_0004 });
    assert!(matches!(s.set_device_volume("arctis", 0.3), Err(AudioError::Os { .. })));
    assert_eq!(s.device_volume("arctis").unwrap().volume, 0.74);
}

// ------------------------------------------------------------------------------------------------ mixer
#[test]
fn mixer_one_row_per_app_making_sound_system_last() {
    let mut s = svc();
    let apps = s.apps("arctis").unwrap();
    let names: Vec<&str> = apps.iter().map(|a| a.look.name.as_str()).collect();
    assert_eq!(names, vec!["Spotify", "Discord", "System sounds"], "Chrome is quiet (no active session); the game plays on spk");
    let d = &apps[1];
    assert_eq!(d.sessions, vec!["discord-1", "discord-2"], "both Discord processes grouped");
    assert_eq!(d.pids, vec![200, 201]);
    assert_eq!(d.volume, 0.8);
    assert!(apps[2].system);
    assert_eq!(s.apps("spk").unwrap()[0].look.name, "Game");
}

#[test]
fn app_volume_sets_every_session_unmutes_and_undo() {
    let mut s = svc();
    s.os_mut().sessions[2].1.muted = true; // discord-2 muted
    let g = s.apps("arctis").unwrap()[1].group.clone();
    let c = s.set_app_volume("arctis", &g, 0.3).unwrap();
    assert_eq!(
        log(&s),
        vec!["set_session_volume discord-1 30", "set_session_volume discord-2 30", "set_session_mute discord-2 false"],
        "moving the slider unmutes, as in Windows"
    );
    s.undo(&c).unwrap();
    let ss = &s.os().sessions;
    assert_eq!((ss[1].1.volume, ss[1].1.muted), (0.8, false));
    assert_eq!((ss[2].1.volume, ss[2].1.muted), (0.6, true));
}

#[test]
fn app_mute_and_undo() {
    let mut s = svc();
    let g = s.apps("arctis").unwrap()[0].group.clone();
    let c = s.set_app_mute("arctis", &g, true).unwrap();
    assert!(s.apps("arctis").unwrap()[0].muted);
    s.undo(&c).unwrap();
    assert!(!s.apps("arctis").unwrap()[0].muted);
    assert_eq!(s.set_app_mute("arctis", "c:\\nope.exe", true), Err(AudioError::NotFound("c:\\nope.exe".into())));
}

#[test]
fn undo_skips_an_app_that_closed() {
    let mut s = svc();
    let g = s.apps("arctis").unwrap()[1].group.clone();
    let c = s.set_app_volume("arctis", &g, 0.2).unwrap();
    s.os_mut().sessions.retain(|(_, x)| x.key != "discord-2");
    s.undo(&c).unwrap();
    assert_eq!(s.os().sessions[1].1.volume, 0.8);
}

#[test]
fn app_without_exe_path_groups_by_pid() {
    let mut f = FakeOs::drawing();
    f.sessions.push(("arctis".into(), session("x-1", 777, "", SessionState::Active, 1.0)));
    let mut s = AudioService::new(f);
    let apps = s.apps("arctis").unwrap();
    assert!(apps.iter().any(|a| a.group == "pid:777" && a.look.name == "App 777"));
}

/// Order 036: the reset puts a flow's three roles back as they were (e.g. calls on another device), refuses a device that
/// is gone or switched off, and leaves roles already right alone.
#[test]
fn set_defaults_puts_every_role_back() {
    let mut s = svc();
    let mut to = Defaults::default();
    to.set(Role::Console, Some("spk".into()));
    to.set(Role::Multimedia, Some("spk".into()));
    to.set(Role::Communications, Some("arctis".into()));
    s.set_defaults(Flow::Output, &to).unwrap();
    assert_eq!(s.defaults(Flow::Output).unwrap(), to);
    assert_eq!(log(&s), vec!["set_default spk Console", "set_default spk Multimedia"], "the calls role was already right");
    // a device that is gone / switched off: nothing changes
    s.os_mut().log.clear();
    to.set(Role::Console, Some("gone".into()));
    assert!(matches!(s.set_defaults(Flow::Output, &to), Err(AudioError::NotFound(_))));
    s.os_mut().devices.iter_mut().find(|d| d.id == "nv").unwrap().state = DeviceState::Off;
    to.set(Role::Console, Some("nv".into()));
    assert!(matches!(s.set_defaults(Flow::Output, &to), Err(AudioError::DeviceOff(_))));
    assert!(log(&s).is_empty());
}

// ------------------------------------------------------------------------------------------------ glyphs + colour
#[test]
fn device_kind_from_name_and_form_factor() {
    use DeviceKind::*;
    assert_eq!(DeviceKind::classify("Wireless Controller", 5, Flow::Output), Controller, "PS5 pad says Headset");
    assert_eq!(DeviceKind::classify("Headset Microphone (DualSense)", 4, Flow::Input), Controller);
    assert_eq!(DeviceKind::classify("Microphone (C920 HD Pro Webcam)", 4, Flow::Input), Webcam);
    assert_eq!(DeviceKind::classify("Headphones (Arctis Nova)", 3, Flow::Output), Headphones);
    assert_eq!(DeviceKind::classify("DELL S2721DGF (NVIDIA High Definition Audio)", 9, Flow::Output), Monitor);
    assert_eq!(DeviceKind::classify("Speakers (Realtek(R) Audio)", 1, Flow::Output), Speakers);
    assert_eq!(DeviceKind::classify("Microphone (Shure MV7)", 4, Flow::Input), Microphone);
    assert_eq!(DeviceKind::classify("Line In", 2, Flow::Input), Microphone);
    let _ = dev("x", "y", Speakers, Flow::Output);
}

#[test]
fn icon_colour_picks_the_icons_hue_or_grey() {
    use bu_audio::colour::icon_colour;
    let solid = |b: u8, g: u8, r: u8, a: u8| Icon { w: 4, h: 4, bgra: [b, g, r, a].repeat(16) };
    let (c, c2) = icon_colour(&solid(0x40, 0xc4, 0x2f, 255)); // a Spotify-like green
    let (r, g, b) = ((c >> 16) & 255, (c >> 8) & 255, c & 255);
    assert!(g > r + 80 && g > b + 80, "green: {c:06x}");
    assert!((c2 & 0xff00) >> 8 > g, "the second colour is lighter: {c2:06x}");
    assert_eq!(icon_colour(&solid(0x80, 0x80, 0x80, 255)), GREY, "grey icon → the drawing's grey");
    assert_eq!(icon_colour(&solid(0, 0, 0xff, 0)), GREY, "transparent → grey");
    assert_eq!(icon_colour(&Icon { w: 0, h: 0, bgra: vec![] }), GREY);
}
