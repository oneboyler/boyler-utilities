//! The bytes Raw Accel's driver takes (WRITE ioctl) and gives back (READ ioctl), laid out like the v1.7.0 C++ structs
//! compiled for x64 (MSVC, default packing): `io_base` + N × `modifier_settings` (profile + `init_data`'s precomputed
//! `data_t`) + M × `device_settings`. The driver never validates or recomputes — it uses these bytes as they come — so
//! they are built exactly the way Raw Accel's wrapper builds them (zero-initialised, then each member written).
//! The layout is proven against a READ of a live driver in the order report (`mouse-show`: "byte-exact").

use super::args::*;
use super::curves::{init_data, Accel, ModifierData};

pub const IO_BASE: usize = 40;
pub const ACCEL_ARGS: usize = 2184;
pub const SPEED_ARGS: usize = 40;
pub const PROFILE: usize = 5016;
pub const DATA_T: usize = 168;
pub const MODIFIER_SETTINGS: usize = PROFILE + DATA_T; // 5184
pub const DEVICE_CONFIG: usize = 32;
pub const DEVICE_SETTINGS: usize = 1456;
pub const ACCEL_UNION: usize = 72;

/// Total WRITE size for n profiles and m devices.
pub fn write_size(n: usize, m: usize) -> usize {
    IO_BASE + n * MODIFIER_SETTINGS + m * DEVICE_SETTINGS
}

