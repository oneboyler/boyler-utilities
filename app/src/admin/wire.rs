//! The line between the normal app and its elevated copy: frames (a list of byte fields) over TWO one-way named pipes -
//! `…-req` (app → copy: the ops) and `…-rep` (copy → app: answers and program output). Two pipes, because one synchronous
//! pipe handle serialises its reads and writes: a copy streaming DISM's output must still hear a Cancel.
//!
//! - The app makes both pipes (`FILE_FLAG_FIRST_PIPE_INSTANCE`, one instance each, local clients only, access for
//!   SYSTEM / Administrators / the owner) with a random 128-bit name, starts the copy, and accepts a client only if it
//!   IS that copy (`GetNamedPipeClientProcessId` = the started process).
//! - The copy opens them with `SECURITY_IDENTIFICATION` (the pipe's maker can never act as the admin it talks to) and
//!   talks only to a pipe made by the app's own exe (`GetNamedPipeServerProcessId` → that process's image = our image);
//!   it reads the clicking user's SID from that process (HKCU writes go to that user's hive).
//! - Frame = u32 LE byte length, then per field u32 LE length + bytes. At most [`MAX_FRAME`] bytes, [`MAX_FIELDS`] fields.
//! - Tests use [`mem_pair`] (channels, no pipes).

use std::io::{self, Read, Write};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

pub const MAX_FRAME: usize = 1 << 20;
pub const MAX_FIELDS: usize = 4096;

pub type Frame = Vec<Vec<u8>>;

/// Sends frames (any thread).
pub trait Tx: Send + Sync {
    fn send(&self, f: &[&[u8]]) -> io::Result<()>;
}

/// Receives frames (one reader). `Ok(None)` = the other side closed.
pub trait Rx: Send {
    fn recv(&mut self) -> io::Result<Option<Frame>>;
}

pub fn encode(f: &[&[u8]]) -> io::Result<Vec<u8>> {
    let body: usize = f.iter().map(|x| 4 + x.len()).sum();
    if body > MAX_FRAME || f.len() > MAX_FIELDS {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too big"));
    }
    let mut out = Vec::with_capacity(4 + body);
    out.extend_from_slice(&(body as u32).to_le_bytes());
    for x in f {
        out.extend_from_slice(&(x.len() as u32).to_le_bytes());
        out.extend_from_slice(x);
    }
    Ok(out)
}

/// One frame from a byte stream. `Ok(None)` = the stream ended cleanly between frames.
pub fn read_frame(r: &mut dyn Read) -> io::Result<Option<Frame>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof || e.kind() == io::ErrorKind::BrokenPipe => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too big"));
    }
    let mut body = vec![0u8; n];
    r.read_exact(&mut body)?;
    decode_body(&body).map(Some)
}

pub fn decode_body(body: &[u8]) -> io::Result<Frame> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "bad frame");
    let mut out = Vec::new();
    let mut i = 0;
    while i < body.len() {
        if out.len() >= MAX_FIELDS || i + 4 > body.len() {
            return Err(bad());
        }
        let l = u32::from_le_bytes(body[i..i + 4].try_into().unwrap()) as usize;
        i += 4;
        if l > body.len() - i {
            return Err(bad());
        }
        out.push(body[i..i + l].to_vec());
        i += l;
    }
    Ok(out)
}

/// The fields as text (every field must be UTF-8).
pub fn texts(f: &[Vec<u8>]) -> Option<Vec<String>> {
    f.iter().map(|x| String::from_utf8(x.clone()).ok()).collect()
}

// ------------------------------------------------------------------ in memory (tests)

pub struct MemTx(Mutex<Sender<Frame>>);
pub struct MemRx(Receiver<Frame>);

impl Tx for MemTx {
    fn send(&self, f: &[&[u8]]) -> io::Result<()> {
        // the same limits as the pipe
        let body = encode(f)?;
        let frame = decode_body(&body[4..])?;
        self.0.lock().unwrap().send(frame).map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }
}

impl Rx for MemRx {
    fn recv(&mut self) -> io::Result<Option<Frame>> {
        Ok(self.0.recv().ok())
    }
}

/// A one-way in-memory line.
pub fn mem_pair() -> (MemTx, MemRx) {
    let (tx, rx) = channel();
    (MemTx(Mutex::new(tx)), MemRx(rx))
}

// ------------------------------------------------------------------ named pipes

pub struct PipeTx(Mutex<std::fs::File>);
pub struct PipeRx(std::fs::File);

impl Tx for PipeTx {
    fn send(&self, f: &[&[u8]]) -> io::Result<()> {
        let bytes = encode(f)?;
        let mut w = self.0.lock().unwrap();
        w.write_all(&bytes)?;
        w.flush()
    }
}

