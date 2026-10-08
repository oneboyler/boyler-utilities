//! The acceleration card's own state (DESIGN §3.4 "Mouse acceleration"): the curve popup, each curve's values (Gain and
//! Cap type included — switching curves never resets anything), the shared sens multiplier, presets, and the mapping onto
//! Raw Accel's settings exactly like Raw Accel's GUI does it (`AccelGUI.MakeSettingsFromFields` + `AccelTypeOptions.SetArgs`:
//! start from the C++ defaults, set only the fields the curve shows; Linear = classic with exponent 2; Output DPI =
//! sens × 1000; in whole mode the vertical args equal the horizontal ones).
//!
//! Calls made (also in the report):
//! - Rows whose DESIGN range starts at a value Raw Accel's own validation refuses start one step above it: Acceleration
//!   (0 → "acceleration must be positive") and Motivity (1 → "motivity must be greater than 1").
//! - Steps and decimals follow the approved drawing's `APAR` table (menu-v22; Order 019 replaced the earlier guesses).
//!   Motivity starts at 1.05 (the drawing's 1 + one .05 step: Raw Accel refuses 1).
//! - "Off" (a row's target, "Everywhere else", or the switch off) = Raw Accel's "Off" curve (noaccel) with sens 1.0 —
//!   plain 1:1; the driver stays installed and running.
//! - Raising Input offset to or past Cap: input while Cap type is Input / Both moves Cap: input to min(120, offset + 30),
//!   the same rule DESIGN gives for switching the cap type (Raw Accel refuses "cap < offset").

use super::args::{AccelArgs, AccelMode, CapMode, Profile, Vec2, NORMALIZED_DPI};
use super::switch::PresetId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The curve popup (Raw Accel's Look-up table is left out — DESIGN).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Curve {
    Linear,
    Classic,
    Natural,
    Jump,
    Synchronous,
    Power,
}

impl Curve {
    pub const ALL: [Curve; 6] = [Curve::Linear, Curve::Classic, Curve::Natural, Curve::Jump, Curve::Synchronous, Curve::Power];
    pub fn name(self) -> &'static str {
        match self {
            Curve::Linear => "Linear",
            Curve::Classic => "Classic",
            Curve::Natural => "Natural",
            Curve::Jump => "Jump",
            Curve::Synchronous => "Synchronous",
            Curve::Power => "Power",
        }
    }
    /// Curves with the Cap type control.
    pub fn has_cap(self) -> bool {
        matches!(self, Curve::Linear | Curve::Classic | Curve::Power)
    }
}

/// Cap type segmented control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CapType {
    Input,
    Output,
    Both,
}

impl CapType {
    pub fn mode(self) -> CapMode {
        match self {
            CapType::Input => CapMode::Input,
            CapType::Output => CapMode::Output,
            CapType::Both => CapMode::InOut,
        }
    }
    pub fn from_mode(m: CapMode) -> Self {
        match m {
            CapMode::Input => CapType::Input,
            CapMode::Output => CapType::Output,
            CapMode::InOut => CapType::Both,
        }
    }
}

/// One slider row's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Field {
    Acceleration,
    Exponent,
    InputOffset,
    CapInput,
    CapOutput,
    DecayRate,
    Limit,
    JumpInput,
    JumpOutput,
    Smooth,
    SyncSpeed,
    Motivity,
    Gamma,
    Scale,
    OutputOffset,
}

/// A row: label, min, max, step, decimals shown, default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowSpec {
    pub field: Field,
    pub label: &'static str,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub decimals: usize,
    /// shown with a "×"
    pub times: bool,
    pub default: f64,
}

#[allow(clippy::too_many_arguments)]
const fn r(field: Field, label: &'static str, min: f64, max: f64, step: f64, decimals: usize, times: bool, default: f64) -> RowSpec {
    RowSpec { field, label, min, max, step, decimals, times, default }
}

