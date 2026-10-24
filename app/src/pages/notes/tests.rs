//! Order 116: the Notes tab on its own (no services: the notes live in memory; the file is store.rs's tests).

use super::*;
use crate::gfx::Gfx;
use crate::ui::cx::State;
use std::sync::Mutex;

fn page() -> (Notes, Gfx, State) {
    let mut p = Notes::default();
    p.open(&Env { test: true, ..Env::default() }, 0.0);
    (p, Gfx::new(1.0), State::default())
}

fn ev(p: &mut Notes, g: &Gfx, st: &mut State, e: Ev, now: f64) {
    let mut cx = Cx::new(now, false, g, st);
    p.event(&e, &mut cx);
}

/// `ev` with the modifier keys held (Ctrl / Shift).
fn evm(p: &mut Notes, g: &Gfx, st: &mut State, e: Ev, now: f64, ctrl: bool, shift: bool) {
    let mut cx = Cx::new(now, false, g, st);
    cx.mods.ctrl = ctrl;
    cx.mods.shift = shift;
    p.event(&e, &mut cx);
}

/// the clipboard is one static for every test: the tests that use it take turns
static CLIP: Mutex<()> = Mutex::new(());

fn build(p: &mut Notes, g: &Gfx, st: &mut State, now: f64) -> Vec<El> {
    let mut cx = Cx::new(now, false, g, st);
    p.build(&mut cx)
}

/// The box with key `k` anywhere in the tree.
fn find(els: &[El], k: Key) -> Option<&El> {
    for e in els {
        if e.key == Some(k) {
            return Some(e);
        }
        if let Some(f) = find(&e.children, k) {
            return Some(f);
        }
    }
    None
}

fn tab_key(p: &Notes, i: usize) -> Key {
    idx(K_TAB, p.n.as_ref().unwrap().books[i].id as usize)
}

fn type_str(p: &mut Notes, g: &Gfx, st: &mut State, k: Key, s: &str) {
    for c in s.chars() {
        ev(p, g, st, Ev::Char(k, c), 1.0);
    }
}

#[test]
fn plus_note_adds_a_note_typed_straight_in() {
    let (mut p, g, mut st) = page();
    ev(&mut p, &g, &mut st, Ev::Click(K_ADD), 1.0);
    let id = p.n.as_ref().unwrap().books[0].notes[0].id;
    let kt = idx(K_TITLE, id as usize);
    assert_eq!(st.focus, Some(kt), "the new note's title takes the keys");
    type_str(&mut p, &g, &mut st, kt, "Shopping");
    ev(&mut p, &g, &mut st, Ev::Char(kt, '\r'), 1.0);
    let kx = idx(K_TEXT, id as usize);
    assert_eq!(st.focus, Some(kx), "Enter in the title goes on in the text");
    type_str(&mut p, &g, &mut st, kx, "milk\reggs");
    ev(&mut p, &g, &mut st, Ev::Key(kx, edit::VK_BACK), 1.0);
    let note = &p.n.as_ref().unwrap().books[0].notes[0];
    assert_eq!((note.title.as_str(), note.text.as_str()), ("Shopping", "milk\negg"));
    assert!(p.describe().starts_with("tabs=1 cur=0 notes=1"), "{}", p.describe());
}

#[test]
fn ctrl_v_pastes_the_clipboards_text() {
    let (mut p, g, mut st) = page();
    ev(&mut p, &g, &mut st, Ev::Click(K_ADD), 1.0);
    let id = p.n.as_ref().unwrap().books[0].notes[0].id;
    let kx = idx(K_TEXT, id as usize);
    st.focus = Some(kx);
    let _l = CLIP.lock().unwrap_or_else(|e| e.into_inner());
    crate::clipboard::set_test_clip("a\nb");
    let mut cx = Cx::new(1.0, false, &g, &mut st);
    cx.mods.ctrl = true;
    p.event(&Ev::Key(kx, VK_V), &mut cx);
    assert_eq!(p.n.as_ref().unwrap().books[0].notes[0].text, "a\nb");
}

