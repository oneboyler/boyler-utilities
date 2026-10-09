//! The Keyboard tab (Order 058): key sounds, a keyboard picture (remap / action / macro per key) and the macro builder.
//! Engine: `crates/keysound` (bu-keysound); the always-on part is `glue.rs`, what is remembered `prefs.rs`, what the picture
//! edits `model.rs`, the keyboard drawing `pic.rs`, the page's boxes `view.rs`. Drawing: boss\mockups\keyboard-v2.html.
//!
//! Key sounds are OFF by default and while off nothing listens. The pressed key is used only to pick the sound and is
//! forgotten at once (bu-rawin hands the sounds a class + up / down, never a key). A remap is Windows' own key map (one admin
//! Yes, a restart); a key's action or macro works at once through the keys manager, and is left alone while a game or a
//! full-screen window is in front.

use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, sub, El, Key};
use crate::ui::pieces::nbox::{self, Filter};
use bu_keysound::binds::{Bind, Preset};
use bu_keysound::layout::Layout;
use bu_keysound::macros::{Macro, Step};
use bu_keysound::remap::{self, Code, Mapping};
use bu_keysound::{Kind, Pack, PackId, Rule};

pub mod gallery;
mod getter;
pub mod glue;
pub mod model;
pub mod pic;
pub mod prefs;
mod view;
#[cfg(test)]
mod tests;

use model::{Mode, Model};
use prefs::{Prefs, Size};

const K_ON: Key = key("kbd.on");
const K_PACK: Key = key("kbd.pack");
const K_PLAY: Key = key("kbd.play");
const K_VOL: Key = key("kbd.vol");
const K_REP: Key = key("kbd.rep");
const K_MTPL: Key = key("kbd.mtpl");
const K_GAME: Key = key("kbd.game");
const K_MOUSE: Key = key("kbd.mouse");
const K_MVOL: Key = key("kbd.mvol");
const K_TRY: Key = key("kbd.try");
const K_RADD: Key = key("kbd.radd");
const K_RAPP: Key = key("kbd.rapp");
const K_RPACK: Key = key("kbd.rpack");
const K_RDEL: Key = key("kbd.rdel");
const K_SIZE: Key = key("kbd.size");
const K_RESETALL: Key = key("kbd.resetall");
const K_KEY: Key = key("kbd.key");
const K_KEYT: Key = key("kbd.keyt");
/// The key's small window (a popup like the controller's part window).
const K_KD: Key = key("kbd.kd");
const KD_W: f32 = 420.0;
const K_MODE: Key = key("kbd.mode");
const K_TARGET: Key = key("kbd.target");
const K_TPICK: Key = key("kbd.tpick");
const K_ACT: Key = key("kbd.act");
const K_ATEXT: Key = key("kbd.atext");
const K_ABROWSE: Key = key("kbd.abrowse");
const K_MACDD: Key = key("kbd.macdd");
const K_MACEDIT: Key = key("kbd.macedit");
const K_RESETKEY: Key = key("kbd.resetkey");
const K_APPLY: Key = key("kbd.apply");
const K_MNEW: Key = key("kbd.mnew");
const K_MEDIT: Key = key("kbd.medit");
const K_MENU: Key = key("kbd.menu");
const K_MD: Key = key("kbd.md");
const K_MDNAME: Key = key("kbd.mdname");
const K_MDADD: Key = key("kbd.mdadd");
const K_MDREC: Key = key("kbd.mdrec");
const K_MDDEL: Key = key("kbd.mddel");
const K_MDDONE: Key = key("kbd.mddone");
const K_STEP: Key = key("kbd.step");
const K_RESET: Key = key("kbd.reset");
const K_PLAYON: Key = key("kbd.playon");
/// The Get-more-sounds window: its search box and a row's play button.
const K_GSEARCH: Key = key("kbd.gsearch");
const K_GPLAY: Key = key("kbd.gplay");
/// The "Remove this sound?" question (right-click on an imported or downloaded sound in the list).
const K_DELQ: Key = key("kbd.delq");

/// The job that writes the remaps (the admin prompt never holds the menu).
const JOB_APPLY: &str = "kbd.apply";
/// The second box of the ISO Enter is `K_KEY` index + this.
const ISO_LOWER: usize = 1000;

