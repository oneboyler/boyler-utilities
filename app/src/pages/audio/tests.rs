//! Order 018 Audio page tests: every action against bu-audio's / bu-micmute's FAKES (a test never changes the PC), and
//! the page's boxes where Chromium lays out the drawing (menu-v22, tools/ref/dom_dump.js).

use super::*;
use crate::gfx::Gfx;
use crate::ui::cx::State;
use crate::ui::el::Content;
use crate::ui::lay::Laid;

fn env() -> Env {
    Env { test: true, real_read: false, frozen: true, rm: false, ..Env::default() }
}

fn page() -> Audio {
    mute::reset_for_test();
    svc::set_rules(svc::Rules::default());
    let mut a = Audio::new();
    a.open(&env(), 0.0);
    a
}

/// The page laid out as the frame does (`.pg`: 600 wide, padding 2 26 18 26), window coordinates (page top 56).
fn lay(a: &mut Audio, g: &Gfx, st: &mut State, now: f64) -> Laid {
    let mut cx = Cx::new(now, false, g, st);
    let kids = a.build(&mut cx);
    Laid::new(g, El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids), 600.0, None)
}

fn text_box(l: &Laid, s: &str) -> Option<(f32, f32, f32, f32)> {
    l.nodes.iter().find(|n| matches!(&n.el.content, Content::Text(t) if t.s == s)).map(|n| (n.rect.0, n.rect.1 + 56.0, n.rect.2, n.rect.3))
}

/// The page's own clock in a test: the moment it next looks at the worker (Order 055: the worker is looked at once per
/// step - 33 ms with sound, 100 ms in silence - so a test that ticked at one fixed time would look once).
fn step(a: &mut Audio) -> bool {
    let now = a.st.as_ref().map_or(1000.0, |s| s.next_poll);
    a.tick(now)
}

fn wait(a: &mut Audio, ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
    step(a);
}

fn click(a: &mut Audio, k: Key, g: &Gfx, st: &mut State) {
    let mut cx = Cx::new(1000.0, false, g, st).for_page("aud");
    a.event(&Ev::Click(k), &mut cx);
}

/// Where the drawing has them (Chromium, menu-v22 at rest; window px): the group headers, rows, the link, the names.
#[test]
fn boxes_match_the_drawing() {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut a = page();
    let l = lay(&mut a, &g, &mut st, 0.0);
    let near = |a: f32, b: f32| (a - b).abs() < 0.02;
    let chk = |s: &str, x: f32, y: f32| {
        let b = text_box(&l, s).unwrap_or_else(|| panic!("no text {s}"));
        assert!(near(b.0, x) && near(b.1, y), "{s} at {:?}, the drawing {x},{y}", b);
    };
    // the owner Oct 8: "Mute settings" moved up beside the mic icon, "no new space": the Input row lost the drawing's extra
    // 14 px (`.dvr.inr{padding-bottom:21px}` -> 7), so everything below it sits 14 px higher than in menu-v22
    const UP: f32 = 14.0;
    chk("Devices", 38.0, 110.0);
    chk("Apps", 38.0, 301.2344 - UP);
    chk("Spotifast", 72.0, 331.2969 - UP);
    chk("Discord", 72.0, 370.2969 - UP);
    chk("VALORANT", 72.0, 409.2969 - UP);
    chk("Chrome", 72.0, 448.2969 - UP);
    chk("System sounds", 72.0, 487.2969 - UP);
    chk("Reset this page", text_box(&l, "Reset this page").unwrap().0, 601.4688 - UP);
    // the page's height (the drawing's .pg scrollHeight 581.66 = rsl bottom 617.66 + 2 + 18 - 56)
    assert!((l.height - (581.6563 - UP)).abs() < 0.05, "height {}", l.height);
    // "Input" + the small "Mute settings" under it: together centred on the mic icon, the row as tall as Output
    let inp = text_box(&l, "Input").unwrap();
    let mml = text_box(&l, "Mute settings").unwrap();
    let mic = l.rect_of(K_MIC).map(|r| (r.0, r.1 + 56.0, r.2, r.3)).unwrap();
    let pair = (inp.1 + mml.1 + mml.3) / 2.0;
    assert!((pair - (mic.1 + mic.3 / 2.0)).abs() < 0.6, "pair centre {pair} vs mic {mic:?}");
    assert_eq!(mml.0, inp.0, "the link starts under Input");
    let out = text_box(&l, "Output").unwrap();
    assert!((inp.1 - out.1 - 44.0).abs() < 1.0 || inp.1 - out.1 < 44.0, "Input row no taller than 44: {} {}", out.1, inp.1);
    assert!(mml.2 <= 46.0 + 12.0, "the link ends before the slider: {}", mml.2);
}

#[test]
fn opens_with_the_drawings_values_and_nothing_slow() {
    let a = page();
    let d = a.describe();
    assert!(d.contains("out=arctis in=mv7 outvol=74 invol=90"), "{d}");
    for want in ["app Spotifast vol=64 muted=false", "app Discord vol=80", "app VALORANT vol=72", "app Chrome vol=100 muted=true", "app System sounds vol=50"] {
        assert!(d.contains(want), "{want} in {d}");
    }
    // opening changed nothing
    assert!(!d.contains("\ncall "), "{d}");
}

#[test]
fn picking_devices_and_switching_one_off() {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut a = page();
    click(&mut a, K_OUT_PICK, &g, &mut st);
    assert!(a.describe().contains("menu=out"));
    // the switched-off DualSense can't be picked: its switch nudges, nothing changes
    click(&mut a, k_dev(3), &g, &mut st);
    assert!(a.describe().contains("menu=out"));
    click(&mut a, k_dev(0), &g, &mut st);
    wait(&mut a, 60);
    let d = a.describe();
    assert!(d.contains("out=spk") && d.contains("call set_default spk Console"), "{d}");
    assert!(d.contains("menu=none"));
    // a device's own switch (Monitor off), then the last one on can't be switched off
    click(&mut a, K_IN_PICK, &g, &mut st);
    click(&mut a, k_devsw(2), &g, &mut st);
    wait(&mut a, 60);
    assert!(a.describe().contains("call set_enabled c920 false"), "{}", a.describe());
}

