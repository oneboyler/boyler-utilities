//! The real speed test transport: Cloudflare's speed test endpoints (the ones speed.cloudflare.com itself uses),
//! over WinHTTP (Windows' own HTTP stack: system proxy, Schannel TLS, no extra library).
//!   download  GET  https://speed.cloudflare.com/__down?bytes=N
//!   upload    POST https://speed.cloudflare.com/__up   (body of N bytes)
//!   latency   ICMP echo to the same server (TCP handshake when ICMP is blocked) - see `latency`
//!   server    the `colo` response header (the Cloudflare location that answered) -> city via `colo_city`
//! Which service and why: reports/order_008.md.

use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts, WinHttpWriteData, INTERNET_DEFAULT_HTTPS_PORT,
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_REFRESH, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_CUSTOM,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

use super::util::{from_wide, wide};
use crate::error::{NetError, Result};
use crate::speedtest::{ServerInfo, SpeedTransport};

const HOST: &str = "speed.cloudflare.com";
const CHUNK: usize = 64 * 1024;
const MODE_ICMP: u8 = 1;
const MODE_TCP: u8 = 2;

struct Handle(*mut core::ffi::c_void);
// WinHTTP handles may be used from any thread (the session is shared by the streams; each request is one thread's).
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn err(call: &'static str) -> impl Fn(windows::core::Error) -> NetError {
    move |e| {
        let code = e.code().0 as u32 & 0xFFFF;
        match code {
            12002 => NetError::Timeout,                                     // ERROR_WINHTTP_TIMEOUT
            12007 => NetError::Resolve(HOST.into()),                        // ERROR_WINHTTP_NAME_NOT_RESOLVED
            12029 | 12030 => NetError::Unreachable(format!("{HOST}: {call}")), // cannot connect / connection aborted
            12017 => NetError::Cancelled,                                   // ERROR_WINHTTP_OPERATION_CANCELLED
            _ => NetError::Os { call, code },
        }
    }
}

/// One open request (its connection handle lives as long as it).
struct Req {
    req: Handle,
    _conn: Handle,
}

impl Req {
    fn status(&self) -> u32 {
        let mut code = 0u32;
        let mut len = 4u32;
        let mut idx = 0u32;
        let ok = unsafe {
            WinHttpQueryHeaders(
                self.req.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some(&mut code as *mut u32 as *mut _),
                &mut len,
                &mut idx,
            )
        };
        if ok.is_ok() {
            code
        } else {
            0
        }
    }

    fn header(&self, name: &str) -> Option<String> {
        let n = wide(name);
        let mut buf = vec![0u16; 512];
        let mut len = (buf.len() * 2) as u32;
        let mut idx = 0u32;
        unsafe {
            WinHttpQueryHeaders(
                self.req.0,
                WINHTTP_QUERY_CUSTOM,
                PCWSTR(n.as_ptr()),
                Some(buf.as_mut_ptr() as *mut _),
                &mut len,
                &mut idx,
            )
            .ok()?;
        }
        buf.truncate(len as usize / 2);
        Some(from_wide(&buf))
    }
}

/// Cloudflare speed test over WinHTTP.
pub struct CloudflareSpeed {
    session: Handle,
    /// speed.cloudflare.com, looked up once per test (for the latency handshakes).
    addr: std::sync::Mutex<Option<std::net::SocketAddr>>,
    /// Latency method of this test: 0 = not decided yet, 1 = ICMP, 2 = TCP handshake.
    latency_mode: AtomicU8,
}

impl CloudflareSpeed {
    pub fn new() -> Result<CloudflareSpeed> {
        let s = unsafe {
            WinHttpOpen(w!("BoylerUtilities/0.1"), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0)
        };
        if s.is_null() {
            return Err(NetError::Os { call: "WinHttpOpen", code: unsafe { windows::Win32::Foundation::GetLastError().0 } });
        }
        let session = Handle(s);
        unsafe { WinHttpSetTimeouts(session.0, 5_000, 5_000, 15_000, 15_000) }.map_err(err("WinHttpSetTimeouts"))?;
        Ok(CloudflareSpeed { session, addr: std::sync::Mutex::new(None), latency_mode: AtomicU8::new(0) })
    }

