//! Order 039: the admin path on fakes - every op, its argument checks (bad input refused), the system checks, the line,
//! scopes (one prompt per action), declines, the proxies. Nothing here starts an elevated copy or touches the real
//! system: the "copy" is the same helper code on a thread over the crates' fakes; the one real-pipe test talks to itself.

use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::client::{in_process, Admin, Decline, Launcher};
use super::exec::{self, Sys};
use super::wire::{self, Rx, Tx};
use super::*;

// ------------------------------------------------------------------ the fake world

/// bu-toggles' fake shared between the page's side and the "copy".
#[derive(Clone, Default)]
pub(crate) struct ST(Arc<Mutex<bu_toggles::fake::FakeOs>>);

impl ST {
    fn get(&self) -> std::sync::MutexGuard<'_, bu_toggles::fake::FakeOs> {
        self.0.lock().unwrap()
    }
}

use bu_toggles::os as tos;
use bu_toggles::TogglesOs as _;

impl bu_toggles::TogglesOs for ST {
    fn reg_read(&self, h: tos::Hive, p: &str, n: &str) -> bu_toggles::Result<Option<tos::RegValue>> {
        self.get().reg_read(h, p, n)
    }
    fn reg_write(&mut self, h: tos::Hive, p: &str, n: &str, v: &tos::RegValue) -> bu_toggles::Result<()> {
        self.get().reg_write(h, p, n, v)
    }
    fn reg_delete_value(&mut self, h: tos::Hive, p: &str, n: &str) -> bu_toggles::Result<()> {
        self.get().reg_delete_value(h, p, n)
    }
    fn reg_key_exists(&self, h: tos::Hive, p: &str) -> bu_toggles::Result<bool> {
        self.get().reg_key_exists(h, p)
    }
    fn reg_create_key(&mut self, h: tos::Hive, p: &str) -> bu_toggles::Result<()> {
        self.get().reg_create_key(h, p)
    }
    fn reg_delete_tree(&mut self, h: tos::Hive, p: &str) -> bu_toggles::Result<()> {
        self.get().reg_delete_tree(h, p)
    }
    fn reg_values(&self, h: tos::Hive, p: &str) -> bu_toggles::Result<Vec<(String, tos::RegValue)>> {
        self.get().reg_values(h, p)
    }
    fn is_elevated(&self) -> bool {
        self.get().is_elevated()
    }
    fn spi_get(&self, i: tos::SpiItem) -> bu_toggles::Result<u32> {
        self.get().spi_get(i)
    }
    fn spi_set(&mut self, i: tos::SpiItem, v: u32) -> bu_toggles::Result<()> {
        self.get().spi_set(i, v)
    }
    fn reload_language_hotkeys(&mut self) -> bu_toggles::Result<()> {
        self.get().reload_language_hotkeys()
    }
    fn power_read(&self, s: tos::PowerSetting) -> bu_toggles::Result<tos::PowerValues> {
        self.get().power_read(s)
    }
    fn power_write(&mut self, s: tos::PowerSetting, v: tos::PowerValues) -> bu_toggles::Result<()> {
        self.get().power_write(s, v)
    }
    fn has_battery(&self) -> bool {
        self.get().has_battery()
    }
    fn hibernate_on(&self) -> bu_toggles::Result<bool> {
        self.get().hibernate_on()
    }
    fn gpu_scheduling(&self) -> bu_toggles::Result<tos::GpuScheduling> {
        self.get().gpu_scheduling()
    }
    fn bluetooth(&self) -> bu_toggles::Result<Option<bool>> {
        self.get().bluetooth()
    }
    fn set_bluetooth(&mut self, on: bool) -> bu_toggles::Result<()> {
        self.get().set_bluetooth(on)
    }
    fn copilot_installed(&self) -> bu_toggles::Result<bool> {
        self.get().copilot_installed()
    }
    fn remove_copilot(&mut self) -> bu_toggles::Result<()> {
        self.get().remove_copilot()
    }
    fn broadcast_setting_change(&mut self, a: Option<&str>) -> bu_toggles::Result<()> {
        self.get().broadcast_setting_change(a)
    }
    fn restart_explorer(&mut self) -> bu_toggles::Result<()> {
        self.get().restart_explorer()
    }
    fn refresh_shell(&mut self) -> bu_toggles::Result<()> {
        self.get().refresh_shell()
    }
    fn registered_browsers(&self) -> bu_toggles::Result<Vec<tos::RegisteredBrowser>> {
        self.get().registered_browsers()
    }
    fn default_browser_progid(&self) -> bu_toggles::Result<Option<String>> {
        self.get().default_browser_progid()
    }
    fn assoc_app(&self, w: &str) -> bu_toggles::Result<Option<tos::AssocApp>> {
        self.get().assoc_app(w)
    }
    fn open_uri(&mut self, u: &str) -> bu_toggles::Result<()> {
        self.get().open_uri(u)
    }
    fn open_with_dialog(&mut self, e: &str) -> bu_toggles::Result<()> {
        self.get().open_with_dialog(e)
    }
}

const ETH: &str = "{4D36E972-E325-11CE-BFC1-08002BE10318}";
const SPK: &str = "{0.0.0.00000000}.{11111111-2222-3333-4444-555555555555}";
const MIC: &str = "{0.0.1.00000000}.{66666666-2222-3333-4444-555555555555}";

/// Everything the copy works on, as fakes; Clone = the same world (the page side and the copy see one PC).
#[derive(Clone)]
pub(crate) struct FakeSys {
    pub(crate) t: ST,
    pub(crate) net: Arc<bu_network::fake::FakeNet>,
    pub(crate) st: Arc<bu_startup::fake::FakeOs>,
    pub(crate) sec: bu_security::FakeOs,
    pub(crate) aud: Arc<Mutex<bu_audio::FakeOs>>,
    pub(crate) fix: bu_quickfix::fake::FakeFixOs,
    pub(crate) sto: Arc<bu_storage::FakeOs>,
    pub(crate) temp_cleans: Arc<AtomicUsize>,
}

