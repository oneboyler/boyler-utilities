//! The Controller tab's logic: games, the page's data, changes written to Steam's files, backups, undo, resets.
//!
//! Rules (order 015 + the Oct 7 / Oct 8 Steam tests):
//! - Every change re-reads the file first (Steam may have saved its own copy since), changes only the lines it needs and
//!   writes the whole file in one replace. Steam re-reads a game's layout when the game window gets focus (measured live
//!   with a real pad: "Loaded Config for Local Selection Path … 252950" on focus; a Cross/Circle swap worked
//!   without a Steam restart). The app never starts, closes or restarts Steam.
//! - Before the FIRST write of a file the app keeps its original bytes (`<backups>\<account>\<path under Steam>.original`,
//!   or a `.absent` marker if the app created the file) — "Back to how your PC was" puts them back.
//! - Every write is one undo step (all files it touched); undo refuses when the file changed since (someone else wrote).
//! - A community / template layout becomes the user's own copy on the first change (Steam does the same): the layout is
//!   copied to `<game>\controller_<type>.vdf` with its `url` set, and `configset_controller_<type>.vdf` points the game at it.

use crate::error::{Error, Result};
use crate::layout::{ActionSet, Layout};
use crate::os::SteamOs;
use crate::parts::{PadKind, Part};
use crate::prefs::{PrefSetting, Prefs};
use crate::settings::{Change, PadView};
use crate::steam::{configset_entry, configset_put_entry, configset_to_autosave, Game, LayoutSource, SteamPaths, DESKTOP_APPID};
use std::path::{Path, PathBuf};

/// One undoable step: every file it changed with its bytes before (None = the file did not exist) and after.
#[derive(Debug, Clone)]
struct Step {
    label: String,
    files: Vec<(PathBuf, Option<Vec<u8>>, Vec<u8>)>,
}

/// The opened layout of one game for one controller type.
#[derive(Debug, Clone)]
pub struct Opened {
    pub game: Game,
    pub kind: PadKind,
    /// The file Steam reads now.
    pub path: PathBuf,
    pub layout: Layout,
}

/// The Controller tab's service over an OS layer.
pub struct ControllerService<O: SteamOs> {
    os: O,
    steam: SteamPaths,
    backups: PathBuf,
    undo: Vec<Step>,
}

impl<O: SteamOs> ControllerService<O> {
    /// Find Steam + the account. `backups` = the app's own folder for original copies (never inside Steam).
    pub fn new(os: O, backups: impl Into<PathBuf>) -> Result<Self> {
        let steam = SteamPaths::find(&os)?;
        Ok(ControllerService { os, steam, backups: backups.into(), undo: Vec::new() })
    }

    pub fn with_paths(os: O, steam: SteamPaths, backups: impl Into<PathBuf>) -> Self {
        ControllerService { os, steam, backups: backups.into(), undo: Vec::new() }
    }

    pub fn os(&self) -> &O {
        &self.os
    }
    pub fn steam(&self) -> &SteamPaths {
        &self.steam
    }
    pub fn steam_running(&self) -> bool {
        self.os.steam_running()
    }

    /// The games with a layout for this controller type (the game popup).
    pub fn games(&self, kind: PadKind) -> Result<Vec<Game>> {
        self.steam.games(&self.os, kind)
    }

    /// One game by its key (app id or shortcut name). The Desktop is refused (Steam keeps it in memory).
    pub fn game(&self, key: &str, kind: PadKind) -> Result<Game> {
        if key == DESKTOP_APPID.to_string() || key.eq_ignore_ascii_case("desktop") {
            return Err(Error::DesktopNotAFile);
        }
        self.games(kind)?.into_iter().find(|g| g.key.eq_ignore_ascii_case(key)).ok_or_else(|| Error::NoLayoutFile(key.to_string()))
    }

    /// For reading only (names, views): odd bytes become U+FFFD.
    fn read_text(&self, path: &Path) -> Result<String> {
        let b = self.os.read(path)?;
        Ok(String::from_utf8_lossy(&b).into_owned())
    }

