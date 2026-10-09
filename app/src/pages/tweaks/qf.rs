//! Tweaks › Quick fixes (menu-v22 `QF`, `qfRow`, `qfGo`; the last group of Tweaks since v21), wired to bu-quickfix.
//! Nothing runs on its own: each fix starts only from its button. The long ones run off the UI thread (TEMP: a plain
//! worker thread / the crate's own RepairRun until Order 014 item 2 hands pages the job runner) and the row shows their
//! progress; the row's line then says how it went.

use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use bu_quickfix::repair::RepairRun;
use bu_quickfix::{cache, gfx, restore, FixError, FixOs};

use crate::gfx::{Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{idx, lh, El, Key};
use crate::ui::pieces::{button, group};
use crate::ui::{ACC, FG2, GREEN, TRK};


pub const K_QF: Key = crate::ui::el::key("tgl.qf");
pub const K_QTIP: Key = crate::ui::el::key("tgl.qf.tip");

/// The drawing's rows: (title, line, button). `QF[]` word for word.
pub const QF: [(&str, &str, &str); 4] = [
    ("Reset graphics driver", "The same as Win + Ctrl + Shift + B · the screen flickers black for a moment", "Reset"),
    ("Repair Windows files", "Windows’ own DISM and sfc scans in one go · takes 10–20 minutes", "Repair"),
    ("Rebuild icon & thumbnail cache", "Fixes blank or wrong icons and previews · restarts File Explorer", "Rebuild"),
    ("Make a restore point", "Only when you press it", "Create"),
];
/// `q.k='repair fix quick'` (+ the group name) for the search.
pub const QF_K: &str = "repair fix quick";

/// The layer for ONE admin fix (Order 039): every admin call of that fix goes to one elevated copy, one prompt. None =
/// the page's own layer (a test copy's fake).
pub type ForOne = Box<dyn Fn(crate::admin::Purpose) -> Arc<dyn FixOs>>;

enum Run {
    Repair(RepairRun),
    /// a one-shot fix on a worker thread: the row's progress text, and its answer (line, toast) or the error
    Once { text: String, rx: Receiver<Result<(String, String), String>> },
}

pub struct Qf {
    os: Arc<dyn FixOs>,
    for_one: Option<ForOne>,
    runs: [Option<Run>; 4],
    /// the row's end line (green tick + text), replacing its own line
    done: [Option<String>; 4],
    /// the restore point row's own line ("Only when you press it · last one …"), read off the UI thread at open
    rp_line: Option<String>,
    rp_rx: Option<Receiver<String>>,
    /// Order 055: counts what the rows show changing (a run's progress step, a run ended, the restore point line, a press) -
    /// `Tweaks::tick` compares this number instead of building the rows' texts twice per tick
    ver: u64,
    /// the Repair run's progress as last seen (phase, tenths of a percent): a new step is a change
    seen: Option<(bu_quickfix::repair::Phase, Option<i32>)>,
}

impl Qf {
    /// With the layer for one admin fix (Windows).
    pub fn with_admin(os: Arc<dyn FixOs>, for_one: ForOne) -> Qf {
        Qf { for_one: Some(for_one), ..Qf::new(os) }
    }

    /// The layer one fix runs on.
    fn os_for(&self, p: crate::admin::Purpose) -> Arc<dyn FixOs> {
        self.for_one.as_ref().map(|f| f(p)).unwrap_or_else(|| self.os.clone())
    }

    pub fn new(os: Arc<dyn FixOs>) -> Qf {
        // the status read (WMI) is a light read, not a job, but never waited for on the UI thread
        let (tx, rx) = channel();
        let o = os.clone();
        let _ = std::thread::Builder::new().name("bu-qf-rp-status".into()).spawn(move || {
            if let Ok(st) = o.restore_status() {
                let _ = tx.send(restore::status_line(o.as_ref(), &st));
            }
            // Order 047: the menu draws the line when it is in (no frames while it is read)
            crate::services::Waker.wake();
        });
        Qf { os, for_one: None, runs: [None, None, None, None], done: [None, None, None, None], rp_line: None, rp_rx: Some(rx), ver: 0, seen: None }
    }

    pub fn needs_admin(i: usize) -> bool {
        bu_quickfix::Fix::ALL[i].needs_admin()
    }

    pub fn running(&self, i: usize) -> bool {
        self.runs[i].is_some()
    }

    /// Order 047 / 055: changes whenever what the rows show changes (progress, end line, own line, running) - the page draws
    /// again only when it moved. A number, not the rows' texts: comparing it costs nothing.
    pub fn version(&self) -> u64 {
        self.ver
    }

    /// Order 047: when to look again while nothing else wakes the menu - a running Repair (its progress wakes the menu
    /// itself; its end does not: looked at twice a second). The one-shot fixes and the status read wake it when they end.
    pub fn wake_at(&self, now: f64) -> Option<f64> {
        self.runs.iter().any(|r| matches!(r, Some(Run::Repair(_)))).then_some(now + 500.0)
    }

    /// The row's line now.
    pub fn line(&self, i: usize) -> String {
        if i == 3 {
            if let Some(l) = &self.rp_line {
                return l.clone();
            }
        }
        QF[i].1.to_string()
    }

    /// Polls the runs; returns the toasts of the runs that ended.
    pub fn poll(&mut self) -> Vec<String> {
        let mut toasts = Vec::new();
        if let Some(rx) = &self.rp_rx {
            match rx.try_recv() {
                Ok(l) => {
                    self.rp_line = Some(l);
                    self.rp_rx = None;
                    self.ver += 1;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.rp_rx = None,
                Err(_) => {}
            }
        }
        // the Repair run's progress (shared with its worker): only a new step counts
        let seen = match &self.runs[1] {
            Some(Run::Repair(r)) => {
                let p = r.progress();
                Some((p.phase, p.percent.map(|v| (v * 10.0) as i32)))
            }
            _ => None,
        };
        if seen != self.seen {
            self.seen = seen;
            self.ver += 1;
        }
        for i in 0..4 {
            let end = match &self.runs[i] {
                Some(Run::Repair(r)) if r.is_finished() => Some(None),
                Some(Run::Once { rx, .. }) => match rx.try_recv() {
                    Ok(v) => Some(Some(v)),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Some(Err("the fix stopped".into()))),
                    Err(_) => None,
                },
                _ => None,
            };
            let Some(v) = end else { continue };
            let run = self.runs[i].take();
            self.ver += 1;
            let v = match (run, v) {
                (Some(Run::Repair(r)), None) => {
                    let rep = r.wait();
                    let now = self.os.local(self.os.now());
                    let line = rep.line(now);
                    let toast = match rep.outcome {
                        bu_quickfix::repair::RepairOutcome::NoProblems => "Windows files checked · no problems found".to_string(),
                        _ => line.trim_start_matches("✓ ").to_string(),
                    };
                    Ok((line, toast))
                }
                (_, Some(v)) => v,
                _ => continue,
            };
            match v {
                Ok((line, toast)) => {
                    self.done[i] = Some(line.trim_start_matches("✓ ").to_string());
                    toasts.push(toast);
                }
                Err(e) => toasts.push(e),
            }
        }
        toasts
    }

    /// The row's button was pressed (the Rebuild row asks first: the page shows the confirm and calls this with
    /// `confirmed`). Returns the toast to show now, if any.
    pub fn press(&mut self, i: usize, confirmed: bool) -> Option<String> {
        self.ver += 1;
        if let Some(r) = &self.runs[i] {
            // Cancel (Repair; a restore point / rebuild can't be stopped half-way: their button is disabled)
            if let Run::Repair(r) = r {
                r.cancel();
            }
            return None;
        }
        if Self::needs_admin(i) && !self.os.is_elevated() {
            return Some(crate::admin::NOT_CHANGED.into());
        }
        match i {
            0 => match gfx::reset_with_chord(self.os.as_ref()) {
                Ok(()) => {
                    self.done[0] = Some("Reset just now".into());
                    Some("Graphics driver reset".into())
                }
                Err(e) => Some(err_text(&e)),
            },
            // Order 047: each step of the progress wakes the menu (no frames in between)
            1 => match RepairRun::start(self.os_for(crate::admin::Purpose::Repair), |_| crate::services::Waker.wake()) {
                Ok(r) => {
                    self.runs[1] = Some(Run::Repair(r));
                    self.done[1] = None;
                    None
                }
                Err(e) => Some(err_text(&e)),
            },
            2 if confirmed => {
                self.spawn(2, "Restarting File Explorer…", |os| {
                    cache::rebuild(os).map(|r| (r.line(), "Icon & thumbnail cache rebuilt".to_string())).map_err(|e| err_text(&e))
                });
                None
            }
            3 => {
                self.spawn(3, "Making a restore point", |os| {
                    let now = os.local(os.now());
                    restore::make_restore_point(os)
                        .map(|o| {
                            let toast = match &o {
                                restore::RestoreOutcome::Made { .. } => "Restore point made · System Restore can go back to it".to_string(),
                                other => other.line(now),
                            };
                            (o.line(now), toast)
                        })
                        .map_err(|e| err_text(&e))
                });
                None
            }
            _ => None,
        }
    }

    fn spawn(&mut self, i: usize, text: &str, f: impl FnOnce(&dyn FixOs) -> Result<(String, String), String> + Send + 'static) {
        let (tx, rx) = channel();
        // the restore point: its reads and the call in one elevated copy (one prompt); the cache rebuild needs no admin
        let os = if Self::needs_admin(i) { self.os_for(crate::admin::Purpose::RestorePoint) } else { self.os.clone() };
        let ok = std::thread::Builder::new()
            .name("bu-quickfix".into())
            .spawn(move || {
                let _ = tx.send(f(os.as_ref()));
                // Order 047: the menu draws the end when it is in
                crate::services::Waker.wake();
            })
            .is_ok();
        if ok {
            self.done[i] = None;
            self.runs[i] = Some(Run::Once { text: text.into(), rx });
        }
    }

    /// (progress text, share 0..1) of a running row.
    fn progress(&self, i: usize) -> Option<(String, f32)> {
        match self.runs[i].as_ref()? {
            Run::Repair(r) => {
                let p = r.progress();
                // the drawing's two steps: DISM 0-48 %, sfc 48-100 % of the bar
                let pc = p.percent.unwrap_or(0.0) / 100.0;
                let share = match p.phase {
                    bu_quickfix::repair::Phase::Dism => 0.48 * pc,
                    bu_quickfix::repair::Phase::Sfc => 0.48 + 0.52 * pc,
                };
                Some((p.text(), share))
            }
            Run::Once { text, .. } => Some((text.clone(), 1.0)),
        }
    }

    /// One row (`qfRow`): `.row.qfr.trow{min-height:52px}` = label (title + shield, its line / progress / end line) and
    /// the button `.btn.qfb` (min-width 80, centred, padding 0 12; "Cancel" while running; disabled while it can't stop).
    pub fn row(&self, cx: &mut Cx, i: usize, first: bool, title: El) -> El {
        let mut ttl = El::row().center().gap(4.0).min_w(0.0).child(title);
        if Self::needs_admin(i) {
            ttl = ttl.child(crate::ui::pieces::tip::rq(cx, idx(K_QTIP, i), crate::ui::pieces::tip::Rq::Adm, 18.0, crate::ui::pieces::tip::texts::ADM, false));
        }
        let mut lbl = El::col().flex1().child(ttl);
        let small = |t: &str| El::text(t, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0);
        let run = self.progress(i);
        if let Some((text, share)) = &run {
            lbl = lbl.child(qfp(text, *share));
        } else if let Some(d) = &self.done[i] {
            lbl = lbl.child(qfok(d).margin(1.0, 0.0, 0.0, 0.0));
        } else {
            lbl = lbl.child(small(&self.line(i)));
        }
        let (label, disabled) = match &self.runs[i] {
            Some(Run::Repair(_)) => ("Cancel", false),
            Some(Run::Once { .. }) => (QF[i].2, true),
            None => (QF[i].2, false),
        };
        let k = idx(K_QF, i);
        // `#sw .btn.qfb{min-width:80px;justify-content:center;padding:0 12px}` `.btn.qfb:disabled{opacity:.5}`
        let mut b = button::btn(cx, k, "", label, false).min_w(80.0).pad(0.0, 12.0, 0.0, 12.0).justify(taffy::style::JustifyContent::CENTER);
        if disabled {
            b = b.opacity(0.5).key(k).no_hit();
        }
        group::row(first, vec![lbl, group::ctl(vec![b])]).min_h(52.0)
    }
}

fn err_text(e: &FixError) -> String {
    match e {
        FixError::NeedsAdmin(_) => crate::admin::NOT_CHANGED.into(),
        other => {
            let s = other.to_string();
            let mut c = s.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    }
}

/// `.qfp{display:flex;align-items:center;gap:8px;margin-top:4px;font-size:11px;color:var(--fg2);font-variant-numeric:tabular-nums;
/// white-space:nowrap}` `.qfbar{width:120px;height:4px;border-radius:2px;background:var(--trk);overflow:hidden}`
/// `.qfbar i{width:<share>;border-radius:inherit;background:var(--acc)}`; the text = "<step> · <n> %".
fn qfp(text: &str, share: f32) -> El {
    let share = share.clamp(0.0, 1.0);
    let bar = El::block()
        .size(120.0, 4.0)
        .none()
        .radius(2.0)
        .bg(TRK())
        .clip()
        .child(El::block().abs(0.0, 0.0, f32::NAN, 0.0).w(120.0 * share).radius(2.0).bg(ACC()));
    El::row().center().gap(8.0).margin(4.0, 0.0, 0.0, 0.0).child(bar).child(El::text(text, Font::new(11.0, 400).tnum(), FG2(), lh(11.0, 1.35)).none())
}

/// `.qfok{display:inline-flex;align-items:center;gap:5px;color:var(--green)}` `svg{width:12px;height:12px;stroke-width:1.8}`
/// `.qfok span{color:var(--fg2)}` inside the row's `small` (11 px).
fn qfok(text: &str) -> El {
    El::row()
        .center()
        .gap(5.0)
        .child(El::icon("dcheck", 12.0, 1.8, GREEN()).none())
        .child(El::text(text, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).ellipsis())
}


#[cfg(test)]
mod tests {
    use super::*;
    use bu_quickfix::fake::FakeFixOs;

    fn wait(q: &mut Qf) -> Vec<String> {
        for _ in 0..400 {
            let t = q.poll();
            if !q.runs.iter().any(|r| r.is_some()) {
                return t;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the fix never ended");
    }

    #[test]
    fn nothing_runs_until_a_button_is_pressed() {
        let f = FakeFixOs::new();
        let mut q = Qf::new(Arc::new(f.clone()));
        let _ = wait(&mut q);
        assert!(f.spawned().is_empty() && f.chords_sent() == 0 && f.create_calls().is_empty());
        assert_eq!(f.events(), Vec::<String>::new());
    }

    /// Order 055: the version moves when a row's picture does (a press, a run's end, the restore point line), not otherwise.
    #[test]
    fn the_version_moves_only_when_the_rows_change() {
        let f = FakeFixOs::new();
        f.add_file("iconcache_32.db", 4096);
        let mut q = Qf::new(Arc::new(f.clone()));
        // (the restore point line comes from its own thread: wait for it first)
        for _ in 0..400 {
            q.poll();
            if q.rp_rx.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let v = q.version();
        q.poll();
        q.poll();
        assert_eq!(q.version(), v, "nothing new: the same number");
        q.press(2, true);
        assert_ne!(q.version(), v, "a press");
        let v = q.version();
        let _ = wait(&mut q);
        assert_ne!(q.version(), v, "the run ended");
    }

    #[test]
    fn reset_graphics_sends_the_chord_only_on_press() {
        let f = FakeFixOs::new();
        let mut q = Qf::new(Arc::new(f.clone()));
        assert_eq!(q.press(0, false).as_deref(), Some("Graphics driver reset"));
        assert_eq!(f.chords_sent(), 1);
        assert_eq!(q.done[0].as_deref(), Some("Reset just now"));
    }

    #[test]
    fn admin_fixes_say_so_when_not_elevated() {
        let f = FakeFixOs::new();
        let mut q = Qf::new(Arc::new(f.clone()));
        assert_eq!(q.press(1, false).as_deref(), Some(crate::admin::NOT_CHANGED));
        assert_eq!(q.press(3, false).as_deref(), Some(crate::admin::NOT_CHANGED));
        assert!(f.spawned().is_empty() && f.create_calls().is_empty());
    }

    #[test]
    fn rebuild_asks_first_then_runs_and_says_how_it_went() {
        let f = FakeFixOs::new();
        f.add_file("iconcache_32.db", 4096);
        let mut q = Qf::new(Arc::new(f.clone()));
        assert_eq!(q.press(2, false), None);
        assert!(!q.running(2) && f.files().len() == 1, "nothing before the confirm");
        q.press(2, true);
        let t = wait(&mut q);
        assert_eq!(t, vec!["Icon & thumbnail cache rebuilt".to_string()]);
        assert_eq!(q.done[2].as_deref(), Some("Rebuilt just now · icons refill as you browse"));
        assert!(f.explorer_running());
    }

    #[test]
    fn restore_point_when_elevated() {
        let f = FakeFixOs::new().elevated();
        let mut q = Qf::new(Arc::new(f.clone()));
        q.press(3, false);
        let t = wait(&mut q);
        assert_eq!(t, vec!["Restore point made · System Restore can go back to it".to_string()]);
        assert_eq!(f.create_calls().len(), 1);
        assert!(q.done[3].as_deref().unwrap_or("").starts_with("Made today, 21:37"));
    }
}
