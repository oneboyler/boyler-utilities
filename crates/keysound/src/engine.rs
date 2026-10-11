//! The key-sound engine (Windows): wires the raw-input sink, the rules, the mixer and the one output stream together.
//!
//! Path of a key press (all on the "bu-rawin" thread, no message, no queue):
//! `bu-rawin` reads the key packet → [`bu_rawin::SoundEvent`] (class + up / down; the key is already forgotten) →
//! [`Shared::on_event`]: what is in front ([`crate::front`]), the pack ([`crate::rules::choose`]), a voice in the mixer
//! (a shared pointer to the pre-made sound + a slightly random speed) → one `SetEvent` wakes the render thread.
//!
//! The render thread ("bu-keysound") exists only while the sounds are switched on. It sleeps in `WaitForMultipleObjects`
//! (0 CPU) until a sound is waiting, then opens the stream, fills it every period while sounds play, and closes it again
//! [`IDLE_STOP`] after the last one ended. Nothing about a key is kept anywhere in here.

use crate::front::Front;
use crate::import::Clip;
use crate::kind::{kind_of, Kind};
use crate::layers::{speed, Layer, Layers, Made, RELEASE_UP};
use crate::mixer::Mixer;
use crate::rules::{choose, choose_mouse, choose_pad, gain, Pack, PlayOn, Settings};
use crate::stream::Stream;
use crate::synth::{render, render_clicks, ClickSet, ClickStyle, MouseButtonClass, SoundSet};
use bu_rawin::{MouseSink, MouseSoundEvent, PadSink, PadSoundClass, PadSoundEvent, SoundEvent, SoundSink};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects, INFINITE};

