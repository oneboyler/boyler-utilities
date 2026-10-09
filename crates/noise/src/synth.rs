//! Making a noise: its LOOP, once, in memory (Order 062). No recordings.
//!
//! The loop is made in the frequency domain: every frequency gets the level the noise's spectrum asks for and a random phase,
//! and one inverse FFT turns that into the sound. What comes out repeats EXACTLY every `frames` samples (an inverse FFT's
//! output is periodic by construction), so the loop point is not "crossfaded to be inaudible" - there is no seam at all: the
//! sample after the last one is the first one, of the same continuous sound. The spectrum is exact at any sample rate (no
//! filter that was tuned for one rate), and the loop needs no warm-up. The real and the imaginary part of the one complex
//! signal are two independent noises: the left and the right channel (a wide, uncorrelated sound).
//!
//! A loop is 2^20 frames at up to 52 kHz (21.8 s at 48 kHz), 2^21 up to 104 kHz, and so on: at least 20 s. It is kept as 16-bit
//! (4.2 MB at 48 kHz) and made once per pick, on the worker thread.

use crate::fft::fft;
use crate::kind::Kind;
use crate::sound::{Mix, Sound};

/// A loop is at least this long (seconds); its length is the next power of two of samples.
pub const MIN_LOOP_SECS: u32 = 20;
/// Every loop is scaled to this RMS (full scale = 1.0), so the six noises come out about equally loud...
const TARGET_RMS: f32 = 0.2;
/// ...unless its loudest sample would pass this (brown has big swings): then it is scaled down to fit.
const PEAK_MAX: f32 = 0.95;
/// Everything below this (Hz) is rolled off by a 2nd-order high-pass: nobody hears it, and it only eats headroom.
const LOW_CUT_HZ: f32 = 20.0;

/// A finished loop: interleaved left / right, 16-bit.
pub struct Loop {
    pub rate: u32,
    pub frames: usize,
    pub data: Vec<i16>,
}

/// The loop length for a sample rate: the next power of two of `MIN_LOOP_SECS` of samples.
pub fn loop_frames(rate: u32) -> usize {
    (rate as usize * MIN_LOOP_SECS as usize).next_power_of_two()
}

/// ISO 226:2003, 40 phon: the sound level (dB SPL) at each frequency that sounds as loud as 40 dB at 1 kHz. Written from my
/// memory of the standard's table (the standard's text isn't available here): unchecked against it, so treat as +-1 dB.
const ISO_40_PHON: [(f32, f32); 29] = [
    (20.0, 99.85),
    (25.0, 93.94),
    (31.5, 88.17),
    (40.0, 82.63),
    (50.0, 77.78),
    (63.0, 73.08),
    (80.0, 68.48),
    (100.0, 64.37),
    (125.0, 60.59),
    (160.0, 56.70),
    (200.0, 53.41),
    (250.0, 50.40),
    (315.0, 47.58),
    (400.0, 44.98),
    (500.0, 43.05),
    (630.0, 41.34),
    (800.0, 40.06),
    (1000.0, 40.01),
    (1250.0, 41.82),
    (1600.0, 42.51),
    (2000.0, 39.23),
    (2500.0, 36.51),
    (3150.0, 35.61),
    (4000.0, 36.65),
    (5000.0, 40.01),
    (6300.0, 45.83),
    (8000.0, 51.80),
    (10000.0, 54.28),
    (12500.0, 51.49),
];
/// Grey boosts the lows by at most this (dB over 1 kHz): the contour's +50 dB at 20 Hz would make it all rumble.
const GREY_MAX_BOOST_DB: f32 = 24.0;

/// Grey's level in dB relative to 1 kHz at `f` Hz (linear in log-frequency between the table's points, flat above 12.5 kHz).
fn grey_db(f: f32) -> f32 {
    let t = &ISO_40_PHON;
    let at_1k = 40.01;
    let spl = if f <= t[0].0 {
        t[0].1
    } else if f >= t[t.len() - 1].0 {
        t[t.len() - 1].1
    } else {
        let i = t.iter().position(|p| p.0 > f).unwrap_or(t.len() - 1);
        let ((f0, v0), (f1, v1)) = (t[i - 1], t[i]);
        let x = (f.ln() - f0.ln()) / (f1.ln() - f0.ln());
        v0 + (v1 - v0) * x
    };
    (spl - at_1k).min(GREY_MAX_BOOST_DB)
}

