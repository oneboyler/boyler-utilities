//! An in-memory stand-in for Windows: monitors with known pictures, a clock, a file system, a clipboard, a Recycle Bin and an
//! Explorer that only record what was asked. Every behaviour of the engine is tested against it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::error::{Error, Result};
use crate::geom::{self, Monitor, Rect, Rotation};
use crate::image::Image;
use crate::naming::LocalTime;
use crate::os::{Capture, CaptureTiming, ColorPath, LiveFeed, Method, MonitorFrame, ScreenshotOs};

/// What the fake clipboard holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clip {
    Picture { png: Vec<u8>, dib: Vec<u8> },
    Files(Vec<PathBuf>),
}

#[derive(Default)]
pub struct State {
    pub monitors: Vec<Monitor>,
    /// The picture each monitor shows now (by number). Missing → a pattern made from the monitor number.
    pub screens: BTreeMap<usize, Image>,
    pub time: LocalTime,
    pub unix_ms: u64,
    pub default_dir: PathBuf,
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    pub dirs: BTreeSet<PathBuf>,
    pub clipboard: Option<Clip>,
    pub recycled: Vec<PathBuf>,
    pub shown: Vec<Vec<PathBuf>>,
    pub opened: Vec<PathBuf>,
    pub pick: Option<PathBuf>,
    /// Folders on a drive with no Recycle Bin.
    pub no_bin: Vec<PathBuf>,
    /// The next capture fails with this.
    pub capture_error: Option<Error>,
    /// Methods this "PC" lacks.
    pub unsupported: Vec<Method>,
    /// Calls to capture, by method.
    pub captures: Vec<Method>,
    /// Bumped by [`FakeOs::change_screens`]; Live feeds compare it.
    pub generation: u64,
}

/// The fake OS layer. Cheap to clone (shared state), so a test can keep a handle and look inside after the engine ran.
#[derive(Clone, Default)]
pub struct FakeOs {
    state: Arc<Mutex<State>>,
}

impl FakeOs {
    /// Two monitors side by side like a common setup: 1920×1080 primary at (0,0) and a 2560×1440 one to its right, top-aligned.
    pub fn two_monitors() -> Self {
        Self::with_monitors(vec![
            mon(0, 0, 1920, 1080, true, false),
            mon(1920, 0, 2560, 1440, false, false),
        ])
    }

    pub fn with_monitors(mut monitors: Vec<Monitor>) -> Self {
        geom::number_monitors(&mut monitors);
        let os = FakeOs::default();
        {
            let mut s = os.state();
            s.monitors = monitors;
            s.time = LocalTime { year: 2026, month: 10, day: 8, hour: 1, minute: 36, second: 5 };
            s.unix_ms = 1_791_416_165_000;
            s.default_dir = PathBuf::from(r"C:\Users\test\Pictures\Screenshots");
            let d = s.default_dir.clone();
            s.dirs.insert(d);
        }
        os
    }

    pub fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The picture monitor `n` shows now (its set picture or the pattern).
    pub fn screen(&self, n: usize) -> Image {
        let s = self.state();
        screen_of(&s, n)
    }

    /// Something moved on screen: every monitor's pattern changes (Live feeds see a new frame).
    pub fn change_screens(&self) {
        let mut s = self.state();
        s.generation += 1;
        let g = s.generation;
        let nums: Vec<usize> = s.monitors.iter().map(|m| m.number).collect();
        for n in nums {
            let img = pattern(&s.monitors[n - 1], g);
            s.screens.insert(n, img);
        }
    }

    pub fn add_dir(&self, p: impl Into<PathBuf>) {
        self.state().dirs.insert(p.into());
    }
}

/// A monitor for fake layouts.
pub fn mon(x: i32, y: i32, w: u32, h: u32, primary: bool, hdr: bool) -> Monitor {
    Monitor {
        number: 0,
        device: String::new(),
        rect: Rect::new(x, y, w, h),
        primary,
        dpi: 96,
        rotation: Rotation::None,
        hdr,
        sdr_white_nits: 80.0,
        handle: 0,
    }
}

/// A picture where every pixel tells where it came from: B = monitor number, G = x mod 251, R = y mod 251 (+ generation).
pub fn pattern(m: &Monitor, generation: u64) -> Image {
    let mut img = Image::black(m.rect.w, m.rect.h);
    for y in 0..m.rect.h {
        for x in 0..m.rect.w {
            img.set_pixel(x, y, [m.number as u8, (x % 251) as u8, ((y as u64 + generation) % 251) as u8, 255]);
        }
    }
    img
}

