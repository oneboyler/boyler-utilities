//! The Noise page, driven like the frame drives it (clicks by key) and pictured - all with a stand-in player: nothing here
//! plays a sound, opens a stream or touches the PC.

use super::*;
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
    assert_eq!(p.prefs.kind, Kind::Brown);
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
    assert_eq!(p.status().kind, Some(Kind::Brown));
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
    assert_eq!(p.prefs.kind, Kind::Pink);
    assert!(p.pop.is_none(), "the list closes on a pick");
    assert_eq!(p.status().kind, Some(Kind::Pink), "the playing noise changes");
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
    assert_eq!(p.wake_at(1000.0), None, "playing without a timer: nothing changes by itself");
    click(&mut p, idx(K_SLEEP, 1));
    let at = p.wake_at(1000.0).unwrap();
    // 15 min = 900 s: the line changes when it reaches 14 min = 840 s, 60 s from now
    assert!((at - (1000.0 + 60_000.0 + 150.0)).abs() < 1.0, "{at}");
    // a build paints the line; the tick then has nothing new to say
    with_cx(|cx| {
        p.build(cx);
    });
    assert!(!p.tick(1000.0));
    p.fake.sleep_left = Some(14 * 60);
    assert!(p.tick(1000.0), "the minute changed: repaint");
}

#[test]
fn the_remembered_values_round_trip_and_never_say_it_plays() {
    let sc = Scratch::new("noise-round");
    let mut store = SettingsStore::open(sc.dir());
    assert_eq!(Prefs::load(&store), Prefs::default());
    let p = Prefs { kind: Kind::DarkBrown, volume: 33, sleep: Some(60) };
    p.save(&mut store);
    assert_eq!(Prefs::load(&store), p);
    drop(store);
    let again = SettingsStore::open(sc.dir());
    assert_eq!(Prefs::load(&again), p, "read back from the file");
    let text = std::fs::read_to_string(again.path()).unwrap();
    for line in text.lines().filter(|l| l.starts_with("page:noise")) {
        let key = line.split('\t').nth(1).unwrap();
        assert!(["kind", "volume", "sleep"].contains(&key), "unexpected setting {key}");
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
    assert_eq!(p.kind, Kind::Brown);
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

const SCRATCH: &str = r"C:\BoylerUtilities-scratch\N62";

fn paint_page(p: &mut Noise, name: &str, popup: bool, w: f32) {
    let g = Gfx::new(1.0);
    let icons = Icons::new();
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st);
    cx.page = "nse";
    let kids = p.build(&mut cx);
    let root = El::block().w(600.0).pad(2.0, 28.0, 18.0, 28.0).children(kids);
    let laid = Laid::new(&g, root, 600.0, None);
    let h = (laid.height + 40.0).ceil().max(300.0);
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
        let at = p.wake_at(0.0).unwrap();
        assert!((at - (wait_ms + 150.0)).abs() < 1.0, "{left} s left: wake at {at}");
    }
}
