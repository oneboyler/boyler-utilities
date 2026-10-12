//! Order 015 proof against the FAKE OS layer + hand-written Steam-format files: read, every setting written with the
//! rest byte-identical, backup + undo, community layouts, action sets, per-controller preferences, controllers, live view.

mod common;

use bu_controller::binding::{Action, MouseButton, PadButton};
use bu_controller::fake::{FakePads, FakeSteam};
use bu_controller::layout::{Layout, Press};
use bu_controller::os::{Connection, LiveEvent, PadInfo, PadOs, PadSource};
use bu_controller::settings::*;
use bu_controller::steam::{LayoutSource, SteamInputSwitch};
use bu_controller::{ButtonId, ControllerService, Error, LiveView, PadKind, Part, PrefSetting, Side};
use common::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const EDGE: PadKind = PadKind::DualSenseEdge;

fn text(s: &ControllerService<FakeSteam>, p: &std::path::Path) -> String {
    s.os().text(p).expect("file exists")
}

fn val<T: PartialEq + Copy>(list: &[(T, Option<i64>)], k: T) -> Option<i64> {
    list.iter().find(|(s, _)| *s == k).and_then(|(_, v)| *v)
}

// ------------------------------------------------------------------------------------------------ finding + reading

#[test]
fn finds_steam_the_account_and_the_games() {
    let s = service();
    assert_eq!(s.steam().account, ACCOUNT);
    let games = s.games(EDGE).unwrap();
    let names: Vec<&str> = games.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["Rocket League", "Yakuza 0", "Epic Games Launcher"], "Desktop (413080) is not a file: left out");
    let rl = &games[0];
    assert_eq!((rl.appid, rl.source.clone(), rl.installed, rl.shortcut), (Some(252950), LayoutSource::Autosave, true, false));
    assert_eq!(rl.steam_input, SteamInputSwitch::Value("2".into()), "read from localconfig.vdf, never written");
    assert_eq!(games[1].source, LayoutSource::Workshop("3275392801".into()));
    assert!(games[1].installed, "found in the second library (D:)");
    assert!(games[1].is_community());
    assert!(games[2].shortcut && games[2].appid.is_none());
    assert!(matches!(s.game("413080", EDGE), Err(Error::DesktopNotAFile)));
    assert!(matches!(s.game("desktop", EDGE), Err(Error::DesktopNotAFile)));
    assert!(matches!(s.game("999", EDGE), Err(Error::NoLayoutFile(_))));
    // no xbox layouts in this fake Steam: an empty list, not an error
    assert!(s.games(PadKind::Xbox).unwrap().is_empty());
}

#[test]
fn no_steam_is_a_typed_error() {
    let f = FakeSteam::default();
    assert!(matches!(ControllerService::new(f, BACKUPS), Err(Error::NoSteam)));
    let f = FakeSteam::new(STEAM);
    f.put(r"C:\Steam\steam.exe", b"MZ");
    assert!(matches!(ControllerService::new(f, BACKUPS), Err(Error::NoAccount)));
}

#[test]
fn reads_his_rocket_league_layout_like_the_drawing() {
    let s = service();
    let v = s.view("252950", EDGE, 0).unwrap();
    let ls = v.sticks.iter().find(|x| x.side == Side::Left).unwrap();
    assert_eq!(ls.mode, StickMode::Joystick);
    assert_eq!(val(&ls.settings, StickSetting::DeadZone), Some(3357));
    assert_eq!(val(&ls.settings, StickSetting::FullAt), Some(25602));
    assert_eq!(radius_to_pct(3357).round(), 10.0, "the drawing: dead zone 10 %");
    assert_eq!(radius_to_pct(25602).round(), 78.0, "the drawing: full at 78 %");
    assert_eq!(val(&ls.settings, StickSetting::Curve), Some(5), "Custom");
    assert_eq!(val(&ls.settings, StickSetting::CurveShape), Some(192));
    assert_eq!(ls.press, Action::Pad(PadButton::L3));
    let rs = v.sticks.iter().find(|x| x.side == Side::Right).unwrap();
    assert_eq!(val(&rs.settings, StickSetting::DeadZone), Some(3395));
    assert_eq!(val(&rs.settings, StickSetting::SendsTo), Some(1));
    let b = |id| v.buttons.iter().find(|b| b.id == id).unwrap().clone();
    assert_eq!(b(ButtonId::BackRightUpper).presses[0], (Press::Full, Action::Key("F5".into())), "R4 = F5");
    assert_eq!(b(ButtonId::BackRightUpper).name, "R4");
    assert_eq!(val(&b(ButtonId::Cross).settings, PressSetting::FireEndDelay), Some(14));
    assert_eq!(val(&b(ButtonId::Triangle).settings, PressSetting::FireEndDelay), Some(20));
    assert_eq!(b(ButtonId::Mute).presses[0].1, Action::Steam(bu_controller::SteamAction::Screenshot));
    assert_eq!(b(ButtonId::Create).presses[0].1, Action::Pad(PadButton::Create));
    assert_eq!(b(ButtonId::Options).presses[0].1, Action::Pad(PadButton::Options));
    assert!(b(ButtonId::Home).fixed && b(ButtonId::Home).presses.is_empty());
    assert_eq!(b(ButtonId::BackLeftUpper).presses[0].1, Action::Nothing);
    let tp = v.touchpad.clone().unwrap();
    assert_eq!(tp.right_click, Action::light_bar_red(), "right touchpad click = light bar red");
    assert_eq!(tp.left_click, Action::Pad(PadButton::Create));
    assert_eq!(tp.touch, TouchMode::Nothing);
    assert!(v.triggers.iter().all(|t| t.analog));
    assert_eq!(v.gyro.as_ref().unwrap().mode, GyroMode::Off, "no gyro group in the user's layout");
    assert_eq!(v.buttons.len(), 20, "the Edge has all 20 (4 back slots)");
    assert_eq!(s.view("252950", PadKind::DualSense, 0).unwrap().buttons.len(), 16);
    let xb = s.view("252950", PadKind::Xbox, 0);
    assert!(xb.is_err(), "no Xbox layout for RL in this fake (only ps5 files)");
    // the amber dots: the user's changes against Steam's own (official) layout
    let ch = s.changed_parts("252950", EDGE, 0).unwrap();
    assert_eq!(ch, vec![Part::Button(ButtonId::BackRightUpper), Part::Stick(Side::Left), Part::Stick(Side::Right), Part::Touchpad]);
}

