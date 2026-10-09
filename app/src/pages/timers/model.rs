//! The timers themselves (menu-v22 Timers: TM + CLK of the drawing). They must keep running while the menu is closed
//! (the menu window and its pages are dropped on close), so the model lives in a thread-local of the UI thread - the
//! page only borrows it while shown. Everything time-related goes through bu-timers: `Stopwatch` / `Countdown` on one
//! clock (`MonoClock`, or a `FakeClock` in test copies), the end chime through `SoundOs` (`RealSound`, or `FakeSound`
//! in test copies), the world clock through `zones::ZoneOs` (`RealZones` / `FakeZones`).

use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use bu_timers::parse::{format_countdown, format_stopwatch};
use bu_timers::sound::SoundOs;
use bu_timers::stopwatch::LapRow;
use bu_timers::zones::{self, Place, ZoneOs};
use bu_timers::{Clock, FakeClock, FakeSound, MonoClock};

use crate::gfx::Rgba;

/// One clock for every timer (an `Arc` so each Stopwatch / Countdown holds the same one).
#[derive(Clone)]
pub struct Clk(pub Arc<dyn Clock>);

impl Clock for Clk {
    fn now(&self) -> Duration {
        self.0.now()
    }
}

/// `TBCOL` - each new timer takes the next colour.
pub const TBCOL: [u32; 5] = [0x0a84ff, 0x2fd6c4, 0xffb340, 0xff6b8a, 0xbf5af2];
/// Order 055: the Timers page repaints a running countdown's line / ring in steps of a quarter pixel of the 260 px line.
pub const RING_STEPS: f64 = 1040.0;
/// `.thn` maxlength
pub const NAME_MAX: usize = 22;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Sw,
    Cd,
    Clk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Sw,
    Cd,
}

pub struct Timer {
    pub id: u32,
    pub kind: Kind,
    /// Order 078: the tab's own Stopwatch / Countdown (the big one on top): never in "Your timers", never removed
    pub own: bool,
    pub name: String,
    pub colour: u32,
    /// "On screen"
    pub screen: bool,
    /// its key (the key field's text); the keys manager registers it once Order 014 item 2 is in
    pub key: Option<String>,
    pub sw: bu_timers::stopwatch::Stopwatch<Clk>,
    pub cd: bu_timers::countdown::Countdown<Clk>,
}

impl Timer {
    pub fn rgba(&self) -> Rgba {
        Rgba::hex(self.colour)
    }
    pub fn running(&self) -> bool {
        match self.kind {
            Kind::Sw => self.sw.is_running(),
            Kind::Cd => self.cd.is_running(),
        }
    }
    /// The list row's time: a stopwatch without its hundredths (`fmtSw(..).replace(/\.\d\d$/,'')`), a countdown's left.
    pub fn short_text(&self) -> String {
        match self.kind {
            Kind::Sw => {
                let t = format_stopwatch(self.sw.elapsed());
                t[..t.len() - 3].to_string()
            }
            Kind::Cd => self.cd.text(),
        }
    }
    /// The big time.
    pub fn big_text(&self) -> String {
        match self.kind {
            Kind::Sw => self.sw.text(),
            Kind::Cd => self.cd.text(),
        }
    }
    /// The row's small line: "Stopwatch" / "Countdown · 12:00".
    pub fn kind_text(&self) -> String {
        match self.kind {
            Kind::Sw => "Stopwatch".into(),
            Kind::Cd => format!("Countdown \u{b7} {}", format_countdown(self.cd.set_time())),
        }
    }
    /// The countdown's share left (the ring / the line): 0..1; a stopwatch 0.
    pub fn share_left(&self) -> f32 {
        if self.kind == Kind::Sw || self.cd.set_time().is_zero() {
            return 0.0;
        }
        (self.cd.left().as_secs_f64() / self.cd.set_time().as_secs_f64()).clamp(0.0, 1.0) as f32
    }
    /// Shown on screen while it runs (a stopwatch also while it holds a time).
    pub fn busy(&self) -> bool {
        self.running() || (self.kind == Kind::Sw && self.sw.elapsed() > Duration::ZERO)
    }
}

/// A place of the World clock list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceItem {
    pub place: Place,
    pub screen: bool,
}

/// One result of the "Add a place" search: the place and its time there now ("04:37", a day later / earlier / the same).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub place: Place,
    pub time: String,
    pub day: i64,
}

