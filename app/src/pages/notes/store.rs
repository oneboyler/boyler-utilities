//! Order 116: what the Notes tab keeps - its tabs ("books": like browser tabs), each a list of notes (title + text) - and
//! the file it lives in. User data, not a setting: its own file `notes.cfg` next to settings.cfg (so "Reset the app's own
//! settings" never wipes anyone's notes), plain UTF-8 text, written by one background thread (the UI never waits for the
//! disk), the newest content wins.
//!
//! File: first line `boyler-notes 1`, then one line per item, fields split by TAB, `\` `TAB` `newline` escaped:
//! `cur <n>` (the tab shown), `book <name>`, `note <title> <text> <colour 0-3>` (a note belongs to the book line above it; Order 116 files have no colour).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

/// How many paper colours there are (yellow, pink, blue, green).
pub const COLOURS: u8 = 4;

/// One note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// stays the same while the app runs (the page's element keys; not saved)
    pub id: u64,
    pub title: String,
    pub text: String,
    /// Order 119: the paper colour (0 yellow, 1 pink, 2 blue, 3 green); saved with the note
    pub colour: u8,
}

/// One tab of notes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Book {
    pub id: u64,
    pub name: String,
    pub notes: Vec<Note>,
}

/// Every tab of notes, and which one is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notes {
    pub books: Vec<Book>,
    pub cur: usize,
    next: u64,
}

impl Default for Notes {
    fn default() -> Self {
        Notes::new()
    }
}

impl Notes {
    /// No file yet: one empty tab "Notes".
    pub fn new() -> Notes {
        let mut n = Notes { books: Vec::new(), cur: 0, next: 1 };
        let id = n.id();
        n.books.push(Book { id, name: "Notes".to_string(), notes: Vec::new() });
        n
    }

    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next - 1
    }

    /// "+": a new empty tab after the others, shown at once ("Notes 2", "Notes 3"... the lowest number not taken).
    pub fn add_book(&mut self) -> usize {
        let taken = |k: usize| self.books.iter().any(|b| b.name == format!("Notes {k}"));
        let k = (2..).find(|&k| !taken(k)).unwrap_or(2);
        let id = self.id();
        self.books.push(Book { id, name: format!("Notes {k}"), notes: Vec::new() });
        self.cur = self.books.len() - 1;
        self.cur
    }

    /// Double-click: a new name (spaces around it dropped; an empty name keeps the old one).
    pub fn rename_book(&mut self, i: usize, name: &str) {
        let t = name.trim();
        if let (Some(b), false) = (self.books.get_mut(i), t.is_empty()) {
            b.name = t.to_string();
        }
    }

    /// Right-click > delete: the tab and its notes go. The last tab cannot be gone: an empty "Notes" takes its place.
    pub fn delete_book(&mut self, i: usize) {
        if i >= self.books.len() {
            return;
        }
        self.books.remove(i);
        if self.books.is_empty() {
            let id = self.id();
            self.books.push(Book { id, name: "Notes".to_string(), notes: Vec::new() });
        }
        if self.cur > i || (self.cur == i && i > 0) {
            self.cur -= 1;
        }
        self.cur = self.cur.min(self.books.len() - 1);
    }

    /// "+ Note": an empty note at the end of the shown tab; its id.
    pub fn add_note(&mut self) -> u64 {
        let id = self.id();
        let cur = self.cur.min(self.books.len() - 1);
        // the next colour in turn after the shown tab's last note (yellow first)
        let colour = self.books[cur].notes.last().map_or(0, |n| (n.colour + 1) % COLOURS);
        self.books[cur].notes.push(Note { id, title: String::new(), text: String::new(), colour });
        id
    }

    /// The note with this id (any tab).
    pub fn note_mut(&mut self, id: u64) -> Option<&mut Note> {
        self.books.iter_mut().flat_map(|b| b.notes.iter_mut()).find(|n| n.id == id)
    }

    /// Right-click a note > delete.
    pub fn delete_note(&mut self, id: u64) {
        for b in self.books.iter_mut() {
            b.notes.retain(|n| n.id != id);
        }
    }
}

/// The file's name, next to settings.cfg.
pub const FILE_NAME: &str = "notes.cfg";
const HEAD: &str = "boyler-notes 1";

impl Notes {
    /// The file's text.
    pub fn to_text(&self) -> String {
        let mut out = format!("{HEAD}\ncur\t{}\n", self.cur);
        for b in &self.books {
            out += &format!("book\t{}\n", esc(&b.name));
            for n in &b.notes {
                out += &format!("note\t{}\t{}\t{}\n", esc(&n.title), esc(&n.text), n.colour);
            }
        }
        out
    }

