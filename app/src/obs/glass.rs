//! The new Glass popup look (Order 035, the default): the menu's own Liquid glass - its glass style's numbers (Settings ›
//! Glass style; Liquid = tint rgba(20,20,26,.22) over the real desktop blurred 13 px, saturate 170 %, brightness 1.04,
//! rim inset 1 px white .22) - in the drawing's notification shape (#ctoast: radius 12, padding 10 18 10 12, gap 12,
//! title 600 13/17, line 12/16 at --fg2, shadow 0 0 0 .5px rgba(0,0,0,.5) + 0 14px 36px rgba(0,0,0,.4)) with ClipPing's
//! icon glyph in the drawing's state colours. On screen it is a composition window like the menu (comp.rs); for test
//! pictures the same drawing goes over a sample desktop with Skia's copy of the glass recipe.

use bu_obs::engine::{Color, PopMsg};
use skia_safe as sk;

use crate::gfx::{sh, Align, CssColor, Font, Gfx, Rgba, Shadow};
use crate::png::Pixels;

pub const RADIUS: f32 = 12.0;
const PAD_L: f32 = 12.0;
const PAD_R: f32 = 18.0;
const PAD_V: f32 = 10.0;
const GAP: f32 = 12.0;
const ICON: f32 = 22.0;
const LH_MAIN: f32 = 17.0;
const LH_TOP: f32 = 16.0;
/// shadow room around the box (CSS px): 1.5 x the 36 px blur, moved 14 px down
pub const M_L: f32 = 54.0;
pub const M_R: f32 = 54.0;
pub const M_T: f32 = 40.0;
pub const M_B: f32 = 68.0;

const SHADOWS: [Shadow; 2] = [sh(0.0, 0.0, 0.0, 0.5, Rgba::rgba(0, 0, 0, 0.5)), sh(0.0, 14.0, 36.0, 0.0, Rgba::rgba(0, 0, 0, 0.4))];
/// light glass: the same shadow, as light as the menu's own light shadow is against its dark one (.6 -> .2, .45 -> .28)
const SHADOWS_LIGHT: [Shadow; 2] = [sh(0.0, 0.0, 0.0, 0.5, Rgba::rgba(0, 0, 0, 0.17)), sh(0.0, 14.0, 36.0, 0.0, Rgba::rgba(0, 0, 0, 0.25))];
const F_MAIN: Font = Font::new(13.0, 600);
const F_TOP: Font = Font::new(12.0, 400);

/// The drawing's colours (dark / light glass).
pub struct Palette {
    pub light: bool,
    pub fg: Rgba,
    pub fg2: Rgba,
    pub tint: Rgba,
    pub rim: Rgba,
    pub green: Rgba,
    pub red: Rgba,
    pub blue: Rgba,
    pub amber: Rgba,
}

impl Palette {
    /// The theme showing now (Settings › Theme): the menu's palette tokens + its glass numbers.
    pub fn current() -> Palette {
        let n = crate::ui::glass_numbers();
        Palette {
            light: crate::ui::is_light(),
            fg: crate::ui::FG(),
            fg2: crate::ui::FG2(),
            tint: Rgba::rgba(n.tint_rgb[0], n.tint_rgb[1], n.tint_rgb[2], n.tint_alpha),
            rim: Rgba::rgba(255, 255, 255, n.highlight_alpha),
            green: crate::ui::GREEN(),
            red: crate::ui::RED(),
            blue: crate::ui::ACC(),
            amber: crate::ui::AMBER(),
        }
    }
    fn state(&self, c: Color) -> Rgba {
        match c {
            Color::Green => self.green,
            Color::Red => self.red,
            Color::Blue => self.blue,
            Color::Amber => self.amber,
            Color::Grey => self.fg2,
        }
    }
}

/// The box size in CSS px.
pub fn measure(g: &Gfx, m: &PopMsg) -> (f32, f32) {
    let tw = g.text_width(&m.main, F_MAIN).max(g.text_width(&m.top, F_TOP));
    let w = (PAD_L + ICON + GAP + tw + PAD_R).ceil();
    let h = PAD_V + LH_MAIN + 1.0 + LH_TOP + PAD_V;
    (w, h)
}

