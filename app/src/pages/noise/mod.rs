//! The Noise tab (Order 062): brown / pink / white ... noise that plays in the background, made by ourselves (no recordings).
//! Engine: `crates/noise` (bu-noise); the always-on player is `glue.rs`, what is remembered `prefs.rs`.
//!
//! The noise is OFF until Play is pressed - every time, also after a restart of the app. It keeps playing with the menu closed
//! until Stop (or the sleep timer, or the tray's "Stop noise"). Playing it costs next to nothing: the loop is made once when a
//! noise is picked, played from memory through one output stream by one thread that wakes 14 times a second; stopped, there
//! is no thread, no stream and no loop in memory.
//!
//! Order 080: "Custom" in the list = your own mix of three sliders (Tone, Rumble, Waves) that change the sound while it plays;
//! "Save as my sound" names the mix and puts it in the list under "My sounds" (Remove takes it out again).

use crate::pages::{Background, Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, El, Key};
use crate::ui::pieces::button::{self, Kind as BKind};
use crate::ui::pieces::mitems::{self, It, Place, Row};
use crate::ui::pieces::nbox::{self, Filter};
use crate::ui::pieces::{self, dropdown, group, seg, slider, tinput};
use crate::ui::{WIN_H, WIN_W};
use bu_noise::{Kind, Mix, Sound, Status, SLEEP_CHOICES};

pub mod glue;
pub mod prefs;
#[cfg(test)]
mod tests;

use prefs::{Pick, Prefs};

const K_KIND: Key = key("nse.kind");
const K_PLAY: Key = key("nse.play");
const K_VOL: Key = key("nse.vol");
const K_SLEEP: Key = key("nse.sleep");
const K_MENU: Key = key("nse.menu");
const K_TONE: Key = key("nse.tone");
const K_RUMBLE: Key = key("nse.rumble");
const K_WAVES: Key = key("nse.waves");
const K_SAVE: Key = key("nse.save");
const K_NAME: Key = key("nse.name");
const K_NSAVE: Key = key("nse.nsave");
const K_NCANCEL: Key = key("nse.ncancel");
const K_REMOVE: Key = key("nse.remove");

/// The name given when Save is pressed with nothing typed.
const DEFAULT_NAME: &str = "My sound";

/// The three sliders of the Custom mix.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Tone,
    Rumble,
    Waves,
}

impl Part {
    const ALL: [Part; 3] = [Part::Tone, Part::Rumble, Part::Waves];

    fn key(self) -> Key {
        match self {
            Part::Tone => K_TONE,
            Part::Rumble => K_RUMBLE,
            Part::Waves => K_WAVES,
        }
    }
    fn title(self) -> &'static str {
        match self {
            Part::Tone => "Tone",
            Part::Rumble => "Rumble",
            Part::Waves => "Waves",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Part::Tone => "Deep to bright",
            Part::Rumble => "How much low bass",
            Part::Waves => "Steady to a slow sea-like swell",
        }
    }
    fn get(self, m: Mix) -> u8 {
        match self {
            Part::Tone => m.tone,
            Part::Rumble => m.rumble,
            Part::Waves => m.waves,
        }
    }
    fn set(self, m: &mut Mix, v: u8) {
        match self {
            Part::Tone => m.tone = v,
            Part::Rumble => m.rumble = v,
            Part::Waves => m.waves = v,
        }
    }
}

/// One line of the noise list.
#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Item(Pick),
    Sep,
    Section(&'static str),
}