#[test]
fn the_eye_blurs_the_text_not_the_title_and_a_click_shows_the_note() {
    let (mut p, g, mut st) = page();
    ev(&mut p, &g, &mut st, Ev::Click(K_ADD), 1.0);
    let id = p.n.as_ref().unwrap().books[0].notes[0].id as usize;
    st.focus = None;
    p.revealed.clear();
    let els = build(&mut p, &g, &mut st, 2.0);
    assert_eq!(find(&els, idx(K_TEXT, id)).unwrap().blur, 0.0, "not blurred while the eye is off");
    ev(&mut p, &g, &mut st, Ev::Click(K_EYE), 3.0);
    let els = build(&mut p, &g, &mut st, 4.0);
    assert_eq!(find(&els, idx(K_TEXT, id)).unwrap().blur, NOTE_BLUR, "the text blurs");
    assert_eq!(find(&els, idx(K_TITLE, id)).unwrap().blur, 0.0, "the title stays readable");
    // a click on the note shows it - until the tab is left
    ev(&mut p, &g, &mut st, Ev::Press(idx(K_CARD, id), 10.0, 10.0, (0.0, 0.0, 100.0, 100.0)), 5.0);
    let els = build(&mut p, &g, &mut st, 6.0);
    assert_eq!(find(&els, idx(K_TEXT, id)).unwrap().blur, 0.0);
    p.close();
    p.open(&Env { test: true, ..Env::default() }, 7.0);
    assert!(!p.eye.on() && p.revealed.is_empty(), "leaving the tab forgets the shown notes (and the eye reads its setting again)");
}

#[test]
fn tabs_add_switch_rename_and_delete() {
    let (mut p, g, mut st) = page();
    ev(&mut p, &g, &mut st, Ev::Click(K_ADDTAB), 1.0);
    assert_eq!(p.n.as_ref().unwrap().cur, 1);
    // a click shows a tab
    let k0 = tab_key(&p, 0);
    ev(&mut p, &g, &mut st, Ev::Click(k0), 2000.0);
    assert_eq!(p.n.as_ref().unwrap().cur, 0);
    // a double-click renames: the name is typed, Enter keeps it
    ev(&mut p, &g, &mut st, Ev::Click(k0), 2200.0);
    assert!(p.rename.is_some() && st.focus == Some(K_REN));
    for _ in 0..5 {
        ev(&mut p, &g, &mut st, Ev::Key(K_REN, edit::VK_BACK), 2300.0);
    }
    type_str(&mut p, &g, &mut st, K_REN, "Work");
    ev(&mut p, &g, &mut st, Ev::Char(K_REN, '\r'), 2400.0);
    assert_eq!(p.n.as_ref().unwrap().books[0].name, "Work");
    // two slow clicks are not a double-click
    ev(&mut p, &g, &mut st, Ev::Click(k0), 5000.0);
    ev(&mut p, &g, &mut st, Ev::Click(k0), 6000.0);
    assert!(p.rename.is_none());
    // Esc while renaming goes back to the old name
    ev(&mut p, &g, &mut st, Ev::Click(k0), 6100.0);
    type_str(&mut p, &g, &mut st, K_REN, "xyz");
    ev(&mut p, &g, &mut st, Ev::Key(K_REN, VK_ESCAPE), 6200.0);
    ev(&mut p, &g, &mut st, Ev::Blur(K_REN), 6200.0);
    assert_eq!(p.n.as_ref().unwrap().books[0].name, "Work");
    // right-click an EMPTY tab: it goes at once
    let k1 = tab_key(&p, 1);
    ev(&mut p, &g, &mut st, Ev::Context(k1, 100.0, 100.0), 7000.0);
    assert_eq!(p.n.as_ref().unwrap().books.len(), 1);
    // right-click a tab WITH notes: a confirm first
    ev(&mut p, &g, &mut st, Ev::Click(K_ADD), 7100.0);
    let k0 = tab_key(&p, 0);
    ev(&mut p, &g, &mut st, Ev::Context(k0, 100.0, 100.0), 7200.0);
    assert!(matches!(p.menu, Some(Menu::Tab(0, ..))));
    {
        let mut cx = Cx::new(7300.0, false, &g, &mut st);
        assert!(p.popup(&mut cx).is_some());
    }
    ev(&mut p, &g, &mut st, Ev::Click(sub(K_CONF, "no")), 7400.0);
    assert_eq!(p.n.as_ref().unwrap().books[0].notes.len(), 1, "Cancel keeps it");
    ev(&mut p, &g, &mut st, Ev::Context(k0, 100.0, 100.0), 7500.0);
    ev(&mut p, &g, &mut st, Ev::Click(sub(K_CONF, "go")), 7600.0);
    let n = p.n.as_ref().unwrap();
    assert_eq!((n.books.len(), n.books[0].name.as_str(), n.books[0].notes.len()), (1, "Notes", 0), "the last tab gone: an empty one");
}

