//! Admin actions (Order 039): the app never runs elevated. A change that needs admin starts a copy of the app with
//! Windows' own admin prompt (`ShellExecuteEx "runas"`, `BoylerUtilities.exe --bu-admin <purpose> <pipe id>`) for ONE user
//! action; the copy does it and exits. Nothing is installed or registered and nothing stays running.
//!
//! - The pages keep their feature crates' logic: each crate's OS layer is wrapped by a proxy (`proxy.rs`) that does the
//!   reads itself and hands only the admin-only calls (a registry value, an adapter switch, a service's start type …) to
//!   the elevated copy as one [`Op`]. A declined prompt comes back as the crate's own "needs admin" error, and the page
//!   says [`NOT_CHANGED`].
//! - One user action = one prompt: the copy lives for a [`client::Scope`] (one click, a whole repair, one reset of many
//!   lines - Order 036's undo-all batch) and takes several ops of that purpose through its pipe; without a scope each op
//!   is its own short copy.
//! - Safety (`helper.rs`, `exec.rs`): the elevated copy takes ONLY the [`Op`]s below, only those its [`Purpose`] allows,
//!   each with strictly checked arguments - here (shape) and again against the system itself (the registry values of the
//!   Tweaks rows' own table, adapters / services / tasks / Run entries / threats / sound devices that really exist, our
//!   own fixed programs and folders). Never a command line, a script or a path of the caller's choosing.
//! - Results come back through a pipe the normal app made (`wire.rs`); each side checks the other is the process it
//!   expects.

pub mod client;
pub mod exec;
pub mod helper;
pub mod proxy;
pub mod wire;

#[cfg(test)]
pub(crate) mod tests;

use std::net::{Ipv4Addr, Ipv6Addr};

/// The line a row / toast shows when the admin prompt was answered No (or the copy could not start).
pub const NOT_CHANGED: &str = "Needs admin \u{2014} not changed";

/// The elevated copy's command-line switch.
pub const ARG: &str = "--bu-admin";

/// What the elevated copy is started for (its command line names it; Windows' prompt shows it under "details"). Each
/// purpose allows only its own ops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Purpose {
    Tweaks,
    Network,
    Startup,
    Security,
    Audio,
    Repair,
    RestorePoint,
    Storage,
    /// The Keyboard tab's key remap: Windows' Scancode Map (Order 058).
    Keyboard,
    /// "Back to how your PC was" / "Windows defaults" / the uninstaller's undo: every op that puts a setting back.
    Reset,
}

impl Purpose {
    pub const ALL: [Purpose; 10] = [
        Purpose::Tweaks,
        Purpose::Network,
        Purpose::Startup,
        Purpose::Security,
        Purpose::Audio,
        Purpose::Repair,
        Purpose::RestorePoint,
        Purpose::Storage,
        Purpose::Keyboard,
        Purpose::Reset,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Purpose::Tweaks => "tweaks",
            Purpose::Network => "network",
            Purpose::Startup => "startup",
            Purpose::Security => "security",
            Purpose::Audio => "audio",
            Purpose::Repair => "repair",
            Purpose::RestorePoint => "restore-point",
            Purpose::Storage => "storage",
            Purpose::Keyboard => "keyboard",
            Purpose::Reset => "reset",
        }
    }

    pub fn parse(s: &str) -> Option<Purpose> {
        Purpose::ALL.into_iter().find(|p| p.name() == s)
    }

    /// Does this purpose allow `op`?
    pub fn allows(self, op: &Op) -> bool {
        use Op::*;
        match self {
            Purpose::Tweaks => matches!(op, RegSet { .. } | RegDelete { .. } | UsbSuspend { .. }),
            Purpose::Network => matches!(op, NetAdapter { .. } | NetDns { .. }),
            Purpose::Startup => matches!(op, Approved { .. } | ApprovedDelete { .. } | Task { .. } | Service { .. }),
            Purpose::Security => {
                matches!(op, DefenderAllow(_) | DefenderDisallow(_) | DefenderRestore { .. } | DefenderRemoveActive | DefenderOffline)
            }
            Purpose::Audio => matches!(op, AudioEndpoint { .. }),
            Purpose::Repair => matches!(op, Spawn(_) | Kill(_)),
            Purpose::RestorePoint => matches!(op, RestoreStatus | RestorePoint { .. }),
            Purpose::Storage => matches!(op, CleanWindowsTemp | DiskHealth(_)),
            Purpose::Keyboard => matches!(op, ScancodeMap { .. }),
            Purpose::Reset => matches!(
                op,
                RegSet { .. }
                    | RegDelete { .. }
                    | UsbSuspend { .. }
                    | NetAdapter { .. }
                    | NetDns { .. }
                    | Approved { .. }
                    | ApprovedDelete { .. }
                    | Task { .. }
                    | Service { .. }
                    | DefenderAllow(_)
                    | DefenderDisallow(_)
                    | AudioEndpoint { .. }
                    | ScancodeMap { .. }
            ),
        }
    }
}