impl Rx for PipeRx {
    fn recv(&mut self) -> io::Result<Option<Frame>> {
        read_frame(&mut self.0)
    }
}

/// `\\.\pipe\BoylerUtilities-admin-<id>-req` / `-rep`.
pub fn pipe_name(id: &str, dir: &str) -> String {
    format!(r"\\.\pipe\BoylerUtilities-admin-{id}-{dir}")
}

/// A pipe id: 32 lowercase hex characters.
pub fn check_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A fresh random pipe id (128 bits from Windows' random number generator).
pub fn new_id() -> String {
    let mut b = [0u8; 16];
    // SAFETY: the buffer's length is passed with it.
    let ok = unsafe { windows::Win32::Security::Cryptography::BCryptGenRandom(None, &mut b, windows::Win32::Security::Cryptography::BCRYPT_USE_SYSTEM_PREFERRED_RNG) }.is_ok();
    if !ok {
        // never reached in practice; still unpredictable enough to avoid a clash (the pipe is also first-instance only)
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        b = (t ^ ((std::process::id() as u128) << 64)).to_le_bytes();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// SYSTEM, Administrators and the owner (the user who runs the app); nothing inherited.
const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)";

/// The app's end: both pipes made, not yet connected.
pub struct Server {
    req: windows::Win32::Foundation::HANDLE,
    rep: windows::Win32::Foundation::HANDLE,
    id: String,
}

// SAFETY: the handles are plain kernel handles, used from one owner at a time.
unsafe impl Send for Server {}

impl Drop for Server {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;
        // SAFETY: owned handles, closed once (the connected ones were handed out as Files and zeroed).
        unsafe {
            if !self.req.is_invalid() {
                let _ = CloseHandle(self.req);
            }
            if !self.rep.is_invalid() {
                let _ = CloseHandle(self.rep);
            }
        }
    }
}

impl Server {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Make both pipes with a new random id.
    pub fn create() -> io::Result<Server> {
        use windows::core::HSTRING;
        use windows::Win32::Foundation::{LocalFree, HLOCAL};
        use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
        use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
        use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND};
        use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT};
        let id = new_id();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: freed below with LocalFree after its last use.
        unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(&HSTRING::from(PIPE_SDDL), SDDL_REVISION_1, &mut sd, None) }.map_err(io::Error::other)?;
        let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd.0, bInheritHandle: false.into() };
        let make = |dir: &str, access| {
            // SAFETY: sa and its descriptor live across the call.
            let h = unsafe {
                CreateNamedPipeW(
                    &HSTRING::from(pipe_name(&id, dir)),
                    access | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
                    65536,
                    65536,
                    0,
                    Some(&sa),
                )
            };
            if h.is_invalid() {
                Err(io::Error::last_os_error())
            } else {
                Ok(h)
            }
        };
        let req = make("req", PIPE_ACCESS_OUTBOUND);
        let rep = make("rep", PIPE_ACCESS_INBOUND);
        // SAFETY: made above.
        unsafe { LocalFree(Some(HLOCAL(sd.0))) };
        let mut s = Server { req: Default::default(), rep: Default::default(), id };
        s.req = req?;
        s.rep = rep?;
        Ok(s)
    }

    /// Wait until the copy (process `child`) opened both pipes, at most `timeout`, giving up early when `child` ends or
    /// `stop()` says so. A client that is not `child` is refused.
    pub fn accept(
        mut self,
        child: windows::Win32::Foundation::HANDLE,
        timeout: std::time::Duration,
        stop: &dyn Fn() -> bool,
    ) -> io::Result<(PipeTx, PipeRx)> {
        use std::os::windows::io::FromRawHandle;
        use windows::Win32::System::Threading::{GetProcessId, WaitForSingleObject};
        let child_pid = unsafe { GetProcessId(child) };
        let mut got = Vec::new();
        for (h, dir) in [(self.req, "req"), (self.rep, "rep")] {
            let raw = h.0 as usize;
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                use windows::Win32::System::Pipes::ConnectNamedPipe;
                let r = unsafe { ConnectNamedPipe(windows::Win32::Foundation::HANDLE(raw as *mut _), None) };
                // ERROR_PIPE_CONNECTED (535) = it connected between the create and this call: fine
                let ok = r.is_ok() || r.as_ref().err().map(|e| e.code() == windows::Win32::Foundation::ERROR_PIPE_CONNECTED.to_hresult()).unwrap_or(false);
                let _ = tx.send(ok);
            });
            let t0 = std::time::Instant::now();
            let ok = loop {
                if let Ok(ok) = rx.recv_timeout(std::time::Duration::from_millis(100)) {
                    break ok;
                }
                let gone = unsafe { WaitForSingleObject(child, 0) } == windows::Win32::Foundation::WAIT_OBJECT_0;
                if gone || t0.elapsed() > timeout || stop() {
                    // free the waiting ConnectNamedPipe by connecting to it ourselves, then drop that end
                    let _ = std::fs::OpenOptions::new().read(dir == "req").write(dir == "rep").open(pipe_name(&self.id, dir));
                    let _ = rx.recv_timeout(std::time::Duration::from_secs(2));
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "The admin helper did not connect"));
                }
            };
            if !ok {
                return Err(io::Error::new(io::ErrorKind::ConnectionRefused, "The admin helper could not connect"));
            }
            if client_pid(h) != Some(child_pid) {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Another program connected to the admin helper's pipe"));
            }
            got.push(h);
        }
        // the Files own the handles now
        self.req = Default::default();
        self.rep = Default::default();
        // SAFETY: connected pipe handles, owned from here by the Files.
        unsafe { Ok((PipeTx(Mutex::new(std::fs::File::from_raw_handle(got[0].0))), PipeRx(std::fs::File::from_raw_handle(got[1].0)))) }
    }
}

