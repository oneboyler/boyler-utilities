//! The sounds: made by our OWN small synth (Order 058) — no recordings from anyone, so no licence questions. Each pack is
//! a few parts (a pitched tone that glides, or a burst of band-passed noise) with a fast attack and an exponential decay;
//! [`render`] turns them into the five sounds of a pack at the output's sample rate. Deterministic: the same pack at the
//! same rate is the same samples (the noise comes from a fixed-seed generator). The "slight random pitch" that keeps it
//! from sounding robotic is applied per press by the mixer (a playback-rate nudge), not here.
//!
//! Clean keyboard packs: Linear (smooth thock), Tactile (soft bump), Clicky (crisp click), Typewriter. Satisfying packs:
//! Bubble, Glass tap, Water drop, Wood block, Marble.

use crate::kind::{Kind, ALL_KINDS, KINDS};
use std::f32::consts::TAU;
use std::sync::Arc;

/// The nine built-in packs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PackId {
    Linear,
    Tactile,
    Clicky,
    Typewriter,
    Bubble,
    GlassTap,
    WaterDrop,
    WoodBlock,
    Marble,
}

impl PackId {
    pub const ALL: [PackId; 9] = [
        PackId::Linear,
        PackId::Tactile,
        PackId::Clicky,
        PackId::Typewriter,
        PackId::Bubble,
        PackId::GlassTap,
        PackId::WaterDrop,
        PackId::WoodBlock,
        PackId::Marble,
    ];

    /// The name stored in settings (stable) and shown in the page.
    pub fn key(self) -> &'static str {
        match self {
            PackId::Linear => "linear",
            PackId::Tactile => "tactile",
            PackId::Clicky => "clicky",
            PackId::Typewriter => "typewriter",
            PackId::Bubble => "bubble",
            PackId::GlassTap => "glass-tap",
            PackId::WaterDrop => "water-drop",
            PackId::WoodBlock => "wood-block",
            PackId::Marble => "marble",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            PackId::Linear => "Linear",
            PackId::Tactile => "Tactile",
            PackId::Clicky => "Clicky",
            PackId::Typewriter => "Typewriter",
            PackId::Bubble => "Bubble",
            PackId::GlassTap => "Glass tap",
            PackId::WaterDrop => "Water drop",
            PackId::WoodBlock => "Wood block",
            PackId::Marble => "Marble",
        }
    }

    pub fn from_key(s: &str) -> Option<PackId> {
        PackId::ALL.into_iter().find(|p| p.key() == s)
    }

    /// The first four are the clean keyboard sounds, the rest the satisfying ones (the page groups them so).
    pub fn is_keyboard(self) -> bool {
        matches!(self, PackId::Linear | PackId::Tactile | PackId::Clicky | PackId::Typewriter)
    }
}

/// The five sounds of one pack (mono, f32) at one sample rate. Cloning is cheap (shared samples).
#[derive(Debug, Clone)]
pub struct SoundSet {
    pub rate: u32,
    pub sounds: [Arc<[f32]>; KINDS],
}

impl SoundSet {
    pub fn get(&self, k: Kind) -> &Arc<[f32]> {
        &self.sounds[k as usize]
    }
}

#[derive(Clone, Copy)]
enum Wave {
    Sine,
    Triangle,
}

#[derive(Clone, Copy)]
enum Src {
    /// A tone gliding (exponentially) from `f0` to `f1` Hz.
    Tone { wave: Wave, f0: f32, f1: f32 },
    /// White noise through a band-pass at `hz` with quality `q`.
    Noise { hz: f32, q: f32 },
}

/// What a part is in a real key press (Order 059): the layers of a keyboard sound are a short noise TRANSIENT (the cap
/// hitting), a damped BODY resonance (the key and plate ringing for a moment) and the case TAIL (a low, soft rumble). A key
/// coming up has the transient but little body and tail; Space / Enter have a bigger tail. The "satisfying" packs are one
/// plain part each (`Single`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Layer {
    Hit,
    Body,
    Tail,
    Single,
}

