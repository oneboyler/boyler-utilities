//! Cursors (DESIGN §3.4 "Cursors"): the current scheme, switching a whole scheme or one role, the size slider, importing
//! .cur / .ani packs, and the "Matches your other cursors" suggestion.
//!
//! How (Windows): the active set is `HKCU\Control Panel\Cursors` — one path per Windows cursor role (17 values), the scheme
//! name in `(Default)`, `Scheme Source` (0 = Windows default, 1 = a user scheme, 2 = a system scheme) and `CursorBaseSize`
//! (pixels). `SystemParametersInfo(SPI_SETCURSORS)` makes Windows reload them; the Win11 stuck-scheme bug is answered by
//! re-pushing each changed role with `SetSystemCursor`. Schemes: user ones in `HKCU\Control Panel\Cursors\Schemes`, system
//! ones in `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Control Panel\Cursors\Schemes` (read only). Windows' own default
//! per role: `HKLM\…\Control Panel\Cursors\Default` (read only). All of this measured on a test PC.
//!
//! Calls made for what DESIGN left unclear (also in the report):
//! - The 7 bubbles map onto Windows roles as Normal = Arrow, Link = Hand, Text = IBeam, Busy = Wait, Working = AppStarting,
//!   Move = SizeAll, Resize = all four of SizeNS / SizeWE / SizeNWSE / SizeNESW. The other 10 Windows roles (Help,
//!   Crosshair, NWPen, No, UpArrow, Pin, Person…) are never changed by a role pick; a whole-scheme switch sets all 17.
//! - Size writes `CursorBaseSize` (px) AND `HKCU\Software\Microsoft\Accessibility\CursorSize` (1–15, what Windows' own
//!   slider shows), then re-writes the current role paths and reloads — so custom sets are kept (DESIGN: method is a guess).

use crate::error::{Error, Result};
use crate::os::{Hive, MouseOs, RegValue};
use crate::service::{Mouse, UndoKey, UndoValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const CURSORS_KEY: &str = r"Control Panel\Cursors";
pub const USER_SCHEMES_KEY: &str = r"Control Panel\Cursors\Schemes";
pub const SYSTEM_SCHEMES_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Control Panel\Cursors\Schemes";
pub const WINDOWS_DEFAULT_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Control Panel\Cursors\Default";
pub const ACCESSIBILITY_KEY: &str = r"Software\Microsoft\Accessibility";

/// Size slider 1–15; px = 32 + (n − 1)·16 (1 = 32 px … 15 = 256 px, like Windows).
pub const SIZE_RANGE: (u32, u32) = (1, 15);

pub fn size_px(n: u32) -> u32 {
    32 + (n.clamp(SIZE_RANGE.0, SIZE_RANGE.1) - 1) * 16
}

/// Slider position for a pixel size Windows reports (nearest; under 32 → 1).
pub fn size_step(px: u32) -> u32 {
    (px.saturating_sub(32) + 8) / 16 + 1
}

/// The 17 Windows cursor roles, in the order of a scheme string (measured from the "Windows Aero" scheme on a test PC).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WinRole {
    Arrow,
    Help,
    AppStarting,
    Wait,
    Crosshair,
    IBeam,
    NWPen,
    No,
    SizeNS,
    SizeWE,
    SizeNWSE,
    SizeNESW,
    SizeAll,
    UpArrow,
    Hand,
    Pin,
    Person,
}

impl WinRole {
    pub const ALL: [WinRole; 17] = [
        WinRole::Arrow,
        WinRole::Help,
        WinRole::AppStarting,
        WinRole::Wait,
        WinRole::Crosshair,
        WinRole::IBeam,
        WinRole::NWPen,
        WinRole::No,
        WinRole::SizeNS,
        WinRole::SizeWE,
        WinRole::SizeNWSE,
        WinRole::SizeNESW,
        WinRole::SizeAll,
        WinRole::UpArrow,
        WinRole::Hand,
        WinRole::Pin,
        WinRole::Person,
    ];

