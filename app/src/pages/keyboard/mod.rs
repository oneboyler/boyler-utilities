//! The Keyboard tab (Order 058, rebuilt in Order 090 to keyboard-v8.html): the keyboard FIRST - click a key = its window
//! (Normal / Remap / Action / Macro with the macro's steps right there + its Sound: the pack's sound and your own on top);
//! drag a box, Ctrl- or Shift-click = several keys ("N keys" window: one sound for all, pitch and loudness Same / Rising /
//! A little random). Then the "Keyboard sounds" card, folded like Mouse acceleration: Sound (the packs, yours, downloaded or
//! imported, Get more sounds…, Import…, Make a pack from one sound…), Volume, Play on, More (Ignore repeats within 0-400 ms,
//! Off while a game is in front, Try it, Different in some apps). Macros are made and edited only in a key's window.
//! Engine: `crates/keysound` (bu-keysound); the always-on part is `glue.rs`, what is remembered `prefs.rs`, what the picture
//! edits `model.rs`, the keyboard drawing `pic.rs`, the page's boxes `view.rs`, the shared window parts `pages::btnwin`.
//!
//! Key sounds are OFF by default and while off nothing listens. The pressed key is used only to pick its sound and is not
//! kept. A remap is Windows' own key map (one admin Yes, a restart); a key's action or macro works at once through the keys
//! manager, and is left alone while a game or a full-screen window is in front.

use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, sub, El, Key};
use crate::ui::pieces::nbox::{self, Filter};
use bu_keysound::binds::{Bind, Preset};
use bu_keysound::layout::Layout;
use bu_keysound::remap::{self, Code, Mapping};
use bu_keysound::{Pack, PackId, Rule};

mod detect;
pub mod gallery;
mod getter;
pub mod glue;
pub mod model;
pub mod pic;
pub mod prefs;
mod view;
pub use view::combo_of;
#[cfg(test)]
mod tests;

use model::{Mode, Model};
use prefs::{Prefs, Size};

const K_ON: Key = key("kbd.on");
const K_PACK: Key = key("kbd.pack");
const K_PLAY: Key = key("kbd.play");
const K_VOL: Key = key("kbd.vol");
const K_REP: Key = key("kbd.rep");
const K_GAME: Key = key("kbd.game");
const K_TRY: Key = key("kbd.try");
/// Order 090: the Keyboard sounds card (folds like Mouse acceleration), its header and chevron.
const K_CARD: Key = key("kbd.card");
const K_CARDH: Key = key("kbd.cardh");
const K_CHEV: Key = key("kbd.chev");
/// The picture's background (a press there starts a box), the picking line's Clear / Change N keys.
const K_PICBG: Key = key("kbd.picbg");
const K_CLR: Key = key("kbd.clr");
const K_CHG: Key = key("kbd.chg");
/// The key window's Sound part and macro editor (pages::btnwin), the "N keys" window.
const K_SND: Key = key("kbd.snd");
const K_MAC: Key = key("kbd.mac");
const K_KN: Key = key("kbd.kn");
/// "Make a pack from one sound": the window, its two file boxes (+ their ×), the variation, the picture, the name, Save.
const K_MP: Key = key("kbd.mp");
const K_MPP: Key = key("kbd.mpp");
const K_MPR: Key = key("kbd.mpr");
const K_MPV: Key = key("kbd.mpv");
const K_MPNAME: Key = key("kbd.mpname");
const K_MPSAVE: Key = key("kbd.mpsave");
/// The width of the choice + slider column of the pack maker's Pitch / Loudness rows (pack-noise-v1).
const MAKE_VARY_W: f32 = 330.0;
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
const KD_W: f32 = 440.0;
const K_MODE: Key = key("kbd.mode");
const K_TARGET: Key = key("kbd.target");
const K_TPICK: Key = key("kbd.tpick");
const K_ACT: Key = key("kbd.act");
const K_ATEXT: Key = key("kbd.atext");
const K_ABROWSE: Key = key("kbd.abrowse");
const K_RESETKEY: Key = key("kbd.resetkey");
const K_APPLY: Key = key("kbd.apply");
const K_MENU: Key = key("kbd.menu");
const K_RESET: Key = key("kbd.reset");
const K_PLAYON: Key = key("kbd.playon");
/// The Get-more-sounds window: its search box and a row's play button.
const K_GSEARCH: Key = key("kbd.gsearch");
const K_GPLAY: Key = key("kbd.gplay");
/// The "Remove this sound?" question (right-click on an imported or downloaded sound in the list).
const K_DELQ: Key = key("kbd.delq");

