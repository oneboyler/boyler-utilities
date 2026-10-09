//! Layout, paint and hit-testing of an `El` tree (Order 014).
//! - Layout: taffy (flexbox / grid / block with the CSS rules); text boxes measured with the painter's own shaping
//!   (the same widths the text is drawn with, LayoutUnits); positions kept in LayoutUnits (1/64 px) like Blink.
//! - Paint: Blink's order inside a box (outer shadows, background, inset shadows, border, content, children sorted by
//!   z-index), every operation through gfx.rs (pixel-snapped boxes, Chromium's shadows / gradients / text).
//! - Hit test: topmost box under a point (clip and pointer-events respected), with the chain of keyed boxes above it
//!   (CSS `:hover` / `:active` apply to an element and all its ancestors).

use std::cell::{Cell, RefCell};

use skia_safe as sk;
use taffy::prelude::*;
use taffy::{compute_leaf_layout, LayoutInput, LayoutOutput};
use windows_numerics::Matrix3x2;

use super::el::{Content, Cursor, El, Fill, Key, Text, Wrap, RADIUS_PILL};
use crate::gfx::{Align, Gfx, Rgba};
use crate::icons::Icons;

/// One laid-out box.
pub struct Node {
    pub el: El,
    pub kids: Vec<usize>,
    pub parent: Option<usize>,
    /// border box in window DIPs (x, y, w, h)
    pub rect: (f32, f32, f32, f32),
    /// content box (inside border + padding): where text and icons go
    pub content: (f32, f32, f32, f32),
    tid: NodeId,
}

/// A laid-out tree: boxes in tree order (index 0 = the root).
pub struct Laid {
    pub nodes: Vec<Node>,
    /// the content's full height (scrolling pages)
    pub height: f32,
    /// Order 055: where the live pass painted at its last paint (the union of the live boxes with their outer shadows, in
    /// the tree's own coordinates) - the frame redraws only that area for moving meters
    pub live_ink: Cell<Option<(f32, f32, f32, f32)>>,
}

fn lu(v: f32) -> f32 {
    (v * 64.0).round() / 64.0
}

/// Lines of a wrapped text for a width (Blink breaks at spaces; a word wider than the line stays whole).
pub fn wrap_lines(g: &Gfx, t: &Text, maxw: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in t.s.split('\n') {
        let mut cur = String::new();
        for w in para.split(' ') {
            let cand = if cur.is_empty() { w.to_string() } else { format!("{} {}", cur, w) };
            if !cur.is_empty() && g.text_width(&cand, t.font) > maxw + 0.001 {
                lines.push(std::mem::take(&mut cur));
                cur = w.to_string();
            } else {
                cur = cand;
            }
        }
        lines.push(cur);
    }
    lines
}

fn measure_text(g: &Gfx, t: &Text, known: Size<Option<f32>>, avail: Size<AvailableSpace>) -> Size<f32> {
    let natural = g.text_width(&t.s.replace('\n', " "), t.font);
    let w = match (known.width, t.wrap) {
        (Some(w), _) => w,
        (None, Wrap::Wrap) => match avail.width {
            AvailableSpace::Definite(a) => natural.min(a),
            AvailableSpace::MinContent => t.s.split([' ', '\n']).map(|w| g.text_width(w, t.font)).fold(0.0, f32::max),
            AvailableSpace::MaxContent => natural,
        },
        (None, _) => natural,
    };
    let h = match known.height {
        Some(h) => h,
        None => {
            let n = if t.wrap == Wrap::Wrap { wrap_lines(g, t, w).len() } else { 1 };
            t.lh * n as f32
        }
    };
    Size { width: w, height: h }
}

