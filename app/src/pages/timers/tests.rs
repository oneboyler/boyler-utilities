//! Timers page tests - against the fake model only (a fake clock that moves only when a test moves it, a silent sound
//! that only counts, the drawing's "now"). No window, no Windows timer, nothing on the screen, nothing played.

use std::time::Duration;

use super::model::{self, Kind, Mode, Spot};
use super::*;
use crate::gfx::Gfx;
use crate::ui::cx::State;

struct T {
    page: Timers,
    g: Gfx,
    st: State,
    now: f64,
}

impl T {
    fn new(frozen: bool) -> T {
        model::drop_model();
        overlay::reset_for_tests();
        let mut page = Timers::default();
        page.open(&Env { test: true, real_read: false, frozen, rm: false, ..Env::default() }, 0.0);
        T { page, g: Gfx::new(1.0), st: State::default(), now: 0.0 }
    }
    fn ev(&mut self, e: Ev) {
        let mut cx = Cx::new(self.now, true, &self.g, &mut self.st);
        self.page.event(&e, &mut cx);
    }
    fn click(&mut self, k: Key) {
        self.ev(Ev::Press(k, 0.0, 0.0, (0.0, 0.0, 10.0, 10.0)));
        self.ev(Ev::Click(k));
    }
    fn build(&mut self) -> Vec<El> {
        let mut cx = Cx::new(self.now, true, &self.g, &mut self.st);
        self.page.build(&mut cx)
    }
    fn advance(&mut self, ms: u64) {
        self.now += ms as f64;
        model::with(true, false, |m| m.fake_clock.as_ref().unwrap().advance_ms(ms));
    }
    fn m<R>(&self, f: impl FnOnce(&mut model::Model) -> R) -> R {
        model::with(true, false, f)
    }
}

fn sel_text(t: &T) -> String {
    t.m(|m| m.selected().map(|x| x.big_text()).unwrap_or_default())
}

#[test]
fn frozen_sample_is_the_drawings() {
    let t = T::new(true);
    t.m(|m| {
        let names: Vec<&str> = m.timers.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["Stopwatch", "Pizza", "Ultimate"]);
        assert_eq!(m.get(2).unwrap().cd.text(), "8:41");
        assert!(m.get(2).unwrap().running());
        assert_eq!(m.get(2).unwrap().kind_text(), "Countdown \u{b7} 12:00");
        let u = m.get(3).unwrap();
        assert!(u.screen && u.key.as_deref() == Some("F8") && u.colour == model::TBCOL[1]);
        assert_eq!(m.places.iter().map(|p| p.city.as_str()).collect::<Vec<_>>(), ["New York", "Tokyo"]);
        let w = m.world();
        assert_eq!((w.home_name.as_str(), w.home_time.as_str(), w.home_line.as_str()), ("Zagreb", "21:37", "Thursday 8 Oct \u{b7} your time"));
    });
}

#[test]
fn stopwatch_start_lap_stop_reset() {
    let mut t = T::new(false);
    assert_eq!(sel_text(&t), "0:00.00");
    t.click(K_GO);
    t.advance(1_230);
    assert_eq!(sel_text(&t), "0:01.23");
    t.click(K_LAP);
    t.advance(770);
    t.click(K_LAP);
    let laps = t.m(|m| m.laps(m.sel));
    assert_eq!(laps.len(), 2);
    assert_eq!(laps[0].label, "Lap 2");
    assert!(laps[0].best && !laps[1].best);
    t.click(K_GO);
    t.advance(5_000);
    assert_eq!(sel_text(&t), "0:02.00");
    t.m(|m| assert_eq!(m.selected().unwrap().sw.button(), bu_timers::stopwatch::SwButton::Resume));
    t.click(K_RST);
    assert_eq!(sel_text(&t), "0:00.00");
    assert!(t.m(|m| m.laps(m.sel).is_empty()));
}

