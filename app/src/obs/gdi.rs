//! ClipPing's popup looks (popup.c), drawn exactly as ClipPing draws them - the same GDI calls, fonts, layout numbers and
//! pixel loops - so its six looks (Card, Pill, Accent edge, Timer bar, Tile, Floating text) look the same here. Also the
//! status icon's tiles (status.c) and the placement overlay (place.c). Every picture is premultiplied BGRA
//! (u32 = 0xAARRGGBB) for UpdateLayeredWindow.

use bu_obs::engine::{Color, Icon, PopMsg};
use bu_obs::settings::{ST_ACCENT, ST_CARD, ST_FLOAT, ST_PILL, ST_TILE, ST_TIMER};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, LPARAM, RECT, SIZE};
use windows::Win32::Graphics::Gdi::*;

/// A picture: premultiplied BGRA.
#[derive(Clone, Debug, PartialEq)]
pub struct Img {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u32>,
}

const STATE_DARK: [u32; 5] = [0x97C459, 0xF09595, 0x85B7EB, 0xEF9F27, 0xB4B2A9];
const STATE_LIGHT: [u32; 5] = [0x3B6D11, 0xA32D2D, 0x185FA5, 0x854F0B, 0x5F5E5A];

/// Segoe Fluent Icons (Win 11) / Segoe MDL2 Assets (Win 10) code points; second = fallback if missing
pub const GLYPHS: [(u16, u16); 11] = [
    (0xE73E, 0xE73E),
    (0xE711, 0xE711),
    (0xE783, 0xE783),
    (0xE768, 0xE768),
    (0xE71A, 0xE71A),
    (0xE7C8, 0xE7C8),
    (0xE8AB, 0xE8AB),
    (0xECF0, 0xE703),
    (0xECF0, 0xE703),
    (0xE713, 0xE713),
    (0xEDA2, 0xEDA2),
];
const BADGE_X: u16 = 0xED2E;
const BADGE_BG: u16 = 0xE91F;

pub fn color_index(c: Color) -> usize {
    match c {
        Color::Green => 0,
        Color::Red => 1,
        Color::Blue => 2,
        Color::Amber => 3,
        Color::Grey => 4,
    }
}
pub fn icon_index(i: Icon) -> usize {
    match i {
        Icon::Check => 0,
        Icon::Cross => 1,
        Icon::Bang => 2,
        Icon::Play => 3,
        Icon::Stop => 4,
        Icon::Rec => 5,
        Icon::Switch => 6,
        Icon::Plug => 7,
        Icon::PlugX => 8,
        Icon::Gear => 9,
        Icon::Drive => 10,
    }
}

/// The state colour of a popup on dark / light backgrounds.
pub fn state_color(c: Color, light: bool) -> u32 {
    (if light { STATE_LIGHT } else { STATE_DARK })[color_index(c)]
}

/// Win32 MulDiv (rounded to nearest)
pub fn muldiv(a: i32, b: i32, c: i32) -> i32 {
    if c == 0 {
        return -1;
    }
    let p = a as i64 * b as i64;
    let c = c as i64;
    let r = if (p < 0) != (c < 0) { (p - c.abs() / 2 * c.signum()) / c } else { (p + c / 2) / c };
    r as i32
}

fn cr(c: u32) -> COLORREF {
    COLORREF(((c >> 16) & 255) | (((c >> 8) & 255) << 8) | ((c & 255) << 16))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

unsafe extern "system" fn font_cb(_lf: *const LOGFONTW, _tm: *const TEXTMETRICW, _t: u32, lp: LPARAM) -> i32 {
    *(lp.0 as *mut i32) = 1;
    0
}

/// "Segoe Fluent Icons" if installed, else "Segoe MDL2 Assets" (asked once).
pub fn icon_face() -> &'static str {
    static F: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    F.get_or_init(|| unsafe {
        let mut lf = LOGFONTW { lfCharSet: DEFAULT_CHARSET, ..Default::default() };
        for (i, c) in "Segoe Fluent Icons".encode_utf16().enumerate() {
            lf.lfFaceName[i] = c;
        }
        let dc = GetDC(None);
        let mut found = 0i32;
        EnumFontFamiliesExW(dc, &lf, Some(font_cb), LPARAM(&mut found as *mut i32 as isize), 0);
        ReleaseDC(None, dc);
        if found != 0 {
            "Segoe Fluent Icons"
        } else {
            "Segoe MDL2 Assets"
        }
    })
}

