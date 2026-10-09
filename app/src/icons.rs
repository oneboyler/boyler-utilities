//! The drawing's line icons, copied verbatim from menu-v17.html (ICON table), and a tiny SVG-element reader
//! (path / rect / circle / g scale) that draws them the way Blink paints inline SVG: paths as Skia paths, `<rect rx>` as
//! Blink's rounded-rect path, `<circle>` as an oval, all stroked with round caps and joins.

use std::cell::RefCell;
use std::collections::HashMap;
use skia_safe as sk;
use windows_numerics::Matrix3x2;

use crate::gfx::{Gfx, Rgba};
use crate::svg;

const MIC: &str = r#"<rect x="7.25" y="2.5" width="5.5" height="9.5" rx="2.75"/><path d="M4.5 9.5a5.5 5.5 0 0 0 11 0M10 15v2.5"/>"#;

fn gear_path() -> String {
    // 8 rounded teeth around a ring, centred on 10,10 (the drawing's gearPath())
    let (n, ri, ro) = (8, 5.7f64, 7.5f64);
    let f = |x: f64| {
        let s = format!("{:.2}", x);
        s
    };
    let mut d = String::new();
    for i in 0..n {
        let a = i as f64 / n as f64 * std::f64::consts::PI * 2.0 - std::f64::consts::PI / 2.0;
        let p: Vec<(f64, f64)> = [(a - 0.27, ri), (a - 0.15, ro), (a + 0.15, ro), (a + 0.27, ri)]
            .iter()
            .map(|q| (10.0 + q.0.cos() * q.1, 10.0 + q.0.sin() * q.1))
            .collect();
        d += &format!(
            "{}{} {}L{} {}L{} {}L{} {}",
            if i > 0 { "L" } else { "M" },
            f(p[0].0),
            f(p[0].1),
            f(p[1].0),
            f(p[1].1),
            f(p[2].0),
            f(p[2].1),
            f(p[3].0),
            f(p[3].1)
        );
        let b = (i + 1) as f64 / n as f64 * std::f64::consts::PI * 2.0 - std::f64::consts::PI / 2.0 - 0.27;
        d += &format!("A{} {} 0 0 1 {} {}", ri, ri, f(10.0 + b.cos() * ri), f(10.0 + b.sin() * ri));
    }
    d + "Z"
}