#[test]
fn countdown_typed_time_enter_starts_and_esc_cancels() {
    let mut t = T::new(false);
    t.click(idx(K_SEG, 1));
    assert_eq!(t.m(|m| (m.mode, m.selected().unwrap().kind)), (Mode::Cd, Kind::Cd));
    assert_eq!(sel_text(&t), "5:00");
    // focus selects the whole time: typing replaces it
    t.ev(Ev::Press(K_TIME, 0.0, 0.0, (0.0, 0.0, 1.0, 1.0)));
    for c in "1:30".chars() {
        t.ev(Ev::Char(K_TIME, c));
    }
    t.ev(Ev::Key(K_TIME, 0x0D));
    t.m(|m| {
        let x = m.selected().unwrap();
        assert_eq!(x.cd.set_time(), Duration::from_secs(90));
        assert!(x.running());
    });
    // while it runs the time can't be typed
    t.ev(Ev::Press(K_TIME, 0.0, 0.0, (0.0, 0.0, 1.0, 1.0)));
    assert!(t.page.time_edit.is_none());
    t.click(K_GO); // pause
    t.ev(Ev::Press(K_TIME, 0.0, 0.0, (0.0, 0.0, 1.0, 1.0)));
    t.ev(Ev::Char(K_TIME, '9'));
    t.ev(Ev::Key(K_TIME, 0x1B));
    t.ev(Ev::Blur(K_TIME));
    assert_eq!(t.m(|m| m.selected().unwrap().cd.set_time()), Duration::from_secs(90));
    // not a time: no change
    t.ev(Ev::Press(K_TIME, 0.0, 0.0, (0.0, 0.0, 1.0, 1.0)));
    for c in "abc".chars() {
        t.ev(Ev::Char(K_TIME, c));
    }
    t.ev(Ev::Blur(K_TIME));
    assert_eq!(t.m(|m| m.selected().unwrap().cd.set_time()), Duration::from_secs(90));
}

#[test]
fn countdown_end_chimes_and_toasts_once() {
    let mut t = T::new(false);
    t.click(idx(K_SEG, 1));
    let id = t.m(|m| m.sel);
    t.m(|m| m.type_time(id, "3s"));
    // the sound is OFF by default: the end shows the toast but plays nothing
    t.click(K_GO);
    t.advance(2_000);
    assert!(!t.page.tick(t.now) || t.page.toast.is_none());
    t.advance(1_000);
    assert!(t.page.tick(t.now));
    assert_eq!(t.page.toast.as_ref().map(|x| x.0.as_str()), Some("Countdown \u{b7} done"));
    assert_eq!(t.page.end_at.map(|e| e.0), Some(id));
    assert_eq!(t.m(|m| m.chimes), 0, "off by default");
    // the ▶ preview plays only with the sound on
    t.click(K_PV);
    assert_eq!(t.m(|m| m.chimes), 0);
    t.click(K_SND);
    t.click(K_PV);
    assert_eq!(t.m(|m| m.chimes), 1);
    // switched on: the next end chimes once
    t.m(|m| assert_eq!(m.selected().unwrap().cd.button(), bu_timers::countdown::CdButton::Again));
    t.click(K_GO);
    t.advance(3_000);
    assert!(t.page.tick(t.now));
    assert_eq!(t.m(|m| m.chimes), 2);
    t.advance(5_000);
    t.page.tick(t.now);
    assert_eq!(t.m(|m| m.chimes), 2, "once");
}

