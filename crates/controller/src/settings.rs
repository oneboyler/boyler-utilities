//! Every setting the Controller page shows (v21 drawing + DESIGN v19 §3.19), mapped to Steam's own keys.
//!
//! Where the keys and values come from (all read 2026-10-08, read-only):
//! - **Measured** = seen in a real Rocket League layout or the 128 local layouts / Steam's templates.
//! - **Steam** = Steam's own settings schema in its UI code (`steamui\chunk~2dcc5aaf7.js`: setting ids, choices and their
//!   numbers, e.g. curve Linear 0 / Aggressive 1 / Relaxed 2 / Wide 3 / Extra wide 4 / Custom 5) + the key names in
//!   `steamclient64.dll`. The UI code names each setting by the same short key it saves under, which ties schema ids to
//!   file keys (e.g. the trigger's "Threshold" uses the same key as every "Outer ring radius" → `edge_binding_radius`).
//! - **Guessed** = the best match, not proven (the [`Sure`] flag says so; the report lists each one).
//!
//! A value of `None` means "not in the file" = Steam's default (Steam only writes values that differ from its default).
//! Writing `None` removes the line again.

use crate::binding::Action;
use crate::layout::{LResult, Layout, Press};
use crate::parts::{ButtonId, PadKind, Side};

/// How sure the key / value mapping is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sure {
    Measured,
    Steam,
    Guessed,
}

/// How a number is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// on / off (`"1"` / `"0"`)
    Bool,
    /// Steam's 0–32767 radius, shown as %
    Radius,
    Percent,
    Ms,
    Degrees,
    /// one of a fixed list (value, shown name)
    Choice(&'static [(i64, &'static str)]),
    /// a plain number in Steam's own units (shown as is)
    Raw,
}

/// One setting: its key in the file, its shown name, unit and how sure the mapping is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Def {
    pub key: &'static str,
    pub label: &'static str,
    pub unit: Unit,
    pub sure: Sure,
}

const fn d(key: &'static str, label: &'static str, unit: Unit, sure: Sure) -> Def {
    Def { key, label, unit, sure }
}

/// Haptics on a press / stick (Steam: Off 0, Low 1, Medium 2, High 3).
pub const HAPTICS: &[(i64, &str)] = &[(0, "Off"), (1, "Low"), (2, "Medium"), (3, "High")];
/// Haptics on a whole group (Steam's `haptic_intensity_override`: "use the press's own" 5, Off 0 … High 3).
pub const HAPTICS_GROUP: &[(i64, &str)] = &[(5, "Each press's own"), (0, "Off"), (1, "Low"), (2, "Medium"), (3, "High")];
pub const CURVES: &[(i64, &str)] = &[(0, "Linear"), (2, "Relaxed"), (1, "Aggressive"), (3, "Wide"), (4, "Extra wide"), (5, "Custom")];
pub const DZ_SHAPES: &[(i64, &str)] = &[(1, "Circle"), (0, "Cross"), (2, "Square")];
pub const DZ_SOURCES: &[(i64, &str)] = &[(2, "Custom"), (1, "This controller's own"), (0, "None")];
pub const STICK_OUTPUTS: &[(i64, &str)] = &[(0, "Left stick"), (1, "Right stick"), (2, "Mouse")];
pub const TRIGGER_OUTPUTS: &[(i64, &str)] = &[(1, "Left trigger"), (2, "Right trigger"), (0, "Nothing (click only)")];
/// Steam's flick-stick snap choices (no "16 directions" exists in Steam; the drawing's list differs — report).
pub const FLICK_SNAPS: &[(i64, &str)] = &[(0, "Off"), (1, "2 directions"), (2, "4 directions"), (3, "6 directions"), (4, "8 directions"), (5, "Forward only")];
pub const GYRO_AXES: &[(i64, &str)] = &[(0, "Left-right"), (1, "Tilt"), (2, "Both")];
/// "On while held" (Steam's gyro enable buttons for PlayStation pads; Edge back buttons per Steam's Edge list).
pub const GYRO_BUTTONS: &[(i64, &str)] = &[
    (0, "Always on"),
    (5, "R1"),
    (6, "L1"),
    (9, "R2 (full pull)"),
    (10, "L2 (full pull)"),
    (11, "R2 (soft pull)"),
    (12, "L2 (soft pull)"),
    (13, "Cross"),
    (14, "Circle"),
    (15, "Square"),
    (16, "Triangle"),
    (17, "L3"),
    (18, "R3"),
    (1, "Touchpad right touch"),
    (2, "Touchpad left touch"),
    (20, "Touchpad touch"),
    (3, "Touchpad right click"),
    (4, "Touchpad left click"),
    (7, "Right back button"),
    (8, "Left back button"),
    (24, "Right Fn"),
    (25, "Left Fn"),
];
/// Gyro "while held": Steam's choices On 1 / Off 0 / Toggle 2 (key name guessed).
pub const GYRO_HELD: &[(i64, &str)] = &[(1, "Held = on"), (0, "Held = off"), (2, "Press = toggle")];

