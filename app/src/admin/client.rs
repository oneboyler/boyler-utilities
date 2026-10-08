//! The normal app's side: start the elevated copy (one admin prompt), send it ops, read its answers.
//!
//! - [`Admin::call`]: one op. Inside a [`Scope`] of the op's purpose (or a `Reset` scope that allows it) it goes to that
//!   scope's copy - started at the scope's first op, ended with the scope - so one user action is one prompt; without a
//!   scope it is a short copy of its own. A prompt answered No ends the scope's admin part: later ops in it fail at once
//!   ([`AdminError::Declined`]), no second prompt.
//! - [`Admin::spawn`]: DISM / sfc in the copy; its output comes back as a stream ([`Remote`]).
//! - Who starts the copy is a [`Launcher`]: the real one (`ShellExecuteEx "runas"`, or in this process when the app
//!   itself runs elevated), the [`Decline`] one (test copies, unit tests: never a prompt), or a test's in-process helper.
//! - [`admin`] = the app's one hub (a unit test thread gets its own).

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicIsize, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use super::wire::{self, Rx, Tx};
use super::{AdminError, Op, Prog, Purpose, Reply};

/// Starts an elevated copy for a purpose and hands back its line.
pub trait Launcher: Send + Sync {
    fn launch(&self, purpose: Purpose) -> Result<(Box<dyn Tx>, Box<dyn Rx>), AdminError>;
}

/// Never starts anything: every op is "declined" (test copies of the app, unit tests that don't install a helper).
pub struct Decline;

impl Launcher for Decline {
    fn launch(&self, _: Purpose) -> Result<(Box<dyn Tx>, Box<dyn Rx>), AdminError> {
        Err(AdminError::Declined)
    }
}

/// Windows' admin prompt is up (any thread): the menu losing the focus to it is not a click outside (main.rs).
static PROMPTING: AtomicUsize = AtomicUsize::new(0);
/// The menu window, the prompt's owner (so it comes up in front of the menu).
static OWNER: AtomicIsize = AtomicIsize::new(0);

pub fn prompting() -> bool {
    PROMPTING.load(Ordering::SeqCst) > 0
}

pub fn set_owner(hwnd: isize) {
    OWNER.store(hwnd, Ordering::SeqCst);
}

/// The real launcher.
pub struct Real;

impl Launcher for Real {
    fn launch(&self, purpose: Purpose) -> Result<(Box<dyn Tx>, Box<dyn Rx>), AdminError> {
        if is_elevated() {
            // the app itself runs as admin: no prompt, the same helper in this process
            let sid = wire::process_user_sid(std::process::id()).unwrap_or_default();
            return Ok(in_process(purpose, Box::new(move || Box::new(super::exec::RealSys::new(&sid)) as Box<dyn super::exec::Sys>)));
        }
        // Order 047: the prompt waits for the user (seconds) and the copy's line up to 60 s: never on the menu's thread (the
        // services live there) - a caller that still does it is named in the timing log, so it can be found and moved
        if crate::services::in_use() || crate::services::try_with(|_| ()).is_some() {
            crate::timing::note(&format!("admin prompt on the menu's thread ({}) - Order 047: move its caller off it", purpose.name()));
        }
        let server = wire::Server::create().map_err(|e| AdminError::Failed(format!("The admin helper could not start ({e})")))?;
        let exe = std::env::current_exe().map_err(|e| AdminError::Failed(e.to_string()))?;
        let child = run_as(&exe, &format!("{} {} {}", super::ARG, purpose.name(), server.id()))?;
        let r = server.accept(child.0, std::time::Duration::from_secs(60), &|| false);
        drop(child);
        let (tx, rx) = r.map_err(|e| AdminError::Failed(e.to_string()))?;
        Ok((Box::new(tx), Box::new(rx)))
    }
}

struct Proc(windows::Win32::Foundation::HANDLE);
impl Drop for Proc {
    fn drop(&mut self) {
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

/// `ShellExecuteEx "runas"` (Windows' admin prompt), hidden; No → Declined.
fn run_as(exe: &std::path::Path, params: &str) -> Result<Proc, AdminError> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    let (verb, file, params) = (HSTRING::from("runas"), HSTRING::from(exe.as_os_str()), HSTRING::from(params));
    // ShellExecuteEx wants COM on its thread
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok();
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        hwnd: HWND(OWNER.load(Ordering::SeqCst) as *mut _),
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    PROMPTING.fetch_add(1, Ordering::SeqCst);
    // SAFETY: sei and the strings it points to live across the call.
    let r = unsafe { ShellExecuteExW(&mut sei) };
    PROMPTING.fetch_sub(1, Ordering::SeqCst);
    if com {
        unsafe { CoUninitialize() };
    }
    match r {
        Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => Err(AdminError::Declined),
        Err(e) => Err(AdminError::Failed(format!("The admin helper did not start: {}", e.message()))),
        Ok(()) if sei.hProcess.is_invalid() => Err(AdminError::Failed("The admin helper did not start".into())),
        Ok(()) => Ok(Proc(sei.hProcess)),
    }
}

/// Does this process run elevated?
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    // SAFETY: the token handle is closed below; the struct is local.
    unsafe {
        let mut tok = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok).is_err() {
            return false;
        }
        let mut e = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(tok, TokenElevation, Some(&mut e as *mut _ as *mut _), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut len).is_ok();
        let _ = CloseHandle(tok);
        ok && e.TokenIsElevated != 0
    }
}

