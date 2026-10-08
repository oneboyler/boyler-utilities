//! Settings against FAKES only: the fake release server (in memory, never the internet), the fake install step (starts
//! nothing), the fake Start-with-Windows switch (never the registry). The real updater is only asked with NO repo, which
//! answers "not set up" without any network use.

use super::*;
use crate::gfx::Gfx;
use crate::ui::cx::State as CssState;
use crate::ui::lay::Laid;
use std::sync::Arc;

fn page(frozen: bool) -> Settings {
    let mut s = Settings::default();
    s.open(&Env { test: true, frozen, ..Env::default() }, 0.0);
    s
}

fn ev(s: &mut Settings, g: &Gfx, e: Ev, now: f64) {
    let mut st = CssState::default();
    let mut cx = Cx::new(now, false, g, &mut st).for_page("set");
    s.event(&e, &mut cx);
}

/// Run the page's clock forward (real time for the worker threads, page time `now` for the drawing's stage timings).
fn run(s: &mut Settings, until: impl Fn(&Settings) -> bool, mut now: f64) -> f64 {
    for _ in 0..6000 {
        s.tick(now);
        if until(s) {
            return now;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
        now += 10.0;
    }
    panic!("never got there: {}", s.describe());
}

#[test]
fn opening_never_asks_for_updates() {
    let s = page(false);
    assert!(s.describe().contains("about=NotChecked checking=false"), "{}", s.describe());
    assert!(!s.driver.as_ref().unwrap().busy);
}

#[test]
fn no_repo_means_updates_not_set_up_without_any_network() {
    // the REAL updater: with no repo it must answer before any request
    let mut s = Settings::default();
    s.env = Env { test: true, ..Env::default() };
    s.driver = Some(Driver::real_for("", "0.1.0"));
    s.check();
    run(&mut s, |s| !s.checking, 0.0);
    assert_eq!(s.ab, AbLine::NotSetUp);
    let g = Gfx::new(1.0);
    let mut st = CssState::default();
    let mut cx = Cx::new(0.0, false, &g, &mut st);
    let line = s.about_line(&mut cx);
    let El { content: crate::ui::el::Content::Text(t), .. } = line else { panic!("one text") };
    assert_eq!(t.s, format!("Version {VERSION} \u{b7} updates not set up yet"));
}

#[test]
fn updates_and_links_use_the_public_repo() {
    assert_eq!(update::REPO, "oneboyler/boyler-utilities");
    assert_eq!(update::page_url(true), "https://github.com/oneboyler/boyler-utilities/releases");
    assert_eq!(update::page_url(false), "https://github.com/oneboyler/boyler-utilities");
    assert_eq!(VERSION, "1.0.0");
}

#[test]
fn a_new_version_updates_itself_through_the_updating_window() {
    let g = Gfx::new(1.0);
    let mut s = page(false);
    s.driver = Some(Driver::fake("0.1.0", std::time::Duration::ZERO));
    ev(&mut s, &g, Ev::Click(K_CHECK), 0.0);
    assert!(s.checking);
    let now = run(&mut s, |s| s.upd.is_some(), 0.0);
    assert_eq!(s.ab, AbLine::Updating("0.2.0".into()));
    // the window is locked while it works
    s.popup_dismiss();
    let mut st = CssState::default();
    let mut cx = Cx::new(now, false, &g, &mut st);
    assert!(s.popup(&mut cx).is_some());
    // downloading -> installing (>= 1.6 s) -> restarting (1.3 s) -> the fake ends like the drawing
    let now = run(&mut s, |s| s.upd.as_ref().is_some_and(|u| u.ready), now);
    let plans = s.driver.as_ref().unwrap().installer.as_ref().unwrap().plans.lock().unwrap().len();
    assert_eq!(plans, 1, "the fake install step got the plan (and started nothing)");
    run(&mut s, |s| s.upd.is_none(), now);
    assert_eq!(s.version, "0.2.0");
    assert_eq!(s.ab, AbLine::UpToDate);
    assert!(s.describe().contains("toast=\"Updated to 0.2.0\""));
    assert!(!s.reqs.contains(&TempReq::ExitForUpdate), "a fake never asks the app to exit");
    // checking again: you have the newest
    ev(&mut s, &g, Ev::Click(K_CHECK), 1e6);
    run(&mut s, |s| !s.checking, 1e6);
    assert_eq!(s.ab, AbLine::Newest);
}

#[test]
fn cancel_stops_the_download_and_says_so() {
    let g = Gfx::new(1.0);
    let mut s = page(false);
    s.driver = Some(Driver::fake("0.1.0", std::time::Duration::from_millis(40)));
    ev(&mut s, &g, Ev::Click(K_CHECK), 0.0);
    let now = run(&mut s, |s| s.upd.as_ref().is_some_and(|u| u.done > 0), 0.0);
    ev(&mut s, &g, Ev::Click(K_UPD_NO), now);
    // the window closes at once; "cancelled" waits for update()'s own answer
    let mut st = CssState::default();
    let mut cx = Cx::new(now, false, &g, &mut st);
    assert!(s.popup(&mut cx).is_none());
    assert_eq!(s.ab, AbLine::Cancelling("0.2.0".into()));
    run(&mut s, |s| s.upd.is_none(), now);
    assert_eq!(s.ab, AbLine::Cancelled("0.2.0".into()));
    // the update thread has ended (Cancelled) and left nothing behind
    assert!(!s.driver.as_ref().unwrap().busy && !s.driver.as_ref().unwrap().is_spent());
    assert_eq!(s.driver.as_ref().unwrap().installer.as_ref().unwrap().plans.lock().unwrap().len(), 0);
}

/// REVIEW_024 HOLD 1: Cancel clicked after the point of no return - update() still returns Ok. The page follows the RESULT:
/// the window comes back ("Installing…" -> "Restarting…") and the app is asked to exit; never "cancelled".
#[test]
fn ok_after_cancel_still_finishes_the_update() {
    let g = Gfx::new(1.0);
    let mut s = page(false);
    s.driver = Some(Driver::fake("0.1.0", std::time::Duration::ZERO));
    ev(&mut s, &g, Ev::Click(K_CHECK), 0.0);
    let now = run(&mut s, |s| s.upd.is_some(), 0.0);
    // the worker finishes (staged) before the page has read a single progress message: the Cancel below comes too late
    for _ in 0..5000 {
        if s.driver.as_ref().unwrap().is_spent() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(s.driver.as_ref().unwrap().is_spent());
    assert_eq!(s.upd.as_ref().unwrap().stage, 0, "the page still shows Downloading");
    ev(&mut s, &g, Ev::Click(K_UPD_NO), now);
    assert_eq!(s.ab, AbLine::Cancelling("0.2.0".into()));
    // a copy that can restart (not the fake's ending): the app is asked to exit for the install step
    s.env.test = false;
    let now = run(&mut s, |s| s.reqs.contains(&TempReq::ExitForUpdate), now);
    assert_eq!(s.ab, AbLine::Updating("0.2.0".into()), "never 'cancelled'");
    let u = s.upd.clone().unwrap();
    assert!(u.ready && u.shown() && u.stage == 2, "{}", s.describe());
    // spent: nothing more is checked or updated in this app run
    ev(&mut s, &g, Ev::Click(K_CHECK), now + 10.0);
    assert!(!s.checking);
    assert!(!s.driver.as_mut().unwrap().check());
    assert_eq!(s.driver.as_ref().unwrap().installer.as_ref().unwrap().plans.lock().unwrap().len(), 1);
}

/// REVIEW_024 HOLD 1: leaving the tab during "Installing…" does not cancel; the result is acted on when the tab is back.
#[test]
fn closing_the_tab_while_installing_keeps_the_update() {
    let g = Gfx::new(1.0);
    let mut s = page(false);
    s.driver = Some(Driver::fake("0.1.0", std::time::Duration::ZERO));
    ev(&mut s, &g, Ev::Click(K_CHECK), 0.0);
    let now = run(&mut s, |s| s.upd.as_ref().is_some_and(|u| u.stage == 1), 0.0);
    s.close();
    let kept = |f: &dyn Fn(&Kept) -> bool| KEPT.with(|c| c.borrow().as_ref().is_some_and(f));
    assert!(kept(&|k| k.upd.as_ref().is_some_and(|u| u.shown())), "kept, not cancelled");
    // the worker ends while the tab is closed (nobody polls): the latch is set by the worker itself
    for _ in 0..5000 {
        if kept(&|k| k.driver.as_ref().is_some_and(|d| d.is_spent())) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(kept(&|k| k.driver.as_ref().is_some_and(|d| d.is_spent())));
    s.open(&Env { test: true, ..Env::default() }, now);
    let mut st = CssState::default();
    let mut cx = Cx::new(now, false, &g, &mut st);
    assert!(s.popup(&mut cx).is_some(), "the Updating window is back");
    // the fake ends like the drawing (a real copy asks to exit, see above)
    run(&mut s, |s| s.upd.is_none(), now);
    assert_eq!(s.ab, AbLine::UpToDate);
    assert_eq!(s.version, "0.2.0");
}

/// REVIEW_024 HOLD 1: leaving the tab while downloading cancels; Check while that cancelled update is still ending does
/// nothing (no stuck "Checking…"); the result comes in when the tab is back; after it, Check works again.
#[test]
fn check_waits_while_a_cancelled_update_is_still_ending() {
    let g = Gfx::new(1.0);
    let mut s = page(false);
    // slow fake: 200 ms per 256 KB chunk, so the cancel is seen at the next chunk
    s.driver = Some(Driver::fake("0.1.0", std::time::Duration::from_millis(200)));
    ev(&mut s, &g, Ev::Click(K_CHECK), 0.0);
    let now = run(&mut s, |s| s.upd.as_ref().is_some_and(|u| u.done > 0), 0.0);
    s.close();
    s.open(&Env { test: true, ..Env::default() }, now);
    assert_eq!(s.ab, AbLine::Cancelling("0.2.0".into()));
    ev(&mut s, &g, Ev::Click(K_CHECK), now);
    assert!(!s.checking && s.ab == AbLine::Cancelling("0.2.0".into()), "{}", s.describe());
    let now = run(&mut s, |s| s.upd.is_none(), now);
    assert_eq!(s.ab, AbLine::Cancelled("0.2.0".into()));
    ev(&mut s, &g, Ev::Click(K_CHECK), now);
    assert!(s.checking, "Check works again once the update has ended");
    run(&mut s, |s| s.upd.is_some(), now);
    s.cancel_update();
    run(&mut s, |s| s.upd.is_none(), now);
}

#[test]
fn glass_theme_and_start_with_windows() {
    let g = Gfx::new(1.0);
    let mut s = page(false);
    assert!(s.describe().starts_with("start=false"));
    ev(&mut s, &g, Ev::Click(K_START), 1.0);
    assert!(s.describe().starts_with("start=true"), "the fake switch, never the registry");
    ev(&mut s, &g, Ev::Click(idx(K_GLASS, 1)), 2.0);

    assert_eq!(s.glass, GlassStyle::Frosted);
    assert_eq!(s.reqs, vec![TempReq::Glass(GlassStyle::Frosted)]);
    // the app's own choices and the ONE updater survive leaving the tab AND a menu close (the page object is dropped)
    let inst = s.driver.as_ref().unwrap().installer.clone().unwrap();
    s.close();
    assert!(s.reqs.is_empty());
    s.open(&Env { test: true, ..Env::default() }, 4.0);
    assert_eq!(s.glass, GlassStyle::Frosted);
    assert!(Arc::ptr_eq(&inst, s.driver.as_ref().unwrap().installer.as_ref().unwrap()), "re-opening keeps the same updater");
    drop(s);
    let s = page(false);
    assert_eq!(s.glass, GlassStyle::Frosted, "kept over a menu close");
    assert!(Arc::ptr_eq(&inst, s.driver.as_ref().unwrap().installer.as_ref().unwrap()));
}

#[test]
fn reset_the_apps_own_settings_asks_first() {
    let g = Gfx::new(1.0);
    let mut s = page(false);
    ev(&mut s, &g, Ev::Click(idx(K_GLASS, 2)), 1.0);
    ev(&mut s, &g, Ev::Press(K_RS_APP, 530.0, 760.0, (500.66, 746.39, 61.34, 26.0)), 2.0);
    ev(&mut s, &g, Ev::Click(K_RS_APP), 2.0);
    assert!(s.describe().contains("pop=ask"));
    ev(&mut s, &g, Ev::Click(k_ask_no()), 3.0);
    assert_eq!(s.glass, GlassStyle::WindowsLook);
    ev(&mut s, &g, Ev::Click(K_RS_APP), 4.0);
    ev(&mut s, &g, Ev::Click(k_ask_go()), 5.0);
    assert_eq!(s.glass, GlassStyle::Liquid);
    assert!(s.reqs.contains(&TempReq::ResetApp));
}

#[test]
fn the_review_lists_every_tab_and_counts_the_ticked_lines() {
    // (test pictures only: the drawing's sample log; every other copy opens the frame's review over the change log)
    let g = Gfx::new(1.0);
    let mut s = page(true);
    ev(&mut s, &g, Ev::Click(K_RS_WAS), 1.0);
    assert!(s.describe().contains("pop=review:was:21"), "{}", s.describe());
    ev(&mut s, &g, Ev::Click(idx(K_RV, 0)), 2.0);
    assert!(s.describe().contains("pop=review:was:20"));
    ev(&mut s, &g, Ev::Click(K_RV_GO), 3.0);
    assert!(s.describe().contains("toast=\"Back to how it was \u{b7} 20 settings reset\""), "{}", s.describe());
}

/// Order 036: Start with Windows writes ONE entry (the value before the first change), the reset puts it back, an unticked
/// line is kept. The fake switch only - never the registry.
#[test]
fn start_with_windows_goes_into_the_change_log_and_back() {
    use crate::undo::{read_record, Kind, Resettable, Review};
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    let g = Gfx::new(1.0);
    let mut s = page(false);
    assert!(s.describe().starts_with("start=false"));
    ev(&mut s, &g, Ev::Click(K_START), 1.0);
    ev(&mut s, &g, Ev::Click(K_START), 2.0);
    ev(&mut s, &g, Ev::Click(K_START), 3.0);
    assert!(s.describe().starts_with("start=true"));
    let r = crate::services::with(|x| read_record(&x.store, "set", "autostart")).flatten().expect("one entry");
    assert_eq!((r.was.raw.as_str(), r.now.raw.as_str(), r.label.as_str()), ("off", "on", "Start with Windows"), "the value before the FIRST change");
    // untick = kept
    let mut rv = crate::services::with(|x| Review::for_page(Kind::HowItWas, &s, &x.store)).unwrap();
    assert_eq!(rv.lines.len(), 1);
    assert_eq!(rv.lines[0].change_text(), "On  →  Off");
    rv.toggle(0);
    let res = rv.apply_each(&mut [&mut s as &mut dyn Resettable], &mut |_| Ok(()));
    assert!(res.is_empty());
    assert!(s.describe().starts_with("start=true"), "an unticked line stays as it is");
    // ticked = back
    rv.toggle(0);
    let res = crate::services::with(|x| rv.apply(&mut x.store, &mut [&mut s as &mut dyn Resettable])).unwrap();
    assert_eq!(res[0].outcome, crate::undo::Outcome::Ok);
    assert!(s.describe().starts_with("start=false"), "back to how the PC was");
    assert!(crate::services::with(|x| Review::for_page(Kind::HowItWas, &s, &x.store)).unwrap().is_empty(), "nothing left to reset");
    assert!(crate::services::with(|x| Review::for_page(Kind::WindowsDefaults, &s, &x.store)).unwrap().is_empty(), "Windows has no default for it");
    crate::services::shutdown();
}

#[test]
fn shortcut_rows_open_their_feature() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    crate::services::with(|s| {
        s.add_action(crate::keys::Action::new("shot.key", "Screenshot", "shot"), |_| {});
        s.add_action(crate::keys::Action::new("srch.open", "Search", "srch"), |_| {});
    });
    let g = Gfx::new(1.0);
    let mut s = page(false);
    // the keys manager's actions are the rows (Search's own row holds its field); a row opens its tab
    let n = {
        let mut st = CssState::default();
        let cx = Cx::new(0.0, false, &g, &mut st);
        let list = s.shortcuts(&cx);
        assert!(list.iter().all(|r| r.tab != "srch"), "Search has its own row");
        list.iter().position(|r| r.id == "shot.key").expect("the Screenshot key is listed")
    };
    ev(&mut s, &g, Ev::Click(idx(K_SC, n)), 1.0);
    assert_eq!(s.reqs, vec![TempReq::ShowTab("shot".into(), "Screenshot".into())]);
    // Search's field listens on the keys manager
    ev(&mut s, &g, Ev::Click(K_SRCH), 2.0);
    assert_eq!(crate::services::with(|x| x.listening.as_ref().map(|l| l.action.clone())).unwrap().as_deref(), Some("srch.open"));
    assert!(crate::services::key_message(true, 0x77, 0)); // F8
    assert_eq!(crate::services::with(|x| x.field("srch.open").0).unwrap().as_deref(), Some("F8"));
    crate::services::shutdown();
}

/// The page's boxes land where Chromium lays out the drawing (menu-v22 `set`, tools/ref/dom_dump.js, page coordinates =
/// window - 56): App gh 54 / grp 75.84 (131.39 tall), All shortcuts grp 249.08 (294), Reset grp 584.92 (142.17),
/// About grp 768.94 (60), the links row 836.94.
#[test]
fn the_boxes_match_the_drawing() {
    let g = Gfx::new(1.0);
    let mut s = page(true);
    let mut st = CssState::default();
    let mut cx = Cx::new(0.0, false, &g, &mut st);
    let kids = s.build(&mut cx);
    let root = El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
    let laid = Laid::new(&g, root, 600.0, None);
    let top: Vec<(f32, f32)> = laid.nodes.iter().filter(|n| n.parent == Some(0)).map(|n| (n.rect.1, n.rect.3)).collect();
    let near = |a: f32, b: f32| (a - b).abs() < 0.02;
    // Order 033: the Theme row is back (A_027_01 hid it until the light theme existed) - every box where the drawing has it
    const T: f32 = 0.0;
    let want = [(54.0, 14.84), (75.84, 131.39 - T), (227.23 - T, 14.84), (249.08 - T, 294.0), (563.08 - T, 14.84), (584.92 - T, 142.17), (747.09 - T, 14.84), (768.94 - T, 60.0), (836.94 - T, 16.19)];
    for (i, w) in want.iter().enumerate() {
        let got = top[i + 1];
        assert!(near(got.0, w.0) && near(got.1, w.1), "box {i}: got {got:?} want {w:?} (all {top:?})");
    }
}

/// The Updating window where Chromium puts it (menu-v22, frozen sample): `.dlg.mdlg.upddlg` at (130, 160.7344) 340 x 198.5156,
/// its bar at y 269.7344, the status line at 283.7344, Cancel at (376, 313.25) 76 x 30.
#[test]
fn the_updating_window_matches_the_drawing() {
    let g = Gfx::new(1.0);
    let mut s = page(true);
    s.upd = Some(UpdUi { ver: "0.2.0".into(), opened_at: -1e6, stage: 0, stage_at: 0.0, done: 1, total: update::FAKE_SIZE, dl_from: 0.0, ready: false, cancel_asked: false });
    let mut st = CssState::default();
    let mut cx = Cx::new(0.0, false, &g, &mut st);
    let p = s.popup(&mut cx).expect("the window");
    let laid = Laid::new(&g, El::block().w(600.0).h(520.0).child(p), 600.0, Some(520.0));
    let r = |w: f32, h: f32| laid.nodes.iter().find(|n| (n.rect.2 - w).abs() < 0.02 && (n.rect.3 - h).abs() < 0.6).map(|n| n.rect);
    let dlg = laid.nodes.iter().find(|n| (n.rect.2 - 340.0).abs() < 0.01).map(|n| n.rect).expect("340 wide");
    // 198.5156 with the shared dialog piece of fix-014 588fb11 (`.mdb{padding-bottom:2px}`), 196.5156 with 708d4dc's piece
    assert!((dlg.3 - 198.5156).abs() < 0.02 || (dlg.3 - 196.5156).abs() < 0.02, "dialog {dlg:?}");
    assert!(((dlg.1 + dlg.3 / 2.0) - 260.0).abs() < 0.02, "centred {dlg:?}");
    // this page's own content, from the window's top: the bar 109, Cancel 152.5156 (x 376)
    let bar = r(304.0, 6.0).expect("bar");
    assert!((bar.1 - dlg.1 - 109.0).abs() < 0.02, "bar {bar:?}");
    let cancel = r(76.0, 30.0).expect("cancel");
    assert!((cancel.0 - 376.0).abs() < 0.02 && (cancel.1 - dlg.1 - 152.5156).abs() < 0.02, "cancel {cancel:?}");
}

/// REVIEW_024 9483f05 HOLD: a menu close drops the page WITHOUT `close()` and the next open makes a new one. The ONE updater,
/// its running update and its window must survive that: a download is cancelled (its answer read on the next show), an
/// update past the download goes on; the next page has the same updater, which refuses a 2nd check while busy and after Ok.
#[test]
fn closing_the_menu_keeps_the_one_updater_and_its_update() {
    let g = Gfx::new(1.0);
    let reopen = || page(false);
    // 1) the menu closes mid-download
    let mut s = page(false);
    s.driver = Some(Driver::fake("0.1.0", std::time::Duration::from_millis(200)));
    let inst = s.driver.as_ref().unwrap().installer.clone().unwrap();
    ev(&mut s, &g, Ev::Click(K_CHECK), 0.0);
    let now = run(&mut s, |s| s.upd.as_ref().is_some_and(|u| u.done > 0), 0.0);
    drop(s);
    let mut s = reopen();
    assert!(Arc::ptr_eq(&inst, s.driver.as_ref().unwrap().installer.as_ref().unwrap()), "the same updater");
    assert_eq!(s.ab, AbLine::Cancelling("0.2.0".into()), "{}", s.describe());
    assert!(!s.driver.as_mut().unwrap().check(), "no 2nd check while the cancelled update still ends");
    let now = run(&mut s, |s| s.upd.is_none(), now);
    assert_eq!(s.ab, AbLine::Cancelled("0.2.0".into()));
    // 2) the menu closes during "Installing…": nothing is cancelled, the Ok is acted on in the next menu
    s.driver = Some(Driver::fake("0.1.0", std::time::Duration::ZERO));
    let inst = s.driver.as_ref().unwrap().installer.clone().unwrap();
    ev(&mut s, &g, Ev::Click(K_CHECK), now);
    let now = run(&mut s, |s| s.upd.as_ref().is_some_and(|u| u.stage == 1), now);
    drop(s);
    for _ in 0..5000 {
        if KEPT.with(|c| c.borrow().as_ref().and_then(|k| k.driver.as_ref()).is_some_and(|d| d.is_spent())) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let mut s = reopen();
    assert!(Arc::ptr_eq(&inst, s.driver.as_ref().unwrap().installer.as_ref().unwrap()), "the same updater");
    assert!(s.driver.as_ref().unwrap().is_spent());
    assert!(!s.driver.as_mut().unwrap().check(), "spent: no check after the Ok");
    let mut st = CssState::default();
    let mut cx = Cx::new(now, false, &g, &mut st);
    assert!(s.popup(&mut cx).is_some(), "the Updating window is back");
    run(&mut s, |s| s.upd.is_none(), now);
    assert_eq!(s.ab, AbLine::UpToDate);
    assert_eq!(inst.plans.lock().unwrap().len(), 1, "one install step, not two");
    // 3) a page the menu made but never showed stashes nothing (the kept state is not overwritten)
    s.close();
    drop(Settings::default());
    let s = reopen();
    assert_eq!(s.version, "0.2.0", "the kept state survived an unshown page's drop");
}

// ---------------------------------------------------------------- Order 040: About › Licences

/// The generated file reads, and every part has a licence text with words in it.
#[test]
fn the_licences_file_parses_and_no_text_is_empty() {
    let d = licences::parse(licences::SRC).expect("licences.txt");
    assert!(d.parts.len() >= 50, "{} parts", d.parts.len());
    for p in &d.parts {
        assert!(!p.name.is_empty() && !p.version.is_empty() && !p.declared.is_empty() && !p.group.is_empty(), "{p:?}");
        assert!(!p.texts.is_empty(), "{} has no text", p.name);
        for t in &p.texts {
            let words: usize = d.texts[*t].lines.iter().map(|l| l.split_whitespace().count()).sum();
            assert!(words >= 20, "{}: text {} has {} words", p.name, d.texts[*t].id, words);
        }
    }
    // every text once, and used
    let mut ids: Vec<&str> = d.texts.iter().map(|t| t.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), d.texts.len(), "a text id twice");
    for (i, t) in d.texts.iter().enumerate() {
        assert!(d.parts.iter().any(|p| p.texts.contains(&i)), "text {} unused", t.id);
    }
    // a broken file is refused, never shown half
    assert!(licences::parse("P\tg\tx\t1\tMIT\tnope\n").is_err());
    assert!(licences::parse("T\tMIT\tMIT License\n|\n|  \n").is_err());
    assert!(licences::parse("C\tMIT\tCopyright\n").is_err());
    // a copyright line for a text the part does not use
    let words = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty";
    let ok = format!("P\tg\tx\t1\tMIT\tMIT\nC\tMIT\tCopyright 1\nT\tMIT\tMIT License\n|{words}\n").leak();
    assert!(licences::parse(ok).is_ok());
    let bad = format!("P\tg\tx\t1\tMIT\tMIT\nC\tZlib\tCopyright 1\nT\tMIT\tMIT License\nT\tZlib\tzlib\n|{words}\n").leak();
    assert!(licences::parse(bad).is_err());
    assert!(licences::parse("X\n").is_err());
}

/// The parts that are not crates: Skia and what is built into it, the Rust standard library, the setup's Inno Setup and
/// Everything, the Raw Accel add-on.
#[test]
fn the_licences_name_every_non_crate_part() {
    let d = licences::parse(licences::SRC).unwrap();
    for n in ["Skia", "zlib (Chromium)", "libpng", "libjpeg-turbo", "HarfBuzz", "ICU", "Wuffs", "Expat", "Rust standard library", "Inno Setup", "Everything (voidtools)", "Raw Accel"] {
        assert!(d.parts.iter().any(|p| p.name == n), "{n} missing");
    }
    let skia = d.parts.iter().find(|p| p.name == "Skia").unwrap();
    assert!(skia.copyright.iter().any(|c| c.1.contains("Google") && c.0 == skia.texts[0]), "{skia:?}");
}

/// Every crate listed is one Cargo.lock builds (at that version), and every crate a Cargo.toml of the app or of the
/// workspace crates it links depends on is listed. (The whole list is `cargo tree -p bu-app -e normal`, the release build's
/// own graph: `tools/licences/gen.py --check` fails when the file is stale.)
#[test]
fn every_dependency_has_its_licence() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).expect("Cargo.lock");
    let mut locked: Vec<(String, String)> = Vec::new();
    let mut name = None;
    for l in lock.lines() {
        if let Some(n) = l.strip_prefix("name = ") {
            name = Some(n.trim_matches('"').to_string());
        } else if let (Some(v), Some(n)) = (l.strip_prefix("version = "), name.take()) {
            locked.push((n, v.trim_matches('"').to_string()));
        }
    }
    let d = licences::parse(licences::SRC).unwrap();
    let crates: Vec<&licences::Part> = d.parts.iter().filter(|p| p.group.starts_with("Rust") && p.name != "Rust standard library").collect();
    assert!(crates.len() >= 40, "{} crates", crates.len());
    for p in &crates {
        assert!(locked.iter().any(|(n, v)| n == p.name && v == p.version), "{} {} is not in Cargo.lock", p.name, p.version);
    }
    let mut tomls = vec![root.join("app").join("Cargo.toml")];
    for e in std::fs::read_dir(root.join("crates")).unwrap().flatten() {
        if e.path().join("Cargo.toml").exists() {
            tomls.push(e.path().join("Cargo.toml"));
        }
    }
    let normal = |h: &str| h.ends_with("dependencies") && !h.ends_with("dev-dependencies") && !h.ends_with("build-dependencies");
    let mut deps: Vec<String> = Vec::new();
    for t in tomls {
        let text = std::fs::read_to_string(&t).unwrap();
        let mut in_deps = false;
        for l in text.lines() {
            let l = l.trim();
            if l.starts_with('[') {
                let h = l.trim_matches(|c| c == '[' || c == ']');
                in_deps = normal(h);
                // `[dependencies.windows]` / `[target.'cfg(windows)'.dependencies.windows]`
                if let Some((before, n)) = h.rsplit_once('.') {
                    if normal(before) {
                        deps.push(n.to_string());
                    }
                }
                continue;
            }
            if in_deps && !l.starts_with('#') && !l.contains("path =") && !l.contains("optional = true") {
                if let Some((n, _)) = l.split_once('=') {
                    let n = n.trim();
                    if !n.is_empty() && !n.contains(' ') {
                        deps.push(n.to_string());
                    }
                }
            }
        }
    }
    deps.sort();
    deps.dedup();
    assert!(deps.iter().any(|d| d == "skia-safe") && deps.iter().any(|d| d == "windows") && deps.iter().any(|d| d == "taffy"), "{deps:?}");
    for dep in &deps {
        assert!(crates.iter().any(|p| p.name == dep), "{dep} (a dependency) has no licence entry");
    }
}