fn client_pid(h: windows::Win32::Foundation::HANDLE) -> Option<u32> {
    use windows::Win32::System::Pipes::GetNamedPipeClientProcessId;
    let mut pid = 0u32;
    unsafe { GetNamedPipeClientProcessId(h, &mut pid) }.ok().map(|_| pid)
}

/// The copy's end: open both pipes of `id` (identification only), check they were made by the app's own exe, and read
/// the clicking user's SID from that process. Returns (reader of ops, writer of answers, user SID).
pub fn connect(id: &str) -> io::Result<(PipeRx, PipeTx, String)> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Storage::FileSystem::{SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT};
    if !check_id(id) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad pipe id"));
    }
    let flags = SECURITY_SQOS_PRESENT.0 | SECURITY_IDENTIFICATION.0;
    let req = std::fs::OpenOptions::new().read(true).security_qos_flags(flags).open(pipe_name(id, "req"))?;
    let rep = std::fs::OpenOptions::new().write(true).security_qos_flags(flags).open(pipe_name(id, "rep"))?;
    let mut sid = None;
    for f in [&req, &rep] {
        let h = windows::Win32::Foundation::HANDLE(f.as_raw_handle());
        let pid = server_pid(h).ok_or_else(|| io::Error::other("no pipe server"))?;
        if !same_exe(pid) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the pipe was not made by Boyler Utilities"));
        }
        let s = process_user_sid(pid).ok_or_else(|| io::Error::other("could not read the user"))?;
        if sid.get_or_insert_with(|| s.clone()) != &s {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the two pipes belong to different users"));
        }
    }
    Ok((PipeRx(req), PipeTx(Mutex::new(rep)), sid.unwrap_or_default()))
}

fn server_pid(h: windows::Win32::Foundation::HANDLE) -> Option<u32> {
    use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
    let mut pid = 0u32;
    unsafe { GetNamedPipeServerProcessId(h, &mut pid) }.ok().map(|_| pid)
}

/// A process's image path (QueryFullProcessImageNameW).
pub fn process_image(pid: u32) -> Option<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    // SAFETY: the handle is closed below; the buffer's length goes with it.
    unsafe {
        let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        let r = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut n);
        let _ = CloseHandle(p);
        r.ok()?;
        Some(String::from_utf16_lossy(&buf[..n as usize]))
    }
}

/// Is process `pid` running the same exe as this process?
fn same_exe(pid: u32) -> bool {
    let (Some(theirs), Some(mine)) = (process_image(pid), process_image(std::process::id())) else { return false };
    theirs.eq_ignore_ascii_case(&mine)
}

/// The string SID ("S-1-5-21-…") of the user process `pid` runs as.
pub fn process_user_sid(pid: u32) -> Option<String> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
    // SAFETY: handles closed below; the token buffer is sized by the first call; the SID string is freed with LocalFree.
    unsafe {
        let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut tok = HANDLE::default();
        let ok = OpenProcessToken(p, TOKEN_QUERY, &mut tok).is_ok();
        let _ = CloseHandle(p);
        if !ok {
            return None;
        }
        let mut len = 0u32;
        let _ = GetTokenInformation(tok, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u8; len as usize];
        let ok = GetTokenInformation(tok, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len).is_ok();
        let _ = CloseHandle(tok);
        if !ok {
            return None;
        }
        let tu = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = windows::core::PWSTR::null();
        ConvertSidToStringSidW(tu.User.Sid, &mut s).ok()?;
        let out = s.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(s.0.cast())));
        out.filter(|x| check_sid(x))
    }
}

/// `S-1-5-21-…`: digits and dashes after "S-1-".
pub fn check_sid(s: &str) -> bool {
    s.len() < 200 && s.starts_with("S-1-") && s[4..].split('-').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}