/// The built-in packs are made at this rate once; the mixer plays them at `48000 / output rate` speed.
pub const BUILTIN_RATE: u32 = 48_000;
/// The stream is STOPPED this long after the last sound ended (it stays initialised, parked: no CPU; a press fills and starts it).
pub const IDLE_STOP: Duration = Duration::from_millis(3000);
/// A parked stream is closed after this much silence (the next press then opens a new one, on the current default device).
pub const PARK_FOR: Duration = Duration::from_secs(60);
/// Each press plays at a random speed within ± this share (the "not robotic" pitch).
const JITTER: f32 = 0.04;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Which device a button belongs to (Order 090: each has its own layers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dev {
    Keys,
    Mouse,
    Pad,
}

/// A pack made from one sound, as the engine plays it: its five sounds (every down kind = the press file, Up = the release
/// file or the press again) and every key's pitch + loudness.
#[derive(Debug, Clone)]
pub struct MadeSet {
    pub set: SoundSet,
    /// Semitones added on a release (no release file: the press plays again a touch higher).
    pub up_pitch: f32,
    pub keys: HashMap<u16, (f32, u16)>,
}

impl MadeSet {
    /// Builds the playable pack from its description and its decoded press / release files.
    pub fn new(m: &Made, press: &Clip, release: Option<&Clip>) -> MadeSet {
        let up = match release {
            // (a release file at another rate is played from the press's rate: resampled once here)
            Some(r) if r.rate == press.rate => r.mono.clone(),
            Some(r) => {
                let ratio = r.rate as f32 / press.rate as f32;
                let n = ((r.mono.len() as f32) / ratio) as usize;
                Arc::from((0..n.max(2)).map(|i| {
                    let p = i as f32 * ratio;
                    let k = p as usize;
                    let a = r.mono.get(k).copied().unwrap_or(0.0);
                    let b = r.mono.get(k + 1).copied().unwrap_or(a);
                    a + (b - a) * (p - k as f32)
                }).collect::<Vec<f32>>().into_boxed_slice())
            }
            None => press.mono.clone(),
        };
        let p = press.mono.clone();
        MadeSet {
            set: SoundSet { rate: press.rate, sounds: [p.clone(), up, p.clone(), p.clone(), p] },
            up_pitch: if release.is_some() { 0.0 } else { RELEASE_UP },
            keys: m.keys.iter().map(|(c, st, l)| (*c, (*st, *l))).collect(),
        }
    }
}

/// One sound to hear now or a little later (the page's ▶ buttons): what, how loud, how high, when.
#[derive(Debug, Clone)]
pub struct Hear {
    pub sound: Arc<[f32]>,
    pub rate: u32,
    /// Sample multiplier (already the volume's gain × loudness).
    pub gain: f32,
    /// Semitones.
    pub pitch: f32,
    /// Milliseconds after the call.
    pub at_ms: u32,
}

/// One sound to start: (samples, gain, playback speed).
type Play = (Arc<[f32]>, f32, f32);

struct Live {
    settings: Settings,
    lib: HashMap<Pack, SoundSet>,
    imported: HashMap<String, SoundSet>,
    /// Packs made from one sound (Order 090), by name.
    made: HashMap<String, MadeSet>,
    /// The layers of every key / mouse button / controller button (Order 090).
    layers: HashMap<Dev, Layers>,
    /// "Your sound" files, decoded, by file id.
    clips: HashMap<String, Clip>,
    /// The mouse clicks of every pack character in use (Order 064); empty while "Mouse clicks too" and "Controller too" are off (Order 081: the controller's triggers play them too).
    clicks: HashMap<ClickStyle, ClickSet>,
    front: Front,
    rng: u32,
    test_mute: bool,
    /// [`KeySounds::hear`] plays nothing (tests).
    test_mute_all: bool,
}

/// The click that goes with a pack (a pack the user imported has none of its own).
fn style_of(p: &Pack) -> ClickStyle {
    match p {
        Pack::Builtin(id) => ClickStyle::of(*id),
        Pack::Imported(_) | Pack::Made(_) => ClickStyle::IMPORTED,
    }
}

impl Live {
    /// Makes the sounds of every pack the settings use (built-ins once, imported ones come from `imported`); drops the rest.
    fn ensure(&mut self) {
        let want: Vec<Pack> = self.settings.packs();
        let mut styles: Vec<ClickStyle> = if self.settings.mouse_on || self.settings.pad_on { want.iter().map(style_of).collect() } else { Vec::new() };
        if let (true, Some(c)) = (self.settings.mouse_on, self.settings.mouse_click) {
            styles.push(c);
        }
        self.clicks.retain(|k, _| styles.contains(k));
        for st in styles {
            self.clicks.entry(st).or_insert_with(|| render_clicks(st, BUILTIN_RATE));
        }
        self.lib.retain(|k, _| want.contains(k));
        for p in want {
            if self.lib.contains_key(&p) {
                continue;
            }
            match &p {
                Pack::Builtin(id) => {
                    self.lib.insert(p.clone(), render(*id, BUILTIN_RATE));
                }
                Pack::Imported(n) => {
                    if let Some(s) = self.imported.get(n) {
                        self.lib.insert(p.clone(), s.clone());
                    }
                }
                Pack::Made(n) => {
                    if let Some(m) = self.made.get(n) {
                        self.lib.insert(p.clone(), m.set.clone());
                    }
                }
            }
        }
    }

    /// The pack layer's own pitch + loudness for `slot` of a made pack (others: 0 st, 100 %), and the release's added pitch.
    fn made_mod(&self, pack: &Pack, slot: u16, kind: Kind) -> (f32, f32) {
        let Pack::Made(n) = pack else { return (0.0, 1.0) };
        let Some(m) = self.made.get(n) else { return (0.0, 1.0) };
        let (st, l) = m.keys.get(&slot).copied().unwrap_or((0.0, 100));
        (st + if kind == Kind::Up { m.up_pitch } else { 0.0 }, f32::from(l) / 100.0)
    }

    /// "Your sound" of `layer` for a press / release at `g` (the device's volume gain): (sound, rate, gain, semitones).
    fn own(&self, layer: Option<&Layer>, down: bool, g: f32) -> Option<(Arc<[f32]>, u32, f32, f32)> {
        let l = layer?;
        let c = self.clips.get(l.own_file(down)?)?;
        // "Same as press": the release plays the press file again, a touch higher (the drawing's zHit)
        let up = if !down && l.release == crate::layers::Release::SameAsPress { RELEASE_UP } else { 0.0 };
        Some((c.mono.clone(), c.rate, g * f32::from(l.loud) / 100.0, l.pitch + up))
    }

    fn jitter(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        1.0 + ((x as f32 / u32::MAX as f32) * 2.0 - 1.0) * JITTER
    }
}

impl Live {
    /// The pack layer of a mouse button: the side buttons play the pack's key sound, the others a click (Click sounds › Sound,
    /// or the one that suits the pack).
    fn mouse_pack_sound(&self, pack: &Pack, b: MouseButtonClass, down: bool) -> Option<(Arc<[f32]>, u32)> {
        if b == MouseButtonClass::Side {
            let set = self.lib.get(pack)?;
            let k = if down || (self.settings.play_on == PlayOn::Release && set.get(Kind::Up).len() <= 2) { Kind::Down } else { Kind::Up };
            return Some((set.get(k).clone(), set.rate));
        }
        let style = self.settings.mouse_click.unwrap_or_else(|| style_of(pack));
        let set = self.clicks.get(&style)?;
        Some((set.get(b, down)?.clone(), set.rate))
    }

    /// The pack layer of a controller button: a button plays the pack's key sound, a trigger the left / right click that suits it.
    fn pad_pack_sound(&self, pack: &Pack, class: PadSoundClass, down: bool) -> Option<(Arc<[f32]>, u32)> {
        let click = match class {
            PadSoundClass::Button => None,
            PadSoundClass::TriggerLeft => Some(MouseButtonClass::Left),
            PadSoundClass::TriggerRight => Some(MouseButtonClass::Right),
        };
        match click {
            None => {
                let set = self.lib.get(pack)?;
                let k = if down || (self.settings.play_on == PlayOn::Release && set.get(Kind::Up).len() <= 2) { Kind::Down } else { Kind::Up };
                Some((set.get(k).clone(), set.rate))
            }
            Some(b) => {
                let set = self.clicks.get(&style_of(pack))?;
                Some((set.get(b, down)?.clone(), set.rate))
            }
        }
    }

    /// Each press is a little louder or softer (about +-1.2 dB): no two presses are the same.
    fn amp_jitter(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        1.0 + ((x as f32 / u32::MAX as f32) * 2.0 - 1.0) * 0.14
    }
}

/// What the engine is doing (for the page and the tests).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    pub enabled: bool,
    /// The output stream is open right now (sounds are playing or just played).
    pub stream_open: bool,
    /// The mouse buttons are being listened to (Order 064: "Mouse clicks too" is on and the sounds are).
    pub mouse_listening: bool,
    /// The controllers are being listened to (Order 081: "Controller too" is on and the sounds are).
    pub pad_listening: bool,
    /// The output's sample rate and the stream's added delay (ms), from the last time it was opened.
    pub rate: u32,
    pub period_ms: f32,
    pub stream_latency_ms: f32,
    /// The stream is the low-latency one (IAudioClient3); false = the device only did the plain shared stream.
    pub low_latency: bool,
    /// Sounds started so far.
    pub plays: u64,
    /// Times the stream was opened.
    pub opens: u64,
    /// MEASURED: from the press (the sink call) to its first samples being in the output buffer, last / worst (µs). The
    /// press-to-ear time is this + `stream_latency_ms`.
    pub last_submit_us: u64,
    pub worst_submit_us: u64,
    /// Why the last open failed (Windows' own text), if it did.
    pub error: Option<String>,
}

struct Shared {
    live: Mutex<Live>,
    /// The render thread runs for the user's switch OR while something is being heard (the ▶ buttons, sounds off).
    hearing: AtomicU32,
    /// The sounds are switched on (set BEFORE the thread is started, so a preview ending at that moment never stops it).
    user_on: AtomicBool,
    mixer: Mutex<Mixer>,
    status: Mutex<Status>,
    wake: isize,
    stop_ev: isize,
    stop: AtomicBool,
    out_rate: AtomicU32,
    /// The mouse sink is registered with the raw-input owner right now.
    mouse_listening: AtomicBool,
    /// The controller sink is registered with the raw-input owner right now.
    pad_listening: AtomicBool,
    /// Nanoseconds (since `epoch`) of the oldest sound that hasn't reached the buffer yet; 0 = none.
    pending_ns: AtomicU64,
    epoch: Instant,
}

