//! The Windows side: the "bu-procwatch" thread with its WinEvent hooks, the process snapshot, and the exit waits.

use crate::state::{self, Watch};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, WPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, OpenProcess, OpenProcessToken, RegisterWaitForSingleObject, UnregisterWaitEx, INFINITE, PROCESS_SYNCHRONIZE, WT_EXECUTEONLYONCE,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK, WINEVENTPROC};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EnumWindows, GetAncestor, GetDesktopWindow, GetMessageW, GetWindowThreadProcessId, KillTimer, PeekMessageW,
    PostThreadMessageW, SetTimer, CHILDID_SELF, EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, GA_PARENT, MSG, OBJID_WINDOW, PM_NOREMOVE,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP, WM_TIMER, WM_USER,
};

/// Called for every matching process start: (process id, exe file name as Windows lists it, e.g. "Game.exe").
pub type StartFn = Arc<dyn Fn(u32, String) + Send + Sync>;
/// Called once when the waited-for process ended.
pub type OnExit = Box<dyn FnOnce() + Send>;

/// "Re-read what is wanted" (subscribers / exit waits changed).
const WM_REFRESH: u32 = WM_APP + 1;
/// "Take a snapshot now" (an unseen process made a window, or `rescan`).
const WM_CHECK: u32 = WM_APP + 2;
/// After a window of a process waited for by snapshot is destroyed, one more look this much later: the process may
/// still be ending when the destroy event arrives. A single one-shot timer per destroy burst, never a repeating poll.
const RECHECK_MS: u32 = 1500;

enum Thread {
    Off,
    /// spawned, its message queue not ready yet: it reads the state itself once it is
    Starting,
    On(u32),
}

struct Shared {
    watch: Watch<StartFn, Arc<ExitCtx>>,
    thread: Thread,
    /// the last thing that went wrong (a hook Windows refused, the thread not starting), for diagnostics
    problem: Option<String>,
}

static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn shared() -> MutexGuard<'static, Shared> {
    SHARED
        .get_or_init(|| Mutex::new(Shared { watch: Watch::new(), thread: Thread::Off, problem: None }))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Makes the thread match the state: starts it when there is work and it is not running, or tells the running one to
