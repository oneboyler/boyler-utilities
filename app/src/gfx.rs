//! The painter: Skia (CPU raster) with Chromium's own settings and Blink's paint rules (Order 003).
//! The research (internal research notes) is the spec; every paint operation here copies
//! the operation Chromium records for the same CSS (read from its own command log, tools/ref/cdp_dump.js):
//! - boxes on pixel-snapped rects, anti-aliased rounded rects (Skia's own coverage);
//! - outer box-shadows: clip out the border box (shrunk 1 px when the background is opaque), draw the shadow shape with the
//!   shadow colour and a blur mask filter (sigma = blur / 2);
//! - inset box-shadows: clip to the box, then a rect with a rounded hole (DRRect), offset, blurred the same way;
//! - gradients: premultiplied interpolation in sRGB, always dithered;
//! - text: HarfBuzz shaping, Segoe UI Variable Text / Display, subpixel positioning, normal hinting, ClearType edging on a
//!   surface of unknown pixel layout (= grey masks made from ClearType masks), Skia's text gamma = sRGB curve, contrast 1.0.
//! Everything is in DIPs (CSS px); the canvas matrix carries the display scale.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use skia_safe as sk;
use skia_safe::{gradient_shader, shaper, Canvas, ClipOp, Color4f, FontMgr, Paint, PaintStyle, Path, Point, RRect, Rect};
use windows_numerics::Matrix3x2;

/// A CSS colour (straight alpha).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba(pub f32, pub f32, pub f32, pub f32);

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
        Rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Rgba {
        Rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a)
    }
    pub fn hex(h: u32) -> Rgba {
        Rgba::rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
    }
    pub fn a(self, a: f32) -> Rgba {
        Rgba(self.0, self.1, self.2, a)
    }
    pub fn mul_a(self, k: f32) -> Rgba {
        Rgba(self.0, self.1, self.2, self.3 * k)
    }
    pub fn mix(self, o: Rgba, t: f32) -> Rgba {
        Rgba(self.0 + (o.0 - self.0) * t, self.1 + (o.1 - self.1) * t, self.2 + (o.2 - self.2) * t, self.3 + (o.3 - self.3) * t)
    }
    /// CSS `grayscale(1)`.
    pub fn gray(self) -> Rgba {
        let l = 0.2126 * self.0 + 0.7152 * self.1 + 0.0722 * self.2;
        Rgba(l, l, l, self.3)
    }
    /// The colour as Chromium hands it to Skia: a legacy CSS colour (rgb() / rgba() / hex) is 8 bits per channel,
    /// alpha included (rgba(255,255,255,.16) -> alpha 41/255). Proven in the Order 003 lab: float colours give 1-level
    /// differences on every gradient, 8-bit colours give none.
    pub fn c4(self) -> Color4f {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() / 255.0;
        Color4f::new(q(self.0), q(self.1), q(self.2), q(self.3))
    }
}

/// A font: Segoe UI Variable at a size and weight; `display` = the "Display" optical cut (page titles).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Font {
    pub size100: u32,
    pub weight: u16,
    pub display: bool,
    pub tnum: bool,
    /// letter-spacing in 1/1000 px (#sw sets -.006em at 13 px = -0.078 px, inherited as a length)
    pub ls1000: i32,
    /// Order 025: `"Cascadia Mono","Consolas",monospace` (the drawing's address fields)
    pub mono: bool,
}

