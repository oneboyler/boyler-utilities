use super::fake::*;
use super::*;
use crate::settings::scratch::Scratch;

const VK_A: u16 = 0x41;
const VK_M: u16 = 0x4D;
const VK_Q: u16 = 0x51;
const VK_V: u16 = 0x56;
const VK_Z: u16 = 0x5A;
const VK_F8: u16 = 0x77;

fn cs(vk: u16) -> Combo {
    Combo::new(Mods::CTRL.with(Mods::SHIFT), vk)
}

fn manager(store: &SettingsStore, os: FakeKeysOs) -> KeysManager<FakeKeysOs> {
    let mut m = KeysManager::new(os, store);
    m.add_action(Action::new("mic.toggle", "Mic mute", "audio"));
    m.add_action(Action::new("shot.take", "Screenshot", "screenshots"));
    m.add_action(Action::new("voice.talk", "Voice to text", "voice").with_release());
    m
}

#[test]
fn bind_registers_and_saves() {
    let s = Scratch::new("keys-bind");
    let mut st = SettingsStore::open(s.dir());
    let mut m = manager(&st, FakeKeysOs::new());
    m.bind(&mut st, "mic.toggle", cs(VK_M)).unwrap();
    assert_eq!(m.state("mic.toggle"), KeyState::Working(cs(VK_M)));
    assert_eq!(m.os().registered.values().copied().collect::<Vec<_>>(), vec![cs(VK_M)]);
    let slot = *m.os().registered.keys().next().unwrap();
    assert_eq!(m.action_for_slot(slot), Some("mic.toggle"));
    assert_eq!(st.get_list(Scope::App, SETTING).unwrap(), &["mic.toggle=5,77".to_string()]);
}

#[test]
fn refuses_a_key_used_anywhere_in_the_app_and_says_by_whom() {
    let s = Scratch::new("keys-double");
    let mut st = SettingsStore::open(s.dir());
    let mut m = manager(&st, FakeKeysOs::new());
    m.bind(&mut st, "mic.toggle", cs(VK_M)).unwrap();
    let e = m.bind(&mut st, "shot.take", cs(VK_M)).unwrap_err();
    assert_eq!(e, BindError::UsedBy { action: "mic.toggle".into(), name: "Mic mute".into() });
    assert_eq!(e.message(), "Already used by Mic mute");
    // also against a release (Raw Input) action
    m.bind(&mut st, "voice.talk", cs(VK_F8)).unwrap();
    assert_eq!(m.bind(&mut st, "mic.toggle", cs(VK_F8)).unwrap_err().message(), "Already used by Voice to text");
    // the refused action keeps its old key, the other is untouched
    assert_eq!(m.state("mic.toggle"), KeyState::Working(cs(VK_M)));
    assert_eq!(m.state("shot.take"), KeyState::Unbound);
    // binding the same key again to its own action is fine
    m.bind(&mut st, "mic.toggle", cs(VK_M)).unwrap();
    assert_eq!(m.used_by(cs(VK_M)).map(|a| a.id.as_str()), Some("mic.toggle"));
}

#[test]
fn rejects_numpad_keys() {
    let s = Scratch::new("keys-numpad");
    let mut st = SettingsStore::open(s.dir());
    let mut m = manager(&st, FakeKeysOs::new());
    for vk in (0x60..=0x6F).chain([VK_NUMLOCK, VK_CLEAR]) {
        assert_eq!(m.bind(&mut st, "mic.toggle", Combo::new(Mods::CTRL, vk)), Err(BindError::Numpad), "vk {vk:#x}");
    }
    assert!(m.os().log.is_empty(), "nothing reached the OS");
    assert_eq!(m.state("mic.toggle"), KeyState::Unbound);
}

