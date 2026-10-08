//! The process list (DESIGN §3.8 "Processes"): one row per app with its helpers counted, CPU / RAM / GPU, Windows'
//! own processes locked, End task / End process tree with the "asks first" data, priority with undo.
//!
//! Rules this crate decided (DESIGN marks them **unclear**; reported as calls nobody approved):
//! * **One row per app** = processes with the same exe path in the same session ("Google Chrome (15)").
//! * **Windows' own** (hidden by default, greyed, locked, no End) = the kernel ones (System, Idle, Registry, Memory
//!   Compression, Secure System), the boot-critical list (smss, csrss, wininit, winlogon, services, lsass, lsaiso —
//!   ending one is a blue screen), anything whose exe is inside the Windows folder (except Explorer), and Defender
//!   (MsMpEng, NisSrv, MpDefenderCoreService).
//! * **Protected** (hidden and locked like Windows' own, its own menu line) = not yours and its file can't even be
//!   read without admin (third-party services, other users, DWM) — it could not be ended without admin anyway.
//! * **Isn't plainly an app → asks first**: Explorer, the anti-cheat / driver-helper list (with the drawing's why
//!   lines), any process without a window (background), and anything not running as you.
//! * **Priority on locked processes**: refused (nothing vital is touched). No Realtime ever. No priority change on a
//!   game protected by Vanguard while Vanguard runs (research C §0).

use crate::{EndHow, PerfError, PerfOs, Priority, ProcessUser, RawProcess, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Kernel / boot-critical processes — locked always.
const CRITICAL: &[&str] = &[
    "system", "idle", "system idle process", "registry", "memory compression", "secure system", "smss.exe", "csrss.exe",
    "wininit.exe", "winlogon.exe", "services.exe", "lsass.exe", "lsaiso.exe",
];
/// Defender lives outside the Windows folder but is Windows' own.
const DEFENDER: &[&str] = &["msmpeng.exe", "nissrv.exe", "mpdefendercoreservice.exe"];

/// Processes that ask first, with the drawing's why lines (and the same style for the rest of the warn list).
const ASK_FIRST: &[(&str, &str)] = &[
    ("explorer.exe", "Your taskbar and desktop vanish until Windows starts it again."),
    ("nvcontainer.exe", "A background part of the graphics driver. Its overlay and recording stop until you restart."),
    ("vgtray.exe", "Anti-cheat. Ending it only stops VALORANT from working until you restart the PC."),
    ("vgc.exe", "Anti-cheat. Ending it only stops VALORANT from working until you restart the PC."),
    ("easyanticheat.exe", "Anti-cheat. Ending it only stops its game from working until the game starts again."),
    ("easyanticheat_eos.exe", "Anti-cheat. Ending it only stops its game from working until the game starts again."),
    ("faceit.exe", "Anti-cheat. Ending it only stops FACEIT matches from working until you start it again."),
    ("faceitservice.exe", "Anti-cheat. Ending it only stops FACEIT matches from working until you restart the PC."),
    ("beservice.exe", "Anti-cheat. Ending it only stops its game from working until the game starts again."),
];
const NO_WINDOW_WHY: &str = "Runs in the background with no window. Whatever it does stops until it starts again.";
const NOT_YOURS_WHY: &str = "Runs as a Windows service or another user, not as you.";

/// Games protected by Riot Vanguard: no live priority change while `vgc.exe` runs.
const VANGUARD_GAMES: &[&str] = &["valorant-win64-shipping.exe", "valorant.exe", "league of legends.exe"];
/// Anti-cheat processes: never re-prioritised.
const ANTI_CHEAT: &[&str] =
    &["vgc.exe", "vgtray.exe", "easyanticheat.exe", "easyanticheat_eos.exe", "faceit.exe", "faceitservice.exe", "beservice.exe"];

/// What pressing End does for a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndRule {
    /// A plain app: ends at once.
    Instant,
    /// "End <Name>?" + this why line, then Cancel / End task.
    AskFirst { why: String },
    /// Windows' own: no End.
    Locked,
}

