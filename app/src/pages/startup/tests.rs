//! Startup against the FAKE bu-startup (nothing on the PC changes).

use super::*;
use crate::gfx::Gfx;
use crate::ui::cx::State;

fn page() -> Startup {
    let mut p = Startup::default();
    p.open(&Env { test: true, ..Env::default() }, 0.0);
    p
}

fn click(p: &mut Startup, k: Key) {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st);
    p.event(&Ev::Click(k), &mut cx);
}

fn build(p: &mut Startup) -> Vec<El> {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st);
    let k = p.build(&mut cx);
    let _ = p.popup(&mut cx);
    k
}

fn ix(p: &Startup, name: &str) -> usize {
    p.list.as_ref().unwrap().entries.iter().position(|e| e.name == name).unwrap_or_else(|| panic!("no {name}"))
}

#[test]
fn opens_with_the_drawings_list() {
    let p = page();
    let names: Vec<&str> = p.list.as_ref().unwrap().entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names.len(), 15);
    // the owner's order (Oct 8): on and not Windows' own (the crate's order: normal rows, then tasks / services), then
    // Windows' own that are on, then the ones that are off
    assert_eq!(
        names,
        [
            "Discord", "Steam", "OneDrive", "Spotify", "Wootility", "NVIDIA App", "Adobe Acrobat Update Task", "GoogleUpdateTaskMachineUA",
            "MicrosoftEdgeUpdateTaskMachineCore", "EpicOnlineServices", "NVIDIA LocalSystem Container", "Windows Security notification icon",
            "Microsoft Defender Antivirus Service", "Windows Audio", "OBS Studio"
        ]
    );
    let l = p.list.as_ref().unwrap();
    assert!(!l.entries[ix(&p, "OBS Studio")].enabled);
    let own: Vec<&str> = l.entries.iter().filter(|e| e.windows_own).map(|e| e.name.as_str()).collect();
    assert_eq!(own, ["Windows Security notification icon", "Microsoft Defender Antivirus Service", "Windows Audio"]);
    assert!(matches!(l.entries[ix(&p, "Discord")].impact, ImpactState::Measured(Impact::High, _)));
    assert!(matches!(l.entries[ix(&p, "Spotify")].impact, ImpactState::Measured(Impact::Medium, _)));
    assert!(matches!(l.entries[ix(&p, "Wootility")].impact, ImpactState::Measured(Impact::Low, _)));
    assert_eq!(l.counts(View::All), (14, 15));
}

#[test]
fn a_normal_row_switches_off_and_back() {
    let mut p = page();
    let i = ix(&p, "Steam");
    click(&mut p, idx(K_TG, i));
    assert!(!p.list.as_ref().unwrap().entries[i].enabled);
    assert_eq!(p.toast.as_ref().unwrap().0, "Steam won’t start with Windows · switch it back any time");
    click(&mut p, idx(K_TG, i));
    assert!(p.list.as_ref().unwrap().entries[i].enabled);
    assert_eq!(p.toast.as_ref().unwrap().0, "Steam starts with Windows again");
}

#[test]
fn a_windows_entry_asks_first() {
    let mut p = page();
    let i = ix(&p, "Windows Security notification icon");
    click(&mut p, idx(K_TG, i));
    assert!(matches!(p.pop, Some(Pop::Ask(..))));
    assert!(p.list.as_ref().unwrap().entries[i].enabled, "nothing before the answer");
    build(&mut p);
    click(&mut p, sub(K_ASK, "no"));
    assert!(p.pop.is_none() && p.list.as_ref().unwrap().entries[i].enabled, "Keep on");
    click(&mut p, idx(K_TG, i));
    build(&mut p);
    click(&mut p, sub(K_ASK, "go"));
    // it lives in the machine-wide Run key: Turn off needs admin (not elevated here) - nothing changes, the toast says so
    assert!(p.list.as_ref().unwrap().entries[i].enabled);
    assert_eq!(p.toast.as_ref().unwrap().0, crate::admin::NOT_CHANGED);
    // elevated: it goes off
    p.svc.as_ref().unwrap().fake().unwrap().state().admin = true;
    click(&mut p, idx(K_TG, i));
    build(&mut p);
    click(&mut p, sub(K_ASK, "go"));
    assert!(!p.list.as_ref().unwrap().entries[i].enabled, "Turn off");
}