impl Laid {
    /// Lay `root` out in a box `w` wide (and `h` high, or as high as its content when None).
    pub fn new(g: &Gfx, root: El, w: f32, h: Option<f32>) -> Laid {
        let mut tree: TaffyTree<usize> = TaffyTree::new();
        tree.disable_rounding();
        let mut nodes: Vec<Node> = Vec::new();
        fn add(tree: &mut TaffyTree<usize>, nodes: &mut Vec<Node>, mut el: El, parent: Option<usize>) -> usize {
            let kids_el = std::mem::take(&mut el.children);
            let i = nodes.len();
            let tid = tree.new_leaf_with_context(el.style.clone(), i).expect("taffy node");
            nodes.push(Node { el, kids: Vec::new(), parent, rect: (0.0, 0.0, 0.0, 0.0), content: (0.0, 0.0, 0.0, 0.0), tid });
            for k in kids_el {
                let c = add(tree, nodes, k, Some(i));
                nodes[i].kids.push(c);
                let ct = nodes[c].tid;
                tree.add_child(tid, ct).expect("taffy child");
            }
            i
        }
        add(&mut tree, &mut nodes, root, None);
        let avail = Size { width: AvailableSpace::Definite(w), height: h.map(AvailableSpace::Definite).unwrap_or(AvailableSpace::MaxContent) };
        {
            let ns = &nodes;
            let _ = tree.compute_layout_with_measure(nodes[0].tid, avail, |inputs: LayoutInput, _id, ctx: Option<&mut usize>, style: &Style| -> LayoutOutput {
                let Some(&mut i) = ctx else { return compute_leaf_layout(inputs, style, |_, _| 0.0, |_, _| Size::ZERO) };
                match &ns[i].el.content {
                    Content::Text(t) => compute_leaf_layout(inputs, style, |_, _| 0.0, |known, av| measure_text(g, t, known, av)),
                    _ => compute_leaf_layout(inputs, style, |_, _| 0.0, |_, _| Size::ZERO),
                }
            });
        }
        // absolute positions, in LayoutUnits
        fn place(tree: &TaffyTree<usize>, nodes: &mut Vec<Node>, i: usize, ox: f32, oy: f32) {
            let l = tree.layout(nodes[i].tid).expect("layout");
            let (x, y) = (lu(ox + l.location.x), lu(oy + l.location.y));
            nodes[i].rect = (x, y, lu(l.size.width), lu(l.size.height));
            let (pl, pt) = (l.padding.left + l.border.left, l.padding.top + l.border.top);
            let (pr, pb) = (l.padding.right + l.border.right, l.padding.bottom + l.border.bottom);
            nodes[i].content = (lu(x + pl), lu(y + pt), lu(l.size.width - pl - pr), lu(l.size.height - pt - pb));
            for k in nodes[i].kids.clone() {
                place(tree, nodes, k, x, y);
            }
        }
        place(&tree, &mut nodes, 0, 0.0, 0.0);
        let root_l = tree.layout(nodes[0].tid).expect("layout");
        let height = root_l.size.height;
        Laid { nodes, height, live_ink: Cell::new(None) }
    }

    /// Children of `i` in paint order (z-index, then tree order).
    fn order(&self, i: usize) -> Vec<usize> {
        let mut k = self.nodes[i].kids.clone();
        k.sort_by_key(|&c| self.nodes[c].el.z);
        k
    }

    /// The box of the first element with this key.
    pub fn rect_of(&self, k: Key) -> Option<(f32, f32, f32, f32)> {
        let i = self.nodes.iter().position(|n| n.el.key == Some(k))?;
        let (x, y, w, h) = self.nodes[i].rect;
        // Order 025: moved by its own and its ancestors' `translate_pct` (as painted and hit) - 0 for every other element
        let (mut dx, mut dy) = (0.0, 0.0);
        let mut c = Some(i);
        while let Some(j) = c {
            let n = &self.nodes[j];
            dx += n.el.translate_pct.0 * n.rect.2;
            dy += n.el.translate_pct.1 * n.rect.3;
            c = n.parent;
        }
        Some((x + dx, y + dy, w, h))
    }

    /// Order 025 (test proofs): every box with what it draws (`text:…` / `icon:…` / `box`), for comparing with Chromium's
    /// boxes (tools/ref/gallery_w.js --dump). Test builds only.
    #[cfg(test)]
    pub fn debug_boxes(&self) -> Vec<(f32, f32, f32, f32, String)> {
        self.nodes
            .iter()
            .map(|n| {
                let what = match &n.el.content {
                    Content::Text(t) => format!("text:{}", t.s),
                    Content::Icon(i) => format!("icon:{}", if i.name.starts_with('<') { "svg" } else { &i.name }),
                    _ => "box".to_string(),
                };
                (n.rect.0, n.rect.1, n.rect.2, n.rect.3, what)
            })
            .collect()
    }

    /// Order 025: the innermost element of a hover chain (`State::hover`, innermost first) that carries a tip - the
    /// drawing's `e.target.closest('[data-tip]')`: its key, text and box.
    /// Order 045: + whether it is a plain `title` hover name (`El::title`, the longer delay).
    pub fn tip_in(&self, hover: &[Key]) -> Option<(Key, std::rc::Rc<str>, (f32, f32, f32, f32), bool)> {
        hover.iter().find_map(|&k| self.nodes.iter().find(|n| n.el.key == Some(k) && n.el.tip.is_some()).map(|n| (k, n.el.tip.clone().unwrap_or_else(|| "".into()), n.rect, n.el.tip_title)))
    }

    /// Order 025: the slim scrollbar thumb of node i (`El::slim_thumb`): its one child = the content, moved up by the
    /// offset. Blink's geometry (ScrollbarTheme): track = the box's height (no buttons), thumb length = round(track x
    /// visible / total) (at least `min_len`), position = offset x (track - thumb) / (total - visible) truncated (a part
    /// pixel moved shows as 1); the lane sits at the right edge, the thumb inset by its transparent border.
    fn paint_slim_thumb(&self, g: &Gfx, i: usize, st: &super::el::SlimThumb) {
        let n = &self.nodes[i];
        let Some(&c) = n.kids.first() else { return };
        let (x, y, w, h) = n.rect;
        let (total, off) = (self.nodes[c].rect.3, -self.nodes[c].el.translate.1);
        if total <= h + 0.01 {
            return;
        }
        let track = h.round();
        let len = (track * h / total).round().max(st.min_len);
        if len > track {
            return;
        }
        let p = off.max(0.0) * (track - len) / (total - h);
        let pos = if p > 0.0 && p < 1.0 { 1.0 } else { p.trunc() };
        let b = st.border;
        g.fill_rr(x + w - st.lane + b, y + pos + b, st.lane - 2.0 * b, len - 2.0 * b, (st.radius - b).max(0.0), st.color);
    }

