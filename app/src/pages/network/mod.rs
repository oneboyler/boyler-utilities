//! The Network tab (menu-v22 page `net`, v18a + v21 + v22): Connection (every adapter with its type and switch, the live
//! ping pill, DNS popup with Custom IPv4 + IPv6, Flush DNS), Wi-Fi networks (fold card: connect / password / disconnect /
//! forget), Speed test (glass gauge, Start only on press) and Game servers (pick ONE game, Start pings all its regions once
//! a second until Stop). Wired to crates/network: the FAKE PC (`FakeNet::drawing`) in every test copy, the real one
//! otherwise. Nothing slow starts on open: only the 1-per-second connection ping runs while the page is open (v22 rule:
//! "Live only while this page is open"); the speed test and the game-server pings start only on their buttons.

mod gauge;
pub(crate) mod temp;
mod view;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bu_network::fake::{FakeNet, FakeSpeed};
use bu_network::gameregions::{self, Game};
use bu_network::gameservers::{GameServerEvent, GameServerSampler};
use bu_network::ping::{PingSample, PingSampler, PingTarget};
use bu_network::speedtest::{SpeedConfig, SpeedEvent, SpeedPhase, SpeedResult, SpeedTest, SpeedTransport};
use bu_network::{ConnectionState, DnsChoice, DnsCurrent, DnsServers, DnsState, NetError, NetworkOs, NetworkService, WifiNetwork};

use crate::pages::{Env, Page};
use crate::undo::{DefaultItem, Kind, Resettable, Val};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, sub, El, Key};

// ---- element keys ("net.<name>")
const K_TG: Key = key("net.tg");
const K_DNS: Key = key("net.dns");
const K_FLUSH: Key = key("net.flush");
const K_DNSM: Key = key("net.dnsm");
const K_DNSF: Key = key("net.dnsf");
const K_WF: Key = key("net.wf");
const K_WCONN: Key = key("net.wf.conn");
const K_WDISC: Key = key("net.wf.disc");
const K_WFORGET: Key = key("net.wf.forget");
const K_WROW: Key = key("net.wf.row");
const K_WPW: Key = key("net.wf.pw");
const K_WAUTO: Key = key("net.wf.auto");
const K_WNO: Key = key("net.wf.no");
const K_WGO: Key = key("net.wf.go");
const K_START: Key = key("net.start");
const K_AGAIN: Key = key("net.again");
const K_GSPICK: Key = key("net.gspick");
const K_GSM: Key = key("net.gsm");
const K_GSGO: Key = key("net.gsgo");
const K_TOAST: Key = key("net.toast");
const K_RESET: Key = key("net.reset");

/// The four Custom DNS fields: IPv4 primary / secondary, IPv6 primary / secondary.
const DNS_FIELDS: [&str; 4] = ["v4a", "v4b", "v6a", "v6b"];

/// A result a background call hands back to the page (read on the next `tick`).
enum Msg {
    State(Result<ConnectionState, NetError>, Option<DnsState>),
    Wifi(Result<Vec<WifiNetwork>, NetError>),
    Ping(PingSample),
    /// an adapter's switch finished (its id), with the toast to show
    Switched(String, String),
    DnsSet(String),
    Flushed(Result<(), NetError>),
    Toast(String),
}

type Inbox = Arc<Mutex<Vec<Msg>>>;

#[cfg(test)]
thread_local! {
    /// Order 047's proof (one test): a change runs on its own thread as in the app, `Some(ms)` slower (else in place)
    static SLOW_CHANGE_MS: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// The speed test and the game-server pings live HERE, not in the page (the owner Oct 8, F2: closing the window or leaving the
/// tab must not stop or wipe them): kept in the app's store (`env.keep`, key [`BG_KEY`]) for the app's whole life. Their
/// threads write into it and wake the menu; the page shows what is in it - at once when the tab opens again.
#[derive(Default)]
pub(crate) struct NetBg {
    speed: Speed,
    speed_test: Option<SpeedTest>,
    game: usize,
    gs: Option<GameServerSampler>,
    gs_run: u64,
    gs_rounds: u32,
    /// when the last round of pings came back ("last pinged 5 min ago" after the menu closed them)
    pinged_at: Option<std::time::SystemTime>,
    pills: HashMap<String, Pill>,
    /// toasts from the threads (a finished speed test) for the page's next tick (dropped when the tab opens: stale)
    toasts: Vec<String>,
    /// the Network tab is shown: only then do the threads wake the menu (a closed tab / menu costs no repaints)
    open: bool,
}

type Bg = Arc<Mutex<NetBg>>;
const BG_KEY: &str = "net.bg";

/// The speed test's events into the kept state (pure: tested without threads).
fn speed_event(s: &mut Speed, e: SpeedEvent) {
    match e {
        SpeedEvent::Server(i) => {
            s.foot = Some(if i.city.is_empty() { "Testing".into() } else { format!("Testing · nearest server: {}", i.city) });
            s.server = Some(i);
        }
        SpeedEvent::PhaseStarted(p) => {
            s.phase = Some(p);
            s.live = 0.0;
        }
        SpeedEvent::Progress { phase, mbps, .. } => {
            s.live = mbps;
            match phase {
                SpeedPhase::Download => s.down = Some(mbps),
                SpeedPhase::Upload => s.up = Some(mbps),
                SpeedPhase::Latency => {}
            }
        }
        SpeedEvent::LatencySample { ms } => {
            s.lat = Some(ms);
            s.ping = Some(s.ping.map_or(ms, |p| p.min(ms)));
        }
        SpeedEvent::PhaseDone { phase, value } => match phase {
            SpeedPhase::Download => s.down = Some(value),
            SpeedPhase::Upload => s.up = Some(value),
            SpeedPhase::Latency => s.ping = Some(value),
        },
        SpeedEvent::Done(r) => {
            s.down = Some(r.download_mbps);
            s.up = Some(r.upload_mbps);
            s.ping = Some(r.ping_ms);
            s.jit = Some(r.jitter_ms);
        }
    }
}

/// Which popup is open.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pop {
    /// the DNS list under the DNS button
    Dns,
    /// the Custom DNS form (same place)
    DnsForm,
    /// the game picker's list
    Games,
}

