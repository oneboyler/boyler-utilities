//! The element tree every page and piece is built from (Order 014): one `El` = one box of the drawing's HTML, with the
//! drawing's CSS numbers. Layout = taffy (CSS flexbox / grid / block with the web's rules), paint = gfx.rs (Blink's paint
//! rules), so a page written from the drawing's structure and numbers is exact by construction.
//! Pages rebuild their tree whenever something changed (it is cheap: a page is a few hundred boxes) - no retained widget
//! objects, no hidden state in pieces: the page owns the values, the `Cx` owns hover / press / transitions by key.

use std::rc::Rc;

use taffy::prelude::*;
use taffy::style::{AlignContent, AlignItems, AlignSelf, JustifyContent, Overflow, Position};

use crate::gfx::{Align, Font, Gfx, Rgba, Shadow};

/// An element's identity across rebuilds (hover, press, transitions, clicks are tied to it). Made from a string.
pub type Key = u64;

/// A key from a name (FNV-1a).
pub const fn key(s: &str) -> Key {
    let b = s.as_bytes();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < b.len() {
        h ^= b[i] as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
        i += 1;
    }
    h
}

/// A child key: `parent` + a part name (a piece's own sub-elements: a slider's thumb, a popup's items...).
pub fn sub(parent: Key, part: &str) -> Key {
    let mut h = parent ^ 0x9e37_79b9_7f4a_7c15;
    for c in part.bytes() {
        h ^= c as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A child key by index (rows of a list).
pub fn idx(parent: Key, i: usize) -> Key {
    sub(parent, &i.to_string())
}

/// CSS `background`: a colour or a linear gradient (angle in CSS degrees: 90 = to the right, 180 = to the bottom).
#[derive(Clone, Debug)]
pub enum Fill {
    Color(Rgba),
    Linear(f32, Vec<(f32, Rgba)>),
}

/// How text is laid out: one line (`white-space: nowrap`, optionally with `text-overflow: ellipsis`) or wrapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum Wrap {
    Line,
    Ellipsis,
    Wrap,
}

#[derive(Clone, Debug)]
pub struct Text {
    pub s: String,
    pub font: Font,
    pub color: Rgba,
    /// the used line height (LayoutUnits: `lh(size, factor)` or a px value)
    pub lh: f32,
    pub wrap: Wrap,
    pub align: Align,
    pub underline: bool,
}

/// An inline SVG icon from the drawing's ICON table, stroked (or filled) like Blink paints it.
#[derive(Clone, Debug)]
pub struct Icon {
    pub name: String,
    pub size: f32,
    pub stroke_w: f32,
    pub color: Rgba,
    /// extra opacity per SVG class (e.g. the mute cross)
    pub class_op: Vec<(String, f32)>,
    /// Order 025: an `<svg>` box of (w, h) whose viewBox is fitted the SVG way (`xMidYMid meet`), see `El::icon_fit`
    pub fit: Option<(f32, f32)>,
    /// Order 025: parts painted other than the default stroke (filled icons, per-class fill / colour), see `El::icon_paint`
    pub paint: Option<IconPaint>,
}

/// Order 025: an inner list's slim scrollbar (`::-webkit-scrollbar{width:<lane>}` with no buttons and no track rule, the
/// thumb `border:<border>px solid transparent;background-clip:padding-box;border-radius:<radius>px;background-color:<color>`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlimThumb {
    pub lane: f32,
    pub border: f32,
    pub radius: f32,
    pub color: Rgba,
    /// the shortest thumb (CSS px); **unclear** for Chromium's custom scrollbars (not measured) - the lane width
    pub min_len: f32,
}

impl SlimThumb {
    /// `.rsl2` (the reset review list), `.pml` (Controller's action list), `.cpnb` (capture notes): 9 px lane, a 5 px wide
    /// rgba(255,255,255,.3) thumb. "Inner popups (menus, the emoji grid, lists) keep a slim 9 px glass thumb".
    pub const GLASS: SlimThumb = SlimThumb { lane: 9.0, border: 2.0, radius: 5.0, color: Rgba(1.0, 1.0, 1.0, 0.3), min_len: 9.0 };
}

