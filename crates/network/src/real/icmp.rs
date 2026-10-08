//! ICMP echo through IP Helper (IcmpSendEcho2 / Icmp6SendEcho2) - the documented ping API, no admin, no raw sockets.

use std::net::{IpAddr, Ipv6Addr};
use std::time::{Duration, Instant};

use windows::Win32::NetworkManagement::IpHelper::{
    Icmp6CreateFile, Icmp6SendEcho2, IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho2, ICMPV6_ECHO_REPLY_LH,
    ICMP_ECHO_REPLY,
};
use windows::Win32::Networking::WinSock::{AF_INET6, IN6_ADDR, IN6_ADDR_0, SOCKADDR_IN6, SOCKADDR_IN6_0};

use crate::error::{NetError, Result};

const IP_SUCCESS: u32 = 0;
const IP_REQ_TIMED_OUT: u32 = 11010;

fn sockaddr6(ip: Ipv6Addr) -> SOCKADDR_IN6 {
    SOCKADDR_IN6 {
        sin6_family: AF_INET6,
        sin6_port: 0,
        sin6_flowinfo: 0,
        sin6_addr: IN6_ADDR { u: IN6_ADDR_0 { Byte: ip.octets() } },
        Anonymous: SOCKADDR_IN6_0 { sin6_scope_id: 0 },
    }
}

/// One echo; the round trip is Windows' own (whole ms). Under 1 ms Windows says 0, so the wall time is used
/// instead (sub-ms precision for the router ping).
pub fn ping(ip: IpAddr, timeout: Duration) -> Result<Duration> {
    let ms = timeout.as_millis().clamp(1, u32::MAX as u128) as u32;
    let payload = [0x42u8; 32];
    // Reply buffer: reply struct + payload + 8 bytes ICMP error room (per the docs) - generous.
    let mut reply = vec![0u64; 64];
    let reply_bytes = (reply.len() * 8) as u32;
    unsafe {
        let t0 = Instant::now();
        let (n, status, rtt_ms) = match ip {
            IpAddr::V4(v4) => {
                let h = IcmpCreateFile().map_err(|e| NetError::Os { call: "IcmpCreateFile", code: e.code().0 as u32 })?;
                let n = IcmpSendEcho2(
                    h,
                    None,
                    None,
                    None,
                    u32::from_ne_bytes(v4.octets()),
                    payload.as_ptr() as *const _,
                    payload.len() as u16,
                    None,
                    reply.as_mut_ptr() as *mut _,
                    reply_bytes,
                    ms,
                );
                let _ = IcmpCloseHandle(h);
                let r = &*(reply.as_ptr() as *const ICMP_ECHO_REPLY);
                (n, r.Status, r.RoundTripTime)
            }
            IpAddr::V6(v6) => {
                let h = Icmp6CreateFile().map_err(|e| NetError::Os { call: "Icmp6CreateFile", code: e.code().0 as u32 })?;
                let src = sockaddr6(Ipv6Addr::UNSPECIFIED);
                let dst = sockaddr6(v6);
                let n = Icmp6SendEcho2(
                    h,
                    None,
                    None,
                    None,
                    &src,
                    &dst,
                    payload.as_ptr() as *const _,
                    payload.len() as u16,
                    None,
                    reply.as_mut_ptr() as *mut _,
                    reply_bytes,
                    ms,
                );
                let _ = IcmpCloseHandle(h);
                let r = &*(reply.as_ptr() as *const ICMPV6_ECHO_REPLY_LH);
                (n, r.Status, r.RoundTripTime)
            }
        };
        let wall = t0.elapsed();
        if n == 0 {
            let code = windows::Win32::Foundation::GetLastError().0;
            return Err(if code == IP_REQ_TIMED_OUT { NetError::Timeout } else { NetError::Unreachable(format!("{ip} (code {code})")) });
        }
        match status {
            IP_SUCCESS => Ok(if rtt_ms == 0 { wall.min(Duration::from_millis(1)) } else { Duration::from_millis(rtt_ms as u64) }),
            IP_REQ_TIMED_OUT => Err(NetError::Timeout),
            other => Err(NetError::Unreachable(format!("{ip} (ICMP status {other})"))),
        }
    }
}
