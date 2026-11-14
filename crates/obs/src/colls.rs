//! OBS's scene collections, read-only (Order 132): the Switch scene list's "+ Add scene" lists the scenes of every
//! collection, and a row can be in another collection than the open one. OBS only reports the open collection's scenes
//! (obs-websocket), so the others come from their files (`basic\scenes\*.json`): each scene in OBS's order, the monitor
//! of its top-most visible display capture, and a capture device's own resolution (a hint for "Auto" until OBS reports
//! the real size).

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::ini;

/// One scene of a collection file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileScene {
    pub name: String,
    /// its top-most visible display capture's monitor_id (empty = none)
    pub mon_id: String,
    /// its top-most visible video capture device's own resolution (0 x 0 = none / the device's default)
    pub hint: (i32, i32),
}

/// One collection file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Coll {
    pub name: String,
    pub scenes: Vec<FileScene>,
}

/// Input kinds that are a video capture device (Windows' dshow, macOS, Linux, Blackmagic).
pub const CAPTURE_KINDS: [&str; 4] = ["dshow_input", "av_capture_input", "v4l2_input", "decklink-input"];

/// The folder OBS keeps the collections in: global.ini's [Locations] SceneCollections (like the profiles), else the
/// usual one.
pub fn dir(obs_dir: &Path) -> PathBuf {
    let glob = std::fs::read(obs_dir.join("global.ini")).ok().map(|b| String::from_utf8_lossy(&b).into_owned());
    if let Some(l) = glob.as_deref().and_then(|g| ini::get(g, "Locations", "SceneCollections")).filter(|l| !l.is_empty()) {
        let d = PathBuf::from(l.replace("\\\\", "\\")).join("obs-studio").join("basic").join("scenes");
        if d.exists() {
            return d;
        }
    }
    obs_dir.join("basic").join("scenes")
}

/// "3840x2160" -> (3840, 2160)
fn wxh(s: &str) -> (i32, i32) {
    let mut it = s.split(['x', 'X']).map(|p| p.trim().parse::<i32>().unwrap_or(0));
    match (it.next(), it.next()) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w, h),
        _ => (0, 0),
    }
}

/// One collection file's text -> its scenes (None = not a collection).
pub fn parse(text: &str) -> Option<Coll> {
    let v: Value = serde_json::from_str(text).ok()?;
    let name = v.get("name")?.as_str()?.to_string();
    let sources = v.get("sources").and_then(|s| s.as_array()).cloned().unwrap_or_default();
    let id = |s: &Value| s.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let sname = |s: &Value| s.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let scene = |n: &str| -> Option<FileScene> {
        let sc = sources.iter().find(|s| id(s) == "scene" && sname(s) == n)?;
        let mut f = FileScene { name: n.to_string(), ..Default::default() };
        let items = sc.get("settings").and_then(|s| s.get("items")).and_then(|i| i.as_array()).cloned().unwrap_or_default();
        // items are saved bottom to top: the last visible one wins
        for it in items.iter().filter(|i| i.get("visible").and_then(|x| x.as_bool()).unwrap_or(true)) {
            let Some(src) = sources.iter().find(|s| sname(s) == it.get("name").and_then(|x| x.as_str()).unwrap_or("")) else { continue };
            let st = src.get("settings");
            let k = id(src);
            if k == "monitor_capture" {
                f.mon_id = st.and_then(|s| s.get("monitor_id")).and_then(|x| x.as_str()).unwrap_or("").to_string();
            } else if CAPTURE_KINDS.contains(&k.as_str()) {
                // "resolution" counts only when the device is set to a custom one (res_type 1)
                let custom = st.and_then(|s| s.get("res_type")).and_then(|x| x.as_i64()) == Some(1);
                f.hint = if custom { wxh(st.and_then(|s| s.get("resolution")).and_then(|x| x.as_str()).unwrap_or("")) } else { (0, 0) };
            }
        }
        Some(f)
    };
    let mut scenes = Vec::new();
    let order = v.get("scene_order").and_then(|o| o.as_array()).cloned().unwrap_or_default();
    for o in &order {
        if let Some(f) = o.get("name").and_then(|x| x.as_str()).and_then(scene) {
            scenes.push(f);
        }
    }
    // scenes missing from scene_order (an older file): after the others, in file order
    for s in sources.iter().filter(|s| id(s) == "scene") {
        let n = sname(s);
        if !scenes.iter().any(|f| f.name == n) {
            if let Some(f) = scene(&n) {
                scenes.push(f);
            }
        }
    }
    Some(Coll { name, scenes })
}