/// HKLM, or the signed-in user's own hive (HKCU of the user who clicked - also when another account typed the admin
/// password: the copy writes to that user's `HKEY_USERS\<SID>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hive {
    Hklm,
    Hkcu,
}

/// The StartupApproved subkey (HKLM only - the user's own entries need no admin).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Run,
    Run32,
    StartupFolder,
}

impl Slot {
    pub fn name(self) -> &'static str {
        match self {
            Slot::Run => "Run",
            Slot::Run32 => "Run32",
            Slot::StartupFolder => "StartupFolder",
        }
    }
}

/// A service's start type the app may set (never Boot / System).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    Automatic,
    Manual,
    Disabled,
}

/// The two programs of "Repair Windows files", with their fixed arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prog {
    /// `dism.exe /Online /Cleanup-Image /RestoreHealth`
    Dism,
    /// `sfc.exe /scannow`
    Sfc,
}

impl Prog {
    pub fn program(self) -> &'static str {
        match self {
            Prog::Dism => "dism.exe",
            Prog::Sfc => "sfc.exe",
        }
    }
    pub fn args(self) -> &'static [&'static str] {
        match self {
            Prog::Dism => &["/Online", "/Cleanup-Image", "/RestoreHealth"],
            Prog::Sfc => &["/scannow"],
        }
    }
    /// The program + arguments a crate asked for, if they are exactly one of ours.
    pub fn of(program: &str, args: &[&str]) -> Option<Prog> {
        [Prog::Dism, Prog::Sfc].into_iter().find(|p| p.program().eq_ignore_ascii_case(program) && p.args() == args)
    }
}

/// One thing the elevated copy may do. The fields are checked here for their shape ([`Op::parse`]) and in `exec.rs`
/// against the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    // ---- Tweaks: one of the admin rows' own registry values (exec checks hive + path + name + value against
    // bu_toggles' row table)
    RegSet { hive: Hive, path: String, name: String, dword: u32 },
    RegDelete { hive: Hive, path: String, name: String },
    /// USB selective suspend in the active power plan (plugged in / on battery), 0 or 1.
    UsbSuspend { ac: u32, dc: u32 },
    // ---- Network: an adapter that exists (its interface GUID)
    NetAdapter { id: String, on: bool },
    /// Empty lists = automatic (from the router).
    NetDns { id: String, v4: Vec<Ipv4Addr>, v6: Vec<Ipv6Addr> },
    // ---- Startup
    /// HKLM `Explorer\StartupApproved\<slot>` value `name` = Task Manager's 12 bytes (02 = on, 03 + time = off).
    Approved { slot: Slot, name: String, data: Vec<u8> },
    ApprovedDelete { slot: Slot, name: String },
    /// A scheduled task with a logon / boot trigger, by its full path.
    Task { path: String, on: bool },
    Service { name: String, start: Start, delayed: bool },
    // ---- Security (Defender)
    DefenderAllow(i64),
    DefenderDisallow(i64),
    /// Put back the quarantined copy of `file` of threat `id` (exec checks the pair is in Defender's own quarantine).
    DefenderRestore { id: i64, file: String },
    DefenderRemoveActive,
    DefenderOffline,
    // ---- Audio: a sound device that exists (its endpoint id)
    AudioEndpoint { id: String, on: bool },
    // ---- Quick fixes
    /// Start DISM / sfc; its output streams back; the reply's field is the stream id.
    Spawn(Prog),
    /// Stop a stream's program (Cancel).
    Kill(u32),
    RestoreStatus,
    /// `SRSetRestorePointW` with our own description ("Boyler Utilities · 7 Oct 2026").
    RestorePoint { description: String },
    // ---- Storage
    /// Empty Windows' own Temp folder (`<Windows>\Temp`, from Windows, never from the caller).
    CleanWindowsTemp,
    /// Read-only health of physical disk N.
    DiskHealth(u32),
    // ---- Keyboard
    /// Windows' Scancode Map (HKLM `Keyboard Layout`): exactly these remaps; none = remove the map. Checked with the
    /// Keyboard crate's own rules (each key once, never onto itself, at most 64).
    ScancodeMap { maps: Vec<bu_keysound::remap::Mapping> },
}

