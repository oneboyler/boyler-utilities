//! Order 090 (E21): a broken pack or sound file never crashes the app. Decoding a file someone else made (an imported or
//! downloaded pack, a sound of your own) runs a decoder over bytes we don't control; a bug in it panics, and the release build
//! ends the whole process on a panic (`panic = "abort"`), so catching it is impossible. Instead the decoding runs in a short
//! HELPER copy of the app (`<app exe> --bu-sound-decode …`, no window, no tray, the same user) that writes the decoded samples
//! to a temporary file and ends: a decoder that falls over only ends the helper, and the app says "this pack is damaged".
//!
//! What was decoded once is kept next to its source as a small cache (`.bu-sounds` in a pack's folder, `<file>.bu-pcm` beside
//! a sound of yours), read back by this module's own bounds-checked reader - so the helper runs once per pack / file, not at
//! every start. The cache carries the source's size + time and is ignored when the source changed.
//!
//! Without [`use_helper`] (the tests, tools) everything runs in-process.

use crate::import::{self, Clip, Imported};
use crate::kind::KINDS;
use crate::synth::SoundSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// The argument that starts the helper.
pub const ARG: &str = "--bu-sound-decode";
/// The longest a helper may take before it is ended (a few MB of audio takes well under a second).
const HELPER_TIMEOUT_MS: u64 = 30_000;
const MAGIC: &[u8; 6] = b"BUSND1";
/// The pack folder's cache file.
pub const PACK_CACHE: &str = ".bu-sounds";
/// A sound file's cache: `<file>` + this.
pub const SOUND_CACHE: &str = ".bu-pcm";

static HELPER: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Files that could not be decoded, with the stamp they had (Order 090 review: a broken pack is not decoded again at every
/// settings save - only once it changes). (path, stamp, why)
#[allow(clippy::type_complexity)]
static FAILED: Mutex<Vec<(PathBuf, (u64, u64), String)>> = Mutex::new(Vec::new());

fn failed_before(p: &Path, st: (u64, u64)) -> Option<String> {
    FAILED.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(q, s, _)| q == p && *s == st).map(|(_, _, w)| w.clone())
}

fn remember(p: &Path, st: (u64, u64), r: &Result<impl Sized, String>) {
    let mut f = FAILED.lock().unwrap_or_else(|e| e.into_inner());
    f.retain(|(q, _, _)| q != p);
    if let Err(e) = r {
        if f.len() >= 64 {
            f.remove(0);
        }
        f.push((p.to_path_buf(), st, e.clone()));
    }
}

/// The app turns the helper on at start with its own exe (tests leave it off: decoding is in-process there).
pub fn use_helper(exe: Option<PathBuf>) {
    *HELPER.lock().unwrap_or_else(|p| p.into_inner()) = exe;
}

fn helper() -> Option<PathBuf> {
    HELPER.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

// ------------------------------------------------------------------ the decoded-sounds file (helper output and the caches)

/// (name, rate, sounds) as bytes: `BUSND1`, source stamp (u64 len, u64 secs), name, rate, count, each sound's samples.
fn encode(stamp: (u64, u64), name: &str, rate: u32, sounds: &[&[f32]]) -> Vec<u8> {
    let mut v = Vec::with_capacity(64 + sounds.iter().map(|s| s.len() * 4 + 4).sum::<usize>());
    v.extend_from_slice(MAGIC);
    v.extend_from_slice(&stamp.0.to_le_bytes());
    v.extend_from_slice(&stamp.1.to_le_bytes());
    let nb = name.as_bytes();
    let nb = &nb[..nb.len().min(400)];
    v.extend_from_slice(&(nb.len() as u32).to_le_bytes());
    v.extend_from_slice(nb);
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(sounds.len() as u32).to_le_bytes());
    for s in sounds {
        v.extend_from_slice(&(s.len() as u32).to_le_bytes());
        for x in s.iter() {
            v.extend_from_slice(&x.to_le_bytes());
        }
    }
    v
}