    /// Order 025: the image filter of node i's CSS `filter` colour functions, applied in order (`El::color_filter`).
    /// None = no filter could be made (then no layer: a paint never panics).
    fn color_filter_of(&self, i: usize) -> Option<sk::ImageFilter> {
        let e = &self.nodes[i].el;
        // Lane K's `El::brightness` (when also set) applies first
        let mut f: Option<sk::ImageFilter> = None;
        if (e.brightness - 1.0).abs() > 1e-4 {
            f = sk::image_filters::color_filter(crate::gfx::css_color_filter(crate::gfx::CssColor::Brightness(e.brightness)), None, None);
        }
        for c in &e.color_filter {
            f = sk::image_filters::color_filter(crate::gfx::css_color_filter(*c), f, None);
        }
        f
    }

    /// Order 025: Blink's filter-layer bounds for node i: its box and every descendant's (moved by its own translate), each
    /// grown by its outer shadows. Not counted: a child's scale / rotate and glyph ink past a text box - a child scaled above
    /// 1 or overhanging glyphs would be cut at these bounds (no current user has either).
    fn subtree_ink(&self, g: &Gfx, i: usize) -> sk::Rect {
        let mut boxes = Vec::new();
        let mut stack = vec![(i, 0.0f32, 0.0f32)];
        while let Some((k, ox, oy)) = stack.pop() {
            let n = &self.nodes[k];
            let (tx, ty) = if k == i { (0.0, 0.0) } else { (ox + n.el.translate.0 + n.el.translate_pct.0 * n.rect.2, oy + n.el.translate.1 + n.el.translate_pct.1 * n.rect.3) };
            boxes.push((n.rect.0 + tx, n.rect.1 + ty, n.rect.2, n.rect.3, &n.el.shadows[..]));
            stack.extend(n.kids.iter().map(|&c| (c, tx, ty)));
        }
        g.ink(&boxes)
    }

    /// The innermost scrolling box (`Cx::scroll_box`) among `chain` (a hit's keys, innermost first): its key and how far
    /// its content can scroll (content height - box height).
    pub fn scroll_box_in(&self, chain: &[Key]) -> Option<(Key, f32)> {
        for k in chain {
            if let Some(i) = self.nodes.iter().position(|n| n.el.key == Some(*k) && n.el.scrolls) {
                let n = &self.nodes[i];
                let inner = n.kids.first().map(|&c| self.nodes[c].rect.3).unwrap_or(0.0);
                return Some((*k, (inner - n.rect.3).max(0.0)));
            }
        }
        None
    }

    /// Is any box painted in the live pass (`El::live`)?
    pub fn has_live(&self) -> bool {
        self.nodes.iter().any(|n| n.el.live)
    }

    /// Does any box stick (`El::sticky`)?
    pub fn has_sticky(&self) -> bool {
        self.nodes.iter().any(|n| n.el.sticky.is_some())
    }

    /// CSS `position:sticky` for a page scrolled by `scroll`: each sticky box moves down to `scroll + top`, but never above
    /// its own place nor out of its parent's box (it and everything inside it move together).
    pub fn apply_sticky(&mut self, scroll: f32) {
        for i in 0..self.nodes.len() {
            let Some(top) = self.nodes[i].el.sticky else { continue };
            let (_, y, _, h) = self.nodes[i].rect;
            let (py, ph) = self.nodes[i].parent.map(|p| (self.nodes[p].rect.1, self.nodes[p].rect.3)).unwrap_or((y, h));
            let want = (scroll + top).min(py + ph - h).max(y);
            let dy = want - y;
            if dy.abs() < 1e-4 {
                continue;
            }
            let mut stack = vec![i];
            while let Some(j) = stack.pop() {
                self.nodes[j].rect.1 += dy;
                self.nodes[j].content.1 += dy;
                stack.extend(self.nodes[j].kids.iter().copied());
            }
        }
    }

    // ------------------------------------------------------------------ paint
    /// Paint the whole tree, moved by (dx, dy). `base` = what is behind (for backdrop filters), device pixels.
    pub fn paint(&self, g: &Gfx, icons: &Icons, dx: f32, dy: f32, base: Option<&sk::Image>) {
        if self.nodes.is_empty() {
            return;
        }
        let t0 = g.transform();
        g.set_transform(&(Matrix3x2::translation(dx, dy) * t0));
        let lives = RefCell::new(Vec::new());
        self.paint_node(g, icons, 0, base, &[], &lives, (0.0, 0.0));
        g.set_transform(&t0);
        let u = lives.into_inner().into_iter().reduce(|a, b| {
            let (l, t) = (a.0.min(b.0), a.1.min(b.1));
            (l, t, (a.0 + a.2).max(b.0 + b.2) - l, (a.1 + a.3).max(b.1 + b.3) - t)
        });
        self.live_ink.set(u);
    }

