//! One capture, from the key press to the finished shot (coStart / coFinish / coWhole / coSnapNow): the frozen picture taken
//! the moment the overlay opens, the model, and what each `Out` does with bu-screenshot - copy, save, the gallery. Generic
//! over the OS layer, so every action is tested against bu-screenshot's FAKE (no screen, no clipboard, no files of the PC).

use bu_screenshot::{Frozen, Image, Rect, ScreenshotOs, Screenshots, Shot, Target};

use super::compose;
use super::model::{Ann, Finish, Keep, Model, Out, Pt};

/// A Copy / Save to carry out off the UI thread (Order 048: grabbing, the PNG encode, the file and the clipboard can take
/// long, and the overlay's topmost windows must never wait on them): everything the picture needs, owned.
pub struct FinishJob {
    pub kind: Finish,
    /// the picture to crop (None = this moment: Live, or a Snap still on its way - the job grabs the screen first)
    pub frozen: Option<Frozen>,
    /// the box (desktop px)
    pub sel: Rect,
    pub anns: Vec<Ann>,
}

/// Carry out a Copy / Save (any thread; `svc` = that thread's own engine). The same steps the overlay's `apply` takes.
pub fn run_finish<O: ScreenshotOs>(svc: &Screenshots<O>, job: FinishJob) -> bu_screenshot::Result<Done> {
    let frozen = match job.frozen {
        Some(f) => f,
        None => svc.capture_all()?,
    };
    let r = job.sel.intersect(&frozen.area).ok_or(bu_screenshot::Error::BadRegion("is outside the desktop"))?;
    let image = compose::compose(&frozen, &r, &job.anns)?;
    drop(frozen);
    let size = format!("{}×{}", image.width, image.height);
    match job.kind {
        Finish::Copy => {
            // Copy: the clipboard, and the gallery (the drawing: "Both land in the gallery" - a gallery entry is a saved file)
            svc.copy(&image)?;
            let shot = Some(svc.save(&image)?);
            Ok(Done { image, shot, title: "Screenshot copied", sub: size, flash: None })
        }
        Finish::Save => {
            let shot = svc.save(&image)?;
            let dir = shot.path.parent().map(|p| p.display().to_string()).unwrap_or_default();
            Ok(Done { image, shot: Some(shot), title: "Screenshot saved", sub: format!("{size} · {dir}"), flash: None })
        }
    }
}

/// A shot that is done: its picture, its gallery entry, and the capture toast's two lines.
#[derive(Clone, Debug)]
pub struct Done {
    pub image: Image,
    pub shot: Option<Shot>,
    /// "Screenshot copied" / "Screenshot saved"
    pub title: &'static str,
    /// "620×400" (Copy), "620×400 · <folder>" (Save), "1920×1080 · Monitor 1" (a click)
    pub sub: String,
    /// the whole screen(s) taken with a click: the soft white flash over them (desktop px)
    pub flash: Option<Rect>,
}

/// What happened after an input.
#[derive(Debug)]
pub enum Step {
    /// the overlay stays (redraw)
    Stay,
    /// closed without a shot
    Closed,
    /// closed with a shot
    Shot(Done),
}

pub struct Capture<O: ScreenshotOs> {
    pub svc: Screenshots<O>,
    /// the picture the overlay shows and crops from: the moment of the key press (or of the last Snap)
    pub frozen: Frozen,
    pub model: Model,
}

impl<O: ScreenshotOs> Capture<O> {
    /// The Screenshot key: every monitor is captured at once (frozen by default), then the overlay opens on it.
    pub fn start(svc: Screenshots<O>, pointer: Pt, now: f64, keep: Option<Keep>) -> bu_screenshot::Result<Self> {
        let frozen = svc.capture_all()?;
        Ok(Self::with_frozen(svc, frozen, pointer, now, keep))
    }

    /// The same with the picture already taken (Order 048: the overlay grabs on a worker thread, then opens on it).
    pub fn with_frozen(svc: Screenshots<O>, frozen: Frozen, pointer: Pt, now: f64, keep: Option<Keep>) -> Self {
        let model = Model::new(frozen.monitors.clone(), pointer, now, keep);
        Capture { svc, frozen, model }
    }

    /// Copy / Save as a job for another thread (None = no box yet: nothing to take). `fresh` = a new picture is still on
    /// its way (a Snap / Live off on a worker): the job takes this moment itself instead of the old picture. The frozen
    /// picture moves into the job (the overlay only fades out after a finish; its monitors' slices are separate copies).
    pub fn finish_job(&mut self, kind: Finish, fresh: bool) -> Option<FinishJob> {
        let sel = self.model.sel.and_then(|s| s.rect())?;
        let frozen = if self.model.live || fresh {
            None
        } else {
            let empty = Frozen { area: self.frozen.area, image: Image { width: 0, height: 0, bgra: Vec::new() }, monitors: Vec::new(), color: Vec::new(), timing: Default::default() };
            Some(std::mem::replace(&mut self.frozen, empty))
        };
        Some(FinishJob { kind, frozen, sel, anns: self.model.anns.clone() })
    }

