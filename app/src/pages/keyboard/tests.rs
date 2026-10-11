//! The Keyboard page, driven like the frame drives it (clicks by key) and pictured (the page's boxes painted by the app's own
//! painter over a flat backdrop) - all with fakes: nothing here touches the PC, the registry, a stream or a key.

use super::getter::{K_GDONE, K_GROW};
use super::*;
use crate::gfx::Gfx;
use crate::icons::Icons;
use crate::ui::cx::State;
use crate::ui::lay::Laid;
use bu_keysound::macros::{Macro, Repeat, Step};
use bu_keysound::PlayOn;

fn page() -> Keyboard {
    let mut p = Keyboard::default();
    p.open(&Env { test: true, ..Env::default() }, 0.0);
    p
}

fn idx_of(p: &Keyboard, code: Code) -> usize {
    p.keys.iter().position(|k| k.code == code).unwrap_or_else(|| panic!("no key {code:X} in this size"))
}

/// Runs `f` with a Cx (hover / focus state fresh each call).
fn with_cx<R>(f: impl FnOnce(&mut Cx) -> R) -> R {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st);
    cx.page = "kbd";
    f(&mut cx)
}

fn click(p: &mut Keyboard, k: Key) {
    with_cx(|cx| p.event(&Ev::Click(k), cx));
}

/// A click with Ctrl / Shift held.
fn click_mods(p: &mut Keyboard, k: Key, ctrl: bool, shift: bool) {
    with_cx(|cx| {
        cx.mods.ctrl = ctrl;
        cx.mods.shift = shift;
        p.event(&Ev::Click(k), cx)
    });
}

/// A button's box was pressed (the lists open under it).
fn press(p: &mut Keyboard, k: Key) {
    with_cx(|cx| p.event(&Ev::Press(k, 10.0, 10.0, (0.0, 0.0, 190.0, 28.0)), cx));
}

fn key_click(p: &mut Keyboard, code: Code) {
    let i = idx_of(p, code);
    click(p, idx(K_KEY, i));
}

#[test]
fn a_fresh_page_is_off_full_size_and_unchanged() {
    let p = page();
    assert!(!p.prefs.on, "key sounds are off by default: nothing listens");
    assert!(!p.card_open, "Order 090: the Keyboard sounds card starts folded");
    assert_eq!(p.prefs.size, Size::Full);
    assert_eq!(p.model.remap_count() + p.model.bind_count(), 0);
    assert_eq!(p.keys.len(), pic::keys(Size::Full).0.len());
    // labelled by the (test) Croatian layout
    let z = idx_of(&p, 0x15);
    assert_eq!(p.texts[z], "Z");
    assert_eq!(p.texts[idx_of(&p, 0x27)], "Č");
}

/// Order 090 (v8): the card folds like Mouse acceleration - folded at first, switching on opens it, the header / chevron
/// open and close it any time.
#[test]
fn the_keyboard_sounds_card_folds_like_mouse_acceleration() {
    let mut p = page();
    click(&mut p, K_CARDH);
    assert!(p.card_open);
    click(&mut p, K_CHEV);
    assert!(!p.card_open);
    click(&mut p, K_ON);
    assert!(p.prefs.on && p.card_open, "switching on opens it");
    click(&mut p, K_CARDH);
    assert!(!p.card_open && p.prefs.on, "closable while on");
    // no mouse / controller switches on this tab any more (they are in their own tabs)
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(0.0, false, &g, &mut st);
    p.card_open = true;
    let root = El::block().w(600.0).children(p.build(&mut cx));
    let laid = Laid::new(&g, root, 600.0, None);
    let texts: Vec<String> = laid.nodes.iter().filter_map(|n| if let crate::ui::el::Content::Text(t) = &n.el.content { Some(t.s.to_string()) } else { None }).collect();
    for gone in ["Controller too", "Mouse clicks too", "Macros", "+ New macro", "+ Ready-made macro"] {
        assert!(!texts.iter().any(|t| t == gone), "{gone} is not on the Keyboard tab");
    }
    assert!(texts.iter().any(|t| t == "Keyboard sounds"));
}

#[test]
fn a_remap_is_made_in_the_keys_window_by_picking_the_new_key() {
    let mut p = page();
    key_click(&mut p, 0x3A);
    assert_eq!(p.sel, Some(0x3A), "the key's own window opens");
    // its Remap mode
    click(&mut p, idx(K_MODE, 1));
    assert!(p.choosing, "now it waits for the new key");
    with_cx(|cx| p.choose(Choice::Target(0x01), cx));
    assert_eq!(p.model.remap_of(0x3A), Some(0x01));
    assert!(!p.choosing);
    assert!(p.model.dirty(), "Apply is needed (admin + restart)");
    assert_eq!(p.model.mode(0x3A, false), Mode::Remap);
    // a key "becoming itself" is the key as Windows made it: nothing is stored (Order 059)
    click(&mut p, K_TARGET);
    with_cx(|cx| p.choose(Choice::Target(0x3A), cx));
    assert_eq!(p.model.remap_of(0x3A), None);
    assert_eq!(p.model.mode(0x3A, false), Mode::Normal);
    // Reset this key
    p.model.set_remap(0x3A, 0x01).unwrap();
    click(&mut p, K_RESETKEY);
    assert_eq!(p.model.remap_of(0x3A), None);
}

