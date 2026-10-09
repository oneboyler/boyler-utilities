//! "Get more cursors" (Order 066) - the window of the Mouse tab's cursor pickers that lists cursor packs with an open licence from
//! their official GitHub release pages, each with its own real arrow / link hand / text beam, who made it, its licence and size;
//! one click downloads it and adds it as a cursor scheme (this Windows user only, no admin). Like Keyboard's "Get more sounds".
//! The list and the install are `bu_mouse::store`; this file is the network (WinHTTP), the shared state the window paints from,
//! the two jobs and the window. Nothing runs while the window is closed; a job starts only from a button.

use super::*;
use crate::ui::pieces::button::{self, Kind as BKind};
use crate::ui::pieces::dialog;
use bu_mouse::curfile::{self, CursorImage};
use bu_mouse::store::{self as site, Fetch, Listing};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub const JOB_PREVIEW: &str = "cur.store.preview";
pub const JOB_GET: &str = "cur.store.get";
const K_GET: Key = key("cur.get");
pub(super) const K_GROW: Key = key("cur.grow");
pub(super) const K_GDONE: Key = key("cur.gdone");

const NOTE: &str = "These cursor packs are made by the community and shared on GitHub under open licences (GPL-3.0). Boyler Utilities doesn't host or copy them: a click downloads the pack from its official release page to your PC and adds it as a cursor scheme for you only - no admin. The licences are in Settings \u{203a} About \u{203a} Licences.";

/// What the window paints.
#[derive(Clone, Default)]
pub struct Gal {
    /// pack id -> its real pictures (Normal, Link, Text)
    pub pics: BTreeMap<String, Vec<(Role, Arc<CursorImage>)>>,
    /// pack id -> why its pictures could not be read
    pub failed: BTreeMap<String, String>,
    /// the pictures are being looked up (the job runs)
    pub busy: bool,
    pub status: String,
    /// the pack being downloaded (id) and its progress text
    pub getting: Option<(String, String)>,
}

static GAL: Mutex<Option<Gal>> = Mutex::new(None);

pub fn snapshot() -> Gal {
    GAL.lock().ok().and_then(|g| g.clone()).unwrap_or_default()
}

pub(super) fn update(f: impl FnOnce(&mut Gal)) {
    if let Ok(mut g) = GAL.lock() {
        f(g.get_or_insert_with(Gal::default));
    }
}

/// The window is open: the picture lookups stop when it is closed.
pub static OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Test copies: pictures made up from the files in the test's own folder, nothing from the network.
pub fn fixture() {
    update(|g| *g = Gal::default());
}

/// The real network: Windows' own WinHTTP (system certificates, system proxy), only for github.com release links.
pub struct WinFetch(bu_updater::WinHttp, Option<Arc<std::sync::atomic::AtomicBool>>);

impl WinFetch {
    pub fn new() -> Self {
        WinFetch(bu_updater::WinHttp::new("BoylerUtilities"), None)
    }

