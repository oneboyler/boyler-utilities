//! The Apps tab (menu-v22 page `apps`, Order 023): one clean list of every installed app (desktop + Store) - tick box,
//! icon, name (+ "Store" tag / lock), publisher, size, install date and the always-visible actions
//! [Modify / Repair] · Open install folder · Uninstall - sortable columns, the header search, the floating selection bar,
//! the right-click menu, one confirm before anything goes, and rows that say "Uninstalling…" while it runs.
//! Wired to crates/apps (`bu-apps`): the FAKE OS layer in every test copy (made from the drawing's sample apps), the real
//! one otherwise. Nothing is uninstalled or modified except from the user's own click on the confirm / the wrench.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{mpsc, Arc};

use bu_apps::fake::{d, s, FakeOs};
use bu_apps::os::{RawEntry, RawPackage, Signature};
use bu_apps::{AppKind, AppList, AppsError, AppsOs, Fix, FixOutcome, Hive, InstalledApp, OsError, Outcome, Progress, RegView, SortKey};
use taffy::prelude::{fr, length, minmax};
use taffy::style::{AlignItems, JustifyContent};

use crate::anim::{Bezier, EASE, EASE_OUT};
use crate::gfx::{sh, Align, Font, Rgba};
use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev, PAGE};
use crate::ui::el::{idx, key, lh, sub, Cursor, El, Key, RADIUS_PILL};
use crate::ui::pieces::button::{cbtn_sized, Kind, DFT};
use crate::ui::pieces::listrow::{tile, Tile};
use crate::ui::pieces::mitems::{self, It, Place, Row};
use crate::ui::pieces::selbar::{self, Sbb};
use crate::ui::pieces::udlg::{self, UdRow};
use crate::ui::pieces::{self, bits, group, search, toast};
use crate::ui::{cmix, ACC, CTL, CTL_H, DASH, FG, FG2, FG3, HAIR, HOV, RED, SEL, WHITE};

const K_SEARCH: Key = key("apps.search");
const K_SORT: Key = key("apps.sort");
const K_ALL: Key = key("apps.all");
const K_ROW: Key = key("apps.row");
const K_MENU: Key = key("apps.menu");
const K_FIXM: Key = key("apps.fixmenu");
const K_DLG: Key = key("apps.dlg");
const K_BAR: Key = key("apps.bar");
const K_TOAST: Key = key("apps.toast");

/// `.prh.aphd,.prr.aprow{grid-template-columns:minmax(0,1fr) 62px 84px 84px;column-gap:8px;padding:0 8px 0 30px}`
const COLS: [(SortKey, &str); 3] = [(SortKey::Name, "Name"), (SortKey::Size, "Size"), (SortKey::Installed, "Installed")];

// ------------------------------------------------------------------------------------------------- the OS layer

/// The real or the fake layer behind one type (the page's service is shared with its worker threads).
pub enum Os {
    #[cfg(windows)]
    Real(bu_apps::real::RealOs),
    Fake(Box<FakeOs>),
}

macro_rules! each {
    ($s:expr, $o:ident => $e:expr) => {
        match $s {
            #[cfg(windows)]
            Os::Real($o) => $e,
            Os::Fake($o) => $e,
        }
    };
}

impl AppsOs for Os {
    fn uninstall_entries(&self) -> Result<Vec<RawEntry>, OsError> {
        each!(self, o => o.uninstall_entries())
    }
    fn store_packages(&self) -> Result<Vec<RawPackage>, OsError> {
        each!(self, o => o.store_packages())
    }
    fn entry_exists(&self, h: Hive, v: RegView, k: &str) -> bool {
        each!(self, o => o.entry_exists(h, v, k))
    }
    fn package_installed(&self, f: &str) -> bool {
        each!(self, o => o.package_installed(f))
    }
    fn run_uninstaller(&self, c: &str) -> Result<u32, OsError> {
        each!(self, o => o.run_uninstaller(c))
    }
    fn remove_package(&self, f: &str) -> Result<(), OsError> {
        each!(self, o => o.remove_package(f))
    }
    fn folder_size(&self, p: &Path) -> Option<u64> {
        each!(self, o => o.folder_size(p))
    }
    fn expand_env(&self, x: &str) -> String {
        each!(self, o => o.expand_env(x))
    }
    fn run_setup(&self, c: &str) -> Result<u32, OsError> {
        each!(self, o => o.run_setup(c))
    }
    fn is_dir(&self, p: &Path) -> bool {
        each!(self, o => o.is_dir(p))
    }
    fn open_folder(&self, p: &Path) -> Result<(), OsError> {
        each!(self, o => o.open_folder(p))
    }
    fn open_settings(&self, u: &str) -> Result<(), OsError> {
        each!(self, o => o.open_settings(u))
    }
}

pub type Svc = Arc<bu_apps::Apps<Os>>;

// ------------------------------------------------------------------------------------------------- the drawing's sample apps

/// One app of the drawing's INST list (menu-v22 `INST`): name, publisher, tile glyph + gradient, GB, install date, version,
/// the wrench's choices, locked, Store.
struct Sample {
    n: &'static str,
    p: &'static str,
    g: &'static str,
    a: u32,
    b: u32,
    gb: f64,
    d: &'static str,
    v: &'static str,
    m: &'static [&'static str],
    lock: bool,
    store: bool,
}

const fn sa(n: &'static str, p: &'static str, g: &'static str, a: u32, b: u32, gb: f64, d: &'static str, v: &'static str) -> Sample {
    Sample { n, p, g, a, b, gb, d, v, m: &[], lock: false, store: false }
}

/// menu-v22 `INST` (drawing data, made up), in its order.
const INST: [Sample; 20] = [
    sa("Call of Duty", "Activision", "pad", 0x7d8592, 0x2f343d, 236.0, "2026-09-02", "1.62.4"),
    sa("VALORANT", "Riot Games, Inc", "pad", 0xff7a76, 0xd83f4c, 46.2, "2025-03-14", "11.07"),
    sa("Apex Legends", "Electronic Arts", "pad", 0xff8a5c, 0xc4382a, 71.4, "2026-05-21", "v3.0.82"),
    sa("Counter-Strike 2", "Valve", "pad", 0xffb84d, 0xd97a14, 38.9, "2025-11-08", "1.40.9"),
    sa("Rocket League", "Psyonix LLC", "pad", 0x5aa8ff, 0x2a5fd6, 33.7, "2024-12-19", "2.53"),
    sa("Adobe Premiere Pro 2026", "Adobe Inc.", "mplay", 0xa58bff, 0x4b2fbf, 9.8, "2026-06-30", "26.1"),
    Sample { m: &["Repair"], ..sa("Epic Games Launcher", "Epic Games, Inc.", "pad", 0x4a4f5a, 0x1d2027, 2.9, "2024-12-19", "18.4.1") },
    sa("NVIDIA App", "NVIDIA Corporation", "chip", 0x9be15d, 0x4a9a1c, 1.4, "2026-09-28", "11.0.5"),
    sa("Steam", "Valve Corporation", "pad", 0x6f8fb8, 0x2b3f5c, 1.2, "2024-02-11", "2.10.91"),
    Sample { m: &["Modify"], ..sa("Google Chrome", "Google LLC", "globe", 0xffd35a, 0xe6493b, 0.74, "2026-10-01", "141.0") },
    Sample {
        m: &["Modify", "Repair"],
        ..sa("Microsoft Visual C++ 2015-2022 Redistributable (x64)", "Microsoft Corporation", "cube", 0x9a8cff, 0x5a4bd0, 0.025, "2026-03-11", "14.44.35211")
    },
    Sample { lock: true, ..sa("Microsoft Edge", "Microsoft Corporation", "globe", 0x4fd3a8, 0x2a76e8, 0.62, "2026-10-04", "141.0") },
    Sample { store: true, ..sa("Microsoft Teams", "Microsoft Corporation", "chat", 0x8a8ff0, 0x4b50c4, 0.52, "2026-04-09", "25.2") },
    sa("OBS Studio", "OBS Project", "rec", 0x7a808c, 0x3a3e47, 0.41, "2026-08-17", "32.0.1"),
    Sample { store: true, ..sa("WhatsApp", "WhatsApp Inc.", "chat", 0x5fe08a, 0x1f9e55, 0.41, "2026-07-23", "2.2538") },
    sa("Discord", "Discord Inc.", "chat", 0x8f95ff, 0x5a5fe0, 0.39, "2025-01-05", "1.0.9205"),
    Sample { store: true, ..sa("Spotify", "Spotify AB", "note", 0x46d989, 0x1c9a5a, 0.32, "2025-06-12", "1.2.71") },
    sa("Wootility", "Wooting", "kb16", 0xffb86b, 0xe0661c, 0.21, "2025-09-30", "5.1.2"),
    sa("Riot Vanguard", "Riot Games, Inc", "shd16", 0xff7a76, 0xd83f4c, 0.04, "2025-03-14", "1.17"),
    sa("7-Zip 24.09", "Igor Pavlov", "zip", 0xa2abbd, 0x6c7487, 0.006, "2024-11-30", "24.09"),
];

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

// ------------------------------------------------------------------------------------------------- the three groups

/// Which group a row sits in (the owner, test build 1: "sorting here depending on whats system, windows, and whats actual
/// programs"): the programs the user installed first, then runtimes / drivers / anti-cheats, then what Windows ships.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Cat {
    Programs,
    System,
    Windows,
}

impl Cat {
    pub const ALL: [Cat; 3] = [Cat::Programs, Cat::System, Cat::Windows];
    pub fn title(self) -> &'static str {
        match self {
            Cat::Programs => "Programs",
            Cat::System => "System and drivers",
            Cat::Windows => "Windows",
        }
    }
}

