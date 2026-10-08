//! Display page tests (Order 019) - against the FAKE runtime only (bu-display's FakeDisplayOs with the drawing's
//! sample): nothing here reads or changes a real monitor, and no window is ever shown.

use std::sync::Arc;
use std::time::Duration;

use bu_display::autoswitch::AppEvent;
use bu_display::fake::FakeCall;
use bu_display::{GpuScaling, MonitorId, RefreshRate, Vcp};

use super::rt::{AnyOs, Rt};
use super::*;
use crate::gfx::Gfx;
use crate::ui::cx::State;

fn env() -> Env {
    Env { test: true, ..Env::default() }
}

struct T {
    p: Display,
    g: Gfx,
    st: State,
    now: f64,
}

impl T {
    fn new() -> T {
        T::with(Rt::fake_sample())
    }
    fn with(rt: Arc<Rt>) -> T {
        let mut p = Display::default();
        p.open_rt(rt, &env(), 0.0);
        // Order 047: the monitors are read on the page's worker
        p.settle(0.0);
        let mut t = T { p, g: Gfx::new(1.0), st: State::default(), now: 0.0 };
        t.build();
        t
    }
    fn build(&mut self) -> Vec<El> {
        self.now += 16.0;
        let mut cx = Cx::new(self.now, false, &self.g, &mut self.st);
        let v = self.p.build(&mut cx);
        let _ = self.p.popup(&mut cx);
        v
    }
    fn ev(&mut self, e: Ev) {
        self.now += 16.0;
        let mut cx = Cx::new(self.now, false, &self.g, &mut self.st);
        self.p.event(&e, &mut cx);
        // Order 047: its worker job (a mode change, a read) answered and landed
        let now = self.now;
        self.p.settle(now);
        self.build();
    }
    fn click(&mut self, k: Key) {
        self.ev(Ev::Click(k));
    }
    fn rt(&self) -> Arc<Rt> {
        self.p.rt.clone().unwrap()
    }
    fn calls(&self) -> Vec<FakeCall> {
        let rt = self.rt();
        let s = rt.svc.lock().unwrap();
        match s.os() {
            AnyOs::Fake(f) => f.calls.clone(),
            #[allow(unreachable_patterns)]
            _ => vec![],
        }
    }
    fn toast(&self) -> String {
        self.p.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default()
    }
    fn fields(&self) -> Fields {
        self.p.f.unwrap()
    }
    fn type_into(&mut self, fld: Fld, s: &str) {
        self.ev(Ev::Press(fld.key(), 0.0, 0.0, (300.0, 178.0, 58.0, 32.0)));
        for c in s.chars() {
            self.ev(Ev::Char(fld.key(), c));
        }
        self.ev(Ev::Key(fld.key(), 0x0D));
    }
}

fn texts(e: &El, out: &mut Vec<String>) {
    if let crate::ui::el::Content::Text(t) = &e.content {
        out.push(t.s.clone());
    }
    for c in &e.children {
        texts(c, out);
    }
}

fn page_text(t: &mut T) -> String {
    let mut v = Vec::new();
    for e in t.build() {
        texts(&e, &mut v);
    }
    v.join("|")
}

const DELL: &str = "fake-dell";
const LG: &str = "fake-lg";

#[test]
fn opens_on_the_drawings_sample_without_changing_anything() {
    let mut t = T::new();
    let f = t.fields();
    assert_eq!((f.w, f.h, f.sc), (1920, 1080, GpuScaling::KeepAspect));
    assert_eq!(f.hz, RefreshRate::new(164_950, 1000));
    let s = page_text(&mut t);
    for want in ["Display", "1", "DELL 27″", "LG 24″", "Width", "Height", "Hz", "1920", "1080", "165", "Apply", "Scaling", "Keep aspect", "Main display", "Picture", "Brightness", "70 %", "Contrast", "50 %", "Vibrance", "Presets", "1920 × 1080 · 165 Hz", " · Keep aspect", "1440 × 1080 · 165 Hz", "1280 × 960 · 144 Hz", "Switch automatically", "while an app is running", "VALORANT", "Counter-Strike 2", "Fortnite", "Add app"] {
        assert!(s.split('|').any(|x| x == want), "missing {want:?} in {s}");
    }
    // v21/v22: no OBS setting on Display; no personal names
    assert!(!s.contains("OBS") && !s.contains("the owner"), "{s}");
    assert!(t.calls().is_empty(), "opening changed something: {:?}", t.calls());
}

#[test]
fn apply_shows_the_keep_bar_and_keep_stores_it() {
    let mut t = T::new();
    t.click(idx(K_SCALE, 0)); // Stretch
    t.type_into(Fld::W, "1440");
    t.click(K_APPLY);
    let id = MonitorId(DELL.into());
    let want = bu_display::Mode { width: 1440, height: 1080, refresh: RefreshRate::new(164_950, 1000), scaling: GpuScaling::Stretch };
    assert_eq!(t.calls(), vec![FakeCall::ApplyMode(id.clone(), want)]);
    assert!(t.p.cfm_on.is_some(), "keep bar not shown");
    assert!(t.rt().svc.lock().unwrap().pending().is_some());
    t.click(K_KEEP);
    assert_eq!(t.calls().last(), Some(&FakeCall::SaveCurrent(id.clone())));
    assert_eq!(t.toast(), "Kept 1440 × 1080 · 165 Hz");
    assert!(t.p.cfm_on.is_none());
    // the kept mode = the 1440 × 1080 Stretch chip: a rule may now use it on this monitor
    let rt = t.rt();
    let st = rt.store.lock().unwrap();
    let p = st.presets.items().iter().find(|p| p.width == 1440).unwrap();
    assert!(st.presets.is_kept_on(p.id, &id));
}