    /// For a file that may be WRITTEN back: exact UTF-8 or refused (`NotUtf8`), so no byte can ever change by decoding.
    fn read_text_exact(&self, path: &Path) -> Result<String> {
        let b = self.os.read(path)?;
        String::from_utf8(b).map_err(|_| Error::NotUtf8(path.to_path_buf()))
    }

    fn parse_layout(&self, path: &Path, text: String) -> Result<Layout> {
        Layout::parse(text).map_err(|err| Error::Layout { path: path.to_path_buf(), err })
    }

    /// Read the layout Steam uses now for this game.
    pub fn open(&self, key: &str, kind: PadKind) -> Result<Opened> {
        let game = self.game(key, kind)?;
        let path = self.steam.active_file(&self.os, &game, kind).ok_or_else(|| Error::NoLayoutFile(game.name.clone()))?;
        if !self.os.exists(&path) {
            return Err(Error::LayoutMissing(path));
        }
        let layout = self.parse_layout(&path, self.read_text(&path)?)?;
        Ok(Opened { game, kind, path, layout })
    }

    /// Everything the page shows for one game, controller and action set.
    pub fn view(&self, key: &str, kind: PadKind, set: u32) -> Result<PadView> {
        Ok(self.open(key, kind)?.layout.pad_view(set, kind))
    }

    pub fn action_sets(&self, key: &str, kind: PadKind) -> Result<Vec<ActionSet>> {
        Ok(self.open(key, kind)?.layout.action_sets())
    }

    /// Steam's own layout for this game (what "Steam's layout" / "Steam's setting for this" go back to): the layout's
    /// `progenitor` (an official / community workshop layout or a template) — or the active file itself while the game
    /// still uses a community / template layout directly.
    pub fn steam_layout(&self, key: &str, kind: PadKind) -> Result<Layout> {
        self.steam_layout_read(key, kind, false)
    }

    /// `exact` = Steam's text is going to be WRITTEN into the user's file: refuse (`NotUtf8`) instead of decoding lossily.
    fn steam_layout_read(&self, key: &str, kind: PadKind, exact: bool) -> Result<Layout> {
        let o = self.open(key, kind)?;
        if o.game.source != LayoutSource::Autosave {
            return Ok(o.layout);
        }
        let prog = o.layout.header().progenitor;
        let path = self.steam.progenitor_file(&self.os, &prog).ok_or(Error::NoSteamDefault)?;
        let text = if exact { self.read_text_exact(&path)? } else { self.read_text(&path)? };
        self.parse_layout(&path, text)
    }

    /// Parts that differ from Steam's own layout (amber dots); empty when Steam's layout is not on this PC.
    pub fn changed_parts(&self, key: &str, kind: PadKind, set: u32) -> Result<Vec<Part>> {
        let mine = self.view(key, kind, set)?;
        match self.steam_layout(key, kind) {
            Ok(l) => {
                let ss = if l.action_sets().iter().any(|s| s.id == set) { set } else { 0 };
                Ok(mine.changed_parts(&l.pad_view(ss, kind)))
            }
            Err(Error::NoSteamDefault) => Ok(vec![]),
            Err(e) => Err(e),
        }
    }

    // ------------------------------------------------------------------------------------------ writing

    fn backup_path(&self, file: &Path, suffix: &str) -> PathBuf {
        let rel = file.strip_prefix(&self.steam.dir).map(Path::to_path_buf).unwrap_or_else(|_| PathBuf::from(file.file_name().unwrap_or_default()));
        let mut p = self.backups.join(&self.steam.account).join(rel).into_os_string();
        p.push(suffix);
        PathBuf::from(p)
    }

    /// Keep the file's original bytes once (never overwritten later).
    fn keep_original(&self, file: &Path, before: Option<&[u8]>) -> Result<()> {
        let orig = self.backup_path(file, ".original");
        let absent = self.backup_path(file, ".absent");
        if self.os.exists(&orig) || self.os.exists(&absent) {
            return Ok(());
        }
        if let Some(dir) = orig.parent() {
            self.os.create_dir_all(dir)?;
        }
        match before {
            Some(b) => self.os.write(&orig, b),
            None => self.os.write(&absent, b""),
        }
    }

