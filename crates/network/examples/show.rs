//! Prints the REAL network state, read-only: adapters, which is in use, link speed, Wi-Fi radio, DNS (hand-set +
//! in use), the device each switch would act on, admin or not, and a few pings. Changes NOTHING.
//!
//!   cargo run -p bu-network --example network-show                 state + pings
//!   ... -- --wifi        + the nearby Wi-Fi list (read)    ... -- --regions  + one round of every game's regions (Order 022)
//!   cargo run -p bu-network --example network-show -- --games      + one game-server round
//!   cargo run -p bu-network --example network-show -- --speedtest  + ONE real speed test (uses real bandwidth, ~1 GB at gigabit)
//!   add --hide-ssid to print the Wi-Fi name as "<hidden>"

#[cfg(windows)]
fn main() {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use bu_network::gameservers::{eu_servers, round, GameServerEvent, Probe};
    use bu_network::ping::{ping_once, PingTarget};
    use bu_network::real::{CloudflareSpeed, WindowsNet};
    use bu_network::speedtest::{self, SpeedConfig, SpeedEvent};
    use bu_network::{NetworkOs, NetworkService};

    let args: Vec<String> = std::env::args().collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let real = Arc::new(WindowsNet::new());
    let os: Arc<dyn NetworkOs> = real.clone();
    let svc = NetworkService::new(os.clone());

    println!("elevated (admin): {}", os.is_elevated());
    println!("Wi-Fi radio (Windows.Devices.Radios): {:?}", real.wifi_radio());
    let t0 = Instant::now();
    let conn = match svc.connection_state() {
        Ok(c) => c,
        Err(e) => {
            println!("connection_state error: {e}");
            return;
        }
    };
    println!("connection_state read in {:.1} ms", t0.elapsed().as_secs_f64() * 1000.0);
    println!("offline: {}", conn.offline());
    for a in &conn.adapters {
        let in_use = conn.in_use.as_deref() == Some(a.id.as_str());
        println!("\n[{}] {:?}  {}{}", a.name, a.kind, a.description, if in_use { "   <- IN USE" } else { "" });
        println!("  id {}  if_index {}", a.id, a.if_index);
        println!("  switch on: {}   connected: {}   disabled in Device Manager: {}", a.enabled, a.connected, a.device_disabled);
        println!(
            "  link speed: {}",
            a.link_speed_bps.map(|b| format!("{} Mb/s", b / 1_000_000)).unwrap_or_else(|| "-".into())
        );
        if a.ssid.is_some() || a.signal_pct.is_some() {
            let ssid = if has("--hide-ssid") { a.ssid.as_ref().map(|_| "<hidden>".to_string()) } else { a.ssid.clone() };
            println!("  Wi-Fi: ssid {:?}  signal {:?} %", ssid, a.signal_pct);
        }
        println!("  gateways: {:?}", a.gateways);
        println!("  DNS in use: {:?}", a.dns_servers);
        println!("  DNS set by hand: {:?}", os.dns_servers(&a.id));
        println!("  device for the switch: {:?}", real.device_of(&a.id));
        println!(
            "  switch needs admin: {}",
            NetworkService::switch_action(a).needs_admin()
        );
    }
    match svc.dns_state() {
        Ok(d) => println!(
            "\nDNS header button: \"DNS {}\" on {} (configured {:?}, effective {:?})",
            d.current.label(),
            d.adapter_name,
            d.configured,
            d.effective
        ),
        Err(e) => println!("\nDNS header button: disabled ({e})"),
    }

    println!("\nPing pill (ICMP, 5 each):");
    for target in [PingTarget::Internet, PingTarget::Gateway] {
        let samples: Vec<String> = (0..5)
            .map(|_| {
                let s = ping_once(&os, &target, Duration::from_secs(1));
                std::thread::sleep(Duration::from_millis(200));
                match s.rtt {
                    Some(d) => format!("{:.2} ms {:?}", d.as_secs_f64() * 1000.0, s.level),
                    None => format!("lost ({:?})", s.error),
                }
            })
            .collect();
        println!("  {:?}: {}", target, samples.join(" | "));
    }

    if has("--games") {
        println!("\nGame servers (one round, {} tries per probe, lowest counts):", bu_network::gameservers::TRIES);
        let list = eu_servers();
        let t0 = Instant::now();
        round(os.as_ref(), &list, &mut |e| {
            if let GameServerEvent::Result(r) = e {
                let g = list.iter().find(|g| g.id == r.id).unwrap();
                let how = match r.method {
                    Some(Probe::Icmp) => "ICMP".to_string(),
                    Some(Probe::Udp(p)) => format!("UDP {p}"),
                    Some(Probe::Tcp(p)) => format!("TCP {p}"),
                    None => "-".into(),
                };
                println!(
                    "  {:<18} {:<40} {:<16} {:>9} {:<8} {:?}{}",
                    g.game,
                    format!("{}{}", g.sub_line(), if r.stand_in { " (≈ stand-in)" } else { "" }),
                    r.addr.map(|a| a.to_string()).unwrap_or_default(),
                    r.rtt.map(|d| format!("{:.1} ms", d.as_secs_f64() * 1000.0)).unwrap_or_else(|| "-".into()),
                    how,
                    r.level,
                    r.error.map(|e| format!("  ({e})")).unwrap_or_default()
                );
            }
        });
        println!("  round took {:.2} s", t0.elapsed().as_secs_f64());
    }

    if has("--wifi") {
        // Order 022: the nearby list Windows keeps (WlanGetAvailableNetworkList - a read; no scan is started)
        println!("
Wi-Fi networks (strongest first):");
        match svc.wifi_networks() {
            Ok(list) => {
                for w in list {
                    println!("  {:<32} {:>3} %  bars {}  secured {:<5}  saved {:<5}  connected {}", w.ssid, w.signal_pct, w.bars(), w.secured, w.saved, w.connected);
                }
            }
            Err(e) => println!("  error: {e}"),
        }
    }

    if has("--regions") {
        // Order 022: every game's regions, one round (1 try per probe, like the page's once-a-second pinger)
        for g in bu_network::gameregions::games() {
            println!("
{} ({}):", g.name, g.publisher);
            let list: Vec<_> = g.regions.iter().map(|r| r.server.clone()).filter(|s| !s.targets.is_empty()).collect();
            let t0 = Instant::now();
            let mut rows = Vec::new();
            bu_network::gameservers::round_until(os.as_ref(), &list, &mut |e| {
                if let GameServerEvent::Result(r) = e {
                    rows.push(r);
                }
            }, &AtomicBool::new(false));
            for reg in &g.regions {
                let r = rows.iter().find(|r| r.id == reg.server.id);
                let how = r.and_then(|r| r.method).map(|m| match m {
                    Probe::Icmp => "ICMP".to_string(),
                    Probe::Udp(p) => format!("UDP {p}"),
                    Probe::Tcp(p) => format!("TCP {p}"),
                });
                println!(
                    "  {:<20} {:<14} {:>9} {:<8} {}{}",
                    reg.server.region,
                    reg.place,
                    r.and_then(|r| r.rtt).map(|d| format!("{:.1} ms", d.as_secs_f64() * 1000.0)).unwrap_or_else(|| "-".into()),
                    how.unwrap_or_else(|| "-".into()),
                    r.and_then(|r| r.addr).map(|a| a.to_string()).unwrap_or_else(|| if reg.server.targets.is_empty() { "no address known".into() } else { String::new() }),
                    if r.is_some_and(|r| r.stand_in) { "  (≈ stand-in)" } else { "" }
                );
            }
            println!("  round took {:.2} s", t0.elapsed().as_secs_f64());
        }
    }

    if has("--latency") {
        // No bandwidth: 20 round trips each way to the speed test server, three methods side by side.
        println!("\nSpeed test latency methods (20 each, ms):");
        match CloudflareSpeed::new() {
            Ok(t) => {
                use bu_network::speedtest::{ping_and_jitter, SpeedTransport};
                let addr = t.server_addr();
                println!("  server address: {addr:?}");
                let show = |name: &str, f: &mut dyn FnMut() -> bu_network::Result<Duration>| {
                    let _ = f(); // warm-up, not counted (like the engine)
                    let v: Vec<f64> = (0..20)
                        .filter_map(|_| {
                            std::thread::sleep(Duration::from_millis(60));
                            f().ok().map(|d| d.as_secs_f64() * 1000.0)
                        })
                        .collect();
                    let (p, j) = ping_and_jitter(&v).unwrap_or((f64::NAN, f64::NAN));
                    let s: Vec<String> = v.iter().map(|x| format!("{x:.1}")).collect();
                    println!("  {name:<16} ping {p:>5.1}  jitter {j:>5.2}   [{}]", s.join(" "));
                };
                show("engine latency", &mut || t.latency());
                if let Ok(a) = addr {
                    show("TCP handshake", &mut || os.tcp_ping(a, Duration::from_secs(3)));
                }
                show("HTTP bytes=0", &mut || t.http_latency());
                if let Ok(a) = addr {
                    show("ICMP", &mut || os.icmp_ping(a.ip(), Duration::from_secs(1)));
                }
            }
            Err(e) => println!("  cannot start: {e}"),
        }
    }

    if has("--speedtest") {
        println!("\nSpeed test (Cloudflare, default config) - WARNING: uses real bandwidth, about 1 GB per run on a gigabit line:");
        let transport = match CloudflareSpeed::new() {
            Ok(t) => t,
            Err(e) => {
                println!("  cannot start: {e}");
                return;
            }
        };
        let mut last_tick = Instant::now();
        let t0 = Instant::now();
        let r = speedtest::run(&os, &transport, &SpeedConfig::default(), &AtomicBool::new(false), &mut |e| match e {
            SpeedEvent::Progress { phase, mbps, elapsed } => {
                if last_tick.elapsed() >= Duration::from_millis(1000) {
                    last_tick = Instant::now();
                    println!("  {:?} {:>5.1} s  live {:>7.1} Mb/s", phase, elapsed.as_secs_f64(), mbps);
                }
            }
            SpeedEvent::LatencySample { ms } => print!(" {ms:.1}"),
            other => println!("  {other:?}"),
        });
        println!();
        match r {
            Ok(r) => println!(
                "  RESULT  down {:.1} Mb/s  up {:.1} Mb/s  ping {:.1} ms  jitter {:.2} ms  data used: {:.1} MB down + {:.1} MB up  server {} ({})  total {:.1} s",
                r.download_mbps,
                r.upload_mbps,
                r.ping_ms,
                r.jitter_ms,
                r.bytes_down as f64 / 1e6,
                r.bytes_up as f64 / 1e6,
                r.server.city,
                r.server.code,
                t0.elapsed().as_secs_f64()
            ),
            Err(e) => println!("  FAILED: {e}"),
        }
    }
}

#[cfg(not(windows))]
fn main() {
    println!("Windows only.");
}