/// Job ends already acted on. The page is made new every time its tab opens, but a finished job stays in the job list: without
/// this, an old "Get" download that ended earlier would be acted on again at every open and make its pack the sound again
/// (Order 076, the owner: "i change the sound, switch tabs, come back and its back to ... the one i had selected before").
struct Seen<T>(Vec<T>);

impl<T: PartialEq + Copy> Seen<T> {
    /// True the first time `id` is asked, false ever after.
    fn first(&mut self, id: T) -> bool {
        if self.0.contains(&id) {
            return false;
        }
        if self.0.len() >= 64 {
            self.0.remove(0);
        }
        self.0.push(id);
        true
    }
}

static SEEN: std::sync::Mutex<Seen<(&'static str, crate::jobs::JobId)>> = std::sync::Mutex::new(Seen(Vec::new()));

/// Is this the first time the page sees the end of job `id` of the job kind `key`?
fn first_end(key: &'static str, id: crate::jobs::JobId) -> bool {
    SEEN.lock().unwrap_or_else(|p| p.into_inner()).first((key, id))
}

/// A list that is open (its anchor = the button's box, from the press before the click).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Pop {
    Pack,
    RulePack(usize),
    Target,
    Action,
    MacroList,
    Templates,
    StepAdd,
}

/// What a row of the open list means.
#[derive(Debug, Clone, PartialEq)]
enum Choice {
    Heading,
    Pack(Pack),
    PackOff,
    GetMore,
    Import,
    Template(usize),
    Target(Code),
    Preset(Preset),
    AppAction(String),
    Macro(String),
    NewMacro,
    Step(u8),
}

#[derive(Default)]
pub struct Keyboard {
    loaded: bool,
    prefs: Prefs,
    model: Model,
    /// The keyboard picture of the chosen size and its labels (the user's layout).
    keys: Vec<pic::KeyBox>,
    texts: Vec<String>,
    w_keys: f32,
    h_keys: f32,
    sel: Option<Code>,
    /// When the key's window opened (its open motion).
    key_at: f64,
    /// The mode chosen on the card for a key that has no data yet.
    want: Option<Mode>,
    /// The Remap card waits for the new key (press it, or click it on the picture).
    choosing: bool,
    pop: Option<(Pop, (f32, f32, f32, f32))>,
    press: Option<(Key, (f32, f32, f32, f32))>,
    /// A red line in the key card (the keys manager's refusal, "numpad keys can't …").
    err: Option<String>,
    try_text: String,
    /// The macro being edited (its id) and the window's state.
    edit: Option<String>,
    rec: bool,
    rec_at: Option<f64>,
    listen_step: Option<usize>,
    opened_at: f64,
    /// Remaps were written this session: Windows needs a restart for them.
    restart: bool,
    test: bool,
    /// The user's app actions (the keys manager's), asked at open.
    app_actions: Vec<(String, String)>,
    sound_msg: Option<String>,
    /// Why the last pack import was refused (a plain line under the sounds).
    import_msg: Option<String>,
    /// A key was left as Windows made it because its action pressed that same key (the line under the keyboard).
    loop_note: Option<String>,
    /// The "Get more sounds" window is open and the line it shows.
    get_open: bool,
    get_msg: Option<String>,
    /// The search box of the Get-more-sounds window.
    get_q: String,
    /// "Remove this sound?" is asked for this imported pack, at this place.
    ask_del: Option<(String, (f32, f32))>,
}

impl Keyboard {
    fn layout(&self) -> Box<dyn Layout> {
        if self.test {
            Box::new(pic::Qwertz)
        } else {
            Box::new(bu_keysound::layout::current())
        }
    }

    fn rebuild_keys(&mut self) {
        let (ks, w, h) = pic::keys(self.prefs.size);
        let lay = self.layout();
        self.texts = pic::labels(&ks, &*lay);
        self.keys = ks;
        self.w_keys = w;
        self.h_keys = h;
    }

    fn label_of(&self, c: Code) -> String {
        // the full layout always knows every key the picture can show
        let (ks, _, _) = pic::keys(Size::Full);
        let lay = self.layout();
        let t = pic::labels(&ks, &*lay);
        ks.iter().position(|k| k.code == c).map(|i| t[i].clone()).unwrap_or_else(|| remap::name(c))
    }

