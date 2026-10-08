//! Speed test: download -> upload -> ping + jitter (DESIGN §3.11 order), with live progress events for the gauge.
//! The network side is the [`SpeedTransport`] trait; the real one talks to Cloudflare's speed test servers
//! (`crate::real::CloudflareSpeed`), the tests use a fake.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use crate::error::{NetError, Result};
use crate::os::NetworkOs;
use crate::service::NetworkService;

/// The test server's side of a speed test.
pub trait SpeedTransport: Send + Sync {
    /// The server that answers (shown as "Testing · nearest server: <city>").
    fn server(&self) -> Result<ServerInfo>;
    /// Downloads up to `bytes` in one request. `on_bytes(n)` is called for every chunk received; returning false
    /// ends the request early (the phase is over).
    fn download(&self, bytes: u64, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<()>;
    /// Uploads up to `bytes` in one request; `on_bytes(n)` after every chunk sent; false ends it early.
    fn upload(&self, bytes: u64, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<()>;
    /// One tiny request; returns its round trip with the server's own processing time taken out.
    fn latency(&self) -> Result<Duration>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    /// "Vienna" (empty when the service does not say).
    pub city: String,
    /// The service's location code, e.g. Cloudflare's "VIE".
    pub code: String,
    pub provider: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedPhase {
    Download,
    Upload,
    Latency,
}

/// Numbers that shape a run. `Default`: 15 s down, 15 s up (speedtest.net's length - the owner Oct 8: the old 7 + 6 s run was
/// "very short"), ~1.6 s ping.
#[derive(Debug, Clone)]
pub struct SpeedConfig {
    pub download_time: Duration,
    pub upload_time: Duration,
    /// The first part of each transfer phase is not counted (TCP is still speeding up); the gauge still moves.
    pub warmup: Duration,
    pub download_streams: usize,
    pub upload_streams: usize,
    /// Size of one download request; a stream asks again until the phase ends.
    pub download_request_bytes: u64,
    pub upload_request_bytes: u64,
    pub latency_samples: usize,
    /// Pause between latency samples.
    pub latency_spacing: Duration,
    /// How often a Progress event is sent.
    pub tick: Duration,
    /// The live number is the speed over this last stretch.
    pub live_window: Duration,
}

impl Default for SpeedConfig {
    fn default() -> Self {
        SpeedConfig {
            download_time: Duration::from_secs(15),
            upload_time: Duration::from_secs(15),
            warmup: Duration::from_secs(1),
            download_streams: 6,
            upload_streams: 4,
            download_request_bytes: 25_000_000,
            upload_request_bytes: 10_000_000,
            latency_samples: 20,
            latency_spacing: Duration::from_millis(60),
            tick: Duration::from_millis(100),
            live_window: Duration::from_millis(1000),
        }
    }
}

/// Events for the animated gauge and the results column.
#[derive(Debug, Clone, PartialEq)]
pub enum SpeedEvent {
    /// The server is known (footer "Testing · nearest server: <city>").
    Server(ServerInfo),
    PhaseStarted(SpeedPhase),
    /// Live value while download / upload runs: Mb/s over the last `live_window`.
    Progress { phase: SpeedPhase, mbps: f64, elapsed: Duration },
    /// One latency round trip (ms) while the ping phase runs.
    LatencySample { ms: f64 },
    /// A phase's final number: Mb/s for download / upload, ping ms for latency.
    PhaseDone { phase: SpeedPhase, value: f64 },
    Done(SpeedResult),
}

/// The result row ("Only the last result is kept").
#[derive(Debug, Clone, PartialEq)]
pub struct SpeedResult {
    pub download_mbps: f64,
    pub upload_mbps: f64,
    /// Lowest round trip of the latency phase.
    pub ping_ms: f64,
    /// Average change between consecutive round trips.
    pub jitter_ms: f64,
    /// Data used, both directions (what the test cost on the user's plan).
    pub bytes_down: u64,
    pub bytes_up: u64,
    pub server: ServerInfo,
    pub finished: SystemTime,
}

impl SpeedResult {
    /// "Speed test done · ↓ 899 · ↑ 105 Mb/s"
    pub fn toast(&self) -> String {
        format!("Speed test done · ↓ {:.0} · ↑ {:.0} Mb/s", self.download_mbps, self.upload_mbps)
    }
}

/// Megabits per second from bytes over a time.
pub fn mbps(bytes: u64, over: Duration) -> f64 {
    let s = over.as_secs_f64();
    if s <= 0.0 {
        return 0.0;
    }
    bytes as f64 * 8.0 / s / 1_000_000.0
}

/// Ping = lowest sample; jitter = average absolute change between consecutive samples (DESIGN §3.11).
/// Returns None with no samples; jitter is 0 with one sample.
pub fn ping_and_jitter(samples_ms: &[f64]) -> Option<(f64, f64)> {
    let ping = samples_ms.iter().copied().fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.min(x))))?;
    let jitter = if samples_ms.len() < 2 {
        0.0
    } else {
        let sum: f64 = samples_ms.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
        sum / (samples_ms.len() - 1) as f64
    };
    Some((ping, jitter))
}