    /// Order 042: the frame composites the live layer OVER the static page layer, so a box painted after a live box
    /// (`El::live`) that overlaps it would end up under it (the owner's test 2: the Audio slider's knob under its level pill,
    /// the Controller's hover label under the live picture). Chromium's overlap rule: such a box is painted in the live
    /// pass too, and then counts as live itself. `lives` = the boxes already painted live (with their outer shadows).
    fn over_live(lives: &RefCell<Vec<(f32, f32, f32, f32)>>, r: (f32, f32, f32, f32)) -> bool {
        lives.borrow().iter().any(|l| r.0 < l.0 + l.2 && l.0 < r.0 + r.2 && r.1 < l.1 + l.3 && l.1 < r.1 + r.3)
    }

    /// A box grown by its outer shadows (ceil(1.5 blur) + spread, moved by the offset).
    fn with_shadows(r: (f32, f32, f32, f32), shadows: &[crate::gfx::Shadow]) -> (f32, f32, f32, f32) {
        let (mut x0, mut y0, mut x1, mut y1) = (r.0, r.1, r.0 + r.2, r.1 + r.3);
        for s in shadows {
            let e = (1.5 * s.blur).ceil() + s.spread.max(0.0);
            x0 = x0.min(r.0 + s.dx - e);
            y0 = y0.min(r.1 + s.dy - e);
            x1 = x1.max(r.0 + r.2 + s.dx + e);
            y1 = y1.max(r.1 + r.3 + s.dy + e);
        }
        (x0, y0, x1 - x0, y1 - y0)
    }