#[test]
fn a_key_can_get_an_action_or_a_macro() {
    let mut p = page();
    key_click(&mut p, 0x44);
    click(&mut p, idx(K_MODE, 2));
    assert_eq!(p.model.binds.get(0x44), Some(&Bind::Preset(Preset::PlayPause)), "a first action is chosen at once");
    // the list offers the presets, grouped
    let list = p.list(Pop::Action);
    let names: Vec<&str> = list.iter().map(|(l, _, _)| l.as_str()).collect();
    for want in ["Edit", "Copy", "Paste", "Cut", "Undo", "Redo", "Select all", "Windows", "Task Manager", "Lock the PC", "Show the desktop", "Emoji panel", "Browser", "Back", "Forward", "Refresh", "New tab", "Close tab", "Reopen closed tab", "Media", "Play / pause", "Volume up", "Mute", "Switch audio output", "Open", "A website…"] {
        assert!(names.contains(&want), "{want} in {names:?}");
    }
    // a new macro, right in the key's window: Runs [list] -> New macro
    click(&mut p, idx(K_MODE, 3));
    assert_eq!(p.model.binds.get(0x44), Some(&Bind::Preset(Preset::PlayPause)), "no macro yet: the action stays until one is chosen");
    press(&mut p, sub(K_MAC, "list"));
    click(&mut p, sub(K_MAC, "list"));
    // (no macros yet: the list is New macro, then the ready-made ones)
    click(&mut p, idx(sub(K_MAC, "menu"), 0));
    assert_eq!(p.model.macros.len(), 1);
    assert_eq!(p.model.binds.get(0x44), Some(&Bind::Macro("m1".into())));
    assert!(!p.model.dirty(), "an action or macro never needs Apply");
}

#[test]
fn numpad_keys_cannot_carry_an_action() {
    let mut p = page();
    key_click(&mut p, 0x47);
    click(&mut p, idx(K_MODE, 2));
    assert!(p.model.binds.get(0x47).is_none());
    assert!(p.err.as_deref().unwrap_or("").contains("Numpad"));
}

#[test]
fn the_size_switch_redraws_the_keys_and_clears_the_choice() {
    let mut p = page();
    key_click(&mut p, 0x3A);
    click(&mut p, idx(K_SIZE, 3));
    assert_eq!(p.prefs.size, Size::P60);
    assert_eq!(p.sel, None);
    assert!(p.keys.iter().all(|k| k.code != 0x3B), "no function row on a 60 %");
    assert!(p.keys.iter().any(|k| k.code == 0x3A));
}

#[test]
fn reset_all_clears_every_key_but_keeps_the_macros_and_the_sounds() {
    let mut p = page();
    p.model.set_remap(0x3A, 0x01).unwrap();
    let id = p.model.new_macro().unwrap();
    p.model.set_macro(0x44, &id).unwrap();
    p.prefs.keys.set(0x1C, bu_keysound::Layer { press: Some("boing.wav".into()), ..Default::default() }).unwrap();
    click(&mut p, K_RESETALL);
    assert_eq!(p.model.remap_count() + p.model.bind_count(), 0);
    assert_eq!(p.model.macros.len(), 1);
    assert_eq!(p.prefs.keys.len(), 1, "the sounds stay");
}

/// Order 090: a macro is built right in the key's window - Key down / up, Press, Type, Wait, Click, Open; moved, copied,
/// deleted; Repeat; "Delete macro" frees its keys.
#[test]
fn a_macro_is_built_step_by_step_in_the_keys_window() {
    let mut p = page();
    let id = p.model.new_macro().unwrap();
    p.model.set_macro(0x44, &id).unwrap();
    key_click(&mut p, 0x44);
    assert_eq!(p.mode_of(0x44), Mode::Macro);
    let add = |p: &mut Keyboard, i: usize| click(p, idx(sub(K_MAC, "add"), i));
    for kind in [3usize, 4, 6, 5] {
        add(&mut p, kind);
    }
    assert_eq!(p.model.macros[0].steps, vec![Step::Type(String::new()), Step::Wait(100), Step::Open(String::new()), Step::Click(0)]);
    let step = |i: usize, part: &str| sub(idx(sub(K_MAC, "step"), i), part);
    with_cx(|cx| {
        for c in "hi".chars() {
            p.event(&Ev::Char(step(0, "val"), c), cx);
        }
    });
    assert_eq!(p.model.macros[0].steps[0], Step::Type("hi".into()));
    click(&mut p, step(0, "dn"));
    assert_eq!(p.model.macros[0].steps[1], Step::Type("hi".into()));
    click(&mut p, step(1, "cp"));
    assert_eq!(p.model.macros[0].steps[2], Step::Type("hi".into()), "copied below itself");
    click(&mut p, step(3, "rm"));
    assert_eq!(p.model.macros[0].steps.len(), 4);
    // a wait takes digits and Backspace
    with_cx(|cx| p.event(&Ev::Key(step(0, "val"), 0x08), cx));
    assert_eq!(p.model.macros[0].steps[0], Step::Wait(10));
    // a Key down step listens for its key
    add(&mut p, 0);
    let last = p.model.macros[0].steps.len() - 1;
    with_cx(|cx| p.event(&Ev::Key(step(last, "key"), 0x10), cx));
    assert_eq!(p.model.macros[0].steps[last], Step::Down(0x10));
    // Repeat: While the key is held
    press(&mut p, sub(K_MAC, "rep"));
    click(&mut p, sub(K_MAC, "rep"));
    click(&mut p, idx(sub(K_MAC, "menu"), 5));
    assert_eq!(p.model.macros[0].rep, Repeat::Held);
    // Delete macro: gone, and its key back to normal
    click(&mut p, sub(K_MAC, "del"));
    assert!(p.model.macros.is_empty());
    assert!(p.model.binds.get(0x44).is_none());
}

#[test]
fn recording_keeps_only_what_is_pressed_until_stop() {
    let mut p = page();
    let id = p.model.new_macro().unwrap();
    p.model.set_macro(0x44, &id).unwrap();
    key_click(&mut p, 0x44);
    click(&mut p, sub(K_MAC, "rec"));
    assert!(p.med.rec);
    // (combo_of reads the real modifier state; the test presses a plain key)
    with_cx(|cx| p.event(&Ev::Key(sub(K_MAC, "rec"), 0x41), cx));
    with_cx(|cx| p.event(&Ev::Key(sub(K_MAC, "rec"), 0x42), cx));
    assert!(matches!(p.model.macros[0].steps.last(), Some(Step::Keys(v)) if v.last() == Some(&0x42)));
    assert_eq!(p.model.macros[0].steps.iter().filter(|s| matches!(s, Step::Keys(_))).count(), 2);
    // Esc stops it
    with_cx(|cx| p.event(&Ev::Key(sub(K_MAC, "rec"), 0x1B), cx));
    assert!(!p.med.rec);
}