/// Words in a name that mark a runtime, driver, SDK or anti-cheat (lower case).
const SYSTEM_WORDS: [&str; 16] = [
    "redistributable",
    "runtime",
    "driver",
    ".net",
    "visual c++",
    "directx",
    "physx",
    "chipset",
    "sdk",
    "framework",
    "vanguard",
    "anti-cheat",
    "anticheat",
    "battleye",
    "firmware",
    "webview2",
];

/// Names Windows itself puts on the PC (lower case, start of the name; publisher Microsoft).
const WINDOWS_NAMES: [&str; 10] = [
    "windows ",
    "microsoft edge",
    "microsoft onedrive",
    "microsoft update health",
    "microsoft store",
    "microsoft gameinput",
    "xbox",
    "update for windows",
    "microsoft windows",
    "microsoft 365 (office)",
];

/// The group of one app.
pub fn category(a: &InstalledApp) -> Cat {
    let name = a.name.to_lowercase();
    let microsoft = a.publisher.as_deref().map(|p| p.to_lowercase().contains("microsoft")).unwrap_or(false);
    if a.lock == Some(bu_apps::LockReason::WindowsKeeps) {
        return Cat::Windows;
    }
    if microsoft {
        // a Store app of Microsoft's own family ("Microsoft.WindowsCalculator_8wekyb3d8bbwe") came with Windows
        if let bu_apps::AppSource::Store { family_name, .. } = &a.source {
            if family_name.starts_with("Microsoft.") {
                return Cat::Windows;
            }
        }
        if WINDOWS_NAMES.iter().any(|w| name.starts_with(w)) {
            return Cat::Windows;
        }
    }
    if SYSTEM_WORDS.iter().any(|w| name.contains(w)) {
        return Cat::System;
    }
    Cat::Programs
}

/// FILETIME of a "yyyy-mm-dd" (00:00 UTC).
fn filetime(date: &str) -> u64 {
    let p: Vec<i64> = date.split('-').map(|x| x.parse().unwrap_or(1)).collect();
    let (y, m, dd) = (p[0], p[1], p[2]);
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + dd - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    116_444_736_000_000_000 + (days as u64) * 86_400 * 10_000_000
}

/// The FAKE OS layer holding the drawing's 20 sample apps (what a test copy and the pixel proof show). Store apps get a
/// measured folder size so they show the drawing's sizes.
pub fn sample_os() -> FakeOs {
    let mut os = FakeOs::new();
    for (i, a) in INST.iter().enumerate() {
        let dir = format!(r"C:\Program Files\{}", a.n);
        os = os.dir(&dir);
        if a.store {
            let name = format!("Sample.{}", a.n.replace(' ', ""));
            os = os
                .package(RawPackage {
                    full_name: format!("{name}_{}_x64__s4mpl3", a.v),
                    family_name: format!("{name}_s4mpl3"),
                    name: name.clone(),
                    display_name: Some(a.n.into()),
                    publisher: Some(a.p.into()),
                    version: a.v.into(),
                    installed: Some(filetime(a.d)),
                    logo: None,
                    installed_path: Some(dir.clone().into()),
                    is_framework: false,
                    is_resource: false,
                    is_bundle: false,
                    is_optional: false,
                    signature: Signature::Store,
                })
                .folder(&dir, (a.gb * 1024.0 * 1024.0).round() as u64 * 1024);
            continue;
        }
        let kb = (a.gb * 1024.0 * 1024.0).round() as u32;
        let date = a.d.replace('-', "");
        let mut v: Vec<(&str, bu_apps::os::RegValue)> = vec![
            ("DisplayName", s(a.n)),
            ("Publisher", s(a.p)),
            ("DisplayVersion", s(a.v)),
            ("EstimatedSize", d(kb)),
            ("InstallDate", s(&date)),
            ("InstallLocation", s(&dir)),
        ];
        let keyname = match a.m {
            // Repair only = an MSI that says NoModify (MsiExec.exe /f{GUID})
            ["Repair"] => {
                v.push(("WindowsInstaller", d(1)));
                v.push(("NoModify", d(1)));
                format!("{{5A3B0C{:02X}-0000-4000-8000-000000000023}}", i)
            }
            ["Modify"] => {
                v.push(("ModifyPath", s(&format!("\"{dir}\\setup.exe\" --modify"))));
                v.push(("UninstallString", s(&format!("\"{dir}\\setup.exe\" --uninstall"))));
                a.n.to_string()
            }
            ["Modify", "Repair"] => {
                v.push(("ModifyPath", s(&format!("\"{dir}\\bundle.exe\" /modify"))));
                v.push(("BundleCachePath", s(&format!("{dir}\\bundle.exe"))));
                v.push(("UninstallString", s(&format!("\"{dir}\\bundle.exe\" /uninstall"))));
                a.n.to_string()
            }
            _ => {
                v.push(("UninstallString", s(&format!("\"{dir}\\uninstall.exe\""))));
                a.n.to_string()
            }
        };
        os = os.entry(Hive::LocalMachine, RegView::Bits64, &keyname, &v, None);
    }
    os
}

/// The drawing's tile for a sample app (by name); a real app gets the plain grey app tile.
fn sample_tile(name: &str) -> Tile {
    match INST.iter().find(|a| a.n == name) {
        Some(a) => Tile::Glyph { glyph: a.g, a: Rgba::hex(a.a), b: Rgba::hex(a.b) },
        None => Tile::Glyph { glyph: "appw", a: Rgba::hex(0xa2abbd), b: Rgba::hex(0x6c7487) },
    }
}

// ------------------------------------------------------------------------------------------------- formatting

/// The drawing's `GBf` (sizes in GB): ≥1000 → "x.xx TB", ≥100 → "236 GB", ≥1 → "46.2 GB", >0 → "26 MB", else "0 B".
pub fn gbf(gb: f64) -> String {
    if gb >= 1000.0 {
        format!("{:.2} TB", gb / 1024.0)
    } else if gb >= 100.0 {
        format!("{} GB", gb.round())
    } else if gb >= 1.0 {
        format!("{:.1} GB", gb)
    } else if gb > 0.0 {
        format!("{} MB", ((gb * 1024.0).round() as i64).max(1))
    } else {
        "0 B".into()
    }
}

fn size_text(a: &InstalledApp) -> String {
    a.size_bytes.map(|b| gbf(b as f64 / GIB)).unwrap_or_else(|| "\u{2014}".into())
}

fn gb_of(a: &InstalledApp) -> f64 {
    a.size_bytes.map(|b| b as f64 / GIB).unwrap_or(0.0)
}

// ------------------------------------------------------------------------------------------------- page state

/// An open popup list.
#[derive(Clone, Debug, PartialEq)]
enum Menu {
    /// the right-click menu of a row at the pointer
    Context { id: String, x: f32, y: f32 },
    /// the wrench's two choices under its button
    Fix { id: String, bx: (f32, f32, f32, f32) },
}

/// What a right-click menu row does.
#[derive(Clone, Copy, Debug, PartialEq)]
enum MenuAct {
    Uninstall,
    Fix(Fix),
    Folder,
}

/// A row while an uninstall runs.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RowRun {
    Waiting,
    Now,
    /// folding away since (ms)
    Gone(f64),
}

enum Msg {
    Listed(AppList),
    Progress(usize, Progress),
    Done(Vec<Result<Outcome, AppsError>>),
    Fixed(String, Result<FixOutcome, AppsError>),
    Opened(String, Result<(), AppsError>),
}

/// A worker thread's message to the page, and the menu woken so it shows (Order 047: the page no longer asks for a frame
/// every 3 ms while a worker runs - the menu sleeps until this wake).
fn post(tx: &mpsc::Sender<Msg>, m: Msg) {
    let _ = tx.send(m);
    crate::services::Waker.wake();
}

#[derive(Default)]
pub struct Apps {
    env: Env,
    svc: Option<Svc>,
    apps: Vec<InstalledApp>,
    loading: bool,
    query: String,
    sort: Option<(SortKey, bool)>,
    sel: HashSet<String>,
    anchor: Option<String>,
    menu: Option<Menu>,
    menu_at: f64,
    /// the confirm: the apps it asks about + when it opened
    dlg: Option<(Vec<String>, f64)>,
    /// the confirm is closing since (Cancel / a click beside it): it fades out, then goes
    dlg_closing: Option<f64>,
    /// the uninstall in progress: the apps in order + each row's state
    run: Option<(Vec<InstalledApp>, HashMap<String, RowRun>)>,
    toast: Option<(String, f64)>,
    rx: Option<mpsc::Receiver<Msg>>,
    tx: Option<mpsc::Sender<Msg>>,
    /// an app's own Modify / Repair setup is running (its name): no second one starts meanwhile
    fixing: Option<String>,
    /// wrench button boxes (window coordinates) from their press, for the two-choice menu
    press_box: HashMap<Key, (f32, f32, f32, f32)>,
    /// a click on a locked row pulses its lock (`if(a.lock){nudge(a.el.querySelector('.prk'));return;}`): the row + since
    nudge: Option<(String, f64)>,
    now: f64,
}

impl Apps {
    fn svc(&self) -> Option<Svc> {
        self.svc.clone()
    }

    fn sorted(&self) -> (SortKey, bool) {
        self.sort.unwrap_or((SortKey::Size, true))
    }

    /// The rows in the list's order (sorted; every app, matching or not).
    fn ordered(&self) -> Vec<&InstalledApp> {
        let (k, desc) = self.sorted();
        let mut v: Vec<InstalledApp> = self.apps.clone();
        bu_apps::sort(&mut v, k, desc);
        // the groups (Programs, System, Windows) in that order, each sorted by the column (a stable sort keeps it)
        v.sort_by_key(category);
        let order: Vec<String> = v.into_iter().map(|a| a.id).collect();
        order.iter().filter_map(|id| self.apps.iter().find(|a| &a.id == id)).collect()
    }

