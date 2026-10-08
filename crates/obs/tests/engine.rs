//! ClipPing's behaviour, scene by scene, on the engine with a pretend clock and a pretend OBS (fake OS layer; the
//! requests the engine sends are answered by the test). Nothing real is touched: no OBS, no sound, no window.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use bu_obs::engine::{Engine, Host, Input, KeyWhich, KeysView, ObsChange, PopMsg, View};
use bu_obs::fake::FakeOs;
use bu_obs::keys::KeyBind;
use bu_obs::monitors;
use bu_obs::os::ObsOs;
use bu_obs::settings::Settings;
use bu_obs::sound::Sound;
use bu_obs::ws::WsEvent;
use bu_obs::{Color, Icon};
use serde_json::{json, Value};

const M1: &str = "\\\\?\\DISPLAY#DEL1#5&a&0&UID1#{x}";
const M2: &str = "\\\\?\\DISPLAY#LG2#5&b&0&UID2#{x}";

struct TH {
    now: u64,
    os: FakeOs,
    sent: Vec<Value>,
    answered: HashSet<String>,
    popups: Vec<PopMsg>,
    sounds: Vec<Sound>,
    dialogs: Vec<String>,
    view: View,
    keys: Vec<KeysView>,
    gen: u32,
    busy: bool,
    aborted: bool,
    exits: Vec<u32>,
    log: Vec<String>,
}

impl Host for TH {
    fn now_ms(&self) -> u64 {
        self.now
    }
    fn os(&mut self) -> &mut dyn ObsOs {
        &mut self.os
    }
    fn ws_start(&mut self, _port: u16) -> u32 {
        if self.busy {
            return 0;
        }
        self.busy = true;
        self.gen += 1;
        self.gen
    }
    fn ws_busy(&self) -> bool {
        self.busy
    }
    fn ws_send(&mut self, text: &str) {
        self.sent.push(serde_json::from_str(text).unwrap());
    }
    fn ws_abort(&mut self) {
        self.aborted = true;
    }
    fn popup(&mut self, m: &PopMsg, _clipped: Option<usize>) {
        self.popups.push(m.clone());
    }
    fn sound(&mut self, ev: Sound, _set: &Settings) {
        self.sounds.push(ev);
    }
    fn publish(&mut self, v: &View) {
        self.view = v.clone();
    }
    fn dialog(&mut self, text: &str) {
        self.dialogs.push(text.into());
    }
    fn wait_exit(&mut self, pid: u32, _timeout_ms: u32) {
        self.exits.push(pid);
    }
    fn watch(&mut self, _dirs: Vec<PathBuf>) {}
    fn keys(&mut self, k: &KeysView) {
        self.keys.push(k.clone());
    }
    fn log(&mut self, line: &str) {
        self.log.push(line.into());
    }
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("bu-obs-test").join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("plugin_config/obs-websocket")).unwrap();
    std::fs::create_dir_all(d.join("basic/profiles/P")).unwrap();
    std::fs::write(d.join("plugin_config/obs-websocket/config.json"), "{\n    \"server_enabled\": true,\n    \"server_port\": 4455,\n    \"auth_required\": true,\n    \"server_password\": \"secret123\"\n}\n").unwrap();
    std::fs::write(d.join("user.ini"), "[Basic]\r\nProfileDir=P\r\n").unwrap();
    std::fs::write(
        d.join("basic/profiles/P/basic.ini"),
        "[General]\r\nName=P\r\n\r\n[SimpleOutput]\r\nRecRBTime=60\r\nFilePath=C:/Clips\r\n\r\n[Video]\r\nFPSType=1\r\nFPSInt=60\r\n\r\n[Hotkeys]\r\nReplayBuffer={\"ReplayBuffer.Save\":[{\"key\":\"OBS_KEY_F9\"}]}\r\n",
    )
    .unwrap();
    d
}

struct T {
    e: Engine,
    h: TH,
    dir: PathBuf,
}

