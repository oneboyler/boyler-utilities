//! The feature crates' OS layers with their admin-only calls handed to the elevated copy. A page wraps the crate's real
//! layer in one of these; the crate's own logic (checks, snapshots, read-backs, its in-memory undo) stays as it is. Each
//! proxy says it is elevated (so the crate's own "needs admin" gate lets the change through), does every read and every
//! no-admin call itself, and sends only the admin call as one [`Op`]. A declined prompt comes back as the crate's own
//! "needs admin" error.

use std::sync::Arc;

use super::client::{Admin, RemoteOut};
use super::{AdminError, Hive, Op, Prog, Purpose, Slot, Start};

// ------------------------------------------------------------------ Tweaks (bu-toggles)

fn toggles_err(e: AdminError) -> bu_toggles::Error {
    match e {
        AdminError::Declined => bu_toggles::Error::NeedsAdmin { row: String::new() },
        other => bu_toggles::Error::Admin(other.to_string()),
    }
}

/// bu-toggles' layer: the Tweaks admin rows' registry values and USB selective suspend go to the elevated copy.
pub struct TweaksOs<O: bu_toggles::TogglesOs> {
    pub inner: O,
    admin: Arc<Admin>,
}

impl<O: bu_toggles::TogglesOs> TweaksOs<O> {
    pub fn new(inner: O, admin: Arc<Admin>) -> Self {
        TweaksOs { inner, admin }
    }
    fn hive(h: bu_toggles::os::Hive) -> Hive {
        if h == bu_toggles::os::Hive::Hklm {
            Hive::Hklm
        } else {
            Hive::Hkcu
        }
    }
}

