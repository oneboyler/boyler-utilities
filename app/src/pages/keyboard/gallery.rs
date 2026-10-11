//! "Get more sounds" (Order 061): the data and the jobs behind the list of community packs from mechvibes.com. The page-free
//! parts (reading the site, the cache) are `bu_keysound::gallery`; this file is the network (WinHTTP), the shared state the
//! page paints from, and the two jobs. A job starts only from a button (the job runner's rule): "Get more sounds…" lists,
//! and a row's "Get" downloads one pack. Nothing here runs while the window is closed.

use bu_keysound::gallery::{self, Cache, Fetch, Listed};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub const JOB_LIST: &str = "kbd.gallery";
pub const JOB_GET: &str = "kbd.get";
pub const JOB_PREVIEW: &str = "kbd.gprev";

/// What the window paints.
#[derive(Clone, Debug, Default)]
pub struct Gal {
    pub list: Vec<Listed>,
    pub sizes: BTreeMap<String, u64>,
    /// site id -> the pack folder it was installed as
    pub installed: BTreeMap<String, String>,
    /// the list is being fetched / sizes are being looked up (the job runs)
    pub busy: bool,
    pub status: String,
    /// a plain line when the site could not be read (the saved list, if any, is still shown)
    pub error: Option<String>,
    /// the pack being downloaded (site id) and its progress text
    pub getting: Option<(String, String)>,
    /// the pack being fetched to be heard (site id): its play button waits
    pub previewing: Option<String>,
}

static GAL: Mutex<Option<Gal>> = Mutex::new(None);

pub fn snapshot() -> Gal {
    GAL.lock().ok().and_then(|g| g.clone()).unwrap_or_default()
}

fn update(f: impl FnOnce(&mut Gal)) {
    if let Ok(mut g) = GAL.lock() {
        f(g.get_or_insert_with(Gal::default));
    }
}

/// The window is open: the size lookups stop when it is closed.
pub static OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn reset() {
    if let Ok(mut g) = GAL.lock() {
        *g = None;
    }
}

/// Test copies: three made-up packs, nothing from the network.
pub fn fixture() {
    let p = |id: &str, name: &str, tags: &[&str]| Listed { id: id.into(), name: name.into(), tags: tags.iter().map(|t| t.to_string()).collect(), pre_installed: false };
    // one assignment: tests running side by side all set the same thing
    update(|g| {
        *g = Gal {
            list: vec![p("custom-sound-pack-1", "Model F XT", &["keyboard", "retro"]), p("custom-sound-pack-2", "Typewriter", &["retro"]), p("custom-sound-pack-3", "Bubble pop", &["fun"])],
            sizes: [("custom-sound-pack-1".to_string(), 858_858), ("custom-sound-pack-2".to_string(), 2_400_000)].into_iter().collect(),
            installed: [("custom-sound-pack-3".to_string(), "Bubble pop".to_string())].into_iter().collect(),
            ..Gal::default()
        };
    });
}

/// The real network: Windows' own WinHTTP (system certificates, system proxy), only for mechvibes.com URLs.
pub struct WinFetch(bu_updater::WinHttp);

impl WinFetch {
    pub fn new() -> Self {
        WinFetch(bu_updater::WinHttp::new("BoylerUtilities"))
    }
}

impl Default for WinFetch {
    fn default() -> Self {
        Self::new()
    }
}

/// A sink that takes at most `max` bytes.
struct Cap {
    buf: Vec<u8>,
    max: usize,
}

