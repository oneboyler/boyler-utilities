//! The ping pill: one ping per second to a fixed host (or the gateway) while the Network page is open.
//! Started by the page, stopped when it closes; stopped = no thread, no timer, zero cost.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::error::{NetError, Result};
use crate::os::NetworkOs;
use crate::service::NetworkService;

/// The pill colour (DESIGN §3.11: green < 40 / amber 40-79 / red >= 80 ms).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PingLevel {
    Green,
    Amber,
    Red,
    /// No answer (timeout / unreachable): the pill greys out.
    Lost,
}

impl PingLevel {
    pub fn from_ms(ms: u32) -> PingLevel {
        match ms {
            0..=39 => PingLevel::Green,
            40..=79 => PingLevel::Amber,
            _ => PingLevel::Red,
        }
    }
    pub fn of(rtt: Option<Duration>) -> PingLevel {
        rtt.map(|d| PingLevel::from_ms(round_ms(d))).unwrap_or(PingLevel::Lost)
    }
}

/// Milliseconds as the pill shows them (rounded; under 0.5 ms shows 0 - the menu may show "<1").
pub fn round_ms(d: Duration) -> u32 {
    ((d.as_secs_f64() * 1000.0).round() as u64).min(u32::MAX as u64) as u32
}

/// What the pill pings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PingTarget {
    /// The default: Cloudflare's 1.1.1.1 - answers ICMP everywhere, so the pill shows the internet ping, not just
    /// the router's (the router answers in ~1 ms and would always be green).
    Internet,
    /// The gateway (router) of the adapter in use - shows Wi-Fi / LAN trouble only.
    Gateway,
    /// Any fixed address.
    Host(IpAddr),
}

pub const INTERNET_PING_HOST: IpAddr = IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1));

/// One tick of the pill.
#[derive(Debug, Clone, PartialEq)]
pub struct PingSample {
    pub target: Option<IpAddr>,
    /// None = no answer.
    pub rtt: Option<Duration>,
    pub level: PingLevel,
    /// Why there was no answer (Offline, Timeout, ...).
    pub error: Option<NetError>,
}

impl PingSample {
    pub fn ms(&self) -> Option<u32> {
        self.rtt.map(round_ms)
    }
}

/// Resolves the target for this tick (the gateway follows the adapter in use).
fn resolve_target(service: &NetworkService, target: &PingTarget) -> Result<IpAddr> {
    match target {
        PingTarget::Internet => Ok(INTERNET_PING_HOST),
        PingTarget::Host(ip) => Ok(*ip),
        PingTarget::Gateway => {
            let conn = service.connection_state()?;
            let a = conn.in_use_adapter().ok_or(NetError::Offline)?;
            a.gateways
                .iter()
                .find(|g| g.is_ipv4())
                .or_else(|| a.gateways.first())
                .copied()
                .ok_or(NetError::Offline)
        }
    }
}

/// One ping right now (no sampler).
pub fn ping_once(os: &Arc<dyn NetworkOs>, target: &PingTarget, timeout: Duration) -> PingSample {
    let service = NetworkService::new(os.clone());
    let ip = match resolve_target(&service, target) {
        Ok(ip) => ip,
        Err(e) => return PingSample { target: None, rtt: None, level: PingLevel::Lost, error: Some(e) },
    };
    ping_ip(os, ip, timeout)
}

fn ping_ip(os: &Arc<dyn NetworkOs>, ip: IpAddr, timeout: Duration) -> PingSample {
    match os.icmp_ping(ip, timeout) {
        Ok(rtt) => PingSample { target: Some(ip), rtt: Some(rtt), level: PingLevel::of(Some(rtt)), error: None },
        Err(e) => PingSample { target: Some(ip), rtt: None, level: PingLevel::Lost, error: Some(e) },
    }
}

/// Pings every `interval` on its own thread until stopped. The first ping is sent at once.
/// The thread sleeps in a channel wait between pings (no busy loop) and ends the moment `stop()` is called or the
/// sampler is dropped.
pub struct PingSampler {
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl PingSampler {
    /// DESIGN: 1 s interval. Timeout per ping = the interval (capped at 1 s) so ticks never pile up.
    pub const INTERVAL: Duration = Duration::from_secs(1);

    pub fn start(
        os: Arc<dyn NetworkOs>,
        target: PingTarget,
        interval: Duration,
        mut on_sample: impl FnMut(PingSample) + Send + 'static,
    ) -> PingSampler {
        let (tx, rx) = mpsc::channel::<()>();
        let timeout = interval.min(Duration::from_secs(1));
        let thread = std::thread::Builder::new()
            .name("bu-network-ping".into())
            .spawn(move || {
                // The gateway is looked up once and kept while it answers; a lost ping looks it up again next tick
                // (the adapter in use may have changed). So a tick is one ping, not a full adapter read.
                let mut gateway: Option<IpAddr> = None;
                loop {
                    let sample = match gateway {
                        Some(ip) => ping_ip(&os, ip, timeout),
                        None => ping_once(&os, &target, timeout),
                    };
                    gateway = match target {
                        PingTarget::Gateway if sample.rtt.is_some() => sample.target,
                        _ => None,
                    };
                    on_sample(sample);
                    match rx.recv_timeout(interval) {
                        Err(RecvTimeoutError::Timeout) => continue,
                        _ => break, // stop() sent, or the sampler was dropped
                    }
                }
            })
            .ok();
        PingSampler { stop: Some(tx), thread }
    }

    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Stops and waits for the thread to end (at most one ping timeout).
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for PingSampler {
    fn drop(&mut self) {
        self.shutdown();
    }
}