impl T {
    fn new(name: &str, set: Settings) -> T {
        let dir = scratch(name);
        let os = FakeOs::new(&dir);
        let mons = monitors::fake(&format!("{M1},2560,1440,0,0,1 {M2},1920,1080,2560,0,0"));
        let h = TH {
            now: 1_000_000,
            os,
            sent: vec![],
            answered: HashSet::new(),
            popups: vec![],
            sounds: vec![],
            dialogs: vec![],
            view: View::default(),
            keys: vec![],
            gen: 0,
            busy: false,
            aborted: false,
            exits: vec![],
            log: vec![],
        };
        T { e: Engine::new(set, mons), h, dir }
    }
    fn input(&mut self, i: Input) {
        self.e.input(&mut self.h, i);
    }
    /// let pretend time pass, firing timers in order
    fn wait(&mut self, ms: u64) {
        let end = self.h.now + ms;
        while let Some(d) = self.e.next_due().filter(|d| *d <= end) {
            self.h.now = self.h.now.max(d);
            self.e.fire_timers(&mut self.h);
        }
        self.h.now = end;
    }
    fn msg(&mut self, v: Value) {
        let g = self.h.gen;
        self.input(Input::Ws(WsEvent::Msg(g, v.to_string())));
    }
    fn pending(&self, ty: &str) -> Vec<Value> {
        self.h.sent.iter().filter(|m| m["op"] == 6 && m["d"]["requestType"] == ty && !self.h.answered.contains(m["d"]["requestId"].as_str().unwrap())).cloned().collect()
    }
    fn has_sent(&self, ty: &str) -> bool {
        self.h.sent.iter().any(|m| m["op"] == 6 && m["d"]["requestType"] == ty)
    }
    /// answer the oldest unanswered request of this type
    fn answer(&mut self, ty: &str, ok: bool, data: Value) -> Value {
        let req = self.pending(ty).into_iter().next().unwrap_or_else(|| panic!("no {ty} request; sent: {:?}", self.h.sent.iter().map(|m| m["d"]["requestType"].clone()).collect::<Vec<_>>()));
        let id = req["d"]["requestId"].as_str().unwrap().to_string();
        self.h.answered.insert(id.clone());
        let code = if ok { 100 } else { 500 };
        self.msg(json!({"op": 7, "d": {"requestType": ty, "requestId": id, "requestStatus": {"result": ok, "code": code}, "responseData": data}}));
        req
    }
    fn event(&mut self, ty: &str, data: Value) {
        self.msg(json!({"op": 5, "d": {"eventType": ty, "eventIntent": 1, "eventData": data}}));
    }
    /// OBS open, connect, answer every first request; replay buffer on / off as given
    fn connect(&mut self, rb: bool) {
        self.h.os.with(|s| s.running = true);
        self.e.start(&mut self.h);
        let g = self.h.gen;
        assert!(g > 0, "connect started");
        self.input(Input::Ws(WsEvent::Open(g)));
        self.msg(json!({"op": 0, "d": {"obsWebSocketVersion": "5.5.0", "rpcVersion": 1, "authentication": {"challenge": "c", "salt": "s"}}}));
        let id = self.h.sent.last().unwrap().clone();
        assert_eq!(id["op"], 1);
        assert_eq!(id["d"]["eventSubscriptions"], 207);
        assert_eq!(id["d"]["authentication"], bu_obs::auth::obs_auth("secret123", "s", "c"));
        self.msg(json!({"op": 2, "d": {"negotiatedRpcVersion": 1}}));
        self.answer("GetVersion", true, json!({"obsVersion": "31.0.0", "obsWebSocketVersion": "5.5.0"}));
        self.answer("GetRecordDirectory", true, json!({"recordDirectory": "C:/Clips"}));
        self.answer("GetProfileParameter", true, json!({"parameterValue": "Simple"}));
        self.answer("GetProfileParameter", true, json!({"parameterValue": "60"}));
        self.answer("GetReplayBufferStatus", true, json!({"outputActive": rb}));
        self.answer("GetRecordStatus", true, json!({"outputActive": false}));
        self.answer("GetCurrentProgramScene", true, json!({"sceneName": "16:9"}));
        self.answer("GetVideoSettings", true, json!({"fpsNumerator": 60, "fpsDenominator": 1, "baseWidth": 2560, "baseHeight": 1440, "outputWidth": 2560, "outputHeight": 1440}));
        self.answer("GetSceneList", true, json!({"scenes": [{"sceneName": "21:9", "sceneIndex": 0}, {"sceneName": "16:9", "sceneIndex": 1}]}));
        for (s, src) in [("16:9", "D1"), ("21:9", "D2")] {
            let req = self.pending("GetSceneItemList").into_iter().find(|r| r["d"]["requestData"]["sceneName"] == s).unwrap();
            let id = req["d"]["requestId"].as_str().unwrap().to_string();
            self.h.answered.insert(id.clone());
            self.msg(json!({"op": 7, "d": {"requestType": "GetSceneItemList", "requestId": id, "requestStatus": {"result": true, "code": 100},
                "responseData": {"sceneItems": [{"inputKind": "monitor_capture", "sourceName": src, "sceneItemEnabled": true}]}}}));
        }
        for (src, mon) in [("D1", M1), ("D2", M2)] {
            let req = self.pending("GetInputSettings").into_iter().find(|r| r["d"]["requestData"]["inputName"] == src).unwrap();
            let id = req["d"]["requestId"].as_str().unwrap().to_string();
            self.h.answered.insert(id.clone());
            self.msg(json!({"op": 7, "d": {"requestType": "GetInputSettings", "requestId": id, "requestStatus": {"result": true, "code": 100},
                "responseData": {"inputSettings": {"monitor_id": mon}}}}));
        }
    }
    fn last_popup(&self) -> &PopMsg {
        self.h.popups.last().expect("a popup")
    }
    fn popups_since(&self, n: usize) -> Vec<(String, String)> {
        self.h.popups[n..].iter().map(|p| (p.main.clone(), p.top.clone())).collect()
    }
}

