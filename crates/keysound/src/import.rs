//! "Import a Mechvibes pack" (Order 058): reads such a pack's `config.json` + its audio file(s) and turns it into the same
//! five-sound set as our own packs. Only the pack FORMAT is read (the shape of `config.json`, WAV / Ogg Vorbis audio) — no
//! code or sound of anyone's is shipped with the app; a pack is the user's own file.
//!
//! `config.json` (single type): `"sound": "sound.ogg"` and `"defines": { "<scancode>": [start_ms, length_ms], … }`, a key
//! coming up is `"<scancode>-up"`; multi type: `"defines": { "<scancode>": "file.wav", … }`. The engine only knows a key's
//! CLASS, so the pack's A key (its first letter key when it has no A) gives the plain down / up sounds, and Space (57),
//! Enter (28) and Backspace (14) their own — each falls back to the plain sound when the pack doesn't define it.
//!
//! Safe by construction: file names in the config must be plain names inside the pack's folder (no paths, no `..`), every
//! file and slice has a size limit, nothing is executed.

use crate::kind::{Kind, KINDS};
use crate::synth::{SoundSet, PEAK};
use std::path::Path;
use std::sync::Arc;

const MAX_FILE: u64 = 40 * 1024 * 1024;
const MAX_SECONDS: f32 = 3.0;
const MAX_CONFIG: u64 = 2 * 1024 * 1024;

/// A pack read from a folder.
#[derive(Debug, Clone)]
pub struct Imported {
    pub name: String,
    pub set: SoundSet,
}

/// Decoded audio: mono, f32.
struct Audio {
    rate: u32,
    mono: Vec<f32>,
}

/// Reads the pack in `dir`.
pub fn read_pack(dir: &Path) -> Result<Imported, String> {
    let cfg_path = dir.join("config.json");
    let meta = std::fs::metadata(&cfg_path).map_err(|_| "this folder has no config.json (is it a Mechvibes pack?)".to_string())?;
    if meta.len() > MAX_CONFIG {
        return Err("config.json is too big".into());
    }
    let text = std::fs::read_to_string(&cfg_path).map_err(|e| format!("config.json: {e}"))?;
    let fallback = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported".into());
    from_parts(&text, &fallback, &|name| {
        let p = dir.join(name);
        let m = std::fs::metadata(&p).map_err(|_| format!("the pack's sound file {name} is missing"))?;
        if m.len() > MAX_FILE {
            return Err(format!("{name} is too big ({} MB)", m.len() / 1024 / 1024));
        }
        std::fs::read(&p).map_err(|e| format!("{name}: {e}"))
    })
}

/// A plain file name (no folders, no `..`): the only kind a pack's config may name.
fn plain_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 120 && !s.contains(['/', '\\', ':', '\0']) && s != "." && s != ".."
}

