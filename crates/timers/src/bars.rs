//! On-screen timer bars — the STATE behind the folded card (the drawing, menu-v18). Drawing the bars on screen (one
//! click-through topmost window) is a later UI order; mapping each bar's key is a later app-layer order (keys are
//! never registered here). This module holds what the UI shows and does:
//!
//! * the card switch (**off** by default); off = nothing starts and running bars are cleared;
//! * each bar: colour (click = next colour), name, duration ("45" = 45 s, "1:30", "2m"), its key's name, Try it, remove;
//! * starting a bar starts it from full — pressing its key again starts it over;
//! * what the screen shows now ([`TimerBars::screen`]): running bars, or all bars frozen part-way while the card is open
//!   (preview) or while "Move bars" is on; the last 3 s red; finished bars fade out;
//! * "On screen": position (6 quick spots + a dragged spot), size S / M / L, opacity, which monitor.

use crate::parse::{format_countdown, parse_time, BareUnit};
use crate::{Clock, Result, TimerError};
use std::time::Duration;

/// The drawing's bar colours, in "next colour" order.
pub const COLOURS: [u32; 5] = [0x0a84ff, 0x2fd6c4, 0xffb340, 0xff6b8a, 0xbf5af2];
/// Longest bar name (the drawing's maxlength).
pub const NAME_MAX: usize = 18;
/// A bar turns red for its last 3 seconds.
pub const LOW_SECS: u64 = 3;
/// A new bar (+ Add a bar): "Timer N", 30 s.
pub const NEW_BAR_SECS: u64 = 30;
/// The pill at size M, px (the drawing's .tbx).
pub const PILL_M: (u32, u32) = (196, 32);
/// Distance from the screen edge for the quick spots, px (the drawing's EDGE).
pub const EDGE_PX: i32 = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bar {
    pub id: u32,
    pub name: String,
    pub duration: Duration,
    /// Index into [`COLOURS`].
    pub colour: usize,
    /// The key's name as the key field shows it (e.g. "F7"), `None` = "Click to set a key". The app layer owns keys.
    pub key: Option<String>,
}