impl Drop for T {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn no_keep() -> Settings {
    Settings { keep_rb: false, ..Settings::default() }
}

#[test]
fn connects_with_auth_greets_once_and_makes_the_default_scene_list() {
    let mut t = T::new("greet", no_keep());
    t.connect(true);
    let p = t.last_popup();
    assert_eq!((p.color, p.icon, p.main.as_str(), p.top.as_str()), (Color::Grey, Icon::Plug, "OBS connected", "Monitor 1 \u{00B7} instant replay on"));
    assert!(t.h.sounds.is_empty(), "grey popups are silent");
    assert!(t.h.view.connected && t.h.view.replay);
    assert_eq!(t.h.view.tip, "Clipping monitor 1 \u{00B7} instant replay on");
    assert_eq!(t.h.view.list, vec!["16:9".to_string(), "21:9".to_string()]);
    assert_eq!(t.h.view.scenes.iter().map(|s| (s.name.as_str(), s.mon)).collect::<Vec<_>>(), vec![("16:9", 1), ("21:9", 2)]);
    assert_eq!((t.h.view.cliplen, t.h.view.fps), (60, 6000));
    assert_eq!(t.h.keys.last().unwrap().clip, vec![KeyBind::new(0x78, 0)]);
}

#[test]
fn keep_instant_replay_on_starts_it_on_connect_without_a_popup_of_its_own() {
    let mut t = T::new("keep", Settings::default());
    t.connect(false);
    t.answer("StartReplayBuffer", true, json!({}));
    let n = t.h.popups.len();
    t.event("ReplayBufferStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    // Connect purpose: the normal "Instant replay on" popup shows
    assert_eq!(t.popups_since(n), vec![("Instant replay on".to_string(), "Monitor 1 \u{00B7} 60 seconds".to_string())]);
    assert_eq!(*t.h.sounds.last().unwrap(), Sound::Changed);
}

#[test]
fn save_clip_says_clipped_last_n_seconds() {
    let mut t = T::new("clip", no_keep());
    t.connect(true);
    t.event("ReplayBufferStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    t.wait(12_000);
    let clip = t.dir.join("clip.mp4");
    std::fs::write(&clip, b"x").unwrap();
    t.h.os.with(|s| {
        s.exists.insert(clip.clone());
    });
    t.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false });
    t.input(Input::Key { which: KeyWhich::Clip, down: false, mouse: false });
    t.wait(300);
    t.event("ReplayBufferSaved", json!({"savedReplayPath": clip.to_string_lossy()}));
    let p = t.last_popup().clone();
    assert_eq!((p.color, p.main.as_str(), p.top.as_str(), p.detail.as_str()), (Color::Green, "Clipped last 12 seconds", "Monitor 1", "12 s"));
    assert_eq!(*t.h.sounds.last().unwrap(), Sound::Saved);
    // no save within 2 s of a press: "Clip failed" (a press right after a save is OBS's own announcement: skipped)
    t.wait(500);
    t.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false });
    t.wait(2100);
    assert_eq!(t.last_popup().main, "Clip failed");
    assert_eq!(t.last_popup().top, "OBS couldn't save the file");
}