    /// The value name under `HKCU\Control Panel\Cursors`.
    pub fn reg_name(self) -> &'static str {
        match self {
            WinRole::Arrow => "Arrow",
            WinRole::Help => "Help",
            WinRole::AppStarting => "AppStarting",
            WinRole::Wait => "Wait",
            WinRole::Crosshair => "Crosshair",
            WinRole::IBeam => "IBeam",
            WinRole::NWPen => "NWPen",
            WinRole::No => "No",
            WinRole::SizeNS => "SizeNS",
            WinRole::SizeWE => "SizeWE",
            WinRole::SizeNWSE => "SizeNWSE",
            WinRole::SizeNESW => "SizeNESW",
            WinRole::SizeAll => "SizeAll",
            WinRole::UpArrow => "UpArrow",
            WinRole::Hand => "Hand",
            WinRole::Pin => "Pin",
            WinRole::Person => "Person",
        }
    }

    /// The `SetSystemCursor` id (OCR_* / IDC_* values from WinUser.h).
    pub fn ocr_id(self) -> u32 {
        match self {
            WinRole::Arrow => 32512,       // OCR_NORMAL
            WinRole::IBeam => 32513,       // OCR_IBEAM
            WinRole::Wait => 32514,        // OCR_WAIT
            WinRole::Crosshair => 32515,   // OCR_CROSS
            WinRole::UpArrow => 32516,     // OCR_UP
            WinRole::SizeNWSE => 32642,    // OCR_SIZENWSE
            WinRole::SizeNESW => 32643,    // OCR_SIZENESW
            WinRole::SizeWE => 32644,      // OCR_SIZEWE
            WinRole::SizeNS => 32645,      // OCR_SIZENS
            WinRole::SizeAll => 32646,     // OCR_SIZEALL
            WinRole::No => 32648,          // OCR_NO
            WinRole::Hand => 32649,        // OCR_HAND
            WinRole::AppStarting => 32650, // OCR_APPSTARTING
            WinRole::Help => 32651,        // OCR_HELP (IDC_HELP)
            WinRole::NWPen => 32631,       // IDC_PEN
            WinRole::Pin => 32671,         // IDC_PIN
            WinRole::Person => 32672,      // IDC_PERSON
        }
    }

    /// The key a cursor pack's `install.inf` [Strings] section uses for this role (the common Windows pack layout).
    pub fn inf_keys(self) -> &'static [&'static str] {
        match self {
            WinRole::Arrow => &["pointer", "arrow", "normal"],
            WinRole::Help => &["help"],
            WinRole::AppStarting => &["work", "working", "appstarting"],
            WinRole::Wait => &["busy", "wait"],
            WinRole::Crosshair => &["cross", "precision", "crosshair"],
            WinRole::IBeam => &["text", "ibeam", "beam"],
            WinRole::NWPen => &["hand", "handwriting", "pen", "nwpen"],
            WinRole::No => &["unavailable", "unavail", "no"],
            WinRole::SizeNS => &["vert", "sizens"],
            WinRole::SizeWE => &["horz", "sizewe"],
            WinRole::SizeNWSE => &["dgn1", "sizenwse"],
            WinRole::SizeNESW => &["dgn2", "sizenesw"],
            WinRole::SizeAll => &["move", "sizeall"],
            WinRole::UpArrow => &["alternate", "alt", "uparrow", "up"],
            WinRole::Hand => &["link"],
            WinRole::Pin => &["pin", "location"],
            WinRole::Person => &["person"],
        }
    }

    /// File-name words that point at this role in a pack without an install.inf (a guess, checked in that order).
    fn file_words(self) -> &'static [&'static str] {
        match self {
            WinRole::Arrow => &["normal", "pointer", "arrow", "default"],
            WinRole::Help => &["help"],
            WinRole::AppStarting => &["working", "appstarting", "background", "work"],
            WinRole::Wait => &["busy", "wait", "loading"],
            WinRole::Crosshair => &["precision", "crosshair", "cross"],
            WinRole::IBeam => &["text", "ibeam", "beam"],
            WinRole::NWPen => &["handwriting", "pen"],
            WinRole::No => &["unavailable", "unavail", "not allowed", "no"],
            WinRole::SizeNS => &["vertical", "vert", "sizens", "_ns", "ns"],
            WinRole::SizeWE => &["horizontal", "horz", "sizewe", "_ew", "ew", "we"],
            WinRole::SizeNWSE => &["diagonal1", "dgn1", "nwse"],
            WinRole::SizeNESW => &["diagonal2", "dgn2", "nesw"],
            WinRole::SizeAll => &["move", "sizeall"],
            WinRole::UpArrow => &["alternate", "uparrow", "up"],
            WinRole::Hand => &["link", "hand"],
            WinRole::Pin => &["pin", "location"],
            WinRole::Person => &["person"],
        }
    }
}

/// The 7 bubbles on the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Role {
    Normal,
    Link,
    Text,
    Busy,
    Working,
    Move,
    Resize,
}

impl Role {
    pub const ALL: [Role; 7] = [Role::Normal, Role::Link, Role::Text, Role::Busy, Role::Working, Role::Move, Role::Resize];

    pub fn name(self) -> &'static str {
        match self {
            Role::Normal => "Normal",
            Role::Link => "Link",
            Role::Text => "Text",
            Role::Busy => "Busy",
            Role::Working => "Working",
            Role::Move => "Move",
            Role::Resize => "Resize",
        }
    }

    /// The Windows roles one bubble sets (see the module calls).
    pub fn win_roles(self) -> &'static [WinRole] {
        match self {
            Role::Normal => &[WinRole::Arrow],
            Role::Link => &[WinRole::Hand],
            Role::Text => &[WinRole::IBeam],
            Role::Busy => &[WinRole::Wait],
            Role::Working => &[WinRole::AppStarting],
            Role::Move => &[WinRole::SizeAll],
            Role::Resize => &[WinRole::SizeNS, WinRole::SizeWE, WinRole::SizeNWSE, WinRole::SizeNESW],
        }
    }

    /// Busy and Working spin (animated) in the bubbles.
    pub fn spins(self) -> bool {
        matches!(self, Role::Busy | Role::Working)
    }
}

/// Where a role's cursor comes from — the rows of the role picker.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SetId {
    /// "The cursors Windows came with"
    WindowsDefault,
    /// "Frosted glass", made by the app (its files live in `AppDirs::glass`)
    Glass,
    /// an imported pack, by name
    Pack(String),
    /// Order 042: a cursor scheme Windows has installed (Control Panel › Mouse › Pointers: "Windows Black", "Windows
    /// Inverted" ... and the user's saved ones), by name
    Scheme(String),
    /// "Your file" — one file picked for this role only
    Own,
    /// something set elsewhere (another scheme in Control Panel, another tool): the scheme name if known, else "Other"
    Other(String),
}

impl SetId {
    /// The set name the bubble tip shows ("<Role>: <set> · click to change").
    pub fn label(&self) -> String {
        match self {
            SetId::WindowsDefault => "Windows default".into(),
            SetId::Glass => "Glass".into(),
            SetId::Pack(n) => n.clone(),
            SetId::Scheme(n) => n.clone(),
            SetId::Own => "Your file".into(),
            SetId::Other(n) => n.clone(),
        }
    }

    /// A real set the suggestion may point at (never Windows default, never a single own file, never "something else").
    fn suggestible(&self) -> bool {
        matches!(self, SetId::Glass | SetId::Pack(_) | SetId::Scheme(_))
    }
}