    /// An action of the app's own keys manager on this key (not ours): its name.
    fn app_action_on(&self, c: Code) -> Option<String> {
        if self.test {
            return None;
        }
        let vk = self.layout().vk_of(c)?;
        crate::services::with(|s| s.keys.used_by(crate::keys::Combo::new(crate::keys::Mods::NONE, vk)).filter(|a| !a.id.starts_with("kbd.key.")).map(|a| a.name.clone())).flatten()
    }

    fn mode_of(&self, c: Code) -> Mode {
        self.model.mode(c, self.app_action_on(c).is_some())
    }

    fn load(&mut self, env: &Env) {
        self.test = env.fake();
        self.prefs = crate::services::with(|s| Prefs::load(&s.store)).unwrap_or_default();
        let applied = if self.test { Vec::new() } else { remap::real::read().unwrap_or_default() };
        self.model = Model::new(applied, self.prefs.binds.clone(), self.prefs.macros.clone());
        self.app_actions = crate::services::with(|s| s.action_list()).unwrap_or_default().into_iter().filter(|a| !a.id.starts_with("kbd.key.")).map(|a| (a.id, a.name)).collect();
        self.rebuild_keys();
        self.loaded = true;
        self.drop_self_loops();
        self.err = None;
    }

    /// Writes what changed to the settings file and brings the always-on parts (sounds, the keys that carry something) to it.
    fn save(&mut self) {
        self.drop_self_loops();
        self.prefs.binds = self.model.binds.clone();
        self.prefs.macros = self.model.macros.clone();
        let prefs = self.prefs.clone();
        let test = self.test;
        let errs = crate::services::with(|s| {
            prefs.save(&mut s.store);
            if test {
                return Vec::new();
            }
            glue::publish(&prefs.binds, &prefs.macros);
            let dir = glue::packs_dir(s.store.folder());
            self.sound_msg = glue::apply(&prefs, &dir).err();
            let lay = bu_keysound::layout::current();
            glue::sync_keys(s, &prefs.binds, &prefs.macros, &lay)
        })
        .unwrap_or_default();
        if let Some((_, e)) = errs.into_iter().next() {
            self.err = Some(e);
        }
    }

    /// A key whose action presses that same key (J -> a macro that presses J, F5 -> "Refresh") would run itself and never
    /// type: it is left as Windows made it, with a plain line saying why (Order 059).
    fn drop_self_loops(&mut self) {
        let lay = self.layout();
        let (kept, dropped) = bu_keysound::binds::without_self_loops(&self.model.binds, &self.model.macros, &|c| lay.vk_of(c));
        if dropped.is_empty() {
            return;
        }
        let names: Vec<String> = dropped.iter().map(|c| self.label_of(*c)).collect();
        self.model.binds = kept;
        let msg = format!(
            "{} can't run something that presses {} itself - it would run itself and the key would stop typing - so it stays as Windows made it. A typed text, or a combination like Ctrl + {}, works.",
            names.join(", "),
            if names.len() == 1 { "that same key" } else { "their own keys" },
            names[0]
        );
        self.err = Some(msg.clone());
        self.loop_note = Some(msg);
        if self.sel.is_some_and(|s| dropped.contains(&s)) {
            self.want = None;
        }
    }

    fn pack_name(&self, p: &Pack) -> String {
        match p {
            Pack::Builtin(id) => id.name().to_string(),
            Pack::Imported(n) => n.clone(),
        }
    }

    fn imported(&self) -> Vec<String> {
        let dir = crate::services::with(|s| glue::packs_dir(s.store.folder()));
        match (self.test, dir) {
            (false, Some(d)) => bu_keysound::import::installed(&d),
            _ => Vec::new(),
        }
    }

    // ------------------------------------------------------------------ the open list

