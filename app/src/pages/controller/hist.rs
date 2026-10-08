//! Undo / redo of the Controller tab (Order 042 item 9, the owner test build 2: "some things, like the controller settings,
//! really need a ctrl z type of thing, i just changed my deadzone by accident and can't remember what it was on before").
//!
//! The tab keeps its own list of the changes made in it while the app runs (kept across tab switches and menu closes in
//! the app's `Keep`, gone when the app ends). Each step holds the values from before and after the change and writes them
//! back through the same bu-controller path as the change itself (`apply_all` for a game's layout, `set_preferences` for a
//! controller's own file), so an undo is a normal write: one backup / change-log entry, Steam reads it on focus.
//! Ctrl+Z undoes, Ctrl+Y (or Ctrl+Shift+Z) redoes; a new change clears the redo list. A small "Undo" link sits in the row
//! of the last changed control (`Open::tail`).

use bu_controller::{Change, PadKind, PadView, Part, PrefSetting};

use super::Open;
use crate::ui::el::Key;

/// The values a step writes back.
#[derive(Clone, Debug, PartialEq)]
pub enum What {
    /// a game's layout (one controller type, one action set): the part's values before / after that differ
    Layout { game: String, kind: PadKind, set: u32, before: Vec<Change>, after: Vec<Change> },
    /// a controller's own settings file: the values before / after that differ (None = not in the file)
    Prefs { serial: String, before: Vec<(PrefSetting, Option<String>)>, after: Vec<(PrefSetting, Option<String>)> },
    /// "New action set…" (no way to remove a set by value: undone by bu-controller's own exact undo while it is still
    /// its last write; redone by making the set again)
    NewSet { game: String, kind: PadKind, from: u32, title: String, id: u32 },
}

/// One undoable change: what it wrote, its words ("Left stick · Dead zone"), the control it was made with.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub what: What,
    pub label: String,
    pub at: Option<Key>,
}

/// The tab's history (newest last).
#[derive(Clone, Debug, Default)]
pub struct Hist {
    pub undo: Vec<Step>,
    pub redo: Vec<Step>,
}

/// The `Keep` slot the history lives in between openings of the tab.
pub const KEEP: &str = "pad.hist";
/// Steps kept (older ones fall off).
const MAX: usize = 100;

/// The part a layout change belongs to.
pub fn part_of(c: &Change) -> Part {
    match c {
        Change::ButtonAction { button, .. } | Change::ButtonSetting { button, .. } => Part::Button(*button),
        Change::StickMode { side, .. } | Change::StickSetting { side, .. } | Change::StickRing { side, .. } => Part::Stick(*side),
        Change::TriggerAnalog { side, .. } | Change::TriggerSetting { side, .. } | Change::TriggerAction { side, .. } => Part::Trigger(*side),
        Change::GyroMode { .. } | Change::GyroSetting { .. } => Part::Gyro,
        Change::TouchMode { .. } | Change::TouchClick { .. } | Change::TouchSetting { .. } => Part::Touchpad,
    }
}

/// What differs between two views in the given parts, both ways: (the values to put back for undo, the values for redo).
/// Only the values that changed are written back - a key combo on another press of the button is never rewritten.
pub fn layout_diff(before: &PadView, after: &PadView, parts: &[Part], kind: PadKind) -> (Vec<Change>, Vec<Change>) {
    let (mut b, mut a) = (Vec::new(), Vec::new());
    for p in parts {
        let (pb, pa) = (before.part_changes(*p), after.part_changes(*p));
        b.extend(pb.iter().filter(|c| !pa.contains(c) && c.fits(kind)).cloned());
        a.extend(pa.iter().filter(|c| !pb.contains(c) && c.fits(kind)).cloned());
    }
    (b, a)
}