/// The rows of a curve (DESIGN table), before the cap rows are filtered by type. Steps and shown decimals = the approved
/// drawing's (menu-v22 `APAR`, Order 019); the minimums Raw Accel refuses start one step above (module doc).
pub fn rows(c: Curve) -> Vec<RowSpec> {
    use Field::*;
    match c {
        Curve::Linear => vec![
            r(Acceleration, "Acceleration", 0.05, 5.0, 0.05, 2, false, 2.8),
            r(InputOffset, "Input offset", 0.0, 120.0, 1.0, 0, false, 55.0),
            r(CapInput, "Cap: input", 1.0, 120.0, 1.0, 0, false, 15.0),
            r(CapOutput, "Cap: output", 1.0, 5.0, 0.05, 2, true, 2.6),
        ],
        Curve::Classic => vec![
            r(Acceleration, "Acceleration", 0.001, 0.1, 0.001, 3, false, 0.020),
            r(Exponent, "Exponent", 1.1, 5.0, 0.05, 2, false, 2.5),
            r(InputOffset, "Input offset", 0.0, 120.0, 1.0, 0, false, 20.0),
            r(CapInput, "Cap: input", 1.0, 120.0, 1.0, 0, false, 60.0),
            r(CapOutput, "Cap: output", 1.0, 5.0, 0.05, 2, true, 2.4),
        ],
        Curve::Natural => vec![
            r(DecayRate, "Decay rate", 0.01, 1.0, 0.01, 2, false, 0.10),
            r(Limit, "Limit", 1.0, 5.0, 0.05, 2, true, 1.5),
            r(InputOffset, "Input offset", 0.0, 120.0, 1.0, 0, false, 0.0),
        ],
        Curve::Jump => vec![
            r(JumpInput, "Jump: input", 1.0, 120.0, 1.0, 0, false, 15.0),
            r(JumpOutput, "Jump: output", 1.0, 5.0, 0.05, 2, true, 2.6),
            r(Smooth, "Smooth", 0.0, 1.0, 0.05, 2, false, 0.50),
        ],
        Curve::Synchronous => vec![
            r(SyncSpeed, "Sync speed", 1.0, 100.0, 1.0, 0, false, 5.0),
            r(Motivity, "Motivity", 1.05, 5.0, 0.05, 2, false, 1.50),
            r(Gamma, "Gamma", 0.1, 3.0, 0.05, 2, false, 1.00),
            r(Smooth, "Smooth", 0.0, 1.0, 0.05, 2, false, 0.50),
        ],
        Curve::Power => vec![
            r(Scale, "Scale", 0.1, 5.0, 0.05, 2, false, 1.00),
            r(Exponent, "Exponent", 0.01, 1.0, 0.01, 2, false, 0.05),
            r(OutputOffset, "Output offset", 0.0, 1.0, 0.05, 2, false, 0.00),
            r(CapInput, "Cap: input", 1.0, 120.0, 1.0, 0, false, 15.0),
            r(CapOutput, "Cap: output", 1.0, 5.0, 0.05, 2, true, 2.6),
        ],
    }
}

/// Sens multiplier row (always last): 0.1–3, step .05, default 1.00×.
pub const SENS: RowSpec = r(Field::Scale, "Sens multiplier", 0.1, 3.0, 0.05, 2, true, 1.0);

/// One curve's values (Gain and Cap type included).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurveValues {
    pub gain: bool,
    /// only for Linear / Classic / Power
    pub cap_type: CapType,
    pub values: BTreeMap<Field, f64>,
}

impl CurveValues {
    /// DESIGN defaults: Gain on, Cap type Output.
    pub fn defaults(c: Curve) -> Self {
        Self { gain: true, cap_type: CapType::Output, values: rows(c).into_iter().map(|r| (r.field, r.default)).collect() }
    }
    pub fn get(&self, f: Field) -> f64 {
        self.values.get(&f).copied().unwrap_or(0.0)
    }
}

/// Snaps a value onto its row: clamp to min–max, round to the step (from min).
pub fn snap(spec: &RowSpec, v: f64) -> f64 {
    let v = v.clamp(spec.min, spec.max);
    let n = ((v - spec.min) / spec.step).round();
    let s = spec.min + n * spec.step;
    // keep the shown decimals exact (0.1 + 3 × 0.05 = 0.25000000000000006 → 0.25)
    let p = 10f64.powi(spec.decimals as i32 + 2);
    ((s * p).round() / p).clamp(spec.min, spec.max)
}

