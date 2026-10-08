//! A fake PC for tests (and for building the menu without touching Windows). It keeps adapters, DNS settings, ping
//! answers in memory and logs every change call, so tests can prove that refused changes never reached the OS.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::Duration;

use crate::error::{NetError, Result};
use crate::model::{Adapter, AdapterKind, DnsServers, WifiNetwork};
use crate::os::NetworkOs;

#[derive(Debug, Default)]
pub struct FakeState {
    pub adapters: Vec<Adapter>,
    /// Interface index with the internet route (None = offline). Recomputed by the fake when an adapter goes off.
    pub internet_if: Option<u32>,
    pub elevated: bool,
    pub has_wifi_radio: bool,
    /// Hand-set DNS per adapter id.
    pub dns: HashMap<String, DnsServers>,
    /// ICMP answers per address; missing = timeout.
    pub icmp: HashMap<IpAddr, Duration>,
    /// TCP connect answers per address; missing = timeout.
    pub tcp: HashMap<SocketAddr, Duration>,
    /// Every ping / connect / UDP echo takes this long (to test stopping in the middle of a slow round).
    pub probe_delay: Duration,
    /// UDP echo answers per address; missing = timeout.
    pub udp: HashMap<SocketAddr, Duration>,
    /// Name -> addresses; missing = lookup error.
    pub names: HashMap<String, Vec<IpAddr>>,
    /// Make the next change call fail with this error (once).
    pub fail_next_change: Option<NetError>,
    /// Every change call, in order: "wifi_radio on", "adapter {id} off", "dns {id} ...", "flush".
    pub log: Vec<String>,
    /// Number of read calls of any kind (pings included) - proves a stopped sampler costs nothing.
    pub reads: u64,
    /// Nearby Wi-Fi networks (Windows' list).
    pub wifi: Vec<WifiNetwork>,
    /// The password a secured network accepts (ssid -> password); a wrong one fails like Windows does.
    pub wifi_passwords: HashMap<String, String>,
}

#[derive(Debug, Default)]
pub struct FakeNet {
    pub state: Mutex<FakeState>,
}

impl FakeNet {
    /// Ethernet (in use, 1 Gb/s, DHCP DNS) + Wi-Fi (radio on, not connected). Not elevated.
    pub fn typical() -> FakeNet {
        let eth = Adapter {
            id: "{ETH-0001}".into(),
            if_index: 12,
            name: "Ethernet".into(),
            description: "Fake Ethernet Controller".into(),
            kind: AdapterKind::Ethernet,
            enabled: true,
            device_disabled: false,
            connected: true,
            link_speed_bps: Some(1_000_000_000),
            ssid: None,
            signal_pct: None,
            gateways: vec!["192.168.1.1".parse().unwrap()],
            dns_servers: vec!["192.168.1.1".parse().unwrap()],
        };
        let wifi = Adapter {
            id: "{WIFI-0001}".into(),
            if_index: 17,
            name: "Wi-Fi".into(),
            description: "Fake Wi-Fi 6 Adapter".into(),
            kind: AdapterKind::Wifi,
            enabled: true,
            device_disabled: false,
            connected: false,
            link_speed_bps: None,
            ssid: None,
            signal_pct: None,
            gateways: vec![],
            dns_servers: vec![],
        };
        let mut icmp = HashMap::new();
        icmp.insert("192.168.1.1".parse().unwrap(), Duration::from_millis(1));
        icmp.insert("1.1.1.1".parse().unwrap(), Duration::from_millis(19));
        FakeNet {
            state: Mutex::new(FakeState {
                adapters: vec![wifi, eth],
                internet_if: Some(12),
                elevated: false,
                has_wifi_radio: true,
                icmp,
                ..Default::default()
            }),
        }
    }

