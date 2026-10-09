//! The "Get more sounds" window of the Keyboard tab (Order 061): the community packs of mechvibes.com, each with its name,
//! tags and size, a play button to hear it first (Order 076) and one click that downloads and imports it. A search box
//! narrows the list. The data and jobs are `gallery.rs`.

use super::gallery::{self, Gal, JOB_GET, JOB_LIST, JOB_PREVIEW};
use super::*;
use crate::gfx::Font;
use crate::ui::el::lh;
use crate::ui::pieces::button::{self, Kind as BKind};
use crate::ui::pieces::{dialog, group, search};
use crate::ui::{ACC, FG, FG2, FG3};
use bu_keysound::gallery as site;
use std::sync::Arc;
use taffy::style::AlignItems;

const K_GET: Key = key("kbd.get");
pub(super) const K_GROW: Key = key("kbd.grow");
pub(super) const K_GDONE: Key = key("kbd.gdone");
/// How many rows the list can hold (the site lists about 100 packs on one page).
const MAX_ROWS: usize = 300;

/// The one short line under the title.
const NOTE: &str = "From mechvibes.com \u{b7} licences vary";

/// Does the pack match what was typed in the search box (its name or one of its tags, any letter case)?
pub(super) fn matches(p: &site::Listed, q: &str) -> bool {
    let q = q.trim().to_lowercase();
    q.is_empty() || p.name.to_lowercase().contains(&q) || p.tags.iter().any(|t| t.to_lowercase().contains(&q))
}

impl Keyboard {
    /// "Get more sounds…": opens the window and asks the site for its list (a click starts it; nothing runs on its own).
    pub(super) fn open_get(&mut self, cx: &mut Cx) {
        self.pop = None;
        self.get_open = true;
        self.opened_at = cx.now;
        self.get_msg = None;
        self.get_q.clear();
        gallery::OPEN.store(true, std::sync::atomic::Ordering::SeqCst);
        if self.test {
            gallery::fixture();
            return;
        }
        let Some(dir) = crate::services::with(|s| glue::packs_dir(s.store.folder())) else { return };
        let fetch: Arc<dyn site::Fetch + Send + Sync> = Arc::new(gallery::WinFetch::new());
        match cx.start_job(JOB_LIST, gallery::list_job(gallery::cache_path(&dir), fetch)) {
            Ok(()) => {}
            // already running: the window just shows it
            Err(e) if e.contains("AlreadyRunning") => {}
            Err(e) => self.get_msg = Some(e),
        }
    }

    pub(super) fn close_get(&mut self, cx: &mut Cx) {
        gallery::OPEN.store(false, std::sync::atomic::Ordering::SeqCst);
        self.get_open = false;
        cx.stop_job(JOB_LIST);
        cx.stop_job(JOB_PREVIEW);
    }