#[derive(Default)]
pub struct Noise {
    prefs: Prefs,
    loaded: bool,
    /// A value changed that is not written yet (a slider is dragged: it is written when the button comes up or the tab closes).
    dirty: bool,
    /// The noise list is open (its anchor = the button's box, from the press before the click).
    pop: Option<(f32, f32, f32, f32)>,
    press: Option<(Key, (f32, f32, f32, f32))>,
    /// A test copy: nothing is played, a stand-in keeps what the page asked.
    test: bool,
    fake: Status,
    /// The sleep line as last painted (the page repaints when it changes).
    shown: Option<String>,
    /// The list was closed a moment ago by the press that is now being clicked (a press outside a popup closes it and the click
    /// still arrives: on the button that must not open the list again).
    dismissed_at: Option<std::time::Instant>,
    press_dismissed: bool,
    /// "Save as my sound" was pressed: the name being typed.
    naming: Option<String>,
}

/// "Stops in 27 min" for a running sleep timer.
fn sleep_line(st: &Status) -> Option<String> {
    if !st.playing {
        return None;
    }
    if st.stopping {
        return Some("Fading out".into());
    }
    let s = st.sleep_left?;
    Some(if s < 60 { "Stops in under a minute".into() } else { format!("Stops in {} min", s.div_ceil(60)) })
}

/// "Tone 35 · Rumble 30 · Waves 0".
fn mix_line(m: Mix) -> String {
    format!("Tone {} · Rumble {} · Waves {}", m.tone, m.rumble, m.waves)
}

impl Noise {
    fn load(&mut self, env: &Env) {
        self.test = env.fake();
        self.prefs = crate::services::with(|s| Prefs::load(&s.store)).unwrap_or_default();
        self.loaded = true;
    }

    fn save(&mut self) {
        self.dirty = false;
        let p = self.prefs.clone();
        crate::services::with(|s| p.save(&mut s.store));
    }

    fn status(&self) -> Status {
        if self.test {
            self.fake.clone()
        } else {
            glue::engine().status()
        }
    }

    fn play(&mut self) {
        let sound = self.prefs.sound();
        if self.test {
            self.fake = Status { playing: true, sound: Some(sound), volume: self.prefs.volume, sleep_left: self.prefs.sleep.map(|m| m * 60), ..Status::default() };
        } else {
            glue::engine().play(sound, self.prefs.volume, self.prefs.sleep);
        }
    }

    fn stop(&mut self) {
        if self.test {
            self.fake = Status::default();
        } else {
            glue::engine().stop();
        }
    }

    /// What is playing / will play is now `prefs.sound()`.
    fn push_sound(&mut self) {
        let s: Sound = self.prefs.sound();
        if self.test {
            if self.fake.playing {
                self.fake.sound = Some(s);
            }
        } else {
            glue::engine().set_sound(s);
        }
    }

    fn set_pick(&mut self, p: Pick) {
        self.naming = None;
        self.prefs.pick = p;
        self.save();
        self.push_sound();
    }

    /// A Custom slider moved: the sound follows at once.
    fn set_part(&mut self, part: Part, v: f32) {
        let v = (v.clamp(0.0, 1.0) * 100.0).round() as u8;
        let mut m = self.prefs.custom;
        part.set(&mut m, v);
        if m != self.prefs.custom {
            self.prefs.custom = m;
            self.dirty = true;
            self.push_sound();
        }
    }

    fn set_volume(&mut self, v: f32) {
        self.prefs.volume = (v.clamp(0.0, 1.0) * 100.0).round() as u8;
        self.dirty = true;
        if self.test {
            self.fake.volume = self.prefs.volume;
        } else {
            glue::engine().set_volume(self.prefs.volume);
        }
    }

    fn set_sleep(&mut self, m: Option<u32>) {
        self.prefs.sleep = m;
        self.save();
        if self.test {
            if self.fake.playing {
                self.fake.sleep_left = m.map(|m| m * 60);
            }
        } else {
            glue::engine().set_sleep(m);
        }
    }

    /// The name that was typed is the sound's: it is kept and picked (it sounds the same, so nothing changes in the speakers).
    fn commit_name(&mut self) {
        let Some(text) = self.naming.take() else { return };
        let name = if prefs::clean_name(&text).is_empty() { DEFAULT_NAME.to_string() } else { text };
        if self.prefs.save_mine(&name).is_some() {
            self.save();
            self.push_sound();
        }
    }

