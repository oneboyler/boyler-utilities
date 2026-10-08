//! The marks on the picture, drawn with Skia exactly as the drawing's canvas draws them (menu-v22 strokePts / soft / drawAnn /
//! rr; Chromium's canvas 2D is Skia too): pen 4 px smoothed, highlighter 18 px at 38 %, arrow 4 px with a filled head up to
//! 20 px, box 4 px radius 4, text 600 20 px, emoji 38 px with its 260 ms pop-in. The canvas' matrix must map DESKTOP pixels to
//! the target; every CSS px size is multiplied by the mark's own scale.

use std::cell::RefCell;

use skia_safe as sk;
use sk::{Color4f, Paint, PaintCap, PaintJoin, PaintStyle, Path, PathBuilder, Point};

use super::model::{Ann, Pt};

pub fn color(c: u32, a: f32) -> Color4f {
    Color4f::new(((c >> 16) & 255) as f32 / 255.0, ((c >> 8) & 255) as f32 / 255.0, (c & 255) as f32 / 255.0, a)
}

/// The canvas' shadowBlur -> a Gaussian sigma (Chromium: `SkBlurMask::ConvertRadiusToSigma(blur / 2)`... see the report:
/// the one that matches the drawing pixel for pixel is used).
pub fn shadow_sigma(blur: f32) -> f32 {
    blur / 2.0
}

/// ctx.shadowColor / shadowBlur / shadowOffsetY as a drop shadow under what the paint draws.
fn shadowed(p: &mut Paint, a: f32, blur: f32, dy: f32, s: f32) {
    let sg = shadow_sigma(blur) * s;
    p.set_image_filter(sk::image_filters::drop_shadow((0.0, dy * s), (sg, sg), color(0, a), None, None, None));
}

fn stroke(c: u32, a: f32, w: f32) -> Paint {
    let mut p = Paint::new(color(c, a), None);
    p.set_anti_alias(true).set_style(PaintStyle::Stroke).set_stroke_width(w).set_stroke_cap(PaintCap::Round).set_stroke_join(PaintJoin::Round);
    p
}

/// strokePts: a smooth line through the points (quadratic curves through the midpoints).
pub fn smooth_path(pts: &[Pt]) -> Path {
    let mut path = PathBuilder::new();
    let Some(&first) = pts.first() else { return path.detach() };
    path.move_to(first);
    if pts.len() < 3 {
        for p in pts {
            path.line_to(*p);
        }
        let l = pts[pts.len() - 1];
        path.line_to((l.0 + 0.01, l.1));
    } else {
        for i in 1..pts.len() - 1 {
            let (p, q) = (pts[i], pts[i + 1]);
            path.quad_to(p, ((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0));
        }
        path.line_to(pts[pts.len() - 1]);
    }
    path.detach()
}

/// rr: the rounded box path made with arcTo, like the drawing.
pub fn rr_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut p = PathBuilder::new();
    p.move_to((x + r, y));
    p.arc_to_tangent((x + w, y), (x + w, y + h), r);
    p.arc_to_tangent((x + w, y + h), (x, y + h), r);
    p.arc_to_tangent((x, y + h), (x, y), r);
    p.arc_to_tangent((x, y), (x + w, y), r);
    p.close();
    p.detach()
}

thread_local! {
    static FM: sk::FontMgr = sk::FontMgr::new();
    static FONTS: RefCell<Vec<(String, u16, sk::Typeface)>> = const { RefCell::new(Vec::new()) };
}

/// A typeface by family and weight (cached).
pub fn typeface(family: &str, weight: u16) -> Option<sk::Typeface> {
    if let Some(t) = FONTS.with(|f| f.borrow().iter().find(|x| x.0 == family && x.1 == weight).map(|x| x.2.clone())) {
        return Some(t);
    }
    let style = sk::FontStyle::new(sk::font_style::Weight::from(weight as i32), sk::font_style::Width::NORMAL, sk::font_style::Slant::Upright);
    let tf = FM.with(|fm| fm.match_family_style(family, style))?;
    FONTS.with(|f| f.borrow_mut().push((family.to_string(), weight, tf.clone())));
    Some(tf)
}

/// Segoe UI Variable at a weight with its optical size set to the CSS font size, as Chromium does for a variable font
/// (`font-optical-sizing: auto` -> opsz = the font size in CSS px).
pub fn opsz_typeface(weight: u16, css_size: f32) -> Option<sk::Typeface> {
    let tf = typeface("Segoe UI Variable Text", weight)?;
    let tag = |t: &[u8; 4]| sk::FourByteTag::from_chars(t[0] as char, t[1] as char, t[2] as char, t[3] as char);
    let coords = [
        sk::font_arguments::variation_position::Coordinate { axis: tag(b"wght"), value: weight as f32 },
        sk::font_arguments::variation_position::Coordinate { axis: tag(b"opsz"), value: css_size },
    ];
    let args = sk::FontArguments::new().set_variation_design_position(sk::font_arguments::VariationPosition { coordinates: &coords });
    tf.clone_with_arguments(&args).or(Some(tf))
}

