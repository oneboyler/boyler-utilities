//! The feature's own settings (ClipPing's `Settings g_set` + its keys), their defaults (ClipPing's, except the popup look:
//! the new Glass style is the default, Order 035), the import of ClipPing's NotificationsForOBS.ini and a flat key / value
//! form for the app's settings store.

use std::path::{Path, PathBuf};

use crate::keys::KeyBind;

/// Popups "Show on" (ClipPing W_*); also the status icon's monitor choice.
pub const W_OTHER: i32 = 0;
pub const W_SAME: i32 = 1;
pub const W_MON1: i32 = 2;
pub const W_MON2: i32 = 3;
pub const W_NONE: i32 = 4;
/// Popup position (ClipPing P_*).
pub const P_TL: i32 = 0;
pub const P_TC: i32 = 1;
pub const P_TR: i32 = 2;
pub const P_BL: i32 = 3;
pub const P_BC: i32 = 4;
pub const P_BR: i32 = 5;
pub const P_NEAR: i32 = 6;
pub const P_CUSTOM: i32 = 7;
/// Popup looks: ClipPing's six (same numbers as its settings file) + the app's own Glass.
pub const ST_CARD: i32 = 0;
pub const ST_PILL: i32 = 1;
pub const ST_ACCENT: i32 = 2;
pub const ST_TIMER: i32 = 3;
pub const ST_TILE: i32 = 4;
pub const ST_FLOAT: i32 = 5;
pub const ST_GLASS: i32 = 6;
/// Animations (ClipPing AN_*).
pub const AN_SLIDE: i32 = 0;
pub const AN_FADE: i32 = 1;
pub const AN_NONE: i32 = 2;
/// Status icon (ClipPing SI_*).
pub const SI_OFF: i32 = 0;
pub const SI_REC_MON: i32 = 1;
pub const SI_OTHER: i32 = 2;
pub const SI_MON1: i32 = 3;
pub const SI_MON2: i32 = 4;
/// A sound choice: 0 = none, 1..n = built-in, SND_CUSTOM = a .wav file.
pub const SND_CUSTOM: i32 = 99;
pub const MAX_LIST: usize = 32;

pub const WHERE_NAMES: [&str; 5] = ["Other monitor", "Recording monitor", "Monitor 1", "Monitor 2", "None"];
pub const POS_NAMES: [&str; 8] = ["Top left", "Top center", "Top right", "Bottom left", "Bottom center", "Bottom right", "Nearest to game", "Custom…"];
/// The look list as the page shows it: Glass first (the default), then ClipPing's six.
pub const STYLE_ORDER: [i32; 7] = [ST_GLASS, ST_CARD, ST_PILL, ST_ACCENT, ST_TIMER, ST_TILE, ST_FLOAT];
pub fn style_name(s: i32) -> &'static str {
    match s {
        ST_CARD => "Card",
        ST_PILL => "Pill",
        ST_ACCENT => "Accent edge",
        ST_TIMER => "Timer bar",
        ST_TILE => "Tile",
        ST_FLOAT => "Floating text",
        _ => "Glass",
    }
}
pub const BG_PRESETS: [u32; 6] = [0x2C2C2A, 0x0A0A0A, 0x0C2A4A, 0x26215C, 0x173404, 0xF1EFE8];
pub const BG_NAMES: [&str; 7] = ["Dark grey", "Black", "Deep blue", "Deep purple", "Dark green", "Light", "Custom…"];
pub const ANIM_NAMES: [&str; 3] = ["Slide in", "Fade", "None"];
pub const STATUS_NAMES: [&str; 5] = ["Off", "Recording monitor", "Other monitor", "Monitor 1", "Monitor 2"];

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub where_: i32,
    pub pos: i32,
    pub snd: [i32; 4],
    pub sndfile: [String; 4],
    pub vol: i32,
    pub start_obs: bool,
    pub keep_rb: bool,
    pub off_rec: bool,
    pub style: i32,
    pub bg: i32,
    pub bgcustom: u32,
    pub anim: i32,
    pub scale: i32,
    /// Custom position: the popup's centre as fractions x10000 of the work area (-1 = not set)
    pub cx: i32,
    pub cy: i32,
    pub status: i32,
    pub scenes: Vec<String>,
    /// the scene list is the user's own (ClipPing's [Scenes] Init): no default list is made
    pub scenes_init: bool,
    /// the Switch scene key (ClipPing's own key, not OBS's)
    pub switch_key: Option<KeyBind>,
    /// ClipPing's ClipKeyFallback: a Save clip key used while OBS's own is unknown
    pub clip_fallback: Option<KeyBind>,
    /// where OBS was seen running (earlier versions saved it; read-only)
    pub obs_path: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            where_: W_OTHER,
            pos: P_BR,
            snd: [3, 2, 3, 2], // Pop, Thud, Two tone, Knock
            sndfile: Default::default(),
            vol: 30,
            start_obs: true,
            keep_rb: true,
            off_rec: true,
            style: ST_GLASS,
            bg: 0,
            bgcustom: 0x2C2C2A,
            anim: AN_SLIDE,
            scale: 100,
            cx: -1,
            cy: -1,
            status: SI_OFF,
            scenes: Vec::new(),
            scenes_init: false,
            switch_key: None,
            clip_fallback: None,
            obs_path: String::new(),
        }
    }
}

