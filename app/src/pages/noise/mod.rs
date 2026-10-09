//! The Noise tab (Order 062): brown / pink / white ... noise that plays in the background, made by ourselves (no recordings).
//! Engine: `crates/noise` (bu-noise); the always-on player is `glue.rs`, what is remembered `prefs.rs`.
//!
//! The noise is OFF until Play is pressed - every time, also after a restart of the app. It keeps playing with the menu closed
//! until Stop (or the sleep timer, or the tray's "Stop noise"). Playing it costs next to nothing: the loop is made once when a
//! noise is picked, played from memory through one output stream by one thread that wakes 14 times a second; stopped, there
//! is no thread, no stream and no loop in memory.

use crate::pages::{Background, Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, El, Key};
use crate::ui::pieces::button::{self, Kind as BKind};
use crate::ui::pieces::mitems::{self, It, Place, Row};
use crate::ui::pieces::{self, dropdown, group, seg, slider};
use crate::ui::{WIN_H, WIN_W};
use bu_noise::{Kind, Status, SLEEP_CHOICES};

pub mod glue;
pub mod prefs;
#[cfg(test)]
mod tests;

use prefs::Prefs;

const K_KIND: Key = key("nse.kind");
const K_PLAY: Key = key("nse.play");
const K_VOL: Key = key("nse.vol");
const K_SLEEP: Key = key("nse.sleep");
const K_MENU: Key = key("nse.menu");

#[derive(Default)]
pub struct Noise {
    prefs: Prefs,
    loaded: bool,
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

impl Noise {
    fn load(&mut self, env: &Env) {
        self.test = env.fake();
        self.prefs = crate::services::with(|s| Prefs::load(&s.store)).unwrap_or_default();
        self.loaded = true;
    }

    fn save(&self) {
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
        if self.test {
            self.fake = Status { playing: true, kind: Some(self.prefs.kind), volume: self.prefs.volume, sleep_left: self.prefs.sleep.map(|m| m * 60), ..Status::default() };
        } else {
            glue::engine().play(self.prefs.kind, self.prefs.volume, self.prefs.sleep);
        }
    }

    fn stop(&mut self) {
        if self.test {
            self.fake = Status::default();
        } else {
            glue::engine().stop();
        }
    }

    fn set_kind(&mut self, k: Kind) {
        self.prefs.kind = k;
        self.save();
        if self.test {
            if self.fake.playing {
                self.fake.kind = Some(k);
            }
        } else {
            glue::engine().set_kind(k);
        }
    }

    fn set_volume(&mut self, v: f32) {
        self.prefs.volume = (v.clamp(0.0, 1.0) * 100.0).round() as u8;
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

    fn view(&mut self, cx: &mut Cx) -> Vec<El> {
        let st = self.status();
        let on = st.playing && !st.stopping;
        let line = sleep_line(&st);
        self.shown = line.clone();
        let kind = dropdown::dropdown(cx, K_KIND, self.prefs.kind.name(), Some(130.0));
        // the same width for both words: the row never shifts when Play becomes Stop
        let play = if on { button::cbtn(cx, K_PLAY, "Stop", BKind::Ghost, false, false, 76.0) } else { button::cbtn(cx, K_PLAY, "Play", BKind::Primary, false, false, 76.0) };
        let v = self.prefs.volume;
        let vol = slider::slider(cx, K_VOL, f32::from(v) / 100.0, 150.0, 20.0, slider::default());
        let labels: Vec<String> = SLEEP_CHOICES.iter().map(|m| m.map_or("Off".to_string(), |m| format!("{m} min"))).collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let at = SLEEP_CHOICES.iter().position(|m| *m == self.prefs.sleep).unwrap_or(0);
        let sleep = seg::seg(cx, K_SLEEP, &labels, at, true);
        let rows = vec![
            group::row(true, vec![group::lbl("Noise", Some(self.prefs.kind.feel())), group::ctl(vec![kind, play])]),
            group::row(false, vec![group::lbl("Volume", None), group::ctl(vec![vol, slider::value_label(&format!("{v} %"))])]),
            group::row(false, vec![group::lbl("Sleep timer", line.as_deref()), group::ctl(vec![sleep])]),
        ];
        let mut out = vec![pieces::header(self.name(), None), group::gh("Background noise"), group::grp(rows)];
        if let Some(e) = &st.error {
            out.push(group::gf(&format!("The sound couldn't start: {e}")));
        }
        out
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
        let rows: Vec<Row> = Kind::ALL.iter().map(|k| Row::Item(It::tick(k.name(), *k == self.prefs.kind))).collect();
        let menu = mitems::menu(cx, K_MENU, &rows, Place::Under(a.0, a.1, a.2, a.3), a.2.max(150.0)).z(20);
        Some(El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(WIN_W, WIN_H).no_hit().child(menu))
    }
    fn popup_dismiss(&mut self) {
        if self.pop.take().is_some() {
            self.dismissed_at = Some(std::time::Instant::now());
        }
    }
    fn event(&mut self, ev: &Ev, _cx: &mut Cx) {
        match ev {
            Ev::Press(k, x, _, r) => {
                self.press = Some((*k, *r));
                self.press_dismissed = self.dismissed_at.take().is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(100));
                if *k == K_VOL {
                    self.set_volume(slider::value_at(*r, *x));
                }
            }
            Ev::Drag(k, x, _, r) if *k == K_VOL => self.set_volume(slider::value_at(*r, *x)),
            Ev::Release(k) if *k == K_VOL => self.save(),
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
                } else if self.pop.is_some() {
                    for (i, kind) in Kind::ALL.iter().enumerate() {
                        if k == idx(K_MENU, i) {
                            self.pop = None;
                            self.set_kind(*kind);
                            return;
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
        format!(
            "kind={} vol={} sleep={} playing={} stopping={} pop={}",
            self.prefs.kind.key(),
            self.prefs.volume,
            self.prefs.sleep.map_or("off".to_string(), |m| m.to_string()),
            st.playing,
            st.stopping,
            self.pop.is_some()
        )
    }
}
