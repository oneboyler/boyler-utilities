//! Settings store (Order 014 change 5): ONE file holds everything the app remembers.
//! - normal runs: `%APPDATA%\Boyler Utilities\settings.cfg` ([`SettingsStore::default_folder`]); tests pass a scratch folder
//!   to [`SettingsStore::open`] — nothing here reads %APPDATA% on its own;
//! - three scopes: [`Scope::App`] (glass style, keys, app switches), [`Scope::Page`] (each page's own settings, namespaced
//!   by page id) and [`Scope::Pc`] ("how your PC was" — the undo module's records; "Reset the app's own settings" keeps it);
//! - typed get / set: bool, i64, f64, string, string list; a value of another type reads as "not set";
//! - every set writes the whole file at once (temp file + rename) and only when the value really changed;
//! - Order 050: the app's store ([`SettingsStore::write_in_background`]) hands each new file to ONE writer thread - the
//!   UI thread never waits for the disk (a flush to disk takes 10-100+ ms, seconds while the disk is busy); the writer
//!   always writes the newest file (several changes in a row = one write) and dropping the store waits for it;
//! - a broken file is kept aside as `settings.cfg.broken` and the defaults are used ([`SettingsStore::load_note`]).
//!   The file is plain UTF-8 text, one value per line: `section TAB key TAB type TAB value` (tab, newline, `\` escaped).

mod format;
mod glass;
mod theme;
#[cfg(test)]
pub(crate) mod scratch;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

pub use glass::{GlassNumbers, GlassStyle};
pub use theme::Theme;

/// The file's name inside the settings folder.
pub const FILE_NAME: &str = "settings.cfg";
/// The folder's name under %APPDATA%.
pub const FOLDER_NAME: &str = "Boyler Utilities";

const GLASS_KEY: &str = "glass";
const THEME_KEY: &str = "theme";

/// Where a value lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope<'a> {
    /// The app's own settings (glass style, keys, …).
    App,
    /// One page's own settings; the id is the page's id (e.g. "audio").
    Page(&'a str),
    /// "How your PC was" records — only the undo module writes here; survives "Reset the app's own settings".
    Pc,
}

impl Scope<'_> {
    fn section(&self) -> String {
        match self {
            Scope::App => "app".to_string(),
            Scope::Page(id) => format!("page:{id}"),
            Scope::Pc => "pc".to_string(),
        }
    }
}

/// One stored value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<String>),
}

impl Value {
    /// Same value (floats compared bit for bit, so NaN == NaN and a write is skipped).
    fn same(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
            _ => self == other,
        }
    }
}

/// What happened when the file was read (None = read fine or no file yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadNote {
    /// The file could not be understood: it was kept aside at this path; defaults are used.
    Broken { kept_as: PathBuf },
    /// The file could not be understood and could not be kept aside either (the reason); defaults are used.
    BrokenNotKept(String),
    /// The file exists but could not be read (the reason); defaults are used.
    Unreadable(String),
}

/// The settings store. One per app; the app passes the real folder, tests a scratch folder.
pub struct SettingsStore {
    folder: PathBuf,
    values: BTreeMap<(String, String), Value>,
    note: Option<LoadNote>,
    /// Order 050: the background writer (None = every write on the caller's thread - unit tests)
    writer: Option<writer::Writer>,
}

