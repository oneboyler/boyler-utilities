//! The Notifications for OBS tab (Order 035; addons-v1 page `ntf`, icon `bell`): ClipPing's own settings window (settings.c)
//! recreated with the menu's shared pieces - every setting, the same groups in the same order: Keys (+ the scene list it
//! switches through), Recording, Sounds, Popups (+ the preview), and the fullscreen warning line. ClipPing's "Start with
//! Windows" is the app's own (Settings) and its "Check for updates" too (A_035_01).
//! OBS-side values (OBS's own keys, clip length, FPS, clips folder) wait in the page until "Apply to OBS" on the bar at the
//! bottom (ClipPing's Done, with its one confirmation when OBS or instant replay must restart); everything else applies at
//! once, and a Popups change shows ONE silent test popup exactly as it will look (ClipPing's rule).
//! The tab is in the top row only while the add-on is on (`crate::addons`).

use std::collections::HashMap;
use std::path::PathBuf;

use bu_obs::engine::{needs, ObsChange, PopMsg};
use bu_obs::keys::KeyBind;
use bu_obs::popup::{anim_frame, custom_xy, pick_corner};
use bu_obs::settings::{self as st, Settings};
use bu_obs::sound::{self, Sound, SOUND_LABELS};
use skia_safe as sk;

use crate::gfx::{Align, Font, Rgba};
use crate::pages::{Background, Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, lh, sub, El, Key};
use crate::ui::pieces::button::{self, Kind};
use crate::ui::pieces::dropdown::{self, Item};
use crate::ui::pieces::inote::{self, inote};
use crate::ui::pieces::rowbits;
use crate::ui::pieces::selbar::{self, Sbb};
use crate::ui::pieces::{self, dialog, group, keyfield, mitems, slider, toggle};
use crate::ui::{AMBER, FG, FG2, FG3, HAIR, WELL};

const K_KEY: [Key; 4] = [key("ntf.key0"), key("ntf.key1"), key("ntf.key2"), key("ntf.key3")];
const K_SCENE: Key = key("ntf.scene");
const K_SDEL: Key = key("ntf.sdel");
const K_ADD: Key = key("ntf.add");
const K_CLIP: Key = key("ntf.cliplen");
const K_FPS: Key = key("ntf.fps");
const K_OPEN: Key = key("ntf.open");
const K_CHANGE: Key = key("ntf.change");
const K_KEEP: Key = key("ntf.keep");
const K_OFFREC: Key = key("ntf.offrec");
const K_STARTOBS: Key = key("ntf.startobs");
const K_SND: Key = key("ntf.snd");
const K_PLAY: Key = key("ntf.play");
const K_VOL: Key = key("ntf.vol");
const K_WHERE: Key = key("ntf.where");
const K_POS: Key = key("ntf.pos");
const K_STYLE: Key = key("ntf.style");
const K_BG: Key = key("ntf.bg");
const K_ANIM: Key = key("ntf.anim");
const K_STATUS: Key = key("ntf.status");
const K_MENU: Key = key("ntf.menu");
const K_BAR: Key = key("ntf.bar");
const K_CF: Key = key("ntf.cf");
const K_DLG: Key = key("ntf.dlg");
const K_DLG_OK: Key = key("ntf.dlgok");
const K_CONNECT: Key = key("ntf.connect");
const K_CON_DLG: Key = key("ntf.condlg");
const K_CON_GO: Key = key("ntf.congo");
const K_CON_NO: Key = key("ntf.conno");
const K_OTHER: Key = key("ntf.other");

const CLIPLENS: [i32; 10] = [10, 15, 20, 30, 45, 60, 90, 120, 180, 300];
const FPSS: [i32; 5] = [3000, 6000, 12000, 14400, 24000];
const WARN: &str = "Popups can't show over fullscreen games on the recording monitor. Use borderless / windowed, or pick Other monitor.";
const CONNECT_TEXT: &str = "Notifications for OBS needs to turn on OBS's built-in remote control. OBS will close and reopen for a few seconds.";
const CONNECT_NOTE: &str = "This lets Notifications for OBS change OBS settings for you and get notified when clips are saved, recordings start, or instant replay turns on and off. It's password protected, and you can turn it off anytime in OBS under Tools \u{2192} WebSocket Server Settings.";

/// The open dropdown: which one, its anchor, its items (label, value).
#[derive(Clone)]
struct Menu {
    of: Key,
    anchor: (f32, f32, f32, f32),
    items: Vec<(String, i32, bool)>,
}

#[derive(Default)]
pub struct Notifications {
    env: Env,
    menu: Option<Menu>,
    boxes: HashMap<Key, (f32, f32, f32, f32)>,
    /// OBS-side edits waiting for Apply (ClipPing's M): clip length, FPS (hundredths), folder
    cliplen: Option<i32>,
    fps: Option<i32>,
    folder: Option<PathBuf>,
    /// the Apply confirmation (text) under the bar's button
    confirm: Option<String>,
    /// an engine message box (ClipPing's dialog_confirm)
    dialog: Option<(String, f64)>,
    /// "Connect to OBS" open since
    connect: Option<f64>,
    asked: bool,
    /// the preview plays the animation since
    prev_t0: Option<f64>,
    played: [Option<f64>; 4],
    /// the drag of the volume slider
    vol_drag: Option<(f32, f32, f32, f32)>,
    jump_to: Option<Key>,
    /// the Switch scene field was used (its new key becomes the setting once it is set)
    switch_touched: bool,
    /// test copies: what would have opened (a folder) - the test hook shows it
    pub reqs: Vec<String>,
}

