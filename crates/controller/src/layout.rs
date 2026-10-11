//! One Steam Input layout file (`controller_<type>.vdf`): its groups, action sets, inputs and activators, read and edited
//! through [`crate::vdf::Doc`] so every byte that is not changed stays exactly as it was.
//!
//! Shape (measured on a real Rocket League file + 128 local layouts, 2026-10-08):
//! ```text
//! "controller_mappings" { "version" "3" "title" … "url" … "controller_type" …
//!   "actions" { "Default" { "title" "Default" "legacy_set" "1" } "Preset_1000001" { "title" "In menus" … } }   (only with >1 set)
//!   "group" { "id" "3" "mode" "joystick_move" "inputs" { "click" { "activators" { "Full_Press" { "bindings" { "binding" "…" }
//!            "settings" { … } } } } } "settings" { "deadzone_inner_radius" "3357" … } }      (one per input area and mode)
//!   "preset" { "id" "0" "name" "Default" "group_source_bindings" { "3" "joystick active" "9" "right_joystick inactive" … } }
//!   "settings" { … } }
//! ```
//! A preset = an action set. A source (`joystick`, `right_joystick`, `button_diamond`, `dpad`, `switch`, `left_trigger`,
//! `right_trigger`, `left_trackpad`, `right_trackpad`, `center_trackpad`, `gyro`) has at most one ACTIVE group per set.

use crate::binding::Action;
use crate::vdf::{Addr, Doc, EditError, Kind, ParseError};

/// Errors of the layout layer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error("this is not a Steam controller layout (no \"controller_mappings\")")]
    NotALayout,
    #[error("no action set with id {0} in this layout")]
    NoSuchSet(u32),
    #[error("the layout has no group for {0}")]
    NoGroup(String),
}

pub type LResult<T> = std::result::Result<T, LayoutError>;

/// One action set (Steam's `preset`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionSet {
    /// The preset's `id` (0 = the first / Default set).
    pub id: u32,
    /// The preset's `name` (`Default`, `Preset_1000001` …).
    pub name: String,
    /// The shown title from the `actions` block (falls back to the name).
    pub title: String,
}

/// The layout's own header lines.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Header {
    pub title: String,
    pub controller_type: String,
    pub url: String,
    pub progenitor: String,
    pub revision: String,
}

/// An activator = one way of pressing (Steam's names).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Press {
    /// "Does" (Steam: Regular Press)
    Full,
    Long,
    Double,
    Start,
    Release,
    /// Chorded press ("Together with"). Activator name `Chord` is a GUESS (no local layout uses one; steamclient64.dll
    /// names the setting `chord_button`).
    Chord,
    /// Trigger soft pull. Activator name `Soft_Press` is a GUESS (no local layout uses one).
    Soft,
}

impl Press {
    pub const ALL: [Press; 7] = [Press::Full, Press::Long, Press::Double, Press::Start, Press::Release, Press::Chord, Press::Soft];
    pub fn steam(self) -> &'static str {
        match self {
            Press::Full => "Full_Press",
            Press::Long => "Long_Press",
            Press::Double => "Double_Press",
            Press::Start => "Start_Press",
            Press::Release => "Release_Press",
            Press::Chord => "Chord",
            Press::Soft => "Soft_Press",
        }
    }
}

/// A Steam Input layout, ready to read and edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    doc: Doc,
}

const ROOT: [usize; 1] = [0];

impl Layout {
    pub fn parse(text: impl Into<String>) -> LResult<Layout> {
        let doc = Doc::parse(text)?;
        match doc.top() {
            Some(n) if n.key.eq_ignore_ascii_case("controller_mappings") && n.is_block() => Ok(Layout { doc }),
            _ => Err(LayoutError::NotALayout),
        }
    }

    pub fn text(&self) -> &str {
        self.doc.text()
    }
    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    pub fn header(&self) -> Header {
        let top = self.doc.top().expect("checked in parse");
        let v = |k: &str| top.child_value(k).unwrap_or("").to_string();
        Header { title: v("title"), controller_type: v("controller_type"), url: v("url"), progenitor: v("progenitor"), revision: v("revision") }
    }

    /// Set (or add) a header line, e.g. `url` / `progenitor` when a community layout becomes the user's own copy.
    pub fn set_header(&mut self, key: &str, value: &str) -> LResult<()> {
        if self.doc.find(&ROOT, key).is_some() {
            Ok(self.doc.upsert(&ROOT, key, value)?)
        } else {
            // header lines go in front of the first block (Steam's order), not at the end of the file
            let first_block = self.doc.top().and_then(|t| t.children().iter().position(|n| n.is_block()));
            match first_block {
                Some(i) => {
                    let sep = self.doc.separator();
                    let line = format!("\"{}\"{sep}\"{}\"", crate::vdf::escape(key), crate::vdf::escape(value));
                    Ok(self.doc.insert_lines_before(&[0, i], &[line])?)
                }
                None => Ok(self.doc.insert_value(&ROOT, key, value)?),
            }
        }
    }

    // ---------------------------------------------------------------- action sets