/// A line wider than the box breaks at a space and keeps its indent; nothing is lost; every real text fits the box.
#[test]
fn licence_lines_wrap_at_spaces_and_keep_every_word() {
    let lines = ["short", "   1. Definitions of words that go on and on beyond the edge of the box here", "", "averyveryverylongwordwithoutanyspaceatallinsideitthatkeepsgoing"];
    let w = licences::wrap(&lines, 24);
    assert!(w.iter().all(|l| l.chars().count() <= 24), "{w:#?}");
    assert_eq!(w[0], "short");
    assert!(w[1].starts_with("   1. Definitions"), "{w:#?}");
    assert!(w[2].starts_with("   ") && !w[2].starts_with("    "), "the indent is kept: {w:#?}");
    let words = |v: &[&str]| v.iter().flat_map(|l| l.split_whitespace()).collect::<String>();
    let ws: Vec<&str> = w.iter().map(|s| s.as_str()).collect();
    assert_eq!(words(&ws), words(&lines));
    let g = Gfx::new(1.0);
    let cols = licences::cols(&g);
    assert!(cols >= 70, "{cols} columns");
    let d = licences::parse(licences::SRC).unwrap();
    for t in &d.texts {
        for l in licences::wrap(&t.lines, cols) {
            assert!(g.text_width(&l, licences::MONO) <= licences::TEXT_W + 0.5, "{}: too wide: {l}", t.id);
        }
        let ws: Vec<String> = licences::wrap(&t.lines, cols);
        let ws: Vec<&str> = ws.iter().map(|s| s.as_str()).collect();
        assert_eq!(words(&ws), words(&t.lines), "{}: words lost", t.id);
    }
}

