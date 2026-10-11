//! The Controller tab's sounds (Order 090, keyboard-v8.html): each button / trigger window gets the shared Sound part
//! (`pages::btnwin`: the pack's sound switch + your sound on top) with a note while Button sounds are off, and "Controller
//! settings" gets "Button sounds" (switch · Sound: Same as keyboard / any pack, with ▶ · Volume, its own). The settings live
//! with the Keyboard tab's (`keyboard::prefs`, one engine for every sound); a button is known to the sounds by its place
//! (`bu_rawin::padbtn`, Cross = A = SOUTH), so a controller the app can't read (no layout) plays the pack only.

use super::*;
use crate::pages::btnwin::{self, HearOf, SoundEd, BUTTON_WORDS};
use crate::pages::keyboard::{glue, prefs::Prefs};
use crate::ui::pieces::{group, slider, toggle};
use bu_keysound::{Dev, Pack, PackId};
use bu_rawin::{padbtn, PadSoundClass};

pub(crate) const K_PSND: Key = key("pad.snd");
pub(crate) const K_BSON: Key = key("pad.bson");
pub(crate) const K_BSPACK: Key = key("pad.bspack");
const K_BSPLAY: Key = key("pad.bsplay");
pub(crate) const K_BSVOL: Key = key("pad.bsvol");
pub(crate) const K_BSMENU: Key = key("pad.bsmenu");

/// The sounds' number of a part of the picture (None = a part without a sound of its own: sticks, gyro, Fn, back buttons).
pub(super) fn slot_of(id: Pid) -> Option<u16> {
    let n = match id {
        Pid::B(b) => match b {
            ButtonId::Cross => padbtn::SOUTH,
            ButtonId::Circle => padbtn::EAST,
            ButtonId::Square => padbtn::WEST,
            ButtonId::Triangle => padbtn::NORTH,
            ButtonId::DpadUp => padbtn::DPAD_UP,
            ButtonId::DpadDown => padbtn::DPAD_DOWN,
            ButtonId::DpadLeft => padbtn::DPAD_LEFT,
            ButtonId::DpadRight => padbtn::DPAD_RIGHT,
            ButtonId::L1 => padbtn::LB,
            ButtonId::R1 => padbtn::RB,
            ButtonId::L3 => padbtn::LS,
            ButtonId::R3 => padbtn::RS,
            ButtonId::Create => padbtn::BACK,
            ButtonId::Options => padbtn::START,
            ButtonId::Home => padbtn::HOME,
            ButtonId::Mute => padbtn::MIC,
            // the Edge's back buttons don't come through the controller's report the app reads
            ButtonId::BackLeftUpper | ButtonId::BackLeftLower | ButtonId::BackRightUpper | ButtonId::BackRightLower => return None,
        },
        Pid::Trig(Side::Left) => padbtn::LT,
        Pid::Trig(Side::Right) => padbtn::RT,
        Pid::Touch => padbtn::TOUCHPAD,
        _ => return None,
    };
    Some(u16::from(n))
}

/// What kind of sound a slot makes with the pack (a trigger clicks, a button plays the key sound).
fn class_of(slot: u16) -> Option<PadSoundClass> {
    Some(match slot {
        s if s == u16::from(padbtn::LT) => PadSoundClass::TriggerLeft,
        s if s == u16::from(padbtn::RT) => PadSoundClass::TriggerRight,
        _ => PadSoundClass::Button,
    })
}

#[derive(Default)]
pub(super) struct PadSounds {
    loaded: bool,
    test: bool,
    prefs: Prefs,
    snd: SoundEd,
    /// The Sound list of Button sounds is open (under this box).
    pop: Option<(f32, f32, f32, f32)>,
    press: Option<(Key, (f32, f32, f32, f32))>,
}

impl PadSounds {
    fn ensure(&mut self, test: bool) {
        if !self.loaded {
            self.test = test;
            self.prefs = crate::services::with(|s| Prefs::load(&s.store)).unwrap_or_default();
            self.loaded = true;
        }
    }

    fn save(&mut self) {
        let prefs = self.prefs.clone();
        let test = self.test;
        crate::services::with(|s| {
            prefs.save(&mut s.store);
            if !test {
                let dir = glue::packs_dir(s.store.folder());
                let _ = glue::apply(&prefs, &dir);
            }
        });
    }