/// The settings of one press (Steam's "Regular Press Settings" and the other activators').
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PressSetting {
    HoldToRepeat,
    RepeatRate,
    Toggle,
    CycleCommands,
    Interruptible,
    InvertInput,
    FireStartDelay,
    FireEndDelay,
    Haptics,
    LongPressTime,
    DoublePressTime,
    /// Chorded press: the other button (Steam's button list, as for the gyro).
    ChordButton,
}

impl PressSetting {
    pub const ALL: [PressSetting; 12] = [
        PressSetting::HoldToRepeat,
        PressSetting::RepeatRate,
        PressSetting::Toggle,
        PressSetting::CycleCommands,
        PressSetting::Interruptible,
        PressSetting::InvertInput,
        PressSetting::FireStartDelay,
        PressSetting::FireEndDelay,
        PressSetting::Haptics,
        PressSetting::LongPressTime,
        PressSetting::DoublePressTime,
        PressSetting::ChordButton,
    ];
    pub fn def(self) -> Def {
        use Sure::*;
        use Unit::*;
        match self {
            PressSetting::HoldToRepeat => d("hold_repeats", "Hold to repeat (turbo)", Bool, Measured),
            PressSetting::RepeatRate => d("repeat_rate", "Repeat every", Ms, Measured),
            PressSetting::Toggle => d("toggle", "Toggle", Bool, Measured),
            PressSetting::CycleCommands => d("cycle", "Cycle commands", Bool, Steam),
            PressSetting::Interruptible => d("interruptable", "Interruptible", Bool, Measured),
            PressSetting::InvertInput => d("invert", "Invert input", Bool, Guessed),
            PressSetting::FireStartDelay => d("delay_start", "Fire start delay", Ms, Measured),
            PressSetting::FireEndDelay => d("delay_end", "Fire end delay", Ms, Measured),
            PressSetting::Haptics => d("haptic_intensity", "Haptics", Choice(HAPTICS), Measured),
            PressSetting::LongPressTime => d("long_press_time", "Long press after", Ms, Measured),
            PressSetting::DoublePressTime => d("double_tap_time", "Double press within", Ms, Measured),
            PressSetting::ChordButton => d("chord_button", "Together with", Choice(GYRO_BUTTONS), Guessed),
        }
    }
}

/// The settings of a stick group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StickSetting {
    DeadZone,
    FullAt,
    DeadZoneShape,
    DeadZoneSource,
    AntiDeadZone,
    AntiDeadZoneBuffer,
    Curve,
    CurveShape,
    Sensitivity,
    LeftRightSpeed,
    UpDownSpeed,
    SendsTo,
    InvertLeftRight,
    InvertUpDown,
    Smoothing,
    RingStartsAt,
    RingInsideInstead,
    Haptics,
    FlickTurnSpeed,
    FlickSnap,
    FlickForwardZone,
    FlickSweepSpeed,
}

