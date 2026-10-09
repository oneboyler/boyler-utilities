//! What the keyboard picture edits (pure, no Windows): which keys are remapped (what Windows has + what the page wants), which
//! carry a preset action or a macro, and the macros. One key does ONE thing: switching it to another kind clears the old kind.

use bu_keysound::binds::{Bind, Binds, Preset};
use bu_keysound::macros::{self, Macro, Step};
use bu_keysound::remap::{self, Code, Mapping, DISABLED};

/// What a key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// As Windows made it.
    Normal,
    /// Another key (Windows' own key map).
    Remap,
    /// A preset action (media, volume, open …) or one of the app's own actions.
    Action,
    Macro,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Normal, Mode::Remap, Mode::Action, Mode::Macro];
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "Normal",
            Mode::Remap => "Remap",
            Mode::Action => "Action",
            Mode::Macro => "Macro",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Model {
    /// The remaps Windows has now (read from the registry when the page opens).
    pub applied: Vec<Mapping>,
    /// The remaps the page wants; differs from `applied` until Apply.
    pub pending: Vec<Mapping>,
    pub binds: Binds,
    pub macros: Vec<Macro>,
}

impl Model {
    pub fn new(applied: Vec<Mapping>, binds: Binds, macros: Vec<Macro>) -> Model {
        Model { pending: applied.clone(), applied, binds, macros }
    }

    pub fn remap_of(&self, c: Code) -> Option<Code> {
        self.pending.iter().find(|m| m.from == c).map(|m| m.to)
    }

    /// The mode a key is in now (an app action bound through the keys manager is asked by the page: `app_action`).
    pub fn mode(&self, c: Code, app_action: bool) -> Mode {
        if self.remap_of(c).is_some() {
            Mode::Remap
        } else {
            match self.binds.get(c) {
                Some(Bind::Preset(_)) => Mode::Action,
                Some(Bind::Macro(_)) => Mode::Macro,
                Some(Bind::App(_)) => Mode::Action,
                None if app_action => Mode::Action,
                None => Mode::Normal,
            }
        }
    }

    /// Is the key changed (it glows)?
    pub fn changed(&self, c: Code, app_action: bool) -> bool {
        self.mode(c, app_action) != Mode::Normal
    }

    /// The key gets remapped to `to` (0 = turned off). Err (nothing changed): the list wouldn't be valid.
    pub fn set_remap(&mut self, c: Code, to: Code) -> Result<(), String> {
        // a key "becoming itself" is the key as Windows made it - nothing is stored (Order 059)
        if c == to {
            self.reset_key(c);
            return Ok(());
        }
        let mut next: Vec<Mapping> = self.pending.iter().filter(|m| m.from != c).copied().collect();
        next.push(Mapping { from: c, to });
        remap::check(&next)?;
        self.pending = next;
        self.binds.remove(c);
        Ok(())
    }

    /// The key runs a preset action.
    pub fn set_preset(&mut self, c: Code, p: Preset) -> Result<(), String> {
        p.check()?;
        self.binds.set(c, Bind::Preset(p))?;
        self.pending.retain(|m| m.from != c);
        Ok(())
    }

    /// The key runs macro `id`.
    pub fn set_macro(&mut self, c: Code, id: &str) -> Result<(), String> {
        if !self.macros.iter().any(|m| m.id == id) {
            return Err("no such macro".into());
        }
        self.binds.set(c, Bind::Macro(id.to_string()))?;
        self.pending.retain(|m| m.from != c);
        Ok(())
    }

    /// "Reset this key": as Windows made it.
    pub fn reset_key(&mut self, c: Code) {
        self.pending.retain(|m| m.from != c);
        self.binds.remove(c);
    }

    /// "Reset all".
    pub fn reset_all(&mut self) {
        self.pending.clear();
        self.binds.clear();
    }

    /// The remaps differ from what Windows has: Apply is needed.
    pub fn dirty(&self) -> bool {
        let key = |v: &[Mapping]| {
            let mut k: Vec<(Code, Code)> = v.iter().map(|m| (m.from, m.to)).collect();
            k.sort_unstable();
            k
        };
        key(&self.applied) != key(&self.pending)
    }

    /// The remaps were written: they are what Windows has now (it uses them after a restart).
    pub fn mark_applied(&mut self) {
        self.applied = self.pending.clone();
    }

    pub fn remap_count(&self) -> usize {
        self.pending.len()
    }

    /// Keys with a preset action or a macro.
    pub fn bind_count(&self) -> usize {
        self.binds.len()
    }

    pub fn macro_by_id(&self, id: &str) -> Option<&Macro> {
        self.macros.iter().find(|m| m.id == id)
    }

    /// A new, empty macro (named "New macro N"); Err at the limit.
    pub fn new_macro(&mut self) -> Result<String, String> {
        if self.macros.len() >= macros::MAX_MACROS {
            return Err(format!("at most {} macros", macros::MAX_MACROS));
        }
        let id = macros::new_id(&self.macros);
        let n = self.macros.len() + 1;
        self.macros.push(Macro::new(&id, &format!("New macro {n}")));
        Ok(id)
    }

