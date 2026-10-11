//! The Controller tab's worker (Order 047, the owner's test 3: "i clicked launch steam, and a lot of times i notice this, the
//! entire bottom right of the screen gets a big black box, and then steam opens and it unfreezes"). When Steam started,
//! the tab read every game's files again ON THE MENU'S THREAD (the game list 4-5 times: every appmanifest of every library
//! + localconfig.vdf, MBs) while Steam was writing them; every setting click wrote, backed up and read it all again there
//! too; the tab's opening listed the controllers (one report per PlayStation pad for its battery, up to 400 ms each).
//!
//! Now ONE thread per open tab does all of it, in the order it was asked (two quick clicks write in that order): it makes
//! the Steam service (finding Steam reads the registry), lists the controllers, starts the live view, reads, writes (each
//! write: bu-controller's own call, its change-log entry, the files read again) and hands every answer back through a
//! channel the page's `tick` drains; it wakes the menu after each answer. The page shows the last known state at once
//! and a change at once (made to its own copy of the layout, `Open::local`), and takes the files' truth when the answer
//! lands. The thread ends when the tab closes (its job channel closes) - after the writes still queued.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use bu_controller::layout::{ActionSet, Layout};
use bu_controller::live::LiveView;
use bu_controller::os::{PadInfo, PadOs};
use bu_controller::{Change, Game, PadKind, PadView, Part, PrefSetting, Prefs};

use super::data::{self, Shared, Svc};
use super::hist::Step;
use crate::ui::el::Key;

/// One change-log entry (item, label, before, after), written by the page (`cx.record`).
pub type Rec = (String, String, crate::undo::Val, crate::undo::Val);

/// Which game / controller type / action set to read (`key` None = the first game, `set` None = its first set).
#[derive(Clone, Debug)]
pub struct Want {
    pub kind: PadKind,
    pub key: Option<String>,
    pub set: Option<u32>,
    /// read the game names again (the tab opened, Steam started)
    pub fresh: bool,
}

/// What the worker read: the page's whole view of one game (as `load_games` + `load_view` read it on the menu's thread
/// before Order 047).
#[derive(Clone, Debug)]
pub struct Loaded {
    pub kind: PadKind,
    pub games: Vec<Game>,
    pub gi: usize,
    pub set: u32,
    /// None = no game to read (the sets stay as they were)
    pub sets: Option<Vec<ActionSet>>,
    pub lay: Option<Layout>,
    pub steam_lay: Option<Layout>,
    pub view: Option<PadView>,
    pub steam: Option<PadView>,
    /// None = the controllers' files could not be read (the page keeps what it has)
    pub prefs: Option<Vec<Prefs>>,
    pub note: Option<String>,
}

impl Loaded {
    fn empty(kind: PadKind, note: Option<String>) -> Loaded {
        Loaded { kind, games: vec![], gi: 0, set: 0, sets: None, lay: None, steam_lay: None, view: None, steam: None, prefs: None, note }
    }
}

/// A write (each one bu-controller's own call: one write, one backup / undo step).
#[derive(Clone, Debug)]
pub enum Wr {
    /// `gone` = an undo / redo: the game must still be in the list ("That game's layout is gone")
    Layout { key: String, kind: PadKind, set: u32, changes: Vec<Change>, gone: bool },
    PartToSteam { key: String, kind: PadKind, set: u32, part: Part },
    NewSet { key: String, kind: PadKind, from: u32, title: String },
    /// bu-controller's exact undo of a new action set, only while it is still its last write
    UndoNewSet { key: String, kind: PadKind, from: u32 },
    Prefs { serial: String, vals: Vec<(PrefSetting, Option<String>)>, label: String },
}

/// The page's own bookkeeping of a write (its undo step, its words), handed back with the answer.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum After {
    Layout { before: Option<PadView>, parts: Vec<Part>, game: String, kind: PadKind, set: u32, acting: Option<(Key, String)>, fallback: String, say: String },
    PartToSteam { before: Option<PadView>, part: Part, game: String, kind: PadKind, set: u32, name: String },
    NewSet { game: String, kind: PadKind, from: u32, title: String },
    Prefs { before: Prefs, label: String, acting: Option<(Key, String)> },
    /// an undo (`redo` false) / redo of this step
    Replay { step: Step, redo: bool },
}

