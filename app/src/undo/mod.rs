//! Undo / reset (Order 014 change 7; the drawing's v21 RESET block, menu-v22.html `resetPop`).
//! - every change the app makes to Windows is recorded with [`record`]: page id, item id, label, OLD and NEW value
//!   ([`Val`] = what the page applies + what the popup shows). The FIRST old value ever recorded per item is "how your PC
//!   was"; it is kept in the settings store's PC part, so it survives restarts and "Reset the app's own settings";
//! - per tab two resets: [`Kind::HowItWas`] ("Back to how your PC was" — from the records) and [`Kind::WindowsDefaults`]
//!   ("Windows defaults" — the page's own list of Windows' factory values, [`Resettable::windows_defaults`]; a page with
//!   none, e.g. Startup, says so with [`Resettable::has_windows_defaults`]; the Controller names it "Steam’s layout"
//!   via [`Resettable::defaults_name`]);
//! - [`Review`] = the review popup's data: title, one line, the ticked lines ("Pointer speed · 10 → 8", all ticked),
//!   untick to keep one, "Reset N"; Settings › Reset uses [`Review::for_all`] (same popup, grouped by tab);
//! - [`Review::apply`] calls each ticked line's page through [`Resettable::apply`] and returns ok / failed (reason) per
//!   line; every ok is recorded as a change, so the page's "how your PC was" list shrinks by it;
//! - Settings › "Reset the app's own settings" = `SettingsStore::reset_app_settings` (the PC is not touched).
//!   Pages are made when opened: the Settings-wide reset needs every page's [`Resettable`] — the app makes them for it.
//! - Order 036: every page that changes Windows records through [`note`] (any thread: a page's worker, a key handler, a
//!   background part) or `Cx::record`; the main loop writes queued notes with [`flush`]. A page's `Resettable` methods
//!   may run while the frame holds the services: they never call `services::with` (use `try_with`).
//! - the uninstaller's undo with no window = [`undo_everything`] (main.rs `--undo-windows`; `--undo-windows-count` only
//!   counts).

#[cfg(test)]
mod tests;

use std::io;

use crate::settings::{Scope, SettingsStore};

/// One value of one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Val {
    /// What the page puts back (its own text form: "8", "{guid}", "on").
    pub raw: String,
    /// What the popup shows ("8", "Speakers (Realtek)", "On").
    pub text: String,
}

impl Val {
    pub fn new(raw: &str, text: &str) -> Self {
        Val { raw: raw.into(), text: text.into() }
    }
    /// Raw and shown text are the same.
    pub fn plain(s: &str) -> Self {
        Val::new(s, s)
    }
}

/// Which reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// "Back to how your PC was".
    HowItWas,
    /// "Windows defaults" (or the page's own name for it).
    WindowsDefaults,
}

/// One item's record: how it was before the app first changed it, and what the app last set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub page: String,
    pub item: String,
    pub label: String,
    /// How the PC was (the first old value ever recorded).
    pub was: Val,
    /// What the app set last.
    pub now: Val,
    /// When first / last changed (seconds since 1970).
    pub first: u64,
    pub last: u64,
}

/// One Windows default the page knows: its item, label, value now and Windows' own value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultItem {
    pub item: String,
    pub label: String,
    pub now: Val,
    pub default: Val,
}

/// A page (or its crate's service) that can put items back.
pub trait Resettable {
    /// The page id ("mouse").
    fn page_id(&self) -> &str;
    /// The page's title ("Mouse"), for the popup's title and Settings' groups.
    fn page_title(&self) -> &str;
    /// The item's value right now; None = use the last recorded value.
    fn current(&self, _item: &str) -> Option<Val> {
        None
    }
    /// Does the page still have this item? false = an old change-log entry of something the app no longer has (Order 040:
    /// Hibernate) - it gets no line and is never applied.
    fn has_item(&self, _item: &str) -> bool {
        true
    }
    /// Does "Windows defaults" exist for this page? (Startup: no — Windows has no default startup list.)
    fn has_windows_defaults(&self) -> bool {
        true
    }
    /// The link's name when it isn't "Windows defaults" (Controller: "Steam’s layout").
    fn defaults_name(&self) -> Option<&str> {
        None
    }
    /// Windows' own factory values for this page's items (only where Windows has one).
    fn windows_defaults(&self) -> Vec<DefaultItem> {
        Vec::new()
    }
    /// Put one item to this value. Err = the reason, shown to the user.
    fn apply(&mut self, item: &str, to: &Val) -> Result<(), String>;
}