    pub fn action_sets(&self) -> Vec<ActionSet> {
        let top = self.doc.top().expect("checked in parse");
        let titles = top.child("actions");
        top.children_named("preset")
            .filter_map(|p| {
                let id = p.child_value("id")?.trim().parse::<u32>().ok()?;
                let name = p.child_value("name").unwrap_or("").to_string();
                let title = titles.and_then(|a| a.child(&name)).and_then(|s| s.child_value("title")).map(str::to_string).unwrap_or_else(|| name.clone());
                Some(ActionSet { id, name, title })
            })
            .collect()
    }

    fn preset_addr(&self, set: u32) -> Option<Addr> {
        let top = self.doc.top()?;
        let i = top
            .children()
            .iter()
            .position(|n| n.key.eq_ignore_ascii_case("preset") && n.child_value("id").and_then(|v| v.trim().parse::<u32>().ok()) == Some(set))?;
        Some(vec![0, i])
    }

    fn group_addr_by_id(&self, id: &str) -> Option<Addr> {
        let top = self.doc.top()?;
        let i = top.children().iter().position(|n| n.key.eq_ignore_ascii_case("group") && n.child_value("id").map(str::trim) == Some(id))?;
        Some(vec![0, i])
    }

    /// The group id bound ACTIVE to `source` in action set `set`.
    pub fn active_group_id(&self, set: u32, source: &str) -> Option<String> {
        let p = self.doc.get(&self.preset_addr(set)?)?;
        let gsb = p.child("group_source_bindings")?;
        gsb.children().iter().find_map(|b| {
            let v = b.value()?;
            let mut w = v.split_whitespace();
            (w.next()? == source && w.next()? == "active" && w.next().is_none()).then(|| b.key.clone())
        })
    }

    /// The address of the active group for `source` (stays valid until the next edit).
    pub fn group(&self, set: u32, source: &str) -> Option<Addr> {
        self.group_addr_by_id(&self.active_group_id(set, source)?)
    }

    /// The mode of the active group for `source` (`None` = nothing bound there).
    pub fn group_mode(&self, set: u32, source: &str) -> Option<String> {
        let g = self.doc.get(&self.group(set, source)?)?;
        g.child_value("mode").map(str::to_string)
    }

    /// A value from the active group's `settings` block.
    pub fn group_setting(&self, set: u32, source: &str, key: &str) -> Option<String> {
        let g = self.doc.get(&self.group(set, source)?)?;
        g.child("settings")?.child_value(key).map(str::to_string)
    }

    fn next_group_id(&self) -> u32 {
        let top = self.doc.top().expect("checked");
        top.children_named("group").filter_map(|g| g.child_value("id")?.trim().parse::<u32>().ok()).max().map(|m| m + 1).unwrap_or(0)
    }

    /// Make sure `source` has an active group in `set`; creates one with `mode` (Steam's autosave shape) if not.
    /// An existing group keeps its mode (use [`Layout::set_group_mode`] to change it).
    pub fn ensure_group(&mut self, set: u32, source: &str, mode: &str) -> LResult<Addr> {
        if let Some(a) = self.group(set, source) {
            return Ok(a);
        }
        let paddr = self.preset_addr(set).ok_or(LayoutError::NoSuchSet(set))?;
        let id = self.next_group_id().to_string();
        let sep = self.doc.separator();
        let kv = |k: &str, v: &str| format!("\t\"{k}\"{sep}\"{v}\"");
        let lines = vec![
            "\"group\"".to_string(),
            "{".into(),
            kv("id", &id),
            kv("mode", mode),
            kv("name", ""),
            kv("description", ""),
            "\t\"inputs\"".into(),
            "\t{".into(),
            "\t}".into(),
            "}".into(),
        ];
        // in front of the first preset (Steam's order: groups, then presets)
        let first_preset = self.doc.top().and_then(|t| t.children().iter().position(|n| n.key.eq_ignore_ascii_case("preset"))).map(|i| vec![0, i]);
        match first_preset {
            Some(a) => self.doc.insert_lines_before(&a, &lines)?,
            None => self.doc.insert_lines(&ROOT, &lines)?,
        }
        let _ = paddr;
        self.bind_source(set, &id, source)?;
        self.group_addr_by_id(&id).ok_or_else(|| LayoutError::NoGroup(source.into()))
    }

    /// Bind group `id` ACTIVE to `source` in `set` (an older active group of the same source is set inactive).
    pub fn bind_source(&mut self, set: u32, id: &str, source: &str) -> LResult<()> {
        if let Some(old) = self.active_group_id(set, source) {
            if old == id {
                return Ok(());
            }
            let p = self.preset_addr(set).ok_or(LayoutError::NoSuchSet(set))?;
            let g = self.doc.find(&p, "group_source_bindings").ok_or(LayoutError::NoSuchSet(set))?;
            let o = self.doc.find(&g, &old).ok_or(LayoutError::NoSuchSet(set))?;
            self.doc.set_value(&o, &format!("{source} inactive"))?;
        }
        let p = self.preset_addr(set).ok_or(LayoutError::NoSuchSet(set))?;
        let g = self.doc.ensure_block(&p, "group_source_bindings")?;
        Ok(self.doc.upsert(&g, id, &format!("{source} active"))?)
    }