#[test]
fn on_screen_switch_pills_and_never_a_window_in_tests() {
    let mut t = T::new(true);
    // Timers open = preview: Ultimate (On screen) shows although it does not run
    assert_eq!(t.m(|m| m.pills(true).len()), 1);
    assert_eq!(t.m(|m| m.pills(false).len()), 0);
    t.click(K_SCR); // the Stopwatch on screen
    assert_eq!(t.page.toast.as_ref().map(|x| x.0.as_str()), Some("Stopwatch shows on your screen while it runs"));
    assert_eq!(t.m(|m| m.pills(true).len()), 2);
    t.click(K_GO);
    t.advance(1_000);
    let p = t.m(|m| m.pills(false));
    assert_eq!(p.len(), 1);
    assert_eq!((p[0].name.as_str(), p[0].time.as_str(), p[0].line), ("Stopwatch", "0:01", None));
    // a countdown's last 3 seconds are red
    t.m(|m| {
        m.type_time(3, "4s");
        m.toggle(3);
    });
    t.advance(1_500);
    let p = t.m(|m| m.pills(false));
    assert!(p.iter().any(|x| x.name == "Ultimate" && x.low && x.time == "0:03"));
    t.click(K_MOVE);
    assert!(t.m(|m| m.moving));
    t.click(K_SCR);
    assert_eq!(t.page.toast.as_ref().map(|x| x.0.as_str()), Some("Stopwatch \u{b7} off your screen"));
    // places on screen: always
    t.click(sub(idx(K_PLACE, 0), "sb"));
    assert!(t.m(|m| m.pills(false).iter().any(|x| x.name == "New York" && x.time == "15:37")));
    overlay::sync();
    assert!(!overlay::window_exists());
    assert!(!overlay::timer_armed());
    assert!(t.page.describe().contains("window=false timer=false"));
}

#[test]
fn new_timer_names_remove_and_pick() {
    let mut t = T::new(true);
    t.click(K_ADD);
    t.m(|m| assert_eq!(m.selected().unwrap().name, "Stopwatch 2"));
    assert_eq!(t.page.name_edit.as_ref().map(|e| e.1), Some(true));
    t.click(idx(K_SEG, 1));
    t.click(K_ADD);
    t.m(|m| assert_eq!(m.selected().unwrap().name, "Countdown 3"));
    let id = t.m(|m| m.sel);
    t.click(sub(idx(K_ROW, id as usize), "del"));
    t.m(|m| {
        assert!(m.get(id).is_none());
        assert_eq!(m.selected().unwrap().name, "Pizza");
    });
    // a row click picks it (and the switch follows its kind)
    t.click(idx(K_ROW, 1));
    assert_eq!(t.m(|m| (m.sel, m.mode)), (1, Mode::Sw));
    // a row's play / pause
    t.click(sub(idx(K_ROW, 3), "pb"));
    assert!(t.m(|m| m.get(3).unwrap().running()));
    // removing the last of everything leaves a fresh one
    let ids: Vec<u32> = t.m(|m| m.timers.iter().map(|x| x.id).collect());
    for i in ids {
        t.click(sub(idx(K_ROW, i as usize), "del"));
    }
    t.m(|m| {
        assert_eq!(m.timers.len(), 1);
        assert_eq!(m.selected().unwrap().id, m.timers[0].id);
    });
}

#[test]
fn rename_by_typing() {
    let mut t = T::new(false);
    t.ev(Ev::Press(K_NAME, 0.0, 0.0, (0.0, 0.0, 1.0, 1.0)));
    for c in "Tea".chars() {
        t.ev(Ev::Char(K_NAME, c));
    }
    t.ev(Ev::Key(K_NAME, 0x0D));
    assert_eq!(t.m(|m| m.selected().unwrap().name.clone()), "Tea");
    // Esc keeps the old name; at most 22 characters; blank keeps the old one
    t.ev(Ev::Press(K_NAME, 0.0, 0.0, (0.0, 0.0, 1.0, 1.0)));
    t.ev(Ev::Char(K_NAME, 'x'));
    t.ev(Ev::Key(K_NAME, 0x1B));
    t.ev(Ev::Blur(K_NAME));
    assert_eq!(t.m(|m| m.selected().unwrap().name.clone()), "Tea");
    t.ev(Ev::Press(K_NAME, 0.0, 0.0, (0.0, 0.0, 1.0, 1.0)));
    for _ in 0..30 {
        t.ev(Ev::Char(K_NAME, 'a'));
    }
    t.ev(Ev::Blur(K_NAME));
    assert_eq!(t.m(|m| m.selected().unwrap().name.chars().count()), 22);
    t.ev(Ev::Press(K_NAME, 0.0, 0.0, (0.0, 0.0, 1.0, 1.0)));
    t.ev(Ev::Key(K_NAME, 0x08));
    t.ev(Ev::Blur(K_NAME));
    assert_eq!(t.m(|m| m.selected().unwrap().name.chars().count()), 22);
}