#[test]
fn windows_own_services_are_locked() {
    let mut p = page();
    let i = ix(&p, "Microsoft Defender Antivirus Service");
    click(&mut p, idx(K_TG, i));
    assert!(p.list.as_ref().unwrap().entries[i].enabled);
    assert_eq!(p.toast.as_ref().unwrap().0, "Locked · Microsoft Defender Antivirus Service is part of Windows");
}

#[test]
fn hidden_rows_need_admin_and_say_so() {
    let mut p = page();
    let i = ix(&p, "EpicOnlineServices");
    click(&mut p, idx(K_TG, i));
    assert!(p.list.as_ref().unwrap().entries[i].enabled);
    assert_eq!(p.toast.as_ref().unwrap().0, crate::admin::NOT_CHANGED);
    assert!(p.svc.as_ref().unwrap().fake().unwrap().state().writes.is_empty());
}

#[test]
fn filter_and_counts() {
    let mut p = page();
    click(&mut p, idx(K_SEG, 2));
    assert_eq!(p.entries().len(), 7);
    assert_eq!(p.appear.len(), 0, "Hidden rows were already shown under All");
    click(&mut p, idx(K_SEG, 1));
    assert_eq!(p.entries().len(), 8);
    assert_eq!(p.appear.len(), 8, "every Normal row slides in");
    assert!(p.describe().contains("shown=8"));
}

#[test]
fn right_click_menu_never_opens_anything_in_a_test_copy() {
    let mut p = page();
    let i = ix(&p, "Steam");
    p.context(i, 200.0, 200.0);
    build(&mut p);
    click(&mut p, idx(K_MENU, 1));
    assert_eq!(p.opened, vec![r"select:C:\Program Files (x86)\Steam\steam.exe".to_string()]);
    p.context(i, 200.0, 200.0);
    click(&mut p, idx(K_MENU, 2));
    assert_eq!(p.opened[1], "https://www.bing.com/search?q=Steam");
}

#[test]
fn closing_drops_the_list() {
    let mut p = page();
    p.close();
    assert!(p.svc.is_none() && p.list.is_none() && p.rs.borrow().is_none());
}

#[test]
fn rows_go_on_then_windows_on_then_off_and_stay_put_while_switched() {
    let mut p = page();
    let ranks: Vec<u8> = p.list.as_ref().unwrap().entries.iter().map(rank).collect();
    assert!(ranks.windows(2).all(|w| w[0] <= w[1]), "{ranks:?}");
    assert!(ranks.contains(&0) && ranks.contains(&1) && ranks.contains(&2), "{ranks:?}");
    let before: Vec<String> = p.list.as_ref().unwrap().entries.iter().map(|e| e.id.clone()).collect();
    // switching Steam off keeps every row where it was
    let i = ix(&p, "Steam");
    click(&mut p, idx(K_TG, i));
    let after: Vec<String> = p.list.as_ref().unwrap().entries.iter().map(|e| e.id.clone()).collect();
    assert_eq!(before, after);
    assert!(!p.list.as_ref().unwrap().entries[i].enabled);
}

#[test]
fn a_right_click_on_a_row_or_its_switch_opens_the_menu() {
    let mut p = page();
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st);
    p.event(&Ev::Context(idx(K_ROW, 1), 200.0, 210.0), &mut cx);
    assert!(matches!(p.pop, Some(Pop::Menu(1, x, y)) if x == 200.0 && y == 210.0));
    p.pop = None;
    p.event(&Ev::Context(idx(K_TG, 2), 10.0, 20.0), &mut cx);
    assert!(matches!(p.pop, Some(Pop::Menu(2, ..))));
}

