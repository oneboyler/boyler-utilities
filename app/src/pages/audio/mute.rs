//! Mic mute on the Audio page (menu-v22, v21 review): the Input row's mic icon mutes / unmutes; the small blue
//! "Mute settings" link under "Input" opens the small popup window (`.dlg.mdlg.mmdlg`, 430 px) with the old Mic mute
//! card's settings: the Mic mute switch (+ the Live / Muted pill), the mute key (one key or separate keys), the sound
//! (on mute / on unmute / volume), the icon on screen (Off / When it changes / Always + position, style, size, opacity,
//! monitor, the fullscreen note). Wired to crates/micmute (bu-micmute).
//!
//! The service and the settings live for the app's life (a mute key and its sound work while the menu
//! is closed): made and loaded at app start (`Page::background` -> `load`), saved in settings.cfg on every change.
//! Keys: the app's keys manager (services.rs) - three actions registered at app start (`register`); the key fields are
//! its fields (listen, refuse a key used elsewhere, register with Windows).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use taffy::style::AlignItems;

use bu_micmute::fake::{FakeMicOs, FakeSoundOut};
use bu_micmute::{MicMute, Sound, SoundSettings};

use super::micicon;
use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Font, Rgba};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, lh, sub, Cursor, El, Key, RADIUS_PILL};
use crate::ui::pieces::dropdown::{self, Item};
use crate::ui::pieces::keyfield::{self, Show};
use crate::ui::pieces::{btn_font, button, card, dialog, group, inote, link, rowbits, seg, slider, toggle};
use crate::ui::{cmix, ACC, AMBER, CTL, CTL_H, FG, FG2, FG3, GREEN, GRP, HAIR, RED, WHITE, WIN_H, WIN_W};

pub const K_MM: Key = key("aud.mm");
const K_ON: Key = key("aud.mm.on");
const K_STP: Key = key("aud.mm.stp");
const K_SEP: Key = key("aud.mm.sep");
const K_ONE: Key = key("aud.mm.one");
const K_KEY_ONE: Key = key("aud.mm.key.one");
const K_KEY_MUTE: Key = key("aud.mm.key.mute");
const K_KEY_UNMUTE: Key = key("aud.mm.key.unmute");
const K_SND: Key = key("aud.mm.snd");
const K_SND_CHG: Key = key("aud.mm.sndchg");
const K_SND_MUTE: Key = key("aud.mm.sndmute");
const K_SND_UNMUTE: Key = key("aud.mm.sndunmute");
const K_PV_MUTE: Key = key("aud.mm.pvmute");
const K_PV_UNMUTE: Key = key("aud.mm.pvunmute");
const K_VOL: Key = key("aud.mm.vol");
const K_ICON: Key = key("aud.mm.icon");
const K_CPK: Key = key("aud.mm.cpk");
const K_MOVE: Key = key("aud.mm.move");
const K_STYLE: Key = key("aud.mm.style");
const K_SIZE: Key = key("aud.mm.size");
const K_OP: Key = key("aud.mm.op");
const K_MON: Key = key("aud.mm.mon");
const K_LIST: Key = key("aud.mm.list");

const POP: Bezier = Bezier::new(0.3, 1.35, 0.5, 1.0);

/// The drawing's quick spots (`SPOTS`): name, horizontal, vertical.
const SPOTS: [(&str, char, char); 6] =
    [("Top left", 'L', 'T'), ("Top middle", 'C', 'T'), ("Top right", 'R', 'T'), ("Bottom left", 'L', 'B'), ("Bottom middle", 'C', 'B'), ("Bottom right", 'R', 'B')];
const ICON_MODES: [&str; 3] = ["Off", "When it changes", "Always"];
const STYLES: [&str; 3] = ["Pill with text", "Icon only", "Dot"];
const SIZES: [&str; 3] = ["S", "M", "L"];

/// The settings the popup shows (the drawing's S defaults: off, one key, sound OFF (5 %), icon "When it changes" top right,
/// pill, M, 100 %, main monitor). The sound settings live in the bu-micmute service.
/// The keys are the app's keys manager's (services.rs): these three actions, registered at app start (`register`).
#[derive(Clone, Debug, PartialEq)]
pub struct MuteSet {
    pub on: bool,
    pub sep: bool,
    pub snd_open: bool,
    pub icon: usize,
    /// where the icon sits on the screen (a quick spot or where it was dragged)
    pub spot: micicon::Spot,
    pub style: usize,
    pub size: usize,
    pub op: f32,
    pub mon: usize,
}

impl Default for MuteSet {
    fn default() -> Self {
        MuteSet { on: false, sep: false, snd_open: false, icon: 1, spot: micicon::quick(2), style: 0, size: 1, op: 1.0, mon: 0 }
    }
}

/// The keys manager's actions (one key, or separate mute / unmute keys).
pub const A_ONE: &str = "mic.toggle";
pub const A_MUTE: &str = "mic.mute";
pub const A_UNMUTE: &str = "mic.unmute";

fn action_of(k: Key) -> &'static str {
    match k {
        K_KEY_MUTE => A_MUTE,
        K_KEY_UNMUTE => A_UNMUTE,
        _ => A_ONE,
    }
}

/// The key set for an action ("Ctrl + Shift + M"), None = no key. Inside a key handler the services are busy: then None.
pub fn bound(action: &str) -> Option<String> {
    crate::services::try_with(|s| s.field(action).0).flatten()
}

/// Mic mute is on and has its key(s). (Inside a key handler, where the services can't be read, a key just fired: yes.)
fn keys_ready(s: &MuteSet) -> bool {
    if !s.on {
        return false;
    }
    let ok = crate::services::try_with(|sv| {
        let set = |a: &str| sv.field(a).0.is_some();
        if s.sep {
            set(A_MUTE) || set(A_UNMUTE)
        } else {
            set(A_ONE)
        }
    });
    ok.unwrap_or_else(crate::services::in_use)
}

/// Register the three keys (`Page::start`, once at app start - they work with the menu closed). `fake` = a test copy.
pub fn register(s: &mut crate::services::Services) {
    use crate::keys::Action;
    let fake = s.test;
    s.add_action(Action::new(A_ONE, "Mic mute", "aud"), move |down| on_key(A_ONE, down, fake));
    s.add_action(Action::new(A_MUTE, "Mic mute: mute", "aud"), move |down| on_key(A_MUTE, down, fake));
    s.add_action(Action::new(A_UNMUTE, "Mic mute: unmute", "aud"), move |down| on_key(A_UNMUTE, down, fake));
}

/// A mute key went down (menu open or closed): bu-micmute mutes / unmutes with its sound; the on-screen
/// icon follows. Only while Mic mute is switched on, and only the keys of the chosen way (one key / separate keys).
pub(super) fn on_key(action: &str, down: bool, fake: bool) {
    let s = settings();
    if !down || !s.on || (s.sep == (action == A_ONE)) {
        return;
    }
    let m = service(fake, false, false);
    if fake {
        // a test copy's fake mic answers at once: in place, as before
        if let Ok(st) = press(&m, action) {
            CHANGED.store(true, Ordering::Relaxed);
            sync_icon(st.muted, false, false, true);
        }
        return;
    }
    let a = match action {
        A_MUTE => A_MUTE,
        A_UNMUTE => A_UNMUTE,
        _ => A_ONE,
    };
    send_press(m, a, true);
}