    /// The rows of the open list and what each means (the same for painting and for the click).
    fn list(&self, p: Pop) -> Vec<(String, Choice, bool)> {
        let mut v: Vec<(String, Choice, bool)> = Vec::new();
        let packs = |v: &mut Vec<(String, Choice, bool)>, cur: Option<&Pack>, with_import: bool| {
            v.push(("Keyboard".into(), Choice::Heading, false));
            for id in PackId::ALL.iter().filter(|p| p.is_keyboard()) {
                v.push((id.name().into(), Choice::Pack(Pack::Builtin(*id)), cur == Some(&Pack::Builtin(*id))));
            }
            v.push(("Satisfying".into(), Choice::Heading, false));
            for id in PackId::ALL.iter().filter(|p| !p.is_keyboard()) {
                v.push((id.name().into(), Choice::Pack(Pack::Builtin(*id)), cur == Some(&Pack::Builtin(*id))));
            }
            let imp = self.imported();
            if !imp.is_empty() {
                v.push(("Imported or downloaded · right-click to remove".into(), Choice::Heading, false));
                for n in imp {
                    let p = Pack::Imported(n.clone());
                    v.push((n, Choice::Pack(p.clone()), cur == Some(&p)));
                }
            }
            if with_import {
                v.push(("Get more sounds…".into(), Choice::GetMore, false));
                v.push(("Import a pack…".into(), Choice::Import, false));
            }
        };
        match p {
            Pop::Pack => packs(&mut v, Some(&self.prefs.s.pack), true),
            Pop::RulePack(i) => {
                let cur = self.prefs.s.rules.get(i).and_then(|r| r.pack.clone());
                v.push(("Off".into(), Choice::PackOff, cur.is_none()));
                packs(&mut v, cur.as_ref(), false);
            }
            Pop::Target => {
                v.push(("Disabled (does nothing)".into(), Choice::Target(remap::DISABLED), false));
                for c in [0x01u16, 0x1D, 0xE01D, 0x2A, 0x36, 0x38, 0xE038, 0xE05B, 0xE05C, 0xE05D, 0x3A, 0x0F, 0x0E, 0x1C, 0xE053, 0xE052, 0xE047, 0xE04F, 0xE049, 0xE051] {
                    v.push((self.label_of(c), Choice::Target(c), self.sel.and_then(|s| self.model.remap_of(s)) == Some(c)));
                }
            }
            Pop::Action => {
                let cur = self.sel.and_then(|c| self.model.binds.get(c)).and_then(|b| if let Bind::Preset(p) = b { Some(p.id()) } else { None });
                let mut group = "";
                for p in Preset::SIMPLE.iter().cloned().chain([Preset::OpenApp(String::new()), Preset::OpenFolder(String::new()), Preset::OpenWeb(String::new())]) {
                    if p.group() != group {
                        group = p.group();
                        v.push((group.to_string(), Choice::Heading, false));
                    }
                    v.push((p.name().to_string(), Choice::Preset(p.clone()), cur == Some(p.id())));
                }
                if !self.app_actions.is_empty() {
                    v.push(("Boyler Utilities".into(), Choice::Heading, false));
                    for (id, name) in &self.app_actions {
                        v.push((name.clone(), Choice::AppAction(id.clone()), false));
                    }
                }
            }
            Pop::MacroList => {
                let cur = self.sel.and_then(|c| self.model.binds.get(c)).and_then(|b| if let Bind::Macro(m) = b { Some(m.clone()) } else { None });
                for m in &self.model.macros {
                    v.push((m.name.clone(), Choice::Macro(m.id.clone()), cur.as_deref() == Some(m.id.as_str())));
                }
                v.push(("New macro…".into(), Choice::NewMacro, false));
                v.push(("Ready-made".into(), Choice::Heading, false));
                for (i, (name, _)) in bu_keysound::macros::templates().into_iter().enumerate() {
                    v.push((name.to_string(), Choice::Template(i), false));
                }
            }
            Pop::Templates => {
                for (i, (name, _)) in bu_keysound::macros::templates().into_iter().enumerate() {
                    v.push((name.to_string(), Choice::Template(i), false));
                }
            }
            Pop::StepAdd => {
                for (i, l) in ["Press a key", "Type a text", "Wait", "Open an app, file or website"].iter().enumerate() {
                    v.push((l.to_string(), Choice::Step(i as u8), false));
                }
            }
        }
        v
    }

    fn open_pop(&mut self, p: Pop, k: Key) {
        // the button's box comes with the press before the click
        let a = self.press.filter(|(pk, _)| *pk == k).map(|(_, r)| r).unwrap_or((0.0, 0.0, 0.0, 0.0));
        self.pop = if self.pop.map(|(q, _)| q) == Some(p) { None } else { Some((p, a)) };
    }