struct AudRef(Arc<Mutex<bu_audio::FakeOs>>);
impl bu_audio::AudioOs for AudRef {
    fn devices(&mut self, f: bu_audio::Flow) -> bu_audio::Result<Vec<bu_audio::Device>> {
        self.0.lock().unwrap().devices(f)
    }
    fn defaults(&mut self, f: bu_audio::Flow) -> bu_audio::Result<bu_audio::Defaults> {
        self.0.lock().unwrap().defaults(f)
    }
    fn set_default(&mut self, id: &str, r: bu_audio::Role) -> bu_audio::Result<()> {
        self.0.lock().unwrap().set_default(id, r)
    }
    fn volume(&mut self, id: &str) -> bu_audio::Result<bu_audio::VolumeMute> {
        self.0.lock().unwrap().volume(id)
    }
    fn set_volume(&mut self, id: &str, v: f32) -> bu_audio::Result<()> {
        self.0.lock().unwrap().set_volume(id, v)
    }
    fn set_mute(&mut self, id: &str, m: bool) -> bu_audio::Result<()> {
        self.0.lock().unwrap().set_mute(id, m)
    }
    fn peak(&mut self, id: &str) -> bu_audio::Result<f32> {
        self.0.lock().unwrap().peak(id)
    }
    fn set_enabled(&mut self, id: &str, on: bool) -> bu_audio::Result<()> {
        self.0.lock().unwrap().set_enabled(id, on)
    }
    fn sessions(&mut self, d: &str) -> bu_audio::Result<Vec<bu_audio::SessionInfo>> {
        self.0.lock().unwrap().sessions(d)
    }
    fn set_session_volume(&mut self, k: &str, v: f32) -> bu_audio::Result<()> {
        self.0.lock().unwrap().set_session_volume(k, v)
    }
    fn set_session_mute(&mut self, k: &str, m: bool) -> bu_audio::Result<()> {
        self.0.lock().unwrap().set_session_mute(k, m)
    }
    fn session_peak(&mut self, k: &str) -> bu_audio::Result<f32> {
        self.0.lock().unwrap().session_peak(k)
    }
    fn app_look(&mut self, s: &bu_audio::SessionInfo) -> bu_audio::AppLook {
        self.0.lock().unwrap().app_look(s)
    }
}

/// The copy's side of the fake world (its audio layer is a handle to the shared fake).
struct CopySys {
    w: FakeSys,
    aud: AudRef,
}

impl Sys for CopySys {
    fn toggles(&mut self) -> Result<&mut dyn bu_toggles::TogglesOs, String> {
        Ok(&mut self.w.t)
    }
    fn net(&mut self) -> Result<Arc<dyn bu_network::NetworkOs>, String> {
        Ok(self.w.net.clone())
    }
    fn startup(&mut self) -> Result<&dyn bu_startup::StartupOs, String> {
        Ok(&*self.w.st)
    }
    fn security(&mut self) -> Result<Arc<dyn bu_security::SecurityOs>, String> {
        Ok(Arc::new(self.w.sec.clone()))
    }
    fn audio(&mut self) -> Result<&mut dyn bu_audio::AudioOs, String> {
        Ok(&mut self.aud)
    }
    fn fix(&mut self) -> Result<Arc<dyn bu_quickfix::FixOs>, String> {
        Ok(Arc::new(self.w.fix.clone()))
    }
    fn storage(&mut self) -> Result<Arc<dyn bu_storage::StorageOs>, String> {
        Ok(self.w.sto.clone())
    }
    fn clean_windows_temp(&mut self) -> Result<(u64, u64, u64, u64), String> {
        self.w.temp_cleans.fetch_add(1, Ordering::SeqCst);
        Ok((1000, 3, 200, 1))
    }
}

