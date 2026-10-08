//! Order 041 (smoothness): which parts of a page changed between two builds of its boxes, so the frame rasters only the
//! tiles that changed (Chromium's invalidation) instead of the whole page every frame (10-30 ms a frame on the CPU before).
//! Two `Laid` trees of the same page are compared box by box (same tree position): its painted place (the border box
//! with its outer shadows, moved by its own and its ancestors' CSS transforms), what it paints itself, and what its
//! ancestors do to it (opacity, filters, clips), and whether it is painted in the live pass. A box that differs damages
//! its old AND new place. When the trees differ in shape (boxes came or went) the answer is "everything". A box with its
//! own painting code and no `El::sig` counts as changed at every build, over its box and as much again on every side (a
//! painter may draw past its box: glows, scaled cursor pictures).

use std::collections::hash_map::DefaultHasher;
use std::fmt::Write as _;
use std::hash::Hasher;

use super::el::{Content, El};
use super::lay::Laid;

/// What changed (content DIPs: x, y from the top of the page's content).
#[derive(Clone, Debug, PartialEq)]
pub enum Damage {
    Full,
    Rects(Vec<(f32, f32, f32, f32)>),
}

impl Damage {
    /// Add `d` to what is collected in `acc` (None = nothing yet).
    pub fn add(acc: &mut Option<Damage>, d: Damage) {
        *acc = Some(match (acc.take(), d) {
            (None, d) => d,
            (Some(Damage::Full), _) | (_, Damage::Full) => Damage::Full,
            (Some(Damage::Rects(mut a)), Damage::Rects(b)) => {
                a.extend(b);
                // many small boxes (a list's rows): one band over all of them is cheaper to hand around
                if a.len() > 64 {
                    let u = a.iter().copied().reduce(union).unwrap_or_default();
                    Damage::Rects(vec![u])
                } else {
                    Damage::Rects(a)
                }
            }
        });
    }
}

fn union(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    let (l, t) = (a.0.min(b.0), a.1.min(b.1));
    let (r, bt) = ((a.0 + a.2).max(b.0 + b.2), (a.1 + a.3).max(b.1 + b.3));
    (l, t, r - l, bt - t)
}

struct H(DefaultHasher);
impl std::fmt::Write for H {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.write(s.as_bytes());
        Ok(())
    }
}

/// What the box paints itself (None = it can't be told: its own painting code).
fn own_sig(e: &El) -> Option<u64> {
    if matches!(e.content, Content::Paint(_)) && !e.live && e.paint_sig.is_none() {
        return None;
    }
    let mut h = H(DefaultHasher::new());
    let _ = write!(
        h,
        "{:?}|{:?}|{}|{:?}|{:?}|{:?}|{}|{:?}|{}|{}|{}|{}|{}|{}|{:?}",
        e.style.display, e.bg, e.radius, e.shadows, e.insets, e.border, e.z, e.slim_thumb, e.live, e.backdrop, e.backdrop_sat, e.brightness, e.clip, e.opacity, e.color_filter
    );
    let _ = match &e.content {
        Content::None => write!(h, "N"),
        Content::Text(t) => write!(h, "T{:?}", t),
        Content::Icon(i) => write!(h, "I{:?}", i),
        Content::Paint(_) => write!(h, "P{:?}", e.paint_sig),
    };
    Some(h.0.finish())
}

/// A row-vector affine transform (x' = x*a + y*c + e, y' = x*b + y*d + f), like `Matrix3x2`.
#[derive(Clone, Copy, PartialEq)]
struct M(f32, f32, f32, f32, f32, f32);

