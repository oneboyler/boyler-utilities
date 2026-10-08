//! The WebSocket transport to OBS (obsws.c; there WinHTTP, here a small RFC 6455 client on a plain TCP socket to
//! 127.0.0.1): a worker thread blocks on receive and hands each message on, so nothing runs until OBS says something
//! (0 % CPU idle). Sub-protocol `obswebsocket.json`. One connection at a time; each start has a generation number so a
//! late message of an old connection is told apart.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the transport reports (obsws.c WM_WS_OPEN / WM_WS_MSG / WM_WS_CLOSED).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsEvent {
    Open(u32),
    Msg(u32, String),
    /// generation, close code: 0 = could not connect, 1 = was connected (no code given), else OBS's close code (4009 =
    /// authentication failed)
    Closed(u32, u16),
}

#[derive(Default)]
struct Shared {
    writer: Mutex<Option<TcpStream>>,
    busy: AtomicBool,
    gen: AtomicU32,
    /// the last generation abort() was called for (a connection still on its way then ends right after connecting)
    aborted: AtomicU32,
}

/// The client. Cheap to clone (shared inside).
#[derive(Clone, Default)]
pub struct WsClient {
    s: Arc<Shared>,
}

fn rand_bytes(n: usize) -> Vec<u8> {
    // a mask / key only has to be unpredictable enough for a local socket: time + counter through xorshift
    static C: AtomicU32 = AtomicU32::new(0x9E37_79B9);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    let mut x = t ^ ((C.fetch_add(0x6D2B_79F5, Ordering::Relaxed) as u64) << 32) | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// One client frame (always masked).
fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(payload.len() + 14);
    f.push(0x80 | opcode);
    let n = payload.len();
    if n < 126 {
        f.push(0x80 | n as u8);
    } else if n < 65536 {
        f.push(0x80 | 126);
        f.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        f.push(0x80 | 127);
        f.extend_from_slice(&(n as u64).to_be_bytes());
    }
    let m = rand_bytes(4);
    f.extend_from_slice(&m);
    f.extend(payload.iter().enumerate().map(|(i, b)| b ^ m[i & 3]));
    f
}

impl WsClient {
    pub fn new() -> Self {
        Self::default()
    }

    /// A connection is being made or is open.
    pub fn busy(&self) -> bool {
        self.s.busy.load(Ordering::Acquire)
    }

    /// Connect to 127.0.0.1:`port` on a new thread; `on` gets every event (from that thread). Returns the generation (0 =
    /// a connection is already there).
    pub fn start(&self, port: u16, on: impl Fn(WsEvent) + Send + 'static) -> u32 {
        if self.s.busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return 0;
        }
        let gen = self.s.gen.fetch_add(1, Ordering::AcqRel) + 1;
        let s = self.s.clone();
        let spawned = std::thread::Builder::new().name("obs-ws".into()).stack_size(256 * 1024).spawn(move || {
            let code = run(&s, port, gen, &on);
            if let Ok(mut w) = s.writer.lock() {
                *w = None;
            }
            s.busy.store(false, Ordering::Release);
            on(WsEvent::Closed(gen, code));
        });
        if spawned.is_err() {
            self.s.busy.store(false, Ordering::Release);
            return 0;
        }
        gen
    }

    /// Send one text message (false = not connected / failed).
    pub fn send(&self, text: &str) -> bool {
        let Ok(mut w) = self.s.writer.lock() else { return false };
        match w.as_mut() {
            Some(t) => t.write_all(&frame(1, text.as_bytes())).is_ok(),
            None => false,
        }
    }

    /// Close the connection (the reader thread then reports Closed).
    pub fn abort(&self) {
        self.s.aborted.store(self.s.gen.load(Ordering::Acquire), Ordering::Release);
        if let Ok(mut w) = self.s.writer.lock() {
            if let Some(t) = w.as_mut() {
                let _ = t.write_all(&frame(8, &1000u16.to_be_bytes()));
                let _ = t.shutdown(Shutdown::Both);
            }
        }
    }
}

fn read_exact_or(r: &mut impl Read, n: usize) -> Option<Vec<u8>> {
    let mut b = vec![0u8; n];
    r.read_exact(&mut b).ok()?;
    Some(b)
}

