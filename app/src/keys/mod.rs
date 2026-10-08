//! Keys manager (Order 014 change 6): the ONLY place in the app that registers keys.
//! - feature crates / pages expose [`Action`]s (id + display name + owner page); the manager maps one key combo to each
//!   ([`Combo`]: Ctrl / Alt / Shift / Win + one key, OR modifiers only ("Ctrl + Shift", fires on release), OR modifiers +
//!   mouse button 3 / 4 / 5 ("Mouse 4", fires on button down) — the drawing's key field allows all three);
//! - one key is never used twice: [`KeysManager::bind`] refuses a combo another action has and says which one
//!   ([`BindError::UsedBy`] → "Already used by Mic mute", the drawing's red line);
//! - no numpad keys (for keyboards without a numpad): VK_NUMPAD0-9, * + separator - . /, NumLock, Clear; numpad Enter
//!   and the NumLock-off numpad arrows / Home / … are caught by the key field ([`capture`]) through the extended-key flag;
//! - Croatian QWERTZ: key names come from the keyboard layout (č ć ž š đ show as Č Ć Ž Š Đ), never a US table;
//! - ANY real key can be bound, typing keys too (a plain letter, digit, Space, Enter, Tab; the owner Oct 8 test 2 overruled
//!   boss A_014_01's "types a character" refusal - such a key then does the action instead of typing, everywhere). Only
//!   plain Esc / Backspace stay the key field's own (cancel / clear); with a modifier they bind too;
//! - the OS layer is the [`KeysOs`] trait: [`real::RealKeysOs`] = RegisterHotKey / UnregisterHotKey on the app's window
//!   (WM_HOTKEY, wParam = slot → [`KeysManager::action_for_slot`]) for normal keys; Raw Input (RIDEV_INPUTSINK, listen
//!   only, no hooks anywhere — boss A_014_01) for modifier-only keys, mouse buttons and actions flagged
//!   [`Action::needs_release`], registered only while such a key is bound ([`raw`]) — through bu-rawin, the process's
//!   one Raw Input owner on its own thread (Order 048): it wakes the message window with `services::WM_RAWKEYS` only
//!   for key packets and mouse buttons / wheel (never a move) → `services::raw_packets` → [`KeysManager::on_raw`] →
//!   `(action id, down)`. [`fake::FakeKeysOs`] for tests — tests never register keys or raw input;
//! - the mapping is saved in the settings store (app scope, key "keys") on every change, so "Reset the app's own
//!   settings" clears it ([`KeysManager::reload`] afterwards).

pub mod capture;
pub mod fake;
pub mod raw;
pub mod real;
#[cfg(test)]
mod raw_tests;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use crate::settings::{Scope, SettingsStore};

pub use capture::{Capture, KeyEvent, Step};
pub use raw::{Packet, RawRouter, VK_MBUTTON, VK_XBUTTON1, VK_XBUTTON2};

/// The settings key (app scope) the mapping is saved under: a list of `action_id=mods,vk`.
pub const SETTING: &str = "keys";

