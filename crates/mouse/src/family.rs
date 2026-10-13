//! The driver layer (Order 104): one file per mouse family - `pulsar.rs`, `logitech.rs`, `razer.rs`, `steelseries.rs` - each
//! a port of OpenMouse's driver for that family (github.com/OpenMouse-Project/openmouse, AGPL-3.0-or-later, at
//! [`OPENMOUSE_COMMIT`]; its protocol package @openmouse/protocol [`OPENMOUSE_PROTOCOL`]). No JS engine: the bytes are
//! built here, in Rust.
//!
//! Scope (Order 104): DPI, polling rate, lift-off distance and battery ONLY, and only the commands OpenMouse itself sends
//! for them - never a firmware, flash-update or reset command. Opening the tab = reads only; a write only from a click.
//!
//! The pattern every family file follows (copy it for a new family):
//! 1. the codec: pure functions that build every request and decode every answer - tests compare them with the bytes
//!    OpenMouse sends for the same setting (encode) and with answers decoded the way OpenMouse decodes them (decode);
//! 2. `matches(hid, vid, pid) -> Option<Matched>`: the device table - VID:PID -> model + family, from OpenMouse's
//!    SUPPORTED list only (not "likely" / "pending") - and the interface the family talks to;
//! 3. a unit struct implementing [`Driver`]: `read` (read requests only) and `write` (one setting, read back);
//! 4. `read_request_allowed(out)`: the family's allow-list for the read-only proof on a real mouse;
//! 5. one line in [`FAMILIES`] (and its arm in [`driver`] / [`read_request_allowed`]).

use crate::error::Result;
use crate::os::{HidInfo, HidTransfer};

/// The OpenMouse source this layer is ported from (`tools/openmouse-sync.md`: how to re-sync - never automatic).
pub const OPENMOUSE_COMMIT: &str = "c64437f65a1b9a992a56b48c330662c40a371085";
/// The protocol package that commit pins (package-lock.json), where the family drivers live.
pub const OPENMOUSE_PROTOCOL: &str = "@openmouse/protocol 0.40.1";

/// Which driver a mouse gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    /// Pulsar report-8 "cMouse" protocol (X2 CrazyLight and the other flash-protocol Pulsars) - `pulsar.rs`
    PulsarCmouse,
    /// Pulsar XS-1: 64-byte feature reports (X3 family) - `pulsar.rs`
    PulsarXs1,
    /// Logitech HID++ 2.0 - `logitech.rs`
    Logitech,
    /// Razer 90-byte feature reports - `razer.rs`
    Razer,
    /// SteelSeries (per-model command sets) - `steelseries.rs`
    SteelSeries,
    /// ATK / VXE - `atk.rs`
    Atk,
    /// Endgame Gear - `endgame.rs`
    Endgame,
    /// Lamzu - `lamzu.rs`
    Lamzu,
    /// WLMouse - `wlmouse.rs`
    WlMouse,
    /// Redragon - `redragon.rs`
    Redragon,
    /// Attack Shark - `attackshark.rs`
    AttackShark,
    /// GearHub (Lingbao M5 Pro, Attack Shark R2) - `gearhub.rs`
    GearHub,
    /// VAXEE - `vaxee.rs`
    Vaxee,
    /// Corsair - `corsair.rs`
    Corsair,
    /// Glorious - `glorious.rs`
    Glorious,
    /// Ninjutso - `ninjutso.rs`
    Ninjutso,
    /// Finalmouse - `finalmouse.rs`
    Finalmouse,
    /// Zaunkoenig - `zaunkoenig.rs`
    Zaunkoenig,
    /// Orbital - `orbital.rs`
    Orbital,
    /// RAWM - `rawm.rs`
    Rawm,
    /// Mchose - `mchose.rs`
    Mchose,
    /// Delux - `delux.rs`
    Delux,
    /// Dareu - `dareu.rs`
    Dareu,
    /// Keychron - `keychron.rs`
    Keychron,
    /// Incott - `incott.rs`
    Incott,
    /// K-snake / Noir Gear - `ksnake.rs`
    Ksnake,
    /// Microsoft - `microsoft.rs`
    Microsoft,
    /// moddoMOUSE - `moddo.rs`
    Moddo,
    /// Teevolution - `teevolution.rs`
    Teevolution,
    /// Wallhack - `wallhack.rs`
    Wallhack,
    /// VGN - `vgn.rs`
    Vgn,
    /// ASUS - `asus.rs`
    Asus,
    /// Ryunix - `ryunix.rs`
    Ryunix,
    /// Rapoo - `rapoo.rs`
    Rapoo,
}

