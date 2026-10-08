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
use bu_timers::zones::{self, ZoneOs};
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
    pub city: String,
    pub screen: bool,
}

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
    pub spot: Spot,
    /// "Move": the on-screen timers can be dragged on the screen
    pub moving: bool,
    /// what finished since the page last looked: (timer id, toast text)
    pub ended: Vec<(u32, String)>,
    /// how many times the chime played (tests read it; the sound itself goes through `sound`)
    pub chimes: u32,
    /// the world clock of the current minute (Windows' zone rules are read once a minute, not per frame)
    world_cache: RefCell<Option<(u64, Vec<String>, zones::WorldView)>>,
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
            spot: Spot::default(),
            moving: false,
            ended: Vec::new(),
            chimes: 0,
            world_cache: RefCell::new(None),
        }
    }

    /// The real model: Windows' clock, sound and time zones; one stopwatch to start with.
    pub fn real() -> Model {
        #[cfg(windows)]
        let (sound, zones): (Box<dyn SoundOs>, Box<dyn ZoneOs>) = (Box::new(bu_timers::RealSound), Box::new(zones::RealZones));
        #[cfg(not(windows))]
        let (sound, zones): (Box<dyn SoundOs>, Box<dyn ZoneOs>) = (Box::new(FakeSound::default()), Box::new(zones::FakeZones::default()));
        let mut m = Model::empty(false, Clk(Arc::new(MonoClock::new())), None, sound, zones);
        let id = m.add(Kind::Sw, "Stopwatch");
        m.sel = id;
        m
    }

    /// A test copy's model: a fake clock that only moves when a test moves it, a silent sound, the drawing's "now".
    /// `frozen` = the drawing's sample data: a stopwatch, "Pizza" (12:00, running, 8:41 left), "Ultimate" (1:30, on screen,
    /// key F8); places New York + Tokyo.
    pub fn fake(frozen: bool) -> Model {
        let fc = FakeClock::new();
        let mut m = Model::empty(true, Clk(Arc::new(fc.clone())), Some(fc.clone()), Box::new(FakeSound::default()), Box::new(zones::FakeZones::default()));
        let a = m.add(Kind::Sw, "Stopwatch");
        m.sel = a;
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
            m.places = vec![PlaceItem { city: "New York".into(), screen: false }, PlaceItem { city: "Tokyo".into(), screen: false }];
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

    /// The header switch (`tmSetMode`): Stopwatch / Countdown pick the first timer of that kind (or make one).
    pub fn set_mode(&mut self, m: Mode) -> bool {
        if m == self.mode {
            return false;
        }
        self.mode = m;
        let want = match m {
            Mode::Sw => Kind::Sw,
            Mode::Cd => Kind::Cd,
            Mode::Clk => return true,
        };
        if self.selected().map(|t| t.kind) != Some(want) {
            self.sel = match self.timers.iter().find(|t| t.kind == want) {
                Some(t) => t.id,
                None => self.add(want, if want == Kind::Sw { "Stopwatch" } else { "Countdown" }),
            };
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

    /// "New timer" (`tmAdd`): "Stopwatch 2", "Countdown 3"... of the current kind, picked.
    pub fn add_new(&mut self) -> u32 {
        let kind = if self.mode == Mode::Cd { Kind::Cd } else { Kind::Sw };
        let n = self.timers.iter().filter(|t| t.kind == kind).count() + 1;
        let id = self.add(kind, &format!("{} {}", if kind == Kind::Sw { "Stopwatch" } else { "Countdown" }, n));
        self.sel = id;
        id
    }

    /// The row's × (`tmRemove`): the pick moves to another timer of the same kind (or any, or a new one).
    pub fn remove(&mut self, id: u32) {
        let Some(i) = self.timers.iter().position(|t| t.id == id) else { return };
        let t = self.timers.remove(i);
        if t.id == self.sel {
            self.sel = match self.timers.iter().find(|x| x.kind == t.kind).or(self.timers.first()) {
                Some(x) => x.id,
                None => self.add(t.kind, if t.kind == Kind::Sw { "Stopwatch" } else { "Countdown" }),
            };
            if self.mode != Mode::Clk {
                self.mode = if self.get(self.sel).map(|x| x.kind) == Some(Kind::Sw) { Mode::Sw } else { Mode::Cd };
            }
        }
        if self.moving && !self.timers.iter().any(|t| t.screen) {
            self.moving = false;
        }
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

    pub fn add_place(&mut self, city: &str) {
        if zones::place(city).is_some() && !self.places.iter().any(|p| p.city == city) {
            self.places.push(PlaceItem { city: city.into(), screen: false });
        }
    }
    pub fn remove_place(&mut self, city: &str) {
        self.places.retain(|p| p.city != city);
    }
    pub fn toggle_place_screen(&mut self, city: &str) {
        if let Some(p) = self.places.iter_mut().find(|p| p.city == city) {
            p.screen = !p.screen;
        }
    }
    /// The places not in the list yet (the "Add a place" menu): (city, "City · Country").
    pub fn places_left(&self) -> Vec<(&'static str, String)> {
        zones::PLACES.iter().filter(|p| !self.places.iter().any(|x| x.city == p.city)).map(|p| (p.city, format!("{} \u{b7} {}", p.city, p.land))).collect()
    }

    pub fn world(&self) -> zones::WorldView {
        let minute = self.zones.utc_now().as_secs() / 60;
        let cities: Vec<String> = self.places.iter().map(|p| p.city.clone()).collect();
        if let Some((m, c, w)) = self.world_cache.borrow().as_ref() {
            if *m == minute && *c == cities {
                return w.clone();
            }
        }
        let refs: Vec<&str> = cities.iter().map(|c| c.as_str()).collect();
        let w = zones::world_view(self.zones.as_ref(), &refs);
        *self.world_cache.borrow_mut() = Some((minute, cities, w.clone()));
        w
    }
    /// Tests: a new "now" or new zones take effect at once.
    #[cfg(test)]
    pub fn set_zones(&mut self, z: Box<dyn ZoneOs>) {
        self.zones = z;
        *self.world_cache.borrow_mut() = None;
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
                v.push(Pill { key: format!("c{city}"), name: city, time, colour: Rgba::hex(0x8e8e93), line: None, low: false });
            }
        }
        v
    }
}