impl<O: bu_toggles::TogglesOs> bu_toggles::TogglesOs for TweaksOs<O> {
    fn reg_read(&self, hive: bu_toggles::os::Hive, path: &str, name: &str) -> bu_toggles::Result<Option<bu_toggles::os::RegValue>> {
        self.inner.reg_read(hive, path, name)
    }
    fn reg_write(&mut self, hive: bu_toggles::os::Hive, path: &str, name: &str, value: &bu_toggles::os::RegValue) -> bu_toggles::Result<()> {
        if !super::exec::reg_allowed(Self::hive(hive), path, name, None) {
            return self.inner.reg_write(hive, path, name, value);
        }
        let bu_toggles::os::RegValue::Dword(d) = value else {
            return Err(bu_toggles::Error::Admin("That old value can't be put back".into()));
        };
        let op = Op::RegSet { hive: Self::hive(hive), path: path.into(), name: name.into(), dword: *d };
        self.admin.call(Purpose::Tweaks, op).map(|_| ()).map_err(toggles_err)
    }
    fn reg_delete_value(&mut self, hive: bu_toggles::os::Hive, path: &str, name: &str) -> bu_toggles::Result<()> {
        if !super::exec::reg_allowed(Self::hive(hive), path, name, None) {
            return self.inner.reg_delete_value(hive, path, name);
        }
        let op = Op::RegDelete { hive: Self::hive(hive), path: path.into(), name: name.into() };
        self.admin.call(Purpose::Tweaks, op).map(|_| ()).map_err(toggles_err)
    }
    fn reg_key_exists(&self, hive: bu_toggles::os::Hive, path: &str) -> bu_toggles::Result<bool> {
        self.inner.reg_key_exists(hive, path)
    }
    fn reg_create_key(&mut self, hive: bu_toggles::os::Hive, path: &str) -> bu_toggles::Result<()> {
        self.inner.reg_create_key(hive, path)
    }
    fn reg_delete_tree(&mut self, hive: bu_toggles::os::Hive, path: &str) -> bu_toggles::Result<()> {
        self.inner.reg_delete_tree(hive, path)
    }
    fn reg_values(&self, hive: bu_toggles::os::Hive, path: &str) -> bu_toggles::Result<Vec<(String, bu_toggles::os::RegValue)>> {
        self.inner.reg_values(hive, path)
    }
    fn is_elevated(&self) -> bool {
        true
    }
    fn spi_get(&self, item: bu_toggles::os::SpiItem) -> bu_toggles::Result<u32> {
        self.inner.spi_get(item)
    }
    fn spi_set(&mut self, item: bu_toggles::os::SpiItem, value: u32) -> bu_toggles::Result<()> {
        self.inner.spi_set(item, value)
    }
    fn reload_language_hotkeys(&mut self) -> bu_toggles::Result<()> {
        self.inner.reload_language_hotkeys()
    }
    fn power_read(&self, setting: bu_toggles::os::PowerSetting) -> bu_toggles::Result<bu_toggles::os::PowerValues> {
        self.inner.power_read(setting)
    }
    fn power_write(&mut self, setting: bu_toggles::os::PowerSetting, values: bu_toggles::os::PowerValues) -> bu_toggles::Result<()> {
        if setting != bu_toggles::os::PowerSetting::UsbSelectiveSuspend {
            return self.inner.power_write(setting, values);
        }
        let op = Op::UsbSuspend { ac: values.ac, dc: values.dc };
        self.admin.call(Purpose::Tweaks, op).map(|_| ()).map_err(toggles_err)
    }
    fn has_battery(&self) -> bool {
        self.inner.has_battery()
    }
    fn hibernate_on(&self) -> bu_toggles::Result<bool> {
        self.inner.hibernate_on()
    }
    fn gpu_scheduling(&self) -> bu_toggles::Result<bu_toggles::os::GpuScheduling> {
        self.inner.gpu_scheduling()
    }
    fn bluetooth(&self) -> bu_toggles::Result<Option<bool>> {
        self.inner.bluetooth()
    }
    fn set_bluetooth(&mut self, on: bool) -> bu_toggles::Result<()> {
        self.inner.set_bluetooth(on)
    }
    fn copilot_installed(&self) -> bu_toggles::Result<bool> {
        self.inner.copilot_installed()
    }
    fn remove_copilot(&mut self) -> bu_toggles::Result<()> {
        self.inner.remove_copilot()
    }
    fn broadcast_setting_change(&mut self, area: Option<&str>) -> bu_toggles::Result<()> {
        self.inner.broadcast_setting_change(area)
    }
    fn restart_explorer(&mut self) -> bu_toggles::Result<()> {
        // in THIS (normal) process: Explorer never comes back elevated
        self.inner.restart_explorer()
    }
    fn refresh_shell(&mut self) -> bu_toggles::Result<()> {
        // in THIS (normal) process, like the restart
        self.inner.refresh_shell()
    }
    fn registered_browsers(&self) -> bu_toggles::Result<Vec<bu_toggles::os::RegisteredBrowser>> {
        self.inner.registered_browsers()
    }
    fn default_browser_progid(&self) -> bu_toggles::Result<Option<String>> {
        self.inner.default_browser_progid()
    }
    fn assoc_app(&self, what: &str) -> bu_toggles::Result<Option<bu_toggles::os::AssocApp>> {
        self.inner.assoc_app(what)
    }
    fn open_uri(&mut self, uri: &str) -> bu_toggles::Result<()> {
        self.inner.open_uri(uri)
    }
    fn open_with_dialog(&mut self, ext: &str) -> bu_toggles::Result<()> {
        self.inner.open_with_dialog(ext)
    }
}

// ------------------------------------------------------------------ Network

fn net_err(e: AdminError) -> bu_network::NetError {
    match e {
        AdminError::Declined => bu_network::NetError::NeedsAdmin,
        AdminError::NotFound(s) => bu_network::NetError::NoSuchAdapter(s),
        other => bu_network::NetError::Admin(other.to_string()),
    }
}

/// bu-network's layer: the Ethernet adapter switch and the DNS servers go to the elevated copy.
pub struct NetOs {
    inner: Arc<dyn bu_network::NetworkOs>,
    admin: Arc<Admin>,
}

impl NetOs {
    pub fn new(inner: Arc<dyn bu_network::NetworkOs>, admin: Arc<Admin>) -> Self {
        NetOs { inner, admin }
    }
}

