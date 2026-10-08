//! Finding Steam, the account, the user's Steam games and which layout file Steam uses for each (read-only).
//!
//! Paths and formats as Steam writes them (2026-10 reads):
//! - layouts: `<Steam>\steamapps\common\Steam Controller Configs\<account>\config\<appid>\controller_<type>.vdf`
//!   (a non-Steam shortcut uses its lower-case title as the folder name, e.g. `epic games launcher`)
//! - which one is active: `config\configset_controller_<type>.vdf` → per game `"autosave" "1"` (the file above),
//!   `"workshop" "<id>"` (a community / official layout: `steamapps\workshop\content\241100\<id>\*_legacy.bin`, VDF text)
//!   or `"template" "<file>"` (`<Steam>\controller_base\templates\<file>`)
//! - per controller (all games): `config\preferences_<serial>.vdf`
//! - game names: `appmanifest_<appid>.acf` in every library of `steamapps\libraryfolders.vdf`; shortcut names:
//!   `userdata\<account>\config\shortcuts.vdf` (binary KeyValues)
//! - Steam Input on/off per game: `userdata\<account>\config\localconfig.vdf` → `apps\<appid>\UseSteamControllerConfig`
//!   (a Steam setting: read, never written)
//! - the Desktop layout (app 413080) is NOT a file (measured Oct 7: Steam loads `controller_base\empty.vdf` as override).

use crate::error::{Error, Result};
use crate::os::SteamOs;
use crate::parts::PadKind;
use crate::vdf::{Doc, Node};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Steam's app id of the Desktop layout.
pub const DESKTOP_APPID: u32 = 413080;
/// Steam Cloud app of the controller configs (241100) — also the workshop folder of community layouts.
pub const CONFIGS_APPID: u32 = 241100;

/// Where Steam takes a game's layout from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutSource {
    /// The user's own copy (`<game>\controller_<type>.vdf`).
    Autosave,
    /// A workshop (community / official) layout id.
    Workshop(String),
    /// One of Steam's templates (file name).
    Template(String),
    /// Something else Steam wrote (kept, shown, not edited).
    Other(String, String),
}

/// Per-game Steam Input switch (Steam's `UseSteamControllerConfig`; meaning of the numbers: 0 = off is measured on other
/// PCs' files only by name — shown raw, never written).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SteamInputSwitch {
    /// Steam's default for this game (no line).
    Default,
    Value(String),
}

/// One game that has a layout for the chosen controller type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    /// The configset key: the app id, or the shortcut's lower-case title.
    pub key: String,
    pub appid: Option<u32>,
    /// Shown name (appmanifest / shortcut name / the key).
    pub name: String,
    /// A non-Steam game added to Steam.
    pub shortcut: bool,
    /// Installed now (has an appmanifest) — shortcuts count as installed.
    pub installed: bool,
    pub source: LayoutSource,
    pub steam_input: SteamInputSwitch,
}

impl Game {
    pub fn is_community(&self) -> bool {
        matches!(self.source, LayoutSource::Workshop(_) | LayoutSource::Template(_))
    }
}

/// Steam on this PC + the account whose layouts are shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamPaths {
    pub dir: PathBuf,
    pub account: String,
}

fn read_doc(os: &dyn SteamOs, path: &Path) -> Result<Doc> {
    let bytes = os.read(path)?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Doc::parse(text).map_err(|err| Error::Parse { path: path.to_path_buf(), err })
}

impl SteamPaths {
    /// Find Steam and pick the account: the logged-in one if it has controller configs, else the one whose configs were
    /// changed last, else the first.
    pub fn find(os: &dyn SteamOs) -> Result<SteamPaths> {
        let dir = os.steam_dir().ok_or(Error::NoSteam)?;
        if !os.exists(&dir) {
            return Err(Error::NoSteam);
        }
        let accounts = Self::accounts(os, &dir);
        if accounts.is_empty() {
            return Err(Error::NoAccount);
        }
        let account = match os.active_account().map(|a| a.to_string()) {
            Some(a) if accounts.contains(&a) => a,
            _ => accounts[0].clone(),
        };
        Ok(SteamPaths { dir, account })
    }