/// The decoded file read back; None for anything malformed (never a panic: every length is checked against what is left).
#[allow(clippy::type_complexity)]
fn decode_file(b: &[u8]) -> Option<((u64, u64), String, u32, Vec<Vec<f32>>)> {
    let mut at = 0usize;
    let take = |at: &mut usize, n: usize| -> Option<&[u8]> {
        let end = at.checked_add(n)?;
        let s = b.get(*at..end)?;
        *at = end;
        Some(s)
    };
    if take(&mut at, 6)? != MAGIC {
        return None;
    }
    let u64_ = |at: &mut usize| -> Option<u64> { Some(u64::from_le_bytes(take(at, 8)?.try_into().ok()?)) };
    let u32_ = |at: &mut usize| -> Option<u32> { Some(u32::from_le_bytes(take(at, 4)?.try_into().ok()?)) };
    let stamp = (u64_(&mut at)?, u64_(&mut at)?);
    let nl = u32_(&mut at)? as usize;
    if nl > 400 {
        return None;
    }
    let name = String::from_utf8_lossy(take(&mut at, nl)?).into_owned();
    let rate = u32_(&mut at)?;
    let count = u32_(&mut at)? as usize;
    if count == 0 || count > 16 || !(8000..=192_000).contains(&rate) {
        return None;
    }
    let mut sounds = Vec::with_capacity(count);
    for _ in 0..count {
        let n = u32_(&mut at)? as usize;
        if n > 192_000 * 12 {
            return None;
        }
        let raw = take(&mut at, n.checked_mul(4)?)?;
        let s: Vec<f32> = raw.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).map(|x| if x.is_finite() { x.clamp(-1.0, 1.0) } else { 0.0 }).collect();
        sounds.push(s);
    }
    (at == b.len()).then_some((stamp, name, rate, sounds))
}

fn set_of(rate: u32, sounds: Vec<Vec<f32>>) -> Option<SoundSet> {
    if sounds.len() != KINDS {
        return None;
    }
    let mut it = sounds.into_iter().map(|v| if v.len() < 2 { vec![0.0; 2] } else { v }).map(|v| Arc::<[f32]>::from(v.into_boxed_slice()));
    Some(SoundSet { rate, sounds: std::array::from_fn(|_| it.next().expect("five sounds")) })
}

fn pack_bytes(stamp: (u64, u64), p: &Imported) -> Vec<u8> {
    let s: Vec<&[f32]> = p.set.sounds.iter().map(|a| &a[..]).collect();
    encode(stamp, &p.name, p.set.rate, &s)
}

fn pack_from(b: &[u8]) -> Option<((u64, u64), Imported)> {
    let (stamp, name, rate, sounds) = decode_file(b)?;
    Some((stamp, Imported { name, set: set_of(rate, sounds)? }))
}

fn clip_from(b: &[u8]) -> Option<((u64, u64), Clip)> {
    let (stamp, _, rate, mut sounds) = decode_file(b)?;
    (sounds.len() == 1 && sounds[0].len() >= 2).then(|| (stamp, Clip { rate, mono: Arc::from(sounds.remove(0).into_boxed_slice()) }))
}

/// A file's (size, modified seconds) - the cache is valid for exactly this.
fn stamp(p: &Path) -> (u64, u64) {
    let m = std::fs::metadata(p).ok();
    let len = m.as_ref().map(|m| m.len()).unwrap_or(0);
    let t = m.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    (len, t)
}

// ------------------------------------------------------------------ the helper

