//! The Screenshots tab (menu-v22 page `shot`). Order 014 made the header (title + `.phr`: the small blue "Change path", a
//! thin separator, the Screenshot key row); Order 019 fills the page from the drawing and wires crates/screenshot:
//! the gallery of recent screenshots (newest first, 4 per row, the grid glides when shots come or go), Explorer-style
//! selection (click / Ctrl / Shift, Ctrl+A, Delete) with the selection bar, Copy / Delete (Recycle Bin), the "Change path"
//! window (the folder now, Open = Explorer there, Change = Windows' folder picker, saved at once) and the reset line.
//! Test copies use the FAKE engine with the drawing's sample gallery (`gallery::sample_engine`); nothing on the PC changes.
//! The capture itself (the overlay, flash, toast and tray thumbnail) is `overlay.rs`.

pub mod gallery;
pub mod key;
pub mod lbhost;
pub mod lightbox;
pub mod overlay;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

use skia_safe as sk;
use taffy::style::JustifyContent;

use crate::anim::{Bezier, EASE, EASE_IN, EASE_OUT};
use crate::gfx::{sh, Align, Font, Rgba};
use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, sub, El, Key};
use crate::ui::pieces::mitems::{self, It, Place, Row};
use crate::ui::pieces::{self, button, dialog, ibtn, keyfield, link, reset, selbar, toast};
use crate::ui::{cmix, ACC, FG, FG2, FG3, HAIR, ICO_ON, LH125, PAGE_TOP, RED, WELL, WIN_H, WIN_W};

use bu_screenshot::fake::FakeOs;
use bu_screenshot::naming::{self, LocalTime};
use bu_screenshot::{Error as ShotError, Image, ScreenshotOs, Screenshots as Engine, Shot};
use gallery::{Mods, Sel};

/// The keys manager's action for the Screenshot key (saved in the settings file under this id).
pub const KEY_ACTION: &str = "shot.key";

const K_PATH: Key = key("shot.path");
const K_KEY: Key = key("shot.key");
const K_SHOT: Key = key("shot.item");
const K_GAL: Key = key("shot.gal");
const K_DLG: Key = key("shot.dlg");
const K_DLG_OPEN: Key = key("shot.dlg.open");
const K_DLG_CHANGE: Key = key("shot.dlg.change");
const K_DLG_BODY: Key = key("shot.dlg.body");
const K_SEL: Key = key("shot.selbar");
/// the selection bar's buttons (`pieces::selbar`: action i = `idx(K_SEL, i)`, the × after them)
fn k_sel_copy() -> Key {
    idx(K_SEL, 0)
}
fn k_sel_del() -> Key {
    idx(K_SEL, 1)
}
fn k_sel_x() -> Key {
    idx(K_SEL, 2)
}
const K_TOAST: Key = key("shot.toast");
const K_RESET: Key = key("shot.reset");
/// the right-click menu on a picture (row i = `idx(K_CTX, i)`) and its Delete confirm (`sub(K_CTXQ, "go" / "no")`)
const K_CTX: Key = key("shot.ctx");
const K_CTXQ: Key = key("shot.ctxq");

/// Order 042 (the owner's test 2: "u can't right click on a single image in screenshot to delete it"): the right-click menu
/// of a picture - on a picture of a bigger selection it acts on the whole selection, else on that one picture.
struct Ctx {
    ids: Vec<u64>,
    /// the menu's head line: the file name (one picture) or "N screenshots"
    head: String,
    x: f32,
    y: f32,
    /// Delete was picked: the menu shows its confirm
    ask: bool,
}

/// The right-click menu's items, in row order after the head line (row 0).
#[derive(Clone, Copy, PartialEq)]
enum CtxItem {
    Open,
    Copy,
    Folder,
    Delete,
}

fn ctx_items(n: usize) -> Vec<CtxItem> {
    let mut v = Vec::new();
    if n == 1 {
        v.push(CtxItem::Open);
    }
    v.extend([CtxItem::Copy, CtxItem::Folder, CtxItem::Delete]);
    v
}

/// `.sth{transition:transform .2s cubic-bezier(.3,.7,.2,1)}`
const LIFT: Bezier = Bezier::new(0.3, 0.7, 0.2, 1.0);
/// `.sth{background:#0d0f15}` (and the drawing's paintImg letterbox)
const THUMB_BG: Rgba = Rgba::rgb(13, 15, 21);
/// the double-click window when Windows can't be asked (its default)
const DBL_MS: f64 = 500.0;
/// the drawing's fake folder picker takes 700 ms (`setTimeout(..., 700)`); the fake engine shows the same wait
const FAKE_PICK_MS: f64 = 700.0;

/// Bumped by whoever saves a screenshot (the capture overlay): the page shows new shots the next frame it is open.
static GALLERY_GEN: AtomicU64 = AtomicU64::new(0);

/// A new screenshot was saved: an open Screenshots page reads its gallery again (the new one glides in at the front).
pub fn gallery_changed() {
    GALLERY_GEN.fetch_add(1, Ordering::Relaxed);
}

// ---------------------------------------------------------------- the engine, real or fake
/// What the page asks of bu-screenshot (one object for the real and the fake engine).
trait Api {
    fn gallery(&self) -> Result<Vec<Shot>, ShotError>;
    fn thumbnail(&self, id: u64) -> Result<Image, ShotError>;
    /// the whole picture (the lightbox)
    fn load(&self, id: u64) -> Result<Image, ShotError>;
    fn delete(&self, ids: &[u64]) -> Result<(), ShotError>;
    fn copy_shots(&self, ids: &[u64]) -> Result<(), ShotError>;
    /// Explorer with the file(s) selected
    fn show_in_folder(&self, ids: &[u64]) -> Result<(), ShotError>;
    /// the files a drag out of the gallery carries
    fn drag_paths(&self, ids: &[u64]) -> Result<Vec<PathBuf>, ShotError>;
    fn save_dir(&self) -> Result<PathBuf, ShotError>;
    fn saved_dir(&self) -> Result<Option<PathBuf>, ShotError>;
    fn default_save_dir(&self) -> Result<PathBuf, ShotError>;
    fn reset_save_dir(&self) -> Result<(), ShotError>;
    fn set_save_dir(&self, dir: &std::path::Path) -> Result<(), ShotError>;
    fn open_save_dir(&self) -> Result<(), ShotError>;
    fn pick_save_dir(&self) -> Result<PathBuf, ShotError>;
    fn shot_time(&self, s: &Shot) -> LocalTime;
    fn now_local(&self) -> LocalTime;
}