/// The speed test's state on the page.
#[derive(Clone, Debug, Default)]
struct Speed {
    run: bool,
    done: bool,
    phase: Option<SpeedPhase>,
    /// the live Mb/s of the running phase
    live: f64,
    /// the last latency sample (the big number during the ping phase)
    lat: Option<f64>,
    down: Option<f64>,
    up: Option<f64>,
    ping: Option<f64>,
    jit: Option<f64>,
    /// "Testing · nearest server: Zagreb" / "Today 21:37 · Ethernet"
    foot: Option<String>,
    /// the server the test talks to (what the ping goes to: "Cloudflare · Zagreb (ZAG)")
    server: Option<bu_network::speedtest::ServerInfo>,
}

/// One region pill.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Pill {
    ms: Option<u32>,
    /// pinged at least once in this run (a number or a timeout came back)
    seen: bool,
}

#[derive(Default)]
pub struct Network {
    env: Env,
    os: Option<Arc<dyn NetworkOs>>,
    /// the fake PC of a test copy (tests read its change log)
    fake: Option<Arc<FakeNet>>,
    svc: Option<Arc<NetworkService>>,
    inbox: Inbox,
    conn: Option<ConnectionState>,
    dns: Option<DnsState>,
    wifi: Vec<WifiNetwork>,
    ping: Option<PingSampler>,
    ping_ms: Option<u32>,
    /// adapters whose switch is being changed (`.tg.wait`)
    waiting: Vec<String>,
    dns_wait: bool,
    flushed_at: Option<f64>,
    pop: Option<Pop>,
    pop_at: f64,
    dns_anchor: (f32, f32, f32, f32),
    gs_anchor: (f32, f32, f32, f32),
    dns_fields: [String; 4],
    dns_bad: [bool; 4],
    wf_open: bool,
    /// the Wi-Fi row whose password field is open
    wf_pw_for: Option<String>,
    wf_pw: String,
    /// when Connect was pressed with an empty password (the field's nudge pulse)
    nudge_at: Option<f64>,
    wf_auto: bool,
    /// the kept speed test + game pings (`NetBg`); the fields below are the page's copy of it, taken each tick
    bg: Bg,
    speed: Speed,
    speed_running: bool,
    games: Vec<Game>,
    game: usize,
    gs_running: bool,
    gs_rounds: u32,
    pinged_at: Option<std::time::SystemTime>,
    /// stops the game pings when the menu window closes (this page object lives as long as the window: its Drop)
    guard: PingGuard,
    /// test copies only (`BU_TEST_STATE gsrun`): shown as running, no pings sent (the pixel proof's fixed numbers)
    gs_preset: bool,
    pills: HashMap<String, Pill>,
    toast: Option<(String, f64)>,
    now: f64,
    /// test copies only (`BU_TEST_STATE scroll=<px>`): the page drawn moved up by this much - what scrolling does inside the
    /// page's clip - so the off-screen proof reaches below the fold until the test hook has a scroll command
    shift: f32,
    /// Order 036: the reset link's box (where the shared review opens) and, for a CLOSED page (Settings › Reset, the
    /// uninstaller), the service a reset uses - made on first use: the fake PC in a test copy, Windows otherwise
    reset_at: (f32, f32, f32, f32),
    rs_svc: std::cell::OnceCell<Arc<NetworkService>>,
}

/// A test copy's preset states for the pixel proof (`BU_TEST_STATE`, comma separated; read only in a FAKE test copy): `scroll=<px>`,
/// `dns` (the DNS list open), `dnsform` (the Custom form), `wifi` (the card open), `wifipw` (+ a password field open),
/// `games` (the game list open), `speeddone` (a finished test with sample numbers).
fn test_states() -> Vec<String> {
    std::env::var("BU_TEST_STATE").map(|s| s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()).unwrap_or_default()
}

impl Network {
    fn apply_test_states(&mut self) {
        for t in test_states() {
            match t.as_str() {
                "dns" => {
                    self.pop = Some(Pop::Dns);
                    self.dns_anchor = (356.8, 110.0 - self.shift, 123.56, 22.0);
                }
                "dnsform" => {
                    self.dns_anchor = (356.8, 110.0 - self.shift, 123.56, 22.0);
                    self.open_dns_form();
                }
                "wifi" => self.wf_open = true,
                "wifipw" => {
                    self.wf_open = true;
                    self.wf_pw_for = Some("TP-Link_8F2C".into());
                }
                "games" => {
                    self.pop = Some(Pop::Games);
                    self.gs_anchor = (72.0, 817.3 - self.shift, 128.0, 24.0);
                }
                // REVIEW 022 HOLD 1: the running states at fixed numbers (the drawing is set to the same numbers by script)
                "speedping" => {
                    // the ping phase: "Ping" + its icon over the last round trip (ms), track and ticks faded, Ping + Jitter rows "now"
                    self.speed = Speed {
                        run: true,
                        phase: Some(SpeedPhase::Latency),
                        lat: Some(9.0),
                        down: Some(921.4),
                        up: Some(108.7),
                        ping: Some(9.0),
                        foot: Some("Testing · nearest server: Zagreb".into()),
                        ..Speed::default()
                    }
                }
                "speeddown" => {
                    self.speed = Speed {
                        run: true,
                        phase: Some(SpeedPhase::Download),
                        live: 612.4,
                        down: Some(612.4),
                        foot: Some("Testing · nearest server: Zagreb".into()),
                        ..Speed::default()
                    }
                }
                "gsrun" => {
                    // VALORANT running, second round in: every region at the drawing's base number, "Best" on the lowest
                    self.gs_preset = true;
                    self.gs_rounds = 2;
                    let base = [19u32, 29, 37, 44, 26, 47, 52];
                    for (r, ms) in self.games[self.game].regions.iter().zip(base) {
                        self.pills.insert(r.server.id.clone(), Pill { ms: Some(ms), seen: true });
                    }
                }
                "speeddone" => {
                    self.speed = Speed {
                        done: true,
                        down: Some(921.4),
                        up: Some(108.7),
                        ping: Some(9.0),
                        jit: Some(0.6),
                        foot: Some("Today 21:37 · Ethernet".into()),
                        ..Speed::default()
                    }
                }
                s => {
                    if let Some(px) = s.strip_prefix("scroll=").and_then(|v| v.parse().ok()) {
                        self.shift = px;
                    }
                }
            }
        }
    }

