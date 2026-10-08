//! `DisplayService` — the one object the menu talks to: read state, apply a change, keep / revert, undo.
//! Generic over the OS layer, so every rule here is tested against the fake.

use crate::error::{DisplayError, Result};
use crate::fields::{mode_text, rates_for, resolve_fields};
use crate::os::DisplayOs;
use crate::picture::{ddc_value_for_percent, vibrance_level, vibrance_percent};
use crate::presets::Preset;
use crate::types::*;
use std::time::{Duration, Instant};

/// The keep bar's countdown (DESIGN §3.2.2: a ring that drains over 10 s).
pub const KEEP_SECONDS: u64 = 10;

/// A mode change waiting for Keep / Revert.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingKeep {
    /// Per monitor: the mode to go back to. Applying again while the bar is up keeps the ORIGINAL here.
    pub originals: Vec<(MonitorId, Mode)>,
    /// Per monitor: the mode the last Apply set (what Keep stores, scaling included).
    pub applied: Vec<(MonitorId, Mode)>,
    /// When it reverts by itself.
    pub deadline: Instant,
}

/// What Apply did.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    pub monitor: MonitorId,
    pub mode: Mode,
    /// What Revert / the countdown goes back to.
    pub revert_to: Mode,
    pub deadline: Instant,
}

/// What a revert did (for the toast "Not kept, back to 1920 × 1080 · 165 Hz" / "Back to …").
#[derive(Clone, Debug, PartialEq)]
pub struct Reverted {
    pub restored: Vec<(MonitorId, Mode)>,
    /// True when the countdown ran out (vs. the Revert button / Esc).
    pub timed_out: bool,
}

/// One undoable change (every change remembers the old value).
#[derive(Clone, Debug, PartialEq)]
pub enum UndoEntry {
    /// A kept mode change: go back to `previous`.
    Mode { monitor: MonitorId, previous: Mode },
    /// Main display moved: `previous` was main before.
    Main { previous: MonitorId },
    Dpi { monitor: MonitorId, previous: u32 },
    Ddc { monitor: MonitorId, vcp: Vcp, previous: u32 },
    Vibrance { monitor: MonitorId, previous_level: i32 },
}

pub struct DisplayService<O: DisplayOs> {
    os: O,
    keep_for: Duration,
    pending: Option<PendingKeep>,
    undo: Vec<UndoEntry>,
    /// Order 042: the last mode this service set on each monitor. Windows' read-back of a path does not carry the GPU
    /// scaling picked with Apply (the owner's test 2: Stretch / Black bars came back as Keep aspect), so a monitor that still
    /// shows that size and rate is reported with the scaling that was set.
    set_modes: Vec<(MonitorId, Mode)>,
}

impl<O: DisplayOs> DisplayService<O> {
    pub fn new(os: O) -> Self {
        Self { os, keep_for: Duration::from_secs(KEEP_SECONDS), pending: None, undo: Vec::new(), set_modes: Vec::new() }
    }

    /// Countdown length (10 s by default; tests use short ones).
    pub fn with_keep_duration(mut self, d: Duration) -> Self {
        self.keep_for = d;
        self
    }

    pub fn os(&self) -> &O {
        &self.os
    }
    pub fn os_mut(&mut self) -> &mut O {
        &mut self.os
    }

    // ---------- read ----------

    /// The selector: every monitor in number order (name, main, position, DPI, HDR, applied mode).
    pub fn monitors(&self) -> Result<Vec<MonitorInfo>> {
        Ok(self.os.monitors()?.into_iter().map(|m| self.with_set_scaling(m)).collect())
    }

    /// `m` with the scaling this service set on it while it still shows the mode that was set (see `set_modes`).
    fn with_set_scaling(&self, mut m: MonitorInfo) -> MonitorInfo {
        if let Some((_, s)) = self.set_modes.iter().find(|(id, _)| *id == m.id) {
            let c = m.current;
            if (c.width, c.height, c.refresh) == (s.width, s.height, s.refresh) {
                m.current.scaling = s.scaling;
            }
        }
        m
    }

