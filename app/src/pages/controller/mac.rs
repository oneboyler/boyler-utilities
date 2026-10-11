//! A controller button's macro (Order 090, boss A_090_01): the app's macro steps <-> Steam's "extra commands" with fire start
//! delays (`bu_controller::layout::MacroCmd`). Steam plays it, so it works in games. Only what Steam can press is offered:
//! a key or combo (a tap), a key held while the button is held ("Key down"), a mouse click, a controller button, waits.
//! Typing a text and opening something are not Steam's (the editor doesn't offer them).

use bu_controller::layout::MacroCmd;
use bu_controller::{Action, MouseButton, PadButton, Press};
use bu_keysound::macros::{Macro, Step};

/// Steam's key token for a Windows virtual key (None = a key Steam's layouts don't name).
pub fn token(vk: u16) -> Option<String> {
    Some(match vk {
        0x41..=0x5A | 0x30..=0x39 => char::from(vk as u8).to_string(),
        0x70..=0x7B => format!("F{}", vk - 0x6F),
        0x20 => "SPACE".into(),
        0x0D => "RETURN".into(),
        0x1B => "ESCAPE".into(),
        0x09 => "TAB".into(),
        0x10 | 0xA0 => "LEFT_SHIFT".into(),
        0xA1 => "RIGHT_SHIFT".into(),
        0x11 | 0xA2 => "LEFT_CONTROL".into(),
        0xA3 => "RIGHT_CONTROL".into(),
        0x12 | 0xA4 => "LEFT_ALT".into(),
        0xA5 => "RIGHT_ALT".into(),
        0x08 => "BACKSPACE".into(),
        0x26 => "UP_ARROW".into(),
        0x28 => "DOWN_ARROW".into(),
        0x25 => "LEFT_ARROW".into(),
        0x27 => "RIGHT_ARROW".into(),
        0x24 => "HOME".into(),
        0x23 => "END".into(),
        0x21 => "PAGE_UP".into(),
        0x22 => "PAGE_DOWN".into(),
        0x2D => "INSERT".into(),
        0x2E => "DELETE".into(),
        _ => return None,
    })
}

/// The virtual key of a Steam key token (the reverse of [`token`]).
pub fn vk_of(token: &str) -> Option<u16> {
    let t = token.to_ascii_uppercase();
    if t.len() == 1 {
        let c = t.as_bytes()[0];
        if c.is_ascii_alphanumeric() {
            return Some(u16::from(c));
        }
    }
    if let Some(n) = t.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()) {
        if (1..=12).contains(&n) {
            return Some(0x6F + n);
        }
    }
    (0u16..0xFF).find(|v| token_eq(*v, &t))
}

fn token_eq(vk: u16, t: &str) -> bool {
    // only the canonical vk of each token (VK_SHIFT, not VK_LSHIFT)
    !matches!(vk, 0xA0 | 0xA2 | 0xA4) && token(vk).as_deref() == Some(t)
}

/// A controller button number (`bu_rawin::padbtn`) <-> Steam's pad button.
pub fn pad_button(b: u8) -> Option<PadButton> {
    use bu_rawin::padbtn as p;
    Some(match b {
        p::SOUTH => PadButton::Cross,
        p::EAST => PadButton::Circle,
        p::WEST => PadButton::Square,
        p::NORTH => PadButton::Triangle,
        p::LB => PadButton::L1,
        p::RB => PadButton::R1,
        p::LT => PadButton::L2,
        p::RT => PadButton::R2,
        p::LS => PadButton::L3,
        p::RS => PadButton::R3,
        p::DPAD_UP => PadButton::DpadUp,
        p::DPAD_DOWN => PadButton::DpadDown,
        p::DPAD_LEFT => PadButton::DpadLeft,
        p::DPAD_RIGHT => PadButton::DpadRight,
        p::BACK => PadButton::Create,
        p::START => PadButton::Options,
        _ => return None,
    })
}

fn pad_number(b: PadButton) -> u8 {
    (0..=18u8).find(|n| pad_button(*n) == Some(b)).unwrap_or(bu_rawin::padbtn::SOUTH)
}

fn mouse(b: u8) -> Option<MouseButton> {
    MouseButton::ALL.get(usize::from(b)).copied()
}

