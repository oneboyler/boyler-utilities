//! The Notes tab (Order 116, the owner Oct 23: "a tab that has sticky notes that you can type in ... multiple tabs of sticky
//! notes ... super clean and fit the style", between Screenshots and Mouse). Built only from the app's pieces and look:
//! - the header: "Notes" + the privacy eye (`pieces::eye`) at its right end;
//! - a row of tabs like browser tabs, drawn as the app's chips (Mouse's preset chip `.apc`: 26 px pill, `--ctl`, the
//!   shown one `--sel` + the accent ring): click = show it, double-click = rename (a well in the chip, Enter / a click
//!   elsewhere keeps it, Esc goes back), right-click = delete (a confirm when it has notes), "+" = a new tab;
//! - the shown tab's notes in ONE grid of two columns, each note a glass card (the group box `.grp`, no paper colours)
//!   with its title and text typed straight in, saved at once (`store`: notes.cfg, survives a restart); the last cell is
//!   "+ Note"; right-click a note = "Delete note" (an empty note goes at once).
//! - The eye blurs every note's TEXT (titles stay readable); a click on a blurred note shows it until the tab is left.
//!
//! Nothing slow on open: the file is read once per app run on a worker thread (the frame holds the tab until it is in),
//! then kept in memory; every change goes to one background writer.

pub mod edit;
pub mod store;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;


use crate::anim::EASE;
use crate::gfx::{sh, Align, Font, Rgba};
use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, lh, sub, Cursor, El, Key, RADIUS_PILL};
use crate::ui::pieces::mitems::{self, It, Place, Row};
use crate::ui::pieces::{self, button, eye, group};
use crate::ui::{cmix, ACC, CTL, CTL_H, DASH, FG, HAIR, HOV, ICO_ON, SEL, WELL};

const K_EYE: Key = key("nts.eye");
const K_TAB: Key = key("nts.tab");
const K_ADDTAB: Key = key("nts.addtab");
const K_REN: Key = key("nts.ren");
const K_ADD: Key = key("nts.add");
const K_CARD: Key = key("nts.note");
const K_TITLE: Key = key("nts.title");
const K_TEXT: Key = key("nts.text");
const K_MENU: Key = key("nts.menu");
const K_CONF: Key = key("nts.conf");

const VK_ESCAPE: u16 = 0x1B;
const VK_V: u16 = 0x56;
const VK_A: u16 = 0x41;
const VK_C: u16 = 0x43;
const VK_X: u16 = 0x58;
/// the note menu's rows (`mitems::rows`: row i = `idx(K_MENU, i)`): colours, line, Copy, Paste, line, Delete
const MENU_COPY: usize = 2;
const MENU_PASTE: usize = 3;
const MENU_DELETE: usize = 5;
/// Order 121: the selection colour - the app's own text boxes' (`tinput`): blue with white letters shows on every paper
const SEL_BG: Rgba = Rgba(0.0, 65.0 / 255.0, 198.0 / 255.0, 0.8);
const SEL_FG: Rgba = Rgba::rgb(255, 255, 255);

/// the page's content width (600 - 2 x 26) and the grid: three columns, 12 px apart; a paper note is a square (at least)
const CONTENT_W: f32 = 548.0;
const GAP: f32 = 12.0;
const COL_W: f32 = (CONTENT_W - 2.0 * GAP) / 3.0;
/// a paper note's padding (option A: `padding:12px 13px`)
const PAD: (f32, f32) = (12.0, 13.0);
const TEXT_W: f32 = COL_W - 2.0 * PAD.1;
/// the paper colours in turn (option A: yellow, pink, blue, green) and the ink on them (`color:#2a2a2a`)
const PAPER: [Rgba; 4] = [Rgba::rgb(0xf6, 0xe2, 0x7a), Rgba::rgb(0xf5, 0xb8, 0xc4), Rgba::rgb(0xa9, 0xd8, 0xf2), Rgba::rgb(0xb8, 0xe3, 0xa8)];
const INK: Rgba = Rgba::rgb(0x2a, 0x2a, 0x2a);
/// each note's tilt in degrees (option A: -1.5 .. 1.4): fixed by the note, so a note never moves while it is drawn
const TILTS: [f32; 8] = [-1.5, 1.0, -0.6, 1.4, 0.5, -1.1, 0.8, -0.3];

/// The seed of a note's fake letters: this run's own random salt + the note + its words, so the same words look the same until
/// they change, nothing can be worked back from them, and the next run draws a new set.
fn fake_seed(id: u64, text: &str) -> u64 {
    use std::hash::BuildHasher;
    static SALT: std::sync::OnceLock<std::collections::hash_map::RandomState> = std::sync::OnceLock::new();
    SALT.get_or_init(std::collections::hash_map::RandomState::new).hash_one((id, text))
}

fn tilt(id: u64) -> f32 {
    TILTS[(id % TILTS.len() as u64) as usize]
}
/// the title: `.ch .ttl` 13 px / 600; the text: `.lbl small` style one step up - 12.5 px, --fg2
const F_TITLE: Font = Font::new(13.0, 600);
const F_TEXT: Font = Font::new(12.5, 400);
const LH_TITLE: f32 = 18.0;
const LH_TEXT: f32 = 18.0;
/// a note's text box is at least this many lines tall (it fills a square note: a click on the empty paper types)
const MIN_LINES: usize = 7;
/// Order 121: how much a blurred note's fake letters blur (the eye's own `eye::BLUR` read nothing at all; the owner Oct 24: "you can
/// see that some text is there just very blurred ... blurs a bit less")
const NOTE_BLUR: f32 = 2.2;
/// the chip text (`.apc{font-size:12px}`)
const F_CHIP: Font = Font::new(12.0, 400);
/// Windows' default double-click time
const DBL_MS: f64 = 500.0;
const TITLE_MAX: usize = 60;
const NAME_MAX: usize = 30;
/// a note's text is at most this long (typed or pasted): a pasted log never makes every redraw slow
const TEXT_MAX: usize = 20_000;