/// Everything under `HKCU\Control Panel\Cursors` (+ the accessibility size) exactly as read — undo puts back exactly this.
#[derive(Clone, Debug, PartialEq)]
pub struct CursorSnapshot {
    pub scheme_name: Option<RegValue>,
    pub scheme_source: Option<RegValue>,
    pub roles: Vec<(WinRole, Option<RegValue>)>,
    pub base_size: Option<RegValue>,
    pub access_size: Option<RegValue>,
}

/// One bubble's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleState {
    pub role: Role,
    /// the file of the role's first Windows role, `%vars%` expanded; empty = Windows' built-in cursor
    pub file: String,
    pub set: SetId,
}

/// What the Cursors group shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorsState {
    /// `(Default)` of the cursor key — the scheme name Windows shows ("Windows Black"; empty = none)
    pub scheme: String,
    /// `Scheme Source`: 0 = Windows default, 1 = user scheme, 2 = system scheme
    pub scheme_source: Option<u32>,
    pub roles: Vec<RoleState>,
    /// size slider position 1–15
    pub size: u32,
    /// `CursorBaseSize` in px (32 when missing)
    pub size_px: u32,
}

/// A cursor scheme Windows knows (Control Panel › Mouse › Pointers).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scheme {
    pub name: String,
    /// from HKLM (Windows' own) or HKCU (saved by the user)
    pub system: bool,
    /// 17 paths in `WinRole::ALL` order (empty = Windows' built-in for that role)
    pub paths: Vec<String>,
}

/// Splits a scheme string ("a.cur,b.cur,,…") into the 17 role paths (extra trailing items, like the display-name
/// resource "@main.cpl,-1020" in Windows' own schemes, are ignored; missing ones are empty).
pub fn parse_scheme(s: &str) -> Vec<String> {
    let mut v: Vec<String> = s.split(',').take(17).map(|p| p.trim().to_string()).collect();
    v.resize(17, String::new());
    v
}

/// An imported pack (its manifest `pack.json` in its folder).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pack {
    pub name: String,
    /// file name (inside the pack folder) per Windows role, when the pack has one
    pub roles: BTreeMap<WinRole, String>,
    /// every .cur / .ani file copied
    pub files: Vec<String>,
}

impl Pack {
    /// The pack has a cursor for every Windows role of this bubble.
    pub fn has(&self, role: Role) -> bool {
        role.win_roles().iter().any(|r| self.roles.contains_key(r))
    }
}

/// Checks the first bytes: `.cur` = ICONDIR with type 2, `.ani` = RIFF … ACON.
pub fn is_cursor_bytes(b: &[u8]) -> bool {
    (b.len() >= 6 && b[0] == 0 && b[1] == 0 && b[2] == 2 && b[3] == 0 && (b[4] != 0 || b[5] != 0))
        || (b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"ACON")
}

fn is_cursor_ext(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("cur") || e.eq_ignore_ascii_case("ani")).unwrap_or(false)
}

/// Reads `install.inf` [Strings]: key = "file.cur" → role mapping by the usual key names.
pub fn parse_install_inf(text: &str) -> BTreeMap<WinRole, String> {
    let mut in_strings = false;
    let mut kv: BTreeMap<String, String> = BTreeMap::new();
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_strings = l.eq_ignore_ascii_case("[strings]");
            continue;
        }
        if !in_strings || l.starts_with(';') {
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            kv.insert(k.trim().to_ascii_lowercase(), v.trim().trim_matches('"').to_string());
        }
    }
    let mut out = BTreeMap::new();
    for r in WinRole::ALL {
        if let Some(f) = r.inf_keys().iter().find_map(|k| kv.get(*k)) {
            let f = f.rsplit(['\\', '/']).next().unwrap_or(f).to_string();
            if is_cursor_ext(Path::new(&f)) {
                out.insert(r, f);
            }
        }
    }
    out
}

/// Guesses roles from file names (pack without install.inf). Each file is used for at most one role; earlier roles in
/// `WinRole::ALL` pick first, longer words beat shorter ones (so "busy" does not take "busy_working.ani" from Working).
pub fn guess_roles(files: &[String]) -> BTreeMap<WinRole, String> {
    let mut out = BTreeMap::new();
    let mut used = std::collections::BTreeSet::new();
    // Longest words first across all roles, so a specific word wins over a generic one.
    let mut words: Vec<(WinRole, &str)> = WinRole::ALL.iter().flat_map(|r| r.file_words().iter().map(move |w| (*r, *w))).collect();
    words.sort_by_key(|(_, w)| std::cmp::Reverse(w.len()));
    for (role, w) in words {
        if out.contains_key(&role) {
            continue;
        }
        for f in files {
            if used.contains(f) {
                continue;
            }
            let stem = Path::new(f).file_stem().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
            let hit = if w.len() <= 3 {
                // short words must be a whole token ("no", "up", "ns"…)
                stem.split(|c: char| !c.is_ascii_alphanumeric()).any(|t| t == w.trim_start_matches('_'))
            } else {
                stem.contains(w)
            };
            if hit {
                out.insert(role, f.clone());
                used.insert(f.clone());
                break;
            }
        }
    }
    out
}