fn handle(h: isize) -> HANDLE {
    HANDLE(h as *mut c_void)
}

impl Shared {
    fn now_ns(&self) -> u64 {
        (self.epoch.elapsed().as_nanos() as u64).max(1)
    }

    /// The playback speed of a sound made at `rate` played `st` semitones up.
    fn step(&self, rate: u32, st: f32) -> f32 {
        rate as f32 / self.out_rate.load(Ordering::Relaxed).max(1) as f32 * speed(st)
    }

    /// A key went down or up (called on the raw-input thread). Decides, and starts the sound(s); returns at once. Order 090:
    /// the pack's sound (unless the key's "Pack's sound" is off; a made pack gives every key its own pitch + loudness) AND the
    /// key's own sound on top.
    fn on_event(&self, e: SoundEvent) {
        let kind = kind_of(e);
        let t0 = self.now_ns();
        let mut out: [Option<Play>; 2] = [None, None];
        {
            let mut l = lock(&self.live);
            if !l.settings.keys_on || !l.settings.play_on.plays(e.down) {
                return;
            }
            // (the pack is picked while `front` is borrowed: no copy of the app's name per press)
            let picked = {
                let Live { settings, front, .. } = &mut *l;
                let (game, exe) = front.now();
                choose(settings, exe, game).cloned()
            };
            let Some(pack) = picked else { return };
            let layer = l.layers.get(&Dev::Keys).and_then(|ls| ls.get(e.key)).cloned();
            let g = gain(l.settings.volume);
            if layer.as_ref().is_none_or(|x| x.pack_on) {
                if let Some(set) = l.lib.get(&pack) {
                    // "Release only" with a pack that has no key-up sound of its own plays its key sound on the release
                    let kind = if kind == Kind::Up && l.settings.play_on == PlayOn::Release && set.get(Kind::Up).len() <= 2 { Kind::Down } else { kind };
                    let (sound, rate) = (set.get(kind).clone(), set.rate);
                    let (st, loud) = l.made_mod(&pack, e.key, kind);
                    let jit = l.jitter();
                    let g = g * loud * l.amp_jitter();
                    out[0] = Some((sound, g, self.step(rate, st) * jit));
                }
            }
            if let Some((s, rate, g, st)) = l.own(layer.as_ref(), e.down, g) {
                out[1] = Some((s, g, self.step(rate, st)));
            }
        }
        for (sound, g, step) in out.into_iter().flatten() {
            self.play(&sound, g, step, t0);
        }
    }

    /// A mouse button went down or up (called on the raw-input thread). The side buttons play the chosen pack's key sound, the
    /// others the click that suits it; the mouse volume, the game switch and the per-app rules decide whether and how loud.
    fn on_mouse_event(&self, e: MouseSoundEvent) {
        let t0 = self.now_ns();
        let mut out: [Option<Play>; 2] = [None, None];
        {
            let mut l = lock(&self.live);
            if !l.settings.mouse_on || !l.settings.play_on.plays(e.down) {
                return;
            }
            // (the pack is picked while `front` is borrowed: no copy of the app's name per press)
            let picked = {
                let Live { settings, front, .. } = &mut *l;
                let (game, exe) = front.now();
                choose_mouse(settings, exe, game).cloned()
            };
            let Some(pack) = picked else { return };
            let layer = l.layers.get(&Dev::Mouse).and_then(|ls| ls.get(u16::from(e.index))).cloned();
            let g = gain(l.settings.mouse_volume);
            if layer.as_ref().is_none_or(|x| x.pack_on) {
                if let Some((s, rate)) = l.mouse_pack_sound(&pack, e.button, e.down) {
                    let jit = l.jitter();
                    let g = g * l.amp_jitter();
                    out[0] = Some((s, g, self.step(rate, 0.0) * jit));
                }
            }
            if let Some((s, rate, g, st)) = l.own(layer.as_ref(), e.down, g) {
                out[1] = Some((s, g, self.step(rate, st)));
            }
        }
        for (sound, g, step) in out.into_iter().flatten() {
            self.play(&sound, g, step, t0);
        }
    }

    /// A controller button or trigger went down or up (called on the raw-input thread). A button plays the chosen pack's key sound,
    /// a trigger the left / right click that suits it; the keys' volume, the game switch and the per-app rules decide whether and how loud.
    fn on_pad_event(&self, e: PadSoundEvent) {
        let t0 = self.now_ns();
        let mut out: [Option<Play>; 2] = [None, None];
        {
            let mut l = lock(&self.live);
            if !l.settings.pad_on || !l.settings.play_on.plays(e.down) {
                return;
            }
            // (the pack is picked while `front` is borrowed: no copy of the app's name per press)
            let picked = {
                let Live { settings, front, .. } = &mut *l;
                let (game, exe) = front.now();
                choose_pad(settings, exe, game).cloned()
            };
            let Some(pack) = picked else { return };
            let layer = (e.button != bu_rawin::padbtn::UNKNOWN).then(|| l.layers.get(&Dev::Pad).and_then(|ls| ls.get(u16::from(e.button))).cloned()).flatten();
            let g = gain(l.settings.pad_volume);
            if layer.as_ref().is_none_or(|x| x.pack_on) {
                if let Some((s, rate)) = l.pad_pack_sound(&pack, e.class, e.down) {
                    let jit = l.jitter();
                    let g = g * l.amp_jitter();
                    out[0] = Some((s, g, self.step(rate, 0.0) * jit));
                }
            }
            if let Some((s, rate, g, st)) = l.own(layer.as_ref(), e.down, g) {
                out[1] = Some((s, g, self.step(rate, st)));
            }
        }
        for (sound, g, step) in out.into_iter().flatten() {
            self.play(&sound, g, step, t0);
        }
    }