/// Order 047: the Audio page's mic button / Mute settings' pill: muted <-> live. The fake (a test copy) in place: its new
/// state. A real mic (Core Audio: a fresh device enumerator a call, 10-50 ms) on the key's worker, like the key - the
/// state to show at once is the flip of the one shown (`shown`); the worker's answer follows (`CHANGED`, and with `icon`
/// the on-screen icon / the tray badge).
pub fn toggle_now(m: &MicMute, shown: bool, fake: bool, icon: bool) -> Option<bool> {
    if fake {
        return m.toggle().ok().map(|s| s.muted);
    }
    send_press(m.clone(), A_ONE, icon);
    Some(!shown)
}

/// The key's own work: mute / unmute / toggle the chosen mic (with its sound).
fn press(m: &MicMute, action: &str) -> bu_micmute::Result<bu_micmute::MicState> {
    match action {
        A_MUTE => m.mute(),
        A_UNMUTE => m.unmute(),
        _ => m.toggle(),
    }
}

/// Order 047: the mute key's Core Audio work (find the mic, read its flag, set it, list the devices: 10-50 ms) held the
/// menu's thread - every key press froze the menu's painting for it. It now runs on one worker thread of its own, fed in
/// order (two quick presses toggle twice, never at the same time); the mute itself happens there at once, with its sound.
/// The menu's thread only hands the press over, and takes the answer for the on-screen icon / the tray badge (they live
/// on the menu's thread) with a 10 ms Windows timer that stops once no press is on its way.
static KEYQ: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Sender<KeyJob>>> = std::sync::OnceLock::new();
/// presses handed over and not yet done
static KEY_PENDING: AtomicUsize = AtomicUsize::new(0);
/// the last press's answer (muted?) for the icon, taken on the menu's thread
static KEY_DONE: std::sync::Mutex<Option<bool>> = std::sync::Mutex::new(None);
/// the mic's last known state (any read or press): what an opening page shows until its own read is in
static LAST_MUTED: AtomicBool = AtomicBool::new(false);

/// One job of the key worker - presses, state reads and the watch's start / stop, done one after another in the order
/// given (so an older read can never land after a newer one, a watch never starts after its stop).
enum KeyJob {
    Press(MicMute, &'static str),
    Read(MicMute, Arc<std::sync::Mutex<Option<bool>>>),
    Watch(MicMute, bool),
}

thread_local! {
    /// the menu thread's timer that takes the key's answer (0 = none armed)
    static KEY_TIMER: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The mic's last known state (Order 047: an opening Audio page shows it at once; its own read follows).
pub fn last_muted() -> bool {
    LAST_MUTED.load(Ordering::Relaxed)
}

fn run_job(j: KeyJob) {
    match j {
        KeyJob::Press(m, a) => {
            if let Ok(st) = press(&m, a) {
                LAST_MUTED.store(st.muted, Ordering::Relaxed);
                CHANGED.store(true, Ordering::Relaxed);
                if let Ok(mut d) = KEY_DONE.lock() {
                    *d = Some(st.muted);
                }
            }
            KEY_PENDING.fetch_sub(1, Ordering::AcqRel);
        }
        KeyJob::Read(m, slot) => {
            let v = mic_muted(&m);
            if let Ok(mut s) = slot.lock() {
                *s = Some(v);
            }
        }
        KeyJob::Watch(m, true) => watch(&m),
        KeyJob::Watch(m, false) => unwatch(&m),
    }
    // an open Audio page shows the new state (it reads `CHANGED` / its read slot)
    crate::services::Waker.wake();
}

/// Hand one job to the key worker (made on the first job); Err = no worker (its thread could not start): the job back.
fn key_job(j: KeyJob) -> Result<(), KeyJob> {
    let q = KEYQ.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<KeyJob>();
        let _ = std::thread::Builder::new().name("bu-mic-key".into()).spawn(move || {
            // (the real layer sets COM up itself on every call)
            for j in rx {
                run_job(j);
            }
        });
        std::sync::Mutex::new(tx)
    });
    match q.lock() {
        Ok(tx) => tx.send(j).map_err(|e| e.0),
        Err(_) => Err(j),
    }
}

/// Order 047: read the mic's state into `slot` on the key worker (Core Audio, 10-50 ms), in order with the presses.
pub fn read_state(m: &MicMute, slot: Arc<std::sync::Mutex<Option<bool>>>) {
    if let Err(j) = key_job(KeyJob::Read(m.clone(), slot)) {
        run_job(j);
    }
}

/// Order 047: start / stop the change watch on the key worker (it registers with Core Audio; stopping waits for its
/// thread), in order with everything else there.
pub fn watch_off(m: &MicMute, on: bool) {
    if let Err(j) = key_job(KeyJob::Watch(m.clone(), on)) {
        run_job(j);
    }
}

/// Hand one press to the key worker; `arm` = the icon follows (off in unit tests: no timer, no icon window).
fn send_press(m: MicMute, action: &'static str, arm: bool) {
    KEY_PENDING.fetch_add(1, Ordering::AcqRel);
    if let Err(KeyJob::Press(m, action)) = key_job(KeyJob::Press(m, action)) {
        // no worker: in place, as before
        KEY_PENDING.fetch_sub(1, Ordering::AcqRel);
        if let Ok(st) = press(&m, action) {
            LAST_MUTED.store(st.muted, Ordering::Relaxed);
            CHANGED.store(true, Ordering::Relaxed);
            if arm {
                sync_icon(st.muted, false, false, true);
            }
        }
        return;
    }
    if arm {
        arm_key_timer();
    }
}

/// Menu thread: the key worker's answer -> the icon and the tray badge. True = a press is still on its way.
fn key_answer() -> bool {
    // (read before taking the answer: the worker stores its answer before it counts the press done)
    let waiting = KEY_PENDING.load(Ordering::Acquire) > 0;
    if let Some(muted) = KEY_DONE.lock().ok().and_then(|mut d| d.take()) {
        sync_icon(muted, false, false, true);
    }
    waiting
}

fn arm_key_timer() {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};
        unsafe extern "system" fn tick(_h: windows::Win32::Foundation::HWND, _m: u32, _id: usize, _t: u32) {
            if !key_answer() {
                let id = KEY_TIMER.with(|t| t.replace(0));
                if id != 0 {
                    unsafe {
                        let _ = KillTimer(None, id);
                    }
                }
            }
        }
        if KEY_TIMER.with(|t| t.get()) != 0 {
            return;
        }
        let id = unsafe { SetTimer(None, 0, 10, Some(tick)) };
        KEY_TIMER.with(|t| t.set(id));
    }
}