#[test]
fn revert_goes_back_and_says_so() {
    let mut t = T::new();
    t.type_into(Fld::H, "960");
    t.type_into(Fld::W, "1280");
    t.click(K_APPLY);
    t.click(K_REVERT);
    let id = MonitorId(DELL.into());
    let back = bu_display::Mode { width: 1920, height: 1080, refresh: RefreshRate::new(164_950, 1000), scaling: GpuScaling::KeepAspect };
    assert_eq!(t.calls().last(), Some(&FakeCall::ApplyMode(id, back)));
    assert_eq!(t.toast(), "Back to 1920 × 1080 · 165 Hz");
    assert_eq!(t.fields().w, 1920);
}

#[test]
fn the_countdown_reverts_even_with_the_menu_closed() {
    let rt = Rt::fake_sample_keep(Duration::from_millis(150));
    let mut t = T::with(rt.clone());
    t.type_into(Fld::W, "1280");
    t.type_into(Fld::H, "720");
    t.click(K_APPLY);
    t.p.close(); // the page is dropped; the runtime counts on
    std::thread::sleep(Duration::from_millis(600));
    let s = rt.svc.lock().unwrap();
    assert!(s.pending().is_none());
    assert_eq!(s.monitor(&MonitorId(DELL.into())).unwrap().current.width, 1920);
    drop(s);
    assert_eq!(rt.take_notes(), vec!["Not kept, back to 1920 × 1080 · 165 Hz".to_string()]);
}

#[test]
fn reopening_while_a_change_waits_shows_the_bar_again() {
    let rt = Rt::fake_sample();
    let mut t = T::with(rt.clone());
    t.type_into(Fld::W, "1280");
    t.type_into(Fld::H, "960");
    t.click(K_APPLY);
    t.p.close();
    let t2 = T::with(rt);
    assert!(t2.p.cfm_on.is_some());
}

#[test]
fn typed_hz_snaps_to_a_rate_the_monitor_reports() {
    let mut t = T::new();
    t.type_into(Fld::Hz, "150");
    assert_eq!(t.fields().hz, RefreshRate::new(143_970, 1000));
    assert!(t.p.snap[2].is_some(), "no snap cue");
    t.type_into(Fld::Hz, "999");
    assert_eq!(t.fields().hz, RefreshRate::new(164_950, 1000));
    // a real rate typed exactly: no cue
    let mut t = T::new();
    t.type_into(Fld::Hz, "120");
    assert_eq!(t.fields().hz, RefreshRate::new(120_000, 1000));
    assert!(t.p.snap[2].is_none());
}

#[test]
fn the_hz_field_lists_only_the_monitors_rates() {
    let mut t = T::new();
    t.ev(Ev::Press(K_HZ, 0.0, 0.0, (312.797, 178.0, 58.0, 32.0)));
    let m = t.p.menu.as_ref().unwrap();
    assert_eq!((m.kind, m.r), (MenuKind::Rates, (312.797, 178.0, 58.0, 32.0)));
    // the list opens under the field (placeMenu: left edge, top + height + 4), whole pixels
    let (x, y, _, _) = mitems::place(Place::Under(m.r.0, m.r.1, m.r.2, m.r.3), 150.0, 7.0 * 26.0 + 10.0);
    assert_eq!((x, y), (313.0, 214.0));
    let labels: Vec<String> = t
        .p
        .menu_items(MenuKind::Rates)
        .into_iter()
        .filter_map(|i| match i {
            MItem::Item { label, .. } => Some(label),
            MItem::Sep => None,
        })
        .collect();
    assert_eq!(labels, ["165 Hz", "144 Hz", "120 Hz", "119.88 Hz", "100 Hz", "60 Hz", "59.94 Hz"]);
    t.click(idx(K_MENU, 3));
    assert_eq!(t.fields().hz, RefreshRate::new(119_880, 1000));
    assert!(t.p.menu.is_none());
}

/// Order 045 item 4: the wheel over a field steps it like the arrows (no click needed first).
#[test]
fn the_wheel_steps_the_fields() {
    let mut t = T::new();
    let w0 = t.fields().w;
    t.ev(Ev::Wheel(K_W, 1));
    assert!(t.fields().w > w0, "one notch up");
    t.ev(Ev::Wheel(K_W, -1));
    assert_eq!(t.fields().w, w0, "and back");
    t.ev(Ev::Wheel(K_HZ, -1));
    assert_eq!(t.fields().hz, RefreshRate::new(143_970, 1000));
}

#[test]
fn arrow_keys_walk_the_common_values() {
    let mut t = T::new();
    t.ev(Ev::Press(K_W, 0.0, 0.0, (0.0, 0.0, 66.0, 32.0)));
    t.ev(Ev::Key(K_W, 0x26));
    assert_eq!(t.fields().w, 2560);
    t.ev(Ev::Key(K_W, 0x28));
    t.ev(Ev::Key(K_W, 0x28));
    assert_eq!(t.fields().w, 1680);
    t.ev(Ev::Press(K_HZ, 0.0, 0.0, (0.0, 0.0, 58.0, 32.0)));
    t.ev(Ev::Key(K_HZ, 0x28));
    assert_eq!(t.fields().hz, RefreshRate::new(143_970, 1000));
    t.ev(Ev::Key(K_HZ, 0x1B));
    assert!(t.p.edit.is_none());
}