    fn play(&self, sound: &Arc<[f32]>, g: f32, step: f32, t0: u64) {
        let _ = self.pending_ns.compare_exchange(0, t0, Ordering::AcqRel, Ordering::Relaxed);
        lock(&self.mixer).trigger(sound, g, step);
        lock(&self.status).plays += 1;
        // SAFETY: our own event handle, alive as long as `Shared`.
        let _ = unsafe { SetEvent(handle(self.wake)) };
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        // SAFETY: the two handles were created in `KeySounds::new` and are closed once, here.
        unsafe {
            let _ = CloseHandle(handle(self.wake));
            let _ = CloseHandle(handle(self.stop_ev));
        }
    }
}

/// The Keyboard tab's key sounds. Off until [`KeySounds::enable`]; while off nothing listens, nothing runs, nothing is kept.
pub struct KeySounds {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Default for KeySounds {
    fn default() -> Self {
        Self::new()
    }
}

impl KeySounds {
    pub fn new() -> KeySounds {
        // SAFETY: plain auto-reset event objects, closed in `Shared::drop`.
        let (wake, stop_ev) = unsafe {
            (
                CreateEventW(None, false, false, None).map(|h| h.0 as isize).unwrap_or(0),
                CreateEventW(None, true, false, None).map(|h| h.0 as isize).unwrap_or(0),
            )
        };
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(1) | 1;
        KeySounds {
            shared: Arc::new(Shared {
                hearing: AtomicU32::new(0),
                user_on: AtomicBool::new(false),
                live: Mutex::new(Live {
                    settings: Settings::default(),
                    lib: HashMap::new(),
                    imported: HashMap::new(),
                    made: HashMap::new(),
                    layers: HashMap::new(),
                    clips: HashMap::new(),
                    clicks: HashMap::new(),
                    front: Front::new(),
                    rng: seed,
                    test_mute: false,
                    test_mute_all: false,
                }),
                mixer: Mutex::new(Mixer::new()),
                status: Mutex::new(Status::default()),
                wake,
                stop_ev,
                stop: AtomicBool::new(false),
                out_rate: AtomicU32::new(BUILTIN_RATE),
                mouse_listening: AtomicBool::new(false),
                pad_listening: AtomicBool::new(false),
                pending_ns: AtomicU64::new(0),
                epoch: Instant::now(),
            }),
            thread: Mutex::new(None),
        }
    }

    /// Switches the sounds on: makes the packs, registers the (listen-only) key sink, starts the render thread (which
    /// sleeps until the first sound). Err = Windows refused the registration or the thread, with its own text.
    pub fn enable(&self, settings: Settings) -> Result<(), String> {
        let (keys, mouse, pad) = (settings.keys_on, settings.mouse_on, settings.pad_on);
        let was_enabled = lock(&self.shared.status).enabled;
        self.shared.user_on.store(true, Ordering::SeqCst);
        let updated = self.apply_settings(settings);
        self.start_thread()?;
        // Order 090: the keyboard is listened to only while Keyboard sounds is on (mouse / controller sounds have their own switches)
        let sink: Option<SoundSink> = keys.then(|| {
            let sh = self.shared.clone();
            Arc::new(move |e| sh.on_event(e)) as SoundSink
        });
        if let Err(e) = bu_rawin::set_key_sound(sink) {
            if !was_enabled {
                self.shared.user_on.store(false, Ordering::SeqCst);
                self.stop_if_idle();
            }
            return Err(e);
        }
        lock(&self.shared.status).enabled = true;
        // the keys work even when Windows refuses the mouse; the page shows why the clicks are silent (one try per save)
        if was_enabled {
            return updated;
        }
        let m = self.sync_mouse(mouse).map_err(|e| format!("mouse clicks: {e}"));
        let p = self.sync_pad(pad).map_err(|e| format!("controller: {e}"));
        m.and(p)
    }

    /// Registers (true) or releases (false) the mouse-button sink, only when that differs from now. While "Mouse clicks too" is
    /// off the mouse is not registered at all: nothing is read, nothing runs.
    fn sync_mouse(&self, want: bool) -> Result<(), String> {
        if self.shared.mouse_listening.load(Ordering::SeqCst) == want {
            return Ok(());
        }
        let sink: Option<MouseSink> = want.then(|| {
            let sh = self.shared.clone();
            Arc::new(move |e| sh.on_mouse_event(e)) as MouseSink
        });
        bu_rawin::set_mouse_sound(sink)?;
        self.shared.mouse_listening.store(want, Ordering::SeqCst);
        lock(&self.shared.status).mouse_listening = want;
        Ok(())
    }

    /// Registers (true) or releases (false) the controller sink, only when that differs from now. While "Controller too" is off no
    /// controller is registered at all: nothing is read, nothing runs.
    fn sync_pad(&self, want: bool) -> Result<(), String> {
        if self.shared.pad_listening.load(Ordering::SeqCst) == want {
            return Ok(());
        }
        let sink: Option<PadSink> = want.then(|| {
            let sh = self.shared.clone();
            Arc::new(move |e| sh.on_pad_event(e)) as PadSink
        });
        bu_rawin::set_pad_sound(sink)?;
        self.shared.pad_listening.store(want, Ordering::SeqCst);
        lock(&self.shared.status).pad_listening = want;
        Ok(())
    }

    /// Starts the render thread unless it runs. Every start / stop happens under the `thread` lock (Order 090 review: a
    /// preview ending while the sounds were switched on could stop the thread or leave a dead handle behind).
    fn start_thread(&self) -> Result<(), String> {
        let mut t = lock(&self.thread);
        // a thread that ended (stopped while its handle was kept) is replaced
        if t.as_ref().is_some_and(|j| j.is_finished()) {
            if let Some(j) = t.take() {
                let _ = j.join();
            }
        }
        if t.is_none() {
            self.shared.stop.store(false, Ordering::SeqCst);
            // SAFETY: our own event.
            let _ = unsafe { windows::Win32::System::Threading::ResetEvent(handle(self.shared.stop_ev)) };
            let sh = self.shared.clone();
            let j = std::thread::Builder::new().name("bu-keysound".into()).spawn(move || run(sh)).map_err(|e| format!("key sound thread: {e}"))?;
            *t = Some(j);
        }
        Ok(())
    }