/// How many results the "Add a place" box shows.
pub const FOUND_MAX: usize = 8;

/// One pill of the on-screen timers (#tbars .tbx).
#[derive(Clone, Debug, PartialEq)]
pub struct Pill {
    pub key: String,
    pub name: String,
    pub time: String,
    pub colour: Rgba,
    /// the drain line 0..1 (`None` = no line: a stopwatch or a place, `.tbx.nl`)
    pub line: Option<f32>,
    /// the last 3 s of a running countdown (`.low`)
    pub low: bool,
}

/// The drawing's `S.tb.spot` (where the on-screen timers sit; top middle, 24 px down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    /// 'L' / 'C' / 'R'
    pub h: char,
    pub dx: f32,
    /// 'T' / 'B'
    pub v: char,
    pub dy: f32,
}

impl Default for Spot {
    fn default() -> Self {
        Spot { h: 'C', dx: 0.0, v: 'T', dy: 24.0 }
    }
}

pub struct Model {
    /// a test copy: fake clock / sound / zones, never a window on the screen, never a Windows timer
    pub test: bool,
    pub clock: Clk,
    pub fake_clock: Option<FakeClock>,
    pub sound: Box<dyn SoundOs>,
    pub zones: Box<dyn ZoneOs>,
    pub timers: Vec<Timer>,
    seq: u32,
    /// the picked timer (`tmSel`)
    pub sel: u32,
    pub mode: Mode,
    pub places: Vec<PlaceItem>,
    /// Order 078: the tab's own Stopwatch / Countdown (0 = not made yet; the Countdown is made when first shown)
    own_sw: u32,
    own_cd: u32,
    pub spot: Spot,
    /// "Move": the on-screen timers can be dragged on the screen
    pub moving: bool,
    /// what finished since the page last looked: (timer id, toast text)
    pub ended: Vec<(u32, String)>,
    /// how many times the chime played (tests read it; the sound itself goes through `sound`)
    pub chimes: u32,
    /// the world clock of the current minute (Windows' zone rules are read once a minute, not per frame)
    world_cache: RefCell<Option<(u64, Vec<Place>, zones::WorldView)>>,
    /// the last "Add a place" search: (what was typed, the UTC minute, the results) - the page asks on every build
    found_cache: RefCell<Option<(String, u64, Vec<Found>)>>,
}

thread_local! {
    static MODEL: RefCell<Option<Model>> = const { RefCell::new(None) };
}

/// Use the model (made on first use: `test` = a test copy, `frozen` = the drawing's sample timers).
pub fn with<R>(test: bool, frozen: bool, f: impl FnOnce(&mut Model) -> R) -> R {
    MODEL.with(|m| {
        let mut m = m.borrow_mut();
        if m.is_none() {
            *m = Some(if test { Model::fake(frozen) } else { Model::real() });
        }
        f(m.as_mut().unwrap())
    })
}

/// Use the model if it exists (the overlay's timer, the alarm).
pub fn with_existing<R>(f: impl FnOnce(&mut Model) -> R) -> Option<R> {
    MODEL.with(|m| m.borrow_mut().as_mut().map(f))
}

/// Forget the model (tests).
#[cfg(test)]
pub fn drop_model() {
    MODEL.with(|m| *m.borrow_mut() = None);
}

impl Model {
    fn empty(test: bool, clock: Clk, fake_clock: Option<FakeClock>, sound: Box<dyn SoundOs>, zones: Box<dyn ZoneOs>) -> Model {
        Model {
            test,
            clock,
            fake_clock,
            sound,
            zones,
            timers: Vec::new(),
            seq: 0,
            sel: 0,
            mode: Mode::Sw,
            places: Vec::new(),
            own_sw: 0,
            own_cd: 0,
            spot: Spot::default(),
            moving: false,
            ended: Vec::new(),
            chimes: 0,
            world_cache: RefCell::new(None),
            found_cache: RefCell::new(None),
        }
    }

    /// The tab's own timer of a kind (made on first use).
    fn own_id(&mut self, kind: Kind) -> u32 {
        let have = if kind == Kind::Sw { self.own_sw } else { self.own_cd };
        if have != 0 && self.get(have).is_some() {
            return have;
        }
        let id = self.add(kind, if kind == Kind::Sw { "Stopwatch" } else { "Countdown" });
        if let Some(t) = self.get_mut(id) {
            t.own = true;
        }
        if kind == Kind::Sw {
            self.own_sw = id;
        } else {
            self.own_cd = id;
        }
        id
    }