/// Only the keys that do something are registered with Windows: none while Mic mute is off, the one key or the two
/// separate keys as chosen (a registered key swallows its keystroke everywhere). Their keys stay set either way.
pub fn sync_keys() {
    let s = settings();
    crate::services::try_with(|sv| {
        sv.keys.set_active(A_ONE, s.on && !s.sep);
        sv.keys.set_active(A_MUTE, s.on && s.sep);
        sv.keys.set_active(A_UNMUTE, s.on && s.sep);
    });
}

/// Move a key from one action to another (switching between one key and separate keys keeps the key).
fn move_key(from: &str, to: &str) {
    crate::services::with(|s| {
        if s.field(to).0.is_some() {
            return;
        }
        let c = match s.keys.state(from) {
            crate::keys::KeyState::Working(c) | crate::keys::KeyState::NotWorking(c, _) => c,
            crate::keys::KeyState::Unbound => return,
        };
        let _ = s.keys.unbind(&mut s.store, from);
        let _ = s.keys.bind(&mut s.store, to, c);
    });
}

// ---------------------------------------------------------------- saved (settings.cfg, page "aud": the owner's choices survive a restart)
const SAVE: &str = "mm";

fn sound_idx(s: Sound) -> usize {
    Sound::ALL.iter().position(|x| *x == s).unwrap_or(0)
}

/// Write the settings + the mic service's sound choices.
fn save(m: Option<&MicMute>) {
    let s = settings();
    let mut v = vec![
        format!("on={}", s.on as u8),
        format!("sep={}", s.sep as u8),
        format!("icon={}", s.icon),
        format!("spot={},{},{},{}", s.spot.h, s.spot.dx, s.spot.v, s.spot.dy),
        format!("style={}", s.style),
        format!("size={}", s.size),
        format!("op={}", s.op),
        format!("mon2={}", s.mon),
    ];
    if let Some(m) = m {
        let snd = m.sound();
        v.push(format!("snd2={},{},{},{}", snd.enabled as u8, sound_idx(snd.on_mute), sound_idx(snd.on_unmute), snd.volume));
    } else if let Some(old) = crate::services::try_with(|sv| sv.store.get_list(crate::settings::Scope::Page("aud"), SAVE).map(|l| l.to_vec())).flatten() {
        v.extend(old.into_iter().filter(|l| l.starts_with("snd=") || l.starts_with("snd2=")));
    }
    crate::services::try_with(|sv| sv.store.set_list(crate::settings::Scope::Page("aud"), SAVE, &v));
}

/// Read them back (app start): the settings, and the sound into the mic service.
pub fn load(m: &MicMute) {
    let Some(list) = crate::services::with(|sv| sv.store.get_list(crate::settings::Scope::Page("aud"), SAVE).map(|l| l.to_vec())).flatten() else { return };
    let mut s = MuteSet::default();
    for l in &list {
        let Some((k, v)) = l.split_once('=') else { continue };
        let n = |d: usize| v.parse::<usize>().unwrap_or(d);
        match k {
            "on" => s.on = v == "1",
            "sep" => s.sep = v == "1",
            "icon" => s.icon = n(1).min(2),
            "style" => s.style = n(0).min(2),
            "size" => s.size = n(1).min(2),
            "op" => s.op = v.parse::<f32>().unwrap_or(1.0).clamp(0.3, 1.0),
            // before Order 040 "mon" was the list row: All monitors = the monitor count then
            "mon" => s.mon = micicon::from_old_row(n(0), micicon::monitor_count()),
            "mon2" => s.mon = n(0),
            "spot" => {
                let p: Vec<&str> = v.split(',').collect();
                if let [h, dx, vv, dy] = p[..] {
                    if let (Some(h), Ok(dx), Some(vv), Ok(dy)) = (h.chars().next(), dx.parse(), vv.chars().next(), dy.parse()) {
                        s.spot = micicon::Spot { h, dx, v: vv, dy };
                    }
                }
            }
            // Order 046: the sound is off by default at 5 %. `snd=` is the old format (sound on, 60 %, a linear volume
            // scale): its picks are kept but the sound is turned OFF once and the volume goes to the new default
            // (the numbers meant something else then). `snd2=` is what is saved now.
            "snd" if !list.iter().any(|l| l.starts_with("snd2=")) => {
                let p: Vec<usize> = v.split(',').filter_map(|x| x.parse().ok()).collect();
                if let [_, a, b, _] = p[..] {
                    let pick = |i: usize| Sound::ALL.get(i).copied().unwrap_or(Sound::None);
                    m.set_sound(SoundSettings { enabled: false, on_mute: pick(a), on_unmute: pick(b), volume: bu_micmute::sound::DEFAULT_VOLUME });
                }
            }
            "snd2" => {
                let p: Vec<usize> = v.split(',').filter_map(|x| x.parse().ok()).collect();
                if let [en, a, b, vol] = p[..] {
                    let pick = |i: usize| Sound::ALL.get(i).copied().unwrap_or(Sound::None);
                    m.set_sound(SoundSettings { enabled: en == 1, on_mute: pick(a), on_unmute: pick(b), volume: vol.min(100) as u8 });
                }
            }
            _ => {}
        }
    }
    SET.with(|x| *x.borrow_mut() = s);
    sync_keys();
}

thread_local! {
    static SVC: RefCell<Option<MicMute>> = const { RefCell::new(None) };
    static SET: RefCell<MuteSet> = RefCell::new(MuteSet::default());
}

/// Another app muted / unmuted the mic (bu-micmute's watch): the page reads the state again.
static CHANGED: AtomicBool = AtomicBool::new(false);

/// The mic mute service (made once: the FAKE in test copies, read-only in `--real-read` copies).
pub fn service(fake: bool, frozen: bool, read_only: bool) -> MicMute {
    SVC.with(|s| {
        let mut s = s.borrow_mut();
        if let Some(m) = s.as_ref() {
            return m.clone();
        }
        let m = make(fake, frozen, read_only);
        *s = Some(m.clone());
        m
    })
}

fn make(fake: bool, frozen: bool, read_only: bool) -> MicMute {
    #[cfg(windows)]
    if !fake {
        use bu_micmute::real::{RealMicOs, RealSoundOut};
        let os = if read_only { RealMicOs::read_only() } else { RealMicOs::new() };
        return MicMute::new(Arc::new(os), Arc::new(RealSoundOut::new()));
    }
    let _ = read_only;
    // the drawing's mics (Audio's Input list)
    let os = FakeMicOs::new(&[("mv7", "Microphone (Shure MV7)"), ("arctis-mic", "Headset Microphone (Arctis Nova)"), ("c920", "Webcam Microphone (C920)")]);
    let _ = (fake, frozen);
    MicMute::new(Arc::new(os), Arc::new(FakeSoundOut::default()))
}

pub fn settings() -> MuteSet {
    SET.with(|s| s.borrow().clone())
}
fn set(f: impl FnOnce(&mut MuteSet)) {
    SET.with(|s| f(&mut s.borrow_mut()));
}

