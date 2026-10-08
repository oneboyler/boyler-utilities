//! Modifier-only keys and mouse buttons 3 / 4 / 5 (Raw Input, boss A_014_01) — fake OS only: nothing real is
//! registered, no input is ever sent.

use super::fake::*;
use super::raw::*;
use super::*;
use crate::settings::scratch::Scratch;

const VK_M: u16 = 0x4D;
const VK_F8: u16 = 0x77;

fn cs(vk: u16) -> Combo {
    Combo::new(Mods::CTRL.with(Mods::SHIFT), vk)
}
fn ev(vk: u16, mods: Mods) -> KeyEvent {
    KeyEvent::new(vk, mods)
}
fn kd(vk: u16) -> Packet {
    Packet::Key { vk, make: if vk == VK_SHIFT { 0x2A } else { 0 }, flags: 0 }
}
fn ku(vk: u16) -> Packet {
    Packet::Key { vk, make: if vk == VK_SHIFT { 0x2A } else { 0 }, flags: RI_KEY_BREAK }
}
fn btn(buttons: u16) -> Packet {
    Packet::Mouse { buttons }
}

fn raw_manager(st: &SettingsStore, os: FakeKeysOs) -> KeysManager<FakeKeysOs> {
    let mut m = KeysManager::new(os, st);
    for (id, name) in [("a", "Mic mute"), ("b", "Screenshot"), ("c", "Search")] {
        m.add_action(Action::new(id, name, "p"));
    }
    m.add_action(Action::new("talk", "Voice to text", "voice").with_release());
    m
}

fn fired(m: &mut KeysManager<FakeKeysOs>, packets: &[Packet]) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for p in packets {
        m.on_raw(*p, |id, down| out.push((id.to_string(), down)));
    }
    out
}

#[test]
fn capture_modifier_only_commits_when_the_last_modifier_goes_up() {
    let mut c = Capture::new();
    let ctrl_shift = Mods::CTRL.with(Mods::SHIFT);
    assert_eq!(c.key_down(ev(VK_LCONTROL, Mods::CTRL)), Step::Listening { held: Mods::CTRL });
    assert_eq!(c.key_down(ev(VK_LSHIFT, ctrl_shift)), Step::Listening { held: ctrl_shift });
    assert_eq!(c.key_up(ev(VK_LSHIFT, ctrl_shift)), Step::Listening { held: Mods::CTRL });
    assert_eq!(c.key_up(ev(VK_LCONTROL, Mods::CTRL)), Step::Done(Combo::mods_only(ctrl_shift)));
    // a real key in between = a normal combo, never modifiers only
    let mut c = Capture::new();
    c.key_down(ev(VK_LCONTROL, Mods::CTRL));
    assert_eq!(c.key_down(ev(VK_M, Mods::CTRL)), Step::Done(Combo::new(Mods::CTRL, VK_M)));
    assert_eq!(c.key_up(ev(VK_LCONTROL, Mods::CTRL)), Step::Ignored);
    // a numpad refusal forgets the modifiers held so far
    let mut c = Capture::new();
    c.key_down(ev(VK_LCONTROL, Mods::CTRL));
    assert_eq!(c.key_down(ev(0x61, Mods::CTRL)), Step::Refused(BindError::Numpad));
    assert_eq!(c.key_up(ev(VK_LCONTROL, Mods::CTRL)), Step::Listening { held: Mods::NONE });
}

#[test]
fn capture_mouse_buttons_3_4_5_and_their_names() {
    let mut c = Capture::new();
    assert_eq!(c.mouse_down(1, Mods::NONE), Step::Ignored);
    assert_eq!(c.mouse_down(2, Mods::NONE), Step::Ignored);
    assert_eq!(c.mouse_down(4, Mods::ALT), Step::Done(Combo::mouse(Mods::ALT, 4)));
    assert_eq!(c.mouse_down(5, Mods::NONE), Step::Ignored, "already ended");
    let s = Scratch::new("keys-mnames");
    let st = SettingsStore::open(s.dir());
    let m = raw_manager(&st, FakeKeysOs::new());
    assert_eq!(m.combo_text(Combo::mouse(Mods::NONE, 3)), "Mouse 3");
    assert_eq!(m.combo_text(Combo::mouse(Mods::ALT, 4)), "Alt + Mouse 4");
    assert_eq!(m.combo_text(Combo::mouse(Mods::CTRL.with(Mods::SHIFT), 5)), "Ctrl + Shift + Mouse 5");
    assert_eq!(m.combo_text(Combo::mods_only(Mods::SHIFT.with(Mods::CTRL))), "Ctrl + Shift");
}

