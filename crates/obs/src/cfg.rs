//! OBS's own settings files, read-only (obscfg.c): the WebSocket server (port, password), the active profile folder, the
//! Save clip / instant replay / recording keys, clip length, FPS, recording folder and an estimated recording bitrate.

use std::path::{Path, PathBuf};

use crate::ini;
use crate::keys::{parse_obs_hotkey, KeyBind, MAX_BINDS};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ObsCfg {
    pub ws_known: bool,
    pub ws_enabled: bool,
    pub ws_port: u16,
    pub ws_auth: bool,
    /// read from obs-websocket's config.json (OBS 28+)
    pub ws_json: bool,
    pub ws_password: String,
    pub profile_dir: PathBuf,
    /// the Save clip key was read
    pub clip_known: bool,
    pub clip: Vec<KeyBind>,
    /// a [Hotkeys] section was found in the profile
    pub have_hotkeys: bool,
    /// OBSBasic.StartReplayBuffer's first binding
    pub rbkey: Option<KeyBind>,
    /// OBSBasic.StartRecording's first binding
    pub reckey: Option<KeyBind>,
    /// advanced output mode with a custom FFmpeg recording
    pub ffmpeg_rec: bool,
    pub adv: bool,
    pub rb_sec: i32,
    pub rb_mb: i32,
    /// recording bitrate estimate incl. audio (kbit/s), 0 = unknown
    pub kbps: i32,
    pub fps_num: i32,
    pub fps_den: i32,
    pub rec_dir: PathBuf,
}

fn read(p: &Path) -> Option<String> {
    let b = std::fs::read(p).ok()?;
    Some(String::from_utf8_lossy(&b).into_owned())
}

/// OBS escapes backslashes in some ini paths; '/' becomes '\', doubled '\' become one (obscfg.c `ini_path`).
fn ini_path(t: &str, sec: &str, key: &str) -> PathBuf {
    let Some(v) = ini::get(t, sec, key).filter(|v| !v.is_empty()) else { return PathBuf::new() };
    let mut out = String::new();
    for c in v.chars() {
        let c = if c == '/' { '\\' } else { c };
        if c == '\\' && out.len() > 1 && out.ends_with('\\') {
            continue;
        }
        out.push(c);
    }
    PathBuf::from(out)
}

/// Video bitrate from an encoder json; 0 = not bitrate based; a missing file = encoder defaults (2500).
fn enc_kbps(pdir: &Path, file: &str) -> i32 {
    match read(&pdir.join(file)) {
        None => 2500,
        Some(t) => match serde_json::from_str::<serde_json::Value>(&t) {
            Ok(v) => {
                let rc = v.get("rate_control").and_then(|x| x.as_str()).unwrap_or("CBR");
                if ["CBR", "VBR", "ABR"].iter().any(|r| rc.eq_ignore_ascii_case(r)) {
                    v.get("bitrate").and_then(|x| x.as_f64()).unwrap_or(2500.0) as i32
                } else {
                    0
                }
            }
            Err(_) => 0,
        },
    }
}

fn jbool(v: &serde_json::Value, k: &str) -> bool {
    v.get(k).map(|x| x.as_bool().unwrap_or_else(|| x.as_f64().is_some_and(|n| n != 0.0))).unwrap_or(false)
}

impl ObsCfg {
    /// Read everything from `obs_dir` (%APPDATA%\obs-studio). Never writes.
    pub fn read(obs_dir: &Path) -> ObsCfg {
        let mut c = ObsCfg { rb_sec: 20, ..Default::default() };
        let glob = read(&obs_dir.join("global.ini"));
        let user = read(&obs_dir.join("user.ini"));

        // the WebSocket server: OBS 28+ keeps it in the plugin's config.json
        let wsj = read(&obs_dir.join("plugin_config").join("obs-websocket").join("config.json"));
        if let Some(v) = wsj.as_deref().and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok()) {
            c.ws_known = true;
            c.ws_json = true;
            c.ws_enabled = jbool(&v, "server_enabled");
            c.ws_port = v.get("server_port").and_then(|x| x.as_f64()).unwrap_or(4455.0) as u16;
            c.ws_auth = jbool(&v, "auth_required");
            let pw = v.get("server_password").and_then(|x| x.as_str()).unwrap_or("");
            if pw.len() < 128 {
                c.ws_password = pw.to_string();
            }
        } else if let Some(g) = glob.as_deref().filter(|g| ini::get(g, "OBSWebSocket", "ServerPort").is_some()) {
            c.ws_known = true;
            c.ws_enabled = ini::is(g, "OBSWebSocket", "ServerEnabled", "true", false);
            c.ws_port = ini::get_int(g, "OBSWebSocket", "ServerPort", 4455) as u16;
            c.ws_auth = ini::is(g, "OBSWebSocket", "AuthRequired", "true", false);
            if let Some(pw) = ini::get(g, "OBSWebSocket", "ServerPassword").filter(|p| p.len() < 128) {
                c.ws_password = pw;
            }
        }

