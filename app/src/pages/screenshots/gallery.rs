//! The Screenshots page's plain logic (Order 019): the Explorer-style selection, the gallery grid's geometry (for the
//! FLIP glides), the folder shown short, and the drawing's sample gallery for test copies (FAKE service only).

use std::path::{Path, PathBuf};

use bu_screenshot::fake::FakeOs;
use bu_screenshot::gallery::{self, Shot};
use bu_screenshot::naming::{self, days_from_civil, LocalTime};
use bu_screenshot::ScreenshotOs;

// ---------------------------------------------------------------- selection
/// The modifier keys held during a click.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
}

/// What the gallery has selected (ids in no order; `anchor` = the last plainly / Ctrl-clicked one).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sel {
    pub ids: Vec<u64>,
    pub anchor: Option<u64>,
}

impl Sel {
    pub fn has(&self, id: u64) -> bool {
        self.ids.contains(&id)
    }
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    pub fn clear(&mut self) {
        self.ids.clear();
        self.anchor = None;
    }
    /// The selected ids in gallery order (`order` = the gallery, newest first).
    pub fn in_order(&self, order: &[u64]) -> Vec<u64> {
        order.iter().copied().filter(|i| self.has(*i)).collect()
    }
    /// Only ids still in the gallery stay selected (a shot went away).
    pub fn keep_only(&mut self, order: &[u64]) {
        self.ids.retain(|i| order.contains(i));
        if self.anchor.is_some_and(|a| !order.contains(&a)) {
            self.anchor = None;
        }
    }

    /// A click on `it` the usual Explorer way (the drawing's shotClick): plain = just this one; Ctrl = add / remove it
    /// (and it becomes the anchor); Shift = everything from the anchor to it (the anchor stays), Ctrl+Shift = add that range.
    pub fn click(&mut self, order: &[u64], it: u64, m: Mods) {
        if m.shift {
            if let Some(a) = self.anchor.filter(|a| order.contains(a)) {
                let (ia, ib) = (order.iter().position(|x| *x == a).unwrap_or(0), order.iter().position(|x| *x == it).unwrap_or(0));
                let range = &order[ia.min(ib)..=ia.max(ib)];
                if !m.ctrl {
                    self.ids.clear();
                }
                for x in range {
                    if !self.has(*x) {
                        self.ids.push(*x);
                    }
                }
                return;
            }
        }
        if m.ctrl {
            if let Some(p) = self.ids.iter().position(|x| *x == it) {
                self.ids.remove(p);
            } else {
                self.ids.push(it);
            }
            self.anchor = Some(it);
            return;
        }
        self.ids = vec![it];
        self.anchor = Some(it);
    }

    /// Ctrl+A: every shot (the anchor stays).
    pub fn all(&mut self, order: &[u64]) {
        self.ids = order.to_vec();
    }

    /// A drag starts on `it`: one that isn't selected is picked alone (like Explorer); the drag carries the selection.
    pub fn for_drag(&mut self, order: &[u64], it: u64) -> Vec<u64> {
        if !self.has(it) {
            self.ids = vec![it];
            self.anchor = Some(it);
        }
        self.in_order(order)
    }
}

// ---------------------------------------------------------------- grid geometry
/// `.gal{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:10px 8px}` inside the 548 px page content box.
pub const COLS: usize = 4;
pub const GAL_W: f32 = 548.0;
pub const COL_GAP: f32 = 8.0;
pub const ROW_GAP: f32 = 10.0;
/// one column: (548 - 3 x 8) / 4 = 131
pub const CELL_W: f32 = (GAL_W - COL_GAP * (COLS as f32 - 1.0)) / COLS as f32;
/// `.sth{aspect-ratio:16/9}` = 73.6875; `.scap{margin:6px 2px 0;line-height:14px}` under it
pub const THUMB_H: f32 = CELL_W * 9.0 / 16.0;
pub const CELL_H: f32 = THUMB_H + 6.0 + 14.0;

/// Where gallery cell `i` sits inside the grid (top-left).
pub fn cell_pos(i: usize) -> (f32, f32) {
    ((i % COLS) as f32 * (CELL_W + COL_GAP), (i / COLS) as f32 * (CELL_H + ROW_GAP))
}

// ---------------------------------------------------------------- text
/// `.scap` right side: `1920×1080` (the drawing's szTxt).
pub fn size_text(w: u32, h: u32) -> String {
    format!("{w}\u{d7}{h}")
}

