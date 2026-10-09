//! The noise player (Windows): ONE worker thread that exists only while the noise plays, ONE output stream.
//!
//! `play` starts the thread; it opens the stream, makes the loop (once, on this thread), fills the stream's buffer 200 ms ahead
//! about 14 times a second and sleeps in between (`WaitForSingleObject` with a timeout: a command from the page wakes it at
//! once). `stop` / the sleep timer fade the sound out (one second); when the fade has been played the stream closes, the thread
//! ends and every buffer is freed: nothing is left running or in memory. Off until `play` is called - never on its own.
//!
//! If the output device goes away (unplugged) or the default device is changed in Windows, the stream is reopened on the new
//! default and the sound fades in again (a loop made for another sample rate is made again). While there is no output device
//! at all it tries again every 2 s and the sleep timer keeps counting.

use crate::player::Core;
use crate::sound::Sound;
use crate::stream::{self, Stream};
use crate::synth::make_loop;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::{AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW, SetEvent, WaitForSingleObject};

/// How far ahead the stream's buffer is kept full: the volume, a stop or another noise is heard at most this late.
const AHEAD_MS: u32 = 200;
/// How long the worker sleeps between top-ups.
const WAKE_MS: u32 = 70;
/// A lost / missing output is tried again this often.
const RETRY: Duration = Duration::from_secs(2);
/// "Did the default output device change?" is asked this often.
const DEVICE_CHECK: Duration = Duration::from_secs(3);
/// After the last fade the buffer's tail is played out for at most this long before the stream closes.
const DRAIN_MAX: Duration = Duration::from_millis(500);
/// A stop that is still fading after this long (the fade is 1 s) is ended by force: the output is not taking sound.
const STOP_FORCE: Duration = Duration::from_millis(2500);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// What the page shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    /// The noise is on (playing, or fading out).
    pub playing: bool,
    /// A stop is fading out.
    pub stopping: bool,
    pub sound: Option<Sound>,
    pub volume: u8,
    /// Seconds until the sleep timer ends it.
    pub sleep_left: Option<u32>,
    /// Why there is no sound (no output device ...): Windows' own text.
    pub error: Option<String>,
    /// The loop is in memory and the stream is open at this rate (0 = not yet).
    pub rate: u32,
    /// Bytes of loop held right now.
    pub bytes: usize,
    /// Times the stream was opened since this player was made.
    pub opens: u32,
    /// Frames written to the stream so far: it advances at the sample rate while the device plays them.
    pub written: u64,
}

#[derive(Default)]
struct State {
    core: Option<Core>,
    alive: bool,
    thread: Option<JoinHandle<()>>,
    error: Option<String>,
    rate: u32,
    opens: u32,
    written: u64,
    /// The number of the worker that owns `core` (a new `play` makes a new one).
    gen: u64,
}

struct Shared {
    st: Mutex<State>,
    wake: isize,
    quit: AtomicBool,
    test_mute: AtomicBool,
    on_change: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    epoch: Instant,
}

fn handle(h: isize) -> HANDLE {
    HANDLE(h as *mut c_void)
}

impl Shared {
    fn now(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }
    fn poke(&self) {
        // SAFETY: our own event handle, alive as long as `Shared`.
        let _ = unsafe { SetEvent(handle(self.wake)) };
    }
    fn changed(&self) {
        let cb = lock(&self.on_change).clone();
        if let Some(cb) = cb {
            cb();
        }
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        // SAFETY: created in `Noise::new`, closed once, here.
        unsafe {
            let _ = CloseHandle(handle(self.wake));
        }
    }
}

/// The Noise tab's player.
pub struct Noise {
    shared: Arc<Shared>,
}

impl Default for Noise {
    fn default() -> Self {
        Self::new()
    }
}