#[test]
fn needs_admin_shows_the_helper_toast() {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut a = page();
    a.st.as_ref().unwrap().svc.with_fake(|f| f.enable_needs_admin = true);
    click(&mut a, K_OUT_PICK, &g, &mut st);
    click(&mut a, k_devsw(0), &g, &mut st);
    wait(&mut a, 60);
    let t = a.st.as_ref().unwrap().toast.clone().map(|t| t.0).unwrap_or_default();
    assert!(t.contains("Needs admin"), "toast {t}");
}

#[test]
fn mute_unmute_type_and_drag() {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut a = page();
    click(&mut a, k_mute(0), &g, &mut st);
    wait(&mut a, 700);
    let d = a.describe();
    assert!(d.contains("app Spotifast vol=64 muted=true") && d.contains("call set_session_mute spotifast true"), "{d}");
    // typing 250 clamps to 100
    click(&mut a, k_pct(1), &g, &mut st);
    {
        let mut cx = Cx::new(1000.0, false, &g, &mut st);
        for c in "250".chars() {
            a.event(&Ev::Char(k_pct(1), c), &mut cx);
        }
        a.event(&Ev::Key(k_pct(1), 0x0D), &mut cx);
    }
    wait(&mut a, 700);
    let d = a.describe();
    assert!(d.contains("app Discord vol=100") && d.contains("call set_session_volume discord 100"), "{d}");
    // typing a % unmutes a muted app (Chrome starts muted), as in Windows
    click(&mut a, k_pct(3), &g, &mut st);
    {
        let mut cx = Cx::new(1000.0, false, &g, &mut st);
        for c in "35".chars() {
            a.event(&Ev::Char(k_pct(3), c), &mut cx);
        }
        a.event(&Ev::Blur(k_pct(3)), &mut cx);
    }
    wait(&mut a, 700);
    let d = a.describe();
    assert!(d.contains("app Chrome vol=35 muted=false") && d.contains("call set_session_mute chrome false"), "{d}");
    // Esc cancels
    click(&mut a, K_OUT_PCT, &g, &mut st);
    {
        let mut cx = Cx::new(1000.0, false, &g, &mut st);
        a.event(&Ev::Char(K_OUT_PCT, '4'), &mut cx);
        a.event(&Ev::Key(K_OUT_PCT, 0x1B), &mut cx);
    }
    wait(&mut a, 700);
    assert!(a.describe().contains("outvol=74"));
    // a drag on the Output pill: the slider's box 130..292 -> 50 % at x = 130 + 8 + 146 * .5
    {
        let mut cx = Cx::new(1000.0, false, &g, &mut st);
        let r = (130.0, 138.84, 162.0, 30.0);
        a.event(&Ev::Press(K_OUT_VOL, 211.0, 150.0, r), &mut cx);
        a.event(&Ev::Release(K_OUT_VOL), &mut cx);
    }
    wait(&mut a, 700);
    let d = a.describe();
    assert!(d.contains("outvol=50") && d.contains("call set_volume arctis 50"), "{d}");
}

// ------------------------------------------------------------------ Order 036: the change log + the shared reset
fn rec(item: &str) -> Option<crate::undo::Record> {
    crate::services::with(|s| crate::undo::read_record(&s.store, "aud", item)).flatten()
}

/// The worker's entries are written on the page's next tick.
fn wait_rec(a: &mut Audio, item: &str, now_raw: &str) -> crate::undo::Record {
    for _ in 0..300 {
        step(a);
        if let Some(r) = rec(item).filter(|r| r.now.raw == now_raw) {
            return r;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("no entry {item} -> {now_raw}: {:?}", rec(item));
}

fn review(a: &Audio, kind: Kind) -> crate::undo::Review {
    crate::services::with(|s| {
        crate::undo::flush(&mut s.store);
        crate::undo::Review::for_page(kind, a, &s.store)
    })
    .unwrap()
}

/// As the frame does it: the page applies outside the services, each ok line is noted.
fn reset(a: &mut Audio, rv: &crate::undo::Review) -> Vec<crate::undo::LineResult> {
    let res = rv.apply_each(&mut [a as &mut dyn Resettable], &mut |l| {
        crate::undo::note(&l.page, &l.item, &l.label, &l.from, &l.to);
        Ok(())
    });
    crate::services::with(|s| crate::undo::flush(&mut s.store));
    res
}

fn fake_default(a: &Audio, flow: Flow) -> String {
    a.st.as_ref().unwrap().svc.with_fake(|f| f.defaults[&(flow, bu_audio::Role::Communications)].clone()).unwrap()
}

/// A picked device = ONE entry with all three roles as they were before the FIRST pick; untick = kept; the reset puts every
/// role back through the page's fake; then nothing is left to reset.
#[test]
fn a_new_default_device_goes_into_the_change_log_and_back() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut a = page();
    click(&mut a, K_OUT_PICK, &g, &mut st);
    click(&mut a, k_dev(0), &g, &mut st);
    let r = wait_rec(&mut a, "out.default", "spk|spk|spk");
    assert_eq!((r.label.as_str(), r.was.raw.as_str(), r.was.text.as_str()), ("Default output", "arctis|arctis|arctis", "Headphones (Arctis Nova)"));
    click(&mut a, K_OUT_PICK, &g, &mut st);
    click(&mut a, k_dev(2), &g, &mut st);
    let r = wait_rec(&mut a, "out.default", "nv|nv|nv");
    assert_eq!(r.was.raw, "arctis|arctis|arctis", "how the PC was = before the FIRST change");
    wait(&mut a, 300);
    let mut rv = review(&a, Kind::HowItWas);
    assert_eq!(rv.lines.len(), 1);
    assert_eq!(rv.lines[0].change_text(), "Monitor (NVIDIA HD Audio)  →  Headphones (Arctis Nova)");
    rv.toggle(0);
    assert!(reset(&mut a, &rv).is_empty());
    assert_eq!(fake_default(&a, Flow::Output), "nv", "an unticked line stays as it is");
    rv.toggle(0);
    let res = reset(&mut a, &rv);
    assert_eq!(res[0].outcome, crate::undo::Outcome::Ok);
    assert_eq!(fake_default(&a, Flow::Output), "arctis", "back to how the PC was (calls too)");
    wait(&mut a, 400);
    assert!(review(&a, Kind::HowItWas).is_empty(), "nothing left to reset");
    crate::services::shutdown();
}

/// Switching off the device in use: its switch AND the default Windows was moved off it are logged; the reset switches
/// it on first ("dev:" sorts before "out.default"), then makes it the default again.
#[test]
fn a_switched_off_device_and_the_moved_default_come_back() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut a = page();
    click(&mut a, K_OUT_PICK, &g, &mut st);
    click(&mut a, k_devsw(1), &g, &mut st);
    let r = wait_rec(&mut a, "dev:arctis", "off");
    assert_eq!((r.label.as_str(), r.was.text.as_str(), r.now.text.as_str()), ("Headphones (Arctis Nova)", "On", "Off"));
    let d = wait_rec(&mut a, "out.default", "spk|spk|spk");
    assert_eq!(d.was.raw, "arctis|arctis|arctis");
    wait(&mut a, 300);
    let rv = review(&a, Kind::HowItWas);
    assert_eq!(rv.lines.iter().map(|l| l.item.as_str()).collect::<Vec<_>>(), ["dev:arctis", "out.default"]);
    let res = reset(&mut a, &rv);
    assert!(res.iter().all(|r| r.outcome == crate::undo::Outcome::Ok), "{res:?}");
    let on = a.st.as_ref().unwrap().svc.with_fake(|f| f.devices.iter().find(|d| d.id == "arctis").unwrap().state).unwrap();
    assert_eq!(on, bu_audio::DeviceState::On);
    assert_eq!(fake_default(&a, Flow::Output), "arctis");
    crate::services::shutdown();
}

