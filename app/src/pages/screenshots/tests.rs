//! The Screenshots page against the FAKE engine (Order 019): every action the page triggers, and the boxes against the
//! drawing (menu-v22 page `shot`, Chromium's layout from tools/ref/dom_dump.js).

use super::*;
use crate::gfx::Gfx;
use crate::ui::cx::State;
use crate::ui::lay::Laid;

fn page(now: f64) -> Screenshots {
    let mut p = Screenshots::default();
    p.open(&Env { test: true, frozen: true, ..Env::default() }, now);
    p
}

fn ev(p: &mut Screenshots, e: Ev, now: f64) {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(now, false, &g, &mut st);
    p.event(&e, &mut cx);
}

fn build(p: &mut Screenshots, now: f64) -> Laid {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(now, false, &g, &mut st);
    let kids = p.build(&mut cx);
    let root = El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
    Laid::new(&g, root, 600.0, None)
}

fn tile_key(p: &Screenshots, i: usize) -> Key {
    sub(K_SHOT, &p.order()[i].to_string())
}

#[test]
fn opens_with_the_sample_gallery_and_drops_everything_on_close() {
    let mut p = page(0.0);
    assert_eq!(p.order().len(), 8);
    assert!(p.shots.iter().all(|v| v.img.is_some()), "the first 8 pictures decode on open");
    assert_eq!(p.dir, r"Pictures\Screenshots");
    let caps: Vec<&str> = p.shots.iter().map(|v| v.cap.as_str()).collect();
    assert_eq!(caps, ["21:34", "21:31", "21:12", "20:47", "20:05", "Yesterday", "Yesterday", "Mon"]);
    p.close();
    assert!(p.svc.is_none() && p.shots.is_empty() && p.fake.is_none());
}

#[test]
fn gallery_boxes_are_chromiums() {
    // dom_dump of menu-v22 (page shot): .gal at window (26, 98) 548 wide; .shot 131 x 93.6875; .sth 131 x 73.6875;
    // the caption text at (28, 177.6875); the second row's first .shot at y 201.6875; .rsl at y 317.375
    let mut p = page(0.0);
    build(&mut p, 0.0); // the keymap
    let l = build(&mut p, 0.0);
    let r = |k: Key| l.rect_of(k).map(|(x, y, w, h)| (x, y + PAGE_TOP, w, h)).unwrap();
    assert_eq!(r(K_GAL), (26.0, 98.0, 548.0, 197.375));
    assert_eq!(r(tile_key(&p, 0)), (26.0, 98.0, 131.0, 93.6875));
    assert_eq!(r(sub(tile_key(&p, 0), "th")), (26.0, 98.0, 131.0, 73.6875));
    assert_eq!(r(tile_key(&p, 3)).0, 443.0);
    assert_eq!(r(tile_key(&p, 4)).1, 201.6875);
    let rs = r(sub(K_RESET, "pc"));
    assert!((rs.1 - 317.4688).abs() < 0.02, "reset link at {rs:?}");
}

#[test]
fn click_selects_and_two_or_more_show_the_bar() {
    let mut p = page(0.0);
    build(&mut p, 0.0);
    let k0 = tile_key(&p, 0);
    ev(&mut p, Ev::Click(k0), 10.0);
    assert_eq!(p.sel.len(), 1);
    // Ctrl+A (focus on a shot)
    p.sel.all(&p.order());
    assert_eq!(p.sel.len(), 8);
    // the bar is the page's window-fixed overlay, at the window's bottom centre
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(2000.0, false, &g, &mut st);
    assert!(p.bar_shown());
    let bar = p.overlay(&mut cx).unwrap();
    let l = Laid::new(&g, El::block().w(WIN_W).h(WIN_H).child(bar), WIN_W, Some(WIN_H));
    let (_, y, _, h) = l.rect_of(K_SEL).expect("the bar is there with 8 selected");
    assert_eq!(y + h, WIN_H - 14.0);
    assert!(l.rect_of(k_sel_copy()).is_some());
    // a plain click on another one = just that one; the bar fades out
    let k2 = tile_key(&p, 2);
    ev(&mut p, Ev::Click(k2), 3000.0);
    assert_eq!(p.sel.in_order(&p.order()), vec![p.order()[2]]);
    // the × clears
    p.sel.all(&p.order());
    ev(&mut p, Ev::Click(k_sel_x()), 4000.0);
    assert!(p.sel.is_empty());
}

