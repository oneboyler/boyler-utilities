//! The engine against the fake OS: every capture target, Live, Copy, Save (names, folder, "Change path"), and every gallery
//! action (list, prune, thumbnails, delete to the Recycle Bin, show in folder, drag files, copy N), plus the error paths.

use std::path::PathBuf;
use std::time::Duration;

use bu_screenshot::encode;
use bu_screenshot::fake::{mon, Clip, FakeOs};
use bu_screenshot::service::{plan, THUMB_H, THUMB_W};
use bu_screenshot::{ColorPath, Error, Image, Method, Rect, Screenshots, Target};

const DATA: &str = r"C:\fake\appdata\screenshots";

fn engine() -> (FakeOs, Screenshots<FakeOs>) {
    let os = FakeOs::two_monitors();
    (os.clone(), Screenshots::new(os, DATA))
}

fn shot_image() -> Image {
    let mut img = Image::black(64, 36);
    for y in 0..36 {
        for x in 0..64 {
            img.set_pixel(x, y, [x as u8 * 3, y as u8 * 7, (x ^ y) as u8, 255]);
        }
    }
    img
}

// ---------- capture ----------

#[test]
fn capture_all_is_every_monitor_side_by_side_at_exact_resolution() {
    let (os, s) = engine();
    let f = s.capture_all().unwrap();
    assert_eq!(f.area, Rect::new(0, 0, 4480, 1440));
    assert_eq!((f.image.width, f.image.height), (4480, 1440));
    let (m1, m2) = (os.screen(1), os.screen(2));
    for (x, y) in [(0, 0), (1919, 1079), (777, 333)] {
        assert_eq!(f.image.pixel(x, y), m1.pixel(x, y));
    }
    for (x, y) in [(0, 0), (2559, 1439), (1000, 1200)] {
        assert_eq!(f.image.pixel(1920 + x, y), m2.pixel(x, y));
    }
    assert_eq!(f.image.pixel(5, 1300), [0, 0, 0, 255], "gap under the 1080p monitor is black");
    assert_eq!(f.monitor(1).unwrap(), m1, "monitor 1 out of the frozen picture = its exact pixels, none missing");
    assert_eq!(f.monitor(2).unwrap(), m2);
    assert_eq!(os.state().captures, vec![Method::DesktopDuplication], "one capture call for all monitors");
}

#[test]
fn capture_one_monitor_grabs_only_it() {
    let (os, s) = engine();
    let f = s.capture(Target::Monitor(2)).unwrap();
    assert_eq!(f.area, Rect::new(1920, 0, 2560, 1440));
    assert_eq!(f.image, os.screen(2));
    assert_eq!(f.color, vec![(2, ColorPath::Sdr)]);
    assert!(matches!(s.capture(Target::Monitor(3)), Err(Error::NoMonitor(3))));
}

#[test]
fn capture_region_crosses_monitors_and_is_clamped() {
    let (_os, s) = engine();
    let all = s.capture_all().unwrap();
    let r = Rect::new(1900, 100, 50, 40);
    let f = s.capture(Target::Region(r)).unwrap();
    assert_eq!(f.area, r);
    assert_eq!(f.image, all.crop(&r).unwrap());
    assert_eq!(f.color.len(), 2, "both monitors were grabbed");
    // Partly off the desktop → clamped.
    let f = s.capture(Target::Region(Rect::new(-10, -10, 30, 30))).unwrap();
    assert_eq!(f.area, Rect::new(0, 0, 20, 20));
    // Only one monitor touched → only it is grabbed.
    let f = s.capture(Target::Region(Rect::new(10, 10, 100, 100))).unwrap();
    assert_eq!(f.color, vec![(1, ColorPath::Sdr)]);
}

#[test]
fn bad_regions_are_refused() {
    let (_os, s) = engine();
    assert!(matches!(s.capture(Target::Region(Rect::new(0, 0, 3, 100))), Err(Error::BadRegion(_))));
    assert!(matches!(s.capture(Target::Region(Rect::new(9000, 0, 100, 100))), Err(Error::BadRegion(_))));
    // Inside the bounding box but in the gap under the smaller monitor: no monitor there.
    assert!(matches!(s.capture(Target::Region(Rect::new(10, 1200, 100, 100))), Err(Error::BadRegion(_))));
}