/// What a family's matcher found for one mouse: its model (when the table names it), the interface to talk to, and the
/// family's own per-model detail (`variant`, e.g. which SteelSeries command set).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Matched {
    pub family: Family,
    /// the model's name from the device table (`None` = the table does not pin one id to one model, e.g. a shared receiver)
    pub model: Option<&'static str>,
    /// `Some` = the id says wired (`false`) or wireless (`true`)
    pub wireless: Option<bool>,
    pub config: HidInfo,
    pub variant: u16,
}

/// The DPI values a mouse can store: bands of (from, to, step), ascending.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DpiCaps {
    pub bands: Vec<(u32, u32, u32)>,
}

impl DpiCaps {
    pub fn uniform(min: u32, max: u32, step: u32) -> Self {
        Self { bands: vec![(min, max, step.max(1))] }
    }
    pub fn min(&self) -> u32 {
        self.bands.first().map(|b| b.0).unwrap_or(0)
    }
    pub fn max(&self) -> u32 {
        self.bands.last().map(|b| b.1).unwrap_or(0)
    }
    /// Can the mouse store exactly this value?
    pub fn allows(&self, dpi: u32) -> bool {
        self.bands.iter().any(|(a, b, s)| (*a..=*b).contains(&dpi) && (dpi - a).is_multiple_of(*s))
    }
    /// A typed value → the nearest value the mouse can store (ties go up), clamped to the range.
    pub fn nearest(&self, typed: u32) -> u32 {
        let mut best: Option<u32> = None;
        for (a, b, s) in &self.bands {
            let t = typed.clamp(*a, *b);
            let lo = a + (t - a) / s * s;
            for c in [lo, (lo + s).min(*b)] {
                if !self.allows(c) {
                    continue;
                }
                best = Some(match best {
                    Some(o) if o.abs_diff(typed) < c.abs_diff(typed) || (o.abs_diff(typed) == c.abs_diff(typed) && o > c) => o,
                    _ => c,
                });
            }
        }
        best.unwrap_or(typed)
    }
    /// One wheel notch over the Custom box (Order 045: ±50, or the mouse's own step where it is coarser).
    pub fn wheel(&self, v: u32, up: bool) -> u32 {
        let step = self.bands.iter().find(|(a, b, _)| (*a..=*b).contains(&v)).map(|b| b.2).unwrap_or(50);
        let d = step.max(50);
        self.nearest(if up { v + d } else { v.saturating_sub(d) })
    }
}

/// What a mouse can do, as its driver declares it (the Mouse page shows only these rows / chips).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Caps {
    /// `None` = DPI can't be set
    pub dpi: Option<DpiCaps>,
    /// polling rates (Hz) that can be SET, ascending; empty = no polling control
    pub polling: Vec<u32>,
    /// lift-off distances (tenths of a mm) that can be set, ascending; empty = none
    pub lift_off: Vec<u32>,
    /// the mouse reports a battery level
    pub battery: bool,
}

/// One setting a click changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Dpi(u32),
    /// Hz
    Polling(u32),
    /// tenths of a millimetre
    LiftOff(u32),
}

impl Setting {
    /// The undo key's name ("dpi", "polling", "lift_off").
    pub fn key(&self) -> &'static str {
        match self {
            Setting::Dpi(_) => "dpi",
            Setting::Polling(_) => "polling",
            Setting::LiftOff(_) => "lift_off",
        }
    }
    pub fn value(&self) -> u32 {
        match self {
            Setting::Dpi(v) | Setting::Polling(v) | Setting::LiftOff(v) => *v,
        }
    }
    /// The same kind of setting with another value.
    pub fn with(&self, v: u32) -> Setting {
        match self {
            Setting::Dpi(_) => Setting::Dpi(v),
            Setting::Polling(_) => Setting::Polling(v),
            Setting::LiftOff(_) => Setting::LiftOff(v),
        }
    }
}