/// re-read the state (it ends itself when there is nothing left to do).
fn wake(s: &mut Shared) {
    match s.thread {
        // SAFETY: posting to our own thread's queue (it exists: the thread set `On` only after creating it).
        Thread::On(tid) => {
            let _ = unsafe { PostThreadMessageW(tid, WM_REFRESH, WPARAM(0), LPARAM(0)) };
        }
        Thread::Starting => {}
        Thread::Off => {
            if s.watch.idle() {
                return;
            }
            s.thread = Thread::Starting;
            if let Err(e) = std::thread::Builder::new().name("bu-procwatch".into()).spawn(run) {
                s.thread = Thread::Off;
                s.problem = Some(format!("could not start the bu-procwatch thread: {e}"));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Subscribing to starts

/// A live subscription; dropping it unsubscribes (a start already being reported on the watcher thread may still
/// arrive just after the drop).
pub struct Subscription {
    id: u64,
}

/// Calls `on_start` (on the "bu-procwatch" thread) for every process that starts from now on whose exe file name is one
/// of `names` (any case). No names = nothing is hooked for this subscriber.
pub fn subscribe(names: Vec<String>, on_start: StartFn) -> Subscription {
    let id = next_id();
    let mut s = shared();
    s.watch.subscribe(id, &names, on_start);
    wake(&mut s);
    Subscription { id }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut s = shared();
        let sink = s.watch.unsubscribe(self.id);
        wake(&mut s);
        drop(s);
        // dropped outside the lock: the sink may own things whose drop takes this lock again
        drop(sink);
    }
}

/// Takes a process snapshot now, as a new window of an unseen process would (proofs and tests; does nothing while no
/// one listens).
pub fn rescan() {
    if let Thread::On(tid) = shared().thread {
        // SAFETY: posting to the watcher thread's queue.
        let _ = unsafe { PostThreadMessageW(tid, WM_CHECK, WPARAM(0), LPARAM(0)) };
    }
}

/// What last went wrong in the watcher (a hook Windows refused, the thread not starting), if anything.
pub fn problem() -> Option<String> {
    shared().problem.clone()
}

// ---------------------------------------------------------------------------------------------------------------
// Exit waits

/// One exit callback, shared by whoever may fire it (the thread pool, the watcher thread) and the `ExitWait` that may
/// cancel it. The callback runs while `f`'s lock is held, so a cancel from another thread waits for a running callback
/// to finish: nothing is ever called after `ExitWait` was dropped.
struct ExitCtx {
    f: Mutex<Option<OnExit>>,
    ended: AtomicBool,
}

thread_local! {
    /// The exit callback running on this thread right now (its `ExitCtx` address), so dropping its own `ExitWait` from
    /// inside it neither waits for itself nor deadlocks.
    static FIRING: Cell<usize> = const { Cell::new(0) };
}

impl ExitCtx {
    fn addr(&self) -> usize {
        self as *const Self as usize
    }

    fn fire(&self) {
        let mut g = self.f.lock().unwrap_or_else(|e| e.into_inner());
        let Some(f) = g.take() else { return };
        self.ended.store(true, Ordering::SeqCst);
        let prev = FIRING.with(|c| c.replace(self.addr()));
        f();
        FIRING.with(|c| c.set(prev));
        drop(g);
    }

    fn firing_here(&self) -> bool {
        FIRING.with(|c| c.get()) == self.addr()
    }

    /// Never call it from now on (waits if it is running on another thread right now).
    fn cancel(&self) {
        if self.firing_here() {
            return;
        }
        let f = self.f.lock().unwrap_or_else(|e| e.into_inner()).take();
        drop(f);
    }
}

enum How {
    /// already reported (the process was gone when the wait began) or cancelled
    Done,
    /// the thread pool waits on the SYNCHRONIZE handle (handles kept as numbers so the value can move threads)
    Os { wait: usize, process: usize, ctx_ref: usize },
    /// the process refused SYNCHRONIZE: snapshots decide
    Snapshots { id: u64 },
}

/// A running exit wait. Dropping it cancels it: `on_exit` is never called after the drop.
pub struct ExitWait {
    ctx: Arc<ExitCtx>,
    how: How,
}

impl ExitWait {
    /// True once `on_exit` was called (or is running): the process ended. Lets the caller skip keeping a finished wait.
    pub fn ended(&self) -> bool {
        self.ctx.ended.load(Ordering::SeqCst)
    }
}

/// Calls `on_exit` once when process `pid` ends. Opens ONLY a SYNCHRONIZE handle (A_004_01) and lets the Windows thread
/// pool wait on it. If even that is refused, snapshots decide (see the crate docs). A process already gone is reported
/// at once, on the caller's thread, before this returns.
pub fn wait_exit(pid: u32, on_exit: OnExit) -> ExitWait {
    let ctx = Arc::new(ExitCtx { f: Mutex::new(Some(on_exit)), ended: AtomicBool::new(false) });
    // SAFETY: SYNCHRONIZE only - no read, no query, nothing else.
    if let Ok(process) = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
        // The registration owns one reference; given back in `ExitWait::drop` after the wait is unregistered.
        let raw = Arc::into_raw(ctx.clone());
        let mut wait = HANDLE::default();
        // SAFETY: `raw` stays valid until the wait is unregistered (drop); the callback takes its own reference.
        let ok = unsafe { RegisterWaitForSingleObject(&mut wait, process, Some(on_signalled), Some(raw as *const c_void), INFINITE, WT_EXECUTEONLYONCE) };
        if ok.is_ok() {
            return ExitWait { ctx, how: How::Os { wait: wait.0 as usize, process: process.0 as usize, ctx_ref: raw as usize } };
        }
        // SAFETY: not registered: the reference is ours again; the handle is ours.
        unsafe {
            drop(Arc::from_raw(raw));
            let _ = CloseHandle(process);
        }
    }
    let name = match snapshot() {
        Some(procs) => match state::alive_in(&procs, pid) {
            Some(n) => Some(n.to_string()),
            None => {
                // Already gone (a short-lived exe, or noticed late): say so now, or the exit would never come.
                ctx.fire();
                return ExitWait { ctx, how: How::Done };
            }
        },
        None => None,
    };
    let id = next_id();
    let mut s = shared();
    s.watch.add_exit(id, pid, name.as_deref(), ctx.clone());
    wake(&mut s);
    ExitWait { ctx, how: How::Snapshots { id } }
}

/// The thread pool's callback: the process handle was signalled (the process ended).
unsafe extern "system" fn on_signalled(param: *mut c_void, _timed_out: bool) {
    let p = param as *const ExitCtx;
    // SAFETY: `p` came from `Arc::into_raw` and is alive while the wait is registered; take our own reference so a drop
    // from inside the callback (which does not wait for us) cannot free it under our feet.
    let ctx = unsafe {
        Arc::increment_strong_count(p);
        Arc::from_raw(p)
    };
    ctx.fire();
}

impl Drop for ExitWait {
    fn drop(&mut self) {
        let inside = self.ctx.firing_here();
        self.ctx.cancel();
        match std::mem::replace(&mut self.how, How::Done) {
            How::Done => {}
            How::Os { wait, process, ctx_ref } => {
                // Wait for a running callback to finish (INVALID_HANDLE_VALUE), except from inside our own callback,
                // where that would wait for itself.
                let done = if inside { None } else { Some(INVALID_HANDLE_VALUE) };
                // SAFETY: our own registration and handle, released exactly once (`how` is now Done).
                unsafe {
                    let _ = UnregisterWaitEx(HANDLE(wait as *mut c_void), done);
                    let _ = CloseHandle(HANDLE(process as *mut c_void));
                    drop(Arc::from_raw(ctx_ref as *const ExitCtx));
                }
            }
            How::Snapshots { id } => {
                let mut s = shared();
                let gone = s.watch.remove_exit(id);
                wake(&mut s);
                drop(s);
                drop(gone);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The process snapshot

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|c| *c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

/// (pid, exe file name) of every running process, from one Toolhelp snapshot. No process handle is opened.
/// None when Windows refused the snapshot (then nothing is decided from it).
fn snapshot() -> Option<Vec<(u32, String)>> {
    // SAFETY: plain snapshot calls on our own snapshot handle, closed below.
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;
    let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut out = None;
    if unsafe { Process32FirstW(snap, &mut e) }.is_ok() {
        let mut v = Vec::with_capacity(512);
        loop {
            v.push((e.th32ProcessID, wide_to_string(&e.szExeFile)));
            if unsafe { Process32NextW(snap, &mut e) }.is_err() {
                break;
            }
        }
        out = Some(v);
    }
    let _ = unsafe { CloseHandle(snap) };
    out
}

// ---------------------------------------------------------------------------------------------------------------
// The watcher thread

/// State only the watcher thread touches (its hook callbacks run on it).
#[derive(Default)]
struct Local {
    tid: u32,
    /// a WM_CHECK is already queued (many new windows in a burst = one snapshot)
    check_queued: bool,
    /// processes whose exit is decided by snapshots, and the threads that own their windows (a destroyed window can no
    /// longer be asked for its process, but the event names the thread)
    exit_pids: HashSet<u32>,
    exit_threads: HashMap<u32, u32>,
    /// the one-shot re-check timer (0 = none)
    recheck: usize,
}

thread_local! {
    static LOCAL: RefCell<Local> = RefCell::new(Local::default());
}

/// Runs `f` on the thread's state; skipped (never a panic) if a callback ever re-entered while it is in use.
fn local<R>(f: impl FnOnce(&mut Local) -> R) -> Option<R> {
    LOCAL.with(|c| c.try_borrow_mut().ok().map(|mut l| f(&mut l)))
}

#[derive(Default)]
struct Hooks {
    create: Option<HWINEVENTHOOK>,
    destroy: Option<HWINEVENTHOOK>,
}

fn hook(event: u32, proc_: WINEVENTPROC) -> Option<HWINEVENTHOOK> {
    // SAFETY: an out-of-context WinEvent hook (an accessibility notification delivered to this thread's message loop;
    // nothing is injected into other processes). Our own windows are skipped.
    let h = unsafe { SetWinEventHook(event, event, None, proc_, 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS) };
    (!h.is_invalid()).then_some(h)
}

fn unhook(h: &mut Option<HWINEVENTHOOK>) {
    if let Some(x) = h.take() {
        // SAFETY: our own hook, removed once.
        let _ = unsafe { UnhookWinEvent(x) };
    }
}

fn run() {
    let mut msg = MSG::default();
    // SAFETY: creates this thread's message queue before anyone is told our id (PostThreadMessageW needs it).
    unsafe {
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
    }
    let tid = unsafe { GetCurrentThreadId() };
    local(|l| l.tid = tid);
    shared().thread = Thread::On(tid);
    let mut hooks = Hooks::default();
    while refresh(&mut hooks) {
        // SAFETY: a plain message loop; the hook callbacks run inside GetMessageW.
        let r = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if r.0 == 0 || r.0 == -1 {
            let mut s = shared();
            s.thread = Thread::Off;
            s.watch.forget();
            if r.0 == -1 {
                s.problem = Some("bu-procwatch: GetMessageW failed".into());
            }
            break;
        }
        if msg.hwnd.is_invalid() {
            match msg.message {
                WM_REFRESH => continue,
                WM_CHECK => {
                    local(|l| l.check_queued = false);
                    check_now();
                    continue;
                }
                WM_TIMER => {
                    let id = local(|l| std::mem::take(&mut l.recheck)).unwrap_or(0);
                    // SAFETY: our own thread timer.
                    let _ = unsafe { KillTimer(None, msg.wParam.0) };
                    if id != 0 {
                        check_now();
                    }
                    continue;
                }
                _ => {}
            }
        }
        // SAFETY: nothing else is expected; hand it on like any message loop.
        unsafe { DispatchMessageW(&msg) };
    }
    unhook(&mut hooks.create);
    unhook(&mut hooks.destroy);
    local(|l| {
        if l.recheck != 0 {
            // SAFETY: our own thread timer.
            let _ = unsafe { KillTimer(None, l.recheck) };
        }
        *l = Local::default();
    });
}

/// Brings the hooks in line with what is wanted. False = nothing is wanted any more: the thread ends (the state is
/// marked Off under the lock first, so a new subscriber starts a fresh thread instead of posting to this one).
fn refresh(h: &mut Hooks) -> bool {
    let (exit_pids, added) = {
        let mut s = shared();
        if s.watch.idle() {
            s.thread = Thread::Off;
            s.watch.forget();
            return false;
        }
        (s.watch.exit_pids(), s.watch.take_exits_added())
    };
    let mut check = added;
    // The creation hook runs whenever the thread runs: for starts, and because every snapshot also checks the exits.
    if h.create.is_none() {
        h.create = hook(EVENT_OBJECT_CREATE, Some(on_create));
        match h.create {
            // just installed: the first snapshot records what already runs
            Some(_) => check = true,
            None => shared().problem = Some("Windows refused SetWinEventHook(EVENT_OBJECT_CREATE)".into()),
        }
    }
    if exit_pids.is_empty() {
        unhook(&mut h.destroy);
    } else if h.destroy.is_none() {
        h.destroy = hook(EVENT_OBJECT_DESTROY, Some(on_destroy));
        if h.destroy.is_none() {
            shared().problem = Some("Windows refused SetWinEventHook(EVENT_OBJECT_DESTROY)".into());
        }
    }
    let pids: HashSet<u32> = exit_pids.into_iter().collect();
    let changed = local(|l| {
        if l.exit_pids == pids {
            return false;
        }
        l.exit_pids = pids.clone();
        true
    })
    .unwrap_or(false);
    if changed {
        let threads = window_threads_of(&pids);
        local(|l| l.exit_threads = threads);
    }
    if check {
        check_now();
    }
    true
}

/// One snapshot: report the new matching processes and the ended fallback exits. The lock is held only to update the
/// state; the sinks and callbacks run after it is released.
fn check_now() {
    let Some(procs) = snapshot() else { return };
    let report = shared().watch.apply(&procs);
    for (sink, pid, exe) in report.starts {
        sink(pid, exe);
    }
    for ctx in report.exits {
        ctx.fire();
    }
}

fn queue_check() {
    local(|l| {
        if !l.check_queued {
            l.check_queued = true;
            // SAFETY: posting to our own queue; handled by the loop, off the hook callback's stack.
            let _ = unsafe { PostThreadMessageW(l.tid, WM_CHECK, WPARAM(0), LPARAM(0)) };
        }
    });
}

/// (window thread -> process) for every top-level window of these processes.
fn window_threads_of(pids: &HashSet<u32>) -> HashMap<u32, u32> {
    struct F<'a> {
        pids: &'a HashSet<u32>,
        out: HashMap<u32, u32>,
    }
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> BOOL {
        let f = unsafe { &mut *(lp.0 as *mut F) };
        let mut pid = 0u32;
        let tid = unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
        if f.pids.contains(&pid) {
            f.out.insert(tid, pid);
        }
        BOOL(1)
    }
    let mut f = F { pids, out: HashMap::new() };
    if !pids.is_empty() {
        // SAFETY: `f` outlives the call; the callback only reads window owners.
        let _ = unsafe { EnumWindows(Some(cb), LPARAM(&mut f as *mut F as isize)) };
    }
    f.out
}

/// A new window somewhere. Fires for every object created anywhere, so it filters cheaply and returns early.
unsafe extern "system" fn on_create(_h: HWINEVENTHOOK, _event: u32, hwnd: HWND, id_object: i32, id_child: i32, _thread: u32, _time: u32) {
    if id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 || hwnd.is_invalid() {
        return;
    }
    // top-level windows only (child controls are created by the thousand)
    if unsafe { GetAncestor(hwnd, GA_PARENT) } != unsafe { GetDesktopWindow() } {
        return;
    }
    let mut pid = 0u32;
    let tid = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return;
    }
    local(|l| {
        if l.exit_pids.contains(&pid) {
            l.exit_threads.insert(tid, pid);
        }
    });
    if shared().watch.is_seen(pid) {
        return;
    }
    queue_check();
}

/// A window was destroyed (hooked only while some exit is decided by snapshots): one of theirs = look now, and once more
/// a moment later (the process may still be ending).
unsafe extern "system" fn on_destroy(_h: HWINEVENTHOOK, _event: u32, hwnd: HWND, id_object: i32, id_child: i32, thread: u32, _time: u32) {
    if id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 {
        return;
    }
    let mut pid = 0u32;
    if !hwnd.is_invalid() {
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    }
    let theirs = local(|l| (pid != 0 && l.exit_pids.contains(&pid)) || l.exit_threads.contains_key(&thread)).unwrap_or(false);
    if !theirs {
        return;
    }
    queue_check();
    local(|l| {
        // SAFETY: our own one-shot thread timer (re-aimed, never repeating: killed when it fires).
        unsafe {
            if l.recheck != 0 {
                let _ = KillTimer(None, l.recheck);
            }
            l.recheck = SetTimer(None, 0, RECHECK_MS, None);
        }
    });
}

// ---------------------------------------------------------------------------------------------------------------

/// True when this process runs elevated (its token says so). The switchers use the exact admin-only WMI start trace
/// only then, so a normal start never touches WMI at all.
pub fn is_elevated() -> bool {
    let mut token = HANDLE::default();
    // SAFETY: a query-only handle to our own process token, closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return false;
    }
    let mut e = TOKEN_ELEVATION::default();
    let mut len = 0u32;
    let ok = unsafe {
        GetTokenInformation(token, TokenElevation, Some(&mut e as *mut TOKEN_ELEVATION as *mut c_void), size_of::<TOKEN_ELEVATION>() as u32, &mut len)
    }
    .is_ok();
    let _ = unsafe { CloseHandle(token) };
    ok && e.TokenIsElevated != 0
}