    fn choose(&mut self, c: Choice, cx: &mut Cx) {
        let sel = self.sel;
        match c.clone() {
            Choice::Heading => {}
            Choice::Pack(p) => match self.pop.map(|(q, _)| q) {
                Some(Pop::RulePack(i)) => {
                    if let Some(r) = self.prefs.s.rules.get_mut(i) {
                        r.pack = Some(p.clone());
                    }
                    self.save();
                    self.preview(&p);
                }
                _ => {
                    self.prefs.s.pack = p.clone();
                    self.save();
                    self.preview(&p);
                }
            },
            Choice::PackOff => {
                if let Some(Pop::RulePack(i)) = self.pop.map(|(q, _)| q) {
                    if let Some(r) = self.prefs.s.rules.get_mut(i) {
                        r.pack = None;
                    }
                    self.save();
                }
            }
            Choice::GetMore => self.open_get(cx),
            Choice::Import => {
                self.pop = None;
                self.import_pack(cx);
            }
            Choice::Template(i) => {
                self.pop = None;
                if let Some((name, steps)) = bu_keysound::macros::templates().into_iter().nth(i) {
                    match self.model.new_macro_from(name, steps) {
                        Ok(id) => {
                            if let Some(s) = sel {
                                let _ = self.model.set_macro(s, &id);
                            }
                            self.open_editor(&id, cx.now);
                        }
                        Err(e) => self.err = Some(e),
                    }
                }
            }
            Choice::Target(code) => {
                if let Some(s) = sel {
                    match self.model.set_remap(s, code) {
                        Ok(()) => {
                            self.choosing = false;
                            self.err = None;
                            self.save();
                        }
                        Err(e) => self.err = Some(e),
                    }
                }
            }
            Choice::Preset(p) => {
                if let Some(s) = sel {
                    self.set_preset(s, p);
                }
            }
            Choice::AppAction(id) => {
                if let Some(s) = sel {
                    self.bind_app_action(s, &id);
                }
            }
            Choice::Macro(id) => {
                if let Some(s) = sel {
                    match self.model.set_macro(s, &id) {
                        Ok(()) => {
                            self.err = None;
                            self.save();
                        }
                        Err(e) => self.err = Some(e),
                    }
                }
            }
            Choice::NewMacro => {
                self.pop = None;
                if let Some(id) = self.new_macro() {
                    if let Some(s) = sel {
                        let _ = self.model.set_macro(s, &id);
                        self.save();
                    }
                    self.open_editor(&id, cx.now);
                }
            }
            Choice::Step(k) => {
                if let Some(m) = self.edit.clone().and_then(|id| self.model.macros.iter_mut().find(|x| x.id == id)) {
                    m.steps.push(match k {
                        0 => Step::Keys(Vec::new()),
                        1 => Step::Type(String::new()),
                        2 => Step::Wait(200),
                        _ => Step::Open(String::new()),
                    });
                    if let Step::Keys(_) = m.steps.last().unwrap() {
                        self.listen_step = Some(m.steps.len() - 1);
                    }
                }
                self.save();
            }
        }
        self.pop = None;
    }

    fn set_preset(&mut self, code: Code, p: Preset) {
        if pic::is_numpad(code) {
            self.err = Some("Numpad keys can't carry an action: not every keyboard has one".into());
            return;
        }
        match self.model.set_preset(code, p) {
            Ok(()) => {
                self.err = None;
                self.save();
            }
            Err(e) => self.err = Some(e),
        }
    }

    /// One of the app's own actions (Mic mute …) gets this key: it is the action's key from now on (the keys manager's).
    fn bind_app_action(&mut self, code: Code, action: &str) {
        if self.test {
            return;
        }
        if pic::is_numpad(code) {
            self.err = Some("Numpad keys can't be used: not every keyboard has one".into());
            return;
        }
        let lay = bu_keysound::layout::current();
        let Some(vk) = lay.vk_of(code) else {
            self.err = Some("This key has no place on your layout".into());
            return;
        };
        let r = crate::services::with(|s| {
            // the key is this action's now; a macro / preset of ours on it goes
            s.keys.bind(&mut s.store, action, crate::keys::Combo::new(crate::keys::Mods::NONE, vk)).map_err(|e| e.message())
        })
        .unwrap_or_else(|| Err("not ready".into()));
        match r {
            Ok(()) => {
                self.model.reset_key(code);
                self.err = None;
                self.save();
            }
            Err(e) => self.err = Some(e),
        }
    }