impl<O: ScreenshotOs> Api for Engine<O> {
    fn gallery(&self) -> Result<Vec<Shot>, ShotError> {
        Engine::gallery(self)
    }
    fn thumbnail(&self, id: u64) -> Result<Image, ShotError> {
        Engine::thumbnail(self, id)
    }
    fn load(&self, id: u64) -> Result<Image, ShotError> {
        Engine::load(self, id)
    }
    fn delete(&self, ids: &[u64]) -> Result<(), ShotError> {
        Engine::delete(self, ids)
    }
    fn copy_shots(&self, ids: &[u64]) -> Result<(), ShotError> {
        Engine::copy_shots(self, ids)
    }
    fn show_in_folder(&self, ids: &[u64]) -> Result<(), ShotError> {
        Engine::show_in_folder(self, ids)
    }
    fn drag_paths(&self, ids: &[u64]) -> Result<Vec<PathBuf>, ShotError> {
        Engine::drag_paths(self, ids)
    }
    fn save_dir(&self) -> Result<PathBuf, ShotError> {
        Engine::save_dir(self)
    }
    fn saved_dir(&self) -> Result<Option<PathBuf>, ShotError> {
        Engine::saved_dir(self)
    }
    fn default_save_dir(&self) -> Result<PathBuf, ShotError> {
        Engine::default_save_dir(self)
    }
    fn reset_save_dir(&self) -> Result<(), ShotError> {
        Engine::reset_save_dir(self)
    }
    fn set_save_dir(&self, dir: &std::path::Path) -> Result<(), ShotError> {
        Engine::set_save_dir(self, dir)
    }
    fn open_save_dir(&self) -> Result<(), ShotError> {
        Engine::open_save_dir(self)
    }
    fn pick_save_dir(&self) -> Result<PathBuf, ShotError> {
        Engine::pick_save_dir(self)
    }
    fn shot_time(&self, s: &Shot) -> LocalTime {
        Engine::shot_time(self, s)
    }
    fn now_local(&self) -> LocalTime {
        Engine::now_local(self)
    }
}

/// Windows calls that may show a window of their own or wait on the shell (the folder picker, the Recycle Bin) run on a
/// worker thread with their own engine, so the menu's thread never sits inside them; the page polls the answer.
enum Job {
    Pick(mpsc::Receiver<Result<PathBuf, ShotError>>),
    Delete(mpsc::Receiver<Result<(), ShotError>>, Vec<u64>),
    /// the fake engine's answer, shown after the drawing's wait
    FakePick(Result<PathBuf, ShotError>),
}

#[cfg(windows)]
/// `read_only` = a --real-read test copy: the engine refuses every change.
fn real_engine(read_only: bool) -> Option<Engine<bu_screenshot::real::RealOs>> {
    let os = if read_only { bu_screenshot::real::RealOs::read_only() } else { bu_screenshot::real::RealOs::new() };
    bu_screenshot::real::default_data_dir().map(|d| Engine::new(os, d))
}

// ---------------------------------------------------------------- one gallery tile
struct View {
    id: u64,
    width: u32,
    height: u32,
    cap: String,
    /// the picture (decoded lazily: a few per frame) and whether it fills the box (the drawing's sample canvases) or is
    /// fitted inside it with dark bars (real thumbnails, the drawing's paintImg "contain")
    img: Option<sk::Image>,
    fill: bool,
    /// fading out (Delete): when it started
    gone_at: Option<f64>,
}

fn to_sk(img: &Image) -> Option<sk::Image> {
    let ii = sk::ImageInfo::new((img.width as i32, img.height as i32), sk::ColorType::BGRA8888, sk::AlphaType::Premul, Some(sk::ColorSpace::new_srgb()));
    sk::images::raster_from_data(&ii, sk::Data::new_copy(&img.bgra), (img.width * 4) as usize)
}

/// The modifier keys held at this event (the frame's `cx.mods`, 014 item 1c).
fn mods(cx: &Cx) -> Mods {
    Mods { ctrl: cx.mods.ctrl, shift: cx.mods.shift }
}

fn double_click_ms() -> f64 {
    #[cfg(windows)]
    {
        let t = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() };
        if t > 0 {
            return t as f64;
        }
    }
    DBL_MS
}

// ---------------------------------------------------------------- the page
#[derive(Default)]
pub struct Screenshots {
    /// the keys manager's last refusal for the Screenshot key ("Already used by Mic mute") and since when it shows
    key_err: Option<(String, f64)>,

    svc: Option<Box<dyn Api>>,
    /// the fake engine's OS (test copies): what a test looks at, and the drawing's picker answers
    fake: Option<FakeOs>,
    /// a --real-read test copy (its workers' engines refuse every change too)
    real_read: bool,
    profile: Option<PathBuf>,
    shots: Vec<View>,
    sel: Sel,
    /// FLIP: a shot's old cell index and when it started gliding to its new one (340 ms EASE_OUT)
    moved: HashMap<u64, (usize, f64)>,
    /// a new shot appearing (scale .92 -> 1, opacity 0 -> 1, 300 ms EASE_OUT)
    appeared: HashMap<u64, f64>,
    gen_seen: u64,
    /// the last click on a picture (for the double-click) and where the button went down
    last_click: Option<(u64, f64)>,
    press: Option<(u64, f32, f32)>,
    /// a drag started on these shots (the frame's drag-out is 014 item 1c; see `describe`)
    dragging: Option<Vec<u64>>,
    /// the open lightbox: which shot, since when (its full-screen window: see lightbox.rs)
    lb: Option<(u64, f64)>,
    /// the lightbox window is up (the page holds an empty popup meanwhile, so Esc in the menu closes it)
    lb_up: bool,

    /// "Change path": open since, the folder shown, its highlight after a change, the picker running
    dlg: Option<f64>,
    dir: String,
    dir_new_at: Option<f64>,
    wait: Option<(f64, Job)>,

    /// the folder as it was when "Change" was pressed (the change log's old value)
    dir_before: Option<crate::undo::Val>,
    /// the engine (+ the profile folder for its short texts) of a CLOSED page for the reset (Settings › Reset, the
    /// uninstaller): made on first need, never in `resettable()`
    lazy: std::cell::OnceCell<(Box<dyn Api>, Option<PathBuf>)>,
    link_box: HashMap<Key, (f32, f32, f32, f32)>,
    /// the open right-click menu (Order 042)
    ctx: Option<Ctx>,
    /// the error line under the header (`.kerr.shErr`) and when it showed
    err: Option<(String, f64)>,
    toast: Option<(String, f64)>,
    /// a shot had focus and something else took it: if no element of the page was pressed, it was empty space
    blur_pending: bool,
    keymap: HashMap<Key, u64>,
}

impl Screenshots {
    fn order(&self) -> Vec<u64> {
        self.shots.iter().filter(|v| v.gone_at.is_none()).map(|v| v.id).collect()
    }

    fn short(&self, p: &std::path::Path) -> String {
        gallery::short_dir(p, self.profile.as_deref())
    }

    fn show_toast(&mut self, t: impl Into<String>, now: f64) {
        self.toast = Some((t.into(), now));
    }