#[derive(Clone, Copy)]
struct Sz {
    dpi: i32,
    scale: i32,
}
fn s(z: Sz, v: i32) -> i32 {
    muldiv(v, z.dpi * z.scale, 96 * 100)
}

fn mkfont(z: Sz, face: &str, px: i32, weight: i32, q: FONT_QUALITY) -> HFONT {
    let mut f: Vec<u16> = wide(face);
    f.push(0);
    unsafe {
        CreateFontW(-s(z, px), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, OUT_TT_PRECIS, CLIP_DEFAULT_PRECIS, q, DEFAULT_PITCH.0 as u32, PCWSTR(f.as_ptr()))
    }
}

struct Fonts {
    top: HFONT,
    mainf: HFONT,
    line: HFONT,
    lineb: HFONT,
    icon: HFONT,
    iconm: HFONT,
    iconl: HFONT,
    badge: HFONT,
    tileb: HFONT,
    tiled: HFONT,
}

impl Fonts {
    fn new(z: Sz, grey: bool) -> Fonts {
        let q = if grey { ANTIALIASED_QUALITY } else { CLEARTYPE_QUALITY };
        let ic = icon_face();
        Fonts {
            top: mkfont(z, "Segoe UI", 12, FW_NORMAL.0 as i32, q),
            mainf: mkfont(z, "Segoe UI", 14, FW_SEMIBOLD.0 as i32, q),
            line: mkfont(z, "Segoe UI", 13, FW_NORMAL.0 as i32, q),
            lineb: mkfont(z, "Segoe UI", 13, FW_SEMIBOLD.0 as i32, q),
            icon: mkfont(z, ic, 22, FW_NORMAL.0 as i32, q),
            iconm: mkfont(z, ic, 20, FW_NORMAL.0 as i32, q),
            iconl: mkfont(z, ic, 30, FW_NORMAL.0 as i32, q),
            badge: mkfont(z, ic, 14, FW_NORMAL.0 as i32, q),
            tileb: mkfont(z, "Segoe UI", 13, FW_SEMIBOLD.0 as i32, q),
            tiled: mkfont(z, "Segoe UI", 12, FW_NORMAL.0 as i32, q),
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            for f in [self.top, self.mainf, self.line, self.lineb, self.icon, self.iconm, self.iconl, self.badge, self.tileb, self.tiled] {
                let _ = DeleteObject(f.into());
            }
        }
    }
}

struct Look {
    style: i32,
    light: bool,
    bg: u32,
    main: u32,
    top: u32,
    state: u32,
}

fn is_light(c: u32) -> bool {
    (2126 * ((c >> 16) & 255) + 7152 * ((c >> 8) & 255) + 722 * (c & 255)) / 10000 > 140
}

fn look(style: i32, bg: u32, color: Color) -> Look {
    let light = style != ST_FLOAT && is_light(bg);
    Look {
        style,
        light,
        bg,
        main: if light { 0x2C2C2A } else { 0xF1EFE8 },
        top: if light { 0x5F5E5A } else { 0xB4B2A9 },
        state: state_color(color, light),
    }
}

#[derive(Default, Clone, Copy)]
struct Lay {
    w: i32,
    h: i32,
    radius: i32,
    icon: RECT,
    top: RECT,
    mainr: RECT,
    line: RECT,
    title: RECT,
    detail: RECT,
    strip: RECT,
    dot: RECT,
    bar: RECT,
    /// 0 none, 1 = 22, 2 = 20, 3 = 30
    iconsz: i32,
}

fn setr(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT { left: x, top: y, right: x + w, bottom: y + h }
}