/// An app's slider: ONE entry per drag (not one per step), the volume before the drag; the reset puts it back.
#[test]
fn an_app_volume_drag_is_one_entry_and_comes_back() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut a = page();
    let i = a.st.as_ref().unwrap().apps.iter().position(|x| x.name == "Discord").expect("Discord row");
    {
        let mut cx = Cx::new(1000.0, false, &g, &mut st).for_page("aud");
        let r = (200.0, 300.0, 200.0, 22.0);
        a.event(&Ev::Press(k_vol(i), 300.0, 311.0, r), &mut cx);
        a.event(&Ev::Drag(k_vol(i), 280.0, 311.0, r), &mut cx);
        a.event(&Ev::Drag(k_vol(i), 263.2, 311.0, r), &mut cx);
        a.event(&Ev::Release(k_vol(i)), &mut cx);
    }
    let all = crate::services::with(|s| crate::undo::records(&s.store, Some("aud"))).unwrap();
    assert_eq!(all.len(), 1, "{all:?}");
    let r = &all[0];
    assert_eq!(r.item, r"app:c:\apps\discord.exe");
    assert_eq!((r.label.as_str(), r.was.raw.as_str(), r.now.raw.as_str(), r.was.text.as_str()), ("Discord volume", "0.80|0", "0.30|0", "80 %"));
    wait(&mut a, 700);
    let rv = review(&a, Kind::HowItWas);
    assert_eq!(rv.lines[0].change_text(), "30 %  →  80 %");
    assert_eq!(reset(&mut a, &rv)[0].outcome, crate::undo::Outcome::Ok);
    wait(&mut a, 700);
    assert!(a.describe().contains("app Discord vol=80 muted=false"), "{}", a.describe());
    crate::services::shutdown();
}

/// Keep my devices / New apps volume: a switch is an entry (old -> new); "Windows defaults" = both off and every app at
/// 100 % (one line per app), applied through the page.
#[test]
fn switches_log_and_windows_defaults_put_everything_to_windows_values() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut a = page();
    click(&mut a, K_KEEP, &g, &mut st);
    let r = rec("keep").expect("an entry");
    assert_eq!((r.label.as_str(), r.was.raw.as_str(), r.now.raw.as_str()), ("Keep my devices", "on", "off"));
    click(&mut a, K_KEEP, &g, &mut st);
    assert!(review(&a, Kind::HowItWas).is_empty(), "switched back: nothing to reset");
    let rv = review(&a, Kind::WindowsDefaults);
    let lines: Vec<String> = rv.lines.iter().map(|l| format!("{} · {}", l.label, l.change_text())).collect();
    assert_eq!(lines[..2], ["Keep my devices · On  →  Off".to_string(), "New apps volume · On · 50 %  →  Off".to_string()]);
    assert!(lines.contains(&"Chrome volume · Muted  →  100 %".to_string()), "{lines:?}");
    assert_eq!(lines.len(), 7, "{lines:?}");
    let res = reset(&mut a, &rv);
    assert!(res.iter().all(|r| r.outcome == crate::undo::Outcome::Ok), "{res:?}");
    wait(&mut a, 700);
    let d = a.describe();
    assert!(d.contains("keep=false newapps=false"), "{d}");
    assert!(d.contains("app Chrome vol=100 muted=false") && d.contains("app Discord vol=100"), "{d}");
    wait(&mut a, 300);
    assert!(review(&a, Kind::WindowsDefaults).is_empty(), "everything at Windows' values");
    svc::set_rules(svc::Rules::default());
    crate::services::shutdown();
}