/// Settings › "Reset the app's own settings": the store's Audio part is wiped - the live values go back too (Mic mute
/// off and the mic left unmuted, default sounds), else the next change would save the old ones again.
pub fn reset_to_defaults(fake: bool) {
    SET.with(|s| *s.borrow_mut() = MuteSet::default());
    let m = service(fake, false, false);
    let _ = m.turn_off();
    m.set_sound(SoundSettings::default());
    sync_keys();
    sync_icon(mic_muted(&m), false, false, false);
}

/// Test copies start each run from the drawing's defaults.
pub fn reset_for_test() {
    SET.with(|s| *s.borrow_mut() = MuteSet::default());
    SVC.with(|s| *s.borrow_mut() = None);
}

/// The popup lists the window can open (a dropdown over the window).
#[derive(Clone, Copy, Debug, PartialEq)]
enum List {
    OnMute,
    OnUnmute,
    ShowOn,
}

/// The popup while it is open (dropped when it closes).
pub struct MuteUi {
    pub opened_at: f64,
    mic: MicMute,
    muted: bool,
    /// which key field listens (its key) and since when
    list: Option<(List, f32, f32, f32)>,
    rects: HashMap<Key, (f32, f32, f32, f32)>,
    moving: bool,
    /// the ▶ that just played (blue for 300 ms)
    played: Option<(Key, f64)>,
    stp_since: f64,
    monitors: Vec<String>,
    /// Order 047: a real mic (Core Audio, 10-50 ms a read): its reads and the pill's toggle run off the menu's thread
    real: bool,
    /// the last read off the menu's thread, for the next `refresh`
    read: Arc<std::sync::Mutex<Option<bool>>>,
}

/// How the mic is now (the Input row's icon and the pill).
pub fn mic_muted(m: &MicMute) -> bool {
    let v = m.state().map(|s| s.muted).unwrap_or(false);
    LAST_MUTED.store(v, Ordering::Relaxed);
    v
}

impl MuteUi {
    pub fn open(mic: MicMute, now: f64, frozen: bool) -> MuteUi {
        let muted = mic_muted(&mic);
        MuteUi::make(mic, now, frozen, muted, false)
    }

    /// Order 047: a real mic - the popup opens at once on the page's last known state (`muted`); a fresh read runs off
    /// the menu's thread and `refresh` shows it.
    pub fn open_real(mic: MicMute, now: f64, frozen: bool, muted: bool) -> MuteUi {
        let u = MuteUi::make(mic, now, frozen, muted, true);
        u.read_off();
        u
    }

    fn make(mic: MicMute, now: f64, frozen: bool, muted: bool, real: bool) -> MuteUi {
        let monitors = if frozen { vec!["Main (DELL 27)".into(), "Second (LG 24)".into(), "All monitors".into()] } else { monitors() };
        sync_icon(muted, true, false, false);
        MuteUi { opened_at: now, mic, muted, list: None, rects: HashMap::new(), moving: false, played: None, stp_since: -1e9, monitors, real, read: Arc::default() }
    }

