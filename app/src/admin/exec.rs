//! What the elevated copy does for one [`Op`], AFTER `Op::parse` checked its shape and its [`Purpose`](super::Purpose)
//! allowed it: every argument is checked once more against the system itself, then the one Windows call is made through
//! the feature crate's own OS layer ([`Sys`]: the real layers in the copy, the crates' fakes in tests).
//!
//! The checks:
//! - registry: hive + path + name must be one of the Tweaks admin rows' own values (bu_toggles' row table) and the
//!   value one of that row's two values (a delete = Windows' default, allowed for those values only);
//! - power: only USB selective suspend, 0 / 1;
//! - an adapter / a sound device / a physical disk: its id must be one Windows lists now;
//! - Startup: the HKLM Run value / all-users Startup file must exist (or its StartupApproved value already exists),
//!   the task must be one of the logon / boot tasks Windows lists, the service must exist, never Boot / System start;
//! - Defender: Allow only an id Defender knows (a detection or a threat), Disallow only an id on its allow list, Restore
//!   only a (threat id, file) pair in Defender's own quarantine list;
//! - DISM / sfc only with their fixed arguments; a restore point only with our own description;
//! - Windows' Temp folder from Windows (`GetSystemWindowsDirectoryW`, not an environment variable), every delete by an
//!   open handle whose final path is re-checked to be inside it (a link swapped in meanwhile can't redirect it).

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex};

use super::{AdminError, Hive, Op, Reply, Slot, Start};
use bu_quickfix::{FixOs, ProcCtl};
use bu_toggles::os::{PowerSetting, PowerValues, RegValue};
use bu_toggles::rows::{Method, RegData, ROWS};

/// The feature crates' OS layers the copy works through (real ones in the copy, fakes in tests). Each is made when an
/// op first needs it.
pub trait Sys {
    fn toggles(&mut self) -> Result<&mut dyn bu_toggles::TogglesOs, String>;
    fn net(&mut self) -> Result<Arc<dyn bu_network::NetworkOs>, String>;
    fn startup(&mut self) -> Result<&dyn bu_startup::StartupOs, String>;
    fn security(&mut self) -> Result<Arc<dyn bu_security::SecurityOs>, String>;
    fn audio(&mut self) -> Result<&mut dyn bu_audio::AudioOs, String>;
    fn fix(&mut self) -> Result<Arc<dyn FixOs>, String>;
    fn storage(&mut self) -> Result<Arc<dyn bu_storage::StorageOs>, String>;
    /// Empty Windows' own Temp folder: (freed bytes, freed files, in-use bytes, in-use files).
    fn clean_windows_temp(&mut self) -> Result<(u64, u64, u64, u64), String>;
}

/// The program streams of one copy: id → its control (for Kill).
#[derive(Default, Clone)]
pub struct Streams {
    pub ctl: Arc<Mutex<HashMap<u32, Arc<dyn ProcCtl>>>>,
    pub threads: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    next: Arc<Mutex<u32>>,
}

impl Streams {
    /// Wait for every running program's output to end (the copy exits only after that).
    pub fn wait_all(&self) {
        let ts: Vec<_> = std::mem::take(&mut *self.threads.lock().unwrap());
        for t in ts {
            let _ = t.join();
        }
    }
}

fn failed(e: impl std::fmt::Display) -> AdminError {
    AdminError::Failed(e.to_string())
}

fn refused(s: &str) -> AdminError {
    AdminError::Refused(s.into())
}

/// The registry values the Tweaks admin rows write: (hive, path, name) → the DWORDs allowed.
pub fn admin_reg_values() -> Vec<(Hive, &'static str, &'static str, Vec<u32>)> {
    let hive = |h: bu_toggles::os::Hive| if h == bu_toggles::os::Hive::Hklm { Hive::Hklm } else { Hive::Hkcu };
    let mut out = Vec::new();
    for r in ROWS.iter().filter(|r| r.needs_admin()) {
        match r.method {
            Method::Reg { values, .. } => {
                for v in values {
                    let mut ok = Vec::new();
                    for d in [v.on, v.off] {
                        if let RegData::D(n) = d {
                            ok.push(n);
                        }
                    }
                    out.push((hive(v.hive), v.path, v.name, ok));
                }
            }
            Method::Hags => out.push((Hive::Hklm, bu_toggles::rows::HAGS_PATH, bu_toggles::rows::HAGS_VALUE, vec![1, 2])),
            Method::FastStartup => out.push((Hive::Hklm, bu_toggles::rows::HIBERBOOT_PATH, bu_toggles::rows::HIBERBOOT_VALUE, vec![0, 1])),
            _ => {}
        }
    }
    out
}