impl Font {
    pub const fn new(size: f32, weight: u16) -> Font {
        Font { size100: (size * 100.0) as u32, weight, display: false, tnum: false, ls1000: -78, mono: false }
    }
    pub const fn display(size: f32, weight: u16) -> Font {
        Font { size100: (size * 100.0) as u32, weight, display: true, tnum: false, ls1000: (-0.018 * size * 1000.0) as i32, mono: false }
    }
    /// Order 025: the monospace family (`"Cascadia Mono","Consolas",monospace`).
    pub const fn mono(mut self) -> Font {
        self.mono = true;
        self
    }
    pub const fn ls(mut self, px1000: i32) -> Font {
        self.ls1000 = px1000;
        self
    }
    pub const fn tnum(mut self) -> Font {
        self.tnum = true;
        self
    }
    pub fn size(&self) -> f32 {
        self.size100 as f32 / 100.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// One shaped line of text: glyphs, their x offsets (letter-spacing included) and the CSS width.
pub struct TextBox {
    font: sk::Font,
    glyphs: Vec<sk::GlyphId>,
    xs: Vec<f32>,
    /// the width as Blink lays it out (LayoutUnit, rounded up to 1/64 px)
    pub width: f32,
    /// the shaped width itself (centring uses it)
    pub raw: f32,
}

#[derive(Clone, Copy)]
struct FontMetrics {
    ascent: f32,
    descent: f32,
}

/// How a CSS line box places its text: integer-rounded ascent/descent, half-leading, baseline on a whole device pixel.
pub fn baseline_in_line(asc: f32, desc: f32, lh: f32) -> f32 {
    let (a, d) = (asc.round(), desc.round());
    // Chromium's CalculateLeadingSpace: the half-leading above the text is floored to a whole pixel
    let hl = (((lh - (a + d)) * 64.0).round() / 64.0 / 2.0).floor();
    hl + a
}

/// Blink's LayoutUnit: a float rounded UP to 1/64 px (text widths).
pub fn lu_ceil(v: f32) -> f32 {
    (v * 64.0).ceil() / 64.0
}

/// One CSS box-shadow (all in CSS px).
#[derive(Clone, Copy, Debug)]
pub struct Shadow {
    pub dx: f32,
    pub dy: f32,
    pub blur: f32,
    pub spread: f32,
    pub c: Rgba,
}
pub const fn sh(dx: f32, dy: f32, blur: f32, spread: f32, c: Rgba) -> Shadow {
    Shadow { dx, dy, blur, spread, c }
}

/// Test-only experiment switches for gradients: `BU_GRAD=nodither,unpremul,nocs` (Order 003 root-cause hunting).
fn grad_var(name: &str) -> bool {
    thread_local! {
        static V: String = crate::testmode::env("BU_GRAD").unwrap_or_default();
    }
    V.with(|v| v.split(',').any(|x| x == name))
}

thread_local! {
    /// test-only A/B switch: BU_NOSNAP=1 draws boxes at their exact fractional positions
    static NO_SNAP: bool = crate::testmode::env("BU_NOSNAP").is_some();
}

/// The surface properties Chromium rasters text with on Windows: unknown pixel layout (grey masks from ClearType masks),
/// text contrast 1.0, text gamma 0 (= the sRGB curve) — or the ClearType Tuner's values when that registry key exists.
pub fn surface_props() -> sk::SurfaceProps {
    let (contrast, gamma) = crate::textparams::contrast_gamma();
    sk::SurfaceProps::new_with_text_properties(sk::SurfacePropsFlags::default(), sk::PixelGeometry::Unknown, contrast, gamma)
}

/// A CPU raster surface (premultiplied BGRA, sRGB-tagged like Chromium's raster tiles).
pub fn new_surface(w: i32, h: i32) -> Option<sk::Surface> {
    let ii = sk::ImageInfo::new((w.max(1), h.max(1)), sk::ColorType::BGRA8888, sk::AlphaType::Premul, Some(sk::ColorSpace::new_srgb()));
    sk::surfaces::raster(&ii, None, Some(&surface_props()))
}

/// The painter. It draws on whatever canvas is current (`begin` / `end`), so the UI code stays target-free.
pub struct Gfx {
    pub scale: f32,
    cur: Cell<*const Canvas>,
    stack: RefCell<Vec<*const Canvas>>,
    fm: FontMgr,
    shaper: sk::Shaper,
    fonts: RefCell<HashMap<Font, sk::Font>>,
    metrics: RefCell<HashMap<Font, FontMetrics>>,
    layouts: RefCell<HashMap<(String, Font, i32), Rc<TextBox>>>,
    geoms: RefCell<HashMap<String, Path>>,
    layers: RefCell<Vec<usize>>,
    pass: Cell<Pass>,
    live_depth: Cell<u32>,
}

/// Which part of the content a drawing pass paints: everything, only the static part, or only the live part (the level
/// meters - in the drawing they are composited layers of their own: canvases, range inputs, `.lvl i`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    All,
    Static,
    Live,
}

fn rr(x: f32, y: f32, w: f32, h: f32, r: f32) -> RRect {
    // CSS radii: scaled down together only when they overlap (one uniform radius here)
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    RRect::new_rect_xy(Rect::from_xywh(x, y, w, h), r, r)
}

fn paint(c: Rgba) -> Paint {
    let mut p = Paint::new(c.c4(), None);
    p.set_anti_alias(true);
    p
}

/// A rounded rect grown (spread > 0) or shrunk by `s` with the CSS shadow-shape corner rule.
fn spread_rr(x: f32, y: f32, w: f32, h: f32, r: f32, s: f32) -> RRect {
    let nr = if s > 0.0 && r < s && r > 0.0 {
        let q = r / s - 1.0;
        r + s * (1.0 + q * q * q)
    } else if r <= 0.0 {
        0.0
    } else {
        (r + s).max(0.0)
    };
    rr(x - s, y - s, w + 2.0 * s, h + 2.0 * s, nr)
}

impl Gfx {
    pub fn new(scale: f32) -> Gfx {
        let fm = FontMgr::default();
        let shaper = sk::Shaper::new_shape_dont_wrap_or_reorder(None).expect("shaper");
        Gfx {
            scale,
            cur: Cell::new(std::ptr::null()),
            stack: RefCell::new(Vec::new()),
            fm,
            shaper,
            fonts: RefCell::new(HashMap::new()),
            metrics: RefCell::new(HashMap::new()),
            layouts: RefCell::new(HashMap::new()),
            geoms: RefCell::new(HashMap::new()),
            layers: RefCell::new(Vec::new()),
            pass: Cell::new(Pass::All),
            live_depth: Cell::new(0),
        }
    }

    pub fn set_pass(&self, p: Pass) {
        self.pass.set(p);
    }
    /// Draw the live part (level meters): skipped in the static pass, the only thing drawn in the live pass.
    pub fn live(&self, f: impl FnOnce()) {
        self.live_depth.set(self.live_depth.get() + 1);
        f();
        self.live_depth.set(self.live_depth.get() - 1);
    }
    fn off(&self) -> bool {
        match self.pass.get() {
            Pass::All => false,
            Pass::Static => self.live_depth.get() > 0,
            Pass::Live => self.live_depth.get() == 0,
        }
    }

    /// Start drawing on `c` (the display scale goes on its matrix). Calls nest.
    pub fn begin(&self, c: &Canvas) {
        self.stack.borrow_mut().push(self.cur.get());
        self.cur.set(c as *const Canvas);
        c.save();
        c.reset_matrix();
        c.scale((self.scale, self.scale));
    }
    pub fn end(&self) {
        self.cv().restore();
        let prev = self.stack.borrow_mut().pop().unwrap_or(std::ptr::null());
        self.cur.set(prev);
    }
    /// A surface of the same kind as the current canvas (on the GPU when it draws on the GPU - Order 051), for a picture that
    /// is made now and drawn into it; a CPU surface when the canvas is a recording or a CPU one.
    pub fn surface_like(&self, w: i32, h: i32) -> Option<sk::Surface> {
        let ii = sk::ImageInfo::new((w.max(1), h.max(1)), sk::ColorType::BGRA8888, sk::AlphaType::Premul, Some(sk::ColorSpace::new_srgb()));
        let c = self.cur.get();
        if !c.is_null() {
            if let Some(s) = unsafe { &*c }.new_surface(&ii, Some(&surface_props())) {
                return Some(s);
            }
        }
        new_surface(w, h)
    }

    pub fn cv(&self) -> &Canvas {
        let p = self.cur.get();
        assert!(!p.is_null(), "no canvas");
        unsafe { &*p }
    }