#[test]
fn the_page_closing_forgets_what_was_typed() {
    let mut p = page();
    p.try_text = "secret".into();
    p.close();
    assert!(p.try_text.is_empty());
    assert!(!p.loaded);
}

#[test]
fn volume_follows_the_slider_and_the_games_switch_toggles() {
    let mut p = page();
    with_cx(|cx| p.event(&Ev::Press(K_VOL, 100.0, 0.0, (0.0, 0.0, 200.0, 20.0)), cx));
    assert!(p.prefs.s.volume > 0 && p.prefs.s.volume <= 100);
    let off = p.prefs.s.off_in_game;
    click(&mut p, K_GAME);
    assert_ne!(p.prefs.s.off_in_game, off);
}

#[test]
fn the_page_builds_in_every_state_without_panicking() {
    let mut p = page();
    let g = Gfx::new(1.0);
    for size in Size::ALL {
        p.change_size(size);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        assert!(!p.build(&mut cx).is_empty());
    }
    p.model.set_remap(0x3A, 0x01).unwrap();
    p.model.set_preset(0x44, Preset::PlayPause).unwrap();
    let id = p.model.new_macro().unwrap();
    p.model.set_macro(0x57, &id).unwrap();
    p.card_open = true;
    for (sel, mode) in [(0x3Au16, Mode::Remap), (0x44, Mode::Action), (0x57, Mode::Macro), (0x1E, Mode::Normal)] {
        p.select(Some(sel));
        p.want = Some(mode);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        assert!(!p.build(&mut cx).is_empty());
        assert!(p.popups(&mut cx).is_some(), "the key window");
    }
    p.select(None);
    p.pick = vec![0x1E, 0x1F, 0x20];
    p.many = true;
    let mut st = State::default();
    let mut cx = Cx::new(0.0, false, &g, &mut st);
    assert!(p.popups(&mut cx).is_some(), "the N keys window");
    p.many = false;
    p.open_make(0.0);
    assert!(p.popups(&mut cx).is_some(), "Make a pack from one sound");
    p.make = None;
    p.pop = Some((Pop::Pack, (10.0, 10.0, 100.0, 24.0)));
    assert!(p.popups(&mut cx).is_some());
}

/// Order 090 (v8): several keys are picked like files on the desktop - Ctrl-click adds / takes away, Shift-click = every key
/// between, a box drag, Clear / Esc / a click between the keys let go; "Change N keys" opens the N keys window.
#[test]
fn several_keys_are_picked_by_ctrl_shift_and_a_box() {
    let mut p = page();
    let a = idx_of(&p, 0x1E);
    let d = idx_of(&p, 0x20);
    click(&mut p, idx(K_KEY, a));
    assert_eq!(p.sel, Some(0x1E), "a plain click opens the key");
    click(&mut p, sub(K_KD, "x"));
    click_mods(&mut p, idx(K_KEY, d), true, false);
    assert_eq!(p.pick, vec![0x20]);
    click_mods(&mut p, idx(K_KEY, a), true, false);
    assert_eq!(p.pick, vec![0x20, 0x1E]);
    assert_eq!(p.sel, None, "a Ctrl-click never opens a window");
    click_mods(&mut p, idx(K_KEY, a), true, false);
    assert_eq!(p.pick, vec![0x20], "Ctrl-click again takes it away");
    // Shift-click from the anchor (A) to G: A S D F G
    let g = idx_of(&p, 0x22);
    click_mods(&mut p, idx(K_KEY, g), false, true);
    let mut picked = p.pick.clone();
    picked.sort_unstable();
    assert_eq!(picked, vec![0x1E, 0x1F, 0x20, 0x21, 0x22]);
    assert_eq!(p.reading(&p.pick), vec![0x1E, 0x1F, 0x20, 0x21, 0x22], "reading order");
    // Change N keys -> the N keys window
    click(&mut p, K_CHG);
    assert!(p.many);
    click(&mut p, sub(K_KN, "x"));
    assert!(!p.many && p.pick.len() == 5, "closing it keeps the keys picked");
    click(&mut p, K_CLR);
    assert!(p.pick.is_empty());
    // a box: from Q's middle to D's middle picks Q W E (row 2) and A S D (row 3)
    let w = 544.0 - 16.0;
    let s = w / (p.w_keys * pic::U);
    let q = idx_of(&p, 0x10);
    let bq = pic::boxes(&p.keys[q], s)[0];
    let bd = pic::boxes(&p.keys[d], s)[0];
    let origin = (100.0, 200.0);
    let at = |b: (f32, f32, f32, f32)| (origin.0 + b.0 + b.2 / 2.0, origin.1 + b.1 + b.3 / 2.0);
    with_cx(|cx| {
        let (x, y) = at(bq);
        p.event(&Ev::Press(idx(K_KEY, q), x, y, (origin.0 + bq.0, origin.1 + bq.1, bq.2, bq.3)), cx);
        let (x2, y2) = at(bd);
        p.event(&Ev::Drag(idx(K_KEY, q), x2, y2, (0.0, 0.0, 0.0, 0.0)), cx);
        p.event(&Ev::Release(idx(K_KEY, q)), cx);
        p.event(&Ev::Click(idx(K_KEY, q)), cx);
    });
    let mut picked = p.pick.clone();
    picked.sort_unstable();
    assert_eq!(picked, vec![0x10, 0x11, 0x12, 0x1E, 0x1F, 0x20]);
    assert_eq!(p.sel, None, "the end of a box is not a click on the key");
    // Esc lets go of them
    with_cx(|cx| p.event(&Ev::Key(crate::ui::cx::PAGE, 0x1B), cx));
    assert!(p.pick.is_empty());
}