/// The pack described by `config` (the text of config.json); `load(name)` gives a file's bytes. `fallback` names the pack when
/// the config has no name.
pub fn from_parts(config: &str, fallback: &str, load: &dyn Fn(&str) -> Result<Vec<u8>, String>) -> Result<Imported, String> {
    let v: serde_json::Value = serde_json::from_str(config).map_err(|e| format!("config.json isn't valid JSON: {e}"))?;
    let name = v
        .get("name")
        .and_then(|n| n.as_str())
        .map(|s| s.trim().chars().filter(|c| !c.is_control()).take(60).collect::<String>())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback.to_string());
    let defines = v.get("defines").and_then(|d| d.as_object()).ok_or("config.json has no \"defines\"")?;
    // real packs leave keys empty ("" or null = no sound for that key): those are not defined
    let defines: serde_json::Map<String, serde_json::Value> = defines.iter().filter(|(_, x)| !(x.is_null() || x.as_str() == Some(""))).map(|(k, x)| (k.clone(), x.clone())).collect();
    let multi = v.get("key_define_type").and_then(|t| t.as_str()) == Some("multi");

    // which defined entry gives which of the five sounds
    let letter = ["30", "31", "32", "16", "17", "18", "19", "20", "21", "22", "23", "24", "25", "38", "39", "40", "44", "45", "46", "47", "48", "49", "50"];
    let plain = letter
        .iter()
        .find(|k| defines.contains_key(**k))
        .map(|k| k.to_string())
        .or_else(|| defines.keys().filter(|k| !k.ends_with("-up") && k.parse::<u32>().is_ok()).min_by_key(|k| k.parse::<u32>().unwrap_or(u32::MAX)).cloned())
        .ok_or("the pack defines no keys")?;
    let pick = |key: &str| defines.get(key).map(|_| key.to_string());
    let chosen: [Option<String>; KINDS] = [
        Some(plain.clone()),
        pick(&format!("{plain}-up")).or_else(|| defines.keys().find(|k| k.ends_with("-up")).cloned()),
        pick("57").or(Some(plain.clone())),
        pick("28").or(Some(plain.clone())),
        pick("14").or(Some(plain.clone())),
    ];

    let mut rate = 0u32;
    let mut sounds: Vec<Vec<f32>> = Vec::with_capacity(KINDS);
    if multi {
        let mut cache: Vec<(String, Audio)> = Vec::new();
        for c in &chosen {
            let Some(key) = c else {
                sounds.push(vec![0.0; 2]);
                continue;
            };
            let file = defines[key].as_str().ok_or_else(|| format!("define \"{key}\" isn't a file name"))?;
            if !plain_name(file) {
                return Err(format!("\"{file}\" isn't a plain file name inside the pack"));
            }
            if !cache.iter().any(|(n, _)| n == file) {
                let a = decode(file, &load(file)?)?;
                cache.push((file.to_string(), a));
            }
            let a = &cache.iter().find(|(n, _)| n == file).expect("cached").1;
            if rate == 0 {
                rate = a.rate;
            }
            sounds.push(if a.rate == rate { limit(a.mono.clone(), rate) } else { resample(&a.mono, a.rate, rate) });
        }
    } else {
        let file = v.get("sound").and_then(|s| s.as_str()).ok_or("config.json has no \"sound\" file")?;
        if !plain_name(file) {
            return Err(format!("\"{file}\" isn't a plain file name inside the pack"));
        }
        let a = decode(file, &load(file)?)?;
        rate = a.rate;
        for c in &chosen {
            let Some(key) = c else {
                sounds.push(vec![0.0; 2]);
                continue;
            };
            let slice = defines[key].as_array().filter(|a| a.len() == 2).ok_or_else(|| format!("define \"{key}\" isn't [start_ms, length_ms]"))?;
            let (st, ln) = (slice[0].as_f64().unwrap_or(-1.0), slice[1].as_f64().unwrap_or(-1.0));
            if st < 0.0 || ln <= 0.0 {
                return Err(format!("define \"{key}\" has a bad start / length"));
            }
            let s0 = ((st / 1000.0) * rate as f64) as usize;
            let s1 = (((st + ln) / 1000.0) * rate as f64) as usize;
            if s0 >= a.mono.len() {
                return Err(format!("define \"{key}\" starts after the end of {file}"));
            }
            sounds.push(limit(a.mono[s0..s1.min(a.mono.len())].to_vec(), rate));
        }
    }

    // soft edges, then the same loudness rule as our own packs: the plain key-down sound peaks at PEAK, the others keep their level
    let fade = (rate as usize / 1000).max(1);
    for s in sounds.iter_mut() {
        let n = s.len();
        for i in 0..fade.min(n) {
            let f = i as f32 / fade as f32;
            s[i] *= f;
            s[n - 1 - i] *= f;
        }
    }
    let peak = sounds[Kind::Down as usize].iter().fold(0.0f32, |m, x| m.max(x.abs())).max(1.0e-6);
    let scale = PEAK / peak;
    for s in sounds.iter_mut() {
        for x in s.iter_mut() {
            *x *= scale;
        }
    }
    let mut it = sounds.into_iter().map(|v| Arc::<[f32]>::from(v.into_boxed_slice()));
    Ok(Imported { name, set: SoundSet { rate, sounds: std::array::from_fn(|_| it.next().expect("five sounds")) } })
}

/// At most MAX_SECONDS of sound (a pack's slice or file can't be made huge to eat memory).
fn limit(mut v: Vec<f32>, rate: u32) -> Vec<f32> {
    v.truncate((MAX_SECONDS * rate as f32) as usize);
    if v.len() < 2 {
        v.resize(2, 0.0);
    }
    v
}

fn resample(v: &[f32], from: u32, to: u32) -> Vec<f32> {
    let ratio = from as f32 / to as f32;
    let n = ((v.len() as f32) / ratio) as usize;
    let out: Vec<f32> = (0..n)
        .map(|i| {
            let p = i as f32 * ratio;
            let k = p as usize;
            let f = p - k as f32;
            let a = v.get(k).copied().unwrap_or(0.0);
            let b = v.get(k + 1).copied().unwrap_or(a);
            a + (b - a) * f
        })
        .collect();
    limit(out, to)
}

/// Order 090: one sound of the user's own ("Your sound" on a key, "Make a pack from one sound"), decoded: mono f32 at its own
/// rate. Cloning is cheap (shared samples).
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub rate: u32,
    pub mono: Arc<[f32]>,
}

/// The longest sound of your own kept (a funny noise on a key may run a bit; a file can't be made huge to eat memory).
pub const MAX_OWN_SECONDS: f32 = 10.0;