/// One row of the list (an app with its helpers).
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessRow {
    /// Stable key while the app runs: lower-case exe path (or exe name) + session.
    pub key: String,
    /// "Google Chrome" (the exe's description, else the exe name without ".exe").
    pub name: String,
    pub exe: String,
    pub path: Option<PathBuf>,
    /// Every process id in the group, the main one first.
    pub pids: Vec<u32>,
    pub cpu_pct: f64,
    pub ram_bytes: u64,
    pub gpu_pct: f64,
    pub user: ProcessUser,
    /// Windows' own (hidden unless "Show Windows processes"; greyed with a lock).
    pub windows_own: bool,
    /// Not ours and its file can't be read without admin (a service, another user, a protected process): hidden
    /// with Windows' own and locked the same way — it couldn't be ended without admin anyway.
    pub protected: bool,
    pub end_rule: EndRule,
    /// Not running as you and we are not admin: End / priority will need admin.
    pub needs_admin: bool,
    /// Highest priority in the group (pill when not Normal).
    pub priority: Priority,
}

impl ProcessRow {
    /// "Google Chrome (15)" — N = helpers + 1.
    pub fn display_name(&self) -> String {
        if self.pids.len() > 1 {
            format!("{} ({})", self.name, self.pids.len())
        } else {
            self.name.clone()
        }
    }
    /// "Part of Windows · locked" / the path, for the right-click menu's first line.
    pub fn menu_title(&self) -> String {
        if self.windows_own {
            "Part of Windows · it can’t be ended".to_string()
        } else if self.protected {
            "A service or another user’s process · needs admin".to_string()
        } else {
            self.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| self.exe.clone())
        }
    }
    /// Cells turn bright at CPU ≥ 5 %, RAM ≥ 1000 MB, GPU ≥ 5 %.
    pub fn bright(&self) -> (bool, bool, bool) {
        (self.cpu_pct >= 5.0, self.ram_bytes >= 1000 * 1024 * 1024, self.gpu_pct >= 5.0)
    }
}

/// Sort columns (DESIGN: click to sort, again to reverse; default CPU descending, Name starts A→Z).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Name,
    Cpu,
    Ram,
    Gpu,
}

/// "x.x %".
pub fn format_pct(p: f64) -> String {
    format!("{p:.1} %")
}

/// "N MB", or "x.x GB" from 1000 MB.
pub fn format_ram(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{mb:.0} MB")
    }
}

fn is_windows_own(p: &RawProcess, windows_dir: &str) -> bool {
    let exe = p.exe.to_lowercase();
    if p.pid == 0 || p.pid == 4 || CRITICAL.contains(&exe.as_str()) || DEFENDER.contains(&exe.as_str()) {
        return true;
    }
    if exe == "explorer.exe" {
        return false;
    }
    match &p.path {
        Some(path) => {
            let s = path.to_string_lossy().to_lowercase();
            !windows_dir.is_empty() && s.starts_with(&format!("{windows_dir}\\"))
        }
        None => false,
    }
}

fn end_rule(group: &[&RawProcess], windows_own: bool) -> EndRule {
    if windows_own {
        return EndRule::Locked;
    }
    let exe = group[0].exe.to_lowercase();
    if let Some((_, why)) = ASK_FIRST.iter().find(|(n, _)| *n == exe) {
        return EndRule::AskFirst { why: why.to_string() };
    }
    if group.iter().any(|p| p.user != ProcessUser::Me) {
        return EndRule::AskFirst { why: NOT_YOURS_WHY.to_string() };
    }
    if !group.iter().any(|p| p.has_window) {
        return EndRule::AskFirst { why: NO_WINDOW_WHY.to_string() };
    }
    EndRule::Instant
}

/// CPU % per pid from what each process used since the last refresh (`(pid, cpu time 100 ns, cycles)`, the Idle process =
/// pid 0 included) over `dt_100ns` on `cpus` logical processors. With cycle counts (the Idle process's included) a share is
/// its cycles / all cycles × 100 - what Task Manager shows; else (no cycle counts, e.g. the fake) CPU time / (dt × cpus).
pub fn cpu_shares(used: &[(u32, u64, u64)], dt_100ns: f64, cpus: f64) -> HashMap<u32, f64> {
    let mut out = HashMap::new();
    if dt_100ns <= 0.0 {
        return out;
    }
    let total: u64 = used.iter().map(|u| u.2).sum();
    let idle: u64 = used.iter().filter(|u| u.0 == 0).map(|u| u.2).sum();
    for &(pid, t, c) in used {
        let pct = if idle > 0 && total > 0 { c as f64 / total as f64 * 100.0 } else { t as f64 / (dt_100ns * cpus.max(1.0)) * 100.0 };
        *out.entry(pid).or_insert(0.0) += pct;
    }
    for v in out.values_mut() {
        *v = v.clamp(0.0, 100.0);
    }
    out
}