impl bu_network::NetworkOs for NetOs {
    fn adapters(&self) -> bu_network::Result<Vec<bu_network::Adapter>> {
        self.inner.adapters()
    }
    fn internet_if_index(&self) -> bu_network::Result<Option<u32>> {
        self.inner.internet_if_index()
    }
    fn is_elevated(&self) -> bool {
        true
    }
    fn set_wifi_radio(&self, on: bool) -> bu_network::Result<()> {
        self.inner.set_wifi_radio(on)
    }
    fn set_adapter_enabled(&self, id: &str, on: bool) -> bu_network::Result<()> {
        self.admin.call(Purpose::Network, Op::NetAdapter { id: id.into(), on }).map(|_| ()).map_err(net_err)
    }
    fn icmp_ping(&self, ip: std::net::IpAddr, timeout: std::time::Duration) -> bu_network::Result<std::time::Duration> {
        self.inner.icmp_ping(ip, timeout)
    }
    fn tcp_ping(&self, addr: std::net::SocketAddr, timeout: std::time::Duration) -> bu_network::Result<std::time::Duration> {
        self.inner.tcp_ping(addr, timeout)
    }
    fn udp_ping(&self, addr: std::net::SocketAddr, timeout: std::time::Duration) -> bu_network::Result<std::time::Duration> {
        self.inner.udp_ping(addr, timeout)
    }
    fn resolve(&self, host: &str) -> bu_network::Result<Vec<std::net::IpAddr>> {
        self.inner.resolve(host)
    }
    fn flush_dns(&self) -> bu_network::Result<()> {
        self.inner.flush_dns()
    }
    fn dns_servers(&self, id: &str) -> bu_network::Result<bu_network::DnsServers> {
        self.inner.dns_servers(id)
    }
    fn set_dns_servers(&self, id: &str, servers: &bu_network::DnsServers) -> bu_network::Result<()> {
        let op = Op::NetDns { id: id.into(), v4: servers.v4.clone(), v6: servers.v6.clone() };
        self.admin.call(Purpose::Network, op).map(|_| ()).map_err(net_err)
    }
    fn wifi_networks(&self) -> bu_network::Result<Vec<bu_network::WifiNetwork>> {
        self.inner.wifi_networks()
    }
    fn wifi_connect(&self, ssid: &str, password: Option<&str>, auto: bool) -> bu_network::Result<()> {
        self.inner.wifi_connect(ssid, password, auto)
    }
    fn wifi_disconnect(&self) -> bu_network::Result<()> {
        self.inner.wifi_disconnect()
    }
    fn wifi_forget(&self, ssid: &str) -> bu_network::Result<()> {
        self.inner.wifi_forget(ssid)
    }
}

// ------------------------------------------------------------------ Startup

fn startup_err(e: AdminError) -> bu_startup::OsError {
    match e {
        AdminError::Declined => bu_startup::OsError::NeedsAdmin,
        AdminError::NotFound(_) => bu_startup::OsError::NotFound,
        AdminError::Denied(_) => bu_startup::OsError::AccessDenied,
        other => bu_startup::OsError::Admin(other.to_string()),
    }
}

/// bu-startup's layer: HKLM StartupApproved flags, tasks and services go to the elevated copy.
pub struct StartupOs {
    pub inner: Arc<dyn bu_startup::StartupOs + Send + Sync>,
    admin: Arc<Admin>,
}

impl StartupOs {
    pub fn new(inner: Arc<dyn bu_startup::StartupOs + Send + Sync>, admin: Arc<Admin>) -> Self {
        StartupOs { inner, admin }
    }
    /// The HKLM StartupApproved subkey of `path`, if it is one.
    fn slot(path: &str) -> Option<Slot> {
        let rest = path.strip_prefix(bu_startup::APPROVED)?.strip_prefix('\\')?;
        [Slot::Run, Slot::Run32, Slot::StartupFolder].into_iter().find(|s| s.name().eq_ignore_ascii_case(rest))
    }
}

