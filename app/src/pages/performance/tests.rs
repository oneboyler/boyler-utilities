//! Performance against the FAKE bu-perf (nothing on the PC changes; no thread runs in a test copy).

use super::*;
use crate::gfx::Gfx;
use crate::ui::cx::State;

fn page() -> Performance {
    let mut p = Performance::default();
    p.open(&Env { test: true, ..Env::default() }, 0.0);
    p
}

fn click(p: &mut Performance, k: Key) {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st);
    p.event(&Ev::Click(k), &mut cx);
}

fn build(p: &mut Performance) {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st);
    let _ = p.build(&mut cx);
    let _ = p.popup(&mut cx);
}

fn names(p: &Performance) -> Vec<String> {
    p.rows().iter().map(|r| r.display_name()).collect()
}

#[test]
fn opens_with_the_drawings_values() {
    let p = page();
    assert!(p.worker.is_none(), "a test copy starts no thread");
    assert_eq!(
        names(&p),
        [
            "VALORANT (2)",
            "OBS Studio",
            "Google Chrome (15)",
            "Discord (6)",
            "Spotify (5)",
            "Windows Explorer",
            "Steam (4)",
            "NVIDIA Container (3)",
            "Wootility (3)",
            "Riot Vanguard tray"
        ]
    );
    let v = &p.rows()[0];
    assert_eq!(processes::format_pct(v.cpu_pct), "9.4 %");
    assert_eq!(processes::format_ram(v.ram_bytes), "3.1 GB");
    assert_eq!(processes::format_pct(v.gpu_pct), "31.0 %");
    let cells: Vec<String> = p.snap.specs.as_ref().unwrap().cells().into_iter().map(|c| c.value).collect();
    assert_eq!(cells[6], "Intel Ethernet I226-V · 2.5 Gbps");
    assert_eq!(short_cpu("AMD Ryzen 7 7800X3D 8-Core Processor"), "Ryzen 7 7800X3D");
    assert_eq!(short_cpu("Intel(R) Core(TM) i7-14700K"), "i7-14700K");
    assert_eq!(gbf(612 * 1024 * 1024 * 1024), "612 GB");
}

#[test]
fn sort_search_and_windows_processes() {
    let mut p = page();
    click(&mut p, idx(K_SORT, 0));
    assert_eq!(names(&p)[0], "Discord (6)", "Name: A -> Z");
    click(&mut p, idx(K_SORT, 0));
    assert_eq!(names(&p)[0], "Wootility (3)", "again = the other way");
    click(&mut p, idx(K_SORT, 2));
    assert_eq!(names(&p)[0], "VALORANT (2)", "RAM: high first");
    p.query = "st".into();
    assert_eq!(names(&p), ["OBS Studio", "Steam (4)"]);
    p.query = "zzz".into();
    assert!(p.rows().is_empty());
    build(&mut p);
    p.query.clear();
    click(&mut p, K_WPS);
    assert!(p.show_windows);
    assert_eq!(p.rows().len(), 17);
    let dwm = p.rows().into_iter().find(|r| r.name == "Desktop Window Manager").unwrap();
    assert!(dwm.windows_own && dwm.end_rule == EndRule::Locked);
}

#[test]
fn end_a_plain_app_at_once_and_ask_for_the_rest() {
    let mut p = page();
    build(&mut p);
    let n = p.rows().iter().position(|r| r.name == "Spotify").unwrap();
    click(&mut p, idx(K_END, n));
    assert_eq!(p.toast.as_ref().unwrap().0, "Spotify and its 4 helper processes closed");
    let acts = p.fake.as_ref().unwrap().0.actions();
    assert!(acts.iter().any(|a| a.starts_with("end ")), "{acts:?}");
    // Explorer asks first
    p.refresh_fake();
    let n = p.rows().iter().position(|r| r.name == "Windows Explorer").unwrap();
    p.pressed = (500.0, 300.0, 40.0, 22.0);
    click(&mut p, idx(K_END, n));
    assert!(matches!(p.pop, Some(Pop::Ask(..))));
    build(&mut p);
    click(&mut p, sub(K_ASK, "no"));
    assert!(p.pop.is_none() && p.rows().iter().any(|r| r.name == "Windows Explorer"), "Cancel ends nothing");
}