#[test]
fn raw_input_is_registered_only_while_needed() {
    let s = Scratch::new("keys-rawneed");
    let mut st = SettingsStore::open(s.dir());
    let mut m = raw_manager(&st, FakeKeysOs::new());
    m.bind(&mut st, "c", cs(VK_M)).unwrap();
    assert_eq!(m.os().raw, (false, false), "a normal key needs no Raw Input");
    m.bind(&mut st, "a", Combo::mouse(Mods::NONE, 4)).unwrap();
    assert_eq!(m.os().raw, (false, true), "a mouse key: raw mouse only");
    m.bind(&mut st, "b", Combo::mods_only(Mods::CTRL.with(Mods::SHIFT))).unwrap();
    assert_eq!(m.os().raw, (true, true), "modifier-only: keyboard + mouse (a click cancels it)");
    m.unbind(&mut st, "a").unwrap();
    assert_eq!(m.os().raw, (true, true));
    m.unbind(&mut st, "b").unwrap();
    assert_eq!(m.os().raw, (false, false), "the last one went: everything removed");
    m.bind(&mut st, "talk", Combo::new(Mods::NONE, VK_F8)).unwrap();
    assert_eq!(m.os().raw, (true, false), "a release key: raw keyboard only");
    m.bind(&mut st, "a", Combo::mouse(Mods::ALT, 5)).unwrap();
    drop(m);
    // a restart brings them back from the file, with one registration call
    let m = raw_manager(&st, FakeKeysOs::new());
    assert_eq!(m.state("a"), KeyState::Working(Combo::mouse(Mods::ALT, 5)));
    assert_eq!(m.os().raw, (true, true));
    assert_eq!(m.os().log.iter().filter(|l| l.starts_with("raw")).count(), 2, "{:?}", m.os().log);
}

#[test]
fn doubles_refused_across_kinds() {
    let s = Scratch::new("keys-rawdouble");
    let mut st = SettingsStore::open(s.dir());
    let mut m = raw_manager(&st, FakeKeysOs::new());
    m.bind(&mut st, "a", Combo::mouse(Mods::NONE, 4)).unwrap();
    assert_eq!(m.bind(&mut st, "b", Combo::mouse(Mods::NONE, 4)).unwrap_err().message(), "Already used by Mic mute");
    m.bind(&mut st, "b", Combo::mods_only(Mods::CTRL.with(Mods::SHIFT))).unwrap();
    assert_eq!(
        m.bind(&mut st, "c", Combo::mods_only(Mods::CTRL.with(Mods::SHIFT))).unwrap_err().message(),
        "Already used by Screenshot"
    );
    assert_eq!(m.bind(&mut st, "talk", Combo::mouse(Mods::NONE, 4)).unwrap_err().message(), "Already used by Mic mute");
    // different combos are different keys: Alt + Mouse 4, Ctrl + Shift + M
    m.bind(&mut st, "c", Combo::mouse(Mods::ALT, 4)).unwrap();
    m.bind(&mut st, "talk", cs(VK_M)).unwrap();
    assert_eq!(m.bind(&mut st, "a", cs(VK_M)).unwrap_err().message(), "Already used by Voice to text");
    // not keys: no modifier at all, left / right button
    assert_eq!(m.bind(&mut st, "c", Combo::mods_only(Mods::NONE)), Err(BindError::NotAKey));
    assert_eq!(m.bind(&mut st, "c", Combo::new(Mods::CTRL, 0x01)), Err(BindError::NotAKey));
    assert_eq!(m.bind(&mut st, "c", Combo::new(Mods::CTRL, 0x02)), Err(BindError::NotAKey));
}