        // the active profile folder
        let mut profiles = PathBuf::new();
        if let Some(g) = glob.as_deref() {
            let p = ini_path(g, "Locations", "Profiles");
            if !p.as_os_str().is_empty() {
                profiles = p.join("obs-studio").join("basic").join("profiles");
            }
        }
        if profiles.as_os_str().is_empty() || !profiles.exists() {
            profiles = obs_dir.join("basic").join("profiles");
        }
        let pd = user
            .as_deref()
            .and_then(|u| ini::get(u, "Basic", "ProfileDir"))
            .or_else(|| glob.as_deref().and_then(|g| ini::get(g, "Basic", "ProfileDir")));
        if let Some(pd) = pd {
            c.profile_dir = profiles.join(pd);
        }
        if c.profile_dir.as_os_str().is_empty() {
            return c;
        }
        let Some(prof) = read(&c.profile_dir.join("basic.ini")) else { return c };

        if let Some(v) = ini::get(&prof, "Hotkeys", "ReplayBuffer") {
            c.clip = parse_obs_hotkey(&v, "ReplayBuffer.Save", MAX_BINDS);
            c.clip_known = !c.clip.is_empty();
            c.have_hotkeys = true;
        }
        if let Some(v) = ini::get(&prof, "Hotkeys", "OBSBasic.StartReplayBuffer") {
            c.rbkey = parse_obs_hotkey(&v, "bindings", 1).first().copied();
            c.have_hotkeys = true;
        }
        if let Some(v) = ini::get(&prof, "Hotkeys", "OBSBasic.StartRecording") {
            c.reckey = parse_obs_hotkey(&v, "bindings", 1).first().copied();
            c.have_hotkeys = true;
        }

