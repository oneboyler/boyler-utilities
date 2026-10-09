//! The Keyboard page, driven like the frame drives it (clicks by key) and pictured (the page's boxes painted by the app's own
//! painter over a flat backdrop) - all with fakes: nothing here touches the PC, the registry, a stream or a key.

use super::*;
use crate::gfx::Gfx;
use crate::icons::Icons;
use crate::ui::cx::State;
use crate::ui::lay::Laid;

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

#[test]
fn a_fresh_page_is_off_full_size_and_unchanged() {
    let p = page();
    assert!(!p.prefs.on, "key sounds are off by default: nothing listens");
    assert_eq!(p.prefs.size, Size::Full);
    assert_eq!(p.model.remap_count() + p.model.bind_count(), 0);
    assert_eq!(p.keys.len(), pic::keys(Size::Full).0.len());
    // labelled by the (test) Croatian layout
    let z = idx_of(&p, 0x15);
    assert_eq!(p.texts[z], "Z");
    assert_eq!(p.texts[idx_of(&p, 0x27)], "Č");
}

#[test]
fn a_remap_is_made_by_picking_the_key_then_the_new_key() {
    let mut p = page();
    let caps = idx_of(&p, 0x3A);
    let esc = idx_of(&p, 0x01);
    click(&mut p, idx(K_KEY, caps));
    assert_eq!(p.sel, Some(0x3A));
    // the card: Remap
    click(&mut p, idx(K_MODE, 1));
    assert!(p.choosing, "now it waits for the new key");
    click(&mut p, idx(K_KEY, esc));
    assert_eq!(p.model.remap_of(0x3A), Some(0x01));
    assert!(!p.choosing);
    assert!(p.model.dirty(), "Apply is needed (admin + restart)");
    assert_eq!(p.model.mode(0x3A, false), Mode::Remap);
    // it can't become itself (the field is clicked: it waits for a key again)
    click(&mut p, K_TARGET);
    click(&mut p, idx(K_KEY, caps));
    // (the key clicked while choosing is the target: Caps -> Caps is refused, the old remap stays)
    assert_eq!(p.model.remap_of(0x3A), Some(0x01));
    assert!(p.err.is_some());
    // Reset this key
    click(&mut p, K_RESETKEY);
    assert_eq!(p.model.remap_of(0x3A), None);
}

#[test]
fn a_key_can_get_an_action_or_a_macro() {
    let mut p = page();
    let f10 = idx_of(&p, 0x44);
    click(&mut p, idx(K_KEY, f10));
    click(&mut p, idx(K_MODE, 2));
    assert_eq!(p.model.binds.get(0x44), Some(&Bind::Preset(Preset::PlayPause)), "a first action is chosen at once");
    // the list offers the presets, grouped
    let list = p.list(Pop::Action);
    let names: Vec<&str> = list.iter().map(|(l, _, _)| l.as_str()).collect();
    for want in ["Media", "Play / pause", "Volume up", "Mute", "Open", "A website…"] {
        assert!(names.contains(&want), "{want} in {names:?}");
    }
    // a new macro through the card's list
    click(&mut p, idx(K_MODE, 3));
    assert_eq!(p.model.binds.get(0x44), Some(&Bind::Preset(Preset::PlayPause)), "no macro yet: the action stays until one is chosen");
    p.pop = Some((Pop::MacroList, (0.0, 0.0, 100.0, 24.0)));
    let items = p.list(Pop::MacroList);
    assert_eq!(items.last().map(|(l, _, _)| l.as_str()), Some("New macro…"));
    with_cx(|cx| p.choose(Choice::NewMacro, cx));
    assert_eq!(p.model.macros.len(), 1);
    assert_eq!(p.model.binds.get(0x44), Some(&Bind::Macro("m1".into())));
    assert!(p.edit.is_some(), "the macro window opens");
    assert!(!p.model.dirty(), "an action or macro never needs Apply");
}