// ---------------------------------------------------------------- the ONE change log (Order 036), fakes only

use crate::undo::{read_record, Kind as RKind, Outcome, Resettable, Review};

fn start() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
}

/// A click with the page's Cx (`for_page("sup")`: its records go under this page).
fn click_sup(p: &mut Startup, k: Key) {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st).for_page("sup");
    p.event(&Ev::Click(k), &mut cx);
}

fn review(p: &Startup) -> Review {
    crate::services::with(|s| Review::for_page(RKind::HowItWas, p, &s.store)).unwrap()
}

#[test]
fn a_switch_goes_into_the_change_log_with_its_old_value_and_back() {
    start();
    let mut p = page();
    let i = ix(&p, "Steam");
    // off, on, off: one entry, the value before the FIRST change
    click_sup(&mut p, idx(K_TG, i));
    click_sup(&mut p, idx(K_TG, i));
    click_sup(&mut p, idx(K_TG, i));
    assert!(!p.list.as_ref().unwrap().entries[i].enabled);
    let r = crate::services::with(|s| read_record(&s.store, "sup", "flag|HKCU|Run|Steam")).flatten().expect("one entry");
    assert_eq!((r.was.raw.as_str(), r.now.raw.as_str(), r.label.as_str()), ("on", "off", "Steam"));
    let mut rv = review(&p);
    assert_eq!(rv.title(), "Startup · back to how it was?");
    assert_eq!(rv.lines.len(), 1);
    assert_eq!(rv.lines[0].change_text(), "Off  →  Starts with Windows");
    // untick = kept
    rv.toggle(0);
    assert!(rv.apply_each(&mut [&mut p as &mut dyn Resettable], &mut |_| Ok(())).is_empty());
    assert!(!p.list.as_ref().unwrap().entries[i].enabled, "an unticked line stays as it is");
    // ticked = back, and the open page shows it
    rv.toggle(0);
    let res = crate::services::with(|s| rv.apply(&mut s.store, &mut [&mut p as &mut dyn Resettable])).unwrap();
    assert_eq!(res[0].outcome, Outcome::Ok);
    let i = ix(&p, "Steam");
    assert!(p.list.as_ref().unwrap().entries[i].enabled, "back to how the PC was");
    assert!(review(&p).is_empty(), "nothing left to reset");
    // Windows has no default startup list
    assert!(!p.has_windows_defaults());
    assert!(crate::services::with(|s| Review::for_page(RKind::WindowsDefaults, &p, &s.store)).unwrap().is_empty());
    crate::services::shutdown();
}

#[test]
fn a_hidden_task_says_on_off_and_goes_back() {
    start();
    let mut p = page();
    p.svc.as_ref().unwrap().fake().unwrap().state().admin = true;
    let i = ix(&p, "Adobe Acrobat Update Task");
    click_sup(&mut p, idx(K_TG, i));
    assert!(!p.list.as_ref().unwrap().entries[i].enabled);
    let rv = review(&p);
    assert_eq!((rv.lines[0].item.as_str(), rv.lines[0].label.as_str()), (r"task|\Adobe Acrobat Update Task", "Adobe Acrobat Update Task"));
    assert_eq!(rv.lines[0].change_text(), "Off  →  On");
    let res = crate::services::with(|s| rv.apply(&mut s.store, &mut [&mut p as &mut dyn Resettable])).unwrap();
    assert_eq!(res[0].outcome, Outcome::Ok);
    assert!(p.list.as_ref().unwrap().entries[ix(&p, "Adobe Acrobat Update Task")].enabled);
    crate::services::shutdown();
}