/// The helper in this process on its own thread, over an in-memory line (an elevated app; tests).
pub fn in_process(purpose: Purpose, make: Box<dyn FnOnce() -> Box<dyn super::exec::Sys> + Send>) -> (Box<dyn Tx>, Box<dyn Rx>) {
    let (to_helper, mut helper_rx) = wire::mem_pair();
    let (helper_tx, from_helper) = wire::mem_pair();
    std::thread::Builder::new()
        .name("bu-admin-inproc".into())
        .spawn(move || {
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
            }
            let mut sys = make();
            super::helper::serve(purpose, &mut helper_rx, Arc::new(helper_tx), sys.as_mut())
        })
        .ok();
    (Box::new(to_helper), Box::new(from_helper))
}

// ------------------------------------------------------------------ one copy's line

enum Chunk {
    Data(Vec<u8>),
    End(u32),
}

struct StreamSlot {
    tx: Sender<Chunk>,
    rx: Option<Receiver<Chunk>>,
}

#[derive(Default)]
struct Shared {
    pending: HashMap<u64, Sender<Reply>>,
    streams: HashMap<u32, StreamSlot>,
    closed: bool,
}

/// One running elevated copy. Dropping the last `Arc` closes its line: the copy exits.
pub struct Session {
    tx: Mutex<Option<Box<dyn Tx>>>,
    shared: Arc<Mutex<Shared>>,
    seq: AtomicU64,
}

impl Session {
    pub fn new(tx: Box<dyn Tx>, mut rx: Box<dyn Rx>) -> Arc<Session> {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let sh = shared.clone();
        std::thread::Builder::new()
            .name("bu-admin-read".into())
            .spawn(move || {
                while let Ok(Some(frame)) = rx.recv() {
                    if !dispatch(&sh, frame) {
                        break;
                    }
                }
                // the copy is gone: every waiting call fails, every stream ends
                let mut g = sh.lock().unwrap();
                g.closed = true;
                for (_, s) in g.pending.drain() {
                    let _ = s.send(Err(AdminError::Failed("The admin helper stopped".into())));
                }
                for s in g.streams.values() {
                    let _ = s.tx.send(Chunk::End(u32::MAX));
                }
            })
            .ok();
        Arc::new(Session { tx: Mutex::new(Some(tx)), shared, seq: AtomicU64::new(0) })
    }

    /// Send one op and wait for its answer.
    pub fn call(&self, op: &Op) -> Reply {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = channel();
        {
            let mut g = self.shared.lock().unwrap();
            if g.closed {
                return Err(AdminError::Failed("The admin helper stopped".into()));
            }
            g.pending.insert(seq, tx);
        }
        let mut f: Vec<String> = vec!["op".into(), seq.to_string()];
        f.extend(op.fields());
        let bytes: Vec<&[u8]> = f.iter().map(|s| s.as_bytes()).collect();
        let sent = match self.tx.lock().unwrap().as_ref() {
            Some(t) => t.send(&bytes),
            None => Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed")),
        };
        if let Err(e) = sent {
            self.shared.lock().unwrap().pending.remove(&seq);
            return Err(AdminError::Failed(format!("The admin helper stopped ({e})")));
        }
        rx.recv().unwrap_or_else(|_| Err(AdminError::Failed("The admin helper stopped".into())))
    }

    fn take_stream(&self, sid: u32) -> Receiver<Chunk> {
        let mut g = self.shared.lock().unwrap();
        let slot = g.streams.entry(sid).or_insert_with(|| {
            let (tx, rx) = channel();
            StreamSlot { tx, rx: Some(rx) }
        });
        slot.rx.take().unwrap_or_else(|| channel().1)
    }
}

