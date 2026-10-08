//! Tweaks against the FAKE bu-toggles / bu-quickfix (nothing on the PC changes).

use super::*;
use crate::gfx::Gfx;
use crate::ui::cx::State;

/// The page opened on the fake, its open read landed (Order 047: the read runs on the page's worker).
fn page() -> Tweaks {
    let mut p = Tweaks::default();
    p.open(&Env { test: true, ..Env::default() }, 0.0);
    p.settle();
    p
}

/// The page's ONE service (the worker's too).
fn lock_svc(p: &Tweaks) -> MutexGuard<'_, Svc> {
    lock(p.svc.as_ref().unwrap())
}

/// A click, then its worker job's answer landed (Order 047).
fn click(p: &mut Tweaks, k: Key) {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st);
    p.event(&Ev::Click(k), &mut cx);
    p.settle();
}

fn build(p: &mut Tweaks) -> usize {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st);
    let kids = p.build(&mut cx);
    let _ = p.popup(&mut cx);
    kids.len()
}

#[test]
fn opens_at_the_drawings_sample_values() {
    let p = page();
    for (i, r) in ROWS.iter().enumerate() {
        match r.sample {
            svc::Sample::On(on) => assert_eq!(p.value(i), Some(&Value::Switch(on)), "{}", r.id),
            svc::Sample::Time(s) => assert_eq!(p.value(i), Some(&Value::Timeout(Timeout::from_seconds(s))), "{}", r.id),
            svc::Sample::Games => assert_eq!(p.games.len(), 1),
        }
    }
    let d = p.defaults.as_ref().unwrap();
    assert_eq!(d.browser.app.as_ref().unwrap().name, "Chrome");
    assert_eq!(d.file_types.len(), 8);
    // nothing was changed by opening the page
    assert!(lock_svc(&p).fake().unwrap().log.is_empty());
}

#[test]
fn every_drawn_row_has_its_crate_row() {
    for r in ROWS {
        assert!(bu_toggles::rows::find(r.crate_id).is_some(), "{}", r.id);
    }
    assert_eq!(ROWS.len(), 50, "52 drawn rows minus Hibernate and Windows key lock (Order 040); Calls turn other sounds down is in since Order 045");
}

#[test]
fn a_switch_flips_reads_back_and_toasts() {
    let mut p = page();
    let i = row_ix("secs").unwrap();
    assert!(!p.on(i));
    click(&mut p, idx(K_TG, i));
    assert!(p.on(i));
    // the same through the crate: read back
    assert_eq!(lock_svc(&p).read("clock_seconds").unwrap().value, Value::Switch(true));
    let j = row_ix("micacc").unwrap();
    click(&mut p, idx(K_TG, j));
    assert_eq!(p.toast.as_ref().unwrap().0, "No app can use your microphone now · Discord too");
}

#[test]
fn admin_rows_say_so_and_change_nothing() {
    let mut p = page();
    let i = row_ix("hags").unwrap();
    let before = p.value(i).cloned();
    click(&mut p, idx(K_TG, i));
    assert_eq!(p.value(i).cloned(), before);
    assert_eq!(p.toast.as_ref().unwrap().0, crate::admin::NOT_CHANGED);
}

#[test]
fn hibernate_off_in_windows_greys_fast_startup() {
    let mut p = page();
    lock_svc(&p).fake_mut().unwrap().hibernate_on = false;
    p.refresh(false);
    p.settle();
    let f = row_ix("fast").unwrap();
    assert!(!p.on(f));
    let fs = p.st[f].as_ref().unwrap();
    assert!(!fs.enabled);
    // a dimmed row can't be flipped
    click(&mut p, idx(K_TG, f));
    assert!(!p.on(f));
}

#[test]
fn hibernate_and_windows_key_lock_rows_are_gone() {
    // the owner, Oct 8: "nothing that can dramatically change the pc should be in the app"
    assert!(row_ix("hib").is_none() && row_ix("winlock").is_none());
    assert!(!ROWS.iter().any(|r| r.crate_id == "hibernate" || r.crate_id == "windows_key_lock"));
}

#[test]
fn old_change_log_entries_of_removed_rows_are_ignored() {
    start();
    let p = page();
    // a change log written before Order 040: Hibernate switched off once
    crate::services::with(|s| crate::undo::record(&mut s.store, "tgl", "hib", "Hibernate", &Val::new("on", "On"), &Val::new("off", "Off")))
        .unwrap()
        .unwrap();
    assert!(review(&p, RKind::HowItWas).lines.is_empty(), "no line for a row the app no longer has");
    crate::services::shutdown();
}

