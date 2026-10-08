//! A FAKE Windows for the Quick fixes tests: no key is sent, no program runs, no file is deleted, Explorer is never
//! stopped, no restore point is made — everything is recorded instead. Programs are scripted (output chunks + exit
//! code), optionally "hanging" until killed, so Cancel is tested without real time. The clock is set by the test.

use crate::os::{
    CreateCall, DisplayAdapter, ExplorerPause, FixOs, LocalTime, ProcCtl, RestorePoint, RestoreStatus, Spawned, Stamp,
};
use crate::{FixError, Result};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A scripted console program.
#[derive(Clone, Debug, Default)]
pub struct Script {
    pub chunks: Vec<Vec<u8>>,
    pub exit_code: u32,
    /// After the chunks, block until killed (a long DISM / sfc run).
    pub hang: bool,
}

impl Script {
    pub fn new(chunks: Vec<Vec<u8>>, exit_code: u32) -> Self {
        Script { chunks, exit_code, hang: false }
    }
    pub fn hanging(mut self) -> Self {
        self.hang = true;
        self
    }
}

/// UTF-16LE bytes, no BOM (what sfc writes into a pipe).
pub fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

#[derive(Default)]
struct ProcState {
    killed: bool,
}

struct FakeProc {
    state: Mutex<ProcState>,
    cv: Condvar,
    exit_code: u32,
}

impl ProcCtl for FakeProc {
    fn kill(&self) {
        lock(&self.state).killed = true;
        self.cv.notify_all();
    }
    fn wait(&self) -> Result<u32> {
        Ok(if lock(&self.state).killed { 1 } else { self.exit_code })
    }
}

struct FakeOutput {
    chunks: std::collections::VecDeque<Vec<u8>>,
    hang: bool,
    proc: Arc<FakeProc>,
    started: Arc<(Mutex<u32>, Condvar)>,
}

impl Read for FakeOutput {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if lock(&self.proc.state).killed {
            return Ok(0);
        }
        if let Some(mut c) = self.chunks.pop_front() {
            let n = c.len().min(buf.len());
            buf[..n].copy_from_slice(&c[..n]);
            if n < c.len() {
                self.chunks.push_front(c.split_off(n));
            }
            return Ok(n);
        }
        if self.hang {
            // tell the test "everything printed, now running", then block until killed
            {
                let (m, cv) = &*self.started;
                *lock(m) += 1;
                cv.notify_all();
            }
            let mut st = lock(&self.proc.state);
            while !st.killed {
                st = self.proc.cv.wait(st).unwrap_or_else(|e| e.into_inner());
            }
        }
        Ok(0)
    }
}

#[derive(Default)]
struct World {
    elevated: bool,
    foreground_ours: bool,
    read_only: bool,
    chords_sent: u32,
    adapters: Vec<DisplayAdapter>,
    scripts: HashMap<String, Vec<Script>>,
    spawned: Vec<(String, Vec<String>)>,
    procs: Vec<Arc<FakeProc>>,
    cbs_log: Option<String>,
    cache_dir: PathBuf,
    files: HashMap<PathBuf, Vec<(String, u64)>>,
    locked: Vec<String>,
    /// "stop explorer", "delete <name>", "restart explorer" in order.
    events: Vec<String>,
    explorer_running: bool,
    fail_restart: bool,
    frequency_minutes: u32,
    points: Vec<RestorePoint>,
    points_readable: bool,
    unreadable_until_create: bool,
    protection_off: bool,
    create_calls: Vec<String>,
    now: i64,
}

/// The fake. Clone = the same world.
#[derive(Clone)]
pub struct FakeFixOs {
    w: Arc<Mutex<World>>,
    started: Arc<(Mutex<u32>, Condvar)>,
}

impl Default for FakeFixOs {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeFixOs {
    /// Not elevated, our menu in front, one NVIDIA adapter, the Explorer cache folder, 24 h restore rule, no points,
    /// now = 7 Oct 2026 21:37 (UTC = "local" in the fake).
    pub fn new() -> Self {
        let cache_dir = PathBuf::from(r"C:\Users\someone\AppData\Local\Microsoft\Windows\Explorer");
        let w = World {
            foreground_ours: true,
            adapters: vec![DisplayAdapter {
                name: "NVIDIA GeForce RTX 4090".into(),
                instance_id: r"PCI\VEN_10DE&DEV_2684&SUBSYS_00000000&REV_A1\4&FAKE&0&0008".into(),
            }],
            cache_dir: cache_dir.clone(),
            files: HashMap::from([(cache_dir, Vec::new())]),
            explorer_running: true,
            frequency_minutes: 1440,
            points_readable: true,
            now: stamp(2026, 10, 7, 21, 37).0,
            ..Default::default()
        };
        FakeFixOs { w: Arc::new(Mutex::new(w)), started: Arc::new((Mutex::new(0), Condvar::new())) }
    }