        // FPS: type 0 = common list ("60", "29.97"), 1 = whole number, 2 = fraction
        let ty = ini::get_int(&prof, "Video", "FPSType", 0);
        c.fps_den = 1;
        if ty == 1 {
            c.fps_num = ini::get_int(&prof, "Video", "FPSInt", 30) as i32;
        } else if ty == 2 {
            c.fps_num = ini::get_int(&prof, "Video", "FPSNum", 30) as i32;
            c.fps_den = ini::get_int(&prof, "Video", "FPSDen", 1) as i32;
        } else {
            match ini::get(&prof, "Video", "FPSCommon") {
                Some(fc) => {
                    c.fps_num = ini::atoi(&fc) as i32;
                    if fc.contains('.') {
                        // 29.97 = 30000/1001, 59.94 = 60000/1001
                        c.fps_num = (c.fps_num + 1) * 1000;
                        c.fps_den = 1001;
                    }
                }
                None => c.fps_num = 30,
            }
        }
        if c.fps_den <= 0 {
            c.fps_den = 1;
        }
        c.adv = ini::is(&prof, "Output", "Mode", "Advanced", false);
        if c.adv {
            let ff = ini::is(&prof, "AdvOut", "RecType", "FFmpeg", false);
            c.ffmpeg_rec = ff;
            c.rb_sec = ini::get_int(&prof, "AdvOut", "RecRBTime", 20) as i32;
            c.rb_mb = ini::get_int(&prof, "AdvOut", "RecRBSize", 512) as i32;
            c.rec_dir = ini_path(&prof, "AdvOut", if ff { "FFFilePath" } else { "RecFilePath" });
            if ff {
                c.kbps = (ini::get_int(&prof, "AdvOut", "FFVBitrate", 2500) + ini::get_int(&prof, "AdvOut", "FFABitrate", 160)) as i32;
            } else {
                let tracks = ini::get_int(&prof, "AdvOut", "RecTracks", 1);
                // "none" = the recording reuses the streaming encoder
                let video = if ini::get(&prof, "AdvOut", "RecEncoder").as_deref() == Some("none") {
                    enc_kbps(&c.profile_dir, "streamEncoder.json")
                } else {
                    enc_kbps(&c.profile_dir, "recordEncoder.json")
                };
                if video > 0 {
                    c.kbps = video;
                    for i in 0..6 {
                        if tracks & (1 << i) != 0 {
                            c.kbps += ini::get_int(&prof, "AdvOut", &format!("Track{}Bitrate", i + 1), 160) as i32;
                        }
                    }
                }
            }
        } else {
            let q = ini::get(&prof, "SimpleOutput", "RecQuality");
            c.rb_sec = ini::get_int(&prof, "SimpleOutput", "RecRBTime", 20) as i32;
            c.rb_mb = ini::get_int(&prof, "SimpleOutput", "RecRBSize", 512) as i32;
            c.rec_dir = ini_path(&prof, "SimpleOutput", "FilePath");
            // only "Same as stream" quality is bitrate based; the others are quality based (unknown)
            if q.is_none() || q.as_deref() == Some("Stream") {
                c.kbps = (ini::get_int(&prof, "SimpleOutput", "VBitrate", 2500) + ini::get_int(&prof, "SimpleOutput", "ABitrate", 160)) as i32;
            }
        }
        c
    }

    /// The folder that holds every profile (the file watcher watches it, recursive: each profile's basic.ini).
    pub fn profiles_root(&self) -> Option<PathBuf> {
        self.profile_dir.parent().map(Path::to_path_buf).filter(|p| !p.as_os_str().is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join("bu-obs-test").join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reads_a_simple_profile_and_the_json_websocket_config() {
        let d = scratch("cfg1");
        std::fs::create_dir_all(d.join("plugin_config/obs-websocket")).unwrap();
        std::fs::write(d.join("plugin_config/obs-websocket/config.json"), r#"{"server_enabled":true,"server_port":4460,"auth_required":true,"server_password":"pw123456"}"#).unwrap();
        std::fs::write(d.join("user.ini"), "[Basic]\r\nProfileDir=Untitled\r\n").unwrap();
        let p = d.join("basic/profiles/Untitled");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("basic.ini"),
            "[Video]\nFPSCommon=59.94\n[SimpleOutput]\nRecRBTime=60\nFilePath=C:/Clips//Raw\nVBitrate=6000\nABitrate=160\n[Hotkeys]\nReplayBuffer={\"ReplayBuffer.Save\":[{\"key\":\"OBS_KEY_F9\"}]}\nOBSBasic.StartReplayBuffer={\"bindings\":[{\"key\":\"OBS_KEY_F10\",\"control\":true}]}\n",
        )
        .unwrap();
        let c = ObsCfg::read(&d);
        assert!(c.ws_known && c.ws_json && c.ws_enabled && c.ws_auth);
        assert_eq!((c.ws_port, c.ws_password.as_str()), (4460, "pw123456"));
        assert_eq!(c.profile_dir, p);
        assert_eq!(c.clip, vec![KeyBind::new(0x78, 0)]);
        assert_eq!(c.rbkey, Some(KeyBind::new(0x79, 1)));
        assert_eq!(c.reckey, None);
        assert_eq!((c.fps_num, c.fps_den), (60000, 1001));
        assert_eq!(c.rb_sec, 60);
        assert_eq!(c.rec_dir, PathBuf::from("C:\\Clips\\Raw"));
        assert_eq!(c.kbps, 6160);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn reads_advanced_output_and_the_old_global_ini_websocket() {
        let d = scratch("cfg2");
        std::fs::write(d.join("global.ini"), "[OBSWebSocket]\nServerEnabled=false\nServerPort=4444\n[Basic]\nProfileDir=P\n").unwrap();
        let p = d.join("basic/profiles/P");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("basic.ini"), "[Output]\nMode=Advanced\n[AdvOut]\nRecRBTime=30\nRecFilePath=D:\\\\Rec\nRecTracks=3\nTrack2Bitrate=320\n[Video]\nFPSType=2\nFPSNum=120\nFPSDen=1\n").unwrap();
        std::fs::write(p.join("recordEncoder.json"), r#"{"rate_control":"CBR","bitrate":40000}"#).unwrap();
        let c = ObsCfg::read(&d);
        assert!(c.ws_known && !c.ws_json && !c.ws_enabled);
        assert_eq!(c.ws_port, 4444);
        assert!(c.adv && !c.ffmpeg_rec);
        assert_eq!(c.rb_sec, 30);
        assert_eq!(c.rec_dir, PathBuf::from("D:\\Rec"));
        assert_eq!(c.kbps, 40000 + 160 + 320);
        assert_eq!((c.fps_num, c.fps_den), (120, 1));
        let _ = std::fs::remove_dir_all(&d);
    }
}