    fn matches(&self, a: &InstalledApp) -> bool {
        !bu_apps::search(std::slice::from_ref(a), &self.query).is_empty()
    }

    /// The rows shown (sorted, matching the search).
    fn view(&self) -> Vec<&InstalledApp> {
        self.ordered().into_iter().filter(|a| self.matches(a)).collect()
    }

    fn by_id(&self, id: &str) -> Option<&InstalledApp> {
        self.apps.iter().find(|a| a.id == id)
    }

    fn row_key(id: &str) -> Key {
        sub(K_ROW, id)
    }

    fn busy(&self) -> bool {
        self.run.is_some()
    }

    fn send(&self) -> mpsc::Sender<Msg> {
        self.tx.clone().expect("open")
    }

    fn show_toast(&mut self, t: String) {
        self.toast = Some((t, self.now));
    }

    /// Read the list: right away from the fake; on a worker thread from Windows (registry + package manager, ~1 s).
    fn load(&mut self) {
        let Some(svc) = self.svc() else { return };
        if self.env.fake() {
            let mut l = svc.list();
            // the drawing shows the Store apps' sizes: the fake measures them (instant); a real Store app stays "—" until a
            // later order adds a Measure button (measuring is slow work: only on a button)
            for a in l.apps.iter_mut().filter(|a| a.kind == AppKind::Store) {
                *a = svc.measure_size(a);
            }
            self.apply_list(l);
        } else {
            self.loading = true;
            let tx = self.send();
            std::thread::spawn(move || {
                post(&tx, Msg::Listed(svc.list()));
            });
        }
    }

    fn apply_list(&mut self, l: AppList) {
        self.loading = false;
        self.apps = l.apps;
        let ids: HashSet<String> = self.apps.iter().filter(|a| a.can_uninstall()).map(|a| a.id.clone()).collect();
        self.sel.retain(|i| ids.contains(i));
    }

    // ---- selecting (the drawing's apClick)

    /// A click on a row: just this one (a second click on the only selected one clears it). Ctrl / Shift wait for the page
    /// API (PIECES_WANTED: modifiers on a click).
    fn click_row(&mut self, id: &str) {
        let Some(a) = self.by_id(id) else { return };
        if !a.can_uninstall() {
            self.nudge = Some((id.to_string(), self.now));
            return;
        }
        let only = self.sel.len() == 1 && self.sel.contains(id);
        self.sel.clear();
        if !only {
            self.sel.insert(id.to_string());
        }
        self.anchor = Some(id.to_string());
    }

    /// Ctrl+click adds / removes the row; Shift+click = the range from the last clicked row (with Ctrl: added to what is
    /// picked); locked rows are skipped (the drawing's apClick).
    fn click_row_mod(&mut self, id: &str, ctrl: bool, shift: bool) {
        match self.by_id(id) {
            None => return,
            Some(a) if !a.can_uninstall() => {
                // the lock check comes first in apClick: Ctrl / Shift + click on a locked row pulses its lock too
                self.nudge = Some((id.to_string(), self.now));
                return;
            }
            Some(_) => {}
        }
        let view: Vec<String> = self.view().iter().map(|a| a.id.clone()).collect();
        let anchor = self.anchor.clone().and_then(|a| view.iter().position(|v| *v == a));
        if let (true, Some(i)) = (shift, anchor) {
            let j = view.iter().position(|v| v == id).unwrap_or(i);
            if !ctrl {
                self.sel.clear();
            }
            for v in &view[i.min(j)..=i.max(j)] {
                if self.by_id(v).map(|a| a.can_uninstall()).unwrap_or(false) {
                    self.sel.insert(v.clone());
                }
            }
            return;
        }
        self.tick_row(id);
    }

    /// Keys while nothing has focus (the drawing): Ctrl+F = the search, Ctrl+A = every row shown, Delete = uninstall the
    /// picked ones, Esc = clear the pick.
    fn page_key(&mut self, vk: u16, cx: &mut Cx) {
        let m = cx.mods;
        if m.alt || self.menu.is_some() || self.dlg.is_some() {
            return;
        }
        match vk {
            0x46 if m.ctrl => {
                cx.focus(Some(K_SEARCH));
                cx.used = true;
            }
            0x41 if m.ctrl => {
                let vis: Vec<String> = self.view().iter().filter(|a| a.can_uninstall()).map(|a| a.id.clone()).collect();
                self.sel.extend(vis);
                cx.used = true;
            }
            0x2E if !self.sel.is_empty() => {
                let ids = self.selected().iter().map(|a| a.id.clone()).collect();
                self.confirm(ids);
                cx.used = true;
            }
            0x1B if !self.sel.is_empty() => {
                self.sel.clear();
                cx.used = true;
            }
            _ => {}
        }
    }

    fn tick_row(&mut self, id: &str) {
        if self.by_id(id).map(|a| !a.can_uninstall()).unwrap_or(true) {
            return;
        }
        if !self.sel.remove(id) {
            self.sel.insert(id.to_string());
        }
        self.anchor = Some(id.to_string());
    }

    fn select_all(&mut self) {
        let vis: Vec<String> = self.view().iter().filter(|a| a.can_uninstall()).map(|a| a.id.clone()).collect();
        let all = !vis.is_empty() && vis.iter().all(|i| self.sel.contains(i));
        self.sel.clear();
        if !all {
            self.sel.extend(vis);
        }
    }

    fn selected(&self) -> Vec<&InstalledApp> {
        self.ordered().into_iter().filter(|a| self.sel.contains(&a.id)).collect()
    }

    // ---- the right-click menu (opened by the page API's right-click event once it lands; tests call it directly)

    /// Open the row's right-click menu at the pointer (window coordinates).
    pub fn context_menu(&mut self, id: &str, x: f32, y: f32) {
        if self.by_id(id).is_none() || self.busy() {
            return;
        }
        self.menu = Some(Menu::Context { id: id.to_string(), x, y });
        self.menu_at = self.now;
    }

    // ---- actions

    fn confirm(&mut self, ids: Vec<String>) {
        let ids: Vec<String> = ids.into_iter().filter(|i| self.by_id(i).map(|a| a.can_uninstall()).unwrap_or(false)).collect();
        if ids.is_empty() || self.busy() {
            return;
        }
        self.menu = None;
        self.dlg = Some((ids, self.now));
        self.dlg_closing = None;
    }

    /// The confirm's Uninstall: one after another on a worker thread (the app's own uninstaller may open; Windows may ask
    /// for admin). Each row says "Waiting…" / "Uninstalling…", then folds away.
    fn uninstall(&mut self) {
        let Some((ids, _)) = self.dlg.take() else { return };
        self.dlg_closing = None;
        let Some(svc) = self.svc() else { return };
        let list: Vec<InstalledApp> = ids.iter().filter_map(|i| self.by_id(i).cloned()).collect();
        if list.is_empty() {
            return;
        }
        self.sel.clear();
        let states = list.iter().map(|a| (a.id.clone(), RowRun::Waiting)).collect();
        self.run = Some((list.clone(), states));
        let tx = self.send();
        let job = move || {
            let refs: Vec<&InstalledApp> = list.iter().collect();
            let tx2 = tx.clone();
            let out = svc.uninstall_many(&refs, move |i, p| {
                post(&tx2, Msg::Progress(i, p));
            });
            post(&tx, Msg::Done(out));
        };
        if self.env.fake() {
            job();
        } else {
            std::thread::spawn(job);
        }
        self.pump();
    }

    fn fix(&mut self, id: &str, f: Fix) {
        self.menu = None;
        let (Some(svc), Some(a)) = (self.svc(), self.by_id(id).cloned()) else { return };
        if let Some(n) = &self.fixing {
            self.show_toast(format!("{n}\u{2019}s setup is still open"));
            return;
        }
        self.fixing = Some(a.name.clone());
        let tx = self.send();
        let job = move || {
            let r = svc.fix(&a, f);
            post(&tx, Msg::Fixed(a.name.clone(), r));
        };
        if self.env.fake() {
            job();
        } else {
            std::thread::spawn(job);
        }
        self.pump();
    }

    /// The wrench: one choice = straight away; two = a small menu under the button.
    fn wrench(&mut self, id: &str, k: Key) {
        let Some(a) = self.by_id(id) else { return };
        let fx = a.fixes();
        match fx.len() {
            0 => {}
            1 => self.fix(id, fx[0]),
            _ => {
                let bx = self.press_box.get(&k).copied().unwrap_or((0.0, 0.0, 24.0, 24.0));
                self.menu = Some(Menu::Fix { id: id.to_string(), bx });
                self.menu_at = self.now;
            }
        }
    }

    fn open_folder(&mut self, id: &str) {
        self.menu = None;
        let (Some(svc), Some(a)) = (self.svc(), self.by_id(id).cloned()) else { return };
        // off the UI thread, like `fix`: the folder check on an offline network path can take the SMB timeout
        let tx = self.send();
        let job = move || {
            let r = svc.open_folder(&a);
            post(&tx, Msg::Opened(a.name.clone(), r));
        };
        if self.env.fake() {
            job();
        } else {
            std::thread::spawn(job);
        }
        self.pump();
    }