/// ClipPing's `w_to_int`: optional '-', optional 0x (hex), digits.
pub fn parse_int(s: &str) -> i64 {
    let s = s.trim_start_matches(' ');
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let (hex, s) = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(r) => (true, r),
        None => (false, s),
    };
    let mut v: i64 = 0;
    for c in s.chars() {
        let d = match c.to_digit(if hex { 16 } else { 10 }) {
            Some(d) => d as i64,
            None => break,
        };
        v = v.wrapping_mul(if hex { 16 } else { 10 }).wrapping_add(d);
    }
    if neg {
        -v
    } else {
        v
    }
}

impl Settings {
    /// Every value back in its range (ClipPing's `settings_load` checks).
    pub fn clamp(&mut self) {
        if !(SI_OFF..=SI_MON2).contains(&self.status) {
            self.status = SI_OFF;
        }
        if !(ST_CARD..=ST_GLASS).contains(&self.style) {
            self.style = ST_GLASS;
        }
        if !(0..=6).contains(&self.bg) {
            self.bg = 0;
        }
        if !(AN_SLIDE..=AN_NONE).contains(&self.anim) {
            self.anim = AN_SLIDE;
        }
        if !(60..=200).contains(&self.scale) {
            self.scale = 100;
        }
        if !(P_TL..=P_CUSTOM).contains(&self.pos) {
            self.pos = P_BR;
        }
        if !(W_OTHER..=W_NONE).contains(&self.where_) {
            self.where_ = W_OTHER;
        }
        self.vol = self.vol.clamp(0, 100);
        self.scenes.truncate(MAX_LIST);
        self.bgcustom &= 0xFFFFFF;
    }

    /// The popups' background colour (Glass ignores it).
    pub fn popup_bg(&self) -> u32 {
        if self.bg == 6 {
            self.bgcustom
        } else {
            BG_PRESETS[self.bg.clamp(0, 5) as usize]
        }
    }

    /// The store's form: (key, value) pairs, every value.
    pub fn to_pairs(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = vec![
            ("where".into(), self.where_.to_string()),
            ("pos".into(), self.pos.to_string()),
            ("vol".into(), self.vol.to_string()),
            ("startobs".into(), (self.start_obs as i32).to_string()),
            ("keep".into(), (self.keep_rb as i32).to_string()),
            ("offrec".into(), (self.off_rec as i32).to_string()),
            ("style".into(), self.style.to_string()),
            ("bg".into(), self.bg.to_string()),
            ("bgcustom".into(), self.bgcustom.to_string()),
            ("anim".into(), self.anim.to_string()),
            ("scale".into(), self.scale.to_string()),
            ("cx".into(), self.cx.to_string()),
            ("cy".into(), self.cy.to_string()),
            ("status".into(), self.status.to_string()),
            ("scenesinit".into(), (self.scenes_init as i32).to_string()),
            ("switchkey".into(), self.switch_key.map(|k| k.pack()).unwrap_or(0).to_string()),
            ("clipfallback".into(), self.clip_fallback.map(|k| k.pack()).unwrap_or(0).to_string()),
            ("obspath".into(), self.obs_path.clone()),
        ];
        for i in 0..4 {
            v.push((format!("snd{i}"), self.snd[i].to_string()));
            v.push((format!("sndfile{i}"), self.sndfile[i].clone()));
        }
        v
    }

