//! Raw Accel's acceleration maths, ported line by line from its v1.7.0 source (tag v1.7.0 = commit d179e22e, MIT):
//! `common/accel-classic.hpp`, `accel-jump.hpp`, `accel-natural.hpp`, `accel-power.hpp`, `accel-synchronous.hpp`,
//! `accel-lookup.hpp`, `accel-noaccel.hpp`, `accel-union.hpp`, `rawaccel.hpp` (`modifier::modify`, `init_data`),
//! `rawaccel-validate.hpp`. Same operations in the same order, so the values (and the precomputed bytes the driver gets)
//! are the ones Raw Accel itself computes.
//!
//! `Accel::new(args)` = the C++ constructor of the mode's struct (what `init_data` puts in the driver's union);
//! `Accel::eval(x, args)` = its `operator()` — the sensitivity multiplier at input speed `x` (counts/ms at 1000 DPI).

use super::args::{AccelArgs, AccelMode, CapMode, Profile, Vec2, LUT_RAW_DATA_CAPACITY};
use std::f64::consts::PI;

const DBL_MAX: f64 = f64::MAX;

fn minsd(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else {
        b
    }
}
fn maxsd(a: f64, b: f64) -> f64 {
    if b < a {
        a
    } else {
        b
    }
}
fn clampsd(v: f64, lo: f64, hi: f64) -> f64 {
    minsd(maxsd(v, lo), hi)
}

/// `rawaccel::ilogb`: the unbiased exponent bits.
pub fn ilogb(x: f64) -> i32 {
    ((x.to_bits() >> 52) & 0x7ff) as i32 - 0x3ff
}

/// `rawaccel::scalbn`: x · 2^n for n in [-1022, 1023].
pub fn scalbn(x: f64, n: i32) -> f64 {
    x * f64::from_bits(((0x3ff + n) as u64) << 52)
}

/// `rawaccel::lerp`.
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    let x = a + t * (b - a);
    if (t > 1.0) == (a < b) {
        return maxsd(x, b);
    }
    minsd(x, b)
}

/// `fp_rep_range` — [2^start, 2^stop] with `num` linear steps per power of two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FpRepRange {
    pub start: i32,
    pub stop: i32,
    pub num: i32,
}

impl FpRepRange {
    fn for_each(&self, mut f: impl FnMut(f64)) {
        for e in 0..(self.stop - self.start) {
            let exp_scale = scalbn(1.0, e + self.start) / self.num as f64;
            for i in 0..self.num {
                f((i + self.num) as f64 * exp_scale);
            }
        }
        f(scalbn(1.0, self.stop));
    }
    pub fn size(&self) -> i32 {
        (self.stop - self.start) * self.num + 1
    }
}

/// The per-mode precomputed state (`accel_union` member). Field order = C++ member order (see `bytes.rs`).
#[derive(Clone, Debug, PartialEq)]
pub enum Accel {
    Noaccel,
    Lookup { size: i32, velocity: bool },
    ClassicGain { accel_raised: f64, cap: Vec2, constant: f64, sign: f64 },
    ClassicLegacy { accel_raised: f64, cap: f64, sign: f64 },
    JumpGain { step: Vec2, smooth_rate: f64, c: f64 },
    JumpLegacy { step: Vec2, smooth_rate: f64 },
    NaturalGain { offset: f64, accel: f64, limit: f64, constant: f64 },
    NaturalLegacy { offset: f64, accel: f64, limit: f64 },
    PowerGain { offset: Vec2, scale: f64, constant: f64, cap: Vec2, constant_b: f64 },
    PowerLegacy { offset: Vec2, scale: f64, constant: f64, cap: f64 },
    /// activation_framework<GAIN>: its table lives in the profile's `accel_args.data` (97 floats)
    SyncGain { velocity: bool, range: FpRepRange, x_start: f64, table: Vec<f32> },
    SyncLegacy(SyncLegacy),
}

