//! The sounds (sound.c): soft sine tones made in code (3-4 choices per event), custom .wav files, one volume for all.
//! Every sound is scaled in memory before it plays, so the volume applies to custom files too. The maths is ClipPing's
//! exactly (its own sine, the same float order), so the bytes are the same.

use std::path::{Path, PathBuf};

use crate::settings::{Settings, SND_CUSTOM};

/// The four sound events ("Worked", "Didn't work", "Changed", "Warning").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    Saved = 0,
    Failed = 1,
    Changed = 2,
    Warning = 3,
}

pub const SOUND_LABELS: [&str; 4] = ["Worked", "Didn't work", "Changed", "Warning"];

const RATE: i32 = 22050;
const PEAK: f32 = 0.1259; // -18 dBFS at 100 % volume
const PI: f32 = std::f32::consts::PI;

#[derive(Clone, Copy)]
struct Tone {
    f0: f32,
    f1: f32,
    start: i32,
    dur: i32,
    amp: f32,
    h2: f32,
    h3: f32,
    att: i32,
    tau: i32,
}

#[allow(clippy::too_many_arguments)]
const fn t(f0: f32, f1: f32, start: i32, dur: i32, amp: f32, h2: f32, h3: f32, att: i32, tau: i32) -> Tone {
    Tone { f0, f1, start, dur, amp, h2, h3, att, tau }
}

const W1: &[Tone] = &[t(1046.5, 1046.5, 0, 110, 1.0, 0.12, 0.0, 6, 70), t(1318.5, 1318.5, 70, 170, 1.0, 0.12, 0.0, 6, 80)];
const W2: &[Tone] = &[t(784.0, 784.0, 0, 230, 1.0, 0.25, 0.08, 4, 90)];
const W3: &[Tone] = &[t(620.0, 980.0, 0, 130, 1.0, 0.08, 0.0, 6, 70)];
const W4: &[Tone] = &[t(1318.5, 1318.5, 0, 70, 0.8, 0.1, 0.0, 4, 40), t(1568.0, 1568.0, 55, 70, 0.8, 0.1, 0.0, 4, 40), t(2093.0, 2093.0, 110, 130, 0.8, 0.1, 0.0, 4, 60)];
const F1: &[Tone] = &[t(440.0, 440.0, 0, 110, 1.0, 0.10, 0.0, 8, 80), t(349.2, 349.2, 95, 150, 1.0, 0.10, 0.0, 8, 90)];
const F2: &[Tone] = &[t(240.0, 170.0, 0, 170, 1.0, 0.15, 0.05, 5, 60)];
const F3: &[Tone] = &[t(330.0, 330.0, 0, 220, 1.0, 0.06, 0.10, 15, 0)];
const C1: &[Tone] = &[t(698.5, 698.5, 0, 110, 1.0, 0.08, 0.0, 6, 60)];
const C2: &[Tone] = &[t(1200.0, 1200.0, 0, 50, 1.0, 0.0, 0.0, 2, 15)];
const C3: &[Tone] = &[t(587.3, 587.3, 0, 80, 1.0, 0.08, 0.0, 6, 50), t(698.5, 698.5, 70, 120, 1.0, 0.08, 0.0, 6, 60)];
const C4: &[Tone] = &[t(500.0, 700.0, 0, 160, 1.0, 0.06, 0.0, 10, 90)];
const A1: &[Tone] = &[t(659.3, 659.3, 0, 80, 1.0, 0.10, 0.0, 6, 50), t(659.3, 659.3, 130, 100, 1.0, 0.10, 0.0, 6, 60)];
const A2: &[Tone] = &[t(523.3, 523.3, 0, 60, 1.0, 0.15, 0.0, 3, 25), t(523.3, 523.3, 110, 70, 1.0, 0.15, 0.0, 3, 25)];
const A3: &[Tone] = &[t(523.3, 784.0, 0, 220, 1.0, 0.08, 0.0, 20, 0)];

const OPTS: [&[(&str, &[Tone])]; 4] = [
    &[("Chime", W1), ("Bell", W2), ("Pop", W3), ("Sparkle", W4)],
    &[("Low tone", F1), ("Thud", F2), ("Hum", F3)],
    &[("Blip", C1), ("Tick", C2), ("Two tone", C3), ("Glide", C4)],
    &[("Double", A1), ("Knock", A2), ("Rise", A3)],
];
const FILES: [&str; 4] = ["saved.wav", "failed.wav", "changed.wav", "warning.wav"];