    /// Take in what the worker threads sent.
    fn pump(&mut self) -> bool {
        let mut msgs = Vec::new();
        if let Some(rx) = &self.rx {
            while let Ok(m) = rx.try_recv() {
                msgs.push(m);
            }
        }
        let any = !msgs.is_empty();
        for m in msgs {
            match m {
                Msg::Listed(l) => self.apply_list(l),
                Msg::Progress(i, p) => {
                    let now = self.now;
                    if let Some((list, st)) = &mut self.run {
                        if let Some(a) = list.get(i) {
                            match p {
                                Progress::Waiting => {
                                    st.insert(a.id.clone(), RowRun::Waiting);
                                }
                                Progress::Uninstalling => {
                                    st.insert(a.id.clone(), RowRun::Now);
                                }
                                Progress::Done(Outcome::Uninstalled { .. }) => {
                                    st.insert(a.id.clone(), RowRun::Gone(now));
                                }
                                // not uninstalled / finishes after a restart / failed: the row comes back as it was
                                _ => {
                                    st.remove(&a.id);
                                }
                            }
                        }
                    }
                }
                Msg::Done(out) => {
                    let list = self.run.as_ref().map(|r| r.0.clone()).unwrap_or_default();
                    self.finish(&list, &out);
                }
                Msg::Fixed(name, r) => {
                    self.fixing = None;
                    let t = match r {
                        Ok(FixOutcome::SetupEnded { .. }) => format!("{name} \u{00b7} its setup closed"),
                        Ok(FixOutcome::OpenedSettings) => format!("{name} \u{00b7} Repair and Reset are in Windows Settings"),
                        Err(AppsError::Os(OsError::Cancelled)) => format!("{name} \u{00b7} cancelled"),
                        Err(AppsError::NeedsAdmin) => format!("{name} \u{00b7} needs admin"),
                        Err(e) => format!("{name} \u{00b7} {e}"),
                    };
                    self.show_toast(t);
                }
                Msg::Opened(_, Ok(())) => {}
                Msg::Opened(name, Err(AppsError::NoFolder)) => self.show_toast(format!("{name}: no install folder")),
                Msg::Opened(name, Err(e)) => self.show_toast(format!("{name} \u{00b7} {e}")),
            }
        }
        any
    }

    /// The end of an uninstall run: the toast (the drawing's "N apps uninstalled · X freed"), what did not go, and the list
    /// read again.
    fn finish(&mut self, list: &[InstalledApp], out: &[Result<Outcome, AppsError>]) {
        let mut gone = Vec::new();
        let mut restart = Vec::new();
        let mut kept = Vec::new();
        for (a, r) in list.iter().zip(out) {
            match r {
                Ok(Outcome::Uninstalled { .. }) => gone.push(a),
                Ok(Outcome::NeedsRestart) => restart.push(a),
                _ => kept.push(a),
            }
        }
        let gb: f64 = gone.iter().map(|a| gb_of(a)).sum();
        let t = if !kept.is_empty() {
            if kept.len() == 1 {
                format!("{} was not uninstalled", kept[0].name)
            } else {
                format!("{} apps were not uninstalled", kept.len())
            }
        } else if !restart.is_empty() {
            format!("{} finishes after a restart", restart[0].name)
        } else if gone.len() > 1 {
            format!("{} apps uninstalled \u{00b7} {} freed", gone.len(), gbf(gb))
        } else if let Some(a) = gone.first() {
            format!("{} uninstalled \u{00b7} {} freed", a.name, gbf(gb))
        } else {
            String::new()
        };
        if !t.is_empty() {
            self.show_toast(t);
        }
        // the folding rows finish their 220 ms, then the list is read again
        let now = self.now;
        if let Some((_, st)) = &mut self.run {
            st.retain(|_, v| matches!(v, RowRun::Gone(_)));
            for v in st.values_mut() {
                if let RowRun::Gone(t0) = v {
                    *t0 = t0.min(now);
                }
            }
        }
        let still: Vec<String> = self.run.as_ref().map(|r| r.1.keys().cloned().collect()).unwrap_or_default();
        if still.is_empty() {
            self.run = None;
        }
        self.load();
    }

    // ------------------------------------------------------------------------------------------------- building

    fn head(&self, cx: &mut Cx) -> El {
        let (sk, desc) = self.sorted();
        let mut kids = Vec::new();
        for (i, (k, label)) in COLS.iter().enumerate() {
            let bk = idx(K_SORT, i);
            let on = *k == sk;
            let up = on && !desc;
            let hv = cx.hover_t(bk, 120.0, EASE);
            let col = if on { FG() } else { cmix(FG3(), FG2(), hv) };
            let ar = cx.tr(bk, 1, if on { 1.0 } else { 0.0 }, 120.0, EASE);
            let rot = cx.tr(bk, 2, if up { 1.0 } else { 0.0 }, 200.0, EASE);
            // `.prh button i{display:block;width:8px;height:5px;opacity:0}` `.on i{opacity:1}` `.up i{transform:rotate(180deg)}`
            // `.prh svg{width:8px;height:5px;stroke:currentColor;stroke-width:1.4}`
            let arrow = El::icon("cd", 8.0, 1.4, col).size(8.0, 5.0).none().opacity(ar).rotate(180.0 * rot).no_hit();
            let text = El::text(*label, pieces::btn_font(11.0, 600), col, lh(11.0, 1.35)).none();
            // `#sw .prh button{display:flex;align-items:center;justify-content:flex-end;gap:4px;height:22px;margin:0 -5px;padding:0 5px;
            //   border-radius:5px;font-size:11px;font-weight:600;color:var(--fg3)}` `:hover{background:var(--hov);color:var(--fg2)}`
            //   `.on{color:var(--fg)}` `#sw .prh.aphd button.nm{margin-left:31px}` + `.nm{justify-content:flex-start}`
            //   `#sw .prh.aphd button:not(.nm){flex-direction:row-reverse;justify-content:flex-start}` (= the label at the right edge)
            let mut b = El::row().center().gap(4.0).h(22.0).pad(0.0, 5.0, 0.0, 5.0).radius(5.0).bg(HOV().mul_a(hv)).on_click(bk).cursor(Cursor::Hand);
            if *k == SortKey::Name {
                b = b.margin(0.0, -5.0, 0.0, 31.0).justify(JustifyContent::FLEX_START).child(text).child(arrow);
            } else {
                b = b.margin(0.0, -5.0, 0.0, -5.0).justify(JustifyContent::FLEX_END).child(arrow).child(text);
            }
            kids.push(b);
        }
        // the select-all box: `.aphd .tkb{position:absolute;left:7px;top:50%;margin-top:-9px;opacity:0}`
        // `.aphd:hover .tkb,.apg.apany .tkb{opacity:1}`
        let vis: Vec<&InstalledApp> = self.view().into_iter().filter(|a| a.can_uninstall()).collect();
        let all = !vis.is_empty() && vis.iter().all(|a| self.sel.contains(&a.id));
        let some = !all && !self.sel.is_empty();
        let show = cx.hovered(sub(K_SORT, "hd")) || !self.sel.is_empty();
        let op = cx.tr(K_ALL, 9, if show { 1.0 } else { 0.0 }, 120.0, EASE);
        let tb = tick_box(cx, K_ALL, all, some).abs(7.0, 6.0, f32::NAN, f32::NAN).opacity(op).z(1);
        grid_row().h(30.0).inset(&[sh(0.0, -1.0, 0.0, 0.0, HAIR())]).key(sub(K_SORT, "hd")).children(kids).child(tb)
    }