    /// The scene list (stored as its own list).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>, scenes: Option<Vec<String>>) -> Settings {
        let d = Settings::default();
        let int = |k: &str, def: i32| get(k).filter(|s| !s.is_empty()).map(|s| parse_int(&s) as i32).unwrap_or(def);
        let key = |k: &str| match get(k).map(|s| parse_int(&s) as u32).unwrap_or(0) {
            0 => None,
            v => Some(KeyBind::unpack(v)),
        };
        let mut s = Settings {
            where_: int("where", d.where_),
            pos: int("pos", d.pos),
            snd: [int("snd0", 3), int("snd1", 2), int("snd2", 3), int("snd3", 2)],
            sndfile: [0, 1, 2, 3].map(|i| get(&format!("sndfile{i}")).unwrap_or_default()),
            vol: int("vol", d.vol),
            start_obs: int("startobs", 1) != 0,
            keep_rb: int("keep", 1) != 0,
            off_rec: int("offrec", 1) != 0,
            style: int("style", d.style),
            bg: int("bg", d.bg),
            bgcustom: int("bgcustom", d.bgcustom as i32) as u32,
            anim: int("anim", d.anim),
            scale: int("scale", d.scale),
            cx: int("cx", -1),
            cy: int("cy", -1),
            status: int("status", d.status),
            scenes: scenes.unwrap_or_default(),
            scenes_init: int("scenesinit", 0) != 0,
            switch_key: key("switchkey"),
            clip_fallback: key("clipfallback"),
            obs_path: get("obspath").unwrap_or_default(),
        };
        s.clamp();
        s
    }
}

// ---------------------------------------------------------------- ClipPing's settings file (import, read-only)

/// ClipPing's ini text, decoded: UTF-16 LE with its BOM (how ClipPing creates it), else UTF-8 / ANSI as bytes.
pub fn decode_ini(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let u: Vec<u16> = bytes[2..].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        return String::from_utf16_lossy(&u);
    }
    String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes)).into_owned()
}