#[test]
fn right_click_a_note_deletes_it_an_empty_one_at_once() {
    let (mut p, g, mut st) = page();
    ev(&mut p, &g, &mut st, Ev::Click(K_ADD), 1.0);
    ev(&mut p, &g, &mut st, Ev::Click(K_ADD), 1.0);
    let ids: Vec<u64> = p.n.as_ref().unwrap().books[0].notes.iter().map(|n| n.id).collect();
    ev(&mut p, &g, &mut st, Ev::Context(idx(K_CARD, ids[0] as usize), 10.0, 10.0), 2.0);
    assert_eq!(p.n.as_ref().unwrap().books[0].notes.len(), 1, "an empty note goes at once");
    type_str(&mut p, &g, &mut st, idx(K_TEXT, ids[1] as usize), "keep?");
    ev(&mut p, &g, &mut st, Ev::Context(idx(K_TEXT, ids[1] as usize), 10.0, 10.0), 3.0);
    assert!(matches!(p.menu, Some(Menu::Note(..))));
    ev(&mut p, &g, &mut st, Ev::Click(idx(K_MENU, MENU_DELETE)), 4.0);
    assert!(p.n.as_ref().unwrap().books[0].notes.is_empty());
}

/// The page as the pictures show it: two tabs ("Notes" shown, "Work"), four notes.
fn demo() -> Notes {
    let mut p = Notes::default();
    let mut n = store::Notes::new();
    for (t, x) in [
        ("Shopping", "Milk, eggs, bread|Coffee beans|Batteries (AA)"),
        ("Valorant settings", "Sens 0.35 at 1600 DPI|Crosshair 0-1-1-2, green"),
        ("Ideas", "A sticky note that blurs when the eye is on, and shows again with one click."),
        ("Call back", "Friday 18:00 about the parts order"),
        ("Todo", "Print the stand"),
    ] {
        let id = n.add_note();
        let note = n.note_mut(id).unwrap();
        note.title = t.into();
        note.text = x.replace('|', "\n");
    }
    n.add_book();
    n.rename_book(1, "Work");
    n.cur = 0;
    p.n = Some(n);
    p
}

const PICS: &str = r"C:\BoylerUtilities-scratch\116";

/// Order 116's pictures (looked at before DONE): Notes plain and blurred (one note clicked open), the top row's menu and
/// rearrange mode with an icon held - at 1920 x 1080 (scale 1.0) and 3440 x 1440 (scale 1.25).
#[test]
#[ignore]
fn pictures_116() {
    for (scale, size) in [(1.0f32, "1920x1080"), (1.25, "3440x1440")] {
        let (pages, tab) = crate::ui::test_pages_with(Box::new(demo()));
        crate::ui::test_picture(pages, tab, scale, &format!(r"{PICS}\116_notes_{size}.png"), |_, _, now| now);
        let mut d = demo();
        d.eye.load_with(true);
        let open = d.n.as_ref().unwrap().books[0].notes[1].id;
        d.revealed.insert(open);
        let (pages, tab) = crate::ui::test_pages_with(Box::new(d));
        crate::ui::test_picture(pages, tab, scale, &format!(r"{PICS}\116_notes_blurred_{size}.png"), |_, _, now| now);
        let (pages, tab) = crate::ui::test_pages_with(Box::new(demo()));
        crate::ui::test_picture(pages, tab, scale, &format!(r"{PICS}\116_tabs_menu_{size}.png"), |ui, _, now| {
            let (x, y) = ui.target_point(&format!("dock:{tab}")).unwrap();
            ui.mouse_move(x, y, now);
            ui.context(x, y, now);
            now + 400.0
        });
        let (pages, tab) = crate::ui::test_pages_with(Box::new(demo()));
        crate::ui::test_picture(pages, tab, scale, &format!(r"{PICS}\116_rearrange_{size}.png"), |ui, g, now| {
            let (x, y) = ui.target_point(&format!("dock:{tab}")).unwrap();
            ui.mouse_move(x, y, now);
            ui.context(x, y, now);
            ui.update(now + 50.0);
            ui.prepare_page(g, now + 50.0);
            // the menu's first row ("Rearrange tabs"): the menu opens 2 px off the mouse, 5 px padding, 26 px rows
            ui.click(&format!("xy:{},{}", x + 40.0, y + 2.0 + 5.0 + 13.0), now + 100.0);
            ui.mouse_down(x, y, now + 200.0);
            ui.mouse_move(x + 50.0, y, now + 300.0);
            ui.mouse_move(x + 80.0, y, now + 400.0);
            now + 900.0
        });
    }
}

