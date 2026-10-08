//! Connection rows, switches, Flush DNS, DNS switcher, undo, admin path - all against the fake PC.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use bu_network::fake::FakeNet;
use bu_network::*;

fn setup() -> (Arc<FakeNet>, NetworkService) {
    let fake = Arc::new(FakeNet::typical());
    let svc = NetworkService::new(fake.clone());
    (fake, svc)
}

const ETH: &str = "{ETH-0001}";
const WIFI: &str = "{WIFI-0001}";

#[test]
fn reads_adapters_ethernet_first_and_which_is_in_use() {
    // v21 drawing order (Order 022): Ethernet, Wi-Fi, VPN, virtual, Bluetooth (was Wi-Fi first)
    let (_f, svc) = setup();
    let c = svc.connection_state().unwrap();
    assert_eq!(c.adapters.len(), 2);
    assert_eq!(c.adapters[0].kind, AdapterKind::Ethernet);
    assert_eq!(c.adapters[1].kind, AdapterKind::Wifi);
    assert_eq!(c.in_use.as_deref(), Some(ETH));
    assert!(!c.offline());
    let eth = c.in_use_adapter().unwrap();
    assert_eq!(eth.link_speed_bps, Some(1_000_000_000));
    assert_eq!(eth.name, "Ethernet");
}

#[test]
fn offline_when_no_route() {
    let (f, svc) = setup();
    f.with(|s| s.internet_if = None);
    let c = svc.connection_state().unwrap();
    assert!(c.offline());
    assert!(c.in_use_adapter().is_none());
}

#[test]
fn route_over_a_virtual_adapter_falls_back_to_the_connected_physical_one() {
    let (f, svc) = setup();
    f.with(|s| s.internet_if = Some(99)); // e.g. a VPN / Hyper-V vEthernet, not in the list
    assert_eq!(svc.connection_state().unwrap().in_use.as_deref(), Some(ETH));
}

#[test]
fn admin_matrix() {
    assert!(!Action::SwitchWifi.needs_admin());
    assert!(Action::SwitchDevice.needs_admin());
    assert!(!Action::FlushDns.needs_admin());
    assert!(Action::SetDns.needs_admin());
    let (_f, svc) = setup();
    let c = svc.connection_state().unwrap();
    assert_eq!(NetworkService::switch_action(c.adapter(WIFI).unwrap()), Action::SwitchWifi);
    assert_eq!(NetworkService::switch_action(c.adapter(ETH).unwrap()), Action::SwitchDevice);
}

#[test]
fn wifi_switch_needs_no_admin_and_undoes() {
    let (f, svc) = setup();
    let ch = svc.set_adapter(WIFI, false).unwrap();
    assert_eq!(ch.kind, ChangeKind::WifiRadio { was_on: true, now_on: false });
    assert_eq!(ch.toast(), "Wi-Fi off");
    assert!(!svc.connection_state().unwrap().adapter(WIFI).unwrap().enabled);
    let back = svc.undo(ch.id).unwrap();
    assert_eq!(back.kind, ChangeKind::WifiRadio { was_on: false, now_on: true });
    assert!(svc.connection_state().unwrap().adapter(WIFI).unwrap().enabled);
    assert_eq!(f.log(), vec!["wifi_radio off", "wifi_radio on"]);
    assert!(svc.changes().is_empty());
}

/// Seen on the real test PC (examples/show, 2026-10-08): the Wi-Fi card is disabled in Device Manager, so Windows has no
/// Wi-Fi radio. Its switch must then go through the device (admin), and undo puts the device back off.
#[test]
fn wifi_card_disabled_in_device_manager_switches_through_the_device() {
    let (f, svc) = setup();
    f.with(|s| {
        let w = s.adapters.iter_mut().find(|a| a.id == WIFI).unwrap();
        w.enabled = false;
        w.device_disabled = true;
    });
    let c = svc.connection_state().unwrap();
    assert_eq!(NetworkService::switch_action(c.adapter(WIFI).unwrap()), Action::SwitchDevice);
    assert_eq!(svc.set_adapter(WIFI, true), Err(NetError::NeedsAdmin));
    assert!(f.log().is_empty());
    f.with(|s| s.elevated = true);
    let ch = svc.set_adapter(WIFI, true).unwrap();
    assert_eq!(ch.kind, ChangeKind::Adapter { id: WIFI.into(), name: "Wi-Fi".into(), was_on: false, now_on: true });
    let c = svc.connection_state().unwrap();
    assert!(c.adapter(WIFI).unwrap().enabled);
    // Device on again: now the row is a plain radio switch.
    assert_eq!(NetworkService::switch_action(c.adapter(WIFI).unwrap()), Action::SwitchWifi);
    svc.undo(ch.id).unwrap();
    assert!(svc.connection_state().unwrap().adapter(WIFI).unwrap().device_disabled);
    assert_eq!(f.log(), vec![format!("adapter {WIFI} on"), format!("adapter {WIFI} off")]);
}