    fn remove_mine(&mut self, name: &str) {
        if self.prefs.remove_mine(name) {
            self.save();
            self.push_sound();
        }
    }

    /// The lines of the list: the six noises, Custom, then the saved sounds.
    fn entries(&self) -> Vec<Entry> {
        let mut v: Vec<Entry> = Kind::ALL.iter().map(|k| Entry::Item(Pick::Preset(*k))).collect();
        v.push(Entry::Sep);
        v.push(Entry::Item(Pick::Custom));
        if !self.prefs.mine.is_empty() {
            v.push(Entry::Section("My sounds"));
            v.extend(self.prefs.mine.iter().map(|m| Entry::Item(Pick::Mine(m.name.clone()))));
        }
        v
    }

    /// The text under "Noise": what the pick sounds like.
    fn feel(&self) -> String {
        match &self.prefs.pick {
            Pick::Preset(k) => k.feel().to_string(),
            Pick::Custom => "Your own mix, set with the sliders below".to_string(),
            Pick::Mine(_) => "Your own sound".to_string(),
        }
    }

    fn view(&mut self, cx: &mut Cx) -> Vec<El> {
        let st = self.status();
        let on = st.playing && !st.stopping;
        let line = sleep_line(&st);
        self.shown = line.clone();
        let kind = dropdown::dropdown(cx, K_KIND, &self.prefs.pick.name(), Some(130.0));
        // the same width for both words: the row never shifts when Play becomes Stop
        let play = if on { button::cbtn(cx, K_PLAY, "Stop", BKind::Ghost, false, false, 76.0) } else { button::cbtn(cx, K_PLAY, "Play", BKind::Primary, false, false, 76.0) };
        let v = self.prefs.volume;
        let vol = slider::slider(cx, K_VOL, f32::from(v) / 100.0, 150.0, 20.0, slider::default());
        let labels: Vec<String> = SLEEP_CHOICES.iter().map(|m| m.map_or("Off".to_string(), |m| format!("{m} min"))).collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let at = SLEEP_CHOICES.iter().position(|m| *m == self.prefs.sleep).unwrap_or(0);
        let sleep = seg::seg(cx, K_SLEEP, &labels, at, true);
        let feel = self.feel();
        let rows = vec![
            group::row(true, vec![group::lbl("Noise", Some(&feel)), group::ctl(vec![kind, play])]),
            group::row(false, vec![group::lbl("Volume", None), group::ctl(vec![vol, slider::value_label(&format!("{v} %"))])]),
            group::row(false, vec![group::lbl("Sleep timer", line.as_deref()), group::ctl(vec![sleep])]),
        ];
        let mut out = vec![pieces::header(self.name(), None), group::gh("Background noise"), group::grp(rows)];
        match self.prefs.pick.clone() {
            Pick::Custom => {
                out.push(group::gh("Your mix"));
                out.push(group::grp(self.mix_rows(cx)));
            }
            Pick::Mine(name) => {
                let m = self.prefs.sound();
                let summary = if let Sound::Mix(m) = m { mix_line(m) } else { String::new() };
                let remove = button::cbtn(cx, K_REMOVE, "Remove", BKind::Quiet, false, false, 76.0);
                out.push(group::gh("Your sound"));
                out.push(group::grp(vec![group::row(true, vec![group::lbl(&name, Some(&summary)), group::ctl(vec![remove])])]));
            }
            Pick::Preset(_) => {}
        }
        if let Some(e) = &st.error {
            out.push(group::gf(&format!("The sound couldn't start: {e}")));
        }
        out
    }