    fn open(&self, verb: &str, path: &str) -> Result<Req> {
        let host = wide(HOST);
        let conn = Handle(unsafe { WinHttpConnect(self.session.0, PCWSTR(host.as_ptr()), INTERNET_DEFAULT_HTTPS_PORT, 0) });
        if conn.0.is_null() {
            return Err(NetError::Os { call: "WinHttpConnect", code: unsafe { windows::Win32::Foundation::GetLastError().0 } });
        }
        let v = wide(verb);
        let p = wide(path);
        let req = Handle(unsafe {
            WinHttpOpenRequest(
                conn.0,
                PCWSTR(v.as_ptr()),
                PCWSTR(p.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE | WINHTTP_FLAG_REFRESH,
            )
        });
        if req.0.is_null() {
            return Err(NetError::Os { call: "WinHttpOpenRequest", code: unsafe { windows::Win32::Foundation::GetLastError().0 } });
        }
        Ok(Req { req, _conn: conn })
    }

    /// The test server's address (IPv4 first), looked up once.
    pub fn server_addr(&self) -> Result<std::net::SocketAddr> {
        let mut a = self.addr.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(addr) = *a {
            return Ok(addr);
        }
        use std::net::ToSocketAddrs;
        let mut all: Vec<std::net::SocketAddr> =
            (HOST, 443u16).to_socket_addrs().map_err(|_| NetError::Resolve(HOST.into()))?.collect();
        all.sort_by_key(|s| !s.is_ipv4());
        let addr = *all.first().ok_or_else(|| NetError::Resolve(HOST.into()))?;
        *a = Some(addr);
        Ok(addr)
    }

    /// The HTTP way Cloudflare's own engine measures latency: a `bytes=0` request, minus `cfRequestDuration` from
    /// Server-Timing when the server sends it. Kept for comparison only (examples: `--latency`); it includes the
    /// server's own time today.
    pub fn http_latency(&self) -> Result<Duration> {
        let t0 = Instant::now();
        let r = self.get("/__down?bytes=0")?;
        let total = t0.elapsed();
        let server = r.header("server-timing").and_then(|h| server_timing_ms(&h)).unwrap_or(0.0);
        Ok(total.saturating_sub(Duration::from_secs_f64((server / 1000.0).max(0.0))))
    }

    /// GET with an empty body; returns the open request after the response headers arrived.
    fn get(&self, path: &str) -> Result<Req> {
        let r = self.open("GET", path)?;
        unsafe {
            WinHttpSendRequest(r.req.0, None, None, 0, 0, 0).map_err(err("WinHttpSendRequest"))?;
            WinHttpReceiveResponse(r.req.0, std::ptr::null_mut()).map_err(err("WinHttpReceiveResponse"))?;
        }
        match r.status() {
            200 => Ok(r),
            s => Err(NetError::Http(s)),
        }
    }
}

/// The server's own time from `Server-Timing`: only the `cfRequestDuration` entry (what Cloudflare's speed test
/// engine subtracts). Other entries (cfSpeedEdge / cfSpeedWorker / cfL4) are not processing time of this request
/// alone, so they are not subtracted. "cfRequestDuration;dur=12.345" -> 12.345 ms.
pub fn server_timing_ms(h: &str) -> Option<f64> {
    h.split(',').find_map(|entry| {
        let mut parts = entry.split(';').map(str::trim);
        if parts.next()? != "cfRequestDuration" {
            return None;
        }
        parts.find_map(|p| p.strip_prefix("dur=")).and_then(|v| v.parse().ok())
    })
}

/// Cloudflare location codes (IATA) in Europe -> city, for "Testing · nearest server: <city>".
/// Unknown codes show as the code itself. Source: Cloudflare's IATA codes = airport codes of the city.
pub fn colo_city(code: &str) -> Option<&'static str> {
    Some(match code {
        "LJU" => "Ljubljana",
        "ZAG" => "Zagreb",
        "VIE" => "Vienna",
        "BUD" => "Budapest",
        "BEG" => "Belgrade",
        "SOF" => "Sofia",
        "OTP" => "Bucharest",
        "MXP" | "MIL" => "Milan",
        "FCO" => "Rome",
        "MUC" => "Munich",
        "FRA" => "Frankfurt",
        "DUS" => "Düsseldorf",
        "HAM" => "Hamburg",
        "TXL" | "BER" => "Berlin",
        "PRG" => "Prague",
        "WAW" => "Warsaw",
        "ZRH" => "Zurich",
        "GVA" => "Geneva",
        "AMS" => "Amsterdam",
        "BRU" => "Brussels",
        "CDG" | "PAR" => "Paris",
        "MRS" => "Marseille",
        "LHR" | "LON" => "London",
        "MAN" => "Manchester",
        "DUB" => "Dublin",
        "MAD" => "Madrid",
        "BCN" => "Barcelona",
        "LIS" => "Lisbon",
        "ARN" | "STO" => "Stockholm",
        "CPH" => "Copenhagen",
        "OSL" => "Oslo",
        "HEL" => "Helsinki",
        "ATH" => "Athens",
        "SKG" => "Thessaloniki",
        "IST" => "Istanbul",
        "SKP" => "Skopje",
        "SJJ" => "Sarajevo",
        "TGD" => "Podgorica",
        "KBP" => "Kyiv",
        "KIV" => "Chisinau",
        _ => return None,
    })
}