    /// Read a file's text; None = not a notes file (or broken).
    pub fn parse(text: &str) -> Option<Notes> {
        let mut lines = text.split('\n');
        if lines.next()?.trim_end_matches('\r') != HEAD {
            return None;
        }
        let mut n = Notes { books: Vec::new(), cur: 0, next: 1 };
        for line in lines {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                [""] => {}
                ["cur", v] => n.cur = v.parse().ok()?,
                ["book", name] => {
                    let id = n.id();
                    n.books.push(Book { id, name: unesc(name)?, notes: Vec::new() });
                }
                ["note", title, body] | ["note", title, body, _] => {
                    let id = n.id();
                    let (title, text) = (unesc(title)?, unesc(body)?);
                    let book = n.books.last_mut()?;
                    // (a note saved by Order 116 has no colour: they take the four in turn)
                    let colour = match f.get(3) {
                        Some(c) => c.parse::<u8>().ok().filter(|c| *c < COLOURS)?,
                        None => book.notes.len() as u8 % COLOURS,
                    };
                    book.notes.push(Note { id, title, text, colour });
                }
                _ => return None,
            }
        }
        if n.books.is_empty() {
            let id = n.id();
            n.books.push(Book { id, name: "Notes".to_string(), notes: Vec::new() });
        }
        n.cur = n.cur.min(n.books.len() - 1);
        Some(n)
    }
}

/// `\` `TAB` `LF` `CR` as `\\` `\t` `\n` `\r` (one line per item, fields split by TAB).
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\t' => o.push_str("\\t"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            c => o.push(c),
        }
    }
    o
}

fn unesc(s: &str) -> Option<String> {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        o.push(match it.next()? {
            '\\' => '\\',
            't' => '\t',
            'n' => '\n',
            'r' => '\r',
            _ => return None,
        });
    }
    Some(o)
}

