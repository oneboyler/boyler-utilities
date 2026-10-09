//! The keyboard picture (Order 058, keyboard-v2.html): the keys of a keyboard drawn in the glass style, their places in
//! drawing units (one key = 22 units), the sizes Full / TKL / 75 % / 60 %, and the painter. The labels of the printing keys
//! come from the user's CURRENT Windows layout (`bu_keysound::layout`), so a Croatian QWERTZ keyboard shows Z where a US
//! one shows Y and Č Ć Ž Š Đ where it has them; the other keys keep fixed names. A key is named by its scancode (its place).

use super::prefs::Size;
use crate::gfx::{sh, Align, Font, Gfx, Rgba};
use crate::ui::{cmix, ACC, ACC_GLOW, ACC_S, CTL, CTL_H, FG, FG2, HAIR, SEL};
use bu_keysound::remap::Code;

/// A Croatian QWERTZ layout, for test copies (so their pictures are the same on every PC).
pub struct Qwertz;

impl bu_keysound::layout::Layout for Qwertz {
    fn vk_of(&self, code: Code) -> Option<u16> {
        Some(code)
    }
    fn char_of(&self, vk: u16) -> Option<char> {
        Some(match vk {
            0x29 => '¸',
            0x02..=0x0A => char::from_digit(u32::from(vk) - 1, 10)?,
            0x0B => '0',
            0x0C => '\'',
            0x0D => '+',
            0x10..=0x14 => ['q', 'w', 'e', 'r', 't'][usize::from(vk) - 0x10],
            0x15 => 'z',
            0x16..=0x19 => ['u', 'i', 'o', 'p'][usize::from(vk) - 0x16],
            0x1A => 'š',
            0x1B => 'đ',
            0x1E..=0x26 => ['a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l'][usize::from(vk) - 0x1E],
            0x27 => 'č',
            0x28 => 'ć',
            0x2B => 'ž',
            0x56 => '<',
            0x2C => 'y',
            0x2D..=0x32 => ['x', 'c', 'v', 'b', 'n', 'm'][usize::from(vk) - 0x2D],
            0x33 => ',',
            0x34 => '.',
            0x35 => '-',
            _ => return None,
        })
    }
}

/// One key = this many drawing units (the drawing's `U`).
pub const U: f32 = 22.0;
/// The gap between keys, in units.
const GAP: f32 = 1.6;
/// The Pause key sends its own multi-byte sequence: it can't be remapped, so it is drawn but not clickable.
pub const PAUSE: Code = 0xE11D;

const CHR: u8 = 1;
const SM: u8 = 2;
const ISO: u8 = 4;
const DEAD: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Rect,
    /// The tall (ISO) Enter: two rows, narrower below.
    Iso,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyBox {
    pub code: Code,
    /// The name when the layout doesn't give one.
    pub label: &'static str,
    /// A printing key: its label comes from the layout.
    pub chr: bool,
    /// Position and size in keys (1 = one key).
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub shape: Shape,
    pub small: bool,
    pub dead: bool,
}

fn k(label: &'static str, w: f32, code: Code, f: u8) -> (&'static str, f32, Code, u8) {
    (label, w, code, f)
}

