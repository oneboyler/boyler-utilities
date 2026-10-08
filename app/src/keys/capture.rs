//! The "Keybind" key field while it listens (the drawing's `startCap` / `commitCap`), as a pure state machine.
//! - the app feeds it WM_KEYDOWN / WM_SYSKEYDOWN / …UP as [`KeyEvent`]s (vk, the extended-key flag = lParam bit 24, the
//!   repeat flag = bit 30, and the modifiers held right now from GetKeyState);
//! - a modifier shows "Ctrl + …" ([`Step::Listening`]); the first other key ends it with [`Step::Done`] (the app then
//!   calls `KeysManager::bind`; on a refusal it shows the red line and calls [`Capture::restart`] — the field keeps
//!   listening, as drawn); plain Esc = [`Step::Cancelled`]; plain Backspace = [`Step::Cleared`] (the app unbinds); with a
//!   modifier held they are keys like any other (the owner Oct 8: any real key);
//! - a numpad key = [`Step::Refused`] and it keeps listening: VK_NUMPAD*, * + - . /, NumLock, Clear, numpad Enter
//!   (VK_RETURN + extended) and, with NumLock off, the numpad's Ins / Del / Home / End / PgUp / PgDn / arrows (they come
//!   WITHOUT the extended flag; the real ones have it);
//! - modifiers only: when the last held modifier goes up and no other key came, it ends with [`Step::Done`] of
//!   `Combo::mods_only(every modifier that was held)` ("Ctrl + Shift"; the drawing commits modifiers on release);
//! - mouse buttons 3 / 4 / 5 (WM_MBUTTONDOWN / WM_XBUTTONDOWN while listening): [`Capture::mouse_down`] ("Mouse 4");
//! - PrtSc: Windows often sends only its key-up — a PrtSc up without a down counts as the press (as the drawing does).
//!   The app pauses the keys manager while a field listens (`KeysManager::pause` / `resume`).

use super::{is_nav_vk, is_numpad_vk, BindError, Combo, Mods, VK_BACK, VK_ESCAPE, VK_RETURN, VK_SNAPSHOT};

/// One key message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub vk: u16,
    /// lParam bit 24 (extended key).
    pub extended: bool,
    /// lParam bit 30 (the key was already down = auto-repeat).
    pub repeat: bool,
    /// The modifiers held now (GetKeyState of Ctrl / Alt / Shift / LWin|RWin).
    pub mods: Mods,
}

impl KeyEvent {
    pub fn new(vk: u16, mods: Mods) -> Self {
        KeyEvent { vk, extended: false, repeat: false, mods }
    }
    pub fn extended(mut self) -> Self {
        self.extended = true;
        self
    }
    pub fn repeat(mut self) -> Self {
        self.repeat = true;
        self
    }
}

/// What the field does after one key message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Still listening; show these modifiers ("Ctrl + …"), or "Press a key…" when empty.
    Listening { held: Mods },
    /// A combo was pressed: bind it.
    Done(Combo),
    /// Esc: stop listening, keep the old key.
    Cancelled,
    /// Backspace: stop listening, clear the key.
    Cleared,
    /// Refused here (numpad); still listening.
    Refused(BindError),
    /// Nothing to do (a repeat, an up of a normal key, or the capture already ended).
    Ignored,
}

/// One listening key field.
#[derive(Debug, Default)]
pub struct Capture {
    held: Mods,
    /// Every modifier held since none were (the modifier-only combo, committed when they are all up again).
    peak: Mods,
    ended: bool,
    /// The last non-modifier key that went down (to tell a lone PrtSc key-up).
    down: Option<u16>,
}

impl Capture {
    pub fn new() -> Self {
        Self::default()
    }

    /// The modifiers shown right now.
    pub fn held(&self) -> Mods {
        self.held
    }

    pub fn is_listening(&self) -> bool {
        !self.ended
    }

    /// After the manager refused a combo: keep listening from scratch (the drawing: `cap.held = ''`).
    pub fn restart(&mut self) {
        self.held = Mods::NONE;
        self.peak = Mods::NONE;
        self.ended = false;
        self.down = None;
    }

    pub fn key_down(&mut self, ev: KeyEvent) -> Step {
        if self.ended || ev.repeat {
            return Step::Ignored;
        }
        if let Some(m) = Mods::of_vk(ev.vk) {
            self.held = ev.mods.with(m);
            self.peak = self.peak.with(self.held);
            return Step::Listening { held: self.held };
        }
        match ev.vk {
            VK_ESCAPE if ev.mods.is_empty() => self.end(Step::Cancelled),
            VK_BACK if ev.mods.is_empty() => self.end(Step::Cleared),
            vk if is_numpad_vk(vk) || (vk == VK_RETURN && ev.extended) || (is_nav_vk(vk) && !ev.extended) => {
                self.held = Mods::NONE;
                self.peak = Mods::NONE;
                Step::Refused(BindError::Numpad)
            }
            vk if vk < 0x07 => Step::Ignored,
            vk => {
                self.down = Some(vk);
                self.end(Step::Done(Combo::new(ev.mods, vk)))
            }
        }
    }

    pub fn key_up(&mut self, ev: KeyEvent) -> Step {
        if self.ended {
            return Step::Ignored;
        }
        if let Some(m) = Mods::of_vk(ev.vk) {
            self.held = ev.mods.without(m);
            if self.held.is_empty() && !self.peak.is_empty() {
                // the last modifier went up and no other key came: modifiers only ("Ctrl + Shift")
                return self.end(Step::Done(Combo::mods_only(self.peak)));
            }
            return Step::Listening { held: self.held };
        }
        if ev.vk == VK_SNAPSHOT && self.down != Some(VK_SNAPSHOT) {
            return self.end(Step::Done(Combo::new(ev.mods, VK_SNAPSHOT)));
        }
        Step::Ignored
    }

    /// Mouse button 3 / 4 / 5 pressed while listening (`mods` = the modifiers held now) → "Mouse 4". Other buttons are
    /// left to the app (a click elsewhere): [`Step::Ignored`].
    pub fn mouse_down(&mut self, button: u8, mods: Mods) -> Step {
        if self.ended || !(3..=5).contains(&button) {
            return Step::Ignored;
        }
        self.end(Step::Done(Combo::mouse(mods, button)))
    }

    fn end(&mut self, step: Step) -> Step {
        self.ended = true;
        self.held = Mods::NONE;
        self.peak = Mods::NONE;
        step
    }
}
