//! "Switch automatically when an app starts" (DESIGN §3.2.5): rules + the logic "app X's PROCESS starts → apply its
//! preset (and vibrance) BEFORE its window exists; its process ends → switch back". Fed by `AppEvent`s from the app
//! watcher (`win::watch`, event-driven) or from tests.
//!
//! Rules from the owner via the boss (NOTE_004_01 + addenda, Oct 8):
//! - The trigger is the process starting, so the desktop is switched before the game goes fullscreen.
//! - Never switch while the game already has a window ("switching res while the game is running makes it glitch out a
//!   ton"): a late start is left alone and a note is left for the menu ("…it will be next launch").
//! - No keep/revert prompt for automatic switches. A preset can be used by a rule only on a monitor where it was
//!   applied and KEPT once by hand. If Windows reports the change failed: switch back silently + a note for the menu.
//!
//! Calls made for what DESIGN left unclear (also in the report):
//! - **Which monitor:** the main display (the game has no window yet when its process starts).
//! - **Two matching apps at once:** per monitor, the last one started wins; when it closes, the one still running
//!   takes over again; when the last one closes, the monitor goes back to how it was before the first one.
//! - **Vibrance goes back too** when the app closes ("Switches back" = everything it changed).

use crate::error::Result;
use crate::os::DisplayOs;
use crate::presets::{PresetId, PresetList};
use crate::service::DisplayService;
use crate::types::{Mode, MonitorId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RuleId(pub u64);

/// One row: app → preset (+ vibrance chip) with its switch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRule {
    pub id: RuleId,
    /// Full path of the .exe ("Browse for an app…") or just its file name (recent games list).
    pub exe: String,
    /// `None` = "Choose a preset" (also after its preset was deleted).
    pub preset: Option<PresetId>,
    /// `None` = "—" (no vibrance change); else 60..=100 in steps of 10 from the chip's menu.
    pub vibrance: Option<u8>,
    pub enabled: bool,
}

/// The vibrance chip's menu: "No change", "Vibrance 60 %" … "100 %".
pub const VIBRANCE_CHOICES: [u8; 5] = [60, 70, 80, 90, 100];

/// What the app watcher reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppEvent {
    /// A watched process started. `exe` = its full path if known, else its file name. `monitor` = where it will open
    /// (the main display). `has_window` = it already showed a window when the start was noticed (too late to switch).
    Started { pid: u32, exe: String, monitor: MonitorId, has_window: bool },
    Stopped { pid: u32 },
}

/// What the switcher asks the service to do.
#[derive(Clone, Debug, PartialEq)]
pub enum SwitchAction {
    ApplyPreset { monitor: MonitorId, preset: PresetId },
    RestoreMode { monitor: MonitorId, mode: Mode },
    SetVibrance { monitor: MonitorId, percent: u8 },
}

/// Notes for the menu (shown later as a quiet toast; never a prompt, never in a game).
#[derive(Clone, Debug, PartialEq)]
pub enum SwitchNote {
    /// The app already had a window when its start was noticed: nothing switched (it will be next launch).
    TooLate { rule: RuleId, exe: String },
    /// The rule's preset was never applied + kept on this monitor, so it isn't used automatically.
    PresetNotKept { rule: RuleId, preset: PresetId, monitor: MonitorId },
    /// Windows refused the switch; everything was put back silently.
    SwitchFailed { rule: RuleId, monitor: MonitorId, detail: String },
}

/// The state of one monitor before the first rule switched it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Saved {
    pub mode: Mode,
    pub vibrance: Option<u8>,
}

#[derive(Clone, Debug)]
struct Active {
    pid: u32,
    rule: RuleId,
    monitor: MonitorId,
}

#[derive(Clone, Debug)]
struct MonitorSession {
    monitor: MonitorId,
    saved: Saved,
    vibrance_changed: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AutoSwitcher {
    rules: Vec<AppRule>,
    next_id: u64,
    #[serde(skip)]
    active: Vec<Active>,
    #[serde(skip)]
    sessions: Vec<MonitorSession>,
    #[serde(skip)]
    notes: Vec<SwitchNote>,
}

fn file_name(p: &str) -> &str {
    p.rsplit(['\\', '/']).next().unwrap_or(p)
}

/// Rule matching: a full path in the rule must equal the process path (case-insensitive) when the process path is
/// known; otherwise the file names must match (case-insensitive).
pub fn exe_matches(rule_exe: &str, process_exe: &str) -> bool {
    let rule_has_dir = rule_exe.contains(['\\', '/']);
    let proc_has_dir = process_exe.contains(['\\', '/']);
    if rule_has_dir && proc_has_dir {
        rule_exe.eq_ignore_ascii_case(process_exe)
    } else {
        file_name(rule_exe).eq_ignore_ascii_case(file_name(process_exe))
    }
}

impl AutoSwitcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rules(&self) -> &[AppRule] {
        &self.rules
    }

    /// "+ Add app": a new row (first Stretch preset, on, no vibrance change — the caller passes that preset).
    pub fn add_rule(&mut self, exe: impl Into<String>, preset: Option<PresetId>) -> RuleId {
        self.next_id += 1;
        let id = RuleId(self.next_id);
        self.rules.push(AppRule { id, exe: exe.into(), preset, vibrance: None, enabled: true });
        id
    }