    fn row(&self, cx: &mut Cx, a: &InstalledApp, first: bool) -> El {
        let rk = Self::row_key(&a.id);
        let lock = !a.can_uninstall();
        let sel = self.sel.contains(&a.id);
        let run = self.run.as_ref().and_then(|r| r.1.get(&a.id).copied());
        let hv = if run.is_some() { 0.0 } else { cx.hover_t(rk, 120.0, EASE) };
        let row_hover = cx.hovered(rk);
        // `.prr:hover{background:var(--hov)}` `.prr.aprow.sel{background:var(--sel)}` `.aprow.apwait{background:transparent!important}`
        let bg = if run.is_some() { Rgba(0.0, 0.0, 0.0, 0.0) } else if sel { SEL() } else { HOV().mul_a(hv) };

        // the name: `.apnm{display:flex;align-items:center;gap:6px;min-width:0;font-size:12.5px;line-height:17px}`
        // `.tti{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;padding-bottom:2px;margin-bottom:-2px}`
        // `.prr.prot .prn>span:not(.ait):not(.prk){color:var(--fg3)}`
        let name_col = if lock { FG3() } else { FG() };
        let mut nm = El::row().center().gap(6.0).min_w(0.0).child(
            El::text(a.name.clone(), Font::new(12.5, 400), name_col, 17.0).ellipsis().shrink(1.0).pad(0.0, 0.0, 2.0, 0.0).margin(0.0, 0.0, -2.0, 0.0),
        );
        if a.kind == AppKind::Store {
            // `.apnm .tag{height:15px;line-height:15px;padding:0 5px;font-size:9.5px}` over `.tag{background:var(--ctl);color:var(--fg3);
            // font-weight:600;letter-spacing:.02em;border-radius:8px}`
            nm = nm.child(
                El::row().center().h(15.0).none().pad(0.0, 5.0, 0.0, 5.0).radius(8.0).bg(CTL()).child(El::text("Store", Font::new(9.5, 600).ls(200), FG3(), 15.0)),
            );
        }
        if lock {
            // `.prk{display:grid;place-items:center;width:14px;height:14px;color:var(--fg3)}` `.prk svg{width:11px;height:11px;stroke-width:1.3}`
            // `data-tip="Windows keeps this one · it can’t be uninstalled"`; a click on the row = `nudge(.prk)`: `if(RM||!el)return;`
            // `anim(el,[{transform:'scale(1)'},{transform:'scale(1.14)',offset:.35},{transform:'scale(1)'}],{duration:340,easing:EASE_OUT})`
            let ns = match &self.nudge {
                Some((id, t0)) if *id == a.id && !cx.rm && cx.now - t0 < 340.0 => {
                    cx.st.busy = true;
                    crate::anim::nudge(cx.now - t0) as f32
                }
                _ => 1.0,
            };
            nm = nm.child(
                El::block()
                    .size(14.0, 14.0)
                    .none()
                    .place_center()
                    .scale(ns)
                    .key(sub(rk, "lock"))
                    .tip("Windows keeps this one \u{00b7} it can\u{2019}t be uninstalled")
                    .child(El::icon("lock", 11.0, 1.3, FG3()).no_hit()),
            );
        }
        // `.aptx small{font-size:11px;line-height:14px;color:var(--fg3);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;
        //   padding-bottom:2px;margin-bottom:-2px}`
        let small = El::text(a.publisher.clone().unwrap_or_default(), Font::new(11.0, 400), FG3(), 14.0)
            .ellipsis()
            .pad(0.0, 0.0, 2.0, 0.0)
            .margin(0.0, 0.0, -2.0, 0.0);
        let tx = El::col().min_w(0.0).child(nm).child(small);
        // `.prr.prot .ait{filter:grayscale(1);opacity:.5}`
        let t = match sample_tile(&a.name) {
            Tile::Glyph { glyph, a: c1, b: c2 } if lock => Tile::Glyph { glyph, a: c1.gray(), b: c2.gray() },
            t => t,
        };
        let mut ti = tile(&t, 26.0);
        if lock {
            ti = ti.opacity(0.5);
        }
        // `.prn{display:flex;align-items:center;gap:9px;min-width:0}` `.aprow .prn{gap:10px}`
        let prn = El::row().center().gap(10.0).min_w(0.0).child(ti).child(tx);

        // `.prc{text-align:right;font-size:12px;color:var(--fg2);font-variant-numeric:tabular-nums;white-space:nowrap}`
        // `.prr.prot .prc{color:var(--fg3)}`
        let pc = if lock { FG3() } else { FG2() };
        let size = El::text(size_text(a), Font::new(12.0, 400).tnum(), pc, lh(12.0, 1.35)).align(Align::Right);
        // `.aprow .apdt{font-size:11.5px;display:flex;align-items:center;justify-content:flex-end;gap:6px}`
        // `.aprow.apwait .apdt{color:var(--fg3)}` `.aprow.apnow .apdt{color:var(--fg)}`
        let df = Font::new(11.5, 400).tnum();
        let dl = lh(11.5, 1.35);
        let mut date = El::row().center().justify(JustifyContent::FLEX_END).gap(6.0);
        date = match run {
            Some(RowRun::Waiting) => date.child(El::text("Waiting\u{2026}", df, FG3(), dl)),
            Some(RowRun::Now) | Some(RowRun::Gone(_)) => date.child(bits::uspin(cx, 0.0)).child(El::text("Uninstalling\u{2026}", df, FG(), dl)),
            None => date.child(El::text(a.install_date.map(|d| d.display()).unwrap_or_else(|| "\u{2014}".into()), df, pc, dl)),
        };

        // the actions (always visible): [Modify / Repair] · Open install folder · Uninstall
        // `apB(ic,tip,..)`: each icon carries `data-tip` = its label (`fixL.join(' / ')`, "Open install folder", "Uninstall")
        let mut act = El::row().center().justify(JustifyContent::FLEX_END).gap(4.0);
        if run.is_none() {
            let fixes = a.fixes();
            if !fixes.is_empty() {
                let fl = fixes.iter().map(|f| f.label()).collect::<Vec<_>>().join(" / ");
                act = act.child(apq(cx, sub(rk, "fix"), "tool", false, row_hover).tip(&fl));
            }
            if a.folder().is_some() {
                act = act.child(apq(cx, sub(rk, "fold"), "fold", false, row_hover).tip("Open install folder"));
            }
            if lock {
                // `.apq.ph0{visibility:hidden;pointer-events:none}` - the empty trash slot at the very end
                act = act.child(El::block().size(24.0, 24.0).none());
            } else {
                act = act.child(apq(cx, sub(rk, "del"), "trash", true, row_hover).tip("Uninstall"));
            }
        }

        // the tick box: `.aprow .tkb{position:absolute;left:7px;top:50%;margin-top:-9px;opacity:0;transition:opacity .12s ease}`
        // `.aprow:hover .tkb,.aprow.sel .tkb,.apg.apany .tkb{opacity:1}` `.aprow.prot .tkb{visibility:hidden}`
        let show = !lock && run.is_none() && (row_hover || sel || !self.sel.is_empty());
        let op = cx.tr(sub(rk, "ck"), 9, if show { 1.0 } else { 0.0 }, 120.0, EASE);

        let mut r = grid_row()
            .h(46.0)
            .bg(bg)
            .on_click(rk)
            .child(prn)
            .child(size)
            .child(date)
            .child(act);
        if !lock && run.is_none() {
            r = r.child(tick_box(cx, sub(rk, "ck"), sel, false).abs(7.0, 14.0, f32::NAN, f32::NAN).opacity(op).z(1));
        }
        if !first {
            // `.prr::before{left:10px;right:0;top:0;height:1px;background:var(--hair)}` `.prr.aprow::before{left:30px}`
            r = r.child(El::block().abs(30.0, 0.0, 0.0, f32::NAN).h(1.0).bg(HAIR()).no_hit());
        }
        if let Some(RowRun::Gone(t0)) = run {
            // the row folds away: height 46 -> 0, opacity 1 -> 0, 220 ms EASE_OUT
            let e = EASE_OUT.ease(((self.now - t0) / 220.0).clamp(0.0, 1.0)) as f32;
            cx.st.busy = true;
            r = r.h(46.0 * (1.0 - e)).opacity(1.0 - e).clip();
        }
        r
    }

    /// The list in its groups (Programs, System and drivers, Windows): each group's header (`.gh` + its count) and box; the
    /// column header row sits in the first box. A group with no row to show (search) is left out; nothing at all = one box
    /// with the "No app matches" line.
    fn groups(&self, cx: &mut Cx) -> Vec<El> {
        let view: Vec<InstalledApp> = self.view().into_iter().cloned().collect();
        let gone = |a: &InstalledApp| matches!(self.run.as_ref().and_then(|r| r.1.get(&a.id).copied()), Some(RowRun::Gone(t0)) if self.now - t0 > 220.0);
        let count = |n: usize| El::text(n.to_string(), Font::new(11.0, 400), FG3(), lh(11.0, 1.35));
        let mut out = Vec::new();
        for c in Cat::ALL {
            let shown: Vec<&InstalledApp> = view.iter().filter(|a| category(a) == c && !gone(a)).collect();
            if shown.is_empty() {
                continue;
            }
            let mut rows = Vec::new();
            if out.is_empty() {
                rows.push(self.head(cx));
            }
            for (i, a) in shown.into_iter().enumerate() {
                rows.push(self.row(cx, a, i == 0));
            }
            out.push(group::gh(c.title()).child(count(self.apps.iter().filter(|a| category(a) == c).count())));
            // `.prg{overflow:hidden}`
            out.push(group::grp(rows).clip());
        }
        if out.is_empty() {
            let mut rows = vec![self.head(cx)];
            if !self.query.trim().is_empty() {
                // `.pnone{padding:22px 0;text-align:center;font-size:12.5px;color:var(--fg2)}`
                rows.push(
                    El::block()
                        .pad(22.0, 0.0, 22.0, 0.0)
                        .child(El::text(format!("No app matches \u{201c}{}\u{201d}", self.query.trim()), Font::new(12.5, 400), FG2(), lh(12.5, 1.35)).align(Align::Center)),
                );
            }
            out.push(group::gh("Installed apps").child(count(self.apps.len())));
            out.push(group::grp(rows).clip());
        }
        out
    }

    fn menu_el(&self, cx: &mut Cx) -> Option<El> {
        let m = self.menu.clone()?;
        Some(match m {
            Menu::Context { id, x, y } => {
                let a = self.by_id(&id)?.clone();
                // the drawing's apMenu: "publisher · version v" on top, Uninstall (red), a line, Modify / Repair (Store: Repair, Reset),
                // Open install folder
                let multi = self.sel.contains(&a.id) && self.sel.len() > 1;
                let head = format!("{} \u{00b7} version {}", a.publisher.clone().unwrap_or_default(), a.version.clone().unwrap_or_default());
                let un = if multi { format!("Uninstall {} apps", self.sel.len()) } else { "Uninstall".to_string() };
                let fixes = a.fixes();
                let mut list = vec![Row::Head(&head), Row::Item(It::icon("trash", &un).danger().disabled(!a.can_uninstall())), Row::Sep];
                for f in &fixes {
                    list.push(Row::Item(It::icon(if *f == Fix::Reset { "undo" } else { "tool" }, f.label())));
                }
                if a.folder().is_some() {
                    list.push(Row::Item(It::icon("fold", "Open install folder")));
                }
                mitems::menu(cx, K_MENU, &list, Place::At(x, y), 214.0)
            }
            Menu::Fix { id, bx } => {
                let a = self.by_id(&id)?.clone();
                let list: Vec<Row> = a.fixes().iter().map(|f| Row::Item(It::icon(if *f == Fix::Reset { "undo" } else { "tool" }, f.label()))).collect();
                mitems::menu(cx, K_FIXM, &list, Place::Under(bx.0, bx.1, bx.2, bx.3), 150.0)
            }
        })
    }

    /// What row i of the open right-click menu does (its rows: the header, Uninstall, a line, the fixes, Open install folder).
    fn context_action(&self, id: &str, i: usize) -> Option<MenuAct> {
        let a = self.by_id(id)?;
        let fixes = a.fixes();
        match i {
            1 => Some(MenuAct::Uninstall),
            i if i >= 3 && i - 3 < fixes.len() => Some(MenuAct::Fix(fixes[i - 3])),
            i if i == 3 + fixes.len() && a.folder().is_some() => Some(MenuAct::Folder),
            _ => None,
        }
    }

