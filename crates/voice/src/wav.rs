//! Tiny 16-bit PCM WAV reader / writer + the voice level of each slice (the fake engine's input in tests; never recorded).

use crate::error::{Result, VoiceError};

/// Mono 16-bit samples and their rate.
#[derive(Debug, Clone, PartialEq)]
pub struct Pcm {
    pub rate: u32,
    pub samples: Vec<i16>,
}

/// A 16-bit PCM WAV file's bytes (mono).
pub fn encode(p: &Pcm) -> Vec<u8> {
    let data_len = (p.samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&p.rate.to_le_bytes());
    b.extend_from_slice(&(p.rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in &p.samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

/// Read a 16-bit PCM WAV (mono, or the first channel of several).
pub fn decode(bytes: &[u8]) -> Result<Pcm> {
    let bad = |w: &str| VoiceError::File(format!("not a 16-bit PCM WAV: {w}"));
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad("no RIFF/WAVE header"));
    }
    let (mut pos, mut rate, mut chans, mut bits) = (12usize, 0u32, 0u16, 0u16);
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = pos + 8;
        let end = body.checked_add(len).filter(|e| *e <= bytes.len()).ok_or_else(|| bad("chunk past the end"))?;
        if id == b"fmt " {
            if len < 16 || u16::from_le_bytes([bytes[body], bytes[body + 1]]) != 1 {
                return Err(bad("format is not PCM"));
            }
            chans = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]);
            rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().unwrap());
            bits = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]);
        } else if id == b"data" {
            if bits != 16 || chans == 0 || rate == 0 {
                return Err(bad("not 16-bit"));
            }
            let frame = 2 * chans as usize;
            let samples = bytes[body..end].chunks_exact(frame).map(|f| i16::from_le_bytes([f[0], f[1]])).collect();
            return Ok(Pcm { rate, samples });
        }
        pos = end + (len & 1);
    }
    Err(bad("no data chunk"))
}

/// The voice level of each `slice_ms` slice: RMS scaled so normal speech (~-20 dBFS) is ~0.6, clamped 0..1.
pub fn levels(p: &Pcm, slice_ms: u32) -> Vec<f32> {
    let n = ((p.rate as u64 * slice_ms as u64) / 1000).max(1) as usize;
    p.samples
        .chunks(n)
        .map(|c| {
            let sum: f64 = c.iter().map(|s| (*s as f64 / 32768.0).powi(2)).sum();
            let rms = (sum / c.len() as f64).sqrt() as f32;
            (rms * 6.0).clamp(0.0, 1.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_wav_reads_back_and_its_levels_follow_the_loudness() {
        let rate = 16000;
        let mut samples = vec![0i16; rate as usize / 10];
        samples.extend((0..rate / 10).map(|i| ((i as f32 * 0.3).sin() * 3000.0) as i16));
        let p = Pcm { rate, samples };
        let back = decode(&encode(&p)).unwrap();
        assert_eq!(back, p);
        let l = levels(&back, 100);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0], 0.0);
        assert!(l[1] > 0.3 && l[1] < 0.5, "{l:?}");
        assert!(decode(b"RIFF....WAVEjunk").is_err());
    }
}