/// "Matches your other cursors" (DESIGN): the set last picked for ANOTHER role, else the set most of the other roles use.
/// Never Windows default (nor a single own file / something set elsewhere). The set must have a cursor for `role`.
/// `None` = hide the row. Ties in "most used" go to the most recently picked, then by name.
pub fn suggest_match(role: Role, current: &[RoleState], picks: &[(Role, SetId)], has: &dyn Fn(&SetId, Role) -> bool) -> Option<SetId> {
    if let Some((_, s)) = picks.iter().rev().find(|(r, s)| *r != role && s.suggestible()) {
        if has(s, role) {
            return Some(s.clone());
        }
    }
    let mut counts: BTreeMap<&SetId, usize> = BTreeMap::new();
    for rs in current.iter().filter(|rs| rs.role != role && rs.set.suggestible()) {
        *counts.entry(&rs.set).or_default() += 1;
    }
    let recency = |s: &SetId| picks.iter().rposition(|(_, p)| p == s).map(|i| i as i64).unwrap_or(-1);
    counts
        .into_iter()
        .filter(|(s, _)| has(s, role))
        .max_by(|(a, ca), (b, cb)| ca.cmp(cb).then(recency(a).cmp(&recency(b))).then(b.cmp(a)))
        .map(|(s, _)| s.clone())
}

fn same_path(a: &str, b: &str) -> bool {
    a.replace('/', "\\").eq_ignore_ascii_case(&b.replace('/', "\\"))
}

fn under(path: &str, dir: &Path) -> bool {
    let d = dir.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
    let p = path.replace('/', "\\").to_ascii_lowercase();
    p.starts_with(&(d.trim_end_matches('\\').to_string() + "\\"))
}

fn unique_dir(parent: &Path, name: &str) -> (String, PathBuf) {
    let clean: String = name.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    let clean = clean.trim().trim_end_matches('.').to_string();
    let base = if clean.is_empty() { "Imported".to_string() } else { clean };
    let mut n = base.clone();
    let mut i = 2;
    while parent.join(&n).exists() {
        n = format!("{base} {i}");
        i += 1;
    }
    let p = parent.join(&n);
    (n, p)
}

impl<O: MouseOs> Mouse<O> {
    fn cursor_value(&self, r: WinRole) -> Result<Option<RegValue>> {
        self.os.reg_read(Hive::Hkcu, CURSORS_KEY, r.reg_name())
    }

    /// Everything about the cursors exactly as Windows holds it now.
    pub fn cursor_snapshot(&self) -> Result<CursorSnapshot> {
        let mut roles = Vec::new();
        for r in WinRole::ALL {
            roles.push((r, self.cursor_value(r)?));
        }
        Ok(CursorSnapshot {
            scheme_name: self.os.reg_read(Hive::Hkcu, CURSORS_KEY, "")?,
            scheme_source: self.os.reg_read(Hive::Hkcu, CURSORS_KEY, "Scheme Source")?,
            roles,
            base_size: self.os.reg_read(Hive::Hkcu, CURSORS_KEY, "CursorBaseSize")?,
            access_size: self.os.reg_read(Hive::Hkcu, ACCESSIBILITY_KEY, "CursorSize")?,
        })
    }

    /// Windows' own default file per role (`HKLM\…\Cursors\Default`, expanded; empty = built-in).
    pub fn windows_default_paths(&self) -> Result<BTreeMap<WinRole, String>> {
        let vals = self.os.reg_values(Hive::Hklm, WINDOWS_DEFAULT_KEY)?;
        let mut out = BTreeMap::new();
        for r in WinRole::ALL {
            let v = vals.iter().find(|(n, _)| n.eq_ignore_ascii_case(r.reg_name())).and_then(|(_, v)| v.as_str()).unwrap_or("");
            out.insert(r, self.os.expand_env(v));
        }
        Ok(out)
    }

    /// Which set a file belongs to.
    fn classify(&self, r: WinRole, file: &str, defaults: &BTreeMap<WinRole, String>, packs: &[Pack], scheme: &str) -> SetId {
        if file.is_empty() || defaults.get(&r).map(|d| same_path(d, file)).unwrap_or(false) {
            return SetId::WindowsDefault;
        }
        if under(file, &self.dirs.glass()) {
            return SetId::Glass;
        }
        if under(file, &self.dirs.own_cursors()) {
            return SetId::Own;
        }
        let packs_dir = self.dirs.packs();
        for p in packs {
            if under(file, &packs_dir.join(&p.name)) {
                return SetId::Pack(p.name.clone());
            }
        }
        // a scheme Windows has installed: the one Windows names as current first
        let schemes = self.schemes().unwrap_or_default();
        let i = WinRole::ALL.iter().position(|x| *x == r).unwrap_or(0);
        let in_scheme = |s: &Scheme| s.paths.get(i).is_some_and(|p| !p.is_empty() && same_path(&self.os.expand_env(p), file));
        if let Some(s) = schemes.iter().filter(|s| s.name == scheme).chain(schemes.iter()).find(|s| in_scheme(s)) {
            return SetId::Scheme(s.name.clone());
        }
        SetId::Other(if scheme.is_empty() { "Other".into() } else { scheme.into() })
    }

    /// The Cursors group: scheme, the 7 bubbles, size.
    pub fn cursors(&self) -> Result<CursorsState> {
        let snap = self.cursor_snapshot()?;
        let defaults = self.windows_default_paths()?;
        let packs = self.packs()?;
        let scheme = snap.scheme_name.as_ref().and_then(|v| v.as_str()).unwrap_or("").to_string();
        let file_of = |r: WinRole| -> String {
            snap.roles.iter().find(|(x, _)| *x == r).and_then(|(_, v)| v.as_ref()).and_then(|v| v.as_str()).map(|s| self.os.expand_env(s)).unwrap_or_default()
        };
        let roles = Role::ALL
            .iter()
            .map(|role| {
                let first = role.win_roles()[0];
                let file = file_of(first);
                RoleState { role: *role, set: self.classify(first, &file, &defaults, &packs, &scheme), file }
            })
            .collect();
        let size_px = snap.base_size.as_ref().and_then(|v| v.as_dword()).unwrap_or(32);
        Ok(CursorsState {
            scheme,
            scheme_source: snap.scheme_source.as_ref().and_then(|v| v.as_dword()),
            roles,
            size: size_step(size_px).clamp(SIZE_RANGE.0, SIZE_RANGE.1),
            size_px,
        })
    }