#[test]
fn main_display_moves_and_the_main_one_cant_be_switched_off() {
    let mut t = T::new();
    t.click(K_MAIN); // DELL is main: only a nudge
    assert!(t.calls().is_empty());
    assert!(!t.p.nudges.is_empty());
    t.click(idx(K_MON, 1)); // LG
    assert_eq!(t.p.mons[t.p.sel].name, "LG 24GL600F");
    t.click(K_MAIN);
    assert_eq!(t.calls(), vec![FakeCall::SetMain(MonitorId(LG.into()))]);
    assert_eq!(t.toast(), "Main display: 2 · LG 24GL600F");
}

#[test]
fn a_monitor_without_ddc_greys_out_brightness_and_contrast() {
    let mut t = T::new();
    t.click(idx(K_MON, 1));
    let s = page_text(&mut t);
    assert!(s.contains("LG 24″ doesn’t answer · turn on DDC/CI in the monitor’s own menu."), "{s}");
    // DELL answers: no footer
    t.click(idx(K_MON, 0));
    let s = page_text(&mut t);
    assert!(!s.contains("doesn’t answer"), "{s}");
}

#[test]
fn picture_sliders_go_to_the_monitor() {
    let mut t = T::new();
    t.ev(Ev::Press(K_BRI, 340.0, 0.0, (332.0, 0.0, 168.0, 20.0)));
    t.ev(Ev::Drag(K_BRI, 500.0, 0.0, (332.0, 0.0, 168.0, 20.0)));
    t.ev(Ev::Press(K_VIB, 416.0, 0.0, (332.0, 0.0, 168.0, 20.0)));
    std::thread::sleep(Duration::from_millis(300));
    let c = t.calls();
    assert_eq!(c.iter().rfind(|c| matches!(c, FakeCall::DdcSet(_, Vcp::Brightness, _))), Some(&FakeCall::DdcSet(MonitorId(DELL.into()), Vcp::Brightness, 100)));
    // 416 = the middle of the track: vibrance 50 % = the driver's normal level, already set -> nothing sent
    assert!(!c.iter().any(|c| matches!(c, FakeCall::VibranceSet(..))), "{c:?}");
}

#[test]
fn presets_apply_save_and_delete() {
    let mut t = T::new();
    let ps = t.p.presets();
    let stretch = ps.iter().find(|p| p.scaling == GpuScaling::Stretch).unwrap().clone();
    t.click(idx(K_PST, stretch.id.0 as usize));
    let want = bu_display::Mode { width: 1440, height: 1080, refresh: RefreshRate::new(164_950, 1000), scaling: GpuScaling::Stretch };
    assert_eq!(t.calls(), vec![FakeCall::ApplyMode(MonitorId(DELL.into()), want)]);
    assert!(t.p.cfm_on.is_some());
    // the fields are now an existing chip: saving says so
    t.click(K_PNEW);
    assert_eq!(t.toast(), "Already a preset");
    assert_eq!(t.p.presets().len(), 3);
    t.type_into(Fld::W, "1280");
    t.type_into(Fld::H, "720");
    t.click(K_PNEW);
    assert_eq!(t.p.presets().len(), 4);
    // delete the Stretch chip: the two rules on it go to "Choose a preset"
    t.click(sub(idx(K_PST, stretch.id.0 as usize), "del"));
    t.now += 200.0;
    let now = t.now;
    t.p.tick(now);
    assert_eq!(t.p.presets().len(), 3);
    let rules = t.p.rules();
    assert_eq!(rules.iter().filter(|r| r.preset.is_none()).count(), 2);
    assert!(page_text(&mut t).contains("Choose a preset"));
}

#[test]
fn rules_switch_vibrance_app_remove_and_add() {
    let mut t = T::new();
    // Fortnite (off) on
    t.click(sub(idx(K_RULE, 2), "tg"));
    assert!(t.p.rules()[2].enabled);
    // VALORANT's vibrance chip: "No change"
    t.ev(Ev::Press(sub(idx(K_RULE, 0), "vib"), 0.0, 0.0, (400.0, 600.0, 56.0, 24.0)));
    t.click(idx(K_MENU, 0));
    assert_eq!(t.p.rules()[0].vibrance, None);
    t.ev(Ev::Press(sub(idx(K_RULE, 0), "vib"), 0.0, 0.0, (400.0, 600.0, 56.0, 24.0)));
    t.click(idx(K_MENU, 5));
    assert_eq!(t.p.rules()[0].vibrance, Some(100));
    // CS2 → Rocket League
    t.ev(Ev::Press(sub(idx(K_RULE, 1), "app"), 0.0, 0.0, (40.0, 600.0, 144.0, 24.0)));
    t.click(idx(K_MENU, 4));
    assert_eq!(t.p.rules()[1].exe, "RocketLeague.exe");
    // remove Fortnite (after its fade)
    t.click(sub(idx(K_RULE, 2), "del"));
    t.now += 300.0;
    let now = t.now;
    t.p.tick(now);
    assert_eq!(t.p.rules().len(), 2);
    // Add app: a new row on the first Stretch preset, its app list open
    t.ev(Ev::Press(K_ADD, 0.0, 0.0, (40.0, 700.0, 80.0, 26.0)));
    assert_eq!(t.p.rules().len(), 3);
    assert!(t.p.rules()[2].exe.is_empty());
    assert!(matches!(t.p.menu.as_ref().map(|m| m.kind), Some(MenuKind::App(2))));
    assert!(page_text(&mut t).contains("Choose an app"));
}