#[test]
fn replay_off_first_press_warns_second_press_turns_it_on_after_the_key_is_let_go() {
    let mut t = T::new("ra", no_keep());
    t.connect(false);
    t.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false });
    t.input(Input::Key { which: KeyWhich::Clip, down: false, mouse: false });
    assert_eq!(t.last_popup().main, "Nothing was saved");
    assert_eq!(t.last_popup().top, "Instant replay is off \u{00B7} press again to turn it on");
    t.wait(1000);
    t.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false });
    t.wait(2000); // held: nothing yet
    assert!(!t.has_sent("StartReplayBuffer"));
    t.input(Input::Key { which: KeyWhich::Clip, down: false, mouse: false });
    t.wait(299);
    assert!(!t.has_sent("StartReplayBuffer"), "waits 300 ms after the release");
    t.wait(2);
    assert!(t.has_sent("StartReplayBuffer"));
    t.answer("StartReplayBuffer", true, json!({}));
    let n = t.h.popups.len();
    t.event("ReplayBufferStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    assert_eq!(t.popups_since(n), vec![("Instant replay on".to_string(), "Monitor 1 \u{00B7} 60 seconds".to_string())]);
    // OBS's own save of that same press (a ~1 s clip) gets no popup
    let n = t.h.popups.len();
    t.event("ReplayBufferSaved", json!({"savedReplayPath": "C:/x.mp4"}));
    assert_eq!(t.h.popups.len(), n);
}

#[test]
fn recording_pauses_instant_replay_and_brings_it_back() {
    let mut t = T::new("rec", Settings::default());
    t.connect(true);
    t.h.os.with(|s| s.free = Some(500 * 1024 * 1024 * 1024));
    let n = t.h.popups.len();
    t.event("RecordStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    assert!(t.has_sent("StopReplayBuffer"));
    t.event("ReplayBufferStateChanged", json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED"}));
    assert_eq!(t.popups_since(n), vec![("Recording".to_string(), "Monitor 1 \u{00B7} instant replay paused".to_string())]);
    // a Save clip press while recording: grey, silent
    let s = t.h.sounds.len();
    t.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false });
    assert_eq!(t.last_popup().main, "It's all in the recording");
    assert_eq!(t.h.sounds.len(), s);
    t.wait(65 * 60 * 1000 + 4000);
    t.answer("GetRecordStatus", true, json!({"outputActive": true, "outputBytes": 0, "outputDuration": 1}));
    let n = t.h.popups.len();
    t.event("RecordStateChanged", json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED"}));
    assert_eq!(t.popups_since(n), vec![("Recording saved".to_string(), "Monitor 1 \u{00B7} 1:05:04 \u{00B7} instant replay on".to_string())]);
    assert!(t.pending("StartReplayBuffer").len() == 1, "instant replay turned back on");
}

#[test]
fn storage_almost_full_warns_once() {
    let mut t = T::new("sto", no_keep());
    t.connect(true);
    // 2660 kbit/s (OBS's defaults 2500 + 160) = 332 500 bytes/s; 0.5 GB = ~26 minutes
    t.h.os.with(|s| s.free = Some(512 * 1024 * 1024));
    t.event("RecordStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    let warn: Vec<&PopMsg> = t.h.popups.iter().filter(|p| p.main == "Storage almost full").collect();
    assert_eq!(warn.len(), 1);
    let left = 512u64 * 1024 * 1024 / (2660 * 125) / 60;
    assert_eq!(warn[0].top, format!("About {left} minutes of recording left"));
    assert_eq!(warn[0].color, Color::Amber);
}

#[test]
fn switch_scene_changes_resolution_and_restarts_instant_replay() {
    let mut t = T::new("sw", no_keep());
    t.connect(true);
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false });
    t.answer("GetRecordStatus", true, json!({"outputActive": false}));
    t.answer("GetVideoSettings", true, json!({"baseWidth": 2560, "baseHeight": 1440, "outputWidth": 2560, "outputHeight": 1440}));
    t.answer("GetReplayBufferStatus", true, json!({"outputActive": true}));
    t.answer("StopReplayBuffer", true, json!({}));
    t.event("ReplayBufferStateChanged", json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED"}));
    let r = t.answer("SetCurrentProgramScene", true, json!({}));
    assert_eq!(r["d"]["requestData"]["sceneName"], "21:9");
    let r = t.answer("SetVideoSettings", true, json!({}));
    assert_eq!(r["d"]["requestData"], json!({"baseWidth": 1920, "baseHeight": 1080, "outputWidth": 1920, "outputHeight": 1080}));
    t.wait(501);
    t.answer("StartReplayBuffer", true, json!({}));
    let n = t.h.popups.len();
    t.event("ReplayBufferStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    t.event("CurrentProgramSceneChanged", json!({"sceneName": "21:9"}));
    assert_eq!(t.popups_since(n), vec![("Now clipping 21:9".to_string(), "Monitor 2 \u{00B7} instant replay on".to_string())]);
    assert_eq!(t.h.view.clipped, Some(1));
}

#[test]
fn switching_while_recording_is_refused() {
    let mut t = T::new("swrec", no_keep());
    t.connect(true);
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false });
    t.answer("GetRecordStatus", true, json!({"outputActive": true}));
    assert_eq!(t.last_popup().main, "Can't switch while recording");
}