/// Order 025: how an icon's parts are painted beyond the drawing's default `fill:none;stroke:currentColor`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IconPaint {
    /// every part filled with the icon's colour, no stroke (`.pb svg{fill:currentColor}`, `.wbdg svg`)
    pub fill_all: bool,
    /// per SVG class (the drawing's `.f{fill:currentColor;stroke:none}`, `.wfs .on{stroke:currentColor}` `.of{stroke:var(--fg3)}`)
    pub classes: Vec<(String, ClassPaint)>,
}

/// One SVG class's paint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClassPaint {
    /// filled with this colour, no stroke
    Fill(Rgba),
    /// stroked with this colour (instead of the icon's)
    Stroke(Rgba),
}

/// What a box draws inside itself besides its background.
#[derive(Clone)]
pub enum Content {
    None,
    Text(Text),
    Icon(Icon),
    /// own painting into the box's laid-out rect (x, y, w, h); e.g. Audio's live meters
    Paint(Rc<dyn Fn(&Gfx, (f32, f32, f32, f32))>),
}

/// The mouse pointer over an element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Cursor {
    #[default]
    Default,
    Hand,
    Text,
    /// CSS `cursor:grab` / `grabbing` (shown as the hand on Windows)
    Grab,
    Grabbing,
}

#[derive(Clone)]
pub struct El {
    pub key: Option<Key>,
    pub style: Style,
    pub bg: Option<Fill>,
    /// border-radius (one value; `RADIUS_PILL` = 50 %)
    pub radius: f32,
    pub shadows: Vec<Shadow>,
    pub insets: Vec<Shadow>,
    /// a solid border (CSS `border`) drawn inside the box: width, colour
    pub border: Option<(f32, Rgba, bool)>,
    pub opacity: f32,
    /// CSS transform about the box centre: translate (px) then scale
    pub translate: (f32, f32),
    pub scale: f32,
    /// Order 025: a CSS translate in percent of the box's own size (`translate(-50%)` = (-0.5, 0)), added to `translate`;
    /// the box is laid out (and its parts pixel-snapped) where it stands, then moved - as Blink rasters a transformed layer
    pub translate_pct: (f32, f32),
    /// Order 025: CSS `filter` colour functions on the box AND its subtree (`brightness(1.08)`, `grayscale(1)`), one layer
    pub color_filter: Vec<crate::gfx::CssColor>,
    /// Order 025: the slim scrollbar thumb of an inner scrolling list (`::-webkit-scrollbar` 9 px, no arrows), drawn over
    /// the box's right edge from its ONE child's height and upward move (Lane K's `cx.scroll_box` shape)
    pub slim_thumb: Option<SlimThumb>,
    /// CSS rotate(deg) about the box centre (after the translate and scale)
    pub rotate: f32,
    /// overflow: hidden (children clipped to the padding box, rounded)
    pub clip: bool,
    pub z: i32,
    pub content: Content,
    pub children: Vec<El>,
    pub cursor: Cursor,
    /// pointer-events: none when false
    pub hit: bool,
    /// clicks on this element are delivered to the page as `Ev::Click(key)`
    pub click: bool,
    /// painted in the live pass (moving every frame: meters), not in the cached page layer
    pub live: bool,
    /// a backdrop-filter blur (px) behind the box (popups, the dock label)
    pub backdrop: f32,
    /// Order 025: the drawing's `data-tip` - the shared tip bubble shows this text after a short hover (`pieces::tip`)
    pub tip: Option<Rc<str>>,
    /// Order 045: the tip is the drawing's plain `title="…"` hover name (same bubble, Windows' longer hover delay)
    pub tip_title: bool,
    /// Order 045: a slider's value 0..1 (`slider::slider`): Tab reaches it, the arrows move it, the focus ring circles
    /// its thumb (not painted from here); (value, one arrow step) - the step 0.01 unless the page sets its own
    pub range: Option<(f32, f32)>,
    /// Order 045: the wheel over it steps its value (`Ev::Wheel`) instead of scrolling the page (`El::wheel_steps`)
    pub wheel: bool,
    /// the backdrop-filter's saturate() factor (CSS: 180% = 1.8)
    pub backdrop_sat: f32,
    /// CSS `filter: brightness(b)` on the whole painted box (1 = none)
    pub brightness: f32,
    /// a scrolling box (`overflow-y:auto`, made by `Cx::scroll_box`): the wheel over it moves its content first
    pub scrolls: bool,
    /// CSS `position:sticky; top:<px>` inside the scrolling page
    pub sticky: Option<f32>,
    /// Order 041: what an own-painted box (`El::paint`) draws, as a number (`El::sig`): the same number = the same picture,
    /// so the frame rasters it again only when it changes (ui/damage.rs); None = it counts as changed at every build
    pub paint_sig: Option<u64>,
}