    /// Tests only: the render thread and the packs, but NO key listening at all (nothing is registered with Windows, no key is
    /// read): sounds play only when `preview` is called. The latency proof uses it, so it never even listens to the keyboard.
    pub fn enable_without_keys(&self, settings: Settings) -> Result<(), String> {
        self.shared.user_on.store(true, Ordering::SeqCst);
        self.update(settings);
        self.start_thread()?;
        lock(&self.shared.status).enabled = true;
        Ok(())
    }

    /// Switches the sounds off: nothing is registered with Windows any more, the thread ends, the packs are dropped (RAM).
    pub fn disable(&self) {
        let _ = bu_rawin::set_key_sound(None);
        let _ = self.sync_mouse(false);
        let _ = self.sync_pad(false);
        lock(&self.shared.status).enabled = false;
        self.shared.user_on.store(false, Ordering::SeqCst);
        self.stop_if_idle();
        let mut l = lock(&self.shared.live);
        l.lib.clear();
        l.clicks.clear();
        drop(l);
        lock(&self.shared.mixer).stop_all();
        let mut s = lock(&self.shared.status);
        s.enabled = false;
        s.stream_open = false;
    }

    /// Stops the render thread when nothing needs it (sounds off, nothing being heard) - decided, signalled and joined under
    /// the `thread` lock, so a start waiting for the lock always comes after a finished stop.
    fn stop_if_idle(&self) {
        let mut t = lock(&self.thread);
        if self.shared.user_on.load(Ordering::SeqCst) || self.shared.hearing.load(Ordering::SeqCst) > 0 {
            return;
        }
        if let Some(j) = t.take() {
            self.shared.stop.store(true, Ordering::SeqCst);
            // SAFETY: our own event.
            let _ = unsafe { SetEvent(handle(self.shared.stop_ev)) };
            let _ = j.join();
        }
    }

    /// New settings (pack, volume, rules, game switch). Takes effect on the next press.
    pub fn update(&self, settings: Settings) {
        if let Err(e) = self.apply_settings(settings) {
            lock(&self.shared.status).error = Some(e);
        }
    }

    /// [`KeySounds::update`], giving back why the mouse could not be (un)registered.
    fn apply_settings(&self, settings: Settings) -> Result<(), String> {
        bu_rawin::set_sound_chatter(u32::from(settings.repeat_ms));
        let (mouse, pad) = (settings.mouse_on, settings.pad_on);
        {
            let mut l = lock(&self.shared.live);
            l.settings = settings;
            l.ensure();
        }
        // The raw thread locks `live` on every click, so the registration (which waits for that thread) is done without it.
        if lock(&self.shared.status).enabled {
            let m = self.sync_mouse(mouse).map_err(|e| format!("mouse clicks: {e}"));
            let p = self.sync_pad(pad).map_err(|e| format!("controller: {e}"));
            m?;
            p?;
        }
        Ok(())
    }

    /// Order 090: the layers of one device's buttons (keys by scan code, mouse buttons, controller buttons).
    pub fn set_layers(&self, dev: Dev, layers: Layers) {
        lock(&self.shared.live).layers.insert(dev, layers);
    }

    /// Order 090: a decoded "your sound" file under its id (None removes it).
    pub fn set_clip(&self, id: &str, clip: Option<Clip>) {
        let mut l = lock(&self.shared.live);
        match clip {
            Some(c) => {
                l.clips.insert(id.to_string(), c);
            }
            None => {
                l.clips.remove(id);
            }
        }
    }

    /// Which "your sound" files the engine holds now.
    pub fn clip_ids(&self) -> Vec<String> {
        lock(&self.shared.live).clips.keys().cloned().collect()
    }

    /// Order 090: a pack made from one sound under its name (None removes it).
    pub fn set_made(&self, name: &str, set: Option<MadeSet>) {
        let mut l = lock(&self.shared.live);
        match set {
            Some(s) => {
                l.made.insert(name.to_string(), s);
            }
            None => {
                l.made.remove(name);
            }
        }
        l.lib.retain(|k, _| *k != Pack::Made(name.to_string()));
        l.ensure();
    }

    /// The pack sound a button of `dev` makes (`slot` for a made pack's per-key pitch / loudness) - for the ▶ of the button
    /// windows: (sound, rate, loudness share, semitones). A built-in pack is made on the spot when it isn't loaded.
    pub fn pack_sound(&self, dev: Dev, pack: &Pack, slot: u16, down: bool, mouse: Option<MouseButtonClass>, pad: Option<PadSoundClass>) -> Option<(Arc<[f32]>, u32, f32, f32)> {
        let mut l = lock(&self.shared.live);
        if !l.lib.contains_key(pack) {
            let set = match pack {
                Pack::Builtin(id) => render(*id, BUILTIN_RATE),
                Pack::Imported(n) => l.imported.get(n)?.clone(),
                Pack::Made(n) => l.made.get(n)?.set.clone(),
            };
            l.lib.insert(pack.clone(), set);
        }
        let kind = if down { Kind::Down } else { Kind::Up };
        match dev {
            Dev::Keys => {
                let set = l.lib.get(pack)?;
                let (s, rate) = (set.get(kind).clone(), set.rate);
                let (st, loud) = l.made_mod(pack, slot, kind);
                Some((s, rate, loud, st))
            }
            Dev::Mouse => {
                let b = mouse?;
                let style = l.settings.mouse_click.unwrap_or_else(|| style_of(pack));
                if b != MouseButtonClass::Side {
                    l.clicks.entry(style).or_insert_with(|| render_clicks(style, BUILTIN_RATE));
                }
                l.mouse_pack_sound(pack, b, down).map(|(s, r)| (s, r, 1.0, 0.0))
            }
            Dev::Pad => {
                let c = pad?;
                if c != PadSoundClass::Button {
                    let style = style_of(pack);
                    l.clicks.entry(style).or_insert_with(|| render_clicks(style, BUILTIN_RATE));
                }
                l.pad_pack_sound(pack, c, down).map(|(s, r)| (s, r, 1.0, 0.0))
            }
        }
    }

