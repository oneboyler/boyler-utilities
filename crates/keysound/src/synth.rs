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
    Square,
}

#[derive(Clone, Copy)]
enum Src {
    /// A tone gliding (exponentially) from `f0` to `f1` Hz.
    Tone { wave: Wave, f0: f32, f1: f32 },
    /// White noise through a band-pass at `hz` with quality `q`.
    Noise { hz: f32, q: f32 },
}

/// One part of a sound: it starts `at` seconds in, lasts `dur`, peaks at `gain`.
#[derive(Clone, Copy)]
struct Part {
    src: Src,
    dur: f32,
    gain: f32,
    at: f32,
}

const fn tone(wave: Wave, f0: f32, f1: f32, dur: f32, gain: f32) -> Part {
    Part { src: Src::Tone { wave, f0, f1 }, dur, gain, at: 0.0 }
}
const fn tone_at(wave: Wave, f0: f32, f1: f32, dur: f32, gain: f32, at: f32) -> Part {
    Part { src: Src::Tone { wave, f0, f1 }, dur, gain, at }
}
const fn noise(hz: f32, q: f32, dur: f32, gain: f32) -> Part {
    Part { src: Src::Noise { hz, q }, dur, gain, at: 0.0 }
}

fn parts(p: PackId) -> Vec<Part> {
    use Wave::*;
    match p {
        // smooth thock: a low soft body + a muffled tick
        PackId::Linear => vec![tone(Sine, 150.0, 80.0, 0.11, 0.8), noise(1100.0, 0.7, 0.03, 0.35)],
        // a soft bump under a short tap
        PackId::Tactile => vec![tone(Sine, 190.0, 105.0, 0.08, 0.7), noise(2300.0, 1.0, 0.02, 0.55)],
        // crisp: a bright snap, a hard little edge, a tiny body
        PackId::Clicky => vec![noise(4300.0, 3.0, 0.018, 1.0), tone(Square, 3200.0, 1900.0, 0.012, 0.12), tone(Sine, 240.0, 140.0, 0.05, 0.3)],
        // clack + a low thud + a hint of the type bar's ring
        PackId::Typewriter => vec![noise(1700.0, 1.1, 0.03, 0.8), tone(Sine, 100.0, 55.0, 0.13, 0.7), tone(Triangle, 2500.0, 2100.0, 0.05, 0.15)],
        // a bubble popping up
        PackId::Bubble => vec![tone(Sine, 420.0, 980.0, 0.12, 0.65)],
        // a glass tap: two high partials that ring
        PackId::GlassTap => vec![tone(Sine, 2600.0, 2580.0, 0.34, 0.4), tone(Sine, 3900.0, 3880.0, 0.22, 0.2), noise(6000.0, 2.0, 0.008, 0.3)],
        // a drop falling into water, a smaller one after it
        PackId::WaterDrop => vec![tone(Sine, 1250.0, 480.0, 0.16, 0.6), tone_at(Sine, 900.0, 420.0, 0.1, 0.25, 0.07)],
        // a wooden block
        PackId::WoodBlock => vec![tone(Triangle, 760.0, 690.0, 0.07, 0.8), noise(1500.0, 1.4, 0.025, 0.5)],
        // a marble on stone
        PackId::Marble => vec![tone(Sine, 1900.0, 1560.0, 0.07, 0.6), tone(Sine, 2850.0, 2700.0, 0.16, 0.25), noise(5200.0, 2.0, 0.01, 0.4)],
    }
}

/// (pitch, length, loudness) of each of the five sounds, relative to the plain key down.
fn variant(k: Kind) -> (f32, f32, f32) {
    match k {
        Kind::Down => (1.0, 1.0, 1.0),
        Kind::Up => (1.3, 0.55, 0.5),
        Kind::Space => (0.8, 1.15, 1.0),
        Kind::Enter => (0.86, 1.1, 1.0),
        Kind::Backspace => (1.12, 0.9, 0.9),
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
    let (pitch, len, loud) = variant(kind);
    let sr = rate as f32;
    let ps = parts(pack);
    let total = ps.iter().map(|p| p.at + p.dur * len).fold(0.0f32, f32::max) + 0.004;
    let mut out = vec![0.0f32; (total * sr).ceil() as usize + 1];
    let mut rng = Rng(0x9E37_79B9 ^ (pack as u32 + 1).wrapping_mul(2654435761) ^ (kind as u32 + 1).wrapping_mul(40503));
    for part in ps {
        let dur = part.dur * len;
        let start = (part.at * sr) as usize;
        let n = ((dur * sr) as usize).min(out.len().saturating_sub(start));
        let gain = part.gain * loud;
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
                        Wave::Square => {
                            if phase < std::f32::consts::PI {
                                0.6
                            } else {
                                -0.6
                            }
                        }
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
    out
}

/// The five sounds of `pack` at `rate` Hz.
pub fn render(pack: PackId, rate: u32) -> SoundSet {
    let mut raw: Vec<Vec<f32>> = ALL_KINDS.iter().map(|&k| render_kind(pack, k, rate)).collect();
    let peak = raw[Kind::Down as usize].iter().fold(0.0f32, |m, x| m.max(x.abs())).max(1.0e-6);
    let scale = PEAK / peak;
    let fade = (rate as usize / 500).max(1); // the last 2 ms go to 0, so no sound can end on a click
    for v in raw.iter_mut() {
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