pub const RADIUS_PILL: f32 = 1e6;

/// Blink's used line height for `line-height: <number>`: font-size x number, in LayoutUnits (floored to 1/64 px).
pub fn lh(size: f32, factor: f32) -> f32 {
    (size * factor * 64.0).floor() / 64.0
}

impl Default for El {
    fn default() -> Self {
        El {
            key: None,
            style: Style { display: Display::Block, ..Style::default() },
            bg: None,
            radius: 0.0,
            shadows: Vec::new(),
            insets: Vec::new(),
            border: None,
            opacity: 1.0,
            translate: (0.0, 0.0),
            translate_pct: (0.0, 0.0),
            color_filter: Vec::new(),
            slim_thumb: None,
            scale: 1.0,
            rotate: 0.0,
            clip: false,
            z: 0,
            content: Content::None,
            children: Vec::new(),
            cursor: Cursor::Default,
            hit: true,
            click: false,
            live: false,
            backdrop: 0.0,
            tip: None,
            tip_title: false,
            range: None,
            wheel: false,
            backdrop_sat: 1.0,
            brightness: 1.0,
            scrolls: false,
            sticky: None,
            paint_sig: None,
        }
    }
}

fn lpa(v: f32) -> LengthPercentageAuto {
    LengthPercentageAuto::length(v)
}
fn lp(v: f32) -> LengthPercentage {
    LengthPercentage::length(v)
}

/// The builder: names follow CSS (`w` = width, `gap`, `pad` = padding ...). Every value is a CSS px number from the drawing.
impl El {
    /// `display: block`
    pub fn block() -> El {
        El::default()
    }
    /// `display: flex` (row)
    pub fn row() -> El {
        let mut e = El::default();
        e.style.display = Display::Flex;
        e.style.flex_direction = FlexDirection::Row;
        e
    }
    /// `display: flex; flex-direction: column`
    pub fn col() -> El {
        let mut e = El::row();
        e.style.flex_direction = FlexDirection::Column;
        e
    }
    /// `display: grid`
    pub fn grid() -> El {
        let mut e = El::default();
        e.style.display = Display::Grid;
        e
    }
    /// An inline-level text run as its own box (Blink: an anonymous line box).
    pub fn text(s: impl Into<String>, font: Font, color: Rgba, lh: f32) -> El {
        let s = crate::testmode::demo_text(s.into());
        El { content: Content::Text(Text { s, font, color, lh, wrap: Wrap::Line, align: Align::Left, underline: false }), ..El::default() }
    }
    pub fn icon(name: &str, size: f32, stroke_w: f32, color: Rgba) -> El {
        let mut e = El::default().w(size).h(size).shrink(0.0);
        e.content = Content::Icon(Icon { name: name.to_string(), size, stroke_w, color, class_op: Vec::new(), fit: None, paint: None });
        e
    }
    /// Order 025: a page's own inline `<svg>` (one not in the drawing's ICON table, e.g. Security's shield states or
    /// its drop-zone glyphs): `src` = the drawing's SVG markup word for word (`<svg viewBox=…>…</svg>`), drawn exactly
    /// like an ICON-table icon of `size` px (stroke, round caps / joins, classes, `<g transform="scale">`).
    pub fn icon_svg(src: &str, size: f32, stroke_w: f32, color: Rgba) -> El {
        El::icon(src, size, stroke_w, color)
    }
    /// Order 025: an icon (ICON name or `<svg>` source) in an `<svg>` box of `w` x `h` whose viewBox has another
    /// shape - SVG's default `preserveAspectRatio="xMidYMid meet"`: scaled to fit, centred, the centring offset kept
    /// at sub-pixel precision inside the pixel-snapped box (Blink snaps the `<svg>` box, not its viewBox transform).
    /// E.g. `ICON.chev` (viewBox 9 x 14) in an 8 x 12 svg.
    pub fn icon_fit(name: &str, w: f32, h: f32, stroke_w: f32, color: Rgba) -> El {
        let mut e = El::default().w(w).h(h).shrink(0.0);
        e.content = Content::Icon(Icon { name: name.to_string(), size: w, stroke_w, color, class_op: Vec::new(), fit: Some((w, h)), paint: None });
        e
    }
    /// Order 025: this icon's parts painted as `p` says (a filled icon, per-class fill / stroke colour). On a non-icon El
    /// it does nothing.
    pub fn icon_paint(mut self, p: IconPaint) -> El {
        if let Content::Icon(ic) = &mut self.content {
            ic.paint = Some(p);
        }
        self
    }
    pub fn paint(f: impl Fn(&Gfx, (f32, f32, f32, f32)) + 'static) -> El {
        El { content: Content::Paint(Rc::new(f)), ..El::default() }
    }
    /// Order 041: everything an `El::paint` box's drawing depends on besides its place (values, colours, a picture's
    /// identity): the frame repaints it only when this changes. Floats go in as `f32::to_bits`.
    pub fn sig(mut self, v: impl std::hash::Hash) -> El {
        use std::hash::Hasher;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        v.hash(&mut h);
        self.paint_sig = Some(h.finish());
        self
    }