    /// Write a set of files as one undo step (originals kept first; files in the given order).
    fn commit(&mut self, label: &str, files: Vec<(PathBuf, Vec<u8>)>) -> Result<()> {
        let mut step = Step { label: label.to_string(), files: Vec::new() };
        for (path, after) in files {
            let before = if self.os.exists(&path) { Some(self.os.read(&path)?) } else { None };
            if before.as_deref() == Some(after.as_slice()) {
                continue;
            }
            self.keep_original(&path, before.as_deref())?;
            if let Some(dir) = path.parent() {
                if !self.os.exists(dir) {
                    self.os.create_dir_all(dir)?;
                }
            }
            if let Err(e) = self.os.write(&path, &after) {
                // put back what this step already wrote (best effort; the backups stay), then report
                for (p, b, _) in step.files.iter().rev() {
                    let _ = match b {
                        Some(b) => self.os.write(p, b),
                        None => self.os.remove(p),
                    };
                }
                return Err(e);
            }
            step.files.push((path, before, after));
        }
        if !step.files.is_empty() {
            self.undo.push(step);
        }
        Ok(())
    }

    /// Change a game's layout with `f`. Re-reads the file, makes the user's own copy of a community layout if needed,
    /// writes only when something changed.
    pub fn edit<F>(&mut self, key: &str, kind: PadKind, label: &str, f: F) -> Result<()>
    where
        F: FnOnce(&mut Layout) -> crate::layout::LResult<()>,
    {
        let o = self.open(key, kind)?;
        let exact = self.read_text_exact(&o.path)?; // refuse a file that isn't exact UTF-8 before changing anything
        let original = self.parse_layout(&o.path, exact)?;
        let mut layout = original.clone();
        let own_path = self.steam.autosave_path(&o.game.key, kind);
        let mut files = Vec::new();
        let mut configset = None;
        if o.game.source != LayoutSource::Autosave {
            // first change of a community / template layout → the user's own copy (what Steam does on its first save)
            let url = format!("autosave://{}", own_path.display());
            layout.set_header("url", &url).map_err(|err| Error::Layout { path: o.path.clone(), err })?;
            let prog = match &o.game.source {
                LayoutSource::Workshop(id) => Some(format!("workshop://{id}")),
                LayoutSource::Template(f) => Some(format!("template://{f}")),
                _ => None,
            };
            if let Some(p) = prog {
                if layout.header().progenitor.is_empty() {
                    layout.set_header("progenitor", &p).map_err(|err| Error::Layout { path: o.path.clone(), err })?;
                }
            }
            let cs_path = self.steam.configset_path(kind);
            let cs = self.read_text_exact(&cs_path)?;
            let new_cs = configset_to_autosave(&cs, &o.game.key).map_err(|err| Error::Edit { path: cs_path.clone(), err })?;
            // this game's own entry as it was: kept once (below, only when the change goes ahead) so "Back to how your PC
            // was" puts back only this entry
            let entry = configset_entry(&cs, &o.game.key).map_err(|err| Error::Parse { path: cs_path.clone(), err })?;
            configset = Some((cs_path, new_cs.into_bytes(), entry));
        }
        f(&mut layout).map_err(|err| Error::Layout { path: own_path.clone(), err })?;
        if configset.is_none() && layout.text() == original.text() {
            return Ok(()); // nothing changed: not one byte written
        }
        // re-parse what we write (never hand Steam a file we can't read back)
        Layout::parse(layout.text()).map_err(|err| Error::Layout { path: own_path.clone(), err })?;
        files.push((own_path, layout.text().as_bytes().to_vec()));
        if let Some((cs_path, bytes, entry)) = configset {
            self.keep_entry(&cs_path, &o.game.key, entry.as_deref())?; // before the index is written
            files.push((cs_path, bytes)); // after the layout: the index never points at a missing file
        }
        self.commit(&format!("{} · {label}", o.game.name), files)
    }

    /// Apply one change (one undo step).
    pub fn apply(&mut self, key: &str, kind: PadKind, set: u32, change: &Change) -> Result<()> {
        self.apply_all(key, kind, set, std::slice::from_ref(change))
    }