/// Test pictures (frozen): the drawing's sample log in the page's own review; its Reset changes nothing. A live copy asks
/// the frame's shared review instead.
#[test]
fn the_sample_review_shows_in_test_pictures_only() {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut a = page();
    click(&mut a, sub(K_RESET, "win"), &g, &mut st);
    assert!(a.describe().contains("review=win"));
    click(&mut a, K_RV_GO, &g, &mut st);
    let d = a.describe();
    assert!(d.contains("keep=true newapps=true") && d.contains("review=none"), "{d}");
    assert_eq!(a.st.as_ref().unwrap().toast.as_ref().unwrap().0, "Windows defaults \u{00b7} 3 settings reset");
    a.st.as_mut().unwrap().frozen = false;
    let mut cx = Cx::new(1000.0, false, &g, &mut st).for_page("aud");
    a.event(&Ev::Click(sub(K_RESET, "pc")), &mut cx);
    assert!(matches!(cx.reqs.as_slice(), [crate::ui::cx::Req::Reset(Kind::HowItWas, _)]), "{:?}", cx.reqs);
    drop(cx);
    assert!(a.describe().contains("review=none"));
}

/// Settings › Reset / the uninstaller ask a CLOSED page: `resettable()` opens nothing; the reset goes through the page's
/// own fake (a test never reaches Windows), the switches through the app's rules.
#[test]
fn a_closed_page_resets_through_its_own_fake() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    svc::set_rules(svc::Rules::default());
    let mut a = Audio::new();
    assert!(a.resettable().is_some());
    assert!(a.closed_fake.get().is_none(), "resettable() is cheap: nothing made");
    crate::undo::note("aud", "out.default", "Default output", &Val::new("spk|spk|arctis", "Speakers (Realtek)"), &Val::new("arctis|arctis|arctis", "x"));
    crate::undo::note("aud", "keep", "Keep my devices", &svc::on_val(false), &svc::on_val(true));
    // its Windows defaults read the apps from the same fake (made now: the Output is still the Arctis)
    assert_eq!(review(&a, Kind::WindowsDefaults).lines.len(), 7, "Keep + New apps volume + 5 apps");
    let rv = review(&a, Kind::HowItWas);
    assert_eq!(rv.lines.len(), 2);
    assert_eq!(rv.lines[1].change_text(), "x  →  Speakers (Realtek)", "closed: the last recorded value is now");
    let res = reset(&mut a, &rv);
    assert!(res.iter().all(|r| r.outcome == crate::undo::Outcome::Ok), "{res:?}");
    let f = a.closed_fake.get().expect("made on first use").clone();
    let got = f.with(|f| (f.defaults[&(Flow::Output, bu_audio::Role::Console)].clone(), f.defaults[&(Flow::Output, bu_audio::Role::Communications)].clone()));
    assert_eq!(got, ("spk".to_string(), "arctis".to_string()));
    assert!(!svc::rules().keep);
    svc::set_rules(svc::Rules::default());
    crate::services::shutdown();
}

/// Settings › "Reset the app's own settings" turns the switches back on: that change goes into the change log too.
#[test]
fn the_apps_own_reset_logs_the_switches_it_changes() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    mute::reset_for_test();
    svc::set_rules(svc::Rules { keep: false, new_on: true, new_vol: 0.5 });
    reset_app_settings(true);
    let r = rec("keep").expect("an entry");
    assert_eq!((r.was.raw.as_str(), r.now.raw.as_str()), ("off", "on"));
    assert!(rec("newapps").is_none(), "unchanged: no entry");
    svc::set_rules(svc::Rules::default());
    mute::reset_for_test();
    crate::services::shutdown();
}

#[test]
fn mic_icon_and_mute_settings() {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut a = page();
    click(&mut a, K_MIC, &g, &mut st);
    assert!(a.describe().contains("micmuted=true"));
    click(&mut a, K_MIC, &g, &mut st);
    assert!(a.describe().contains("micmuted=false"));
    click(&mut a, K_MML, &g, &mut st);
    let d = a.describe();
    assert!(d.contains("mute=true") && d.contains("mute on=false"), "{d}");
    // the popup's switch, then the dialog's x closes it
    click(&mut a, key("aud.mm.on"), &g, &mut st);
    assert!(a.describe().contains("mute on=true"));
    // the mute sound is OFF until the user switches it on (Order 046), at 5 %
    assert!(a.describe().contains("sound=false") && a.describe().contains("vol=5"), "{}", a.describe());
    click(&mut a, key("aud.mm.snd"), &g, &mut st);
    assert!(a.describe().contains("sound=true"));
    // Order 092: the Volume slider works with the sound on and "Change" never opened (it sits right under the switch):
    // a press at the middle of its 150 px track = 50 %, and letting go keeps it
    {
        let mut cx = Cx::new(1000.0, false, &g, &mut st).for_page("aud");
        let r = (300.0, 200.0, 150.0, 20.0);
        a.event(&Ev::Press(key("aud.mm.vol"), r.0 + 8.0 + 0.5 * (r.2 - 16.0), 210.0, r), &mut cx);
        a.event(&Ev::Release(key("aud.mm.vol")), &mut cx);
    }
    assert!(a.describe().contains("vol=50"), "{}", a.describe());
    click(&mut a, key("aud.mm.snd"), &g, &mut st);
    assert!(a.describe().contains("sound=false"));
    click(&mut a, sub(mute::K_MM, "x"), &g, &mut st);
    assert!(a.describe().contains("mute=false"));
    mute::reset_for_test();
}

#[test]
fn old_saved_mute_sound_is_turned_off_once_and_a_new_one_is_kept() {
    use bu_micmute::{Sound, SoundSettings};
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    let m = mute::service(true, false, false);
    let put = |l: &[&str]| {
        let v: Vec<String> = l.iter().map(|s| s.to_string()).collect();
        crate::services::with(|sv| sv.store.set_list(crate::settings::Scope::Page("aud"), "mm", &v));
    };
    // the old format: sound on, Soft click / Chime, 60 % -> kept picks, sound OFF, volume 5 %
    put(&["on=1", "snd=1,0,3,60"]);
    mute::load(&m);
    assert_eq!(m.sound(), SoundSettings { enabled: false, on_mute: Sound::SoftClick, on_unmute: Sound::Chime, volume: 5 });
    // the new format is read as saved
    put(&["on=1", "snd2=1,1,2,30"]);
    mute::load(&m);
    assert_eq!(m.sound(), SoundSettings { enabled: true, on_mute: Sound::BlipDown, on_unmute: Sound::BlipUp, volume: 30 });
    m.set_sound(SoundSettings::default());
    mute::reset_for_test();
    crate::services::shutdown();
}