    /// The drawing's PC (menu-v22 NW + WIFI lists): Ethernet in use (2.5 Gb/s), Wi-Fi connected to "MyHome", a VPN, the WSL
    /// switch, VirtualBox's host-only adapter (off) and Bluetooth PAN (off); five nearby networks. Not elevated.
    pub fn drawing() -> FakeNet {
        let mk = |id: &str, idx: u32, name: &str, desc: &str, kind: AdapterKind, on: bool, conn: bool, gw: bool| Adapter {
            id: id.into(),
            if_index: idx,
            name: name.into(),
            description: desc.into(),
            kind,
            enabled: on,
            device_disabled: !on,
            connected: conn,
            link_speed_bps: conn.then_some(if kind == AdapterKind::Ethernet { 2_500_000_000 } else { 1_200_000_000 }),
            ssid: (kind == AdapterKind::Wifi && conn).then(|| "MyHome".to_string()),
            signal_pct: (kind == AdapterKind::Wifi && conn).then_some(92),
            gateways: if gw { vec!["192.168.1.1".parse().unwrap()] } else { vec![] },
            dns_servers: if gw { vec!["192.168.1.1".parse().unwrap()] } else { vec![] },
        };
        let adapters = vec![
            mk("{ETH-0001}", 12, "Ethernet", "Intel(R) Ethernet Controller I226-V", AdapterKind::Ethernet, true, true, true),
            mk("{WIFI-0001}", 17, "Wi-Fi", "Intel(R) Wi-Fi 6E AX210 160MHz", AdapterKind::Wifi, true, true, true),
            mk("{VPN-0001}", 31, "Mullvad VPN", "Connected · Sweden · WireGuard", AdapterKind::Vpn, true, true, false),
            mk("{WSL-0001}", 40, "vEthernet (WSL)", "Hyper-V · used by WSL", AdapterKind::Virtual, true, true, false),
            mk("{VBOX-0001}", 0, "VirtualBox Host-Only", "VirtualBox", AdapterKind::Virtual, false, false, false),
            mk("{BT-0001}", 0, "Bluetooth Network", "Personal area network", AdapterKind::Bluetooth, false, false, false),
        ];
        let w = |n: &str, sig: u8, secured: bool, saved: bool, connected: bool| WifiNetwork { ssid: n.into(), signal_pct: sig, secured, saved, connected };
        let mut icmp = HashMap::new();
        icmp.insert("1.1.1.1".parse().unwrap(), Duration::from_millis(12));
        let mut pw = HashMap::new();
        pw.insert("TP-Link_8F2C".to_string(), "correct horse".to_string());
        FakeNet {
            state: Mutex::new(FakeState {
                adapters,
                internet_if: Some(12),
                has_wifi_radio: true,
                icmp,
                wifi: vec![
                    w("MyHome", 92, true, true, true),
                    w("MyHome_5G", 70, true, true, false),
                    w("TP-Link_8F2C", 64, true, false, false),
                    w("Vodafone-Guest", 40, false, false, false),
                    w("DIRECT-7A-HP OfficeJet", 18, true, false, false),
                ],
                wifi_passwords: pw,
                ..Default::default()
            }),
        }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut FakeState) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    pub fn log(&self) -> Vec<String> {
        self.with(|s| s.log.clone())
    }

    pub fn reads(&self) -> u64 {
        self.with(|s| s.reads)
    }

    fn change(&self, s: &mut FakeState, what: String) -> Result<()> {
        s.log.push(what);
        match s.fail_next_change.take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// When the adapter carrying the route goes off, the route moves to another connected+enabled adapter (or none).
fn recompute_route(s: &mut FakeState) {
    let cur_ok = s
        .internet_if
        .map(|i| s.adapters.iter().any(|a| a.if_index == i && a.enabled && a.connected))
        .unwrap_or(false);
    if !cur_ok {
        s.internet_if = s
            .adapters
            .iter()
            .find(|a| a.enabled && a.connected && !a.gateways.is_empty())
            .map(|a| a.if_index);
    }
}

impl NetworkOs for FakeNet {
    fn adapters(&self) -> Result<Vec<Adapter>> {
        self.with(|s| {
            s.reads += 1;
            Ok(s.adapters.clone())
        })
    }

    fn internet_if_index(&self) -> Result<Option<u32>> {
        self.with(|s| {
            s.reads += 1;
            Ok(s.internet_if)
        })
    }

    fn is_elevated(&self) -> bool {
        self.with(|s| s.elevated)
    }

    fn set_wifi_radio(&self, on: bool) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        // Like Windows: a Wi-Fi card disabled in Device Manager has no radio.
        if !s.has_wifi_radio || s.adapters.iter().any(|a| a.kind == AdapterKind::Wifi && a.device_disabled) {
            return Err(NetError::NoWifiRadio);
        }
        self.change(&mut s, format!("wifi_radio {}", if on { "on" } else { "off" }))?;
        for a in s.adapters.iter_mut().filter(|a| a.kind == AdapterKind::Wifi) {
            a.enabled = on;
            if !on {
                a.connected = false;
            }
        }
        recompute_route(&mut s);
        Ok(())
    }

    fn set_adapter_enabled(&self, id: &str, on: bool) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        if !s.elevated {
            // The real Windows call fails like this without admin; the service must never get here.
            return Err(NetError::AccessDenied("fake: not elevated".into()));
        }
        if !s.adapters.iter().any(|a| a.id == id) {
            return Err(NetError::NoSuchAdapter(id.into()));
        }
        self.change(&mut s, format!("adapter {id} {}", if on { "on" } else { "off" }))?;
        let a = s.adapters.iter_mut().find(|a| a.id == id).unwrap();
        a.enabled = on;
        a.device_disabled = !on;
        a.connected = on && !a.gateways.is_empty();
        recompute_route(&mut s);
        Ok(())
    }