/// Keeps the previous snapshot so CPU % can be worked out; the menu calls [`ProcessMonitor::refresh`] once per tick
/// while the page is open.
pub struct ProcessMonitor {
    /// (pid, create time) -> (cpu_time, cycle_time) of the last refresh
    prev: HashMap<(u32, u64), (u64, u64)>,
    prev_at: Option<Instant>,
    windows_dir: String,
    /// Pid → parent pid, from the last refresh (for End process tree).
    parents: HashMap<u32, u32>,
    last: Vec<RawProcess>,
}

impl ProcessMonitor {
    /// `windows_dir` = `C:\Windows` (from `%SystemRoot%`).
    pub fn new(windows_dir: &Path) -> Self {
        ProcessMonitor {
            prev: HashMap::new(),
            prev_at: None,
            windows_dir: windows_dir.to_string_lossy().trim_end_matches('\\').to_lowercase(),
            parents: HashMap::new(),
            last: Vec::new(),
        }
    }

    /// For the real OS: the Windows folder from `%SystemRoot%`.
    pub fn for_this_pc() -> Self {
        let w = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        Self::new(&w)
    }

    /// A new list. CPU % is 0 on the first call (it needs two snapshots). `gpu_by_pid` comes from the live
    /// sampler's latest reading (empty → GPU 0 %). `show_windows` = the "Show Windows processes" switch.
    pub fn refresh(&mut self, os: &dyn PerfOs, gpu_by_pid: &HashMap<u32, f64>, show_windows: bool) -> Result<Vec<ProcessRow>> {
        self.refresh_at(os, gpu_by_pid, show_windows, Instant::now())
    }

    /// Same as [`refresh`](Self::refresh) with the time given (tests).
    pub fn refresh_at(
        &mut self,
        os: &dyn PerfOs,
        gpu_by_pid: &HashMap<u32, f64>,
        show_windows: bool,
        now: Instant,
    ) -> Result<Vec<ProcessRow>> {
        let procs = os.processes()?;
        let elevated = os.is_elevated();
        let cpus = os.cpu_count().max(1) as f64;
        let dt_100ns = self.prev_at.map(|t| now.saturating_duration_since(t).as_nanos() as f64 / 100.0).unwrap_or(0.0);
        let mut next_prev = HashMap::with_capacity(procs.len());
        // what each process used since the last refresh: (pid, cpu time, cycles). A process born since then used all it has.
        let mut used: Vec<(u32, u64, u64)> = Vec::with_capacity(procs.len());
        for p in &procs {
            let k = (p.pid, p.create_time);
            if self.prev_at.is_some() {
                let (c0, y0) = self.prev.get(&k).copied().unwrap_or((0, 0));
                used.push((p.pid, p.cpu_time.saturating_sub(c0), p.cycle_time.saturating_sub(y0)));
            }
            next_prev.insert(k, (p.cpu_time, p.cycle_time));
        }
        let cpu_of = cpu_shares(&used, dt_100ns, cpus);
        self.prev = next_prev;
        self.prev_at = Some(now);
        self.parents = procs.iter().map(|p| (p.pid, p.parent_pid)).collect();

        // Group: same exe path (or name when the path can't be read) + same session. Idle (pid 0) is skipped.
        let mut groups: Vec<(String, Vec<&RawProcess>)> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for p in procs.iter().filter(|p| p.pid != 0) {
            let id = p.path.as_ref().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_else(|| format!("#{}", p.exe.to_lowercase()));
            let key = format!("{id}|{}", p.session_id);
            match index.get(&key) {
                Some(&i) => groups[i].1.push(p),
                None => {
                    index.insert(key.clone(), groups.len());
                    groups.push((key, vec![p]));
                }
            }
        }
        let mut rows = Vec::with_capacity(groups.len());
        for (key, mut g) in groups {
            // Main process first: the one whose parent is not in the group (then the oldest).
            let pids: HashSet<u32> = g.iter().map(|p| p.pid).collect();
            g.sort_by_key(|p| (pids.contains(&p.parent_pid), p.create_time));
            let main = g[0];
            let windows_own = is_windows_own(main, &self.windows_dir);
            let protected = !windows_own && main.path.is_none() && main.user != ProcessUser::Me;
            if (windows_own || protected) && !show_windows {
                continue;
            }
            let name = main
                .description
                .clone()
                .filter(|d| !d.trim().is_empty())
                .unwrap_or_else(|| main.exe.trim_end_matches(".exe").trim_end_matches(".EXE").to_string());
            let user = if g.iter().all(|p| p.user == ProcessUser::Me) { ProcessUser::Me } else { main.user };
            rows.push(ProcessRow {
                key,
                name,
                exe: main.exe.clone(),
                path: main.path.clone(),
                pids: g.iter().map(|p| p.pid).collect(),
                cpu_pct: g.iter().map(|p| cpu_of.get(&p.pid).copied().unwrap_or(0.0)).sum::<f64>().min(100.0),
                ram_bytes: g.iter().map(|p| p.ram_bytes).sum(),
                gpu_pct: g.iter().map(|p| gpu_by_pid.get(&p.pid).copied().unwrap_or(0.0)).fold(0.0, f64::max),
                user,
                windows_own,
                protected,
                end_rule: end_rule(&g, windows_own || protected),
                needs_admin: !elevated && user != ProcessUser::Me,
                priority: g
                    .iter()
                    .map(|p| p.priority)
                    .min_by_key(|p| Priority::MENU.iter().position(|m| m == p).unwrap_or(0))
                    .unwrap_or(Priority::Normal),
            });
        }
        self.last = procs;
        sort(&mut rows, SortBy::Cpu, true);
        Ok(rows)
    }