    /// Reads the gallery from the engine (cheap: the small index file). Pictures are decoded later, a few per frame.
    fn load(&mut self, now: f64, animate_new: bool) {
        let Some(svc) = &self.svc else { return };
        let list = match svc.gallery() {
            Ok(l) => l,
            Err(e) => {
                self.err = Some((format!("Can't read the gallery · {e}"), now));
                return;
            }
        };
        let now_l = svc.now_local();
        let old_idx: HashMap<u64, usize> = self.order().iter().enumerate().map(|(i, id)| (*id, i)).collect();
        let mut old: HashMap<u64, View> = self.shots.drain(..).filter(|v| v.gone_at.is_none()).map(|v| (v.id, v)).collect();
        let fill = self.fake.is_some();
        for (i, s) in list.iter().enumerate() {
            let cap = naming::caption(&now_l, &svc.shot_time(s));
            match old.remove(&s.id) {
                Some(mut v) => {
                    v.cap = cap;
                    if animate_new && old_idx.get(&s.id).is_some_and(|o| *o != i) {
                        self.moved.insert(s.id, (old_idx[&s.id], now));
                    }
                    self.shots.push(v);
                }
                None => {
                    if animate_new {
                        self.appeared.insert(s.id, now);
                    }
                    self.shots.push(View { id: s.id, width: s.width, height: s.height, cap, img: None, fill, gone_at: None });
                }
            }
        }
        let order = self.order();
        self.sel.keep_only(&order);
    }

    /// Decodes up to `n` missing thumbnails. Returns true while some are still missing.
    fn decode_some(&mut self, n: usize) -> bool {
        let Some(svc) = &self.svc else { return false };
        let mut left = n;
        for v in self.shots.iter_mut().filter(|v| v.img.is_none() && v.gone_at.is_none()) {
            if left == 0 {
                return true;
            }
            left -= 1;
            // a broken / missing picture keeps the dark box (img stays None but is not retried every frame)
            v.img = svc.thumbnail(v.id).ok().and_then(|i| to_sk(&i)).or_else(|| to_sk(&Image { width: 1, height: 1, bgra: vec![21, 15, 13, 255] }));
        }
        false
    }

    // ---- actions
    fn delete(&mut self, ids: Vec<u64>, now: f64) {
        if ids.is_empty() {
            return;
        }
        if self.fake.is_some() {
            let r = self.svc.as_ref().map(|s| s.delete(&ids)).unwrap_or(Ok(()));
            self.after_delete(r, ids, now);
            return;
        }
        #[cfg(windows)]
        {
            let (tx, rx) = mpsc::channel();
            let worker_ids = ids.clone();
            let ro = self.real_read;
            std::thread::spawn(move || {
                let r = real_engine(ro).map(|e| e.delete(&worker_ids)).unwrap_or(Err(ShotError::BadData("no data folder".into())));
                let _ = tx.send(r);
            });
            self.wait = Some((now, Job::Delete(rx, ids)));
        }
    }

    fn after_delete(&mut self, r: Result<(), ShotError>, ids: Vec<u64>, now: f64) {
        match r {
            Ok(()) => {
                // they fade out together (170 ms ease-in, scale .9), then the rest glide into place
                for v in self.shots.iter_mut().filter(|v| ids.contains(&v.id)) {
                    v.gone_at = Some(now);
                }
                let order = self.order();
                self.sel.keep_only(&order);
                let n = ids.len();
                self.show_toast(if n > 1 { format!("{n} screenshots moved to the Recycle Bin") } else { "Moved to the Recycle Bin".into() }, now);
            }
            Err(ShotError::NoRecycleBin(_)) => self.show_toast("Not deleted · that drive has no Recycle Bin", now),
            Err(e) => self.show_toast(format!("Not deleted · {e}"), now),
        }
    }

    /// A right-click on picture `id` at (x, y): on a picture of a bigger selection the menu acts on the selection, else
    /// that picture alone becomes the selection (Explorer's way).
    fn context(&mut self, id: u64, x: f32, y: f32) {
        let order = self.order();
        if !(self.sel.has(id) && self.sel.len() > 1) {
            self.sel.click(&order, id, Mods::default());
        }
        let ids = self.sel.in_order(&order);
        let head = match ids.len() {
            1 => self
                .svc
                .as_ref()
                .and_then(|s| s.drag_paths(&ids).ok())
                .and_then(|p| p.first().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "1 screenshot".into()),
            n => format!("{n} screenshots"),
        };
        self.ctx = Some(Ctx { ids, head, x, y, ask: false });
    }

    /// A click while the right-click menu is open: true = it was the menu's (or its confirm's).
    fn ctx_click(&mut self, k: Key, now: f64) -> bool {
        let Some(c) = self.ctx.as_mut() else { return false };
        if c.ask {
            if k == sub(K_CTXQ, "go") {
                let ids = c.ids.clone();
                self.ctx = None;
                self.delete(ids, now);
                return true;
            }
            if k == sub(K_CTXQ, "no") {
                self.ctx = None;
                return true;
            }
            return false;
        }
        let items = ctx_items(c.ids.len());
        let Some(it) = (0..items.len()).find(|&i| idx(K_CTX, i + 1) == k).map(|i| items[i]) else { return false };
        let ids = c.ids.clone();
        match it {
            CtxItem::Delete => {
                c.ask = true;
                return true;
            }
            CtxItem::Open => {
                self.lb = Some((ids[0], now));
                if self.fake.is_none() {
                    self.open_lightbox(ids[0], now);
                }
            }
            CtxItem::Copy => self.copy(ids, now),
            CtxItem::Folder => {
                if let Some(Err(e)) = self.svc.as_ref().map(|s| s.show_in_folder(&ids)) {
                    self.show_toast(format!("Can't show it in its folder · {e}"), now);
                }
            }
        }
        self.ctx = None;
        true
    }

    /// The right-click menu (or its Delete confirm) at the pointer.
    fn ctx_menu(&self, cx: &mut Cx) -> Option<El> {
        let c = self.ctx.as_ref()?;
        let n = c.ids.len();
        if c.ask {
            let (title, text) = if n > 1 { (format!("Delete {n} screenshots?"), "They go to the Recycle Bin.") } else { ("Delete this screenshot?".to_string(), "It goes to the Recycle Bin.") };
            return Some(mitems::confirm(cx, K_CTXQ, &title, text, "Cancel", "Delete", button::Kind::Red, Place::At(c.x, c.y), 260.0));
        }
        let mut rows = vec![Row::Head(&c.head)];
        for it in ctx_items(n) {
            rows.push(Row::Item(match it {
                CtxItem::Open => It::icon("open", "Open"),
                CtxItem::Copy => It::icon("copy", "Copy"),
                CtxItem::Folder => It::icon("fold", "Show in folder"),
                CtxItem::Delete => It::icon("trash", "Delete").danger(),
            }));
        }
        Some(mitems::menu(cx, K_CTX, &rows, Place::At(c.x, c.y), 190.0))
    }

    fn copy(&mut self, ids: Vec<u64>, now: f64) {
        if ids.is_empty() {
            return;
        }
        let r = self.svc.as_ref().map(|s| s.copy_shots(&ids)).unwrap_or(Ok(()));
        match r {
            Ok(()) => {
                let n = ids.len();
                self.show_toast(if n > 1 { format!("{n} screenshots copied") } else { "Copied to clipboard".into() }, now);
            }
            Err(e) => self.show_toast(format!("Not copied · {e}"), now),
        }
    }