    /// The Custom group: three sliders and the way to keep the mix.
    fn mix_rows(&mut self, cx: &mut Cx) -> Vec<El> {
        let mut rows = Vec::new();
        for (i, part) in Part::ALL.into_iter().enumerate() {
            let v = part.get(self.prefs.custom);
            let s = slider::slider(cx, part.key(), f32::from(v) / 100.0, 150.0, 20.0, slider::default());
            rows.push(group::row(i == 0, vec![group::lbl(part.title(), Some(part.hint())), group::ctl(vec![s, slider::value_label(&v.to_string())])]));
        }
        let full = self.prefs.mine.len() >= prefs::MAX_MINE;
        let save_row = if let Some(text) = self.naming.clone() {
            let field = tinput::kdin(cx, K_NAME, &text, DEFAULT_NAME, 150.0);
            let ok = button::cbtn(cx, K_NSAVE, "Save", BKind::Primary, false, false, 64.0);
            let no = button::cbtn(cx, K_NCANCEL, "Cancel", BKind::Ghost, false, false, 64.0);
            group::row(false, vec![group::lbl("Name it", Some("Enter saves, Esc cancels")), group::ctl(vec![field, ok, no])])
        } else {
            let note = if full { format!("{} sounds kept - remove one first", prefs::MAX_MINE) } else { "Keeps this mix in the list, next to Brown".to_string() };
            let b = button::cbtn(cx, K_SAVE, "Save as my sound", BKind::Ghost, false, full, 76.0);
            group::row(false, vec![group::lbl("Keep it", Some(&note)), group::ctl(vec![b])])
        };
        rows.push(save_row);
        rows
    }

    fn slider_at(&mut self, k: Key, x: f32, r: (f32, f32, f32, f32)) {
        if k == K_VOL {
            self.set_volume(slider::value_at(r, x));
        } else if let Some(p) = Part::ALL.into_iter().find(|p| p.key() == k) {
            self.set_part(p, slider::value_at(r, x));
        }
    }
}

