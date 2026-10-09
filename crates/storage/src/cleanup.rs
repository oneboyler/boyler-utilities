//! Clean up, sizes first (DESIGN §3.12 item 3): Recycle bin · Temp files · Shader caches · Launcher caches.
//!
//! [`measure`] reads every size (read-only). Only a [`CleanPlan`] — the result of measuring — can clean, and it
//! cleans only the rows it is given (the ticked ones). Files in use stay and are counted. A launcher cache is skipped
//! while that launcher runs ("Close Steam first"). Cleaning can't be undone (the API says so: [`CleanKind::undoable`]).
//!
//! Safety: the cleaner only ever deletes *inside* its own target folders (never the folder itself), never follows a
//! junction / symlink, and refuses a target that is shallow or not a known cache name ([`is_safe_target`]).

use crate::{KnownDirs, Result, StorageError, StorageOs};
use std::io;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CleanKind {
    RecycleBin,
    TempFiles,
    ShaderCaches,
    LauncherCaches,
}

impl CleanKind {
    pub const ALL: [CleanKind; 4] =
        [CleanKind::RecycleBin, CleanKind::TempFiles, CleanKind::ShaderCaches, CleanKind::LauncherCaches];

    pub fn name(self) -> &'static str {
        match self {
            CleanKind::RecycleBin => "Recycle bin",
            CleanKind::TempFiles => "Temp files",
            CleanKind::ShaderCaches => "Shader caches",
            CleanKind::LauncherCaches => "Launcher caches",
        }
    }
    /// The detail line under the name.
    pub fn detail(self) -> &'static str {
        match self {
            CleanKind::RecycleBin => "Everything in the recycle bin, all drives",
            CleanKind::TempFiles => "Leftovers apps put in the temp folders",
            CleanKind::ShaderCaches => "DirectX / NVIDIA / AMD · games recompile them (first-run stutter)",
            CleanKind::LauncherCaches => "Steam, Epic, Riot, Battle.net · web caches only, you stay logged in",
        }
    }
    /// Ticked on its own (Order 069, the owner: game and launcher caches "should be unticked anyway by default" - clearing them
    /// makes games recompile shaders and launchers reload their caches).
    pub fn ticked_by_default(self) -> bool {
        matches!(self, CleanKind::RecycleBin | CleanKind::TempFiles)
    }
    /// No row can be undone: deleted files are gone. (The recycle bin's ⓘ tip says so in the design.)
    pub fn undoable(self) -> bool {
        false
    }
}

/// A launcher whose cache must not be cleaned while it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Launcher {
    pub name: &'static str,
    /// Lower-case exe names that mean "it is running".
    pub exes: &'static [&'static str],
}

pub const STEAM: Launcher = Launcher { name: "Steam", exes: &["steam.exe", "steamwebhelper.exe"] };
pub const EPIC: Launcher = Launcher { name: "Epic Games", exes: &["epicgameslauncher.exe", "epicwebhelper.exe"] };
pub const RIOT: Launcher =
    Launcher { name: "Riot Client", exes: &["riotclientservices.exe", "riot client.exe", "riotclientux.exe"] };
/// Only `Battle.net.exe`: its updater is called `Agent.exe`, a name too generic to trust (could be any app).
pub const BATTLE_NET: Launcher = Launcher { name: "Battle.net", exes: &["battle.net.exe"] };

/// One folder a row cleans.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub kind: CleanKind,
    /// "NVIDIA DirectX cache", "Steam app cache" …
    pub name: String,
    pub path: PathBuf,
    pub launcher: Option<Launcher>,
    /// Listing / cleaning it needs admin (C:\Windows\Temp).
    pub needs_admin: bool,
}