#[test]
fn closing_drops_everything() {
    let mut a = page();
    a.close();
    assert!(a.st.is_none());
    assert_eq!(a.describe(), "");
}

#[test]
fn mic_icon_shows_like_the_drawing_and_never_on_screen_in_tests() {
    micicon::reset_for_tests();
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut a = page();
    // Mute settings open with Mic mute on: the preview shows it (the drawing's "Live" pill)
    click(&mut a, K_MML, &g, &mut st);
    assert!(!micicon::showing());
    click(&mut a, key("aud.mm.on"), &g, &mut st);
    assert!(micicon::showing(), "the preview shows while Mute settings is open with Mic mute on");
    // Off hides it
    click(&mut a, crate::ui::el::idx(key("aud.mm.icon"), 0), &g, &mut st);
    assert!(!micicon::showing());
    click(&mut a, crate::ui::el::idx(key("aud.mm.icon"), 1), &g, &mut st);
    assert!(micicon::showing());
    // closing the popup ends the preview; without a key "When it changes" has nothing to flash for
    click(&mut a, sub(mute::K_MM, "x"), &g, &mut st);
    assert!(!micicon::showing());
    click(&mut a, K_MIC, &g, &mut st);
    assert!(!micicon::showing());
    // never a window or a timer in a test copy
    assert!(!micicon::window_exists() && !micicon::timer_armed());
    mute::reset_for_test();
}

#[test]
fn mic_icon_flashes_on_a_change_when_a_key_is_set() {
    micicon::reset_for_tests();
    mute::reset_for_test();
    let look = micicon::Look { muted: true, style: 0, size: 1, op: 1.0, moving: false };
    let want = micicon::Want { active: true, always: false, preview: false, moving: false };
    micicon::update(want, look, micicon::quick(2), false);
    assert!(!micicon::showing(), "nothing changed yet");
    micicon::update(want, look, micicon::quick(2), true);
    assert!(micicon::showing(), "a change flashes it");
    micicon::update(micicon::Want { always: true, ..want }, look, micicon::quick(2), false);
    assert!(micicon::showing());
    micicon::update(micicon::Want { active: false, ..want }, look, micicon::quick(2), false);
    assert!(!micicon::showing());
}

#[test]
fn mic_icon_spots_and_snapping() {
    // the six quick spots: 24 px from the work area's edges, the middle centred
    let work = (0.0, 0.0, 1920.0, 1032.0);
    assert_eq!(micicon::place(micicon::quick(2), work, (96.0, 34.0)), (1800.0, 24.0));
    assert_eq!(micicon::place(micicon::quick(1), work, (96.0, 34.0)), (912.0, 24.0));
    assert_eq!(micicon::place(micicon::quick(3), work, (96.0, 34.0)), (24.0, 974.0));
    // a drag near the right edge snaps to it
    let s = micicon::snap_spot(1805.0, 300.0, (96.0, 34.0), (1920.0, 1032.0));
    assert_eq!((s.h, s.dx), ('R', 24.0));
    // pop spring ends at 1
    assert_eq!(micicon::pop_scale(400.0, false), 1.0);
    assert!((micicon::pop_scale(0.0, false) - 0.85).abs() < 1e-3);
}

/// The proof picture of the icon over the drawing's desktop (BU_P_DESK = desk.png, BU_P_OUT = the picture).
#[test]
#[ignore]
fn mic_icon_picture() {
    let desk = std::env::var("BU_P_DESK").expect("BU_P_DESK");
    let out = std::env::var("BU_P_OUT").expect("BU_P_OUT");
    // WIC needs COM on this thread
    let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
    let d = crate::png::load_png(&desk).expect("desk");
    let look = micicon::Look { muted: false, style: 0, size: 1, op: 1.0, moving: false };
    let p = micicon::render(&look, micicon::quick(2), &d, 1.0).expect("render");
    crate::png::save_png(&p, &out).expect("save");
}

/// The mute key is the keys manager's (registered at app start, works with the menu closed, only while Mic mute is on),
/// and Mic mute's settings + the two switches are saved and read back at the next start (the owner Oct 8: settings survive).
#[test]
fn the_mute_key_works_with_the_menu_closed_and_choices_survive_a_restart() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    mute::reset_for_test();
    let names: Vec<String> = crate::services::with(|s| s.action_list().into_iter().map(|a| a.id).collect()).unwrap();
    for a in [mute::A_ONE, mute::A_MUTE, mute::A_UNMUTE] {
        assert!(names.iter().any(|n| n == a), "{a} registered at start: {names:?}");
    }
    let m = mute::service(true, false, false);
    let before = mute::mic_muted(&m);
    // Mic mute off: the key does nothing
    mute::on_key(mute::A_ONE, true, true);
    assert_eq!(mute::mic_muted(&m), before);
    // on (the popup's switch), one key: it toggles; the separate keys don't
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut a = Audio::new();
    a.open(&env(), 0.0);
    click(&mut a, K_MML, &g, &mut st);
    click(&mut a, key("aud.mm.on"), &g, &mut st);
    mute::on_key(mute::A_MUTE, true, true);
    assert_eq!(mute::mic_muted(&m), before, "a separate key does nothing in one-key mode");
    mute::on_key(mute::A_ONE, true, true);
    assert_eq!(mute::mic_muted(&m), !before, "the one key toggles");
    mute::on_key(mute::A_ONE, false, true);
    assert_eq!(mute::mic_muted(&m), !before, "the release does nothing");
    // saved: back to defaults in memory, then the start-up read brings it back
    svc::set_rules(svc::Rules { keep: false, new_on: true, new_vol: 0.3 });
    a.close();
    mute::reset_for_test();
    svc::set_rules_mem(svc::Rules::default());
    let m = mute::service(true, false, false);
    mute::load(&m);
    svc::load_rules();
    assert!(mute::settings().on, "Mic mute on came back");
    assert_eq!(svc::rules(), svc::Rules { keep: false, new_on: true, new_vol: 0.3 });
    crate::services::shutdown();
    mute::reset_for_test();
    svc::set_rules_mem(svc::Rules::default());
}