    /// A decoded "your sound" the engine holds.
    pub fn clip(&self, id: &str) -> Option<Clip> {
        lock(&self.shared.live).clips.get(id).cloned()
    }

    /// Plays `list` (each at its own time), whether the sounds are switched on or not: while they are off the render thread
    /// runs only for this and is let go after the last sound rang out. Returns at once (a short-lived thread times them).
    pub fn hear(&'static self, list: Vec<Hear>) {
        if list.is_empty() || lock(&self.shared.live).test_mute_all {
            return;
        }
        let me = self;
        self.shared.hearing.fetch_add(1, Ordering::SeqCst);
        let started = self.start_thread();
        let spawned = std::thread::Builder::new().name("bu-keysound-hear".into()).spawn(move || {
            if started.is_ok() {
                let t0 = Instant::now();
                let mut list = list;
                list.sort_by_key(|h| h.at_ms);
                for h in list {
                    let wait = Duration::from_millis(u64::from(h.at_ms)).saturating_sub(t0.elapsed());
                    std::thread::sleep(wait);
                    let step = me.shared.step(h.rate, h.pitch);
                    me.shared.play(&h.sound, h.gain, step, me.shared.now_ns());
                }
                // let the last one ring out
                std::thread::sleep(Duration::from_millis(1500));
            }
            me.shared.hearing.fetch_sub(1, Ordering::SeqCst);
            me.stop_if_idle();
        });
        if spawned.is_err() {
            self.shared.hearing.fetch_sub(1, Ordering::SeqCst);
            self.stop_if_idle();
        }
    }

    /// Makes an imported pack's sounds available under `name` (None removes it).
    pub fn set_imported(&self, name: &str, set: Option<SoundSet>) {
        let mut l = lock(&self.shared.live);
        match set {
            Some(s) => {
                l.imported.insert(name.to_string(), s);
            }
            None => {
                l.imported.remove(name);
            }
        }
        l.lib.retain(|k, _| *k != Pack::Imported(name.to_string()));
        l.ensure();
    }

    /// The page's ▶: plays `kind` of `pack` at the user's volume now (a built-in pack is made on the spot when the page
    /// asks, so a bubble that is not the selected pack can be tried). Silent while the sounds are off.
    pub fn preview(&self, pack: &Pack, kind: Kind) {
        if !lock(&self.shared.status).enabled {
            return;
        }
        let t0 = self.shared.now_ns();
        let (sound, g, step) = {
            let mut l = lock(&self.shared.live);
            let set = match l.lib.get(pack) {
                Some(s) => s.clone(),
                None => match pack {
                    Pack::Builtin(id) => render(*id, BUILTIN_RATE),
                    Pack::Imported(n) => match l.imported.get(n) {
                        Some(s) => s.clone(),
                        None => return,
                    },
                    Pack::Made(n) => match l.made.get(n) {
                        Some(m) => m.set.clone(),
                        None => return,
                    },
                },
            };
            let g = gain(l.settings.volume);
            let jit = l.jitter();
            (set.get(kind).clone(), g, set.rate as f32 / self.shared.out_rate.load(Ordering::Relaxed).max(1) as f32 * jit)
        };
        self.shared.play(&sound, g, step, t0);
    }

    /// Tests only: mute this engine's own audio session, so the REAL stream runs but nothing is heard.
    pub fn set_test_mute(&self, mute: bool) {
        lock(&self.shared.live).test_mute = mute;
    }

    /// Tests / test copies of the app: [`KeySounds::hear`] plays nothing at all (no thread, no stream).
    pub fn set_hear_off(&self, off: bool) {
        lock(&self.shared.live).test_mute_all = off;
    }

