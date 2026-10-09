//! The always-on part of the Keyboard tab: the process's ONE key-sound engine, the keys that carry a macro or an action (made
//! known to the keys manager so they work with the menu closed), and what happens while a game is in front. Not part of the
//! tab's UI; costs nothing while nothing is on: no key sounds listening, no key carrying anything, no watcher.

use super::prefs::Prefs;
use crate::keys::{Action, Combo, Mods};
use crate::services::Services;
use bu_keysound::binds::{Bind, Binds};
use bu_keysound::layout::{self, Layout};
use bu_keysound::macros::Macro;
use bu_keysound::remap::Code;
use bu_keysound::{import, send, KeySounds, Pack};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

static ENGINE: OnceLock<KeySounds> = OnceLock::new();

/// The engine (made on first use; making it opens nothing and starts nothing).
pub fn engine() -> &'static KeySounds {
    ENGINE.get_or_init(KeySounds::new)
}

/// Where imported Mechvibes packs live: `<settings folder>\keysounds`.
pub fn packs_dir(settings_folder: &Path) -> PathBuf {
    settings_folder.join("keysounds")
}

/// Brings the engine to `prefs`: on = the sounds are made, the key sink is registered; off = nothing is registered, the
/// thread ends, the sounds are dropped. Imported packs the settings use are read from `dir` first (one that can't be read
/// is left out: its keys stay silent). Err = Windows' own refusal text.
pub fn apply(prefs: &Prefs, dir: &Path) -> Result<(), String> {
    if !prefs.on {
        engine().disable();
        return Ok(());
    }
    let mut want: Vec<&String> = Vec::new();
    for p in std::iter::once(&prefs.s.pack).chain(prefs.s.rules.iter().filter_map(|r| r.pack.as_ref())) {
        if let Pack::Imported(n) = p {
            if !want.contains(&n) {
                want.push(n);
            }
        }
    }
    for n in want {
        match import::read_installed(dir, n) {
            Ok(p) => engine().set_imported(n, Some(p.set)),
            Err(_) => engine().set_imported(n, None),
        }
    }
    engine().enable(prefs.s.clone())
}

// ------------------------------------------------------------------ keys that carry a macro or an action

/// What the keys run when pressed (the handlers read it; the page publishes it on every change).
struct Live {
    binds: Binds,
    macros: Vec<Macro>,
    /// How the last press ended ("Not run: a game or full-screen window is in front", …) - the page shows it.
    last: Option<String>,
}

static LIVE: Mutex<Live> = Mutex::new(Live { binds: Binds::new(), macros: Vec::new(), last: None });
/// A game / full-screen window is in front (the watcher's last word).
static GAME: AtomicBool = AtomicBool::new(false);
/// The watcher said something new and the keys wait for the UI thread to follow it.
static GAME_CHANGED: AtomicBool = AtomicBool::new(false);
static WATCH: Mutex<Option<bu_keysound::watch::GameWatch>> = Mutex::new(None);

fn live() -> std::sync::MutexGuard<'static, Live> {
    LIVE.lock().unwrap_or_else(|p| p.into_inner())
}

/// The id of the keys manager's action for the key at `code`.
pub fn action_id(code: Code) -> String {
    format!("kbd.key.{code:X}")
}

fn code_of_action(id: &str) -> Option<Code> {
    id.strip_prefix("kbd.key.").and_then(|h| u16::from_str_radix(h, 16).ok())
}

/// Hands the keys' bindings to the always-on handlers.
pub fn publish(binds: &Binds, macros: &[Macro]) {
    let mut l = live();
    l.binds = binds.clone();
    l.macros = macros.to_vec();
}

/// How the last key press ended (None = nothing yet / fine).
pub fn last_message() -> Option<String> {
    live().last.clone()
}

/// What a press of the key at `code` does now. Presets and macros run through bu-keysound's guarded senders: a macro never runs
/// while a game / full-screen window or an administrator window is in front.
pub fn run_key(code: Code) -> Result<(), String> {
    let (bind, mac) = {
        let l = live();
        let b = l.binds.get(code).cloned();
        let m = match &b {
            Some(Bind::Macro(id)) => l.macros.iter().find(|m| &m.id == id).cloned(),
            _ => None,
        };
        (b, m)
    };
    let r = match bind {
        Some(Bind::Preset(p)) => send::run_preset(&p),
        Some(Bind::Macro(_)) => match mac {
            Some(m) => send::run_macro(&m),
            None => Err("This key's macro is gone".into()),
        },
        _ => Ok(()),
    };
    live().last = r.as_ref().err().cloned();
    r
}

/// What the manager should say a key's action is called ("Keyboard: Caps Lock key").
fn action_name(code: Code, lay: &dyn Layout) -> String {
    format!("Keyboard: {} key", layout::label(lay, code))
}