/// End review: the uninstaller's undo (no window, no `Page::background`) judges "Keep my devices" by the SAVED switch -
/// a switch the user already turned back off offers nothing.
#[test]
fn the_uninstallers_undo_reads_the_saved_switches() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    crate::services::with(|s| {
        let _ = s.store.set_bool(crate::settings::Scope::Page("aud"), "keep", false);
        let _ = crate::undo::record(&mut s.store, "aud", svc::KEEP, "Keep my devices", &svc::on_val(false), &svc::on_val(false));
    });
    svc::set_rules_mem(svc::Rules::default());
    load_for_undo();
    let a = Audio::new();
    assert_eq!(crate::undo::Resettable::current(&a, svc::KEEP).map(|v| v.raw), Some(svc::on_val(false).raw));
    assert!(review(&a, Kind::HowItWas).is_empty(), "already off: nothing to offer");
    crate::services::shutdown();
}

#[test]
fn mic_icon_show_on_picks_the_monitors() {
    use micicon::{list_index, monitor_labels, order_monitors, place_on, stored, targets, Mon, MON_ALL};
    // a common setup: a 3440 x 1440 main monitor and a 1920 x 1080 one at 125 % left of it (taskbar 48 px at the bottom)
    let main = Mon { work: (0, 0, 3440, 1392), scale: 1.0, primary: true };
    let left = Mon { work: (-1920, 180, 1920, 1020), scale: 1.25, primary: false };
    let right = Mon { work: (3440, 0, 2560, 1392), scale: 1.0, primary: false };
    let mons = order_monitors(vec![right, main, left]);
    assert_eq!(mons, vec![main, left, right], "Main first, then left to right");
    assert_eq!(monitor_labels(1), ["Main"]);
    assert_eq!(monitor_labels(3), ["Main", "Monitor 2", "Monitor 3", "All monitors"]);
    // picking a row saves it; "All monitors" is saved as MON_ALL
    assert_eq!(stored(1, 3), 1);
    assert_eq!(stored(3, 3), MON_ALL);
    assert_eq!(list_index(MON_ALL, 3), 3);
    assert_eq!(list_index(3, 3), 0, "a saved Monitor 4 with 3 monitors is gone: Main, never All");
    assert_eq!(list_index(2, 2), 0, "a saved Monitor 3 after one was unplugged: Main, never All");
    // a save from before Order 040 (the list row): All = the monitor count then
    assert_eq!(micicon::from_old_row(3, 3), MON_ALL);
    assert_eq!(micicon::from_old_row(1, 3), 1);
    assert_eq!(micicon::from_old_row(0, 1), 0);
    assert_eq!(list_index(2, 1), 0, "a monitor that is gone falls back to Main");
    assert_eq!(list_index(MON_ALL, 1), 0);
    assert_eq!(targets(0, 3), [0]);
    assert_eq!(targets(1, 3), [1]);
    assert_eq!(targets(MON_ALL, 3), [0, 1, 2]);
    assert_eq!(targets(MON_ALL, 1), [0]);
    assert_eq!(targets(5, 2), [0]);
    // the same spot (top right, 24 px) on each monitor, in its own pixels
    let spot = micicon::quick(2);
    let size = (96.0, 34.0);
    assert_eq!(place_on(&main, spot, size), (3440 - 24 - 96, 24));
    assert_eq!(place_on(&left, spot, size), (-(24 + 96) * 5 / 4, 180 + 24 * 5 / 4), "125 %: 24 + 96 DIPs = 150 px");
    assert_eq!(place_on(&right, micicon::quick(3), size), (3440 + 24, 1392 - 24 - 34));
}

/// Order 042 proof picture (run on purpose: `cargo test -p bu-app proof_042 -- --ignored`): the Audio page as the frame
/// composites it (static pass, then the live meters over it) - the Output / Input knobs must cover their level pills.
#[test]
#[ignore]
fn proof_042_audio_slider_knob_over_its_line() {
    if std::env::var("BU_PIC_OUT").is_err() {
        return;
    }
    let mut a = page();
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let _ = lay(&mut a, &g, &mut st, 0.0);
    let mut cx = Cx::new(2000.0, false, &g, &mut st);
    let kids = a.build(&mut cx);
    let root = El::block().w(600.0).h(300.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
    crate::ui::lay::proof_png(root, 600.0, 300.0, 2.0, "audio_slider.png");
}

/// Order 045 item 1: separate keys - once the Mute key is taken (and there is no Unmute key yet) the Unmute field starts
/// listening by itself (`commitCap`, L3383).
#[test]
fn separate_keys_the_unmute_field_listens_once_mute_is_taken() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    crate::services::with(mute::register);
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut a = page();
    click(&mut a, K_MML, &g, &mut st);
    click(&mut a, key("aud.mm.on"), &g, &mut st);
    click(&mut a, key("aud.mm.sep"), &g, &mut st);
    click(&mut a, key("aud.mm.key.mute"), &g, &mut st);
    let built = |a: &mut Audio, st: &mut State, now: f64| {
        let mut cx = Cx::new(now, false, &g, st).for_page("aud");
        let _ = a.build(&mut cx);
        let _ = a.popup(&mut cx);
        cx.key_field(mute::A_UNMUTE).1.is_some()
    };
    assert!(!built(&mut a, &mut st, 1100.0), "only the Mute field listens");
    // (a key message reads the real modifiers: only when none is held)
    if crate::keys::real::mods_now().is_empty() {
        assert!(crate::services::key_message(true, 0x77, 0), "F8 goes to the listening field");
        let _ = crate::services::key_message(false, 0x77, 0);
        assert!(built(&mut a, &mut st, 1200.0), "the Unmute field listens by itself");
    }
    crate::services::with(|s| s.stop_listening());
    mute::reset_for_test();
    crate::services::shutdown();
}

