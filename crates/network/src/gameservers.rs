//! Game-server ping: one row per game, one EU region each. Measured only while asked: a sampler that runs one round
//! when started (the page opens) and one per `refresh()` (the Refresh button); stopped = no thread, zero cost.
//!
//! Many game servers ignore ICMP and most publishers publish no address at all. So every row has a list of
//! targets, tried in order: the game's own server first when one is public, then a STAND-IN in the same city
//! (a public endpoint in that data-centre region, flagged `stand_in` so the menu can show "≈"). Each target has
//! probes tried in order: ICMP echo, a UDP echo beacon, or a TCP connect (one handshake = one round trip).
//! The list and where each address comes from: [`eu_servers`] / reports/order_008.md.

use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc::{self, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::error::{NetError, Result};
use crate::os::NetworkOs;
use crate::ping::PingLevel;

/// How one target is pinged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Icmp,
    /// A UDP echo beacon on this port (e.g. AWS GameLift ping beacons, port 7770): send a few bytes, time the echo.
    Udp(u16),
    /// TCP connect to this port, closed at once (no data sent).
    Tcp(u16),
}

/// One address to ping for a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Host name or IP. A name with several addresses (e.g. Epic's) is pinged on every address; the lowest counts.
    pub host: String,
    pub probes: Vec<Probe>,
    /// True = not the game's own server, a public endpoint in the same city (the menu shows "≈").
    pub stand_in: bool,
    /// What the address is + source URL.
    pub source: String,
}

/// One row of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameServer {
    pub id: String,
    /// "VALORANT"
    pub game: String,
    /// "Riot Games"
    pub publisher: String,
    /// "Frankfurt" (the sub-line is "<publisher> · <region>")
    pub region: String,
    /// Tried in order; the first that answers gives the number.
    pub targets: Vec<Target>,
}

impl GameServer {
    pub fn sub_line(&self) -> String {
        format!("{} · {}", self.publisher, self.region)
    }
}

/// One row's measurement.
#[derive(Debug, Clone, PartialEq)]
pub struct GameServerResult {
    pub id: String,
    /// The address that answered (or the last one tried).
    pub addr: Option<IpAddr>,
    /// Lowest of the tries (None = nothing answered).
    pub rtt: Option<Duration>,
    pub level: PingLevel,
    /// The probe that answered.
    pub method: Option<Probe>,
    /// The number comes from a stand-in in the same city, not the game's own server.
    pub stand_in: bool,
    pub error: Option<NetError>,
}

/// Tries on the best address; the lowest answer counts (the line's base round trip, like the speed test's
/// "ping = lowest").
pub const TRIES: usize = 3;
pub const TIMEOUT: Duration = Duration::from_millis(1000);
/// At most this many addresses of one host name are pinged.
pub const MAX_ADDRESSES: usize = 12;

fn probe_once(os: &dyn NetworkOs, p: Probe, ip: IpAddr, timeout: Duration) -> Result<Duration> {
    match p {
        Probe::Icmp => os.icmp_ping(ip, timeout),
        Probe::Udp(port) => os.udp_ping(SocketAddr::new(ip, port), timeout),
        Probe::Tcp(port) => os.tcp_ping(SocketAddr::new(ip, port), timeout),
    }
}

/// Measures one row now: targets in order, probes in order. Every address of a target gets one try; the fastest
/// address then gets `tries - 1` more and the lowest counts.
pub fn measure(os: &dyn NetworkOs, s: &GameServer, tries: usize, timeout: Duration) -> GameServerResult {
    measure_until(os, s, tries, timeout, &AtomicBool::new(false))
}