    /// The real model: Windows' clock, sound and time zones; one stopwatch to start with.
    pub fn real() -> Model {
        #[cfg(windows)]
        let (sound, zones): (Box<dyn SoundOs>, Box<dyn ZoneOs>) = (Box::new(bu_timers::RealSound), Box::new(zones::RealZones));
        #[cfg(not(windows))]
        let (sound, zones): (Box<dyn SoundOs>, Box<dyn ZoneOs>) = (Box::new(FakeSound::default()), Box::new(zones::FakeZones::default()));
        let mut m = Model::empty(false, Clk(Arc::new(MonoClock::new())), None, sound, zones);
        m.sel = m.own_id(Kind::Sw);
        m
    }

    /// A test copy's model: a fake clock that only moves when a test moves it, a silent sound, the drawing's "now".
    /// `frozen` = the drawing's sample data: a stopwatch, "Pizza" (12:00, running, 8:41 left), "Ultimate" (1:30, on screen,
    /// key F8); places New York + Tokyo.
    pub fn fake(frozen: bool) -> Model {
        let fc = FakeClock::new();
        let mut m = Model::empty(true, Clk(Arc::new(fc.clone())), Some(fc.clone()), Box::new(FakeSound::default()), Box::new(zones::FakeZones::default()));
        m.sel = m.own_id(Kind::Sw);
        if frozen {
            let b = m.add(Kind::Cd, "Pizza");
            let c = m.add(Kind::Cd, "Ultimate");
            let _ = m.get_mut(b).map(|t| t.cd.type_time("12:00"));
            if let Some(t) = m.get_mut(c) {
                let _ = t.cd.type_time("1:30");
                t.screen = true;
                t.colour = TBCOL[1];
                t.key = Some("F8".into());
            }
            // Pizza started 199.5 s ago: 520.5 s left = "8:41" (the drawing's tmB.left = 521 shown as ceil)
            if let Some(t) = m.get_mut(b) {
                t.cd.start_pause();
            }
            fc.advance(Duration::from_millis(199_500));
            m.places = ["New York", "Tokyo"].iter().filter_map(|c| bu_timers::cities::search(c, 1).first().copied()).map(|place| PlaceItem { place, screen: false }).collect();
        }
        m
    }

    /// `newTimer`: the next id and colour.
    pub fn add(&mut self, kind: Kind, name: &str) -> u32 {
        self.seq += 1;
        let id = self.seq;
        let colour = TBCOL[((id - 1) % TBCOL.len() as u32) as usize];
        self.timers.push(Timer {
            id,
            kind,
            own: false,
            name: name.into(),
            colour,
            screen: false,
            key: None,
            sw: bu_timers::stopwatch::Stopwatch::new(self.clock.clone()),
            cd: bu_timers::countdown::Countdown::new(self.clock.clone()),
        });
        id
    }

    pub fn get(&self, id: u32) -> Option<&Timer> {
        self.timers.iter().find(|t| t.id == id)
    }
    pub fn get_mut(&mut self, id: u32) -> Option<&mut Timer> {
        self.timers.iter_mut().find(|t| t.id == id)
    }
    pub fn selected(&self) -> Option<&Timer> {
        self.get(self.sel)
    }

    /// The header switch (`tmSetMode`): Stopwatch / Countdown show the tab's OWN timer of that kind (made when first shown);
    /// a click on the kind already shown while a listed timer is picked brings the own one back.
    pub fn set_mode(&mut self, m: Mode) -> bool {
        let want = match m {
            Mode::Sw => Kind::Sw,
            Mode::Cd => Kind::Cd,
            Mode::Clk => {
                if m == self.mode {
                    return false;
                }
                self.mode = m;
                return true;
            }
        };
        let own = self.own_id(want);
        if m == self.mode {
            // the shown kind clicked again: a listed timer picked -> the own one comes back
            if self.sel == own {
                return false;
            }
            self.sel = own;
            return true;
        }
        // coming from another kind (or the world clock): a picked timer of this kind stays picked
        self.mode = m;
        if self.selected().map(|t| t.kind) != Some(want) {
            self.sel = own;
        }
        true
    }