#[test]
fn mouse_button_fires_on_down_with_its_modifiers() {
    let s = Scratch::new("keys-rawmouse");
    let mut st = SettingsStore::open(s.dir());
    let mut m = raw_manager(&st, FakeKeysOs::new());
    m.bind(&mut st, "a", Combo::mouse(Mods::NONE, 4)).unwrap();
    m.bind(&mut st, "b", Combo::mouse(Mods::ALT, 5)).unwrap();
    m.bind(&mut st, "c", Combo::mouse(Mods::NONE, 3)).unwrap();
    assert!(fired(&mut m, &[btn(0), btn(0), btn(RI_MOUSE_LEFT_DOWN), btn(RI_MOUSE_LEFT_UP)]).is_empty());
    assert_eq!(fired(&mut m, &[btn(RI_MOUSE_BUTTON_4_DOWN), btn(RI_MOUSE_BUTTON_4_UP)]), vec![("a".into(), true)]);
    assert_eq!(fired(&mut m, &[btn(RI_MOUSE_MIDDLE_DOWN)]), vec![("c".into(), true)]);
    // Alt + Mouse 5: the modifiers come from Windows on the press
    assert!(fired(&mut m, &[btn(RI_MOUSE_BUTTON_5_DOWN)]).is_empty());
    drop(m);
    let mut os = FakeKeysOs::new();
    os.mods = Some(Mods::ALT);
    let mut m2 = raw_manager(&st, os);
    assert_eq!(fired(&mut m2, &[btn(RI_MOUSE_BUTTON_5_DOWN)]), vec![("b".into(), true)]);
    assert!(fired(&mut m2, &[btn(RI_MOUSE_BUTTON_4_DOWN)]).is_empty(), "Alt held: plain Mouse 4 doesn't fire");
}

#[test]
fn modifier_only_fires_on_release_only_when_nothing_else_came() {
    let s = Scratch::new("keys-rawmods");
    let mut st = SettingsStore::open(s.dir());
    let mut m = raw_manager(&st, FakeKeysOs::new());
    m.bind(&mut st, "a", Combo::mods_only(Mods::CTRL.with(Mods::SHIFT))).unwrap();
    m.bind(&mut st, "b", Combo::mods_only(Mods::ALT)).unwrap();
    let ctrl_shift = [kd(VK_CONTROL), kd(VK_SHIFT), kd(VK_SHIFT), ku(VK_SHIFT), ku(VK_CONTROL)];
    assert_eq!(fired(&mut m, &ctrl_shift), vec![("a".into(), true)]);
    // Ctrl + Shift + S (Save as): no
    assert!(fired(&mut m, &[kd(VK_CONTROL), kd(VK_SHIFT), kd(0x53), ku(0x53), ku(VK_SHIFT), ku(VK_CONTROL)]).is_empty());
    // Ctrl + Shift + click / wheel: no
    let click = [kd(VK_CONTROL), kd(VK_SHIFT), btn(RI_MOUSE_LEFT_DOWN), btn(RI_MOUSE_LEFT_UP), ku(VK_SHIFT), ku(VK_CONTROL)];
    assert!(fired(&mut m, &click).is_empty());
    assert!(fired(&mut m, &[kd(VK_CONTROL), kd(VK_SHIFT), btn(RI_MOUSE_WHEEL), ku(VK_SHIFT), ku(VK_CONTROL)]).is_empty());
    // only Ctrl: not Ctrl + Shift
    assert!(fired(&mut m, &[kd(VK_CONTROL), ku(VK_CONTROL)]).is_empty());
    // right Alt alone (E0) fires "b"; Alt while W is held (walking in a game) doesn't
    let ralt = |f| Packet::Key { vk: VK_MENU, make: 0x38, flags: RI_KEY_E0 | f };
    assert_eq!(fired(&mut m, &[ralt(0), ralt(RI_KEY_BREAK)]), vec![("b".into(), true)]);
    assert!(fired(&mut m, &[kd(0x57), kd(VK_MENU), ku(VK_MENU), ku(0x57)]).is_empty());
    // the mouse moving meanwhile changes nothing
    assert_eq!(fired(&mut m, &[kd(VK_MENU), btn(0), btn(0), ku(VK_MENU)]), vec![("b".into(), true)]);
}