/// How many built-in choices an event has.
pub fn opt_count(ev: Sound) -> usize {
    OPTS[ev as usize].len()
}
/// A built-in choice's name, i = 1..=count.
pub fn opt_name(ev: Sound, i: i32) -> &'static str {
    OPTS[ev as usize].get((i - 1).max(0) as usize).map(|o| o.0).unwrap_or("")
}

/// ClipPing's own sine (range reduction + a Taylor series), so the samples match its bytes.
fn fsin(mut x: f32) -> f32 {
    let two: f32 = std::f32::consts::TAU; // ClipPing: 6.2831853f (the same f32)
    x -= two * ((x / two) as i32) as f32;
    if x > PI {
        x -= two;
    } else if x < -PI {
        x += two;
    }
    if x > PI / 2.0 {
        x = PI - x;
    } else if x < -PI / 2.0 {
        x = -PI - x;
    }
    let x2 = x * x;
    x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0 * (1.0 - x2 / 72.0))))
}

/// Float samples of one choice, peak normalised to 1.0.
fn gen(ev: Sound, opt: i32) -> Vec<f32> {
    let tones = OPTS[ev as usize][(opt - 1) as usize].1;
    let mut total = 0;
    for t in tones {
        total = total.max(t.start + t.dur);
    }
    let total = (total * RATE / 1000) as usize;
    let mut mix = vec![0f32; total];
    for t in tones {
        let s0 = (t.start * RATE / 1000) as usize;
        let n = t.dur * RATE / 1000;
        let mut att = t.att * RATE / 1000;
        let rel = if n / 3 < RATE * 25 / 1000 { n / 3 } else { RATE * 25 / 1000 };
        let (mut ph, mut env) = (0f32, 1f32);
        let dec = if t.tau != 0 { 1.0f32 - 1.0f32 / ((t.tau * RATE) as f32 / 1000.0f32) } else { 1.0 };
        if att < 1 {
            att = 1;
        }
        let mut i = 0;
        while i < n && s0 + (i as usize) < total {
            let f = t.f0 + (t.f1 - t.f0) * i as f32 / n as f32;
            let mut g = env;
            if i < att {
                g *= 0.5f32 - 0.5f32 * fsin(PI * i as f32 / att as f32 + PI / 2.0);
            }
            if n - i < rel {
                g *= 0.5f32 - 0.5f32 * fsin(PI * (n - i) as f32 / rel as f32 + PI / 2.0);
            }
            let v = fsin(ph) + t.h2 * fsin(2.0 * ph) + t.h3 * fsin(3.0 * ph);
            mix[s0 + i as usize] += t.amp * g * v;
            ph += 2.0 * PI * f / RATE as f32;
            if ph > 2.0 * PI {
                ph -= 2.0 * PI;
            }
            if i >= att {
                env *= dec;
            }
            i += 1;
        }
    }
    let peak = mix.iter().fold(0f32, |p, v| p.max(v.abs()));
    if peak > 0.0 {
        for v in &mut mix {
            *v /= peak;
        }
    }
    mix
}

fn wav_from_float(s: &[f32], gain: f32) -> Vec<u8> {
    let n = s.len() as u32;
    let mut w = Vec::with_capacity(44 + s.len() * 2);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + n * 2).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&(RATE as u32).to_le_bytes());
    w.extend_from_slice(&(RATE as u32 * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(n * 2).to_le_bytes());
    for v in s {
        w.extend_from_slice(&((v * gain * 32767.0f32) as i16).to_le_bytes());
    }
    w
}

fn get16(p: &[u8]) -> u32 {
    p[0] as u32 | (p[1] as u32) << 8
}
fn get32(p: &[u8]) -> u32 {
    p[0] as u32 | (p[1] as u32) << 8 | (p[2] as u32) << 16 | (p[3] as u32) << 24
}