    fn icmp_ping(&self, ip: IpAddr, _timeout: Duration) -> Result<Duration> {
        self.delay();
        self.with(|s| {
            s.reads += 1;
            if s.internet_if.is_none() {
                return Err(NetError::Unreachable(ip.to_string()));
            }
            s.icmp.get(&ip).copied().ok_or(NetError::Timeout)
        })
    }

    fn tcp_ping(&self, addr: SocketAddr, _timeout: Duration) -> Result<Duration> {
        self.delay();
        self.with(|s| {
            s.reads += 1;
            if s.internet_if.is_none() {
                return Err(NetError::Unreachable(addr.to_string()));
            }
            s.tcp.get(&addr).copied().ok_or(NetError::Timeout)
        })
    }

    fn udp_ping(&self, addr: SocketAddr, _timeout: Duration) -> Result<Duration> {
        self.delay();
        self.with(|s| {
            s.reads += 1;
            if s.internet_if.is_none() {
                return Err(NetError::Unreachable(addr.to_string()));
            }
            s.udp.get(&addr).copied().ok_or(NetError::Timeout)
        })
    }

    fn resolve(&self, host: &str) -> Result<Vec<IpAddr>> {
        self.with(|s| {
            s.reads += 1;
            if let Ok(ip) = host.parse::<IpAddr>() {
                return Ok(vec![ip]);
            }
            s.names.get(host).cloned().ok_or_else(|| NetError::Resolve(host.into()))
        })
    }

    fn flush_dns(&self) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        self.change(&mut s, "flush".into())
    }

    fn dns_servers(&self, id: &str) -> Result<DnsServers> {
        self.with(|s| {
            s.reads += 1;
            if !s.adapters.iter().any(|a| a.id == id) {
                return Err(NetError::NoSuchAdapter(id.into()));
            }
            Ok(s.dns.get(id).cloned().unwrap_or_default())
        })
    }

    fn wifi_networks(&self) -> Result<Vec<WifiNetwork>> {
        self.with(|s| {
            s.reads += 1;
            if !s.adapters.iter().any(|a| a.kind == AdapterKind::Wifi && a.enabled) {
                return Ok(vec![]);
            }
            Ok(s.wifi.clone())
        })
    }

    fn wifi_connect(&self, ssid: &str, password: Option<&str>, auto: bool) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        let Some(i) = s.wifi.iter().position(|n| n.ssid == ssid) else {
            return Err(NetError::Unreachable(ssid.into()));
        };
        if s.wifi[i].secured && !s.wifi[i].saved {
            let ok = match (password, s.wifi_passwords.get(ssid)) {
                (Some(p), Some(want)) => p == want,
                (Some(_), None) => true,
                (None, _) => false,
            };
            if !ok {
                s.log.push(format!("wifi_connect {ssid} refused"));
                return Err(NetError::AccessDenied("wrong password".into()));
            }
        }
        self.change(&mut s, format!("wifi_connect {ssid} auto={auto}{}", if password.is_some() { " +password" } else { "" }))?;
        for n in s.wifi.iter_mut() {
            n.connected = false;
        }
        s.wifi[i].connected = true;
        s.wifi[i].saved = true;
        Ok(())
    }

    fn wifi_disconnect(&self) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        self.change(&mut s, "wifi_disconnect".into())?;
        for n in s.wifi.iter_mut() {
            n.connected = false;
        }
        Ok(())
    }

    fn wifi_forget(&self, ssid: &str) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        if !s.wifi.iter().any(|n| n.ssid == ssid && n.saved) {
            return Err(NetError::Unreachable(ssid.into()));
        }
        self.change(&mut s, format!("wifi_forget {ssid}"))?;
        if let Some(n) = s.wifi.iter_mut().find(|n| n.ssid == ssid) {
            n.saved = false;
            n.connected = false;
        }
        Ok(())
    }

    fn set_dns_servers(&self, id: &str, servers: &DnsServers) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        if !s.elevated {
            return Err(NetError::AccessDenied("fake: not elevated".into()));
        }
        if !s.adapters.iter().any(|a| a.id == id) {
            return Err(NetError::NoSuchAdapter(id.into()));
        }
        let v4: Vec<String> = servers.v4.iter().map(|x| x.to_string()).collect();
        let v6: Vec<String> = servers.v6.iter().map(|x| x.to_string()).collect();
        self.change(&mut s, format!("dns {id} v4=[{}] v6=[{}]", v4.join(","), v6.join(",")))?;
        if servers.is_automatic() {
            s.dns.remove(id);
        } else {
            s.dns.insert(id.into(), servers.clone());
        }
        Ok(())
    }
}