/// The notes once read in this app run (by folder: a test copy has its own) - true = the file could not be read (another
/// program held it): shown empty, never saved over, read again at the next open - and the one writer.
static CACHE: Mutex<Option<(PathBuf, store::Notes, bool)>> = Mutex::new(None);
static SAVER: Mutex<Option<(PathBuf, store::Saver)>> = Mutex::new(None);

fn cached(folder: &PathBuf) -> Option<(store::Notes, bool)> {
    CACHE.lock().ok()?.as_ref().filter(|c| &c.0 == folder).map(|c| (c.1.clone(), c.2))
}

/// The newest notes on disk now (the app quits / Windows signs out): the writer finishes and stops.
pub fn flush() {
    let s = SAVER.lock().ok().and_then(|mut s| s.take());
    drop(s);
}

/// Which popup is up.
#[derive(Clone, Debug, PartialEq)]
enum Menu {
    /// "Delete <tab>?" for the tab (its index) at (x, y)
    Tab(usize, f32, f32),
    /// the note menu (colours, Copy, Paste, Delete) for the note (its id) at (x, y): the field right-clicked (1 title, 2 text)
    /// and its selection (lo, hi) - the caret when lo == hi - as they were (the menu takes the focus, and the selection goes)
    Note(u64, f32, f32, u8, usize, usize),
}

#[derive(Default)]
pub struct Notes {
    /// the notes (None until read)
    n: Option<store::Notes>,
    /// where they are kept (None = a unit test: in memory only)
    folder: Option<PathBuf>,
    eye: eye::Eye,
    /// notes shown although the eye blurs (clicked), until the tab is left
    revealed: HashSet<u64>,
    /// the caret of the field that has the focus (a char index) and when it last moved (its blink)
    caret: usize,
    caret_at: f64,
    /// Order 121: the selection's fixed end (the caret is the moving end) and the field it is in
    anchor: usize,
    sel_key: Option<Key>,
    /// the last press on a note field (a second one soon after = double-click: the word)
    last_press: Option<(Key, f64)>,
    /// the word a double-click selected: a pointer that stays inside it keeps the word selected (no shrinking by jitter)
    word_sel: Option<(usize, usize)>,
    /// a tab being renamed: its index, the name so far, the caret
    rename: Option<(usize, String, usize)>,
    /// the last click on a tab (double-click = rename)
    last_click: Option<(Key, f64)>,
    menu: Option<Menu>,
    /// the file could not be read: nothing is saved (and it was said once)
    readonly: bool,
    told: bool,
    /// each note's drawn lines for its text (a hash of the text): measured once, not at every caret blink
    lines: std::collections::HashMap<u64, (u64, Vec<(usize, usize)>)>,
}

impl Notes {
    /// Every change: written to notes.cfg by the background writer (and kept for the next open in this app run).
    fn save(&mut self) {
        let (Some(n), Some(f)) = (&self.n, &self.folder) else { return };
        if self.readonly {
            return;
        }
        let text = n.to_text();
        if let Ok(mut c) = CACHE.lock() {
            *c = Some((f.clone(), n.clone(), false));
        }
        if let Ok(mut s) = SAVER.lock() {
            if s.as_ref().is_none_or(|s| &s.0 != f) {
                *s = store::Saver::start(f.clone()).map(|sv| (f.clone(), sv));
            }
            if let Some((_, sv)) = s.as_ref() {
                sv.send(text);
            }
        }
    }

    fn take_cached(&mut self) {
        if self.n.is_none() {
            if let Some((n, ro)) = self.folder.as_ref().and_then(cached) {
                self.n = Some(n);
                self.readonly = ro;
            }
        }
    }

    fn book_index(&self, id: u64) -> Option<usize> {
        self.n.as_ref()?.books.iter().position(|b| b.id == id)
    }

    /// The note (id) a key belongs to (its card, title or text), and which part: 0 card, 1 title, 2 text.
    fn note_of(&self, k: Key) -> Option<(u64, u8)> {
        let n = self.n.as_ref()?;
        let b = n.books.get(n.cur)?;
        b.notes.iter().find_map(|x| {
            let i = x.id as usize;
            if k == idx(K_CARD, i) {
                Some((x.id, 0))
            } else if k == idx(K_TITLE, i) {
                Some((x.id, 1))
            } else if k == idx(K_TEXT, i) {
                Some((x.id, 2))
            } else {
                None
            }
        })
    }

    fn blurred(&self, id: u64) -> bool {
        self.eye.on() && !self.revealed.contains(&id)
    }

    fn commit_rename(&mut self) {
        if let (Some((i, name, _)), Some(n)) = (self.rename.take(), self.n.as_mut()) {
            n.rename_book(i, &name);
            self.save();
        }
    }

    /// The chosen letters (lo, hi) of the field `k`, while it holds the selection.
    fn sel_of(&self, k: Key) -> Option<(usize, usize)> {
        (self.sel_key == Some(k) && self.anchor != self.caret).then(|| (self.anchor.min(self.caret), self.anchor.max(self.caret)))
    }

    /// The caret of field `k` goes to `at`; `extend` keeps the selection's fixed end (Shift, a drag), else the selection goes.
    fn set_caret(&mut self, k: Key, at: usize, extend: bool, now: f64) {
        if !(extend && self.sel_key == Some(k)) {
            self.anchor = at;
        }
        self.sel_key = Some(k);
        self.caret = at;
        self.caret_at = now;
    }