/// One part of a sound: it starts `at` seconds in, lasts `dur`, peaks at `gain`.
#[derive(Clone, Copy)]
struct Part {
    src: Src,
    dur: f32,
    gain: f32,
    at: f32,
    layer: Layer,
}

const fn tone(wave: Wave, f0: f32, f1: f32, dur: f32, gain: f32) -> Part {
    Part { src: Src::Tone { wave, f0, f1 }, dur, gain, at: 0.0, layer: Layer::Single }
}
const fn tone_at(wave: Wave, f0: f32, f1: f32, dur: f32, gain: f32, at: f32) -> Part {
    Part { src: Src::Tone { wave, f0, f1 }, dur, gain, at, layer: Layer::Single }
}
const fn noise(hz: f32, q: f32, dur: f32, gain: f32) -> Part {
    Part { src: Src::Noise { hz, q }, dur, gain, at: 0.0, layer: Layer::Single }
}
const fn layer(p: Part, layer: Layer) -> Part {
    Part { layer, ..p }
}

/// The parts of a pack, and the low-pass (Hz) that rounds its top off (no harsh ping, no hiss).
fn parts(p: PackId) -> (Vec<Part>, f32) {
    use Layer::{Body, Hit, Tail};
    use Wave::*;
    match p {
        // smooth thock: a soft tick, a deep damped body (+ a plate mode), a low case rumble
        PackId::Linear => (
            vec![
                layer(noise(2300.0, 0.8, 0.012, 0.26), Hit),
                layer(tone(Sine, 178.0, 122.0, 0.17, 0.78), Body),
                layer(tone(Sine, 340.0, 270.0, 0.08, 0.26), Body),
                layer(noise(420.0, 0.6, 0.2, 0.24), Tail),
                layer(tone(Sine, 96.0, 80.0, 0.22, 0.3), Tail),
            ],
            8500.0,
        ),
        // a soft bump under a short tap
        PackId::Tactile => (
            vec![
                layer(noise(3000.0, 1.0, 0.014, 0.46), Hit),
                layer(tone(Sine, 215.0, 142.0, 0.13, 0.66), Body),
                layer(tone(Triangle, 520.0, 420.0, 0.06, 0.2), Body),
                layer(noise(600.0, 0.7, 0.15, 0.2), Tail),
                layer(tone(Sine, 110.0, 90.0, 0.16, 0.2), Tail),
            ],
            9000.0,
        ),
        // crisp: a bright snap, a hard little edge, a short body, a thin tail
        PackId::Clicky => (
            vec![
                layer(noise(4300.0, 2.2, 0.02, 0.9), Hit),
                layer(tone(Triangle, 3100.0, 2300.0, 0.012, 0.13), Hit),
                layer(tone(Sine, 260.0, 170.0, 0.1, 0.46), Body),
                layer(noise(900.0, 0.8, 0.1, 0.18), Tail),
            ],
            9500.0,
        ),
        // clack + a low thud + a hint of the type bar's ring + the carriage's rumble
        PackId::Typewriter => (
            vec![
                layer(noise(1800.0, 1.1, 0.035, 0.8), Hit),
                layer(tone(Sine, 112.0, 60.0, 0.15, 0.7), Body),
                layer(tone(Triangle, 2400.0, 2100.0, 0.05, 0.13), Body),
                layer(noise(380.0, 0.6, 0.22, 0.24), Tail),
            ],
            8500.0,
        ),
        // a bubble popping up, a small echo of it
        PackId::Bubble => (vec![tone(Sine, 420.0, 980.0, 0.12, 0.65), tone_at(Sine, 840.0, 1700.0, 0.07, 0.1, 0.01)], 6000.0),
        // a glass tap: two lower partials that ring briefly (no needle-high ping)
        PackId::GlassTap => (vec![tone(Sine, 1900.0, 1880.0, 0.26, 0.42), tone(Sine, 2850.0, 2830.0, 0.15, 0.13), noise(4500.0, 1.5, 0.008, 0.14)], 5200.0),
        // a drop falling into water, a smaller one after it
        PackId::WaterDrop => (vec![tone(Sine, 1250.0, 480.0, 0.16, 0.6), tone_at(Sine, 900.0, 420.0, 0.1, 0.25, 0.07)], 6000.0),
        // a wooden block
        PackId::WoodBlock => (vec![tone(Triangle, 760.0, 690.0, 0.07, 0.8), noise(1500.0, 1.4, 0.025, 0.5)], 4800.0),
        // a marble on stone
        PackId::Marble => (vec![tone(Sine, 1500.0, 1250.0, 0.07, 0.62), tone(Sine, 2250.0, 2150.0, 0.14, 0.18), noise(4200.0, 1.5, 0.01, 0.25)], 5200.0),
    }
}