/// About › Licences opens the list in the tab; a row opens its part, Esc / the back arrow go back one level, then to the
/// settings; leaving the tab forgets it.
#[test]
fn licences_open_a_part_and_go_back() {
    let g = Gfx::new(1.0);
    let mut s = page(false);
    assert!(s.describe().ends_with("lic=-"), "{}", s.describe());
    ev(&mut s, &g, Ev::Click(K_LIC), 0.0);
    assert!(s.describe().contains("lic=list:"), "{}", s.describe());
    // the list: header, line, a group header + box per group
    let mut st = CssState::default();
    let mut cx = Cx::new(0.0, false, &g, &mut st);
    assert_eq!(s.build(&mut cx).len(), 2 + 2 * 4);
    let d = licences::parse(licences::SRC).unwrap();
    let icu = d.parts.iter().position(|p| p.name == "ICU").unwrap();
    ev(&mut s, &g, Ev::Click(idx(licences::K_ROW, icu)), 0.0);
    assert!(s.describe().ends_with("lic=ICU"), "{}", s.describe());
    // the part: header, line, the text's group header + box, which is as tall as the text's lines
    let mut st = CssState::default();
    let mut cx = Cx::new(0.0, false, &g, &mut st);
    let kids = s.build(&mut cx);
    assert_eq!(kids.len(), 4);
    let laid = Laid::new(&g, El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids), 600.0, None);
    let h = laid.nodes[0].rect.3;
    let n = d.texts[d.parts[icu].texts[0]].lines.len() as f32;
    assert!(h > n * 16.5 && h < n * 16.5 * 1.5, "page {h} px for {n} lines");
    // Esc: back to the list (the menu stays open); the back arrow: back to the settings
    let mut st = CssState::default();
    let mut cx = Cx::new(0.0, false, &g, &mut st).for_page("set");
    s.event(&Ev::Key(crate::ui::cx::PAGE, VK_ESCAPE), &mut cx);
    assert!(cx.used, "Esc is used");
    assert!(s.describe().contains("lic=list:"), "{}", s.describe());
    ev(&mut s, &g, Ev::Click(licences::K_BACK), 0.0);
    assert!(s.describe().ends_with("lic=-"), "{}", s.describe());
    // leaving the tab drops it
    ev(&mut s, &g, Ev::Click(K_LIC), 0.0);
    s.close();
    let s = page(false);
    assert!(s.describe().ends_with("lic=-"), "{}", s.describe());
}

