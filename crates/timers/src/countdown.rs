//! The countdown tile: click the digits to type a time ("5" = 5 min, "1:30", "90s", "1h 20m"), Enter applies and
//! starts. Start / Pause / Resume / Again, Reset. A thin line drains while it runs (empty at rest). At zero: the chime
//! (if "Soft sound at the end" is on), the digits blink (UI), the toast "Countdown done · 5:00".
//!
//! Nothing ticks: [`Countdown::deadline`] says when it ends, the app sleeps until then (see [`crate::alarm`]) and calls
//! [`Countdown::check`] once.

use crate::parse::{format_countdown, parse_time, BareUnit};
use crate::sound::{chime_wav, SoundOs};
use crate::{Clock, Result, TimerError};
use std::time::Duration;

/// The main button's word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdButton {
    Start,
    Pause,
    Resume,
    Again,
}

/// What happened at zero, for the app to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountdownDone {
    /// The time that was set (e.g. 5:00).
    pub set: Duration,
    /// "Countdown done · 5:00" — the toast (in the menu) / the Windows notification text (menu closed).
    pub toast: String,
    /// The chime was played (sound switch on and it played).
    pub chimed: bool,
}

pub struct Countdown<C: Clock> {
    clock: C,
    set: Duration,
    left: Duration,
    end: Option<Duration>,
    done: bool,
    /// "Sound at the end" — OFF by default (the owner, Oct 8); when on, the chime plays at 5 %.
    pub sound_on: bool,
}

/// The drawing's starting time: 5:00.
pub const DEFAULT_SET: Duration = Duration::from_secs(300);

impl<C: Clock> Countdown<C> {
    pub fn new(clock: C) -> Self {
        Countdown { clock, set: DEFAULT_SET, left: DEFAULT_SET, end: None, done: false, sound_on: false }
    }

    pub fn is_running(&self) -> bool {
        self.end.is_some()
    }
    pub fn is_done(&self) -> bool {
        self.done
    }
    pub fn set_time(&self) -> Duration {
        self.set
    }

    /// Time left now.
    pub fn left(&self) -> Duration {
        match self.end {
            Some(end) => end.saturating_sub(self.clock.now()),
            None => self.left,
        }
    }

    /// The digits: "4:59" (partial seconds round up).
    pub fn text(&self) -> String {
        format_countdown(self.left())
    }

    /// At rest (set, never started since): the line is empty and Reset is disabled.
    pub fn at_rest(&self) -> bool {
        !self.is_running() && !self.done && self.left == self.set
    }

    /// The draining line, 1.0 = full … 0.0 = empty. Empty at rest.
    pub fn line(&self) -> f64 {
        if self.at_rest() || self.set.is_zero() {
            return 0.0;
        }
        (self.left().as_secs_f64() / self.set.as_secs_f64()).clamp(0.0, 1.0)
    }

    /// Typing a new time (field left / Enter). Refused while running (the digits are read-only then).
    pub fn type_time(&mut self, text: &str) -> Result<Duration> {
        if self.is_running() {
            return Err(TimerError::Running);
        }
        let secs = parse_time(text, BareUnit::Minutes).ok_or_else(|| TimerError::InvalidTime(text.to_string()))?;
        self.set = Duration::from_secs(secs);
        self.left = self.set;
        self.done = false;
        Ok(self.set)
    }

    /// Enter in the field: apply the typed time and start.
    pub fn enter(&mut self, text: &str) -> Result<Duration> {
        let set = self.type_time(text)?;
        if !self.is_running() {
            self.start_pause();
        }
        Ok(set)
    }

    /// The main button: Start / Pause / Resume / Again.
    pub fn start_pause(&mut self) {
        let now = self.clock.now();
        match self.end.take() {
            Some(end) => self.left = end.saturating_sub(now),
            None => {
                if self.done || self.left.is_zero() {
                    self.left = self.set;
                    self.done = false;
                }
                self.end = Some(now.saturating_add(self.left));
            }
        }
    }

    pub fn button(&self) -> CdButton {
        if self.is_running() {
            CdButton::Pause
        } else if self.done {
            CdButton::Again
        } else if self.left < self.set {
            CdButton::Resume
        } else {
            CdButton::Start
        }
    }

    pub fn reset_enabled(&self) -> bool {
        !self.at_rest()
    }

    /// Back to the set time, stopped.
    pub fn reset(&mut self) {
        self.end = None;
        self.left = self.set;
        self.done = false;
    }

    /// When it reaches zero, on this countdown's clock (`None` while not running).
    pub fn deadline(&self) -> Option<Duration> {
        self.end
    }

    /// Call when the alarm wakes (or any time): at or past zero it finishes ONCE — plays the chime through `sound` if
    /// the switch is on — and returns what to show. Before zero, or when not running: `None`.
    pub fn check(&mut self, sound: &mut dyn SoundOs) -> Option<CountdownDone> {
        let end = self.end?;
        if self.clock.now() < end {
            return None;
        }
        self.end = None;
        self.left = Duration::ZERO;
        self.done = true;
        let chimed = self.sound_on && sound.play_wav(chime_wav()).is_ok();
        Some(CountdownDone { set: self.set, toast: format!("Countdown done · {}", format_countdown(self.set)), chimed })
    }

    /// The ▶ preview next to the sound switch: plays the chime if the switch is on (the drawing previews "none" when off).
    pub fn preview_sound(&self, sound: &mut dyn SoundOs) -> Result<bool> {
        if !self.sound_on {
            return Ok(false);
        }
        sound.play_wav(chime_wav())?;
        Ok(true)
    }
}