    /// The window (None when closed).
    pub(super) fn getter(&mut self, cx: &mut Cx) -> Option<El> {
        if !self.get_open {
            return None;
        }
        let g = gallery::snapshot();
        // the folder listing once per paint, not once per row
        let have = self.imported();
        let mut rows: Vec<El> = Vec::new();
        let mut shown = 0usize;
        for (i, p) in g.list.iter().take(MAX_ROWS).enumerate() {
            if matches(p, &self.get_q) {
                shown += 1;
                rows.push(self.get_row(cx, i, p, &g, &have));
            }
        }
        if rows.is_empty() {
            let t = match (&g.error, g.busy, g.list.is_empty()) {
                (Some(e), _, true) => e.clone(),
                (None, true, true) => "Reading the pack list from mechvibes.com…".to_string(),
                (_, _, false) => "No sound matches that.".to_string(),
                _ => "No packs found.".to_string(),
            };
            rows.push(El::text(t, Font::new(12.0, 400), FG3(), lh(12.0, 1.35)).wrapping().pad(16.0, 16.0, 16.0, 16.0));
        }
        let (bg, rim) = group::glass();
        let list = cx.scroll_box(sub(K_GET, "rows"), rows).items(AlignItems::STRETCH).max_h(290.0).radius(10.0).bg(bg).inset(&rim).margin(10.0, 0.0, 0.0, 0.0);
        let note = El::text(NOTE, Font::new(11.0, 400), FG3(), lh(11.0, 1.4)).ellipsis().flex1();
        let find = search::search(cx, K_GSEARCH, &self.get_q, "Search sounds", true);
        let top = El::row().center().gap(10.0).margin(0.0, 0.0, 0.0, 2.0).child(note).child(find);
        let mut status = String::new();
        if g.busy && !g.list.is_empty() {
            status = if g.status.is_empty() { "Reading the list…".into() } else { g.status.clone() };
        } else if let (Some(e), false) = (&g.error, g.list.is_empty()) {
            status = e.clone();
        } else if !g.list.is_empty() {
            status = if self.get_q.trim().is_empty() { format!("{} sounds", g.list.len()) } else { format!("{shown} of {} sounds", g.list.len()) };
        }
        if let Some(m) = &self.get_msg {
            status = m.clone();
        }
        let foot_text = El::text(status, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).ellipsis().flex1();
        let done = button::cbtn(cx, K_GDONE, "Done", BKind::Primary, false, false, 76.0);
        let foot = El::row().center().gap(8.0).margin(12.0, 0.0, 0.0, 0.0).child(foot_text).child(done);
        Some(dialog::dialog(cx, K_GET, 460.0, "Get more sounds", vec![top, list, foot], vec![], true, self.opened_at))
    }

    fn get_row(&mut self, cx: &mut Cx, i: usize, p: &site::Listed, g: &Gal, have: &[String]) -> El {
        let installed = g.installed.get(&p.id).filter(|n| self.test || have.contains(n)).cloned();
        let getting = g.getting.as_ref().filter(|(id, _)| *id == p.id).map(|(_, t)| t.clone());
        let size = g.sizes.get(&p.id).map(|s| site::size_text(*s));
        let mut sub_parts: Vec<String> = Vec::new();
        if p.pre_installed {
            sub_parts.push("comes with Mechvibes".into());
        }
        sub_parts.extend(p.tags.iter().cloned());
        sub_parts.push(size.unwrap_or_else(|| if g.busy { "size…".into() } else { String::new() }));
        sub_parts.retain(|s| !s.is_empty());
        let name = El::text(p.name.clone(), Font::new(12.5, 400), FG(), lh(12.5, 1.35)).ellipsis();
        let line = El::text(sub_parts.join(" \u{b7} "), Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0);
        let left = El::col().flex1().child(name).child(line);
        let busy_other = g.getting.is_some();
        let hearing = g.previewing.as_deref() == Some(p.id.as_str());
        // the play button: hear the pack before getting it (nothing is kept); one thing at a time
        let play = if hearing {
            El::text("Loading…", Font::new(11.0, 400), FG3(), lh(11.0, 1.35)).none()
        } else {
            let b = button::icon_btn(cx, idx(K_GPLAY, i), "play", 10.0, 1.0).title("Hear it first");
            if busy_other || g.previewing.is_some() {
                b.opacity(0.4).no_hit()
            } else {
                b
            }
        };
        let in_use = installed.as_ref().is_some_and(|n| self.prefs.s.pack == Pack::Imported(n.clone()));
        let right = if let Some(t) = getting {
            El::text(t, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).none()
        } else if in_use {
            El::text("\u{2713} In use", Font::new(11.5, 600), ACC(), lh(11.5, 1.35)).none()
        } else if installed.is_some() {
            El::text("Installed", Font::new(11.5, 600), FG2(), lh(11.5, 1.35)).none()
        } else {
            button::cbtn(cx, idx(K_GROW, i), "Get", BKind::Ghost, true, busy_other, 0.0)
        };
        El::row().center().gap(8.0).min_h(40.0).pad(5.0, 10.0, 5.0, 12.0).child(left).child(play).child(right)
    }

