//! The glass: Windows.UI.Composition on the menu window, cut to the 14 px rounded shape by a MASK made of Skia's own
//! anti-aliased coverage of that shape (Order 013), Skia's pixels on top (the tint is part of them: ui::draw_rim paints
//! it as the window's background, like the drawing's CSS; the outer shadow's pixels inside the window rectangle too).
//! On screen the window's edge pixel = Skia's frame over (glass x coverage) over the desktop - the drawing's own order
//! (Chromium: the element's layer over the backdrop cut by the rounded box), with the same coverage numbers.
//! The drawing's glass (the owner's final numbers, Oct 7): tint rgba(20,20,26,.22) over blur(13px) saturate(170%) brightness(1.04).
//! Ways to get the real desktop behind it (`GlassMode`):
//! - `Host` (default): DWM's host backdrop (the desktop, already blurred by Windows for acrylic) -> saturate 170 % x
//!   brightness 1.04 (+ an optional extra blur), masked by the coverage. Exact colour recipe; the blur is Windows' own.
//! - `BlurBehind` (test only, `BU_GLASS=blur`; the Order 001-003 default): Windows' accent "blur behind" (set in menu.rs).
//!   It fills the whole window RECTANGLE (a window region does not cut it - measured on screen, Order 013), so its
//!   corners can't be smooth; no colour boost / brightness either.

use windows::core::*;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::Composition::*;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows_numerics::Vector2;

use crate::effects;
use crate::present::Chain;
use windows::Win32::Graphics::Dxgi::IDXGISwapChain2;

pub struct Glass {
    pub compositor: Compositor,
    _target: DesktopWindowTarget,
    pub root: ContainerVisual,
    pub content: SpriteVisual,
    glass: SpriteVisual,
    geom: CompositionRoundedRectangleGeometry,
    mode: GlassMode,
    /// the rounded edge's coverage (the glass is masked by it); kept for `restyle`
    mask_brush: CompositionSurfaceBrush,
    /// the brush showing the menu's own swap chain; kept for `attach` (Order 079)
    content_brush: CompositionSurfaceBrush,
}

/// The blur Windows' host backdrop already has, in CSS px: the Liquid style's 13 px (the drawing's number stood for it,
/// Order 013) - how far Windows' own blur really is: **unclear** (not measured).
const HOST_BLUR_PX: f32 = 13.0;

/// The extra Gaussian blur a style needs on top of the host backdrop's: Gaussians add in squares (sigma² = a² + b²), so
/// a 24 px style gets sqrt(24² - 13²) ≈ 20.2 px more; none for 13 px or less. The test switch BU_EXTRA_BLUR wins.
fn style_extra_sigma(blur_px: f32) -> f32 {
    let t = extra_blur_sigma();
    if t > 0.0 {
        return t;
    }
    (blur_px * blur_px - HOST_BLUR_PX * HOST_BLUR_PX).max(0.0).sqrt()
}

/// Order 041 item 7 (adaptive glass, the owner Oct 8: "when theres a straight white background, its IMPOSSIBLE to see anything
/// ... i dont want to add any shadows to text"): what the glass does with a backdrop too bright (dark theme) or too dark
/// (light theme) for its text. Each colour channel of the filtered backdrop is kept at most `Cap` / at least `Floor`
/// (0..1, sRGB): everything already darker than the cap (lighter than the floor) stays exactly as it was; brighter parts
/// are held at the cap, so the glass there is as dark as the text needs - per pixel, smoothly, by Windows' own
/// compositor (no capture, no CPU, nothing to flicker). The level is the brightest backdrop (darkest, light theme) under
/// which the window's text still has 7:1 (WCAG AAA) for the main text and 3:1 for the secondary text through the tint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Adapt {
    Cap(f32),
    Floor(f32),
}

/// Contrast targets (WCAG): the main text, the secondary text (`--fg2`).
pub const CONTRAST_FG: f32 = 7.0;
pub const CONTRAST_FG2: f32 = 3.0;