/// The rows the card shows for these values (DESIGN: Output shows only Cap: output, Input only Cap: input, Both shows
/// both caps and hides the rate row — Acceleration / Scale).
pub fn visible_rows(c: Curve, v: &CurveValues) -> Vec<RowSpec> {
    rows(c)
        .into_iter()
        .filter(|r| {
            if !c.has_cap() {
                return true;
            }
            !matches!(
                (r.field, v.cap_type),
                (Field::CapInput, CapType::Output) | (Field::CapOutput, CapType::Input) | (Field::Acceleration | Field::Scale, CapType::Both)
            )
        })
        .collect()
}

/// Raw Accel's args for a curve, built like its GUI: C++ defaults + the curve's shown fields.
pub fn to_args(c: Curve, v: &CurveValues) -> AccelArgs {
    let mut a = AccelArgs { gain: v.gain, ..AccelArgs::default() };
    match c {
        Curve::Linear | Curve::Classic => {
            a.mode = AccelMode::Classic;
            a.acceleration = v.get(Field::Acceleration);
            a.cap = Vec2 { x: v.get(Field::CapInput), y: v.get(Field::CapOutput) };
            a.cap_mode = v.cap_type.mode();
            a.input_offset = v.get(Field::InputOffset);
            if c == Curve::Classic {
                a.exponent_classic = v.get(Field::Exponent);
            }
        }
        Curve::Natural => {
            a.mode = AccelMode::Natural;
            a.decay_rate = v.get(Field::DecayRate);
            a.limit = v.get(Field::Limit);
            a.input_offset = v.get(Field::InputOffset);
        }
        Curve::Jump => {
            a.mode = AccelMode::Jump;
            a.cap = Vec2 { x: v.get(Field::JumpInput), y: v.get(Field::JumpOutput) };
            a.smooth = v.get(Field::Smooth);
        }
        Curve::Synchronous => {
            a.mode = AccelMode::Synchronous;
            a.sync_speed = v.get(Field::SyncSpeed);
            a.motivity = v.get(Field::Motivity);
            a.gamma = v.get(Field::Gamma);
            a.smooth = v.get(Field::Smooth);
        }
        Curve::Power => {
            a.mode = AccelMode::Power;
            a.scale = v.get(Field::Scale);
            a.exponent_power = v.get(Field::Exponent);
            a.output_offset = v.get(Field::OutputOffset);
            a.cap = Vec2 { x: v.get(Field::CapInput), y: v.get(Field::CapOutput) };
            a.cap_mode = v.cap_type.mode();
        }
    }
    a
}

/// Raw Accel's "Off" (noaccel): what "Off" writes.
pub fn off_args() -> AccelArgs {
    AccelArgs { mode: AccelMode::Noaccel, ..AccelArgs::default() }
}

/// The reverse, for "Copy its curve" / mirroring the installed Raw Accel: which curve (Raw Accel's GUI rule: classic with
/// exponent exactly 2 = Linear) and its values. Values are taken as they are (not snapped), so the graph stays exact.
/// `None` for Off (noaccel) and for a look-up table (left out of the app).
pub fn from_args(a: &AccelArgs) -> Option<(Curve, CurveValues)> {
    let curve = match a.mode {
        AccelMode::Classic if a.exponent_classic == 2.0 => Curve::Linear,
        AccelMode::Classic => Curve::Classic,
        AccelMode::Natural => Curve::Natural,
        AccelMode::Jump => Curve::Jump,
        AccelMode::Synchronous => Curve::Synchronous,
        AccelMode::Power => Curve::Power,
        AccelMode::Lut | AccelMode::Noaccel => return None,
    };
    let mut v = CurveValues::defaults(curve);
    v.gain = a.gain;
    v.cap_type = CapType::from_mode(a.cap_mode);
    let mut set = |f: Field, x: f64| {
        v.values.insert(f, x);
    };
    match curve {
        Curve::Linear | Curve::Classic => {
            set(Field::Acceleration, a.acceleration);
            set(Field::InputOffset, a.input_offset);
            set(Field::CapInput, a.cap.x);
            set(Field::CapOutput, a.cap.y);
            if curve == Curve::Classic {
                set(Field::Exponent, a.exponent_classic);
            }
        }
        Curve::Natural => {
            set(Field::DecayRate, a.decay_rate);
            set(Field::Limit, a.limit);
            set(Field::InputOffset, a.input_offset);
        }
        Curve::Jump => {
            set(Field::JumpInput, a.cap.x);
            set(Field::JumpOutput, a.cap.y);
            set(Field::Smooth, a.smooth);
        }
        Curve::Synchronous => {
            set(Field::SyncSpeed, a.sync_speed);
            set(Field::Motivity, a.motivity);
            set(Field::Gamma, a.gamma);
            set(Field::Smooth, a.smooth);
        }
        Curve::Power => {
            set(Field::Scale, a.scale);
            set(Field::Exponent, a.exponent_power);
            set(Field::OutputOffset, a.output_offset);
            set(Field::CapInput, a.cap.x);
            set(Field::CapOutput, a.cap.y);
        }
    }
    Some((curve, v))
}