/// The parts a set of changes touches (each once, in order).
pub fn parts_of(changes: &[Change]) -> Vec<Part> {
    let mut out: Vec<Part> = Vec::new();
    for c in changes {
        let p = part_of(c);
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// A controller file's values that differ: (before, after).
pub fn prefs_diff(before: &bu_controller::Prefs, after: &bu_controller::Prefs) -> (Vec<(PrefSetting, Option<String>)>, Vec<(PrefSetting, Option<String>)>) {
    let (mut b, mut a) = (Vec::new(), Vec::new());
    for s in PrefSetting::ALL {
        let (x, y) = (before.get(s).map(str::to_string), after.get(s).map(str::to_string));
        if x != y {
            b.push((s, x));
            a.push((s, y));
        }
    }
    (b, a)
}

impl Hist {
    pub fn push(&mut self, s: Step) {
        self.undo.push(s);
        if self.undo.len() > MAX {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
}

impl Open {
    /// A change was written: keep it as one step (nothing that differs = nothing kept).
    pub(super) fn remember(&mut self, what: What, fallback: &str) {
        let empty = match &what {
            What::Layout { before, after, .. } => before.is_empty() && after.is_empty(),
            What::Prefs { before, .. } => before.is_empty(),
            What::NewSet { .. } => false,
        };
        let acting = self.acting.take();
        if empty {
            return;
        }
        let (at, label) = match acting {
            Some((k, l)) => (Some(k), l),
            None => (None, fallback.to_string()),
        };
        self.hist.push(Step { what, label, at });
    }

    /// The control of the newest step, when it is about what the tab shows now (its game, controller type and action set
    /// / its controller): its row shows the "Undo" link.
    pub(super) fn undo_at(&self) -> Option<Key> {
        let s = self.hist.undo.last()?;
        let here = match &s.what {
            What::Layout { game, kind, set, .. } => self.game().is_some_and(|g| &g.key == game) && *kind == self.kind && *set == self.set,
            What::Prefs { serial, .. } => self.pref().is_some_and(|p| &p.serial == serial),
            What::NewSet { game, kind, .. } => self.game().is_some_and(|g| &g.key == game) && *kind == self.kind,
        };
        if here {
            s.at
        } else {
            None
        }
    }

    /// Ctrl+Z (`redo` = false) / Ctrl+Y: the newest step of that list, written back; the tab shows where it happened.
    pub(super) fn undo(&mut self, redo: bool, now: f64) {
        let step = if redo { self.hist.redo.pop() } else { self.hist.undo.pop() };
        let Some(mut step) = step else {
            self.say(if redo { "Nothing to redo" } else { "Nothing to undo" }, now);
            return;
        };
        match self.replay(&mut step, redo) {
            Ok(()) => {
                self.say(format!("{} \u{b7} {}", if redo { "Redone" } else { "Undone" }, step.label), now);
                if redo {
                    self.hist.undo.push(step);
                } else {
                    self.hist.redo.push(step);
                }
            }
            Err(e) => {
                // not written: the step stays where it was
                if redo {
                    self.hist.redo.push(step);
                } else {
                    self.hist.undo.push(step);
                }
                self.say(e, now);
            }
        }
    }

    fn replay(&mut self, step: &mut Step, redo: bool) -> Result<(), String> {
        match &mut step.what {
            What::Layout { game, kind, set, before, after } => {
                self.show_at(game, *kind, Some(*set))?;
                let changes = if redo { after.clone() } else { before.clone() };
                self.apply_layout(&changes)
            }
            What::Prefs { serial, before, after } => {
                let vals = if redo { after.clone() } else { before.clone() };
                let v: Vec<(PrefSetting, Option<&str>)> = vals.iter().map(|(s, v)| (*s, v.as_deref())).collect();
                let had = self.had_prefs(serial);
                let r = match &mut self.svc {
                    Ok(s) => s.set_preferences(serial, &v, &step.label).map_err(|e| e.to_string()),
                    Err(e) => Err(e.clone()),
                };
                r?;
                let serial = serial.clone();
                self.log_prefs(&serial, had);
                self.reload_prefs();
                Ok(())
            }
            What::NewSet { game, kind, from, title, id } => {
                self.show_at(game, *kind, None)?;
                let g = self.game().cloned().ok_or("That game is gone")?;
                let had = self.had_layout(&g.key);
                if redo {
                    let r = match &mut self.svc {
                        Ok(s) => s.add_action_set(&g.key, *kind, *from, title).map_err(|e| e.to_string()),
                        Err(e) => Err(e.clone()),
                    };
                    *id = r?;
                    self.set = *id;
                } else {
                    // bu-controller's exact undo, only while the new set is still its last write
                    let last = self.svc.as_ref().ok().and_then(|s| s.undo_label());
                    if last.as_deref() != Some(format!("{} \u{b7} new action set", g.name).as_str()) {
                        return Err("The new action set can\u{2019}t be taken back any more (the layout was changed since)".into());
                    }
                    let r = match &mut self.svc {
                        Ok(s) => s.undo().map(|_| ()).map_err(|e| e.to_string()),
                        Err(e) => Err(e.clone()),
                    };
                    r?;
                    self.set = *from;
                }
                self.log_layout(&g, had);
                self.load_games_keep();
                Ok(())
            }
        }
    }

    /// Show this game / controller type / action set (an undo of something not on screen shows it first).
    fn show_at(&mut self, game: &str, kind: PadKind, set: Option<u32>) -> Result<(), String> {
        if kind != self.kind {
            self.kind = kind;
            self.pic = super::pic::pic(kind);
            self.sel = None;
            self.load_games();
            self.start_live();
        }
        let gi = self.games.iter().position(|g| g.key == game).ok_or("That game\u{2019}s layout is gone")?;
        if gi != self.gi {
            self.gi = gi;
            self.sel = None;
            self.set = set.unwrap_or(0);
            self.load_view();
        } else if let Some(s) = set.filter(|s| *s != self.set) {
            self.set = s;
            self.load_view();
        }
        Ok(())
    }
}
