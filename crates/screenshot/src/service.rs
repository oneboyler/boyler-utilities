//! [`Screenshots`]: the engine the overlay and the Screenshots tab call. Capture (all monitors frozen, one monitor, a region,
//! Live), output (clipboard, PNG into the chosen folder), and the gallery (list, thumbnails, delete to the Recycle Bin, show in
//! folder, the files to drag out).

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::encode::{self, PngLevel};
use crate::error::{Error, Result};
use crate::gallery::{self, Shot};
use crate::geom::{self, Monitor, Rect};
use crate::image::{self, Image};
use crate::naming;
use crate::os::{CaptureTiming, ColorPath, LiveFeed, Method, MonitorFrame, ScreenshotOs};

/// The gallery thumbnail box (16:9). The picture is "contain"-fitted inside; the menu draws the letterbox. 384×216 is twice a
/// ~190 px tile, so it stays sharp at 200 % scaling.
pub const THUMB_W: u32 = 384;
pub const THUMB_H: u32 = 216;

const SETTINGS_FILE: &str = "settings.txt";
const INDEX_FILE: &str = "index.txt";
const THUMBS_DIR: &str = "thumbs";

/// What to capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Every monitor, side by side (the frozen overlay picture, and the "All" button).
    All,
    /// One monitor by number (1, 2 …).
    Monitor(usize),
    /// A region in desktop pixels (a typed or dragged box). Clamped to the desktop; under 4 px a side is refused.
    Region(Rect),
}

/// A frozen picture of part of the desktop, with the layout it came from — the overlay draws it and crops the user's box out
/// of it without capturing again.
#[derive(Debug, Clone)]
pub struct Frozen {
    /// The part of the desktop the picture covers (desktop pixels). For All = the bounding box of every monitor.
    pub area: Rect,
    pub image: Image,
    /// The monitor layout at capture time.
    pub monitors: Vec<Monitor>,
    /// Per captured monitor: how its colours were made (SDR / HDR converted).
    pub color: Vec<(usize, ColorPath)>,
    pub timing: CaptureTiming,
}

impl Frozen {
    /// The pixels of a desktop rectangle (the user's box). It must lie inside [`Frozen::area`].
    pub fn crop(&self, r: &Rect) -> Result<Image> {
        let local = Rect::new(r.x - self.area.x, r.y - self.area.y, r.w, r.h);
        self.image.crop(&local).ok_or(Error::BadRegion("is outside the frozen picture"))
    }

    /// One whole monitor out of the frozen picture.
    pub fn monitor(&self, number: usize) -> Result<Image> {
        let m = self.monitors.iter().find(|m| m.number == number).ok_or(Error::NoMonitor(number))?;
        self.crop(&m.rect)
    }
}

/// A Live source (DESIGN: "Live: the picture keeps moving until you snap").
pub struct Live {
    feed: Box<dyn LiveFeed>,
    area: Rect,
    monitors: Vec<Monitor>,
}

impl Live {
    /// Blocks until the picture changes or `timeout` passes. `Ok(true)` = a new frame — redraw.
    pub fn wait_frame(&mut self, timeout: Duration) -> Result<bool> {
        self.feed.wait(timeout)
    }

    /// The current moment as a frozen picture (Snap / Space / turning Live off).
    pub fn snap(&mut self) -> Result<Frozen> {
        let frames = self.feed.latest()?;
        Ok(assemble(self.area, &self.monitors, frames, CaptureTiming::default()))
    }

    pub fn area(&self) -> Rect {
        self.area
    }
}

/// The Screenshots engine.
pub struct Screenshots<O: ScreenshotOs> {
    os: O,
    method: Method,
    data_dir: PathBuf,
    png_level: PngLevel,
}

impl<O: ScreenshotOs> Screenshots<O> {
    /// `data_dir` holds the engine's own small files (settings, the gallery index, thumbnails) — the app passes
    /// `%LOCALAPPDATA%\BoylerUtilities\screenshots`, tests pass a scratch folder. Uses the faster capture method measured in
    /// the report ([`DEFAULT_METHOD`]).
    pub fn new(os: O, data_dir: impl Into<PathBuf>) -> Self {
        Screenshots { os, method: DEFAULT_METHOD, data_dir: data_dir.into(), png_level: PngLevel::Fast }
    }