/// The macro's steps as Steam's commands; the steps Steam can't play are left out (their number comes back too).
pub fn to_cmds(m: &Macro) -> (Vec<MacroCmd>, usize) {
    let mut t: u32 = 0;
    let mut out = Vec::new();
    let mut skipped = 0;
    for s in &m.steps {
        let cmd = |press: Press, actions: Vec<Action>, t: u32| MacroCmd { press, actions, delay_ms: t };
        match s {
            Step::Wait(ms) => t = t.saturating_add(*ms).min(bu_controller::layout::MAX_MACRO_MS * 4),
            Step::Keys(vks) => {
                let a: Option<Vec<Action>> = vks.iter().map(|v| token(*v).map(Action::Key)).collect();
                match a {
                    Some(a) if !a.is_empty() => out.push(cmd(Press::Start, a, t)),
                    _ => skipped += 1,
                }
            }
            Step::Down(vk) => match token(*vk) {
                Some(k) => out.push(cmd(Press::Full, vec![Action::Key(k)], t)),
                None => skipped += 1,
            },
            Step::Click(b) => match mouse(*b) {
                Some(mb) => out.push(cmd(Press::Start, vec![Action::Mouse(mb)], t)),
                None => skipped += 1,
            },
            Step::Pad(b) => match pad_button(*b) {
                Some(pb) => out.push(cmd(Press::Start, vec![Action::Pad(pb)], t)),
                None => skipped += 1,
            },
            // a key is let go when the button is (Steam holds a "Key down" while the button is held); text / open: not Steam's
            Step::Up(_) | Step::Type(_) | Step::Open(_) => skipped += 1,
        }
    }
    // (the layout marks it as the app's macro itself: `Layout::set_macro`)
    (out, skipped)
}

/// Steam's commands as editable steps (None = something the editor can't show - a macro made in Steam itself).
pub fn from_cmds(cmds: &[MacroCmd]) -> Option<Vec<Step>> {
    let mut sorted: Vec<&MacroCmd> = cmds.iter().collect();
    sorted.sort_by_key(|c| c.delay_ms);
    let mut t = 0u32;
    let mut steps = Vec::new();
    for c in sorted {
        if c.delay_ms > t {
            steps.push(Step::Wait(c.delay_ms - t));
        }
        t = t.max(c.delay_ms);
        let step = match (c.press, c.actions.as_slice()) {
            (Press::Full, [Action::Key(k)]) => Step::Down(vk_of(k)?),
            (Press::Start, [Action::Mouse(mb)]) => Step::Click(MouseButton::ALL.iter().position(|x| x == mb)? as u8),
            (Press::Start, [Action::Pad(pb)]) => Step::Pad(pad_number(*pb)),
            (Press::Start, keys) if !keys.is_empty() => Step::Keys(keys.iter().map(|a| if let Action::Key(k) = a { vk_of(k) } else { None }).collect::<Option<Vec<u16>>>()?),
            _ => return None,
        };
        steps.push(step);
    }
    Some(steps)
}

/// The id the editor uses for a controller button's macro (it lives in Steam's layout, not in the app's list).
pub const PAD_MACRO_ID: &str = "pad";

/// The button's macro as the editor's macro (named for the window).
pub fn as_macro(steps: Vec<Step>) -> Macro {
    let mut m = Macro::new(PAD_MACRO_ID, "This button's macro");
    m.steps = steps;
    m
}

// ---- the page's side: Does → a macro, its steps under Does

use super::{Ctl, Open, Pid, AW};
use crate::pages::btnwin::{MacroEd, MacroFor};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{key, El, Key};
use bu_controller::{ButtonId, Change};

pub(super) const K_PMAC: Key = key("pad.mac");
const MAC: &str = "\u{1}mac:";

/// A "Macro" item of the Does list (`new` = New macro…, else a keyboard macro's id).
fn item(id: &str) -> Action {
    Action::Other(format!("{MAC}{id}"))
}

/// The macro id of a Does-list item, if it is one.
pub(super) fn item_id(a: &Action) -> Option<&str> {
    match a {
        Action::Other(s) => s.strip_prefix(MAC),
        _ => None,
    }
}