    fn refresh_dir(&mut self) {
        if let Some(d) = self.svc.as_ref().and_then(|s| s.save_dir().ok()) {
            self.dir = self.short(&d);
        }
    }

    fn change_dir(&mut self, now: f64) {
        if self.wait.is_some() {
            return;
        }
        self.dir_before = self.svc.as_deref().and_then(|s| Self::folder_val(s, self.profile.as_deref()));
        if let Some(os) = &self.fake {
            // the fake answers like the drawing: the other of its two folders
            let cur = self.svc.as_ref().and_then(|s| s.save_dir().ok()).unwrap_or_default();
            let clips = PathBuf::from(r"D:\Clips\Screenshots");
            os.state().pick = Some(if cur == clips { os.default_save_dir().unwrap_or_default() } else { clips });
            let r = self.svc.as_ref().map(|s| s.pick_save_dir()).unwrap_or(Err(ShotError::Cancelled));
            self.wait = Some((now, Job::FakePick(r)));
            return;
        }
        #[cfg(windows)]
        {
            let (tx, rx) = mpsc::channel();
            let ro = self.real_read;
            std::thread::spawn(move || {
                let r = real_engine(ro).map(|e| e.pick_save_dir()).unwrap_or(Err(ShotError::BadData("no data folder".into())));
                let _ = tx.send(r);
            });
            self.wait = Some((now, Job::Pick(rx)));
        }
    }

    fn after_pick(&mut self, r: Result<PathBuf, ShotError>, now: f64) {
        match r {
            Ok(d) => {
                // into the app's change log (the drawing's RS.shot "Screenshots folder"): the folder before the change
                let new = self.svc.as_deref().and_then(|s| Self::folder_val(s, self.profile.as_deref()));
                if let (Some(old), Some(new)) = (self.dir_before.take(), new) {
                    rec(FOLDER, FOLDER_LABEL, &old, &new);
                }
                self.dir = self.short(&d);
                self.dir_new_at = Some(now);
                let s = self.dir.clone();
                self.show_toast(format!("New screenshots go to {s}"), now);
            }
            Err(ShotError::Cancelled) => {}
            Err(e) => self.show_toast(format!("Folder not changed · {e}"), now),
        }
    }

    /// Polls a running worker job. Returns true while one runs.
    fn poll(&mut self, now: f64) -> bool {
        let Some((since, job)) = self.wait.take() else { return false };
        match job {
            Job::FakePick(r) => {
                if now - since >= FAKE_PICK_MS {
                    self.after_pick(r, now);
                } else {
                    self.wait = Some((since, Job::FakePick(r)));
                }
            }
            Job::Pick(rx) => match rx.try_recv() {
                Ok(r) => self.after_pick(r, now),
                Err(mpsc::TryRecvError::Empty) => self.wait = Some((since, Job::Pick(rx))),
                Err(mpsc::TryRecvError::Disconnected) => {}
            },
            Job::Delete(rx, ids) => match rx.try_recv() {
                Ok(r) => self.after_delete(r, ids, now),
                Err(mpsc::TryRecvError::Empty) => self.wait = Some((since, Job::Delete(rx, ids))),
                Err(mpsc::TryRecvError::Disconnected) => {}
            },
        }
        self.wait.is_some()
    }

    /// Faded-out shots leave the grid; the others glide from their old cells (the drawing's galFlip).
    fn settle(&mut self, now: f64) {
        if !self.shots.iter().any(|v| v.gone_at.is_some_and(|t| now - t >= 170.0)) {
            return;
        }
        let before: HashMap<u64, usize> = self.shots.iter().enumerate().map(|(i, v)| (v.id, i)).collect();
        self.shots.retain(|v| !v.gone_at.is_some_and(|t| now - t >= 170.0));
        for (i, v) in self.shots.iter().enumerate() {
            if before[&v.id] != i {
                self.moved.insert(v.id, (before[&v.id], now));
            }
        }
    }

    /// The open page's engine + profile, else (a closed page) one made on first need: the real one, the fake in test
    /// copies and unit tests, a --real-read copy's read-only one (its `apply` refuses before it).
    fn any_engine(&self) -> (&dyn Api, Option<&std::path::Path>) {
        if let Some(s) = &self.svc {
            return (s.as_ref(), self.profile.as_deref());
        }
        let (e, p) = self.lazy.get_or_init(|| {
            let fake = cfg!(test) || (crate::testmode::on() && !crate::testmode::real_read());
            let eng: Box<dyn Api> = if fake {
                Box::new(gallery::sample_engine().1)
            } else {
                #[cfg(windows)]
                {
                    match real_engine(crate::testmode::real_read()) {
                        Some(e) => Box::new(e),
                        None => Box::new(gallery::sample_engine().1),
                    }
                }
                #[cfg(not(windows))]
                Box::new(gallery::sample_engine().1)
            };
            (eng, gallery::profile_dir(fake))
        });
        (e.as_ref(), p.as_deref())
    }

    /// The screenshots folder as the change log keeps it: raw = the chosen folder ("" = none chosen: Windows' own
    /// Screenshots folder; Windows' folder picked by hand is the same), shown short ("Pictures\Screenshots").
    fn folder_val(svc: &dyn Api, profile: Option<&std::path::Path>) -> Option<crate::undo::Val> {
        let def = svc.default_save_dir().ok();
        let raw = svc.saved_dir().ok()?.filter(|d| Some(d) != def.as_ref()).map(|d| d.display().to_string()).unwrap_or_default();
        let shown = svc.save_dir().ok()?;
        Some(crate::undo::Val::new(&raw, &gallery::short_dir(&shown, profile)))
    }

    /// The lightbox window for shot `id` (real copies): the whole picture, its name + "W × H · folder", grown out of the
    /// thumbnail's box on screen.
    fn open_lightbox(&mut self, id: u64, now: f64) {
        let Some(shot) = self.svc.as_ref().and_then(|s| s.gallery().ok()).and_then(|g| g.into_iter().find(|s| s.id == id)) else { return };
        let Some(img) = self.svc.as_ref().and_then(|s| s.load(id).ok()).and_then(|i| to_sk(&i)) else {
            self.show_toast("Couldn’t open the picture", now);
            return;
        };
        let name = shot.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dir = shot.path.parent().map(|p| gallery::short_dir(p, self.profile.as_deref())).unwrap_or_default();
        let info = format!("{} × {} · {}", shot.width, shot.height, dir);
        let Some((mr, sc)) = key::menu_window() else { return };
        let th = self.link_box.get(&sub(sub(K_SHOT, &id.to_string()), "th")).copied();
        let thumb = th.map(|b| windows::Win32::Foundation::RECT {
            left: mr.left + (b.0 * sc).round() as i32,
            top: mr.top + (b.1 * sc).round() as i32,
            right: mr.left + ((b.0 + b.2) * sc).round() as i32,
            bottom: mr.top + ((b.1 + b.3) * sc).round() as i32,
        });
        match lbhost::open(img, (shot.width, shot.height), name, info, mr, thumb) {
            Ok(()) => self.lb_up = lbhost::is_open(),
            Err(e) => self.show_toast(e, now),
        }
    }