    pub fn with_method(mut self, method: Method) -> Self {
        self.method = method;
        self
    }

    pub fn method(&self) -> Method {
        self.method
    }

    pub fn os(&self) -> &O {
        &self.os
    }

    // ---------- capture ----------

    pub fn monitors(&self) -> Result<Vec<Monitor>> {
        self.os.monitors()
    }

    /// Captures every monitor in one go — the frozen picture the overlay opens with.
    pub fn capture_all(&self) -> Result<Frozen> {
        self.capture(Target::All)
    }

    pub fn capture(&self, target: Target) -> Result<Frozen> {
        let monitors = self.os.monitors()?;
        let (area, wanted) = plan(&monitors, target)?;
        let cap = self.os.capture(self.method, &wanted)?;
        Ok(assemble(area, &monitors, cap.frames, cap.timing))
    }

    /// Starts Live for a target (normally [`Target::All`]).
    pub fn live(&self, target: Target) -> Result<Live> {
        let monitors = self.os.monitors()?;
        let (area, wanted) = plan(&monitors, target)?;
        let feed = self.os.live(self.method, &wanted)?;
        Ok(Live { feed, area, monitors })
    }

    // ---------- output ----------

    /// Copy: the picture onto the clipboard as PNG + DIB. Writes no file (DESIGN leaves "does Copy also write a file?"
    /// **unclear**; the menu can call [`Screenshots::save`] too if the owner wants copied shots in the gallery).
    pub fn copy(&self, img: &Image) -> Result<()> {
        let png = encode::png_bytes(img, self.png_level)?;
        self.os.set_clipboard(&png, &encode::dib_bytes(img))
    }

    /// Save: the picture as `Screenshot YYYY-MM-DD HH-MM.png` into the screenshots folder, added to the front of the gallery,
    /// with its thumbnail. Returns the new gallery entry.
    pub fn save(&self, img: &Image) -> Result<Shot> {
        let mut shots = self.read_index()?; // first: a damaged index must not leave an unlisted file behind
        let dir = self.save_dir()?;
        let path = naming::free_path(&dir, &self.os.local_time(), |p| self.os.exists(p));
        let png = encode::png_bytes(img, self.png_level)?;
        self.os.write_file(&path, &png)?;
        let shot = Shot { id: gallery::new_id(&shots, self.os.unix_ms()), path, width: img.width, height: img.height };
        let thumb = img.thumbnail(THUMB_W, THUMB_H);
        self.os.write_file(&self.thumb_path(shot.id), &encode::png_bytes(&thumb, PngLevel::Fast)?)?;
        shots.push(shot.clone());
        self.write_index(&shots)?;
        Ok(shot)
    }

    /// The folder new shots go to: the one the user chose with "Change path", else Windows' Screenshots folder.
    pub fn save_dir(&self) -> Result<PathBuf> {
        match self.saved_dir()? {
            Some(d) => Ok(d),
            None => self.os.default_save_dir(),
        }
    }

    /// The folder chosen with "Change path", if any.
    pub fn saved_dir(&self) -> Result<Option<PathBuf>> {
        let Some(bytes) = self.os.read_file(&self.data_dir.join(SETTINGS_FILE))? else { return Ok(None) };
        let text = String::from_utf8(bytes).map_err(|_| Error::BadData(SETTINGS_FILE.into()))?;
        Ok(text.lines().find_map(|l| l.strip_prefix("save_dir=")).filter(|s| !s.is_empty()).map(PathBuf::from))
    }

    /// "Change path": saves the new folder at once (DESIGN: "The pick is saved at once"). Older shots stay where they are.
    pub fn set_save_dir(&self, dir: &Path) -> Result<()> {
        if !self.os.is_dir(dir) {
            return Err(Error::NotAFolder(dir.to_path_buf()));
        }
        self.os.write_file(&self.data_dir.join(SETTINGS_FILE), format!("save_dir={}\n", dir.display()).as_bytes())
    }

    /// Windows' own Screenshots folder (where shots go when no folder was chosen) - the reset line's "before" value.
    pub fn default_save_dir(&self) -> Result<PathBuf> {
        self.os.default_save_dir()
    }