/// activation_framework<LEGACY>.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SyncLegacy {
    pub log_motivity: f64,
    pub gamma_const: f64,
    pub log_syncspeed: f64,
    pub syncspeed: f64,
    pub sharpness: f64,
    pub sharpness_recip: f64,
    pub use_linear_clamp: bool,
    pub minimum_sens: f64,
    pub maximum_sens: f64,
}

impl SyncLegacy {
    fn new(a: &AccelArgs) -> Self {
        let log_motivity = a.motivity.ln();
        let sharpness = if a.smooth == 0.0 { 16.0 } else { 0.5 / a.smooth };
        Self {
            log_motivity,
            gamma_const: a.gamma / log_motivity,
            log_syncspeed: a.sync_speed.ln(),
            syncspeed: a.sync_speed,
            sharpness,
            sharpness_recip: 1.0 / sharpness,
            use_linear_clamp: sharpness >= 16.0,
            minimum_sens: 1.0 / a.motivity,
            maximum_sens: a.motivity,
        }
    }

    fn eval(&self, x: f64) -> f64 {
        if self.use_linear_clamp {
            let log_space = self.gamma_const * (x.ln() - self.log_syncspeed);
            if log_space < -1.0 {
                return self.minimum_sens;
            }
            if log_space > 1.0 {
                return self.maximum_sens;
            }
            return (log_space * self.log_motivity).exp();
        }
        if x == self.syncspeed {
            return 1.0;
        }
        let log_x = x.ln();
        let log_diff = log_x - self.log_syncspeed;
        if log_diff > 0.0 {
            let log_space = self.gamma_const * log_diff;
            let exponent = log_space.powf(self.sharpness).tanh().powf(self.sharpness_recip);
            (exponent * self.log_motivity).exp()
        } else {
            let log_space = -self.gamma_const * log_diff;
            let exponent = -log_space.powf(self.sharpness).tanh().powf(self.sharpness_recip);
            (exponent * self.log_motivity).exp()
        }
    }
}

// ---- classic helpers (classic_base / classic<GAIN> statics) ----
fn classic_base_fn(x: f64, accel_raised: f64, a: &AccelArgs) -> f64 {
    accel_raised * (x - a.input_offset).powf(a.exponent_classic) / x
}
fn classic_base_accel(x: f64, y: f64, a: &AccelArgs) -> f64 {
    let power = a.exponent_classic;
    (x * y * (x - a.input_offset).powf(-power)).powf(1.0 / (power - 1.0))
}
fn classic_gain(x: f64, accel: f64, power: f64, offset: f64) -> f64 {
    power * (accel * (x - offset)).powf(power - 1.0)
}
fn classic_gain_inverse(y: f64, accel: f64, power: f64, offset: f64) -> f64 {
    (accel * offset + (y / power).powf(1.0 / (power - 1.0))) / accel
}
fn classic_gain_accel(x: f64, y: f64, power: f64, offset: f64) -> f64 {
    -(y / power).powf(1.0 / (power - 1.0)) / (offset - x)
}

// ---- power helpers ----
fn power_gain(input: f64, power: f64, scale: f64) -> f64 {
    (power + 1.0) * (input * scale).powf(power)
}
fn power_gain_inverse(gain: f64, power: f64, scale: f64) -> f64 {
    (gain / (power + 1.0)).powf(1.0 / power) / scale
}
fn power_scale_from_gain_point(input: f64, gain: f64, power: f64) -> f64 {
    (gain / (power + 1.0)).powf(1.0 / power) / input
}
fn power_scale_from_output_point(input: f64, output: f64, power: f64, c: f64) -> f64 {
    (output - c / input).powf(1.0 / power) / input
}
fn power_base_fn(offset: Vec2, scale: f64, constant: f64, x: f64, a: &AccelArgs) -> f64 {
    if x <= offset.x {
        offset.y
    } else {
        (scale * x).powf(a.exponent_power) + constant / x
    }
}