/// Order 090: a key's Sound part - your file on top of the pack (dropped on Press), the pack's sound off for that key only,
/// Press = None; the picture's dot and the foot count follow.
#[test]
fn a_keys_own_sound_is_set_in_its_window() {
    let mut p = page();
    key_click(&mut p, 0x1C);
    with_cx(|cx| p.event(&Ev::Drop(sub(K_SND, "press"), vec![r"C:\x\boing.wav".into()]), cx));
    let l = p.prefs.keys.of(0x1C);
    assert_eq!(l.press.as_deref(), Some("boing.wav"));
    assert!(l.pack_on, "the pack's sound stays: yours is on top");
    // a file that isn't a sound says why
    with_cx(|cx| p.event(&Ev::Drop(sub(K_SND, "press"), vec![r"C:\x\notes.txt".into()]), cx));
    assert_eq!(p.prefs.keys.of(0x1C).press.as_deref(), Some("boing.wav"));
    // pitch slider
    with_cx(|cx| p.event(&Ev::Press(sub(K_SND, "pitch"), 0.0, 0.0, (0.0, 0.0, 200.0, 20.0)), cx));
    assert_eq!(p.prefs.keys.of(0x1C).pitch, -12.0);
    // the pack's sound off for this key only
    click(&mut p, sub(K_SND, "pack"));
    assert!(!p.prefs.keys.of(0x1C).pack_on);
    assert!(p.prefs.keys.of(0x1E).pack_on, "other keys keep it");
    // Release: None
    press(&mut p, sub(K_SND, "rel"));
    click(&mut p, sub(K_SND, "rel"));
    click(&mut p, idx(sub(K_SND, "menu"), 1));
    assert_eq!(p.prefs.keys.of(0x1C).release, bu_keysound::Release::None);
    // Press = None (the "Remove your sound" link is gone, pack-noise-v1): only the file goes
    press(&mut p, sub(K_SND, "press"));
    click(&mut p, sub(K_SND, "press"));
    click(&mut p, idx(sub(K_SND, "menu"), 0));
    let l = p.prefs.keys.of(0x1C);
    assert!(l.press.is_none() && !l.pack_on);
    assert_eq!(p.prefs.keys.with_own().count(), 1, "still marked: its pack sound is off");
}

/// Order 090 / 098: "N keys" gives the picked keys one file; Pitch and Loudness are Same (one value, the special keys with
/// their shape on top) or Random (a fixed difference per normal key).
#[test]
fn n_keys_get_one_sound_with_the_same_pitch_and_a_little_random_loudness() {
    let mut p = page();
    let letters = [0x1E, 0x1F, 0x20, 0x21, 0x22];
    p.pick = vec![0x1E, 0x1F, 0x20, 0x21, 0x22, 0x39, 0x1C];
    click(&mut p, K_CHG);
    assert!(p.many);
    // Pitch: Same, +2 (before the file: kept for when it comes)
    p.snd.vary.pitch.set_value(true, 2.0);
    with_cx(|cx| p.event(&Ev::Drop(sub(K_SND, "press"), vec![r"C:\x\tick.wav".into()]), cx));
    let pitches: Vec<f32> = letters.iter().map(|c| p.prefs.keys.of(*c).pitch).collect();
    assert!(pitches.iter().all(|v| *v == 2.0), "every normal key identical: {pitches:?}");
    assert_eq!(p.prefs.keys.of(0x39).pitch, -3.0, "Space = +2 and its shape (-5)");
    assert_eq!(p.prefs.keys.of(0x1C).pitch, -1.0, "Enter = +2 and its shape (-3)");
    assert!(p.prefs.keys.iter().all(|(_, l)| l.press.as_deref() == Some("tick.wav")));
    // Loudness: Random (fixed per key); Space keeps its shape
    click(&mut p, idx(sub(K_SND, "lm"), 1));
    let louds: Vec<u16> = letters.iter().map(|c| p.prefs.keys.of(*c).loud).collect();
    assert!(louds.iter().all(|l| (90..=110).contains(l)) && louds.iter().any(|l| *l != 100), "{louds:?}");
    assert_eq!(p.prefs.keys.of(0x39).loud, 112, "Space: 100 % and its shape, no random");
    // the pack's sound off for all of them
    click(&mut p, sub(K_SND, "pack"));
    assert!(letters.iter().all(|c| !p.prefs.keys.of(*c).pack_on));
}

#[test]
fn the_keys_window_opens_on_a_key_and_closes_with_its_x_beside_it_or_esc() {
    let mut p = page();
    let caps = idx_of(&p, 0x3A);
    with_cx(|cx| p.event(&Ev::Click(idx(K_KEY, caps)), cx));
    assert_eq!(p.sel, Some(0x3A));
    assert!(p.key_at > 0.0, "its open motion starts now");
    // × and a click beside it close it; a click inside is its own
    click(&mut p, sub(K_KD, "win"));
    assert_eq!(p.sel, Some(0x3A));
    click(&mut p, sub(K_KD, "x"));
    assert_eq!(p.sel, None);
    click(&mut p, idx(K_KEY, caps));
    click(&mut p, sub(K_KD, "out"));
    assert_eq!(p.sel, None);
    // Esc: a list first, then the window
    click(&mut p, idx(K_KEY, caps));
    p.pop = Some((Pop::Action, (0.0, 0.0, 100.0, 24.0)));
    p.popup_dismiss();
    assert!(p.pop.is_none() && p.sel.is_some());
    p.popup_dismiss();
    assert_eq!(p.sel, None);
}

