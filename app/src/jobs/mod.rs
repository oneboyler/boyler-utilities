//! Job runner for slow work (Order 014 change 8; the owner Oct 8 "NOTHING HEAVY ON ITS OWN": scans, pings, speed tests,
//! sizing start ONLY on the user's button; opening a tab never starts them).
//! - [`JobRunner::start`] needs a [`Pressed`] token, and a token can only be made by [`Clicks::press`]. There is ONE
//!   [`Clicks`] per process ([`Clicks::take`] gives it once) and the UI's click dispatch owns it: it makes a token for
//!   the one click it is delivering and lends it to the page's click handler. A page's open / build path is never handed
//!   one, and a token borrows the `Clicks` for the length of that dispatch, so it can't be kept for later. So a job can't
//!   start on open — by the API, not by care;
//! - each job runs on its own worker thread (never the UI thread); it reports progress (0..1 or indeterminate) and a
//!   status text through [`JobCtx`]; the UI polls [`JobRunner::view`] (a short lock, no waiting on the job);
//! - stop = a cooperative flag: [`JobRunner::stop`] sets it, the job's closure checks [`JobCtx::stopped`] /
//!   [`JobCtx::check`] and returns; the end is [`End::Done`] / [`End::Failed`] / [`End::Stopped`] (a panic = Failed);
//! - every progress / status / end calls the waker (`Fn() + Send`; the app posts a message to its window);
//! - dropping the runner stops every job and waits for its thread (a job must check its stop flag between steps).
//!   Pages get the runner from the page API; a job's key (e.g. "network.speedtest") lets a second press find the running one.

#[cfg(test)]
mod tests;

use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// The one source of click tokens. Owned by the UI's click dispatch; never given to pages.
pub struct Clicks {
    _only_here: (),
}

static CLICKS_TAKEN: AtomicBool = AtomicBool::new(false);

impl Clicks {
    /// The process's one `Clicks` (the UI takes it at start-up). A second call gets None.
    pub fn take() -> Option<Clicks> {
        (!CLICKS_TAKEN.swap(true, Ordering::SeqCst)).then_some(Clicks { _only_here: () })
    }

    /// Tests only: a `Clicks` of their own (the tests stand in for the UI's click dispatch).
    #[cfg(test)]
    pub(crate) fn for_test() -> Clicks {
        Clicks { _only_here: () }
    }

    /// A token for the click being delivered right now. It borrows `self`, so it lives only as long as the dispatch.
    pub fn press(&mut self) -> Pressed<'_> {
        Pressed { _click: PhantomData }
    }
}

/// "The user pressed a button" — the only way to start a job. Made only by [`Clicks::press`]; not Clone, not Send.
pub struct Pressed<'a> {
    _click: PhantomData<(&'a mut Clicks, *const ())>,
}

/// A job's progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Progress {
    /// 0.0 ..= 1.0.
    Part(f32),
    /// Busy, no measure (spinner / "Updating…").
    Busy,
}

/// How a job ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    /// Finished; the job's last word (may be empty).
    Done(String),
    /// Failed; the reason.
    Failed(String),
    /// Stopped by the user.
    Stopped,
}

/// What a job's closure returns on the way out early.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobError {
    Stopped,
    Failed(String),
}

impl From<String> for JobError {
    fn from(s: String) -> Self {
        JobError::Failed(s)
    }
}

impl From<&str> for JobError {
    fn from(s: &str) -> Self {
        JobError::Failed(s.into())
    }
}

/// What the UI shows for one job.
#[derive(Debug, Clone, PartialEq)]
pub struct JobView {
    pub id: JobId,
    pub key: String,
    pub progress: Progress,
    pub status: String,
    /// None = still running.
    pub end: Option<End>,
    /// Stop was pressed (shown as "Stopping…" until the job returns).
    pub stopping: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JobId(u64);

/// Why a job didn't start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartError {
    /// A job with this key is running (its id).
    AlreadyRunning(JobId),
    /// The worker thread couldn't be made.
    NoThread(String),
}

type Waker = Arc<Mutex<Box<dyn Fn() + Send>>>;

struct Shared {
    progress: Progress,
    status: String,
    end: Option<End>,
}

struct Job {
    id: JobId,
    key: String,
    stop: Arc<AtomicBool>,
    shared: Arc<Mutex<Shared>>,
    thread: Option<JoinHandle<()>>,
}

/// The job's side: report progress, check for stop.
pub struct JobCtx {
    stop: Arc<AtomicBool>,
    shared: Arc<Mutex<Shared>>,
    waker: Waker,
}

impl JobCtx {
    /// Progress 0..1 (clamped).
    pub fn progress(&self, part: f32) {
        self.set(|s| s.progress = Progress::Part(part.clamp(0.0, 1.0)));
    }

    /// Busy with no measure.
    pub fn busy(&self) {
        self.set(|s| s.progress = Progress::Busy);
    }

    pub fn status(&self, text: &str) {
        self.set(|s| s.status = text.to_string());
    }