fn tw(dc: HDC, f: HFONT, s: &str, hgt: Option<&mut i32>) -> i32 {
    let w = wide(s);
    let mut z = SIZE::default();
    let mut tm = TEXTMETRICW::default();
    unsafe {
        SelectObject(dc, f.into());
        if !w.is_empty() {
            let _ = GetTextExtentPoint32W(dc, &w, &mut z);
        }
        let _ = GetTextMetricsW(dc, &mut tm);
    }
    if let Some(h) = hgt {
        *h = tm.tmHeight;
    }
    if s.is_empty() {
        0
    } else {
        z.cx
    }
}

fn pill_text(m: &PopMsg) -> String {
    if m.detail.is_empty() {
        m.title.clone()
    } else {
        format!("{} · {}", m.title, m.detail)
    }
}

fn layout(dc: HDC, f: &Fonts, z: Sz, m: &PopMsg, style: i32) -> Lay {
    let mut l = Lay::default();
    let (mut ha, mut hb) = (0, 0);
    match style {
        ST_ACCENT => {
            let a = tw(dc, f.top, &m.top, Some(&mut ha));
            let b = tw(dc, f.mainf, &m.main, Some(&mut hb));
            let wmax = a.max(b);
            l.top = setr(s(z, 4 + 12), s(z, 10), wmax, ha);
            l.mainr = setr(l.top.left, l.top.top + ha, wmax, hb);
            l.w = l.top.left + wmax + s(z, 14);
            l.h = l.mainr.bottom + s(z, 10);
            l.strip = setr(0, 0, s(z, 4), l.h);
            l.radius = s(z, 6);
        }
        ST_PILL => {
            let a = tw(dc, f.lineb, &m.title, Some(&mut ha));
            let full = pill_text(m);
            let rest: String = full.chars().skip(m.title.chars().count()).collect();
            let t = tw(dc, f.line, &rest, None);
            l.dot = setr(s(z, 10), 0, s(z, 8), s(z, 8));
            l.line = setr(s(z, 10 + 8 + 8), s(z, 7), a + t, ha);
            l.w = l.line.right + s(z, 14);
            l.h = l.line.bottom + s(z, 7);
            l.dot.top = (l.h - s(z, 8)) / 2;
            l.dot.bottom = l.dot.top + s(z, 8);
            l.radius = l.h / 2;
        }
        ST_TILE => {
            let ih = s(z, 34);
            let a = tw(dc, f.tileb, &m.title, Some(&mut ha));
            let b = tw(dc, f.tiled, &m.detail, Some(&mut hb));
            let wmax = a.max(b);
            l.w = (wmax + s(z, 24)).max(s(z, 120));
            l.iconsz = 3;
            let mut y = s(z, 12);
            l.icon = setr(0, y, l.w, ih);
            y += ih + s(z, 4);
            l.title = setr(s(z, 12), y, l.w - s(z, 24), ha);
            y += ha;
            if !m.detail.is_empty() {
                l.detail = setr(s(z, 12), y, l.w - s(z, 24), hb);
                y += hb;
            }
            l.h = y + s(z, 12);
            l.radius = s(z, 10);
        }
        _ => {
            // Card, Timer bar, Floating text
            let pad = if style == ST_FLOAT { 8 } else { 0 };
            let iw = if style == ST_TIMER { 20 } else { 22 };
            let a = tw(dc, f.top, &m.top, Some(&mut ha));
            let b = tw(dc, f.mainf, &m.main, Some(&mut hb));
            let wmax = a.max(b);
            l.iconsz = if style == ST_TIMER { 2 } else { 1 };
            l.top = setr(s(z, 12 + pad + iw + 10), s(z, 10 + pad), wmax, ha);
            l.mainr = setr(l.top.left, l.top.top + ha, wmax, hb);
            l.w = l.top.left + wmax + s(z, 14 + pad);
            l.h = l.mainr.bottom + s(z, 10 + pad);
            if style == ST_TIMER {
                l.w = l.w.max(s(z, 220));
                l.h += s(z, 3);
                l.bar = setr(0, l.h - s(z, 3), l.w, s(z, 3));
            }
            l.icon = setr(s(z, 12 + pad), 0, s(z, iw), l.h - if style == ST_TIMER { s(z, 3) } else { 0 });
            l.radius = if style == ST_FLOAT { 0 } else { s(z, 10) };
        }
    }
    l
}