/// Is (hive, path, name) an admin row's value, and `dword` (None = delete) allowed for it?
pub fn reg_allowed(h: Hive, path: &str, name: &str, dword: Option<u32>) -> bool {
    admin_reg_values().iter().any(|(vh, vp, vn, ok)| *vh == h && vp.eq_ignore_ascii_case(path) && vn.eq_ignore_ascii_case(name) && dword.is_none_or(|d| ok.contains(&d)))
}

fn th(h: Hive) -> bu_toggles::os::Hive {
    if h == Hive::Hklm {
        bu_toggles::os::Hive::Hklm
    } else {
        bu_toggles::os::Hive::Hkcu
    }
}

/// Run one op (its shape and purpose already checked). `out` sends a program's output frames (`Spawn`).
pub fn run(op: &Op, sys: &mut dyn Sys, streams: &Streams, out: Arc<dyn Fn(&[&[u8]]) + Send + Sync>) -> Reply {
    match op {
        Op::RegSet { hive, path, name, dword } => {
            if !reg_allowed(*hive, path, name, Some(*dword)) {
                return Err(refused("not a value of the Tweaks admin rows"));
            }
            sys.toggles().map_err(failed)?.reg_write(th(*hive), path, name, &RegValue::Dword(*dword)).map_err(failed)?;
            Ok(vec![])
        }
        Op::RegDelete { hive, path, name } => {
            if !reg_allowed(*hive, path, name, None) {
                return Err(refused("not a value of the Tweaks admin rows"));
            }
            sys.toggles().map_err(failed)?.reg_delete_value(th(*hive), path, name).map_err(failed)?;
            Ok(vec![])
        }
        Op::UsbSuspend { ac, dc } => {
            sys.toggles().map_err(failed)?.power_write(PowerSetting::UsbSelectiveSuspend, PowerValues { ac: *ac, dc: *dc }).map_err(failed)?;
            Ok(vec![])
        }
        Op::NetAdapter { id, on } => {
            let net = sys.net().map_err(failed)?;
            if !net.adapters().map_err(failed)?.iter().any(|a| a.id.eq_ignore_ascii_case(id)) {
                return Err(AdminError::NotFound("That network adapter is gone".into()));
            }
            net.set_adapter_enabled(id, *on).map_err(failed)?;
            Ok(vec![])
        }
        Op::NetDns { id, v4, v6 } => {
            let net = sys.net().map_err(failed)?;
            if !net.adapters().map_err(failed)?.iter().any(|a| a.id.eq_ignore_ascii_case(id)) {
                return Err(AdminError::NotFound("That network adapter is gone".into()));
            }
            net.set_dns_servers(id, &bu_network::DnsServers { v4: v4.clone(), v6: v6.clone() }).map_err(failed)?;
            Ok(vec![])
        }
        Op::Approved { slot, name, data } => {
            let s = sys.startup().map_err(failed)?;
            approved_entry_exists(s, *slot, name)?;
            s.reg_set_binary(bu_startup::Hive::LocalMachine, &approved_path(*slot), name, data).map_err(startup_err)?;
            Ok(vec![])
        }
        Op::ApprovedDelete { slot, name } => {
            let s = sys.startup().map_err(failed)?;
            approved_entry_exists(s, *slot, name)?;
            s.reg_delete_value(bu_startup::Hive::LocalMachine, &approved_path(*slot), name).map_err(startup_err)?;
            Ok(vec![])
        }
        Op::Task { path, on } => {
            let s = sys.startup().map_err(failed)?;
            if !s.logon_tasks().map_err(startup_err)?.iter().any(|t| t.path.eq_ignore_ascii_case(path)) {
                return Err(AdminError::NotFound("That task is gone".into()));
            }
            s.set_task_enabled(path, *on).map_err(startup_err)?;
            Ok(vec![])
        }
        Op::Service { name, start, delayed } => {
            let s = sys.startup().map_err(failed)?;
            // only a Win32 service Windows lists (the startup crate's list holds Win32 services only): never a kernel
            // driver, whatever its start type (a demand-start driver must not be made to load at every boot)
            if !s.services(std::slice::from_ref(name)).map_err(startup_err)?.iter().any(|x| x.name.eq_ignore_ascii_case(name)) {
                return Err(AdminError::NotFound("That service isn't there".into()));
            }
            let (cur, _) = s.service_start(name).map_err(startup_err)?;
            // a driver's start type is never ours to change
            if matches!(cur, bu_startup::ServiceStart::Boot | bu_startup::ServiceStart::System) {
                return Err(refused("a driver's start type"));
            }
            let st = match start {
                Start::Automatic => bu_startup::ServiceStart::Automatic,
                Start::Manual => bu_startup::ServiceStart::Manual,
                Start::Disabled => bu_startup::ServiceStart::Disabled,
            };
            s.set_service_start(name, st, *delayed).map_err(startup_err)?;
            Ok(vec![])
        }
        Op::DefenderAllow(id) => {
            let sec = sys.security().map_err(failed)?;
            let known = sec.detections().map_err(failed)?.iter().any(|d| d.threat_id == *id) || sec.threats().map_err(failed)?.iter().any(|t| t.threat_id == *id);
            if !known {
                return Err(AdminError::NotFound("Defender doesn't know that threat".into()));
            }
            sec.allow_threat(*id).map_err(failed)?;
            Ok(vec![])
        }
        Op::DefenderDisallow(id) => {
            let sec = sys.security().map_err(failed)?;
            if !sec.allowed_threat_ids().map_err(failed)?.contains(id) {
                return Err(AdminError::NotFound("That threat isn't allowed in Defender".into()));
            }
            sec.disallow_threat(*id).map_err(failed)?;
            Ok(vec![])
        }
        Op::DefenderRestore { id, file } => {
            let sec = sys.security().map_err(failed)?;
            let page = bu_security::SecurityService::new(sec.clone()).page().map_err(failed)?;
            let Some(row) = page.quarantine.iter().find(|r| r.threat_id == *id && r.path().eq_ignore_ascii_case(file)) else {
                return Err(AdminError::NotFound("That file isn't in Defender's quarantine".into()));
            };
            // Defender writes the file back (as SYSTEM) to its old path: never through a folder that is a link now (a
            // folder of the user's swapped for a junction to a system folder would plant the file there). A swap in the
            // moment between this check and Defender's write stays possible (unclear whether Defender follows links)
            if link_on_the_way(&row.path()) {
                return Err(AdminError::Refused("a folder on the file's way back is a link".into()));
            }
            // the path as Defender lists it, never the caller's spelling
            sec.restore_quarantined(&row.path()).map_err(failed)?;
            Ok(vec![])
        }
        Op::DefenderRemoveActive => {
            sys.security().map_err(failed)?.remove_active_threats().map_err(failed)?;
            Ok(vec![])
        }
        Op::DefenderOffline => {
            sys.security().map_err(failed)?.start_offline_scan().map_err(failed)?;
            Ok(vec![])
        }
        Op::AudioEndpoint { id, on } => {
            let a = sys.audio().map_err(failed)?;
            let mut known = false;
            for flow in [bu_audio::Flow::Output, bu_audio::Flow::Input] {
                known |= a.devices(flow).map_err(failed)?.iter().any(|d| d.id.eq_ignore_ascii_case(id));
            }
            if !known {
                return Err(AdminError::NotFound("That sound device is gone".into()));
            }
            a.set_enabled(id, *on).map_err(failed)?;
            Ok(vec![])
        }
        Op::Spawn(p) => {
            let fix = sys.fix().map_err(failed)?;
            let sp = fix.spawn(p.program(), p.args()).map_err(failed)?;
            let sid = {
                let mut n = streams.next.lock().unwrap();
                *n += 1;
                *n
            };
            streams.ctl.lock().unwrap().insert(sid, sp.ctl.clone());
            let (ctl, map) = (sp.ctl.clone(), streams.ctl.clone());
            let mut output = sp.output;
            let t = std::thread::Builder::new()
                .name("bu-admin-stream".into())
                .spawn(move || {
                    let sid_s = sid.to_string();
                    let mut buf = [0u8; 16384];
                    loop {
                        match output.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => out(&[b"out", sid_s.as_bytes(), &buf[..n]]),
                        }
                    }
                    let code = ctl.wait().map(|c| c.to_string()).unwrap_or_else(|_| "4294967295".into());
                    map.lock().unwrap().remove(&sid);
                    out(&[b"end", sid_s.as_bytes(), code.as_bytes()]);
                })
                .map_err(failed)?;
            streams.threads.lock().unwrap().push(t);
            Ok(vec![sid.to_string()])
        }
        Op::Kill(sid) => {
            if let Some(c) = streams.ctl.lock().unwrap().get(sid).cloned() {
                c.kill();
            }
            Ok(vec![])
        }
        Op::RestoreStatus => {
            let st = sys.fix().map_err(failed)?.restore_status().map_err(failed)?;
            Ok(restore_status_fields(&st))
        }
        Op::RestorePoint { description } => {
            let r = sys.fix().map_err(failed)?.create_restore_point(description).map_err(failed)?;
            Ok(vec![if r == bu_quickfix::CreateCall::Accepted { "accepted" } else { "protection-off" }.into()])
        }
        Op::CleanWindowsTemp => {
            let (fb, ff, ub, uf) = sys.clean_windows_temp().map_err(failed)?;
            Ok(vec![fb.to_string(), ff.to_string(), ub.to_string(), uf.to_string()])
        }
        Op::DiskHealth(n) => {
            let st = sys.storage().map_err(failed)?;
            if !st.physical_disks().map_err(failed)?.iter().any(|d| d.number == *n) {
                return Err(AdminError::NotFound("That disk is gone".into()));
            }
            Ok(health_fields(&st.disk_health(*n).map_err(failed)?))
        }
    }
}