    // ------------------------------------------------------------------ text
    fn sk_font(&self, f: Font) -> sk::Font {
        if let Some(x) = self.fonts.borrow().get(&f) {
            return x.clone();
        }
        let fam = if f.display { "Segoe UI Variable Display" } else { "Segoe UI Variable Text" };
        let style = sk::FontStyle::new(sk::font_style::Weight::from(f.weight as i32), sk::font_style::Width::NORMAL, sk::font_style::Slant::Upright);
        // Order 025: the monospace fields ("Cascadia Mono","Consolas")
        let mono = if f.mono { self.fm.match_family_style("Cascadia Mono", style).or_else(|| self.fm.match_family_style("Consolas", style)) } else { None };
        let tf = mono.or_else(|| self.fm.match_family_style(fam, style)).or_else(|| self.fm.match_family_style("Segoe UI", style)).expect("font");
        let mut font = sk::Font::from_typeface(tf, f.size());
        font.set_subpixel(true)
            .set_hinting(sk::FontHinting::Normal)
            .set_edging(sk::font::Edging::SubpixelAntiAlias)
            .set_embedded_bitmaps(true)
            .set_linear_metrics(false);
        self.fonts.borrow_mut().insert(f, font.clone());
        font
    }

    fn font_metrics(&self, f: Font) -> FontMetrics {
        if let Some(m) = self.metrics.borrow().get(&f) {
            return *m;
        }
        let (_, m) = self.sk_font(f).metrics();
        let fm = FontMetrics { ascent: -m.ascent, descent: m.descent };
        self.metrics.borrow_mut().insert(f, fm);
        fm
    }

    fn shape(&self, s: &str, f: Font) -> (sk::Font, Vec<sk::GlyphId>, Vec<f32>, f32) {
        struct Grab {
            g: Vec<sk::GlyphId>,
            p: Vec<Point>,
            cur: Vec<sk::GlyphId>,
            curp: Vec<Point>,
        }
        impl shaper::run_handler::RunHandler for Grab {
            fn begin_line(&mut self) {}
            fn run_info(&mut self, _i: &shaper::run_handler::RunInfo) {}
            fn commit_run_info(&mut self) {}
            fn run_buffer(&mut self, i: &shaper::run_handler::RunInfo) -> shaper::run_handler::Buffer {
                self.cur = vec![0; i.glyph_count];
                self.curp = vec![Point::default(); i.glyph_count];
                shaper::run_handler::Buffer::new(&mut self.cur, &mut self.curp, None)
            }
            fn commit_run_buffer(&mut self, _i: &shaper::run_handler::RunInfo) {
                self.g.extend_from_slice(&self.cur);
                self.p.extend_from_slice(&self.curp);
            }
            fn commit_line(&mut self) {}
        }
        let mut font = self.sk_font(f);
        // a character the font lacks (✓): the text is shaped with the font Windows falls back to for it, like Chromium
        if let Some(ch) = s.chars().find(|&c| !c.is_whitespace() && font.unichar_to_glyph(c as i32) == 0) {
            let style = font.typeface().font_style();
            if let Some(tf) = self.fm.match_family_style_character("", style, &["en"], ch as i32) {
                font.set_typeface(tf);
            }
        }
        let mut gr = Grab { g: vec![], p: vec![], cur: vec![], curp: vec![] };
        let feats: Vec<shaper::Feature> =
            if f.tnum { vec![shaper::Feature { tag: u32::from_be_bytes(*b"tnum"), value: 1, start: 0, end: usize::MAX }] } else { vec![] };
        let n = s.len();
        let mut fri = sk::Shaper::new_trivial_font_run_iterator(&font, n);
        let mut bri = sk::Shaper::new_trivial_bidi_run_iterator(0, n);
        let mut sri = sk::Shaper::new_trivial_script_run_iterator(0, n);
        let mut lri = sk::Shaper::new_trivial_language_run_iterator("en", n);
        self.shaper.shape_with_iterators_and_features(s, &mut fri, &mut bri, &mut sri, &mut lri, &feats, f32::MAX, &mut gr);
        let mut wd = vec![0.0f32; gr.g.len()];
        font.get_widths(&gr.g, &mut wd);
        let ls = f.ls1000 as f32 / 1000.0;
        // Blink: letter-spacing after every glyph
        let xs: Vec<f32> = gr.p.iter().enumerate().map(|(i, p)| p.x + ls * i as f32).collect();
        let w = gr.p.last().map(|p| p.x).unwrap_or(0.0) + wd.last().copied().unwrap_or(0.0) + ls * gr.g.len() as f32;
        (font, gr.g, xs, w)
    }

    /// A cached shaped line. `maxw` > 0 trims with an ellipsis (CSS text-overflow: ellipsis).
    pub fn text_box(&self, s: &str, f: Font, maxw: f32) -> Rc<TextBox> {
        let key = (s.to_string(), f, (maxw * 8.0) as i32);
        if let Some(t) = self.layouts.borrow().get(&key) {
            return t.clone();
        }
        let (font, mut glyphs, mut xs, mut w) = self.shape(s, f);
        if maxw > 0.0 && lu_ceil(w) > maxw {
            // keep whole characters while the text + "…" fits
            let chars: Vec<char> = s.chars().collect();
            let mut n = chars.len();
            loop {
                n = n.saturating_sub(1);
                let t: String = chars[..n].iter().collect::<String>().trim_end().to_string() + "\u{2026}";
                let (_, g2, x2, w2) = self.shape(&t, f);
                if lu_ceil(w2) <= maxw || n == 0 {
                    glyphs = g2;
                    xs = x2;
                    w = w2;
                    break;
                }
            }
        }
        let tb = Rc::new(TextBox { font, glyphs, xs, width: lu_ceil(w), raw: w });
        let mut c = self.layouts.borrow_mut();
        if c.len() > 2000 {
            c.clear();
        }
        c.insert(key, tb.clone());
        tb
    }

    pub fn text_width(&self, s: &str, f: Font) -> f32 {
        self.text_box(s, f, 0.0).width
    }