#[test]
fn action_sets_are_read_with_their_titles() {
    let s = service();
    let sets = s.action_sets("638970", EDGE).unwrap();
    assert_eq!(sets.iter().map(|x| (x.id, x.title.as_str())).collect::<Vec<_>>(), vec![(0, "Default"), (1, "In menus")]);
    let v0 = s.view("638970", EDGE, 0).unwrap();
    let v1 = s.view("638970", EDGE, 1).unwrap();
    assert_eq!(v0.gyro.as_ref().unwrap().mode, GyroMode::Mouse);
    assert_eq!(val(&v0.gyro.as_ref().unwrap().settings, GyroSetting::OnWhileHeld), Some(5), "R1");
    assert_eq!(v1.gyro.as_ref().unwrap().mode, GyroMode::Off, "bound inactive in set 1");
    let cross0 = v0.buttons.iter().find(|b| b.id == ButtonId::Cross).unwrap();
    assert_eq!(cross0.presses[1], (Press::Long, Action::Key("SPACE".into())));
    let circle0 = v0.buttons.iter().find(|b| b.id == ButtonId::Circle).unwrap();
    assert_eq!(circle0.extra_bindings, 1, "a key combo: Ctrl + C");
    let up1 = v1.buttons.iter().find(|b| b.id == ButtonId::Triangle).unwrap();
    assert_eq!(up1.presses[0].1, Action::Key("UP_ARROW".into()), "a dpad-mode group on the face buttons: north = Triangle");
}

// ------------------------------------------------------------------------------------------------ writing: every setting

/// One value for a setting of this unit (different from what the fixture has).
fn sample(unit: Unit) -> i64 {
    match unit {
        Unit::Bool => 1,
        Unit::Radius => 12345,
        Unit::Percent => 137,
        Unit::Ms => 250,
        Unit::Degrees => 15,
        Unit::Choice(c) => c[c.len() - 1].0,
        Unit::Raw => 777,
    }
}

/// A sample value that differs from `was`.
fn other(unit: Unit, was: Option<i64>) -> i64 {
    match unit {
        Unit::Choice(c) => c.iter().rev().map(|x| x.0).find(|v| Some(*v) != was).unwrap(),
        u => sample(u),
    }
}

/// Apply `change`, check the view shows it and only `max_lines` lines changed; then put the old value back and check
/// the file is byte-identical to before.
fn round_trip(key: &str, path: &std::path::Path, set: u32, change: Change, undo_change: Change, max_lines: usize, check: impl Fn(&PadView) -> bool) {
    let mut s = service();
    let before = text(&s, path);
    s.apply(key, EDGE, set, &change).unwrap_or_else(|e| panic!("{change:?}: {e}"));
    let after = text(&s, path);
    let v = s.view(key, EDGE, set).unwrap();
    assert!(check(&v), "{change:?} not shown after writing");
    let (rem, add) = line_diff(&before, &after);
    assert!(rem.len() <= max_lines && add.len() <= max_lines.max(1), "{change:?}: -{rem:?} +{add:?}");
    Layout::parse(after).expect("still a valid layout");
    s.apply(key, EDGE, set, &undo_change).unwrap();
    assert_eq!(text(&s, path), before, "{change:?}: putting the old value back must restore every byte");
}

#[test]
fn every_stick_setting_written_and_put_back_byte_identical() {
    let s = service();
    let v = s.view("252950", EDGE, 0).unwrap();
    for side in [Side::Left, Side::Right] {
        let old = v.sticks.iter().find(|x| x.side == side).unwrap().settings.clone();
        for st in StickSetting::ALL {
            let was = val(&old, st);
            let new = other(st.def().unit, was);
            round_trip(
                "252950",
                &rl_path(),
                0,
                Change::StickSetting { side, setting: st, value: Some(new) },
                Change::StickSetting { side, setting: st, value: was },
                1,
                |v| val(&v.sticks.iter().find(|x| x.side == side).unwrap().settings, st) == Some(new),
            );
        }
    }
}

#[test]
fn every_trigger_setting_written_and_put_back_byte_identical() {
    let s = service();
    let v = s.view("252950", EDGE, 0).unwrap();
    for side in [Side::Left, Side::Right] {
        let old = v.triggers.iter().find(|x| x.side == side).unwrap().settings.clone();
        for st in TriggerSetting::ALL {
            let new = sample(st.def().unit);
            let was = val(&old, st);
            round_trip(
                "252950",
                &rl_path(),
                0,
                Change::TriggerSetting { side, setting: st, value: Some(new) },
                Change::TriggerSetting { side, setting: st, value: was },
                1,
                |v| val(&v.triggers.iter().find(|x| x.side == side).unwrap().settings, st) == Some(new),
            );
        }
        // Analog / Click only
        round_trip(
            "252950",
            &rl_path(),
            0,
            Change::TriggerAnalog { side, analog: false },
            Change::TriggerAnalog { side, analog: true },
            1,
            |v| !v.triggers.iter().find(|x| x.side == side).unwrap().analog,
        );
    }
}

#[test]
fn every_press_setting_of_every_button_written_and_put_back() {
    let s = service();
    let v = s.view("252950", EDGE, 0).unwrap();
    // buttons whose input block exists in the user's file: put back = byte-identical
    for id in [ButtonId::Cross, ButtonId::Circle, ButtonId::Square, ButtonId::Triangle, ButtonId::DpadUp, ButtonId::L1, ButtonId::R1, ButtonId::Create, ButtonId::Options, ButtonId::Mute, ButtonId::BackRightUpper, ButtonId::L3, ButtonId::R3] {
        let old = v.buttons.iter().find(|b| b.id == id).unwrap().settings.clone();
        for ps in PressSetting::ALL.into_iter().filter(|p| *p != PressSetting::ChordButton) {
            let new = sample(ps.def().unit);
            let was = val(&old, ps);
            // a new settings block = 3 lines + the value; an existing block = 1 line
            round_trip(
                "252950",
                &rl_path(),
                0,
                Change::ButtonSetting { button: id, setting: ps, value: Some(new) },
                Change::ButtonSetting { button: id, setting: ps, value: was },
                4,
                |v| val(&v.buttons.iter().find(|b| b.id == id).unwrap().settings, ps) == Some(new),
            );
        }
    }
}

#[test]
fn every_button_action_and_press_kind_written() {
    let actions = [
        Action::Key("G".into()),
        Action::Pad(PadButton::Triangle),
        Action::Mouse(MouseButton::Right),
        Action::WheelUp,
        Action::Steam(bu_controller::SteamAction::ShowKeyboard),
        Action::light_bar_red(),
    ];
    let s0 = service();
    let v = s0.view("252950", EDGE, 0).unwrap();
    for b in &v.buttons {
        if b.fixed {
            continue;
        }
        for (press, was) in b.presses.clone() {
            for a in &actions {
                if *a == was {
                    continue;
                }
                let mut s = service();
                let before = text(&s, &rl_path());
                s.apply("252950", EDGE, 0, &Change::ButtonAction { button: b.id, press, action: a.clone() }).unwrap();
                let v2 = s.view("252950", EDGE, 0).unwrap();
                let got = v2.buttons.iter().find(|x| x.id == b.id).unwrap().presses.iter().find(|(p, _)| *p == press).unwrap().1.clone();
                assert_eq!(&got, a, "{:?} {press:?}", b.id);
                // every other button / press unchanged
                for ob in &v.buttons {
                    if ob.id != b.id {
                        assert_eq!(v2.buttons.iter().find(|x| x.id == ob.id).unwrap(), ob);
                    }
                }
                // back to what it was: byte-identical for EVERY button and press, bound or not (an input that ends
                // up with nothing bound leaves the file again, as Steam keeps unbound inputs out of it)
                s.apply("252950", EDGE, 0, &Change::ButtonAction { button: b.id, press, action: was.clone() }).unwrap();
                assert_eq!(text(&s, &rl_path()), before, "{:?} {press:?} -> {a:?} -> back", b.id);
            }
        }
    }
}