    /// The reset line ("Back to how your PC was" / "Windows defaults"): forget the chosen folder, so new shots go to
    /// Windows' Screenshots folder again. Older shots stay where they are.
    pub fn reset_save_dir(&self) -> Result<()> {
        if self.saved_dir()?.is_none() {
            return Ok(());
        }
        self.os.write_file(&self.data_dir.join(SETTINGS_FILE), b"save_dir=\n")
    }

    /// The wall-clock time a shot was saved (its caption).
    pub fn shot_time(&self, shot: &Shot) -> crate::naming::LocalTime {
        self.os.local_time_of(shot.saved_unix_ms())
    }

    /// The wall-clock time now.
    pub fn now_local(&self) -> crate::naming::LocalTime {
        self.os.local_time()
    }

    /// "Change path" › Change: Windows' folder picker, then saved at once. Returns the new folder.
    pub fn pick_save_dir(&self) -> Result<PathBuf> {
        let start = self.save_dir().ok();
        let dir = self.os.pick_folder(start.as_deref())?;
        self.set_save_dir(&dir)?;
        Ok(dir)
    }

    /// "Change path" › Open: the screenshots folder in Explorer.
    pub fn open_save_dir(&self) -> Result<()> {
        let dir = self.save_dir()?;
        self.os.open_folder(&dir)
    }

    // ---------- gallery ----------

    /// Every shot this app saved, newest first. Shots whose file was moved or deleted outside the app are dropped from the
    /// index here (with their thumbnails) — DESIGN leaves this **unclear**; this is a call, written in the report.
    pub fn gallery(&self) -> Result<Vec<Shot>> {
        let shots = self.read_index()?;
        let (keep, gone): (Vec<Shot>, Vec<Shot>) = shots.into_iter().partition(|s| self.os.exists(&s.path));
        if !gone.is_empty() {
            self.write_index(&keep)?;
            for s in &gone {
                self.os.remove_file(&self.thumb_path(s.id))?;
            }
        }
        Ok(keep)
    }

    /// A shot's thumbnail (fits 384×216). Made at save time; rebuilt from the PNG if its cache file is missing.
    pub fn thumbnail(&self, id: u64) -> Result<Image> {
        let shot = self.shot(id)?;
        if let Some(bytes) = self.os.read_file(&self.thumb_path(id))? {
            if let Ok(img) = encode::decode_png(&bytes) {
                return Ok(img);
            }
        }
        let bytes = self.os.read_file(&shot.path)?.ok_or(Error::UnknownShot(id))?;
        let thumb = encode::decode_png(&bytes)?.thumbnail(THUMB_W, THUMB_H);
        self.os.write_file(&self.thumb_path(id), &encode::png_bytes(&thumb, PngLevel::Fast)?)?;
        Ok(thumb)
    }

    /// The full picture of a shot (the lightbox, and "Copy" from the gallery).
    pub fn load(&self, id: u64) -> Result<Image> {
        let shot = self.shot(id)?;
        let bytes = self.os.read_file(&shot.path)?.ok_or(Error::UnknownShot(id))?;
        encode::decode_png(&bytes)
    }

    /// Gallery "Copy" / "Copy N screenshots". One shot → the picture (PNG + DIB). Several → the files (as Explorer's Copy does;
    /// pasting into Explorer or Discord gives all N) — the clipboard holds only one picture, and DESIGN does not say what
    /// "Copy N" puts there (**unclear**; this is a call, in the report).
    pub fn copy_shots(&self, ids: &[u64]) -> Result<()> {
        let shots = self.selected(ids)?;
        match shots.as_slice() {
            [] => Err(Error::BadData("empty selection".into())),
            [one] => self.copy(&self.load(one.id)?),
            many => {
                let paths: Vec<PathBuf> = many.iter().map(|s| s.path.clone()).filter(|p| self.os.exists(p)).collect();
                self.os.set_clipboard_files(&paths)
            }
        }
    }