/// A shaped line of one font (emoji sequences with VS16 / ZWJ come out as one glyph): the blob drawn with its baseline at
/// y = 0, its advance, and the font's ascent / descent.
#[derive(Clone)]
pub struct Shaped {
    pub blob: Option<sk::TextBlob>,
    pub adv: f32,
    pub asc: f32,
    pub desc: f32,
}

thread_local! {
    static SHAPER: sk::Shaper = sk::Shaper::new_shape_dont_wrap_or_reorder(None).expect("shaper");
    static SHAPED: RefCell<std::collections::HashMap<(String, String, u16, u32), Shaped>> = RefCell::new(std::collections::HashMap::new());
}

/// Blink's canvas baselines (`NormalizedTypoAscentAndDescent`): the OS/2 typo ascender / descender scaled so the two add
/// up to the font size, in LayoutUnits. `textBaseline = top` puts the baseline this ascent under y; `middle` puts it
/// (ascent - descent) / 2 under y.
pub fn typo_ascent_descent(family: &str, weight: u16, size: f32) -> Option<(f32, f32)> {
    let tf = typeface(family, weight)?;
    let tag = u32::from_be_bytes(*b"OS/2");
    let mut d = vec![0u8; tf.get_table_size(tag)?];
    if tf.get_table_data(tag, &mut d) < 72 {
        return None;
    }
    let a = i16::from_be_bytes([d[68], d[69]]) as f32;
    let dsc = -(i16::from_be_bytes([d[70], d[71]]) as f32);
    let h = a + dsc;
    if h <= 0.0 || a < 0.0 || a > h {
        return None;
    }
    let lu = |v: f32| (v * 64.0).round() / 64.0;
    let asc = lu(a * size / h);
    Some((asc, lu(size) - asc))
}

pub fn shaped(s: &str, family: &str, weight: u16, size: f32) -> Option<Shaped> {
    shaped_tf(s, family, weight, size, None)
}

/// `shaped` with an optical size for Segoe UI Variable (see `opsz_typeface`).
pub fn shaped_tf(s: &str, family: &str, weight: u16, size: f32, opsz: Option<f32>) -> Option<Shaped> {
    let k = (format!("{}|{}", s, opsz.unwrap_or(0.0)), family.to_string(), weight, (size * 64.0) as u32);
    if let Some(v) = SHAPED.with(|c| c.borrow().get(&k).cloned()) {
        return Some(v);
    }
    struct Grab {
        g: Vec<sk::GlyphId>,
        p: Vec<Point>,
        cur: Vec<sk::GlyphId>,
        curp: Vec<Point>,
    }
    impl sk::shaper::run_handler::RunHandler for Grab {
        fn begin_line(&mut self) {}
        fn run_info(&mut self, _i: &sk::shaper::run_handler::RunInfo) {}
        fn commit_run_info(&mut self) {}
        fn run_buffer(&mut self, i: &sk::shaper::run_handler::RunInfo) -> sk::shaper::run_handler::Buffer {
            self.cur = vec![0; i.glyph_count];
            self.curp = vec![Point::default(); i.glyph_count];
            sk::shaper::run_handler::Buffer::new(&mut self.cur, &mut self.curp, None)
        }
        fn commit_run_buffer(&mut self, _i: &sk::shaper::run_handler::RunInfo) {
            self.g.extend_from_slice(&self.cur);
            self.p.extend_from_slice(&self.curp);
        }
        fn commit_line(&mut self) {}
    }
    let tf = match opsz {
        Some(o) => opsz_typeface(weight, o)?,
        None => typeface(family, weight)?,
    };
    let mut font = sk::Font::from_typeface(tf, size);
    font.set_subpixel(true)
        .set_hinting(sk::FontHinting::Normal)
        .set_edging(if family == "Segoe UI Emoji" { sk::font::Edging::AntiAlias } else { sk::font::Edging::SubpixelAntiAlias })
        .set_embedded_bitmaps(true)
        .set_linear_metrics(false);
    let mut gr = Grab { g: vec![], p: vec![], cur: vec![], curp: vec![] };
    let n = s.len();
    let mut fri = sk::Shaper::new_trivial_font_run_iterator(&font, n);
    let mut bri = sk::Shaper::new_trivial_bidi_run_iterator(0, n);
    let mut sri = sk::Shaper::new_trivial_script_run_iterator(0, n);
    let mut lri = sk::Shaper::new_trivial_language_run_iterator("en", n);
    SHAPER.with(|sh| sh.shape_with_iterators(s, &mut fri, &mut bri, &mut sri, &mut lri, f32::MAX, &mut gr));
    let mut wd = vec![0.0f32; gr.g.len()];
    font.get_widths(&gr.g, &mut wd);
    let adv = gr.p.last().map(|p| p.x).unwrap_or(0.0) + wd.last().copied().unwrap_or(0.0);
    let xs: Vec<f32> = gr.p.iter().map(|p| p.x).collect();
    let blob = if gr.g.is_empty() { None } else { sk::TextBlob::from_pos_text_h(&gr.g[..], &xs, 0.0, &font) };
    let (_, m) = font.metrics();
    let v = Shaped { blob, adv, asc: -m.ascent, desc: m.descent };
    SHAPED.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 4000 {
            c.clear();
        }
        c.insert(k, v.clone());
    });
    Some(v)
}