/// Reads one sound file of the user's: WAV or Ogg Vorbis here, anything else (MP3 above all) through Windows' own decoder.
/// Cut to [`MAX_OWN_SECONDS`], soft edges, loudest sample at the packs' level. Call it through [`crate::safe::read_sound`]
/// (a broken file must not be able to end the app).
pub fn read_sound_file(path: &Path) -> Result<Clip, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let len = std::fs::metadata(path).map_err(|_| format!("{name} can't be read"))?.len();
    if len > MAX_FILE {
        return Err(format!("{name} is too big ({} MB)", len / 1024 / 1024));
    }
    if len == 0 {
        return Err(format!("{name} is empty"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{name}: {e}"))?;
    let ours = (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE")) || bytes.starts_with(b"OggS");
    let (rate, mono) = if ours {
        let a = decode(&name, &bytes)?;
        (a.rate, a.mono)
    } else {
        os_decode(path, &name)?
    };
    clip_of(rate, mono)
}

#[cfg(windows)]
fn os_decode(path: &Path, _name: &str) -> Result<(u32, Vec<f32>), String> {
    crate::mf::decode_file(path, (MAX_OWN_SECONDS * 192_000.0) as usize)
}

#[cfg(not(windows))]
fn os_decode(_path: &Path, name: &str) -> Result<(u32, Vec<f32>), String> {
    Err(format!("{name}: only WAV and Ogg Vorbis can be read here"))
}

/// Raw samples -> a [`Clip`]: cut, soft edges (1 ms), the loudest sample at [`PEAK`].
pub fn clip_of(rate: u32, mut v: Vec<f32>) -> Result<Clip, String> {
    if !(8000..=192_000).contains(&rate) {
        return Err(format!("unusable sound ({rate} Hz)"));
    }
    v.truncate((MAX_OWN_SECONDS * rate as f32) as usize);
    for x in v.iter_mut() {
        if !x.is_finite() {
            *x = 0.0;
        }
    }
    let peak = v.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if v.len() < 2 || peak < 1.0e-5 {
        return Err("there is no sound in this file (silence)".into());
    }
    let fade = (rate as usize / 1000).max(1).min(v.len() / 2);
    let n = v.len();
    for i in 0..fade {
        let f = i as f32 / fade as f32;
        v[i] *= f;
        v[n - 1 - i] *= f;
    }
    // (measured again after the fade: a sound that starts at full level lost its first peak to it)
    let peak = v.iter().fold(0.0f32, |m, x| m.max(x.abs())).max(1.0e-6);
    let scale = PEAK / peak;
    for x in v.iter_mut() {
        *x *= scale;
    }
    Ok(Clip { rate, mono: Arc::from(v.into_boxed_slice()) })
}

fn decode(name: &str, bytes: &[u8]) -> Result<Audio, String> {
    let lower = name.to_ascii_lowercase();
    let a = if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
        decode_wav(bytes)?
    } else if bytes.starts_with(b"OggS") {
        decode_ogg(bytes)?
    } else if lower.ends_with(".mp3") || bytes.starts_with(b"ID3") || bytes.starts_with(&[0xFF, 0xFB]) {
        return Err(format!("{name} is an MP3: Boyler Utilities reads WAV and Ogg Vorbis packs"));
    } else {
        return Err(format!("{name}: unsupported audio format (WAV and Ogg Vorbis are supported)"));
    };
    if a.rate < 8000 || a.rate > 192_000 || a.mono.is_empty() {
        return Err(format!("{name}: unusable audio ({} Hz, {} samples)", a.rate, a.mono.len()));
    }
    Ok(a)
}

fn decode_ogg(bytes: &[u8]) -> Result<Audio, String> {
    use lewton::inside_ogg::OggStreamReader;
    let mut r = OggStreamReader::new(std::io::Cursor::new(bytes)).map_err(|e| format!("not a readable Ogg Vorbis file: {e}"))?;
    let ch = r.ident_hdr.audio_channels.max(1) as usize;
    let rate = r.ident_hdr.audio_sample_rate;
    let mut mono: Vec<f32> = Vec::new();
    let cap = (MAX_FILE as usize) / 2;
    loop {
        match r.read_dec_packet_itl() {
            Ok(Some(p)) => {
                for f in p.chunks_exact(ch) {
                    mono.push(f.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / ch as f32);
                }
                if mono.len() > cap {
                    return Err("the Ogg file is too long".into());
                }
            }
            Ok(None) => break,
            Err(e) => return Err(format!("the Ogg file is damaged: {e}")),
        }
    }
    Ok(Audio { rate, mono })
}

fn decode_wav(b: &[u8]) -> Result<Audio, String> {
    let u16le = |o: usize| b.get(o..o + 2).map(|x| u16::from_le_bytes([x[0], x[1]]));
    let u32le = |o: usize| b.get(o..o + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]));
    let mut pos = 12;
    let (mut tag, mut ch, mut rate, mut bits) = (0u16, 0usize, 0u32, 0u16);
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let size = u32le(pos + 4).unwrap_or(0) as usize;
        let body = pos + 8;
        let end = (body + size).min(b.len());
        if id == b"fmt " {
            tag = u16le(body).ok_or("short fmt chunk")?;
            ch = u16le(body + 2).ok_or("short fmt chunk")? as usize;
            rate = u32le(body + 4).ok_or("short fmt chunk")?;
            bits = u16le(body + 14).ok_or("short fmt chunk")?;
            if tag == 0xFFFE {
                // extensible: the real format is the first two bytes of the sub-format GUID
                tag = u16le(body + 24).unwrap_or(0);
            }
        } else if id == b"data" {
            data = Some(&b[body..end]);
            break;
        }
        pos = body + size + (size & 1);
    }
    let data = data.ok_or("the WAV file has no sound data")?;
    if ch == 0 || ch > 8 || rate == 0 {
        return Err("the WAV file's format is unreadable".into());
    }
    let bytes = (bits / 8) as usize;
    if bytes == 0 || data.len() / (bytes * ch) == 0 {
        return Err("the WAV file is empty".into());
    }
    let frames = data.len() / (bytes * ch);
    let sample = |f: usize, c: usize| -> Result<f32, String> {
        let o = (f * ch + c) * bytes;
        let d = &data[o..o + bytes];
        Ok(match (tag, bits) {
            (1, 8) => (d[0] as f32 - 128.0) / 128.0,
            (1, 16) => i16::from_le_bytes([d[0], d[1]]) as f32 / 32768.0,
            (1, 24) => (i32::from_le_bytes([0, d[0], d[1], d[2]]) >> 8) as f32 / 8_388_608.0,
            (1, 32) => i32::from_le_bytes([d[0], d[1], d[2], d[3]]) as f32 / 2_147_483_648.0,
            (3, 32) => f32::from_le_bytes([d[0], d[1], d[2], d[3]]),
            _ => return Err(format!("the WAV file's format isn't supported (tag {tag}, {bits} bit)")),
        })
    };
    let mut mono = Vec::with_capacity(frames);
    for f in 0..frames {
        let mut s = 0.0;
        for c in 0..ch {
            s += sample(f, c)?;
        }
        mono.push(s / ch as f32);
    }
    Ok(Audio { rate, mono })
}