fn glyph_for(dc: HDC, icon: usize) -> u16 {
    let g = GLYPHS[icon].0;
    let mut gi = [0u16; 1];
    let r = unsafe { GetGlyphIndicesW(dc, PCWSTR([g, 0].as_ptr()), 1, gi.as_mut_ptr(), GGI_MARK_NONEXISTING_GLYPHS) };
    if r == u32::MAX || gi[0] == 0xFFFF {
        GLYPHS[icon].1
    } else {
        g
    }
}

fn txt(dc: HDC, f: HFONT, col: u32, s: &[u16], r: &RECT, fl: DRAW_TEXT_FORMAT) {
    if s.is_empty() {
        return; // nothing to draw (and an empty slice's pointer must not reach GDI)
    }
    let mut rr = *r;
    let mut buf = s.to_vec();
    unsafe {
        SelectObject(dc, f.into());
        SetTextColor(dc, cr(col));
        DrawTextW(dc, &mut buf, &mut rr, fl | DT_SINGLELINE | DT_NOPREFIX);
    }
}

fn draw_icon(dc: HDC, f: &Fonts, z: Sz, l: &Lay, icon: usize, col: u32, bg: u32) {
    let fi = match l.iconsz {
        2 => f.iconm,
        3 => f.iconl,
        _ => f.icon,
    };
    unsafe {
        SelectObject(dc, fi.into());
    }
    let g = glyph_for(dc, icon);
    txt(dc, fi, col, &[g], &l.icon, DT_CENTER | DT_VCENTER);
    if icon == 8 {
        // small "x" badge on the plug's lower right, with a cut-out ring
        let cx = (l.icon.left + l.icon.right) / 2;
        let cy = (l.icon.top + l.icon.bottom) / 2;
        let br = setr(cx - s(z, 1), cy + s(z, 1), s(z, 16), s(z, 16));
        txt(dc, f.badge, bg, &[BADGE_BG], &br, DT_CENTER | DT_VCENTER);
        txt(dc, f.badge, col, &[BADGE_X], &br, DT_CENTER | DT_VCENTER);
    }
}

fn fillrect(px: &mut [u32], w: i32, h: i32, r: &RECT, col: u32) {
    for y in r.top.max(0)..r.bottom.min(h) {
        for x in r.left.max(0)..r.right.min(w) {
            px[(y * w + x) as usize] = col;
        }
    }
}

fn cover_circle(x: i32, y: i32, cx8: i32, cy8: i32, r8: i32) -> i32 {
    let mut n = 0;
    for sy in 0..4 {
        for sx in 0..4 {
            let dx = x * 8 + 2 * sx + 1 - cx8;
            let dy = y * 8 + 2 * sy + 1 - cy8;
            if dx * dx + dy * dy <= r8 * r8 {
                n += 1;
            }
        }
    }
    n
}

fn mix16(a: u32, b: u32, c: u32) -> u32 {
    let r = (((a >> 16) & 255) * c + ((b >> 16) & 255) * (16 - c)) / 16;
    let g = (((a >> 8) & 255) * c + ((b >> 8) & 255) * (16 - c)) / 16;
    let bl = ((a & 255) * c + (b & 255) * (16 - c)) / 16;
    (r << 16) | (g << 8) | bl
}

fn dot(px: &mut [u32], w: i32, h: i32, r: &RECT, col: u32) {
    let d = r.right - r.left;
    for y in r.top..r.bottom.min(h) {
        for x in r.left..r.right.min(w) {
            let c = cover_circle(x, y, (r.left * 2 + d) * 4, (r.top * 2 + d) * 4, d * 4);
            if c > 0 {
                let i = (y * w + x) as usize;
                px[i] = mix16(col, px[i] & 0xFFFFFF, c as u32);
            }
        }
    }
}