    // ---- identity / input
    pub fn key(mut self, k: Key) -> El {
        self.key = Some(k);
        self
    }
    /// Clickable: `Ev::Click(key)` on a click (press and release on it).
    pub fn on_click(mut self, k: Key) -> El {
        self.key = Some(k);
        self.click = true;
        self
    }
    pub fn cursor(mut self, c: Cursor) -> El {
        self.cursor = c;
        self
    }
    /// Order 025: the drawing's `data-tip="…"` - give the element a key too (the tip shows for the hovered key).
    pub fn tip(mut self, text: &str) -> El {
        self.tip = Some(Rc::from(text));
        self
    }
    /// Order 045: the drawing's plain `title="…"` hover name - shown by the same tip bubble after Windows' hover delay
    /// (`tip::TITLE_DELAY_MS`); give the element a key too.
    pub fn title(mut self, text: &str) -> El {
        self.tip = Some(Rc::from(text));
        self.tip_title = true;
        self
    }
    /// Order 045: the drawing's `box.addEventListener('wheel', e => { e.preventDefault(); step(e.deltaY < 0 ? 1 : -1) })`:
    /// the wheel over this element reaches the page as `Ev::Wheel(key, ±1)` and does not scroll it. Needs a key.
    pub fn wheel_steps(mut self) -> El {
        self.wheel = true;
        self
    }
    pub fn no_hit(mut self) -> El {
        self.hit = false;
        self
    }
    pub fn live(mut self) -> El {
        self.live = true;
        self
    }

    // ---- tree
    pub fn child(mut self, c: El) -> El {
        self.children.push(c);
        self
    }
    pub fn child_if(self, on: bool, c: impl FnOnce() -> El) -> El {
        if on {
            self.child(c())
        } else {
            self
        }
    }
    pub fn children(mut self, c: impl IntoIterator<Item = El>) -> El {
        self.children.extend(c);
        self
    }

    // ---- box size
    pub fn w(mut self, v: f32) -> El {
        self.style.size.width = Dimension::length(v);
        self
    }
    pub fn h(mut self, v: f32) -> El {
        self.style.size.height = Dimension::length(v);
        self
    }
    pub fn size(self, w: f32, h: f32) -> El {
        self.w(w).h(h)
    }
    pub fn w_pct(mut self, p: f32) -> El {
        self.style.size.width = Dimension::percent(p / 100.0);
        self
    }
    pub fn h_pct(mut self, p: f32) -> El {
        self.style.size.height = Dimension::percent(p / 100.0);
        self
    }
    pub fn min_w(mut self, v: f32) -> El {
        self.style.min_size.width = lpa(v);
        self
    }
    pub fn min_h(mut self, v: f32) -> El {
        self.style.min_size.height = lpa(v);
        self
    }
    pub fn max_w(mut self, v: f32) -> El {
        self.style.max_size.width = lpa(v);
        self
    }
    pub fn max_h(mut self, v: f32) -> El {
        self.style.max_size.height = lpa(v);
        self
    }
    /// padding: top right bottom left
    pub fn pad(mut self, t: f32, r: f32, b: f32, l: f32) -> El {
        self.style.padding = taffy::Rect { top: lp(t), right: lp(r), bottom: lp(b), left: lp(l) };
        self
    }
    pub fn pad_all(self, v: f32) -> El {
        self.pad(v, v, v, v)
    }
    /// padding: vertical horizontal
    pub fn pad2(self, v: f32, h: f32) -> El {
        self.pad(v, h, v, h)
    }
    /// margin: top right bottom left (`f32::NAN` = auto)
    pub fn margin(mut self, t: f32, r: f32, b: f32, l: f32) -> El {
        let m = |v: f32| if v.is_nan() { LengthPercentageAuto::auto() } else { lpa(v) };
        self.style.margin = taffy::Rect { top: m(t), right: m(r), bottom: m(b), left: m(l) };
        self
    }
    pub fn ml_auto(mut self) -> El {
        self.style.margin.left = LengthPercentageAuto::auto();
        self
    }
    /// border width (layout only; `border()` also paints it)
    pub fn border(mut self, w: f32, c: Rgba) -> El {
        self.style.border = taffy::Rect { top: lp(w), right: lp(w), bottom: lp(w), left: lp(w) };
        self.border = Some((w, c, false));
        self
    }
    pub fn border_top(mut self, w: f32, c: Rgba) -> El {
        self.style.border.top = lp(w);
        self.border = Some((w, c, true));
        self
    }