    /// A click inside the window. True = it was ours.
    pub(super) fn get_clicked(&mut self, k: Key, cx: &mut Cx) -> bool {
        if !self.get_open {
            return false;
        }
        if k == K_GDONE || k == sub(K_GET, "x") || k == sub(K_GET, "out") {
            self.close_get(cx);
            return true;
        }
        if k == sub(K_GSEARCH, "x") {
            self.get_q.clear();
            return true;
        }
        if k == K_GSEARCH {
            return true;
        }
        for i in 0..MAX_ROWS {
            if k == idx(K_GROW, i) {
                let g = gallery::snapshot();
                if g.getting.is_some() {
                    return true;
                }
                if let Some(p) = g.list.get(i).cloned() {
                    self.start_get(p, cx);
                }
                return true;
            }
            if k == idx(K_GPLAY, i) {
                let g = gallery::snapshot();
                if g.getting.is_some() || g.previewing.is_some() {
                    return true;
                }
                if let Some(p) = g.list.get(i).cloned() {
                    self.start_preview(p, cx);
                }
                return true;
            }
        }
        false
    }

    fn start_get(&mut self, p: site::Listed, cx: &mut Cx) {
        self.get_msg = None;
        if self.test {
            // test copies never touch the network: the pack counts as installed under its own name
            return;
        }
        let Some(dir) = crate::services::with(|s| glue::packs_dir(s.store.folder())) else { return };
        let fetch: Arc<dyn site::Fetch + Send + Sync> = Arc::new(gallery::WinFetch::new());
        let name = p.name.clone();
        match cx.start_job(JOB_GET, gallery::get_job(p, gallery::cache_path(&dir), dir, fetch)) {
            Ok(()) => cx.toast(&format!("Downloading {name}…")),
            Err(e) => self.get_msg = Some(e),
        }
    }

    /// The play button of a row: the pack is fetched into memory, a few keys of it are played, nothing is kept.
    fn start_preview(&mut self, p: site::Listed, cx: &mut Cx) {
        self.get_msg = None;
        if self.test {
            return;
        }
        let fetch: Arc<dyn site::Fetch + Send + Sync> = Arc::new(gallery::WinFetch::new());
        match cx.start_job(JOB_PREVIEW, gallery::preview_job(p, fetch, self.prefs.s.clone())) {
            Ok(()) => {}
            Err(e) => self.get_msg = Some(e),
        }
    }

    /// A preview that could not be played says why (once).
    pub(super) fn follow_preview(&mut self, cx: &mut Cx) {
        let Some(v) = cx.job(JOB_PREVIEW) else { return };
        let Some(end) = v.end.clone() else { return };
        if !first_end(JOB_PREVIEW, v.id) {
            return;
        }
        if let crate::jobs::End::Failed(e) = end {
            self.get_msg = Some(format!("Can't play it: {e}"));
        }
    }

    /// The download job ended: the new pack becomes the sound (like an import), or the reason is shown.
    pub(super) fn follow_get(&mut self, cx: &mut Cx) {
        let Some(v) = cx.job(JOB_GET) else { return };
        let Some(end) = v.end.clone() else { return };
        if !first_end(JOB_GET, v.id) {
            return;
        }
        match end {
            crate::jobs::End::Done(name) => {
                self.import_msg = None;
                self.get_msg = None;
                self.prefs.s.pack = Pack::Imported(name.clone());
                self.save();
                cx.toast(&format!("{name} downloaded \u{b7} it is the sound now"));
            }
            crate::jobs::End::Failed(e) => {
                self.get_msg = Some(format!("Not downloaded: {e}"));
                cx.toast(&format!("Not downloaded: {e}"));
            }
            crate::jobs::End::Stopped => {}
        }
    }
}