#[test]
fn a_time_list_sets_the_timeout() {
    let mut p = page();
    let i = row_ix("scroff").unwrap();
    p.pressed = (400.0, 100.0, 128.0, 24.0);
    click(&mut p, idx(K_TO, i));
    assert!(matches!(p.pop, Some(Pop::Times(..))));
    build(&mut p);
    click(&mut p, idx(K_TMENU, time_row(9)));
    assert_eq!(p.value(i), Some(&Value::Timeout(Timeout::Never)));
    assert_eq!(p.toast.as_ref().unwrap().0, "The screen stays on");
    assert!(p.pop.is_none());
}

#[test]
fn games_window_add_switch_remove() {
    let mut p = page();
    click(&mut p, K_FSO);
    assert!(matches!(p.pop, Some(Pop::Games(_))));
    build(&mut p);
    click(&mut p, K_FADD);
    assert_eq!(p.games.len(), 2);
    assert_eq!(p.toast.as_ref().unwrap().0, "Counter-Strike 2 added · off from its next start");
    let cs = p.games.iter().position(|g| g.exe.ends_with("cs2.exe")).unwrap();
    click(&mut p, idx(K_FTG, cs));
    assert!(!p.games[cs].off, "switched back on stays in the list");
    assert!(!lock_svc(&p).fso_games().unwrap().iter().any(|g| g.exe.ends_with("cs2.exe")));
    click(&mut p, idx(K_FDEL, cs));
    assert_eq!(p.games.len(), 1);
    click(&mut p, sub(K_FDLG, "x"));
    assert!(p.pop.is_none());
}

#[test]
fn search_filters_rows_and_folds_stay() {
    let mut p = page();
    p.query = "bluetooth".into();
    let n = build(&mut p);
    // header + one group (Devices & power) + the reset line
    assert_eq!(n, 3);
    p.query = "zzzz".into();
    let n = build(&mut p);
    assert_eq!(n, 3, "header + Nothing matches + reset line");
    p.query.clear();
    click(&mut p, idx(K_GH, 0));
    assert!(p.shut[0]);
    p.query = "file".into();
    click(&mut p, idx(K_GH, 1));
    assert!(!p.shut[1], "no folding while searching");
}

#[test]
fn default_apps_change_opens_windows_own_window() {
    let mut p = page();
    click(&mut p, idx(K_DPK, 1));
    assert_eq!(lock_svc(&p).fake().unwrap().log.last().map(String::as_str), Some("open_with:.png"));
    p.pressed = (480.0, 300.0, 60.0, 22.0);
    click(&mut p, idx(K_DPK, 0));
    build(&mut p);
    // Edge (sorted: Chrome, Edge, Firefox)
    click(&mut p, idx(K_BMENU, 1));
    assert!(lock_svc(&p).fake().unwrap().log.last().unwrap().starts_with("open_uri:ms-settings:defaultapps"));
}

#[test]
fn closing_drops_everything() {
    let mut p = page();
    p.close();
    assert!(p.svc.is_none() && p.qf.is_none() && p.st.is_empty() && p.defaults.is_none());
}

// ---------------------------------------------------------------- the ONE change log (Order 036), fakes only

use crate::undo::{read_record, records, Kind as RKind, Outcome, Resettable, Review};

fn start() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
}

/// The page's Cx (`for_page("tgl")`: its records go under this page).
fn with_cx(p: &mut Tweaks, f: impl FnOnce(&mut Tweaks, &mut Cx)) {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st).for_page("tgl");
    f(p, &mut cx);
}

fn click_tgl(p: &mut Tweaks, k: Key) {
    with_cx(p, |p, cx| p.event(&Ev::Click(k), cx));
    p.settle();
}

fn review(p: &Tweaks, kind: RKind) -> Review {
    crate::services::with(|s| Review::for_page(kind, p, &s.store)).unwrap()
}

/// The review applied; the open page's read after it landed (Order 047).
fn apply(p: &mut Tweaks, rv: &Review) -> Vec<crate::undo::LineResult> {
    let r = crate::services::with(|s| rv.apply(&mut s.store, &mut [&mut *p as &mut dyn Resettable])).unwrap();
    p.settle();
    r
}