    /// Every scheme Windows knows: the system ones (HKLM) then the user's saved ones (HKCU).
    pub fn schemes(&self) -> Result<Vec<Scheme>> {
        let mut out = Vec::new();
        for (hive, key, system) in [(Hive::Hklm, SYSTEM_SCHEMES_KEY, true), (Hive::Hkcu, USER_SCHEMES_KEY, false)] {
            for (name, v) in self.os.reg_values(hive, key)? {
                if let Some(s) = v.as_str() {
                    out.push(Scheme { name, system, paths: parse_scheme(s) });
                }
            }
        }
        Ok(out)
    }

    /// Order 042: the schemes the role pickers list (Windows' own, then the user's saved ones) with the bubbles each has
    /// a cursor for - all but the ones that are just Windows' default files again ("Windows Default" itself).
    pub fn installed_schemes(&self) -> Result<Vec<(String, Vec<Role>)>> {
        let defaults = self.windows_default_paths()?;
        let mut out: Vec<(String, Vec<Role>)> = Vec::new();
        for s in self.schemes()? {
            let file = |r: &WinRole| {
                let i = WinRole::ALL.iter().position(|x| x == r).unwrap_or(0);
                s.paths.get(i).map(|p| self.os.expand_env(p)).unwrap_or_default()
            };
            let differs = WinRole::ALL.iter().any(|r| {
                let f = file(r);
                !f.is_empty() && !defaults.get(r).is_some_and(|d| same_path(d, &f))
            });
            if !differs || out.iter().any(|(n, _)| *n == s.name) {
                continue;
            }
            let roles: Vec<Role> = Role::ALL.iter().copied().filter(|role| role.win_roles().iter().any(|r| !file(r).is_empty())).collect();
            if !roles.is_empty() {
                out.push((s.name.clone(), roles));
            }
        }
        Ok(out)
    }

    /// The imported packs (each folder under `packs` with a readable `pack.json`).
    pub fn packs(&self) -> Result<Vec<Pack>> {
        let dir = self.dirs.packs();
        let Ok(rd) = std::fs::read_dir(&dir) else { return Ok(vec![]) };
        let mut out = Vec::new();
        for e in rd.flatten() {
            if let Ok(t) = std::fs::read_to_string(e.path().join("pack.json")) {
                if let Ok(p) = serde_json::from_str::<Pack>(&t) {
                    out.push(p);
                }
            }
        }
        out.sort_by_key(|a| a.name.to_lowercase());
        Ok(out)
    }

    /// The Glass set's files (role → file name) in `AppDirs::glass()` — empty until [`Mouse::install_glass`] put them there
    /// (the Mouse tab does that when it opens; see [`crate::glass`]).
    pub fn glass_set(&self) -> BTreeMap<WinRole, String> {
        let mut out = BTreeMap::new();
        let dir = self.dirs.glass();
        for r in WinRole::ALL {
            for ext in ["ani", "cur"] {
                let f = format!("{}.{ext}", r.reg_name().to_ascii_lowercase());
                if dir.join(&f).is_file() {
                    out.insert(r, f);
                    break;
                }
            }
        }
        out
    }

    /// Does this set have a cursor for this bubble?
    pub fn set_has(&self, set: &SetId, role: Role) -> bool {
        match set {
            SetId::WindowsDefault => true,
            SetId::Glass => {
                let g = self.glass_set();
                role.win_roles().iter().any(|r| g.contains_key(r))
            }
            SetId::Pack(n) => self.packs().ok().and_then(|ps| ps.into_iter().find(|p| &p.name == n)).map(|p| p.has(role)).unwrap_or(false),
            SetId::Scheme(_) => role.win_roles().iter().any(|r| self.set_file(set, *r).ok().flatten().is_some()),
            SetId::Own | SetId::Other(_) => false,
        }
    }

    /// The role picker's "Matches your other cursors" row (`None` = hidden).
    pub fn suggestion(&self, role: Role) -> Result<Option<SetId>> {
        let st = self.cursors()?;
        let picks: Vec<(Role, SetId)> = self.cursor_picks.clone();
        Ok(suggest_match(role, &st.roles, &picks, &|s, r| self.set_has(s, r)))
    }

    /// The file a set gives one Windows role (`None` = the set has nothing for it).
    fn set_file(&self, set: &SetId, r: WinRole) -> Result<Option<String>> {
        Ok(match set {
            SetId::WindowsDefault => Some(self.windows_default_paths()?.remove(&r).unwrap_or_default()),
            SetId::Glass => self.glass_set().get(&r).map(|f| self.dirs.glass().join(f).to_string_lossy().into_owned()),
            SetId::Pack(n) => {
                let p = self.packs()?.into_iter().find(|p| &p.name == n).ok_or_else(|| Error::NotFound(format!("cursor pack {n}")))?;
                p.roles.get(&r).map(|f| self.dirs.packs().join(&p.name).join(f).to_string_lossy().into_owned())
            }
            SetId::Scheme(n) => {
                let s = self.schemes()?.into_iter().find(|s| &s.name == n).ok_or_else(|| Error::NotFound(format!("cursor scheme {n}")))?;
                let i = WinRole::ALL.iter().position(|x| *x == r).unwrap_or(0);
                // as the scheme stores it (%SystemRoot% unexpanded): written as REG_EXPAND_SZ, like Windows does
                s.paths.get(i).filter(|p| !p.is_empty()).cloned()
            }
            SetId::Own | SetId::Other(_) => None,
        })
    }