/// [`measure`] that gives up (Err(Cancelled)) before its next probe once `stop` is set - so stopping a round waits
/// at most for the probes already in flight (one `timeout`).
pub fn measure_until(
    os: &dyn NetworkOs,
    s: &GameServer,
    tries: usize,
    timeout: Duration,
    stop: &AtomicBool,
) -> GameServerResult {
    let mut last_err = NetError::Timeout;
    let mut last_addr = None;
    let probe = |p, ip| {
        if stop.load(Ordering::Relaxed) {
            return Err(NetError::Cancelled);
        }
        probe_once(os, p, ip, timeout)
    };
    for t in &s.targets {
        if stop.load(Ordering::Relaxed) {
            last_err = NetError::Cancelled;
            break;
        }
        let ips = match os.resolve(&t.host) {
            Ok(ips) => {
                let v4: Vec<IpAddr> = ips.iter().filter(|i| i.is_ipv4()).copied().collect();
                let mut use_ips = if v4.is_empty() { ips } else { v4 };
                use_ips.truncate(MAX_ADDRESSES);
                use_ips
            }
            Err(e) => {
                last_err = e;
                continue;
            }
        };
        for &p in &t.probes {
            let mut best: Option<(Duration, IpAddr)> = None;
            for &ip in &ips {
                last_addr = Some(ip);
                match probe(p, ip) {
                    Ok(d) => {
                        if best.is_none_or(|(b, _)| d < b) {
                            best = Some((d, ip));
                        }
                    }
                    Err(e) => last_err = e,
                }
            }
            if let Some((mut rtt, ip)) = best {
                for _ in 1..tries.max(1) {
                    if let Ok(d) = probe(p, ip) {
                        rtt = rtt.min(d);
                    }
                }
                return GameServerResult {
                    id: s.id.clone(),
                    addr: Some(ip),
                    rtt: Some(rtt),
                    level: PingLevel::of(Some(rtt)),
                    method: Some(p),
                    stand_in: t.stand_in,
                    error: None,
                };
            }
        }
    }
    GameServerResult {
        id: s.id.clone(),
        addr: last_addr,
        rtt: None,
        level: PingLevel::Lost,
        method: None,
        stand_in: false,
        error: Some(last_err),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GameServerEvent {
    /// Every row shows the greyed pulsing "…".
    RoundStarted,
    /// One row is done (rows arrive as they finish; the menu fills them 130 ms apart).
    Result(GameServerResult),
    RoundDone,
}

enum Cmd {
    Round,
    Stop,
}

/// Runs a round at start and on every `refresh()`; optional `repeat` adds a round every `repeat` while running.
/// Between rounds the thread waits on a channel (no timer unless `repeat` is set).
pub struct GameServerSampler {
    tx: Option<Sender<Cmd>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl GameServerSampler {
    pub fn start(
        os: Arc<dyn NetworkOs>,
        servers: Vec<GameServer>,
        repeat: Option<Duration>,
        on_event: impl FnMut(GameServerEvent) + Send + 'static,
    ) -> GameServerSampler {
        Self::start_with_tries(os, servers, repeat, TRIES, on_event)
    }

    /// [`Self::start`] with `tries` probes per row and round. v22's game-server pinger: `repeat` = 1 s, `tries` = 1 (one
    /// ping per region per second; AWS's beacons allow 3 per second).
    pub fn start_with_tries(
        os: Arc<dyn NetworkOs>,
        servers: Vec<GameServer>,
        repeat: Option<Duration>,
        tries: usize,
        mut on_event: impl FnMut(GameServerEvent) + Send + 'static,
    ) -> GameServerSampler {
        let (tx, rx) = mpsc::channel::<Cmd>();
        let _ = tx.send(Cmd::Round);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("bu-network-gameservers".into())
            .spawn(move || loop {
                let cmd = match repeat {
                    Some(every) => match rx.recv_timeout(every) {
                        Ok(c) => c,
                        Err(mpsc::RecvTimeoutError::Timeout) => Cmd::Round,
                        Err(_) => Cmd::Stop,
                    },
                    None => rx.recv().unwrap_or(Cmd::Stop),
                };
                match cmd {
                    Cmd::Stop => break,
                    Cmd::Round => {
                        // Refresh pressed twice during one round = one more round, not a queue of them.
                        while let Ok(Cmd::Round) = rx.try_recv() {}
                        round_tries(os.as_ref(), &servers, tries, &mut on_event, &stop_flag);
                    }
                }
            })
            .ok();
        GameServerSampler { tx: Some(tx), stop, thread }
    }

    /// The Refresh button.
    pub fn refresh(&self) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Cmd::Round);
        }
    }

    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Stops and waits for the thread. A round in progress gives up before its next probe, so this waits at most
    /// for the probes already in flight (one `TIMEOUT`, 1 s); no results are reported after it.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Cmd::Stop);
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for GameServerSampler {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// One round: every server in parallel (one short-lived thread each), results reported as they come.
pub fn round(os: &dyn NetworkOs, servers: &[GameServer], on_event: &mut dyn FnMut(GameServerEvent)) {
    round_until(os, servers, on_event, &AtomicBool::new(false));
}

/// [`round`] that ends early once `stop` is set: nothing more is reported (no RoundDone either).
pub fn round_until(
    os: &dyn NetworkOs,
    servers: &[GameServer],
    on_event: &mut dyn FnMut(GameServerEvent),
    stop: &AtomicBool,
) {
    round_tries(os, servers, TRIES, on_event, stop)
}

fn round_tries(
    os: &dyn NetworkOs,
    servers: &[GameServer],
    tries: usize,
    on_event: &mut dyn FnMut(GameServerEvent),
    stop: &AtomicBool,
) {
    on_event(GameServerEvent::RoundStarted);
    let (rtx, rrx) = mpsc::channel();
    std::thread::scope(|scope| {
        for s in servers {
            let rtx = rtx.clone();
            scope.spawn(move || {
                let _ = rtx.send(measure_until(os, s, tries, TIMEOUT, stop));
            });
        }
        drop(rtx);
        for r in rrx {
            if !stop.load(Ordering::Relaxed) {
                on_event(GameServerEvent::Result(r));
            }
        }
    });
    if !stop.load(Ordering::Relaxed) {
        on_event(GameServerEvent::RoundDone);
    }
}

/// The EU list (one region per game, the DESIGN rows). Sources: src/gamelist.rs, reports/order_008.md.
pub fn eu_servers() -> Vec<GameServer> {
    crate::gamelist::eu()
}