/// The gauge math of one transfer phase, fed with (time since the phase started, total bytes so far) once per tick.
/// Pure (no clock, no threads), so it is tested with exact numbers.
#[derive(Debug, Clone)]
pub struct RateMeter {
    warmup: Duration,
    live_window: Duration,
    history: VecDeque<(Duration, u64)>,
    warm: Option<(Duration, u64)>,
    /// every tick's slice: (its length, bytes moved in it) - the phase's number is worked out from these
    slices: Vec<(Duration, u64)>,
    last: (Duration, u64),
}

/// The share of the slowest / fastest slices left out of a phase's number (Ookla's Speedtest method: the slowest 30 %
/// - TCP still speeding up, a stall - and the fastest 10 % - bursts - are dropped, the rest is averaged).
pub const TRIM_SLOW: f64 = 0.30;
pub const TRIM_FAST: f64 = 0.10;
/// Fewer slices than this: the plain average from the warm-up mark (too few to trim).
pub const MIN_SLICES: usize = 10;

impl RateMeter {
    pub fn new(warmup: Duration, live_window: Duration) -> RateMeter {
        RateMeter { warmup, live_window, history: VecDeque::from([(Duration::ZERO, 0)]), warm: None, slices: Vec::new(), last: (Duration::ZERO, 0) }
    }

    /// The phase's number the way speedtest.net works it out (the owner Oct 8: "follow speedtest.net"): the tick slices sorted by
    /// speed, the slowest [`TRIM_SLOW`] and the fastest [`TRIM_FAST`] of them left out, the rest averaged (their bytes over
    /// their time). None with fewer than [`MIN_SLICES`] slices.
    pub fn trimmed(&self) -> Option<f64> {
        let n = self.slices.len();
        if n < MIN_SLICES {
            return None;
        }
        let mut s: Vec<(Duration, u64)> = self.slices.clone();
        s.sort_by(|a, b| mbps(a.1, a.0).total_cmp(&mbps(b.1, b.0)));
        let lo = (n as f64 * TRIM_SLOW).floor() as usize;
        let hi = n - (n as f64 * TRIM_FAST).floor() as usize;
        let kept = &s[lo..hi.max(lo + 1)];
        let t: Duration = kept.iter().map(|x| x.0).sum();
        let b: u64 = kept.iter().map(|x| x.1).sum();
        Some(mbps(b, t))
    }

    /// One tick: returns the live Mb/s over the last `live_window`. The first tick at or after `warmup` is the
    /// warm-up mark.
    pub fn sample(&mut self, at: Duration, bytes: u64) -> f64 {
        if self.warm.is_none() && at >= self.warmup {
            self.warm = Some((at, bytes));
        }
        if at > self.last.0 {
            self.slices.push((at - self.last.0, bytes.saturating_sub(self.last.1)));
            self.last = (at, bytes);
        }
        self.history.push_back((at, bytes));
        while self.history.len() > 2 && at.saturating_sub(self.history[1].0) >= self.live_window {
            self.history.pop_front();
        }
        let (t0, b0) = self.history[0];
        mbps(bytes.saturating_sub(b0), at.saturating_sub(t0))
    }

    /// The phase's number: [`trimmed`](Self::trimmed) (speedtest.net's way); with too few ticks for that, Mb/s from the
    /// warm-up mark to `at` - from the start when no tick reached the warm-up, or when the mark is `at` itself (a busy PC ran
    /// the ticker so late that the warm-up tick is the last one).
    pub fn result(&self, at: Duration, bytes: u64) -> f64 {
        if let Some(v) = self.trimmed() {
            return v;
        }
        let (wt, wb) = match self.warm {
            Some((wt, wb)) if wt < at => (wt, wb),
            _ => (Duration::ZERO, 0),
        };
        mbps(bytes.saturating_sub(wb), at.saturating_sub(wt))
    }
}

/// Runs a whole test on the calling thread. `cancel` stops it within one tick (Err(Cancelled)).
/// Err(Offline) before anything is sent when no adapter carries an internet route.
pub fn run(
    os: &Arc<dyn NetworkOs>,
    transport: &dyn SpeedTransport,
    cfg: &SpeedConfig,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(SpeedEvent),
) -> Result<SpeedResult> {
    if NetworkService::new(os.clone()).connection_state()?.offline() {
        return Err(NetError::Offline);
    }
    let server = transport.server()?;
    on_event(SpeedEvent::Server(server.clone()));

    let (down, bytes_down) = transfer(SpeedPhase::Download, transport, cfg, cancel, on_event)?;
    let (up, bytes_up) = transfer(SpeedPhase::Upload, transport, cfg, cancel, on_event)?;

    on_event(SpeedEvent::PhaseStarted(SpeedPhase::Latency));
    // One unrecorded round trip first: it may include opening a fresh connection (TCP + TLS handshakes).
    let _ = transport.latency();
    let mut samples = Vec::with_capacity(cfg.latency_samples);
    let mut last_err = None;
    for i in 0..cfg.latency_samples {
        if cancel.load(Ordering::Relaxed) {
            return Err(NetError::Cancelled);
        }
        if i > 0 {
            std::thread::sleep(cfg.latency_spacing);
        }
        match transport.latency() {
            Ok(d) => {
                let ms = d.as_secs_f64() * 1000.0;
                samples.push(ms);
                on_event(SpeedEvent::LatencySample { ms });
            }
            Err(e) => last_err = Some(e),
        }
    }
    let (ping, jitter) = ping_and_jitter(&samples).ok_or(last_err.unwrap_or(NetError::Timeout))?;
    on_event(SpeedEvent::PhaseDone { phase: SpeedPhase::Latency, value: ping });

    let result = SpeedResult {
        download_mbps: down,
        upload_mbps: up,
        ping_ms: ping,
        jitter_ms: jitter,
        bytes_down,
        bytes_up,
        server,
        finished: SystemTime::now(),
    };
    on_event(SpeedEvent::Done(result.clone()));
    Ok(result)
}