    /// Writes role paths, marks the scheme as the user's own mix, reloads, re-pushes (Win11 stuck-scheme bug).
    fn write_roles(&mut self, files: &[(WinRole, String)]) -> Result<()> {
        let before = self.cursor_snapshot()?;
        for (r, f) in files {
            self.os.reg_write(CURSORS_KEY, r.reg_name(), &RegValue::ExpandSz(f.clone()))?;
        }
        // A mix of sets is no longer the named scheme: Windows shows "(None)" with Scheme Source 0 for a custom mix.
        self.os.reg_write(CURSORS_KEY, "", &RegValue::Sz(String::new()))?;
        self.os.reg_write(CURSORS_KEY, "Scheme Source", &RegValue::Dword(0))?;
        self.finish_cursor_change(files)?;
        self.remember(UndoKey::Cursors, UndoValue::Cursors(Box::new(before)));
        Ok(())
    }

    fn finish_cursor_change(&mut self, files: &[(WinRole, String)]) -> Result<()> {
        self.os.reload_cursors()?;
        for (r, f) in files {
            let f = self.os.expand_env(f);
            if !f.is_empty() {
                self.os.set_system_cursor(&f, r.ocr_id())?;
            }
        }
        Ok(())
    }

    /// Picks a set for one bubble ("single role"). Roles of the bubble the set has no file for keep their cursor.
    pub fn set_role(&mut self, role: Role, set: SetId) -> Result<()> {
        let mut files = Vec::new();
        for r in role.win_roles() {
            if let Some(f) = self.set_file(&set, *r)? {
                files.push((*r, f));
            }
        }
        if files.is_empty() {
            return Err(Error::NotFound(format!("{} has no {} cursor", set.label(), role.name())));
        }
        self.write_roles(&files)?;
        self.cursor_picks.push((role, set));
        Ok(())
    }

    /// "Choose your own file…" for one bubble: the file is checked, copied into the app's own folder, then applied.
    pub fn set_role_file(&mut self, role: Role, file: &Path) -> Result<PathBuf> {
        let bytes = std::fs::read(file).map_err(|e| Error::io(format!("read {}", file.display()), e))?;
        if !is_cursor_ext(file) || !is_cursor_bytes(&bytes) {
            return Err(Error::NotACursor(file.display().to_string()));
        }
        let dir = self.dirs.own_cursors().join(role.name());
        std::fs::create_dir_all(&dir).map_err(|e| Error::io("create own-cursor folder", e))?;
        let dst = dir.join(file.file_name().unwrap_or_default());
        std::fs::write(&dst, &bytes).map_err(|e| Error::io(format!("copy to {}", dst.display()), e))?;
        let s = dst.to_string_lossy().into_owned();
        let files: Vec<(WinRole, String)> = role.win_roles().iter().map(|r| (*r, s.clone())).collect();
        self.write_roles(&files)?;
        self.cursor_picks.push((role, SetId::Own));
        Ok(dst)
    }

    /// Switches the whole scheme (all 17 roles + its name and source), like Control Panel's Pointers tab.
    pub fn set_scheme(&mut self, name: &str) -> Result<()> {
        let s = self.schemes()?.into_iter().find(|s| s.name == name).ok_or_else(|| Error::NotFound(format!("cursor scheme {name}")))?;
        let before = self.cursor_snapshot()?;
        let files: Vec<(WinRole, String)> = WinRole::ALL.iter().copied().zip(s.paths.iter().cloned()).collect();
        for (r, f) in &files {
            self.os.reg_write(CURSORS_KEY, r.reg_name(), &RegValue::ExpandSz(f.clone()))?;
        }
        self.os.reg_write(CURSORS_KEY, "", &RegValue::Sz(s.name.clone()))?;
        self.os.reg_write(CURSORS_KEY, "Scheme Source", &RegValue::Dword(if s.system { 2 } else { 1 }))?;
        self.finish_cursor_change(&files)?;
        self.remember(UndoKey::Cursors, UndoValue::Cursors(Box::new(before)));
        Ok(())
    }

    /// Size slider 1–15: writes CursorBaseSize (px) + Accessibility CursorSize, re-writes the current role paths, reloads.
    pub fn set_cursor_size(&mut self, n: u32) -> Result<()> {
        let (lo, hi) = SIZE_RANGE;
        if !(lo..=hi).contains(&n) {
            return Err(Error::range("cursor size", format!("{n} is outside {lo}–{hi}")));
        }
        let before = self.cursor_snapshot()?;
        self.os.reg_write(CURSORS_KEY, "CursorBaseSize", &RegValue::Dword(size_px(n)))?;
        self.os.reg_write(ACCESSIBILITY_KEY, "CursorSize", &RegValue::Dword(n))?;
        let mut files = Vec::new();
        for (r, v) in &before.roles {
            if let Some(v) = v {
                self.os.reg_write(CURSORS_KEY, r.reg_name(), v)?;
                if let Some(s) = v.as_str() {
                    files.push((*r, s.to_string()));
                }
            }
        }
        self.finish_cursor_change(&files)?;
        self.remember(UndoKey::Cursors, UndoValue::Cursors(Box::new(before)));
        Ok(())
    }