/// Is any existing folder on `path`'s way (its parents, up to the drive) a junction / symbolic link?
pub fn link_on_the_way(path: &str) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, FILE_ATTRIBUTE_REPARSE_POINT, INVALID_FILE_ATTRIBUTES};
    let mut p = std::path::Path::new(path).parent();
    while let Some(d) = p {
        if d.as_os_str().is_empty() {
            break;
        }
        // SAFETY: a plain attribute read.
        let a = unsafe { GetFileAttributesW(&HSTRING::from(d.as_os_str())) };
        if a != INVALID_FILE_ATTRIBUTES && a & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return true;
        }
        p = d.parent();
    }
    false
}

fn startup_err(e: bu_startup::OsError) -> AdminError {
    match e {
        bu_startup::OsError::AccessDenied => AdminError::Denied("Windows refused it".into()),
        bu_startup::OsError::NotFound => AdminError::NotFound("It is gone".into()),
        other => failed(other),
    }
}

fn approved_path(slot: Slot) -> String {
    format!(r"{}\{}", bu_startup::APPROVED, slot.name())
}

/// The StartupApproved value belongs to a real HKLM entry: a value of HKLM Run (64- / 32-bit view) or a file of the
/// all-users Startup folder - or its StartupApproved value is already there (an entry removed since, put back by a reset).
fn approved_entry_exists(s: &dyn bu_startup::StartupOs, slot: Slot, name: &str) -> Result<(), AdminError> {
    use bu_startup::{Hive as H, RegView};
    if s.reg_binary(H::LocalMachine, &approved_path(slot), name).map_err(startup_err)?.is_some() {
        return Ok(());
    }
    let found = match slot {
        Slot::Run => s.reg_strings(H::LocalMachine, RegView::Bits64, bu_startup::RUN).map_err(startup_err)?.iter().any(|v| v.name.eq_ignore_ascii_case(name)),
        Slot::Run32 => s.reg_strings(H::LocalMachine, RegView::Bits32, bu_startup::RUN).map_err(startup_err)?.iter().any(|v| v.name.eq_ignore_ascii_case(name)),
        Slot::StartupFolder => s.startup_folder(true).map_err(startup_err)?.iter().any(|f| f.file_name.eq_ignore_ascii_case(name)),
    };
    if found {
        Ok(())
    } else {
        Err(AdminError::NotFound("That startup entry is gone".into()))
    }
}