/// GDI leaves alpha at 0: alpha from the rounded-rectangle shape, then premultiplied.
fn shape_alpha(px: &mut [u32], w: i32, h: i32, r: i32) {
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let p = px[i];
            let mut a: u32 = 255;
            if r > 0 && (x < r || x >= w - r) && (y < r || y >= h - r) {
                let mut n = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let (px8, py8) = (x * 8 + 2 * sx + 1, y * 8 + 2 * sy + 1);
                        let dx = if px8 < r * 8 {
                            r * 8 - px8
                        } else if px8 > (w - r) * 8 {
                            px8 - (w - r) * 8
                        } else {
                            0
                        };
                        let dy = if py8 < r * 8 {
                            r * 8 - py8
                        } else if py8 > (h - r) * 8 {
                            py8 - (h - r) * 8
                        } else {
                            0
                        };
                        if dx * dx + dy * dy <= r * r * 64 {
                            n += 1;
                        }
                    }
                }
                a = n * 255 / 16;
            }
            px[i] = (a << 24) | ((((p >> 16) & 255) * a / 255) << 16) | ((((p >> 8) & 255) * a / 255) << 8) | ((p & 255) * a / 255);
        }
    }
}

/// Floating text: glyph masks, then the text over a soft dark halo, all with real alpha.
#[allow(clippy::too_many_arguments)]
fn render_float(dc: HDC, px: &mut [u32], l: &Lay, f: &Fonts, z: Sz, m: &PopMsg, lk: &Look) {
    let n = (l.w * l.h) as usize;
    let col = [lk.state, lk.top, lk.main];
    let mut mk: Vec<Vec<u8>> = Vec::new();
    for k in 0..3 {
        px[..n].fill(0);
        match k {
            0 => draw_icon(dc, f, z, l, icon_index(m.icon), 0xFFFFFF, 0),
            1 => txt(dc, f.top, 0xFFFFFF, &wide(&m.top), &l.top, DT_LEFT | DT_TOP),
            _ => txt(dc, f.mainf, 0xFFFFFF, &wide(&m.main), &l.mainr, DT_LEFT | DT_TOP),
        }
        unsafe {
            let _ = GdiFlush();
        }
        mk.push(px[..n].iter().map(|p| ((p >> 8) & 255) as u8).collect());
    }
    // halo = text coverage grown by ~2 px, then softened twice
    let rr = s(z, 2).max(1);
    let mut halo = vec![0u8; n];
    for y in 0..l.h {
        for x in 0..l.w {
            let mut best = 0u8;
            for dy in -rr..=rr {
                for dx in -rr..=rr {
                    let (xx, yy) = (x + dx, y + dy);
                    if xx < 0 || yy < 0 || xx >= l.w || yy >= l.h {
                        continue;
                    }
                    let j = (yy * l.w + xx) as usize;
                    best = best.max(mk[0][j]).max(mk[1][j]).max(mk[2][j]);
                }
            }
            halo[(y * l.w + x) as usize] = best;
        }
    }
    for _ in 0..2 {
        let mut tmp = vec![0u8; n];
        for y in 0..l.h {
            for x in 0..l.w {
                let (mut sum, mut c) = (0u32, 0u32);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (xx, yy) = (x + dx, y + dy);
                        if xx < 0 || yy < 0 || xx >= l.w || yy >= l.h {
                            continue;
                        }
                        sum += halo[(yy * l.w + xx) as usize] as u32;
                        c += 1;
                    }
                }
                tmp[(y * l.w + x) as usize] = (sum / c) as u8;
            }
        }
        halo = tmp;
    }
    for i in 0..n {
        let (mut at, mut sel) = (mk[0][i] as u32, 0usize);
        if mk[1][i] as u32 > at {
            at = mk[1][i] as u32;
            sel = 1;
        }
        if mk[2][i] as u32 > at {
            at = mk[2][i] as u32;
            sel = 2;
        }
        let a_s = halo[i] as u32 * 200 / 255;
        let a = at + a_s * (255 - at) / 255;
        let r = ((col[sel] >> 16) & 255) * at / 255;
        let g = ((col[sel] >> 8) & 255) * at / 255;
        let b = (col[sel] & 255) * at / 255;
        px[i] = (a << 24) | (r << 16) | (g << 8) | b;
    }
}