/// Longest text field (a registry path, a task path, a file path).
const MAX_TEXT: usize = 1024;

fn on_off(s: &str) -> Result<bool, String> {
    match s {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("not on / off: {s:?}")),
    }
}

fn oo(b: bool) -> String {
    if b { "on" } else { "off" }.into()
}

/// A printable, bounded text: no control characters, no NUL, not empty.
pub fn check_text(s: &str, what: &str) -> Result<(), String> {
    if s.is_empty() || s.len() > MAX_TEXT || s.chars().any(|c| c.is_control()) {
        return Err(format!("bad {what}"));
    }
    Ok(())
}

/// `{8-4-4-4-12}` hex, the form Windows gives an adapter's interface GUID.
pub fn check_guid(s: &str) -> Result<(), String> {
    let b = s.as_bytes();
    let ok = b.len() == 38
        && b[0] == b'{'
        && b[37] == b'}'
        && s[1..37].char_indices().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() });
    if ok {
        Ok(())
    } else {
        Err("bad adapter id".into())
    }
}

/// A sound endpoint id: `{0.0.0.00000000}.{guid}` / `{0.0.1.00000000}.{guid}`.
pub fn check_endpoint(s: &str) -> Result<(), String> {
    let ok = s.len() == 55
        && (s.starts_with("{0.0.0.00000000}.") || s.starts_with("{0.0.1.00000000}."))
        && check_guid(&s[17..]).is_ok();
    if ok {
        Ok(())
    } else {
        Err("bad sound device id".into())
    }
}

/// A service's key name: letters, digits and `_ - . $ @ #` and spaces, at most 256 (Windows' limit).
pub fn check_service(s: &str) -> Result<(), String> {
    let ok = !s.is_empty() && s.len() <= 256 && s.chars().all(|c| c.is_ascii_alphanumeric() || "_-.$@# ".contains(c)) && !s.contains(['\\', '/']);
    if ok {
        Ok(())
    } else {
        Err("bad service name".into())
    }
}

/// A scheduled task's full path: `\folder\name`, no `..`, no control characters.
pub fn check_task(s: &str) -> Result<(), String> {
    check_text(s, "task path")?;
    if !s.starts_with('\\') || s.ends_with('\\') || s.split('\\').skip(1).any(|p| p.is_empty() || p == "." || p == "..") || s.contains('/') {
        return Err("bad task path".into());
    }
    Ok(())
}

/// A registry path relative to its hive: `A\B\C`, no empty parts, no leading / trailing backslash.
pub fn check_reg_path(s: &str) -> Result<(), String> {
    check_text(s, "registry path")?;
    if s.split('\\').any(str::is_empty) {
        return Err("bad registry path".into());
    }
    Ok(())
}