    fn svc(&self) -> Option<Arc<NetworkService>> {
        self.svc.clone()
    }

    fn post(inbox: &Inbox, m: Msg) {
        if let Ok(mut q) = inbox.lock() {
            q.push(m);
        }
        // the menu reads it on its next tick
        crate::services::Waker.wake();
    }

    fn toast(&mut self, t: impl Into<String>) {
        self.toast = Some((t.into(), self.now));
    }

    /// Reads adapters + DNS + Wi-Fi (light reads; never a scan). The fake answers at once, so a test copy reads in place
    /// (the first picture is complete); the real PC is read on a short-lived thread (no wait on the UI thread).
    fn read_all(&mut self) {
        let Some(svc) = self.svc() else { return };
        let inbox = self.inbox.clone();
        let job = move || {
            let st = svc.connection_state();
            let dns = svc.dns_state().ok();
            Self::post(&inbox, Msg::State(st, dns));
            Self::post(&inbox, Msg::Wifi(svc.wifi_networks()));
        };
        if self.env.fake() {
            job();
            self.drain();
        } else {
            let _ = std::thread::Builder::new().name("bu-net-read".into()).spawn(job);
        }
    }

    /// Runs one change on a short-lived thread; its result comes back through the inbox.
    fn spawn(&self, f: impl FnOnce(&NetworkService, &Inbox) + Send + 'static) {
        let (Some(svc), inbox) = (self.svc(), self.inbox.clone()) else { return };
        #[cfg(test)]
        if let Some(ms) = SLOW_CHANGE_MS.with(|c| c.get()) {
            // Order 047's proof: the app's own path (a thread), each change as slow as a slow PC
            let _ = std::thread::Builder::new().name("bu-net-change".into()).spawn(move || {
                std::thread::sleep(Duration::from_millis(ms));
                f(&svc, &inbox)
            });
            return;
        }
        if cfg!(test) {
            // unit tests: in place (a change's change-log entry is then written on the test's own thread, into its own store)
            f(&svc, &inbox);
            return;
        }
        let _ = std::thread::Builder::new().name("bu-net-change".into()).spawn(move || f(&svc, &inbox));
    }

    fn drain(&mut self) -> bool {
        let msgs: Vec<Msg> = match self.inbox.lock() {
            Ok(mut q) => std::mem::take(&mut *q),
            Err(_) => return false,
        };
        let any = !msgs.is_empty();
        for m in msgs {
            self.apply(m);
        }
        any
    }

    fn apply(&mut self, m: Msg) {
        match m {
            Msg::State(st, dns) => {
                match st {
                    Ok(c) => self.conn = Some(c),
                    Err(e) => self.toast(format!("Can't read the adapters · {e}")),
                }
                self.dns = dns;
            }
            Msg::Wifi(w) => self.wifi = w.unwrap_or_default(),
            Msg::Ping(p) => self.ping_ms = p.ms(),
            Msg::Switched(id, t) => {
                self.waiting.retain(|w| *w != id);
                self.toast(t);
                self.read_all();
            }
            Msg::DnsSet(t) => {
                self.dns_wait = false;
                self.toast(t);
                self.read_all();
            }
            Msg::Flushed(r) => match r {
                Ok(()) => {
                    self.flushed_at = Some(self.now);
                    self.toast("DNS cache flushed");
                }
                Err(e) => self.toast(format!("Couldn't flush DNS · {e}")),
            },
            Msg::Toast(t) => {
                self.toast(t);
                self.read_all();
            }
        }
    }

    fn offline(&self) -> bool {
        self.conn.as_ref().map(|c| c.offline()).unwrap_or(true)
    }

    /// Admin changes: the real layer asks Windows' admin prompt itself (its proxy counts as elevated); a layer that can't
    /// (a test copy's fake) says so and changes nothing (COMMON: never a silent failure).
    fn admin_ok(&mut self) -> bool {
        if self.os.as_ref().map(|o| o.is_elevated()).unwrap_or(false) {
            return true;
        }
        self.toast(crate::admin::NOT_CHANGED);
        false
    }

    // ---- actions (only ever from a click)

    fn switch_adapter(&mut self, i: usize) {
        let Some(a) = self.conn.as_ref().and_then(|c| c.adapters.get(i)).cloned() else { return };
        if self.waiting.contains(&a.id) {
            return;
        }
        if NetworkService::switch_action(&a).needs_admin() && !self.admin_ok() {
            return;
        }
        let on = !a.enabled;
        self.waiting.push(a.id.clone());
        let id = a.id.clone();
        let log = can_log();
        self.spawn(move |svc, inbox| {
            let t = match svc.set_adapter(&id, on) {
                Ok(ch) => {
                    if log {
                        note_change(&ch, &id, &a.name);
                    }
                    let now = svc.connection_state().ok();
                    match now.as_ref().and_then(|c| c.in_use_adapter()) {
                        None => "You’re offline".to_string(),
                        Some(u) if !on => format!("{} is off · {} takes over", a.name, u.name),
                        _ => ch.toast(),
                    }
                }
                Err(e) => format!("{} · {}", a.name, net_err(&e)),
            };
            Self::post(inbox, Msg::Switched(id, t));
        });
    }

    fn flush(&mut self) {
        if self.flushed_at.is_some_and(|t| self.now - t < 1600.0) {
            return;
        }
        self.spawn(|svc, inbox| Self::post(inbox, Msg::Flushed(svc.flush_dns())));
    }