#[test]
fn a_switch_goes_into_the_change_log_with_its_old_value_and_back() {
    start();
    let mut p = page();
    let i = row_ix("ext").unwrap();
    // off, on, off: one entry, the value before the FIRST change
    click_tgl(&mut p, idx(K_TG, i));
    click_tgl(&mut p, idx(K_TG, i));
    click_tgl(&mut p, idx(K_TG, i));
    assert!(!p.on(i));
    let r = crate::services::with(|s| read_record(&s.store, "tgl", "ext")).flatten().expect("one entry");
    assert_eq!((r.was.raw.as_str(), r.now.raw.as_str(), r.label.as_str()), ("on", "off", "Show file extensions"));
    let mut rv = review(&p, RKind::HowItWas);
    assert_eq!(rv.title(), "Tweaks · back to how it was?");
    assert_eq!(rv.lines.len(), 1);
    assert_eq!(rv.lines[0].change_text(), "Off  →  On");
    // untick = kept
    rv.toggle(0);
    assert!(apply(&mut p, &rv).is_empty());
    assert!(!p.on(i), "an unticked line stays as it is");
    // ticked = back; the open page shows it
    rv.toggle(0);
    assert_eq!(apply(&mut p, &rv)[0].outcome, Outcome::Ok);
    assert!(p.on(i), "back to how the PC was");
    assert_eq!(lock_svc(&p).read("show_file_extensions").unwrap().value, Value::Switch(true));
    assert!(review(&p, RKind::HowItWas).is_empty(), "nothing left to reset");
    crate::services::shutdown();
}

#[test]
fn a_time_goes_back_to_its_seconds() {
    start();
    let mut p = page();
    let i = row_ix("scroff").unwrap();
    with_cx(&mut p, |p, cx| p.set_time(i, 0, cx));
    p.settle();
    let rv = review(&p, RKind::HowItWas);
    assert_eq!((rv.lines[0].item.as_str(), rv.lines[0].label.as_str()), ("scroff", "Screen off after"));
    assert_eq!(rv.lines[0].change_text(), "Never  →  10 min");
    assert_eq!(rv.lines[0].to.raw, "600,300", "plugged in AND on battery");
    assert_eq!(apply(&mut p, &rv)[0].outcome, Outcome::Ok);
    assert_eq!(p.value(i), Some(&Value::Timeout(Timeout::Seconds(600))));
    crate::services::shutdown();
}

#[test]
fn sleep_comes_back_with_its_own_time() {
    start();
    let mut p = page();
    let after = row_ix("sleepafter").unwrap();
    let sleep = row_ix("sleep").unwrap();
    with_cx(&mut p, |p, cx| p.set_time(after, 3600, cx));
    p.settle();
    click_tgl(&mut p, idx(K_TG, sleep));
    assert!(!p.on(sleep));
    let r = crate::services::with(|s| read_record(&s.store, "tgl", "sleep")).flatten().unwrap();
    assert_eq!((r.was.raw.as_str(), r.now.raw.as_str()), ("on", "off"));
    // Sleep off set "Sleep after" to never: that row is in the log too, with the time before the FIRST change
    let r = crate::services::with(|s| read_record(&s.store, "tgl", "sleepafter")).flatten().unwrap();
    assert_eq!((r.was.raw.as_str(), r.was.text.as_str(), r.now.raw.as_str()), ("1800,900", "30 min", "0,900"));
    let mut rv = review(&p, RKind::HowItWas);
    let lines: Vec<(&str, String)> = rv.lines.iter().map(|l| (l.item.as_str(), l.change_text())).collect();
    assert_eq!(lines, [("sleep", "Off  →  On".to_string()), ("sleepafter", "Never  →  30 min".to_string())]);
    // only the Sleep line: Sleep on again (with the 1 hour it had)
    rv.toggle(1);
    assert_eq!(apply(&mut p, &rv)[0].outcome, Outcome::Ok);
    assert!(p.on(sleep));
    assert_eq!(p.value(after), Some(&Value::Timeout(Timeout::Seconds(3600))));
    // then "Sleep after" back to its 30 min
    let rv = review(&p, RKind::HowItWas);
    assert_eq!(rv.lines.len(), 1);
    assert_eq!(rv.lines[0].change_text(), "1 hour  →  30 min");
    apply(&mut p, &rv);
    assert_eq!(p.value(after), Some(&Value::Timeout(Timeout::Seconds(1800))));
    assert!(review(&p, RKind::HowItWas).is_empty());
    crate::services::shutdown();
}