#[test]
fn nothing_removes_the_unbound_input_and_a_combo_becomes_one_binding() {
    let mut s = service();
    s.apply("252950", EDGE, 0, &Change::ButtonAction { button: ButtonId::Circle, press: Press::Full, action: Action::Nothing }).unwrap();
    let t = text(&s, &rl_path());
    assert!(!t.contains("xinput_button B, , "));
    let l = Layout::parse(t).unwrap();
    assert!(!l.has_activator(0, "button_diamond", "button_b", Press::Full), "nothing bound = the input leaves the file (Steam's way)");
    assert!(l.has_activator(0, "button_diamond", "button_a", Press::Full), "the other buttons stay");
    // the community layout's Ctrl + C combo becomes one binding (the copy is the user's own file)
    s.apply("638970", EDGE, 0, &Change::ButtonAction { button: ButtonId::Circle, press: Press::Full, action: Action::Key("X".into()) }).unwrap();
    let v = s.view("638970", EDGE, 0).unwrap();
    let c = v.buttons.iter().find(|b| b.id == ButtonId::Circle).unwrap();
    assert_eq!((c.presses[0].1.clone(), c.extra_bindings), (Action::Key("X".into()), 0));
}

#[test]
fn stick_modes_ring_trigger_actions_touchpad() {
    let mut s = service();
    let before = text(&s, &rl_path());
    for m in StickMode::LISTED {
        s.apply("252950", EDGE, 0, &Change::StickMode { side: Side::Right, mode: m.clone() }).unwrap();
        assert_eq!(s.view("252950", EDGE, 0).unwrap().sticks[1].mode, m);
    }
    s.apply("252950", EDGE, 0, &Change::StickMode { side: Side::Right, mode: StickMode::Joystick }).unwrap();
    assert_eq!(text(&s, &rl_path()), before, "mode back = identical");
    s.apply("252950", EDGE, 0, &Change::StickRing { side: Side::Left, action: Action::Key("LEFT_SHIFT".into()) }).unwrap();
    assert_eq!(s.view("252950", EDGE, 0).unwrap().sticks[0].ring_action, Action::Key("LEFT_SHIFT".into()));
    s.apply("252950", EDGE, 0, &Change::TriggerAction { side: Side::Right, soft: true, action: Action::Mouse(MouseButton::Left) }).unwrap();
    s.apply("252950", EDGE, 0, &Change::TriggerAction { side: Side::Right, soft: false, action: Action::Key("SPACE".into()) }).unwrap();
    let t = s.view("252950", EDGE, 0).unwrap().triggers[1].clone();
    assert_eq!((t.click, t.soft_pull), (Action::Key("SPACE".into()), Action::Mouse(MouseButton::Left)));
    s.apply("252950", EDGE, 0, &Change::TouchMode { mode: TouchMode::Mouse }).unwrap();
    s.apply("252950", EDGE, 0, &Change::TouchClick { half: Side::Left, action: Action::Key("TAB".into()) }).unwrap();
    for ts in TouchSetting::ALL {
        let n = sample(ts.def().unit);
        s.apply("252950", EDGE, 0, &Change::TouchSetting { setting: ts, value: Some(n) }).unwrap();
        assert_eq!(val(&s.view("252950", EDGE, 0).unwrap().touchpad.unwrap().settings, ts), Some(n));
    }
    let tp = s.view("252950", EDGE, 0).unwrap().touchpad.unwrap();
    assert_eq!((tp.touch, tp.left_click), (TouchMode::Mouse, Action::Key("TAB".into())));
    Layout::parse(text(&s, &rl_path())).unwrap();
}

#[test]
fn gyro_on_off_and_every_gyro_setting() {
    // on a layout with no gyro group: "As mouse" creates one (Steam's shape), "Off" unbinds it again
    let mut s = service();
    s.apply("252950", EDGE, 0, &Change::GyroMode { mode: GyroMode::Mouse }).unwrap();
    assert_eq!(s.view("252950", EDGE, 0).unwrap().gyro.unwrap().mode, GyroMode::Mouse);
    for gs in GyroSetting::ALL {
        let n = sample(gs.def().unit);
        s.apply("252950", EDGE, 0, &Change::GyroSetting { setting: gs, value: Some(n) }).unwrap();
        assert_eq!(val(&s.view("252950", EDGE, 0).unwrap().gyro.unwrap().settings, gs), Some(n), "{gs:?}");
    }
    s.apply("252950", EDGE, 0, &Change::GyroMode { mode: GyroMode::Off }).unwrap();
    assert_eq!(s.view("252950", EDGE, 0).unwrap().gyro.unwrap().mode, GyroMode::Off);
    // set 1 of the community layout has the gyro group bound INACTIVE: turning it on reuses that group
    s.apply("638970", EDGE, 1, &Change::GyroMode { mode: GyroMode::Camera }).unwrap();
    let l = Layout::parse(text(&s, &yakuza_own_path())).unwrap();
    assert_eq!(l.active_group_id(1, "gyro").as_deref(), Some("2"));
    assert_eq!(l.count_groups(), 4, "no new group made");
    // an Xbox pad has no gyro / touchpad
    assert!(matches!(s.apply("252950", PadKind::Xbox, 0, &Change::GyroMode { mode: GyroMode::Mouse }), Err(Error::NotOnThisPad("gyro"))));
    assert!(matches!(s.apply("252950", PadKind::DualSense, 0, &Change::ButtonAction { button: ButtonId::BackLeftLower, press: Press::Full, action: Action::Nothing }), Err(Error::NotOnThisPad(_))));
    assert!(matches!(s.apply("252950", EDGE, 0, &Change::ButtonAction { button: ButtonId::Home, press: Press::Full, action: Action::Nothing }), Err(Error::NotOnThisPad(_))), "the PS button stays Steam's");
}

#[test]
fn several_changes_are_one_write_and_one_undo_step() {
    let mut s = service();
    let before = text(&s, &rl_path());
    s.apply_all(
        "252950",
        EDGE,
        0,
        &[
            Change::StickSetting { side: Side::Left, setting: StickSetting::DeadZone, value: Some(pct_to_radius(5.0)) },
            Change::StickSetting { side: Side::Left, setting: StickSetting::FullAt, value: Some(pct_to_radius(90.0)) },
        ],
    )
    .unwrap();
    assert_eq!(s.os().writes().iter().filter(|w| w.contains("controller_ps5.vdf") && !w.contains("backups")).count(), 1);
    assert_eq!(s.undo().unwrap(), "Rocket League · change");
    assert_eq!(text(&s, &rl_path()), before);
    assert!(matches!(s.undo(), Err(Error::NothingToUndo)));
}