// ------------------------------------------------------------------ installed packs (the app's own folder)

/// The files a config names (the single `sound`, or every file of a multi pack), each a plain file name.
pub fn referenced_files(config: &str) -> Result<Vec<String>, String> {
    let v: serde_json::Value = serde_json::from_str(config).map_err(|e| format!("config.json isn't valid JSON: {e}"))?;
    let mut out: Vec<String> = Vec::new();
    // a multi pack's stray "sound" entry (the Opera GX pack names a sound.ogg it does not ship) is not a file it needs
    let multi = v.get("key_define_type").and_then(|t| t.as_str()) == Some("multi");
    if let (false, Some(s)) = (multi, v.get("sound").and_then(|s| s.as_str())) {
        out.push(s.to_string());
    }
    if let Some(d) = v.get("defines").and_then(|d| d.as_object()) {
        out.extend(d.values().filter_map(|x| x.as_str()).filter(|s| !s.is_empty()).map(str::to_string));
    }
    out.sort();
    out.dedup();
    if let Some(bad) = out.iter().find(|f| !plain_name(f)) {
        return Err(format!("\"{bad}\" isn't a plain file name inside the pack"));
    }
    if out.len() > 200 {
        return Err("the pack names too many files".into());
    }
    Ok(out)
}

/// A folder-safe name for a pack: letters, digits, space, `-`, `_`, `.`; at most 60; never a Windows device name.
pub fn folder_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || " -_.".contains(c) { c } else { ' ' }).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s: String = s.chars().take(60).collect::<String>().trim_matches(['.', ' ']).to_string();
    let lower = s.to_ascii_lowercase();
    let reserved = ["con", "prn", "aux", "nul"].contains(&lower.as_str())
        || (lower.len() == 4 && (lower.starts_with("com") || lower.starts_with("lpt")) && lower.ends_with(|c: char| c.is_ascii_digit()));
    if s.is_empty() {
        "Imported".into()
    } else if reserved {
        format!("pack {s}")
    } else {
        s
    }
}

/// Imports the pack in `src` into the app's pack folder `root`: reads and decodes it first (a bad pack changes nothing),
/// then copies its `config.json` and the files it names into `root\<name>`. Returns the pack (its name = the folder's).
/// `src` may hold the pack directly or in ONE folder inside it (a zip unpacked by Explorer is that).
pub fn install(src: &Path, root: &Path) -> Result<Imported, String> {
    let src = pack_dir(src)?;
    let pack = read_pack(&src)?;
    let config = std::fs::read_to_string(src.join("config.json")).map_err(|e| format!("config.json: {e}"))?;
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for f in referenced_files(&config)? {
        let bytes = std::fs::read(src.join(&f)).map_err(|e| format!("{f}: {e}"))?;
        files.push((f, bytes));
    }
    put(pack, &config, &files, root)
}

/// The folder that holds `config.json`: `src` itself, or the one folder inside it that does.
fn pack_dir(src: &Path) -> Result<std::path::PathBuf, String> {
    if src.join("config.json").is_file() {
        return Ok(src.to_path_buf());
    }
    let inner: Vec<std::path::PathBuf> = std::fs::read_dir(src)
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("config.json").is_file()).collect())
        .unwrap_or_default();
    match inner.as_slice() {
        [one] => Ok(one.clone()),
        _ => Err("this folder is not a Mechvibes pack: no config.json inside".into()),
    }
}