/// End review: a value already there is no failure ("Sleep after" = never while Sleep itself is back off), and a time
/// comes back with BOTH its plugged-in and on-battery seconds.
#[test]
fn a_time_already_there_is_ok_and_both_power_values_come_back() {
    start();
    let p = page();
    let mut g = lock_svc(&p);
    let s = &mut *g;
    let sleep_id = ROWS[row_ix("sleep").unwrap()].crate_id;
    s.set(sleep_id, false).unwrap();
    assert_eq!(put(s, "sleepafter", &Val::new("0,900", "Never")), Ok(()), "already never: nothing to do, not a failure");
    s.set(sleep_id, true).unwrap();
    assert_eq!(put(s, "scroff", &Val::new("1200,60", "20 min")), Ok(()));
    let v = s.power_values(ROWS[row_ix("scroff").unwrap()].crate_id).unwrap();
    assert_eq!((v.ac, v.dc), (1200, 60), "both values exactly as they were");
    crate::services::shutdown();
}

#[test]
fn a_games_flag_goes_into_the_change_log_and_back() {
    start();
    let mut p = page();
    click_tgl(&mut p, K_FSO);
    click_tgl(&mut p, K_FADD);
    let cs = p.games.iter().find(|g| g.exe.ends_with("cs2.exe")).unwrap().exe.clone();
    let item = format!("fso:{cs}");
    let rv = review(&p, RKind::HowItWas);
    assert_eq!(rv.lines.len(), 1);
    assert_eq!((rv.lines[0].item.as_str(), rv.lines[0].label.as_str()), (item.as_str(), "Fullscreen optimizations · Counter-Strike 2"));
    assert_eq!(rv.lines[0].change_text(), "Off  →  On");
    assert_eq!(apply(&mut p, &rv)[0].outcome, Outcome::Ok);
    assert!(!lock_svc(&p).fso_state(&cs).unwrap(), "the flag is gone again");
    assert!(review(&p, RKind::HowItWas).is_empty());
    // switched off, then removed by hand: back where it was, nothing to reset
    click_tgl(&mut p, K_FADD);
    let i = p.games.iter().position(|g| g.exe.ends_with("FortniteClient-Win64-Shipping.exe")).unwrap();
    click_tgl(&mut p, idx(K_FDEL, i));
    assert!(review(&p, RKind::HowItWas).is_empty());
    crate::services::shutdown();
}

#[test]
fn windows_defaults_come_from_the_table_and_are_logged_too() {
    start();
    let mut p = page();
    let mut rv = review(&p, RKind::WindowsDefaults);
    assert_eq!(rv.title(), "Tweaks · Windows defaults?");
    // the drawing's sample: file extensions shown (Windows: hidden); Game Mode is on as Windows has it
    let ext = rv.lines.iter().find(|l| l.item == "ext").expect("ext differs");
    assert_eq!((ext.label.as_str(), ext.change_text()), ("Show file extensions", "On  →  Off".to_string()));
    assert!(!rv.lines.iter().any(|l| l.item == "gmode"));
    assert!(!rv.lines.iter().any(|l| l.item == "copilot" || l.item == "bt"), "no factory value");
    for n in 0..rv.lines.len() {
        if rv.lines[n].item != "ext" {
            rv.toggle(n);
        }
    }
    assert_eq!(apply(&mut p, &rv)[0].outcome, Outcome::Ok);
    assert!(!p.on(row_ix("ext").unwrap()));
    assert!(!review(&p, RKind::WindowsDefaults).lines.iter().any(|l| l.item == "ext"));
    // the reset itself is a change: "how it was" brings the shown extensions back
    let back = review(&p, RKind::HowItWas);
    assert_eq!(back.lines.len(), 1);
    assert_eq!(back.lines[0].change_text(), "Off  →  On");
    crate::services::shutdown();
}