/// What a supported mouse reports (read requests only).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct OnMouse {
    /// the model as the mouse itself names it, when its family can tell
    pub model: Option<String>,
    /// the mouse is awake and answering (behind a receiver it may be asleep)
    pub online: bool,
    pub battery_percent: Option<u8>,
    pub charging: Option<bool>,
    pub dpi: Option<u32>,
    pub polling_hz: Option<u32>,
    /// tenths of a millimetre (10 = 1 mm, 20 = 2 mm, 7 = 0.7 mm)
    pub lift_off: Option<u32>,
    /// `Some` when the mouse's answer says how it is linked
    pub wireless: Option<bool>,
    pub caps: Caps,
    /// Order 110: the extra settings the driver offers (Debounce time, Motion Sync, Angle Snap, Ripple Control, Turbo Mode -
    /// only those its OpenMouse client sends), in the card's order, each with its value
    pub extras: Vec<ExtraRow>,
}

impl OnMouse {
    /// The value read for one extra setting.
    pub fn extra(&self, e: Extra) -> Option<i32> {
        self.extras.iter().find(|r| r.extra == e).and_then(|r| r.value)
    }
}

/// One of the five settings Order 110 adds under Lift-off distance (the owner, A_110_02), in the card's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Extra {
    /// click debounce, ms (a range: the mouse's own limits)
    Debounce,
    MotionSync,
    AngleSnap,
    RippleControl,
    /// Pulsar's turbo / highest-performance mode (the Compx performanceState byte, or a client's turboMode)
    Turbo,
}

impl Extra {
    /// The card's order.
    pub const ALL: [Extra; 5] = [Extra::Debounce, Extra::MotionSync, Extra::AngleSnap, Extra::RippleControl, Extra::Turbo];

    /// The row's words on the card (Pulsar's own names).
    pub fn label(self) -> &'static str {
        match self {
            Extra::Debounce => "Debounce time",
            Extra::MotionSync => "Motion Sync",
            Extra::AngleSnap => "Angle Snap",
            Extra::RippleControl => "Ripple Control",
            Extra::Turbo => "Turbo Mode",
        }
    }
}

/// What an extra setting takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// on (1) / off (0)
    Switch,
    /// any value min..=max in steps; `unit` follows the number ("ms")
    Range { min: i32, max: i32, step: i32, unit: &'static str },
    /// only these values, ascending (e.g. ATK's debounce 0, 1, 2, 4, 8, 15, 20 ms)
    Steps { values: Vec<i32>, unit: &'static str },
}

impl Kind {
    /// Does the mouse take this value?
    pub fn allows(&self, v: i32) -> bool {
        match self {
            Kind::Switch => v == 0 || v == 1,
            Kind::Range { min, max, step, .. } => (*min..=*max).contains(&v) && (v - min) % (*step).max(1) == 0,
            Kind::Steps { values, .. } => values.contains(&v),
        }
    }
    /// A value's words ("4 ms", "On").
    pub fn words(&self, v: i32) -> String {
        match self {
            Kind::Switch => if v != 0 { "On" } else { "Off" }.to_string(),
            Kind::Range { unit: "", .. } | Kind::Steps { unit: "", .. } => v.to_string(),
            Kind::Range { unit, .. } | Kind::Steps { unit, .. } => format!("{v} {unit}"),
        }
    }
    /// The card's − / + from `v`: the next value the mouse takes below / above it (`None` at the end, or for a switch). A value
    /// read outside the range goes to the nearest end with either button (never a stuck row).
    pub fn step(&self, v: i32, up: bool) -> Option<i32> {
        match self {
            Kind::Switch => None,
            Kind::Range { min, max, .. } if !(*min..=*max).contains(&v) => Some(v.clamp(*min, *max)),
            Kind::Range { min, max, step, .. } => Some(if up { v + step } else { v - step }).filter(|t| (*min..=*max).contains(t)),
            Kind::Steps { values, .. } => match (values.first(), values.last()) {
                (Some(lo), Some(hi)) if v < *lo || v > *hi => Some(v.clamp(*lo, *hi)),
                _ => if up { values.iter().find(|x| **x > v) } else { values.iter().rev().find(|x| **x < v) }.copied(),
            },
        }
    }
}

/// One extra setting on the card: what it is, what it takes, its value as read (`None` = not answered).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtraRow {
    pub extra: Extra,
    pub kind: Kind,
    pub value: Option<i32>,
}