    /// `tints` = the flat background colours of the ancestors painted in this pass (over `base`, behind this box).
    fn paint_node(&self, g: &Gfx, icons: &Icons, i: usize, base: Option<&sk::Image>, tints: &[Rgba], lives: &RefCell<Vec<(f32, f32, f32, f32)>>, off: (f32, f32)) {
        let n = &self.nodes[i];
        let e = &n.el;
        if e.opacity <= 0.0 || matches!(e.style.display, Display::None) {
            return;
        }
        let (x, y, w, h) = n.rect;
        let r = if e.radius >= RADIUS_PILL { w.min(h) / 2.0 } else { e.radius.min(w / 2.0).min(h / 2.0) };
        let t0 = g.transform();
        let moved = e.translate != (0.0, 0.0) || e.translate_pct != (0.0, 0.0) || e.scale != 1.0 || e.rotate != 0.0;
        if moved {
            // CSS transform with the default origin (the box centre): translate, scale, rotate
            let (cx, cy) = (x + w / 2.0, y + h / 2.0);
            let (s, c) = e.rotate.to_radians().sin_cos();
            let m = Matrix3x2::translation(-cx, -cy)
                * Matrix3x2 { M11: c * e.scale, M12: s * e.scale, M21: -s * e.scale, M22: c * e.scale, M31: 0.0, M32: 0.0 }
                * Matrix3x2::translation(cx + e.translate.0 + e.translate_pct.0 * w, cy + e.translate.1 + e.translate_pct.1 * h);
            g.set_transform(&(m * t0));
        }
        let layer = e.opacity < 1.0;
        if layer {
            let ink = g.ink(&[(x, y, w, h, &e.shadows)]);
            g.push_layer_in(e.opacity, ink);
        }
        // Order 025: CSS `filter` colour functions - the box and its subtree into one layer (bounds = their ink), coloured
        // (Lane K's `brightness` joins that list when both are set)
        let filter = if e.color_filter.is_empty() { None } else { self.color_filter_of(i) };
        let filtered_w = filter.is_some();
        if let Some(f) = filter {
            g.push_filter_in(f, self.subtree_ink(g, i));
        }
        // CSS applies `filter` first, then `opacity`: the filter layer sits inside the opacity layer
        let filtered = !filtered_w && (e.brightness - 1.0).abs() > 1e-4;
        if filtered {
            let ink = g.ink(&[(x, y, w, h, &e.shadows)]);
            g.push_layer_in(1.0, ink);
            if let Some(f) = sk::image_filters::color_filter(crate::gfx::css_color_filter(crate::gfx::CssColor::Brightness(e.brightness)), None, None) {
                g.push_filter_in(f, ink);
            }
        }
        let own = || {
            if e.backdrop > 0.0 {
                if let Some(b) = base {
                    g.backdrop_tinted(b, x, y, w, h, r, e.backdrop, tints, &[crate::gfx::CssColor::Saturate(e.backdrop_sat)]);
                }
            }
            let opaque = matches!(e.bg, Some(Fill::Color(c)) if c.3 >= 1.0);
            if !e.shadows.is_empty() {
                g.box_shadows(x, y, w, h, r, &e.shadows, opaque);
            }
            match &e.bg {
                Some(Fill::Color(c)) => g.fill_rr(x, y, w, h, r, *c),
                Some(Fill::Linear(a, stops)) => {
                    let (sx, sy, sw, sh) = g.snap(x, y, w, h);
                    let rad = a.to_radians();
                    let (dx, dy) = (rad.sin(), -rad.cos());
                    let len = (sw * dx).abs() + (sh * dy).abs();
                    let (cx, cy) = (sx + sw / 2.0, sy + sh / 2.0);
                    let sh_ = g.hgrad(cx - dx * len / 2.0, cy - dy * len / 2.0, cx + dx * len / 2.0, cy + dy * len / 2.0, stops);
                    g.fill_rr_shader(x, y, w, h, r, &sh_, 1.0);
                }
                None => {}
            }
            if !e.insets.is_empty() {
                g.inset_shadows(x, y, w, h, r, &e.insets);
            }
            if let Some((bw, c, top_only)) = e.border {
                if top_only {
                    // a top border only (a hairline above a footer)
                    g.fill_rect(x, y, w, bw, c);
                } else {
                    let (sx, sy, sw, sh) = g.snap(x, y, w, h);
                    g.stroke_rr(sx + bw / 2.0, sy + bw / 2.0, sw - bw, sh - bw, (r - bw / 2.0).max(0.0), bw, c);
                }
            }
            match &e.content {
                Content::Text(t) => paint_text(g, t, n.content),
                Content::Icon(ic) if ic.fit.is_some() => paint_icon_fit(g, icons, ic, n.content),
                Content::Icon(ic) if ic.paint.is_some() => paint_icon_styled(g, icons, ic, n.content),
                Content::Icon(ic) => {
                    let ops = ic.class_op.clone();
                    let (cx_, cy_) = (n.content.0, n.content.1);
                    icons.draw(g, &ic.name, cx_, cy_, ic.size, ic.stroke_w, ic.color, &|c: &str| ops.iter().find(|o| o.0 == c).map(|o| o.1).unwrap_or(1.0));
                }
                Content::Paint(f) => f(g, n.rect),
                Content::None => {}
            }
        };
        // the overlap rule works in page coordinates: the layout box moved by its own translate and every ancestor's (`off`:
        // a scroll box's content is moved that way); scale / rotate are left out (no page scales a box over a live one)
        let moved_by = (off.0 + e.translate.0 + e.translate_pct.0 * w, off.1 + e.translate.1 + e.translate_pct.1 * h);
        let ink = Self::with_shadows((x + moved_by.0, y + moved_by.1, w, h), &e.shadows);
        if e.live || Self::over_live(lives, ink) {
            g.live(own);
            // only a box that paints something covers what comes after it (a bare layout row or hit box does not)
            let paints = e.bg.is_some() || !e.shadows.is_empty() || !e.insets.is_empty() || e.border.is_some() || e.backdrop > 0.0 || !matches!(e.content, Content::None);
            if e.live || paints {
                lives.borrow_mut().push(ink);
            }
        } else {
            own();
        }
        if !n.kids.is_empty() {
            if e.clip {
                g.push_clip_rr4(x, y, w, h, [r; 4]);
            }
            let mut kt = tints.to_vec();
            if let Some(Fill::Color(c)) = e.bg {
                kt.push(c);
            }
            for c in self.order(i) {
                self.paint_node(g, icons, c, base, &kt, lives, moved_by);
            }
            if e.clip {
                g.pop_clip();
            }
        }
        if let Some(st) = e.slim_thumb {
            // the thumb sits over the box's content: live when any of it is
            if Self::over_live(lives, (x + moved_by.0, y + moved_by.1, w, h)) {
                g.live(|| self.paint_slim_thumb(g, i, &st));
            } else {
                self.paint_slim_thumb(g, i, &st);
            }
        }
        if filtered_w {
            g.pop_filter();
        }
        if filtered {
            g.pop_layer();
        }
        if layer {
            g.pop_layer();
        }
        if moved {
            g.set_transform(&t0);
        }
    }

    // ------------------------------------------------------------------ hit test
    /// The topmost box under (x, y): its index, then the keys of it and its ancestors (innermost first).
    pub fn hit(&self, x: f32, y: f32) -> Option<(usize, Vec<Key>)> {
        if self.nodes.is_empty() {
            return None;
        }
        let i = self.hit_node(0, x, y)?;
        let mut keys = Vec::new();
        let mut c = Some(i);
        while let Some(k) = c {
            if let Some(key) = self.nodes[k].el.key {
                keys.push(key);
            }
            c = self.nodes[k].parent;
        }
        Some((i, keys))
    }