    fn set_dns(&mut self, servers: Option<DnsServers>, choice: Option<DnsChoice>) {
        self.pop = None;
        if !self.admin_ok() {
            return;
        }
        self.dns_wait = true;
        let log = can_log();
        self.spawn(move |svc, inbox| {
            let r = match (choice, servers) {
                (Some(c), _) => svc.set_dns(c),
                (None, Some(s)) => svc.set_dns_custom(s),
                _ => return,
            };
            if let (true, Ok(ch)) = (log, &r) {
                note_change(ch, "", "");
            }
            Self::post(inbox, Msg::DnsSet(match r {
                Ok(ch) => ch.toast(),
                Err(e @ (NetError::NeedsAdmin | NetError::AccessDenied(_))) => net_err(&e),
                Err(e) => format!("DNS not changed · {e}"),
            }));
        });
    }

    fn dns_save(&mut self) {
        let f = &self.dns_fields;
        match DnsServers::from_fields(&f[0], &f[1], &f[2], &f[3]) {
            Ok(s) => self.set_dns(Some(s), None),
            Err(bad) => {
                self.dns_bad = bad;
                self.toast("That address doesn’t look right");
            }
        }
    }

    fn open_dns_form(&mut self) {
        self.pop = Some(Pop::DnsForm);
        self.pop_at = self.now;
        self.dns_bad = [false; 4];
        let cur = self.dns.as_ref().filter(|d| d.current == DnsCurrent::Custom).map(|d| d.configured.clone());
        self.dns_fields = match cur {
            Some(c) => [
                c.v4.first().map(|a| a.to_string()).unwrap_or_default(),
                c.v4.get(1).map(|a| a.to_string()).unwrap_or_default(),
                c.v6.first().map(|a| a.to_string()).unwrap_or_default(),
                c.v6.get(1).map(|a| a.to_string()).unwrap_or_default(),
            ],
            // the drawing's own sample (Quad9) in a test copy; empty on the real PC
            None if self.env.fake() => ["9.9.9.9", "149.112.112.112", "2620:fe::fe", "2620:fe::9"].map(String::from),
            None => Default::default(),
        };
    }

    fn wifi_connect(&mut self, ssid: String, password: Option<String>) {
        let auto = self.wf_auto;
        self.wf_pw_for = None;
        self.wf_pw.clear();
        self.toast(format!("Connecting to {ssid}…"));
        self.spawn(move |svc, inbox| {
            let t = match svc.wifi_connect(&ssid, password.as_deref(), auto) {
                Ok(()) => format!("Connected to {ssid}"),
                Err(NetError::AccessDenied(_)) => format!("Couldn't connect to {ssid} · check the password"),
                Err(e) => format!("Couldn't connect to {ssid} · {e}"),
            };
            Self::post(inbox, Msg::Toast(t));
        });
    }

    /// The page's copy of the kept speed test + game pings (`NetBg`), and the speed test's end once its thread is done.
    /// True = something new for the page.
    fn sync(&mut self) -> bool {
        let (finished, toasts) = {
            let Ok(mut b) = self.bg.lock() else { return false };
            self.speed = b.speed.clone();
            self.speed_running = b.speed_test.is_some();
            self.game = b.game.min(self.games.len().saturating_sub(1));
            self.gs_running = b.gs.is_some();
            self.gs_rounds = b.gs_rounds;
            self.pinged_at = b.pinged_at;
            self.pills = b.pills.clone();
            let f = if b.speed_test.as_ref().is_some_and(|t| t.is_finished()) { b.speed_test.take() } else { None };
            (f, std::mem::take(&mut b.toasts))
        };
        let any = finished.is_some() || !toasts.is_empty();
        for t in toasts {
            self.toast(t);
        }
        if let Some(t) = finished {
            // joined outside the lock: the thread is done, this returns at once
            let r = t.join();
            if let Ok(mut b) = self.bg.lock() {
                b.speed.run = false;
                match r {
                    Ok(_) | Err(NetError::Cancelled) => {}
                    Err(e) => {
                        b.speed = Speed::default();
                        b.toasts.push(if e == NetError::Offline { "You’re offline".to_string() } else { format!("Speed test failed · {e}") });
                    }
                }
            }
            self.speed_running = false;
            return self.sync() || any;
        }
        any
    }

    /// The test copy's preset states (`BU_TEST_STATE`) go into the kept state too (the page shows what is kept).
    fn push_test_states(&mut self) {
        if let Ok(mut b) = self.bg.lock() {
            b.speed = self.speed.clone();
            b.pills = self.pills.clone();
            b.gs_rounds = self.gs_rounds;
            b.game = self.game;
        }
    }

    fn start_speed(&mut self) {
        if self.speed_running {
            return;
        }
        if self.offline() {
            self.toast("You’re offline");
            return;
        }
        let Some(os) = self.os.clone() else { return };
        let transport: Arc<dyn SpeedTransport> = if self.env.fake() {
            Arc::new(FakeSpeed::drawing())
        } else {
            match bu_network::real::CloudflareSpeed::new() {
                Ok(t) => Arc::new(t),
                Err(e) => {
                    self.toast(format!("Speed test failed · {e}"));
                    return;
                }
            }
        };
        // a test copy runs a short test (the fake's numbers are fixed); the real one speedtest.net's length
        let cfg = if self.env.fake() {
            SpeedConfig { download_time: Duration::from_millis(1500), upload_time: Duration::from_millis(1500), ..SpeedConfig::default() }
        } else {
            SpeedConfig::default()
        };
        // "Today 21:37 · Ethernet" when it ends: the adapter in use now
        let on = self.conn.as_ref().and_then(|c| c.in_use_adapter()).map(|a| a.name.clone()).unwrap_or_else(|| "—".into());
        let frozen = self.env.frozen;
        // a Weak handle: the threads must not keep the kept state alive (it owns them - a cycle)
        let (bg, w) = (Arc::downgrade(&self.bg), self.env.waker());
        if let Ok(mut b) = self.bg.lock() {
            if b.speed_test.is_some() {
                return;
            }
            b.speed = Speed { run: true, foot: Some("Testing".into()), ..Speed::default() };
            b.toasts.clear();
        }
        // the thread writes into the kept state: it goes on (and its result is kept) with the tab left or the window closed
        let test = SpeedTest::start(os, transport, cfg, move |e| {
            let Some(bg) = bg.upgrade() else { return };
            let mut wake = false;
            if let Ok(mut b) = bg.lock() {
                wake = b.open;
                let done = if let SpeedEvent::Done(r) = &e { Some(view::speed_toast(r)) } else { None };
                speed_event(&mut b.speed, e);
                if let Some(t) = done {
                    let when = if frozen { "21:37".to_string() } else { bu_network::real::WindowsNet::local_hhmm() };
                    b.speed.run = false;
                    b.speed.done = true;
                    b.speed.phase = None;
                    b.speed.foot = Some(format!("Today {when} · {on}"));
                    b.toasts.push(t);
                }
            }
            // the gauge moves 10 times a second while the tab is shown; nothing while it is not
            if wake {
                w.wake();
            }
        });
        if let Ok(mut b) = self.bg.lock() {
            b.speed_test = Some(test);
        }
        self.sync();
    }