/// power_base constructor → (offset, scale, constant).
fn power_base(a: &AccelArgs) -> (Vec2, f64, f64) {
    let n = a.exponent_power;
    let scale = if a.cap_mode != CapMode::InOut {
        a.scale
    } else if a.gain {
        power_scale_from_gain_point(a.cap.x, a.cap.y, n)
    } else {
        // legacy + io: offset ignored (circular dependency scale -> constant -> offset)
        let constant = 0.0;
        let scale = power_scale_from_output_point(a.cap.x, a.cap.y, n, constant);
        return (Vec2 { x: 0.0, y: 0.0 }, scale, constant);
    };
    let ox = power_gain_inverse(a.output_offset, n, scale);
    let oy = a.output_offset;
    let constant = ox * oy * n / (n + 1.0);
    (Vec2 { x: ox, y: oy }, scale, constant)
}

impl Accel {
    /// The C++ constructor for `args.mode` / `args.gain` (`accel_union::visit` + `impl = { args }`).
    pub fn new(a: &AccelArgs) -> Accel {
        match (a.mode, a.gain) {
            (AccelMode::Classic, true) => {
                let accel_raised: f64;
                let mut cap = Vec2 { x: DBL_MAX, y: DBL_MAX };
                let mut constant = 0.0;
                let mut sign = 1.0;
                match a.cap_mode {
                    CapMode::InOut => {
                        cap.x = a.cap.x;
                        cap.y = a.cap.y - 1.0;
                        if cap.y < 0.0 {
                            cap.y = -cap.y;
                            sign = -sign;
                        }
                        let acc = classic_gain_accel(cap.x, cap.y, a.exponent_classic, a.input_offset);
                        accel_raised = acc.powf(a.exponent_classic - 1.0);
                        constant = (classic_base_fn(cap.x, accel_raised, a) - cap.y) * cap.x;
                    }
                    CapMode::Input => {
                        accel_raised = a.acceleration.powf(a.exponent_classic - 1.0);
                        if a.cap.x > 0.0 {
                            cap.x = a.cap.x;
                            cap.y = classic_gain(cap.x, a.acceleration, a.exponent_classic, a.input_offset);
                            constant = (classic_base_fn(cap.x, accel_raised, a) - cap.y) * cap.x;
                        }
                    }
                    CapMode::Output => {
                        accel_raised = a.acceleration.powf(a.exponent_classic - 1.0);
                        if a.cap.y > 0.0 {
                            cap.y = a.cap.y - 1.0;
                            if cap.y == 0.0 {
                                cap.x = 0.0;
                            } else {
                                if cap.y < 0.0 {
                                    cap.y = -cap.y;
                                    sign = -sign;
                                }
                                cap.x = classic_gain_inverse(cap.y, a.acceleration, a.exponent_classic, a.input_offset);
                                constant = (classic_base_fn(cap.x, accel_raised, a) - cap.y) * cap.x;
                            }
                        }
                    }
                }
                Accel::ClassicGain { accel_raised, cap, constant, sign }
            }
            (AccelMode::Classic, false) => {
                let accel_raised: f64;
                let mut cap = DBL_MAX;
                let mut sign = 1.0;
                match a.cap_mode {
                    CapMode::InOut => {
                        cap = a.cap.y - 1.0;
                        if cap < 0.0 {
                            cap = -cap;
                            sign = -sign;
                        }
                        let acc = classic_base_accel(a.cap.x, cap, a);
                        accel_raised = acc.powf(a.exponent_classic - 1.0);
                    }
                    CapMode::Input => {
                        accel_raised = a.acceleration.powf(a.exponent_classic - 1.0);
                        if a.cap.x > 0.0 {
                            cap = classic_base_fn(a.cap.x, accel_raised, a);
                        }
                    }
                    CapMode::Output => {
                        accel_raised = a.acceleration.powf(a.exponent_classic - 1.0);
                        if a.cap.y > 0.0 {
                            cap = a.cap.y - 1.0;
                            if cap < 0.0 {
                                cap = -cap;
                                sign = -sign;
                            }
                        }
                    }
                }
                Accel::ClassicLegacy { accel_raised, cap, sign }
            }
            (AccelMode::Jump, gain) => {
                let step = Vec2 { x: a.cap.x, y: a.cap.y - 1.0 };
                let rate_inverse = a.smooth * step.x;
                let smooth_rate = if rate_inverse < 1.0 { 0.0 } else { 2.0 * PI / rate_inverse };
                if gain {
                    let c = -jump_smooth_antideriv(step, smooth_rate, 0.0);
                    Accel::JumpGain { step, smooth_rate, c }
                } else {
                    Accel::JumpLegacy { step, smooth_rate }
                }
            }
            (AccelMode::Natural, gain) => {
                let offset = a.input_offset;
                let limit = a.limit - 1.0;
                let accel = a.decay_rate / limit.abs();
                if gain {
                    Accel::NaturalGain { offset, accel, limit, constant: -limit / accel }
                } else {
                    Accel::NaturalLegacy { offset, accel, limit }
                }
            }
            (AccelMode::Power, false) => {
                let (offset, scale, constant) = power_base(a);
                let mut cap = DBL_MAX;
                match a.cap_mode {
                    CapMode::InOut => cap = a.cap.y,
                    CapMode::Input => {
                        if a.cap.x > 0.0 {
                            cap = power_base_fn(offset, scale, constant, a.cap.x, a);
                        }
                    }
                    CapMode::Output => {
                        if a.cap.y > 0.0 {
                            cap = a.cap.y;
                        }
                    }
                }
                Accel::PowerLegacy { offset, scale, constant, cap }
            }
            (AccelMode::Power, true) => {
                let (offset, scale, constant) = power_base(a);
                let mut cap = Vec2 { x: DBL_MAX, y: DBL_MAX };
                match a.cap_mode {
                    CapMode::InOut => cap = a.cap,
                    CapMode::Input => {
                        if a.cap.x > 0.0 {
                            if a.cap.x <= offset.x {
                                return Accel::PowerGain { offset, scale, constant, cap: Vec2 { x: 0.0, y: offset.y }, constant_b: 0.0 };
                            }
                            cap.x = a.cap.x;
                            cap.y = power_gain(a.cap.x, a.exponent_power, scale);
                        }
                    }
                    CapMode::Output => {
                        if a.cap.y > 0.0 {
                            cap.x = power_gain_inverse(a.cap.y, a.exponent_power, scale);
                            cap.y = a.cap.y;
                        }
                    }
                }
                let constant_b = (power_base_fn(offset, scale, constant, cap.x, a) - cap.y) * cap.x;
                Accel::PowerGain { offset, scale, constant, cap, constant_b }
            }
            (AccelMode::Synchronous, false) => Accel::SyncLegacy(SyncLegacy::new(a)),
            (AccelMode::Synchronous, true) => {
                let range = FpRepRange { start: -3, stop: 9, num: 8 };
                let velocity = true;
                let sig = SyncLegacy::new(a);
                let mut sum = 0.0;
                let mut lo = 0.0;
                let mut table = Vec::with_capacity(range.size() as usize);
                range.for_each(|b| {
                    let partitions = 2;
                    let interval = (b - lo) / partitions as f64;
                    for i in 1..=partitions {
                        sum += sig.eval(lo + i as f64 * interval) * interval;
                    }
                    lo = b;
                    let mut y = sum;
                    if !velocity {
                        y /= b;
                    }
                    table.push(y as f32);
                });
                Accel::SyncGain { velocity, range, x_start: scalbn(1.0, range.start), table }
            }
            (AccelMode::Lut, gain) => Accel::Lookup { size: a.data.len() as i32 / 2, velocity: gain },
            (AccelMode::Noaccel, _) => Accel::Noaccel,
        }
    }

