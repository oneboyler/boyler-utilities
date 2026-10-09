//! The player's brain (Order 062): what plays, how loud, fades, the sleep timer. Pure: no Windows, no clock of its own (the
//! caller says what time it is), so every rule is tested without a sound device.
//!
//! One `Core` exists only while the noise plays. It holds the loop (16-bit, interleaved L / R) and walks through it; the
//! stream's worker thread asks it to `render` the next stretch of frames. What it does per frame is one multiply per sample
//! (loop sample x a gain that moves smoothly), so playing costs next to nothing; nothing is generated at play time.
//!
//! Order 080: while your own mix plays and a slider moves, the worker makes the new loop and this swaps it in by a short
//! CROSSFADE (both loops play at the same position for [`MIX_FADE_SECS`]), not by fading through silence like a switch
//! between two noises - so the sound just changes under your hand.

use crate::sound::Sound;
use crate::synth::Loop;

/// Fade in on Play, fade out on Stop / when the sleep timer ends.
pub const FADE_SECS: f32 = 1.0;
/// Fade out of the old noise and in of the new one when another noise is picked while it plays.
pub const SWITCH_FADE_SECS: f32 = 0.4;
/// A moved slider of your own mix: the old loop turns into the new one over this long (equal power, no dip, no click).
pub const MIX_FADE_SECS: f32 = 0.15;
/// The volume slider is followed over this long (no zipper noise while it is dragged).
const VOLUME_SMOOTH_SECS: f32 = 0.06;
/// the owner's sound rule: quiet by default.
pub const DEFAULT_VOLUME: u8 = 10;
/// The sleep timer's choices in minutes (None = off).
pub const SLEEP_CHOICES: [Option<u32>; 5] = [None, Some(15), Some(30), Some(60), Some(90)];

/// The slider's percent as the amplitude it plays at: a gentle curve (v^1.5) so the quiet end has room. The loop is at about
/// -14 dBFS RMS, so 10 % is about -44 dBFS and 100 % about -14 dBFS. (The curve is my pick, not measured against his ears.)
pub fn gain_of(percent: u8) -> f32 {
    (f32::from(percent.min(100)) / 100.0).powf(1.5)
}

/// A loop that is turning into the loaded one (a mix whose slider moved).
struct Xfade {
    sound: Sound,
    data: Vec<i16>,
    /// 0 (all old) .. 1 (all new)
    t: f32,
}

pub struct Core {
    rate: u32,
    data: Vec<i16>,
    frames: usize,
    pos: usize,
    loaded: Option<Sound>,
    /// A loop that was made while the old one still plays: swapped in when the old one has faded out (or crossfaded in).
    pending: Option<(Sound, Loop)>,
    xf: Option<Xfade>,
    want: Sound,
    volume: u8,
    /// The volume's smoothed gain.
    vol_cur: f32,
    /// The fade's level, 0 (silent) .. 1 (full).
    env: f32,
    fade_secs: f32,
    stopping: bool,
    done: bool,
    sleep_at: Option<f64>,
}

impl Core {
    pub fn new(sound: impl Into<Sound>, volume: u8, rate: u32) -> Core {
        Core {
            rate: rate.max(1),
            data: Vec::new(),
            frames: 0,
            pos: 0,
            loaded: None,
            pending: None,
            xf: None,
            want: sound.into(),
            volume: volume.min(100),
            vol_cur: gain_of(volume),
            env: 0.0,
            fade_secs: FADE_SECS,
            stopping: false,
            done: false,
            sleep_at: None,
        }
    }

    // ------------------------------------------------------------------ what the page asks

    pub fn sound(&self) -> Sound {
        self.want
    }
    pub fn volume(&self) -> u8 {
        self.volume
    }
    pub fn rate(&self) -> u32 {
        self.rate
    }
    /// A stop (Stop, or the sleep timer) is fading out.
    pub fn is_stopping(&self) -> bool {
        self.stopping && !self.done
    }
    /// Faded out after a stop: the stream may close and everything is freed.
    pub fn is_done(&self) -> bool {
        self.done
    }
    /// Bytes of loop data held now.
    pub fn bytes(&self) -> usize {
        self.data.len() * 2 + self.pending.as_ref().map_or(0, |p| p.1.data.len() * 2) + self.xf.as_ref().map_or(0, |x| x.data.len() * 2)
    }

