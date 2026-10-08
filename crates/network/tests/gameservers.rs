//! Game-server ping: probe fallback, results, the sampler (start = one round, refresh = one more, stop = nothing).

use std::net::SocketAddr;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use bu_network::fake::FakeNet;
use bu_network::gameservers::*;
use bu_network::ping::PingLevel;
use bu_network::{NetError, NetworkOs};

fn target(host: &str, probes: Vec<Probe>, stand_in: bool) -> Target {
    Target { host: host.into(), probes, stand_in, source: "test".into() }
}

fn server(id: &str, host: &str, probes: Vec<Probe>) -> GameServer {
    GameServer {
        id: id.into(),
        game: id.into(),
        publisher: "Pub".into(),
        region: "Frankfurt".into(),
        targets: vec![target(host, probes, false)],
    }
}

fn fake() -> Arc<FakeNet> {
    let f = Arc::new(FakeNet::typical());
    f.with(|s| {
        s.names.insert("icmp.example".into(), vec!["10.0.0.1".parse().unwrap()]);
        s.names.insert("tcp.example".into(), vec!["10.0.0.2".parse().unwrap()]);
        s.names.insert("dead.example".into(), vec!["10.0.0.3".parse().unwrap()]);
        s.icmp.insert("10.0.0.1".parse().unwrap(), Duration::from_millis(23));
        s.tcp.insert("10.0.0.2:443".parse::<SocketAddr>().unwrap(), Duration::from_millis(55));
    });
    f
}

#[test]
fn icmp_answer() {
    let f = fake();
    let r = measure(f.as_ref(), &server("a", "icmp.example", vec![Probe::Icmp, Probe::Tcp(443)]), 3, TIMEOUT);
    assert_eq!(r.rtt, Some(Duration::from_millis(23)));
    assert_eq!(r.method, Some(Probe::Icmp));
    assert_eq!(r.level, PingLevel::Green);
}

#[test]
fn falls_back_to_tcp_when_icmp_is_blocked() {
    let f = fake();
    let r = measure(f.as_ref(), &server("b", "tcp.example", vec![Probe::Icmp, Probe::Tcp(443)]), 3, TIMEOUT);
    assert_eq!(r.rtt, Some(Duration::from_millis(55)));
    assert_eq!(r.method, Some(Probe::Tcp(443)));
    assert_eq!(r.level, PingLevel::Amber);
    assert_eq!(r.addr, Some("10.0.0.2".parse().unwrap()));
}

#[test]
fn nothing_answers() {
    let f = fake();
    let r = measure(f.as_ref(), &server("c", "dead.example", vec![Probe::Icmp, Probe::Tcp(443)]), 3, TIMEOUT);
    assert_eq!((r.rtt, r.level, r.error), (None, PingLevel::Lost, Some(NetError::Timeout)));
}

#[test]
fn udp_beacon() {
    let f = fake();
    f.with(|s| {
        s.names.insert("beacon.example".into(), vec!["10.0.0.4".parse().unwrap()]);
        s.udp.insert("10.0.0.4:7770".parse::<SocketAddr>().unwrap(), Duration::from_millis(21));
    });
    let r = measure(f.as_ref(), &server("v", "beacon.example", vec![Probe::Udp(7770)]), 3, TIMEOUT);
    assert_eq!((r.rtt, r.method), (Some(Duration::from_millis(21)), Some(Probe::Udp(7770))));
}

#[test]
fn own_server_silent_then_stand_in_answers_and_is_flagged() {
    let f = fake();
    let mut g = server("lol", "dead.example", vec![Probe::Icmp]);
    g.targets.push(target("icmp.example", vec![Probe::Icmp], true));
    let r = measure(f.as_ref(), &g, 3, TIMEOUT);
    assert_eq!(r.rtt, Some(Duration::from_millis(23)));
    assert!(r.stand_in);
    // The own server answering wins and is not flagged.
    let r = measure(f.as_ref(), &server("a", "icmp.example", vec![Probe::Icmp]), 3, TIMEOUT);
    assert!(!r.stand_in);
}

#[test]
fn every_address_of_a_name_is_tried_and_the_lowest_counts() {
    let f = fake();
    f.with(|s| {
        s.names.insert(
            "multi.example".into(),
            vec!["10.1.0.1".parse().unwrap(), "10.1.0.2".parse().unwrap(), "10.1.0.3".parse().unwrap()],
        );
        s.icmp.insert("10.1.0.1".parse().unwrap(), Duration::from_millis(29));
        s.icmp.insert("10.1.0.2".parse().unwrap(), Duration::from_millis(16));
        s.icmp.insert("10.1.0.3".parse().unwrap(), Duration::from_millis(23));
    });
    let reads = f.reads();
    let r = measure(f.as_ref(), &server("fn", "multi.example", vec![Probe::Icmp]), 3, TIMEOUT);
    assert_eq!((r.rtt, r.addr), (Some(Duration::from_millis(16)), Some("10.1.0.2".parse().unwrap())));
    // 1 lookup + 3 addresses x 1 try + 2 more tries on the best one.
    assert_eq!(f.reads() - reads, 1 + 3 + 2);
}

#[test]
fn unknown_host() {
    let f = fake();
    let r = measure(f.as_ref(), &server("d", "nope.example", vec![Probe::Icmp]), 3, TIMEOUT);
    assert_eq!(r.error, Some(NetError::Resolve("nope.example".into())));
}

