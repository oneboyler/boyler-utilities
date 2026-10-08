//! The Network page against the FAKE PC (`FakeNet::drawing`): what it shows, every action, nothing slow on open.

use super::*;
use crate::gfx::Gfx;
use crate::ui::cx::State;
use crate::ui::lay::Laid;

fn env() -> Env {
    Env { test: true, frozen: true, ..Env::default() }
}

fn opened() -> Network {
    let mut n = Network::default();
    n.open(&env(), 0.0);
    n
}

fn build(n: &mut Network, st: &mut State, now: f64) -> Laid {
    let g = Gfx::new(1.0);
    let mut cx = Cx::new(now, false, &g, st);
    let kids = n.build(&mut cx);
    Laid::new(&g, El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids), 600.0, None)
}

fn click(n: &mut Network, st: &mut State, k: Key, now: f64) {
    let g = Gfx::new(1.0);
    let mut cx = Cx::new(now, false, &g, st);
    n.event(&Ev::Click(k), &mut cx);
}

/// Waits (up to `ms`) until `f` holds, ticking the page like the frame does.
fn wait_for(n: &mut Network, ms: u64, f: impl Fn(&Network) -> bool) -> bool {
    let t0 = std::time::Instant::now();
    while t0.elapsed().as_millis() < ms as u128 {
        n.tick(t0.elapsed().as_millis() as f64);
        if f(n) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn log(n: &Network) -> Vec<String> {
    n.fake.as_ref().unwrap().log()
}

#[test]
fn open_reads_the_drawings_pc_and_starts_nothing_slow() {
    let n = opened();
    let c = n.conn.as_ref().unwrap();
    assert_eq!(c.adapters.len(), 6);
    assert_eq!(n.wifi.len(), 5);
    assert_eq!(n.dns.as_ref().unwrap().current, DnsCurrent::Automatic);
    // the speed test and the game pings start only on their buttons
    assert!(!n.speed_running && !n.speed.run);
    assert!(!n.gs_running);
    // the only live thing: the connection ping (1 per second while the page is open)
    assert!(n.ping.is_some());
    assert!(log(&n).is_empty());
}

#[test]
fn page_builds_with_every_section() {
    let mut n = opened();
    let mut st = State::default();
    let l = build(&mut n, &mut st, 0.0);
    eprintln!("page height {}", l.height);
    for (nm, k) in [("tg0", idx(K_TG, 0)), ("tg5", idx(K_TG, 5)), ("wf", sub(K_WF, "fold")), ("start", K_START), ("again", K_AGAIN), ("gspick", K_GSPICK), ("gsgo", K_GSGO), ("pill0", idx(K_GSGO, 100)), ("pill6", idx(K_GSGO, 106))] {
        eprintln!("{nm} {:?}", l.rect_of(k).map(|r| (r.0, r.1 + 56.0, r.2, r.3)));
    }
    assert!(l.height > 600.0, "height {}", l.height);
    for k in [K_DNS, K_FLUSH, K_START, K_GSGO, K_GSPICK, idx(K_TG, 0), idx(K_TG, 5)] {
        assert!(l.rect_of(k).is_some(), "missing key");
    }
}

#[test]
fn close_drops_everything() {
    let mut n = opened();
    n.close();
    assert!(n.os.is_none() && n.conn.is_none() && n.ping.is_none() && n.wifi.is_empty());
}

/// REVIEW 022 HOLD 8: the gauge's cached pictures are dropped when the page closes.
#[test]
fn close_drops_the_gauge_pictures() {
    let mut n = opened();
    let mut st = State::default();
    let l = build(&mut n, &mut st, 0.0);
    let g = Gfx::new(1.0);
    let icons = crate::icons::Icons::new();
    let mut s = crate::gfx::new_surface(600, l.height.ceil() as i32).expect("surface");
    g.begin(s.canvas());
    l.paint(&g, &icons, 0.0, 0.0, None);
    g.end();
    assert!(gauge::cached_count() > 0, "painting the page caches the gauge picture");
    n.close();
    assert_eq!(gauge::cached_count(), 0);
}

#[test]
fn admin_switches_and_dns_say_so_and_change_nothing() {
    let mut n = opened();
    let mut st = State::default();
    let _ = build(&mut n, &mut st, 0.0);
    // Ethernet (index 0) is a device switch: admin
    click(&mut n, &mut st, idx(K_TG, 0), 10.0);
    assert_eq!(n.toast.as_ref().unwrap().0, crate::admin::NOT_CHANGED);
    n.pop = Some(Pop::Dns);
    click(&mut n, &mut st, idx(K_DNSM, 1), 20.0);
    assert!(n.pop.is_none());
    assert_eq!(n.toast.as_ref().unwrap().0, crate::admin::NOT_CHANGED);
    std::thread::sleep(Duration::from_millis(50));
    n.tick(60.0);
    assert!(log(&n).is_empty());
}

#[test]
fn wifi_switch_needs_no_admin_and_goes_to_the_fake() {
    let mut n = opened();
    let mut st = State::default();
    let _ = build(&mut n, &mut st, 0.0);
    click(&mut n, &mut st, idx(K_TG, 1), 10.0);
    assert!(wait_for(&mut n, 2000, |n| n.waiting.is_empty() && n.toast.is_some()));
    assert_eq!(log(&n), ["wifi_radio off"]);
    assert!(!n.conn.as_ref().unwrap().adapters[1].enabled);
}

#[test]
fn flush_dns_shows_flushed() {
    let mut n = opened();
    let mut st = State::default();
    click(&mut n, &mut st, K_FLUSH, 10.0);
    assert!(wait_for(&mut n, 2000, |n| n.flushed_at.is_some()));
    assert_eq!(log(&n), ["flush"]);
    assert_eq!(n.toast.as_ref().unwrap().0, "DNS cache flushed");
}

#[test]
fn custom_dns_form_checks_each_field() {
    let mut n = opened();
    let mut st = State::default();
    n.pop = Some(Pop::Dns);
    click(&mut n, &mut st, idx(K_DNSM, 3), 10.0);
    assert_eq!(n.pop, Some(Pop::DnsForm));
    // the drawing's sample in a test copy
    assert_eq!(n.dns_fields[0], "9.9.9.9");
    n.dns_fields[0] = "9.9.9".into();
    n.dns_fields[2] = "zz".into();
    click(&mut n, &mut st, sub(K_DNSF, "save"), 20.0);
    assert_eq!(n.dns_bad, [true, false, true, false]);
    assert_eq!(n.pop, Some(Pop::DnsForm));
    // typing into a field clears its red mark
    let g = Gfx::new(1.0);
    let mut cx = Cx::new(30.0, false, &g, &mut st);
    n.event(&Ev::Key(sub(K_DNSF, "v4a"), 0x08), &mut cx);
    n.event(&Ev::Char(sub(K_DNSF, "v4a"), '9'), &mut cx);
    assert_eq!(n.dns_fields[0], "9.9.9");
    n.event(&Ev::Char(sub(K_DNSF, "v4a"), '.'), &mut cx);
    n.event(&Ev::Char(sub(K_DNSF, "v4a"), '9'), &mut cx);
    assert!(!n.dns_bad[0]);
    n.dns_fields[2].clear();
    // valid now: not elevated -> said so, nothing written
    n.event(&Ev::Key(sub(K_DNSF, "v4a"), 0x0D), &mut cx);
    assert!(n.pop.is_none());
    assert_eq!(n.toast.as_ref().unwrap().0, crate::admin::NOT_CHANGED);
    assert!(log(&n).is_empty());
}

#[test]
fn wifi_password_flow() {
    let mut n = opened();
    let mut st = State::default();
    n.wf_open = true;
    // TP-Link_8F2C (index 2): secured, not saved -> the password field opens
    click(&mut n, &mut st, idx(K_WCONN, 2), 10.0);
    assert_eq!(n.wf_pw_for.as_deref(), Some("TP-Link_8F2C"));
    let _ = build(&mut n, &mut st, 20.0);
    // Connect with an empty field does nothing but nudge the field (REVIEW 022 HOLD 5: scale 1 -> 1.14 -> 1, 340 ms)
    n.now = 30.0;
    click(&mut n, &mut st, K_WGO, 30.0);
    assert!(log(&n).is_empty());
    assert_eq!(n.nudge_at, Some(30.0));
    assert_eq!(view::nudge_scale(0.0, false), Some(1.0));
    let peak = (1..340).map(|ms| view::nudge_scale(ms as f64, false).unwrap()).fold(0.0f32, f32::max);
    assert!((peak - 1.14).abs() < 0.002, "{peak}");
    assert_eq!(view::nudge_scale(340.0, false), None);
    assert_eq!(view::nudge_scale(100.0, true), None, "reduced motion: no pulse");
    let _ = build(&mut n, &mut st, 100.0);
    let g = Gfx::new(1.0);
    {
        let mut cx = Cx::new(40.0, false, &g, &mut st);
        for c in "correct horse".chars() {
            n.event(&Ev::Char(K_WPW, c), &mut cx);
        }
        n.event(&Ev::Key(K_WPW, 0x0D), &mut cx);
    }
    assert!(n.wf_pw.is_empty() && n.wf_pw_for.is_none());
    assert!(wait_for(&mut n, 2000, |n| n.wifi.first().is_some_and(|w| w.ssid == "TP-Link_8F2C" && w.connected)));
    assert_eq!(log(&n), ["wifi_connect TP-Link_8F2C auto=true +password"]);
    // forget a saved one
    let i = n.wifi.iter().position(|w| w.ssid == "MyHome_5G").unwrap();
    click(&mut n, &mut st, idx(K_WFORGET, i), 50.0);
    assert!(wait_for(&mut n, 2000, |n| n.wifi.iter().any(|w| w.ssid == "MyHome_5G" && !w.saved)));
}

/// REVIEW 022 HOLD 7 - errors: offline, a wrong Wi-Fi password, a failed flush each say so (and change nothing they shouldn't).
#[test]
fn offline_start_and_game_start_say_youre_offline() {
    let mut n = opened();
    let mut st = State::default();
    n.fake.as_ref().unwrap().with(|s| s.internet_if = None);
    n.read_all();
    assert!(n.offline());
    click(&mut n, &mut st, K_START, 10.0);
    assert!(!n.speed_running && !n.speed.run, "no speed test while offline");
    assert_eq!(n.toast.as_ref().unwrap().0, "You’re offline");
    n.toast = None;
    click(&mut n, &mut st, K_GSGO, 20.0);
    assert!(!n.gs_running, "no game pings while offline");
    assert_eq!(n.toast.as_ref().unwrap().0, "You’re offline");
    // the page shows it: the red "Offline" mark in the Connection header builds
    let _ = build(&mut n, &mut st, 30.0);
}

#[test]
fn a_wrong_wifi_password_says_check_the_password() {
    let mut n = opened();
    let mut st = State::default();
    n.fake.as_ref().unwrap().with(|s| {
        s.wifi_passwords.insert("TP-Link_8F2C".into(), "right one".into());
    });
    n.wf_open = true;
    click(&mut n, &mut st, idx(K_WCONN, 2), 10.0);
    let g = Gfx::new(1.0);
    {
        let mut cx = Cx::new(20.0, false, &g, &mut st);
        for c in "wrong one".chars() {
            n.event(&Ev::Char(K_WPW, c), &mut cx);
        }
        n.event(&Ev::Key(K_WPW, 0x0D), &mut cx);
    }
    assert!(wait_for(&mut n, 2000, |n| n.toast.as_ref().is_some_and(|t| t.0.contains("check the password"))));
    assert_eq!(n.toast.as_ref().unwrap().0, "Couldn't connect to TP-Link_8F2C · check the password");
    assert_eq!(log(&n), ["wifi_connect TP-Link_8F2C refused"]);
    assert!(!n.wifi.iter().any(|w| w.ssid == "TP-Link_8F2C" && w.connected));
}

#[test]
fn a_failed_flush_says_so() {
    let mut n = opened();
    let mut st = State::default();
    n.fake.as_ref().unwrap().with(|s| s.fail_next_change = Some(NetError::AccessDenied("refused".into())));
    click(&mut n, &mut st, K_FLUSH, 10.0);
    assert!(wait_for(&mut n, 2000, |n| n.toast.is_some()));
    let t = &n.toast.as_ref().unwrap().0;
    assert!(t.starts_with("Couldn't flush DNS · "), "{t}");
    assert!(n.flushed_at.is_none(), "no green 'Flushed' after a failure");
}

#[test]
fn speed_test_runs_only_on_start_and_ends_with_the_result() {
    let mut n = opened();
    let mut st = State::default();
    let _ = build(&mut n, &mut st, 0.0);
    assert!(!n.speed_running);
    click(&mut n, &mut st, K_START, 10.0);
    assert!(n.speed.run && n.speed_running);
    // a second press while it runs starts nothing new
    click(&mut n, &mut st, K_START, 20.0);
    assert!(wait_for(&mut n, 30_000, |n| n.speed.done));
    let s = &n.speed;
    assert!(s.down.unwrap() > 500.0 && s.up.unwrap() > 50.0, "{s:?}");
    assert!(s.ping.unwrap() >= 7.0 && s.jit.is_some());
    assert!(n.toast.as_ref().unwrap().0.starts_with("Speed test done · ↓ "));
    assert_eq!(s.foot.as_deref(), Some("Today 21:37 · Ethernet"));
}

/// F2 (the owner Oct 8): leaving the tab / closing the window does not stop the speed test or the game pings - they go on in
/// the app's kept state and the page shows them (and the result) when it opens again.
#[test]
fn leaving_the_tab_keeps_the_speed_test_and_the_pings_and_their_results() {
    let e = env();
    let mut n = Network::default();
    n.open(&e, 0.0);
    let mut st = State::default();
    click(&mut n, &mut st, K_START, 10.0);
    click(&mut n, &mut st, K_GSGO, 20.0);
    assert!(n.speed.run && n.gs_running);
    // the tab is left (same menu window = same page object): both go on
    n.close();
    assert!(n.ping.is_none(), "the connection ping is live only while the page is open");
    n.open(&e, 100.0);
    assert!(n.speed_running && n.gs_running);
    // a round comes back (the fake has no answers: every region times out, the round still ends)
    assert!(wait_for(&mut n, 8000, |n| n.gs_rounds >= 1));
    n.close();
    // the MENU closes (its pages are dropped): the endless pings stop, the finite speed test goes on (boss call, Oct 8)
    drop(n);
    let bg = e.keep.get::<Bg>(BG_KEY).unwrap();
    assert!(bg.lock().unwrap().gs.is_none(), "pings run only while the menu is open");
    let t0 = std::time::Instant::now();
    while !bg.lock().unwrap().speed.done && t0.elapsed().as_secs() < 30 {
        std::thread::sleep(Duration::from_millis(20));
    }
    // the next menu shows the result and the last pings at once, with Start
    let mut n = Network::default();
    n.open(&e, 200.0);
    assert!(n.speed.done && n.speed.down.is_some(), "{:?}", n.speed);
    assert!(n.toast.is_none(), "a result made while away is shown, not toasted late");
    assert!(!n.gs_running && n.pinged_at.is_some() && !n.pills.is_empty());
    click(&mut n, &mut st, K_GSGO, 210.0);
    assert!(n.gs_running, "Start pings again");
}

#[test]
fn game_servers_ping_only_after_start_until_stop() {
    let mut n = opened();
    let mut st = State::default();
    let _ = build(&mut n, &mut st, 0.0);
    assert!(!n.gs_running && n.pills.is_empty());
    click(&mut n, &mut st, K_GSGO, 10.0);
    assert!(n.gs_running);
    // the fake has no game-server answers: every region times out (or fails to resolve) - rounds still complete
    assert!(wait_for(&mut n, 8000, |n| n.gs_rounds >= 1));
    click(&mut n, &mut st, K_GSGO, 20.0);
    assert!(!n.gs_running);
    let rounds = n.gs_rounds;
    std::thread::sleep(Duration::from_millis(1300));
    n.tick(30.0);
    assert_eq!(n.gs_rounds, rounds, "no results after Stop");
}

#[test]
fn picking_another_game_stops_the_pings() {
    let mut n = opened();
    let mut st = State::default();
    click(&mut n, &mut st, K_GSGO, 10.0);
    assert!(n.gs_running);
    n.pop = Some(Pop::Games);
    click(&mut n, &mut st, idx(K_GSM, 1), 20.0);
    assert!(!n.gs_running);
    assert_eq!(n.games[n.game].id, "cs2");
    let l = build(&mut n, &mut st, 30.0);
    assert!(l.rect_of(K_GSGO).is_some());
}

/// Debug aid for the pixel proof: `BU_DUMP=<y0>,<y1> cargo test -p bu-app <page>::tests::dump_layout -- --nocapture` prints every
/// box whose top (window coordinates) is in [y0, y1) with its text - to compare with tools/ref/dom_dump.js. Does nothing otherwise.
#[test]
fn dump_layout() {
    let Ok(r) = std::env::var("BU_DUMP") else { return };
    let (y0, y1) = r.split_once(',').map(|(a, b)| (a.parse::<f32>().unwrap(), b.parse::<f32>().unwrap())).unwrap();
    let mut p = opened();
    let mut st = State::default();
    let l = build(&mut p, &mut st, 0.0);
    eprintln!("page height {}", l.height);
    for n in &l.nodes {
        let (x, y, w, h) = n.rect;
        let y = y + 56.0;
        if y >= y0 && y < y1 {
            let t = match &n.el.content {
                crate::ui::el::Content::Text(t) => format!("{:?}", t.s),
                crate::ui::el::Content::Icon(i) => format!("icon {}", i.name),
                _ => String::new(),
            };
            eprintln!("[{x:.2}, {y:.2}, {w:.2}, {h:.2}] {t}");
        }
    }
}

#[test]
fn popups_build_without_panicking() {
    for pop in [Pop::Dns, Pop::DnsForm, Pop::Games] {
        let mut n = opened();
        let mut st = State::default();
        n.dns_anchor = (356.8, 110.0, 123.56, 22.0);
        n.gs_anchor = (72.0, 134.3, 128.0, 24.0);
        if pop == Pop::DnsForm {
            n.open_dns_form();
        } else {
            n.pop = Some(pop);
        }
        let _ = build(&mut n, &mut st, 0.0);
        let g = Gfx::new(1.0);
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        let p = n.popup(&mut cx).expect("a popup");
        let l = Laid::new(&g, El::block().w(600.0).h(520.0).child(p), 600.0, Some(520.0));
        assert!(l.nodes.len() > 5);
    }
}

// ------------------------------------------------------------------ Order 036: the change log + the shared reset
fn rec(item: &str) -> Option<crate::undo::Record> {
    crate::services::with(|s| crate::undo::read_record(&s.store, "net", item)).flatten()
}

fn review(n: &Network, kind: Kind) -> crate::undo::Review {
    crate::services::with(|s| {
        crate::undo::flush(&mut s.store);
        crate::undo::Review::for_page(kind, n, &s.store)
    })
    .unwrap()
}

/// As the frame does it: the page applies outside the services, each ok line is noted.
fn reset(n: &mut Network, rv: &crate::undo::Review) -> Vec<crate::undo::LineResult> {
    let res = rv.apply_each(&mut [n as &mut dyn Resettable], &mut |l| {
        crate::undo::note(&l.page, &l.item, &l.label, &l.from, &l.to);
        Ok(())
    });
    crate::services::with(|s| crate::undo::flush(&mut s.store));
    res
}

fn elevated() -> Network {
    let n = opened();
    n.fake.as_ref().unwrap().with(|s| s.elevated = true);
    n
}

/// An adapter's switch = ONE entry (its state before the first change); untick = kept; the reset puts it back.
#[test]
fn an_adapter_switch_goes_into_the_change_log_and_back() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    let mut n = elevated();
    let mut st = State::default();
    let i = n.conn.as_ref().unwrap().adapters.iter().position(|a| a.id == "{VBOX-0001}").unwrap();
    click(&mut n, &mut st, idx(K_TG, i), 10.0);
    assert!(wait_for(&mut n, 2000, |n| n.waiting.is_empty() && n.conn.as_ref().unwrap().adapters[i].enabled));
    let r = rec("adapter:{VBOX-0001}").expect("an entry");
    assert_eq!((r.label.as_str(), r.was.raw.as_str(), r.now.raw.as_str()), ("VirtualBox Host-Only", "off", "on"));
    let mut rv = review(&n, Kind::HowItWas);
    assert_eq!(rv.lines.len(), 1);
    assert_eq!(rv.lines[0].change_text(), "On  →  Off");
    rv.toggle(0);
    assert!(reset(&mut n, &rv).is_empty());
    assert!(!log(&n).contains(&"adapter {VBOX-0001} off".to_string()), "an unticked line stays as it is");
    rv.toggle(0);
    assert_eq!(reset(&mut n, &rv)[0].outcome, crate::undo::Outcome::Ok);
    assert_eq!(log(&n).last().map(String::as_str), Some("adapter {VBOX-0001} off"));
    assert!(wait_for(&mut n, 2000, |n| !n.conn.as_ref().unwrap().adapters[i].enabled));
    assert!(review(&n, Kind::HowItWas).is_empty(), "nothing left to reset");
    crate::services::shutdown();
}

/// DNS: the pick is an entry ("Automatic" before it); "Windows defaults" = every adapter on, DNS automatic (the drawing's
/// RS.net), applied through the page.
#[test]
fn dns_goes_into_the_change_log_and_windows_defaults_put_everything_back() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    let mut n = elevated();
    let mut st = State::default();
    n.pop = Some(Pop::Dns);
    click(&mut n, &mut st, idx(K_DNSM, 1), 20.0);
    assert!(wait_for(&mut n, 2000, |n| !n.dns_wait));
    let r = rec("dns:{ETH-0001}").expect("an entry");
    assert_eq!((r.label.as_str(), r.was.text.as_str(), r.now.text.as_str(), r.was.raw.as_str()), ("DNS (Ethernet)", "Automatic", "Cloudflare", "auto"));
    let how = review(&n, Kind::HowItWas);
    assert_eq!(how.lines[0].change_text(), "Cloudflare  →  Automatic");
    let rv = review(&n, Kind::WindowsDefaults);
    let lines: Vec<String> = rv.lines.iter().map(|l| format!("{} · {}", l.label, l.change_text())).collect();
    assert_eq!(lines, ["VirtualBox Host-Only · Off  →  On", "Bluetooth Network · Off  →  On", "DNS (Ethernet) · Cloudflare  →  Automatic"]);
    let res = reset(&mut n, &rv);
    assert!(res.iter().all(|r| r.outcome == crate::undo::Outcome::Ok), "{res:?}");
    assert!(n.fake.as_ref().unwrap().with(|s| s.dns.is_empty()));
    assert!(wait_for(&mut n, 2000, |n| n.conn.as_ref().unwrap().adapters.iter().all(|a| a.enabled)));
    assert!(review(&n, Kind::WindowsDefaults).is_empty(), "everything at Windows' values");
    // the DNS is back to how it was; the two adapters Windows defaults switched on are now changes of their own
    let how: Vec<String> = review(&n, Kind::HowItWas).lines.iter().map(|l| l.item.clone()).collect();
    assert_eq!(how, ["adapter:{BT-0001}", "adapter:{VBOX-0001}"]);
    crate::services::shutdown();
}