    /// Your mix is playing and another mix is wanted: the change is a crossfade, not a fade through silence.
    fn crossfades(&self) -> bool {
        matches!((self.loaded, self.want), (Some(Sound::Mix(a)), Sound::Mix(b)) if a != b)
    }

    /// Another sound. While one plays it fades out, the new one is swapped in and fades in (a mix to another mix: a crossfade).
    pub fn set_sound(&mut self, s: impl Into<Sound>) {
        let s = s.into();
        if s == self.want {
            return;
        }
        self.want = s;
        if self.loaded.is_some() && !(s.is_mix() && self.loaded.is_some_and(Sound::is_mix)) {
            self.fade_secs = SWITCH_FADE_SECS;
        }
        if let Some((pk, _)) = &self.pending {
            if *pk != s {
                self.pending = None;
            }
        }
    }

    pub fn set_volume(&mut self, percent: u8) {
        self.volume = percent.min(100);
    }

    /// The sleep timer: `minutes` from `now` (None = off).
    pub fn set_sleep(&mut self, now: f64, minutes: Option<u32>) {
        self.sleep_at = minutes.map(|m| now + f64::from(m) * 60.0);
    }

    /// Seconds until the sleep timer stops the noise (None = no timer).
    pub fn sleep_left(&self, now: f64) -> Option<f64> {
        self.sleep_at.map(|t| (t - now).max(0.0))
    }

    /// Stop: fade out over a second, then `is_done`.
    pub fn stop(&mut self) {
        if !self.stopping {
            self.stopping = true;
            self.fade_secs = FADE_SECS;
            self.pending = None;
            self.sleep_at = None;
        }
    }

    /// Play pressed again while it was fading out: fade back in.
    pub fn resume(&mut self) {
        if self.stopping && !self.done {
            self.stopping = false;
            self.fade_secs = FADE_SECS;
        }
    }

    /// The caller's clock moved: the sleep timer may be up.
    pub fn tick(&mut self, now: f64) {
        if let Some(t) = self.sleep_at {
            if now >= t {
                self.stop();
            }
        }
    }

    // ------------------------------------------------------------------ what the worker does

    /// The stream was (re)opened - maybe on another device: start from silence and fade in again.
    pub fn restart_fade(&mut self) {
        self.env = 0.0;
    }

    /// Nothing can be faded (there is no output): a stop is a stop at once, everything is freed.
    pub fn abort(&mut self) {
        if self.stopping {
            self.done = true;
            self.free();
        }
    }

    /// Everything held is let go (after the last fade-out).
    fn free(&mut self) {
        self.data = Vec::new();
        self.frames = 0;
        self.pos = 0;
        self.loaded = None;
        self.pending = None;
        self.xf = None;
    }

    /// The output's sample rate is `rate` (the stream was opened on another device): a loop made for another rate can't play.
    pub fn set_rate(&mut self, rate: u32) {
        let rate = rate.max(1);
        if rate != self.rate {
            self.rate = rate;
            self.free();
            self.env = 0.0;
        }
    }

    /// The sound that has to be made now (the worker makes it outside the lock and hands it to `set_loop`).
    pub fn needs_loop(&self) -> Option<Sound> {
        // (a crossfade in progress finishes first: the next step is asked for when it is over)
        if self.stopping || self.done || self.loaded == Some(self.want) || self.xf.is_some() {
            return None;
        }
        if matches!(&self.pending, Some((k, _)) if *k == self.want) {
            return None;
        }
        Some(self.want)
    }

    /// A made loop. false = not wanted any more (another sound was picked meanwhile, another rate, or it stopped).
    pub fn set_loop(&mut self, sound: impl Into<Sound>, l: Loop) -> bool {
        let sound = sound.into();
        if sound != self.want || l.rate != self.rate || self.stopping || self.done || l.frames == 0 {
            return false;
        }
        self.pending = Some((sound, l));
        true
    }