    // ---- flex / grid
    pub fn gap(mut self, v: f32) -> El {
        self.style.gap = Size { width: lp(v), height: lp(v) };
        self
    }
    pub fn gap2(mut self, row: f32, col: f32) -> El {
        self.style.gap = Size { width: lp(col), height: lp(row) };
        self
    }
    pub fn grow(mut self, v: f32) -> El {
        self.style.flex_grow = v;
        self
    }
    pub fn shrink(mut self, v: f32) -> El {
        self.style.flex_shrink = v;
        self
    }
    /// `flex: 1` (grow 1, shrink 1, basis 0) + `min-width: 0` - the drawing's `.lbl`
    pub fn flex1(mut self) -> El {
        self.style.flex_grow = 1.0;
        self.style.flex_shrink = 1.0;
        self.style.flex_basis = Dimension::length(0.0);
        self.style.min_size.width = lpa(0.0);
        self
    }
    /// `flex: 1` inside a box whose width comes from its content (an auto-width button / field): grow and shrink from the
    /// content's own width (taffy sizes a basis-0 item to 0 there, Chromium to its text)
    pub fn flex1_auto(mut self) -> El {
        self.style.flex_grow = 1.0;
        self.style.flex_shrink = 1.0;
        self.style.min_size.width = lpa(0.0);
        self
    }
    /// `flex: none`
    pub fn none(mut self) -> El {
        self.style.flex_grow = 0.0;
        self.style.flex_shrink = 0.0;
        self
    }
    pub fn wrap(mut self) -> El {
        self.style.flex_wrap = FlexWrap::Wrap;
        self
    }
    pub fn items(mut self, a: AlignItems) -> El {
        self.style.align_items = Some(a);
        self
    }
    pub fn center(self) -> El {
        self.items(AlignItems::CENTER)
    }
    pub fn self_align(mut self, a: AlignSelf) -> El {
        self.style.align_self = Some(a);
        self
    }
    pub fn justify(mut self, j: JustifyContent) -> El {
        self.style.justify_content = Some(j);
        self
    }
    pub fn content(mut self, a: AlignContent) -> El {
        self.style.align_content = Some(a);
        self
    }
    /// `display: grid; place-items: center` (the drawing's icon boxes)
    pub fn place_center(mut self) -> El {
        self.style.display = Display::Grid;
        self.style.align_items = Some(AlignItems::CENTER);
        self.style.justify_items = Some(AlignItems::CENTER);
        self
    }
    /// `grid-template-columns: repeat(n, minmax(0, 1fr))`
    pub fn cols(mut self, n: u16) -> El {
        self.style.grid_template_columns = vec![repeat(n, vec![minmax(length(0.0), fr(1.0))])];
        self
    }
    /// any other taffy style change
    pub fn style(mut self, f: impl FnOnce(&mut Style)) -> El {
        f(&mut self.style);
        self
    }

    // ---- positioning
    /// `position: absolute` with left / top / right / bottom (`f32::NAN` = auto)
    pub fn abs(mut self, l: f32, t: f32, r: f32, b: f32) -> El {
        let m = |v: f32| if v.is_nan() { LengthPercentageAuto::auto() } else { lpa(v) };
        self.style.position = Position::Absolute;
        self.style.inset = taffy::Rect { left: m(l), top: m(t), right: m(r), bottom: m(b) };
        self
    }
    pub fn z(mut self, z: i32) -> El {
        self.z = z;
        self
    }