const SEP: char = '\u{1f}';

/// The uninstaller's undo with no window runs (main.rs `undo_windows`): a page's closed-page service starts nothing that
/// would act on its own (Display's per-game watcher).
static HEADLESS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_headless() {
    HEADLESS.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn headless() -> bool {
    HEADLESS.load(std::sync::atomic::Ordering::Relaxed)
}

/// One change noted from any thread, waiting for [`flush`].
type Note = (String, String, String, Val, Val);
#[cfg(not(test))]
static PENDING: std::sync::Mutex<Vec<Note>> = std::sync::Mutex::new(Vec::new());
#[cfg(not(test))]
static HAS_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// Unit tests run in parallel, each with its own services on its own thread: one queue per thread there, so a test never
// flushes another test's notes into its own store (the app has ONE queue: notes from every thread, one UI thread).
#[cfg(test)]
thread_local! {
    static PENDING_T: std::cell::RefCell<Vec<Note>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(not(test))]
fn push(n: Note) {
    if let Ok(mut q) = PENDING.lock() {
        q.push(n);
        HAS_PENDING.store(true, std::sync::atomic::Ordering::Release);
    }
}
#[cfg(test)]
fn push(n: Note) {
    PENDING_T.with(|q| q.borrow_mut().push(n));
}

#[cfg(not(test))]
fn take() -> Vec<Note> {
    match PENDING.lock() {
        Ok(mut q) => {
            HAS_PENDING.store(false, std::sync::atomic::Ordering::Release);
            std::mem::take(&mut *q)
        }
        Err(_) => Vec::new(),
    }
}
#[cfg(test)]
fn take() -> Vec<Note> {
    PENDING_T.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

/// Record a change from ANY thread (a page's worker, a key handler, a background part; the settings store lives on the
/// UI thread): written at once when the services are free on this thread, else queued and written at the main loop's
/// next wake-up ([`flush`], woken here). Order kept: a queued note is always written before a later one.
pub fn note(page: &str, item: &str, label: &str, old: &Val, new: &Val) {
    push((page.into(), item.into(), label.into(), old.clone(), new.clone()));
    if crate::services::try_with(|s| flush(&mut s.store)).is_none() {
        crate::services::Waker.wake();
    }
}

/// Are notes waiting? (cheap: the main loop asks after every wake-up)
pub fn pending() -> bool {
    #[cfg(not(test))]
    return HAS_PENDING.load(std::sync::atomic::Ordering::Acquire);
    #[cfg(test)]
    return PENDING_T.with(|q| !q.borrow().is_empty());
}

/// Write every queued [`note`] into the store, oldest first.
pub fn flush(store: &mut SettingsStore) {
    for (p, i, l, o, n) in take() {
        let _ = record(store, &p, &i, &l, &o, &n);
    }
}

/// Forget an item's record: the page's change was taken back on its own (e.g. Display's 10 s keep / revert ran out),
/// nothing of the app is left on the PC for it.
pub fn forget(store: &mut SettingsStore, page: &str, item: &str) -> io::Result<()> {
    store.remove(Scope::Pc, &rec_key(page, item)).map(|_| ())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn rec_key(page: &str, item: &str) -> String {
    format!("{page}{SEP}{item}")
}

/// Record one change the app made: `old` → `new`. The first `old` ever recorded for the item stays as "how your PC was".
pub fn record(store: &mut SettingsStore, page: &str, item: &str, label: &str, old: &Val, new: &Val) -> io::Result<()> {
    let t = now_secs();
    let (was, first) = match read_record(store, page, item) {
        Some(r) => (r.was, r.first),
        None => (old.clone(), t),
    };
    let r = Record { page: page.into(), item: item.into(), label: label.into(), was, now: new.clone(), first, last: t };
    write_record(store, &r)
}

/// Every record (of one page, or all), sorted by page then item.
pub fn records(store: &SettingsStore, page: Option<&str>) -> Vec<Record> {
    store
        .keys(Scope::Pc)
        .into_iter()
        .filter_map(|k| k.split_once(SEP))
        .filter(|(p, _)| page.is_none_or(|want| *p == want))
        .filter_map(|(p, i)| read_record(store, p, i))
        .collect()
}

/// One item's record.
pub fn read_record(store: &SettingsStore, page: &str, item: &str) -> Option<Record> {
    let l = store.get_list(Scope::Pc, &rec_key(page, item))?;
    let [label, was_raw, was_text, now_raw, now_text, first, last] = l else { return None };
    Some(Record {
        page: page.into(),
        item: item.into(),
        label: label.clone(),
        was: Val::new(was_raw, was_text),
        now: Val::new(now_raw, now_text),
        first: first.parse().ok()?,
        last: last.parse().ok()?,
    })
}

fn write_record(store: &mut SettingsStore, r: &Record) -> io::Result<()> {
    let list = [
        r.label.clone(),
        r.was.raw.clone(),
        r.was.text.clone(),
        r.now.raw.clone(),
        r.now.text.clone(),
        r.first.to_string(),
        r.last.to_string(),
    ];
    store.set_list(Scope::Pc, &rec_key(&r.page, &r.item), &list).map(|_| ())
}

/// One line of the review popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub page: String,
    pub page_title: String,
    pub item: String,
    pub label: String,
    pub from: Val,
    pub to: Val,
    pub ticked: bool,
}

impl Line {
    /// The line's small text: "10  →  8" (the drawing: now, two spaces, arrow, two spaces, back).
    pub fn change_text(&self) -> String {
        format!("{}  →  {}", self.from.text, self.to.text)
    }
}

/// What happened to one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineResult {
    pub page: String,
    pub item: String,
    pub label: String,
    pub outcome: Outcome,
}

/// The review popup: title, one line, the ticked lines.
#[derive(Debug, Clone)]
pub struct Review {
    pub kind: Kind,
    /// Settings › Reset (every page, grouped) rather than one tab.
    pub all: bool,
    /// For one tab: its title and its name for "Windows defaults" (None = that).
    page_title: String,
    defaults_name: Option<String>,
    pub lines: Vec<Line>,
}

impl Review {
    /// One tab's reset line.
    pub fn for_page(kind: Kind, page: &dyn Resettable, store: &SettingsStore) -> Review {
        Review {
            kind,
            all: false,
            page_title: page.page_title().into(),
            defaults_name: page.defaults_name().map(String::from),
            lines: lines_of(kind, page, store),
        }
    }

    /// Settings › Reset: every page, in the given order, grouped by tab.
    pub fn for_all(kind: Kind, pages: &[&dyn Resettable], store: &SettingsStore) -> Review {
        Review {
            kind,
            all: true,
            page_title: String::new(),
            defaults_name: None,
            lines: pages.iter().flat_map(|p| lines_of(kind, *p, store)).collect(),
        }
    }

    /// The popup's bold title (the drawing's words).
    pub fn title(&self) -> String {
        match (self.kind, self.all, &self.defaults_name) {
            (Kind::HowItWas, true, _) => "Back to how your PC was?".into(),
            (Kind::HowItWas, false, _) => format!("{} · back to how it was?", self.page_title),
            (Kind::WindowsDefaults, true, _) => "Windows defaults for everything?".into(),
            (Kind::WindowsDefaults, false, Some(name)) => format!("{} · back to {name}?", self.page_title),
            (Kind::WindowsDefaults, false, None) => format!("{} · Windows defaults?", self.page_title),
        }
    }

    /// The popup's one line under the title (the drawing's words; the Controller's is its own).
    pub fn subline(&self) -> &'static str {
        match (self.kind, self.defaults_name.is_some()) {
            (Kind::HowItWas, _) => "Each one goes back to the value it had before this app changed it.",
            (Kind::WindowsDefaults, true) => "The layout goes back to the one Steam made for this game.",
            (Kind::WindowsDefaults, false) => "Each one goes to Windows’ own value.",
        }
    }

    /// The red button: "Reset N" (or "Reset", disabled, with nothing ticked).
    pub fn button_text(&self) -> String {
        match self.ticked() {
            0 => "Reset".into(),
            n => format!("Reset {n}"),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn ticked(&self) -> usize {
        self.lines.iter().filter(|l| l.ticked).count()
    }

    /// Click a line: untick to keep it / tick it again.
    pub fn toggle(&mut self, i: usize) {
        if let Some(l) = self.lines.get_mut(i) {
            l.ticked = !l.ticked;
        }
    }

    /// The groups for Settings (tab title + its line indexes), in order.
    pub fn groups(&self) -> Vec<(String, Vec<usize>)> {
        let mut out: Vec<(String, Vec<usize>)> = Vec::new();
        for (i, l) in self.lines.iter().enumerate() {
            match out.last_mut() {
                Some((t, v)) if *t == l.page_title => v.push(i),
                _ => out.push((l.page_title.clone(), vec![i])),
            }
        }
        out
    }

    /// Reset the ticked lines through their pages; each ok is recorded as a change. A line whose page isn't given fails.
    pub fn apply(&self, store: &mut SettingsStore, pages: &mut [&mut dyn Resettable]) -> Vec<LineResult> {
        self.apply_each(pages, &mut |l| record(store, &l.page, &l.item, &l.label, &l.from, &l.to))
    }

    /// [`Review::apply`] with the recording handed in: the frame runs the pages' `apply` while it does NOT hold the
    /// services (a page may save its own settings meanwhile) and records each ok line through `rec`.
    pub fn apply_each(&self, pages: &mut [&mut dyn Resettable], rec: &mut dyn FnMut(&Line) -> io::Result<()>) -> Vec<LineResult> {
        // Order 039: every admin line of this reset (any page) goes to ONE elevated copy - one admin prompt for the whole
        // batch, started at the first admin line; a "No" leaves every admin line "Needs admin - not changed"
        let _admin = crate::admin::client::admin().scope(crate::admin::Purpose::Reset);
        let mut out = Vec::new();
        for l in self.lines.iter().filter(|l| l.ticked) {
            let outcome = match pages.iter_mut().find(|p| p.page_id() == l.page) {
                None => Outcome::Failed("The page isn't loaded".into()),
                Some(p) => match p.apply(&l.item, &l.to) {
                    Ok(()) => match rec(l) {
                        Ok(()) => Outcome::Ok,
                        Err(e) => Outcome::Failed(format!("Reset, but not saved: {e}")),
                    },
                    Err(why) => Outcome::Failed(why),
                },
            };
            out.push(LineResult { page: l.page.clone(), item: l.item.clone(), label: l.label.clone(), outcome });
        }
        out
    }
}

fn lines_of(kind: Kind, page: &dyn Resettable, store: &SettingsStore) -> Vec<Line> {
    let mk = |item: &str, label: &str, from: Val, to: Val| Line {
        page: page.page_id().into(),
        page_title: page.page_title().into(),
        item: item.into(),
        label: label.into(),
        from,
        to,
        ticked: true,
    };
    match kind {
        Kind::HowItWas => records(store, Some(page.page_id()))
            .into_iter()
            .filter(|r| page.has_item(&r.item))
            .map(|r| {
                let from = page.current(&r.item).unwrap_or(r.now);
                (r.item, r.label, from, r.was)
            })
            .filter(|(_, _, from, to)| from.raw != to.raw)
            .map(|(i, l, f, t)| mk(&i, &l, f, t))
            .collect(),
        Kind::WindowsDefaults if !page.has_windows_defaults() => Vec::new(),
        Kind::WindowsDefaults => page
            .windows_defaults()
            .into_iter()
            .filter(|d| d.now.raw != d.default.raw)
            .map(|d| mk(&d.item, &d.label, d.now, d.default))
            .collect(),
    }
}

/// The uninstaller's "Undo my Windows changes too?" with no window (main.rs `--undo-windows`): every page's "Back to how
/// your PC was" lines, all ticked, applied. `apply` false = only count them (`--undo-windows-count`). Returns (lines,
/// failed). Runs on the UI thread with the services started; the pages are fresh (closed) ones, so each page's
/// `resettable()` builds what it needs on demand.
pub fn undo_everything(pages: &mut [Box<dyn crate::pages::Page>], apply: bool) -> (usize, usize) {
    crate::services::with(|s| flush(&mut s.store));
    let review = {
        let rs: Vec<&mut dyn Resettable> = pages.iter_mut().filter_map(|p| p.resettable()).collect();
        let refs: Vec<&dyn Resettable> = rs.iter().map(|r| &**r).collect();
        crate::services::with(|s| Review::for_all(Kind::HowItWas, &refs, &s.store))
    };
    let Some(review) = review else { return (0, 0) };
    let n = review.ticked();
    if !apply || n == 0 {
        return (n, 0);
    }
    let mut rp: Vec<&mut dyn Resettable> = pages.iter_mut().filter_map(|p| p.resettable()).collect();
    let results = review.apply_each(&mut rp, &mut |l| {
        note(&l.page, &l.item, &l.label, &l.from, &l.to);
        Ok(())
    });
    crate::services::with(|s| flush(&mut s.store));
    (n, results.iter().filter(|r| matches!(r.outcome, Outcome::Failed(_))).count())
}