    /// Apply several changes as ONE write and ONE undo step (e.g. a slider drag that settled).
    pub fn apply_all(&mut self, key: &str, kind: PadKind, set: u32, changes: &[Change]) -> Result<()> {
        for c in changes {
            if !c.fits(kind) {
                return Err(Error::NotOnThisPad(match c {
                    Change::GyroMode { .. } | Change::GyroSetting { .. } => "gyro",
                    Change::TouchMode { .. } | Change::TouchClick { .. } | Change::TouchSetting { .. } => "touchpad",
                    _ => "such button",
                }));
            }
        }
        self.edit(key, kind, "change", |l| {
            for c in changes {
                l.apply(set, c)?;
            }
            Ok(())
        })
    }

    /// New action set as a copy of `from` ("New action set…"). Returns its id.
    pub fn add_action_set(&mut self, key: &str, kind: PadKind, from: u32, title: &str) -> Result<u32> {
        let mut id = 0;
        self.edit(key, kind, "new action set", |l| {
            id = l.add_action_set(from, title)?;
            Ok(())
        })?;
        Ok(id)
    }

    pub fn rename_action_set(&mut self, key: &str, kind: PadKind, set: u32, title: &str) -> Result<()> {
        self.edit(key, kind, "rename action set", |l| l.rename_action_set(set, title))
    }

    /// "Steam's setting for this": one part back to Steam's own layout.
    pub fn part_to_steam(&mut self, key: &str, kind: PadKind, set: u32, part: Part) -> Result<()> {
        let steam = self.steam_layout(key, kind)?;
        let ss = if steam.action_sets().iter().any(|s| s.id == set) { set } else { 0 };
        let changes: Vec<Change> = steam.pad_view(ss, kind).part_changes(part).into_iter().filter(|c| c.fits(kind)).collect();
        self.edit(key, kind, "Steam's setting", |l| {
            for c in &changes {
                l.apply(set, c)?;
            }
            Ok(())
        })
    }

    /// "Steam's layout": the whole game back to Steam's own layout (the file becomes Steam's text with the user's `url`).
    pub fn layout_to_steam(&mut self, key: &str, kind: PadKind) -> Result<()> {
        let Some((o, bytes)) = self.steam_target(key, kind)? else {
            return Ok(()); // the game already uses Steam's / the community layout directly
        };
        self.commit(&format!("{} · Steam's layout", o.game.name), vec![(o.path, bytes)])
    }

    /// What "Steam's layout" writes: the opened game + the file's new bytes (Steam's text with the user's `url`); None =
    /// the game uses Steam's / the community layout directly (nothing to write).
    fn steam_target(&self, key: &str, kind: PadKind) -> Result<Option<(Opened, Vec<u8>)>> {
        let o = self.open(key, kind)?;
        if o.game.source != LayoutSource::Autosave {
            return Ok(None);
        }
        let mut steam = self.steam_layout_read(key, kind, true)?;
        let url = o.layout.header().url;
        let prog = o.layout.header().progenitor;
        steam.set_header("url", &url).map_err(|err| Error::Layout { path: o.path.clone(), err })?;
        steam.set_header("progenitor", &prog).map_err(|err| Error::Layout { path: o.path.clone(), err })?;
        let bytes = steam.text().as_bytes().to_vec();
        Ok(Some((o, bytes)))
    }

    /// Is the game's layout Steam's own right now ("Steam's layout" would write nothing)? A game with no Steam layout on
    /// this PC counts as yes (there is nothing to go back to).
    pub fn is_steam_layout(&self, key: &str, kind: PadKind) -> Result<bool> {
        match self.steam_target(key, kind) {
            Ok(Some((o, bytes))) => Ok(self.os.read(&o.path)? == bytes),
            Ok(None) | Err(Error::NoSteamDefault) => Ok(true),
            Err(e) => Err(e),
        }
    }