    fn pack(&self) -> Pack {
        self.prefs.s.pad_pack.clone().unwrap_or_else(|| self.prefs.s.pack.clone())
    }

    fn hear(&self, ids: &[u16]) {
        if self.test {
            return;
        }
        let s = self.prefs.engine_settings();
        let pack = self.pack();
        let mouse = |_: u16| None;
        let pad = |slot: u16| class_of(slot);
        btnwin::hear(&HearOf { dev: Dev::Pad, pack: Some(&pack), volume: s.pad_volume, play_on: s.play_on, mouse: &mouse, pad: &pad }, &self.prefs.pad, ids);
    }

    /// The macros made on keys (the Keyboard tab's list) - a controller button can run one (Steam plays it).
    pub(super) fn macros(&self) -> Vec<bu_keysound::macros::Macro> {
        // (read fresh: the Keyboard tab edits them)
        crate::services::with(|s| Prefs::load(&s.store).macros).unwrap_or_default()
    }

    pub(super) fn reset_window(&mut self) {
        self.snd.reset();
        // the next window reads the settings again (the Keyboard tab may have changed them meanwhile)
        self.loaded = false;
    }

    /// The Sound part at the end of a button's window (with the note while Button sounds are off).
    pub(super) fn part(&mut self, cx: &mut Cx, slot: u16, test: bool) -> Vec<El> {
        self.ensure(test);
        let names = |f: &str| f.to_string();
        let mut out = vec![El::block().h(1.0).bg(crate::ui::HAIR()).margin(14.0, 0.0, 0.0, 0.0), self.snd.view(cx, K_PSND, &self.prefs.pad, &[slot], &BUTTON_WORDS, &names)];
        if !self.prefs.s.pad_on {
            out.push(group::gf("Button sounds are off \u{b7} Controller settings \u{203a} Button sounds"));
        }
        out
    }

    /// "Button sounds" in Controller settings: switch · Sound [Same as keyboard / a pack] ▶ · Volume (greyed while off).
    pub(super) fn settings_rows(&mut self, cx: &mut Cx, test: bool) -> El {
        self.ensure(test);
        let on = self.prefs.s.pad_on;
        let name = match &self.prefs.s.pad_pack {
            None => "Same as keyboard".to_string(),
            Some(p) => pack_name(p),
        };
        let v = self.prefs.s.pad_volume;
        // the controller's own rows' look (`.pdvd .prw`: 30 px rows, labels 150 px)
        const LW: f32 = 150.0;
        let head = look::prw("Button sounds", LW, vec![toggle::toggle(cx, K_BSON, on, false)], false, None);
        let rows = El::col()
            .items(AlignItems::STRETCH)
            .child(look::prw("Sound", LW, vec![dropdown::dropdown(cx, K_BSPACK, &name, Some(160.0)), btnwin::play_btn(cx, K_BSPLAY, "Play this sound")], false, None))
            .child(look::prw("Volume", LW, vec![slider::slider(cx, K_BSVOL, f32::from(v) / 100.0, 146.0, 20.0, slider::default()), slider::value_label(&format!("{v} %"))], false, None));
        let rows = if on { rows } else { rows.opacity(0.4).no_hit() };
        pieces::group::grp(vec![El::col().items(AlignItems::STRETCH).child(head).child(rows)]).pad(2.0, 12.0, 6.0, 12.0).margin(10.0, 0.0, 0.0, 0.0)
    }

    /// The packs Button sounds can play: Same as keyboard, then the keyboard's sounds.
    fn list(&self) -> Vec<(String, Option<Pack>)> {
        let mut v = vec![("Same as keyboard".to_string(), None)];
        for id in PackId::ALL {
            v.push((id.name().to_string(), Some(Pack::Builtin(id))));
        }
        let dir = crate::services::with(|s| glue::packs_dir(s.store.folder()));
        if let (false, Some(d)) = (self.test, dir) {
            for n in glue::made_list(&d) {
                v.push((n.clone(), Some(Pack::Made(n))));
            }
            for n in bu_keysound::import::installed(&d) {
                v.push((n.clone(), Some(Pack::Imported(n))));
            }
        }
        v
    }

