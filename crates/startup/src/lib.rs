//! bu-startup — DESIGN.md §3.7 Startup, no UI.
//!
//! Everything that starts with Windows, in one list: Run / RunOnce keys (HKCU + HKLM, 64- and 32-bit views), the Startup folders,
//! Store apps' StartupTasks, scheduled tasks with a logon or boot trigger, and services set to Automatic. Switching follows
//! Windows' own way and never deletes anything:
//! - Run keys + Startup folders: the `Explorer\StartupApproved` flag Task Manager writes (byte 0: 02 on / 03 off).
//! - Scheduled tasks: enabled / disabled.
//! - Services: Automatic ↔ Manual.
//! - Store apps: only Windows Settings can switch them back on → `Switch::Settings`.
//! - RunOnce: runs once at the next sign-in (Microsoft Learn, "Run and RunOnce Registry Keys"). Whether Windows honours a
//!   StartupApproved flag for it is **unclear** (no source found; DESIGN §3.7 says "HKCU Run / RunOnce: flip StartupApproved" —
//!   a conflict) → listed, locked (boss answer A_006_02: no switch that may silently do nothing).
//!
//! Every change returns a [`Change`] holding the old value; [`Startup::undo`] puts it back exactly.

pub mod command;
pub mod fake;
pub mod impact;
pub mod os;
#[cfg(windows)]
pub mod real;
pub mod saved;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub use impact::{Cost, Impact};
pub use os::{Hive, OsError, RegView, ServiceStart, StartupOs, TaskTrigger};

pub const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const RUN_ONCE: &str = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
pub const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";
/// Where Windows Settings switches Store apps' startup.
pub const SETTINGS_STARTUP_APPS: &str = "ms-settings:startupapps";

/// Services nobody may switch (DESIGN §3.7: "Never touch vgc, EasyAntiCheat, BEService or FACEIT"). Compared without case.
pub const ANTI_CHEAT_SERVICES: &[&str] = &["vgc", "vgk", "EasyAntiCheat", "EasyAntiCheat_EOS", "BEService", "FACEIT", "FACEITService"];

/// Row group, as the header's All · Normal · Hidden control filters them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// Run key / Startup folder / Store app — Task Manager shows these too. Tag "Startup".
    Normal,
    /// Tag "Hidden: task".
    HiddenTask,
    /// Tag "Hidden: service".
    HiddenService,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    All,
    Normal,
    Hidden,
}

/// Which `StartupApproved` subkey holds an entry's flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ApprovedKey {
    Run,
    Run32,
    StartupFolder,
}

impl ApprovedKey {
    pub fn name(self) -> &'static str {
        match self {
            ApprovedKey::Run => "Run",
            ApprovedKey::Run32 => "Run32",
            ApprovedKey::StartupFolder => "StartupFolder",
        }
    }
}

/// The exact registry value Task Manager flips for an entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovedSlot {
    pub hive: Hive,
    pub key: ApprovedKey,
    pub value_name: String,
}

impl ApprovedSlot {
    pub fn path(&self) -> String {
        format!(r"{APPROVED}\{}", self.key.name())
    }
}

/// Where an entry lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    RunKey { hive: Hive, view: RegView, once: bool, value_name: String },
    StartupFolder { all_users: bool, file: PathBuf },
    StoreApp { package_family: String, task_id: String },
    Task { path: String, triggers: Vec<TaskTrigger> },
    Service { name: String, delayed: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockReason {
    /// Anti-cheat service (vgc, EasyAntiCheat, BEService, FACEIT): never touched.
    AntiCheat,
    /// A Windows (Microsoft) service while the policy keeps those read-only.
    WindowsOwnService,
    /// A Windows (Microsoft) scheduled task while the policy keeps those read-only.
    WindowsOwnTask,
    /// RunOnce: runs one time at the next sign-in; whether a StartupApproved flag works for it is unclear → locked (A_006_02).
    RunOnce,
}

/// Whether and how a row can be switched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    /// Switchable without admin.
    Free,
    /// Switchable, needs admin (the menu shows the shield).
    NeedsAdmin,
    /// Only Windows Settings can switch it; open this URI.
    Settings(&'static str),
    Locked(LockReason),
}

/// Startup impact of one row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImpactState {
    /// No boot report could be read (see [`ImpactSource`]).
    Unknown,
    /// A report was read and this program wasn't in it (Task Manager's "Not measured").
    NotMeasured,
    Measured(Impact, Cost),
}