/// Read the notes from `folder` (no file = one empty tab). A file that is not a notes file is kept aside as
/// `notes.cfg.broken` (a second one as `.broken-<seconds>` - never thrown away) and the page starts empty. None = the file is
/// there but cannot be read now (another program holds it; tried for half a second): the page must NOT save over it.
pub fn load(folder: &Path) -> Option<Notes> {
    let path = folder.join(FILE_NAME);
    let mut tries = 0;
    let bytes = loop {
        match std::fs::read(&path) {
            Ok(b) => break b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(Notes::new()),
            Err(_) if tries < 10 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    };
    match std::str::from_utf8(&bytes).ok().and_then(Notes::parse) {
        Some(n) => Some(n),
        None => {
            let mut aside = folder.join(format!("{FILE_NAME}.broken"));
            if aside.exists() {
                let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                aside = folder.join(format!("{FILE_NAME}.broken-{secs}"));
            }
            if std::fs::rename(&path, &aside).is_err() {
                // could not be kept aside: never written over either
                return None;
            }
            Some(Notes::new())
        }
    }
}

/// The file written whole: a temp file, then renamed over the old one (a crash never leaves half a file).
fn write_file(folder: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(folder)?;
    let tmp = folder.join(format!("{FILE_NAME}.tmp"));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, folder.join(FILE_NAME)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[derive(Default)]
struct Slot {
    /// the newest text not written yet
    text: Option<String>,
    quit: bool,
}

/// The one background writer of an open Notes tab: `send` hands it the newest text (an older one not written yet is
/// dropped); dropping it waits until the newest is on disk (temp file + rename).
pub struct Saver {
    shared: Arc<(Mutex<Slot>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Saver {
    pub fn start(folder: PathBuf) -> Option<Saver> {
        let shared: Arc<(Mutex<Slot>, Condvar)> = Arc::default();
        let sh = shared.clone();
        let thread = std::thread::Builder::new()
            .name("bu-notes-write".into())
            .spawn(move || {
                let (m, cv) = &*sh;
                let Ok(mut g) = m.lock() else { return };
                loop {
                    if let Some(text) = g.text.take() {
                        drop(g);
                        // a write another program refused (it holds the file) is tried again (up to ~5 s), unless a newer
                        // text came meanwhile
                        let mut ok = write_file(&folder, &text).is_ok();
                        let Ok(g2) = m.lock() else { return };
                        g = g2;
                        let mut tries = 0;
                        while !ok && g.text.is_none() && tries < 10 {
                            tries += 1;
                            let Ok((g2, _)) = cv.wait_timeout(g, std::time::Duration::from_millis(500)) else { return };
                            g = g2;
                            if g.text.is_none() {
                                drop(g);
                                ok = write_file(&folder, &text).is_ok();
                                let Ok(g2) = m.lock() else { return };
                                g = g2;
                            }
                        }
                        continue;
                    }
                    if g.quit {
                        return;
                    }
                    let Ok(g2) = cv.wait(g) else { return };
                    g = g2;
                }
            })
            .ok()?;
        Some(Saver { shared, thread: Some(thread) })
    }

    pub fn send(&self, text: String) {
        let (m, cv) = &*self.shared;
        if let Ok(mut g) = m.lock() {
            g.text = Some(text);
            cv.notify_all();
        }
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        let (m, cv) = &*self.shared;
        if let Ok(mut g) = m.lock() {
            g.quit = true;
            cv.notify_all();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_with_one_empty_tab_called_notes() {
        let n = Notes::new();
        assert_eq!(n.books.len(), 1);
        assert_eq!(n.books[0].name, "Notes");
        assert!(n.books[0].notes.is_empty());
        assert_eq!(n.cur, 0);
    }

    /// Order 119: new notes take the four paper colours in turn (yellow first), saved with the note.
    #[test]
    fn new_notes_take_the_four_colours_in_turn_and_keep_them() {
        let mut n = Notes::new();
        for _ in 0..5 {
            n.add_note();
        }
        let colours = |n: &Notes| n.books[0].notes.iter().map(|x| x.colour).collect::<Vec<_>>();
        assert_eq!(colours(&n), [0, 1, 2, 3, 0]);
        // deleting one does not recolour the others; the next one follows the last
        let id = n.books[0].notes[1].id;
        n.delete_note(id);
        assert_eq!(colours(&n), [0, 2, 3, 0]);
        n.add_note();
        assert_eq!(colours(&n), [0, 2, 3, 0, 1]);
        let back = Notes::parse(&n.to_text()).unwrap();
        assert_eq!(colours(&back), [0, 2, 3, 0, 1]);
    }

    /// Order 119: a file written by Order 116 (no colour) loads, its notes taking the colours in turn; a bad colour is a broken file.
    #[test]
    fn a_file_without_colours_loads_and_a_bad_colour_does_not() {
        let old = format!("{HEAD}\ncur\t0\nbook\tNotes\nnote\ta\tb\nnote\tc\td\nnote\te\tf\n");
        let n = Notes::parse(&old).unwrap();
        assert_eq!(n.books[0].notes.iter().map(|x| x.colour).collect::<Vec<_>>(), [0, 1, 2]);
        assert!(Notes::parse(&format!("{HEAD}\nbook\tNotes\nnote\ta\tb\t4\n")).is_none());
        assert!(Notes::parse(&format!("{HEAD}\nbook\tNotes\nnote\ta\tb\tx\n")).is_none());
    }

    fn names(n: &Notes) -> Vec<&str> {
        n.books.iter().map(|b| b.name.as_str()).collect()
    }

    #[test]
    fn plus_adds_a_numbered_tab_and_shows_it() {
        let mut n = Notes::new();
        assert_eq!(n.add_book(), 1);
        assert_eq!(n.add_book(), 2);
        assert_eq!(names(&n), ["Notes", "Notes 2", "Notes 3"]);
        assert_eq!(n.cur, 2);
        // a freed number is taken again
        n.delete_book(1);
        n.add_book();
        assert_eq!(names(&n), ["Notes", "Notes 3", "Notes 2"]);
    }

    #[test]
    fn rename_trims_and_an_empty_name_keeps_the_old_one() {
        let mut n = Notes::new();
        n.rename_book(0, "  Work  ");
        assert_eq!(n.books[0].name, "Work");
        n.rename_book(0, "   ");
        assert_eq!(n.books[0].name, "Work");
    }

    #[test]
    fn deleting_tabs_keeps_the_shown_one_and_never_leaves_none() {
        let mut n = Notes::new();
        n.add_book();
        n.add_book();
        n.cur = 2;
        n.delete_book(0);
        assert_eq!(names(&n), ["Notes 2", "Notes 3"]);
        assert_eq!(n.cur, 1, "still on Notes 3");
        n.delete_book(1);
        assert_eq!(n.cur, 0, "the shown one went: the one before it shows");
        n.delete_book(0);
        assert_eq!(names(&n), ["Notes"]);
        assert!(n.books[0].notes.is_empty());
    }

    #[test]
    fn notes_are_added_to_the_shown_tab_edited_and_deleted() {
        let mut n = Notes::new();
        n.add_book();
        let a = n.add_note();
        let b = n.add_note();
        assert_ne!(a, b);
        assert!(n.books[0].notes.is_empty());
        assert_eq!(n.books[1].notes.len(), 2);
        n.note_mut(a).unwrap().title = "Shopping".into();
        n.note_mut(b).unwrap().text = "milk\neggs".into();
        n.delete_note(a);
        assert_eq!(n.books[1].notes.len(), 1);
        assert_eq!(n.books[1].notes[0].text, "milk\neggs");
        assert!(n.note_mut(a).is_none());
    }

    /// What a test compares: names, titles, texts, the shown tab (ids are the running app's own).
    fn content(n: &Notes) -> (usize, Vec<(String, Vec<(String, String)>)>) {
        (n.cur, n.books.iter().map(|b| (b.name.clone(), b.notes.iter().map(|x| (x.title.clone(), x.text.clone())).collect())).collect())
    }

    fn sample() -> Notes {
        let mut n = Notes::new();
        let a = n.add_note();
        n.note_mut(a).unwrap().title = "Shop\tping \\ list".into();
        n.note_mut(a).unwrap().text = "milk\neggs\n\nčaj ☕\r\nend".into();
        n.add_note();
        n.add_book();
        n.rename_book(1, "Work stuff");
        let b = n.add_note();
        n.note_mut(b).unwrap().text = "line\\n not a break".into();
        n
    }

    #[test]
    fn the_file_text_reads_back_the_same_notes() {
        let n = sample();
        let back = Notes::parse(&n.to_text()).expect("parses");
        assert_eq!(content(&back), content(&n));
        assert!(n.to_text().starts_with("boyler-notes 1\n"));
        // every note and tab gets its own id again
        let mut ids: Vec<u64> = back.books.iter().map(|b| b.id).chain(back.books.iter().flat_map(|b| b.notes.iter().map(|x| x.id))).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 2 + 3);
    }

    #[test]
    fn a_text_that_is_not_a_notes_file_is_refused() {
        assert!(Notes::parse("").is_none());
        assert!(Notes::parse("hello\nbook\tA\n").is_none());
        assert!(Notes::parse("boyler-notes 1\nnote\tbefore any book\t\n").is_none());
        // no tab at all = one empty tab (a file written with nothing in it reads fine)
        let n = Notes::parse("boyler-notes 1\n").unwrap();
        assert_eq!(content(&n), content(&Notes::new()));
        // a shown tab past the end is the last one
        let n = Notes::parse("boyler-notes 1\ncur\t9\nbook\tA\nbook\tB\n").unwrap();
        assert_eq!(n.cur, 1);
    }


    #[test]
    fn notes_survive_a_restart_through_the_file() {
        let sc = crate::settings::scratch::Scratch::new("notes-116");
        let n = sample();
        assert_eq!(content(&load(sc.dir()).unwrap()), content(&Notes::new()), "no file yet: one empty tab");
        {
            let saver = Saver::start(sc.dir().to_path_buf()).unwrap();
            saver.send(Notes::new().to_text());
            saver.send(n.to_text());
        } // dropped: the newest is on disk
        assert_eq!(content(&load(sc.dir()).unwrap()), content(&n));
        assert!(!sc.dir().join("notes.cfg.tmp").exists());
    }

    #[test]
    fn a_file_another_program_holds_is_not_taken_for_no_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let sc = crate::settings::scratch::Scratch::new("notes-116-locked");
        std::fs::create_dir_all(sc.dir()).unwrap();
        std::fs::write(sc.dir().join(FILE_NAME), sample().to_text()).unwrap();
        {
            // held with no sharing (as a backup / sync tool may): it cannot be read now
            let _held = std::fs::OpenOptions::new().read(true).share_mode(0).open(sc.dir().join(FILE_NAME)).unwrap();
            assert!(load(sc.dir()).is_none(), "unreadable is not 'no notes'");
        }
        assert_eq!(content(&load(sc.dir()).unwrap()), content(&sample()), "free again: read");
        // a second broken file does not replace the first kept copy
        std::fs::write(sc.dir().join(FILE_NAME), "garbage 1").unwrap();
        load(sc.dir()).unwrap();
        std::fs::write(sc.dir().join(FILE_NAME), "garbage 2").unwrap();
        load(sc.dir()).unwrap();
        assert_eq!(std::fs::read_to_string(sc.dir().join("notes.cfg.broken")).unwrap(), "garbage 1");
        let kept = std::fs::read_dir(sc.dir()).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().starts_with("notes.cfg.broken-")).count();
        assert_eq!(kept, 1);
    }

    #[test]
    fn a_broken_file_is_kept_aside_not_lost() {
        let sc = crate::settings::scratch::Scratch::new("notes-116-broken");
        std::fs::create_dir_all(sc.dir()).unwrap();
        std::fs::write(sc.dir().join(FILE_NAME), "garbage").unwrap();
        assert_eq!(content(&load(sc.dir()).unwrap()), content(&Notes::new()));
        assert_eq!(std::fs::read_to_string(sc.dir().join("notes.cfg.broken")).unwrap(), "garbage");
    }
}