    fn preview(&self, p: &Pack) {
        if !self.prefs.on || self.test {
            return;
        }
        let (p, play_on) = (p.clone(), self.prefs.s.play_on);
        // the key-up sound a moment after the key-down (a short-lived thread, only for this click)
        std::thread::spawn(move || {
            if play_on.plays(true) {
                glue::engine().preview(&p, Kind::Down);
            }
            std::thread::sleep(std::time::Duration::from_millis(90));
            if play_on.plays(false) {
                glue::engine().preview(&p, Kind::Up);
            }
        });
    }

    /// "Import a pack…": ONE picker for the .zip as the site hands it out or a pack folder (a folder is picked by the
    /// config.json inside it: a file dialog can't pick both kinds). A clear line when it isn't a pack.
    fn import_pack(&mut self, cx: &mut Cx) {
        let picked = cx.pick_file("Pick a Mechvibes pack: its .zip, or the config.json inside its folder", &[("Mechvibes pack (.zip or config.json)", "*.zip;config.json")]);
        let Some(src) = picked else { return };
        let dir = crate::services::with(|s| glue::packs_dir(s.store.folder()));
        let Some(dir) = dir else { return };
        match bu_keysound::import::install_any(std::path::Path::new(&src), &dir) {
            Ok(p) => {
                self.import_msg = None;
                self.prefs.s.pack = Pack::Imported(p.name.clone());
                self.save();
                cx.toast(&format!("Imported {} · it is the sound now", p.name));
            }
            Err(e) => {
                self.import_msg = Some(e.clone());
                cx.toast(&format!("Not imported: {e}"));
            }
        }
    }

    /// Right-click on an imported or downloaded sound in the list: ask before removing it.
    fn ask_remove(&mut self, k: Key, x: f32, y: f32) {
        if self.pop.is_none() {
            return;
        }
        for i in 0..64 {
            if k == idx(K_MENU, i) {
                if let Some(p) = self.pop.map(|(p, _)| p) {
                    if let Some((_, Choice::Pack(Pack::Imported(n)), _)) = self.list(p).get(i).cloned() {
                        self.ask_del = Some((n, (x, y)));
                    }
                }
                return;
            }
        }
    }

    /// The pack is gone from the PC: the settings that used it go back to a sound that exists (the general sound to Linear,
    /// a program's own sound to Off) and the engine lets its sounds go. Pure state; the files are `remove_pack`.
    fn forget_pack(&mut self, name: &str) -> bool {
        let gone = Pack::Imported(name.to_string());
        let was_in_use = self.prefs.s.pack == gone;
        if was_in_use {
            self.prefs.s.pack = Pack::Builtin(bu_keysound::PackId::Linear);
        }
        for r in self.prefs.s.rules.iter_mut().filter(|r| r.pack.as_ref() == Some(&gone)) {
            r.pack = None;
        }
        was_in_use
    }

    /// "Remove" was confirmed: the pack's folder is deleted, the settings that used it fall back.
    fn remove_pack(&mut self, name: &str, cx: &mut Cx) {
        let dir = crate::services::with(|s| glue::packs_dir(s.store.folder()));
        if !self.test {
            let Some(dir) = dir else { return };
            if let Err(e) = bu_keysound::import::remove(&dir, name) {
                cx.toast(&format!("Not removed: {e}"));
                return;
            }
            glue::engine().set_imported(name, None);
        }
        let was_in_use = self.forget_pack(name);
        self.save();
        cx.toast(&if was_in_use { format!("{name} removed · the sound is Linear again") } else { format!("{name} removed") });
    }

    fn new_macro(&mut self) -> Option<String> {
        self.model.new_macro().ok()
    }

    fn open_editor(&mut self, id: &str, now: f64) {
        self.edit = Some(id.to_string());
        self.rec = false;
        self.rec_at = None;
        self.listen_step = None;
        self.opened_at = now;
        self.pop = None;
        self.save();
    }

    fn close_editor(&mut self) {
        self.edit = None;
        self.rec = false;
        self.listen_step = None;
        self.pop = None;
        self.save();
    }

    fn macro_mut(&mut self) -> Option<&mut Macro> {
        let id = self.edit.clone()?;
        self.model.macros.iter_mut().find(|m| m.id == id)
    }

    fn select(&mut self, code: Option<Code>) {
        self.sel = code;
        self.want = None;
        self.choosing = false;
        self.err = None;
        self.pop = None;
    }

