//! The mute / unmute sounds (DESIGN §3.5 row 3 "Sound"): Soft click, Blip down, Blip up, Chime, None; a volume 0–100.
//!
//! No sound files ship yet: each sound is made in code as a short 16-bit mono WAV (44.1 kHz) and handed to
//! [`SoundOut::play_wav`] — the real one plays it with `PlaySoundW(SND_MEMORY | SND_ASYNC)`. Volume scales the samples
//! on a decibel curve (see [`gain`]), so 0 % is silence, 5 % is really quiet and the file itself carries the loudness —
//! Windows' own volume stays untouched.

use crate::os::SoundOut;
use crate::Result;
use std::f32::consts::TAU;

/// The sounds the popups offer (DESIGN: "Soft click, Blip down, Blip up, Chime, None").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sound {
    SoftClick,
    BlipDown,
    BlipUp,
    Chime,
    None,
}

impl Sound {
    pub const ALL: [Sound; 5] = [Sound::SoftClick, Sound::BlipDown, Sound::BlipUp, Sound::Chime, Sound::None];

    /// The popup label.
    pub fn label(self) -> &'static str {
        match self {
            Sound::SoftClick => "Soft click",
            Sound::BlipDown => "Blip down",
            Sound::BlipUp => "Blip up",
            Sound::Chime => "Chime",
            Sound::None => "None",
        }
    }

    /// The ▶ preview button is disabled for None (DESIGN).
    pub fn can_preview(self) -> bool {
        self != Sound::None
    }
}

/// The Sound row's settings. Defaults (the owner, Oct 8): OFF, "Blip down" on mute, "Blip up" on unmute, volume 5 %.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundSettings {
    pub enabled: bool,
    pub on_mute: Sound,
    pub on_unmute: Sound,
    /// 0–100 (values above 100 count as 100).
    pub volume: u8,
}

impl Default for SoundSettings {
    fn default() -> Self {
        SoundSettings { enabled: false, on_mute: Sound::BlipDown, on_unmute: Sound::BlipUp, volume: DEFAULT_VOLUME }
    }
}

impl SoundSettings {
    /// The sound for a change to `muted`, or `None` when the switch is off / the sound is None / volume 0.
    pub fn sound_for(&self, muted: bool) -> Option<Sound> {
        let s = if muted { self.on_mute } else { self.on_unmute };
        (self.enabled && s != Sound::None && self.volume > 0).then_some(s)
    }
}

pub const SAMPLE_RATE: u32 = 44_100;
/// Loudest sample at volume 100, as a share of full scale (headroom so no sound is harsh).
pub const PEAK_AT_FULL: f32 = 0.5;
/// The volume a fresh install starts at (the owner, Oct 8: "like 5%").
pub const DEFAULT_VOLUME: u8 = 5;
/// Volume 1 % is this many dB below volume 100 % (0 % is silence).
pub const RANGE_DB: f32 = 40.0;

/// The sample multiplier for a volume 0–100 (above 100 counts as 100): 0 → silence, otherwise `PEAK_AT_FULL` at 100 %
/// falling by [`RANGE_DB`] dB over the slider, i.e. a perceptual curve. (Before Order 046 it was linear amplitude: 60 % was
/// only 4 dB under 100 %, so the old default sat at about −10 dBFS in the ear's most sensitive range — "extremely loud".)
pub fn gain(volume: u8) -> f32 {
    let v = f32::from(volume.min(100));
    if v <= 0.0 {
        return 0.0;
    }
    PEAK_AT_FULL * 10f32.powf(-RANGE_DB * (1.0 - v / 100.0) / 20.0)
}

/// The WAV bytes for `sound` at `volume` (0–100). `None` for [`Sound::None`].
pub fn wav(sound: Sound, volume: u8) -> Option<Vec<u8>> {
    let samples = samples(sound)?;
    let gain = gain(volume);
    let pcm: Vec<i16> = samples.iter().map(|s| (s * gain * 32767.0).round().clamp(-32768.0, 32767.0) as i16).collect();
    Some(wav_from_pcm(&pcm, SAMPLE_RATE))
}

/// Plays `sound` through `out` (the "Picking a sound plays it" / ▶ preview / volume-release blip). None → nothing.
pub fn play(out: &dyn SoundOut, sound: Sound, volume: u8) -> Result<()> {
    match wav(sound, volume) {
        Some(bytes) => out.play_wav(bytes),
        None => Ok(()),
    }
}

/// Unit-peak samples (−1…1) of each sound. Deterministic: the same bytes every time.
fn samples(sound: Sound) -> Option<Vec<f32>> {
    let sr = SAMPLE_RATE as f32;
    let n = |ms: f32| (sr * ms / 1000.0) as usize;
    Some(match sound {
        Sound::None => return None,
        // 18 ms tick: a 2.2 kHz tone with a fast exponential decay
        Sound::SoftClick => (0..n(18.0)).map(|i| {
            let t = i as f32 / sr;
            (TAU * 2200.0 * t).sin() * (-t / 0.004).exp()
        }).collect(),
        // 110 ms sweeps (down 880 → 440 Hz / up 440 → 880 Hz), 6 ms fade in, 40 ms fade out
        Sound::BlipDown => sweep(880.0, 440.0, n(110.0), n(6.0), n(40.0)),
        Sound::BlipUp => sweep(440.0, 880.0, n(110.0), n(6.0), n(40.0)),
        // 420 ms: C6 + G6 bell-like, exponential decay
        Sound::Chime => {
            let len = n(420.0);
            let attack = n(4.0);
            (0..len).map(|i| {
                let t = i as f32 / sr;
                let a = if i < attack { i as f32 / attack as f32 } else { 1.0 };
                let v = 0.62 * (TAU * 1046.5 * t).sin() + 0.38 * (TAU * 1568.0 * t).sin();
                v * a * (-t / 0.12).exp()
            }).collect()
        }
    })
}

/// A sine sweeping linearly from `f0` to `f1` over `len` samples, with linear fade in / out.
fn sweep(f0: f32, f1: f32, len: usize, fade_in: usize, fade_out: usize) -> Vec<f32> {
    let sr = SAMPLE_RATE as f32;
    let dur = len as f32 / sr;
    (0..len)
        .map(|i| {
            let t = i as f32 / sr;
            // phase of a linear chirp: 2π (f0 t + (f1 − f0) t² / 2T)
            let phase = TAU * (f0 * t + (f1 - f0) * t * t / (2.0 * dur));
            let a_in = if i < fade_in { i as f32 / fade_in as f32 } else { 1.0 };
            let left = len - i;
            let a_out = if left < fade_out { left as f32 / fade_out as f32 } else { 1.0 };
            phase.sin() * a_in * a_out
        })
        .collect()
}

/// A canonical 44-byte-header PCM WAV (mono, 16-bit).
pub fn wav_from_pcm(pcm: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&sample_rate.to_le_bytes());
    w.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    w.extend_from_slice(&2u16.to_le_bytes()); // block align
    w.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}