#[test]
fn frozen_crop_outside_is_an_error_not_a_panic() {
    let (_os, s) = engine();
    let f = s.capture(Target::Monitor(1)).unwrap();
    assert!(matches!(f.crop(&Rect::new(1900, 0, 100, 10)), Err(Error::BadRegion(_))));
    assert!(matches!(f.monitor(9), Err(Error::NoMonitor(9))));
}

#[test]
fn chosen_method_is_used_and_unsupported_is_reported() {
    let os = FakeOs::two_monitors();
    let s = Screenshots::new(os.clone(), DATA).with_method(Method::GraphicsCapture);
    s.capture_all().unwrap();
    assert_eq!(os.state().captures, vec![Method::GraphicsCapture]);
    os.state().unsupported.push(Method::GraphicsCapture);
    assert!(matches!(s.capture_all(), Err(Error::MethodUnsupported(_))));
    assert!(matches!(s.live(Target::All), Err(Error::MethodUnsupported(_))));
}

#[test]
fn capture_failure_comes_back_typed() {
    let (os, s) = engine();
    os.state().capture_error = Some(Error::NoFrame { monitor: 2, ms: 1000 });
    assert_eq!(s.capture_all().unwrap_err(), Error::NoFrame { monitor: 2, ms: 1000 });
    assert!(s.capture_all().is_ok(), "next capture works again");
}

#[test]
fn hdr_monitor_reports_its_colour_path() {
    let os = FakeOs::with_monitors(vec![mon(0, 0, 100, 50, true, true), mon(100, 0, 100, 50, false, false)]);
    let s = Screenshots::new(os, DATA);
    let f = s.capture_all().unwrap();
    assert_eq!(f.color, vec![(1, ColorPath::HdrConverted), (2, ColorPath::Sdr)]);
}

#[test]
fn plan_lists_the_monitors_each_target_needs() {
    let os = FakeOs::two_monitors();
    let mons = os.state().monitors.clone();
    assert_eq!(plan(&mons, Target::All).unwrap().1.len(), 2);
    assert_eq!(plan(&mons, Target::Monitor(1)).unwrap().0, Rect::new(0, 0, 1920, 1080));
    assert_eq!(plan(&mons, Target::Region(Rect::new(2000, 5, 10, 10))).unwrap().1[0].number, 2);
}

// ---------- Live ----------

#[test]
fn live_waits_for_change_and_snaps_the_current_moment() {
    let (os, s) = engine();
    let mut live = s.live(Target::All).unwrap();
    assert_eq!(live.area(), Rect::new(0, 0, 4480, 1440));
    let before = live.snap().unwrap();
    assert!(!live.wait_frame(Duration::from_millis(1)).unwrap(), "nothing moved");
    os.change_screens();
    assert!(live.wait_frame(Duration::from_millis(1)).unwrap(), "screen changed");
    let after = live.snap().unwrap();
    assert_ne!(before.image, after.image);
    assert_eq!(after.monitor(2).unwrap(), os.screen(2), "the snap is exactly what is on screen now");
    assert!(!live.wait_frame(Duration::from_millis(1)).unwrap(), "no further change");
}

// ---------- output ----------

#[test]
fn copy_puts_png_and_dib_of_the_exact_pixels() {
    let (os, s) = engine();
    let img = shot_image();
    s.copy(&img).unwrap();
    let Some(Clip::Picture { png, dib }) = os.state().clipboard.clone() else { panic!("no picture on the clipboard") };
    assert_eq!(encode::decode_png(&png).unwrap(), img);
    assert_eq!(dib, encode::dib_bytes(&img));
    assert!(os.state().files.keys().all(|p| !p.starts_with(r"C:\Users\test\Pictures")), "copy writes no file");
}

#[test]
fn save_writes_named_png_into_the_default_folder_and_the_gallery() {
    let (os, s) = engine();
    let img = shot_image();
    let shot = s.save(&img).unwrap();
    assert_eq!(shot.path, PathBuf::from(r"C:\Users\test\Pictures\Screenshots\Screenshot 2026-10-08 01-36.png"));
    assert_eq!((shot.width, shot.height), (64, 36));
    let bytes = os.state().files.get(&shot.path).cloned().unwrap();
    assert_eq!(encode::decode_png(&bytes).unwrap(), img, "saved pixels are exact");
    assert_eq!(s.gallery().unwrap(), vec![shot.clone()]);
    assert_eq!(shot.saved_unix_ms(), 1_791_416_165_000);
}

