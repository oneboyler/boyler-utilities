//! ClipPing's behaviour, scene by scene, on the engine with a pretend clock and a pretend OBS (fake OS layer; the
//! requests the engine sends are answered by the test). Nothing real is touched: no OBS, no sound, no window.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use bu_obs::engine::{Engine, Host, Input, KeyWhich, KeysView, ObsChange, PopMsg, View};
use bu_obs::fake::FakeOs;
use bu_obs::keys::KeyBind;
use bu_obs::monitors;
use bu_obs::os::ObsOs;
use bu_obs::settings::{SceneRow, Settings};
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
    retracts: usize,
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
    fn retract_clip_failed(&mut self) {
        self.retracts += 1;
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
    // his two scene collections (Order 132): "Untitled" (open, its file as OBS last saved it) and "PS5 4K60" (the Elgato)
    std::fs::create_dir_all(d.join("basic/scenes")).unwrap();
    std::fs::write(d.join("basic/scenes/Untitled.json"), UNTITLED_FILE.replace("M1", &M1.replace('\\', "\\\\")).replace("M2", &M2.replace('\\', "\\\\"))).unwrap();
    std::fs::write(d.join("basic/scenes/PS5_4K60.json"), PS5_FILE).unwrap();
    std::fs::write(d.join("basic/scenes/PS5_4K60.json.bak"), PS5_FILE).unwrap();
    d
}

const UNTITLED_FILE: &str = r#"{"name":"Untitled","current_scene":"16:9","scene_order":[{"name":"21:9"},{"name":"16:9"},{"name":"ps5"}],"sources":[
 {"id":"monitor_capture","name":"D1","settings":{"monitor_id":"M1"}},{"id":"monitor_capture","name":"D2","settings":{"monitor_id":"M2"}},
 {"id":"dshow_input","name":"Elgato","settings":{"resolution":"3840x2160","res_type":1}},
 {"id":"scene","name":"16:9","settings":{"items":[{"name":"D1","visible":true}]}},{"id":"scene","name":"21:9","settings":{"items":[{"name":"D2","visible":true}]}},
 {"id":"scene","name":"ps5","settings":{"items":[{"name":"Elgato","visible":true}]}}]}"#;
const PS5_FILE: &str = r#"{"name":"PS5 4K60","current_scene":"PS5","scene_order":[{"name":"PS5"}],"sources":[
 {"id":"dshow_input","name":"PS5 (Elgato 4K Pro)","settings":{"resolution":"3840x2160","res_type":1}},
 {"id":"scene","name":"PS5","settings":{"items":[{"name":"PS5 (Elgato 4K Pro)","visible":true}]}}]}"#;

/// GetSceneItemList's item for a capture device showing `w` x `h` (0 x 0 = no signal)
fn elgato_item(w: i32, hh: i32) -> Value {
    json!({"inputKind": "dshow_input", "sourceName": "Elgato", "sceneItemEnabled": true, "sceneItemTransform": {"sourceWidth": w, "sourceHeight": hh}})
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
            retracts: 0,
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
        self.connect_with(rb, false);
    }
    /// `ps5` = the open collection also has the scene "ps5" (an Elgato showing 3840 x 2160, no monitor)
    fn connect_with(&mut self, rb: bool, ps5: bool) {
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
        self.answer("GetSceneCollectionList", true, json!({"currentSceneCollectionName": "Untitled", "sceneCollections": ["PS5 4K60", "Untitled"]}));
        let mut scenes = vec![json!({"sceneName": "21:9", "sceneIndex": 1}), json!({"sceneName": "16:9", "sceneIndex": 2})];
        if ps5 {
            scenes.push(json!({"sceneName": "ps5", "sceneIndex": 0}));
        }
        self.answer("GetSceneList", true, json!({ "scenes": scenes }));
        if ps5 {
            self.answer_items("ps5", vec![elgato_item(3840, 2160)]);
        }
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
    /// answer the GetSceneItemList of this scene
    fn answer_items(&mut self, scene: &str, items: Vec<Value>) {
        let req = self.pending("GetSceneItemList").into_iter().find(|r| r["d"]["requestData"]["sceneName"] == scene).unwrap_or_else(|| panic!("no GetSceneItemList {scene}"));
        let id = req["d"]["requestId"].as_str().unwrap().to_string();
        self.h.answered.insert(id.clone());
        self.msg(json!({"op": 7, "d": {"requestType": "GetSceneItemList", "requestId": id, "requestStatus": {"result": true, "code": 100}, "responseData": {"sceneItems": items}}}));
    }
    /// answer with a failure code (not 500, which a video change retries)
    fn fail(&mut self, ty: &str, code: i64) {
        let req = self.pending(ty).into_iter().next().unwrap_or_else(|| panic!("no {ty} request"));
        let id = req["d"]["requestId"].as_str().unwrap().to_string();
        self.h.answered.insert(id.clone());
        self.msg(json!({"op": 7, "d": {"requestType": ty, "requestId": id, "requestStatus": {"result": false, "code": code}}}));
    }
    /// the request types sent since `n` (requests only)
    fn sent_since(&self, n: usize) -> Vec<String> {
        self.h.sent[n..].iter().filter(|m| m["op"] == 6).map(|m| m["d"]["requestType"].as_str().unwrap().to_string()).collect()
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
    assert_eq!(t.h.view.list.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["16:9", "21:9"]);
    assert_eq!(t.h.view.colls[0].scenes.iter().map(|s| (s.name.as_str(), s.mon)).collect::<Vec<_>>(), vec![("16:9", 1), ("21:9", 2)]);
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
    t.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false, mods: 0 });
    t.input(Input::Key { which: KeyWhich::Clip, down: false, mouse: false, mods: 0 });
    t.wait(300);
    t.event("ReplayBufferSaved", json!({"savedReplayPath": clip.to_string_lossy()}));
    let p = t.last_popup().clone();
    assert_eq!((p.color, p.main.as_str(), p.top.as_str(), p.detail.as_str()), (Color::Green, "Clipped last 12 seconds", "Monitor 1", "12 s"));
    assert_eq!(*t.h.sounds.last().unwrap(), Sound::Saved);
    // no save at all after a press: "Clip failed" only after the long wait (a press right after a save is OBS's own announcement: skipped)
    t.wait(500);
    t.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false, mods: 0 });
    t.wait(2100);
    assert_eq!(t.last_popup().main, "Clipped last 12 seconds", "a slow save is not a failure");
    t.wait(27_000);
    assert_eq!(t.last_popup().main, "Clipped last 12 seconds");
    t.wait(1_000);
    assert_eq!(t.last_popup().main, "Clip failed");
    assert_eq!(t.last_popup().top, "OBS couldn't save the file");
}

