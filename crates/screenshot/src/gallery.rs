//! The gallery index: the small file listing every shot THIS app saved, wherever it was saved (DESIGN: "every screenshot this
//! app took … each shot remembers its own folder"). That is how "ours" are told apart from Windows' own shots in the same folder.
//!
//! Format (UTF-8 text, one shot per line, tab-separated — Windows paths cannot contain tabs or line breaks):
//! ```text
//! bu-screenshot index 1
//! <id>\t<width>\t<height>\t<full path>
//! ```
//! `id` = the save time in milliseconds since 1970 (UTC), made unique by +1 if two shots land in the same millisecond.

use std::path::PathBuf;

use crate::error::{Error, Result};

pub const HEADER: &str = "bu-screenshot index 1";

/// One saved shot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shot {
    pub id: u64,
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
}

impl Shot {
    /// When it was saved (ms since 1970, UTC) — the gallery caption's time.
    pub fn saved_unix_ms(&self) -> u64 {
        self.id
    }
}

/// Reads the index text. Newest first.
pub fn parse(text: &str) -> Result<Vec<Shot>> {
    let mut lines = text.lines();
    match lines.next() {
        None => return Ok(Vec::new()),
        Some(h) if h.trim_start_matches('\u{feff}') == HEADER => {}
        Some(_) => return Err(Error::BadData("gallery index header".into())),
    }
    let mut shots = Vec::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let mut f = line.splitn(4, '\t');
        let (Some(id), Some(w), Some(h), Some(p)) = (f.next(), f.next(), f.next(), f.next()) else {
            return Err(Error::BadData("gallery index line".into()));
        };
        let num = |s: &str| s.parse::<u64>().map_err(|_| Error::BadData("gallery index number".into()));
        shots.push(Shot { id: num(id)?, width: num(w)? as u32, height: num(h)? as u32, path: PathBuf::from(p) });
    }
    shots.sort_by_key(|s| std::cmp::Reverse(s.id));
    Ok(shots)
}

/// Writes the index text (newest first).
pub fn render(shots: &[Shot]) -> String {
    let mut s = String::from(HEADER);
    s.push('\n');
    let mut sorted: Vec<&Shot> = shots.iter().collect();
    sorted.sort_by_key(|s| std::cmp::Reverse(s.id));
    for sh in sorted {
        s.push_str(&format!("{}\t{}\t{}\t{}\n", sh.id, sh.width, sh.height, sh.path.display()));
    }
    s
}

/// A fresh id for a shot saved at `now_ms`: unique against the ids already in the index.
pub fn new_id(shots: &[Shot], now_ms: u64) -> u64 {
    let max = shots.iter().map(|s| s.id).max().unwrap_or(0);
    now_ms.max(max + 1)
}