    /// Children (and their children …) of `pid`, deepest first — End process tree ends these, then `pid`.
    pub fn descendants(&self, pid: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut stack = vec![pid];
        let mut seen = HashSet::from([pid]);
        while let Some(p) = stack.pop() {
            for (&child, &parent) in &self.parents {
                // pid reuse guard: a child must not be older than its parent (create_time check).
                if parent == p && seen.insert(child) && self.older_or_same(p, child) {
                    out.push(child);
                    stack.push(child);
                }
            }
        }
        out.reverse();
        out
    }

    fn older_or_same(&self, parent: u32, child: u32) -> bool {
        let t = |pid: u32| self.last.iter().find(|p| p.pid == pid).map(|p| p.create_time);
        match (t(parent), t(child)) {
            (Some(a), Some(b)) => a <= b,
            _ => true,
        }
    }

    /// Is a Vanguard-protected game's anti-cheat running?
    fn vanguard_running(&self) -> bool {
        self.last.iter().any(|p| p.exe.eq_ignore_ascii_case("vgc.exe"))
    }
}

/// Sort rows in place.
pub fn sort(rows: &mut [ProcessRow], by: SortBy, descending: bool) {
    rows.sort_by(|a, b| {
        let o = match by {
            SortBy::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortBy::Cpu => a.cpu_pct.total_cmp(&b.cpu_pct),
            SortBy::Ram => a.ram_bytes.cmp(&b.ram_bytes),
            SortBy::Gpu => a.gpu_pct.total_cmp(&b.gpu_pct),
        };
        let o = if descending { o.reverse() } else { o };
        o.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// Rows whose name matches the search (case-insensitive). Empty query = all.
pub fn filter<'a>(rows: &'a [ProcessRow], query: &str) -> Vec<&'a ProcessRow> {
    let q = query.trim().to_lowercase();
    rows.iter().filter(|r| q.is_empty() || r.name.to_lowercase().contains(&q) || r.exe.to_lowercase().contains(&q)).collect()
}

/// What happened when ending.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EndReport {
    pub ended: Vec<u32>,
    /// Pids Windows refused (access denied) — need admin.
    pub needs_admin: Vec<u32>,
    /// Already gone.
    pub gone: Vec<u32>,
}

impl EndReport {
    /// "<Name> closed" / "<Name> and its <k> helper processes closed".
    pub fn toast(&self, name: &str) -> String {
        match self.ended.len() {
            0 => format!("{name} could not be ended"),
            1 => format!("{name} closed"),
            n => format!("{name} and its {} helper processes closed", n - 1),
        }
    }
}

fn end_pids(os: &dyn PerfOs, pids: &[u32], how: EndHow) -> EndReport {
    let mut r = EndReport::default();
    for &pid in pids {
        match os.end_process(pid, how) {
            Ok(()) => r.ended.push(pid),
            Err(PerfError::NeedsAdmin(_)) => r.needs_admin.push(pid),
            Err(_) => r.gone.push(pid),
        }
    }
    r
}