#[test]
fn the_app_list_counts_its_separator_as_a_row_and_carries_the_tiles() {
    let mut t = T::new();
    let (kind, n) = (MenuKind::App(1), apps::KNOWN.len());
    assert_eq!(t.p.row_item(kind, 0), Some(0));
    assert_eq!(t.p.row_item(kind, n - 1), Some(n - 1));
    assert_eq!(t.p.row_item(kind, n), None, "the separator is no item");
    assert_eq!(t.p.row_item(kind, n + 1), Some(n), "Browse for an app…");
    assert_eq!(t.p.row_item(kind, n + 2), None);
    let mut cx = Cx::new(0.0, false, &t.g, &mut t.st);
    let m = t.p.menu_box(&mut cx, kind, (40.0, 600.0, 144.0, 24.0));
    assert_eq!(m.children.len(), n + 2);
    // an app row: tick, tile, name; the separator; Browse: tick, name (no tile)
    assert_eq!(m.children[0].children.len(), 3);
    assert_eq!(m.children[n].children.len(), 0);
    assert_eq!(m.children[n + 1].children.len(), 2);
    // Browse outside a click opens nothing and changes nothing
    let before = t.p.rules()[1].exe.clone();
    t.ev(Ev::Press(sub(idx(K_RULE, 1), "app"), 0.0, 0.0, (40.0, 600.0, 144.0, 24.0)));
    t.click(idx(K_MENU, n + 1));
    assert!(t.p.menu.is_none());
    assert_eq!(t.p.rules()[1].exe, before);
    // row 0 picks the first known app (VALORANT)
    t.ev(Ev::Press(sub(idx(K_RULE, 1), "app"), 0.0, 0.0, (40.0, 600.0, 144.0, 24.0)));
    t.click(idx(K_MENU, 0));
    assert_eq!(t.p.rules()[1].exe, apps::KNOWN[0].exe);
}

#[test]
fn rules_switch_at_process_start_without_a_prompt_and_never_mid_game() {
    let rt = Rt::fake_sample();
    let dell = MonitorId(DELL.into());
    // the 1440 × 1080 Stretch preset applied + kept once by hand on DELL (a rule may only use a kept preset)
    {
        let mut t = T::with(rt.clone());
        let p = t.p.presets().into_iter().find(|p| p.scaling == GpuScaling::Stretch).unwrap();
        t.click(idx(K_PST, p.id.0 as usize));
        t.click(K_KEEP);
        t.p.close();
    }
    // undo the kept change: the monitor is at 1920 again
    {
        let mut s = rt.svc.lock().unwrap();
        let _ = s.undo();
        assert_eq!(s.monitor(&dell).unwrap().current.width, 1920);
    }
    let started = |has_window| AppEvent::Started { pid: 77, exe: r"C:\Riot Games\VALORANT\live\ShooterGame\Binaries\Win64\VALORANT-Win64-Shipping.exe".into(), monitor: dell.clone(), has_window };
    // a late start (the game already has a window): nothing switches, a note waits for the page
    rt.on_app_event(&started(true));
    assert_eq!(rt.svc.lock().unwrap().monitor(&dell).unwrap().current.width, 1920);
    assert_eq!(rt.take_notes(), vec!["VALORANT was already open · it switches next launch".to_string()]);
    rt.on_app_event(&AppEvent::Stopped { pid: 77 });
    // at process start: switched at once, no keep bar
    rt.on_app_event(&started(false));
    {
        let s = rt.svc.lock().unwrap();
        assert_eq!(s.monitor(&dell).unwrap().current.width, 1440);
        assert!(s.pending().is_none(), "an automatic switch must not ask");
    }
    // the game ends: back
    rt.on_app_event(&AppEvent::Stopped { pid: 77 });
    assert_eq!(rt.svc.lock().unwrap().monitor(&dell).unwrap().current.width, 1920);
}

#[test]
fn identify_in_a_test_copy_shows_nothing() {
    let mut t = T::new();
    let before = identify::FAKE_SHOWN.load(std::sync::atomic::Ordering::Relaxed);
    t.click(K_ID);
    assert_eq!(identify::FAKE_SHOWN.load(std::sync::atomic::Ordering::Relaxed), before + 1);
    assert!(!identify::running());
}

#[test]
fn identify_numbers_follow_the_drawings_timing() {
    assert_eq!(identify::look_at(0.0, 0), (0.0, 0.86));
    let (o, s) = identify::look_at(340.0, 0);
    assert!((o - 1.0).abs() < 1e-6 && (s - 1.0).abs() < 1e-6);
    // the second number starts 40 ms later
    assert_eq!(identify::look_at(40.0, 1), (0.0, 0.86));
    // gone after 1950 + 320 ms
    assert!(identify::look_at(2270.0, 0).0 < 1e-6);
}