    /// The accounts with a controller-config folder (numeric folder names).
    pub fn accounts(os: &dyn SteamOs, dir: &Path) -> Vec<String> {
        let base = dir.join("steamapps").join("common").join("Steam Controller Configs");
        let mut v: Vec<String> = os
            .list(&base)
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.is_dir && !e.name.is_empty() && e.name.chars().all(|c| c.is_ascii_digit()))
            .map(|e| e.name)
            .collect();
        v.sort();
        v
    }

    pub fn config_dir(&self) -> PathBuf {
        self.dir.join("steamapps").join("common").join("Steam Controller Configs").join(&self.account).join("config")
    }

    pub fn configset_path(&self, kind: PadKind) -> PathBuf {
        self.config_dir().join(format!("configset_controller_{}.vdf", kind.layout_type()))
    }

    /// The user's own layout file of a game.
    pub fn autosave_path(&self, game_key: &str, kind: PadKind) -> PathBuf {
        self.config_dir().join(game_key).join(format!("controller_{}.vdf", kind.layout_type()))
    }

    pub fn workshop_dir(&self, id: &str) -> PathBuf {
        self.dir.join("steamapps").join("workshop").join("content").join(CONFIGS_APPID.to_string()).join(id)
    }

    /// The layout file inside a workshop item folder (`<number>_legacy.bin`, VDF text; the first `.bin` / `.vdf`).
    pub fn workshop_file(&self, os: &dyn SteamOs, id: &str) -> Option<PathBuf> {
        let dir = self.workshop_dir(id);
        let mut files: Vec<String> = os.list(&dir).ok()?.into_iter().filter(|e| !e.is_dir).map(|e| e.name).collect();
        files.sort();
        files.into_iter().find(|n| n.ends_with(".bin") || n.ends_with(".vdf")).map(|n| dir.join(n))
    }

    pub fn template_path(&self, file: &str) -> PathBuf {
        self.dir.join("controller_base").join("templates").join(file)
    }

    /// The file Steam reads for this game now (None = Steam keeps it elsewhere).
    pub fn active_file(&self, os: &dyn SteamOs, game: &Game, kind: PadKind) -> Option<PathBuf> {
        match &game.source {
            LayoutSource::Autosave => Some(self.autosave_path(&game.key, kind)),
            LayoutSource::Workshop(id) => self.workshop_file(os, id),
            LayoutSource::Template(f) => Some(self.template_path(f)),
            LayoutSource::Other(..) => None,
        }
    }

    /// Steam's own layout behind a `progenitor` line (`workshop://<id>` / `template://<file>` / a bare template name).
    pub fn progenitor_file(&self, os: &dyn SteamOs, progenitor: &str) -> Option<PathBuf> {
        if let Some(id) = progenitor.strip_prefix("workshop://") {
            return self.workshop_file(os, id.trim());
        }
        if let Some(f) = progenitor.strip_prefix("template://") {
            let p = self.template_path(f.trim());
            return os.exists(&p).then_some(p);
        }
        None
    }

    /// The configset entries for this controller type: (game key, source) in file order.
    pub fn configset(&self, os: &dyn SteamOs, kind: PadKind) -> Result<Vec<(String, LayoutSource)>> {
        let path = self.configset_path(kind);
        if !os.exists(&path) {
            return Ok(vec![]);
        }
        let doc = read_doc(os, &path)?;
        let Some(top) = doc.top() else { return Ok(vec![]) };
        Ok(top
            .children()
            .iter()
            .filter(|n| n.is_block())
            .map(|n| {
                let src = if n.child_value("autosave").is_some() {
                    LayoutSource::Autosave
                } else if let Some(w) = n.child_value("workshop") {
                    LayoutSource::Workshop(w.to_string())
                } else if let Some(t) = n.child_value("template") {
                    LayoutSource::Template(t.to_string())
                } else {
                    let first = n.children().first();
                    LayoutSource::Other(first.map(|c| c.key.clone()).unwrap_or_default(), first.and_then(|c| c.value()).unwrap_or("").to_string())
                };
                (n.key.clone(), src)
            })
            .collect())
    }

    /// Steam library folders (the install folder first).
    pub fn libraries(&self, os: &dyn SteamOs) -> Vec<PathBuf> {
        let mut out = vec![self.dir.clone()];
        if let Ok(doc) = read_doc(os, &self.dir.join("steamapps").join("libraryfolders.vdf")) {
            if let Some(top) = doc.top() {
                for lib in top.children().iter().filter(|n| n.is_block()) {
                    if let Some(p) = lib.child_value("path") {
                        let p = PathBuf::from(p);
                        if !out.iter().any(|o| same_path(o, &p)) {
                            out.push(p);
                        }
                    }
                }
            }
        }
        out
    }

    /// App id → name for every installed Steam game.
    pub fn app_names(&self, os: &dyn SteamOs) -> BTreeMap<u32, String> {
        let mut out = BTreeMap::new();
        for lib in self.libraries(os) {
            let apps = lib.join("steamapps");
            for e in os.list(&apps).unwrap_or_default() {
                let Some(id) = e.name.strip_prefix("appmanifest_").and_then(|r| r.strip_suffix(".acf")).and_then(|s| s.parse::<u32>().ok()) else { continue };
                if let Ok(doc) = read_doc(os, &apps.join(&e.name)) {
                    if let Some(name) = doc.top().and_then(|t| t.child_value("name")) {
                        out.insert(id, name.to_string());
                    }
                }
            }
        }
        out
    }

    /// Shortcut names (non-Steam games added to Steam) from the binary `shortcuts.vdf`.
    pub fn shortcut_names(&self, os: &dyn SteamOs) -> Vec<String> {
        let path = self.dir.join("userdata").join(&self.account).join("config").join("shortcuts.vdf");
        match os.read(&path) {
            Ok(b) => crate::binvdf::app_names(&b),
            Err(_) => vec![],
        }
    }

    /// Steam Input switch per app id (localconfig.vdf), read-only.
    pub fn steam_input_switches(&self, os: &dyn SteamOs) -> BTreeMap<String, String> {
        let path = self.dir.join("userdata").join(&self.account).join("config").join("localconfig.vdf");
        let mut out = BTreeMap::new();
        if let Ok(doc) = read_doc(os, &path) {
            if let Some(apps) = doc.top().and_then(|t| find_path(t, &["apps"]).or_else(|| find_path(t, &["Software", "Valve", "Steam", "apps"]))) {
                for a in apps.children() {
                    if let Some(v) = a.child_value("UseSteamControllerConfig") {
                        out.insert(a.key.clone(), v.to_string());
                    }
                }
            }
        }
        out
    }

    /// Every game with a layout for this controller type (the game popup's list).
    pub fn games(&self, os: &dyn SteamOs, kind: PadKind) -> Result<Vec<Game>> {
        let names = self.app_names(os);
        let shortcuts = self.shortcut_names(os);
        let switches = self.steam_input_switches(os);
        let mut out = Vec::new();
        for (key, source) in self.configset(os, kind)? {
            let appid = key.parse::<u32>().ok();
            if appid == Some(DESKTOP_APPID) {
                continue; // Desktop: not a file (shown greyed by the page)
            }
            let (name, shortcut, installed) = match appid {
                Some(id) => match names.get(&id) {
                    Some(n) => (n.clone(), false, true),
                    None => (format!("Steam app {id}"), false, false),
                },
                None => {
                    let n = shortcuts.iter().find(|s| s.eq_ignore_ascii_case(&key)).cloned().unwrap_or_else(|| title_case(&key));
                    (n, true, true)
                }
            };
            let steam_input = match switches.get(&key) {
                Some(v) => SteamInputSwitch::Value(v.clone()),
                None => SteamInputSwitch::Default,
            };
            out.push(Game { key, appid, name, shortcut, installed, source, steam_input });
        }
        Ok(out)
    }

    /// Every `preferences_<serial>.vdf` (serial, path).
    pub fn preference_files(&self, os: &dyn SteamOs) -> Vec<(String, PathBuf)> {
        let dir = self.config_dir();
        let mut v: Vec<(String, PathBuf)> = os
            .list(&dir)
            .unwrap_or_default()
            .into_iter()
            .filter(|e| !e.is_dir)
            .filter_map(|e| e.name.strip_prefix("preferences_").and_then(|r| r.strip_suffix(".vdf")).map(|s| (s.to_string(), dir.join(&e.name))))
            .collect();
        v.sort();
        v
    }
}