/// Draw the popup's own paint (shadow, tint, rim, icon, text) with the box's top-left at (x, y), CSS px.
pub fn draw(g: &Gfx, m: &PopMsg, x: f32, y: f32, p: &Palette) {
    let (w, h) = measure(g, m);
    g.box_shadows(x, y, w, h, RADIUS, if p.light { &SHADOWS_LIGHT } else { &SHADOWS }, false);
    g.fill_rr(x, y, w, h, RADIUS, p.tint);
    g.inset_ring(x, y, w, h, RADIUS, 1.0, p.rim);
    let cp = char::from_u32(super::gdi::GLYPHS[super::gdi::icon_index(m.icon)].0 as u32).unwrap_or(' ');
    // an icon font's glyph fills its em box above the baseline: centred = baseline half an em below the middle
    g.glyph(super::gdi::icon_face(), cp, ICON - 2.0, x + PAD_L + 1.0, y + h / 2.0 + (ICON - 2.0) / 2.0, p.state(m.color));
    let tx = x + PAD_L + ICON + GAP;
    let maxw = w - (tx - x) - PAD_R + 1.0;
    g.text(&m.main, F_MAIN, tx, y + PAD_V, LH_MAIN, p.fg, Align::Left, maxw);
    g.text(&m.top, F_TOP, tx, y + PAD_V + LH_MAIN + 1.0, LH_TOP, p.fg2, Align::Left, maxw);
}

/// A sample desktop for the test pictures: a soft colourful wallpaper with a few shapes (so the blur and the colour boost
/// show).
pub fn sample_desktop(w: i32, h: i32) -> Option<sk::Image> {
    let mut s = crate::gfx::new_surface(w, h)?;
    let c = s.canvas();
    let mut p = sk::Paint::default();
    p.set_shader(sk::gradient_shader::linear(
        (sk::Point::new(0.0, 0.0), sk::Point::new(w as f32, h as f32)),
        sk::gradient_shader::GradientShaderColors::Colors(&[sk::Color::from_rgb(0x1d, 0x3b, 0x6e), sk::Color::from_rgb(0x6a, 0x2c, 0x70), sk::Color::from_rgb(0xd8, 0x6b, 0x3a)]),
        None,
        sk::TileMode::Clamp,
        None,
        None,
    ));
    c.draw_rect(sk::Rect::from_wh(w as f32, h as f32), &p);
    let mut q = sk::Paint::default();
    q.set_anti_alias(true);
    for (i, (cx, cy, r, col)) in [(0.2f32, 0.3f32, 0.18f32, 0xFF3F_D0C9_u32), (0.75, 0.6, 0.22, 0xFFF5_D76E), (0.5, 0.85, 0.12, 0xFFFF_FFFF), (0.9, 0.15, 0.1, 0xFF2B_2B2B)].iter().enumerate() {
        q.set_color(sk::Color::new(*col));
        q.set_alpha(if i == 2 { 200 } else { 255 });
        c.draw_circle((cx * w as f32, cy * h as f32), r * h as f32, &q);
    }
    Some(s.image_snapshot())
}

/// A test picture: the popup over the sample desktop (glass recipe by Skia), `pad` CSS px around the box.
pub fn picture(m: &PopMsg, scale: f32) -> Option<Pixels> {
    let n = crate::ui::glass_numbers();
    let g = Gfx::new(scale);
    let (bw, bh) = measure(&g, m);
    let (pw, ph) = (bw + 2.0 * 24.0, bh + 24.0 + 40.0);
    let (w, h) = ((pw * scale).ceil() as i32, (ph * scale).ceil() as i32);
    let desk = sample_desktop(w, h)?;
    let mut s = crate::gfx::new_surface(w, h)?;
    g.begin(s.canvas());
    g.draw_image_rect(&desk, 0.0, 0.0, pw, ph);
    g.backdrop(&desk, 24.0, 24.0, bw, bh, RADIUS, n.blur_px, &[CssColor::Saturate(n.saturate), CssColor::Brightness(n.brightness)]);
    draw(&g, m, 24.0, 24.0, &Palette::current());
    g.end();
    Some(crate::png::from_surface(&mut s))
}