fn set_now() -> Settings {
    crate::obs::settings().unwrap_or_default()
}

fn fps_text(v: i32) -> String {
    if v % 100 == 0 {
        format!("{}", v / 100)
    } else {
        format!("{}.{:02}", v / 100, v % 100)
    }
}

impl Notifications {
    fn cliplen(&self, v: &bu_obs::View) -> i32 {
        self.cliplen.unwrap_or(v.cliplen)
    }
    fn fps(&self, v: &bu_obs::View) -> i32 {
        self.fps.unwrap_or(v.fps)
    }
    fn folder(&self, v: &bu_obs::View) -> PathBuf {
        self.folder.clone().unwrap_or_else(|| v.folder.clone())
    }

    /// The OBS keys being edited: (index, the keys manager's key) where it differs from OBS's own.
    fn key_edits(&self) -> Vec<(usize, Option<KeyBind>)> {
        let obs = crate::obs::obs_keys();
        let errs = crate::obs::key_errors();
        (0..3)
            .filter_map(|i| {
                let id = crate::obs::ACTIONS[i].0;
                let cur = crate::obs::current_key(id);
                (cur != obs[i] && !errs.iter().any(|(e, _)| e == id)).then_some((i, cur))
            })
            .collect()
    }

    fn pending(&self, v: &bu_obs::View) -> bool {
        !v.applying && (!self.key_edits().is_empty() || self.cliplen.is_some_and(|c| c != v.cliplen) || self.fps.is_some_and(|f| f != v.fps) || self.folder.as_ref().is_some_and(|f| *f != v.folder))
    }

    fn change(&self, v: &bu_obs::View) -> ObsChange {
        let obs = crate::obs::obs_keys();
        let mut c = ObsChange::default();
        let edits = self.key_edits();
        for i in 0..3 {
            c.key[i] = edits.iter().find(|(j, _)| *j == i).map(|(_, k)| *k).unwrap_or(obs[i]);
            // a key OBS has no name for can't be written: OBS keeps its own
            if c.key[i].is_some_and(|k| bu_obs::keys::binding_json(k).is_none()) {
                c.key[i] = obs[i];
            }
        }
        c.keys_changed = c.key != obs;
        let cl = self.cliplen(v);
        c.cliplen = cl;
        c.cliplen_changed = cl != v.cliplen && cl > 0;
        let f = self.fps(v);
        c.fps = f / 100;
        c.fps_changed = f != v.fps && f > 0;
        let fo = self.folder(v);
        c.folder_changed = !fo.as_os_str().is_empty() && !fo.to_string_lossy().eq_ignore_ascii_case(&v.folder.to_string_lossy());
        c.folder = fo;
        c
    }

    /// Undo: OBS's own values back (keys included).
    fn undo(&mut self) {
        let obs = crate::obs::obs_keys();
        crate::services::try_with(|s| {
            for (i, (id, _, _)) in crate::obs::ACTIONS.iter().take(3).enumerate() {
                let _ = match obs[i] {
                    Some(k) => s.keys.bind(&mut s.store, id, crate::keys::Combo::new(mods_of(k), k.vk)).map(|_| ()),
                    None => s.keys.unbind(&mut s.store, id).map(|_| ()),
                };
            }
        });
        self.cliplen = None;
        self.fps = None;
        self.folder = None;
        self.confirm = None;
    }

    fn apply(&mut self, v: &bu_obs::View) {
        let c = self.change(v);
        if c.keys_changed || c.cliplen_changed || c.folder_changed || c.fps_changed {
            crate::obs::apply(c);
        }
        self.cliplen = None;
        self.fps = None;
        self.folder = None;
        self.confirm = None;
    }