#[test]
fn every_real_key_binds_typing_keys_too() {
    // the owner Oct 8 (test 2): ANY real key - a plain letter, digit, Space, Enter, Tab, AltGr combos - overrules A_014_01
    let s = Scratch::new("keys-any");
    let mut st = SettingsStore::open(s.dir());
    let mut m = manager(&st, FakeKeysOs::new());
    let ctrl_alt = Mods::CTRL.with(Mods::ALT);
    for combo in [
        Combo::new(Mods::NONE, VK_A),
        Combo::new(Mods::NONE, 0x31),
        Combo::new(Mods::SHIFT, VK_OEM_1),
        Combo::new(Mods::NONE, VK_SPACE),
        Combo::new(Mods::NONE, VK_RETURN),
        Combo::new(Mods::NONE, VK_TAB),
        Combo::new(ctrl_alt, VK_V),
        Combo::new(ctrl_alt, VK_Q),
        Combo::new(Mods::NONE, VK_F8),
        Combo::new(Mods::CTRL, VK_BACK),
        Combo::new(Mods::SHIFT, VK_ESCAPE),
    ] {
        m.bind(&mut st, "mic.toggle", combo).unwrap_or_else(|e| panic!("{combo:?}: {e:?}"));
        assert_eq!(m.state("mic.toggle"), KeyState::Working(combo));
    }
    // still one key once: the letter bound to Mic mute is refused for Screenshot
    m.bind(&mut st, "mic.toggle", Combo::new(Mods::NONE, VK_A)).unwrap();
    assert_eq!(m.bind(&mut st, "shot.take", Combo::new(Mods::NONE, VK_A)).unwrap_err().message(), "Already used by Mic mute");
    // plain Esc / Backspace are the key field's own (cancel / clear); numpad keys stay out (the project rules)
    assert_eq!(m.bind(&mut st, "shot.take", Combo::new(Mods::NONE, VK_ESCAPE)), Err(BindError::NotAKey));
    assert_eq!(m.bind(&mut st, "shot.take", Combo::new(Mods::NONE, VK_BACK)), Err(BindError::NotAKey));
}

#[test]
fn names_come_from_the_layout_croatian() {
    let s = Scratch::new("keys-names");
    let st = SettingsStore::open(s.dir());
    let m = manager(&st, FakeKeysOs::new());
    assert_eq!(m.combo_text(cs(VK_OEM_1)), "Ctrl + Shift + Č");
    assert_eq!(m.combo_text(Combo::new(Mods::ALT, VK_OEM_7)), "Alt + Ć");
    assert_eq!(m.combo_text(Combo::new(Mods::WIN, VK_OEM_5)), "Win + Ž");
    assert_eq!(m.combo_text(Combo::new(Mods::CTRL, VK_OEM_4)), "Ctrl + Š");
    assert_eq!(m.combo_text(Combo::new(Mods::CTRL, VK_OEM_6)), "Ctrl + Đ");
    assert_eq!(m.combo_text(Combo::new(Mods::CTRL, VK_Z)), "Ctrl + Z");
    assert_eq!(m.combo_text(Combo::new(Mods::CTRL, VK_OEM_PLUS)), "Ctrl + +");
    // fixed names (the drawing's KEYMAP), modifiers always Ctrl, Alt, Shift, Win
    let all = Mods::WIN.with(Mods::SHIFT).with(Mods::ALT).with(Mods::CTRL);
    assert_eq!(m.combo_text(Combo::new(all, VK_PRIOR)), "Ctrl + Alt + Shift + Win + PgUp");
    assert_eq!(m.combo_text(Combo::new(Mods::NONE, VK_F8)), "F8");
    assert_eq!(m.combo_text(Combo::new(Mods::NONE, VK_SNAPSHOT)), "PrtSc");
    assert_eq!(m.held_text(Mods::SHIFT.with(Mods::CTRL)), "Ctrl + Shift");
}

#[test]
fn os_refusal_keeps_the_old_key() {
    let s = Scratch::new("keys-osref");
    let mut st = SettingsStore::open(s.dir());
    let mut m = manager(&st, FakeKeysOs::new().take(cs(VK_Q)));
    m.bind(&mut st, "mic.toggle", cs(VK_M)).unwrap();
    let e = m.bind(&mut st, "mic.toggle", cs(VK_Q)).unwrap_err();
    assert!(matches!(e, BindError::Refused(_)));
    assert_eq!(e.message(), "Windows or another app uses this key");
    assert_eq!(m.state("mic.toggle"), KeyState::Working(cs(VK_M)));
    assert_eq!(m.os().registered.values().copied().collect::<Vec<_>>(), vec![cs(VK_M)]);
    assert_eq!(st.get_list(Scope::App, SETTING).unwrap(), &["mic.toggle=5,77".to_string()]);
}

