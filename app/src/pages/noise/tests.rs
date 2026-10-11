//! The Noise page, driven like the frame drives it (clicks by key) and pictured - all with a stand-in player: nothing here
//! plays a sound, opens a stream or touches the PC.

use super::*;
use bu_noise::{Mix, Sound};
use crate::gfx::Gfx;
use crate::icons::Icons;
use crate::settings::scratch::Scratch;
use crate::settings::SettingsStore;
use crate::ui::cx::State;
use crate::ui::lay::Laid;

fn page() -> Noise {
    let mut p = Noise::default();
    p.open(&Env { test: true, ..Env::default() }, 0.0);
    p
}

fn with_cx<R>(f: impl FnOnce(&mut Cx) -> R) -> R {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st);
    cx.page = "nse";
    f(&mut cx)
}

fn click(p: &mut Noise, k: Key) {
    with_cx(|cx| p.event(&Ev::Click(k), cx));
}

fn press(p: &mut Noise, k: Key, x: f32, r: (f32, f32, f32, f32)) {
    with_cx(|cx| p.event(&Ev::Press(k, x, 0.0, r), cx));
}

#[test]
fn a_fresh_page_is_brown_quiet_timer_off_and_not_playing() {
    let p = page();
    assert_eq!(p.prefs.pick, Pick::Preset(Kind::Brown));
    assert_eq!(p.prefs.volume, 10, "quiet by default");
    assert_eq!(p.prefs.sleep, None);
    assert!(!p.status().playing, "off until Play is pressed");
    assert_eq!(p.id(), "nse");
    assert_eq!(p.name(), "Noise");
}

#[test]
fn play_starts_stop_ends_and_the_button_says_which() {
    let mut p = page();
    click(&mut p, K_PLAY);
    assert!(p.status().playing);
    assert_eq!(p.status().sound, Some(Kind::Brown.into()));
    assert_eq!(p.status().volume, 10);
    click(&mut p, K_PLAY);
    assert!(!p.status().playing);
}

#[test]
fn nothing_plays_just_because_the_page_was_opened_or_built() {
    let mut p = page();
    with_cx(|cx| assert!(!p.build(cx).is_empty()));
    p.close();
    p.open(&Env { test: true, ..Env::default() }, 0.0);
    assert!(!p.status().playing);
}

#[test]
fn a_noise_is_picked_from_the_list_and_follows_while_it_plays() {
    let mut p = page();
    click(&mut p, K_PLAY);
    // the press before the click gives the list its anchor
    press(&mut p, K_KIND, 0.0, (300.0, 150.0, 130.0, 28.0));
    click(&mut p, K_KIND);
    assert!(p.pop.is_some(), "the list is open");
    with_cx(|cx| assert!(p.popup(cx).is_some()));
    let pink = Kind::ALL.iter().position(|k| *k == Kind::Pink).unwrap();
    click(&mut p, idx(K_MENU, pink));
    assert_eq!(p.prefs.pick, Pick::Preset(Kind::Pink));
    assert!(p.pop.is_none(), "the list closes on a pick");
    assert_eq!(p.status().sound, Some(Kind::Pink.into()), "the playing noise changes");
    p.popup_dismiss();
    assert!(p.pop.is_none());
}

#[test]
fn all_six_noises_are_in_the_list_in_order() {
    let names: Vec<&str> = Kind::ALL.iter().map(|k| k.name()).collect();
    assert_eq!(names, ["White", "Pink", "Brown", "Dark brown", "Grey", "Blue"]);
}

#[test]
fn the_volume_slider_sets_a_percent() {
    let mut p = page();
    let r = (100.0, 200.0, 150.0, 20.0);
    press(&mut p, K_VOL, 100.0 + 75.0, r);
    assert!((45..=55).contains(&p.prefs.volume), "half way = about 50 % ({})", p.prefs.volume);
    press(&mut p, K_VOL, 100.0 - 40.0, r);
    assert_eq!(p.prefs.volume, 0, "left of the slider is 0");
    press(&mut p, K_VOL, 900.0, r);
    assert_eq!(p.prefs.volume, 100);
}

#[test]
fn the_sleep_timer_is_one_of_the_five_choices() {
    let mut p = page();
    click(&mut p, K_PLAY);
    for (i, want) in [(1usize, Some(15u32)), (2, Some(30)), (3, Some(60)), (4, Some(90)), (0, None)] {
        click(&mut p, idx(K_SLEEP, i));
        assert_eq!(p.prefs.sleep, want);
    }
    click(&mut p, idx(K_SLEEP, 2));
    assert_eq!(p.status().sleep_left, Some(1800), "30 min counting while it plays");
}

