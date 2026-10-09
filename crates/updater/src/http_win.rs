//! The real [`Http`]: Windows' WinHTTP through the `windows` crate. Windows does the TLS (system certificate store), the proxy
//! settings and the redirects (up to 10; WinHTTP's default policy never follows an https link down to http). One session per
//! request, closed when the request ends - nothing stays open or running between a check and the next.

use crate::error::{Result, UpdateError};
use crate::http::{parse_url, Http, Request, Response};
use std::ffi::c_void;
use std::io::Write;
use windows::core::PCWSTR;
use windows::Win32::Networking::WinHttp::*;

pub struct WinHttp {
    user_agent: String,
}

impl WinHttp {
    pub fn new(user_agent: &str) -> Self {
        WinHttp { user_agent: user_agent.to_string() }
    }
}

/// Closes a WinHTTP handle when it goes out of scope.
struct Handle(*mut c_void);

impl Handle {
    fn open(h: *mut c_void, what: &str) -> Result<Handle> {
        if h.is_null() {
            Err(last_error(what))
        } else {
            Ok(Handle(h))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle came from WinHttpOpen/Connect/OpenRequest and is closed exactly once, here.
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn net_err(context: &str, e: &windows::core::Error) -> UpdateError {
    UpdateError::Network { context: context.to_string(), detail: format!("{} (0x{:08X})", e.message().trim(), e.code().0 as u32) }
}

fn last_error(context: &str) -> UpdateError {
    net_err(context, &windows::core::Error::from_thread())
}

fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "[::1]"
}

impl WinHttp {
    /// Order 066: GET of a part of a file (`bytes=<from>-<to>`, or `bytes=-<last n>` for the tail): the answer is 206 (a 2xx,
    /// so the body goes to `sink`). Used to read one picture out of a big .zip without downloading all of it.
    pub fn get_range(&self, url: &str, range: &str, sink: &mut dyn Write, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Response> {
        self.get_with(&Request { url, accept: "*/*" }, &format!("Range: bytes={range}\r\n"), sink, progress)
    }
}

impl Http for WinHttp {
    fn get(&self, req: &Request, sink: &mut dyn Write, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Response> {
        self.get_with(req, "", sink, progress)
    }
}

impl WinHttp {
    fn get_with(&self, req: &Request, extra_headers: &str, sink: &mut dyn Write, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Response> {
        let url = parse_url(req.url)?;
        let agent = wide(&self.user_agent);
        let host = wide(&url.host);
        let path = wide(&url.path);
        let headers = wide(&format!("Accept: {}\r\nX-GitHub-Api-Version: 2022-11-28\r\n{extra_headers}", req.accept));
        let access = if is_loopback(&url.host) { WINHTTP_ACCESS_TYPE_NO_PROXY } else { WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY };

        // SAFETY: every pointer passed below points into the Vec<u16>s above (alive for this whole function, NUL-terminated)
        // or into locals; handles are closed by `Handle::drop`.
        unsafe {
            let session = Handle::open(WinHttpOpen(PCWSTR(agent.as_ptr()), access, PCWSTR::null(), PCWSTR::null(), 0), "opening the connection")?;
            WinHttpSetTimeouts(session.0, 15_000, 15_000, 30_000, 30_000).map_err(|e| net_err("setting timeouts", &e))?;
            let connect = Handle::open(WinHttpConnect(session.0, PCWSTR(host.as_ptr()), url.port, 0), "connecting")?;
            let flags = WINHTTP_OPEN_REQUEST_FLAGS(if url.https { WINHTTP_FLAG_SECURE.0 } else { 0 });
            let request = Handle::open(
                WinHttpOpenRequest(connect.0, windows::core::w!("GET"), PCWSTR(path.as_ptr()), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), flags),
                "creating the request",
            )?;
            WinHttpSendRequest(request.0, Some(&headers[..headers.len() - 1]), None, 0, 0, 0).map_err(|e| net_err("sending the request", &e))?;
            WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(|e| net_err("waiting for the answer", &e))?;

            let mut status: u32 = 0;
            let mut len = std::mem::size_of::<u32>() as u32;
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some(&mut status as *mut u32 as *mut c_void),
                &mut len,
                std::ptr::null_mut(),
            )
            .map_err(|e| net_err("reading the status", &e))?;

            let mut clen: u32 = 0;
            let mut len = std::mem::size_of::<u32>() as u32;
            let content_length = WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some(&mut clen as *mut u32 as *mut c_void),
                &mut len,
                std::ptr::null_mut(),
            )
            .ok()
            .map(|_| clen as u64);

            let response = Response { status: status as u16, content_length };
            if !(200..300).contains(&response.status) {
                return Ok(response);
            }
            let mut buf = vec![0u8; 64 * 1024];
            let mut done = 0u64;
            loop {
                let mut read: u32 = 0;
                WinHttpReadData(request.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut read).map_err(|e| net_err("downloading", &e))?;
                if read == 0 {
                    break;
                }
                sink.write_all(&buf[..read as usize]).map_err(|e| UpdateError::io("writing the download", &e))?;
                done += read as u64;
                progress(done, content_length);
            }
            Ok(response)
        }
    }
}
