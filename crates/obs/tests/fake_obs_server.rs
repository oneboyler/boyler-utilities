//! The whole feature against a FAKE OBS: a local obs-websocket v5 server on 127.0.0.1 (handshake, Hello with a password
//! challenge, Identify checked, requests answered, events pushed), the real service thread and WebSocket client, the fake
//! OS layer. No real OBS, no sound, no window.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

use bu_obs::engine::{KeysView, PopMsg, View};
use bu_obs::fake::FakeOs;
use bu_obs::ws::server::{read_frame, write_frame};
use bu_obs::{monitors, Options, Service, Settings, Ui};
use serde_json::{json, Value};

struct RecUi(Sender<String>);
impl Ui for RecUi {
    fn popup(&mut self, m: &PopMsg, _c: Option<usize>) {
        let _ = self.0.send(format!("POPUP {} | {}", m.main, m.top));
    }
    fn publish(&mut self, _v: &View) {}
    fn dialog(&mut self, t: &str) {
        let _ = self.0.send(format!("DIALOG {t}"));
    }
    fn keys(&mut self, _k: &KeysView) {}
    fn log(&mut self, l: &str) {
        let _ = self.0.send(format!("LOG {l}"));
    }
}

fn scratch(name: &str, port: u16, pw: &str) -> PathBuf {
    let d = std::env::temp_dir().join("bu-obs-test").join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("plugin_config/obs-websocket")).unwrap();
    std::fs::write(d.join("plugin_config/obs-websocket/config.json"), json!({"server_enabled": true, "server_port": port, "auth_required": true, "server_password": pw}).to_string()).unwrap();
    d
}

/// One fake OBS connection: handshake, Hello / Identify (password "obs-pass"), then answers; `events` are pushed as they
/// come. Returns the close code it sent (or 0).
fn fake_obs(l: TcpListener, events: Receiver<Value>) -> std::thread::JoinHandle<u16> {
    std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut r = BufReader::new(s.try_clone().unwrap());
        loop {
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
        }
        s.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Protocol: obswebsocket.json\r\n\r\n").unwrap();
        let hello = json!({"op": 0, "d": {"obsWebSocketVersion": "5.5.0", "rpcVersion": 1, "authentication": {"challenge": "ch4ll", "salt": "s4lt"}}});
        write_frame(&mut s, 1, hello.to_string().as_bytes()).unwrap();
        let (_, id) = read_frame(&mut r).unwrap();
        let id: Value = serde_json::from_slice(&id).unwrap();
        if id["d"]["authentication"] != bu_obs::auth::obs_auth("obs-pass", "s4lt", "ch4ll") {
            write_frame(&mut s, 8, &4009u16.to_be_bytes()).unwrap();
            return 4009;
        }
        write_frame(&mut s, 1, json!({"op": 2, "d": {"negotiatedRpcVersion": 1}}).to_string().as_bytes()).unwrap();
        s.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        r.get_ref().set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        let mut idle = 0;
        loop {
            while let Ok(ev) = events.try_recv() {
                if ev == json!("close") {
                    write_frame(&mut s, 8, &1000u16.to_be_bytes()).unwrap();
                    return 1000;
                }
                write_frame(&mut s, 1, json!({"op": 5, "d": {"eventType": ev["type"], "eventIntent": 1, "eventData": ev["data"]}}).to_string().as_bytes()).unwrap();
            }
            match read_frame(&mut r) {
                Some((1, p)) => {
                    idle = 0;
                    let m: Value = serde_json::from_slice(&p).unwrap();
                    let ty = m["d"]["requestType"].as_str().unwrap_or("").to_string();
                    let data = match ty.as_str() {
                        "GetReplayBufferStatus" => json!({"outputActive": true}),
                        "GetRecordStatus" => json!({"outputActive": false}),
                        "GetCurrentProgramScene" => json!({"sceneName": "Game"}),
                        "GetSceneList" => json!({"scenes": [{"sceneName": "Game", "sceneIndex": 0}]}),
                        "GetSceneItemList" => json!({"sceneItems": []}),
                        "GetProfileParameter" => json!({"parameterValue": if m["d"]["requestData"]["parameterName"] == "Mode" { "Simple" } else { "30" }}),
                        "GetVideoSettings" => json!({"fpsNumerator": 60, "fpsDenominator": 1}),
                        _ => json!({}),
                    };
                    let resp = json!({"op": 7, "d": {"requestType": ty, "requestId": m["d"]["requestId"], "requestStatus": {"result": true, "code": 100}, "responseData": data}});
                    write_frame(&mut s, 1, resp.to_string().as_bytes()).unwrap();
                }
                Some(_) => {}
                None => {
                    idle += 1;
                    if idle > 1500 {
                        return 0; // 30 s without anything: give up
                    }
                }
            }
        }
    })
}

