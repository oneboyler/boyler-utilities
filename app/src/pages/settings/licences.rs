//! Settings › About › Licences (Order 040): every third-party part the app ships - the crates compiled into the exe, the
//! Rust standard library, Skia and the libraries built into it, the setup's Inno Setup and Everything, the Raw Accel add-on -
//! each opening its licence text. The data is `app/assets/licences.txt` (made by `tools/licences/gen.py`, its format there),
//! built into the exe; it is read only when the view opens and dropped with it (nothing at idle).
//!
//! Two levels inside the Settings tab: the list (a back arrow + "Licences", the parts in groups, rows like All shortcuts')
//! and one part (a back arrow + its name, its version and licence, each licence text in a group box under its name, the
//! part's own copyright lines first). A text is shown as
//! written (monospace, the file's own lines; only a line wider than the box wraps, at a space, keeping its indent) and only
//! the lines in view are painted, so the longest text (ICU, 560 lines) scrolls like any page.

use std::rc::Rc;

use crate::anim::EASE;
use crate::gfx::{Align, Font, Gfx};
use crate::ui::cx::Cx;
use crate::ui::el::{idx, key, lh, Cursor, El, Key};
use crate::ui::pieces::group;
use crate::ui::{cmix, CTL_H, F20, FG, FG2, FG3, HOV, WIN_W};

/// The generated file (tools/licences/gen.py).
pub const SRC: &str = include_str!("../../../assets/licences.txt");

pub const K_BACK: Key = key("set.lic.back");
pub const K_ROW: Key = key("set.lic.row");

/// The licence text's font: the app's monospace (`"Cascadia Mono","Consolas"`), 11 px, line height 1.5.
pub const MONO: Font = Font::new(11.0, 400).ls(0).mono();
const MONO_LH: f32 = 16.5;
/// The page's content width (`.pg` 600 - padding 26 + 26) less the text box's padding (12 + 12).
pub const TEXT_W: f32 = WIN_W - 52.0 - 24.0;

/// One part: where it is listed, what it is, which licence text(s) it uses and its own copyright lines.
#[derive(Debug)]
pub struct Part {
    pub group: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    /// the licence as the part declares it ("MIT OR Apache-2.0")
    pub declared: &'static str,
    /// indices into `Data::texts`
    pub texts: Vec<usize>,
    /// the part's own copyright lines, each with the text (index) it goes with
    pub copyright: Vec<(usize, &'static str)>,
}

/// One licence text (each distinct text once; parts share it).
#[derive(Debug)]
pub struct Text {
    pub id: &'static str,
    pub title: &'static str,
    pub lines: Vec<&'static str>,
}

#[derive(Debug)]
pub struct Data {
    pub parts: Vec<Part>,
    pub texts: Vec<Text>,
}

/// Read the generated file. Every part must name texts that exist; every text must have words in it.
pub fn parse(src: &'static str) -> Result<Data, String> {
    type Ids = Vec<&'static str>;
    let mut parts: Vec<(Part, Ids, Vec<(&'static str, &'static str)>)> = Vec::new();
    let mut texts: Vec<Text> = Vec::new();
    for (n, line) in src.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(l) = line.strip_prefix('|') {
            texts.last_mut().ok_or(format!("line {}: text line before any text", n + 1))?.lines.push(l);
            continue;
        }
        let f: Vec<&'static str> = line.split('\t').collect();
        match f[0] {
            "P" if f.len() == 6 => {
                let part = Part { group: f[1], name: f[2], version: f[3], declared: f[4], texts: Vec::new(), copyright: Vec::new() };
                parts.push((part, f[5].split(',').filter(|s| !s.is_empty()).collect(), Vec::new()));
            }
            "C" if f.len() == 3 => parts.last_mut().ok_or(format!("line {}: copyright before any part", n + 1))?.2.push((f[1], f[2])),
            "T" if f.len() == 3 => texts.push(Text { id: f[1], title: f[2], lines: Vec::new() }),
            _ => return Err(format!("line {}: not understood", n + 1)),
        }
    }
    for t in &mut texts {
        while t.lines.last().is_some_and(|l| l.trim().is_empty()) {
            t.lines.pop();
        }
        if t.lines.iter().all(|l| l.trim().is_empty()) {
            return Err(format!("text {} is empty", t.id));
        }
    }
    let mut out = Vec::new();
    for (mut p, ids, crs) in parts {
        for id in ids {
            let i = texts.iter().position(|t| t.id == id).ok_or(format!("{}: no text {}", p.name, id))?;
            p.texts.push(i);
        }
        for (id, c) in crs {
            let i = p.texts.iter().copied().find(|i| texts[*i].id == id).ok_or(format!("{}: copyright for text {} it does not use", p.name, id))?;
            p.copyright.push((i, c));
        }
        if p.texts.is_empty() {
            return Err(format!("{}: no licence text", p.name));
        }
        out.push(p);
    }
    Ok(Data { parts: out, texts })
}

