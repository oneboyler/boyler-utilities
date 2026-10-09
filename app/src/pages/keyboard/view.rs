//! The Keyboard tab's boxes and its input (keyboard-v2.html): key sounds, the apps with their own sound, the keyboard picture
//! with the key's card, the macros and the macro window.

use super::*;
use bu_keysound::PlayOn;
use crate::anim::EASE;
use crate::gfx::{sh, Align, Font, Rgba};
use crate::ui::el::{lh, Cursor, RADIUS_PILL};
use crate::ui::pieces::button::{self, Kind as BKind};
use crate::ui::pieces::dropdown;
use crate::ui::pieces::keyfield::{self, Show};
use crate::ui::pieces::mitems::{self, It, Place, Row};
use crate::ui::pieces::tip::{self, Rq};
use crate::ui::pieces::{self, dialog, group, link, reset, seg, slider, tinput, toggle};
use crate::ui::{cmix, ACC, CTL, CTL_H, FG, FG2, FG3, WIN_H, WIN_W};
use taffy::style::AlignItems;

const TIP_PRIVACY: &str = "Only that a key went down or up is heard - never which key";
const K_TIP: Key = key("kbd.tip");

fn title_of(exe: &str) -> String {
    let stem = exe.strip_suffix(".exe").unwrap_or(exe);
    let mut c = stem.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

impl Keyboard {
    pub(super) fn view(&mut self, cx: &mut Cx) -> Vec<El> {
        let mut kids = vec![pieces::header(self.name(), None)];
        kids.extend(self.keys_group(cx));
        kids.extend(self.sounds(cx));
        kids.extend(self.macros_group(cx));
        kids.push(reset::reset_line(cx, K_RESET, Some("Windows defaults")));
        kids
    }

    // ------------------------------------------------------------------ 1. key sounds (+ different in some apps, one card)

    /// A small "i" with a one-line tip (the page keeps its explanations here instead of in paragraphs).
    fn info(&self, cx: &mut Cx, n: usize, text: &str) -> El {
        tip::rq(cx, idx(K_TIP, n), Rq::Info, 16.0, text, false)
    }

    fn sounds(&mut self, cx: &mut Cx) -> Vec<El> {
        let on = self.prefs.on;
        let head = group::row(
            true,
            vec![group::lbl("Key sounds", Some("A soft sound on every key press")), group::ctl(vec![self.info(cx, 0, TIP_PRIVACY), toggle::toggle(cx, K_ON, on, false)])],
        );
        let pack = dropdown::dropdown(cx, K_PACK, &self.pack_name(&self.prefs.s.pack), Some(170.0));
        let hv = cx.hover_t(K_PLAY, 150.0, EASE);
        let play = El::block()
            .size(28.0, 28.0)
            .none()
            .radius(RADIUS_PILL)
            .bg(cmix(CTL(), ACC(), hv))
            .place_center()
            .on_click(K_PLAY)
            .cursor(Cursor::Hand)
            .title("Play this sound")
            .child(El::icon("play", 9.0, 1.0, cmix(FG2(), crate::ui::WHITE, hv)).no_hit());
        let v = self.prefs.s.volume;
        let vol = slider::slider(cx, K_VOL, f32::from(v) / 100.0, 150.0, 20.0, slider::default());
        let labels: Vec<&str> = PlayOn::ALL.iter().map(|p| p.label()).collect();
        let at = PlayOn::ALL.iter().position(|p| *p == self.prefs.s.play_on).unwrap_or(0);
        let play_on = seg::seg(cx, K_PLAYON, &labels, at, true);
        let repeats = if self.prefs.s.repeat_ms == 0 { "Off".to_string() } else { format!("{} ms", self.prefs.s.repeat_ms) };
        let mut rows = vec![
            group::row(false, vec![group::lbl("Sound", None), group::ctl(vec![pack, play])]),
            group::row(false, vec![group::lbl("Volume", None), group::ctl(vec![vol, slider::value_label(&format!("{v} %"))])]),
            group::row(false, vec![group::lbl("Play on", None), group::ctl(vec![play_on])]),
            group::row(
                false,
                vec![
                    group::lbl("Ignore repeats within", None),
                    group::ctl(vec![
                        self.info(cx, 1, "A second press of the same key within this time makes no sound"),
                        slider::slider(cx, K_REP, f32::from(self.prefs.s.repeat_ms) / 80.0, 110.0, 20.0, slider::default()),
                        slider::value_label(&repeats),
                    ]),
                ],
            ),
            group::row(
                false,
                vec![group::lbl("Off while a game is in front", None), group::ctl(vec![self.info(cx, 2, "Nothing plays over a fullscreen game"), toggle::toggle(cx, K_GAME, self.prefs.s.off_in_game, false)])],
            ),
            group::row(
                false,
                vec![
                    group::lbl("Mouse clicks too", None),
                    group::ctl(vec![self.info(cx, 3, "Only which button went down or up is heard - never where the mouse is"), toggle::toggle(cx, K_MOUSE, self.prefs.s.mouse_on, false)]),
                ],
            ),
        ];
        if self.prefs.s.mouse_on {
            let mv = self.prefs.s.mouse_volume;
            let vol = slider::slider(cx, K_MVOL, f32::from(mv) / 100.0, 150.0, 20.0, slider::default());
            rows.push(group::row(false, vec![group::lbl("Mouse volume", None), group::ctl(vec![vol, slider::value_label(&format!("{mv} %"))])]));
        }
        rows.push(group::row(
            false,
            vec![group::lbl("Try it", Some("Nothing you type here is kept")), group::ctl(vec![tinput::kdin(cx, K_TRY, &self.try_text, "Type here to hear it", 230.0)])],
        ));
        rows.extend(self.rules(cx));
        let mut rest = El::col().items(AlignItems::STRETCH).children(rows).opacity(if on { 1.0 } else { 0.38 });
        if !on {
            rest = rest.no_hit();
        }
        let mut out = vec![group::gh("Key sounds"), group::grp(vec![head, rest])];
        if let Some(m) = &self.import_msg {
            out.push(group::gf(&format!("Not imported: {m}")));
        }
        if let Some(m) = &self.sound_msg {
            out.push(group::gf(&format!("The sound couldn't start: {m}")));
        }
        out
    }

    // ------------------------------------------------------------------ 2. different in some apps (rows of the same card)

    fn rules(&mut self, cx: &mut Cx) -> Vec<El> {
        let mut rows = vec![group::row(
            false,
            vec![group::lbl("Different in some apps", None), group::ctl(vec![self.info(cx, 4, "Everywhere else uses the sound above"), link::link(cx, K_RADD, "+ Add app", 12.0)])],
        )];
        for (i, r) in self.prefs.s.rules.clone().iter().enumerate() {
            let app = dropdown::dropdown(cx, idx(K_RAPP, i), &title_of(&r.exe), Some(190.0));
            let pack = dropdown::dropdown(cx, idx(K_RPACK, i), &r.pack.as_ref().map(|p| self.pack_name(p)).unwrap_or_else(|| "Off".into()), Some(150.0));
            let del = button::icon_btn(cx, idx(K_RDEL, i), "x", 9.0, 1.5);
            rows.push(group::row(false, vec![app, El::text("→", Font::new(13.0, 400), FG3(), lh(13.0, 1.35)).none(), pack, El::block().flex1(), del]));
        }
        rows
    }

    // ------------------------------------------------------------------ 3. the keyboard picture

    fn keys_group(&mut self, cx: &mut Cx) -> Vec<El> {
        let labels: Vec<&str> = Size::ALL.iter().map(|s| s.label()).collect();
        let on = Size::ALL.iter().position(|s| *s == self.prefs.size).unwrap_or(0);
        let seg_el = seg::seg(cx, K_SIZE, &labels, on, true);
        let any = self.model.remap_count() + self.model.bind_count() > 0;
        let reset_all = link::link(cx, K_RESETALL, "Reset all", 12.0);
        let reset_all = if any { reset_all } else { reset_all.opacity(0.4).no_hit() };
        let head = El::row()
            .center()
            .gap(6.0)
            .margin(20.0, 12.0, 7.0, 12.0)
            .child(El::text("Keys", Font::new(11.0, 500), FG2(), lh(11.0, 1.35)))
            .child(El::row().ml_auto().center().gap(10.0).children(vec![seg_el, reset_all]));
        let (bg, rim) = group::glass();
        let inner_w = 544.0 - 16.0;
        let pic = self.picture(cx, inner_w);
        let hint = El::text("Click a key to change it · keys in blue do something special", Font::new(12.0, 400), FG3(), lh(12.0, 1.35)).align(Align::Center).margin(8.0, 0.0, 0.0, 0.0);
        let card = El::col().items(AlignItems::STRETCH).pad(14.0, 8.0, 10.0, 8.0).radius(12.0).bg(bg).inset(&rim).child(pic).child(hint);
        let mut out = vec![head, card];
        out.push(self.keys_foot(cx));
        if let Some(n) = &self.loop_note {
            out.push(group::gf(n));
        }
        out
    }

    fn picture(&mut self, cx: &mut Cx, w: f32) -> El {
        let h = pic::height(self.h_keys, self.w_keys, w);
        let s = w / (self.w_keys * pic::U);
        let mut looks = Vec::with_capacity(self.keys.len());
        let mut descr: Vec<Option<String>> = Vec::with_capacity(self.keys.len());
        for (i, k) in self.keys.iter().enumerate() {
            let boxes = pic::boxes(k, s).len();
            let hovered = (0..boxes).any(|b| cx.hovered(idx(K_KEY, i + b * ISO_LOWER)));
            let hv = cx.tr(idx(K_KEYT, i), 1, if hovered && !k.dead { 1.0 } else { 0.0 }, 150.0, EASE);
            let app = self.app_action_on(k.code);
            let changed = self.model.changed(k.code, app.is_some());
            looks.push(pic::Look { hv, sel: self.sel == Some(k.code), changed });
            descr.push(if changed {
                Some(match (self.model.remap_of(k.code), self.model.binds.get(k.code)) {
                    (Some(to), _) => format!("→ {}", if to == remap::DISABLED { "Off".to_string() } else { self.label_of(to) }),
                    (None, Some(Bind::Preset(p))) => p.name().to_string(),
                    (None, Some(Bind::Macro(id))) => self.model.macro_by_id(id).map(|m| m.name.clone()).unwrap_or_default(),
                    _ => app.unwrap_or_default(),
                })
            } else {
                None
            });
        }
        let (keys, texts, wk) = (self.keys.clone(), self.texts.clone(), self.w_keys);
        let paint = El::paint(move |g, (x, y, w, _)| pic::paint(g, &keys, &texts, &looks, &|i| descr[i].clone(), (x, y, w), wk)).abs(0.0, 0.0, f32::NAN, f32::NAN).size(w, h).no_hit();
        let mut wrap = El::block().size(w, h).none().child(paint);
        for (i, k) in self.keys.iter().enumerate() {
            if k.dead {
                continue;
            }
            for (b, (l, t, bw, bh)) in pic::boxes(k, s).into_iter().enumerate() {
                wrap = wrap.child(El::block().abs(l, t, f32::NAN, f32::NAN).size(bw, bh).on_click(idx(K_KEY, i + b * ISO_LOWER)).cursor(Cursor::Hand));
            }
        }
        wrap
    }

    /// The key's own small window (the controller's part window): what this key does - Normal / Remap / Action / Macro and
    /// "Reset this key". Nothing of it sits in the tab.
    fn key_window(&mut self, cx: &mut Cx) -> Option<El> {
        let code = self.sel?;
        let app = self.app_action_on(code);
        let mode = self.want.unwrap_or_else(|| self.mode_of(code));
        let sub_t = if mode == Mode::Normal { "as Windows made it" } else { "changed" };
        let labels: Vec<&str> = Mode::ALL.iter().map(|m| m.label()).collect();
        let on = Mode::ALL.iter().position(|m| *m == mode).unwrap_or(0);
        let seg_el = seg::seg(cx, K_MODE, &labels, on, false);
        let state = El::row().center().gap(8.0).child(seg_el).child(El::block().flex1()).child(El::text(sub_t, Font::new(11.5, 400), FG3(), lh(11.5, 1.35)).none());
        let mut kids = vec![state.margin(0.0, 0.0, 10.0, 0.0)];
        match mode {
            Mode::Normal => {
                if let Some(a) = &app {
                    kids.push(group::gf(&format!("This key runs “{a}” (set in Settings › All shortcuts).")));
                } else {
                    kids.push(group::gf("Pick what this key should do"));
                }
            }
            Mode::Remap => {
                let to = self.model.remap_of(code);
                let name = to.map(|t| if t == remap::DISABLED { "Disabled".to_string() } else { self.label_of(t) });
                let show = if self.choosing { Show::Listening(None) } else if let Some(n) = name.as_deref() { Show::Set(n) } else { Show::Empty };
                let field = keyfield::keyfield(cx, K_TARGET, show, self.key_at, true).w(160.0);
                let pick = dropdown::dropdown(cx, K_TPICK, "Pick from a list", None);
                kids.push(group::grp(vec![group::row(true, vec![group::lbl("Becomes", None), group::ctl(vec![field, pick])])]));
                kids.push(group::gf("Press the new key, or pick it · needs one admin Yes and a restart"));
            }
            Mode::Action => {
                let cur = match self.model.binds.get(code) {
                    Some(Bind::Preset(p)) => p.name().to_string(),
                    _ => app.clone().unwrap_or_else(|| "Choose an action".into()),
                };
                let mut rows = vec![group::row(true, vec![group::lbl("Does", None), group::ctl(vec![dropdown::dropdown(cx, K_ACT, &cur, Some(220.0))])])];
                if let Some(Bind::Preset(p)) = self.model.binds.get(code) {
                    if let Some(t) = p.target() {
                        let ph = match p {
                            Preset::OpenWeb(_) => "https://…",
                            Preset::OpenFolder(_) => "C:\\Users\\…\\Documents",
                            _ => "C:\\Program Files\\…\\app.exe",
                        };
                        let what = match p {
                            Preset::OpenWeb(_) => "Website",
                            Preset::OpenFolder(_) => "Folder",
                            _ => "App or file",
                        };
                        let field = tinput::kdin(cx, K_ATEXT, t, ph, 200.0);
                        let browse = if matches!(p, Preset::OpenWeb(_)) { El::block() } else { link::link(cx, K_ABROWSE, "Browse…", 12.0) };
                        rows.push(group::row(false, vec![group::lbl(what, None), group::ctl(vec![field, browse])]));
                    }
                }
                kids.push(group::grp(rows));
                kids.push(group::gf("Works right away · not while a game is in front"));
            }
            Mode::Macro => {
                let cur = match self.model.binds.get(code) {
                    Some(Bind::Macro(id)) => self.model.macro_by_id(id).map(|m| m.name.clone()).unwrap_or_else(|| "Choose a macro".into()),
                    _ => "Choose a macro".into(),
                };
                let mut ctl = vec![dropdown::dropdown(cx, K_MACDD, &cur, Some(210.0))];
                if matches!(self.model.binds.get(code), Some(Bind::Macro(_))) {
                    ctl.push(link::link(cx, K_MACEDIT, "Edit", 12.0));
                }
                kids.push(group::grp(vec![group::row(true, vec![group::lbl("Runs", None), group::ctl(ctl)])]));
                kids.push(group::gf("Works right away · not while a game is in front"));
            }
        }
        if let Some(e) = &self.err {
            kids.push(keyfield::error_line(e));
        }
        if mode != Mode::Normal {
            kids.push(El::row().justify(taffy::style::JustifyContent::FLEX_END).margin(10.0, 0.0, 0.0, 0.0).child(link::link(cx, K_RESETKEY, "Reset this key", 12.0)));
        }
        let body = El::col().items(AlignItems::STRETCH).children(kids);
        Some(dialog::dialog(cx, K_KD, KD_W, &format!("{} key", self.label_of(code)), vec![body], vec![], true, self.key_at))
    }

    fn keys_foot(&mut self, cx: &mut Cx) -> El {
        let dirty = self.model.dirty();
        let nr = self.model.remap_count();
        let na = self.model.bind_count();
        let text = if dirty {
            "Remaps not applied yet · Windows asks for admin once, then it needs a restart".to_string()
        } else if self.restart || glue::restart_pending() {
            "Saved · restart Windows to use the remaps".to_string()
        } else if nr + na > 0 {
            let mut parts = Vec::new();
            if nr > 0 {
                parts.push(format!("{nr} remapped"));
            }
            if na > 0 {
                parts.push(format!("{na} with an action or macro"));
            }
            parts.join(" · ")
        } else {
            "Every key is as Windows made it".to_string()
        };
        let running = cx.job(JOB_APPLY).is_some_and(|v| v.end.is_none());
        let btn = button::cbtn(cx, K_APPLY, if running { "Waiting…" } else { "Apply remaps" }, BKind::Primary, false, !dirty || running, 0.0);
        El::row().center().gap(12.0).margin(8.0, 4.0, 0.0, 4.0).min_h(30.0).child(El::text(&text, Font::new(11.5, 400), FG2(), lh(11.5, 1.35)).wrapping().flex1()).child(btn)
    }

    // ------------------------------------------------------------------ 4. macros

    fn macros_group(&mut self, cx: &mut Cx) -> Vec<El> {
        let mut rows = Vec::new();
        for (i, m) in self.model.macros.clone().iter().enumerate() {
            let used: Vec<String> = self.model.binds.keys_of_macro(&m.id).into_iter().map(|c| self.label_of(c)).collect();
            let sub_t = if m.steps.is_empty() { "No steps yet".to_string() } else { m.steps.iter().map(|s| self.step_text(s)).collect::<Vec<_>>().join(" · ") };
            let name = El::col()
                .flex1()
                .child(El::text(&m.name, Font::new(13.0, 600), FG(), lh(13.0, 1.3)).ellipsis())
                .child(El::text(&sub_t, Font::new(11.0, 400), FG2(), lh(11.0, 1.3)).ellipsis());
            let on = if used.is_empty() { "not on a key".to_string() } else { format!("on {}", used.join(", ")) };
            rows.push(group::row(
                i == 0,
                vec![name, El::text(&on, Font::new(11.5, 400), FG2(), lh(11.5, 1.35)).none(), link::link(cx, idx(K_MEDIT, i), "Edit", 12.0)],
            ));
        }
        rows.push(group::row(self.model.macros.is_empty(), vec![link::link(cx, K_MNEW, "+ New macro", 12.0), El::block().flex1(), link::link(cx, K_MTPL, "+ Ready-made macro", 12.0)]));
        vec![
            group::gh("Macros"),
            group::grp(rows),
            group::gf("Put a macro on a key from the picture above · it never runs while a game is in front"),
        ]
    }

    fn step_text(&self, s: &Step) -> String {
        match s {
            Step::Keys(k) => format!("Press {}", self.combo_text(k)),
            Step::Type(t) => format!("Type “{t}”"),
            Step::Wait(ms) => format!("Wait {ms} ms"),
            Step::Open(o) => format!("Open {o}"),
        }
    }

    /// "Ctrl + Shift + K" from virtual keys (the keys manager's names: the layout's own).
    fn combo_text(&self, vks: &[u16]) -> String {
        if vks.is_empty() {
            return String::new();
        }
        crate::services::with(|s| vks.iter().map(|v| s.keys.key_name(*v)).collect::<Vec<_>>().join(" + ")).unwrap_or_else(|| vks.iter().map(|v| format!("{v:X}")).collect::<Vec<_>>().join(" + "))
    }

    // ------------------------------------------------------------------ the open list + the macro window

    pub(super) fn popups(&mut self, cx: &mut Cx) -> Option<El> {
        let mut kids: Vec<El> = Vec::new();
        if let Some(d) = self.key_window(cx) {
            kids.push(d);
        }
        if let Some(d) = self.getter(cx) {
            kids.push(d);
        }
        if let Some(id) = self.edit.clone() {
            if let Some(d) = self.editor(cx, &id) {
                kids.push(d);
            }
        }
        if let Some((name, (x, y))) = self.ask_del.clone() {
            let q = mitems::confirm(cx, K_DELQ, &format!("Remove {name}?"), "Its files are deleted from this PC. A downloaded sound can be got again.", "Cancel", "Remove", BKind::Red, Place::At(x, y), 260.0);
            kids.push(q.z(30));
        }
        if let Some((p, a)) = self.pop {
            let list = self.list(p);
            let rows: Vec<Row> = list
                .iter()
                .map(|(label, c, on)| match c {
                    Choice::Heading => Row::Section(label.as_str()),
                    _ => Row::Item(It::tick(label.as_str(), *on)),
                })
                .collect();
            // a long list (the ready-made actions) scrolls inside its 300 px box
            let at = Place::Under(a.0, a.1, a.2, a.3);
            let menu = if rows.len() > 14 { mitems::menu_scroll(cx, K_MENU, &rows, at, a.2.max(170.0)) } else { mitems::menu(cx, K_MENU, &rows, at, a.2.max(170.0)) };
            kids.push(menu.z(20));
        }
        if kids.is_empty() {
            None
        } else {
            Some(El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(WIN_W, WIN_H).no_hit().children(kids))
        }
    }

    fn editor(&mut self, cx: &mut Cx, id: &str) -> Option<El> {
        let m = self.model.macros.iter().find(|m| m.id == id)?.clone();
        let name = tinput::kdin(cx, K_MDNAME, &m.name, "Macro name", 404.0);
        let mut rows: Vec<El> = Vec::new();
        for (i, s) in m.steps.iter().enumerate() {
            rows.push(self.step_row(cx, i, s, m.steps.len()));
        }
        if rows.is_empty() {
            let t = if self.rec { "Press the keys now…" } else { "No steps yet. Add one, or press Record and do it once." };
            rows.push(El::text(t, Font::new(12.0, 400), FG3(), lh(12.0, 1.35)).align(Align::Center).pad(16.0, 16.0, 16.0, 16.0));
        }
        let (bg, rim) = group::glass();
        let list = cx.scroll_box(sub(K_MD, "steps"), rows).items(AlignItems::STRETCH).max_h(230.0).radius(10.0).bg(bg).inset(&rim).margin(10.0, 0.0, 0.0, 0.0);
        let add = button::cbtn(cx, K_MDADD, "Add a step", BKind::Ghost, false, false, 0.0);
        let rec = button::cbtn(cx, K_MDREC, if self.rec { "Stop recording" } else { "Record" }, if self.rec { BKind::Red } else { BKind::Ghost }, false, false, 0.0);
        let bar = El::row().center().gap(8.0).margin(10.0, 0.0, 0.0, 0.0).child(add).child(rec);
        let safe = El::text(
            "Runs when you press its key. Never while a game or a full-screen window is in front, and never into admin windows. “Record” keeps only what you press until you stop, and only what you save.",
            Font::new(11.0, 400), FG3(), lh(11.0, 1.4),
        )
        .wrapping()
        .margin(10.0, 2.0, 0.0, 2.0);
        let del = button::cbtn(cx, K_MDDEL, "Delete macro", BKind::RedText, false, false, 0.0);
        let done = button::cbtn(cx, K_MDDONE, "Done", BKind::Primary, false, false, 76.0);
        let foot = El::row().center().gap(8.0).margin(12.0, 0.0, 0.0, 0.0).child(del).child(El::block().flex1()).child(done);
        Some(dialog::dialog(cx, K_MD, 440.0, "Macro", vec![name, list, bar, safe, foot], vec![], true, self.opened_at))
    }

    fn step_row(&mut self, cx: &mut Cx, i: usize, s: &Step, n: usize) -> El {
        let k = idx(K_STEP, i);
        let (icon, label) = match s {
            Step::Keys(_) => ("kbd", "Press"),
            Step::Type(_) => ("note", "Type"),
            Step::Wait(_) => ("gauge", "Wait (ms)"),
            Step::Open(_) => ("arrow", "Open"),
        };
        let ic = El::block().size(22.0, 22.0).none().radius(7.0).bg(CTL()).place_center().child(El::icon(icon, 13.0, 1.5, FG2()).no_hit());
        let val: El = match s {
            Step::Keys(v) => {
                let listening = self.listen_step == Some(i);
                let t = self.combo_text(v);
                let show = if listening { Show::Listening(None) } else if t.is_empty() { Show::Empty } else { Show::Set(&t) };
                keyfield::keyfield(cx, sub(k, "key"), show, self.opened_at, true).w(190.0)
            }
            Step::Type(t) => tinput::kdin(cx, sub(k, "val"), t, "text", 190.0),
            Step::Wait(ms) => tinput::kdin(cx, sub(k, "val"), &ms.to_string(), "200", 190.0),
            Step::Open(t) => tinput::kdin(cx, sub(k, "val"), t, "https://…  or  C:\\…\\app.exe", 190.0),
        };
        let up = button::icon_btn(cx, sub(k, "up"), "chevDw", 11.0, 1.5).rotate(180.0);
        let up = if i == 0 { up.opacity(0.3).no_hit() } else { up };
        let dn = button::icon_btn(cx, sub(k, "dn"), "chevDw", 11.0, 1.5);
        let dn = if i + 1 >= n { dn.opacity(0.3).no_hit() } else { dn };
        let rm = button::icon_btn(cx, sub(k, "rm"), "x", 9.0, 1.5);
        El::row().center().gap(8.0).min_h(38.0).pad(5.0, 8.0, 5.0, 10.0).child(ic).child(El::text(label, Font::new(11.5, 400), FG2(), lh(11.5, 1.35)).w(62.0).none()).child(val).child(up).child(dn).child(rm)
    }

    // ------------------------------------------------------------------ input

    pub(super) fn handle(&mut self, ev: &Ev, cx: &mut Cx) {
        match ev {
            Ev::Press(k, x, y, r) => {
                self.press = Some((*k, *r));
                if *k == K_VOL {
                    self.set_volume(slider::value_at(*r, *x), cx);
                }
                if *k == K_REP {
                    self.set_repeat(slider::value_at(*r, *x));
                }
                if *k == K_MVOL {
                    self.set_mouse_volume(slider::value_at(*r, *x));
                }
                let _ = y;
            }
            Ev::Drag(k, x, _, r) if *k == K_VOL => self.set_volume(slider::value_at(*r, *x), cx),
            Ev::Drag(k, x, _, r) if *k == K_REP => self.set_repeat(slider::value_at(*r, *x)),
            Ev::Drag(k, x, _, r) if *k == K_MVOL => self.set_mouse_volume(slider::value_at(*r, *x)),
            Ev::Release(k) if *k == K_VOL || *k == K_REP || *k == K_MVOL => self.save(),
            Ev::Click(k) => self.clicked(*k, cx),
            Ev::Char(k, c) => self.typed(*k, *c, cx),
            Ev::Key(k, vk) => self.key_down(*k, *vk, cx),
            Ev::Context(k, x, y) => self.ask_remove(*k, *x, *y),
            Ev::Blur(k) if *k == K_TARGET => self.choosing = false,
            _ => {}
        }
    }

    /// "Ignore repeats within": 0 (off) to 80 ms.
    pub(super) fn set_repeat(&mut self, v: f32) {
        self.prefs.s.repeat_ms = (v.clamp(0.0, 1.0) * 80.0).round() as u8;
        if !self.test && self.prefs.on {
            glue::engine().update(self.prefs.s.clone());
        }
    }

    /// The mouse sounds' own volume, 0-100 %.
    pub(super) fn set_mouse_volume(&mut self, v: f32) {
        self.prefs.s.mouse_volume = (v.clamp(0.0, 1.0) * 100.0).round() as u8;
        if !self.test && self.prefs.on {
            glue::engine().update(self.prefs.s.clone());
        }
    }

    fn set_volume(&mut self, v: f32, _cx: &mut Cx) {
        self.prefs.s.volume = (v.clamp(0.0, 1.0) * 100.0).round() as u8;
        // only while the sounds are on: off = nothing is made, nothing is kept
        if !self.test && self.prefs.on {
            glue::engine().update(self.prefs.s.clone());
        }
    }

    fn clicked(&mut self, k: Key, cx: &mut Cx) {
        // "Remove this sound?" - the question owns the next click
        if let Some((name, _)) = self.ask_del.clone() {
            self.ask_del = None;
            if k == sub(K_DELQ, "go") {
                self.pop = None;
                self.remove_pack(&name, cx);
            }
            return;
        }
        if self.get_clicked(k, cx) {
            return;
        }
        // the reset line
        if k == sub(K_RESET, "pc") || k == sub(K_RESET, "win") {
            let rect = self.press.filter(|(pk, _)| *pk == k).map(|(_, r)| r).unwrap_or((0.0, 0.0, 0.0, 0.0));
            cx.open_reset(if k == sub(K_RESET, "pc") { crate::undo::Kind::HowItWas } else { crate::undo::Kind::WindowsDefaults }, rect);
            return;
        }
        // the open list
        if self.pop.is_some() {
            for i in 0..64 {
                if k == idx(K_MENU, i) {
                    let p = self.pop.map(|(p, _)| p).unwrap();
                    if let Some((_, c, _)) = self.list(p).get(i).cloned() {
                        self.choose(c, cx);
                    }
                    return;
                }
            }
        }
        // the key's window: its × / a click beside it close it; a click inside is its own
        if self.edit.is_none() && self.sel.is_some() {
            if k == sub(K_KD, "win") {
                return;
            }
            if k == sub(K_KD, "x") || k == sub(K_KD, "out") {
                if self.pop.is_some() {
                    self.pop = None;
                } else {
                    self.select(None);
                }
                return;
            }
        }
        // the macro window
        if self.edit.is_some() {
            if k == sub(K_MD, "x") || k == sub(K_MD, "out") || k == K_MDDONE {
                self.close_editor();
                return;
            }
            if k == K_MDADD {
                self.open_pop(Pop::StepAdd, k);
                return;
            }
            if k == K_MDREC {
                self.rec = !self.rec;
                self.rec_at = None;
                self.listen_step = None;
                if self.rec {
                    cx.focus(Some(K_MDREC));
                }
                return;
            }
            if k == K_MDDEL {
                if let Some(id) = self.edit.clone() {
                    self.model.delete_macro(&id);
                    cx.toast("Macro deleted");
                }
                self.close_editor();
                return;
            }
            let steps = self.model.macros.iter().find(|m| Some(&m.id) == self.edit.as_ref()).map(|m| m.steps.len()).unwrap_or(0);
            for i in 0..steps {
                let s = idx(K_STEP, i);
                if k == sub(s, "up") && i > 0 {
                    if let Some(m) = self.macro_mut() {
                        m.steps.swap(i, i - 1);
                    }
                    return;
                }
                if k == sub(s, "dn") && i + 1 < steps {
                    if let Some(m) = self.macro_mut() {
                        m.steps.swap(i, i + 1);
                    }
                    return;
                }
                if k == sub(s, "rm") {
                    if let Some(m) = self.macro_mut() {
                        m.steps.remove(i);
                    }
                    self.listen_step = None;
                    return;
                }
                if k == sub(s, "key") {
                    self.listen_step = Some(i);
                    return;
                }
            }
            return;
        }
        match k {
            K_ON => {
                self.prefs.on = !self.prefs.on;
                self.save();
                cx.toast(if self.prefs.on { "Key sounds on" } else { "Key sounds off · the app stops listening" });
            }
            K_PACK => self.open_pop(Pop::Pack, k),
            K_PLAY => {
                let p = self.prefs.s.pack.clone();
                self.preview(&p);
            }
            K_GAME => {
                self.prefs.s.off_in_game = !self.prefs.s.off_in_game;
                self.save();
            }
            K_MOUSE => {
                self.prefs.s.mouse_on = !self.prefs.s.mouse_on;
                self.save();
                cx.toast(if self.prefs.s.mouse_on { "Mouse clicks on" } else { "Mouse clicks off · the app stops listening to the mouse" });
            }
            K_RADD => {
                if let Some(path) = cx.pick_file("Pick the app", &[("Programs", "*.exe")]) {
                    let exe = path.rsplit(['\\', '/']).next().unwrap_or("").to_ascii_lowercase();
                    if !exe.is_empty() && !self.prefs.s.rules.iter().any(|r| r.exe == exe) && self.prefs.s.rules.len() < prefs::MAX_RULES {
                        self.prefs.s.rules.push(Rule { exe, pack: None });
                        self.save();
                    }
                }
            }
            K_RESETALL => {
                self.model.reset_all();
                self.select(None);
                self.save();
                cx.toast("Every key is back as Windows made it · Apply remaps to save");
            }
            K_TARGET => {
                self.choosing = true;
                cx.focus(Some(K_TARGET));
            }
            K_TPICK => self.open_pop(Pop::Target, k),
            K_ACT => self.open_pop(Pop::Action, k),
            K_MACDD => self.open_pop(Pop::MacroList, k),
            K_MACEDIT => {
                if let Some(Bind::Macro(id)) = self.sel.and_then(|c| self.model.binds.get(c)).cloned() {
                    self.open_editor(&id, cx.now);
                }
            }
            K_ABROWSE => {
                if let (Some(c), Some(Bind::Preset(p))) = (self.sel, self.sel.and_then(|c| self.model.binds.get(c)).cloned()) {
                    let picked = if matches!(p, Preset::OpenFolder(_)) { cx.pick_folder("Pick the folder") } else { cx.pick_file("Pick an app or file", &[("Programs and files", "*.*")]) };
                    if let Some(path) = picked {
                        let np = if matches!(p, Preset::OpenFolder(_)) { Preset::OpenFolder(path) } else { Preset::OpenApp(path) };
                        self.set_preset(c, np);
                    }
                }
            }
            K_RESETKEY => {
                if let Some(c) = self.sel {
                    self.model.reset_key(c);
                    self.want = None;
                    self.choosing = false;
                    self.err = None;
                    self.save();
                    cx.toast(&format!("{} is back as Windows made it", self.label_of(c)));
                }
            }
            K_APPLY => self.start_apply(cx),
            K_MTPL => self.open_pop(Pop::Templates, k),
            K_MNEW => {
                if let Some(id) = self.new_macro() {
                    self.open_editor(&id, cx.now);
                }
            }
            _ => {
                for (i, p) in PlayOn::ALL.iter().enumerate() {
                    if k == idx(K_PLAYON, i) {
                        self.prefs.s.play_on = *p;
                        self.save();
                        return;
                    }
                }
                for (i, s) in Size::ALL.iter().enumerate() {
                    if k == idx(K_SIZE, i) {
                        self.change_size(*s);
                        return;
                    }
                }
                for (i, m) in Mode::ALL.iter().enumerate() {
                    if k == idx(K_MODE, i) {
                        if let Some(c) = self.sel {
                            self.set_mode(c, *m);
                        }
                        return;
                    }
                }
                for i in 0..self.keys.len() {
                    if k == idx(K_KEY, i) || k == idx(K_KEY, i + ISO_LOWER) {
                        self.key_clicked(i, cx);
                        return;
                    }
                }
                for i in 0..self.prefs.s.rules.len() {
                    if k == idx(K_RPACK, i) {
                        self.open_pop(Pop::RulePack(i), k);
                        return;
                    }
                    if k == idx(K_RAPP, i) {
                        if let Some(path) = cx.pick_file("Pick the app", &[("Programs", "*.exe")]) {
                            let exe = path.rsplit(['\\', '/']).next().unwrap_or("").to_ascii_lowercase();
                            if !exe.is_empty() {
                                self.prefs.s.rules[i].exe = exe;
                                self.save();
                            }
                        }
                        return;
                    }
                    if k == idx(K_RDEL, i) {
                        self.prefs.s.rules.remove(i);
                        self.save();
                        return;
                    }
                }
                for i in 0..self.model.macros.len() {
                    if k == idx(K_MEDIT, i) {
                        let id = self.model.macros[i].id.clone();
                        self.open_editor(&id, cx.now);
                        return;
                    }
                }
            }
        }
    }

    fn typed(&mut self, k: Key, c: char, _cx: &mut Cx) {
        let mut sel = false;
        if k == K_TRY {
            nbox::type_char(&mut self.try_text, &mut sel, c, 40, Filter::Any);
        } else if k == K_GSEARCH {
            crate::ui::pieces::search::edit_char(&mut self.get_q, c);
        } else if k == K_ATEXT {
            if let Some(code) = self.sel {
                if let Some(Bind::Preset(p)) = self.model.binds.get(code).cloned() {
                    let mut t = p.target().unwrap_or("").to_string();
                    nbox::type_char(&mut t, &mut sel, c, bu_keysound::binds::MAX_TARGET, Filter::Any);
                    self.set_target_text(code, &p, t);
                }
            }
        } else if k == K_MDNAME {
            if let Some(m) = self.macro_mut() {
                nbox::type_char(&mut m.name, &mut sel, c, bu_keysound::macros::MAX_NAME, Filter::Any);
            }
        } else if let Some(i) = self.step_of(k, "val") {
            self.edit_step_text(i, |t| nbox::type_char(t, &mut false, c, 400, Filter::Any), c);
        }
    }

    fn step_of(&self, k: Key, part: &str) -> Option<usize> {
        let n = self.model.macros.iter().find(|m| Some(&m.id) == self.edit.as_ref()).map(|m| m.steps.len())?;
        (0..n).find(|i| k == sub(idx(K_STEP, *i), part))
    }

    fn edit_step_text(&mut self, i: usize, f: impl FnOnce(&mut String), c: char) {
        if let Some(m) = self.macro_mut() {
            match m.steps.get_mut(i) {
                Some(Step::Type(t)) | Some(Step::Open(t)) => f(t),
                Some(Step::Wait(ms)) if c.is_ascii_digit() => {
                    let mut t = ms.to_string();
                    if t == "0" {
                        t.clear();
                    }
                    t.push(c);
                    *ms = t.parse::<u32>().unwrap_or(*ms).min(bu_keysound::macros::MAX_WAIT_MS);
                }
                _ => {}
            }
        }
    }

    fn set_target_text(&mut self, code: Code, p: &Preset, t: String) {
        let np = match p {
            Preset::OpenApp(_) => Preset::OpenApp(t),
            Preset::OpenFolder(_) => Preset::OpenFolder(t),
            _ => Preset::OpenWeb(t),
        };
        if self.model.set_preset(code, np).is_ok() {
            self.save();
        }
    }

    fn key_down(&mut self, k: Key, vk: u16, cx: &mut Cx) {
        let back = vk == 0x08;
        if k == K_TARGET && self.choosing {
            if vk != 0x1B {
                cx.used = true;
                self.target_pressed(vk);
            }
            return;
        }
        if k == K_TRY && back {
            self.try_text.pop();
        } else if k == K_GSEARCH {
            crate::ui::pieces::search::edit_key(&mut self.get_q, vk);
        } else if k == K_ATEXT && back {
            if let Some(code) = self.sel {
                if let Some(Bind::Preset(p)) = self.model.binds.get(code).cloned() {
                    let mut t = p.target().unwrap_or("").to_string();
                    t.pop();
                    self.set_target_text(code, &p, t);
                }
            }
        } else if k == K_MDNAME && back {
            if let Some(m) = self.macro_mut() {
                m.name.pop();
            }
        } else if let Some(i) = self.step_of(k, "val") {
            if back {
                if let Some(m) = self.macro_mut() {
                    match m.steps.get_mut(i) {
                        Some(Step::Type(t)) | Some(Step::Open(t)) => {
                            t.pop();
                        }
                        Some(Step::Wait(ms)) => *ms /= 10,
                        _ => {}
                    }
                }
            } else if vk == 0x0D {
                // Enter in a text step: a new line in a typed text
                if let Some(m) = self.macro_mut() {
                    if let Some(Step::Type(t)) = m.steps.get_mut(i) {
                        t.push('\n');
                    }
                }
            }
        } else if let Some(i) = self.step_of(k, "key") {
            // click the step, then press the key or combo
            if self.listen_step == Some(i) && vk != 0x1B {
                if let Some(c) = combo_of(vk) {
                    cx.used = true;
                    if let Some(m) = self.macro_mut() {
                        if let Some(Step::Keys(v)) = m.steps.get_mut(i) {
                            *v = c;
                        }
                    }
                    self.listen_step = None;
                }
            }
        } else if k == K_MDREC && self.rec {
            if vk == 0x1B {
                self.rec = false;
                return;
            }
            cx.used = true;
            if let Some(c) = combo_of(vk) {
                let now = cx.now;
                let gap = self.rec_at.map(|t| now - t).unwrap_or(0.0);
                self.rec_at = Some(now);
                let rec_gap = ((gap / 50.0).round() * 50.0) as u32;
                if let Some(m) = self.macro_mut() {
                    if rec_gap >= 350 && !m.steps.is_empty() {
                        m.steps.push(Step::Wait(rec_gap.min(bu_keysound::macros::MAX_WAIT_MS)));
                    }
                    if m.steps.len() < bu_keysound::macros::MAX_STEPS {
                        m.steps.push(Step::Keys(c));
                    }
                }
            }
        }
    }
}

/// The key combo for a key press: the modifiers held now (Ctrl, Alt, Shift, Win - in this order) then the key. A modifier
/// pressed alone is not a combo yet (None). Windows only reads the held modifiers.
pub(super) fn combo_of(vk: u16) -> Option<Vec<u16>> {
    if matches!(vk, 0x10 | 0x11 | 0x12 | 0xA0..=0xA5 | 0x5B | 0x5C) {
        return None;
    }
    let held = |k: u16| (unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(i32::from(k)) } as u16) & 0x8000 != 0;
    Some(combo_from(vk, [held(0x11), held(0x12), held(0x10), held(0x5B) || held(0x5C)]))
}