    /// A test copy's picture state (`BU_TEST_PAGE_STATE=shot:<state>`): the page as the drawing shows it after the clicks
    /// the test hook can't send yet (014 item 1c). Animations already finished.
    fn test_state(&mut self, s: &str, now: f64) {
        let order = self.order();
        let t = now - 5000.0;
        match s {
            "dlg" => self.dlg = Some(t),
            "dlgwait" => {
                self.dlg = Some(t);
                self.wait = Some((now + 1e9, Job::FakePick(Err(ShotError::Cancelled))));
            }
            "dlgnew" => {
                self.dlg = Some(t);
                if let Some(os) = &self.fake {
                    os.state().pick = Some(PathBuf::from(r"D:\Clips\Screenshots"));
                }
                if let Some(Ok(d)) = self.svc.as_ref().map(|s| s.pick_save_dir()) {
                    self.dir = self.short(&d);
                }
                self.dir_new_at = Some(now - 200.0);
            }
            "sel1" => self.sel.click(&order, order[1], Mods::default()),
            "sel3" => {
                for i in [0, 2, 5] {
                    self.sel.click(&order, order[i], Mods { ctrl: true, shift: false });
                }
            }
            "all" => self.sel.all(&order),
            "err" => self.err = Some(("Already used by Mic mute".into(), t)),
            // shown "later" so it is still up when the test picture is taken (seconds after the page opened)
            "toast" => self.toast = Some(("Moved to the Recycle Bin".into(), now + 2500.0)),
            _ => {}
        }
    }

    // ---- painting
    fn tile(&self, cx: &mut Cx, i: usize, v: &View) -> El {
        let k = sub(K_SHOT, &v.id.to_string());
        let th = sub(k, "th");
        let now = cx.now;
        let selected = self.sel.has(v.id);
        // `.shot.hv .sth{transform:translateY(-2px);box-shadow:0 0 0 .5px rgba(0,0,0,.42),0 10px 22px rgba(0,0,0,.3)}`
        let lift = cx.hover_t(k, 200.0, LIFT);
        let shade = cx.tr(k, 7, if cx.hovered(k) { 1.0 } else { 0.0 }, 200.0, EASE);
        let lerp = |a: f32, b: f32| a + (b - a) * shade;
        // `.sth::after{box-shadow:inset 0 0 0 1px rgba(255,255,255,.09);transition:box-shadow .18s ease}`
        // `.shot.sel .sth::after{box-shadow:inset 0 0 0 2px var(--acc);background:rgba(10,132,255,.13)}`
        let st = cx.tr(k, 8, if selected { 1.0 } else { 0.0 }, 180.0, EASE);
        let ring = cmix(Rgba(1.0, 1.0, 1.0, 0.09), ACC(), st);
        let img = v.img.clone();
        let fill = v.fill;
        let (iw, ih) = img.as_ref().map(|m| (m.width() as f32, m.height() as f32)).unwrap_or((1.0, 1.0));
        // Blink paints the whole `.sth` on its pixel-snapped box (131 x 73.6875 -> 131 x 74 / 73): the background, the
        // overflow clip, the <canvas> (replaced content) and the ::after ring - so they are painted here on that box
        let tint = Rgba(10.0 / 255.0, 132.0 / 255.0, 1.0, 0.13 * st);
        let ring_w = 1.0 + st;
        let pic = El::paint(move |g, (x, y, w, h)| {
            let (x, y, w, h) = g.snap(x, y, w, h);
            g.push_clip_rr4(x, y, w, h, [8.0; 4]);
            g.fill_rect(x, y, w, h, THUMB_BG);
            if let Some(m) = &img {
                if fill && (iw, ih) == (w, h) {
                    // already the box's size (the sample pictures): pixel for pixel
                    g.draw_image(m, x, y, 1.0);
                } else if fill {
                    g.draw_image_rect(m, x, y, w, h);
                } else {
                    let s = (w / iw).min(h / ih);
                    let (dw, dh) = (iw * s, ih * s);
                    g.draw_image_rect(m, x + (w - dw) / 2.0, y + (h - dh) / 2.0, dw, dh);
                }
            }
            // ::after is a child of the overflow:hidden box: its tint and ring are clipped by the same rounded clip
            if tint.3 > 0.0 {
                g.fill_rr(x, y, w, h, 8.0, tint);
            }
            g.inset_shadows(x, y, w, h, 8.0, &[sh(0.0, 0.0, 0.0, ring_w, ring)]);
            g.pop_clip();
        })
        .abs(0.0, 0.0, 0.0, 0.0)
        .no_hit();
        let thumb = El::block()
            .w_pct(100.0)
            .h(gallery::THUMB_H)
            .radius(8.0)
            .bg(THUMB_BG)
            .shadow(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, 0.42)), sh(0.0, lerp(1.0, 10.0), lerp(3.0, 22.0), 0.0, Rgba(0.0, 0.0, 0.0, lerp(0.2, 0.3)))])
            .translate(0.0, -2.0 * lift)
            .opacity(if self.dragging.as_ref().is_some_and(|d| d.contains(&v.id)) { 0.35 } else { 1.0 })
            .key(th)
            .child(pic);
        // `.scap{display:flex;justify-content:space-between;gap:8px;margin:6px 2px 0;font-size:11px;line-height:14px;
        //   color:var(--fg2);font-variant-numeric:tabular-nums;white-space:nowrap}` `.scap span+span{color:var(--fg3)}`
        // `.shot.sel .scap span:first-child{color:var(--fg)}`
        let f = Font::new(11.0, 400).tnum();
        let cap = El::row()
            .justify(JustifyContent::SPACE_BETWEEN)
            .gap(8.0)
            .margin(6.0, 2.0, 0.0, 2.0)
            .child(El::text(v.cap.clone(), f, if selected { FG() } else { FG2() }, 14.0).none())
            .child(El::text(gallery::size_text(v.width, v.height), f, FG3(), 14.0).none());
        let mut tile = El::block().min_w(0.0).on_click(k).child(thumb).child(cap);
        // motion: the FLIP glide, a new shot's pop-in, a deleted one's fade
        let (cx0, cy0) = gallery::cell_pos(i);
        if let Some((from, t0)) = self.moved.get(&v.id) {
            let p = ((now - t0) / 340.0).clamp(0.0, 1.0);
            let e = 1.0 - EASE_OUT.ease(p) as f32;
            let (fx, fy) = gallery::cell_pos(*from);
            tile = tile.translate((fx - cx0) * e, (fy - cy0) * e);
            if p < 1.0 {
                cx.st.busy = true;
            }
        }
        if let Some(t0) = self.appeared.get(&v.id) {
            let p = ((now - t0) / 300.0).clamp(0.0, 1.0);
            let e = EASE_OUT.ease(p) as f32;
            tile = tile.opacity(e).scale(0.92 + 0.08 * e);
            if p < 1.0 {
                cx.st.busy = true;
            }
        }
        if let Some(t0) = v.gone_at {
            let p = ((now - t0) / 170.0).clamp(0.0, 1.0);
            let e = EASE_IN.ease(p) as f32;
            tile = tile.opacity(1.0 - e).scale(1.0 - 0.1 * e).no_hit();
            cx.st.busy = true;
        }
        tile
    }

    fn folder_window(&mut self, cx: &mut Cx, opened: f64) -> El {
        let now = cx.now;
        // `.dpath{display:flex;align-items:center;gap:8px;min-width:0;height:32px;padding:0 11px 0 9px;border-radius:8px;
        //   background:var(--well);box-shadow:inset 0 0 0 .5px var(--hair);font-size:12.5px;color:var(--fg)}`
        // `.dpath svg{width:16px;height:16px;stroke:var(--fg2);stroke-width:1.4}` `.dpath b{font-weight:400;text-overflow:ellipsis}`
        // `.dpath.wait{opacity:.55;transition:opacity .2s ease}` `.dpath.new b{animation:sdhl 1.4s ease}`
        // `@keyframes sdhl{0%,40%{color:var(--ico-on)}100%{color:var(--fg2)}}`
        let waiting = self.wait.as_ref().is_some_and(|(_, j)| matches!(j, Job::Pick(_) | Job::FakePick(_)));
        let op = cx.tr(K_DLG_BODY, 1, if waiting { 0.55 } else { 1.0 }, 200.0, EASE);
        let col = match self.dir_new_at {
            Some(t) if now - t < 1400.0 => {
                cx.st.busy = true;
                let p = ((now - t) / 1400.0) as f32;
                if p <= 0.4 {
                    ICO_ON()
                } else {
                    cmix(ICO_ON(), FG2(), EASE.ease(((p - 0.4) / 0.6) as f64) as f32)
                }
            }
            _ => FG(),
        };
        let path = El::row()
            .center()
            .gap(8.0)
            .min_w(0.0)
            .h(32.0)
            .pad(0.0, 11.0, 0.0, 9.0)
            .radius(8.0)
            .bg(WELL())
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
            .opacity(op)
            .none()
            // Order 045: `dlgBox.title=S.shotDir`
            .key(sub(K_DLG_BODY, "path"))
            .title(&self.dir)
            .child(El::icon("fold", 16.0, 1.4, FG2()))
            .child(El::text(self.dir.clone(), Font::new(12.5, 400), col, LH125).ellipsis().flex1());
        // `.dft{display:flex;justify-content:flex-end;gap:8px;margin-top:18px}` - placed here, not as the piece's footer:
        // the folder window is `.dlg` (padding-bottom 16) = the piece's `.dlg.mdlg` (14) + its `.mdb` (2, 014 item 1c)
        let foot = El::row()
            .justify(JustifyContent::FLEX_END)
            .gap(8.0)
            .margin(18.0, 0.0, 0.0, 0.0)
            .child(ibtn::icbtn(cx, K_DLG_OPEN, "open", "Open", false))
            .child(ibtn::icbtn(cx, K_DLG_CHANGE, "fold", "Change", true));
        let body = El::col().on_click(K_DLG_BODY).child(path).child(foot);
        dialog::dialog(cx, K_DLG, 388.0, "Screenshots folder", vec![body], Vec::new(), true, opened)
    }
}