impl ExtraRow {
    pub fn new(extra: Extra, kind: Kind, value: Option<i32>) -> Self {
        Self { extra, kind, value }
    }
    pub fn switch(extra: Extra, value: Option<bool>) -> Self {
        Self { extra, kind: Kind::Switch, value: value.map(i32::from) }
    }
}

/// One change from a click on an extra row (Order 110). The driver writes it the way OpenMouse does and reads it back; the
/// page then reads the mouse again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// an extra setting → this value (a switch: 1 / 0)
    Extra(Extra, i32),
}

/// The error for a change the mouse's driver does not offer.
pub fn not_offered(c: Change) -> crate::error::Error {
    let Change::Extra(e, _) = c;
    crate::error::Error::range(e.label(), "this mouse has no such setting here".to_string())
}

/// Finds the row of `e` in a read and checks the value against its kind (the driver's guard before any byte leaves).
pub fn check_extra(extras: &[ExtraRow], e: Extra, v: i32) -> Result<()> {
    match extras.iter().find(|r| r.extra == e) {
        Some(r) if r.kind.allows(v) => Ok(()),
        Some(r) => Err(crate::error::Error::range(e.label(), format!("{} is not a value this mouse takes", r.kind.words(v)))),
        None => Err(not_offered(Change::Extra(e, v))),
    }
}

/// A change that went through: what the mouse has now and what it had before (for undo; `None` = could not be read).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Changed {
    pub now: u32,
    pub before: Option<u32>,
}

/// How hard to try reading a sleeping mouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadOptions {
    /// how many times to ask "online?" before giving up (the read-only proof uses 1: A_009_01 "do not loop")
    pub online_tries: u32,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self { online_tries: 150 }
    }
}

/// The one HID interface a driver talks through.
pub trait Port {
    /// One request → its answer (`[]` for the transfers that have none).
    fn exchange(&mut self, out: &[u8], how: &HidTransfer) -> Result<Vec<u8>>;
    /// A pause between two requests (a waking mouse, a receiver that wants a gap). Fakes return at once.
    fn pause_ms(&mut self, ms: u64);
    /// The interface (its report lengths).
    fn info(&self) -> &HidInfo;
}

/// One family's driver.
pub trait Driver {
    /// Everything the Mouse page shows - READ requests only (OpenMouse's own reads).
    fn read(&self, m: &Matched, port: &mut dyn Port, opt: ReadOptions) -> Result<OnMouse>;
    /// One setting, the way OpenMouse writes it, read back to confirm.
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed>;
    /// Order 110: one stage / extra change, the way OpenMouse writes it, read back to confirm. Families without it refuse
    /// before a byte leaves.
    fn change(&self, _m: &Matched, _port: &mut dyn Port, c: Change) -> Result<()> {
        Err(not_offered(c))
    }
}

/// The families, in OpenMouse's registry order (the first that matches wins).
pub const FAMILIES: [Family; 34] = [Family::PulsarXs1, Family::PulsarCmouse, Family::Logitech, Family::Razer, Family::SteelSeries, Family::Atk, Family::Endgame, Family::Lamzu, Family::WlMouse, Family::Redragon, Family::AttackShark, Family::GearHub, Family::Vaxee, Family::Corsair, Family::Glorious, Family::Ninjutso, Family::Finalmouse, Family::Zaunkoenig, Family::Orbital, Family::Rawm, Family::Mchose, Family::Delux, Family::Dareu, Family::Keychron, Family::Incott, Family::Ksnake, Family::Microsoft, Family::Moddo, Family::Teevolution, Family::Wallhack, Family::Vgn, Family::Asus, Family::Ryunix, Family::Rapoo];