impl bu_startup::StartupOs for StartupOs {
    fn is_admin(&self) -> bool {
        true
    }
    fn reg_strings(&self, hive: bu_startup::Hive, view: bu_startup::RegView, path: &str) -> Result<Vec<bu_startup::os::RegString>, bu_startup::OsError> {
        self.inner.reg_strings(hive, view, path)
    }
    fn reg_binary(&self, hive: bu_startup::Hive, path: &str, name: &str) -> Result<Option<Vec<u8>>, bu_startup::OsError> {
        self.inner.reg_binary(hive, path, name)
    }
    fn reg_set_binary(&self, hive: bu_startup::Hive, path: &str, name: &str, data: &[u8]) -> Result<(), bu_startup::OsError> {
        match (hive, Self::slot(path)) {
            (bu_startup::Hive::LocalMachine, Some(slot)) => {
                self.admin.call(Purpose::Startup, Op::Approved { slot, name: name.into(), data: data.to_vec() }).map(|_| ()).map_err(startup_err)
            }
            (bu_startup::Hive::LocalMachine, None) => Err(bu_startup::OsError::NeedsAdmin),
            _ => self.inner.reg_set_binary(hive, path, name, data),
        }
    }
    fn reg_delete_value(&self, hive: bu_startup::Hive, path: &str, name: &str) -> Result<(), bu_startup::OsError> {
        match (hive, Self::slot(path)) {
            (bu_startup::Hive::LocalMachine, Some(slot)) => {
                self.admin.call(Purpose::Startup, Op::ApprovedDelete { slot, name: name.into() }).map(|_| ()).map_err(startup_err)
            }
            (bu_startup::Hive::LocalMachine, None) => Err(bu_startup::OsError::NeedsAdmin),
            _ => self.inner.reg_delete_value(hive, path, name),
        }
    }
    fn startup_folder(&self, all_users: bool) -> Result<Vec<bu_startup::os::FolderItem>, bu_startup::OsError> {
        self.inner.startup_folder(all_users)
    }
    fn store_startup_tasks(&self) -> Result<Vec<bu_startup::os::StoreStartupTask>, bu_startup::OsError> {
        self.inner.store_startup_tasks()
    }
    fn logon_tasks(&self) -> Result<Vec<bu_startup::os::RawTask>, bu_startup::OsError> {
        self.inner.logon_tasks()
    }
    fn set_task_enabled(&self, path: &str, enabled: bool) -> Result<(), bu_startup::OsError> {
        self.admin.call(Purpose::Startup, Op::Task { path: path.into(), on: enabled }).map(|_| ()).map_err(startup_err)
    }
    fn task_enabled(&self, path: &str) -> Result<bool, bu_startup::OsError> {
        self.inner.task_enabled(path)
    }
    fn services(&self, also: &[String]) -> Result<Vec<bu_startup::os::RawService>, bu_startup::OsError> {
        self.inner.services(also)
    }
    fn set_service_start(&self, name: &str, start: bu_startup::ServiceStart, delayed: bool) -> Result<(), bu_startup::OsError> {
        let start = match start {
            bu_startup::ServiceStart::Automatic => Start::Automatic,
            bu_startup::ServiceStart::Manual => Start::Manual,
            bu_startup::ServiceStart::Disabled => Start::Disabled,
            // never set by the app (a driver's start type)
            _ => return Err(bu_startup::OsError::AccessDenied),
        };
        self.admin.call(Purpose::Startup, Op::Service { name: name.into(), start, delayed }).map(|_| ()).map_err(startup_err)
    }
    fn service_start(&self, name: &str) -> Result<(bu_startup::ServiceStart, bool), bu_startup::OsError> {
        self.inner.service_start(name)
    }
    fn remembered_services(&self) -> Result<Vec<(String, bool)>, bu_startup::OsError> {
        self.inner.remembered_services()
    }
    fn remember_service(&self, name: &str, delayed: bool) -> Result<(), bu_startup::OsError> {
        // the app's own HKCU note: in this process (the user's own hive)
        self.inner.remember_service(name, delayed)
    }
    fn forget_service(&self, name: &str) -> Result<(), bu_startup::OsError> {
        self.inner.forget_service(name)
    }
    fn file_info(&self, path: &std::path::Path) -> bu_startup::os::FileInfo {
        self.inner.file_info(path)
    }
    fn expand_env(&self, s: &str) -> String {
        self.inner.expand_env(s)
    }
    fn file_exists(&self, path: &std::path::Path) -> bool {
        self.inner.file_exists(path)
    }
    fn subdirs(&self, dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        self.inner.subdirs(dir)
    }
    fn impact_reports(&self) -> Result<Vec<String>, bu_startup::OsError> {
        // a read the app never asks admin for (Startup impact only shows when the app itself runs as admin)
        self.inner.impact_reports()
    }
    fn now_filetime(&self) -> u64 {
        self.inner.now_filetime()
    }
}

