//! The stopwatch tile: Start / Stop / Resume, Lap, Reset. Laps newest first (lap time + total), the best lap marked
//! (only when there are at least two laps, as drawn).

use crate::parse::format_stopwatch;
use crate::Clock;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lap {
    /// 1, 2, 3 … in the order they were taken.
    pub n: u32,
    /// This lap's own time.
    pub lap: Duration,
    /// The stopwatch total when the lap was taken.
    pub total: Duration,
}

/// One row of the lap list, ready to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LapRow {
    pub lap: Lap,
    /// "Lap 3"
    pub label: String,
    pub lap_text: String,
    pub total_text: String,
    /// The fastest lap (green). Every lap that ties the fastest is marked.
    pub best: bool,
}

/// The main button's word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwButton {
    Start,
    Stop,
    Resume,
}

pub struct Stopwatch<C: Clock> {
    clock: C,
    running_since: Option<Duration>,
    acc: Duration,
    laps: Vec<Lap>,
}

impl<C: Clock> Stopwatch<C> {
    pub fn new(clock: C) -> Self {
        Stopwatch { clock, running_since: None, acc: Duration::ZERO, laps: Vec::new() }
    }

    pub fn is_running(&self) -> bool {
        self.running_since.is_some()
    }

    /// The time on the stopwatch now.
    pub fn elapsed(&self) -> Duration {
        self.acc + self.running_since.map(|t0| self.clock.now().saturating_sub(t0)).unwrap_or_default()
    }

    /// "m:ss.cc"
    pub fn text(&self) -> String {
        format_stopwatch(self.elapsed())
    }

    /// The main button: Start → Stop → Resume → Stop …
    pub fn start_stop(&mut self) {
        match self.running_since.take() {
            Some(t0) => self.acc += self.clock.now().saturating_sub(t0),
            None => self.running_since = Some(self.clock.now()),
        }
    }

    pub fn button(&self) -> SwButton {
        if self.is_running() {
            SwButton::Stop
        } else if self.elapsed() > Duration::ZERO {
            SwButton::Resume
        } else {
            SwButton::Start
        }
    }

    /// Takes a lap. Only while running (the Lap button is disabled otherwise) — returns `None` then.
    pub fn lap(&mut self) -> Option<Lap> {
        if !self.is_running() {
            return None;
        }
        let total = self.elapsed();
        let prev = self.laps.last().map(|l| l.total).unwrap_or_default();
        let l = Lap { n: self.laps.len() as u32 + 1, lap: total.saturating_sub(prev), total };
        self.laps.push(l);
        Some(l)
    }

    pub fn lap_enabled(&self) -> bool {
        self.is_running()
    }

    pub fn reset_enabled(&self) -> bool {
        self.is_running() || self.elapsed() > Duration::ZERO
    }

    /// Back to 0:00.00, no laps, stopped.
    pub fn reset(&mut self) {
        self.running_since = None;
        self.acc = Duration::ZERO;
        self.laps.clear();
    }

    /// The laps as taken (oldest first).
    pub fn laps(&self) -> &[Lap] {
        &self.laps
    }

    /// The lap list as shown: newest first, the best lap marked when there are two or more.
    pub fn lap_rows(&self) -> Vec<LapRow> {
        let best = if self.laps.len() > 1 { self.laps.iter().map(|l| l.lap).min() } else { None };
        self.laps
            .iter()
            .rev()
            .map(|l| LapRow {
                lap: *l,
                label: format!("Lap {}", l.n),
                lap_text: format_stopwatch(l.lap),
                total_text: format_stopwatch(l.total),
                best: Some(l.lap) == best,
            })
            .collect()
    }
}