    /// The confirm (`.dlgw` + `.dlg.udlg`, the shared `udlg` piece): what goes, the space it frees, a word if it matters;
    /// Cancel · Uninstall.
    fn dialog(&self, cx: &mut Cx) -> Option<El> {
        let (ids, at) = self.dlg.clone()?;
        let svc = self.svc()?;
        let list: Vec<&InstalledApp> = ids.iter().filter_map(|i| self.by_id(i)).collect();
        if list.is_empty() {
            return None;
        }
        let c = svc.confirm(&list);
        let gb: f64 = list.iter().map(|a| gb_of(a)).sum();
        let title = if list.len() > 1 { format!("Uninstall {} apps?", list.len()) } else { format!("Uninstall {}?", list[0].name) };
        let line = format!(
            "Frees about {}. {}",
            gbf(gb),
            if c.uninstaller_may_open { "An app\u{2019}s own uninstaller may open: finish it there." } else { "Store apps are removed straight away." }
        );
        // the list only for more than one app (the drawing hides it for one)
        let sizes: Vec<String> = list.iter().map(|a| size_text(a)).collect();
        let rows: Vec<UdRow> = if list.len() > 1 {
            list.iter().zip(&sizes).take(4).map(|(a, s)| UdRow { tile: sample_tile(&a.name), name: &a.name, size: s }).collect()
        } else {
            Vec::new()
        };
        let more = if list.len() > 4 { list.len() - 4 } else { 0 };
        let warn = c.warnings.join(" ");
        let footer = vec![
            cbtn_sized(cx, sub(K_DLG, "no"), "Cancel", Kind::Ghost, DFT, false, 76.0),
            cbtn_sized(cx, sub(K_DLG, "go"), "Uninstall", Kind::Red, DFT, false, 76.0),
        ];
        let warn = if warn.is_empty() { None } else { Some(warn.as_str()) };
        Some(udlg::udlg(cx, K_DLG, &title, &line, &rows, more, warn, footer, at, self.dlg_closing))
    }

    /// Cancel / a click beside the confirm: it fades out (`udlg::close_ms`), then `popup` drops it.
    fn close_dialog(&mut self) {
        if self.dlg.is_some() && self.dlg_closing.is_none() {
            self.dlg_closing = Some(self.now);
        }
    }

    /// The selection bar (the shared `.selbar` piece): "N selected" · total size · Uninstall · ×.
    fn bar(&self, cx: &mut Cx) -> Option<El> {
        let on = !self.sel.is_empty() && !self.busy() && self.dlg.is_none();
        if !on && cx.tr(K_BAR, 1, 0.0, 140.0, crate::anim::EASE) <= 0.001 {
            return None;
        }
        let n = self.sel.len();
        let gb: f64 = self.selected().iter().map(|a| gb_of(a)).sum();
        let count = format!("{n} selected");
        let size = gbf(gb);
        Some(selbar::selbar_x(cx, K_BAR, &count, Some(&size), &[Sbb { icon: "trash", label: "Uninstall", danger: true }], on, Some("Clear selection")))
    }
}

// ------------------------------------------------------------------------------------------------- page-local boxes

/// `.prh.aphd,.prr.aprow{display:grid;grid-template-columns:minmax(0,1fr) 62px 84px 84px;align-items:center;column-gap:8px;
/// padding:0 8px 0 30px}` (position: relative for the tick box)
fn grid_row() -> El {
    El::grid()
        .style(|s| {
            s.grid_template_columns = vec![minmax(length(0.0), fr(1.0)), length(62.0), length(84.0), length(84.0)];
        })
        .gap2(0.0, 8.0)
        .items(AlignItems::CENTER)
        .pad(0.0, 8.0, 0.0, 30.0)
}

/// The tick box (`reset::tick` = `.tkb`), with the header's "some ticked" look: `.tkb.tsome i{background:var(--acc);box-shadow:none}`
/// `.tkb.tsome i::after{content:'';width:7px;height:1.6px;border-radius:1px;background:#fff}` `.tkb.tsome svg{display:none}`.
fn tick_box(cx: &mut Cx, k: Key, on: bool, some: bool) -> El {
    if some {
        let b = El::block().size(15.0, 15.0).radius(4.0).bg(ACC()).place_center().child(El::block().size(7.0, 1.6).radius(1.0).bg(WHITE));
        return El::block().size(18.0, 18.0).none().place_center().on_click(k).cursor(Cursor::Hand).child(b);
    }
    let _ = DASH();
    crate::ui::pieces::reset::tick(cx, sub(k, "t"), on).on_click(k).cursor(Cursor::Hand)
}

/// One action icon: `#sw .apq{display:grid;place-items:center;width:24px;height:24px;border-radius:6px;background:transparent;
/// color:var(--fg2);transition:background-color .12s ease,color .12s ease}` `#sw .aprow .apq{color:var(--fg3)}`
/// `#sw .aprow:hover .apq,#sw .aprow .apq:hover{color:var(--fg)}` `#sw .aprow .apq.del{color:rgba(255,105,97,.78)}`
/// `#sw .aprow:hover .apq.del{color:var(--red)}` `#sw .apq:hover{background:var(--ctl-h)}` `#sw .apq.del:hover{background:rgba(255,69,58,.16)}`
/// `.apq svg{width:15px;height:15px;stroke-width:1.5}` `.apq:active{transform:scale(.92)}`
fn apq(cx: &mut Cx, k: Key, icon: &str, del: bool, row_hover: bool) -> El {
    let hv = cx.hover_t(k, 120.0, EASE);
    let rh = cx.tr(k, 3, if row_hover { 1.0 } else { 0.0 }, 120.0, EASE);
    let pr = cx.active(k);
    let col = if del { cmix(Rgba::rgba(255, 105, 97, 0.78), RED(), rh.max(hv)) } else { cmix(FG3(), FG(), rh.max(hv)) };
    let bg = if del { Rgba::rgba(255, 69, 58, 0.16) } else { CTL_H() };
    El::block()
        .size(24.0, 24.0)
        .none()
        .radius(6.0)
        .bg(bg.mul_a(hv))
        .place_center()
        .scale(if pr { 0.92 } else { 1.0 })
        .on_click(k)
        .cursor(Cursor::Hand)
        .child(El::icon(icon, 15.0, 1.5, col).no_hit())
}

// ------------------------------------------------------------------------------------------------- the page

impl Page for Apps {
    fn id(&self) -> &'static str {
        "apps"
    }
    fn name(&self) -> &'static str {
        "Apps"
    }
    fn icon(&self) -> &'static str {
        "apps"
    }
    fn open(&mut self, env: &Env, now: f64) {
        self.env = env.clone();
        self.now = now;
        let os = if env.fake() {
            Os::Fake(Box::new(sample_os()))
        } else {
            #[cfg(windows)]
            {
                Os::Real(bu_apps::real::RealOs::new())
            }
            #[cfg(not(windows))]
            {
                Os::Fake(Box::new(FakeOs::new()))
            }
        };
        self.svc = Some(Arc::new(bu_apps::Apps::new(os)));
        let (tx, rx) = mpsc::channel();
        self.tx = Some(tx);
        self.rx = Some(rx);
        self.load();
    }
    fn close(&mut self) {
        // closed = nothing kept (a running uninstall finishes on its own thread; its messages are dropped)
        let env = std::mem::take(&mut self.env);
        *self = Apps { env, ..Apps::default() };
    }
    fn tick(&mut self, now: f64) -> bool {
        self.now = now;
        self.pump()
    }
    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        self.now = cx.now;
        self.pump();
        // Order 047: a worker thread that is busy (the list read, an uninstall run) no longer keeps frames coming - each of
        // its messages wakes the menu (`post`) and `tick` pumps it. What moves meanwhile asks for its own frames: the
        // "Uninstalling…" spinner (`bits::uspin`), a folding row, a lock's pulse.
        if let Some((list, st)) = &self.run {
            let _ = list;
            if st.values().all(|v| matches!(v, RowRun::Gone(t0) if cx.now - t0 > 220.0)) && !st.is_empty() {
                self.run = None;
            }
        }
        vec![
            pieces::header(self.name(), Some(search::search(cx, K_SEARCH, &self.query, "Search apps", false))),
            El::block().children(self.groups(cx)),
        ]
    }
    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        // (Order 045: the event's own time - a lock's pulse starts at the click, not at the last build)
        self.now = cx.now;
        match ev {
            Ev::Char(k, c) if *k == K_SEARCH => search::edit_char(&mut self.query, *c),
            Ev::Key(k, vk) if *k == K_SEARCH => search::edit_key(&mut self.query, *vk),
            Ev::Key(k, vk) if *k == PAGE => self.page_key(*vk, cx),
            Ev::Click(k) if *k == sub(K_SEARCH, "x") => self.query.clear(),
            Ev::Press(k, _, _, bx) => {
                self.press_box.insert(*k, *bx);
            }
            Ev::Click(k) => {
                // Ctrl+click adds / removes a row, Shift+click picks a range (the drawing's apClick)
                let m = cx.mods;
                let row = self.apps.iter().map(|a| a.id.clone()).find(|id| *k == Self::row_key(id));
                match row {
                    Some(id) if (m.shift || m.ctrl) && !self.busy() => self.click_row_mod(&id, m.ctrl, m.shift),
                    _ => self.click(*k),
                }
            }
            Ev::Context(k, x, y) => {
                // a right-click on a row (or any part of it) opens its menu at the pointer
                let id = self.apps.iter().map(|a| a.id.clone()).find(|id| {
                    let rk = Self::row_key(id);
                    *k == rk || [sub(rk, "ck"), sub(rk, "del"), sub(rk, "fold"), sub(rk, "fix")].contains(k)
                });
                if let Some(id) = id {
                    self.context_menu(&id, *x, *y);
                }
            }
            _ => {}
        }
    }
    fn overlay(&mut self, cx: &mut Cx) -> Option<El> {
        self.bar(cx)
    }
    fn bar_shown(&self) -> bool {
        !self.sel.is_empty() && !self.busy() && self.dlg.is_none()
    }
    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let mut kids = Vec::new();
        if let Some(m) = self.menu_el(cx) {
            kids.push(m.z(20));
        }
        if let Some(t) = self.dlg_closing {
            if cx.now - t > udlg::close_ms(cx.rm) {
                self.dlg = None;
                self.dlg_closing = None;
            }
        }
        if let Some(d) = self.dialog(cx) {
            kids.push(d);
        }
        if let Some((t, at)) = self.toast.clone() {
            if cx.now - at < toast::SHOW_MS + 400.0 {
                kids.push(toast::toast(cx, K_TOAST, &t, at, !self.sel.is_empty()));
            } else {
                self.toast = None;
            }
        }
        if kids.is_empty() {
            return None;
        }
        Some(El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(crate::ui::WIN_W, crate::ui::WIN_H).no_hit().children(kids))
    }
    fn popup_dismiss(&mut self) {
        // a click beside the menu closes it (the confirm's own dim takes the clicks beside it: "out"); Esc closes the confirm
        self.menu = None;
        self.close_dialog();
    }
    fn describe(&self) -> String {
        let (k, desc) = self.sorted();
        format!(
            "apps={} shown={} sel={} sort={:?}{} menu={} dlg={} busy={} toast={:?}",
            self.apps.len(),
            self.view().len(),
            self.sel.len(),
            k,
            if desc { "-" } else { "+" },
            match &self.menu {
                Some(Menu::Context { .. }) => "context",
                Some(Menu::Fix { .. }) => "fix",
                None => "none",
            },
            self.dlg.as_ref().map(|d| d.0.len()).unwrap_or(0),
            self.busy(),
            self.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default()
        )
    }
}