/// The driver of a family.
pub fn driver(f: Family) -> &'static dyn Driver {
    match f {
        Family::PulsarCmouse => &crate::pulsar::Cmouse,
        Family::PulsarXs1 => &crate::pulsar::Xs1,
        Family::Logitech => &crate::logitech::Logitech,
        Family::Razer => &crate::razer::Razer,
        Family::SteelSeries => &crate::steelseries::SteelSeries,
        Family::Atk => &crate::atk::Atk,
        Family::Endgame => &crate::endgame::Endgame,
        Family::Lamzu => &crate::lamzu::Lamzu,
        Family::WlMouse => &crate::wlmouse::WlMouse,
        Family::Redragon => &crate::redragon::Redragon,
        Family::AttackShark => &crate::attackshark::AttackShark,
        Family::GearHub => &crate::gearhub::GearHub,
        Family::Vaxee => &crate::vaxee::Vaxee,
        Family::Corsair => &crate::corsair::Corsair,
        Family::Glorious => &crate::glorious::Glorious,
        Family::Ninjutso => &crate::ninjutso::Ninjutso,
        Family::Finalmouse => &crate::finalmouse::Finalmouse,
        Family::Zaunkoenig => &crate::zaunkoenig::Zaunkoenig,
        Family::Orbital => &crate::orbital::Orbital,
        Family::Rawm => &crate::rawm::Rawm,
        Family::Mchose => &crate::mchose::Mchose,
        Family::Delux => &crate::delux::Delux,
        Family::Dareu => &crate::dareu::Dareu,
        Family::Keychron => &crate::keychron::Keychron,
        Family::Incott => &crate::incott::Incott,
        Family::Ksnake => &crate::ksnake::Ksnake,
        Family::Microsoft => &crate::microsoft::Microsoft,
        Family::Moddo => &crate::moddo::Moddo,
        Family::Teevolution => &crate::teevolution::Teevolution,
        Family::Wallhack => &crate::wallhack::Wallhack,
        Family::Vgn => &crate::vgn::Vgn,
        Family::Asus => &crate::asus::Asus,
        Family::Ryunix => &crate::ryunix::Ryunix,
        Family::Rapoo => &crate::rapoo::Rapoo,
    }
}