#[test]
fn a_click_on_empty_space_clears_the_selection() {
    let mut p = page(0.0);
    build(&mut p, 0.0);
    let k = tile_key(&p, 1);
    ev(&mut p, Ev::Press(k, 0.0, 0.0, (0.0, 0.0, 1.0, 1.0)), 5.0);
    ev(&mut p, Ev::Click(k), 6.0);
    assert_eq!(p.sel.len(), 1);
    // the frame blurs the shot (nothing keyed under the pointer) and sends no Press
    ev(&mut p, Ev::Blur(k), 900.0);
    build(&mut p, 901.0);
    assert!(p.sel.is_empty());
    // but a blur followed by a press on another element of the page keeps it
    ev(&mut p, Ev::Click(k), 1000.0);
    ev(&mut p, Ev::Blur(k), 1500.0);
    ev(&mut p, Ev::Press(k_sel_copy(), 0.0, 0.0, (0.0, 0.0, 1.0, 1.0)), 1500.0);
    build(&mut p, 1501.0);
    assert_eq!(p.sel.len(), 1);
}

#[test]
fn double_click_opens_the_lightbox_and_drag_picks_the_selection() {
    let mut p = page(0.0);
    build(&mut p, 0.0);
    let k = tile_key(&p, 2);
    ev(&mut p, Ev::Click(k), 100.0);
    ev(&mut p, Ev::Click(k), 250.0);
    assert_eq!(p.lb.map(|l| l.0), Some(p.order()[2]));
    // slow clicks are two clicks
    let mut q = page(0.0);
    build(&mut q, 0.0);
    let k = tile_key(&q, 2);
    ev(&mut q, Ev::Click(k), 100.0);
    ev(&mut q, Ev::Click(k), 100.0 + double_click_ms() + 50.0);
    assert!(q.lb.is_none());
    // a drag of 5 px or more starts on the thumbnail
    let th = sub(tile_key(&q, 5), "th");
    ev(&mut q, Ev::Press(th, 100.0, 100.0, (0.0, 0.0, 131.0, 73.0)), 2000.0);
    ev(&mut q, Ev::Drag(th, 102.0, 101.0, (0.0, 0.0, 131.0, 73.0)), 2010.0);
    assert!(q.dragging.is_none());
    ev(&mut q, Ev::Drag(th, 106.0, 104.0, (0.0, 0.0, 131.0, 73.0)), 2020.0);
    assert_eq!(q.dragging.as_deref(), Some(&[q.order()[5]][..]));
    ev(&mut q, Ev::Release(th), 2100.0);
    assert!(q.dragging.is_none());
}

#[test]
fn delete_moves_to_the_recycle_bin_fades_and_the_rest_glide() {
    let mut p = page(0.0);
    build(&mut p, 0.0);
    let ids = vec![p.order()[1], p.order()[2]];
    p.sel.ids = ids.clone();
    let k = tile_key(&p, 0);
    ev(&mut p, Ev::Key(k, 0x2E), 1000.0);
    let os = p.fake.clone().unwrap();
    assert_eq!(os.state().recycled.len(), 2, "both files went to the Recycle Bin");
    assert!(p.toast.as_ref().is_some_and(|t| t.0 == "2 screenshots moved to the Recycle Bin"));
    // fading (still in the grid), then gone; the shots behind them glide from their old cells
    assert_eq!(p.shots.len(), 8);
    assert_eq!(p.order().len(), 6);
    build(&mut p, 1100.0);
    assert_eq!(p.shots.len(), 8);
    build(&mut p, 1180.0);
    assert_eq!(p.shots.len(), 6);
    let third = p.shots[1].id; // was at index 3
    assert_eq!(p.moved.get(&third).map(|m| m.0), Some(3));
    // the gallery itself no longer lists them
    assert_eq!(p.svc.as_ref().unwrap().gallery().unwrap().len(), 6);
}