    /// Are the game's files as they were before the app's first change ("Back to how your PC was" would write nothing)?
    /// The layout file against its backup and this game's index entry against its kept entry; what the app never changed
    /// counts as as-it-was.
    pub fn is_original(&self, key: &str, kind: PadKind) -> Result<bool> {
        let game = self.game(key, kind)?;
        let layout = self.steam.autosave_path(&game.key, kind);
        if let Some(target) = self.original_of(&layout)? {
            let now = if self.os.exists(&layout) { Some(self.os.read(&layout)?) } else { None };
            if now != target {
                return Ok(false);
            }
        }
        let cs_path = self.steam.configset_path(kind);
        if let Some(entry) = self.entry_of(&cs_path, &game.key)? {
            let now = self.read_text_exact(&cs_path)?;
            let cur = configset_entry(&now, &game.key).map_err(|err| Error::Parse { path: cs_path.clone(), err })?;
            if cur != entry {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Is this controller's settings file as it was before the app's first change (or never changed by the app)?
    pub fn preferences_are_original(&self, serial: &str) -> Result<bool> {
        let path = self.steam.config_dir().join(format!("preferences_{serial}.vdf"));
        match self.original_of(&path)? {
            Some(target) => {
                let now = if self.os.exists(&path) { Some(self.os.read(&path)?) } else { None };
                Ok(now == target)
            }
            None => Ok(true),
        }
    }

    /// Did the app ever change this controller's settings file?
    pub fn has_preferences_original(&self, serial: &str) -> bool {
        let path = self.steam.config_dir().join(format!("preferences_{serial}.vdf"));
        self.os.exists(&self.backup_path(&path, ".original")) || self.os.exists(&self.backup_path(&path, ".absent"))
    }

    /// "Back to how your PC was" for one game: the layout (and the index, if the app changed it) as they were before the
    /// app's first change. One undo step itself.
    pub fn back_to_original(&mut self, key: &str, kind: PadKind) -> Result<()> {
        let game = self.game(key, kind)?;
        let label = format!("{} · back to how it was", game.name);
        let layout = self.steam.autosave_path(&game.key, kind);
        let cs_path = self.steam.configset_path(kind);
        let mut plan: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
        // 1. the shared index FIRST (then it no longer points at an own copy that step 2 removes): only THIS game's
        //    entry, and only if the app changed it (an entry backup exists); every other game's entry and anything Steam
        //    changed meanwhile stay as they are now
        if let Some(entry) = self.entry_of(&cs_path, &game.key)? {
            let now = self.read_text_exact(&cs_path)?;
            let new = configset_put_entry(&now, &game.key, entry.as_deref()).map_err(|err| Error::Edit { path: cs_path.clone(), err })?;
            plan.push((cs_path, Some(new.into_bytes())));
        }
        // 2. the layout file from its own backup
        if let Some(target) = self.original_of(&layout)? {
            plan.push((layout, target));
        }
        if plan.is_empty() {
            return Err(Error::NoBackup);
        }
        let mut step = Step { label, files: Vec::new() };
        for (path, target) in plan {
            let now = if self.os.exists(&path) { Some(self.os.read(&path)?) } else { None };
            if now == target {
                continue;
            }
            let r = match &target {
                Some(b) => self.os.write(&path, b),
                None => self.os.remove(&path),
            };
            if let Err(e) = r {
                // put back what this reset already changed (like `commit`), then report: no half reset
                for (p, b, _) in step.files.iter().rev() {
                    let _ = match b {
                        Some(b) => self.os.write(p, b),
                        None => self.os.remove(p),
                    };
                }
                return Err(e);
            }
            step.files.push((path, now, target.unwrap_or_default()));
        }
        if !step.files.is_empty() {
            self.undo.push(step);
        }
        Ok(())
    }

    /// The backup of a file: Some(Some(bytes)) = its original, Some(None) = it did not exist, None = no backup.
    fn original_of(&self, file: &Path) -> Result<Option<Option<Vec<u8>>>> {
        let orig = self.backup_path(file, ".original");
        if self.os.exists(&orig) {
            return Ok(Some(Some(self.os.read(&orig)?)));
        }
        Ok(self.os.exists(&self.backup_path(file, ".absent")).then_some(None))
    }

    /// A lossless file-name part for a game key: the hex of its (lower-case) UTF-8 bytes — two different keys never share
    /// one entry backup (Steam keys shortcuts by their title, which may be any text).
    fn entry_suffix(game_key: &str) -> String {
        let hex: String = game_key.to_lowercase().bytes().map(|b| format!("{b:02x}")).collect();
        format!(".entry-{hex}")
    }

    /// Keep one game's configset entry as it was before the app's first change of it (once, never overwritten).
    fn keep_entry(&self, cs_path: &Path, game_key: &str, entry: Option<&str>) -> Result<()> {
        let sfx = Self::entry_suffix(game_key);
        let (orig, absent) = (self.backup_path(cs_path, &format!("{sfx}.original")), self.backup_path(cs_path, &format!("{sfx}.absent")));
        if self.os.exists(&orig) || self.os.exists(&absent) {
            return Ok(());
        }
        if let Some(dir) = orig.parent() {
            self.os.create_dir_all(dir)?;
        }
        match entry {
            Some(e) => self.os.write(&orig, e.as_bytes()),
            None => self.os.write(&absent, game_key.as_bytes()), // the key itself: checked on the way back
        }
    }

    /// The kept entry: Some(Some(text)) / Some(None) = there was no entry / None = the app never changed it.
    fn entry_of(&self, cs_path: &Path, game_key: &str) -> Result<Option<Option<String>>> {
        let sfx = Self::entry_suffix(game_key);
        let orig = self.backup_path(cs_path, &format!("{sfx}.original"));
        // every kept entry must be THIS game's (its own key inside), else nothing is put back
        if self.os.exists(&orig) {
            let text = self.read_text_exact(&orig)?;
            let own = crate::vdf::Doc::parse(text.as_str()).ok().and_then(|d| d.top().map(|t| t.key.eq_ignore_ascii_case(game_key))).unwrap_or(false);
            if !own {
                return Err(Error::BackupMismatch(orig));
            }
            return Ok(Some(Some(text)));
        }
        let absent = self.backup_path(cs_path, &format!("{sfx}.absent"));
        if self.os.exists(&absent) {
            let key = self.read_text_exact(&absent)?;
            if !key.eq_ignore_ascii_case(game_key) {
                return Err(Error::BackupMismatch(absent));
            }
            return Ok(Some(None));
        }
        Ok(None)
    }

    /// Did the app ever change this game's files (is there something to go back to)?
    pub fn has_original(&self, key: &str, kind: PadKind) -> bool {
        let Ok(game) = self.game(key, kind) else { return false };
        let p = self.steam.autosave_path(&game.key, kind);
        let cs = self.steam.configset_path(kind);
        let sfx = Self::entry_suffix(&game.key);
        self.os.exists(&self.backup_path(&p, ".original"))
            || self.os.exists(&self.backup_path(&p, ".absent"))
            || self.os.exists(&self.backup_path(&cs, &format!("{sfx}.original")))
            || self.os.exists(&self.backup_path(&cs, &format!("{sfx}.absent")))
    }

    fn restore_originals(&mut self, label: &str, files: &[PathBuf]) -> Result<()> {
        let mut step = Step { label: label.to_string(), files: Vec::new() };
        let mut any = false;
        for path in files {
            let orig = self.backup_path(path, ".original");
            let absent = self.backup_path(path, ".absent");
            let target: Option<Vec<u8>> = if self.os.exists(&orig) {
                Some(self.os.read(&orig)?)
            } else if self.os.exists(&absent) {
                None
            } else {
                continue;
            };
            any = true;
            let now = if self.os.exists(path) { Some(self.os.read(path)?) } else { None };
            if now == target {
                continue;
            }
            match &target {
                Some(b) => self.os.write(path, b)?,
                None => self.os.remove(path)?,
            }
            // undo of a restore = write back what was there
            step.files.push((path.clone(), now, target.unwrap_or_default()));
        }
        if !any {
            return Err(Error::NoBackup);
        }
        if !step.files.is_empty() {
            self.undo.push(step);
        }
        Ok(())
    }

    // ------------------------------------------------------------------------------------------ undo

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// What the next undo would undo.
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|s| s.label.as_str())
    }

    /// Undo the last step: every file back to its bytes before (exactly). Refused if a file changed since.
    pub fn undo(&mut self) -> Result<String> {
        let step = self.undo.last().cloned().ok_or(Error::NothingToUndo)?;
        for (path, _, after) in &step.files {
            let now = if self.os.exists(path) { Some(self.os.read(path)?) } else { None };
            let expect_absent = after.is_empty() && now.is_none();
            if now.as_deref() != Some(after.as_slice()) && !expect_absent {
                return Err(Error::ChangedOutside(path.clone()));
            }
        }
        // restore in reverse order (the index first, then the layout)
        for (path, before, _) in step.files.iter().rev() {
            match before {
                Some(b) => self.os.write(path, b)?,
                None => {
                    if self.os.exists(path) {
                        self.os.remove(path)?
                    }
                }
            }
        }
        self.undo.pop();
        Ok(step.label)
    }

    // ------------------------------------------------------------------------------------------ this controller (all games)

    /// Every controller Steam has per-controller settings for.
    pub fn preferences(&self) -> Result<Vec<Prefs>> {
        let mut out = Vec::new();
        for (serial, path) in self.steam.preference_files(&self.os) {
            let text = self.read_text(&path)?;
            out.push(Prefs::parse(&serial, &text).map_err(|err| Error::Parse { path, err })?);
        }
        Ok(out)
    }

    /// Set / remove one per-controller value (one undo step). Steam reads this file when the controller connects
    /// (guessed from its log; whether a running Steam picks it up at once is not measured).
    pub fn set_preference(&mut self, serial: &str, setting: PrefSetting, value: Option<&str>) -> Result<()> {
        self.set_preferences(serial, &[(setting, value)], setting.label())
    }

    /// Several per-controller values in ONE write and ONE undo step (e.g. the light colour's red, green and blue).
    pub fn set_preferences(&mut self, serial: &str, values: &[(PrefSetting, Option<&str>)], label: &str) -> Result<()> {
        let path = self.steam.config_dir().join(format!("preferences_{serial}.vdf"));
        if !self.os.exists(&path) {
            return Err(Error::LayoutMissing(path));
        }
        let text = self.read_text_exact(&path)?;
        let mut new = text.clone();
        for (setting, value) in values {
            new = crate::prefs::set(&new, *setting, *value).map_err(|err| Error::Edit { path: path.clone(), err })?;
        }
        if new == text {
            return Ok(());
        }
        self.commit(&format!("Controller settings · {label}"), vec![(path, new.into_bytes())])
    }

    /// The light bar's colour. Steam keeps it PER CONTROLLER (`color_red/green/blue` in `preferences_<serial>.vdf`;
    /// measured: no layout file has a colour key), so the page's "Light bar" part changes it for every game.
    pub fn light_bar(&self, serial: &str) -> Result<Option<(u8, u8, u8)>> {
        Ok(self.preferences()?.into_iter().find(|p| p.serial == serial).and_then(|p| p.led()))
    }

    pub fn set_light_bar(&mut self, serial: &str, rgb: (u8, u8, u8)) -> Result<()> {
        let (r, g, b) = (rgb.0.to_string(), rgb.1.to_string(), rgb.2.to_string());
        self.set_preferences(
            serial,
            &[(PrefSetting::LedRed, Some(&r)), (PrefSetting::LedGreen, Some(&g)), (PrefSetting::LedBlue, Some(&b))],
            "light bar colour",
        )
    }

    /// "Back to how your PC was" for one controller's settings.
    pub fn preferences_to_original(&mut self, serial: &str) -> Result<()> {
        let path = self.steam.config_dir().join(format!("preferences_{serial}.vdf"));
        self.restore_originals("Controller settings · back to how they were", &[path])
    }

    /// Steam's own controller screen for a game ("Open in Steam"); the app opens it, this crate only builds the link.
    /// Refused while Steam is closed: a `steam://` link would START Steam, and the app never starts Steam.
    pub fn open_in_steam_link(&self, game: &Game) -> Result<String> {
        if !self.os.steam_running() {
            return Err(Error::SteamClosed);
        }
        game.appid.map(|id| format!("steam://controllerconfig/{id}")).ok_or_else(|| Error::NoLayoutFile(game.name.clone()))
    }
}