impl std::io::Write for Cap {
    fn write(&mut self, d: &[u8]) -> std::io::Result<usize> {
        if self.buf.len() + d.len() > self.max {
            return Err(std::io::Error::other("bigger than expected"));
        }
        self.buf.extend_from_slice(d);
        Ok(d.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A sink that takes the first chunk and refuses the next one: the answer's announced length is known by then.
struct FirstChunk(bool);

impl std::io::Write for FirstChunk {
    fn write(&mut self, d: &[u8]) -> std::io::Result<usize> {
        if self.0 {
            return Err(std::io::Error::other("enough"));
        }
        self.0 = true;
        Ok(d.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Fetch for WinFetch {
    fn get(&self, url: &str, max: usize, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Vec<u8>, String> {
        use bu_updater::{Http, Request};
        let mut sink = Cap { buf: Vec::new(), max };
        let r = self.0.get(&Request { url, accept: "*/*" }, &mut sink, progress).map_err(|e| e.to_string())?;
        if !(200..300).contains(&r.status) {
            return Err(format!("HTTP {}", r.status));
        }
        Ok(sink.buf)
    }

    fn length(&self, url: &str) -> Result<Option<u64>, String> {
        use bu_updater::{Http, Request};
        let mut seen: Option<u64> = None;
        let mut sink = FirstChunk(false);
        let r = self.0.get(&Request { url, accept: "*/*" }, &mut sink, &mut |_, total| {
            if total.is_some() {
                seen = total;
            }
        });
        match r {
            Ok(r) if !(200..300).contains(&r.status) => Err(format!("HTTP {}", r.status)),
            Ok(r) => Ok(r.content_length.or(seen)),
            // refused on purpose after the first chunk: the length was announced before it
            Err(_) if seen.is_some() => Ok(seen),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// The saved list file (beside the pack folders).
pub fn cache_path(packs_dir: &std::path::Path) -> PathBuf {
    packs_dir.with_file_name("sound-gallery.json")
}

/// Job: read the site's list, then look up the size of every pack not seen before (found once per pack, ever).
pub fn list_job(cache_file: PathBuf, fetch: Arc<dyn Fetch + Send + Sync>) -> impl FnOnce(&crate::jobs::JobCtx) -> Result<String, crate::jobs::JobError> + Send + 'static {
    move |job| {
        let mut cache = Cache::load(&cache_file);
        update(|g| {
            g.busy = true;
            g.error = None;
            g.status = "Reading mechvibes.com…".into();
            g.list = cache.list.clone();
            g.sizes = cache.sizes.clone();
            g.installed = cache.installed.clone();
        });
        job.status("Reading mechvibes.com…");
        match gallery::fetch_list(&*fetch) {
            Ok(l) => {
                cache.list = l.clone();
                let _ = cache.save(&cache_file);
                update(|g| g.list = l);
            }
            Err(e) => {
                let had = !cache.list.is_empty();
                update(|g| {
                    g.busy = false;
                    g.error = Some(if had { format!("{e} - showing the list saved last time") } else { e.clone() });
                });
                return if had { Ok(String::new()) } else { Err(crate::jobs::JobError::Failed(e)) };
            }
        }
        let todo: Vec<Listed> = cache.list.iter().filter(|p| !cache.sizes.contains_key(&p.id)).cloned().collect();
        let n = todo.len();
        for (i, p) in todo.iter().enumerate() {
            if job.stopped() || !OPEN.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            update(|g| g.status = format!("Looking up sizes {}/{n}", i + 1));
            job.status("Looking up sizes");
            if let Ok(Some(sz)) = gallery::size_of(&*fetch, p) {
                cache.sizes.insert(p.id.clone(), sz);
                update(|g| {
                    g.sizes.insert(p.id.clone(), sz);
                });
                if i % 8 == 7 {
                    let _ = cache.save(&cache_file);
                }
            }
            // be gentle with a free community site
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        let _ = cache.save(&cache_file);
        update(|g| {
            g.busy = false;
            g.status.clear();
        });
        Ok(String::new())
    }
}

/// Job: download one pack and import it. The result (`Done`) is the pack folder's name.
pub fn get_job(
    p: Listed,
    cache_file: PathBuf,
    packs_dir: PathBuf,
    fetch: Arc<dyn Fetch + Send + Sync>,
) -> impl FnOnce(&crate::jobs::JobCtx) -> Result<String, crate::jobs::JobError> + Send + 'static {
    move |job| {
        let id = p.id.clone();
        let say = |t: &str| {
            let t = t.to_string();
            update(|g| g.getting = Some((id.clone(), t)));
        };
        say("Downloading…");
        job.status("Downloading…");
        let r = (|| -> Result<String, String> {
            let bytes = gallery::download(&*fetch, &p, &mut |done, total| {
                let t = match total {
                    Some(t) if t > 0 => {
                        job.progress(done as f32 / t as f32);
                        format!("{} of {}", gallery::size_text(done), gallery::size_text(t))
                    }
                    _ => gallery::size_text(done),
                };
                say(&t);
            })?;
            say("Importing…");
            // Order 090 (E21): decoded in the helper copy - a broken download can't crash the app
            let imported = bu_keysound::safe::install_zip(&bytes, &p.name, &packs_dir)?;
            Ok(imported.name)
        })();
        update(|g| g.getting = None);
        match r {
            Ok(name) => {
                let mut cache = Cache::load(&cache_file);
                cache.installed.insert(p.id.clone(), name.clone());
                let _ = cache.save(&cache_file);
                update(|g| {
                    g.installed.insert(p.id.clone(), name.clone());
                });
                Ok(name)
            }
            Err(e) => Err(crate::jobs::JobError::Failed(e)),
        }
    }
}

/// The name a pack has in the engine while it is heard before it is got (no folder can have it: `folder_name` never makes it).
const PREVIEW_NAME: &str = "\u{1}preview";

/// A few keys: (sound, wait before it in ms).
const PREVIEW_KEYS: [(bu_keysound::Kind, u64); 10] = [
    (bu_keysound::Kind::Down, 0),
    (bu_keysound::Kind::Up, 80),
    (bu_keysound::Kind::Down, 170),
    (bu_keysound::Kind::Up, 70),
    (bu_keysound::Kind::Down, 200),
    (bu_keysound::Kind::Up, 80),
    (bu_keysound::Kind::Space, 260),
    (bu_keysound::Kind::Up, 90),
    (bu_keysound::Kind::Enter, 320),
    (bu_keysound::Kind::Up, 90),
];

/// Job: the play button of a row. Fetches the pack in memory, plays a few keys of it through the engine and lets it go -
/// nothing is written to disk, nothing is kept. Works while the key sounds are off too (the engine then runs only for this,
/// listening to no key, and stops again unless the switch was turned on meanwhile).
pub fn preview_job(
    p: Listed,
    fetch: Arc<dyn Fetch + Send + Sync>,
    mut settings: bu_keysound::Settings,
) -> impl FnOnce(&crate::jobs::JobCtx) -> Result<String, crate::jobs::JobError> + Send + 'static {
    move |job| {
        // (cleared however the job ends, a panic included)
        struct Clear;
        impl Drop for Clear {
            fn drop(&mut self) {
                update(|g| g.previewing = None);
            }
        }
        update(|g| g.previewing = Some(p.id.clone()));
        let _clear = Clear;
        job.status("Loading the sound…");
        let r = (|| -> Result<(), String> {
            let bytes = gallery::download(&*fetch, &p, &mut |_, _| {})?;
            let heard = bu_keysound::safe::preview_zip(&bytes, &p.name)?;
            if job.stopped() || !OPEN.load(std::sync::atomic::Ordering::SeqCst) {
                return Ok(());
            }
            let play_on = settings.play_on;
            let e = super::glue::engine();
            let was_on = e.status().enabled;
            if !was_on {
                settings.mouse_on = false;
                settings.pad_on = false;
                e.enable_without_keys(settings)?;
            }
            e.set_imported(PREVIEW_NAME, Some(heard.set));
            let pack = bu_keysound::Pack::Imported(PREVIEW_NAME.to_string());
            // Order 090 (E21): the hear-first follows "Play on" (press only = no release sounds, release only = no press sounds)
            for (kind, wait) in PREVIEW_KEYS {
                if job.stopped() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(wait));
                if play_on.plays(kind != bu_keysound::Kind::Up) {
                    e.preview(&pack, kind);
                }
            }
            // let the last sound ring out, then let the pack go
            std::thread::sleep(std::time::Duration::from_millis(1200));
            e.set_imported(PREVIEW_NAME, None);
            if !was_on && !super::glue::sounds_on() {
                e.disable();
            }
            Ok(())
        })();

        r.map(|_| String::new()).map_err(crate::jobs::JobError::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Order 061 real-site proof (`cargo test -p bu-app real_site -- --ignored --nocapture`): the real list, two sizes, one real
    /// download + import into a scratch folder. Reads mechvibes.com only; writes only under the scratch folder.
    /// `real_site_all` below does it for EVERY pack of the list (once, to learn which import).
    #[test]
    #[ignore]
    fn real_site_all() {
        let f = WinFetch::new();
        let list = gallery::fetch_list(&f).unwrap();
        let scratch = std::path::PathBuf::from(r"C:\BoylerUtilities-scratch\F61\packs-all");
        let _ = std::fs::remove_dir_all(&scratch);
        let (mut good, mut bad, mut total) = (0, 0, 0usize);
        for p in &list {
            match gallery::download(&f, p, &mut |_, _| {}) {
                Err(e) => {
                    bad += 1;
                    println!("DOWNLOAD FAIL {} ({}): {e}", p.name, p.id);
                }
                Ok(b) => {
                    total += b.len();
                    match bu_keysound::import::install_zip(&b, &p.name, &scratch) {
                        Ok(_) => good += 1,
                        Err(e) => {
                            bad += 1;
                            println!("IMPORT FAIL {} ({}): {e}", p.name, p.id);
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        println!("{good} imported, {bad} failed, {} MB downloaded", total / 1048576);
    }

    #[test]
    #[ignore]
    fn real_site() {
        let f = WinFetch::new();
        let list = gallery::fetch_list(&f).unwrap();
        println!("{} packs", list.len());
        assert!(list.len() > 50);
        let small = list.iter().filter(|p| !p.pre_installed).take(2).cloned().collect::<Vec<_>>();
        for p in &small {
            println!("{} -> {:?}", p.name, gallery::size_of(&f, p));
        }
        let scratch = std::path::PathBuf::from(r"C:\BoylerUtilities-scratch\F61\packs");
        let _ = std::fs::remove_dir_all(&scratch);
        let p = &small[0];
        let bytes = gallery::download(&f, p, &mut |_, _| {}).unwrap();
        let imp = bu_keysound::import::install_zip(&bytes, &p.name, &scratch).unwrap();
        println!("downloaded {} bytes, imported as {:?}", bytes.len(), imp.name);
        assert!(bu_keysound::import::installed(&scratch).contains(&imp.name));
    }

    #[test]
    fn the_cache_file_sits_beside_the_pack_folders() {
        let p = cache_path(std::path::Path::new(r"C:\x\BoylerUtilities\keysound-packs"));
        assert_eq!(p, std::path::Path::new(r"C:\x\BoylerUtilities\sound-gallery.json"));
    }

    #[test]
    fn the_fixture_has_three_packs_one_installed() {
        fixture();
        let g = snapshot();
        assert_eq!(g.list.len(), 3);
        assert!(g.installed.contains_key("custom-sound-pack-3"));
    }
}
