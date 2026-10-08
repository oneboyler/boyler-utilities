//! A small SVG path-data parser (M L H V C S Q T A Z, absolute + relative) that builds Skia paths.
//! The icons are copied verbatim from the drawing's SVG strings.


#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Quad(f32, f32, f32, f32),
    Arc { rx: f32, ry: f32, rot: f32, large: bool, sweep: bool, x: f32, y: f32 },
    Close,
}

struct Lex<'a> {
    s: &'a [u8],
    i: usize,
}

impl Lex<'_> {
    fn skip(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b',' | b'\n' | b'\t' | b'\r') {
            self.i += 1;
        }
    }
    fn cmd(&mut self) -> Option<u8> {
        self.skip();
        if self.i < self.s.len() && self.s[self.i].is_ascii_alphabetic() {
            self.i += 1;
            Some(self.s[self.i - 1])
        } else {
            None
        }
    }
    fn has_num(&mut self) -> bool {
        self.skip();
        self.i < self.s.len() && matches!(self.s[self.i], b'0'..=b'9' | b'-' | b'+' | b'.')
    }
    fn num(&mut self) -> f32 {
        self.skip();
        let st = self.i;
        let s = self.s;
        if self.i < s.len() && (s[self.i] == b'-' || s[self.i] == b'+') {
            self.i += 1;
        }
        let mut dot = false;
        while self.i < s.len() {
            let c = s[self.i];
            if c.is_ascii_digit() {
                self.i += 1;
            } else if c == b'.' && !dot {
                dot = true;
                self.i += 1;
            } else if (c == b'e' || c == b'E') && self.i + 1 < s.len() {
                self.i += 1;
                if s[self.i] == b'-' || s[self.i] == b'+' {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
        std::str::from_utf8(&s[st..self.i]).ok().and_then(|t| t.parse().ok()).unwrap_or(0.0)
    }
    fn flag(&mut self) -> bool {
        self.skip();
        let c = self.s.get(self.i).copied().unwrap_or(b'0');
        self.i += 1;
        c == b'1'
    }
}

/// Parse path data into absolute segments (S/T are turned into C/Q).
pub fn parse(d: &str) -> Vec<Seg> {
    let mut out = Vec::new();
    let mut lx = Lex { s: d.as_bytes(), i: 0 };
    let (mut cx, mut cy, mut sx, mut sy) = (0f32, 0f32, 0f32, 0f32);
    let (mut pcx, mut pcy) = (0f32, 0f32); // last control point for S / T
    let mut last = b' ';
    let mut cmd = b' ';
    loop {
        if let Some(c) = lx.cmd() {
            cmd = c;
        } else if !lx.has_num() {
            break;
        } else if cmd == b'M' {
            cmd = b'L';
        } else if cmd == b'm' {
            cmd = b'l';
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                cx = ox + lx.num();
                cy = oy + lx.num();
                sx = cx;
                sy = cy;
                out.push(Seg::Move(cx, cy));
            }
            b'L' => {
                cx = ox + lx.num();
                cy = oy + lx.num();
                out.push(Seg::Line(cx, cy));
            }
            b'H' => {
                cx = (if rel { cx } else { 0.0 }) + lx.num();
                out.push(Seg::Line(cx, cy));
            }
            b'V' => {
                cy = (if rel { cy } else { 0.0 }) + lx.num();
                out.push(Seg::Line(cx, cy));
            }
            b'C' => {
                let (x1, y1, x2, y2, x, y) = (ox + lx.num(), oy + lx.num(), ox + lx.num(), oy + lx.num(), ox + lx.num(), oy + lx.num());
                out.push(Seg::Cubic(x1, y1, x2, y2, x, y));
                pcx = x2;
                pcy = y2;
                cx = x;
                cy = y;
            }
            b'S' => {
                let (x1, y1) = if matches!(last.to_ascii_uppercase(), b'C' | b'S') { (2.0 * cx - pcx, 2.0 * cy - pcy) } else { (cx, cy) };
                let (x2, y2, x, y) = (ox + lx.num(), oy + lx.num(), ox + lx.num(), oy + lx.num());
                out.push(Seg::Cubic(x1, y1, x2, y2, x, y));
                pcx = x2;
                pcy = y2;
                cx = x;
                cy = y;
            }
            b'Q' => {
                let (x1, y1, x, y) = (ox + lx.num(), oy + lx.num(), ox + lx.num(), oy + lx.num());
                out.push(Seg::Quad(x1, y1, x, y));
                pcx = x1;
                pcy = y1;
                cx = x;
                cy = y;
            }
            b'T' => {
                let (x1, y1) = if matches!(last.to_ascii_uppercase(), b'Q' | b'T') { (2.0 * cx - pcx, 2.0 * cy - pcy) } else { (cx, cy) };
                let (x, y) = (ox + lx.num(), oy + lx.num());
                out.push(Seg::Quad(x1, y1, x, y));
                pcx = x1;
                pcy = y1;
                cx = x;
                cy = y;
            }
            b'A' => {
                let rx = lx.num();
                let ry = lx.num();
                let rot = lx.num();
                let large = lx.flag();
                let sweep = lx.flag();
                let x = ox + lx.num();
                let y = oy + lx.num();
                out.push(Seg::Arc { rx, ry, rot, large, sweep, x, y });
                cx = x;
                cy = y;
            }
            b'Z' => {
                out.push(Seg::Close);
                cx = sx;
                cy = sy;
            }
            _ => break,
        }
        last = cmd;
    }
    out
}

/// Build a Skia path from parsed segments the way Blink's SVG path builder does (arcs through SkPath::arcTo = conics).
pub fn to_path(segs: &[Seg]) -> skia_safe::Path {
    use skia_safe::{path_builder::ArcSize, PathBuilder, PathDirection};
    let mut b = PathBuilder::new();
    for s in segs {
        match *s {
            Seg::Move(x, y) => {
                b.move_to((x, y));
            }
            Seg::Line(x, y) => {
                b.line_to((x, y));
            }
            Seg::Cubic(x1, y1, x2, y2, x, y) => {
                b.cubic_to((x1, y1), (x2, y2), (x, y));
            }
            Seg::Quad(x1, y1, x, y) => {
                b.quad_to((x1, y1), (x, y));
            }
            Seg::Arc { rx, ry, rot, large, sweep, x, y } => {
                b.arc_to_radius((rx, ry), rot, if large { ArcSize::Large } else { ArcSize::Small }, if sweep { PathDirection::CW } else { PathDirection::CCW }, (x, y));
            }
            Seg::Close => {
                b.close();
            }
        }
    }
    b.detach()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_relative_and_implicit() {
        let s = parse("M3.5 8h2.8L10 5v10l-3.7-3H3.5z");
        assert_eq!(s[0], Seg::Move(3.5, 8.0));
        assert!(matches!(s[1], Seg::Line(x, y) if (x - 6.3).abs() < 1e-5 && y == 8.0));
        assert_eq!(s[2], Seg::Line(10.0, 5.0));
        assert_eq!(s[3], Seg::Line(10.0, 15.0));
        assert_eq!(s.last(), Some(&Seg::Close));
        let a = parse("M13 7.6a3.4 3.4 0 0 1 0 4.8");
        assert!(matches!(a[1], Seg::Arc { sweep: true, large: false, .. }));
        let p = parse("M1 1 2 2l.5.5");
        assert_eq!(p[1], Seg::Line(2.0, 2.0));
        assert_eq!(p[2], Seg::Line(2.5, 2.5));
    }
}