#[test]
fn a_closed_page_reads_and_puts_back_on_its_own_service() {
    start();
    // as if the app had switched the clock's seconds off once (the sample has them off)
    crate::services::with(|s| crate::undo::record(&mut s.store, "tgl", "secs", "Seconds on the clock", &Val::new("on", "On"), &Val::new("off", "Off")))
        .unwrap()
        .unwrap();
    let mut closed = Tweaks::default();
    let r = closed.resettable().unwrap();
    let rv = crate::services::with(|s| Review::for_page(RKind::HowItWas, &*r, &s.store)).unwrap();
    assert_eq!(rv.lines.len(), 1);
    assert_eq!(rv.lines[0].change_text(), "Off  →  On");
    let res = rv.apply_each(&mut [r], &mut |_| Ok(()));
    assert_eq!(res[0].outcome, Outcome::Ok);
    assert_eq!(closed.current("secs"), Some(Val::new("on", "On")), "put back on the fake");
    assert!(closed.svc.is_none(), "the closed page stays closed");
    assert!(!closed.windows_defaults().is_empty());
    crate::services::shutdown();
}

#[test]
fn what_changes_nothing_writes_nothing() {
    start();
    let mut p = page();
    // refused (needs admin): nothing changed, no entry
    click_tgl(&mut p, idx(K_TG, row_ix("hags").unwrap()));
    // Default apps: Windows' own window opens, the app changes nothing
    click_tgl(&mut p, idx(K_DPK, 1));
    // a quick fix (a one-time repair)
    click_tgl(&mut p, idx(qf::K_QF, 0));
    assert!(crate::services::with(|s| records(&s.store, Some("tgl"))).unwrap().is_empty());
    crate::services::shutdown();
}

#[test]
fn a_read_only_test_copy_refuses_the_reset() {
    let mut p = page();
    p.real_read = true;
    assert_eq!(Resettable::apply(&mut p, "ext", &Val::plain("off")), Err("A read-only test copy changes nothing".to_string()));
    assert!(p.on(row_ix("ext").unwrap()));
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

// ---------------------------------------------------------------- Order 047: the menu's thread never waits

/// The page on a SLOW fake (every worker job sleeps 300 ms first - a stand-in for an Explorer restart, a settings
/// broadcast, the shell opening Settings): its open must hand the menu's thread back within one frame.
fn slow_page() -> Tweaks {
    let mut p = Tweaks::default();
    p.slow = 300;
    let env = Env { test: true, ..Env::default() };
    crate::offui::assert_quick("Tweaks open", || p.open(&env, 0.0));
    p
}

/// A click that must not wait for its answer (the Cx is made outside the timed part).
fn quick_click(p: &mut Tweaks, k: Key, what: &str) {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st);
    crate::offui::assert_quick(what, || p.event(&Ev::Click(k), &mut cx));
}

/// The open's read (~50 rows, the games, the default apps) runs on the worker; the next open shows the last read at once.
#[test]
fn the_tab_opens_without_waiting_for_its_read() {
    let mut p = slow_page();
    assert!(p.st.is_empty() && !p.ready(), "the read is still on its way");
    p.settle();
    assert!(p.ready() && p.defaults.is_some() && p.games.len() == 1);
    let keep = p.keep.clone();
    p.close();
    let mut q = Tweaks::default();
    q.slow = 300;
    let env = Env { test: true, keep, ..Env::default() };
    crate::offui::assert_quick("Tweaks open again", || q.open(&env, 0.0));
    assert!(q.ready() && q.st.len() == ROWS.len() && q.defaults.is_some() && q.games.len() == 1, "the last read at once");
    q.settle();
    assert_eq!(q.games.len(), 1);
}

/// The switches that restart Explorer (Classic right-click menu, Recommendations in Start) and one that broadcasts a
/// settings change: the click returns at once, the switch flips when the worker's answer lands.
#[test]
fn a_slow_switch_never_holds_the_menu() {
    let mut p = slow_page();
    p.settle();
    for id in ["ctx", "recs", "secs"] {
        let i = row_ix(id).unwrap();
        let was = p.on(i);
        quick_click(&mut p, idx(K_TG, i), id);
        assert_eq!(p.on(i), was, "{id}: the switch flips when Windows has answered");
        p.settle();
        assert_eq!(p.on(i), !was, "{id}: switched and read back");
    }
}

/// A time picked in the list: set on the worker.
#[test]
fn a_time_never_holds_the_menu() {
    let mut p = slow_page();
    p.settle();
    let i = row_ix("scroff").unwrap();
    p.pressed = (400.0, 100.0, 128.0, 24.0);
    click(&mut p, idx(K_TO, i));
    build(&mut p);
    quick_click(&mut p, idx(K_TMENU, time_row(9)), "Screen off after › Never");
    p.settle();
    assert_eq!(p.value(i), Some(&Value::Timeout(Timeout::Never)));
    assert_eq!(p.toast.as_ref().unwrap().0, "The screen stays on");
}