/// Scale a .wav file's samples in place (PCM 8 / 16 / 24 / 32-bit or 32-bit float). False = a format it can't scale.
pub fn wav_scale(w: &mut [u8], gain: f32) -> bool {
    let len = w.len();
    if len < 12 || &w[0..4] != b"RIFF" || &w[8..12] != b"WAVE" {
        return false;
    }
    let (mut p, mut tag, mut bits) = (12usize, 0u32, 0u32);
    let mut data: Option<(usize, usize)> = None;
    while p + 8 <= len {
        let cl = get32(&w[p + 4..]) as usize;
        if &w[p..p + 4] == b"fmt " && cl >= 16 && p + 8 + 16 <= len {
            tag = get16(&w[p + 8..]);
            bits = get16(&w[p + 22..]);
            if tag == 0xFFFE && cl >= 26 && p + 8 + 26 <= len {
                tag = get16(&w[p + 32..]);
            }
        } else if &w[p..p + 4] == b"data" {
            let dl = cl.min(len - p - 8);
            data = Some((p + 8, dl));
            break;
        }
        p += 8 + cl + (cl & 1);
    }
    let Some((d0, dl)) = data else { return false };
    let d = &mut w[d0..d0 + dl];
    match (tag, bits) {
        (1, 16) => {
            for c in d.as_chunks_mut::<2>().0.iter_mut() {
                let s = i16::from_le_bytes([c[0], c[1]]);
                c.copy_from_slice(&((s as f32 * gain) as i16).to_le_bytes());
            }
        }
        (1, 8) => {
            for b in d.iter_mut() {
                *b = (128.0 + (*b as i32 - 128) as f32 * gain) as i32 as u8;
            }
        }
        (1, 24) => {
            for c in d.as_chunks_mut::<3>().0.iter_mut() {
                let v = ((c[0] as u32) << 8 | (c[1] as u32) << 16 | (c[2] as u32) << 24) as i32 >> 8;
                let v = (v as f32 * gain) as i32;
                c[0] = v as u8;
                c[1] = (v >> 8) as u8;
                c[2] = (v >> 16) as u8;
            }
        }
        (1, 32) => {
            for c in d.as_chunks_mut::<4>().0.iter_mut() {
                let s = i32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                c.copy_from_slice(&((s as f64 * gain as f64) as i32).to_le_bytes());
            }
        }
        (3, 32) => {
            for c in d.as_chunks_mut::<4>().0.iter_mut() {
                let s = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                c.copy_from_slice(&(s * gain).to_le_bytes());
            }
        }
        _ => return false,
    }
    true
}

/// Where a sound came from (tests / the test hook).
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    BuiltIn(&'static str),
    File(PathBuf),
}

/// The exact bytes that play for an event with these settings (None = silent). `exe_dir` = where saved.wav etc. may be
/// dropped to replace the built-in sound (ClipPing: next to its exe; here: next to the app's exe).
pub fn build(set: &Settings, ev: Sound, exe_dir: Option<&Path>) -> Option<(Vec<u8>, Source)> {
    let mut kind = set.snd[ev as usize];
    if kind == 0 || set.vol <= 0 {
        return None;
    }
    let vol = set.vol as f32 / 100.0f32;
    let custom = &set.sndfile[ev as usize];
    let dropped = exe_dir.map(|d| d.join(FILES[ev as usize]));
    let file = if kind == SND_CUSTOM && !custom.is_empty() && Path::new(custom).is_file() {
        Some(PathBuf::from(custom))
    } else if kind != SND_CUSTOM && dropped.as_ref().is_some_and(|p| p.is_file()) {
        dropped
    } else {
        None
    };
    if let Some(f) = file {
        if let Ok(mut w) = std::fs::read(&f) {
            let _ = wav_scale(&mut w, vol); // a format it can't scale plays as it is
            return Some((w, Source::File(f)));
        }
    }
    if kind == SND_CUSTOM || kind < 1 || kind as usize > opt_count(ev) {
        kind = 1; // custom file missing: the first built-in
    }
    let s = gen(ev, kind);
    Some((wav_from_float(&s, PEAK * vol), Source::BuiltIn(opt_name(ev, kind))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_sounds_have_the_right_length_and_volume() {
        let s = Settings::default();
        let (w, src) = build(&s, Sound::Saved, None).unwrap();
        assert_eq!(src, Source::BuiltIn("Pop"));
        assert_eq!(w.len(), 44 + 2 * (130 * 22050 / 1000) as usize);
        let peak = w[44..].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c).unsigned_abs()).max().unwrap();
        // 1.0 x 0.1259 x 0.30 x 32767 = 1237.6 -> 1237
        assert_eq!(peak, 1237);
        assert!(build(&Settings { vol: 0, ..s.clone() }, Sound::Saved, None).is_none());
        assert!(build(&Settings { snd: [0, 2, 3, 2], ..s.clone() }, Sound::Saved, None).is_none());
        assert_eq!(opt_count(Sound::Failed), 3);
        assert_eq!(opt_name(Sound::Changed, 3), "Two tone");
    }

    #[test]
    fn fsin_is_close_to_sin() {
        for i in -100..100 {
            let x = i as f32 * 0.37;
            assert!((fsin(x) - x.sin()).abs() < 1e-3, "{x}");
        }
    }

    #[test]
    fn scaling_a_16_bit_file() {
        let mut w = wav_from_float(&[1.0, -0.5, 0.25], 1.0);
        assert!(wav_scale(&mut w, 0.5));
        let s: Vec<i16> = w[44..].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect();
        assert_eq!(s, vec![16383, -8191, 4095]);
    }
}