/// Order 116: with the app's services (a test copy's scratch settings folder) the notes go to notes.cfg and come back at
/// the next open - and at the next start (read from the file).
#[test]
fn notes_are_saved_and_come_back() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    let folder = crate::services::with(|s| s.store.folder().to_path_buf()).unwrap();
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut p = Notes::default();
    p.open(&Env { test: true, ..Env::default() }, 0.0);
    // the first open reads the file on a worker thread: wait until it is in
    let t0 = std::time::Instant::now();
    while !p.ready() && t0.elapsed() < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    build(&mut p, &g, &mut st, 1.0);
    ev(&mut p, &g, &mut st, Ev::Click(K_ADD), 1.0);
    let id = p.n.as_ref().unwrap().books[0].notes[0].id;
    type_str(&mut p, &g, &mut st, idx(K_TITLE, id as usize), "Kept");
    p.close();
    // the next open in this run: from memory
    p.open(&Env { test: true, ..Env::default() }, 2.0);
    build(&mut p, &g, &mut st, 2.0);
    assert_eq!(p.n.as_ref().unwrap().books[0].notes[0].title, "Kept");
    // the next start: from the file
    let t0 = std::time::Instant::now();
    loop {
        let n = store::load(&folder).unwrap();
        if n.books[0].notes.first().is_some_and(|x| x.title == "Kept") {
            break;
        }
        assert!(t0.elapsed() < std::time::Duration::from_secs(5), "notes.cfg never got the note");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    crate::services::shutdown();
}