    /// The OS's apply_mode, remembering the mode it set (`set_modes`).
    fn set_mode(&mut self, id: &MonitorId, mode: &Mode, save: bool) -> Result<()> {
        self.os.apply_mode(id, mode, save)?;
        self.set_modes.retain(|(m, _)| m != id);
        self.set_modes.push((id.clone(), *mode));
        Ok(())
    }

    pub fn monitor(&self, id: &MonitorId) -> Result<MonitorInfo> {
        self.monitors()?.into_iter().find(|m| &m.id == id).ok_or_else(|| DisplayError::MonitorNotFound(id.clone()))
    }

    /// The monitor's real modes, sorted: biggest first, then fastest.
    pub fn modes(&self, id: &MonitorId) -> Result<Vec<VideoMode>> {
        let mut v = self.os.modes(id)?;
        v.sort_by(|a, b| {
            (b.width as u64 * b.height as u64).cmp(&(a.width as u64 * a.height as u64)).then(b.width.cmp(&a.width)).then(b.refresh.cmp(&a.refresh))
        });
        v.dedup();
        Ok(v)
    }

    /// Selector label, e.g. "1 · DELL 27″" (number · brand · inches).
    pub fn selector_label(m: &MonitorInfo) -> String {
        let brand = m.name.split_whitespace().next().unwrap_or("Monitor");
        match m.diagonal_inches {
            Some(d) => format!("{} · {} {}″", m.number, brand, d.round() as u32),
            None => format!("{} · {}", m.number, brand),
        }
    }

    /// Selector tooltip, e.g. "DELL S2721DGF · 1920 × 1080 · 165 Hz".
    pub fn selector_tooltip(&self, m: &MonitorInfo) -> String {
        let rates = self.os.modes(&m.id).map(|v| rates_for(&v, m.current.width, m.current.height)).unwrap_or_default();
        format!("{} · {}", if m.name.is_empty() { "Monitor" } else { &m.name }, mode_text(&m.current, &rates))
    }

    // ---------- apply + keep / revert ----------

    /// Apply from the fields: W/H clamped, Hz snapped to a rate the monitor reports, then the keep countdown starts.
    pub fn apply_fields(&mut self, id: &MonitorId, width: u32, height: u32, hz_typed: f64, scaling: GpuScaling, now: Instant) -> Result<Applied> {
        let modes = self.os.modes(id)?;
        let mode = resolve_fields(&modes, width, height, hz_typed, scaling)?;
        self.apply_mode(id, mode, now)
    }

    /// One click on a preset chip: its values (Hz snapped to this monitor) + Apply, with the keep bar.
    pub fn apply_preset(&mut self, id: &MonitorId, preset: &Preset, now: Instant) -> Result<Applied> {
        let modes = self.os.modes(id)?;
        let mode = preset.resolve(&modes)?;
        self.apply_mode(id, mode, now)
    }

    /// Applies an exact mode and (re)starts the keep countdown. The revert target stays the mode from BEFORE the
    /// first Apply while the bar is up.
    pub fn apply_mode(&mut self, id: &MonitorId, mode: Mode, now: Instant) -> Result<Applied> {
        let before = self.monitor(id)?.current;
        // Not stored as Windows' saved setting yet — only Keep does that (a crash during the countdown comes back to
        // the old mode).
        self.set_mode(id, &mode, false)?;
        let deadline = now + self.keep_for;
        let p = self.pending.get_or_insert(PendingKeep { originals: Vec::new(), applied: Vec::new(), deadline });
        p.deadline = deadline;
        p.applied.retain(|(m, _)| m != id);
        p.applied.push((id.clone(), mode));
        if !p.originals.iter().any(|(m, _)| m == id) {
            p.originals.push((id.clone(), before));
        }
        let revert_to = p.originals.iter().find(|(m, _)| m == id).map(|(_, md)| *md).unwrap_or(before);
        // Report what Windows actually shows now (read back), not what was asked.
        let shown = self.monitor(id).map(|m| m.current).unwrap_or(mode);
        Ok(Applied { monitor: id.clone(), mode: shown, revert_to, deadline })
    }