#[allow(clippy::too_many_arguments)]
fn render(dc: HDC, px: &mut [u32], l: &Lay, f: &Fonts, z: Sz, m: &PopMsg, lk: &Look, barq: i32) {
    unsafe {
        SetBkMode(dc, TRANSPARENT);
    }
    if lk.style == ST_FLOAT {
        render_float(dc, px, l, f, z, m, lk);
        return;
    }
    let n = (l.w * l.h) as usize;
    px[..n].fill(lk.bg);
    match lk.style {
        ST_CARD | ST_TIMER => {
            draw_icon(dc, f, z, l, icon_index(m.icon), lk.state, lk.bg);
            txt(dc, f.top, lk.top, &wide(&m.top), &l.top, DT_LEFT | DT_TOP);
            txt(dc, f.mainf, lk.main, &wide(&m.main), &l.mainr, DT_LEFT | DT_TOP);
            unsafe {
                let _ = GdiFlush();
            }
            if lk.style == ST_TIMER {
                fillrect(px, l.w, l.h, &l.bar, 0x444441);
                let mut fr = l.bar;
                fr.right = fr.left + muldiv(l.w, barq, 1000);
                fillrect(px, l.w, l.h, &fr, lk.state);
            }
        }
        ST_ACCENT => {
            txt(dc, f.top, lk.top, &wide(&m.top), &l.top, DT_LEFT | DT_TOP);
            txt(dc, f.mainf, lk.main, &wide(&m.main), &l.mainr, DT_LEFT | DT_TOP);
            unsafe {
                let _ = GdiFlush();
            }
            fillrect(px, l.w, l.h, &l.strip, lk.state);
        }
        ST_PILL => {
            let mut r = l.line;
            let full = pill_text(m);
            let rest: String = full.chars().skip(m.title.chars().count()).collect();
            txt(dc, f.lineb, lk.main, &wide(&m.title), &r, DT_LEFT | DT_TOP);
            r.left += tw(dc, f.lineb, &m.title, None);
            txt(dc, f.line, lk.top, &wide(&rest), &r, DT_LEFT | DT_TOP);
            unsafe {
                let _ = GdiFlush();
            }
            dot(px, l.w, l.h, &l.dot, lk.state);
        }
        ST_TILE => {
            draw_icon(dc, f, z, l, icon_index(m.icon), lk.state, lk.bg);
            txt(dc, f.tileb, lk.main, &wide(&m.title), &l.title, DT_CENTER | DT_TOP);
            if !m.detail.is_empty() {
                txt(dc, f.tiled, lk.top, &wide(&m.detail), &l.detail, DT_CENTER | DT_TOP);
            }
            unsafe {
                let _ = GdiFlush();
            }
        }
        _ => {}
    }
    shape_alpha(px, l.w, l.h, l.radius);
}

/// A 32-bit top-down DIB section on a fresh memory DC.
pub struct Dib {
    pub dc: HDC,
    bm: HBITMAP,
    old: HGDIOBJ,
    pub bits: *mut u32,
    pub w: i32,
    pub h: i32,
}

impl Dib {
    pub fn new(w: i32, h: i32) -> Option<Dib> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..Default::default() },
                ..Default::default()
            };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let Ok(bm) = CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0) else {
                let _ = DeleteDC(dc);
                return None;
            };
            let old = SelectObject(dc, bm.into());
            Some(Dib { dc, bm, old, bits: bits as *mut u32, w, h })
        }
    }
    pub fn px(&mut self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize) }
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bm.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Draw one popup in a ClipPing look (popup.c `popup_draw`): `scale` % (30..400), `barq` = Timer bar fill 0..1000.
pub fn popup_draw(m: &PopMsg, style: i32, bg: u32, scale: i32, dpi: i32, barq: i32) -> Img {
    let z = Sz { dpi, scale: scale.clamp(30, 400) };
    let lk = look(style, bg, m.color);
    let f = Fonts::new(z, style == ST_FLOAT);
    let mdc = unsafe { CreateCompatibleDC(None) };
    let l = layout(mdc, &f, z, m, style);
    unsafe {
        let _ = DeleteDC(mdc);
    }
    let Some(mut dib) = Dib::new(l.w.max(1), l.h.max(1)) else { return Img { w: 0, h: 0, px: vec![] } };
    let dc = dib.dc;
    {
        let px = dib.px();
        render(dc, px, &l, &f, z, m, &lk, barq);
    }
    Img { w: l.w, h: l.h, px: dib.px().to_vec() }
}

