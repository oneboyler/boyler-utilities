//! A fake OS for tests: scripted live readings, a process table, specs. Nothing real is touched.

use crate::specs::PcSpecs;
use crate::{EndHow, Icon, LiveReading, LiveSource, PerfError, PerfOs, Priority, RawProcess, Result};
use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
struct State {
    readings: VecDeque<LiveReading>,
    processes: Vec<RawProcess>,
    specs: PcSpecs,
    elevated: bool,
    cpu_count: u32,
    /// Pids that answer "access denied" (other users / elevated apps).
    protected: HashSet<u32>,
    actions: Vec<String>,
    open_live_fails: bool,
}

/// The fake. Shared counters show how often the live source was read (zero-cost-when-stopped test).
#[derive(Debug, Default, Clone)]
pub struct FakeOs {
    s: Arc<Mutex<State>>,
    live_reads: Arc<AtomicU64>,
    live_open: Arc<AtomicU64>,
}

impl FakeOs {
    pub fn new() -> Self {
        let f = FakeOs::default();
        f.st().cpu_count = 4;
        f
    }
    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.s.lock().unwrap_or_else(|p| p.into_inner())
    }
    /// Readings handed out in order; the last one repeats.
    pub fn push_reading(&self, r: LiveReading) -> &Self {
        self.st().readings.push_back(r);
        self
    }
    pub fn with_processes(&self, p: Vec<RawProcess>) -> &Self {
        self.st().processes = p;
        self
    }
    pub fn with_specs(&self, s: PcSpecs) -> &Self {
        self.st().specs = s;
        self
    }
    pub fn with_elevated(&self, e: bool) -> &Self {
        self.st().elevated = e;
        self
    }
    pub fn with_cpu_count(&self, n: u32) -> &Self {
        self.st().cpu_count = n;
        self
    }
    pub fn with_protected(&self, pid: u32) -> &Self {
        self.st().protected.insert(pid);
        self
    }
    pub fn with_open_live_failing(&self) -> &Self {
        self.st().open_live_fails = true;
        self
    }
    /// Every action asked ("end 12 Close", "priority 12 High", "open C:\…").
    pub fn actions(&self) -> Vec<String> {
        self.st().actions.clone()
    }
    /// How many live readings were taken so far.
    pub fn live_reads(&self) -> u64 {
        self.live_reads.load(Ordering::SeqCst)
    }
    /// Live sources open right now (0 when the sampler is stopped).
    pub fn live_open(&self) -> u64 {
        self.live_open.load(Ordering::SeqCst)
    }
}

struct FakeLive {
    os: FakeOs,
}

impl LiveSource for FakeLive {
    fn read(&mut self) -> Result<LiveReading> {
        self.os.live_reads.fetch_add(1, Ordering::SeqCst);
        let mut st = self.os.st();
        let r = if st.readings.len() > 1 { st.readings.pop_front() } else { st.readings.front().cloned() };
        Ok(r.unwrap_or_default())
    }
}

impl Drop for FakeLive {
    fn drop(&mut self) {
        self.os.live_open.fetch_sub(1, Ordering::SeqCst);
    }
}

impl PerfOs for FakeOs {
    fn open_live(&self) -> Result<Box<dyn LiveSource>> {
        if self.st().open_live_fails {
            return Err(PerfError::Os { context: "PdhOpenQuery".into(), code: 1 });
        }
        self.live_open.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeLive { os: self.clone() }))
    }
    fn processes(&self) -> Result<Vec<RawProcess>> {
        Ok(self.st().processes.clone())
    }
    fn cpu_count(&self) -> u32 {
        self.st().cpu_count
    }
    fn specs(&self) -> Result<PcSpecs> {
        Ok(self.st().specs.clone())
    }
    fn end_process(&self, pid: u32, how: EndHow) -> Result<()> {
        let mut st = self.st();
        if st.protected.contains(&pid) {
            return Err(PerfError::NeedsAdmin(format!("end {pid}")));
        }
        let i = st.processes.iter().position(|p| p.pid == pid).ok_or_else(|| PerfError::NotFound(format!("pid {pid}")))?;
        st.processes.remove(i);
        st.actions.push(format!("end {pid} {how:?}"));
        Ok(())
    }
    fn set_priority(&self, pid: u32, p: Priority) -> Result<()> {
        let mut st = self.st();
        if st.protected.contains(&pid) {
            return Err(PerfError::NeedsAdmin(format!("priority {pid}")));
        }
        let proc_ = st.processes.iter_mut().find(|x| x.pid == pid).ok_or_else(|| PerfError::NotFound(format!("pid {pid}")))?;
        proc_.priority = p;
        st.actions.push(format!("priority {pid} {p:?}"));
        Ok(())
    }
    fn open_file_location(&self, path: &Path) -> Result<()> {
        self.st().actions.push(format!("open {}", path.display()));
        Ok(())
    }
    fn icon_rgba(&self, _path: &Path, size: u32) -> Result<Icon> {
        Ok(Icon { width: size, height: size, rgba: vec![0; (size * size * 4) as usize] })
    }
    fn is_elevated(&self) -> bool {
        self.st().elevated
    }
}