impl Page for Screenshots {
    fn id(&self) -> &'static str {
        "shot"
    }
    fn name(&self) -> &'static str {
        "Screenshots"
    }
    fn icon(&self) -> &'static str {
        "cam"
    }
    /// Two or more selected: the selection bar, fixed to the window's bottom (built every frame so it fades in / out).
    fn overlay(&mut self, cx: &mut Cx) -> Option<El> {
        let n = self.sel.len();
        let btns = [selbar::Sbb { icon: "copy", label: "Copy", danger: false }, selbar::Sbb { icon: "trash", label: "Delete", danger: true }];
        // Order 045: `sbBtn('x','',..,'sbx','Clear selection (or click empty space)')`
        Some(selbar::selbar_x(cx, K_SEL, &format!("{n} selected"), None, &btns, n > 1, Some("Clear selection (or click empty space)")))
    }
    fn bar_shown(&self) -> bool {
        self.sel.len() > 1
    }
    /// The Screenshot key: pressed anywhere (menu open or closed) = the capture overlay.
    fn start(&self, s: &mut crate::services::Services) {
        s.add_action(crate::keys::Action::new(KEY_ACTION, "Screenshot", "shot"), |down| {
            if down {
                let _ = key::screenshot_key();
            }
        });
    }
    fn open(&mut self, env: &Env, now: f64) {
        key::SHOWN.store(true, Ordering::Relaxed);
        self.gen_seen = GALLERY_GEN.load(Ordering::Relaxed);
        self.profile = gallery::profile_dir(env.fake());
        self.real_read = env.real_read;
        if env.fake() {
            let (os, eng) = gallery::sample_engine();
            self.fake = Some(os);
            self.svc = Some(Box::new(eng));
        } else {
            #[cfg(windows)]
            {
                // a --real-read test copy only reads: the engine refuses every change and every capture
                let os = if env.real_read { bu_screenshot::real::RealOs::read_only() } else { bu_screenshot::real::RealOs::new() };
                self.svc = bu_screenshot::real::default_data_dir().map(|d| Box::new(Engine::new(os, d)) as Box<dyn Api>);
            }
        }
        self.refresh_dir();
        self.load(now, false);
        // the first row(s) at once (the gallery shows without a blink); the rest a few per frame (tick)
        self.decode_some(8);
        // test pictures of a state (a test copy only: crate::testmode::env is None in a normal copy)
        if env.test {
            if let Some(s) = crate::testmode::env("BU_TEST_PAGE_STATE").and_then(|s| s.strip_prefix("shot:").map(str::to_string)) {
                self.test_state(&s, now);
            }
        }
    }
    fn close(&mut self) {
        // the key field listening when the menu closed (no Blur comes): stop it, or every app key stays paused
        crate::services::try_with(|s| s.stop_listening());
        // the lightbox belongs to the open menu (a menu closed by the taskbar / Win / Alt+Tab takes it along)
        lbhost::close();
        key::SHOWN.store(false, Ordering::Relaxed);
        // closed = nothing kept (pictures, selection, popups); a running picker / delete answers into nothing
        *self = Screenshots::default();
    }
    fn tick(&mut self, now: f64) -> bool {
        let mut busy = false;
        let up = lbhost::is_open();
        if up != self.lb_up {
            self.lb_up = up;
            busy = true;
        }
        let g = GALLERY_GEN.load(Ordering::Relaxed);
        if g != self.gen_seen && self.svc.is_some() {
            self.gen_seen = g;
            self.load(now, true);
            busy = true;
        }
        busy |= self.decode_some(2);
        busy |= self.poll(now);
        self.settle(now);
        busy
    }
    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        let now = cx.now;
        if self.blur_pending {
            // the focus left a shot and nothing of the page was pressed: a click on empty space clears the selection
            self.blur_pending = false;
            self.sel.clear();
        }
        self.settle(now);
        self.moved.retain(|_, (_, t)| now - *t < 340.0);
        self.appeared.retain(|_, t| now - *t < 300.0);
        if self.toast.as_ref().is_some_and(|(_, t)| now - t > toast::SHOW_MS + 400.0) {
            self.toast = None;
        }

        // the Screenshot key lives in the keys manager (it works with the menu closed); the field shows its state
        let (set, listening, kerr) = cx.key_field(KEY_ACTION);
        match (&kerr, &self.key_err) {
            (Some(e), Some((old, _))) if e == old => {}
            (Some(e), _) => self.key_err = Some((e.clone(), now)),
            (None, _) => self.key_err = None,
        }
        let since = listening.as_ref().map(|l| l.1).unwrap_or(0.0);
        let show = match (&listening, &set) {
            (Some((held, _)), _) => keyfield::Show::Listening(held.as_deref()),
            (None, Some(k)) => keyfield::Show::Set(k),
            (None, None) => keyfield::Show::Empty,
        };
        // Order 045: `fShot.el.title='Click to change'` (on the field itself: key_row's last child)
        let mut krow = keyfield::key_row(cx, K_KEY, Some("Screenshot key"), show, since);
        if let Some(f) = krow.children.last_mut() {
            *f = std::mem::take(f).title("Click to change");
        }
        // .phr{display:flex;align-items:center;gap:10px} .phr .gxs{margin:0}
        let phr = El::row()
            .center()
            .gap(10.0)
            .none()
            // Order 045: `cpLnk.title='Where screenshots are saved'`
            .child(link::link(cx, K_PATH, "Change path", 12.0).title("Where screenshots are saved"))
            .child(pieces::separator(0.0))
            .child(krow);
        let mut kids = vec![pieces::header(self.name(), Some(phr))];

        // `.kerr{font-size:11.5px;line-height:15px;color:var(--red)}` `.shErr{margin:-4px 2px 4px;text-align:right}`
        // `.kerr.on{display:block;animation:kin .2s ease-out}` (`kin`: opacity 0, translateY(-3px) -> none)
        if let Some((e, t)) = self.key_err.as_ref().or(self.err.as_ref()) {
            let p = ((now - t) / 200.0).clamp(0.0, 1.0);
            let ee = crate::anim::EASE_OUT_CSS.ease(p) as f32;
            if p < 1.0 {
                cx.st.busy = true;
            }
            kids.push(El::text(e.clone(), Font::new(11.5, 400), RED(), 15.0).wrapping().align(Align::Right).margin(-4.0, 2.0, 4.0, 2.0).opacity(ee).translate(0.0, -3.0 * (1.0 - ee)));
        }

        // `.shg{margin-top:6px}` > `.gal{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:10px 8px}`
        self.keymap.clear();
        let views = std::mem::take(&mut self.shots);
        let mut grid = El::grid().cols(gallery::COLS as u16).gap2(gallery::ROW_GAP, gallery::COL_GAP).key(K_GAL);
        for (i, v) in views.iter().enumerate() {
            let k = sub(K_SHOT, &v.id.to_string());
            self.keymap.insert(k, v.id);
            self.keymap.insert(sub(k, "th"), v.id);
            grid = grid.child(self.tile(cx, i, v));
        }
        self.shots = views;
        kids.push(El::block().margin(6.0, 0.0, 0.0, 0.0).child(grid));

        // the reset line (`.rsl`): the screenshots folder (the change log, see `Resettable` below)
        kids.push(reset::reset_line(cx, K_RESET, Some("Windows defaults")));

        // (two or more selected: the selection bar is the page's window-fixed overlay - `Page::overlay`)
        if self.wait.is_some() || self.shots.iter().any(|v| v.img.is_none()) {
            cx.st.busy = true;
        }
        kids
    }
    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        let now = cx.now;
        // Order 045: Space / Enter close the lightbox too (`if(lbIt&&(e.key==='Escape'||e.key===' '||e.key==='Enter'))
        // {e.preventDefault();closeLB();return;}`, L7961) - whatever has the focus
        if let (true, Ev::Key(_, 0x20 | 0x0D)) = (self.lb_up, ev) {
            lbhost::close();
            cx.used = true;
            return;
        }
        // any element of the page pressed: not a click on empty space
        if let Ev::Press(k, x, y, b) = ev {
            self.blur_pending = false;
            self.link_box.insert(*k, *b);
            if let Some(id) = self.keymap.get(k).copied() {
                if *k == sub(sub(K_SHOT, &id.to_string()), "th") {
                    self.press = Some((id, *x, *y));
                }
            }
        }
        // ---- the right-click menu (Order 042)
        if let Ev::Context(k, x, y) = ev {
            if let Some(id) = self.keymap.get(k).copied() {
                self.context(id, *x, *y);
            }
            return;
        }
        if let Ev::Click(k) = ev {
            if self.ctx_click(*k, now) {
                return;
            }
        }
        match ev {
            // ---- header: the key field - the keys manager captures the key (Esc while listening keeps the old one)
            Ev::Click(k) if *k == K_KEY => cx.listen_key(KEY_ACTION),
            Ev::Click(k) if *k == sub(K_KEY, "clr") => cx.clear_key(KEY_ACTION),
            Ev::Blur(k) if *k == K_KEY => cx.stop_listening(),
            Ev::Click(k) if *k == K_PATH => {
                self.refresh_dir();
                self.dlg = Some(now);
                self.dir_new_at = None;
            }

            // ---- the gallery
            Ev::Click(k) if self.keymap.contains_key(k) => {
                let id = self.keymap[k];
                let order = self.order();
                let m = mods(cx);
                let dbl = self.last_click.is_some_and(|(i, t)| i == id && now - t <= double_click_ms());
                if dbl && !m.ctrl && !m.shift {
                    // double-click = open (the lightbox grows out of the thumbnail; its own window, real copies only)
                    self.lb = Some((id, now));
                    self.last_click = None;
                    if self.fake.is_none() {
                        self.open_lightbox(id, now);
                    }
                } else {
                    self.sel.click(&order, id, m);
                    self.last_click = Some((id, now));
                }
            }
            Ev::Drag(k, x, y, _) if self.keymap.contains_key(k) => {
                if let Some((id, x0, y0)) = self.press {
                    if self.dragging.is_none() && ((x - x0).powi(2) + (y - y0).powi(2)).sqrt() >= 5.0 {
                        let order = self.order();
                        let ids = self.sel.for_drag(&order, id);
                        // Windows' own drag (to Discord, Explorer...) runs in the frame right after this event; the
                        // Release comes after it ends
                        if let Some(Ok(paths)) = self.svc.as_ref().map(|s| s.drag_paths(&ids)) {
                            if !paths.is_empty() {
                                cx.drag_out(paths.iter().map(|p| p.to_string_lossy().into_owned()).collect());
                            }
                        }
                        self.dragging = Some(ids);
                    }
                }
            }
            Ev::Release(k) if self.keymap.contains_key(k) => {
                self.press = None;
                self.dragging = None;
            }
            Ev::Key(k, vk) if self.keymap.contains_key(k) || *k == K_GAL => {
                let order = self.order();
                match *vk {
                    0x41 if mods(cx).ctrl => self.sel.all(&order),
                    0x2E => {
                        let ids = self.sel.in_order(&order);
                        self.delete(ids, now);
                    }
                    0x1B => self.blur_pending = false,
                    _ => {}
                }
            }
            Ev::Blur(k) if self.keymap.contains_key(k) || *k == K_GAL => {
                if !self.sel.is_empty() {
                    self.blur_pending = true;
                }
            }

            // ---- the selection bar
            Ev::Click(k) if *k == k_sel_copy() => {
                let ids = self.sel.in_order(&self.order());
                self.copy(ids, now);
            }
            Ev::Click(k) if *k == k_sel_del() => {
                let ids = self.sel.in_order(&self.order());
                self.delete(ids, now);
            }
            Ev::Click(k) if *k == k_sel_x() => self.sel.clear(),

            // ---- the folder window
            Ev::Click(k) if *k == sub(K_DLG, "x") || *k == sub(K_DLG, "out") => self.dlg = None,
            Ev::Click(k) if *k == K_DLG_OPEN => {
                if let Some(Err(e)) = self.svc.as_ref().map(|s| s.open_save_dir()) {
                    self.show_toast(format!("Can't open the folder · {e}"), now);
                }
            }
            Ev::Click(k) if *k == K_DLG_CHANGE => self.change_dir(now),

            // ---- the reset line: the frame's shared review over the app's change log, under the link
            Ev::Click(k) if *k == sub(K_RESET, "pc") || *k == sub(K_RESET, "win") => {
                let kind = if *k == sub(K_RESET, "pc") { crate::undo::Kind::HowItWas } else { crate::undo::Kind::WindowsDefaults };
                let b = self.link_box.get(k).copied().unwrap_or((220.0, PAGE_TOP + 261.0, 132.0, 16.0));
                cx.open_reset(kind, b);
            }
            _ => {}
        }
    }
    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let mut layer = El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(WIN_W, WIN_H).no_hit();
        let mut any = false;
        if let Some(t) = self.dlg {
            layer = layer.child(self.folder_window(cx, t));
            any = true;
        }
        if self.lb_up {
            // nothing to see: it only makes the frame's Esc reach `popup_dismiss` while the lightbox is up
            layer = layer.child(El::block().size(0.0, 0.0).no_hit());
            any = true;
        }
        if let Some((t, at)) = self.toast.clone() {
            if cx.now - at < toast::SHOW_MS + 400.0 {
                layer = layer.child(toast::toast(cx, K_TOAST, &t, at, self.sel.len() > 1));
                any = true;
            }
        }
        if let Some(m) = self.ctx_menu(cx) {
            layer = layer.child(m);
            any = true;
        }
        any.then_some(layer)
    }
    fn popup_dismiss(&mut self) {
        if self.lb_up {
            lbhost::close();
        }
        self.ctx = None;
        self.dlg = None;
        // (the toast fades by itself: the frame also calls this on every page scroll while the selection bar's layer
        // exists, which wiped "Copied" at once)
    }
    fn describe(&self) -> String {
        let order = self.order();
        format!(
            "shots={} sel={} dlg={} dir={} wait={} lb={} drag={} toast={}",
            order.len(),
            self.sel.in_order(&order).iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","),
            self.dlg.is_some(),
            self.dir,
            self.wait.is_some(),
            self.lb.map(|l| l.0.to_string()).unwrap_or_default(),
            self.dragging.as_ref().map(|d| d.len()).unwrap_or(0),
            self.toast.as_ref().map(|t| t.0.as_str()).unwrap_or(""),
        )
    }
    /// The reset line's items (below). Cheap: the engine is made only when a line needs it.
    fn resettable(&mut self) -> Option<&mut dyn crate::undo::Resettable> {
        Some(self)
    }
}