    /// A fetcher whose downloads end at once when `stop` is set.
    pub fn stoppable(stop: Arc<std::sync::atomic::AtomicBool>) -> Self {
        WinFetch(bu_updater::WinHttp::new("BoylerUtilities"), Some(stop))
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
    stop: Option<Arc<std::sync::atomic::AtomicBool>>,
}

impl std::io::Write for Cap {
    fn write(&mut self, d: &[u8]) -> std::io::Result<usize> {
        if self.stop.as_ref().is_some_and(|s| s.load(std::sync::atomic::Ordering::SeqCst)) {
            return Err(std::io::Error::other("stopped"));
        }
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

impl Fetch for WinFetch {
    fn get(&self, url: &str, range: Option<&str>, max: usize, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Vec<u8>, String> {
        use bu_updater::{Http, Request};
        let mut sink = Cap { buf: Vec::new(), max, stop: self.1.clone() };
        let r = match range {
            Some(r) => self.0.get_range(url, r, &mut sink, progress),
            None => self.0.get(&Request { url, accept: "application/octet-stream" }, &mut sink, progress),
        }
        .map_err(|e| e.to_string())?;
        if !(200..300).contains(&r.status) {
            return Err(format!("HTTP {}", r.status));
        }
        Ok(sink.buf)
    }
}

/// Job: every pack's pictures - from the kept copies first, else from the pack's release page (a few KB each, once, then kept).
pub fn preview_job(data: PathBuf) -> impl FnOnce(&crate::jobs::JobCtx) -> Result<String, crate::jobs::JobError> + Send + 'static {
    move |job| {
        update(|g| {
            g.busy = true;
            g.status.clear();
        });
        let fetch = WinFetch::stoppable(job.stop_flag());
        let n = site::LIST.len();
        for (i, l) in site::LIST.iter().enumerate() {
            if job.stopped() || !OPEN.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            let dir = site::cache_dir(&data, l.id);
            let mut pics = site::load_preview(&dir);
            if pics.is_empty() {
                update(|g| g.status = format!("Looking at the packs {}/{n}", i + 1));
                job.status("Looking at the packs");
                match site::fetch_preview(&fetch, l) {
                    Ok(p) => {
                        let _ = site::save_preview(&dir, &p);
                        pics = p;
                    }
                    Err(e) => {
                        update(|g| {
                            g.failed.insert(l.id.to_string(), e);
                        });
                    }
                }
                // be gentle with GitHub
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
            let decoded: Vec<(Role, Arc<CursorImage>)> = pics.iter().filter_map(|(r, b)| curfile::decode(b, 64).map(|c| (*r, Arc::new(c)))).collect();
            update(|g| {
                g.pics.insert(l.id.to_string(), decoded);
            });
            // (a job update wakes the window: the new pictures show at once)
            job.progress((i + 1) as f32 / n as f32);
        }
        update(|g| {
            g.busy = false;
            g.status.clear();
        });
        Ok(String::new())
    }
}

/// Job: download one pack's .zip into the app's folder. The result (`Done`) is where it was put; the Mouse page's worker
/// installs it from there.
pub fn get_job(l: Listing, data: PathBuf) -> impl FnOnce(&crate::jobs::JobCtx) -> Result<String, crate::jobs::JobError> + Send + 'static {
    move |job| {
        let id = l.id.to_string();
        let say = |t: &str| {
            let t = t.to_string();
            update(|g| g.getting = Some((id.clone(), t)));
        };
        say("Downloading\u{2026}");
        job.status("Downloading\u{2026}");
        let r = (|| -> Result<String, String> {
            let zip = site::download(&WinFetch::stoppable(job.stop_flag()), &l, &mut |done, total| {
                let total = total.filter(|t| *t > 0).unwrap_or(l.bytes);
                job.progress(done as f32 / total as f32);
                say(&format!("{} of {}", site::size_text(done), site::size_text(total)));
            })?;
            say("Installing\u{2026}");
            let dir = data.join("cursors").join("store");
            std::fs::create_dir_all(&dir).map_err(|e| format!("couldn\u{2019}t write the download: {e}"))?;
            let path = dir.join(format!("{}.zip", l.id));
            std::fs::write(&path, zip).map_err(|e| format!("couldn\u{2019}t write the download: {e}"))?;
            Ok(path.to_string_lossy().into_owned())
        })();
        update(|g| g.getting = None);
        r.map_err(crate::jobs::JobError::Failed)
    }
}

impl Mouse {
    /// "Get more cursors\u{2026}": opens the window and starts looking at the packs (a click starts it; nothing runs on its own).
    pub(super) fn open_store(&mut self, cx: &mut Cx) {
        self.menu = None;
        self.store_open = true;
        self.store_at = cx.now;
        self.store_msg = None;
        OPEN.store(true, std::sync::atomic::Ordering::SeqCst);
        if self.test {
            fixture();
            return;
        }
        // a download left behind by an app that quit before installing it
        if let Ok(rd) = std::fs::read_dir(svc::data_dir().join("cursors").join("store")) {
            for e in rd.flatten() {
                if e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")) && self.store_getting.is_none() {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        match cx.start_job(JOB_PREVIEW, preview_job(svc::data_dir())) {
            Ok(()) => {}
            Err(e) if e.contains("AlreadyRunning") => {}
            Err(e) => self.store_msg = Some(e),
        }
    }

    pub(super) fn close_store(&mut self, cx: &mut Cx) {
        OPEN.store(false, std::sync::atomic::Ordering::SeqCst);
        self.store_open = false;
        cx.stop_job(JOB_PREVIEW);
        pic::forget();
    }

    /// The window (None when closed).
    pub(super) fn store_window(&mut self, cx: &mut Cx) -> Option<El> {
        if !self.store_open {
            return None;
        }
        let g = snapshot();
        let mut rows: Vec<El> = Vec::new();
        for (i, l) in site::LIST.iter().enumerate() {
            rows.push(self.store_row(cx, i, l, &g));
        }
        let (bg, rim) = group::glass();
        let list = cx.scroll_box(sub(K_GET, "rows"), rows).items(AlignItems::STRETCH).max_h(300.0).radius(10.0).bg(bg).inset(&rim).margin(10.0, 0.0, 0.0, 0.0);
        let note = El::text(NOTE, Font::new(11.0, 400), FG3(), lh(11.0, 1.4)).wrapping().margin(0.0, 0.0, 0.0, 2.0);
        let mut status = String::new();
        if g.busy {
            status = if g.status.is_empty() { "Looking at the packs\u{2026}".into() } else { g.status.clone() };
        }
        if let Some(m) = &self.store_msg {
            status = m.clone();
        }
        let foot_text = El::text(status, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).ellipsis().flex1();
        let done = button::cbtn(cx, K_GDONE, "Done", button::Kind::Primary, false, false, 76.0);
        let foot = El::row().center().gap(8.0).margin(12.0, 0.0, 0.0, 0.0).child(foot_text).child(done);
        Some(dialog::dialog(cx, K_GET, 480.0, "Get more cursors", vec![note, list, foot], vec![], true, self.store_at))
    }

    fn store_row(&mut self, cx: &mut Cx, i: usize, l: &Listing, g: &Gal) -> El {
        let installed = self.v.packs.iter().any(|p| p.name == l.name);
        let getting = g.getting.as_ref().filter(|(id, _)| id == l.id).map(|(_, t)| t.clone());
        let busy_other = g.getting.is_some();
        // the three real pictures: Normal, Link, Text
        let mut tiles = El::row().gap(4.0).none();
        for role in site::PREVIEW_ROLES {
            let pic = g.pics.get(l.id).and_then(|v| v.iter().find(|(r, _)| *r == role)).and_then(|(_, ci)| pic::keyed(&format!("store:{}:{}", l.id, role.name()), ci));
            let mut t = El::block().size(34.0, 34.0).none().radius(8.0).bg(if crate::ui::is_light() { Rgba::rgba(60, 60, 67, 0.13) } else { WELL() }).inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())]);
            if let Some(p) = pic {
                t = t.child(El::paint(move |g, (x, y, _, _)| pic::paint(g, &p, x + 4.0, y + 4.0, 26.0, 1.0)).abs(0.0, 0.0, 0.0, 0.0).no_hit());
            }
            tiles = tiles.child(t);
        }
        let name = El::text(l.name, Font::new(12.5, 500), FG(), lh(12.5, 1.35)).ellipsis();
        let about = El::text(l.about, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).ellipsis();
        let who = El::text(format!("{} \u{b7} {} \u{b7} {}", l.maker, l.licence, site::size_text(l.bytes)), Font::new(10.5, 400), FG3(), lh(10.5, 1.35)).ellipsis();
        let left = El::col().flex1().child(name).child(about).child(who);
        let right = if let Some(t) = getting {
            El::text(t, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).none()
        } else if installed {
            El::text("Installed", Font::new(11.5, 600), FG2(), lh(11.5, 1.35)).none()
        } else {
            button::cbtn(cx, idx(K_GROW, i), "Get", BKind::Ghost, true, busy_other, 0.0)
        };
        El::row().center().gap(10.0).min_h(54.0).pad(6.0, 10.0, 6.0, 12.0).child(tiles).child(left).child(right)
    }