impl ProcessMonitor {
    /// "End task": politely close the app's windows; helpers without a window are ended. `confirmed` must be true
    /// for an AskFirst row (the menu shows the confirm first). Locked rows refuse.
    pub fn end_task(&self, os: &dyn PerfOs, row: &ProcessRow, confirmed: bool) -> Result<EndReport> {
        self.check_end(row, confirmed)?;
        let windowed: Vec<u32> = row.pids.iter().copied().filter(|pid| self.last.iter().any(|p| p.pid == *pid && p.has_window)).collect();
        let mut report = end_pids(os, &windowed, EndHow::Close);
        let rest: Vec<u32> = row.pids.iter().copied().filter(|p| !windowed.contains(p)).collect();
        let more = end_pids(os, &rest, EndHow::Terminate);
        report.ended.extend(more.ended);
        report.needs_admin.extend(more.needs_admin);
        report.gone.extend(more.gone);
        Ok(report)
    }

    /// "End process tree": `TerminateProcess` on every process of the row and all their children, deepest first.
    pub fn end_tree(&self, os: &dyn PerfOs, row: &ProcessRow, confirmed: bool) -> Result<EndReport> {
        self.check_end(row, confirmed)?;
        let mut order: Vec<u32> = Vec::new();
        for &pid in &row.pids {
            for d in self.descendants(pid) {
                if !order.contains(&d) && !row.pids.contains(&d) {
                    order.push(d);
                }
            }
        }
        // Never reach into Windows' own processes through a tree.
        order.retain(|pid| {
            self.last.iter().find(|p| p.pid == *pid).map(|p| !is_windows_own(p, &self.windows_dir)).unwrap_or(false)
        });
        let mut group = row.pids.clone();
        group.reverse(); // helpers before the main process
        order.extend(group);
        Ok(end_pids(os, &order, EndHow::Terminate))
    }

    fn check_end(&self, row: &ProcessRow, confirmed: bool) -> Result<()> {
        match &row.end_rule {
            EndRule::Locked => Err(PerfError::Locked(row.name.clone())),
            EndRule::AskFirst { .. } if !confirmed => Err(PerfError::Refused(format!("End {}? needs the confirm first", row.name))),
            _ => Ok(()),
        }
    }

    /// Set a priority for every process of the row. Returns the undo (the old priority per pid).
    pub fn set_priority(&self, os: &dyn PerfOs, row: &ProcessRow, p: Priority) -> Result<PriorityUndo> {
        if p == Priority::Realtime {
            return Err(PerfError::Refused("Realtime priority can freeze the mouse".into()));
        }
        if row.windows_own || row.protected {
            return Err(PerfError::Locked(row.name.clone()));
        }
        let exe = row.exe.to_lowercase();
        if VANGUARD_GAMES.contains(&exe.as_str()) && self.vanguard_running() {
            return Err(PerfError::Refused("no priority changes on a Vanguard-protected game".into()));
        }
        if ANTI_CHEAT.contains(&exe.as_str()) {
            return Err(PerfError::Refused("anti-cheat processes are left alone".into()));
        }
        let mut undo = PriorityUndo { old: Vec::new() };
        for &pid in &row.pids {
            let old = self.last.iter().find(|x| x.pid == pid).map(|x| x.priority).unwrap_or(Priority::Normal);
            match os.set_priority(pid, p) {
                Ok(()) => undo.old.push((pid, old)),
                Err(e) if undo.old.is_empty() => return Err(e),
                Err(_) => {}
            }
        }
        Ok(undo)
    }

    /// "Open file location".
    pub fn open_file_location(&self, os: &dyn PerfOs, row: &ProcessRow) -> Result<()> {
        match &row.path {
            Some(p) => os.open_file_location(p),
            None => Err(PerfError::NotFound(format!("{}'s file", row.name))),
        }
    }
}

/// Switches a priority change back (the owner: undoable).
#[derive(Debug, Clone, PartialEq)]
pub struct PriorityUndo {
    pub old: Vec<(u32, Priority)>,
}

impl PriorityUndo {
    pub fn undo(&self, os: &dyn PerfOs) -> Result<()> {
        for &(pid, p) in &self.old {
            match os.set_priority(pid, p) {
                Ok(()) | Err(PerfError::NotFound(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
    /// Toast: "<Name> · priority <level> until it closes".
    pub fn toast(name: &str, p: Priority) -> String {
        format!("{name} · priority {} until it closes", p.name().to_lowercase())
    }
}