/// One frame from the copy. false = it can't be trusted any more (stop reading).
fn dispatch(sh: &Arc<Mutex<Shared>>, frame: wire::Frame) -> bool {
    let Some(kind) = frame.first().map(|k| k.as_slice()) else { return false };
    let text = |i: usize| frame.get(i).and_then(|b| std::str::from_utf8(b).ok()).map(str::to_string);
    let mut g = sh.lock().unwrap();
    match kind {
        b"ok" | b"err" => {
            let Some(seq) = text(1).and_then(|s| s.parse::<u64>().ok()) else { return false };
            let Some(waiter) = g.pending.remove(&seq) else { return true };
            let reply = if kind == b"ok" {
                match wire::texts(&frame[2..]) {
                    Some(f) => Ok(f),
                    None => Err(AdminError::Failed("bad answer".into())),
                }
            } else {
                let msg = text(3).unwrap_or_default();
                Err(match text(2).as_deref() {
                    Some("refused") => AdminError::Refused(msg),
                    Some("denied") => AdminError::Denied(msg),
                    Some("notfound") => AdminError::NotFound(msg),
                    _ => AdminError::Failed(msg),
                })
            };
            let _ = waiter.send(reply);
            true
        }
        b"out" | b"end" => {
            let Some(sid) = text(1).and_then(|s| s.parse::<u32>().ok()) else { return false };
            let slot = g.streams.entry(sid).or_insert_with(|| {
                let (tx, rx) = channel();
                StreamSlot { tx, rx: Some(rx) }
            });
            let chunk = if kind == b"out" {
                Chunk::Data(frame.get(2).cloned().unwrap_or_default())
            } else {
                Chunk::End(text(2).and_then(|s| s.parse().ok()).unwrap_or(u32::MAX))
            };
            let _ = slot.tx.send(chunk);
            true
        }
        _ => false,
    }
}

/// A program running in the copy: its output (Read) and its control ([`bu_quickfix::ProcCtl`]: kill = Cancel).
pub struct Remote {
    rx: Mutex<Receiver<Chunk>>,
    buf: Mutex<(Vec<u8>, usize)>,
    end: Mutex<Option<u32>>,
    sid: u32,
    session: Arc<Session>,
}

impl Remote {
    fn next(&self) -> Option<Chunk> {
        self.rx.lock().unwrap().recv().ok()
    }
}

/// The output side of a [`Remote`].
pub struct RemoteOut(pub Arc<Remote>);

impl io::Read for RemoteOut {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let r = &self.0;
        loop {
            {
                let mut b = r.buf.lock().unwrap();
                if b.1 < b.0.len() {
                    let n = out.len().min(b.0.len() - b.1);
                    out[..n].copy_from_slice(&b.0[b.1..b.1 + n]);
                    b.1 += n;
                    return Ok(n);
                }
            }
            if r.end.lock().unwrap().is_some() {
                return Ok(0);
            }
            match r.next() {
                Some(Chunk::Data(d)) => *r.buf.lock().unwrap() = (d, 0),
                Some(Chunk::End(c)) => *r.end.lock().unwrap() = Some(c),
                None => *r.end.lock().unwrap() = Some(u32::MAX),
            }
        }
    }
}

impl bu_quickfix::ProcCtl for Remote {
    fn kill(&self) {
        let _ = self.session.call(&Op::Kill(self.sid));
    }
    fn wait(&self) -> bu_quickfix::Result<u32> {
        loop {
            if let Some(c) = *self.end.lock().unwrap() {
                return if c == u32::MAX { Err(bu_quickfix::FixError::Os { context: "The admin helper stopped".into(), code: 0 }) } else { Ok(c) };
            }
            match self.next() {
                Some(Chunk::Data(_)) => {} // output nobody reads any more
                Some(Chunk::End(c)) => *self.end.lock().unwrap() = Some(c),
                None => *self.end.lock().unwrap() = Some(u32::MAX),
            }
        }
    }
}

// ------------------------------------------------------------------ the hub

#[derive(Clone)]
enum Slot {
    /// no copy yet (started at the first op)
    Idle,
    Open(Arc<Session>),
    /// the prompt was answered No / the copy failed to start: later ops of this scope fail at once
    Gone(AdminError),
}

struct ScopeEntry {
    purpose: Purpose,
    users: usize,
    slot: Arc<Mutex<Slot>>,
}

/// The app's way to its elevated copies.
pub struct Admin {
    launcher: Box<dyn Launcher>,
    scopes: Mutex<Vec<ScopeEntry>>,
}

/// While alive, ops of its purpose share one copy (one prompt). Dropping the last one of a purpose closes the copy.
pub struct Scope {
    admin: Arc<Admin>,
    purpose: Purpose,
}