    /// A click inside the window. True = it was ours.
    pub(super) fn store_clicked(&mut self, k: Key, cx: &mut Cx) -> bool {
        if !self.store_open {
            return false;
        }
        if k == K_GDONE || k == sub(K_GET, "x") || k == sub(K_GET, "out") {
            self.close_store(cx);
            return true;
        }
        for i in 0..site::LIST.len() {
            if k == idx(K_GROW, i) {
                if snapshot().getting.is_none() {
                    self.start_get(site::LIST[i], cx);
                }
                return true;
            }
        }
        false
    }

    fn start_get(&mut self, l: Listing, cx: &mut Cx) {
        self.store_msg = None;
        if self.test {
            // test copies never touch the network
            return;
        }
        match cx.start_job(JOB_GET, get_job(l, svc::data_dir())) {
            Ok(()) => {
                self.store_getting = Some(l.name.to_string());
                cx.toast(&format!("Downloading {}\u{2026}", l.name));
            }
            Err(e) => self.store_msg = Some(e),
        }
    }

    /// The download job ended: the worker installs the .zip, or the reason is shown.
    pub(super) fn follow_store(&mut self, cx: &mut Cx) {
        let Some(v) = cx.job(JOB_GET) else { return };
        let Some(end) = v.end.clone() else { return };
        if self.store_done == Some(v.id) {
            return;
        }
        self.store_done = Some(v.id);
        match end {
            crate::jobs::End::Done(path) => {
                let name = self.store_getting.take().unwrap_or_default();
                self.store_msg = None;
                self.send(Cmd::InstallStore(name, PathBuf::from(path)));
            }
            crate::jobs::End::Failed(e) => {
                self.store_getting = None;
                self.store_msg = Some(format!("Not downloaded: {e}"));
                cx.toast(&format!("Not downloaded: {e}"));
            }
            crate::jobs::End::Stopped => {
                self.store_getting = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Order 066 real-site proof (`BU_STORE_CACHE=<folder> cargo test -p bu-app pages::mouse::store::tests::real_site -- --ignored --nocapture`):
    /// every pack of the list gets its three real pictures from its GitHub release page (range requests only, kept in the cache folder),
    /// and five packs are downloaded whole and installed into a scratch folder on the FAKE registry. Reads github.com only.
    #[test]
    #[ignore]
    fn real_site() {
        let Ok(cache) = std::env::var("BU_STORE_CACHE") else { return };
        let cache = PathBuf::from(cache);
        let f = WinFetch::new();
        let mut bad = Vec::new();
        for l in site::LIST.iter() {
            let dir = site::cache_dir(&cache, l.id);
            let t = std::time::Instant::now();
            match site::fetch_preview(&f, l) {
                Err(e) => {
                    println!("PREVIEW FAIL {}: {e}", l.id);
                    bad.push(l.id);
                }
                Ok(p) => {
                    let _ = site::save_preview(&dir, &p);
                    let sizes: Vec<String> = p.iter().map(|(r, b)| format!("{}={:?}", r.name(), curfile::decode(b, 64).map(|c| (c.w, c.h)))).collect();
                    println!("{:<26} {:>5} ms  {} bytes read  {}", l.id, t.elapsed().as_millis(), p.iter().map(|(_, b)| b.len()).sum::<usize>(), sizes.join(" "));
                    if !p.iter().any(|(r, b)| *r == Role::Normal && curfile::decode(b, 64).is_some()) {
                        bad.push(l.id);
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        let scratch = cache.join("install-scratch");
        let _ = std::fs::remove_dir_all(&scratch);
        for id in ["google-dot-black", "rose-pine", "nordzy-dark", "macos-black", "bibata-modern-classic"] {
            let l = site::LIST.iter().find(|l| l.id == id).unwrap();
            let zip = site::download(&f, l, &mut |_, _| {}).expect("download");
            let mut m = bu_mouse::Mouse::new(bu_mouse::fake::FakeOs::new(), bu_mouse::AppDirs::new(&scratch));
            let pack = m.install_store_zip(l.name, &zip).unwrap_or_else(|e| panic!("{id}: {e}"));
            let dir = scratch.join("cursors").join("packs").join(l.name);
            let roles: Vec<String> = pack.roles.iter().map(|(r, f)| format!("{}={f}", r.reg_name())).collect();
            println!("INSTALLED {id}: {} MB zip, {} roles: {}", zip.len() / 1_000_000, pack.roles.len(), roles.join(" "));
            for (r, file) in &pack.roles {
                let b = std::fs::read(dir.join(file)).unwrap();
                assert!(curfile::decode(&b, 64).is_some(), "{id}: {file} ({}) does not decode", r.reg_name());
            }
            assert!(pack.roles.contains_key(&bu_mouse::cursors::WinRole::Arrow) && pack.roles.contains_key(&bu_mouse::cursors::WinRole::Wait), "{id}");
        }
        assert!(bad.is_empty(), "no picture for {bad:?}");
    }
}