    pub fn elevated(self) -> Self {
        lock(&self.w).elevated = true;
        self
    }
    /// Every change refused (like a read-only real layer).
    pub fn read_only(self) -> Self {
        lock(&self.w).read_only = true;
        self
    }
    pub fn set_foreground_ours(&self, ours: bool) {
        lock(&self.w).foreground_ours = ours;
    }
    pub fn chords_sent(&self) -> u32 {
        lock(&self.w).chords_sent
    }
    pub fn set_adapters(&self, a: Vec<DisplayAdapter>) {
        lock(&self.w).adapters = a;
    }
    /// The next run of `program` (e.g. "dism.exe") follows `s`. Several queued → used in order.
    pub fn script(&self, program: &str, s: Script) {
        lock(&self.w).scripts.entry(program.to_ascii_lowercase()).or_default().push(s);
    }
    pub fn spawned(&self) -> Vec<(String, Vec<String>)> {
        lock(&self.w).spawned.clone()
    }
    /// Programs killed so far.
    pub fn killed(&self) -> usize {
        lock(&self.w).procs.iter().filter(|p| lock(&p.state).killed).count()
    }
    /// Blocks until `n` hanging programs have printed everything and are "running".
    pub fn wait_running(&self, n: u32) {
        let (m, cv) = &*self.started;
        let mut c = lock(m);
        while *c < n {
            c = cv.wait(c).unwrap_or_else(|e| e.into_inner());
        }
    }
    pub fn set_cbs_log(&self, s: &str) {
        lock(&self.w).cbs_log = Some(s.to_string());
    }
    pub fn cache_dir(&self) -> PathBuf {
        lock(&self.w).cache_dir.clone()
    }
    pub fn add_file(&self, name: &str, bytes: u64) {
        let mut w = lock(&self.w);
        let d = w.cache_dir.clone();
        w.files.entry(d).or_default().push((name.to_string(), bytes));
    }
    pub fn files(&self) -> Vec<String> {
        let w = lock(&self.w);
        w.files.get(&w.cache_dir).map(|v| v.iter().map(|f| f.0.clone()).collect()).unwrap_or_default()
    }
    /// Another app holds this file open — delete fails.
    pub fn lock_file(&self, name: &str) {
        lock(&self.w).locked.push(name.to_string());
    }
    pub fn events(&self) -> Vec<String> {
        lock(&self.w).events.clone()
    }
    pub fn explorer_running(&self) -> bool {
        lock(&self.w).explorer_running
    }
    pub fn fail_restart(&self) {
        lock(&self.w).fail_restart = true;
    }
    pub fn set_frequency(&self, minutes: u32) {
        lock(&self.w).frequency_minutes = minutes;
    }
    pub fn add_point(&self, created: Stamp, description: &str) {
        let mut w = lock(&self.w);
        let seq = w.points.len() as u32 + 1;
        w.points.push(RestorePoint { created, description: description.into(), sequence: seq });
    }
    pub fn points(&self) -> Vec<RestorePoint> {
        lock(&self.w).points.clone()
    }
    pub fn set_points_readable(&self, r: bool) {
        lock(&self.w).points_readable = r;
    }
    /// The point list can't be read until a restore point call was made (tests the check after the call).
    pub fn unreadable_until_create(&self) {
        let mut w = lock(&self.w);
        w.points_readable = false;
        w.unreadable_until_create = true;
    }
    pub fn set_protection_off(&self) {
        lock(&self.w).protection_off = true;
    }
    pub fn create_calls(&self) -> Vec<String> {
        lock(&self.w).create_calls.clone()
    }
    pub fn set_now(&self, t: Stamp) {
        lock(&self.w).now = t.0;
    }