// ---------------------------------------------------------------- Order 036: the app's ONE change log
// The drawing's RS.shot lists "Screenshots folder" (the app's own choice; the drawing lists it, so it is in the log) and
// "Print Screen key". This tab changes nothing of Windows for the key: the Screenshot key is the keys manager's
// RegisterHotKey (gone when the app ends); Windows' "Print Screen opens Snipping Tool" value is the Tweaks tab's row
// (`print_screen_snipping`), logged there.
const PAGE: &str = "shot";
const FOLDER: &str = "folder";
const FOLDER_LABEL: &str = "Screenshots folder";

/// Note one change into the change log: at once where the services are free (notes queued before it first), else
/// queued for the main loop. A unit test without the app's services keeps nothing.
fn rec(item: &str, label: &str, old: &crate::undo::Val, new: &crate::undo::Val) {
    let done = crate::services::try_with(|s| {
        crate::undo::flush(&mut s.store);
        let _ = crate::undo::record(&mut s.store, PAGE, item, label, old, new);
    });
    if done.is_none() && !cfg!(test) {
        crate::undo::note(PAGE, item, label, old, new);
    }
}

impl crate::undo::Resettable for Screenshots {
    fn page_id(&self) -> &str {
        PAGE
    }
    fn page_title(&self) -> &str {
        "Screenshots"
    }
    /// The folder now (one small file read).
    fn current(&self, item: &str) -> Option<crate::undo::Val> {
        if item != FOLDER {
            return None;
        }
        let (svc, profile) = self.any_engine();
        Self::folder_val(svc, profile)
    }
    /// No folder chosen: new shots go to Windows' own Screenshots folder.
    fn windows_defaults(&self) -> Vec<crate::undo::DefaultItem> {
        let (svc, profile) = self.any_engine();
        let (Some(now), Ok(def)) = (Self::folder_val(svc, profile), svc.default_save_dir()) else { return Vec::new() };
        let default = crate::undo::Val::new("", &gallery::short_dir(&def, profile));
        vec![crate::undo::DefaultItem { item: FOLDER.into(), label: FOLDER_LABEL.into(), now, default }]
    }
    fn apply(&mut self, item: &str, to: &crate::undo::Val) -> Result<(), String> {
        if item != FOLDER {
            return Err("Not a Screenshots setting".into());
        }
        if crate::testmode::real_read() {
            return Err("A read-only test copy changes nothing".into());
        }
        let (svc, _) = self.any_engine();
        let r = if to.raw.is_empty() { svc.reset_save_dir() } else { svc.set_save_dir(std::path::Path::new(&to.raw)) };
        r.map_err(|e| e.to_string())?;
        // an open page shows the folder now
        self.refresh_dir();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