impl StickSetting {
    pub const ALL: [StickSetting; 22] = [
        StickSetting::DeadZone,
        StickSetting::FullAt,
        StickSetting::DeadZoneShape,
        StickSetting::DeadZoneSource,
        StickSetting::AntiDeadZone,
        StickSetting::AntiDeadZoneBuffer,
        StickSetting::Curve,
        StickSetting::CurveShape,
        StickSetting::Sensitivity,
        StickSetting::LeftRightSpeed,
        StickSetting::UpDownSpeed,
        StickSetting::SendsTo,
        StickSetting::InvertLeftRight,
        StickSetting::InvertUpDown,
        StickSetting::Smoothing,
        StickSetting::RingStartsAt,
        StickSetting::RingInsideInstead,
        StickSetting::Haptics,
        StickSetting::FlickTurnSpeed,
        StickSetting::FlickSnap,
        StickSetting::FlickForwardZone,
        StickSetting::FlickSweepSpeed,
    ];
    pub fn def(self) -> Def {
        use Sure::*;
        use Unit::*;
        match self {
            StickSetting::DeadZone => d("deadzone_inner_radius", "Dead zone", Radius, Measured),
            StickSetting::FullAt => d("deadzone_outer_radius", "Full at", Radius, Measured),
            StickSetting::DeadZoneShape => d("deadzone_shape", "Shape", Choice(DZ_SHAPES), Steam),
            StickSetting::DeadZoneSource => d("deadzone_enable_type", "Source", Choice(DZ_SOURCES), Steam),
            StickSetting::AntiDeadZone => d("anti_deadzone", "Anti-dead zone", Raw, Steam),
            StickSetting::AntiDeadZoneBuffer => d("anti_deadzone_buffer", "Anti-dead zone buffer", Raw, Steam),
            StickSetting::Curve => d("curve_exponent", "Curve", Choice(CURVES), Steam),
            StickSetting::CurveShape => d("custom_curve_exponent", "Curve shape", Raw, Measured),
            StickSetting::Sensitivity => d("sensitivity", "Sensitivity", Percent, Measured),
            StickSetting::LeftRightSpeed => d("sensitivity_horiz_scale", "Left-right speed", Percent, Measured),
            StickSetting::UpDownSpeed => d("sensitivity_vert_scale", "Up-down speed", Percent, Measured),
            StickSetting::SendsTo => d("output_joystick", "Sends to", Choice(STICK_OUTPUTS), Steam),
            StickSetting::InvertLeftRight => d("invert_x", "Invert left-right", Bool, Steam),
            StickSetting::InvertUpDown => d("invert_y", "Invert up-down", Bool, Measured),
            StickSetting::Smoothing => d("joystick_smoothing", "Smoothing", Bool, Measured),
            StickSetting::RingStartsAt => d("edge_binding_radius", "Ring starts at", Radius, Measured),
            StickSetting::RingInsideInstead => d("edge_binding_invert", "Inside instead", Bool, Steam),
            StickSetting::Haptics => d("haptic_intensity", "Haptic intensity", Choice(HAPTICS), Measured),
            StickSetting::FlickTurnSpeed => d("flickstick_rotation_sensitivity", "Turn speed", Raw, Guessed),
            StickSetting::FlickSnap => d("flickstick_snap_mode", "Snap", Choice(FLICK_SNAPS), Steam),
            StickSetting::FlickForwardZone => d("flickstick_forward_deadzone_angle", "Forward zone", Degrees, Guessed),
            StickSetting::FlickSweepSpeed => d("flickstick_sweep_sensitivity", "Sweep speed", Raw, Guessed),
        }
    }
}

/// The settings of a trigger group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TriggerSetting {
    /// Analog output: which trigger it sends to; 0 = no analog = "Click only".
    SendsTo,
    ClicksAt,
    DeadZone,
    FullAt,
    Curve,
    Haptics,
}

impl TriggerSetting {
    pub const ALL: [TriggerSetting; 6] =
        [TriggerSetting::SendsTo, TriggerSetting::ClicksAt, TriggerSetting::DeadZone, TriggerSetting::FullAt, TriggerSetting::Curve, TriggerSetting::Haptics];
    pub fn def(self) -> Def {
        use Sure::*;
        use Unit::*;
        match self {
            TriggerSetting::SendsTo => d("output_trigger", "Sends to", Choice(TRIGGER_OUTPUTS), Measured),
            TriggerSetting::ClicksAt => d("edge_binding_radius", "Clicks at", Radius, Steam),
            TriggerSetting::DeadZone => d("deadzone_inner_radius", "Dead zone", Radius, Steam),
            TriggerSetting::FullAt => d("deadzone_outer_radius", "Full at", Radius, Steam),
            TriggerSetting::Curve => d("curve_exponent", "Curve", Choice(CURVES), Steam),
            TriggerSetting::Haptics => d("haptic_intensity_override", "Haptics", Choice(HAPTICS_GROUP), Steam),
        }
    }
}