    fn gs_start(&mut self) {
        if self.gs_running {
            return;
        }
        if self.offline() {
            self.toast("You’re offline");
            return;
        }
        let Some(os) = self.os.clone() else { return };
        let Ok(mut b) = self.bg.lock() else { return };
        if b.gs.is_some() {
            return;
        }
        b.gs_run += 1;
        b.gs_rounds = 0;
        b.pills.clear();
        let run = b.gs_run;
        let (bg, w) = (Arc::downgrade(&self.bg), self.env.waker());
        let servers = self.games[b.game.min(self.games.len() - 1)].regions.iter().map(|r| r.server.clone()).filter(|s| !s.targets.is_empty()).collect();
        // one ping per region per second (`setInterval(gsTick,1000)`), until Stop, another game or the menu closing (PingGuard) - it
        // keeps going with the tab left; a stopped run's late events are dropped (the run number moved on)
        b.gs = Some(GameServerSampler::start_with_tries(os, servers, Some(Duration::from_secs(1)), 1, move |e| {
            let Some(bg) = bg.upgrade() else { return };
            let mut wake = false;
            if let Ok(mut b) = bg.lock() {
                // one repaint a round (not one per region), and only while the tab is shown
                wake = b.open && matches!(e, GameServerEvent::RoundDone);
                if b.gs_run == run {
                    match e {
                        GameServerEvent::RoundStarted => {}
                        GameServerEvent::Result(r) => {
                            let p = b.pills.entry(r.id.clone()).or_default();
                            p.ms = r.rtt.map(bu_network::ping::round_ms);
                            p.seen = true;
                        }
                        GameServerEvent::RoundDone => {
                            b.gs_rounds += 1;
                            b.pinged_at = Some(std::time::SystemTime::now());
                        }
                    }
                }
            }
            if wake {
                w.wake();
            }
        }));
        drop(b);
        self.sync();
    }

    fn gs_stop(&mut self) {
        let g = self.bg.lock().ok().and_then(|mut b| {
            b.gs_run += 1;
            b.gs.take()
        });
        if let Some(g) = g {
            // the thread ends within one probe timeout; don't hold the UI (or the kept state's lock) for it
            let _ = std::thread::Builder::new().name("bu-net-gs-stop".into()).spawn(move || g.stop());
        }
        self.sync();
    }
}

/// The menu window closed (its pages go with it): the game pings stop - an endless job runs only while the menu is open
/// (boss call, Oct 8: "no CPU/RAM unless you are doing something in it"). Their last numbers stay, with when they came
/// ("last pinged 5 min ago") and a Start button. The speed test, a finite job, goes on to its end (F2).
#[derive(Default)]
struct PingGuard {
    bg: Option<Bg>,
}

impl Drop for PingGuard {
    fn drop(&mut self) {
        let Some(bg) = self.bg.take() else { return };
        let g = bg.lock().ok().and_then(|mut b| {
            b.open = false;
            b.gs_run += 1;
            b.gs.take()
        });
        if let Some(g) = g {
            // stopped on its own thread: never hold the closing window (or the lock) for its last probes
            let _ = std::thread::Builder::new().name("bu-net-gs-stop".into()).spawn(move || g.stop());
        }
    }
}