    pub fn status(&self) -> Status {
        lock(&self.shared.status).clone()
    }
}

impl Drop for KeySounds {
    fn drop(&mut self) {
        self.disable();
    }
}

/// The render thread: sleeps (0 CPU) until a sound waits, then plays it through the stream, then lets the stream go.
fn run(sh: Arc<Shared>) {
    // SAFETY: COM for this thread (Core Audio), balanced below.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    // SAFETY: an auto-reset event Windows signals once per period.
    let ev = unsafe { CreateEventW(None, false, false, None) }.map(|h| h.0 as isize).unwrap_or(0);
    let stopped = || sh.stop.load(Ordering::SeqCst);
    // a press that found the stream gone (or broken) is served at once by opening a new one, without waiting for the next wake
    let mut again = false;
    'outer: loop {
        if !again {
            // idle, no stream: nothing is polled, nothing runs
            // SAFETY: handles alive for the thread's life.
            unsafe { WaitForMultipleObjects(&[handle(sh.wake), handle(sh.stop_ev)], false, INFINITE) };
        }
        again = false;
        if stopped() {
            break;
        }
        if lock(&sh.mixer).active() == 0 {
            continue;
        }
        let mute = lock(&sh.live).test_mute;
        let stream = match Stream::open(handle(ev), mute) {
            Ok(s) => s,
            Err(e) => {
                lock(&sh.mixer).stop_all();
                sh.pending_ns.store(0, Ordering::Release);
                lock(&sh.status).error = Some(e);
                // a broken device must not become a busy loop
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
        };
        sh.out_rate.store(stream.rate, Ordering::Relaxed);
        {
            let mut s = lock(&sh.status);
            s.opens += 1;
            s.rate = stream.rate;
            s.period_ms = stream.period_frames as f32 * 1000.0 / stream.rate as f32;
            s.stream_latency_ms = stream.latency_ms;
            s.low_latency = stream.low_latency;
            s.error = None;
        }
        // The stream lives on, in two states: PLAYING (filled every period while sounds play, 3 s after the last one ends it
        // is stopped) and PARKED (initialised but stopped: no thread wakes, no CPU, a press only has to fill + start it). A
        // parked stream is closed after PARK_FOR of silence, so a changed default device is picked up then at the latest
        // (and at once if Windows says the device is gone).
        'session: loop {
            lock(&sh.status).stream_open = true;
            // the first buffer already holds the waiting sound, so it is heard the moment the stream starts
            let first = {
                let mut m = lock(&sh.mixer);
                stream.fill(&mut m)
            };
            note_submit(&sh);
            if first.is_err() || stream.start().is_err() {
                stream.stop();
                lock(&sh.status).stream_open = false;
                // the device went away while parked: open the new default at once if a sound waits
                again = lock(&sh.mixer).active() > 0;
                continue 'outer;
            }
            let mut idle_since: Option<Instant> = None;
            loop {
                // SAFETY: handles alive for the thread's life.
                unsafe { WaitForMultipleObjects(&[handle(ev), handle(sh.stop_ev)], false, 500) };
                if stopped() {
                    stream.stop();
                    break 'outer;
                }
                let (res, active) = {
                    let mut m = lock(&sh.mixer);
                    let r = stream.fill(&mut m);
                    (r, m.active())
                };
                note_submit(&sh);
                if let Err(e) = res {
                    lock(&sh.status).error = Some(e);
                    stream.stop();
                    lock(&sh.status).stream_open = false;
                    again = lock(&sh.mixer).active() > 0;
                    continue 'outer;
                }
                if active == 0 {
                    let t = *idle_since.get_or_insert_with(Instant::now);
                    if t.elapsed() >= IDLE_STOP {
                        break;
                    }
                } else {
                    idle_since = None;
                }
            }
            // PARKED
            stream.stop();
            lock(&sh.status).stream_open = false;
            let parked = Instant::now();
            loop {
                let left = PARK_FOR.saturating_sub(parked.elapsed());
                if left.is_zero() {
                    continue 'outer; // closed: the stream is dropped here; the next press opens a new one
                }
                // SAFETY: handles alive for the thread's life.
                unsafe { WaitForMultipleObjects(&[handle(sh.wake), handle(sh.stop_ev)], false, left.as_millis() as u32) };
                if stopped() {
                    break 'outer;
                }
                if lock(&sh.mixer).active() > 0 {
                    continue 'session; // a press: fill + start the parked stream
                }
            }
        }
    }
    lock(&sh.status).stream_open = false;
    // SAFETY: closing what this thread created.
    unsafe {
        if ev != 0 {
            let _ = CloseHandle(handle(ev));
        }
        if com {
            CoUninitialize();
        }
    }
}