#[test]
fn second_shot_in_the_same_minute_gets_a_number_and_goes_first() {
    let (os, s) = engine();
    let a = s.save(&shot_image()).unwrap();
    let b = s.save(&shot_image()).unwrap();
    assert!(b.path.ends_with("Screenshot 2026-10-08 01-36 (2).png"));
    assert!(b.id > a.id, "unique id even at the same millisecond");
    os.state().unix_ms += 60_000;
    os.state().time.minute = 37;
    let c = s.save(&shot_image()).unwrap();
    assert!(c.path.ends_with("Screenshot 2026-10-08 01-37.png"));
    let ids: Vec<u64> = s.gallery().unwrap().iter().map(|x| x.id).collect();
    assert_eq!(ids, vec![c.id, b.id, a.id], "newest first");
}

#[test]
fn change_path_saves_at_once_new_shots_go_there_old_ones_stay() {
    let (os, s) = engine();
    let old = s.save(&shot_image()).unwrap();
    let new_dir = PathBuf::from(r"D:\Shots");
    assert!(matches!(s.set_save_dir(&new_dir), Err(Error::NotAFolder(_))), "folder must exist");
    os.add_dir(&new_dir);
    s.set_save_dir(&new_dir).unwrap();
    assert_eq!(s.saved_dir().unwrap(), Some(new_dir.clone()));
    assert_eq!(s.save_dir().unwrap(), new_dir);
    let new = s.save(&shot_image()).unwrap();
    assert!(new.path.starts_with(&new_dir));
    let g = s.gallery().unwrap();
    assert_eq!(g.len(), 2);
    assert!(g.iter().any(|x| x.path == old.path), "older shot stays where it was");
}

#[test]
fn pick_save_dir_uses_the_picker_and_saves_or_reports_cancel() {
    let (os, s) = engine();
    assert_eq!(s.pick_save_dir().unwrap_err(), Error::Cancelled);
    assert_eq!(s.saved_dir().unwrap(), None, "cancel changes nothing");
    let d = PathBuf::from(r"E:\Pics");
    os.add_dir(&d);
    os.state().pick = Some(d.clone());
    assert_eq!(s.pick_save_dir().unwrap(), d);
    assert_eq!(s.save_dir().unwrap(), d);
}

#[test]
fn open_save_dir_opens_the_current_folder() {
    let (os, s) = engine();
    s.open_save_dir().unwrap();
    assert_eq!(os.state().opened, vec![PathBuf::from(r"C:\Users\test\Pictures\Screenshots")]);
}

// ---------- gallery ----------

#[test]
fn thumbnails_fit_the_tile_and_are_rebuilt_when_missing() {
    let (os, s) = engine();
    let mut big = Image::black(1920, 1080);
    big.set_pixel(0, 0, [255, 255, 255, 255]);
    let shot = s.save(&big).unwrap();
    let t = s.thumbnail(shot.id).unwrap();
    assert_eq!((t.width, t.height), (THUMB_W, THUMB_H));
    let thumb_path = PathBuf::from(DATA).join("thumbs").join(format!("{}.png", shot.id));
    assert!(os.state().files.contains_key(&thumb_path), "made at save time");
    os.state().files.remove(&thumb_path);
    assert_eq!(s.thumbnail(shot.id).unwrap(), t, "rebuilt from the PNG");
    assert!(os.state().files.contains_key(&thumb_path), "and cached again");
    assert!(matches!(s.thumbnail(424242), Err(Error::UnknownShot(424242))));
}

#[test]
fn load_gives_the_full_picture() {
    let (_os, s) = engine();
    let img = shot_image();
    let shot = s.save(&img).unwrap();
    assert_eq!(s.load(shot.id).unwrap(), img);
}

#[test]
fn files_removed_outside_the_app_drop_out_of_the_gallery() {
    let (os, s) = engine();
    let a = s.save(&shot_image()).unwrap();
    let b = s.save(&shot_image()).unwrap();
    os.state().files.remove(&a.path); // deleted in Explorer
    assert_eq!(s.gallery().unwrap(), vec![b.clone()]);
    let thumb_a = PathBuf::from(DATA).join("thumbs").join(format!("{}.png", a.id));
    assert!(!os.state().files.contains_key(&thumb_a), "its thumbnail is removed too");
    assert_eq!(s.gallery().unwrap(), vec![b], "index rewritten");
}