#[test]
fn release_key_fires_down_and_up_once() {
    let s = Scratch::new("keys-rawrel");
    let mut st = SettingsStore::open(s.dir());
    let mut m = raw_manager(&st, FakeKeysOs::new());
    m.bind(&mut st, "talk", Combo::new(Mods::NONE, VK_F8)).unwrap();
    assert_eq!(
        fired(&mut m, &[kd(VK_F8), kd(VK_F8), kd(VK_F8), ku(VK_F8)]),
        vec![("talk".into(), true), ("talk".into(), false)]
    );
    // with Ctrl held it is another combo
    assert!(fired(&mut m, &[kd(VK_CONTROL), kd(VK_F8), ku(VK_F8), ku(VK_CONTROL)]).is_empty());
}

#[test]
fn raw_registration_failure_refuses_and_leaves_nothing() {
    let s = Scratch::new("keys-rawfail");
    let mut st = SettingsStore::open(s.dir());
    let mut os = FakeKeysOs::new();
    os.raw_fails = true;
    let mut m = raw_manager(&st, os);
    let e = m.bind(&mut st, "a", Combo::mouse(Mods::NONE, 4)).unwrap_err();
    assert!(matches!(e, BindError::Refused(_)));
    assert_eq!(m.state("a"), KeyState::Unbound);
    assert_eq!(m.os().raw, (false, false));
    assert!(fired(&mut m, &[btn(RI_MOUSE_BUTTON_4_DOWN)]).is_empty());
}

#[test]
fn pause_drops_raw_input_and_resume_brings_it_back() {
    let s = Scratch::new("keys-rawpause");
    let mut st = SettingsStore::open(s.dir());
    let mut m = raw_manager(&st, FakeKeysOs::new());
    m.bind(&mut st, "a", Combo::mouse(Mods::NONE, 4)).unwrap();
    m.pause();
    assert_eq!(m.os().raw, (false, false));
    assert!(fired(&mut m, &[btn(RI_MOUSE_BUTTON_4_DOWN)]).is_empty());
    m.resume();
    assert_eq!(m.os().raw, (false, true));
    assert_eq!(fired(&mut m, &[btn(RI_MOUSE_BUTTON_4_DOWN)]), vec![("a".into(), true)]);
}

/// The WM_INPUT cost at 8 kHz mouse polling. Run once, by hand, in release:
/// `cargo test --release -p bu-app wm_input_mouse_move_cost -- --ignored --nocapture`.
/// 80,000 synthetic RAWINPUT mouse-move packets (10 s at 8000 Hz) go through `KeysManager::on_rawinput` — the exact
/// function the window procedure calls after GetRawInputData (that one Windows call is not in this number: it needs a
/// real WM_INPUT). Nothing real is registered or sent; a mouse key and a modifier-only key are bound (raw mouse on).
#[test]
#[ignore]
fn wm_input_mouse_move_cost() {
    use windows::Win32::UI::Input::{RAWINPUT, RIM_TYPEMOUSE};
    let s = Scratch::new("keys-bench");
    let mut st = SettingsStore::open(s.dir());
    let mut m = raw_manager(&st, FakeKeysOs::new());
    m.bind(&mut st, "a", Combo::mouse(Mods::NONE, 4)).unwrap();
    m.bind(&mut st, "b", Combo::mods_only(Mods::CTRL.with(Mods::SHIFT))).unwrap();
    assert_eq!(m.os().raw, (true, true));
    const N: usize = 80_000;
    let packets: Vec<RAWINPUT> = (0..N)
        .map(|i| {
            let mut r = RAWINPUT::default();
            r.header.dwType = RIM_TYPEMOUSE.0;
            r.header.dwSize = std::mem::size_of::<RAWINPUT>() as u32;
            r.data.mouse.lLastX = (i % 7) as i32 - 3;
            r.data.mouse.lLastY = (i % 5) as i32 - 2;
            r
        })
        .collect();
    let mut fires = 0u64;
    let mut runs = Vec::new();
    for _ in 0..5 {
        let t = std::time::Instant::now();
        for p in &packets {
            m.on_rawinput(std::hint::black_box(p), |_, _| fires += 1);
        }
        runs.push(t.elapsed().as_nanos() as f64 / N as f64);
    }
    assert_eq!(fires, 0);
    let best = runs.iter().copied().fold(f64::MAX, f64::min);
    let worst = runs.iter().copied().fold(0.0, f64::max);
    println!(
        "WM_INPUT mouse move: runs {runs:.2?} ns/packet; best {best:.2} worst {worst:.2} ns; at 8000/s = {:.5} % .. {:.5} % of one core",
        best * 8000.0 / 1e9 * 100.0,
        worst * 8000.0 / 1e9 * 100.0
    );
}