impl M {
    const ID: M = M(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
    /// self then o
    fn then(self, o: M) -> M {
        M(
            self.0 * o.0 + self.1 * o.2,
            self.0 * o.1 + self.1 * o.3,
            self.2 * o.0 + self.3 * o.2,
            self.2 * o.1 + self.3 * o.3,
            self.4 * o.0 + self.5 * o.2 + o.4,
            self.4 * o.1 + self.5 * o.3 + o.5,
        )
    }
    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.0 + y * self.2 + self.4, x * self.1 + y * self.3 + self.5)
    }
    /// The bounding box of a rect moved by this transform.
    fn bounds(&self, (x, y, w, h): (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
        let p = [self.apply(x, y), self.apply(x + w, y), self.apply(x, y + h), self.apply(x + w, y + h)];
        let (l, t) = p.iter().fold((f32::MAX, f32::MAX), |a, q| (a.0.min(q.0), a.1.min(q.1)));
        let (r, b) = p.iter().fold((f32::MIN, f32::MIN), |a, q| (a.0.max(q.0), a.1.max(q.1)));
        (l, t, r - l, b - t)
    }
}

/// A box's own CSS transform about its centre (lay.rs `paint_node`).
fn own_m(e: &El, (x, y, w, h): (f32, f32, f32, f32)) -> M {
    if e.translate == (0.0, 0.0) && e.translate_pct == (0.0, 0.0) && e.scale == 1.0 && e.rotate == 0.0 {
        return M::ID;
    }
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (s, c) = e.rotate.to_radians().sin_cos();
    M(1.0, 0.0, 0.0, 1.0, -cx, -cy)
        .then(M(c * e.scale, s * e.scale, -s * e.scale, c * e.scale, 0.0, 0.0))
        .then(M(1.0, 0.0, 0.0, 1.0, cx + e.translate.0 + e.translate_pct.0 * w, cy + e.translate.1 + e.translate_pct.1 * h))
}

/// Per box: its painted place, what its ancestors do to it, what it paints.
struct Info {
    ink: (f32, f32, f32, f32),
    chain: u64,
    own: Option<u64>,
}