fn lin(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
/// WCAG relative luminance of an sRGB colour (0..1 channels).
pub fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}
/// WCAG contrast ratio.
pub fn contrast(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}
fn over(fg: [f32; 3], a: f32, bg: [f32; 3]) -> [f32; 3] {
    [fg[0] * a + bg[0] * (1.0 - a), fg[1] * a + bg[1] * (1.0 - a), fg[2] * a + bg[2] * (1.0 - a)]
}

/// The glass under the text when its filtered backdrop is the grey `level` (tint, then the option bubble the text sits
/// on), and the main / secondary text's contrast there.
pub fn text_contrast(n: &crate::settings::GlassNumbers, light: bool, level: f32) -> (f32, f32) {
    let t = n.tint_rgb.map(|v| v as f32 / 255.0);
    // the tint over the backdrop, then the option bubble the text sits on (`--grp`: white at the style's alpha)
    let glass = over([1.0; 3], n.bubbles_alpha, over(t, n.tint_alpha, [level; 3]));
    let p = if light { &crate::ui::LIGHT } else { &crate::ui::DARK };
    let rgb = |c: crate::gfx::Rgba| [c.0, c.1, c.2];
    let fg = over(rgb(p.fg), p.fg.3, glass);
    let fg2 = over(rgb(p.fg2), p.fg2.3, glass);
    (contrast(fg, glass), contrast(fg2, glass))
}

/// The cap (dark theme) / floor (light theme) for a style's numbers.
pub fn adapt(n: &crate::settings::GlassNumbers, light: bool) -> Adapt {
    // test copy only: BU_ADAPT=off = the glass as it was before (the proof's "before" pictures)
    if crate::testmode::env("BU_ADAPT").as_deref() == Some("off") {
        return if light { Adapt::Floor(0.0) } else { Adapt::Cap(1.0) };
    }
    let ok = |l: f32| {
        let (a, b) = text_contrast(n, light, l);
        a >= CONTRAST_FG && b >= CONTRAST_FG2
    };
    // dark: the brightest level that is still fine (none needed: 1); light: the darkest (none needed: 0)
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    if light {
        if ok(0.0) {
            return Adapt::Floor(0.0);
        }
        for _ in 0..24 {
            let m = (lo + hi) / 2.0;
            if ok(m) {
                hi = m;
            } else {
                lo = m;
            }
        }
        Adapt::Floor(hi)
    } else {
        if ok(1.0) {
            return Adapt::Cap(1.0);
        }
        for _ in 0..24 {
            let m = (lo + hi) / 2.0;
            if ok(m) {
                lo = m;
            } else {
                hi = m;
            }
        }
        Adapt::Cap(lo)
    }
}

/// The same step as a CSS-like colour filter for the off-screen pictures (gfx::CssColor), None = nothing to do.
pub fn adapt_filter(a: Adapt) -> Option<crate::gfx::CssColor> {
    match a {
        Adapt::Cap(l) if l < 1.0 => Some(crate::gfx::CssColor::Cap(l)),
        Adapt::Floor(l) if l > 0.0 => Some(crate::gfx::CssColor::Floor(l)),
        _ => None,
    }
}

/// Host mode's glass brush: DWM's host backdrop -> (extra blur) -> saturate x brightness -> the adaptive cap / floor,
/// times the edge mask.
fn host_brush(compositor: &Compositor, mask: &CompositionSurfaceBrush, saturate: f32, brightness: f32, extra: f32, adapt: Adapt) -> Result<CompositionMaskBrush> {
    match host_brush_with(compositor, mask, saturate, brightness, extra, Some(adapt)) {
        Ok(b) => Ok(b),
        Err(e) => {
            // a compositor that refuses the blend: the glass as before (and said in the log)
            crate::timing::note(&format!("adaptive glass refused by the compositor: {e}"));
            host_brush_with(compositor, mask, saturate, brightness, extra, None)
        }
    }
}