    pub fn rule_mut(&mut self, id: RuleId) -> Option<&mut AppRule> {
        self.rules.iter_mut().find(|r| r.id == id)
    }

    /// Row ×: removes the rule (an app it already switched still switches back when it closes).
    pub fn remove_rule(&mut self, id: RuleId) -> Option<AppRule> {
        let i = self.rules.iter().position(|r| r.id == id)?;
        Some(self.rules.remove(i))
    }

    /// After a preset was deleted: rows pointing at it fall back to "Choose a preset".
    pub fn forget_preset(&mut self, preset: PresetId) {
        for r in &mut self.rules {
            if r.preset == Some(preset) {
                r.preset = None;
            }
        }
    }

    /// Every row as one line of text (the app's change log keeps it to put the rows back later, Order 036). No rows = `[]`.
    pub fn rules_raw(&self) -> String {
        serde_json::to_string(&self.rules).unwrap_or_else(|_| "[]".into())
    }

    /// Puts back the rows of [`AutoSwitcher::rules_raw`] in place of today's (a row pointing at a preset that is gone falls
    /// back to "Choose a preset"); later new rows get ids after every id seen. An app it already switched still switches
    /// back when it closes.
    pub fn set_rules_raw(&mut self, raw: &str, presets: &PresetList) -> Result<()> {
        let mut rules: Vec<AppRule> = serde_json::from_str(raw).map_err(|e| crate::error::DisplayError::os("read the rules", e))?;
        for r in &mut rules {
            if r.preset.is_some_and(|p| presets.get(p).is_none()) {
                r.preset = None;
            }
        }
        self.next_id = rules.iter().map(|r| r.id.0).chain([self.next_id]).max().unwrap_or(0);
        self.rules = rules;
        Ok(())
    }