/// The lines of a text for a box `cols` characters wide: a longer line breaks at its last space that fits (a word longer
/// than the box at the box's edge); the rest keeps the line's indent.
pub fn wrap(lines: &[&str], cols: usize) -> Vec<String> {
    let cols = cols.max(20);
    let mut out = Vec::new();
    for l in lines {
        let chars: Vec<char> = l.chars().collect();
        if chars.len() <= cols {
            out.push(l.to_string());
            continue;
        }
        let indent = chars.iter().take_while(|c| **c == ' ').count().min(cols / 2);
        let mut rest: &[char] = &chars;
        let mut first = true;
        while !rest.is_empty() {
            let lead = if first { 0 } else { indent };
            let room = cols - lead;
            if rest.len() <= room {
                out.push(" ".repeat(lead) + &rest.iter().collect::<String>());
                break;
            }
            // the last space inside the room (not the line's own leading spaces)
            let from = if first { indent + 1 } else { 1 };
            let cut = (from..=room).rev().find(|&i| rest[i] == ' ').unwrap_or(room);
            out.push(" ".repeat(lead) + rest[..cut].iter().collect::<String>().trim_end());
            rest = &rest[cut..];
            while rest.first() == Some(&' ') {
                rest = &rest[1..];
            }
            first = false;
        }
    }
    out
}

/// The view inside the Settings tab while Licences is open.
pub struct View {
    pub data: Data,
    /// the part shown (None = the list)
    pub open: Option<usize>,
    /// the open part's texts, wrapped for the box (made once when it opens)
    wrapped: Option<(usize, Vec<Rc<Vec<String>>>)>,
}

impl View {
    pub fn new() -> Result<View, String> {
        Ok(View { data: parse(SRC)?, open: None, wrapped: None })
    }

    /// Back one level: true = still in Licences (a part went back to the list).
    pub fn back(&mut self) -> bool {
        if self.open.take().is_some() {
            self.wrapped = None;
            return true;
        }
        false
    }

    /// A row of the list was clicked: its part opens.
    pub fn open_row(&mut self, k: Key) -> bool {
        match (0..self.data.parts.len()).find(|i| k == idx(K_ROW, *i)) {
            Some(i) => {
                self.open = Some(i);
                self.wrapped = None;
                true
            }
            None => false,
        }
    }

    pub fn describe(&self) -> String {
        match self.open {
            Some(i) => format!("lic={}", self.data.parts[i].name),
            None => format!("lic=list:{}", self.data.parts.len()),
        }
    }

    pub fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        match self.open {
            Some(i) => self.part(cx, i),
            None => self.list(cx),
        }
    }

    /// The list: the groups of parts, each row `name` / `version · licence` with the row chevron.
    fn list(&self, cx: &mut Cx) -> Vec<El> {
        let mut out = vec![head(cx, "Licences")];
        let n = self.data.parts.len();
        out.push(sub_line(format!("Boyler Utilities is built with these {n} parts from other people. Each one opens its licence.")));
        let mut group_of: Option<&str> = None;
        let mut rows: Vec<El> = Vec::new();
        for (i, p) in self.data.parts.iter().enumerate() {
            if group_of != Some(p.group) {
                if let Some(g) = group_of {
                    out.push(group::gh(g));
                    out.push(group::grp(std::mem::take(&mut rows)).clip());
                }
                group_of = Some(p.group);
            }
            rows.push(part_row(cx, idx(K_ROW, i), rows.is_empty(), p.name, &format!("{} \u{b7} {}", p.version, p.declared)));
        }
        if let Some(g) = group_of {
            out.push(group::gh(g));
            out.push(group::grp(rows).clip());
        }
        out
    }

    /// One part: its name, version and licence, then each licence text in a group box (its copyright lines first).
    fn part(&mut self, cx: &mut Cx, i: usize) -> Vec<El> {
        if self.wrapped.as_ref().is_none_or(|w| w.0 != i) {
            let cols = cols(cx.g);
            let w = self.data.parts[i].texts.iter().map(|t| Rc::new(wrap(&self.data.texts[*t].lines, cols))).collect();
            self.wrapped = Some((i, w));
        }
        let p = &self.data.parts[i];
        let wrapped = &self.wrapped.as_ref().map(|w| w.1.clone()).unwrap_or_default();
        let mut out = vec![head(cx, p.name)];
        out.push(sub_line(format!("Version {} \u{b7} {}", p.version, p.declared)));
        for (t, lines) in p.texts.iter().zip(wrapped.iter()) {
            out.push(group::gh(self.data.texts[*t].title));
            let mut body = El::col().pad(11.0, 12.0, 12.0, 12.0);
            let own: Vec<&str> = p.copyright.iter().filter(|c| c.0 == *t).map(|c| c.1).collect();
            if !own.is_empty() {
                for c in own {
                    body = body.child(El::text(c, Font::new(12.0, 400), FG(), lh(12.0, 1.4)).wrapping());
                }
                body = body.child(El::block().h(10.0).none());
            }
            body = body.child(mono_block(lines.clone()));
            out.push(group::grp(vec![body]));
        }
        out
    }
}