/// The ready-made macros are in the key window's macro list (the Macros block is gone, Order 090 ADD).
#[test]
fn a_ready_made_macro_is_added_and_bound_from_the_keys_window() {
    let mut p = page();
    key_click(&mut p, 0x44);
    click(&mut p, idx(K_MODE, 3));
    press(&mut p, sub(K_MAC, "list"));
    click(&mut p, sub(K_MAC, "list"));
    // [New macro] [Ready-made] [Type my e-mail] [Open 3 sites] ...
    click(&mut p, idx(sub(K_MAC, "menu"), 3));
    assert_eq!(p.model.macros.len(), 1);
    assert_eq!(p.model.macros[0].name, "Open 3 sites");
    assert_eq!(p.model.macros[0].steps.len(), 5);
    assert_eq!(p.model.binds.get(0x44), Some(&Bind::Macro(p.model.macros[0].id.clone())));
}

#[test]
fn the_repeat_window_reaches_400_ms_and_the_sound_list_has_v8s_items() {
    let mut p = page();
    assert_eq!(p.prefs.s.repeat_ms, 0, "off by default");
    with_cx(|cx| p.event(&Ev::Press(K_REP, 100.0, 0.0, (0.0, 0.0, 200.0, 20.0)), cx));
    assert!(p.prefs.s.repeat_ms > 0 && p.prefs.s.repeat_ms <= 400);
    p.set_repeat(1.0);
    assert_eq!(p.prefs.s.repeat_ms, 400, "Order 090 (E26): up to 400 ms");
    p.set_repeat(0.2501);
    assert_eq!(p.prefs.s.repeat_ms % 5, 0, "steps of 5");
    p.set_repeat(0.0);
    assert_eq!(p.prefs.s.repeat_ms, 0);
    let names: Vec<String> = p.list(Pop::Pack).into_iter().map(|(l, _, _)| l).collect();
    let tail: Vec<&str> = names.iter().rev().take(3).rev().map(String::as_str).collect();
    assert_eq!(tail, ["Get more sounds…", "Import…", "Make a pack from one sound…"], "{names:?}");
}

/// Order 059 (the owner: J -> a macro with "J" "doesnt work anymore at all, its like an infinite loop"): a key that would press
/// itself is left as Windows made it, with a plain line; the same macro typing the letter is fine.
#[test]
fn a_key_that_presses_itself_is_left_as_windows_made_it() {
    let mut p = page();
    let j = 0x24; // the J key; the test layout's virtual key is its scancode
    let id = p.model.new_macro().unwrap();
    p.model.macros[0].steps = vec![Step::Keys(vec![j])];
    p.model.set_macro(j, &id).unwrap();
    p.save();
    assert_eq!(p.model.binds.get(j), None, "J stays as Windows made it");
    assert_eq!(p.model.mode(j, false), Mode::Normal);
    assert!(p.err.as_deref().unwrap_or("").contains("itself"), "{:?}", p.err);
    assert!(p.loop_note.is_some());
    // typing the letter instead is fine on the same key
    p.model.macros[0].steps = vec![Step::Type("j".into())];
    p.model.set_macro(j, "m1").unwrap();
    p.save();
    assert_eq!(p.model.binds.get(j), Some(&Bind::Macro("m1".into())), "a typed text can run on its own key");
    // J -> J as a remap is the key as Windows made it
    p.model.reset_all();
    p.model.set_remap(j, j).unwrap();
    assert_eq!(p.model.remap_count(), 0);
    // an old saved bind of this kind is dropped when the page opens
    let mut q = page();
    q.prefs.macros = vec![{
        let mut m = Macro::new("m9", "Press J");
        m.steps = vec![Step::Keys(vec![j])];
        m
    }];
    q.prefs.binds.set(j, Bind::Macro("m9".into())).unwrap();
    q.model = Model::new(Vec::new(), q.prefs.binds.clone(), q.prefs.macros.clone());
    q.drop_self_loops();
    assert_eq!(q.model.binds.len(), 0);
}

/// Order 090: "Make a pack from one sound" - a press file (its name names the pack), the spread, Save = it is the sound and
/// in Your packs.
#[test]
fn a_pack_is_made_from_one_sound() {
    let mut p = page();
    press(&mut p, K_PACK);
    click(&mut p, K_PACK);
    let list = p.list(Pop::Pack);
    let i = list.iter().position(|(_, c, _)| *c == Choice::MakePack).unwrap();
    click(&mut p, idx(K_MENU, i));
    assert!(p.make.is_some());
    with_cx(|cx| p.event(&Ev::Drop(K_MPP, vec![r"C:\x\thock.wav".into()]), cx));
    let mp = p.make.clone().unwrap();
    assert_eq!(mp.press.as_ref().map(|f| f.1.as_str()), Some("thock.wav"));
    assert_eq!(mp.name, "thock", "named after the file");
    assert_eq!(mp.vary.pitch.mode, bu_keysound::layers::Mode::Same, "Same by default");
    // Loudness: Random
    click(&mut p, idx(sub(K_MPV, "lm"), 1));
    assert_eq!(p.make.as_ref().unwrap().vary.loud.mode, bu_keysound::layers::Mode::Random);
    click(&mut p, K_MPSAVE);
    assert!(p.make.is_none());
    assert_eq!(p.prefs.s.pack, Pack::Made("thock".into()));
    let names: Vec<String> = p.list(Pop::Pack).into_iter().map(|(l, _, _)| l).collect();
    assert!(names.iter().any(|n| n.starts_with("Your packs")) && names.contains(&"thock".to_string()));
    // the spread it saves: Space deepest
    let keys = bu_keysound::Made::spread(&bu_keysound::Vary::for_pack(), &pic::codes());
    let space = keys.iter().find(|k| k.0 == 0x39).unwrap().1;
    assert!(keys.iter().all(|k| k.1 >= space));
}

// ------------------------------------------------------------------ Order 061: Get more sounds