#[test]
fn places_menu_add_remove() {
    let mut t = T::new(true);
    t.click(idx(K_SEG, 2));
    assert_eq!(t.m(|m| m.mode), Mode::Clk);
    t.ev(Ev::Press(K_ADD, 500.0, 300.0, (466.0, 236.0, 96.0, 22.0)));
    assert!(t.page.menu.is_some());
    let el = {
        let mut cx = Cx::new(0.0, true, &t.g, &mut t.st);
        t.page.popup(&mut cx)
    };
    assert!(el.is_some());
    // the list = the places not added yet, in the drawing's order: Los Angeles first
    t.ev(Ev::Click(idx(K_MENU, 0)));
    assert!(t.page.menu.is_none());
    assert_eq!(t.m(|m| m.places.iter().map(|p| p.city.clone()).collect::<Vec<_>>()), ["New York", "Tokyo", "Los Angeles"]);
    t.click(sub(idx(K_PLACE, 0), "del"));
    assert_eq!(t.m(|m| m.places.len()), 2);
    // a click beside the open list closes it
    t.ev(Ev::Press(K_ADD, 500.0, 300.0, (466.0, 236.0, 96.0, 22.0)));
    t.page.popup_dismiss();
    assert!(t.page.menu.is_none());
    // no places: the empty line
    t.click(sub(idx(K_PLACE, 0), "del"));
    t.click(sub(idx(K_PLACE, 0), "del"));
    let kids = t.build();
    assert_eq!(kids.len(), 3);
}


#[test]
fn nothing_runs_on_open_and_close_keeps_timers() {
    let mut t = T::new(false);
    assert!(!t.m(|m| m.any_running()));
    assert!(!t.page.tick(0.0));
    t.click(K_GO);
    assert!(t.page.tick(0.0));
    t.page.name_edit = Some(("Run".into(), false));
    t.page.close();
    // the page let go of its own state; the timer keeps running in the model
    assert!(t.page.name_edit.is_none() && t.page.toast.is_none());
    t.advance(2_000);
    t.m(|m| {
        assert_eq!(m.selected().unwrap().name, "Run");
        assert!(m.any_running());
        assert_eq!(m.selected().unwrap().big_text(), "0:02.00");
    });
    // the world clock repaints only when the minute changes
    let mut t = T::new(false);
    t.click(idx(K_SEG, 2));
    t.build();
    assert!(!t.page.tick(0.0));
    model::with(true, false, |m| {
        let z = bu_timers::zones::FakeZones { now: Duration::from_secs(bu_timers::zones::DRAWING_NOW_UTC + 60), ..Default::default() };
        m.set_zones(Box::new(z));
    });
    assert!(t.page.tick(0.0));
}

#[test]
fn mode_switch_makes_a_timer_of_that_kind() {
    let mut t = T::new(false);
    t.click(idx(K_SEG, 1));
    t.m(|m| {
        assert_eq!(m.timers.len(), 2);
        assert_eq!(m.selected().unwrap().name, "Countdown");
    });
    t.click(idx(K_SEG, 0));
    assert_eq!(t.m(|m| m.selected().unwrap().name.clone()), "Stopwatch");
    assert!(t.page.hero_at.is_some());
}

#[test]
fn overlay_place_and_snap() {
    let work = (0.0, 0.0, 1920.0, 1032.0);
    assert_eq!(overlay::place(Spot::default(), work, (196.0, 32.0)), (862.0, 24.0));
    let s = Spot { h: 'R', dx: 24.0, v: 'B', dy: 24.0 };
    assert_eq!(overlay::place(s, work, (196.0, 70.0)), (1700.0, 938.0));
    // dropped 8 px from the right edge spot: snaps to it
    let sp = overlay::snap_spot(1920.0 - 24.0 - 196.0 + 8.0, 300.0, (196.0, 32.0), (1920.0, 1032.0));
    assert_eq!((sp.h, sp.dx, sp.v), ('R', 24.0, 'T'));
    let sp = overlay::snap_spot(862.0 + 5.0, 20.0, (196.0, 32.0), (1920.0, 1032.0));
    assert_eq!((sp.h, sp.dx, sp.v, sp.dy), ('C', 0.0, 'T', 24.0));
}

