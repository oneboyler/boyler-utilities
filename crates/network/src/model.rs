//! Plain data the menu shows: adapters, the connection state, DNS settings.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// The Windows adapter GUID, e.g. `{4D36E972-...}` (stable across reboots; `AdapterName` in IP Helper).
pub type AdapterId = String;

/// Which kind of connection a row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdapterKind {
    /// IF_TYPE_IEEE80211 (71). Its switch is the Wi-Fi radio (like Quick Settings), no admin.
    Wifi,
    /// IF_TYPE_ETHERNET_CSMACD (6). Its switch disables the device, needs admin.
    Ethernet,
    /// A VPN tunnel (WireGuard / Wintun / TAP / IF_TYPE_PROP_VIRTUAL 53 / PPP 23). Its switch = the device, admin.
    Vpn,
    /// A virtual switch adapter (Hyper-V / WSL, VirtualBox, VMware). Its switch = the device, admin.
    Virtual,
    /// Bluetooth personal area network. Its switch = the device, admin.
    Bluetooth,
}

impl AdapterKind {
    /// The type badge the row shows (v21: "Ethernet", "Wi-Fi", "VPN", "Virtual", "Bluetooth").
    pub fn badge(self) -> &'static str {
        match self {
            AdapterKind::Wifi => "Wi-Fi",
            AdapterKind::Ethernet => "Ethernet",
            AdapterKind::Vpn => "VPN",
            AdapterKind::Virtual => "Virtual",
            AdapterKind::Bluetooth => "Bluetooth",
        }
    }

    /// The order of the Connection group: Ethernet, Wi-Fi, then VPN, virtual, Bluetooth (the drawing's NW list).
    pub fn order(self) -> u8 {
        match self {
            AdapterKind::Ethernet => 0,
            AdapterKind::Wifi => 1,
            AdapterKind::Vpn => 2,
            AdapterKind::Virtual => 3,
            AdapterKind::Bluetooth => 4,
        }
    }

    /// A Wi-Fi or wired card (not a tunnel / virtual switch / Bluetooth PAN): only these can carry "In use".
    pub fn physical(self) -> bool {
        matches!(self, AdapterKind::Wifi | AdapterKind::Ethernet)
    }
}

/// Classifies an interface from its IF type, the "hardware interface" flag and its description (pure, tested).
/// `None` = not a row of the Connection group (loopback, tunnels like Teredo, kernel debug, WAN miniports).
pub fn classify_adapter(if_type: u32, hardware: bool, description: &str) -> Option<AdapterKind> {
    let d = description.to_ascii_lowercase();
    let vpn_words = ["vpn", "wireguard", "wintun", "tap-windows", "tap-", "openvpn", "tailscale", "zerotier", "nordlynx", "mullvad", "proton", "fortinet", "cisco anyconnect"];
    let virt_words = ["hyper-v", "virtualbox", "vmware", "virtual ethernet", "vethernet", "docker"];
    if d.contains("wan miniport") || d.contains("kernel debug") || d.contains("loopback") || d.contains("teredo") || d.contains("6to4") || d.contains("isatap") || d.contains("ip-https") {
        return None;
    }
    match if_type {
        71 => Some(AdapterKind::Wifi),
        6 | 53 | 23 => {
            if d.contains("bluetooth") {
                Some(AdapterKind::Bluetooth)
            } else if if_type != 6 || vpn_words.iter().any(|w| d.contains(w)) {
                Some(AdapterKind::Vpn)
            } else if !hardware || virt_words.iter().any(|w| d.contains(w)) {
                Some(AdapterKind::Virtual)
            } else {
                Some(AdapterKind::Ethernet)
            }
        }
        _ => None,
    }
}