#[test]
fn closing_drops_everything() {
    let mut t = T::new();
    t.p.close();
    assert!(t.p.rt.is_none() && t.p.mons.is_empty() && t.p.f.is_none() && t.p.menu.is_none());
}

#[test]
fn an_apply_error_is_said_not_swallowed() {
    let mut t = T::new();
    {
        let rt = t.rt();
        let mut s = rt.svc.lock().unwrap();
        if let AnyOs::Fake(f) = s.os_mut() {
            f.fail_next_change = Some("the driver said no".into());
        }
    }
    t.type_into(Fld::W, "1280");
    t.type_into(Fld::H, "720");
    t.click(K_APPLY);
    assert!(t.toast().starts_with("Couldn’t apply:"), "{}", t.toast());
    assert!(t.p.cfm_on.is_none());
}


// ---------------------------------------------------------------- Order 036: the app's ONE change log
use crate::undo::{read_record, records, Kind, Outcome, Resettable, Review};

fn start_services() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
}

fn rec_of(item: &str) -> Option<crate::undo::Record> {
    crate::services::with(|s| read_record(&s.store, "dsp", item)).flatten()
}

fn review(p: &Display, kind: Kind) -> Review {
    crate::services::with(|s| Review::for_page(kind, p, &s.store)).unwrap()
}

/// Every change of this tab writes ONE entry with the value before the FIRST change; the reset puts the ticked ones back
/// through the page, an unticked one is kept. The FAKE runtime only.
#[test]
fn every_display_change_goes_into_the_change_log_and_back() {
    start_services();
    let mut t = T::new();
    let (dell, lg) = (MonitorId(DELL.into()), MonitorId(LG.into()));
    // rules made before the change log: "none" before the app (seeded at open), shown like the drawing
    let r = rec_of("rules").expect("the rules' line");
    assert_eq!((r.was.text.as_str(), r.now.text.as_str(), r.label.as_str()), ("none", "2 apps", "Switch automatically"));
    // a kept resolution: 1440 × 1080 Stretch
    t.click(idx(K_SCALE, 0));
    t.type_into(Fld::W, "1440");
    t.click(K_APPLY);
    assert!(rec_of(&format!("mode:{DELL}")).is_none(), "nothing before Keep");
    t.click(K_KEEP);
    let r = rec_of(&format!("mode:{DELL}")).expect("the mode's line");
    assert_eq!(
        (r.label.as_str(), r.was.text.as_str(), r.now.text.as_str()),
        ("DELL 27″ · resolution", "1920 × 1080 · 165 Hz · Keep aspect", "1440 × 1080 · 165 Hz · Stretch")
    );
    // main display -> LG
    t.click(idx(K_MON, 1));
    t.click(K_MAIN);
    let r = rec_of("main").expect("the main display's line");
    assert_eq!((r.was.raw.as_str(), r.was.text.as_str(), r.now.text.as_str()), (DELL, "DELL 27″", "LG 24″"));
    // DELL's brightness (a press + a drag = one line) and vibrance 60 %
    t.click(idx(K_MON, 0));
    t.ev(Ev::Press(K_BRI, 340.0, 0.0, (332.0, 0.0, 168.0, 20.0)));
    t.ev(Ev::Drag(K_BRI, 500.0, 0.0, (332.0, 0.0, 168.0, 20.0)));
    let r = rec_of(&format!("bri:{DELL}")).expect("the brightness line");
    assert_eq!((r.label.as_str(), r.was.text.as_str(), r.now.text.as_str()), ("DELL 27″ · brightness", "70 %", "100 %"));
    t.ev(Ev::Press(K_VIB, 332.0 + 8.0 + 152.0 * 0.6, 0.0, (332.0, 0.0, 168.0, 20.0)));
    let r = rec_of(&format!("vib:{DELL}")).expect("the vibrance line");
    assert_eq!((r.label.as_str(), r.was.text.as_str(), r.now.text.as_str()), ("DELL 27″ · vibrance", "50 %", "60 %"));
    // a rule switched on: the same line, its first old value kept
    t.click(sub(idx(K_RULE, 2), "tg"));
    let r = rec_of("rules").unwrap();
    assert_eq!((r.was.text.as_str(), r.now.text.as_str()), ("none", "3 apps"));
    // the review: every line ticked; keep the vibrance (untick), reset the rest
    let mut rv = review(&t.p, Kind::HowItWas);
    let mut items: Vec<String> = rv.lines.iter().map(|l| l.item.clone()).collect();
    items.sort();
    assert_eq!(items, [format!("bri:{DELL}"), "main".into(), format!("mode:{DELL}"), "rules".into(), format!("vib:{DELL}")]);
    assert!(rv.lines.iter().all(|l| l.ticked));
    let vib = rv.lines.iter().position(|l| l.item == format!("vib:{DELL}")).unwrap();
    assert_eq!(rv.lines[vib].change_text(), "60 %  →  50 %");
    rv.toggle(vib);
    let res = crate::services::with(|s| rv.apply(&mut s.store, &mut [&mut t.p as &mut dyn Resettable])).unwrap();
    assert!(res.iter().all(|r| r.outcome == Outcome::Ok), "{res:?}");
    let rt = t.rt();
    {
        let mut s = rt.svc.lock().unwrap();
        let m = s.monitor(&dell).unwrap();
        assert_eq!((m.current.width, m.current.scaling, m.is_main), (1920, GpuScaling::KeepAspect, true));
        assert!(!s.monitor(&lg).unwrap().is_main);
        assert_eq!(s.picture(&dell).unwrap().brightness.unwrap().current, 70);
        assert_eq!(s.vibrance_percent(&dell).unwrap(), 60, "an unticked line is kept");
    }
    assert!(t.p.rules().is_empty(), "back to no rules");
    // only the kept one is left to offer
    let again = review(&t.p, Kind::HowItWas);
    assert_eq!(again.lines.iter().map(|l| l.item.clone()).collect::<Vec<_>>(), [format!("vib:{DELL}")]);
    crate::services::shutdown();
}