/// (pitch, length, loudness) of each of the five sounds, relative to the plain key down. Space / Enter / Backspace sit lower.
fn variant(k: Kind) -> (f32, f32, f32) {
    match k {
        Kind::Down => (1.0, 1.0, 1.0),
        Kind::Up => (1.25, 0.55, 0.55),
        Kind::Space => (0.72, 1.2, 1.0),
        Kind::Enter => (0.8, 1.12, 1.0),
        Kind::Backspace => (0.9, 0.95, 0.95),
    }
}

/// How much of each layer a sound has: a key coming up is mostly the transient; Space / Enter have a bigger case tail.
fn layer_mul(k: Kind, l: Layer) -> f32 {
    match (k, l) {
        (_, Layer::Single) => 1.0,
        (Kind::Down, _) | (Kind::Backspace, _) => 1.0,
        (Kind::Up, Layer::Hit) => 0.9,
        (Kind::Up, Layer::Body) => 0.28,
        (Kind::Up, Layer::Tail) => 0.12,
        (Kind::Space, Layer::Hit) => 0.8,
        (Kind::Space, Layer::Body) => 1.0,
        (Kind::Space, Layer::Tail) => 1.6,
        (Kind::Enter, Layer::Hit) => 0.9,
        (Kind::Enter, Layer::Body) => 1.0,
        (Kind::Enter, Layer::Tail) => 1.3,
    }
}

