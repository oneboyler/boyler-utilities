//! Order 022 additions, against the FAKE only: every adapter with its type, Wi-Fi networks (connect / password /
//! disconnect / forget), Custom DNS (IPv4 + IPv6), the per-game region pinger (Start ... Stop).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bu_network::fake::FakeNet;
use bu_network::gameregions::{self, Game};
use bu_network::gameservers::{GameServerEvent, GameServerSampler};
use bu_network::{classify_adapter, AdapterKind, DnsCurrent, DnsServers, NetError, NetworkService};

fn drawing() -> (Arc<FakeNet>, NetworkService) {
    let f = Arc::new(FakeNet::drawing());
    let s = NetworkService::new(f.clone());
    (f, s)
}

#[test]
fn every_adapter_with_its_type_in_the_drawings_order() {
    let (_f, s) = drawing();
    let c = s.connection_state().unwrap();
    let rows: Vec<(&str, &str)> = c.adapters.iter().map(|a| (a.name.as_str(), a.kind.badge())).collect();
    assert_eq!(
        rows,
        [
            ("Ethernet", "Ethernet"),
            ("Wi-Fi", "Wi-Fi"),
            ("Mullvad VPN", "VPN"),
            ("vEthernet (WSL)", "Virtual"),
            ("VirtualBox Host-Only", "Virtual"),
            ("Bluetooth Network", "Bluetooth")
        ]
    );
    // a VPN / virtual adapter never becomes "In use" (the physical card carrying the route is)
    assert_eq!(c.in_use.as_deref(), Some("{ETH-0001}"));
}

#[test]
fn non_physical_switches_need_admin_and_change_nothing_without_it() {
    let (f, s) = drawing();
    for id in ["{VPN-0001}", "{WSL-0001}", "{BT-0001}"] {
        assert_eq!(s.set_adapter(id, false), Err(NetError::NeedsAdmin));
    }
    assert!(f.log().is_empty());
    f.with(|st| st.elevated = true);
    let ch = s.set_adapter("{WSL-0001}", false).unwrap();
    assert_eq!(f.log(), ["adapter {WSL-0001} off"]);
    s.undo(ch.id).unwrap();
    assert_eq!(f.log(), ["adapter {WSL-0001} off", "adapter {WSL-0001} on"]);
}

#[test]
fn classify_real_interface_rows() {
    assert_eq!(classify_adapter(71, true, "Intel(R) Wi-Fi 6E AX210 160MHz"), Some(AdapterKind::Wifi));
    assert_eq!(classify_adapter(6, true, "Intel(R) Ethernet Controller I226-V"), Some(AdapterKind::Ethernet));
    assert_eq!(classify_adapter(6, false, "Hyper-V Virtual Ethernet Adapter"), Some(AdapterKind::Virtual));
    assert_eq!(classify_adapter(6, true, "VirtualBox Host-Only Ethernet Adapter"), Some(AdapterKind::Virtual));
    assert_eq!(classify_adapter(53, false, "WireGuard Tunnel"), Some(AdapterKind::Vpn));
    assert_eq!(classify_adapter(6, false, "TAP-Windows Adapter V9"), Some(AdapterKind::Vpn));
    assert_eq!(classify_adapter(6, true, "Bluetooth Device (Personal Area Network)"), Some(AdapterKind::Bluetooth));
    assert_eq!(classify_adapter(24, false, "Software Loopback Interface 1"), None);
    assert_eq!(classify_adapter(131, false, "Teredo Tunneling Pseudo-Interface"), None);
    assert_eq!(classify_adapter(23, false, "WAN Miniport (PPTP)"), None);
    assert_eq!(classify_adapter(6, false, "Microsoft Kernel Debug Network Adapter"), None);
}

