//! The ping pill: thresholds, targets, and the sampler's start / stop (zero cost when stopped).

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use bu_network::fake::FakeNet;
use bu_network::ping::*;
use bu_network::{NetError, NetworkOs};

#[test]
fn pill_thresholds() {
    assert_eq!(PingLevel::from_ms(0), PingLevel::Green);
    assert_eq!(PingLevel::from_ms(39), PingLevel::Green);
    assert_eq!(PingLevel::from_ms(40), PingLevel::Amber);
    assert_eq!(PingLevel::from_ms(79), PingLevel::Amber);
    assert_eq!(PingLevel::from_ms(80), PingLevel::Red);
    assert_eq!(PingLevel::of(None), PingLevel::Lost);
    assert_eq!(round_ms(Duration::from_micros(39_499)), 39);
    assert_eq!(round_ms(Duration::from_micros(39_500)), 40);
}

fn os() -> (Arc<FakeNet>, Arc<dyn NetworkOs>) {
    let f = Arc::new(FakeNet::typical());
    (f.clone(), f)
}

#[test]
fn internet_target_is_1_1_1_1() {
    let (_f, os) = os();
    let s = ping_once(&os, &PingTarget::Internet, Duration::from_secs(1));
    assert_eq!(s.target, Some("1.1.1.1".parse().unwrap()));
    assert_eq!(s.ms(), Some(19));
    assert_eq!(s.level, PingLevel::Green);
}

#[test]
fn gateway_target_follows_the_adapter_in_use() {
    let (_f, os) = os();
    let s = ping_once(&os, &PingTarget::Gateway, Duration::from_secs(1));
    assert_eq!(s.target, Some("192.168.1.1".parse().unwrap()));
    assert_eq!(s.ms(), Some(1));
}

#[test]
fn red_and_lost() {
    let (f, os) = os();
    f.with(|s| {
        s.icmp.insert("9.9.9.9".parse().unwrap(), Duration::from_millis(120));
    });
    let s = ping_once(&os, &PingTarget::Host("9.9.9.9".parse().unwrap()), Duration::from_secs(1));
    assert_eq!(s.level, PingLevel::Red);
    let s = ping_once(&os, &PingTarget::Host("9.9.9.8".parse().unwrap()), Duration::from_secs(1));
    assert_eq!((s.level, s.error), (PingLevel::Lost, Some(NetError::Timeout)));
}

#[test]
fn offline_greys_the_pill() {
    let (f, os) = os();
    f.with(|s| s.internet_if = None);
    let s = ping_once(&os, &PingTarget::Gateway, Duration::from_secs(1));
    assert_eq!((s.level, s.error), (PingLevel::Lost, Some(NetError::Offline)));
    let s = ping_once(&os, &PingTarget::Internet, Duration::from_secs(1));
    assert_eq!(s.level, PingLevel::Lost);
}

#[test]
fn sampler_ticks_while_running_and_costs_nothing_after_stop() {
    let (f, os) = os();
    let (tx, rx) = mpsc::channel();
    let sampler = PingSampler::start(os, PingTarget::Internet, Duration::from_millis(20), move |s| {
        let _ = tx.send(s);
    });
    // The first ping is sent at once, then one per interval.
    let first = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(first.ms(), Some(19));
    for _ in 0..3 {
        rx.recv_timeout(Duration::from_secs(30)).unwrap();
    }
    assert!(sampler.is_running());
    sampler.stop();
    let reads = f.reads();
    std::thread::sleep(Duration::from_millis(120));
    assert_eq!(f.reads(), reads, "no pings after stop");
    // The callback was dropped with the thread: the channel is closed.
    while rx.try_recv().is_ok() {}
    assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Disconnected)));
}

/// Review 008 remark: a gateway tick is one ping, not a re-read of every adapter.
#[test]
fn gateway_sampler_looks_the_gateway_up_once() {
    let (f, os) = os();
    let (tx, rx) = mpsc::channel();
    let sampler = PingSampler::start(os, PingTarget::Gateway, Duration::from_millis(10), move |s| {
        let _ = tx.send(s);
    });
    let samples: Vec<PingSample> = (0..6).map(|_| rx.recv_timeout(Duration::from_secs(30)).unwrap()).collect();
    sampler.stop();
    let extra: usize = rx.try_iter().count();
    assert!(samples.iter().all(|s| s.ms() == Some(1)));
    // 1 adapter read + 1 route read for the lookup, then one ping per sample.
    assert_eq!(f.reads(), 2 + (samples.len() + extra) as u64);
}

#[test]
fn dropping_the_sampler_stops_it() {
    let (f, os) = os();
    let s = PingSampler::start(os, PingTarget::Internet, Duration::from_secs(3600), |_| {});
    std::thread::sleep(Duration::from_millis(50));
    let t0 = std::time::Instant::now();
    drop(s); // must not wait for the hour-long interval
    assert!(t0.elapsed() < Duration::from_secs(10), "generous bound for a busy PC; the interval is an hour");
    let reads = f.reads();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(f.reads(), reads);
}