#[test]
fn right_click_menu_priority_and_location() {
    let mut p = page();
    let key = p.rows()[1].key.clone();
    p.context(&key, 200.0, 300.0);
    build(&mut p);
    click(&mut p, idx(K_MENU, 5));
    assert!(matches!(p.pop, Some(Pop::Menu(_, _, _, true))));
    build(&mut p);
    click(&mut p, idx(sub(K_MENU, "p"), 1));
    assert!(p.toast.as_ref().unwrap().0.contains("priority high"), "{:?}", p.toast);
    p.context(&key, 200.0, 300.0);
    click(&mut p, idx(K_MENU, 4));
    assert!(p.log.last().unwrap().starts_with(r"open:C:\Program Files\obs-studio"), "{:?}", p.log);
}

#[test]
fn copy_all_says_copied_and_never_touches_the_clipboard_in_a_test() {
    let mut p = page();
    click(&mut p, K_COPY);
    assert_eq!(p.log, vec!["copy:8".to_string()]);
    assert_eq!(p.toast.as_ref().unwrap().0, "Your PC copied · paste it anywhere");
    assert!(p.copied.is_some());
}

#[test]
fn closing_stops_everything() {
    let mut p = page();
    p.close();
    assert!(p.worker.is_none() && p.fake.is_none() && p.snap.rows.is_empty() && p.snap.latest.is_none());
}

fn texts(e: &El, out: &mut Vec<String>) {
    if let crate::ui::el::Content::Text(t) = &e.content {
        out.push(t.s.clone());
    }
    for c in &e.children {
        texts(c, out);
    }
}

fn page_texts(p: &mut Performance) -> Vec<String> {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st);
    let mut v = Vec::new();
    for e in p.build(&mut cx) {
        texts(&e, &mut v);
    }
    v
}

/// A tick with new numbers: every row's CPU % becomes `f(index)`, then the page takes the snapshot like `tick` does.
fn tick_cpu(p: &mut Performance, f: impl Fn(usize) -> f64) {
    for (i, r) in p.snap.rows.iter_mut().enumerate() {
        r.cpu_pct = f(i);
    }
    p.on_snap();
}

#[test]
fn live_order_off_keeps_the_rows_where_they_are_while_the_numbers_change() {
    let mut p = page();
    assert!(!p.live, "Live order starts off");
    let before = names(&p);
    // the order of the CPU numbers is turned upside down; the rows stay, the numbers are the new ones
    tick_cpu(&mut p, |i| i as f64 * 3.0);
    assert_eq!(names(&p), before);
    let new: Vec<f64> = p.snap.rows.iter().map(|r| r.cpu_pct).collect();
    for r in p.rows() {
        let i = p.snap.rows.iter().position(|s| s.key == r.key).unwrap();
        assert_eq!(r.cpu_pct, new[i], "the number in the row is the new one");
    }
    tick_cpu(&mut p, |i| 50.0 - i as f64);
    assert_eq!(names(&p), before, "still the same rows in the same places");
}

#[test]
fn live_order_off_new_rows_go_to_the_end_and_ended_ones_drop_out() {
    let mut p = page();
    let before = names(&p);
    let gone = p.snap.rows.remove(1);
    let mut fresh = p.snap.rows[0].clone();
    fresh.key = "new".into();
    fresh.name = "Brand new".into();
    fresh.pids.truncate(1);
    fresh.cpu_pct = 99.0;
    p.snap.rows.insert(0, fresh);
    p.on_snap();
    let now = names(&p);
    let mut want: Vec<String> = before.iter().filter(|n| **n != gone.display_name()).cloned().collect();
    want.push("Brand new".into());
    assert_eq!(now, want);
}