// ------------------------------------------------------------------ Security

fn sec_err(e: AdminError) -> bu_security::SecurityError {
    match e {
        AdminError::Declined => bu_security::SecurityError::NeedsAdmin,
        other => bu_security::SecurityError::Admin(other.to_string()),
    }
}

/// bu-security's layer: Allow / take the Allow away / Restore / Remove / the offline scan go to the elevated copy.
pub struct SecurityOs {
    inner: Arc<dyn bu_security::SecurityOs>,
    admin: Arc<Admin>,
}

impl SecurityOs {
    pub fn new(inner: Arc<dyn bu_security::SecurityOs>, admin: Arc<Admin>) -> Self {
        SecurityOs { inner, admin }
    }
    fn call(&self, op: Op) -> bu_security::Result<()> {
        self.admin.call(Purpose::Security, op).map(|_| ()).map_err(sec_err)
    }
}

impl bu_security::SecurityOs for SecurityOs {
    fn is_elevated(&self) -> bool {
        true
    }
    fn now(&self) -> bu_security::Stamp {
        self.inner.now()
    }
    fn defender_status(&self) -> bu_security::Result<bu_security::DefenderStatus> {
        self.inner.defender_status()
    }
    fn antivirus_products(&self) -> bu_security::Result<Vec<bu_security::AvProduct>> {
        self.inner.antivirus_products()
    }
    fn detections(&self) -> bu_security::Result<Vec<bu_security::Detection>> {
        self.inner.detections()
    }
    fn threats(&self) -> bu_security::Result<Vec<bu_security::ThreatInfo>> {
        self.inner.threats()
    }
    fn path_exists(&self, path: &str) -> bool {
        self.inner.path_exists(path)
    }
    fn run_scan(&self, kind: &bu_security::ScanKind, cancel: &bu_security::CancelToken) -> bu_security::Result<bu_security::ScanExit> {
        self.inner.run_scan(kind, cancel)
    }
    fn update_definitions(&self) -> bu_security::Result<()> {
        self.inner.update_definitions()
    }
    fn start_offline_scan(&self) -> bu_security::Result<()> {
        self.call(Op::DefenderOffline)
    }
    fn remove_active_threats(&self) -> bu_security::Result<()> {
        self.call(Op::DefenderRemoveActive)
    }
    fn allow_threat(&self, threat_id: i64) -> bu_security::Result<()> {
        self.call(Op::DefenderAllow(threat_id))
    }
    fn disallow_threat(&self, threat_id: i64) -> bu_security::Result<()> {
        self.call(Op::DefenderDisallow(threat_id))
    }
    fn restore_quarantined(&self, file_path: &str) -> bu_security::Result<()> {
        // the threat id isn't passed here: the crate restores by file; the copy finds the pair in Defender's list
        let id = self
            .inner
            .detections()?
            .iter()
            .find(|d| d.resources.iter().any(|r| bu_security::resource_path(r).is_some_and(|p| p.eq_ignore_ascii_case(file_path))))
            .map(|d| d.threat_id)
            .ok_or(bu_security::SecurityError::PathMissing(file_path.into()))?;
        self.call(Op::DefenderRestore { id, file: file_path.into() })
    }
    fn open_protection_history(&self) -> bu_security::Result<()> {
        self.inner.open_protection_history()
    }
    fn allowed_threat_ids(&self) -> bu_security::Result<Vec<i64>> {
        self.inner.allowed_threat_ids()
    }
}