    /// Delete / "Delete N screenshots": to the Recycle Bin, no confirm; then out of the gallery.
    pub fn delete(&self, ids: &[u64]) -> Result<()> {
        let chosen = self.selected(ids)?;
        let paths: Vec<PathBuf> = chosen.iter().map(|s| s.path.clone()).filter(|p| self.os.exists(p)).collect();
        if !paths.is_empty() {
            self.os.recycle(&paths)?;
        }
        let rest: Vec<Shot> = self.read_index()?.into_iter().filter(|s| !ids.contains(&s.id)).collect();
        self.write_index(&rest)?;
        for s in &chosen {
            self.os.remove_file(&self.thumb_path(s.id))?;
        }
        Ok(())
    }

    /// "Show in folder": Explorer with the file(s) selected (one window per folder).
    pub fn show_in_folder(&self, ids: &[u64]) -> Result<()> {
        let paths: Vec<PathBuf> = self.selected(ids)?.into_iter().map(|s| s.path).collect();
        self.os.show_in_folder(&paths)
    }

    /// The files a drag carries (existing ones, in gallery order). The real layer turns them into the shell data object:
    /// [`crate::real::drag_data_object`].
    pub fn drag_paths(&self, ids: &[u64]) -> Result<Vec<PathBuf>> {
        Ok(self.selected(ids)?.into_iter().map(|s| s.path).filter(|p| self.os.exists(p)).collect())
    }

    fn shot(&self, id: u64) -> Result<Shot> {
        self.read_index()?.into_iter().find(|s| s.id == id).ok_or(Error::UnknownShot(id))
    }

    fn selected(&self, ids: &[u64]) -> Result<Vec<Shot>> {
        let all = self.read_index()?;
        for id in ids {
            if !all.iter().any(|s| s.id == *id) {
                return Err(Error::UnknownShot(*id));
            }
        }
        Ok(all.into_iter().filter(|s| ids.contains(&s.id)).collect())
    }

    fn read_index(&self) -> Result<Vec<Shot>> {
        match self.os.read_file(&self.data_dir.join(INDEX_FILE))? {
            None => Ok(Vec::new()),
            Some(b) => gallery::parse(&String::from_utf8(b).map_err(|_| Error::BadData(INDEX_FILE.into()))?),
        }
    }

    fn write_index(&self, shots: &[Shot]) -> Result<()> {
        self.os.write_file(&self.data_dir.join(INDEX_FILE), gallery::render(shots).as_bytes())
    }

    fn thumb_path(&self, id: u64) -> PathBuf {
        self.data_dir.join(THUMBS_DIR).join(format!("{id}.png"))
    }
}

/// The method the engine uses unless told otherwise — the faster one to a frozen frame in the proof run (report §1).
pub const DEFAULT_METHOD: Method = Method::DesktopDuplication;

/// Which desktop area a target covers, and which monitors must be grabbed for it.
pub fn plan(monitors: &[Monitor], target: Target) -> Result<(Rect, Vec<Monitor>)> {
    match target {
        Target::All => Ok((geom::desktop_bounds(monitors), monitors.to_vec())),
        Target::Monitor(n) => {
            let m = monitors.iter().find(|m| m.number == n).ok_or(Error::NoMonitor(n))?;
            Ok((m.rect, vec![m.clone()]))
        }
        Target::Region(r) => {
            let area = geom::clamp_box(&r, &geom::desktop_bounds(monitors)).ok_or(Error::BadRegion("is under 4 px or off the desktop"))?;
            let wanted: Vec<Monitor> = monitors.iter().filter(|m| m.rect.intersect(&area).is_some()).cloned().collect();
            if wanted.is_empty() {
                return Err(Error::BadRegion("touches no monitor"));
            }
            Ok((area, wanted))
        }
    }
}

fn assemble(area: Rect, monitors: &[Monitor], frames: Vec<MonitorFrame>, timing: CaptureTiming) -> Frozen {
    let parts: Vec<(Rect, &Image)> = frames
        .iter()
        .filter_map(|f| monitors.iter().find(|m| m.number == f.monitor).map(|m| (m.rect, &f.image)))
        .collect();
    let image = if let [(r, img)] = parts.as_slice() {
        // One monitor exactly → no copy through a black canvas.
        if *r == area {
            (*img).clone()
        } else {
            image::compose(&area, &parts)
        }
    } else {
        image::compose(&area, &parts)
    };
    Frozen { area, image, monitors: monitors.to_vec(), color: frames.iter().map(|f| (f.monitor, f.color)).collect(), timing }
}