    /// Puts every cursor value back exactly as before the last cursor change made here.
    pub fn undo_cursors(&mut self) -> Result<()> {
        let Some(UndoValue::Cursors(s)) = self.take_undo(&UndoKey::Cursors) else {
            return Err(Error::NothingToUndo("cursors".into()));
        };
        let restore = |os: &mut O, key: &str, name: &str, v: &Option<RegValue>| -> Result<()> {
            // A value that did not exist before is written empty (the crate never deletes values in Windows' key).
            let v = v.clone().unwrap_or(RegValue::ExpandSz(String::new()));
            os.reg_write(key, name, &v)
        };
        for (r, v) in &s.roles {
            restore(&mut self.os, CURSORS_KEY, r.reg_name(), v)?;
        }
        if let Some(v) = &s.scheme_name {
            self.os.reg_write(CURSORS_KEY, "", v)?;
        }
        if let Some(v) = &s.scheme_source {
            self.os.reg_write(CURSORS_KEY, "Scheme Source", v)?;
        }
        if let Some(v) = &s.base_size {
            self.os.reg_write(CURSORS_KEY, "CursorBaseSize", v)?;
        }
        if let Some(v) = &s.access_size {
            self.os.reg_write(ACCESSIBILITY_KEY, "CursorSize", v)?;
        }
        let files: Vec<(WinRole, String)> =
            s.roles.iter().filter_map(|(r, v)| v.as_ref().and_then(|v| v.as_str()).map(|f| (*r, f.to_string()))).collect();
        self.finish_cursor_change(&files)
    }

    // ---- the app's change log (Order 036): the cursors as ONE text value, the size as another, and putting either back
    //      with no earlier state (a fresh process: the uninstaller's undo)

    /// The cursors' look as one text (JSON list): the scheme name, `Scheme Source`, then the 17 role paths in
    /// `WinRole::ALL` order, exactly as written (a missing value = "", a missing source = 0 — what Windows reads for them).
    pub fn cursor_look_text(&self) -> Result<String> {
        let s = self.cursor_snapshot()?;
        let text = |v: &Option<RegValue>| v.as_ref().and_then(|v| v.as_str()).unwrap_or("").to_string();
        let mut list = vec![text(&s.scheme_name), s.scheme_source.as_ref().and_then(|v| v.as_dword()).unwrap_or(0).to_string()];
        list.extend(s.roles.iter().map(|(_, v)| text(v)));
        serde_json::to_string(&list).map_err(|e| Error::io("cursor look", e))
    }

    /// Windows' own cursors as a [`Mouse::cursor_look_text`] (no scheme name, source 0, `HKLM\…\Cursors\Default`'s files) —
    /// what a whole switch to "Windows default" writes.
    pub fn windows_default_look_text(&self) -> Result<String> {
        let d = self.windows_default_paths()?;
        let mut list = vec![String::new(), "0".to_string()];
        list.extend(WinRole::ALL.iter().map(|r| d.get(r).cloned().unwrap_or_default()));
        serde_json::to_string(&list).map_err(|e| Error::io("cursor look", e))
    }

    /// Puts the cursors' look back from a [`Mouse::cursor_look_text`]: the 17 role paths, the scheme name and source,
    /// then Windows reloads them (+ the Win11 re-push). The size is not touched.
    pub fn restore_cursor_look(&mut self, text: &str) -> Result<()> {
        let list: Vec<String> = serde_json::from_str(text).map_err(|_| Error::NotFound("the cursors' earlier look".into()))?;
        if list.len() != 2 + WinRole::ALL.len() {
            return Err(Error::NotFound("the cursors' earlier look".into()));
        }
        let source: u32 = list[1].parse().unwrap_or(0);
        let files: Vec<(WinRole, String)> = WinRole::ALL.iter().copied().zip(list[2..].iter().cloned()).collect();
        for (r, f) in &files {
            self.os.reg_write(CURSORS_KEY, r.reg_name(), &RegValue::ExpandSz(f.clone()))?;
        }
        self.os.reg_write(CURSORS_KEY, "", &RegValue::Sz(list[0].clone()))?;
        self.os.reg_write(CURSORS_KEY, "Scheme Source", &RegValue::Dword(source))?;
        self.finish_cursor_change(&files)
    }

    /// The cursor size as one text: `"<CursorBaseSize px>,<Accessibility CursorSize>"` (missing = 32 / 1, Windows' own).
    pub fn cursor_size_text(&self) -> Result<String> {
        let px = self.os.reg_read(Hive::Hkcu, CURSORS_KEY, "CursorBaseSize")?.and_then(|v| v.as_dword()).unwrap_or(32);
        let n = self.os.reg_read(Hive::Hkcu, ACCESSIBILITY_KEY, "CursorSize")?.and_then(|v| v.as_dword()).unwrap_or(1);
        Ok(format!("{px},{n}"))
    }

    /// Puts the cursor size back from a [`Mouse::cursor_size_text`] (both values), re-writes the current role paths and
    /// reloads (like the size slider).
    pub fn restore_cursor_size(&mut self, text: &str) -> Result<()> {
        let (px, n) = text
            .split_once(',')
            .and_then(|(a, b)| Some((a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?)))
            .ok_or_else(|| Error::NotFound("the earlier cursor size".into()))?;
        let now = self.cursor_snapshot()?;
        self.os.reg_write(CURSORS_KEY, "CursorBaseSize", &RegValue::Dword(px))?;
        self.os.reg_write(ACCESSIBILITY_KEY, "CursorSize", &RegValue::Dword(n))?;
        let mut files = Vec::new();
        for (r, v) in &now.roles {
            if let Some(v) = v {
                self.os.reg_write(CURSORS_KEY, r.reg_name(), v)?;
                if let Some(s) = v.as_str() {
                    files.push((*r, s.to_string()));
                }
            }
        }
        self.finish_cursor_change(&files)
    }

    /// The "re-apply" for the Win11 stuck-scheme bug (also at sign-in): reload + re-push every role as the registry has it.
    pub fn reapply_cursors(&mut self) -> Result<()> {
        let s = self.cursor_snapshot()?;
        let files: Vec<(WinRole, String)> =
            s.roles.iter().filter_map(|(r, v)| v.as_ref().and_then(|v| v.as_str()).map(|f| (*r, f.to_string()))).collect();
        self.finish_cursor_change(&files)
    }