    /// Order 025: where a font's baseline sits in a line box `lh` high (Blink's half-leading rule) - for inline text of
    /// two sizes on one line (a value + its smaller unit).
    pub fn baseline(&self, f: Font, lh: f32) -> f32 {
        let m = self.font_metrics(f);
        baseline_in_line(m.ascent, m.descent, lh)
    }

    /// Order 025: a font's ascent and descent rounded to whole px as Blink uses them (a selection highlight is that tall).
    pub fn asc_desc(&self, f: Font) -> (f32, f32) {
        let m = self.font_metrics(f);
        (m.ascent.round(), m.descent.round())
    }

    /// Draw one line of text in a CSS line box whose top is `y` and height `lh`.
    pub fn text(&self, s: &str, f: Font, x: f32, y: f32, lh: f32, c: Rgba, align: Align, maxw: f32) {
        if self.off() {
            return;
        }
        if s.is_empty() || c.3 <= 0.0 {
            return;
        }
        let m = self.font_metrics(f);
        let tb = self.text_box(s, f, maxw);
        let base = y + baseline_in_line(m.ascent, m.descent, lh);
        let x0 = match align {
            Align::Left => x,
            Align::Center => x - tb.width / 2.0,
            Align::Right => x - tb.width,
        };
        if tb.glyphs.is_empty() {
            return;
        }
        // Skia itself keeps x in quarter pixels and puts the baseline on a whole device pixel
        let xs: Vec<f32> = tb.xs.iter().map(|v| v + x0).collect();
        if let Some(blob) = sk::TextBlob::from_pos_text_h(&tb.glyphs[..], &xs, base, &tb.font) {
            self.cv().draw_text_blob(&blob, (0.0, 0.0), &paint(c));
        }
    }

    /// One glyph of a font by code point, drawn the way Chromium's NativeThemeFluent draws its scrollbar arrows:
    /// SkFont(typeface, size) with anti-aliased edging and subpixel positioning, origin (x, baseline).
    pub fn glyph(&self, family: &str, cp: char, size: f32, x: f32, baseline: f32, c: Rgba) {
        if self.off() {
            return;
        }
        let Some(tf) = self.fm.match_family_style(family, sk::FontStyle::normal()) else { return };
        let mut font = sk::Font::from_typeface(tf, size);
        font.set_edging(sk::font::Edging::AntiAlias).set_subpixel(true);
        let id = font.unichar_to_glyph(cp as i32);
        if id == 0 {
            return;
        }
        if let Some(blob) = sk::TextBlob::from_pos_text_h(&[id], &[0.0], 0.0, &font) {
            self.cv().draw_text_blob(&blob, (x, baseline), &paint(c));
        }
    }

    /// Text in its own box with `overflow: hidden` (+ ellipsis): the box is `maxw` wide when `fixed`, else as wide as the
    /// text (at most `maxw`); its clip is the pixel-snapped box.
    pub fn text_clipped(&self, s: &str, f: Font, x: f32, y: f32, lh: f32, c: Rgba, maxw: f32, fixed: bool) {
        let w = if fixed { maxw } else { self.text_box(s, f, maxw).width.min(maxw) };
        let (cx, cy, cw, ch) = self.snap(x, y, w, lh);
        self.push_clip(cx, cy, cw, ch);
        self.text(s, f, x, y, lh, c, Align::Left, maxw);
        self.pop_clip();
    }

    // ------------------------------------------------------------------ shapes
    /// Chromium paints boxes (backgrounds, borders, box-shadows) on pixel-snapped rectangles: the edges are rounded to
    /// whole device pixels in the box's own coordinates (before any transform, like a composited transform).
    pub fn snap(&self, x: f32, y: f32, w: f32, h: f32) -> (f32, f32, f32, f32) {
        if NO_SNAP.with(|n| *n) {
            return (x, y, w, h);
        }
        let s = self.scale;
        let rnd = |v: f32| (v + 0.5).floor();
        let (l, t) = (rnd(x * s) / s, rnd(y * s) / s);
        let (r, b) = (rnd((x + w) * s) / s, rnd((y + h) * s) / s);
        (l, t, r - l, b - t)
    }

    pub fn fill_rect(&self, x: f32, y: f32, w: f32, h: f32, c: Rgba) {
        if self.off() {
            return;
        }
        if c.3 <= 0.0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let (x, y, w, h) = self.snap(x, y, w, h);
        self.cv().draw_rect(Rect::from_xywh(x, y, w, h), &paint(c));
    }
    /// A rect without anti-aliasing (`shape-rendering: crispEdges`).
    pub fn fill_rect_crisp(&self, x: f32, y: f32, w: f32, h: f32, c: Rgba) {
        if self.off() {
            return;
        }
        let mut p = paint(c);
        p.set_anti_alias(false);
        self.cv().draw_rect(Rect::from_xywh(x, y, w, h), &p);
    }
    pub fn fill_rr(&self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Rgba) {
        if self.off() {
            return;
        }
        if c.3 <= 0.0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let (x, y, w, h) = self.snap(x, y, w, h);
        self.cv().draw_rrect(rr(x, y, w, h, r), &paint(c));
    }
    /// A rounded rect filled with a shader (a CSS gradient: always dithered).
    pub fn fill_rr_shader(&self, x: f32, y: f32, w: f32, h: f32, r: f32, sh: &sk::Shader, alpha: f32) {
        if self.off() {
            return;
        }
        let (x, y, w, h) = self.snap(x, y, w, h);
        let mut p = Paint::new(Color4f::new(0.0, 0.0, 0.0, alpha.clamp(0.0, 1.0)), None);
        p.set_anti_alias(true);
        p.set_dither(!grad_var("nodither"));
        p.set_shader(sh.clone());
        self.cv().draw_rrect(rr(x, y, w, h, r), &p);
    }
    /// A circle the way Blink paints `border-radius: 50%` on a square box (an oval rrect).
    pub fn fill_circle(&self, cx: f32, cy: f32, r: f32, c: Rgba) {
        if self.off() {
            return;
        }
        if c.3 <= 0.0 || r <= 0.0 {
            return;
        }
        let (x, y, w, h) = self.snap(cx - r, cy - r, 2.0 * r, 2.0 * r);
        self.cv().draw_rrect(rr(x, y, w, h, w.min(h) / 2.0), &paint(c));
    }
    pub fn line(&self, x1: f32, y1: f32, x2: f32, y2: f32, w: f32, c: Rgba, round: bool) {
        if self.off() {
            return;
        }
        if c.3 <= 0.0 {
            return;
        }
        let mut p = paint(c);
        p.set_style(PaintStyle::Stroke).set_stroke_width(w);
        if round {
            p.set_stroke_cap(sk::paint::Cap::Round);
        }
        self.cv().draw_line((x1, y1), (x2, y2), &p);
    }