    fn open_menu(&mut self, of: Key, set: &Settings, v: &bu_obs::View) {
        let anchor = self.boxes.get(&of).copied().unwrap_or((0.0, 0.0, 120.0, 24.0));
        let mut items: Vec<(String, i32, bool)> = Vec::new();
        let mk = |names: &[&str], sel: i32| -> Vec<(String, i32, bool)> { names.iter().enumerate().map(|(i, n)| (n.to_string(), i as i32, i as i32 == sel)).collect() };
        if of == K_CLIP {
            let cur = self.cliplen(v);
            let mut placed = false;
            for c in CLIPLENS {
                if !placed && cur < c && cur > 0 {
                    items.push((format!("{cur} sec"), cur, true));
                    placed = true;
                }
                if c == cur {
                    placed = true;
                }
                items.push((format!("{c} sec"), c, c == cur));
            }
            if !placed && cur > 0 {
                items.push((format!("{cur} sec"), cur, true));
            }
        } else if of == K_FPS {
            let cur = self.fps(v);
            let mut placed = false;
            for f in FPSS {
                if !placed && cur < f && cur > 0 {
                    items.push((fps_text(cur), cur, true));
                    placed = true;
                }
                if f == cur {
                    placed = true;
                }
                items.push((fps_text(f), f, f == cur));
            }
            if !placed && cur > 0 {
                items.push((fps_text(cur), cur, true));
            }
        } else if of == K_WHERE {
            items = mk(&st::WHERE_NAMES, set.where_);
        } else if of == K_POS {
            items = mk(&st::POS_NAMES, set.pos);
        } else if of == K_STYLE {
            items = st::STYLE_ORDER.iter().map(|s| (st::style_name(*s).to_string(), *s, *s == set.style)).collect();
        } else if of == K_BG {
            items = mk(&st::BG_NAMES, set.bg);
        } else if of == K_ANIM {
            items = mk(&st::ANIM_NAMES, set.anim);
        } else if of == K_STATUS {
            items = mk(&st::STATUS_NAMES, set.status);
        } else if of == K_ADD {
            for s in &v.scenes {
                if !set.scenes.contains(&s.name) && items.len() < 20 {
                    items.push((s.name.clone(), items.len() as i32, false));
                }
            }
            if items.is_empty() {
                items.push((if v.connected { "(no other scenes)" } else { "(OBS not connected)" }.to_string(), -1, false));
            }
        } else if let Some(e) = (0..4).find(|e| of == idx(K_SND, *e)) {
            let ev = [Sound::Saved, Sound::Failed, Sound::Changed, Sound::Warning][e];
            items.push(("None".into(), 0, set.snd[e] == 0));
            for i in 1..=sound::opt_count(ev) as i32 {
                items.push((sound::opt_name(ev, i).to_string(), i, set.snd[e] == i));
            }
            items.push(("Custom file\u{2026}".into(), st::SND_CUSTOM, set.snd[e] == st::SND_CUSTOM));
        }
        self.menu = Some(Menu { of, anchor, items });
    }

    fn choose(&mut self, i: usize, cx: &mut Cx) {
        let Some(m) = self.menu.take() else { return };
        let Some((label, v, _)) = m.items.get(i).cloned() else { return };
        let mut set = set_now();
        let of = m.of;
        if of == K_CLIP {
            self.cliplen = Some(v);
        } else if of == K_FPS {
            self.fps = Some(v);
        } else if of == K_WHERE {
            set.where_ = v;
            crate::obs::popups_changed(set);
        } else if of == K_POS {
            if v == st::P_CUSTOM {
                crate::obs::place_open(); // saved (and shown) there on Enter
            } else {
                set.pos = v;
                crate::obs::popups_changed(set);
            }
        } else if of == K_STYLE {
            set.style = v;
            crate::obs::popups_changed(set);
        } else if of == K_BG {
            if v == 6 {
                if let Some(c) = crate::obs::pick_color(set.bgcustom, self.env.fake()) {
                    set.bgcustom = c;
                    set.bg = 6;
                }
            } else {
                set.bg = v;
            }
            crate::obs::popups_changed(set);
        } else if of == K_ANIM {
            set.anim = v;
            crate::obs::popups_changed(set);
            // the preview plays it once
            self.prev_t0 = Some(cx.now);
        } else if of == K_STATUS {
            set.status = v;
            crate::obs::set_settings(set);
        } else if of == K_ADD {
            if v >= 0 && set.scenes.len() < st::MAX_LIST {
                set.scenes.push(label);
                set.scenes_init = true;
                crate::obs::set_settings(set);
            }
        } else if let Some(e) = (0..4).find(|e| of == idx(K_SND, *e)) {
            if v == st::SND_CUSTOM {
                if let Some(p) = cx.pick_file("Choose a sound", &[("Sound (*.wav)", "*.wav")]) {
                    set.sndfile[e] = p;
                    set.snd[e] = st::SND_CUSTOM;
                }
            } else {
                set.snd[e] = v;
            }
            crate::obs::set_settings(set);
        }
    }

    fn combo_text(&self, of: Key, set: &Settings, v: &bu_obs::View) -> String {
        if of == K_CLIP {
            return format!("{} sec", self.cliplen(v));
        }
        if of == K_FPS {
            let f = self.fps(v);
            return if f > 0 { fps_text(f) } else { String::new() };
        }
        if of == K_WHERE {
            return st::WHERE_NAMES[set.where_.clamp(0, 4) as usize].into();
        }
        if of == K_POS {
            return if set.pos == st::P_CUSTOM { "Custom".into() } else { st::POS_NAMES[set.pos.clamp(0, 7) as usize].into() };
        }
        if of == K_STYLE {
            return st::style_name(set.style).into();
        }
        if of == K_BG {
            return if set.bg == 6 { "Custom".into() } else { st::BG_NAMES[set.bg.clamp(0, 6) as usize].into() };
        }
        if of == K_ANIM {
            return st::ANIM_NAMES[set.anim.clamp(0, 2) as usize].into();
        }
        if of == K_STATUS {
            return st::STATUS_NAMES[set.status.clamp(0, 4) as usize].into();
        }
        if let Some(e) = (0..4).find(|e| of == idx(K_SND, *e)) {
            let ev = [Sound::Saved, Sound::Failed, Sound::Changed, Sound::Warning][e];
            return match set.snd[e] {
                0 => "None".into(),
                st::SND_CUSTOM => {
                    let f = &set.sndfile[e];
                    let b = f.rsplit(['\\', '/']).next().unwrap_or("");
                    if b.is_empty() {
                        "Custom file\u{2026}".into()
                    } else {
                        b.into()
                    }
                }
                i => sound::opt_name(ev, i).into(),
            };
        }
        String::new()
    }

