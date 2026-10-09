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
use crate::kind::{kind_of, Kind};
use crate::mixer::Mixer;
use crate::rules::{choose, gain, Pack, Settings};
use crate::stream::Stream;
use crate::synth::{render, SoundSet};
use bu_rawin::{SoundEvent, SoundSink};
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

struct Live {
    settings: Settings,
    lib: HashMap<Pack, SoundSet>,
    imported: HashMap<String, SoundSet>,
    front: Front,
    rng: u32,
    test_mute: bool,
}

impl Live {
    /// Makes the sounds of every pack the settings use (built-ins once, imported ones come from `imported`); drops the rest.
    fn ensure(&mut self) {
        let mut want: Vec<Pack> = vec![self.settings.pack.clone()];
        want.extend(self.settings.rules.iter().filter_map(|r| r.pack.clone()));
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
            }
        }
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

/// What the engine is doing (for the page and the tests).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    pub enabled: bool,
    /// The output stream is open right now (sounds are playing or just played).
    pub stream_open: bool,
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
    mixer: Mutex<Mixer>,
    status: Mutex<Status>,
    wake: isize,
    stop_ev: isize,
    stop: AtomicBool,
    out_rate: AtomicU32,
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

    /// A key went down or up (called on the raw-input thread). Decides, and starts the sound; returns at once.
    fn on_event(&self, e: SoundEvent) {
        let kind = kind_of(e);
        let t0 = self.now_ns();
        let (sound, g, step) = {
            let mut l = lock(&self.live);
            let Live { settings, lib, front, .. } = &mut *l;
            let (game, exe) = front.now();
            let Some(pack) = choose(settings, exe, game) else { return };
            let Some(set) = lib.get(pack) else { return };
            let sound = set.get(kind).clone();
            let rate = set.rate as f32;
            let g = gain(settings.volume);
            let jit = l.jitter();
            (sound, g, rate / self.out_rate.load(Ordering::Relaxed).max(1) as f32 * jit)
        };
        self.play(&sound, g, step, t0);
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
                live: Mutex::new(Live {
                    settings: Settings::default(),
                    lib: HashMap::new(),
                    imported: HashMap::new(),
                    front: Front::new(),
                    rng: seed,
                    test_mute: false,
                }),
                mixer: Mutex::new(Mixer::new()),
                status: Mutex::new(Status::default()),
                wake,
                stop_ev,
                stop: AtomicBool::new(false),
                out_rate: AtomicU32::new(BUILTIN_RATE),
                pending_ns: AtomicU64::new(0),
                epoch: Instant::now(),
            }),
            thread: Mutex::new(None),
        }
    }

    /// Switches the sounds on: makes the packs, registers the (listen-only) key sink, starts the render thread (which
    /// sleeps until the first sound). Err = Windows refused the registration or the thread, with its own text.
    pub fn enable(&self, settings: Settings) -> Result<(), String> {
        self.update(settings);
        self.start_thread()?;
        let sh = self.shared.clone();
        let sink: SoundSink = Arc::new(move |e| sh.on_event(e));
        if let Err(e) = bu_rawin::set_key_sound(Some(sink)) {
            self.stop_thread();
            return Err(e);
        }
        lock(&self.shared.status).enabled = true;
        Ok(())
    }

    fn start_thread(&self) -> Result<(), String> {
        let mut t = lock(&self.thread);
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
        self.update(settings);
        self.start_thread()?;
        lock(&self.shared.status).enabled = true;
        Ok(())
    }

    /// Switches the sounds off: nothing is registered with Windows any more, the thread ends, the packs are dropped (RAM).
    pub fn disable(&self) {
        let _ = bu_rawin::set_key_sound(None);
        self.stop_thread();
        let mut l = lock(&self.shared.live);
        l.lib.clear();
        drop(l);
        lock(&self.shared.mixer).stop_all();
        let mut s = lock(&self.shared.status);
        s.enabled = false;
        s.stream_open = false;
    }

    fn stop_thread(&self) {
        let j = lock(&self.thread).take();
        if let Some(j) = j {
            self.shared.stop.store(true, Ordering::SeqCst);
            // SAFETY: our own event.
            let _ = unsafe { SetEvent(handle(self.shared.stop_ev)) };
            let _ = j.join();
        }
    }

    /// New settings (pack, volume, rules, game switch). Takes effect on the next press.
    pub fn update(&self, settings: Settings) {
        let mut l = lock(&self.shared.live);
        l.settings = settings;
        l.ensure();
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
