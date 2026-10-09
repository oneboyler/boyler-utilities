//! The acceleration card's always-on part (Order 063). Before it, nothing in the app ran the card unless the Mouse tab was
//! open: the switch, the presets and the per-game rows were not kept, and nothing listened for a game starting - so the
//! per-game switch never happened at all.
//!
//! One thread ("bu-accel") owns one `Mouse<RealOs>` (the file `accel.json` is its card). It
//! - at app start loads the saved card and writes the driver ONLY when the user had acceleration ON and the driver does not
//!   already run it (`Mouse::start_accel`); a card that was off, or never saved, writes nothing;
//! - listens (`AppWatcher`, event-driven, no polling) for the games in the per-game rows - none listed = no watcher at all;
//! - on a game start / stop sets the driver to what the card says (only when the driver differs: its own READ decides - and never
//!   over a NEWER write of Raw Accel's own app: `Mouse::sync_driver_auto`, Order 077);
//! - a listed game that is ALREADY running (when added, at app start, when the switch goes on) gets its preset at once: a process
//!   snapshot finds it (`Mouse::adopt_running_games`), no handle to the game;
//! - sleeps otherwise (blocked on its channel: 0 % CPU).
//!
//! The Mouse tab does not write the driver itself on the real PC: it saves the card to the file and calls [`apply_now`],
//! which takes this engine's lock, reads the file and syncs - so the games that are running are never forgotten.
//! No handle is ever opened to a game (the watcher uses window events, a process list and SYNCHRONIZE-only waits).

use std::collections::HashSet;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use bu_mouse::accel::switch::{AppEvent, RowId};
use bu_mouse::win::watch::AppWatcher;
use bu_mouse::win::RealOs;
use bu_mouse::Mouse;

/// How long a game's EXIT waits before the driver is set back (a quick relaunch should not flip it twice). A game's START
/// is applied at once.
const STOP_SETTLE: Duration = Duration::from_millis(500);

/// What the Mouse tab shows about the games (read without waiting for the engine's lock).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// rows whose game runs now and was switched to
    pub active: Vec<RowId>,
    /// which source hears the games ("window creation + process snapshot (no admin)"), `None` = none listed / not running
    pub source: Option<&'static str>,
}

enum Msg {
    App(AppEvent),
    /// the card changed on disk (the tab saved it): read it again
    Reload,
    Quit,
}

struct Inner {
    m: Mouse<RealOs>,
    watcher: Option<AppWatcher>,
    /// the names the watcher listens for now (a tab edit that changes no game leaves it alone: no gap with nobody listening)
    names: Vec<String>,
    /// the processes that have an exit wait now (a game's STOP is heard by it)
    waited: HashSet<u32>,
    tx: Sender<Msg>,
}

struct Engine {
    inner: Mutex<Inner>,
    tx: Sender<Msg>,
    status: Mutex<Status>,
}

static ENGINE: OnceLock<Engine> = OnceLock::new();

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Inner {
    /// The watcher listens for exactly the games in the rows (none = no watcher, no thread, no hook).
    fn sync_watcher(&mut self) {
        let names = self.m.accel().per_app.watched_names();
        if names.is_empty() {
            self.watcher = None;
            self.names.clear();
            self.waited.clear();
            return;
        }
        if self.watcher.is_some() && names == self.names {
            return;
        }
        self.names = names.clone();
        if let Some(w) = &self.watcher {
            if w.set_names(names.clone()).is_ok() {
                return;
            }
        }
        let tx = self.tx.clone();
        // (a new watcher holds no exit waits yet)
        self.waited.clear();
        self.watcher = AppWatcher::start(names, move |ev| {
            let _ = tx.send(Msg::App(ev));
        })
        .ok();
    }

    /// Listed games that run already count as running (Order 077), and every running game has an exit wait.
    fn adopt(&mut self) {
        let _ = self.m.adopt_running_games();
        let Some(w) = &self.watcher else { return };
        for pid in self.m.accel().per_app.active_pids() {
            if self.waited.insert(pid) {
                w.watch_exit(pid);
            }
        }
    }

    fn publish(&self, status: &Mutex<Status>) {
        let a = self.m.accel();
        let rows: Vec<RowId> = a.per_app.rows().iter().map(|r| r.id).collect();
        *lock(status) = Status {
            active: rows.iter().copied().filter(|r| a.per_app.is_active(*r)).collect(),
            source: self.watcher.as_ref().and_then(|w| w.active_source()),
        };
    }
}