/// REVIEW 008 7becaeb remark: Wi-Fi radio switched off (no admin), then the card is disabled in Device Manager. Undo is
/// now a device switch, so it needs admin - refused before Windows, the change stays undoable.
#[test]
fn wifi_undo_after_the_card_was_disabled_meanwhile_needs_admin() {
    let (f, svc) = setup();
    let ch = svc.set_adapter(WIFI, false).unwrap();
    assert_eq!(ch.kind, ChangeKind::WifiRadio { was_on: true, now_on: false });
    f.with(|s| s.adapters.iter_mut().find(|a| a.id == WIFI).unwrap().device_disabled = true);
    assert_eq!(svc.undo(ch.id), Err(NetError::NeedsAdmin));
    assert_eq!(f.log(), vec!["wifi_radio off"], "the refused undo never reached Windows");
    assert_eq!(svc.changes().len(), 1, "still undoable");
    f.with(|s| s.elevated = true);
    svc.undo(ch.id).unwrap();
    assert_eq!(f.log(), vec!["wifi_radio off".to_string(), format!("adapter {WIFI} on")]);
    assert!(!svc.connection_state().unwrap().adapter(WIFI).unwrap().device_disabled);
    assert!(svc.changes().is_empty());
}

#[test]
fn no_wifi_radio_is_a_typed_error() {
    let (f, svc) = setup();
    f.with(|s| s.has_wifi_radio = false);
    assert_eq!(svc.set_adapter(WIFI, false), Err(NetError::NoWifiRadio));
}

#[test]
fn ethernet_switch_without_admin_is_refused_before_windows() {
    let (f, svc) = setup();
    assert_eq!(svc.set_adapter(ETH, false), Err(NetError::NeedsAdmin));
    assert!(f.log().is_empty(), "nothing reached the OS");
    assert!(svc.changes().is_empty());
    assert!(svc.connection_state().unwrap().adapter(ETH).unwrap().enabled);
}

#[test]
fn ethernet_switch_with_admin_goes_offline_and_undoes() {
    let (f, svc) = setup();
    f.with(|s| s.elevated = true);
    let ch = svc.set_adapter(ETH, false).unwrap();
    assert_eq!(ch.toast(), "Ethernet off");
    let c = svc.connection_state().unwrap();
    assert!(!c.adapter(ETH).unwrap().enabled);
    assert!(c.offline(), "Wi-Fi isn't connected, so nothing carries the route");
    svc.undo(ch.id).unwrap();
    let c = svc.connection_state().unwrap();
    assert!(c.adapter(ETH).unwrap().enabled);
    assert_eq!(c.in_use.as_deref(), Some(ETH));
    assert_eq!(f.log(), vec![format!("adapter {ETH} off"), format!("adapter {ETH} on")]);
}

#[test]
fn ethernet_undo_needs_admin_too() {
    let (f, svc) = setup();
    f.with(|s| s.elevated = true);
    let ch = svc.set_adapter(ETH, false).unwrap();
    f.with(|s| s.elevated = false);
    assert_eq!(svc.undo(ch.id), Err(NetError::NeedsAdmin));
    assert_eq!(svc.changes().len(), 1, "still undoable later");
}

#[test]
fn switching_to_the_same_state_touches_nothing() {
    let (f, svc) = setup();
    let ch = svc.set_adapter(WIFI, true).unwrap();
    assert_eq!(ch.kind, ChangeKind::WifiRadio { was_on: true, now_on: true });
    assert!(f.log().is_empty());
}