    /// CSS outer `box-shadow` list of a (snapped) rounded box, painted the way Blink does: last listed first, all clipped
    /// out of the border box (shrunk by 1 px when the box's background is opaque).
    pub fn box_shadows(&self, x: f32, y: f32, w: f32, h: f32, r: f32, list: &[Shadow], opaque_bg: bool) {
        if self.off() {
            return;
        }
        if list.iter().all(|s| s.c.3 <= 0.0) {
            return;
        }
        let (x, y, w, h) = self.snap(x, y, w, h);
        let cv = self.cv();
        cv.save();
        let clip = if opaque_bg { spread_rr(x, y, w, h, r, -1.0) } else { rr(x, y, w, h, r) };
        cv.clip_rrect(clip, ClipOp::Difference, true);
        for s in list.iter().rev() {
            if s.c.3 <= 0.0 {
                continue;
            }
            let mut p = paint(s.c);
            if s.blur > 0.0 {
                p.set_mask_filter(sk::MaskFilter::blur(sk::BlurStyle::Normal, s.blur / 2.0, true));
            }
            let moved = s.dx != 0.0 || s.dy != 0.0;
            if moved {
                cv.save();
                cv.translate((s.dx, s.dy));
            }
            cv.draw_rrect(spread_rr(x, y, w, h, r, s.spread), &p);
            if moved {
                cv.restore();
            }
        }
        cv.restore();
    }
    /// One outer box-shadow (see `box_shadows`).
    pub fn box_shadow(&self, x: f32, y: f32, w: f32, h: f32, r: f32, dx: f32, dy: f32, blur: f32, spread: f32, c: Rgba, opaque_bg: bool) {
        self.box_shadows(x, y, w, h, r, &[sh(dx, dy, blur, spread, c)], opaque_bg);
    }
    /// `box-shadow: 0 0 0 <w>px c` (a ring just outside the edge).
    pub fn outset_ring(&self, x: f32, y: f32, w: f32, h: f32, r: f32, lw: f32, c: Rgba) {
        self.box_shadow(x, y, w, h, r, 0.0, 0.0, 0.0, lw, c, false);
    }

    /// CSS inset `box-shadow` list, painted the way Blink does (last listed first): each one clipped to the box, a rect with
    /// a rounded hole (the box shrunk by the spread) moved by the offset, blurred with sigma = blur / 2.
    pub fn inset_shadows(&self, x: f32, y: f32, w: f32, h: f32, r: f32, list: &[Shadow]) {
        if self.off() {
            return;
        }
        let (x, y, w, h) = self.snap(x, y, w, h);
        let cv = self.cv();
        for s in list.iter().rev() {
            if s.c.3 <= 0.0 {
                continue;
            }
            cv.save();
            cv.clip_rrect(rr(x, y, w, h, r), ClipOp::Intersect, true);
            // AreaCastingShadowInHole: the box grown by the blur (and a negative spread), united with the box moved back
            let g = s.blur + if s.spread < 0.0 { -s.spread } else { 0.0 };
            let a = Rect::from_xywh(x - g, y - g, w + 2.0 * g, h + 2.0 * g);
            let b = Rect::from_xywh(x - s.dx, y - s.dy, w, h);
            let outer = Rect::new(a.left.min(b.left), a.top.min(b.top), a.right.max(b.right), a.bottom.max(b.bottom));
            let hole = spread_rr(x, y, w, h, r, -s.spread);
            let mut p = paint(s.c);
            if s.blur > 0.0 {
                p.set_mask_filter(sk::MaskFilter::blur(sk::BlurStyle::Normal, s.blur / 2.0, true));
            }
            let moved = s.dx != 0.0 || s.dy != 0.0;
            if moved {
                cv.save();
                cv.translate((s.dx, s.dy));
            }
            cv.draw_drrect(RRect::new_rect(outer), hole, &p);
            if moved {
                cv.restore();
            }
            cv.restore();
        }
    }
    /// `box-shadow: inset 0 0 0 <w>px c` (a ring just inside the edge).
    pub fn inset_ring(&self, x: f32, y: f32, w: f32, h: f32, r: f32, lw: f32, c: Rgba) {
        self.inset_shadows(x, y, w, h, r, &[sh(0.0, 0.0, 0.0, lw, c)]);
    }

    /// A CSS linear gradient as a shader: premultiplied interpolation in sRGB (legacy colours).
    pub fn hgrad(&self, x0: f32, y0: f32, x1: f32, y1: f32, stops: &[(f32, Rgba)]) -> sk::Shader {
        let cols: Vec<Color4f> = stops.iter().map(|s| s.1.c4()).collect();
        let pos: Vec<f32> = stops.iter().map(|s| s.0).collect();
        let interp = sk::gradient::Interpolation {
            in_premul: if grad_var("unpremul") { sk::gradient::interpolation::InPremul::No } else { sk::gradient::interpolation::InPremul::Yes },
            color_space: sk::gradient::interpolation::ColorSpace::Destination,
            hue_method: sk::gradient::interpolation::HueMethod::Shorter,
        };
        gradient_shader::linear_with_interpolation(((x0, y0), (x1, y1)), (&cols[..], if grad_var("nocs") { None } else { Some(sk::ColorSpace::new_srgb()) }), &pos[..], sk::TileMode::Clamp, interp, None)
            .expect("gradient")
    }