#[test]
fn an_unchanged_value_writes_nothing() {
    let mut s = service();
    s.apply("252950", EDGE, 0, &Change::StickSetting { side: Side::Left, setting: StickSetting::DeadZone, value: Some(3357) }).unwrap();
    s.apply("252950", EDGE, 0, &Change::ButtonAction { button: ButtonId::Cross, press: Press::Full, action: Action::Pad(PadButton::Cross) }).unwrap();
    assert!(s.os().writes().is_empty(), "{:?}", s.os().writes());
    assert!(!s.can_undo());
}

#[test]
fn crlf_files_stay_crlf() {
    let f = fake();
    let crlf = RL.replace('\n', "\r\n");
    f.put(rl_path(), &crlf);
    let mut f = f;
    f.active = Some(ACCOUNT.parse().unwrap());
    let mut s = ControllerService::new(f, BACKUPS).unwrap();
    s.apply("252950", EDGE, 0, &Change::ButtonSetting { button: ButtonId::Circle, setting: PressSetting::Toggle, value: Some(1) }).unwrap();
    let t = text(&s, &rl_path());
    assert!(!t.replace("\r\n", "").contains('\n'), "every line ending is still CRLF");
    let (rem, add) = line_diff(&crlf, &t);
    assert!(rem.is_empty() && add.len() == 4, "{add:?}");
}

// ------------------------------------------------------------------------------------------------ backup, undo, resets

#[test]
fn backup_before_the_first_write_and_undo_restores_exactly() {
    let mut s = service();
    let orig = text(&s, &rl_path());
    s.apply("252950", EDGE, 0, &Change::StickSetting { side: Side::Left, setting: StickSetting::DeadZone, value: Some(1000) }).unwrap();
    let backup = std::path::PathBuf::from(BACKUPS).join(ACCOUNT).join(r"steamapps\common\Steam Controller Configs").join(ACCOUNT).join(r"config\252950\controller_ps5.vdf.original");
    assert_eq!(s.os().text(&backup).as_deref(), Some(orig.as_str()), "the original kept before the first write");
    let w = s.os().writes();
    assert!(w[0].ends_with(".original") && w[1].ends_with("controller_ps5.vdf"), "backup first: {w:?}");
    s.apply("252950", EDGE, 0, &Change::StickSetting { side: Side::Left, setting: StickSetting::DeadZone, value: Some(2000) }).unwrap();
    assert_eq!(s.os().text(&backup).as_deref(), Some(orig.as_str()), "the backup is never overwritten");
    assert!(s.has_original("252950", EDGE));
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(text(&s, &rl_path()), orig);
}

#[test]
fn undo_refuses_when_steam_changed_the_file_since() {
    let mut s = service();
    s.apply("252950", EDGE, 0, &Change::StickSetting { side: Side::Left, setting: StickSetting::DeadZone, value: Some(1000) }).unwrap();
    let t = text(&s, &rl_path()).replace("\"revision\"\t\t\"57\"","\"revision\"\t\t\"149\"");
    s.os().put(rl_path(), &t); // Steam saved its own copy meanwhile
    assert!(matches!(s.undo(), Err(Error::ChangedOutside(_))));
    assert_eq!(text(&s, &rl_path()), t, "nothing overwritten");
    // a new change starts from Steam's newer file (re-read before every write)
    s.apply("252950", EDGE, 0, &Change::StickSetting { side: Side::Left, setting: StickSetting::FullAt, value: Some(30000) }).unwrap();
    assert!(text(&s, &rl_path()).contains("\"revision\"\t\t\"149\""));
}

#[test]
fn back_to_how_it_was_and_steams_layout() {
    let mut s = service();
    let orig = text(&s, &rl_path());
    assert!(matches!(s.back_to_original("252950", EDGE), Err(Error::NoBackup)));
    s.apply("252950", EDGE, 0, &Change::StickSetting { side: Side::Left, setting: StickSetting::DeadZone, value: Some(1000) }).unwrap();
    s.apply("252950", EDGE, 0, &Change::ButtonAction { button: ButtonId::Cross, press: Press::Full, action: Action::Key("Q".into()) }).unwrap();
    s.back_to_original("252950", EDGE).unwrap();
    assert_eq!(text(&s, &rl_path()), orig);
    // the reset is one undo step itself
    s.undo().unwrap();
    assert!(text(&s, &rl_path()).contains("key_press Q, , "));

    // "Steam's setting for this": one part back to the official layout, the rest stays the user's
    let mut s = service();
    s.part_to_steam("252950", EDGE, 0, Part::Stick(Side::Left)).unwrap();
    let v = s.view("252950", EDGE, 0).unwrap();
    let off = Layout::parse(RL_OFFICIAL).unwrap().pad_view(0, EDGE);
    assert_eq!(v.sticks[0], off.sticks[0]);
    assert_eq!(v.buttons.iter().find(|b| b.id == ButtonId::BackRightUpper).unwrap().presses[0].1, Action::Key("F5".into()), "R4 still the user's");
    s.part_to_steam("252950", EDGE, 0, Part::Button(ButtonId::BackRightUpper)).unwrap();
    s.part_to_steam("252950", EDGE, 0, Part::Touchpad).unwrap();
    s.part_to_steam("252950", EDGE, 0, Part::Stick(Side::Right)).unwrap();
    assert!(s.changed_parts("252950", EDGE, 0).unwrap().is_empty(), "every part back = nothing differs");

    // "Steam's layout": the whole file = Steam's text + the user's url / progenitor
    let mut s = service();
    s.layout_to_steam("252950", EDGE).unwrap();
    let t = text(&s, &rl_path());
    let l = Layout::parse(t.clone()).unwrap();
    assert_eq!(l.header().progenitor, "workshop://1700935741");
    assert!(l.header().url.starts_with("autosave://"));
    assert_eq!(l.pad_view(0, EDGE), off);
    s.undo().unwrap();
    assert_eq!(text(&s, &rl_path()), orig);
}

// ------------------------------------------------------------------------------------------------ community layouts