/// The connection's life; returns the close code.
fn run(s: &Shared, port: u16, gen: u32, on: &dyn Fn(WsEvent)) -> u16 {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(3)) else { return 0 };
    // abort() can shut the socket from now on (also during the handshake: no thread left behind when the feature stops)
    if let (Ok(c), Ok(mut g)) = (stream.try_clone(), s.writer.lock()) {
        *g = Some(c);
    }
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let key = crate::auth::b64(&rand_bytes(16));
    let req = format!(
        "GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: obswebsocket.json\r\n\r\n"
    );
    let Ok(mut w) = stream.try_clone() else { return 0 };
    if w.write_all(req.as_bytes()).is_err() {
        return 0;
    }
    let mut r = BufReader::new(stream);
    let mut status = String::new();
    if r.read_line(&mut status).is_err() || status.split_whitespace().nth(1) != Some("101") {
        return 0;
    }
    loop {
        let mut line = String::new();
        match r.read_line(&mut line) {
            Ok(0) | Err(_) => return 0,
            Ok(_) if line == "\r\n" || line == "\n" => break,
            Ok(_) => {}
        }
    }
    if s.aborted.load(Ordering::Acquire) >= gen {
        return 0; // switched off while connecting
    }
    // connected: block on receive from now on (no timeout = no wake-ups while OBS is quiet)
    let _ = r.get_ref().set_read_timeout(None);
    if let Ok(mut g) = s.writer.lock() {
        *g = Some(w);
    }
    on(WsEvent::Open(gen));
    let mut code: u16 = 1;
    let mut msg: Vec<u8> = Vec::new();
    while let Some(h) = read_exact_or(&mut r, 2) {
        let (fin, op) = (h[0] & 0x80 != 0, h[0] & 0x0F);
        let masked = h[1] & 0x80 != 0;
        let mut n = (h[1] & 0x7F) as u64;
        if n == 126 {
            let Some(b) = read_exact_or(&mut r, 2) else { break };
            n = u16::from_be_bytes([b[0], b[1]]) as u64;
        } else if n == 127 {
            let Some(b) = read_exact_or(&mut r, 8) else { break };
            n = u64::from_be_bytes(b.try_into().unwrap_or([0; 8]));
        }
        if n > 64 * 1024 * 1024 {
            break; // nothing OBS sends is this big
        }
        let mask = if masked { read_exact_or(&mut r, 4) } else { None };
        let Some(mut p) = read_exact_or(&mut r, n as usize) else { break };
        if let Some(m) = mask {
            for (i, b) in p.iter_mut().enumerate() {
                *b ^= m[i & 3];
            }
        }
        match op {
            8 => {
                if p.len() >= 2 {
                    let c = u16::from_be_bytes([p[0], p[1]]);
                    if c != 0 {
                        code = c;
                    }
                }
                break;
            }
            9 => {
                if let Ok(mut g) = s.writer.lock() {
                    if let Some(t) = g.as_mut() {
                        let _ = t.write_all(&frame(10, &p));
                    }
                }
            }
            10 => {}
            0..=2 => {
                msg.extend_from_slice(&p);
                if fin {
                    on(WsEvent::Msg(gen, String::from_utf8_lossy(&msg).into_owned()));
                    msg = Vec::new();
                }
            }
            _ => {}
        }
    }
    if let Ok(mut g) = s.writer.lock() {
        if let Some(t) = g.take() {
            let _ = t.shutdown(Shutdown::Both);
        }
    }
    code
}

/// Server-side pieces for test fakes (a fake OBS): read one client frame, write one server frame.
pub mod server {
    use std::io::{Read, Write};

    /// Read one frame from a client: (opcode, payload unmasked). None = closed.
    pub fn read_frame(r: &mut impl Read) -> Option<(u8, Vec<u8>)> {
        let mut h = [0u8; 2];
        r.read_exact(&mut h).ok()?;
        let mut n = (h[1] & 0x7F) as usize;
        if n == 126 {
            let mut b = [0u8; 2];
            r.read_exact(&mut b).ok()?;
            n = u16::from_be_bytes(b) as usize;
        } else if n == 127 {
            let mut b = [0u8; 8];
            r.read_exact(&mut b).ok()?;
            n = u64::from_be_bytes(b) as usize;
        }
        let mut m = [0u8; 4];
        if h[1] & 0x80 != 0 {
            r.read_exact(&mut m).ok()?;
        }
        let mut p = vec![0u8; n];
        r.read_exact(&mut p).ok()?;
        if h[1] & 0x80 != 0 {
            for (i, b) in p.iter_mut().enumerate() {
                *b ^= m[i & 3];
            }
        }
        Some((h[0] & 0x0F, p))
    }

    /// Write one unmasked server frame.
    pub fn write_frame(w: &mut impl Write, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
        let mut f = vec![0x80 | opcode];
        let n = payload.len();
        if n < 126 {
            f.push(n as u8);
        } else if n < 65536 {
            f.push(126);
            f.extend_from_slice(&(n as u16).to_be_bytes());
        } else {
            f.push(127);
            f.extend_from_slice(&(n as u64).to_be_bytes());
        }
        f.extend_from_slice(payload);
        w.write_all(&f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;

    #[test]
    fn connects_exchanges_and_reports_the_close_code() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let srv = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut req = String::new();
            loop {
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                req.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            assert!(req.contains("Sec-WebSocket-Protocol: obswebsocket.json"));
            s.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n").unwrap();
            // a big message, in two fragments
            let big = format!("{{\"op\":0,\"d\":\"{}\"}}", "x".repeat(70000));
            let (a, b) = big.as_bytes().split_at(30000);
            s.write_all(&[0x01, 126, (30000u16 >> 8) as u8, (30000u16 & 255) as u8]).unwrap();
            s.write_all(a).unwrap();
            server::write_frame(&mut s, 0, b).unwrap(); // continuation, FIN
            let (op, p) = server::read_frame(&mut r).unwrap();
            assert_eq!((op, p.as_slice()), (1, &b"hello"[..]));
            server::write_frame(&mut s, 8, &4009u16.to_be_bytes()).unwrap();
        });
        let (tx, rx) = mpsc::channel();
        let c = WsClient::new();
        let gen = c.start(port, move |e| tx.send(e).unwrap());
        assert_eq!(rx.recv().unwrap(), WsEvent::Open(gen));
        match rx.recv().unwrap() {
            WsEvent::Msg(g, m) => {
                assert_eq!(g, gen);
                assert_eq!(m.len(), 70000 + 15);
            }
            e => panic!("{e:?}"),
        }
        assert!(c.send("hello"));
        assert_eq!(rx.recv().unwrap(), WsEvent::Closed(gen, 4009));
        srv.join().unwrap();
        assert!(!c.busy());
    }

    #[test]
    fn nobody_listening_reports_code_0() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let (tx, rx) = mpsc::channel();
        let c = WsClient::new();
        let gen = c.start(port, move |e| tx.send(e).unwrap());
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), WsEvent::Closed(gen, 0));
    }
}
