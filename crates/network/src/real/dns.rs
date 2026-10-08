//! DNS: read / write the hand-set servers of an adapter (GetInterfaceDnsSettings / SetInterfaceDnsSettings,
//! Windows 10 2004+; setting needs ADMIN) and flush the DNS cache (DnsFlushResolverCache in dnsapi.dll - the call
//! `ipconfig /flushdns` makes; no admin).

use std::net::{Ipv4Addr, Ipv6Addr};

use windows::core::{s, w, PWSTR};
use windows::Win32::NetworkManagement::IpHelper::{
    FreeInterfaceDnsSettings, GetInterfaceDnsSettings, SetInterfaceDnsSettings, DNS_INTERFACE_SETTINGS,
    DNS_INTERFACE_SETTINGS_VERSION1, DNS_SETTING_IPV6, DNS_SETTING_NAMESERVER,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

use super::util::{os_err, parse_guid, pwstr, wide};
use crate::error::{NetError, Result};
use crate::model::DnsServers;

/// "1.1.1.1,1.0.0.1" or "1.1.1.1 1.0.0.1" -> addresses (unparseable parts skipped).
pub fn parse_list<T: std::str::FromStr>(s: &str) -> Vec<T> {
    s.split([',', ' ', ';']).filter(|p| !p.is_empty()).filter_map(|p| p.trim().parse().ok()).collect()
}

fn read_family(guid: windows::core::GUID, ipv6: bool) -> Result<String> {
    let mut st = DNS_INTERFACE_SETTINGS {
        Version: DNS_INTERFACE_SETTINGS_VERSION1,
        Flags: if ipv6 { DNS_SETTING_IPV6 as u64 } else { 0 },
        ..Default::default()
    };
    let r = unsafe { GetInterfaceDnsSettings(guid, &mut st) };
    if r.0 != 0 {
        return Err(os_err("GetInterfaceDnsSettings", r.0));
    }
    let ns = unsafe { pwstr(st.NameServer.0) };
    unsafe { FreeInterfaceDnsSettings(&mut st) };
    Ok(ns)
}

pub fn get(id: &str) -> Result<DnsServers> {
    let guid = parse_guid(id).ok_or_else(|| NetError::NoSuchAdapter(id.into()))?;
    Ok(DnsServers {
        v4: parse_list::<Ipv4Addr>(&read_family(guid, false)?),
        v6: parse_list::<Ipv6Addr>(&read_family(guid, true)?),
    })
}

fn write_family(guid: windows::core::GUID, ipv6: bool, list: &str) -> Result<()> {
    let mut ns = wide(list);
    let st = DNS_INTERFACE_SETTINGS {
        Version: DNS_INTERFACE_SETTINGS_VERSION1,
        Flags: DNS_SETTING_NAMESERVER as u64 | if ipv6 { DNS_SETTING_IPV6 as u64 } else { 0 },
        NameServer: PWSTR(ns.as_mut_ptr()),
        ..Default::default()
    };
    let r = unsafe { SetInterfaceDnsSettings(guid, &st) };
    match r.0 {
        0 => Ok(()),
        5 => Err(NetError::NeedsAdmin),
        c => Err(os_err("SetInterfaceDnsSettings", c)),
    }
}

/// ADMIN. Empty list = back to automatic (DHCP) for that family. Never called by tests.
pub fn set(id: &str, s: &DnsServers) -> Result<()> {
    let guid = parse_guid(id).ok_or_else(|| NetError::NoSuchAdapter(id.into()))?;
    let v4: Vec<String> = s.v4.iter().map(|a| a.to_string()).collect();
    let v6: Vec<String> = s.v6.iter().map(|a| a.to_string()).collect();
    // Two calls (Windows sets IPv4 and IPv6 separately). If the IPv6 one fails, the IPv4 one is put back, so a
    // failed switch leaves the adapter as it was (no half-changed state without an undo record).
    let old_v4 = read_family(guid, false)?;
    write_family(guid, false, &v4.join(","))?;
    if let Err(e) = write_family(guid, true, &v6.join(",")) {
        let _ = write_family(guid, false, &old_v4);
        return Err(e);
    }
    Ok(())
}

/// Empties the DNS Client cache. `DnsFlushResolverCache` is exported by dnsapi.dll but not in the SDK headers,
/// so it is looked up at run time.
pub fn flush() -> Result<()> {
    unsafe {
        let lib = LoadLibraryW(w!("dnsapi.dll")).map_err(|e| NetError::Os { call: "LoadLibraryW(dnsapi)", code: e.code().0 as u32 })?;
        let f = GetProcAddress(lib, s!("DnsFlushResolverCache"))
            .ok_or_else(|| NetError::Unsupported("DnsFlushResolverCache missing".into()))?;
        let f: extern "system" fn() -> i32 = std::mem::transmute(f);
        if f() != 0 {
            Ok(())
        } else {
            let code = windows::Win32::Foundation::GetLastError().0;
            Err(os_err("DnsFlushResolverCache", code))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_server_lists_parse_with_commas_or_spaces() {
        assert_eq!(parse_list::<Ipv4Addr>("8.8.8.8,8.8.4.4"), vec![Ipv4Addr::new(8, 8, 8, 8), Ipv4Addr::new(8, 8, 4, 4)]);
        assert_eq!(parse_list::<Ipv4Addr>("1.1.1.1 1.0.0.1"), vec![Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(1, 0, 0, 1)]);
        assert!(parse_list::<Ipv4Addr>("").is_empty());
        assert_eq!(
            parse_list::<Ipv6Addr>("2606:4700:4700::1111,2606:4700:4700::1001").len(),
            2
        );
    }
}