#[test]
fn first_change_of_a_community_layout_makes_the_users_own_copy() {
    let mut s = service();
    let cs_before = text(&s, &configset_path());
    s.apply("638970", EDGE, 0, &Change::StickSetting { side: Side::Right, setting: StickSetting::Sensitivity, value: Some(200) }).unwrap();
    assert_eq!(s.os().text(community_path()).as_deref(), Some(COMMUNITY), "the downloaded layout is never changed");
    let own = Layout::parse(text(&s, &yakuza_own_path())).unwrap();
    assert!(own.header().url.ends_with(r"config\638970\controller_ps5.vdf"));
    assert_eq!(own.header().progenitor, "workshop://3275392801");
    let cs = text(&s, &configset_path());
    let (rem, add) = line_diff(&cs_before, &cs);
    assert_eq!(rem, vec!["\t\t\"workshop\"\t\t\"3275392801\"\n".to_string()]);
    assert_eq!(add, vec!["\t\t\"autosave\"\t\t\"1\"\n".to_string()]);
    let g = s.game("638970", EDGE).unwrap();
    assert_eq!(g.source, LayoutSource::Autosave);
    // the order of writes: the layout first, then the index (it never points at a missing file)
    let w: Vec<String> = s.os().writes().into_iter().filter(|w| !w.contains(r"\BU\")).collect();
    assert!(w[0].ends_with(r"638970\controller_ps5.vdf") && w[1].ends_with("configset_controller_ps5.vdf"), "{w:?}");
    // its changed parts compare against the community layout it came from
    assert_eq!(s.changed_parts("638970", EDGE, 0).unwrap(), vec![Part::Stick(Side::Right)]);
    // undo: the copy goes, the index is back
    s.undo().unwrap();
    assert!(s.os().get(yakuza_own_path()).is_none());
    assert_eq!(text(&s, &configset_path()), cs_before);
}

#[test]
fn new_and_renamed_action_sets() {
    let mut s = service();
    let id = s.add_action_set("252950", EDGE, 0, "Driving").unwrap();
    assert_eq!(id, 1);
    let sets = s.action_sets("252950", EDGE).unwrap();
    assert_eq!(sets.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), ["Default", "Driving"]);
    assert_eq!(s.view("252950", EDGE, 1).unwrap().sticks, s.view("252950", EDGE, 0).unwrap().sticks, "a copy of the Default set");
    // the copy is independent
    s.apply("252950", EDGE, 1, &Change::StickSetting { side: Side::Left, setting: StickSetting::DeadZone, value: Some(9000) }).unwrap();
    assert_eq!(val(&s.view("252950", EDGE, 0).unwrap().sticks[0].settings, StickSetting::DeadZone), Some(3357));
    s.rename_action_set("252950", EDGE, 1, "On the road").unwrap();
    assert_eq!(s.action_sets("252950", EDGE).unwrap()[1].title, "On the road");
    let id2 = s.add_action_set("638970", EDGE, 1, "Third").unwrap();
    assert_eq!(id2, 2);
    assert_eq!(s.action_sets("638970", EDGE).unwrap().len(), 3);
}

// ------------------------------------------------------------------------------------------------ this controller (all games)

#[test]
fn per_controller_preferences_read_write_undo() {
    let mut s = service();
    let p = s.preferences().unwrap();
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].serial, SERIAL);
    assert_eq!(p[0].name, "DualSense Edge Wireless Controller");
    assert_eq!(p[0].led(), Some((10, 20, 30)));
    assert_eq!(p[0].stick_deadzone_pct(PrefSetting::LeftStickDeadZone), None, "-1 = Steam's default");
    for (setting, v) in [
        (PrefSetting::LeftStickDeadZone, "3000"),
        (PrefSetting::RightStickDeadZone, "2500"),
        (PrefSetting::AntiDrift, "1"),
        (PrefSetting::GyroNoiseFilter, "1"),
        (PrefSetting::Rumble, "0"),
        (PrefSetting::LedBrightness, "0.8"),
    ] {
        let before = text(&s, &prefs_path());
        s.set_preference(SERIAL, setting, Some(v)).unwrap();
        let (rem, add) = line_diff(&before, &text(&s, &prefs_path()));
        assert_eq!((rem.len(), add.len()), (1, 1), "{setting:?}");
        assert_eq!(s.preferences().unwrap()[0].get(setting), Some(v));
    }
    s.set_light_bar(SERIAL, (255, 0, 0)).unwrap();
    assert_eq!(s.light_bar(SERIAL).unwrap(), Some((255, 0, 0)));
    assert_eq!(s.undo_label(), Some("Controller settings · light bar colour"));
    s.undo().unwrap();
    assert_eq!(s.light_bar(SERIAL).unwrap(), Some((10, 20, 30)), "one undo = all three colour values");
    s.preferences_to_original(SERIAL).unwrap();
    assert_eq!(text(&s, &prefs_path()), PREFS);
    assert!(matches!(s.set_preference("nope", PrefSetting::Rumble, Some("1")), Err(Error::LayoutMissing(_))));
}

// ------------------------------------------------------------------------------------------------ safety

#[test]
fn a_read_only_layer_refuses_and_changes_nothing() {
    let mut f = fake();
    f.active = Some(ACCOUNT.parse().unwrap());
    f.read_only = true;
    let mut s = ControllerService::new(f, BACKUPS).unwrap();
    let r = s.apply("252950", EDGE, 0, &Change::StickSetting { side: Side::Left, setting: StickSetting::DeadZone, value: Some(1) });
    assert!(matches!(r, Err(Error::ReadOnly(_))), "{r:?}");
    assert_eq!(text(&s, &rl_path()), RL);
    assert!(s.os().writes().is_empty());
}

#[test]
fn open_in_steam_only_while_steam_runs() {
    let mut f = fake();
    f.active = Some(ACCOUNT.parse().unwrap());
    let s = ControllerService::new(f, BACKUPS).unwrap();
    let g = s.game("252950", EDGE).unwrap();
    assert!(matches!(s.open_in_steam_link(&g), Err(Error::SteamClosed)), "a steam:// link would start Steam");
    let mut f = fake();
    f.active = Some(ACCOUNT.parse().unwrap());
    f.running = true;
    let s = ControllerService::new(f, BACKUPS).unwrap();
    assert_eq!(s.open_in_steam_link(&g).unwrap(), "steam://controllerconfig/252950");
}

#[test]
fn a_broken_layout_is_refused_not_written() {
    let mut f = fake();
    f.active = Some(ACCOUNT.parse().unwrap());
    f.put(rl_path(), "\"controller_mappings\"\n{\n\t\"version\"\t\t\"3\"\n");
    let mut s = ControllerService::new(f, BACKUPS).unwrap();
    assert!(matches!(s.view("252950", EDGE, 0), Err(Error::Layout { .. })));
    assert!(s.apply("252950", EDGE, 0, &Change::TriggerAnalog { side: Side::Left, analog: false }).is_err());
    assert!(s.os().writes().is_empty());
}

// ------------------------------------------------------------------------------------------------ controllers + live

fn edge_pad() -> PadInfo {
    PadInfo {
        kind: EDGE,
        name: "DualSense Edge Wireless Controller".into(),
        connection: Connection::Usb,
        battery: None,
        source: PadSource::Hid(r"\\?\hid#vid_054c&pid_0df2#1".into()),
    }
}