    /// `operator()(x, args)` — the sensitivity multiplier at input speed `x`.
    pub fn eval(&self, x: f64, a: &AccelArgs) -> f64 {
        match self {
            Accel::Noaccel => 1.0,
            Accel::ClassicGain { accel_raised, cap, constant, sign } => {
                if x <= a.input_offset {
                    return 1.0;
                }
                let output = if x < cap.x { classic_base_fn(x, *accel_raised, a) } else { constant / x + cap.y };
                sign * output + 1.0
            }
            Accel::ClassicLegacy { accel_raised, cap, sign } => {
                if x <= a.input_offset {
                    return 1.0;
                }
                sign * minsd(classic_base_fn(x, *accel_raised, a), *cap) + 1.0
            }
            Accel::JumpLegacy { step, smooth_rate } => {
                if *smooth_rate != 0.0 {
                    jump_smooth(*step, *smooth_rate, x) + 1.0
                } else if x < step.x {
                    1.0
                } else {
                    1.0 + step.y
                }
            }
            Accel::JumpGain { step, smooth_rate, c } => {
                if x <= 0.0 {
                    return 1.0;
                }
                if *smooth_rate != 0.0 {
                    return 1.0 + (jump_smooth_antideriv(*step, *smooth_rate, x) + c) / x;
                }
                if x < step.x {
                    1.0
                } else {
                    1.0 + step.y * (x - step.x) / x
                }
            }
            Accel::NaturalLegacy { offset, accel, limit } => {
                if x <= *offset {
                    return 1.0;
                }
                let offset_x = offset - x;
                let decay = (accel * offset_x).exp();
                limit * (1.0 - (offset - decay * offset_x) / x) + 1.0
            }
            Accel::NaturalGain { offset, accel, limit, constant } => {
                if x <= *offset {
                    return 1.0;
                }
                let offset_x = offset - x;
                let decay = (accel * offset_x).exp();
                let output = limit * (decay / accel - offset_x) + constant;
                output / x + 1.0
            }
            Accel::PowerLegacy { offset, scale, constant, cap } => minsd(power_base_fn(*offset, *scale, *constant, x, a), *cap),
            Accel::PowerGain { offset, scale, constant, cap, constant_b } => {
                if x < cap.x {
                    power_base_fn(*offset, *scale, *constant, x, a)
                } else {
                    cap.y + constant_b / x
                }
            }
            Accel::SyncLegacy(s) => s.eval(x),
            Accel::SyncGain { velocity, range, x_start, table } => {
                let capacity = LUT_RAW_DATA_CAPACITY as i32;
                let e = ilogb(x).min(range.stop - 1);
                if e >= range.start {
                    let idx_int_log_part = e - range.start;
                    let idx_frac_lin_part = scalbn(x, -e) - 1.0;
                    let idx_f = range.num as f64 * (idx_int_log_part as f64 + idx_frac_lin_part);
                    let idx = (idx_f as i32).min(range.size() - 2);
                    if idx < capacity - 1 {
                        let i = idx as usize;
                        let mut y = lerp(table.get(i).copied().unwrap_or(0.0) as f64, table.get(i + 1).copied().unwrap_or(0.0) as f64, idx_f - idx as f64);
                        if *velocity {
                            y /= x;
                        }
                        return y;
                    }
                }
                let mut y = table.first().copied().unwrap_or(0.0) as f64;
                if *velocity {
                    y /= x_start;
                }
                y
            }
            Accel::Lookup { size, velocity } => lookup_eval(&a.data, *size, *velocity, x),
        }
    }