    /// Read the mic's state into `read`: off the menu's thread for a real mic, in place for the fake.
    fn read_off(&self) {
        if self.real {
            // on the key worker: in order with the presses (an older read never lands last)
            read_state(&self.mic, self.read.clone());
        } else if let Ok(mut s) = self.read.lock() {
            *s = Some(mic_muted(&self.mic));
        }
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    /// The popup window (window coordinates), with an open list above it.
    pub fn popup(&mut self, cx: &mut Cx) -> El {
        let s = settings();
        let snd = self.mic.sound();
        let ready = keys_ready(&s);
        let now = cx.now;
        let age = now - self.opened_at;
        // ---- the head: `.mmd .ch{min-height:56px}` `.row.ch` padding 10px 12px, `.ch .lbl.ap{gap:12px}`
        let mut right = Vec::new();
        if ready {
            right.push(self.pill(cx));
        }
        right.push(toggle::toggle(cx, K_ON, s.on, false));
        let head = card::card_head("mic", "Mic mute", Some("A key that mutes your mic everywhere"), vec![group::ctl(right)]).min_h(56.0);
        // ---- the rows (`.lockb`): staggered slide-in when the popup opens (340 ms, delay 70 + 60 i, ease-out)
        let mut rows: Vec<El> = Vec::new();
        let mut lines: Vec<El> = Vec::new();
        let rm = cx.rm;
        let stag = move |i: usize, e: El| -> El {
            if rm || age > 70.0 + 60.0 * i as f64 + 340.0 + 50.0 {
                return e;
            }
            let p = ((age - 70.0 - 60.0 * i as f64) / 340.0).clamp(0.0, 1.0);
            let t = crate::anim::EASE_OUT.ease(p) as f32;
            e.opacity(t).translate(0.0, -8.0 * (1.0 - t))
        };
        let fields: &[(Key, &str, Option<(Key, &str)>)] = if !s.sep {
            &[(K_KEY_ONE, "Mute key", Some((K_SEP, "Separate keys")))]
        } else {
            &[(K_KEY_MUTE, "Mute key", Some((K_ONE, "One key"))), (K_KEY_UNMUTE, "Unmute key", None)]
        };
        let mut err = None;
        for (i, (k, title, mode)) in fields.iter().enumerate() {
            let (f, e) = self.field(cx, *k);
            err = err.or(e);
            let r = key_row(cx, false, title, *mode, f);
            lines.push(if i == 0 { stag(0, r) } else { r });
        }
        rows.extend(lines);
        // `.klk`: the keys manager's refusal ("Already used by Voice to text", "Windows has this key") under the keys
        if let Some(e) = err {
            rows.push(El::text(e, Font::new(11.0, 400), RED(), lh(11.0, 1.35)).wrapping().pad(0.0, 12.0, 8.0, 12.0));
        }
        // Sound: `row('Sound',[sndChange, toggle])`
        let chg = if snd.enabled { link::link(cx, K_SND_CHG, if s.snd_open { "Hide" } else { "Change" }, 12.0) } else { link::link(cx, K_SND_CHG, "Change", 12.0).opacity(0.0).no_hit() };
        rows.push(stag(1, plain_row(false, "Sound", None, vec![chg, toggle::toggle(cx, K_SND, snd.enabled, false)])));
        // Order 092: the Volume slider sits right under the switch while the sound is on (it used to hide behind "Change")
        let vol_row = plain_row(false, "Volume", None, vec![sl(cx, K_VOL, snd.volume as f32 / 100.0, &format!("{} %", snd.volume))]);
        let vt = card::fold_t(cx, sub(K_MM, "xpvol"), snd.enabled);
        rows.push(card::drop_out(cx, vol_row, 406.0, vt));
        let snd_rows = El::col()
            .items(AlignItems::STRETCH)
            .child(plain_row(false, "On mute", None, vec![self.pick(cx, K_SND_MUTE, snd.on_mute.label()), self.pb(cx, K_PV_MUTE, snd.on_mute.can_preview())]))
            .child(plain_row(false, "On unmute", None, vec![self.pick(cx, K_SND_UNMUTE, snd.on_unmute.label()), self.pb(cx, K_PV_UNMUTE, snd.on_unmute.can_preview())]));
        let ft = card::fold_t(cx, sub(K_MM, "xpsnd"), snd.enabled && s.snd_open);
        rows.push(card::drop_out(cx, snd_rows, 406.0, ft));
        // Icon on screen
        let icon_seg = seg::seg(cx, K_ICON, &ICON_MODES, s.icon, true);
        rows.push(stag(2, plain_row(false, "Icon on screen", Some("Shows Live / Muted on your screen"), vec![icon_seg])));
        let isub = self.isub(cx, &s);
        let ft = card::fold_t(cx, sub(K_MM, "xpico"), s.icon != 0);
        rows.push(card::drop_out(cx, isub, 406.0, ft));
        // `.lockb{transition:opacity .25s ease,filter .25s ease}` `.mmd.locked .lockb{opacity:.36;filter:grayscale(1);pointer-events:none}`
        let lk = cx.tr(sub(K_MM, "lock"), 1, if s.on { 1.0 } else { 0.36 }, 250.0, EASE);
        let mut lockb = El::col().items(AlignItems::STRETCH).children(rows).opacity(lk);
        if lk < 0.999 {
            lockb = lockb.color_filter(crate::gfx::CssColor::Grayscale(((1.0 - lk) / 0.64).clamp(0.0, 1.0)));
        }
        if !s.on {
            lockb = lockb.no_hit();
        }
        // the card `.grp.mmd`
        let body = group::grp(vec![head, lockb]);
        // `.mdb{overflow-y:auto;scrollbar-width:thin}`: 401 px tall in the 468 px window; when its content is taller it
        // scrolls (the wheel over it) with the slim inner thumb, which takes 10 px of its width (measured: .mdb 430 wide,
        // the card 384 = 430 - 36 - 10)
        let full = crate::ui::lay::Laid::new(cx.g, body.clone(), 394.0, None).height + 2.0;
        let bar = if full > 401.0 { 10.0 } else { 0.0 };
        let mdb = cx
            .scroll_box(sub(K_MM, "mdb"), vec![body])
            .items(AlignItems::STRETCH)
            .pad(0.0, 18.0 + bar, 2.0, 18.0)
            .margin(0.0, -18.0, 0.0, -18.0)
            .slim_thumb(crate::ui::el::SlimThumb::GLASS)
            .style(|st| {
                st.flex_shrink = 1.0;
                st.min_size.height = taffy::style::LengthPercentageAuto::length(0.0);
            });
        let dlg = dialog::dialog(cx, K_MM, 430.0, "Mute settings", vec![mdb], vec![], true, self.opened_at);
        let mut kids = vec![dlg];
        if let Some((l, x, y, w)) = self.list {
            let (items, cur): (Vec<String>, usize) = match l {
                List::OnMute => (Sound::ALL.iter().map(|s| s.label().to_string()).collect(), Sound::ALL.iter().position(|x| *x == snd.on_mute).unwrap_or(0)),
                List::OnUnmute => (Sound::ALL.iter().map(|s| s.label().to_string()).collect(), Sound::ALL.iter().position(|x| *x == snd.on_unmute).unwrap_or(0)),
                List::ShowOn => (self.monitors.clone(), micicon::list_index(s.mon, self.mon_count())),
            };
            let it: Vec<Item> = items.into_iter().enumerate().map(|(i, label)| Item { label, checked: i == cur, disabled: false }).collect();
            let h = 10.0 + 26.0 * it.len() as f32;
            let (x, y) = place(x, y, w, 150.0f32.max(w), h);
            kids.push(dropdown::menu(cx, K_LIST, &it, x, y, 150.0f32.max(w)).z(20));
        }
        El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(WIN_W, WIN_H).children(kids)
    }

    /// `.stp` - the Live / Muted pill (click = mute / unmute now). Pops in (scale .85 -> 1, .3 s, delay .12 s) when it shows.
    fn pill(&mut self, cx: &mut Cx) -> El {
        let hv = cx.hover_t(K_STP, 300.0, EASE);
        let m = cx.tr(K_STP, 5, if self.muted { 1.0 } else { 0.0 }, 300.0, EASE);
        let bg = cmix(cmix(CTL(), CTL_H(), hv), Rgba::rgba(255, 69, 58, 0.16), m);
        let txt = if self.muted { "Muted" } else { "Live" };
        // `.stp{height:22px;padding:0 9px 0 8px;border-radius:11px;gap:6px;box-shadow:inset 0 0 0 .5px var(--hair);font-size:11.5px;font-weight:600}`
        // `.stp i{width:7px;height:7px;border-radius:50%;background:var(--green)}` `.stp.m i{background:var(--red)}`
        let pop = if cx.rm { 1.0 } else { POP.ease(((cx.now - self.stp_since - 120.0) / 300.0).clamp(0.0, 1.0)) as f32 };
        if cx.now - self.stp_since < 450.0 {
            cx.st.busy = true;
        }
        El::row()
            .center()
            .gap(6.0)
            .h(22.0)
            .none()
            .pad(0.0, 9.0, 0.0, 8.0)
            .radius(11.0)
            .bg(bg)
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
            .opacity(pop.min(1.0))
            .scale(0.85 + 0.15 * pop)
            .on_click(K_STP)
            .cursor(Cursor::Hand)
            // `title:'Click to mute or unmute now'`
            .title("Click to mute or unmute now")
            .child(El::block().size(7.0, 7.0).none().radius(RADIUS_PILL).bg(cmix(GREEN(), RED(), m)))
            .child(El::text(txt, btn_font(11.5, 600), FG(), lh(11.5, 1.35)))
    }

    /// A key field on the keys manager (it listens, refuses keys used elsewhere, registers the key with Windows): the
    /// field and its refusal, if any.
    fn field(&mut self, cx: &mut Cx, k: Key) -> (El, Option<String>) {
        let (mut set, mut listening, mut err) = cx.key_field(action_of(k));
        // Order 045: separate keys - the Mute key was just taken and there is no Unmute key yet: the Unmute field starts
        // listening by itself (`commitCap`: `if(S.keyMode==='sep'&&f.slot==='mute'&&!S.keys.unmute)startCap(fUnmute)`)
        if k == K_KEY_MUTE && listening.is_none() && set.is_some() {
            if let Some(&since) = cx.st.kf_listen.get(&K_KEY_MUTE) {
                let bound = crate::services::with(|s| s.key_bound_at).flatten();
                if bound.is_some_and(|t| t >= since) && cx.key_field(A_UNMUTE).0.is_none() {
                    cx.listen_key(A_UNMUTE);
                    (set, listening, err) = cx.key_field(action_of(k));
                }
            }
        }
        let show = match (&listening, &set) {
            (Some((held, _)), _) => Show::Listening(held.as_deref()),
            (None, Some(s)) => Show::Set(s),
            _ => Show::Empty,
        };
        let since = listening.as_ref().map(|l| l.1).unwrap_or(0.0);
        (keyfield::keyfield(cx, k, show, since, false), err)
    }

    /// `cPopup(..,'w')` = `.pu.w{width:128px}`
    fn pick(&mut self, cx: &mut Cx, k: Key, label: &str) -> El {
        dropdown::dropdown(cx, k, label, Some(128.0))
    }

    /// `.pb` - the ▶ preview (the shared `rowbits::pb`; the mute sound "plays" 260 ms, the unmute one 300 ms).
    fn pb(&mut self, cx: &mut Cx, k: Key, enabled: bool) -> El {
        let dir = if k == K_PV_MUTE { -1 } else { 1 };
        let since = self.played.filter(|(p, t)| *p == k && cx.now - t < rowbits::play_ms(dir)).map(|p| p.1);
        if let Some(t) = since {
            // Order 047: "playing" is a state with a known end, not motion - built again at its end (the button's own
            // colour fade asks for its frames while it moves)
            cx.wake_at(t + rowbits::play_ms(dir));
        }
        rowbits::pb(cx, k, since, dir, !enabled)
    }

    /// The icon's own small section `.isub` (Position, Style, Size, Opacity, Show on, the fullscreen note).
    fn isub(&mut self, cx: &mut Cx, s: &MuteSet) -> El {
        // Position: the corner picker `.cpk` + the "Move icon" button
        let mut cpk = El::block().size(44.0, 28.0).none().radius(5.0).inset(&[sh(0.0, 0.0, 0.0, 1.2, FG3())]);
        for (i, (_, h, v)) in SPOTS.iter().enumerate() {
            let k = idx(K_CPK, i);
            let hv = cx.hover_t(k, 150.0, EASE);
            let on = micicon::quick_of(s.spot) == Some(i);
            let c = if on { ACC() } else { cmix(FG3(), FG2(), hv) };
            // `.cpk button{width:14px;height:10px}` `i{width:8px;height:4px;border-radius:2px}` `:hover i{transform:scale(1.15)}`
            let l = match h {
                'L' => 2.0,
                'C' => 15.0,
                _ => 44.0 - 2.0 - 14.0,
            };
            let t = if *v == 'T' { 2.0 } else { 28.0 - 2.0 - 10.0 };
            cpk = cpk.child(
                El::grid()
                    .abs(l, t, f32::NAN, f32::NAN)
                    .size(14.0, 10.0)
                    .place_center()
                    .on_click(k)
                    .cursor(Cursor::Hand)
                    // `title:SPOTN[s[0]]` ("Top left" …)
                    .title(SPOTS[i].0)
                    .child(El::block().size(8.0, 4.0).radius(2.0).bg(c).scale(1.0 + 0.15 * hv).no_hit()),
            );
        }
        // `.btn.mvb{min-width:102px;justify-content:center}`
        let mv = button::btn(cx, K_MOVE, "move", if self.moving { "Done" } else { "Move icon" }, self.moving).min_w(102.0).justify(taffy::style::JustifyContent::CENTER);
        let rows = vec![
            sub_row(true, "Position", Some("Drag it on your screen"), vec![cpk, mv]),
            sub_row(false, "Style", None, vec![seg::seg(cx, K_STYLE, &STYLES, s.style, true)]),
            sub_row(false, "Size", None, vec![seg::seg(cx, K_SIZE, &SIZES, s.size, false).min_w(108.0)]),
            sub_row(false, "Opacity", None, vec![sl(cx, K_OP, (s.op - 0.3) / 0.7, &format!("{} %", (s.op * 100.0).round()))]),
            sub_row(false, "Show on", None, vec![self.pick(cx, K_MON, &self.monitors.get(micicon::list_index(s.mon, self.mon_count())).cloned().unwrap_or_else(|| "Main".into()))]),
            inote::inote("Can\u{2019}t show over games in exclusive fullscreen \u{2014} use borderless / windowed fullscreen.", false, &inote::ROW_WRAP),
        ];
        // `.isub{margin:0 12px 12px 56px}` `.mmd .isub{margin-left:24px}` `border-radius:9px;background:var(--grp);box-shadow:inset 0 0 0 .5px var(--hair)`
        El::col().items(AlignItems::STRETCH).margin(0.0, 12.0, 12.0, 24.0).radius(9.0).bg(GRP()).inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())]).children(rows)
    }

    /// Input on the popup's elements. Returns false when the popup should close.
    pub fn event(&mut self, ev: &Ev, cx: &mut Cx) -> bool {
        match ev {
            Ev::Press(k, _, _, r) => {
                self.rects.insert(*k, *r);
                if *k == K_VOL || *k == K_OP {
                    self.slide(*k, slider::value_at(*r, ev_x(ev)));
                }
            }
            Ev::Drag(k, x, _, r) if *k == K_VOL || *k == K_OP => self.slide(*k, slider::value_at(*r, *x)),
            Ev::Release(k) if *k == K_VOL => {
                // letting go of the volume slider: a short blip at the new volume
                let snd = self.mic.sound();
                let s = if self.muted { snd.on_mute } else { snd.on_unmute };
                if snd.enabled {
                    let _ = self.mic.preview(s);
                }
                save(Some(&self.mic));
            }
            Ev::Release(k) if *k == K_OP => save(Some(&self.mic)),
            // a key field lost the focus (a click elsewhere): it stops listening
            Ev::Blur(k) if matches!(*k, K_KEY_ONE | K_KEY_MUTE | K_KEY_UNMUTE) => cx.stop_listening(),
            Ev::Click(k) => {
                let was = self.muted;
                let keep = self.click(*k, cx);
                if !keep {
                    cx.stop_listening();
                }
                save(Some(&self.mic));
                sync_icon(self.muted, keep, self.moving, was != self.muted);
                return keep;
            }
            _ => {}
        }
        true
    }

    fn slide(&mut self, k: Key, v: f32) {
        if k == K_VOL {
            let mut snd = self.mic.sound();
            snd.volume = (v * 100.0).round() as u8;
            self.mic.set_sound(snd);
        } else {
            set(|s| s.op = ((0.3 + 0.7 * v) * 100.0).round() / 100.0);
        }
    }

    fn click(&mut self, k: Key, cx: &mut Cx) -> bool {
        let now = cx.now;
        if k == sub(K_MM, "x") || k == sub(K_MM, "out") {
            if self.list.take().is_some() && k == sub(K_MM, "out") {
                return true;
            }
            return false;
        }
        // a click anywhere closes an open list (and still does what it was on)
        if let Some((l, ..)) = self.list.take() {
            if let Some(i) = (0..8).find(|&i| idx(K_LIST, i) == k) {
                self.pick_item(l, i);
                return true;
            }
        }
        let s = settings();
        match k {
            K_ON => {
                let on = !s.on;
                set(|x| x.on = on);
                sync_keys();
                if !on {
                    // switched off: it all folds away, the mic is never left muted (bu-micmute unmutes silently)
                    cx.stop_listening();
                    self.moving = false;
                    if let Ok(st) = self.mic.turn_off() {
                        self.muted = st.muted;
                    }
                }
            }
            K_STP => self.toggle(now),
            K_SEP | K_ONE => {
                let sep = k == K_SEP;
                cx.stop_listening();
                // the key goes along: one key -> the mute key, and back
                if sep {
                    move_key(A_ONE, A_MUTE);
                } else {
                    move_key(A_MUTE, A_ONE);
                }
                set(|x| x.sep = sep);
                sync_keys();
            }
            K_KEY_ONE | K_KEY_MUTE | K_KEY_UNMUTE => {
                if cx.key_field(action_of(k)).1.is_none() {
                    cx.listen_key(action_of(k));
                }
            }
            _ if k == sub(K_KEY_ONE, "clr") || k == sub(K_KEY_MUTE, "clr") || k == sub(K_KEY_UNMUTE, "clr") => {
                let a = if k == sub(K_KEY_ONE, "clr") {
                    A_ONE
                } else if k == sub(K_KEY_MUTE, "clr") {
                    A_MUTE
                } else {
                    A_UNMUTE
                };
                cx.stop_listening();
                cx.clear_key(a);
            }
            K_SND => {
                let mut snd = self.mic.sound();
                snd.enabled = !snd.enabled;
                self.mic.set_sound(snd);
                if !snd.enabled {
                    set(|x| x.snd_open = false);
                }
            }
            K_SND_CHG => set(|x| x.snd_open = !x.snd_open),
            K_SND_MUTE | K_SND_UNMUTE | K_MON => {
                let l = match k {
                    K_SND_MUTE => List::OnMute,
                    K_SND_UNMUTE => List::OnUnmute,
                    _ => List::ShowOn,
                };
                let r = self.rects.get(&k).copied().unwrap_or((0.0, 0.0, 128.0, 24.0));
                self.list = Some((l, r.0, r.1 + r.3 + 4.0, r.2));
            }
            K_PV_MUTE | K_PV_UNMUTE => {
                let snd = self.mic.sound();
                let s = if k == K_PV_MUTE { snd.on_mute } else { snd.on_unmute };
                if s.can_preview() {
                    let _ = self.mic.preview(s);
                    self.played = Some((k, now));
                }
            }
            K_MOVE => self.moving = !self.moving,
            _ => {
                if let Some(i) = (0..3).find(|&i| idx(K_ICON, i) == k) {
                    set(|x| x.icon = i);
                    if i == 0 {
                        self.moving = false;
                    }
                } else if let Some(i) = (0..3).find(|&i| idx(K_STYLE, i) == k) {
                    set(|x| x.style = i);
                } else if let Some(i) = (0..3).find(|&i| idx(K_SIZE, i) == k) {
                    set(|x| x.size = i);
                } else if let Some(i) = (0..6).find(|&i| idx(K_CPK, i) == k) {
                    set(|x| x.spot = micicon::quick(i));
                }
            }
        }
        true
    }

    /// How many monitors the "Show on" list was made for (its rows minus "All monitors").
    fn mon_count(&self) -> usize {
        if self.monitors.len() > 1 { self.monitors.len() - 1 } else { 1 }
    }
    fn pick_item(&mut self, l: List, i: usize) {
        match l {
            List::OnMute | List::OnUnmute => {
                let Some(&s) = Sound::ALL.get(i) else { return };
                let mut snd: SoundSettings = self.mic.sound();
                if l == List::OnMute {
                    snd.on_mute = s;
                } else {
                    snd.on_unmute = s;
                }
                self.mic.set_sound(snd);
                // picking a sound plays it (the drawing: playSound on pick)
                let _ = self.mic.preview(s);
            }
            List::ShowOn => {
                let n = self.mon_count();
                set(|x| x.mon = micicon::stored(i, n));
            }
        }
    }

    /// Mute / unmute now (the Input row's mic icon, the pill): bu-micmute toggles the mic with its sound.
    pub fn toggle(&mut self, now: f64) {
        if let Some(m) = toggle_now(&self.mic, self.muted, !self.real, false) {
            self.muted = m;
        }
        self.stp_since = self.stp_since.min(now - 1e6);
    }

    /// Esc / a click beside the window.
    pub fn dismiss(&mut self) -> bool {
        self.list.take().is_some()
    }

    /// Another app (or the key / the button) changed the mic: its state read again - off the menu's thread for a real mic
    /// (Order 047), shown at a later tick. True = the state shown changed.
    pub fn refresh(&mut self) -> bool {
        if CHANGED.swap(false, Ordering::Relaxed) {
            self.read_off();
        }
        let Some(v) = self.read.lock().ok().and_then(|mut s| s.take()) else { return false };
        let was = self.muted;
        self.muted = v;
        sync_icon(self.muted, true, self.moving, was != self.muted);
        was != self.muted
    }

    pub fn describe(&self) -> String {
        let s = settings();
        let snd = self.mic.sound();
        format!(
            "mute on={} sep={} key={} muted={} sound={} on_mute={} on_unmute={} vol={} icon={} spot={} style={} size={} op={} mon={} list={:?}",
            s.on,
            s.sep,
            bound(A_ONE).unwrap_or_default(),
            self.muted,
            snd.enabled,
            snd.on_mute.label(),
            snd.on_unmute.label(),
            snd.volume,
            ICON_MODES[s.icon],
            micicon::quick_of(s.spot).map(|i| SPOTS[i].0).unwrap_or("dragged"),
            STYLES[s.style],
            SIZES[s.size],
            (s.op * 100.0).round(),
            s.mon,
            self.list.map(|l| l.0)
        )
    }
}