/// What the page asks of its worker.
#[allow(clippy::large_enum_variant)]
pub enum Job {
    /// the tab opened: the Steam service, the controllers, the first game's view, the live view
    Open { real_read: bool, slow: u64 },
    Load(Want),
    Write { wr: Wr, want: Want, after: After },
    /// the live view of this controller (`old` is stopped here, off the menu's thread)
    Live { pad: Option<PadInfo>, old: Option<LiveView> },
    /// "Open in Steam": the link (needs Steam's process list), opened through the shell
    OpenInSteam { game: Game, test: bool },
    /// "Restart Steam to apply" (Order 085): close Steam cleanly, write the light values again, start it minimised
    RestartSteam { serial: String },
}

impl Job {
    /// Its answer carries a `Loaded` (the page takes the newest one only).
    pub fn is_state(&self) -> bool {
        matches!(self, Job::Open { .. } | Job::Load(_) | Job::Write { .. })
    }
}

/// The worker's answers (one per job, in the jobs' order).
#[allow(clippy::large_enum_variant)]
pub enum Done {
    Opened { pads: Vec<PadInfo>, steam_dir: Option<PathBuf>, svc_ok: bool, running: bool, loaded: Loaded, live: Option<LiveView> },
    Loaded(Loaded),
    /// `r` = Ok(the new action set's id, 0 otherwise) or the error's words
    Wrote { r: Result<u32, String>, recs: Vec<Rec>, loaded: Loaded, after: After },
    Live(Option<LiveView>),
    /// a toast for the page (None = nothing to say)
    Said(Option<String>),
    /// the restart ended: Ok(what to say) or the words of why not
    Restarted(Result<String, String>),
}

impl Done {
    pub fn is_state(&self) -> bool {
        matches!(self, Done::Opened { .. } | Done::Loaded(_) | Done::Wrote { .. })
    }
}

/// The controller type the tab opens with: the one plugged in; none = the type Steam knows (an Edge), else a DualSense.
pub fn open_kind(pads: &[PadInfo], prefs: &[Prefs]) -> PadKind {
    pads.first().map(|p| p.kind).unwrap_or_else(|| prefs.iter().find_map(|p| if p.name.contains("Edge") { Some(PadKind::DualSenseEdge) } else { None }).unwrap_or(PadKind::DualSense))
}

/// Steam's own view of an action set (its set 0 when Steam's layout has no such set).
pub fn steam_view(l: &Layout, set: u32, kind: PadKind) -> PadView {
    let ss = if l.action_sets().iter().any(|s| s.id == set) { set } else { 0 };
    l.pad_view(ss, kind)
}

/// Start the tab's worker: (its job channel, its answers). `fake` = the fake Steam / controllers (test copies).
pub fn start(svc: Shared, fake: bool, waker: crate::services::Waker) -> (Sender<Job>, Receiver<Done>) {
    let (jt, jr) = channel::<Job>();
    let (dt, dr) = channel::<Done>();
    let r = std::thread::Builder::new().name("pad-worker".into()).spawn(move || {
        let mut w = Worker { svc, pads: None, fake, waker };
        // after the tab closed its sender, the jobs still queued are done (no write is lost), then the thread ends
        for job in jr {
            let d = w.run(job);
            let _ = dt.send(d);
            waker.wake();
        }
    });
    if let Err(e) = r {
        crate::timing::note(&format!("pad worker failed: {e}"));
    }
    (jt, dr)
}

struct Worker {
    svc: Shared,
    pads: Option<Box<dyn PadOs>>,
    fake: bool,
    waker: crate::services::Waker,
}