    /// The table this mode writes into its profile's `accel_args.data` (synchronous + gain only).
    pub fn table(&self) -> Option<&[f32]> {
        match self {
            Accel::SyncGain { table, .. } => Some(table),
            _ => None,
        }
    }
}

fn jump_decay(step: Vec2, smooth_rate: f64, x: f64) -> f64 {
    (smooth_rate * (step.x - x)).exp()
}
fn jump_smooth(step: Vec2, smooth_rate: f64, x: f64) -> f64 {
    step.y / (1.0 + jump_decay(step, smooth_rate, x))
}
fn jump_smooth_antideriv(step: Vec2, smooth_rate: f64, x: f64) -> f64 {
    step.y * (x + (1.0 + jump_decay(step, smooth_rate, x)).ln() / smooth_rate)
}

fn lookup_eval(data: &[f32], size: i32, velocity: bool, x: f64) -> f64 {
    let capacity = (LUT_RAW_DATA_CAPACITY / 2) as i32;
    let p = |i: i32| -> (f64, f64) {
        let i = i as usize * 2;
        (data.get(i).copied().unwrap_or(0.0) as f64, data.get(i + 1).copied().unwrap_or(0.0) as f64)
    };
    let mut lo = 0;
    let mut hi = size - 2;
    if x <= 0.0 {
        return 0.0;
    }
    if hi < capacity - 1 {
        while lo <= hi {
            let mid = (lo + hi) / 2;
            let (px, py) = p(mid);
            if x < px {
                hi = mid - 1;
            } else if x > px {
                lo = mid + 1;
            } else {
                let mut y = py;
                if velocity {
                    y /= x;
                }
                return y;
            }
        }
        if lo > 0 {
            let (ax, ay) = p(lo - 1);
            let (bx, by) = p(lo);
            let t = (x - ax) / (bx - ax);
            let mut y = lerp(ay, by, t);
            if velocity {
                y /= x;
            }
            return y;
        }
    }
    let (x0, y0) = p(0);
    let mut y = y0;
    if velocity {
        y /= x0;
    }
    y
}

