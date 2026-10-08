//! The Activity page against the crate's counter over the drawing's week (memory only: nothing on the PC is read or
//! changed), + its boxes against Chromium's layout of the drawing (tools/ref/dom_dump.js, menu-v22, switch on).

use super::data::{Fake, Src, DEMO};
use super::*;
use crate::gfx::Gfx;
use crate::ui::cx::State;
use crate::ui::lay::Laid;

fn page() -> Activity {
    Activity::with_src(Box::new(Fake::demo()))
}

fn click(p: &mut Activity, k: Key, now: f64) {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(now, false, &g, &mut st);
    p.event(&Ev::Click(k), &mut cx);
}

/// A right click on `k` at (x, y).
fn context(p: &mut Activity, k: Key, now: f64) {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(now, false, &g, &mut st);
    p.event(&Ev::Context(k, 300.0, 300.0), &mut cx);
}

/// The page laid out like the frame does (`.pg`: 600 wide, padding 2 26 18 26), animations at their end.
fn laid(p: &mut Activity) -> Laid {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1e9, true, &g, &mut st);
    let _ = p.build(&mut cx);
    // a second pass: transitions started by the first build have ended (reduced motion, 10 ms)
    let mut cx = Cx::new(2e9, true, &g, &mut st);
    let kids = p.build(&mut cx);
    let root = El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
    Laid::new(&g, root, 600.0, None)
}

fn rect(l: &Laid, k: Key) -> (f32, f32, f32, f32) {
    let r = l.rect_of(k).expect("element");
    (r.0, r.1 + 56.0, r.2, r.3) // window coordinates (the page starts at y 56)
}

fn near(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    (a.0 - b.0).abs() < 0.02 && (a.1 - b.1).abs() < 0.02 && (a.2 - b.2).abs() < 0.02 && (a.3 - b.3).abs() < 0.02
}

#[test]
fn off_by_default_and_nothing_counts_on_open() {
    let p = page();
    assert!(!p.on);
    assert!(p.sum.is_none(), "off: nothing is read for the tiles / lists");
    assert!(p.describe().starts_with("on=0 counting=0"), "{}", p.describe());
}

#[test]
fn switch_on_shows_the_drawings_numbers_and_off_stops_counting() {
    let mut p = page();
    click(&mut p, K_ON, 0.0);
    assert!(p.on);
    let s = p.sum.clone().unwrap();
    assert_eq!(s.status, "Counting since Wed 30 Sep");
    assert_eq!(hm_s(s.screen_today_ms), "6 h 18 m");
    assert_eq!(hm_s(s.games_today_ms), "2 h 41 m");
    assert_eq!(hm_s(s.games_week_ms), "17 h 31 m");
    assert_eq!(bu_activity::views::fmt_uptime(s.uptime_ms), "1 d 3 h");
    assert!(s.uptime_text.ends_with("since Mon 18:02"), "{}", s.uptime_text);
    assert!(s.screen_text.ends_with("today \u{b7} since 11:24"), "{}", s.screen_text);
    let rows = p.rows();
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["VALORANT", "Google Chrome", "Discord", "OBS Studio", "Steam", "File Explorer", "Spotify", "Settings", "Notepad", "Boyler Utilities"]);
    assert_eq!(rows.iter().map(|r| hm(r.ms)).collect::<Vec<_>>()[..3], ["2 h 41 m", "1 h 52 m", "47 m"]);
    assert!(rows[0].game && !rows[1].game);
    // the 7 days: every column = the drawing's WEEK
    let cols: Vec<(u64, u64)> = s.last7.iter().map(|d| (d.total_ms / 60_000, d.games_ms / 60_000)).collect();
    assert_eq!(cols, [(250, 110), (305, 140), (400, 185), (500, 250), (365, 165), (210, 40), (378, 161)]);
    // switched on: the bars grow in
    assert!(p.grow_at.is_some());
    // off: the counter stops (the fake counts nothing more), the body folds away
    click(&mut p, K_ON, 10.0);
    assert!(!p.on && p.sum.is_none());
    assert!(p.describe().contains("counting=0"));
}

#[test]
fn counting_only_while_on() {
    let mut f = Fake::demo();
    f.front(DEMO[1].path, DEMO[1].name);
    f.advance_min(30);
    f.front(DEMO[2].path, DEMO[2].name);
    f.set_on(true).unwrap();
    let before = f.summary().screen_today_ms;
    assert_eq!(before, 378 * 60_000, "the 30 min while off were not counted");
    f.front(DEMO[1].path, DEMO[1].name);
    f.advance_min(20);
    assert_eq!(f.summary().screen_today_ms, before + 20 * 60_000);
    f.set_on(false).unwrap();
    f.advance_min(20);
    assert_eq!(f.summary().screen_today_ms, before + 20 * 60_000, "off: nothing more");
}