    /// Stop was pressed.
    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// The stop flag itself (Order 066: a download's writer checks it between chunks, so a stop - or the app quitting - ends it
    /// at once instead of after the whole file).
    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }

    /// `ctx.check()?` between steps: returns Err(Stopped) once stop was pressed.
    pub fn check(&self) -> Result<(), JobError> {
        if self.stopped() {
            Err(JobError::Stopped)
        } else {
            Ok(())
        }
    }

    fn set(&self, f: impl FnOnce(&mut Shared)) {
        if let Ok(mut s) = self.shared.lock() {
            f(&mut s);
        }
        wake(&self.waker);
    }
}

fn wake(w: &Waker) {
    if let Ok(f) = w.lock() {
        f();
    }
}

/// The runner. One per app (pages get it from the page API).
pub struct JobRunner {
    waker: Waker,
    jobs: Vec<Job>,
    next: u64,
}

impl JobRunner {
    /// `waker` is called (from the worker thread) on every progress / status / end; the app posts a message to its window.
    pub fn new(waker: impl Fn() + Send + 'static) -> Self {
        JobRunner { waker: Arc::new(Mutex::new(Box::new(waker))), jobs: Vec::new(), next: 1 }
    }

    /// Start a job — only with a click's token. `key` names the job ("network.speedtest"): while one with the same key
    /// runs, a second press gets [`StartError::AlreadyRunning`].
    pub fn start<F>(&mut self, _pressed: Pressed<'_>, key: &str, work: F) -> Result<JobId, StartError>
    where
        F: FnOnce(&JobCtx) -> Result<String, JobError> + Send + 'static,
    {
        if let Some(j) = self.jobs.iter().find(|j| j.key == key && !is_ended(j)) {
            return Err(StartError::AlreadyRunning(j.id));
        }
        let id = JobId(self.next);
        self.next += 1;
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Mutex::new(Shared { progress: Progress::Busy, status: String::new(), end: None }));
        let ctx = JobCtx { stop: stop.clone(), shared: shared.clone(), waker: self.waker.clone() };
        let thread = std::thread::Builder::new()
            .name(format!("bu-job {key}"))
            .spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&ctx)));
                let end = match r {
                    Ok(Ok(text)) => End::Done(text),
                    Ok(Err(JobError::Stopped)) => End::Stopped,
                    Ok(Err(JobError::Failed(why))) => End::Failed(why),
                    Err(_) => End::Failed("The job crashed".into()),
                };
                ctx.set(|s| {
                    if matches!(end, End::Done(_)) {
                        s.progress = Progress::Part(1.0);
                    }
                    s.end = Some(end);
                });
            })
            .map_err(|e| StartError::NoThread(e.to_string()))?;
        self.jobs.push(Job { id, key: key.into(), stop, shared, thread: Some(thread) });
        Ok(id)
    }

    /// The UI's view of one job (None = unknown / forgotten).
    pub fn view(&self, id: JobId) -> Option<JobView> {
        self.jobs.iter().find(|j| j.id == id).map(view_of)
    }

    /// The newest job with this key (a page finds its job again after being reopened).
    pub fn view_key(&self, key: &str) -> Option<JobView> {
        self.jobs.iter().rev().find(|j| j.key == key).map(view_of)
    }

    /// Jobs not ended yet.
    pub fn running(&self) -> usize {
        self.jobs.iter().filter(|j| !is_ended(j)).count()
    }

    /// Stop button: ask the job to stop (it ends as Stopped when its closure returns).
    pub fn stop(&self, id: JobId) {
        if let Some(j) = self.jobs.iter().find(|j| j.id == id) {
            j.stop.store(true, Ordering::SeqCst);
        }
    }

    pub fn stop_all(&self) {
        for j in &self.jobs {
            j.stop.store(true, Ordering::SeqCst);
        }
    }

    /// Forget an ended job (its thread is joined). A running one is kept.
    pub fn forget(&mut self, id: JobId) {
        if let Some(i) = self.jobs.iter().position(|j| j.id == id && is_ended(j)) {
            let mut j = self.jobs.remove(i);
            if let Some(t) = j.thread.take() {
                let _ = t.join();
            }
        }
    }
}

impl Drop for JobRunner {
    fn drop(&mut self) {
        self.stop_all();
        for j in &mut self.jobs {
            if let Some(t) = j.thread.take() {
                let _ = t.join();
            }
        }
    }
}

fn is_ended(j: &Job) -> bool {
    j.shared.lock().map(|s| s.end.is_some()).unwrap_or(true)
}

fn view_of(j: &Job) -> JobView {
    let (progress, status, end) = match j.shared.lock() {
        Ok(s) => (s.progress, s.status.clone(), s.end.clone()),
        Err(_) => (Progress::Busy, String::new(), Some(End::Failed("The job crashed".into()))),
    };
    JobView { id: j.id, key: j.key.clone(), progress, status, end, stopping: j.stop.load(Ordering::SeqCst) }
}