/// The on-screen icon follows the settings and the mic: `popup` = Mute settings is open (its preview), `changed` = the
/// mic was just muted / unmuted ("When it changes" flashes it for 1.5 s).
pub fn sync_icon(muted: bool, popup: bool, moving: bool, changed: bool) {
    // Order 045: the tray icon's red badge while muted (`byId('btb').classList.toggle('on',S.muted)`)
    crate::tray::set_muted(muted);
    let s = settings();
    let ready = keys_ready(&s);
    let want = micicon::Want { active: ready && s.icon != 0, always: s.icon == 2, preview: popup && s.on && s.icon != 0, moving: moving && s.icon != 0 };
    let look = micicon::Look { muted, style: s.style, size: s.size, op: s.op, moving: moving && s.icon != 0 };
    micicon::set_monitor(s.mon);
    micicon::update(want, look, s.spot, changed);
}

/// The icon was dragged on the screen (the overlay window's own drag).
pub fn set_spot(sp: micicon::Spot) {
    set(|x| x.spot = sp);
    save(None);
}

/// While the Audio page shows: another app muting the mic updates the icon (event-driven, bu-micmute's watch). Order 055:
/// the change wakes the menu (it used to be found by the page's 16 ms polling).
pub fn watch(m: &MicMute) {
    let _ = m.start_watching(Arc::new(|_| {
        CHANGED.store(true, Ordering::Relaxed);
        crate::services::Waker.wake();
    }));
}
/// Another app changed the mic since the last look (the page reads the state again).
pub fn take_changed() -> bool {
    CHANGED.swap(false, Ordering::Relaxed)
}
pub fn unwatch(m: &MicMute) {
    m.stop_watching();
}

