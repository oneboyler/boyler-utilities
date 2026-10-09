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
        assert_eq!(m.places.iter().map(|p| (p.place.city, p.place.land)).collect::<Vec<_>>(), [("New York", "USA"), ("Tokyo", "Japan")]);
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

/// Order 049: the on-screen window sleeps until a pill looks different - a time's next whole second, a countdown line's next
/// quarter pixel - and not at all when nothing on screen runs.
#[test]
fn pills_wake_only_when_they_change() {
    let mut t = T::new(true);
    t.m(|m| {
        m.type_time(3, "4s");
        m.toggle(3);
    });
    // the line: 4 s over 176 px x 4 quarter steps; the text: the left time's next whole second (4.000 s left: in 1 s)
    assert_eq!(t.m(|m| m.next_change(false, 176.0)), Some(Duration::from_secs(4).div_f64(704.0)));
    assert_eq!(t.m(|m| m.next_change(false, 0.0)), Some(Duration::from_secs(1)));
    t.advance(300);
    assert_eq!(t.m(|m| m.next_change(false, 0.0)), Some(Duration::from_millis(700)));
    // paused: nothing moves by itself
    t.m(|m| m.toggle(3));
    assert_eq!(t.m(|m| m.next_change(false, 176.0)), None);
    // running again until zero (before the end alarm finished it): nothing moves either, no frames at the line rate
    t.m(|m| m.toggle(3));
    t.advance(3_700);
    assert_eq!(t.m(|m| m.next_change(false, 176.0)), None);
    // a place on screen: its next minute
    t.click(sub(idx(K_PLACE, 0), "sb"));
    let d = t.m(|m| m.next_change(false, 176.0)).unwrap();
    assert!(d > Duration::ZERO && d <= Duration::from_secs(60), "{d:?}");
    // never an ask or a window in a test
    overlay::sync();
    assert!(!overlay::window_exists() && !overlay::timer_armed());
}

/// Order 049: a 5-minute countdown on screen for a minute, driven the way the window is (wake at `next_change`, paint only
/// a different picture): how many wake-ups and paints - before it was 600 full repaints a minute (every 100 ms).
#[test]
fn a_running_countdown_repaints_only_when_it_changes() {
    let mut t = T::new(true);
    t.m(|m| {
        m.type_time(3, "5m");
        m.toggle(3);
    });
    for scale in [1.0f32, 1.5] {
        let (mut wakes, mut paints, mut ms) = (0u32, 0u32, 0u64);
        let mut last = t.m(|m| m.pills(false));
        while ms < 60_000 {
            let d = t.m(|m| m.next_change(false, overlay::line_px(scale))).expect("it runs");
            let step = (d.as_secs_f64() * 1000.0).ceil().max(1.0) as u64;
            t.advance(step);
            ms += step;
            wakes += 1;
            let now = t.m(|m| m.pills(false));
            if !overlay::same_picture(&last, &now, scale) {
                paints += 1;
            }
            last = now;
        }
        println!("5:00 countdown on screen, scale {scale}: {wakes} wake-ups, {paints} paints in 60 s (before: 600 + 600)");
        assert!(paints <= wakes && wakes < 400 && paints >= 60, "{wakes} {paints}");
    }
}

#[test]
fn new_timer_names_remove_and_pick() {
    let mut t = T::new(true);
    // the frozen sample: the own Stopwatch (id 1) is not in "Your timers"; Pizza + Ultimate (ids 2, 3) are
    assert_eq!(t.m(|m| m.listed().map(|x| x.name.clone()).collect::<Vec<_>>()), ["Pizza", "Ultimate"]);
    t.click(K_ADD);
    t.m(|m| assert_eq!(m.selected().unwrap().name, "Stopwatch 2"));
    assert_eq!(t.page.name_edit.as_ref().map(|e| e.1), Some(true));
    t.click(idx(K_SEG, 1));
    // the Countdown tab shows ITS OWN countdown, made now; it is not listed either
    t.m(|m| {
        let s = m.selected().unwrap();
        assert_eq!((s.name.as_str(), s.own, s.kind), ("Countdown", true, Kind::Cd));
        assert_eq!(m.listed().count(), 3);
    });
    t.click(K_ADD);
    t.m(|m| assert_eq!(m.selected().unwrap().name, "Countdown 4"));
    let id = t.m(|m| m.sel);
    t.click(sub(idx(K_ROW, id as usize), "del"));
    // the picked added countdown went: the own countdown is shown again, nothing new was made
    t.m(|m| {
        assert!(m.get(id).is_none());
        let s = m.selected().unwrap();
        assert_eq!((s.name.as_str(), s.own, m.mode), ("Countdown", true, Mode::Cd));
        assert_eq!(m.timers.len(), 5);
    });
    // a row click picks it (and the switch follows its kind): Pizza is a countdown
    t.click(idx(K_ROW, 2));
    assert_eq!(t.m(|m| (m.sel, m.mode)), (2, Mode::Cd));
    // a row's play / pause
    t.click(sub(idx(K_ROW, 3), "pb"));
    assert!(t.m(|m| m.get(3).unwrap().running()));
}

