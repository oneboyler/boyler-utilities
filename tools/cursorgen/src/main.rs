//! cursorgen (Order 040) - builds the app's own "Glass" cursor set. Not shipped: a dev tool outside the main workspace.
//!
//! Reads one SVG per Windows cursor role from `app/assets/cursors/glass/svg/` (32-unit viewBox; the hotspot in the root's
//! `data-hotspot="x y"`, in the same units), renders every size Windows uses (32 / 48 / 64 / 96 / 128 px = 100-300 %
//! scaling and the bigger size steps) and writes `<role>.cur` (one file, every size) or, for the two animated roles,
//! `<role>.ani` (RIFF ACON: anih + LIST fram of icon chunks, each frame a multi-size cursor) into
//! `app/assets/cursors/glass/`. Every image is stored as PNG (Windows reads PNG entries in cursors since Vista). Measured
//! (bu-mouse tests/glass.rs): when one .cur MIXES a 32-bit BMP entry with PNG entries, LoadImageW takes the BMP for every
//! size asked and scales it (48 / 64 / 96 / 128 came out as the 32 px image scaled); with PNG only it picks each size
//! exactly - that test loads every file through LoadImageW at every size and compares what Windows draws with the files.
//!
//! Animated roles: an element with `data-spin="cx cy r min max"` is the spinner arc (a stroked circle of radius r): per
//! frame the tool sets its `stroke-dasharray` (arc length breathing between min and max of the circumference) and its
//! `transform` (one full turn per loop).
//!
//! Animations: 18 frames at 3 jiffies (50 ms) = one turn in 0.9 s, the same frame count and rate as Windows' own
//! aero_busy.ani.
//!
//! Usage (from the repo root, Git Bash; CARGO_TARGET_DIR = the shared target folder):
//!   cargo run --release --manifest-path tools/cursorgen/Cargo.toml -- [--sheet <png>] [--closeup <png>] [--check <dir>]
//!       [--tiles <dir>] [--probe <role>] [--partial]
//! `--sheet` also writes the preview picture of the whole set (dark and light halves), `--closeup` the 32 px close-up,
//! `--check` every role over busy backgrounds, `--tiles` one picture per role (judging the glass while drawing), `--probe`
//! prints a 128 px render's pixels; `--partial` skips roles that have no SVG yet (never for the files that ship).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;

/// Every size Windows asks for (100 / 150 / 200 / 300 % scaling and the size slider's steps up to 128).
pub const SIZES: [u32; 5] = [32, 48, 64, 96, 128];

#[derive(Clone, Copy)]
struct Anim {
    frames: u32,
    /// display time of one frame in jiffies (1/60 s)
    jiffies: u32,
}

/// (file stem = `WinRole::reg_name()` lower-case, Windows' name for the role, animation)
const ROLES: [(&str, &str, Option<Anim>); 17] = [
    ("arrow", "Normal select", None),
    ("help", "Help select", None),
    ("appstarting", "Working in background", Some(Anim { frames: 18, jiffies: 3 })),
    ("wait", "Busy", Some(Anim { frames: 18, jiffies: 3 })),
    ("crosshair", "Precision select", None),
    ("ibeam", "Text select", None),
    ("nwpen", "Handwriting", None),
    ("no", "Unavailable", None),
    ("sizens", "Vertical resize", None),
    ("sizewe", "Horizontal resize", None),
    ("sizenwse", "Diagonal resize 1", None),
    ("sizenesw", "Diagonal resize 2", None),
    ("sizeall", "Move", None),
    ("uparrow", "Alternate select", None),
    ("hand", "Link select", None),
    ("pin", "Location select", None),
    ("person", "Person select", None),
];

