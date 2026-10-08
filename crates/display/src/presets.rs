//! Presets (DESIGN §3.2.4): resolution · Hz · scaling, global (not per monitor), no names, always sorted:
//! highest resolution first (pixels, then width), then Hz, ties Stretch → Black bars → Keep aspect.

use crate::error::{DisplayError, Result};
use crate::fields::{rates_for, snap_hz};
use crate::types::{GpuScaling, Mode, MonitorId, RefreshRate, VideoMode};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// Stable id of a preset (rules point at it; a deleted preset leaves its rules on "Choose a preset").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PresetId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    pub id: PresetId,
    pub width: u32,
    pub height: u32,
    /// The exact rate it was saved with; snapped to each monitor's own rates when applied.
    pub refresh: RefreshRate,
    pub scaling: GpuScaling,
    /// Monitors this preset was applied on AND kept (Keep pressed). Only there may an automatic rule use it
    /// (NOTE_004_01: the monitor is known to show it; automatic switches never show the keep bar).
    #[serde(default)]
    pub kept_on: Vec<MonitorId>,
}

impl Preset {
    /// The mode this preset gives on a monitor with `modes`: Hz snapped to that monitor's rates at that size.
    pub fn resolve(&self, modes: &[VideoMode]) -> Result<Mode> {
        crate::fields::resolve_fields(modes, self.width, self.height, self.refresh.hz(), self.scaling)
    }
}

fn order(a: &Preset, b: &Preset) -> Ordering {
    let pa = a.width as u64 * a.height as u64;
    let pb = b.width as u64 * b.height as u64;
    pb.cmp(&pa)
        .then(b.width.cmp(&a.width))
        .then(b.refresh.cmp(&a.refresh))
        .then(a.scaling.rank().cmp(&b.scaling.rank()))
        .then(a.id.cmp(&b.id))
}

/// The preset list, kept in display order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PresetList {
    items: Vec<Preset>,
    next_id: u64,
}

impl PresetList {
    pub fn new() -> Self {
        Self::default()
    }

    /// Chips in display order.
    pub fn items(&self) -> &[Preset] {
        &self.items
    }

    pub fn get(&self, id: PresetId) -> Option<&Preset> {
        self.items.iter().find(|p| p.id == id)
    }

    /// "+" chip: saves the fields as a preset. The Hz is snapped to the monitor's rates first (`modes` = the selected
    /// monitor's list). A duplicate (same W, H, scaling and snapped Hz) is refused with the existing chip's index
    /// (the UI pulses it and toasts "Already a preset"). Returns the new preset's id.
    pub fn add(&mut self, width: u32, height: u32, hz_typed: f64, scaling: GpuScaling, modes: &[VideoMode]) -> Result<PresetId> {
        let rates = rates_for(modes, width, height);
        let refresh = snap_hz(hz_typed, &rates).unwrap_or_else(|| RefreshRate::new((hz_typed * 1000.0).round() as u32, 1000));
        if let Some(i) = self.items.iter().position(|p| {
            p.width == width
                && p.height == height
                && p.scaling == scaling
                && snap_hz(p.refresh.hz(), &rates).unwrap_or(p.refresh) == refresh
        }) {
            return Err(DisplayError::DuplicatePreset(i));
        }
        self.next_id += 1;
        let id = PresetId(self.next_id);
        self.items.push(Preset { id, width, height, refresh, scaling, kept_on: Vec::new() });
        self.items.sort_by(order);
        Ok(id)
    }

    /// Hover × → delete. Returns the removed preset (so it can be undone with `restore`).
    pub fn remove(&mut self, id: PresetId) -> Result<Preset> {
        let i = self.items.iter().position(|p| p.id == id).ok_or(DisplayError::PresetNotFound)?;
        Ok(self.items.remove(i))
    }

    /// Puts a removed preset back (undo of `remove`), with its old id.
    pub fn restore(&mut self, p: Preset) {
        if self.get(p.id).is_none() {
            self.next_id = self.next_id.max(p.id.0);
            self.items.push(p);
            self.items.sort_by(order);
        }
    }

    /// After Keep: every preset that gives exactly the kept mode on this monitor becomes usable by automatic rules
    /// there. Call with what `DisplayService::keep()` returned. Returns how many presets were marked.
    pub fn note_kept(&mut self, monitor: &MonitorId, kept: &Mode, modes: &[VideoMode]) -> usize {
        let mut n = 0;
        for p in self.items.iter_mut() {
            if p.resolve(modes).map(|m| m == *kept).unwrap_or(false) && !p.kept_on.contains(monitor) {
                p.kept_on.push(monitor.clone());
                n += 1;
            }
        }
        n
    }

    /// True when the preset was applied and kept on this monitor (so a rule may use it there).
    pub fn is_kept_on(&self, id: PresetId, monitor: &MonitorId) -> bool {
        self.get(id).map(|p| p.kept_on.contains(monitor)).unwrap_or(false)
    }

    /// The chip that equals the monitor's applied mode (gets the ✓): same W, H, scaling and the preset's Hz snapped to
    /// this monitor equals the applied rate.
    pub fn matching(&self, applied: &Mode, modes: &[VideoMode]) -> Option<PresetId> {
        self.items
            .iter()
            .find(|p| p.resolve(modes).map(|m| m == *applied).unwrap_or(false))
            .map(|p| p.id)
    }
}