    /// The exe file names to watch (only these are ever looked at by the watcher). No rules → nothing is watched.
    pub fn watched_names(&self) -> Vec<String> {
        let mut v: Vec<String> =
            self.rules.iter().filter(|r| r.enabled && r.preset.is_some()).map(|r| file_name(&r.exe).to_ascii_lowercase()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// Notes for the menu since the last call.
    pub fn take_notes(&mut self) -> Vec<SwitchNote> {
        std::mem::take(&mut self.notes)
    }

    fn rule(&self, id: RuleId) -> Option<&AppRule> {
        self.rules.iter().find(|r| r.id == id)
    }

    /// Pure planning step: what to do for one event. `current(monitor)` gives the monitor's state right now
    /// (saved before the first switch). The caller runs the actions (`run` does both).
    pub fn plan(&mut self, ev: &AppEvent, presets: &PresetList, current: &mut dyn FnMut(&MonitorId) -> Option<Saved>) -> Vec<SwitchAction> {
        match ev {
            AppEvent::Started { pid, exe, monitor, has_window } => {
                if self.active.iter().any(|a| a.pid == *pid) {
                    return vec![];
                }
                let Some(rule) = self
                    .rules
                    .iter()
                    .find(|r| r.enabled && r.preset.map(|p| presets.get(p).is_some()).unwrap_or(false) && exe_matches(&r.exe, exe))
                    .cloned()
                else {
                    return vec![];
                };
                let preset = rule.preset.unwrap_or(PresetId(0));
                if *has_window {
                    self.notes.push(SwitchNote::TooLate { rule: rule.id, exe: file_name(exe).to_string() });
                    return vec![];
                }
                if !presets.is_kept_on(preset, monitor) {
                    self.notes.push(SwitchNote::PresetNotKept { rule: rule.id, preset, monitor: monitor.clone() });
                    return vec![];
                }
                if !self.sessions.iter().any(|s| &s.monitor == monitor) {
                    let Some(saved) = current(monitor) else { return vec![] };
                    self.sessions.push(MonitorSession { monitor: monitor.clone(), saved, vibrance_changed: false });
                }
                self.active.push(Active { pid: *pid, rule: rule.id, monitor: monitor.clone() });
                let mut out = vec![SwitchAction::ApplyPreset { monitor: monitor.clone(), preset }];
                if let Some(v) = rule.vibrance {
                    if let Some(s) = self.sessions.iter_mut().find(|s| &s.monitor == monitor) {
                        s.vibrance_changed = true;
                    }
                    out.push(SwitchAction::SetVibrance { monitor: monitor.clone(), percent: v });
                }
                out
            }
            AppEvent::Stopped { pid } => {
                let Some(i) = self.active.iter().position(|a| a.pid == *pid) else { return vec![] };
                let gone = self.active.remove(i);
                let monitor = gone.monitor.clone();
                let was_top = !self.active[i..].iter().any(|a| a.monitor == monitor);
                if !was_top {
                    return vec![];
                }
                let next = self.active.iter().rev().find(|a| a.monitor == monitor).cloned();
                let si = self.sessions.iter().position(|s| s.monitor == monitor);
                match (next, si) {
                    (Some(n), Some(si)) => {
                        let mut out = vec![];
                        let rule = self.rule(n.rule).cloned();
                        match rule.as_ref().and_then(|r| r.preset).filter(|p| presets.get(*p).is_some()) {
                            Some(p) => out.push(SwitchAction::ApplyPreset { monitor: monitor.clone(), preset: p }),
                            None => out.push(SwitchAction::RestoreMode { monitor: monitor.clone(), mode: self.sessions[si].saved.mode }),
                        }
                        let s = &self.sessions[si];
                        match rule.and_then(|r| r.vibrance) {
                            Some(v) => out.push(SwitchAction::SetVibrance { monitor: monitor.clone(), percent: v }),
                            None => {
                                if s.vibrance_changed {
                                    if let Some(v) = s.saved.vibrance {
                                        out.push(SwitchAction::SetVibrance { monitor: monitor.clone(), percent: v });
                                    }
                                }
                            }
                        }
                        out
                    }
                    (None, Some(si)) => {
                        let s = self.sessions.remove(si);
                        let mut out = vec![SwitchAction::RestoreMode { monitor: monitor.clone(), mode: s.saved.mode }];
                        if s.vibrance_changed {
                            if let Some(v) = s.saved.vibrance {
                                out.push(SwitchAction::SetVibrance { monitor, percent: v });
                            }
                        }
                        out
                    }
                    _ => vec![],
                }
            }
        }
    }

    /// Plans and runs one event against the service: no keep bar, no undo steps. If a switch for a START fails,
    /// everything is put back silently (saved mode + vibrance), the app is no longer tracked, and a `SwitchFailed` note
    /// is left. Returns the actions that ran.
    pub fn run<O: DisplayOs>(&mut self, ev: &AppEvent, presets: &PresetList, svc: &mut DisplayService<O>) -> Result<Vec<SwitchAction>> {
        let actions = {
            let mut current = |m: &MonitorId| -> Option<Saved> {
                let mode = svc.monitor(m).ok()?.current;
                let vibrance = svc.vibrance_percent(m).ok();
                Some(Saved { mode, vibrance })
            };
            self.plan(ev, presets, &mut current)
        };
        let mut done = Vec::new();
        for a in &actions {
            let r = match a {
                SwitchAction::ApplyPreset { monitor, preset } => (|| {
                    let p = presets.get(*preset).ok_or(crate::DisplayError::PresetNotFound)?;
                    let modes = svc.os().modes(monitor)?;
                    let mode = p.resolve(&modes)?;
                    svc.apply_mode_now(monitor, mode).map(|_| ())
                })(),
                SwitchAction::RestoreMode { monitor, mode } => svc.apply_mode_now(monitor, *mode).map(|_| ()),
                SwitchAction::SetVibrance { monitor, percent } => svc.set_vibrance_percent_auto(monitor, *percent),
            };
            match r {
                Ok(()) => done.push(a.clone()),
                Err(e) => {
                    if let AppEvent::Started { pid, .. } = ev {
                        self.fail_start(*pid, e.to_string(), svc, presets);
                        return Ok(done);
                    }
                    // Switching back failed: keep going with the rest (vibrance etc.), note it.
                    if let SwitchAction::RestoreMode { monitor, .. } | SwitchAction::ApplyPreset { monitor, .. } = a {
                        self.notes.push(SwitchNote::SwitchFailed { rule: RuleId(0), monitor: monitor.clone(), detail: e.to_string() });
                    }
                }
            }
        }
        Ok(done)
    }

    /// A start's switch failed: untrack the app and silently put its monitor back — to the state of the app that still
    /// owns it (if one does), else to the saved one. Same plan as that app stopping; errors are only noted.
    fn fail_start<O: DisplayOs>(&mut self, pid: u32, detail: String, svc: &mut DisplayService<O>, presets: &PresetList) {
        let Some(gone) = self.active.iter().find(|a| a.pid == pid).cloned() else { return };
        self.notes.push(SwitchNote::SwitchFailed { rule: gone.rule, monitor: gone.monitor.clone(), detail });
        let back = self.plan(&AppEvent::Stopped { pid }, presets, &mut |_| None);
        for a in &back {
            let _ = match a {
                SwitchAction::ApplyPreset { monitor, preset } => (|| {
                    let p = presets.get(*preset).ok_or(crate::DisplayError::PresetNotFound)?;
                    let mode = p.resolve(&svc.os().modes(monitor)?)?;
                    set_mode_if_different(svc, monitor, mode)
                })(),
                SwitchAction::RestoreMode { monitor, mode } => set_mode_if_different(svc, monitor, *mode),
                SwitchAction::SetVibrance { monitor, percent } => svc.set_vibrance_percent_auto(monitor, *percent),
            };
        }
    }
}

/// Puts a monitor on `mode` unless it already shows it (no needless display flicker when putting things back).
fn set_mode_if_different<O: DisplayOs>(svc: &mut DisplayService<O>, monitor: &MonitorId, mode: crate::types::Mode) -> Result<()> {
    if svc.monitor(monitor)?.current == mode {
        return Ok(());
    }
    svc.apply_mode_now(monitor, mode).map(|_| ())
}