// ------------------------------------------------------------------ answers with data

pub fn restore_status_fields(st: &bu_quickfix::RestoreStatus) -> Vec<String> {
    let mut v = vec![st.frequency_minutes.to_string(), (st.newest_known as u8).to_string()];
    if let Some(p) = &st.newest {
        v.extend([p.created.0.to_string(), p.sequence.to_string(), p.description.clone()]);
    }
    v
}

pub fn parse_restore_status(f: &[String]) -> Option<bu_quickfix::RestoreStatus> {
    let newest = match f.len() {
        2 => None,
        5 => Some(bu_quickfix::RestorePoint { created: bu_quickfix::Stamp(f[2].parse().ok()?), sequence: f[3].parse().ok()?, description: f[4].clone() }),
        _ => return None,
    };
    Some(bu_quickfix::RestoreStatus { frequency_minutes: f[0].parse().ok()?, newest_known: f[1] == "1", newest })
}

pub fn health_fields(h: &bu_storage::HealthRaw) -> Vec<String> {
    use bu_storage::OsHealthStatus as O;
    let mut v = Vec::new();
    if let Some(s) = h.os_status {
        v.push(format!(
            "os={}",
            match s {
                O::Healthy => "healthy",
                O::Warning => "warning",
                O::Unhealthy => "unhealthy",
                O::Unknown => "unknown",
            }
        ));
    }
    if let Some(t) = h.temperature_c {
        v.push(format!("temp={t}"));
    }
    if let Some(n) = &h.nvme {
        v.push(format!(
            "nvme={},{},{},{},{},{},{},{}",
            n.critical_warning, n.temperature_kelvin, n.available_spare_pct, n.available_spare_threshold_pct, n.percentage_used, n.power_on_hours, n.media_errors, n.unsafe_shutdowns
        ));
    }
    for a in &h.smart {
        v.push(format!("smart={},{},{},{},{}", a.id, a.value, a.worst, a.raw, a.threshold.map(|t| t.to_string()).unwrap_or("-".into())));
    }
    if let Some(r) = &h.reliability {
        let o = |x: Option<u64>| x.map(|n| n.to_string()).unwrap_or("-".into());
        v.push(format!("rel={},{},{}", o(r.temperature_c.map(u64::from)), o(r.wear_pct.map(u64::from)), o(r.power_on_hours)));
    }
    for n in &h.needs_admin {
        v.push(format!("na={n}"));
    }
    v
}