impl Noise {
    pub fn new() -> Noise {
        // SAFETY: a plain auto-reset event object, closed in `Shared::drop`.
        let wake = unsafe { CreateEventW(None, false, false, None) }.map(|h| h.0 as isize).unwrap_or(0);
        Noise {
            shared: Arc::new(Shared {
                st: Mutex::new(State::default()),
                wake,
                quit: AtomicBool::new(false),
                test_mute: AtomicBool::new(false),
                on_change: Mutex::new(None),
                epoch: Instant::now(),
            }),
        }
    }

    /// Called (from the worker thread) when something the page shows changed by itself: the sleep timer ended it, the output
    /// went away. The app wakes its menu.
    pub fn set_on_change(&self, f: Arc<dyn Fn() + Send + Sync>) {
        *lock(&self.shared.on_change) = Some(f);
    }

    /// Tests: the stream's own audio session is muted (it still goes through the real device, silently).
    pub fn set_test_mute(&self, mute: bool) {
        self.shared.test_mute.store(mute, Ordering::Release);
    }

    /// Play `sound` (a Kind or a Mix) at `volume` %, with the sleep timer (`sleep_minutes`) counting from now. Already playing (or fading out):
    /// it carries on / fades back in with these values.
    pub fn play(&self, sound: impl Into<Sound>, volume: u8, sleep_minutes: Option<u32>) {
        let sound = sound.into();
        let sh = &self.shared;
        let mut st = lock(&sh.st);
        let now = sh.now();
        if st.alive {
            if let Some(c) = st.core.as_mut() {
                if !c.is_done() {
                    c.resume();
                    c.set_sound(sound);
                    c.set_volume(volume);
                    c.set_sleep(now, sleep_minutes);
                    drop(st);
                    sh.poke();
                    return;
                }
            }
        }
        // A worker that is still playing out its last fade (it is leaving) is left to leave: it is told apart by its number and
        // never touches this new core (it also isn't waited for - the menu's thread never blocks here).
        st.thread = None;
        st.gen += 1;
        let gen = st.gen;
        let mut c = Core::new(sound, volume, 48_000);
        c.set_sleep(now, sleep_minutes);
        st.core = Some(c);
        st.alive = true;
        st.error = None;
        sh.quit.store(false, Ordering::Release);
        let shared = sh.clone();
        st.thread = std::thread::Builder::new().name("bu-noise".into()).spawn(move || run(shared, gen)).ok();
        if st.thread.is_none() {
            st.alive = false;
            st.core = None;
            st.error = Some("The noise player couldn't start".into());
        }
    }

    /// Fade out (one second), then everything closes and is freed.
    pub fn stop(&self) {
        if let Some(c) = lock(&self.shared.st).core.as_mut() {
            c.stop();
        }
        self.shared.poke();
    }

    /// Another noise (while it plays: fades to it).
    pub fn set_sound(&self, s: impl Into<Sound>) {
        let s = s.into();
        if let Some(c) = lock(&self.shared.st).core.as_mut() {
            c.set_sound(s);
        }
        self.shared.poke();
    }

    pub fn set_volume(&self, percent: u8) {
        if let Some(c) = lock(&self.shared.st).core.as_mut() {
            c.set_volume(percent);
        }
    }

    /// The sleep timer from now (None = off). No effect while nothing plays.
    pub fn set_sleep(&self, minutes: Option<u32>) {
        let now = self.shared.now();
        if let Some(c) = lock(&self.shared.st).core.as_mut() {
            c.set_sleep(now, minutes);
        }
    }

    /// Ends it at once, no fade (the app is quitting). Waits for the thread (it leaves within one wake-up).
    pub fn shutdown(&self) {
        self.shared.quit.store(true, Ordering::Release);
        self.shared.poke();
        let t = lock(&self.shared.st).thread.take();
        if let Some(t) = t {
            // never longer than half a second: a hung audio service must not hold the app's exit
            let until = Instant::now() + Duration::from_millis(500);
            while !t.is_finished() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            if t.is_finished() {
                let _ = t.join();
            }
        }
    }