/// The loudest sample of the key-down sound after rendering (the other sounds keep their level relative to it). The
/// user's volume (see `rules::gain`) scales from here, so a pack is never louder than another by accident.
pub const PEAK: f32 = 0.8;

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> f32 {
        // xorshift32 → [-1, 1)
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

fn render_kind(pack: PackId, kind: Kind, rate: u32) -> Vec<f32> {
    let (ps, soft_hz) = parts(pack);
    let seed = 0x9E37_79B9 ^ (pack as u32 + 1).wrapping_mul(2654435761) ^ (kind as u32 + 1).wrapping_mul(40503);
    render_parts(&ps, soft_hz, kind, variant(kind), seed, rate)
}

/// One sound from `ps`: `kind` picks the layer mix (a key up is mostly the transient), `(pitch, len, loud)` scale it.
fn render_parts(ps: &[Part], soft_hz: f32, kind: Kind, (pitch, len, loud): (f32, f32, f32), seed: u32, rate: u32) -> Vec<f32> {
    let sr = rate as f32;
    let total = ps.iter().map(|p| p.at + p.dur * len).fold(0.0f32, f32::max) + 0.004;
    let mut out = vec![0.0f32; (total * sr).ceil() as usize + 1];
    let mut rng = Rng(seed);
    for part in ps.iter().copied() {
        let dur = part.dur * len;
        let start = (part.at * sr) as usize;
        let n = ((dur * sr) as usize).min(out.len().saturating_sub(start));
        let gain = part.gain * loud * layer_mul(kind, part.layer);
        let decay = (1.0e-4f32).ln();
        match part.src {
            Src::Tone { wave, f0, f1 } => {
                let (f0, f1) = (f0 * pitch, f1 * pitch);
                let ratio = f1 / f0;
                let mut phase = 0.0f32;
                for i in 0..n {
                    let t = i as f32 / sr;
                    let x = t / dur;
                    let f = f0 * ratio.powf(x);
                    phase += TAU * f / sr;
                    if phase > TAU {
                        phase -= TAU;
                    }
                    let s = match wave {
                        Wave::Sine => phase.sin(),
                        Wave::Triangle => (2.0 / std::f32::consts::PI) * phase.sin().asin(),
                    };
                    out[start + i] += s * gain * (decay * x).exp() * (t / 0.002).min(1.0);
                }
            }
            Src::Noise { hz, q } => {
                let hz = (hz * pitch).min(sr * 0.45);
                // RBJ band-pass, constant 0 dB peak
                let w0 = TAU * hz / sr;
                let alpha = w0.sin() / (2.0 * q);
                let a0 = 1.0 + alpha;
                let (b0, b2) = (alpha / a0, -alpha / a0);
                let (a1, a2) = (-2.0 * w0.cos() / a0, (1.0 - alpha) / a0);
                // a narrow band lets little of the white noise through: make that up, so Q doesn't decide the loudness
                let comp = ((sr * 0.5) / (hz / q)).sqrt().min(20.0);
                let (mut z1, mut z2) = (0.0f32, 0.0f32);
                for i in 0..n {
                    let t = i as f32 / sr;
                    let x = t / dur;
                    let inp = rng.next();
                    let y = b0 * inp + z1;
                    z1 = -a1 * y + z2;
                    z2 = b2 * inp - a2 * y;
                    out[start + i] += y * comp * 0.5 * gain * (decay * x).exp() * (t / 0.0015).min(1.0);
                }
            }
        }
    }
    // round the top off: two one-pole low-passes (12 dB / octave) - no needle-high ping, no hiss
    let a = 1.0 - (-TAU * soft_hz.min(sr * 0.45) / sr).exp();
    let (mut y1, mut y2) = (0.0f32, 0.0f32);
    for x in out.iter_mut() {
        y1 += a * (*x - y1);
        y2 += a * (y1 - y2);
        *x = y2;
    }
    out
}

/// The five sounds of `pack` at `rate` Hz.
pub fn render(pack: PackId, rate: u32) -> SoundSet {
    let mut raw: Vec<Vec<f32>> = ALL_KINDS.iter().map(|&k| render_kind(pack, k, rate)).collect();
    let peak = raw[Kind::Down as usize].iter().fold(0.0f32, |m, x| m.max(x.abs())).max(1.0e-6);
    let scale = PEAK / peak;
    let fade = (rate as usize / 500).max(1); // the last 2 ms go to 0, so no sound can end on a click
    for v in raw.iter_mut() {
        // a sound with a big tail (Space, Enter) can come out louder than the plain key: never past 0.95 (no clipping)
        let own = v.iter().fold(0.0f32, |m, x| m.max(x.abs())) * scale;
        let scale = if own > 0.95 { scale * 0.95 / own } else { scale };
        for x in v.iter_mut() {
            *x *= scale;
        }
        let n = v.len();
        for i in 0..fade.min(n) {
            v[n - 1 - i] *= i as f32 / fade as f32;
        }
    }
    let mut it = raw.into_iter().map(|v| Arc::<[f32]>::from(v.into_boxed_slice()));
    SoundSet { rate, sounds: std::array::from_fn(|_| it.next().expect("five sounds")) }
}

// ------------------------------------------------------------------ mouse clicks (Order 064)

/// The mouse click sounds, one per pack character (the owner: the mouse clicks "need different sounds" from the keys, yet fit the
/// pack): a soft silent switch, a light optical tick, a crisp micro-switch, a deeper clicky; the satisfying packs get a short,
/// clean tick made from their own sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClickStyle {
    Silent,
    Optical,
    Micro,
    Deep,
    Tick(PackId),
}

impl ClickStyle {
    /// A pack the user imported has no click of its own: the plain optical tick.
    pub const IMPORTED: ClickStyle = ClickStyle::Optical;

    /// The click that goes with a built-in pack: Linear → silent switch, Tactile → optical, Clicky → micro-switch,
    /// Typewriter → deep click; each satisfying pack → a tick of its own.
    pub fn of(pack: PackId) -> ClickStyle {
        match pack {
            PackId::Linear => ClickStyle::Silent,
            PackId::Tactile => ClickStyle::Optical,
            PackId::Clicky => ClickStyle::Micro,
            PackId::Typewriter => ClickStyle::Deep,
            p => ClickStyle::Tick(p),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ClickStyle::Silent => "Silent switch",
            ClickStyle::Optical => "Optical",
            ClickStyle::Micro => "Micro-switch",
            ClickStyle::Deep => "Deep click",
            ClickStyle::Tick(_) => "Tick",
        }
    }