impl Worker {
    fn run(&mut self, job: Job) -> Done {
        match job {
            Job::Open { real_read, slow } => self.open(real_read, slow),
            Job::Load(want) => {
                let g = self.svc.lock().unwrap_or_else(|p| p.into_inner());
                let l = match &*g {
                    Some(Ok(s)) => load(s, &want),
                    Some(Err(e)) => Loaded::empty(want.kind, Some(e.clone())),
                    None => Loaded::empty(want.kind, None),
                };
                Done::Loaded(l)
            }
            Job::Write { wr, want, after } => self.write(wr, want, after),
            Job::Live { pad, old } => {
                drop(old);
                Done::Live(pad.and_then(|p| self.live(&p)))
            }
            Job::OpenInSteam { game, test } => Done::Said(self.open_in_steam(&game, test)),
            Job::RestartSteam { serial } => Done::Restarted(self.restart_steam(&serial)),
        }
    }

    fn open(&mut self, real_read: bool, slow: u64) -> Done {
        let mut g = self.svc.lock().unwrap_or_else(|p| p.into_inner());
        if g.is_none() {
            let s = data::open_steam(self.fake, real_read);
            if let Ok(s) = &s {
                s.set_fake_delay(slow);
            }
            *g = Some(s);
        }
        let os = data::pads(self.fake, slow);
        let pads = os.list_pads().unwrap_or_default();
        self.pads = Some(os);
        let (svc_ok, steam_dir, running, loaded) = match &*g {
            Some(Ok(s)) => {
                let kind = open_kind(&pads, &s.preferences().unwrap_or_default());
                // the real Steam's process is watched by the page (`SteamWatch`); the fake's switch is read here
                let running = !self.fake || s.steam_running();
                (true, Some(s.steam_dir()), running, load(s, &Want { kind, key: None, set: None, fresh: true }))
            }
            Some(Err(e)) => (false, None, true, Loaded::empty(open_kind(&pads, &[]), Some(e.clone()))),
            None => (false, None, true, Loaded::empty(open_kind(&pads, &[]), None)),
        };
        drop(g);
        let live = if self.fake { None } else { pads.iter().find(|p| p.kind == loaded.kind).cloned().and_then(|p| self.live(&p)) };
        Done::Opened { pads, steam_dir, svc_ok, running, loaded, live }
    }

    fn live(&self, p: &PadInfo) -> Option<LiveView> {
        let os = self.pads.as_ref()?;
        let w = self.waker;
        LiveView::start(os.as_ref(), p, Some(Box::new(move |_| w.wake()))).ok()
    }

    fn write(&mut self, wr: Wr, mut want: Want, after: After) -> Done {
        let mut g = self.svc.lock().unwrap_or_else(|p| p.into_inner());
        let s = match &mut *g {
            Some(Ok(s)) => s,
            Some(Err(e)) => {
                let e = e.clone();
                return Done::Wrote { r: Err(e.clone()), recs: vec![], loaded: Loaded::empty(want.kind, Some(e)), after };
            }
            None => return Done::Wrote { r: Err("Steam is not read yet".into()), recs: vec![], loaded: Loaded::empty(want.kind, None), after },
        };
        let (r, recs) = write_in(s, &wr);
        // the set the page shows after it: the new one / the one it was made from (only when the write went through)
        match (&wr, &r) {
            (Wr::NewSet { .. }, Ok(id)) => want.set = Some(*id),
            (Wr::UndoNewSet { from, .. }, Ok(_)) => want.set = Some(*from),
            _ => {}
        }
        let loaded = load(s, &want);
        Done::Wrote { r, recs, loaded, after }
    }

    fn open_in_steam(&self, game: &Game, test: bool) -> Option<String> {
        let g = self.svc.lock().unwrap_or_else(|p| p.into_inner());
        let s = match &*g {
            Some(Ok(s)) => s,
            Some(Err(e)) => return Some(e.clone()),
            None => return None,
        };
        match s.open_in_steam_link(game) {
            Ok(url) => {
                if test {
                    // a test copy never opens anything on the PC
                    Some(format!("Opens Steam\u{2019}s own controller screen for {}", game.name))
                } else {
                    super::shell_open(&url);
                    None
                }
            }
            Err(bu_controller::Error::SteamClosed) => Some("Steam is closed \u{b7} start Steam first".into()),
            Err(e) => Some(e.to_string()),
        }
    }
}