/// Default apps' "Change" (Windows' "Open with") and the browser list's pick (Settings): opened from the worker.
#[test]
fn opening_windows_settings_never_holds_the_menu() {
    let mut p = slow_page();
    p.settle();
    quick_click(&mut p, idx(K_DPK, 1), "Default apps › Change");
    p.pressed = (480.0, 300.0, 60.0, 22.0);
    click(&mut p, idx(K_DPK, 0));
    build(&mut p);
    quick_click(&mut p, idx(K_BMENU, 1), "the browser list's pick");
    p.settle();
    let log = lock_svc(&p).fake().unwrap().log.clone();
    assert!(log.iter().any(|l| l == "open_with:.png"), "{log:?}");
    assert!(log.last().is_some_and(|l| l.starts_with("open_uri:ms-settings:defaultapps")), "{log:?}");
}

/// The games window: its list read and "Add game" on the worker.
#[test]
fn the_games_window_never_holds_the_menu() {
    let mut p = slow_page();
    p.settle();
    quick_click(&mut p, K_FSO, "Fullscreen optimizations › games");
    assert!(matches!(p.pop, Some(Pop::Games(_))));
    build(&mut p);
    quick_click(&mut p, K_FADD, "Add game");
    p.settle();
    assert_eq!(p.games.len(), 2);
    assert_eq!(p.toast.as_ref().unwrap().0, "Counter-Strike 2 added · off from its next start");
}

/// Nothing moving = no frames: `tick` is false and `wake_at` None; a toast at rest only asks to be woken when it has gone.
#[test]
fn idle_page_asks_for_no_frames() {
    let mut p = page();
    // the quick fixes' restore point line lands once (its own thread)
    std::thread::sleep(std::time::Duration::from_millis(100));
    let mut n = 0;
    while p.tick(5000.0) && n < 10 {
        n += 1;
    }
    assert!(!p.tick(5000.0), "nothing moves: no frames");
    assert_eq!(p.wake_at(5000.0), None);
    p.show_toast("hi", 6000.0);
    assert!(!p.tick(6100.0), "a toast at rest needs no frames");
    let w = p.wake_at(6100.0).expect("wakes when the toast has gone");
    assert!(w > 6100.0);
    assert!(!p.tick(w));
    assert!(p.toast.is_none() && p.wake_at(w).is_none());
}

/// Wait (10 s at most) for a worker's answer.
fn wait_for<T>(mut f: impl FnMut() -> Option<T>) -> T {
    let t0 = std::time::Instant::now();
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(t0.elapsed().as_secs() < 10, "the worker never answered");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Order 047: the reset review reads and puts back through the page's worker copy (`detach`, the page's ONE service):
/// with a slow copy (300 ms a call) its open and its Reset hand the menu's thread back within one frame, the row still
/// goes back, and the open page reads it again (`reset_done`).
#[test]
fn the_reset_review_never_holds_the_menu() {
    use crate::undo::{Applied, Opened};
    start();
    let mut p = page();
    let i = row_ix("ext").unwrap();
    click_tgl(&mut p, idx(K_TG, i));
    assert!(!p.on(i));
    p.slow = 300;
    let review = {
        let mut pages: [&mut dyn Resettable; 1] = [&mut p];
        let opened = crate::services::with(|s| crate::offui::assert_quick("the review opens", || Review::open(RKind::HowItWas, false, &mut pages, &s.store))).unwrap();
        let Opened::Reading(mut job) = opened else { panic!("read on a worker thread") };
        wait_for(|| job.take())
    };
    assert_eq!(review.lines.len(), 1);
    assert_eq!(review.lines[0].change_text(), "Off  →  On");
    let res = {
        let mut pages: [&mut dyn Resettable; 1] = [&mut p];
        let applied = crate::offui::assert_quick("Reset", || review.start_apply(&mut pages));
        let Applied::Running(mut job) = applied else { panic!("put back on a worker thread") };
        wait_for(|| job.take())
    };
    assert!(res.iter().all(|r| r.outcome == Outcome::Ok), "{res:?}");
    p.reset_done();
    p.settle();
    assert!(p.on(i), "back to how the PC was");
    crate::services::shutdown();
}
