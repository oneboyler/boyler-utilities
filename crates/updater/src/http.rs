//! The one place the updater talks to the network: a tiny GET behind a trait. [`WinHttp`] is the real one (Windows' own
//! WinHTTP: system certificates, system proxy, nothing to bundle, nothing running when idle); [`FakeHttp`] answers from memory
//! for the logic tests. Nothing here runs unless `check()` / `update()` called it - there is no timer and no thread.

use crate::error::{Result, UpdateError};
use std::collections::HashMap;
use std::io::Write;
use std::sync::Mutex;

pub struct Request<'a> {
    pub url: &'a str,
    pub accept: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    /// What the server announced (None when it didn't).
    pub content_length: Option<u64>,
}

pub trait Http: Send + Sync {
    /// GET `req.url`, following redirects. The body is written to `sink` ONLY when the status is 2xx (an error page never
    /// lands in a file). `progress(bytes_so_far, content_length)` is called after every chunk. Network trouble is
    /// `Err(Network)`; any HTTP status is `Ok(Response)` - the caller decides what a 404 means.
    fn get(&self, req: &Request, sink: &mut dyn Write, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Response>;
}

/// A `Write` that refuses more than `max` bytes (for the small API answers).
pub(crate) struct LimitedVec {
    pub buf: Vec<u8>,
    pub max: usize,
}

impl Write for LimitedVec {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.buf.len() + data.len() > self.max {
            return Err(std::io::Error::other("the answer is bigger than expected"));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// `scheme://host[:port]/path?query` taken apart (http and https only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUrl {
    pub https: bool,
    pub host: String,
    pub port: u16,
    /// Path and query, always starting with `/`.
    pub path: String,
}

pub fn parse_url(url: &str) -> Result<ParsedUrl> {
    let bad = || UpdateError::BadResponse(format!("not a usable link: {url:?}"));
    let (https, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(bad());
    };
    let rest = rest.split('#').next().unwrap_or("");
    let (authority, path) = match rest.find(['/', '?']) {
        Some(i) if rest.as_bytes()[i] == b'/' => (&rest[..i], rest[i..].to_string()),
        Some(i) => (&rest[..i], format!("/{}", &rest[i..])),
        None => (rest, "/".to_string()),
    };
    if authority.contains('@') || authority.is_empty() || url.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Err(bad());
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(']') || h.starts_with('[') => {
            (h.to_string(), p.parse::<u16>().map_err(|_| bad())?)
        }
        _ => (authority.to_string(), if https { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err(bad());
    }
    Ok(ParsedUrl { https, host, port, path })
}

impl<T: Http + ?Sized> Http for std::sync::Arc<T> {
    fn get(&self, req: &Request, sink: &mut dyn Write, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Response> {
        (**self).get(req, sink, progress)
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// the fake

/// What [`FakeHttp`] answers for one link.
#[derive(Debug, Clone)]
pub enum FakeReply {
    /// Status + body, announced length = body length.
    Body { status: u16, body: Vec<u8> },
    /// The connection dies after `sent` bytes of a `body` that was announced in full (a cut download).
    Cut { body: Vec<u8>, sent: usize },
    /// The announced length lies: server says `announced` but sends `body` (a wrong size).
    WrongLength { body: Vec<u8>, announced: u64 },
    /// No connection at all.
    Offline(String),
}

/// In-memory server for tests: link -> reply, plus a log of every request (so a test can prove "no request was made").
#[derive(Default)]
pub struct FakeHttp {
    routes: Mutex<HashMap<String, FakeReply>>,
    log: Mutex<Vec<String>>,
    chunk: usize,
}

impl FakeHttp {
    pub fn new() -> Self {
        FakeHttp { routes: Mutex::default(), log: Mutex::default(), chunk: 16 * 1024 }
    }
    /// Smaller chunks = more progress events.
    pub fn with_chunk(mut self, chunk: usize) -> Self {
        self.chunk = chunk.max(1);
        self
    }
    pub fn route(&self, url: &str, reply: FakeReply) {
        self.routes.lock().unwrap().insert(url.to_string(), reply);
    }
    pub fn ok(&self, url: &str, body: impl Into<Vec<u8>>) {
        self.route(url, FakeReply::Body { status: 200, body: body.into() });
    }
    pub fn status(&self, url: &str, status: u16) {
        self.route(url, FakeReply::Body { status, body: b"<html>error page</html>".to_vec() });
    }
    /// Every link asked for so far, in order.
    pub fn requests(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl Http for FakeHttp {
    fn get(&self, req: &Request, sink: &mut dyn Write, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Response> {
        self.log.lock().unwrap().push(req.url.to_string());
        let reply = self.routes.lock().unwrap().get(req.url).cloned();
        let chunk = if self.chunk == 0 { 16 * 1024 } else { self.chunk };
        let io = |e: std::io::Error| UpdateError::Io { context: "writing the answer".into(), detail: e.to_string() };
        match reply {
            None => Ok(Response { status: 404, content_length: Some(0) }),
            Some(FakeReply::Offline(why)) => Err(UpdateError::Network { context: "connecting".into(), detail: why }),
            Some(FakeReply::Body { status, body }) => {
                let len = body.len() as u64;
                if !(200..300).contains(&status) {
                    return Ok(Response { status, content_length: Some(len) });
                }
                let mut done = 0u64;
                for c in body.chunks(chunk) {
                    sink.write_all(c).map_err(io)?;
                    done += c.len() as u64;
                    progress(done, Some(len));
                }
                Ok(Response { status, content_length: Some(len) })
            }
            Some(FakeReply::Cut { body, sent }) => {
                let len = body.len() as u64;
                let mut done = 0u64;
                for c in body[..sent.min(body.len())].chunks(chunk) {
                    sink.write_all(c).map_err(io)?;
                    done += c.len() as u64;
                    progress(done, Some(len));
                }
                Err(UpdateError::Network { context: "downloading".into(), detail: "the connection was closed early".into() })
            }
            Some(FakeReply::WrongLength { body, announced }) => {
                let mut done = 0u64;
                for c in body.chunks(chunk) {
                    sink.write_all(c).map_err(io)?;
                    done += c.len() as u64;
                    progress(done, Some(announced));
                }
                Ok(Response { status: 200, content_length: Some(announced) })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_taken_apart() {
        let u = parse_url("https://api.github.com/repos/o/r/releases/latest").unwrap();
        assert_eq!((u.https, u.host.as_str(), u.port, u.path.as_str()), (true, "api.github.com", 443, "/repos/o/r/releases/latest"));
        let u = parse_url("http://127.0.0.1:8080/x?y=1#frag").unwrap();
        assert_eq!((u.https, u.host.as_str(), u.port, u.path.as_str()), (false, "127.0.0.1", 8080, "/x?y=1"));
        let u = parse_url("https://host").unwrap();
        assert_eq!((u.port, u.path.as_str()), (443, "/"));
        let u = parse_url("https://host?a=b").unwrap();
        assert_eq!(u.path, "/?a=b");
    }

    #[test]
    fn bad_links_are_refused() {
        for bad in ["", "ftp://x/y", "file:///c:/x", "https://", "https://user@host/x", "https:///x", "https://host:99999/x", "https://ho st/x", "//host/x"] {
            assert!(parse_url(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn fake_streams_chunks_and_logs() {
        let f = FakeHttp::new().with_chunk(4);
        f.ok("u", vec![1u8; 10]);
        let mut out = Vec::new();
        let mut seen = Vec::new();
        let r = f.get(&Request { url: "u", accept: "*/*" }, &mut out, &mut |d, t| seen.push((d, t))).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(out.len(), 10);
        assert_eq!(seen, vec![(4, Some(10)), (8, Some(10)), (10, Some(10))]);
        assert_eq!(f.requests(), vec!["u".to_string()]);
    }

    #[test]
    fn error_status_writes_no_body() {
        let f = FakeHttp::new();
        f.status("u", 500);
        let mut out = Vec::new();
        let r = f.get(&Request { url: "u", accept: "*/*" }, &mut out, &mut |_, _| {}).unwrap();
        assert_eq!(r.status, 500);
        assert!(out.is_empty());
    }
}