/// The reset line (new on this page) opens the frame's shared review under the clicked link.
#[test]
fn the_reset_line_opens_the_shared_review() {
    let mut n = opened();
    let mut st = State::default();
    let l = build(&mut n, &mut st, 0.0);
    assert!(l.rect_of(sub(K_RESET, "win")).is_some(), "the reset line with Windows defaults");
    let g = Gfx::new(1.0);
    let mut cx = Cx::new(0.0, false, &g, &mut st).for_page("net");
    let b = (200.0, 900.0, 140.0, 16.0);
    n.event(&Ev::Press(sub(K_RESET, "win"), 210.0, 905.0, b), &mut cx);
    n.event(&Ev::Click(sub(K_RESET, "win")), &mut cx);
    assert!(matches!(cx.reqs.as_slice(), [crate::ui::cx::Req::Reset(Kind::WindowsDefaults, r)] if *r == b), "{:?}", cx.reqs);
}

/// Settings › Reset / the uninstaller ask a CLOSED page: `resettable()` opens nothing; the reset goes through a service
/// made on first use (the fake PC in tests - never Windows); without admin it says so and changes nothing.
#[test]
fn a_closed_page_resets_through_its_own_service() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    let mut n = Network::default();
    assert!(n.resettable().is_some());
    assert!(n.rs_svc.get().is_none(), "resettable() is cheap: nothing made");
    crate::undo::note("net", "dns:{ETH-0001}", "DNS (Ethernet)", &Val::new("9.9.9.9,149.112.112.112|2620:fe::fe", "Custom · 9.9.9.9"), &Val::new("auto", "Automatic"));
    let rv = review(&n, Kind::HowItWas);
    assert_eq!(rv.lines[0].change_text(), "Automatic  →  Custom · 9.9.9.9", "now = read from the (fake) PC");
    let res = reset(&mut n, &rv);
    assert_eq!(res[0].outcome, crate::undo::Outcome::Failed(crate::admin::NOT_CHANGED.into()));
    // an elevated fake behind a closed page: the custom servers come back exactly
    let fake = Arc::new(FakeNet::drawing());
    fake.with(|s| s.elevated = true);
    let mut n = Network::default();
    let _ = n.rs_svc.set(Arc::new(NetworkService::new(fake.clone())));
    assert_eq!(reset(&mut n, &rv)[0].outcome, crate::undo::Outcome::Ok);
    let got = fake.with(|s| s.dns.get("{ETH-0001}").cloned()).unwrap();
    assert_eq!(dns_val(&got).raw, "9.9.9.9,149.112.112.112|2620:fe::fe");
    assert!(review(&n, Kind::HowItWas).is_empty());
    crate::services::shutdown();
}