    fn refuse(&self, what: &str) -> Result<()> {
        if lock(&self.w).read_only {
            Err(FixError::Refused(format!("read-only: {what}")))
        } else {
            Ok(())
        }
    }
}

/// A UTC stamp from a civil date + time.
pub fn stamp(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> Stamp {
    let s = format!("{y:04}{mo:02}{d:02}{h:02}{mi:02}00.000000+000");
    crate::restore::parse_cim_datetime(&s).unwrap_or(Stamp(0))
}

/// UTC "local" time (the fake has no time zone).
pub fn utc(t: Stamp) -> LocalTime {
    let days = t.0.div_euclid(86_400);
    let secs = t.0.rem_euclid(86_400);
    // civil_from_days (Howard Hinnant)
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    LocalTime { year: y as u16, month: m as u8, day: d as u8, hour: (secs / 3600) as u8, minute: (secs % 3600 / 60) as u8 }
}

struct FakePause {
    w: Arc<Mutex<World>>,
    done: bool,
}

impl FakePause {
    fn bring_back(&mut self) -> Result<()> {
        if self.done {
            return Ok(());
        }
        self.done = true;
        let mut w = lock(&self.w);
        w.events.push("restart explorer".into());
        if w.fail_restart {
            // Restart Manager failed; the real layer then starts explorer.exe itself — the shell comes back either way
            w.explorer_running = true;
            return Err(FixError::Os { context: "RmRestart".into(), code: 5 });
        }
        w.explorer_running = true;
        Ok(())
    }
}

impl ExplorerPause for FakePause {
    fn restart(mut self: Box<Self>) -> Result<()> {
        self.bring_back()
    }
}

impl Drop for FakePause {
    fn drop(&mut self) {
        let _ = self.bring_back();
    }
}

impl FixOs for FakeFixOs {
    fn is_elevated(&self) -> bool {
        lock(&self.w).elevated
    }

    fn foreground_is_ours(&self) -> bool {
        lock(&self.w).foreground_ours
    }

    fn send_reset_chord(&self) -> Result<()> {
        self.refuse("send_reset_chord")?;
        lock(&self.w).chords_sent += 1;
        Ok(())
    }

    fn display_adapters(&self) -> Result<Vec<DisplayAdapter>> {
        Ok(lock(&self.w).adapters.clone())
    }

    fn spawn(&self, program: &str, args: &[&str]) -> Result<Spawned> {
        self.refuse(program)?;
        let mut w = lock(&self.w);
        w.spawned.push((program.to_string(), args.iter().map(|s| s.to_string()).collect()));
        let q = w.scripts.entry(program.to_ascii_lowercase()).or_default();
        let s = if q.is_empty() {
            return Err(FixError::Os { context: format!("start {program}"), code: 2 });
        } else {
            q.remove(0)
        };
        let proc = Arc::new(FakeProc { state: Mutex::new(ProcState::default()), cv: Condvar::new(), exit_code: s.exit_code });
        w.procs.push(proc.clone());
        let output = FakeOutput { chunks: s.chunks.into(), hang: s.hang, proc: proc.clone(), started: self.started.clone() };
        Ok(Spawned { output: Box::new(output), ctl: proc })
    }

    fn cbs_log_tail(&self) -> Result<String> {
        lock(&self.w).cbs_log.clone().ok_or_else(|| FixError::NeedsAdmin("CBS.log".into()))
    }

    fn explorer_cache_dir(&self) -> Result<PathBuf> {
        Ok(lock(&self.w).cache_dir.clone())
    }

    fn list_files(&self, dir: &Path) -> Result<Vec<(String, u64)>> {
        lock(&self.w).files.get(dir).cloned().ok_or_else(|| FixError::Unavailable(format!("{}", dir.display())))
    }

    fn delete_file(&self, path: &Path) -> Result<()> {
        self.refuse("delete_file")?;
        let mut w = lock(&self.w);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if w.locked.contains(&name) {
            return Err(FixError::Os { context: format!("delete {name}"), code: 32 }); // ERROR_SHARING_VIOLATION
        }
        if w.explorer_running && name.to_ascii_lowercase().starts_with("iconcache") {
            return Err(FixError::Os { context: format!("delete {name} (Explorer holds it)"), code: 32 });
        }
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let list = w.files.get_mut(&dir).ok_or_else(|| FixError::Unavailable("folder".into()))?;
        let before = list.len();
        list.retain(|f| f.0 != name);
        if list.len() == before {
            return Err(FixError::Os { context: format!("delete {name}"), code: 2 });
        }
        w.events.push(format!("delete {name}"));
        Ok(())
    }

    fn stop_explorer(&self) -> Result<Box<dyn ExplorerPause>> {
        self.refuse("stop_explorer")?;
        let mut w = lock(&self.w);
        w.events.push("stop explorer".into());
        w.explorer_running = false;
        Ok(Box::new(FakePause { w: self.w.clone(), done: false }))
    }

    fn restore_status(&self) -> Result<RestoreStatus> {
        let w = lock(&self.w);
        let newest = if w.points_readable { w.points.iter().max_by_key(|p| (p.created, p.sequence)).cloned() } else { None };
        Ok(RestoreStatus { frequency_minutes: w.frequency_minutes, newest, newest_known: w.points_readable })
    }

    fn create_restore_point(&self, description: &str) -> Result<CreateCall> {
        self.refuse("create_restore_point")?;
        let mut w = lock(&self.w);
        w.create_calls.push(description.to_string());
        if w.unreadable_until_create {
            w.points_readable = true;
        }
        if w.protection_off {
            return Ok(CreateCall::ProtectionOff);
        }
        // Windows' own rule: inside the window the call says yes and makes nothing
        let now = w.now;
        let freq = i64::from(w.frequency_minutes);
        let blocked = freq > 0 && w.points.iter().any(|p| now < p.created.0 + freq * 60);
        if !blocked {
            let seq = w.points.len() as u32 + 1;
            w.points.push(RestorePoint { created: Stamp(now), description: description.into(), sequence: seq });
        }
        Ok(CreateCall::Accepted)
    }

    fn now(&self) -> Stamp {
        Stamp(lock(&self.w).now)
    }

    fn local(&self, t: Stamp) -> LocalTime {
        utc(t)
    }
}