    /// Fills `out` (interleaved, `channels` per frame) with the next frames: silence when nothing is loaded yet.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        out.fill(0.0);
        if self.done || channels == 0 {
            return;
        }
        let frames = out.len() / channels;
        let rate = self.rate as f32;
        let step = 1.0 / (self.fade_secs * rate);
        let xstep = 1.0 / (MIX_FADE_SECS * rate);
        let coef = 1.0 - (-1.0 / (VOLUME_SMOOTH_SECS * rate)).exp();
        let vol_target = gain_of(self.volume);
        for f in 0..frames {
            let xfading = self.crossfades();
            // nothing audible and the wrong sound (or none) is loaded: swap in the made one, or wait in silence
            if self.env <= 0.0 && self.loaded != Some(self.want) && !xfading {
                if self.stopping {
                    self.done = true;
                    self.free();
                    break;
                }
                match self.pending.take() {
                    Some((k, l)) if k == self.want => {
                        self.data = l.data;
                        self.frames = l.frames;
                        self.pos = 0;
                        self.loaded = Some(k);
                        self.xf = None;
                    }
                    other => {
                        self.pending = other;
                        self.data = Vec::new();
                        self.frames = 0;
                        self.loaded = None;
                        self.xf = None;
                        break;
                    }
                }
            }
            // your mix, a slider moved, the new loop is made: it takes over from the loaded one by a crossfade, at the same position
            if xfading && self.xf.is_none() && !self.stopping {
                if let Some((k, l)) = self.pending.take() {
                    if k == self.want && l.frames == self.frames {
                        self.xf = Some(Xfade { sound: k, data: l.data, t: 0.0 });
                    } else {
                        // a loop of another length can not be crossfaded: switch through silence instead (never stuck)
                        self.pending = Some((k, l));
                        self.loaded = None;
                    }
                }
            }
            let target = if (self.loaded == Some(self.want) || xfading) && !self.stopping { 1.0 } else { 0.0 };
            if self.env < target {
                self.env = (self.env + step).min(target);
            } else if self.env > target {
                self.env = (self.env - step).max(target);
            }
            if self.env <= 0.0 && target == 0.0 && self.stopping {
                self.done = true;
                self.free();
                break;
            }
            self.vol_cur += (vol_target - self.vol_cur) * coef;
            if (vol_target - self.vol_cur).abs() < 1e-7 {
                self.vol_cur = vol_target;
            }
            if self.frames == 0 {
                break;
            }
            let g = self.vol_cur * self.env * self.env / 32768.0;
            let (i0, i1) = (self.pos * 2, self.pos * 2 + 1);
            let (mut l, mut r) = (f32::from(self.data[i0]), f32::from(self.data[i1]));
            if let Some(x) = self.xf.as_mut() {
                // equal power: the old loop and the new one are two different noises, their powers add up to a constant
                let (s, c) = (x.t * std::f32::consts::FRAC_PI_2).sin_cos();
                l = l * c + f32::from(x.data[i0]) * s;
                r = r * c + f32::from(x.data[i1]) * s;
                x.t += xstep;
            }
            let (l, r) = (l * g, r * g);
            let o = &mut out[f * channels..(f + 1) * channels];
            if channels == 1 {
                o[0] = (l + r) * 0.5;
            } else {
                // front left / right only: a 5.1 / 7.1 output keeps its centre and sub silent
                o[0] = l;
                o[1] = r;
            }
            self.pos += 1;
            if self.pos >= self.frames {
                self.pos = 0;
            }
            if self.xf.as_ref().is_some_and(|x| x.t >= 1.0) {
                if let Some(x) = self.xf.take() {
                    self.data = x.data;
                    self.loaded = Some(x.sound);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::Kind;

    const RATE: u32 = 1000;

    /// A loop of constant full-scale left and half-scale right of `kind`'s making (the content doesn't matter here).
    fn lp(frames: usize) -> Loop {
        let mut data = Vec::new();
        for _ in 0..frames {
            data.push(32767);
            data.push(16384);
        }
        Loop { rate: RATE, frames, data }
    }

    fn started(kind: Kind, vol: u8) -> Core {
        let mut c = Core::new(kind, vol, RATE);
        assert_eq!(c.needs_loop(), Some(kind.into()));
        assert!(c.set_loop(kind, lp(100)));
        assert_eq!(c.needs_loop(), None);
        c
    }

    fn render(c: &mut Core, frames: usize) -> Vec<f32> {
        let mut o = vec![0.0f32; frames * 2];
        c.render(&mut o, 2);
        o
    }

    #[test]
    fn the_gain_curve_is_quiet_at_ten_percent() {
        assert_eq!(gain_of(0), 0.0);
        assert_eq!(gain_of(100), 1.0);
        assert!((gain_of(10) - 0.0316).abs() < 0.001);
        assert!(gain_of(30) < gain_of(31));
        assert_eq!(gain_of(250), 1.0, "never above full");
    }

    #[test]
    fn nothing_plays_until_a_loop_is_there() {
        let mut c = Core::new(Kind::Pink, 50, RATE);
        assert!(render(&mut c, 500).iter().all(|x| *x == 0.0), "silence while waiting");
        assert_eq!(c.needs_loop(), Some(Kind::Pink.into()));
        assert!(c.set_loop(Kind::Pink, lp(100)));
        let o = render(&mut c, 500);
        assert!(o.iter().any(|x| *x != 0.0), "it plays once the loop is in");
    }

    #[test]
    fn it_fades_in_over_a_second_without_a_jump() {
        let mut c = started(Kind::White, 100);
        let o = render(&mut c, RATE as usize + 100);
        let left: Vec<f32> = o.iter().step_by(2).copied().collect();
        assert!(left[0] < 0.001, "starts silent");
        assert!(left.windows(2).all(|w| w[1] >= w[0] - 1e-6), "only rises");
        let max_step = left.windows(2).map(|w| w[1] - w[0]).fold(0.0f32, f32::max);
        assert!(max_step < 0.01, "no jump: biggest step {max_step}");
        assert!((left[RATE as usize + 50] - 32767.0 / 32768.0).abs() < 1e-3, "full after 1 s");
        assert!(left[RATE as usize / 2] < 0.3, "a quarter of the power half way (env squared)");
        // the right channel is the other half of the loop
        assert!((o[2 * (RATE as usize + 50) + 1] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn stop_fades_out_over_a_second_then_it_is_done() {
        let mut c = started(Kind::White, 100);
        render(&mut c, 2 * RATE as usize);
        c.stop();
        assert!(c.is_stopping() && !c.is_done());
        let o = render(&mut c, RATE as usize + 50);
        let left: Vec<f32> = o.iter().step_by(2).copied().collect();
        assert!(left.windows(2).all(|w| w[1] <= w[0] + 1e-6), "only falls");
        assert!(left[RATE as usize + 40] == 0.0);
        assert!(c.is_done());
        assert!(render(&mut c, 100).iter().all(|x| *x == 0.0));
    }

    #[test]
    fn play_again_while_fading_out_fades_back_in() {
        let mut c = started(Kind::White, 100);
        render(&mut c, 2 * RATE as usize);
        c.stop();
        render(&mut c, RATE as usize / 2);
        c.resume();
        assert!(!c.is_stopping());
        let o = render(&mut c, 2 * RATE as usize);
        assert!(o[o.len() - 2] > 0.9, "back at full");
    }

    #[test]
    fn another_noise_fades_out_swaps_and_fades_in() {
        let mut c = started(Kind::Brown, 100);
        render(&mut c, 2 * RATE as usize);
        c.set_sound(Kind::Grey);
        assert_eq!(c.needs_loop(), Some(Kind::Grey.into()), "it is made while the old one still fades");
        let o1 = render(&mut c, 100);
        assert!(o1[0] > 0.9, "the old one is still playing at the start");
        let mut other = lp(50);
        other.data.iter_mut().for_each(|v| *v /= 2);
        assert!(c.set_loop(Kind::Grey, other));
        assert_eq!(c.needs_loop(), None);
        let o2 = render(&mut c, 2 * RATE as usize);
        let left: Vec<f32> = o2.iter().step_by(2).copied().collect();
        let min = left.iter().copied().fold(1.0f32, f32::min);
        assert!(min < 0.001, "it went down to silence before the swap");
        assert!((left[left.len() - 1] - 0.5).abs() < 0.01, "then the new noise (half scale) at full");
        let max_step = left.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(max_step < 0.01, "no click anywhere: biggest step {max_step}");
        assert_eq!(c.sound(), Kind::Grey.into());
    }

    #[test]
    fn a_loop_for_a_noise_nobody_wants_any_more_is_refused() {
        let mut c = Core::new(Kind::Brown, 50, RATE);
        c.set_sound(Kind::Blue);
        assert!(!c.set_loop(Kind::Brown, lp(10)));
        assert!(c.set_loop(Kind::Blue, lp(10)));
        c.set_sound(Kind::White);
        assert_eq!(c.needs_loop(), Some(Kind::White.into()), "the pending blue was dropped");
        let mut wrong_rate = lp(10);
        wrong_rate.rate = 48_000;
        assert!(!c.set_loop(Kind::White, wrong_rate));
        c.stop();
        assert!(!c.set_loop(Kind::White, lp(10)), "nothing is made after a stop");
    }

    #[test]
    fn the_sleep_timer_stops_it_at_the_time_with_a_fade() {
        let mut c = started(Kind::Pink, 50);
        c.set_sleep(100.0, Some(15));
        assert_eq!(c.sleep_left(100.0), Some(900.0));
        assert_eq!(c.sleep_left(700.0), Some(300.0));
        c.tick(100.0 + 899.0);
        assert!(!c.is_stopping());
        c.tick(100.0 + 900.0);
        assert!(c.is_stopping(), "time is up: it fades out");
        render(&mut c, RATE as usize + 50);
        assert!(c.is_done());
        assert_eq!(c.sleep_left(2000.0), None);
    }

    #[test]
    fn the_sleep_timer_can_be_turned_off_and_restarted() {
        let mut c = started(Kind::Pink, 50);
        c.set_sleep(0.0, Some(30));
        c.set_sleep(10.0, None);
        c.tick(100_000.0);
        assert!(!c.is_stopping(), "off means off");
        c.set_sleep(1000.0, Some(60));
        assert_eq!(c.sleep_left(1000.0), Some(3600.0));
        c.set_sleep(2000.0, Some(15));
        assert_eq!(c.sleep_left(2000.0), Some(900.0), "a new choice counts from now");
    }

    #[test]
    fn the_volume_follows_the_slider_smoothly() {
        let mut c = started(Kind::White, 100);
        render(&mut c, 2 * RATE as usize);
        c.set_volume(10);
        let o = render(&mut c, 600);
        let left: Vec<f32> = o.iter().step_by(2).copied().collect();
        assert!(left[0] > 0.9, "no instant jump");
        assert!(left.windows(2).all(|w| w[1] <= w[0] + 1e-6));
        assert!((left[599] - gain_of(10) * 32767.0 / 32768.0).abs() < 0.002, "arrives at the new level");
    }

    #[test]
    fn the_loop_wraps_and_more_than_two_channels_get_front_left_right_only() {
        let mut c = Core::new(Kind::White, 100, RATE);
        let mut l = lp(3);
        l.data = vec![32767, -32767, 16384, -16384, 0, 0];
        c.set_loop(Kind::White, l);
        render(&mut c, 2 * RATE as usize);
        let mut o = vec![0.0f32; 12 * 4];
        c.render(&mut o, 4);
        assert!(o[0] != 0.0 || o[4] != 0.0);
        for fr in o.chunks(4) {
            assert_eq!(fr[2], 0.0, "channel 3 silent");
            assert_eq!(fr[3], 0.0, "channel 4 silent");
        }
    }

    #[test]
    fn a_new_output_rate_drops_the_loop() {
        let mut c = started(Kind::Brown, 50);
        render(&mut c, 100);
        assert!(c.bytes() > 0);
        c.set_rate(44_100);
        assert_eq!(c.bytes(), 0);
        assert_eq!(c.needs_loop(), Some(Kind::Brown.into()));
        assert!(render(&mut c, 100).iter().all(|x| *x == 0.0));
    }

    #[test]
    fn stopping_before_anything_loaded_is_done_at_once() {
        let mut c = Core::new(Kind::Brown, 50, RATE);
        c.stop();
        render(&mut c, 10);
        assert!(c.is_done());
    }
}

#[cfg(test)]
mod loudness {
    use super::*;
    use crate::kind::Kind;
    use crate::synth::make_loop_n;

    /// What the real thing comes out as: a brown loop at the default 10 % is about -44 dBFS RMS, at 100 % about -14 dBFS.
    #[test]
    fn the_default_is_quiet_and_full_is_not_clipping() {
        let rate = 48_000;
        let mut c = Core::new(Kind::Brown, DEFAULT_VOLUME, rate);
        c.set_loop(Kind::Brown, make_loop_n(Kind::Brown, rate, 1 << 16, 1));
        let mut o = vec![0.0f32; 2 * rate as usize];
        c.render(&mut o, 2); // the fade-in second
        let mut o = vec![0.0f32; 2 * 20_000];
        c.render(&mut o, 2);
        let rms = |v: &[f32]| (v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
        let db10 = 20.0 * rms(&o).log10();
        assert!((-47.0..=-41.0).contains(&db10), "10 % = {db10:.1} dBFS RMS (measured)");
        c.set_volume(100);
        let mut o = vec![0.0f32; 2 * rate as usize];
        c.render(&mut o, 2);
        let mut o = vec![0.0f32; 2 * 20_000];
        c.render(&mut o, 2);
        let db100 = 20.0 * rms(&o).log10();
        assert!((-17.0..=-11.0).contains(&db100), "100 % = {db100:.1} dBFS RMS (measured)");
        assert!(o.iter().all(|x| x.abs() <= 1.0), "never past full scale");
    }
}

#[cfg(test)]
mod more {
    use super::*;
    use crate::kind::Kind;

    fn lp() -> Loop {
        Loop { rate: 1000, frames: 4, data: vec![32767, 0, 32767, 0, 32767, 0, 32767, 0] }
    }

    /// No output device: nothing can fade, so a stop (or the sleep timer) must still end it - at once, everything freed.
    #[test]
    fn a_stop_without_any_output_ends_it_at_once() {
        let mut c = Core::new(Kind::Pink, 50, 1000);
        c.set_loop(Kind::Pink, lp());
        c.stop();
        assert!(c.is_stopping() && !c.is_done());
        c.abort();
        assert!(c.is_done());
        assert_eq!(c.bytes(), 0);
        // abort never ends something that was not stopped
        let mut p = Core::new(Kind::Pink, 50, 1000);
        p.abort();
        assert!(!p.is_done());
    }

    #[test]
    fn a_mono_output_gets_both_channels_mixed() {
        let mut c = Core::new(Kind::White, 100, 1000);
        c.set_loop(Kind::White, lp());
        let mut warm = vec![0.0f32; 2000];
        c.render(&mut warm, 1);
        let mut o = vec![0.0f32; 10];
        c.render(&mut o, 1);
        // left full scale, right silent: the mono mix is half
        assert!((o[0] - 0.5 * 32767.0 / 32768.0).abs() < 1e-3, "{}", o[0]);
    }
}

/// Order 080: your own mix changes under the slider (a crossfade), the noises still switch through silence.
#[cfg(test)]
mod mix_fade {
    use super::*;
    use crate::kind::Kind;
    use crate::sound::Mix;

    const RATE: u32 = 1000;
    const A: Mix = Mix { tone: 20, rumble: 0, waves: 0 };
    const B: Mix = Mix { tone: 80, rumble: 0, waves: 0 };
    const C: Mix = Mix { tone: 50, rumble: 40, waves: 10 };

    /// A loop of constant left `l` / right `r` (the content doesn't matter here, the levels do).
    fn lp(frames: usize, l: i16, r: i16) -> Loop {
        let mut data = Vec::new();
        for _ in 0..frames {
            data.push(l);
            data.push(r);
        }
        Loop { rate: RATE, frames, data }
    }

    fn render(c: &mut Core, frames: usize) -> Vec<f32> {
        let mut o = vec![0.0f32; frames * 2];
        c.render(&mut o, 2);
        o.iter().step_by(2).copied().collect()
    }

    /// A playing at full left (32767) and full volume.
    fn playing_a() -> Core {
        let mut c = Core::new(A, 100, RATE);
        assert_eq!(c.needs_loop(), Some(A.into()));
        assert!(c.set_loop(A, lp(100, 32767, 0)));
        render(&mut c, 2 * RATE as usize);
        assert_eq!(c.needs_loop(), None);
        c
    }

    #[test]
    fn a_moved_slider_crossfades_the_mix_without_a_dip_or_a_click() {
        let mut c = playing_a();
        c.set_sound(B);
        assert_eq!(c.needs_loop(), Some(B.into()));
        // before the new loop is there the old one just goes on, at full level (no fade out, no silence)
        let o = render(&mut c, 500);
        assert!(o.iter().all(|x| *x > 0.99), "the old mix plays until the new one is made");
        assert!(c.set_loop(B, lp(100, 16384, 0)));
        let o = render(&mut c, 400);
        // the new one is half as loud: the output goes from 1.0 to 0.5 and never below, in small steps
        assert!(o.iter().all(|x| *x > 0.5 - 1e-3), "no dip below the quieter of the two: {:?}", o.iter().copied().fold(1.0f32, f32::min));
        assert!(o.iter().all(|x| *x < 1.2), "equal power: at most a small bump when two loops of different level meet");
        assert!((o[399] - 0.5).abs() < 1e-3, "it arrives at the new mix");
        let max_step = o.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(max_step < 0.02, "no click: biggest step {max_step}");
        // it took MIX_FADE_SECS: after half of it the new loop is not yet all there
        let mut d = playing_a();
        d.set_sound(B);
        d.set_loop(B, lp(100, 16384, 0));
        let o = render(&mut d, (MIX_FADE_SECS * RATE as f32) as usize + 20);
        assert!(o[(MIX_FADE_SECS * RATE as f32 / 2.0) as usize] > 0.6, "half way is still mostly the old one");
        assert!((o[o.len() - 1] - 0.5).abs() < 1e-3);
        assert_eq!(d.sound(), B.into());
        assert_eq!(d.bytes(), 100 * 2 * 2, "one loop held again after it");
    }

    #[test]
    fn equal_power_two_different_noises_keep_the_level_through_the_fade() {
        // two uncorrelated signals of the same RMS: alternating +-1 (period 2) against a period-3 pattern
        let mut c = Core::new(A, 100, RATE);
        let mut l = lp(6, 0, 0);
        for i in 0..6 {
            l.data[i * 2] = if i % 2 == 0 { 20000 } else { -20000 };
        }
        c.set_loop(A, l);
        render(&mut c, 2 * RATE as usize);
        c.set_sound(B);
        let mut n = lp(6, 0, 0);
        for i in 0..6 {
            n.data[i * 2] = [20000, 20000, -20000][i % 3] * if i < 3 { 1 } else { -1 };
        }
        c.set_loop(B, n);
        let o = render(&mut c, 150);
        let rms = |v: &[f32]| (v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
        let before = 20000.0 / 32768.0;
        for w in o.chunks(30) {
            let r = rms(w);
            assert!((r / f64::from(before) - 1.0).abs() < 0.35, "the level through the fade: {r:.3} vs {before:.3}");
        }
    }

    #[test]
    fn a_noise_to_a_mix_and_back_still_fades_through_silence() {
        let mut c = Core::new(Kind::Brown, 100, RATE);
        c.set_loop(Kind::Brown, lp(100, 32767, 0));
        render(&mut c, 2 * RATE as usize);
        c.set_sound(A);
        assert_eq!(c.needs_loop(), Some(A.into()));
        c.set_loop(A, lp(100, 16384, 0));
        let o = render(&mut c, 2 * RATE as usize);
        assert!(o.iter().copied().fold(1.0f32, f32::min) < 0.001, "a switch between a noise and a mix goes down to silence");
        assert!((o[o.len() - 1] - 0.5).abs() < 0.01);
        c.set_sound(Kind::Pink);
        c.set_loop(Kind::Pink, lp(100, 8192, 0));
        let o = render(&mut c, 2 * RATE as usize);
        assert!(o.iter().copied().fold(1.0f32, f32::min) < 0.001);
    }

    #[test]
    fn another_move_during_a_crossfade_waits_for_it_and_is_made_after() {
        let mut c = playing_a();
        c.set_sound(B);
        c.set_loop(B, lp(100, 16384, 0));
        render(&mut c, 50);
        c.set_sound(C);
        assert_eq!(c.needs_loop(), None, "the crossfade in progress finishes first");
        render(&mut c, 200);
        assert_eq!(c.needs_loop(), Some(C.into()), "then the newest one is made (not the one in between)");
        assert!(c.set_loop(C, lp(100, 8192, 0)));
        let o = render(&mut c, 300);
        assert!((o[299] - 0.25).abs() < 1e-3);
        let max_step = o.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(max_step < 0.02, "no click: {max_step}");
    }

    #[test]
    fn a_loop_for_a_mix_nobody_wants_any_more_is_refused() {
        let mut c = playing_a();
        c.set_sound(B);
        c.set_sound(C);
        assert!(!c.set_loop(B, lp(100, 1, 1)), "B was replaced by C before it was made");
        assert!(c.set_loop(C, lp(100, 1, 1)));
        c.set_sound(B);
        assert_eq!(c.needs_loop(), Some(B.into()), "the pending C was dropped");
        // the same mix again is nothing to make
        c.set_sound(A);
        assert_eq!(c.needs_loop(), None, "back to the playing mix: nothing to do");
    }

    #[test]
    fn a_stop_in_the_middle_of_a_crossfade_ends_cleanly() {
        let mut c = playing_a();
        c.set_sound(B);
        c.set_loop(B, lp(100, 16384, 0));
        render(&mut c, 60);
        c.stop();
        let o = render(&mut c, RATE as usize + 50);
        assert!(c.is_done());
        assert_eq!(c.bytes(), 0, "everything freed");
        assert!(o.iter().all(|x| *x >= 0.0 && *x < 1.2));
        assert!(o[o.len() - 1] == 0.0);
    }

    #[test]
    fn a_new_rate_drops_a_crossfade_too() {
        let mut c = playing_a();
        c.set_sound(B);
        c.set_loop(B, lp(100, 16384, 0));
        render(&mut c, 10);
        c.set_rate(44_100);
        assert_eq!(c.bytes(), 0);
        assert_eq!(c.needs_loop(), Some(B.into()));
    }

    #[test]
    fn a_loop_of_another_length_is_switched_through_silence_not_stuck() {
        let mut c = playing_a();
        c.set_sound(B);
        assert!(c.set_loop(B, lp(77, 16384, 0)));
        let o = render(&mut c, 2 * RATE as usize);
        assert!(o.iter().copied().fold(1.0f32, f32::min) < 0.001, "it went down to silence");
        assert!((o[o.len() - 1] - 0.5).abs() < 0.01, "and the new one plays");
        assert_eq!(c.needs_loop(), None);
    }

    #[test]
    fn moving_back_to_the_playing_mix_does_not_shorten_the_play_fade() {
        let mut c = Core::new(A, 100, RATE);
        c.set_loop(A, lp(100, 32767, 0));
        render(&mut c, 100);
        c.set_sound(B);
        c.set_sound(A);
        assert_eq!(c.fade_secs, FADE_SECS);
    }
}