#[test]
fn range_switch_and_show_all() {
    let mut p = page();
    click(&mut p, K_ON, 0.0);
    click(&mut p, idx(K_RANGE, 1), 100.0);
    assert!(p.week && p.grow_at == Some(100.0), "7 days: the bars grow again");
    let names: Vec<String> = p.rows().iter().map(|r| r.name.clone()).collect();
    assert_eq!(names.len(), 11);
    assert_eq!(names[3], "Rocket League", "Rocket League has time in 7 days only");
    click(&mut p, idx(K_RANGE, 0), 200.0);
    assert!(!p.week);
    assert!(!p.all);
    click(&mut p, K_ALL, 300.0);
    assert!(p.all);
    click(&mut p, K_ALL, 400.0);
    assert!(!p.all);
}

#[test]
fn menu_count_as_game_not_a_game_dont_count() {
    let mut p = page();
    click(&mut p, K_ON, 0.0);
    // a left click on a row opens nothing (the menu is the right click's)
    click(&mut p, row_key(1), 5.0);
    assert!(p.menu.is_none());
    // the menu's rows: 0 = the head, 1 = count as a game / not a game, 2 = don't count
    // Google Chrome (row 1): Count as a game
    context(&mut p, row_key(1), 10.0);
    assert_eq!(p.menu.as_ref().map(|m| m.name.as_str()), Some("Google Chrome"));
    click(&mut p, idx(K_MENU, 1), 20.0);
    assert!(p.menu.is_none());
    assert_eq!(p.last_toast.as_deref(), Some("Google Chrome counts as a game now"));
    let s = p.sum.clone().unwrap();
    assert_eq!(s.games_today_ms, (161 + 112) * 60_000);
    // VALORANT: Not a game
    context(&mut p, row_key(0), 30.0);
    assert!(p.menu.as_ref().unwrap().game);
    click(&mut p, idx(K_MENU, 1), 40.0);
    assert_eq!(p.last_toast.as_deref(), Some("VALORANT no longer counts as a game"));
    assert!(!p.rows().iter().find(|r| r.name == "VALORANT").unwrap().game);
    // Don't count this app: it leaves the lists and the totals
    let i = p.rows().iter().position(|r| r.name == "Discord").unwrap();
    context(&mut p, row_key(i), 50.0);
    click(&mut p, idx(K_MENU, 2), 60.0);
    assert_eq!(p.last_toast.as_deref(), Some("Discord isn\u{2019}t counted any more \u{b7} Settings can bring it back"));
    assert!(p.rows().iter().all(|r| r.name != "Discord"));
    assert_eq!(p.sum.as_ref().unwrap().screen_today_ms, (378 - 47) * 60_000);
    // the head is not an item; the frame's dismiss (a click beside the menu) closes it
    context(&mut p, row_key(0), 70.0);
    click(&mut p, idx(K_MENU, 0), 75.0);
    assert!(p.menu.is_some(), "the head does nothing");
    p.popup_dismiss();
    assert!(p.menu.is_none());
}

#[test]
fn close_drops_the_page_state() {
    let mut p = page();
    click(&mut p, K_ON, 0.0);
    click(&mut p, K_ALL, 1.0);
    p.close();
    assert!(p.src.is_none() && p.sum.is_none() && !p.on && !p.all && p.menu.is_none());
}

#[test]
fn opening_in_a_test_copy_uses_the_fake_only() {
    let mut p = Activity::default();
    p.open(&Env { test: true, ..Env::default() }, 0.0);
    assert!(p.describe().starts_with("on=0 counting=0"));
    click(&mut p, K_ON, 1.0);
    assert!(p.describe().contains("first=VALORANT"), "the drawing's week, not the PC's");
}

#[test]
fn formats_are_the_drawings() {
    assert_eq!(hm(161 * 60_000), "2 h 41 m");
    assert_eq!(hm(65 * 60_000), "1 h 05 m");
    assert_eq!(hm(47 * 60_000), "47 m");
    assert_eq!(hm_s(60 * 60_000), "1 h");
    assert_eq!(hm_s(344 * 60_000), "5 h 44 m");
    // Blink: (112 / 161 x 100).toFixed(2) % of 228 px, in LayoutUnits
    assert_eq!(pct_px(112.0, 161.0, 228.0), 158.609_38);
    assert_eq!(pct_px(110.0, 600.0, 110.0), 20.156_25);
}