fn main() {
    let root = repo_root();
    let svg_dir = root.join("app/assets/cursors/glass/svg");
    let out_dir = root.join("app/assets/cursors/glass");
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(PathBuf::from);

    // `--partial`: skip roles without an SVG yet (while drawing); never for the files that ship
    let partial = args.iter().any(|a| a == "--partial");
    let opt = usvg::Options::default();
    let mut sets: Vec<Rendered> = Vec::new();
    for (stem, label, anim) in ROLES {
        let path = svg_dir.join(format!("{stem}.svg"));
        if partial && !path.is_file() {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{stem}.svg: {e}"));
        let hot = hotspot(&src).unwrap_or_else(|| panic!("{stem}.svg has no data-hotspot"));
        let frames: Vec<String> = match anim {
            None => vec![src.clone()],
            Some(a) => (0..a.frames).map(|i| frame_svg(&src, i as f32 / a.frames as f32)).collect(),
        };
        // frames[f][size index]
        let images: Vec<Vec<Pixmap>> = frames.iter().map(|s| SIZES.iter().map(|px| render(s, *px, &opt)).collect()).collect();
        let curs: Vec<Vec<u8>> = images.iter().map(|imgs| cur_bytes(imgs, hot)).collect();
        let (name, bytes) = match anim {
            None => (format!("{stem}.cur"), curs[0].clone()),
            Some(a) => (format!("{stem}.ani"), ani_bytes(&curs, a.jiffies)),
        };
        std::fs::write(out_dir.join(&name), &bytes).expect("write cursor");
        println!("{name:16} {:>7} bytes  hotspot {:?} at 32 px", bytes.len(), hot_px(hot, 32));
        sets.push(Rendered { stem, label, images });
    }

    // `--probe <stem>`: prints the straight RGBA of the 128 px render along its middle row (checking the glass's alpha)
    if let Some(stem) = args.iter().position(|a| a == "--probe").and_then(|i| args.get(i + 1)) {
        if let Some(r) = sets.iter().find(|r| r.stem == stem) {
            let pm = &r.images[0][4];
            let px = rgba(pm);
            for y in (8..128).step_by(12) {
                let row: Vec<String> = (0..128).step_by(6).map(|x| px[y * 128 + x]).filter(|p| p[3] > 0).map(|p| format!("{:02x}{:02x}{:02x}/{:02x}", p[0], p[1], p[2], p[3])).collect();
                println!("y{y:3}: {}", row.join(" "));
            }
        }
    }
    if let Some(p) = arg("--sheet") {
        write_png(&p, &sheet(&sets));
        println!("sheet -> {}", p.display());
    }
    if let Some(p) = arg("--closeup") {
        write_png(&p, &closeup(&sets));
        println!("close-up -> {}", p.display());
    }
    if let Some(d) = arg("--tiles") {
        for r in &sets {
            write_png(&d.join(format!("tile-{}.png", r.stem)), &tile(r, 0));
            if r.images.len() > 1 {
                write_png(&d.join(format!("tile-{}-f5.png", r.stem)), &tile(r, 5));
            }
        }
    }
    if let Some(d) = arg("--check") {
        std::fs::create_dir_all(&d).ok();
        let p = d.join("busy-backgrounds.png");
        write_png(&p, &busy(&sets));
        println!("check -> {}", p.display());
    }
}

struct Rendered {
    stem: &'static str,
    label: &'static str,
    /// [frame][size index]
    images: Vec<Vec<Pixmap>>,
}

fn repo_root() -> PathBuf {
    // tools/cursorgen -> repo root
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

// ------------------------------------------------------------------------------------------------ SVG

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let i = tag.find(&key)? + key.len();
    let j = tag[i..].find('"')? + i;
    Some(&tag[i..j])
}

fn set_attr(tag: &str, name: &str, value: &str) -> String {
    let key = format!(" {name}=\"");
    match tag.find(&key) {
        Some(i) => {
            let s = i + key.len();
            let e = tag[s..].find('"').map(|j| j + s).expect("attribute end");
            format!("{}{}{}", &tag[..s], value, &tag[e..])
        }
        None => {
            let end = if tag.ends_with("/>") { tag.len() - 2 } else { tag.len() - 1 };
            format!("{} {name}=\"{value}\"{}", &tag[..end], &tag[end..])
        }
    }
}

/// The root's `data-hotspot="x y"` (32-unit grid).
fn hotspot(svg: &str) -> Option<(f32, f32)> {
    let s = svg.find("<svg")?;
    let tag = &svg[s..s + svg[s..].find('>')? + 1];
    let v: Vec<f32> = attr(tag, "data-hotspot")?.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    (v.len() == 2).then(|| (v[0], v[1]))
}

/// The hotspot pixel at `px`: the pixel that holds the hotspot point.
pub fn hot_px(h: (f32, f32), px: u32) -> (u16, u16) {
    let k = px as f32 / 32.0;
    ((h.0 * k).floor() as u16, (h.1 * k).floor() as u16)
}

/// One animation frame (t = 0..1 over the loop): every `data-spin` element gets its arc length and turn.
fn frame_svg(src: &str, t: f32) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(i) = rest.find("data-spin=\"") {
        let start = rest[..i].rfind('<').expect("tag start");
        let end = i + rest[i..].find('>').expect("tag end") + 1;
        out.push_str(&rest[..start]);
        let tag = &rest[start..end];
        let v: Vec<f32> = attr(tag, "data-spin").unwrap().split_whitespace().filter_map(|x| x.parse().ok()).collect();
        let (cx, cy, r, lo, hi) = (v[0], v[1], v[2], v[3], v[4]);
        let circ = 2.0 * std::f32::consts::PI * r;
        let tau = 2.0 * std::f32::consts::PI;
        // the arc breathes once per loop and turns once; its middle turns evenly. Frame 0 shows it longest (the still
        // picture Windows and the app show of an animated cursor is its first frame)
        let len = circ * (lo + (hi - lo) * (0.5 + 0.5 * (tau * t).cos()));
        let rot = -90.0 + 360.0 * t - (len / circ) * 180.0;
        let tag = set_attr(tag, "stroke-dasharray", &format!("{len:.3} {:.3}", circ * 2.0));
        let tag = set_attr(&tag, "transform", &format!("rotate({rot:.3} {cx} {cy})"));
        out.push_str(&tag);
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn render(svg: &str, px: u32, opt: &usvg::Options) -> Pixmap {
    let tree = usvg::Tree::from_str(svg, opt).expect("svg parse");
    let k = px as f32 / tree.size().width();
    let mut pm = Pixmap::new(px, px).unwrap();
    resvg::render(&tree, Transform::from_scale(k, k), &mut pm.as_mut());
    pm
}

// ------------------------------------------------------------------------------------------------ .cur / .ani

/// Straight (not premultiplied) RGBA rows, top first.
fn rgba(pm: &Pixmap) -> Vec<[u8; 4]> {
    pm.pixels().iter().map(|p| {
        let c = p.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    }).collect()
}

fn png_bytes(w: u32, h: u32, px: &[[u8; 4]]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.set_compression(png::Compression::Best);
        e.set_adaptive_filter(png::AdaptiveFilterType::Adaptive);
        let mut wr = e.write_header().unwrap();
        let flat: Vec<u8> = px.iter().flatten().copied().collect();
        wr.write_image_data(&flat).unwrap();
    }
    out
}


/// A .cur file holding every size (largest first, like Windows' own aero cursors).
fn cur_bytes(imgs: &[Pixmap], hot: (f32, f32)) -> Vec<u8> {
    let mut entries: Vec<(u32, (u16, u16), Vec<u8>)> = imgs
        .iter()
        .map(|pm| {
            let (w, h) = (pm.width(), pm.height());
            let px = rgba(pm);
            let data = png_bytes(w, h, &px);
            (w, hot_px(hot, w), data)
        })
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 2, 0]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    let mut off = 6 + 16 * entries.len() as u32;
    for (w, (hx, hy), data) in &entries {
        let b = if *w >= 256 { 0 } else { *w as u8 };
        out.extend_from_slice(&[b, b, 0, 0]);
        out.extend_from_slice(&hx.to_le_bytes());
        out.extend_from_slice(&hy.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&off.to_le_bytes());
        off += data.len() as u32;
    }
    for (_, _, data) in &entries {
        out.extend_from_slice(data);
    }
    out
}

fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 9);
    out.extend_from_slice(id);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

/// RIFF 'ACON': anih (36 bytes; AF_ICON, frames shown in order at one rate) + LIST 'fram' of 'icon' chunks.
fn ani_bytes(frames: &[Vec<u8>], jiffies: u32) -> Vec<u8> {
    let n = frames.len() as u32;
    let mut anih = Vec::new();
    for v in [36u32, n, n, 0, 0, 0, 0, jiffies, 1 /* AF_ICON */] {
        anih.extend_from_slice(&v.to_le_bytes());
    }
    let mut fram = b"fram".to_vec();
    for f in frames {
        fram.extend(chunk(b"icon", f));
    }
    let mut body = b"ACON".to_vec();
    body.extend(chunk(b"anih", &anih));
    body.extend(chunk(b"LIST", &fram));
    chunk(b"RIFF", &body)
}

// ------------------------------------------------------------------------------------------------ pictures

fn write_png(p: &Path, pm: &Pixmap) {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).ok();
    }
    std::fs::write(p, png_bytes(pm.width(), pm.height(), &rgba(pm))).expect("write png");
}

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len() * 4 / 3 + 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

fn img(pm: &Pixmap, x: f32, y: f32, scale: f32) -> String {
    let uri = b64(&png_bytes(pm.width(), pm.height(), &rgba(pm)));
    format!(
        r#"<image x="{x}" y="{y}" width="{w}" height="{h}" image-rendering="optimizeSpeed" href="data:image/png;base64,{uri}"/>"#,
        w = pm.width() as f32 * scale,
        h = pm.height() as f32 * scale
    )
}