fn ds_report(lx: u8, cross: bool) -> Vec<u8> {
    let mut r = vec![0u8; 64];
    r[0] = 0x01;
    r[1] = lx;
    r[2] = 0x80;
    r[3] = 0x80;
    r[4] = 0x80;
    r[8] = 0x08 | if cross { 0x20 } else { 0 };
    r[53] = 0x07;
    r
}

#[test]
fn controller_types_from_usb_ids() {
    assert_eq!(PadKind::from_ids(0x054C, 0x0DF2), Some(PadKind::DualSenseEdge));
    assert_eq!(PadKind::from_ids(0x054C, 0x0CE6), Some(PadKind::DualSense));
    assert_eq!(PadKind::from_ids(0x054C, 0x09CC), Some(PadKind::DualShock4));
    assert_eq!(PadKind::from_ids(0x045E, 0x0B12), Some(PadKind::Xbox));
    assert_eq!(PadKind::from_ids(0x28DE, 0x11FF), None, "Steam's virtual pad is not a controller of the user's");
    assert_eq!(PadKind::DualSenseEdge.layout_type(), "ps5");
    assert_eq!(PadKind::Xbox.layout_type(), "xboxone");
    let pads = FakePads { pads: vec![edge_pad()], ..Default::default() };
    assert_eq!(pads.list_pads().unwrap()[0].kind, EDGE);
}

#[test]
fn live_view_runs_only_while_open_and_reports_changes() {
    let pads = FakePads { pads: vec![edge_pad()], ..Default::default() };
    pads.script.lock().unwrap().extend([
        LiveEvent::Report(ds_report(0x80, false)),
        LiveEvent::Report(ds_report(0x80, false)), // same state: no change call
        LiveEvent::Report(ds_report(0xFF, true)),
    ]);
    let changes = Arc::new(AtomicUsize::new(0));
    let c2 = changes.clone();
    let view = LiveView::start(&pads, &edge_pad(), Some(Box::new(move |_| {
        c2.fetch_add(1, Ordering::SeqCst);
    })))
    .unwrap();
    assert_eq!(*pads.open_now.lock().unwrap(), 1);
    let t0 = std::time::Instant::now();
    while view.reports() < 3 && t0.elapsed().as_secs() < 5 {
        std::thread::yield_now();
    }
    let s = view.latest().unwrap();
    assert!(s.is_pressed(ButtonId::Cross));
    assert!((s.left.0 - 1.0).abs() < 0.01);
    assert_eq!(s.battery.unwrap().percent, Some(75));
    assert_eq!(changes.load(Ordering::SeqCst), 2, "the repeated report changed nothing");
    view.stop();
    assert_eq!(*pads.open_now.lock().unwrap(), 0, "page closed = the device is closed, no thread left");
    // dropping also stops
    let v2 = LiveView::start(&pads, &edge_pad(), None).unwrap();
    drop(v2);
    assert_eq!(*pads.open_now.lock().unwrap(), 0);
}

#[test]
fn an_unplugged_controller_ends_the_live_view() {
    let pads = FakePads { pads: vec![edge_pad()], ..Default::default() };
    pads.script.lock().unwrap().push_back(LiveEvent::Gone);
    let v = LiveView::start(&pads, &edge_pad(), None).unwrap();
    let t0 = std::time::Instant::now();
    while !v.is_gone() && t0.elapsed().as_secs() < 5 {
        std::thread::yield_now();
    }
    assert!(v.is_gone());
    let mut other = edge_pad();
    other.name = "unplugged".into();
    other.source = PadSource::Hid("x".into());
    assert!(matches!(LiveView::start(&pads, &other, None), Err(Error::PadGone(_))));
}

// ------------------------------------------------------------------------------------------------ REVIEW 015 fixes

#[test]
fn every_gyro_and_touchpad_setting_put_back_byte_identical() {
    // gyro: the community layout's set 0 has an active gyro group (after its own copy exists)
    let mut s0 = service();
    s0.apply("638970", EDGE, 0, &Change::StickSetting { side: Side::Right, setting: StickSetting::Sensitivity, value: Some(151) }).unwrap();
    let base = s0.os().text(yakuza_own_path()).unwrap();
    let old = s0.view("638970", EDGE, 0).unwrap().gyro.unwrap().settings;
    for gs in GyroSetting::ALL {
        let was = val(&old, gs);
        let new = other(gs.def().unit, was);
        s0.apply("638970", EDGE, 0, &Change::GyroSetting { setting: gs, value: Some(new) }).unwrap();
        assert_eq!(val(&s0.view("638970", EDGE, 0).unwrap().gyro.unwrap().settings, gs), Some(new));
        s0.apply("638970", EDGE, 0, &Change::GyroSetting { setting: gs, value: was }).unwrap();
        assert_eq!(s0.os().text(yakuza_own_path()).unwrap(), base, "{gs:?}");
    }
    // touchpad: the user's RL right half has no settings block yet; set + put back removes it again
    let old = service().view("252950", EDGE, 0).unwrap().touchpad.unwrap().settings;
    for ts in TouchSetting::ALL {
        let was = val(&old, ts);
        round_trip(
            "252950",
            &rl_path(),
            0,
            Change::TouchSetting { setting: ts, value: Some(other(ts.def().unit, was)) },
            Change::TouchSetting { setting: ts, value: was },
            4,
            |v| val(&v.touchpad.as_ref().unwrap().settings, ts).is_some(),
        );
    }
    for m in [TouchMode::Mouse, TouchMode::Scroll] {
        round_trip("252950", &rl_path(), 0, Change::TouchMode { mode: m.clone() }, Change::TouchMode { mode: TouchMode::Nothing }, 1, |v| v.touchpad.as_ref().unwrap().touch == m);
    }
}

#[test]
fn back_to_how_it_was_for_one_game_keeps_every_other_games_entry() {
    let mut s = service();
    let cs0 = text(&s, &configset_path());
    // 1. Yakuza becomes the user's own copy (its configset entry changes)
    s.apply("638970", EDGE, 0, &Change::StickSetting { side: Side::Right, setting: StickSetting::Sensitivity, value: Some(200) }).unwrap();
    let cs_yakuza = text(&s, &configset_path());
    // 2. Rocket League changed afterwards
    s.apply("252950", EDGE, 0, &Change::StickSetting { side: Side::Left, setting: StickSetting::DeadZone, value: Some(1000) }).unwrap();
    // 3. "Back to how your PC was" for Rocket League: its layout back, the index untouched (Yakuza stays its own copy)
    s.back_to_original("252950", EDGE).unwrap();
    assert_eq!(text(&s, &rl_path()), RL);
    assert_eq!(text(&s, &configset_path()), cs_yakuza, "RL never changed its entry: the index is not touched");
    assert_eq!(s.game("638970", EDGE).unwrap().source, LayoutSource::Autosave);
    // 4. a game the app never touched: nothing to go back to, nothing written
    let n = s.os().writes().len();
    assert!(matches!(s.back_to_original("epic games launcher", EDGE), Err(Error::NoBackup)));
    assert_eq!(s.os().writes().len(), n);
    // 5. Steam changes another game's entry meanwhile; then Yakuza goes back: only Yakuza's lines change
    let steam_edit = text(&s, &configset_path()).replace("\"template\"\t\t\"controller_ps5_wasd.vdf\"", "\"template\"\t\t\"controller_ps5_fps.vdf\"");
    s.os().put(configset_path(), &steam_edit);
    s.back_to_original("638970", EDGE).unwrap();
    let cs = text(&s, &configset_path());
    let (rem, add) = line_diff(&steam_edit, &cs);
    assert_eq!(rem, vec!["\t\t\"autosave\"\t\t\"1\"\n".to_string()]);
    assert_eq!(add, vec!["\t\t\"workshop\"\t\t\"3275392801\"\n".to_string()]);
    assert!(cs.contains("controller_ps5_fps.vdf"), "Steam's own change elsewhere is kept");
    assert!(s.os().get(yakuza_own_path()).is_none(), "the app's own copy is gone again");
    assert_eq!(s.game("638970", EDGE).unwrap().source, LayoutSource::Workshop("3275392801".into()));
    let _ = cs0;
}