    /// Hovering a bubble / a picker row: the real cursor becomes that one (SetSystemCursor) until `end_preview`.
    pub fn preview_cursor(&mut self, role: Role, set: &SetId) -> Result<()> {
        for r in role.win_roles() {
            if let Some(f) = self.set_file(set, *r)? {
                let f = self.os.expand_env(&f);
                if !f.is_empty() {
                    self.os.set_system_cursor(&f, r.ocr_id())?;
                }
            }
        }
        Ok(())
    }

    /// Ends a hover preview: Windows reloads the cursors from the registry (nothing was written).
    pub fn end_preview(&mut self) -> Result<()> {
        self.os.reload_cursors()
    }

    /// "Import cursors…": .cur / .ani files and/or pack folders. Copies them into a new pack folder of the app and returns
    /// the pack + the toast. The pack is NOT applied (DESIGN). Broken / non-cursor files are skipped and listed.
    pub fn import_cursors(&mut self, picked: &[PathBuf]) -> Result<(Pack, String, Vec<String>)> {
        let mut files: Vec<PathBuf> = Vec::new();
        let mut inf: Option<String> = None;
        let mut name: Option<String> = None;
        for p in picked {
            if p.is_dir() {
                name.get_or_insert_with(|| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                let rd = std::fs::read_dir(p).map_err(|e| Error::io(format!("read {}", p.display()), e))?;
                let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
                entries.sort();
                for f in entries {
                    if is_cursor_ext(&f) {
                        files.push(f);
                    } else if f.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("inf")).unwrap_or(false) {
                        inf = std::fs::read_to_string(&f).ok().or(inf);
                    }
                }
            } else if is_cursor_ext(p) {
                name.get_or_insert_with(|| p.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                files.push(p.clone());
            }
        }
        let mut skipped = Vec::new();
        let mut good: Vec<(String, Vec<u8>)> = Vec::new();
        for f in files {
            let fname = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match std::fs::read(&f) {
                Ok(b) if is_cursor_bytes(&b) => {
                    if !good.iter().any(|(n, _)| n.eq_ignore_ascii_case(&fname)) {
                        good.push((fname, b));
                    }
                }
                _ => skipped.push(fname),
            }
        }
        if good.is_empty() {
            return Err(Error::NotACursor("no .cur / .ani cursor in what was picked".into()));
        }
        let packs_dir = self.dirs.packs();
        std::fs::create_dir_all(&packs_dir).map_err(|e| Error::io("create packs folder", e))?;
        let (pack_name, dir) = unique_dir(&packs_dir, name.as_deref().unwrap_or("Imported"));
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(format!("create {}", dir.display()), e))?;
        for (n, b) in &good {
            std::fs::write(dir.join(n), b).map_err(|e| Error::io(format!("copy {n}"), e))?;
        }
        let names: Vec<String> = good.iter().map(|(n, _)| n.clone()).collect();
        let mut roles = inf.as_deref().map(parse_install_inf).unwrap_or_default();
        roles.retain(|_, f| names.iter().any(|n| n.eq_ignore_ascii_case(f)));
        if roles.is_empty() {
            roles = guess_roles(&names);
        }
        let pack = Pack { name: pack_name.clone(), roles, files: names };
        let json = serde_json::to_string_pretty(&pack).map_err(|e| Error::io("pack.json", e))?;
        std::fs::write(dir.join("pack.json"), json).map_err(|e| Error::io("write pack.json", e))?;
        Ok((pack, format!("Imported {pack_name} · pick it in any cursor's list"), skipped))
    }

    /// × on an imported pack: its roles go back to Windows default, then its folder is deleted (only inside the app's
    /// own packs folder). Returns the roles that went back.
    pub fn delete_pack(&mut self, name: &str) -> Result<Vec<Role>> {
        // a pack name is one plain folder name — never "..", ".", a drive or a path (a pack.json could say anything)
        if !is_plain_folder_name(name) {
            return Err(Error::BadName { name: name.into(), why: "not a plain folder name" });
        }
        let packs = self.packs()?;
        if !packs.iter().any(|p| p.name == name) {
            return Err(Error::NotFound(format!("cursor pack {name}")));
        }
        let st = self.cursors()?;
        let gone = SetId::Pack(name.to_string());
        let hit: Vec<Role> = st.roles.iter().filter(|r| r.set == gone).map(|r| r.role).collect();
        if !hit.is_empty() {
            let defaults = self.windows_default_paths()?;
            let files: Vec<(WinRole, String)> =
                hit.iter().flat_map(|r| r.win_roles().iter().map(|w| (*w, defaults.get(w).cloned().unwrap_or_default()))).collect();
            self.write_roles(&files)?;
        }
        let dir = self.dirs.packs().join(name);
        if dir.parent() == Some(self.dirs.packs().as_path()) {
            std::fs::remove_dir_all(&dir).map_err(|e| Error::io(format!("delete {}", dir.display()), e))?;
        }
        self.cursor_picks.retain(|(_, s)| *s != gone);
        Ok(hit)
    }
}

/// One plain folder name: not empty, not "." / "..", no path separators, drive colons or other characters Windows forbids.
pub fn is_plain_folder_name(name: &str) -> bool {
    // Windows strips trailing dots and spaces ("..." or "x." would name another folder, "..." even the packs folder itself),
    // so a name must not end (or start) with either.
    let t = name.trim();
    t == name
        && !t.is_empty()
        && !t.ends_with('.')
        && !t.chars().any(|c| "<>:\"/\\|?*".contains(c) || c.is_control())
}
