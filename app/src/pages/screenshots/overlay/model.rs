//! The capture overlay's state and rules (menu-v22 "CAPTURE OVERLAY": co, coStart, the pointer / key handlers, coFinish,
//! coWhole, coSetLive, coSnapNow, coReset, szEdit, the emoji picker), with no window and no Windows calls - every rule is
//! tested here. Coordinates are DESKTOP PIXELS (physical, the same space as bu-screenshot's `Rect` / `Monitor`); sizes the
//! drawing gives in CSS px (pen 4 px, text 20 px...) are multiplied by the scale of the monitor they are drawn on.
//! The window (window.rs) turns mouse / keys into these calls and does what the returned `Out` asks.

use bu_screenshot::{geom, Monitor, Rect};

use super::emoji::{self, Recent};

/// The five pen colours (PENC), `#ff453a` first (the default).
pub const COLORS: [u32; 5] = [0xff453a, 0xffd60a, 0x30d158, 0x0a84ff, 0xffffff];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// nothing picked yet: the hint, the top bar, the full crosshair
    Idle,
    /// a box is being dragged
    Draw,
    /// a box exists: handles, toolbar, size tag
    Edit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Arrow,
    Box,
    Text,
    Hl,
    Emoji,
}

impl Tool {
    pub const ALL: [Tool; 6] = [Tool::Pen, Tool::Arrow, Tool::Box, Tool::Text, Tool::Hl, Tool::Emoji];
    /// the drawing's button names (tips) and ICON names
    pub fn name(self) -> &'static str {
        ["Pen", "Arrow", "Box", "Text", "Highlighter", "Emoji"][self as usize]
    }
    pub fn icon(self) -> &'static str {
        ["pen", "arrow", "box", "txt", "hlt", "emo"][self as usize]
    }
    fn key(c: char) -> Option<Tool> {
        Some(match c {
            'p' => Tool::Pen,
            'a' => Tool::Arrow,
            'b' => Tool::Box,
            't' => Tool::Text,
            'h' => Tool::Hl,
            'e' => Tool::Emoji,
            _ => return None,
        })
    }
}

/// A point in desktop pixels.
pub type Pt = (f32, f32);

/// One mark on the picture (drawAnn). `s` = the scale of the monitor it was drawn on (CSS px -> desktop px).
#[derive(Clone, Debug, PartialEq)]
pub enum Ann {
    Pen { c: u32, pts: Vec<Pt>, s: f32 },
    Hl { c: u32, pts: Vec<Pt>, s: f32 },
    Arrow { c: u32, a: Pt, b: Pt, s: f32 },
    Box { c: u32, a: Pt, b: Pt, s: f32 },
    /// the text's top-left (the drawing's `x: at.x + 5, y: at.y + 4`)
    Text { c: u32, text: String, x: f32, y: f32, s: f32 },
    /// a 38 px emoji centred at (x, y); `born` = when it was stamped (the 260 ms pop-in)
    Emo { e: String, x: f32, y: f32, size: f32, born: f64, s: f32 },
}

/// The selection (whole pixels). w / h may be 0 while dragging.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Sel {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Sel {
    pub fn rect(&self) -> Option<Rect> {
        (self.w > 0 && self.h > 0).then(|| Rect::new(self.x, self.y, self.w as u32, self.h as u32))
    }
    fn of(r: &Rect) -> Sel {
        Sel { x: r.x, y: r.y, w: r.w as i32, h: r.h as i32 }
    }
    pub fn contains(&self, p: Pt) -> bool {
        p.0 >= self.x as f32 && p.0 <= (self.x + self.w) as f32 && p.1 >= self.y as f32 && p.1 <= (self.y + self.h) as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    Tl,
    Tr,
    Bl,
    Br,
}

#[derive(Clone, Debug, PartialEq)]
enum Drag {
    New { p0: (i32, i32) },
    Move { s0: Sel, p0: (i32, i32), moved: bool },
    Resize { c: Corner, s0: Sel },
    Ann,
    Emo,
}

/// The text being typed (`.ctxt`): its top-left (desktop px), colour, the text, the scale where it sits.
#[derive(Clone, Debug, PartialEq)]
pub struct TextField {
    pub x: f32,
    pub y: f32,
    pub c: u32,
    pub text: String,
    pub s: f32,
}

/// What the pointer looks like (cursorSync's data-m).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pointer {
    /// the crosshair with the faint full-screen guides
    Full,
    /// the crosshair alone (a drawing tool other than pen / highlighter / emoji / text)
    Cross,
    /// the pen / highlighter ring
    Ring,
    /// the emoji under the pointer (faint)
    Emo,
    /// the I-beam (text tool)
    Text,
    /// the move arrows (inside the box, no tool)
    Move,
    /// the normal arrow (over the bars, after drawing)
    Sys,
}

/// How the overlay finishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Finish {
    Copy,
    Save,
}

/// What one capture hands the next one in this run (the drawing keeps them in `co` and `eRecent`): the pen colour, the
/// stamp emoji and the quick row.
#[derive(Clone, Debug)]
pub struct Keep {
    pub color: u32,
    pub emoji: String,
    pub recent: Recent,
}

