//! The REAL Windows implementation of [`NetworkOs`]. Reads are safe; the change calls (radio, device, DNS) are only
//! ever reached through `NetworkService`, which refuses admin changes when the process is not elevated.

mod adapters;
mod device;
mod dns;
mod icmp;
mod radio;
mod speed;
mod util;
mod wlan;

use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use crate::error::{NetError, Result};
use crate::model::{Adapter, DnsServers};
use crate::os::NetworkOs;

pub use speed::CloudflareSpeed;

/// The real PC.
#[derive(Debug, Default)]
pub struct WindowsNet {
    _private: (),
}

impl WindowsNet {
    pub fn new() -> WindowsNet {
        WindowsNet { _private: () }
    }

    /// Read-only: the wired adapter's Device Manager entry (instance id + disabled?) - proof that the device the
    /// Ethernet switch would act on is found. Used by examples/show.rs.
    pub fn device_of(&self, id: &str) -> Result<Option<(String, bool)>> {
        device::describe(id)
    }

    /// The PC's local clock as "21:37" (the speed test footer "Today 21:37 · Ethernet").
    pub fn local_hhmm() -> String {
        let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        format!("{:02}:{:02}", t.wHour, t.wMinute)
    }

    /// Read-only: the Wi-Fi radio state Windows reports (None = no Wi-Fi radio).
    pub fn wifi_radio(&self) -> Result<Option<bool>> {
        radio::wifi_state()
    }
}

impl NetworkOs for WindowsNet {
    fn adapters(&self) -> Result<Vec<Adapter>> {
        adapters::list()
    }

    fn internet_if_index(&self) -> Result<Option<u32>> {
        adapters::internet_if_index()
    }

    fn is_elevated(&self) -> bool {
        util::is_elevated()
    }

    fn set_wifi_radio(&self, on: bool) -> Result<()> {
        radio::set_wifi(on)
    }

    fn set_adapter_enabled(&self, id: &str, on: bool) -> Result<()> {
        if !util::is_elevated() {
            return Err(NetError::NeedsAdmin);
        }
        device::set_enabled(id, on)
    }

    fn icmp_ping(&self, ip: IpAddr, timeout: Duration) -> Result<Duration> {
        icmp::ping(ip, timeout)
    }

    fn tcp_ping(&self, addr: SocketAddr, timeout: Duration) -> Result<Duration> {
        let t0 = Instant::now();
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(s) => {
                let rtt = t0.elapsed();
                drop(s);
                Ok(rtt)
            }
            Err(e) => Err(match e.kind() {
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => NetError::Timeout,
                _ => NetError::Unreachable(format!("{addr}: {e}")),
            }),
        }
    }

    fn udp_ping(&self, addr: SocketAddr, timeout: Duration) -> Result<Duration> {
        let local: SocketAddr = if addr.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }.parse().unwrap();
        let sock = UdpSocket::bind(local).map_err(|e| NetError::Unreachable(format!("bind: {e}")))?;
        sock.connect(addr).map_err(|e| NetError::Unreachable(format!("{addr}: {e}")))?;
        // A per-call random-ish tag so a late echo of an earlier probe isn't taken for this one.
        let now_ns = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
        let tag = (now_ns ^ std::process::id() as u64 ^ addr.port() as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .to_le_bytes();
        let t0 = Instant::now();
        sock.send(&tag).map_err(|e| NetError::Unreachable(format!("{addr}: {e}")))?;
        let mut buf = [0u8; 64];
        loop {
            let left = timeout.checked_sub(t0.elapsed()).filter(|d| !d.is_zero()).ok_or(NetError::Timeout)?;
            sock.set_read_timeout(Some(left)).map_err(|_| NetError::Timeout)?;
            match sock.recv(&mut buf) {
                Ok(n) if buf[..n] == tag => return Ok(t0.elapsed()),
                Ok(_) => continue,
                Err(e) => {
                    return Err(match e.kind() {
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => NetError::Timeout,
                        _ => NetError::Unreachable(format!("{addr}: {e}")),
                    })
                }
            }
        }
    }

    fn resolve(&self, host: &str) -> Result<Vec<IpAddr>> {
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(vec![ip]);
        }
        let mut ips: Vec<IpAddr> = (host, 0u16)
            .to_socket_addrs()
            .map_err(|_| NetError::Resolve(host.into()))?
            .map(|a| a.ip())
            .collect();
        ips.sort_by_key(|i| !i.is_ipv4());
        ips.dedup();
        if ips.is_empty() {
            return Err(NetError::Resolve(host.into()));
        }
        Ok(ips)
    }

    fn flush_dns(&self) -> Result<()> {
        dns::flush()
    }

    fn dns_servers(&self, id: &str) -> Result<DnsServers> {
        dns::get(id)
    }

    fn wifi_networks(&self) -> Result<Vec<crate::model::WifiNetwork>> {
        wlan::networks()
    }

    fn wifi_connect(&self, ssid: &str, password: Option<&str>, auto: bool) -> Result<()> {
        wlan::connect(ssid, password, auto)
    }

    fn wifi_disconnect(&self) -> Result<()> {
        wlan::disconnect()
    }

    fn wifi_forget(&self, ssid: &str) -> Result<()> {
        wlan::forget(ssid)
    }

    fn set_dns_servers(&self, id: &str, servers: &DnsServers) -> Result<()> {
        if !util::is_elevated() {
            return Err(NetError::NeedsAdmin);
        }
        dns::set(id, servers)
    }
}
