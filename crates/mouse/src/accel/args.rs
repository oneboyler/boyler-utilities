//! Raw Accel's settings exactly as its v1.7.0 source defines them (`common/rawaccel-base.hpp`, `common/rawaccel.hpp`,
//! `wrapper/wrapper.cpp`; tag v1.7.0 = commit d179e22e, MIT) — the same field names, defaults and settings.json keys.
//! Reading settings.json follows the wrapper (Newtonsoft with DefaultValueHandling.Populate: a missing key keeps the
//! C++ default). Keys this crate does not know are not kept — writing goes to the driver (bytes) or to the app's own
//! file, never back into Raw Accel's settings.json.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const MAX_NAME_LEN: usize = 256;
pub const MAX_DEV_ID_LEN: usize = 200;
pub const LUT_RAW_DATA_CAPACITY: usize = 514;
pub const NORMALIZED_DPI: f64 = 1000.0;
pub const POLL_RATE_MAX: f64 = 8000.0;
pub const DEFAULT_TIME_MIN: f64 = 1000.0 / POLL_RATE_MAX / 2.0;
pub const DEFAULT_TIME_MAX: f64 = 100.0;
/// The driver waits this long on every write (anti-abuse delay).
pub const WRITE_DELAY_MS: u64 = 1000;

/// `accel_mode` (C++ order = the ints the driver gets). settings.json names: classic | jump | natural | synchronous |
/// power | lut | noaccel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccelMode {
    Classic,
    Jump,
    Natural,
    Synchronous,
    Power,
    Lut,
    Noaccel,
}

impl AccelMode {
    pub fn as_i32(self) -> i32 {
        self as i32
    }
    pub fn from_i32(v: i32) -> Option<Self> {
        [Self::Classic, Self::Jump, Self::Natural, Self::Synchronous, Self::Power, Self::Lut, Self::Noaccel].get(v as usize).copied()
    }
}

/// `cap_mode`: settings.json in_out | input | output = C++ io | in | out = 0 | 1 | 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CapMode {
    #[serde(rename = "in_out")]
    InOut,
    #[serde(rename = "input")]
    Input,
    #[serde(rename = "output")]
    Output,
}

impl CapMode {
    pub fn as_i32(self) -> i32 {
        self as i32
    }
    pub fn from_i32(v: i32) -> Option<Self> {
        [Self::InOut, Self::Input, Self::Output].get(v as usize).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

fn d_true() -> bool {
    true
}

/// `accel_args` (+ settings.json names from the wrapper's `AccelArgs`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccelArgs {
    pub mode: AccelMode,
    #[serde(rename = "Gain / Velocity", default = "d_true")]
    pub gain: bool,
    #[serde(rename = "inputOffset")]
    pub input_offset: f64,
    #[serde(rename = "outputOffset")]
    pub output_offset: f64,
    pub acceleration: f64,
    #[serde(rename = "decayRate")]
    pub decay_rate: f64,
    pub gamma: f64,
    pub motivity: f64,
    #[serde(rename = "exponentClassic")]
    pub exponent_classic: f64,
    pub scale: f64,
    #[serde(rename = "exponentPower")]
    pub exponent_power: f64,
    pub limit: f64,
    #[serde(rename = "syncSpeed")]
    pub sync_speed: f64,
    pub smooth: f64,
    #[serde(rename = "Cap / Jump")]
    pub cap: Vec2,
    #[serde(rename = "Cap mode")]
    pub cap_mode: CapMode,
    /// lookup-table points (lut mode only in settings.json; the synchronous+gain table is computed, never stored here)
    pub data: Vec<f32>,
}

impl Default for AccelArgs {
    fn default() -> Self {
        Self {
            mode: AccelMode::Noaccel,
            gain: true,
            input_offset: 0.0,
            output_offset: 0.0,
            acceleration: 0.005,
            decay_rate: 0.1,
            gamma: 1.0,
            motivity: 1.5,
            exponent_classic: 2.0,
            scale: 1.0,
            exponent_power: 0.05,
            limit: 1.5,
            sync_speed: 5.0,
            smooth: 0.5,
            cap: Vec2 { x: 15.0, y: 1.5 },
            cap_mode: CapMode::Output,
            data: Vec::new(),
        }
    }
}

/// `speed_args`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpeedArgs {
    #[serde(rename = "Whole/combined accel (set false for 'by component' mode)")]
    pub whole: bool,
    #[serde(rename = "lpNorm")]
    pub lp_norm: f64,
    #[serde(rename = "Time in ms after which an input is weighted at half its original value.")]
    pub input_speed_smooth_halflife: f64,
    #[serde(rename = "Time in ms after which scale is weighted at half its original value.")]
    pub scale_smooth_halflife: f64,
    #[serde(rename = "Time in ms after which an output is weighted at half its original value.")]
    pub output_speed_smooth_halflife: f64,
}

impl Default for SpeedArgs {
    fn default() -> Self {
        Self { whole: true, lp_norm: 2.0, input_speed_smooth_halflife: 0.0, scale_smooth_halflife: 0.0, output_speed_smooth_halflife: 0.0 }
    }
}