    pub fn pending(&self) -> Option<&PendingKeep> {
        self.pending.as_ref()
    }

    /// Seconds shown in the ring ("Reverting in N s"), rounded up; `None` when no bar is up.
    pub fn keep_seconds_left(&self, now: Instant) -> Option<u64> {
        self.pending.as_ref().map(|p| {
            let left = p.deadline.saturating_duration_since(now);
            left.as_millis().div_ceil(1000) as u64
        })
    }

    /// Keep (button / Enter): the change stays; it becomes undoable. Returns the KEPT modes per monitor (what Apply set:
    /// Windows took it exactly - real Apply is strict - and its read-back doesn't carry the GPU scaling),
    /// for `PresetList::note_kept` (a preset kept once may then be used by automatic rules on that monitor).
    /// Only now is the mode stored as Windows' saved display setting. If Windows refuses to store it, the bar stays up
    /// (the countdown still reverts) and the error is returned.
    pub fn keep(&mut self) -> Result<Vec<(MonitorId, Mode)>> {
        let p = self.pending.take().ok_or(DisplayError::NoPendingChange)?;
        for (m, mode) in &p.applied {
            if let Err(e) = self.os.save_current(m, mode) {
                self.pending = Some(p);
                return Err(e);
            }
        }
        let mut kept = Vec::new();
        for (m, prev) in &p.originals {
            self.undo.push(UndoEntry::Mode { monitor: m.clone(), previous: *prev });
            if let Some((_, mode)) = p.applied.iter().find(|(a, _)| a == m) {
                kept.push((m.clone(), *mode));
            }
        }
        Ok(kept)
    }

    /// Revert (button / Esc): back to the original mode(s).
    pub fn revert(&mut self) -> Result<Reverted> {
        self.revert_inner(false)
    }

    fn revert_inner(&mut self, timed_out: bool) -> Result<Reverted> {
        let p = self.pending.take().ok_or(DisplayError::NoPendingChange)?;
        let mut first_err = None;
        for (m, mode) in &p.originals {
            if let Err(e) = self.set_mode(m, mode, false) {
                first_err.get_or_insert(e);
            }
        }
        match first_err {
            // Keep the bar so the user can try again; the original stays the target.
            Some(e) => {
                self.pending = Some(p);
                Err(e)
            }
            None => Ok(Reverted { restored: p.originals, timed_out }),
        }
    }

    /// Called by the countdown (see `keep::KeepTimer`) or any time: reverts when the deadline has passed.
    /// Keeps counting and reverting even while the flyout is closed (DESIGN: the safe choice).
    pub fn tick(&mut self, now: Instant) -> Option<Result<Reverted>> {
        let due = self.pending.as_ref().map(|p| now >= p.deadline).unwrap_or(false);
        if due { Some(self.revert_inner(true)) } else { None }
    }

    /// Applies a mode with no keep bar (per-app switching does this; it restores by itself when the app closes).
    /// Never stored as Windows' saved setting: a crash during the game comes back to the user's own mode.
    pub fn apply_mode_now(&mut self, id: &MonitorId, mode: Mode) -> Result<Mode> {
        let before = self.monitor(id)?.current;
        self.set_mode(id, &mode, false)?;
        Ok(before)
    }

    // ---------- main display ----------