/// The raw SVG for an icon name (the same names as the drawing's ICON table): menu-v22's table first (Order 014), then
/// the frame's own glyphs and the older names.
pub fn source(name: &str) -> String {
    // Order 025: `El::icon_svg` - the name IS the page's own SVG markup
    if name.starts_with("<svg") {
        return name.to_string();
    }
    if let Some(v) = crate::icons_v22::source(name) {
        return v.to_string();
    }
    let s: &str = match name {
        // the top row's chevrons (menu-v22 chevL / chevR buttons)
        "dchevL" => r#"<svg viewBox="0 0 12 12"><path d="M7.2 2.6L3.8 6l3.4 3.4"/></svg>"#,
        "dchevR" => r#"<svg viewBox="0 0 12 12"><path d="M4.8 2.6L8.2 6 4.8 9.4"/></svg>"#,
        // the tick of the reset review's tick box (.tkb)
        "tkcheck" => r#"<svg viewBox="0 0 10 10"><path d="M2.2 5.3l1.9 1.9 3.8-4.3"/></svg>"#,
        "spk" => r#"<svg viewBox="0 0 20 20"><path d="M3.5 8h2.8L10 5v10l-3.7-3H3.5z"/><path d="M13 7.6a3.4 3.4 0 0 1 0 4.8M15.2 5.4a6.5 6.5 0 0 1 0 9.2"/></svg>"#,
        "mon" => r#"<svg viewBox="0 0 20 20"><rect x="2.5" y="3.5" width="15" height="10.5" rx="2"/><path d="M7.5 17h5M10 14v3"/></svg>"#,
        "cam" => r#"<svg viewBox="0 0 20 20"><path d="M2.75 7.25A1.75 1.75 0 0 1 4.5 5.5h2l1.25-1.9a1 1 0 0 1 .84-.45h2.82a1 1 0 0 1 .84.45l1.25 1.9h2a1.75 1.75 0 0 1 1.75 1.75v7A1.75 1.75 0 0 1 15.5 16h-11a1.75 1.75 0 0 1-1.75-1.75z"/><circle cx="10" cy="10.6" r="3"/></svg>"#,
        "mouse" => r#"<svg viewBox="0 0 20 20"><rect x="5" y="2.5" width="10" height="15" rx="5"/><path d="M10 2.5v4.6M5 8.1h10" stroke-width="1.3" opacity=".55"/></svg>"#,
        "tool" => r#"<svg viewBox="0 0 20 20"><g transform="scale(.83333)" style="stroke-width:1.8"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></g></svg>"#,
        "tgl" => r#"<svg viewBox="0 0 20 20"><rect x="2.5" y="3.25" width="15" height="6" rx="3"/><circle cx="14.5" cy="6.25" r="1.5"/><rect x="2.5" y="10.75" width="15" height="6" rx="3"/><circle cx="5.5" cy="13.75" r="1.5"/></svg>"#,
        "rocket" => r#"<svg viewBox="0 0 20 20"><path d="M10 2.4c2.5 1.7 3.8 4.4 3.8 7.8v3.4H6.2v-3.4c0-3.4 1.3-6.1 3.8-7.8z"/><circle cx="10" cy="8.3" r="1.5"/><path d="M6.2 10.8L3.8 13.1v2.8l2.4-1.5M13.8 10.8l2.4 2.3v2.8l-2.4-1.5M8.6 16.1l1.4 2.1 1.4-2.1"/></svg>"#,
        "gauge" => r#"<svg viewBox="0 0 20 20"><path d="M3.6 15.2a7.5 7.5 0 1 1 12.8 0"/><path d="M10 11.3l3-3"/><circle cx="10" cy="11.3" r="1.25"/><path d="M3.9 11.3h1.2M10 5v1.2M16.1 11.3h-1.2M5.7 7l.85.85M14.3 7l-.85.85"/></svg>"#,
        "bell" => r#"<svg viewBox="0 0 20 20"><path d="M10 3a4.5 4.5 0 0 0-4.5 4.5v3L4 13h12l-1.5-2.5v-3A4.5 4.5 0 0 0 10 3zM8 15.5a2 2 0 0 0 4 0"/></svg>"#,
        "plug" => r#"<svg viewBox="0 0 20 20"><path d="M7.5 3v3.5M12.5 3v3.5M5.5 6.5h9v3a4.5 4.5 0 0 1-9 0zM10 14v3"/></svg>"#,
        // Order 062: the Noise tab's icon - a rough waveform (the drawing has none for this tab)
        "noise" => r#"<svg viewBox="0 0 20 20"><path d="M2.5 10h1.8l1.2-3.2 1.5 6.6 1.6-9.4 1.6 11.8 1.5-8.4 1.4 4.6 1.2-2h3.2"/></svg>"#,
        "chev" => r#"<svg viewBox="0 0 9 14"><path d="M1.5 5L4.5 2l3 3M1.5 9l3 3 3-3"/></svg>"#,
        "wmin" => r#"<svg viewBox="0 0 10 10"><path d="M0 5.5h10"/></svg>"#,
        "wcls" => r#"<svg viewBox="0 0 10 10"><path d="M.6.6l8.8 8.8M9.4.6L.6 9.4"/></svg>"#,
        "spkM" => r#"<svg viewBox="0 0 20 20"><path d="M3.5 8h2.8L10 5v10l-3.7-3H3.5z"/><path class="w" d="M13 7.6a3.4 3.4 0 0 1 0 4.8M15.2 5.4a6.5 6.5 0 0 1 0 9.2"/><path class="x" d="M13.2 8l4 4M17.2 8l-4 4"/></svg>"#,
        "plus12" => r#"<svg viewBox="0 0 12 12"><path d="M6 2v8M2 6h8"/></svg>"#,
        "note" => r#"<svg viewBox="0 0 16 16"><path d="M6 11.8V4.2l6.2-1.6v7.6"/><circle cx="4.4" cy="11.8" r="1.6"/><circle cx="10.6" cy="10.2" r="1.6"/></svg>"#,
        "chat" => r#"<svg viewBox="0 0 16 16"><path d="M4.5 3h7A1.5 1.5 0 0 1 13 4.5v5a1.5 1.5 0 0 1-1.5 1.5H7.2L4.4 13.2V11A1.5 1.5 0 0 1 3 9.5v-5A1.5 1.5 0 0 1 4.5 3z"/></svg>"#,
        "pad" => r#"<svg viewBox="0 0 16 16"><path d="M5 4.8h6a3 3 0 0 1 2.9 3.7l-.7 2.9a1.4 1.4 0 0 1-2.4.6L9.5 10.6h-3L5.2 12a1.4 1.4 0 0 1-2.4-.6l-.7-2.9A3 3 0 0 1 5 4.8z"/><path d="M5.3 7v2.2M4.2 8.1h2.2"/></svg>"#,
        "globe" => r#"<svg viewBox="0 0 16 16"><circle cx="8" cy="8" r="5.5"/><path d="M2.5 8h11M8 2.5c1.5 1.6 2.2 3.4 2.2 5.5S9.5 11.9 8 13.5C6.5 11.9 5.8 10.1 5.8 8S6.5 4.1 8 2.5z"/></svg>"#,
        "abell" => r#"<svg viewBox="0 0 16 16"><path d="M8 2.6a3.4 3.4 0 0 0-3.4 3.4v2.3l-1.2 2.1h9.2l-1.2-2.1V6A3.4 3.4 0 0 0 8 2.6zM6.6 12.4a1.4 1.4 0 0 0 2.8 0"/></svg>"#,
        "hp" => r#"<svg viewBox="0 0 16 16"><path d="M2.8 10.5V8.2a5.2 5.2 0 0 1 10.4 0v2.3"/><rect x="2.3" y="9.3" width="3" height="4.4" rx="1.2"/><rect x="10.7" y="9.3" width="3" height="4.4" rx="1.2"/></svg>"#,
        "appw" => r#"<svg viewBox="0 0 16 16"><rect x="2.5" y="3" width="11" height="10" rx="1.8"/><path d="M2.5 6h11"/></svg>"#,
        "wcam" => r#"<svg viewBox="0 0 16 16"><circle cx="8" cy="7" r="4.6"/><circle cx="8" cy="7" r="1.6"/><path d="M5 13.5h6"/></svg>"#,
        "mic" => return format!(r#"<svg viewBox="0 0 20 20">{}</svg>"#, MIC),
        "gear" => return format!(r#"<svg viewBox="0 0 20 20"><path d="{}"/><circle cx="10" cy="10" r="2.4"/></svg>"#, gear_path()),
        _ => r#"<svg viewBox="0 0 20 20"></svg>"#,
    };
    s.to_string()
}

#[derive(Clone, Debug)]
enum Kind {
    Path(String),
    Rect(f32, f32, f32, f32, f32),
    Circle(f32, f32, f32),
}

#[derive(Clone, Debug)]
struct El {
    kind: Kind,
    sw: Option<f32>,
    opacity: f32,
    class: String,
    /// a filled part (the battery level: `.bat .lv{fill:var(--green)}`)
    filled: bool,
    scale: f32,
}

#[derive(Clone, Debug)]
pub struct IconDef {
    pub vb: f32,
    pub vbh: f32,
    els: Vec<El>,
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let pat = format!(" {}=\"", name);
    let i = tag.find(&pat)? + pat.len();
    let j = tag[i..].find('"')? + i;
    Some(tag[i..j].to_string())
}
fn num(tag: &str, name: &str) -> f32 {
    attr(tag, name).and_then(|v| v.parse().ok()).unwrap_or(0.0)
}

/// Read the SVG subset the drawing's icons use.
pub fn parse_icon(src: &str) -> IconDef {
    let vbn = |i: usize| attr(src, "viewBox").and_then(|v| v.split_whitespace().nth(i).and_then(|x| x.parse().ok())).unwrap_or(20.0);
    let (vb, vbh) = (vbn(2), vbn(3));
    let mut els = Vec::new();
    let (mut scale, mut gsw) = (1.0f32, None::<f32>);
    let mut rest = src;
    while let Some(i) = rest.find('<') {
        let j = match rest[i..].find('>') {
            Some(j) => i + j,
            None => break,
        };
        let tag = &rest[i..=j];
        rest = &rest[j + 1..];
        if tag.starts_with("<g") {
            if let Some(t) = attr(tag, "transform") {
                if let Some(k) = t.strip_prefix("scale(").and_then(|x| x.strip_suffix(')')) {
                    scale = k.parse().unwrap_or(1.0);
                }
            }
            if let Some(st) = attr(tag, "style") {
                if let Some(v) = st.strip_prefix("stroke-width:") {
                    gsw = v.parse().ok();
                }
            }
            continue;
        }
        if tag.starts_with("</g") {
            scale = 1.0;
            gsw = None;
            continue;
        }
        let kind = if tag.starts_with("<path") {
            Kind::Path(attr(tag, "d").unwrap_or_default())
        } else if tag.starts_with("<rect") {
            Kind::Rect(num(tag, "x"), num(tag, "y"), num(tag, "width"), num(tag, "height"), num(tag, "rx"))
        } else if tag.starts_with("<circle") {
            Kind::Circle(num(tag, "cx"), num(tag, "cy"), num(tag, "r"))
        } else {
            continue;
        };
        els.push(El {
            kind,
            sw: attr(tag, "stroke-width").and_then(|v| v.parse().ok()).or(gsw),
            opacity: attr(tag, "opacity").and_then(|v| v.parse().ok()).unwrap_or(1.0),
            class: attr(tag, "class").unwrap_or_default(),
            filled: attr(tag, "class").map(|c| c == "lv").unwrap_or(false),
            scale,
        });
    }
    IconDef { vb, vbh, els }
}

enum Geo {
    Path(sk::Path),
    Oval(sk::Rect),
}

/// Blink's path for an SVG `<rect rx>`: from the top edge's left end, clockwise, quarter conics at the corners.
fn rect_path(x: f32, y: f32, w: f32, h: f32, rx: f32) -> sk::Path {
    let r = rx.min(w / 2.0).min(h / 2.0);
    let k = std::f32::consts::FRAC_1_SQRT_2;
    let mut b = sk::PathBuilder::new();
    b.move_to((x + r, y))
        .line_to((x + w - r, y))
        .conic_to((x + w, y), (x + w, y + r), k)
        .line_to((x + w, y + h - r))
        .conic_to((x + w, y + h), (x + w - r, y + h), k)
        .line_to((x + r, y + h))
        .conic_to((x, y + h), (x, y + h - r), k)
        .line_to((x, y + r))
        .conic_to((x, y), (x + r, y), k)
        .close();
    b.detach()
}

/// Icon geometry, built once per icon name.
pub struct Icons {
    defs: RefCell<HashMap<String, (IconDef, Vec<Geo>)>>,
}

impl Icons {
    pub fn new() -> Icons {
        Icons { defs: RefCell::new(HashMap::new()) }
    }

    fn ensure(&self, name: &str) {
        if self.defs.borrow().contains_key(name) {
            return;
        }
        let def = parse_icon(&source(name));
        let geoms = def
            .els
            .iter()
            .map(|e| match &e.kind {
                Kind::Path(d) => Geo::Path(crate::svg::to_path(&crate::svg::parse(d))),
                Kind::Rect(x, y, w, h, r) => {
                    if *r > 0.0 {
                        Geo::Path(rect_path(*x, *y, *w, *h, *r))
                    } else {
                        Geo::Path(sk::Path::rect(sk::Rect::from_xywh(*x, *y, *w, *h), None))
                    }
                }
                Kind::Circle(cx, cy, r) => Geo::Oval(sk::Rect::new(cx - r, cy - r, cx + r, cy + r)),
            })
            .collect();
        self.defs.borrow_mut().insert(name.to_string(), (def, geoms));
    }

    /// Order 025: an icon's viewBox width and height (for `El::icon_fit`).
    pub fn view_box(&self, name: &str) -> (f32, f32) {
        self.ensure(name);
        let d = self.defs.borrow();
        (d[name].0.vb, d[name].0.vbh)
    }

    /// Stroke an icon into a `size` px box at (x, y), clipped to that box (an `<svg>` element's overflow clip).
    /// `class_op` gives extra opacity per SVG class (e.g. the mute cross).
    pub fn draw(&self, g: &Gfx, name: &str, x: f32, y: f32, size: f32, stroke: f32, c: Rgba, class_op: &dyn Fn(&str) -> f32) {
        self.draw_ex(g, name, x, y, size, stroke, c, class_op, true);
    }

    pub fn draw_ex(&self, g: &Gfx, name: &str, x: f32, y: f32, size: f32, stroke: f32, c: Rgba, class_op: &dyn Fn(&str) -> f32, clip: bool) {
        self.ensure(name);
        let d = self.defs.borrow();
        let (def, geoms) = &d[name];
        let k = size / def.vb;
        // an <svg> is a replaced element: Blink paints it at its pixel-snapped position
        let (x, y, _, _) = g.snap(x, y, size, size);
        let t0 = g.transform();
        g.set_transform(&(Matrix3x2::translation(x, y) * t0));
        if clip {
            g.push_clip(0.0, 0.0, size, size * def.vbh / def.vb);
        }
        // the caption glyphs use SVG's default butt caps; minimize is `shape-rendering: crispEdges`
        let round = !matches!(name, "wmin" | "wcls");
        let aa = name != "wmin";
        let t1 = g.transform();
        for (e, gm) in def.els.iter().zip(geoms.iter()) {
            let op = e.opacity * class_op(&e.class);
            if op <= 0.001 {
                continue;
            }
            let s = k * e.scale;
            g.set_transform(&(Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: 0.0, M32: 0.0 } * t1));
            let w = e.sw.unwrap_or(stroke);
            match gm {
                // an element's opacity: one draw, so Chromium folds it into the paint (float, after the 8-bit colour)
                Geo::Path(p) if e.filled => g.fill_geom(p, crate::ui::GREEN().mul_a(op)),
                Geo::Path(p) => g.stroke_geom_ex(p, w, c, round, aa, op),
                Geo::Oval(r) => g.stroke_oval(*r, w, c, op),
            }
        }
        if clip {
            g.pop_clip();
        }
        g.set_transform(&t0);
    }
    /// Order 025: like `draw`, with parts filled / coloured as `p` says (`El::icon_paint`): `fill_all` fills every part with
    /// `c` and strokes none; a class's `Fill(col)` fills that part (no stroke), `Stroke(col)` strokes it in `col`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_styled(&self, g: &Gfx, name: &str, x: f32, y: f32, size: f32, stroke: f32, c: Rgba, p: &crate::ui::el::IconPaint, class_op: &dyn Fn(&str) -> f32) {
        use crate::ui::el::ClassPaint;
        self.ensure(name);
        let d = self.defs.borrow();
        let (def, geoms) = &d[name];
        let k = size / def.vb;
        let (x, y, _, _) = g.snap(x, y, size, size);
        let t0 = g.transform();
        g.set_transform(&(Matrix3x2::translation(x, y) * t0));
        g.push_clip(0.0, 0.0, size, size * def.vbh / def.vb);
        let t1 = g.transform();
        for (e, gm) in def.els.iter().zip(geoms.iter()) {
            let op = e.opacity * class_op(&e.class);
            if op <= 0.001 {
                continue;
            }
            let s = k * e.scale;
            g.set_transform(&(Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: 0.0, M32: 0.0 } * t1));
            let w = e.sw.unwrap_or(stroke);
            let cls = p.classes.iter().find(|(n, _)| !n.is_empty() && e.class.split_whitespace().any(|x| x == n)).map(|(_, cp)| *cp);
            let fill = if p.fill_all { Some(c) } else if let Some(ClassPaint::Fill(col)) = cls { Some(col) } else { None };
            let path = match gm {
                Geo::Path(pa) => pa.clone(),
                Geo::Oval(r) => sk::Path::oval(*r, None),
            };
            match fill {
                Some(col) => g.fill_geom(&path, col.mul_a(op)),
                None => {
                    let col = if let Some(ClassPaint::Stroke(sc)) = cls { sc } else { c };
                    match gm {
                        Geo::Path(pa) => g.stroke_geom_ex(pa, w, col, true, true, op),
                        Geo::Oval(r) => g.stroke_oval(*r, w, col, op),
                    }
                }
            }
        }
        g.pop_clip();
        g.set_transform(&t0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_icons() {
        let t = parse_icon(&source("tool"));
        assert_eq!(t.els.len(), 1);
        assert!((t.els[0].scale - 0.83333).abs() < 1e-5);
        assert_eq!(t.els[0].sw, Some(1.8));
        let m = parse_icon(&source("mouse"));
        assert_eq!(m.els.len(), 2);
        assert!((m.els[1].opacity - 0.55).abs() < 1e-6);
        let s = parse_icon(&source("spkM"));
        assert_eq!(s.els[2].class, "x");
        let g = parse_icon(&source("gear"));
        assert_eq!(g.els.len(), 2);
        assert_eq!(parse_icon(&source("chev")).vb, 9.0);
    }
}
