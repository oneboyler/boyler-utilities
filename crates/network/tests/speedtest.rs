//! Speed test engine against a fake server: order of phases, live progress, results, data used, errors, cancel.
//! Load-proof: the fake runs on real threads that share the PC with other builds, so the threaded runs only assert
//! what does not depend on timing; the speed numbers are tested exactly on [`RateMeter`] with synthetic ticks.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use bu_network::fake::FakeNet;
use bu_network::speedtest::*;
use bu_network::{NetError, NetworkOs};

/// A fake server with a fixed speed per stream (bytes/s), delivered by elapsed time. Every request delivers a first
/// chunk at once, so a stream counts bytes however late the OS runs it.
struct FakeServer {
    down_rate: f64,
    up_rate: f64,
    latencies: Mutex<VecDeque<f64>>,
    fail_down: Option<NetError>,
    calls: AtomicUsize,
}

impl FakeServer {
    fn new(down_mbps_per_stream: f64, up_mbps_per_stream: f64, lat: &[f64]) -> FakeServer {
        FakeServer {
            down_rate: down_mbps_per_stream * 1e6 / 8.0,
            up_rate: up_mbps_per_stream * 1e6 / 8.0,
            latencies: Mutex::new(lat.iter().copied().collect()),
            fail_down: None,
            calls: AtomicUsize::new(0),
        }
    }

    fn pump(rate: f64, bytes: u64, on: &mut dyn FnMut(u64) -> bool) {
        const FIRST: u64 = 1000;
        let t0 = Instant::now();
        let mut sent = 0u64;
        let mut first = true;
        while sent < bytes {
            if !std::mem::take(&mut first) {
                std::thread::sleep(Duration::from_millis(2));
            }
            let due = (FIRST + (t0.elapsed().as_secs_f64() * rate) as u64).min(bytes);
            let n = due - sent;
            sent = due;
            if n > 0 && !on(n) {
                return;
            }
        }
    }
}

impl SpeedTransport for FakeServer {
    fn server(&self) -> bu_network::Result<ServerInfo> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(ServerInfo { city: "Vienna".into(), code: "VIE".into(), provider: "Fake".into() })
    }
    fn download(&self, bytes: u64, on: &mut dyn FnMut(u64) -> bool) -> bu_network::Result<()> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if let Some(e) = &self.fail_down {
            return Err(e.clone());
        }
        Self::pump(self.down_rate, bytes, on);
        Ok(())
    }
    fn upload(&self, bytes: u64, on: &mut dyn FnMut(u64) -> bool) -> bu_network::Result<()> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Self::pump(self.up_rate, bytes, on);
        Ok(())
    }
    fn latency(&self) -> bu_network::Result<Duration> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let ms = self.latencies.lock().unwrap().pop_front().ok_or(NetError::Timeout)?;
        Ok(Duration::from_secs_f64(ms / 1000.0))
    }
}

fn quick() -> SpeedConfig {
    SpeedConfig {
        download_time: Duration::from_millis(800),
        upload_time: Duration::from_millis(600),
        warmup: Duration::from_millis(200),
        download_streams: 3,
        upload_streams: 2,
        download_request_bytes: 400_000,
        upload_request_bytes: 300_000,
        latency_samples: 5,
        latency_spacing: Duration::from_millis(1),
        tick: Duration::from_millis(25),
        live_window: Duration::from_millis(200),
    }
}

fn os() -> (Arc<FakeNet>, Arc<dyn NetworkOs>) {
    let f = Arc::new(FakeNet::typical());
    (f.clone(), f)
}