// ------------------------------------------------------------------ Audio

/// bu-audio's layer: switching a sound device on / off needs no admin (Windows' Sound panel does it without a prompt);
/// only if Windows says "access denied" anyway does the switch go to the elevated copy.
pub struct AudioOs<O: bu_audio::AudioOs> {
    pub inner: O,
    admin: Arc<Admin>,
}

impl<O: bu_audio::AudioOs> AudioOs<O> {
    pub fn new(inner: O, admin: Arc<Admin>) -> Self {
        AudioOs { inner, admin }
    }
}

impl<O: bu_audio::AudioOs> bu_audio::AudioOs for AudioOs<O> {
    fn devices(&mut self, flow: bu_audio::Flow) -> bu_audio::Result<Vec<bu_audio::Device>> {
        self.inner.devices(flow)
    }
    fn defaults(&mut self, flow: bu_audio::Flow) -> bu_audio::Result<bu_audio::Defaults> {
        self.inner.defaults(flow)
    }
    fn set_default(&mut self, id: &str, role: bu_audio::Role) -> bu_audio::Result<()> {
        self.inner.set_default(id, role)
    }
    fn volume(&mut self, id: &str) -> bu_audio::Result<bu_audio::VolumeMute> {
        self.inner.volume(id)
    }
    fn set_volume(&mut self, id: &str, volume: f32) -> bu_audio::Result<()> {
        self.inner.set_volume(id, volume)
    }
    fn set_mute(&mut self, id: &str, muted: bool) -> bu_audio::Result<()> {
        self.inner.set_mute(id, muted)
    }
    fn peak(&mut self, id: &str) -> bu_audio::Result<f32> {
        self.inner.peak(id)
    }
    fn set_enabled(&mut self, id: &str, on: bool) -> bu_audio::Result<()> {
        match self.inner.set_enabled(id, on) {
            Err(bu_audio::AudioError::NeedsAdmin(ctx)) => match self.admin.call(Purpose::Audio, Op::AudioEndpoint { id: id.into(), on }) {
                Ok(_) => Ok(()),
                Err(AdminError::Declined) => Err(bu_audio::AudioError::NeedsAdmin(ctx)),
                Err(AdminError::NotFound(s)) => Err(bu_audio::AudioError::NotFound(s)),
                Err(other) => Err(bu_audio::AudioError::Unavailable(other.to_string())),
            },
            r => r,
        }
    }
    fn sessions(&mut self, device_id: &str) -> bu_audio::Result<Vec<bu_audio::SessionInfo>> {
        self.inner.sessions(device_id)
    }
    fn set_session_volume(&mut self, key: &str, volume: f32) -> bu_audio::Result<()> {
        self.inner.set_session_volume(key, volume)
    }
    fn set_session_mute(&mut self, key: &str, muted: bool) -> bu_audio::Result<()> {
        self.inner.set_session_mute(key, muted)
    }
    fn session_peak(&mut self, key: &str) -> bu_audio::Result<f32> {
        self.inner.session_peak(key)
    }
    fn app_look(&mut self, s: &bu_audio::SessionInfo) -> bu_audio::AppLook {
        self.inner.app_look(s)
    }
}

// ------------------------------------------------------------------ Quick fixes

fn fix_err(e: AdminError) -> bu_quickfix::FixError {
    match e {
        AdminError::Declined => bu_quickfix::FixError::NeedsAdmin(String::new()),
        other => bu_quickfix::FixError::Refused(other.to_string()),
    }
}