    /// A list row clicked (`tmPick`).
    pub fn pick(&mut self, id: u32) {
        if let Some(k) = self.get(id).map(|t| t.kind) {
            self.sel = id;
            self.mode = if k == Kind::Sw { Mode::Sw } else { Mode::Cd };
        }
    }

    /// "New timer" (`tmAdd`): "Stopwatch 2", "Countdown 3"... of the current kind, picked. (The tab's own one is number 1.)
    pub fn add_new(&mut self) -> u32 {
        let kind = if self.mode == Mode::Cd { Kind::Cd } else { Kind::Sw };
        let n = self.timers.iter().filter(|t| t.kind == kind && !t.own).count() + 2;
        let id = self.add(kind, &format!("{} {}", if kind == Kind::Sw { "Stopwatch" } else { "Countdown" }, n));
        self.sel = id;
        id
    }

    /// The row's × (`tmRemove`): only a timer that was added; if it was the picked one the tab's own timer of its kind is
    /// shown again. Nothing is made new - removing the last one leaves the list empty.
    pub fn remove(&mut self, id: u32) {
        let Some(i) = self.timers.iter().position(|t| t.id == id && !t.own) else { return };
        let t = self.timers.remove(i);
        if t.id == self.sel {
            self.sel = self.own_id(t.kind);
            if self.mode != Mode::Clk {
                self.mode = if t.kind == Kind::Sw { Mode::Sw } else { Mode::Cd };
            }
        }
        if self.moving && !self.timers.iter().any(|t| t.screen) {
            self.moving = false;
        }
    }

    /// The timers of "Your timers": the ones that were added (not the tab's own two).
    pub fn listed(&self) -> impl Iterator<Item = &Timer> {
        self.timers.iter().filter(|t| !t.own)
    }

    /// Start / Stop / Pause / Resume / Again (`tmToggle`).
    pub fn toggle(&mut self, id: u32) {
        if let Some(t) = self.get_mut(id) {
            match t.kind {
                Kind::Sw => t.sw.start_stop(),
                Kind::Cd => t.cd.start_pause(),
            }
        }
    }

    pub fn reset(&mut self, id: u32) {
        if let Some(t) = self.get_mut(id) {
            match t.kind {
                Kind::Sw => t.sw.reset(),
                Kind::Cd => t.cd.reset(),
            }
        }
    }

    pub fn lap(&mut self, id: u32) {
        if let Some(t) = self.get_mut(id) {
            let _ = t.sw.lap();
        }
    }

    pub fn laps(&self, id: u32) -> Vec<LapRow> {
        self.get(id).map(|t| t.sw.lap_rows().into_iter().take(3).collect()).unwrap_or_default()
    }

    /// "On screen" switched; returns the toast the drawing shows for the big timer's chip.
    pub fn set_screen(&mut self, id: u32, on: bool) -> Option<String> {
        let t = self.get_mut(id)?;
        t.screen = on;
        let name = t.name.clone();
        if !on && self.moving && !self.timers.iter().any(|t| t.screen) {
            self.moving = false;
        }
        Some(if on { format!("{name} shows on your screen while it runs") } else { format!("{name} \u{b7} off your screen") })
    }

    pub fn rename(&mut self, id: u32, name: &str) {
        let n: String = name.trim().chars().take(NAME_MAX).collect();
        if let (Some(t), false) = (self.get_mut(id), n.is_empty()) {
            t.name = n;
        }
    }

    /// The big time typed (`cdApply`): a countdown's new time (5 = 5 min · 1:30 · 90s · 1h 20m); not a time = no change.
    pub fn type_time(&mut self, id: u32, text: &str) -> bool {
        match self.get_mut(id) {
            Some(t) if t.kind == Kind::Cd && !t.cd.is_running() => t.cd.type_time(text).is_ok(),
            _ => false,
        }
    }

    /// The key field: another timer already has this key -> its name.
    pub fn key_owner(&self, id: u32, key: &str) -> Option<String> {
        self.timers.iter().find(|t| t.id != id && t.key.as_deref() == Some(key)).map(|t| format!("Timer \u{b7} {}", t.name))
    }