    fn hit_node(&self, i: usize, x: f32, y: f32) -> Option<usize> {
        let n = &self.nodes[i];
        if matches!(n.el.style.display, Display::None) || n.el.opacity <= 0.0 {
            return None;
        }
        let (rx, ry, rw, rh) = n.rect;
        let (x, y) = (x - n.el.translate.0 - n.el.translate_pct.0 * rw, y - n.el.translate.1 - n.el.translate_pct.1 * rh);
        let inside = x >= rx && x < rx + rw && y >= ry && y < ry + rh;
        if n.el.clip && !inside {
            return None;
        }
        for c in self.order(i).into_iter().rev() {
            if let Some(h) = self.hit_node(c, x, y) {
                return Some(h);
            }
        }
        if inside && n.el.hit {
            Some(i)
        } else {
            None
        }
    }

    /// Order 045: an element keyed only so its tip shows (`El::tip` / `El::title`, nothing to press: not clickable, no
    /// slider, no wheel, no cursor of its own) - a press inside it belongs to the control around it.
    pub fn tip_only(&self, k: Key) -> bool {
        self.nodes.iter().find(|n| n.el.key == Some(k)).is_some_and(|n| {
            let e = &n.el;
            e.tip.is_some() && !e.click && e.range.is_none() && !e.wheel && e.cursor == Cursor::Default && !e.scrolls
        })
    }

    /// Order 045: the innermost element of a hover chain that steps with the wheel (`El::wheel_steps`).
    pub fn wheel_in(&self, chain: &[Key]) -> Option<Key> {
        chain.iter().copied().find(|&k| self.nodes.iter().any(|n| n.el.key == Some(k) && n.el.wheel))
    }

    /// Order 045: the keyboard's tab order - every element Tab can reach (a clickable one: button, switch, row, link,
    /// segment; a slider `El::range`; a text field `Cursor::Text`), in tree (= document) order; hidden subtrees
    /// (`display:none`, opacity 0) skipped.
    pub fn focusables(&self) -> Vec<Key> {
        let mut out = Vec::new();
        if !self.nodes.is_empty() {
            self.collect_focusables(0, &mut out);
        }
        out
    }

    fn collect_focusables(&self, i: usize, out: &mut Vec<Key>) {
        let n = &self.nodes[i];
        if matches!(n.el.style.display, Display::None) || n.el.opacity <= 0.0 {
            return;
        }
        if let Some(k) = n.el.key {
            let control = n.el.click || n.el.range.is_some() || n.el.cursor == Cursor::Text;
            if n.el.hit && control && n.rect.2 > 0.0 && n.rect.3 > 0.0 && !out.contains(&k) {
                out.push(k);
            }
        }
        for &c in &n.kids {
            self.collect_focusables(c, out);
        }
    }

    /// Order 045: what the keyboard focus needs of a focused element: its radius, slider value (`El::range`), whether a
    /// click is its action (Enter / Space press it) and whether it is a text field (`Cursor::Text`: keys type into it).
    pub fn focus_info(&self, k: Key) -> Option<(f32, Option<(f32, f32)>, bool, bool)> {
        let n = self.nodes.iter().find(|n| n.el.key == Some(k))?;
        Some((n.el.radius, n.el.range, n.el.click, n.el.cursor == Cursor::Text))
    }

    /// The cursor of the box under a point (nearest ancestor that sets one).
    pub fn cursor_at(&self, i: usize) -> Cursor {
        let mut c = Some(i);
        while let Some(k) = c {
            if self.nodes[k].el.cursor != Cursor::Default {
                return self.nodes[k].el.cursor;
            }
            c = self.nodes[k].parent;
        }
        Cursor::Default
    }

    /// The nearest clickable box at or above `i`.
    pub fn clickable(&self, i: usize) -> Option<Key> {
        let mut c = Some(i);
        while let Some(k) = c {
            if self.nodes[k].el.click {
                return self.nodes[k].el.key;
            }
            c = self.nodes[k].parent;
        }
        None
    }
}

/// Text in its laid-out box: one line (clipped with an ellipsis when asked) or wrapped lines.
pub fn paint_text(g: &Gfx, t: &Text, (x, y, w, _h): (f32, f32, f32, f32)) {
    // centred text (a button's label): Blink centres the line's own width (not rounded to LayoutUnits) in the box
    let ax = |tw: f32| match t.align {
        Align::Left => x,
        Align::Center => x + (w - g.text_box(&t.s, t.font, 0.0).raw.min(tw)) / 2.0,
        Align::Right => x + w - tw,
    };
    match t.wrap {
        Wrap::Wrap => {
            for (k, line) in wrap_lines(g, t, w).iter().enumerate() {
                let tw = g.text_width(line, t.font);
                g.text(line, t.font, ax(tw), y + t.lh * k as f32, t.lh, t.color, Align::Left, 0.0);
            }
        }
        Wrap::Ellipsis => {
            let tw = g.text_box(&t.s, t.font, w).width.min(w);
            g.text_clipped(&t.s, t.font, ax(tw), y, t.lh, t.color, w, true);
        }
        Wrap::Line => {
            let tw = g.text_width(&t.s, t.font);
            g.text(&t.s, t.font, ax(tw), y, t.lh, t.color, Align::Left, 0.0);
            if t.underline {
                // CSS text-decoration: underline with text-underline-offset 2px: a 1 px line under the baseline
                let base = y + (t.lh + t.font.size()) / 2.0;
                g.fill_rect(ax(tw), base + 2.0, tw, 1.0, t.color);
            }
        }
    }
}