#[test]
fn obs_closed_first_press_says_so_second_press_starts_it() {
    let mut t = T::new("la", no_keep());
    t.h.os.with(|s| {
        s.running = false;
        s.exists.insert(PathBuf::from("C:\\fake\\obs-studio\\bin\\64bit\\obs64.exe"));
    });
    t.e.start(&mut t.h);
    t.input(Input::Key { which: KeyWhich::Replay, down: true, mouse: false });
    assert_eq!((t.last_popup().main.as_str(), t.last_popup().top.as_str()), ("OBS isn't open", "Press again to start it"));
    t.wait(1000);
    t.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false });
    let started = t.h.os.with(|s| s.started.clone());
    assert_eq!(started, vec![(PathBuf::from("C:\\fake\\obs-studio\\bin\\64bit\\obs64.exe"), Some("--minimize-to-tray --startreplaybuffer".to_string()), true)]);
    // more than 5 s later the first press counts again
    let mut t2 = T::new("la2", no_keep());
    t2.h.os.with(|s| s.exists.insert(PathBuf::from("C:\\fake\\obs-studio\\bin\\64bit\\obs64.exe")));
    t2.e.start(&mut t2.h);
    t2.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false });
    t2.wait(5100);
    t2.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false });
    assert!(t2.h.os.with(|s| s.started.is_empty()));
    assert_eq!(t2.last_popup().top, "OBS isn't open \u{00B7} press again to start it");
}

#[test]
fn applying_new_keys_closes_obs_writes_the_profile_and_reopens_it() {
    let mut t = T::new("oa", no_keep());
    t.connect(false);
    let c = ObsChange { keys_changed: true, key: [Some(KeyBind::new(0x77, 1)), None, Some(KeyBind::new(0x13, 0))], ..Default::default() };
    t.input(Input::Apply(c));
    t.answer("GetRecordStatus", true, json!({"outputActive": false}));
    assert_eq!(t.h.os.with(|s| s.closed.clone()), vec![4242]);
    assert_eq!(t.h.exits, vec![4242]);
    t.input(Input::ObsExited(true));
    let ini = std::fs::read_to_string(t.dir.join("basic/profiles/P/basic.ini")).unwrap();
    assert!(ini.contains("ReplayBuffer={\"ReplayBuffer.Save\":[{\"key\":\"OBS_KEY_F8\",\"control\":true}]}\r\n"), "{ini}");
    assert!(ini.contains("OBSBasic.StartReplayBuffer={\"bindings\":[]}\r\nOBSBasic.StopReplayBuffer={\"bindings\":[]}\r\n"));
    assert!(ini.contains("OBSBasic.StartRecording={\"bindings\":[{\"key\":\"OBS_KEY_PAUSE\"}]}\r\nOBSBasic.StopRecording={\"bindings\":[{\"key\":\"OBS_KEY_PAUSE\"}]}"));
    assert!(ini.starts_with("[General]\r\nName=P\r\n\r\n[SimpleOutput]\r\nRecRBTime=60\r\n"), "the rest stays as it was");
    assert_eq!(t.h.os.with(|s| s.started.len()), 1, "OBS reopened");
    assert_eq!(t.h.keys.last().unwrap().clip, vec![KeyBind::new(0x77, 1)]);
}