/// What the helper is asked to do.
enum Op<'a> {
    /// A pack folder (`read_pack`).
    Pack(&'a Path),
    /// Import from a folder / config.json / .zip into `root`.
    Install(&'a Path, &'a Path),
    /// A .zip (a file) heard first, nothing written: (zip, fallback name).
    Zip(&'a Path, &'a str),
    /// A .zip (a file) installed into `root`: (zip, fallback, root).
    InstallZip(&'a Path, &'a str, &'a Path),
    /// One sound file of yours.
    Sound(&'a Path),
}

/// Does `op` here, in this process (the helper's side, and everything when no helper is set).
fn run_here(op: &Op) -> Result<Vec<u8>, String> {
    match op {
        Op::Pack(dir) => import::read_pack(dir).map(|p| pack_bytes((0, 0), &p)),
        Op::Install(src, root) => import::install_any(src, root).map(|p| pack_bytes((0, 0), &p)),
        Op::Zip(zip, fb) => {
            let b = std::fs::read(zip).map_err(|e| format!("can't read the .zip: {e}"))?;
            import::preview_zip(&b, fb).map(|p| pack_bytes((0, 0), &p))
        }
        Op::InstallZip(zip, fb, root) => {
            let b = std::fs::read(zip).map_err(|e| format!("can't read the .zip: {e}"))?;
            import::install_zip(&b, fb, root).map(|p| pack_bytes((0, 0), &p))
        }
        Op::Sound(p) => import::read_sound_file(p).map(|c| encode((0, 0), "", c.rate, &[&c.mono[..]])),
    }
}

static SEQ: AtomicU32 = AtomicU32::new(0);

/// Does `op` in the helper when one is set (else here). Err: the decoder's own sentence, or "damaged" when the helper ended
/// without one (it fell over) or took too long.
fn run(op: &Op) -> Result<Vec<u8>, String> {
    let Some(exe) = helper() else { return run_here(op) };
    let out = std::env::temp_dir().join(format!("bu-sound-{}-{}.bin", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    let mut args: Vec<std::ffi::OsString> = vec![ARG.into()];
    match op {
        Op::Pack(d) => args.extend(["pack".into(), d.as_os_str().into()]),
        Op::Install(s, r) => args.extend(["install".into(), s.as_os_str().into(), r.as_os_str().into()]),
        Op::Zip(z, f) => args.extend(["zip".into(), z.as_os_str().into(), (*f).into()]),
        Op::InstallZip(z, f, r) => args.extend(["installzip".into(), z.as_os_str().into(), (*f).into(), r.as_os_str().into()]),
        Op::Sound(p) => args.extend(["sound".into(), p.as_os_str().into()]),
    }
    args.push(out.as_os_str().into());
    let res = spawn_wait(&exe, &args);
    let bytes = std::fs::read(&out).ok();
    let _ = std::fs::remove_file(&out);
    match (res, bytes) {
        (Ok(0), Some(b)) => Ok(b),
        (Ok(2), Some(b)) => Err(String::from_utf8_lossy(&b).into_owned()),
        (Err(e), _) => Err(e),
        _ => Err("the file is damaged: reading it failed".into()),
    }
}

#[cfg(windows)]
fn spawn_wait(exe: &Path, args: &[std::ffi::OsString]) -> Result<i32, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut child = std::process::Command::new(exe)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("the sound reader couldn't start: {e}"))?;
    let t0 = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return Ok(st.code().unwrap_or(-1)),
            Ok(None) if t0.elapsed().as_millis() as u64 > HELPER_TIMEOUT_MS => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("the file is damaged: reading it never finished".into());
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(15)),
            Err(e) => return Err(format!("the sound reader: {e}")),
        }
    }
}

#[cfg(not(windows))]
fn spawn_wait(exe: &Path, args: &[std::ffi::OsString]) -> Result<i32, String> {
    std::process::Command::new(exe).args(args).status().map(|s| s.code().unwrap_or(-1)).map_err(|e| e.to_string())
}

/// The helper's side: `<exe> --bu-sound-decode <op> <args…> <out>` does the op, writes the result (or the error sentence) to
/// `<out>` and gives the exit code (0 = done, 2 = refused with a sentence). None when the app was not started as the helper.
pub fn run_if_requested(args: &[String]) -> Option<i32> {
    let i = args.iter().position(|a| a == ARG)?;
    let a: Vec<&str> = args[i + 1..].iter().map(String::as_str).collect();
    let (op, out) = match a.as_slice() {
        ["pack", d, out] => (Op::Pack(Path::new(d)), *out),
        ["install", s, r, out] => (Op::Install(Path::new(s), Path::new(r)), *out),
        ["zip", z, f, out] => (Op::Zip(Path::new(z), f), *out),
        ["installzip", z, f, r, out] => (Op::InstallZip(Path::new(z), f, Path::new(r)), *out),
        ["sound", p, out] => (Op::Sound(Path::new(p)), *out),
        _ => return Some(3),
    };
    Some(match run_here(&op) {
        Ok(b) => {
            if std::fs::write(out, b).is_ok() {
                0
            } else {
                3
            }
        }
        Err(e) => {
            let _ = std::fs::write(out, e.as_bytes());
            2
        }
    })
}

// ------------------------------------------------------------------ what the app calls

/// One installed pack (its name is the folder's): from the cache when it is current, else decoded safely (and cached).
pub fn read_installed(root: &Path, name: &str) -> Result<Imported, String> {
    if import::folder_name(name) != name {
        return Err("not a pack name".into());
    }
    let dir = root.join(name);
    let cfg = dir.join("config.json");
    if !cfg.is_file() {
        return Err("this pack's folder has no config.json".into());
    }
    let st = stamp(&cfg);
    let cache = dir.join(PACK_CACHE);
    if let Some((s, mut p)) = std::fs::read(&cache).ok().as_deref().and_then(pack_from) {
        if s == st {
            p.name = name.to_string();
            return Ok(p);
        }
    }
    if let Some(e) = failed_before(&cfg, st) {
        return Err(e);
    }
    let r = run(&Op::Pack(&dir)).and_then(|b| pack_from(&b).map(|x| x.1).ok_or_else(|| "the pack is damaged: its sounds came back unreadable".to_string()));
    remember(&cfg, st, &r);
    let mut p = r?;
    p.name = name.to_string();
    let _ = std::fs::write(&cache, pack_bytes(st, &p));
    Ok(p)
}