/// Order 091: a save that takes longer than 2 s is not a failure (no "Clip failed" before the "Clipped").
#[test]
fn a_slow_save_shows_only_clipped() {
    let mut t = T::new("slowclip", no_keep());
    t.connect(true);
    t.event("ReplayBufferStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    t.wait(12_000);
    let clip = t.dir.join("slow.mp4");
    std::fs::write(&clip, b"x").unwrap();
    t.h.os.with(|s| {
        s.exists.insert(clip.clone());
    });
    let n = t.h.popups.len();
    t.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false, mods: 0 });
    t.input(Input::Key { which: KeyWhich::Clip, down: false, mouse: false, mods: 0 });
    t.wait(5_000);
    t.event("ReplayBufferSaved", json!({"savedReplayPath": clip.to_string_lossy()}));
    let got = t.popups_since(n);
    assert!(got.len() == 1 && got[0].0.starts_with("Clipped last") && got[0].1 == "Monitor 1", "{got:?}");
    assert_eq!(t.h.retracts, 0);
    t.wait(40_000);
    assert_eq!(t.popups_since(n).len(), 1, "no failure popup later");
}

/// Order 091: when "Clip failed" was already shown, a late "saved" takes it away and shows "Clipped".
#[test]
fn a_late_saved_replaces_a_shown_clip_failed() {
    let mut t = T::new("lateclip", no_keep());
    t.connect(true);
    t.event("ReplayBufferStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    t.wait(12_000);
    let clip = t.dir.join("late.mp4");
    std::fs::write(&clip, b"x").unwrap();
    t.h.os.with(|s| {
        s.exists.insert(clip.clone());
    });
    t.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false, mods: 0 });
    t.input(Input::Key { which: KeyWhich::Clip, down: false, mouse: false, mods: 0 });
    t.wait(31_000);
    assert_eq!(t.last_popup().main, "Clip failed");
    assert_eq!(t.h.retracts, 0);
    t.event("ReplayBufferSaved", json!({"savedReplayPath": clip.to_string_lossy()}));
    assert_eq!(t.h.retracts, 1, "the red popup is taken away");
    assert!(t.last_popup().main.starts_with("Clipped last"));
    // a later, normal save does not retract anything
    t.wait(5_000);
    t.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false, mods: 0 });
    t.input(Input::Key { which: KeyWhich::Clip, down: false, mouse: false, mods: 0 });
    t.event("ReplayBufferSaved", json!({"savedReplayPath": clip.to_string_lossy()}));
    assert_eq!(t.h.retracts, 1);
}

#[test]
fn replay_off_first_press_warns_second_press_turns_it_on_after_the_key_is_let_go() {
    let mut t = T::new("ra", no_keep());
    t.connect(false);
    t.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false, mods: 0 });
    t.input(Input::Key { which: KeyWhich::Clip, down: false, mouse: false, mods: 0 });
    assert_eq!(t.last_popup().main, "Nothing was saved");
    assert_eq!(t.last_popup().top, "Instant replay is off \u{00B7} press again to turn it on");
    t.wait(1000);
    t.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false, mods: 0 });
    t.wait(2000); // held: nothing yet
    assert!(!t.has_sent("StartReplayBuffer"));
    t.input(Input::Key { which: KeyWhich::Clip, down: false, mouse: false, mods: 0 });
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
    t.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false, mods: 0 });
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

/// the owner's profile (Order 131): Advanced, NVENC HEVC CBR 200 000 kbit/s, 5 audio tracks x 320 = 201 600 kbit/s
fn ps5_profile(t: &T) {
    let p = t.dir.join("basic/profiles/P");
    std::fs::write(
        p.join("basic.ini"),
        "[General]\r\nName=P\r\n\r\n[Output]\r\nMode=Advanced\r\n\r\n[AdvOut]\r\nRecType=Standard\r\nRecFilePath=C:/Users/user/Videos/obs\r\nRecFormat2=hybrid_mp4\r\nRecTracks=31\r\nRecEncoder=obs_nvenc_hevc_tex\r\nTrack1Bitrate=320\r\nTrack2Bitrate=320\r\nTrack3Bitrate=320\r\nTrack4Bitrate=320\r\nTrack5Bitrate=320\r\nTrack6Bitrate=320\r\nRecRBTime=60\r\n\r\n[Video]\r\nFPSType=1\r\nFPSInt=120\r\n",
    )
    .unwrap();
    std::fs::write(p.join("recordEncoder.json"), "{\"rate_control\":\"CBR\",\"bitrate\":200000}").unwrap();
}

const GB: u64 = 1024 * 1024 * 1024;
/// bytes per second at 201 600 kbit/s
const PS5_BPS: u64 = 201_600 * 125;

fn rec_status(bytes: u64, dur_ms: u64) -> Value {
    json!({"outputActive": true, "outputPaused": false, "outputBytes": bytes, "outputDuration": dur_ms})
}

fn storage_warnings(t: &T) -> usize {
    t.h.popups.iter().filter(|p| p.main == "Storage almost full").count()
}