/// Which family (if any) drives the device `vid:pid`, given all its interfaces.
pub fn find(hid: &[HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    let own: Vec<&HidInfo> = hid.iter().filter(|h| h.vid == vid && h.pid == pid).collect();
    FAMILIES.iter().find_map(|f| match f {
        Family::PulsarCmouse => crate::pulsar::matches_cmouse(&own, vid, pid),
        Family::PulsarXs1 => crate::pulsar::matches_xs1(&own, vid, pid),
        Family::Logitech => crate::logitech::matches(&own, vid, pid),
        Family::Razer => crate::razer::matches(&own, vid, pid),
        Family::SteelSeries => crate::steelseries::matches(&own, vid, pid),
        Family::Atk => crate::atk::matches(&own, vid, pid),
        Family::Endgame => crate::endgame::matches(&own, vid, pid),
        Family::Lamzu => crate::lamzu::matches(&own, vid, pid),
        Family::WlMouse => crate::wlmouse::matches(&own, vid, pid),
        Family::Redragon => crate::redragon::matches(&own, vid, pid),
        Family::AttackShark => crate::attackshark::matches(&own, vid, pid),
        Family::GearHub => crate::gearhub::matches(&own, vid, pid),
        Family::Vaxee => crate::vaxee::matches(&own, vid, pid),
        Family::Corsair => crate::corsair::matches(&own, vid, pid),
        Family::Glorious => crate::glorious::matches(&own, vid, pid),
        Family::Ninjutso => crate::ninjutso::matches(&own, vid, pid),
        Family::Finalmouse => crate::finalmouse::matches(&own, vid, pid),
        Family::Zaunkoenig => crate::zaunkoenig::matches(&own, vid, pid),
        Family::Orbital => crate::orbital::matches(&own, vid, pid),
        Family::Rawm => crate::rawm::matches(&own, vid, pid),
        Family::Mchose => crate::mchose::matches(&own, vid, pid),
        Family::Delux => crate::delux::matches(&own, vid, pid),
        Family::Dareu => crate::dareu::matches(&own, vid, pid),
        Family::Keychron => crate::keychron::matches(&own, vid, pid),
        Family::Incott => crate::incott::matches(&own, vid, pid),
        Family::Ksnake => crate::ksnake::matches(&own, vid, pid),
        Family::Microsoft => crate::microsoft::matches(&own, vid, pid),
        Family::Moddo => crate::moddo::matches(&own, vid, pid),
        Family::Teevolution => crate::teevolution::matches(&own, vid, pid),
        Family::Wallhack => crate::wallhack::matches(&own, vid, pid),
        Family::Vgn => crate::vgn::matches(&own, vid, pid),
        Family::Asus => crate::asus::matches(&own, vid, pid),
        Family::Ryunix => crate::ryunix::matches(&own, vid, pid),
        Family::Rapoo => crate::rapoo::matches(&own, vid, pid),
    })
}

/// The read-only proof's allow-list (A_009_01): may this request go to a real mouse of family `f`? Only the family's
/// READ requests pass; anything that changes the mouse is refused before a byte leaves.
pub fn read_request_allowed(f: Family, out: &[u8]) -> bool {
    match f {
        Family::PulsarCmouse => crate::pulsar::read_request_allowed(out),
        Family::PulsarXs1 => crate::pulsar::xs1_read_request_allowed(out),
        Family::Logitech => crate::logitech::read_request_allowed(out),
        Family::Razer => crate::razer::read_request_allowed(out),
        Family::SteelSeries => crate::steelseries::read_request_allowed(out),
        Family::Atk => crate::atk::read_request_allowed(out),
        Family::Endgame => crate::endgame::read_request_allowed(out),
        Family::Lamzu => crate::lamzu::read_request_allowed(out),
        Family::WlMouse => crate::wlmouse::read_request_allowed(out),
        Family::Redragon => crate::redragon::read_request_allowed(out),
        Family::AttackShark => crate::attackshark::read_request_allowed(out),
        Family::GearHub => crate::gearhub::read_request_allowed(out),
        Family::Vaxee => crate::vaxee::read_request_allowed(out),
        Family::Corsair => crate::corsair::read_request_allowed(out),
        Family::Glorious => crate::glorious::read_request_allowed(out),
        Family::Ninjutso => crate::ninjutso::read_request_allowed(out),
        Family::Finalmouse => crate::finalmouse::read_request_allowed(out),
        Family::Zaunkoenig => crate::zaunkoenig::read_request_allowed(out),
        Family::Orbital => crate::orbital::read_request_allowed(out),
        Family::Rawm => crate::rawm::read_request_allowed(out),
        Family::Mchose => crate::mchose::read_request_allowed(out),
        Family::Delux => crate::delux::read_request_allowed(out),
        Family::Dareu => crate::dareu::read_request_allowed(out),
        Family::Keychron => crate::keychron::read_request_allowed(out),
        Family::Incott => crate::incott::read_request_allowed(out),
        Family::Ksnake => crate::ksnake::read_request_allowed(out),
        Family::Microsoft => crate::microsoft::read_request_allowed(out),
        Family::Moddo => crate::moddo::read_request_allowed(out),
        Family::Teevolution => crate::teevolution::read_request_allowed(out),
        Family::Wallhack => crate::wallhack::read_request_allowed(out),
        Family::Vgn => crate::vgn::read_request_allowed(out),
        Family::Asus => crate::asus::read_request_allowed(out),
        Family::Ryunix => crate::ryunix::read_request_allowed(out),
        Family::Rapoo => crate::rapoo::read_request_allowed(out),
    }
}

/// "1 mm", "0.7 mm" - a lift-off chip's words.
pub fn lift_off_label(tenths: u32) -> String {
    if tenths.is_multiple_of(10) {
        format!("{} mm", tenths / 10)
    } else {
        format!("{}.{} mm", tenths / 10, tenths % 10)
    }
}

/// The polling chips the page shows: the (at most four) highest rates the mouse can be set to.
pub fn polling_chips(c: &Caps) -> Vec<u32> {
    let n = c.polling.len();
    c.polling[n.saturating_sub(4)..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pulsar() -> DpiCaps {
        DpiCaps { bands: vec![(50, 10000, 10), (10050, 26000, 50)] }
    }

    #[test]
    fn nearest_dpi_follows_the_bands() {
        let c = pulsar();
        assert_eq!(c.nearest(1234), 1230);
        assert_eq!(c.nearest(1235), 1240, "a tie goes up");
        assert_eq!(c.nearest(12345), 12350);
        assert_eq!(c.nearest(10020), 10000, "the gap between the bands: the nearer side");
        assert_eq!(c.nearest(10030), 10050);
        assert_eq!(c.nearest(1), 50);
        assert_eq!(c.nearest(99999), 26000);
        assert!(c.allows(10050) && !c.allows(10040) && !c.allows(10060));
        assert_eq!((c.min(), c.max()), (50, 26000));
        let u = DpiCaps::uniform(100, 25600, 50);
        assert_eq!(u.nearest(1234), 1250);
        assert_eq!(u.nearest(1224), 1200);
    }

    #[test]
    fn the_wheel_steps_50_or_the_mouses_own_step() {
        let c = pulsar();
        assert_eq!((c.wheel(800, true), c.wheel(800, false)), (850, 750), "Order 045: ±50");
        assert_eq!((c.wheel(10000, true), c.wheel(10050, false)), (10050, 10000));
        assert_eq!((c.wheel(26000, true), c.wheel(50, false)), (26000, 50), "stops at the ends");
        let h = DpiCaps::uniform(100, 25600, 100);
        assert_eq!((h.wheel(800, true), h.wheel(800, false)), (900, 700), "a 100-step mouse never sticks");
    }

    #[test]
    fn minus_and_plus_go_to_the_next_value_the_mouse_takes() {
        let r = Kind::Range { min: 0, max: 15, step: 1, unit: "ms" };
        assert_eq!((r.step(4, true), r.step(4, false), r.step(15, true), r.step(0, false)), (Some(5), Some(3), None, None));
        // ATK's debounce: only 0, 1, 2, 4, 8, 15, 20 ms (OpenMouse)
        let s = Kind::Steps { values: vec![0, 1, 2, 4, 8, 15, 20], unit: "ms" };
        assert_eq!((s.step(2, true), s.step(4, false), s.step(20, true), s.step(0, false)), (Some(4), Some(2), None, None));
        assert_eq!(s.step(3, true), Some(4), "a value between steps goes to the next one");
        assert_eq!(s.step(3, false), Some(2));
        assert!(s.allows(8) && !s.allows(3));
        assert_eq!(s.words(8), "8 ms");
        // a value the mouse reports outside its own range (flash 0xFF): either button brings it back to the nearest end
        assert_eq!((r.step(255, false), r.step(255, true), r.step(-3, true)), (Some(15), Some(15), Some(0)));
        assert_eq!((s.step(99, false), s.step(99, true)), (Some(20), Some(20)));
    }

    #[test]
    fn chips_and_labels() {
        let c = Caps { polling: vec![125, 250, 500, 1000, 2000, 4000, 8000], ..Default::default() };
        assert_eq!(polling_chips(&c), vec![1000, 2000, 4000, 8000]);
        let c = Caps { polling: vec![125, 500, 1000], ..Default::default() };
        assert_eq!(polling_chips(&c), vec![125, 500, 1000]);
        assert_eq!(lift_off_label(10), "1 mm");
        assert_eq!(lift_off_label(7), "0.7 mm");
    }
}