/// bu-quickfix's layer: DISM / sfc and the restore point run in the elevated copy (a repair = one copy for both
/// programs: the page holds a `Repair` scope; a restore point = one `RestorePoint` scope for its reads and the call).
pub struct FixOs {
    inner: Arc<dyn bu_quickfix::FixOs>,
    admin: Arc<Admin>,
    /// one fix's scope, alive as long as this layer (a repair's thread holds it to its end, also after the menu closed)
    _scope: Option<super::client::Scope>,
}

impl FixOs {
    pub fn new(inner: Arc<dyn bu_quickfix::FixOs>, admin: Arc<Admin>) -> Self {
        FixOs { inner, admin, _scope: None }
    }
    /// The same layer for ONE fix: every admin call of it goes to one elevated copy (one prompt).
    pub fn scoped(&self, purpose: Purpose) -> FixOs {
        FixOs { inner: self.inner.clone(), admin: self.admin.clone(), _scope: Some(self.admin.scope(purpose)) }
    }
}

impl bu_quickfix::FixOs for FixOs {
    fn is_elevated(&self) -> bool {
        true
    }
    fn foreground_is_ours(&self) -> bool {
        self.inner.foreground_is_ours()
    }
    fn send_reset_chord(&self) -> bu_quickfix::Result<()> {
        self.inner.send_reset_chord()
    }
    fn display_adapters(&self) -> bu_quickfix::Result<Vec<bu_quickfix::DisplayAdapter>> {
        self.inner.display_adapters()
    }
    fn spawn(&self, program: &str, args: &[&str]) -> bu_quickfix::Result<bu_quickfix::Spawned> {
        // only DISM / sfc with their fixed arguments; anything else (pnputil) is not an admin action of the app
        let prog = Prog::of(program, args).ok_or_else(|| bu_quickfix::FixError::Refused(format!("{program} is not run as admin")))?;
        let r = self.admin.spawn(Purpose::Repair, prog).map_err(fix_err)?;
        Ok(bu_quickfix::Spawned { output: Box::new(RemoteOut(r.clone())), ctl: r })
    }
    fn cbs_log_tail(&self) -> bu_quickfix::Result<String> {
        self.inner.cbs_log_tail()
    }
    fn explorer_cache_dir(&self) -> bu_quickfix::Result<std::path::PathBuf> {
        self.inner.explorer_cache_dir()
    }
    fn list_files(&self, dir: &std::path::Path) -> bu_quickfix::Result<Vec<(String, u64)>> {
        self.inner.list_files(dir)
    }
    fn delete_file(&self, path: &std::path::Path) -> bu_quickfix::Result<()> {
        self.inner.delete_file(path)
    }
    fn stop_explorer(&self) -> bu_quickfix::Result<Box<dyn bu_quickfix::ExplorerPause>> {
        self.inner.stop_explorer()
    }
    fn restore_status(&self) -> bu_quickfix::Result<bu_quickfix::RestoreStatus> {
        // inside "Make a restore point" (its scope is open) the copy reads the newest point, which needs admin; the
        // row's line at open reads it itself (no prompt)
        if !self.admin.has_scope(Purpose::RestorePoint) {
            return self.inner.restore_status();
        }
        let f = self.admin.call(Purpose::RestorePoint, Op::RestoreStatus).map_err(fix_err)?;
        super::exec::parse_restore_status(&f).ok_or_else(|| bu_quickfix::FixError::Refused("bad answer".into()))
    }
    fn create_restore_point(&self, description: &str) -> bu_quickfix::Result<bu_quickfix::CreateCall> {
        let f = self.admin.call(Purpose::RestorePoint, Op::RestorePoint { description: description.into() }).map_err(fix_err)?;
        Ok(if f.first().map(String::as_str) == Some("protection-off") { bu_quickfix::CreateCall::ProtectionOff } else { bu_quickfix::CreateCall::Accepted })
    }
    fn now(&self) -> bu_quickfix::Stamp {
        self.inner.now()
    }
    fn local(&self, t: bu_quickfix::Stamp) -> bu_quickfix::LocalTime {
        self.inner.local(t)
    }
}