impl Open {
    /// The "Macro" section of a button's Does list (v8: New macro… + the macros made on keys).
    pub(super) fn macro_items(&self, at: Key, q: &str) -> Vec<(Option<&'static str>, Action)> {
        if !matches!(self.ctl.get(&at), Some(Ctl::Act { w: AW::Btn(_, Press::Full), .. })) {
            return vec![];
        }
        let s = q.trim().to_lowercase();
        let mut v: Vec<(String, Action)> = vec![("New macro\u{2026}".into(), item("new"))];
        v.extend(self.snd.macros().into_iter().map(|m| (m.name.clone(), item(&m.id))));
        v.into_iter()
            .filter(|(n, _)| s.is_empty() || n.to_lowercase().contains(&s) || "macro".contains(s.as_str()))
            .enumerate()
            .map(|(i, (_, a))| (if i == 0 { Some("Macro") } else { None }, a))
            .collect()
    }

    /// What a Does-list item says (a macro item: its name).
    pub(super) fn act_text(&self, a: &Action, xbox: bool) -> String {
        match item_id(a) {
            Some("new") => "New macro\u{2026}".into(),
            Some(id) => self.snd.macros().into_iter().find(|m| m.id == id).map(|m| m.name).unwrap_or_else(|| "Macro".into()),
            None => a.label(xbox),
        }
    }

    /// The button's macro: Some(Some) = editable, Some(None) = made in Steam (not editable here), None = no macro.
    pub(super) fn button_macro(&self, b: ButtonId) -> Option<Option<Macro>> {
        let cmds = self.view.as_ref().and_then(|v| v.buttons.iter().find(|x| x.id == b)).map(|v| v.macro_cmds.clone()).unwrap_or_default();
        if let Some((pb, m)) = &self.pad_mac {
            // the one being edited, while the layout has it (or it has no step Steam plays yet); an undo wins over it
            let mine = to_cmds(m).0;
            if *pb == b && (mine == cmds || (mine.is_empty() && cmds.is_empty())) {
                return Some(Some(m.clone()));
            }
        }
        if cmds.is_empty() {
            return None;
        }
        Some(from_cmds(&cmds).map(as_macro))
    }

    fn layout_has_macro(&self, b: ButtonId) -> bool {
        self.view.as_ref().and_then(|v| v.buttons.iter().find(|x| x.id == b)).is_some_and(|v| !v.macro_cmds.is_empty())
    }

    /// A Does-list macro item was picked for button `b`.
    pub(super) fn choose_macro(&mut self, b: ButtonId, id: &str, now: f64) {
        self.pmed.reset();
        self.pmed.opened(now);
        if id == "new" {
            // nothing is written until it has a step Steam plays
            self.pad_mac = Some((b, as_macro(Vec::new())));
            return;
        }
        let Some(m) = self.snd.macros().into_iter().find(|m| m.id == id) else { return };
        let (cmds, skipped) = to_cmds(&m);
        if cmds.is_empty() {
            self.say("Steam can\u{2019}t play this macro\u{2019}s steps (typing a text, opening something)", now);
            return;
        }
        if skipped > 0 {
            self.say(format!("Steam plays it without {skipped} of its steps (typing, opening, key up)"), now);
        }
        self.pad_mac = Some((b, as_macro(from_cmds(&cmds).unwrap_or_default())));
        self.write(vec![Change::ButtonMacro { button: b, cmds }], now);
    }

    /// The steps under Does while the button runs a macro (empty = it doesn't).
    pub(super) fn macro_rows(&mut self, cx: &mut Cx, b: ButtonId) -> Vec<El> {
        match self.button_macro(b) {
            Some(Some(m)) => {
                let mut out = self.pmed.view(cx, K_PMAC, &[m], Some(PAD_MACRO_ID), MacroFor::Pad, false, &[]);
                out.push(El::row().child(El::block().flex1()).child(MacroEd::delete_link(cx, K_PMAC)));
                out
            }
            Some(None) => vec![super::look::pnote("A macro made in Steam \u{b7} its commands are changed in Steam")],
            None => vec![],
        }
    }

    pub(super) fn macro_popup(&mut self, cx: &mut Cx) -> Option<El> {
        let Some(Pid::B(b)) = self.sel else { return None };
        let Some(Some(m)) = self.button_macro(b) else { return None };
        self.pmed.popup(cx, K_PMAC, &[m], Some(PAD_MACRO_ID), MacroFor::Pad)
    }

    /// Esc / a press beside: the editor's open list first. True = something closed.
    pub(super) fn macro_dismiss(&mut self) -> bool {
        self.pmed.escape()
    }