#[test]
fn unknown_adapter() {
    let (_f, svc) = setup();
    assert_eq!(svc.set_adapter("{NOPE}", true), Err(NetError::NoSuchAdapter("{NOPE}".into())));
}

#[test]
fn os_failure_is_returned_and_not_remembered() {
    let (f, svc) = setup();
    f.with(|s| s.fail_next_change = Some(NetError::AccessDenied("radio".into())));
    assert_eq!(svc.set_adapter(WIFI, false), Err(NetError::AccessDenied("radio".into())));
    assert!(svc.changes().is_empty());
}

#[test]
fn flush_dns_needs_no_admin() {
    let (f, svc) = setup();
    svc.flush_dns().unwrap();
    assert_eq!(f.log(), vec!["flush"]);
}

#[test]
fn dns_reads_automatic_by_default() {
    let (_f, svc) = setup();
    let d = svc.dns_state().unwrap();
    assert_eq!(d.adapter, ETH);
    assert_eq!(d.current, DnsCurrent::Automatic);
    assert_eq!(d.current.label(), "Automatic");
    assert_eq!(d.current.choice(), Some(DnsChoice::Automatic));
    assert_eq!(d.effective, vec![IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))]);
}

#[test]
fn dns_set_without_admin_is_refused_before_windows() {
    let (f, svc) = setup();
    assert_eq!(svc.set_dns(DnsChoice::Cloudflare), Err(NetError::NeedsAdmin));
    assert!(f.log().is_empty());
}

#[test]
fn dns_set_cloudflare_writes_v4_and_v6_flushes_and_undoes() {
    let (f, svc) = setup();
    f.with(|s| s.elevated = true);
    let ch = svc.set_dns(DnsChoice::Cloudflare).unwrap();
    assert_eq!(ch.toast(), "DNS: Cloudflare (1.1.1.1) on Ethernet");
    let d = svc.dns_state().unwrap();
    assert_eq!(d.current, DnsCurrent::Preset { choice: DnsChoice::Cloudflare, ipv6: true });
    assert_eq!(d.current.label(), "Cloudflare");
    assert_eq!(
        f.log(),
        vec![
            format!("dns {ETH} v4=[1.1.1.1,1.0.0.1] v6=[2606:4700:4700::1111,2606:4700:4700::1001]"),
            "flush".to_string()
        ]
    );
    let back = svc.undo(ch.id).unwrap();
    assert_eq!(back.toast(), "DNS back to automatic · from your router");
    assert_eq!(svc.dns_state().unwrap().current, DnsCurrent::Automatic);
    assert_eq!(f.log()[2], format!("dns {ETH} v4=[] v6=[]"));
    assert_eq!(f.log()[3], "flush");
}

#[test]
fn dns_google_then_automatic() {
    let (f, svc) = setup();
    f.with(|s| s.elevated = true);
    svc.set_dns(DnsChoice::Google).unwrap();
    assert_eq!(
        f.log()[0],
        format!("dns {ETH} v4=[8.8.8.8,8.8.4.4] v6=[2001:4860:4860::8888,2001:4860:4860::8844]")
    );
    let ch = svc.set_dns(DnsChoice::Automatic).unwrap();
    assert_eq!(ch.toast(), "DNS back to automatic · from your router");
    // Undo of "automatic" puts Google back exactly.
    svc.undo(ch.id).unwrap();
    assert_eq!(svc.dns_state().unwrap().current, DnsCurrent::Preset { choice: DnsChoice::Google, ipv6: true });
}

#[test]
fn dns_custom_servers_are_kept_by_undo() {
    let (f, svc) = setup();
    f.with(|s| {
        s.elevated = true;
        s.dns.insert(ETH.into(), DnsServers { v4: vec![Ipv4Addr::new(9, 9, 9, 9)], v6: vec![] });
    });
    assert_eq!(svc.dns_state().unwrap().current, DnsCurrent::Custom);
    assert_eq!(svc.dns_state().unwrap().current.choice(), None);
    let ch = svc.set_dns(DnsChoice::Cloudflare).unwrap();
    svc.undo(ch.id).unwrap();
    assert_eq!(f.with(|s| s.dns.get(ETH).cloned()).unwrap().v4, vec![Ipv4Addr::new(9, 9, 9, 9)]);
}