/// `modifier_flags(profile)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModifierFlags {
    pub apply_rotate: bool,
    pub compute_ref_angle: bool,
    pub apply_snap: bool,
    pub clamp_speed: bool,
    pub apply_directional_weight: bool,
    pub apply_dir_mul_x: bool,
    pub apply_dir_mul_y: bool,
}

impl ModifierFlags {
    pub fn new(p: &Profile) -> Self {
        let apply_snap = p.degrees_snap != 0.0;
        let apply_directional_weight = p.speed.whole && p.range_weights.x != p.range_weights.y;
        Self {
            clamp_speed: p.speed_max > 0.0 && p.speed_min <= p.speed_max,
            apply_rotate: p.degrees_rotation != 0.0,
            apply_snap,
            apply_directional_weight,
            compute_ref_angle: apply_snap || apply_directional_weight,
            apply_dir_mul_x: p.lr_output_dpi_ratio != 1.0,
            apply_dir_mul_y: p.ud_output_dpi_ratio != 1.0,
        }
    }
}

/// `modifier_settings::data_t` as `init_data` fills it.
#[derive(Clone, Debug, PartialEq)]
pub struct ModifierData {
    pub flags: ModifierFlags,
    pub rot_direction: Vec2,
    pub accel_x: Accel,
    pub accel_y: Accel,
}

/// `direction(degrees)`.
pub fn direction(degrees: f64) -> Vec2 {
    let r = degrees * PI / 180.0;
    Vec2 { x: r.cos(), y: r.sin() }
}

/// `init_data(settings)`.
pub fn init_data(p: &Profile) -> ModifierData {
    ModifierData { flags: ModifierFlags::new(p), rot_direction: direction(p.degrees_rotation), accel_x: Accel::new(&p.accel_x), accel_y: Accel::new(&p.accel_y) }
}