/// Order 047 (idle cost): with nothing moving the page asks for no frames - `tick` is false, `wake_at` is None, a build
/// does not ask for the next frame; "Flushed" asks to be built again at its end (not every frame); a running speed test
/// is polled for its end at a future time.
#[test]
fn nothing_moving_asks_for_no_frames() {
    let mut n = opened();
    let mut st = State::default();
    let _ = build(&mut n, &mut st, 0.0);
    // (the connection ping lands once a second: right after open nothing new is in)
    n.tick(10.0);
    assert!(!n.tick(20.0), "nothing new: no repaint");
    assert_eq!(n.wake_at(20.0), None);
    let mut st = State::default();
    let _ = build(&mut n, &mut st, 30.0);
    assert!(!st.busy, "an idle page asks for no frames");
    // Flush DNS: the green "Flushed" stays 1.6 s - one build at its end, no frames in between
    click(&mut n, &mut st, K_FLUSH, 40.0);
    assert!(wait_for(&mut n, 2000, |n| n.flushed_at.is_some()));
    let at = n.flushed_at.unwrap();
    n.toast = None;
    let mut st = State::default();
    let _ = build(&mut n, &mut st, at + 500.0);
    assert!(!st.busy, "\"Flushed\" is not motion");
    assert_eq!(st.wake, Some(at + crate::ui::pieces::mbtn::DONE_MS));
    // a running speed test: polled for its end at a future time (its samples wake the menu themselves)
    click(&mut n, &mut st, K_START, 50.0);
    assert!(n.speed_running);
    assert!(n.wake_at(60.0).is_some_and(|t| t > 60.0));
}