    // ---- paint
    pub fn bg(mut self, c: Rgba) -> El {
        self.bg = Some(Fill::Color(c));
        self
    }
    pub fn bg_linear(mut self, angle: f32, stops: &[(f32, Rgba)]) -> El {
        self.bg = Some(Fill::Linear(angle, stops.to_vec()));
        self
    }
    pub fn radius(mut self, r: f32) -> El {
        self.radius = r;
        self
    }
    pub fn shadow(mut self, s: &[Shadow]) -> El {
        self.shadows = s.to_vec();
        self
    }
    pub fn inset(mut self, s: &[Shadow]) -> El {
        self.insets = s.to_vec();
        self
    }
    pub fn opacity(mut self, o: f32) -> El {
        self.opacity = o;
        self
    }
    pub fn translate(mut self, x: f32, y: f32) -> El {
        self.translate = (x, y);
        self
    }
    /// Order 025: add a CSS `filter` colour function (`CssColor::Brightness(1.08)`, `CssColor::Grayscale(1.0)`); several are
    /// applied in order. The box and everything inside it are drawn into one layer that the filter colours (Blink's filter
    /// layer; under the box's opacity). `CssColor::Brightness(1.0)` / `Grayscale(0.0)` change nothing, but still make the layer.
    pub fn color_filter(mut self, f: crate::gfx::CssColor) -> El {
        self.color_filter.push(f);
        self
    }
    /// Order 025: an inner list's slim scrollbar thumb (`SlimThumb::GLASS` = `.rsl2` / `.pml` /
    /// `.cpnb`). Put it on the clipping box whose one child is the moved content (`cx.scroll_box`).
    pub fn slim_thumb(mut self, t: SlimThumb) -> El {
        self.slim_thumb = Some(t);
        self
    }
    /// Order 025: `transform: translate(<px*100>%, <py*100>%)` of the box's own size (see `translate_pct`).
    pub fn translate_pct(mut self, px: f32, py: f32) -> El {
        self.translate_pct = (px, py);
        self
    }
    pub fn scale(mut self, s: f32) -> El {
        self.scale = s;
        self
    }
    /// `transform: rotate(<deg>deg)`
    pub fn rotate(mut self, deg: f32) -> El {
        self.rotate = deg;
        self
    }
    pub fn clip(mut self) -> El {
        self.clip = true;
        self.style.overflow = taffy::Point { x: Overflow::Hidden, y: Overflow::Hidden };
        self
    }
    /// CSS `filter: brightness(<b>)`: the box, its shadows, rim and content painted, then brightened as one.
    /// CSS `position:sticky; top:<top>px`: while the page is scrolled past it, the box stays `top` px below the page's
    /// visible top, inside its parent's box (it stops at the parent's bottom). Painted in tree order: put it after what
    /// it should cover, or give it `.z(1)`.
    pub fn sticky(mut self, top: f32) -> El {
        self.sticky = Some(top);
        self
    }
    pub fn brightness(mut self, b: f32) -> El {
        self.brightness = b;
        self
    }
    /// CSS `backdrop-filter: blur(<blur>px) saturate(<saturate>)`
    pub fn backdrop(mut self, blur: f32, saturate: f32) -> El {
        self.backdrop = blur;
        self.backdrop_sat = saturate;
        self
    }

    // ---- text options (on a text element)
    pub fn ellipsis(mut self) -> El {
        if let Content::Text(t) = &mut self.content {
            t.wrap = Wrap::Ellipsis;
        }
        self.style.min_size.width = lpa(0.0);
        self
    }
    pub fn wrapping(mut self) -> El {
        if let Content::Text(t) = &mut self.content {
            t.wrap = Wrap::Wrap;
        }
        self
    }
    pub fn align(mut self, a: Align) -> El {
        if let Content::Text(t) = &mut self.content {
            t.align = a;
        }
        self
    }
    pub fn underline(mut self, on: bool) -> El {
        if let Content::Text(t) = &mut self.content {
            t.underline = on;
        }
        self
    }
    /// extra opacity for one SVG class of an icon
    pub fn class_op(mut self, class: &str, op: f32) -> El {
        if let Content::Icon(i) = &mut self.content {
            i.class_op.push((class.to_string(), op));
        }
        self
    }
}