/// Every folder target (the recycle bin is not a folder; it is measured through the shell).
///
/// Paths (sources): temp = `%TEMP%` + `C:\Windows\Temp` (DESIGN). Shader caches = `%LocalAppData%\D3DSCache`,
/// `NVIDIA\DXCache`, `NVIDIA\GLCache`, `AMD\DxCache`, `AMD\DxcCache`, `AMD\GLCache` (DESIGN) + `LocalLow\NVIDIA\
/// PerDriverVersion\DXCache` / `GLCache` (where current NVIDIA drivers put it; seen on a real PC) + `AMD\VkCache`
/// (AMD's Vulkan shader cache; seen on a real PC). Steam's `steamapps\shadercache` is left out (DESIGN: unclear —
/// big, re-downloaded). Launchers: Steam `appcache` + the pure cache folders inside `%LocalAppData%\Steam\htmlcache`, Epic
/// the same inside `%LocalAppData%\EpicGamesLauncher\Saved\webcache*` (`webcache_4430` seen on a real PC). Order 069: those
/// two are Chromium profiles that also hold the launcher's web login (Cookies, Local Storage, Session Storage, IndexedDB,
/// Login Data), so only [`WEB_CACHE_SUBS`] go, never the profile itself. Riot
/// `%LocalAppData%\Riot Games\Riot Client\HttpCache` (seen on a real PC — no Riot document), Battle.net
/// `%ProgramData%\Blizzard Entertainment\Battle.net\Cache` (Blizzard's old cache-delete guidance; not installed on
/// a real PC, so unverified; no test fixture for it either).
pub fn targets(os: &dyn StorageOs) -> Vec<Target> {
    let k = os.known_dirs();
    let mut out = Vec::new();
    let mut add = |kind, name: &str, path: Option<PathBuf>, launcher, needs_admin| {
        if let Some(path) = path {
            out.push(Target { kind, name: name.to_string(), path, launcher, needs_admin });
        }
    };
    let local = |sub: &str| k.local_appdata.as_ref().map(|p| p.join(sub));
    let low = |sub: &str| k.local_low.as_ref().map(|p| p.join(sub));
    add(CleanKind::TempFiles, "Your temp folder", k.user_temp.clone(), None, false);
    add(CleanKind::TempFiles, "Windows temp folder", k.windows_temp.clone(), None, true);
    add(CleanKind::ShaderCaches, "DirectX shader cache", local("D3DSCache"), None, false);
    add(CleanKind::ShaderCaches, "NVIDIA DirectX cache", local("NVIDIA\\DXCache"), None, false);
    add(CleanKind::ShaderCaches, "NVIDIA OpenGL cache", local("NVIDIA\\GLCache"), None, false);
    add(CleanKind::ShaderCaches, "NVIDIA DirectX cache (new)", low("NVIDIA\\PerDriverVersion\\DXCache"), None, false);
    add(CleanKind::ShaderCaches, "NVIDIA OpenGL cache (new)", low("NVIDIA\\PerDriverVersion\\GLCache"), None, false);
    add(CleanKind::ShaderCaches, "AMD DirectX cache", local("AMD\\DxCache"), None, false);
    add(CleanKind::ShaderCaches, "AMD DirectX 12 cache", local("AMD\\DxcCache"), None, false);
    add(CleanKind::ShaderCaches, "AMD OpenGL cache", local("AMD\\GLCache"), None, false);
    add(CleanKind::ShaderCaches, "AMD Vulkan cache", local("AMD\\VkCache"), None, false);
    add(CleanKind::LauncherCaches, "Steam app cache", k.steam.as_ref().map(|s| s.join("appcache")), Some(STEAM), false);
    if let Some(html) = local("Steam\\htmlcache") {
        for dir in web_cache_dirs(&html) {
            add(CleanKind::LauncherCaches, "Steam web cache", Some(dir), Some(STEAM), false);
        }
    }
    if let Some(saved) = local("EpicGamesLauncher\\Saved") {
        let mut webcaches: Vec<String> = os
            .read_dir(&saved)
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.is_dir && !e.is_reparse && e.name.to_ascii_lowercase().starts_with("webcache"))
            .map(|e| e.name)
            .collect();
        webcaches.sort();
        for w in webcaches {
            for dir in web_cache_dirs(&saved.join(w)) {
                add(CleanKind::LauncherCaches, "Epic web cache", Some(dir), Some(EPIC), false);
            }
        }
    }
    add(CleanKind::LauncherCaches, "Riot Client cache", local("Riot Games\\Riot Client\\HttpCache"), Some(RIOT), false);
    add(
        CleanKind::LauncherCaches,
        "Battle.net cache",
        k.program_data.as_ref().map(|p| p.join("Blizzard Entertainment\\Battle.net\\Cache")),
        Some(BATTLE_NET),
        false,
    );
    out
}