#[test]
fn wifi_list_strongest_first_with_the_connected_one_on_top() {
    let (_f, s) = drawing();
    let n = s.wifi_networks().unwrap();
    let names: Vec<&str> = n.iter().map(|w| w.ssid.as_str()).collect();
    assert_eq!(names, ["MyHome", "MyHome_5G", "TP-Link_8F2C", "Vodafone-Guest", "DIRECT-7A-HP OfficeJet"]);
    let subs: Vec<&str> = n.iter().map(|w| w.sub_line()).collect();
    assert_eq!(subs, ["Connected · secured", "Saved", "Secured", "Open · not secured", "Secured"]);
    let bars: Vec<u8> = n.iter().map(|w| w.bars()).collect();
    assert_eq!(bars, [4, 3, 3, 2, 1]);
}

#[test]
fn wifi_connect_needs_a_password_only_for_new_secured_networks() {
    let (f, s) = drawing();
    // a new secured network without a password: nothing reaches Windows
    assert_eq!(s.wifi_connect("TP-Link_8F2C", None, true), Err(NetError::PasswordNeeded));
    assert_eq!(s.wifi_connect("TP-Link_8F2C", Some(""), true), Err(NetError::PasswordNeeded));
    assert!(f.log().is_empty());
    // a wrong password fails like Windows does and saves nothing
    assert!(matches!(s.wifi_connect("TP-Link_8F2C", Some("nope"), true), Err(NetError::AccessDenied(_))));
    assert!(!s.wifi_networks().unwrap().iter().any(|w| w.ssid == "TP-Link_8F2C" && w.saved));
    s.wifi_connect("TP-Link_8F2C", Some("correct horse"), false).unwrap();
    let n = s.wifi_networks().unwrap();
    assert_eq!(n[0].ssid, "TP-Link_8F2C");
    assert!(n[0].connected && n[0].saved);
    // a saved network connects without a password (none is sent); an open one too
    s.wifi_connect("MyHome", None, true).unwrap();
    s.wifi_connect("Vodafone-Guest", None, true).unwrap();
    assert_eq!(
        f.log()[1..],
        ["wifi_connect TP-Link_8F2C auto=false +password", "wifi_connect MyHome auto=true", "wifi_connect Vodafone-Guest auto=true"]
    );
}

#[test]
fn wifi_disconnect_and_forget() {
    let (f, s) = drawing();
    s.wifi_disconnect().unwrap();
    assert!(!s.wifi_networks().unwrap().iter().any(|w| w.connected));
    s.wifi_forget("MyHome_5G").unwrap();
    let n = s.wifi_networks().unwrap();
    assert!(!n.iter().find(|w| w.ssid == "MyHome_5G").unwrap().saved);
    // an unsaved network can't be forgotten
    assert!(s.wifi_forget("Vodafone-Guest").is_err());
    assert_eq!(f.log(), ["wifi_disconnect", "wifi_forget MyHome_5G"]);
}

#[test]
fn wifi_radio_off_lists_nothing() {
    let (f, s) = drawing();
    s.set_adapter("{WIFI-0001}", false).unwrap();
    assert!(s.wifi_networks().unwrap().is_empty());
    assert_eq!(f.log(), ["wifi_radio off"]);
}

#[test]
fn custom_dns_fields_are_checked_one_by_one() {
    assert_eq!(
        DnsServers::from_fields("9.9.9.9", "149.112.112.112", "2620:fe::fe", "2620:fe::9").unwrap(),
        DnsServers {
            v4: vec!["9.9.9.9".parse().unwrap(), "149.112.112.112".parse().unwrap()],
            v6: vec!["2620:fe::fe".parse().unwrap(), "2620:fe::9".parse().unwrap()]
        }
    );
    // IPv6 and the IPv4 secondary may stay empty; IPv4 primary is needed
    assert_eq!(DnsServers::from_fields(" 9.9.9.9 ", "", "", "").unwrap().v6.len(), 0);
    assert_eq!(DnsServers::from_fields("", "", "", ""), Err([true, false, false, false]));
    assert_eq!(DnsServers::from_fields("9.9.9", "1.1.1.1x", "zz::1", "2620:fe::9"), Err([true, true, true, false]));
    assert_eq!(DnsServers::from_fields("9.9.9.9", "", "9.9.9.9", ""), Err([false, false, true, false]));
}