/// Task Manager's StartupApproved bytes: `02 00 00 00` + 8 zero bytes (on) or `03 00 00 00` + a FILETIME (off).
pub fn check_approved(d: &[u8]) -> Result<(), String> {
    let ok = d.len() == 12 && d[1..4] == [0, 0, 0] && (d[0] == 3 || (d[0] == 2 && d[4..].iter().all(|&b| b == 0)));
    if ok {
        Ok(())
    } else {
        Err("bad StartupApproved value".into())
    }
}

fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) || s.len() > 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("bad hex".into());
    }
    Ok((0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect())
}

fn num<T: std::str::FromStr>(s: &str, what: &str) -> Result<T, String> {
    // digits only (and a leading minus for ids): no "+5", no spaces
    let body = s.strip_prefix('-').unwrap_or(s);
    if body.is_empty() || body.len() > 20 || !body.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("bad {what}"));
    }
    s.parse().map_err(|_| format!("bad {what}"))
}

fn ip_list<T: std::str::FromStr>(s: &str) -> Result<Vec<T>, String> {
    if s == "-" {
        return Ok(Vec::new());
    }
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() > 4 {
        return Err("too many DNS servers".into());
    }
    parts.iter().map(|p| p.parse().map_err(|_| format!("bad DNS server {p:?}"))).collect()
}

fn list_text<T: std::fmt::Display>(v: &[T]) -> String {
    if v.is_empty() {
        "-".into()
    } else {
        v.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(",")
    }
}

impl Op {
    /// The op's wire form: name, then its fields.
    pub fn fields(&self) -> Vec<String> {
        let h = |h: &Hive| if *h == Hive::Hklm { "hklm" } else { "hkcu" }.to_string();
        match self {
            Op::RegSet { hive, path, name, dword } => vec!["reg-set".into(), h(hive), path.clone(), name.clone(), dword.to_string()],
            Op::RegDelete { hive, path, name } => vec!["reg-delete".into(), h(hive), path.clone(), name.clone()],
            Op::UsbSuspend { ac, dc } => vec!["usb-suspend".into(), ac.to_string(), dc.to_string()],
            Op::NetAdapter { id, on } => vec!["net-adapter".into(), id.clone(), oo(*on)],
            Op::NetDns { id, v4, v6 } => vec!["net-dns".into(), id.clone(), list_text(v4), list_text(v6)],
            Op::Approved { slot, name, data } => vec!["approved".into(), slot.name().into(), name.clone(), hex(data)],
            Op::ApprovedDelete { slot, name } => vec!["approved-delete".into(), slot.name().into(), name.clone()],
            Op::Task { path, on } => vec!["task".into(), path.clone(), oo(*on)],
            Op::Service { name, start, delayed } => {
                let s = match start {
                    Start::Automatic => "auto",
                    Start::Manual => "manual",
                    Start::Disabled => "disabled",
                };
                vec!["service".into(), name.clone(), s.into(), oo(*delayed)]
            }
            Op::DefenderAllow(id) => vec!["defender-allow".into(), id.to_string()],
            Op::DefenderDisallow(id) => vec!["defender-disallow".into(), id.to_string()],
            Op::DefenderRestore { id, file } => vec!["defender-restore".into(), id.to_string(), file.clone()],
            Op::DefenderRemoveActive => vec!["defender-remove-active".into()],
            Op::DefenderOffline => vec!["defender-offline".into()],
            Op::AudioEndpoint { id, on } => vec!["audio-endpoint".into(), id.clone(), oo(*on)],
            Op::Spawn(p) => vec!["spawn".into(), if *p == Prog::Dism { "dism" } else { "sfc" }.into()],
            Op::Kill(s) => vec!["kill".into(), s.to_string()],
            Op::RestoreStatus => vec!["restore-status".into()],
            Op::RestorePoint { description } => vec!["restore-point".into(), description.clone()],
            Op::CleanWindowsTemp => vec!["clean-windows-temp".into()],
            Op::DiskHealth(n) => vec!["disk-health".into(), n.to_string()],
            Op::ScancodeMap { maps } => vec![
                "scancode-map".into(),
                if maps.is_empty() { "-".into() } else { maps.iter().map(|m| format!("{:x}:{:x}", m.from, m.to)).collect::<Vec<_>>().join(",") },
            ],
        }
    }