    /// ClipPing's `disabled`: with "Show on: None" the look settings grey out; Background with Floating text (and Glass,
    /// which has its own tint).
    fn disabled(of: Key, set: &Settings) -> bool {
        (set.where_ == st::W_NONE && (of == K_POS || of == K_STYLE || of == K_BG || of == K_ANIM)) || (of == K_BG && (set.style == st::ST_FLOAT || set.style == st::ST_GLASS))
    }

    fn dd(&self, cx: &mut Cx, of: Key, set: &Settings, v: &bu_obs::View, w: f32) -> El {
        let t = self.combo_text(of, set, v);
        let d = dropdown::dropdown(cx, of, &t, Some(w));
        if Self::disabled(of, set) {
            d.opacity(0.4)
        } else {
            d
        }
    }

    /// the fullscreen warning: popups set to appear on the monitor being recorded
    fn warn(set: &Settings, v: &bu_obs::View) -> bool {
        if set.where_ == st::W_NONE {
            return false;
        }
        if set.where_ == st::W_SAME {
            return true;
        }
        v.clipped.is_some() && bu_obs::monitors::pick_monitor(&v.mons, set.where_, v.clipped) == v.clipped
    }

    fn key_row(&self, cx: &mut Cx, i: usize, first: bool) -> Vec<El> {
        let (id, name, _) = crate::obs::ACTIONS[i];
        let f = keyfield::action_field(cx, K_KEY[i], id, false);
        let mut out = vec![group::row(first, vec![group::lbl(name, None), group::ctl(vec![f])])];
        let err = cx.key_field(id).2.or_else(|| crate::obs::key_errors().into_iter().find(|(a, _)| a == id).map(|(_, e)| format!("OBS's key can't be used here: {e}")));
        if let Some(e) = err {
            out.push(keyfield::error_line(&e).margin(0.0, 12.0, 6.0, 12.0));
        }
        out
    }

    /// The preview (settings.c `draw_preview`): the popup at about 1.5 x its real size, where it will appear, in the chosen
    /// look; after an Animation change it plays the animation once.
    fn preview(&self, cx: &mut Cx, set: &Settings, v: &bu_obs::View) -> El {
        const PH: f32 = 150.0;
        let none = set.where_ == st::W_NONE;
        let label = El::text("Preview", Font::new(11.0, 400), if none { FG3().mul_a(0.6) } else { FG3() }, lh(11.0, 1.35)).abs(10.0, 6.0, f32::NAN, f32::NAN).no_hit();
        let mut b = El::block().h(PH).radius(8.0).bg(WELL()).inset(&[crate::gfx::sh(0.0, 0.0, 0.0, 0.5, HAIR())]).clip().child(label);
        if none {
            return b;
        }
        let scale = cx.g.scale;
        let m = PopMsg::sample();
        let t = self.prev_t0.map(|t0| ((cx.now - t0) * 1000.0) as i32);
        let frame = t.and_then(|t| anim_frame(set.anim, 0, 0, t));
        let barq = frame.map(|f| f.3).unwrap_or(650);
        let corner = pick_corner(set, &v.mons, bu_obs::monitors::pick_monitor(&v.mons, set.where_, v.clipped), v.clipped);
        let mons_ok = !v.mons.is_empty();
        let _ = mons_ok;
        let set2 = set.clone();
        let anim = set.anim;
        let glass = set.style == st::ST_GLASS;
        // ClipPing's looks: its own picture (device px at the menu's scale), handed to Skia
        let img: Option<(sk::Image, f32, f32)> = if glass {
            None
        } else {
            let k = set.scale * 150 / 100;
            let i = crate::obs::gdi::popup_draw(&m, set.style, set.popup_bg(), k, (96.0 * scale).round() as i32, barq);
            let px = crate::png::Pixels { w: i.w as u32, h: i.h as u32, data: i.px.iter().flat_map(|p| p.to_le_bytes()).collect() };
            crate::png::to_image(&px).map(|im| (im, i.w as f32 / scale, i.h as f32 / scale))
        };
        let gl_k = set.scale as f32 * 1.5 / 100.0;
        b = b.child(
            El::paint(move |g, (x, y, w, h)| {
                // the area stands for the monitor's work area (the popup keeps its usual margin from the edge)
                let (bx, by, bw, bh) = (x + 12.0, y + 24.0, w - 24.0, h - 36.0);
                let (mut pw, mut ph, mut k) = match &img {
                    Some((_, iw, ih)) => (*iw, *ih, 1.0f32),
                    None => {
                        let (gw, gh) = crate::obs::glass::measure(g, &m);
                        (gw * gl_k, gh * gl_k, gl_k)
                    }
                };
                // shrink only if it doesn't fit
                let fit = (bw / pw).min(bh / ph).min(1.0);
                pw *= fit;
                ph *= fit;
                k *= fit;
                let (px, py, dir) = if corner == st::P_CUSTOM {
                    let wk = bu_obs::monitors::Rect { left: bx as i32, top: by as i32, right: (bx + bw) as i32, bottom: (by + bh) as i32 };
                    let (x2, y2) = custom_xy(&wk, pw as i32, ph as i32, set2.cx, set2.cy);
                    (x2 as f32, y2 as f32, 3)
                } else {
                    let col = corner % 3;
                    let px = match col {
                        0 => bx,
                        1 => bx + (bw - pw) / 2.0,
                        _ => bx + bw - pw,
                    };
                    let py = if corner <= st::P_TR { by } else { by + bh - ph };
                    (px, py, if col == 0 { 0 } else if col == 2 { 1 } else if corner <= st::P_TR { 2 } else { 3 })
                };
                let (mut dx, mut dy, mut al) = (0.0f32, 0.0f32, 1.0f32);
                if let Some(t) = t {
                    if let Some((fx, fy, a, _)) = anim_frame(anim, dir, (24.0 * k) as i32, t) {
                        dx = fx as f32;
                        dy = fy as f32;
                        al = a as f32 / 255.0;
                    }
                }
                g.push_layer(al, None);
                match &img {
                    Some((im, _, _)) => g.draw_image_rect(im, px + dx, py + dy, pw, ph),
                    None => {
                        let cv = g.cv();
                        cv.save();
                        cv.translate(((px + dx) * g.scale, (py + dy) * g.scale));
                        cv.scale((k, k));
                        crate::obs::glass::draw(g, &m, 0.0, 0.0, &crate::obs::glass::Palette::current());
                        cv.restore();
                    }
                }
                g.pop_layer();
            })
            .abs(0.0, 0.0, 0.0, 0.0),
        );
        b
    }
}