    /// Take `source`'s active group out of `set` (its binding line goes; the group itself stays, as Steam keeps unused
    /// groups). Used for gyro "Off".
    pub fn unbind_source(&mut self, set: u32, source: &str) -> LResult<()> {
        let Some(id) = self.active_group_id(set, source) else { return Ok(()) };
        let p = self.preset_addr(set).ok_or(LayoutError::NoSuchSet(set))?;
        let g = self.doc.find(&p, "group_source_bindings").ok_or(LayoutError::NoSuchSet(set))?;
        let a = self.doc.find(&g, &id).ok_or(LayoutError::NoSuchSet(set))?;
        Ok(self.doc.remove(&a)?)
    }

    /// A group that is bound to `source` but not active (e.g. a switched-off gyro group), to reuse it.
    pub fn inactive_group_id(&self, set: u32, source: &str) -> Option<String> {
        let p = self.doc.get(&self.preset_addr(set)?)?;
        p.child("group_source_bindings")?.children().iter().find_map(|b| {
            let mut w = b.value()?.split_whitespace();
            (w.next()? == source && w.next()? == "inactive").then(|| b.key.clone())
        })
    }

    pub fn set_group_mode(&mut self, set: u32, source: &str, mode: &str) -> LResult<()> {
        let g = self.ensure_group(set, source, mode)?;
        Ok(self.doc.upsert(&g, "mode", mode)?)
    }

    /// Set (Some) or remove (None = Steam's default) a key in the active group's `settings`.
    pub fn set_group_setting(&mut self, set: u32, source: &str, default_mode: &str, key: &str, value: Option<&str>) -> LResult<()> {
        match value {
            Some(v) => {
                if self.group_setting(set, source, key).as_deref() == Some(v) {
                    return Ok(());
                }
                let g = self.ensure_group(set, source, default_mode)?;
                let s = self.doc.ensure_block(&g, "settings")?;
                Ok(self.doc.upsert(&s, key, v)?)
            }
            None => {
                let Some(g) = self.group(set, source) else { return Ok(()) };
                let Some(s) = self.doc.find(&g, "settings") else { return Ok(()) };
                let Some(a) = self.doc.find(&s, key) else { return Ok(()) };
                self.doc.remove(&a)?;
                // the last value gone: the empty block goes too (GUESS that Steam writes no empty group settings block;
                // it makes set + put back byte-identical)
                if let Some(s) = self.group(set, source).and_then(|g| self.doc.find(&g, "settings")) {
                    if self.doc.get(&s).map(|n| n.children().is_empty()).unwrap_or(false) {
                        self.doc.remove(&s)?;
                    }
                }
                Ok(())
            }
        }
    }

    // ---------------------------------------------------------------- inputs + activators

    fn activator_addr(&self, set: u32, source: &str, input: &str, press: Press) -> Option<Addr> {
        let g = self.group(set, source)?;
        let i = self.doc.find(&g, "inputs")?;
        let inp = self.doc.find(&i, input)?;
        let acts = self.doc.find(&inp, "activators")?;
        self.doc.find(&acts, press.steam())
    }

    /// Every binding of one activator (usually one; a key combo has several).
    pub fn bindings(&self, set: u32, source: &str, input: &str, press: Press) -> Vec<String> {
        let Some(a) = self.activator_addr(set, source, input, press) else { return vec![] };
        let Some(n) = self.doc.get(&a) else { return vec![] };
        n.child("bindings").map(|b| b.children_named("binding").filter_map(|x| x.value().map(str::to_string)).collect()).unwrap_or_default()
    }

    /// What one press does (the first binding; [`Action::Nothing`] when unbound).
    pub fn action(&self, set: u32, source: &str, input: &str, press: Press) -> Action {
        self.bindings(set, source, input, press).first().map(|b| Action::parse(b)).unwrap_or(Action::Nothing)
    }

    pub fn has_activator(&self, set: u32, source: &str, input: &str, press: Press) -> bool {
        self.activator_addr(set, source, input, press).is_some()
    }