fn find_path<'a>(n: &'a Node, path: &[&str]) -> Option<&'a Node> {
    let mut cur = n;
    for p in path {
        cur = cur.child(p)?;
    }
    Some(cur)
}

fn same_path(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase();
    n(a) == n(b)
}

fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The configset text with one game's entry switched to the user's own copy (`"autosave" "1"`), every other byte kept.
pub fn configset_to_autosave(text: &str, game_key: &str) -> std::result::Result<String, crate::vdf::EditError> {
    let mut doc = Doc::parse(text)?;
    let g = match doc.find(&[0], game_key) {
        Some(g) => g,
        None => {
            doc.insert_block(&[0], game_key)?;
            doc.find_last(&[0], game_key).ok_or(crate::vdf::EditError::NoSuchNode)?
        }
    };
    let node = doc.get(&g).cloned().ok_or(crate::vdf::EditError::NoSuchNode)?;
    match node.children().first() {
        Some(first) if first.key.eq_ignore_ascii_case("autosave") && first.value() == Some("1") => {}
        Some(first) if first.value().is_some() => {
            let mut a = g.clone();
            a.push(0);
            let _ = first;
            doc.set_key(&a, "autosave")?;
            doc.set_value(&a, "1")?;
            // any further lines of that entry go (Steam's autosave entries hold just the one line)
            while doc.get(&g).map(|n| n.children().len()).unwrap_or(0) > 1 {
                let mut l = g.clone();
                l.push(1);
                doc.remove(&l)?;
            }
        }
        _ => doc.insert_value(&g, "autosave", "1")?,
    }
    Ok(doc.into_text())
}