/// GetPrivateProfileString-like read (section / key case-insensitive, spaces trimmed).
fn pp_get(text: &str, section: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for raw in text.lines() {
        let line = raw.trim_matches(|c: char| c == ' ' || c == '\t' || c == '\u{feff}' || c == '\r');
        if let Some(rest) = line.strip_prefix('[') {
            inside = rest.split(']').next().unwrap_or("").trim().eq_ignore_ascii_case(section);
            continue;
        }
        if !inside {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim().eq_ignore_ascii_case(key) {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// ClipPing's settings as it loads them (app.c `settings_load` + the keys), from its file's text. The section is
/// "NotificationsForOBS" (or "ClipPing" in the first versions). The popup look stays the app's Glass (A_035_01: the
/// file's PopupStyle is only the default an older ClipPing wrote); every other value it has is taken; missing ones keep
/// ClipPing's defaults.
pub fn import_clipping(text: &str) -> Settings {
    let sec = if pp_get(text, "NotificationsForOBS", "PopupWhere").is_some() || text.to_ascii_lowercase().contains("[notificationsforobs]") {
        "NotificationsForOBS"
    } else {
        "ClipPing"
    };
    let get = |k: &str| pp_get(text, sec, k).filter(|v| !v.is_empty());
    let int = |k: &str, def: i32| get(k).map(|v| parse_int(&v) as i32).unwrap_or(def);
    let d = Settings::default();
    let key = |k: &str| match get(k).map(|v| parse_int(&v) as u32).unwrap_or(0) {
        0 => None,
        v => Some(KeyBind::unpack(v)),
    };
    let n = pp_get(text, "Scenes", "Count").map(|v| parse_int(&v)).unwrap_or(0).clamp(0, MAX_LIST as i64) as usize;
    let scenes = (1..=n).map(|i| pp_get(text, "Scenes", &i.to_string()).unwrap_or_default()).collect();
    let mut s = Settings {
        where_: int("PopupWhere", d.where_),
        pos: int("PopupPos", d.pos),
        snd: [int("Sound0", 3), int("Sound1", 2), int("Sound2", 3), int("Sound3", 2)],
        sndfile: [0, 1, 2, 3].map(|i| get(&format!("SoundFile{i}")).unwrap_or_default()),
        vol: int("Volume", 30),
        start_obs: int("StartObs", 1) != 0,
        keep_rb: int("KeepReplay", int("AutoReplay", 1)) != 0,
        off_rec: int("ReplayOffWhileRecording", 1) != 0,
        style: ST_GLASS,
        bg: int("PopupBg", 0),
        bgcustom: int("PopupBgCustom", 0x2C2C2A) as u32,
        anim: int("PopupAnim", AN_SLIDE),
        scale: int("PopupScale", 100),
        cx: int("PopupCustomX", -1),
        cy: int("PopupCustomY", -1),
        status: int("StatusIcon", SI_OFF),
        scenes,
        scenes_init: pp_get(text, "Scenes", "Init").map(|v| parse_int(&v) != 0).unwrap_or(false) || n > 0,
        switch_key: key("SwitchKey"),
        clip_fallback: key("ClipKeyFallback"),
        obs_path: get("ObsPath").unwrap_or_default(),
    };
    s.clamp();
    s
}

/// Where ClipPing keeps its settings: NotificationsForOBS.ini (or ClipPing.ini) next to its exe. The exes looked at:
/// the running NotificationsForOBS.exe, the one its "Start with Windows" entry starts. First file found.
pub fn find_clipping_ini(exes: &[PathBuf]) -> Option<PathBuf> {
    for exe in exes {
        let Some(dir) = exe.parent() else { continue };
        for name in ["NotificationsForOBS.ini", "ClipPing.ini"] {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// The exe a Run entry value starts (`"C:\x\y.exe" --autostart` -> C:\x\y.exe).
pub fn exe_of_command(v: &str) -> Option<PathBuf> {
    let v = v.trim();
    let p = if let Some(r) = v.strip_prefix('"') { r.split('"').next()? } else { v.split(' ').next()? };
    (!p.is_empty()).then(|| PathBuf::from(p))
}

/// Read + import a ClipPing settings file (never writes it).
pub fn import_file(p: &Path) -> Option<Settings> {
    let b = std::fs::read(p).ok()?;
    Some(import_clipping(&decode_ini(&b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of a real file, with made-up scene names.
    const HIS: &str = "\u{feff}\r\n[NotificationsForOBS]\r\nStartWithWindows=0x1\r\nSetup=0x1\r\nPopupWhere=0x1\r\nPopupPos=0x5\r\nVolume=0x1E\r\nSound0=0x3\r\nSoundFile0=\r\nSound1=0x2\r\nSoundFile1=\r\nSound2=0x3\r\nSoundFile2=\r\nSound3=0x2\r\nSoundFile3=\r\nStartObs=0x1\r\nKeepReplay=0x1\r\nReplayOffWhileRecording=0x1\r\nPopupStyle=0x0\r\nPopupBg=0x0\r\nPopupBgCustom=0x2C2C2A\r\nPopupAnim=0x0\r\nPopupScale=0x64\r\nStatusIcon=0x1\r\nPopupCustomX=-1\r\nPopupCustomY=-1\r\nObsPath=C:\\Program Files\\obs-studio\\bin\\64bit\\obs64.exe\r\nSwitchKey=0x13\r\n[Scenes]\r\nCount=0x2\r\n1=A\r\n2=B\r\n";

    fn utf16(s: &str) -> Vec<u8> {
        let mut b = vec![0xFF, 0xFE];
        for u in s.encode_utf16() {
            b.extend_from_slice(&u.to_le_bytes());
        }
        b
    }

    #[test]
    fn imports_his_file_with_glass_kept() {
        let s = import_clipping(&decode_ini(&utf16(HIS)));
        assert_eq!(s.where_, W_SAME);
        assert_eq!(s.pos, P_BR);
        assert_eq!(s.vol, 30);
        assert_eq!(s.snd, [3, 2, 3, 2]);
        assert_eq!(s.style, ST_GLASS);
        assert_eq!(s.status, SI_REC_MON);
        assert_eq!(s.switch_key, Some(KeyBind::new(0x13, 0)));
        assert_eq!(s.scenes, vec!["A".to_string(), "B".to_string()]);
        assert!(s.scenes_init && s.keep_rb && s.off_rec && s.start_obs);
        assert_eq!(s.obs_path, "C:\\Program Files\\obs-studio\\bin\\64bit\\obs64.exe");
    }

    #[test]
    fn missing_values_keep_clippings_defaults_and_store_round_trip() {
        let s = import_clipping("[NotificationsForOBS]\nVolume=0x50\n");
        let d = Settings::default();
        assert_eq!(s.vol, 80);
        assert_eq!(Settings { vol: 80, ..d.clone() }, s);
        let pairs = s.to_pairs();
        let back = Settings::from_lookup(|k| pairs.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()), Some(vec![]));
        assert_eq!(back, s);
        assert_eq!(parse_int("-1"), -1);
        assert_eq!(parse_int("0x2C2C2A"), 0x2C2C2A);
    }

    #[test]
    fn run_entry_command() {
        assert_eq!(exe_of_command("\"C:\\Users\\User\\Downloads\\NotificationsForOBS.exe\" --autostart"), Some(PathBuf::from("C:\\Users\\User\\Downloads\\NotificationsForOBS.exe")));
        assert_eq!(exe_of_command("C:\\a.exe --x"), Some(PathBuf::from("C:\\a.exe")));
    }
}