fn mods_of(k: KeyBind) -> crate::keys::Mods {
    let mut m = crate::keys::Mods::NONE;
    if k.mods & bu_obs::keys::MOD_C != 0 {
        m = m.with(crate::keys::Mods::CTRL);
    }
    if k.mods & bu_obs::keys::MOD_A != 0 {
        m = m.with(crate::keys::Mods::ALT);
    }
    if k.mods & bu_obs::keys::MOD_S != 0 {
        m = m.with(crate::keys::Mods::SHIFT);
    }
    if k.mods & bu_obs::keys::MOD_W != 0 {
        m = m.with(crate::keys::Mods::WIN);
    }
    m
}

/// The feature's part that lives as long as the app: the engine runs while the add-on is on.
struct Running;
impl Background for Running {
    fn describe(&self) -> String {
        format!("obs on={}", crate::obs::running())
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        crate::obs::stop();
    }
}

impl Page for Notifications {
    fn id(&self) -> &'static str {
        crate::obs::PAGE
    }
    fn name(&self) -> &'static str {
        "Notifications for OBS"
    }
    fn icon(&self) -> &'static str {
        "bell"
    }

    fn background(&self, env: &Env) -> Option<Box<dyn Background>> {
        if crate::addons::is_on("obs") {
            crate::obs::start(env.test);
        }
        Some(Box::new(Running))
    }

    fn open(&mut self, env: &Env, _now: f64) {
        self.env = env.clone();
        self.asked = false;
        crate::obs::refresh_other_app();
    }

    fn close(&mut self) {
        self.menu = None;
        self.confirm = None;
        self.prev_t0 = None;
        self.vol_drag = None;
    }

    fn jump(&mut self, target: &str) {
        if let Some(i) = crate::obs::ACTIONS.iter().position(|a| a.0 == target) {
            self.jump_to = Some(K_KEY[i]);
        }
    }

    fn tick(&mut self, now: f64) -> bool {
        if let Some(t0) = self.prev_t0 {
            if now - t0 > 3.2 {
                self.prev_t0 = None;
            }
        }
        self.prev_t0.is_some()
    }

    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        let now = cx.now;
        let mut out = vec![pieces::header(self.name(), None)];
        let (Some(v), Some(set)) = (crate::obs::view(), crate::obs::settings()) else {
            out.push(inote("Notifications for OBS is off. Switch it on in Add-ons.", true, &inote::UDW));
            return out;
        };
        if let Some(k) = self.jump_to.take() {
            cx.scroll_to(k);
        }
        // the Switch scene key is the feature's own: a key the user set (or cleared) in its field becomes the setting - only after
        // a touch of that field (a key the keys manager refused must not erase the saved one)
        let sw = crate::obs::current_key("obs.switch");
        if self.switch_touched && sw != set.switch_key && cx.key_field("obs.switch").1.is_none() {
            self.switch_touched = false;
            let mut s2 = set.clone();
            s2.switch_key = sw;
            crate::obs::set_settings(s2);
        }
        // an engine message box
        if self.dialog.is_none() {
            if let Some(t) = crate::obs::take_dialog() {
                self.dialog = Some((t, now));
            }
        }
        // "Connect to OBS": asked once when the tab opens while remote control is off
        if v.ask_connect && !self.asked && self.connect.is_none() {
            self.asked = true;
            self.connect = Some(now);
        }
        if self.prev_t0.is_some() {
            cx.st.busy = true;
        }

        // the lines above the groups
        if crate::obs::other_app().is_some() {
            out.push(
                El::row()
                    .center()
                    .gap(10.0)
                    .margin(4.0, 0.0, 0.0, 0.0)
                    .child(inote("Notifications for OBS is also running on its own - close it?", false, &inote::UDW).flex1())
                    .child(button::cbtn(cx, K_OTHER, "Close", Kind::Ghost, true, false, 0.0)),
            );
        }
        if v.ask_connect {
            out.push(
                El::row()
                    .center()
                    .gap(10.0)
                    .margin(4.0, 0.0, 0.0, 0.0)
                    .child(inote("OBS remote control is off", false, &inote::UDW).flex1())
                    .child(button::cbtn(cx, K_CONNECT, "Connect\u{2026}", Kind::Primary, true, false, 0.0)),
            );
        }

        // ---- Keys (+ the scenes the Switch scene key goes through)
        out.push(group::gh("Keys"));
        let mut rows = Vec::new();
        for i in 0..4 {
            rows.extend(self.key_row(cx, i, i == 0));
        }
        out.push(group::grp(rows));
        out.push(group::gf("Switches between these scenes, in this order"));
        let mut srows = Vec::new();
        for (i, s) in set.scenes.iter().enumerate() {
            let mon = v.scenes.iter().find(|x| x.name == *s).map(|x| x.mon).unwrap_or(0);
            let rk = idx(K_SCENE, i);
            let hv = cx.hovered(rk) || cx.hovered(idx(K_SDEL, i));
            let mut l = El::row().center().gap(8.0).flex1().child(El::text(s.clone(), Font::new(13.0, 400), FG(), lh(13.0, 1.35)).ellipsis().shrink(1.0).min_w(0.0));
            if mon > 0 {
                l = l.child(El::text(format!("monitor {mon}"), Font::new(11.0, 400), FG3(), lh(11.0, 1.35)).none());
            }
            srows.push(group::row(i == 0, vec![l, group::ctl(vec![rowbits::rdel(cx, idx(K_SDEL, i), hv)])]).key(rk));
        }
        let add = El::text("+ Add scene", Font::new(13.0, 400), crate::ui::ACC(), lh(13.0, 1.35)).no_hit();
        srows.push(group::row(set.scenes.is_empty(), vec![El::row().flex1().child(add)]).on_click(K_ADD).cursor(crate::ui::el::Cursor::Hand));
        out.push(group::grp(srows).margin(8.0, 0.0, 0.0, 0.0));

        // ---- Recording
        out.push(group::gh("Recording"));
        let folder = self.folder(&v);
        let ftxt = if folder.as_os_str().is_empty() { "(unknown)".to_string() } else { folder.to_string_lossy().into_owned() };
        let fps_dd = self.dd(cx, K_FPS, &set, &v, 72.0);
        let clip_dd = self.dd(cx, K_CLIP, &set, &v, 96.0);
        out.push(group::grp(vec![
            group::row(true, vec![group::lbl("Clip length", None), group::ctl(vec![clip_dd, El::text("FPS", Font::new(13.0, 400), FG2(), lh(13.0, 1.35)).none().margin(0.0, 0.0, 0.0, 6.0), fps_dd])]),
            group::row(
                false,
                vec![
                    group::lbl("Folder", None),
                    group::ctl(vec![
                        El::text(ftxt.clone(), Font::new(12.0, 400), FG2(), lh(12.0, 1.35)).ellipsis().max_w(250.0).tip(&ftxt),
                        button::icon_btn(cx, K_OPEN, "open", 15.0, 1.5).tip("Open folder"),
                        button::icon_btn(cx, K_CHANGE, "fold", 15.0, 1.5).tip("Change folder"),
                    ]),
                ],
            ),
            group::row(false, vec![group::lbl("Keep instant replay on at all times", None), group::ctl(vec![toggle::toggle(cx, K_KEEP, set.keep_rb, false)])]),
            group::row(false, vec![group::lbl("Turn off instant replay while recording", None), group::ctl(vec![toggle::toggle(cx, K_OFFREC, set.off_rec, false)])]),
            group::row(false, vec![group::lbl("Start OBS with Windows", None), group::ctl(vec![toggle::toggle(cx, K_STARTOBS, v.start_obs_shown, false)])]),
        ]));

        // ---- Sounds
        out.push(group::gh("Sounds"));
        let mut rows = Vec::new();
        for (e, lab) in SOUND_LABELS.iter().enumerate() {
            let d = self.dd(cx, idx(K_SND, e), &set, &v, 150.0);
            let p = rowbits::pb(cx, idx(K_PLAY, e), self.played[e], 1, set.snd[e] == 0 || set.vol == 0);
            rows.push(group::row(e == 0, vec![group::lbl(lab, None), group::ctl(vec![d, p])]));
        }
        let vol = slider::slider(cx, K_VOL, set.vol as f32 / 100.0, 150.0, 22.0, slider::default());
        rows.push(group::row(false, vec![group::lbl("Volume", None), group::ctl(vec![vol, slider::value_label(&format!("{} %", set.vol))])]));
        out.push(group::grp(rows));

        // ---- Popups
        out.push(group::gh("Popups"));
        let (wh, pos, sty, bg, an, stt) = (
            self.dd(cx, K_WHERE, &set, &v, 150.0),
            self.dd(cx, K_POS, &set, &v, 150.0),
            self.dd(cx, K_STYLE, &set, &v, 150.0),
            self.dd(cx, K_BG, &set, &v, 150.0),
            self.dd(cx, K_ANIM, &set, &v, 150.0),
            self.dd(cx, K_STATUS, &set, &v, 150.0),
        );
        let pv = self.preview(cx, &set, &v);
        out.push(group::grp(vec![
            group::row(true, vec![group::lbl("Show on", None), group::ctl(vec![wh, pos])]),
            group::row(false, vec![group::lbl("Look", None), group::ctl(vec![sty, bg])]),
            group::row(false, vec![group::lbl("Animation", None), group::ctl(vec![an])]),
            group::row(false, vec![group::lbl("Status icon", None), group::ctl(vec![stt])]),
            El::block().pad(6.0, 12.0, 12.0, 12.0).child(pv),
        ]));
        if Self::warn(&set, &v) {
            out.push(El::row().gap(7.0).margin(8.0, 12.0, 0.0, 12.0).child(El::icon("tri", 14.0, 1.5, AMBER()).none()).child(El::text(WARN, Font::new(11.0, 400), AMBER(), 15.0).wrapping().shrink(1.0).min_w(0.0)));
        }
        // room above the Apply bar while it shows
        if self.pending(&v) {
            out.push(El::block().h(54.0));
        }
        let _ = Rgba(0.0, 0.0, 0.0, 0.0);
        let _ = Align::Left;
        out
    }

    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        let now = cx.now;
        if let Ev::Press(k, _, _, r) = ev {
            self.boxes.insert(*k, *r);
        }
        let (Some(v), Some(mut set)) = (crate::obs::view(), crate::obs::settings()) else { return };
        // the key fields: the keys manager listens / clears (OBS's keys wait for Apply; the Switch scene key is the feature's)
        for (i, (id, _, _)) in crate::obs::ACTIONS.iter().enumerate() {
            if keyfield::action_event(ev, cx, K_KEY[i], id) {
                if i == 3 {
                    self.switch_touched = true;
                }
                cx.dirty = true;
                return;
            }
        }
        match ev {
            Ev::Click(k) if (0..64).any(|i| *k == idx(K_SDEL, i)) => {
                let i = (0..64).find(|i| *k == idx(K_SDEL, *i)).unwrap_or(0);
                if i < set.scenes.len() {
                    set.scenes.remove(i);
                    set.scenes_init = true;
                    crate::obs::set_settings(set);
                }
            }
            Ev::Click(k) if *k == K_ADD || *k == K_CLIP || *k == K_FPS || *k == K_WHERE || *k == K_POS || *k == K_STYLE || *k == K_BG || *k == K_ANIM || *k == K_STATUS || (0..4).any(|e| *k == idx(K_SND, e)) => {
                if Self::disabled(*k, &set) {
                    return;
                }
                if self.menu.as_ref().is_some_and(|m| m.of == *k) {
                    self.menu = None;
                } else {
                    self.open_menu(*k, &set, &v);
                }
            }
            Ev::Click(k) if self.menu.is_some() && (0..64).any(|i| *k == idx(K_MENU, i)) => {
                let i = (0..64).find(|i| *k == idx(K_MENU, *i)).unwrap_or(0);
                if self.menu.as_ref().and_then(|m| m.items.get(i)).is_some_and(|it| it.1 >= 0 || m_is_value_list(self.menu.as_ref())) {
                    self.choose(i, cx);
                }
            }
            Ev::Click(k) if (0..4).any(|e| *k == idx(K_PLAY, e)) => {
                let e = (0..4).find(|e| *k == idx(K_PLAY, *e)).unwrap_or(0);
                self.played[e] = Some(now);
                crate::obs::preview_sound(e);
            }
            Ev::Press(k, x, _, r) if *k == K_VOL => {
                self.vol_drag = Some(*r);
                set.vol = (slider::value_at(*r, *x) * 100.0).round() as i32;
                crate::obs::set_settings_quiet(set);
            }
            Ev::Drag(k, x, _, _) if *k == K_VOL => {
                if let Some(r) = self.vol_drag {
                    set.vol = (slider::value_at(r, *x) * 100.0).round() as i32;
                    crate::obs::set_settings_quiet(set);
                }
            }
            Ev::Release(k) if *k == K_VOL => {
                self.vol_drag = None;
                crate::obs::set_settings(set);
            }
            Ev::Click(k) if *k == K_KEEP => {
                set.keep_rb = !set.keep_rb;
                crate::obs::set_settings(set);
            }
            Ev::Click(k) if *k == K_OFFREC => {
                set.off_rec = !set.off_rec;
                crate::obs::set_settings(set);
            }
            // saves (or explains) itself in the engine
            Ev::Click(k) if *k == K_STARTOBS => crate::obs::send(bu_obs::Input::StartObsToggle),
            Ev::Click(k) if *k == K_OPEN => {
                let f = self.folder(&v);
                if self.env.fake() {
                    self.reqs.push(format!("open {}", f.display()));
                } else if !f.as_os_str().is_empty() {
                    crate::obs::open_folder(&f);
                }
            }
            Ev::Click(k) if *k == K_CHANGE => {
                if let Some(p) = cx.pick_folder("Choose the clips folder") {
                    self.folder = Some(PathBuf::from(p));
                }
            }
            // the Apply bar: Undo / Apply to OBS / ×
            Ev::Click(k) if *k == idx(K_BAR, 0) || *k == idx(K_BAR, 2) => self.undo(),
            Ev::Click(k) if *k == idx(K_BAR, 1) => {
                let c = self.change(&v);
                let (ro, rr) = needs(&c, v.obs_running, v.connected, v.replay);
                let mut msg = String::new();
                if ro {
                    msg.push_str("OBS will restart for a few seconds to apply the new keys.");
                }
                if rr {
                    if !msg.is_empty() {
                        msg.push('\n');
                    }
                    msg.push_str("Instant replay will restart for a few seconds.");
                }
                if msg.is_empty() {
                    self.apply(&v);
                } else {
                    self.confirm = Some(msg);
                }
            }
            Ev::Click(k) if *k == sub(K_CF, "go") => self.apply(&v),
            // Cancel: OBS's values back, the page stays
            Ev::Click(k) if *k == sub(K_CF, "no") => self.undo(),
            Ev::Click(k) if *k == K_DLG_OK || *k == sub(K_DLG, "x") || *k == sub(K_DLG, "out") => self.dialog = None,
            Ev::Click(k) if *k == K_CONNECT => self.connect = Some(now),
            Ev::Click(k) if *k == K_CON_GO => {
                self.connect = None;
                crate::obs::send(bu_obs::Input::Connect(true));
            }
            Ev::Click(k) if *k == K_CON_NO || *k == sub(K_CON_DLG, "x") || *k == sub(K_CON_DLG, "out") => {
                self.connect = None;
                crate::obs::send(bu_obs::Input::Connect(false));
            }
            Ev::Click(k) if *k == K_OTHER => crate::obs::close_other_app(),
            _ => {}
        }
        cx.dirty = true;
    }

    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let now = cx.now;
        if let Some((t, at)) = self.dialog.clone() {
            let ok = button::cbtn_sized(cx, K_DLG_OK, "OK", Kind::Primary, button::DFT, false, 76.0);
            let body = vec![El::text(t, Font::new(13.0, 400), FG(), lh(13.0, 1.4)).wrapping()];
            return Some(dialog::dialog(cx, K_DLG, 340.0, "Notifications for OBS", body, vec![ok], true, at));
        }
        if let Some(at) = self.connect {
            let no = button::cbtn_sized(cx, K_CON_NO, "Not now", Kind::Ghost, button::DFT, false, 76.0);
            let go = button::cbtn_sized(cx, K_CON_GO, "Connect", Kind::Primary, button::DFT, false, 76.0);
            let body = vec![
                El::text(CONNECT_TEXT, Font::new(13.0, 400), FG(), lh(13.0, 1.4)).wrapping(),
                El::text(CONNECT_NOTE, Font::new(11.5, 400), FG2(), 15.5).wrapping().margin(10.0, 0.0, 0.0, 0.0),
            ];
            return Some(dialog::dialog(cx, K_CON_DLG, 380.0, "Connect to OBS", body, vec![no, go], true, at));
        }
        if let Some(msg) = self.confirm.clone() {
            let b = self.boxes.get(&idx(K_BAR, 1)).copied().unwrap_or((300.0, 460.0, 120.0, 30.0));
            return Some(mitems::confirm(cx, K_CF, "Apply to OBS?", &msg, "Cancel", "OK", Kind::Primary, mitems::Place::Under(b.0, b.1, b.2, b.3), 262.0));
        }
        let m = self.menu.clone()?;
        let items: Vec<Item> = m.items.iter().map(|(l, val, ck)| Item { label: l.clone(), checked: *ck, disabled: *val < 0 && m.of == K_ADD }).collect();
        let (x, y, w, h) = m.anchor;
        let _ = now;
        Some(dropdown::menu(cx, K_MENU, &items, x, y + h + 4.0, w.max(150.0)))
    }

    fn popup_dismiss(&mut self) {
        self.menu = None;
        if self.confirm.take().is_some() {
            self.undo();
        }
        self.dialog = None;
        if self.connect.take().is_some() {
            crate::obs::send(bu_obs::Input::Connect(false));
        }
    }

    fn overlay(&mut self, cx: &mut Cx) -> Option<El> {
        let v = crate::obs::view()?;
        let on = self.pending(&v);
        Some(selbar::selbar(
            cx,
            K_BAR,
            "OBS settings changed",
            None,
            &[Sbb { icon: "undo", label: "Undo", danger: false }, Sbb { icon: "check", label: "Apply to OBS", danger: false }],
            on,
        ))
    }

    fn bar_shown(&self) -> bool {
        crate::obs::view().is_some_and(|v| self.pending(&v))
    }

    fn describe(&self) -> String {
        let v = crate::obs::view().unwrap_or_default();
        let s = crate::obs::settings().unwrap_or_default();
        format!(
            "on={} connected={} replay={} rec={} style={} where={} scenes={} pending={} reqs={:?} popups={:?} status={:?}",
            crate::obs::running(),
            v.connected,
            v.replay,
            v.recording,
            st::style_name(s.style),
            s.where_,
            s.scenes.len(),
            self.pending(&v),
            self.reqs,
            crate::obs::popups::shown(),
            crate::obs::status::note()
        )
    }
}

/// Every item of these lists is a value (the "(no other scenes)" line of Add scene is not).
fn m_is_value_list(m: Option<&Menu>) -> bool {
    m.is_some_and(|m| m.of != K_ADD)
}