/// Order 042 (the owner's test 2): a right-click on ONE picture opens its menu (Open, Copy, Show in folder, Delete with a
/// confirm); on a picture of a bigger selection the menu acts on the whole selection.
#[test]
fn right_click_on_one_picture_opens_its_menu_and_delete_asks_first() {
    let mut p = page(0.0);
    build(&mut p, 0.0);
    let os = p.fake.clone().unwrap();
    let id = p.order()[2];
    let k = tile_key(&p, 2);
    ev(&mut p, Ev::Context(k, 200.0, 150.0), 10.0);
    assert_eq!(p.sel.ids, vec![id], "the right-clicked picture becomes the selection");
    let c = p.ctx.as_ref().expect("menu open");
    assert_eq!(c.ids, vec![id]);
    assert!(c.head != "1 screenshot" && !c.head.is_empty(), "the file name: {}", c.head);
    {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(20.0, false, &g, &mut st);
        assert!(p.popup(&mut cx).is_some(), "the menu is drawn");
    }
    // rows after the head: Open, Copy, Show in folder, Delete
    ev(&mut p, Ev::Click(idx(K_CTX, 3)), 30.0);
    assert_eq!(os.state().shown.len(), 1, "Show in folder");
    assert!(p.ctx.is_none());
    let k = tile_key(&p, 2);
    ev(&mut p, Ev::Context(k, 200.0, 150.0), 40.0);
    ev(&mut p, Ev::Click(idx(K_CTX, 4)), 50.0);
    assert!(p.ctx.as_ref().is_some_and(|c| c.ask), "Delete asks first");
    assert!(os.state().recycled.is_empty());
    ev(&mut p, Ev::Click(sub(K_CTXQ, "no")), 60.0);
    assert!(p.ctx.is_none() && os.state().recycled.is_empty(), "Cancel keeps it");
    let k = tile_key(&p, 2);
    ev(&mut p, Ev::Context(k, 200.0, 150.0), 70.0);
    ev(&mut p, Ev::Click(idx(K_CTX, 4)), 80.0);
    ev(&mut p, Ev::Click(sub(K_CTXQ, "go")), 90.0);
    assert_eq!(os.state().recycled.len(), 1, "moved to the Recycle Bin");
    // a picture of a 3-picture selection: the menu is for all three (no Open row)
    let sel = vec![p.order()[0], p.order()[1], p.order()[3]];
    p.sel.ids = sel.clone();
    let k = tile_key(&p, 1);
    ev(&mut p, Ev::Context(k, 10.0, 10.0), 100.0);
    assert_eq!(p.ctx.as_ref().unwrap().ids.len(), 3);
    assert_eq!(p.ctx.as_ref().unwrap().head, "3 screenshots");
    ev(&mut p, Ev::Click(idx(K_CTX, 1)), 110.0);
    assert!(p.toast.as_ref().is_some_and(|t| t.0 == "3 screenshots copied"), "{:?}", p.toast);
}