// ---------------------------------------------------------------- the status icon (status.c)

const TILE: i32 = 20;
const S_RADIUS: i32 = 5;
pub const S_GAP: i32 = 4;
pub const S_MARGIN: i32 = 6;
const S_DOT: i32 = 8;
const GLYPH_PX: i32 = 12;
const C_FILL: u32 = 0x1E1E1D;
const A_FILL: u32 = 217;
const C_EDGE: u32 = 0x444441;
const C_ICON: u32 = 0xF1EFE8;
const C_RED: u32 = 0xE24B4A;
const G_HISTORY: u16 = 0xE81C;

fn rr_cover(x: i32, y: i32, x0: i32, y0: i32, x1: i32, y1: i32, r: i32) -> i32 {
    let mut n = 0;
    for sy in 0..4 {
        for sx in 0..4 {
            let (px, py) = (x * 8 + 2 * sx + 1, y * 8 + 2 * sy + 1);
            if px < x0 * 8 || px >= x1 * 8 || py < y0 * 8 || py >= y1 * 8 {
                continue;
            }
            let dx = if px < (x0 + r) * 8 {
                (x0 + r) * 8 - px
            } else if px > (x1 - r) * 8 {
                px - (x1 - r) * 8
            } else {
                0
            };
            let dy = if py < (y0 + r) * 8 {
                (y0 + r) * 8 - py
            } else if py > (y1 - r) * 8 {
                py - (y1 - r) * 8
            } else {
                0
            };
            if dx * dx + dy * dy <= r * r * 64 {
                n += 1;
            }
        }
    }
    n
}

/// source-over of a straight colour with coverage a (0..255) onto a premultiplied pixel
fn over(d: u32, col: u32, a: u32) -> u32 {
    let da = d >> 24;
    let r = (((col >> 16) & 255) * a + ((d >> 16) & 255) * (255 - a)) / 255;
    let g = (((col >> 8) & 255) * a + ((d >> 8) & 255) * (255 - a)) / 255;
    let b = ((col & 255) * a + (d & 255) * (255 - a)) / 255;
    let da = a + da * (255 - a) / 255;
    (da << 24) | (r << 16) | (g << 8) | b
}

fn sc(v: i32, dpi: i32) -> i32 {
    muldiv(v, dpi, 96)
}

fn glyph_mask(t: i32, dpi: i32) -> Vec<u8> {
    let mut m = vec![0u8; (t * t) as usize];
    let Some(mut dib) = Dib::new(t, t) else { return m };
    let mut face: Vec<u16> = wide(icon_face());
    face.push(0);
    unsafe {
        let f = CreateFontW(-sc(GLYPH_PX, dpi), 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_TT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, DEFAULT_PITCH.0 as u32, PCWSTR(face.as_ptr()));
        let of = SelectObject(dib.dc, f.into());
        SetBkMode(dib.dc, TRANSPARENT);
        SetTextColor(dib.dc, COLORREF(0xFFFFFF));
        let mut rc = RECT { left: 0, top: 0, right: t, bottom: t };
        let mut g = [G_HISTORY];
        DrawTextW(dib.dc, &mut g, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        let _ = GdiFlush();
        for (i, p) in dib.px().iter().enumerate() {
            m[i] = ((p >> 8) & 255) as u8;
        }
        SelectObject(dib.dc, of);
        let _ = DeleteObject(f.into());
    }
    m
}

fn draw_tile(px: &mut [u32], w: i32, y0: i32, replay: bool, dpi: i32, mask: &[u8]) {
    let t = sc(TILE, dpi);
    let r = sc(S_RADIUS, dpi);
    let b = sc(1, dpi).max(1);
    for y in 0..t {
        for x in 0..t {
            let co = rr_cover(x, y, 0, 0, t, t, r);
            let ci = rr_cover(x, y, b, b, t - b, t - b, (r - b).max(0));
            if co == 0 {
                continue;
            }
            let i = ((y0 + y) * w + x) as usize;
            px[i] = over(px[i], C_FILL, A_FILL * ci as u32 / 16);
            if co > ci {
                px[i] = over(px[i], C_EDGE, 255 * (co - ci) as u32 / 16);
            }
        }
    }
    if replay {
        for y in 0..t {
            for x in 0..t {
                let a = mask[(y * t + x) as usize];
                if a > 0 {
                    let i = ((y0 + y) * w + x) as usize;
                    px[i] = over(px[i], C_ICON, a as u32);
                }
            }
        }
    } else {
        let d = sc(S_DOT, dpi);
        let c8 = t * 4;
        for y in 0..t {
            for x in 0..t {
                let mut n = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let dx = x * 8 + 2 * sx + 1 - c8;
                        let dy = y * 8 + 2 * sy + 1 - c8;
                        if dx * dx + dy * dy <= d * d * 16 {
                            n += 1;
                        }
                    }
                }
                if n > 0 {
                    let i = ((y0 + y) * w + x) as usize;
                    px[i] = over(px[i], C_RED, 255 * n / 16);
                }
            }
        }
    }
}