/// A resolution the 10 s countdown or Revert took back leaves no line (the page took it back on its own).
#[test]
fn a_change_taken_back_by_revert_or_the_countdown_leaves_no_line() {
    start_services();
    let rt = Rt::fake_sample_keep(Duration::from_millis(100));
    let mut t = T::with(rt.clone());
    t.type_into(Fld::W, "1280");
    t.type_into(Fld::H, "960");
    t.click(K_APPLY);
    t.click(K_REVERT);
    t.click(K_APPLY);
    std::thread::sleep(Duration::from_millis(400));
    assert!(rt.svc.lock().unwrap().pending().is_none(), "the countdown went back");
    assert!(crate::services::with(|s| records(&s.store, Some("dsp"))).unwrap().iter().all(|r| !r.item.starts_with("mode:")));
    crate::services::shutdown();
}

/// "Windows defaults": vibrance 50 % on each monitor not there, no rules (the drawing's RS.dsp.win).
#[test]
fn windows_defaults_set_vibrance_normal_and_remove_the_rules() {
    start_services();
    let mut t = T::new();
    let lg = MonitorId(LG.into());
    t.rt().svc.lock().unwrap().set_vibrance_percent(&lg, 80).unwrap();
    let rv = review(&t.p, Kind::WindowsDefaults);
    assert_eq!(rv.title(), "Display · Windows defaults?");
    let lines: Vec<(String, String)> = rv.lines.iter().map(|l| (l.label.clone(), l.change_text())).collect();
    assert_eq!(lines, [("LG 24″ · vibrance".to_string(), "80 %  →  50 %".to_string()), ("Switch automatically".to_string(), "2 apps  →  none".to_string())]);
    let res = crate::services::with(|s| rv.apply(&mut s.store, &mut [&mut t.p as &mut dyn Resettable])).unwrap();
    assert!(res.iter().all(|r| r.outcome == Outcome::Ok), "{res:?}");
    assert_eq!(t.rt().svc.lock().unwrap().vibrance_percent(&lg).unwrap(), 50);
    assert!(t.p.rules().is_empty());
    assert!(review(&t.p, Kind::WindowsDefaults).is_empty());
    crate::services::shutdown();
}

/// Settings › Reset and the uninstaller ask CLOSED pages: `resettable()` makes nothing; the runtime comes on first need.
#[test]
fn a_closed_page_is_cheap_and_resets_through_a_runtime_made_on_first_need() {
    start_services();
    let mut fresh = Display::default();
    assert!(fresh.resettable().is_some());
    assert!(fresh.lazy_rt.get().is_none() && fresh.rt.is_none(), "resettable() made a runtime");
    assert!(review(&fresh, Kind::HowItWas).is_empty());
    assert!(fresh.lazy_rt.get().is_none(), "no line = nothing read");
    // a vibrance change made earlier (the page was open then), the page closed now
    let rt = Rt::fake_sample();
    let dell = MonitorId(DELL.into());
    let (old, new) = rt.svc.lock().unwrap().set_vibrance_percent_change(&dell, 80).unwrap().unwrap();
    crate::services::with(|s| crate::undo::record(&mut s.store, "dsp", &format!("vib:{DELL}"), "DELL 27″ · vibrance", &reset::vib_val(&old), &reset::vib_val(&new)).unwrap());
    let mut closed = Display::default();
    let _ = closed.lazy_rt.set(rt.clone());
    let rv = review(&closed, Kind::HowItWas);
    assert_eq!(rv.lines.len(), 1);
    assert_eq!(rv.lines[0].change_text(), "80 %  →  50 %");
    let res = crate::services::with(|s| rv.apply(&mut s.store, &mut [&mut closed as &mut dyn Resettable])).unwrap();
    assert_eq!(res[0].outcome, Outcome::Ok);
    assert_eq!(rt.svc.lock().unwrap().vibrance_percent(&dell).unwrap(), 50);
    assert!(review(&closed, Kind::HowItWas).is_empty());
    crate::services::shutdown();
}

/// The reset links open the frame's ONE review under the link (no page-local popup any more).
#[test]
fn the_reset_links_open_the_frames_review() {
    let mut t = T::new();
    let r = (220.0, 483.0, 132.0, 16.0);
    t.ev(Ev::Press(sub(K_RESET, "pc"), 0.0, 0.0, r));
    let mut cx = Cx::new(0.0, false, &t.g, &mut t.st).for_page("dsp");
    t.p.event(&Ev::Click(sub(K_RESET, "pc")), &mut cx);
    t.p.event(&Ev::Click(sub(K_RESET, "win")), &mut cx);
    let reqs = std::mem::take(&mut cx.reqs);
    assert!(matches!(reqs[0], crate::ui::cx::Req::Reset(Kind::HowItWas, b) if b == r), "{reqs:?}");
    assert!(matches!(reqs[1], crate::ui::cx::Req::Reset(Kind::WindowsDefaults, _)), "{reqs:?}");
}