impl SettingsStore {
    /// The real folder: `%APPDATA%\Boyler Utilities`. Only the app calls this — never a test.
    pub fn default_folder() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join(FOLDER_NAME))
    }

    /// Read the store from `folder` (created on the first write). A missing file = defaults; a broken one is kept aside.
    pub fn open(folder: impl Into<PathBuf>) -> Self {
        let folder = folder.into();
        let path = folder.join(FILE_NAME);
        let mut store = SettingsStore { folder, values: BTreeMap::new(), note: None, writer: None };
        match std::fs::read(&path) {
            Ok(bytes) => match format::parse(&bytes) {
                Ok(values) => store.values = values,
                Err(_) => store.note = Some(keep_aside(&path)),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => store.note = Some(LoadNote::Unreadable(e.to_string())),
        }
        store
    }

    /// Order 050: from now on every write goes to one background writer thread (the app's store: the UI thread never
    /// waits for the disk). [`SettingsStore::wait_written`] / dropping the store waits until the newest file is on disk.
    pub fn write_in_background(&mut self) {
        if self.writer.is_none() {
            self.writer = writer::Writer::start(self.folder.clone());
        }
    }

    /// Wait (at most `max`) until every change so far is on disk. true = all written (or no background writer).
    pub fn wait_written(&self, max: std::time::Duration) -> bool {
        self.writer.as_ref().is_none_or(|w| w.wait(max))
    }

    /// The settings file's full path.
    pub fn path(&self) -> PathBuf {
        self.folder.join(FILE_NAME)
    }

    /// The folder the store lives in.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// What went wrong while reading (None = fine).
    pub fn load_note(&self) -> Option<&LoadNote> {
        self.note.as_ref()
    }

    // ---- reading

    /// The raw value (any type).
    pub fn get(&self, scope: Scope, key: &str) -> Option<&Value> {
        self.values.get(&(scope.section(), key.to_string()))
    }

    pub fn get_bool(&self, scope: Scope, key: &str) -> Option<bool> {
        match self.get(scope, key) {
            Some(Value::Bool(v)) => Some(*v),
            _ => None,
        }
    }

    pub fn get_i64(&self, scope: Scope, key: &str) -> Option<i64> {
        match self.get(scope, key) {
            Some(Value::Int(v)) => Some(*v),
            _ => None,
        }
    }

    pub fn get_f64(&self, scope: Scope, key: &str) -> Option<f64> {
        match self.get(scope, key) {
            Some(Value::Float(v)) => Some(*v),
            _ => None,
        }
    }

    pub fn get_str(&self, scope: Scope, key: &str) -> Option<&str> {
        match self.get(scope, key) {
            Some(Value::Str(v)) => Some(v),
            _ => None,
        }
    }

    pub fn get_list(&self, scope: Scope, key: &str) -> Option<&[String]> {
        match self.get(scope, key) {
            Some(Value::List(v)) => Some(v),
            _ => None,
        }
    }

    pub fn bool_or(&self, scope: Scope, key: &str, default: bool) -> bool {
        self.get_bool(scope, key).unwrap_or(default)
    }

    pub fn i64_or(&self, scope: Scope, key: &str, default: i64) -> i64 {
        self.get_i64(scope, key).unwrap_or(default)
    }

    pub fn f64_or(&self, scope: Scope, key: &str, default: f64) -> f64 {
        self.get_f64(scope, key).unwrap_or(default)
    }

    pub fn str_or<'a>(&'a self, scope: Scope, key: &str, default: &'a str) -> &'a str {
        self.get_str(scope, key).unwrap_or(default)
    }

    /// Every key set in one scope, sorted.
    pub fn keys(&self, scope: Scope) -> Vec<&str> {
        let s = scope.section();
        self.values.keys().filter(|(sec, _)| *sec == s).map(|(_, k)| k.as_str()).collect()
    }

    // ---- writing (each returns Ok(true) when the value changed and the file was written)

    /// Set any value; writes the file only when it changed. On a write error the new value stays in memory.
    pub fn set(&mut self, scope: Scope, key: &str, value: Value) -> io::Result<bool> {
        let k = (scope.section(), key.to_string());
        if self.values.get(&k).is_some_and(|old| old.same(&value)) {
            return Ok(false);
        }
        self.values.insert(k, value);
        self.write().map(|_| true)
    }

    pub fn set_bool(&mut self, scope: Scope, key: &str, v: bool) -> io::Result<bool> {
        self.set(scope, key, Value::Bool(v))
    }

    pub fn set_i64(&mut self, scope: Scope, key: &str, v: i64) -> io::Result<bool> {
        self.set(scope, key, Value::Int(v))
    }

    pub fn set_f64(&mut self, scope: Scope, key: &str, v: f64) -> io::Result<bool> {
        self.set(scope, key, Value::Float(v))
    }

    pub fn set_str(&mut self, scope: Scope, key: &str, v: &str) -> io::Result<bool> {
        self.set(scope, key, Value::Str(v.to_string()))
    }

    pub fn set_list(&mut self, scope: Scope, key: &str, v: &[String]) -> io::Result<bool> {
        self.set(scope, key, Value::List(v.to_vec()))
    }

    /// Forget one value (back to its default).
    pub fn remove(&mut self, scope: Scope, key: &str) -> io::Result<bool> {
        if self.values.remove(&(scope.section(), key.to_string())).is_none() {
            return Ok(false);
        }
        self.write().map(|_| true)
    }

    /// Forget every setting of one page.
    pub fn clear_page(&mut self, page: &str) -> io::Result<bool> {
        let s = Scope::Page(page).section();
        self.retain(|sec| *sec != s)
    }

    /// Settings › "Reset the app's own settings": every app + page setting goes back to its default (keys, glass,
    /// presets, timers …). The PC is not touched and the "how your PC was" records stay.
    pub fn reset_app_settings(&mut self) -> io::Result<bool> {
        let pc = Scope::Pc.section();
        self.retain(|sec| *sec == pc)
    }

    // ---- the glass style (Settings › Glass style)

    pub fn glass(&self) -> GlassStyle {
        self.get_str(Scope::App, GLASS_KEY).and_then(GlassStyle::from_id).unwrap_or_default()
    }

    pub fn set_glass(&mut self, style: GlassStyle) -> io::Result<bool> {
        if style == GlassStyle::default() {
            return self.remove(Scope::App, GLASS_KEY);
        }
        self.set_str(Scope::App, GLASS_KEY, style.id())
    }

    // ---- the theme (Settings › Theme)

    pub fn theme(&self) -> Theme {
        self.get_str(Scope::App, THEME_KEY).and_then(Theme::from_id).unwrap_or_default()
    }

    pub fn set_theme(&mut self, t: Theme) -> io::Result<bool> {
        if t == Theme::default() {
            return self.remove(Scope::App, THEME_KEY);
        }
        self.set_str(Scope::App, THEME_KEY, t.id())
    }

    // ---- internals

    fn retain(&mut self, keep_section: impl Fn(&String) -> bool) -> io::Result<bool> {
        let before = self.values.len();
        self.values.retain(|(sec, _), _| keep_section(sec));
        if self.values.len() == before {
            return Ok(false);
        }
        self.write().map(|_| true)
    }

    /// Write the whole file - on the background writer when there is one (Order 050; a failed write's error comes back
    /// from the next write).
    fn write(&self) -> io::Result<()> {
        let text = format::write(&self.values);
        match &self.writer {
            Some(w) => w.send(text),
            None => write_file(&self.folder, &text),
        }
    }
}