/// Every collection in the folder (OBS's backups and anything else skipped), sorted by name. Never writes.
pub fn read_all(obs_dir: &Path) -> Vec<Coll> {
    let Ok(rd) = std::fs::read_dir(dir(obs_dir)) else { return Vec::new() };
    let mut out: Vec<Coll> = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()).is_none_or(|x| !x.eq_ignore_ascii_case("json")) {
            continue;
        }
        let Ok(b) = std::fs::read(&p) else { continue };
        if let Some(c) = parse(&String::from_utf8_lossy(&b)) {
            if !out.iter().any(|o| o.name == c.name) {
                out.push(c);
            }
        }
    }
    out.sort_by_key(|c| c.name.to_lowercase());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of his two collections (made-up monitor ids).
    const UNTITLED: &str = r#"{"name":"Untitled","current_scene":"21:9","scene_order":[{"name":"ps5"},{"name":"21:9"},{"name":"16:9"}],
        "sources":[
          {"id":"monitor_capture","name":"Display Capture","settings":{"monitor_id":"MON-A"}},
          {"id":"monitor_capture","name":"Display Capture 3","settings":{"monitor_id":"MON-B"}},
          {"id":"monitor_capture","name":"Display Capture 4","settings":{"monitor_id":"MON-B"}},
          {"id":"image_source","name":"Image","settings":{}},
          {"id":"dshow_input","name":"PS5 (Elgato 4K Pro)","settings":{"resolution":"3840x2160","res_type":1}},
          {"id":"scene","name":"16:9","settings":{"items":[{"name":"Display Capture 4","visible":true},{"name":"Image","visible":true}]}},
          {"id":"scene","name":"21:9","settings":{"items":[{"name":"Display Capture","visible":true},{"name":"Display Capture 3","visible":false}]}},
          {"id":"scene","name":"ps5","settings":{"items":[{"name":"PS5 (Elgato 4K Pro)","visible":true}]}},
          {"id":"scene","name":"old","settings":{"items":[]}}]}"#;

    #[test]
    fn reads_scenes_in_obs_order_with_their_monitor_and_capture_size() {
        let c = parse(UNTITLED).unwrap();
        assert_eq!(c.name, "Untitled");
        let got: Vec<(&str, &str, (i32, i32))> = c.scenes.iter().map(|s| (s.name.as_str(), s.mon_id.as_str(), s.hint)).collect();
        assert_eq!(got, vec![("ps5", "", (3840, 2160)), ("21:9", "MON-A", (0, 0)), ("16:9", "MON-B", (0, 0)), ("old", "", (0, 0))]);
        assert!(parse("{\"x\":1}").is_none());
        assert!(parse("not json").is_none());
    }

    #[test]
    fn reads_every_collection_file_but_not_the_backups() {
        let d = std::env::temp_dir().join("bu-obs-test").join(format!("colls-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let sc = d.join("basic/scenes");
        std::fs::create_dir_all(&sc).unwrap();
        std::fs::write(sc.join("Untitled.json"), UNTITLED).unwrap();
        std::fs::write(sc.join("Untitled.json.bak"), UNTITLED.replace("Untitled", "Backup")).unwrap();
        std::fs::write(sc.join("PS5_4K60.json"), r#"{"name":"PS5 4K60","scene_order":[{"name":"PS5"}],"sources":[{"id":"scene","name":"PS5","settings":{"items":[]}}]}"#).unwrap();
        let all = read_all(&d);
        assert_eq!(all.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["PS5 4K60", "Untitled"]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