/// The virtual key for the key at `code` on the user's layout.
fn vk_for(code: Code, lay: &dyn Layout) -> Option<u16> {
    lay.vk_of(code)
}

/// Makes the keys manager's actions for the keys that carry something match `binds`: each gets its action (so it shows in
/// Settings › All shortcuts) and its key (the key itself, no modifiers). Returns what the manager refused, by key: a numpad
/// key, a key another action holds ("Already used by Mic mute"), a key Windows won't give. Starts / stops the game watcher.
pub fn sync_keys(s: &mut Services, binds: &Binds, lay: &dyn Layout) -> Vec<(Code, String)> {
    let mut errs = Vec::new();
    let have: Vec<String> = s.keys.actions().filter(|a| a.id.starts_with("kbd.key.")).map(|a| a.id.clone()).collect();
    for id in &have {
        let keep = code_of_action(id).is_some_and(|c| matches!(binds.get(c), Some(Bind::Preset(_) | Bind::Macro(_))));
        if !keep {
            s.remove_action(id);
        }
    }
    for (code, bind) in binds.iter() {
        if !matches!(bind, Bind::Preset(_) | Bind::Macro(_)) {
            continue;
        }
        let id = action_id(code);
        if !s.has_action(&id) {
            s.add_action(Action::new(&id, &action_name(code, lay), "kbd"), move |down| {
                if down {
                    let _ = run_key(code);
                }
            });
        }
        let Some(vk) = vk_for(code, lay) else {
            errs.push((code, "This key has no place on your layout".to_string()));
            continue;
        };
        let combo = Combo::new(Mods::NONE, vk);
        if let Err(e) = s.keys.bind(&mut s.store, &id, combo) {
            errs.push((code, e.message()));
        }
    }
    let any = !binds.is_empty();
    set_watch(any);
    apply_game(s);
    errs
}

/// The watcher runs only while some key carries something.
fn set_watch(on: bool) {
    let mut w = WATCH.lock().unwrap_or_else(|p| p.into_inner());
    if on && w.is_none() {
        let cb: std::sync::Arc<dyn Fn(bool) + Send + Sync> = std::sync::Arc::new(|game| {
            GAME.store(game, Ordering::Release);
            GAME_CHANGED.store(true, Ordering::Release);
            crate::services::Waker.wake();
        });
        *w = bu_keysound::watch::GameWatch::start(cb).ok();
    } else if !on {
        if let Some(g) = w.take() {
            g.stop();
        }
        GAME.store(false, Ordering::Release);
    }
}

/// While a game / full-screen window is in front the keys go back to the game: the manager lets them go (its `set_active`
/// keeps them set and saved) and takes them again when the game leaves.
fn apply_game(s: &mut Services) {
    let game = GAME.load(Ordering::Acquire);
    let ids: Vec<String> = s.keys.actions().filter(|a| a.id.starts_with("kbd.key.")).map(|a| a.id.clone()).collect();
    for id in ids {
        s.keys.set_active(&id, !game);
    }
}

/// The main loop calls this at every wake-up: follow the watcher.
pub fn drain_pending() {
    if GAME_CHANGED.swap(false, Ordering::AcqRel) {
        crate::services::with(apply_game);
    }
}

/// App start (the menu may never open): the key sounds come back on if they were on, and the keys that carry something work.
pub fn start(s: &mut Services) {
    let prefs = Prefs::load(&s.store);
    publish(&prefs.binds, &prefs.macros);
    if !prefs.binds.is_empty() {
        let lay = layout::current();
        let _ = sync_keys(s, &prefs.binds, &lay);
    }
    if prefs.on {
        let dir = packs_dir(s.store.folder());
        let _ = apply(&prefs, &dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No test ever registers raw input or opens a stream: `apply` with the switch off is a no-op on the engine.
    #[test]
    fn off_changes_nothing_on_the_pc() {
        let p = Prefs::default();
        assert!(!p.on);
        let dir = std::env::temp_dir().join("bu-kb-glue-none");
        assert_eq!(apply(&p, &dir), Ok(()));
        assert!(!engine().status().enabled);
        assert_eq!(engine().status().plays, 0);
    }

    #[test]
    fn imported_packs_live_under_the_settings_folder() {
        assert_eq!(packs_dir(Path::new(r"C:\x\Boyler Utilities")), Path::new(r"C:\x\Boyler Utilities\keysounds"));
    }

    #[test]
    fn action_ids_round_trip() {
        assert_eq!(action_id(0xE038), "kbd.key.E038");
        assert_eq!(code_of_action("kbd.key.E038"), Some(0xE038));
        assert_eq!(code_of_action("micmute.toggle"), None);
    }
}