/// Writes the cache of a pack just installed in `root\<name>` (so its first use needs no helper).
fn cache_installed(root: &Path, p: &Imported) {
    let dir = root.join(&p.name);
    let st = stamp(&dir.join("config.json"));
    let _ = std::fs::write(dir.join(PACK_CACHE), pack_bytes(st, p));
}

fn installed_from(b: &[u8], root: &Path) -> Result<Imported, String> {
    let (_, p) = pack_from(b).ok_or("the pack is damaged: its sounds came back unreadable")?;
    cache_installed(root, &p);
    Ok(p)
}

/// "Import a pack…": a pack folder, its config.json or the .zip (see `import::install_any`), decoded safely.
pub fn install_any(src: &Path, root: &Path) -> Result<Imported, String> {
    let b = run(&Op::Install(src, root))?;
    installed_from(&b, root)
}

/// A temporary copy of an in-memory .zip for the helper (removed when dropped).
struct TempZip(PathBuf);

impl TempZip {
    fn new(zip: &[u8]) -> Result<TempZip, String> {
        let p = std::env::temp_dir().join(format!("bu-pack-{}-{}.zip", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&p, zip).map_err(|e| format!("can't keep the download: {e}"))?;
        Ok(TempZip(p))
    }
}

impl Drop for TempZip {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A downloaded .zip installed into `root`, decoded safely.
pub fn install_zip(zip: &[u8], fallback: &str, root: &Path) -> Result<Imported, String> {
    if helper().is_none() {
        return import::install_zip(zip, fallback, root).inspect(|p| cache_installed(root, p));
    }
    let t = TempZip::new(zip)?;
    let b = run(&Op::InstallZip(&t.0, fallback, root))?;
    installed_from(&b, root)
}

/// A downloaded .zip heard first (nothing written), decoded safely.
pub fn preview_zip(zip: &[u8], fallback: &str) -> Result<Imported, String> {
    if helper().is_none() {
        return import::preview_zip(zip, fallback);
    }
    let t = TempZip::new(zip)?;
    let b = run(&Op::Zip(&t.0, fallback))?;
    pack_from(&b).map(|x| x.1).ok_or_else(|| "the pack is damaged: its sounds came back unreadable".into())
}

/// One sound file of yours: from its cache (`<file>.bu-pcm`) when current, else decoded safely (and cached when `cache`).
pub fn read_sound(path: &Path, cache: bool) -> Result<Clip, String> {
    let st = stamp(path);
    let cpath = PathBuf::from(format!("{}{SOUND_CACHE}", path.display()));
    if cache {
        if let Some((s, c)) = std::fs::read(&cpath).ok().as_deref().and_then(clip_from) {
            if s == st {
                return Ok(c);
            }
        }
    }
    if let Some(e) = failed_before(path, st) {
        return Err(e);
    }
    let r = run(&Op::Sound(path)).and_then(|b| clip_from(&b).map(|x| x.1).ok_or_else(|| "the sound came back unreadable".to_string()));
    remember(path, st, &r);
    let c = r?;
    if cache {
        let _ = std::fs::write(&cpath, encode(st, "", c.rate, &[&c.mono[..]]));
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bu-safe-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A 16-bit mono WAV, a decaying tone `ms` long.
    fn wav(rate: u32, ms: u32) -> Vec<u8> {
        let n = (rate * ms / 1000) as usize;
        let mut d = Vec::new();
        for i in 0..n {
            let t = i as f32 / rate as f32;
            let s = ((t * 440.0 * std::f32::consts::TAU).sin() * (-t * 20.0).exp() * 20000.0) as i16;
            d.extend_from_slice(&s.to_le_bytes());
        }
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + d.len() as u32).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * 2).to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(d.len() as u32).to_le_bytes());
        v.extend_from_slice(&d);
        v
    }