    /// The macro editor's events (a change is written to the layout at once). True = it was the editor's.
    pub(super) fn macro_event(&mut self, ev: &Ev, cx: &mut Cx) -> bool {
        let Some(Pid::B(b)) = self.sel else { return false };
        let Some(Some(m)) = self.button_macro(b) else { return false };
        let mut ms = vec![m];
        let o = self.pmed.event(ev, cx, K_PMAC, &mut ms, Some(PAD_MACRO_ID), MacroFor::Pad);
        if let Some(t) = o.toast.clone() {
            self.say(t, cx.now);
        }
        if o.chose.as_deref() == Some("") {
            // "Delete macro": the button does nothing until something else is picked (one never written: nothing to write)
            self.pad_mac = None;
            if !self.layout_has_macro(b) {
                return true;
            }
            self.write(vec![Change::ButtonAction { button: b, press: Press::Full, action: Action::Nothing }], cx.now);
            return true;
        }
        if o.save {
            if let Some(m) = ms.pop() {
                let (cmds, _) = to_cmds(&m);
                let had = self.layout_has_macro(b);
                self.pad_mac = Some((b, m));
                if !cmds.is_empty() {
                    self.write(vec![Change::ButtonMacro { button: b, cmds }], cx.now);
                } else if had {
                    // its last step Steam plays is gone: the button does nothing (the steps stay in the editor)
                    self.write(vec![Change::ButtonAction { button: b, press: Press::Full, action: Action::Nothing }], cx.now);
                }
            }
        }
        o.used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_through_steams_tokens() {
        for vk in [0x41u16, 0x5A, 0x30, 0x39, 0x70, 0x7B, 0x20, 0x0D, 0x10, 0x11, 0x12, 0x26, 0x2E] {
            let t = token(vk).unwrap();
            assert_eq!(vk_of(&t), Some(vk), "{t}");
        }
        assert_eq!(token(0x5B), None, "Win has no Steam token");
        assert_eq!(vk_of("NOPE"), None);
    }

    /// A macro becomes Steam's commands with the waits as fire start delays, and comes back as the same steps.
    #[test]
    fn a_macro_becomes_steams_commands_and_comes_back() {
        let m = as_macro(vec![Step::Down(0x10), Step::Keys(vec![0x11, 0x43]), Step::Wait(150), Step::Click(1), Step::Wait(50), Step::Pad(bu_rawin::padbtn::SOUTH)]);
        let (cmds, skipped) = to_cmds(&m);
        assert_eq!(skipped, 0);
        assert_eq!(cmds.len(), 4);
        assert_eq!(cmds[0], MacroCmd { press: Press::Full, actions: vec![Action::Key("LEFT_SHIFT".into())], delay_ms: 0 });
        assert_eq!(cmds[1].actions, vec![Action::Key("LEFT_CONTROL".into()), Action::Key("C".into())]);
        assert_eq!((cmds[2].press, cmds[2].delay_ms), (Press::Start, 150));
        assert_eq!(cmds[3].actions, vec![Action::Pad(PadButton::Cross)]);
        assert_eq!(cmds[3].delay_ms, 200);
        assert_eq!(from_cmds(&cmds).unwrap(), m.steps);
        // what Steam can't play is left out and counted
        let t = as_macro(vec![Step::Type("hi".into()), Step::Open("x".into()), Step::Up(0x41), Step::Keys(vec![0x5B, 0x52])]);
        assert_eq!(to_cmds(&t), (vec![], 4));
        // a one-step macro, and steps at the same time keep their order (no timing of their own is added)
        let one = as_macro(vec![Step::Keys(vec![0x41])]);
        let (c, _) = to_cmds(&one);
        assert_eq!(c[0].delay_ms, 0);
        assert_eq!(from_cmds(&c).unwrap(), one.steps);
        let same = as_macro(vec![Step::Keys(vec![0x43]), Step::Down(0x10)]);
        assert_eq!(from_cmds(&to_cmds(&same).0).unwrap(), same.steps);
        // a huge delay read from a file is no panic
        assert!(from_cmds(&[MacroCmd { press: Press::Start, actions: vec![Action::Key("A".into())], delay_ms: u32::MAX }]).is_some());
        // something the editor can't show: a macro made in Steam itself
        assert_eq!(from_cmds(&[MacroCmd { press: Press::Start, actions: vec![Action::Other("controller_action CHANGE_PRESET 2".into())], delay_ms: 5 }]), None);
    }
}