impl Default for Keep {
    fn default() -> Self {
        Keep { color: COLORS[0], emoji: emoji::QUICK_FIRST.to_string(), recent: Recent::default() }
    }
}

/// What the window must do after an input.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Nothing,
    /// close the overlay (Esc, ×, right-click with nothing picked)
    Close,
    /// Copy / Save the box with its marks, then close
    Finish(Finish),
    /// Live switched on (the real screen shows through) / off (this moment is kept: capture now)
    Live(bool),
    /// Live: keep this moment (Snap / Space) - capture now and flash the box
    Snap,
}

/// A white flash over a part of the desktop (cfl / #flash): where, when, peak opacity, length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flash {
    pub r: Sel,
    pub at: f64,
    pub peak: f32,
    pub ms: f64,
}

/// The emoji picker (`.cemj`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Picker {
    pub open: bool,
    /// "More": search + tabs + every section
    pub big: bool,
    pub search: String,
    /// the list's scroll (CSS px) and the lit tab
    pub scroll: f32,
    pub tab: usize,
    pub opened_at: f64,
}

pub struct Model {
    pub monitors: Vec<Monitor>,
    pub desk: Rect,
    /// the monitor the bars live on (the one under the pointer when the overlay opened)
    pub home: usize,
    pub mode: Mode,
    pub sel: Option<Sel>,
    pub anns: Vec<Ann>,
    pub tool: Option<Tool>,
    pub color: u32,
    pub emoji: String,
    pub recent: Recent,
    drag: Option<Drag>,
    pub live: bool,
    pub text: Option<TextField>,
    /// the size tag being typed ("1920 × 1080")
    pub size_edit: Option<String>,
    /// the size field's text is still selected (the field opens with it selected: the first key replaces it)
    pub size_selected: bool,
    pub picker: Picker,
    /// the pointer (desktop px) and whether it is over a bar / the toolbar / the picker / ×
    pub last: Pt,
    pub over_chrome: bool,
    pub flash: Option<Flash>,
    /// when the overlay opened / the toolbar showed / the box was reset (their fade-ins)
    pub opened_at: f64,
    pub tb_at: f64,
    pub reset_at: f64,
    /// the toolbar sits below the box (else above / inside); set by the view after it placed the toolbar
    pub tb_below: bool,
    pub undo_at: f64,
    /// the overlay window has the keyboard (a selected field shows the active selection colour)
    pub focused: bool,
    /// Order 045: the screenshots folder as the page shows it (`S.shotDir`), for Save's hover name; empty = not known
    pub save_dir: String,
}

/// The scale of the monitor under a point (DPI / 96), 1 when off every monitor.
pub fn scale_at(monitors: &[Monitor], p: Pt) -> f32 {
    geom::monitor_at(monitors, p.0.floor() as i32, p.1.floor() as i32).map(|m| m.dpi as f32 / 96.0).unwrap_or(1.0)
}

fn clampf(v: f32, a: f32, b: f32) -> f32 {
    v.max(a).min(b)
}

impl Model {
    /// The overlay opens (coStart): frozen, nothing picked, the pointer where it is. `emoji` / `recent` carry over from the
    /// last capture in this run (`Keep`).
    pub fn new(monitors: Vec<Monitor>, pointer: Pt, now: f64, keep: Option<Keep>) -> Model {
        let desk = geom::desktop_bounds(&monitors);
        let home = monitors
            .iter()
            .position(|m| m.rect.contains(pointer.0.floor() as i32, pointer.1.floor() as i32))
            .or_else(|| monitors.iter().position(|m| m.primary))
            .unwrap_or(0);
        let Keep { color, emoji, recent } = keep.unwrap_or_default();
        Model {
            monitors,
            desk,
            home,
            mode: Mode::Idle,
            sel: None,
            anns: Vec::new(),
            tool: None,
            color,
            emoji,
            recent,
            drag: None,
            live: false,
            text: None,
            size_edit: None,
            size_selected: false,
            picker: Picker::default(),
            last: pointer,
            over_chrome: false,
            flash: None,
            opened_at: now,
            tb_at: f64::NEG_INFINITY,
            reset_at: f64::NEG_INFINITY,
            tb_below: true,
            undo_at: f64::NEG_INFINITY,
            focused: true,
            save_dir: String::new(),
        }
    }

    /// What the next capture starts with.
    pub fn keep(&self) -> Keep {
        Keep { color: self.color, emoji: self.emoji.clone(), recent: self.recent.clone() }
    }