#[test]
fn release_actions_use_raw_keyboard_not_registerhotkey() {
    let s = Scratch::new("keys-release");
    let mut st = SettingsStore::open(s.dir());
    let mut m = manager(&st, FakeKeysOs::new());
    m.bind(&mut st, "voice.talk", cs(VK_F8)).unwrap();
    assert!(m.os().registered.is_empty());
    assert_eq!(m.os().raw, (true, false));
    m.unbind(&mut st, "voice.talk").unwrap();
    assert_eq!(m.os().raw, (false, false));
    assert_eq!(st.get_list(Scope::App, SETTING), None);
}

#[test]
fn mapping_survives_a_restart_and_reset_clears_it() {
    let s = Scratch::new("keys-persist");
    {
        let mut st = SettingsStore::open(s.dir());
        let mut m = manager(&st, FakeKeysOs::new());
        m.bind(&mut st, "mic.toggle", cs(VK_M)).unwrap();
        m.bind(&mut st, "voice.talk", Combo::new(Mods::ALT, VK_OEM_1)).unwrap();
    }
    let mut st = SettingsStore::open(s.dir());
    let mut m = manager(&st, FakeKeysOs::new());
    assert_eq!(m.state("mic.toggle"), KeyState::Working(cs(VK_M)));
    assert_eq!(m.state("voice.talk"), KeyState::Working(Combo::new(Mods::ALT, VK_OEM_1)));
    assert_eq!(m.os().registered.len(), 1);
    assert_eq!(m.os().raw, (true, false));
    st.reset_app_settings().unwrap();
    m.reload(&st);
    assert_eq!(m.state("mic.toggle"), KeyState::Unbound);
    assert!(m.os().registered.is_empty() && m.os().raw == (false, false));
}

#[test]
fn saved_key_taken_by_another_app_shows_not_working() {
    let s = Scratch::new("keys-notworking");
    let mut st = SettingsStore::open(s.dir());
    st.set_list(Scope::App, SETTING, &["mic.toggle=5,77".to_string(), "gone.action=1,65".to_string()]).unwrap();
    let mut m = manager(&st, FakeKeysOs::new().take(cs(VK_M)));
    assert!(matches!(m.state("mic.toggle"), KeyState::NotWorking(c, _) if c == cs(VK_M)));
    // an action that isn't there keeps its saved key in the file
    m.bind(&mut st, "shot.take", Combo::new(Mods::CTRL, VK_F8)).unwrap();
    assert!(st.get_list(Scope::App, SETTING).unwrap().contains(&"gone.action=1,65".to_string()));
}

#[test]
fn pause_unregisters_everything_and_resume_brings_it_back() {
    let s = Scratch::new("keys-pause");
    let mut st = SettingsStore::open(s.dir());
    let mut m = manager(&st, FakeKeysOs::new());
    m.bind(&mut st, "mic.toggle", cs(VK_M)).unwrap();
    m.bind(&mut st, "voice.talk", cs(VK_F8)).unwrap();
    m.pause();
    assert!(m.os().registered.is_empty() && m.os().raw == (false, false));
    assert_eq!(m.action_for_slot(1), None);
    // a bind while paused is checked with the OS but stays quiet until resume
    m.bind(&mut st, "shot.take", cs(VK_A)).unwrap();
    assert!(m.os().registered.is_empty());
    m.resume();
    assert_eq!(m.os().registered.len(), 2);
    assert_eq!(m.os().raw, (true, false));
}

/// A fake the test can still look at after the manager is gone.
struct Spy(std::sync::Arc<std::sync::Mutex<FakeKeysOs>>);

impl KeysOs for Spy {
    fn register(&mut self, slot: i32, combo: Combo) -> Result<(), String> {
        self.0.lock().unwrap().register(slot, combo)
    }
    fn unregister(&mut self, slot: i32) {
        self.0.lock().unwrap().unregister(slot)
    }
    fn raw_devices(&mut self, keyboard: bool, mouse: bool) -> Result<(), String> {
        self.0.lock().unwrap().raw_devices(keyboard, mouse)
    }
    fn mods_now(&self) -> Option<Mods> {
        self.0.lock().unwrap().mods_now()
    }
    fn key_name(&self, vk: u16) -> String {
        self.0.lock().unwrap().key_name(vk)
    }
}