    /// Set what one press does. `Nothing` removes the binding (a Regular press keeps its block + settings; any other
    /// activator block goes completely). A different action replaces every binding of that activator with one.
    pub fn set_action(&mut self, set: u32, source: &str, default_mode: &str, input: &str, press: Press, action: &Action) -> LResult<()> {
        let current = self.bindings(set, source, input, press);
        let new_value = action.binding_value(current.first().map(String::as_str));
        if current.len() <= 1 && current.first().cloned() == new_value && (new_value.is_some() || press == Press::Full || !self.has_activator(set, source, input, press)) {
            return Ok(()); // already so
        }
        match new_value {
            None => {
                let Some(a) = self.activator_addr(set, source, input, press) else { return Ok(()) };
                if press != Press::Full {
                    self.doc.remove(&a)?;
                } else {
                    while let Some(b) = self.activator_addr(set, source, input, press).and_then(|a| self.doc.find(&a, "bindings")).and_then(|b| self.doc.find(&b, "binding")) {
                        self.doc.remove(&b)?;
                    }
                }
                self.drop_empty_input(set, source, input)
            }
            Some(v) => {
                let g = self.ensure_group(set, source, default_mode)?;
                let i = self.doc.ensure_block(&g, "inputs")?;
                let inp = self.doc.ensure_block(&i, input)?;
                let acts = self.doc.ensure_block(&inp, "activators")?;
                let act = self.doc.ensure_block(&acts, press.steam())?;
                let b = self.doc.ensure_block(&act, "bindings")?;
                match self.doc.find(&b, "binding") {
                    Some(first) => {
                        self.doc.set_value(&first, &v)?;
                        // drop the extra bindings of a combo
                        loop {
                            let b = self.activator_addr(set, source, input, press).and_then(|a| self.doc.find(&a, "bindings")).ok_or(LayoutError::NoGroup(source.into()))?;
                            let n = self.doc.get(&b).map(|n| n.children_named("binding").count()).unwrap_or(0);
                            if n <= 1 {
                                break;
                            }
                            let last = self.doc.find_last(&b, "binding").ok_or(LayoutError::NoGroup(source.into()))?;
                            self.doc.remove(&last)?;
                        }
                        Ok(())
                    }
                    None => Ok(self.doc.insert_value(&b, "binding", &v)?),
                }
            }
        }
    }

    /// An input with nothing left (no binding in any activator, no activator settings) leaves the file, as Steam keeps
    /// unbound inputs out of it (measured: unbound back buttons have no input block).
    fn drop_empty_input(&mut self, set: u32, source: &str, input: &str) -> LResult<()> {
        let Some(g) = self.group(set, source) else { return Ok(()) };
        let Some(i) = self.doc.find(&g, "inputs") else { return Ok(()) };
        let Some(inp) = self.doc.find(&i, input) else { return Ok(()) };
        let used = self.doc.get(&inp).and_then(|n| n.child("activators")).map(|acts| {
            acts.children().iter().any(|a| {
                a.child("bindings").map(|b| b.children_named("binding").next().is_some()).unwrap_or(false)
                    || a.child("settings").map(|s| !s.children().is_empty()).unwrap_or(false)
            })
        });
        if used == Some(true) {
            return Ok(());
        }
        Ok(self.doc.remove(&inp)?)
    }

    /// A value from one activator's `settings` (e.g. `delay_end`, `hold_repeats`).
    pub fn activator_setting(&self, set: u32, source: &str, input: &str, press: Press, key: &str) -> Option<String> {
        let a = self.activator_addr(set, source, input, press)?;
        self.doc.get(&a)?.child("settings")?.child_value(key).map(str::to_string)
    }

    /// Set (Some) / remove (None = Steam's default) one activator setting. Setting a value on a missing activator creates
    /// it (with no binding) — callers only do that for the Regular press, which every button has.
    #[allow(clippy::too_many_arguments)] // the address of an activator is (set, source, mode, input, press) — kept flat like Steam's file
    pub fn set_activator_setting(&mut self, set: u32, source: &str, default_mode: &str, input: &str, press: Press, key: &str, value: Option<&str>) -> LResult<()> {
        if self.activator_setting(set, source, input, press, key).as_deref() == value {
            return Ok(());
        }
        match value {
            Some(v) => {
                let g = self.ensure_group(set, source, default_mode)?;
                let i = self.doc.ensure_block(&g, "inputs")?;
                let inp = self.doc.ensure_block(&i, input)?;
                let acts = self.doc.ensure_block(&inp, "activators")?;
                let act = self.doc.ensure_block(&acts, press.steam())?;
                let s = self.doc.ensure_block(&act, "settings")?;
                Ok(self.doc.upsert(&s, key, v)?)
            }
            None => {
                let Some(a) = self.activator_addr(set, source, input, press) else { return Ok(()) };
                let Some(s) = self.doc.find(&a, "settings") else { return Ok(()) };
                let Some(k) = self.doc.find(&s, key) else { return Ok(()) };
                self.doc.remove(&k)?;
                // the last value gone: the empty `settings` block goes too (Steam writes an activator's settings block
                // only when it has a value — every activator without settings in the local layouts has none)
                let s = self.activator_addr(set, source, input, press).and_then(|a| self.doc.find(&a, "settings"));
                if let Some(s) = s {
                    if self.doc.get(&s).map(|n| n.children().is_empty()).unwrap_or(false) {
                        self.doc.remove(&s)?;
                    }
                }
                // an activator Steam has no binding for and no settings: drop it, and the input if nothing is left
                if let Some(a) = self.activator_addr(set, source, input, press) {
                    let empty = self.doc.get(&a).map(|n| n.child("settings").is_none() && n.child("bindings").map(|b| b.children().is_empty()).unwrap_or(true)).unwrap_or(false);
                    if empty && press != Press::Full {
                        self.doc.remove(&a)?;
                    }
                }
                self.drop_empty_input(set, source, input)
            }
        }
    }

    // ---------------------------------------------------------------- action sets: add / rename

