//! One row's switch as plain text, for the app's ONE change log (Order 036): WHERE it lives ([`Target`], stable across
//! restarts: the StartupApproved value / the task path / the service name) and its state ([`State`]). The app keeps the
//! state from before its first change and puts it back later — on a fresh process (the uninstaller), with no list read:
//! [`Startup::state_of`] reads one target's state now, [`Startup::put_back`] sets it the way Windows does (each read back).

use crate::os::{Hive, ServiceStart, StartupOs};
use crate::{approved_bytes, approved_enabled, ApprovedKey, ApprovedSlot, Change, Source, StartupEntry, StartupError, Startup, Undo};

/// Where one switchable row lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A Run key value / Startup folder file: its `StartupApproved` flag.
    Flag(ApprovedSlot),
    /// A scheduled task (its full path).
    Task(String),
    /// A service (its name).
    Service(String),
}

/// A row's switch state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// A flag or a task: on / off.
    Enabled(bool),
    /// A service's start type (+ Automatic (Delayed Start)).
    Service { start: ServiceStart, delayed: bool },
}

impl Target {
    /// The row's target (None: a Store app or a RunOnce value - not switched by the app).
    pub fn of(e: &StartupEntry) -> Option<Target> {
        match &e.source {
            Source::RunKey { .. } | Source::StartupFolder { .. } => e.approved.clone().map(Target::Flag),
            Source::Task { path, .. } => Some(Target::Task(path.clone())),
            Source::Service { name, .. } => Some(Target::Service(name.clone())),
            Source::StoreApp { .. } => None,
        }
    }

    /// The target of a change (from its old value).
    pub fn of_change(c: &Change) -> Target {
        match &c.undo {
            Undo::Flag { slot, .. } => Target::Flag(slot.clone()),
            Undo::Task { path, .. } => Target::Task(path.clone()),
            Undo::Service { name, .. } => Target::Service(name.clone()),
        }
    }

    /// Plain text: `flag|HKCU|Run|<value name>`, `task|<path>`, `service|<name>`.
    pub fn to_text(&self) -> String {
        match self {
            Target::Flag(s) => {
                let h = if s.hive == Hive::LocalMachine { "HKLM" } else { "HKCU" };
                format!("flag|{h}|{}|{}", s.key.name(), s.value_name)
            }
            Target::Task(p) => format!("task|{p}"),
            Target::Service(n) => format!("service|{n}"),
        }
    }

    pub fn from_text(t: &str) -> Option<Target> {
        let (kind, rest) = t.split_once('|')?;
        match kind {
            "flag" => {
                let mut p = rest.splitn(3, '|');
                let hive = match p.next()? {
                    "HKCU" => Hive::CurrentUser,
                    "HKLM" => Hive::LocalMachine,
                    _ => return None,
                };
                let key = match p.next()? {
                    "Run" => ApprovedKey::Run,
                    "Run32" => ApprovedKey::Run32,
                    "StartupFolder" => ApprovedKey::StartupFolder,
                    _ => return None,
                };
                let value_name = p.next().filter(|n| !n.is_empty())?.to_string();
                Some(Target::Flag(ApprovedSlot { hive, key, value_name }))
            }
            "task" if !rest.is_empty() => Some(Target::Task(rest.into())),
            "service" if !rest.is_empty() => Some(Target::Service(rest.into())),
            _ => None,
        }
    }

    /// A row Task Manager shows too (Run key / Startup folder): the menu says "Starts with Windows" for on.
    pub fn is_normal(&self) -> bool {
        matches!(self, Target::Flag(_))
    }
}

impl State {
    /// Does the row start with Windows in this state?
    pub fn is_on(self) -> bool {
        match self {
            State::Enabled(on) => on,
            State::Service { start, .. } => start == ServiceStart::Automatic,
        }
    }

    /// The state before a change (its old value).
    pub fn before(c: &Change) -> State {
        match &c.undo {
            Undo::Flag { old, .. } => State::Enabled(approved_enabled(old.as_deref())),
            Undo::Task { was_enabled, .. } => State::Enabled(*was_enabled),
            Undo::Service { old_start, old_delayed, .. } => State::Service { start: *old_start, delayed: *old_delayed },
        }
    }