    /// Parse an op from its wire form; anything not exactly right is refused (the message says what).
    pub fn parse(f: &[String]) -> Result<Op, String> {
        let name = f.first().map(String::as_str).unwrap_or("");
        let want = |n: usize| -> Result<(), String> {
            if f.len() == n + 1 {
                Ok(())
            } else {
                Err(format!("{name}: {} arguments, wanted {n}", f.len().saturating_sub(1)))
            }
        };
        let hive = |s: &str| match s {
            "hklm" => Ok(Hive::Hklm),
            "hkcu" => Ok(Hive::Hkcu),
            _ => Err(format!("bad hive {s:?}")),
        };
        let slot = |s: &str| match s {
            "Run" => Ok(Slot::Run),
            "Run32" => Ok(Slot::Run32),
            "StartupFolder" => Ok(Slot::StartupFolder),
            _ => Err(format!("bad startup slot {s:?}")),
        };
        let threat = |s: &str| -> Result<i64, String> {
            let id: i64 = num(s, "threat id")?;
            if id <= 0 {
                return Err("bad threat id".into());
            }
            Ok(id)
        };
        let op = match name {
            "reg-set" => {
                want(4)?;
                check_reg_path(&f[2])?;
                check_text(&f[3], "value name")?;
                Op::RegSet { hive: hive(&f[1])?, path: f[2].clone(), name: f[3].clone(), dword: num(&f[4], "value")? }
            }
            "reg-delete" => {
                want(3)?;
                check_reg_path(&f[2])?;
                check_text(&f[3], "value name")?;
                Op::RegDelete { hive: hive(&f[1])?, path: f[2].clone(), name: f[3].clone() }
            }
            "usb-suspend" => {
                want(2)?;
                let (ac, dc): (u32, u32) = (num(&f[1], "value")?, num(&f[2], "value")?);
                if ac > 1 || dc > 1 {
                    return Err("USB selective suspend is 0 or 1".into());
                }
                Op::UsbSuspend { ac, dc }
            }
            "net-adapter" => {
                want(2)?;
                check_guid(&f[1])?;
                Op::NetAdapter { id: f[1].clone(), on: on_off(&f[2])? }
            }
            "net-dns" => {
                want(3)?;
                check_guid(&f[1])?;
                Op::NetDns { id: f[1].clone(), v4: ip_list(&f[2])?, v6: ip_list(&f[3])? }
            }
            "approved" => {
                want(3)?;
                check_text(&f[2], "entry name")?;
                let data = unhex(&f[3])?;
                check_approved(&data)?;
                Op::Approved { slot: slot(&f[1])?, name: f[2].clone(), data }
            }
            "approved-delete" => {
                want(2)?;
                check_text(&f[2], "entry name")?;
                Op::ApprovedDelete { slot: slot(&f[1])?, name: f[2].clone() }
            }
            "task" => {
                want(2)?;
                check_task(&f[1])?;
                Op::Task { path: f[1].clone(), on: on_off(&f[2])? }
            }
            "service" => {
                want(3)?;
                check_service(&f[1])?;
                let start = match f[2].as_str() {
                    "auto" => Start::Automatic,
                    "manual" => Start::Manual,
                    "disabled" => Start::Disabled,
                    s => return Err(format!("bad start type {s:?}")),
                };
                let delayed = on_off(&f[3])?;
                if delayed && start != Start::Automatic {
                    return Err("delayed start is only for Automatic".into());
                }
                Op::Service { name: f[1].clone(), start, delayed }
            }
            "defender-allow" => {
                want(1)?;
                Op::DefenderAllow(threat(&f[1])?)
            }
            "defender-disallow" => {
                want(1)?;
                Op::DefenderDisallow(threat(&f[1])?)
            }
            "defender-restore" => {
                want(2)?;
                check_text(&f[2], "file")?;
                Op::DefenderRestore { id: threat(&f[1])?, file: f[2].clone() }
            }
            "defender-remove-active" => {
                want(0)?;
                Op::DefenderRemoveActive
            }
            "defender-offline" => {
                want(0)?;
                Op::DefenderOffline
            }
            "audio-endpoint" => {
                want(2)?;
                check_endpoint(&f[1])?;
                Op::AudioEndpoint { id: f[1].clone(), on: on_off(&f[2])? }
            }
            "spawn" => {
                want(1)?;
                Op::Spawn(match f[1].as_str() {
                    "dism" => Prog::Dism,
                    "sfc" => Prog::Sfc,
                    s => return Err(format!("not one of our programs: {s:?}")),
                })
            }
            "kill" => {
                want(1)?;
                Op::Kill(num(&f[1], "stream")?)
            }
            "restore-status" => {
                want(0)?;
                Op::RestoreStatus
            }
            "restore-point" => {
                want(1)?;
                let d = &f[1];
                check_text(d, "description")?;
                if !d.starts_with("Boyler Utilities \u{b7} ") || d.chars().count() > 64 {
                    return Err("not our restore point description".into());
                }
                Op::RestorePoint { description: d.clone() }
            }
            "clean-windows-temp" => {
                want(0)?;
                Op::CleanWindowsTemp
            }
            "disk-health" => {
                want(1)?;
                let n: u32 = num(&f[1], "disk number")?;
                if n >= 128 {
                    return Err("bad disk number".into());
                }
                Op::DiskHealth(n)
            }
            "scancode-map" => {
                want(1)?;
                Op::ScancodeMap { maps: parse_maps(&f[1])? }
            }
            other => return Err(format!("unknown action {:?}", other.chars().take(40).collect::<String>())),
        };
        Ok(op)
    }
}