#[test]
fn get_more_sounds_opens_from_the_sound_list_shows_the_packs_and_closes() {
    let mut p = page();
    let list = p.list(Pop::Pack);
    assert!(list.iter().any(|(l, c, _)| l == "Get more sounds…" && *c == Choice::GetMore), "{list:?}");
    // never in the per-app list (that one has no import items either)
    assert!(!p.list(Pop::RulePack(0)).iter().any(|(_, c, _)| *c == Choice::GetMore));
    p.pop = Some((Pop::Pack, (0.0, 0.0, 170.0, 24.0)));
    with_cx(|cx| p.choose(Choice::GetMore, cx));
    assert!(p.get_open && p.pop.is_none());
    let g = gallery::snapshot();
    assert_eq!(g.list.len(), 3, "test copies show three made-up packs and touch no network");
    with_cx(|cx| {
        let el = p.getter(cx);
        assert!(el.is_some());
    });
    // Order 090 (E21): a row's Get is keyed by the pack's id
    click(&mut p, sub(K_GROW, &g.list[0].id));
    assert!(p.get_open && p.get_msg.is_none());
    // a click that is not ours falls through
    assert!(!with_cx(|cx| p.get_clicked(K_PLAY, cx)));
    click(&mut p, K_GDONE);
    assert!(!p.get_open);
    // Esc closes it too
    with_cx(|cx| p.open_get(cx));
    p.popup_dismiss();
    assert!(!p.get_open);
}

// ------------------------------------------------------------------ Order 076