/// `modifier::modify` without smoothing (the grapher's "stateless copy"): the output vector for input counts `(x, y)`
/// over `time` ms with `dpi_factor`.
pub fn modify(p: &Profile, d: &ModifierData, input: Vec2, dpi_factor: f64, time: f64) -> Vec2 {
    let cb = |acc: &Accel, args: &AccelArgs, x: f64, w: f64| 1.0 + (acc.eval(x, args) - 1.0) * w;
    let mut v = input;
    let f = &d.flags;
    let mut reference_angle = 0.0;
    let ips_factor = dpi_factor / time;
    if f.apply_rotate {
        v = Vec2 { x: v.x * d.rot_direction.x - v.y * d.rot_direction.y, y: v.x * d.rot_direction.y + v.y * d.rot_direction.x };
    }
    if f.compute_ref_angle && v.y != 0.0 {
        if v.x == 0.0 {
            reference_angle = PI / 2.0;
        } else {
            reference_angle = (v.y / v.x).abs().atan();
            if f.apply_snap {
                let snap = p.degrees_snap * PI / 180.0;
                let mag = (v.x * v.x + v.y * v.y).sqrt();
                if reference_angle > PI / 2.0 - snap {
                    reference_angle = PI / 2.0;
                    v = Vec2 { x: 0.0, y: mag.copysign(v.y) };
                } else if reference_angle < snap {
                    reference_angle = 0.0;
                    v = Vec2 { x: mag.copysign(v.x), y: 0.0 };
                }
            }
        }
    }
    if f.clamp_speed {
        let speed = (v.x * v.x + v.y * v.y).sqrt() * ips_factor;
        let ratio = clampsd(speed, p.speed_min, p.speed_max) / speed;
        v.x *= ratio;
        v.y *= ratio;
    }
    let w = Vec2 { x: (v.x * ips_factor * p.domain_weights.x).abs(), y: (v.y * ips_factor * p.domain_weights.y).abs() };
    if !p.speed.whole {
        let sx = cb(&d.accel_x, &p.accel_x, w.x, p.range_weights.x);
        let sy = cb(&d.accel_y, &p.accel_y, w.y, p.range_weights.y);
        v.x *= sx;
        v.y *= sy;
    } else {
        let lp = p.speed.lp_norm;
        let speed = if lp >= 16.0 || lp <= 0.0 {
            maxsd(w.x, w.y)
        } else if lp != 2.0 {
            (w.x.abs().powf(lp) + w.y.abs().powf(lp)).powf(1.0 / lp)
        } else {
            (w.x * w.x + w.y * w.y).sqrt()
        };
        let mut weight = p.range_weights.x;
        if f.apply_directional_weight {
            let diff = p.range_weights.y - p.range_weights.x;
            weight += 2.0 / PI * reference_angle * diff;
        }
        let scale = cb(&d.accel_x, &p.accel_x, speed, weight);
        v.x *= scale;
        v.y *= scale;
    }
    let dpi_adjustment = p.output_dpi / super::args::NORMALIZED_DPI * dpi_factor;
    v.x *= dpi_adjustment;
    v.y *= dpi_adjustment * p.yx_output_dpi_ratio;
    if f.apply_dir_mul_x && v.x < 0.0 {
        v.x *= p.lr_output_dpi_ratio;
    }
    if f.apply_dir_mul_y && v.y < 0.0 {
        v.y *= p.ud_output_dpi_ratio;
    }
    v
}

/// What Raw Accel's "Sensitivity" chart plots at input speed `x` counts/ms: |output| / |input| for a pure horizontal
/// movement over 1 ms with dpi_factor 1 (the grapher's call) — includes the sens multiplier (Output DPI / 1000).
pub fn sensitivity(p: &Profile, d: &ModifierData, x: f64) -> f64 {
    if x <= 0.0 {
        return sensitivity(p, d, f64::MIN_POSITIVE);
    }
    let out = modify(p, d, Vec2 { x, y: 0.0 }, 1.0, 1.0);
    (out.x * out.x + out.y * out.y).sqrt() / x
}