    /// Add a new action set as a COPY of set `from` (every group it uses is copied with a new id), named like Steam names
    /// them (`Preset_1000001` …) with `title` shown. Returns the new set's id. The `actions` block (Steam's list of set
    /// titles, measured shape: `"Preset_1000001" { "title" "…" "legacy_set" "1" }`) is created when missing.
    pub fn add_action_set(&mut self, from: u32, title: &str) -> LResult<u32> {
        let sets = self.action_sets();
        let base = sets.iter().find(|s| s.id == from).cloned().ok_or(LayoutError::NoSuchSet(from))?;
        let new_id = sets.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        let mut n = 1_000_001u32;
        while sets.iter().any(|s| s.name == format!("Preset_{n}")) {
            n += 1;
        }
        let new_name = format!("Preset_{n}");
        // 1. copy each group bound in the base set (verbatim text, new id)
        let bindings: Vec<(String, String)> = {
            let p = self.doc.get(&self.preset_addr(from).ok_or(LayoutError::NoSuchSet(from))?).cloned().ok_or(LayoutError::NoSuchSet(from))?;
            p.child("group_source_bindings").map(|g| g.children().iter().filter_map(|b| Some((b.key.clone(), b.value()?.to_string()))).collect()).unwrap_or_default()
        };
        let mut new_bindings = Vec::new();
        for (gid, src) in bindings {
            let Some(ga) = self.group_addr_by_id(&gid) else { continue };
            let gnode = self.doc.get(&ga).cloned().ok_or(LayoutError::NoGroup(gid.clone()))?;
            let nid = self.next_group_id().to_string();
            let text = self.doc.text();
            let block = &text[gnode.line_start..gnode.end];
            let indent = &text[gnode.line_start..gnode.key_span.start];
            let lines: Vec<String> = block.lines().map(|l| l.strip_suffix('\r').unwrap_or(l)).map(|l| l.strip_prefix(indent).unwrap_or(l).to_string()).collect();
            let first_preset = self.doc.top().and_then(|t| t.children().iter().position(|n| n.key.eq_ignore_ascii_case("preset"))).map(|i| vec![0, i]).ok_or(LayoutError::NoSuchSet(from))?;
            self.doc.insert_lines_before(&first_preset, &lines)?;
            // the copy now sits exactly where the first preset was: give it its new id
            let ida = self.doc.find(&first_preset, "id").ok_or(LayoutError::NoGroup(gid.clone()))?;
            self.doc.set_value(&ida, &nid)?;
            new_bindings.push((nid, src));
        }
        // 2. the new preset at the end of the presets
        let sep = self.doc.separator();
        let kv = |k: &str, v: &str| format!("\t\"{k}\"{sep}\"{v}\"");
        let mut lines = vec!["\"preset\"".to_string(), "{".into(), kv("id", &new_id.to_string()), kv("name", &new_name), "\t\"group_source_bindings\"".into(), "\t{".into()];
        for (k, v) in &new_bindings {
            lines.push(format!("\t\t\"{k}\"{sep}\"{v}\""));
        }
        lines.push("\t}".into());
        lines.push("}".into());
        let top = self.doc.top().cloned().ok_or(LayoutError::NotALayout)?;
        let last_preset = top.children().iter().rposition(|n| n.key.eq_ignore_ascii_case("preset")).ok_or(LayoutError::NoSuchSet(from))?;
        match top.children().get(last_preset + 1) {
            Some(_) => self.doc.insert_lines_before(&[0, last_preset + 1], &lines)?,
            None => self.doc.insert_lines(&ROOT, &lines)?,
        }
        // 3. titles
        let had_actions = self.doc.find(&ROOT, "actions").is_some();
        if !had_actions {
            let first_block = self.doc.top().and_then(|t| t.children().iter().position(|n| n.is_block())).ok_or(LayoutError::NotALayout)?;
            self.doc.insert_lines_before(&[0, first_block], &["\"actions\"".into(), "{".into(), "}".into()])?;
        }
        let acts = self.doc.find(&ROOT, "actions").ok_or(LayoutError::NotALayout)?;
        if self.doc.find(&acts, &base.name).is_none() {
            self.add_set_title(&base.name, &base.title)?;
        }
        self.add_set_title(&new_name, title)?;
        Ok(new_id)
    }

    fn add_set_title(&mut self, name: &str, title: &str) -> LResult<()> {
        let acts = self.doc.find(&ROOT, "actions").ok_or(LayoutError::NotALayout)?;
        let sep = self.doc.separator();
        let lines = vec![
            format!("\"{}\"", crate::vdf::escape(name)),
            "{".into(),
            format!("\t\"title\"{sep}\"{}\"", crate::vdf::escape(title)),
            format!("\t\"legacy_set\"{sep}\"1\""),
            "}".into(),
        ];
        Ok(self.doc.insert_lines(&acts, &lines)?)
    }

