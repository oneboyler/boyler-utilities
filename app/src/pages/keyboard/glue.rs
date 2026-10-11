//! The always-on part of the Keyboard tab: the process's ONE key-sound engine, the keys that carry a macro or an action (made
//! known to the keys manager so they work with the menu closed), and what happens while a game is in front. Not part of the
//! tab's UI; costs nothing while nothing is on: no key sounds listening, no key carrying anything, no watcher.

use super::prefs::Prefs;
use crate::keys::{Action, Combo, Mods};
use crate::services::Services;
use bu_keysound::binds::{Bind, Binds, Preset};
use bu_keysound::layout::{self, Layout};
use bu_keysound::macros::{Macro, Repeat, Step};
use bu_keysound::remap::Code;
use bu_keysound::{safe, send, Dev, KeySounds, Made, MadeSet, Pack};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

static ENGINE: OnceLock<KeySounds> = OnceLock::new();
/// The Key sounds switch as the last `apply` saw it (a worker thread can not reach the settings, which live on the UI thread).
static SOUNDS_ON: AtomicBool = AtomicBool::new(false);
/// Remaps were written this run: Windows needs a restart for them (the page is made new at every open, this is not).
static RESTART: AtomicBool = AtomicBool::new(false);

pub fn restart_pending() -> bool {
    RESTART.load(Ordering::Acquire)
}

pub fn set_restart_pending() {
    RESTART.store(true, Ordering::Release);
}

/// Are any of the sounds (keys, mouse, controller) switched on (as of the last change)?
pub fn sounds_on() -> bool {
    SOUNDS_ON.load(Ordering::Acquire)
}

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
    let s = prefs.engine_settings();
    SOUNDS_ON.store(s.any_on(), Ordering::Release);
    if !s.any_on() {
        engine().disable();
        return Ok(());
    }
    // Order 090 (E21): imported / downloaded packs are read through the safe decoder (a broken one is left out, silent)
    for p in s.packs() {
        match &p {
            Pack::Imported(n) => match safe::read_installed(dir, n) {
                Ok(p) => engine().set_imported(n, Some(p.set)),
                Err(_) => engine().set_imported(n, None),
            },
            Pack::Made(n) => engine().set_made(n, read_made(dir, n).ok()),
            Pack::Builtin(_) => {}
        }
    }
    engine().set_layers(Dev::Keys, prefs.keys.clone());
    engine().set_layers(Dev::Mouse, prefs.mouse.clone());
    engine().set_layers(Dev::Pad, prefs.pad.clone());
    load_clips(&yours_dir(dir), &prefs.own_files());
    engine().enable(s)
}

// ------------------------------------------------------------------ "your sounds" and packs made from one sound (Order 090)

/// The files of "your sound": `<packs>\yours`.
pub fn yours_dir(packs: &Path) -> PathBuf {
    packs.join("yours")
}

/// Hands the engine every file in `ids` it doesn't hold yet (from their caches; a missing / broken file stays silent).
pub fn load_clips(dir: &Path, ids: &[String]) {
    let have = engine().clip_ids();
    for id in ids {
        if have.contains(id) || !bu_keysound::layers::file_id_ok(id) {
            continue;
        }
        if let Ok(c) = safe::read_sound(&dir.join(id), true) {
            engine().set_clip(id, Some(c));
        }
    }
}

/// Packs made from one sound live in `<packs>\made\<name>\` (pack.txt + its one or two files).
pub fn made_dir(packs: &Path) -> PathBuf {
    packs.join("made")
}