fn ev_x(ev: &Ev) -> f32 {
    match ev {
        Ev::Press(_, x, ..) | Ev::Drag(_, x, ..) => *x,
        _ => 0.0,
    }
}

/// The drawing's placeMenu: under the button, kept 8 px inside the window (above it when there is no room below).
fn place(x: f32, y: f32, bw: f32, mw: f32, mh: f32) -> (f32, f32) {
    let mut l = x;
    let mut t = y;
    if l + mw > WIN_W - 8.0 {
        l = (x + bw - mw).max(8.0);
    }
    if t + mh > WIN_H - 8.0 {
        t = (y - 4.0 - 24.0 - mh - 4.0).max(8.0);
    }
    (l, t)
}

/// `row(label, ctl, sub)` inside the card: `.mmd .lockb .row{padding-left:12px}`.
fn plain_row(first: bool, title: &str, small: Option<&str>, ctl: Vec<El>) -> El {
    group::row(first, vec![lbl(title, small), group::ctl(ctl)])
}

/// `.lbl{flex:1;min-width:0;font-size:13px}` `.lbl small{display:block;font-size:11px;color:var(--fg2);margin-top:1px}` - the
/// line under the title wraps when the controls leave it little room ("Shows Live / Muted on / your screen").
fn lbl(title: &str, small: Option<&str>) -> El {
    let mut l = El::col().flex1().child(El::text(title, Font::new(13.0, 400), FG(), lh(13.0, 1.35)).wrapping());
    if let Some(s) = small {
        l = l.child(El::text(s, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).wrapping().margin(1.0, 0.0, 0.0, 0.0));
    }
    l
}