// Virtual-key codes the manager needs by name.
pub const VK_BACK: u16 = 0x08;
pub const VK_TAB: u16 = 0x09;
pub const VK_CLEAR: u16 = 0x0C;
pub const VK_RETURN: u16 = 0x0D;
pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12;
pub const VK_PAUSE: u16 = 0x13;
pub const VK_CAPITAL: u16 = 0x14;
pub const VK_ESCAPE: u16 = 0x1B;
pub const VK_SPACE: u16 = 0x20;
pub const VK_PRIOR: u16 = 0x21;
pub const VK_NEXT: u16 = 0x22;
pub const VK_END: u16 = 0x23;
pub const VK_HOME: u16 = 0x24;
pub const VK_LEFT: u16 = 0x25;
pub const VK_UP: u16 = 0x26;
pub const VK_RIGHT: u16 = 0x27;
pub const VK_DOWN: u16 = 0x28;
pub const VK_SNAPSHOT: u16 = 0x2C;
pub const VK_INSERT: u16 = 0x2D;
pub const VK_DELETE: u16 = 0x2E;
pub const VK_LWIN: u16 = 0x5B;
pub const VK_RWIN: u16 = 0x5C;
pub const VK_APPS: u16 = 0x5D;
pub const VK_NUMPAD0: u16 = 0x60;
pub const VK_DIVIDE: u16 = 0x6F;
pub const VK_F1: u16 = 0x70;
pub const VK_F24: u16 = 0x87;
pub const VK_NUMLOCK: u16 = 0x90;
pub const VK_SCROLL: u16 = 0x91;
pub const VK_LSHIFT: u16 = 0xA0;
pub const VK_RSHIFT: u16 = 0xA1;
pub const VK_LCONTROL: u16 = 0xA2;
pub const VK_RCONTROL: u16 = 0xA3;
pub const VK_LMENU: u16 = 0xA4;
pub const VK_RMENU: u16 = 0xA5;

/// Modifier keys held with the main key. Shown in one order: Ctrl, Alt, Shift, Win (the drawing's MOD_ORDER).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Mods(u8);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const CTRL: Mods = Mods(1);
    pub const ALT: Mods = Mods(2);
    pub const SHIFT: Mods = Mods(4);
    pub const WIN: Mods = Mods(8);

    pub fn bits(self) -> u8 {
        self.0
    }
    pub fn from_bits(b: u8) -> Mods {
        Mods(b & 15)
    }
    pub fn contains(self, m: Mods) -> bool {
        self.0 & m.0 == m.0
    }
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
    pub fn with(self, m: Mods) -> Mods {
        Mods(self.0 | m.0)
    }
    pub fn without(self, m: Mods) -> Mods {
        Mods(self.0 & !m.0)
    }

    /// The modifier a virtual key is (None = not a modifier).
    pub fn of_vk(vk: u16) -> Option<Mods> {
        match vk {
            VK_CONTROL | VK_LCONTROL | VK_RCONTROL => Some(Mods::CTRL),
            VK_MENU | VK_LMENU | VK_RMENU => Some(Mods::ALT),
            VK_SHIFT | VK_LSHIFT | VK_RSHIFT => Some(Mods::SHIFT),
            VK_LWIN | VK_RWIN => Some(Mods::WIN),
            _ => None,
        }
    }

    /// The names in the drawing's order: ["Ctrl", "Alt", "Shift", "Win"].
    pub fn names(self) -> Vec<&'static str> {
        [(Mods::CTRL, "Ctrl"), (Mods::ALT, "Alt"), (Mods::SHIFT, "Shift"), (Mods::WIN, "Win")]
            .into_iter()
            .filter(|(m, _)| self.contains(*m))
            .map(|(_, n)| n)
            .collect()
    }
}

/// One key combo: modifiers + one main key (a Windows virtual-key code). `vk` 0 = modifiers only; VK_MBUTTON /
/// VK_XBUTTON1 / VK_XBUTTON2 = mouse button 3 / 4 / 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Combo {
    pub mods: Mods,
    pub vk: u16,
}

impl Combo {
    pub fn new(mods: Mods, vk: u16) -> Self {
        Combo { mods, vk }
    }

    /// Modifiers only ("Ctrl + Shift"): fires when they are released with nothing else pressed.
    pub fn mods_only(mods: Mods) -> Self {
        Combo { mods, vk: 0 }
    }

    /// Modifiers + mouse button `n` (3, 4 or 5; anything else gives button 5).
    pub fn mouse(mods: Mods, n: u8) -> Self {
        let vk = match n {
            3 => VK_MBUTTON,
            4 => VK_XBUTTON1,
            _ => VK_XBUTTON2,
        };
        Combo { mods, vk }
    }

    pub fn is_mods_only(&self) -> bool {
        self.vk == 0
    }