fn infos(l: &Laid, page_w: f32) -> Vec<Info> {
    let n = l.nodes.len();
    let mut out: Vec<Info> = Vec::with_capacity(n);
    let mut ms: Vec<M> = Vec::with_capacity(n);
    let mut lives: Vec<(f32, f32, f32, f32)> = Vec::new();
    for (i, nd) in l.nodes.iter().enumerate() {
        let e = &nd.el;
        let (pm, pchain) = match nd.parent {
            Some(p) if p < i => (ms[p], out[p].chain),
            _ => (M::ID, 0),
        };
        let m = own_m(e, nd.rect).then(pm);
        // outer shadows reach |offset| + spread + 1.5 x blur (3 sigma, sigma = blur / 2); text and AA a few px more
        let mut pad = 4.0f32;
        if matches!(e.content, Content::Paint(_)) && e.paint_sig.is_none() {
            let (_, _, w, h) = nd.rect;
            pad = pad.max(w.max(h));
        }
        for s in &e.shadows {
            pad = pad.max(s.dx.abs().max(s.dy.abs()) + s.spread.max(0.0) + 1.5 * s.blur + 2.0);
        }
        let (x, y, w, h) = nd.rect;
        let mut ink = m.bounds((x - pad, y - pad, w + 2.0 * pad, h + 2.0 * pad));
        // a text may run past its box (nowrap): the whole width of the page
        if matches!(e.content, Content::Text(_)) {
            ink = (0.0, ink.1, page_w, ink.3);
        }
        // what this box does to everything inside it: opacity, filters, a clip (its place included)
        let mut h = H(DefaultHasher::new());
        h.0.write_u64(pchain);
        if e.opacity != 1.0 || !e.color_filter.is_empty() || e.brightness != 1.0 || e.clip {
            let cb = m.bounds(nd.rect);
            let _ = write!(h, "{}|{:?}|{}|{}|{:?}", e.opacity, e.color_filter, e.brightness, e.clip, cb);
        }
        ms.push(m);
        // lay.rs's overlap rule: a box painted over an earlier live box is painted in the live pass too (not in the tiles) -
        // which pass it is in is part of what it looks like in the tiles
        let in_live = e.live || lives.iter().any(|l| over(*l, ink));
        let paints = e.bg.is_some() || !e.shadows.is_empty() || !e.insets.is_empty() || e.border.is_some() || e.backdrop > 0.0 || !matches!(e.content, Content::None);
        if in_live && (e.live || paints) && e.opacity > 0.0 {
            lives.push(ink);
        }
        let own = own_sig(e).map(|o| o ^ (in_live as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
        out.push(Info { ink, chain: h.0.finish(), own });
    }
    out
}

fn over(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// What changed from `old` to `new` (both laid out `page_w` wide).
pub fn diff(old: &Laid, new: &Laid, page_w: f32) -> Damage {
    if old.nodes.len() != new.nodes.len() {
        return Damage::Full;
    }
    if old.nodes.iter().zip(&new.nodes).any(|(a, b)| a.parent != b.parent || a.kids.len() != b.kids.len()) {
        return Damage::Full;
    }
    let (a, b) = (infos(old, page_w), infos(new, page_w));
    let mut rects = Vec::new();
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        let geom = old.nodes[i].rect == new.nodes[i].rect && old.nodes[i].content == new.nodes[i].content;
        if geom && x.ink == y.ink && x.chain == y.chain && x.own.is_some() && x.own == y.own {
            continue;
        }
        rects.push(x.ink);
        rects.push(y.ink);
    }
    let mut acc = None;
    Damage::add(&mut acc, Damage::Rects(rects));
    acc.unwrap_or(Damage::Rects(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::{Gfx, Rgba};
    use crate::ui::el::{key, El};

    fn page(hover: f32, extra: bool) -> El {
        let mut rows: Vec<El> = (0..6).map(|i| El::block().h(40.0).key(crate::ui::el::idx(key("row"), i))).collect();
        rows[2] = El::block().h(40.0).key(crate::ui::el::idx(key("row"), 2)).bg(Rgba(1.0, 1.0, 1.0, 0.1 * hover));
        if extra {
            rows.push(El::block().h(40.0));
        }
        El::block().w(600.0).children(rows)
    }

    #[test]
    fn the_same_page_is_no_damage() {
        let g = Gfx::new(1.0);
        let a = Laid::new(&g, page(0.0, false), 600.0, None);
        let b = Laid::new(&g, page(0.0, false), 600.0, None);
        assert_eq!(diff(&a, &b, 600.0), Damage::Rects(vec![]));
    }

    #[test]
    fn a_hovered_row_damages_only_its_place() {
        let g = Gfx::new(1.0);
        let a = Laid::new(&g, page(0.0, false), 600.0, None);
        let b = Laid::new(&g, page(0.5, false), 600.0, None);
        let Damage::Rects(r) = diff(&a, &b, 600.0) else { panic!("full") };
        assert!(!r.is_empty());
        for (_, y, _, h) in r {
            assert!(y >= 80.0 - 5.0 && y + h <= 120.0 + 5.0, "row 2 is at 80..120: {y} {h}");
        }
    }

    #[test]
    fn a_box_more_or_less_is_everything() {
        let g = Gfx::new(1.0);
        let a = Laid::new(&g, page(0.0, false), 600.0, None);
        let b = Laid::new(&g, page(0.0, true), 600.0, None);
        assert_eq!(diff(&a, &b, 600.0), Damage::Full);
    }

    #[test]
    fn a_moved_parent_damages_its_children_old_and_new_place() {
        let g = Gfx::new(1.0);
        let mk = |dy: f32| El::block().w(600.0).child(El::block().h(100.0).translate(0.0, dy).child(El::block().h(20.0).bg(Rgba(1.0, 0.0, 0.0, 1.0))));
        let a = Laid::new(&g, mk(0.0), 600.0, None);
        let b = Laid::new(&g, mk(200.0), 600.0, None);
        let Damage::Rects(r) = diff(&a, &b, 600.0) else { panic!("full") };
        assert!(r.iter().any(|q| q.1 <= 0.0) && r.iter().any(|q| q.1 + q.3 >= 220.0), "{r:?}");
    }
}
