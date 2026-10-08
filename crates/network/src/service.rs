//! The commands of the Network tab: read the connection state, switch an adapter, flush DNS, switch DNS, undo.
//! Every change returns a [`Change`] that remembers the old value; [`NetworkService::undo`] puts it back.

use std::sync::{Arc, Mutex};

use crate::error::{NetError, Result};
use crate::model::{Adapter, AdapterKind, ConnectionState, DnsChoice, DnsCurrent, DnsServers, DnsState, WifiNetwork};
use crate::os::NetworkOs;

/// A user action, for asking "does this need admin?" before doing it (the shield + "Windows asks for admin once").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Wi-Fi radio on/off - no admin (Windows.Devices.Radios).
    SwitchWifi,
    /// Device on/off - admin (Device Manager enable/disable): a wired adapter, or a Wi-Fi adapter whose device is
    /// disabled (then there is no radio to switch).
    SwitchDevice,
    /// Flush DNS - no admin (DnsFlushResolverCache).
    FlushDns,
    /// DNS switcher - admin (SetInterfaceDnsSettings).
    SetDns,
}

impl Action {
    pub fn needs_admin(self) -> bool {
        matches!(self, Action::SwitchDevice | Action::SetDns)
    }
}

/// What a change did, with the old value so it can be undone.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub id: u64,
    pub kind: ChangeKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChangeKind {
    /// Wi-Fi radio was `was_on`, now `!was_on` (or the same, if it already was).
    WifiRadio { was_on: bool, now_on: bool },
    /// Wired adapter `id` was `was_on`.
    Adapter { id: String, name: String, was_on: bool, now_on: bool },
    /// DNS on adapter `id` was `old` (hand-set servers; empty = automatic).
    Dns { id: String, name: String, old: DnsServers, new: DnsServers },
}

impl Change {
    /// The toast after the change (DESIGN §3.11 wording for DNS).
    pub fn toast(&self) -> String {
        match &self.kind {
            ChangeKind::WifiRadio { now_on, .. } => format!("Wi-Fi {}", if *now_on { "on" } else { "off" }),
            ChangeKind::Adapter { name, now_on, .. } => format!("{name} {}", if *now_on { "on" } else { "off" }),
            ChangeKind::Dns { name, new, .. } => match DnsCurrent::classify(new) {
                DnsCurrent::Automatic => "DNS back to automatic · from your router".to_string(),
                DnsCurrent::Preset { choice, .. } => {
                    format!("DNS: {} ({}) on {name}", choice.label().0, choice.label().1)
                }
                DnsCurrent::Custom => {
                    // the drawing: "DNS: 9.9.9.9 · 149.112.112.112 · IPv6 set on Ethernet"
                    let v4: Vec<String> = new.v4.iter().map(|a| a.to_string()).collect();
                    let v6 = if new.v6.is_empty() { "" } else { " · IPv6 set" };
                    format!("DNS: {}{v6} on {name}", v4.join(" · "))
                }
            },
        }
    }
}

pub struct NetworkService {
    os: Arc<dyn NetworkOs>,
    changes: Mutex<(u64, Vec<Change>)>,
}

impl NetworkService {
    pub fn new(os: Arc<dyn NetworkOs>) -> NetworkService {
        NetworkService { os, changes: Mutex::new((0, Vec::new())) }
    }

    /// The service on the real PC.
    #[cfg(windows)]
    pub fn real() -> NetworkService {
        NetworkService::new(Arc::new(crate::real::WindowsNet::new()))
    }

    pub fn os(&self) -> Arc<dyn NetworkOs> {
        self.os.clone()
    }

    /// Adapters (Wi-Fi first, then Ethernet) and which one is in use.
    pub fn connection_state(&self) -> Result<ConnectionState> {
        let mut adapters = self.os.adapters()?;
        // Ethernet, Wi-Fi, VPN, virtual, Bluetooth (the drawing's order); stable inside a kind (Windows' order).
        adapters.sort_by_key(|a| a.kind.order());
        let route = self.os.internet_if_index()?;
        // The route may run over a virtual adapter (VPN, Hyper-V switch) that is not listed; then the connected
        // physical adapter with a gateway is the one in use.
        let in_use = route.and_then(|i| {
            adapters
                .iter()
                .find(|a| a.if_index == i && a.enabled && a.connected && a.kind.physical())
                .or_else(|| adapters.iter().find(|a| a.enabled && a.connected && a.kind.physical() && !a.gateways.is_empty()))
                .map(|a| a.id.clone())
        });
        Ok(ConnectionState { adapters, in_use })
    }