    /// The words of a note's title (part 1) or text (part 2).
    fn field(&self, id: u64, part: u8) -> Option<String> {
        let n = self.n.as_ref()?;
        let x = n.books.get(n.cur)?.notes.iter().find(|x| x.id == id)?;
        Some(if part == 1 { x.title.clone() } else { x.text.clone() })
    }

    /// Take the letters lo..hi out of a field; the caret goes there (a cut, Delete / Backspace on a selection).
    fn cut(&mut self, k: Key, lo: usize, hi: usize, now: f64) {
        let Some((id, part)) = self.note_of(k) else { return };
        let mut at = lo;
        let Some(note) = self.n.as_mut().and_then(|n| n.note_mut(id)) else { return };
        edit::remove(if part == 1 { &mut note.title } else { &mut note.text }, &mut at, lo, hi);
        self.set_caret(k, at, false, now);
        self.save();
    }

    /// Typed text for the focused field `k`.
    fn type_in(&mut self, k: Key, s: &str, cx: &mut Cx) {
        if k == K_REN {
            if let Some((_, name, at)) = self.rename.as_mut() {
                let room = NAME_MAX.saturating_sub(name.chars().count());
                let s: String = s.chars().filter(|c| !c.is_control()).take(room).collect();
                edit::insert(name, at, &s);
            }
            return;
        }
        let Some((id, part)) = self.note_of(k) else { return };
        // typing over a selection replaces it
        let sel = self.sel_of(k);
        let mut at = self.caret;
        let Some(note) = self.n.as_mut().and_then(|n| n.note_mut(id)) else { return };
        match part {
            1 => {
                if let Some((lo, hi)) = sel {
                    edit::remove(&mut note.title, &mut at, lo, hi);
                }
                let room = TITLE_MAX.saturating_sub(note.title.chars().count());
                let s: String = s.chars().map(|c| if c == '\n' || c == '\t' { ' ' } else { c }).filter(|c| !c.is_control()).take(room).collect();
                edit::insert(&mut note.title, &mut at, &s);
            }
            2 => {
                if let Some((lo, hi)) = sel {
                    edit::remove(&mut note.text, &mut at, lo, hi);
                }
                let room = TEXT_MAX.saturating_sub(note.text.chars().count());
                let s: String = s.chars().map(|c| if c == '\t' { ' ' } else { c }).filter(|c| *c == '\n' || !c.is_control()).take(room).collect();
                edit::insert(&mut note.text, &mut at, &s);
            }
            _ => return,
        }
        self.set_caret(k, at, false, cx.now);
        self.revealed.insert(id);
        self.save();
    }

    /// Where a pointer at (x, y) puts the caret in a note's title / text, whose box is `r`.
    fn pos_at(&self, g: &crate::gfx::Gfx, id: u64, part: u8, x: f32, y: f32, r: (f32, f32, f32, f32)) -> usize {
        let Some(t) = self.field(id, part) else { return 0 };
        let c: Vec<char> = t.chars().collect();
        if part == 1 {
            edit::at_x(&c, &[(0, c.len())], 0, x - r.0, &|s: &str| g.text_width(s, F_TITLE))
        } else {
            let w = |s: &str| g.text_width(s, F_TEXT);
            let ls = edit::lines(&c, TEXT_W, &w);
            let line = (((y - r.1) / LH_TEXT).floor().max(0.0) as usize).min(ls.len() - 1);
            edit::at_x(&c, &ls, line, x - r.0, &w)
        }
    }

    /// A key in a note's title / text: select all, copy, cut, the arrows (with Shift: extend the selection; with Ctrl: by word),
    /// Backspace / Delete (a selection goes first), the rest as `edit::key`.
    fn note_key(&mut self, k: Key, vk: u16, cx: &mut Cx) {
        let Some((id, part)) = self.note_of(k) else { return };
        let (ctrl, shift, now, g) = (cx.mods.ctrl, cx.mods.shift, cx.now, cx.g);
        let Some(text) = self.field(id, part) else { return };
        let all: Vec<char> = text.chars().collect();
        let len = all.len();
        let at = self.caret.min(len);
        let sel = self.sel_of(k).map(|(a, b)| (a.min(len), b.min(len))).filter(|(a, b)| a < b);
        let (font, maxw) = if part == 1 { (F_TITLE, f32::MAX) } else { (F_TEXT, TEXT_W) };
        let w = |s: &str| g.text_width(s, font);
        match vk {
            VK_A if ctrl => {
                self.anchor = 0;
                self.sel_key = Some(k);
                self.caret = len;
                self.caret_at = now;
            }
            VK_C | VK_X if ctrl => {
                if let Some((lo, hi)) = sel {
                    let _ = cx.copy_text(&edit::slice(&text, lo, hi));
                    if vk == VK_X {
                        self.cut(k, lo, hi, now);
                    }
                }
            }
            edit::VK_BACK | edit::VK_DELETE if sel.is_some() => {
                let (lo, hi) = sel.unwrap_or((0, 0));
                self.cut(k, lo, hi, now);
            }
            edit::VK_BACK if ctrl && at > 0 => self.cut(k, edit::word_left(&all, at), at, now),
            edit::VK_DELETE if ctrl && at < len => self.cut(k, at, edit::word_right(&all, at), now),
            edit::VK_LEFT | edit::VK_RIGHT | edit::VK_UP | edit::VK_DOWN | edit::VK_HOME | edit::VK_END => {
                let to = match (vk, sel) {
                    (edit::VK_LEFT, Some((lo, _))) if !shift && !ctrl => lo,
                    (edit::VK_RIGHT, Some((_, hi))) if !shift && !ctrl => hi,
                    (edit::VK_LEFT, _) if ctrl => edit::word_left(&all, at),
                    (edit::VK_RIGHT, _) if ctrl => edit::word_right(&all, at),
                    (edit::VK_HOME, _) if ctrl => 0,
                    (edit::VK_END, _) if ctrl => len,
                    _ => {
                        let (mut t, mut a) = (text.clone(), at);
                        edit::key(&mut t, &mut a, vk, maxw, &w);
                        a
                    }
                };
                self.set_caret(k, to, shift, now);
            }
            _ => {
                let mut at = at;
                let changed = match self.n.as_mut().and_then(|n| n.note_mut(id)) {
                    Some(note) if part == 1 => edit::key(&mut note.title, &mut at, vk, maxw, &w),
                    Some(note) => edit::key(&mut note.text, &mut at, vk, maxw, &w),
                    None => false,
                };
                // (a key that neither moved the caret nor changed the text - Ctrl, Shift, a letter before its character - keeps the selection)
                if changed || at != self.caret {
                    self.set_caret(k, at, false, now);
                }
                if changed {
                    self.save();
                }
            }
        }
    }