impl Page for Noise {
    fn id(&self) -> &'static str {
        "nse"
    }
    fn name(&self) -> &'static str {
        "Noise"
    }
    fn icon(&self) -> &'static str {
        "noise"
    }
    fn open(&mut self, env: &Env, _now: f64) {
        let fake = std::mem::take(&mut self.fake);
        *self = Noise::default();
        self.fake = fake;
        self.load(env);
    }
    fn close(&mut self) {
        if self.dirty {
            self.save();
        }
        // the player goes on (it is the switch's, not the tab's); only what the page held is dropped
        let fake = std::mem::take(&mut self.fake);
        *self = Noise::default();
        self.fake = fake;
    }
    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        if !self.loaded {
            self.load(&Env { test: true, ..Env::default() });
        }
        self.view(cx)
    }
    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let a = self.pop?;
        let entries = self.entries();
        let names: Vec<String> = entries.iter().map(|e| if let Entry::Item(p) = e { p.name() } else { String::new() }).collect();
        let rows: Vec<Row> = entries
            .iter()
            .zip(&names)
            .map(|(e, n)| match e {
                Entry::Item(p) => Row::Item(It::tick(n, *p == self.prefs.pick)),
                Entry::Sep => Row::Sep,
                Entry::Section(t) => Row::Section(t),
            })
            .collect();
        let menu = mitems::menu(cx, K_MENU, &rows, Place::Under(a.0, a.1, a.2, a.3), a.2.max(150.0)).z(20);
        Some(El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(WIN_W, WIN_H).no_hit().child(menu))
    }
    fn popup_dismiss(&mut self) {
        if self.pop.take().is_some() {
            self.dismissed_at = Some(std::time::Instant::now());
        }
    }
    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        match ev {
            Ev::Press(k, x, _, r) => {
                self.press = Some((*k, *r));
                self.press_dismissed = self.dismissed_at.take().is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(100));
                self.slider_at(*k, *x, *r);
            }
            Ev::Drag(k, x, _, r) => self.slider_at(*k, *x, *r),
            Ev::Release(k) if *k == K_VOL || Part::ALL.iter().any(|p| p.key() == *k) => {
                if self.dirty {
                    self.save();
                }
            }
            Ev::Char(k, c) if *k == K_NAME => {
                if let Some(t) = self.naming.as_mut() {
                    nbox::type_char(t, &mut false, *c, prefs::MAX_NAME, Filter::Any);
                }
            }
            Ev::Key(k, vk) if *k == K_NAME => {
                match *vk {
                    0x0D => {
                        self.commit_name();
                        cx.focus(None);
                    }
                    0x1B => {
                        // Esc cancels the name, it does not close the menu
                        cx.used = true;
                        self.naming = None;
                    }
                    vk => {
                        if let Some(t) = self.naming.as_mut() {
                            nbox::edit_key(t, &mut false, vk);
                        }
                    }
                }
            }
            Ev::Click(k) => {
                let k = *k;
                if k == K_KIND && std::mem::take(&mut self.press_dismissed) {
                    // the press that closed the list: the click on the button does not open it again
                } else if k == K_KIND {
                    let a = self.press.filter(|(pk, _)| *pk == k).map(|(_, r)| r).unwrap_or((0.0, 0.0, 0.0, 0.0));
                    self.pop = if self.pop.is_some() { None } else { Some(a) };
                } else if k == K_PLAY {
                    let st = self.status();
                    if st.playing && !st.stopping {
                        self.stop();
                    } else {
                        self.play();
                    }
                } else if k == K_SAVE {
                    if self.prefs.mine.len() < prefs::MAX_MINE {
                        self.naming = Some(String::new());
                        cx.focus(Some(K_NAME));
                    }
                } else if k == K_NSAVE {
                    self.commit_name();
                } else if k == K_NCANCEL {
                    self.naming = None;
                } else if k == K_REMOVE {
                    if let Pick::Mine(n) = self.prefs.pick.clone() {
                        self.remove_mine(&n);
                    }
                } else if self.pop.is_some() {
                    let entries = self.entries();
                    for (i, e) in entries.iter().enumerate() {
                        if let Entry::Item(p) = e {
                            if k == idx(K_MENU, i) {
                                self.pop = None;
                                self.set_pick(p.clone());
                                return;
                            }
                        }
                    }
                } else {
                    for (i, m) in SLEEP_CHOICES.iter().enumerate() {
                        if k == idx(K_SLEEP, i) {
                            self.set_sleep(*m);
                            return;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    /// The sleep line counts down: ask again when its minute changes (nothing playing = None: no frames, no wake-ups).
    fn tick(&mut self, _now: f64) -> bool {
        sleep_line(&self.status()) != self.shown
    }
    fn wake_at(&self, now: f64) -> Option<f64> {
        let st = self.status();
        if !st.playing || st.stopping {
            return None;
        }
        let s = st.sleep_left?;
        // the line changes when the seconds left cross a whole minute (or reach 0)
        let to_edge = if s == 60 { 1 } else if s < 60 { s } else { s - (s.div_ceil(60) - 1) * 60 };
        Some(now + f64::from(to_edge) * 1000.0 + 150.0)
    }
    fn start(&self, s: &mut crate::services::Services) {
        if s.test {
            return; // test copies never start a player
        }
        glue::start();
    }
    fn background(&self, env: &Env) -> Option<Box<dyn Background>> {
        if env.test {
            return None;
        }
        Some(Box::new(glue::Bg))
    }
    fn describe(&self) -> String {
        let st = self.status();
        let c = self.prefs.custom;
        format!(
            "kind={} custom={}/{}/{} mine={} vol={} sleep={} playing={} stopping={} pop={} naming={}",
            self.prefs.pick.key(),
            c.tone,
            c.rumble,
            c.waves,
            self.prefs.mine.len(),
            self.prefs.volume,
            self.prefs.sleep.map_or("off".to_string(), |m| m.to_string()),
            st.playing,
            st.stopping,
            self.pop.is_some(),
            self.naming.is_some()
        )
    }
}