/// Order 076, the owner: "i change the sound, switch tabs, come back and its back to ... the one i had selected before" (a
/// downloaded pack). Cause: the page is made new at every open, but the finished "Get" job stays in the job list - the page
/// acted on its end again and made the downloaded pack the sound again. A job's end is acted on once.
#[test]
fn a_sound_picked_after_a_download_stays_when_the_tab_is_reopened() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    // a "Get" that ended earlier (its job stays in the list)
    let id = crate::services::with(|s| s.start_job(gallery::JOB_GET, |_| Ok("Downloaded".to_string())).unwrap()).unwrap();
    for _ in 0..200 {
        if crate::services::with(|s| s.job(gallery::JOB_GET)).flatten().is_some_and(|v| v.end.is_some()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let build = |p: &mut Keyboard| {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        cx.page = "kbd";
        p.build(&mut cx);
    };
    let mut p = page();
    build(&mut p);
    assert_eq!(p.prefs.s.pack, Pack::Imported("Downloaded".into()), "the first time, the download is the sound ({id:?})");
    // he picks another sound in the list
    p.pop = Some((Pop::Pack, (0.0, 0.0, 170.0, 24.0)));
    with_cx(|cx| p.choose(Choice::Pack(Pack::Builtin(PackId::Tactile)), cx));
    assert_eq!(p.prefs.s.pack, Pack::Builtin(PackId::Tactile));
    // he leaves the tab and comes back, twice
    for _ in 0..2 {
        p.close();
        p.open(&Env { test: true, ..Env::default() }, 0.0);
        build(&mut p);
        assert_eq!(p.prefs.s.pack, Pack::Builtin(PackId::Tactile), "the pick is still the sound");
    }
    crate::services::shutdown();
}

#[test]
fn a_jobs_end_is_acted_on_once() {
    let mut s = Seen(Vec::new());
    assert!(s.first(("a", 1u32)));
    assert!(!s.first(("a", 1)));
    assert!(s.first(("b", 1)) && s.first(("a", 2)));
    for i in 10..200u32 {
        s.first(("a", i));
    }
    assert!(s.0.len() <= 64, "the list never grows");
}

#[test]
fn play_on_is_a_three_way_choice_and_is_saved() {
    let mut p = page();
    assert_eq!(p.prefs.s.play_on, PlayOn::Both);
    click(&mut p, idx(K_PLAYON, 1));
    assert_eq!(p.prefs.s.play_on, PlayOn::Press);
    click(&mut p, idx(K_PLAYON, 2));
    assert_eq!(p.prefs.s.play_on, PlayOn::Release);
    click(&mut p, idx(K_PLAYON, 0));
    assert_eq!(p.prefs.s.play_on, PlayOn::Both);
    let labels: Vec<&str> = PlayOn::ALL.iter().map(|x| x.label()).collect();
    assert_eq!(labels, ["Press + release", "Press only", "Release only"]);
}

#[test]
fn the_search_box_narrows_the_get_more_sounds_list_by_name_or_tag() {
    let l = |name: &str, tags: &[&str]| bu_keysound::gallery::Listed { id: "x".into(), name: name.into(), tags: tags.iter().map(|t| t.to_string()).collect(), pre_installed: false };
    let (a, b) = (l("Model F XT", &["keyboard", "retro"]), l("Bubble pop", &["fun"]));
    assert!(getter::matches(&a, "") && getter::matches(&b, "  "));
    assert!(getter::matches(&a, "model") && !getter::matches(&b, "model"));
    assert!(getter::matches(&a, "RETRO") && !getter::matches(&b, "retro"), "tags count, any letter case");
    let mut p = page();
    with_cx(|cx| p.open_get(cx));
    with_cx(|cx| p.event(&Ev::Char(K_GSEARCH, 'b'), cx));
    with_cx(|cx| p.event(&Ev::Char(K_GSEARCH, 'u'), cx));
    assert_eq!(p.get_q, "bu");
    with_cx(|cx| p.event(&Ev::Key(K_GSEARCH, 0x08), cx));
    assert_eq!(p.get_q, "b");
    click(&mut p, sub(K_GSEARCH, "x"));
    assert!(p.get_q.is_empty());
    // the window builds with a search that matches nothing, and with an installed pack in use
    p.get_q = "zzz".into();
    with_cx(|cx| assert!(p.getter(cx).is_some()));
    p.get_q.clear();
    p.prefs.s.pack = Pack::Imported("Bubble pop".into());
    with_cx(|cx| assert!(p.getter(cx).is_some()));
}

#[test]
fn an_imported_sound_is_removed_after_asking_and_the_settings_fall_back() {
    let mut p = page();
    let gone = Pack::Imported("Holy Panda".into());
    p.prefs.s.pack = gone.clone();
    p.prefs.s.rules = vec![Rule { exe: "notepad.exe".into(), pack: Some(gone.clone()) }, Rule { exe: "chrome.exe".into(), pack: Some(Pack::Builtin(PackId::Clicky)) }];
    p.prefs.s.pad_pack = Some(gone.clone());
    // the question is asked first; a click elsewhere (or Esc) cancels it and removes nothing
    p.ask_del = Some((gone.clone(), (10.0, 10.0)));
    with_cx(|cx| assert!(p.popups(cx).is_some())); // the question is painted
    p.popup_dismiss();
    assert!(p.ask_del.is_none());
    assert_eq!(p.prefs.s.pack, gone);
    p.ask_del = Some((gone.clone(), (10.0, 10.0)));
    click(&mut p, sub(K_DELQ, "no"));
    assert!(p.ask_del.is_none());
    assert_eq!(p.prefs.s.pack, gone);
    // Remove: the general sound goes back to Linear, the program's own sound to Off, the controller's to "Same as keyboard"
    p.ask_del = Some((gone.clone(), (10.0, 10.0)));
    click(&mut p, sub(K_DELQ, "go"));
    assert!(p.ask_del.is_none());
    assert_eq!(p.prefs.s.pack, Pack::Builtin(PackId::Linear));
    assert_eq!(p.prefs.s.rules[0].pack, None);
    assert_eq!(p.prefs.s.rules[1].pack, Some(Pack::Builtin(PackId::Clicky)));
    assert_eq!(p.prefs.s.pad_pack, None);
    // a pack that was not in use changes nothing but itself
    assert!(!p.forget_pack(&Pack::Imported("Other".into())));
}

// ------------------------------------------------------------------ pictures (run: cargo test -p bu-app keyboard::tests::pictures -- --ignored)

const SCRATCH: &str = r"C:\BoylerUtilities-scratch/P098";

/// The page painted by the app's own painter; `popup` = with its windows / lists on top, in the window's 600 x 520 at
/// `scroll` (else the whole page).
fn paint_page_to(p: &mut Keyboard, name: &str, popup: bool, dir: &str, scroll: f32) {
    let g = Gfx::new(1.0);
    let icons = Icons::new();
    let mut st = State::default();
    let mut cx = Cx::new(5000.0, false, &g, &mut st);
    cx.page = "kbd";
    let kids = p.build(&mut cx);
    let root = El::block().w(600.0).pad(2.0, 28.0, 18.0, 28.0).children(kids);
    let laid = Laid::new(&g, root, 600.0, None);
    let h = if popup { crate::ui::WIN_H } else { (laid.height + 20.0).ceil().max(400.0) };
    let bg = crate::gfx::Rgba::rgb(29, 32, 48);
    let Some(mut s) = crate::gfx::new_surface(600, h as i32) else { return };
    g.begin(s.canvas());
    g.fill_rect(0.0, 0.0, 600.0, h, bg);
    g.end();
    let base = s.image_snapshot();
    g.begin(s.canvas());
    laid.paint(&g, &icons, 0.0, -scroll, Some(&base));
    g.end();
    if popup {
        if let Some(pop) = p.popups(&mut cx) {
            let base = s.image_snapshot();
            let l2 = Laid::new(&g, El::block().w(600.0).h(h).child(pop), 600.0, Some(h));
            g.begin(s.canvas());
            l2.paint(&g, &icons, 0.0, 0.0, Some(&base));
            g.end();
        }
    }
    let px = crate::png::from_surface(&mut s);
    let _ = std::fs::create_dir_all(dir);
    // SAFETY: COM for the WIC encoder on this test thread.
    let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
    if let Err(e) = crate::png::save_png(&px, &format!("{dir}\\{name}.png")) {
        panic!("saving {name}: {e}");
    }
}

/// Order 090's pictures of the Keyboard tab (run: cargo test -p bu-app keyboard::tests::pictures -- --ignored).
#[test]
#[ignore]
fn pictures() {
    let mut p = page();
    p.prefs.on = true;
    p.prefs.s.volume = 40;
    p.model.set_remap(0x3A, 0x01).unwrap();
    p.model.set_preset(0x44, Preset::PlayPause).unwrap();
    let id = p.model.new_macro().unwrap();
    p.model.macros[0].name = "Open my notes".into();
    p.model.macros[0].steps = vec![Step::Down(0x5B), Step::Keys(vec![0x52]), Step::Up(0x5B), Step::Wait(300), Step::Type("notepad".into()), Step::Keys(vec![0x0D])];
    p.model.set_macro(0x57, &id).unwrap();
    p.model.applied = vec![Mapping { from: 0x3A, to: 0x01 }];
    p.prefs.keys.set(0x1C, bu_keysound::Layer { press: Some("boing.wav".into()), pitch: -4.0, loud: 120, ..Default::default() }).unwrap();
    p.prefs.keys.set(0x0E, bu_keysound::Layer { pack_on: false, press: Some("pop.mp3".into()), release: bu_keysound::Release::None, pitch: 1.0, ..Default::default() }).unwrap();
    p.prefs.keys.set(0x3A, bu_keysound::Layer { pack_on: false, ..Default::default() }).unwrap();
    paint_page_to(&mut p, "kbd_page_folded", false, SCRATCH, 0.0);
    p.card_open = true;
    paint_page_to(&mut p, "kbd_page_open", false, SCRATCH, 0.0);
    p.card_open = false;
    // the key windows
    p.select(Some(0x1C));
    paint_page_to(&mut p, "kbd_key_enter", true, SCRATCH, 0.0);
    p.select(Some(0x0E));
    paint_page_to(&mut p, "kbd_key_backspace", true, SCRATCH, 0.0);
    p.select(Some(0x57));
    paint_page_to(&mut p, "kbd_key_macro", true, SCRATCH, 0.0);
    p.select(None);
    // several keys picked, then the N keys window
    p.pick = vec![0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26];
    paint_page_to(&mut p, "kbd_picked", false, SCRATCH, 0.0);
    click(&mut p, K_CHG);
    click(&mut p, idx(sub(K_SND, "pm"), 1));
    with_cx(|cx| p.event(&Ev::Drop(sub(K_SND, "press"), vec![r"C:\x\tick.wav".into()]), cx));
    click(&mut p, idx(sub(K_SND, "lm"), 2));
    paint_page_to(&mut p, "kbd_n_keys", true, SCRATCH, 0.0);
    p.many = false;
    p.pick.clear();
    // the Sound list, Make a pack
    p.pop = Some((Pop::Pack, (300.0, 300.0, 190.0, 28.0)));
    p.made_list = vec!["My thock".into()];
    p.card_open = true;
    paint_page_to(&mut p, "kbd_sound_list", true, SCRATCH, 300.0);
    p.pop = None;
    p.open_make(0.0);
    with_cx(|cx| p.event(&Ev::Drop(K_MPP, vec![r"C:\x\thock.wav".into()]), cx));
    click(&mut p, idx(sub(K_MPV, "lm"), 1));
    paint_page_to(&mut p, "kbd_make_pack", true, SCRATCH, 0.0);
}

// ------------------------------------------------------------------ Order 096

fn texts_of(el: El, w: f32) -> Vec<String> {
    let g = Gfx::new(1.0);
    let laid = Laid::new(&g, El::block().w(w).child(el), w, None);
    laid.nodes.iter().filter_map(|n| if let crate::ui::el::Content::Text(t) = &n.el.content { Some(t.s.to_string()) } else { None }).collect()
}

/// the owner, Oct 10: "Look at all this fucking space" - the Keyboard sounds card shows every row, no "More" fold.
#[test]
fn the_keyboard_sounds_card_has_no_more_fold() {
    let mut p = page();
    p.prefs.on = true;
    p.card_open = true;
    let texts = with_cx(|cx| texts_of(El::col().children(p.build(cx)), 600.0));
    assert!(!texts.iter().any(|t| t == "More"), "{texts:?}");
    for row in ["Sound", "Volume", "Play on", "Ignore repeats within", "Off while a game is in front", "Try it", "Different in some apps"] {
        assert!(texts.iter().any(|t| t == row), "{row} is shown: {texts:?}");
    }
}

/// the owner, Oct 10: no "Pick what this key should do" line (and no divider for it); the Sound is ONE card: Pack's sound first.
#[test]
fn the_key_window_is_one_sound_card_and_has_no_pick_what_line() {
    let mut p = page();
    p.select(Some(0x1E));
    let texts = with_cx(|cx| {
        p.build(cx);
        texts_of(p.popups(cx).expect("the key window"), 600.0)
    });
    assert!(!texts.iter().any(|t| t.contains("Pick what")), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "Your sound"), "{texts:?}");
    let at = |s: &str| texts.iter().position(|t| t == s).unwrap_or_else(|| panic!("{s}: {texts:?}"));
    assert!(at("Sound") < at("Pack\u{2019}s sound") && at("Pack\u{2019}s sound") < at("Press") && at("Press") < at("Release"));
    assert!(at("Release") < at("Pitch") && at("Pitch") < at("Loudness"));
}