    /// What the adapter's switch does, and so whether it needs admin: Wi-Fi = the radio (no admin); Ethernet, or a
    /// Wi-Fi adapter disabled in Device Manager (no radio exists then) = the device (admin).
    pub fn switch_action(adapter: &Adapter) -> Action {
        match adapter.kind {
            AdapterKind::Wifi if !adapter.device_disabled => Action::SwitchWifi,
            _ => Action::SwitchDevice,
        }
    }

    fn find_adapter(&self, id: &str) -> Result<Adapter> {
        self.os.adapters()?.into_iter().find(|a| a.id == id).ok_or_else(|| NetError::NoSuchAdapter(id.into()))
    }

    fn admin_gate(&self, action: Action) -> Result<()> {
        if action.needs_admin() && !self.os.is_elevated() {
            return Err(NetError::NeedsAdmin);
        }
        Ok(())
    }

    fn remember(&self, kind: ChangeKind) -> Change {
        let mut c = self.changes.lock().unwrap();
        c.0 += 1;
        let change = Change { id: c.0, kind };
        c.1.push(change.clone());
        change
    }

    /// The adapter's switch (see [`Self::switch_action`]). An admin switch returns Err(NeedsAdmin) and changes
    /// nothing when not elevated.
    pub fn set_adapter(&self, id: &str, on: bool) -> Result<Change> {
        let a = self.find_adapter(id)?;
        self.admin_gate(Self::switch_action(&a))?;
        let kind = self.apply_adapter(&a, on)?;
        Ok(self.remember(kind))
    }

    fn apply_adapter(&self, a: &Adapter, on: bool) -> Result<ChangeKind> {
        match Self::switch_action(a) {
            Action::SwitchWifi => {
                if a.enabled != on {
                    self.os.set_wifi_radio(on)?;
                }
                Ok(ChangeKind::WifiRadio { was_on: a.enabled, now_on: on })
            }
            _ => {
                let was_on = !a.device_disabled;
                if was_on != on {
                    self.os.set_adapter_enabled(&a.id, on)?;
                }
                Ok(ChangeKind::Adapter { id: a.id.clone(), name: a.name.clone(), was_on, now_on: on })
            }
        }
    }

    /// Device undo: straight to the device switch (whatever the adapter kind).
    fn apply_device(&self, a: &Adapter, on: bool) -> Result<ChangeKind> {
        let was_on = !a.device_disabled;
        if was_on != on {
            self.os.set_adapter_enabled(&a.id, on)?;
        }
        Ok(ChangeKind::Adapter { id: a.id.clone(), name: a.name.clone(), was_on, now_on: on })
    }

    /// Flush DNS (the ✓ "Flushed" button). No admin.
    pub fn flush_dns(&self) -> Result<()> {
        self.os.flush_dns()
    }

    /// DNS of the adapter in use. Err(Offline) when nothing is in use (the button is disabled then).
    pub fn dns_state(&self) -> Result<DnsState> {
        let conn = self.connection_state()?;
        let a = conn.in_use_adapter().ok_or(NetError::Offline)?;
        let configured = self.os.dns_servers(&a.id)?;
        Ok(DnsState {
            adapter: a.id.clone(),
            adapter_name: a.name.clone(),
            current: DnsCurrent::classify(&configured),
            configured,
            effective: a.dns_servers.clone(),
        })
    }

    /// The DNS popup pick, on the adapter in use. Admin. Writes IPv4 + IPv6, then flushes the DNS cache so the new
    /// servers are used at once.
    pub fn set_dns(&self, choice: DnsChoice) -> Result<Change> {
        self.set_dns_servers(choice.servers())
    }

    /// v21 "Custom…": your own primary / secondary for IPv4 and IPv6 ([`DnsServers::from_fields`]), on the adapter in
    /// use. Admin. An empty IPv6 list = IPv6 asks the router.
    pub fn set_dns_custom(&self, servers: DnsServers) -> Result<Change> {
        if servers.v4.is_empty() {
            return Err(NetError::Unsupported("an IPv4 server is needed".into()));
        }
        self.set_dns_servers(servers)
    }

    fn set_dns_servers(&self, new: DnsServers) -> Result<Change> {
        let state = self.dns_state()?;
        self.admin_gate(Action::SetDns)?;
        if new != state.configured {
            self.os.set_dns_servers(&state.adapter, &new)?;
            // A failed flush doesn't undo the switch: Windows drops cached answers within their TTL anyway.
            let _ = self.os.flush_dns();
        }
        Ok(self.remember(ChangeKind::Dns {
            id: state.adapter,
            name: state.adapter_name,
            old: state.configured,
            new,
        }))
    }