/// A buffer was just filled: the oldest waiting press is now in it — record how long that took (measured).
fn note_submit(sh: &Shared) {
    let t0 = sh.pending_ns.swap(0, Ordering::AcqRel);
    if t0 != 0 {
        let us = sh.now_ns().saturating_sub(t0) / 1000;
        let mut s = lock(&sh.status);
        s.last_submit_us = us;
        s.worst_submit_us = s.worst_submit_us.max(us);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(class: PadSoundClass, down: bool) -> PadSoundEvent {
        PadSoundEvent { class, down, button: bu_rawin::padbtn::SOUTH }
    }

    fn key(down: bool, key: u16) -> SoundEvent {
        SoundEvent { class: bu_rawin::SoundClass::Other, down, key }
    }

    fn clip(n: usize) -> Clip {
        Clip { rate: 48_000, mono: Arc::from(vec![0.5f32; n].into_boxed_slice()) }
    }

    fn voices(k: &KeySounds) -> usize {
        lock(&k.shared.mixer).active()
    }

    /// Order 090: a key plays the pack's sound AND its own sound on top; the "Pack's sound" switch silences only the pack's;
    /// release follows the layer's choice; a key without a layer plays the pack alone. No stream is opened.
    #[test]
    fn a_key_plays_both_layers_and_the_pack_switch_silences_only_the_pack() {
        let k = KeySounds::new();
        k.update(Settings::default());
        k.set_clip("boing.wav", Some(clip(4000)));
        let mut ls = Layers::new();
        ls.set(0x1C, Layer { press: Some("boing.wav".into()), release: crate::layers::Release::None, ..Layer::default() }).unwrap();
        ls.set(0x0E, Layer { pack_on: false, press: Some("boing.wav".into()), ..Layer::default() }).unwrap();
        ls.set(0x3A, Layer { pack_on: false, ..Layer::default() }).unwrap();
        k.set_layers(Dev::Keys, ls);
        k.shared.on_event(key(true, 0x1E));
        assert_eq!(k.status().plays, 1, "a plain key: the pack only");
        k.shared.on_event(key(true, 0x1C));
        assert_eq!(k.status().plays, 3, "Enter: pack + yours");
        k.shared.on_event(key(false, 0x1C));
        assert_eq!(k.status().plays, 4, "release None: only the pack's release");
        k.shared.on_event(key(true, 0x0E));
        k.shared.on_event(key(false, 0x0E));
        assert_eq!(k.status().plays, 6, "pack off: only yours, press and release (same as press)");
        k.shared.on_event(key(true, 0x3A));
        assert_eq!(k.status().plays, 6, "pack off and nothing of yours: silent");
        // a missing file (not decoded): the pack still plays
        k.set_clip("boing.wav", None);
        k.shared.on_event(key(true, 0x1C));
        assert_eq!(k.status().plays, 7);
        // Keyboard sounds off: nothing, whatever the layers say
        let s = Settings { keys_on: false, ..Settings::default() };
        k.update(s);
        k.shared.on_event(key(true, 0x1C));
        assert_eq!(k.status().plays, 7);
        assert!(voices(&k) > 0);
    }

    /// Order 090: a pack made from one sound gives every key its own pitch and loudness; the release plays the press a touch higher.
    #[test]
    fn a_made_pack_plays_each_keys_pitch_and_loudness() {
        let k = KeySounds::new();
        let m = Made { name: "Thock".into(), press: "t.wav".into(), release: None, vary: crate::layers::Vary::for_pack(), keys: vec![(0x39, -12.0, 50), (0x02, 12.0, 200)] };
        k.set_made("Thock", Some(MadeSet::new(&m, &clip(4800), None)));
        k.update(Settings { pack: Pack::Made("Thock".into()), ..Settings::default() });
        let step_of = |k: &KeySounds| {
            let m = lock(&k.shared.mixer);
            m.last_step()
        };
        k.shared.on_event(key(true, 0x39));
        let low = step_of(&k);
        k.shared.on_event(key(true, 0x02));
        let high = step_of(&k);
        assert!((high / low - 4.0).abs() < 0.4, "24 semitones apart = 4x the speed (± the tiny per-press nudge): {low} {high}");
        let set = MadeSet::new(&m, &clip(10), None);
        assert_eq!(set.up_pitch, RELEASE_UP);
        assert_eq!(MadeSet::new(&m, &clip(10), Some(&clip(20))).up_pitch, 0.0);
        assert_eq!(set.keys[&0x02], (12.0, 200));
    }

    /// Order 090: mouse and controller have switches, volumes and sounds of their own (no keyboard switch needed), and layers too.
    #[test]
    fn mouse_and_controller_layers_and_their_own_switches() {
        let k = KeySounds::new();
        let s = Settings { keys_on: false, mouse_on: true, pad_on: true, mouse_click: Some(ClickStyle::Deep), ..Settings::default() };
        k.update(s.clone());
        assert!(lock(&k.shared.live).clicks.contains_key(&ClickStyle::Deep), "the picked click is made");
        k.set_clip("x.wav", Some(clip(100)));
        let mut ml = Layers::new();
        ml.set(u16::from(bu_rawin::MOUSE_X2), Layer { pack_on: false, press: Some("x.wav".into()), ..Layer::default() }).unwrap();
        k.set_layers(Dev::Mouse, ml);
        let me = |index, button| MouseSoundEvent { button, down: true, index };
        k.shared.on_mouse_event(me(bu_rawin::MOUSE_LEFT, MouseButtonClass::Left));
        assert_eq!(k.status().plays, 1, "the keyboard switch is off, the mouse plays");
        k.shared.on_mouse_event(me(bu_rawin::MOUSE_X2, MouseButtonClass::Side));
        assert_eq!(k.status().plays, 2, "Forward: pack off, yours only");
        let mut pl = Layers::new();
        pl.set(u16::from(bu_rawin::padbtn::SOUTH), Layer { press: Some("x.wav".into()), ..Layer::default() }).unwrap();
        k.set_layers(Dev::Pad, pl);
        k.shared.on_pad_event(ev(PadSoundClass::Button, true));
        assert_eq!(k.status().plays, 4, "Cross: the pack + yours");
        k.shared.on_pad_event(PadSoundEvent { class: PadSoundClass::Button, down: true, button: bu_rawin::padbtn::UNKNOWN });
        assert_eq!(k.status().plays, 5, "an unknown button: the pack only");
        k.update(Settings { pad_volume: 0, ..s });
        k.shared.on_pad_event(ev(PadSoundClass::Button, true));
        assert_eq!(k.status().plays, 5, "the controller's own volume 0 is silent");
    }

    #[test]
    fn the_buttons_pack_sound_is_found_for_the_play_button() {
        let k = KeySounds::new();
        let p = Pack::Builtin(crate::synth::PackId::Clicky);
        assert!(k.pack_sound(Dev::Keys, &p, 0x1E, true, None, None).is_some(), "made on the spot");
        assert!(k.pack_sound(Dev::Mouse, &p, 0, true, Some(MouseButtonClass::Left), None).is_some());
        assert!(k.pack_sound(Dev::Pad, &p, 0, false, None, Some(PadSoundClass::TriggerRight)).is_some());
        assert!(k.pack_sound(Dev::Keys, &Pack::Imported("none".into()), 0, true, None, None).is_none());
    }

    /// Order 081: the controller sounds have their own switch (off by default), play at their own volume (Order 090), follow
    /// "Play on", and a trigger needs the clicks (made only while a switch that uses them is on). No stream is opened.
    #[test]
    fn controller_sounds_follow_their_switch_play_on_and_their_volume() {
        use PadSoundClass::*;
        let k = KeySounds::new();
        let plays = || k.status().plays;
        let mut s = Settings::default();
        k.update(s.clone());
        k.shared.on_pad_event(ev(Button, true));
        assert_eq!(plays(), 0, "off by default: silent");
        s.pad_on = true;
        k.update(s.clone());
        k.shared.on_pad_event(ev(Button, true));
        k.shared.on_pad_event(ev(Button, false));
        assert_eq!(plays(), 2, "a button plays the pack's key sound, down and up");
        k.shared.on_pad_event(ev(TriggerLeft, true));
        k.shared.on_pad_event(ev(TriggerRight, false));
        assert_eq!(plays(), 4, "the triggers play the clicks");
        // "Play on": press only
        s.play_on = PlayOn::Press;
        k.update(s.clone());
        k.shared.on_pad_event(ev(Button, false));
        k.shared.on_pad_event(ev(TriggerLeft, false));
        assert_eq!(plays(), 4, "press only: a release is silent");
        k.shared.on_pad_event(ev(Button, true));
        assert_eq!(plays(), 5);
        // Order 090: their own volume - the keys' and the mouse's don't matter
        s.play_on = PlayOn::Both;
        s.mouse_volume = 0;
        s.volume = 0;
        k.update(s.clone());
        k.shared.on_pad_event(ev(Button, true));
        assert_eq!(plays(), 6);
        s.pad_volume = 0;
        k.update(s.clone());
        k.shared.on_pad_event(ev(Button, true));
        assert_eq!(plays(), 6, "volume 0 is silent");
        // the switch off again: the clicks are dropped, the sounds stay silent
        s.pad_volume = 5;
        s.pad_on = false;
        k.update(s);
        k.shared.on_pad_event(ev(TriggerLeft, true));
        assert_eq!(plays(), 6);
        assert!(lock(&k.shared.live).clicks.is_empty(), "no switch uses the clicks: none are kept");
        assert!(!k.status().pad_listening, "nothing was registered with Windows");
    }
}
