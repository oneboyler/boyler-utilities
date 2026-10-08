//! The soft end chime. Made in memory from the drawing's recipe (menu-v18 `playSound('chime', 1)`): three sine tones —
//! 1046.5 Hz (peak .26, .5 s), 1568 Hz (.2, .6 s) and 2093 Hz (.04, .3 s), the last two 70 ms later — every frequency
//! ×1.12 (the drawing's "up" pitch), each with a 6 ms exponential attack and an exponential fall to silence, through a
//! 5200 Hz low-pass, output gain .9. 44.1 kHz mono 16-bit WAV, ~0.7 s.
//!
//! Played through [`SoundOs`]: [`RealSound`] = `PlaySoundW(SND_MEMORY | SND_ASYNC | SND_NODEFAULT)` on the default output
//! device; [`FakeSound`] (tests) only records. Tests and examples never play anything.

use crate::{Result, TimerError};
use std::sync::OnceLock;

pub const SAMPLE_RATE: u32 = 44_100;

/// Plays a WAV file held in memory.
pub trait SoundOs {
    fn play_wav(&mut self, wav: &'static [u8]) -> Result<()>;
}

/// Records what would have played. `fail` makes every play fail (to test the error path).
#[derive(Debug, Default)]
pub struct FakeSound {
    pub played: Vec<usize>,
    pub fail: bool,
}

impl SoundOs for FakeSound {
    fn play_wav(&mut self, wav: &'static [u8]) -> Result<()> {
        if self.fail {
            return Err(TimerError::Os { context: "PlaySoundW".into(), code: 0x8000_4005 });
        }
        self.played.push(wav.len());
        Ok(())
    }
}

/// Windows' `PlaySoundW` from memory, asynchronous (returns at once; the buffer is static so it outlives the sound).
#[cfg(windows)]
#[derive(Debug, Default)]
pub struct RealSound;

#[cfg(windows)]
impl SoundOs for RealSound {
    fn play_wav(&mut self, wav: &'static [u8]) -> Result<()> {
        use windows::core::PCWSTR;
        use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
        // SAFETY: with SND_MEMORY the first argument points at the WAV bytes; they are 'static.
        let ok = unsafe { PlaySoundW(PCWSTR(wav.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT) };
        if ok.as_bool() {
            Ok(())
        } else {
            Err(TimerError::Os { context: "PlaySoundW".into(), code: 0 })
        }
    }
}

/// The chime's samples (-1.0 … 1.0).
pub fn chime_samples() -> Vec<f32> {
    let p = 1.12f64;
    // (start s, frequency Hz, length s, peak)
    let tones = [(0.0, 1046.5 * p, 0.5, 0.26), (0.07, 1568.0 * p, 0.6, 0.2), (0.07, 2093.0 * p, 0.3, 0.04)];
    let sr = SAMPLE_RATE as f64;
    let total = ((0.07 + 0.6 + 0.03) * sr) as usize;
    let mut out = vec![0f64; total];
    for (t0, f, dur, peak) in tones {
        let s0 = (t0 * sr) as usize;
        let n = ((dur + 0.03) * sr) as usize;
        for i in 0..n.min(total - s0) {
            let t = i as f64 / sr;
            out[s0 + i] += env(t, dur, peak) * (2.0 * std::f64::consts::PI * f * t).sin();
        }
    }
    // the drawing's 5200 Hz low-pass (WebAudio biquad, Q = 1), then the .9 output gain
    let (b0, b1, b2, a1, a2) = lowpass(5200.0, 1.0, sr);
    let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
    out.iter()
        .map(|&x| {
            let y = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
            (x2, x1, y2, y1) = (x1, x, y1, y);
            (y * 0.9) as f32
        })
        .collect()
}

/// WebAudio's exponential ramps: .0001 → peak in 6 ms, then → .0001 at `dur`; silent after.
fn env(t: f64, dur: f64, peak: f64) -> f64 {
    let floor = 0.0001f64;
    if t < 0.006 {
        floor * (peak / floor).powf(t / 0.006)
    } else if t < dur {
        peak * (floor / peak).powf((t - 0.006) / (dur - 0.006))
    } else {
        0.0
    }
}

/// RBJ cookbook low-pass (what WebAudio's BiquadFilter "lowpass" uses), normalised.
fn lowpass(f0: f64, q: f64, sr: f64) -> (f64, f64, f64, f64, f64) {
    let w = 2.0 * std::f64::consts::PI * f0 / sr;
    let alpha = w.sin() / (2.0 * q);
    let c = w.cos();
    let a0 = 1.0 + alpha;
    ((1.0 - c) / 2.0 / a0, (1.0 - c) / a0, (1.0 - c) / 2.0 / a0, -2.0 * c / a0, (1.0 - alpha) / a0)
}

/// The chime plays at this volume (the owner, Oct 8: sounds are off by default and quiet, "5 %"). Same decibel curve as the
/// mic mute sounds: 100 % = the recipe above, then −[`RANGE_DB`] dB over the slider.
pub const CHIME_VOLUME: u8 = 5;
/// 1 % is this many dB under 100 %.
pub const RANGE_DB: f32 = 40.0;

/// The multiplier for a volume 0–100 on the decibel curve (0 → silence).
pub fn volume_gain(volume: u8) -> f32 {
    let v = f32::from(volume.min(100));
    if v <= 0.0 {
        return 0.0;
    }
    10f32.powf(-RANGE_DB * (1.0 - v / 100.0) / 20.0)
}

/// The chime as a WAV file in memory (made once), at [`CHIME_VOLUME`].
pub fn chime_wav() -> &'static [u8] {
    static WAV: OnceLock<Vec<u8>> = OnceLock::new();
    WAV.get_or_init(|| {
        let g = volume_gain(CHIME_VOLUME);
        let quiet: Vec<f32> = chime_samples().iter().map(|s| s * g).collect();
        wav_16bit_mono(&quiet, SAMPLE_RATE)
    })
}

/// A plain PCM WAV (RIFF) file.
pub fn wav_16bit_mono(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + data_len as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&1u16.to_le_bytes()); // mono
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let x = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        v.extend_from_slice(&x.to_le_bytes());
    }
    v
}