impl Bar {
    pub fn colour_rgb(&self) -> u32 {
        COLOURS[self.colour % COLOURS.len()]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Horiz {
    Left,
    Centre,
    Right,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vert {
    Top,
    Bottom,
}

/// Where the bars sit: an anchor + the distance from that edge (px). Measured from the monitor's work area (the
/// taskbar excluded) — the drawing fakes a 48 px taskbar instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spot {
    pub h: Horiz,
    pub v: Vert,
    pub dx: i32,
    pub dy: i32,
}

/// The six quick spots, in the drawing's order (TL, TC, TR, BL, BC, BR).
pub const QUICK_SPOTS: [(Horiz, Vert); 6] = [
    (Horiz::Left, Vert::Top),
    (Horiz::Centre, Vert::Top),
    (Horiz::Right, Vert::Top),
    (Horiz::Left, Vert::Bottom),
    (Horiz::Centre, Vert::Bottom),
    (Horiz::Right, Vert::Bottom),
];

impl Spot {
    pub fn quick(h: Horiz, v: Vert) -> Spot {
        Spot { h, v, dx: if h == Horiz::Centre { 0 } else { EDGE_PX }, dy: EDGE_PX }
    }
    /// Which quick spot this is, if it is one exactly (the drawing lights that spot's button).
    pub fn quick_index(&self) -> Option<usize> {
        QUICK_SPOTS.iter().position(|(h, v)| Spot::quick(*h, *v) == *self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarSize {
    S,
    M,
    L,
}

impl BarSize {
    /// The drawing's scale per size (KS): S .82, M 1, L 1.22.
    pub fn scale(self) -> f64 {
        match self {
            BarSize::S => 0.82,
            BarSize::M => 1.0,
            BarSize::L => 1.22,
        }
    }
    /// The pill's size in px at this size (196 × 32 at M).
    pub fn pill_px(self) -> (u32, u32) {
        let k = self.scale();
        ((PILL_M.0 as f64 * k).round() as u32, (PILL_M.1 as f64 * k).round() as u32)
    }
}

/// "Show on": the drawing's choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShowOn {
    Main,
    /// A monitor by its Windows device name (e.g. "\\.\DISPLAY2").
    Monitor(String),
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarsLook {
    pub spot: Spot,
    pub size: BarSize,
    /// 30–100 %.
    pub opacity_pct: u8,
    pub show_on: ShowOn,
}

impl Default for BarsLook {
    /// Top middle, 24 px, size M, 100 %, main monitor (the drawing's defaults).
    fn default() -> Self {
        BarsLook { spot: Spot::quick(Horiz::Centre, Vert::Top), size: BarSize::M, opacity_pct: 100, show_on: ShowOn::Main }
    }
}

/// One pill as the screen shows it now.
#[derive(Debug, Clone, PartialEq)]
pub struct PillView {
    pub id: u32,
    pub name: String,
    pub colour_rgb: u32,
    pub left: Duration,
    /// "1:29"
    pub left_text: String,
    /// The thin line: 1.0 full … 0.0 empty.
    pub line: f64,
    /// Red: the last 3 s of a running bar.
    pub low: bool,
    /// Running (false = a frozen preview pill).
    pub running: bool,
}

/// What the on-screen window should show now.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScreenView {
    /// Stacked top to bottom in the bars' list order.
    pub pills: Vec<PillView>,
    /// Bars that finished since the last call: fade them out (the drawing: 260 ms).
    pub finished: Vec<u32>,
}

impl ScreenView {
    /// Nothing to show: the window can hide and stop redrawing.
    pub fn is_empty(&self) -> bool {
        self.pills.is_empty()
    }
}

pub struct TimerBars<C: Clock> {
    clock: C,
    on: bool,
    /// The card is unfolded.
    pub open: bool,
    /// "Move bars" is on (drag them on screen).
    moving: bool,
    bars: Vec<Bar>,
    /// (bar id, ends at) while a bar runs.
    running: Vec<(u32, Duration)>,
    seq: u32,
    pub look: BarsLook,
}

impl<C: Clock> TimerBars<C> {
    /// Off, folded, no bars.
    pub fn new(clock: C) -> Self {
        TimerBars { clock, on: false, open: false, moving: false, bars: Vec::new(), running: Vec::new(), seq: 0, look: BarsLook::default() }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    /// The card switch. On unfolds the card; off folds it, stops "Move bars" and clears every running bar.
    pub fn set_on(&mut self, on: bool) {
        self.on = on;
        self.open = on;
        if !on {
            self.moving = false;
            self.running.clear();
        }
    }

    pub fn is_moving(&self) -> bool {
        self.moving
    }
    /// "Move bars" / "Done".
    pub fn set_moving(&mut self, v: bool) {
        self.moving = v && self.on;
    }

    pub fn bars(&self) -> &[Bar] {
        &self.bars
    }

    pub fn bar(&self, id: u32) -> Result<&Bar> {
        self.bars.iter().find(|b| b.id == id).ok_or(TimerError::NoSuchBar(id))
    }
    fn bar_mut(&mut self, id: u32) -> Result<&mut Bar> {
        self.bars.iter_mut().find(|b| b.id == id).ok_or(TimerError::NoSuchBar(id))
    }

    /// Adds a bar with a name and seconds; the colour follows the drawing (`seq % 5`). Returns its id.
    pub fn add(&mut self, name: &str, duration: Duration) -> u32 {
        self.seq += 1;
        let id = self.seq;
        let name = clean_name(name).unwrap_or_else(|| format!("Timer {}", self.bars.len() + 1));
        let duration = duration.min(Duration::from_secs(crate::parse::MAX_SECS));
        self.bars.push(Bar { id, name, duration, colour: id as usize % COLOURS.len(), key: None });
        id
    }

    /// "+ Add a bar": "Timer N", 30 s.
    pub fn add_new(&mut self) -> u32 {
        let n = format!("Timer {}", self.bars.len() + 1);
        self.add(&n, Duration::from_secs(NEW_BAR_SECS))
    }

    /// × on hover.
    pub fn remove(&mut self, id: u32) -> Result<Bar> {
        let i = self.bars.iter().position(|b| b.id == id).ok_or(TimerError::NoSuchBar(id))?;
        self.running.retain(|(r, _)| *r != id);
        Ok(self.bars.remove(i))
    }

    /// Editing the name: trimmed, at most 18 characters; an empty name keeps the old one.
    pub fn rename(&mut self, id: u32, name: &str) -> Result<String> {
        let b = self.bar_mut(id)?;
        if let Some(n) = clean_name(name) {
            b.name = n;
        }
        Ok(b.name.clone())
    }

    /// Editing the duration: a bare number is seconds ("45"), else as the countdown ("1:30", "2m"). A time that can't be
    /// read keeps the old one and returns the error.
    pub fn set_duration_text(&mut self, id: u32, text: &str) -> Result<Duration> {
        let secs = parse_time(text, BareUnit::Seconds).ok_or_else(|| TimerError::InvalidTime(text.to_string()));
        let b = self.bar_mut(id)?;
        b.duration = Duration::from_secs(secs?);
        Ok(b.duration)
    }

    /// Click on the colour dot: the next colour.
    pub fn next_colour(&mut self, id: u32) -> Result<u32> {
        let b = self.bar_mut(id)?;
        b.colour = (b.colour + 1) % COLOURS.len();
        Ok(b.colour_rgb())
    }

    /// Stores the key's name the app layer gave this bar (`None` clears it).
    pub fn set_key(&mut self, id: u32, key: Option<&str>) -> Result<()> {
        self.bar_mut(id)?.key = key.map(str::to_string);
        Ok(())
    }

    /// Two bars with the same key: the "Already used by …" line (one line under the bars). Keys used elsewhere in the app
    /// are checked by the app layer (one key is never used twice — DESIGN §2.5).
    pub fn key_clash(&self) -> Option<String> {
        for (i, a) in self.bars.iter().enumerate() {
            let Some(k) = &a.key else { continue };
            if let Some(b) = self.bars[..i].iter().find(|b| b.key.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(k))) {
                return Some(format!("Already used by Timer bar · {}", b.name));
            }
        }
        None
    }

    /// A bar's key or ▶ Try it: starts it from full; pressing again starts it over. Refused while the card is off.
    pub fn start(&mut self, id: u32) -> Result<()> {
        if !self.on {
            return Err(TimerError::BarsOff);
        }
        let d = self.bar(id)?.duration;
        let end = self.clock.now().saturating_add(d);
        self.running.retain(|(r, _)| *r != id);
        self.running.push((id, end));
        Ok(())
    }

    pub fn is_running(&self, id: u32) -> bool {
        let now = self.clock.now();
        self.running.iter().any(|(r, end)| *r == id && *end > now)
    }

    /// The soonest moment the screen changes by itself (a bar ends, or one turns red) — the app sleeps until then.
    /// While a bar runs the window redraws its digits anyway; this is for the app's own wake-up.
    pub fn deadline(&self) -> Option<Duration> {
        let now = self.clock.now();
        let low = Duration::from_secs(LOW_SECS);
        self.running
            .iter()
            .map(|(_, e)| match e.checked_sub(low) {
                // the red turn first, while it is still ahead
                Some(red) if red > now => red,
                _ => *e,
            })
            .min()
    }

    /// What the screen shows now. `card_visible` = the menu is open on Timers with the card unfolded (then every bar shows,
    /// frozen part-way like the drawing, so the look can be seen). Bars that ended are reported once in `finished`.
    pub fn screen(&mut self, card_visible: bool) -> ScreenView {
        let now = self.clock.now();
        let mut finished = Vec::new();
        self.running.retain(|(id, end)| {
            if now >= *end {
                finished.push(*id);
                false
            } else {
                true
            }
        });
        let preview = self.on && ((card_visible && self.open) || self.moving);
        let mut pills = Vec::new();
        if self.on {
            for (i, b) in self.bars.iter().enumerate() {
                let run = self.running.iter().find(|(id, _)| *id == b.id).map(|(_, e)| *e);
                if run.is_none() && !preview {
                    continue;
                }
                let left = match run {
                    Some(end) => end.saturating_sub(now),
                    // frozen part-way: 70 % / 35 % alternating (the drawing)
                    None => b.duration.mul_f64(if i % 2 == 1 { 0.35 } else { 0.7 }),
                };
                let line = if b.duration.is_zero() { 0.0 } else { (left.as_secs_f64() / b.duration.as_secs_f64()).clamp(0.0, 1.0) };
                pills.push(PillView {
                    id: b.id,
                    name: b.name.clone(),
                    colour_rgb: b.colour_rgb(),
                    left,
                    left_text: format_countdown(left),
                    line,
                    low: run.is_some() && left <= Duration::from_secs(LOW_SECS),
                    running: run.is_some(),
                });
            }
        }
        if preview {
            finished.clear();
        }
        ScreenView { pills, finished }
    }

    /// The opacity slider: 30–100 %.
    pub fn set_opacity(&mut self, pct: u8) {
        self.look.opacity_pct = pct.clamp(30, 100);
    }
}

fn clean_name(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    Some(t.chars().take(NAME_MAX).collect())
}