    /// 3 / 4 / 5 for a mouse-button combo.
    pub fn mouse_button(&self) -> Option<u8> {
        match self.vk {
            VK_MBUTTON => Some(3),
            VK_XBUTTON1 => Some(4),
            VK_XBUTTON2 => Some(5),
            _ => None,
        }
    }

    /// `mods,vk` as saved in the settings file.
    fn to_saved(self) -> String {
        format!("{},{}", self.mods.bits(), self.vk)
    }

    fn from_saved(s: &str) -> Option<Combo> {
        let (m, v) = s.split_once(',')?;
        let m: u8 = m.parse().ok()?;
        (m <= 15).then_some(())?;
        Some(Combo { mods: Mods::from_bits(m), vk: v.parse().ok()? })
    }
}

/// Something a key can do. Feature crates / pages describe their actions; the app adds them at start-up (keys work with
/// the menu closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// Unique in the app, stable (saved in the settings file), e.g. "micmute.toggle".
    pub id: String,
    /// What the user sees ("Mic mute") — also in "Already used by …".
    pub name: String,
    /// The owner page's id ("audio").
    pub page: String,
    /// Needs the key's release too (down + up, e.g. hold-to-talk): watched with Raw Input instead of RegisterHotKey —
    /// the key then also reaches the app in front (Raw Input only listens).
    pub needs_release: bool,
    /// Also fires while MORE modifiers are held than the key has (OBS's own rule: the key's modifiers must be held, extra
    /// ones are allowed - Insert still fires while Ctrl is held to crouch). Order 035 follow-up, used only by Notifications
    /// for OBS; watched with Raw Input like a release key. An exact match always wins over such a loose one.
    pub extra_mods: bool,
}

impl Action {
    pub fn new(id: &str, name: &str, page: &str) -> Self {
        Action { id: id.into(), name: name.into(), page: page.into(), needs_release: false, extra_mods: false }
    }

    /// Flag the action as needing the key's release too.
    pub fn with_release(mut self) -> Self {
        self.needs_release = true;
        self
    }

    /// Flag the action as firing with extra modifiers held too (see `extra_mods`).
    pub fn with_extra_mods(mut self) -> Self {
        self.extra_mods = true;
        self
    }
}

/// Why a key was not set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindError {
    /// No action with this id was added.
    UnknownAction(String),
    /// Another action already has this combo (its id and display name).
    UsedBy { action: String, name: String },
    /// A numpad key.
    Numpad,
    /// Not a key that can be bound (no key at all, left / right mouse button, a lone modifier key's vk, plain Esc /
    /// Backspace — the key field's own keys).
    NotAKey,
    /// Windows or another app holds this combo (RegisterHotKey / Raw Input registration said no) — the reason.
    Refused(String),
    /// The key works but the settings file could not be written (the reason).
    NotSaved(String),
}

impl BindError {
    /// The key field's short red line.
    pub fn message(&self) -> String {
        match self {
            BindError::UnknownAction(_) => "This can't have a key".into(),
            BindError::UsedBy { name, .. } => format!("Already used by {name}"),
            BindError::Numpad => "Numpad keys can't be used".into(),
            BindError::NotAKey => "This key can't be used".into(),
            BindError::Refused(_) => "Windows or another app uses this key".into(),
            BindError::NotSaved(_) => "Set, but not saved".into(),
        }
    }
}

/// The menu's own shortcuts (menu-v22: Apps / lists - Ctrl+F search, Ctrl+A select all, Delete): "used anywhere in the
/// app" (order 014) - a global key on them would take them from the menu (RegisterHotKey wins; REVIEW_014_item1c remark 1).
pub const MENU_KEYS: [(Combo, &str); 3] = [
    (Combo { mods: Mods::CTRL, vk: 0x46 }, "the menu's search (Ctrl+F)"),
    (Combo { mods: Mods::CTRL, vk: 0x41 }, "the menu's Select all (Ctrl+A)"),
    (Combo { mods: Mods::NONE, vk: 0x2E }, "the menu's Delete"),
];