    /// A blurred rounded rect (a canvas-2D style shadow: sigma given directly).
    pub fn fill_rr_blur(&self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Rgba, sigma: f32) {
        if self.off() {
            return;
        }
        if c.3 <= 0.0 || w <= 0.0 {
            return;
        }
        let mut p = paint(c);
        if sigma > 0.0 {
            p.set_mask_filter(sk::MaskFilter::blur(sk::BlurStyle::Normal, sigma, true));
        }
        self.cv().draw_rrect(rr(x, y, w, h, r), &p);
    }

    /// CSS `backdrop-filter: blur(<blur>px) <colour functions>` of a (snapped) rounded box, reading an already drawn
    /// picture of what is behind (`base`, device pixels): cropped to the box with mirrored edges (Chromium's edge mode),
    /// blurred with sigma = blur (Filter Effects: the value IS the standard deviation) in device pixels, then each colour
    /// function in order (each its own clamped colour matrix, as Chromium builds them), painted inside the rounded box.
    pub fn backdrop(&self, base: &sk::Image, x: f32, y: f32, w: f32, h: f32, r: f32, blur: f32, fx: &[CssColor]) {
        self.backdrop_in(base, x, y, w, h, Some(r), blur, &[], fx);
    }

    /// `backdrop` when flat colours were painted over `base` behind the box in this same pass (a popup's dim `.dlgw`):
    /// CSS blurs what is really behind, base + those colours. A flat colour over the whole blur area commutes with the
    /// blur, so it is blended in after the blur and before the colour filters (saturate) - the CSS order.
    #[allow(clippy::too_many_arguments)]
    pub fn backdrop_tinted(&self, base: &sk::Image, x: f32, y: f32, w: f32, h: f32, r: f32, blur: f32, tints: &[Rgba], fx: &[CssColor]) {
        self.backdrop_in(base, x, y, w, h, Some(r), blur, tints, fx);
    }

    /// `backdrop` without the rounded cut (`r` = None): the whole box, for a mask to cut later (test pictures).
    #[allow(clippy::too_many_arguments)]
    pub fn backdrop_in(&self, base: &sk::Image, x: f32, y: f32, w: f32, h: f32, r: Option<f32>, blur: f32, tints: &[Rgba], fx: &[CssColor]) {
        if self.off() {
            return;
        }
        let (x, y, w, h) = self.snap(x, y, w, h);
        let cv = self.cv();
        cv.save();
        match r {
            Some(r) => cv.clip_rrect(rr(x, y, w, h, r), ClipOp::Intersect, true),
            None => cv.clip_rect(Rect::from_xywh(x, y, w, h), ClipOp::Intersect, false),
        };
        let (dev, _) = cv.local_to_device_as_3x3().map_rect(Rect::from_xywh(x, y, w, h));
        let s = blur * self.scale;
        let mut f = sk::image_filters::blur((s, s), sk::TileMode::Mirror, None, sk::image_filters::CropRect::from(dev));
        for t in tints {
            let c = t.c4();
            let c8 = sk::Color::from_argb((c.a * 255.0).round() as u8, (c.r * 255.0).round() as u8, (c.g * 255.0).round() as u8, (c.b * 255.0).round() as u8);
            if let Some(cf) = sk::color_filters::blend(c8, sk::BlendMode::SrcOver) {
                f = f.and_then(|i| sk::image_filters::color_filter(cf, i, None));
            }
        }
        for c in fx {
            f = f.and_then(|i| sk::image_filters::color_filter(css_color_filter(*c), i, None));
        }
        if let Some(f) = f {
            let mut p = Paint::default();
            p.set_image_filter(f);
            cv.reset_matrix();
            cv.draw_image(base, (0.0, 0.0), Some(&p));
        }
        cv.restore();
    }

    /// The coverage of a (snapped) rounded box as white pixels: exactly the anti-aliased clip `backdrop` uses
    /// (same snapping, same clip), so a mask made from it covers the glass the way the drawing's backdrop is cut.
    pub fn clip_coverage(&self, x: f32, y: f32, w: f32, h: f32, r: f32) {
        let (x, y, w, h) = self.snap(x, y, w, h);
        let cv = self.cv();
        cv.save();
        cv.clip_rrect(rr(x, y, w, h, r), ClipOp::Intersect, true);
        cv.draw_paint(&paint(Rgba(1.0, 1.0, 1.0, 1.0)));
        cv.restore();
    }