    fn id(self) -> u32 {
        match self {
            ClickStyle::Silent => 0,
            ClickStyle::Optical => 1,
            ClickStyle::Micro => 2,
            ClickStyle::Deep => 3,
            ClickStyle::Tick(p) => 4 + p as u32,
        }
    }
}

/// The parts of a click style and the low-pass that rounds its top off.
fn click_parts(s: ClickStyle) -> (Vec<Part>, f32) {
    use Layer::{Body, Hit, Tail};
    use Wave::*;
    match s {
        // soft: a muffled tap and a short round body, nothing sharp
        ClickStyle::Silent => (
            vec![layer(noise(1900.0, 0.8, 0.012, 0.34), Hit), layer(tone(Sine, 250.0, 165.0, 0.075, 0.6), Body), layer(noise(520.0, 0.6, 0.09, 0.14), Tail)],
            6500.0,
        ),
        // light and clean: a small tick over a short high-ish body
        ClickStyle::Optical => (
            vec![layer(noise(3200.0, 1.2, 0.008, 0.5), Hit), layer(tone(Sine, 980.0, 640.0, 0.032, 0.42), Body), layer(tone(Sine, 1900.0, 1500.0, 0.02, 0.1), Body)],
            8000.0,
        ),
        // crisp: a bright snap, a hard little edge, a short body
        ClickStyle::Micro => (
            vec![
                layer(noise(4000.0, 1.9, 0.009, 0.9), Hit),
                layer(tone(Triangle, 2700.0, 2100.0, 0.009, 0.12), Hit),
                layer(tone(Sine, 430.0, 300.0, 0.045, 0.38), Body),
                layer(noise(1300.0, 0.8, 0.05, 0.09), Tail),
            ],
            9000.0,
        ),
        // deeper: a firm clack on a low body
        ClickStyle::Deep => (
            vec![
                layer(noise(2400.0, 1.5, 0.014, 0.8), Hit),
                layer(tone(Sine, 195.0, 125.0, 0.1, 0.72), Body),
                layer(tone(Triangle, 1300.0, 950.0, 0.02, 0.18), Body),
                layer(noise(700.0, 0.7, 0.11, 0.16), Tail),
            ],
            8500.0,
        ),
        ClickStyle::Tick(p) => parts(p),
    }
}

pub use bu_rawin::MouseButtonClass;

/// The six click sounds of a style (left, right, wheel click; each down and up) at one sample rate.
#[derive(Debug, Clone)]
pub struct ClickSet {
    pub rate: u32,
    sounds: [Arc<[f32]>; 6],
}

impl ClickSet {
    /// The sound of a button going down / up; None for the side buttons (they play the pack's key sound).
    pub fn get(&self, b: MouseButtonClass, down: bool) -> Option<&Arc<[f32]>> {
        let i = match b {
            MouseButtonClass::Left => 0,
            MouseButtonClass::Right => 1,
            MouseButtonClass::Middle => 2,
            MouseButtonClass::Side => return None,
        };
        Some(&self.sounds[i * 2 + usize::from(!down)])
    }
}

/// (pitch, length, loudness) of a button's click relative to the left button's: the right one a touch lower and softer, the
/// wheel click small, short and low. A tick (the satisfying packs) is also shorter and lighter than the pack's key sound.
fn click_variant(style: ClickStyle, button: usize, down: bool) -> (f32, f32, f32) {
    let (bp, bl, bg) = match button {
        0 => (1.0, 1.0, 1.0),
        1 => (0.93, 1.0, 0.95),
        _ => (0.78, 0.6, 0.7),
    };
    let (sp, sl, sg) = if matches!(style, ClickStyle::Tick(_)) { (1.0, 0.5, 0.8) } else { (1.0, 1.0, 1.0) };
    let (up_p, up_l, up_g) = if down { (1.0, 1.0, 1.0) } else { variant(Kind::Up) };
    (bp * sp * up_p, bl * sl * up_l, bg * sg * up_g)
}