    pub fn status(&self) -> Status {
        let sh = &self.shared;
        let st = lock(&sh.st);
        let now = sh.now();
        let c = st.core.as_ref();
        Status {
            playing: st.alive && !c.is_some_and(|c| c.is_done()),
            stopping: c.is_some_and(|c| c.is_stopping()),
            sound: c.map(|c| c.sound()),
            volume: c.map_or(0, |c| c.volume()),
            sleep_left: c.and_then(|c| c.sleep_left(now)).map(|s| s.ceil() as u32),
            error: st.error.clone(),
            rate: st.rate,
            bytes: c.map_or(0, |c| c.bytes()),
            opens: st.opens,
            written: st.written,
        }
    }
}


/// The worker. Every Windows audio call is made OUTSIDE the lock the page reads (a hung audio service must never freeze the
/// menu): the lock is held only to ask the core things and to render into a scratch buffer.
fn run(sh: Arc<Shared>, gen: u64) {
    // SAFETY: COM for this thread; paired with `CoUninitialize` below.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    // MMCSS "Audio": Windows keeps this thread from being starved by a game for a 200 ms buffer's sake
    let mut task = 0u32;
    // SAFETY: plain call; the handle is given back below.
    let mm = unsafe { AvSetMmThreadCharacteristicsW(windows::core::w!("Audio"), &mut task) }.ok();

    let mut stream: Option<Stream> = None;
    let mut started = false;
    let mut retry_at = Instant::now();
    let mut device_checked = Instant::now();
    let mut drain_until: Option<Instant> = None;
    let mut scratch: Vec<f32> = Vec::new();
    let mut gen_job: Option<(Sound, JoinHandle<crate::synth::Loop>)> = None;
    let mut stopping_since: Option<Instant> = None;
    let mut wrote_any = false;

    loop {
        if sh.quit.load(Ordering::Acquire) || lock(&sh.st).gen != gen {
            break;
        }
        let now = Instant::now();
        // 1. a stream, when there is none
        if stream.is_none() && now >= retry_at {
            match Stream::open(sh.test_mute.load(Ordering::Acquire)) {
                Ok(s) => {
                    {
                        let mut st = lock(&sh.st);
                        st.error = None;
                        st.rate = s.rate;
                        st.opens += 1;
                        if let Some(c) = st.core.as_mut() {
                            c.set_rate(s.rate);
                            c.restart_fade();
                        }
                    }
                    stream = Some(s);
                    started = false;
                    wrote_any = false;
                    device_checked = now;
                    sh.changed();
                }
                Err(e) => {
                    let new = {
                        let mut st = lock(&sh.st);
                        let new = st.error.as_deref() != Some(e.as_str());
                        st.error = Some(e);
                        new
                    };
                    retry_at = now + RETRY;
                    if new {
                        sh.changed();
                    }
                }
            }
        }
        // 2. the loop, made OUTSIDE the lock (a stop or a new pick isn't held up)
        // (on a short-lived plain-priority thread: ~0.1 s of work that must neither starve the top-ups nor run at the audio
        // thread's raised priority)
        if gen_job.as_ref().is_some_and(|(_, j)| j.is_finished()) {
            if let Some((sound, j)) = gen_job.take() {
                if let Ok(l) = j.join() {
                    if let Some(c) = lock(&sh.st).core.as_mut() {
                        c.set_loop(sound, l);
                    }
                }
            }
        }
        if gen_job.is_none() && drain_until.is_none() {
            let need = lock(&sh.st).core.as_ref().and_then(|c| c.needs_loop());
            if let (Some(sound), Some(s)) = (need, stream.as_ref()) {
                let rate = s.rate;
                if let Ok(j) = std::thread::Builder::new().name("bu-noise-make".into()).spawn(move || make_loop(sound, rate)) {
                    gen_job = Some((sound, j));
                }
            }
        }
        // 3a. how much the stream wants (a Windows call, no lock)
        let mut lost = false;
        let mut want = 0u32;
        if let (Some(s), None) = (stream.as_ref(), drain_until) {
            match s.wanted(s.rate / 1000 * AHEAD_MS) {
                Ok(n) => want = n,
                Err(_) => lost = true,
            }
        }
        // 3b. under the lock: the sleep timer, and the next stretch of sound into the scratch buffer
        let done;
        {
            let mut st = lock(&sh.st);
            let t = sh.now();
            if st.gen != gen {
                break;
            }
            let Some(core) = st.core.as_mut() else { break };
            core.tick(t);
            // The fade only advances while sound is rendered. No output to fade on, or one that stopped taking sound: a stop
            // (or the sleep timer) must still end it, so it ends by force once the fade should long have been over.
            if core.is_stopping() {
                let since = *stopping_since.get_or_insert_with(Instant::now);
                if stream.is_none() || since.elapsed() > STOP_FORCE {
                    core.abort();
                }
            } else {
                stopping_since = None;
            }
            if want > 0 {
                let ch = stream.as_ref().map_or(2, |s| s.channels);
                scratch.clear();
                scratch.resize(want as usize * ch, 0.0);
                core.render(&mut scratch, ch);
            }
            done = core.is_done();
        }
        // 3c. into the stream (a Windows call, no lock)
        if want > 0 {
            if let Some(s) = stream.as_ref() {
                match s.write(&scratch) {
                    Ok(()) => {
                        lock(&sh.st).written += u64::from(want);
                        wrote_any = true;
                    }
                    Err(_) => lost = true,
                }
            }
        }
        // the stream starts after its first sound is in; a Start that fails is tried again and said
        if wrote_any && !started && !lost {
            if let Some(s) = stream.as_ref() {
                match s.start() {
                    Ok(()) => started = true,
                    Err(e) => lock(&sh.st).error = Some(e),
                }
            }
        }
        if lost {
            if let Some(s) = stream.take() {
                s.stop();
            }
            retry_at = Instant::now() + Duration::from_millis(500);
            started = false;
            wrote_any = false;
        }
        // 4. after the last fade: let the buffer's tail play, then close
        if done {
            match (stream.as_ref(), drain_until) {
                (None, _) => break,
                (Some(_), None) => drain_until = Some(Instant::now() + DRAIN_MAX),
                (Some(s), Some(until)) => {
                    if Instant::now() >= until || s.padding().map_or(true, |p| p == 0) {
                        break;
                    }
                }
            }
        }
        // 5. does the sound follow the default output device?
        if !done && device_checked.elapsed() >= DEVICE_CHECK {
            device_checked = Instant::now();
            if let (Some(s), Some(now_id)) = (stream.as_ref(), stream::default_id()) {
                if !s.device_id.is_empty() && s.device_id != now_id {
                    if let Some(s) = stream.take() {
                        s.stop();
                    }
                    started = false;
                    wrote_any = false;
                    retry_at = Instant::now();
                }
            }
        }
        // 6. sleep until the next top-up (a command from the page wakes us at once)
        // SAFETY: our own event handle.
        unsafe {
            if sh.wake != 0 {
                WaitForSingleObject(handle(sh.wake), WAKE_MS);
            } else {
                std::thread::sleep(Duration::from_millis(u64::from(WAKE_MS)));
            }
        }
    }

    if let Some(s) = stream.take() {
        s.stop();
    }
    {
        let mut st = lock(&sh.st);
        if st.gen == gen {
            st.core = None;
            st.alive = false;
            st.rate = 0;
            st.error = None;
        }
    }
    // SAFETY: the handle came from `AvSetMmThreadCharacteristicsW`; COM was initialised above.
    unsafe {
        if let Some(h) = mm {
            let _ = AvRevertMmThreadCharacteristics(h);
        }
        if com {
            CoUninitialize();
        }
    }
    sh.changed();
}