#[test]
fn applying_a_clip_length_live_restarts_instant_replay_quietly() {
    let mut t = T::new("live", no_keep());
    t.connect(true);
    let n = t.h.popups.len();
    t.input(Input::Apply(ObsChange { cliplen_changed: true, cliplen: 90, ..Default::default() }));
    let r = t.answer("SetProfileParameter", true, json!({}));
    assert_eq!(r["d"]["requestData"], json!({"parameterCategory": "SimpleOutput", "parameterName": "RecRBTime", "parameterValue": "90"}));
    t.answer("StopReplayBuffer", true, json!({}));
    t.event("ReplayBufferStateChanged", json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED"}));
    t.answer("StartReplayBuffer", true, json!({}));
    t.event("ReplayBufferStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    assert_eq!(t.h.popups.len(), n, "the app's own restart is quiet");
    assert_eq!(t.h.view.cliplen, 90);
    assert!(!t.h.view.applying);
}

#[test]
fn remote_control_off_with_obs_closed_is_turned_on_with_a_new_password() {
    let mut t = T::new("ws", no_keep());
    std::fs::write(t.dir.join("plugin_config/obs-websocket/config.json"), "{\n    \"server_enabled\": false,\n    \"server_port\": 4455,\n    \"auth_required\": false,\n    \"server_password\": \"\"\n}\n").unwrap();
    t.e.start(&mut t.h);
    let cfg = std::fs::read_to_string(t.dir.join("plugin_config/obs-websocket/config.json")).unwrap();
    let v: Value = serde_json::from_str(&cfg).unwrap();
    assert_eq!(v["server_enabled"], true);
    assert_eq!(v["auth_required"], true);
    assert_eq!(v["server_password"].as_str().unwrap().len(), 32);
    assert_eq!(t.last_popup().main, "OBS remote control turned on");
}

#[test]
fn remote_control_off_with_obs_open_asks_and_connect_closes_and_reopens_obs() {
    let mut t = T::new("ask", no_keep());
    std::fs::write(t.dir.join("plugin_config/obs-websocket/config.json"), "{\"server_enabled\": false, \"server_password\": \"longenough1\"}").unwrap();
    t.h.os.with(|s| s.running = true);
    t.e.start(&mut t.h);
    assert!(t.h.view.ask_connect);
    assert_eq!(t.last_popup().main, "Turn on OBS remote control");
    t.input(Input::Connect(true));
    assert_eq!(t.h.os.with(|s| s.closed.clone()), vec![4242]);
    t.input(Input::ObsExited(true));
    let v: Value = serde_json::from_str(&std::fs::read_to_string(t.dir.join("plugin_config/obs-websocket/config.json")).unwrap()).unwrap();
    assert_eq!((v["server_enabled"].clone(), v["server_password"].clone()), (json!(true), json!("longenough1")), "an 8+ character password is kept");
    assert_eq!(t.h.os.with(|s| s.started.len()), 1);
}

#[test]
fn obs_closing_while_recording_says_recording_stopped() {
    let mut t = T::new("close", no_keep());
    t.connect(true);
    t.event("RecordStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    let g = t.h.gen;
    t.h.busy = false;
    t.input(Input::Ws(WsEvent::Closed(g, 1000)));
    assert_eq!((t.last_popup().main.as_str(), t.last_popup().detail.as_str()), ("Recording stopped", "OBS closed"));
    assert!(!t.h.view.connected);
    assert!(t.e.timer_on(bu_obs::engine::T::Retry));
}

#[test]
fn start_obs_with_windows_tick() {
    let mut t = T::new("so", Settings { start_obs: false, keep_rb: false, ..Settings::default() });
    t.e.start(&mut t.h);
    t.input(Input::StartObsToggle);
    assert_eq!(t.h.dialogs, vec!["Open OBS once, then tick this again.".to_string()]);
    t.h.os.with(|s| s.exists.insert(PathBuf::from("C:\\fake\\obs-studio\\bin\\64bit\\obs64.exe")));
    t.input(Input::StartObsToggle);
    assert!(t.h.view.start_obs_shown);
    assert_eq!(t.h.os.with(|s| s.shortcut.clone()), Some(PathBuf::from("C:\\fake\\obs-studio\\bin\\64bit\\obs64.exe")));
    t.input(Input::StartObsToggle);
    assert!(!t.h.view.start_obs_shown && t.h.os.with(|s| s.shortcut.is_none()));
    t.h.os.with(|s| s.user_autostart = true);
    t.input(Input::StartObsToggle);
    assert!(t.h.dialogs.last().unwrap().starts_with("OBS starts with Windows through a shortcut you made"));
}

#[test]
fn a_fake_only_writes_inside_its_own_folder() {
    let mut os = FakeOs::new(Path::new("C:\\nowhere-test"));
    assert!(!os.write_file(Path::new("C:\\Users\\x\\AppData\\Roaming\\obs-studio\\global.ini"), "x"));
}