/// The settings of the gyro group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GyroSetting {
    OnWhileHeld,
    HeldBehaviour,
    Sensitivity,
    SpeedDeadZone,
    PrecisionSpeed,
    UpDownSpeed,
    TurnWith,
    InvertLeftRight,
    InvertUpDown,
}

impl GyroSetting {
    pub const ALL: [GyroSetting; 9] = [
        GyroSetting::OnWhileHeld,
        GyroSetting::HeldBehaviour,
        GyroSetting::Sensitivity,
        GyroSetting::SpeedDeadZone,
        GyroSetting::PrecisionSpeed,
        GyroSetting::UpDownSpeed,
        GyroSetting::TurnWith,
        GyroSetting::InvertLeftRight,
        GyroSetting::InvertUpDown,
    ];
    pub fn def(self) -> Def {
        use Sure::*;
        use Unit::*;
        match self {
            GyroSetting::OnWhileHeld => d("gyro_button", "On while held", Choice(GYRO_BUTTONS), Measured),
            GyroSetting::HeldBehaviour => d("gyro_button_invert", "Held = off", Choice(GYRO_HELD), Guessed),
            GyroSetting::Sensitivity => d("gyro_sensitivity_scale", "Sensitivity", Percent, Guessed),
            GyroSetting::SpeedDeadZone => d("gyro_speed_deadzone", "Speed dead zone", Raw, Steam),
            GyroSetting::PrecisionSpeed => d("gyro_precision_speed", "Precision speed", Raw, Steam),
            GyroSetting::UpDownSpeed => d("gyro_vertical_horizontal_ratio", "Up-down speed", Percent, Steam),
            GyroSetting::TurnWith => d("gyro_axis", "Turn with", Choice(GYRO_AXES), Steam),
            GyroSetting::InvertLeftRight => d("gyro_invert_x", "Invert left-right", Bool, Guessed),
            GyroSetting::InvertUpDown => d("gyro_invert_y", "Invert up-down", Bool, Guessed),
        }
    }
}

/// The settings of the touchpad's mouse group + its clicks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TouchSetting {
    MouseSpeed,
    ClickNeedsPress,
    Haptics,
}

impl TouchSetting {
    pub const ALL: [TouchSetting; 3] = [TouchSetting::MouseSpeed, TouchSetting::ClickNeedsPress, TouchSetting::Haptics];
    pub fn def(self) -> Def {
        use Sure::*;
        use Unit::*;
        match self {
            TouchSetting::MouseSpeed => d("sensitivity", "Mouse speed", Percent, Measured),
            TouchSetting::ClickNeedsPress => d("requires_click", "Click needs a press", Bool, Measured),
            TouchSetting::Haptics => d("haptic_intensity", "Haptics", Choice(HAPTICS), Measured),
        }
    }
}

/// What a stick "acts as" (its group's mode; measured mode names).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StickMode {
    Joystick,
    Camera,
    Mouse,
    Dpad,
    FourButtons,
    ScrollWheel,
    FlickStick,
    RadialMenu,
    Nothing,
    /// a mode the drawing doesn't list (kept as it is)
    Other(String),
}

impl StickMode {
    pub const LISTED: [StickMode; 9] = [
        StickMode::Joystick,
        StickMode::Camera,
        StickMode::Mouse,
        StickMode::Dpad,
        StickMode::FourButtons,
        StickMode::ScrollWheel,
        StickMode::FlickStick,
        StickMode::RadialMenu,
        StickMode::Nothing,
    ];
    pub fn steam(&self) -> &str {
        match self {
            StickMode::Joystick => "joystick_move",
            StickMode::Camera => "joystick_camera",
            StickMode::Mouse => "joystick_mouse",
            StickMode::Dpad => "dpad",
            StickMode::FourButtons => "four_buttons",
            StickMode::ScrollWheel => "scrollwheel",
            StickMode::FlickStick => "flickstick",
            StickMode::RadialMenu => "radial_menu",
            StickMode::Nothing => "disabled",
            StickMode::Other(s) => s,
        }
    }
    pub fn from_steam(s: &str) -> StickMode {
        StickMode::LISTED.into_iter().find(|m| m.steam() == s).unwrap_or_else(|| StickMode::Other(s.to_string()))
    }
    pub fn label(&self) -> &str {
        match self {
            StickMode::Joystick => "Joystick",
            StickMode::Camera => "Joystick · camera",
            StickMode::Mouse => "As mouse",
            StickMode::Dpad => "Directional pad",
            StickMode::FourButtons => "Four buttons",
            StickMode::ScrollWheel => "Scroll wheel",
            StickMode::FlickStick => "Flick stick",
            StickMode::RadialMenu => "Radial menu",
            StickMode::Nothing => "Nothing",
            StickMode::Other(s) => s,
        }
    }
}

