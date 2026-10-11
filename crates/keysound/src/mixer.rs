//! The mixer: a few voices that play pre-made sounds from memory (Order 058). No allocation on a press (a voice just takes
//! a shared pointer to the sound), no thread of its own — the one render thread calls [`Mixer::render`] for every buffer
//! the output asks for. A press picks a free voice (or takes over the oldest one); each voice plays its sound at a slightly
//! different speed (the random pitch), with a linear interpolation between samples.

use std::sync::Arc;

/// How many sounds may overlap (fast typing + the ring of a glass tap). The 9th takes over the oldest.
pub const VOICES: usize = 8;

#[derive(Default)]
struct Voice {
    data: Option<Arc<[f32]>>,
    /// Position in the sound, in samples (fractional).
    pos: f32,
    /// Samples advanced per output frame (1.0 = the original pitch).
    step: f32,
    gain: f32,
    /// How long ago it started (frames), to find the oldest.
    age: u64,
}

pub struct Mixer {
    voices: [Voice; VOICES],
    clock: u64,
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

impl Mixer {
    pub fn new() -> Mixer {
        Mixer { voices: Default::default(), clock: 0 }
    }

    /// Starts `sound` now at `gain` and playback speed `step` (1.0 = as made; 1.04 = 4 % higher and shorter).
    pub fn trigger(&mut self, sound: &Arc<[f32]>, gain: f32, step: f32) {
        self.clock += 1;
        let slot = self
            .voices
            .iter()
            .position(|v| v.data.is_none())
            .unwrap_or_else(|| self.voices.iter().enumerate().min_by_key(|(_, v)| v.age).map(|(i, _)| i).unwrap_or(0));
        self.voices[slot] = Voice { data: Some(sound.clone()), pos: 0.0, step: step.clamp(0.25, 4.0), gain, age: self.clock };
    }

    /// The speed of the sound started last (tests: the pitch a press got).
    pub fn last_step(&self) -> f32 {
        self.voices.iter().filter(|v| v.data.is_some()).max_by_key(|v| v.age).map(|v| v.step).unwrap_or(0.0)
    }
    pub fn active(&self) -> usize {
        self.voices.iter().filter(|v| v.data.is_some()).count()
    }

    pub fn stop_all(&mut self) {
        for v in self.voices.iter_mut() {
            v.data = None;
        }
    }

    /// Fills `out` (interleaved frames of `channels` samples; the mono sound goes to every channel): overwrites it.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        out.fill(0.0);
        let frames = out.len() / channels;
        for v in self.voices.iter_mut() {
            let Some(data) = v.data.as_ref() else { continue };
            let n = data.len();
            let mut done = false;
            for f in 0..frames {
                let i = v.pos as usize;
                if i + 1 >= n {
                    done = true;
                    break;
                }
                let frac = v.pos - i as f32;
                let s = (data[i] + (data[i + 1] - data[i]) * frac) * v.gain;
                for c in 0..channels {
                    out[f * channels + c] += s;
                }
                v.pos += v.step;
            }
            if done {
                v.data = None;
            }
        }
        // never beyond full scale, whatever overlaps
        for x in out.iter_mut() {
            *x = x.clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> Arc<[f32]> {
        Arc::from((0..n).map(|i| i as f32 / n as f32).collect::<Vec<_>>().into_boxed_slice())
    }

    #[test]
    fn a_sound_plays_once_then_the_voice_is_free() {
        let mut m = Mixer::new();
        let s: Arc<[f32]> = Arc::from(vec![0.5f32; 100].into_boxed_slice());
        m.trigger(&s, 1.0, 1.0);
        assert_eq!(m.active(), 1);
        let mut out = vec![9.0f32; 64 * 2];
        m.render(&mut out, 2);
        assert!(out.iter().all(|x| (*x - 0.5).abs() < 1e-6), "mono goes to both channels");
        m.render(&mut out, 2);
        assert_eq!(m.active(), 0, "100 samples played in 2 x 64 frames");
        assert!(out[..70].iter().all(|x| (*x - 0.5).abs() < 1e-6), "the last 35 frames of the sound");
        assert!(out[70..].iter().all(|x| *x == 0.0), "silence after the end");
    }

    #[test]
    fn nothing_playing_is_silence() {
        let mut m = Mixer::new();
        let mut out = vec![1.0f32; 480];
        m.render(&mut out, 2);
        assert!(out.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn a_faster_step_is_higher_and_shorter() {
        let s = ramp(1000);
        let mut a = Mixer::new();
        a.trigger(&s, 1.0, 1.0);
        let mut b = Mixer::new();
        b.trigger(&s, 1.0, 2.0);
        let (mut oa, mut ob) = (vec![0.0f32; 400], vec![0.0f32; 400]);
        a.render(&mut oa, 1);
        b.render(&mut ob, 1);
        assert!((oa[399] - 0.399).abs() < 1e-3);
        assert!((ob[399] - 0.798).abs() < 1e-3, "twice the speed: twice as far");
    }

    #[test]
    fn overlapping_sounds_add_and_the_ninth_takes_the_oldest() {
        let mut m = Mixer::new();
        let long: Arc<[f32]> = Arc::from(vec![0.1f32; 10_000].into_boxed_slice());
        for _ in 0..VOICES {
            m.trigger(&long, 1.0, 1.0);
        }
        assert_eq!(m.active(), VOICES);
        let mut out = vec![0.0f32; 8];
        m.render(&mut out, 1);
        assert!((out[0] - 0.8).abs() < 1e-5, "eight voices add up");
        m.trigger(&long, 1.0, 1.0);
        assert_eq!(m.active(), VOICES, "a ninth press takes a voice over: never a new one");
    }

    #[test]
    fn the_sum_never_passes_full_scale() {
        let mut m = Mixer::new();
        let loud: Arc<[f32]> = Arc::from(vec![0.9f32; 200].into_boxed_slice());
        for _ in 0..6 {
            m.trigger(&loud, 1.0, 1.0);
        }
        let mut out = vec![0.0f32; 64];
        m.render(&mut out, 1);
        assert!(out.iter().all(|x| x.abs() <= 1.0));
    }

    #[test]
    fn gain_scales_and_stop_all_silences() {
        let mut m = Mixer::new();
        let s: Arc<[f32]> = Arc::from(vec![0.5f32; 100].into_boxed_slice());
        m.trigger(&s, 0.1, 1.0);
        let mut out = vec![0.0f32; 4];
        m.render(&mut out, 1);
        assert!((out[0] - 0.05).abs() < 1e-6);
        m.stop_all();
        assert_eq!(m.active(), 0);
    }
}