struct W<'a>(&'a mut [u8]);
impl W<'_> {
    fn f64(&mut self, at: usize, v: f64) {
        self.0[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, at: usize, v: f32) {
        self.0[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, at: usize, v: i32) {
        self.0[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, at: usize, v: u32) {
        self.0[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn b(&mut self, at: usize, v: bool) {
        self.0[at] = v as u8;
    }
    /// wchar_t[cap], NUL-terminated (ByValTStr keeps at most cap − 1 characters)
    fn wstr(&mut self, at: usize, cap: usize, s: &str) {
        for (i, c) in s.encode_utf16().take(cap - 1).enumerate() {
            self.0[at + 2 * i..at + 2 * i + 2].copy_from_slice(&c.to_le_bytes());
        }
    }
}

fn put_device_config(w: &mut W, at: usize, c: &DeviceConfig) {
    w.b(at, c.disable);
    w.b(at + 1, c.set_extra_info);
    w.b(at + 2, c.poll_time_lock);
    w.i32(at + 4, c.dpi);
    w.i32(at + 8, c.polling_rate);
    w.f64(at + 16, c.clamp_min);
    w.f64(at + 24, c.clamp_max);
}

fn put_accel_args(w: &mut W, at: usize, a: &AccelArgs, table: Option<&[f32]>) {
    w.i32(at, a.mode.as_i32());
    w.b(at + 4, a.gain);
    let fs = [a.input_offset, a.output_offset, a.acceleration, a.decay_rate, a.gamma, a.motivity, a.exponent_classic, a.scale, a.exponent_power, a.limit, a.sync_speed, a.smooth];
    for (i, v) in fs.iter().enumerate() {
        w.f64(at + 8 + 8 * i, *v);
    }
    w.f64(at + 104, a.cap.x);
    w.f64(at + 112, a.cap.y);
    w.i32(at + 120, a.cap_mode.as_i32());
    // `length` = the JSON data length (lut points); the synchronous table does not change it
    w.i32(at + 124, a.data.len().min(LUT_RAW_DATA_CAPACITY) as i32);
    let data: &[f32] = table.unwrap_or(&a.data);
    for (i, v) in data.iter().take(LUT_RAW_DATA_CAPACITY).enumerate() {
        w.f32(at + 128 + 4 * i, *v);
    }
}

fn put_union(w: &mut W, at: usize, u: &Accel) {
    match u {
        Accel::Noaccel => {}
        Accel::Lookup { size, velocity } => {
            w.i32(at, *size);
            w.b(at + 4, *velocity);
        }
        Accel::ClassicGain { accel_raised, cap, constant, sign } => {
            w.f64(at, *accel_raised);
            w.f64(at + 8, cap.x);
            w.f64(at + 16, cap.y);
            w.f64(at + 24, *constant);
            w.f64(at + 32, *sign);
        }
        Accel::ClassicLegacy { accel_raised, cap, sign } => {
            w.f64(at, *accel_raised);
            w.f64(at + 8, *cap);
            w.f64(at + 16, *sign);
        }
        Accel::JumpGain { step, smooth_rate, c } => {
            w.f64(at, step.x);
            w.f64(at + 8, step.y);
            w.f64(at + 16, *smooth_rate);
            w.f64(at + 24, *c);
        }
        Accel::JumpLegacy { step, smooth_rate } => {
            w.f64(at, step.x);
            w.f64(at + 8, step.y);
            w.f64(at + 16, *smooth_rate);
        }
        Accel::NaturalGain { offset, accel, limit, constant } => {
            w.f64(at, *offset);
            w.f64(at + 8, *accel);
            w.f64(at + 16, *limit);
            w.f64(at + 24, *constant);
        }
        Accel::NaturalLegacy { offset, accel, limit } => {
            w.f64(at, *offset);
            w.f64(at + 8, *accel);
            w.f64(at + 16, *limit);
        }
        Accel::PowerGain { offset, scale, constant, cap, constant_b } => {
            w.f64(at, offset.x);
            w.f64(at + 8, offset.y);
            w.f64(at + 16, *scale);
            w.f64(at + 24, *constant);
            w.f64(at + 32, cap.x);
            w.f64(at + 40, cap.y);
            w.f64(at + 48, *constant_b);
        }
        Accel::PowerLegacy { offset, scale, constant, cap } => {
            w.f64(at, offset.x);
            w.f64(at + 8, offset.y);
            w.f64(at + 16, *scale);
            w.f64(at + 24, *constant);
            w.f64(at + 32, *cap);
        }
        Accel::SyncGain { velocity, range, x_start, .. } => {
            w.b(at, *velocity);
            w.i32(at + 4, range.start);
            w.i32(at + 8, range.stop);
            w.i32(at + 12, range.num);
            w.f64(at + 16, *x_start);
        }
        Accel::SyncLegacy(s) => {
            w.f64(at, s.log_motivity);
            w.f64(at + 8, s.gamma_const);
            w.f64(at + 16, s.log_syncspeed);
            w.f64(at + 24, s.syncspeed);
            w.f64(at + 32, s.sharpness);
            w.f64(at + 40, s.sharpness_recip);
            w.b(at + 48, s.use_linear_clamp);
            w.f64(at + 56, s.minimum_sens);
            w.f64(at + 64, s.maximum_sens);
        }
    }
}

fn put_modifier_settings(w: &mut W, at: usize, p: &Profile, d: &ModifierData) {
    w.wstr(at, MAX_NAME_LEN, &p.name);
    w.f64(at + 512, p.domain_weights.x);
    w.f64(at + 520, p.domain_weights.y);
    w.f64(at + 528, p.range_weights.x);
    w.f64(at + 536, p.range_weights.y);
    put_accel_args(w, at + 544, &p.accel_x, d.accel_x.table());
    put_accel_args(w, at + 544 + ACCEL_ARGS, &p.accel_y, d.accel_y.table());
    let s = at + 4912;
    w.b(s, p.speed.whole);
    w.f64(s + 8, p.speed.lp_norm);
    w.f64(s + 16, p.speed.input_speed_smooth_halflife);
    w.f64(s + 24, p.speed.scale_smooth_halflife);
    w.f64(s + 32, p.speed.output_speed_smooth_halflife);
    let tail = [p.output_dpi, p.yx_output_dpi_ratio, p.lr_output_dpi_ratio, p.ud_output_dpi_ratio, p.degrees_rotation, p.degrees_snap, p.speed_min, p.speed_max];
    for (i, v) in tail.iter().enumerate() {
        w.f64(at + 4952 + 8 * i, *v);
    }
    // data_t
    let dt = at + PROFILE;
    let f = d.flags;
    for (i, b) in [f.apply_rotate, f.compute_ref_angle, f.apply_snap, f.clamp_speed, f.apply_directional_weight, f.apply_dir_mul_x, f.apply_dir_mul_y].iter().enumerate() {
        w.b(dt + i, *b);
    }
    w.f64(dt + 8, d.rot_direction.x);
    w.f64(dt + 16, d.rot_direction.y);
    put_union(w, dt + 24, &d.accel_x);
    put_union(w, dt + 24 + ACCEL_UNION, &d.accel_y);
}

/// The WRITE buffer for a whole settings.json (what Raw Accel's `DriverConfig::Activate` sends).
pub fn to_bytes(c: &DriverConfig) -> Vec<u8> {
    let mut buf = vec![0u8; write_size(c.profiles.len(), c.devices.len())];
    let mut w = W(&mut buf);
    put_device_config(&mut w, 0, &c.default_device_config);
    w.u32(32, c.profiles.len() as u32);
    w.u32(36, c.devices.len() as u32);
    for (i, p) in c.profiles.iter().enumerate() {
        put_modifier_settings(&mut w, IO_BASE + i * MODIFIER_SETTINGS, p, &init_data(p));
    }
    let dev0 = IO_BASE + c.profiles.len() * MODIFIER_SETTINGS;
    for (i, d) in c.devices.iter().enumerate() {
        let at = dev0 + i * DEVICE_SETTINGS;
        w.wstr(at, MAX_NAME_LEN, &d.name);
        w.wstr(at + 512, MAX_NAME_LEN, &d.profile);
        w.wstr(at + 1024, MAX_DEV_ID_LEN, &d.id);
        put_device_config(&mut w, at + 1424, &d.config);
    }
    buf
}

/// Byte ranges that are struct padding (never read by the driver; their content depends on how the C++ copy was made).
pub fn padding_ranges(n_profiles: usize, n_devices: usize) -> Vec<std::ops::Range<usize>> {
    let mut r = vec![3..4, 12..16];
    for i in 0..n_profiles {
        let at = IO_BASE + i * MODIFIER_SETTINGS;
        for a in [at + 544, at + 544 + ACCEL_ARGS] {
            r.push(a + 5..a + 8);
        }
        r.push(at + 4912 + 1..at + 4912 + 8);
        r.push(at + PROFILE + 7..at + PROFILE + 8);
    }
    let dev0 = IO_BASE + n_profiles * MODIFIER_SETTINGS;
    for i in 0..n_devices {
        let at = dev0 + i * DEVICE_SETTINGS + 1424;
        r.push(at + 3..at + 4);
        r.push(at + 12..at + 16);
    }
    r
}

/// Differences between two WRITE/READ buffers, padding ignored: (offset, ours, theirs), at most `max` entries.
pub fn diff(ours: &[u8], theirs: &[u8], n_profiles: usize, n_devices: usize, max: usize) -> Vec<(usize, u8, u8)> {
    let pad = padding_ranges(n_profiles, n_devices);
    let mut out = Vec::new();
    for i in 0..ours.len().max(theirs.len()) {
        if pad.iter().any(|r| r.contains(&i)) {
            continue;
        }
        let (a, b) = (ours.get(i).copied().unwrap_or(0xEE), theirs.get(i).copied().unwrap_or(0xEE));
        if a != b {
            out.push((i, a, b));
            if out.len() >= max {
                break;
            }
        }
    }
    out
}

/// The header of a READ: (modifier count, device count, default device config).
pub fn read_header(b: &[u8]) -> Option<(u32, u32)> {
    (b.len() >= IO_BASE).then(|| (u32::from_le_bytes([b[32], b[33], b[34], b[35]]), u32::from_le_bytes([b[36], b[37], b[38], b[39]])))
}

fn rd_f64(b: &[u8], at: usize) -> f64 {
    f64::from_le_bytes(b[at..at + 8].try_into().unwrap_or([0; 8]))
}
fn rd_i32(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(b[at..at + 4].try_into().unwrap_or([0; 4]))
}
fn rd_wstr(b: &[u8], at: usize, cap: usize) -> String {
    let w: Vec<u16> = (0..cap).map(|i| u16::from_le_bytes([b[at + 2 * i], b[at + 2 * i + 1]])).take_while(|c| *c != 0).collect();
    String::from_utf16_lossy(&w)
}

fn rd_accel_args(b: &[u8], at: usize) -> AccelArgs {
    let f = |i: usize| rd_f64(b, at + 8 + 8 * i);
    let len = rd_i32(b, at + 124).clamp(0, LUT_RAW_DATA_CAPACITY as i32) as usize;
    AccelArgs {
        mode: AccelMode::from_i32(rd_i32(b, at)).unwrap_or(AccelMode::Noaccel),
        gain: b[at + 4] != 0,
        input_offset: f(0),
        output_offset: f(1),
        acceleration: f(2),
        decay_rate: f(3),
        gamma: f(4),
        motivity: f(5),
        exponent_classic: f(6),
        scale: f(7),
        exponent_power: f(8),
        limit: f(9),
        sync_speed: f(10),
        smooth: f(11),
        cap: Vec2 { x: rd_f64(b, at + 104), y: rd_f64(b, at + 112) },
        cap_mode: CapMode::from_i32(rd_i32(b, at + 120)).unwrap_or(CapMode::Output),
        data: (0..len).map(|i| f32::from_le_bytes(b[at + 128 + 4 * i..at + 132 + 4 * i].try_into().unwrap_or([0; 4]))).collect(),
    }
}

/// The profiles the driver runs now, decoded from a READ (only the profile parts; `data_t` is derived from them).
pub fn read_profiles(b: &[u8]) -> Vec<Profile> {
    let Some((n, _)) = read_header(b) else { return vec![] };
    let mut out = Vec::new();
    for i in 0..n as usize {
        let at = IO_BASE + i * MODIFIER_SETTINGS;
        if b.len() < at + MODIFIER_SETTINGS {
            break;
        }
        let s = at + 4912;
        out.push(Profile {
            name: rd_wstr(b, at, MAX_NAME_LEN),
            domain_weights: Vec2 { x: rd_f64(b, at + 512), y: rd_f64(b, at + 520) },
            range_weights: Vec2 { x: rd_f64(b, at + 528), y: rd_f64(b, at + 536) },
            accel_x: rd_accel_args(b, at + 544),
            accel_y: rd_accel_args(b, at + 544 + ACCEL_ARGS),
            speed: SpeedArgs { whole: b[s] != 0, lp_norm: rd_f64(b, s + 8), input_speed_smooth_halflife: rd_f64(b, s + 16), scale_smooth_halflife: rd_f64(b, s + 24), output_speed_smooth_halflife: rd_f64(b, s + 32) },
            output_dpi: rd_f64(b, at + 4952),
            yx_output_dpi_ratio: rd_f64(b, at + 4960),
            lr_output_dpi_ratio: rd_f64(b, at + 4968),
            ud_output_dpi_ratio: rd_f64(b, at + 4976),
            degrees_rotation: rd_f64(b, at + 4984),
            degrees_snap: rd_f64(b, at + 4992),
            speed_min: rd_f64(b, at + 5000),
            speed_max: rd_f64(b, at + 5008),
        });
    }
    out
}