    /// "Main display" switch turned on for `id`. Turning it off on the main one is not a change (it only nudges):
    /// returns Ok(false) when `id` already is main.
    pub fn set_main(&mut self, id: &MonitorId) -> Result<bool> {
        let mons = self.os.monitors()?;
        let target = mons.iter().find(|m| &m.id == id).ok_or_else(|| DisplayError::MonitorNotFound(id.clone()))?;
        if target.is_main {
            return Ok(false);
        }
        let previous = mons.iter().find(|m| m.is_main).map(|m| m.id.clone());
        self.os.set_main(id)?;
        if let Some(previous) = previous {
            self.undo.push(UndoEntry::Main { previous });
        }
        Ok(true)
    }

    /// Toast text after a main switch: "Main display: 2 · LG 24GL600F".
    pub fn main_toast(m: &MonitorInfo) -> String {
        format!("Main display: {} · {}", m.number, m.name)
    }

    // ---------- Windows scaling % ----------

    pub fn set_dpi_percent(&mut self, id: &MonitorId, percent: u32) -> Result<()> {
        let m = self.monitor(id)?;
        let dpi = m.dpi.ok_or(DisplayError::DpiNotOffered(percent))?;
        if !dpi.allowed_percent.contains(&percent) {
            return Err(DisplayError::DpiNotOffered(percent));
        }
        if dpi.current_percent == percent {
            return Ok(());
        }
        self.os.set_dpi_percent(id, percent)?;
        self.undo.push(UndoEntry::Dpi { monitor: id.clone(), previous: dpi.current_percent });
        Ok(())
    }

    // ---------- Picture ----------

    /// Reads brightness / contrast (live, from the monitor) and vibrance (live, from the driver).
    pub fn picture(&mut self, id: &MonitorId) -> Result<PictureState> {
        let b = self.ddc_get(id, Vcp::Brightness);
        let c = self.ddc_get(id, Vcp::Contrast);
        let ddc = match (&b, &c) {
            (Err(DisplayError::DdcExcludedModel), _) => DdcState::ExcludedModel,
            (Err(DisplayError::DdcBlockedAfterCrash), _) | (_, Err(DisplayError::DdcBlockedAfterCrash)) => DdcState::BlockedAfterCrash,
            (Ok(_), _) | (_, Ok(_)) => DdcState::Answers,
            _ => DdcState::NoAnswer,
        };
        if let (Err(e @ DisplayError::MonitorNotFound(_)), _) = (&b, &c) {
            return Err(e.clone());
        }
        let vib = self.os.vibrance_get(id).ok();
        Ok(PictureState {
            ddc,
            brightness: b.ok(),
            contrast: c.ok(),
            vibrance_percent: vib.as_ref().map(vibrance_percent),
            vibrance_vendor: vib.map(|v| v.vendor),
        })
    }

    /// Every DDC read goes through here: a model on the exclusion list is never asked (the monitor must still exist).
    fn ddc_get(&mut self, id: &MonitorId, vcp: Vcp) -> Result<VcpValue> {
        if crate::picture::ddc_excluded(id) {
            self.monitor(id)?;
            return Err(DisplayError::DdcExcludedModel);
        }
        self.os.ddc_get(id, vcp)
    }

    /// Brightness / Contrast slider (0–100 %), applied live over DDC/CI.
    pub fn set_ddc_percent(&mut self, id: &MonitorId, vcp: Vcp, percent: u8) -> Result<()> {
        self.set_ddc_percent_change(id, vcp, percent).map(|_| ())
    }

    /// [`Self::set_ddc_percent`] that says what it changed: `Some((before, after))` (the monitor's own values, with its
    /// max) when a value was written, `None` when it already was there (the app's change log, Order 036).
    pub fn set_ddc_percent_change(&mut self, id: &MonitorId, vcp: Vcp, percent: u8) -> Result<Option<(VcpValue, VcpValue)>> {
        let cur = self.ddc_get(id, vcp)?;
        let value = ddc_value_for_percent(cur.max, percent);
        if value == cur.current {
            return Ok(None);
        }
        self.os.ddc_set(id, vcp, value)?;
        self.push_undo_coalesced(UndoEntry::Ddc { monitor: id.clone(), vcp, previous: cur.current });
        Ok(Some((cur, VcpValue { current: value, max: cur.max })))
    }