pub(crate) fn world() -> FakeSys {
    // the copy runs elevated: the fakes allow admin writes
    let t = ST::default();
    t.get().elevated = true;
    let net = Arc::new(bu_network::fake::FakeNet::typical());
    net.with(|s| {
        s.elevated = true;
        let i = s.adapters.iter().position(|a| a.kind == bu_network::AdapterKind::Ethernet).unwrap();
        s.adapters[i].id = ETH.into();
    });
    let st = bu_startup::fake::FakeOs::new()
        .admin(true)
        .run(bu_startup::Hive::LocalMachine, bu_startup::RegView::Bits64, bu_startup::RUN, "Vendor Tray", r"C:\Program Files\V\tray.exe")
        .folder_item(true, r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Startup", "Helper.lnk", Some(r"C:\H\h.exe"), None)
        .task(bu_startup::os::RawTask {
            path: r"\Vendor\Updater".into(),
            name: "Updater".into(),
            enabled: true,
            triggers: vec![bu_startup::TaskTrigger::Logon],
            command: None,
            arguments: None,
            author: None,
        })
        .service(raw_service("VendorSvc", bu_startup::ServiceStart::Automatic))
        .service(raw_service("disk", bu_startup::ServiceStart::Boot));
    let sec = bu_security::FakeOs::protected();
    sec.set_elevated(true);
    sec.add_threat(
        bu_security::ThreatInfo { threat_id: 77, name: "PUA:Win32/X".into(), severity: bu_security::Severity::Low, active: false },
        bu_security::Detection {
            detection_id: "d1".into(),
            threat_id: 77,
            status_id: 3,
            found: None,
            status_changed: None,
            resources: vec![r"file:_C:\Users\u\Downloads\x.exe".into()],
        },
    );
    let mut a = bu_audio::FakeOs::default();
    a.devices = vec![
        bu_audio::fake::dev(SPK, "Speakers", bu_audio::DeviceKind::Speakers, bu_audio::Flow::Output),
        bu_audio::fake::dev(MIC, "Mic", bu_audio::DeviceKind::Microphone, bu_audio::Flow::Input),
    ];
    let fix = bu_quickfix::fake::FakeFixOs::new().elevated();
    let sto = bu_storage::FakeOs::new();
    sto.add_disk(
        bu_storage::PhysicalDisk { number: 1, model: "SATA SSD".into(), media: bu_storage::MediaKind::Ssd, bus: Some("SATA".into()), size_bytes: 500 << 30 },
        bu_storage::HealthRaw {
            os_status: Some(bu_storage::OsHealthStatus::Healthy),
            temperature_c: Some(38),
            smart: vec![bu_storage::SmartAttribute { id: 5, value: 100, worst: 100, raw: 0, threshold: Some(10) }],
            reliability: Some(bu_storage::ReliabilityCounter { temperature_c: Some(38), wear_pct: Some(3), power_on_hours: Some(12345) }),
            ..Default::default()
        },
    );
    FakeSys { t, net, st: Arc::new(st), sec, aud: Arc::new(Mutex::new(a)), fix, sto: Arc::new(sto), temp_cleans: Arc::new(AtomicUsize::new(0)) }
}

fn raw_service(name: &str, start: bu_startup::ServiceStart) -> bu_startup::os::RawService {
    bu_startup::os::RawService { name: name.into(), display_name: name.into(), start, delayed: false, image_path: None }
}

/// Starts the "copy" in this process over the fake world and counts its prompts.
struct TestLauncher {
    w: FakeSys,
    launches: Arc<AtomicUsize>,
    decline: bool,
}

impl Launcher for TestLauncher {
    fn launch(&self, purpose: Purpose) -> Result<(Box<dyn Tx>, Box<dyn Rx>), AdminError> {
        self.launches.fetch_add(1, Ordering::SeqCst);
        if self.decline {
            return Err(AdminError::Declined);
        }
        let w = self.w.clone();
        Ok(in_process(purpose, Box::new(move || Box::new(CopySys { aud: AudRef(w.aud.clone()), w }) as Box<dyn Sys>)))
    }
}

pub(crate) fn hub(w: &FakeSys, decline: bool) -> (Arc<Admin>, Arc<AtomicUsize>) {
    let n = Arc::new(AtomicUsize::new(0));
    (Admin::new(Box::new(TestLauncher { w: w.clone(), launches: n.clone(), decline })), n)
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

// ------------------------------------------------------------------ shape: every op, and bad input refused

fn every_op() -> Vec<Op> {
    vec![
        Op::RegSet { hive: Hive::Hklm, path: bu_toggles::rows::HAGS_PATH.into(), name: "HwSchMode".into(), dword: 2 },
        Op::RegDelete { hive: Hive::Hkcu, path: r"Software\Policies\Microsoft\Windows\Explorer".into(), name: "DisableSearchBoxSuggestions".into() },
        Op::UsbSuspend { ac: 0, dc: 1 },
        Op::NetAdapter { id: ETH.into(), on: false },
        Op::NetDns { id: ETH.into(), v4: vec!["1.1.1.1".parse().unwrap(), "1.0.0.1".parse().unwrap()], v6: vec!["2606:4700:4700::1111".parse().unwrap()] },
        Op::NetDns { id: ETH.into(), v4: vec![], v6: vec![] },
        Op::Approved { slot: Slot::Run, name: "Vendor Tray".into(), data: [vec![3, 0, 0, 0], 0x01DC_0000_1234_5678u64.to_le_bytes().to_vec()].concat() },
        Op::ApprovedDelete { slot: Slot::StartupFolder, name: "Helper.lnk".into() },
        Op::Task { path: r"\Vendor\Updater".into(), on: false },
        Op::Service { name: "VendorSvc".into(), start: Start::Automatic, delayed: true },
        Op::Service { name: "VendorSvc".into(), start: Start::Manual, delayed: false },
        Op::Service { name: "VendorSvc".into(), start: Start::Disabled, delayed: false },
        Op::DefenderAllow(77),
        Op::DefenderDisallow(77),
        Op::DefenderRestore { id: 77, file: r"C:\Users\u\Downloads\x.exe".into() },
        Op::DefenderRemoveActive,
        Op::DefenderOffline,
        Op::AudioEndpoint { id: SPK.into(), on: true },
        Op::Spawn(Prog::Dism),
        Op::Spawn(Prog::Sfc),
        Op::Kill(3),
        Op::RestoreStatus,
        Op::RestorePoint { description: "Boyler Utilities \u{b7} 7 Oct 2026".into() },
        Op::CleanWindowsTemp,
        Op::DiskHealth(1),
    ]
}

#[test]
fn every_op_survives_its_wire_form() {
    for op in every_op() {
        assert_eq!(Op::parse(&op.fields()), Ok(op.clone()), "{op:?}");
    }
}

#[test]
fn bad_input_is_refused() {
    let guid = ETH;
    let bad: Vec<Vec<String>> = vec![
        s(&[]),
        s(&["format-c"]),
        s(&["cmd", "/c", "del"]),
        s(&["reg-set", "hkcr", "x", "y", "1"]),
        s(&["reg-set", "hklm", r"A\\B", "y", "1"]),
        s(&["reg-set", "hklm", r"\A\B", "y", "1"]),
        s(&["reg-set", "hklm", "A\nB", "y", "1"]),
        s(&["reg-set", "hklm", "A", "y", "-1"]),
        s(&["reg-set", "hklm", "A", "y", "+1"]),
        s(&["reg-set", "hklm", "A", "y", "1", "extra"]),
        s(&["reg-set", "hklm", "A", ""]),
        s(&["usb-suspend", "2", "0"]),
        s(&["usb-suspend", "1"]),
        s(&["net-adapter", "{ETH-0001}", "on"]),
        s(&["net-adapter", guid, "yes"]),
        s(&["net-adapter", &format!("{guid} "), "on"]),
        s(&["net-dns", guid, "1.1.1.1;calc", "-"]),
        s(&["net-dns", guid, "1.1.1.1,1.1.1.2,1.1.1.3,1.1.1.4,1.1.1.5", "-"]),
        s(&["net-dns", guid, "-", "1.1.1.1"]),
        s(&["approved", "Run", "x", "0200000000000000000000"]),
        s(&["approved", "Run", "x", "020000000100000000000000"]),
        s(&["approved", "Run", "x", "040000000000000000000000"]),
        s(&["approved", "Run", "x", "zz0000000000000000000000"]),
        s(&["approved", "RunOnce", "x", "020000000000000000000000"]),
        s(&["task", r"Vendor\Updater", "on"]),
        s(&["task", r"\Vendor\..\Microsoft\Windows\Defrag", "on"]),
        s(&["task", r"\Vendor\", "on"]),
        s(&["task", "\\Vendor/Updater", "on"]),
        s(&["service", r"..\evil", "manual", "off"]),
        s(&["service", "x", "boot", "off"]),
        s(&["service", "x", "system", "off"]),
        s(&["service", "x", "manual", "on"]),
        s(&["defender-allow", "0"]),
        s(&["defender-allow", "-5"]),
        s(&["defender-allow", "1e3"]),
        s(&["defender-allow", "99999999999999999999999"]),
        s(&["defender-restore", "77", ""]),
        s(&["defender-offline", "now"]),
        s(&["audio-endpoint", "{0.0.2.00000000}.{11111111-2222-3333-4444-555555555555}", "on"]),
        s(&["audio-endpoint", "speakers", "on"]),
        s(&["spawn", "cmd"]),
        s(&["spawn", "dism", "/Online", "/Remove-Package"]),
        s(&["kill", "x"]),
        s(&["restore-point", "Evil point"]),
        s(&["restore-point", &format!("Boyler Utilities \u{b7} {}", "x".repeat(60))]),
        s(&["disk-health", "128"]),
        s(&["disk-health", "C:"]),
        s(&["clean-windows-temp", r"C:\Users"]),
    ];
    for b in bad {
        assert!(Op::parse(&b).is_err(), "accepted {b:?}");
    }
}

#[test]
fn each_purpose_allows_only_its_ops() {
    let ops = every_op();
    let allowed = |p: Purpose| ops.iter().filter(|o| p.allows(o)).map(|o| o.fields()[0].clone()).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(allowed(Purpose::Repair), ["kill", "spawn"].iter().map(|x| x.to_string()).collect());
    assert_eq!(allowed(Purpose::RestorePoint), ["restore-point", "restore-status"].iter().map(|x| x.to_string()).collect());
    assert_eq!(allowed(Purpose::Storage), ["clean-windows-temp", "disk-health"].iter().map(|x| x.to_string()).collect());
    assert_eq!(allowed(Purpose::Network), ["net-adapter", "net-dns"].iter().map(|x| x.to_string()).collect());
    // the reset puts settings back - never a scan, a restore, a removal, a program, a clean-up
    let reset = allowed(Purpose::Reset);
    for no in ["defender-offline", "defender-restore", "defender-remove-active", "spawn", "kill", "clean-windows-temp", "restore-point", "disk-health"] {
        assert!(!reset.contains(no), "{no}");
    }
    for yes in ["reg-set", "reg-delete", "usb-suspend", "net-adapter", "net-dns", "approved", "task", "service", "defender-allow", "defender-disallow", "audio-endpoint"] {
        assert!(reset.contains(yes), "{yes}");
    }
    assert!(!Purpose::Tweaks.allows(&Op::NetAdapter { id: ETH.into(), on: true }));
    assert!(Purpose::ALL.iter().all(|p| Purpose::parse(p.name()) == Some(*p)));
}

#[test]
fn the_command_line_must_be_exact() {
    let id = "0123456789abcdef0123456789abcdef";
    assert_eq!(helper::parse_args(&s(&["app", ARG, "tweaks", id])), Some((Purpose::Tweaks, id.into())));
    assert_eq!(helper::run_if_requested(&s(&["app", "--open"])), None);
    assert_eq!(helper::run_if_requested(&s(&["app"])), None);
    for bad in [
        s(&["app", ARG]),
        s(&["app", ARG, "tweaks"]),
        s(&["app", ARG, "everything", id]),
        s(&["app", ARG, "tweaks", "0123456789ABCDEF0123456789ABCDEF"]),
        s(&["app", ARG, "tweaks", "..\\..\\pipe"]),
        s(&["app", ARG, "tweaks", id, "reg-set"]),
    ] {
        assert_eq!(helper::run_if_requested(&bad), Some(helper::CODE_BAD_ARGS), "{bad:?}");
    }
}

#[test]
fn frames_are_checked() {
    let f: Vec<&[u8]> = vec![b"op", b"1", b"", &[0xff, 0x00]];
    let bytes = wire::encode(&f).unwrap();
    let back = wire::read_frame(&mut &bytes[..]).unwrap().unwrap();
    assert_eq!(back, f.iter().map(|x| x.to_vec()).collect::<Vec<_>>());
    assert!(wire::texts(&back).is_none(), "not UTF-8");
    // a field longer than its frame, a truncated length
    assert!(wire::decode_body(&[9, 0, 0, 0, b'a']).is_err());
    assert!(wire::decode_body(&[1, 0]).is_err());
    assert!(wire::read_frame(&mut &[0xff, 0xff, 0xff, 0x7f][..]).is_err(), "too big");
    assert_eq!(wire::read_frame(&mut &[][..]).unwrap(), None, "clean end");
    assert!(wire::check_id("0123456789abcdef0123456789abcdef"));
    assert!(!wire::check_id("0123456789abcdef0123456789abcde"));
    assert!(wire::check_sid("S-1-5-21-1-2-3-1001"));
    assert!(!wire::check_sid(r"S-1-5-21-1\..\x"));
    assert!(!wire::check_sid("S-1-"));
}

#[test]
fn only_the_tweaks_admin_rows_values_are_writable() {
    use exec::reg_allowed;
    let hags = bu_toggles::rows::HAGS_PATH;
    assert!(reg_allowed(Hive::Hklm, hags, "HwSchMode", Some(1)));
    assert!(reg_allowed(Hive::Hklm, hags, "HwSchMode", Some(2)));
    assert!(reg_allowed(Hive::Hklm, &hags.to_lowercase(), "hwschmode", Some(2)));
    assert!(!reg_allowed(Hive::Hklm, hags, "HwSchMode", Some(3)));
    assert!(!reg_allowed(Hive::Hkcu, hags, "HwSchMode", Some(2)), "wrong hive");
    assert!(reg_allowed(Hive::Hklm, bu_toggles::rows::HIBERBOOT_PATH, "HiberbootEnabled", Some(0)));
    assert!(reg_allowed(Hive::Hkcu, r"Software\Policies\Microsoft\Windows\Explorer", "DisableSearchBoxSuggestions", Some(1)));
    assert!(reg_allowed(Hive::Hkcu, r"Software\Policies\Microsoft\Windows\Explorer", "DisableSearchBoxSuggestions", None));
    assert!(reg_allowed(Hive::Hklm, r"SOFTWARE\Policies\Microsoft\Dsh", "AllowNewsAndInterests", Some(0)));
    assert!(reg_allowed(Hive::Hklm, r"SOFTWARE\Policies\Microsoft\Windows\Explorer", "HideRecommendedSection", Some(1)));
    assert!(reg_allowed(Hive::Hklm, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\LogonUI\BootAnimation", "DisableStartupSound", Some(1)));
    // a row that needs no admin, and anything else
    assert!(!reg_allowed(Hive::Hkcu, bu_toggles::rows::ADV, "TaskbarAl", Some(0)));
    assert!(!reg_allowed(Hive::Hklm, r"SYSTEM\CurrentControlSet\Services\WinDefend", "Start", Some(4)));
    assert!(!reg_allowed(Hive::Hklm, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon", "Shell", None));
    // exactly the 7 rows of the order (Hibernate is gone, Order 040)
    let rows: std::collections::BTreeSet<&str> = bu_toggles::rows::ROWS.iter().filter(|r| r.needs_admin()).map(|r| r.id).collect();
    assert_eq!(
        rows,
        ["fast_startup", "gpu_scheduling", "start_recommended_section", "startup_sound", "usb_power_saving", "web_results_in_search", "widgets"].into_iter().collect()
    );
}

#[test]
fn temp_paths_stay_inside() {
    use exec::temp::inside;
    let root = r"\\?\C:\Windows\Temp";
    assert!(inside(r"\\?\C:\Windows\Temp\a.tmp", root));
    assert!(inside(r"\\?\c:\windows\temp\x\y.log", root));
    assert!(!inside(root, root));
    assert!(!inside(r"\\?\C:\Windows\Temp2\a", root));
    assert!(!inside(r"\\?\C:\Windows\System32\drivers\x.sys", root));
    assert!(!inside(r"\\?\D:\Windows\Temp\a", root));
}

#[test]
fn health_and_restore_status_survive_the_line() {
    let h = bu_storage::HealthRaw {
        os_status: Some(bu_storage::OsHealthStatus::Warning),
        nvme: Some(bu_storage::NvmeHealthLog { critical_warning: 1, temperature_kelvin: 310, available_spare_pct: 99, available_spare_threshold_pct: 10, percentage_used: 4, power_on_hours: 77, media_errors: 0, unsafe_shutdowns: 12 }),
        temperature_c: Some(37),
        smart: vec![bu_storage::SmartAttribute { id: 194, value: 64, worst: 40, raw: 36, threshold: None }],
        reliability: Some(bu_storage::ReliabilityCounter { temperature_c: None, wear_pct: Some(2), power_on_hours: None }),
        needs_admin: vec!["SATA SMART attributes".into()],
    };
    assert_eq!(exec::parse_health(&exec::health_fields(&h)), Some(h));
    assert_eq!(exec::parse_health(&s(&["smart=1,2,3"])), None);
    let st = bu_quickfix::RestoreStatus {
        frequency_minutes: 1440,
        newest_known: true,
        newest: Some(bu_quickfix::RestorePoint { created: bu_quickfix::Stamp(1_790_000_000), description: "Boyler Utilities \u{b7} 1 Oct 2026".into(), sequence: 9 }),
    };
    assert_eq!(exec::parse_restore_status(&exec::restore_status_fields(&st)), Some(st));
}

// ------------------------------------------------------------------ the copy: system checks and the one Windows call

#[test]
fn the_copy_checks_every_argument_against_the_system() {
    let w = world();
    let (a, _) = hub(&w, false);
    let call = |p: Purpose, op: Op| a.call(p, op);
    // Tweaks
    assert_eq!(call(Purpose::Tweaks, Op::RegSet { hive: Hive::Hklm, path: bu_toggles::rows::HAGS_PATH.into(), name: "HwSchMode".into(), dword: 2 }), Ok(vec![]));
    assert_eq!(w.t.get().reg_read(tos::Hive::Hklm, bu_toggles::rows::HAGS_PATH, "HwSchMode").unwrap(), Some(tos::RegValue::Dword(2)));
    assert!(matches!(call(Purpose::Tweaks, Op::RegSet { hive: Hive::Hklm, path: r"SYSTEM\Setup".into(), name: "CmdLine".into(), dword: 1 }), Err(AdminError::Refused(_))));
    assert_eq!(call(Purpose::Tweaks, Op::UsbSuspend { ac: 0, dc: 0 }), Ok(vec![]));
    assert_eq!(w.t.get().power[&tos::PowerSetting::UsbSelectiveSuspend], tos::PowerValues { ac: 0, dc: 0 });
    // a Tweaks copy refuses a network op
    assert!(matches!(call(Purpose::Tweaks, Op::NetAdapter { id: ETH.into(), on: false }), Err(AdminError::Refused(_))));
}

#[test]
fn a_copy_refuses_ops_of_another_purpose() {
    let w = world();
    let (a, n) = hub(&w, false);
    let _scope = a.scope(Purpose::Tweaks);
    // the Tweaks copy is open; a network op of the page asks with its own purpose → a copy of its own
    assert_eq!(a.call(Purpose::Network, Op::NetAdapter { id: ETH.into(), on: false }), Ok(vec![]));
    assert_eq!(n.load(Ordering::SeqCst), 1);
    // straight to a Tweaks copy, a network op is refused by the copy itself
    let (tx, rx) = in_process(Purpose::Tweaks, Box::new({
        let w = w.clone();
        move || Box::new(CopySys { aud: AudRef(w.aud.clone()), w }) as Box<dyn Sys>
    }));
    let sess = client::Session::new(tx, rx);
    assert!(matches!(sess.call(&Op::NetAdapter { id: ETH.into(), on: true }), Err(AdminError::Refused(m)) if m.contains("not part of tweaks")));
    assert!(matches!(sess.call(&Op::Spawn(Prog::Dism)), Err(AdminError::Refused(_))));
}

#[test]
fn network_startup_security_audio_storage_checks() {
    let w = world();
    let (a, _) = hub(&w, false);
    // network: only an adapter Windows lists
    assert!(matches!(a.call(Purpose::Network, Op::NetAdapter { id: "{00000000-0000-0000-0000-000000000000}".into(), on: false }), Err(AdminError::NotFound(_))));
    assert_eq!(a.call(Purpose::Network, Op::NetDns { id: ETH.into(), v4: vec!["9.9.9.9".parse().unwrap()], v6: vec![] }), Ok(vec![]));
    assert_eq!(w.net.with(|s| s.dns.get(ETH).cloned()).unwrap().v4, vec!["9.9.9.9".parse::<std::net::Ipv4Addr>().unwrap()]);
    // startup: a real HKLM Run value / all-users file, a listed task, an existing non-driver service
    let off = [vec![3u8, 0, 0, 0], 5u64.to_le_bytes().to_vec()].concat();
    assert_eq!(a.call(Purpose::Startup, Op::Approved { slot: Slot::Run, name: "Vendor Tray".into(), data: off.clone() }), Ok(vec![]));
    assert_eq!(w.st.get_binary(bu_startup::Hive::LocalMachine, &format!(r"{}\Run", bu_startup::APPROVED), "Vendor Tray"), Some(off.clone()));
    assert!(matches!(a.call(Purpose::Startup, Op::Approved { slot: Slot::Run, name: "Not There".into(), data: off.clone() }), Err(AdminError::NotFound(_))));
    assert!(matches!(a.call(Purpose::Startup, Op::Approved { slot: Slot::Run32, name: "Vendor Tray".into(), data: off.clone() }), Err(AdminError::NotFound(_))), "the 32-bit view has no such value");
    assert_eq!(a.call(Purpose::Startup, Op::ApprovedDelete { slot: Slot::StartupFolder, name: "Helper.lnk".into() }), Ok(vec![]));
    assert_eq!(a.call(Purpose::Startup, Op::Task { path: r"\Vendor\Updater".into(), on: false }), Ok(vec![]));
    assert!(matches!(a.call(Purpose::Startup, Op::Task { path: r"\Microsoft\Windows\Defrag\ScheduledDefrag".into(), on: false }), Err(AdminError::NotFound(_))));
    assert_eq!(a.call(Purpose::Startup, Op::Service { name: "VendorSvc".into(), start: Start::Manual, delayed: false }), Ok(vec![]));
    assert!(matches!(a.call(Purpose::Startup, Op::Service { name: "disk".into(), start: Start::Disabled, delayed: false }), Err(AdminError::Refused(_))), "a driver");
    // security: only ids / files Defender knows
    assert!(matches!(a.call(Purpose::Security, Op::DefenderAllow(12345)), Err(AdminError::NotFound(_))));
    assert_eq!(a.call(Purpose::Security, Op::DefenderAllow(77)), Ok(vec![]));
    assert_eq!(a.call(Purpose::Security, Op::DefenderDisallow(77)), Ok(vec![]));
    assert!(matches!(a.call(Purpose::Security, Op::DefenderDisallow(77)), Err(AdminError::NotFound(_))), "no longer allowed");
    assert!(matches!(a.call(Purpose::Security, Op::DefenderRestore { id: 77, file: r"C:\Windows\System32\cmd.exe".into() }), Err(AdminError::NotFound(_))));
    assert!(matches!(a.call(Purpose::Security, Op::DefenderRestore { id: 78, file: r"C:\Users\u\Downloads\x.exe".into() }), Err(AdminError::NotFound(_))));
    assert_eq!(a.call(Purpose::Security, Op::DefenderRestore { id: 77, file: r"c:\users\u\downloads\X.EXE".into() }), Ok(vec![]));
    assert!(w.sec.state().log.iter().any(|l| l.contains(r"C:\Users\u\Downloads\x.exe")), "restored by Defender's own spelling: {:?}", w.sec.state().log);
    // audio: only a listed device
    assert!(matches!(a.call(Purpose::Audio, Op::AudioEndpoint { id: "{0.0.0.00000000}.{99999999-2222-3333-4444-555555555555}".into(), on: false }), Err(AdminError::NotFound(_))));
    assert_eq!(a.call(Purpose::Audio, Op::AudioEndpoint { id: MIC.into(), on: false }), Ok(vec![]));
    // storage: only a listed disk; the health comes back whole
    assert!(matches!(a.call(Purpose::Storage, Op::DiskHealth(0)), Err(AdminError::NotFound(_))));
    let h = a.call(Purpose::Storage, Op::DiskHealth(1)).unwrap();
    assert_eq!(exec::parse_health(&h).unwrap().reliability.unwrap().power_on_hours, Some(12345));
    assert_eq!(a.call(Purpose::Storage, Op::CleanWindowsTemp), Ok(s(&["1000", "3", "200", "1"])));
    // restore point
    assert_eq!(a.call(Purpose::RestorePoint, Op::RestorePoint { description: "Boyler Utilities \u{b7} 7 Oct 2026".into() }), Ok(s(&["accepted"])));
    assert!(exec::parse_restore_status(&a.call(Purpose::RestorePoint, Op::RestoreStatus).unwrap()).is_some());
}

#[test]
fn programs_stream_back_and_cancel() {
    let w = world();
    w.fix.script("dism.exe", bu_quickfix::fake::Script::new(vec![b"[==  10.0%  ]\r".to_vec(), b"The operation completed successfully.\r\n".to_vec()], 0));
    w.fix.script("sfc.exe", bu_quickfix::fake::Script::new(vec![bu_quickfix::fake::utf16("Verification 5% complete.\r")], 0).hanging());
    let (a, n) = hub(&w, false);
    let _scope = a.scope(Purpose::Repair);
    let d = a.spawn(Purpose::Repair, Prog::Dism).unwrap();
    let mut text = Vec::new();
    client::RemoteOut(d.clone()).read_to_end(&mut text).unwrap();
    assert_eq!(String::from_utf8_lossy(&text), "[==  10.0%  ]\rThe operation completed successfully.\r\n");
    assert_eq!(bu_quickfix::ProcCtl::wait(&*d).unwrap(), 0);
    // sfc in the same copy (one prompt for the whole repair), then Cancel
    let sfc = a.spawn(Purpose::Repair, Prog::Sfc).unwrap();
    w.fix.wait_running(1);
    bu_quickfix::ProcCtl::kill(&*sfc);
    let _ = bu_quickfix::ProcCtl::wait(&*sfc);
    assert_eq!(w.fix.killed(), 1);
    assert_eq!(n.load(Ordering::SeqCst), 1);
    assert_eq!(w.fix.spawned().iter().map(|(p, a)| format!("{p} {}", a.join(" "))).collect::<Vec<_>>(), ["dism.exe /Online /Cleanup-Image /RestoreHealth", "sfc.exe /scannow"]);
}

// ------------------------------------------------------------------ prompts: one per action, a No ends it

#[test]
fn one_prompt_per_scope_and_one_per_op_without() {
    let w = world();
    let (a, n) = hub(&w, false);
    {
        let _s = a.scope(Purpose::Startup);
        for on in [false, true, false] {
            a.call(Purpose::Startup, Op::Task { path: r"\Vendor\Updater".into(), on }).unwrap();
        }
    }
    assert_eq!(n.load(Ordering::SeqCst), 1);
    a.call(Purpose::Startup, Op::Task { path: r"\Vendor\Updater".into(), on: true }).unwrap();
    a.call(Purpose::Startup, Op::Task { path: r"\Vendor\Updater".into(), on: false }).unwrap();
    assert_eq!(n.load(Ordering::SeqCst), 3);
}

#[test]
fn the_reset_takes_every_page_in_one_prompt() {
    let w = world();
    let (a, n) = hub(&w, false);
    let _s = a.scope(Purpose::Reset);
    a.call(Purpose::Tweaks, Op::RegSet { hive: Hive::Hklm, path: bu_toggles::rows::HAGS_PATH.into(), name: "HwSchMode".into(), dword: 1 }).unwrap();
    a.call(Purpose::Network, Op::NetAdapter { id: ETH.into(), on: false }).unwrap();
    a.call(Purpose::Startup, Op::Service { name: "VendorSvc".into(), start: Start::Automatic, delayed: false }).unwrap();
    a.call(Purpose::Security, Op::DefenderAllow(77)).unwrap();
    a.call(Purpose::Audio, Op::AudioEndpoint { id: SPK.into(), on: true }).unwrap();
    assert_eq!(n.load(Ordering::SeqCst), 1);
    // an op the reset never does goes to a copy of its own purpose
    a.call(Purpose::Security, Op::DefenderOffline).unwrap();
    assert_eq!(n.load(Ordering::SeqCst), 2);
}

#[test]
fn a_no_ends_the_scope_without_a_second_prompt() {
    let w = world();
    let (a, n) = hub(&w, true);
    let s = a.scope(Purpose::Reset);
    for _ in 0..3 {
        assert_eq!(a.call(Purpose::Tweaks, Op::UsbSuspend { ac: 1, dc: 1 }), Err(AdminError::Declined));
    }
    assert_eq!(n.load(Ordering::SeqCst), 1);
    drop(s);
    // a new action asks again
    assert_eq!(a.call(Purpose::Tweaks, Op::UsbSuspend { ac: 1, dc: 1 }), Err(AdminError::Declined));
    assert_eq!(n.load(Ordering::SeqCst), 2);
    assert_eq!(AdminError::Declined.to_string(), "Needs admin \u{2014} not changed");
}

#[test]
fn the_default_hub_never_prompts() {
    // a unit test (and a test copy of the app) gets the Decline launcher
    assert_eq!(client::admin().call(Purpose::Tweaks, Op::UsbSuspend { ac: 1, dc: 1 }), Err(AdminError::Declined));
    let d: Box<dyn Launcher> = Box::new(Decline);
    assert!(d.launch(Purpose::Reset).is_err());
}

// ------------------------------------------------------------------ the proxies: the crates' own logic, the admin call in the copy

#[test]
fn tweaks_rows_go_through_the_copy() {
    let w = world();
    let (a, n) = hub(&w, false);
    let mut t = bu_toggles::Toggles::new(proxy::TweaksOs::new(w.t.clone(), a.clone()));
    t.set("gpu_scheduling", true).unwrap();
    assert_eq!(w.t.get().reg_read(tos::Hive::Hklm, bu_toggles::rows::HAGS_PATH, "HwSchMode").unwrap(), Some(tos::RegValue::Dword(2)));
    let ap = t.set("widgets", false).unwrap();
    assert!(ap.explorer_restarted, "Explorer restarts in the app itself, after the copy wrote the policy");
    t.set("usb_power_saving", false).unwrap();
    assert_eq!(n.load(Ordering::SeqCst), 3, "one prompt per switch");
    // a no-admin row never asks
    t.set("show_file_extensions", true).unwrap();
    assert_eq!(n.load(Ordering::SeqCst), 3);
    // declined: the crate's own "needs admin", nothing changed
    let (d, _) = hub(&w, true);
    let mut t = bu_toggles::Toggles::new(proxy::TweaksOs::new(w.t.clone(), d));
    assert!(matches!(t.set("gpu_scheduling", false), Err(bu_toggles::Error::NeedsAdmin { row }) if row == "gpu_scheduling"));
    assert_eq!(w.t.get().reg_read(tos::Hive::Hklm, bu_toggles::rows::HAGS_PATH, "HwSchMode").unwrap(), Some(tos::RegValue::Dword(2)));
}

#[test]
fn network_goes_through_the_copy() {
    let w = world();
    let (a, n) = hub(&w, false);
    let svc = bu_network::NetworkService::new(Arc::new(proxy::NetOs::new(w.net.clone(), a)));
    svc.set_adapter(ETH, false).unwrap();
    assert!(w.net.with(|s| s.log.iter().any(|l| l == &format!("adapter {ETH} off"))), "{:?}", w.net.with(|s| s.log.clone()));
    assert_eq!(n.load(Ordering::SeqCst), 1);
    let (d, _) = hub(&w, true);
    let svc = bu_network::NetworkService::new(Arc::new(proxy::NetOs::new(w.net.clone(), d)));
    assert!(matches!(svc.set_adapter(ETH, true), Err(bu_network::NetError::NeedsAdmin)));
}

#[test]
fn startup_goes_through_the_copy() {
    let w = world();
    let (a, n) = hub(&w, false);
    let p = proxy::StartupOs::new(w.st.clone(), a);
    use bu_startup::StartupOs as _;
    p.set_task_enabled(r"\Vendor\Updater", false).unwrap();
    p.set_service_start("VendorSvc", bu_startup::ServiceStart::Manual, false).unwrap();
    p.reg_set_binary(bu_startup::Hive::LocalMachine, &format!(r"{}\Run", bu_startup::APPROVED), "Vendor Tray", &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(n.load(Ordering::SeqCst), 3);
    // any other HKLM write is refused in the app (never sent), the user's own entries need no copy
    assert_eq!(p.reg_set_binary(bu_startup::Hive::LocalMachine, r"Software\Microsoft\Windows\CurrentVersion\Run", "x", &[1]), Err(bu_startup::OsError::NeedsAdmin));
    p.reg_set_binary(bu_startup::Hive::CurrentUser, &format!(r"{}\Run", bu_startup::APPROVED), "Mine", &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(n.load(Ordering::SeqCst), 3);
    let (d, _) = hub(&w, true);
    let p = proxy::StartupOs::new(w.st.clone(), d);
    assert_eq!(p.set_task_enabled(r"\Vendor\Updater", true), Err(bu_startup::OsError::NeedsAdmin));
}

#[test]
fn a_restore_is_one_prompt_for_the_allow_and_the_file() {
    let w = world();
    let (a, n) = hub(&w, false);
    let svc = bu_security::SecurityService::new(Arc::new(proxy::SecurityOs::new(Arc::new(w.sec.clone()), a.clone())));
    {
        let _s = a.scope(Purpose::Security);
        svc.restore_quarantined(77, r"C:\Users\u\Downloads\x.exe").unwrap();
    }
    assert_eq!(n.load(Ordering::SeqCst), 1);
    assert!(w.sec.state().allowed.contains(&77));
    let (d, _) = hub(&w, true);
    let svc = bu_security::SecurityService::new(Arc::new(proxy::SecurityOs::new(Arc::new(w.sec.clone()), d)));
    assert!(matches!(svc.remove_allow(77), Err(bu_security::SecurityError::NeedsAdmin)));
}

#[test]
fn a_sound_device_asks_only_when_windows_says_access_denied() {
    let w = world();
    let (a, n) = hub(&w, false);
    let mut p = proxy::AudioOs::new(AudRef(w.aud.clone()), a);
    use bu_audio::AudioOs as _;
    p.set_enabled(MIC, false).unwrap();
    assert_eq!(n.load(Ordering::SeqCst), 0, "no admin needed: no prompt");
    w.aud.lock().unwrap().enable_needs_admin = true;
    // the copy's own fake says access denied too here (one world): the copy's answer comes back as a plain failure
    let r = p.set_enabled(MIC, true);
    assert_eq!(n.load(Ordering::SeqCst), 1);
    assert!(r.is_err());
}

#[test]
fn a_repair_is_one_prompt_for_dism_and_sfc() {
    let w = world();
    w.fix.script("dism.exe", bu_quickfix::fake::Script::new(vec![b"The restore operation completed successfully.\r\n".to_vec()], 0));
    w.fix.script("sfc.exe", bu_quickfix::fake::Script::new(vec![bu_quickfix::fake::utf16("Windows Resource Protection did not find any integrity violations.\r\n")], 0));
    let (a, n) = hub(&w, false);
    let os: Arc<dyn bu_quickfix::FixOs> = Arc::new(proxy::FixOs::new(Arc::new(bu_quickfix::fake::FakeFixOs::new()), a.clone()));
    let scope = a.scope(Purpose::Repair);
    let rep = bu_quickfix::repair::RepairRun::start(os, |_| {}).unwrap().wait();
    drop(scope);
    assert_eq!(rep.outcome, bu_quickfix::repair::RepairOutcome::NoProblems, "{rep:?}");
    assert_eq!(n.load(Ordering::SeqCst), 1);
    // a restore point: its reads and the call in one copy
    let os = proxy::FixOs::new(Arc::new(bu_quickfix::fake::FakeFixOs::new()), a.clone());
    let s = a.scope(Purpose::RestorePoint);
    let o = bu_quickfix::restore::make_restore_point(&os).unwrap();
    drop(s);
    assert!(matches!(o, bu_quickfix::restore::RestoreOutcome::Made { .. }), "{o:?}");
    assert_eq!(n.load(Ordering::SeqCst), 2);
    // other programs never go to the copy
    assert!(bu_quickfix::FixOs::spawn(&os, "pnputil.exe", &["/restart-device", "x"]).is_err());
}

// ------------------------------------------------------------------ the real pipes (this process talks to itself; nothing elevated)

#[test]
fn the_pipes_carry_frames_and_check_who_is_on_the_other_end() {
    let server = wire::Server::create().unwrap();
    let id = server.id().to_string();
    let t = std::thread::spawn(move || wire::connect(&id));
    let me = unsafe { windows::Win32::System::Threading::GetCurrentProcess() };
    let (tx, mut rx) = server.accept(me, std::time::Duration::from_secs(10), &|| false).unwrap();
    let (mut crx, ctx, sid) = t.join().unwrap().unwrap();
    assert!(wire::check_sid(&sid));
    tx.send(&[b"op", b"1", b"restore-status"]).unwrap();
    assert_eq!(crx.recv().unwrap().unwrap(), vec![b"op".to_vec(), b"1".to_vec(), b"restore-status".to_vec()]);
    ctx.send(&[b"ok", b"1"]).unwrap();
    assert_eq!(rx.recv().unwrap().unwrap(), vec![b"ok".to_vec(), b"1".to_vec()]);
    drop(tx);
    assert_eq!(crx.recv().unwrap(), None, "closing the line ends the copy's loop");
    // a pipe id that is not ours is refused before anything is opened
    assert!(wire::connect(r"..\..\other").is_err());
}

#[test]
fn a_client_that_is_not_the_copy_is_refused() {
    let server = wire::Server::create().unwrap();
    let id = server.id().to_string();
    let t = std::thread::spawn(move || wire::connect(&id));
    // the started "copy" is another process (a short hidden ping); this process connects instead: refused
    let mut child = std::process::Command::new("ping").args(["-n", "5", "127.0.0.1"]).stdout(std::process::Stdio::null()).spawn().unwrap();
    use std::os::windows::io::AsRawHandle;
    let h = windows::Win32::Foundation::HANDLE(child.as_raw_handle());
    let r = server.accept(h, std::time::Duration::from_secs(10), &|| false);
    assert!(matches!(r, Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied));
    let _ = t.join();
    let _ = child.kill();
    let _ = child.wait();
}

// ------------------------------------------------------------------ Order 039 review fixes

#[test]
fn a_driver_is_never_a_service_of_ours() {
    let w = world();
    let (a, _) = hub(&w, false);
    // a demand-start kernel driver (not in Windows' list of Win32 services) - whatever its start type
    for name in ["WinRing0x64", "RTCore64"] {
        assert!(matches!(a.call(Purpose::Startup, Op::Service { name: name.into(), start: Start::Automatic, delayed: false }), Err(AdminError::NotFound(_))), "{name}");
    }
    assert_eq!(a.call(Purpose::Startup, Op::Service { name: "VendorSvc".into(), start: Start::Automatic, delayed: false }), Ok(vec![]));
}

/// A restore never goes back through a folder that is a link now (a junction made in the board's scratch folder; nothing
/// else is touched).
#[test]
fn a_restore_path_through_a_link_is_refused() {
    let root = std::path::PathBuf::from(r"C:\BoylerUtilities-scratch\039").join(format!("links-{}", std::process::id()));
    let real = root.join("real");
    let j = root.join("j");
    std::fs::create_dir_all(&real).unwrap();
    let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&j).arg(&real).stdout(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    assert!(made, "the junction could not be made");
    assert!(exec::link_on_the_way(&j.join("x.exe").to_string_lossy()));
    assert!(exec::link_on_the_way(&j.join("deeper").join("x.exe").to_string_lossy()));
    assert!(!exec::link_on_the_way(&real.join("x.exe").to_string_lossy()));
    assert!(!exec::link_on_the_way(r"C:\Users\u\Downloads\x.exe"), "folders that don't exist are no links");
    let _ = std::fs::remove_dir(&j);
    let _ = std::fs::remove_dir_all(&root);
}