/// The status icon's tiles: recording (red dot) on top, instant replay (history glyph) below, at the corner.
pub fn status_draw(replay: bool, rec: bool, dpi: i32) -> Img {
    let t = sc(TILE, dpi);
    let n = replay as i32 + rec as i32;
    let g = sc(S_GAP, dpi);
    let mask = glyph_mask(t, dpi);
    let w = t;
    let h = n * t + if n > 1 { g } else { 0 };
    let mut px = vec![0u32; (w * h).max(0) as usize];
    let mut y = 0;
    if rec {
        draw_tile(&mut px, w, y, false, dpi, &mask);
        y += t + g;
    }
    if replay {
        draw_tile(&mut px, w, y, true, dpi, &mask);
    }
    Img { w, h, px }
}

/// An image composited onto an opaque backdrop (for test pictures): premultiplied over solid.
pub fn on_backdrop(img: &Img, backdrop: u32, pad: i32) -> Img {
    let (w, h) = (img.w + pad * 2, img.h + pad * 2);
    let mut px = vec![0xFF00_0000 | backdrop; (w * h) as usize];
    for y in 0..img.h {
        for x in 0..img.w {
            let s = img.px[(y * img.w + x) as usize];
            let a = s >> 24;
            let d = backdrop;
            let r = (((s >> 16) & 255) + ((d >> 16) & 255) * (255 - a) / 255).min(255);
            let g = (((s >> 8) & 255) + ((d >> 8) & 255) * (255 - a) / 255).min(255);
            let b = ((s & 255) + (d & 255) * (255 - a) / 255).min(255);
            px[((y + pad) * w + x + pad) as usize] = 0xFF00_0000 | r << 16 | g << 8 | b;
        }
    }
    Img { w, h, px }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn muldiv_rounds_like_windows() {
        assert_eq!(muldiv(10, 120, 96), 13);
        assert_eq!(muldiv(12, 96 * 150, 96 * 100), 18);
        assert_eq!(muldiv(-5, 3, 2), -8);
    }

    #[test]
    fn every_look_draws_with_a_shape() {
        let m = PopMsg::sample();
        for st in [ST_CARD, ST_PILL, ST_ACCENT, ST_TIMER, ST_TILE, ST_FLOAT] {
            let i = popup_draw(&m, st, 0x2C2C2A, 100, 96, 1000);
            assert!(i.w > 60 && i.h > 20, "{st}: {}x{}", i.w, i.h);
            assert!(i.px.iter().any(|p| p >> 24 != 0));
            if st != ST_FLOAT {
                // rounded corners: the corner pixel is see-through, the middle opaque
                assert_eq!(i.px[0] >> 24, 0, "{st}");
                assert_eq!(i.px[(i.h / 2 * i.w + i.w / 2) as usize] >> 24, 255, "{st}");
            }
        }
        let s = status_draw(true, true, 96);
        assert_eq!((s.w, s.h), (20, 44));
    }
}