/// What the gyro does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GyroMode {
    Off,
    Mouse,
    Joystick,
    Camera,
    Other(String),
}

impl GyroMode {
    pub fn steam(&self) -> Option<&str> {
        match self {
            GyroMode::Off => None,
            GyroMode::Mouse => Some("gyro_to_mouse"),
            GyroMode::Joystick => Some("joystick_move"),
            GyroMode::Camera => Some("joystick_camera"),
            GyroMode::Other(s) => Some(s),
        }
    }
    fn from_steam(s: Option<&str>) -> GyroMode {
        match s {
            None => GyroMode::Off,
            Some("gyro_to_mouse") => GyroMode::Mouse,
            Some("joystick_move") => GyroMode::Joystick,
            Some("joystick_camera") => GyroMode::Camera,
            Some(o) => GyroMode::Other(o.to_string()),
        }
    }
}

/// What touching the touchpad does (the right half's group: Steam's PlayStation layouts split the pad into halves).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TouchMode {
    Nothing,
    Mouse,
    Scroll,
    Other(String),
}

impl TouchMode {
    fn from_steam(s: Option<&str>) -> TouchMode {
        match s {
            None | Some("single_button") | Some("disabled") => TouchMode::Nothing,
            Some("absolute_mouse") => TouchMode::Mouse,
            Some("scrollwheel") => TouchMode::Scroll,
            Some(o) => TouchMode::Other(o.to_string()),
        }
    }
    fn steam(&self) -> &str {
        match self {
            TouchMode::Nothing => "single_button",
            TouchMode::Mouse => "absolute_mouse",
            TouchMode::Scroll => "scrollwheel",
            TouchMode::Other(s) => s,
        }
    }
}

// ------------------------------------------------------------------------------------------------ conversions

/// Steam's 0–32767 radius → % (Rocket League: 3357 → 10 %, 25602 → 78 %, as the drawing shows).
pub fn radius_to_pct(raw: i64) -> f64 {
    raw as f64 * 100.0 / 32767.0
}
/// % → Steam's 0–32767 radius (rounded; 0–100 % only).
pub fn pct_to_radius(pct: f64) -> i64 {
    (pct.clamp(0.0, 100.0) * 32767.0 / 100.0).round() as i64
}

fn int(v: Option<String>) -> Option<i64> {
    v.and_then(|s| s.trim().parse::<i64>().ok())
}

// ------------------------------------------------------------------------------------------------ read views