impl Drop for SettingsStore {
    fn drop(&mut self) {
        // the newest file goes to disk before the app ends (the writer's own Drop waits for it and joins)
        self.writer.take();
    }
}

/// `settings.cfg.tmp` (flushed to disk), then renamed over `settings.cfg`.
fn write_file(folder: &Path, text: &str) -> io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(folder)?;
    let path = folder.join(FILE_NAME);
    let tmp = folder.join(format!("{FILE_NAME}.tmp"));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Order 050: the one writer thread of the app's store.
mod writer {
    use std::io;
    use std::path::PathBuf;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    #[derive(Default)]
    struct Slot {
        /// the newest file not written yet
        text: Option<String>,
        /// files handed in / written (all on disk when `done == sent`)
        sent: u64,
        done: u64,
        /// the last write's error, handed back by the next `send`
        err: Option<String>,
        quit: bool,
    }

    pub(super) struct Writer {
        shared: Arc<(Mutex<Slot>, Condvar)>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Writer {
        pub(super) fn start(folder: PathBuf) -> Option<Writer> {
            let shared: Arc<(Mutex<Slot>, Condvar)> = Arc::default();
            let sh = shared.clone();
            let thread = std::thread::Builder::new()
                .name("bu-settings-write".into())
                .spawn(move || {
                    let (m, cv) = &*sh;
                    let Ok(mut g) = m.lock() else { return };
                    loop {
                        if let Some(text) = g.text.take() {
                            let upto = g.sent;
                            drop(g);
                            let r = super::write_file(&folder, &text);
                            let Ok(g2) = m.lock() else { return };
                            g = g2;
                            g.err = r.err().map(|e| e.to_string());
                            g.done = upto;
                            cv.notify_all();
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
            Some(Writer { shared, thread: Some(thread) })
        }

        /// Hand in the newest file (an older one not written yet is dropped). Returns the previous write's error.
        pub(super) fn send(&self, text: String) -> io::Result<()> {
            let (m, cv) = &*self.shared;
            let mut g = m.lock().map_err(|_| io::Error::other("the settings writer stopped"))?;
            g.text = Some(text);
            g.sent += 1;
            cv.notify_all();
            match g.err.take() {
                Some(e) => Err(io::Error::other(e)),
                None => Ok(()),
            }
        }

        /// Wait until every file handed in is written (at most `max`).
        pub(super) fn wait(&self, max: Duration) -> bool {
            let (m, cv) = &*self.shared;
            let end = Instant::now() + max;
            let Ok(mut g) = m.lock() else { return false };
            while g.done < g.sent {
                let left = end.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return false;
                }
                match cv.wait_timeout(g, left) {
                    Ok((g2, _)) => g = g2,
                    Err(_) => return false,
                }
            }
            true
        }
    }

    impl Drop for Writer {
        fn drop(&mut self) {
            let _ = self.wait(Duration::from_secs(5));
            if let Ok(mut g) = self.shared.0.lock() {
                g.quit = true;
            }
            self.shared.1.notify_all();
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }
}

/// Move a broken file to `<name>.broken` (an older one there is replaced); copy it when it can't be moved.
fn keep_aside(path: &Path) -> LoadNote {
    let broken = path.with_file_name(format!("{FILE_NAME}.broken"));
    let _ = std::fs::remove_file(&broken);
    match std::fs::rename(path, &broken).or_else(|_| std::fs::copy(path, &broken).map(|_| ())) {
        Ok(()) => LoadNote::Broken { kept_as: broken },
        Err(e) => LoadNote::BrokenNotKept(e.to_string()),
    }
}