/// One game's whole entry in a configset text (its lines, with their line endings), `None` = no entry.
pub fn configset_entry(text: &str, game_key: &str) -> std::result::Result<Option<String>, crate::vdf::ParseError> {
    let doc = Doc::parse(text)?;
    let Some(top) = doc.top() else { return Ok(None) };
    Ok(top.children().iter().find(|n| n.key.eq_ignore_ascii_case(game_key)).map(|n| {
        let eol = text[n.end..].find('\n').map(|i| n.end + i + 1).unwrap_or(text.len());
        text[n.line_start..eol].to_string()
    }))
}

/// The configset text with ONE game's entry replaced by `entry` (`None` = removed); every other byte kept.
pub fn configset_put_entry(text: &str, game_key: &str, entry: Option<&str>) -> std::result::Result<String, crate::vdf::EditError> {
    let doc = Doc::parse(text)?;
    let top = doc.top().ok_or(crate::vdf::EditError::NoSuchNode)?;
    let (at, to) = match top.children().iter().find(|n| n.key.eq_ignore_ascii_case(game_key)) {
        Some(n) => (n.line_start, text[n.end..].find('\n').map(|i| n.end + i + 1).unwrap_or(text.len())),
        None => {
            let crate::vdf::Kind::Block { close, .. } = &top.kind else { return Err(crate::vdf::EditError::NotABlock) };
            let ls = text[..*close].rfind('\n').map(|i| i + 1).unwrap_or(0);
            if !text[ls..*close].chars().all(|c| c == ' ' || c == '\t') {
                return Err(crate::vdf::EditError::OddLayout);
            }
            (ls, ls)
        }
    };
    let out = format!("{}{}{}", &text[..at], entry.unwrap_or(""), &text[to..]);
    Doc::parse(out.as_str())?;
    Ok(out)
}