    /// A note's text lines (kept while its text stays the same).
    fn lines_of(&mut self, g: &crate::gfx::Gfx, id: u64, text: &str) -> Vec<(usize, usize)> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        let sig = h.finish();
        if let Some((s, l)) = self.lines.get(&id) {
            if *s == sig {
                return l.clone();
            }
        }
        let chars: Vec<char> = text.chars().collect();
        let l = edit::lines(&chars, TEXT_W, &|s: &str| g.text_width(s, F_TEXT));
        self.lines.insert(id, (sig, l.clone()));
        l
    }

    // ---------------------------------------------------------------- the boxes
    fn tab_chip(&self, cx: &mut Cx, i: usize, id: u64, name: &str, on: bool) -> El {
        // `.apc{display:inline-flex;align-items:center;height:26px;padding:0 10px;border-radius:13px;background:var(--ctl);
        //   box-shadow:inset 0 0 0 .5px var(--hair);font-size:12px}` `:hover{background:var(--ctl-h)}` `:active{transform:
        //   scale(.97)}` `.on{background:var(--sel);box-shadow:inset 0 0 0 1.5px var(--acc);font-weight:600}`
        let k = idx(K_TAB, id as usize);
        let hv = cx.hover_t(k, 150.0, EASE);
        let pr = cx.active_t(k, 120.0, EASE);
        let c = El::row()
            .center()
            .h(26.0)
            .none()
            .pad(0.0, 10.0, 0.0, 10.0)
            .radius(13.0)
            .bg(if on { SEL() } else { cmix(CTL(), CTL_H(), hv) })
            .inset(&[if on { sh(0.0, 0.0, 0.0, 1.5, ACC()) } else { sh(0.0, 0.0, 0.0, 0.5, HAIR()) }])
            .scale(1.0 - 0.03 * pr)
            .on_click(k)
            .cursor(Cursor::Hand);
        let w = if on { 600 } else { 400 };
        match &self.rename {
            Some((ri, t, at)) if *ri == i => {
                // the name being typed: `.apin{height:20px;margin:0 -4px;padding:0 4px;border-radius:4px;background:var(--well);
                // font-weight:600}` with the caret
                let caret = cx.focused(K_REN) && ((cx.now - self.caret_at) % 1060.0) < 530.0;
                c.child(line_with_caret(t, *at, Font::new(12.0, 600), FG(), 20.0, caret).h(20.0).margin(0.0, -4.0, 0.0, -4.0).pad(0.0, 4.0, 0.0, 4.0).radius(4.0).bg(WELL()).key(K_REN).cursor(Cursor::Text))
            }
            _ => c.child(El::text(name, Font::new(12.0, w), FG(), lh(12.0, 1.35)).none().no_hit()),
        }
    }

    fn add_tab_chip(&self, cx: &mut Cx) -> El {
        let hv = cx.hover_t(K_ADDTAB, 150.0, EASE);
        let pr = cx.active_t(K_ADDTAB, 120.0, EASE);
        El::block()
            .size(26.0, 26.0)
            .none()
            .radius(RADIUS_PILL)
            .bg(cmix(CTL(), CTL_H(), hv))
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
            .scale(1.0 - 0.03 * pr)
            .place_center()
            .on_click(K_ADDTAB)
            .cursor(Cursor::Hand)
            .title("New tab")
            .child(El::icon("dplus", 14.0, 1.6, ICO_ON()).no_hit())
    }

    fn note_card(&mut self, cx: &mut Cx, note: &store::Note) -> El {
        let i = note.id as usize;
        let (kt, kx) = (idx(K_TITLE, i), idx(K_TEXT, i));
        let blink = ((cx.now - self.caret_at) % 1060.0) < 530.0;
        // the title (one line; "Title" while empty): drawn as one painted line while it has the focus (selection + caret)
        let tf = cx.focused(kt);
        let title = if tf {
            let c: Vec<char> = note.title.chars().collect();
            let sel = self.sel_of(kt).map(|(a, b)| (a.min(c.len()), b.min(c.len())));
            let caret = blink.then_some(self.caret.min(c.len()));
            let ls = vec![(0, c.len())];
            El::paint(move |g, (x, y, _, _)| draw_field(g, F_TITLE, LH_TITLE, x, y, &c, &ls, caret, sel))
                .sig((&note.title, caret, sel, tf))
                .h(LH_TITLE)
                .no_hit()
                .clip()
        } else if note.title.is_empty() {
            El::text("Title", F_TITLE, INK.mul_a(0.45), LH_TITLE).no_hit()
        } else {
            El::text(note.title.clone(), F_TITLE, INK, LH_TITLE).ellipsis().no_hit()
        };
        let title = El::block().h(LH_TITLE).child(title).key(kt).cursor(Cursor::Text);
        // the text (wraps; "Type here" while empty)
        let xf = cx.focused(kx);
        let chars: Vec<char> = note.text.chars().collect();
        let ls = self.lines_of(cx.g, note.id, &note.text);
        let h = ls.len().max(MIN_LINES) as f32 * LH_TEXT;
        let caret = (xf && blink).then_some(self.caret.min(chars.len()));
        let sel = if xf { self.sel_of(kx).map(|(a, b)| (a.min(chars.len()), b.min(chars.len()))) } else { None };
        let empty = chars.is_empty();
        // Order 121 (A_121_01): a blurred note draws FAKE letters (the real ones never reach the screen), the same lines and
        // spaces, fixed for the note until its text changes - blurred less than before, so it still reads as text
        let hidden = self.blurred(note.id) && !xf;
        let mut shown = chars.clone();
        if hidden {
            shown = edit::fake(&chars, fake_seed(note.id, &note.text));
            edit::fit(&mut shown, &ls, TEXT_W - 1.0, &|s: &str| cx.g.text_width(s, F_TEXT));
        }
        let shown_s: String = if hidden { shown.iter().collect() } else { note.text.clone() };
        let (c2, l2) = (shown, ls.clone());
        let paint = El::paint(move |g, (x, y, _, _)| {
            if empty && caret.is_none() && !xf {
                g.text("Type here", F_TEXT, x, y, LH_TEXT, INK.mul_a(0.45), Align::Left, 0.0);
            }
            draw_field(g, F_TEXT, LH_TEXT, x, y, &c2, &l2, caret, sel);
        })
        .sig((&shown_s, caret, sel, xf, TEXT_W.to_bits()))
        .h(h)
        .no_hit();
        let mut text = El::block().h(h).margin(6.0, 0.0, 0.0, 0.0).child(paint).key(kx).cursor(Cursor::Text);
        // the eye: the TEXT blurs, the title stays readable; a click shows the note
        if hidden {
            text = text.blur(NOTE_BLUR).cursor(Cursor::Hand);
        }
        // Order 119, option A: square pastel paper (`border-radius:4px; box-shadow:0 6px 14px rgba(0,0,0,.45)`), tilted
        El::block()
            .bg(PAPER[(note.colour % store::COLOURS) as usize])
            .radius(4.0)
            .shadow(&[sh(0.0, 6.0, 14.0, 0.0, Rgba::rgba(0, 0, 0, 0.45))])
            .min_h(COL_W)
            .pad(PAD.0, PAD.1, PAD.0, PAD.1)
            .rotate(tilt(note.id))
            .children(vec![title, text])
            .key(idx(K_CARD, i))
    }

    fn add_note_tile(&self, cx: &mut Cx) -> El {
        // the "+ Note" cell (option A): a dashed square, the add button's look inside (`.addb`: --ico-on, 12.5 px / 600, the
        // 14 px plus)
        let hv = cx.hover_t(K_ADD, 120.0, EASE);
        let pr = cx.active_t(K_ADD, 120.0, EASE);
        let dash = DASH();
        let outline = El::paint(move |g, (x, y, w, h)| {
            // 1 px dashes (5 on, 4 off) along the four sides, inset half a pixel
            let (l, t, r, b) = (x + 0.5, y + 0.5, x + w - 0.5, y + h - 0.5);
            let side = |x1: f32, y1: f32, x2: f32, y2: f32| {
                let len = (x2 - x1).abs() + (y2 - y1).abs();
                let (dx, dy) = ((x2 - x1) / len, (y2 - y1) / len);
                let mut s = 0.0;
                while s < len {
                    let e = (s + 5.0).min(len);
                    g.line(x1 + dx * s, y1 + dy * s, x1 + dx * e, y1 + dy * e, 1.0, dash, false);
                    s += 9.0;
                }
            };
            side(l, t, r, t);
            side(r, t, r, b);
            side(l, b, r, b);
            side(l, t, l, b);
        })
        .sig((dash.0.to_bits(), dash.1.to_bits(), dash.2.to_bits(), dash.3.to_bits()))
        .abs(0.0, 0.0, 0.0, 0.0)
        .no_hit();
        El::row()
            .center()
            .justify(taffy::style::JustifyContent::CENTER)
            .gap(6.0)
            .min_h(COL_W)
            .radius(4.0)
            .bg(HOV().mul_a(hv))
            .scale(1.0 - 0.015 * pr)
            .on_click(K_ADD)
            .cursor(Cursor::Hand)
            .child(outline)
            .child(El::icon("dplus", 14.0, 1.6, ICO_ON()).no_hit())
            .child(El::text("Note", pieces::btn_font(12.5, 600), ICO_ON(), lh(12.5, 1.35)).none().no_hit())
    }
}

