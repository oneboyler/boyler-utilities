//! Audio for the Audio tab. `Audio` is what the UI talks to; `Real` is Windows Core Audio, `Fake` is a stand-in
//! with the drawing's data that tests use (tests never touch the real devices, volumes or default device).
//! Everything here is called only while the menu is open; nothing runs while it is closed.

use crate::gfx::Rgba;

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub glyph: &'static str,
}

/// How an app's tile looks: the drawing's coloured tile + glyph, or the app's real icon.
#[derive(Clone, Debug)]
pub enum Tile {
    Glyph { glyph: &'static str, a: Rgba, b: Rgba },
    Icon(std::sync::Arc<crate::png::Pixels>),
}

#[derive(Clone, Debug)]
pub struct Session {
    pub key: String,
    pub name: String,
    pub tile: Tile,
    pub c: Rgba,
    pub c2: Rgba,
    pub vol: f32,
    pub muted: bool,
    /// the raw level right now (0..1); the UI smooths it
    pub level: f32,
    /// the drawing marks some apps "quiet" by hand; None = decided from the live level
    pub quiet: Option<bool>,
}

pub trait Audio {
    fn devices(&mut self, capture: bool) -> Vec<Device>;
    fn default_device(&mut self, capture: bool) -> Option<String>;
    fn set_default(&mut self, capture: bool, id: &str);
    fn volume(&mut self, capture: bool) -> f32;
    fn set_volume(&mut self, capture: bool, v: f32);
    /// the device's level right now (0..1)
    fn peak(&mut self, capture: bool, now_ms: f64) -> f32;
    /// re-read which apps are playing (cheap enough to call a few times a second)
    fn refresh_sessions(&mut self);
    /// the current app list with live levels
    fn sessions(&mut self, now_ms: f64) -> Vec<Session>;
    fn set_session_volume(&mut self, key: &str, v: f32);
    fn set_session_mute(&mut self, key: &str, m: bool);
    /// what changed through this layer (tests read it)
    fn log(&self) -> Vec<String> {
        Vec::new()
    }
}

// =====================================================================================================  FAKE
/// The drawing's fake setup: its devices, its five apps and its fake sound (music beat, voice, game bursts, blips).
pub struct Fake {
    outs: Vec<Device>,
    ins: Vec<Device>,
    out: String,
    inp: String,
    out_vol: f32,
    in_vol: f32,
    apps: Vec<FakeApp>,
    seed: u64,
    beat_t: f64,
    beat: f32,
    voice: (f32, f64, i32),
    pub calls: Vec<String>,
    pub frozen: bool,
}

struct FakeApp {
    s: Session,
    mode: &'static str,
    tgt: f32,
    nt: f64,
    bt: f64,
}

fn dev(id: &str, name: &str, glyph: &'static str) -> Device {
    Device { id: id.into(), name: name.into(), glyph }
}

impl Fake {
    pub fn new() -> Fake {
        let app = |name: &str, glyph: &'static str, a: u32, b: u32, c: u32, c2: u32, vol: f32, mode: &'static str, muted: bool| FakeApp {
            s: Session {
                key: name.to_lowercase(),
                name: name.into(),
                tile: Tile::Glyph { glyph, a: Rgba::hex(a), b: Rgba::hex(b) },
                c: Rgba::hex(c),
                c2: Rgba::hex(c2),
                vol,
                muted,
                level: 0.0,
                quiet: Some(matches!(mode, "none" | "blip")),
            },
            mode,
            tgt: 0.0,
            nt: 0.0,
            bt: 0.0,
        };
        Fake {
            outs: vec![
                dev("spk", "Speakers (Realtek)", "spk"),
                dev("arctis", "Headphones (Arctis Nova)", "hp"),
                dev("nv", "Monitor (NVIDIA HD Audio)", "mon"),
                dev("ds", "Wireless Controller (DualSense)", "pad"),
            ],
            ins: vec![
                dev("mv7", "Microphone (Shure MV7)", "mic"),
                dev("arctis", "Headset Microphone (Arctis Nova)", "hp"),
                dev("c920", "Webcam Microphone (C920)", "wcam"),
                dev("ds", "Wireless Controller (DualSense)", "pad"),
            ],
            out: "arctis".into(),
            inp: "mv7".into(),
            out_vol: 0.74,
            in_vol: 0.90,
            apps: vec![
                app("Spotifast", "note", 0x46d989, 0x1c9a5a, 0x2fc46f, 0x86eeb2, 0.64, "music", false),
                app("Discord", "chat", 0x8f95ff, 0x5a5fe0, 0x7277f6, 0xb4b8ff, 0.80, "voice", false),
                app("VALORANT", "pad", 0xff7a76, 0xd83f4c, 0xee4f5a, 0xffa29d, 0.72, "game", false),
                app("Chrome", "globe", 0x5ab4ff, 0x2a74e6, 0x3f95f2, 0x94d0ff, 1.00, "none", true),
                app("System sounds", "abell", 0xa2abbd, 0x6c7487, 0x8d96a8, 0xc6ccd8, 0.50, "blip", false),
            ],
            seed: 0x2545_f491_4f6c_dd1d,
            beat_t: 0.0,
            beat: 0.0,
            voice: (0.0, 0.0, 0),
            calls: Vec::new(),
            frozen: false,
        }
    }
    fn rnd(&mut self) -> f32 {
        // xorshift: the fake sound only needs to look alive
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }
    fn step(&mut self, now: f64) {
        for i in 0..self.apps.len() {
            let mode = self.apps[i].mode;
            let mut t = 0.0;
            if mode == "blip" {
                if now > self.apps[i].nt {
                    let r = self.rnd() as f64;
                    self.apps[i].nt = now + 3200.0 + r * 3000.0;
                    self.apps[i].bt = now + 170.0;
                }
                t = if now < self.apps[i].bt { 0.5 } else { 0.0 };
            } else if mode != "none" {
                if now > self.apps[i].nt {
                    let (r1, r2) = (self.rnd(), self.rnd() as f64);
                    let a = &mut self.apps[i];
                    match mode {
                        "music" => {
                            a.tgt = 0.52 + r1 * 0.34;
                            a.nt = now + 110.0 + r2 * 90.0;
                        }
                        "voice" => {
                            let talk = r1 < 0.62;
                            a.tgt = if talk { 0.3 + (r2 as f32) * 0.5 } else { 0.0 };
                            a.nt = now + if talk { 80.0 + r2 * 170.0 } else { 260.0 + r2 * 700.0 };
                        }
                        _ => {
                            let hit = r1 < 0.3;
                            a.tgt = if hit { 0.6 + (r2 as f32) * 0.35 } else { 0.16 + (r2 as f32) * 0.14 };
                            a.nt = now + if hit { 60.0 + r2 * 90.0 } else { 120.0 + r2 * 240.0 };
                        }
                    }
                }
                t = self.apps[i].tgt;
            }
            let a = &mut self.apps[i];
            // the drawing: level = sound x vol^0.7 (0 when muted)
            a.s.level = if a.s.muted { 0.0 } else { t * a.s.vol.powf(0.7) };
        }
        // the fake voice for the input
        let (env, until, left) = self.voice;
        if now >= until {
            if left > 0 && env > 0.1 {
                let r = self.rnd() as f64;
                self.voice = (0.06, now + 30.0 + r * 40.0, left);
            } else if left > 0 {
                let (r1, r2) = (self.rnd(), self.rnd() as f64);
                self.voice = (0.38 + r1 * 0.55, now + 90.0 + r2 * 130.0, left - 1);
            } else {
                let (r1, r2, r3) = (self.rnd(), self.rnd(), self.rnd() as f64);
                let n = 1 + (r1 * 4.0) as i32;
                let pause = if r2 < 0.18 { 900.0 + r3 * 900.0 } else { 220.0 + r3 * 380.0 };
                self.voice = (0.0, now + pause, n);
            }
        }
        if now > self.beat_t {
            self.beat_t = now + 469.0;
            self.beat = 1.0;
        }
        self.beat *= 0.9;
    }
}

impl Audio for Fake {
    fn devices(&mut self, capture: bool) -> Vec<Device> {
        if capture { self.ins.clone() } else { self.outs.clone() }
    }
    fn default_device(&mut self, capture: bool) -> Option<String> {
        Some(if capture { self.inp.clone() } else { self.out.clone() })
    }
    fn set_default(&mut self, capture: bool, id: &str) {
        self.calls.push(format!("set_default {} {}", if capture { "in" } else { "out" }, id));
        if capture {
            self.inp = id.into();
        } else {
            self.out = id.into();
        }
    }
    fn volume(&mut self, capture: bool) -> f32 {
        if capture { self.in_vol } else { self.out_vol }
    }
    fn set_volume(&mut self, capture: bool, v: f32) {
        self.calls.push(format!("set_volume {} {}", if capture { "in" } else { "out" }, (v * 100.0).round()));
        if capture {
            self.in_vol = v;
        } else {
            self.out_vol = v;
        }
    }
    fn peak(&mut self, capture: bool, now: f64) -> f32 {
        if self.frozen {
            return if capture { 0.62 } else { 0.70 };
        }
        self.step(now);
        if capture {
            self.voice.0 * (0.9 + 0.1 * ((now / 41.0).sin() as f32))
        } else {
            let e: f32 = self.apps.iter().map(|a| a.s.level).sum();
            (e * 0.6).min(1.0) * (0.84 + 0.24 * self.beat)
        }
    }
    fn refresh_sessions(&mut self) {}
    fn sessions(&mut self, now: f64) -> Vec<Session> {
        if !self.frozen {
            self.step(now);
        } else {
            let lv = [0.55, 0.12, 0.30, 0.0, 0.0];
            for (a, l) in self.apps.iter_mut().zip(lv) {
                a.s.level = if a.s.muted { 0.0 } else { l };
            }
        }
        self.apps.iter().map(|a| a.s.clone()).collect()
    }
    fn set_session_volume(&mut self, key: &str, v: f32) {
        self.calls.push(format!("set_session_volume {} {}", key, (v * 100.0).round()));
        if let Some(a) = self.apps.iter_mut().find(|a| a.s.key == key) {
            a.s.vol = v;
        }
    }
    fn set_session_mute(&mut self, key: &str, m: bool) {
        self.calls.push(format!("set_session_mute {} {}", key, m));
        if let Some(a) = self.apps.iter_mut().find(|a| a.s.key == key) {
            a.s.muted = m;
        }
    }
    fn log(&self) -> Vec<String> {
        self.calls.clone()
    }
}

// =====================================================================================================  THREADED
/// The real audio layer on its own thread. Windows' audio service sometimes takes 100-220 ms to answer even a
/// volume / mute / default-device question (measured, Oct 7); on the drawing thread that froze page switches. The
/// worker asks Windows (levels every 16 ms, volumes every 250 ms, devices + apps every second) and keeps the latest
/// answers here; the UI reads them without waiting. Changes go to the worker as commands. The worker lives only
/// while the menu is open (dropped with the menu).
pub struct Threaded {
    snap: std::sync::Arc<std::sync::Mutex<Snap>>,
    tx: std::sync::mpsc::Sender<Cmd>,
    join: Option<std::thread::JoinHandle<()>>,
    /// a value the UI just set wins over the worker's answers for a moment (the worker may still be reading the old one)
    hold: std::collections::HashMap<String, (f32, std::time::Instant)>,
    hold_def: [Option<(String, std::time::Instant)>; 2],
}

#[derive(Default, Clone)]
struct Snap {
    devs: [Vec<Device>; 2],
    def: [Option<String>; 2],
    vol: [f32; 2],
    peak: [f32; 2],
    sessions: Vec<Session>,
    ready: bool,
    /// time spent (ms): slow, fast, levels, peaks, publish; and the number of passes
    prof: [f64; 6],
}

enum Cmd {
    SetDefault(bool, String),
    SetVolume(bool, f32),
    SessionVolume(String, f32),
    SessionMute(String, bool),
    Quit,
}

const HOLD_MS: u128 = 600;

/// The worker's last full answer, kept between opens.
static LAST: std::sync::Mutex<Option<Snap>> = std::sync::Mutex::new(None);

impl Threaded {
    /// Starts the worker and returns at once: the menu starts from the answers of the last open (kept while closed, a few
    /// KB) and the worker refreshes them within milliseconds â€” the page content only fades in ~320 ms after the open
    /// anyway. Opening never waits for Windows' audio service (that wait was 126-1076 ms per open, measured Oct 8).
    pub fn start() -> Option<Threaded> {
        let first = LAST.lock().unwrap().clone().unwrap_or_default();
        let snap = std::sync::Arc::new(std::sync::Mutex::new(first));
        let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
        let s2 = snap.clone();
        let join = std::thread::spawn(move || worker(s2, rx));
        Some(Threaded { snap, tx, join: Some(join), hold: Default::default(), hold_def: [None, None] })
    }    fn held(&mut self, k: &str) -> Option<f32> {
        match self.hold.get(k) {
            Some((v, t)) if t.elapsed().as_millis() < HOLD_MS => Some(*v),
            Some(_) => {
                self.hold.remove(k);
                None
            }
            None => None,
        }
    }
}

impl Drop for Threaded {
    fn drop(&mut self) {
        let p = self.snap.lock().unwrap().prof;
        crate::timing::note(&format!("audio_worker ms: slow {:.0} fast {:.0} levels {:.0} peaks {:.0} publish {:.0} passes {}", p[0], p[1], p[2], p[3], p[4], p[5]));
        let _ = self.tx.send(Cmd::Quit);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn worker(snap: std::sync::Arc<std::sync::Mutex<Snap>>, rx: std::sync::mpsc::Receiver<Cmd>) {
    use windows::Win32::System::Com::*;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    'run: {
        // no Core Audio (very rare): the lists stay empty
        let Ok(mut r) = Real::new() else { break 'run };
        let t0 = std::time::Instant::now();
        let (mut last_slow, mut last_fast) = (-1e9f64, -1e9f64);
        let mut s = Snap::default();
        loop {
            let now = t0.elapsed().as_secs_f64() * 1000.0;
            let q0 = std::time::Instant::now();
            let mut fast = now - last_fast >= 250.0;
            if now - last_slow >= 1000.0 {
                last_slow = now;
                s.devs = [r.devices(false), r.devices(true)];
                r.refresh_sessions();
                fast = true;
            }
            let q1 = std::time::Instant::now();
            if fast {
                last_fast = now;
                s.def = [r.default_device(false), r.default_device(true)];
                s.vol = [r.volume(false), r.volume(true)];
                s.sessions = r.sessions(now);
            } else {
                let lv = r.session_levels();
                for x in s.sessions.iter_mut() {
                    if let Some((_, l)) = lv.iter().find(|(k, _)| *k == x.key) {
                        x.level = *l;
                    }
                }
            }
            let q2 = std::time::Instant::now();
            s.peak = [r.peak(false, now), r.peak(true, now)];
            s.ready = true;
            let q3 = std::time::Instant::now();
            *snap.lock().unwrap() = s.clone();
            if fast {
                *LAST.lock().unwrap() = Some(Snap { prof: [0.0; 6], ..s.clone() });
            }
            let q4 = std::time::Instant::now();
            let ms = |a: std::time::Instant, b: std::time::Instant| (b - a).as_secs_f64() * 1000.0;
            s.prof[0] += ms(q0, q1);
            if fast { s.prof[1] += ms(q1, q2) } else { s.prof[2] += ms(q1, q2) }
            s.prof[3] += ms(q2, q3);
            s.prof[4] += ms(q3, q4);
            s.prof[5] += 1.0;
            // commands wake the worker at once; otherwise the next levels in 16 ms (one 60 Hz frame; the meters are smoothed)
            match rx.recv_timeout(std::time::Duration::from_millis(16)) {
                Ok(c) => {
                    let mut c = Some(c);
                    while let Some(cmd) = c.take().or_else(|| rx.try_recv().ok()) {
                        match cmd {
                            Cmd::Quit => break 'run,
                            Cmd::SetDefault(cap, id) => r.set_default(cap, &id),
                            Cmd::SetVolume(cap, v) => r.set_volume(cap, v),
                            Cmd::SessionVolume(k, v) => r.set_session_volume(&k, v),
                            Cmd::SessionMute(k, m) => r.set_session_mute(&k, m),
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break 'run,
            }
        }
    }
    unsafe {
        CoUninitialize();
    }
}

impl Audio for Threaded {
    fn devices(&mut self, capture: bool) -> Vec<Device> {
        self.snap.lock().unwrap().devs[capture as usize].clone()
    }
    fn default_device(&mut self, capture: bool) -> Option<String> {
        let i = capture as usize;
        let now = self.snap.lock().unwrap().def[i].clone();
        match &self.hold_def[i] {
            // Windows switches a moment later; until it does (at most 1.5 s) the picker shows the new pick
            Some((id, t)) if Some(id) != now.as_ref() && t.elapsed().as_millis() < 1500 => Some(id.clone()),
            _ => {
                self.hold_def[i] = None;
                now
            }
        }
    }
    fn set_default(&mut self, capture: bool, id: &str) {
        self.hold_def[capture as usize] = Some((id.to_string(), std::time::Instant::now()));
        let _ = self.tx.send(Cmd::SetDefault(capture, id.to_string()));
    }
    fn volume(&mut self, capture: bool) -> f32 {
        let k = if capture { "\u{1}in" } else { "\u{1}out" };
        let v = self.snap.lock().unwrap().vol[capture as usize];
        self.held(k).unwrap_or(v)
    }
    fn set_volume(&mut self, capture: bool, v: f32) {
        let k = if capture { "\u{1}in" } else { "\u{1}out" };
        self.hold.insert(k.to_string(), (v, std::time::Instant::now()));
        let _ = self.tx.send(Cmd::SetVolume(capture, v));
    }
    fn peak(&mut self, capture: bool, _now_ms: f64) -> f32 {
        self.snap.lock().unwrap().peak[capture as usize]
    }
    fn refresh_sessions(&mut self) {}
    fn sessions(&mut self, _now_ms: f64) -> Vec<Session> {
        let mut l = self.snap.lock().unwrap().sessions.clone();
        for x in l.iter_mut() {
            if let Some(v) = self.held(&format!("v{}", x.key)) {
                x.vol = v;
            }
            if let Some(m) = self.held(&format!("m{}", x.key)) {
                x.muted = m > 0.5;
            }
        }
        l
    }
    fn set_session_volume(&mut self, key: &str, v: f32) {
        self.hold.insert(format!("v{}", key), (v, std::time::Instant::now()));
        let _ = self.tx.send(Cmd::SessionVolume(key.to_string(), v));
    }
    fn set_session_mute(&mut self, key: &str, m: bool) {
        self.hold.insert(format!("m{}", key), (if m { 1.0 } else { 0.0 }, std::time::Instant::now()));
        let _ = self.tx.send(Cmd::SessionMute(key.to_string(), m));
    }
}

// =====================================================================================================  REAL
pub use real::Real;

mod real {
    use super::*;
    use std::collections::HashMap;
    use windows::core::*;
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Foundation::*;
    use windows::Win32::Media::Audio::Endpoints::*;
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::StructuredStorage::*;
    use windows::Win32::System::Com::*;

    #[windows::core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    unsafe trait IPolicyConfig: IUnknown {
        fn GetMixFormat(&self, id: PCWSTR, fmt: *mut *mut core::ffi::c_void) -> HRESULT;
        fn GetDeviceFormat(&self, id: PCWSTR, default: i32, fmt: *mut *mut core::ffi::c_void) -> HRESULT;
        fn ResetDeviceFormat(&self, id: PCWSTR) -> HRESULT;
        fn SetDeviceFormat(&self, id: PCWSTR, endpoint: *mut core::ffi::c_void, mix: *mut core::ffi::c_void) -> HRESULT;
        fn GetProcessingPeriod(&self, id: PCWSTR, default: i32, def: *mut i64, min: *mut i64) -> HRESULT;
        fn SetProcessingPeriod(&self, id: PCWSTR, period: *mut i64) -> HRESULT;
        fn GetShareMode(&self, id: PCWSTR, mode: *mut core::ffi::c_void) -> HRESULT;
        fn SetShareMode(&self, id: PCWSTR, mode: *mut core::ffi::c_void) -> HRESULT;
        fn GetPropertyValue(&self, id: PCWSTR, fx: i32, key: *const core::ffi::c_void, v: *mut core::ffi::c_void) -> HRESULT;
        fn SetPropertyValue(&self, id: PCWSTR, fx: i32, key: *const core::ffi::c_void, v: *mut core::ffi::c_void) -> HRESULT;
        fn SetDefaultEndpoint(&self, id: PCWSTR, role: i32) -> HRESULT;
        fn SetEndpointVisibility(&self, id: PCWSTR, visible: i32) -> HRESULT;
    }
    const CLSID_POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

    struct Endpoint {
        id: String,
        vol: Option<IAudioEndpointVolume>,
        meter: Option<IAudioMeterInformation>,
    }

    struct RealSession {
        s: Session,
        vol: ISimpleAudioVolume,
        meter: Option<IAudioMeterInformation>,
    }

    pub struct Real {
        en: IMMDeviceEnumerator,
        eps: [Option<Endpoint>; 2],
        mgr: Option<(String, IAudioSessionManager2)>,
        list: Vec<RealSession>,
        known: HashMap<String, (String, Tile, Rgba, Rgba)>,
    }

    fn pwstr_take(p: PWSTR) -> String {
        if p.is_null() {
            return String::new();
        }
        let s = unsafe { p.to_string().unwrap_or_default() };
        unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
        s
    }

    fn flow(capture: bool) -> EDataFlow {
        if capture { eCapture } else { eRender }
    }

    impl Real {
        pub fn new() -> Result<Real> {
            let en: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
            Ok(Real { en, eps: [None, None], mgr: None, list: Vec::new(), known: HashMap::new() })
        }

        /// Only the apps' live levels (the meters), not their volume / mute.
        pub fn session_levels(&mut self) -> Vec<(String, f32)> {
            self.list
                .iter()
                .map(|r| (r.s.key.clone(), r.meter.as_ref().and_then(|m| unsafe { m.GetPeakValue().ok() }).unwrap_or(0.0)))
                .collect()
        }

        /// The default device's volume + meter objects (re-made when `default_device` sees the default change).
        fn endpoint(&mut self, capture: bool) -> Option<&Endpoint> {
            let i = capture as usize;
            if self.eps[i].is_none() {
                self.renew(capture);
            }
            self.eps[i].as_ref()
        }
        fn renew(&mut self, capture: bool) {
            self.eps[capture as usize] = unsafe {
                self.en.GetDefaultAudioEndpoint(flow(capture), eConsole).ok().map(|d| Endpoint {
                    id: pwstr_take(d.GetId().unwrap_or(PWSTR::null())),
                    vol: d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok(),
                    meter: d.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None).ok(),
                })
            };
        }
    }

    fn glyph_for(name: &str, form: u32, capture: bool) -> &'static str {
        let n = name.to_lowercase();
        if n.contains("controller") || n.contains("dualsense") || n.contains("xbox") {
            return "pad";
        }
        if n.contains("webcam") || n.contains("camera") || n.contains("c920") {
            return "wcam";
        }
        // EndpointFormFactor: 1 Speakers, 2 LineLevel, 3 Headphones, 4 Microphone, 5 Headset, 6 Handset, 8 SPDIF, 9 DigitalAudioDisplayDevice
        match form {
            3 | 5 => "hp",
            9 => "mon",
            4 => "mic",
            _ => {
                if capture {
                    "mic"
                } else if n.contains("headphone") || n.contains("headset") {
                    "hp"
                } else if n.contains("nvidia") || n.contains("monitor") || n.contains("display") || n.contains("hdmi") {
                    "mon"
                } else {
                    "spk"
                }
            }
        }
    }

    impl Audio for Real {
        fn devices(&mut self, capture: bool) -> Vec<Device> {
            let mut out = Vec::new();
            unsafe {
                let Ok(col) = self.en.EnumAudioEndpoints(flow(capture), DEVICE_STATE_ACTIVE) else { return out };
                let n = col.GetCount().unwrap_or(0);
                for i in 0..n {
                    let Ok(d) = col.Item(i) else { continue };
                    let id = pwstr_take(d.GetId().unwrap_or(PWSTR::null()));
                    let mut name = String::new();
                    let mut form = 0u32;
                    if let Ok(ps) = d.OpenPropertyStore(STGM_READ) {
                        if let Ok(v) = ps.GetValue(&PKEY_Device_FriendlyName) {
                            name = PropVariantToStringAlloc(&v).map(pwstr_take).unwrap_or_default();
                        }
                        if let Ok(v) = ps.GetValue(&PKEY_AudioEndpoint_FormFactor) {
                            form = PropVariantToUInt32(&v).unwrap_or(0);
                        }
                    }
                    let glyph = glyph_for(&name, form, capture);
                    out.push(Device { id, name, glyph });
                }
            }
            out
        }
        fn default_device(&mut self, capture: bool) -> Option<String> {
            let id = unsafe { self.en.GetDefaultAudioEndpoint(flow(capture), eConsole).ok().map(|d| pwstr_take(d.GetId().unwrap_or(PWSTR::null()))) };
            let i = capture as usize;
            if self.eps[i].as_ref().map(|e| Some(&e.id) != id.as_ref()).unwrap_or(true) {
                self.renew(capture);
            }
            id
        }
        fn set_default(&mut self, _capture: bool, id: &str) {
            // all three roles, like the Sound panel; on a worker thread so the menu never stutters
            let id = id.to_string();
            std::thread::spawn(move || unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                if let Ok(pc) = CoCreateInstance::<_, IPolicyConfig>(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) {
                    let w: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
                    for role in [eConsole, eMultimedia, eCommunications] {
                        let _ = pc.SetDefaultEndpoint(PCWSTR(w.as_ptr()), role.0);
                    }
                }
                CoUninitialize();
            });
        }
        fn volume(&mut self, capture: bool) -> f32 {
            self.endpoint(capture).and_then(|e| e.vol.as_ref()).and_then(|v| unsafe { v.GetMasterVolumeLevelScalar().ok() }).unwrap_or(0.0)
        }
        fn set_volume(&mut self, capture: bool, v: f32) {
            if let Some(ev) = self.endpoint(capture).and_then(|e| e.vol.clone()) {
                unsafe {
                    let _ = ev.SetMasterVolumeLevelScalar(v.clamp(0.0, 1.0), std::ptr::null());
                }
            }
        }
        fn peak(&mut self, capture: bool, _now: f64) -> f32 {
            self.endpoint(capture).and_then(|e| e.meter.as_ref()).and_then(|m| unsafe { m.GetPeakValue().ok() }).unwrap_or(0.0)
        }
        fn refresh_sessions(&mut self) {
            unsafe {
                // the session manager of the current default output device
                let Some(cur) = self.default_device(false) else {
                    self.list.clear();
                    return;
                };
                if self.mgr.as_ref().map(|m| m.0 != cur).unwrap_or(true) {
                    self.mgr = self
                        .en
                        .GetDefaultAudioEndpoint(eRender, eConsole)
                        .ok()
                        .and_then(|d| d.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None).ok())
                        .map(|m| (cur.clone(), m));
                    self.list.clear();
                }
                let Some((_, mgr)) = &self.mgr else { return };
                let Ok(en) = mgr.GetSessionEnumerator() else { return };
                let n = en.GetCount().unwrap_or(0);
                let mut next: Vec<RealSession> = Vec::new();
                for i in 0..n {
                    let Ok(c) = en.GetSession(i) else { continue };
                    let Ok(c2) = c.cast::<IAudioSessionControl2>() else { continue };
                    let system = c2.IsSystemSoundsSession() == S_OK;
                    let state = c2.GetState().unwrap_or(AudioSessionStateInactive);
                    // "Only apps making sound show up": active sessions, plus System sounds (as the drawing lists it)
                    if state != AudioSessionStateActive && !system {
                        continue;
                    }
                    let key = pwstr_take(c2.GetSessionInstanceIdentifier().unwrap_or(PWSTR::null()));
                    if next.iter().any(|s| s.s.key == key) {
                        continue;
                    }
                    let pid = c2.GetProcessId().unwrap_or(0);
                    let Ok(vol) = c.cast::<ISimpleAudioVolume>() else { continue };
                    let meter = c.cast::<IAudioMeterInformation>().ok();
                    let (name, tile, ca, cb) = match self.known.get(&key) {
                        Some(k) => k.clone(),
                        None => {
                            let (info, ready) = crate::appinfo::describe_cached(pid, system, &pwstr_take(c2.GetDisplayName().unwrap_or(PWSTR::null())));
                            // the icon is still being read: ask again on the next refresh
                            if ready {
                                self.known.insert(key.clone(), info.clone());
                            }
                            info
                        }
                    };
                    let v = vol.GetMasterVolume().unwrap_or(1.0);
                    let m = vol.GetMute().map(|b| b.as_bool()).unwrap_or(false);
                    next.push(RealSession { s: Session { key, name, tile, c: ca, c2: cb, vol: v, muted: m, level: 0.0, quiet: None }, vol, meter });
                }
                // System sounds goes last, like the drawing
                next.sort_by_key(|s| matches!(&s.s.tile, Tile::Glyph { glyph: "abell", .. }));
                self.list = next;
            }
        }
        fn sessions(&mut self, _now: f64) -> Vec<Session> {
            self.list
                .iter_mut()
                .map(|r| unsafe {
                    r.s.vol = r.vol.GetMasterVolume().unwrap_or(r.s.vol);
                    r.s.muted = r.vol.GetMute().map(|b| b.as_bool()).unwrap_or(r.s.muted);
                    r.s.level = r.meter.as_ref().and_then(|m| m.GetPeakValue().ok()).unwrap_or(0.0);
                    r.s.clone()
                })
                .collect()
        }
        fn set_session_volume(&mut self, key: &str, v: f32) {
            if let Some(r) = self.list.iter().find(|r| r.s.key == key) {
                unsafe {
                    let _ = r.vol.SetMasterVolume(v.clamp(0.0, 1.0), std::ptr::null());
                }
            }
        }
        fn set_session_mute(&mut self, key: &str, m: bool) {
            if let Some(r) = self.list.iter().find(|r| r.s.key == key) {
                unsafe {
                    let _ = r.vol.SetMute(m, std::ptr::null());
                }
            }
        }
    }

    #[allow(dead_code)]
    fn _unused(_: PROPVARIANT) {}
}