/// The proof pictures of About › Licences (Order 040), made with NO window and no running app: the real frame (`Ui`, every
/// page on its fakes - a test copy's switch is set first) painted the way `Menu::snapshot_over` paints the comparison
/// picture - the desktop picture, the glass backdrop recipe, the window's tint, caption buttons, page, edge and top row -
/// for dark and light. Writes <dir>\lic_<theme>_<step>.png (1920 x 1080 with the menu at its place) + _win crops.
/// `BU_LIC_SHOTS=<dir> BU_LIC_DESK=<desk.png> cargo test -p bu-app licences_png -- --ignored`
#[test]
#[ignore]
fn licences_png() {
    use crate::gfx::CssColor;
    use crate::ui::{Frame, Ui, PAGE_TOP, RADIUS, WIN_H, WIN_W};
    let dir = std::env::var("BU_LIC_SHOTS").expect("set BU_LIC_SHOTS=<folder>");
    let desk = std::env::var("BU_LIC_DESK").expect("set BU_LIC_DESK=<desk.png>");
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
    }
    // a test copy: every service fake (Audio, the first tab, opens with the menu)
    crate::testmode::set(true, false);
    let dp = crate::png::load_png(&desk).expect("desk");
    let dimg = crate::png::to_image(&dp).expect("desk image");
    let (sw, sh) = (dp.w as i32, dp.h as i32);
    let (wx, wy) = (sw as f32 - 12.0 - WIN_W, sh as f32 - 48.0 - 12.0 - WIN_H);
    let g = Gfx::new(1.0);
    let icons = crate::icons::Icons::new();
    let snap = |ui: &mut Ui, now: f64, name: &str| {
        let f = Frame { g: &g, icons: &icons, now };
        // the page layer alone (what the top row's frosted parts read)
        let mut page = crate::gfx::new_surface(WIN_W as i32, WIN_H as i32).unwrap();
        g.begin(page.canvas());
        ui.draw_pages(&f);
        g.end();
        let base = page.image_snapshot();
        let mut out = crate::gfx::new_surface(sw, sh).unwrap();
        out.canvas().draw_image(&dimg, (0, 0), None);
        g.begin(out.canvas());
        let gn = crate::ui::glass_numbers();
        g.backdrop(&dimg, wx, wy, WIN_W, WIN_H, RADIUS, gn.blur_px, &[CssColor::Saturate(gn.saturate), CssColor::Brightness(gn.brightness)]);
        g.end();
        let mut win = crate::gfx::new_surface(sw, sh).unwrap();
        g.begin(win.canvas());
        g.cv().translate((wx, wy));
        ui.draw_rim(&g);
        ui.draw_caps(&f);
        ui.draw_pages(&f);
        ui.draw_edge(&g);
        // (Order 041: the top row, then its names / chevrons and the page's scrollbar, which frost the page)
        ui.draw_dock(&f);
        ui.draw_dock_overlays(&f, &base);
        ui.draw_page_scrollbar(&f, &base);
        g.end();
        out.canvas().draw_image(win.image_snapshot(), (0, 0), None);
        ui.update_tips(&g, now);
        if ui.popup_open() {
            let b = out.image_snapshot();
            g.begin(out.canvas());
            g.cv().translate((wx, wy));
            ui.draw_popup(&f, &b);
            g.end();
        }
        let full = crate::png::from_surface(&mut out);
        crate::png::save_png(&full, &format!("{dir}\\{name}.png")).expect("save");
        let crop = out.image_snapshot_with_bounds(skia_safe::IRect::from_xywh(wx as i32 - 8, wy as i32 - 8, WIN_W as i32 + 16, WIN_H as i32 + 16)).unwrap();
        let mut cs = crate::gfx::new_surface(crop.width(), crop.height()).unwrap();
        cs.canvas().draw_image(&crop, (0, 0), None);
        crate::png::save_png(&crate::png::from_surface(&mut cs), &format!("{dir}\\{name}_win.png")).expect("save");
    };
    let _ = PAGE_TOP;
    for light in [false, true] {
        crate::ui::set_light(light);
        let theme = if light { "light" } else { "dark" };
        let mut now = 5000.0;
        let mut ui = Ui::new(false, true, 0.0);
        let step = |ui: &mut Ui, now: &mut f64| {
            for _ in 0..40 {
                *now += 50.0;
                ui.update(*now);
                let f = Frame { g: &g, icons: &icons, now: *now };
                let mut s = crate::gfx::new_surface(WIN_W as i32, WIN_H as i32).unwrap();
                g.begin(s.canvas());
                ui.draw_pages(&f);
                g.end();
            }
            ui.mouse_leave(*now);
            ui.update(*now);
        };
        step(&mut ui, &mut now);
        let set = crate::ui::tab_index("set").unwrap();
        ui.show_tab(set, now);
        step(&mut ui, &mut now);
        // the About section (the Licences link stays where it was)
        ui.scroll_now(2000.0, now);
        step(&mut ui, &mut now);
        snap(&mut ui, now, &format!("lic_{theme}_0_about"));
        // About › Licences: the list, its top and further down
        ui.click("el:set.lic", now);
        step(&mut ui, &mut now);
        snap(&mut ui, now, &format!("lic_{theme}_1_list"));
        ui.scroll_now(1500.0, now);
        step(&mut ui, &mut now);
        snap(&mut ui, now, &format!("lic_{theme}_2_list_down"));
        ui.scroll_now(100_000.0, now);
        step(&mut ui, &mut now);
        snap(&mut ui, now, &format!("lic_{theme}_2b_list_end"));
        ui.scroll_now(1500.0, now);
        step(&mut ui, &mut now);
        // a row hovered (the pointer resting on it)
        if let Some((x, y)) = ui.target_point("el:set.lic.row/32") {
            ui.mouse_move(x, y, now);
            step_hover(&mut ui, &mut now, &g, &icons, x, y);
            snap(&mut ui, now, &format!("lic_{theme}_3_list_hover"));
            ui.mouse_leave(now);
        }
        // one crate (MIT with its copyright line), one long text (ICU) at its top and in the middle
        ui.scroll_now(0.0, now);
        step(&mut ui, &mut now);
        let d = licences::parse(licences::SRC).unwrap();
        for (name, scroll, tag) in [("serde_json", 0.0, "4_serde_json"), ("Skia", 0.0, "5_skia"), ("ICU", 0.0, "6_icu"), ("ICU", 4000.0, "7_icu_down"), ("libjpeg-turbo", 0.0, "8_libjpeg"), ("Rust standard library", 0.0, "9_std"), ("Everything (voidtools)", 0.0, "9b_everything")] {
            let i = d.parts.iter().position(|p| p.name == name).unwrap();
            ui.click(&format!("el:set.lic.row/{i}"), now);
            step(&mut ui, &mut now);
            if scroll > 0.0 {
                ui.scroll_now(scroll, now);
                step(&mut ui, &mut now);
            }
            snap(&mut ui, now, &format!("lic_{theme}_{tag}"));
            if tag == "7_icu_down" {
                // what one frame of a scroll costs on the longest text: the page built + laid out + painted (the frame
                // builds the page again when it scrolls), 60 frames over 3000 px
                let mut s = crate::gfx::new_surface(WIN_W as i32, WIN_H as i32).unwrap();
                let t0 = std::time::Instant::now();
                for k in 0..60 {
                    ui.scroll_now(1000.0 + k as f32 * 50.0, now);
                    ui.update(now);
                    let f = Frame { g: &g, icons: &icons, now };
                    g.begin(s.canvas());
                    ui.draw_pages(&f);
                    g.end();
                }
                let ms = t0.elapsed().as_secs_f64() * 1000.0 / 60.0;
                let build = if cfg!(debug_assertions) { "debug" } else { "release" };
                let _ = std::fs::write(format!("{dir}/lic_{theme}_scroll_ms.txt"), format!("{ms:.3} ms per frame (ICU, build + layout + paint, {build} build)"));
            }
            // back to the list (Esc)
            let _ = ui.key(0x1B, now);
            step(&mut ui, &mut now);
            ui.scroll_now(0.0, now);
            step(&mut ui, &mut now);
        }
    }
}