/// One field's drawing at (x, y): its lines in the ink, the selection as a blue box with the chosen letters white (as the
/// app's own text boxes), the caret as a 1 px bar. `ls` = the drawn lines (first char, chars).
#[allow(clippy::too_many_arguments)]
fn draw_field(g: &crate::gfx::Gfx, f: Font, lh_: f32, x: f32, y: f32, all: &[char], ls: &[(usize, usize)], caret: Option<usize>, sel: Option<(usize, usize)>) {
    let part = |a: usize, b: usize| all[a..b].iter().collect::<String>();
    for (k, (s, n)) in ls.iter().enumerate() {
        let (ly, e) = (y + k as f32 * lh_, s + n);
        let (lo, hi) = sel.map_or((0, 0), |(a, b)| (a.max(*s), b.min(e)));
        if lo < hi {
            let (x0, x1) = (g.text_width(&part(*s, lo), f), g.text_width(&part(*s, hi), f));
            g.fill_rect(x + x0, ly, x1 - x0, lh_, SEL_BG);
            for (a, b, c, dx) in [(*s, lo, INK, 0.0), (lo, hi, SEL_FG, x0), (hi, e, INK, x1)] {
                if a < b {
                    g.text(&part(a, b), f, x + dx, ly, lh_, c, Align::Left, 0.0);
                }
            }
        } else if e > *s {
            g.text(&part(*s, e), f, x, ly, lh_, INK, Align::Left, 0.0);
        }
    }
    if let Some(at) = caret {
        let (k, cxp) = edit::caret_at(all, ls, at, &|s: &str| g.text_width(s, f));
        g.fill_rect(x + cxp, y + k as f32 * lh_ + 2.0, 1.0, lh_ - 4.0, INK);
    }
}