/// Items go to text and back (they survive restarts: the uninstaller runs in a fresh process).
#[test]
fn item_ids_read_back() {
    let m = MonitorId(r"\\?\DISPLAY#DEL41B6#5&2a6e&0&UID4352#{e6f07b5f}".into());
    for it in [reset::Item::Mode(m.clone()), reset::Item::Main, reset::Item::Ddc(m.clone(), Vcp::Brightness), reset::Item::Ddc(m.clone(), Vcp::Contrast), reset::Item::Vib(m), reset::Item::Rules] {
        assert_eq!(reset::Item::parse(&it.id()), Some(it));
    }
    assert_eq!(reset::Item::parse("nope:x"), None);
}

// ---------------------------------------------------------------- Order 047: the menu's thread never waits

impl T {
    /// The page on a SLOW worker (every job sleeps 300 ms first - a stand-in for SetDisplayConfig, the monitors' read, a
    /// DDC/CI write holding the service): its open hands the menu's thread back within one frame.
    fn slow() -> T {
        let mut p = Display::default();
        p.slow = 300;
        let rt = Rt::fake_sample();
        crate::offui::assert_quick("Display open", || p.open_rt(rt, &env(), 0.0));
        assert!(p.mons.is_empty(), "the monitors are read on the worker");
        p.settle(0.0);
        let mut t = T { p, g: Gfx::new(1.0), st: State::default(), now: 0.0 };
        t.build();
        t
    }
    /// An event that must not wait (the Cx is made outside the timed part); its answer is not waited for.
    fn quick(&mut self, e: Ev, what: &str) {
        self.now += 16.0;
        let mut cx = Cx::new(self.now, false, &self.g, &mut self.st);
        let p = &mut self.p;
        crate::offui::assert_quick(what, || p.event(&e, &mut cx));
    }
    /// The answers landed.
    fn settle(&mut self) {
        let now = self.now;
        self.p.settle(now);
        self.build();
    }
}

/// Apply, Revert and Keep (SetDisplayConfig on a real PC: 0.5 - 3 s) return at once; the keep bar and its countdown
/// come with Windows' answer, exactly as before.
#[test]
fn apply_revert_and_keep_never_hold_the_menu() {
    let mut t = T::slow();
    t.type_into(Fld::W, "1280");
    t.type_into(Fld::H, "720");
    t.quick(Ev::Click(K_APPLY), "Apply");
    assert!(t.calls().is_empty() && t.p.cfm_on.is_none(), "Windows hasn't answered yet");
    t.settle();
    assert!(matches!(t.calls().last(), Some(FakeCall::ApplyMode(..))), "{:?}", t.calls());
    assert!(t.p.cfm_on.is_some() && t.rt().svc.lock().unwrap().pending().is_some(), "the keep bar after Windows' answer");
    t.quick(Ev::Click(K_REVERT), "Revert");
    t.settle();
    assert_eq!(t.toast(), "Back to 1920 × 1080 · 165 Hz");
    assert_eq!(t.fields().w, 1920);
    t.type_into(Fld::W, "1280");
    t.type_into(Fld::H, "720");
    t.quick(Ev::Click(K_APPLY), "Apply again");
    t.settle();
    t.quick(Ev::Click(K_KEEP), "Keep");
    assert!(t.p.cfm_on.is_none(), "the bar goes with the click");
    t.settle();
    assert!(t.toast().starts_with("Kept 1280 × 720"), "{}", t.toast());
    assert!(matches!(t.calls().last(), Some(FakeCall::SaveCurrent(_))), "{:?}", t.calls());
    assert!(t.rt().svc.lock().unwrap().pending().is_none());
}

/// Main display (SetDisplayConfig) returns at once; the toast comes with the answer.
#[test]
fn main_display_never_holds_the_menu() {
    let mut t = T::slow();
    t.click(idx(K_MON, 1));
    t.quick(Ev::Click(K_MAIN), "Main display");
    t.settle();
    assert_eq!(t.calls().last(), Some(&FakeCall::SetMain(MonitorId(LG.into()))));
    assert_eq!(t.toast(), "Main display: 2 · LG 24GL600F");
}

/// The tab opens on the last read at once (`env.keep`); the fresh read follows from the worker.
#[test]
fn the_tab_opens_on_its_last_read() {
    let e = env();
    let mut p = Display::default();
    p.open_rt(Rt::fake_sample(), &e, 0.0);
    p.settle(0.0);
    p.close();
    let mut q = Display::default();
    q.slow = 300;
    let rt = Rt::fake_sample();
    crate::offui::assert_quick("Display open again", || q.open_rt(rt, &e, 0.0));
    assert!(!q.mons.is_empty() && q.f.is_some() && q.ready(), "the last read at once");
    q.settle(0.0);
    assert_eq!(q.mons.len(), 2);
}