#[test]
fn a_change_that_failed_writes_nothing() {
    start();
    let mut p = page();
    // a hidden service without admin: refused, nothing changed, no entry
    let i = ix(&p, "EpicOnlineServices");
    click_sup(&mut p, idx(K_TG, i));
    // a locked one
    let i = ix(&p, "Microsoft Defender Antivirus Service");
    click_sup(&mut p, idx(K_TG, i));
    assert!(crate::services::with(|s| crate::undo::records(&s.store, Some("sup"))).unwrap().is_empty());
    crate::services::shutdown();
}

#[test]
fn a_closed_page_reads_and_puts_back_on_its_own_service() {
    start();
    // the drawing's sample has OBS Studio off: as if the app had switched it off (before: on)
    crate::services::with(|s| {
        crate::undo::record(&mut s.store, "sup", "flag|HKCU|Run|OBS Studio", "OBS Studio", &Val::new("on", "Starts with Windows"), &Val::new("off", "Off"))
    })
    .unwrap()
    .unwrap();
    // and a service it can't put back without admin
    crate::services::with(|s| {
        crate::undo::record(&mut s.store, "sup", "service|EpicOnlineServices", "EpicOnlineServices", &Val::new("manual", "Off"), &Val::new("auto", "On"))
    })
    .unwrap()
    .unwrap();
    let mut closed = Startup::default();
    assert!(closed.rs.borrow().is_none(), "resettable() builds nothing");
    let r = closed.resettable().unwrap();
    let rv = crate::services::with(|s| Review::for_page(RKind::HowItWas, &*r, &s.store)).unwrap();
    assert_eq!(rv.lines.len(), 2);
    let res = rv.apply_each(&mut [r], &mut |_| Ok(()));
    let get = |item: &str| res.iter().find(|x| x.item == item).unwrap().outcome.clone();
    assert_eq!(get("flag|HKCU|Run|OBS Studio"), Outcome::Ok);
    assert_eq!(get("service|EpicOnlineServices"), Outcome::Failed(crate::admin::NOT_CHANGED.into()));
    assert_eq!(closed.current("flag|HKCU|Run|OBS Studio"), Some(Val::new("on", "Starts with Windows")), "put back on the fake");
    crate::services::shutdown();
}

#[test]
fn a_read_only_test_copy_refuses_the_reset() {
    let mut p = page();
    p.real_read = true;
    assert_eq!(Resettable::apply(&mut p, "flag|HKCU|Run|Steam", &Val::plain("off")), Err("A read-only test copy changes nothing".to_string()));
    assert!(p.svc.as_ref().unwrap().fake().unwrap().state().writes.is_empty());
}

/// Order 042 proof picture (`BU_PIC_OUT=<folder> cargo test -p bu-app proof_042 -- --ignored`): the rows with the apps' own icons, read the way
/// the real page reads them (`entry_icon`: the shell's icon of the entry's file, read only) - an entry whose file isn't on
/// this PC keeps the drawing's tile. The second picture has every row switched off (greyed icons).
#[test]
#[ignore]
fn proof_042_startup_real_icons() {
    if std::env::var("BU_PIC_OUT").is_err() {
        return;
    }
    let mut p = page();
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
    }
    let entries = p.list.as_ref().unwrap().entries.clone();
    p.icons = entries.iter().filter_map(|e| entry_icon(e).map(|px| (e.id.clone(), std::sync::Arc::new(px)))).collect();
    assert!(!p.icons.is_empty(), "no entry file of the sample list exists on this PC");
    for (name, off) in [("startup_icons", false), ("startup_icons_off", true)] {
        if off {
            for e in p.list.as_mut().unwrap().entries.iter_mut() {
                e.enabled = false;
            }
        }
        let kids = build(&mut p);
        let root = El::block().w(600.0).h(560.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
        crate::ui::lay::proof_png(root, 600.0, 560.0, 2.0, &format!("{name}.png"));
    }
}