/// Order 045 proof picture (`BU_PIC_OUT=<folder> cargo test -p bu-app proof_045 -- --ignored --test-threads=1`): Mute
/// settings with separate keys right after the Mute key was taken - the Unmute field listening by itself.
#[test]
#[ignore]
fn proof_045_unmute_listens() {
    if std::env::var("BU_PIC_OUT").is_err() || !crate::keys::real::mods_now().is_empty() {
        return;
    }
    for (light, theme) in [(false, "dark"), (true, "light")] {
        crate::ui::set_light(light);
        crate::services::init(windows::Win32::Foundation::HWND::default(), true);
        crate::services::with(mute::register);
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut a = page();
        click(&mut a, K_MML, &g, &mut st);
        click(&mut a, key("aud.mm.on"), &g, &mut st);
        click(&mut a, key("aud.mm.sep"), &g, &mut st);
        click(&mut a, key("aud.mm.key.mute"), &g, &mut st);
        {
            let mut cx = Cx::new(1100.0, false, &g, &mut st).for_page("aud");
            let _ = a.popup(&mut cx);
        }
        let _ = crate::services::key_message(true, 0x77, 0);
        let _ = crate::services::key_message(false, 0x77, 0);
        let mut cx = Cx::new(3000.0, false, &g, &mut st).for_page("aud");
        let kids = a.build(&mut cx);
        let pop = a.popup(&mut cx);
        let pg = El::block().abs(0.0, crate::ui::PAGE_TOP, f32::NAN, f32::NAN).w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
        let root = El::block().w(600.0).h(crate::ui::WIN_H).child(pg).children(pop);
        crate::ui::lay::proof_png(root, 600.0, crate::ui::WIN_H, 1.5, &format!("01_unmute_listens_{theme}.png"));
        drop(cx);
        crate::services::with(|s| s.stop_listening());
        mute::reset_for_test();
        crate::services::shutdown();
    }
    crate::ui::set_light(false);
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

/// Order 047: Audio's reset (Windows defaults: both switches off, every app at 100 %) is read and put back on the
/// review's worker thread (a real PC: a Core Audio service made per call); the switches are saved on the menu's thread
/// when it has ended. Same lines, same results.
#[test]
fn the_reset_review_reads_and_puts_back_off_the_menus_thread() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    svc::set_rules(svc::Rules::default());
    let mut a = page();
    let (rv, res) = reset_off_the_menu(&mut a, Kind::WindowsDefaults);
    let lines: Vec<String> = rv.lines.iter().map(|l| format!("{} · {}", l.label, l.change_text())).collect();
    assert_eq!(lines[..2], ["Keep my devices · On  →  Off".to_string(), "New apps volume · On · 50 %  →  Off".to_string()]);
    assert!(lines.contains(&"Chrome volume · Muted  →  100 %".to_string()), "{lines:?}");
    assert_eq!(lines.len(), 7, "{lines:?}");
    assert!(res.iter().all(|r| r.outcome == crate::undo::Outcome::Ok), "{res:?}");
    assert!(!svc::rules().keep && !svc::rules().new_on, "the switches saved by reset_done");
    wait(&mut a, 700);
    let d = a.describe();
    assert!(d.contains("keep=false newapps=false"), "{d}");
    assert!(d.contains("app Chrome vol=100 muted=false") && d.contains("app Discord vol=100"), "{d}");
    svc::set_rules(svc::Rules::default());
    crate::services::shutdown();
}

/// Order 047: switching the default Output device (Core Audio's policy config, all three roles) never holds the menu -
/// the pick only hands it to the page's worker and returns within one frame; the fake PC has the new default after.
/// (The fake audio layer has no slow mode: the proof is the pick's own time and the change made by the worker.)
#[test]
fn a_default_output_pick_never_holds_the_menu() {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut a = page();
    click(&mut a, K_OUT_PICK, &g, &mut st);
    let mut cx = Cx::new(1000.0, false, &g, &mut st).for_page("aud");
    crate::offui::assert_quick("the Output pick", || a.event(&Ev::Click(k_dev(0)), &mut cx));
    drop(cx);
    let t0 = std::time::Instant::now();
    while fake_default(&a, Flow::Output) != "spk" && t0.elapsed().as_secs() < 5 {
        wait(&mut a, 20);
    }
    assert_eq!(fake_default(&a, Flow::Output), "spk", "the worker switched it");
}

/// Order 055: the meters are DATA - a frozen picture (nothing moves) asks for no frame, and a tick that comes before the
/// next look at the worker does nothing at all.
#[test]
fn a_tick_with_nothing_new_asks_for_no_frame_and_no_wake() {
    let mut a = page();
    let t0 = a.st.as_ref().unwrap().next_poll;
    assert!(!a.tick(t0), "the first look finds nothing new in a frozen picture");
    let np = a.st.as_ref().unwrap().next_poll;
    assert!(np >= t0 + SILENT_MS, "no meter moves: the next look is at the silent rate ({np} after {t0})");
    // (the frames of a hover / a scroll pass here many times between two looks)
    assert!(!a.tick(t0 + 1.0) && !a.tick(np - 5.0));
    assert_eq!(a.st.as_ref().unwrap().next_poll, np, "no look was made");
    assert_eq!(a.wake_at(t0), None, "a frozen picture never wakes the menu");
    assert!(!step(&mut a));
}