/// A preset chip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: PresetId,
    pub name: String,
    pub curve: Curve,
    pub values: CurveValues,
    pub sens: f64,
}

/// What a target means for the driver: the curve's args + the sens multiplier, or Off.
#[derive(Clone, Debug, PartialEq)]
pub struct Setting {
    pub args: AccelArgs,
    pub sens: f64,
}

impl Setting {
    pub fn off() -> Self {
        Setting { args: off_args(), sens: 1.0 }
    }
    /// The profile the driver gets: the user's own Raw Accel profile (`base`, kept as it is — rotation, DPI ratios,
    /// smoothing…) with these accel args on both axes and Output DPI = sens × 1000.
    pub fn apply_to(&self, base: &Profile) -> Profile {
        let mut p = base.clone();
        p.accel_x = self.args.clone();
        p.accel_y = self.args.clone();
        p.output_dpi = self.sens * NORMALIZED_DPI;
        p
    }
}

/// Max preset name length (DESIGN: inline box, max 20 characters).
pub const PRESET_NAME_MAX: usize = 20;

/// The card's whole state (saved with the app's settings).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    /// the header switch
    pub on: bool,
    pub expanded: bool,
    pub curve: Curve,
    /// each curve's own values
    pub values: BTreeMap<Curve, CurveValues>,
    /// shared by all curves
    pub sens: f64,
    pub presets: Vec<Preset>,
    /// the chip with the accent ring
    pub loaded: Option<PresetId>,
    next_preset: u64,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            on: false,
            expanded: false,
            curve: Curve::Linear,
            values: Curve::ALL.iter().map(|c| (*c, CurveValues::defaults(*c))).collect(),
            sens: SENS.default,
            presets: Vec::new(),
            loaded: None,
            next_preset: 0,
        }
    }
}

impl Panel {
    pub fn current_values(&self) -> CurveValues {
        self.values.get(&self.curve).cloned().unwrap_or_else(|| CurveValues::defaults(self.curve))
    }

    pub fn current_setting(&self) -> Setting {
        Setting { args: to_args(self.curve, &self.current_values()), sens: self.sens }
    }

    pub fn preset(&self, id: PresetId) -> Option<&Preset> {
        self.presets.iter().find(|p| p.id == id)
    }

    pub fn preset_setting(&self, id: PresetId) -> Option<Setting> {
        self.preset(id).map(|p| Setting { args: to_args(p.curve, &p.values), sens: p.sens })
    }

    /// The amber dot + "Update <name>": anything differs from the loaded preset.
    pub fn changed_since_loaded(&self) -> bool {
        match self.loaded.and_then(|id| self.preset(id)) {
            Some(p) => p.curve != self.curve || p.values != self.current_values() || p.sens != self.sens,
            None => false,
        }
    }

    /// Curve popup. Never resets anything.
    pub fn set_curve(&mut self, c: Curve) {
        self.curve = c;
    }

    pub fn set_gain(&mut self, gain: bool) {
        self.values.entry(self.curve).or_insert_with(|| CurveValues::defaults(self.curve)).gain = gain;
    }