/// Order 025: `El::icon_fit` - the `<svg>` box (w x h) is pixel-snapped like Blink snaps a replaced element; inside it the
/// viewBox is scaled to fit (`xMidYMid meet`) and centred with its sub-pixel offset kept (part of the SVG's own transform).
fn paint_icon_fit(g: &Gfx, icons: &Icons, ic: &super::el::Icon, (x, y, _, _): (f32, f32, f32, f32)) {
    let Some((w, h)) = ic.fit else { return };
    let (vb, vbh) = icons.view_box(&ic.name);
    let k = (w / vb).min(h / vbh);
    let (sx, sy, _, _) = g.snap(x, y, w, h);
    // the <svg> element's overflow clip is its own box (w x h), not the viewBox
    g.push_clip(sx, sy, w, h);
    let t0 = g.transform();
    g.set_transform(&(Matrix3x2::translation((w - vb * k) / 2.0, (h - vbh * k) / 2.0) * t0));
    let ops = ic.class_op.clone();
    icons.draw_ex(g, &ic.name, sx, sy, vb * k, ic.stroke_w, ic.color, &|c: &str| ops.iter().find(|o| o.0 == c).map(|o| o.1).unwrap_or(1.0), false);
    g.set_transform(&t0);
    g.pop_clip();
}

/// Order 025: an icon whose parts are filled / coloured per class (`El::icon_paint`), drawn at its snapped box.
fn paint_icon_styled(g: &Gfx, icons: &Icons, ic: &super::el::Icon, (x, y, _, _): (f32, f32, f32, f32)) {
    let Some(p) = &ic.paint else { return };
    let ops = ic.class_op.clone();
    icons.draw_styled(g, &ic.name, x, y, ic.size, ic.stroke_w, ic.color, p, &|c: &str| ops.iter().find(|o| o.0 == c).map(|o| o.1).unwrap_or(1.0));
}