    /// Carry out what the model asked for.
    pub fn apply(&mut self, out: Out) -> bu_screenshot::Result<Step> {
        match out {
            Out::Nothing => Ok(Step::Stay),
            Out::Close => Ok(Step::Closed),
            // Live on: the real screen shows through until a snap; Live off / Snap: this moment becomes the picture
            Out::Live(true) => Ok(Step::Stay),
            Out::Live(false) | Out::Snap => {
                self.frozen = self.svc.capture_all()?;
                Ok(Step::Stay)
            }
            Out::Finish(kind) => {
                let Some(sel) = self.model.sel.and_then(|s| s.rect()) else { return Ok(Step::Stay) };
                if self.model.live {
                    // Live: the picture is this moment (coFinish: deskSnap())
                    self.frozen = self.svc.capture_all()?;
                }
                // the same steps the overlay's worker runs (`run_finish`), here on the calling thread
                let job = FinishJob { kind, frozen: Some(self.frozen.clone()), sel, anns: self.model.anns.clone() };
                Ok(Step::Shot(run_finish(&self.svc, job)?))
            }
        }
    }

    /// The region a Target means (for callers that capture without the overlay, e.g. a later "whole screen" key).
    pub fn target_rect(&self, t: Target) -> Option<Rect> {
        bu_screenshot::service::plan(&self.frozen.monitors, t).ok().map(|p| p.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::screenshots::overlay::model::{Ann, Sel, Tool};
    use bu_screenshot::fake::{Clip, FakeOs};
    use std::path::PathBuf;

    const DATA: &str = r"C:\fake\BoylerUtilities\screenshots";

    fn cap() -> (FakeOs, Capture<FakeOs>) {
        let os = FakeOs::two_monitors();
        let svc = Screenshots::new(os.clone(), PathBuf::from(DATA));
        (os.clone(), Capture::start(svc, (100.0, 100.0), 0.0, None).unwrap())
    }
    fn boxed(c: &mut Capture<FakeOs>) {
        c.model.press((100.0, 100.0), None, 0.0);
        c.model.moved((300.0, 250.0), false);
        c.model.release(0.0);
    }

    #[test]
    fn the_key_freezes_every_monitor_at_once() {
        let (os, c) = cap();
        assert_eq!(c.frozen.area, Rect::new(0, 0, 4480, 1440));
        assert_eq!(os.state().captures.len(), 1, "one capture for all monitors");
        assert_eq!(c.model.monitors.len(), 2);
    }

    #[test]
    fn copy_puts_the_box_with_its_marks_on_the_clipboard_and_in_the_gallery() {
        let (os, mut c) = cap();
        boxed(&mut c);
        c.model.pick_tool(Tool::Box, 0.0);
        c.model.press((120.0, 120.0), None, 0.0);
        c.model.moved((200.0, 200.0), false);
        c.model.release(0.0);
        let out = c.model.key(0x43, Some('c'), true, false, false, 0.0);
        let Step::Shot(d) = c.apply(out).unwrap() else { panic!("no shot") };
        assert_eq!((d.image.width, d.image.height, d.title, d.sub.as_str()), (200, 150, "Screenshot copied", "200×150"));
        assert!(matches!(os.state().clipboard, Some(Clip::Picture { .. })));
        let shot = d.shot.unwrap();
        assert_eq!(c.svc.gallery().unwrap()[0].id, shot.id, "it joins the gallery");
        assert_ne!(d.image, c.frozen.crop(&Rect::new(100, 100, 200, 150)).unwrap(), "the mark is in the picture");
        assert_eq!(c.model.anns.len(), 1);
        let _ = Ann::Box { c: 0, a: (0.0, 0.0), b: (0.0, 0.0), s: 1.0 };
    }

    #[test]
    fn save_writes_into_the_screenshots_folder() {
        let (os, mut c) = cap();
        boxed(&mut c);
        let out = c.model.key(0x53, Some('s'), true, false, false, 0.0);
        let Step::Shot(d) = c.apply(out).unwrap() else { panic!("no shot") };
        let path = d.shot.unwrap().path;
        assert!(path.starts_with(r"C:\Users\test\Pictures\Screenshots"), "{path:?}");
        assert!(os.state().files.contains_key(&path));
        assert_eq!(os.state().clipboard, None, "Save does not touch the clipboard");
        assert_eq!(d.sub, r"200×150 · C:\Users\test\Pictures\Screenshots");
        assert_eq!(d.title, "Screenshot saved");
    }

    #[test]
    fn a_click_selects_the_whole_monitor_and_only_copy_takes_it() {
        let (os, mut c) = cap();
        c.model.press((2000.0, 50.0), None, 0.0);
        let out = c.model.release(0.0);
        assert!(matches!(c.apply(out).unwrap(), Step::Stay), "a click never takes the shot (the owner Oct 8)");
        assert_eq!(os.state().clipboard, None);
        let out = c.model.key(0x43, Some('c'), true, false, false, 1.0);
        let Step::Shot(d) = c.apply(out).unwrap() else { panic!("no shot") };
        assert_eq!((d.image.width, d.image.height), (2560, 1440));
        assert_eq!(d.image, os.screen(2), "exactly the monitor's pixels");
    }

    #[test]
    fn a_typed_size_and_a_monitor_button_capture_exactly_that() {
        let (os, mut c) = cap();
        boxed(&mut c);
        c.model.size_start();
        c.model.size_edit = Some("640x360".into());
        c.model.size_commit(true);
        let Step::Shot(d) = c.apply(Out::Finish(Finish::Copy)).unwrap() else { panic!() };
        assert_eq!((d.image.width, d.image.height), (640, 360));
        c.model.pick_preset(2, 0.0);
        let Step::Shot(d) = c.apply(Out::Finish(Finish::Copy)).unwrap() else { panic!() };
        assert_eq!((d.image.width, d.image.height), (4480, 1440), "All = the monitors side by side");
        let _ = os;
    }

    #[test]
    fn frozen_stays_frozen_and_live_takes_the_moment_of_the_snap() {
        let (os, mut c) = cap();
        let before = c.frozen.image.clone();
        os.change_screens();
        boxed(&mut c);
        let Step::Shot(d) = c.apply(Out::Finish(Finish::Copy)).unwrap() else { panic!() };
        assert_eq!(d.image, before.crop(&Rect::new(100, 100, 200, 150)).unwrap(), "frozen = the moment of the key press");
        let out = c.model.set_live(true);
        assert!(matches!(c.apply(out).unwrap(), Step::Stay));
        os.change_screens();
        let out = c.model.snap(1.0);
        assert_eq!(out, Out::Snap);
        c.apply(out).unwrap();
        assert_eq!(c.frozen.image.crop(&Rect::new(0, 0, 1920, 1080)).unwrap(), os.screen(1), "the snap is this moment");
        assert_eq!(os.state().captures.len(), 2);
    }

    #[test]
    fn esc_closes_without_a_shot_and_nothing_is_written() {
        let (os, mut c) = cap();
        boxed(&mut c);
        let out = c.model.key(0x1B, None, false, false, false, 0.0);
        assert!(matches!(c.apply(out).unwrap(), Step::Closed));
        assert!(os.state().files.is_empty());
        assert_eq!(os.state().clipboard, None);
    }

    #[test]
    fn marks_are_clipped_to_the_box() {
        let (_os, mut c) = cap();
        boxed(&mut c);
        c.model.pick_tool(Tool::Pen, 0.0);
        // a stroke from inside to far outside the box
        c.model.press((150.0, 150.0), None, 0.0);
        c.model.moved((900.0, 900.0), false);
        c.model.release(0.0);
        let Step::Shot(d) = c.apply(Out::Finish(Finish::Copy)).unwrap() else { panic!() };
        assert_eq!((d.image.width, d.image.height), (200, 150), "the picture is only the box");
        assert_eq!(c.model.sel, Some(Sel { x: 100, y: 100, w: 200, h: 150 }));
    }

    #[test]
    fn a_failed_capture_is_reported_not_hidden() {
        let os = FakeOs::two_monitors();
        os.state().capture_error = Some(bu_screenshot::Error::Cancelled);
        let svc = Screenshots::new(os.clone(), PathBuf::from(DATA));
        assert!(Capture::start(svc, (0.0, 0.0), 0.0, None).is_err());
    }

    /// Order 048: the job the overlay hands its worker gives exactly the shot `apply` gives, and takes this moment when a
    /// new picture is still on its way.
    #[test]
    fn a_finish_job_on_another_engine_makes_the_same_shot() {
        let (os, mut c) = cap();
        boxed(&mut c);
        let want = c.frozen.crop(&Rect::new(100, 100, 200, 150)).unwrap();
        let job = c.finish_job(Finish::Copy, false).expect("a box");
        assert!(job.frozen.is_some(), "the frozen picture moves into the job");
        let worker = Screenshots::new(os.clone(), PathBuf::from(DATA));
        let d = run_finish(&worker, job).unwrap();
        assert_eq!((d.image.clone(), d.title, d.sub.as_str()), (want, "Screenshot copied", "200×150"));
        assert!(matches!(os.state().clipboard, Some(Clip::Picture { .. })));
        assert!(d.shot.is_some(), "Copy joins the gallery too");
        // a Snap on its way: the job grabs the screen itself
        let job = c.finish_job(Finish::Save, true).unwrap();
        assert!(job.frozen.is_none());
        os.change_screens();
        let d = run_finish(&worker, job).unwrap();
        assert_eq!(d.image, os.screen(1).crop(&Rect::new(100, 100, 200, 150)).unwrap(), "this moment, not the old picture");
        assert_eq!(d.title, "Screenshot saved");
        // no box: no job
        let (_os, mut c2) = cap();
        assert!(c2.finish_job(Finish::Copy, false).is_none());
    }
}