impl Apps {
    fn click(&mut self, k: Key) {
        // sort headers
        for (i, (sk, _)) in COLS.iter().enumerate() {
            if k == idx(K_SORT, i) {
                let (cur, desc) = self.sorted();
                self.sort = Some(if cur == *sk { (cur, !desc) } else { (*sk, *sk != SortKey::Name) });
                return;
            }
        }
        if k == K_ALL {
            self.select_all();
            return;
        }
        // popups
        if k == sub(K_DLG, "no") || k == sub(K_DLG, "out") {
            self.close_dialog();
            return;
        }
        if k == sub(K_DLG, "go") {
            if self.dlg_closing.is_none() {
                self.uninstall();
            }
            return;
        }
        if k == sub(K_DLG, "win") {
            return;
        }
        if k == idx(K_BAR, 0) {
            let ids = self.selected().iter().map(|a| a.id.clone()).collect();
            self.confirm(ids);
            return;
        }
        if k == idx(K_BAR, 1) {
            self.sel.clear();
            return;
        }
        if let Some(Menu::Context { id, .. }) = self.menu.clone() {
            if let Some(i) = (0..8).find(|i| k == idx(K_MENU, *i)) {
                self.menu = None;
                match self.context_action(&id, i) {
                    Some(MenuAct::Uninstall) => {
                        let ids = if self.sel.contains(&id) && self.sel.len() > 1 { self.selected().iter().map(|a| a.id.clone()).collect() } else { vec![id] };
                        self.confirm(ids);
                    }
                    Some(MenuAct::Fix(f)) => self.fix(&id, f),
                    Some(MenuAct::Folder) => self.open_folder(&id),
                    None => {}
                }
                return;
            }
        }
        if let Some(Menu::Fix { id, .. }) = self.menu.clone() {
            let fx = self.by_id(&id).map(|a| a.fixes()).unwrap_or_default();
            for (i, f) in fx.iter().enumerate() {
                if k == idx(K_FIXM, i) {
                    self.menu = None;
                    self.fix(&id, *f);
                    return;
                }
            }
        }
        if self.busy() {
            return;
        }
        // rows and their parts
        let ids: Vec<String> = self.apps.iter().map(|a| a.id.clone()).collect();
        for id in ids {
            let rk = Self::row_key(&id);
            if k == rk {
                self.click_row(&id);
            } else if k == sub(rk, "ck") {
                self.tick_row(&id);
            } else if k == sub(rk, "del") {
                self.confirm(vec![id]);
            } else if k == sub(rk, "fold") {
                self.open_folder(&id);
            } else if k == sub(rk, "fix") {
                self.wrench(&id, k);
            } else {
                continue;
            }
            return;
        }
    }
}