impl Drop for Scope {
    fn drop(&mut self) {
        let mut g = self.admin.scopes.lock().unwrap();
        if let Some(i) = g.iter().position(|e| e.purpose == self.purpose) {
            g[i].users -= 1;
            if g[i].users == 0 {
                g.remove(i);
            }
        }
    }
}

impl Admin {
    pub fn new(launcher: Box<dyn Launcher>) -> Arc<Admin> {
        Arc::new(Admin { launcher, scopes: Mutex::new(Vec::new()) })
    }

    /// One user action of this purpose: every op until the scope ends goes to one copy.
    pub fn scope(self: &Arc<Self>, purpose: Purpose) -> Scope {
        let mut g = self.scopes.lock().unwrap();
        match g.iter_mut().find(|e| e.purpose == purpose) {
            Some(e) => e.users += 1,
            None => g.push(ScopeEntry { purpose, users: 1, slot: Arc::new(Mutex::new(Slot::Idle)) }),
        }
        Scope { admin: self.clone(), purpose }
    }

    /// Is a scope of this purpose open?
    pub fn has_scope(&self, purpose: Purpose) -> bool {
        self.scopes.lock().unwrap().iter().any(|e| e.purpose == purpose)
    }

    /// The scope slot an op of `purpose` uses: an open Reset scope that allows it, else its own purpose's scope.
    fn slot_for(&self, purpose: Purpose, op: &Op) -> Option<(Purpose, Arc<Mutex<Slot>>)> {
        let g = self.scopes.lock().unwrap();
        g.iter()
            .find(|e| e.purpose == Purpose::Reset && Purpose::Reset.allows(op))
            .or_else(|| g.iter().find(|e| e.purpose == purpose))
            .map(|e| (e.purpose, e.slot.clone()))
    }

    fn open(&self, purpose: Purpose) -> Result<Arc<Session>, AdminError> {
        let (tx, rx) = self.launcher.launch(purpose)?;
        Ok(Session::new(tx, rx))
    }

    /// The session for an op: the scope's (started now if it has none), or a fresh one-op copy.
    fn session(&self, purpose: Purpose, op: &Op) -> Result<Arc<Session>, AdminError> {
        let Some((p, slot)) = self.slot_for(purpose, op) else { return self.open(purpose) };
        // the slot's lock is held while the prompt is up: a second op of the same action waits for the same copy
        let mut s = slot.lock().unwrap();
        match &*s {
            Slot::Open(sess) => return Ok(sess.clone()),
            Slot::Gone(e) => return Err(e.clone()),
            Slot::Idle => {}
        }
        match self.open(p) {
            Ok(sess) => {
                *s = Slot::Open(sess.clone());
                Ok(sess)
            }
            Err(e) => {
                *s = Slot::Gone(e.clone());
                Err(e)
            }
        }
    }

    /// Run one op as admin.
    pub fn call(&self, purpose: Purpose, op: Op) -> Reply {
        self.session(purpose, &op)?.call(&op)
    }

    /// Start DISM / sfc as admin; the output streams back.
    pub fn spawn(&self, purpose: Purpose, prog: Prog) -> Result<Arc<Remote>, AdminError> {
        let op = Op::Spawn(prog);
        let sess = self.session(purpose, &op)?;
        let f = sess.call(&op)?;
        let sid: u32 = f.first().and_then(|s| s.parse().ok()).ok_or_else(|| AdminError::Failed("bad answer".into()))?;
        let rx = sess.take_stream(sid);
        Ok(Arc::new(Remote { rx: Mutex::new(rx), buf: Mutex::new((Vec::new(), 0)), end: Mutex::new(None), sid, session: sess }))
    }
}

#[cfg(not(test))]
static ADMIN: std::sync::OnceLock<Arc<Admin>> = std::sync::OnceLock::new();

/// The app's hub: the real launcher, or [`Decline`] in a test copy (a test copy never asks for admin).
#[cfg(not(test))]
pub fn admin() -> Arc<Admin> {
    ADMIN.get_or_init(|| if crate::testmode::on() { Admin::new(Box::new(Decline)) } else { Admin::new(Box::new(Real)) }).clone()
}

#[cfg(test)]
thread_local! {
    static ADMIN_T: std::cell::RefCell<Option<Arc<Admin>>> = const { std::cell::RefCell::new(None) };
}

/// Unit tests: this test thread's hub ([`Decline`] unless the test set one with [`set_for_test`]).
#[cfg(test)]
pub fn admin() -> Arc<Admin> {
    ADMIN_T.with(|a| a.borrow_mut().get_or_insert_with(|| Admin::new(Box::new(Decline))).clone())
}

#[cfg(test)]
pub fn set_for_test(a: Arc<Admin>) {
    ADMIN_T.with(|x| *x.borrow_mut() = Some(a));
}