#[test]
fn remove_action_and_drop_unregister_every_key() {
    let s = Scratch::new("keys-drop");
    let mut st = SettingsStore::open(s.dir());
    let fake = std::sync::Arc::new(std::sync::Mutex::new(FakeKeysOs::new()));
    let mut m = KeysManager::new(Spy(fake.clone()), &st);
    m.add_action(Action::new("a", "A", "p"));
    m.add_action(Action::new("b", "B", "p"));
    m.add_action(Action::new("c", "C", "p").with_release());
    m.bind(&mut st, "a", cs(VK_M)).unwrap();
    m.bind(&mut st, "b", cs(VK_A)).unwrap();
    m.bind(&mut st, "c", cs(VK_F8)).unwrap();
    m.remove_action("a");
    assert_eq!(fake.lock().unwrap().registered.len(), 1);
    drop(m);
    let f = fake.lock().unwrap();
    assert!(f.registered.is_empty() && f.raw == (false, false), "{:?}", f.log);
}

#[test]
fn unknown_action_is_refused() {
    let s = Scratch::new("keys-unknown");
    let mut st = SettingsStore::open(s.dir());
    let mut m = manager(&st, FakeKeysOs::new());
    assert_eq!(m.bind(&mut st, "nope", cs(VK_M)), Err(BindError::UnknownAction("nope".into())));
}

// ---- the key field's capture

fn ev(vk: u16, mods: Mods) -> KeyEvent {
    KeyEvent::new(vk, mods)
}

#[test]
fn capture_builds_a_combo() {
    let mut c = Capture::new();
    assert_eq!(c.key_down(ev(VK_LCONTROL, Mods::CTRL)), Step::Listening { held: Mods::CTRL });
    assert_eq!(c.key_down(ev(VK_LSHIFT, Mods::CTRL.with(Mods::SHIFT))), Step::Listening { held: Mods::CTRL.with(Mods::SHIFT) });
    assert_eq!(c.key_down(ev(VK_LSHIFT, Mods::CTRL.with(Mods::SHIFT)).repeat()), Step::Ignored);
    assert_eq!(c.key_up(ev(VK_LSHIFT, Mods::CTRL)), Step::Listening { held: Mods::CTRL });
    assert_eq!(c.key_down(ev(VK_OEM_1, Mods::CTRL)), Step::Done(Combo::new(Mods::CTRL, VK_OEM_1)));
    assert!(!c.is_listening());
    assert_eq!(c.key_down(ev(VK_A, Mods::NONE)), Step::Ignored);
}

#[test]
fn capture_esc_cancels_backspace_clears() {
    let mut c = Capture::new();
    assert_eq!(c.key_down(ev(VK_ESCAPE, Mods::NONE)), Step::Cancelled);
    let mut c = Capture::new();
    assert_eq!(c.key_down(ev(VK_BACK, Mods::NONE)), Step::Cleared);
    // with a modifier held they are keys like any other (the owner Oct 8: any real key)
    let mut c = Capture::new();
    c.key_down(ev(VK_LCONTROL, Mods::CTRL));
    assert_eq!(c.key_down(ev(VK_ESCAPE, Mods::CTRL)), Step::Done(Combo::new(Mods::CTRL, VK_ESCAPE)));
    let mut c = Capture::new();
    assert_eq!(c.key_down(ev(VK_BACK, Mods::SHIFT)), Step::Done(Combo::new(Mods::SHIFT, VK_BACK)));
}