    fn set_mode(&mut self, code: Code, m: Mode) {
        self.err = None;
        match m {
            Mode::Normal => {
                self.model.reset_key(code);
                self.want = None;
                self.choosing = false;
                self.save();
            }
            Mode::Remap => {
                self.want = Some(Mode::Remap);
                self.choosing = self.model.remap_of(code).is_none();
                if self.model.remap_of(code).is_none() {
                    // a key can't be both: the choice is made, its old kind goes when the new key is chosen
                }
            }
            Mode::Action => {
                self.want = Some(Mode::Action);
                self.choosing = false;
                if self.model.binds.get(code).is_none_or(|b| !matches!(b, Bind::Preset(_))) {
                    self.set_preset(code, Preset::PlayPause);
                }
            }
            Mode::Macro => {
                self.want = Some(Mode::Macro);
                self.choosing = false;
                if self.model.binds.get(code).is_none_or(|b| !matches!(b, Bind::Macro(_))) {
                    if let Some(id) = self.model.macros.first().map(|m| m.id.clone()) {
                        match self.model.set_macro(code, &id) {
                            Ok(()) => self.save(),
                            Err(e) => self.err = Some(e),
                        }
                    }
                }
            }
        }
    }

    /// A key was clicked on the picture.
    fn key_clicked(&mut self, n: usize, cx: &mut Cx) {
        let Some(k) = self.keys.get(n % ISO_LOWER).cloned() else { return };
        // the key's own window opens (a click on the open one's key can't happen: its dim covers the picture)
        self.select(Some(k.code));
        self.key_at = cx.now;
    }

    /// The key pressed on the remap field: its scancode is the new key.
    fn target_pressed(&mut self, vk: u16) {
        let Some(from) = self.sel else { return };
        let Some(code) = bu_keysound::layout::code_of_vk(vk) else { return };
        match self.model.set_remap(from, code) {
            Ok(()) => {
                self.choosing = false;
                self.err = None;
                self.save();
            }
            Err(e) => self.err = Some(e),
        }
    }

    fn remap_text(maps: &[Mapping]) -> String {
        if maps.is_empty() {
            "-".into()
        } else {
            maps.iter().map(|m| format!("{:x}:{:x}", m.from, m.to)).collect::<Vec<_>>().join(",")
        }
    }

    fn start_apply(&mut self, cx: &mut Cx) {
        let maps = self.model.pending.clone();
        let r = cx.start_job(JOB_APPLY, move |job| {
            job.status("Waiting for Windows…");
            use crate::admin::{AdminError, Op, Purpose};
            match crate::admin::client::admin().call(Purpose::Keyboard, Op::ScancodeMap { maps }) {
                Ok(_) => Ok("applied".to_string()),
                Err(AdminError::Declined) => Err(crate::jobs::JobError::Failed(crate::admin::NOT_CHANGED.to_string())),
                Err(e) => Err(crate::jobs::JobError::Failed(e.to_string())),
            }
        });
        if let Err(e) = r {
            cx.toast(&format!("Not applied: {e:?}"));
        }
    }

    /// The apply job ended: what Windows has now, the undo record, the toast.
    fn follow_job(&mut self, cx: &mut Cx) {
        let Some(v) = cx.job(JOB_APPLY) else { return };
        let Some(end) = v.end.clone() else { return };
        if !first_end(JOB_APPLY, v.id) {
            return;
        }
        match end {
            crate::jobs::End::Done(_) => {
                let before = self.model.applied.clone();
                let now = self.model.pending.clone();
                self.model.mark_applied();
                self.restart = true;
                glue::set_restart_pending();
                cx.record(
                    "remap",
                    "Key remaps",
                    &crate::undo::Val::new(&Self::remap_text(&before), &self.describe_maps(&before)),
                    &crate::undo::Val::new(&Self::remap_text(&now), &self.describe_maps(&now)),
                );
                cx.toast("Remaps saved · restart Windows to use them");
            }
            crate::jobs::End::Failed(e) => cx.toast(&e),
            crate::jobs::End::Stopped => {}
        }
    }