/// The job that writes the remaps (the admin prompt never holds the menu).
const JOB_APPLY: &str = "kbd.apply";
/// Order 096: which keyboard is plugged in (picks the picture size).
const JOB_DETECT: &str = "kbd.detect";
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
}

/// What a row of the open list means.
#[derive(Debug, Clone, PartialEq)]
enum Choice {
    Heading,
    /// A separator line.
    Sep,
    Pack(Pack),
    PackOff,
    GetMore,
    Import,
    /// Order 090: "Make a pack from one sound…".
    MakePack,
    Target(Code),
    Preset(Preset),
    AppAction(String),
}

/// A press on the picture that may become a box (Order 090: pick several keys like files on the desktop).
#[derive(Debug, Clone, PartialEq)]
struct Band {
    /// The picture's top-left in window px.
    origin: (f32, f32),
    start: (f32, f32),
    now: (f32, f32),
    /// The picked keys when it started (Ctrl / Shift add to them).
    base: Vec<Code>,
    add: bool,
    moved: bool,
}

/// "Make a pack from one sound" while its window is open.
#[derive(Debug, Clone)]
struct MakePack {
    /// (full path of the picked file, its name)
    press: Option<(String, String)>,
    release: Option<(String, String)>,
    vary: bu_keysound::Vary,
    name: String,
    /// The name was typed (a new press file no longer renames it).
    named: bool,
    opened: f64,
    msg: Option<String>,
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
    /// "Remove this sound?" is asked for this imported / downloaded / made pack, at this place.
    ask_del: Option<(Pack, (f32, f32))>,
    /// Order 090: the keys picked for "Change N keys" (in picking order), the key a Shift-click spans from, a box being drawn.
    pick: Vec<Code>,
    anchor: Option<Code>,
    band: Option<Band>,
    /// The "N keys" window is open (for `pick`), since when.
    many: bool,
    many_at: f64,
    /// The key / N keys window's Sound part and the key window's macro editor.
    snd: crate::pages::btnwin::SoundEd,
    med: crate::pages::btnwin::MacroEd,
    /// The Keyboard sounds card is open (starts folded; switching on opens it).
    card_open: bool,
    /// "Make a pack from one sound" and its press / release files decoded (to hear them).
    make: Option<MakePack>,
    make_clips: (Option<bu_keysound::import::Clip>, Option<bu_keysound::import::Clip>),
    /// Which pack the open list's right-click asked about (for the made packs, whose rows aren't Imported).
    made_list: Vec<String>,
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

    /// The packs made from one sound (Order 090), by name.
    fn made(&self) -> Vec<String> {
        let dir = crate::services::with(|s| glue::packs_dir(s.store.folder()));
        match (self.test, dir) {
            (false, Some(d)) => glue::made_list(&d),
            _ => self.made_list.clone(),
        }
    }