#[test]
fn offline_rows_are_lost() {
    let f = fake();
    f.with(|s| s.internet_if = None);
    let r = measure(f.as_ref(), &server("a", "icmp.example", vec![Probe::Icmp]), 3, TIMEOUT);
    assert_eq!(r.level, PingLevel::Lost);
}

fn three() -> Vec<GameServer> {
    vec![
        server("a", "icmp.example", vec![Probe::Icmp]),
        server("b", "tcp.example", vec![Probe::Icmp, Probe::Tcp(443)]),
        server("c", "dead.example", vec![Probe::Icmp]),
    ]
}

fn collect_round(rx: &mpsc::Receiver<GameServerEvent>) -> Vec<GameServerResult> {
    assert_eq!(rx.recv_timeout(Duration::from_secs(30)).unwrap(), GameServerEvent::RoundStarted);
    let mut out = Vec::new();
    loop {
        match rx.recv_timeout(Duration::from_secs(30)).unwrap() {
            GameServerEvent::Result(r) => out.push(r),
            GameServerEvent::RoundDone => return out,
            e => panic!("unexpected {e:?}"),
        }
    }
}

#[test]
fn sampler_start_refresh_stop() {
    let f = fake();
    let os: Arc<dyn NetworkOs> = f.clone();
    let (tx, rx) = mpsc::channel();
    let s = GameServerSampler::start(os, three(), None, move |e| {
        let _ = tx.send(e);
    });
    let mut first = collect_round(&rx);
    first.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(first.len(), 3);
    assert_eq!(first[0].rtt, Some(Duration::from_millis(23)));
    assert_eq!(first[1].method, Some(Probe::Tcp(443)));
    assert_eq!(first[2].level, PingLevel::Lost);

    // Idle between rounds: no measuring until Refresh.
    let reads = f.reads();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(f.reads(), reads, "no background measuring between rounds");
    assert!(rx.try_recv().is_err());

    s.refresh();
    assert_eq!(collect_round(&rx).len(), 3);
    assert!(s.is_running());
    s.stop();
    let reads = f.reads();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(f.reads(), reads);
}

/// Review 008 remark 5: a slow round (many addresses, every probe timing out) must not hold `stop()` / drop for the
/// whole round - it gives up before its next probe.
#[test]
fn stop_in_the_middle_of_a_slow_round_returns_within_one_probe() {
    let f = fake();
    f.with(|s| {
        s.probe_delay = Duration::from_secs(2);
        let many: Vec<std::net::IpAddr> = (1..=12).map(|i| format!("10.9.0.{i}").parse().unwrap()).collect();
        s.names.insert("slow.example".into(), many);
    });
    // 12 addresses x (ICMP + TCP) x 2 s each = 24 probes, ~48 s per row if it ran to the end (rows run side by side).
    let slow: Vec<GameServer> = (0..3)
        .map(|i| server(&format!("s{i}"), "slow.example", vec![Probe::Icmp, Probe::Tcp(443)]))
        .collect();
    let rows = slow.len() as u64;
    let before = f.reads();
    let (tx, rx) = mpsc::channel();
    let s = GameServerSampler::start(f.clone(), slow, None, move |e| {
        let _ = tx.send(e);
    });
    assert_eq!(rx.recv_timeout(Duration::from_secs(30)).unwrap(), GameServerEvent::RoundStarted);
    std::thread::sleep(Duration::from_millis(300));
    let t0 = std::time::Instant::now();
    s.stop();
    // Quiet PC: the one 2 s probe in flight. Generous bound for a busy PC (13.5 s seen with 24 test runs on 8 cores)
    // - still far below the 48 s round.
    assert!(t0.elapsed() < Duration::from_secs(30), "stop took {:?}", t0.elapsed());
    assert!(f.reads() - before < rows * 25, "the round ran to the end");
    // Nothing reported after stop (no results of the cancelled rows, no RoundDone).
    assert!(rx.try_iter().all(|e| e == GameServerEvent::RoundStarted));
    let reads = f.reads();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(f.reads(), reads);
}

#[test]
fn sampler_optional_repeat() {
    let f = fake();
    let (tx, rx) = mpsc::channel();
    let s = GameServerSampler::start(f, three(), Some(Duration::from_millis(30)), move |e| {
        let _ = tx.send(e);
    });
    collect_round(&rx);
    collect_round(&rx); // came by itself
    drop(s);
}

#[test]
fn the_builtin_list_is_complete() {
    let list = eu_servers();
    let games: Vec<&str> = list.iter().map(|g| g.game.as_str()).collect();
    assert_eq!(
        games,
        vec!["VALORANT", "Counter-Strike 2", "Fortnite", "Rocket League", "League of Legends", "Apex Legends", "Dota 2"]
    );
    for g in &list {
        assert!(!g.targets.is_empty(), "{g:?}");
        for t in &g.targets {
            assert!(!t.host.is_empty() && !t.probes.is_empty() && !t.source.is_empty(), "{t:?}");
        }
        assert!(g.sub_line().contains(" · "));
    }
    // Only the public game-owned addresses are not stand-ins.
    let own: Vec<&str> =
        list.iter().filter(|g| g.targets.iter().any(|t| !t.stand_in)).map(|g| g.id.as_str()).collect();
    assert_eq!(own, vec!["cs2", "fortnite", "league", "dota2"]);
    let ids: std::collections::HashSet<_> = list.iter().map(|g| g.id.clone()).collect();
    assert_eq!(ids.len(), list.len());
}