#[test]
fn stats_math() {
    assert_eq!(mbps(125_000_000, Duration::from_secs(1)), 1000.0);
    assert_eq!(mbps(1, Duration::ZERO), 0.0);
    assert_eq!(ping_and_jitter(&[]), None);
    assert_eq!(ping_and_jitter(&[20.0]), Some((20.0, 0.0)));
    // ping = lowest; jitter = mean |change| = (2 + 3 + 6) / 3
    assert_eq!(ping_and_jitter(&[20.0, 22.0, 19.0, 25.0]), Some((19.0, 11.0 / 3.0)));
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// 15 MB/s = 120 Mb/s, ticks every 100 ms for 7 s, warm-up 1 s, live window 1 s (the real defaults).
#[test]
fn meter_steady_speed() {
    let mut m = RateMeter::new(ms(1000), ms(1000));
    for t in (100..=7000).step_by(100) {
        let live = m.sample(ms(t), 15_000 * t);
        assert!(close(live, 120.0), "tick {t}: live {live}");
    }
    assert!(close(m.result(ms(7000), 15_000 * 7000), 120.0));
}

/// TCP still speeding up: nothing in the first second, then 120 Mb/s. The result leaves the warm-up out; the live
/// value is the speed over the last second only.
#[test]
fn meter_leaves_out_the_warm_up_and_shows_the_last_second() {
    let bytes = |t: u64| 15_000 * t.saturating_sub(1000);
    let mut m = RateMeter::new(ms(1000), ms(1000));
    let mut live = Vec::new();
    for t in (100..=7000).step_by(100) {
        live.push((t, m.sample(ms(t), bytes(t))));
    }
    assert!(close(live[4].1, 0.0), "0.5 s: nothing yet");
    assert!(close(live[14].1, 60.0), "1.5 s: half of the last second moved data -> {}", live[14].1);
    assert!(close(live[29].1, 120.0), "3.0 s: full speed -> {}", live[29].1);
    // From the warm-up mark: 90 MB in 6 s = 120 Mb/s (counting from 0 would give 102.9).
    assert!(close(m.result(ms(7000), bytes(7000)), 120.0));
}

/// A busy PC runs the ticker late and unevenly: the warm-up mark is the first tick at or after the warm-up, the
/// numbers use the real tick times, so they stay right.
#[test]
fn meter_with_late_uneven_ticks() {
    let mut m = RateMeter::new(ms(1000), ms(1000));
    for t in [300, 1400, 1450, 2900, 4000, 4100, 7300] {
        let live = m.sample(ms(t), 15_000 * t);
        assert!(close(live, 120.0), "tick {t}: live {live}");
    }
    // Warm mark = 1.4 s (21 MB); result over 1.4 .. 7.3 s.
    assert!(close(m.result(ms(7300), 15_000 * 7300), 120.0));
    let mut early = RateMeter::new(ms(1000), ms(1000));
    early.sample(ms(500), 1_000_000);
    // No tick reached the warm-up: counted from the start (1 MB in 0.5 s = 16 Mb/s).
    assert!(close(early.result(ms(500), 1_000_000), 16.0));
    assert_eq!(RateMeter::new(ms(1000), ms(1000)).sample(Duration::ZERO, 0), 0.0, "no time, no division by zero");
    // The ticker stalled past the whole phase: its warm-up tick is also the last one -> counted from the start, not 0.
    let mut stalled = RateMeter::new(ms(1000), ms(1000));
    stalled.sample(ms(100), 1_500_000);
    stalled.sample(ms(7200), 15_000 * 7200);
    assert!(close(stalled.result(ms(7200), 15_000 * 7200), 120.0));
}

#[test]
fn full_run_order_values_and_data_used() {
    let (_f, os) = os();
    // 3 x 40 = 120 Mb/s down, 2 x 15 = 30 Mb/s up. First latency value is the discarded warm-up.
    let server = FakeServer::new(40.0, 15.0, &[300.0, 20.0, 22.0, 19.0, 25.0, 21.0]);
    let cfg = quick();
    let mut events = Vec::new();
    let r = run(&os, &server, &cfg, &AtomicBool::new(false), &mut |e| events.push(e)).unwrap();

    // Speeds depend on how the OS runs the fake's threads - on a busy PC no stream may run between two ticks (exact
    // numbers: the meter_* tests). Deterministic here: data moved and was counted, the numbers are real numbers.
    for v in [r.download_mbps, r.upload_mbps] {
        assert!(v.is_finite() && v >= 0.0, "{v}");
    }
    assert!(r.bytes_down > 0 && r.bytes_up > 0, "{} / {}", r.bytes_down, r.bytes_up);
    assert_eq!(r.ping_ms, 19.0);
    assert!(close(r.jitter_ms, (2.0 + 3.0 + 6.0 + 4.0) / 4.0));
    assert_eq!(r.server.city, "Vienna");
    assert_eq!(r.toast(), format!("Speed test done · ↓ {:.0} · ↑ {:.0} Mb/s", r.download_mbps, r.upload_mbps));
    // The phase numbers are the result's numbers; Done carries the returned result.
    let done_value = |p: SpeedPhase| {
        events.iter().find_map(|e| match e {
            SpeedEvent::PhaseDone { phase, value } if *phase == p => Some(*value),
            _ => None,
        })
    };
    assert_eq!(done_value(SpeedPhase::Download), Some(r.download_mbps));
    assert_eq!(done_value(SpeedPhase::Upload), Some(r.upload_mbps));
    assert_eq!(done_value(SpeedPhase::Latency), Some(19.0));
    assert_eq!(events.last(), Some(&SpeedEvent::Done(r.clone())));

    // Order: server, download, upload, latency, done.
    let marks: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            SpeedEvent::Server(_) => Some("server".into()),
            SpeedEvent::PhaseStarted(p) => Some(format!("start {p:?}")),
            SpeedEvent::PhaseDone { phase, .. } => Some(format!("done {phase:?}")),
            SpeedEvent::Done(_) => Some("done".into()),
            _ => None,
        })
        .collect();
    assert_eq!(
        marks,
        vec![
            "server",
            "start Download",
            "done Download",
            "start Upload",
            "done Upload",
            "start Latency",
            "done Latency",
            "done"
        ]
    );
    // Live progress for the gauge, both phases: ticks with rising elapsed, the last one at the end of the phase (how
    // many ticks fit depends on the PC's load).
    for (p, time) in [(SpeedPhase::Download, cfg.download_time), (SpeedPhase::Upload, cfg.upload_time)] {
        let ticks: Vec<(f64, Duration)> = events
            .iter()
            .filter_map(|e| match e {
                SpeedEvent::Progress { phase, mbps, elapsed } if *phase == p => Some((*mbps, *elapsed)),
                _ => None,
            })
            .collect();
        assert!(!ticks.is_empty(), "{p:?}: no ticks");
        assert!(ticks.windows(2).all(|w| w[1].1 > w[0].1), "{p:?}: elapsed must rise");
        assert!(ticks.last().unwrap().1 >= time, "{p:?}: the phase ends at its time, not before");
        assert!(ticks.iter().all(|t| t.0 >= 0.0 && t.0.is_finite()));
    }
    let lat: Vec<f64> = events
        .iter()
        .filter_map(|e| match e {
            SpeedEvent::LatencySample { ms } => Some(*ms),
            _ => None,
        })
        .collect();
    assert_eq!(lat.len(), 5);
    assert!((lat[0] - 20.0).abs() < 1e-9, "the warm-up sample (300) is not shown");
}