/// Order 078: "theres always one timer down here no matter what" - the big Stopwatch / Countdown on top is the tab's own and
/// never a row of "Your timers"; removing the last added timer leaves the list empty.
#[test]
fn the_list_holds_only_added_timers_and_can_be_empty() {
    let mut t = T::new(false);
    assert_eq!(t.m(|m| (m.listed().count(), m.timers.len(), m.selected().unwrap().own)), (0, 1, true));
    // the page builds with no list box at all (header + hero + the add button row)
    let kids = t.build();
    assert_eq!(kids.len(), 3);
    // add two, remove both: back to nothing, and the own stopwatch is shown
    t.click(K_ADD);
    let a = t.m(|m| m.sel);
    t.click(K_ADD);
    let b = t.m(|m| m.sel);
    assert_eq!(t.m(|m| m.listed().map(|x| x.name.clone()).collect::<Vec<_>>()), ["Stopwatch 2", "Stopwatch 3"]);
    t.click(sub(idx(K_ROW, a as usize), "del"));
    // (the picked one is b: removing a keeps b picked)
    assert_eq!(t.m(|m| m.sel), b);
    t.click(sub(idx(K_ROW, b as usize), "del"));
    t.m(|m| {
        assert_eq!((m.listed().count(), m.timers.len()), (0, 1), "no timer is made to fill the list");
        assert!(m.selected().unwrap().own && m.mode == Mode::Sw);
    });
    // the own timers cannot be removed (no button, and the model refuses too)
    let own = t.m(|m| m.sel);
    t.m(|m| m.remove(own));
    assert_eq!(t.m(|m| m.timers.len()), 1);
    // the same on the Countdown tab
    t.click(idx(K_SEG, 1));
    assert_eq!(t.m(|m| (m.listed().count(), m.timers.len(), m.selected().unwrap().own)), (0, 2, true));
    t.click(K_ADD);
    let c = t.m(|m| m.sel);
    t.m(|m| assert_eq!(m.selected().unwrap().name, "Countdown 2"));
    t.click(sub(idx(K_ROW, c as usize), "del"));
    t.m(|m| {
        assert_eq!((m.listed().count(), m.timers.len(), m.mode), (0, 2, Mode::Cd));
        assert_eq!(m.selected().unwrap().name, "Countdown");
    });
    assert_eq!(t.build().len(), 3);
}