/// The five rows of the main block (a Croatian / ISO keyboard: 15 keys wide, the tall Enter, the key right of left Shift).
fn main_rows() -> [Vec<(&'static str, f32, Code, u8)>; 5] {
    let digits = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
    let mut r1 = vec![k("¸", 1.0, 0x29, CHR)];
    r1.extend(digits.iter().enumerate().map(|(i, d)| k(d, 1.0, 0x02 + i as Code, CHR)));
    r1.extend([k("'", 1.0, 0x0C, CHR), k("+", 1.0, 0x0D, CHR), k("Backspace", 2.0, 0x0E, SM)]);
    let top = ["Q", "W", "E", "R", "T", "Z", "U", "I", "O", "P"];
    let mut r2 = vec![k("Tab", 1.5, 0x0F, SM)];
    r2.extend(top.iter().enumerate().map(|(i, c)| k(c, 1.0, 0x10 + i as Code, CHR)));
    r2.extend([k("Š", 1.0, 0x1A, CHR), k("Đ", 1.0, 0x1B, CHR), k("Enter", 1.5, 0x1C, ISO)]);
    let home = ["A", "S", "D", "F", "G", "H", "J", "K", "L"];
    let mut r3 = vec![k("Caps Lock", 1.75, 0x3A, SM)];
    r3.extend(home.iter().enumerate().map(|(i, c)| k(c, 1.0, 0x1E + i as Code, CHR)));
    r3.extend([k("Č", 1.0, 0x27, CHR), k("Ć", 1.0, 0x28, CHR), k("Ž", 1.0, 0x2B, CHR)]);
    let bottom = ["Y", "X", "C", "V", "B", "N", "M"];
    let mut r4 = vec![k("Shift", 1.25, 0x2A, SM), k("<", 1.0, 0x56, CHR)];
    r4.extend(bottom.iter().enumerate().map(|(i, c)| k(c, 1.0, 0x2C + i as Code, CHR)));
    r4.extend([k(",", 1.0, 0x33, CHR), k(".", 1.0, 0x34, CHR), k("-", 1.0, 0x35, CHR), k("Shift", 2.75, 0x36, SM)]);
    let r5 = vec![
        k("Ctrl", 1.25, 0x1D, SM),
        k("Win", 1.25, 0xE05B, SM),
        k("Alt", 1.25, 0x38, SM),
        k("Space", 6.25, 0x39, 0),
        k("Alt Gr", 1.25, 0xE038, SM),
        k("Win", 1.25, 0xE05C, SM),
        k("Menu", 1.25, 0xE05D, SM),
        k("Ctrl", 1.25, 0xE01D, SM),
    ];
    [r1, r2, r3, r4, r5]
}

struct Builder {
    out: Vec<KeyBox>,
}

impl Builder {
    fn key(&mut self, label: &'static str, w: f32, h: f32, x: f32, y: f32, code: Code, f: u8) {
        let shape = if f & ISO != 0 { Shape::Iso } else { Shape::Rect };
        self.out.push(KeyBox { code, label, chr: f & CHR != 0, x, y, w, h, shape, small: f & SM != 0, dead: f & DEAD != 0 });
    }
    fn row(&mut self, row: &[(&'static str, f32, Code, u8)], x0: f32, y: f32) {
        let mut x = x0;
        for (l, w, c, f) in row {
            self.key(l, *w, 1.0, x, y, *c, *f);
            x += w;
        }
    }
}

/// The keys of `size` and the picture's size in keys (width, height).
pub fn keys(size: Size) -> (Vec<KeyBox>, f32, f32) {
    let mut b = Builder { out: Vec::new() };
    let m = main_rows();
    let f_keys = |b: &mut Builder, x: f32, first: u32| {
        let codes: [Code; 12] = [0x3B, 0x3C, 0x3D, 0x3E, 0x3F, 0x40, 0x41, 0x42, 0x43, 0x44, 0x57, 0x58];
        let names = ["F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12"];
        for i in 0..4 {
            let n = first as usize + i;
            b.key(names[n], 1.0, 1.0, x + i as f32, 0.0, codes[n], SM);
        }
    };
    match size {
        Size::P60 => {
            b.key("Esc", 1.0, 1.0, 0.0, 0.0, 0x01, SM);
            b.row(&m[0][1..], 1.0, 0.0);
            for (i, row) in m.iter().enumerate().skip(1) {
                b.row(row, 0.0, i as f32);
            }
            (b.out, 15.0, 5.0)
        }
        Size::P75 => {
            b.key("Esc", 1.0, 1.0, 0.0, 0.0, 0x01, SM);
            f_keys(&mut b, 1.0, 0);
            f_keys(&mut b, 5.0, 4);
            f_keys(&mut b, 9.0, 8);
            b.key("PrtSc", 1.0, 1.0, 13.0, 0.0, 0xE037, SM);
            b.key("Ins", 1.0, 1.0, 14.0, 0.0, 0xE052, SM);
            b.row(&m[0], 0.0, 1.25);
            b.row(&m[1], 0.0, 2.25);
            b.row(&m[2], 0.0, 3.25);
            let mut r4 = m[3][..12].to_vec();
            r4.push(k("Shift", 1.75, 0x36, SM));
            b.row(&r4, 0.0, 4.25);
            b.key("↑", 1.0, 1.0, 14.0, 4.25, 0xE048, 0);
            let r5 = vec![m[4][0], m[4][1], m[4][2], m[4][3], k("Alt Gr", 1.0, 0xE038, SM), k("Ctrl", 1.0, 0xE01D, SM)];
            b.row(&r5, 0.0, 5.25);
            b.key("←", 1.0, 1.0, 12.0, 5.25, 0xE04B, 0);
            b.key("↓", 1.0, 1.0, 13.0, 5.25, 0xE050, 0);
            b.key("→", 1.0, 1.0, 14.0, 5.25, 0xE04D, 0);
            for (i, (l, c)) in [("Del", 0xE053u16), ("Home", 0xE047), ("PgUp", 0xE049), ("PgDn", 0xE051), ("End", 0xE04F)].into_iter().enumerate() {
                let y = if i == 0 { 0.0 } else { i as f32 + 0.25 };
                b.key(l, 1.0, 1.0, 15.25, y, c, SM);
            }
            (b.out, 16.25, 6.25)
        }
        Size::Full | Size::Tkl => {
            b.key("Esc", 1.0, 1.0, 0.0, 0.0, 0x01, SM);
            f_keys(&mut b, 2.0, 0);
            f_keys(&mut b, 6.5, 4);
            f_keys(&mut b, 11.0, 8);
            b.key("PrtSc", 1.0, 1.0, 15.25, 0.0, 0xE037, SM);
            b.key("ScrLk", 1.0, 1.0, 16.25, 0.0, 0x46, SM);
            b.key("Pause", 1.0, 1.0, 17.25, 0.0, PAUSE, SM | DEAD);
            for (i, row) in m.iter().enumerate() {
                b.row(row, 0.0, 1.5 + i as f32);
            }
            for (i, (l, c)) in [("Ins", 0xE052u16), ("Home", 0xE047), ("PgUp", 0xE049)].into_iter().enumerate() {
                b.key(l, 1.0, 1.0, 15.25 + i as f32, 1.5, c, SM);
            }
            for (i, (l, c)) in [("Del", 0xE053u16), ("End", 0xE04F), ("PgDn", 0xE051)].into_iter().enumerate() {
                b.key(l, 1.0, 1.0, 15.25 + i as f32, 2.5, c, SM);
            }
            b.key("↑", 1.0, 1.0, 16.25, 4.5, 0xE048, 0);
            b.key("←", 1.0, 1.0, 15.25, 5.5, 0xE04B, 0);
            b.key("↓", 1.0, 1.0, 16.25, 5.5, 0xE050, 0);
            b.key("→", 1.0, 1.0, 17.25, 5.5, 0xE04D, 0);
            if size == Size::Tkl {
                return (b.out, 18.25, 6.5);
            }
            let nx = 18.5;
            let pad: [(&'static str, f32, f32, Code, u8); 13] = [
                ("Num", nx, 1.5, 0x45, SM),
                ("/", nx + 1.0, 1.5, 0xE035, 0),
                ("*", nx + 2.0, 1.5, 0x37, 0),
                ("−", nx + 3.0, 1.5, 0x4A, 0),
                ("7", nx, 2.5, 0x47, 0),
                ("8", nx + 1.0, 2.5, 0x48, 0),
                ("9", nx + 2.0, 2.5, 0x49, 0),
                ("4", nx, 3.5, 0x4B, 0),
                ("5", nx + 1.0, 3.5, 0x4C, 0),
                ("6", nx + 2.0, 3.5, 0x4D, 0),
                ("1", nx, 4.5, 0x4F, 0),
                ("2", nx + 1.0, 4.5, 0x50, 0),
                ("3", nx + 2.0, 4.5, 0x51, 0),
            ];
            for (l, x, y, c, f) in pad {
                b.key(l, 1.0, 1.0, x, y, c, f);
            }
            b.key("+", 1.0, 2.0, nx + 3.0, 2.5, 0x4E, 0);
            b.key("Enter", 1.0, 2.0, nx + 3.0, 4.5, 0xE01C, SM);
            b.key("0", 2.0, 1.0, nx, 5.5, 0x52, 0);
            b.key(".", 1.0, 1.0, nx + 2.0, 5.5, 0x53, 0);
            (b.out, 22.5, 6.5)
        }
    }
}

/// Is this one of the numpad's keys? (The keys manager takes none of them: keyboards without a numpad; the picture lets them
/// be remapped, not given an action or macro.)
pub fn is_numpad(code: Code) -> bool {
    matches!(code, 0x45 | 0x37 | 0x4A | 0x4E | 0x47..=0x49 | 0x4B..=0x4D | 0x4F..=0x53 | 0xE035 | 0xE01C)
}

/// What each key shows: the layout's own label for printing keys, else the fixed name.
pub fn labels(keys: &[KeyBox], lay: &dyn bu_keysound::layout::Layout) -> Vec<String> {
    keys.iter()
        .map(|k| {
            if k.chr {
                let l = bu_keysound::layout::label(lay, k.code);
                if l.starts_with("Key 0x") {
                    k.label.to_string()
                } else {
                    l
                }
            } else {
                k.label.to_string()
            }
        })
        .collect()
}

/// How one key looks now.
#[derive(Clone, Copy, Default)]
pub struct Look {
    /// Hover, 0..1 (a transition).
    pub hv: f32,
    pub sel: bool,
    /// Changed: remapped or carrying an action / macro (it glows).
    pub changed: bool,
}

/// The picture's height in px for a width.
pub fn height(h_keys: f32, w_keys: f32, width: f32) -> f32 {
    h_keys * U * (width / (w_keys * U))
}

/// Where a key's boxes are, in picture px (x, y, w, h); the ISO Enter has two.
pub fn boxes(k: &KeyBox, scale: f32) -> Vec<(f32, f32, f32, f32)> {
    let (x, y, w, h) = (k.x * U * scale, k.y * U * scale, k.w * U * scale, k.h * U * scale);
    match k.shape {
        Shape::Rect => vec![(x, y, w, h)],
        Shape::Iso => {
            let notch = 0.25 * U * scale;
            vec![(x, y, w, h), (x + notch, y + h, w - notch, h)]
        }
    }
}

fn key_font(size: f32, weight: u16) -> Font {
    Font::new(size, weight).ls(0)
}

/// Paints the whole keyboard into (x, y) with the picture width `w` px. `descr(i)` = what a changed key does (a short text for
/// the wide ones).
pub fn paint(g: &Gfx, keys: &[KeyBox], texts: &[String], looks: &[Look], descr: &dyn Fn(usize) -> Option<String>, (x, y, w): (f32, f32, f32), w_keys: f32) {
    let s = w / (w_keys * U);
    let r = 4.0 * s;
    for (i, k) in keys.iter().enumerate() {
        let lk = looks.get(i).copied().unwrap_or_default();
        let on = if lk.sel { 1.0 } else { lk.hv };
        let (base, edge) = (CTL(), HAIR());
        let mut fill = cmix(base, CTL_H(), on);
        let mut stroke = cmix(edge, ACC_S(), on);
        let mut sw = 0.8 * s;
        if lk.changed {
            fill = SEL();
            stroke = ACC();
            sw = 1.2 * s;
        }
        if lk.sel {
            stroke = ACC();
            sw = 1.6 * s;
        }
        let a = if k.dead { 0.55 } else { 1.0 };
        let (kx, ky) = (x + (k.x * U + GAP / 2.0) * s, y + (k.y * U + GAP / 2.0) * s);
        let (kw, kh) = ((k.w * U - GAP) * s, (k.h * U - GAP) * s);
        match k.shape {
            Shape::Rect => {
                if lk.changed {
                    g.box_shadows(kx, ky, kw, kh, r, &[sh(0.0, 0.0, 6.0 * s, 0.0, ACC_GLOW().mul_a(a))], false);
                }
                g.fill_rr(kx, ky, kw, kh, r, fill.mul_a(a));
                g.stroke_rr(kx, ky, kw, kh, r, sw, stroke.mul_a(a));
            }
            Shape::Iso => {
                // x0..x1 on the first row, narrower (notch) on the second
                let x1 = kx + kw;
                let notch = 0.25 * U * s;
                let y2 = ky + (U * s) - GAP * s / 2.0;
                let y3 = ky + 2.0 * U * s - GAP * s;
                let d = format!("M{kx} {ky}H{x1}V{y3}H{} V{y2}H{kx}Z", kx + notch);
                let p = g.path(&d);
                g.fill_geom(&p, fill.mul_a(a));
                g.stroke_geom_ex(&p, sw, stroke.mul_a(a), true, true, 1.0);
            }
        }
        let text = texts.get(i).map(String::as_str).unwrap_or(k.label);
        let wide_desc = if lk.changed && k.w >= 1.5 { descr(i) } else { None };
        let fs = if k.small || text.chars().count() > 2 { 7.0 * s } else { 8.6 * s };
        let (font, col) = (key_font(fs, 500), if lk.changed { FG() } else { FG2() });
        let lh = (fs * 1.3).round().max(8.0);
        let ty = if k.shape == Shape::Iso { ky + (U * s - lh) / 2.0 + 2.0 * s } else if wide_desc.is_some() { ky + kh / 2.0 - lh + 1.0 * s } else { ky + (kh - lh) / 2.0 };
        g.text(text, font, kx + kw / 2.0, ty, lh, col.mul_a(a), Align::Center, kw);
        if let Some(d) = wide_desc {
            let df = key_font(6.4 * s, 600);
            let dl = (6.4 * s * 1.3).round().max(8.0);
            g.text(&d, df, kx + kw / 2.0, ky + kh - dl - 1.5 * s, dl, ACC(), Align::Center, kw);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_size_has_its_keys_inside_its_width_and_none_twice() {
        for (size, w, n) in [(Size::Full, 22.5f32, 104usize), (Size::Tkl, 18.25, 87), (Size::P75, 16.25, 82), (Size::P60, 15.0, 61)] {
            let (ks, kw, kh) = keys(size);
            assert_eq!(kw, w, "{size:?}");
            assert!(kh >= 5.0 && kh < 7.0);
            for k in &ks {
                assert!(k.x >= 0.0 && k.x + k.w <= kw + 0.001, "{size:?}: {} sticks out ({} + {} > {kw})", k.label, k.x, k.w);
                assert!(k.y >= 0.0 && k.y + k.h <= kh + 0.001, "{size:?}: {} sticks out vertically", k.label);
            }
            let mut codes: Vec<Code> = ks.iter().map(|k| k.code).collect();
            codes.sort_unstable();
            let total = codes.len();
            codes.dedup();
            assert_eq!(codes.len(), total, "{size:?}: a key is there twice");
            assert!((total as i32 - n as i32).abs() <= 3, "{size:?}: {total} keys, about {n}");
        }
    }

    #[test]
    fn keys_do_not_overlap() {
        for size in Size::ALL {
            let (ks, _, _) = keys(size);
            for (i, a) in ks.iter().enumerate() {
                for b in &ks[i + 1..] {
                    let ox = a.x < b.x + b.w - 0.01 && b.x < a.x + a.w - 0.01;
                    let oy = a.y < b.y + b.h - 0.01 && b.y < a.y + a.h - 0.01;
                    assert!(!(ox && oy), "{size:?}: {} and {} overlap", a.label, b.label);
                }
            }
        }
    }

    #[test]
    fn the_main_block_is_fifteen_keys_wide_on_every_row() {
        let (ks, _, _) = keys(Size::P60);
        for row in 0..5 {
            let right = ks.iter().filter(|k| (k.y - row as f32).abs() < 0.01).map(|k| k.x + k.w).fold(0.0f32, f32::max);
            // the Caps Lock row ends where the tall Enter narrows (its lower part is the Enter key of the row above)
            let want = if row == 2 { 13.75 } else { 15.0 };
            assert!((right - want).abs() < 0.01, "row {row} ends at {right}");
        }
    }

    #[test]
    fn labels_come_from_the_layout() {
        struct Qwertz;
        impl bu_keysound::layout::Layout for Qwertz {
            fn vk_of(&self, c: Code) -> Option<u16> {
                Some(c)
            }
            fn char_of(&self, vk: u16) -> Option<char> {
                // the picture's own table says Z at 0x15 and Y at 0x2C; this layout says the opposite (a US one): the layout wins
                match vk {
                    0x15 => Some('y'),
                    0x2C => Some('z'),
                    _ => None,
                }
            }
        }
        let (ks, _, _) = keys(Size::P60);
        let t = labels(&ks, &Qwertz);
        let at = |c: Code| t[ks.iter().position(|k| k.code == c).unwrap()].clone();
        assert_eq!(at(0x15), "Y");
        assert_eq!(at(0x2C), "Z");
        assert_eq!(at(0x1E), "A", "no character known: the picture's own label");
        assert_eq!(at(0x39), "Space");
        assert_eq!(at(0x0E), "Backspace");
    }

    #[test]
    fn numpad_keys_are_known_and_the_arrows_are_not_numpad() {
        for c in [0x45, 0x37, 0x47, 0x4E, 0x53, 0xE035, 0xE01C] {
            assert!(is_numpad(c), "{c:X}");
        }
        for c in [0xE047, 0xE048, 0x1C, 0xE037, 0x39, 0x01] {
            assert!(!is_numpad(c), "{c:X}");
        }
    }

    #[test]
    fn the_iso_enter_has_two_boxes_and_the_rest_one() {
        let (ks, _, _) = keys(Size::Full);
        let enter = ks.iter().find(|k| k.code == 0x1C).unwrap();
        assert_eq!(boxes(enter, 1.0).len(), 2);
        assert!(ks.iter().filter(|k| k.code != 0x1C).all(|k| boxes(k, 1.0).len() == 1));
        assert!(ks.iter().find(|k| k.code == PAUSE).unwrap().dead);
    }

    #[test]
    fn the_picture_scales_to_the_width() {
        assert!((height(6.5, 22.5, 540.0) - 156.0).abs() < 0.01);
        assert!(height(5.0, 15.0, 540.0) > 150.0);
    }
}
