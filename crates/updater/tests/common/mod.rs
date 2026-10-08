#![allow(dead_code)]
//! Shared test helpers: a scratch folder (under the board's scratch\N, removed afterwards), a small local HTTP server that plays
//! GitHub, and fake app bytes.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const SCRATCH_ROOT: &str = r"C:\BoylerUtilities-scratch\N";

/// A folder that exists for one test and is removed when the test ends.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Scratch {
        let root = std::env::var("BU_SCRATCH").unwrap_or_else(|_| SCRATCH_ROOT.to_string());
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let p = Path::new(&root).join(format!("{name}-{}-{n}-{nanos:x}", std::process::id()));
        std::fs::create_dir_all(&p).expect("scratch folder");
        Scratch(p)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
    pub fn join(&self, p: &str) -> PathBuf {
        self.0.join(p)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        for _ in 0..20 {
            if std::fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(250));
        }
    }
}

/// Bytes that look like a Windows program to the updater (`MZ` + a pattern), `len` long in total.
pub fn fake_exe_bytes(tag: &str, len: usize) -> Vec<u8> {
    let mut v = b"MZ".to_vec();
    let mut i = 0u32;
    while v.len() < len {
        v.push((i.wrapping_mul(31).wrapping_add(7) % 251) as u8);
        i += 1;
    }
    v.extend_from_slice(format!("\nBU-TAG:{tag}\n").as_bytes());
    v
}

pub fn sha256_of(data: &[u8]) -> String {
    bu_updater::sha256::sha256_hex(data)
}

pub fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if cond() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    cond()
}

// ---------------------------------------------------------------------------------------------------------------------
// local server that plays GitHub

#[derive(Clone)]
pub enum Route {
    Body { status: u16, body: Vec<u8> },
    Redirect(String),
    /// Announces the full length, sends only `sent` bytes, then closes the connection.
    Cut { body: Vec<u8>, sent: usize },
}

pub struct TestServer {
    pub addr: String,
    routes: Arc<Mutex<HashMap<String, Route>>>,
    /// (path, raw request headers) of every request, in order.
    hits: Arc<Mutex<Vec<(String, String)>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    pub fn start() -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap().to_string();
        let routes: Arc<Mutex<HashMap<String, Route>>> = Arc::default();
        let hits: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (r, h, s) = (routes.clone(), hits.clone(), stop.clone());
        let thread = thread::spawn(move || {
            for conn in listener.incoming() {
                if s.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(conn) = conn else { continue };
                let (r, h) = (r.clone(), h.clone());
                thread::spawn(move || {
                    let _ = serve(conn, &r, &h);
                });
            }
        });
        TestServer { addr, routes, hits, stop, thread: Some(thread) }
    }

    pub fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub fn route(&self, path: &str, r: Route) {
        self.routes.lock().unwrap().insert(path.to_string(), r);
    }

    pub fn body(&self, path: &str, body: impl Into<Vec<u8>>) {
        self.route(path, Route::Body { status: 200, body: body.into() });
    }

    pub fn hits(&self) -> Vec<(String, String)> {
        self.hits.lock().unwrap().clone()
    }

    pub fn paths(&self) -> Vec<String> {
        self.hits().into_iter().map(|(p, _)| p).collect()
    }

    /// The GitHub "latest release" answer for `owner/name`.
    pub fn release(&self, repo: &str, tag: &str, asset_name: &str, asset_path: &str, size: u64, digest: Option<&str>) {
        let digest = match digest {
            Some(d) => format!("\"{d}\""),
            None => "null".to_string(),
        };
        let json = format!(
            r#"{{"tag_name":"{tag}","name":"Version {tag}","body":"notes","html_url":"{}","draft":false,"prerelease":false,
            "assets":[{{"name":"{asset_name}","size":{size},"browser_download_url":"{}","digest":{digest}}}]}}"#,
            self.url("/page"),
            self.url(asset_path)
        );
        self.body(&format!("/repos/{repo}/releases/latest"), json);
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(&self.addr); // wakes the accept loop
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn serve(mut conn: TcpStream, routes: &Mutex<HashMap<String, Route>>, hits: &Mutex<Vec<(String, String)>>) -> std::io::Result<()> {
    conn.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 2048];
    while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = conn.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        raw.extend_from_slice(&buf[..n]);
    }
    let text = String::from_utf8_lossy(&raw).into_owned();
    let first = text.lines().next().unwrap_or("");
    let path = first.split_whitespace().nth(1).unwrap_or("/").to_string();
    hits.lock().unwrap().push((path.clone(), text.clone()));
    let route = routes.lock().unwrap().get(&path).cloned();
    match route {
        None => {
            conn.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot found")?;
        }
        Some(Route::Body { status, body }) => {
            write!(conn, "HTTP/1.1 {status} X\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())?;
            conn.write_all(&body)?;
        }
        Some(Route::Redirect(to)) => {
            write!(conn, "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
        }
        Some(Route::Cut { body, sent }) => {
            write!(conn, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())?;
            conn.write_all(&body[..sent.min(body.len())])?;
        }
    }
    conn.flush()?;
    let _ = conn.shutdown(Shutdown::Both);
    Ok(())
}