#[test]
fn capture_refuses_numpad_and_keeps_listening() {
    let mut c = Capture::new();
    c.key_down(ev(VK_LCONTROL, Mods::CTRL));
    assert_eq!(c.key_down(ev(0x61, Mods::CTRL)), Step::Refused(BindError::Numpad));
    assert!(c.is_listening());
    assert_eq!(c.held(), Mods::NONE);
    // numpad Enter (extended) refused; the main Enter is a key
    assert_eq!(c.key_down(ev(VK_RETURN, Mods::CTRL).extended()), Step::Refused(BindError::Numpad));
    // NumLock-off numpad Home (not extended) refused; the real Home (extended) passes
    assert_eq!(c.key_down(ev(VK_HOME, Mods::CTRL)), Step::Refused(BindError::Numpad));
    assert_eq!(c.key_down(ev(VK_HOME, Mods::CTRL).extended()), Step::Done(Combo::new(Mods::CTRL, VK_HOME)));
    let mut c = Capture::new();
    assert_eq!(c.key_down(ev(VK_CLEAR, Mods::NONE)), Step::Refused(BindError::Numpad));
    assert_eq!(c.key_down(ev(VK_RETURN, Mods::ALT)), Step::Done(Combo::new(Mods::ALT, VK_RETURN)));
}

#[test]
fn capture_prtsc_up_only_counts() {
    let mut c = Capture::new();
    assert_eq!(c.key_up(ev(VK_SNAPSHOT, Mods::NONE)), Step::Done(Combo::new(Mods::NONE, VK_SNAPSHOT)));
    let mut c = Capture::new();
    assert_eq!(c.key_down(ev(VK_SNAPSHOT, Mods::ALT)), Step::Done(Combo::new(Mods::ALT, VK_SNAPSHOT)));
    assert_eq!(c.key_up(ev(VK_SNAPSHOT, Mods::ALT)), Step::Ignored);
}

#[test]
fn capture_then_manager_refusal_restarts_listening() {
    let s = Scratch::new("keys-flow");
    let mut st = SettingsStore::open(s.dir());
    let mut m = manager(&st, FakeKeysOs::new());
    m.bind(&mut st, "mic.toggle", cs(VK_M)).unwrap();
    m.pause();
    let mut c = Capture::new();
    let Step::Done(combo) = c.key_down(ev(VK_M, Mods::CTRL.with(Mods::SHIFT))) else { panic!() };
    let err = m.bind(&mut st, "shot.take", combo).unwrap_err();
    assert_eq!(err.message(), "Already used by Mic mute");
    c.restart();
    assert!(c.is_listening());
    let Step::Done(combo) = c.key_down(ev(VK_F8, Mods::NONE)) else { panic!() };
    m.bind(&mut st, "shot.take", combo).unwrap();
    m.resume();
    assert_eq!(m.os().registered.len(), 2);
}

#[test]
fn refuses_the_menus_own_shortcuts() {
    let s = Scratch::new("keys-menu");
    let mut st = SettingsStore::open(s.dir());
    let mut m = manager(&st, FakeKeysOs::new());
    for (combo, _) in MENU_KEYS {
        match m.bind(&mut st, "mic.toggle", combo) {
            Err(BindError::UsedBy { action, name }) => {
                assert_eq!(action, "menu");
                assert!(name.starts_with("the menu's"), "{name}");
            }
            r => panic!("{combo:?} -> {r:?}"),
        }
    }
    assert!(m.os().registered.is_empty());
    // the same letters with another modifier are free
    m.bind(&mut st, "mic.toggle", cs(0x46)).unwrap();
}

/// A feature switched off (Mic mute off, or the way it doesn't use): its key stays set and saved but is not registered -
/// the keystroke goes to the other apps; on again = registered again. Resume after a key field keeps it off.
#[test]
fn an_action_switched_off_keeps_its_key_but_does_not_take_it() {
    let s = Scratch::new("keys-active");
    let mut st = SettingsStore::open(s.dir());
    let mut m = manager(&st, FakeKeysOs::new());
    m.bind(&mut st, "mic.toggle", cs(VK_M)).unwrap();
    m.set_active("mic.toggle", false);
    assert!(m.os().registered.is_empty(), "off: not registered");
    assert!(matches!(m.state("mic.toggle"), KeyState::Working(_)), "the key stays set");
    m.pause();
    m.resume();
    assert!(m.os().registered.is_empty(), "a key field's resume keeps it off");
    m.set_active("mic.toggle", true);
    assert_eq!(m.os().registered.values().copied().collect::<Vec<_>>(), vec![cs(VK_M)]);
}