    // ------------------------------------------------------------------ layers / transforms
    /// The current transform in DIPs (without the display scale).
    pub fn transform(&self) -> Matrix3x2 {
        let m = self.cv().local_to_device_as_3x3();
        let s = 1.0 / self.scale;
        Matrix3x2 { M11: m.scale_x() * s, M12: m.skew_y() * s, M21: m.skew_x() * s, M22: m.scale_y() * s, M31: m.translate_x() * s, M32: m.translate_y() * s }
    }
    pub fn set_transform(&self, t: &Matrix3x2) {
        let s = self.scale;
        let m = sk::Matrix::new_all(t.M11 * s, t.M21 * s, t.M31 * s, t.M12 * s, t.M22 * s, t.M32 * s, 0.0, 0.0, 1.0);
        self.cv().set_matrix(&sk::M44::from(m));
    }
    /// Group opacity (CSS `opacity` on an element: one layer, blended once), optionally clipped to a rounded rect.
    pub fn push_layer(&self, opacity: f32, clip: Option<(f32, f32, f32, f32, f32)>) {
        let cv = self.cv();
        self.layers.borrow_mut().push(cv.save_count());
        cv.save();
        if let Some((x, y, w, h, r)) = clip {
            cv.clip_rrect(rr(x, y, w, h, r), ClipOp::Intersect, true);
        }
        if opacity < 1.0 {
            // Blink records an element's opacity as a layer with an 8-bit alpha (.74 -> 189/255, Chromium's own log)
            cv.save_layer_alpha_f(None, (opacity.clamp(0.0, 1.0) * 255.0).round() / 255.0);
        }
    }
    pub fn pop_layer(&self) {
        // the clip and the layer (if any) go together
        if let Some(n) = self.layers.borrow_mut().pop() {
            self.cv().restore_to_count(n);
        }
    }
    /// A composited layer of its own (Chromium rasters it separately and blends it once), with its opacity.
    /// A composited layer of its own with its bounds (Chromium's layer box) - same pixels, far less work than a
    /// window-sized layer.
    pub fn push_isolated_in(&self, opacity: f32, bounds: Rect) {
        let cv = self.cv();
        self.layers.borrow_mut().push(cv.save_count());
        let mut p = Paint::default();
        p.set_alpha_f(opacity.clamp(0.0, 1.0));
        cv.save_layer(&sk::canvas::SaveLayerRec::default().bounds(&bounds).paint(&p));
    }
    pub fn push_isolated(&self, opacity: f32) {
        let cv = self.cv();
        self.layers.borrow_mut().push(cv.save_count());
        cv.save_layer_alpha_f(None, opacity.clamp(0.0, 1.0));
    }
    /// Clip to a rounded rect with its own corner radii [top-left, top-right, bottom-right, bottom-left].
    pub fn push_clip_rr4(&self, x: f32, y: f32, w: f32, h: f32, r: [f32; 4]) {
        let cv = self.cv();
        cv.save();
        let rad = [Point::new(r[0], r[0]), Point::new(r[1], r[1]), Point::new(r[2], r[2]), Point::new(r[3], r[3])];
        let rr = RRect::new_rect_radii(Rect::from_xywh(x, y, w, h), &rad);
        cv.clip_rrect(rr, ClipOp::Intersect, true);
    }
    pub fn push_clip(&self, x: f32, y: f32, w: f32, h: f32) {
        let cv = self.cv();
        cv.save();
        cv.clip_rect(Rect::from_xywh(x, y, w, h), ClipOp::Intersect, true);
    }
    pub fn pop_clip(&self) {
        self.cv().restore();
    }
    /// A layer with an image filter (CSS `filter`), e.g. blur or drop-shadow.
    /// An element's opacity as Blink records it: a layer with an 8-bit alpha and BOUNDS = the element's ink rect
    /// (see `ink`). Skia starts the layer's pixel grid at the bounds' corner, so the gradient dither inside depends on it.
    pub fn push_layer_in(&self, opacity: f32, bounds: Rect) {
        let cv = self.cv();
        self.layers.borrow_mut().push(cv.save_count());
        cv.save();
        if opacity < 1.0 {
            let mut p = Paint::default();
            p.set_alpha_f((opacity.clamp(0.0, 1.0) * 255.0).round() / 255.0);
            cv.save_layer(&sk::canvas::SaveLayerRec::default().bounds(&bounds).paint(&p));
        }
    }
    /// A CSS `filter` layer with Blink's bounds (the element's ink rect).
    pub fn push_filter_in(&self, f: sk::ImageFilter, bounds: Rect) {
        let mut p = Paint::default();
        p.set_image_filter(f);
        self.cv().save_layer(&sk::canvas::SaveLayerRec::default().bounds(&bounds).paint(&p));
    }
    /// Blink's ink rect of painted boxes: each box (x, y, w, h) grown by its outer shadows - a shadow reaches
    /// ceil(1.5 x blur) (= 3 sigma, rounded up) + spread beyond the box, moved by its offset - all united and rounded
    /// out to whole device pixels. (Fits every saveLayer bound in Chromium's own paint log of the drawing.)
    pub fn ink(&self, boxes: &[(f32, f32, f32, f32, &[Shadow])]) -> Rect {
        let (mut l, mut t, mut r, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (x, y, w, h, shs) in boxes {
            let mut add = |x0: f32, y0: f32, x1: f32, y1: f32| {
                l = l.min(x0);
                t = t.min(y0);
                r = r.max(x1);
                b = b.max(y1);
            };
            add(*x, *y, x + w, y + h);
            for s in shs.iter() {
                let e = (1.5 * s.blur).ceil() + s.spread;
                add(x + s.dx - e, y + s.dy - e, x + w + s.dx + e, y + h + s.dy + e);
            }
        }
        let k = self.scale;
        Rect::new((l * k).floor() / k, (t * k).floor() / k, (r * k).ceil() / k, (b * k).ceil() / k)
    }
    pub fn push_filter(&self, f: sk::ImageFilter) {
        let mut p = Paint::default();
        p.set_image_filter(f);
        self.cv().save_layer(&sk::canvas::SaveLayerRec::default().paint(&p));
    }
    pub fn pop_filter(&self) {
        self.cv().restore();
    }

    // ------------------------------------------------------------------ geometry
    pub fn rr_path(&self, x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
        Path::rrect(rr(x, y, w, h, r), None)
    }
    /// A path kept by name — for shapes drawn every frame.
    pub fn cached_geom(&self, key: &str, make: impl FnOnce() -> Path) -> Path {
        if let Some(g) = self.geoms.borrow().get(key) {
            return g.clone();
        }
        let g = make();
        self.geoms.borrow_mut().insert(key.to_string(), g.clone());
        g
    }
    /// A path for SVG path data (cached by its text), built like Blink (arcs become conics through SkPath::arcTo).
    pub fn path(&self, d: &str) -> Path {
        if let Some(g) = self.geoms.borrow().get(d) {
            return g.clone();
        }
        let g = crate::svg::to_path(&crate::svg::parse(d));
        self.geoms.borrow_mut().insert(d.to_string(), g.clone());
        g
    }
    /// SVG stroke (round caps and joins: the drawing's `stroke-linecap:round; stroke-linejoin:round`).
    pub fn stroke_geom(&self, g: &Path, w: f32, c: Rgba) {
        if self.off() {
            return;
        }
        if c.3 <= 0.0 {
            return;
        }
        let mut p = paint(c);
        p.set_style(PaintStyle::Stroke).set_stroke_width(w).set_stroke_cap(sk::paint::Cap::Round).set_stroke_join(sk::paint::Join::Round);
        self.cv().draw_path(g, &p);
    }
    /// `fold`: an element opacity Chromium folds into this single draw (8-bit colour, then x opacity as a float).
    pub fn stroke_oval(&self, r: Rect, w: f32, c: Rgba, fold: f32) {
        if self.off() {
            return;
        }
        if c.3 <= 0.0 || fold <= 0.0 {
            return;
        }
        let mut p = paint(c);
        p.set_alpha_f(p.alpha_f() * fold.min(1.0));
        p.set_style(PaintStyle::Stroke).set_stroke_width(w).set_stroke_cap(sk::paint::Cap::Round).set_stroke_join(sk::paint::Join::Round);
        self.cv().draw_oval(r, &p);
    }
    pub fn fill_geom(&self, g: &Path, c: Rgba) {
        if self.off() {
            return;
        }
        if c.3 <= 0.0 {
            return;
        }
        self.cv().draw_path(g, &paint(c));
    }
    /// An `<img>` scaled into a box (bilinear + mipmaps, like Blink's default image quality).
    pub fn draw_image_rect(&self, img: &sk::Image, x: f32, y: f32, w: f32, h: f32) {
        if self.off() {
            return;
        }
        let mut p = Paint::default();
        p.set_anti_alias(true);
        let so = sk::SamplingOptions::new(sk::FilterMode::Linear, sk::MipmapMode::Linear);
        self.cv().draw_image_rect_with_sampling_options(img, None, Rect::from_xywh(x, y, w, h), so, &p);
    }
    /// A stroke with explicit caps/joins and anti-aliasing (SVG `stroke-linecap`, `shape-rendering`).
    pub fn stroke_geom_ex(&self, g: &Path, w: f32, c: Rgba, round: bool, aa: bool, fold: f32) {
        if self.off() {
            return;
        }
        if c.3 <= 0.0 || fold <= 0.0 {
            return;
        }
        let mut p = paint(c);
        p.set_alpha_f(p.alpha_f() * fold.min(1.0));
        p.set_anti_alias(aa);
        p.set_style(PaintStyle::Stroke).set_stroke_width(w);
        if round {
            p.set_stroke_cap(sk::paint::Cap::Round).set_stroke_join(sk::paint::Join::Round);
        }
        self.cv().draw_path(g, &p);
    }
    /// A stroked rounded rect (centred on the outline).
    pub fn stroke_rr(&self, x: f32, y: f32, w: f32, h: f32, r: f32, lw: f32, c: Rgba) {
        if self.off() {
            return;
        }
        let mut p = paint(c);
        p.set_style(PaintStyle::Stroke).set_stroke_width(lw);
        self.cv().draw_rrect(rr(x, y, w, h, r), &p);
    }
    pub fn draw_image(&self, img: &sk::Image, x: f32, y: f32, alpha: f32) {
        if self.off() {
            return;
        }
        let mut p = Paint::new(Color4f::new(0.0, 0.0, 0.0, alpha), None);
        p.set_anti_alias(true);
        self.cv().draw_image(img, (x, y), Some(&p));
    }
}

/// A CSS filter function that is a colour matrix.
#[derive(Clone, Copy, Debug)]
pub enum CssColor {
    Saturate(f32),
    Brightness(f32),
    /// Order 025: `grayscale(a)` (the filter-effects matrix, as Chromium builds it)
    Grayscale(f32),
    /// Order 041 (adaptive glass, comp.rs `Adapt`): every channel at most / at least this level (0..1) - the compositor's
    /// D2D Blend darken / lighten with a flat grey, for the off-screen pictures
    Cap(f32),
    Floor(f32),
}

/// The colour matrix Chromium builds for a CSS filter function (cc/paint/render_surface_filters.cc), clamped like
/// SkColorFilters::Matrix.
pub fn css_color_filter(f: CssColor) -> sk::ColorFilter {
    let level = |l: f32, mode: sk::BlendMode| {
        let v = (l.clamp(0.0, 1.0) * 255.0).round() as u8;
        sk::color_filters::blend(sk::Color::from_rgb(v, v, v), mode).expect("blend filter")
    };
    let m = match f {
        CssColor::Cap(l) => return level(l, sk::BlendMode::Darken),
        CssColor::Floor(l) => return level(l, sk::BlendMode::Lighten),
        CssColor::Saturate(s) => saturate_matrix(s),
        CssColor::Brightness(b) => [b, 0.0, 0.0, 0.0, 0.0, 0.0, b, 0.0, 0.0, 0.0, 0.0, 0.0, b, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        CssColor::Grayscale(g) => {
            let a = 1.0 - g.clamp(0.0, 1.0);
            #[rustfmt::skip]
            let m = [
                0.2126 + 0.7874 * a, 0.7152 - 0.7152 * a, 0.0722 - 0.0722 * a, 0.0, 0.0,
                0.2126 - 0.2126 * a, 0.7152 + 0.2848 * a, 0.0722 - 0.0722 * a, 0.0, 0.0,
                0.2126 - 0.2126 * a, 0.7152 - 0.7152 * a, 0.0722 + 0.9278 * a, 0.0, 0.0,
                0.0, 0.0, 0.0, 1.0, 0.0,
            ];
            m
        }
    };
    sk::color_filters::matrix_row_major(&m, None)
}

/// CSS `saturate(s)` (Filter Effects: weights .213 / .715 / .072), row-major 4 x 5, the third weight of each row computed
/// as 1 - (the other two) the way Chromium's GetSaturateMatrix does.
pub fn saturate_matrix(s: f32) -> [f32; 20] {
    let m0 = 0.213 + 0.787 * s;
    let m1 = 0.715 - 0.715 * s;
    let m5 = 0.213 - 0.213 * s;
    let m6 = 0.715 + 0.285 * s;
    let m10 = 0.213 - 0.213 * s;
    let m11 = 0.715 - 0.715 * s;
    [
        m0, m1, 1.0 - (m0 + m1), 0.0, 0.0,
        m5, m6, 1.0 - (m5 + m6), 0.0, 0.0,
        m10, m11, 1.0 - (m10 + m11), 0.0, 0.0,
        0.0, 0.0, 0.0, 1.0, 0.0,
    ]
}