/// The boxes Chromium lays out for the drawing with the switch on (dom_dump.js, scratch\P\act\on.json).
#[test]
fn boxes_match_the_drawing() {
    let mut p = page();
    click(&mut p, K_ON, 0.0);
    p.grow_at = None;
    let l = laid(&mut p);
    let want = [
        (row_key(0), (26.0, 297.6875, 548.0, 40.0)),
        (row_key(5), (26.0, 497.6875, 548.0, 40.0)),
        (K_UP, (545.9844, 179.4219, 16.0, 16.0)),
        (col_key(0), (70.0, 616.9219, 69.7031, 130.0)),
        (col_key(6), (488.2813, 616.9219, 69.7188, 130.0)),
        (K_ALL, (38.0, 543.6875, 55.9531, 15.0)),
    ];
    let mut bad = Vec::new();
    for (k, w) in want {
        let r = rect(&l, k);
        if !near(r, w) {
            bad.push(format!("{:?} want {:?}", r, w));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
    // the whole page: .gf.actq at 763.92 + 15.39 + the 18 px bottom padding
    assert!((l.height + 56.0 - (763.9219 + 15.3906 + 18.0)).abs() < 0.02, "height {}", l.height);
}

/// The drawing's actRender(true): columns scaleY 0 -> 1 in 420 ms after i x 35 ms, row bars scaleX 0 -> 1 in 380 ms after
/// 60 + i x 30 ms (EASE_OUT, fill: backwards); they keep the frame busy until done.
#[test]
fn bars_grow_like_the_drawing() {
    let mut p = page();
    click(&mut p, K_ON, 1000.0);
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0 + 35.0 * 3.0, false, &g, &mut st);
    assert_eq!(p.grow(&mut cx, 35.0 * 3.0, 420.0), 0.0, "column 3 starts after 105 ms");
    assert!(cx.st.busy);
    let mut cx = Cx::new(1000.0 + 105.0 + 210.0, false, &g, &mut st);
    let half = p.grow(&mut cx, 105.0, 420.0);
    assert!((half - EASE_OUT.ease(0.5) as f32).abs() < 1e-6);
    let mut st2 = State::default();
    let mut cx = Cx::new(1000.0 + 60.0 + 30.0 * 9.0 + 380.0, false, &g, &mut st2);
    assert_eq!(p.grow(&mut cx, 60.0 + 30.0 * 9.0, 380.0), 1.0);
    assert!(!cx.st.busy, "done: no more frames");
}

/// the owner Oct 8 ("it shows no programs"): while the tab shows with counting on, the numbers follow the counter (read again
/// on the ticker's wake-up), and with nothing counted yet the list says so instead of standing empty.
#[test]
fn the_list_follows_the_counter_and_says_when_nothing_is_counted_yet() {
    let mut f = Fake::demo();
    f.a.set_on(true, f.now).unwrap();
    let mut p = Activity::with_src(Box::new(f));
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let first = p.sum.as_ref().map(|s| s.screen_today_ms).unwrap();
    // the page reads again only after LIVE_MS (not on every build)
    let mut cx = Cx::new(5_000.0, false, &g, &mut st);
    let _ = p.build(&mut cx);
    assert_eq!(p.read_at, 0.0);
    let mut cx = Cx::new(LIVE_MS as f64, false, &g, &mut st);
    let _ = p.build(&mut cx);
    assert_eq!(p.read_at, LIVE_MS as f64, "read again after the ticker's period");
    assert_eq!(p.sum.as_ref().map(|s| s.screen_today_ms), Some(first));
    // nothing counted: the line instead of an empty box
    let mut empty = Fake { a: bu_activity::Activity::new(bu_activity::MemStore::default(), Vec::new(), data::demo_now()), now: data::demo_now(), uptime_ms: 0 };
    empty.a.set_on(true, empty.now).unwrap();
    let mut p = Activity::with_src(Box::new(empty));
    let mut cx = Cx::new(0.0, false, &g, &mut st);
    let kids = p.build(&mut cx);
    let l = Laid::new(&g, El::block().w(600.0).children(kids), 600.0, None);
    assert!(l.nodes.iter().any(|n| matches!(&n.el.content, crate::ui::el::Content::Text(t) if t.s.starts_with("Nothing counted yet"))));
}

/// Order 047: with counting on and the bars grown in, the tab asks for no frames (its numbers come with the ticker's
/// wake-up every 10 s); only the bars' grow-in (a real motion) rebuilds it every frame.
#[test]
fn at_rest_with_counting_on_the_tab_asks_for_no_frames() {
    let mut p = page();
    click(&mut p, K_ON, 0.0);
    assert!(p.on);
    assert!(!p.tick(1.0), "nothing of the page moves by itself");
    assert_eq!(p.wake_at(1.0), None);
    let g = Gfx::new(1.0);
    let mut st = State::default();
    // the grow-in is running: built again every frame while it moves
    let mut cx = Cx::new(100.0, true, &g, &mut st);
    let _ = p.build(&mut cx);
    drop(cx);
    assert!(st.busy, "the bars grow in");
    // long after: everything at rest
    for now in [1e6, 2e6] {
        st.busy = false;
        let mut cx = Cx::new(now, true, &g, &mut st);
        let _ = p.build(&mut cx);
    }
    assert!(!st.busy, "at rest nothing asks for frames");
    assert!(!p.tick(2e6));
}