#[test]
fn a_sort_click_re_sorts_once_and_the_rows_stay_after_it() {
    let mut p = page();
    tick_cpu(&mut p, |i| i as f64 * 3.0);
    click(&mut p, idx(K_SORT, 1));
    // CPU was the sort: the click turns it round (low first), once, by the numbers of that moment
    let cpu: Vec<f64> = p.rows().iter().map(|r| r.cpu_pct).collect();
    assert!(cpu.windows(2).all(|w| w[0] <= w[1]), "{cpu:?}");
    let sorted = names(&p);
    tick_cpu(&mut p, |i| 50.0 - i as f64);
    assert_eq!(names(&p), sorted, "the next tick moves nothing");
    click(&mut p, idx(K_SORT, 2));
    assert_eq!(names(&p)[0], "VALORANT (2)", "RAM: high first, one more re-sort");
}

#[test]
fn live_order_on_re_sorts_every_tick() {
    let mut p = page();
    click(&mut p, K_LIVE);
    assert!(p.live);
    for f in [(|i: usize| i as f64 * 3.0) as fn(usize) -> f64, |i| 50.0 - i as f64] {
        tick_cpu(&mut p, f);
        let cpu: Vec<f64> = p.rows().iter().map(|r| r.cpu_pct).collect();
        assert!(cpu.windows(2).all(|w| w[0] >= w[1]), "sorted by CPU, high first: {cpu:?}");
    }
    // off again: the rows stay as the last tick left them
    click(&mut p, K_LIVE);
    let kept = names(&p);
    tick_cpu(&mut p, |i| i as f64);
    assert_eq!(names(&p), kept);
}

#[test]
fn the_header_says_live_order_and_windows() {
    let mut p = page();
    let t = page_texts(&mut p);
    assert!(t.iter().any(|s| s == "Live order"), "{t:?}");
    assert!(t.iter().any(|s| s == "Windows"), "{t:?}");
    assert!(!t.iter().any(|s| s.contains("Show Windows processes")), "{t:?}");
}

#[test]
fn the_end_button_is_in_the_row_not_laid_over_the_numbers() {
    fn is_end(e: &El) -> bool {
        e.children.iter().any(|t| matches!(&t.content, crate::ui::el::Content::Text(x) if x.s == "End"))
    }
    fn count(e: &El, abs: bool) -> usize {
        usize::from(is_end(e) && (e.style.position == taffy::style::Position::Absolute) == abs) + e.children.iter().map(|c| count(c, abs)).sum::<usize>()
    }
    let mut p = page();
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st);
    let els = p.build(&mut cx);
    assert!(els.iter().map(|e| count(e, false)).sum::<usize>() > 0, "the rows carry End buttons in the flow");
    assert_eq!(els.iter().map(|e| count(e, true)).sum::<usize>(), 0, "no End button is positioned over the numbers");
}

/// Order 045 item 5: Ctrl+F puts the focus in the search box (whatever had it before); F alone does not.
#[test]
fn ctrl_f_focuses_the_search() {
    let mut p = page();
    let g = Gfx::new(1.0);
    let mut st = State::default();
    {
        let mut cx = Cx::new(1000.0, false, &g, &mut st);
        p.event(&Ev::Key(crate::ui::cx::PAGE, 0x46), &mut cx);
        assert!(!cx.used && cx.st.focus.is_none(), "F alone is not the search key");
    }
    let mut cx = Cx::new(1000.0, false, &g, &mut st);
    cx.mods.ctrl = true;
    p.event(&Ev::Key(crate::ui::cx::PAGE, 0x46), &mut cx);
    assert!(cx.used);
    assert_eq!(cx.st.focus, Some(K_SEARCH));
}