pub fn parse_health(f: &[String]) -> Option<bu_storage::HealthRaw> {
    use bu_storage::OsHealthStatus as O;
    let mut h = bu_storage::HealthRaw::default();
    let opt = |s: &str| -> Option<Option<u64>> {
        if s == "-" {
            Some(None)
        } else {
            s.parse().ok().map(Some)
        }
    };
    for x in f {
        let (k, val) = x.split_once('=')?;
        let p: Vec<&str> = val.split(',').collect();
        match k {
            "os" => {
                h.os_status = Some(match val {
                    "healthy" => O::Healthy,
                    "warning" => O::Warning,
                    "unhealthy" => O::Unhealthy,
                    "unknown" => O::Unknown,
                    _ => return None,
                })
            }
            "temp" => h.temperature_c = Some(val.parse().ok()?),
            "nvme" if p.len() == 8 => {
                h.nvme = Some(bu_storage::NvmeHealthLog {
                    critical_warning: p[0].parse().ok()?,
                    temperature_kelvin: p[1].parse().ok()?,
                    available_spare_pct: p[2].parse().ok()?,
                    available_spare_threshold_pct: p[3].parse().ok()?,
                    percentage_used: p[4].parse().ok()?,
                    power_on_hours: p[5].parse().ok()?,
                    media_errors: p[6].parse().ok()?,
                    unsafe_shutdowns: p[7].parse().ok()?,
                })
            }
            "smart" if p.len() == 5 => h.smart.push(bu_storage::SmartAttribute {
                id: p[0].parse().ok()?,
                value: p[1].parse().ok()?,
                worst: p[2].parse().ok()?,
                raw: p[3].parse().ok()?,
                threshold: if p[4] == "-" { None } else { Some(p[4].parse().ok()?) },
            }),
            "rel" if p.len() == 3 => {
                h.reliability = Some(bu_storage::ReliabilityCounter {
                    temperature_c: opt(p[0])?.map(|n| n as u32),
                    wear_pct: opt(p[1])?.map(|n| n as u32),
                    power_on_hours: opt(p[2])?,
                })
            }
            "na" => h.needs_admin.push(val.to_string()),
            _ => return None,
        }
    }
    Some(h)
}