impl Page for Network {
    fn id(&self) -> &'static str {
        "net"
    }
    fn name(&self) -> &'static str {
        "Network"
    }
    fn icon(&self) -> &'static str {
        "netw"
    }
    fn ready(&self) -> bool {
        self.svc.is_none() || self.conn.is_some()
    }

    fn open(&mut self, env: &Env, now: f64) {
        self.close();
        self.env = env.clone();
        self.now = now;
        let os: Arc<dyn NetworkOs> = if env.fake() {
            let f = Arc::new(FakeNet::drawing());
            self.fake = Some(f.clone());
            f
        } else {
            // the Ethernet switch and DNS go to the app's elevated copy: one admin prompt each (Order 039)
            Arc::new(crate::admin::proxy::NetOs::new(Arc::new(bu_network::real::WindowsNet::new()), crate::admin::client::admin()))
        };
        self.svc = Some(Arc::new(NetworkService::new(os.clone())));
        self.games = gameregions::games();
        // the kept speed test + game pings (they ran on while the tab was away): shown at once; old toasts are stale
        self.bg = match env.keep.get::<Bg>(BG_KEY) {
            Some(b) => b,
            None => {
                let b: Bg = Arc::default();
                env.keep.put(BG_KEY, b.clone());
                b
            }
        };
        if let Ok(mut b) = self.bg.lock() {
            b.toasts.clear();
            b.open = true;
        }
        if self.guard.bg.is_none() {
            self.guard.bg = Some(self.bg.clone());
        }
        self.sync();
        self.wf_auto = true;
        self.read_all();
        // the connection pill: one ping a second to 1.1.1.1 while this page is open (the drawing's nwPing); stopped in close
        let inbox = self.inbox.clone();
        self.ping = Some(PingSampler::start(os.clone(), PingTarget::Internet, PingSampler::INTERVAL, move |s| Self::post(&inbox, Msg::Ping(s))));
        self.os = Some(os);
        if env.fake() {
            // the fake answers 12 ms at once: the first picture has its number (the drawing's sample is 10-14 ms)
            std::thread::sleep(Duration::from_millis(5));
            self.drain();
        }
        if env.fake() {
            // scroll first: the popups' anchors follow it
            if let Some(px) = test_states().iter().find_map(|t| t.strip_prefix("scroll=").and_then(|v| v.parse().ok())) {
                self.shift = px;
            }
            self.apply_test_states();
            self.push_test_states();
        }
    }

    fn close(&mut self) {
        // leaving the tab stops the connection ping ("Live only while this page is open"); the speed test and the game pings
        // go on in the kept state (F2)
        if let Some(p) = self.ping.take() {
            let _ = std::thread::Builder::new().name("bu-net-ping-stop".into()).spawn(move || p.stop());
        }
        if let Ok(mut b) = self.bg.lock() {
            b.open = false;
        }
        // the gauge's cached pictures go too (closed = small RAM; REVIEW 022 HOLD 8)
        gauge::clear();
        *self = Network { inbox: Arc::new(Mutex::new(Vec::new())), guard: std::mem::take(&mut self.guard), ..Network::default() };
    }

    fn tick(&mut self, now: f64) -> bool {
        self.now = now;
        let a = self.drain();
        let b = self.sync();
        a || b
    }

    /// Order 047: the threads wake the menu with each result (the connection ping once a second, the speed test's
    /// samples, a round of game pings), so `tick` is true only when one came. One gap: a speed test's thread ends just
    /// after its last wake, and its end (the join, Start again) is seen at a tick - so while one runs, a tick every
    /// 100 ms (it paints only when `sync` says something changed).
    fn wake_at(&self, now: f64) -> Option<f64> {
        self.speed_running.then_some(now + 100.0)
    }

    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        self.now = cx.now;
        view::page(self, cx)
    }

    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        self.now = cx.now;
        match ev {
            Ev::Press(k, _, _, r) if *k == K_DNS => self.dns_anchor = *r,
            Ev::Press(k, _, _, r) if *k == K_GSPICK => self.gs_anchor = *r,
            Ev::Press(k, _, _, r) if *k == sub(K_RESET, "pc") || *k == sub(K_RESET, "win") => self.reset_at = *r,
            Ev::Click(k) => self.click(*k, cx),
            Ev::Char(k, c) => {
                if let Some(i) = (0..4).find(|i| *k == sub(K_DNSF, DNS_FIELDS[*i])) {
                    crate::ui::pieces::search::edit_char(&mut self.dns_fields[i], *c);
                    self.dns_bad[i] = false;
                } else if *k == K_WPW {
                    crate::ui::pieces::search::edit_char(&mut self.wf_pw, *c);
                }
            }
            Ev::Key(k, vk) => {
                let field = (0..4).find(|i| *k == sub(K_DNSF, DNS_FIELDS[*i]));
                match (*vk, field, *k == K_WPW) {
                    (0x0D, Some(_), _) => self.dns_save(),
                    (0x1B, Some(_), _) => self.pop = None,
                    (0x08, Some(i), _) => {
                        self.dns_fields[i].pop();
                        self.dns_bad[i] = false;
                    }
                    // Tab: the next field
                    // (Order 045: used - the frame's own Tab order does not move it on)
                    (0x09, Some(i), _) => {
                        cx.focus(Some(sub(K_DNSF, DNS_FIELDS[(i + 1) % 4])));
                        cx.used = true;
                    }
                    (0x0D, None, true) => self.wifi_go(),
                    (0x1B, None, true) => {
                        self.wf_pw_for = None;
                        self.wf_pw.clear();
                    }
                    (0x08, None, true) => {
                        self.wf_pw.pop();
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        view::popup(self, cx)
    }

    fn popup_dismiss(&mut self) {
        self.pop = None;
    }

    fn describe(&self) -> String {
        let ad = self.conn.as_ref().map(|c| c.adapters.len()).unwrap_or(0);
        format!(
            "adapters={ad} wifi={} speed={} gs={} game={} rounds={} pop={:?}",
            self.wifi.len(),
            if self.speed_running { "run" } else if self.speed.done { "done" } else { "idle" },
            if self.gs_running { "run" } else { "idle" },
            self.games.get(self.game).map(|g| g.id).unwrap_or("-"),
            self.gs_rounds,
            self.pop
        )
    }
    /// Order 036: cheap (nothing is opened here); a closed page makes its service on the first reset call.
    fn resettable(&mut self) -> Option<&mut dyn Resettable> {
        Some(self)
    }
}

impl Network {
    fn wifi_go(&mut self) {
        let Some(ssid) = self.wf_pw_for.clone() else { return };
        if self.wf_pw.is_empty() {
            // `if(!pw.value){nudge(pw.parentNode);return;}` - the field pulses: "type here"
            self.nudge_at = Some(self.now);
            return;
        }
        let pw = std::mem::take(&mut self.wf_pw);
        self.wifi_connect(ssid, Some(pw));
    }

    fn click(&mut self, k: Key, cx: &mut Cx) {
        let n_ad = self.conn.as_ref().map(|c| c.adapters.len()).unwrap_or(0);
        if let Some(i) = (0..n_ad).find(|i| k == idx(K_TG, *i)) {
            self.switch_adapter(i);
            return;
        }
        let n_wf = self.wifi.len();
        if let Some(i) = (0..n_wf).find(|i| k == idx(K_WCONN, *i)) {
            let w = self.wifi[i].clone();
            if w.secured && !w.saved {
                self.wf_pw_for = Some(w.ssid);
                self.wf_pw.clear();
                cx.focus(Some(K_WPW));
            } else {
                self.wifi_connect(w.ssid, None);
            }
            return;
        }
        if let Some(i) = (0..n_wf).find(|i| k == idx(K_WDISC, *i)) {
            let ssid = self.wifi[i].ssid.clone();
            let other = self.conn.as_ref().and_then(|c| c.adapters.iter().find(|a| a.kind == bu_network::AdapterKind::Ethernet && a.connected)).map(|_| " · Ethernet stays on").unwrap_or("");
            let t = format!("Disconnected from {ssid}{other}");
            self.spawn(move |svc, inbox| {
                Self::post(inbox, Msg::Toast(match svc.wifi_disconnect() {
                    Ok(()) => t,
                    Err(e) => format!("Couldn't disconnect · {e}"),
                }))
            });
            return;
        }
        if let Some(i) = (0..n_wf).find(|i| k == idx(K_WFORGET, *i)) {
            let ssid = self.wifi[i].ssid.clone();
            self.spawn(move |svc, inbox| {
                Self::post(inbox, Msg::Toast(match svc.wifi_forget(&ssid) {
                    Ok(()) => format!("{ssid} forgotten · its password is gone from this PC"),
                    Err(e) => format!("Couldn't forget {ssid} · {e}"),
                }))
            });
            return;
        }
        let n_dns = 4;
        if let Some(i) = (0..n_dns).find(|i| k == idx(K_DNSM, *i)) {
            match i {
                0 => self.set_dns(None, Some(DnsChoice::Automatic)),
                1 => self.set_dns(None, Some(DnsChoice::Cloudflare)),
                2 => self.set_dns(None, Some(DnsChoice::Google)),
                _ => self.open_dns_form(),
            }
            return;
        }
        if let Some(i) = (0..self.games.len()).find(|i| k == idx(K_GSM, *i)) {
            self.pop = None;
            if i != self.game {
                self.gs_stop();
                if let Ok(mut b) = self.bg.lock() {
                    b.game = i;
                    b.pills.clear();
                    b.gs_rounds = 0;
                }
                self.sync();
            }
            return;
        }
        if let Some(i) = (0..4).find(|i| k == sub(K_DNSF, DNS_FIELDS[*i])) {
            cx.focus(Some(sub(K_DNSF, DNS_FIELDS[i])));
            return;
        }
        match k {
            _ if k == K_DNS => {
                if self.pop == Some(Pop::Dns) || self.pop == Some(Pop::DnsForm) {
                    self.pop = None;
                } else if !self.offline() && !self.dns_wait {
                    self.pop = Some(Pop::Dns);
                    self.pop_at = self.now;
                }
            }
            _ if k == K_FLUSH => self.flush(),
            // Order 036: the frame's ONE review over the change log, applied through `Resettable` below
            _ if k == sub(K_RESET, "pc") => cx.open_reset(Kind::HowItWas, self.reset_at),
            _ if k == sub(K_RESET, "win") => cx.open_reset(Kind::WindowsDefaults, self.reset_at),
            _ if k == sub(K_DNSF, "save") => self.dns_save(),
            _ if k == sub(K_DNSF, "cancel") => self.pop = None,
            _ if k == sub(K_WF, "fold") || k == sub(K_WF, "cx") => self.wf_open = !self.wf_open,
            _ if k == K_WAUTO => self.wf_auto = !self.wf_auto,
            _ if k == K_WNO => {
                self.wf_pw_for = None;
                self.wf_pw.clear();
            }
            // an empty field: the drawing only nudges it (`go.click`, Enter does the same)
            _ if k == K_WGO => self.wifi_go(),
            _ if k == K_WPW => cx.focus(Some(K_WPW)),
            _ if k == K_START || k == K_AGAIN => self.start_speed(),
            _ if k == K_GSPICK => {
                if self.pop == Some(Pop::Games) {
                    self.pop = None;
                } else {
                    self.pop = Some(Pop::Games);
                    self.pop_at = self.now;
                }
            }
            _ if k == K_GSGO => {
                if self.gs_running {
                    self.gs_stop();
                } else {
                    self.gs_start();
                }
            }
            _ => {}
        }
    }
}

// =================================================================================================== Order 036: reset
// Network's change-log items (the drawing's RS.net): an adapter's switch ("adapter:<id>": the device, or Wi-Fi's radio) and
// an adapter's hand-set DNS ("dns:<id>"). Flush DNS, Wi-Fi connect / disconnect / forget, the speed test and the pings
// write nothing (one-time actions / live connections, nothing that stays changed in Windows' settings).

/// The app is running (it has the services, maybe busy further up the stack): a unit test without them logs nothing (its
/// note would wait in the process-wide queue for another test's store).
fn can_log() -> bool {
    crate::services::in_use() || crate::services::with(|_| ()).is_some()
}

fn on_val(on: bool) -> Val {
    if on {
        Val::new("on", "On")
    } else {
        Val::new("off", "Off")
    }
}

/// "auto" or "v4,v4|v6,v6"; shown as the drawing does: "Automatic", "Cloudflare", "Google", "Custom · 9.9.9.9".
fn dns_val(s: &DnsServers) -> Val {
    if s.is_automatic() {
        return Val::new("auto", "Automatic");
    }
    let j = |v: Vec<String>| v.join(",");
    let raw = format!("{}|{}", j(s.v4.iter().map(|a| a.to_string()).collect()), j(s.v6.iter().map(|a| a.to_string()).collect()));
    let text = match DnsCurrent::classify(s) {
        DnsCurrent::Custom => {
            let first = s.v4.first().map(|a| a.to_string()).or_else(|| s.v6.first().map(|a| a.to_string())).unwrap_or_default();
            format!("Custom \u{00b7} {first}")
        }
        c => c.label().to_string(),
    };
    Val::new(&raw, &text)
}

fn parse_dns(raw: &str) -> Option<DnsServers> {
    if raw == "auto" {
        return Some(DnsServers::default());
    }
    let (v4, v6) = raw.split_once('|')?;
    Some(DnsServers { v4: addr_list(v4)?, v6: addr_list(v6)? })
}

fn addr_list<T: std::str::FromStr>(s: &str) -> Option<Vec<T>> {
    s.split(',').filter(|x| !x.is_empty()).map(|x| x.parse().ok()).collect()
}

fn dns_label(name: &str) -> String {
    format!("DNS ({name})")
}

/// One change the page made (on its change thread) into the change log: old → new, only when something changed. A Wi-Fi
/// radio change names its adapter (`wifi_id`, `wifi_name`: the switched row).
fn note_change(ch: &bu_network::Change, wifi_id: &str, wifi_name: &str) {
    use bu_network::ChangeKind;
    match &ch.kind {
        ChangeKind::WifiRadio { was_on, now_on } if was_on != now_on => {
            crate::undo::note("net", &format!("adapter:{wifi_id}"), wifi_name, &on_val(*was_on), &on_val(*now_on))
        }
        ChangeKind::Adapter { id, name, was_on, now_on } if was_on != now_on => {
            crate::undo::note("net", &format!("adapter:{id}"), name, &on_val(*was_on), &on_val(*now_on))
        }
        ChangeKind::Dns { id, name, old, new } if old != new => crate::undo::note("net", &format!("dns:{id}"), &dns_label(name), &dns_val(old), &dns_val(new)),
        _ => {}
    }
}

fn net_err(e: &NetError) -> String {
    match e {
        NetError::NeedsAdmin | NetError::AccessDenied(_) => crate::admin::NOT_CHANGED.to_string(),
        e => e.to_string(),
    }
}

impl Network {
    /// The open page's service, or (closed: Settings › Reset, the uninstaller) one made now - the fake PC in a test copy
    /// (and in unit tests), Windows otherwise.
    fn rs_service(&self) -> Arc<NetworkService> {
        if let Some(s) = self.svc() {
            return s;
        }
        self.rs_svc
            .get_or_init(|| {
                if crate::testmode::on() || cfg!(test) {
                    Arc::new(NetworkService::new(Arc::new(FakeNet::drawing())))
                } else {
                    #[cfg(windows)]
                    {
                        Arc::new(NetworkService::new(Arc::new(crate::admin::proxy::NetOs::new(Arc::new(bu_network::real::WindowsNet::new()), crate::admin::client::admin()))))
                    }
                    #[cfg(not(windows))]
                    {
                        Arc::new(NetworkService::new(Arc::new(FakeNet::drawing())))
                    }
                }
            })
            .clone()
    }
}

// The reset's reads and writes, for the page itself and its detached copy (Order 047: `NetReset`, on the review's worker
// thread) alike: the service + the open page's last read of the adapters (None = closed).

fn rs_current(svc: &NetworkService, conn: Option<&ConnectionState>, item: &str) -> Option<Val> {
    if let Some(id) = item.strip_prefix("dns:") {
        // one IP Helper read
        return svc.os().dns_servers(id).ok().map(|s| dns_val(&s));
    }
    // an adapter's switch: the open page's last read (Windows' radio state is a slow read)
    let id = item.strip_prefix("adapter:")?;
    conn?.adapter(id).map(|a| on_val(a.enabled))
}

/// Every adapter switched off → On; every Ethernet / Wi-Fi adapter with hand-set DNS → Automatic (a VPN's own DNS is
/// the VPN's business).
fn rs_defaults(svc: &NetworkService, conn: Option<&ConnectionState>) -> Vec<DefaultItem> {
    let conn = match conn {
        Some(c) => Some(c.clone()),
        None => svc.connection_state().ok(),
    };
    let Some(conn) = conn else { return Vec::new() };
    let mut v = Vec::new();
    for a in &conn.adapters {
        v.push(DefaultItem { item: format!("adapter:{}", a.id), label: a.name.clone(), now: on_val(a.enabled), default: on_val(true) });
    }
    for a in conn.adapters.iter().filter(|a| a.kind.physical()) {
        if let Ok(s) = svc.os().dns_servers(&a.id) {
            v.push(DefaultItem { item: format!("dns:{}", a.id), label: dns_label(&a.name), now: dns_val(&s), default: dns_val(&DnsServers::default()) });
        }
    }
    v
}

fn rs_apply(svc: &NetworkService, item: &str, to: &Val) -> Result<(), String> {
    if crate::testmode::real_read() {
        return Err("A read-only test copy changes nothing".into());
    }
    if let Some(id) = item.strip_prefix("adapter:") {
        svc.set_adapter(id, to.raw == "on").map_err(|e| net_err(&e))?;
    } else if let Some(id) = item.strip_prefix("dns:") {
        let s = parse_dns(&to.raw).ok_or("Unknown value")?;
        svc.set_dns_on(id, &s).map_err(|e| net_err(&e))?;
    } else {
        return Err("Unknown setting".into());
    }
    Ok(())
}

impl Resettable for Network {
    fn page_id(&self) -> &str {
        "net"
    }
    fn page_title(&self) -> &str {
        "Network"
    }
    fn current(&self, item: &str) -> Option<Val> {
        rs_current(&self.rs_service(), self.conn.as_ref(), item)
    }
    fn windows_defaults(&self) -> Vec<DefaultItem> {
        rs_defaults(&self.rs_service(), self.conn.as_ref())
    }
    fn apply(&mut self, item: &str, to: &Val) -> Result<(), String> {
        rs_apply(&self.rs_service(), item, to)?;
        // the open page shows the new state
        self.reset_done();
        Ok(())
    }
    /// Order 047: the review reads (IP Helper, the adapters of a closed page) and puts back (an admin prompt) on its
    /// worker thread.
    fn detach(&mut self) -> Option<crate::undo::Detached> {
        Some(Box::new(NetReset { svc: self.rs_service(), conn: self.conn.clone() }))
    }
    fn reset_done(&mut self) {
        if self.svc.is_some() {
            self.read_all();
        }
    }
}

/// Order 047: the Network page's reset as a copy for the review's worker thread.
struct NetReset {
    svc: Arc<NetworkService>,
    conn: Option<ConnectionState>,
}

impl Resettable for NetReset {
    fn page_id(&self) -> &str {
        "net"
    }
    fn page_title(&self) -> &str {
        "Network"
    }
    fn current(&self, item: &str) -> Option<Val> {
        rs_current(&self.svc, self.conn.as_ref(), item)
    }
    fn windows_defaults(&self) -> Vec<DefaultItem> {
        rs_defaults(&self.svc, self.conn.as_ref())
    }
    fn apply(&mut self, item: &str, to: &Val) -> Result<(), String> {
        rs_apply(&self.svc, item, to)
    }
}
