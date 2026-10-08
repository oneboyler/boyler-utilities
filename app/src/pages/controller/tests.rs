//! Tests of the Controller page against the FAKE Steam + fake controller (Order 020): every kind of control writes the
//! game's layout (or the controller's file) through bu-controller; pick / back; popups; resets; nothing is ever written to
//! the PC (the fake is in memory).

use super::*;
use crate::gfx::Gfx;
use crate::ui::cx::State;

struct T {
    p: Controller,
    g: Gfx,
    st: State,
    now: f64,
}

impl T {
    fn new() -> T {
        let mut p = Controller::default();
        p.open(&Env { test: true, frozen: true, ..Env::default() }, 0.0);
        let mut t = T { p, g: Gfx::new(1.0), st: State::default(), now: 1000.0 };
        t.settle();
        t.build();
        t
    }
    /// Order 047: the tab's worker reads and writes off the menu's thread - wait (frame steps, as the menu makes them)
    /// until every answer asked for so far is in.
    fn settle(&mut self) {
        let t0 = std::time::Instant::now();
        // nothing asked = no frame step at all (a test's own ticks - the drift check - stay the only ones)
        while self.p.o.as_ref().is_some_and(|o| !o.idle()) {
            let now = self.now;
            self.p.tick(now);
            assert!(t0.elapsed().as_secs() < 20, "the tab's worker never answered");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    /// The tab's fake Steam answers this slowly from now on (ms per file read).
    fn slow(&mut self, ms: u64) {
        self.o().svc_do(|s| s.set_fake_delay(ms)).unwrap();
    }
    fn o(&mut self) -> &mut Open {
        self.p.o.as_mut().unwrap()
    }
    fn build(&mut self) -> Vec<El> {
        self.now += 2000.0; // past every animation
        let mut cx = Cx::new(self.now, false, &self.g, &mut self.st).for_page("pad");
        let els = self.p.build(&mut cx);
        let _ = self.p.popup(&mut cx);
        els
    }
    fn ev(&mut self, e: Ev) {
        let mut cx = Cx::new(self.now, false, &self.g, &mut self.st).for_page("pad");
        self.p.event(&e, &mut cx);
        self.settle();
        self.build();
    }
    fn click(&mut self, k: Key) {
        self.ev(Ev::Press(k, 100.0, 300.0, (90.0, 290.0, 20.0, 20.0)));
        self.ev(Ev::Click(k));
    }
    /// A click in the middle of a picture part (the picture at window (0, 0)).
    fn click_part(&mut self, id: Pid) {
        let s = BIG_W / pic::PIC_W;
        let p = self.o().pic.parts.iter().find(|p| p.id == id).unwrap().clone();
        let (l, t, r, b) = p.hit_boxes(&self.g)[0];
        let k = Open::box_key(id, 0);
        let (cx, cy) = match &p.s[0] {
            pic::Sh::Circle { cx, cy, .. } => (*cx, *cy),
            _ => ((l + r) / 2.0, (t + b) / 2.0),
        };
        self.ev(Ev::Press(k, cx * s, cy * s, (l * s, t * s, (r - l) * s, (b - t) * s)));
        self.ev(Ev::Click(k));
    }
    fn slide(&mut self, id: &str, frac: f32) {
        let k = Open::k(id);
        let r = (0.0, 0.0, 216.0, 20.0);
        let x = 8.0 + 200.0 * frac;
        self.ev(Ev::Press(k, x, 10.0, r));
        self.ev(Ev::Drag(k, x, 10.0, r));
        self.ev(Ev::Release(k));
    }
    fn writes(&mut self) -> Vec<String> {
        self.o().svc_do(|s| s.fake_writes()).unwrap()
    }
    fn rl(&mut self) -> String {
        self.o().svc_do(|s| s.fake_text(&data::config().join(r"252950\controller_ps5.vdf"))).unwrap().unwrap()
    }
    fn prefs(&mut self) -> String {
        self.o().svc_do(|s| s.fake_text(&data::config().join(format!("preferences_{}.vdf", data::SERIAL)))).unwrap().unwrap()
    }
    fn toast(&mut self) -> String {
        self.o().toast.clone().map(|t| t.0).unwrap_or_default()
    }
}

#[test]
fn opens_on_the_drawings_sample() {
    let mut t = T::new();
    let o = t.o();
    assert_eq!(o.kind, PadKind::DualSenseEdge);
    let names: Vec<&str> = o.games.iter().map(|g| g.name.as_str()).collect();
    assert!(names.contains(&"Rocket League") && names.contains(&"Epic Games Launcher"), "{names:?}");
    assert_eq!(o.game().unwrap().name, "Rocket League");
    // R4 = F5 (the user's layout), the touchpad's right click = light bar red
    let v = o.view.clone().unwrap();
    let r4 = v.buttons.iter().find(|b| b.id == ButtonId::BackRightUpper).unwrap();
    assert_eq!(r4.presses[0].1, Action::Key("F5".into()));
    assert_eq!(v.touchpad.unwrap().right_click, Action::light_bar_red());
    assert!(o.connected().is_some(), "the fake Edge is plugged in");
    assert_eq!(o.pref().unwrap().led(), Some((59, 130, 255)));
    assert!(t.writes().is_empty(), "opening the tab writes nothing");
}

#[test]
fn pick_a_part_and_back() {
    let mut t = T::new();
    t.click_part(Pid::B(ButtonId::Cross));
    assert_eq!(t.o().sel, Some(Pid::B(ButtonId::Cross)));
    // another part: just swaps
    t.click_part(Pid::Stick(Side::Left));
    assert_eq!(t.o().sel, Some(Pid::Stick(Side::Left)));
    // the window's own empty space keeps it; a click beside the window = back to the controller
    t.click(K_PANEL);
    t.click(sub(K_PANEL, "win"));
    assert!(t.o().sel.is_some());
    t.click(sub(K_PANEL, "out"));
    assert_eq!(t.o().sel, None);
    // the × too
    t.click_part(Pid::Trig(Side::Right));
    t.click(sub(K_PANEL, "x"));
    assert_eq!(t.o().sel, None);
    // the light bar opens the Controller settings (A_015_02)
    t.click_part(Pid::Light);
    assert!(t.o().dlg.is_some() && t.o().sel.is_none());
    assert!(t.writes().is_empty());
}

/// the owner (test build 1): "a better approach to show settings for each clicked thing would be a pop up window ... bigger,
/// and you could scroll down if needed ... each bubble should ofc show the settings you opened up are for", and the
/// controller stays big + centred (no shrinking to the side).
#[test]
fn a_picked_part_opens_its_own_scrolling_window_and_the_controller_stays_big() {
    let mut t = T::new();
    t.click_part(Pid::Stick(Side::Left));
    assert_eq!(t.o().sel, Some(Pid::Stick(Side::Left)));
    t.now += 2000.0;
    let mut cx = Cx::new(t.now, false, &t.g, &mut t.st);
    // the page: the picture is still the big one
    let kids = t.p.build(&mut cx);
    let page = crate::ui::lay::Laid::new(&t.g, El::block().w(WIN_W).pad(2.0, 26.0, 18.0, 26.0).children(kids), WIN_W, None);
    let (_, _, pw, _) = page.rect_of(K_PIC).unwrap();
    assert_eq!(pw, BIG_W);
    // the window: 480 wide, centred, titled with the part, its settings in a scroll box no taller than the window allows
    let pop = t.p.popup(&mut cx).expect("the part's window");
    let l = crate::ui::lay::Laid::new(&t.g, El::block().w(WIN_W).h(crate::ui::WIN_H).child(pop), WIN_W, Some(crate::ui::WIN_H));
    let (wx, wy, ww, wh) = l.rect_of(sub(K_PANEL, "win")).unwrap();
    assert_eq!((ww, wx), (PANEL_W, (WIN_W - PANEL_W) / 2.0));
    assert!(wh <= 468.0 && wy >= 0.0, "fits the window: y {wy} h {wh}");
    assert_eq!(t.o().pname(Pid::Stick(Side::Left)), "Left stick");
    let (_, sy, _, sh) = l.rect_of(K_PANEL).expect("the scroll box");
    assert!(sy > wy && sy + sh <= wy + wh, "the settings scroll inside the window");
}

#[test]
fn exact_shapes_decide_the_part() {
    let mut t = T::new();
    // a press inside the touchpad's box but on the left light strip -> the light bar (it is painted on top)
    let s = BIG_W / pic::PIC_W;
    let k = K_PIC;
    // (the picture moved down by dy when its shoulder buttons needed the room - `pic::lift`)
    let ph = t.o().pic.h;
    let dy = ph - pic::PIC_H;
    t.ev(Ev::Press(k, 157.0 * s, (77.0 + dy) * s, (0.0, 0.0, BIG_W, BIG_W * ph / pic::PIC_W)));
    assert_eq!(t.o().pressed_part, Some(Pid::Light));
    // the body between the handles is not a part
    t.ev(Ev::Press(k, 240.0 * s, (260.0 + dy) * s, (0.0, 0.0, BIG_W, 400.0)));
    assert_eq!(t.o().pressed_part, None);
}

#[test]
fn a_switch_writes_the_layout_once_and_reads_it_back() {
    let mut t = T::new();
    t.click_part(Pid::B(ButtonId::Cross));
    t.click(Open::k("cross.turbo"));
    let w = t.writes();
    assert!(w.iter().any(|l| l.contains(r"252950\controller_ps5.vdf")), "{w:?}");
    assert!(w.iter().any(|l| l.contains(".original")), "the original was kept first: {w:?}");
    assert!(t.rl().contains("\"hold_repeats\"\t\t\"1\""));
    assert!(t.toast().starts_with("Saved"));
    // the panel shows it: the next click turns it off again (Steam's default = the line goes)
    t.click(Open::k("cross.turbo"));
    assert!(!t.rl().contains("\"hold_repeats\""));
}

#[test]
fn a_slider_writes_once_on_release() {
    let mut t = T::new();
    t.click_part(Pid::Stick(Side::Left));
    let before = t.writes().len();
    // dead zone 0..60 %: 50 % of the track = 30 %
    t.slide("ls.dz", 0.5);
    let after: Vec<String> = t.writes()[before..].iter().filter(|w| w.contains(r"Steam\steamapps")).cloned().collect();
    assert_eq!(after.len(), 1, "one write of the layout for one drag: {after:?}");
    let want = bu_controller::settings::pct_to_radius(30.0);
    assert!(t.rl().contains(&format!("\"deadzone_inner_radius\"\t\t\"{want}\"")), "{}", t.rl());
}

#[test]
fn the_action_picker_searches_and_writes() {
    let mut t = T::new();
    t.click_part(Pid::B(ButtonId::Square));
    t.click(Open::k("square.full"));
    assert!(matches!(t.o().pop, Some(Pop::Act { .. })));
    for c in "f6".chars() {
        t.ev(Ev::Char(K_ASRCH, c));
    }
    let items = t.o().act_items("f6");
    assert_eq!(items[0].1, Action::Key("F6".into()), "{items:?}");
    t.ev(Ev::Key(K_ASRCH, 0x0D));
    assert!(t.o().pop.is_none());
    assert!(t.rl().contains("key_press F6"), "Square does F6 now");
    // nothing matches
    assert!(t.o().act_items("zzzz").is_empty());
}

#[test]
fn popup_buttons_segments_and_modes() {
    let mut t = T::new();
    t.click(K_GYRO);
    assert_eq!(t.o().sel, Some(Pid::Gyro));
    t.click(Open::k("gy.mode"));
    assert_eq!(t.o().pop, Some(Pop::Menu(Open::k("gy.mode"))));
    t.click(idx(K_MENU, 1)); // As mouse
    assert_eq!(t.o().view.as_ref().unwrap().gyro.as_ref().unwrap().mode, GyroMode::Mouse);
    // a segment: the trigger's Click only
    t.click_part(Pid::Trig(Side::Right));
    t.click(idx(Open::k("r2.mode"), 1));
    assert!(!t.o().view.as_ref().unwrap().triggers[1].analog);
    // the stick's mode
    t.click_part(Pid::Stick(Side::Right));
    t.click(Open::k("rs.mode"));
    t.click(idx(K_MENU, 2)); // As mouse
    assert_eq!(t.o().view.as_ref().unwrap().sticks[1].mode, StickMode::Mouse);
}

#[test]
fn things_steam_has_no_file_for_are_shown_but_never_written() {
    let mut t = T::new();
    t.click_part(Pid::Fn(Side::Left));
    let n = t.writes().len();
    t.click(Open::k("fn"));
    assert_eq!(t.writes().len(), n);
    assert!(t.o().pop.is_none());
}

/// Order 045 item 13: the drawing's gyro Smoothing, Calibrate, touchpad "One button" and "Turn off when idle" have no slot
/// in Steam's files - "Nothing on screen may do nothing": they are not built at all.
#[test]
fn controls_steam_has_no_slot_for_are_not_there() {
    let mut t = T::new();
    let gone = |t: &mut T, id: &str| {
        t.now += 2000.0;
        let mut cx = Cx::new(t.now, false, &t.g, &mut t.st).for_page("pad");
        let mut kids = t.p.build(&mut cx);
        kids.extend(t.p.popup(&mut cx));
        let laid = crate::ui::lay::Laid::new(&t.g, El::block().w(WIN_W).children(kids), WIN_W, None);
        laid.rect_of(Open::k(id)).is_none()
    };
    t.click(K_GYRO);
    assert!(!gone(&mut t, "gy.ix"), "the gyro's panel is open");
    assert!(gone(&mut t, "gy.smooth") && gone(&mut t, "gy.cal"));
    t.click_part(Pid::Touch);
    assert!(!gone(&mut t, "tp.req"), "the touchpad's panel is open");
    assert!(gone(&mut t, "tp.one"));
    t.click(K_CTLSET);
    assert!(!gone(&mut t, "pf.rumble"), "Controller settings are open");
    assert!(gone(&mut t, "pf.idle"));
}

#[test]
fn controller_settings_write_the_controllers_file() {
    let mut t = T::new();
    t.click(K_CTLSET);
    assert!(t.o().dlg.is_some());
    t.click(Open::k("pf.anti"));
    assert!(t.prefs().contains("\"antidrift_enabled_sw\"\t\t\"0\""));
    // the light bar: red (A_015_02 - per controller, every game)
    t.click(idx(Open::k("pf.lc"), 1));
    assert!(t.prefs().contains("\"color_red\"\t\t\"255\"") && t.prefs().contains("\"color_green\"\t\t\"69\""));
    t.slide("pf.lsdz", 0.25); // 0..40 % -> 10 %
    let want = bu_controller::settings::pct_to_radius(10.0);
    assert!(t.prefs().contains(&format!("\"stick_left_deadzone\"\t\t\"{want}\"")), "{}", t.prefs());
    // the game's layout was not touched
    assert!(!t.writes().iter().any(|w| w.contains("252950")));
    t.click(sub(K_DLG, "x"));
    assert!(t.o().dlg.is_none());
}

#[test]
fn steams_setting_for_this() {
    let mut t = T::new();
    t.click_part(Pid::B(ButtonId::Cross));
    t.click(Open::k("cross.turbo"));
    assert!(t.rl().contains("hold_repeats"));
    // one part back to Steam's own layout
    t.click(K_STEAMSET);
    assert!(!t.rl().contains("hold_repeats"));
    assert!(t.toast().contains("back to Steam"));
}

// ------------------------------------------------------------------------------------------------ the change log (Order 036)

/// The app's services for one test (a scratch settings store: the change log), stopped at the end.
struct Sv;
impl Sv {
    fn start() -> Sv {
        crate::services::init(windows::Win32::Foundation::HWND::default(), true);
        Sv
    }
}
impl Drop for Sv {
    fn drop(&mut self) {
        crate::services::shutdown();
    }
}

const RL_ITEM: &str = "layout:ps5:252950";

fn rec(item: &str) -> Option<crate::undo::Record> {
    crate::services::with(|s| crate::undo::read_record(&s.store, "pad", item)).flatten()
}

fn review(p: &Controller, kind: crate::undo::Kind) -> crate::undo::Review {
    crate::services::with(|s| crate::undo::Review::for_page(kind, p, &s.store)).unwrap()
}

fn reset(p: &mut Controller, rv: &crate::undo::Review) -> Vec<crate::undo::LineResult> {
    crate::services::with(|s| rv.apply(&mut s.store, &mut [p as &mut dyn crate::undo::Resettable])).unwrap()
}

fn texts(rv: &crate::undo::Review) -> Vec<String> {
    rv.lines.iter().map(|l| format!("{}: {}", l.label, l.change_text())).collect()
}

impl T {
    /// The tab as a normal copy shows it (not a test picture): its reset links open the frame's review.
    fn live() -> T {
        let mut p = Controller::default();
        p.open(&Env { test: true, ..Env::default() }, 0.0);
        let mut t = T { p, g: Gfx::new(1.0), st: State::default(), now: 1000.0 };
        t.settle();
        t.build();
        t
    }
    /// A click's requests to the frame.
    fn click_reqs(&mut self, k: Key, r: (f32, f32, f32, f32)) -> Vec<crate::ui::cx::Req> {
        let mut cx = Cx::new(self.now, false, &self.g, &mut self.st).for_page("pad");
        self.p.event(&Ev::Press(k, r.0 + 2.0, r.1 + 2.0, r), &mut cx);
        self.p.event(&Ev::Click(k), &mut cx);
        std::mem::take(&mut cx.reqs)
    }
}

/// A layout write = ONE entry whose old value is the bytes from before the app's FIRST change (the crate's backup);
/// the reset puts them back through the open tab; an unticked line stays.
#[test]
fn a_layout_change_goes_into_the_change_log_and_back() {
    let _s = Sv::start();
    let mut t = T::live();
    let original = t.rl();
    t.click_part(Pid::B(ButtonId::Cross));
    t.click(Open::k("cross.turbo"));
    t.slide("cross.rate", 0.5);
    assert_ne!(t.rl(), original);
    let r = rec(RL_ITEM).expect("one entry");
    assert_eq!(r.label, "Rocket League \u{b7} DualSense Edge");
    assert_eq!((r.was.raw.as_str(), r.now.raw.as_str(), r.now.text.as_str()), ("orig", "edits", "your edits"));
    assert!(r.was.text.starts_with("as on ") && r.was.text.ends_with(" (backup)"), "{}", r.was.text);
    assert_eq!(crate::services::with(|s| crate::undo::records(&s.store, Some("pad")).len()).unwrap(), 1);
    // the reset link: the frame's review under the link
    let b = (200.0, 900.0, 140.0, 16.0);
    let reqs = t.click_reqs(sub(K_RESET, "pc"), b);
    assert!(matches!(reqs.as_slice(), [crate::ui::cx::Req::Reset(crate::undo::Kind::HowItWas, a)] if *a == b), "{reqs:?}");
    assert!(t.o().pop.is_none());
    let mut rv = review(&t.p, crate::undo::Kind::HowItWas);
    assert_eq!(rv.lines.len(), 1);
    assert_eq!(rv.lines[0].from.text, "your edits");
    // untick = kept
    rv.toggle(0);
    assert!(reset(&mut t.p, &rv).is_empty());
    assert_ne!(t.rl(), original);
    // ticked = the bytes from before
    rv.toggle(0);
    assert!(reset(&mut t.p, &rv).iter().all(|r| r.outcome == crate::undo::Outcome::Ok));
    assert_eq!(t.rl(), original);
    assert!(review(&t.p, crate::undo::Kind::HowItWas).is_empty(), "back: the line is gone");
}

/// "Steam’s layout" is the tab's Windows defaults: the shown game's layout goes to Steam's own; that change is logged
/// too, so "Back to how your PC was" brings the user's own layout back.
#[test]
fn steams_layout_is_the_tabs_windows_defaults() {
    let _s = Sv::start();
    let mut t = T::live();
    let original = t.rl();
    let b = (300.0, 900.0, 90.0, 16.0);
    let reqs = t.click_reqs(sub(K_RESET, "win"), b);
    assert!(matches!(reqs.as_slice(), [crate::ui::cx::Req::Reset(crate::undo::Kind::WindowsDefaults, a)] if *a == b), "{reqs:?}");
    let rv = review(&t.p, crate::undo::Kind::WindowsDefaults);
    assert_eq!(rv.title(), "Controller \u{b7} back to Steam\u{2019}s layout?");
    assert_eq!(texts(&rv), ["Rocket League \u{b7} DualSense Edge: your layout  \u{2192}  Steam\u{2019}s layout"]);
    assert!(reset(&mut t.p, &rv).iter().all(|r| r.outcome == crate::undo::Outcome::Ok));
    assert_ne!(t.rl(), original);
    assert!(review(&t.p, crate::undo::Kind::WindowsDefaults).is_empty(), "Steam's own now");
    let rv = review(&t.p, crate::undo::Kind::HowItWas);
    assert_eq!(texts(&rv), ["Rocket League \u{b7} DualSense Edge: Steam\u{2019}s layout  \u{2192}  your layout"]);
    assert!(reset(&mut t.p, &rv).iter().all(|r| r.outcome == crate::undo::Outcome::Ok));
    assert_eq!(t.rl(), original);
}

/// The controller's own settings file (this controller, every game): one entry, put back from its backup.
#[test]
fn controller_settings_go_into_the_change_log_and_back() {
    let _s = Sv::start();
    let mut t = T::live();
    let original = t.prefs();
    t.click(K_CTLSET);
    t.click(Open::k("pf.anti"));
    t.click(idx(Open::k("pf.lc"), 1));
    assert_ne!(t.prefs(), original);
    let item = format!("prefs:{}", data::SERIAL);
    let r = rec(&item).expect("one entry");
    assert_eq!(r.label, "Controller settings \u{b7} DualSense Edge Wireless Controller");
    assert_eq!((r.was.raw.as_str(), r.now.text.as_str()), ("orig", "your settings"));
    assert!(rec(RL_ITEM).is_none(), "the game's layout was not touched");
    let rv = review(&t.p, crate::undo::Kind::HowItWas);
    assert_eq!(rv.lines.len(), 1);
    assert!(reset(&mut t.p, &rv).iter().all(|r| r.outcome == crate::undo::Outcome::Ok));
    assert_eq!(t.prefs(), original);
    assert!(review(&t.p, crate::undo::Kind::HowItWas).is_empty());
}

/// A tab that was never opened (Settings › Reset, the uninstaller): `resettable()` makes nothing; the first question
/// makes its own Steam service (the fake in tests) - here one whose Rocket League the app changed earlier.
#[test]
fn a_closed_tab_resets_through_its_own_service() {
    let _s = Sv::start();
    let mut p = Controller::default();
    assert!(p.resettable().is_some());
    assert!(p.cold.lock().unwrap().is_none() && p.o.is_none(), "nothing made by resettable()");
    // the PC as an earlier run left it: Rocket League changed (its backup kept), the entry in the change log
    let mut svc = data::open_steam(true, false).unwrap();
    let rl = data::config().join(r"252950\controller_ps5.vdf");
    let original = svc.fake_text(&rl).unwrap();
    let c = bu_controller::Change::ButtonSetting { button: ButtonId::Cross, setting: bu_controller::PressSetting::HoldToRepeat, value: Some(1) };
    svc.apply_all("252950", PadKind::DualSenseEdge, 0, &[c]).unwrap();
    assert_ne!(svc.fake_text(&rl).unwrap(), original);
    *p.cold.lock().unwrap() = Some(Ok(svc));
    let label = data::layout_label("Rocket League", PadKind::DualSenseEdge);
    crate::services::with(|s| crate::undo::record(&mut s.store, "pad", RL_ITEM, &label, &data::orig_val(true), &crate::undo::Val::new("edits", "your edits")))
        .unwrap()
        .unwrap();
    let rv = review(&p, crate::undo::Kind::HowItWas);
    assert_eq!(rv.lines.len(), 1);
    assert!(reset(&mut p, &rv).iter().all(|r| r.outcome == crate::undo::Outcome::Ok));
    let now = match &*p.cold.lock().unwrap() {
        Some(Ok(s)) => s.fake_text(&rl).unwrap(),
        _ => panic!("no service"),
    };
    assert_eq!(now, original);
    assert!(review(&p, crate::undo::Kind::HowItWas).is_empty());
    // the closed tab's "Steam’s layout": the games the app changed (Rocket League is the user's own layout, not Steam's)
    assert_eq!(texts(&review(&p, crate::undo::Kind::WindowsDefaults)), ["Rocket League \u{b7} DualSense Edge: your layout  \u{2192}  Steam\u{2019}s layout"]);
}

#[test]
fn change_log_items_keep_their_ids() {
    for it in [data::Item::Layout(PadKind::DualSense, "252950".into()), data::Item::Layout(PadKind::Xbox, "my game: 2".into()), data::Item::Prefs("X1".into())] {
        assert_eq!(data::Item::parse(&it.id()).map(|p| p.id()), Some(it.id()));
    }
    // the DualSense and the Edge share their layout files: one item
    assert_eq!(data::Item::Layout(PadKind::DualSense, "1".into()).id(), data::Item::Layout(PadKind::DualSenseEdge, "1".into()).id());
    assert_eq!(data::Item::parse("nope"), None);
}

#[test]
fn a_community_layout_becomes_your_own_copy() {
    let mut t = T::new();
    let gi = t.o().games.iter().position(|g| g.is_community()).expect("the fake has a community layout");
    t.click(K_GAME);
    t.click(idx(K_MENU, gi));
    assert!(t.o().game().unwrap().is_community());
    t.click_part(Pid::B(ButtonId::Cross));
    t.click(Open::k("cross.turbo"));
    let w = t.writes();
    assert!(w.iter().any(|l| l.contains("configset_controller_ps5.vdf")), "the index points at the own copy: {w:?}");
    assert!(!t.o().game().unwrap().is_community(), "it is the user's own copy now");
}

#[test]
fn another_controller_type_and_action_sets() {
    let mut t = T::new();
    t.click(K_SET);
    let n = t.o().sets.len();
    t.click(idx(K_MENU, n.max(1))); // New action set…
    assert_eq!(t.o().sets.len(), n + 1);
    assert!(t.toast().contains("made"));
    // the Xbox outline: no gyro / touchpad / light (the fake has Rocket League's layout for every type)
    t.click(K_DEV);
    let xi = PadKind::ALL.iter().position(|k| *k == PadKind::Xbox).unwrap();
    t.click(idx(K_MENU, xi));
    assert_eq!(t.o().kind, PadKind::Xbox);
    assert_eq!(t.o().game().map(|g| g.name.clone()).as_deref(), Some("Rocket League"));
    assert!(t.o().view.as_ref().unwrap().gyro.is_none() && t.o().view.as_ref().unwrap().touchpad.is_none());
    assert!(t.o().pic.parts.iter().all(|p| p.id != Pid::Touch));
    t.click_part(Pid::B(ButtonId::Cross));
    t.build();
}

#[test]
fn open_in_steam_never_opens_anything_in_a_test_copy() {
    let mut t = T::new();
    t.click(K_OPENSTEAM);
    assert!(t.toast().contains("Steam"));
}

#[test]
fn closing_drops_everything() {
    let mut t = T::new();
    t.p.close();
    assert!(t.p.o.is_none());
    assert!(!t.p.tick(5000.0));
}

#[test]
fn every_part_of_every_controller_builds_its_panel() {
    for k in PadKind::ALL {
        let mut t = T::new();
        if k != PadKind::DualSenseEdge {
            t.click(K_DEV);
            t.click(idx(K_MENU, PadKind::ALL.iter().position(|x| *x == k).unwrap()));
        }
        let ids: Vec<Pid> = t.o().pic.parts.iter().map(|p| p.id).filter(|p| *p != Pid::Light).collect();
        for id in ids {
            t.click_part(id);
            assert_eq!(t.o().sel, Some(id), "{k:?} {id:?}");
        }
    }
}

#[test]
fn dragging_the_dead_zone_rings() {
    let mut t = T::new();
    t.click_part(Pid::Stick(Side::Left));
    let k = Open::k("ls.well");
    let r = (0.0, 0.0, 136.0, 118.0); // the circle at 1:1 (centre 68 / 59, R 48)
    // press near the outer ring (78 %): it moves; drag to 90 % of the radius
    t.ev(Ev::Press(k, 68.0 + 48.0 * 0.77, 59.0, r));
    t.ev(Ev::Drag(k, 68.0 + 48.0 * 0.9, 59.0, r));
    t.ev(Ev::Release(k));
    let want = bu_controller::settings::pct_to_radius(90.0);
    assert!(t.rl().contains(&format!("\"deadzone_outer_radius\"		\"{want}\"")), "{}", t.rl());
    // press at the centre: the inner ring (10 %) moves to 4 %
    t.ev(Ev::Press(k, 68.0 + 48.0 * 0.04, 59.0, r));
    t.ev(Ev::Release(k));
    let want = bu_controller::settings::pct_to_radius(4.0);
    assert!(t.rl().contains(&format!("\"deadzone_inner_radius\"		\"{want}\"")), "{}", t.rl());
}

#[test]
fn re_picking_what_is_set_writes_nothing() {
    let mut t = T::new();
    t.click_part(Pid::Trig(Side::Right));
    let n = t.writes().len();
    // the segment that is on (Analog) and the menu item that is set (Curve: Linear)
    t.click(idx(Open::k("r2.mode"), 0));
    t.click(Open::k("r2.curve"));
    t.click(idx(K_MENU, 0));
    assert_eq!(t.writes().len(), n, "{:?}", t.writes());
    assert!(t.toast().is_empty());
}

#[test]
fn a_stick_mode_the_list_doesnt_have_is_never_rewritten() {
    let mut t = T::new();
    // a layout whose right stick is in a mode the page doesn't list (written straight through the crate, like Steam would)
    let key = t.o().game().unwrap().key.clone();
    let other = StickMode::Other("mouse_region".into());
    t.o().svc_do(|s| s.apply_all(&key, PadKind::DualSenseEdge, 0, &[Change::StickMode { side: Side::Right, mode: other.clone() }])).unwrap().unwrap();
    t.o().reload();
    t.settle();
    let before = t.rl();
    t.click_part(Pid::Stick(Side::Right));
    t.click(Open::k("rs.mode"));
    let n = match t.o().ctl.get(&Open::k("rs.mode")) {
        Some(Ctl::Menu { items, .. }) => items.len(),
        _ => 0,
    };
    assert_eq!(n, 10, "the 9 modes + the layout's own");
    t.click(idx(K_MENU, 9)); // the shown, unlisted one
    assert_eq!(t.rl(), before, "nothing rewritten");
    assert_eq!(t.o().view.as_ref().unwrap().sticks[1].mode, other);
}

#[test]
fn the_dead_zone_stays_under_full_at() {
    let mut t = T::new();
    t.click_part(Pid::Stick(Side::Left)); // dead zone 10 %, full at 78 %
    t.slide("ls.full", 0.0); // the slider's bottom: 40 %
    t.slide("ls.dz", 1.0); // 60 % wanted, kept at "full at" - 5 (full at as read back from the file: 13107 = 40.0006 %)
    let want = bu_controller::settings::pct_to_radius(bu_controller::settings::radius_to_pct(13107) - 5.0);
    assert!(t.rl().contains(&format!("\"deadzone_inner_radius\"\t\t\"{want}\"")), "{}", t.rl());
}

/// Straight into the fake Steam's file (like Steam or another program would write it), then read back.
fn steam_writes(t: &mut T, c: Vec<Change>) {
    let key = t.o().game().unwrap().key.clone();
    t.o().svc_do(|s| s.apply_all(&key, PadKind::DualSenseEdge, 0, &c)).unwrap().unwrap();
    t.o().reload();
    t.settle();
    t.build();
}

#[test]
fn odd_dead_zones_in_a_layout_never_crash_the_sliders_or_the_circle() {
    let mut t = T::new();
    let r = |p: f64| Some(bu_controller::settings::pct_to_radius(p));
    steam_writes(&mut t, vec![
        Change::StickSetting { side: Side::Left, setting: StickSetting::DeadZone, value: r(97.0) },
        Change::StickSetting { side: Side::Left, setting: StickSetting::FullAt, value: r(2.0) },
    ]);
    t.click_part(Pid::Stick(Side::Left));
    // both sliders and the circle: no panic (f64::clamp with min > max would abort the app)
    t.slide("ls.dz", 0.5);
    t.slide("ls.full", 0.5);
    let k = Open::k("ls.well");
    let wr = (0.0, 0.0, 136.0, 118.0);
    t.ev(Ev::Press(k, 68.0 + 20.0, 59.0, wr));
    t.ev(Ev::Drag(k, 68.0 + 40.0, 59.0, wr));
    t.ev(Ev::Release(k));
    t.ev(Ev::Press(k, 68.0, 59.0, wr));
    t.ev(Ev::Release(k));
    assert_eq!(keep_in(50.0, (0.0, 60.0), (0.0, -3.0)), 0.0);
    assert_eq!(keep_in(50.0, (40.0, 100.0), (102.0, 100.0)), 100.0);
}

#[test]
fn a_touch_mode_the_segments_dont_list_shows_none_and_nothing_writes() {
    let mut t = T::new();
    steam_writes(&mut t, vec![Change::TouchMode { mode: TouchMode::Other("touch_menu".into()) }]);
    t.click_part(Pid::Touch);
    let n = t.writes().iter().filter(|w| w.contains(r"Steam\steamapps")).count();
    t.click(idx(Open::k("tp.touch"), 0)); // "Nothing"
    let m = t.writes().iter().filter(|w| w.contains(r"Steam\steamapps")).count();
    assert_eq!(m, n + 1, "one write");
    assert_eq!(t.o().view.as_ref().unwrap().touchpad.as_ref().unwrap().touch, TouchMode::Nothing);
}

#[test]
fn trigger_haptics_shows_the_files_own_value() {
    let mut t = T::new();
    steam_writes(&mut t, vec![Change::TriggerSetting { side: Side::Right, setting: TriggerSetting::Haptics, value: Some(5) }]);
    t.click_part(Pid::Trig(Side::Right));
    let Some(Ctl::Menu { items, cur, .. }) = t.o().ctl.get(&Open::k("r2.hap")).cloned() else { panic!("no haptics menu") };
    assert_eq!(cur, Some(5));
    assert!(items.iter().any(|(v, _)| *v == Some(5)));
    // "Off" is a real change now: one write
    let n = t.writes().len();
    t.click(Open::k("r2.hap"));
    t.click(idx(K_MENU, 0));
    assert!(t.writes().len() > n);
}

#[test]
fn picking_another_controller_fades_the_picture_in() {
    let mut t = T::new();
    t.click(K_DEV);
    let mut cx = Cx::new(t.now, false, &t.g, &mut t.st);
    let i = PadKind::ALL.iter().position(|k| *k == PadKind::DualShock4).unwrap();
    t.p.event(&Ev::Click(idx(K_MENU, i)), &mut cx);
    assert!(t.p.o.as_ref().unwrap().dev_at.is_some(), "the fade + scale-in started");
}

#[test]
fn unlisted_menu_values_are_their_own_item() {
    let mut t = T::new();
    // the trigger curve's 5 (Custom) is not in the trigger's list: shown as its own item, picking it writes nothing
    steam_writes(&mut t, vec![Change::TriggerSetting { side: Side::Right, setting: TriggerSetting::Curve, value: Some(5) }]);
    t.click_part(Pid::Trig(Side::Right));
    let Some(Ctl::Menu { items, cur, .. }) = t.o().ctl.get(&Open::k("r2.curve")).cloned() else { panic!() };
    assert_eq!(cur, Some(5));
    let i = items.iter().position(|(v, _)| *v == Some(5)).expect("its own item");
    assert_eq!(items[i].1, "Other (5)");
    let n = t.writes().len();
    t.click(Open::k("r2.curve"));
    t.click(idx(K_MENU, i));
    assert_eq!(t.writes().len(), n);
}

#[test]
fn esc_goes_back_to_the_big_controller() {
    let mut t = T::new();
    t.click_part(Pid::Stick(Side::Left));
    t.ev(Ev::Key(crate::ui::cx::PAGE, 0x1B));
    assert_eq!(t.o().sel, None);
}

#[test]
fn the_action_list_opens_at_the_current_item_and_scrolls() {
    let mut t = T::new();
    t.click_part(Pid::B(ButtonId::BackRightUpper)); // R4 = F5 (a key: far down the list)
    t.click(Open::k("backrightupper.full"));
    let off = t.st.scroll_y.get(&sub(K_ACT, "list")).copied().unwrap_or(0.0);
    assert!(off > 250.0, "scrolled to F5: {off}");
    // the next build keeps the user's own scroll (the wheel)
    t.st.scroll_y.insert(sub(K_ACT, "list"), 10.0);
    t.build();
    assert_eq!(t.st.scroll_y[&sub(K_ACT, "list")], 10.0);
}

/// Debug pictures of the four controller outlines (run on purpose: `BU_PIC_OUT=<folder> cargo test -p bu-app pad_pictures --
/// --ignored`); nothing on screen.
#[test]
#[ignore]
fn pad_pictures() {
    let Ok(dir) = std::env::var("BU_PIC_OUT") else { return };
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    let g = Gfx::new(1.0);
    for (n, k) in [("dse", PadKind::DualSenseEdge), ("ds", PadKind::DualSense), ("ds4", PadKind::DualShock4), ("xb", PadKind::Xbox)] {
        let p = pic::pic(k);
        // twice the size, so the outline is easy to judge
        let (w, h) = (pic::PIC_W * 2.0, p.h * 2.0);
        let mut s = crate::gfx::new_surface(w as i32, h as i32 + 8).unwrap();
        g.begin(s.canvas());
        g.fill_rect(0.0, 0.0, w, h + 8.0, crate::gfx::Rgba::rgb(20, 24, 40));
        pic::paint(&g, &p, (0.0, 0.0, w), &|_| pic::Look::default(), None);
        g.end();
        let px = crate::png::from_surface(&mut s);
        crate::png::save_png(&px, &format!("{dir}/{n}.png")).expect("save");
    }
}

/// the owner (test build 1): L1 / R1 "sticks into the controller" - "lock in move them both up". On every outline: L1 / R1
/// sit SHOULDER_GAP above the body's outline, L2 / R2 the same gap above them, all inside the picture (painted edges).
#[test]
fn shoulder_buttons_sit_above_the_body_on_every_controller() {
    for k in PadKind::ALL {
        let p = pic::pic(k);
        let rect = |i: usize| match p.parts[i].s[0] {
            pic::Sh::Rect { x, y, w, h, .. } => (x, y, w, h),
            _ => panic!("{k:?}: part {i} is not a tab"),
        };
        let (l2, r2, l1, r1) = (rect(0), rect(1), rect(2), rect(3));
        assert_eq!((l2.1, l1.1), (r2.1, r1.1), "{k:?}: mirrored");
        let body_top = pic::top_between(&p.body, l1.0, l1.0 + l1.2) - 0.65;
        let gap1 = body_top - (l1.1 + l1.3 + 0.6);
        let gap2 = (l1.1 - 0.6) - (l2.1 + l2.3 + 0.6);
        assert!((gap1 - pic::SHOULDER_GAP).abs() < 0.01, "{k:?}: L1 to body {gap1}");
        assert!((gap2 - pic::SHOULDER_GAP).abs() < 0.01, "{k:?}: L2 to L1 {gap2}");
        assert!(l2.1 - 0.6 >= -0.001, "{k:?}: L2 leaves the picture ({})", l2.1);
        // the bottom keeps the drawing's room under the body
        assert!(p.h >= pic::PIC_H && p.h - pic::PIC_H < 12.0, "{k:?}: h {}", p.h);
        // the tab labels stay centred in their tabs
        let t = p.parts[2].t.as_ref().unwrap();
        assert!((t.y - (l1.1 + l1.3 / 2.0 + 3.0)).abs() <= 0.05, "{k:?}");
    }
}

#[test]
fn a_path_moves_down_without_changing_its_shape() {
    assert_eq!(pic::shift_path("M151.5 39.6H328.5L318.6 116Q317 126.8 306 126.8Z", 2.0), "M151.5 41.6H328.5L318.6 118.0Q317 128.8 306 128.8Z");
    assert_eq!(pic::shift_path("M234 66.8l12 12M246 66.8l-12 12", 1.5), "M234 68.3l12 12M246 68.3l-12 12");
    assert_eq!(pic::shift_path("M1 2C3 4 5 6 7 8V9", 1.0), "M1 3.0C3 5.0 5 7.0 7 9.0V10.0");
}

/// the owner (test build 1): "havinog to scroll down on the page of controller just for the little bit of text at the bottom is
/// stupid (when nothing is selected) simply move the bottom text parts up so no scroll is needed" - every controller's
/// page fits the menu's page area (520 - 56 = 464 px) with nothing picked.
#[test]
fn nothing_picked_needs_no_scrolling() {
    for k in PadKind::ALL {
        let mut t = T::new();
        t.o().kind = k;
        t.o().pic = pic::pic(k);
        let kids = t.build();
        let page = crate::ui::lay::Laid::new(&t.g, El::block().w(WIN_W).pad(2.0, 26.0, 18.0, 26.0).children(kids), WIN_W, None);
        assert!(
            page.height <= crate::ui::PAGE_H,
            "{k:?}: the page is {} px tall: pic {:?} gyro {:?} ctlset {:?} reset {:?}",
            page.height,
            page.rect_of(K_PIC),
            page.rect_of(K_GYRO),
            page.rect_of(K_CTLSET),
            page.rect_of(sub(K_RESET, "pc"))
        );
        let (_, ry, _, rh) = page.rect_of(K_CTLSET).unwrap();
        assert!(ry + rh <= crate::ui::PAGE_H, "{k:?}: Controller settings at {ry}");
    }
}

/// The links, the Gyro chip and the hint moved into the picture sit in the empty space between the grips: no point of
/// the body's outline or its lines comes within 3 px of them, on any controller.
#[test]
fn the_parts_under_the_arch_stay_clear_of_the_outline() {
    for k in PadKind::ALL {
        let mut t = T::new();
        t.o().kind = k;
        t.o().pic = pic::pic(k);
        let kids = t.build();
        let page = crate::ui::lay::Laid::new(&t.g, El::block().w(WIN_W).pad(2.0, 26.0, 18.0, 26.0).children(kids), WIN_W, None);
        let (px, py, pw, _) = page.rect_of(K_PIC).unwrap();
        let s = pw / pic::PIC_W;
        let p = t.o().pic.clone();
        let mut pts = Vec::new();
        for d in [&p.body, &p.lines] {
            let path = crate::svg::to_path(&crate::svg::parse(d));
            for c in skia_safe::ContourMeasureIter::new(&path, false, None) {
                let n = (c.length() / 0.5).ceil() as usize;
                for i in 0..=n {
                    if let Some((q, _)) = c.pos_tan(c.length() * i as f32 / n.max(1) as f32) {
                        pts.push((px + q.x * s, py + q.y * s));
                    }
                }
            }
        }
        let mut boxes = vec![page.rect_of(K_CTLSET).unwrap(), page.rect_of(K_OPENSTEAM).unwrap()];
        if k.has_gyro() {
            boxes.push(page.rect_of(K_GYRO).expect("the Gyro chip"));
        }
        for (bx, by, bw, bh) in boxes {
            let hit = pts.iter().find(|(x, y)| *x > bx - 3.0 && *x < bx + bw + 3.0 && *y > by - 3.0 && *y < by + bh + 3.0);
            assert!(hit.is_none(), "{k:?}: the outline at {hit:?} touches the box {:?}", (bx, by, bw, bh));
        }
    }
}

/// the owner (test build 1): "why can i change them when i don't have a controller plugged in" - Controller settings (and the
/// light bar, which opens them) do nothing while the shown controller is not plugged in.
#[test]
fn controller_settings_need_the_controller_plugged_in() {
    let mut t = T::new();
    t.o().pads.clear();
    assert!(t.o().connected().is_none());
    t.build();
    t.click(K_CTLSET);
    assert!(t.o().dlg.is_none());
    t.click_part(Pid::Light);
    assert!(t.o().dlg.is_none() && t.o().sel.is_none());
    // plugged in again: they open
    let mut u = T::new();
    u.click(K_CTLSET);
    assert!(u.o().dlg.is_some());
}

// ------------------------------------------------------------------------------------------------ Order 042 (test build 2)

impl T {
    /// The popup layer as the frame lays it out (window coordinates).
    fn popup_laid(&mut self) -> crate::ui::lay::Laid {
        self.now += 2000.0;
        let mut cx = Cx::new(self.now, false, &self.g, &mut self.st).for_page("pad");
        let pop = self.p.popup(&mut cx).expect("a popup");
        crate::ui::lay::Laid::new(&self.g, El::block().w(WIN_W).h(crate::ui::WIN_H).child(pop), WIN_W, Some(crate::ui::WIN_H))
    }
    /// The keys under a window point of the popup layer (innermost first).
    fn hit_keys(&mut self, x: f32, y: f32) -> Vec<Key> {
        self.popup_laid().hit(x, y).map(|h| h.1).unwrap_or_default()
    }
}

/// the owner (test build 2): "the, acts as joystick option on top ... it can't be changed". The list a control opens inside the
/// part window lies ABOVE the window (it was drawn and hit under the window's layer): a click on its item reaches the item
/// and the stick's mode (Steam's own group `mode` in the layout) changes.
#[test]
fn a_list_opened_in_the_part_window_is_above_it_and_changes_the_stick_mode() {
    let mut t = T::new();
    t.click_part(Pid::Stick(Side::Left));
    t.click(Open::k("ls.mode"));
    assert_eq!(t.o().pop, Some(Pop::Menu(Open::k("ls.mode"))));
    let l = t.popup_laid();
    let (x, y, w, h) = l.rect_of(idx(K_MENU, 2)).expect("the list's third item (As mouse)");
    let (_, keys) = l.hit(x + w / 2.0, y + h / 2.0).expect("something under the item");
    assert_eq!(keys.first(), Some(&idx(K_MENU, 2)), "the item itself is hit, not the window under it: {keys:?}");
    // the same for the Controller settings' list (gyro noise filter)
    t.click(idx(K_MENU, 2));
    assert_eq!(t.o().view.as_ref().unwrap().sticks[0].mode, StickMode::Mouse);
    assert!(t.rl().contains("\"mode\"\t\t\"joystick_mouse\""), "Steam's own value in the file");
    t.click(sub(K_PANEL, "x"));
    t.click(K_CTLSET);
    t.click(Open::k("pf.noise"));
    let l = t.popup_laid();
    let (x, y, w, h) = l.rect_of(idx(K_MENU, 0)).unwrap();
    assert_eq!(l.hit(x + w / 2.0, y + h / 2.0).map(|h| h.1[0]), Some(idx(K_MENU, 0)));
}

/// the owner (test build 2): "outer ring, haptics, etc, are closed down menus ... rather than being open inside of left stick":
/// every group of a part window is open - its controls are laid out (have a box) without any click.
#[test]
fn a_part_windows_groups_are_open() {
    let mut t = T::new();
    t.click_part(Pid::Stick(Side::Left));
    let l = t.popup_laid();
    for id in ["ls.ring", "ls.rr", "ls.rinv", "ls.hap", "ls.anti", "ls.smooth"] {
        let r = l.rect_of(Open::k(id)).unwrap_or_else(|| panic!("{id} is laid out"));
        assert!(r.3 > 0.0, "{id} has a height: {r:?}");
    }
    t.click(sub(K_PANEL, "x"));
    t.click_part(Pid::B(ButtonId::Cross));
    let l = t.popup_laid();
    assert!(l.rect_of(Open::k("cross.hap")).is_some_and(|r| r.3 > 0.0) && l.rect_of(Open::k("cross.start")).is_some_and(|r| r.3 > 0.0));
}

impl T {
    /// A key with no element focused (Ctrl / Shift held as given).
    fn key(&mut self, vk: u16, ctrl: bool, shift: bool) {
        let mut cx = Cx::new(self.now, false, &self.g, &mut self.st).for_page("pad");
        cx.mods = crate::ui::cx::Mods { ctrl, shift, alt: false };
        self.p.event(&Ev::Key(crate::ui::cx::PAGE, vk), &mut cx);
        self.settle();
        self.build();
    }
    fn undo(&mut self) {
        self.key(0x5A, true, false);
    }
    fn redo(&mut self) {
        self.key(0x59, true, false);
    }
    fn left_stick(&mut self) -> bu_controller::settings::StickView {
        self.o().view.as_ref().unwrap().sticks[0].clone()
    }
    /// The fake Steam started / closed (its process switch), seen by the tab's next frame.
    fn steam_running(&mut self, on: bool) {
        self.set_steam(on, 0);
        let now = self.now;
        self.p.tick(now);
        self.settle();
        self.build();
    }
    /// The fake Steam started / closed (its process switch), its files answering `slow` ms per read from now on; seen by
    /// the tab's next frame step.
    fn set_steam(&mut self, on: bool, slow: u64) {
        let mut f = data::fake_steam();
        f.running = on;
        let slot = self.o().svc.clone();
        let mut g = slot.lock().unwrap();
        // the same files as now (what the tab wrote so far stays)
        if let Some(Ok(data::Svc::Fake(s))) = &*g {
            for (p, b) in s.os().files.lock().unwrap().values() {
                f.put(p, b);
            }
        }
        let s = data::Svc::Fake(bu_controller::ControllerService::new(f, data::BACKUPS).unwrap());
        s.set_fake_delay(slow);
        *g = Some(Ok(s));
    }
    /// The page as the frame lays it out (window x / y of the page's content box).
    fn page_laid(&mut self) -> crate::ui::lay::Laid {
        self.now += 2000.0;
        let mut cx = Cx::new(self.now, false, &self.g, &mut self.st).for_page("pad");
        let kids = self.p.build(&mut cx);
        crate::ui::lay::Laid::new(&self.g, El::block().w(WIN_W).pad(2.0, 26.0, 18.0, 26.0).children(kids), WIN_W, None)
    }
}

/// the owner (test build 2): "i shouldn't be able to change settings unless steam is launched, yet i can. there needs to be some
/// sort of pop up over the controllers, blocking out pressing anything, by saying please launch steam".
#[test]
fn steam_closed_covers_the_tab_and_nothing_under_it_changes() {
    let mut t = T::new();
    t.click_part(Pid::Stick(Side::Left));
    t.steam_running(false);
    assert!(!t.o().steam_up);
    assert!(t.o().sel.is_none(), "the part's window closed under the glass");
    // the glass lies over the picture: a press there reaches the glass, not a part
    let l = t.page_laid();
    let (x, y, w, h) = l.rect_of(K_PIC).unwrap();
    for (fx, fy) in [(0.2, 0.3), (0.5, 0.5), (0.8, 0.8)] {
        let keys = l.hit(x + w * fx, y + h * fy).map(|h| h.1).unwrap_or_default();
        assert!(keys.contains(&K_GATE) && !keys.contains(&K_PIC), "{keys:?}");
    }
    // the reset line and the pickers too
    let (rx, ry, rw, rh) = l.rect_of(sub(K_RESET, "pc")).unwrap();
    assert!(l.hit(rx + rw / 2.0, ry + rh / 2.0).unwrap().1.contains(&K_GATE));
    let (gx, gy, gw, gh) = l.rect_of(K_GAME).unwrap();
    assert!(l.hit(gx + gw / 2.0, gy + gh / 2.0).unwrap().1.contains(&K_GATE));
    let (bx, by, bw, bh) = l.rect_of(K_LAUNCH).expect("the Launch Steam button");
    assert_eq!(l.hit(bx + bw / 2.0, by + bh / 2.0).unwrap().1[0], K_LAUNCH);
    // nothing reaches the controls (even events sent straight to them), nothing is written
    let n = t.writes().len();
    t.click_part(Pid::Stick(Side::Left));
    t.click(K_GYRO);
    t.click(K_GAME);
    t.click(K_CTLSET);
    t.slide("ls.dz", 0.5);
    t.undo();
    assert!(t.o().sel.is_none() && t.o().pop.is_none() && t.o().dlg.is_none());
    assert_eq!(t.writes().len(), n);
    // "Launch Steam": a test copy starts nothing (says so); the button waits
    t.click(K_LAUNCH);
    assert!(t.toast().contains("Starts Steam"), "{}", t.toast());
    assert!(t.o().launch_at.is_some());
    // Steam runs: the glass goes by itself, the controls work again
    t.steam_running(true);
    assert!(t.o().steam_up && t.o().launch_at.is_none());
    assert!(t.page_laid().rect_of(K_GATE).is_none());
    t.click_part(Pid::Stick(Side::Left));
    assert_eq!(t.o().sel, Some(Pid::Stick(Side::Left)));
}

/// The real copy watches Steam's process only while the tab is open: dropping the watch (the tab closes) tells its thread
/// at once (its channel closes); reading the process list changes nothing.
#[test]
fn the_steam_watch_ends_with_the_tab() {
    let w = data::SteamWatch::start(crate::services::Waker, true);
    let _ = w.up();
    let t0 = std::time::Instant::now();
    drop(w);
    assert!(t0.elapsed() < std::time::Duration::from_millis(100));
}

/// the owner (test build 2): "the controller settings, really need a ctrl z type of thing, i just changed my deadzone by accident
/// and can't remember what it was on before". Ctrl+Z puts the value back (written like the change), Ctrl+Y / Ctrl+Shift+Z
/// does it again; the row shows "Undo" while it holds the last change.
#[test]
fn ctrl_z_undoes_a_dead_zone_change_and_ctrl_y_redoes_it() {
    let mut t = T::new();
    t.click_part(Pid::Stick(Side::Left));
    let (orig, orig_file) = (t.left_stick(), t.rl());
    t.slide("ls.dz", 0.5);
    let changed = t.left_stick();
    assert_ne!(changed, orig);
    let dzk = Open::k("ls.dz");
    assert!(matches!(t.o().ctl.get(&sub(dzk, "undo")), Some(Ctl::Undo)), "the dead zone row shows Undo");
    let n = t.writes().len();
    t.undo();
    assert_eq!(t.left_stick(), orig, "the stick is as before");
    assert_eq!(t.rl(), orig_file, "the file too");
    assert!(t.writes().len() > n, "written through bu-controller like the change");
    assert_eq!(t.toast(), "Undone \u{b7} Left stick \u{b7} Dead zone");
    assert!(!t.o().ctl.contains_key(&sub(dzk, "undo")));
    t.redo();
    assert_eq!(t.left_stick(), changed);
    assert_eq!(t.toast(), "Redone \u{b7} Left stick \u{b7} Dead zone");
    // the row's "Undo" link does the same as Ctrl+Z
    t.click(sub(dzk, "undo"));
    assert_eq!(t.left_stick(), orig);
    t.key(0x5A, true, true); // Ctrl+Shift+Z = redo
    assert_eq!(t.left_stick(), changed);
    // a new change clears the redo list
    t.undo();
    t.click(Open::k("ls.ix"));
    t.redo();
    assert_eq!(t.toast(), "Nothing to redo");
    // nothing left: says so, writes nothing
    t.undo();
    let n = t.writes().len();
    t.undo();
    assert_eq!(t.toast(), "Nothing to undo");
    assert_eq!(t.writes().len(), n);
    assert_eq!(t.left_stick(), orig);
}

/// Every kind of change undoes: a switch, an action, the stick's mode, a segment, "Steam's setting for this", a list item,
/// a Controller settings value (its file byte for byte), a new action set.
#[test]
fn every_kind_of_change_undoes() {
    let mut t = T::new();
    let view0 = t.o().view.clone().unwrap();
    t.click_part(Pid::B(ButtonId::Square));
    t.click(Open::k("square.turbo"));
    t.click(Open::k("square.full"));
    for c in "f6".chars() {
        t.ev(Ev::Char(K_ASRCH, c));
    }
    t.ev(Ev::Key(K_ASRCH, 0x0D));
    assert!(t.rl().contains("key_press F6"));
    t.click(sub(K_PANEL, "x"));
    t.click_part(Pid::Stick(Side::Left));
    t.click(Open::k("ls.mode"));
    t.click(idx(K_MENU, 2));
    t.click(idx(Open::k("ls.shape"), 2));
    t.click(K_STEAMSET);
    t.click(sub(K_PANEL, "x"));
    t.click_part(Pid::Trig(Side::Right));
    t.click(Open::k("r2.curve"));
    t.click(idx(K_MENU, 1));
    assert_eq!(t.o().hist.undo.len(), 6);
    for _ in 0..6 {
        t.undo();
    }
    assert_eq!(t.o().view.clone().unwrap(), view0, "every part as it was");
    assert!(!t.rl().contains("key_press F6"));
    // the controller's own file: back byte for byte
    let prefs0 = t.prefs();
    t.click(K_CTLSET);
    t.click(Open::k("pf.anti"));
    t.click(Open::k("pf.rumble"));
    t.click(idx(Open::k("pf.lc"), 1));
    assert_ne!(t.prefs(), prefs0);
    assert!(matches!(t.o().ctl.get(&sub(Open::k("pf.lc"), "undo")), Some(Ctl::Undo)));
    t.undo();
    t.undo();
    t.undo();
    assert_eq!(t.prefs(), prefs0);
    t.click(sub(K_DLG, "x"));
    // a new action set: taken back by bu-controller's own exact undo, made again by redo
    let sets = t.o().sets.len();
    t.click(K_SET);
    t.click(idx(K_MENU, sets.max(1)));
    assert_eq!(t.o().sets.len(), sets + 1);
    assert!(matches!(t.o().ctl.get(&sub(K_SET, "undo")), Some(Ctl::Undo)), "Undo next to the set's button");
    t.undo();
    assert_eq!(t.o().sets.len(), sets);
    t.redo();
    assert_eq!(t.o().sets.len(), sets + 1);
}

/// The undo list lasts while the app runs: the tab closed and opened again (the menu closed) still undoes.
#[test]
fn undo_survives_closing_the_tab() {
    let env = Env { test: true, frozen: true, ..Env::default() };
    let mut t = T::new();
    t.p.close();
    t.p.open(&env, 0.0);
    t.settle();
    t.build();
    t.click_part(Pid::Stick(Side::Left));
    let orig = t.left_stick();
    t.slide("ls.dz", 0.5);
    t.p.close();
    t.p.open(&env, 0.0);
    t.settle();
    t.build();
    assert_eq!(t.o().hist.undo.len(), 1);
    t.undo();
    assert_eq!(t.left_stick(), orig);
}

/// "each value can go back to Steam's default": a row whose value differs from Steam's layout has its small reset; it
/// writes Steam's value for that one setting only (one undoable change).
#[test]
fn a_single_value_goes_back_to_steams() {
    let mut t = T::new();
    t.click_part(Pid::Stick(Side::Left));
    let backs: Vec<Key> = t.o().ctl.iter().filter(|(_, c)| matches!(c, Ctl::Back(_))).map(|(k, _)| *k).collect();
    assert!(!backs.is_empty(), "the user's layout's stick differs from Steam's somewhere");
    let st = t.o().steam.clone().unwrap().sticks[0].clone();
    let before = t.left_stick();
    let k = backs[0];
    t.click(k);
    let after = t.left_stick();
    // at most one setting moved, and it moved to Steam's value
    let moved: Vec<_> = before.settings.iter().zip(&after.settings).filter(|(a, b)| a != b).map(|(_, b)| *b).collect();
    assert!(moved.len() <= 1, "{moved:?}");
    for (s, v) in &moved {
        assert_eq!(sv(&st.settings, *s), *v);
    }
    assert!(!t.o().ctl.contains_key(&k), "same as Steam's now: no reset");
    t.undo();
    assert_eq!(t.left_stick(), before);
    // an action ("does" chip): R4 = F5 in the user's layout
    t.click(sub(K_PANEL, "x"));
    t.click_part(Pid::B(ButtonId::BackRightUpper));
    let bk = sub(Open::k("backrightupper.full"), "back");
    assert!(matches!(t.o().ctl.get(&bk), Some(Ctl::Back(_))), "R4 differs from Steam's");
    t.click(bk);
    let r4 = |v: &PadView| v.buttons.iter().find(|b| b.id == ButtonId::BackRightUpper).unwrap().presses[0].1.clone();
    let steam = t.o().steam.clone().unwrap();
    assert_eq!(r4(t.o().view.as_ref().unwrap()), r4(&steam));
    // the controller's own file: Steam's default = the value taken out
    t.click(sub(K_PANEL, "x"));
    t.click(K_CTLSET);
    let ak = sub(Open::k("pf.anti"), "back");
    assert!(matches!(t.o().ctl.get(&ak), Some(Ctl::Back(_))));
    t.click(ak);
    assert!(!t.prefs().contains("antidrift_enabled_sw"), "{}", t.prefs());
    assert!(!t.o().ctl.contains_key(&sub(Open::k("pf.rumble"), "back")), "rumble on = Steam's default: no reset");
}

#[test]
fn the_suggested_dead_zone_is_just_above_the_drift() {
    assert_eq!(drift_dz(0.029, 100.0), 5.0);
    assert_eq!(drift_dz(0.031, 100.0), 6.0);
    assert_eq!(drift_dz(0.0, 100.0), 2.0);
    assert_eq!(drift_dz(0.9, 100.0), 60.0, "the slider's top");
    assert_eq!(drift_dz(0.5, 40.0), 35.0, "under full at - 5");
}

/// Order 042 item 11 (the owner, test build 2: "some sort of cool setting to show like, calculate stick drift"): hands off for
/// 5 s, the largest distance from the centre = the drift, a dead zone just above it, one "Use it" (undoable). Nothing is
/// written before "Use it".
#[test]
fn check_stick_drift_with_a_made_up_drift() {
    let mut t = T::new();
    t.click_part(Pid::Stick(Side::Left));
    let n = t.writes().len();
    t.click(Open::k("ls.drift"));
    let Some(Drift::Run { at, .. }) = t.o().drift else { panic!("runs") };
    // a stick resting a little off the centre, wobbling (largest 3.1 %)
    let wobble = [(0.012, -0.004), (0.02, 0.01), (-0.018, 0.025), (0.0, 0.031), (0.01, 0.0), (0.024, -0.012)];
    for (i, (x, y)) in wobble.iter().enumerate() {
        t.o().lv.ls = (*x, *y);
        assert!(t.p.tick(at + 400.0 + 700.0 * i as f64), "frames while it runs");
    }
    assert!(matches!(t.o().drift, Some(Drift::Run { .. })), "still running before 5 s");
    t.o().lv.ls = (0.0, 0.0);
    t.p.tick(at + DRIFT_MS + 1.0);
    let Some(Drift::Done { max, dz, .. }) = t.o().drift else { panic!("done") };
    assert!((max - 0.031).abs() < 1e-6, "{max}");
    assert_eq!(dz, 6.0);
    assert_eq!(t.writes().len(), n, "measuring writes nothing");
    t.build();
    // "Use it": a normal dead-zone change, undoable
    let before = t.left_stick();
    t.click(Open::k("ls.drift.use"));
    let want = bu_controller::settings::pct_to_radius(6.0);
    assert!(t.rl().contains(&format!("\"deadzone_inner_radius\"\t\t\"{want}\"")), "{}", t.rl());
    assert!(t.o().drift.is_none());
    assert!(matches!(t.o().ctl.get(&sub(Open::k("ls.dz"), "undo")), Some(Ctl::Undo)), "Undo next to the dead zone");
    t.undo();
    assert_eq!(t.left_stick(), before);
}

/// The fake controller's live loop (a test copy) wobbles the stick a little while the check runs: about 3 % -> 5 %.
#[test]
fn the_fake_controller_drifts_a_little_while_checked() {
    let mut t = T::live();
    t.click_part(Pid::Stick(Side::Right));
    t.click(Open::k("rs.drift"));
    let Some(Drift::Run { at, .. }) = t.o().drift else { panic!("runs") };
    let mut ms = 0.0;
    while ms <= DRIFT_MS + 16.0 {
        t.p.tick(at + ms);
        ms += 16.0;
    }
    let Some(Drift::Done { max, dz, .. }) = t.o().drift else { panic!("done") };
    assert!(max > 0.01 && max < 0.03, "{max}");
    assert_eq!(dz, 5.0);
    // the window closed: the check is gone
    t.click(sub(K_PANEL, "x"));
    assert!(t.o().drift.is_none());
}

/// Without the controller plugged in there is nothing to measure: the button is dimmed and does nothing.
#[test]
fn check_stick_drift_needs_the_controller() {
    let mut t = T::new();
    t.o().pads.clear();
    t.click_part(Pid::Stick(Side::Left));
    t.click(Open::k("ls.drift"));
    assert!(t.o().drift.is_none());
}

/// Order 042 look pictures (run on purpose: `BU_PIC_OUT=<folder> cargo test -p bu-app pad_order42_look -- --ignored`): the
/// tab painted off-screen with the app's painter (page at the window's page top + its popup layer), nothing on screen.
#[test]
#[ignore]
fn pad_order42_look() {
    let Ok(dir) = std::env::var("BU_PIC_OUT") else { return };
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    let icons = crate::icons::Icons::new();
    let shot = |t: &mut T, name: &str, scroll: f32| {
        t.now += 2000.0;
        if scroll > 0.0 {
            t.st.scroll_y.insert(K_PANEL, scroll);
        }
        let mut cx = Cx::new(t.now, false, &t.g, &mut t.st).for_page("pad");
        let kids = t.p.build(&mut cx);
        let pop = t.p.popup(&mut cx);
        drop(cx);
        let page = crate::ui::lay::Laid::new(&t.g, El::block().w(WIN_W).pad(2.0, 26.0, 18.0, 26.0).children(kids), WIN_W, None);
        let (w, h) = (WIN_W, crate::ui::WIN_H);
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
    // 1. Steam closed: the glass over the tab
    let mut t = T::new();
    t.steam_running(false);
    shot(&mut t, "1_steam_closed", 0.0);
    // 2. the left stick's window: open groups, no handles on the circle, the rows' undo / reset slot, drift
    let mut t = T::new();
    t.click_part(Pid::Stick(Side::Left));
    t.slide("ls.dz", 0.2);
    shot(&mut t, "2_stick_after_change", 0.0);
    shot(&mut t, "2b_drift_idle", 170.0);
    t.click(Open::k("ls.drift"));
    let Some(Drift::Run { at, .. }) = t.o().drift else { return };
    t.o().lv.ls = (0.02, 0.023);
    t.p.tick(at + 2100.0);
    t.now = at + 2100.0 - 2000.0;
    shot(&mut t, "3_drift_running", 170.0);
    t.p.tick(at + DRIFT_MS + 1.0);
    shot(&mut t, "4_drift_done", 170.0);
    shot(&mut t, "5_stick_scrolled", 520.0);
    // 6. Controller settings with a change (Undo + resets)
    let mut t = T::new();
    t.click(K_CTLSET);
    t.click(Open::k("pf.rumble"));
    shot(&mut t, "6_controller_settings", 0.0);
    // 7. "Acts as" list open over the window
    let mut t = T::new();
    t.click_part(Pid::Stick(Side::Left));
    t.click(Open::k("ls.mode"));
    shot(&mut t, "7_acts_as_list", 0.0);
}

/// Order 045 proof pictures (`BU_PIC_OUT=<folder> cargo test -p bu-app proof_045 -- --ignored --test-threads=1`): the
/// gyro's and the touchpad's panels and Controller settings without the four controls Steam has no slot for.
#[test]
#[ignore]
fn proof_045_controller_panels() {
    if std::env::var("BU_PIC_OUT").is_err() {
        return;
    }
    for (light, theme) in [(false, "dark"), (true, "light")] {
        crate::ui::set_light(light);
        for name in ["gyro", "touchpad", "settings"] {
            let mut t = T::new();
            match name {
                "gyro" => t.click(K_GYRO),
                "touchpad" => t.click_part(Pid::Touch),
                _ => t.click(K_CTLSET),
            }
            t.now += 2000.0;
            let mut cx = Cx::new(t.now, false, &t.g, &mut t.st).for_page("pad");
            let kids = t.p.build(&mut cx);
            let pop = t.p.popup(&mut cx);
            let page = El::block().abs(0.0, crate::ui::PAGE_TOP, f32::NAN, f32::NAN).w(WIN_W).pad(2.0, 26.0, 18.0, 26.0).children(kids);
            let root = El::block().w(WIN_W).h(crate::ui::WIN_H).child(page).children(pop);
            crate::ui::lay::proof_png(root, WIN_W, crate::ui::WIN_H, 1.5, &format!("13_controller_{name}_{theme}.png"));
        }
    }
    crate::ui::set_light(false);
}

// ------------------------------------------------------------------------------------------------ Order 047 (test 3)

/// the "Launch Steam" freeze (a big black box until Steam opened): the "Launch Steam" click and the moment Steam runs (its files read again - Steam writes them
/// while it starts) hand the menu's thread back within a frame, even with a slow Steam library; the files are still read.
#[test]
fn launch_steam_and_steam_starting_never_hold_the_menu() {
    let mut t = T::new();
    t.steam_running(false);
    assert!(!t.o().steam_up, "the glass");
    {
        let mut cx = Cx::new(t.now, false, &t.g, &mut t.st).for_page("pad");
        crate::offui::assert_quick("the Launch Steam click", || t.p.event(&Ev::Click(K_LAUNCH), &mut cx));
    }
    assert!(t.o().launch_at.is_some(), "Starting Steam\u{2026}");
    // Steam starts; its files answer slowly (40 ms per read)
    t.set_steam(true, 40);
    let now = t.now;
    let shown = crate::offui::assert_quick("Steam started: its files read again", || t.p.tick(now));
    assert!(shown && t.o().steam_up && t.o().launch_at.is_none(), "the glass goes at once");
    assert!(!t.o().idle(), "the files are read on the tab's worker");
    t.settle();
    t.build();
    assert_eq!(t.o().game().map(|g| g.name.clone()).as_deref(), Some("Rocket League"));
    assert!(t.o().view.is_some());
}

/// Order 047: a setting's click (one write + its backup + the files read back, on the worker) hands the menu's thread back
/// within a frame and the panel shows the new value at once; the file has it when the answer is in ("Saved"). Two quick
/// clicks are written in their order.
#[test]
fn a_setting_click_never_holds_the_menu_and_shows_at_once() {
    let mut t = T::new();
    t.click_part(Pid::B(ButtonId::Cross));
    t.slow(40);
    let k = Open::k("cross.turbo");
    let before = t.o().view.clone();
    {
        let mut cx = Cx::new(t.now, false, &t.g, &mut t.st).for_page("pad");
        crate::offui::assert_quick("a setting click", || t.p.event(&Ev::Click(k), &mut cx));
    }
    assert_ne!(t.o().view, before, "the panel shows it at once");
    assert!(!t.o().idle(), "written on the tab's worker");
    t.settle();
    assert!(t.rl().contains("\"hold_repeats\"\t\t\"1\""), "{}", t.rl());
    assert!(t.toast().starts_with("Saved"));
    t.build();
    // off, then on again at once: the file ends on
    for _ in 0..2 {
        {
            let mut cx = Cx::new(t.now, false, &t.g, &mut t.st).for_page("pad");
            crate::offui::assert_quick("a quick second click", || t.p.event(&Ev::Click(k), &mut cx));
        }
        t.build();
    }
    t.settle();
    t.slow(0);
    assert!(t.rl().contains("\"hold_repeats\"\t\t\"1\""), "written in the clicks' order: {}", t.rl());
}

/// Order 047: opening the tab hands the menu's thread back within a frame even with a slow Steam library and a slow
/// controller list (the worker makes the Steam service, lists the controllers, reads the files); until its answer the
/// frame keeps the old page a moment (`ready`), then the tab fills in. The next opening shows the kept state at once.
#[test]
fn opening_the_tab_never_holds_the_menu() {
    let env = Env { test: true, frozen: true, ..Env::default() };
    data::TEST_SLOW_MS.with(|c| c.set(40));
    let mut p = Controller::default();
    crate::offui::assert_quick("opening the Controller tab", || p.open(&env, 0.0));
    assert!(!p.ready(), "nothing read yet");
    let mut t = T { p, g: Gfx::new(1.0), st: State::default(), now: 1000.0 };
    t.settle();
    assert!(t.p.ready());
    assert_eq!(t.o().game().map(|g| g.name.clone()).as_deref(), Some("Rocket League"));
    assert!(t.o().view.is_some() && t.o().connected().is_some());
    // again (the menu closed and opened): the last state at once, read again behind it
    t.p.close();
    crate::offui::assert_quick("opening the Controller tab again", || t.p.open(&env, 0.0));
    assert!(t.p.ready() && t.o().view.is_some(), "the kept state, at once");
    assert!(!t.o().idle());
    data::TEST_SLOW_MS.with(|c| c.set(0));
    t.settle();
    assert_eq!(t.o().game().map(|g| g.name.clone()).as_deref(), Some("Rocket League"));
}

/// Order 047 (~1.7 cores with the menu just open): with nothing moving the tab asks for no frames - `tick` says
/// nothing changed and `wake_at` names no time, or one still to come (a toast's end), never one already past.
#[test]
fn nothing_moving_asks_for_no_frames() {
    let mut t = T::new();
    let now = t.now + 16.0;
    assert!(!t.p.tick(now), "nothing changed");
    assert!(t.p.wake_at(now).is_none_or(|w| w > now));
    // a change: its answer is shown once, then only the toast's end is waited for
    t.click_part(Pid::B(ButtonId::Cross));
    t.click(Open::k("cross.turbo"));
    let now = t.now + 16.0;
    assert!(!t.p.tick(now), "the answer was shown already");
    let w = t.p.wake_at(now);
    assert!(w.is_none_or(|w| w > now), "{w:?}");
    if let Some(w) = w {
        t.p.tick(w);
        assert!(t.p.wake_at(w).is_none_or(|x| x > w));
    }
}

/// Order 047: the open tab's Reset review is read and put back on a worker thread (`detach`; the tab's Steam service
/// answering slowly here): opening the review and Reset hand the menu's thread back within a frame; its line and the
/// outcome are as before, and the tab reads its files again when the reset has ended (`reset_done`).
#[test]
fn the_reset_review_reads_and_resets_off_the_menus_thread() {
    use crate::undo::{Applied, Kind, Opened, Review};
    let _s = Sv::start();
    let mut t = T::live();
    let original = t.rl();
    t.click_part(Pid::B(ButtonId::Cross));
    t.click(Open::k("cross.turbo"));
    assert_ne!(t.rl(), original);
    t.slow(30);
    let review = {
        let mut pages: [&mut dyn Resettable; 1] = [&mut t.p];
        let opened = crate::offui::assert_quick("opening the review", || crate::services::with(|s| Review::open(Kind::HowItWas, false, &mut pages, &s.store)).unwrap());
        let Opened::Reading(mut job) = opened else { panic!("the tab's lines are read on a worker thread") };
        let t0 = std::time::Instant::now();
        loop {
            if let Some(r) = job.take() {
                break r;
            }
            assert!(t0.elapsed().as_secs() < 20, "the review was never read");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    };
    assert_eq!(review.lines.len(), 1);
    assert_eq!(review.lines[0].from.text, "your edits");
    let res = {
        let mut pages: [&mut dyn Resettable; 1] = [&mut t.p];
        let applied = crate::offui::assert_quick("Reset", || review.start_apply(&mut pages));
        let Applied::Running(mut job) = applied else { panic!("the tab's line is put back on a worker thread") };
        let t0 = std::time::Instant::now();
        loop {
            if let Some(r) = job.take() {
                break r;
            }
            assert!(t0.elapsed().as_secs() < 20, "the reset never ended");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    };
    assert!(res.iter().all(|r| r.outcome == crate::undo::Outcome::Ok), "{res:?}");
    t.p.reset_done();
    t.slow(0);
    t.settle();
    assert_eq!(t.rl(), original, "the bytes from before");
}

/// Order 047 review: Ctrl+Z pressed while the newest change is still being written undoes THAT change (it waits for its
/// answer), not the one before it.
#[test]
fn ctrl_z_during_a_write_undoes_the_newest_change() {
    let mut t = T::new();
    t.click_part(Pid::B(ButtonId::Cross));
    t.click(Open::k("cross.turbo")); // change A, written
    assert!(t.rl().contains("\"hold_repeats\"\t\t\"1\""));
    t.click_part(Pid::Stick(Side::Left));
    let orig = t.left_stick();
    t.slow(30);
    // change B and Ctrl+Z at once (no answer in between)
    let k = Open::k("ls.dz");
    let mut cx = Cx::new(t.now, false, &t.g, &mut t.st).for_page("pad");
    t.p.event(&Ev::Press(k, 108.0, 10.0, (0.0, 0.0, 216.0, 20.0)), &mut cx);
    t.p.event(&Ev::Release(k), &mut cx);
    cx.mods = crate::ui::cx::Mods { ctrl: true, shift: false, alt: false };
    t.p.event(&Ev::Key(crate::ui::cx::PAGE, 0x5A), &mut cx);
    drop(cx);
    t.settle();
    t.slow(0);
    assert_eq!(t.left_stick(), orig, "B undone");
    assert!(t.rl().contains("\"hold_repeats\"\t\t\"1\""), "A kept");
    assert_eq!(t.toast(), "Undone \u{b7} Left stick \u{b7} Dead zone");
}