// ------------------------------------------------------------------ the real layers (the elevated copy)

/// The real OS layers, made on first use. HKCU writes go to the clicking user's hive (`sid`).
pub struct RealSys {
    sid: String,
    toggles: Option<bu_toggles::real::RealOs>,
    net: Option<Arc<dyn bu_network::NetworkOs>>,
    startup: Option<bu_startup::real::RealOs>,
    security: Option<Arc<dyn bu_security::SecurityOs>>,
    audio: Option<bu_audio::RealOs>,
    fix: Option<Arc<dyn FixOs>>,
    storage: Option<Arc<dyn bu_storage::StorageOs>>,
}

impl RealSys {
    pub fn new(sid: &str) -> RealSys {
        RealSys { sid: sid.into(), toggles: None, net: None, startup: None, security: None, audio: None, fix: None, storage: None }
    }
}

impl Sys for RealSys {
    fn toggles(&mut self) -> Result<&mut dyn bu_toggles::TogglesOs, String> {
        if self.toggles.is_none() {
            self.toggles = Some(bu_toggles::real::RealOs::for_user(&self.sid).ok_or("no user")?);
        }
        Ok(self.toggles.as_mut().unwrap())
    }
    fn net(&mut self) -> Result<Arc<dyn bu_network::NetworkOs>, String> {
        Ok(self.net.get_or_insert_with(|| Arc::new(bu_network::real::WindowsNet::new())).clone())
    }
    fn startup(&mut self) -> Result<&dyn bu_startup::StartupOs, String> {
        Ok(self.startup.get_or_insert_with(bu_startup::real::RealOs::new))
    }
    fn security(&mut self) -> Result<Arc<dyn bu_security::SecurityOs>, String> {
        Ok(self.security.get_or_insert_with(|| Arc::new(bu_security::RealOs::new())).clone())
    }
    fn audio(&mut self) -> Result<&mut dyn bu_audio::AudioOs, String> {
        if self.audio.is_none() {
            self.audio = Some(bu_audio::RealOs::new().map_err(|e| e.to_string())?);
        }
        Ok(self.audio.as_mut().unwrap())
    }
    fn fix(&mut self) -> Result<Arc<dyn FixOs>, String> {
        Ok(self.fix.get_or_insert_with(|| Arc::new(bu_quickfix::real::RealOs::new())).clone())
    }
    fn storage(&mut self) -> Result<Arc<dyn bu_storage::StorageOs>, String> {
        Ok(self.storage.get_or_insert_with(|| Arc::new(bu_storage::RealOs::new())).clone())
    }
    fn clean_windows_temp(&mut self) -> Result<(u64, u64, u64, u64), String> {
        temp::clean()
    }
}