#[test]
fn earlier_recordings_bytes_dont_shrink_minutes_left() {
    // His Nov 5 session: OBS's Hybrid MP4 output keeps counting outputBytes across recordings (two earlier ones today,
    // 23 104 215 + 1 681 003 KiB in OBS's log) while outputDuration restarts. The old rate = all those bytes / this
    // recording's minute = ~3.6 Gbit/s = "About 20 minutes of recording left" with 506 GB free.
    let mut t = T::new("ps5", no_keep());
    ps5_profile(&t);
    t.connect(true);
    t.h.os.with(|s| s.free = Some(506 * GB));
    let earlier: u64 = (23_104_215 + 1_681_003) * 1024;
    t.event("RecordStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    t.answer("GetRecordStatus", true, rec_status(earlier, 0));
    for min in 1..=5u64 {
        t.wait(60_000);
        t.answer("GetRecordStatus", true, rec_status(earlier + min * 60 * PS5_BPS, min * 60_000));
        let m = t.e.minutes_left(&mut t.h).unwrap();
        assert!(m >= 330, "minute {min}: {m} minutes left");
    }
    assert_eq!(storage_warnings(&t), 0);
    assert_eq!(t.e.minutes_left(&mut t.h), Some((506 * GB / PS5_BPS / 60) as i64));
}

#[test]
fn the_rate_is_measured_from_this_recordings_first_status() {
    // the same earlier bytes, but OBS really writes 1.5x the profile: that measured rate is used (not the profile's)
    let mut t = T::new("ps5meas", no_keep());
    ps5_profile(&t);
    t.connect(true);
    t.h.os.with(|s| s.free = Some(506 * GB));
    let earlier: u64 = (23_104_215 + 1_681_003) * 1024;
    t.event("RecordStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    t.answer("GetRecordStatus", true, rec_status(earlier, 0));
    t.wait(60_000);
    t.answer("GetRecordStatus", true, rec_status(earlier + 60 * PS5_BPS * 3 / 2, 60_000));
    assert_eq!(t.e.minutes_left(&mut t.h), Some((506 * GB / (PS5_BPS * 3 / 2) / 60) as i64));
    assert_eq!(storage_warnings(&t), 0);
}

#[test]
fn reconnecting_mid_recording_measures_from_the_first_status() {
    // the app (re)connects while he records: the first GetRecordStatus is the starting point, not a rate
    let mut t = T::new("ps5re", no_keep());
    ps5_profile(&t);
    t.h.os.with(|s| s.free = Some(506 * GB));
    t.h.os.with(|s| s.running = true);
    t.e.start(&mut t.h);
    let g = t.h.gen;
    t.input(Input::Ws(WsEvent::Open(g)));
    t.msg(json!({"op": 0, "d": {"obsWebSocketVersion": "5.7.3", "rpcVersion": 1, "authentication": {"challenge": "c", "salt": "s"}}}));
    t.msg(json!({"op": 2, "d": {"negotiatedRpcVersion": 1}}));
    // OBS writes 1.5x the profile here, so the measured rate shows in the answer
    let bps = PS5_BPS * 3 / 2;
    let earlier = 30 * GB;
    t.answer("GetRecordStatus", true, rec_status(earlier + 120 * bps, 120_000));
    t.wait(60_000);
    t.answer("GetRecordStatus", true, rec_status(earlier + 180 * bps, 180_000));
    assert_eq!(t.e.minutes_left(&mut t.h), Some((506 * GB / bps / 60) as i64), "measured from the connect's status");
    assert_eq!(storage_warnings(&t), 0);
}

#[test]
fn an_unbelievable_measured_rate_falls_back_to_the_profile() {
    let mut t = T::new("ps5odd", no_keep());
    ps5_profile(&t);
    t.connect(true);
    t.h.os.with(|s| s.free = Some(506 * GB));
    let profile = Some((506 * GB / PS5_BPS / 60) as i64);
    t.event("RecordStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    t.answer("GetRecordStatus", true, rec_status(0, 0));
    // 5x the profile's bitrate in the first minute: not believed
    t.wait(60_000);
    t.answer("GetRecordStatus", true, rec_status(5 * 60 * PS5_BPS, 60_000));
    assert_eq!(t.e.minutes_left(&mut t.h), profile);
    // a tenth of it: not believed either
    let mut t2 = T::new("ps5odd2", no_keep());
    ps5_profile(&t2);
    t2.connect(true);
    t2.h.os.with(|s| s.free = Some(506 * GB));
    t2.event("RecordStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    t2.answer("GetRecordStatus", true, rec_status(0, 0));
    t2.wait(60_000);
    t2.answer("GetRecordStatus", true, rec_status(6 * PS5_BPS, 60_000));
    assert_eq!(t2.e.minutes_left(&mut t2.h), profile);
    // within range (1.5x): used
    t2.wait(60_000);
    t2.answer("GetRecordStatus", true, rec_status(180 * PS5_BPS, 120_000));
    assert_eq!(t2.e.minutes_left(&mut t2.h), Some((506 * GB / (PS5_BPS * 3 / 2) / 60) as i64));
    assert_eq!(storage_warnings(&t) + storage_warnings(&t2), 0);
}

#[test]
fn low_space_warns_at_once_on_the_profile_rate_and_needs_two_low_measurements() {
    // 20 GB free at 201 600 kbit/s = ~14 minutes: warns as the recording starts
    let mut t = T::new("ps5low", no_keep());
    ps5_profile(&t);
    t.connect(true);
    t.h.os.with(|s| s.free = Some(20 * GB));
    t.event("RecordStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    assert_eq!(storage_warnings(&t), 1);
    let left = 20 * GB / PS5_BPS / 60;
    assert_eq!(t.last_popup().top, format!("About {left} minutes of recording left"));
    t.answer("GetRecordStatus", true, rec_status(0, 0));
    t.wait(60_000);
    t.answer("GetRecordStatus", true, rec_status(60 * PS5_BPS, 60_000));
    assert_eq!(storage_warnings(&t), 1, "warned once");

    // 80 GB free = ~56 minutes on the profile; OBS really writes 3x that (in range): one low reading doesn't warn,
    // the second in a row does
    let mut t = T::new("ps5low2", no_keep());
    ps5_profile(&t);
    t.connect(true);
    t.h.os.with(|s| s.free = Some(80 * GB));
    t.event("RecordStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    t.answer("GetRecordStatus", true, rec_status(0, 0));
    assert_eq!(storage_warnings(&t), 0);
    t.wait(60_000);
    t.answer("GetRecordStatus", true, rec_status(3 * 60 * PS5_BPS, 60_000));
    assert_eq!(storage_warnings(&t), 0, "one low sample never warns");
    // instant replay starting in between is no new sample
    t.event("ReplayBufferStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    assert_eq!(storage_warnings(&t), 0, "still one low sample");
    t.wait(60_000);
    t.answer("GetRecordStatus", true, rec_status(3 * 120 * PS5_BPS, 120_000));
    assert_eq!(storage_warnings(&t), 1);
    let left = 80 * GB / (PS5_BPS * 3) / 60;
    assert_eq!(t.last_popup().top, format!("About {left} minutes of recording left"));
}

// ---------------------------------------------------------------- Order 132: the Save clip key, one press = one clip

/// OBS 32.1.2's own hotkey handling for one binding (libobs/obs-hotkey.c `handle_binding` / `inject_hotkey` /
/// `press_released_binding`), with the strict modifiers OBS turns on (libobs/obs.c:1162): the hotkey thread polls the key
/// with GetAsyncKeyState every 25 ms (obs-windows.c `vk_down`, obs-hotkey.c `obs_hotkey_thread`); a binding fires only when
/// the held modifiers EQUAL its own; obs-websocket's TriggerHotkeyByKeySequence injects (false, true, false). A fire =
/// one "save" (obs-ffmpeg-mux.c `replay_buffer_hotkey`).
struct ObsHotkey {
    key: &'static str,
    mods: u8,
    modifiers_match: bool,
    pressed: bool,
    /// obs_hotkey_t.pressed (counts the hotkey's pressed bindings)
    hotkey_pressed: i32,
    saves: u32,
}

impl ObsHotkey {
    fn new(key: &'static str, mods: u8) -> Self {
        ObsHotkey { key, mods, modifiers_match: true, pressed: false, hotkey_pressed: 0, saves: 0 }
    }
    fn press(&mut self) {
        self.pressed = true;
        self.hotkey_pressed += 1;
        if self.hotkey_pressed == 1 {
            self.saves += 1;
        }
    }
    fn release(&mut self) {
        self.pressed = false;
        self.hotkey_pressed -= 1;
    }
    /// one hotkey-thread poll: `down` = the key is down now, `held` = the modifiers down now
    fn poll(&mut self, down: bool, held: u8) {
        let m = held == self.mods; // strict
        if !self.modifiers_match || !m || !down {
            self.modifiers_match = m;
            if self.pressed {
                self.release();
            }
            return;
        }
        if !self.pressed {
            self.press();
        }
    }
    /// TriggerHotkeyByKeySequence (keyId, keyModifiers)
    fn inject(&mut self, req: &Value) {
        let d = &req["d"]["requestData"];
        let km = &d["keyModifiers"];
        let mut mods = 0u8;
        for (k, b) in [("shift", bu_obs::keys::MOD_S), ("control", bu_obs::keys::MOD_C), ("alt", bu_obs::keys::MOD_A), ("command", bu_obs::keys::MOD_W)] {
            if km[k] == true {
                mods |= b;
            }
        }
        if mods != self.mods || d["keyId"] != self.key {
            return;
        }
        // (false, true, false): only the "true" can press
        self.modifiers_match = true;
        if !self.pressed {
            self.press();
        }
    }
}

/// One press of OBS's Save clip key (Insert) as the owner makes it: `held` modifiers down the whole time, the key down at `at`
/// ms for `hold` ms; OBS's hotkey thread polls at `phase` + every 25 ms; the app's Raw Input sees the press at once. Returns
/// how many saves OBS made.
fn press_insert(t: &mut T, obs: &mut ObsHotkey, held: u8, at: u64, hold: u64, phase: u64) -> u32 {
    let before = obs.saves;
    let mut sent = t.h.sent.len();
    let t0 = t.h.now;
    let end = at + hold + 100;
    let mut poll = phase;
    let mut ms = 0;
    while ms <= end {
        if ms == at {
            t.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false, mods: held });
        }
        if ms == at + hold {
            t.input(Input::Key { which: KeyWhich::Clip, down: false, mouse: false, mods: 0 });
        }
        // what the app asked OBS (through obs-websocket) reaches OBS right away
        while sent < t.h.sent.len() {
            let m = t.h.sent[sent].clone();
            sent += 1;
            if m["d"]["requestType"] == "TriggerHotkeyByKeySequence" {
                obs.inject(&m);
            }
            if m["d"]["requestType"] == "SaveReplayBuffer" {
                obs.saves += 1;
            }
        }
        if ms == poll {
            obs.poll(ms >= at && ms < at + hold, held);
            poll += 25;
        }
        ms += 1;
        t.h.now = t0 + ms;
    }
    obs.saves - before
}

fn insert_profile(t: &T) {
    let p = t.dir.join("basic/profiles/P/basic.ini");
    let ini = std::fs::read_to_string(&p).unwrap().replace("OBS_KEY_F9", "OBS_KEY_INSERT");
    std::fs::write(p, ini).unwrap();
}

#[test]
fn a_save_clip_press_with_ctrl_or_shift_held_still_clips_once() {
    // his Save clip key = Insert (no modifiers); he crouches on Ctrl / runs on Shift while pressing it
    let mut t = T::new("ctrlclip", no_keep());
    insert_profile(&t);
    t.connect(true);
    t.event("ReplayBufferStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    t.wait(5_000);
    let mut obs = ObsHotkey::new("OBS_KEY_INSERT", 0);
    use bu_obs::keys::{MOD_C, MOD_S};
    let saved = press_insert(&mut t, &mut obs, MOD_C, 10, 90, 3);
    // what he sees: OBS announces the save it made (or nothing) - never the red "Clip failed"
    if saved > 0 {
        let clip = t.dir.join("ctrl.mp4");
        std::fs::write(&clip, b"x").unwrap();
        t.h.os.with(|s| {
            s.exists.insert(clip.clone());
        });
        t.event("ReplayBufferSaved", json!({"savedReplayPath": clip.to_string_lossy()}));
    }
    t.wait(31_000);
    assert!(t.h.popups.iter().all(|p| p.main != "Clip failed"), "his symptom: {:?}", t.popups_since(0));
    assert_eq!(saved, 1, "Insert while Ctrl is held");
    t.wait(5_000);
    assert_eq!(press_insert(&mut t, &mut obs, MOD_S | MOD_C, 10, 120, 17), 1, "Insert while Ctrl + Shift are held");
}

#[test]
fn a_press_obs_takes_itself_is_left_to_obs() {
    // no extra modifier held: OBS saves it on its own, so nothing is sent (a late press from here could save twice)
    let mut t = T::new("tapclip", no_keep());
    insert_profile(&t);
    t.connect(true);
    t.event("ReplayBufferStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    t.wait(5_000);
    let mut obs = ObsHotkey::new("OBS_KEY_INSERT", 0);
    let n = t.h.sent.len();
    assert_eq!(press_insert(&mut t, &mut obs, 0, 30, 80, 7), 1);
    assert!(!t.sent_since(n).iter().any(|r| r == "TriggerHotkeyByKeySequence"));
}

#[test]
fn a_ctrl_press_right_after_a_save_still_clips() {
    // review: a press within 400 ms of OBS's "saved" (the earlier clip) is still pressed in OBS
    let mut t = T::new("ctrlsoon", no_keep());
    insert_profile(&t);
    t.connect(true);
    t.event("ReplayBufferStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    t.wait(5_000);
    t.event("ReplayBufferSaved", json!({"savedReplayPath": "C:/earlier.mp4"}));
    let mut obs = ObsHotkey::new("OBS_KEY_INSERT", 0);
    assert_eq!(press_insert(&mut t, &mut obs, bu_obs::keys::MOD_C, 50, 80, 3), 1);
}

#[test]
fn a_normal_press_is_one_clip_never_two() {
    let mut t = T::new("oneclip", no_keep());
    insert_profile(&t);
    t.connect(true);
    t.event("ReplayBufferStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    t.wait(5_000);
    let mut obs = ObsHotkey::new("OBS_KEY_INSERT", 0);
    // every phase of OBS's poll against the press (OBS's own poll first, or the app's press first), short and long holds
    for phase in 0..25 {
        for hold in [26, 60, 150, 600] {
            t.wait(3_000);
            assert_eq!(press_insert(&mut t, &mut obs, 0, 10, hold, phase), 1, "phase {phase} hold {hold}");
        }
    }
}

#[test]
fn switch_scene_changes_resolution_and_restarts_instant_replay() {
    let mut t = T::new("sw", no_keep());
    t.connect(true);
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false, mods: 0 });
    t.answer("GetRecordStatus", true, json!({"outputActive": false}));
    t.answer("GetVideoSettings", true, video(2560, 1440, 60));
    t.answer("GetReplayBufferStatus", true, json!({"outputActive": true}));
    t.answer("StopReplayBuffer", true, json!({}));
    t.event("ReplayBufferStateChanged", json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED"}));
    let r = t.answer("SetCurrentProgramScene", true, json!({}));
    assert_eq!(r["d"]["requestData"]["sceneName"], "21:9");
    // Order 132: the size and the FPS in one change (a row without its own FPS keeps OBS's)
    let r = t.answer("SetVideoSettings", true, json!({}));
    assert_eq!(r["d"]["requestData"], json!({"baseWidth": 1920, "baseHeight": 1080, "outputWidth": 1920, "outputHeight": 1080, "fpsNumerator": 60, "fpsDenominator": 1}));
    t.wait(501);
    t.answer("StartReplayBuffer", true, json!({}));
    let n = t.h.popups.len();
    t.event("ReplayBufferStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    t.event("CurrentProgramSceneChanged", json!({"sceneName": "21:9"}));
    assert_eq!(t.popups_since(n), vec![("Now clipping 21:9".to_string(), "1920 x 1080 \u{00B7} 60 fps \u{00B7} instant replay on".to_string())]);
    assert_eq!(t.h.view.clipped, Some(1));
}

fn video(w: i32, hh: i32, fps: i32) -> Value {
    json!({"baseWidth": w, "baseHeight": hh, "outputWidth": w, "outputHeight": hh, "fpsNumerator": fps, "fpsDenominator": 1})
}

fn row(name: &str, coll: &str, res: Option<(i32, i32)>, fps: i32) -> SceneRow {
    SceneRow { name: name.into(), coll: coll.into(), res, fps }
}

fn with_rows(rows: Vec<SceneRow>) -> Settings {
    Settings { scenes: rows, scenes_init: true, ..no_keep() }
}

const DOT: &str = " \u{00B7} ";

/// Order 132 (4): nothing differs (the row's size and FPS = OBS's now): only the scene, instant replay is not touched
#[test]
fn switching_to_a_row_with_the_same_video_only_sets_the_scene() {
    let mut t = T::new("swsame", with_rows(vec![row("16:9", "Untitled", None, 0), row("21:9", "Untitled", Some((2560, 1440)), 6000)]));
    t.connect(true);
    let n = t.h.sent.len();
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false, mods: 0 });
    t.answer("GetRecordStatus", true, json!({"outputActive": false}));
    t.answer("GetVideoSettings", true, video(2560, 1440, 60));
    t.answer("SetCurrentProgramScene", true, json!({}));
    assert_eq!(t.sent_since(n), vec!["GetRecordStatus", "GetVideoSettings", "SetCurrentProgramScene"]);
    assert_eq!(t.last_popup().main, "Now clipping 21:9");
    assert_eq!(t.last_popup().top, format!("2560 x 1440{DOT}60 fps{DOT}instant replay on"));
    assert!(t.h.view.replay);
}

/// Order 132 (4): only the FPS differs: instant replay restarts around one video change with the row's FPS
#[test]
fn a_row_with_its_own_fps_changes_it_with_the_size_in_one_go() {
    let mut t = T::new("swfps", with_rows(vec![row("16:9", "Untitled", None, 0), row("21:9", "Untitled", Some((2560, 1440)), 12000)]));
    t.connect(true);
    let n = t.h.sent.len();
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false, mods: 0 });
    t.answer("GetRecordStatus", true, json!({"outputActive": false}));
    t.answer("GetVideoSettings", true, video(2560, 1440, 60));
    t.answer("GetReplayBufferStatus", true, json!({"outputActive": true}));
    t.answer("StopReplayBuffer", true, json!({}));
    t.event("ReplayBufferStateChanged", json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED"}));
    t.answer("SetCurrentProgramScene", true, json!({}));
    let r = t.answer("SetVideoSettings", true, json!({}));
    assert_eq!(r["d"]["requestData"], json!({"baseWidth": 2560, "baseHeight": 1440, "outputWidth": 2560, "outputHeight": 1440, "fpsNumerator": 120, "fpsDenominator": 1}));
    t.wait(501);
    t.answer("StartReplayBuffer", true, json!({}));
    t.event("ReplayBufferStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    assert_eq!(
        t.sent_since(n),
        vec!["GetRecordStatus", "GetVideoSettings", "GetReplayBufferStatus", "StopReplayBuffer", "SetCurrentProgramScene", "SetVideoSettings", "StartReplayBuffer"]
    );
    assert_eq!(t.last_popup().top, format!("2560 x 1440{DOT}120 fps{DOT}instant replay on"));
    assert_eq!(t.h.view.fps, 12000);
}

/// Order 132 (2): Auto on a scene with no monitor = its video source's size as OBS reports it; no signal = the size stays
#[test]
fn auto_on_a_capture_card_scene_takes_the_size_obs_reports() {
    let mut t = T::new("swcap", with_rows(vec![row("16:9", "Untitled", None, 0), row("ps5", "Untitled", None, 6000)]));
    t.connect_with(true, true);
    // the page: Auto means 3840 x 2160 for ps5 (OBS's report), the monitor's size for 16:9
    let live = &t.h.view.colls[0];
    assert_eq!(live.name, "Untitled");
    assert_eq!(live.scenes.iter().map(|s| (s.name.as_str(), s.mon, s.auto)).collect::<Vec<_>>(), vec![("16:9", 1, (2560, 1440)), ("21:9", 2, (1920, 1080)), ("ps5", 0, (3840, 2160))]);
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false, mods: 0 });
    t.answer("GetRecordStatus", true, json!({"outputActive": false}));
    t.answer("GetVideoSettings", true, video(2560, 1440, 60));
    t.answer_items("ps5", vec![elgato_item(3840, 2160), json!({"inputKind": "image_source", "sceneItemEnabled": true, "sceneItemTransform": {"sourceWidth": 400, "sourceHeight": 300}})]);
    t.answer("GetReplayBufferStatus", true, json!({"outputActive": true}));
    t.answer("StopReplayBuffer", true, json!({}));
    t.event("ReplayBufferStateChanged", json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED"}));
    t.answer("SetCurrentProgramScene", true, json!({}));
    let r = t.answer("SetVideoSettings", true, json!({}));
    assert_eq!(r["d"]["requestData"], json!({"baseWidth": 3840, "baseHeight": 2160, "outputWidth": 3840, "outputHeight": 2160, "fpsNumerator": 60, "fpsDenominator": 1}));

    // no signal: OBS reports 0 x 0 - the size stays (same FPS too: just the scene)
    let mut t = T::new("swcap0", with_rows(vec![row("16:9", "Untitled", None, 0), row("ps5", "Untitled", None, 0)]));
    t.connect_with(true, true);
    let n = t.h.sent.len();
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false, mods: 0 });
    t.answer("GetRecordStatus", true, json!({"outputActive": false}));
    t.answer("GetVideoSettings", true, video(2560, 1440, 60));
    t.answer_items("ps5", vec![elgato_item(0, 0)]);
    t.answer("SetCurrentProgramScene", true, json!({}));
    assert_eq!(t.sent_since(n), vec!["GetRecordStatus", "GetVideoSettings", "GetSceneItemList", "SetCurrentProgramScene"]);
    assert_eq!(t.last_popup().top, format!("2560 x 1440{DOT}60 fps{DOT}instant replay on"));

    // review: a size left as it is keeps OBS's output scaling (base 2560 x 1440, output 1920 x 1080); only the row's FPS changes
    let mut t = T::new("swcap1", with_rows(vec![row("16:9", "Untitled", None, 0), row("ps5", "Untitled", None, 12000)]));
    t.connect_with(true, true);
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false, mods: 0 });
    t.answer("GetRecordStatus", true, json!({"outputActive": false}));
    t.answer("GetVideoSettings", true, json!({"baseWidth": 2560, "baseHeight": 1440, "outputWidth": 1920, "outputHeight": 1080, "fpsNumerator": 60, "fpsDenominator": 1}));
    t.answer_items("ps5", vec![elgato_item(0, 0), json!({"inputKind": "browser_source", "sceneItemEnabled": true, "sceneItemTransform": {"sourceWidth": 3840, "sourceHeight": 2160}})]);
    t.answer("GetReplayBufferStatus", true, json!({"outputActive": true}));
    t.answer("StopReplayBuffer", true, json!({}));
    t.event("ReplayBufferStateChanged", json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED"}));
    t.answer("SetCurrentProgramScene", true, json!({}));
    let r = t.answer("SetVideoSettings", true, json!({}));
    assert_eq!(r["d"]["requestData"], json!({"baseWidth": 2560, "baseHeight": 1440, "outputWidth": 1920, "outputHeight": 1080, "fpsNumerator": 120, "fpsDenominator": 1}));
}

/// Order 132 (3 + 4): a row in another scene collection: the collection first, then the scene, then its size (OBS's
/// report of the Elgato) and FPS in one change; instant replay restarted around it
#[test]
fn a_row_in_another_collection_opens_it_then_sets_the_scene_size_and_fps() {
    let mut t = T::new("swcoll", with_rows(vec![row("16:9", "Untitled", None, 0), row("PS5", "PS5 4K60", None, 6000)]));
    t.connect(true);
    // "+ Add scene" lists every collection: the open one (OBS's), then the others (their files; backups skipped)
    assert_eq!(t.h.view.coll, "Untitled");
    assert_eq!(t.h.view.colls.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["Untitled", "PS5 4K60"]);
    let ps5 = &t.h.view.colls[1].scenes[0];
    assert_eq!((ps5.name.as_str(), ps5.mon, ps5.auto), ("PS5", 0, (3840, 2160)), "Auto from the device's own resolution until OBS reports it");
    let n = t.h.sent.len();
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false, mods: 0 });
    t.answer("GetRecordStatus", true, json!({"outputActive": false}));
    t.answer("GetVideoSettings", true, video(3440, 1440, 120));
    t.answer("GetReplayBufferStatus", true, json!({"outputActive": true}));
    t.answer("StopReplayBuffer", true, json!({}));
    t.event("ReplayBufferStateChanged", json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED"}));
    let r = t.answer("SetCurrentSceneCollection", true, json!({}));
    assert_eq!(r["d"]["requestData"], json!({"sceneCollectionName": "PS5 4K60"}));
    t.event("CurrentSceneCollectionChanged", json!({"sceneCollectionName": "PS5 4K60"}));
    let r = t.answer("SetCurrentProgramScene", true, json!({}));
    assert_eq!(r["d"]["requestData"]["sceneName"], "PS5");
    // review: a capture device in a collection that just opened reports 0 x 0 until its first frame (an overlay's size never
    // counts): asked again every 500 ms
    t.answer_items("PS5", vec![elgato_item(0, 0), json!({"inputKind": "image_source", "sceneItemEnabled": true, "sceneItemTransform": {"sourceWidth": 400, "sourceHeight": 300}})]);
    assert!(t.pending("SetVideoSettings").is_empty());
    t.wait(501);
    t.answer_items("PS5", vec![elgato_item(3840, 2160)]);
    let r = t.answer("SetVideoSettings", true, json!({}));
    assert_eq!(r["d"]["requestData"], json!({"baseWidth": 3840, "baseHeight": 2160, "outputWidth": 3840, "outputHeight": 2160, "fpsNumerator": 60, "fpsDenominator": 1}));
    t.wait(501);
    t.answer("StartReplayBuffer", true, json!({}));
    let p = t.h.popups.len();
    t.event("ReplayBufferStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    assert_eq!(
        t.sent_since(n).into_iter().filter(|r| r != "GetSceneList").collect::<Vec<_>>(),
        vec![
            "GetRecordStatus",
            "GetVideoSettings",
            "GetReplayBufferStatus",
            "StopReplayBuffer",
            "SetCurrentSceneCollection",
            "SetCurrentProgramScene",
            "GetSceneItemList",
            "GetSceneItemList",
            "SetVideoSettings",
            "StartReplayBuffer"
        ]
    );
    // the drawing's popup (panel 4)
    assert_eq!(t.popups_since(p), vec![("Now clipping PS5".to_string(), format!("3840 x 2160{DOT}60 fps{DOT}instant replay on"))]);
    assert_eq!(t.last_popup().icon, Icon::Switch);
    assert_eq!(t.h.view.coll, "PS5 4K60");

    // the same key again: back to 16:9 in Untitled (its monitor's size, OBS's FPS since the row has none: 60 now)
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false, mods: 0 });
    t.answer("GetRecordStatus", true, json!({"outputActive": false}));
    t.answer("GetVideoSettings", true, video(3840, 2160, 60));
    t.answer("GetReplayBufferStatus", true, json!({"outputActive": true}));
    t.answer("StopReplayBuffer", true, json!({}));
    t.event("ReplayBufferStateChanged", json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED"}));
    let r = t.answer("SetCurrentSceneCollection", true, json!({}));
    assert_eq!(r["d"]["requestData"], json!({"sceneCollectionName": "Untitled"}));
    let r = t.answer("SetCurrentProgramScene", true, json!({}));
    assert_eq!(r["d"]["requestData"]["sceneName"], "16:9");
    let r = t.answer("SetVideoSettings", true, json!({}));
    assert_eq!(r["d"]["requestData"], json!({"baseWidth": 2560, "baseHeight": 1440, "outputWidth": 2560, "outputHeight": 1440, "fpsNumerator": 60, "fpsDenominator": 1}));
}