/// Order 055: with sound a meter steps every 33 ms - the ticks between two steps are no frame, and `wake_at` names the next
/// step; a step that only moves the meters is a live-pass repaint (no build).
#[test]
fn meters_step_every_33_ms_and_the_ticks_between_ask_for_no_frame() {
    mute::reset_for_test();
    svc::set_rules(svc::Rules::default());
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut a = Audio::new();
    a.open(&Env { test: true, real_read: false, frozen: false, rm: false, ..Env::default() }, 0.0);
    lay(&mut a, &g, &mut st, 0.0);
    // the fake's music starts: the rows leave "quiet" (their fade is a build), the meters rise
    assert!(a.tick(40.0), "a moving meter wants a frame");
    assert_eq!(a.wake_at(41.0), Some(40.0 + STEP_MS));
    let last = a.st.as_ref().unwrap().last;
    // inside the step: no frame, no new level, the same wake
    assert!(!a.tick(50.0) && !a.tick(70.0));
    assert_eq!(a.st.as_ref().unwrap().last, last, "the levels did not step");
    assert_eq!(a.wake_at(50.0), Some(40.0 + STEP_MS));
    // the next step: only the meters moved
    assert!(a.tick(40.0 + STEP_MS));
    assert!(a.live_only(), "the live pass repaints the meters; nothing is built");
    assert_eq!(a.st.as_ref().unwrap().last, 40.0 + STEP_MS);
    assert_eq!(a.wake_at(80.0), Some(40.0 + 2.0 * STEP_MS));
}

/// Order 081: the Output row's speaker icon mutes / unmutes the default output device - on the page's FAKE worker only
/// (a test never touches the PC's device); the icon follows what the device really says, and the meter pill dims.
#[test]
fn speaker_icon_mutes_the_default_output() {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut a = page();
    let out_muted = |a: &Audio| {
        let id = a.st.as_ref().unwrap().out.as_ref().map(|o| o.0.clone()).unwrap();
        a.st.as_ref().unwrap().svc.with_fake(|f| f.volumes[&id].muted).unwrap()
    };
    assert!(!out_muted(&a) && a.describe().contains("outmuted=false"));
    let vol0 = a.st.as_ref().unwrap().out.as_ref().unwrap().1;
    click(&mut a, K_SPK, &g, &mut st);
    assert!(a.describe().contains("outmuted=true"), "the icon flips at once");
    let t0 = std::time::Instant::now();
    while !out_muted(&a) && t0.elapsed().as_secs() < 5 {
        wait(&mut a, 20);
    }
    assert!(out_muted(&a), "the worker muted the fake device");
    wait(&mut a, 700);
    assert!(a.describe().contains("outmuted=true"), "the icon follows the device: {}", a.describe());
    assert_eq!(a.st.as_ref().unwrap().out.as_ref().unwrap().1, vol0, "the volume is left alone");
    // the mic is a different device: untouched
    assert!(a.describe().contains("micmuted=false"));
    click(&mut a, K_SPK, &g, &mut st);
    let t0 = std::time::Instant::now();
    while out_muted(&a) && t0.elapsed().as_secs() < 5 {
        wait(&mut a, 20);
    }
    assert!(!out_muted(&a), "unmuted again");
    wait(&mut a, 700);
    assert!(a.describe().contains("outmuted=false"));
    // a mute made elsewhere (Windows' own mixer) shows on the icon too
    let id = a.st.as_ref().unwrap().out.as_ref().unwrap().0.clone();
    a.st.as_ref().unwrap().svc.with_fake(|f| f.volumes.get_mut(&id).unwrap().muted = true);
    let t0 = std::time::Instant::now();
    while !a.describe().contains("outmuted=true") && t0.elapsed().as_secs() < 5 {
        wait(&mut a, 100);
    }
    assert!(a.describe().contains("outmuted=true"), "{}", a.describe());
    // the tooltip / icon of the built page
    let l = lay(&mut a, &g, &mut st, 0.0);
    assert!(l.rect_of(K_SPK).is_some());
}

/// Order 081 picture (`BU_PIC_OUT=<folder> cargo test -p bu-app speaker_muted_picture -- --ignored`): the Audio page with the
/// Output speaker muted (red icon with the slash, the level pill dimmed) beside the unmuted Input row.
#[test]
#[ignore]
fn speaker_muted_picture() {
    if std::env::var("BU_PIC_OUT").is_err() {
        return;
    }
    let mut a = page();
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let _ = lay(&mut a, &g, &mut st, 0.0);
    click(&mut a, K_SPK, &g, &mut st);
    let _ = lay(&mut a, &g, &mut st, 1000.0);
    let mut cx = Cx::new(2000.0, false, &g, &mut st);
    let kids = a.build(&mut cx);
    let root = El::block().w(600.0).h(300.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
    crate::ui::lay::proof_png(root, 600.0, 300.0, 2.0, "audio_speaker_muted.png");
}

/// Order 092 picture (`BU_PIC_OUT=<folder> cargo test -p bu-app proof_092 -- --ignored`): Mute settings with the mute sound
/// switched on - the Volume slider sits right under the switch, "Change" still closed.
#[test]
#[ignore]
fn proof_092_volume_under_the_switch() {
    if std::env::var("BU_PIC_OUT").is_err() {
        return;
    }
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    crate::services::with(mute::register);
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut a = page();
    click(&mut a, K_MML, &g, &mut st);
    click(&mut a, key("aud.mm.on"), &g, &mut st);
    {
        let mut cx = Cx::new(1000.0, false, &g, &mut st).for_page("aud");
        let _ = a.popup(&mut cx);
    }
    click(&mut a, key("aud.mm.snd"), &g, &mut st);
    let mut cx = Cx::new(3000.0, false, &g, &mut st).for_page("aud");
    let _ = a.popup(&mut cx);
    let mut cx = Cx::new(4000.0, false, &g, &mut st).for_page("aud");
    let kids = a.build(&mut cx);
    let pop = a.popup(&mut cx);
    let pg = El::block().abs(0.0, crate::ui::PAGE_TOP, f32::NAN, f32::NAN).w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
    let root = El::block().w(600.0).h(crate::ui::WIN_H).child(pg).children(pop);
    crate::ui::lay::proof_png(root, 600.0, crate::ui::WIN_H, 1.5, "092_mute_volume.png");
    drop(cx);
    mute::reset_for_test();
    crate::services::shutdown();
}
