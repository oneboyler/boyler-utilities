use std::collections::BTreeMap;

use super::*;
use crate::settings::scratch::Scratch;

/// A fake crate: items with values; some refuse to change.
struct FakePage {
    id: &'static str,
    title: &'static str,
    values: BTreeMap<String, String>,
    defaults: Vec<(&'static str, &'static str, &'static str)>,
    refuse: Vec<&'static str>,
    has_defaults: bool,
    defaults_name: Option<&'static str>,
    applied: Vec<(String, String)>,
}

impl FakePage {
    fn new(id: &'static str, title: &'static str) -> Self {
        FakePage {
            id,
            title,
            values: BTreeMap::new(),
            defaults: Vec::new(),
            refuse: Vec::new(),
            has_defaults: true,
            defaults_name: None,
            applied: Vec::new(),
        }
    }
    fn with(mut self, item: &str, v: &str) -> Self {
        self.values.insert(item.into(), v.into());
        self
    }
    /// A change made "through the app": sets the value and records it.
    fn change(&mut self, store: &mut SettingsStore, item: &str, label: &str, to: &str) {
        let old = self.values.get(item).cloned().unwrap_or_default();
        self.values.insert(item.into(), to.into());
        record(store, self.id, item, label, &Val::plain(&old), &Val::plain(to)).unwrap();
    }
}

impl Resettable for FakePage {
    fn page_id(&self) -> &str {
        self.id
    }
    fn page_title(&self) -> &str {
        self.title
    }
    fn current(&self, item: &str) -> Option<Val> {
        self.values.get(item).map(|v| Val::plain(v))
    }
    fn has_windows_defaults(&self) -> bool {
        self.has_defaults
    }
    fn defaults_name(&self) -> Option<&str> {
        self.defaults_name
    }
    fn windows_defaults(&self) -> Vec<DefaultItem> {
        self.defaults
            .iter()
            .map(|(item, label, d)| DefaultItem {
                item: item.to_string(),
                label: label.to_string(),
                now: Val::plain(self.values.get(*item).map(String::as_str).unwrap_or("")),
                default: Val::plain(d),
            })
            .collect()
    }
    fn apply(&mut self, item: &str, to: &Val) -> Result<(), String> {
        if self.refuse.contains(&item) {
            return Err("Windows said no (needs admin)".into());
        }
        self.values.insert(item.into(), to.raw.clone());
        self.applied.push((item.into(), to.raw.clone()));
        Ok(())
    }
}

#[test]
fn first_old_value_is_how_the_pc_was_and_survives_restart_and_app_reset() {
    let s = Scratch::new("undo-first");
    let mut st = SettingsStore::open(s.dir());
    let mut mouse = FakePage::new("mouse", "Mouse").with("speed", "10");
    mouse.change(&mut st, "speed", "Pointer speed", "12");
    mouse.change(&mut st, "speed", "Pointer speed", "8");
    st.set_i64(Scope::Page("mouse"), "something", 1).unwrap();
    st.reset_app_settings().unwrap();
    let st = SettingsStore::open(s.dir());
    let r = read_record(&st, "mouse", "speed").unwrap();
    assert_eq!(r.was, Val::plain("10"));
    assert_eq!(r.now, Val::plain("8"));
    assert_eq!(r.label, "Pointer speed");
    assert!(r.first <= r.last && r.first > 0);
}

#[test]
fn how_it_was_review_lists_changed_items_all_ticked() {
    let s = Scratch::new("undo-review");
    let mut st = SettingsStore::open(s.dir());
    let mut mouse = FakePage::new("mouse", "Mouse").with("speed", "10").with("epp", "On").with("dbl", "500");
    mouse.change(&mut st, "speed", "Pointer speed", "8");
    mouse.change(&mut st, "epp", "Enhance pointer precision", "Off");
    mouse.change(&mut st, "dbl", "Double-click speed", "400");
    mouse.change(&mut st, "dbl", "Double-click speed", "500"); // back where it was: not listed
    let r = Review::for_page(Kind::HowItWas, &mouse, &st);
    assert_eq!(r.title(), "Mouse · back to how it was?");
    assert_eq!(r.subline(), "Each one goes back to the value it had before this app changed it.");
    assert_eq!(r.lines.len(), 2);
    assert!(r.lines.iter().all(|l| l.ticked));
    let speed = r.lines.iter().find(|l| l.item == "speed").unwrap();
    assert_eq!((speed.label.as_str(), speed.change_text()), ("Pointer speed", "8  →  10".to_string()));
    assert_eq!(r.button_text(), "Reset 2");
}

#[test]
fn applying_ticked_lines_calls_the_page_and_records() {
    let s = Scratch::new("undo-apply");
    let mut st = SettingsStore::open(s.dir());
    let mut mouse = FakePage::new("mouse", "Mouse").with("speed", "10").with("epp", "On").with("size", "1");
    mouse.change(&mut st, "speed", "Pointer speed", "8");
    mouse.change(&mut st, "epp", "Enhance pointer precision", "Off");
    mouse.change(&mut st, "size", "Cursor size", "2");
    mouse.refuse.push("size");
    let mut r = Review::for_page(Kind::HowItWas, &mouse, &st);
    let epp = r.lines.iter().position(|l| l.item == "epp").unwrap();
    r.toggle(epp); // keep this one
    assert_eq!(r.button_text(), "Reset 2");
    let results = r.apply(&mut st, &mut [&mut mouse]);
    assert_eq!(results.len(), 2);
    let get = |item: &str| results.iter().find(|x| x.item == item).unwrap().outcome.clone();
    assert_eq!(get("speed"), Outcome::Ok);
    assert_eq!(get("size"), Outcome::Failed("Windows said no (needs admin)".into()));
    assert_eq!(mouse.applied, vec![("speed".to_string(), "10".to_string())]);
    // the page's list now only has what is still changed
    let again = Review::for_page(Kind::HowItWas, &mouse, &st);
    let mut items: Vec<&str> = again.lines.iter().map(|l| l.item.as_str()).collect();
    items.sort();
    assert_eq!(items, vec!["epp", "size"]);
    // "how it was" is still the first old value
    assert_eq!(read_record(&st, "mouse", "speed").unwrap().was, Val::plain("10"));
}

#[test]
fn current_value_from_the_page_wins_over_the_log() {
    let s = Scratch::new("undo-current");
    let mut st = SettingsStore::open(s.dir());
    let mut mouse = FakePage::new("mouse", "Mouse").with("speed", "10");
    mouse.change(&mut st, "speed", "Pointer speed", "8");
    // changed back in Windows' own settings: nothing to reset
    mouse.values.insert("speed".into(), "10".into());
    assert!(Review::for_page(Kind::HowItWas, &mouse, &st).is_empty());
    // changed elsewhere to 14: the line shows 14 → 10
    mouse.values.insert("speed".into(), "14".into());
    assert_eq!(Review::for_page(Kind::HowItWas, &mouse, &st).lines[0].change_text(), "14  →  10");
}

#[test]
fn windows_defaults_come_from_the_page() {
    let s = Scratch::new("undo-defaults");
    let mut st = SettingsStore::open(s.dir());
    let mut tweaks = FakePage::new("tweaks", "Tweaks").with("accel", "Off").with("widgets", "Off").with("gamemode", "On");
    tweaks.defaults = vec![("accel", "Mouse acceleration", "On"), ("widgets", "Widgets", "On"), ("gamemode", "Game Mode", "On")];
    let r = Review::for_page(Kind::WindowsDefaults, &tweaks, &st);
    assert_eq!(r.title(), "Tweaks · Windows defaults?");
    assert_eq!(r.subline(), "Each one goes to Windows’ own value.");
    assert_eq!(r.lines.iter().map(|l| l.item.as_str()).collect::<Vec<_>>(), vec!["accel", "widgets"]);
    let res = r.apply(&mut st, &mut [&mut tweaks]);
    assert!(res.iter().all(|x| x.outcome == Outcome::Ok));
    assert!(Review::for_page(Kind::WindowsDefaults, &tweaks, &st).is_empty());
    // the reset itself was recorded with the old value: "how it was" can bring Off back
    let back = Review::for_page(Kind::HowItWas, &tweaks, &st);
    assert_eq!(back.lines.len(), 2);
    assert!(back.lines.iter().all(|l| l.to == Val::plain("Off")));
}

#[test]
fn startup_has_no_windows_defaults_and_controller_names_its_own() {
    let s = Scratch::new("undo-names");
    let st = SettingsStore::open(s.dir());
    let mut startup = FakePage::new("startup", "Startup").with("obs", "Off");
    startup.has_defaults = false;
    startup.defaults = vec![("obs", "OBS Studio", "On")];
    assert!(Review::for_page(Kind::WindowsDefaults, &startup, &st).is_empty());
    let mut pad = FakePage::new("controller", "Controller").with("rl", "yours");
    pad.defaults_name = Some("Steam’s layout");
    pad.defaults = vec![("rl", "Rocket League · DualSense Edge", "steam")];
    let r = Review::for_page(Kind::WindowsDefaults, &pad, &st);
    assert_eq!(r.title(), "Controller · back to Steam’s layout?");
    assert_eq!(r.subline(), "The layout goes back to the one Steam made for this game.");
}

#[test]
fn settings_review_covers_every_page_grouped() {
    let s = Scratch::new("undo-all");
    let mut st = SettingsStore::open(s.dir());
    let mut audio = FakePage::new("audio", "Audio").with("out", "Speakers").with("keep", "Off");
    let mut mouse = FakePage::new("mouse", "Mouse").with("speed", "10");
    let net = FakePage::new("network", "Network");
    audio.change(&mut st, "out", "Default output", "Headphones");
    audio.change(&mut st, "keep", "Keep my devices", "On");
    mouse.change(&mut st, "speed", "Pointer speed", "8");
    let r = Review::for_all(Kind::HowItWas, &[&audio, &mouse, &net], &st);
    assert_eq!(r.title(), "Back to how your PC was?");
    assert_eq!(r.groups(), vec![("Audio".to_string(), vec![0, 1]), ("Mouse".to_string(), vec![2])]);
    assert_eq!(r.button_text(), "Reset 3");
    // a page not handed to apply fails its lines with a reason, the others go through
    let res = r.apply(&mut st, &mut [&mut audio]);
    assert_eq!(res.iter().filter(|x| x.outcome == Outcome::Ok).count(), 2);
    assert_eq!(res.iter().find(|x| x.page == "mouse").unwrap().outcome, Outcome::Failed("The page isn't loaded".into()));
    assert_eq!(Review::for_all(Kind::WindowsDefaults, &[&audio], &st).title(), "Windows defaults for everything?");
}

#[test]
fn nothing_ticked_means_nothing_applied() {
    let s = Scratch::new("undo-none");
    let mut st = SettingsStore::open(s.dir());
    let mut mouse = FakePage::new("mouse", "Mouse").with("speed", "10");
    mouse.change(&mut st, "speed", "Pointer speed", "8");
    let mut r = Review::for_page(Kind::HowItWas, &mouse, &st);
    r.toggle(0);
    assert_eq!(r.button_text(), "Reset");
    assert!(r.apply(&mut st, &mut [&mut mouse]).is_empty());
    assert!(mouse.applied.is_empty());
}

/// Order 039: a reset's admin lines from several pages = ONE admin prompt (one elevated copy for the batch); a "No"
/// fails every admin line with "Needs admin - not changed" and asks no second time.
#[test]
fn a_reset_asks_for_admin_once_for_all_its_admin_lines() {
    use crate::admin::{Op, Purpose};
    /// A page whose items are admin settings: putting one back is one op to the elevated copy.
    struct AdminPage(&'static str, Vec<Op>);
    impl Resettable for AdminPage {
        fn page_id(&self) -> &str {
            self.0
        }
        fn page_title(&self) -> &str {
            self.0
        }
        fn apply(&mut self, item: &str, _to: &Val) -> Result<(), String> {
            let op = self.1[item.parse::<usize>().unwrap()].clone();
            let p = if self.0 == "tgl" { Purpose::Tweaks } else if self.0 == "net" { Purpose::Network } else { Purpose::Startup };
            crate::admin::client::admin().call(p, op).map(|_| ()).map_err(|e| e.to_string())
        }
    }
    let line = |page: &str, item: &str| Line { page: page.into(), page_title: page.into(), item: item.into(), label: item.into(), from: Val::plain("x"), to: Val::plain("y"), ticked: true };
    let review = Review { kind: Kind::HowItWas, all: true, page_title: String::new(), defaults_name: None, lines: vec![line("tgl", "0"), line("tgl", "1"), line("net", "0"), line("sup", "0")] };
    for decline in [false, true] {
        let w = crate::admin::tests::world();
        let (hub, n) = crate::admin::tests::hub(&w, decline);
        crate::admin::client::set_for_test(hub);
        let mut tgl = AdminPage("tgl", vec![
            Op::RegSet { hive: crate::admin::Hive::Hklm, path: bu_toggles::rows::HAGS_PATH.into(), name: "HwSchMode".into(), dword: 1 },
            Op::UsbSuspend { ac: 1, dc: 1 },
        ]);
        let mut net = AdminPage("net", vec![Op::NetAdapter { id: "{4D36E972-E325-11CE-BFC1-08002BE10318}".into(), on: true }]);
        let mut sup = AdminPage("sup", vec![Op::Service { name: "VendorSvc".into(), start: crate::admin::Start::Automatic, delayed: false }]);
        let res = review.apply_each(&mut [&mut tgl, &mut net, &mut sup], &mut |_| Ok(()));
        assert_eq!(n.load(std::sync::atomic::Ordering::SeqCst), 1, "one prompt for the whole reset");
        if decline {
            assert!(res.iter().all(|r| r.outcome == Outcome::Failed(crate::admin::NOT_CHANGED.into())), "{res:?}");
        } else {
            assert!(res.iter().all(|r| r.outcome == Outcome::Ok), "{res:?}");
        }
    }
}