/// An action's key as it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyState {
    Unbound,
    /// Set and registered.
    Working(Combo),
    /// Set and saved, but Windows refused it at start-up (another app took it) — the reason.
    NotWorking(Combo, String),
}

/// The OS layer. REAL = RegisterHotKey + Raw Input ([`real::RealKeysOs`]); FAKE = [`fake::FakeKeysOs`].
pub trait KeysOs {
    /// Register a hotkey under `slot` (RegisterHotKey id; WM_HOTKEY's wParam). Err = the reason Windows refused.
    fn register(&mut self, slot: i32, combo: Combo) -> Result<(), String>;
    fn unregister(&mut self, slot: i32);
    /// Make the registered raw devices exactly these (keyboard, mouse): RIDEV_INPUTSINK to add, RIDEV_REMOVE to drop.
    /// Called only when the wanted set changes.
    fn raw_devices(&mut self, keyboard: bool, mouse: bool) -> Result<(), String>;
    /// The modifiers held now, from Windows (None = unknown: keep the tracked ones). Asked only on a press packet.
    fn mods_now(&self) -> Option<Mods>;
    /// The key's name on the current keyboard layout ("Č", "Z", "+"); used for keys the manager has no fixed name for.
    fn key_name(&self, vk: u16) -> String;
}

/// The numpad keys (by virtual key alone).
pub fn is_numpad_vk(vk: u16) -> bool {
    (VK_NUMPAD0..=VK_DIVIDE).contains(&vk) || vk == VK_NUMLOCK || vk == VK_CLEAR
}

/// Keys that exist on the numpad too (NumLock off): the numpad sends them WITHOUT the extended flag.
pub fn is_nav_vk(vk: u16) -> bool {
    matches!(vk, VK_INSERT | VK_DELETE | VK_HOME | VK_END | VK_PRIOR | VK_NEXT | VK_LEFT | VK_UP | VK_RIGHT | VK_DOWN)
}

/// The fixed names (the drawing's KEYMAP + F-keys); None = ask the layout.
pub fn fixed_name(vk: u16) -> Option<String> {
    let n = match vk {
        VK_MBUTTON => "Mouse 3",
        VK_XBUTTON1 => "Mouse 4",
        VK_XBUTTON2 => "Mouse 5",
        VK_SPACE => "Space",
        VK_UP => "Up",
        VK_DOWN => "Down",
        VK_LEFT => "Left",
        VK_RIGHT => "Right",
        VK_ESCAPE => "Esc",
        VK_DELETE => "Del",
        VK_INSERT => "Ins",
        VK_PRIOR => "PgUp",
        VK_NEXT => "PgDn",
        VK_CAPITAL => "Caps",
        VK_BACK => "Backspace",
        VK_RETURN => "Enter",
        VK_TAB => "Tab",
        VK_HOME => "Home",
        VK_END => "End",
        VK_PAUSE => "Pause",
        VK_SCROLL => "ScrLk",
        VK_SNAPSHOT => "PrtSc",
        VK_APPS => "Menu",
        VK_NUMLOCK => "NumLk",
        VK_F1..=VK_F24 => return Some(format!("F{}", vk - VK_F1 + 1)),
        _ => return None,
    };
    Some(n.to_string())
}

struct Entry {
    action: Action,
    slot: i32,
    combo: Option<Combo>,
    /// Registered with the OS right now.
    live: bool,
    /// Why the OS refused the saved combo (start-up / resume).
    refused: Option<String>,
    /// Its feature is switched off (`set_active`): the key stays set and saved but is not registered with Windows, so
    /// the keystroke reaches the other apps.
    off: bool,
}

/// The keys manager. One per app, on the UI thread (the real layer registers on the app's window).
pub struct KeysManager<O: KeysOs> {
    os: O,
    entries: Vec<Entry>,
    /// The saved mapping (action id → combo), including actions not added (yet): nothing is lost when one is missing.
    saved: BTreeMap<String, Combo>,
    next_slot: i32,
    paused: bool,
    /// The Raw Input keys (modifier-only, mouse buttons, release keys) and which raw devices are registered now.
    router: RawRouter,
    raw_on: (bool, bool),
}

