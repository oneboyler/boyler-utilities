//! The Keyboard tab's boxes and its input (Order 090, keyboard-v8.html): the keyboard picture first (click a key = its
//! window; drag a box / Ctrl / Shift = several keys), the line under it, the foot; then the "Keyboard sounds" card (folds like
//! Mouse acceleration) with More; the key window, the "N keys" window and "Make a pack from one sound".

use super::*;
use crate::anim::EASE;
use crate::gfx::{Font, Rgba};
use crate::pages::btnwin::{self, HearOf, MacroEd, MacroFor, KEY_WORDS};
use crate::ui::el::{lh, Cursor};
use crate::ui::pieces::button::{self, Kind as BKind};
use crate::ui::pieces::dropdown;
use crate::ui::pieces::keyfield::{self, Show};
use crate::ui::pieces::mitems::{self, It, Place, Row};
use crate::ui::pieces::tip::{self, Rq};
use crate::ui::pieces::{self, card, dialog, fold, group, link, reset, seg, slider, tinput, toggle};
use crate::ui::{ACC, FG, FG2, FG3, HAIR, HOV, WIN_H, WIN_W};
use bu_keysound::{Dev, Layers, PlayOn};
use taffy::style::{AlignItems, JustifyContent};

/// Order 090: the key is used to pick its own sound now - the tip says so plainly.
const TIP_PRIVACY: &str = "The key is used only to pick its sound \u{b7} nothing you type is kept";
const K_TIP: Key = key("kbd.tip");
/// The content width the card's fold measures with.
const CARD_W: f32 = 544.0;

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
        kids.push(self.sounds_card(cx));
        if let Some(m) = &self.import_msg {
            kids.push(group::gf(&format!("Not imported: {m}")));
        }
        if let Some(m) = &self.sound_msg {
            kids.push(group::gf(&format!("The sound couldn't start: {m}")));
        }
        kids.push(reset::reset_line(cx, K_RESET, Some("Windows defaults")));
        kids
    }

    /// A small "i" with a one-line tip.
    fn info(&self, cx: &mut Cx, n: usize, text: &str) -> El {
        tip::rq(cx, idx(K_TIP, n), Rq::Info, 16.0, text, false)
    }

    // ------------------------------------------------------------------ 1. the keyboard picture

    fn keys_group(&mut self, cx: &mut Cx) -> Vec<El> {
        let labels: Vec<&str> = Size::ALL.iter().map(|s| s.label()).collect();
        let on = Size::ALL.iter().position(|s| *s == self.prefs.size).unwrap_or(0);
        let seg_el = seg::seg(cx, K_SIZE, &labels, on, true);
        let any = self.model.remap_count() + self.model.bind_count() > 0;
        let reset_all = link::link(cx, K_RESETALL, "Reset all", 12.0).title("What every key does goes back to Windows\u{2019} \u{b7} the sounds stay");
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
        let hint = self.hint_line(cx);
        let card = El::col().items(AlignItems::STRETCH).pad(14.0, 8.0, 10.0, 8.0).radius(12.0).bg(bg).inset(&rim).child(pic).child(hint);
        let mut out = vec![head, card];
        out.push(self.keys_foot(cx));
        if let Some(n) = &self.loop_note {
            out.push(group::gf(n));
        }
        out
    }

    /// The line under the picture: what a click does + the dot's meaning; with keys picked "N keys picked · Clear · Change".
    fn hint_line(&mut self, cx: &mut Cx) -> El {
        let n = self.pick.len();
        if n == 0 {
            let dot = El::block().size(5.0, 5.0).none().radius(3.0).bg(FG());
            return El::row()
                .center()
                .justify(JustifyContent::CENTER)
                .gap(6.0)
                .margin(8.0, 0.0, 0.0, 0.0)
                .child(El::text("Click a key to change it \u{b7} keys in blue do something special", Font::new(12.0, 400), FG3(), lh(12.0, 1.35)).none())
                .child(El::row().center().gap(5.0).margin(0.0, 0.0, 0.0, 8.0).child(dot).child(El::text("own sound", Font::new(12.0, 400), FG3(), lh(12.0, 1.35)).none()));
        }
        let names: Vec<String> = self.reading(&self.pick).iter().map(|c| self.label_of(*c)).collect();
        let mut list = names.iter().take(12).cloned().collect::<Vec<_>>().join("  ");
        if names.len() > 12 {
            list.push_str("  \u{2026}");
        }
        let chg = button::cbtn(cx, K_CHG, &if n == 1 { "Change this key".to_string() } else { format!("Change {n} keys") }, BKind::Primary, true, false, 0.0);
        El::row()
            .center()
            .gap(10.0)
            .margin(8.0, 4.0, 0.0, 4.0)
            .min_h(26.0)
            .child(El::text(if n == 1 { "1 key picked".to_string() } else { format!("{n} keys picked") }, Font::new(12.0, 600), FG(), lh(12.0, 1.35)).none())
            .child(El::text(list, Font::new(12.0, 400), FG2(), lh(12.0, 1.35)).ellipsis().flex1())
            .child(link::link(cx, K_CLR, "Clear", 12.0))
            .child(chg)
    }

    /// `codes` in reading order (row by row, left to right).
    pub(super) fn reading(&self, codes: &[Code]) -> Vec<Code> {
        let (ks, _, _) = pic::keys(Size::Full);
        let mut v: Vec<(i32, f32, Code)> = codes
            .iter()
            .map(|c| {
                let k = ks.iter().find(|k| k.code == *c);
                (k.map(|k| (k.y * 4.0).round() as i32).unwrap_or(99), k.map(|k| k.x).unwrap_or(99.0), *c)
            })
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        v.into_iter().map(|t| t.2).collect()
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
            let dot = self.prefs.keys.get(k.code).is_some_and(|l| l.press.is_some() || !l.pack_on);
            looks.push(pic::Look { hv, sel: self.sel == Some(k.code), changed, pick: self.pick.contains(&k.code), dot, tint: None });
            descr.push(if changed {
                Some(match (self.model.remap_of(k.code), self.model.binds.get(k.code)) {
                    (Some(to), _) => format!("\u{2192} {}", if to == remap::DISABLED { "Off".to_string() } else { self.label_of(to) }),
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
        // the picture's own background takes a press too: a box can start between the keys
        let mut wrap = El::block().size(w, h).none().on_click(K_PICBG).child(paint);
        for (i, k) in self.keys.iter().enumerate() {
            if k.dead {
                continue;
            }
            for (b, (l, t, bw, bh)) in pic::boxes(k, s).into_iter().enumerate() {
                wrap = wrap.child(El::block().abs(l, t, f32::NAN, f32::NAN).size(bw, bh).on_click(idx(K_KEY, i + b * ISO_LOWER)).cursor(Cursor::Hand));
            }
        }
        // the box being drawn (`.kband`: a light accent fill, an accent rim)
        if let Some(b) = self.band.as_ref().filter(|b| b.moved) {
            let (x0, y0) = (b.start.0.min(b.now.0) - b.origin.0, b.start.1.min(b.now.1) - b.origin.1);
            let (bw, bh) = ((b.start.0 - b.now.0).abs(), (b.start.1 - b.now.1).abs());
            wrap = wrap.child(El::block().abs(x0, y0, f32::NAN, f32::NAN).size(bw, bh).radius(3.0).bg(ACC().mul_a(0.14)).inset(&[crate::gfx::sh(0.0, 0.0, 0.0, 1.0, ACC().mul_a(0.7))]).no_hit());
        }
        wrap
    }

    /// The key's own window: what it does (Normal / Remap / Action / Macro with its steps here) + its Sound.
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
                    kids.push(group::gf(&format!("This key runs \u{201c}{a}\u{201d} (set in Settings \u{203a} All shortcuts).")));
                }
            }
            Mode::Remap => {
                let to = self.model.remap_of(code);
                let name = to.map(|t| if t == remap::DISABLED { "Disabled".to_string() } else { self.label_of(t) });
                let show = if self.choosing { Show::Listening(None) } else if let Some(n) = name.as_deref() { Show::Set(n) } else { Show::Empty };
                let field = keyfield::keyfield(cx, K_TARGET, show, self.key_at, true).w(160.0);
                let pick = dropdown::dropdown(cx, K_TPICK, "Pick from a list", None);
                kids.push(group::grp(vec![group::row(true, vec![group::lbl("Becomes", None), group::ctl(vec![field, pick])])]));
                kids.push(group::gf("Press the new key, or pick it \u{b7} needs one admin Yes and a restart"));
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
                            Preset::OpenWeb(_) => "https://\u{2026}",
                            Preset::OpenFolder(_) => "C:\\Users\\\u{2026}\\Documents",
                            _ => "C:\\Program Files\\\u{2026}\\app.exe",
                        };
                        let what = match p {
                            Preset::OpenWeb(_) => "Website",
                            Preset::OpenFolder(_) => "Folder",
                            _ => "App or file",
                        };
                        let field = tinput::kdin(cx, K_ATEXT, t, ph, 200.0);
                        let browse = if matches!(p, Preset::OpenWeb(_)) { El::block() } else { link::link(cx, K_ABROWSE, "Browse\u{2026}", 12.0) };
                        rows.push(group::row(false, vec![group::lbl(what, None), group::ctl(vec![field, browse])]));
                    }
                }
                kids.push(group::grp(rows));
                kids.push(group::gf("Works right away \u{b7} not while a game is in front"));
            }
            Mode::Macro => {
                let cur = match self.model.binds.get(code) {
                    Some(Bind::Macro(id)) => Some(id.clone()),
                    _ => None,
                };
                let also: Vec<String> = cur.as_deref().map(|id| self.model.binds.keys_of_macro(id).into_iter().filter(|c| *c != code).map(|c| self.label_of(c)).collect()).unwrap_or_default();
                let macros = self.model.macros.clone();
                kids.extend(self.med.view(cx, K_MAC, &macros, cur.as_deref(), MacroFor::Keys, true, &also));
            }
        }
        if let Some(e) = &self.err {
            kids.push(keyfield::error_line(e));
        }
        if mode != Mode::Normal {
            let mut r = El::row().center().margin(10.0, 0.0, 0.0, 0.0);
            if mode == Mode::Macro && matches!(self.model.binds.get(code), Some(Bind::Macro(_))) {
                r = r.child(MacroEd::delete_link(cx, K_MAC));
            }
            kids.push(r.child(El::block().flex1()).child(link::link(cx, K_RESETKEY, "Reset this key", 12.0)));
        }
        // a plain key has nothing above the Sound part, so no divider either
        if mode != Mode::Normal || app.is_some() {
            kids.push(El::block().h(1.0).bg(HAIR()).margin(14.0, 0.0, 0.0, 0.0));
        }
        let names = |f: &str| f.to_string();
        kids.push(self.snd.view(cx, K_SND, &self.prefs.keys, &[code], &KEY_WORDS, &names));
        let body = cx.scroll_box(sub(K_KD, "body"), kids).items(AlignItems::STRETCH).style(|s| {
            s.flex_shrink = 1.0;
            s.min_size.height = taffy::style::LengthPercentageAuto::length(0.0);
        });
        Some(dialog::dialog(cx, K_KD, KD_W, &format!("{} key", self.label_of(code)), vec![body], vec![], true, self.key_at))
    }

    /// "N keys": one sound for the picked keys; what each one does stays as it is.
    fn many_window(&mut self, cx: &mut Cx) -> Option<El> {
        if !self.many || self.pick.len() < 2 {
            return None;
        }
        let ids = self.reading(&self.pick);
        let names: Vec<String> = ids.iter().map(|c| self.label_of(*c)).collect();
        let mut list = names.iter().take(16).cloned().collect::<Vec<_>>().join(", ");
        if names.len() > 16 {
            list.push_str(", \u{2026}");
        }
        let line = El::text(format!("{list} \u{b7} what each one does stays as it is"), Font::new(11.5, 400), FG3(), lh(11.5, 1.35)).wrapping().margin(-6.0, 2.0, 0.0, 2.0);
        let names_of = |f: &str| f.to_string();
        let part = self.snd.view(cx, K_SND, &self.prefs.keys, &ids, &KEY_WORDS, &names_of);
        let body = cx.scroll_box(sub(K_KN, "body"), vec![line, part]).items(AlignItems::STRETCH).style(|s| {
            s.flex_shrink = 1.0;
            s.min_size.height = taffy::style::LengthPercentageAuto::length(0.0);
        });
        Some(dialog::dialog(cx, K_KN, KD_W, &format!("{} keys", ids.len()), vec![body], vec![], true, self.many_at))
    }

    fn keys_foot(&mut self, cx: &mut Cx) -> El {
        let dirty = self.model.dirty();
        let nr = self.model.remap_count();
        let na = self.model.bind_count();
        let no = self.prefs.keys.with_own().count();
        let text = if dirty {
            "Remaps not applied yet \u{b7} Windows asks for admin once, then it needs a restart".to_string()
        } else if self.restart || glue::restart_pending() {
            "Saved \u{b7} restart Windows to use the remaps".to_string()
        } else if nr + na + no > 0 {
            let mut parts = Vec::new();
            if nr > 0 {
                parts.push(format!("{nr} remapped"));
            }
            if na > 0 {
                parts.push(format!("{na} with an action or macro"));
            }
            if no > 0 {
                parts.push(if no == 1 { "1 with its own sound".to_string() } else { format!("{no} with their own sound") });
            }
            parts.join(" \u{b7} ")
        } else {
            "Every key is as Windows made it".to_string()
        };
        let running = cx.job(JOB_APPLY).is_some_and(|v| v.end.is_none());
        let btn = button::cbtn(cx, K_APPLY, if running { "Waiting\u{2026}" } else { "Apply remaps" }, BKind::Primary, false, !dirty || running, 0.0);
        El::row().center().gap(12.0).margin(8.0, 4.0, 0.0, 4.0).min_h(30.0).child(El::text(&text, Font::new(11.5, 400), FG2(), lh(11.5, 1.35)).wrapping().flex1()).child(btn)
    }

    // ------------------------------------------------------------------ 2. the Keyboard sounds card

    fn sounds_card(&mut self, cx: &mut Cx) -> El {
        let on = self.prefs.on;
        let open = self.card_open;
        let hv = cx.hover_t(K_CARDH, 150.0, EASE);
        let line = if on { format!("{} \u{b7} {} %", self.pack_name(&self.prefs.s.pack), self.prefs.s.volume) } else { "A soft sound on every key press".to_string() };
        let right = group::ctl(vec![self.info(cx, 0, TIP_PRIVACY), toggle::toggle(cx, K_ON, on, false), fold::chev(cx, K_CHEV, open, false)]);
        let head = card::card_head("kbd", "Keyboard sounds", Some(&line), vec![right]).bg(HOV().mul_a(hv)).on_click(K_CARDH).cursor(Cursor::Hand);
        let body = self.card_body(cx);
        // switched off = greyed and untouchable (as Mouse acceleration)
        let lt = cx.tr(K_CARD, 20, if on { 1.0 } else { 0.36 }, 250.0, EASE);
        let body = if on { body.opacity(lt) } else { body.opacity(lt).no_hit() };
        card::card(cx, K_CARD, head, Some(body), open, CARD_W).margin(12.0, 0.0, 0.0, 0.0)
    }

    fn card_body(&mut self, cx: &mut Cx) -> El {
        let pack = dropdown::dropdown(cx, K_PACK, &self.pack_name(&self.prefs.s.pack), Some(190.0));
        let play = btnwin::play_btn(cx, K_PLAY, "Play this sound");
        let v = self.prefs.s.volume;
        let vol = slider::slider(cx, K_VOL, f32::from(v) / 100.0, 150.0, 20.0, slider::default());
        let labels: Vec<&str> = PlayOn::ALL.iter().map(|p| p.label()).collect();
        let at = PlayOn::ALL.iter().position(|p| *p == self.prefs.s.play_on).unwrap_or(0);
        let play_on = seg::seg(cx, K_PLAYON, &labels, at, true);
        let mut rows = vec![
            group::row(false, vec![group::lbl("Sound", None), group::ctl(vec![pack, play])]),
            group::row(false, vec![group::lbl("Volume", None), group::ctl(vec![vol, slider::value_label(&format!("{v} %"))])]),
            group::row(false, vec![group::lbl("Play on", None), group::ctl(vec![play_on])]),
        ];
        rows.extend(self.extra_rows(cx));
        El::col().items(AlignItems::STRETCH).children(rows)
    }

    fn extra_rows(&mut self, cx: &mut Cx) -> Vec<El> {
        let rep = self.prefs.s.repeat_ms;
        let repeats = if rep == 0 { "Off".to_string() } else { format!("{rep} ms") };
        let mut rows = vec![
            group::row(
                false,
                vec![
                    group::lbl("Ignore repeats within", None),
                    group::ctl(vec![slider::slider(cx, K_REP, f32::from(rep) / f32::from(bu_keysound::MAX_REPEAT_MS), 110.0, 20.0, slider::default()), slider::value_label(&repeats)]),
                ],
            ),
            group::row(false, vec![group::lbl("Off while a game is in front", Some("Nothing plays over a fullscreen game")), group::ctl(vec![toggle::toggle(cx, K_GAME, self.prefs.s.off_in_game, false)])]),
            group::row(false, vec![group::lbl("Try it", Some("Nothing you type here is kept")), group::ctl(vec![tinput::kdin(cx, K_TRY, &self.try_text, "Type here to hear it", 230.0)])]),
        ];
        rows.extend(self.rules(cx));
        rows
    }

    fn rules(&mut self, cx: &mut Cx) -> Vec<El> {
        let mut rows = vec![group::row(false, vec![group::lbl("Different in some apps", Some("Everywhere else uses the sound above")), group::ctl(vec![link::link(cx, K_RADD, "+ Add app", 12.0)])])];
        for (i, r) in self.prefs.s.rules.clone().iter().enumerate() {
            let app = dropdown::dropdown(cx, idx(K_RAPP, i), &title_of(&r.exe), Some(190.0));
            let pack = dropdown::dropdown(cx, idx(K_RPACK, i), &r.pack.as_ref().map(|p| self.pack_name(p)).unwrap_or_else(|| "Off".into()), Some(150.0));
            let del = button::icon_btn(cx, idx(K_RDEL, i), "x", 9.0, 1.5);
            rows.push(group::row(false, vec![app, El::text("\u{2192}", Font::new(13.0, 400), FG3(), lh(13.0, 1.35)).none(), pack, El::block().flex1(), del]));
        }
        rows
    }

    // ------------------------------------------------------------------ 3. Make a pack from one sound

    pub(super) fn open_make(&mut self, now: f64) {
        self.make = Some(MakePack { press: None, release: None, vary: bu_keysound::Vary::for_pack(), name: String::new(), named: false, opened: now, msg: None });
        self.make_clips = (None, None);
    }

    fn make_window(&mut self, cx: &mut Cx) -> Option<El> {
        let mp = self.make.clone()?;
        let have = mp.press.is_some();
        // pack-noise-v1: Press and the optional Release side by side in one card: a file box each (drop a file on it, or click)
        let chip = |cx: &mut Cx, k: Key, file: Option<&(String, String)>, empty: &str| -> El {
            let hv = cx.hover_t(k, 120.0, EASE);
            let set = file.is_some();
            let mut r = El::row()
                .center()
                .gap(7.0)
                .h(30.0)
                .flex1()
                .min_w(0.0)
                .pad(0.0, 8.0, 0.0, 10.0)
                .radius(7.0)
                .bg(if set { crate::ui::cmix(crate::ui::CTL(), crate::ui::CTL_H(), hv) } else { crate::ui::cmix(crate::ui::CTL().mul_a(0.45), crate::ui::CTL_H().mul_a(0.7), hv) })
                .on_click(k)
                .cursor(Cursor::Hand)
                .child(El::icon("note", 12.0, 1.5, if set { ACC() } else { FG3() }).no_hit())
                .child(El::text(file.map(|f| f.1.clone()).unwrap_or_else(|| empty.to_string()), Font::new(12.5, 400), if set { FG() } else { FG2() }, lh(12.5, 1.35)).ellipsis().flex1().no_hit());
            if set {
                r = r.inset(&[crate::gfx::sh(0.0, 0.0, 0.0, 1.0, ACC().mul_a(0.8))]).child(button::icon_btn(cx, sub(k, "x"), "x", 8.0, 1.5).size(20.0, 20.0).title("Remove this file"));
            } else {
                r = r.inset(&[crate::gfx::sh(0.0, 0.0, 0.0, 1.0, Rgba(1.0, 1.0, 1.0, 0.14))]);
            }
            r
        };
        let lab = |t: &str, w: f32| El::text(t, Font::new(13.0, 500), FG(), lh(13.0, 1.35)).none().min_w(w);
        let pc = chip(cx, K_MPP, mp.press.as_ref(), "Drop a file here, or click");
        let rc = chip(cx, K_MPR, mp.release.as_ref(), "optional \u{b7} add a file");
        let files = group::grp(vec![El::row().center().gap(14.0).min_h(46.0).pad(6.0, 12.0, 6.0, 12.0).child(lab("Press", 38.0)).child(pc).child(lab("Release", 52.0).margin(0.0, 0.0, 0.0, 4.0)).child(rc)]);
        let vary = group::grp(btnwin::vary_rows(cx, K_MPV, &mp.vary, MAKE_VARY_W)).margin(10.0, 0.0, 0.0, 0.0);
        let vary = if have { vary } else { vary.opacity(0.38).no_hit() };
        let name = tinput::kdin(cx, K_MPNAME, &mp.name, "Pack name", 300.0);
        let save = button::cbtn_sized(cx, K_MPSAVE, "Save pack", BKind::Primary, button::DFT, !have, 96.0);
        let foot = El::row().center().gap(10.0).margin(12.0, 0.0, 0.0, 0.0).child(El::text("Name", Font::new(12.0, 400), FG2(), lh(12.0, 1.35)).none()).child(name).child(El::block().flex1()).child(save);
        let mut kids = vec![files, vary];
        if let Some(m) = &mp.msg {
            kids.push(keyfield::error_line(m));
        }
        kids.push(foot);
        let body = cx.scroll_box(sub(K_MP, "body"), kids).items(AlignItems::STRETCH).style(|s| {
            s.flex_shrink = 1.0;
            s.min_size.height = taffy::style::LengthPercentageAuto::length(0.0);
        });
        Some(dialog::dialog(cx, K_MP, 560.0, "Make a pack from one sound", vec![body], vec![], true, mp.opened))
    }

    /// A file for the pack maker's press (`release` false) or release: decoded at once (safely) so it can be heard.
    fn make_file(&mut self, path: &str, release: bool) {
        let test = self.test;
        let Some(mp) = self.make.as_mut() else { return };
        let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = std::path::Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if !["mp3", "wav", "ogg"].contains(&ext.as_str()) {
            mp.msg = Some(format!("{name} \u{b7} use an .mp3, .wav or .ogg file"));
            return;
        }
        let clip = if test {
            None
        } else {
            match bu_keysound::safe::read_sound(std::path::Path::new(path), false) {
                Ok(c) => Some(c),
                Err(e) => {
                    mp.msg = Some(format!("{name} \u{b7} {e}"));
                    return;
                }
            }
        };
        mp.msg = None;
        if release {
            mp.release = Some((path.to_string(), name));
            self.make_clips.1 = clip;
        } else {
            if !mp.named {
                mp.name = std::path::Path::new(&name).file_stem().map(|s| s.to_string_lossy().chars().take(32).collect()).unwrap_or_default();
            }
            mp.press = Some((path.to_string(), name));
            self.make_clips.0 = clip;
        }
    }

    /// Plays keys `codes` of the pack being made (each: its press, then its release).
    fn make_hear(&self, codes: &[Code]) {
        if self.test {
            return;
        }
        let (Some(mp), Some(press)) = (self.make.as_ref(), self.make_clips.0.as_ref()) else { return };
        let g = bu_keysound::gain(self.prefs.s.volume.max(5));
        let mut list = Vec::new();
        for (i, c) in codes.iter().enumerate() {
            let (st, loud) = mp.vary.at(*c, true);
            let at = i as u32 * 130;
            let gl = g * f32::from(loud) / 100.0;
            if self.prefs.s.play_on.plays(true) {
                list.push(bu_keysound::Hear { sound: press.mono.clone(), rate: press.rate, gain: gl, pitch: st, at_ms: at });
            }
            if self.prefs.s.play_on.plays(false) {
                let (s, r, up) = match self.make_clips.1.as_ref() {
                    Some(rel) => (rel.mono.clone(), rel.rate, 0.0),
                    None => (press.mono.clone(), press.rate, bu_keysound::layers::RELEASE_UP),
                };
                list.push(bu_keysound::Hear { sound: s, rate: r, gain: gl * 0.9, pitch: st + up, at_ms: at + 90 });
            }
        }
        glue::engine().hear(list);
    }

    /// Save: the pack is written (its files copied in), it becomes the sound and plays.
    fn make_save(&mut self, cx: &mut Cx) {
        let Some(mp) = self.make.clone() else { return };
        let Some((press, _)) = mp.press.clone() else { return };
        let name = if mp.name.trim().is_empty() { "My pack".to_string() } else { mp.name.trim().to_string() };
        let made = bu_keysound::Made { name: name.clone(), press: String::new(), release: None, vary: mp.vary, keys: bu_keysound::Made::spread(&mp.vary, &pic::codes()) };
        let saved = if self.test {
            self.made_list.push(name.clone());
            Ok(name)
        } else {
            let Some(dir) = crate::services::with(|s| glue::packs_dir(s.store.folder())) else { return };
            glue::save_made(&dir, made, std::path::Path::new(&press), mp.release.as_ref().map(|r| std::path::Path::new(&r.0)))
        };
        match saved {
            Ok(n) => {
                self.make = None;
                self.prefs.s.pack = Pack::Made(n.clone());
                self.save();
                let p = self.prefs.s.pack.clone();
                self.preview(&p);
                cx.toast(&format!("Saved \u{b7} \u{201c}{n}\u{201d} is in Your packs and plays now"));
            }
            Err(e) => {
                if let Some(m) = self.make.as_mut() {
                    m.msg = Some(e);
                }
            }
        }
    }

    /// The pack maker's events; true = its own (its window takes every event while it is open).
    fn make_event(&mut self, ev: &Ev, cx: &mut Cx) -> bool {
        let Some(mp) = self.make.as_mut() else { return false };
        if let Some(done) = btnwin::vary_event(ev, K_MPV, &mut mp.vary) {
            if done {
                self.make_hear(&[0x1E, 0x1F, 0x20, 0x21, 0x22, 0x39]);
            }
            return true;
        }
        match ev {
            Ev::Drop(k, paths) if [K_MPP, K_MPR].contains(k) => {
                if let Some(p) = paths.first() {
                    let release = *k == K_MPR;
                    self.make_file(p, release);
                    self.make_hear(&[if release { 0x39 } else { 0x1E }]);
                }
                true
            }
            Ev::Char(k, c) if *k == K_MPNAME => {
                mp.named = true;
                nbox::type_char(&mut mp.name, &mut false, *c, 32, Filter::Any);
                true
            }
            Ev::Key(k, vk) if *k == K_MPNAME => {
                if *vk == 0x08 {
                    mp.named = true;
                    mp.name.pop();
                } else if *vk == 0x0D {
                    self.make_save(cx);
                }
                true
            }
            Ev::Click(k) => {
                let k = *k;
                if k == sub(K_MP, "x") || k == sub(K_MP, "out") {
                    self.make = None;
                } else if k == sub(K_MPP, "x") {
                    mp.press = None;
                    self.make_clips.0 = None;
                } else if k == sub(K_MPR, "x") {
                    mp.release = None;
                    self.make_clips.1 = None;
                } else if k == K_MPP || k == K_MPR {
                    if !self.test {
                        if let Some(p) = cx.pick_file("Pick a sound", &[btnwin::SOUND_FILTER]) {
                            self.make_file(&p, k == K_MPR);
                            self.make_hear(&[if k == K_MPR { 0x39 } else { 0x1E }]);
                        }
                    }
                } else if k == K_MPSAVE {
                    self.make_save(cx);
                } else if k == K_MPNAME {
                    cx.focus(Some(K_MPNAME));
                }
                true
            }
            _ => true,
        }
    }

    // ------------------------------------------------------------------ the open lists + the windows

    pub(super) fn popups(&mut self, cx: &mut Cx) -> Option<El> {
        let mut kids: Vec<El> = Vec::new();
        if let Some(d) = self.key_window(cx) {
            kids.push(d);
        }
        if let Some(d) = self.many_window(cx) {
            kids.push(d);
        }
        if let Some(d) = self.make_window(cx) {
            kids.push(d);
        }
        if let Some(d) = self.getter(cx) {
            kids.push(d);
        }
        // the windows' own lists
        let ids: Vec<Code> = if self.many { self.reading(&self.pick) } else { self.sel.into_iter().collect() };
        if !ids.is_empty() {
            if let Some(m) = self.snd.popup(cx, K_SND, &self.prefs.keys, &ids) {
                kids.push(m);
            }
        }
        if let Some(code) = self.sel {
            let cur = match self.model.binds.get(code) {
                Some(Bind::Macro(id)) => Some(id.clone()),
                _ => None,
            };
            if let Some(m) = self.med.popup(cx, K_MAC, &self.model.macros, cur.as_deref(), MacroFor::Keys) {
                kids.push(m);
            }
        }
        if let Some((pack, (x, y))) = self.ask_del.clone() {
            let name = self.pack_name(&pack);
            let what = if matches!(pack, Pack::Made(_)) { "Its files are deleted from this PC." } else { "Its files are deleted from this PC. A downloaded sound can be got again." };
            let q = mitems::confirm(cx, K_DELQ, &format!("Remove {name}?"), what, "Cancel", "Remove", BKind::Red, Place::At(x, y), 260.0);
            kids.push(q.z(30));
        }
        if let Some((p, a)) = self.pop {
            let list = self.list(p);
            let rows: Vec<Row> = list
                .iter()
                .map(|(label, c, on)| match c {
                    Choice::Heading => Row::Section(label.as_str()),
                    Choice::Sep => Row::Sep,
                    _ => Row::Item(It::tick(label.as_str(), *on)),
                })
                .collect();
            // a long list (the ready-made actions, many packs) scrolls inside its 300 px box
            let at = Place::Under(a.0, a.1, a.2, a.3);
            let menu = if rows.len() > 14 { mitems::menu_scroll(cx, K_MENU, &rows, at, a.2.max(190.0)) } else { mitems::menu(cx, K_MENU, &rows, at, a.2.max(190.0)) };
            kids.push(menu.z(20));
        }
        if kids.is_empty() {
            None
        } else {
            Some(El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(WIN_W, WIN_H).no_hit().children(kids))
        }
    }

    /// Plays keys `ids` as they sound now (the pack's sound unless off for the key + its own sound); works with the sounds off.
    fn hear_keys(&self, ids: &[Code]) {
        if self.test {
            return;
        }
        self.load_pack(&self.prefs.s.pack);
        crate::pages::btnwin::load_clips(&self.prefs.keys.files());
        let mouse = |_: u16| None;
        let pad = |_: u16| None;
        let h = HearOf { dev: Dev::Keys, pack: Some(&self.prefs.s.pack), volume: self.prefs.s.volume, play_on: self.prefs.s.play_on, mouse: &mouse, pad: &pad };
        btnwin::hear(&h, &self.prefs.keys, ids);
    }

    // ------------------------------------------------------------------ input

    pub(super) fn handle(&mut self, ev: &Ev, cx: &mut Cx) {
        // the pack maker's window takes everything while it is open
        if self.make.is_some() && self.make_event(ev, cx) {
            return;
        }
        // the key window / N keys window: their Sound part, then (a key) its macro editor
        let ids: Vec<Code> = if self.many { self.reading(&self.pick) } else { self.sel.into_iter().collect() };
        if !ids.is_empty() && !self.get_open {
            let mut layers: Layers = std::mem::take(&mut self.prefs.keys);
            let o = self.snd.event(ev, cx, K_SND, &mut layers, &ids, self.test, true);
            self.prefs.keys = layers;
            if o.save {
                self.save();
            } else if o.changed && !self.test {
                glue::engine().set_layers(Dev::Keys, self.prefs.keys.clone());
            }
            if o.play {
                self.hear_keys(&ids);
            }
            if o.used {
                return;
            }
            if let (false, Some(code)) = (self.many, self.sel) {
                if self.want.unwrap_or_else(|| self.mode_of(code)) == Mode::Macro {
                    let cur = match self.model.binds.get(code) {
                        Some(Bind::Macro(id)) => Some(id.clone()),
                        _ => None,
                    };
                    let mut macros = std::mem::take(&mut self.model.macros);
                    let mo = self.med.event(ev, cx, K_MAC, &mut macros, cur.as_deref(), MacroFor::Keys);
                    self.model.macros = macros;
                    if let Some(t) = &mo.toast {
                        cx.toast(t);
                    }
                    if let Some(id) = mo.chose {
                        if id.is_empty() {
                            // deleted: every key / mouse button that ran it goes back to normal
                            if let Some(c) = &cur {
                                self.model.binds.drop_macro(c);
                                self.prefs.mouse_binds.drop_macro(c);
                            }
                            self.want = None;
                            self.save();
                        } else {
                            match self.model.set_macro(code, &id) {
                                Ok(()) => {
                                    self.err = None;
                                    self.save();
                                }
                                Err(e) => self.err = Some(e),
                            }
                        }
                    } else if mo.save {
                        self.save();
                    }
                    if mo.used {
                        return;
                    }
                }
            }
        }
        match ev {
            Ev::Press(k, x, y, r) => {
                self.press = Some((*k, *r));
                if *k == K_VOL {
                    self.set_volume(slider::value_at(*r, *x), cx);
                }
                if *k == K_REP {
                    self.set_repeat(slider::value_at(*r, *x));
                }
                self.band_press(*k, (*x, *y), *r, cx);
            }
            Ev::Drag(k, x, _, r) if *k == K_VOL => self.set_volume(slider::value_at(*r, *x), cx),
            Ev::Drag(k, x, _, r) if *k == K_REP => self.set_repeat(slider::value_at(*r, *x)),
            Ev::Drag(_, x, y, _) if self.band.is_some() => self.band_drag((*x, *y)),
            Ev::Release(k) if *k == K_VOL || *k == K_REP => self.save(),
            Ev::Click(k) => self.clicked(*k, cx),
            Ev::Char(k, c) => self.typed(*k, *c, cx),
            Ev::Key(k, vk) => self.key_down(*k, *vk, cx),
            Ev::Context(k, x, y) => self.ask_remove(*k, *x, *y),
            Ev::Blur(k) if *k == K_TARGET => self.choosing = false,
            _ => {}
        }
    }

    /// A press on the picture (a key or between them) may start a box; where the picture is comes from the pressed box.
    fn band_press(&mut self, k: Key, at: (f32, f32), r: (f32, f32, f32, f32), cx: &mut Cx) {
        let w = 544.0 - 16.0;
        let s = w / (self.w_keys * pic::U);
        let origin = if k == K_PICBG {
            Some((r.0, r.1))
        } else {
            self.keys.iter().enumerate().find_map(|(i, kb)| pic::boxes(kb, s).into_iter().enumerate().find(|(b, _)| k == idx(K_KEY, i + b * ISO_LOWER)).map(|(_, (l, t, _, _))| (r.0 - l, r.1 - t)))
        };
        self.band = origin.map(|origin| Band { origin, start: at, now: at, base: self.pick.clone(), add: cx.mods.ctrl || cx.mods.shift, moved: false });
    }

    pub(super) fn band_drag(&mut self, at: (f32, f32)) {
        let w = 544.0 - 16.0;
        let s = w / (self.w_keys * pic::U);
        let Some(b) = self.band.as_mut() else { return };
        b.now = at;
        if !b.moved && ((at.0 - b.start.0).powi(2) + (at.1 - b.start.1).powi(2)).sqrt() < 5.0 {
            return;
        }
        b.moved = true;
        let (l, r) = (b.start.0.min(at.0) - b.origin.0, b.start.0.max(at.0) - b.origin.0);
        let (t, bt) = (b.start.1.min(at.1) - b.origin.1, b.start.1.max(at.1) - b.origin.1);
        let mut hit: Vec<Code> = if b.add { b.base.clone() } else { Vec::new() };
        for k in self.keys.iter().filter(|k| !k.dead) {
            let inside = pic::boxes(k, s).into_iter().any(|(x, y, w, h)| x + w > l && x < r && y + h > t && y < bt);
            if inside && !hit.contains(&k.code) {
                hit.push(k.code);
            }
        }
        self.pick = hit;
    }

    /// Shift-click: every key inside the rectangle the two keys' centres span.
    fn range(&self, a: Code, b: Code) -> Vec<Code> {
        let c = |code: Code| self.keys.iter().find(|k| k.code == code).map(|k| (k.x + k.w / 2.0, k.y + k.h / 2.0));
        let (Some(pa), Some(pb)) = (c(a), c(b)) else { return vec![b] };
        let (x0, x1, y0, y1) = (pa.0.min(pb.0) - 0.01, pa.0.max(pb.0) + 0.01, pa.1.min(pb.1) - 0.01, pa.1.max(pb.1) + 0.01);
        self.keys
            .iter()
            .filter(|k| !k.dead)
            .filter(|k| {
                let (x, y) = (k.x + k.w / 2.0, k.y + k.h / 2.0);
                x >= x0 && x <= x1 && y >= y0 && y <= y1
            })
            .map(|k| k.code)
            .collect()
    }

    /// A click on a key (not the end of a box): Ctrl adds / takes away, Shift picks every key between, a plain click opens it.
    fn key_clicked(&mut self, n: usize, cx: &mut Cx) {
        let Some(k) = self.keys.get(n % ISO_LOWER).cloned() else { return };
        let (ctrl, shift) = (cx.mods.ctrl, cx.mods.shift);
        let code = k.code;
        if shift {
            let a = self.anchor.unwrap_or(code);
            let mut p = if ctrl { self.pick.clone() } else { Vec::new() };
            for c in self.range(a, code) {
                if !p.contains(&c) {
                    p.push(c);
                }
            }
            self.pick = p;
            if self.anchor.is_none() {
                self.anchor = Some(code);
            }
            return;
        }
        if ctrl {
            if let Some(i) = self.pick.iter().position(|c| *c == code) {
                self.pick.remove(i);
            } else {
                self.pick.push(code);
            }
            self.anchor = Some(code);
            return;
        }
        self.anchor = Some(code);
        self.pick.clear();
        self.select(Some(code));
        self.key_at = cx.now;
        self.med.opened(cx.now);
    }

    /// "Ignore repeats within": 0 (off) to 400 ms, in steps of 5 (Order 090).
    pub(super) fn set_repeat(&mut self, v: f32) {
        let max = f32::from(bu_keysound::MAX_REPEAT_MS);
        self.prefs.s.repeat_ms = ((v.clamp(0.0, 1.0) * max / 5.0).round() * 5.0) as u16;
        if !self.test && self.prefs.on {
            glue::engine().update(self.prefs.engine_settings());
        }
    }

    fn set_volume(&mut self, v: f32, _cx: &mut Cx) {
        self.prefs.s.volume = (v.clamp(0.0, 1.0) * 100.0).round() as u8;
        // only while the sounds are on: off = nothing is made, nothing is kept
        if !self.test && self.prefs.on {
            glue::engine().update(self.prefs.engine_settings());
        }
    }

    fn clicked(&mut self, k: Key, cx: &mut Cx) {
        // "Remove this sound?" - the question owns the next click
        if let Some((pack, _)) = self.ask_del.clone() {
            self.ask_del = None;
            if k == sub(K_DELQ, "go") {
                self.pop = None;
                self.remove_pack(&pack, cx);
            }
            return;
        }
        if self.get_clicked(k, cx) {
            return;
        }
        // the end of a box on the picture is not a click
        if let Some(b) = self.band.take() {
            if b.moved {
                return;
            }
        }
        // the reset line
        if k == sub(K_RESET, "pc") || k == sub(K_RESET, "win") {
            let rect = self.press.filter(|(pk, _)| *pk == k).map(|(_, r)| r).unwrap_or((0.0, 0.0, 0.0, 0.0));
            cx.open_reset(if k == sub(K_RESET, "pc") { crate::undo::Kind::HowItWas } else { crate::undo::Kind::WindowsDefaults }, rect);
            return;
        }
        // the open list
        if self.pop.is_some() {
            for i in 0..80 {
                if k == idx(K_MENU, i) {
                    let p = self.pop.map(|(p, _)| p).unwrap();
                    if let Some((_, c, _)) = self.list(p).get(i).cloned() {
                        self.choose(c, cx);
                    }
                    return;
                }
            }
        }
        // the N keys window: its × / a click beside it close it
        if self.many {
            if k == sub(K_KN, "x") || k == sub(K_KN, "out") {
                self.many = false;
                self.snd.reset();
            }
            return;
        }
        // the key's window: its × / a click beside it close it; a click inside is its own
        if self.sel.is_some() {
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
        match k {
            K_ON => {
                self.prefs.on = !self.prefs.on;
                // switching on opens the card (as Mouse acceleration)
                if self.prefs.on {
                    self.card_open = true;
                }
                self.save();
                cx.toast(if self.prefs.on { "Keyboard sounds on" } else { "Keyboard sounds off \u{b7} the app stops listening" });
            }
            K_CARDH | K_CHEV => self.card_open = !self.card_open,
            K_PICBG => {
                // a click between the keys lets go of the picked ones
                if !(cx.mods.ctrl || cx.mods.shift) {
                    self.pick.clear();
                }
            }
            K_CLR => self.pick.clear(),
            K_CHG => {
                if self.pick.len() == 1 {
                    let c = self.pick[0];
                    self.pick.clear();
                    self.select(Some(c));
                    self.key_at = cx.now;
                    self.med.opened(cx.now);
                } else if self.pick.len() > 1 {
                    self.snd.reset();
                    self.many = true;
                    self.many_at = cx.now;
                }
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
                cx.toast("Every key is back as Windows made it \u{b7} Apply remaps to save");
            }
            K_TARGET => {
                self.choosing = true;
                cx.focus(Some(K_TARGET));
            }
            K_TPICK => self.open_pop(Pop::Target, k),
            K_ACT => self.open_pop(Pop::Action, k),
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
                    self.med.reset();
                    self.save();
                    cx.toast(&format!("{} is back as Windows made it", self.label_of(c)));
                }
            }
            K_APPLY => self.start_apply(cx),
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
        // Esc lets go of the picked keys (before the page's own Esc)
        if k == crate::ui::cx::PAGE && vk == 0x1B && !self.pick.is_empty() && self.sel.is_none() && !self.many {
            self.pick.clear();
            cx.used = true;
            return;
        }
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
        }
    }
}

/// The key combo for a key press: the modifiers held now (Ctrl, Alt, Shift, Win - in this order) then the key. A modifier
/// pressed alone is not a combo yet (None). Windows only reads the held modifiers.
pub fn combo_of(vk: u16) -> Option<Vec<u16>> {
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