fn host_brush_with(compositor: &Compositor, mask: &CompositionSurfaceBrush, saturate: f32, brightness: f32, extra: f32, adapt: Option<Adapt>) -> Result<CompositionMaskBrush> {
    let backdrop = compositor.CreateHostBackdropBrush()?;
    let src = CompositionEffectSourceParameter::Create(h!("backdrop"))?;
    let mut chain_src: windows::Graphics::Effects::IGraphicsEffectSource = src.cast()?;
    if extra > 0.01 {
        let edge = effects::mirror_edges(chain_src);
        chain_src = effects::gaussian_blur(extra, edge.cast()?).cast()?;
    }
    let mut col = effects::saturate_brightness(saturate, brightness, chain_src);
    match adapt {
        Some(Adapt::Cap(l)) if l < 1.0 => col = effects::clamp_level(effects::BLEND_DARKEN, l, col.cast()?)?,
        Some(Adapt::Floor(l)) if l > 0.0 => col = effects::clamp_level(effects::BLEND_LIGHTEN, l, col.cast()?)?,
        _ => {}
    }
    let factory = compositor.CreateEffectFactory(&col)?;
    let eb = factory.CreateBrush()?;
    eb.SetSourceParameter(h!("backdrop"), &backdrop)?;
    let mb = compositor.CreateMaskBrush()?;
    mb.SetSource(&eb)?;
    mb.SetMask(mask)?;
    Ok(mb)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GlassMode {
    BlurBehind,
    Host,
    /// test only (Order 013, Q_013_01): the desktop captured behind the menu, blurred by Skia with the drawing's
    /// exact recipe and painted into the frame (capture.rs); the glass visual stays empty
    Own,
    /// test only (option 3 of Q_013_01): the captured desktop stays on the graphics card (a swap chain) and the
    /// compositor applies the recipe: crop to the window box, mirrored edges, Gaussian blur, saturate + brightness, mask
    Gpu,
}

/// Test-only switch for comparing the ways on a real screen: `BU_GLASS=blur` (default: host backdrop).
pub fn glass_mode() -> GlassMode {
    // (a BU_* switch counts only in a test copy: crate::testmode::env)
    match crate::testmode::env("BU_GLASS").as_deref() {
        Some("blur") => GlassMode::BlurBehind,
        Some("own") => GlassMode::Own,
        Some("gpu") => GlassMode::Gpu,
        _ => GlassMode::Host,
    }
}

/// Host mode only: extra blur on top of DWM's own host-backdrop blur (default none).
pub fn extra_blur_sigma() -> f32 {
    crate::testmode::env("BU_EXTRA_BLUR").and_then(|v| v.parse().ok()).unwrap_or(0.0)
}

pub const SATURATE: f32 = 1.7;
pub const BRIGHTNESS: f32 = 1.04;

/// A brush showing a swap chain 1:1 from the top-left (no stretching, no filtering at rest).
fn surface_brush(compositor: &Compositor, swap: &IDXGISwapChain2) -> Result<CompositionSurfaceBrush> {
    let ci: ICompositorInterop = compositor.cast()?;
    let surf = unsafe { ci.CreateCompositionSurfaceForSwapChain(swap)? };
    let sb = compositor.CreateSurfaceBrushWithSurface(&surf)?;
    sb.SetStretch(CompositionStretch::None)?;
    sb.SetHorizontalAlignmentRatio(0.0)?;
    sb.SetVerticalAlignmentRatio(0.0)?;
    Ok(sb)
}

impl Glass {
    /// `chain`: the menu's own pixels (`w` x `h` px; a CPU or a GPU swap chain - Order 051).
    /// `mask`: the swap chain holding Skia's coverage of the rounded window shape (white, alpha = coverage).
    /// `capture`: option 3's swap chain with the desktop behind the window (its top-left = the window's), and the
    /// display scale (the blur's sigma is 13 CSS px).
    #[allow(clippy::too_many_arguments)]
    pub fn new(hwnd: HWND, chain: (&IDXGISwapChain2, u32, u32), mask: &IDXGISwapChain2, capture: Option<(&Chain, f32)>, w: f32, h: f32, radius: f32, mode: GlassMode) -> Result<Glass> {
        let compositor = Compositor::new()?;
        let interop: ICompositorDesktopInterop = compositor.cast()?;
        let target = unsafe { interop.CreateDesktopWindowTarget(hwnd, false)? };
        let root = compositor.CreateContainerVisual()?;
        root.SetSize(Vector2 { X: w, Y: h })?;
        target.SetRoot(&root)?;

        let geom = compositor.CreateRoundedRectangleGeometry()?;
        geom.SetSize(Vector2 { X: w, Y: h })?;
        geom.SetCornerRadius(Vector2 { X: radius, Y: radius })?;

        // Host mode: backdrop -> (extra blur) -> saturate(1.7) brightness(1.04), times the edge mask. Blur-behind mode:
        // Windows draws it, nothing here.
        let glass = compositor.CreateSpriteVisual()?;
        glass.SetSize(Vector2 { X: w, Y: h })?;
        let mask_brush = surface_brush(&compositor, mask)?;
        if mode == GlassMode::Host {
            let n = crate::ui::glass_numbers();
            let light = crate::ui::is_light();
            glass.SetBrush(&host_brush(&compositor, &mask_brush, n.saturate, n.brightness, style_extra_sigma(n.blur_px), adapt(&n, light))?)?;
        }
        if let (GlassMode::Gpu, Some((cap, scale))) = (mode, capture) {
            let src = CompositionEffectSourceParameter::Create(h!("desk"))?;
            // the swap chain is exactly the window's box (a crop effect was refused by the compositor: E_INVALIDARG)
            let edge = effects::mirror_edges(src.cast()?);
            let blurred = effects::gaussian_blur_quality(13.0 * scale, edge.cast()?);
            let col = effects::saturate_brightness(SATURATE, BRIGHTNESS, blurred.cast()?);
            let factory = compositor.CreateEffectFactory(&col)?;
            let eb = factory.CreateBrush()?;
            eb.SetSourceParameter(h!("desk"), &surface_brush(&compositor, &cap.swap)?)?;
            let mb = compositor.CreateMaskBrush()?;
            mb.SetSource(&eb)?;
            mb.SetMask(&surface_brush(&compositor, mask)?)?;
            glass.SetBrush(&mb)?;
        }

        let ci: ICompositorInterop = compositor.cast()?;
        let surf = unsafe { ci.CreateCompositionSurfaceForSwapChain(chain.0)? };
        let sb = compositor.CreateSurfaceBrushWithSurface(&surf)?;
        sb.SetStretch(CompositionStretch::None)?;
        sb.SetHorizontalAlignmentRatio(0.0)?;
        sb.SetVerticalAlignmentRatio(0.0)?;
        let content = compositor.CreateSpriteVisual()?;
        content.SetBrush(&sb)?;
        content.SetSize(Vector2 { X: chain.1 as f32, Y: chain.2 as f32 })?;

        let kids = root.Children()?;
        kids.InsertAtTop(&glass)?;
        kids.InsertAtTop(&content)?;
        Ok(Glass { compositor, _target: target, root, content, glass, geom, mode, mask_brush, content_brush: sb })
    }

    /// Settings › Glass style changed while the menu is open: the glass brush again with that style's numbers (Host mode;
    /// the test-only modes keep their own recipe).
    pub fn restyle(&self, n: crate::settings::GlassNumbers) -> Result<()> {
        if self.mode != GlassMode::Host {
            return Ok(());
        }
        let light = crate::ui::is_light();
        self.glass.SetBrush(&host_brush(&self.compositor, &self.mask_brush, n.saturate, n.brightness, style_extra_sigma(n.blur_px), adapt(&n, light))?)
    }

    /// Order 079: show other swap chains (the menu moved between the GPU and the CPU path) in the SAME visuals: the window's
    /// compositor, target and glass effect stay, so the window is never composed without them (closing and making them
    /// again left it empty for a moment - a blink). The new chains must hold a presented frame already.
    pub fn attach(&self, chain: (&IDXGISwapChain2, u32, u32), mask: &IDXGISwapChain2) -> Result<()> {
        let ci: ICompositorInterop = self.compositor.cast()?;
        let m = unsafe { ci.CreateCompositionSurfaceForSwapChain(mask)? };
        let c = unsafe { ci.CreateCompositionSurfaceForSwapChain(chain.0)? };
        self.mask_brush.SetSurface(&m)?;
        self.content_brush.SetSurface(&c)?;
        self.content.SetSize(Vector2 { X: chain.1 as f32, Y: chain.2 as f32 })
    }

    /// Opacity of the whole flyout (glass + content), for the open / close fade.
    pub fn set_opacity(&self, o: f32) {
        let _ = self.root.SetOpacity(o);
    }

    /// Move the glass + content inside the window (used for the open rise / close drop; Order 047 item 12: the whole
    /// motion, from the room the window has above the menu's box).
    pub fn set_offset_y(&self, dy: f32) {
        let _ = self.root.SetOffset(windows_numerics::Vector3 { X: 0.0, Y: dy, Z: 0.0 });
    }

    /// Tear the visual tree down now (not whenever the compositor gets to it), so the menu's GPU memory goes with the menu.
    pub fn close(&self) {
        let _ = self.root.Children().and_then(|c| c.RemoveAll());
        let _ = self.content.SetBrush(None);
        let _ = self.glass.SetBrush(None);
        let _ = self._target.SetRoot(None);
        let _ = self._target.Close();
        let _ = self.compositor.Close();
    }

    #[allow(dead_code)]
    pub fn resize(&self, w: f32, h: f32) {
        let _ = self.geom.SetSize(Vector2 { X: w, Y: h });
        let _ = self.glass.SetSize(Vector2 { X: w, Y: h });
        let _ = self.root.SetSize(Vector2 { X: w, Y: h });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::GlassStyle;

    /// Order 041 item 7: every glass style, both themes - the text at the cap / floor has its contrast, and the cap is
    /// the brightest such level (a little brighter is not enough); a style already fine everywhere gets no change.
    #[test]
    fn the_adaptive_level_gives_the_text_its_contrast() {
        for light in [false, true] {
            let styles: Vec<crate::settings::GlassNumbers> =
                if light { vec![crate::settings::GlassNumbers::LIGHT] } else { GlassStyle::ALL.iter().map(|s| s.numbers()).collect() };
            for n in styles {
                let a = adapt(&n, light);
                let l = match a {
                    Adapt::Cap(l) | Adapt::Floor(l) => l,
                };
                let (c1, c2) = text_contrast(&n, light, l);
                assert!(c1 >= CONTRAST_FG - 0.01 && c2 >= CONTRAST_FG2 - 0.01, "{n:?} {a:?}: {c1} {c2}");
                match a {
                    Adapt::Cap(l) if l < 1.0 => {
                        let (d1, d2) = text_contrast(&n, light, l + 0.01);
                        assert!(d1 < CONTRAST_FG || d2 < CONTRAST_FG2, "the cap is the brightest fine level: {n:?}");
                    }
                    Adapt::Floor(l) if l > 0.0 => {
                        let (d1, d2) = text_contrast(&n, light, l - 0.01);
                        assert!(d1 < CONTRAST_FG || d2 < CONTRAST_FG2, "the floor is the darkest fine level: {n:?}");
                    }
                    _ => {}
                }
            }
        }
        // the default dark glass (Liquid) on pure white had 1.5:1 for its text: it needs the cap
        assert!(matches!(adapt(&GlassStyle::Liquid.numbers(), false), Adapt::Cap(l) if l < 0.5));
        assert!(text_contrast(&GlassStyle::Liquid.numbers(), false, 1.0).0 < 2.0);
    }

    /// The styles' extra blur over the host backdrop's 13 px: Liquid none, Frosted sqrt(24² - 13²), Windows look
    /// sqrt(31² - 13²) - Gaussians add in squares.
    #[test]
    fn each_style_gets_its_extra_blur() {
        let e = |s: GlassStyle| style_extra_sigma(s.numbers().blur_px);
        assert_eq!(e(GlassStyle::Liquid), 0.0);
        assert!((e(GlassStyle::Frosted) - (24.0f32 * 24.0 - 169.0).sqrt()).abs() < 1e-4);
        assert!((e(GlassStyle::WindowsLook) - (31.0f32 * 31.0 - 169.0).sqrt()).abs() < 1e-4);
    }
}