#[test]
fn the_sleep_line_counts_down_in_minutes_then_fades() {
    let mut st = Status { playing: true, sleep_left: Some(27 * 60 - 20), ..Status::default() };
    assert_eq!(sleep_line(&st).as_deref(), Some("Stops in 27 min"));
    st.sleep_left = Some(61);
    assert_eq!(sleep_line(&st).as_deref(), Some("Stops in 2 min"));
    st.sleep_left = Some(60);
    assert_eq!(sleep_line(&st).as_deref(), Some("Stops in 1 min"));
    st.sleep_left = Some(30);
    assert_eq!(sleep_line(&st).as_deref(), Some("Stops in under a minute"));
    st.stopping = true;
    assert_eq!(sleep_line(&st).as_deref(), Some("Fading out"));
    st.playing = false;
    assert_eq!(sleep_line(&st), None);
    assert_eq!(sleep_line(&Status { playing: true, ..Status::default() }), None, "no timer, no line");
}

#[test]
fn the_page_asks_to_be_woken_only_while_a_timer_counts() {
    let mut p = page();
    assert_eq!(p.wake_at(1000.0), None, "nothing playing: no wake-ups, no frames");
    click(&mut p, K_PLAY);
    assert_eq!(sleep_wake(&p.status(), 1000.0), None, "playing without a timer: the sleep line changes by nothing");
    assert_eq!(p.wake_at(1000.0), Some(1000.0), "...but the tide asks for its first frame");
    click(&mut p, idx(K_SLEEP, 1));
    let at = sleep_wake(&p.status(), 1000.0).unwrap();
    assert!(p.wake_at(1000.0).unwrap() <= at);
    // 15 min = 900 s: the line changes when it reaches 14 min = 840 s, 60 s from now
    assert!((at - (1000.0 + 60_000.0 + 150.0)).abs() < 1.0, "{at}");
    // a build paints the line; the tick then has nothing new to say
    with_cx(|cx| {
        p.build(cx);
    });
    p.tick(1000.0); // (the tide's first frame)
    assert!(!p.tick(1000.0));
    p.fake.sleep_left = Some(14 * 60);
    assert!(p.tick(1000.0), "the minute changed: repaint");
}

#[test]
fn the_remembered_values_round_trip_and_never_say_it_plays() {
    let sc = Scratch::new("noise-round");
    let mut store = SettingsStore::open(sc.dir());
    assert_eq!(Prefs::load(&store), Prefs::default());
    let p = Prefs { pick: Pick::Preset(Kind::DarkBrown), volume: 33, sleep: Some(60), ..Prefs::default() };
    p.save(&mut store);
    assert_eq!(Prefs::load(&store), p);
    drop(store);
    let again = SettingsStore::open(sc.dir());
    assert_eq!(Prefs::load(&again), p, "read back from the file");
    let text = std::fs::read_to_string(again.path()).unwrap();
    for line in text.lines().filter(|l| l.starts_with("page:noise")) {
        let key = line.split('\t').nth(1).unwrap();
        assert!(["kind", "volume", "sleep", "custom", "mine"].contains(&key), "unexpected setting {key}");
    }
}

#[test]
fn broken_values_fall_back_to_the_defaults() {
    let sc = Scratch::new("noise-broken");
    let mut store = SettingsStore::open(sc.dir());
    let _ = store.set_str(scope_for_test(), "kind", "purple");
    let _ = store.set_i64(scope_for_test(), "volume", 900);
    let _ = store.set_i64(scope_for_test(), "sleep", 7);
    let p = Prefs::load(&store);
    assert_eq!(p.pick, Pick::Preset(Kind::Brown));
    assert_eq!(p.volume, 100, "clamped");
    assert_eq!(p.sleep, None, "7 min is not a choice");
}

fn scope_for_test() -> crate::settings::Scope<'static> {
    crate::settings::Scope::Page(prefs::PAGE)
}

#[test]
fn the_player_is_never_made_by_looking_at_the_page_or_the_tray() {
    // the tray's question and the page's start leave a player that was never made alone (no thread, no stream, no event)
    assert!(!glue::playing());
}

// ------------------------------------------------------------------ pictures (run: cargo test -p bu-app noise::tests::pictures -- --ignored)

const SCRATCH: &str = r"C:\BoylerUtilities-scratch\N80";

