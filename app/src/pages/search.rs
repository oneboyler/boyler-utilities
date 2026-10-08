//! The Search tab (menu-v22 page `srch`, Order 023 / 030): "like the old Windows search" - one big field (focused when the
//! tab opens), small filter chips + a file-type picker (the owner, test build 1: "shouldn't it let you pick what type of
//! something ur looking for?"), results as you type grouped Apps · Folders · Files (tile, name with the typed words marked,
//! path, size / date), ↑ ↓ + Enter, the right-click menu (Open file location / Copy path / Open with…). Only what is on
//! this PC. No search-key setting here (it lives in Settings › All shortcuts).
//!
//! Wired to `bu-search`. Files and folders come only from Everything (the owner, Oct 8): the tab starts it hidden when it opens
//! (the user's own copy is used when it runs) and leaving the tab / closing the menu quits ours - nothing runs while the tab
//! is not used. While its index is not ready the page says what it does (starting / building / loading its file list) and
//! for how long (the owner, test build 2: "it can't just say wait"); not installed = one line + "Install Everything" (the official installer,
//! checked, one admin prompt). Apps are found either way.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use bu_search as srch;
use taffy::style::{AlignItems, JustifyContent};

use crate::anim::EASE;
use crate::gfx::{sh, Align, Font, Rgba};
use crate::pages::srch_data::SDATA;
use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, lh, sub, Cursor, El, Key};
use crate::ui::pieces::button::{cbtn, Kind};
use crate::ui::pieces::listrow::{tile, Tile};
use crate::ui::pieces::mitems::{self, It, Place, Right, Row};
use crate::ui::pieces::{self, bits, group, link, toast};
use crate::ui::{cmix, ACC, ACC_S, CTL, CTL_H, FG, FG2, FG3, GRP, HAIR, HOV, KEY, SEL};

const K_FIELD: Key = key("srch.field");
const K_CHIP: Key = key("srch.chip");
const K_TYPE: Key = key("srch.type");
const K_TYPEM: Key = key("srch.typemenu");
const K_ROW: Key = key("srch.row");
const K_MORE: Key = key("srch.more");
const K_MENU: Key = key("srch.menu");
const K_INSTALL: Key = key("srch.install");
const K_TOAST: Key = key("srch.toast");

const VK_BACK: u16 = 0x08;
const VK_ENTER: u16 = 0x0D;
const VK_ESC: u16 = 0x1B;
const VK_UP: u16 = 0x26;
const VK_DOWN: u16 = 0x28;

/// The type picker's list: the extension + what it is (one extension at a time; "Any type" first).
const TYPES: [(&str, &str); 9] = [
    ("txt", "Text"),
    ("pdf", "PDF"),
    ("docx", "Word"),
    ("xlsx", "Excel"),
    ("png", "Picture"),
    ("jpg", "Picture"),
    ("mp4", "Video"),
    ("zip", "Archive"),
    ("exe", "Program"),
];

/// The tab's service, kept for the app's life (+ a test copy's fake behind it).
#[derive(Clone)]
struct Held {
    svc: Arc<srch::SearchService>,
    fake: Option<Arc<srch::FakeOs>>,
}

const KEEP_SVC: &str = "srch.svc";

/// How long the page waits for a started Everything to answer at all. An index that is loading is followed as long as the
/// page is open (a first build can take minutes; the waiter gave up after 90 s and left "catching up" up for good).
const ENGINE_WAIT: Duration = Duration::from_secs(90);