/// A rule switched / removed: the app watcher (WMI's answer: 0.1 - 1.5 s on a real PC) is brought up to date on its
/// own thread (here slowed by `offui::set_test_delay`).
#[test]
fn a_rule_change_never_waits_for_the_watcher() {
    let mut t = T::new();
    let before = crate::offui::done();
    crate::offui::set_test_delay(300);
    t.quick(Ev::Click(sub(idx(K_RULE, 2), "tg")), "a rule switched on");
    assert!(t.p.rules()[2].enabled);
    let t0 = std::time::Instant::now();
    while crate::offui::done() <= before && t0.elapsed().as_secs() < 5 {
        std::thread::sleep(Duration::from_millis(5));
    }
    crate::offui::set_test_delay(0);
    assert!(crate::offui::done() > before, "the watcher's update ran");
}

/// Nothing moving = no frames: `tick` is false, `wake_at` None; a deleted chip's fade ends at a timed wake-up.
#[test]
fn idle_page_asks_for_no_frames() {
    let mut t = T::new();
    let now = t.now + 1000.0;
    assert!(!t.p.tick(now), "nothing moves: no frames");
    assert_eq!(t.p.wake_at(now), None);
    let id = t.p.presets()[0].id;
    t.click(sub(idx(K_PST, id.0 as usize), "del"));
    let t0 = t.p.pst_out.unwrap().1;
    assert_eq!(t.p.wake_at(t0 + 1.0), Some(t0 + 160.0));
    assert!(t.p.tick(t0 + 160.0), "the chip leaves at its fade's end");
    assert!(t.p.pst_out.is_none() && t.p.wake_at(t0 + 160.0).is_none());
    assert!(!t.p.tick(t0 + 200.0));
    assert_eq!(t.p.presets().len(), 2);
}

/// Order 047: the reset review reads and puts back through the page's worker copy (`detach`, the page's runtime): with
/// a slow copy (300 ms a call) its open and its Reset (a mode change on a real PC) hand the menu's thread back within one
/// frame, the mode still goes back, and the open page reads it again (`reset_done`).
#[test]
fn the_reset_review_never_holds_the_menu() {
    use crate::undo::{Applied, Opened};
    fn wait_for<T>(mut f: impl FnMut() -> Option<T>) -> T {
        let t0 = std::time::Instant::now();
        loop {
            if let Some(v) = f() {
                return v;
            }
            assert!(t0.elapsed().as_secs() < 10, "the worker never answered");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    start_services();
    let mut t = T::new();
    let dell = MonitorId(DELL.into());
    t.click(idx(K_SCALE, 0));
    t.type_into(Fld::W, "1440");
    t.click(K_APPLY);
    t.click(K_KEEP);
    assert_eq!(t.rt().svc.lock().unwrap().monitor(&dell).unwrap().current.width, 1440);
    t.p.slow = 300;
    let review = {
        let mut pages: [&mut dyn Resettable; 1] = [&mut t.p];
        let opened = crate::services::with(|s| crate::offui::assert_quick("the review opens", || Review::open(Kind::HowItWas, false, &mut pages, &s.store))).unwrap();
        let Opened::Reading(mut job) = opened else { panic!("read on a worker thread") };
        wait_for(|| job.take())
    };
    let mode = review.lines.iter().position(|l| l.item == format!("mode:{DELL}")).expect("the mode's line");
    assert_eq!(review.lines[mode].change_text(), "1440 × 1080 · 165 Hz · Stretch  →  1920 × 1080 · 165 Hz · Keep aspect");
    let res = {
        let mut pages: [&mut dyn Resettable; 1] = [&mut t.p];
        let applied = crate::offui::assert_quick("Reset", || review.start_apply(&mut pages));
        let Applied::Running(mut job) = applied else { panic!("put back on a worker thread") };
        wait_for(|| job.take())
    };
    assert!(res.iter().all(|r| r.outcome == Outcome::Ok), "{res:?}");
    assert_eq!(t.rt().svc.lock().unwrap().monitor(&dell).unwrap().current.width, 1920, "back to how the PC was");
    t.p.reset_done();
    t.settle();
    assert_eq!(t.fields().w, 1920, "the open page shows it");
    crate::services::shutdown();
}

/// Order 047: a second mode click while one is on the worker (here a preset during an Apply) is remembered and sent when
/// the first has answered - the fields, the keep bar and Windows end on the second one.
#[test]
fn a_mode_click_during_a_mode_change_is_sent_after_it() {
    let mut t = T::slow();
    t.type_into(Fld::W, "1280");
    t.type_into(Fld::H, "720");
    t.quick(Ev::Click(K_APPLY), "Apply");
    let stretch = t.p.presets().into_iter().find(|p| p.scaling == GpuScaling::Stretch).unwrap();
    t.quick(Ev::Click(idx(K_PST, stretch.id.0 as usize)), "a preset during the Apply");
    t.settle();
    let applied: Vec<FakeCall> = t.calls().into_iter().filter(|c| matches!(c, FakeCall::ApplyMode(..))).collect();
    assert_eq!(applied.len(), 2, "{applied:?}");
    let want = bu_display::Mode { width: 1440, height: 1080, refresh: RefreshRate::new(164_950, 1000), scaling: GpuScaling::Stretch };
    assert_eq!(applied[1], FakeCall::ApplyMode(MonitorId(DELL.into()), want));
    assert_eq!((t.fields().w, t.fields().sc), (1440, GpuScaling::Stretch));
    assert!(t.p.cfm_on.is_some() && t.p.next.is_none());
}