/// Where the impact numbers came from this time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImpactSource {
    /// Read from Windows' newest StartupInfo boot report.
    Report,
    /// The report folder is admin-only and we are not admin.
    NeedsAdmin,
    /// No report exists yet (e.g. right after install).
    NoReport,
    Failed(OsError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupEntry {
    /// Stable id, unique in one list.
    pub id: String,
    /// The name to show: the exe's description where that is meaningful, else the entry's own name.
    pub name: String,
    /// The entry's own name: registry value name / file name / task path / service name.
    pub key_name: String,
    pub publisher: Option<String>,
    /// The full command line as stored.
    pub command: String,
    /// The program (exe) the command starts, when it can be found.
    pub path: Option<PathBuf>,
    /// File holding the icon (exe, dll, ico or png) + icon index inside it.
    pub icon_path: Option<PathBuf>,
    pub icon_index: i32,
    pub kind: Kind,
    pub source: Source,
    /// Where it lives, in words a power user knows (e.g. `HKCU\…\Run`, a folder path, a task path).
    pub location: String,
    pub enabled: bool,
    pub impact: ImpactState,
    /// Made by Microsoft (Windows' own): exe company name contains "Microsoft", or a task under `\Microsoft\`.
    pub windows_own: bool,
    pub switch: Switch,
    /// The `StartupApproved` value for Run-key / Startup-folder rows.
    pub approved: Option<ApprovedSlot>,
}

impl StartupEntry {
    pub fn needs_admin(&self) -> bool {
        self.switch == Switch::NeedsAdmin
    }
    pub fn can_switch(&self) -> bool {
        matches!(self.switch, Switch::Free | Switch::NeedsAdmin)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupList {
    /// Normal rows first, then tasks, then services (third-party before Windows' own in each hidden group).
    pub entries: Vec<StartupEntry>,
    pub impact_source: ImpactSource,
    /// Sources that could not be read (the rest of the list is still there).
    pub problems: Vec<(String, OsError)>,
}

impl StartupList {
    pub fn shown(&self, view: View) -> Vec<&StartupEntry> {
        self.entries
            .iter()
            .filter(|e| match view {
                View::All => true,
                View::Normal => e.kind == Kind::Normal,
                View::Hidden => e.kind != Kind::Normal,
            })
            .collect()
    }
    /// The group header's "<on> of <shown> on".
    pub fn counts(&self, view: View) -> (usize, usize) {
        let s = self.shown(view);
        (s.iter().filter(|e| e.enabled).count(), s.len())
    }
    pub fn get(&self, id: &str) -> Option<&StartupEntry> {
        self.entries.iter().find(|e| e.id == id)
    }
}

/// What may be switched beyond third-party rows. Defaults = the boss answer A_006_01 (Oct 8): Windows' own services AND tasks are
/// listed + marked but locked; one switch each so the owner can open them later.
/// (Both `false` by default = both locked.)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    pub windows_services_switchable: bool,
    pub windows_tasks_switchable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StartupError {
    #[error("needs administrator rights")]
    NeedsAdmin,
    #[error("this one is locked ({0:?})")]
    Locked(LockReason),
    #[error("only Windows Settings can switch this one ({0})")]
    UseSettings(&'static str),
    #[error("Windows refused the change even with admin rights")]
    Refused,
    #[error("Windows did not keep the change (read back differs)")]
    WriteBlocked,
    #[error("the entry is gone")]
    Gone,
    #[error(transparent)]
    Os(#[from] OsError),
}

/// The old value of one change, to put it back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Undo {
    Flag { slot: ApprovedSlot, old: Option<Vec<u8>> },
    Task { path: String, was_enabled: bool },
    Service { name: String, old_start: ServiceStart, old_delayed: bool, old_memory: Option<bool>, needs_admin: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub entry_id: String,
    pub entry_name: String,
    /// The state the change set.
    pub now_enabled: bool,
    pub undo: Undo,
}

/// The `StartupApproved` bytes Task Manager writes. On: `02 00 00 00` + 8 zero bytes. Off: `03 00 00 00` + FILETIME of the switch.
pub fn approved_bytes(on: bool, filetime: u64) -> Vec<u8> {
    let mut v = vec![if on { 0x02 } else { 0x03 }, 0, 0, 0];
    v.extend_from_slice(&if on { 0u64 } else { filetime }.to_le_bytes());
    v
}

/// A missing value = on; 02 = on, 03 = off (research v2 §7: MS Q&A "autoruns … StartupApproved", dedoimedo.com; on this PC only
/// 03 values exist). Other values: byte 0 odd = off, even = on — a guess (06 / 07 are reported by users, not documented).
pub fn approved_enabled(value: Option<&[u8]>) -> bool {
    match value.and_then(|v| v.first()) {
        None => true,
        Some(b) => b & 1 == 0,
    }
}

pub fn is_anti_cheat(service_name: &str) -> bool {
    ANTI_CHEAT_SERVICES.iter().any(|n| n.eq_ignore_ascii_case(service_name))
}

pub struct Startup<O: StartupOs> {
    os: O,
    policy: Policy,
}

impl<O: StartupOs> Startup<O> {
    pub fn new(os: O) -> Self {
        Startup { os, policy: Policy::default() }
    }
    pub fn with_policy(os: O, policy: Policy) -> Self {
        Startup { os, policy }
    }
    pub fn os(&self) -> &O {
        &self.os
    }
    pub fn policy(&self) -> Policy {
        self.policy
    }

    /// Read everything (READ-ONLY).
    pub fn list(&self) -> StartupList {
        let mut problems = Vec::new();
        let (impacts, impact_source) = self.read_impact();
        let mut entries = Vec::new();

        // Run keys: HKCU (one key for both views on current Windows; the 32-bit view is read too and duplicates dropped), HKLM 64, HKLM 32.
        for (hive, view) in [
            (Hive::CurrentUser, RegView::Bits64),
            (Hive::CurrentUser, RegView::Bits32),
            (Hive::LocalMachine, RegView::Bits64),
            (Hive::LocalMachine, RegView::Bits32),
        ] {
            for once in [false, true] {
                let key = if once { RUN_ONCE } else { RUN };
                match self.os.reg_strings(hive, view, key) {
                    Ok(values) => {
                        for v in values {
                            let dup = entries.iter().any(|e: &StartupEntry| {
                                matches!(&e.source, Source::RunKey { hive: h, once: o, value_name, .. }
                                    if hive == Hive::CurrentUser && *h == hive && *o == once && value_name.eq_ignore_ascii_case(&v.name))
                                    && e.command == v.data
                            });
                            if !dup {
                                entries.push(self.run_entry(hive, view, once, &v.name, &v.data, &impacts));
                            }
                        }
                    }
                    Err(e) => problems.push((format!("{}\\{key} ({view:?})", hive_name(hive)), e)),
                }
            }
        }

        for all_users in [false, true] {
            match self.os.startup_folder(all_users) {
                Ok(items) => {
                    for it in items.into_iter().filter(|i| !i.file_name.eq_ignore_ascii_case("desktop.ini")) {
                        entries.push(self.folder_entry(all_users, it, &impacts));
                    }
                }
                Err(e) => problems.push((format!("Startup folder ({})", if all_users { "all users" } else { "you" }), e)),
            }
        }

        match self.os.store_startup_tasks() {
            Ok(tasks) => {
                for t in tasks {
                    entries.push(StartupEntry {
                        id: format!("store|{}|{}", t.package_family, t.task_id),
                        name: t.display_name.clone().unwrap_or_else(|| t.task_id.clone()),
                        key_name: t.task_id.clone(),
                        publisher: t.publisher.clone(),
                        command: String::new(),
                        path: None,
                        icon_path: t.logo.clone(),
                        icon_index: 0,
                        kind: Kind::Normal,
                        location: format!("Store app {} (StartupTask {})", t.package_family, t.task_id),
                        enabled: matches!(t.state, 2 | 4),
                        impact: if impact_source == ImpactSource::Report { ImpactState::NotMeasured } else { ImpactState::Unknown },
                        windows_own: store_app_is_windows_part(&t.package_family, t.publisher.as_deref()),
                        switch: Switch::Settings(SETTINGS_STARTUP_APPS),
                        approved: None,
                        source: Source::StoreApp { package_family: t.package_family, task_id: t.task_id },
                    });
                }
            }
            Err(e) => problems.push(("Store apps".into(), e)),
        }

        match self.os.logon_tasks() {
            Ok(tasks) => {
                let mut rows: Vec<StartupEntry> = tasks.into_iter().map(|t| self.task_entry(t, &impacts)).collect();
                rows.sort_by(|a, b| a.windows_own.cmp(&b.windows_own).then(a.key_name.to_lowercase().cmp(&b.key_name.to_lowercase())));
                entries.extend(rows);
            }
            Err(e) => problems.push(("Scheduled tasks".into(), e)),
        }

        let memory = match self.os.remembered_services() {
            Ok(m) => m,
            Err(e) => {
                problems.push(("Remembered services".into(), e));
                Vec::new()
            }
        };
        let also: Vec<String> = memory.iter().map(|(n, _)| n.clone()).collect();
        match self.os.services(&also) {
            Ok(services) => {
                let mut rows: Vec<StartupEntry> = services
                    .into_iter()
                    .filter(|s| s.start == ServiceStart::Automatic || also.iter().any(|n| n.eq_ignore_ascii_case(&s.name)))
                    .map(|s| self.service_entry(s, &impacts))
                    .collect();
                rows.sort_by(|a, b| a.windows_own.cmp(&b.windows_own).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
                entries.extend(rows);
            }
            Err(e) => problems.push(("Services".into(), e)),
        }

        StartupList { entries, impact_source, problems }
    }

    /// Switch one row on or off the way Windows does. Returns the change (with the old value) for [`Startup::undo`].
    pub fn set_enabled(&self, entry: &StartupEntry, on: bool) -> Result<Change, StartupError> {
        match entry.switch {
            Switch::Locked(r) => return Err(StartupError::Locked(r)),
            Switch::Settings(uri) => return Err(StartupError::UseSettings(uri)),
            Switch::NeedsAdmin if !self.os.is_admin() => return Err(StartupError::NeedsAdmin),
            _ => {}
        }
        let undo = match &entry.source {
            Source::RunKey { .. } | Source::StartupFolder { .. } => {
                let slot = entry.approved.clone().ok_or(StartupError::Gone)?;
                let old = self.os.reg_binary(slot.hive, &slot.path(), &slot.value_name).map_err(|e| self.denied(e))?;
                let new = approved_bytes(on, self.os.now_filetime());
                self.write_flag(&slot, Some(&new))?;
                Undo::Flag { slot, old }
            }
            Source::Task { path, .. } => {
                let was_enabled = self.os.task_enabled(path).map_err(|e| self.denied(e))?;
                self.write_task(path, on)?;
                Undo::Task { path: path.clone(), was_enabled }
            }
            Source::Service { name, .. } => {
                let (old_start, old_delayed) = self.os.service_start(name).map_err(|e| self.denied(e))?;
                let memory = self.os.remembered_services()?;
                let old_memory = memory.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, d)| *d);
                if on {
                    let delayed = old_memory.unwrap_or(old_delayed);
                    self.write_service(name, ServiceStart::Automatic, delayed)?;
                    self.os.forget_service(name)?;
                } else {
                    self.write_service(name, ServiceStart::Manual, false)?;
                    // Keep it listed (and remember Automatic vs Automatic (Delayed Start)) — only when it was Automatic.
                    if old_start == ServiceStart::Automatic {
                        self.os.remember_service(name, old_delayed)?;
                    }
                }
                Undo::Service { name: name.clone(), old_start, old_delayed, old_memory, needs_admin: entry.needs_admin() }
            }
            Source::StoreApp { .. } => return Err(StartupError::UseSettings(SETTINGS_STARTUP_APPS)),
        };
        Ok(Change { entry_id: entry.id.clone(), entry_name: entry.name.clone(), now_enabled: on, undo })
    }

    /// Put the old value back exactly.
    pub fn undo(&self, change: &Change) -> Result<(), StartupError> {
        match &change.undo {
            Undo::Flag { slot, old } => {
                if slot.hive == Hive::LocalMachine && !self.os.is_admin() {
                    return Err(StartupError::NeedsAdmin);
                }
                self.write_flag(slot, old.as_deref())
            }
            Undo::Task { path, was_enabled } => {
                if !self.os.is_admin() {
                    return Err(StartupError::NeedsAdmin);
                }
                self.write_task(path, *was_enabled)
            }
            Undo::Service { name, old_start, old_delayed, old_memory, needs_admin } => {
                if *needs_admin && !self.os.is_admin() {
                    return Err(StartupError::NeedsAdmin);
                }
                self.write_service(name, *old_start, *old_delayed)?;
                match old_memory {
                    Some(d) => self.os.remember_service(name, *d)?,
                    None => self.os.forget_service(name)?,
                }
                Ok(())
            }
        }
    }

    // ---- writes, each read back ----

    fn write_flag(&self, slot: &ApprovedSlot, value: Option<&[u8]>) -> Result<(), StartupError> {
        let path = slot.path();
        match value {
            Some(v) => self.os.reg_set_binary(slot.hive, &path, &slot.value_name, v),
            None => self.os.reg_delete_value(slot.hive, &path, &slot.value_name),
        }
        .map_err(|e| self.denied(e))?;
        let back = self.os.reg_binary(slot.hive, &path, &slot.value_name).map_err(|e| self.denied(e))?;
        if back.as_deref() != value {
            return Err(StartupError::WriteBlocked);
        }
        Ok(())
    }

    fn write_task(&self, path: &str, on: bool) -> Result<(), StartupError> {
        self.os.set_task_enabled(path, on).map_err(|e| self.denied(e))?;
        if self.os.task_enabled(path).map_err(|e| self.denied(e))? != on {
            return Err(StartupError::WriteBlocked);
        }
        Ok(())
    }

    fn write_service(&self, name: &str, start: ServiceStart, delayed: bool) -> Result<(), StartupError> {
        self.os.set_service_start(name, start, delayed).map_err(|e| self.denied(e))?;
        let (s, d) = self.os.service_start(name).map_err(|e| self.denied(e))?;
        if s != start || (start == ServiceStart::Automatic && d != delayed) {
            return Err(StartupError::WriteBlocked);
        }
        Ok(())
    }

    /// Access denied without admin = "needs admin"; with admin = Windows locks it.
    fn denied(&self, e: OsError) -> StartupError {
        match e {
            OsError::AccessDenied if self.os.is_admin() => StartupError::Refused,
            OsError::AccessDenied | OsError::NeedsAdmin => StartupError::NeedsAdmin,
            OsError::NotFound => StartupError::Gone,
            other => StartupError::Os(other),
        }
    }

    // ---- reading helpers ----

    fn read_impact(&self) -> (HashMap<String, Cost>, ImpactSource) {
        match self.os.impact_reports() {
            Ok(reports) => match reports.first() {
                Some(xml) => (impact::parse_report(xml), ImpactSource::Report),
                None => (HashMap::new(), ImpactSource::NoReport),
            },
            Err(OsError::AccessDenied) => (HashMap::new(), ImpactSource::NeedsAdmin),
            Err(OsError::NotFound) => (HashMap::new(), ImpactSource::NoReport),
            Err(e) => (HashMap::new(), ImpactSource::Failed(e)),
        }
    }

    fn impact_of(&self, path: Option<&Path>, impacts: &HashMap<String, Cost>) -> ImpactState {
        if impacts.is_empty() {
            // No report read (or an empty one).
            return ImpactState::Unknown;
        }
        match path.and_then(|p| impacts.get(&p.to_string_lossy().to_lowercase())) {
            Some(c) => ImpactState::Measured(impact::classify(*c), *c),
            None => ImpactState::NotMeasured,
        }
    }

    fn program(&self, cmd: &str) -> Option<(PathBuf, String)> {
        let exists = |p: &str| self.os.file_exists(Path::new(p));
        let expand = |s: &str| self.os.expand_env(s);
        command::split_command(cmd, &exists, &expand)
    }

    /// Squirrel installers (Discord, FACEIT, …) start `Update.exe --processStart App.exe`; the app itself sits in the newest
    /// `app-<version>` folder next to Update.exe. Returns that app exe when it exists.
    fn squirrel_app(&self, prog: &Path, args: &str) -> Option<PathBuf> {
        if !prog.file_name()?.to_string_lossy().eq_ignore_ascii_case("update.exe") {
            return None;
        }
        let i = args.find("--processStart").or_else(|| args.find("--processstart"))?;
        let rest = args[i + "--processstart".len()..].trim_start();
        let app = match rest.strip_prefix('"') {
            Some(r) => &r[..r.find('"')?],
            None => rest.split(' ').next()?,
        };
        if app.is_empty() {
            return None;
        }
        let dir = prog.parent()?;
        let mut versions: Vec<(Vec<u64>, PathBuf)> = self
            .os
            .subdirs(dir)
            .into_iter()
            .filter_map(|d| {
                let name = d.file_name()?.to_string_lossy().to_lowercase();
                let v = name.strip_prefix("app-")?;
                Some((v.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect(), d))
            })
            .collect();
        versions.sort();
        versions.into_iter().rev().map(|(_, d)| d.join(app)).find(|p| self.os.file_exists(p))
    }

    fn run_entry(&self, hive: Hive, view: RegView, once: bool, value: &str, data: &str, impacts: &HashMap<String, Cost>) -> StartupEntry {
        let prog = self.program(data).map(|(p, args)| self.squirrel_app(&p, &args).unwrap_or(p));
        let info = prog.as_deref().map(|p| self.os.file_info(p)).unwrap_or_default();
        let approved = if once {
            None
        } else {
            Some(ApprovedSlot {
                hive,
                key: if view == RegView::Bits32 { ApprovedKey::Run32 } else { ApprovedKey::Run },
                value_name: value.to_string(),
            })
        };
        let enabled = match &approved {
            None => true,
            Some(slot) => approved_enabled(self.os.reg_binary(slot.hive, &slot.path(), &slot.value_name).ok().flatten().as_deref()),
        };
        let switch = if once {
            Switch::Locked(LockReason::RunOnce)
        } else if hive == Hive::LocalMachine {
            Switch::NeedsAdmin
        } else {
            Switch::Free
        };
        let wow = if view == RegView::Bits32 && hive == Hive::LocalMachine { r"\WOW6432Node" } else { "" };
        let key = if once { "RunOnce" } else { "Run" };
        StartupEntry {
            id: format!("run|{}|{view:?}|{key}|{value}", hive_name(hive)),
            name: display_name(value, prog.as_deref(), &info),
            key_name: value.to_string(),
            publisher: info.company.clone(),
            command: data.to_string(),
            impact: self.impact_of(prog.as_deref(), impacts),
            icon_path: prog.clone(),
            icon_index: 0,
            path: prog.clone(),
            kind: Kind::Normal,
            location: format!(r"{}\Software{wow}\Microsoft\Windows\CurrentVersion\{key}", hive_name(hive)),
            enabled,
            windows_own: self.windows_own(&info, prog.as_deref()),
            switch,
            approved,
            source: Source::RunKey { hive, view, once, value_name: value.to_string() },
        }
    }

    fn folder_entry(&self, all_users: bool, it: os::FolderItem, impacts: &HashMap<String, Cost>) -> StartupEntry {
        let target = it.target.clone().unwrap_or_else(|| it.path.to_string_lossy().into_owned());
        let prog = self.program(&format!("\"{target}\"")).map(|(p, _)| p);
        let info = prog.as_deref().map(|p| self.os.file_info(p)).unwrap_or_default();
        let windows_own = self.windows_own(&info, prog.as_deref());
        let hive = if all_users { Hive::LocalMachine } else { Hive::CurrentUser };
        let slot = ApprovedSlot { hive, key: ApprovedKey::StartupFolder, value_name: it.file_name.clone() };
        let enabled = approved_enabled(self.os.reg_binary(hive, &slot.path(), &slot.value_name).ok().flatten().as_deref());
        let (icon_path, icon_index) = match it.icon.as_deref().and_then(parse_icon_location) {
            Some((p, i)) => (Some(PathBuf::from(self.os.expand_env(&p))), i),
            None => (prog.clone(), 0),
        };
        let stem = Path::new(&it.file_name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(it.file_name.clone());
        let command = match it.arguments.as_deref().filter(|a| !a.is_empty()) {
            Some(a) => format!("\"{target}\" {a}"),
            None => format!("\"{target}\""),
        };
        StartupEntry {
            id: format!("folder|{}|{}", if all_users { "all" } else { "user" }, it.file_name),
            name: stem,
            key_name: it.file_name.clone(),
            publisher: info.company.clone(),
            command,
            impact: self.impact_of(prog.as_deref(), impacts),
            path: prog,
            icon_path,
            icon_index,
            kind: Kind::Normal,
            location: it.path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            enabled,
            windows_own,
            switch: if all_users { Switch::NeedsAdmin } else { Switch::Free },
            approved: Some(slot),
            source: Source::StartupFolder { all_users, file: it.path },
        }
    }

    fn task_entry(&self, t: os::RawTask, impacts: &HashMap<String, Cost>) -> StartupEntry {
        let prog = t.command.as_deref().and_then(|c| {
            let c = c.trim();
            let quoted = if c.starts_with('"') { c.to_string() } else { format!("\"{c}\"") };
            self.program(&quoted).map(|(p, _)| p)
        });
        let info = prog.as_deref().map(|p| self.os.file_info(p)).unwrap_or_default();
        // a task under \Microsoft\Windows\ is Windows' own unless its program says another company made it (Order 074)
        let in_windows_folder = t.path.to_lowercase().starts_with(r"\microsoft\windows\");
        let windows_own = (in_windows_folder && (info.company.is_none() || is_microsoft(&info))) || self.windows_own(&info, prog.as_deref());
        let switch = if windows_own && !self.policy.windows_tasks_switchable {
            Switch::Locked(LockReason::WindowsOwnTask)
        } else {
            Switch::NeedsAdmin
        };
        let command = match (&t.command, &t.arguments) {
            (Some(c), Some(a)) if !a.is_empty() => format!("{c} {a}"),
            (Some(c), _) => c.clone(),
            (None, _) => String::new(),
        };
        StartupEntry {
            id: format!("task|{}", t.path),
            name: t.name.clone(),
            key_name: t.path.clone(),
            publisher: info.company.clone().or(t.author.clone()),
            command,
            impact: self.impact_of(prog.as_deref(), impacts),
            icon_path: prog.clone(),
            icon_index: 0,
            path: prog,
            kind: Kind::HiddenTask,
            location: format!("Task Scheduler {}", t.path),
            enabled: t.enabled,
            windows_own,
            switch,
            approved: None,
            source: Source::Task { path: t.path, triggers: t.triggers },
        }
    }

    fn service_entry(&self, s: os::RawService, impacts: &HashMap<String, Cost>) -> StartupEntry {
        let prog = s.image_path.as_deref().and_then(|c| self.program(c)).map(|(p, _)| p);
        let info = prog.as_deref().map(|p| self.os.file_info(p)).unwrap_or_default();
        let windows_own = self.windows_own(&info, prog.as_deref());
        let switch = if is_anti_cheat(&s.name) {
            Switch::Locked(LockReason::AntiCheat)
        } else if windows_own && !self.policy.windows_services_switchable {
            Switch::Locked(LockReason::WindowsOwnService)
        } else {
            Switch::NeedsAdmin
        };
        StartupEntry {
            id: format!("service|{}", s.name),
            name: if s.display_name.is_empty() { s.name.clone() } else { s.display_name.clone() },
            key_name: s.name.clone(),
            publisher: info.company.clone(),
            command: s.image_path.clone().unwrap_or_default(),
            impact: self.impact_of(prog.as_deref(), impacts),
            icon_path: prog.clone(),
            icon_index: 0,
            path: prog,
            kind: Kind::HiddenService,
            location: format!(r"Services · HKLM\SYSTEM\CurrentControlSet\Services\{}", s.name),
            enabled: s.start == ServiceStart::Automatic,
            windows_own,
            switch,
            approved: None,
            source: Source::Service { name: s.name, delayed: s.delayed },
        }
    }
}

fn hive_name(h: Hive) -> &'static str {
    match h {
        Hive::CurrentUser => "HKCU",
        Hive::LocalMachine => "HKLM",
    }
}

/// A Store app that is a PART OF WINDOWS (Order 074): Windows' own packages (`MicrosoftWindows.*` = publisher "Microsoft Windows",
/// `Microsoft.Windows.*`, `Windows.*`) and the Microsoft.* components that ship inside it (Start feed, Windows Security).
/// Everyone else - Spotify, Claude, iTunes, and Microsoft's own apps that are not Windows (Xbox, Phone Link, Windows Terminal) -
/// is an ordinary row, whatever the startup kind. The publisher must say Microsoft too, so no other company's package can pass.
fn store_app_is_windows_part(package_family: &str, publisher: Option<&str>) -> bool {
    if !publisher.is_some_and(|p| p.to_lowercase().contains("microsoft")) {
        return false;
    }
    let name = package_family.split('_').next().unwrap_or("").to_lowercase();
    name.starts_with("microsoftwindows.")
        || name.starts_with("microsoft.windows.")
        || name.starts_with("windows.")
        || matches!(name.as_str(), "microsoft.startexperiencesapp" | "microsoft.sechealthui")
}

fn is_microsoft(info: &os::FileInfo) -> bool {
    info.company.as_deref().is_some_and(|c| c.to_lowercase().contains("microsoft"))
}

impl<O: StartupOs> Startup<O> {
    /// Windows' own (the v21/v22 drawing's "Windows" badge, Order 021): a PART OF WINDOWS - a Microsoft program under the
    /// Windows folder or in Windows Defender's / Windows Security's own folders (`…\Windows Defender\…`, e.g. MsMpEng.exe).
    /// Microsoft apps that are not part of Windows (OneDrive, Edge's updater, Teams…) are ordinary rows, as drawn.
    /// Without version info: a program under the Windows folder (fails closed: an unknown system file is treated as Windows'
    /// own and so stays locked).
    fn windows_own(&self, info: &os::FileInfo, prog: Option<&Path>) -> bool {
        let mut windir = self.os.expand_env("%SystemRoot%").to_lowercase();
        if windir.is_empty() || windir.contains('%') {
            windir = self.os.expand_env("%windir%").to_lowercase();
        }
        let path = prog.map(|p| p.to_string_lossy().to_lowercase()).unwrap_or_default();
        let in_windir = !windir.is_empty() && !windir.contains('%') && path.starts_with(&format!("{}\\", windir.trim_end_matches('\\')));
        if is_microsoft(info) {
            return in_windir || path.contains("\\windows defender") || path.contains("\\windows security");
        }
        info.company.is_none() && in_windir
    }
}

/// The exe's description when meaningful, else the entry's own name. Squirrel launchers (`Update.exe --processStart X`) describe
/// themselves generically, so those keep the entry's name.
fn display_name(own: &str, prog: Option<&Path>, info: &os::FileInfo) -> String {
    let launcher = prog
        .and_then(|p| p.file_name())
        .map(|f| f.to_string_lossy().eq_ignore_ascii_case("update.exe"))
        .unwrap_or(false);
    match info.description.as_deref().map(str::trim) {
        Some(d) if !d.is_empty() && !launcher => d.to_string(),
        _ => own.to_string(),
    }
}

/// `"C:\x\y.dll,-12"` → (`C:\x\y.dll`, -12).
fn parse_icon_location(s: &str) -> Option<(String, i32)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    match s.rsplit_once(',') {
        Some((p, i)) if i.trim().parse::<i32>().is_ok() => Some((p.trim_matches('"').to_string(), i.trim().parse().ok()?)),
        _ => Some((s.trim_matches('"').to_string(), 0)),
    }
}