/// "0:07", "12:34", "1:02:03": how long the page has waited for Everything (the page's own clock, ms).
fn waited_text(ms: f64) -> String {
    let s = (ms.max(0.0) / 1000.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// The drawing's app tiles (menu-v22 `TAPP`): glyph + 135° gradient.
const TAPP: &[(&str, &str, u32, u32)] = &[
    ("discord", "chat", 0x8f95ff, 0x5a5fe0),
    ("steam", "pad", 0x6f8fb8, 0x2b3f5c),
    ("val", "aim", 0xff7a76, 0xd83f4c),
    ("riot", "aim", 0xff8a6b, 0xc43a2a),
    ("obs", "rec", 0x7a808c, 0x3a3e47),
    ("spot", "note", 0x46d989, 0x1c9a5a),
    ("chrome", "globe", 0xffd35a, 0xe6493b),
    ("note", "mtxt", 0x7fb2ff, 0x3b6fd6),
    ("paint", "dpen", 0xffc56b, 0xe0861c),
    ("woot", "kb16", 0xffb86b, 0xe0661c),
    ("nv", "chip", 0x9be15d, 0x4a9a1c),
    ("rl", "pad", 0x5ab4ff, 0x2a6ee6),
    ("epic", "pad", 0x8a8f99, 0x3d4149),
    ("code", "doc", 0x5ab4ff, 0x1f6fd0),
    ("exp", "fold", 0xffd35a, 0xe8a33a),
    ("sys", "cog16", 0xa2abbd, 0x6c7487),
    ("win", "win16", 0x7fb2ff, 0x3b6fd6),
    ("vlc", "mplay", 0xffb04a, 0xe8701c),
    ("zip", "zip", 0xa2abbd, 0x6c7487),
    ("calc", "appw", 0xa2abbd, 0x6c7487),
];
/// `TFILE`: pic, vid, doc, txt, zip, exe, cfg.
const TFILE: &[(&str, &str, u32, u32)] = &[
    ("pic", "img", 0x5ab4ff, 0x2a74e6),
    ("vid", "mplay", 0xff8fb6, 0xd9467e),
    ("doc", "doc", 0xff7a76, 0xc4313f),
    ("txt", "mtxt", 0xa2abbd, 0x6c7487),
    ("zip", "zip", 0xb58cff, 0x6f4ae0),
    ("exe", "appw", 0x7fb2ff, 0x3b6fd6),
    ("cfg", "cog16", 0xa2abbd, 0x6c7487),
];

fn glyph(t: &(&str, &'static str, u32, u32)) -> Tile {
    Tile::Glyph { glyph: t.1, a: Rgba::hex(t.2), b: Rgba::hex(t.3) }
}

/// The row's tile: the drawing's for a sample item; else by kind / file type (a folder = the yellow folder tile).
fn tile_of(it: &srch::Item, sample: Option<&str>) -> Tile {
    if it.kind == srch::ItemKind::Folder {
        return Tile::Glyph { glyph: "fold", a: Rgba::hex(0xffd35a), b: Rgba::hex(0xe8a33a) };
    }
    if let Some(s) = sample {
        let table = if it.kind == srch::ItemKind::App { TAPP } else { TFILE };
        if let Some(t) = table.iter().find(|t| t.0 == s) {
            return glyph(t);
        }
    }
    let k = match it.file_type {
        Some(srch::FileType::Picture) => "pic",
        Some(srch::FileType::Video) => "vid",
        Some(srch::FileType::Document) => "doc",
        Some(srch::FileType::Text) => "txt",
        Some(srch::FileType::Archive) => "zip",
        Some(srch::FileType::Program) => "exe",
        Some(srch::FileType::Config) => "cfg",
        _ => "",
    };
    match TFILE.iter().find(|t| t.0 == k) {
        Some(t) if it.kind == srch::ItemKind::File => glyph(t),
        _ => Tile::Glyph { glyph: "appw", a: Rgba::hex(0xa2abbd), b: Rgba::hex(0x6c7487) },
    }
}

/// The drawing's tile for a sample item (a test copy's fake: menu-v22 `SDATA`).
fn sample_tile(it: &srch::Item) -> Option<&'static str> {
    SDATA
        .iter()
        .find(|d| d.1 == it.name && (d.2 == it.path || it.path.rsplit_once('\\').map(|(dir, _)| dir == d.2).unwrap_or(false)))
        .map(|d| d.5)
        .filter(|t| !t.is_empty())
}

/// "84.2 MB" / "2 KB" / "1.8 GB" -> bytes (the drawing's sample sizes).
fn bytes_of(s: &str) -> Option<u64> {
    let (n, u) = s.split_once(' ')?;
    let n: f64 = n.parse().ok()?;
    let k = match u {
        "KB" => 1024.0,
        "MB" => 1024.0 * 1024.0,
        "GB" => 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    Some((n * k).round() as u64)
}

/// "6 Oct 2026" -> a stamp.
fn stamp_of(s: &str) -> Option<srch::Stamp> {
    const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mut p = s.split(' ');
    let d: u8 = p.next()?.parse().ok()?;
    let mon = p.next()?;
    let m = M.iter().position(|x| *x == mon)? as u8 + 1;
    let y: u16 = p.next()?.parse().ok()?;
    Some(srch::Stamp::new(y, m, d, 12, 0))
}

/// The FAKE Windows of a test copy: the drawing's sample apps, folders and files, Everything running (`engine`) or not
/// installed.
pub fn sample_os(engine: bool) -> Arc<srch::FakeOs> {
    let os = Arc::new(srch::FakeOs::new());
    {
        let mut st = os.state();
        st.everything = if engine { srch::EverythingStatus::Running { version: 1 } } else { srch::EverythingStatus::NotInstalled };
        for (kind, name, path, date, size, _) in SDATA {
            match *kind {
                "app" => st.apps.push(srch::AppEntry { name: name.to_string(), parsing_name: path.to_string(), program_path: (!path.starts_with("ms-settings")).then(|| path.to_string()) }),
                "dir" => {
                    st.everything_files.push(srch::Hit { name: name.to_string(), path: path.to_string(), is_folder: true, size: None, modified: stamp_of(date) });
                    if let Some(n) = size.strip_suffix(" items").and_then(|n| n.parse().ok()) {
                        st.counts.push((path.to_string(), n));
                    }
                }
                _ => st.everything_files.push(srch::Hit { name: name.to_string(), path: format!("{path}\\{name}"), is_folder: false, size: bytes_of(size), modified: stamp_of(date) }),
            }
        }
    }
    os
}

enum Msg {
    /// a query's answer + the "<n> items" of its folder rows (read with the results, never per frame)
    Results(u64, srch::Result<srch::SearchResults>, std::collections::HashMap<String, u64>),
    /// Everything right now (sent when it changes while the page waits for it)
    Engine(srch::EverythingStatus),
    /// the install ended
    Installed(srch::Result<()>),
    /// a started Everything did not answer within `ENGINE_WAIT` (the waiter stopped)
    NoAnswer,
}

#[derive(Default)]
pub struct Search {
    env: Env,
    svc: Option<Arc<srch::SearchService>>,
    /// a test copy's fake Windows (tests look into it; the drawing's tiles come from it)
    fake: Option<Arc<srch::FakeOs>>,
    text: String,
    filter: Option<srch::Filter>,
    /// the type picker: only files of this extension (None = any)
    ext: Option<&'static str>,
    /// the type picker's list is open (under its chip's box)
    type_menu: Option<(f32, f32, f32, f32)>,
    results: Option<srch::SearchResults>,
    /// the selected row in the flat list (↑ ↓, the first one after typing)
    sel: usize,
    /// a right-click menu: (flat row index, x, y)
    menu: Option<(usize, f32, f32)>,
    toast: Option<(String, f64)>,
    focus_once: bool,
    /// Everything right now; None until the first answer
    engine: Option<srch::EverythingStatus>,
    installing: bool,
    /// when the page started waiting for Everything (its clock, ms): the "<m:ss> so far" while it is not ready
    wait_since: f64,
    /// the started Everything never answered (`Msg::NoAnswer`)
    no_answer: bool,
    install_err: Option<String>,
    /// the engine waiter stops (the page closed)
    stop: Arc<AtomicBool>,
    folder_counts: std::collections::HashMap<String, u64>,
    press_box: std::collections::HashMap<Key, (f32, f32, f32, f32)>,
    seq: u64,
    cancel: Option<srch::Cancel>,
    tx: Option<mpsc::Sender<Msg>>,
    rx: Option<mpsc::Receiver<Msg>>,
    now: f64,
    /// Order 047: what an open / a menu entry said back - the shell's call runs on its own thread (`crate::offui`, it
    /// woke the menu); `tick` turns it into the toast
    shell: Arc<std::sync::Mutex<Vec<String>>>,
}

impl Search {
    fn filter(&self) -> srch::Filter {
        self.filter.unwrap_or(srch::Filter::All)
    }
    fn query(&self) -> srch::Query {
        srch::Query::new(&self.text, self.filter()).with_ext(self.ext)
    }
    fn flat(&self) -> Vec<srch::Item> {
        self.results.as_ref().map(|r| r.flat().into_iter().cloned().collect()).unwrap_or_default()
    }
    fn ready(&self) -> bool {
        matches!(self.engine, Some(srch::EverythingStatus::Running { .. }))
    }

    /// Run blocking crate work: inline on the fake, else on a worker thread whose answer repaints the menu.
    fn spawn(&self, job: impl FnOnce() + Send + 'static) {
        if self.env.fake() {
            job();
        } else {
            let wake = self.env.waker();
            std::thread::spawn(move || {
                job();
                wake.wake();
            });
        }
    }

    /// The tab opened (or the install ended): make Everything answer - the user's own copy, or ours started hidden - and
    /// follow it until its index is loaded (each change is sent; the waiter ends when ready, when the page closes, or when
    /// it has not answered for `ENGINE_WAIT` - a loading index is followed to its end). Off the UI thread on a real PC.
    fn prepare(&mut self) {
        let Some(svc) = self.svc.clone() else { return };
        let tx = self.tx.clone().expect("open");
        let stop = self.stop.clone();
        let wake = self.env.waker();
        let fake = self.env.fake();
        self.wait_since = self.now;
        self.no_answer = false;
        let job = move || {
            // NotInstalled comes back as the status below; other start errors leave it NotRunning
            let _ = svc.start_engine();
            let t0 = std::time::Instant::now();
            // since when it has not answered at all
            let mut silent = std::time::Instant::now();
            let mut last = None;
            loop {
                let st = svc.engine();
                if st != srch::EverythingStatus::NotRunning {
                    silent = std::time::Instant::now();
                }
                if last != Some(st) {
                    last = Some(st);
                    if tx.send(Msg::Engine(st)).is_err() {
                        return;
                    }
                    wake.wake();
                }
                let done = matches!(st, srch::EverythingStatus::Running { .. } | srch::EverythingStatus::NotInstalled);
                if done || fake || stop.load(Ordering::SeqCst) {
                    return;
                }
                if silent.elapsed() > ENGINE_WAIT {
                    let _ = tx.send(Msg::NoAnswer);
                    wake.wake();
                    return;
                }
                // not answering (e.g. ours quit by an earlier close just now): start it again, every 2 s at most
                if st == srch::EverythingStatus::NotRunning && t0.elapsed().as_millis() % 2000 < 250 {
                    let _ = svc.start_engine();
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        };
        if fake {
            job();
        } else {
            std::thread::spawn(job);
        }
        self.pump();
    }

    /// "Install Everything": the official installer, checked, run silently (Windows asks for admin once).
    fn install(&mut self) {
        if self.installing {
            return;
        }
        let Some(svc) = self.svc.clone() else { return };
        self.installing = true;
        self.install_err = None;
        let tx = self.tx.clone().expect("open");
        self.spawn(move || {
            let _ = tx.send(Msg::Installed(svc.install_engine()));
        });
        self.pump();
    }

    /// Search as you type: the fake answers at once; the real one on a worker thread (the previous query is cancelled).
    fn run(&mut self) {
        self.sel = 0;
        self.menu = None;
        if let Some(c) = self.cancel.take() {
            c.cancel();
        }
        let Some(svc) = self.svc.clone() else { return };
        if self.text.trim().is_empty() {
            self.results = None;
            return;
        }
        self.seq += 1;
        let seq = self.seq;
        let q = self.query();
        let c = srch::Cancel::new();
        self.cancel = Some(c.clone());
        let tx = self.tx.clone().expect("open");
        self.spawn(move || {
            let r = svc.search(&q, &c);
            let mut counts = std::collections::HashMap::new();
            if let Ok(res) = &r {
                for it in res.flat() {
                    if it.kind == srch::ItemKind::Folder {
                        if let Some(n) = svc.folder_item_count(it) {
                            counts.insert(it.path.clone(), n);
                        }
                    }
                }
            }
            let _ = tx.send(Msg::Results(seq, r, counts));
        });
        self.pump();
    }

    fn pump(&mut self) -> bool {
        let mut msgs = Vec::new();
        if let Some(rx) = &self.rx {
            while let Ok(m) = rx.try_recv() {
                msgs.push(m);
            }
        }
        let any = !msgs.is_empty();
        for m in msgs {
            match m {
                Msg::NoAnswer => self.no_answer = true,
                Msg::Engine(st) => {
                    let was = self.ready();
                    self.engine = Some(st);
                    if st != srch::EverythingStatus::NotRunning {
                        self.no_answer = false;
                    }
                    // it caught up while words were typed: ask again (files and folders now)
                    if !was && self.ready() && !self.text.trim().is_empty() {
                        self.run();
                    }
                }
                Msg::Installed(r) => {
                    self.installing = false;
                    match r {
                        Ok(()) => self.prepare(),
                        Err(srch::SearchError::InstallCancelled) => self.install_err = Some("The install was cancelled".into()),
                        Err(e) => self.install_err = Some(format!("The install failed \u{00b7} {e}")),
                    }
                }
                Msg::Results(seq, r, counts) => {
                    if seq != self.seq {
                        continue; // an older query's answer
                    }
                    match r {
                        Ok(r) => {
                            self.results = Some(r);
                            self.folder_counts = counts;
                            // the rows changed: a menu opened meanwhile would point at another row
                            self.menu = None;
                            self.sel = self.sel.min(self.flat().len().saturating_sub(1));
                        }
                        Err(srch::SearchError::Cancelled) => {}
                        Err(_) => self.results = None,
                    }
                }
            }
        }
        any
    }

    /// Order 047: Enter / a click on a result opens it through the shell (ShellExecute: 0.1-3 s for a cold app or a network
    /// path) on its own thread - the menu keeps painting; a failure comes back as the same toast (`take_shell`).
    fn open_item(&mut self, i: usize) {
        let flat = self.flat();
        let (Some(it), Some(svc)) = (flat.get(i).cloned(), self.svc.clone()) else { return };
        let out = self.shell.clone();
        crate::offui::spawn("srch-open", move || {
            if svc.open(&it).is_err() {
                if let Ok(mut v) = out.lock() {
                    v.push(format!("{} could not be opened", it.name));
                }
            }
        });
    }

    /// Order 047: the toast a shell call on its own thread asked for (the last one wins). True = one came in.
    fn take_shell(&mut self) -> bool {
        let t = self.shell.lock().ok().and_then(|mut v| {
            let last = v.pop();
            v.clear();
            last
        });
        match t {
            Some(t) => {
                self.toast = Some((t, self.now));
                true
            }
            None => false,
        }
    }

    /// The right-click menu of a result at the pointer.
    pub fn context_menu(&mut self, i: usize, x: f32, y: f32) {
        if i < self.flat().len() {
            self.sel = i;
            self.menu = Some((i, x, y));
        }
    }

    // ------------------------------------------------------------------------------------------------- building

    /// `.sbig`: the big field. the owner (test build 1): the "Search" placeholder kept popping in and out of its box - the
    /// blinking caret was a CHILD of the text element, so every 530 ms the text's own layout changed with it. Now the text
    /// and the caret sit side by side in one box (the caret absolutely placed in it): the blink moves nothing.
    fn field(&self, cx: &mut Cx) -> El {
        let focused = cx.focused(K_FIELD);
        let ring = cx.tr(K_FIELD, 3, if focused { 1.0 } else { 0.0 }, 150.0, EASE);
        // `.sbq{flex:1;min-width:0;height:42px;padding:0 6px 0 0;font:15px/42px var(--font)}` `::placeholder{color:var(--fg3)}`
        let font = Font::new(15.0, 400).ls(0);
        let empty = self.text.is_empty();
        let mut q = El::block()
            .flex1()
            .min_w(0.0)
            .h(42.0)
            .pad(0.0, 6.0, 0.0, 0.0)
            .no_hit()
            // no_hit: a click on the words must reach the FIELD (the owner, test build 2: "it doesn't let me click and type" - a
            // click on the placeholder / typed words gave the focus to this keyed text, and every key typed went to it)
            .child(El::text(if empty { "Search this PC" } else { &self.text }, font, if empty { FG3() } else { FG() }, 42.0).ellipsis().key(sub(K_FIELD, "q")).no_hit());
        // the caret blinks (530 ms); frozen test pictures leave it out (the drawing's capture has it off)
        if focused && !self.env.frozen {
            let on = ((cx.now / 530.0) as i64) % 2 == 0;
            cx.wake_every(530.0, 0.0);
            let tw = if empty { 0.0 } else { cx.g.text_width(&self.text, font) };
            if on {
                q = q.child(El::block().abs(tw.round(), 12.0, f32::NAN, f32::NAN).size(1.0, 18.0).bg(FG()).no_hit());
            }
        }
        // `.sbig{display:flex;align-items:center;height:42px;margin-top:2px;border-radius:11px;background:var(--grp);
        //   box-shadow:inset 0 0 0 .5px var(--hair)}` `.sbig>i{width:42px;height:42px;color:var(--fg2)}` `svg{16px;stroke-width:1.5}`
        let mut f = El::row()
            .center()
            .h(42.0)
            .margin(2.0, 0.0, 0.0, 0.0)
            .radius(11.0)
            .bg(GRP())
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
            .key(K_FIELD)
            .cursor(Cursor::Text)
            .child(El::block().size(42.0, 42.0).none().place_center().no_hit().child(El::icon("search", 16.0, 1.5, FG2())))
            .child(q);
        if !empty {
            // `.tsx{width:20px;height:20px;border-radius:50%;color:var(--fg3)}` `:hover{background:var(--ctl-h);color:var(--fg)}`
            // `#sw .sbig .tsx{margin-right:10px}`
            let xk = sub(K_FIELD, "x");
            let xh = cx.hover_t(xk, 150.0, EASE);
            f = f.child(
                El::block()
                    .size(20.0, 20.0)
                    .none()
                    .margin(0.0, 10.0, 0.0, 0.0)
                    .radius(10.0)
                    .bg(CTL_H().mul_a(xh))
                    .place_center()
                    .on_click(xk)
                    .cursor(Cursor::Hand)
                    // `title:'Clear'`
                    .title("Clear")
                    .child(El::icon("x", 8.0, 1.5, cmix(FG3(), FG(), xh)).no_hit()),
            );
        }
        if ring > 0.001 {
            // `.sbig::after{box-shadow:0 0 0 3px var(--acc-s),inset 0 0 0 1px var(--acc)}` `:focus-within::after{opacity:1}`
            f = f.child(El::block().abs(0.0, 0.0, 0.0, 0.0).radius(11.0).shadow(&[sh(0.0, 0.0, 0.0, 3.0, ACC_S())]).inset(&[sh(0.0, 0.0, 0.0, 1.0, ACC())]).opacity(ring).no_hit());
        }
        f
    }

    /// One chip `.fch` (label + an optional chevron).
    fn chip(cx: &mut Cx, k: Key, label: &str, on: bool, chevron: bool) -> El {
        let hv = cx.hover_t(k, 150.0, EASE);
        let pr = cx.active_t(k, 120.0, EASE);
        // `#sw .fch{height:24px;padding:0 10px;border-radius:12px;background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair);
        //   font-size:11.5px}` `:hover{background:var(--ctl-h)}` `:active{scale(.96)}` `.on{background:var(--sel);
        //   box-shadow:inset 0 0 0 1.5px var(--acc);font-weight:600}`
        let (bg, ins) = if on { (SEL(), sh(0.0, 0.0, 0.0, 1.5, ACC())) } else { (cmix(CTL(), CTL_H(), hv), sh(0.0, 0.0, 0.0, 0.5, HAIR())) };
        let mut c = El::row()
            .center()
            .gap(5.0)
            .h(24.0)
            .none()
            .pad(0.0, 10.0, 0.0, 10.0)
            .radius(12.0)
            .bg(bg)
            .inset(&[ins])
            .scale(1.0 - 0.04 * pr)
            .on_click(k)
            .cursor(Cursor::Hand)
            .child(El::text(label, pieces::btn_font(11.5, if on { 600 } else { 400 }), FG(), lh(11.5, 1.35)).no_hit());
        if chevron {
            // the select's chevron (`.dd svg` look, 8 x 5, stroke 1.4)
            c = c.pad(0.0, 9.0, 0.0, 10.0).child(El::icon("cd", 8.0, 1.4, FG2()).size(8.0, 5.0).none().no_hit());
        }
        c
    }

    /// `.fchs` with the seven chips + the type picker.
    fn chips(&self, cx: &mut Cx) -> El {
        let mut kids = Vec::new();
        for (i, f) in srch::Filter::CHIPS.iter().enumerate() {
            // a picked type replaces the chips' choice: none of them is on then
            let on = self.ext.is_none() && *f == self.filter();
            kids.push(Self::chip(cx, idx(K_CHIP, i), f.label(), on, false));
        }
        let label = match self.ext {
            Some(e) => format!(".{e}"),
            None => "Type".to_string(),
        };
        kids.push(Self::chip(cx, K_TYPE, &label, self.ext.is_some(), true));
        // `.fchs{display:flex;flex-wrap:wrap;gap:6px;margin:10px 0 0}`
        El::row().wrap().gap(6.0).margin(10.0, 0.0, 0.0, 0.0).children(kids)
    }

    /// `.shint`: the line under the chips while the field is empty (`padding:48px 0 10px;text-align:center;font-size:12.5px;
    /// color:var(--fg2);line-height:1.5` + `small{display:block;font-size:11px;color:var(--fg3)}`).
    fn hint(&self, line: &str, small: Option<&str>, pad_top: f32) -> El {
        let mut h = El::col()
            .items(AlignItems::CENTER)
            .pad(pad_top, 0.0, 10.0, 0.0)
            .child(El::text(line, Font::new(12.5, 400), FG2(), lh(12.5, 1.5)).align(Align::Center));
        if let Some(s) = small {
            h = h.child(El::text(s, Font::new(11.0, 400), FG3(), lh(11.0, 1.5)).align(Align::Center).wrapping());
        }
        h
    }

    /// What the page says about Everything when it is not ready (None = ready, or not known yet): not installed (one line +
    /// "Install Everything"), installing, a failed / cancelled install, starting / building / loading (+ how long), or not
    /// answering.
    fn engine_note(&self, cx: &mut Cx, pad_top: f32) -> Option<El> {
        let st = self.engine?;
        if self.installing {
            let row = El::row().center().justify(JustifyContent::CENTER).gap(8.0).child(bits::uspin(cx, 0.0)).child(El::text("Installing Everything\u{2026}", Font::new(12.5, 400), FG2(), lh(12.5, 1.5)));
            return Some(
                El::col()
                    .items(AlignItems::CENTER)
                    .pad(pad_top, 0.0, 10.0, 0.0)
                    .child(row)
                    .child(El::text("Windows asks once for permission", Font::new(11.0, 400), FG3(), lh(11.0, 1.5))),
            );
        }
        match st {
            srch::EverythingStatus::Running { .. } => None,
            srch::EverythingStatus::NotInstalled => {
                let small = self.install_err.clone().unwrap_or_else(|| "Free, from voidtools \u{00b7} apps are found without it".to_string());
                let label = if self.install_err.is_some() { "Try again" } else { "Install Everything" };
                Some(
                    El::col()
                        .items(AlignItems::CENTER)
                        .pad(pad_top, 0.0, 10.0, 0.0)
                        .child(El::text("Search uses Everything (free)", Font::new(12.5, 400), FG2(), lh(12.5, 1.5)))
                        .child(El::text(small, Font::new(11.0, 400), FG3(), lh(11.0, 1.5)).align(Align::Center).wrapping())
                        .child(El::block().margin(10.0, 0.0, 0.0, 0.0).child(cbtn(cx, K_INSTALL, label, Kind::Primary, true, false, 0.0))),
                )
            }
            // started, but it never answered: said once, no spinner (opening the tab again starts it again)
            srch::EverythingStatus::NotRunning if self.no_answer => Some(self.hint("Everything isn\u{2019}t answering", Some("It is started again when you open this tab \u{00b7} apps still show"), pad_top)),
            // started, its index not ready yet: what it does + how long the page has waited (Everything 1.4 tells no file
            // count or percent while it indexes: no ETA is made up)
            st => {
                let (line, after) = match st {
                    srch::EverythingStatus::Loading { building: true } => ("Everything is building its file list\u{2026}", "keep this tab open until it\u{2019}s done \u{00b7} apps already show"),
                    srch::EverythingStatus::Loading { .. } => ("Everything is loading its file list\u{2026}", "files and folders show when it\u{2019}s done \u{00b7} apps already do"),
                    _ => ("Starting Everything\u{2026}", "apps already show"),
                };
                let small = format!("{} so far \u{00b7} {after}", waited_text(cx.now - self.wait_since));
                let row = El::row().center().justify(JustifyContent::CENTER).gap(8.0).child(bits::uspin(cx, 0.0)).child(El::text(line, Font::new(12.5, 400), FG2(), lh(12.5, 1.5)));
                Some(
                    El::col()
                        .items(AlignItems::CENTER)
                        .pad(pad_top, 0.0, 10.0, 0.0)
                        .child(row)
                        .child(El::text(small, Font::new(11.0, 400).tnum(), FG3(), lh(11.0, 1.5)).align(Align::Center).wrapping()),
                )
            }
        }
    }

    /// The name with the typed words marked (`mark{background:var(--sel);border-radius:3px;box-shadow:0 0 0 1.5px var(--sel)}`;
    /// selected row: `mark{background:transparent;box-shadow:none;font-weight:600}`).
    fn marked(name: &str, words: &[String], selected: bool) -> El {
        let ranges = srch::match_ranges(name, words);
        let chars: Vec<char> = name.chars().collect();
        let mut row = El::row().min_w(0.0).clip();
        let mut at = 0;
        let f = Font::new(13.0, 400);
        let l = lh(13.0, 1.35);
        for (s, e) in ranges.into_iter().chain(std::iter::once((chars.len(), chars.len()))) {
            if s > at {
                row = row.child(El::text(chars[at..s].iter().collect::<String>(), f, FG(), l).none());
            }
            if e > s {
                let t: String = chars[s..e].iter().collect();
                row = row.child(if selected {
                    El::text(t, Font::new(13.0, 600), FG(), l).none()
                } else {
                    El::text(t, f, FG(), l).none().radius(3.0).bg(SEL()).shadow(&[sh(0.0, 0.0, 0.0, 1.5, SEL())])
                });
            }
            at = e;
        }
        row
    }

    /// One result row (`.srw`).
    fn row(&self, cx: &mut Cx, it: &srch::Item, i: usize, first: bool, words: &[String]) -> El {
        let k = idx(K_ROW, i);
        let selected = i == self.sel;
        let hv = cx.hover_t(k, 100.0, EASE);
        let ctx = self.menu.map(|m| m.0 == i).unwrap_or(false);
        // `.srw{display:flex;align-items:center;gap:11px;height:44px;padding:0 12px;transition:background-color .1s ease}`
        // `.srw:hover,.srw.ctx{background:var(--hov)}` `.srw.sel{background:var(--sel)}`
        let bg = if selected { SEL() } else { HOV().mul_a(if ctx { 1.0 } else { hv }) };
        let sample = if self.fake.is_some() { sample_tile(it) } else { None };
        let where_ = if it.kind == srch::ItemKind::App && it.path.starts_with("ms-settings") { "Windows Settings".to_string() } else { it.path.clone() };
        let where_ = if it.kind == srch::ItemKind::File { it.path.rsplit_once('\\').map(|(d, _)| d.to_string()).unwrap_or_default() } else { where_ };
        // `.srw .snm{flex:1;min-width:0}` `>span{font-size:13px;ellipsis}` `small{font-size:11px;color:var(--fg3);ellipsis}`
        // `h('small',{text:where,title:where})`
        let mut small = El::text(where_.clone(), Font::new(11.0, 400), FG3(), lh(11.0, 1.35)).ellipsis();
        if !where_.is_empty() {
            small = small.key(sub(k, "w")).title(&where_);
        }
        let snm = El::col().flex1().child(Self::marked(&it.name, words, selected)).child(small);
        // `.srw .smeta{flex:none;font-size:11px;color:var(--fg3);text-align:right;font-variant-numeric:tabular-nums;line-height:14px}`
        // `.smeta b{display:block;font-weight:400;color:var(--fg2)}`
        let mf = Font::new(11.0, 400).tnum();
        let meta = match it.kind {
            srch::ItemKind::App => El::col().items(AlignItems::FLEX_END).none().child(El::text("App", mf, FG3(), 14.0)),
            _ => {
                let top = if it.kind == srch::ItemKind::Folder {
                    self.folder_counts.get(&it.path).copied().map(srch::items_text).unwrap_or_default()
                } else {
                    it.size_text().unwrap_or_default()
                };
                El::col().items(AlignItems::FLEX_END).none().child(El::text(top, mf, FG2(), 14.0)).child(El::text(it.date_text().unwrap_or_default(), mf, FG3(), 14.0))
            }
        };
        // `.srw .ent{width:22px;height:18px;margin-left:-4px;border-radius:4px;background:var(--key);box-shadow:inset 0 0 0 .5px var(--hair),
        //   0 1px 0 rgba(0,0,0,.22);font:600 11px/1;color:var(--fg2);opacity:0}` `.srw.sel .ent{opacity:1}`
        let ent = El::block()
            .size(22.0, 18.0)
            .none()
            .margin(0.0, 0.0, 0.0, -4.0)
            .radius(4.0)
            .bg(KEY())
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
            .shadow(&[sh(0.0, 1.0, 0.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.22))])
            .place_center()
            .opacity(if selected { 1.0 } else { 0.0 })
            .child(El::text("\u{21b5}", Font::new(11.0, 600), FG2(), 11.0));
        let mut r = El::row()
            .center()
            .gap(11.0)
            .h(44.0)
            .pad(0.0, 12.0, 0.0, 12.0)
            .bg(bg)
            .on_click(k)
            .child(tile(&tile_of(it, sample), 24.0))
            .child(snm)
            .child(meta)
            .child(ent);
        // `.srw::before{left:47px;right:0;top:0;height:1px;background:var(--hair)}` `.srw.first::before{display:none}`
        // `.srw.sel::before,.srw.sel+.srw::before{opacity:0}`
        let prev_sel = i > 0 && i - 1 == self.sel;
        if !first && !selected && !prev_sel {
            r = r.child(El::block().abs(47.0, 0.0, 0.0, f32::NAN).h(1.0).bg(HAIR()).no_hit());
        }
        r
    }

    fn results(&self, cx: &mut Cx) -> Vec<El> {
        let mut out = Vec::new();
        let Some(res) = self.results.clone() else { return out };
        let words = self.query().words();
        let mut i = 0;
        for (gi, g) in res.groups.iter().enumerate() {
            if g.items.is_empty() {
                continue;
            }
            let mut head = group::gh(g.title).child(El::text(if g.total_is_lower_bound { format!("{}+", g.total) } else { g.total.to_string() }, Font::new(11.0, 400), FG3(), lh(11.0, 1.35)));
            if let Some(n) = g.show_all() {
                // `lnk('Show all N')` with `margin-left:auto;font-size:11px`
                head = head.child(link::link(cx, idx(K_MORE, gi), &format!("Show all {n}"), 11.0).ml_auto());
            }
            let mut rows = Vec::new();
            for (j, it) in g.items.iter().enumerate() {
                rows.push(self.row(cx, it, i, j == 0, &words));
                i += 1;
            }
            // `.grp.srg{overflow:hidden}`
            out.push(El::block().child(head).child(group::grp(rows).clip()));
        }
        let no_files = res.files_from == srch::FilesFrom::Nothing && res.note.is_some();
        let pad = if res.is_empty() { 40.0 } else { 22.0 };
        if no_files {
            // Everything is not there / not ready: said calmly, in the hint's style, with what to do (no error popup)
            match self.engine_note(cx, pad) {
                Some(n) => out.push(n),
                None => out.push(self.hint("Files and folders can\u{2019}t be searched right now", None, pad)),
            }
        } else if res.is_empty() {
            // `.snone{padding:40px 0 10px;text-align:center;font-size:12.5px;color:var(--fg2)}`
            out.push(
                El::block()
                    .pad(40.0, 0.0, 10.0, 0.0)
                    .child(El::text(srch::none_text(&self.text), Font::new(12.5, 400), FG2(), lh(12.5, 1.35)).align(Align::Center)),
            );
        }
        out
    }

    fn menu_el(&self, cx: &mut Cx) -> Option<El> {
        let (i, x, y) = self.menu?;
        let flat = self.flat();
        let it = flat.get(i)?;
        let svc = self.svc.clone()?;
        let acts = svc.menu_for(it);
        if acts.is_empty() {
            return None;
        }
        let head = svc.menu_header(it);
        // `h('div',{class:'mhead',text:full,title:full})`
        let mut list = vec![Row::HeadTitled(&head, &head)];
        for a in &acts {
            let ic = match a {
                srch::MenuAction::OpenFileLocation => "fold",
                srch::MenuAction::CopyPath => "copy",
                srch::MenuAction::OpenWith => "open",
            };
            list.push(Row::Item(It::icon(ic, a.label())));
        }
        // row j + 1 = action j (the header is row 0)
        Some(mitems::menu(cx, K_MENU, &list, Place::At(x, y), 200.0))
    }

    /// The type picker's list (a radio list under its chip): "Any type", a line, then one extension per row with what it is.
    fn type_menu_el(&self, cx: &mut Cx) -> Option<El> {
        let bx = self.type_menu?;
        let labels: Vec<String> = TYPES.iter().map(|(e, _)| format!(".{e}")).collect();
        let mut list = vec![Row::Item(It::tick("Any type", self.ext.is_none())), Row::Sep];
        for (i, (e, what)) in TYPES.iter().enumerate() {
            list.push(Row::Item(It::tick(&labels[i], self.ext == Some(*e)).right(Right::Mr(what))));
        }
        Some(mitems::menu(cx, K_TYPEM, &list, Place::Under(bx.0, bx.1, bx.2, bx.3), 150.0))
    }

    /// A pick in the type list (row 0 = Any type, row 1 = the line, row 2.. = TYPES).
    fn pick_type(&mut self, row: usize) {
        self.type_menu = None;
        self.ext = match row {
            0 => None,
            r if r >= 2 => TYPES.get(r - 2).map(|t| t.0),
            _ => return,
        };
        if self.ext.is_some() {
            self.filter = None;
        }
        self.run();
    }
}

impl Page for Search {
    fn id(&self) -> &'static str {
        "srch"
    }
    fn name(&self) -> &'static str {
        "Search"
    }
    fn icon(&self) -> &'static str {
        "find"
    }
    fn open(&mut self, env: &Env, now: f64) {
        self.env = env.clone();
        self.now = now;
        // a test copy: the drawing's sample on the fake (BU_SRCH_NOENGINE=1 = Everything not installed, for its picture);
        // a real-read test copy: this PC, read only (never starts / installs Everything); the app: this PC
        let var = |n: &str| env.test && std::env::var(n).map(|v| v == "1").unwrap_or(false);
        // ONE service for the app's life (env.keep): its start / stop of our Everything are serialised, so a quick reopen
        // never races the stop of the last close (the end review, Order 030)
        let held = match env.keep.get::<Held>(KEEP_SVC) {
            Some(h) => h,
            None => {
                let h = if env.fake() {
                    let f = sample_os(!var("BU_SRCH_NOENGINE"));
                    Held { svc: Arc::new(srch::SearchService::new(f.clone())), fake: Some(f) }
                } else {
                    Held { svc: Arc::new(Self::real_service(env.test)), fake: None }
                };
                env.keep.put(KEEP_SVC, h.clone());
                h
            }
        };
        self.fake = held.fake;
        self.svc = Some(held.svc);
        let (tx, rx) = mpsc::channel();
        self.tx = Some(tx);
        self.rx = Some(rx);
        self.stop = Arc::new(AtomicBool::new(false));
        self.focus_once = true;
        self.prepare();
    }
    fn close(&mut self) {
        if let Some(c) = self.cancel.take() {
            c.cancel();
        }
        self.stop.store(true, Ordering::SeqCst);
        // leaving the tab / closing the menu: OUR Everything quits (the owner: no RAM / CPU unless it is being used)
        if let Some(s) = self.svc.clone() {
            if self.env.fake() {
                s.release();
            } else {
                std::thread::spawn(move || s.release());
            }
        }
        let env = std::mem::take(&mut self.env);
        *self = Search { env, ..Search::default() };
    }
    fn start(&self, s: &mut crate::services::Services) {
        // the Search key (set in Settings › All shortcuts): the menu opens on this tab, its field focused
        s.add_action(crate::keys::Action::new("srch.open", "Search", "srch"), |down| {
            if down {
                crate::services::show_menu("srch", Some("focus"));
            }
        });
    }
    fn jump(&mut self, target: &str) {
        if target == "focus" {
            self.focus_once = true;
        }
    }
    fn ready(&self) -> bool {
        // the engine's first answer is in (no "catching up" flash on a PC where it already runs)
        self.engine.is_some()
    }
    fn tick(&mut self, now: f64) -> bool {
        self.now = now;
        // (Order 047: true only when an answer came in - every thread of the page wakes the menu, so no polling)
        let shell = self.take_shell();
        self.pump() || shell
    }
    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        self.now = cx.now;
        self.pump();
        self.take_shell();
        if self.focus_once {
            // the drawing's srFocus: the field is focused when the tab opens, ready to type
            self.focus_once = false;
            cx.focus(Some(K_FIELD));
        }
        let mut v = vec![pieces::header(self.name(), None), self.field(cx), self.chips(cx)];
        if self.text.trim().is_empty() {
            match self.engine_note(cx, 48.0) {
                Some(n) => v.push(n),
                None => v.push(self.hint("Type to find apps, folders and files on this PC.", None, 48.0)),
            }
        } else {
            v.extend(self.results(cx));
        }
        v
    }
    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        match ev {
            Ev::Press(k, _, _, bx) => {
                self.press_box.insert(*k, *bx);
            }
            Ev::Char(k, c) if *k == K_FIELD => {
                if !c.is_control() {
                    self.text.push(*c);
                    self.run();
                }
            }
            Ev::Key(k, vk) if *k == K_FIELD => match *vk {
                VK_BACK => {
                    self.text.pop();
                    self.run();
                }
                VK_ESC => {
                    // the drawing: Esc clears the text; an empty field loses focus (the frame blurs it on Esc)
                    if !self.text.is_empty() {
                        self.text.clear();
                        self.run();
                    }
                }
                VK_UP => self.sel = self.sel.saturating_sub(1),
                VK_DOWN => self.sel = (self.sel + 1).min(self.flat().len().saturating_sub(1)),
                VK_ENTER => self.open_item(self.sel),
                _ => {}
            },
            // a right-click on a result row opens its menu at the pointer
            Ev::Context(k, x, y) => {
                if let Some(i) = (0..self.flat().len()).find(|i| *k == idx(K_ROW, *i)) {
                    self.context_menu(i, *x, *y);
                }
            }
            Ev::Click(k) => {
                if *k == sub(K_FIELD, "x") {
                    self.text.clear();
                    self.run();
                    cx.focus(Some(K_FIELD));
                    return;
                }
                if *k == K_INSTALL {
                    self.install();
                    return;
                }
                if *k == K_TYPE {
                    self.type_menu = if self.type_menu.is_some() { None } else { Some(self.press_box.get(&K_TYPE).copied().unwrap_or((380.0, 150.0, 60.0, 24.0))) };
                    return;
                }
                if self.type_menu.is_some() {
                    if let Some(r) = (0..TYPES.len() + 2).find(|r| *k == idx(K_TYPEM, *r)) {
                        self.pick_type(r);
                        cx.focus(Some(K_FIELD));
                        return;
                    }
                }
                for (i, f) in srch::Filter::CHIPS.iter().enumerate() {
                    if *k == idx(K_CHIP, i) {
                        self.filter = Some(*f);
                        self.ext = None;
                        self.run();
                        cx.focus(Some(K_FIELD));
                        return;
                    }
                }
                if let Some(r) = self.results.clone() {
                    for (gi, g) in r.groups.iter().enumerate() {
                        if *k == idx(K_MORE, gi) {
                            if self.ext.is_none() {
                                self.filter = Some(match g.kind {
                                    srch::ItemKind::App => srch::Filter::Apps,
                                    srch::ItemKind::Folder => srch::Filter::Folders,
                                    srch::ItemKind::File => srch::Filter::CHIPS.iter().copied().find(|f| f.label() == g.title).unwrap_or(srch::Filter::Files),
                                });
                            }
                            self.run();
                            cx.focus(Some(K_FIELD));
                            return;
                        }
                    }
                }
                let n = self.flat().len();
                for i in 0..n {
                    if *k == idx(K_ROW, i) {
                        self.sel = i;
                        self.open_item(i);
                        return;
                    }
                }
                if let Some((i, _, _)) = self.menu {
                    let flat = self.flat();
                    if let (Some(it), Some(svc)) = (flat.get(i), self.svc.clone()) {
                        for (j, a) in svc.menu_for(it).iter().enumerate() {
                            if *k == idx(K_MENU, j + 1) {
                                self.menu = None;
                                // Order 047: Open file location (SHOpenFolderAndSelectItems) / Open with… (SHOpenWithDialog)
                                // / Copy path hold their thread 0.1-3 s: on their own thread, the toast comes back
                                // (`take_shell`). (Open with… is Windows' own dialog with no owner window: it stays up on
                                // that thread until it is answered.)
                                let (svc, it, a, out) = (svc.clone(), it.clone(), *a, self.shell.clone());
                                crate::offui::spawn("srch-menu", move || {
                                    let t = match svc.run_menu(&it, a) {
                                        Ok(()) if a == srch::MenuAction::CopyPath => Some("Path copied".to_string()),
                                        Ok(()) => None,
                                        Err(_) => Some(format!("{} \u{00b7} not possible", a.label())),
                                    };
                                    if let (Some(t), Ok(mut v)) = (t, out.lock()) {
                                        v.push(t);
                                    }
                                });
                                return;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let mut kids = Vec::new();
        if let Some(m) = self.menu_el(cx) {
            kids.push(m.z(20));
        }
        if let Some(m) = self.type_menu_el(cx) {
            kids.push(m.z(20));
        }
        if let Some((t, at)) = self.toast.clone() {
            if cx.now - at < toast::SHOW_MS + 400.0 {
                kids.push(toast::toast(cx, K_TOAST, &t, at, false));
            } else {
                self.toast = None;
            }
        }
        if kids.is_empty() {
            return None;
        }
        Some(El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(crate::ui::WIN_W, crate::ui::WIN_H).no_hit().children(kids))
    }
    fn popup_dismiss(&mut self) {
        self.menu = None;
        self.type_menu = None;
    }
    fn describe(&self) -> String {
        let r = self.results.as_ref();
        format!(
            "text={:?} filter={} type={} groups={} rows={} sel={} engine={:?} installing={} menu={}",
            self.text,
            self.filter().label(),
            self.ext.unwrap_or("any"),
            r.map(|r| r.groups.iter().map(|g| format!("{}:{}", g.title, g.total)).collect::<Vec<_>>().join(",")).unwrap_or_default(),
            self.flat().len(),
            self.sel,
            self.engine,
            self.installing,
            self.menu.is_some()
        )
    }
}

impl Search {
    /// This PC's search: the app's layer (starts / quits / installs Everything), or a real-read test copy's read-only one.
    fn real_service(test: bool) -> srch::SearchService {
        #[cfg(windows)]
        {
            srch::SearchService::new(Arc::new(if test { srch::RealOs::read_only() } else { srch::RealOs::new() }))
        }
        #[cfg(not(windows))]
        {
            let _ = test;
            srch::SearchService::new(sample_os(false))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::Gfx;
    use crate::ui::cx::State;
    use crate::ui::lay::Laid;

    fn page(engine: bool) -> Search {
        let mut p = Search::default();
        if engine {
            p.open(&Env { test: true, ..Env::default() }, 0.0);
        } else {
            p.open(&Env { test: true, ..Env::default() }, 0.0);
            // the same page on a PC without Everything
            let f = p.fake.clone().unwrap();
            f.state().everything = srch::EverythingStatus::NotInstalled;
            p.engine = None;
            p.prepare();
        }
        p
    }
    fn fake(p: &Search) -> Arc<srch::FakeOs> {
        p.fake.clone().unwrap()
    }
    /// Order 047: a shell call runs on its own thread - tick the page (like the menu does when it is woken) until `done`.
    pub(super) fn wait_for(p: &mut Search, done: impl Fn(&Search) -> bool) {
        let t0 = std::time::Instant::now();
        while !done(p) {
            assert!(t0.elapsed().as_secs() < 10, "the shell call never ended");
            std::thread::sleep(std::time::Duration::from_millis(5));
            let now = p.now;
            p.tick(now);
        }
    }
    fn cx_run(f: impl FnOnce(&mut Cx)) {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        f(&mut cx);
    }
    fn typ(p: &mut Search, s: &str) {
        cx_run(|cx| {
            for c in s.chars() {
                p.event(&Ev::Char(K_FIELD, c), cx);
            }
        });
    }
    fn click(p: &mut Search, k: Key) {
        cx_run(|cx| p.event(&Ev::Click(k), cx));
    }
    fn texts(e: &El, out: &mut Vec<String>) {
        if let crate::ui::el::Content::Text(t) = &e.content {
            out.push(t.s.to_string());
        }
        e.children.iter().for_each(|c| texts(c, out));
    }
    fn shown(p: &mut Search) -> Vec<String> {
        shown_at(p, 0.0)
    }
    /// The page's texts built at this moment of its clock (ms).
    fn shown_at(p: &mut Search, now: f64) -> Vec<String> {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(now, false, &g, &mut st);
        let mut v = Vec::new();
        p.build(&mut cx).iter().for_each(|e| texts(e, &mut v));
        v
    }

    #[test]
    fn opening_searches_nothing_and_focuses_the_field() {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut p = page(true);
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        let _ = p.build(&mut cx);
        assert!(cx.focused(K_FIELD));
        assert!(p.results.is_none());
        assert!(fake(&p).state().queries.is_empty());
        assert_eq!(p.engine, Some(srch::EverythingStatus::Running { version: 1 }));
    }

    #[test]
    fn grouped_and_capped_like_the_drawing() {
        let mut p = page(true);
        typ(&mut p, "val");
        let r = p.results.clone().unwrap();
        let names: Vec<(&str, Vec<&str>)> = r.groups.iter().map(|g| (g.title, g.items.iter().map(|i| i.name.as_str()).collect())).collect();
        assert_eq!(names[0], ("Apps", vec!["VALORANT"]));
        assert_eq!(names[1], ("Folders", vec!["VALORANT"]));
        assert_eq!(names[2].0, "Files");
        assert_eq!(names[2].1[0], "valorant_settings.txt");
        assert_eq!(r.files_from, srch::FilesFrom::Everything { version: 1 });
        // "s" matches many: Apps capped at 4 with Show all
        let mut p = page(true);
        typ(&mut p, "s");
        let g = &p.results.as_ref().unwrap().groups[0];
        assert_eq!(g.items.len(), 4);
        assert!(g.show_all().is_some());
    }

    #[test]
    fn keys_move_and_enter_opens() {
        let mut p = page(true);
        typ(&mut p, "steam");
        cx_run(|cx| {
            p.event(&Ev::Key(K_FIELD, VK_DOWN), cx);
            assert_eq!(p.sel, 1);
            p.event(&Ev::Key(K_FIELD, VK_ENTER), cx);
        });
        let it = p.flat()[1].clone();
        // (Order 047: the open runs on its own thread)
        wait_for(&mut p, |p| !fake(p).state().actions.is_empty());
        assert_eq!(fake(&p).state().actions, vec![format!("open {}", it.path)]);
        cx_run(|cx| p.event(&Ev::Key(K_FIELD, VK_ESC), cx));
        assert!(p.text.is_empty() && p.results.is_none());
    }

    #[test]
    fn chips_filter() {
        let mut p = page(true);
        typ(&mut p, "screenshot");
        click(&mut p, idx(K_CHIP, 4));
        let r = p.results.clone().unwrap();
        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.groups[0].title, "Pictures");
        assert!(r.groups[0].items.iter().all(|i| i.file_type == Some(srch::FileType::Picture)));
    }

    /// the owner (test build 1): pick what TYPE of file - e.g. .txt.
    #[test]
    fn the_type_picker_keeps_one_extension() {
        let mut p = page(true);
        typ(&mut p, "valorant");
        click(&mut p, K_TYPE);
        assert!(p.type_menu.is_some());
        click(&mut p, idx(K_TYPEM, 2)); // .txt
        assert_eq!(p.ext, Some("txt"));
        assert!(p.type_menu.is_none());
        let r = p.results.clone().unwrap();
        assert_eq!(r.groups.len(), 1, "files only");
        assert!(r.groups[0].items.iter().all(|i| i.name.ends_with(".txt")), "{:?}", r.groups[0].items);
        assert!(shown(&mut p).contains(&".txt".to_string()), "the chip says which type");
        // a chip clears the type; "Any type" too
        click(&mut p, idx(K_CHIP, 0));
        assert_eq!(p.ext, None);
        click(&mut p, K_TYPE);
        click(&mut p, idx(K_TYPEM, 8)); // .mp4
        assert_eq!(p.ext, Some("mp4"));
        click(&mut p, K_TYPE);
        click(&mut p, idx(K_TYPEM, 0));
        assert_eq!(p.ext, None);
    }

    #[test]
    fn right_click_menu_runs_through_the_service() {
        let mut p = page(true);
        typ(&mut p, "autoexec");
        p.context_menu(0, 200.0, 200.0);
        click(&mut p, idx(K_MENU, 2)); // header, Open file location, Copy path
        // (Order 047: the menu entry runs on its own thread; its toast comes back through `tick`)
        wait_for(&mut p, |p| p.toast.is_some());
        assert_eq!(p.toast.as_ref().unwrap().0, "Path copied");
        let log = fake(&p).state().actions.clone();
        assert!(log[0].starts_with("copy C:\\Program Files (x86)\\Steam"), "{log:?}");
    }

    /// the owner, Oct 8: Everything not installed = one clean line + "Install Everything"; apps still come up; no error popup.
    #[test]
    fn without_everything_it_offers_the_install() {
        let mut p = page(false);
        assert_eq!(p.engine, Some(srch::EverythingStatus::NotInstalled));
        let s = shown(&mut p);
        assert!(s.contains(&"Search uses Everything (free)".to_string()), "{s:?}");
        assert!(s.contains(&"Install Everything".to_string()));
        typ(&mut p, "val");
        let r = p.results.clone().unwrap();
        assert_eq!(r.groups.len(), 1, "apps only");
        assert_eq!(r.groups[0].title, "Apps");
        assert!(p.toast.is_none(), "no error popup");
        assert!(shown(&mut p).contains(&"Install Everything".to_string()), "the install line under the apps");
        // a cancelled install says so and offers it again
        fake(&p).state().install_result = Err(srch::SearchError::InstallCancelled);
        click(&mut p, K_INSTALL);
        assert!(!p.installing);
        assert_eq!(p.install_err.as_deref(), Some("The install was cancelled"));
        assert!(shown(&mut p).contains(&"Try again".to_string()));
        // installed: started, the words asked again with files and folders
        fake(&p).state().install_result = Ok(());
        click(&mut p, K_INSTALL);
        assert_eq!(fake(&p).state().actions, vec!["install everything", "install everything", "start everything"]);
        assert!(p.ready());
        let r = p.results.clone().unwrap();
        assert!(r.groups.iter().any(|g| g.kind == srch::ItemKind::Folder), "files and folders now");
    }

    /// the owner, Oct 8: our Everything runs only while the tab is used; a line saying what it does until its index is in.
    #[test]
    fn ours_starts_with_the_tab_builds_and_quits_with_it() {
        let mut p = page(true);
        let f = fake(&p);
        {
            let mut st = f.state();
            st.everything = srch::EverythingStatus::NotRunning;
            st.everything_starts_as = srch::EverythingStatus::Loading { building: true };
        }
        p.engine = None;
        p.prepare();
        assert_eq!(p.engine, Some(srch::EverythingStatus::Loading { building: true }));
        assert!(shown(&mut p).contains(&"Everything is building its file list\u{2026}".to_string()));
        // it caught up while words were typed: files show then
        typ(&mut p, "val");
        assert_eq!(p.results.as_ref().unwrap().groups.len(), 1, "apps only while it loads");
        f.state().everything = srch::EverythingStatus::Running { version: 1 };
        p.tx.as_ref().unwrap().send(Msg::Engine(srch::EverythingStatus::Running { version: 1 })).unwrap();
        p.pump();
        assert!(p.results.as_ref().unwrap().groups.len() >= 2);
        p.close();
        assert_eq!(f.state().actions, vec!["start everything", "stop everything"]);
    }

    /// the owner (test build 2): "it can't just say wait and it could be days or weeks" - every not-ready state says what
    /// Everything is doing and how long the page has waited (m:ss, the page's clock); never a bare "wait".
    #[test]
    fn while_it_is_not_ready_the_page_says_what_and_how_long() {
        let mut p = page(true);
        let f = fake(&p);
        let opened = 10_000.0;
        let case = |p: &mut Search, st: srch::EverythingStatus, at: f64| -> Vec<String> {
            p.engine = None;
            p.now = opened;
            {
                // (the fake turns a NotRunning one into `everything_starts_as` when the page starts it)
                let mut s = f.state();
                s.everything = st;
                s.everything_starts_as = st;
            }
            p.prepare();
            shown_at(p, opened + at)
        };
        // starting (no answer yet)
        let s = case(&mut p, srch::EverythingStatus::NotRunning, 3_400.0);
        assert!(s.contains(&"Starting Everything\u{2026}".to_string()), "{s:?}");
        assert!(s.contains(&"0:03 so far \u{00b7} apps already show".to_string()), "{s:?}");
        // ours, no index file yet: building, minutes and hours counted
        let s = case(&mut p, srch::EverythingStatus::Loading { building: true }, 83_000.0);
        assert!(s.contains(&"Everything is building its file list\u{2026}".to_string()), "{s:?}");
        assert!(s.contains(&"1:23 so far \u{00b7} keep this tab open until it\u{2019}s done \u{00b7} apps already show".to_string()), "{s:?}");
        let s = case(&mut p, srch::EverythingStatus::Loading { building: true }, 3_723_000.0);
        assert!(s.iter().any(|t| t.starts_with("1:02:03 so far")), "{s:?}");
        // a saved index (or the user's own copy)
        let s = case(&mut p, srch::EverythingStatus::Loading { building: false }, 1_400.0);
        assert!(s.contains(&"Everything is loading its file list\u{2026}".to_string()), "{s:?}");
        assert!(s.contains(&"0:01 so far \u{00b7} files and folders show when it\u{2019}s done \u{00b7} apps already do".to_string()), "{s:?}");
        // the same line under the apps while words are typed
        typ(&mut p, "val");
        assert!(shown_at(&mut p, opened + 61_000.0).iter().any(|t| t.starts_with("1:01 so far")));
        // never answered: said plainly, no clock
        p.tx.as_ref().unwrap().send(Msg::Engine(srch::EverythingStatus::NotRunning)).unwrap();
        p.tx.as_ref().unwrap().send(Msg::NoAnswer).unwrap();
        p.pump();
        let s = shown_at(&mut p, opened + 200_000.0);
        assert!(s.contains(&"Everything isn\u{2019}t answering".to_string()), "{s:?}");
        assert!(!s.iter().any(|t| t.contains("so far")), "{s:?}");
        // ready: no line at all
        p.tx.as_ref().unwrap().send(Msg::Engine(srch::EverythingStatus::Running { version: 1 })).unwrap();
        p.pump();
        assert!(!p.no_answer);
        let s = shown_at(&mut p, opened + 200_000.0);
        assert!(!s.iter().any(|t| t.contains("Everything")), "{s:?}");
        assert_eq!(waited_text(-5.0), "0:00");
        assert_eq!(waited_text(599_999.0), "9:59");
    }

    /// the owner (test build 2): "even after it said it scanned everything, it doesn't let me click and type anything". The
    /// frame gives the focus to the innermost keyed element under a press (`Ui::mouse_down`: `keys.first()`); the field's
    /// text had its own key, so a click on the placeholder / the typed words focused the TEXT and every key went there.
    /// Clicks on every part of the field must focus the field, and typing then types.
    #[test]
    fn a_click_anywhere_on_the_field_then_typing_types() {
        let g = Gfx::new(1.0);
        let mut p = page(true);
        let mut st = State::default();
        for (what, pick) in [("the placeholder", 0usize), ("the field's middle", 1), ("its right end", 2), ("the typed words", 0)] {
            let laid = {
                let mut cx = Cx::new(0.0, false, &g, &mut st);
                let kids = p.build(&mut cx);
                Laid::new(&g, El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids), 600.0, None)
            };
            let (fx, fy, fw, fh) = laid.rect_of(K_FIELD).unwrap();
            let (x, y) = match pick {
                0 => {
                    let (tx, ty, tw, th) = laid.rect_of(sub(K_FIELD, "q")).unwrap();
                    (tx + tw.min(40.0) / 2.0, ty + th / 2.0)
                }
                1 => (fx + fw / 2.0, fy + fh / 2.0),
                _ => (fx + fw - 4.0, fy + fh / 2.0),
            };
            // what the frame does on the press: blur, then focus the innermost keyed element
            st.focus = None;
            let (_, keys) = laid.hit(x, y).unwrap();
            st.focus = keys.first().copied();
            assert_eq!(st.focus, Some(K_FIELD), "a click on {what} focuses the field (got {:?})", st.focus);
            let before = p.text.clone();
            let mut cx = Cx::new(0.0, false, &g, &mut st);
            for c in "ab".chars() {
                let k = cx.st.focus.unwrap();
                p.event(&Ev::Char(k, c), &mut cx);
            }
            assert_eq!(p.text, format!("{before}ab"), "typing after a click on {what}");
        }
        assert!(p.results.is_some(), "and it searched");
    }

    #[test]
    fn a_users_own_everything_is_never_stopped() {
        let mut p = page(true);
        let f = fake(&p);
        p.close();
        assert!(f.state().actions.is_empty(), "{:?}", f.state().actions);
    }

    #[test]
    fn closing_drops_everything() {
        let mut p = page(true);
        typ(&mut p, "a");
        p.close();
        assert!(p.svc.is_none() && p.results.is_none() && p.text.is_empty());
    }

    /// The placeholder's box never changes with the caret's blink (the owner: "Search" popped in and out of its box).
    #[test]
    fn the_caret_blink_moves_nothing() {
        let g = Gfx::new(1.0);
        let p = page(true);
        let mut at = Vec::new();
        for now in [0.0, 600.0] {
            let mut st = State::default();
            st.focus = Some(K_FIELD);
            let mut cx = Cx::new(now, false, &g, &mut st);
            let f = p.field(&mut cx);
            let laid = Laid::new(&g, El::block().w(548.0).child(f), 548.0, None);
            at.push(laid.rect_of(sub(K_FIELD, "q")));
        }
        assert!(at[0].is_some());
        assert_eq!(at[0], at[1], "caret on / off: the text stays put");
    }

    /// Boxes = Chromium's (dom_dump on menu-v22, page srch): the field (26, 98) 548 x 42, the first chip (26, 150) 33.73 x 24,
    /// the second (65.73, 150), the hint (26, 174) 548 x 76.75.
    #[test]
    fn boxes_match_the_drawing() {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut p = page(true);
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        let kids = p.build(&mut cx);
        let root = El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
        let laid = Laid::new(&g, root, 600.0, None);
        let r = |k: Key| laid.rect_of(k).map(|(x, y, w, h)| (x, y + 56.0, w, h)).unwrap();
        let near = |a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)| (a.0 - b.0).abs() < 0.05 && (a.1 - b.1).abs() < 0.05 && (a.2 - b.2).abs() < 0.05 && (a.3 - b.3).abs() < 0.05;
        assert!(near(r(K_FIELD), (26.0, 98.0, 548.0, 42.0)), "field {:?}", r(K_FIELD));
        assert!(near(r(idx(K_CHIP, 0)), (26.0, 150.0, 33.7344, 24.0)), "chip 0 {:?}", r(idx(K_CHIP, 0)));
        assert!(near(r(idx(K_CHIP, 1)), (65.7344, 150.0, 45.8281, 24.0)), "chip 1 {:?}", r(idx(K_CHIP, 1)));
        assert!(near(r(idx(K_CHIP, 6)), (354.6563, 150.0, 77.8281, 24.0)), "chip 6 {:?}", r(idx(K_CHIP, 6)));
        assert!((laid.height - (250.75 - 56.0 + 18.0)).abs() < 0.05, "page height {}", laid.height);
    }

    /// The proof pictures of Order 043, made with NO window and no running app: the real frame (`Ui`, every page on its
    /// fakes - a test copy's switch is set first) painted like `Menu::snapshot_over` (desktop picture, glass, tint, caption
    /// buttons, page, edge, top row): the tab while Everything builds its file list, then ready with "val" typed after a
    /// click on the field (through the frame's own mouse_down / mouse_up / char_input). Writes <dir>\srch_<step>.png (the
    /// window + 8 px around it).
    /// `BU_SRCH_SHOTS=<dir> BU_SRCH_DESK=<desk.png> cargo test -p bu-app search_pictures -- --ignored`
    #[test]
    #[ignore]
    fn search_pictures() {
        use crate::gfx::CssColor;
        use crate::ui::{Frame, Ui, RADIUS, WIN_H, WIN_W};
        let dir = std::env::var("BU_SRCH_SHOTS").expect("set BU_SRCH_SHOTS=<folder>");
        let desk = std::env::var("BU_SRCH_DESK").expect("set BU_SRCH_DESK=<desk.png>");
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        crate::testmode::set(true, false);
        // the tab's service: the drawing's sample; Everything not answering yet, then building its file list
        let f = sample_os(true);
        {
            let mut st = f.state();
            st.everything = srch::EverythingStatus::NotRunning;
            st.everything_starts_as = srch::EverythingStatus::Loading { building: true };
        }
        crate::keep::app().put(KEEP_SVC, Held { svc: Arc::new(srch::SearchService::new(f.clone())), fake: Some(f.clone()) });
        let dp = crate::png::load_png(&desk).expect("desk");
        let dimg = crate::png::to_image(&dp).expect("desk image");
        let (sw, sh_) = (dp.w as i32, dp.h as i32);
        let (wx, wy) = (sw as f32 - 12.0 - WIN_W, sh_ as f32 - 48.0 - 12.0 - WIN_H);
        let g = Gfx::new(1.0);
        let icons = crate::icons::Icons::new();
        let snap = |ui: &mut Ui, now: f64, name: &str| {
            let fr = Frame { g: &g, icons: &icons, now };
            let mut page = crate::gfx::new_surface(WIN_W as i32, WIN_H as i32).unwrap();
            g.begin(page.canvas());
            ui.draw_pages(&fr);
            g.end();
            let base = page.image_snapshot();
            let mut out = crate::gfx::new_surface(sw, sh_).unwrap();
            out.canvas().draw_image(&dimg, (0, 0), None);
            g.begin(out.canvas());
            let gn = crate::ui::glass_numbers();
            g.backdrop(&dimg, wx, wy, WIN_W, WIN_H, RADIUS, gn.blur_px, &[CssColor::Saturate(gn.saturate), CssColor::Brightness(gn.brightness)]);
            g.end();
            let mut win = crate::gfx::new_surface(sw, sh_).unwrap();
            g.begin(win.canvas());
            g.cv().translate((wx, wy));
            ui.draw_rim(&g);
            ui.draw_caps(&fr);
            ui.draw_pages(&fr);
            ui.draw_edge(&g);
            // (Order 041: the top row, then its names / chevrons and the page's scrollbar, which frost the page)
            ui.draw_dock(&fr);
            ui.draw_dock_overlays(&fr, &base);
            ui.draw_page_scrollbar(&fr, &base);
            g.end();
            out.canvas().draw_image(win.image_snapshot(), (0, 0), None);
            let crop = out.image_snapshot_with_bounds(skia_safe::IRect::from_xywh(wx as i32 - 8, wy as i32 - 8, WIN_W as i32 + 16, WIN_H as i32 + 16)).unwrap();
            let mut cs = crate::gfx::new_surface(crop.width(), crop.height()).unwrap();
            cs.canvas().draw_image(&crop, (0, 0), None);
            crate::png::save_png(&crate::png::from_surface(&mut cs), &format!("{dir}\\{name}.png")).expect("save");
        };
        let step = |ui: &mut Ui, now: &mut f64| {
            for _ in 0..20 {
                *now += 50.0;
                ui.update(*now);
                let fr = Frame { g: &g, icons: &icons, now: *now };
                let mut s = crate::gfx::new_surface(WIN_W as i32, WIN_H as i32).unwrap();
                g.begin(s.canvas());
                ui.draw_pages(&fr);
                g.end();
            }
        };
        let mut now = 5000.0;
        let mut ui = Ui::new(false, true, 0.0);
        step(&mut ui, &mut now);
        let srch_tab = crate::ui::tab_index("srch").unwrap();
        ui.show_tab(srch_tab, now);
        step(&mut ui, &mut now);
        // 1:23 after the tab opened
        now += 83_000.0 - 1000.0;
        step(&mut ui, &mut now);
        snap(&mut ui, now, "srch_1_building");
        assert!(ui.describe().contains("building: true"), "{}", ui.describe());
        // a saved index loading (the next opens)
        ui.show_tab(0, now);
        step(&mut ui, &mut now);
        f.state().everything_starts_as = srch::EverythingStatus::Loading { building: false };
        ui.show_tab(srch_tab, now);
        step(&mut ui, &mut now);
        snap(&mut ui, now, "srch_2_loading");
        // ready: a click on the field (its middle, where the placeholder's words are), then typing
        ui.show_tab(0, now);
        step(&mut ui, &mut now);
        f.state().everything_starts_as = srch::EverythingStatus::Running { version: 1 };
        ui.show_tab(srch_tab, now);
        step(&mut ui, &mut now);
        ui.click("xy:300,300", now);
        step(&mut ui, &mut now);
        ui.click("el:srch.field/q", now);
        for c in "val".chars() {
            ui.char_input(c, now);
        }
        step(&mut ui, &mut now);
        assert!(ui.describe().contains("text=\"val\""), "typed after a click: {}", ui.describe());
        snap(&mut ui, now, "srch_3_ready_typed");
    }
}

/// Order 047: opening a result and the right-click menu's entries never hold the menu's thread (the shell's calls run
/// through `crate::offui`); at rest the page asks for no frames.
#[cfg(test)]
mod offui_tests {
    use super::*;
    use crate::gfx::Gfx;
    use crate::ui::cx::State;

    fn page() -> Search {
        let mut p = Search::default();
        p.open(&Env { test: true, ..Env::default() }, 0.0);
        p
    }
    fn ev(p: &mut Search, e: Ev) {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        p.event(&e, &mut cx);
    }
    fn typ(p: &mut Search, s: &str) {
        for c in s.chars() {
            ev(p, Ev::Char(K_FIELD, c));
        }
    }
    fn actions(p: &Search) -> Vec<String> {
        p.fake.clone().unwrap().state().actions.clone()
    }

    /// Enter on a result with a shell that takes 300 ms: the key hands the thread back within one frame, the open still
    /// happens (on its own thread), and the page asks for no frames once it is done.
    #[test]
    fn enter_opens_off_the_menus_thread() {
        let mut p = page();
        typ(&mut p, "steam");
        let path = p.flat()[0].path.clone();
        crate::offui::set_test_delay(300);
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        // (the painter is made before: only the key itself is timed)
        crate::offui::assert_quick("Search › Enter on a result", || p.event(&Ev::Key(K_FIELD, VK_ENTER), &mut cx));
        drop(cx);
        super::tests::wait_for(&mut p, |p| !actions(p).is_empty());
        crate::offui::set_test_delay(0);
        assert_eq!(actions(&p), vec![format!("open app {path}")]);
        // at rest: no frames, no timed wake-up in the past
        assert!(!p.tick(5000.0), "nothing moves: no frames");
        assert!(p.wake_at(5000.0).is_none_or(|t| t > 5000.0));
    }

    /// The right-click menu's Open file location with a slow shell: the click returns at once, Explorer is asked on its
    /// own thread.
    #[test]
    fn a_menu_entry_runs_off_the_menus_thread() {
        let mut p = page();
        typ(&mut p, "autoexec");
        p.context_menu(0, 200.0, 200.0);
        crate::offui::set_test_delay(300);
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        crate::offui::assert_quick("Search › Open file location", || p.event(&Ev::Click(idx(K_MENU, 1)), &mut cx));
        drop(cx);
        assert!(p.menu.is_none(), "the menu closed at once");
        super::tests::wait_for(&mut p, |p| !actions(p).is_empty());
        crate::offui::set_test_delay(0);
        assert!(actions(&p)[0].starts_with("reveal "), "{:?}", actions(&p));
        assert!(p.toast.is_none(), "a reveal says nothing");
    }

    /// A tick with nothing new (no answer, no toast from a shell call) says "nothing moves".
    #[test]
    fn at_rest_tick_asks_for_no_frames() {
        let mut p = page();
        let _ = p.tick(10.0);
        assert!(!p.tick(20.0), "no answer came in: no frames");
        assert!(p.wake_at(20.0).is_none_or(|t| t > 20.0));
    }
}