    /// Plain text: `on`, `off`, `auto`, `auto-delayed`, `manual`, `disabled`, `boot`, `system`.
    pub fn to_text(self) -> String {
        match self {
            State::Enabled(true) => "on".into(),
            State::Enabled(false) => "off".into(),
            State::Service { start, delayed } => match start {
                ServiceStart::Automatic if delayed => "auto-delayed".into(),
                ServiceStart::Automatic => "auto".into(),
                ServiceStart::Manual => "manual".into(),
                ServiceStart::Disabled => "disabled".into(),
                ServiceStart::Boot => "boot".into(),
                ServiceStart::System => "system".into(),
            },
        }
    }

    pub fn from_text(t: &str) -> Option<State> {
        let svc = |start, delayed| Some(State::Service { start, delayed });
        match t {
            "on" => Some(State::Enabled(true)),
            "off" => Some(State::Enabled(false)),
            "auto" => svc(ServiceStart::Automatic, false),
            "auto-delayed" => svc(ServiceStart::Automatic, true),
            "manual" => svc(ServiceStart::Manual, false),
            "disabled" => svc(ServiceStart::Disabled, false),
            "boot" => svc(ServiceStart::Boot, false),
            "system" => svc(ServiceStart::System, false),
            _ => None,
        }
    }
}

impl<O: StartupOs> Startup<O> {
    /// One target's state right now (one read: the flag / the task / the service's start type).
    pub fn state_of(&self, t: &Target) -> Result<State, StartupError> {
        Ok(match t {
            Target::Flag(slot) => {
                State::Enabled(approved_enabled(self.os.reg_binary(slot.hive, &slot.path(), &slot.value_name).map_err(|e| self.denied(e))?.as_deref()))
            }
            Target::Task(path) => State::Enabled(self.os.task_enabled(path).map_err(|e| self.denied(e))?),
            Target::Service(name) => {
                let (start, delayed) = self.os.service_start(name).map_err(|e| self.denied(e))?;
                State::Service { start, delayed: start == ServiceStart::Automatic && delayed }
            }
        })
    }

    /// Put one target to a state, the way Windows does (a flag: Task Manager's 02 / 03 bytes; a task: enabled /
    /// disabled; a service: its start type), read back. A service set to something other than Automatic stays in the
    /// app's list (remembered, as `set_enabled` does); set to Automatic it is forgotten.
    pub fn put_back(&self, t: &Target, s: State) -> Result<(), StartupError> {
        match (t, s) {
            (Target::Flag(slot), State::Enabled(on)) => {
                if self.state_of(t)? == s {
                    return Ok(());
                }
                if slot.hive == Hive::LocalMachine && !self.os.is_admin() {
                    return Err(StartupError::NeedsAdmin);
                }
                self.write_flag(slot, Some(&approved_bytes(on, self.os.now_filetime())))
            }
            (Target::Task(path), State::Enabled(on)) => {
                if !self.os.is_admin() {
                    return Err(StartupError::NeedsAdmin);
                }
                self.write_task(path, on)
            }
            (Target::Service(name), State::Service { start, delayed }) => {
                if !self.os.is_admin() {
                    return Err(StartupError::NeedsAdmin);
                }
                let memory = self.os.remembered_services()?;
                let remembered = memory.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, d)| *d);
                let (cur, cur_delayed) = self.os.service_start(name).map_err(|e| self.denied(e))?;
                self.write_service(name, start, delayed)?;
                if start == ServiceStart::Automatic {
                    self.os.forget_service(name)?;
                } else {
                    // remember Automatic vs Automatic (Delayed Start) for switching it on again
                    let d = remembered.unwrap_or(if cur == ServiceStart::Automatic { cur_delayed } else { delayed });
                    self.os.remember_service(name, d)?;
                }
                Ok(())
            }
            _ => Err(StartupError::Gone),
        }
    }
}