/// One download or upload phase with parallel streams. Returns (Mb/s after warm-up, all bytes moved).
fn transfer(
    phase: SpeedPhase,
    t: &dyn SpeedTransport,
    cfg: &SpeedConfig,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(SpeedEvent),
) -> Result<(f64, u64)> {
    let (time, streams, req) = match phase {
        SpeedPhase::Download => (cfg.download_time, cfg.download_streams, cfg.download_request_bytes),
        _ => (cfg.upload_time, cfg.upload_streams, cfg.upload_request_bytes),
    };
    on_event(SpeedEvent::PhaseStarted(phase));
    let moved = AtomicU64::new(0);
    let over = AtomicBool::new(false);
    let errors: Mutex<Vec<NetError>> = Mutex::new(Vec::new());
    let start = Instant::now();
    let deadline = start + time;

    let outcome = std::thread::scope(|scope| {
        for _ in 0..streams.max(1) {
            scope.spawn(|| {
                let mut go = |n: u64| {
                    moved.fetch_add(n, Ordering::Relaxed);
                    !over.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) && Instant::now() < deadline
                };
                // At least one request per stream: a stream the OS starts late (busy PC) still counts its first
                // chunk, then `go` ends it.
                while !cancel.load(Ordering::Relaxed) {
                    let r = match phase {
                        SpeedPhase::Download => t.download(req, &mut go),
                        _ => t.upload(req, &mut go),
                    };
                    if let Err(e) = r {
                        errors.lock().unwrap().push(e);
                        break;
                    }
                    if over.load(Ordering::Relaxed) || Instant::now() >= deadline {
                        break;
                    }
                }
            });
        }

        // The ticker: live value every tick, warm-up mark, end of phase.
        let mut meter = RateMeter::new(cfg.warmup, cfg.live_window);
        loop {
            std::thread::sleep(cfg.tick);
            let now = Instant::now();
            let elapsed = now.duration_since(start);
            let bytes = moved.load(Ordering::Relaxed);
            if cancel.load(Ordering::Relaxed) {
                over.store(true, Ordering::Relaxed);
                return Err(NetError::Cancelled);
            }
            let live = meter.sample(elapsed, bytes);
            on_event(SpeedEvent::Progress { phase, mbps: live, elapsed });
            let all_failed = errors.lock().unwrap().len() >= streams.max(1);
            if now >= deadline || all_failed {
                over.store(true, Ordering::Relaxed);
                return Ok(meter.result(elapsed, bytes));
            }
        }
    });
    let value = outcome?;
    // Bytes that arrived while the streams were winding down still count as data used.
    let total = moved.load(Ordering::Relaxed);
    if total == 0 {
        return Err(errors.into_inner().unwrap().into_iter().next().unwrap_or(NetError::Timeout));
    }
    on_event(SpeedEvent::PhaseDone { phase, value });
    Ok((value, total))
}

/// A running speed test on its own thread (the page's Start / Test again button).
pub struct SpeedTest {
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<SpeedResult>>>,
}

impl SpeedTest {
    pub fn start(
        os: Arc<dyn NetworkOs>,
        transport: Arc<dyn SpeedTransport>,
        cfg: SpeedConfig,
        mut on_event: impl FnMut(SpeedEvent) + Send + 'static,
    ) -> SpeedTest {
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        let thread = std::thread::Builder::new()
            .name("bu-network-speedtest".into())
            .spawn(move || run(&os, transport.as_ref(), &cfg, &c, &mut on_event))
            .ok();
        SpeedTest { cancel, thread }
    }

    /// Stop early (page closed). The thread ends within one tick / one latency sample.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    /// Waits for the end and returns the result.
    pub fn join(mut self) -> Result<SpeedResult> {
        match self.thread.take() {
            Some(t) => t.join().unwrap_or(Err(NetError::Cancelled)),
            None => Err(NetError::Unsupported("could not start a thread".into())),
        }
    }
}

impl Drop for SpeedTest {
    fn drop(&mut self) {
        self.cancel();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