/// Starts the engine (once per app run; a second call does nothing). The thread loads the card and sets the driver only if
/// the user had it on and it differs, then waits for games. Never blocks the caller.
pub fn start() -> Option<Guard> {
    if ENGINE.get().is_some() || std::env::var_os("APPDATA").is_none() {
        return None;
    }
    let (tx, rx) = channel::<Msg>();
    let eng = Engine { inner: Mutex::new(Inner { m: super::svc::real_bare_pub(), watcher: None, names: Vec::new(), waited: HashSet::new(), tx: tx.clone() }), tx, status: Mutex::new(Status::default()) };
    if ENGINE.set(eng).is_err() {
        return None;
    }
    let eng = ENGINE.get()?;
    std::thread::Builder::new().name("bu-accel".into()).spawn(move || run(eng, rx)).ok().map(|_| Guard)
}

/// Held by the app for its life; dropping it (app exit) ends the engine's thread.
pub struct Guard;

impl crate::pages::Background for Guard {
    fn describe(&self) -> String {
        "acceleration: saved card + per-game switch".into()
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(e) = ENGINE.get() {
            let _ = e.tx.send(Msg::Quit);
        }
    }
}

fn run(eng: &'static Engine, rx: Receiver<Msg>) {
    {
        let mut g = lock(&eng.inner);
        g.m.accel_mut().rawaccel_dir = super::svc::find_rawaccel_folder();
        let _ = g.m.start_accel();
        g.sync_watcher();
        g.adopt();
        g.publish(&eng.status);
    }
    // when the driver is next set: a start at once, a stop after a short wait
    let mut due: Option<Instant> = None;
    loop {
        let msg = match due {
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(t) => rx.recv_timeout(t.saturating_duration_since(Instant::now())),
        };
        match msg {
            Ok(Msg::Quit) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Msg::App(ev)) => {
                let mut g = lock(&eng.inner);
                match &ev {
                    // (the watcher gave a started process its exit wait itself)
                    AppEvent::Started { pid, .. } => {
                        g.waited.insert(*pid);
                    }
                    AppEvent::Stopped { pid } => {
                        g.waited.remove(pid);
                    }
                }
                if g.m.accel_app_event(&ev) {
                    let at = match ev {
                        AppEvent::Started { .. } => Instant::now(),
                        AppEvent::Stopped { .. } => Instant::now() + STOP_SETTLE,
                    };
                    due = Some(due.map_or(at, |d| d.min(at)));
                }
                g.publish(&eng.status);
            }
            Ok(Msg::Reload) => {
                let mut g = lock(&eng.inner);
                reload(&mut g);
                g.publish(&eng.status);
            }
            Err(RecvTimeoutError::Timeout) => {
                due = None;
                let mut g = lock(&eng.inner);
                if g.m.accel().panel.on {
                    let _ = g.m.sync_driver_auto();
                }
                g.publish(&eng.status);
            }
        }
    }
    lock(&eng.inner).watcher = None;
}

/// The card from its file again (the tab saved it): rows, presets, switch. Games that run now stay "active" by process id.
fn reload(g: &mut Inner) {
    let active = g.m.accel().per_app.clone();
    if g.m.load_accel().unwrap_or(false) {
        g.m.accel_mut().per_app.keep_running_from(&active);
    }
    g.sync_watcher();
    g.adopt();
}