fn paint_page(p: &mut Noise, name: &str, popup: bool, w: f32) {
    let g = Gfx::new(1.0);
    let icons = Icons::new();
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st);
    cx.page = "nse";
    let kids = p.build(&mut cx);
    let root = El::block().w(600.0).pad(2.0, 28.0, 18.0, 28.0).children(kids);
    let laid = Laid::new(&g, root, 600.0, None);
    let h = (laid.height + 40.0).ceil().max(if popup { 520.0 } else { 300.0 });
    let bg = crate::gfx::Rgba::rgb(29, 32, 48);
    let Some(mut s) = crate::gfx::new_surface(w as i32, h as i32) else { return };
    g.begin(s.canvas());
    g.fill_rect(0.0, 0.0, w, h, bg);
    g.end();
    let base = s.image_snapshot();
    g.begin(s.canvas());
    laid.paint(&g, &icons, 0.0, 0.0, Some(&base));
    g.end();
    if popup {
        if let Some(pop) = p.popup(&mut cx) {
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
fn every_state_builds_and_paints() {
    let mut p = page();
    let g = Gfx::new(1.0);
    for state in 0..4 {
        match state {
            1 => click(&mut p, K_PLAY),
            2 => click(&mut p, idx(K_SLEEP, 2)),
            3 => {
                p.fake.stopping = true;
            }
            _ => {}
        }
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        assert!(!p.build(&mut cx).is_empty());
    }
    p.pop = Some((10.0, 10.0, 100.0, 24.0));
    let mut st = State::default();
    let mut cx = Cx::new(0.0, false, &g, &mut st);
    assert!(p.popup(&mut cx).is_some());
}

#[test]
#[ignore]
fn pictures() {
    let mut p = page();
    paint_page(&mut p, "noise_idle", false, 600.0);
    click(&mut p, K_PLAY);
    click(&mut p, idx(K_SLEEP, 2));
    p.fake.sleep_left = Some(27 * 60);
    paint_page(&mut p, "noise_playing", false, 600.0);
    p.pop = Some((310.0, 150.0, 130.0, 28.0));
    paint_page(&mut p, "noise_list", true, 600.0);
}

#[test]
fn a_press_outside_the_list_closes_it_and_the_click_on_the_button_does_not_open_it_again() {
    let mut p = page();
    let r = (300.0, 150.0, 130.0, 28.0);
    press(&mut p, K_KIND, 0.0, r);
    click(&mut p, K_KIND);
    assert!(p.pop.is_some());
    // the frame: the press on the button closes the popup first, then delivers the press and the click
    p.popup_dismiss();
    press(&mut p, K_KIND, 0.0, r);
    click(&mut p, K_KIND);
    assert!(p.pop.is_none(), "the button closes the list, it does not reopen it");
    // the next press + click opens it again
    press(&mut p, K_KIND, 0.0, r);
    click(&mut p, K_KIND);
    assert!(p.pop.is_some());
}

#[test]
fn the_wake_for_the_last_minute_is_one_second_after_one_minute_left() {
    let mut p = page();
    click(&mut p, K_PLAY);
    for (left, wait_ms) in [(60u32, 1000.0), (45, 45_000.0), (61, 1000.0), (120, 60_000.0)] {
        p.fake.sleep_left = Some(left);
        let at = sleep_wake(&p.status(), 0.0).unwrap();
        assert!((at - (wait_ms + 150.0)).abs() < 1.0, "{left} s left: wake at {at}");
    }
}

// ------------------------------------------------------------------ Order 080: Custom, saved sounds

fn pick_custom(p: &mut Noise) {
    press(p, K_KIND, 0.0, (300.0, 150.0, 130.0, 28.0));
    click(p, K_KIND);
    let i = p.entries().iter().position(|e| *e == Entry::Item(Pick::Custom)).unwrap();
    click(p, idx(K_MENU, i));
    assert_eq!(p.prefs.pick, Pick::Custom);
}

fn drag_part(p: &mut Noise, k: Key, frac: f32) {
    let r = (100.0, 200.0, 150.0, 20.0);
    // the thumb centre runs from 8 to w - 8
    let x = 100.0 + 8.0 + frac * (150.0 - 16.0);
    with_cx(|cx| p.event(&Ev::Drag(k, x, 0.0, r), cx));
}

fn type_name(p: &mut Noise, text: &str) {
    for c in text.chars() {
        with_cx(|cx| p.event(&Ev::Char(K_NAME, c), cx));
    }
}

fn key_down(p: &mut Noise, vk: u16) -> bool {
    with_cx(|cx| {
        p.event(&Ev::Key(K_NAME, vk), cx);
        cx.used
    })
}

fn list_names(p: &Noise) -> Vec<String> {
    p.entries()
        .iter()
        .map(|e| match e {
            Entry::Item(x) => x.name(),
            Entry::Sep => "-".into(),
            Entry::Section(s) => format!("[{s}]"),
        })
        .collect()
}

#[test]
fn the_list_has_the_six_then_custom_then_my_sounds() {
    let mut p = page();
    assert_eq!(list_names(&p), ["White", "Pink", "Brown", "Dark brown", "Grey", "Blue", "-", "Custom"]);
    pick_custom(&mut p);
    click(&mut p, K_SAVE);
    type_name(&mut p, "Rain");
    key_down(&mut p, 0x0D);
    assert_eq!(list_names(&p), ["White", "Pink", "Brown", "Dark brown", "Grey", "Blue", "-", "Custom", "[My sounds]", "Rain"]);
}

#[test]
fn custom_has_three_sliders_that_change_the_playing_sound_live() {
    let mut p = page();
    click(&mut p, K_PLAY);
    pick_custom(&mut p);
    assert_eq!(p.status().sound, Some(Sound::Mix(Mix::DEFAULT)), "the mix plays after the pick");
    drag_part(&mut p, K_TONE, 1.0);
    drag_part(&mut p, K_RUMBLE, 0.0);
    drag_part(&mut p, K_WAVES, 0.5);
    assert_eq!(p.prefs.custom, Mix::new(100, 0, 50));
    assert_eq!(p.status().sound, Some(Sound::Mix(Mix::new(100, 0, 50))), "while it plays, each move reaches the player at once");
    drag_part(&mut p, K_TONE, 0.25);
    assert_eq!(p.status().sound, Some(Sound::Mix(Mix::new(25, 0, 50))));
    // the sliders go all the way to 0 and 100, also beyond the ends
    let r = (100.0, 200.0, 150.0, 20.0);
    press(&mut p, K_WAVES, 900.0, r);
    assert_eq!(p.prefs.custom.waves, 100);
    press(&mut p, K_WAVES, -50.0, r);
    assert_eq!(p.prefs.custom.waves, 0);
    // nothing but the three sliders changes the mix
    assert_eq!(p.prefs.volume, 10);
}

#[test]
fn the_sliders_show_only_for_custom_and_the_mix_is_remembered() {
    let sc = Scratch::new("noise-custom-mem");
    let mut store = SettingsStore::open(sc.dir());
    let p = Prefs { pick: Pick::Custom, custom: Mix::new(12, 99, 3), ..Prefs::default() };
    p.save(&mut store);
    assert_eq!(Prefs::load(&store).pick, Pick::Custom);
    assert_eq!(Prefs::load(&store).custom, Mix::new(12, 99, 3));
    // the page: the "Your mix" group is there for Custom only
    let mut page = page();
    let count = |page: &mut Noise| with_cx(|cx| page.build(cx).len());
    let plain = count(&mut page);
    pick_custom(&mut page);
    assert!(count(&mut page) > plain, "Custom adds a group");
    press(&mut page, K_KIND, 0.0, (300.0, 150.0, 130.0, 28.0));
    click(&mut page, K_KIND);
    click(&mut page, idx(K_MENU, 0));
    assert_eq!(count(&mut page), plain);
}

#[test]
fn save_as_my_sound_asks_for_a_name_and_keeps_the_mix_in_the_list() {
    let mut p = page();
    pick_custom(&mut p);
    drag_part(&mut p, K_TONE, 0.8);
    drag_part(&mut p, K_WAVES, 1.0);
    let mix = p.prefs.custom;
    click(&mut p, K_SAVE);
    assert_eq!(p.naming.as_deref(), Some(""), "the name field opens empty");
    type_name(&mut p, "Ocean, deep");
    assert_eq!(p.naming.as_deref(), Some("Ocean, deep"));
    key_down(&mut p, 0x08);
    assert_eq!(p.naming.as_deref(), Some("Ocean, dee"));
    type_name(&mut p, "p");
    let used = key_down(&mut p, 0x0D);
    assert!(!used);
    assert!(p.naming.is_none());
    assert_eq!(p.prefs.mine, vec![prefs::Mine { name: "Ocean, deep".into(), mix }]);
    assert_eq!(p.prefs.pick, Pick::Mine("Ocean, deep".into()), "it is picked (and sounds the same)");
    assert_eq!(p.prefs.sound(), Sound::Mix(mix));
    assert_eq!(p.prefs.pick.name(), "Ocean, deep");
    with_cx(|cx| assert!(!p.build(cx).is_empty()));
}

#[test]
fn naming_can_be_cancelled_and_an_empty_name_gets_the_default() {
    let mut p = page();
    pick_custom(&mut p);
    click(&mut p, K_SAVE);
    type_name(&mut p, "x");
    click(&mut p, K_NCANCEL);
    assert!(p.naming.is_none() && p.prefs.mine.is_empty());
    click(&mut p, K_SAVE);
    type_name(&mut p, "x");
    assert!(key_down(&mut p, 0x1B), "Esc is used by the field: it does not close the menu");
    assert!(p.naming.is_none() && p.prefs.mine.is_empty());
    click(&mut p, K_SAVE);
    click(&mut p, K_NSAVE);
    assert_eq!(p.prefs.mine.len(), 1);
    assert_eq!(p.prefs.mine[0].name, "My sound");
    // picking something else while a name is typed drops the name
    pick_custom(&mut p);
    click(&mut p, K_SAVE);
    assert!(p.naming.is_some());
    press(&mut p, K_KIND, 0.0, (300.0, 150.0, 130.0, 28.0));
    click(&mut p, K_KIND);
    click(&mut p, idx(K_MENU, 0));
    assert!(p.naming.is_none());
}

#[test]
fn names_are_made_unique_and_short() {
    let mut p = Prefs::default();
    assert_eq!(p.save_mine("Rain").as_deref(), Some("Rain"));
    assert_eq!(p.save_mine("rain").as_deref(), Some("rain 2"), "case does not make it another name");
    assert_eq!(p.save_mine("Rain").as_deref(), Some("Rain 3"));
    assert_eq!(p.save_mine("Brown").as_deref(), Some("Brown 2"), "not like a noise");
    assert_eq!(p.save_mine("custom").as_deref(), Some("custom 2"), "not like Custom");
    assert_eq!(p.save_mine("   ").as_deref(), None, "a blank name is nothing");
    let long = p.save_mine(&"a".repeat(60)).unwrap();
    assert_eq!(long.chars().count(), prefs::MAX_NAME);
    let again = p.save_mine(&"a".repeat(60)).unwrap();
    assert!(again.chars().count() <= prefs::MAX_NAME && again.ends_with(" 2") && again != long, "{again}");
    assert_eq!(p.save_mine("tab\there\n").as_deref(), Some("tabhere"));
}

#[test]
fn twelve_sounds_at_most() {
    let mut p = page();
    pick_custom(&mut p);
    for i in 0..prefs::MAX_MINE {
        click(&mut p, K_SAVE);
        type_name(&mut p, &format!("s{i}"));
        click(&mut p, K_NSAVE);
        pick_custom(&mut p);
    }
    assert_eq!(p.prefs.mine.len(), prefs::MAX_MINE);
    click(&mut p, K_SAVE);
    assert!(p.naming.is_none(), "the list is full: no name field");
    assert_eq!(p.prefs.save_mine("one more"), None);
}

#[test]
fn picking_a_saved_sound_plays_its_mix_and_remove_takes_it_out() {
    let mut p = page();
    pick_custom(&mut p);
    drag_part(&mut p, K_TONE, 0.2);
    drag_part(&mut p, K_RUMBLE, 0.9);
    let mine = p.prefs.custom;
    click(&mut p, K_SAVE);
    type_name(&mut p, "Rumbly");
    click(&mut p, K_NSAVE);
    // change Custom, then come back to the saved one
    pick_custom(&mut p);
    drag_part(&mut p, K_TONE, 1.0);
    click(&mut p, K_PLAY);
    assert_eq!(p.status().sound, Some(Sound::Mix(Mix::new(100, mine.rumble, mine.waves))));
    press(&mut p, K_KIND, 0.0, (300.0, 150.0, 130.0, 28.0));
    click(&mut p, K_KIND);
    let i = p.entries().iter().position(|e| *e == Entry::Item(Pick::Mine("Rumbly".into()))).unwrap();
    with_cx(|cx| assert!(p.popup(cx).is_some(), "the list with My sounds builds"));
    click(&mut p, idx(K_MENU, i));
    assert_eq!(p.prefs.pick, Pick::Mine("Rumbly".into()));
    assert_eq!(p.status().sound, Some(Sound::Mix(mine)), "the playing sound follows");
    assert_eq!(p.prefs.custom.tone, 100, "Custom keeps its own sliders");
    // Remove: gone from the list; Custom holds its values so what plays does not change
    click(&mut p, K_REMOVE);
    assert!(p.prefs.mine.is_empty());
    assert_eq!(p.prefs.pick, Pick::Custom);
    assert_eq!(p.prefs.custom, mine);
    assert_eq!(p.status().sound, Some(Sound::Mix(mine)));
    assert!(!p.entries().iter().any(|e| matches!(e, Entry::Section(_))), "no empty My sounds heading");
}

#[test]
fn saved_sounds_and_the_mix_round_trip_through_the_file() {
    let sc = Scratch::new("noise-mine-round");
    let mut store = SettingsStore::open(sc.dir());
    let mut p = Prefs { custom: Mix::new(1, 2, 3), ..Prefs::default() };
    p.save_mine("Rain, soft");
    p.custom = Mix::new(90, 0, 100);
    p.save_mine("Sea");
    p.pick = Pick::Mine("Rain, soft".into());
    p.save(&mut store);
    drop(store);
    let again = SettingsStore::open(sc.dir());
    let q = Prefs::load(&again);
    assert_eq!(q, p);
    assert_eq!(q.sound(), Sound::Mix(Mix::new(1, 2, 3)));
}

#[test]
fn broken_saved_sounds_are_skipped_and_a_missing_pick_falls_back() {
    let sc = Scratch::new("noise-mine-broken");
    let mut store = SettingsStore::open(sc.dir());
    let lines = ["10,20,30,Good", "x,1,2,Bad", "1,2,3", "101,0,0,Too much", "5,5,5,   ", "7,8,9,good", "40,50,60,Second"];
    let lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    let _ = store.set_list(scope_for_test(), "mine", &lines);
    let _ = store.set_str(scope_for_test(), "kind", "mine:Vanished");
    let _ = store.set_str(scope_for_test(), "custom", "1,2");
    let p = Prefs::load(&store);
    let names: Vec<&str> = p.mine.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["Good", "Second"], "bad lines and a repeated name are dropped");
    assert_eq!(p.pick, Pick::Preset(Kind::Brown), "a saved sound that is gone: the default");
    assert_eq!(p.custom, Mix::DEFAULT, "a bad Custom value: the default");
    let _ = store.set_str(scope_for_test(), "kind", "mine:SECOND");
    assert_eq!(Prefs::load(&store).pick, Pick::Mine("Second".into()), "names are matched without case");
}

#[test]
fn a_dragged_slider_is_written_when_the_button_comes_up_or_the_tab_closes() {
    let mut p = page();
    pick_custom(&mut p);
    drag_part(&mut p, K_TONE, 0.9);
    assert!(p.dirty);
    with_cx(|cx| p.event(&Ev::Release(K_TONE), cx));
    assert!(!p.dirty);
    drag_part(&mut p, K_VOL, 0.4);
    assert!(p.dirty, "the volume the same");
    p.close();
    assert!(!p.dirty);
}

#[test]
fn every_custom_state_builds() {
    let mut p = page();
    let g = Gfx::new(1.0);
    pick_custom(&mut p);
    let build = |p: &mut Noise| {
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        assert!(!p.build(&mut cx).is_empty());
    };
    build(&mut p);
    click(&mut p, K_SAVE);
    build(&mut p);
    type_name(&mut p, "Name");
    click(&mut p, K_NSAVE);
    build(&mut p);
    assert!(p.describe().contains("kind=mine:Name"), "{}", p.describe());
}

#[test]
#[ignore]
fn pictures_custom() {
    let mut p = page();
    pick_custom(&mut p);
    drag_part(&mut p, K_TONE, 0.35);
    drag_part(&mut p, K_RUMBLE, 0.3);
    drag_part(&mut p, K_WAVES, 0.6);
    paint_page(&mut p, "noise_custom", false, 600.0);
    click(&mut p, K_SAVE);
    type_name(&mut p, "Slow sea");
    paint_page(&mut p, "noise_custom_naming", false, 600.0);
    click(&mut p, K_NSAVE);
    paint_page(&mut p, "noise_mine", false, 600.0);
    click(&mut p, K_PLAY);
    p.fake.sleep_left = Some(27 * 60);
    pick_custom(&mut p);
    click(&mut p, K_SAVE);
    type_name(&mut p, "A rather long name here");
    click(&mut p, K_NSAVE);
    p.pop = Some((310.0, 150.0, 130.0, 28.0));
    paint_page(&mut p, "noise_list_mine", true, 600.0);
}

// ------------------------------------------------------------------ Order 092: the listening tracker

/// Local seconds of 10 Oct 2026 (a Saturday) at `h:m`.
fn sat(h: i64, m: i64) -> i64 {
    listen::days_from_civil(2026, 10, 10) * 86_400 + h * 3600 + m * 60
}

#[test]
fn nothing_is_counted_until_play_and_stop_have_both_happened() {
    let mut p = page();
    p.clock = sat(20, 0);
    assert_eq!(p.listened_now(), Totals::default(), "a fresh page has listened to nothing");
    p.open(&Env { test: true, ..Env::default() }, 0.0);
    assert_eq!(p.listened, Totals::default());
    click(&mut p, K_PLAY);
    // 15 min later the tab is opened again: the running stretch shows up to now, but is not kept
    p.clock = sat(20, 15);
    p.wrote = 900.0;
    p.close();
    p.open(&Env { test: true, ..Env::default() }, 0.0);
    assert_eq!((p.listened.today, p.listened.all), (900, 900));
    assert!(p.tracker.is_open(), "the stretch is still open: only Stop ends it");
}

#[test]
fn stop_adds_the_stretch_to_today_this_week_this_month_this_year_and_all_time() {
    let mut p = page();
    p.clock = sat(20, 0);
    click(&mut p, K_PLAY);
    p.clock = sat(20, 45);
    p.wrote = 2700.0;
    click(&mut p, K_PLAY); // the button says Stop now
    assert!(!p.status().playing);
    let want = 45 * 60;
    assert_eq!(p.listened, Totals { today: want, week: want, month: want, year: want, all: want });
    // a second stretch, the next evening: today restarts, the others add up
    p.clock = sat(20, 0) + 86_400;
    click(&mut p, K_PLAY);
    p.clock += 600;
    p.wrote += 600.0;
    click(&mut p, K_PLAY);
    assert_eq!(p.listened, Totals { today: 600, week: want + 600, month: want + 600, year: want + 600, all: want + 600 });
}

#[test]
fn a_sleeping_pc_or_a_missing_output_does_not_count_as_listening() {
    let mut p = page();
    p.clock = sat(23, 0);
    click(&mut p, K_PLAY);
    // 9 hours of clock (the PC slept), the player wrote 5 minutes
    p.clock += 9 * 3600;
    p.wrote = 300.0;
    click(&mut p, K_PLAY);
    assert_eq!(p.listened.all, 300);
}

#[test]
fn the_time_listened_row_has_the_five_names_in_order_and_builds() {
    let mut p = page();
    with_cx(|cx| assert!(!p.build(cx).is_empty()));
    let t = Totals { today: 90, week: 3700, month: 3700, year: 7200 + 120, all: 133 * 3600 };
    let words: Vec<String> = [t.today, t.week, t.month, t.year, t.all].into_iter().map(listen::format).collect();
    assert_eq!(words, ["1m", "1h 01m", "1h 01m", "2h 02m", "133h"]);
    let line = with_cx(|cx| texts_of(listened_line(cx, t)));
    assert_eq!(line, ["1m today", "·", "1h 01m this week", "·", "1h 01m this month", "·", "2h 02m this year", "·", "133h in all"]);
}

/// Picture of the "Time listened" row with some time in it (run: cargo test -p bu-app noise::tests::picture_listened -- --ignored).
#[test]
#[ignore]
fn picture_listened() {
    let mut p = page();
    p.listened = Totals { today: 95 * 60, week: 7 * 3600 + 20 * 60, month: 26 * 3600 + 5 * 60, year: 211 * 3600, all: 340 * 3600 };
    paint_page(&mut p, "noise_listened", false, 600.0);
}

// ------------------------------------------------------------------ Order 097: Soft tide + the one grey line

fn texts_of(el: El) -> Vec<String> {
    let g = Gfx::new(1.0);
    let laid = Laid::new(&g, El::block().w(560.0).child(el), 560.0, None);
    laid.nodes.iter().filter_map(|n| if let crate::ui::el::Content::Text(t) = &n.el.content { Some(t.s.to_string()) } else { None }).collect()
}

/// the owner, Oct 10 (pack-noise-v1 option 1): no animation, no frame and no timer unless noise plays AND the tab is on screen.
#[test]
fn the_tide_asks_for_nothing_while_nothing_plays() {
    let mut p = page();
    assert!(!p.tick(0.0), "nothing plays: no frame");
    assert_eq!(p.wake_at(0.0), None, "nothing plays: no timer");
    assert_eq!(p.level, 0.0);
    let n = with_cx(|cx| p.build(cx).len());
    click(&mut p, K_PLAY);
    assert!(p.tick(10.0), "it starts moving");
    assert!(with_cx(|cx| p.build(cx).len()) == n + 1, "the tide is in the page now");
}

#[test]
fn the_tide_moves_about_twenty_times_a_second_and_settles_when_stopped() {
    let mut p = page();
    click(&mut p, K_PLAY);
    let mut frames = 0;
    let mut t = 0.0;
    while t < 1000.0 {
        if p.tick(t) {
            frames += 1;
        }
        t += 4.0; // the loop may ask far more often than it draws
    }
    assert!((18..=21).contains(&frames), "{frames} frames in one second");
    assert!(p.level > 0.5 && p.level <= 1.0, "faded in: {}", p.level);
    assert_eq!(p.wake_at(1000.0), p.anim_at.map(|a| a + TIDE_MS), "asks again for its next step");
    // Stop: it fades and ends; then no frame and no timer again
    click(&mut p, K_PLAY);
    let mut last = 0.0;
    for i in 0..400 {
        t = 1000.0 + i as f64 * 50.0;
        p.tick(t);
        last = t;
        if p.level == 0.0 {
            break;
        }
    }
    assert_eq!(p.level, 0.0, "it faded away");
    assert!(!p.tick(last + 100.0));
    assert_eq!(p.wake_at(last + 100.0), None);
    assert!(with_cx(|cx| p.build(cx).len()) == with_cx(|cx| page().build(cx).len()), "the tide left the page");
}

/// The box of big numbers is gone: ONE small line with the five totals.
#[test]
fn the_time_listened_is_one_small_line_under_the_card() {
    let mut p = page();
    p.listened = Totals { today: 95 * 60, week: 7 * 3600 + 20 * 60, month: 26 * 3600 + 5 * 60, year: 211 * 3600, all: 340 * 3600 };
    let texts = with_cx(|cx| texts_of(El::col().children(p.build(cx))));
    assert!(!texts.iter().any(|t| t == "Time listened"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "1h 35m today") && texts.iter().any(|t| t == "340h in all"), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "Today" || t == "All time"), "the old boxes are gone: {texts:?}");
}