    pub(super) fn popup(&mut self, cx: &mut Cx, slot: Option<u16>) -> Vec<El> {
        let mut out = Vec::new();
        if let Some(s) = slot {
            if let Some(m) = self.snd.popup(cx, K_PSND, &self.prefs.pad, &[s]) {
                out.push(m.z(12));
            }
        }
        if let Some(a) = self.pop {
            let cur = self.prefs.s.pad_pack.clone();
            let items: Vec<dropdown::Item> = self.list().into_iter().map(|(l, p)| dropdown::Item { label: l, checked: p == cur, disabled: false }).collect();
            out.push(dropdown::menu(cx, K_BSMENU, &items, a.0, a.1 + a.3 + 4.0, a.2.max(160.0)).z(12));
        }
        out
    }

    /// A press beside an open list closes it. True = one was open.
    pub(super) fn dismiss(&mut self) -> bool {
        self.pop.take().is_some() || self.snd.escape()
    }

    /// One event (`slot` = the open button window's, `dlg` = Controller settings is open). True = it was this part's.
    pub(super) fn event(&mut self, ev: &Ev, cx: &mut Cx, slot: Option<u16>, dlg: bool) -> bool {
        if !self.loaded {
            return false;
        }
        if let Some(s) = slot {
            let ids = [s];
            let o = self.snd.event(ev, cx, K_PSND, &mut self.prefs.pad, &ids, self.test, false);
            if o.save {
                self.save();
            } else if o.changed && !self.test {
                glue::engine().set_layers(Dev::Pad, self.prefs.pad.clone());
            }
            if o.play {
                self.hear(&ids);
            }
            if o.used {
                return true;
            }
        }
        if !dlg {
            return false;
        }
        match ev {
            Ev::Press(k, x, _, r) => {
                self.press = Some((*k, *r));
                if *k == K_BSVOL {
                    self.set_volume(slider::value_at(*r, *x));
                    return true;
                }
                false
            }
            Ev::Drag(k, x, _, r) if *k == K_BSVOL => {
                self.set_volume(slider::value_at(*r, *x));
                true
            }
            Ev::Release(k) if *k == K_BSVOL => {
                self.save();
                true
            }
            Ev::Click(k) => {
                let k = *k;
                if let Some(_a) = self.pop.take() {
                    let list = self.list();
                    if let Some(i) = (0..list.len()).find(|i| k == idx(K_BSMENU, *i)) {
                        self.prefs.s.pad_pack = list[i].1.clone();
                        self.save();
                        self.hear(&[u16::from(padbtn::SOUTH)]);
                    }
                    return true;
                }
                if k == K_BSON {
                    self.prefs.s.pad_on = !self.prefs.s.pad_on;
                    self.save();
                    cx.toast(if self.prefs.s.pad_on { "Button sounds on" } else { "Button sounds off \u{b7} the app stops listening to controllers" });
                    return true;
                }
                if k == K_BSPACK {
                    self.pop = Some(self.press.filter(|(pk, _)| *pk == k).map(|(_, r)| r).unwrap_or((200.0, 300.0, 160.0, 28.0)));
                    return true;
                }
                if k == K_BSPLAY {
                    self.hear(&[u16::from(padbtn::SOUTH)]);
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn set_volume(&mut self, t: f32) {
        self.prefs.s.pad_volume = (t.clamp(0.0, 1.0) * 100.0).round() as u8;
        if !self.test && self.prefs.s.pad_on {
            glue::engine().update(self.prefs.engine_settings());
        }
    }

    #[cfg(test)]
    pub(super) fn prefs(&self) -> &Prefs {
        &self.prefs
    }
}

fn pack_name(p: &Pack) -> String {
    match p {
        Pack::Builtin(id) => id.name().to_string(),
        Pack::Imported(n) | Pack::Made(n) => n.clone(),
    }
}

/// The keys tests drive.
#[cfg(test)]
pub(super) mod keys {
    pub(crate) use super::{K_BSMENU, K_BSON, K_BSPACK, K_BSVOL, K_PSND};
}