/// The pills off-screen over the drawing's desktop picture (the proof): BU_TMR_SHOT_DESK=<desk.png> BU_TMR_SHOT_OUT=<dir>
/// `cargo test -p bu-app timers::tests::overlay_picture -- --ignored`
#[test]
#[ignore]
fn overlay_picture() {
    let (Ok(desk), Ok(out)) = (std::env::var("BU_TMR_SHOT_DESK"), std::env::var("BU_TMR_SHOT_OUT")) else { return };
    let _t = T::new(true);
    // SAFETY: COM for WIC on this test thread
    let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
    let d = crate::png::load_png(&desk).expect("desk");
    let pills = model::with(true, true, |m| m.pills(true));
    let p = overlay::render(&pills, false, Spot::default(), &d, 1.0).expect("render");
    crate::png::save_png(&p, &format!("{out}\\pills.png")).expect("save");
    // a 3440 x 1440 screen: BU_TMR_SHOT_DESK2 = the drawing's desktop at that size
    if let Ok(big) = std::env::var("BU_TMR_SHOT_DESK2") {
        let d2 = crate::png::load_png(&big).expect("desk2");
        let p = overlay::render(&pills, false, Spot::default(), &d2, 1.0).expect("render");
        crate::png::save_png(&p, &format!("{out}/pills_3440.png")).expect("save");
    }
    let mv = overlay::render(&pills, true, Spot::default(), &d, 1.0).expect("render");
    crate::png::save_png(&mv, &format!("{out}\\pills_move.png")).expect("save");
    // the 3440 x 1440 screen (desk scaled by the test) and a running, low countdown
    model::with(true, true, |m| {
        m.type_time(3, "4s");
        m.toggle(3);
        m.fake_clock.as_ref().unwrap().advance_ms(1_500);
    });
    let pills = model::with(true, true, |m| m.pills(true));
    let p = overlay::render(&pills, false, Spot { h: 'R', dx: 24.0, v: 'B', dy: 24.0 }, &d, 1.0).expect("render");
    crate::png::save_png(&p, &format!("{out}\\pills_low_br.png")).expect("save");
}

/// A timer's key is the keys manager's: the field listens there, a key used by another feature is refused, the key
/// starts / pauses its timer from anywhere (menu closed too), and a deleted timer frees its key.
#[test]
fn a_timer_key_is_the_keys_managers_and_works_anywhere() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    let mut t = T::new(false);
    let _ = t.build();
    let sel = t.m(|m| m.sel);
    let aid = key_action(sel);
    assert!(crate::services::with(|s| s.has_action(&aid)).unwrap(), "shown = its action exists");
    t.click(K_KEY);
    assert_eq!(crate::services::with(|s| s.listening.as_ref().map(|l| l.action.clone())).unwrap(), Some(aid.clone()));
    // F9 down + up: bound
    assert!(crate::services::key_message(true, 0x78, 0));
    let _ = crate::services::key_message(false, 0x78, 0); // (bound on the way down: the release is no longer the field's)
    assert_eq!(crate::services::with(|s| s.field(&aid).0).unwrap().as_deref(), Some("F9"));
    // the key (as the keys manager fires it): the timer runs, again: it pauses
    let running = |t: &T| t.m(|m| m.get(sel).map(|x| x.running()).unwrap_or(false));
    assert!(!running(&t));
    crate::services::with(|s| s.fire_for_test(&aid, true));
    assert!(running(&t), "the key started it");
    crate::services::with(|s| s.fire_for_test(&aid, true));
    assert!(!running(&t), "and paused it");
    // the × clears it
    t.click(sub(K_KEY, "clr"));
    assert_eq!(crate::services::with(|s| s.field(&aid).0).unwrap(), None);
    // a deleted timer: its action goes
    let k = idx(K_ROW, 0);
    let id0 = t.m(|m| m.timers[0].id);
    let _ = k;
    ensure_action(id0, "x");
    drop_action(id0);
    assert!(!crate::services::with(|s| s.has_action(&key_action(id0))).unwrap());
    crate::services::shutdown();
}
