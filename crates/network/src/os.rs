//! The OS layer: everything that touches Windows goes through this trait, so every behaviour can be tested
//! against [`crate::fake::FakeNet`]. The real implementation is [`crate::real::WindowsNet`].

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use crate::error::{NetError, Result};
use crate::model::{Adapter, DnsServers, WifiNetwork};

pub trait NetworkOs: Send + Sync {
    /// Physical Wi-Fi and Ethernet adapters, incl. switched-off ones. `enabled` of a Wi-Fi adapter = its radio.
    fn adapters(&self) -> Result<Vec<Adapter>>;
    /// Interface index that carries the route to the internet right now (no traffic is sent), None = offline.
    fn internet_if_index(&self) -> Result<Option<u32>>;
    /// True when this process runs elevated (admin). Admin changes are refused before touching Windows otherwise.
    fn is_elevated(&self) -> bool;

    /// Wi-Fi radio on/off (all Wi-Fi radios, like the Quick Settings button). No admin.
    fn set_wifi_radio(&self, on: bool) -> Result<()>;
    /// Enable / disable a wired adapter's device. ADMIN. Only called when `is_elevated()`.
    fn set_adapter_enabled(&self, id: &str, on: bool) -> Result<()>;

    /// One ICMP echo. Err(Timeout) when nothing answers in `timeout`.
    fn icmp_ping(&self, ip: IpAddr, timeout: Duration) -> Result<Duration>;
    /// Time of one TCP connect (SYN -> SYN/ACK = one round trip); the socket is closed at once.
    fn tcp_ping(&self, addr: SocketAddr, timeout: Duration) -> Result<Duration>;
    /// Time of one UDP echo (a few bytes to an echo beacon, until the same bytes come back).
    fn udp_ping(&self, addr: SocketAddr, timeout: Duration) -> Result<Duration>;
    /// Host name -> addresses (IPv4 first).
    fn resolve(&self, host: &str) -> Result<Vec<IpAddr>>;

    /// Empty Windows' DNS cache (the DNS Client service). No admin.
    fn flush_dns(&self) -> Result<()>;
    /// DNS servers set BY HAND on the adapter (empty = automatic).
    fn dns_servers(&self, id: &str) -> Result<DnsServers>;
    /// Writes hand-set DNS servers (IPv4 and IPv6 lists; an empty list = back to automatic for that family). ADMIN.
    fn set_dns_servers(&self, id: &str, servers: &DnsServers) -> Result<()>;

    /// Nearby Wi-Fi networks from Windows' last scan (WlanGetAvailableNetworkList - a read of the list Windows keeps;
    /// no new scan is started). One entry per network name, strongest first. No admin.
    fn wifi_networks(&self) -> Result<Vec<WifiNetwork>> {
        Err(NetError::Unsupported("Wi-Fi networks".into()))
    }
    /// Connect to `ssid`. A secured network without a saved profile needs `password` (a profile is saved for it,
    /// `auto` = connect automatically). No admin.
    fn wifi_connect(&self, _ssid: &str, _password: Option<&str>, _auto: bool) -> Result<()> {
        Err(NetError::Unsupported("Wi-Fi connect".into()))
    }
    /// Disconnect the Wi-Fi adapter from its network. No admin.
    fn wifi_disconnect(&self) -> Result<()> {
        Err(NetError::Unsupported("Wi-Fi disconnect".into()))
    }
    /// Delete the saved profile of `ssid` (its password is gone from the PC). No admin for the user's profiles.
    fn wifi_forget(&self, _ssid: &str) -> Result<()> {
        Err(NetError::Unsupported("Wi-Fi forget".into()))
    }
}