/// Order 035 follow-up: an action flagged `with_extra_mods` (Notifications for OBS) fires like OBS's own hotkeys - Insert
/// also while Ctrl / Shift / Alt is held (crouching on Ctrl in a game); an exact match of another action still wins; every
/// other action keeps the exact rule.
#[test]
fn an_extra_mods_action_fires_with_more_modifiers_held_and_an_exact_one_wins() {
    let sc = Scratch::new("keys-extra-mods");
    let mut st = SettingsStore::open(sc.dir());
    let mut m = KeysManager::new(FakeKeysOs::new(), &st);
    m.add_action(Action::new("obs.clip", "Save clip", "ntf").with_release().with_extra_mods());
    m.add_action(Action::new("talk", "Voice to text", "voice").with_release());
    m.add_action(Action::new("obs.rec", "Recording on/off", "ntf").with_release().with_extra_mods());
    m.add_action(Action::new("shot", "Screenshot", "p").with_release());
    m.bind(&mut st, "obs.clip", Combo::new(Mods::NONE, VK_INSERT)).unwrap();
    m.bind(&mut st, "talk", Combo::new(Mods::NONE, VK_F8)).unwrap();
    m.bind(&mut st, "obs.rec", Combo::new(Mods::CTRL, VK_M)).unwrap();
    m.bind(&mut st, "shot", Combo::new(Mods::CTRL.with(Mods::SHIFT), VK_M)).unwrap();
    // registered through Raw Input (listen only), never RegisterHotKey
    assert!(m.os().registered.is_empty(), "{:?}", m.os().registered);
    let ins_d = Packet::Key { vk: VK_INSERT, make: 0x52, flags: RI_KEY_E0 };
    let ins_u = Packet::Key { vk: VK_INSERT, make: 0x52, flags: RI_KEY_E0 | RI_KEY_BREAK };
    // plain Insert, and Insert while Ctrl is held (and Ctrl + Shift)
    assert_eq!(fired(&mut m, &[ins_d, ins_u]), vec![("obs.clip".to_string(), true), ("obs.clip".to_string(), false)]);
    assert_eq!(fired(&mut m, &[kd(VK_LCONTROL), ins_d, ins_u, ku(VK_LCONTROL)]), vec![("obs.clip".to_string(), true), ("obs.clip".to_string(), false)]);
    assert_eq!(fired(&mut m, &[kd(VK_LCONTROL), kd(VK_SHIFT), ins_d, ins_u, ku(VK_SHIFT), ku(VK_LCONTROL)]).len(), 2);
    // an exact-rule action does not: F8 with Ctrl held is not Voice to text
    assert!(fired(&mut m, &[kd(VK_LCONTROL), kd(VK_F8), ku(VK_F8), ku(VK_LCONTROL)]).is_empty());
    assert_eq!(fired(&mut m, &[kd(VK_F8), ku(VK_F8)]).len(), 2);
    // Ctrl + M = Recording (loose), Ctrl + Shift + M = Screenshot (exact wins over the loose Ctrl + M)
    assert_eq!(fired(&mut m, &[kd(VK_LCONTROL), kd(VK_M), ku(VK_M), ku(VK_LCONTROL)])[0].0, "obs.rec");
    assert_eq!(fired(&mut m, &[kd(VK_LCONTROL), kd(VK_SHIFT), kd(VK_M), ku(VK_M), ku(VK_SHIFT), ku(VK_LCONTROL)])[0].0, "shot");
    // Alt + Ctrl + M (no exact action): the loose Ctrl + M
    assert_eq!(fired(&mut m, &[kd(VK_LCONTROL), kd(VK_LMENU), kd(VK_M), ku(VK_M), ku(VK_LMENU), ku(VK_LCONTROL)])[0].0, "obs.rec");
    // a modifier the key needs must still be held: M alone is nothing
    assert!(fired(&mut m, &[kd(VK_M), ku(VK_M)]).is_empty());
    // any real key binds (the owner Oct 8): a plain letter for a loose action too
    assert!(m.check("obs.clip", Combo::new(Mods::NONE, 0x41)).is_ok());
}