/// Order 078: a picked listed timer goes back to the tab's own one by its tab; On screen and the key work for both.
#[test]
fn the_tab_returns_to_its_own_timer_and_on_screen_works_for_both() {
    let mut t = T::new(false);
    let own = t.m(|m| m.sel);
    t.click(K_ADD);
    let extra = t.m(|m| m.sel);
    assert_ne!(own, extra);
    // the Stopwatch tab clicked while the added one is shown: the own stopwatch comes back
    t.click(idx(K_SEG, 0));
    assert_eq!(t.m(|m| m.sel), own);
    // On screen for the own one, then for the listed one (the row's switch)
    t.click(K_SCR);
    assert!(t.m(|m| m.get(own).unwrap().screen));
    t.click(sub(idx(K_ROW, extra as usize), "sb"));
    assert!(t.m(|m| m.get(extra).unwrap().screen));
    assert_eq!(t.m(|m| m.pills(true).len()), 2);
    // the key (Bind) belongs to whichever timer is shown: its own action each
    assert_ne!(key_action(own), key_action(extra));
    // coming back from the world clock, a picked timer of that kind stays picked
    t.click(idx(K_ROW, extra as usize));
    t.click(idx(K_SEG, 2));
    t.click(idx(K_SEG, 0));
    assert_eq!(t.m(|m| m.sel), extra);
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

/// The "Add a place" list's popup, built the way the frame builds it.
fn popup_of(t: &mut T) -> Option<El> {
    let mut cx = Cx::new(t.now, true, &t.g, &mut t.st);
    t.page.popup(&mut cx)
}

fn type_in_search(t: &mut T, text: &str) {
    for c in text.chars() {
        t.ev(Ev::Char(K_CQ, c));
    }
}

fn open_add_place(t: &mut T) {
    t.ev(Ev::Press(K_ADD, 500.0, 300.0, (466.0, 236.0, 96.0, 22.0)));
}

fn cities(t: &T) -> Vec<String> {
    t.m(|m| m.places.iter().map(|p| format!("{} \u{b7} {}", p.place.city, p.place.land)).collect())
}

/// Order 078: "i thought it would be a search bar ... search any place up and it show the time there".
#[test]
fn add_a_place_by_searching_any_city() {
    let mut t = T::new(true);
    t.click(idx(K_SEG, 2));
    assert_eq!(t.m(|m| m.mode), Mode::Clk);
    open_add_place(&mut t);
    assert!(t.page.menu.is_some());
    // the box opens with the search field focused, nothing typed: a hint and no results
    assert_eq!(t.st.focus, Some(K_CQ));
    assert!(popup_of(&mut t).is_some());
    assert!(t.m(|m| m.search_places("")).is_empty());
    // one letter is not enough
    type_in_search(&mut t, "z");
    let typed = t.page.cq.clone();
    assert!(t.m(|m| m.search_places(&typed)).is_empty());
    // a city that was never in the old list of ten: results carry the country and the time there (Zagreb = the PC's own)
    type_in_search(&mut t, "agr");
    let found = t.m(|m| m.search_places("zagr"));
    assert_eq!((found[0].place.city, found[0].place.land, found[0].time.as_str(), found[0].day), ("Zagreb", "Croatia", "21:37", 0));
    assert!(popup_of(&mut t).is_some());
    // Enter adds the first one and closes the box
    t.ev(Ev::Key(K_CQ, 0x0D));
    assert!(t.page.menu.is_none() && t.page.cq.is_empty());
    assert_eq!(cities(&t), ["New York \u{b7} USA", "Tokyo \u{b7} Japan", "Zagreb \u{b7} Croatia"]);
    // the added place shows its time through Windows' zone rules (the fake: Zagreb = the drawing's 21:37)
    let w = t.m(|m| m.world());
    assert_eq!(w.places[2], ("Zagreb".into(), "21:37".into(), "Croatia \u{b7} same time".into()));
}

#[test]
fn search_results_keep_accents_off_skip_added_places_and_show_the_day() {
    let mut t = T::new(true);
    t.click(idx(K_SEG, 2));
    // accents and capitals do not matter; a place already in the list is not offered again
    let r = t.m(|m| m.search_places("TOKY"));
    assert!(!r.iter().any(|f| f.place.city == "Tokyo"));
    assert!(r.iter().any(|f| f.place.city.starts_with("Tokyo") || f.place.city.contains("Tokyo")), "{r:?}");
    let r = t.m(|m| m.search_places("s\u{e3}o pa"));
    assert_eq!((r[0].place.city, r[0].place.land), ("S\u{e3}o Paulo", "Brazil"));
    assert_eq!((r[0].time.as_str(), r[0].day), ("16:37", 0));
    // a place on the other side of the date line shows the day
    let r = t.m(|m| m.search_places("osaka"));
    assert_eq!((r[0].place.city, r[0].time.as_str(), r[0].day), ("Osaka", "04:37", 1));
    // at most eight, and the country alone lists its biggest
    assert_eq!(t.m(|m| m.search_places("san")).len(), 8);
    let r = t.m(|m| m.search_places("croatia"));
    assert_eq!((r[0].place.city, r[0].place.land), ("Zagreb", "Croatia"));
    // nothing found
    assert!(t.m(|m| m.search_places("qqqqzzzz")).is_empty());
}

#[test]
fn add_a_place_by_click_arrows_remove_and_dismiss() {
    let mut t = T::new(true);
    t.click(idx(K_SEG, 2));
    // typed, then a click on the second result
    open_add_place(&mut t);
    type_in_search(&mut t, "san");
    let second = t.m(|m| m.search_places("san")[1].place);
    t.ev(Ev::Click(idx(K_MENU, 1)));
    assert!(t.page.menu.is_none());
    assert_eq!(t.m(|m| m.places.last().unwrap().place), second);
    // arrows move the pick, Enter takes it; Backspace edits
    open_add_place(&mut t);
    type_in_search(&mut t, "san");
    t.ev(Ev::Key(K_CQ, 0x28));
    assert_eq!(t.page.cq_sel, 1);
    t.ev(Ev::Key(K_CQ, 0x26));
    assert_eq!(t.page.cq_sel, 0);
    t.ev(Ev::Key(K_CQ, 0x08));
    assert_eq!(t.page.cq, "sa");
    t.ev(Ev::Key(K_CQ, 0x0D));
    assert_eq!(t.m(|m| m.places.len()), 4);
    // a click beside the open list closes it and forgets the text
    open_add_place(&mut t);
    type_in_search(&mut t, "par");
    t.page.popup_dismiss();
    assert!(t.page.menu.is_none() && t.page.cq.is_empty());
    // the x of the search box clears the text
    open_add_place(&mut t);
    type_in_search(&mut t, "par");
    t.ev(Ev::Click(sub(K_CQ, "x")));
    assert!(t.page.cq.is_empty() && t.page.menu.is_some());
    t.page.popup_dismiss();
    // Enter with nothing found changes nothing and keeps the box
    open_add_place(&mut t);
    type_in_search(&mut t, "qqqqzzzz");
    t.ev(Ev::Key(K_CQ, 0x0D));
    assert!(t.page.menu.is_some() && t.m(|m| m.places.len()) == 4);
    t.page.popup_dismiss();
    // Esc closed the list but left the field focused: typing + Enter add nothing, and the field lets go
    open_add_place(&mut t);
    t.page.popup_dismiss();
    type_in_search(&mut t, "paris");
    t.ev(Ev::Key(K_CQ, 0x0D));
    assert!(t.m(|m| m.places.len()) == 4 && t.page.cq.is_empty() && t.st.focus != Some(K_CQ));
    // Down stops at the last result
    open_add_place(&mut t);
    type_in_search(&mut t, "zagreb");
    for _ in 0..5 {
        t.ev(Ev::Key(K_CQ, 0x28));
    }
    assert_eq!(t.page.cq_sel, 0);
    t.page.popup_dismiss();
    // remove by the row's x; no places: the empty line
    for _ in 0..4 {
        t.click(sub(idx(K_PLACE, 0), "del"));
    }
    assert_eq!(t.m(|m| m.places.len()), 0);
    let kids = t.build();
    assert_eq!(kids.len(), 3);
    // the add button of Stopwatch / Countdown never opens the places list
    t.click(idx(K_SEG, 0));
    t.ev(Ev::Press(K_ADD, 500.0, 300.0, (466.0, 236.0, 96.0, 22.0)));
    assert!(t.page.menu.is_none());
}

/// The same place twice is one place; two places with the same name stay two.
#[test]
fn same_names_stay_two_places() {
    let mut t = T::new(true);
    t.click(idx(K_SEG, 2));
    let a = bu_timers::cities::search("springfield", 40);
    assert!(a.len() > 3);
    t.m(|m| {
        for p in &a {
            m.add_place(*p);
        }
        for p in &a {
            m.add_place(*p);
        }
        assert_eq!(m.places.len(), a.len() + 2, "the same place twice is one (+ the two of the sample)");
    });
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
fn mode_switch_shows_the_tabs_own_timer() {
    let mut t = T::new(false);
    t.click(idx(K_SEG, 1));
    t.m(|m| {
        assert_eq!(m.timers.len(), 2);
        assert_eq!(m.selected().unwrap().name, "Countdown");
        assert!(m.selected().unwrap().own);
    });
    t.click(idx(K_SEG, 0));
    assert_eq!(t.m(|m| m.selected().unwrap().name.clone()), "Stopwatch");
    assert!(t.page.hero_at.is_some());
    // the same tab clicked again, own timer shown: nothing happens
    t.page.hero_at = None;
    t.click(idx(K_SEG, 0));
    assert!(t.page.hero_at.is_none());
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

/// Order 049 proof (run by hand: `cargo test -p bu-app --release pill_paint_cost -- --ignored --nocapture`): one pill
/// paint, the old way (a new Skia surface, copied out, a new DC + DIB, copied in, deleted) vs the kept DIB Skia draws into.
/// Off-screen: no window, nothing shown (the hand-over to the window is the same call both ways and is left out).
#[test]
#[ignore]
fn pill_paint_cost() {
    use windows::Win32::Graphics::Gdi::*;
    let t = T::new(true);
    t.m(|m| {
        m.type_time(3, "5m");
        m.toggle(3);
    });
    let pills = t.m(|m| m.pills(false));
    let (g, icons, s) = (Gfx::new(1.5), crate::icons::Icons::new(), 1.5f32);
    let size = (overlay::PILL_W, overlay::PILL_H);
    let (bw, bh) = (((size.0 + 96.0) * s).ceil() as i32, ((size.1 + 96.0) * s).ceil() as i32);
    let n = 500;
    let t0 = crate::timing::now();
    for _ in 0..n {
        let mut surf = crate::gfx::new_surface(bw, bh).unwrap();
        g.begin(surf.canvas());
        overlay::paint_stack(&g, &icons, &pills, false, false, 48.0, 48.0, None);
        g.end();
        let px = crate::png::from_surface(&mut surf);
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: bw, biHeight: -bh, biPlanes: 1, biBitCount: 32, ..Default::default() }, ..Default::default() };
            let mut bits = std::ptr::null_mut();
            let bmp = CreateDIBSection(Some(mem), &bi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
            std::ptr::copy_nonoverlapping(px.data.as_ptr(), bits as *mut u8, px.data.len());
            let old = SelectObject(mem, bmp.into());
            SelectObject(mem, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
        }
    }
    let before = (crate::timing::now() - t0) / n as f64;
    let mut dib = crate::dib::Dib::new();
    let t0 = crate::timing::now();
    for _ in 0..n {
        let mut surf = dib.surface(bw, bh).unwrap();
        g.begin(surf.canvas());
        overlay::paint_stack(&g, &icons, &pills, false, false, 48.0, 48.0, None);
        g.end();
    }
    let after = (crate::timing::now() - t0) / n as f64;
    println!("one pill paint at 150 %: before {before:.3} ms, after {after:.3} ms ({bw}x{bh} px)");
}

/// Order 049: the kept DIB gives the window exactly the pixels the old fresh surface did (no window, nothing shown).
#[test]
fn kept_pixels_are_the_same_picture() {
    let t = T::new(true);
    t.m(|m| {
        m.type_time(3, "5m");
        m.toggle(3);
    });
    let pills = t.m(|m| m.pills(true));
    let (g, icons) = (Gfx::new(1.25), crate::icons::Icons::new());
    let (bw, bh) = (380, 260);
    let mut fresh = crate::gfx::new_surface(bw, bh).unwrap();
    g.begin(fresh.canvas());
    overlay::paint_stack(&g, &icons, &pills, true, false, 48.0, 48.0, None);
    g.end();
    let want = crate::png::from_surface(&mut fresh);
    let mut dib = crate::dib::Dib::new();
    // twice: the second paint reuses (and first clears) the same pixels
    for _ in 0..2 {
        let mut s = dib.surface(bw, bh).unwrap();
        g.begin(s.canvas());
        overlay::paint_stack(&g, &icons, &pills, true, false, 48.0, 48.0, None);
        g.end();
        let got = crate::png::from_surface(&mut s);
        assert!(got.data == want.data, "byte for byte");
    }
}

/// Order 055: a running stopwatch asks for its own next look (its hundredths at 30 Hz at most), with no input at all; a page
/// with nothing running asks for nothing. A still screen does not rebuild between two looks.
#[test]
fn a_running_stopwatch_wakes_itself_at_30_hz_and_a_resting_page_sleeps() {
    let mut t = T::new(false);
    assert_eq!(t.page.wake_at(0.0), None);
    t.click(K_GO);
    t.build();
    t.advance(10);
    let w = t.page.wake_at(t.now).expect("a running stopwatch wakes itself");
    assert!(w > t.now && w <= t.now + 33.0, "wake {w} now {}", t.now);
    // the hundredths changed but the last repaint is only 10 ms old: no frame yet
    assert!(!t.page.tick(t.now));
    t.advance(25);
    assert!(t.page.tick(t.now), "33 ms later the hundredths repaint");
    // a whole second changes the digits at once
    t.build();
    t.advance(1_000);
    assert!(t.page.tick(t.now));
    // stopped: one last repaint, then it sleeps
    t.click(K_GO);
    assert!(t.page.tick(t.now));
    assert_eq!(t.page.wake_at(t.now), None);
    assert!(!t.page.tick(t.now));
}

/// Order 078 pictures, drawn off-screen (no window, nothing on the screen): `BU_TMR_PICS=<dir> cargo test -p bu-app
/// timers::tests::pictures_078 -- --ignored`.
#[test]
#[ignore]
fn pictures_078() {
    let Ok(dir) = std::env::var("BU_TMR_PICS") else { return };
    let _ = std::fs::create_dir_all(&dir);
    // SAFETY: COM for the WIC encoder on this test thread.
    let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
    let icons = crate::icons::Icons::new();
    let shot = |t: &mut T, name: &str, popup: bool| {
        t.now += 2000.0;
        let mut cx = Cx::new(t.now, false, &t.g, &mut t.st).for_page("tmr");
        let kids = t.page.build(&mut cx);
        let pop = if popup { t.page.popup(&mut cx) } else { None };
        drop(cx);
        let page = crate::ui::lay::Laid::new(&t.g, El::block().w(crate::ui::WIN_W).pad(2.0, 26.0, 18.0, 26.0).children(kids), crate::ui::WIN_W, None);
        let (w, h) = (crate::ui::WIN_W, crate::ui::WIN_H);
        let mut s = crate::gfx::new_surface(w as i32, h as i32).unwrap();
        t.g.begin(s.canvas());
        t.g.fill_rect(0.0, 0.0, w, h, crate::gfx::Rgba::rgb(28, 30, 38));
        page.paint(&t.g, &icons, 0.0, crate::ui::PAGE_TOP, None);
        if let Some(p) = pop {
            let l = crate::ui::lay::Laid::new(&t.g, El::block().w(w).h(h).child(p), w, Some(h));
            l.paint(&t.g, &icons, 0.0, 0.0, None);
        }
        t.g.end();
        let px = crate::png::from_surface(&mut s);
        crate::png::save_png(&px, &format!("{dir}/{name}.png")).expect("save");
    };
    // 1. a fresh Stopwatch tab: no "Your timers" at all, only the New timer button
    let mut t = T::new(false);
    shot(&mut t, "1_stopwatch_no_list", false);
    // 2. two timers added: they are listed (the top one is not)
    t.click(K_ADD);
    t.ev(Ev::Key(K_NAME, 0x0D));
    t.click(K_ADD);
    shot(&mut t, "2_stopwatch_two_added", false);
    // 3. the Countdown tab with the drawing's sample list
    let mut t = T::new(true);
    t.click(idx(K_SEG, 1));
    shot(&mut t, "3_countdown_sample", false);
    // (the pictures read Windows' own zone rules - read-only - so the times shown are real)
    t.m(|m| m.set_zones(Box::new(bu_timers::zones::RealZones)));
    // 4. World clock, the Add a place box just opened (the button's box as the page reports it), nothing typed
    t.click(idx(K_SEG, 2));
    t.ev(Ev::Press(K_ADD, 500.0, 300.0, (466.0, 236.0, 96.0, 22.0)));
    shot(&mut t, "4_world_add_empty", true);
    // 5. typed "san": results with country + time there
    type_in_search(&mut t, "san");
    shot(&mut t, "5_world_add_san", true);
    // 6. typed a city never in the old list, one hit
    t.ev(Ev::Key(K_CQ, 0x08));
    t.ev(Ev::Key(K_CQ, 0x08));
    t.ev(Ev::Key(K_CQ, 0x08));
    type_in_search(&mut t, "mumbai");
    shot(&mut t, "6_world_add_mumbai", true);
    // 7. added, the list has it with its time
    t.ev(Ev::Key(K_CQ, 0x0D));
    shot(&mut t, "7_world_after_add", false);
    // 8. nothing found
    t.ev(Ev::Press(K_ADD, 500.0, 300.0, (466.0, 236.0, 96.0, 22.0)));
    type_in_search(&mut t, "qqqqzzzz");
    shot(&mut t, "8_world_add_none", true);
}