/// The AMPLITUDE a noise has at `f` Hz (its spectrum's square root; the overall level doesn't matter - it is scaled after).
pub fn amplitude(kind: Kind, f: f32) -> f32 {
    if f <= 0.0 {
        return 0.0;
    }
    let low_cut = 1.0 / (1.0 + (LOW_CUT_HZ / f).powi(4)).sqrt();
    let shape = match kind {
        Kind::White => 1.0,
        Kind::Pink => 1.0 / f.sqrt(),
        Kind::Brown => 1.0 / (1.0 + (f / 40.0).powi(2)).sqrt(),
        Kind::DarkBrown => 1.0 / ((1.0 + (f / 40.0).powi(2)) * (1.0 + (f / 200.0).powi(2))).sqrt(),
        Kind::Grey => 10f32.powf(grey_db(f) / 20.0),
        Kind::Blue => f.sqrt(),
    };
    shape * low_cut
}


/// Where Rumble's low shelf turns (Hz) and how much it adds at Rumble 100 (dB).
const RUMBLE_CORNER_HZ: f32 = 120.0;
const RUMBLE_MAX_DB: f32 = 15.0;
/// Where Tone's slope begins (Hz): the same corner brown has, so Tone 0 IS brown.
const TONE_CORNER_HZ: f32 = 40.0;

/// The AMPLITUDE of your own mix at `f` Hz: Tone tilts the spectrum from brown (-6 dB per octave, Tone 0) through pink (-3,
/// Tone 50) to white (flat, Tone 100); Rumble lifts everything below ~120 Hz with a shelf of up to +15 dB.
pub fn mix_amplitude(m: Mix, f: f32) -> f32 {
    if f <= 0.0 {
        return 0.0;
    }
    let low_cut = 1.0 / (1.0 + (LOW_CUT_HZ / f).powi(4)).sqrt();
    let t = f32::from(m.tone.min(Mix::MAX)) / 100.0;
    // amplitude ~ f^-(1 - t) above the corner (power: -6 (1 - t) dB per octave), flat below it
    let tilt = (1.0 + (f / TONE_CORNER_HZ).powi(2)).powf(-(1.0 - t) / 2.0);
    let g = 10f32.powf(RUMBLE_MAX_DB * f32::from(m.rumble.min(Mix::MAX)) / 100.0 / 20.0);
    let shelf = (1.0 + (g * g - 1.0) / (1.0 + (f / RUMBLE_CORNER_HZ).powi(2))).sqrt();
    tilt * shelf * low_cut
}

/// The amplitude of any sound at `f` Hz.
pub fn sound_amplitude(s: Sound, f: f32) -> f32 {
    match s {
        Sound::Preset(k) => amplitude(k, f),
        Sound::Mix(m) => mix_amplitude(m, f),
    }
}

/// How deep the swell goes at Waves 100: the level between two waves is this far down (a fraction of the crest).
const SWELL_MAX_DEPTH: f32 = 0.97;
/// The swell is a few slow sines that all fit a whole number of times into the loop (so the loop still has no seam): 2, 3 and
/// 5 cycles in a ~22 s loop = a wave every 11 s, 7 s and 4.4 s, added to something irregular and sea-like. (cycles, weight, phase)
const SWELL_PARTS: [(f32, f32, f32); 3] = [(3.0, 0.5, 0.7), (2.0, 0.3, 2.1), (5.0, 0.2, 4.0)];
/// The right channel's swell is this far (a fraction of the loop) after the left one's: the wash moves across the stereo field.
const SWELL_RIGHT_SHIFT: f32 = 1.0 / 32.0;
/// The swell curve is computed at this many points per loop and joined in straight lines (it is slow: no need for every sample).
const SWELL_GRID: usize = 4096;

/// The swell at position `u` (0 .. 1 through the loop): 1 at a crest, `1 - depth` between two waves. Periodic in `u`.
fn swell(u: f32, depth: f32) -> f32 {
    let x: f32 = SWELL_PARTS.iter().map(|(c, w, p)| w * (std::f32::consts::TAU * c * u + p).sin()).sum();
    // x is in -1 .. 1: squared after mapping to 0 .. 1, so the crests are narrower than the troughs (waves, not a hum)
    let w = ((x + 1.0) * 0.5).powi(2);
    1.0 - depth + depth * w
}