/// The biggest .zip taken (a Mechvibes pack is a few MB), and the most it may unpack to.
const MAX_ZIP: u64 = 100 * 1024 * 1024;
const MAX_UNPACKED: usize = 160 * 1024 * 1024;

/// Imports a Mechvibes pack as the site hands it out: the `.zip` itself (its `config.json` at the top or inside one folder
/// of it) - read in memory, nothing is unpacked to disk but the pack's own files - or a folder. Err: a plain sentence
/// ("This isn't a Mechvibes pack: …").
pub fn install_any(src: &Path, root: &Path) -> Result<Imported, String> {
    if src.is_dir() {
        return install(src, root);
    }
    // the one picker takes a .zip or a pack folder: a folder is picked by its config.json (a file dialog can't pick both)
    if src.file_name().is_some_and(|n| n.eq_ignore_ascii_case("config.json")) {
        if let Some(dir) = src.parent() {
            return install(dir, root);
        }
    }
    let is_zip = src.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    let len = std::fs::metadata(src).map_err(|e| format!("can't read {}: {e}", src.display()))?.len();
    if len > MAX_ZIP {
        return Err(format!("that file is too big for a sound pack ({} MB)", len / 1024 / 1024));
    }
    let bytes = std::fs::read(src).map_err(|e| format!("can't read {}: {e}", src.display()))?;
    if !is_zip && !bytes.starts_with(b"PK") {
        return Err("This isn't a Mechvibes pack: pick the .zip you downloaded, or the config.json inside the pack's folder".into());
    }
    let fallback = src.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported".into());
    install_zip(&bytes, &fallback, root)
}

/// [`install_any`] for a zip already in memory; `fallback` names a pack whose config has no name.
pub fn install_zip(zip: &[u8], fallback: &str, root: &Path) -> Result<Imported, String> {
    let (pack, config, files) = decode_zip(zip, fallback)?;
    put(pack, &config, &files, root)
}

/// A zip read and decoded in memory, nothing written (Order 076: the "Get more sounds" play button hears a pack first).
pub fn preview_zip(zip: &[u8], fallback: &str) -> Result<Imported, String> {
    decode_zip(zip, fallback).map(|d| d.0)
}

type Decoded = (Imported, String, Vec<(String, Vec<u8>)>);

fn decode_zip(zip: &[u8], fallback: &str) -> Result<Decoded, String> {
    let entries = bu_addons::zip::read(zip, MAX_UNPACKED).map_err(|_| "This isn't a Mechvibes pack: that .zip can't be read (damaged, encrypted or not a zip)".to_string())?;
    let norm = |n: &str| n.replace(std::path::MAIN_SEPARATOR, "/");
    // the shallowest config.json (not macOS' __MACOSX copies)
    let cfg = entries
        .iter()
        .filter(|e| !e.dir)
        .filter(|e| {
            let n = norm(&e.name);
            !n.starts_with("__MACOSX/") && n.rsplit('/').next().is_some_and(|f| f.eq_ignore_ascii_case("config.json"))
        })
        .min_by_key(|e| norm(&e.name).matches('/').count())
        .ok_or("This isn't a Mechvibes pack: there is no config.json in that .zip")?;
    let cfg_name = norm(&cfg.name);
    let base = &cfg_name[..cfg_name.len() - "config.json".len()];
    if cfg.data.len() as u64 > MAX_CONFIG {
        return Err("config.json is too big".into());
    }
    let config = String::from_utf8_lossy(&cfg.data).trim_start_matches('\u{feff}').to_string();
    let fallback = if base.is_empty() { fallback.to_string() } else { base.trim_end_matches('/').rsplit('/').next().unwrap_or(fallback).to_string() };
    // (a pack's config often names a file in other letter case than the zip holds it)
    let find = |name: &str| {
        let want = format!("{base}{name}");
        entries.iter().find(|e| !e.dir && norm(&e.name) == want).or_else(|| entries.iter().find(|e| !e.dir && norm(&e.name).eq_ignore_ascii_case(&want)))
    };
    let pack = from_parts(&config, &fallback, &|name| match find(name) {
        Some(e) if e.data.len() as u64 > MAX_FILE => Err(format!("{name} is too big ({} MB)", e.data.len() / 1024 / 1024)),
        Some(e) => Ok(e.data.clone()),
        None => Err(format!("the pack's sound file {name} is missing from the .zip")),
    })
    .map_err(|e| format!("This isn't a usable Mechvibes pack: {e}"))?;
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for f in referenced_files(&config)? {
        // a file that is named but not shipped only matters when one of the five sounds needs it (from_parts said so above)
        if let Some(e) = find(&f) {
            files.push((f, e.data.clone()));
        }
    }
    Ok((pack, config, files))
}