/// One line of text with the caret at char `at` (a 1 px bar in the text colour while `on`): the text before it, the bar,
/// the text after it.
fn line_with_caret(t: &str, at: usize, f: Font, c: crate::gfx::Rgba, lh_: f32, on: bool) -> El {
    let ch: Vec<char> = t.chars().collect();
    let at = at.min(ch.len());
    let (a, b): (String, String) = (ch[..at].iter().collect(), ch[at..].iter().collect());
    let bar = El::block().size(1.0, lh_ - 4.0).none().bg(if on { c } else { c.mul_a(0.0) });
    El::row().center().h(lh_).child(El::text(a, f, c, lh_).none()).child(bar).child(El::text(b, f, c, lh_).none()).no_hit()
}

impl Page for Notes {
    fn id(&self) -> &'static str {
        "nts"
    }
    fn name(&self) -> &'static str {
        "Notes"
    }
    fn icon(&self) -> &'static str {
        "stick"
    }

    fn open(&mut self, env: &Env, _now: f64) {
        self.folder = crate::services::try_with(|s| s.store.folder().to_path_buf());
        let Some(f) = self.folder.clone() else {
            // a unit test: in memory only (a test may have filled it already)
            if self.n.is_none() {
                self.n = Some(store::Notes::new());
            }
            return;
        };
        // (a file that could not be read last time is read again)
        if cached(&f).is_some_and(|c| !c.1) {
            self.take_cached();
        }
        if self.n.is_none() {
            // the first open of this app run: the file is read off the menu's thread (the frame holds the tab meanwhile)
            if let Ok(mut c) = CACHE.lock() {
                if c.as_ref().is_some_and(|c| c.0 == f && c.2) {
                    *c = None;
                }
            }
            let w = env.waker();
            std::thread::spawn(move || {
                let (n, ro) = match store::load(&f) {
                    Some(n) => (n, false),
                    None => (store::Notes::new(), true),
                };
                if let Ok(mut c) = CACHE.lock() {
                    if c.as_ref().is_none_or(|c| c.0 != f) {
                        *c = Some((f, n, ro));
                    }
                }
                w.wake();
            });
        }
    }

    fn close(&mut self) {
        self.commit_rename();
        self.n = None;
        self.readonly = false;
        self.told = false;
        self.lines.clear();
        self.eye.reset();
        self.revealed.clear();
        self.menu = None;
        self.anchor = 0;
        self.sel_key = None;
        self.last_press = None;
        self.word_sel = None;
        self.last_click = None;
    }

    fn ready(&self) -> bool {
        self.n.is_some() || self.folder.as_ref().is_some_and(|f| cached(f).is_some())
    }

    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        self.take_cached();
        self.eye.load(cx);
        let mut out = vec![pieces::header(self.name(), Some(self.eye.button(cx, K_EYE)))];
        let Some(n) = self.n.clone() else { return out };
        if self.readonly && !std::mem::replace(&mut self.told, true) {
            cx.toast("Your notes file is in use by another program · nothing is saved until it can be read");
        }
        // the caret blinks (530 ms on / off): built again at each flip, nothing between
        if cx.st.focus.is_some_and(|k| (k == K_REN && self.rename.is_some()) || self.note_of(k).is_some_and(|x| x.1 > 0)) {
            cx.wake_every(530.0, self.caret_at);
        }
        // the tabs
        let mut row = El::row().center().wrap().gap(6.0).margin(0.0, 0.0, 12.0, 0.0);
        for (i, b) in n.books.iter().enumerate() {
            row = row.child(self.tab_chip(cx, i, b.id, &b.name, i == n.cur));
        }
        out.push(row.child(self.add_tab_chip(cx)));
        // the shown tab's notes + "+ Note"
        let mut grid = El::grid().cols(3).gap(GAP);
        if let Some(b) = n.books.get(n.cur) {
            for note in &b.notes {
                grid = grid.child(self.note_card(cx, note));
            }
        }
        out.push(grid.child(self.add_note_tile(cx)));
        out
    }

    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        let now = cx.now;
        if self.eye.event(ev, cx, K_EYE) {
            return;
        }
        cx.dirty = true;
        let Some(n) = self.n.as_ref() else { return };
        let cur = n.cur;
        // each tab's chip key and whether it has notes
        let tabs: Vec<(Key, bool)> = n.books.iter().map(|b| (idx(K_TAB, b.id as usize), !b.notes.is_empty())).collect();
        let tab_of = |k: &Key| tabs.iter().position(|t| t.0 == *k);
        // a click anywhere but the name being typed keeps the new name
        if let Ev::Press(k, ..) = ev {
            if *k != K_REN && self.rename.is_some() {
                self.commit_rename();
            }
        }
        match ev {
            // ---- the tabs
            Ev::Click(k) if *k == K_ADDTAB => {
                if let Some(n) = self.n.as_mut() {
                    n.add_book();
                }
                self.save();
            }
            Ev::Click(k) if tab_of(k).is_some() => {
                let i = tab_of(k).unwrap_or(0);
                let dbl = self.last_click.is_some_and(|(lk, t)| lk == *k && now - t < DBL_MS);
                if dbl && i == cur {
                    let name = self.n.as_ref().map(|n| n.books[i].name.clone()).unwrap_or_default();
                    let len = name.chars().count();
                    self.rename = Some((i, name, len));
                    self.caret_at = now;
                    self.last_click = None;
                    cx.focus(Some(K_REN));
                } else {
                    self.last_click = Some((*k, now));
                    if i != cur {
                        if let Some(n) = self.n.as_mut() {
                            n.cur = i;
                        }
                        self.save();
                    }
                }
            }
            Ev::Context(k, x, y) if tab_of(k).is_some() => {
                let i = tab_of(k).unwrap_or(0);
                // a name being typed is kept (a right-click sends no press)
                if self.rename.is_some() {
                    self.commit_rename();
                    cx.focus(None);
                }
                if !tabs[i].1 {
                    self.rename = None;
                    if let Some(n) = self.n.as_mut() {
                        n.delete_book(i);
                    }
                    self.save();
                } else {
                    self.menu = Some(Menu::Tab(i, *x, *y));
                    cx.focus(Some(sub(K_CONF, "no")));
                }
            }
            Ev::Click(k) if *k == sub(K_CONF, "go") => {
                if let Some(Menu::Tab(i, ..)) = self.menu.take() {
                    self.rename = None;
                    if let Some(n) = self.n.as_mut() {
                        n.delete_book(i);
                    }
                    self.save();
                }
            }
            Ev::Click(k) if *k == sub(K_CONF, "no") => self.menu = None,
            // ---- the notes
            Ev::Click(k) if *k == K_ADD => {
                let id = self.n.as_mut().map(|n| n.add_note());
                if let Some(id) = id {
                    self.revealed.insert(id);
                    self.set_caret(idx(K_TITLE, id as usize), 0, false, now);
                    cx.focus(Some(idx(K_TITLE, id as usize)));
                    cx.scroll_to(idx(K_CARD, id as usize));
                }
                self.save();
            }
            Ev::Context(k, x, y) if self.note_of(*k).is_some() => {
                let (id, part) = self.note_of(*k).unwrap_or((0, 0));
                let empty = self.n.as_ref().and_then(|n| n.books.get(cur)?.notes.iter().find(|x| x.id == id).map(|x| x.title.trim().is_empty() && x.text.trim().is_empty())).unwrap_or(false);
                if empty {
                    if let Some(n) = self.n.as_mut() {
                        n.delete_note(id);
                    }
                    self.save();
                } else {
                    // the field right-clicked (the paper = the text) and its selection / caret now
                    let part = if part == 1 { 1 } else { 2 };
                    let fk = idx(if part == 1 { K_TITLE } else { K_TEXT }, id as usize);
                    let len = self.field(id, part).map_or(0, |t| t.chars().count());
                    let (lo, hi) = if self.sel_key == Some(fk) { (self.anchor.min(self.caret).min(len), self.anchor.max(self.caret).min(len)) } else { (len, len) };
                    self.menu = Some(Menu::Note(id, *x, *y, part, lo, hi));
                }
            }
            // Order 119: a colour in the note's menu: the paper takes it at once (saved per note)
            Ev::Click(k) if (0..PAPER.len()).any(|j| *k == idx(sub(K_MENU, "sw"), j)) => {
                let j = (0..PAPER.len()).position(|j| *k == idx(sub(K_MENU, "sw"), j)).unwrap_or(0) as u8;
                if let Some(Menu::Note(id, ..)) = self.menu.take() {
                    if let Some(note) = self.n.as_mut().and_then(|n| n.note_mut(id)) {
                        note.colour = j;
                    }
                    self.save();
                }
            }
            Ev::Click(k) if *k == idx(K_MENU, MENU_COPY) && matches!(self.menu, Some(Menu::Note(..))) => {
                if let Some(Menu::Note(id, _, _, part, lo, hi)) = self.menu.take() {
                    if let Some(t) = self.field(id, part) {
                        // the selection, else the whole field (the title, or the text)
                        let s = if lo < hi { edit::slice(&t, lo, hi) } else { t };
                        let _ = cx.copy_text(&s);
                    }
                }
            }
            Ev::Click(k) if *k == idx(K_MENU, MENU_PASTE) && matches!(self.menu, Some(Menu::Note(..))) => {
                if let Some(Menu::Note(id, _, _, part, lo, hi)) = self.menu.take() {
                    if let Some(t) = crate::clipboard::get_text() {
                        // into the field right-clicked, over its selection / at its caret, which has the focus again
                        let fk = idx(if part == 1 { K_TITLE } else { K_TEXT }, id as usize);
                        self.anchor = lo;
                        self.caret = hi;
                        self.sel_key = Some(fk);
                        cx.focus(Some(fk));
                        self.type_in(fk, &t, cx);
                    }
                }
            }
            Ev::Click(k) if *k == idx(K_MENU, MENU_DELETE) && matches!(self.menu, Some(Menu::Note(..))) => {
                if let Some(Menu::Note(id, ..)) = self.menu.take() {
                    if let Some(n) = self.n.as_mut() {
                        n.delete_note(id);
                    }
                    self.save();
                }
            }
            // a press on a note: a blurred one shows; on its title / text the caret goes where it was clicked
            Ev::Press(k, x, y, r) if self.note_of(*k).is_some() => {
                let (id, part) = self.note_of(*k).unwrap_or((0, 0));
                self.revealed.insert(id);
                if part == 0 {
                    return;
                }
                // the caret goes where it was clicked; Shift extends the selection; a second press soon after selects the word
                let pos = self.pos_at(cx.g, id, part, *x, *y, *r);
                let dbl = self.last_press.is_some_and(|(lk, t)| lk == *k && now - t < DBL_MS);
                if dbl {
                    let c: Vec<char> = self.field(id, part).unwrap_or_default().chars().collect();
                    let (a, b) = edit::word_at(&c, pos);
                    self.anchor = a;
                    self.sel_key = Some(*k);
                    self.caret = b;
                    self.caret_at = now;
                    self.last_press = None;
                    self.word_sel = Some((a, b));
                } else {
                    self.word_sel = None;
                    self.set_caret(*k, pos, cx.mods.shift, now);
                    self.last_press = Some((*k, now));
                }
            }
            // dragging on a title / text: the caret follows the pointer, the selection grows from where it went down
            Ev::Drag(k, x, y, r) if self.note_of(*k).is_some_and(|x| x.1 > 0) => {
                let (id, part) = self.note_of(*k).unwrap_or((0, 0));
                let pos = self.pos_at(cx.g, id, part, *x, *y, *r);
                if let Some((a, b)) = self.word_sel {
                    if (a..=b).contains(&pos) {
                        return;
                    }
                    self.word_sel = None;
                    if pos < a {
                        self.anchor = b;
                    }
                }
                self.set_caret(*k, pos, true, now);
            }
            Ev::Press(k, x, _, r) if *k == K_REN => {
                if let Some((_, name, at)) = self.rename.as_mut() {
                    let c: Vec<char> = name.chars().collect();
                    *at = edit::at_x(&c, &[(0, c.len())], 0, x - r.0 - 4.0, &|s: &str| cx.g.text_width(s, Font::new(12.0, 600)));
                    self.caret_at = now;
                }
            }
            // ---- typing
            Ev::Char(k, c) => {
                let k = *k;
                match (*c, k == K_REN, self.note_of(k).map(|x| x.1)) {
                    // Enter: the new name is kept / the title goes on in the text / a line break
                    ('\r', true, _) => {
                        self.commit_rename();
                        cx.focus(None);
                    }
                    ('\r', _, Some(1)) => {
                        let (id, _) = self.note_of(k).unwrap_or((0, 0));
                        self.set_caret(idx(K_TEXT, id as usize), 0, false, now);
                        cx.focus(Some(idx(K_TEXT, id as usize)));
                    }
                    ('\r', _, Some(2)) => self.type_in(k, "\n", cx),
                    (c, _, _) if !c.is_control() => self.type_in(k, &c.to_string(), cx),
                    _ => {}
                }
            }
            Ev::Key(k, VK_ESCAPE) if *k == K_REN => self.rename = None,
            Ev::Key(k, VK_V) if cx.mods.ctrl && (*k == K_REN || self.note_of(*k).is_some_and(|x| x.1 > 0)) => {
                if let Some(t) = crate::clipboard::get_text() {
                    self.type_in(*k, &t, cx);
                }
            }
            Ev::Key(k, vk) if *k == K_REN => {
                if let Some((_, name, at)) = self.rename.as_mut() {
                    edit::key(name, at, *vk, f32::MAX, &|s: &str| cx.g.text_width(s, Font::new(12.0, 600)));
                    self.caret_at = now;
                }
            }
            Ev::Key(k, vk) if self.note_of(*k).is_some_and(|x| x.1 > 0) => self.note_key(*k, *vk, cx),
            // a field lost the focus: its selection goes (the caret stays for a Paste from the menu)
            Ev::Blur(k) if self.note_of(*k).is_some_and(|x| x.1 > 0) => self.anchor = self.caret,
            Ev::Blur(k) if *k == K_REN => self.commit_rename(),
            _ => {}
        }
    }

    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        if let Some(m) = self.eye.popup(cx, K_EYE) {
            return Some(m);
        }
        let n = self.n.as_ref()?;
        match self.menu.clone()? {
            Menu::Tab(i, x, y) => {
                let b = n.books.get(i)?;
                let k = b.notes.len();
                let text = if k == 1 { "Its note goes too.".to_string() } else { format!("Its {k} notes go too.") };
                Some(mitems::confirm(cx, K_CONF, &format!("Delete {}?", b.name), &text, "Cancel", "Delete", button::Kind::Red, Place::At(x, y), 260.0))
            }
            Menu::Note(id, x, y, ..) => {
                // Order 119: the four paper colours (the note's own marked), then (Order 121) Copy and Paste, then Delete
                let on = n.books.get(n.cur)?.notes.iter().find(|q| q.id == id)?.colour as usize;
                let rows = [Row::Swatches(&PAPER, on), Row::Sep, Row::Item(It::icon("copy", "Copy")), Row::Item(It::icon("paste", "Paste")), Row::Sep, Row::Item(It::icon("trash", "Delete note").danger())];
                Some(mitems::menu(cx, K_MENU, &rows, Place::At(x, y), 150.0))
            }
        }
    }

    fn popup_dismiss(&mut self) {
        self.eye.dismiss();
        self.menu = None;
    }

    fn describe(&self) -> String {
        match &self.n {
            Some(n) => format!(
                "tabs={} cur={} notes={} blur={} always={} revealed={} rename={}",
                n.books.len(),
                n.cur,
                n.books.get(n.cur).map_or(0, |b| b.notes.len()),
                self.eye.on(),
                self.eye.always(),
                self.revealed.len(),
                self.rename.is_some()
            ),
            None => "loading".to_string(),
        }
    }
}

#[cfg(test)]
mod tests;