#[test]
fn a_file_that_is_not_utf8_is_refused_and_not_written() {
    let mut f = fake();
    f.active = Some(ACCOUNT.parse().unwrap());
    let mut bytes = RL.as_bytes().to_vec();
    let i = RL.find("Official Psyonix").unwrap();
    bytes[i] = 0xE9; // one Latin-1 byte in the title
    f.put(rl_path(), &bytes);
    let mut s = ControllerService::new(f, BACKUPS).unwrap();
    assert!(s.view("252950", EDGE, 0).is_ok(), "it can still be shown");
    let r = s.apply("252950", EDGE, 0, &Change::StickSetting { side: Side::Left, setting: StickSetting::DeadZone, value: Some(1) });
    assert!(matches!(r, Err(Error::NotUtf8(_))), "{r:?}");
    assert_eq!(s.os().get(rl_path()).unwrap(), bytes);
    assert!(s.os().writes().is_empty());
    // the same for the shared index and the per-controller file
    let mut f = fake();
    f.active = Some(ACCOUNT.parse().unwrap());
    let mut cs = CONFIGSET.as_bytes().to_vec();
    let j = CONFIGSET.find("games launcher").unwrap();
    cs[j + 4] = 0xE9; // a Latin-1 byte in another game's key ("games" -> "gam?s")
    f.put(configset_path(), &cs);
    let mut pb = PREFS.as_bytes().to_vec();
    pb.insert(5, 0xE9);
    f.put(prefs_path(), &pb);
    let mut s = ControllerService::new(f, BACKUPS).unwrap();
    assert!(matches!(s.apply("638970", EDGE, 0, &Change::TriggerAnalog { side: Side::Left, analog: false }), Err(Error::NotUtf8(_))));
    assert!(matches!(s.set_preference(SERIAL, PrefSetting::Rumble, Some("1")), Err(Error::NotUtf8(_))));
    assert!(s.os().writes().is_empty());
}

#[test]
fn a_template_layout_gets_its_progenitor_in_the_own_copy() {
    let mut f = fake();
    f.active = Some(ACCOUNT.parse().unwrap());
    f.put(configset_path(), CONFIGSET.replace("\t\"epic games launcher\"", "\t\"730\"\n\t{\n\t\t\"template\"\t\t\"controller_ps5_gamepad_joystick.vdf\"\n\t}\n\t\"epic games launcher\""));
    f.put(r"C:\Steam\controller_base\templates\controller_ps5_gamepad_joystick.vdf", RL_OFFICIAL);
    let mut s = ControllerService::new(f, BACKUPS).unwrap();
    s.apply("730", EDGE, 0, &Change::StickSetting { side: Side::Left, setting: StickSetting::DeadZone, value: Some(2000) }).unwrap();
    let own = Layout::parse(text(&s, &config().join(r"730\controller_ps5.vdf"))).unwrap();
    assert_eq!(own.header().progenitor, "template://controller_ps5_gamepad_joystick.vdf");
    assert_eq!(s.changed_parts("730", EDGE, 0).unwrap(), vec![Part::Stick(Side::Left)], "Steam's layout is found again");
}

#[test]
fn a_failed_second_write_rolls_the_first_back() {
    let mut f = fake();
    f.active = Some(ACCOUNT.parse().unwrap());
    *f.fail_on.lock().unwrap() = Some("configset_controller_ps5.vdf".into());
    let mut s = ControllerService::new(f, BACKUPS).unwrap();
    // backups of the index are written first and must not fail: let only the real index fail
    let r = s.apply("638970", EDGE, 0, &Change::StickSetting { side: Side::Right, setting: StickSetting::Sensitivity, value: Some(200) });
    assert!(r.is_err());
    assert!(s.os().get(yakuza_own_path()).is_none(), "the own copy written first was taken back");
    assert_eq!(text(&s, &configset_path()), CONFIGSET);
    assert!(!s.can_undo());
}

// ------------------------------------------------------------------------------------------------ REVIEW 015 (second) fixes

/// Two non-Steam shortcuts whose titles have the same length in Cyrillic (Steam keys them by their lower-case title), both
/// on the community layout: once a lossy name would have given them ONE entry backup.
fn two_shortcuts() -> ControllerService<FakeSteam> {
    let mut f = fake();
    f.active = Some(ACCOUNT.parse().unwrap());
    let cs = CONFIGSET.replace(
        "\t\"epic games launcher\"",
        "\t\"игра\"\n\t{\n\t\t\"workshop\"\t\t\"3275392801\"\n\t}\n\t\"мода\"\n\t{\n\t\t\"workshop\"\t\t\"3275392801\"\n\t}\n\t\"epic games launcher\"",
    );
    f.put(configset_path(), cs);
    ControllerService::new(f, BACKUPS).unwrap()
}

#[test]
fn same_length_non_ascii_titles_keep_their_own_entry_backups() {
    let mut s = two_shortcuts();
    let ch = Change::StickSetting { side: Side::Right, setting: StickSetting::Sensitivity, value: Some(200) };
    s.apply("игра", EDGE, 0, &ch).unwrap();
    s.apply("мода", EDGE, 0, &ch).unwrap();
    let before = text(&s, &configset_path());
    s.back_to_original("мода", EDGE).unwrap();
    let after = text(&s, &configset_path());
    let (rem, add) = line_diff(&before, &after);
    assert_eq!(rem, vec!["\t\t\"autosave\"\t\t\"1\"\n".to_string()], "only мода's line");
    assert_eq!(add, vec!["\t\t\"workshop\"\t\t\"3275392801\"\n".to_string()]);
    assert_eq!(after.matches("\"игра\"").count(), 1, "the other game's entry stays, once");
    assert_eq!(s.game("игра", EDGE).unwrap().source, LayoutSource::Autosave, "игра keeps its own copy");
    assert_eq!(s.game("мода", EDGE).unwrap().source, LayoutSource::Workshop("3275392801".into()));
    assert!(s.os().get(config().join(r"игра\controller_ps5.vdf")).is_some());
    assert!(s.os().get(config().join(r"мода\controller_ps5.vdf")).is_none());
}