/// `.mmd .isub .row{padding-left:12px;min-height:40px;padding-top:6px;padding-bottom:6px}`
fn sub_row(first: bool, title: &str, small: Option<&str>, ctl: Vec<El>) -> El {
    plain_row(first, title, small, ctl).min_h(40.0).pad(6.0, 12.0, 6.0, 12.0)
}

/// A key row: "Mute key" + `.kmode` ("·" then the link, `margin-left:6px` each, --fg3) and the key field.
fn key_row(cx: &mut Cx, first: bool, title: &str, mode: Option<(Key, &str)>, field: El) -> El {
    let mut l = El::row().items(AlignItems::BASELINE).flex1().child(El::text(title, Font::new(13.0, 400), FG(), lh(13.0, 1.35)).none());
    if let Some((k, t)) = mode {
        // `lSep.title='Use separate mute and unmute keys'` `lOne.title='Use one key to mute and unmute'`
        let lt = if k == K_SEP { "Use separate mute and unmute keys" } else { "Use one key to mute and unmute" };
        let km = El::row()
            .items(AlignItems::BASELINE)
            .none()
            .margin(0.0, 0.0, 0.0, 6.0)
            .child(El::text("\u{00b7}", Font::new(13.0, 400), FG3(), lh(13.0, 1.35)))
            .child(link::link(cx, k, t, 12.0).margin(0.0, 0.0, 0.0, 6.0).title(lt));
        l = l.child(km);
    }
    group::row(first, vec![l, group::ctl(vec![field])])
}

/// `cSlider(...)`: `.sl{display:flex;align-items:center;gap:10px}` = the 150 x 20 range + its `.sv`.
fn sl(cx: &mut Cx, k: Key, v: f32, label: &str) -> El {
    El::row().center().gap(10.0).none().child(slider::slider(cx, k, v, 150.0, 20.0, slider::default())).child(slider::value_label(label))
}


/// The monitors for "Show on" (real runs): "Main" first, each other one numbered left to right, then "All monitors" when
/// there are several (micicon.rs places the icon on the same list).
fn monitors() -> Vec<String> {
    micicon::monitor_labels(micicon::monitor_count())
}

#[allow(dead_code)]
const _WHITE: Rgba = WHITE;

#[cfg(test)]
mod tests {
    use super::*;
    use bu_micmute::os::{EventSink, Watch};
    use bu_micmute::{MicDevice, MicOs};

    /// The fake mic stack, as slow as Core Audio on a busy PC (30 ms a call).
    struct SlowMic(FakeMicOs);
    fn slow() {
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    impl MicOs for SlowMic {
        fn capture_devices(&self) -> bu_micmute::Result<Vec<MicDevice>> {
            slow();
            self.0.capture_devices()
        }
        fn default_capture(&self) -> bu_micmute::Result<Option<String>> {
            slow();
            self.0.default_capture()
        }
        fn is_muted(&self, id: &str) -> bu_micmute::Result<bool> {
            slow();
            self.0.is_muted(id)
        }
        fn set_muted(&self, id: &str, muted: bool) -> bu_micmute::Result<()> {
            slow();
            self.0.set_muted(id, muted)
        }
        fn watch_mute(&self, id: &str, sink: EventSink) -> bu_micmute::Result<Box<dyn Watch>> {
            self.0.watch_mute(id, sink)
        }
        fn watch_devices(&self, sink: EventSink) -> bu_micmute::Result<Box<dyn Watch>> {
            self.0.watch_devices(sink)
        }
    }

    /// the key worker's counters are the app's own (one at a time here)
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn wait(f: impl Fn() -> bool) -> bool {
        let t0 = std::time::Instant::now();
        while !f() && t0.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        f()
    }

    /// Order 047: the mute key hands its Core Audio work (here 4 slow calls = 120 ms) to its own thread - the menu's
    /// thread is back within one frame - and the mic is still muted, then unmuted by the next press, in order.
    #[test]
    fn the_mute_key_never_holds_the_menu() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let f = FakeMicOs::new(&[("mv7", "Microphone (Shure MV7)")]);
        let m = MicMute::new(Arc::new(SlowMic(f.clone())), Arc::new(FakeSoundOut::default()));
        crate::offui::assert_quick("the mic mute key", || send_press(m.clone(), A_ONE, false));
        assert!(wait(|| f.muted("mv7")), "the key still mutes");
        crate::offui::assert_quick("the mic mute key again", || send_press(m.clone(), A_ONE, false));
        assert!(wait(|| !f.muted("mv7") && KEY_PENDING.load(Ordering::Acquire) == 0), "the next press unmutes");
        assert_eq!(f.sets(), vec![("mv7".to_string(), true), ("mv7".to_string(), false)], "one after another");
        assert_eq!(KEY_DONE.lock().unwrap().take(), Some(false), "the icon gets the last answer");
    }

    /// Order 047: the Audio page's mic button (and Mute settings' pill) on a real mic: the flip shows at once, the
    /// Core Audio work (here 120 ms) runs on the key's worker - the menu's thread is back within one frame - and the mic is
    /// muted.
    #[test]
    fn the_mic_button_never_holds_the_menu() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let f = FakeMicOs::new(&[("mv7", "Microphone (Shure MV7)")]);
        let m = MicMute::new(Arc::new(SlowMic(f.clone())), Arc::new(FakeSoundOut::default()));
        let shown = crate::offui::assert_quick("the mic button", || toggle_now(&m, false, false, false));
        assert_eq!(shown, Some(true), "the button shows Muted at once");
        assert!(wait(|| f.muted("mv7") && KEY_PENDING.load(Ordering::Acquire) == 0), "the mic is muted by the worker");
        KEY_DONE.lock().unwrap().take();
    }
}