#[test]
fn numpad_keys_cannot_carry_an_action() {
    let mut p = page();
    let n7 = idx_of(&p, 0x47);
    click(&mut p, idx(K_KEY, n7));
    click(&mut p, idx(K_MODE, 2));
    assert!(p.model.binds.get(0x47).is_none());
    assert!(p.err.as_deref().unwrap_or("").contains("Numpad"));
}

#[test]
fn the_size_switch_redraws_the_keys_and_clears_the_choice() {
    let mut p = page();
    let caps = idx_of(&p, 0x3A);
    click(&mut p, idx(K_KEY, caps));
    click(&mut p, idx(K_SIZE, 3));
    assert_eq!(p.prefs.size, Size::P60);
    assert_eq!(p.sel, None);
    assert!(p.keys.iter().all(|k| k.code != 0x3B), "no function row on a 60 %");
    assert!(p.keys.iter().any(|k| k.code == 0x3A));
}

#[test]
fn reset_all_clears_every_key_but_keeps_the_macros() {
    let mut p = page();
    p.model.set_remap(0x3A, 0x01).unwrap();
    let id = p.new_macro().unwrap();
    p.model.set_macro(0x44, &id).unwrap();
    click(&mut p, K_RESETALL);
    assert_eq!(p.model.remap_count() + p.model.bind_count(), 0);
    assert_eq!(p.model.macros.len(), 1);
}

#[test]
fn a_macro_is_built_step_by_step_and_reordered() {
    let mut p = page();
    let id = p.new_macro().unwrap();
    p.open_editor(&id, 0.0);
    for kind in [1u8, 2, 3] {
        p.pop = Some((Pop::StepAdd, (0.0, 0.0, 0.0, 0.0)));
        with_cx(|cx| p.choose(Choice::Step(kind), cx));
    }
    let steps = p.model.macros[0].steps.clone();
    assert_eq!(steps, vec![Step::Type(String::new()), Step::Wait(200), Step::Open(String::new())]);
    // type into the text step, move it down, delete the open step
    with_cx(|cx| {
        for c in "hi".chars() {
            p.event(&Ev::Char(sub(idx(K_STEP, 0), "val"), c), cx);
        }
    });
    assert_eq!(p.model.macros[0].steps[0], Step::Type("hi".into()));
    click(&mut p, sub(idx(K_STEP, 0), "dn"));
    assert_eq!(p.model.macros[0].steps[1], Step::Type("hi".into()));
    click(&mut p, sub(idx(K_STEP, 2), "rm"));
    assert_eq!(p.model.macros[0].steps.len(), 2);
    // a wait takes digits
    with_cx(|cx| {
        p.event(&Ev::Key(sub(idx(K_STEP, 0), "val"), 0x08), cx);
    });
    assert_eq!(p.model.macros[0].steps[0], Step::Wait(20));
    // Done closes it; the macro is kept
    click(&mut p, K_MDDONE);
    assert!(p.edit.is_none());
    assert_eq!(p.model.macros.len(), 1);
    // Delete macro removes it and frees its keys
    p.model.set_macro(0x44, &id).unwrap();
    p.open_editor(&id, 0.0);
    click(&mut p, K_MDDEL);
    assert!(p.model.macros.is_empty());
    assert!(p.model.binds.get(0x44).is_none());
}

#[test]
fn recording_keeps_only_what_is_pressed_until_stop() {
    let mut p = page();
    let id = p.new_macro().unwrap();
    p.open_editor(&id, 0.0);
    click(&mut p, K_MDREC);
    assert!(p.rec);
    // (combo_of reads the real modifier state; the test presses a plain key)
    with_cx(|cx| p.event(&Ev::Key(K_MDREC, 0x41), cx));
    with_cx(|cx| p.event(&Ev::Key(K_MDREC, 0x42), cx));
    assert!(matches!(p.model.macros[0].steps.last(), Some(Step::Keys(v)) if v.last() == Some(&0x42)));
    assert_eq!(p.model.macros[0].steps.iter().filter(|s| matches!(s, Step::Keys(_))).count(), 2);
    // Esc stops it
    with_cx(|cx| p.event(&Ev::Key(K_MDREC, 0x1B), cx));
    assert!(!p.rec);
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
    let id = p.new_macro().unwrap();
    p.model.set_macro(0x57, &id).unwrap();
    for (sel, mode) in [(0x3Au16, Mode::Remap), (0x44, Mode::Action), (0x57, Mode::Macro), (0x1E, Mode::Normal)] {
        p.select(Some(sel));
        p.want = Some(mode);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        assert!(!p.build(&mut cx).is_empty());
    }
    p.open_editor(&id, 0.0);
    let mut st = State::default();
    let mut cx = Cx::new(0.0, false, &g, &mut st);
    assert!(p.popups(&mut cx).is_some(), "the macro window");
    p.pop = Some((Pop::Pack, (10.0, 10.0, 100.0, 24.0)));
    assert!(p.popups(&mut cx).is_some());
}