impl FakeNet {
    fn delay(&self) {
        let d = self.with(|s| s.probe_delay);
        if !d.is_zero() {
            std::thread::sleep(d);
        }
    }
}

/// A fake speed-test server for the menu's test copies (and tests): a fixed total speed shared by the streams of
/// [`crate::speedtest::SpeedConfig`], delivered by elapsed time; latency samples around `ping_ms`. Sends nothing.
#[derive(Debug, Clone)]
pub struct FakeSpeed {
    /// bytes / s per download stream
    down_rate: f64,
    up_rate: f64,
    ping_ms: f64,
    n: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl FakeSpeed {
    /// The drawing's numbers: about 920 down / 108 up (Mb/s), ping 8-10 ms, for the default 6 / 4 streams.
    pub fn drawing() -> FakeSpeed {
        FakeSpeed::new(920.0, 108.0, 8.0, &crate::speedtest::SpeedConfig::default())
    }

    pub fn new(down_mbps: f64, up_mbps: f64, ping_ms: f64, cfg: &crate::speedtest::SpeedConfig) -> FakeSpeed {
        FakeSpeed {
            down_rate: down_mbps * 1e6 / 8.0 / cfg.download_streams.max(1) as f64,
            up_rate: up_mbps * 1e6 / 8.0 / cfg.upload_streams.max(1) as f64,
            ping_ms,
            n: Default::default(),
        }
    }

    fn pump(rate: f64, bytes: u64, on: &mut dyn FnMut(u64) -> bool) {
        let t0 = std::time::Instant::now();
        let mut sent = 0u64;
        while sent < bytes {
            std::thread::sleep(Duration::from_millis(5));
            let due = ((t0.elapsed().as_secs_f64() * rate) as u64).min(bytes);
            let n = due - sent;
            sent = due;
            if n > 0 && !on(n) {
                return;
            }
        }
    }
}

impl crate::speedtest::SpeedTransport for FakeSpeed {
    fn server(&self) -> Result<crate::speedtest::ServerInfo> {
        Ok(crate::speedtest::ServerInfo { city: "Zagreb".into(), code: "ZAG".into(), provider: "Fake".into() })
    }
    fn download(&self, bytes: u64, on: &mut dyn FnMut(u64) -> bool) -> Result<()> {
        Self::pump(self.down_rate, bytes, on);
        Ok(())
    }
    fn upload(&self, bytes: u64, on: &mut dyn FnMut(u64) -> bool) -> Result<()> {
        Self::pump(self.up_rate, bytes, on);
        Ok(())
    }
    fn latency(&self) -> Result<Duration> {
        // 8, 9, 8, 10, 8 ... (a small made-up jitter)
        let i = self.n.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let extra = [0.0, 1.0, 0.0, 2.0, 0.0, 1.0][(i % 6) as usize];
        std::thread::sleep(Duration::from_secs_f64((self.ping_ms + extra) / 1000.0));
        Ok(Duration::from_secs_f64((self.ping_ms + extra) / 1000.0))
    }
}