    /// Cap type; switching to Input or Both with Cap: input ≤ Input offset moves Cap: input to min(120, offset + 30).
    pub fn set_cap_type(&mut self, t: CapType) {
        let c = self.curve;
        let v = self.values.entry(c).or_insert_with(|| CurveValues::defaults(c));
        v.cap_type = t;
        Self::keep_cap_above_offset(c, v);
    }

    fn keep_cap_above_offset(c: Curve, v: &mut CurveValues) {
        if matches!(c, Curve::Linear | Curve::Classic) && v.cap_type != CapType::Output {
            let off = v.get(Field::InputOffset);
            if v.get(Field::CapInput) <= off {
                v.values.insert(Field::CapInput, (off + 30.0).min(120.0));
            }
        }
    }

    /// A slider moved: the value is snapped onto the row. Returns the stored value. Unknown rows for this curve are ignored.
    pub fn set_value(&mut self, f: Field, value: f64) -> Option<f64> {
        let c = self.curve;
        let spec = rows(c).into_iter().find(|r| r.field == f)?;
        let v = self.values.entry(c).or_insert_with(|| CurveValues::defaults(c));
        let s = snap(&spec, value);
        v.values.insert(f, s);
        if f == Field::InputOffset {
            Self::keep_cap_above_offset(c, v);
        }
        Some(s)
    }

    pub fn set_sens(&mut self, value: f64) -> f64 {
        self.sens = snap(&SENS, value);
        self.sens
    }

    /// Click a chip = load it (its curve, that curve's values, the sens multiplier). Returns the toast.
    pub fn load_preset(&mut self, id: PresetId) -> Option<String> {
        let p = self.preset(id)?.clone();
        self.curve = p.curve;
        self.values.insert(p.curve, p.values.clone());
        self.sens = p.sens;
        self.loaded = Some(id);
        Some(format!("Loaded {}", p.name))
    }

    /// "Update <name>": saves the card into the loaded preset. Returns the toast.
    pub fn update_loaded(&mut self) -> Option<String> {
        let id = self.loaded?;
        let (curve, values, sens) = (self.curve, self.current_values(), self.sens);
        let p = self.presets.iter_mut().find(|p| p.id == id)?;
        p.curve = curve;
        p.values = values;
        p.sens = sens;
        Some(format!("Saved · {}", p.name))
    }

    /// "+ Save as preset": "Preset N" with the current settings, becomes current. Returns (id, toast); the menu opens the
    /// rename box at once.
    pub fn save_as_preset(&mut self) -> (PresetId, String) {
        self.next_preset += 1;
        let id = PresetId(self.next_preset);
        let mut n = self.presets.len() + 1;
        while self.presets.iter().any(|p| p.name == format!("Preset {n}")) {
            n += 1;
        }
        let name = format!("Preset {n}");
        self.presets.push(Preset { id, name: name.clone(), curve: self.curve, values: self.current_values(), sens: self.sens });
        self.loaded = Some(id);
        (id, format!("Saved as {name} · pick it for an app below"))
    }

    /// ✎ rename: trimmed, 1–20 characters, not the name of another preset.
    pub fn rename_preset(&mut self, id: PresetId, name: &str) -> Result<(), &'static str> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a preset needs a name");
        }
        if name.chars().count() > PRESET_NAME_MAX {
            return Err("at most 20 characters");
        }
        if self.presets.iter().any(|p| p.id != id && p.name.eq_ignore_ascii_case(name)) {
            return Err("another preset has this name");
        }
        let p = self.presets.iter_mut().find(|p| p.id == id).ok_or("no such preset")?;
        p.name = name.to_string();
        Ok(())
    }

    /// × delete (no confirm). The per-app side (apps → Off) is done by the caller with `PerApp::forget_preset`.
    pub fn delete_preset(&mut self, id: PresetId) -> Option<Preset> {
        let i = self.presets.iter().position(|p| p.id == id)?;
        if self.loaded == Some(id) {
            self.loaded = None;
        }
        Some(self.presets.remove(i))
    }
}

/// The value text of a row ("2.80", "55", "2.60×").
pub fn value_text(spec: &RowSpec, v: f64) -> String {
    format!("{:.*}{}", spec.decimals, v, if spec.times { "×" } else { "" })
}
