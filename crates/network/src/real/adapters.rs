//! Adapters: the interface table (GetIfTable2) for the physical Wi-Fi / Ethernet adapters, IP Helper
//! (GetAdaptersAddresses) for names, gateways and DNS servers, WLAN API for the SSID, the device manager for
//! switched-off adapters, GetBestInterfaceEx for "which one is in use". All read-only.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};

use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetAdaptersAddresses, GetBestInterfaceEx, GetIfTable2, GAA_FLAG_INCLUDE_GATEWAYS,
    GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH, MIB_IF_TABLE2,
};
use windows::Win32::NetworkManagement::Ndis::{IfOperStatusNotPresent, IfOperStatusUp};
use windows::Win32::Networking::WinSock::{AF_INET, AF_UNSPEC, IN_ADDR, SOCKADDR, SOCKADDR_IN};

use super::util::{from_wide, guid_string, os_err, pstr_or_empty, pwstr, sockaddr_ip, win32};
use super::{device, radio, wlan};
use crate::error::Result;
use crate::model::{classify_adapter, Adapter, AdapterKind};


struct IpInfo {
    name: String,
    gateways: Vec<IpAddr>,
    dns: Vec<IpAddr>,
}

/// GetAdaptersAddresses, keyed by adapter GUID.
fn ip_info() -> Result<HashMap<String, IpInfo>> {
    let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
    let mut size: u32 = 16 * 1024;
    let mut buf: Vec<u64>; // u64 for alignment
    loop {
        buf = vec![0u64; (size as usize).div_ceil(8)];
        let r = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC.0 as u32,
                flags,
                None,
                Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
                &mut size,
            )
        };
        if r == ERROR_BUFFER_OVERFLOW.0 {
            continue;
        }
        if r != NO_ERROR.0 {
            return Err(os_err("GetAdaptersAddresses", r));
        }
        break;
    }
    let mut out = HashMap::new();
    let mut p = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
    while !p.is_null() {
        let a = unsafe { &*p };
        let id = unsafe { pstr_or_empty(a.AdapterName.0) }.to_ascii_uppercase();
        let mut gateways = Vec::new();
        let mut g = a.FirstGatewayAddress;
        while !g.is_null() {
            let gw = unsafe { &*g };
            if let Some(ip) = unsafe { sockaddr_ip(gw.Address.lpSockaddr as *const SOCKADDR) } {
                gateways.push(ip);
            }
            g = gw.Next;
        }
        let mut dns = Vec::new();
        let mut d = a.FirstDnsServerAddress;
        while !d.is_null() {
            let ds = unsafe { &*d };
            if let Some(ip) = unsafe { sockaddr_ip(ds.Address.lpSockaddr as *const SOCKADDR) } {
                // fec0:0:0:ffff::1/2/3 are Windows' old "site-local" placeholders when IPv6 has no DNS - not real.
                let placeholder = matches!(ip, IpAddr::V6(v6) if v6.segments()[0] == 0xfec0);
                if !placeholder {
                    dns.push(ip);
                }
            }
            d = ds.Next;
        }
        out.insert(id, IpInfo { name: unsafe { pwstr(a.FriendlyName.0) }, gateways, dns });
        p = a.Next;
    }
    Ok(out)
}

pub fn list() -> Result<Vec<Adapter>> {
    let ip = ip_info()?;
    let devices = device::list_net_devices().unwrap_or_default();
    let wifi_radio = radio::wifi_state().ok().flatten();
    let wlan = wlan::connections();

    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    win32("GetIfTable2", unsafe { GetIfTable2(&mut table) })?;
    let mut out: Vec<Adapter> = Vec::new();
    unsafe {
        let n = (*table).NumEntries as usize;
        let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), n);
        for r in rows {
            let flags = r.InterfaceAndOperStatusFlags._bitfield;
            let hardware = flags & 0x01 != 0;
            let filter = flags & 0x02 != 0;
            // v21: every adapter the user can switch (Wi-Fi, Ethernet, VPN, virtual switches, Bluetooth PAN). Filter
            // layers (the same adapter again, e.g. "-WFP Native MAC Layer LightWeight Filter") are skipped; a non-hardware
            // interface must be a network connection IP Helper lists (no WAN miniports / kernel debug pseudo rows).
            if filter {
                continue;
            }
            let id = guid_string(&r.InterfaceGuid);
            if out.iter().any(|a| a.id == id) {
                continue;
            }
            let info = ip.get(&id);
            let Some(kind) = classify_adapter(r.Type, hardware, &from_wide(&r.Description)) else {
                continue;
            };
            if !hardware && info.is_none() {
                continue;
            }
            let dev = devices.iter().find(|d| d.guid == id);
            let device_on = dev.map(|d| !d.disabled).unwrap_or(r.OperStatus != IfOperStatusNotPresent);
            let connected = r.OperStatus == IfOperStatusUp;
            let enabled = match kind {
                AdapterKind::Wifi => device_on && wifi_radio.unwrap_or(true),
                _ => device_on,
            };
            let w = wlan.get(&id);
            out.push(Adapter {
                id: id.clone(),
                if_index: r.InterfaceIndex,
                name: info.map(|i| i.name.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| from_wide(&r.Alias)),
                description: from_wide(&r.Description),
                kind,
                enabled,
                device_disabled: !device_on,
                connected,
                link_speed_bps: (connected && r.ReceiveLinkSpeed != u64::MAX && r.ReceiveLinkSpeed > 0)
                    .then_some(r.ReceiveLinkSpeed),
                ssid: w.and_then(|w| w.ssid.clone()),
                signal_pct: w.and_then(|w| w.signal_pct),
                gateways: info.map(|i| i.gateways.clone()).unwrap_or_default(),
                dns_servers: if connected { info.map(|i| i.dns.clone()).unwrap_or_default() } else { vec![] },
            });
        }
        FreeMibTable(table as *const _);
    }
    // Switched-off adapters can drop out of the interface table; the device manager still lists them.
    for d in devices.iter().filter(|d| d.disabled) {
        if out.iter().any(|a| a.id == d.guid) {
            continue;
        }
        let Some(kind) = d.if_type.and_then(|t| classify_adapter(t, d.physical, &d.description)) else {
            continue;
        };
        out.push(Adapter {
            id: d.guid.clone(),
            if_index: 0,
            name: d.connection_name.clone().unwrap_or_else(|| d.description.clone()),
            description: d.description.clone(),
            kind,
            enabled: false,
            device_disabled: true,
            connected: false,
            link_speed_bps: None,
            ssid: None,
            signal_pct: None,
            gateways: vec![],
            dns_servers: vec![],
        });
    }
    Ok(out)
}

/// The interface Windows would send internet traffic out of (route lookup for 1.1.1.1; nothing is sent).
pub fn internet_if_index() -> Result<Option<u32>> {
    let dest = SOCKADDR_IN {
        sin_family: AF_INET,
        sin_port: 0,
        sin_addr: IN_ADDR::from(Ipv4Addr::new(1, 1, 1, 1)),
        sin_zero: [0; 8],
    };
    let mut idx = 0u32;
    let r = unsafe { GetBestInterfaceEx(&dest as *const _ as *const SOCKADDR, &mut idx) };
    Ok((r == NO_ERROR.0).then_some(idx))
}