/// The Mouse tab saved the card: read it, listen for its games, and set the driver now (blocks for the driver's own ~1 s
/// when it changes). Returns an error line for the tab's toast. No engine (a test copy, no APPDATA) = `Ok` and nothing done.
pub fn apply_now(switched: bool) -> Option<String> {
    let eng = ENGINE.get()?;
    let mut g = lock(&eng.inner);
    reload(&mut g);
    let r = if (switched || g.m.accel().panel.on) && g.m.rawaccel_status().is_ok_and(|s| matches!(s, bu_mouse::accel::service::RawAccelStatus::Installed { .. })) { g.m.sync_driver().err().map(|e| e.to_string()) } else { None };
    g.publish(&eng.status);
    r
}

/// The saved card changed behind the tab's back (Reset): the engine reads it again on its own thread - never blocks the caller,
/// and nothing is written to the driver (a card that is off writes nothing).
pub fn reload_soon() {
    if let Some(e) = ENGINE.get() {
        let _ = e.tx.send(Msg::Reload);
    }
}

/// The games' state for the tab (never waits for the driver write).
pub fn status() -> Status {
    ENGINE.get().map(|e| lock(&e.status).clone()).unwrap_or_default()
}

/// Is the engine running (the tab then leaves the driver to it)?
pub fn running() -> bool {
    ENGINE.get().is_some()
}

#[allow(dead_code)]
fn _assert_send() {
    fn is_send<T: Send>() {}
    is_send::<Msg>();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This process' CPU time so far (kernel + user), in 100-ns units.
    fn cpu_100ns() -> u64 {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let _ = unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
        let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
        t(k) + t(u)
    }

    /// Order 063 proof on the REAL PC (`cargo test -p bu-app proof_063 -- --ignored --test-threads=1 --nocapture`): the engine
    /// with a saved card that is OFF and one game listed - it reads the card, listens for the game (real window events and
    /// process list), and writes NOTHING (a card that is off never touches the driver). Prints the CPU it used while idle.
    /// APPDATA points to a scratch folder for it; the driver is only ever READ.
    #[test]
    #[ignore]
    fn proof_063_engine_listens_and_costs_nothing_while_idle() {
        let scratch = std::env::temp_dir().join(format!("BoylerUtilities-test-accel-rt-{}", std::process::id()));
        let dir = scratch.join("Boyler Utilities").join("mouse");
        std::fs::create_dir_all(&dir).unwrap();
        // a card that is OFF with one game (the Mouse tab's sample row)
        let mut m = Mouse::new(bu_mouse::fake::FakeOs::new(), bu_mouse::AppDirs::new(&dir));
        m.accel_mut().per_app.add_row("VALORANT-Win64-Shipping.exe", bu_mouse::accel::switch::Target::Off);
        m.save_accel().unwrap();
        let file = m.dirs().accel_file();
        std::fs::write(file, m.os().byte_files.values().next().cloned().unwrap()).unwrap();
        std::env::set_var("APPDATA", &scratch);
        let guard = start().expect("the engine starts");
        let t0 = std::time::Instant::now();
        while status().source.is_none() && t0.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let st = status();
        println!("listening: {:?} after {} ms", st.source, t0.elapsed().as_millis());
        assert!(st.source.is_some(), "the game is listened for");
        // idle: 20 s, only this process' own CPU
        let (c0, w0) = (cpu_100ns(), std::time::Instant::now());
        std::thread::sleep(std::time::Duration::from_secs(20));
        let (c1, w1) = (cpu_100ns(), w0.elapsed());
        println!("MEASURED idle CPU of the whole test process over {:.1} s: {:.3} ms ({:.4} % of one core)", w1.as_secs_f64(), (c1 - c0) as f64 / 10_000.0, (c1 - c0) as f64 / 1e7 / w1.as_secs_f64() * 100.0);
        // a tab edit while the card is off: nothing is written (no switch click), and it answers at once
        assert_eq!(apply_now(false), None);
        drop(guard);
        let _ = std::fs::remove_dir_all(&scratch);
    }
}