/// The folders inside a launcher's Chromium web profile (Steam `htmlcache`, Epic `webcache*`) that are pure caches. The
/// profile's other folders and files are the launcher's web login and settings (Cookies, Network\Cookies, Local Storage,
/// Session Storage, IndexedDB, Login Data …) and are never targets (Order 069, the owner: "do launcher cache clean logins?").
const WEB_CACHE_SUBS: &[&str] = &["Cache", "Code Cache", "GPUCache", "Service Worker\\CacheStorage"];

/// The pure cache folders of one Chromium profile root: Steam keeps its profile in `Default`, Epic in the root itself.
fn web_cache_dirs(root: &Path) -> Vec<PathBuf> {
    [root.to_path_buf(), root.join("Default")].iter().flat_map(|p| WEB_CACHE_SUBS.iter().map(move |s| p.join(s))).collect()
}

/// The last folder names the cleaner accepts as a target.
const SAFE_LEAVES: &[&str] = &["temp", "d3dscache", "dxcache", "glcache", "dxccache", "vkcache", "appcache", "httpcache", "cache"];
/// Accepted only inside a launcher's web profile (`htmlcache` / `webcache*` somewhere above them).
const WEB_LEAVES: &[&str] = &["code cache", "gpucache", "cachestorage"];

/// A target folder is safe when it is absolute, at least two folders deep (`C:\Windows\Temp`), ends in a known cache
/// name, and is not one of the big known folders itself.
pub fn is_safe_target(path: &Path, known: &KnownDirs) -> bool {
    if !path.is_absolute() {
        return false;
    }
    let normal = path.components().filter(|c| matches!(c, Component::Normal(_))).count();
    if normal < 2 {
        return false;
    }
    let leaf = match path.file_name() {
        Some(l) => l.to_string_lossy().to_lowercase(),
        None => return false,
    };
    // inside a launcher's web profile (the profile itself is never a target)
    let in_web_profile = path.ancestors().skip(1).filter_map(|a| a.file_name()).any(|n| {
        let n = n.to_string_lossy().to_lowercase();
        n == "htmlcache" || n.starts_with("webcache")
    });
    if !(SAFE_LEAVES.contains(&leaf.as_str()) || (WEB_LEAVES.contains(&leaf.as_str()) && in_web_profile)) {
        return false;
    }
    let same = |a: &Path, b: &Path| a.to_string_lossy().trim_end_matches('\\').eq_ignore_ascii_case(b.to_string_lossy().trim_end_matches('\\'));
    let big = [&known.local_appdata, &known.local_low, &known.program_data, &known.windows, &known.steam];
    !big.iter().filter_map(|p| p.as_ref()).any(|p| same(p, path)) && !known.program_files.iter().any(|p| same(p, path))
}