/// Order 090's picture showed "5B" / "52": a macro's keys are named (Win, R ...), also with no keyboard layout at hand.
#[test]
fn macro_steps_show_key_names_not_codes() {
    use crate::pages::btnwin::step_text;
    assert_eq!(step_text(&Step::Down(0x5B)), "Win down");
    assert_eq!(step_text(&Step::Keys(vec![0x5B, 0x52])), "Press Win + R");
    assert_eq!(step_text(&Step::Up(0x5B)), "Win up");
    assert_eq!(step_text(&Step::Keys(vec![0x11, 0x10, 0x4B])), "Press Ctrl + Shift + K");
    assert_eq!(step_text(&Step::Keys(vec![0x0D])), "Press Enter");
    assert_eq!(step_text(&Step::Keys(vec![0x74])), "Press F5");
}

/// Order 096: a known keyboard picks the size when the tab opens; a size picked by hand stays and wins.
#[test]
fn a_known_keyboard_picks_the_size_and_a_manual_pick_wins() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    let detected = |text: &'static str| {
        crate::services::with(|s| s.start_job(JOB_DETECT, move |_| Ok(text.to_string())).unwrap()).unwrap();
        for _ in 0..200 {
            if crate::services::with(|s| s.job(JOB_DETECT)).flatten().is_some_and(|v| v.end.is_some()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    };
    let build = |p: &mut Keyboard| {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        cx.page = "kbd";
        p.build(&mut cx);
    };
    let mut p = page();
    assert!(!p.prefs.size_manual);
    // unknown keyboard (empty answer): as today
    detected("");
    build(&mut p);
    assert_eq!(p.prefs.size, Size::Full);
    // Wooting 80HE
    detected("75");
    build(&mut p);
    assert_eq!(p.prefs.size, Size::P75);
    assert_eq!(p.keys.len(), pic::keys(Size::P75).0.len(), "the picture follows");
    assert!(!p.prefs.size_manual, "an automatic pick is not a manual one");
    // he picks TKL himself: it stays, now and at the next open, whatever is plugged in
    p.change_size(Size::Tkl);
    assert!(p.prefs.size_manual);
    detected("60");
    build(&mut p);
    assert_eq!(p.prefs.size, Size::Tkl);
    p.close();
    p.open(&Env { test: true, ..Env::default() }, 0.0);
    assert_eq!((p.prefs.size, p.prefs.size_manual), (Size::Tkl, true));
    crate::services::shutdown();
}