#[allow(dead_code)]
const _UNUSED: (f32, Rgba) = (RADIUS_PILL, Rgba(0.0, 0.0, 0.0, 0.0));

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::Gfx;
    use crate::ui::cx::State;
    use crate::ui::lay::Laid;

    fn page() -> Apps {
        let mut p = Apps::default();
        p.open(&Env { test: true, frozen: true, ..Env::default() }, 0.0);
        p
    }

    fn id_of(p: &Apps, name: &str) -> String {
        p.apps.iter().find(|a| a.name == name).unwrap().id.clone()
    }

    fn ran(p: &Apps) -> Vec<String> {
        match &p.svc.as_ref().unwrap().os() {
            Os::Fake(f) => f.state().ran.clone(),
            #[cfg(windows)]
            Os::Real(_) => panic!("a test copy uses the fake"),
        }
    }

    #[test]
    fn opens_with_the_drawings_apps_biggest_first() {
        let p = page();
        let v: Vec<&str> = p.view().iter().map(|a| a.name.as_str()).collect();
        assert_eq!(v.len(), 20);
        assert_eq!(&v[..5], &["Call of Duty", "Apex Legends", "VALORANT", "Counter-Strike 2", "Rocket League"]);
        assert_eq!(&v[11..13], &["OBS Studio", "WhatsApp"]);
        assert_eq!(&v[17..], &["Riot Vanguard", "Microsoft Visual C++ 2015-2022 Redistributable (x64)", "Microsoft Edge"], "System and drivers, then Windows, last");
        let a = p.by_id(&id_of(&p, "Call of Duty")).unwrap();
        assert_eq!(size_text(a), "236 GB");
        assert_eq!(a.install_date.unwrap().display(), "2 Sep 2026");
        assert_eq!(size_text(p.by_id(&id_of(&p, "Microsoft Visual C++ 2015-2022 Redistributable (x64)")).unwrap()), "26 MB");
        assert_eq!(size_text(p.by_id(&id_of(&p, "Microsoft Teams")).unwrap()), "532 MB");
        assert_eq!(size_text(p.by_id(&id_of(&p, "7-Zip 24.09")).unwrap()), "6 MB");
        assert!(ran(&p).is_empty(), "opening runs nothing");
    }

    #[test]
    fn the_wrench_slots_match_the_drawing() {
        let p = page();
        let f = |n: &str| p.by_id(&id_of(&p, n)).unwrap().fixes();
        assert_eq!(f("Epic Games Launcher"), vec![Fix::Repair]);
        assert_eq!(f("Google Chrome"), vec![Fix::Modify]);
        assert_eq!(f("Microsoft Visual C++ 2015-2022 Redistributable (x64)"), vec![Fix::Modify, Fix::Repair]);
        assert_eq!(f("Microsoft Teams"), vec![Fix::Repair, Fix::Reset]);
        assert!(f("Steam").is_empty());
        assert!(f("Microsoft Edge").is_empty());
        assert!(!p.by_id(&id_of(&p, "Microsoft Edge")).unwrap().can_uninstall());
        assert!(p.by_id(&id_of(&p, "Microsoft Teams")).unwrap().folder().is_none(), "Store apps: no folder icon");
        assert!(p.by_id(&id_of(&p, "Steam")).unwrap().folder().is_some());
    }

    #[test]
    fn sort_and_search() {
        let mut p = page();
        p.click(idx(K_SORT, 0));
        assert_eq!(p.view()[0].name, "7-Zip 24.09");
        p.click(idx(K_SORT, 0));
        assert_eq!(p.view()[0].name, "Wootility");
        p.click(idx(K_SORT, 2));
        assert_eq!(p.view()[0].name, "Google Chrome", "Installed: newest first (in Programs)");
        p.query = "riot".into();
        let v: Vec<&str> = p.view().iter().map(|a| a.name.as_str()).collect();
        assert_eq!(v, vec!["VALORANT", "Riot Vanguard"]);
        p.query = "zzz".into();
        assert!(p.view().is_empty());
    }

    #[test]
    fn select_tick_and_all_skip_locked() {
        let mut p = page();
        let st = id_of(&p, "Steam");
        p.click(Apps::row_key(&st));
        assert_eq!(p.sel.len(), 1);
        p.click(Apps::row_key(&st));
        assert!(p.sel.is_empty(), "a second click on the only one clears it");
        let edge = id_of(&p, "Microsoft Edge");
        p.click(Apps::row_key(&edge));
        assert!(p.sel.is_empty(), "locked rows can't be picked");
        assert_eq!(p.nudge.as_ref().map(|n| n.0.as_str()), Some(edge.as_str()), "a click on a locked row pulses its lock");
        p.click(K_ALL);
        assert_eq!(p.sel.len(), 19);
        p.click(K_ALL);
        assert!(p.sel.is_empty());
        p.click(sub(Apps::row_key(&st), "ck"));
        p.click(sub(Apps::row_key(&id_of(&p, "Discord")), "ck"));
        assert_eq!(p.sel.len(), 2);
        p.click(idx(K_BAR, 1));
        assert!(p.sel.is_empty());
    }

    #[test]
    fn uninstall_asks_first_then_runs_in_the_fake_only() {
        let mut p = page();
        let dc = id_of(&p, "Discord");
        p.click(sub(Apps::row_key(&dc), "del"));
        assert!(p.dlg.is_some(), "one confirm first");
        assert!(ran(&p).is_empty(), "nothing runs before the confirm");
        p.click(sub(K_DLG, "no"));
        assert!(p.dlg_closing.is_some() && ran(&p).is_empty(), "Cancel fades the confirm out");
        p.click(sub(K_DLG, "go"));
        assert!(ran(&p).is_empty(), "a closing confirm runs nothing");
        p.click(sub(Apps::row_key(&dc), "del"));
        p.click(sub(K_DLG, "go"));
        assert_eq!(ran(&p), vec![r#"run "C:\Program Files\Discord\uninstall.exe""#.to_string()]);
        // the fake's uninstaller removes nothing unless scripted: "not uninstalled"
        assert_eq!(p.toast.as_ref().unwrap().0, "Discord was not uninstalled");
        assert_eq!(p.apps.len(), 20);
    }

    #[test]
    fn uninstall_several_from_the_bar_removes_them() {
        let mut p = page();
        // script the fake's uninstallers to really remove their entries
        for n in ["Discord", "Wootility"] {
            if let Os::Fake(f) = p.svc.as_ref().unwrap().os() {
                f.state().scripts.insert(
                    format!("\"C:\\Program Files\\{n}\\uninstall.exe\""),
                    bu_apps::fake::Script { exit_code: 0, removes_entry: true, prompt_cancelled: false },
                );
            }
        }
        p.click(sub(Apps::row_key(&id_of(&p, "Discord")), "ck"));
        p.click(sub(Apps::row_key(&id_of(&p, "Wootility")), "ck"));
        p.click(idx(K_BAR, 0));
        assert_eq!(p.dlg.as_ref().unwrap().0.len(), 2);
        p.click(sub(K_DLG, "go"));
        assert_eq!(p.toast.as_ref().unwrap().0, "2 apps uninstalled \u{00b7} 614 MB freed");
        assert_eq!(p.apps.len(), 18);
        assert!(p.sel.is_empty());
    }

    #[test]
    fn locked_and_warned_apps_in_the_confirm() {
        let mut p = page();
        let edge = id_of(&p, "Microsoft Edge");
        p.confirm(vec![edge]);
        assert!(p.dlg.is_none(), "a locked app never reaches the confirm");
        let vg = id_of(&p, "Riot Vanguard");
        p.confirm(vec![vg.clone()]);
        let c = p.svc.as_ref().unwrap().confirm(&[p.by_id(&vg).unwrap()]);
        assert_eq!(c.warnings, vec!["Riot Vanguard: VALORANT won't start without it.".to_string()]);
    }

    #[test]
    fn wrench_folder_and_context_menu_go_through_the_fake() {
        let mut p = page();
        let ch = id_of(&p, "Google Chrome");
        p.click(sub(Apps::row_key(&ch), "fix"));
        assert_eq!(ran(&p), vec![r#"setup "C:\Program Files\Google Chrome\setup.exe" --modify"#.to_string()]);
        assert!(p.fixing.is_none(), "the fake's setup ended");
        // while a setup is still open, a second wrench click starts nothing
        p.fixing = Some("Google Chrome".into());
        p.click(sub(Apps::row_key(&ch), "fix"));
        assert_eq!(ran(&p).len(), 1);
        assert_eq!(p.toast.as_ref().unwrap().0, "Google Chrome\u{2019}s setup is still open");
        p.fixing = None;
        let vc = id_of(&p, "Microsoft Visual C++ 2015-2022 Redistributable (x64)");
        p.click(sub(Apps::row_key(&vc), "fix"));
        assert!(matches!(p.menu, Some(Menu::Fix { .. })), "two choices: a small menu");
        p.click(idx(K_FIXM, 1));
        assert!(ran(&p).last().unwrap().ends_with("bundle.exe\" /repair"));
        p.click(sub(Apps::row_key(&id_of(&p, "Steam")), "fold"));
        assert_eq!(ran(&p).last().unwrap(), r"explore C:\Program Files\Steam");
        let tm = id_of(&p, "Microsoft Teams");
        p.context_menu(&tm, 300.0, 200.0);
        p.click(idx(K_MENU, 4));
        assert!(ran(&p).last().unwrap().starts_with("settings ms-settings:appsfeatures-app?"));
        p.context_menu(&tm, 300.0, 200.0);
        p.click(idx(K_MENU, 1));
        assert!(p.dlg.is_some());
        assert!(!ran(&p).iter().any(|r| r.starts_with("run ") || r.starts_with("remove ")), "no uninstall without the confirm");
    }

    #[test]
    fn groups_programs_system_windows() {
        let p = page();
        let c = |n: &str| category(p.by_id(&id_of(&p, n)).unwrap());
        assert_eq!(c("Steam"), Cat::Programs);
        assert_eq!(c("Microsoft Teams"), Cat::Programs, "a Microsoft program is still a program");
        assert_eq!(c("Riot Vanguard"), Cat::System);
        assert_eq!(c("Microsoft Visual C++ 2015-2022 Redistributable (x64)"), Cat::System);
        assert_eq!(c("Microsoft Edge"), Cat::Windows);
        // a Store app of Microsoft's own family came with Windows
        let mut calc = p.by_id(&id_of(&p, "Microsoft Teams")).unwrap().clone();
        calc.name = "Calculator".into();
        calc.source = bu_apps::AppSource::Store { full_name: "Microsoft.WindowsCalculator_11.2_x64__8wekyb3d8bbwe".into(), family_name: "Microsoft.WindowsCalculator_8wekyb3d8bbwe".into() };
        assert_eq!(category(&calc), Cat::Windows);
    }

    #[test]
    fn ctrl_and_shift_clicks_pick_like_the_drawing() {
        let mut p = page();
        let st = id_of(&p, "Steam");
        p.click(Apps::row_key(&st));
        p.click_row_mod(&id_of(&p, "Discord"), true, false);
        assert_eq!(p.sel.len(), 2, "Ctrl adds");
        // Shift: Steam (8) .. OBS Studio (11) in the view's order, without Ctrl the rest goes
        p.anchor = Some(st.clone());
        p.click_row_mod(&id_of(&p, "OBS Studio"), false, true);
        let names: Vec<&str> = p.selected().iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Steam", "Google Chrome", "Microsoft Teams", "OBS Studio"]);
    }

    /// Order 047 (idle cost): with nothing moving the tab asks for no frames - `tick` is false, a build is not busy (a
    /// worker's messages wake the menu themselves, `post`).
    #[test]
    fn nothing_moving_asks_for_no_frames() {
        let mut p = page();
        assert!(!p.tick(10.0), "nothing new: no repaint");
        assert_eq!(p.wake_at(10.0), None);
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(20.0, false, &g, &mut st);
        let _ = p.build(&mut cx);
        drop(cx);
        assert!(!st.busy, "an idle tab asks for no frames");
    }

    #[test]
    fn closing_drops_everything() {
        let mut p = page();
        p.click(K_ALL);
        p.close();
        assert!(p.svc.is_none() && p.apps.is_empty() && p.sel.is_empty() && p.rx.is_none());
    }

    /// The boxes land where Chromium lays out the drawing (tools/ref/dom_dump.js on menu-v22, page apps, 1920 x 1080):
    /// `.gh` (38, 110), the header row (26, 131.84) 548 x 30, the first row (26, 161.84) 548 x 46, its tile (56, 171.84),
    /// the size cell x 320 w 62, the date cell x 390 w 84, the actions x 482 w 84, the trash (542, 172.84).
    #[test]
    fn boxes_match_the_drawing() {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut p = page();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        let kids = p.build(&mut cx);
        let root = El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
        let laid = Laid::new(&g, root, 600.0, None);
        let r = |k: Key| laid.rect_of(k).map(|(x, y, w, h)| (x, y + 56.0, w, h)).unwrap();
        let near = |a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)| (a.0 - b.0).abs() < 0.02 && (a.1 - b.1).abs() < 0.02 && (a.2 - b.2).abs() < 0.02 && (a.3 - b.3).abs() < 0.02;
        let hd = r(sub(K_SORT, "hd"));
        assert!(near(hd, (26.0, 131.8438, 548.0, 30.0)), "header {:?}", hd);
        let cod = Apps::row_key(&id_of(&p, "Call of Duty"));
        let row = r(cod);
        assert!(near(row, (26.0, 161.8438, 548.0, 46.0)), "row {:?}", row);
        let del = r(sub(cod, "del"));
        assert!(near(del, (542.0, 172.8438, 24.0, 24.0)), "trash {:?}", del);
        let nm = r(idx(K_SORT, 0));
        assert!(near(nm, (87.0, 135.8438, 230.0, 22.0)), "name button {:?}", nm);
        let sz = r(idx(K_SORT, 1));
        assert!(near(sz, (315.0, 135.8438, 72.0, 22.0)), "size button {:?}", sz);
        let ins = r(idx(K_SORT, 2));
        assert!(near(ins, (385.0, 135.8438, 94.0, 22.0)), "installed button {:?}", ins);
    }
}

#[cfg(test)]
mod hover_probe {
    use super::*;
    use crate::gfx::Gfx;
    use crate::ui::cx::State;
    use crate::ui::lay::Laid;

    #[test]
    fn builds_with_a_hovered_row() {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut p = Apps::default();
        p.open(&Env { test: true, frozen: true, ..Env::default() }, 0.0);
        let id = p.apps[0].id.clone();
        st.hover = vec![Apps::row_key(&id)];
        let mut cx = Cx::new(10.0, false, &g, &mut st);
        let kids = p.build(&mut cx);
        let root = El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
        let laid = Laid::new(&g, root, 600.0, None);
        assert!(laid.height > 900.0, "height {}", laid.height);
    }
}