    pub fn home_monitor(&self) -> &Monitor {
        &self.monitors[self.home]
    }
    pub fn home_scale(&self) -> f32 {
        self.home_monitor().dpi as f32 / 96.0
    }
    fn round_in(&self, p: Pt) -> (i32, i32) {
        let d = &self.desk;
        (clampf(p.0, d.x as f32, d.right() as f32).round() as i32, clampf(p.1, d.y as f32, d.bottom() as f32).round() as i32)
    }
    fn in_sel(&self, p: Pt) -> bool {
        self.sel.is_some_and(|s| s.contains(p))
    }
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }
    /// The monitor buttons' boxes: each monitor by number, then All (when there are several).
    pub fn presets(&self) -> Vec<(String, Sel)> {
        let mut v: Vec<(String, Sel)> = self.monitors.iter().map(|m| (m.number.to_string(), Sel::of(&m.rect))).collect();
        if self.monitors.len() > 1 {
            v.push(("All".into(), Sel::of(&self.desk)));
        }
        v
    }
    /// Which preset is lit (`.cmb.on`): the box equals it exactly (only while editing).
    pub fn lit(&self) -> Option<usize> {
        let s = self.sel?;
        if self.mode != Mode::Edit {
            return None;
        }
        self.presets().iter().position(|(_, r)| *r == s)
    }

    // ------------------------------------------------------------------ the pointer
    /// What the pointer shows (cursorSync).
    pub fn pointer(&self) -> Pointer {
        if self.over_chrome && self.drag.is_none() {
            return Pointer::Sys;
        }
        if self.mode != Mode::Edit {
            return Pointer::Full;
        }
        if matches!(self.drag, Some(Drag::Move { .. }) | Some(Drag::Resize { .. })) {
            return Pointer::Sys;
        }
        if self.in_sel(self.last) {
            return match self.tool {
                Some(Tool::Pen) | Some(Tool::Hl) => Pointer::Ring,
                Some(Tool::Emoji) => Pointer::Emo,
                Some(Tool::Text) => Pointer::Text,
                Some(_) => Pointer::Cross,
                None => Pointer::Move,
            };
        }
        if self.anns.is_empty() {
            Pointer::Full
        } else {
            Pointer::Sys
        }
    }

    /// The left button went down on the picture (not on a bar, the toolbar, the picker or ×). `handle` = a corner dot
    /// under the pointer.
    pub fn press(&mut self, p: Pt, handle: Option<Corner>, now: f64) -> Out {
        self.last = p;
        if self.size_edit.is_some() {
            // a click away from the size field just applies it
            self.size_commit(true);
            return Out::Nothing;
        }
        let pr = self.round_in(p);
        if self.text.is_some() {
            self.commit_text();
            if self.tool == Some(Tool::Text) {
                return Out::Nothing;
            }
        }
        if self.picker.open {
            self.close_picker();
        }
        if self.mode == Mode::Edit {
            if let Some(c) = handle {
                self.drag = Some(Drag::Resize { c, s0: self.sel.unwrap_or_default() });
                return Out::Nothing;
            }
            if self.in_sel(p) {
                let s = scale_at(&self.monitors, p);
                match self.tool {
                    Some(Tool::Text) => {
                        self.text = Some(TextField { x: p.0.round(), y: (p.1 - 15.0 * s).round(), c: self.color, text: String::new(), s });
                    }
                    Some(Tool::Emoji) => {
                        self.anns.push(Ann::Emo { e: self.emoji.clone(), x: p.0, y: p.1, size: 38.0, born: now, s });
                        self.drag = Some(Drag::Emo);
                    }
                    Some(t) => {
                        let c = self.color;
                        self.anns.push(match t {
                            Tool::Pen => Ann::Pen { c, pts: vec![p], s },
                            Tool::Hl => Ann::Hl { c, pts: vec![p], s },
                            Tool::Arrow => Ann::Arrow { c, a: p, b: p, s },
                            _ => Ann::Box { c, a: p, b: p, s },
                        });
                        self.drag = Some(Drag::Ann);
                    }
                    None => self.drag = Some(Drag::Move { s0: self.sel.unwrap_or_default(), p0: pr, moved: false }),
                }
                return Out::Nothing;
            }
            if !self.anns.is_empty() {
                // once something is drawn, a stray click outside never throws it away
                return Out::Nothing;
            }
        }
        self.mode = Mode::Draw;
        self.anns.clear();
        self.tool = None;
        self.drag = Some(Drag::New { p0: pr });
        self.sel = Some(Sel { x: pr.0, y: pr.1, w: 0, h: 0 });
        Out::Nothing
    }

    /// The pointer moved (with or without the button held).
    pub fn moved(&mut self, p: Pt, over_chrome: bool) {
        self.last = p;
        self.over_chrome = over_chrome;
        let pr = self.round_in(p);
        let desk = self.desk;
        match self.drag.clone() {
            None => {}
            Some(Drag::New { p0 }) => {
                self.sel = Some(Sel { x: p0.0.min(pr.0), y: p0.1.min(pr.1), w: (pr.0 - p0.0).abs(), h: (pr.1 - p0.1).abs() });
            }
            Some(Drag::Move { s0, p0, moved }) => {
                if !moved && (pr.0 - p0.0).abs() + (pr.1 - p0.1).abs() <= 2 {
                    return;
                }
                self.drag = Some(Drag::Move { s0, p0, moved: true });
                let x = (s0.x + pr.0 - p0.0).clamp(desk.x, (desk.right() as i32 - s0.w).max(desk.x));
                let y = (s0.y + pr.1 - p0.1).clamp(desk.y, (desk.bottom() as i32 - s0.h).max(desk.y));
                self.sel = Some(Sel { x, y, ..s0 });
            }
            Some(Drag::Resize { c, s0 }) => {
                let (mut x1, mut y1, mut x2, mut y2) = (s0.x, s0.y, s0.x + s0.w, s0.y + s0.h);
                match c {
                    Corner::Tl | Corner::Bl => x1 = pr.0,
                    _ => x2 = pr.0,
                }
                match c {
                    Corner::Tl | Corner::Tr => y1 = pr.1,
                    _ => y2 = pr.1,
                }
                self.sel = Some(Sel { x: x1.min(x2), y: y1.min(y2), w: (x2 - x1).abs(), h: (y2 - y1).abs() });
            }
            Some(Drag::Ann) => match self.anns.last_mut() {
                Some(Ann::Pen { pts, .. }) | Some(Ann::Hl { pts, .. }) => {
                    let q = *pts.last().unwrap_or(&p);
                    if (p.0 - q.0).hypot(p.1 - q.1) >= 1.5 {
                        pts.push(p);
                    }
                }
                Some(Ann::Arrow { b, .. }) | Some(Ann::Box { b, .. }) => *b = p,
                _ => {}
            },
            Some(Drag::Emo) => {
                if let Some(Ann::Emo { x, y, .. }) = self.anns.last_mut() {
                    *x = p.0;
                    *y = p.1;
                }
            }
        }
    }

    /// The left button came up.
    pub fn release(&mut self, now: f64) -> Out {
        let Some(d) = self.drag.take() else { return Out::Nothing };
        match d {
            Drag::New { p0 } => {
                let s = self.sel.unwrap_or_default();
                if s.w < geom::MIN_SIDE as i32 && s.h < geom::MIN_SIDE as i32 {
                    // a click (< 4 px): the whole monitor under the pointer becomes the box - it is NOT taken; only
                    // Copy / Save (or their keys) finish a shot (the owner Oct 8 test 2: "left clicking anywhere ... takes
                    // the screenshot")
                    let r = geom::monitor_at(&self.monitors, p0.0, p0.1).map(|m| m.rect).unwrap_or(self.home_monitor().rect);
                    self.sel = Some(Sel::of(&r));
                    self.mode = Mode::Edit;
                    self.tb_at = now;
                    return Out::Nothing;
                }
                if s.w < geom::MIN_SIDE as i32 || s.h < geom::MIN_SIDE as i32 {
                    // DESIGN §3.3: a box under 4 px reverts (bu-screenshot refuses a side under 4 px)
                    self.sel = None;
                    self.mode = Mode::Idle;
                    self.reset_at = now;
                    return Out::Nothing;
                }
                self.mode = Mode::Edit;
                self.tb_at = now;
            }
            Drag::Ann => {
                let tiny = match self.anns.last() {
                    Some(Ann::Arrow { a, b, .. }) | Some(Ann::Box { a, b, .. }) => (b.0 - a.0).hypot(b.1 - a.1) < 4.0,
                    _ => false,
                };
                if tiny {
                    self.anns.pop();
                }
            }
            // a click on the box (no move) does nothing: clicks select and draw, they never capture (the owner Oct 8) -
            // Live's moment is kept with Snap / Space
            Drag::Move { moved: false, .. } => {}
            Drag::Move { s0, .. } | Drag::Resize { s0, .. } => {
                let s = self.sel.unwrap_or_default();
                if s.w < geom::MIN_SIDE as i32 || s.h < geom::MIN_SIDE as i32 {
                    self.sel = Some(s0);
                }
            }
            Drag::Emo => {}
        }
        Out::Nothing
    }

    /// The right button went down. `on_bars` = over the toolbar or the picker (nothing happens there).
    pub fn right_press(&mut self, on_bars: bool, now: f64) -> Out {
        if on_bars {
            return Out::Nothing;
        }
        if self.sel.is_some() || self.mode != Mode::Idle || self.text.is_some() {
            self.reset(now);
            Out::Nothing
        } else {
            Out::Close
        }
    }

    /// coReset: the box and its marks go; the overlay stays open.
    pub fn reset(&mut self, now: f64) {
        self.text = None;
        self.size_edit = None;
        self.mode = Mode::Idle;
        self.sel = None;
        self.anns.clear();
        self.tool = None;
        self.drag = None;
        self.close_picker();
        self.reset_at = now;
    }

    // ------------------------------------------------------------------ the bars
    /// A monitor button (index into `presets()`): that whole monitor / all of them is the box.
    pub fn pick_preset(&mut self, i: usize, now: f64) {
        self.commit_text();
        if self.size_edit.is_some() {
            self.size_commit(true);
        }
        let Some((_, s)) = self.presets().get(i).cloned() else { return };
        self.sel = Some(s);
        self.mode = Mode::Edit;
        self.drag = None;
        if self.tb_at < self.opened_at || self.tb_at < self.reset_at {
            self.tb_at = now;
        }
    }

    /// A toolbar tool (pickTool): the same tool again turns it off; Emoji opens / closes the picker.
    pub fn pick_tool(&mut self, t: Tool, now: f64) {
        self.commit_text();
        if t == Tool::Emoji {
            if self.tool == Some(Tool::Emoji) && self.picker.open {
                self.close_picker();
            } else {
                self.tool = Some(Tool::Emoji);
                self.picker = Picker { open: true, opened_at: now, ..Picker::default() };
            }
        } else {
            self.tool = if self.tool == Some(t) { None } else { Some(t) };
            self.close_picker();
        }
    }

    pub fn pick_color(&mut self, c: u32) {
        self.color = c;
    }

    /// Undo (the button or Ctrl+Z): the last mark goes. No redo.
    pub fn undo(&mut self, from_key: bool, now: f64) {
        self.commit_text();
        if self.anns.pop().is_some() && from_key {
            self.undo_at = now;
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.anns.is_empty()
    }

    /// Copy / Save (coFinish): only with a box; the typed text is kept first. The flash over the box starts here.
    pub fn finish(&mut self, k: Finish, now: f64) -> Out {
        let Some(s) = self.sel else { return Out::Nothing };
        if s.rect().is_none() {
            return Out::Nothing;
        }
        self.commit_text();
        self.flash = Some(Flash { r: s, at: now, peak: 0.42, ms: 260.0 });
        Out::Finish(k)
    }

    /// The Live switch.
    pub fn set_live(&mut self, on: bool) -> Out {
        if self.live == on {
            return Out::Nothing;
        }
        self.live = on;
        Out::Live(on)
    }

    /// Snap (the button, Space): this moment is kept; a short flash over the box (or the
    /// home screen when there is none).
    pub fn snap(&mut self, now: f64) -> Out {
        if !self.live {
            return Out::Nothing;
        }
        self.live = false;
        let r = match (self.sel, self.mode) {
            (Some(s), Mode::Edit) => s,
            _ => Sel::of(&self.home_monitor().rect),
        };
        self.flash = Some(Flash { r, at: now, peak: 0.32, ms: 260.0 });
        Out::Snap
    }

    // ------------------------------------------------------------------ the size tag
    /// A click on the size tag (only while editing): it becomes a field holding "W × H".
    pub fn size_start(&mut self) {
        if self.size_edit.is_some() || self.mode != Mode::Edit {
            return;
        }
        let Some(s) = self.sel else { return };
        self.commit_text();
        self.size_edit = Some(format!("{} × {}", s.w, s.h));
        self.size_selected = true;
    }

    /// Enter / a click away (true) applies the first two numbers typed; Esc (false) cancels.
    pub fn size_commit(&mut self, apply: bool) {
        let Some(t) = self.size_edit.take() else { return };
        if !apply {
            return;
        }
        let (Some((w, h)), Some(s)) = (geom::parse_size(&t), self.sel) else { return };
        let cur = Rect::new(s.x, s.y, s.w.max(1) as u32, s.h.max(1) as u32);
        self.sel = Some(Sel::of(&geom::fit_typed_size(&cur, w, h, &self.desk)));
    }

    // ------------------------------------------------------------------ text
    /// commitText: the typed text becomes a mark (empty = nothing).
    pub fn commit_text(&mut self) {
        let Some(t) = self.text.take() else { return };
        let s = t.text.trim();
        if !s.is_empty() {
            self.anns.push(Ann::Text { c: t.c, text: s.to_string(), x: t.x + 5.0 * t.s, y: t.y + 4.0 * t.s, s: t.s });
        }
    }

    // ------------------------------------------------------------------ the emoji picker
    pub fn close_picker(&mut self) {
        self.picker.open = false;
        self.picker.big = false;
        self.picker.search.clear();
    }

    /// More / Back.
    pub fn picker_big(&mut self, on: bool, now: f64) {
        self.picker.big = on;
        self.picker.opened_at = now;
        if on {
            self.picker.search.clear();
            self.picker.scroll = 0.0;
            self.picker.tab = 0;
        }
    }

    /// An emoji picked (from the quick row, or from More: then it joins the quick row's front).
    pub fn pick_emoji(&mut self, e: &str, from_more: bool) {
        if from_more {
            self.recent.picked_from_more(e);
        }
        self.emoji = e.to_string();
        self.tool = Some(Tool::Emoji);
        self.close_picker();
    }

    /// The search results (empty = no search: the sections show).
    pub fn search_hits(&self) -> Vec<&'static str> {
        emoji::search(&self.picker.search)
    }

    // ------------------------------------------------------------------ keys
    /// A key went down (coKey + the fields' own keydown). `vk` = virtual key, `ch` = the character it types on this layout
    /// (lower case), `ctrl` = Ctrl held, `z_pos` = it is the key in the US "Z" position (Ctrl+Z by key position on other
    /// layouts), `repeat` = auto-repeat.
    pub fn key(&mut self, vk: u16, ch: Option<char>, ctrl: bool, z_pos: bool, repeat: bool, now: f64) -> Out {
        const ESC: u16 = 0x1B;
        const ENTER: u16 = 0x0D;
        const BACK: u16 = 0x08;
        const SPACE: u16 = 0x20;
        // a field has the keys first (they never reach the drawing keys)
        if self.size_edit.is_some() {
            match vk {
                ENTER => self.size_commit(true),
                ESC => self.size_commit(false),
                BACK => {
                    if let Some(t) = &mut self.size_edit {
                        if self.size_selected {
                            t.clear();
                        } else {
                            t.pop();
                        }
                    }
                    self.size_selected = false;
                }
                _ => {}
            }
            return Out::Nothing;
        }
        if self.text.is_some() {
            match vk {
                ENTER => self.commit_text(),
                ESC => self.text = None,
                BACK => {
                    if let Some(t) = &mut self.text {
                        t.text.pop();
                    }
                }
                _ => {}
            }
            return Out::Nothing;
        }
        if self.picker.open && self.picker.big {
            match vk {
                ESC => {
                    if self.picker.search.is_empty() {
                        self.close_picker();
                    } else {
                        self.picker.search.clear();
                    }
                }
                ENTER => {
                    if let Some(e) = self.search_hits().first() {
                        self.pick_emoji(e, true);
                    }
                }
                BACK => {
                    self.picker.search.pop();
                }
                _ => {}
            }
            return Out::Nothing;
        }
        if repeat {
            return Out::Nothing;
        }
        if vk == ESC {
            if self.picker.open {
                self.close_picker();
                return Out::Nothing;
            }
            return Out::Close;
        }
        if vk == SPACE {
            return self.snap(now);
        }
        if self.mode != Mode::Edit {
            return Out::Nothing;
        }
        let latin = ch.is_some_and(|c| c.is_ascii_lowercase());
        if ctrl && (ch == Some('z') || (z_pos && !latin)) {
            self.undo(true, now);
            return Out::Nothing;
        }
        if ctrl && ch == Some('c') {
            return self.finish(Finish::Copy, now);
        }
        if ctrl && ch == Some('s') {
            return self.finish(Finish::Save, now);
        }
        if !ctrl {
            if let Some(t) = ch.and_then(Tool::key) {
                self.pick_tool(t, now);
            }
        }
        Out::Nothing
    }

    /// A typed character (WM_CHAR) - only for the fields.
    pub fn char_input(&mut self, c: char) {
        if c.is_control() {
            return;
        }
        if let Some(t) = &mut self.size_edit {
            if self.size_selected {
                t.clear();
                self.size_selected = false;
            }
            t.push(c);
        } else if let Some(t) = &mut self.text {
            t.text.push(c);
        } else if self.picker.open && self.picker.big {
            self.picker.search.push(c);
            self.picker.scroll = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bu_screenshot::fake::mon;

    fn two() -> Vec<Monitor> {
        let mut v = vec![mon(0, 0, 1920, 1080, true, false), mon(1920, 0, 2560, 1440, false, false)];
        geom::number_monitors(&mut v);
        v
    }
    fn drag(m: &mut Model, pts: &[Pt]) -> Out {
        m.press(pts[0], None, 0.0);
        for p in &pts[1..] {
            m.moved(*p, false);
        }
        m.release(0.0)
    }
    fn boxed() -> Model {
        let mut m = Model::new(two(), (560.0, 300.0), 0.0, None);
        drag(&mut m, &[(560.0, 300.0), (900.0, 560.0), (1180.0, 700.0)]);
        m
    }

    #[test]
    fn opens_frozen_on_the_monitor_under_the_pointer() {
        let m = Model::new(two(), (2500.0, 100.0), 0.0, None);
        assert_eq!((m.home, m.mode, m.live, m.sel), (1, Mode::Idle, false, None));
        assert_eq!(m.pointer(), Pointer::Full);
        assert_eq!(m.emoji, "😂");
        assert_eq!(m.presets().iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), ["1", "2", "All"]);
    }

    #[test]
    fn a_drag_makes_a_whole_pixel_box_and_the_toolbar() {
        let m = boxed();
        assert_eq!(m.sel, Some(Sel { x: 560, y: 300, w: 620, h: 400 }));
        assert_eq!(m.mode, Mode::Edit);
        assert_eq!(m.pointer(), Pointer::Move, "inside the box with no tool");
        let mut m2 = Model::new(two(), (0.0, 0.0), 0.0, None);
        drag(&mut m2, &[(10.4, 10.6), (-50.0, 300.2)]);
        assert_eq!(m2.sel, Some(Sel { x: 0, y: 11, w: 10, h: 289 }), "rounded and clamped to the desktop");
    }

    #[test]
    fn a_click_is_the_whole_monitor_under_the_pointer() {
        let mut m = Model::new(two(), (0.0, 0.0), 0.0, None);
        // it becomes the box (the toolbar comes); nothing is taken - only Copy / Save finish
        assert_eq!(drag(&mut m, &[(2000.0, 50.0), (2002.0, 51.0)]), Out::Nothing);
        assert_eq!((m.sel, m.mode), (Some(Sel::of(&m.monitors[1].rect)), Mode::Edit));
        assert_eq!(m.finish(Finish::Copy, 1.0), Out::Finish(Finish::Copy));
    }

    #[test]
    fn a_thin_box_reverts() {
        let mut m = Model::new(two(), (0.0, 0.0), 0.0, None);
        assert_eq!(drag(&mut m, &[(100.0, 100.0), (300.0, 102.0)]), Out::Nothing);
        assert_eq!((m.sel, m.mode), (None, Mode::Idle));
    }

    #[test]
    fn move_resize_and_their_limits() {
        let mut m = boxed();
        drag(&mut m, &[(600.0, 400.0), (700.0, 450.0)]);
        assert_eq!(m.sel, Some(Sel { x: 660, y: 350, w: 620, h: 400 }));
        // moving stops at the desktop's edge
        drag(&mut m, &[(700.0, 400.0), (-900.0, -900.0)]);
        assert_eq!(m.sel.map(|s| (s.x, s.y)), Some((0, 0)));
        // a corner
        m.press((620.0, 400.0), Some(Corner::Br), 0.0);
        m.moved((800.0, 500.0), false);
        m.release(0.0);
        assert_eq!(m.sel, Some(Sel { x: 0, y: 0, w: 800, h: 500 }));
        // a resize under 4 px goes back
        m.press((0.0, 0.0), Some(Corner::Br), 0.0);
        m.moved((2.0, 300.0), false);
        m.release(0.0);
        assert_eq!(m.sel, Some(Sel { x: 0, y: 0, w: 800, h: 500 }));
    }

    #[test]
    fn marks_are_made_and_undone() {
        let mut m = boxed();
        m.pick_tool(Tool::Pen, 0.0);
        assert_eq!(m.pointer(), Pointer::Ring, "the pen ring inside the box");
        drag(&mut m, &[(620.0, 360.0), (640.0, 350.0), (640.5, 350.5), (665.0, 348.0)]);
        assert!(matches!(&m.anns[0], Ann::Pen { pts, .. } if pts.len() == 3), "points closer than 1.5 px are skipped");
        m.pick_tool(Tool::Arrow, 0.0);
        drag(&mut m, &[(760.0, 620.0), (761.0, 621.0)]);
        assert_eq!(m.anns.len(), 1, "an arrow under 4 px is dropped");
        drag(&mut m, &[(760.0, 620.0), (880.0, 520.0)]);
        assert_eq!(m.anns.len(), 2);
        m.pick_tool(Tool::Arrow, 0.0);
        assert_eq!(m.tool, None, "the active tool again turns it off");
        m.undo(true, 5.0);
        assert_eq!((m.anns.len(), m.undo_at), (1, 5.0));
        m.undo(false, 6.0);
        m.undo(false, 7.0);
        assert!(!m.can_undo());
    }

    #[test]
    fn marks_outside_are_never_thrown_away_by_a_stray_click() {
        let mut m = boxed();
        m.pick_tool(Tool::Box, 0.0);
        drag(&mut m, &[(600.0, 400.0), (700.0, 500.0)]);
        m.pick_tool(Tool::Box, 0.0);
        drag(&mut m, &[(100.0, 100.0), (200.0, 200.0)]);
        assert_eq!((m.anns.len(), m.sel.map(|s| s.x)), (1, Some(560)));
    }

    #[test]
    fn text_is_typed_committed_or_dropped() {
        let mut m = boxed();
        m.pick_tool(Tool::Text, 0.0);
        m.press((620.0, 470.0), None, 0.0);
        for c in "Nicex".chars() {
            m.char_input(c);
        }
        m.key(0x08, None, false, false, false, 0.0);
        m.key(0x0D, None, false, false, false, 0.0);
        assert_eq!(m.anns, vec![Ann::Text { c: COLORS[0], text: "Nice".into(), x: 625.0, y: 459.0, s: 1.0 }]);
        m.press((620.0, 500.0), None, 0.0);
        m.char_input('x');
        m.key(0x1B, None, false, false, false, 0.0);
        assert_eq!(m.anns.len(), 1, "Esc drops the field");
        assert_eq!(m.text, None);
    }

    #[test]
    fn emoji_picker_quick_row_more_search() {
        let mut m = boxed();
        m.pick_tool(Tool::Emoji, 0.0);
        assert!(m.picker.open && !m.picker.big);
        m.picker_big(true, 0.0);
        for c in "pizz".chars() {
            m.char_input(c);
        }
        assert_eq!(m.search_hits(), vec!["🍕"]);
        m.key(0x0D, None, false, false, false, 0.0);
        assert_eq!((m.emoji.as_str(), m.recent.0[0].as_str(), m.picker.open), ("🍕", "🍕", false));
        m.press((800.0, 500.0), None, 7.0);
        assert!(matches!(&m.anns[0], Ann::Emo { e, size, born, .. } if e == "🍕" && *size == 38.0 && *born == 7.0));
        m.moved((820.0, 510.0), false);
        m.release(0.0);
        assert!(matches!(&m.anns[0], Ann::Emo { x, y, .. } if (*x, *y) == (820.0, 510.0)), "the stamp follows while held");
        // Esc: search first, then the picker, then the overlay
        m.pick_tool(Tool::Emoji, 0.0);
        m.picker_big(true, 0.0);
        m.char_input('a');
        m.key(0x1B, None, false, false, false, 0.0);
        assert!(m.picker.open && m.picker.search.is_empty());
        m.key(0x1B, None, false, false, false, 0.0);
        assert!(!m.picker.open);
        assert_eq!(m.key(0x1B, None, false, false, false, 0.0), Out::Close);
    }

    #[test]
    fn typed_size_keeps_the_top_left_and_slides_back() {
        let mut m = boxed();
        m.size_start();
        assert_eq!(m.size_edit.as_deref(), Some("620 × 400"));
        for c in "1920x1080".chars() {
            m.char_input(c);
        }
        m.key(0x0D, None, false, false, false, 0.0);
        assert_eq!(m.sel, Some(Sel { x: 560, y: 300, w: 1920, h: 1080 }));
        m.size_start();
        m.size_edit = Some("9999 x 2".into());
        m.size_commit(true);
        assert_eq!(m.sel, Some(Sel { x: 0, y: 300, w: 4480, h: 4 }), "clamped 4 px .. the desktop, slid back in");
        m.size_start();
        m.size_edit = Some("10 10".into());
        m.key(0x1B, None, false, false, false, 0.0);
        assert_eq!(m.sel.map(|s| s.w), Some(4480), "Esc cancels");
    }

    #[test]
    fn presets_light_only_when_equal() {
        let mut m = Model::new(two(), (0.0, 0.0), 0.0, None);
        m.pick_preset(1, 3.0);
        assert_eq!((m.sel, m.lit(), m.mode), (Some(Sel { x: 1920, y: 0, w: 2560, h: 1440 }), Some(1), Mode::Edit));
        m.pick_preset(2, 3.0);
        assert_eq!((m.sel.map(|s| (s.w, s.h)), m.lit()), (Some((4480, 1440)), Some(2)));
        m.pick_preset(0, 3.0);
        drag(&mut m, &[(100.0, 100.0), (150.0, 150.0)]);
        assert_eq!((m.sel.map(|s| (s.x, s.y)), m.lit()), (Some((50, 50)), None));
    }

    #[test]
    fn live_snap_and_finish() {
        let mut m = Model::new(two(), (0.0, 0.0), 0.0, None);
        assert_eq!(m.key(0x20, None, false, false, false, 0.0), Out::Nothing, "Space does nothing while frozen");
        assert_eq!(m.set_live(true), Out::Live(true));
        assert_eq!(m.key(0x20, None, false, false, false, 9.0), Out::Snap);
        assert_eq!(m.flash.map(|f| (f.r, f.peak)), Some((Sel { x: 0, y: 0, w: 1920, h: 1080 }, 0.32)));
        assert!(!m.live);
        let mut m = boxed();
        m.set_live(true);
        assert_eq!(drag(&mut m, &[(700.0, 400.0), (701.0, 400.0)]), Out::Nothing, "a click on the box never captures");
        assert!(m.live, "still Live after the click");
        assert_eq!(m.key(0x43, Some('c'), true, false, false, 1.0), Out::Finish(Finish::Copy));
        assert_eq!(m.key(0x53, Some('s'), true, false, false, 1.0), Out::Finish(Finish::Save));
        let mut idle = Model::new(two(), (0.0, 0.0), 0.0, None);
        assert_eq!(idle.finish(Finish::Copy, 0.0), Out::Nothing, "no box, no finish");
    }

    #[test]
    fn keys_pick_tools_and_ctrl_z_by_position() {
        let mut m = boxed();
        m.key(0x50, Some('p'), false, false, false, 0.0);
        assert_eq!(m.tool, Some(Tool::Pen));
        drag(&mut m, &[(600.0, 400.0), (650.0, 450.0)]);
        // a Cyrillic layout: Ctrl + the key in the Z position undoes
        m.key(0x5A, Some('я'), true, true, false, 0.0);
        assert!(m.anns.is_empty());
        m.key(0x50, Some('p'), false, false, true, 0.0);
        assert_eq!(m.tool, Some(Tool::Pen), "auto-repeat is ignored");
    }

    #[test]
    fn right_click_resets_or_closes() {
        let mut m = boxed();
        assert_eq!(m.right_press(true, 0.0), Out::Nothing, "never on the toolbar");
        assert!(m.sel.is_some());
        assert_eq!(m.right_press(false, 0.0), Out::Nothing);
        assert_eq!((m.sel, m.mode), (None, Mode::Idle));
        assert_eq!(m.right_press(false, 0.0), Out::Close);
    }
}