/// "3a:1,e038:e01d" (hex scancode pairs, from:to) or "-" (none) -> checked remaps.
fn parse_maps(s: &str) -> Result<Vec<bu_keysound::remap::Mapping>, String> {
    if s == "-" {
        return Ok(Vec::new());
    }
    let hex16 = |t: &str| -> Result<u16, String> {
        if t.is_empty() || t.len() > 4 || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("bad scancode {t:?}"));
        }
        u16::from_str_radix(t, 16).map_err(|_| format!("bad scancode {t:?}"))
    };
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() > bu_keysound::remap::MAX_MAPPINGS {
        return Err("too many remaps".into());
    }
    let mut maps = Vec::with_capacity(parts.len());
    for p in parts {
        let (a, b) = p.split_once(':').ok_or("a remap is from:to")?;
        maps.push(bu_keysound::remap::Mapping { from: hex16(a)?, to: hex16(b)? });
    }
    bu_keysound::remap::check(&maps)?;
    Ok(maps)
}

/// How an op went wrong, as the normal app sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminError {
    /// The admin prompt was answered No (or closed).
    Declined,
    /// The elevated copy refused the op (its arguments are not ones it accepts).
    Refused(String),
    /// Windows said no even to admin (a locked service, a protected value).
    Denied(String),
    /// The thing is gone (an adapter unplugged, a task deleted).
    NotFound(String),
    /// Anything else (the copy did not start, Windows failed).
    Failed(String),
}

impl std::fmt::Display for AdminError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdminError::Declined => f.write_str(NOT_CHANGED),
            AdminError::Refused(s) => write!(f, "The admin helper refused it ({s})"),
            AdminError::Denied(s) | AdminError::NotFound(s) | AdminError::Failed(s) => f.write_str(s),
        }
    }
}

/// What the elevated copy answers to one op: ok + its fields, or an error.
pub type Reply = Result<Vec<String>, AdminError>;