#[test]
fn an_entry_backup_of_another_game_is_refused() {
    let mut s = service();
    s.apply("638970", EDGE, 0, &Change::StickSetting { side: Side::Right, setting: StickSetting::Sensitivity, value: Some(200) }).unwrap();
    // someone / something put another game's entry into Yakuza's entry backup
    let hex: String = "638970".bytes().map(|b| format!("{b:02x}")).collect();
    let bk = std::path::PathBuf::from(BACKUPS).join(ACCOUNT).join(r"steamapps\common\Steam Controller Configs").join(ACCOUNT).join(format!(r"config\configset_controller_ps5.vdf.entry-{hex}.original"));
    assert!(s.os().get(&bk).is_some(), "the entry backup is where the test expects it");
    s.os().put(&bk, "\t\"252950\"\n\t{\n\t\t\"autosave\"\t\t\"1\"\n\t}\n");
    let n = s.os().writes().len();
    assert!(matches!(s.back_to_original("638970", EDGE), Err(Error::BackupMismatch(_))));
    assert_eq!(s.os().writes().len(), n, "nothing written");
}

#[test]
fn steams_layout_with_a_non_utf8_byte_is_not_written_into_the_users_file() {
    let mut f = fake();
    f.active = Some(ACCOUNT.parse().unwrap());
    let mut off = RL_OFFICIAL.as_bytes().to_vec();
    let i = RL_OFFICIAL.find("Official").unwrap();
    off[i] = 0xE9;
    f.put(official_path(), &off);
    let mut s = ControllerService::new(f, BACKUPS).unwrap();
    assert!(s.changed_parts("252950", EDGE, 0).is_ok(), "it can still be compared (read-only)");
    assert!(matches!(s.layout_to_steam("252950", EDGE), Err(Error::NotUtf8(_))));
    assert_eq!(text(&s, &rl_path()), RL);
    assert!(s.os().writes().is_empty());
}

#[test]
fn a_failed_reset_puts_back_what_it_already_changed() {
    let mut s = service();
    s.apply("638970", EDGE, 0, &Change::StickSetting { side: Side::Right, setting: StickSetting::Sensitivity, value: Some(200) }).unwrap();
    let cs = text(&s, &configset_path());
    let own = s.os().get(yakuza_own_path()).unwrap();
    // the index is written first, then the own copy is removed; make that removal fail
    *s.os().fail_remove.lock().unwrap() = Some(r"638970\controller_ps5.vdf".into());
    assert!(s.back_to_original("638970", EDGE).is_err());
    assert_eq!(text(&s, &configset_path()), cs, "the index change was put back");
    assert_eq!(s.os().get(yakuza_own_path()).unwrap(), own);
}

/// Order 036 (the app's change log): "as it was" / "Steam's layout" / the controller's file as it was, read from the files
/// (so a reset line whose value is already back disappears); every reset puts the bytes back from the kept backups.
#[test]
fn the_change_log_reads_whether_a_game_is_as_it_was_or_steams() {
    let mut s = service();
    // nothing changed by the app: as it was; the user's own edits differ from Steam's layout
    assert!(s.is_original("252950", EDGE).unwrap());
    assert!(!s.is_steam_layout("252950", EDGE).unwrap());
    // a community layout used directly is Steam's (nothing to write)
    assert!(s.is_steam_layout("638970", EDGE).unwrap());
    s.apply("252950", EDGE, 0, &Change::StickSetting { side: Side::Left, setting: StickSetting::DeadZone, value: Some(1000) }).unwrap();
    assert!(!s.is_original("252950", EDGE).unwrap());
    s.layout_to_steam("252950", EDGE).unwrap();
    assert!(s.is_steam_layout("252950", EDGE).unwrap());
    assert!(!s.is_original("252950", EDGE).unwrap());
    s.back_to_original("252950", EDGE).unwrap();
    assert!(s.is_original("252950", EDGE).unwrap());
    assert_eq!(text(&s, &rl_path()), RL);
    // a community layout made the user's own copy: its index entry counts too
    s.apply("638970", EDGE, 0, &Change::StickSetting { side: Side::Right, setting: StickSetting::Sensitivity, value: Some(200) }).unwrap();
    assert!(!s.is_original("638970", EDGE).unwrap());
    assert!(!s.is_steam_layout("638970", EDGE).unwrap() || s.game("638970", EDGE).unwrap().source == LayoutSource::Autosave);
    s.back_to_original("638970", EDGE).unwrap();
    assert!(s.is_original("638970", EDGE).unwrap());
    // the controller's own file
    assert!(s.preferences_are_original(SERIAL).unwrap() && !s.has_preferences_original(SERIAL));
    s.set_preference(SERIAL, PrefSetting::Rumble, Some("0")).unwrap();
    assert!(!s.preferences_are_original(SERIAL).unwrap() && s.has_preferences_original(SERIAL));
    s.preferences_to_original(SERIAL).unwrap();
    assert!(s.preferences_are_original(SERIAL).unwrap());
}

/// Order 047: the game names (every appmanifest, the shortcuts, localconfig.vdf) are read once and serve every
/// question after it - a page view or a write asked for the game list 4-5 times; the configset is read fresh each time,
/// so which layout a game uses is never stale; `forget_names` reads them again.
#[test]
fn the_game_names_are_read_once_per_change() {
    let s = service();
    let reads = |s: &ControllerService<FakeSteam>| s.os().reads.load(Ordering::Relaxed);
    let r0 = reads(&s);
    let first = s.games(EDGE).unwrap();
    let full = reads(&s) - r0;
    let r1 = reads(&s);
    let o = s.open("252950", EDGE).unwrap();
    let _ = s.steam_layout_of(&o);
    assert_eq!(s.games(EDGE).unwrap(), first);
    let again = reads(&s) - r1;
    assert!(again < full * 2, "names read once: {again} reads for open + Steam's layout + the list, one list alone was {full}");
    // Steam switches a game (Yakuza) to another layout: seen at once (the configset is never kept)
    let cs = text(&s, &configset_path()).replace("\"workshop\"\t\t\"3275392801\"", "\"template\"\t\t\"controller_ps5_fps.vdf\"");
    s.os().put(configset_path(), &cs);
    assert!(s.games(EDGE).unwrap().iter().any(|g| g.source == LayoutSource::Template("controller_ps5_fps.vdf".into())));
    // forgotten: the whole list is read again
    s.forget_names();
    let r2 = reads(&s);
    let _ = s.games(EDGE).unwrap();
    assert_eq!(reads(&s) - r2, full);
}