    /// Vibrance slider (0–100 %, 50 = normal), applied live through the GPU driver.
    pub fn set_vibrance_percent(&mut self, id: &MonitorId, percent: u8) -> Result<()> {
        self.set_vibrance_percent_change(id, percent).map(|_| ())
    }

    /// [`Self::set_vibrance_percent`] that says what it changed: `Some((before, after))` (the driver's levels) when a level
    /// was written, `None` when it already was there (the app's change log, Order 036).
    pub fn set_vibrance_percent_change(&mut self, id: &MonitorId, percent: u8) -> Result<Option<(VibranceRaw, VibranceRaw)>> {
        let raw = self.os.vibrance_get(id)?;
        let level = vibrance_level(&raw, percent);
        if level == raw.current {
            return Ok(None);
        }
        self.os.vibrance_set(id, level)?;
        self.push_undo_coalesced(UndoEntry::Vibrance { monitor: id.clone(), previous_level: raw.current });
        Ok(Some((raw, VibranceRaw { current: level, ..raw })))
    }

    /// Vibrance set by per-app switching: no undo entry (it switches back by itself when the app closes).
    pub fn set_vibrance_percent_auto(&mut self, id: &MonitorId, percent: u8) -> Result<()> {
        let raw = self.os.vibrance_get(id)?;
        let level = vibrance_level(&raw, percent);
        if level != raw.current {
            self.os.vibrance_set(id, level)?;
        }
        Ok(())
    }

    /// Current vibrance in % (for saving before a per-app change).
    pub fn vibrance_percent(&mut self, id: &MonitorId) -> Result<u8> {
        Ok(vibrance_percent(&self.os.vibrance_get(id)?))
    }

    /// A slider drag sends many values; one drag = one undo step (keep the oldest "previous").
    fn push_undo_coalesced(&mut self, e: UndoEntry) {
        let same = match (self.undo.last(), &e) {
            (Some(UndoEntry::Ddc { monitor: a, vcp: va, .. }), UndoEntry::Ddc { monitor: b, vcp: vb, .. }) => a == b && va == vb,
            (Some(UndoEntry::Vibrance { monitor: a, .. }), UndoEntry::Vibrance { monitor: b, .. }) => a == b,
            _ => false,
        };
        if !same {
            self.undo.push(e);
        }
    }

    // ---------- undo ----------

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn undo_stack(&self) -> &[UndoEntry] {
        &self.undo
    }

    /// Switches the last change back. A mode change waiting for Keep is reverted instead.
    pub fn undo(&mut self) -> Result<UndoEntry> {
        if self.pending.is_some() {
            let r = self.revert()?;
            let (monitor, previous) = r.restored.into_iter().next().ok_or(DisplayError::NothingToUndo)?;
            return Ok(UndoEntry::Mode { monitor, previous });
        }
        let e = self.undo.pop().ok_or(DisplayError::NothingToUndo)?;
        let res = match &e {
            UndoEntry::Mode { monitor, previous } => self.set_mode(monitor, previous, true),
            UndoEntry::Main { previous } => self.os.set_main(previous),
            UndoEntry::Dpi { monitor, previous } => self.os.set_dpi_percent(monitor, *previous),
            UndoEntry::Ddc { monitor, vcp, previous } => self.os.ddc_set(monitor, *vcp, *previous),
            UndoEntry::Vibrance { monitor, previous_level } => self.os.vibrance_set(monitor, *previous_level),
        };
        if let Err(err) = res {
            self.undo.push(e);
            return Err(err);
        }
        Ok(e)
    }

    /// Whether a change needs administrator rights (never on Windows for this tab; the fake can say yes).
    pub fn needs_admin(&self, kind: ChangeKind) -> bool {
        self.os.needs_admin(kind)
    }
}