impl<O: KeysOs> KeysManager<O> {
    /// A manager with the saved mapping read from the store; keys are registered as their actions are added.
    pub fn new(os: O, store: &SettingsStore) -> Self {
        KeysManager {
            os,
            entries: Vec::new(),
            saved: read_saved(store),
            next_slot: 1,
            paused: false,
            router: RawRouter::new(),
            raw_on: (false, false),
        }
    }

    pub fn os(&self) -> &O {
        &self.os
    }

    /// Add an action; its saved key (if any, and not taken by an action added earlier) is registered now.
    pub fn add_action(&mut self, action: Action) {
        if self.entries.iter().any(|e| e.action.id == action.id) {
            return;
        }
        let slot = self.next_slot;
        self.next_slot += 1;
        let saved = self.saved.get(&action.id).copied();
        let combo = saved.filter(|c| self.check_combo(&action.id, *c).is_ok());
        self.entries.push(Entry { action, slot, combo, live: false, refused: None, off: false });
        let i = self.entries.len() - 1;
        if combo.is_some() && !self.paused {
            self.go_live(i);
        }
    }

    /// Remove an action and its key registration (its saved key stays in the file).
    pub fn remove_action(&mut self, id: &str) {
        if let Some(i) = self.find(id) {
            self.go_dead(i);
            self.entries.remove(i);
        }
    }

    /// Every added action, in the order added (Settings › All shortcuts).
    pub fn actions(&self) -> impl Iterator<Item = &Action> {
        self.entries.iter().map(|e| &e.action)
    }

    pub fn state(&self, id: &str) -> KeyState {
        match self.find(id).map(|i| &self.entries[i]) {
            Some(Entry { combo: Some(c), refused: Some(why), .. }) => KeyState::NotWorking(*c, why.clone()),
            Some(Entry { combo: Some(c), .. }) => KeyState::Working(*c),
            _ => KeyState::Unbound,
        }
    }

    /// The action that has this combo (None = free).
    pub fn used_by(&self, combo: Combo) -> Option<&Action> {
        self.entries.iter().find(|e| e.combo == Some(combo)).map(|e| &e.action)
    }

    /// May `id` have `combo`? Everything except asking Windows.
    pub fn check(&self, id: &str, combo: Combo) -> Result<(), BindError> {
        if self.find(id).is_none() {
            return Err(BindError::UnknownAction(id.into()));
        }
        self.check_combo(id, combo)
    }

    /// Set an action's key: checked, registered (the old key goes), saved. On any refusal the old key stays.
    pub fn bind(&mut self, store: &mut SettingsStore, id: &str, combo: Combo) -> Result<(), BindError> {
        self.check(id, combo)?;
        let i = self.find(id).ok_or_else(|| BindError::UnknownAction(id.into()))?;
        if self.entries[i].combo == Some(combo) && self.entries[i].refused.is_none() {
            return Ok(());
        }
        let old = self.entries[i].combo;
        self.go_dead(i);
        self.entries[i].combo = Some(combo);
        self.entries[i].refused = None;
        if let Err(why) = self.register(i) {
            self.entries[i].combo = old;
            if old.is_some() && !self.paused {
                self.go_live(i);
            }
            return Err(BindError::Refused(why));
        }
        self.entries[i].live = true;
        if self.paused {
            // the key field is still listening on another field: keep everything quiet until resume
            self.go_dead(i);
        }
        self.saved.insert(id.into(), combo);
        self.save(store).map_err(BindError::NotSaved)
    }

    /// Clear an action's key (the key field's ×, or Backspace while listening).
    pub fn unbind(&mut self, store: &mut SettingsStore, id: &str) -> Result<(), BindError> {
        let i = self.find(id).ok_or_else(|| BindError::UnknownAction(id.into()))?;
        self.go_dead(i);
        self.entries[i].combo = None;
        self.entries[i].refused = None;
        self.saved.remove(id);
        self.save(store).map_err(BindError::NotSaved)
    }