#[test]
fn delete_without_a_recycle_bin_says_so_and_keeps_the_shot() {
    let mut p = page(0.0);
    build(&mut p, 0.0);
    let os = p.fake.clone().unwrap();
    os.state().no_bin.push(PathBuf::from(r"D:\"));
    let id = p.order()[6]; // the D:\Clips one
    ev(&mut p, Ev::Click(k_sel_del()), 0.0); // nothing selected: nothing happens
    assert!(os.state().recycled.is_empty());
    p.sel.ids = vec![id];
    ev(&mut p, Ev::Click(k_sel_del()), 10.0);
    assert!(os.state().recycled.is_empty());
    assert_eq!(p.order().len(), 8);
    assert_eq!(p.toast.as_ref().map(|t| t.0.as_str()), Some("Not deleted · that drive has no Recycle Bin"));
}

#[test]
fn copy_one_is_the_picture_copy_many_is_the_files() {
    let mut p = page(0.0);
    let os = p.fake.clone().unwrap();
    p.sel.ids = vec![p.order()[0]];
    ev(&mut p, Ev::Click(k_sel_copy()), 0.0);
    assert!(matches!(os.state().clipboard, Some(bu_screenshot::fake::Clip::Picture { .. })));
    assert_eq!(p.toast.as_ref().map(|t| t.0.as_str()), Some("Copied to clipboard"));
    p.sel.ids = vec![p.order()[0], p.order()[3], p.order()[7]];
    ev(&mut p, Ev::Click(k_sel_copy()), 10.0);
    assert!(matches!(&os.state().clipboard, Some(bu_screenshot::fake::Clip::Files(f)) if f.len() == 3));
    assert_eq!(p.toast.as_ref().map(|t| t.0.as_str()), Some("3 screenshots copied"));
}

#[test]
fn change_path_window_open_change_and_close() {
    let mut p = page(0.0);
    let os = p.fake.clone().unwrap();
    ev(&mut p, Ev::Click(K_PATH), 0.0);
    assert!(p.dlg.is_some());
    // Open = Explorer there
    ev(&mut p, Ev::Click(K_DLG_OPEN), 100.0);
    assert_eq!(os.state().opened, vec![PathBuf::from(r"C:\Users\test\Pictures\Screenshots")]);
    // Change = the picker (the fake answers after the drawing's 700 ms); saved at once
    ev(&mut p, Ev::Click(K_DLG_CHANGE), 200.0);
    assert!(p.wait.is_some());
    ev(&mut p, Ev::Click(K_DLG_CHANGE), 300.0); // a second click while it waits does nothing
    assert!(!p.poll(950.0));
    assert_eq!(p.dir, r"D:\Clips\Screenshots");
    assert_eq!(p.dir_new_at, Some(950.0));
    assert_eq!(p.toast.as_ref().map(|t| t.0.as_str()), Some(r"New screenshots go to D:\Clips\Screenshots"));
    assert_eq!(p.svc.as_ref().unwrap().saved_dir().unwrap(), Some(PathBuf::from(r"D:\Clips\Screenshots")));
    // and back
    ev(&mut p, Ev::Click(K_DLG_CHANGE), 1000.0);
    p.poll(1800.0);
    assert_eq!(p.dir, r"Pictures\Screenshots");
    // a cancelled picker changes nothing
    os.state().pick = None;
    p.wait = Some((2000.0, Job::FakePick(Err(ShotError::Cancelled))));
    p.poll(2800.0);
    assert_eq!(p.dir, r"Pictures\Screenshots");
    // × and a click beside it close it
    ev(&mut p, Ev::Click(sub(K_DLG, "x")), 3000.0);
    assert!(p.dlg.is_none());
    ev(&mut p, Ev::Click(K_PATH), 3100.0);
    ev(&mut p, Ev::Click(sub(K_DLG, "out")), 3200.0);
    assert!(p.dlg.is_none());
    // Esc / outside click from the frame
    ev(&mut p, Ev::Click(K_PATH), 3300.0);
    p.popup_dismiss();
    assert!(p.dlg.is_none());
}

/// Order 036: "Change" writes ONE entry with the folder before the FIRST change; the reset puts it back through the page,
/// an unticked line is kept; "Windows defaults" = no folder chosen. The FAKE engine only.
#[test]
fn the_folder_goes_into_the_change_log_and_back() {
    use crate::undo::{read_record, Kind, Outcome, Resettable, Review};
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    let review = |p: &Screenshots, kind| crate::services::with(|s| Review::for_page(kind, p, &s.store)).unwrap();
    let mut p = page(0.0);
    // nothing changed: nothing to reset, Windows' folder already
    assert!(review(&p, Kind::HowItWas).is_empty());
    assert!(review(&p, Kind::WindowsDefaults).is_empty());
    // D:\Clips, then back to Windows' folder, then D:\Clips again: one entry, the folder before the FIRST change
    for t in [100.0, 1000.0, 2000.0] {
        ev(&mut p, Ev::Click(K_DLG_CHANGE), t);
        p.poll(t + 800.0);
    }
    assert_eq!(p.dir, r"D:\Clips\Screenshots");
    let r = crate::services::with(|s| read_record(&s.store, "shot", "folder")).flatten().expect("one entry");
    assert_eq!((r.label.as_str(), r.was.raw.as_str(), r.was.text.as_str()), ("Screenshots folder", "", r"Pictures\Screenshots"));
    assert_eq!((r.now.raw.as_str(), r.now.text.as_str()), (r"D:\Clips\Screenshots", r"D:\Clips\Screenshots"));
    // untick = kept
    let mut rv = review(&p, Kind::HowItWas);
    assert_eq!(rv.title(), "Screenshots · back to how it was?");
    assert_eq!(rv.lines.len(), 1);
    assert_eq!(rv.lines[0].change_text(), "D:\\Clips\\Screenshots  →  Pictures\\Screenshots");
    rv.toggle(0);
    assert!(rv.apply_each(&mut [&mut p as &mut dyn Resettable], &mut |_| Ok(())).is_empty());
    assert_eq!(p.svc.as_ref().unwrap().saved_dir().unwrap(), Some(PathBuf::from(r"D:\Clips\Screenshots")));
    // ticked = back (the open page shows it)
    rv.toggle(0);
    let res = crate::services::with(|s| rv.apply(&mut s.store, &mut [&mut p as &mut dyn Resettable])).unwrap();
    assert_eq!(res[0].outcome, Outcome::Ok);
    assert_eq!(p.svc.as_ref().unwrap().saved_dir().unwrap(), None);
    assert_eq!(p.dir, r"Pictures\Screenshots");
    assert!(review(&p, Kind::HowItWas).is_empty());
    // Windows defaults: a chosen folder goes back to none
    ev(&mut p, Ev::Click(K_DLG_CHANGE), 5000.0);
    p.poll(5800.0);
    let rv = review(&p, Kind::WindowsDefaults);
    assert_eq!(rv.lines.len(), 1);
    assert_eq!(rv.lines[0].change_text(), "D:\\Clips\\Screenshots  →  Pictures\\Screenshots");
    let res = crate::services::with(|s| rv.apply(&mut s.store, &mut [&mut p as &mut dyn Resettable])).unwrap();
    assert_eq!(res[0].outcome, Outcome::Ok);
    assert_eq!(p.svc.as_ref().unwrap().saved_dir().unwrap(), None);
    // a cancelled picker writes nothing new
    let before = crate::services::with(|s| read_record(&s.store, "shot", "folder")).flatten().unwrap();
    p.fake.as_ref().unwrap().state().pick = None;
    p.wait = Some((6000.0, Job::FakePick(Err(ShotError::Cancelled))));
    p.poll(7000.0);
    assert_eq!(crate::services::with(|s| read_record(&s.store, "shot", "folder")).flatten().unwrap().now, before.now);
    crate::services::shutdown();
}

/// Settings › Reset and the uninstaller ask CLOSED pages: `resettable()` makes nothing; the engine comes on first need.
#[test]
fn a_closed_page_is_cheap_and_resets_through_an_engine_made_on_first_need() {
    use crate::undo::{Kind, Outcome, Resettable, Review, Val};
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    let mut fresh = Screenshots::default();
    assert!(fresh.resettable().is_some());
    assert!(fresh.lazy.get().is_none() && fresh.svc.is_none(), "resettable() made an engine");
    // a folder chosen earlier (the page was open then), the page closed now: its engine is made for the line
    let (_, eng) = gallery::sample_engine();
    eng.set_save_dir(std::path::Path::new(r"D:\Clips\Screenshots")).unwrap();
    let _ = fresh.lazy.set((Box::new(eng), gallery::profile_dir(true)));
    crate::services::with(|s| crate::undo::record(&mut s.store, "shot", "folder", "Screenshots folder", &Val::new("", r"Pictures\Screenshots"), &Val::plain(r"D:\Clips\Screenshots")).unwrap());
    let rv = crate::services::with(|s| Review::for_page(Kind::HowItWas, &fresh, &s.store)).unwrap();
    assert_eq!(rv.lines.len(), 1);
    let res = crate::services::with(|s| rv.apply(&mut s.store, &mut [&mut fresh as &mut dyn Resettable])).unwrap();
    assert_eq!(res[0].outcome, Outcome::Ok);
    assert_eq!(fresh.lazy.get().unwrap().0.saved_dir().unwrap(), None);
    crate::services::shutdown();
}

/// The reset links open the frame's ONE review under the link (no page-local popup any more).
#[test]
fn the_reset_links_open_the_frames_review() {
    let mut p = page(0.0);
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let r = (220.0, 480.0, 132.0, 16.0);
    let mut cx = Cx::new(0.0, false, &g, &mut st).for_page("shot");
    p.event(&Ev::Press(sub(K_RESET, "pc"), 0.0, 0.0, r), &mut cx);
    p.event(&Ev::Click(sub(K_RESET, "pc")), &mut cx);
    p.event(&Ev::Click(sub(K_RESET, "win")), &mut cx);
    let reqs = std::mem::take(&mut cx.reqs);
    assert!(matches!(reqs[0], crate::ui::cx::Req::Reset(crate::undo::Kind::HowItWas, b) if b == r), "{reqs:?}");
    assert!(matches!(reqs[1], crate::ui::cx::Req::Reset(crate::undo::Kind::WindowsDefaults, _)), "{reqs:?}");
}

#[test]
fn a_new_shot_from_the_capture_glides_in_at_the_front() {
    let mut p = page(0.0);
    let os = p.fake.clone().unwrap();
    // the engine saves one more (as the overlay does), then tells the page
    {
        let mut s = os.state();
        s.unix_ms += 60_000;
        s.time.minute = 38;
    }
    let img = Image { width: 4, height: 2, bgra: vec![255; 32] };
    let eng = Engine::new(os.clone(), PathBuf::from(gallery::FAKE_DATA));
    let shot = eng.save(&img).unwrap();
    gallery_changed();
    assert!(p.tick(500.0));
    assert_eq!(p.order()[0], shot.id);
    assert_eq!(p.order().len(), 9);
    assert!(p.appeared.contains_key(&shot.id));
    assert_eq!(p.moved.get(&p.order()[1]).map(|m| m.0), Some(0));
    assert_eq!(p.shots[0].cap, "21:38");
}

#[test]
fn nothing_slow_on_open() {
    // opening reads the small index and decodes the first 8 thumbnails; a big gallery decodes the rest 2 per frame
    let os = FakeOs::two_monitors();
    let eng = Engine::new(os.clone(), PathBuf::from(r"C:\BU-test\big"));
    for i in 0..30u8 {
        os.state().unix_ms += 1000;
        eng.save(&Image { width: 2, height: 2, bgra: vec![i; 16] }).unwrap();
    }
    let mut p = Screenshots { svc: Some(Box::new(eng)), fake: Some(os), ..Default::default() };
    p.load(0.0, false);
    p.decode_some(8);
    assert_eq!(p.shots.iter().filter(|v| v.img.is_some()).count(), 8);
    assert!(p.tick(16.0));
    assert_eq!(p.shots.iter().filter(|v| v.img.is_some()).count(), 10);
}

#[test]
fn folder_window_boxes_are_chromiums() {
    // dom_dump of menu-v22 with openDirDlg(): .dlg (106, 185.5) 388 x 149; .dpath (124, 238.5) 352 x 32; its text box
    // (157, 246.0625); .cbtn.ic Open (297.625, 288.5) 78.6094 x 30; Change (384.2344, 288.5) 91.7656 x 30
    let mut p = page(0.0);
    ev(&mut p, Ev::Click(K_PATH), 0.0);
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(5000.0, false, &g, &mut st);
    let pop = p.popup(&mut cx).unwrap();
    let l = Laid::new(&g, El::block().w(WIN_W).h(WIN_H).child(pop), WIN_W, Some(WIN_H));
    let near = |a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)| (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01 && (a.2 - b.2).abs() < 0.01 && (a.3 - b.3).abs() < 0.01;
    let win = l.rect_of(sub(K_DLG, "win")).unwrap();
    assert!(near(win, (106.0, 185.5, 388.0, 149.0)), "dlg {win:?}");
    let open = l.rect_of(K_DLG_OPEN).unwrap();
    assert!(near(open, (297.625, 288.5, 78.6094, 30.0)), "open {open:?}");
    let change = l.rect_of(K_DLG_CHANGE).unwrap();
    assert!(near(change, (384.2344, 288.5, 91.7656, 30.0)), "change {change:?}");
    let body = l.rect_of(K_DLG_BODY).unwrap();
    assert!(near((body.0, body.1, body.2, 32.0), (124.0, 238.5, 352.0, 32.0)), "dpath {body:?}");
}

#[test]
fn hover_lifts_the_picture() {
    // `.shot.hv .sth{transform:translateY(-2px)}` after the .2 s transition
    let mut p = page(0.0);
    build(&mut p, 0.0);
    let k = tile_key(&p, 0);
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let views = std::mem::take(&mut p.shots);
    {
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        let t = p.tile(&mut cx, 0, &views[0]);
        assert_eq!(t.children[0].translate, (0.0, 0.0));
    }
    st.hover = vec![sub(k, "th"), k];
    {
        let mut cx = Cx::new(10.0, false, &g, &mut st);
        p.tile(&mut cx, 0, &views[0]);
    }
    let mut cx = Cx::new(400.0, false, &g, &mut st);
    let t = p.tile(&mut cx, 0, &views[0]);
    assert_eq!(t.children[0].translate, (0.0, -2.0));
}

/// Order 019 proof (run by hand, never in the normal suite): the lightbox's boxes painted with the app's painter over the
/// drawing's screen without the lightbox (`<dir>\before.png`), the picture = the drawing's own canvas pixels cropped from
/// `<dir>\after.png` (both made with dom_dump in the reference Chromium); writes `<dir>\app.png` to compare with after.png.
/// `set BU_LB_DIR=<dir> && cargo test -p bu-app lightbox_proof -- --ignored`
#[test]
#[ignore]
fn lightbox_proof() {
    let Ok(dir) = std::env::var("BU_LB_DIR") else { return };
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
    }
    let before = crate::png::load_png(&format!("{dir}/before.png")).unwrap();
    let after = crate::png::load_png(&format!("{dir}/after.png")).unwrap();
    // the drawing's canvas: (299.5, 122.5) 1321 x 743 on the 1920 x 1032 screen (above its 48 px taskbar)
    let (cx0, cy0, cw, ch) = (300u32, 123u32, 1321u32, 743u32);
    let mut pic = vec![0u8; (cw * ch * 4) as usize];
    for y in 0..ch {
        let s = (((cy0 + y) * after.w + cx0) * 4) as usize;
        pic[(y * cw * 4) as usize..((y + 1) * cw * 4) as usize].copy_from_slice(&after.data[s..s + (cw * 4) as usize]);
    }
    let ii = sk::ImageInfo::new((cw as i32, ch as i32), sk::ColorType::BGRA8888, sk::AlphaType::Premul, Some(sk::ColorSpace::new_srgb()));
    let pic = sk::images::raster_from_data(&ii, sk::Data::new_copy(&pic), (cw * 4) as usize).unwrap();
    let base = crate::png::to_image(&before).unwrap();
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(10_000.0, false, &g, &mut st);
    let el = lightbox::lightbox(&mut cx, key("shot.lb"), &pic, (1920, 1080), "Screenshot 2026-10-06 21-34.png", r"1920 × 1080 · Pictures\Screenshots", (1920.0, 1032.0), 0.0, None, None, Some(base.clone()));
    let laid = Laid::new(&g, el, 1920.0, Some(1032.0));
    let mut surf = sk::surfaces::raster_n32_premul((before.w as i32, before.h as i32)).unwrap();
    surf.canvas().draw_image(&base, (0, 0), None);
    let icons = crate::icons::Icons::new();
    g.begin(surf.canvas());
    laid.paint(&g, &icons, 0.0, 0.0, Some(&base));
    g.end();
    let px = crate::png::from_surface(&mut surf);
    crate::png::save_png(&px, &format!("{dir}/app.png")).unwrap();
}

/// REVIEW 019 25f908d HOLD 2: while the lightbox is up the page holds an (empty) popup, so the frame sends Esc to
/// `popup_dismiss`, which closes the lightbox; once it has gone (`tick`) the popup goes too. (A test never opens the real
/// window: `lbhost::is_open()` is false here, as after the closing fade.)
#[test]
fn esc_reaches_the_lightbox_through_the_page_popup() {
    let mut p = page(0.0);
    let popup = |p: &mut Screenshots| {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(100.0, false, &g, &mut st);
        p.popup(&mut cx).is_some()
    };
    assert!(!popup(&mut p));
    p.lb_up = true;
    assert!(popup(&mut p), "no popup while the lightbox is up: the frame's Esc would not reach the page");
    p.popup_dismiss();
    assert!(p.tick(16.0));
    assert!(!p.lb_up);
    assert!(!popup(&mut p));
    // closing the page (the menu closes) takes the lightbox along and keeps nothing
    p.lb_up = true;
    p.close();
    assert!(!p.lb_up && !lbhost::is_open());
}

/// the owner (test build 1): "i can't set my bind whatsoever for screenshots" - the field now listens through the keys
/// manager, binds the key the user presses, shows it, and its × clears it; the action is in the app's key list.
#[test]
fn the_screenshot_key_field_binds_through_the_keys_manager() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut p = page(0.0);
    {
        let mut cx = Cx::new(0.0, false, &g, &mut st).for_page("shot");
        assert!(cx.actions().iter().any(|a| a.id == KEY_ACTION && a.name == "Screenshot" && a.page == "shot"));
        p.event(&Ev::Click(K_KEY), &mut cx);
        let (set, listening, _) = cx.key_field(KEY_ACTION);
        assert!(set.is_none() && listening.is_some(), "a click on the field listens");
    }
    // F9 (no modifier held on the real keyboard: a lone key that types nothing)
    if crate::keys::real::mods_now().is_empty() {
        assert!(crate::services::key_message(true, 0x78, 0));
        // (the key is taken on its press; its release is the menu's own again)
        let _ = crate::services::key_message(false, 0x78, 0);
        let mut cx = Cx::new(10.0, false, &g, &mut st).for_page("shot");
        let (set, listening, err) = cx.key_field(KEY_ACTION);
        assert_eq!(set.as_deref(), Some("F9"), "{err:?}");
        assert!(listening.is_none());
        let _ = p.build(&mut cx);
        p.event(&Ev::Click(sub(K_KEY, "clr")), &mut cx);
        assert!(cx.key_field(KEY_ACTION).0.is_none(), "the × clears the key");
    }
    p.close();
    crate::services::shutdown();
}