// ------------------------------------------------------------------ pictures (run: cargo test -p bu-app keyboard::tests::pictures -- --ignored)

const SCRATCH: &str = r"C:\BoylerUtilities-scratch\K58";

fn paint_page(p: &mut Keyboard, name: &str, popup: bool) {
    let g = Gfx::new(1.0);
    let icons = Icons::new();
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st);
    cx.page = "kbd";
    let kids = p.build(&mut cx);
    let root = El::block().w(600.0).pad(2.0, 28.0, 18.0, 28.0).children(kids);
    let laid = Laid::new(&g, root, 600.0, None);
    let h = (laid.height + 40.0).ceil().max(400.0);
    let bg = crate::gfx::Rgba::rgb(29, 32, 48);
    let Some(mut s) = crate::gfx::new_surface(600, h as i32) else { return };
    g.begin(s.canvas());
    g.fill_rect(0.0, 0.0, 600.0, h, bg);
    g.end();
    let base = s.image_snapshot();
    g.begin(s.canvas());
    laid.paint(&g, &icons, 0.0, 0.0, Some(&base));
    g.end();
    if popup {
        if let Some(pop) = p.popups(&mut cx) {
            let l2 = Laid::new(&g, El::block().w(600.0).h(h).child(pop), 600.0, Some(h));
            g.begin(s.canvas());
            l2.paint(&g, &icons, 0.0, 0.0, Some(&base));
            g.end();
        }
    }
    let px = crate::png::from_surface(&mut s);
    let _ = std::fs::create_dir_all(SCRATCH);
    // SAFETY: COM for the WIC encoder on this test thread.
    let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
    if let Err(e) = crate::png::save_png(&px, &format!("{SCRATCH}\\{name}.png")) {
        panic!("saving {name}: {e}");
    }
}

#[test]
#[ignore]
fn pictures() {
    let mut p = page();
    p.prefs.on = true;
    p.model.set_remap(0x3A, 0x01).unwrap();
    p.model.set_remap(0xE038, 0xE01D).unwrap();
    p.model.set_preset(0x44, Preset::PlayPause).unwrap();
    let id = p.new_macro().unwrap();
    p.model.macros[0].steps = vec![Step::Keys(vec![0x5B, 0x52]), Step::Wait(300), Step::Type("notepad".into()), Step::Keys(vec![0x0D])];
    p.model.set_macro(0x57, &id).unwrap();
    p.model.applied = vec![Mapping { from: 0x3A, to: 0x01 }];
    paint_page(&mut p, "kbd_full", false);
    p.select(Some(0x3A));
    p.want = Some(Mode::Remap);
    paint_page(&mut p, "kbd_key_remap", false);
    p.select(Some(0x44));
    paint_page(&mut p, "kbd_key_action", false);
    p.select(Some(0x57));
    paint_page(&mut p, "kbd_key_macro", false);
    p.change_size(Size::Tkl);
    paint_page(&mut p, "kbd_tkl", false);
    p.change_size(Size::P60);
    paint_page(&mut p, "kbd_60", false);
    p.open_editor(&id, 0.0);
    paint_page(&mut p, "kbd_macro_window", true);
    p.close_editor();
    p.pop = Some((Pop::Pack, (330.0, 160.0, 170.0, 24.0)));
    paint_page(&mut p, "kbd_pack_list", true);
}