/// Writes a decoded pack into `root\<name>` (a free folder name; a failure removes what was written).
fn put(mut pack: Imported, config: &str, files: &[(String, Vec<u8>)], root: &Path) -> Result<Imported, String> {
    std::fs::create_dir_all(root).map_err(|e| format!("the pack folder: {e}"))?;
    let base = folder_name(&pack.name);
    let mut name = base.clone();
    let mut n = 2;
    while root.join(&name).exists() {
        name = format!("{base} {n}");
        n += 1;
    }
    let dest = root.join(&name);
    std::fs::create_dir_all(&dest).map_err(|e| format!("the pack folder: {e}"))?;
    let write = || -> Result<(), String> {
        std::fs::write(dest.join("config.json"), config.as_bytes()).map_err(|e| format!("config.json: {e}"))?;
        for (f, data) in files {
            std::fs::write(dest.join(f), data).map_err(|e| format!("{f}: {e}"))?;
        }
        Ok(())
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(e);
    }
    pack.name = name;
    Ok(pack)
}

/// The installed packs' names (folders of `root` that hold a config.json), sorted.
pub fn installed(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root)
        .map(|d| d.filter_map(|e| e.ok()).filter(|e| e.path().join("config.json").is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    v.sort_by_key(|s| s.to_lowercase());
    v
}

/// One installed pack (its name is the folder's).
pub fn read_installed(root: &Path, name: &str) -> Result<Imported, String> {
    if folder_name(name) != name {
        return Err("not a pack name".into());
    }
    let mut p = read_pack(&root.join(name))?;
    p.name = name.to_string();
    Ok(p)
}

/// Removes an installed pack's folder.
pub fn remove(root: &Path, name: &str) -> Result<(), String> {
    if folder_name(name) != name || !root.join(name).join("config.json").is_file() {
        return Err("no such pack".into());
    }
    std::fs::remove_dir_all(root.join(name)).map_err(|e| format!("removing {name}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 16-bit mono WAV: a decaying 440 Hz tone, `ms` long.
    fn wav(rate: u32, ms: u32, ch: u16) -> Vec<u8> {
        let n = (rate * ms / 1000) as usize;
        let mut pcm = Vec::new();
        for i in 0..n {
            let t = i as f32 / rate as f32;
            let s = ((t * 440.0 * std::f32::consts::TAU).sin() * (-t * 6.0).exp() * 20_000.0) as i16;
            for _ in 0..ch {
                pcm.extend_from_slice(&s.to_le_bytes());
            }
        }
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&ch.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * 2 * ch as u32).to_le_bytes());
        v.extend_from_slice(&(2 * ch).to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        v.extend_from_slice(&pcm);
        v
    }

    fn files(name: &str) -> Result<Vec<u8>, String> {
        match name {
            "sound.wav" => Ok(wav(44_100, 1000, 2)),
            "a.wav" | "space.wav" | "enter.wav" | "bs.wav" | "a-up.wav" => Ok(wav(48_000, 120, 1)),
            _ => Err(format!("missing {name}")),
        }
    }

    #[test]
    fn a_single_file_pack_is_sliced_into_the_five_sounds() {
        let cfg = r#"{"id":"x","name":"Test Switch","key_define_type":"single","sound":"sound.wav",
            "defines":{"1":[0,50],"30":[100,80],"30-up":[300,40],"57":[400,200],"28":[700,150]}}"#;
        let p = from_parts(cfg, "fallback", &files).unwrap();
        assert_eq!(p.name, "Test Switch");
        assert_eq!(p.set.rate, 44_100);
        let len = |k: Kind| p.set.get(k).len();
        let near = |a: usize, b: usize| (a as i64 - b as i64).abs() <= 1; // the start / end are rounded to whole samples
        assert!(near(len(Kind::Down), 3528), "the A key (30) is the plain down sound");
        assert!(near(len(Kind::Up), 1764));
        assert!(near(len(Kind::Space), 8820));
        assert!(near(len(Kind::Enter), 6615));
        assert_eq!(len(Kind::Backspace), len(Kind::Down), "no Backspace (14) define: the plain sound");
        let peak = p.set.get(Kind::Down).iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!((peak - PEAK).abs() < 0.01, "loudness is normalised like our own packs: {peak}");
        assert!(p.set.get(Kind::Down)[0].abs() < 0.05, "a soft start, no click");
    }

    #[test]
    fn a_multi_file_pack_is_read_and_resampled_to_one_rate() {
        let cfg = r#"{"name":"Multi","key_define_type":"multi","defines":
            {"30":"a.wav","30-up":"a-up.wav","57":"space.wav","28":"enter.wav","14":"bs.wav"}}"#;
        let p = from_parts(cfg, "f", &files).unwrap();
        assert_eq!(p.set.rate, 48_000);
        assert!(p.set.sounds.iter().all(|s| s.len() > 1000));
    }

    #[test]
    fn a_pack_without_a_name_takes_its_folders() {
        let cfg = r#"{"sound":"sound.wav","defines":{"2":[0,100]}}"#;
        let p = from_parts(cfg, "My Folder", &files).unwrap();
        assert_eq!(p.name, "My Folder");
        assert_eq!(p.set.get(Kind::Down).len(), p.set.get(Kind::Space).len(), "only one key defined: it is every sound");
    }


    #[test]
    fn real_world_packs_with_empty_defines_and_a_stray_sound_entry_import() {
        // from the mechvibes.com list (Order 061): "" / null defines, and a multi pack naming a "sound" file it doesn't ship
        let cfg = r#"{"name":"Gappy","key_define_type":"multi","sound":"sound.ogg","defines":
            {"1":"","2":null,"30":"a.wav","30-up":"a-up.wav","57":"space.wav","28":"enter.wav","14":"bs.wav"}}"#;
        let p = from_parts(cfg, "f", &files).unwrap();
        assert_eq!(p.name, "Gappy");
        let names = referenced_files(cfg).unwrap();
        assert!(!names.iter().any(|n| n.is_empty() || n == "sound.ogg"), "{names:?}");
        // a single pack still needs its sound file
        assert_eq!(referenced_files(r#"{"sound":"sound.wav","defines":{"30":[0,50]}}"#).unwrap(), vec!["sound.wav"]);
        // a pack that defines nothing but empty names has no keys
        assert!(from_parts(r#"{"key_define_type":"multi","defines":{"30":""}}"#, "f", &files).unwrap_err().contains("no keys"));
    }
    #[test]
    fn bad_packs_are_refused_with_a_reason() {
        let e = |cfg: &str| from_parts(cfg, "f", &files).unwrap_err();
        assert!(e("nope").contains("JSON"));
        assert!(e(r#"{"sound":"sound.wav"}"#).contains("defines"));
        assert!(e(r#"{"sound":"sound.wav","defines":{}}"#).contains("no keys"));
        assert!(e(r#"{"sound":"../sound.wav","defines":{"30":[0,50]}}"#).contains("plain file name"));
        assert!(e(r#"{"sound":"C:\\x.wav","defines":{"30":[0,50]}}"#).contains("plain file name"));
        assert!(e(r#"{"sound":"gone.wav","defines":{"30":[0,50]}}"#).contains("missing"));
        assert!(e(r#"{"sound":"sound.wav","defines":{"30":[99999,50]}}"#).contains("after the end"));
        assert!(e(r#"{"sound":"sound.wav","defines":{"30":[0,-5]}}"#).contains("bad start"));
        assert!(e(r#"{"key_define_type":"multi","defines":{"30":"../x.wav"}}"#).contains("plain file name"));
    }

    #[test]
    fn unsupported_audio_says_so() {
        let mp3 = |_: &str| Ok(b"ID3\x03\x00".to_vec());
        let err = from_parts(r#"{"sound":"s.mp3","defines":{"30":[0,50]}}"#, "f", &mp3).unwrap_err();
        assert!(err.contains("MP3"), "{err}");
        let junk = |_: &str| Ok(vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        assert!(from_parts(r#"{"sound":"s.bin","defines":{"30":[0,50]}}"#, "f", &junk).unwrap_err().contains("unsupported"));
        let ogg = |_: &str| Ok(b"OggS this is not a real stream".to_vec());
        assert!(from_parts(r#"{"sound":"s.ogg","defines":{"30":[0,50]}}"#, "f", &ogg).unwrap_err().contains("Ogg"));
    }

    #[test]
    fn a_pack_is_installed_listed_read_and_removed() {
        let base = std::env::temp_dir().join(format!("bu-keysound-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (src, root) = (base.join("src"), base.join("root"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("config.json"), r#"{"name":"Cherry: Blue/Clicky","sound":"sound.wav","defines":{"30":[0,60],"57":[100,90]}}"#).unwrap();
        std::fs::write(src.join("sound.wav"), wav(44_100, 500, 1)).unwrap();
        std::fs::write(src.join("readme.txt"), "not copied").unwrap();
        let p = install(&src, &root).unwrap();
        assert_eq!(p.name, "Cherry Blue Clicky", "a folder-safe name");
        assert_eq!(installed(&root), vec!["Cherry Blue Clicky".to_string()]);
        assert!(root.join("Cherry Blue Clicky").join("sound.wav").is_file());
        assert!(!root.join("Cherry Blue Clicky").join("readme.txt").exists(), "only what the config names is copied");
        // the same pack again does not replace the first one
        assert_eq!(install(&src, &root).unwrap().name, "Cherry Blue Clicky 2");
        assert_eq!(read_installed(&root, "Cherry Blue Clicky 2").unwrap().set.rate, 44_100);
        assert!(read_installed(&root, "..").is_err());
        assert!(remove(&root, "..").is_err());
        remove(&root, "Cherry Blue Clicky").unwrap();
        assert_eq!(installed(&root), vec!["Cherry Blue Clicky 2".to_string()]);
        // a pack that can't be read changes nothing
        std::fs::write(src.join("sound.wav"), b"junk junk junk junk").unwrap();
        assert!(install(&src, &root).is_err());
        assert_eq!(installed(&root).len(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn folder_names_are_safe() {
        assert_eq!(folder_name("Holy Panda"), "Holy Panda");
        assert_eq!(folder_name("a/b\\c:d*e"), "a b c d e");
        assert_eq!(folder_name("..."), "Imported");
        assert_eq!(folder_name("CON"), "pack CON");
        assert_eq!(folder_name("com1"), "pack com1");
        assert_eq!(folder_name(&"x".repeat(100)).len(), 60);
        assert_eq!(referenced_files(r#"{"sound":"s.ogg","defines":{"1":[0,5]}}"#).unwrap(), vec!["s.ogg"]);
        assert_eq!(referenced_files(r#"{"defines":{"1":"a.wav","2":"a.wav","3":"b.wav"}}"#).unwrap(), vec!["a.wav", "b.wav"]);
        assert!(referenced_files(r#"{"sound":"../s.ogg"}"#).is_err());
    }

    #[test]
    fn a_folder_is_read_from_disk() {
        let dir = std::env::temp_dir().join(format!("bu-keysound-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.json"), r#"{"name":"Disk","sound":"sound.wav","defines":{"30":[0,60],"57":[100,90]}}"#).unwrap();
        std::fs::write(dir.join("sound.wav"), wav(44_100, 500, 1)).unwrap();
        let p = read_pack(&dir).unwrap();
        assert_eq!(p.name, "Disk");
        assert!(read_pack(&dir.join("nothing-here")).unwrap_err().contains("config.json"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Order 059: the .zip as the Mechvibes site hands it out installs - config at the top, inside one folder, deflated or
    /// stored - and a zip / file / folder that isn't a pack says so in one plain line.
    #[test]
    fn a_downloaded_zip_is_imported_as_it_is() {
        let base = std::env::temp_dir().join(format!("bu-keysound-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let root = base.join("root");
        let cfg = br#"{"name":"Zipped Switch","sound":"sound.wav","defines":{"30":[0,60],"57":[100,90]}}"#;
        let snd = wav(44_100, 500, 1);
        // top level, deflated
        let z = bu_addons::zip::build(&[("config.json", &cfg[..]), ("sound.wav", &snd[..]), ("readme.txt", &b"x"[..])], true);
        // Order 076: hearing a pack first reads it in memory and writes nothing
        assert_eq!(preview_zip(&z, "pack").unwrap().name, "Zipped Switch");
        assert!(!root.exists(), "a preview keeps nothing");
        let p = install_zip(&z, "pack", &root).unwrap();
        assert_eq!(p.name, "Zipped Switch");
        assert!(root.join("Zipped Switch").join("sound.wav").is_file());
        assert!(!root.join("Zipped Switch").join("readme.txt").exists());
        // inside one folder, stored, from a file on disk (the way the page calls it)
        let z = bu_addons::zip::build(&[("My Pack/config.json", &cfg[..]), ("My Pack/sound.wav", &snd[..])], false);
        let zf = base.join("download.zip");
        std::fs::write(&zf, &z).unwrap();
        assert_eq!(install_any(&zf, &root).unwrap().name, "Zipped Switch 2");
        // a folder with the pack inside one folder (Explorer's "extract all")
        let ex = base.join("extracted");
        std::fs::create_dir_all(ex.join("inner")).unwrap();
        std::fs::write(ex.join("inner").join("config.json"), cfg).unwrap();
        std::fs::write(ex.join("inner").join("sound.wav"), &snd).unwrap();
        assert_eq!(install_any(&ex, &root).unwrap().name, "Zipped Switch 3");
        // Order 076: the one picker takes a pack folder by its config.json
        assert_eq!(install_any(&ex.join("inner").join("config.json"), &root).unwrap().name, "Zipped Switch 4");
        // not packs: each says why, nothing is installed
        let err = |r: Result<Imported, String>| r.unwrap_err();
        let nocfg = bu_addons::zip::build(&[("a.txt", &b"hi"[..])], true);
        assert!(err(install_zip(&nocfg, "p", &root)).contains("no config.json"));
        assert!(err(install_zip(b"this is not a zip at all", "p", &root)).contains("isn't a Mechvibes pack"));
        let nosound = bu_addons::zip::build(&[("config.json", &cfg[..])], true);
        assert!(err(install_zip(&nosound, "p", &root)).contains("missing"));
        let txt = base.join("notes.txt");
        std::fs::write(&txt, "hello").unwrap();
        assert!(err(install_any(&txt, &root)).contains("isn't a Mechvibes pack"));
        assert!(err(install_any(&base.join("empty-folder-that-is-not-there"), &root)).contains("can't read"));
        assert_eq!(installed(&root).len(), 4);
        let _ = std::fs::remove_dir_all(&base);
    }
}