fn wait_for(rx: &Receiver<String>, what: &str) -> Vec<String> {
    let mut seen = Vec::new();
    loop {
        match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(l) => {
                let hit = l.starts_with(what);
                seen.push(l);
                if hit {
                    return seen;
                }
            }
            Err(_) => panic!("never saw {what:?}; saw {seen:#?}"),
        }
    }
}

#[test]
fn connects_to_a_fake_obs_and_announces_a_saved_clip() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (etx, erx) = channel();
    let srv = fake_obs(l, erx);
    let dir = scratch("srv1", port, "obs-pass");
    let os = FakeOs::new(&dir);
    os.with(|s| s.running = true);
    let clip = dir.join("Replay 2026-10-08.mp4");
    std::fs::write(&clip, b"x").unwrap();
    os.with(|s| s.exists.insert(clip.clone()));
    let (tx, rx) = channel();
    let set = Settings { keep_rb: false, ..Settings::default() };
    let svc = Service::start(Box::new(os.clone()), set, monitors::fake("A,1920,1080,0,0,1"), Box::new(RecUi(tx)), Options { watch: false, exe_dir: None });
    wait_for(&rx, "POPUP OBS connected | Game \u{00B7} instant replay on");
    assert!(svc.view().connected);
    etx.send(json!({"type": "ReplayBufferSaved", "data": {"savedReplayPath": clip.to_string_lossy()}})).unwrap();
    wait_for(&rx, "POPUP Clipped last 30 seconds | Game");
    etx.send(json!("close")).unwrap();
    wait_for(&rx, "LOG DISCONNECTED code=1000");
    assert_eq!(srv.join().unwrap(), 1000);
    assert_eq!(os.with(|s| s.played.len()), 1, "one sound (to the fake speakers; the grey connected popup is silent)");
    drop(svc);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_wrong_password_is_reported_and_nothing_connects() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (_etx, erx) = channel();
    let srv = fake_obs(l, erx);
    let dir = scratch("srv2", port, "wrong");
    let os = FakeOs::new(&dir);
    os.with(|s| s.running = true);
    let (tx, rx) = channel();
    let svc = Service::start(Box::new(os), Settings::default(), monitors::fake("A,1920,1080,0,0,1"), Box::new(RecUi(tx)), Options { watch: false, exe_dir: None });
    wait_for(&rx, "LOG AUTH failed");
    assert_eq!(srv.join().unwrap(), 4009);
    assert!(!svc.view().connected);
    drop(svc);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stopping_the_service_ends_its_threads() {
    let dir = scratch("srv3", 1, "x");
    let os = FakeOs::new(&dir); // OBS not running: only the 5 s check would run
    let (tx, _rx) = channel();
    let svc = Service::start(Box::new(os), Settings::default(), vec![], Box::new(RecUi(tx)), Options { watch: false, exe_dir: None });
    let t = std::time::Instant::now();
    drop(svc);
    assert!(t.elapsed() < Duration::from_secs(1));
    let _ = TcpStream::connect_timeout(&"127.0.0.1:9".parse().unwrap(), Duration::from_millis(1));
    let _ = std::fs::remove_dir_all(&dir);
}