/// Order 047: the frame's reset through the page's detached copy - the review opened and the Reset pressed each inside
/// one frame (16 ms), the reads and the put-backs on the review's worker thread; then the page re-reads (`reset_done`).
/// (The page's fake has no slow mode for these calls: the proof is that both run on the worker - `Reading` / `Running`.)
fn reset_off_the_menu(p: &mut dyn Resettable, kind: Kind) -> (crate::undo::Review, Vec<crate::undo::LineResult>) {
    fn wait<T>(mut f: impl FnMut() -> Option<T>) -> T {
        let t0 = std::time::Instant::now();
        loop {
            if let Some(v) = f() {
                return v;
            }
            assert!(t0.elapsed().as_secs() < 10, "the review's worker never answered");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    let opened = crate::services::with(|s| {
        crate::undo::flush(&mut s.store);
        crate::offui::assert_quick("opening the review", || crate::undo::Review::open(kind, false, &mut [&mut *p], &s.store))
    })
    .unwrap();
    let crate::undo::Opened::Reading(mut job) = opened else { panic!("the review is read on a worker thread") };
    let rv = wait(|| job.take());
    let applied = crate::offui::assert_quick("Reset", || rv.start_apply(&mut [&mut *p]));
    let crate::undo::Applied::Running(mut job) = applied else { panic!("the reset is put back on a worker thread") };
    let res = wait(|| job.take());
    p.reset_done();
    (rv, res)
}

/// Order 047: Network's reset (Windows defaults: the two adapters switched off go On) is read and put back on the
/// review's worker thread - the IP Helper reads and the admin-proxied switch never hold the menu; same lines, same results.
#[test]
fn the_reset_review_reads_and_puts_back_off_the_menus_thread() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    let mut n = elevated();
    let (rv, res) = reset_off_the_menu(&mut n, Kind::WindowsDefaults);
    let lines: Vec<String> = rv.lines.iter().map(|l| format!("{} · {}", l.label, l.change_text())).collect();
    assert_eq!(lines, ["VirtualBox Host-Only · Off  →  On", "Bluetooth Network · Off  →  On"]);
    assert!(res.iter().all(|r| r.outcome == crate::undo::Outcome::Ok), "{res:?}");
    assert_eq!(crate::undo::reset_toast(Kind::WindowsDefaults, 2, &res), "Windows defaults \u{b7} 2 settings reset");
    assert!(wait_for(&mut n, 2000, |n| n.conn.as_ref().unwrap().adapters.iter().all(|a| a.enabled)), "the open page shows it");
    crate::services::shutdown();
}

/// Order 047: picking a DNS server (an IP Helper write behind Windows' admin prompt) never holds the menu: the click
/// hands it to a change thread (here made 200 ms slow) and returns within one frame; the new DNS shows when it is done.
#[test]
fn a_dns_pick_never_holds_the_menu() {
    SLOW_CHANGE_MS.with(|c| c.set(Some(200)));
    let mut n = elevated();
    let mut st = State::default();
    n.pop = Some(Pop::Dns);
    let g = Gfx::new(1.0);
    let mut cx = Cx::new(20.0, false, &g, &mut st);
    crate::offui::assert_quick("the DNS pick", || n.event(&Ev::Click(idx(K_DNSM, 1)), &mut cx));
    drop(cx);
    assert!(n.dns_wait, "waiting for the change thread");
    assert!(wait_for(&mut n, 3000, |n| !n.dns_wait), "the change ends");
    assert_ne!(n.dns.as_ref().unwrap().current, DnsCurrent::Automatic, "the new DNS shows");
    SLOW_CHANGE_MS.with(|c| c.set(None));
}