#[test]
fn custom_dns_needs_admin_then_writes_both_families_and_undoes() {
    let (f, s) = drawing();
    let servers = DnsServers::from_fields("9.9.9.9", "149.112.112.112", "2620:fe::fe", "2620:fe::9").unwrap();
    assert_eq!(s.set_dns_custom(servers.clone()), Err(NetError::NeedsAdmin));
    assert!(f.log().is_empty());
    f.with(|st| st.elevated = true);
    let ch = s.set_dns_custom(servers).unwrap();
    assert_eq!(ch.toast(), "DNS: 9.9.9.9 · 149.112.112.112 · IPv6 set on Ethernet");
    assert_eq!(s.dns_state().unwrap().current, DnsCurrent::Custom);
    assert_eq!(f.log(), ["dns {ETH-0001} v4=[9.9.9.9,149.112.112.112] v6=[2620:fe::fe,2620:fe::9]", "flush"]);
    s.undo(ch.id).unwrap();
    assert_eq!(s.dns_state().unwrap().current, DnsCurrent::Automatic);
    // IPv4 only: the toast says nothing about IPv6
    let ch = s.set_dns_custom(DnsServers::from_fields("9.9.9.9", "", "", "").unwrap()).unwrap();
    assert_eq!(ch.toast(), "DNS: 9.9.9.9 on Ethernet");
    assert!(s.set_dns_custom(DnsServers::default()).is_err());
}

fn val() -> Game {
    gameregions::games().into_iter().find(|g| g.id == "val").unwrap()
}

#[test]
fn region_pinger_runs_once_a_second_until_stopped_and_picks_the_best() {
    let f = Arc::new(FakeNet::drawing());
    let g = val();
    // answers for the stand-ins: AWS beacons by name -> address, UDP 7770 answers; Valve Warsaw by ICMP
    f.with(|st| {
        for (i, r) in g.regions.iter().enumerate() {
            for t in &r.server.targets {
                let ip: std::net::IpAddr = format!("10.0.0.{}", i + 1).parse().unwrap();
                if t.host.starts_with("gamelift") {
                    st.names.insert(t.host.clone(), vec![ip]);
                    st.udp.insert(std::net::SocketAddr::new(ip, 7770), Duration::from_millis(20 + i as u64 * 5));
                } else if let Ok(ip) = t.host.parse() {
                    st.icmp.insert(ip, Duration::from_millis(26));
                }
            }
        }
    });
    let ev = Arc::new(Mutex::new(Vec::new()));
    let e2 = ev.clone();
    let servers: Vec<_> = g.regions.iter().map(|r| r.server.clone()).collect();
    let before = f.reads();
    let p = GameServerSampler::start_with_tries(f.clone(), servers, Some(Duration::from_millis(1000)), 1, move |e| e2.lock().unwrap().push(e));
    std::thread::sleep(Duration::from_millis(2300));
    p.stop();
    let ev = ev.lock().unwrap().clone();
    let rounds = ev.iter().filter(|e| **e == GameServerEvent::RoundDone).count();
    assert!((2..=3).contains(&rounds), "rounds {rounds}");
    let after_stop = f.reads();
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(f.reads(), after_stop, "a stopped pinger sends nothing");
    assert!(after_stop > before);
    // the first round's numbers: Frankfurt 20 ms (best), Istanbul nothing
    let first: Vec<_> = ev
        .iter()
        .take_while(|e| **e != GameServerEvent::RoundDone)
        .filter_map(|e| match e {
            GameServerEvent::Result(r) => Some((r.id.clone(), r.rtt.map(|d| d.as_millis() as u32))),
            _ => None,
        })
        .collect();
    assert_eq!(first.len(), val().regions.len(), "every region reports once a round");
    let best = gameregions::best(first.iter().map(|(id, ms)| (id.as_str(), *ms)));
    assert_eq!(best, Some("val.0"));
    assert_eq!(first.iter().find(|(id, _)| id == "val.6").unwrap().1, None);
}

#[test]
fn nothing_is_pinged_until_start() {
    // building the game list and the services sends nothing (opening the tab = no pings)
    let f = Arc::new(FakeNet::drawing());
    let _s = NetworkService::new(f.clone());
    let _g = gameregions::games();
    assert_eq!(f.reads(), 0);
}