/// `rawaccel::valid(profile)` — the exact messages Raw Accel's writer would refuse with (empty = valid).
pub fn validate(p: &Profile) -> Vec<String> {
    let mut errs: Vec<String> = Vec::new();
    let check = |a: &AccelArgs, errs: &mut Vec<String>| {
        let mut e = |m: &str| errs.push(m.to_string());
        if a.mode == AccelMode::Lut {
            if a.data.len() < 4 {
                e("lookup mode requires at least 2 points");
            } else if a.data.len() > LUT_RAW_DATA_CAPACITY {
                e("too many data points (max=257)");
            }
        } else if a.data.len() > LUT_RAW_DATA_CAPACITY {
            e("data size > max");
        }
        if a.input_offset < 0.0 {
            e("offset can not be negative");
        }
        if a.output_offset < 0.0 {
            e("offset can not be negative");
        }
        let jump_or_io_cap = a.mode == AccelMode::Jump || ((a.mode == AccelMode::Classic || a.mode == AccelMode::Power) && a.cap_mode == CapMode::InOut);
        if a.cap.x < 0.0 {
            e("cap (input) can not be negative");
        } else if a.cap.x == 0.0 && jump_or_io_cap {
            e("cap (input) can not be 0");
        }
        if a.cap.y < 0.0 {
            e("cap (output) can not be negative");
        } else if a.cap.y == 0.0 && jump_or_io_cap {
            e("cap (output) can not be 0");
        }
        if (a.mode == AccelMode::Classic && a.cap.x > 0.0 && a.cap.x < a.input_offset && a.cap_mode != CapMode::Output)
            || (a.mode == AccelMode::Power && a.cap.y > 0.0 && a.cap.y < a.output_offset && a.cap_mode != CapMode::Input)
        {
            e("cap < offset");
        }
        if a.acceleration <= 0.0 {
            e("acceleration must be positive");
        }
        if a.scale <= 0.0 {
            e("scale must be positive");
        }
        if a.gamma <= 0.0 {
            e("gamma must be positive");
        }
        if a.decay_rate <= 0.0 {
            e("decay rate must be positive");
        }
        if a.motivity <= 1.0 {
            e("motivity must be greater than 1");
        }
        if a.exponent_classic <= 1.0 {
            e("exponent must be greater than 1");
        }
        if a.exponent_power <= 0.0 {
            e("exponent must be positive");
        }
        if a.limit <= 0.0 {
            e("limit must be positive");
        }
        if a.sync_speed <= 0.0 {
            e("synchronous speed must be positive");
        }
        if a.smooth < 0.0 || a.smooth > 1.0 {
            e("smooth must be between 0 and 1");
        }
    };
    check(&p.accel_x, &mut errs);
    if !p.speed.whole {
        check(&p.accel_y, &mut errs);
    }
    let mut e = |m: &str| errs.push(m.to_string());
    if p.name.is_empty() {
        e("profile name can not be empty");
    }
    if p.speed_max < 0.0 {
        e("speed cap is negative");
    } else if p.speed_max < p.speed_min {
        e("max speed is less than min speed");
    }
    if p.degrees_snap < 0.0 || p.degrees_snap > 45.0 {
        e("snap angle must be between 0 and 45 degrees");
    }
    if p.output_dpi == 0.0 {
        e("output DPI is 0");
    }
    if p.yx_output_dpi_ratio == 0.0 {
        e("Y/X output DPI ratio is 0");
    }
    if p.domain_weights.x <= 0.0 || p.domain_weights.y <= 0.0 {
        e("domain weights must be positive");
    }
    if p.lr_output_dpi_ratio <= 0.0 || p.ud_output_dpi_ratio <= 0.0 {
        e("output DPI ratio must be positive");
    }
    if p.speed.lp_norm <= 0.0 {
        e("Lp norm must be positive (default=2)");
    }
    if p.range_weights.x < 0.0 || p.range_weights.y < 0.0 {
        e("range weights must be positive");
    }
    errs
}