/// One network adapter of the Connection group: Wi-Fi and Ethernet cards and (v21) VPN, virtual and Bluetooth ones,
/// each with its type badge and switch. Loopback, Teredo / 6to4 tunnels and WAN miniports are left out.
#[derive(Debug, Clone, PartialEq)]
pub struct Adapter {
    pub id: AdapterId,
    /// Interface index (changes when the adapter is re-enabled; use `id` to remember an adapter).
    pub if_index: u32,
    /// "Ethernet", "Wi-Fi" - the name the user sees in Windows.
    pub name: String,
    /// The hardware name, e.g. "Intel(R) Ethernet Controller I225-V".
    pub description: String,
    pub kind: AdapterKind,
    /// The switch state: Wi-Fi = radio on; Ethernet = device enabled.
    pub enabled: bool,
    /// The adapter is disabled in Device Manager (for Wi-Fi: then no radio exists; switching it on enables the
    /// device, admin).
    pub device_disabled: bool,
    /// Link up (cable in / Wi-Fi associated) - OperStatus Up.
    pub connected: bool,
    /// Link speed in bits per second (receive side), when connected.
    pub link_speed_bps: Option<u64>,
    /// Wi-Fi network name, when connected and Windows lets us read it.
    pub ssid: Option<String>,
    /// Wi-Fi signal quality 0-100, when connected.
    pub signal_pct: Option<u8>,
    pub gateways: Vec<IpAddr>,
    /// The DNS servers the adapter uses right now (from DHCP or set by hand).
    pub dns_servers: Vec<IpAddr>,
}

/// Everything the "Connection" group shows.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionState {
    /// Wi-Fi first, then Ethernet, each in Windows' order.
    pub adapters: Vec<Adapter>,
    /// The adapter that carries the route to the internet (the "in use" one, with the ping pill), if any.
    pub in_use: Option<AdapterId>,
}

impl ConnectionState {
    /// No adapter carries an internet route -> the "Offline" tag; DNS switcher and speed test are disabled.
    pub fn offline(&self) -> bool {
        self.in_use.is_none()
    }
    pub fn in_use_adapter(&self) -> Option<&Adapter> {
        let id = self.in_use.as_ref()?;
        self.adapters.iter().find(|a| &a.id == id)
    }
    pub fn adapter(&self, id: &str) -> Option<&Adapter> {
        self.adapters.iter().find(|a| a.id == id)
    }
}

/// DNS servers set BY HAND on an adapter. Empty list for a family = that family is automatic (DHCP / router).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DnsServers {
    pub v4: Vec<Ipv4Addr>,
    pub v6: Vec<Ipv6Addr>,
}

impl DnsServers {
    pub fn is_automatic(&self) -> bool {
        self.v4.is_empty() && self.v6.is_empty()
    }

    /// The Custom DNS form (v21): IPv4 primary (needed) + secondary, IPv6 primary + secondary (both may stay empty).
    /// Err = which of the four fields is wrong ([v4a, v4b, v6a, v6b]), so the page marks exactly those red.
    pub fn from_fields(v4a: &str, v4b: &str, v6a: &str, v6b: &str) -> std::result::Result<DnsServers, [bool; 4]> {
        let (v4a, v4b, v6a, v6b) = (v4a.trim(), v4b.trim(), v6a.trim(), v6b.trim());
        let p4 = |s: &str| s.parse::<Ipv4Addr>().ok();
        let p6 = |s: &str| s.parse::<Ipv6Addr>().ok();
        let bad = [
            p4(v4a).is_none(),
            !v4b.is_empty() && p4(v4b).is_none(),
            !v6a.is_empty() && p6(v6a).is_none(),
            !v6b.is_empty() && p6(v6b).is_none(),
        ];
        if bad.iter().any(|b| *b) {
            return Err(bad);
        }
        let mut s = DnsServers::default();
        s.v4.extend(p4(v4a));
        s.v4.extend(p4(v4b));
        s.v6.extend(p6(v6a));
        s.v6.extend(p6(v6b));
        Ok(s)
    }
}

/// One nearby Wi-Fi network (the v21 "Wi-Fi networks" fold card). Strongest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiNetwork {
    pub ssid: String,
    /// 0-100 (WLAN signal quality); the row's 4-bar glyph = 1 + quality / 25 (capped at 4).
    pub signal_pct: u8,
    /// Needs a password (WPA / WPA2 / WPA3); false = open.
    pub secured: bool,
    /// Windows has a saved profile (it can connect without a password; Forget removes it).
    pub saved: bool,
    pub connected: bool,
}

impl WifiNetwork {
    /// Signal bars 1..=4.
    pub fn bars(&self) -> u8 {
        (1 + self.signal_pct / 25).min(4)
    }
    /// The row's second line (drawing wording).
    pub fn sub_line(&self) -> &'static str {
        if self.connected {
            "Connected · secured"
        } else if self.saved {
            "Saved"
        } else if self.secured {
            "Secured"
        } else {
            "Open · not secured"
        }
    }
}