    pub fn add_place(&mut self, p: Place) {
        if !self.places.iter().any(|x| x.place == p) {
            self.places.push(PlaceItem { place: p, screen: false });
            *self.found_cache.borrow_mut() = None;
        }
    }
    pub fn remove_place(&mut self, i: usize) {
        if i < self.places.len() {
            self.places.remove(i);
            *self.found_cache.borrow_mut() = None;
        }
    }
    pub fn toggle_place_screen(&mut self, i: usize) {
        if let Some(p) = self.places.get_mut(i) {
            p.screen = !p.screen;
        }
    }
    /// The "Add a place" box: the places that fit what was typed (2 letters or more) that are not in the list yet, with their
    /// time now - at most [`FOUND_MAX`]. The same typing in the same minute is not searched again.
    pub fn search_places(&self, q: &str) -> Vec<Found> {
        let minute = self.utc_minute();
        if let Some((cq, m, v)) = self.found_cache.borrow().as_ref() {
            if cq == q && *m == minute {
                return v.clone();
            }
        }
        let found: Vec<Found> = bu_timers::cities::search(q, FOUND_MAX + self.places.len())
            .into_iter()
            .filter(|p| !self.places.iter().any(|x| x.place == *p))
            .take(FOUND_MAX)
            .map(|place| {
                let (time, day) = zones::place_time(self.zones.as_ref(), &place).unwrap_or(("—".into(), 0));
                Found { place, time, day }
            })
            .collect();
        *self.found_cache.borrow_mut() = Some((q.to_string(), minute, found.clone()));
        found
    }

    /// Order 055: the UTC minute the world clock shows now (a change of it = the clock's text changed; no strings built).
    pub fn utc_minute(&self) -> u64 {
        self.zones.utc_now().as_secs() / 60
    }
    /// Order 055: how long until the world clock's next minute starts (the page wakes then, not every 250 ms).
    pub fn to_next_minute(&self) -> Duration {
        let now = self.zones.utc_now();
        let into = Duration::from_secs(now.as_secs() % 60) + Duration::from_nanos(u64::from(now.subsec_nanos()));
        Duration::from_secs(60) - into
    }

    pub fn world(&self) -> zones::WorldView {
        let minute = self.utc_minute();
        let list: Vec<Place> = self.places.iter().map(|p| p.place).collect();
        if let Some((m, c, w)) = self.world_cache.borrow().as_ref() {
            if *m == minute && *c == list {
                return w.clone();
            }
        }
        let w = zones::world_view(self.zones.as_ref(), &list);
        *self.world_cache.borrow_mut() = Some((minute, list, w.clone()));
        w
    }
    /// Tests: a new "now" or new zones take effect at once.
    #[cfg(test)]
    pub fn set_zones(&mut self, z: Box<dyn ZoneOs>) {
        self.zones = z;
        *self.world_cache.borrow_mut() = None;
        *self.found_cache.borrow_mut() = None;
    }

    /// Countdowns that reached zero: they finish once (the chime if their sound is on) -> `ended`.
    pub fn check(&mut self) -> bool {
        let mut any = false;
        let sound = &mut self.sound;
        for t in self.timers.iter_mut().filter(|t| t.kind == Kind::Cd) {
            if let Some(done) = t.cd.check(sound.as_mut()) {
                self.ended.push((t.id, format!("{} \u{b7} done", t.name)));
                self.chimes += done.chimed as u32;
                any = true;
            }
        }
        any
    }

    /// The sound preview button (`.pb` next to "Sound at the end").
    pub fn preview(&mut self, id: u32) -> bool {
        let sound = &mut self.sound;
        let played = match self.timers.iter().find(|t| t.id == id) {
            Some(t) if t.kind == Kind::Cd => t.cd.preview_sound(sound.as_mut()).unwrap_or(false),
            _ => false,
        };
        self.chimes += played as u32;
        played
    }

    pub fn any_running(&self) -> bool {
        self.timers.iter().any(|t| t.running())
    }

    /// How long until the next countdown ends (the wake-up when the menu is closed).
    pub fn next_end(&self) -> Option<Duration> {
        let now = self.clock.now();
        self.timers.iter().filter_map(|t| t.cd.deadline()).map(|d| d.saturating_sub(now)).min()
    }