    fn describe_maps(&self, maps: &[Mapping]) -> String {
        if maps.is_empty() {
            return "No remaps".into();
        }
        maps.iter()
            .map(|m| format!("{} → {}", self.label_of(m.from), if m.to == remap::DISABLED { "Disabled".to_string() } else { self.label_of(m.to) }))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn change_size(&mut self, s: Size) {
        self.prefs.size = s;
        self.select(None);
        self.rebuild_keys();
        self.save();
    }
}

impl Page for Keyboard {
    fn id(&self) -> &'static str {
        "kbd"
    }
    fn name(&self) -> &'static str {
        "Keyboard"
    }
    fn icon(&self) -> &'static str {
        "kbd"
    }
    fn open(&mut self, env: &Env, now: f64) {
        *self = Keyboard::default();
        self.opened_at = now;
        self.load(env);
    }
    fn close(&mut self) {
        gallery::OPEN.store(false, std::sync::atomic::Ordering::SeqCst);
        // nothing typed on this page is kept; the engine and the keys go on (they are the switches', not the tab's)
        *self = Keyboard::default();
    }
    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        if !self.loaded {
            self.load(&Env { test: true, ..Env::default() });
        }
        self.follow_job(cx);
        self.follow_get(cx);
        self.follow_preview(cx);
        self.view(cx)
    }
    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        self.popups(cx)
    }
    fn popup_dismiss(&mut self) {
        if self.ask_del.is_some() {
            self.ask_del = None;
        } else if self.get_open {
            self.get_open = false;
            gallery::OPEN.store(false, std::sync::atomic::Ordering::SeqCst);
            // a preview still going stops too (the window closed some other way than Done)
            crate::services::with(|s| {
                if let Some(v) = s.jobs.view_key(gallery::JOB_PREVIEW) {
                    s.jobs.stop(v.id);
                }
            });
        } else if self.pop.is_some() {
            self.pop = None;
        } else if self.edit.is_some() {
            self.close_editor();
        } else if self.sel.is_some() {
            self.select(None);
        }
    }
    fn start(&self, s: &mut crate::services::Services) {
        if s.test {
            return; // test copies never register raw input, keys or a stream
        }
        glue::start(s);
    }
    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        self.handle(ev, cx);
    }
    fn describe(&self) -> String {
        format!(
            "on={} pack={} vol={} size={} sel={:?} remaps={} binds={} macros={} dirty={}",
            self.prefs.on,
            self.prefs.s.pack.to_key(),
            self.prefs.s.volume,
            self.prefs.size.key(),
            self.sel,
            self.model.remap_count(),
            self.model.bind_count(),
            self.model.macros.len(),
            self.model.dirty()
        )
    }
    fn resettable(&mut self) -> Option<&mut dyn crate::undo::Resettable> {
        Some(self)
    }
}

impl crate::undo::Resettable for Keyboard {
    fn page_id(&self) -> &str {
        "kbd"
    }
    fn page_title(&self) -> &str {
        "Keyboard"
    }
    fn current(&self, item: &str) -> Option<crate::undo::Val> {
        if item != "remap" || self.test {
            return None;
        }
        let maps = remap::real::read().ok()?;
        Some(crate::undo::Val::new(&Self::remap_text(&maps), &self.describe_maps(&maps)))
    }
    fn windows_defaults(&self) -> Vec<crate::undo::DefaultItem> {
        // Windows' own state: no key remapped
        let now = self.current("remap").unwrap_or_else(|| crate::undo::Val::new("-", "No remaps"));
        vec![crate::undo::DefaultItem { item: "remap".into(), label: "Key remaps".into(), now, default: crate::undo::Val::new("-", "No remaps") }]
    }
    fn apply(&mut self, item: &str, to: &crate::undo::Val) -> Result<(), String> {
        if item != "remap" {
            return Err("unknown item".into());
        }
        let maps = parse_remaps(&to.raw)?;
        use crate::admin::{Op, Purpose};
        crate::admin::client::admin().call(Purpose::Reset, Op::ScancodeMap { maps: maps.clone() }).map_err(|e| e.to_string())?;
        self.model.applied = maps.clone();
        self.model.pending = maps;
        self.restart = true;
        glue::set_restart_pending();
        Ok(())
    }
}

/// `3a:1,e038:e01d` / `-` back into remaps.
fn parse_remaps(s: &str) -> Result<Vec<Mapping>, String> {
    if s == "-" {
        return Ok(Vec::new());
    }
    s.split(',')
        .map(|p| {
            let (a, b) = p.split_once(':').ok_or("bad remap")?;
            Ok(Mapping { from: u16::from_str_radix(a, 16).map_err(|_| "bad remap")?, to: u16::from_str_radix(b, 16).map_err(|_| "bad remap")? })
        })
        .collect()
}