/// How long Steam gets to close by itself (a clean shutdown saves its files and the cloud; it is never killed).
pub const CLOSE_WAIT_MS: u64 = 45_000;
/// How often the closing Steam's process is looked at.
const CLOSE_POLL_MS: u64 = 300;

impl Worker {
    /// "Restart Steam to apply" (Order 085): refuse while a game runs, remember the light values, close Steam cleanly and
    /// wait for it, write the values again (a closing Steam may write its own copy back), start Steam minimised. The
    /// service is locked only for each step, never while waiting.
    fn restart_steam(&self, serial: &str) -> Result<String, String> {
        let svc = |f: &mut dyn FnMut(&mut Svc) -> Result<(), String>| -> Result<(), String> {
            let mut g = self.svc.lock().unwrap_or_else(|p| p.into_inner());
            match &mut *g {
                Some(Ok(s)) => f(s),
                Some(Err(e)) => Err(e.clone()),
                None => Err("Steam is not read yet".into()),
            }
        };
        let words = |e: bu_controller::Error| match e {
            bu_controller::Error::GameRunning => "A game is running through Steam \u{b7} close it first, then restart Steam".to_string(),
            bu_controller::Error::SteamClosed => "Steam is closed \u{b7} nothing to restart".to_string(),
            e => e.to_string(),
        };
        let mut wanted = Vec::new();
        svc(&mut |s| {
            s.restart_check().map_err(words)?;
            wanted = s.light_values(serial).map_err(words)?;
            s.steam_shutdown().map_err(words)
        })?;
        let mut waited = 0;
        loop {
            let mut closed = false;
            svc(&mut |s| {
                closed = s.steam_closed();
                Ok(())
            })?;
            if closed {
                break;
            }
            if waited >= CLOSE_WAIT_MS {
                return Err("Steam didn\u{2019}t close by itself \u{b7} it was left running, nothing was forced".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(CLOSE_POLL_MS));
            waited += CLOSE_POLL_MS;
        }
        // Steam is gone: the values go into the file again if its closing wrote an older copy over them, then it starts
        let kept = svc(&mut |s| s.keep_light(serial, &wanted).map(|_| ()).map_err(words));
        let started = svc(&mut |s| s.steam_start_minimised().map_err(words));
        started?;
        kept?;
        Ok("Steam restarted \u{b7} it sets the light to your colour when the controller connects".into())
    }
}

/// The page's view of one game: ONE read of the game list (its names kept by bu-controller, Order 047), one of the
/// layout, one of Steam's own layout, the controllers' files.
pub fn load(s: &Svc, want: &Want) -> Loaded {
    let kind = want.kind;
    if want.fresh {
        s.forget_names();
    }
    let mut note = None;
    let games = match s.games(kind) {
        Ok(g) => g,
        Err(e) => {
            note = Some(e.to_string());
            vec![]
        }
    };
    // installed games first, the order Steam's index has them otherwise; keep the picked one
    let gi = want.key.as_ref().and_then(|k| games.iter().position(|g| &g.key == k)).unwrap_or(0);
    let mut set = want.set.unwrap_or(0);
    let mut l = Loaded::empty(kind, None);
    l.prefs = s.preferences().ok();
    if let Some(g) = games.get(gi).cloned() {
        match s.open_game(g, kind) {
            Ok(o) => {
                let sets = o.layout.action_sets();
                if !sets.iter().any(|x| x.id == set) {
                    set = sets.first().map(|x| x.id).unwrap_or(0);
                }
                l.view = Some(o.layout.pad_view(set, kind));
                note = None;
                l.steam_lay = s.steam_layout_of(&o).ok();
                l.steam = l.steam_lay.as_ref().map(|sl| steam_view(sl, set, kind));
                l.sets = Some(sets);
                l.lay = Some(o.layout);
            }
            Err(e) => {
                l.sets = Some(vec![]);
                note = Some(e.to_string());
            }
        }
    }
    l.games = games;
    l.gi = gi;
    l.set = set;
    l.note = note;
    l
}

/// One write through bu-controller + its change-log entry (an entry only when the app now has a backup of the file).
fn write_in(s: &mut Svc, wr: &Wr) -> (Result<u32, String>, Vec<Rec>) {
    fn gone(s: &Svc, key: &str, kind: PadKind) -> bool {
        !s.games(kind).map(|g| g.iter().any(|x| x.key == key)).unwrap_or(false)
    }
    const GONE: &str = "That game\u{2019}s layout is gone";
    match wr {
        Wr::Layout { key, kind, set, changes, gone: check } => {
            if *check && gone(s, key, *kind) {
                return (Err(GONE.into()), vec![]);
            }
            let had = s.has_original(key, *kind);
            match s.apply_all(key, *kind, *set, changes) {
                Ok(()) => (Ok(0), log_layout(s, key, *kind, had)),
                Err(e) => (Err(e.to_string()), vec![]),
            }
        }
        Wr::PartToSteam { key, kind, set, part } => {
            let had = s.has_original(key, *kind);
            match s.part_to_steam(key, *kind, *set, *part) {
                Ok(()) => (Ok(0), log_layout(s, key, *kind, had)),
                Err(e) => (Err(e.to_string()), vec![]),
            }
        }
        Wr::NewSet { key, kind, from, title } => {
            if gone(s, key, *kind) {
                return (Err(GONE.into()), vec![]);
            }
            let had = s.has_original(key, *kind);
            match s.add_action_set(key, *kind, *from, title) {
                Ok(id) => (Ok(id), log_layout(s, key, *kind, had)),
                Err(e) => (Err(e.to_string()), vec![]),
            }
        }
        Wr::UndoNewSet { key, kind, .. } => {
            let Some(name) = s.games(*kind).ok().and_then(|g| g.into_iter().find(|x| &x.key == key)).map(|g| g.name) else {
                return (Err(GONE.into()), vec![]);
            };
            let had = s.has_original(key, *kind);
            // bu-controller's exact undo, only while the new set is still its last write
            if s.undo_label().as_deref() != Some(format!("{name} \u{b7} new action set").as_str()) {
                return (Err("The new action set can\u{2019}t be taken back any more (the layout was changed since)".into()), vec![]);
            }
            match s.undo() {
                Ok(_) => (Ok(0), log_layout(s, key, *kind, had)),
                Err(e) => (Err(e.to_string()), vec![]),
            }
        }
        Wr::Prefs { serial, vals, label } => {
            let v: Vec<(PrefSetting, Option<&str>)> = vals.iter().map(|(s, v)| (*s, v.as_deref())).collect();
            let had = s.has_preferences_original(serial);
            match s.set_preferences(serial, &v, label) {
                Ok(()) => (Ok(0), log_prefs(s, serial, had)),
                Err(e) => (Err(e.to_string()), vec![]),
            }
        }
    }
}

/// The change log (Order 036): one entry per layout file, its value before = the bytes from before the app's first
/// change (the crate keeps them as backups in the app's folder, across restarts). Nothing written (no backup) = none.
fn log_layout(s: &Svc, key: &str, kind: PadKind, had: bool) -> Vec<Rec> {
    if !s.has_original(key, kind) {
        return vec![];
    }
    let name = s.game(key, kind).map(|g| g.name).unwrap_or_else(|_| key.to_string());
    let item = data::Item::Layout(kind, key.to_string());
    let new = s.log_val(&item).unwrap_or_else(|| crate::undo::Val::new(data::EDITS, "your edits"));
    vec![(item.id(), data::layout_label(&name, kind), data::orig_val(!had), new)]
}

fn log_prefs(s: &Svc, serial: &str, had: bool) -> Vec<Rec> {
    if !s.has_preferences_original(serial) {
        return vec![];
    }
    let item = data::Item::Prefs(serial.to_string());
    let new = s.log_val(&item).unwrap_or_else(|| crate::undo::Val::new(data::EDITS, "your settings"));
    let name = s.preferences().ok().and_then(|p| p.into_iter().find(|p| p.serial == serial)).map(|p| p.name).unwrap_or_else(|| "this controller".into());
    vec![(item.id(), data::prefs_label(&name), data::orig_val(!had), new)]
}