/// The three choices of the DNS popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DnsChoice {
    /// Back to DHCP: the router's DNS.
    Automatic,
    /// 1.1.1.1 / 1.0.0.1 + 2606:4700:4700::1111 / ::1001
    Cloudflare,
    /// 8.8.8.8 / 8.8.4.4 + 2001:4860:4860::8888 / ::8844
    Google,
}

impl DnsChoice {
    pub const ALL: [DnsChoice; 3] = [DnsChoice::Automatic, DnsChoice::Cloudflare, DnsChoice::Google];

    /// The exact servers this choice writes (IPv4 and IPv6 both, or IPv6 would keep the router's DNS).
    /// Sources: Cloudflare https://developers.cloudflare.com/1.1.1.1/ip-addresses/ ;
    /// Google https://developers.google.com/speed/public-dns/docs/using
    pub fn servers(self) -> DnsServers {
        match self {
            DnsChoice::Automatic => DnsServers::default(),
            DnsChoice::Cloudflare => DnsServers {
                v4: vec![Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(1, 0, 0, 1)],
                v6: vec![
                    Ipv6Addr::new(0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1111),
                    Ipv6Addr::new(0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1001),
                ],
            },
            DnsChoice::Google => DnsServers {
                v4: vec![Ipv4Addr::new(8, 8, 8, 8), Ipv4Addr::new(8, 8, 4, 4)],
                v6: vec![
                    Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888),
                    Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8844),
                ],
            },
        }
    }

    /// Popup row text: name + sub-line (DESIGN §3.11).
    pub fn label(self) -> (&'static str, &'static str) {
        match self {
            DnsChoice::Automatic => ("Automatic", "from your router"),
            DnsChoice::Cloudflare => ("Cloudflare", "1.1.1.1"),
            DnsChoice::Google => ("Google", "8.8.8.8"),
        }
    }
}

/// What the adapter's hand-set DNS amounts to (the header button "DNS <current>").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DnsCurrent {
    Automatic,
    /// A preset; `ipv6` = its IPv6 servers are set too (false = IPv6 still asks the router).
    Preset { choice: DnsChoice, ipv6: bool },
    /// Something else set by hand (another tool, the user) - shown as "Custom", never overwritten without a pick.
    Custom,
}

impl DnsCurrent {
    /// Classifies hand-set servers. Order of the servers does not matter.
    pub fn classify(s: &DnsServers) -> DnsCurrent {
        if s.is_automatic() {
            return DnsCurrent::Automatic;
        }
        for choice in [DnsChoice::Cloudflare, DnsChoice::Google] {
            let p = choice.servers();
            let v4_same = same_set(&s.v4, &p.v4);
            let v6_same = same_set(&s.v6, &p.v6);
            if v4_same && (v6_same || s.v6.is_empty()) {
                return DnsCurrent::Preset { choice, ipv6: v6_same };
            }
        }
        DnsCurrent::Custom
    }

    /// Header button text after "DNS ".
    pub fn label(&self) -> &'static str {
        match self {
            DnsCurrent::Automatic => "Automatic",
            DnsCurrent::Preset { choice, .. } => choice.label().0,
            DnsCurrent::Custom => "Custom",
        }
    }

    /// The popup's ✓ row (None for Custom: no row is ticked).
    pub fn choice(&self) -> Option<DnsChoice> {
        match self {
            DnsCurrent::Automatic => Some(DnsChoice::Automatic),
            DnsCurrent::Preset { choice, .. } => Some(*choice),
            DnsCurrent::Custom => None,
        }
    }
}

fn same_set<T: PartialEq>(a: &[T], b: &[T]) -> bool {
    a.len() == b.len() && a.iter().all(|x| b.contains(x))
}

/// DNS state of the adapter in use (for the header button and the popup).
#[derive(Debug, Clone, PartialEq)]
pub struct DnsState {
    pub adapter: AdapterId,
    pub adapter_name: String,
    /// What is set by hand (empty = automatic).
    pub configured: DnsServers,
    /// What the adapter actually uses now (DHCP or hand-set).
    pub effective: Vec<IpAddr>,
    pub current: DnsCurrent,
}