    /// The reset (Order 036): hand-set DNS of adapter `id` (any adapter, not only the one in use) to `servers` (empty =
    /// automatic). Admin. Writes only when different, then flushes the DNS cache.
    pub fn set_dns_on(&self, id: &str, servers: &DnsServers) -> Result<Change> {
        let a = self.find_adapter(id)?;
        self.admin_gate(Action::SetDns)?;
        let old = self.os.dns_servers(id)?;
        if &old != servers {
            self.os.set_dns_servers(id, servers)?;
            let _ = self.os.flush_dns();
        }
        Ok(self.remember(ChangeKind::Dns { id: a.id, name: a.name, old, new: servers.clone() }))
    }

    /// Changes not undone yet, oldest first.
    pub fn changes(&self) -> Vec<Change> {
        self.changes.lock().unwrap().1.clone()
    }

    /// Puts back the old value of change `id`. Admin rules are the same as for the change itself.
    /// The DNS undo writes the old servers to the adapter the change was made on, even if another adapter is in use now.
    pub fn undo(&self, id: u64) -> Result<Change> {
        let change = {
            let c = self.changes.lock().unwrap();
            c.1.iter().find(|c| c.id == id).cloned().ok_or(NetError::NothingToUndo)?
        };
        let back = match &change.kind {
            ChangeKind::WifiRadio { was_on, .. } => {
                let a = self
                    .os
                    .adapters()?
                    .into_iter()
                    .find(|a| a.kind == AdapterKind::Wifi)
                    .ok_or(NetError::NoWifiRadio)?;
                // The card may have been disabled in Device Manager meanwhile: then this is a device switch (admin).
                self.admin_gate(Self::switch_action(&a))?;
                self.apply_adapter(&a, *was_on)?
            }
            ChangeKind::Adapter { id, was_on, .. } => {
                self.admin_gate(Action::SwitchDevice)?;
                let a = self.find_adapter(id)?;
                self.apply_device(&a, *was_on)?
            }
            ChangeKind::Dns { id, name, old, .. } => {
                self.admin_gate(Action::SetDns)?;
                let now = self.os.dns_servers(id)?;
                if &now != old {
                    self.os.set_dns_servers(id, old)?;
                    let _ = self.os.flush_dns();
                }
                ChangeKind::Dns { id: id.clone(), name: name.clone(), old: now, new: old.clone() }
            }
        };
        self.changes.lock().unwrap().1.retain(|c| c.id != id);
        Ok(Change { id, kind: back })
    }

    /// Nearby Wi-Fi networks, strongest first; the connected one first of all. No admin. Reads the list Windows keeps.
    pub fn wifi_networks(&self) -> Result<Vec<WifiNetwork>> {
        let mut n = self.os.wifi_networks()?;
        n.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.signal_pct.cmp(&a.signal_pct)).then(a.ssid.cmp(&b.ssid)));
        n.dedup_by(|a, b| a.ssid == b.ssid);
        Ok(n)
    }

    /// Connect (the row's Connect, or the password field's Connect). A secured network that is not saved needs a
    /// password; nothing is sent to Windows without one.
    pub fn wifi_connect(&self, ssid: &str, password: Option<&str>, auto: bool) -> Result<()> {
        let n = self.os.wifi_networks()?.into_iter().find(|n| n.ssid == ssid).ok_or_else(|| NetError::Unreachable(ssid.into()))?;
        let password = password.filter(|p| !p.is_empty());
        if n.secured && !n.saved && password.is_none() {
            return Err(NetError::PasswordNeeded);
        }
        self.os.wifi_connect(ssid, if n.saved { None } else { password }, auto)
    }

    pub fn wifi_disconnect(&self) -> Result<()> {
        self.os.wifi_disconnect()
    }

    /// Forget a saved network: Windows deletes its profile (the password is gone). Not undoable - the page says so.
    pub fn wifi_forget(&self, ssid: &str) -> Result<()> {
        self.os.wifi_forget(ssid)
    }

    /// Undo the newest change.
    pub fn undo_last(&self) -> Result<Change> {
        let id = self.changes.lock().unwrap().1.last().map(|c| c.id).ok_or(NetError::NothingToUndo)?;
        self.undo(id)
    }
}