fn screen_of(s: &State, n: usize) -> Image {
    s.screens.get(&n).cloned().unwrap_or_else(|| {
        let m = s.monitors.iter().find(|m| m.number == n).expect("fake: monitor exists");
        pattern(m, s.generation)
    })
}

fn frames(s: &State, monitors: &[Monitor]) -> Result<Vec<MonitorFrame>> {
    monitors
        .iter()
        .map(|m| {
            if !s.monitors.iter().any(|x| x.number == m.number) {
                return Err(Error::NoMonitor(m.number));
            }
            let color = if m.hdr { ColorPath::HdrConverted } else { ColorPath::Sdr };
            Ok(MonitorFrame { monitor: m.number, image: screen_of(s, m.number), color })
        })
        .collect()
}

impl ScreenshotOs for FakeOs {
    fn monitors(&self) -> Result<Vec<Monitor>> {
        Ok(self.state().monitors.clone())
    }

    fn capture(&self, method: Method, monitors: &[Monitor]) -> Result<Capture> {
        let mut s = self.state();
        s.captures.push(method);
        if s.unsupported.contains(&method) {
            return Err(Error::MethodUnsupported(method.name()));
        }
        if let Some(e) = s.capture_error.take() {
            return Err(e);
        }
        Ok(Capture { frames: frames(&s, monitors)?, timing: CaptureTiming::default() })
    }

    fn live(&self, method: Method, monitors: &[Monitor]) -> Result<Box<dyn LiveFeed>> {
        let s = self.state();
        if s.unsupported.contains(&method) {
            return Err(Error::MethodUnsupported(method.name()));
        }
        Ok(Box::new(FakeLive { os: self.clone(), monitors: monitors.to_vec(), seen: s.generation, stopped: false }))
    }

    fn local_time(&self) -> LocalTime {
        self.state().time
    }

    fn unix_ms(&self) -> u64 {
        self.state().unix_ms
    }

    fn default_save_dir(&self) -> Result<PathBuf> {
        Ok(self.state().default_dir.clone())
    }

    fn read_file(&self, path: &Path) -> Result<Option<Vec<u8>>> {
        Ok(self.state().files.get(path).cloned())
    }

    fn write_file(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        let mut s = self.state();
        let mut p = path.parent();
        while let Some(d) = p {
            if d.as_os_str().is_empty() {
                break;
            }
            s.dirs.insert(d.to_path_buf());
            p = d.parent();
        }
        s.files.insert(path.to_path_buf(), bytes.to_vec());
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        self.state().files.remove(path);
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        let s = self.state();
        s.files.contains_key(path) || s.dirs.contains(path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.state().dirs.contains(path)
    }

    fn set_clipboard(&self, png: &[u8], dib: &[u8]) -> Result<()> {
        self.state().clipboard = Some(Clip::Picture { png: png.to_vec(), dib: dib.to_vec() });
        Ok(())
    }

    fn set_clipboard_files(&self, paths: &[PathBuf]) -> Result<()> {
        self.state().clipboard = Some(Clip::Files(paths.to_vec()));
        Ok(())
    }

    fn recycle(&self, paths: &[PathBuf]) -> Result<()> {
        let mut s = self.state();
        if let Some(p) = paths.iter().find(|p| s.no_bin.iter().any(|d| p.starts_with(d))) {
            return Err(Error::NoRecycleBin(p.clone()));
        }
        for p in paths {
            s.files.remove(p);
            s.recycled.push(p.clone());
        }
        Ok(())
    }

    fn show_in_folder(&self, paths: &[PathBuf]) -> Result<()> {
        self.state().shown.push(paths.to_vec());
        Ok(())
    }

    fn open_folder(&self, dir: &Path) -> Result<()> {
        self.state().opened.push(dir.to_path_buf());
        Ok(())
    }

    fn pick_folder(&self, _start: Option<&Path>) -> Result<PathBuf> {
        self.state().pick.clone().ok_or(Error::Cancelled)
    }
}

struct FakeLive {
    os: FakeOs,
    monitors: Vec<Monitor>,
    seen: u64,
    stopped: bool,
}

impl LiveFeed for FakeLive {
    fn wait(&mut self, _timeout: Duration) -> Result<bool> {
        if self.stopped {
            return Ok(false);
        }
        let g = self.os.state().generation;
        let new = g != self.seen;
        self.seen = g;
        Ok(new)
    }

    fn latest(&mut self) -> Result<Vec<MonitorFrame>> {
        let s = self.os.state();
        frames(&s, &self.monitors)
    }
}