    fn pack_name(&self, p: &Pack) -> String {
        match p {
            Pack::Builtin(id) => id.name().to_string(),
            Pack::Imported(n) | Pack::Made(n) => n.clone(),
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
        // Order 090 (v8): built-in · Your packs · Downloaded or imported · Get more sounds… · Import… · Make a pack from one sound…
        let packs = |v: &mut Vec<(String, Choice, bool)>, cur: Option<&Pack>, with_import: bool| {
            v.push(("Keyboard".into(), Choice::Heading, false));
            for id in PackId::ALL.iter().filter(|p| p.is_keyboard()) {
                v.push((id.name().into(), Choice::Pack(Pack::Builtin(*id)), cur == Some(&Pack::Builtin(*id))));
            }
            v.push(("Satisfying".into(), Choice::Heading, false));
            for id in PackId::ALL.iter().filter(|p| !p.is_keyboard()) {
                v.push((id.name().into(), Choice::Pack(Pack::Builtin(*id)), cur == Some(&Pack::Builtin(*id))));
            }
            let made = self.made();
            if !made.is_empty() {
                v.push(("Your packs \u{b7} right-click to remove".into(), Choice::Heading, false));
                for n in made {
                    let p = Pack::Made(n.clone());
                    v.push((n, Choice::Pack(p.clone()), cur == Some(&p)));
                }
            }
            let imp = self.imported();
            if !imp.is_empty() {
                v.push(("Downloaded or imported \u{b7} right-click to remove".into(), Choice::Heading, false));
                for n in imp {
                    let p = Pack::Imported(n.clone());
                    v.push((n, Choice::Pack(p.clone()), cur == Some(&p)));
                }
            }
            if with_import {
                v.push((String::new(), Choice::Sep, false));
                v.push(("Get more sounds\u{2026}".into(), Choice::GetMore, false));
                v.push(("Import\u{2026}".into(), Choice::Import, false));
                v.push(("Make a pack from one sound\u{2026}".into(), Choice::MakePack, false));
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
            Choice::Heading | Choice::Sep => {}
            Choice::MakePack => {
                self.pop = None;
                self.open_make(cx.now);
            }
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

    /// Plays pack `p` as the A key would (press + release as "Play on" says - E21: the preview follows Play on). Works with
    /// the sounds off; an imported / made pack is read first when the engine doesn't hold it yet.
    fn preview(&self, p: &Pack) {
        if self.test {
            return;
        }
        self.load_pack(p);
        let none = bu_keysound::Layers::new();
        let mouse = |_: u16| None;
        let pad = |_: u16| None;
        let h = crate::pages::btnwin::HearOf { dev: bu_keysound::Dev::Keys, pack: Some(p), volume: self.prefs.s.volume, play_on: self.prefs.s.play_on, mouse: &mouse, pad: &pad };
        crate::pages::btnwin::hear(&h, &none, &[0x1E]);
    }

    /// Makes sure the engine holds pack `p`'s sounds (an imported / made one is read safely from its cache).
    fn load_pack(&self, p: &Pack) {
        if self.test {
            return;
        }
        let Some(dir) = crate::services::with(|s| glue::packs_dir(s.store.folder())) else { return };
        let e = glue::engine();
        match p {
            Pack::Imported(n) if e.pack_sound(bu_keysound::Dev::Keys, p, 0x1E, true, None, None).is_none() => {
                if let Ok(x) = bu_keysound::safe::read_installed(&dir, n) {
                    e.set_imported(n, Some(x.set));
                }
            }
            Pack::Made(n) if e.pack_sound(bu_keysound::Dev::Keys, p, 0x1E, true, None, None).is_none() => {
                if let Ok(m) = glue::read_made(&dir, n) {
                    e.set_made(n, Some(m));
                }
            }
            _ => {}
        }
    }

    /// "Import a pack…": ONE picker for the .zip as the site hands it out or a pack folder (a folder is picked by the
    /// config.json inside it: a file dialog can't pick both kinds). A clear line when it isn't a pack.
    fn import_pack(&mut self, cx: &mut Cx) {
        let picked = cx.pick_file("Pick a Mechvibes pack: its .zip, or the config.json inside its folder", &[("Mechvibes pack (.zip or config.json)", "*.zip;config.json")]);
        let Some(src) = picked else { return };
        let dir = crate::services::with(|s| glue::packs_dir(s.store.folder()));
        let Some(dir) = dir else { return };
        // Order 090 (E21): read through the helper copy - a broken pack says why and never crashes the app
        match bu_keysound::safe::install_any(std::path::Path::new(&src), &dir) {
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
                    if let Some((_, Choice::Pack(pk @ (Pack::Imported(_) | Pack::Made(_))), _)) = self.list(p).get(i).cloned() {
                        self.ask_del = Some((pk, (x, y)));
                    }
                }
                return;
            }
        }
    }

    /// The pack is gone from the PC: the settings that used it go back to a sound that exists (the general sound to Linear,
    /// a program's own sound to Off) and the engine lets its sounds go. Pure state; the files are `remove_pack`.
    fn forget_pack(&mut self, gone: &Pack) -> bool {
        let gone = gone.clone();
        let was_in_use = self.prefs.s.pack == gone;
        if was_in_use {
            self.prefs.s.pack = Pack::Builtin(bu_keysound::PackId::Linear);
        }
        for r in self.prefs.s.rules.iter_mut().filter(|r| r.pack.as_ref() == Some(&gone)) {
            r.pack = None;
        }
        if self.prefs.s.pad_pack.as_ref() == Some(&gone) {
            self.prefs.s.pad_pack = None;
        }
        was_in_use
    }

    /// "Remove" was confirmed: the pack's folder is deleted, the settings that used it fall back.
    fn remove_pack(&mut self, pack: &Pack, cx: &mut Cx) {
        let dir = crate::services::with(|s| glue::packs_dir(s.store.folder()));
        let name = self.pack_name(pack);
        if !self.test {
            let Some(dir) = dir else { return };
            let r = match pack {
                Pack::Made(n) => glue::remove_made(&dir, n),
                _ => bu_keysound::import::remove(&dir, &name),
            };
            if let Err(e) = r {
                cx.toast(&format!("Not removed: {e}"));
                return;
            }
            match pack {
                Pack::Made(n) => glue::engine().set_made(n, None),
                _ => glue::engine().set_imported(&name, None),
            }
        } else {
            self.made_list.retain(|n| *n != name);
        }
        let was_in_use = self.forget_pack(pack);
        self.save();
        cx.toast(&if was_in_use { format!("{name} removed · the sound is Linear again") } else { format!("{name} removed") });
    }

    fn select(&mut self, code: Option<Code>) {
        self.sel = code;
        self.want = None;
        self.choosing = false;
        self.err = None;
        self.pop = None;
        self.snd.reset();
        self.med.reset();
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

    /// The keyboard detection ended: a known keyboard sets the picture size (a size picked by hand is never touched).
    fn follow_detect(&mut self) {
        let Some(v) = crate::services::with(|s| s.job(JOB_DETECT)).flatten() else { return };
        let Some(crate::jobs::End::Done(text)) = v.end.clone() else { return };
        if !first_end(JOB_DETECT, v.id) || self.prefs.size_manual || text.is_empty() {
            return;
        }
        let size = Size::from_key(&text);
        if size != self.prefs.size {
            self.prefs.size = size;
            self.select(None);
            self.rebuild_keys();
            crate::services::with(|s| self.prefs.save(&mut s.store));
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
        self.prefs.size_manual = true;
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
        // Order 096: which keyboard is plugged in (read-only, off the UI thread) picks the picture size, unless it was picked by hand
        if !env.fake() && !self.prefs.size_manual {
            crate::services::with(|s| s.start_job(JOB_DETECT, |_| Ok(detect::detect_text())).ok());
        }
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
        self.follow_detect();
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
        } else if self.snd.menu_open() || self.med.menu_open() {
            self.snd.escape();
            self.med.escape();
        } else if self.make.is_some() {
            self.make = None;
        } else if self.many {
            self.many = false;
            self.snd.reset();
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
            "on={} card={} pick={} many={} layers={} pack={} vol={} size={} sel={:?} remaps={} binds={} macros={} dirty={}",
            self.prefs.on,
            self.card_open,
            self.pick.len(),
            self.many,
            self.prefs.keys.len(),
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

