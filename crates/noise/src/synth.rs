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

/// The loop of `kind` for `rate`, full length, a new random one each time.
pub fn make_loop(kind: Kind, rate: u32) -> Loop {
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    make_loop_n(kind, rate, loop_frames(rate), seed)
}

/// The loop with `frames` (a power of two) samples and a given seed (tests use short ones).
pub fn make_loop_n(kind: Kind, rate: u32, frames: usize, seed: u64) -> Loop {
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
        let a = amplitude(kind, f);
        let p = rng.phase();
        let (s, c) = p.sin_cos();
        re[k] = a * c;
        im[k] = a * s;
    }
    fft(&mut re, &mut im, true);
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
}