/// Why a part of a row will not be cleaned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartState {
    /// Will be cleaned when its row is ticked.
    Ready,
    /// The folder doesn't exist (that launcher / GPU isn't installed) — 0 B.
    Missing,
    /// Needs admin to read / clean (C:\Windows\Temp without admin).
    NeedsAdmin,
    /// "Close Steam first".
    LauncherRunning(&'static str),
    /// The cleaner refused the path (see [`is_safe_target`]).
    Refused,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PartSize {
    pub name: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub files: u64,
    pub state: PartState,
    /// The launcher that must be closed first (launcher caches).
    pub launcher: Option<Launcher>,
}

/// One measured row.
#[derive(Debug, Clone, PartialEq)]
pub struct CleanRow {
    pub kind: CleanKind,
    /// The size on the right: what Clean would remove (Ready parts only).
    pub bytes: u64,
    /// Files / items behind `bytes`.
    pub items: u64,
    /// Bytes measured in parts that can't be cleaned now (launcher running, needs admin).
    pub blocked_bytes: u64,
    /// The folders behind the row (empty for the recycle bin).
    pub parts: Vec<PartSize>,
}

impl CleanRow {
    /// A 0 B row is greyed (DESIGN).
    pub fn is_empty(&self) -> bool {
        self.bytes == 0
    }
    /// Plain words for blocked parts: "Close Steam first", "Windows temp needs admin".
    pub fn notes(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in &self.parts {
            let note = match &p.state {
                PartState::LauncherRunning(l) => format!("Close {l} first"),
                PartState::NeedsAdmin => format!("{} needs admin", p.name),
                PartState::Refused => format!("{} skipped (unsafe path)", p.name),
                _ => continue,
            };
            if !out.contains(&note) {
                out.push(note);
            }
        }
        out
    }
}

/// The measured sizes. Only this can clean.
#[derive(Debug, Clone, PartialEq)]
pub struct CleanPlan {
    pub rows: Vec<CleanRow>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CleanedRow {
    pub kind: Option<CleanKind>,
    pub freed_bytes: u64,
    pub freed_files: u64,
    /// "✓ 205 MB in use" — files Windows would not let go.
    pub in_use_bytes: u64,
    pub in_use_files: u64,
    /// Parts not touched, with the reason.
    pub skipped: Vec<(String, PartState)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CleanReport {
    pub rows: Vec<CleanedRow>,
}

impl CleanReport {
    pub fn freed_bytes(&self) -> u64 {
        self.rows.iter().map(|r| r.freed_bytes).sum()
    }
    pub fn in_use_bytes(&self) -> u64 {
        self.rows.iter().map(|r| r.in_use_bytes).sum()
    }
}

/// Measure every row (read-only). Launcher state is read once here and again at clean time.
pub fn measure(os: &dyn StorageOs) -> Result<CleanPlan> {
    let known = os.known_dirs();
    let running = os.running_exe_names();
    let elevated = os.is_elevated();
    let rb = os.recycle_bin()?;
    let mut rows = vec![CleanRow { kind: CleanKind::RecycleBin, bytes: rb.bytes, items: rb.items, blocked_bytes: 0, parts: vec![] }];
    let all = targets(os);
    for kind in [CleanKind::TempFiles, CleanKind::ShaderCaches, CleanKind::LauncherCaches] {
        let mut row = CleanRow { kind, bytes: 0, items: 0, blocked_bytes: 0, parts: vec![] };
        for t in all.iter().filter(|t| t.kind == kind) {
            let part = measure_target(os, t, &known, &running, elevated);
            if part.state == PartState::Ready {
                row.bytes += part.bytes;
                row.items += part.files;
            } else {
                row.blocked_bytes += part.bytes;
            }
            row.parts.push(part);
        }
        rows.push(row);
    }
    Ok(CleanPlan { rows })
}

fn launcher_running(l: &Launcher, running: &[String]) -> bool {
    running.iter().any(|r| l.exes.contains(&r.as_str()))
}

fn measure_target(os: &dyn StorageOs, t: &Target, known: &KnownDirs, running: &[String], elevated: bool) -> PartSize {
    let mut part = PartSize { name: t.name.clone(), path: t.path.clone(), bytes: 0, files: 0, state: PartState::Ready, launcher: t.launcher };
    if !is_safe_target(&t.path, known) {
        part.state = PartState::Refused;
        return part;
    }
    match tree_size(os, &t.path) {
        Ok((bytes, files)) => {
            part.bytes = bytes;
            part.files = files;
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            part.state = PartState::Missing;
            return part;
        }
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied && !elevated => {
            part.state = PartState::NeedsAdmin;
            return part;
        }
        Err(_) => {
            part.state = PartState::NeedsAdmin;
            return part;
        }
    }
    if t.needs_admin && !elevated {
        part.state = PartState::NeedsAdmin;
    } else if let Some(l) = &t.launcher {
        if launcher_running(l, running) {
            part.state = PartState::LauncherRunning(l.name);
        }
    }
    part
}

/// Bytes + files under a folder. Unreadable sub-folders count 0; junctions are not followed.
fn tree_size(os: &dyn StorageOs, root: &Path) -> io::Result<(u64, u64)> {
    let mut stack = vec![root.to_path_buf()];
    let (mut bytes, mut files) = (0u64, 0u64);
    let mut first = true;
    while let Some(dir) = stack.pop() {
        let entries = match os.read_dir(&dir) {
            Ok(e) => e,
            Err(e) if first => return Err(e),
            Err(_) => continue,
        };
        first = false;
        for e in entries {
            if e.is_reparse {
                continue;
            }
            if e.is_dir {
                stack.push(dir.join(&e.name));
            } else {
                bytes += if e.is_cloud_only { 0 } else { e.size };
                files += 1;
            }
        }
    }
    Ok((bytes, files))
}

impl CleanPlan {
    pub fn row(&self, kind: CleanKind) -> Option<&CleanRow> {
        self.rows.iter().find(|r| r.kind == kind)
    }
    /// The ticks on open: the rows ticked by default ([`CleanKind::ticked_by_default`]) that have something in them
    /// (a 0 B row is greyed).
    pub fn default_ticked(&self) -> Vec<CleanKind> {
        self.rows.iter().filter(|r| !r.is_empty() && r.kind.ticked_by_default()).map(|r| r.kind).collect()
    }
    /// "Clean 14.4 GB": the ticked total.
    pub fn ticked_bytes(&self, ticked: &[CleanKind]) -> u64 {
        self.rows.iter().filter(|r| ticked.contains(&r.kind)).map(|r| r.bytes).sum()
    }

    /// Delete the ticked rows (only those). Launchers are checked again; anything that started since measuring is
    /// skipped. Returns what was freed and what stayed in use.
    pub fn clean(&self, os: &dyn StorageOs, ticked: &[CleanKind]) -> Result<CleanReport> {
        for k in ticked {
            if self.row(*k).is_none() {
                return Err(StorageError::NotMeasured);
            }
        }
        let known = os.known_dirs();
        let running = os.running_exe_names();
        let mut report = CleanReport::default();
        for row in self.rows.iter().filter(|r| ticked.contains(&r.kind)) {
            let mut done = CleanedRow { kind: Some(row.kind), ..Default::default() };
            if row.kind == CleanKind::RecycleBin {
                if row.bytes > 0 || row.items > 0 {
                    let before = os.recycle_bin()?;
                    os.empty_recycle_bin()?;
                    let after = os.recycle_bin()?;
                    done.freed_bytes = before.bytes.saturating_sub(after.bytes);
                    done.freed_files = before.items.saturating_sub(after.items);
                    done.in_use_bytes = after.bytes;
                    done.in_use_files = after.items;
                }
                report.rows.push(done);
                continue;
            }
            for part in &row.parts {
                let mut state = part.state.clone();
                if state == PartState::Ready && !is_safe_target(&part.path, &known) {
                    state = PartState::Refused;
                }
                if state == PartState::Ready {
                    if let Some(l) = part.launcher {
                        if launcher_running(&l, &running) {
                            state = PartState::LauncherRunning(l.name);
                        }
                    }
                }
                if state != PartState::Ready {
                    if state != PartState::Missing {
                        done.skipped.push((part.name.clone(), state));
                    }
                    continue;
                }
                delete_contents(os, &part.path, &mut done);
            }
            report.rows.push(done);
        }
        Ok(report)
    }
}

/// Delete everything *inside* `root` (never `root` itself). Files that won't go are counted as in use.
fn delete_contents(os: &dyn StorageOs, root: &Path, done: &mut CleanedRow) {
    // Depth-first: files first, then the emptied folders bottom-up.
    let mut dirs_to_remove: Vec<PathBuf> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match os.read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries {
            let p = dir.join(&e.name);
            if e.is_reparse {
                continue; // never follow or delete a link
            }
            if e.is_dir {
                dirs_to_remove.push(p.clone());
                stack.push(p);
            } else {
                let size = if e.is_cloud_only { 0 } else { e.size };
                match os.remove_file(&p) {
                    Ok(()) => {
                        done.freed_bytes += size;
                        done.freed_files += 1;
                    }
                    Err(_) => {
                        done.in_use_bytes += size;
                        done.in_use_files += 1;
                    }
                }
            }
        }
    }
    // Deeper folders were pushed later; remove them first. A folder that still holds an in-use file stays.
    for d in dirs_to_remove.iter().rev() {
        let _ = os.remove_dir(d);
    }
}