/// Order 132 (4): a step fails -> back to the collection, scene, size and FPS it came from (one request after another)
/// + the red popup; instant replay back on
#[test]
fn a_failed_switch_goes_back_to_where_it_came_from() {
    // the scene can't be set in the other collection
    let mut t = T::new("swback", with_rows(vec![row("16:9", "Untitled", None, 0), row("PS5", "PS5 4K60", None, 6000)]));
    t.connect(true);
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false, mods: 0 });
    t.answer("GetRecordStatus", true, json!({"outputActive": false}));
    t.answer("GetVideoSettings", true, video(2560, 1440, 120));
    t.answer("GetReplayBufferStatus", true, json!({"outputActive": true}));
    t.answer("StopReplayBuffer", true, json!({}));
    t.event("ReplayBufferStateChanged", json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED"}));
    t.answer("SetCurrentSceneCollection", true, json!({}));
    let n = t.h.sent.len();
    t.fail("SetCurrentProgramScene", 600);
    assert_eq!((t.last_popup().color, t.last_popup().main.as_str()), (Color::Red, "Couldn't switch"));
    // one at a time: the collection back first ...
    assert_eq!(t.sent_since(n), vec!["SetCurrentSceneCollection"]);
    let r = t.answer("SetCurrentSceneCollection", true, json!({}));
    assert_eq!(r["d"]["requestData"], json!({"sceneCollectionName": "Untitled"}));
    // review: a new switch press while a roll-back request is out does nothing
    let m = t.h.sent.len();
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false, mods: 0 });
    assert!(t.sent_since(m).is_empty());
    // ... then its scene, then instant replay back on (the video was never changed)
    let r = t.answer("SetCurrentProgramScene", true, json!({}));
    assert_eq!(r["d"]["requestData"]["sceneName"], "16:9");
    assert_eq!(t.sent_since(n), vec!["SetCurrentSceneCollection", "SetCurrentProgramScene", "StartReplayBuffer"]);
    assert_eq!(t.h.view.coll, "Untitled");

    // instant replay never comes back on after the video change: collection, scene AND video go back
    let mut t = T::new("swback2", with_rows(vec![row("16:9", "Untitled", None, 0), row("PS5", "PS5 4K60", Some((3840, 2160)), 6000)]));
    t.connect(true);
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false, mods: 0 });
    t.answer("GetRecordStatus", true, json!({"outputActive": false}));
    t.answer("GetVideoSettings", true, video(2560, 1440, 120));
    t.answer("GetReplayBufferStatus", true, json!({"outputActive": true}));
    t.answer("StopReplayBuffer", true, json!({}));
    t.event("ReplayBufferStateChanged", json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED"}));
    t.answer("SetCurrentSceneCollection", true, json!({}));
    t.answer("SetCurrentProgramScene", true, json!({}));
    t.answer("SetVideoSettings", true, json!({}));
    let n = t.h.sent.len();
    // OBS ignores every "start instant replay" for 40 s
    for _ in 0..42 {
        t.wait(1000);
        while !t.pending("GetReplayBufferStatus").is_empty() {
            t.answer("GetReplayBufferStatus", true, json!({"outputActive": false}));
        }
    }
    assert_eq!(t.last_popup().main, "Couldn't switch");
    let back: Vec<String> = t.sent_since(n).into_iter().filter(|r| r.starts_with("Set")).collect();
    assert_eq!(back, vec!["SetCurrentSceneCollection"]);
    t.answer("SetCurrentSceneCollection", true, json!({}));
    t.answer("SetCurrentProgramScene", true, json!({}));
    let r = t.answer("SetVideoSettings", true, json!({}));
    assert_eq!(r["d"]["requestData"], video(2560, 1440, 120));
    assert_eq!(t.h.view.fps, 12000);

    // OBS doesn't answer at all while instant replay starts: the switch times out, goes back, and a late start of instant
    // replay is not a "Now clipping"
    let mut t = T::new("swback4", with_rows(vec![row("16:9", "Untitled", None, 0), row("PS5", "PS5 4K60", Some((3840, 2160)), 6000)]));
    t.connect(true);
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false, mods: 0 });
    t.answer("GetRecordStatus", true, json!({"outputActive": false}));
    t.answer("GetVideoSettings", true, video(2560, 1440, 120));
    t.answer("GetReplayBufferStatus", true, json!({"outputActive": true}));
    t.answer("StopReplayBuffer", true, json!({}));
    t.event("ReplayBufferStateChanged", json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED"}));
    t.answer("SetCurrentSceneCollection", true, json!({}));
    t.answer("SetCurrentProgramScene", true, json!({}));
    t.answer("SetVideoSettings", true, json!({}));
    t.wait(51_000);
    assert_eq!(t.last_popup().main, "Couldn't switch");
    t.event("ReplayBufferStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    assert!(t.h.popups.iter().all(|p| p.main != "Now clipping PS5"), "{:?}", t.popups_since(0));

    // the video change refused: no video to undo, the scene back
    let mut t = T::new("swback3", with_rows(vec![row("16:9", "Untitled", None, 0), row("21:9", "Untitled", None, 0)]));
    t.connect(true);
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false, mods: 0 });
    t.answer("GetRecordStatus", true, json!({"outputActive": false}));
    t.answer("GetVideoSettings", true, video(2560, 1440, 60));
    t.answer("GetReplayBufferStatus", true, json!({"outputActive": true}));
    t.answer("StopReplayBuffer", true, json!({}));
    t.event("ReplayBufferStateChanged", json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED"}));
    t.answer("SetCurrentProgramScene", true, json!({}));
    let n = t.h.sent.len();
    t.fail("SetVideoSettings", 600);
    let r = t.answer("SetCurrentProgramScene", true, json!({}));
    assert_eq!(r["d"]["requestData"]["sceneName"], "16:9");
    assert_eq!(t.sent_since(n), vec!["SetCurrentProgramScene", "StartReplayBuffer"]);
    assert_eq!(t.last_popup().main, "Couldn't switch");
}