    /// Rename an action set's shown title.
    pub fn rename_action_set(&mut self, set: u32, title: &str) -> LResult<()> {
        let s = self.action_sets().into_iter().find(|s| s.id == set).ok_or(LayoutError::NoSuchSet(set))?;
        match self.doc.find(&ROOT, "actions").and_then(|a| self.doc.find(&a, &s.name)) {
            Some(a) => Ok(self.doc.upsert(&a, "title", title)?),
            None => {
                if self.doc.find(&ROOT, "actions").is_none() {
                    let first_block = self.doc.top().and_then(|t| t.children().iter().position(|n| n.is_block())).ok_or(LayoutError::NotALayout)?;
                    self.doc.insert_lines_before(&[0, first_block], &["\"actions\"".into(), "{".into(), "}".into()])?;
                }
                self.add_set_title(&s.name, title)
            }
        }
    }

    /// The root `settings` block value (e.g. `left_trackpad_mode`).
    pub fn root_setting(&self, key: &str) -> Option<String> {
        self.doc.top()?.child("settings")?.child_value(key).map(str::to_string)
    }

    /// Is there any group bound for `source` (active or not) — used to tell "no gyro in this layout".
    pub fn has_source(&self, set: u32, source: &str) -> bool {
        self.active_group_id(set, source).is_some()
    }

    /// The raw value node kinds are only needed by tests.
    #[doc(hidden)]
    pub fn count_groups(&self) -> usize {
        self.doc.top().map(|t| t.children_named("group").count()).unwrap_or(0)
    }

    #[doc(hidden)]
    pub fn is_value(&self, addr: &[usize]) -> bool {
        matches!(self.doc.get(addr).map(|n| &n.kind), Some(Kind::Value { .. }))
    }

    // ---------------------------------------------------------------- Order 090: a button's macro, played by Steam

    fn activators(&self, set: u32, source: &str, input: &str) -> Option<Addr> {
        let g = self.group(set, source)?;
        let i = self.doc.find(&g, "inputs")?;
        let inp = self.doc.find(&i, input)?;
        self.doc.find(&inp, "activators")
    }

    /// The input's Regular / Start activator blocks in file order, as written: (press, bindings, settings).
    fn raw_cmds(&self, set: u32, source: &str, input: &str) -> Vec<RawCmd> {
        let Some(a) = self.activators(set, source, input) else { return vec![] };
        let Some(n) = self.doc.get(&a) else { return vec![] };
        n.children()
            .iter()
            .filter_map(|c| {
                let press = if c.key.eq_ignore_ascii_case(Press::Full.steam()) {
                    Press::Full
                } else if c.key.eq_ignore_ascii_case(Press::Start.steam()) {
                    Press::Start
                } else {
                    return None;
                };
                let bindings = c.child("bindings").map(|b| b.children_named("binding").filter_map(|x| x.value().map(str::to_string)).collect()).unwrap_or_default();
                let settings = c.child("settings").map(|s| s.children().iter().filter_map(|x| x.value().map(|v| (x.key.clone(), v.to_string()))).collect()).unwrap_or_default();
                Some(RawCmd { press, bindings, settings })
            })
            .collect()
    }

    /// The input's Regular / Start commands in file order (Steam's "extra commands"): what each presses, when. A block that
    /// presses nothing (the Regular press only holding its settings, the app's macro marker) is no command.
    pub fn macro_cmds(&self, set: u32, source: &str, input: &str) -> Vec<MacroCmd> {
        self.raw_cmds(set, source, input)
            .into_iter()
            .filter(|r| !r.bindings.is_empty())
            .map(|r| MacroCmd { press: r.press, actions: r.bindings.iter().map(|b| Action::parse(b)).collect(), delay_ms: r.delay() })
            .collect()
    }

    /// Is the input a macro? The app's marker (a Start press that presses nothing, fired 1 ms in), or what only Steam's "Add
    /// extra command" makes: several Regular commands or several Start commands. (One Regular + one Start press, with or
    /// without a fire start delay, is a plain button with a Start press.)
    pub fn is_macro(&self, set: u32, source: &str, input: &str) -> bool {
        let r = self.raw_cmds(set, source, input);
        let n = |p: Press| r.iter().filter(|c| c.press == p && !c.bindings.is_empty()).count();
        r.iter().any(RawCmd::is_marker) || n(Press::Full) > 1 || n(Press::Start) > 1
    }

