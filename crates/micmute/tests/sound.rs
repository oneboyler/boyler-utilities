//! The mute sounds measured from their WAV bytes (never played) — plus written to / read back from the lane's scratch folder.

use bu_micmute::sound::{self, Sound, SoundSettings, PEAK_AT_FULL, SAMPLE_RATE};
use std::path::PathBuf;

// ---------- sounds (measured from the bytes, never played) ----------

struct Wav {
    rate: u32,
    channels: u16,
    bits: u16,
    samples: Vec<i16>,
}

fn parse(w: &[u8]) -> Wav {
    assert_eq!(&w[0..4], b"RIFF");
    assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize, w.len() - 8);
    assert_eq!(&w[8..16], b"WAVEfmt ");
    assert_eq!(u16::from_le_bytes([w[20], w[21]]), 1, "PCM");
    let channels = u16::from_le_bytes([w[22], w[23]]);
    let rate = u32::from_le_bytes(w[24..28].try_into().unwrap());
    let bits = u16::from_le_bytes([w[34], w[35]]);
    assert_eq!(&w[36..40], b"data");
    let len = u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize;
    assert_eq!(len, w.len() - 44);
    let samples = w[44..].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect();
    Wav { rate, channels, bits, samples }
}

fn peak(s: &[i16]) -> f32 {
    s.iter().map(|v| (*v as f32).abs()).fold(0.0, f32::max) / 32767.0
}

/// Sign changes per sample in a slice — rises with pitch.
fn crossings(s: &[i16]) -> usize {
    s.windows(2).filter(|w| (w[0] >= 0) != (w[1] >= 0)).count()
}

#[test]
fn defaults_match_the_design() {
    let s = SoundSettings::default();
    assert_eq!(s, SoundSettings { enabled: false, on_mute: Sound::BlipDown, on_unmute: Sound::BlipUp, volume: 5 });
    assert_eq!(s.sound_for(true), None, "off by default");
    let s = SoundSettings { enabled: true, ..s };
    assert_eq!(Sound::ALL.map(|s| s.label()), ["Soft click", "Blip down", "Blip up", "Chime", "None"]);
    assert_eq!(s.sound_for(true), Some(Sound::BlipDown));
    assert_eq!(s.sound_for(false), Some(Sound::BlipUp));
}

#[test]
fn every_sound_is_a_valid_short_wav_and_none_is_nothing() {
    assert_eq!(sound::wav(Sound::None, 60), None);
    for (s, ms) in [(Sound::SoftClick, 18), (Sound::BlipDown, 110), (Sound::BlipUp, 110), (Sound::Chime, 420)] {
        let w = parse(&sound::wav(s, 60).unwrap());
        assert_eq!((w.rate, w.channels, w.bits), (SAMPLE_RATE, 1, 16));
        assert_eq!(w.samples.len(), (SAMPLE_RATE as usize) * ms / 1000, "{s:?} length");
        assert!(peak(&w.samples) > 0.05, "{s:?} is audible");
        assert_eq!(sound::wav(s, 60), sound::wav(s, 60), "deterministic");
    }
}

#[test]
fn volume_follows_a_decibel_curve() {
    for s in [Sound::SoftClick, Sound::BlipDown, Sound::BlipUp, Sound::Chime] {
        let p100 = peak(&parse(&sound::wav(s, 100).unwrap()).samples);
        let p60 = peak(&parse(&sound::wav(s, 60).unwrap()).samples);
        let p0 = peak(&parse(&sound::wav(s, 0).unwrap()).samples);
        assert!(p100 <= PEAK_AT_FULL + 0.001, "{s:?} headroom: {p100}");
        let want = 10f32.powf(-sound::RANGE_DB * 0.4 / 20.0);
        assert!((p60 / p100 - want).abs() < 0.01, "{s:?} 60 % → {} (want {want})", p60 / p100);
        let p5 = peak(&parse(&sound::wav(s, 5).unwrap()).samples);
        assert!(p5 > 0.0 && p5 / p100 < 0.03, "{s:?} 5 % is far under 100 %: {}", p5 / p100);
        assert!(p5 < 0.01, "{s:?} 5 % peaks under 1 % of full scale: {p5}");
        assert_eq!(p0, 0.0, "{s:?} volume 0 is silence");
        assert_eq!(sound::wav(s, 200), sound::wav(s, 100), "above 100 counts as 100");
    }
}

#[test]
fn blip_down_falls_and_blip_up_rises() {
    let down = parse(&sound::wav(Sound::BlipDown, 100).unwrap()).samples;
    let up = parse(&sound::wav(Sound::BlipUp, 100).unwrap()).samples;
    let h = down.len() / 2;
    assert!(crossings(&down[..h]) > crossings(&down[h..]), "Blip down: pitch falls");
    assert!(crossings(&up[..h]) < crossings(&up[h..]), "Blip up: pitch rises");
}

#[test]
fn wav_from_pcm_header() {
    let w = sound::wav_from_pcm(&[0, 1000, -1000, i16::MAX, i16::MIN], 8000);
    let p = parse(&w);
    assert_eq!(p.rate, 8000);
    assert_eq!(p.samples, vec![0, 1000, -1000, i16::MAX, i16::MIN]);
}

/// The lane's own scratch folder (`BoylerUtilities-board\scratch\lane-i\`), only when it exists — never anywhere else.
fn scratch() -> Option<PathBuf> {
    let desktop = PathBuf::from(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3)?.to_path_buf();
    let lane = desktop.join("BoylerUtilities-board").join("scratch").join("lane-i");
    lane.is_dir().then_some(lane)
}

#[test]
fn sound_files_round_trip_through_the_scratch_folder() {
    let Some(lane) = scratch() else {
        eprintln!("skipped: scratch folder BoylerUtilities-board\\scratch\\lane-i missing");
        return;
    };
    let dir = lane.join(format!("micmute-sounds-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for s in [Sound::SoftClick, Sound::BlipDown, Sound::BlipUp, Sound::Chime] {
        let bytes = sound::wav(s, 60).unwrap();
        let path = dir.join(format!("{}.wav", s.label().replace(' ', "-")));
        std::fs::write(&path, &bytes).unwrap();
        let back = std::fs::read(&path).unwrap();
        assert_eq!(back, bytes);
        let full = peak(&parse(&sound::wav(s, 100).unwrap()).samples);
        assert!((peak(&parse(&back).samples) / full - sound::gain(60) / sound::gain(100)).abs() < 0.01);
    }
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(!dir.exists());
}