/// The combo of a main key and which of Ctrl, Alt, Shift, Win are held.
pub(super) fn combo_from(vk: u16, held: [bool; 4]) -> Vec<u16> {
    let mut v: Vec<u16> = [0x11u16, 0x12, 0x10, 0x5B].iter().zip(held).filter(|(_, h)| *h).map(|(k, _)| *k).collect();
    v.push(vk);
    v
}

fn note(text: &str) -> El {
    El::text(text, Font::new(11.5, 400), FG3(), lh(11.5, 1.35)).wrapping().pad(8.0, 14.0, 4.0, 14.0)
}

#[allow(dead_code)]
fn unused(_: Rgba) {
    let _ = (sh, ACC, CTL_H, pieces::header);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_combo_lists_ctrl_alt_shift_win_then_the_key() {
        assert_eq!(combo_from(0x4B, [true, false, true, false]), vec![0x11, 0x10, 0x4B]);
        assert_eq!(combo_from(0x52, [false, false, false, true]), vec![0x5B, 0x52]);
        assert_eq!(combo_from(0x70, [false; 4]), vec![0x70]);
    }

    #[test]
    fn app_names_come_from_the_exe() {
        assert_eq!(title_of("discord.exe"), "Discord");
        assert_eq!(title_of("notepad++.exe"), "Notepad++");
        assert_eq!(title_of(""), "");
    }
}