/// Emptying `<Windows>\Temp` as admin, safely: the folder comes from Windows, and every file / folder is deleted through
/// a handle opened without following links, whose final path is checked to still be inside that folder - a folder a
/// user swaps for a link meanwhile can never point the delete elsewhere.
pub mod temp {
    use std::path::{Path, PathBuf};
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FileDispositionInfo, GetFileInformationByHandle, GetFinalPathNameByHandleW, SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, DELETE,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_NAME_NORMALIZED,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    /// Is `p` (a final path from a handle, `\\?\C:\…`) inside `root` (also such a path)?
    pub fn inside(p: &str, root: &str) -> bool {
        let (p, root) = (p.to_lowercase(), root.to_lowercase());
        p.len() > root.len() + 1 && p.starts_with(&root) && p.as_bytes()[root.len()] == b'\\'
    }

    struct H(HANDLE);
    impl Drop for H {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    fn open(p: &Path, access: u32) -> Option<H> {
        // SAFETY: a plain open; the handle is closed by H.
        unsafe {
            CreateFileW(
                &HSTRING::from(p.as_os_str()),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )
        }
        .ok()
        .map(H)
    }

    fn final_path(h: &H) -> Option<String> {
        let mut buf = vec![0u16; 1024];
        // SAFETY: the buffer's length goes with it.
        let n = unsafe { GetFinalPathNameByHandleW(h.0, &mut buf, FILE_NAME_NORMALIZED) } as usize;
        (n > 0 && n < buf.len()).then(|| String::from_utf16_lossy(&buf[..n]))
    }

    fn info(h: &H) -> Option<BY_HANDLE_FILE_INFORMATION> {
        let mut i = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: a plain query into a local struct.
        unsafe { GetFileInformationByHandle(h.0, &mut i) }.ok().map(|_| i)
    }

    fn delete(h: &H) -> bool {
        let d = FILE_DISPOSITION_INFO { DeleteFile: true };
        // SAFETY: the struct lives across the call; its size goes with it.
        unsafe { SetFileInformationByHandle(h.0, FileDispositionInfo, &d as *const _ as *const _, std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32) }.is_ok()
    }

    /// `<Windows>\Temp` from Windows itself, refused when Windows or its Temp is a link.
    pub fn root() -> Result<(PathBuf, String), String> {
        let win = bu_addons::helper::windows_dir().ok_or("no Windows folder")?;
        let t = win.join("Temp");
        if bu_addons::helper::is_reparse(&win) || bu_addons::helper::is_reparse(&t) {
            return Err("Windows' Temp folder is redirected".into());
        }
        let h = open(&t, FILE_READ_ATTRIBUTES.0).ok_or("Windows' Temp folder can't be opened")?;
        let fp = final_path(&h).ok_or("Windows' Temp folder can't be read")?;
        Ok((t, fp))
    }

    /// Delete everything inside `<Windows>\Temp` that Windows lets go. (freed bytes, freed files, in-use bytes, in-use files)
    pub fn clean() -> Result<(u64, u64, u64, u64), String> {
        let (root, root_final) = root()?;
        let mut r = (0u64, 0u64, 0u64, 0u64);
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                let Some(h) = open(&p, DELETE.0 | FILE_READ_ATTRIBUTES.0) else {
                    if let Ok(m) = e.metadata() {
                        if !m.is_dir() {
                            r.2 += m.len();
                            r.3 += 1;
                        }
                    }
                    continue;
                };
                let (Some(fp), Some(i)) = (final_path(&h), info(&h)) else { continue };
                // only what is really inside Windows' Temp, never a link (not followed, not deleted)
                if !inside(&fp, &root_final) || i.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
                    continue;
                }
                if i.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
                    dirs.push(p.clone());
                    stack.push(p);
                    continue;
                }
                let size = ((i.nFileSizeHigh as u64) << 32) | i.nFileSizeLow as u64;
                if delete(&h) {
                    r.0 += size;
                    r.1 += 1;
                } else {
                    r.2 += size;
                    r.3 += 1;
                }
            }
        }
        // emptied folders, deepest first (one still holding an in-use file stays)
        for d in dirs.iter().rev() {
            if let Some(h) = open(d, DELETE.0 | FILE_READ_ATTRIBUTES.0) {
                if let (Some(fp), Some(i)) = (final_path(&h), info(&h)) {
                    if inside(&fp, &root_final) && i.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0 {
                        let _ = delete(&h);
                    }
                }
            }
        }
        Ok(r)
    }
}