/// The pointer resting at (x, y) while the frame runs (hover transitions end).
fn step_hover(ui: &mut crate::ui::Ui, now: &mut f64, g: &Gfx, icons: &crate::icons::Icons, x: f32, y: f32) {
    for _ in 0..20 {
        *now += 50.0;
        ui.mouse_move(x, y, *now);
        ui.update(*now);
        let f = crate::ui::Frame { g, icons, now: *now };
        let mut s = crate::gfx::new_surface(crate::ui::WIN_W as i32, crate::ui::WIN_H as i32).unwrap();
        g.begin(s.canvas());
        ui.draw_pages(&f);
        g.end();
    }
}

/// The Updating window stays usable while Licences shows: its Cancel still reaches the update.
#[test]
fn the_updating_window_still_cancels_over_licences() {
    let g = Gfx::new(1.0);
    let mut s = page(false);
    s.driver = Some(Driver::fake("0.1.0", std::time::Duration::from_millis(200)));
    ev(&mut s, &g, Ev::Click(K_CHECK), 0.0);
    let now = run(&mut s, |s| s.upd.as_ref().is_some_and(|u| u.done > 0), 0.0);
    ev(&mut s, &g, Ev::Click(K_LIC), now);
    assert!(s.describe().contains("lic=list:"), "{}", s.describe());
    ev(&mut s, &g, Ev::Click(K_UPD_NO), now);
    assert!(s.upd.as_ref().is_some_and(|u| u.cancel_asked), "{}", s.describe());
    assert!(s.describe().contains("lic=list:"), "Licences stays: {}", s.describe());
    run(&mut s, |s| s.upd.is_none(), now);
}