    #[test]
    fn the_file_format_round_trips_and_rejects_every_truncation() {
        let b = encode((5, 6), "Holy Panda", 48_000, &[&[0.1, -0.2, 0.3], &[0.5, 0.5]]);
        let (st, name, rate, s) = decode_file(&b).unwrap();
        assert_eq!((st, name.as_str(), rate), ((5, 6), "Holy Panda", 48_000));
        assert_eq!(s, vec![vec![0.1, -0.2, 0.3], vec![0.5, 0.5]]);
        for cut in 0..b.len() {
            assert!(decode_file(&b[..cut]).is_none(), "a file cut at {cut} is refused, never a panic");
        }
        let mut longer = b.clone();
        longer.push(0);
        assert!(decode_file(&longer).is_none());
        // garbage of every kind: refused
        let mut x = 7u32;
        for _ in 0..500 {
            let mut g = MAGIC.to_vec();
            for _ in 0..(x % 90) {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                g.push(x as u8);
            }
            let _ = decode_file(&g);
        }
    }

    #[test]
    fn a_sound_of_yours_is_read_and_cached_and_the_cache_follows_the_file() {
        let d = scratch("sound");
        let f = d.join("boing.wav");
        std::fs::write(&f, wav(44_100, 200)).unwrap();
        let c = read_sound(&f, true).unwrap();
        assert_eq!(c.rate, 44_100);
        assert!(c.mono.len() > 8000);
        let peak = c.mono.iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!((peak - crate::synth::PEAK).abs() < 0.01, "as loud as a pack's sound");
        let cache = PathBuf::from(format!("{}{SOUND_CACHE}", f.display()));
        assert!(cache.is_file(), "decoded once, kept beside the file");
        assert_eq!(read_sound(&f, true).unwrap(), c, "read back from the cache");
        // a damaged cache is ignored
        std::fs::write(&cache, b"BUSND1junk").unwrap();
        assert_eq!(read_sound(&f, true).unwrap(), c);
        // a file that isn't a sound says so
        let bad = d.join("notes.txt");
        std::fs::write(&bad, b"hello").unwrap();
        assert!(read_sound(&bad, false).is_err());
        let silent = d.join("silent.wav");
        let mut s = wav(8000, 50);
        let len = s.len();
        s[44..len].fill(0);
        std::fs::write(&silent, s).unwrap();
        assert!(read_sound(&silent, false).unwrap_err().contains("silence"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_installed_pack_is_cached_and_a_bad_one_says_why() {
        let root = scratch("packs");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("s.wav"), wav(22_050, 300)).unwrap();
        std::fs::write(src.join("config.json"), r#"{"name":"Tiny","sound":"s.wav","defines":{"30":[0,80],"57":[100,80]}}"#).unwrap();
        let inst = root.join("installed");
        let p = install_any(&src, &inst).unwrap();
        assert_eq!(p.name, "Tiny");
        assert!(inst.join("Tiny").join(PACK_CACHE).is_file(), "installing writes the cache");
        let again = read_installed(&inst, "Tiny").unwrap();
        assert_eq!(again.set.rate, p.set.rate);
        assert_eq!(&again.set.sounds[0][..], &p.set.sounds[0][..]);
        // damage the pack's audio: the (still current) cache answers; once config.json changes, the damage shows as an error
        std::fs::write(inst.join("Tiny").join("s.wav"), b"RIFF\x10\x00\x00\x00WAVEjunk").unwrap();
        assert!(read_installed(&inst, "Tiny").is_ok());
        std::fs::write(inst.join("Tiny").join("config.json"), r#"{"name":"Tiny","sound":"s.wav","defines":{"30":[0,81]}}"#).unwrap();
        assert!(read_installed(&inst, "Tiny").is_err());
        assert!(read_installed(&inst, "..\\x").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_helper_arguments_are_understood_and_junk_is_refused() {
        assert_eq!(run_if_requested(&["app.exe".into(), "--open".into()]), None);
        assert_eq!(run_if_requested(&["app.exe".into(), ARG.into(), "nonsense".into()]), Some(3));
        let d = scratch("helper");
        let f = d.join("a.wav");
        std::fs::write(&f, wav(16_000, 100)).unwrap();
        let out = d.join("out.bin");
        let code = run_if_requested(&["x".into(), ARG.into(), "sound".into(), f.display().to_string(), out.display().to_string()]);
        assert_eq!(code, Some(0));
        assert!(clip_from(&std::fs::read(&out).unwrap()).is_some());
        let code = run_if_requested(&["x".into(), ARG.into(), "sound".into(), d.join("none.wav").display().to_string(), out.display().to_string()]);
        assert_eq!(code, Some(2), "refused with a sentence");
        assert!(!std::fs::read_to_string(&out).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