/// The six click sounds of `style` at `rate` Hz. The left click's down peaks at [`PEAK`] like a key; the others keep their level
/// relative to it (never past 0.95).
pub fn render_clicks(style: ClickStyle, rate: u32) -> ClickSet {
    let (ps, soft_hz) = click_parts(style);
    let mut raw: Vec<Vec<f32>> = Vec::with_capacity(6);
    for button in 0..3usize {
        for down in [true, false] {
            let kind = if down { Kind::Down } else { Kind::Up };
            let seed = 0x7F4A_7C15 ^ (style.id() + 1).wrapping_mul(2654435761) ^ ((button * 2 + usize::from(!down)) as u32 + 1).wrapping_mul(40503);
            raw.push(render_parts(&ps, soft_hz, kind, click_variant(style, button, down), seed, rate));
        }
    }
    let peak = raw[0].iter().fold(0.0f32, |m, x| m.max(x.abs())).max(1.0e-6);
    let scale = PEAK / peak;
    let fade = (rate as usize / 500).max(1);
    for v in raw.iter_mut() {
        let own = v.iter().fold(0.0f32, |m, x| m.max(x.abs())) * scale;
        let scale = if own > 0.95 { scale * 0.95 / own } else { scale };
        for x in v.iter_mut() {
            *x *= scale;
        }
        let n = v.len();
        for i in 0..fade.min(n) {
            v[n - 1 - i] *= i as f32 / fade as f32;
        }
    }
    let mut it = raw.into_iter().map(|v| Arc::<[f32]>::from(v.into_boxed_slice()));
    ClickSet { rate, sounds: std::array::from_fn(|_| it.next().expect("six sounds")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pack_makes_five_clean_sounds() {
        for p in PackId::ALL {
            let set = render(p, 48_000);
            for k in ALL_KINDS {
                let s = set.get(k);
                assert!(s.len() > 100 && s.len() < 48_000 / 2, "{} {:?}: {} samples", p.name(), k, s.len());
                assert!(s.iter().all(|x| x.is_finite()), "{} {:?} has a NaN", p.name(), k);
                let peak = s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
                assert!(peak > 0.05 && peak <= 1.0, "{} {:?}: peak {}", p.name(), k, peak);
                assert!(s.last().unwrap().abs() < 1.0e-3, "{} {:?} ends on a click", p.name(), k);
            }
            let down = set.get(Kind::Down).iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!((down - PEAK).abs() < 0.01, "{}: key down peaks at {}", p.name(), down);
            let up = set.get(Kind::Up).iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(up < down, "{}: key up is softer than key down", p.name());
        }
    }


    /// Share of a sound's energy that a 2nd-order (RBJ) high-pass (`high`) or low-pass at `fc` lets through: a measure of how
    /// bright (ping, hiss) or deep (thock) a sound is.
    fn band_share(s: &[f32], rate: f32, fc: f32, high: bool) -> f32 {
        let w0 = TAU * fc / rate;
        let alpha = w0.sin() / (2.0 * 0.707);
        let c = w0.cos();
        let (b0, b1, b2) = if high { ((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0) } else { ((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0) };
        let (a0, a1, a2) = (1.0 + alpha, -2.0 * c, 1.0 - alpha);
        let (mut x1, mut x2, mut y1, mut y2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let (mut e_all, mut e_band) = (0.0f32, 0.0f32);
        for &x in s {
            let y = (b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2) / a0;
            (x2, x1, y2, y1) = (x1, x, y1, y);
            e_all += x * x;
            e_band += y * y;
        }
        e_band / e_all.max(1.0e-9)
    }

    /// Order 059: "no harsh ping". The satisfying packs keep (almost) nothing above 6 kHz; the keyboard packs are rounder than a
    /// raw noise click; a key's measured brightness is printed (run with --nocapture) so a change can be compared.
    #[test]
    fn no_pack_has_a_harsh_top() {
        for p in PackId::ALL {
            let set = render(p, 48_000);
            for k in ALL_KINDS {
                let share = band_share(set.get(k), 48_000.0, 6000.0, true);
                println!("{:<11} {:?}: {:.2} % of the energy above 6 kHz", p.name(), k, share * 100.0);
                let limit = if p.is_keyboard() { 0.12 } else { 0.03 };
                assert!(share < limit, "{} {:?}: {:.1} % above 6 kHz", p.name(), k, share * 100.0);
            }
        }
    }

    /// Order 059: Space / Enter sit deeper than a plain key (more of the energy below 110 Hz, a bigger case tail), and a key
    /// coming up is lighter than its press (less low body) in every keyboard pack.
    #[test]
    fn space_and_enter_are_deeper_and_the_release_is_lighter() {
        for p in PackId::ALL.into_iter().filter(|p| p.is_keyboard()) {
            let set = render(p, 48_000);
            let low = |k: Kind| band_share(set.get(k), 48_000.0, 110.0, false);
            assert!(low(Kind::Space) > low(Kind::Down), "{}: Space {:.2} vs key {:.2}", p.name(), low(Kind::Space), low(Kind::Down));
            assert!(low(Kind::Enter) > low(Kind::Down), "{}: Enter deeper than a key", p.name());
            assert!(low(Kind::Up) < low(Kind::Down), "{}: the release is lighter", p.name());
            assert!(set.get(Kind::Space).len() > set.get(Kind::Down).len(), "{}: Space rings longer", p.name());
        }
    }

    #[test]
    fn the_five_sounds_of_a_pack_differ() {
        for p in PackId::ALL {
            let set = render(p, 48_000);
            for a in 0..KINDS {
                for b in a + 1..KINDS {
                    assert_ne!(&*set.sounds[a], &*set.sounds[b], "{}: sounds {a} and {b} are the same", p.name());
                }
            }
        }
    }

    #[test]
    fn rendering_is_deterministic_and_follows_the_rate() {
        let a = render(PackId::Clicky, 48_000);
        let b = render(PackId::Clicky, 48_000);
        assert_eq!(&*a.sounds[0], &*b.sounds[0]);
        let c = render(PackId::Clicky, 44_100);
        let ratio = a.sounds[0].len() as f32 / c.sounds[0].len() as f32;
        assert!((ratio - 48_000.0 / 44_100.0).abs() < 0.02, "same length in seconds at another rate: {ratio}");
    }

    fn every_style() -> Vec<ClickStyle> {
        let mut v = vec![ClickStyle::Silent, ClickStyle::Optical, ClickStyle::Micro, ClickStyle::Deep];
        v.extend(PackId::ALL.into_iter().filter(|p| !p.is_keyboard()).map(ClickStyle::Tick));
        v
    }

    /// Order 064: every pack has a click that suits it, each style makes the six clean sounds, the side buttons have none here.
    #[test]
    fn every_pack_has_a_click_and_every_style_makes_six_clean_sounds() {
        let styles: std::collections::HashSet<_> = PackId::ALL.into_iter().map(ClickStyle::of).collect();
        assert_eq!(styles.len(), 9, "nine packs, nine different clicks: {styles:?}");
        assert_eq!(ClickStyle::of(PackId::Linear), ClickStyle::Silent);
        assert_eq!(ClickStyle::of(PackId::Clicky), ClickStyle::Micro);
        assert_eq!(ClickStyle::of(PackId::Bubble), ClickStyle::Tick(PackId::Bubble));
        for st in every_style() {
            let set = render_clicks(st, 48_000);
            assert!(set.get(MouseButtonClass::Side, true).is_none());
            let mut seen = Vec::new();
            for b in [MouseButtonClass::Left, MouseButtonClass::Right, MouseButtonClass::Middle] {
                for down in [true, false] {
                    let s = set.get(b, down).unwrap();
                    assert!(s.len() > 50 && s.len() < 48_000 / 3, "{st:?} {b:?} {down}: {} samples", s.len());
                    assert!(s.iter().all(|x| x.is_finite()), "{st:?} has a NaN");
                    let peak = s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
                    assert!(peak > 0.02 && peak <= 0.95 + 1e-4, "{st:?} {b:?} {down}: peak {peak}");
                    assert!(s.last().unwrap().abs() < 1.0e-3, "{st:?} ends on a click");
                    seen.push(s.clone());
                }
            }
            let left = set.get(MouseButtonClass::Left, true).unwrap().iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!((left - PEAK).abs() < 0.01, "{st:?}: the left click peaks at {left}");
            for a in 0..seen.len() {
                for b in a + 1..seen.len() {
                    assert_ne!(&*seen[a], &*seen[b], "{st:?}: sounds {a} and {b} are the same");
                }
            }
            let up = set.get(MouseButtonClass::Left, false).unwrap().iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(up < left, "{st:?}: the release is softer");
            let wheel = set.get(MouseButtonClass::Middle, true).unwrap();
            assert!(wheel.len() < set.get(MouseButtonClass::Left, true).unwrap().len(), "{st:?}: the wheel click is the small one");
        }
    }

    /// Order 064: the clicks are NOT the keyboard's sound (the owner: "the mouse clicks need different sounds"), and a click is
    /// no harsher than a key ("no harsh ping").
    #[test]
    fn clicks_differ_from_the_key_sounds_and_have_no_harsh_top() {
        for p in PackId::ALL {
            let key = render(p, 48_000);
            let click = render_clicks(ClickStyle::of(p), 48_000);
            assert_ne!(&**key.get(Kind::Down), &**click.get(MouseButtonClass::Left, true).unwrap(), "{}: the click is its own sound", p.name());
        }
        for st in every_style() {
            let set = render_clicks(st, 48_000);
            let limit = if matches!(st, ClickStyle::Tick(_)) { 0.03 } else { 0.12 };
            for b in [MouseButtonClass::Left, MouseButtonClass::Right, MouseButtonClass::Middle] {
                for down in [true, false] {
                    let share = band_share(set.get(b, down).unwrap(), 48_000.0, 6000.0, true);
                    println!("{st:?} {b:?} {down}: {:.2} % of the energy above 6 kHz", share * 100.0);
                    assert!(share < limit, "{st:?} {b:?}: {:.1} % above 6 kHz", share * 100.0);
                }
            }
        }
    }

    /// The styles keep their characters: the silent switch is the softest top, the micro-switch the brightest, the deep click
    /// has the most low energy.
    #[test]
    fn the_click_characters_are_ordered() {
        let bright = |st| band_share(render_clicks(st, 48_000).get(MouseButtonClass::Left, true).unwrap(), 48_000.0, 3000.0, true);
        let deep = |st| band_share(render_clicks(st, 48_000).get(MouseButtonClass::Left, true).unwrap(), 48_000.0, 300.0, false);
        assert!(bright(ClickStyle::Micro) > bright(ClickStyle::Deep), "micro {} vs deep {}", bright(ClickStyle::Micro), bright(ClickStyle::Deep));
        assert!(bright(ClickStyle::Deep) > bright(ClickStyle::Silent), "deep {} vs silent {}", bright(ClickStyle::Deep), bright(ClickStyle::Silent));
        assert!(deep(ClickStyle::Deep) > deep(ClickStyle::Micro), "deep {} vs micro {}", deep(ClickStyle::Deep), deep(ClickStyle::Micro));
        assert!(deep(ClickStyle::Silent) > deep(ClickStyle::Optical));
    }

    #[test]
    fn the_packs_have_their_names_and_groups() {
        assert_eq!(PackId::ALL.iter().filter(|p| p.is_keyboard()).count(), 4);
        for p in PackId::ALL {
            assert_eq!(PackId::from_key(p.key()), Some(p));
        }
        assert_eq!(PackId::from_key("nope"), None);
        let names: Vec<_> = PackId::ALL.iter().map(|p| p.name()).collect();
        assert_eq!(names, ["Linear", "Tactile", "Clicky", "Typewriter", "Bubble", "Glass tap", "Water drop", "Wood block", "Marble"]);
    }
}