    /// WM_HOTKEY's wParam → the action to run.
    pub fn action_for_slot(&self, slot: i32) -> Option<&str> {
        self.entries.iter().find(|e| e.slot == slot && e.live).map(|e| e.action.id.as_str())
    }

    /// One Raw Input packet (from bu-rawin, parsed): `fire(action id, down)` for each action it triggers. Modifier-only and
    /// mouse-button keys fire once with down = true; release keys fire down and up. A mouse move does nothing.
    #[inline]
    pub fn on_raw(&mut self, p: Packet, mut fire: impl FnMut(&str, bool)) {
        if self.router.is_empty() {
            return;
        }
        if p.is_press() {
            if let Some(m) = self.os.mods_now() {
                self.router.resync(m);
            }
        }
        let entries = &self.entries;
        self.router.feed(p, &mut |slot, down| {
            if let Some(e) = entries.iter().find(|e| e.slot == slot && e.live) {
                fire(&e.action.id, down);
            }
        });
    }

    /// Which raw devices are registered now (keyboard, mouse).
    pub fn raw_devices_on(&self) -> (bool, bool) {
        self.raw_on
    }

    /// While a key field listens: every key is unregistered so the field sees keys that are already set.
    pub fn pause(&mut self) {
        if self.paused {
            return;
        }
        self.paused = true;
        for i in 0..self.entries.len() {
            self.go_dead(i);
        }
    }

    /// The key field stopped listening: every set key is registered again.
    pub fn resume(&mut self) {
        if !self.paused {
            return;
        }
        self.paused = false;
        for i in 0..self.entries.len() {
            if self.entries[i].combo.is_some() {
                self.go_live(i);
            }
        }
    }