/// How many monospace characters fit the text box.
pub fn cols(g: &Gfx) -> usize {
    let adv = g.text_width("0000000000", MONO) / 10.0;
    if adv > 0.0 {
        (TEXT_W / adv).floor() as usize
    } else {
        80
    }
}

/// The header: the back arrow (Storage's `.fbk`: 26 x 26, radius 6, `--ctl-h` on hover, chevron 12 px --fg2 -> --fg) and
/// the title as every page's `h2` (`.ph{margin:0 2px 8px;min-height:32px}`).
fn head(cx: &mut Cx, title: &str) -> El {
    let bh = cx.hover_t(K_BACK, 120.0, EASE);
    let back = El::block()
        .size(26.0, 26.0)
        .none()
        .radius(6.0)
        .bg(CTL_H().mul_a(bh))
        .place_center()
        .child(El::icon("chevL", 12.0, 1.6, cmix(FG2(), FG(), bh)).no_hit())
        .on_click(K_BACK)
        .cursor(Cursor::Hand);
    El::row().center().gap(6.0).margin(0.0, 2.0, 8.0, 2.0).min_h(32.0).child(back).child(El::text(title, F20, FG(), 24.0).ellipsis())
}

/// The line under the header (`.abl`'s look: 12 px --fg2, the group headers' 12 px inset).
fn sub_line(text: String) -> El {
    El::text(text, Font::new(12.0, 400), FG2(), lh(12.0, 1.4)).wrapping().margin(0.0, 12.0, 0.0, 12.0)
}

/// A row of the list - All shortcuts' row (`.row.sc`: --hov on hover, the chevron) with the label's small line.
fn part_row(cx: &mut Cx, k: Key, first: bool, name: &str, small: &str) -> El {
    let hv = cx.hover_t(k, 120.0, EASE);
    let chev = El::icon("chevR", 7.0, 1.5, FG3()).h(12.0).margin(0.0, 0.0, 0.0, 6.0).no_hit();
    group::row(first, vec![group::lbl(name, Some(small)), El::row().none().center().child(chev)]).bg(HOV().mul_a(hv)).on_click(k).cursor(Cursor::Hand)
}

/// A licence text: one box as tall as its lines, painting only the lines inside the visible area (the page's clip).
fn mono_block(lines: Rc<Vec<String>>) -> El {
    let h = lines.len() as f32 * MONO_LH;
    let color = FG2();
    El::paint(move |g: &Gfx, (x, y, _w, _h)| {
        let (top, bottom) = match g.cv().local_clip_bounds() {
            Some(r) => (r.top, r.bottom),
            None => (f32::MIN, f32::MAX),
        };
        let first = (((top - y) / MONO_LH).floor().max(0.0)) as usize;
        let last = ((((bottom - y) / MONO_LH).ceil()).max(0.0) as usize).min(lines.len());
        for (k, l) in lines.iter().enumerate().take(last).skip(first) {
            if !l.is_empty() {
                g.text(l, MONO, x, y + k as f32 * MONO_LH, MONO_LH, color, Align::Left, 0.0);
            }
        }
    })
    .h(h)
    .no_hit()
}