#[test]
fn dns_same_choice_writes_nothing() {
    let (f, svc) = setup();
    f.with(|s| s.elevated = true);
    svc.set_dns(DnsChoice::Automatic).unwrap();
    assert!(f.log().is_empty());
}

#[test]
fn dns_is_disabled_while_offline() {
    let (f, svc) = setup();
    f.with(|s| {
        s.elevated = true;
        s.internet_if = None;
    });
    assert_eq!(svc.dns_state(), Err(NetError::Offline));
    assert_eq!(svc.set_dns(DnsChoice::Google), Err(NetError::Offline));
    assert!(f.log().is_empty());
}

#[test]
fn dns_undo_goes_to_the_adapter_it_was_made_on() {
    let (f, svc) = setup();
    f.with(|s| s.elevated = true);
    let ch = svc.set_dns(DnsChoice::Cloudflare).unwrap();
    // Now Wi-Fi becomes the connection in use.
    f.with(|s| {
        let w = s.adapters.iter_mut().find(|a| a.id == WIFI).unwrap();
        w.connected = true;
        w.gateways = vec!["10.0.0.1".parse().unwrap()];
        s.internet_if = Some(17);
    });
    svc.undo(ch.id).unwrap();
    assert!(f.log().last().is_some_and(|l| l == "flush"));
    assert!(f.log().iter().any(|l| l == &format!("dns {ETH} v4=[] v6=[]")));
}

#[test]
fn dns_classify_ipv4_only_preset_and_order() {
    let cf = DnsChoice::Cloudflare.servers();
    let v4_only = DnsServers { v4: cf.v4.iter().rev().copied().collect(), v6: vec![] };
    assert_eq!(DnsCurrent::classify(&v4_only), DnsCurrent::Preset { choice: DnsChoice::Cloudflare, ipv6: false });
    let mixed = DnsServers { v4: cf.v4.clone(), v6: DnsChoice::Google.servers().v6 };
    assert_eq!(DnsCurrent::classify(&mixed), DnsCurrent::Custom);
}

#[test]
fn undo_errors() {
    let (_f, svc) = setup();
    assert_eq!(svc.undo(42), Err(NetError::NothingToUndo));
    assert_eq!(svc.undo_last(), Err(NetError::NothingToUndo));
    let ch = svc.set_adapter(WIFI, false).unwrap();
    assert_eq!(svc.undo_last().unwrap().id, ch.id);
    assert_eq!(svc.undo(ch.id), Err(NetError::NothingToUndo), "an undo happens once");
}

#[test]
fn dns_popup_rows() {
    let rows: Vec<_> = DnsChoice::ALL.iter().map(|c| c.label()).collect();
    assert_eq!(rows, vec![("Automatic", "from your router"), ("Cloudflare", "1.1.1.1"), ("Google", "8.8.8.8")]);
}

/// Order 036: the reset writes DNS on ANY adapter by id (not only the one in use), needs admin, writes only a change.
#[test]
fn set_dns_on_an_adapter_by_id() {
    let (f, svc) = setup();
    let cf = DnsChoice::Cloudflare.servers();
    assert_eq!(svc.set_dns_on(WIFI, &cf), Err(NetError::NeedsAdmin));
    assert!(f.log().is_empty(), "refused before Windows");
    f.with(|s| s.elevated = true);
    let ch = svc.set_dns_on(WIFI, &cf).unwrap();
    assert!(matches!(&ch.kind, ChangeKind::Dns { id, old, .. } if id == WIFI && old.is_automatic()));
    assert_eq!(f.with(|s| s.dns.get(WIFI).cloned()), Some(cf.clone()));
    let n = f.log().len();
    svc.set_dns_on(WIFI, &cf).unwrap();
    assert_eq!(f.log().len(), n, "already so: nothing written");
    svc.set_dns_on(WIFI, &DnsServers::default()).unwrap();
    assert_eq!(f.with(|s| s.dns.get(WIFI).cloned()), None, "back to automatic");
    assert_eq!(svc.set_dns_on("{NOPE}", &cf), Err(NetError::NoSuchAdapter("{NOPE}".into())));
}