    /// Order 055: how long until what the Timers page shows of the RUNNING timers looks different: (the digits - the next
    /// whole second of a countdown / of a stopwatch's seconds, exact; the countdown's line / ring - its next step of a
    /// quarter pixel (`RING_STEPS` over the 260 px line), exact, the caller keeps it off a faster rate than it can show).
    /// A stopwatch's hundredths are not in here (the page steps them at its own 30 Hz). None = no such timer runs.
    pub fn page_next_change(&self) -> (Option<Duration>, Option<Duration>) {
        let sec = Duration::from_secs(1);
        let (mut digits, mut ring): (Option<Duration>, Option<Duration>) = (None, None);
        let take = |slot: &mut Option<Duration>, d: Duration| *slot = Some(slot.map_or(d, |n| n.min(d)));
        for t in self.timers.iter().filter(|t| t.running()) {
            match t.kind {
                Kind::Sw => take(&mut digits, sec - Duration::from_nanos(u64::from(t.sw.elapsed().subsec_nanos()))),
                Kind::Cd => {
                    let left = t.cd.left();
                    if left.is_zero() {
                        // running at zero: the next `check` finishes it
                        take(&mut digits, Duration::ZERO);
                        continue;
                    }
                    // the digits are the left time rounded up: they change when it passes a whole second
                    let n = left.subsec_nanos();
                    take(&mut digits, if n == 0 { sec } else { Duration::from_nanos(u64::from(n)) });
                    let set = t.cd.set_time().as_secs_f64();
                    if set > 0.0 {
                        let at = left.as_secs_f64() / set * RING_STEPS;
                        // the step the ring sits in: it moves when the share drops below that step's lower edge
                        let edge = if at.fract() == 0.0 { at - 1.0 } else { at.floor() };
                        take(&mut ring, Duration::from_secs_f64(((at - edge) / RING_STEPS * set).max(0.0)));
                    }
                }
            }
        }
        (digits, ring)
    }

    /// Order 049: how long until a pill of `pills(preview)` looks different - a running timer's time text (whole seconds), a
    /// countdown's line moving a quarter of a device pixel (`line_px` = the line's full length in device px), a place's
    /// minute. The on-screen window sleeps until then. None = nothing on screen will change by itself.
    pub fn next_change(&self, preview: bool, line_px: f32) -> Option<Duration> {
        let sec = Duration::from_secs(1);
        let to_whole = |t: Duration| sec - Duration::from_nanos(u64::from(t.subsec_nanos()));
        let mut next: Option<Duration> = None;
        let mut take = |d: Duration| next = Some(next.map_or(d, |n| n.min(d)));
        for t in &self.timers {
            if !t.screen || !(t.busy() || preview || self.moving) || !t.running() {
                continue;
            }
            match t.kind {
                Kind::Sw => take(to_whole(t.sw.elapsed())),
                Kind::Cd => {
                    let left = t.cd.left();
                    // at zero nothing moves any more: the end alarm finishes it
                    if left.is_zero() {
                        continue;
                    }
                    // the text is the left time rounded up: it changes when the left time passes a whole second
                    let n = left.subsec_nanos();
                    take(if n == 0 { sec } else { Duration::from_nanos(u64::from(n)) });
                    if line_px > 0.0 {
                        take(t.cd.set_time().div_f64(f64::from(line_px) * 4.0).max(Duration::from_millis(1)));
                    }
                }
            }
        }
        if self.places.iter().any(|p| p.screen) {
            let now = self.zones.utc_now();
            let into = Duration::from_secs(now.as_secs() % 60) + Duration::from_nanos(u64::from(now.subsec_nanos()));
            take(Duration::from_secs(60) - into);
        }
        next
    }

    /// What the screen shows (`barsPaint`): every timer whose "On screen" is on while it is busy (or always while the
    /// Timers page is open = `preview`, or while moving), then every place whose "On screen" is on.
    pub fn pills(&self, preview: bool) -> Vec<Pill> {
        let mut v = Vec::new();
        for t in &self.timers {
            if !t.screen || !(t.busy() || preview || self.moving) {
                continue;
            }
            let (line, low) = match t.kind {
                Kind::Sw => (None, false),
                Kind::Cd => (Some(t.share_left()), t.cd.is_running() && t.cd.left() <= Duration::from_secs(3)),
            };
            v.push(Pill { key: format!("t{}", t.id), name: t.name.clone(), time: t.short_text(), colour: t.rgba(), line, low });
        }
        let w = self.world();
        for (p, (city, time, _)) in self.places.iter().zip(w.places) {
            if p.screen {
                v.push(Pill { key: format!("c{}/{}", city, p.place.land), name: city, time, colour: Rgba::hex(0x8e8e93), line: None, low: false });
            }
        }
        v
    }
}