    /// Switch an action's key on / off without forgetting it (Mic mute switched off, or the mode it doesn't use): off = not
    /// registered with Windows (the keystroke goes to the other apps); on = registered again if it has a key.
    pub fn set_active(&mut self, id: &str, on: bool) {
        let Some(i) = self.find(id) else { return };
        self.entries[i].off = !on;
        if on {
            if self.entries[i].combo.is_some() && !self.paused {
                self.go_live(i);
            }
        } else {
            self.go_dead(i);
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Re-read the mapping from the store (after Settings › "Reset the app's own settings").
    pub fn reload(&mut self, store: &SettingsStore) {
        for i in 0..self.entries.len() {
            self.go_dead(i);
            self.entries[i].combo = None;
            self.entries[i].refused = None;
        }
        self.saved = read_saved(store);
        for i in 0..self.entries.len() {
            let saved = self.saved.get(&self.entries[i].action.id).copied();
            let id = self.entries[i].action.id.clone();
            self.entries[i].combo = saved.filter(|c| self.check_combo(&id, *c).is_ok());
            if self.entries[i].combo.is_some() && !self.paused {
                self.go_live(i);
            }
        }
    }

    /// "Ctrl + Shift + Č", "Ctrl + Shift" (modifiers only), "Alt + Mouse 4" (the key field's caps are this split on " + ").
    pub fn combo_text(&self, combo: Combo) -> String {
        let mut parts: Vec<String> = combo.mods.names().into_iter().map(String::from).collect();
        if !combo.is_mods_only() {
            parts.push(self.key_name(combo.vk));
        }
        parts.join(" + ")
    }

    /// The modifiers held so far while listening ("Ctrl + Shift"; the field adds "+ …").
    pub fn held_text(&self, mods: Mods) -> String {
        mods.names().join(" + ")
    }

    /// One key's name: the fixed names first, else the keyboard layout's.
    pub fn key_name(&self, vk: u16) -> String {
        fixed_name(vk).unwrap_or_else(|| self.os.key_name(vk))
    }

    // ---- internals

    fn find(&self, id: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.action.id == id)
    }

    fn check_combo(&self, id: &str, combo: Combo) -> Result<(), BindError> {
        let vk = combo.vk;
        if is_numpad_vk(vk) {
            return Err(BindError::Numpad);
        }
        let m = combo.mods;
        let keyboard_key = !combo.is_mods_only() && combo.mouse_button().is_none();
        if combo.is_mods_only() && m.is_empty() {
            return Err(BindError::NotAKey);
        }
        if keyboard_key && (vk < 0x07 || vk > 0xFE || Mods::of_vk(vk).is_some() || (m.is_empty() && matches!(vk, VK_ESCAPE | VK_BACK))) {
            return Err(BindError::NotAKey);
        }
        if let Some((_, name)) = MENU_KEYS.iter().find(|(c, _)| *c == combo) {
            return Err(BindError::UsedBy { action: "menu".into(), name: (*name).into() });
        }
        if let Some(other) = self.entries.iter().find(|e| e.combo == Some(combo) && e.action.id != id) {
            return Err(BindError::UsedBy { action: other.action.id.clone(), name: other.action.name.clone() });
        }
        Ok(())
    }

    /// Goes through Raw Input (not RegisterHotKey).
    fn is_raw(e: &Entry, combo: Combo) -> bool {
        combo.is_mods_only() || combo.mouse_button().is_some() || e.action.needs_release || e.action.extra_mods
    }

    fn register(&mut self, i: usize) -> Result<(), String> {
        let e = &self.entries[i];
        let Some(combo) = e.combo else { return Ok(()) };
        if Self::is_raw(e, combo) {
            let (slot, release, loose) = (e.slot, e.action.needs_release, e.action.extra_mods);
            self.router.add_loose(slot, combo, release, loose);
            self.sync_raw().inspect_err(|_| {
                self.router.remove(slot);
                let _ = self.sync_raw();
            })
        } else {
            self.os.register(e.slot, combo)
        }
    }

    /// Register / drop the raw devices so they match what the bound keys need (nothing bound = nothing registered).
    fn sync_raw(&mut self) -> Result<(), String> {
        let want = self.router.needs();
        if want == self.raw_on {
            return Ok(());
        }
        self.os.raw_devices(want.0, want.1)?;
        if (want.0 && !self.raw_on.0) || (want.1 && !self.raw_on.1) {
            self.router.reset_state();
        }
        self.raw_on = want;
        Ok(())
    }

    fn go_live(&mut self, i: usize) {
        if self.entries[i].live || self.entries[i].off {
            return;
        }
        match self.register(i) {
            Ok(()) => {
                self.entries[i].live = true;
                self.entries[i].refused = None;
            }
            Err(why) => self.entries[i].refused = Some(why),
        }
    }

    fn go_dead(&mut self, i: usize) {
        let e = &mut self.entries[i];
        if !e.live {
            return;
        }
        e.live = false;
        let slot = e.slot;
        if e.combo.is_some_and(|c| Self::is_raw(e, c)) {
            self.router.remove(slot);
            let _ = self.sync_raw();
        } else {
            self.os.unregister(slot);
        }
    }

    fn save(&self, store: &mut SettingsStore) -> Result<(), String> {
        let list: Vec<String> = self.saved.iter().map(|(id, c)| format!("{id}={}", c.to_saved())).collect();
        let r = if list.is_empty() { store.remove(Scope::App, SETTING) } else { store.set_list(Scope::App, SETTING, &list) };
        r.map(|_| ()).map_err(|e| e.to_string())
    }
}

impl<O: KeysOs> Drop for KeysManager<O> {
    fn drop(&mut self) {
        for i in 0..self.entries.len() {
            self.go_dead(i);
        }
    }
}

fn read_saved(store: &SettingsStore) -> BTreeMap<String, Combo> {
    let mut out = BTreeMap::new();
    for line in store.get_list(Scope::App, SETTING).unwrap_or(&[]) {
        if let Some((id, c)) = line.rsplit_once('=') {
            if let Some(c) = Combo::from_saved(c) {
                out.insert(id.to_string(), c);
            }
        }
    }
    out
}