    /// Writes `cmds` as the input's commands (Long / Double / Release / chord stay): every Regular / Start block goes, then
    /// one block per command in order - `"Start_Press" { "bindings" { "binding" "…" } "settings" { "delay_start" "200" } }`,
    /// the shape Steam writes for "Add extra command" + "Fire start delay" - and the marker. The Regular press's own settings
    /// (long press time, turbo, toggle, haptics…) stay on the first Regular block (one that presses nothing when the macro
    /// has no Regular command); a command kept from before keeps its binding's text and its own settings. No commands = the
    /// macro ends: the Regular press is Nothing, its settings kept.
    pub fn set_macro(&mut self, set: u32, source: &str, default_mode: &str, input: &str, cmds: &[MacroCmd]) -> LResult<()> {
        let olds: Vec<RawCmd> = self.raw_cmds(set, source, input).into_iter().filter(|r| !r.is_marker()).collect();
        let regular: Vec<(String, String)> = olds.iter().find(|r| r.press == Press::Full).map(|r| r.settings.iter().filter(|(k, _)| !k.eq_ignore_ascii_case("delay_start")).cloned().collect()).unwrap_or_default();
        while let Some(a) = self.activators(set, source, input) {
            let first = self.doc.get(&a).and_then(|n| n.children().iter().position(|c| c.key.eq_ignore_ascii_case(Press::Full.steam()) || c.key.eq_ignore_ascii_case(Press::Start.steam())));
            let Some(i) = first else { break };
            let mut addr = a.clone();
            addr.push(i);
            self.doc.remove(&addr)?;
        }
        if cmds.is_empty() && regular.is_empty() {
            return self.drop_empty_input(set, source, input);
        }
        let g = self.ensure_group(set, source, default_mode)?;
        let i = self.doc.ensure_block(&g, "inputs")?;
        let inp = self.doc.ensure_block(&i, input)?;
        let acts = self.doc.ensure_block(&inp, "activators")?;
        let mut full_done = false;
        if !regular.is_empty() && !cmds.iter().any(|c| c.press == Press::Full) {
            self.write_cmd(&acts, source, Press::Full, &[], &regular)?;
            full_done = true;
        }
        let mut used = vec![false; olds.len()];
        for c in cmds {
            let old = (0..olds.len()).find(|j| !used[*j] && olds[*j].press == c.press && olds[*j].bindings.iter().map(|b| Action::parse(b)).collect::<Vec<_>>() == c.actions);
            let bindings: Vec<String> = match old {
                Some(j) => {
                    used[j] = true;
                    olds[j].bindings.clone()
                }
                None => c.actions.iter().filter_map(|a| a.binding_value(None)).collect(),
            };
            let mut settings: Vec<(String, String)> = old.map(|j| olds[j].settings.iter().filter(|(k, _)| !k.eq_ignore_ascii_case("delay_start")).cloned().collect()).unwrap_or_default();
            if c.press == Press::Full && !full_done {
                full_done = true;
                for (k, v) in &regular {
                    if !settings.iter().any(|(x, _)| x.eq_ignore_ascii_case(k)) {
                        settings.push((k.clone(), v.clone()));
                    }
                }
            }
            if c.delay_ms > 0 {
                settings.push(("delay_start".into(), c.delay_ms.to_string()));
            }
            self.write_cmd(&acts, source, c.press, &bindings, &settings)?;
        }
        if !cmds.is_empty() {
            self.write_cmd(&acts, source, Press::Start, &[], &[("delay_start".into(), MARKER_MS.into())])?;
        }
        Ok(())
    }

    /// One activator block at the end of `acts`.
    fn write_cmd(&mut self, acts: &[usize], source: &str, press: Press, bindings: &[String], settings: &[(String, String)]) -> LResult<()> {
        let name = press.steam();
        self.doc.insert_block(acts, name)?;
        let a = self.doc.find_last(acts, name).ok_or(LayoutError::NoGroup(source.into()))?;
        self.doc.insert_block(&a, "bindings")?;
        let b = self.doc.find_last(&a, "bindings").ok_or(LayoutError::NoGroup(source.into()))?;
        for v in bindings {
            self.doc.insert_value(&b, "binding", v)?;
        }
        if !settings.is_empty() {
            self.doc.insert_block(&a, "settings")?;
            let s = self.doc.find_last(&a, "settings").ok_or(LayoutError::NoGroup(source.into()))?;
            for (k, v) in settings {
                self.doc.insert_value(&s, k, v)?;
            }
        }
        Ok(())
    }
}

/// The app's mark on a macro it wrote: a Start press that presses nothing, fired this long after the press (Steam does
/// nothing for it; Steam's own screen never makes this shape).
const MARKER_MS: &str = "1";

/// One Regular / Start activator block as written.
struct RawCmd {
    press: Press,
    bindings: Vec<String>,
    settings: Vec<(String, String)>,
}

impl RawCmd {
    fn delay(&self) -> u32 {
        self.settings.iter().find(|(k, _)| k.eq_ignore_ascii_case("delay_start")).and_then(|(_, v)| v.trim().parse::<u32>().ok()).unwrap_or(0)
    }
    fn is_marker(&self) -> bool {
        self.press == Press::Start && self.bindings.is_empty() && self.settings.len() == 1 && self.settings.iter().any(|(k, v)| k.eq_ignore_ascii_case("delay_start") && v.trim() == MARKER_MS)
    }
}

/// Order 090 (boss A_090_01: a controller button's macro is played by Steam, so it works in games): one command of it - an
/// extra command on the button with Steam's "Fire start delay".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroCmd {
    /// [`Press::Start`] = a tap when the button goes down; [`Press::Full`] = held while the button is held.
    pub press: Press,
    /// What it presses (a key combo has several).
    pub actions: Vec<Action>,
    /// How long after the press it fires (ms).
    pub delay_ms: u32,
}

/// The longest wait a controller macro step may add (Steam's field takes any number; a quick helper, as the app's macros).
pub const MAX_MACRO_MS: u32 = 30_000;

#[cfg(test)]
mod macro_tests {
    use super::*;
    use crate::binding::MouseButton;