#[cfg(test)]
mod proof_077 {
    use super::*;

    /// Order 077 proof on the REAL PC (`cargo test -p bu-app proof_077 -- --ignored --test-threads=1 --nocapture`): a listed game that is
    /// ALREADY running is found by the engine at its start from a process snapshot (the stand-in "game" is explorer.exe, always
    /// running; no handle is opened to it) and counts as switched-to; the card is OFF so nothing is written to the driver.
    /// APPDATA points to a scratch folder; the driver is only ever READ.
    #[test]
    #[ignore]
    fn proof_077_a_game_that_is_already_running_is_found_at_start() {
        let scratch = std::env::temp_dir().join(format!("BoylerUtilities-test-accel-077-{}", std::process::id()));
        let dir = scratch.join("Boyler Utilities").join("mouse");
        std::fs::create_dir_all(&dir).unwrap();
        let mut m = Mouse::new(bu_mouse::fake::FakeOs::new(), bu_mouse::AppDirs::new(&dir));
        m.accel_mut().per_app.add_row("explorer.exe", bu_mouse::accel::switch::Target::Off);
        m.accel_mut().per_app.add_row("no-such-game-077.exe", bu_mouse::accel::switch::Target::Off);
        m.save_accel().unwrap();
        let file = m.dirs().accel_file();
        std::fs::write(file, m.os().byte_files.values().next().cloned().unwrap()).unwrap();
        std::env::set_var("APPDATA", &scratch);
        let guard = start().expect("the engine starts");
        let t0 = std::time::Instant::now();
        while status().source.is_none() && t0.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let st = status();
        println!("listening: {:?}; rows running (found by the snapshot): {} of 2 after {} ms", st.source, st.active.len(), t0.elapsed().as_millis());
        assert_eq!(st.active.len(), 1, "explorer.exe (running) is found, the game that does not exist is not");
        drop(guard);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// Order 077 proof on the REAL PC (same command): his real Raw Accel driver is READ through a read-only service (any write
    /// would be refused), the saved card (scratch file) is ON with a curve that is not what the driver runs and no note of an
    /// earlier write of this app exists - the app start must leave the driver alone (`RawAccelChanged`) and its bytes must
    /// be the same before and after.
    #[test]
    #[ignore]
    fn proof_077_app_start_leaves_a_newer_raw_accel_write_alone() {
        use bu_mouse::accel::panel::Field;
        use bu_mouse::accel::service::StartAccel;
        let scratch = std::env::temp_dir().join(format!("BoylerUtilities-test-accel-077b-{}", std::process::id()));
        let dir = scratch.join("Boyler Utilities").join("mouse");
        std::fs::create_dir_all(&dir).unwrap();
        // a card that is ON with Linear / acceleration 0.05 (not what any Raw Accel setup runs)
        let mut card = Mouse::new(bu_mouse::fake::FakeOs::new(), bu_mouse::AppDirs::new(&dir));
        card.accel_mut().panel.on = true;
        card.accel_mut().panel.set_value(Field::Acceleration, 0.05);
        card.save_accel().unwrap();
        std::fs::write(card.dirs().accel_file(), card.os().byte_files.values().next().cloned().unwrap()).unwrap();
        let mut m = Mouse::new(bu_mouse::win::RealOs::read_only(), bu_mouse::AppDirs::new(&dir));
        let Some(before) = m.driver_state().unwrap() else { return println!("no Raw Accel 1.7 driver here: nothing to prove") };
        if bu_mouse::accel::bytes::read_profiles(&before).is_empty() {
            return println!("the driver holds no profile (nobody wrote it since boot): a start would set it - not the case to prove");
        }
        let r = m.start_accel().unwrap();
        let after = m.driver_state().unwrap().unwrap();
        println!("start_accel = {r:?}; driver bytes before == after: {}", before == after);
        assert_eq!(r, StartAccel::RawAccelChanged);
        assert_eq!(before, after);
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