/// Order 132 (5): his old list ("16:9", "21:9" without a collection, size or FPS) works as before - Auto (the monitor's
/// size) and OBS's FPS; a row from before Order 132 whose scene is only in another collection is found there
#[test]
fn an_old_list_switches_as_before_and_finds_scenes_in_other_collections() {
    let old = vec![SceneRow::new("16:9", ""), SceneRow::new("21:9", ""), SceneRow::new("PS5", "")];
    let mut t = T::new("swold", with_rows(old));
    t.connect(true);
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false, mods: 0 });
    t.answer("GetRecordStatus", true, json!({"outputActive": false}));
    t.answer("GetVideoSettings", true, video(2560, 1440, 120));
    t.answer("GetReplayBufferStatus", true, json!({"outputActive": true}));
    t.answer("StopReplayBuffer", true, json!({}));
    t.event("ReplayBufferStateChanged", json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED"}));
    assert!(!t.has_sent("SetCurrentSceneCollection"));
    let r = t.answer("SetCurrentProgramScene", true, json!({}));
    assert_eq!(r["d"]["requestData"]["sceneName"], "21:9");
    let r = t.answer("SetVideoSettings", true, json!({}));
    assert_eq!(r["d"]["requestData"], video(1920, 1080, 120), "Auto + the FPS OBS has now");
    t.wait(501);
    t.answer("StartReplayBuffer", true, json!({}));
    t.event("ReplayBufferStateChanged", json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}));
    assert_eq!(t.last_popup().main, "Now clipping 21:9");
    // next: "PS5" - only in "PS5 4K60"
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false, mods: 0 });
    t.answer("GetRecordStatus", true, json!({"outputActive": false}));
    t.answer("GetVideoSettings", true, video(1920, 1080, 120));
    t.answer("GetReplayBufferStatus", true, json!({"outputActive": true}));
    t.answer("StopReplayBuffer", true, json!({}));
    t.event("ReplayBufferStateChanged", json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED"}));
    let r = t.answer("SetCurrentSceneCollection", true, json!({}));
    assert_eq!(r["d"]["requestData"], json!({"sceneCollectionName": "PS5 4K60"}));
}