#[test]
fn delete_goes_to_the_recycle_bin_and_out_of_the_gallery() {
    let (os, s) = engine();
    let a = s.save(&shot_image()).unwrap();
    let b = s.save(&shot_image()).unwrap();
    let c = s.save(&shot_image()).unwrap();
    s.delete(&[a.id, c.id]).unwrap();
    let recycled = os.state().recycled.clone();
    assert_eq!(recycled.len(), 2);
    assert!(recycled.contains(&a.path) && recycled.contains(&c.path));
    assert_eq!(s.gallery().unwrap(), vec![b]);
    let thumb_c = PathBuf::from(DATA).join("thumbs").join(format!("{}.png", c.id));
    assert!(!os.state().files.contains_key(&thumb_c));
}

#[test]
fn delete_on_a_drive_without_recycle_bin_deletes_nothing() {
    let (os, s) = engine();
    let net = PathBuf::from(r"\\nas\share");
    os.add_dir(&net);
    s.set_save_dir(&net).unwrap();
    let a = s.save(&shot_image()).unwrap();
    os.state().no_bin.push(net.clone());
    assert_eq!(s.delete(&[a.id]).unwrap_err(), Error::NoRecycleBin(a.path.clone()));
    assert!(os.state().files.contains_key(&a.path), "file still there");
    assert_eq!(s.gallery().unwrap().len(), 1, "still in the gallery");
}

#[test]
fn delete_of_an_already_missing_file_just_removes_the_entry() {
    let (os, s) = engine();
    let a = s.save(&shot_image()).unwrap();
    os.state().files.remove(&a.path);
    s.delete(&[a.id]).unwrap();
    assert!(os.state().recycled.is_empty(), "nothing to recycle");
    assert!(s.gallery().unwrap().is_empty());
}

#[test]
fn unknown_ids_are_refused_before_anything_happens() {
    let (os, s) = engine();
    let a = s.save(&shot_image()).unwrap();
    assert_eq!(s.delete(&[a.id, 99]).unwrap_err(), Error::UnknownShot(99));
    assert!(os.state().recycled.is_empty());
    assert_eq!(s.show_in_folder(&[99]).unwrap_err(), Error::UnknownShot(99));
    assert_eq!(s.drag_paths(&[99]).unwrap_err(), Error::UnknownShot(99));
    assert_eq!(s.copy_shots(&[99]).unwrap_err(), Error::UnknownShot(99));
}

#[test]
fn show_in_folder_and_drag_paths_use_the_shots_files() {
    let (os, s) = engine();
    let a = s.save(&shot_image()).unwrap();
    let d = PathBuf::from(r"D:\Elsewhere");
    os.add_dir(&d);
    s.set_save_dir(&d).unwrap();
    let b = s.save(&shot_image()).unwrap();
    s.show_in_folder(&[a.id, b.id]).unwrap();
    assert_eq!(os.state().shown, vec![vec![b.path.clone(), a.path.clone()]], "both files, newest first");
    assert_eq!(s.drag_paths(&[a.id, b.id]).unwrap(), vec![b.path.clone(), a.path.clone()]);
    os.state().files.remove(&a.path);
    assert_eq!(s.drag_paths(&[a.id, b.id]).unwrap(), vec![b.path], "a vanished file is not dragged");
}

#[test]
fn copy_one_shot_is_the_picture_copy_many_is_the_files() {
    let (os, s) = engine();
    let img = shot_image();
    let a = s.save(&img).unwrap();
    let b = s.save(&img).unwrap();
    s.copy_shots(&[a.id]).unwrap();
    assert!(matches!(os.state().clipboard.clone(), Some(Clip::Picture { png, .. }) if encode::decode_png(&png).unwrap() == img));
    s.copy_shots(&[a.id, b.id]).unwrap();
    assert_eq!(os.state().clipboard.clone(), Some(Clip::Files(vec![b.path.clone(), a.path.clone()])));
    assert!(matches!(s.copy_shots(&[]), Err(Error::BadData(_))));
}

#[test]
fn damaged_index_is_an_error_not_a_crash() {
    let (os, s) = engine();
    os.state().files.insert(PathBuf::from(DATA).join("index.txt"), b"garbage".to_vec());
    assert!(matches!(s.gallery(), Err(Error::BadData(_))));
    assert!(matches!(s.save(&shot_image()), Err(Error::BadData(_))));
}