/// Order 042 proof pictures (tests only): `root` (`w` x `h`) painted the way the frame composites a page - the static
/// pass, then the live pass over it - on a flat dark backdrop at `scale`, saved as the PNG `name` in the folder
/// `BU_PIC_OUT` names (nothing without it).
#[cfg(test)]
pub fn proof_png(root: El, w: f32, h: f32, scale: f32, name: &str) {
    let Ok(dir) = std::env::var("BU_PIC_OUT") else { return };
    let path = std::path::Path::new(&dir).join(name).to_string_lossy().into_owned();
    let path = path.as_str();
    use crate::gfx::Pass;
    let g = Gfx::new(scale);
    let icons = Icons::new();
    let l = Laid::new(&g, root, w, Some(h));
    let (pw, ph) = ((w * scale).round() as i32, (h * scale).round() as i32);
    let mut out = crate::gfx::new_surface(pw, ph).expect("surface");
    // (Order 045: a light backdrop for the light theme's pictures)
    out.canvas().clear(if crate::ui::is_light() { sk::Color::from_argb(255, 0xe6, 0xe8, 0xee) } else { sk::Color::from_argb(255, 0x1c, 0x1e, 0x25) });
    for pass in [Pass::Static, Pass::Live] {
        let mut s = crate::gfx::new_surface(pw, ph).expect("surface");
        s.canvas().clear(sk::Color::TRANSPARENT);
        g.begin(s.canvas());
        g.set_pass(pass);
        l.paint(&g, &icons, 0.0, 0.0, None);
        g.set_pass(Pass::All);
        g.end();
        let img = s.image_snapshot();
        out.canvas().draw_image(&img, (0, 0), None);
    }
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
    }
    if let Some(d) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(d);
    }
    crate::png::save_png(&crate::png::from_surface(&mut out), path).expect("png saved");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::cx::{Cx, State};
    use crate::ui::el::key;

    /// `position:sticky; top:10px` inside a 600 px tall parent: it stays at its place until the page passes it, then rides
    /// 10 px below the visible top, and stops at the parent's bottom.
    #[test]
    fn sticky_follows_the_scroll_inside_its_parent() {
        let g = Gfx::new(1.0);
        let make = || {
            let root = El::block().w(300.0).child(El::block().h(100.0)).child(
                El::col().h(600.0).child(El::block().h(40.0).key(key("bar")).sticky(10.0)).child(El::block().h(500.0)),
            );
            Laid::new(&g, root, 300.0, None)
        };
        for (scroll, want) in [(0.0f32, 100.0f32), (95.0, 105.0), (300.0, 310.0), (2000.0, 660.0)] {
            let mut l = make();
            assert!(l.has_sticky());
            l.apply_sticky(scroll);
            assert_eq!(l.rect_of(key("bar")).unwrap().1, want, "scroll {scroll}");
        }
    }

    /// Order 042 (the owner's test 2: the slider's knob under its live level pill): a box painted after a live box that
    /// overlaps it is painted in the live pass (on top), not in the static one; a box beside it stays static.
    #[test]
    fn a_box_over_a_live_box_paints_in_the_live_pass() {
        let g = Gfx::new(1.0);
        let icons = crate::icons::Icons::new();
        let red = Rgba(1.0, 0.0, 0.0, 1.0);
        let blue = Rgba(0.0, 0.0, 1.0, 1.0);
        let root = El::block()
            .size(100.0, 40.0)
            .child(El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(60.0, 20.0).bg(red).live())
            .child(El::block().abs(20.0, 0.0, f32::NAN, f32::NAN).size(20.0, 20.0).bg(blue))
            .child(El::block().abs(70.0, 20.0, f32::NAN, f32::NAN).size(20.0, 20.0).bg(blue));
        let l = Laid::new(&g, root, 100.0, Some(40.0));
        let shot = |pass| {
            let mut s = crate::gfx::new_surface(100, 40).unwrap();
            s.canvas().clear(sk::Color::TRANSPARENT);
            g.begin(s.canvas());
            g.set_pass(pass);
            l.paint(&g, &icons, 0.0, 0.0, None);
            g.set_pass(crate::gfx::Pass::All);
            g.end();
            let mut px = |x: i32, y: i32| {
                let mut b = [0u8; 4];
                let ii = sk::ImageInfo::new((1, 1), sk::ColorType::BGRA8888, sk::AlphaType::Premul, None);
                assert!(s.read_pixels(&ii, &mut b, 4, (x, y)));
                b
            };
            (px(30, 10), px(80, 30))
        };
        let (over, beside) = shot(crate::gfx::Pass::Static);
        assert_eq!(over[3], 0, "the static pass leaves the overlapping box out");
        assert_eq!(beside, [255, 0, 0, 255], "a box beside the live one stays static (BGRA blue)");
        let (over, beside) = shot(crate::gfx::Pass::Live);
        assert_eq!(over, [255, 0, 0, 255], "the live pass paints it over the live box (BGRA blue)");
        assert_eq!(beside[3], 0);
    }

    /// Order 042 review: the overlap rule compares boxes where they are painted - a live box inside a moved parent (a
    /// scrolled scroll box's content) still covers the box after it; a bare layout box over it doesn't widen the live area.
    #[test]
    fn the_overlap_rule_follows_a_moved_parent() {
        let g = Gfx::new(1.0);
        let icons = crate::icons::Icons::new();
        let red = Rgba(1.0, 0.0, 0.0, 1.0);
        let blue = Rgba(0.0, 0.0, 1.0, 1.0);
        let root = El::block()
            .size(100.0, 40.0)
            // the live box sits at y 0..20 in its parent, painted at y 20..40
            .child(El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(100.0, 20.0).translate(0.0, 20.0).child(El::block().size(60.0, 20.0).bg(red).live()))
            .child(El::block().abs(20.0, 20.0, f32::NAN, f32::NAN).size(20.0, 20.0).bg(blue));
        let l = Laid::new(&g, root, 100.0, Some(40.0));
        let at = |pass| {
            let mut s = crate::gfx::new_surface(100, 40).unwrap();
            s.canvas().clear(sk::Color::TRANSPARENT);
            g.begin(s.canvas());
            g.set_pass(pass);
            l.paint(&g, &icons, 0.0, 0.0, None);
            g.set_pass(crate::gfx::Pass::All);
            g.end();
            let mut b = [0u8; 4];
            let ii = sk::ImageInfo::new((1, 1), sk::ColorType::BGRA8888, sk::AlphaType::Premul, None);
            assert!(s.read_pixels(&ii, &mut b, 4, (30, 30)));
            b
        };
        assert_eq!(at(crate::gfx::Pass::Static)[3], 0, "not in the static pass");
        assert_eq!(at(crate::gfx::Pass::Live), [255, 0, 0, 255], "over the live box (BGRA blue)");
    }

    /// A scroll box: the hit chain finds it, and how far its content can move (content 500 - box 200).
    #[test]
    fn a_scroll_box_is_found_with_its_range() {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        let rows: Vec<El> = (0..10).map(|_| El::block().h(50.0)).collect();
        let b = cx.scroll_box(key("list"), rows).h(200.0);
        let l = Laid::new(&g, El::block().w(300.0).child(b), 300.0, None);
        let (i, chain) = l.hit(10.0, 150.0).unwrap();
        let _ = i;
        assert_eq!(l.scroll_box_in(&chain), Some((key("list"), 300.0)));
        // the offset moves the content (and its hit test) up
        cx.st.scroll_y.insert(key("list"), 120.0);
        let b = cx.scroll_box(key("list"), vec![El::block().h(50.0).key(key("r0")), El::block().h(500.0)]).h(200.0);
        let l = Laid::new(&g, El::block().w(300.0).child(b), 300.0, None);
        assert_eq!(l.hit(10.0, 10.0).map(|h| h.1.contains(&key("r0"))), Some(false));
    }
}