/// One button as the panel shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonView {
    pub id: ButtonId,
    pub name: &'static str,
    /// The PS / Xbox button: Steam's own, not changeable.
    pub fixed: bool,
    /// What each press does (Regular, Long, Double, Start, Release, Chord).
    pub presses: Vec<(Press, Action)>,
    /// The Regular press's settings.
    pub settings: Vec<(PressSetting, Option<i64>)>,
    /// How many extra bindings the Regular press has (a key combo); writing an action replaces them with one.
    pub extra_bindings: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StickView {
    pub side: Side,
    pub mode: StickMode,
    pub settings: Vec<(StickSetting, Option<i64>)>,
    /// "At the edge does" (the outer-ring binding).
    pub ring_action: Action,
    /// Press (L3 / R3).
    pub press: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerView {
    pub side: Side,
    /// "Analog" when the trigger sends an analog value, "Click only" when not (Steam's `output_trigger` 0).
    pub analog: bool,
    pub settings: Vec<(TriggerSetting, Option<i64>)>,
    pub click: Action,
    pub soft_pull: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GyroView {
    pub mode: GyroMode,
    pub settings: Vec<(GyroSetting, Option<i64>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchpadView {
    pub touch: TouchMode,
    pub left_click: Action,
    pub right_click: Action,
    pub settings: Vec<(TouchSetting, Option<i64>)>,
}

/// The whole page for one game + controller + action set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PadView {
    pub kind: PadKind,
    pub set: u32,
    pub buttons: Vec<ButtonView>,
    pub sticks: Vec<StickView>,
    pub triggers: Vec<TriggerView>,
    pub gyro: Option<GyroView>,
    pub touchpad: Option<TouchpadView>,
}

fn bool_str(v: i64) -> String {
    v.to_string()
}

impl Layout {
    pub fn button_view(&self, set: u32, kind: PadKind, id: ButtonId) -> ButtonView {
        let name = id.name(kind);
        let Some(src) = id.source() else {
            return ButtonView { id, name, fixed: true, presses: vec![], settings: vec![], extra_bindings: 0 };
        };
        let mode = self.group_mode(set, src);
        let place = id.place(mode.as_deref()).expect("has a source");
        let presses = [Press::Full, Press::Long, Press::Double, Press::Start, Press::Release, Press::Chord]
            .into_iter()
            .map(|p| (p, self.action(set, place.source, place.input, p)))
            .collect();
        let settings = PressSetting::ALL
            .into_iter()
            .map(|s| {
                let press = if s == PressSetting::ChordButton { Press::Chord } else { Press::Full };
                (s, int(self.activator_setting(set, place.source, place.input, press, s.def().key)))
            })
            .collect();
        let extra = self.bindings(set, place.source, place.input, Press::Full).len().saturating_sub(1);
        ButtonView { id, name, fixed: id.is_fixed(), presses, settings, extra_bindings: extra }
    }

    pub fn stick_view(&self, set: u32, side: Side) -> StickView {
        let src = side.stick_source();
        let mode = self.group_mode(set, src).map(|m| StickMode::from_steam(&m)).unwrap_or(StickMode::Nothing);
        let settings = StickSetting::ALL.into_iter().map(|s| (s, int(self.group_setting(set, src, s.def().key)))).collect();
        StickView { side, mode, settings, ring_action: self.action(set, src, "edge", Press::Full), press: self.action(set, src, "click", Press::Full) }
    }

    pub fn trigger_view(&self, set: u32, side: Side) -> TriggerView {
        let src = side.trigger_source();
        let settings: Vec<(TriggerSetting, Option<i64>)> = TriggerSetting::ALL.into_iter().map(|s| (s, int(self.group_setting(set, src, s.def().key)))).collect();
        // measured: triggers write output_trigger 1 / 2; no value = Steam's default for a trigger group (analog, guessed)
        let out = settings.iter().find(|(s, _)| *s == TriggerSetting::SendsTo).and_then(|(_, v)| *v);
        TriggerView {
            side,
            analog: out != Some(0),
            settings,
            click: self.action(set, src, "click", Press::Full),
            soft_pull: self.action(set, src, "click", Press::Soft),
        }
    }

    pub fn gyro_view(&self, set: u32) -> GyroView {
        let mode = GyroMode::from_steam(self.group_mode(set, "gyro").as_deref());
        let settings = GyroSetting::ALL.into_iter().map(|s| (s, int(self.group_setting(set, "gyro", s.def().key)))).collect();
        GyroView { mode, settings }
    }

    pub fn touchpad_view(&self, set: u32) -> TouchpadView {
        let src = Side::Right.trackpad_source();
        let mode = self.group_mode(set, src);
        let touch = TouchMode::from_steam(mode.as_deref());
        let settings = TouchSetting::ALL.into_iter().map(|s| (s, int(self.group_setting(set, src, s.def().key)))).collect();
        TouchpadView {
            touch,
            left_click: self.action(set, Side::Left.trackpad_source(), "click", Press::Full),
            right_click: self.action(set, src, "click", Press::Full),
            settings,
        }
    }

    /// Everything the page shows for this pad and action set.
    pub fn pad_view(&self, set: u32, kind: PadKind) -> PadView {
        PadView {
            kind,
            set,
            buttons: kind.buttons().into_iter().map(|b| self.button_view(set, kind, b)).collect(),
            sticks: vec![self.stick_view(set, Side::Left), self.stick_view(set, Side::Right)],
            triggers: vec![self.trigger_view(set, Side::Left), self.trigger_view(set, Side::Right)],
            gyro: kind.has_gyro().then(|| self.gyro_view(set)),
            touchpad: kind.has_touchpad().then(|| self.touchpad_view(set)),
        }
    }
}

// ------------------------------------------------------------------------------------------------ changes

/// One change the page can make (every control of the drawing maps to one of these).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// What one press of a button does.
    ButtonAction { button: ButtonId, press: Press, action: Action },
    /// A Regular-press setting (or the chord's button); `None` = Steam's default.
    ButtonSetting { button: ButtonId, setting: PressSetting, value: Option<i64> },
    StickMode { side: Side, mode: StickMode },
    StickSetting { side: Side, setting: StickSetting, value: Option<i64> },
    /// "At the edge does".
    StickRing { side: Side, action: Action },
    /// "Mode: Analog / Click only" (`analog = false` writes `output_trigger 0`; `true` sends to its own side).
    TriggerAnalog { side: Side, analog: bool },
    TriggerSetting { side: Side, setting: TriggerSetting, value: Option<i64> },
    /// Full pull (`soft = false`) or soft pull (`true`).
    TriggerAction { side: Side, soft: bool, action: Action },
    GyroMode { mode: GyroMode },
    GyroSetting { setting: GyroSetting, value: Option<i64> },
    TouchMode { mode: TouchMode },
    TouchClick { half: Side, action: Action },
    TouchSetting { setting: TouchSetting, value: Option<i64> },
}

impl Change {
    /// Is this change possible on this pad (no gyro / touchpad on an Xbox pad, no back buttons on a plain DualSense)?
    pub fn fits(&self, kind: PadKind) -> bool {
        match self {
            Change::ButtonAction { button, .. } | Change::ButtonSetting { button, .. } => button.on(kind) && !button.is_fixed(),
            Change::GyroMode { .. } | Change::GyroSetting { .. } => kind.has_gyro(),
            Change::TouchMode { .. } | Change::TouchClick { .. } | Change::TouchSetting { .. } => kind.has_touchpad(),
            _ => true,
        }
    }
}

impl Layout {
    /// Apply one change to action set `set` (only the lines it needs change; nothing else in the file moves).
    pub fn apply(&mut self, set: u32, change: &Change) -> LResult<()> {
        match change {
            Change::ButtonAction { button, press, action } => {
                let src = button.source().ok_or_else(|| crate::layout::LayoutError::NoGroup("PS button".into()))?;
                let mode = self.group_mode(set, src);
                let p = button.place(mode.as_deref()).expect("has a source");
                self.set_action(set, p.source, p.new_mode, p.input, *press, action)
            }
            Change::ButtonSetting { button, setting, value } => {
                let src = button.source().ok_or_else(|| crate::layout::LayoutError::NoGroup("PS button".into()))?;
                let mode = self.group_mode(set, src);
                let p = button.place(mode.as_deref()).expect("has a source");
                let press = if *setting == PressSetting::ChordButton { Press::Chord } else { Press::Full };
                self.set_activator_setting(set, p.source, p.new_mode, p.input, press, setting.def().key, value.map(bool_str).as_deref())
            }
            Change::StickMode { side, mode } => self.set_group_mode(set, side.stick_source(), mode.steam()),
            Change::StickSetting { side, setting, value } => {
                self.set_group_setting(set, side.stick_source(), "joystick_move", setting.def().key, value.map(bool_str).as_deref())
            }
            Change::StickRing { side, action } => self.set_action(set, side.stick_source(), "joystick_move", "edge", Press::Full, action),
            Change::TriggerAnalog { side, analog } => {
                let v = if *analog {
                    match side {
                        Side::Left => 1,
                        Side::Right => 2,
                    }
                } else {
                    0
                };
                self.set_group_setting(set, side.trigger_source(), "trigger", "output_trigger", Some(&v.to_string()))
            }
            Change::TriggerSetting { side, setting, value } => {
                self.set_group_setting(set, side.trigger_source(), "trigger", setting.def().key, value.map(bool_str).as_deref())
            }
            Change::TriggerAction { side, soft, action } => {
                let press = if *soft { Press::Soft } else { Press::Full };
                self.set_action(set, side.trigger_source(), "trigger", "click", press, action)
            }
            Change::GyroMode { mode } => match mode.steam() {
                None => self.unbind_source(set, "gyro"),
                Some(m) => {
                    if self.group(set, "gyro").is_none() {
                        if let Some(id) = self.inactive_group_id(set, "gyro") {
                            self.bind_source(set, &id, "gyro")?;
                        }
                    }
                    self.set_group_mode(set, "gyro", m)
                }
            },
            Change::GyroSetting { setting, value } => self.set_group_setting(set, "gyro", "gyro_to_mouse", setting.def().key, value.map(bool_str).as_deref()),
            Change::TouchMode { mode } => self.set_group_mode(set, Side::Right.trackpad_source(), mode.steam()),
            Change::TouchClick { half, action } => self.set_action(set, half.trackpad_source(), "single_button", "click", Press::Full, action),
            Change::TouchSetting { setting, value } => {
                self.set_group_setting(set, Side::Right.trackpad_source(), "single_button", setting.def().key, value.map(bool_str).as_deref())
            }
        }
    }
}

impl PadView {
    /// The changes that make `part` in another layout look like it does in this view (used for "Steam's setting for
    /// this": the view is Steam's own layout, the changes go to the user's file). Every value is written, `None`s remove.
    pub fn part_changes(&self, part: crate::parts::Part) -> Vec<Change> {
        use crate::parts::Part;
        let mut out = Vec::new();
        let button = |out: &mut Vec<Change>, id: ButtonId| {
            if let Some(b) = self.buttons.iter().find(|b| b.id == id) {
                if b.fixed {
                    return;
                }
                for (press, action) in &b.presses {
                    out.push(Change::ButtonAction { button: id, press: *press, action: action.clone() });
                }
                for (setting, value) in &b.settings {
                    out.push(Change::ButtonSetting { button: id, setting: *setting, value: *value });
                }
            }
        };
        match part {
            Part::Button(id) => button(&mut out, id),
            Part::Stick(side) => {
                if let Some(s) = self.sticks.iter().find(|s| s.side == side) {
                    out.push(Change::StickMode { side, mode: s.mode.clone() });
                    for (setting, value) in &s.settings {
                        out.push(Change::StickSetting { side, setting: *setting, value: *value });
                    }
                    out.push(Change::StickRing { side, action: s.ring_action.clone() });
                }
                button(&mut out, if side == Side::Left { ButtonId::L3 } else { ButtonId::R3 });
            }
            Part::Trigger(side) => {
                if let Some(t) = self.triggers.iter().find(|t| t.side == side) {
                    for (setting, value) in &t.settings {
                        out.push(Change::TriggerSetting { side, setting: *setting, value: *value });
                    }
                    out.push(Change::TriggerAction { side, soft: false, action: t.click.clone() });
                    out.push(Change::TriggerAction { side, soft: true, action: t.soft_pull.clone() });
                }
            }
            Part::Gyro => {
                if let Some(g) = &self.gyro {
                    out.push(Change::GyroMode { mode: g.mode.clone() });
                    if g.mode != GyroMode::Off {
                        for (setting, value) in &g.settings {
                            out.push(Change::GyroSetting { setting: *setting, value: *value });
                        }
                    }
                }
            }
            Part::Touchpad => {
                if let Some(t) = &self.touchpad {
                    out.push(Change::TouchMode { mode: t.touch.clone() });
                    out.push(Change::TouchClick { half: Side::Left, action: t.left_click.clone() });
                    out.push(Change::TouchClick { half: Side::Right, action: t.right_click.clone() });
                    for (setting, value) in &t.settings {
                        out.push(Change::TouchSetting { setting: *setting, value: *value });
                    }
                }
            }
        }
        out
    }

    /// Which parts differ from another view (the drawing's amber dots / "Changed from Steam's layout" list).
    pub fn changed_parts(&self, steam: &PadView) -> Vec<crate::parts::Part> {
        use crate::parts::Part;
        let mut out = Vec::new();
        for b in &self.buttons {
            if steam.buttons.iter().find(|x| x.id == b.id).map(|x| x.presses != b.presses || x.settings != b.settings) != Some(false) {
                out.push(Part::Button(b.id));
            }
        }
        for s in &self.sticks {
            if steam.sticks.iter().find(|x| x.side == s.side) != Some(s) {
                out.push(Part::Stick(s.side));
            }
        }
        for t in &self.triggers {
            if steam.triggers.iter().find(|x| x.side == t.side) != Some(t) {
                out.push(Part::Trigger(t.side));
            }
        }
        if self.gyro.is_some() && self.gyro != steam.gyro {
            out.push(Part::Gyro);
        }
        if self.touchpad.is_some() && self.touchpad != steam.touchpad {
            out.push(Part::Touchpad);
        }
        out
    }
}