    const RL: &str = include_str!("../tests/fixtures/rl_ps5.vdf");

    #[test]
    fn a_macro_is_written_as_steams_extra_commands_and_read_back() {
        let mut l = Layout::parse(RL).unwrap();
        let (src, mode, input) = ("button_diamond", "four_buttons", "button_a");
        let before = l.action(0, src, input, Press::Full);
        assert!(!l.is_macro(0, src, input), "one plain command: no macro ({before:?})");
        // a Start press next to the Regular one is a plain button, not a macro
        l.set_action(0, src, mode, input, Press::Start, &Action::Key("G".into())).unwrap();
        assert!(!l.is_macro(0, src, input));
        assert_eq!(l.action(0, src, input, Press::Full), before, "the Regular press stays");
        l.set_action(0, src, mode, input, Press::Start, &Action::Nothing).unwrap();
        let cmds = vec![
            MacroCmd { press: Press::Start, actions: vec![Action::Key("LEFT_CONTROL".into()), Action::Key("C".into())], delay_ms: 0 },
            MacroCmd { press: Press::Start, actions: vec![Action::Mouse(MouseButton::Left)], delay_ms: 150 },
            MacroCmd { press: Press::Full, actions: vec![Action::Key("SPACE".into())], delay_ms: 300 },
        ];
        l.set_macro(0, src, mode, input, &cmds).unwrap();
        assert_eq!(l.macro_cmds(0, src, input), cmds);
        assert!(l.is_macro(0, src, input));
        // still a layout Steam reads, and only that input changed
        let text = l.text().to_string();
        assert!(Layout::parse(text.clone()).is_ok());
        assert!(text.contains("\"delay_start\"\t\t\"150\"") || text.contains("\"delay_start\" \"150\"") || text.contains("delay_start"), "{text}");
        // the Regular press's own setting (the fixture's "delay_end" 14) stays on the first Regular command
        assert_eq!(l.activator_setting(0, src, input, Press::Full, "delay_end").as_deref(), Some("14"));
        // ending it: no extra command is left, the setting still there
        l.set_macro(0, src, mode, input, &[]).unwrap();
        l.set_action(0, src, mode, input, Press::Full, &before).unwrap();
        assert!(!l.is_macro(0, src, input));
        assert_eq!(l.action(0, src, input, Press::Full), before);
        assert_eq!(l.macro_cmds(0, src, input).len(), 1);
        assert_eq!(l.activator_setting(0, src, input, Press::Full, "delay_end").as_deref(), Some("14"));
    }

    /// Review (Order 090): one-command macros are still macros, a press setting on an all-Start macro keeps it editable,
    /// a plain Start press with its own fire start delay is no macro, and a kept command keeps its binding's text.
    #[test]
    fn macro_marker_settings_and_binding_text() {
        let mut l = Layout::parse(RL).unwrap();
        let (src, mode, input) = ("button_diamond", "four_buttons", "button_a");
        // one Regular command that waits (a "Key down" macro): still a macro, and the setting holder is no command
        let one = vec![MacroCmd { press: Press::Full, actions: vec![Action::Key("A".into())], delay_ms: 100 }];
        l.set_macro(0, src, mode, input, &one).unwrap();
        assert!(l.is_macro(0, src, input));
        assert_eq!(l.macro_cmds(0, src, input), one);
        // all Start: the Regular settings sit on a Regular block that presses nothing; turbo set on it keeps the macro
        let starts = vec![MacroCmd { press: Press::Start, actions: vec![Action::Key("B".into())], delay_ms: 0 }];
        l.set_macro(0, src, mode, input, &starts).unwrap();
        assert_eq!(l.activator_setting(0, src, input, Press::Full, "delay_end").as_deref(), Some("14"));
        l.set_activator_setting(0, src, mode, input, Press::Full, "hold_repeats", Some("1")).unwrap();
        assert!(l.is_macro(0, src, input));
        assert_eq!(l.macro_cmds(0, src, input), starts);
        l.set_macro(0, src, mode, input, &starts).unwrap();
        assert_eq!(l.activator_setting(0, src, input, Press::Full, "hold_repeats").as_deref(), Some("1"), "kept on a rewrite");
        // a kept command keeps its binding's text (label) when the macro is written again
        let t = l.text().replace("key_press B, , ", "key_press B, Boost, ");
        let mut l = Layout::parse(t).unwrap();
        let more = vec![starts[0].clone(), MacroCmd { press: Press::Start, actions: vec![Action::Key("C".into())], delay_ms: 50 }];
        l.set_macro(0, src, mode, input, &more).unwrap();
        assert!(l.text().contains("key_press B, Boost, "), "{}", l.text());
        assert_eq!(l.macro_cmds(0, src, input), more);
        // a plain button with a Start press that has its own fire start delay is no macro
        let mut p = Layout::parse(RL).unwrap();
        p.set_action(0, src, mode, input, Press::Start, &Action::Key("G".into())).unwrap();
        p.set_activator_setting(0, src, mode, input, Press::Start, "delay_start", Some("100")).unwrap();
        assert!(!p.is_macro(0, src, input));
    }
}