/// Order 029 review: the menu closing while the key field listens (no Blur comes) stops the listening - else every app
/// key stays paused.
#[test]
fn closing_the_page_while_the_key_field_listens_stops_it() {
    crate::services::init(windows::Win32::Foundation::HWND::default(), true);
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut p = page(0.0);
    {
        let mut cx = Cx::new(0.0, false, &g, &mut st).for_page("shot");
        p.event(&Ev::Click(K_KEY), &mut cx);
        assert!(cx.key_field(KEY_ACTION).1.is_some());
    }
    p.close();
    assert!(crate::services::with(|s| s.listening.is_none()).unwrap(), "not listening after the close");
    crate::services::shutdown();
}

/// Order 042 proof picture (`BU_PIC_OUT=<folder> cargo test -p bu-app proof_042 -- --ignored`): the gallery with a picture's right-click menu
/// open, then its Delete confirm.
#[test]
#[ignore]
fn proof_042_gallery_right_click_menu() {
    if std::env::var("BU_PIC_OUT").is_err() {
        return;
    }
    let mut p = page(0.0);
    build(&mut p, 0.0);
    let k = tile_key(&p, 1);
    ev(&mut p, Ev::Context(k, 250.0, 160.0), 10.0);
    for (name, ask) in [("gallery_menu", false), ("gallery_delete", true)] {
        if let Some(c) = p.ctx.as_mut() {
            c.ask = ask;
        }
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(5000.0, false, &g, &mut st);
        let kids = p.build(&mut cx);
        let pop = p.popup(&mut cx).unwrap();
        let page = El::block().abs(0.0, PAGE_TOP, f32::NAN, f32::NAN).w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
        let root = El::block().w(WIN_W).h(WIN_H).child(page).child(pop);
        crate::ui::lay::proof_png(root, WIN_W, 420.0, 2.0, &format!("{name}.png"));
    }
}

/// Order 045 item 9: while the lightbox is up, Space and Enter close it like Esc (and nothing else hears them).
#[test]
fn space_and_enter_close_the_lightbox() {
    for vk in [0x20u16, 0x0D] {
        let mut p = page(0.0);
        p.lb_up = true;
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(10.0, false, &g, &mut st);
        p.event(&Ev::Key(crate::ui::cx::PAGE, vk), &mut cx);
        assert!(cx.used, "the key is the lightbox's");
    }
}