    /// A new macro with the template's name and steps; Err at the limit.
    pub fn new_macro_from(&mut self, name: &str, steps: Vec<Step>) -> Result<String, String> {
        let id = self.new_macro()?;
        if let Some(m) = self.macros.iter_mut().find(|m| m.id == id) {
            m.name = name.to_string();
            m.steps = steps;
        }
        Ok(id)
    }

    /// Deletes a macro and frees every key that ran it.
    pub fn delete_macro(&mut self, id: &str) {
        self.macros.retain(|m| m.id != id);
        self.binds.drop_macro(id);
    }

    /// The target of a remap as the card names it: a key name (the layout's), or "Disabled".
    pub fn target_is_disabled(to: Code) -> bool {
        to == DISABLED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> Model {
        let mut mo = Model::new(vec![Mapping { from: 0x3A, to: 0x01 }], Binds::new(), vec![Macro::new("m1", "One")]);
        mo.binds.set(0x44, Bind::Preset(Preset::PlayPause)).unwrap();
        mo
    }

    #[test]
    fn a_key_does_one_thing_at_a_time() {
        let mut mo = m();
        assert_eq!(mo.mode(0x3A, false), Mode::Remap);
        assert_eq!(mo.mode(0x44, false), Mode::Action);
        assert_eq!(mo.mode(0x57, false), Mode::Normal);
        assert_eq!(mo.mode(0x57, true), Mode::Action, "an app action on the key (the keys manager's)");
        // an action on a remapped key replaces the remap, and the other way round
        mo.set_preset(0x3A, Preset::VolumeUp).unwrap();
        assert_eq!(mo.mode(0x3A, false), Mode::Action);
        assert!(mo.remap_of(0x3A).is_none());
        mo.set_remap(0x44, 0x1D).unwrap();
        assert_eq!(mo.mode(0x44, false), Mode::Remap);
        assert!(mo.binds.get(0x44).is_none());
        mo.set_macro(0x44, "m1").unwrap();
        assert_eq!(mo.mode(0x44, false), Mode::Macro);
        assert!(mo.remap_of(0x44).is_none());
    }

    #[test]
    fn bad_changes_change_nothing() {
        let mut mo = m();
        let before = mo.clone();
        assert!(mo.set_remap(0x1E, 0x1E).is_ok(), "a key \"becoming itself\" is just Normal (Order 059)");
        assert_eq!(mo, before, "and changes nothing here: it was Normal already");
        assert!(mo.set_macro(0x57, "nope").is_err());
        assert!(mo.set_preset(0x57, Preset::OpenWeb("ftp://x".into())).is_err());
        assert_eq!(mo, before);
    }

    #[test]
    fn dirty_until_applied_and_order_does_not_matter() {
        let mut mo = m();
        assert!(!mo.dirty());
        mo.set_remap(0xE038, 0xE01D).unwrap();
        assert!(mo.dirty());
        mo.mark_applied();
        assert!(!mo.dirty());
        // the same two remaps in another order are the same
        mo.applied = vec![Mapping { from: 0xE038, to: 0xE01D }, Mapping { from: 0x3A, to: 0x01 }];
        assert!(!mo.dirty());
        // an action or macro change never needs Apply (it works at once)
        mo.set_preset(0x57, Preset::StopMedia).unwrap();
        assert!(!mo.dirty());
        // a disabled key is a remap to 0
        mo.set_remap(0xE05B, DISABLED).unwrap();
        assert!(mo.dirty());
        assert!(Model::target_is_disabled(0));
    }

    #[test]
    fn reset_key_and_reset_all() {
        let mut mo = m();
        mo.reset_key(0x3A);
        mo.reset_key(0x44);
        assert_eq!((mo.remap_count(), mo.bind_count()), (0, 0));
        assert!(mo.dirty(), "the remap Windows has is to be removed: Apply");
        let mut mo = m();
        mo.reset_all();
        assert_eq!((mo.remap_count(), mo.bind_count()), (0, 0));
        assert_eq!(mo.macros.len(), 1, "the macros stay: only the keys are reset");
    }

    #[test]
    fn deleting_a_macro_frees_its_keys() {
        let mut mo = m();
        mo.set_macro(0x57, "m1").unwrap();
        mo.set_macro(0x58, "m1").unwrap();
        mo.delete_macro("m1");
        assert!(mo.macros.is_empty());
        assert_eq!(mo.mode(0x57, false), Mode::Normal);
        assert_eq!(mo.mode(0x58, false), Mode::Normal);
        assert_eq!(mo.mode(0x44, false), Mode::Action, "other keys are untouched");
    }

    #[test]
    fn new_macros_get_fresh_ids_and_names_up_to_the_limit() {
        let mut mo = Model::default();
        assert_eq!(mo.new_macro().unwrap(), "m1");
        assert_eq!(mo.new_macro().unwrap(), "m2");
        assert_eq!(mo.macros[1].name, "New macro 2");
        for _ in 2..macros::MAX_MACROS {
            mo.new_macro().unwrap();
        }
        assert!(mo.new_macro().is_err());
    }
}