impl SpeedTransport for CloudflareSpeed {
    fn server(&self) -> Result<ServerInfo> {
        // `colo` = the Cloudflare location that answered. (The `city` header is the USER's city, not the server's.)
        let r = self.get("/__down?bytes=0")?;
        let code = r.header("colo").or_else(|| r.header("cf-meta-colo")).unwrap_or_default().to_ascii_uppercase();
        Ok(ServerInfo { city: colo_city(&code).unwrap_or(&code).to_string(), code, provider: "Cloudflare".into() })
    }

    fn download(&self, bytes: u64, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<()> {
        let r = self.get(&format!("/__down?bytes={bytes}"))?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            let mut n = 0u32;
            unsafe { WinHttpReadData(r.req.0, buf.as_mut_ptr() as *mut _, CHUNK as u32, &mut n) }
                .map_err(err("WinHttpReadData"))?;
            if n == 0 {
                return Ok(());
            }
            if !on_bytes(n as u64) {
                return Ok(()); // dropping `r` closes the request = the download is aborted
            }
        }
    }

    fn upload(&self, bytes: u64, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<()> {
        let total = bytes.min(u32::MAX as u64) as u32;
        let r = self.open("POST", "/__up")?;
        let headers = wide("Content-Type: application/octet-stream\r\n");
        unsafe {
            WinHttpSendRequest(r.req.0, Some(&headers[..headers.len() - 1]), None, 0, total, 0)
                .map_err(err("WinHttpSendRequest"))?;
        }
        // Not compressible on purpose (xorshift bytes), so no middle box can shrink it.
        let mut data = vec![0u8; CHUNK];
        let mut x: u32 = 0x9E37_79B9;
        for b in data.iter_mut() {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *b = x as u8;
        }
        let mut sent: u64 = 0;
        while sent < total as u64 {
            let len = (total as u64 - sent).min(CHUNK as u64) as u32;
            let mut n = 0u32;
            unsafe { WinHttpWriteData(r.req.0, Some(data.as_ptr() as *const _), len, &mut n) }
                .map_err(err("WinHttpWriteData"))?;
            sent += n as u64;
            if !on_bytes(n as u64) {
                return Ok(()); // phase over: drop the request mid-body
            }
        }
        unsafe { WinHttpReceiveResponse(r.req.0, std::ptr::null_mut()) }.map_err(err("WinHttpReceiveResponse"))?;
        match r.status() {
            200 => Ok(()),
            s => Err(NetError::Http(s)),
        }
    }

    /// Ping to the test server: ICMP echo (same method as the ping pill), or - when ICMP gets no answer on the
    /// first try of a test - a TCP handshake (SYN -> SYN/ACK = one round trip) for the rest of that test.
    /// Not HTTP: two latency-only runs on 2026-10-08 against the same Cloudflare edge, 20 samples each (ping / jitter,
    /// ms) - run 1: ICMP 10.0 / 0.74, TCP handshake 7.7 / 3.43, HTTP `bytes=0` 28.4 / 20.18; run 2: this method (ICMP)
    /// 9.0 / 1.78, ICMP 10.0 / 2.32, TCP 9.8 / 2.11, HTTP 30.3 / 37.68. The HTTP way carries the server's own time and
    /// its Server-Timing has no `cfRequestDuration` to take out.
    fn latency(&self) -> Result<Duration> {
        let addr = self.server_addr()?;
        let mode = self.latency_mode.load(Ordering::Relaxed);
        if mode != MODE_TCP {
            match super::icmp::ping(addr.ip(), Duration::from_secs(1)) {
                Ok(d) => {
                    self.latency_mode.store(MODE_ICMP, Ordering::Relaxed);
                    return Ok(d);
                }
                Err(e) if mode == MODE_ICMP => return Err(e),
                Err(_) => self.latency_mode.store(MODE_TCP, Ordering::Relaxed),
            }
        }
        let t0 = Instant::now();
        match std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
            Ok(s) => {
                let rtt = t0.elapsed();
                drop(s);
                Ok(rtt)
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Err(NetError::Timeout),
            Err(e) => Err(NetError::Unreachable(format!("{addr}: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_timing_takes_only_cf_request_duration() {
        assert_eq!(server_timing_ms("cfRequestDuration;dur=12.5"), Some(12.5));
        assert_eq!(server_timing_ms("cfL4;desc=\"?rtt=19822\", cfRequestDuration;dur=3"), Some(3.0));
        // What the live server sent on 2026-10-08: nothing to subtract.
        assert_eq!(server_timing_ms("cfSpeedEdge;dur=4, cfSpeedWorker;dur=113"), None);
        assert_eq!(server_timing_ms(""), None);
    }

    #[test]
    fn colo_codes_map_to_cities() {
        assert_eq!(colo_city("LJU"), Some("Ljubljana"));
        assert_eq!(colo_city("VIE"), Some("Vienna"));
        assert_eq!(colo_city("XYZ"), None);
    }
}