fn text(x: f32, y: f32, size: f32, weight: u32, fill: &str, anchor: &str, s: &str) -> String {
    format!(r#"<text x="{x}" y="{y}" font-family="Segoe UI" font-size="{size}" font-weight="{weight}" fill="{fill}" text-anchor="{anchor}">{s}</text>"#)
}

fn render_page(svg: &str) -> Pixmap {
    let mut opt = usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(svg, &opt).expect("sheet svg");
    let s = tree.size();
    let mut pm = Pixmap::new(s.width() as u32, s.height() as u32).unwrap();
    resvg::render(&tree, Transform::identity(), &mut pm.as_mut());
    pm
}

/// The whole set: a dark half and a light half, each with every role at 96 px (200 % scaling) shown 1:1 and labelled, the
/// two animations as frame strips (64 px), and every role at 48 px (150 %) and 32 px (100 %, what most people see).
fn sheet(sets: &[Rendered]) -> Pixmap {
    const W: f32 = 1580.0;
    const HALF: f32 = 790.0;
    let mut s = String::new();
    let _ = write!(s, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{}">"#, HALF * 2.0);
    for (hi, (bg, fg, fg2, name)) in [("#1c1d22", "#f5f5f7", "#8e8e96", "on dark"), ("#f3f3f5", "#1d1d1f", "#6e6e76", "on light")].iter().enumerate() {
        let oy = hi as f32 * HALF;
        let _ = write!(s, r#"<rect x="0" y="{oy}" width="{W}" height="{HALF}" fill="{bg}"/>"#);
        s += &text(40.0, oy + 52.0, 26.0, 600, fg, "start", &format!("Glass cursors \u{2014} {name}"));
        s += &text(40.0, oy + 80.0, 14.0, 400, fg2, "start", "Boyler Utilities \u{b7} all 17 Windows cursor roles \u{b7} 96 px (200 % scaling) shown 1:1");
        // 17 roles at 96 px: 9 + 8 per row
        let cell = 168.0;
        for (i, r) in sets.iter().enumerate() {
            let (row, col) = (i / 9, i % 9);
            let x = 40.0 + col as f32 * cell;
            let y = oy + 104.0 + row as f32 * 172.0;
            s += &img(&r.images[0][3], x + (cell - 96.0) / 2.0 - 10.0, y, 1.0);
            s += &text(x + cell / 2.0 - 10.0, y + 118.0, 13.0, 600, fg, "middle", r.label);
            s += &text(x + cell / 2.0 - 10.0, y + 136.0, 11.0, 400, fg2, "middle", &format!("{}.{}", r.stem, if r.images.len() > 1 { "ani" } else { "cur" }));
        }
        // the animations: every 2nd frame at 64 px
        let y = oy + 468.0;
        for (k, r) in sets.iter().filter(|r| r.images.len() > 1).enumerate() {
            let x0 = 40.0 + k as f32 * 740.0;
            s += &text(x0, y, 13.0, 600, fg, "start", &format!("{} \u{b7} {} frames, 50 ms each (every 2nd shown, 64 px)", r.label, r.images.len()));
            for f in 0..r.images.len() / 2 {
                s += &img(&r.images[f * 2][2], x0 + f as f32 * 74.0, y + 12.0, 1.0);
            }
        }
        // every role at 48 px and at 32 px, 1:1
        let y = oy + 590.0;
        s += &text(40.0, y, 13.0, 600, fg, "start", "48 px (150 %), 1:1");
        for (i, r) in sets.iter().enumerate() {
            s += &img(&r.images[0][1], 40.0 + i as f32 * 64.0, y + 14.0, 1.0);
        }
        let y = oy + 690.0;
        s += &text(40.0, y, 13.0, 600, fg, "start", "32 px (100 %), 1:1");
        for (i, r) in sets.iter().enumerate() {
            s += &img(&r.images[0][0], 40.0 + i as f32 * 64.0 + 8.0, y + 14.0, 1.0);
        }
    }
    s += "</svg>";
    render_page(&s)
}

/// 32 px renders zoomed 6x (nearest), on dark and light: what one pixel of the real 100 % cursor looks like.
fn closeup(sets: &[Rendered]) -> Pixmap {
    let z = 6.0;
    let cell = 32.0 * z + 16.0;
    let cols = 9.0;
    let rows = 2.0;
    let w = 20.0 + cols * cell;
    let h_half = 20.0 + rows * cell;
    let mut s = String::new();
    let _ = write!(s, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{}">"#, h_half * 2.0);
    for (hi, bg) in ["#1c1d22", "#f3f3f5"].iter().enumerate() {
        let oy = hi as f32 * h_half;
        let _ = write!(s, r#"<rect x="0" y="{oy}" width="{w}" height="{h_half}" fill="{bg}"/>"#);
        for (i, r) in sets.iter().enumerate() {
            let (row, col) = ((i / 9) as f32, (i % 9) as f32);
            s += &img(&r.images[0][0], 20.0 + col * cell, oy + 20.0 + row * cell, z);
        }
    }
    s += "</svg>";
    render_page(&s)
}

/// Every role at 32 and 64 px over busy backgrounds (a photo-like gradient, mid grey, a text page) - for judging only.
fn busy(sets: &[Rendered]) -> Pixmap {
    let w = 40.0 + 17.0 * 76.0;
    let mut s = String::new();
    let _ = write!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="420"><defs><linearGradient id="g" x1="0" x2="1"><stop offset="0" stop-color="#1b4f9c"/><stop offset=".35" stop-color="#e2793a"/><stop offset=".7" stop-color="#3a8f4c"/><stop offset="1" stop-color="#d9d2c4"/></linearGradient></defs><rect width="{w}" height="140" fill="url(#g)"/><rect y="140" width="{w}" height="140" fill="#808080"/><rect y="280" width="{w}" height="140" fill="#ffffff"/>"##
    );
    for i in 0..9 {
        s += &text(20.0, 300.0 + i as f32 * 14.0, 12.0, 400, "#333", "start", "The quick brown fox jumps over the lazy dog. Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation");
    }
    for band in 0..3 {
        for (i, r) in sets.iter().enumerate() {
            let x = 20.0 + i as f32 * 76.0;
            let y = band as f32 * 140.0;
            s += &img(&r.images[0][0], x + 16.0, y + 10.0, 1.0);
            s += &img(&r.images[0][2], x, y + 56.0, 1.0);
        }
    }
    s += "</svg>";
    render_page(&s)
}

/// One role up close (for judging while drawing): on dark, mid grey, light and a colour gradient - 32 px 1:1, 32 px
/// zoomed 6x, 64 px 1:1, 128 px 1:1.
fn tile(r: &Rendered, frame: usize) -> Pixmap {
    let w = 20.0 + 32.0 + 20.0 + 192.0 + 20.0 + 64.0 + 20.0 + 128.0 + 20.0;
    let hh = 212.0;
    let mut s = String::new();
    let _ = write!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{}"><defs><linearGradient id="g" x1="0" x2="1"><stop offset="0" stop-color="#1b4f9c"/><stop offset=".35" stop-color="#e2793a"/><stop offset=".7" stop-color="#3a8f4c"/><stop offset="1" stop-color="#d9d2c4"/></linearGradient></defs>"##,
        hh * 4.0
    );
    for (i, bg) in ["#1c1d22", "#808080", "#f3f3f5", "url(#g)"].iter().enumerate() {
        let oy = i as f32 * hh;
        let _ = write!(s, r#"<rect x="0" y="{oy}" width="{w}" height="{hh}" fill="{bg}"/>"#);
        let im = &r.images[frame];
        s += &img(&im[0], 20.0, oy + 10.0, 1.0);
        s += &img(&im[0], 72.0, oy + 10.0, 6.0);
        s += &img(&im[2], 284.0, oy + 10.0, 1.0);
        s += &img(&im[4], 368.0, oy + 10.0, 1.0);
    }
    s += "</svg>";
    render_page(&s)
}