/// The made packs' names (their folders), sorted.
pub fn made_list(packs: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(made_dir(packs))
        .map(|d| d.filter_map(|e| e.ok()).filter(|e| e.path().join("pack.txt").is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    v.sort_by_key(|s| s.to_lowercase());
    v
}

/// A made pack's description.
pub fn read_made_spec(packs: &Path, name: &str) -> Result<Made, String> {
    if bu_keysound::import::folder_name(name) != name {
        return Err("not a pack name".into());
    }
    let t = std::fs::read_to_string(made_dir(packs).join(name).join("pack.txt")).map_err(|_| "the pack is gone".to_string())?;
    Made::from_text(&t)
}

/// A made pack, ready to play: its description + its files decoded (safely, from their caches).
pub fn read_made(packs: &Path, name: &str) -> Result<MadeSet, String> {
    let m = read_made_spec(packs, name)?;
    let dir = made_dir(packs).join(name);
    let press = safe::read_sound(&dir.join(&m.press), true)?;
    let release = match &m.release {
        Some(r) => Some(safe::read_sound(&dir.join(r), true)?),
        None => None,
    };
    Ok(MadeSet::new(&m, &press, release.as_ref()))
}

/// Saves a pack made from one sound: a free folder named after it, the press (and release) file copied in as `press.<ext>`
/// (`release.<ext>`), pack.txt. Returns the pack's name (= its folder).
pub fn save_made(packs: &Path, mut m: Made, press_src: &Path, release_src: Option<&Path>) -> Result<String, String> {
    let root = made_dir(packs);
    std::fs::create_dir_all(&root).map_err(|e| format!("the packs folder: {e}"))?;
    let base = bu_keysound::import::folder_name(m.name.trim());
    let base = if base == "Imported" && m.name.trim().is_empty() { "My pack".to_string() } else { base };
    let mut name = base.clone();
    let mut n = 2;
    while root.join(&name).exists() {
        name = format!("{base} {n}");
        n += 1;
    }
    let dir = root.join(&name);
    std::fs::create_dir_all(&dir).map_err(|e| format!("the pack folder: {e}"))?;
    let ext = |p: &Path| p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_else(|| "wav".into());
    let mut write = || -> Result<(), String> {
        m.press = format!("press.{}", ext(press_src));
        std::fs::copy(press_src, dir.join(&m.press)).map_err(|e| format!("the press file: {e}"))?;
        m.release = match release_src {
            Some(r) => {
                let f = format!("release.{}", ext(r));
                std::fs::copy(r, dir.join(&f)).map_err(|e| format!("the release file: {e}"))?;
                Some(f)
            }
            None => None,
        };
        m.name = name.clone();
        std::fs::write(dir.join("pack.txt"), m.to_text()).map_err(|e| format!("pack.txt: {e}"))
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(name)
}

/// Removes a made pack's folder.
pub fn remove_made(packs: &Path, name: &str) -> Result<(), String> {
    if bu_keysound::import::folder_name(name) != name || !made_dir(packs).join(name).join("pack.txt").is_file() {
        return Err("no such pack".into());
    }
    std::fs::remove_dir_all(made_dir(packs).join(name)).map_err(|e| format!("removing {name}: {e}"))
}

// ------------------------------------------------------------------ keys that carry a macro or an action

/// What the keys run when pressed (the handlers read it; the page publishes it on every change).
struct Live {
    binds: Binds,
    /// What the mouse's wheel click / side buttons do besides their own job (by button number), Order 090.
    mouse: Binds,
    macros: Vec<Macro>,
    /// How the last press ended ("Not run: a game or full-screen window is in front", …) - the page shows it.
    last: Option<String>,
}

static LIVE: Mutex<Live> = Mutex::new(Live { binds: Binds::new(), mouse: Binds::new(), macros: Vec::new(), last: None });
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

/// Hands the mouse buttons' jobs to the always-on handlers (Order 090).
pub fn publish_mouse(binds: &Binds) {
    live().mouse = binds.clone();
}

/// How the last key press ended (None = nothing yet / fine).
pub fn last_message() -> Option<String> {
    live().last.clone()
}

/// What a press (`down`) or release of the key at `code` does now. Presets and macros run through bu-keysound's guarded
/// senders: a macro never runs while a game / full-screen window or an administrator window is in front. A macro's Repeat
/// decides what the release does (Order 090: "While the key is held" stops there).
pub fn run_key(code: Code, down: bool) -> Result<(), String> {
    let (bind, mac) = {
        let l = live();
        let b = l.binds.get(code).cloned();
        let m = match &b {
            Some(Bind::Macro(id)) => l.macros.iter().find(|m| &m.id == id).cloned(),
            _ => None,
        };
        (b, m)
    };
    let r = run_bind(bind, mac, down);
    if down {
        live().last = r.as_ref().err().cloned();
    }
    r
}

fn run_bind(bind: Option<Bind>, mac: Option<Macro>, down: bool) -> Result<(), String> {
    match bind {
        Some(Bind::Macro(_)) => match mac {
            Some(m) => send::press_macro(&m, down),
            None if down => Err("This key's macro is gone".into()),
            None => Ok(()),
        },
        _ if !down => Ok(()),
        Some(Bind::Preset(Preset::NextOutput)) => {
            next_output();
            Ok(())
        }
        Some(Bind::Preset(p)) => send::run_preset(&p),
        // a mouse button's "Also press": a key / combo or a mouse button, sent like a one-step macro (never into a game)
        Some(Bind::Also(k)) => {
            let mut m = Macro::new("also", "Also press");
            m.steps = vec![Step::Keys(k)];
            send::run_macro(&m)
        }
        Some(Bind::AlsoClick(b)) => {
            let mut m = Macro::new("also", "Also press");
            m.steps = vec![Step::Click(b)];
            send::run_macro(&m)
        }
        _ => Ok(()),
    }
}

/// What a press of mouse button `slot` (2 wheel click, 3 back, 4 forward) does besides its own job (Order 090).
pub fn run_mouse(slot: u16) -> Result<(), String> {
    // a click this app sent a moment ago (another button's "Also press" or a macro's Click) is not the user's press
    if send::sent_click_within(150) {
        return Ok(());
    }
    let (bind, mut mac) = {
        let l = live();
        let b = l.mouse.get(slot).cloned();
        let m = match &b {
            Some(Bind::Macro(id)) => l.macros.iter().find(|m| &m.id == id).cloned(),
            _ => None,
        };
        (b, m)
    };
    // Windows tells the app only that a mouse button went DOWN: "While the key is held" would never stop, so on a mouse
    // button it runs until the button is pressed again (the shared macro list can hand a Held macro to a mouse button)
    if let Some(m) = mac.as_mut() {
        if m.rep == Repeat::Held {
            m.rep = Repeat::Toggle;
        }
    }
    let r = run_bind(bind, mac, true);
    live().last = r.as_ref().err().cloned();
    r
}

/// The keys manager's action of mouse button `slot`.
pub fn mouse_action_id(slot: u16) -> String {
    format!("kbd.mouse.{slot}")
}

/// The mouse buttons the app can give a job: the wheel click and the two side buttons (Windows' mouse buttons 3 / 4 / 5 -
/// the keys manager listens to those; left / right click always keep only their own job).
pub const MOUSE_JOB_SLOTS: [u16; 3] = [2, 3, 4];

/// Makes the keys manager's actions for the mouse buttons that carry something match `binds` (as [`sync_keys`] does for keys).
pub fn sync_mouse(s: &mut Services, binds: &Binds) -> Vec<(u16, String)> {
    let mut errs = Vec::new();
    let have: Vec<String> = s.keys.actions().filter(|a| a.id.starts_with("kbd.mouse.")).map(|a| a.id.clone()).collect();
    for id in &have {
        let keep = id.strip_prefix("kbd.mouse.").and_then(|n| n.parse::<u16>().ok()).is_some_and(|n| binds.get(n).is_some());
        if !keep {
            s.remove_action(id);
        }
    }
    for (slot, _) in binds.iter() {
        if !MOUSE_JOB_SLOTS.contains(&slot) {
            continue;
        }
        let id = mouse_action_id(slot);
        if !s.has_action(&id) {
            let name = match slot {
                2 => "Mouse: wheel click",
                3 => "Mouse: back (side) button",
                _ => "Mouse: forward (side) button",
            };
            s.add_action(Action::new(&id, name, "mouse"), move |down| {
                if down {
                    let _ = run_mouse(slot);
                }
            });
        }
        if let Err(e) = s.keys.bind(&mut s.store, &id, Combo::mouse(Mods::NONE, (slot + 1) as u8)) {
            errs.push((slot, e.message()));
        }
    }
    set_watch(!binds.is_empty() || s.keys.actions().any(|a| a.id.starts_with("kbd.key.")));
    apply_game(s);
    errs
}

/// "Switch audio output": the next sound output that is switched on becomes Windows' default (all three roles, like picking
/// it in the Audio tab). Done on its own short-lived thread (Core Audio wants its own COM apartment); how it ended goes to
/// [`last_message`].
fn next_output() {
    let _ = std::thread::Builder::new().name("bu-next-output".into()).spawn(|| {
        let r = (|| -> Result<(), String> {
            let os = bu_audio::RealOs::new().map_err(|e| e.to_string())?;
            let mut s = bu_audio::AudioService::new(os);
            let rows = s.device_rows(bu_audio::Flow::Output).map_err(|e| e.to_string())?;
            let on: Vec<_> = rows.iter().filter(|r| r.on).collect();
            if on.len() < 2 {
                return Err("There is only one sound output switched on".into());
            }
            let at = on.iter().position(|r| r.current).unwrap_or(on.len() - 1);
            let next = on[(at + 1) % on.len()];
            s.select_default(bu_audio::Flow::Output, &next.device.id).map(|_| ()).map_err(|e| e.to_string())
        })();
        live().last = r.err();
    });
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
pub fn sync_keys(s: &mut Services, binds: &Binds, macros: &[Macro], lay: &dyn Layout) -> Vec<(Code, String)> {
    let mut errs = Vec::new();
    // a key whose action presses that same key would run itself and never type (Order 059): it is not registered at all, so
    // it types as Windows made it - also for such a bind an older version saved
    let (binds, _) = bu_keysound::binds::without_self_loops(binds, macros, &|c| lay.vk_of(c));
    let binds = &binds;
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
        // a macro that runs "while the key is held" needs the key's release too (watched through Raw Input)
        let held = matches!(bind, Bind::Macro(m) if macros.iter().any(|x| &x.id == m && x.rep == Repeat::Held));
        if s.has_action(&id) && s.keys.actions().any(|a| a.id == id && a.needs_release != held) {
            s.remove_action(&id);
        }
        if !s.has_action(&id) {
            let a = Action::new(&id, &action_name(code, lay), "kbd");
            s.add_action(if held { a.with_release() } else { a }, move |down| {
                let _ = run_key(code, down);
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
    let any = !binds.is_empty() || s.keys.actions().any(|a| a.id.starts_with("kbd.mouse."));
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
    let ids: Vec<String> = s.keys.actions().filter(|a| a.id.starts_with("kbd.key.") || a.id.starts_with("kbd.mouse.")).map(|a| a.id.clone()).collect();
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
    publish_mouse(&prefs.mouse_binds);
    if !prefs.binds.is_empty() {
        let lay = layout::current();
        let _ = sync_keys(s, &prefs.binds, &prefs.macros, &lay);
    }
    if !prefs.mouse_binds.is_empty() {
        let _ = sync_mouse(s, &prefs.mouse_binds);
    }
    if prefs.engine_settings().any_on() {
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