/// Review fix: a right-click on a tab while a name is being typed keeps the name and lets go of the focus (no caret left
/// blinking for a field that is gone).
#[test]
fn a_right_click_while_renaming_keeps_the_name_and_ends_the_typing() {
    let (mut p, g, mut st) = page();
    ev(&mut p, &g, &mut st, Ev::Click(K_ADDTAB), 1.0);
    let k1 = tab_key(&p, 1);
    ev(&mut p, &g, &mut st, Ev::Click(k1), 2000.0);
    ev(&mut p, &g, &mut st, Ev::Click(k1), 2100.0);
    assert_eq!(st.focus, Some(K_REN));
    type_str(&mut p, &g, &mut st, K_REN, "!");
    let k0 = tab_key(&p, 0);
    ev(&mut p, &g, &mut st, Ev::Context(k0, 50.0, 50.0), 2200.0);
    assert!(p.rename.is_none() && st.focus.is_none());
    assert_eq!(p.n.as_ref().unwrap().books.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Notes 2!"], "the empty tab went, the new name stayed");
    st.wake = None;
    build(&mut p, &g, &mut st, 2300.0);
    assert!(st.wake.is_none(), "nothing to blink: the menu can sleep");
}

/// Review fix: a long paste is cut at the note's limit.
#[test]
fn a_huge_paste_is_cut_to_the_limit() {
    let (mut p, g, mut st) = page();
    ev(&mut p, &g, &mut st, Ev::Click(K_ADD), 1.0);
    let id = p.n.as_ref().unwrap().books[0].notes[0].id;
    let kx = idx(K_TEXT, id as usize);
    st.focus = Some(kx);
    let _l = CLIP.lock().unwrap_or_else(|e| e.into_inner());
    crate::clipboard::set_test_clip(&"x".repeat(TEXT_MAX + 500));
    let mut cx = Cx::new(1.0, false, &g, &mut st);
    cx.mods.ctrl = true;
    p.event(&Ev::Key(kx, VK_V), &mut cx);
    assert_eq!(p.n.as_ref().unwrap().books[0].notes[0].text.chars().count(), TEXT_MAX);
}

/// Order 119 (option A): a note is a pastel paper in its saved colour, tilted by a fixed small angle that does not change
/// from one build to the next; the grid has three columns.
#[test]
fn a_note_is_a_tilted_paper_in_its_colour() {
    let (mut p, g, mut st) = page();
    for _ in 0..5 {
        ev(&mut p, &g, &mut st, Ev::Click(K_ADD), 1.0);
    }
    st.focus = None;
    let ids: Vec<(u64, u8)> = p.n.as_ref().unwrap().books[0].notes.iter().map(|n| (n.id, n.colour)).collect();
    let first = build(&mut p, &g, &mut st, 2.0);
    let again = build(&mut p, &g, &mut st, 3.0);
    for (id, colour) in ids {
        let (a, b) = (find(&first, idx(K_CARD, id as usize)).unwrap(), find(&again, idx(K_CARD, id as usize)).unwrap());
        assert!(matches!(a.bg, Some(crate::ui::el::Fill::Color(c)) if c == PAPER[colour as usize]), "paper colour {colour}");
        assert!(a.rotate != 0.0 && a.rotate.abs() <= 1.5, "tilt {}", a.rotate);
        assert_eq!(a.rotate, b.rotate, "the tilt is the note's own, not new at every build");
        assert_eq!(a.rotate, tilt(id));
    }
    assert!(TILTS.iter().all(|t| t.abs() <= 1.5 && *t != 0.0));
    assert!(PAPER.windows(2).all(|w| w[0] != w[1]));
}

const PICS_119: &str = r"C:\BoylerUtilities-scratch\119";

/// Order 119's pictures (looked at before DONE): Notes plain and blurred (one note clicked open), with the whole top row -
/// at 1920 x 1080 (scale 1.0) and 3440 x 1440 (scale 1.25).
#[test]
#[ignore]
fn pictures_119() {
    for (scale, size) in [(1.0f32, "1920x1080"), (1.25, "3440x1440")] {
        let (pages, tab) = crate::ui::test_pages_with(Box::new(demo()));
        crate::ui::test_picture(pages, tab, scale, &format!(r"{PICS_119}\119_notes_{size}.png"), |_, _, now| now);
        let mut d = demo();
        d.eye.load_with(true);
        let open = d.n.as_ref().unwrap().books[0].notes[1].id;
        d.revealed.insert(open);
        let (pages, tab) = crate::ui::test_pages_with(Box::new(d));
        crate::ui::test_picture(pages, tab, scale, &format!(r"{PICS_119}\119_notes_blurred_{size}.png"), |_, _, now| now);
    }
}

/// Order 119 item 2 (the owner: "sure"): right-click a note > a row of the four colours; the pick is saved on that note only.
#[test]
fn right_click_a_note_offers_the_four_colours_and_the_pick_is_saved() {
    let (mut p, g, mut st) = page();
    for _ in 0..2 {
        ev(&mut p, &g, &mut st, Ev::Click(K_ADD), 1.0);
    }
    let ids: Vec<u64> = p.n.as_ref().unwrap().books[0].notes.iter().map(|n| n.id).collect();
    type_str(&mut p, &g, &mut st, idx(K_TEXT, ids[0] as usize), "keep");
    ev(&mut p, &g, &mut st, Ev::Context(idx(K_CARD, ids[0] as usize), 10.0, 10.0), 2.0);
    assert!(matches!(p.menu, Some(Menu::Note(..))));
    {
        let mut cx = Cx::new(3.0, false, &g, &mut st);
        let m = p.popup(&mut cx).expect("the menu");
        for j in 0..4 {
            assert!(find(&[m.clone()], idx(sub(K_MENU, "sw"), j)).is_some(), "swatch {j}");
        }
    }
    ev(&mut p, &g, &mut st, Ev::Click(idx(sub(K_MENU, "sw"), 2)), 4.0);
    let notes = &p.n.as_ref().unwrap().books[0].notes;
    assert_eq!((notes[0].colour, notes[1].colour), (2, 1), "only that note changed");
    assert!(p.menu.is_none(), "the menu closes");
    let back = store::Notes::parse(&p.n.as_ref().unwrap().to_text()).unwrap();
    assert_eq!(back.books[0].notes[0].colour, 2, "saved with the note");
    let els = build(&mut p, &g, &mut st, 5.0);
    assert!(matches!(find(&els, idx(K_CARD, ids[0] as usize)).unwrap().bg, Some(crate::ui::el::Fill::Color(c)) if c == PAPER[2]));
}

/// Order 119 item 2's picture: the right-click menu of the pink note (colour row, its own colour ringed), at both scales.
#[test]
#[ignore]
fn picture_119_colour_menu() {
    for (scale, size) in [(1.0f32, "1920x1080"), (1.25, "3440x1440")] {
        let (pages, tab) = crate::ui::test_pages_with(Box::new(demo()));
        crate::ui::test_picture(pages, tab, scale, &format!(r"{PICS_119}\119_colour_menu_{size}.png"), |ui, g, now| {
            ui.update(now + 50.0);
            ui.prepare_page(g, now + 50.0);
            ui.mouse_move(300.0, 235.0, now + 100.0);
            ui.context(300.0, 235.0, now + 100.0);
            now + 500.0
        });
    }
}

// ---------------------------------------------------------------- Order 121: select + copy


/// A note with `title` / `text` typed in; (page, gfx, state, title key, text key). The text field has the focus, the caret
/// at its end.
fn note_with(title: &str, text: &str) -> (Notes, Gfx, State, Key, Key) {
    let (mut p, g, mut st) = page();
    ev(&mut p, &g, &mut st, Ev::Click(K_ADD), 1.0);
    let id = p.n.as_ref().unwrap().books[0].notes[0].id;
    let (kt, kx) = (idx(K_TITLE, id as usize), idx(K_TEXT, id as usize));
    type_str(&mut p, &g, &mut st, kt, title);
    st.focus = Some(kx);
    p.caret = 0;
    type_str(&mut p, &g, &mut st, kx, text);
    (p, g, st, kt, kx)
}

fn copied(p: &mut Notes, g: &Gfx, st: &mut State, k: Key) -> String {
    crate::clipboard::set_test_clip("<untouched>");
    evm(p, g, st, Ev::Key(k, VK_C), 2.0, true, false);
    crate::clipboard::test_clip().unwrap_or_default()
}

/// the text box as the frame reports it to a press: (left, top, width, height)
fn boxr() -> (f32, f32, f32, f32) {
    (100.0, 200.0, TEXT_W, 126.0)
}

#[test]
fn ctrl_a_ctrl_c_copies_the_whole_text_and_ctrl_x_cuts_it() {
    let _l = CLIP.lock().unwrap_or_else(|e| e.into_inner());
    let (mut p, g, mut st, _, kx) = note_with("IP", "192.168.1.107");
    evm(&mut p, &g, &mut st, Ev::Key(kx, VK_A), 2.0, true, false);
    assert_eq!(copied(&mut p, &g, &mut st, kx), "192.168.1.107");
    evm(&mut p, &g, &mut st, Ev::Key(kx, VK_X), 3.0, true, false);
    assert_eq!(crate::clipboard::test_clip().as_deref(), Some("192.168.1.107"));
    assert_eq!(p.n.as_ref().unwrap().books[0].notes[0].text, "", "cut takes it out of the note");
}

#[test]
fn dragging_with_the_mouse_selects_and_typing_replaces_the_selection() {
    let _l = CLIP.lock().unwrap_or_else(|e| e.into_inner());
    let (mut p, g, mut st, _, kx) = note_with("", "milk\reggs");
    let r = boxr();
    ev(&mut p, &g, &mut st, Ev::Press(kx, r.0, r.1 + 2.0, r), 2.0);
    ev(&mut p, &g, &mut st, Ev::Drag(kx, r.0 + 500.0, r.1 + 2.0, r), 2.1);
    assert_eq!(copied(&mut p, &g, &mut st, kx), "milk", "the first line");
    ev(&mut p, &g, &mut st, Ev::Drag(kx, r.0 + 500.0, r.1 + LH_TEXT + 2.0, r), 2.2);
    assert_eq!(copied(&mut p, &g, &mut st, kx), "milk\neggs", "dragged down to the second line");
    ev(&mut p, &g, &mut st, Ev::Release(kx), 2.3);
    type_str(&mut p, &g, &mut st, kx, "x");
    assert_eq!(p.n.as_ref().unwrap().books[0].notes[0].text, "x");
}

#[test]
fn double_click_selects_a_word_and_shift_arrows_extend_from_the_caret() {
    let _l = CLIP.lock().unwrap_or_else(|e| e.into_inner());
    let (mut p, g, mut st, _, kx) = note_with("", "milk eggs");
    let r = boxr();
    let x = r.0 + g.text_width("milk ", F_TEXT) + 4.0;
    ev(&mut p, &g, &mut st, Ev::Press(kx, x, r.1 + 2.0, r), 10.0);
    ev(&mut p, &g, &mut st, Ev::Release(kx), 10.05);
    ev(&mut p, &g, &mut st, Ev::Press(kx, x, r.1 + 2.0, r), 10.1);
    assert_eq!(copied(&mut p, &g, &mut st, kx), "eggs");
    // shift + Left twice from the end of the text
    ev(&mut p, &g, &mut st, Ev::Key(kx, edit::VK_END), 3.0);
    evm(&mut p, &g, &mut st, Ev::Key(kx, edit::VK_LEFT), 3.1, false, true);
    evm(&mut p, &g, &mut st, Ev::Key(kx, edit::VK_LEFT), 3.2, false, true);
    assert_eq!(copied(&mut p, &g, &mut st, kx), "gs");
    type_str(&mut p, &g, &mut st, kx, "X");
    assert_eq!(p.n.as_ref().unwrap().books[0].notes[0].text, "milk egX");
}

#[test]
fn a_title_selects_and_copies_too() {
    let _l = CLIP.lock().unwrap_or_else(|e| e.into_inner());
    let (mut p, g, mut st, kt, _) = note_with("Router", "x");
    st.focus = Some(kt);
    evm(&mut p, &g, &mut st, Ev::Key(kt, VK_A), 2.0, true, false);
    assert_eq!(copied(&mut p, &g, &mut st, kt), "Router");
}

#[test]
fn the_note_menu_has_copy_and_paste() {
    let _l = CLIP.lock().unwrap_or_else(|e| e.into_inner());
    let (mut p, g, mut st, _, kx) = note_with("T", "milk eggs");
    evm(&mut p, &g, &mut st, Ev::Key(kx, VK_A), 2.0, true, false);
    ev(&mut p, &g, &mut st, Ev::Context(kx, 10.0, 10.0), 3.0);
    // the menu takes the focus (the frame blurs the field): the copy still has the selection
    ev(&mut p, &g, &mut st, Ev::Blur(kx), 3.1);
    crate::clipboard::set_test_clip("<untouched>");
    ev(&mut p, &g, &mut st, Ev::Click(idx(K_MENU, MENU_COPY)), 4.0);
    assert_eq!(crate::clipboard::test_clip().as_deref(), Some("milk eggs"));
    assert!(p.menu.is_none());
    ev(&mut p, &g, &mut st, Ev::Context(kx, 10.0, 10.0), 5.0);
    crate::clipboard::set_test_clip("!");
    ev(&mut p, &g, &mut st, Ev::Click(idx(K_MENU, MENU_PASTE)), 6.0);
    assert!(p.n.as_ref().unwrap().books[0].notes[0].text.contains('!'), "pasted into the note");
}

/// Order 121 (A_121_01): the blurred text box paints fake letters - the same picture every build until the words change.
#[test]
fn a_blurred_note_paints_fake_letters_that_stay_until_the_text_changes() {
    let (mut p, g, mut st, _, kx) = note_with("IP", "192.168.1.107");
    let id = p.n.as_ref().unwrap().books[0].notes[0].id as usize;
    st.focus = None;
    p.revealed.clear();
    let sig = |els: &[El]| find(els, idx(K_TEXT, id)).unwrap().children[0].paint_sig;
    let sharp = sig(&build(&mut p, &g, &mut st, 1.0));
    ev(&mut p, &g, &mut st, Ev::Click(K_EYE), 2.0);
    let (a, b) = (sig(&build(&mut p, &g, &mut st, 3.0)), sig(&build(&mut p, &g, &mut st, 900.0)));
    assert_ne!(a, sharp, "not the real letters");
    assert_eq!(a, b, "no flicker from one build to the next");
    // new words (typed after the note was shown again) = a new set
    ev(&mut p, &g, &mut st, Ev::Press(idx(K_CARD, id), 10.0, 10.0, (0.0, 0.0, 100.0, 100.0)), 4.0);
    st.focus = Some(kx);
    type_str(&mut p, &g, &mut st, kx, "x");
    st.focus = None;
    p.revealed.clear();
    assert_ne!(sig(&build(&mut p, &g, &mut st, 5.0)), a);
}

const PICS_121: &str = r"C:\BoylerUtilities-scratch\121";

/// Order 121's pictures (looked at before DONE): a dragged selection over two lines of the yellow note, a double-clicked word
/// on the blue note, the right-click menu with Copy / Paste, and the blurred notes (fake letters; one note clicked open).
#[test]
#[ignore]
fn pictures_121() {
    for (scale, size) in [(1.0f32, "1920x1080"), (1.25, "3440x1440")] {
        let shot = |name: &str, pg: Notes, act: fn(&mut crate::ui::Ui, &crate::gfx::Gfx, f64) -> f64| {
            let (pages, tab) = crate::ui::test_pages_with(Box::new(pg));
            crate::ui::test_picture(pages, tab, scale, &format!(r"{PICS_121}\121_{name}_{size}.png"), |ui, g, now| {
                ui.update(now + 50.0);
                ui.prepare_page(g, now + 50.0);
                act(ui, g, now + 100.0)
            });
        };
        // one input step, then the page is rebuilt as a frame would
        fn step(ui: &mut crate::ui::Ui, g: &crate::gfx::Gfx, now: f64) {
            ui.update(now);
            ui.prepare_page(g, now);
        }
        // drag from "Milk" to the middle of "Coffee"
        fn drag(ui: &mut crate::ui::Ui, g: &crate::gfx::Gfx, now: f64) -> f64 {
            ui.mouse_move(40.0, 183.0, now);
            step(ui, g, now);
            ui.mouse_down(40.0, 183.0, now + 10.0);
            step(ui, g, now + 10.0);
            ui.mouse_move(90.0, 190.0, now + 20.0);
            step(ui, g, now + 20.0);
            ui.mouse_move(88.0, 201.0, now + 30.0);
            step(ui, g, now + 30.0);
            ui.mouse_up(88.0, 201.0, now + 40.0);
            step(ui, g, now + 40.0);
            now + 600.0
        }
        shot("selection", demo(), drag);
        // double-click "sticky" on the blue note
        shot("word", demo(), |ui, g, now| {
            for t in [0.0, 120.0] {
                ui.mouse_move(440.0, 183.0, now + t);
                step(ui, g, now + t);
                ui.mouse_down(440.0, 183.0, now + t + 10.0);
                step(ui, g, now + t + 10.0);
                ui.mouse_up(440.0, 183.0, now + t + 20.0);
                step(ui, g, now + t + 20.0);
            }
            now + 700.0
        });
        // the menu over the selection
        shot("menu", demo(), |ui, g, now| {
            let t = drag(ui, g, now);
            ui.context(60.0, 190.0, t);
            step(ui, g, t);
            t + 400.0
        });
        let mut d = demo();
        d.eye.load_with(true);
        let open = d.n.as_ref().unwrap().books[0].notes[1].id;
        d.revealed.insert(open);
        shot("blurred", d, |_, _, now| now);
    }
}

/// Order 121 end review: the keydown of Ctrl / Shift / a letter comes before the copy or the character - it must not drop the
/// selection (else Ctrl+C copies nothing and a typed letter does not replace the selection).
#[test]
fn a_modifier_or_letter_keydown_keeps_the_selection() {
    let _l = CLIP.lock().unwrap_or_else(|e| e.into_inner());
    let (mut p, g, mut st, _, kx) = note_with("", "milk eggs");
    evm(&mut p, &g, &mut st, Ev::Key(kx, VK_A), 2.0, true, false);
    for vk in [0x11u16, 0x10, 0x58] {
        ev(&mut p, &g, &mut st, Ev::Key(kx, vk), 2.1);
    }
    assert_eq!(copied(&mut p, &g, &mut st, kx), "milk eggs", "still selected after Ctrl / Shift / X keydowns");
    type_str(&mut p, &g, &mut st, kx, "x");
    assert_eq!(p.n.as_ref().unwrap().books[0].notes[0].text, "x", "the letter replaces the selection");
}

/// Order 121 end review: a double-clicked word stays whole when the pointer jitters inside it.
#[test]
fn a_double_clicked_word_survives_pointer_jitter() {
    let _l = CLIP.lock().unwrap_or_else(|e| e.into_inner());
    let (mut p, g, mut st, _, kx) = note_with("", "milk bread");
    let r = boxr();
    let x = r.0 + g.text_width("milk br", F_TEXT) + 2.0;
    ev(&mut p, &g, &mut st, Ev::Press(kx, x, r.1 + 2.0, r), 10.0);
    ev(&mut p, &g, &mut st, Ev::Release(kx), 10.05);
    ev(&mut p, &g, &mut st, Ev::Press(kx, x, r.1 + 2.0, r), 10.1);
    ev(&mut p, &g, &mut st, Ev::Drag(kx, x + 1.0, r.1 + 2.0, r), 10.12);
    ev(&mut p, &g, &mut st, Ev::Drag(kx, x - 1.0, r.1 + 3.0, r), 10.14);
    assert_eq!(copied(&mut p, &g, &mut st, kx), "bread");
    // dragged out of the word to the left it grows over "milk "
    ev(&mut p, &g, &mut st, Ev::Drag(kx, r.0 + 1.0, r.1 + 2.0, r), 10.2);
    assert_eq!(copied(&mut p, &g, &mut st, kx), "milk bread");
}