/// A folder as the page shows it: under the user's own profile folder it is written from there ("Pictures\Screenshots",
/// as the drawing writes it - and no user name on screen); anything else in full ("D:\Clips\Screenshots").
pub fn short_dir(dir: &Path, profile: Option<&Path>) -> String {
    if let Some(p) = profile {
        if let Ok(rest) = dir.strip_prefix(p) {
            let s = rest.display().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    dir.display().to_string()
}

/// The user's profile folder (`%USERPROFILE%`), for [`short_dir`]. The fake's is `C:\Users\test`.
pub fn profile_dir(fake: bool) -> Option<PathBuf> {
    if fake {
        return Some(PathBuf::from(r"C:\Users\test"));
    }
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

// ---------------------------------------------------------------- the drawing's sample gallery (test copies)
/// The fake engine's own folder (in the fake's memory only - nothing is written to the disk).
pub const FAKE_DATA: &str = r"C:\BU-test\screenshots";

/// The drawing's eight sample shots (menu-v22 `mkShot(...)`, newest first), sizes and caption times as drawn. Their pictures
/// are the drawing's own thumbnail canvases (132 x 74 bitmaps) exactly as the reference Chromium shows them in their cells
/// (scaled into the pixel-snapped 131 x 74 box - 131 x 73 in the second row; scratch tools thumbs.js + canvasprobe.js):
/// sample values, so a test picture compares the page and not two image scalers.
/// The fake clock is Thu 2026-10-08 21:37, so the captions come out "21:34" ... "Yesterday", "Yesterday", "Mon".
pub const SAMPLE_PNG: [&[u8]; 8] = [
    include_bytes!("sample/thumb0.png"),
    include_bytes!("sample/thumb1.png"),
    include_bytes!("sample/thumb2.png"),
    include_bytes!("sample/thumb3.png"),
    include_bytes!("sample/thumb4.png"),
    include_bytes!("sample/thumb5.png"),
    include_bytes!("sample/thumb6.png"),
    include_bytes!("sample/thumb7.png"),
];
/// (y, m, d, h, min, width, height, folder) - "Desktop" / "D:\Clips\Screenshots" as in the drawing; None = the default folder.
const SAMPLES: [(u16, u8, u8, u8, u8, u32, u32, Option<&str>); 8] = [
    (2026, 10, 8, 21, 34, 1920, 1080, None),
    (2026, 10, 8, 21, 31, 640, 300, None),
    (2026, 10, 8, 21, 12, 1920, 1080, None),
    (2026, 10, 8, 20, 47, 1920, 1080, None),
    (2026, 10, 8, 20, 5, 3840, 1080, None),
    (2026, 10, 7, 18, 22, 1920, 1080, Some(r"C:\Users\test\Desktop")),
    (2026, 10, 7, 14, 9, 1920, 1080, Some(r"D:\Clips\Screenshots")),
    (2026, 10, 5, 11, 40, 1920, 1080, None),
];
/// The fake clock (the fake's rule is UTC: wall clock = UTC).
pub const FAKE_NOW: LocalTime = LocalTime { year: 2026, month: 10, day: 8, hour: 21, minute: 37, second: 0 };

fn unix_ms_of(t: &LocalTime) -> u64 {
    let d = days_from_civil(t.year as i64, t.month, t.day);
    ((d * 86_400 + t.hour as i64 * 3600 + t.minute as i64 * 60 + t.second as i64) * 1000) as u64
}

/// The fake engine a test copy uses: the drawing's sample gallery, its clock, its folders.
pub(crate) fn sample_engine() -> (FakeOs, super::Eng<FakeOs>) {
    let os = FakeOs::two_monitors();
    {
        let mut s = os.state();
        s.time = FAKE_NOW;
        s.unix_ms = unix_ms_of(&FAKE_NOW);
        for d in [r"C:\Users\test\Desktop", r"D:\Clips\Screenshots"] {
            s.dirs.insert(PathBuf::from(d));
        }
    }
    let data = PathBuf::from(FAKE_DATA);
    let default_dir = os.default_save_dir().unwrap_or_default();
    let mut shots = Vec::new();
    for (i, (y, mo, d, h, mi, w, hh, dir)) in SAMPLES.iter().enumerate() {
        let t = LocalTime { year: *y, month: *mo, day: *d, hour: *h, minute: *mi, second: 0 };
        let folder = dir.map(PathBuf::from).unwrap_or_else(|| default_dir.clone());
        let path = folder.join(format!("{}.png", naming::base_name(&t)));
        let id = unix_ms_of(&t);
        let _ = os.write_file(&path, SAMPLE_PNG[i]);
        let _ = os.write_file(&data.join("thumbs").join(format!("{id}.png")), SAMPLE_PNG[i]);
        shots.push(Shot { id, path, width: *w, height: *hh });
    }
    let _ = os.write_file(&data.join("index.txt"), gallery::render(&shots).as_bytes());
    let eng = super::Eng::new(os.clone(), data);
    (os, eng)
}

#[cfg(test)]
mod tests {
    use super::*;

    const O: [u64; 6] = [10, 20, 30, 40, 50, 60];
    const N: Mods = Mods { ctrl: false, shift: false };
    const C: Mods = Mods { ctrl: true, shift: false };
    const S: Mods = Mods { ctrl: false, shift: true };
    const CS: Mods = Mods { ctrl: true, shift: true };

    #[test]
    fn selection_works_like_explorer() {
        let mut s = Sel::default();
        s.click(&O, 20, N);
        assert_eq!((s.ids.clone(), s.anchor), (vec![20], Some(20)));
        s.click(&O, 40, C);
        assert_eq!(s.in_order(&O), vec![20, 40]);
        assert_eq!(s.anchor, Some(40));
        s.click(&O, 20, C); // Ctrl on a selected one takes it out
        assert_eq!(s.in_order(&O), vec![40]);
        s.click(&O, 60, S); // Shift: the anchor (20, the last Ctrl-clicked, as in the drawing) .. 60
        assert_eq!(s.in_order(&O), vec![20, 30, 40, 50, 60]);
        assert_eq!(s.anchor, Some(20));
        s.click(&O, 40, S); // Shift again: the range from the same anchor, the rest dropped
        assert_eq!(s.in_order(&O), vec![20, 30, 40]);
        s.click(&O, 60, C); // Ctrl adds 60 (the anchor moves to 60)
        s.click(&O, 50, CS); // Ctrl+Shift adds the range 50..60 to what is there
        assert_eq!(s.in_order(&O), vec![20, 30, 40, 50, 60]);
        s.click(&O, 30, N);
        assert_eq!(s.in_order(&O), vec![30]);
        s.all(&O);
        assert_eq!(s.len(), 6);
        s.clear();
        assert!(s.is_empty() && s.anchor.is_none());
        // Shift with no anchor = a plain click
        s.click(&O, 50, S);
        assert_eq!((s.ids.clone(), s.anchor), (vec![50], Some(50)));
    }

    #[test]
    fn drag_takes_the_selection_or_just_the_one_under_the_mouse() {
        let mut s = Sel::default();
        s.click(&O, 20, N);
        s.click(&O, 30, C);
        assert_eq!(s.for_drag(&O, 30), vec![20, 30]);
        assert_eq!(s.for_drag(&O, 60), vec![60]);
        assert_eq!(s.ids, vec![60]);
    }

    #[test]
    fn gone_shots_leave_the_selection() {
        let mut s = Sel::default();
        s.click(&O, 20, N);
        s.click(&O, 30, C);
        s.keep_only(&[10, 20, 40]);
        assert_eq!(s.ids, vec![20]);
        assert_eq!(s.anchor, None);
    }

    #[test]
    fn grid_is_the_drawings() {
        // Chromium (dom_dump of menu-v22 page shot): .shot 131 x 93.6875, the second row at +103.6875, columns every 139
        assert_eq!(CELL_W, 131.0);
        assert_eq!(THUMB_H, 73.6875);
        assert_eq!(cell_pos(1), (139.0, 0.0));
        assert_eq!(cell_pos(5), (139.0, 103.6875));
    }

    #[test]
    fn folders_are_shown_from_the_profile() {
        let p = Path::new(r"C:\Users\test");
        assert_eq!(short_dir(Path::new(r"C:\Users\test\Pictures\Screenshots"), Some(p)), r"Pictures\Screenshots");
        assert_eq!(short_dir(Path::new(r"D:\Clips\Screenshots"), Some(p)), r"D:\Clips\Screenshots");
        assert_eq!(short_dir(p, Some(p)), r"C:\Users\test");
        assert_eq!(size_text(1920, 1080), "1920\u{d7}1080");
    }

    #[test]
    fn the_sample_gallery_has_the_drawings_captions() {
        let (_os, e) = sample_engine();
        let g = e.gallery().unwrap();
        let now = e.now_local();
        let caps: Vec<String> = g.iter().map(|s| naming::caption(&now, &e.shot_time(s))).collect();
        assert_eq!(caps, ["21:34", "21:31", "21:12", "20:47", "20:05", "Yesterday", "Yesterday", "Mon"]);
        let sizes: Vec<String> = g.iter().map(|s| size_text(s.width, s.height)).collect();
        assert_eq!(sizes[1], "640\u{d7}300");
        assert_eq!(sizes[4], "3840\u{d7}1080");
        // thumbnails come from the cache (the drawing's pictures as Chromium shows them, 131 x 74 / 73)
        let t = e.thumbnail(g[0].id).unwrap();
        assert_eq!((t.width, t.height), (131, 74));
        assert_eq!(e.thumbnail(g[7].id).unwrap().height, 73);
    }
}