/// `profile`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    #[serde(rename = "Stretches domain for horizontal vs vertical inputs")]
    pub domain_weights: Vec2,
    #[serde(rename = "Stretches accel range for horizontal vs vertical inputs")]
    pub range_weights: Vec2,
    #[serde(rename = "Whole or horizontal accel parameters")]
    pub accel_x: AccelArgs,
    #[serde(rename = "Vertical accel parameters")]
    pub accel_y: AccelArgs,
    #[serde(rename = "Input speed calculation parameters")]
    pub speed: SpeedArgs,
    #[serde(rename = "Output DPI")]
    pub output_dpi: f64,
    #[serde(rename = "Y/X output DPI ratio (vertical sens multiplier)")]
    pub yx_output_dpi_ratio: f64,
    #[serde(rename = "L/R output DPI ratio (left sens multiplier)")]
    pub lr_output_dpi_ratio: f64,
    #[serde(rename = "U/D output DPI ratio (up sens multiplier)")]
    pub ud_output_dpi_ratio: f64,
    #[serde(rename = "Degrees of rotation")]
    pub degrees_rotation: f64,
    #[serde(rename = "Degrees of angle snapping")]
    pub degrees_snap: f64,
    /// not in settings.json (JsonIgnore) — always the default 0
    #[serde(skip)]
    pub speed_min: f64,
    #[serde(rename = "Input Speed Cap")]
    pub speed_max: f64,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: "default".into(),
            domain_weights: Vec2 { x: 1.0, y: 1.0 },
            range_weights: Vec2 { x: 1.0, y: 1.0 },
            accel_x: AccelArgs::default(),
            accel_y: AccelArgs::default(),
            speed: SpeedArgs::default(),
            output_dpi: NORMALIZED_DPI,
            yx_output_dpi_ratio: 1.0,
            lr_output_dpi_ratio: 1.0,
            ud_output_dpi_ratio: 1.0,
            degrees_rotation: 0.0,
            degrees_snap: 0.0,
            speed_min: 0.0,
            speed_max: 0.0,
        }
    }
}

/// `device_config`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceConfig {
    pub disable: bool,
    #[serde(rename = "setExtraInfo", skip_serializing_if = "is_false")]
    pub set_extra_info: bool,
    #[serde(rename = "Use constant time interval based on polling rate")]
    pub poll_time_lock: bool,
    #[serde(rename = "DPI (normalizes input speed unit: counts/ms -> in/s)")]
    pub dpi: i32,
    #[serde(rename = "Polling rate Hz (keep at 0 for automatic adjustment)")]
    pub polling_rate: i32,
    #[serde(rename = "minimumTime", skip_serializing_if = "is_default_min")]
    pub clamp_min: f64,
    #[serde(rename = "maximumTime", skip_serializing_if = "is_default_max")]
    pub clamp_max: f64,
}

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_default_min(v: &f64) -> bool {
    *v == DEFAULT_TIME_MIN
}
fn is_default_max(v: &f64) -> bool {
    *v == DEFAULT_TIME_MAX
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self { disable: false, set_extra_info: false, poll_time_lock: false, dpi: 0, polling_rate: 0, clamp_min: DEFAULT_TIME_MIN, clamp_max: DEFAULT_TIME_MAX }
    }
}

/// `device_settings`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DeviceSettings {
    pub name: String,
    pub profile: String,
    pub id: String,
    pub config: DeviceConfig,
}

/// The whole settings.json (`DriverConfig`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DriverConfig {
    pub version: String,
    #[serde(rename = "defaultDeviceConfig")]
    pub default_device_config: DeviceConfig,
    pub profiles: Vec<Profile>,
    pub devices: Vec<DeviceSettings>,
}

impl Default for DriverConfig {
    fn default() -> Self {
        Self { version: "1.7.0".into(), default_device_config: DeviceConfig::default(), profiles: vec![Profile::default()], devices: Vec::new() }
    }
}

impl DriverConfig {
    /// Parses settings.json the way the wrapper does (missing keys → defaults; no profiles → one default profile).
    pub fn from_json(text: &str) -> Result<Self, String> {
        let mut c: DriverConfig = serde_json::from_str(text).map_err(|e| format!("settings.json: {e}"))?;
        if c.profiles.is_empty() {
            c.profiles.push(Profile::default());
        }
        Ok(c)
    }

    /// settings.json text in Raw Accel's own shape (the two "###" comment keys first, then the fields).
    pub fn to_json(&self) -> String {
        let mut m = Map::new();
        m.insert("### Accel modes ###".into(), Value::String("classic | jump | natural | synchronous | power | lut | noaccel".into()));
        m.insert("### Cap modes ###".into(), Value::String("in_out | input | output".into()));
        if let Ok(Value::Object(o)) = serde_json::to_value(self) {
            for (k, v) in o {
                m.insert(k, v);
            }
        }
        serde_json::to_string_pretty(&Value::Object(m)).unwrap_or_default()
    }
}