/// Picture: the Noise tab in the app's 600 x 520 window (page from y 56) while playing, the tide faded in
/// (run: cargo test -p bu-app noise::tests::picture_tide -- --ignored). `BU_PIC_OUT` = the folder.
#[test]
#[ignore]
fn picture_tide() {
    let Ok(dir) = std::env::var("BU_PIC_OUT") else { return };
    let mut p = page();
    p.listened = Totals { today: 95 * 60, week: 7 * 3600 + 20 * 60, month: 26 * 3600 + 5 * 60, year: 211 * 3600, all: 340 * 3600 };
    click(&mut p, K_PLAY);
    for i in 0..200 {
        p.tick(i as f64 * 50.0);
    }
    let g = Gfx::new(1.0);
    let icons = Icons::new();
    let mut st = State::default();
    let mut cx = Cx::new(10_000.0, false, &g, &mut st);
    cx.page = "nse";
    let kids = p.build(&mut cx);
    let root = El::block().w(WIN_W).pad(2.0, 26.0, 18.0, 26.0).children(kids);
    let laid = Laid::new(&g, root, WIN_W, None);
    let Some(mut s) = crate::gfx::new_surface(600, 520) else { return };
    g.begin(s.canvas());
    g.fill_rect(0.0, 0.0, 600.0, 520.0, crate::gfx::Rgba::rgb(29, 32, 48));
    g.end();
    let base = s.image_snapshot();
    g.begin(s.canvas());
    laid.paint(&g, &icons, 0.0, crate::ui::PAGE_TOP, Some(&base));
    g.end();
    let px = crate::png::from_surface(&mut s);
    let _ = std::fs::create_dir_all(&dir);
    // SAFETY: COM for the WIC encoder on this test thread.
    let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
    crate::png::save_png(&px, &format!("{dir}/noise_playing.png")).expect("save");
}

/// Review (Order 097): behind a full-screen game, or with reduced motion, the tide neither draws nor asks for a timer.
#[test]
fn the_tide_stands_still_behind_a_game_and_with_reduced_motion() {

    let mut p = page();
    click(&mut p, K_PLAY);
    assert!(p.tick(0.0) && p.level > 0.0);
    T_COVERED.with(|c| c.set(true));
    assert!(p.tick(100.0), "the tide goes off the page once");
    assert_eq!((p.level, p.wake_at(100.0)), (0.0, None));
    assert!(!p.tick(200.0));
    T_COVERED.with(|c| c.set(false));
    assert!(p.tick(300.0), "back in front: it starts again");
    p.rm = true;
    assert!(p.tick(400.0));
    assert_eq!((p.level, p.wake_at(400.0)), (0.0, None));
}