/// Multiplies one channel by the swell (`shift` = how far this channel is behind, as a fraction of the loop).
fn apply_swell(x: &mut [f32], waves: u8, shift: f32) {
    let n = x.len();
    let depth = SWELL_MAX_DEPTH * f32::from(waves.min(Mix::MAX)) / 100.0;
    if depth <= 0.0 || n < 2 {
        return;
    }
    let grid = SWELL_GRID.min(n);
    let g: Vec<f32> = (0..=grid).map(|i| swell((i as f32 / grid as f32 + shift).fract(), depth)).collect();
    // the last point (i = grid) is the first one again: u = 1 is u = 0
    let per = n as f32 / grid as f32;
    for (i, v) in x.iter_mut().enumerate() {
        let p = i as f32 / per;
        let k = (p as usize).min(grid - 1);
        let a = p - k as f32;
        *v *= g[k] * (1.0 - a) + g[k + 1] * a;
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// 0 .. 2 pi
    fn phase(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32 * std::f32::consts::TAU
    }
}

/// The loop of `sound` (a [`Kind`], a [`Mix`] or a [`Sound`]) for `rate`, full length, a new random one each time.
pub fn make_loop(sound: impl Into<Sound>, rate: u32) -> Loop {
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    make_loop_n(sound, rate, loop_frames(rate), seed)
}

/// The loop with `frames` (a power of two) samples and a given seed (tests use short ones).
pub fn make_loop_n(sound: impl Into<Sound>, rate: u32, frames: usize, seed: u64) -> Loop {
    let sound = sound.into();
    assert!(frames.is_power_of_two() && frames >= 2);
    let mut rng = Rng(seed | 1);
    for _ in 0..4 {
        rng.next();
    }
    let n = frames;
    let mut re = vec![0.0f32; n];
    let mut im = vec![0.0f32; n];
    let bin_hz = rate as f32 / n as f32;
    for k in 1..n {
        // the mirror half has the same level: both channels get the same spectrum
        let f = bin_hz * k.min(n - k) as f32;
        let a = sound_amplitude(sound, f);
        let p = rng.phase();
        let (s, c) = p.sin_cos();
        re[k] = a * c;
        im[k] = a * s;
    }
    fft(&mut re, &mut im, true);
    if let Sound::Mix(m) = sound {
        // Waves: the level of each channel rises and falls (a whole number of cycles per loop: the seam stays invisible)
        apply_swell(&mut re, m.waves, 0.0);
        apply_swell(&mut im, m.waves, SWELL_RIGHT_SHIFT);
    }
    let (mut sum, mut peak) = (0.0f64, 0.0f32);
    for i in 0..n {
        sum += f64::from(re[i]) * f64::from(re[i]) + f64::from(im[i]) * f64::from(im[i]);
        peak = peak.max(re[i].abs()).max(im[i].abs());
    }
    let rms = (sum / (2.0 * n as f64)).sqrt() as f32;
    let scale = if rms > 0.0 && peak > 0.0 { (TARGET_RMS / rms).min(PEAK_MAX / peak) } else { 0.0 };
    let mut data = Vec::with_capacity(n * 2);
    for i in 0..n {
        data.push((re[i] * scale * 32767.0).round() as i16);
        data.push((im[i] * scale * 32767.0).round() as i16);
    }
    Loop { rate, frames: n, data }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;
    const N: usize = 1 << 16;

    /// Mean power (dB) per bin of the bins in [lo, hi] Hz of one channel of the loop.
    fn band_db(l: &Loop, ch: usize, lo: f32, hi: f32) -> f32 {
        let mut re: Vec<f32> = (0..l.frames).map(|i| f32::from(l.data[i * 2 + ch]) / 32768.0).collect();
        let mut im = vec![0.0f32; l.frames];
        fft(&mut re, &mut im, false);
        let bin = l.rate as f32 / l.frames as f32;
        let (a, b) = ((lo / bin).ceil() as usize, (hi / bin).floor() as usize);
        let p: f64 = (a..=b).map(|k| f64::from(re[k]).powi(2) + f64::from(im[k]).powi(2)).sum::<f64>() / (b - a + 1) as f64;
        10.0 * (p.max(1e-30)).log10() as f32
    }

    /// dB per octave between two bands' centres (power per bin, so a band's width doesn't matter).
    fn slope(l: &Loop, ch: usize, f0: f32, f1: f32) -> f32 {
        let w = 1.12;
        let d = band_db(l, ch, f1 / w, f1 * w) - band_db(l, ch, f0 / w, f0 * w);
        d / (f1 / f0).log2()
    }

    #[test]
    fn white_is_flat() {
        let l = make_loop_n(Kind::White, RATE, N, 1);
        let s = slope(&l, 0, 300.0, 12000.0);
        assert!(s.abs() < 0.3, "white: {s:.2} dB / octave (measured)");
    }

    #[test]
    fn pink_falls_3_db_per_octave() {
        let l = make_loop_n(Kind::Pink, RATE, N, 2);
        for ch in 0..2 {
            let s = slope(&l, ch, 200.0, 12000.0);
            assert!((s + 3.0).abs() < 0.3, "pink ch{ch}: {s:.2} dB / octave (measured)");
        }
    }

    #[test]
    fn brown_falls_6_db_per_octave_above_its_corner() {
        let l = make_loop_n(Kind::Brown, RATE, N, 3);
        let s = slope(&l, 0, 400.0, 12000.0);
        assert!((s + 6.0).abs() < 0.4, "brown: {s:.2} dB / octave (measured)");
    }

    #[test]
    fn dark_brown_falls_12_db_per_octave_above_its_corner() {
        let l = make_loop_n(Kind::DarkBrown, RATE, N, 4);
        let s = slope(&l, 0, 1500.0, 12000.0);
        assert!((s + 12.0).abs() < 0.6, "dark brown: {s:.2} dB / octave (measured)");
    }

    #[test]
    fn blue_rises_3_db_per_octave() {
        let l = make_loop_n(Kind::Blue, RATE, N, 5);
        let s = slope(&l, 0, 300.0, 12000.0);
        assert!((s - 3.0).abs() < 0.3, "blue: {s:.2} dB / octave (measured)");
    }

    #[test]
    fn grey_is_the_equal_loudness_shape() {
        let l = make_loop_n(Kind::Grey, RATE, N, 6);
        let at = |f: f32| band_db(&l, 0, f / 1.1, f * 1.1);
        // relative to 1 kHz: the lows are up (capped at +24 dB), the 3 kHz dip is below 1 kHz, the top is up again
        let d_100 = at(100.0) - at(1000.0);
        let d_3k = at(3150.0) - at(1000.0);
        let d_10k = at(10000.0) - at(1000.0);
        assert!((d_100 - 24.0).abs() < 1.5, "100 Hz {d_100:.1} dB (measured)");
        assert!((d_3k - (35.61 - 40.01)).abs() < 1.5, "3.15 kHz {d_3k:.1} dB (measured)");
        assert!((d_10k - (54.28 - 40.01)).abs() < 1.5, "10 kHz {d_10k:.1} dB (measured)");
    }

    #[test]
    fn the_channels_are_two_different_noises() {
        let l = make_loop_n(Kind::Pink, RATE, N, 7);
        let (mut cross, mut a, mut b) = (0.0f64, 0.0f64, 0.0f64);
        for i in 0..l.frames {
            let (x, y) = (f64::from(l.data[i * 2]), f64::from(l.data[i * 2 + 1]));
            cross += x * y;
            a += x * x;
            b += y * y;
        }
        let corr = cross / (a * b).sqrt();
        assert!(corr.abs() < 0.02, "left / right correlation {corr:.4} (measured)");
        assert!((a / b - 1.0).abs() < 0.05, "left and right are equally loud");
    }

    #[test]
    fn the_six_are_about_equally_loud_and_never_clip() {
        for (i, k) in Kind::ALL.into_iter().enumerate() {
            let l = make_loop_n(k, RATE, N, 10 + i as u64);
            let rms = (l.data.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / l.data.len() as f64).sqrt() / 32768.0;
            let peak = l.data.iter().map(|v| i32::from(*v).abs()).max().unwrap_or(0);
            assert!(peak <= 31_200, "{}: peak {peak}", k.name());
            assert!(rms > 0.12 && rms < 0.21, "{}: rms {rms:.3} (measured)", k.name());
        }
    }

    /// The loop point: the step from the last sample to the first is one more step of the same sound.
    #[test]
    fn no_click_at_the_loop_point() {
        for (i, k) in Kind::ALL.into_iter().enumerate() {
            let l = make_loop_n(k, RATE, N, 20 + i as u64);
            for ch in 0..2 {
                let x = |i: usize| f64::from(l.data[(i % l.frames) * 2 + ch]);
                // the step statistic of the whole loop
                let mut s2 = 0.0f64;
                let mut worst = 0.0f64;
                for i in 0..l.frames - 1 {
                    let d = x(i + 1) - x(i);
                    s2 += d * d;
                    worst = worst.max(d.abs());
                }
                let sigma = (s2 / (l.frames - 1) as f64).sqrt();
                let seam = (x(0) - x(l.frames - 1)).abs();
                // the biggest step of the whole loop bounds the seam's step; and it is a normal step of this sound
                assert!(seam <= worst, "{} ch{ch}: seam step {seam} > the loop's own biggest step {worst}", k.name());
                assert!(seam <= 6.0 * sigma, "{} ch{ch}: seam step {seam} = {:.1} sigma (measured)", k.name(), seam / sigma);
                // and the energy just before and just after the seam is the same as anywhere (no dip, no bump)
                let win = 480;
                let e = |from: usize| (0..win).map(|j| x(from + j).powi(2)).sum::<f64>() / win as f64;
                let (before, after, mid) = (e(l.frames - win), e(0), e(l.frames / 2));
                let mean = (before + after + mid) / 3.0;
                for (name, v) in [("before", before), ("after", after)] {
                    // 10 ms of noise: its energy varies by tens of percent on its own (brown especially)
                    assert!(v > mean / 6.0 && v < mean * 6.0, "{} ch{ch}: energy {name} the seam {v:.0} vs {mean:.0}", k.name());
                }
            }
        }
    }

    #[test]
    fn the_loop_is_made_for_the_rate_and_is_long_enough() {
        assert_eq!(loop_frames(48_000), 1 << 20);
        assert_eq!(loop_frames(44_100), 1 << 20);
        assert_eq!(loop_frames(96_000), 1 << 21);
        for r in [22_050u32, 44_100, 48_000, 96_000, 192_000] {
            assert!(loop_frames(r) as f64 / f64::from(r) >= 20.0);
        }
    }

    #[test]
    fn a_full_size_loop_is_made() {
        let t = std::time::Instant::now();
        let l = make_loop(Kind::Brown, 48_000);
        assert_eq!(l.frames, 1 << 20);
        assert_eq!(l.data.len(), 2 << 20);
        eprintln!("full 48 kHz brown loop made in {:?} (this build)", t.elapsed());
    }

    // ------------------------------------------------------------------ Order 080: your own mix

    fn mix(tone: u8, rumble: u8, waves: u8) -> Loop {
        make_loop_n(Mix::new(tone, rumble, waves), RATE, N, 100 + u64::from(tone) * 7 + u64::from(rumble) * 3 + u64::from(waves))
    }

    #[test]
    fn tone_0_is_brown_50_is_pink_100_is_white() {
        for (tone, want) in [(0u8, -6.0f32), (50, -3.0), (100, 0.0)] {
            let l = mix(tone, 0, 0);
            for ch in 0..2 {
                let s = slope(&l, ch, 400.0, 12000.0);
                assert!((s - want).abs() < 0.45, "tone {tone} ch{ch}: {s:.2} dB / octave, wanted {want} (measured)");
            }
        }
        // in between it is in between (the slider moves the tilt smoothly)
        let s25 = slope(&mix(25, 0, 0), 0, 400.0, 12000.0);
        let s75 = slope(&mix(75, 0, 0), 0, 400.0, 12000.0);
        assert!((s25 + 4.5).abs() < 0.5 && (s75 + 1.5).abs() < 0.5, "tone 25 {s25:.2}, tone 75 {s75:.2} dB / octave (measured)");
    }

    #[test]
    fn tone_0_with_no_rumble_has_brown_s_spectrum_exactly() {
        for f in [25.0f32, 60.0, 200.0, 1000.0, 9000.0] {
            let (a, b) = (mix_amplitude(Mix::new(0, 0, 0), f), amplitude(Kind::Brown, f));
            assert!((a - b).abs() < 1e-6 * b.max(1.0), "{f} Hz: {a} vs brown {b}");
        }
    }

    #[test]
    fn rumble_lifts_the_lows_and_leaves_the_top_alone() {
        let at = |l: &Loop, f: f32| band_db(l, 0, f / 1.1, f * 1.1);
        let (none, half, full) = (mix(100, 0, 0), mix(100, 50, 0), mix(100, 100, 0));
        // relative to 4 kHz (above the shelf) so the overall level (re-scaled every time) doesn't matter
        let rel = |l: &Loop, f: f32| at(l, f) - at(l, 4000.0);
        assert!(rel(&none, 60.0).abs() < 1.5, "no rumble: 60 Hz is {:.1} dB vs 4 kHz (measured)", rel(&none, 60.0));
        let (r_half, r_full) = (rel(&half, 60.0), rel(&full, 60.0));
        assert!(r_half > 4.0 && r_half < r_full, "rumble 50: {r_half:.1} dB, rumble 100: {r_full:.1} dB at 60 Hz (measured)");
        // the model says: g = +15 dB, shelf corner 120 Hz -> at 60 Hz 10 log10(1 + (g^2 - 1) / 1.25) = 14 dB
        assert!((r_full - 14.0).abs() < 1.5, "rumble 100 at 60 Hz: {r_full:.1} dB, expected about 14 (measured)");
        assert!(rel(&full, 1000.0).abs() < 1.0 && rel(&full, 10000.0).abs() < 1.0, "the mids and the top don't move");
    }

    /// Per-window RMS of one channel (linear), windows of `win` samples.
    fn windows_rms(l: &Loop, ch: usize, win: usize) -> Vec<f64> {
        (0..l.frames / win)
            .map(|w| {
                let s: f64 = (0..win).map(|j| f64::from(l.data[((w * win + j) * 2) + ch]).powi(2)).sum();
                (s / win as f64).sqrt()
            })
            .collect()
    }

    #[test]
    fn waves_make_the_level_rise_and_fall_in_whole_cycles_per_loop() {
        let range_db = |l: &Loop| {
            let w = windows_rms(l, 0, 960);
            let (mx, mn) = (w.iter().copied().fold(0.0f64, f64::max), w.iter().copied().fold(f64::MAX, f64::min));
            20.0 * (mx / mn).log10()
        };
        let (steady, some, full) = (range_db(&mix(100, 0, 0)), range_db(&mix(100, 0, 50)), range_db(&mix(100, 0, 100)));
        assert!(steady < 2.0, "waves 0: the level moves {steady:.2} dB (measured)");
        assert!(some > 4.0 && some < full, "waves 50: {some:.1} dB, waves 100: {full:.1} dB (measured)");
        assert!(full > 20.0, "waves 100: between two waves it is {full:.1} dB down (measured)");
        // the measured level follows the swell curve (the one that is built in, left; right = a little later)
        for (ch, shift) in [(0usize, 0.0f32), (1, SWELL_RIGHT_SHIFT)] {
            let l = mix(100, 0, 100);
            let w = windows_rms(&l, ch, 960);
            let want: Vec<f64> = (0..w.len())
                .map(|i| f64::from(swell(((i as f32 + 0.5) * 960.0 / l.frames as f32 + shift).fract(), SWELL_MAX_DEPTH)))
                .collect();
            let (mw, mt) = (w.iter().sum::<f64>() / w.len() as f64, want.iter().sum::<f64>() / want.len() as f64);
            let (mut c, mut a, mut b) = (0.0, 0.0, 0.0);
            for i in 0..w.len() {
                c += (w[i] - mw) * (want[i] - mt);
                a += (w[i] - mw).powi(2);
                b += (want[i] - mt).powi(2);
            }
            let corr = c / (a * b).sqrt();
            assert!(corr > 0.98, "ch{ch}: the measured level follows the swell: correlation {corr:.3} (measured)");
        }
    }

    #[test]
    fn the_waves_are_slow_and_sea_like_at_the_real_loop_length() {
        // the swell's parts fit a whole number of times into the loop: the slowest wave and the fastest, in seconds
        let secs = loop_frames(48_000) as f32 / 48_000.0;
        let (cmin, cmax) = SWELL_PARTS.iter().fold((f32::MAX, 0.0f32), |(a, b), p| (a.min(p.0), b.max(p.0)));
        assert!(SWELL_PARTS.iter().all(|p| p.0.fract() == 0.0), "whole cycles only, or the loop would have a seam");
        let (slow, fast) = (secs / cmin, secs / cmax);
        assert!((8.0..=12.0).contains(&slow) && (4.0..=5.0).contains(&fast), "waves every {fast:.1} .. {slow:.1} s (measured)");
        // the swell curve itself: continuous over the seam, 1 at a crest, 1 - depth at the lowest
        assert!((swell(0.0, 0.5) - swell(1.0, 0.5)).abs() < 1e-5);
        let (mn, mx) = (0..2000).map(|i| swell(i as f32 / 2000.0, 0.8)).fold((9.0f32, 0.0f32), |(a, b), v| (a.min(v), b.max(v)));
        assert!(mn >= 0.2 - 1e-4 && mx <= 1.0 + 1e-4 && mx - mn > 0.5, "swell {mn:.2} .. {mx:.2}");
    }

    #[test]
    fn a_mix_is_about_as_loud_as_the_noises_and_never_clips_at_any_corner() {
        for (i, (t, r, w)) in [(0u8, 0u8, 0u8), (100, 0, 0), (0, 100, 0), (100, 100, 0), (0, 0, 100), (100, 0, 100), (0, 100, 100), (100, 100, 100), (35, 30, 0), (50, 50, 50)].into_iter().enumerate() {
            let l = make_loop_n(Mix::new(t, r, w), RATE, N, 300 + i as u64);
            let rms = (l.data.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / l.data.len() as f64).sqrt() / 32768.0;
            let peak = l.data.iter().map(|v| i32::from(*v).abs()).max().unwrap_or(0);
            assert!(peak <= 31_200, "mix {t}/{r}/{w}: peak {peak}");
            eprintln!("mix tone {t} rumble {r} waves {w}: rms {rms:.3}, peak {peak} (measured)");
            assert!(rms > 0.10 && rms < 0.21, "mix {t}/{r}/{w}: rms {rms:.3} (measured)");
        }
    }

    #[test]
    fn no_click_at_the_loop_point_of_a_mix_with_waves() {
        for (i, m) in [Mix::new(100, 0, 100), Mix::DEFAULT, Mix::new(0, 100, 100), Mix::new(60, 60, 60)].into_iter().enumerate() {
            let l = make_loop_n(m, RATE, N, 400 + i as u64);
            for ch in 0..2 {
                let x = |i: usize| f64::from(l.data[(i % l.frames) * 2 + ch]);
                let (mut s2, mut worst) = (0.0f64, 0.0f64);
                for i in 0..l.frames - 1 {
                    let d = x(i + 1) - x(i);
                    s2 += d * d;
                    worst = worst.max(d.abs());
                }
                let sigma = (s2 / (l.frames - 1) as f64).sqrt();
                let seam = (x(0) - x(l.frames - 1)).abs();
                assert!(seam <= worst, "{m:?} ch{ch}: seam step {seam} > the loop's own biggest step {worst}");
                // the swell is slow: at the seam it is the same level on both sides (the step is a normal step of this sound)
                assert!(seam <= 8.0 * sigma, "{m:?} ch{ch}: seam step {:.1} sigma (measured)", seam / sigma);
            }
        }
    }

    #[test]
    fn a_mix_is_a_new_random_noise_each_time_but_the_same_sound() {
        let (a, b) = (make_loop_n(Mix::DEFAULT, RATE, N, 1), make_loop_n(Mix::DEFAULT, RATE, N, 2));
        assert_ne!(a.data, b.data);
        let d = (slope(&a, 0, 400.0, 12000.0) - slope(&b, 0, 400.0, 12000.0)).abs();
        assert!(d < 0.3, "the two have the same spectrum: slopes differ by {d:.2} dB / octave (measured)");
    }
}