/// Draw one mark. `now` = the clock for the emoji pop-in (None = finished, as in the saved picture).
pub fn draw(cv: &sk::Canvas, a: &Ann, now: Option<f64>) {
    match a {
        Ann::Pen { c, pts, s } => {
            let mut p = stroke(*c, 1.0, 4.0 * s);
            shadowed(&mut p, 0.3, 3.0, 1.0, *s);
            cv.draw_path(&smooth_path(pts), &p);
        }
        Ann::Hl { c, pts, s } => {
            cv.draw_path(&smooth_path(pts), &stroke(*c, 0.38, 18.0 * s));
        }
        Ann::Arrow { c, a, b, s } => {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let l = dx.hypot(dy);
            if l < 2.0 {
                return;
            }
            let (ux, uy) = (dx / l, dy / l);
            let hl = (20.0 * s).min(l * 0.5);
            let hw = hl * 0.6;
            let mut p = stroke(*c, 1.0, 4.0 * s);
            shadowed(&mut p, 0.3, 3.0, 1.0, *s);
            let mut line = PathBuilder::new();
            line.move_to(*a);
            line.line_to((b.0 - ux * hl * 0.7, b.1 - uy * hl * 0.7));
            cv.draw_path(&line.detach(), &p);
            let mut head = PathBuilder::new();
            head.move_to(*b);
            head.line_to((b.0 - ux * hl - uy * hw, b.1 - uy * hl + ux * hw));
            head.line_to((b.0 - ux * hl + uy * hw, b.1 - uy * hl - ux * hw));
            head.close();
            p.set_style(PaintStyle::Fill);
            cv.draw_path(&head.detach(), &p);
        }
        Ann::Box { c, a, b, s } => {
            let mut p = stroke(*c, 1.0, 4.0 * s);
            shadowed(&mut p, 0.3, 3.0, 1.0, *s);
            cv.draw_path(&rr_path(a.0.min(b.0), a.1.min(b.1), (b.0 - a.0).abs(), (b.1 - a.1).abs(), 4.0 * s), &p);
        }
        Ann::Text { c, text, x, y, s } => {
            // font 600 20px "Segoe UI Variable Text"; textBaseline top = the font's ascent under y
            let Some(Shaped { blob: Some(blob), .. }) = shaped_tf(text, "Segoe UI Variable Text", 600, 20.0 * s, None) else { return };
            let asc = typo_ascent_descent("Segoe UI Variable Text", 600, 20.0 * s).map(|v| v.0).unwrap_or(20.0 * s);
            let mut p = Paint::new(color(*c, 1.0), None);
            p.set_anti_alias(true);
            shadowed(&mut p, 0.55, 4.0, 1.0, *s);
            cv.draw_text_blob(&blob, (*x, *y + asc), &p);
        }
        Ann::Emo { e, x, y, size, born, s } => {
            let k = match now {
                Some(n) => ((n - born) / 260.0).clamp(0.0, 1.0) as f32,
                None => 1.0,
            };
            let sc = if k < 0.6 { 0.5 + 0.65 * (k / 0.6) } else { 1.15 - 0.15 * ((k - 0.6) / 0.4) };
            let Some(Shaped { blob: Some(blob), adv: w, .. }) = shaped(e, "Segoe UI Emoji", 400, size * s) else { return };
            let (asc, desc) = typo_ascent_descent("Segoe UI Emoji", 400, size * s).unwrap_or((size * s * 0.8, size * s * 0.2));
            let mut p = Paint::default();
            p.set_anti_alias(true);
            shadowed(&mut p, 0.3, 6.0, 2.0, *s);
            cv.save();
            cv.translate((*x, *y));
            cv.scale((sc, sc));
            // textAlign center, textBaseline middle
            cv.draw_text_blob(&blob, (-w / 2.0, (asc - desc) / 2.0), &p);
            cv.restore();
        }
    }
}

/// Every mark, clipped to the box (coRender). True while an emoji is still popping in.
pub fn draw_all(cv: &sk::Canvas, anns: &[Ann], clip: sk::Rect, now: Option<f64>) -> bool {
    cv.save();
    cv.clip_rect(clip, sk::ClipOp::Intersect, false);
    let mut busy = false;
    for a in anns {
        draw(cv, a, now);
        if let (Ann::Emo { born, .. }, Some(n)) = (a, now) {
            busy |= n - born < 260.0;
        }
    }
    cv.restore();
    busy
}