#[test]
fn switching_while_recording_is_refused() {
    let mut t = T::new("swrec", no_keep());
    t.connect(true);
    t.input(Input::Key { which: KeyWhich::Switch, down: true, mouse: false, mods: 0 });
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
    t.input(Input::Key { which: KeyWhich::Replay, down: true, mouse: false, mods: 0 });
    assert_eq!((t.last_popup().main.as_str(), t.last_popup().top.as_str()), ("OBS isn't open", "Press again to start it"));
    t.wait(1000);
    t.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false, mods: 0 });
    let started = t.h.os.with(|s| s.started.clone());
    assert_eq!(started, vec![(PathBuf::from("C:\\fake\\obs-studio\\bin\\64bit\\obs64.exe"), Some("--minimize-to-tray --startreplaybuffer".to_string()), true)]);
    // more than 5 s later the first press counts again
    let mut t2 = T::new("la2", no_keep());
    t2.h.os.with(|s| s.exists.insert(PathBuf::from("C:\\fake\\obs-studio\\bin\\64bit\\obs64.exe")));
    t2.e.start(&mut t2.h);
    t2.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false, mods: 0 });
    t2.wait(5100);
    t2.input(Input::Key { which: KeyWhich::Clip, down: true, mouse: false, mods: 0 });
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