#[test]
fn offline_sends_nothing() {
    let (f, os) = os();
    f.with(|s| s.internet_if = None);
    let server = FakeServer::new(10.0, 10.0, &[]);
    let r = run(&os, &server, &quick(), &AtomicBool::new(false), &mut |_| {});
    assert_eq!(r, Err(NetError::Offline));
    assert_eq!(server.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn server_errors_surface() {
    let (_f, os) = os();
    let mut server = FakeServer::new(10.0, 10.0, &[1.0, 1.0]);
    server.fail_down = Some(NetError::Http(429));
    let mut cfg = quick();
    cfg.download_time = Duration::from_secs(60);
    let t0 = Instant::now();
    let r = run(&os, &server, &cfg, &AtomicBool::new(false), &mut |_| {});
    assert_eq!(r, Err(NetError::Http(429)));
    // Generous bound (a busy PC runs the ticker late): far below the 60 s phase.
    assert!(t0.elapsed() < Duration::from_secs(15), "all streams failed: no waiting out the phase");
}

#[test]
fn no_latency_answer_is_an_error() {
    let (_f, os) = os();
    let server = FakeServer::new(10.0, 10.0, &[]);
    let r = run(&os, &server, &quick(), &AtomicBool::new(false), &mut |_| {});
    assert_eq!(r, Err(NetError::Timeout));
}

#[test]
fn cancel_stops_early() {
    let (_f, os) = os();
    let server: Arc<dyn SpeedTransport> = Arc::new(FakeServer::new(10.0, 10.0, &[]));
    let mut cfg = quick();
    cfg.download_time = Duration::from_secs(60);
    let (tx, rx) = mpsc::channel();
    let t = SpeedTest::start(os, server, cfg, move |e| {
        let _ = tx.send(e);
    });
    // Wait for the gauge to move, then close the page.
    loop {
        if let SpeedEvent::Progress { .. } = rx.recv_timeout(Duration::from_secs(30)).unwrap() {
            break;
        }
    }
    let t0 = Instant::now();
    t.cancel();
    assert_eq!(t.join(), Err(NetError::Cancelled));
    // Within one tick when quiet; generous bound for a busy PC - far below the 60 s phase.
    assert!(t0.elapsed() < Duration::from_secs(15), "cancel took {:?}", t0.elapsed());
}

#[test]
fn background_run_returns_the_result() {
    let (_f, os) = os();
    let server: Arc<dyn SpeedTransport> = Arc::new(FakeServer::new(20.0, 20.0, &[9.0, 10.0, 11.0, 10.0, 12.0, 10.0]));
    let t = SpeedTest::start(os, server, quick(), |_| {});
    let r = t.join().unwrap();
    assert_eq!(r.ping_ms, 10.0);
}

/// speedtest.net's number: 20 slices of 100 ms - 6 still ramping (0), 12 at 120 Mb/s, 2 bursts at 400 Mb/s. The slowest 30 %
/// (6) and the fastest 10 % (2) are left out: 120 Mb/s, where the plain average would say 112.
#[test]
fn meter_result_trims_the_slowest_and_fastest_slices() {
    let mut m = RateMeter::new(ms(0), ms(1000));
    let mut bytes = 0u64;
    for i in 1..=20u64 {
        bytes += match i {
            1..=6 => 0,
            7 | 15 => 5_000_000,
            _ => 1_500_000,
        };
        m.sample(ms(i * 100), bytes);
    }
    assert!(close(m.trimmed().unwrap(), 120.0), "{:?}", m.trimmed());
    assert!(close(m.result(ms(2000), bytes), 120.0));
    assert!(close(mbps(bytes, ms(2000)), 112.0));
    // too few slices to trim: none
    let mut few = RateMeter::new(ms(0), ms(1000));
    few.sample(ms(100), 1_500_000);
    assert_eq!(few.trimmed(), None);
}
