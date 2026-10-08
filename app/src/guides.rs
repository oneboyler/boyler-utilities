//! Order 045: the blue snap guide lines and the corner name tag shown while the on-screen mic icon or the timer bars are
//! dragged (menu-v22 `.guide` / `#gtag`, L106-113; the drags at L6512-6528 / L6935-6951). Two thin click-through windows
//! for the lines (a solid colour, moved - no pictures) and one small click-through window for the tag; they exist only
//! while a drag snaps.
//!
//! `.guide{position:fixed;z-index:19;background:rgba(10,132,255,.8);pointer-events:none}` `.guide.v{top:0;bottom:var(--tb);
//!   width:1px}` `.guide.h{left:0;right:0;height:1px}` (shown at `left` / `top` = `Math.round(pos)`).
//! `#gtag{position:fixed;padding:5px 8px;border-radius:10px;color:#fff;font:600 11px/1 var(--font);
//!   background:rgba(22,22,26,.82);backdrop-filter:blur(12px);box-shadow:inset 0 0 0 .5px rgba(255,255,255,.16),
//!   0 4px 12px rgba(0,0,0,.25);opacity:0;transition:opacity .12s ease}` `#gtag.on{opacity:1}`.

/// The drawing's corner names (`SPOTN`, L3528).
pub fn corner_name(v: char, h: char) -> Option<&'static str> {
    Some(match (v, h) {
        ('T', 'L') => "Top left",
        ('T', 'C') => "Top middle",
        ('T', 'R') => "Top right",
        ('B', 'L') => "Bottom left",
        ('B', 'C') => "Bottom middle",
        ('B', 'R') => "Bottom right",
        _ => return None,
    })
}

/// The snap targets a drag at top-left (l, t) hits (the drawing's `sx` / `sy`): each = (the guide's line, which side
/// 'L' / 'R' / 'C' or 'T' / 'B' / 'C'), the nearest within `snap`. `size` = the dragged thing, `area` = the work area
/// (DIPs). Edges at `edge`, the middle.
pub fn hits(l: f32, t: f32, size: (f32, f32), area: (f32, f32), edge: f32, snap: f32) -> (Option<(f32, char)>, Option<(f32, char)>) {
    let (w, hh) = size;
    let (aw, ah) = area;
    // [snap-to, guide line, side]
    let xs = [(edge, edge, 'L'), (aw - edge - w, aw - edge, 'R'), (aw / 2.0 - w / 2.0, aw / 2.0, 'C')];
    let ys = [(edge, edge, 'T'), (ah - edge - hh, ah - edge, 'B'), (ah / 2.0 - hh / 2.0, ah / 2.0, 'C')];
    let near = |v: f32, list: &[(f32, f32, char)]| {
        list.iter().filter(|c| (v - c.0).abs() <= snap).min_by(|a, b| (v - a.0).abs().total_cmp(&(v - b.0).abs())).map(|c| (c.1, c.2))
    };
    (near(l, &xs), near(t, &ys))
}

/// What one drag step shows: the guide lines (DIPs inside the work area) and the tag (its text, the pointer in DIPs inside
/// the work area). `corner` only when both snapped and the vertical one is not the middle (`sy[2]!=='C'`).
pub fn show_for(work: (i32, i32, i32, i32), scale: f32, l: f32, t: f32, size: (f32, f32), edge: f32, snap: f32, pointer: (f32, f32)) {
    let area = (work.2 as f32 / scale, work.3 as f32 / scale);
    let (sx, sy) = hits(l, t, size, area, edge, snap);
    let tag = match (sx, sy) {
        (Some((_, h)), Some((_, v))) if v != 'C' => corner_name(v, h),
        _ => None,
    };
    show(work, scale, sx.map(|s| s.0), sy.map(|s| s.0), tag, pointer);
}

/// The corner tag `#gtag` at (x, y) inside its picture (its shadow needs room around it).
pub fn tag_el(text: &str, x: f32, y: f32) -> crate::ui::el::El {
    use crate::gfx::{sh, Font, Rgba};
    use crate::ui::el::El;
    El::block()
        .abs(x, y, f32::NAN, f32::NAN)
        .pad(5.0, 8.0, 5.0, 8.0)
        .radius(10.0)
        .bg(Rgba(22.0 / 255.0, 22.0 / 255.0, 26.0 / 255.0, 0.82))
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.16))])
        .shadow(&[sh(0.0, 4.0, 12.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.25))])
        .child(El::text(text, Font::new(11.0, 600), Rgba(1.0, 1.0, 1.0, 1.0), 11.0))
}

#[cfg(windows)]
pub use real::{hide, show};

#[cfg(windows)]
mod real {
    use std::cell::RefCell;

    use windows::core::w;
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    use crate::gfx::Gfx;
    use crate::icons::Icons;
    use crate::ui::el::El;
    use crate::ui::lay::Laid;

    const CLASS: windows::core::PCWSTR = w!("BoylerUtilities.Guide");
    const TIMER: usize = 0x6A1D;
    /// room around the tag for its shadow (0 4px 12px)
    const PAD: f32 = 18.0;

    struct Tag {
        hwnd: HWND,
        text: String,
        /// fading toward on / off since, from opacity
        on: bool,
        t0: std::time::Instant,
        from: f32,
        op: f32,
    }

    #[derive(Default)]
    struct St {
        v: Option<HWND>,
        h: Option<HWND>,
        tag: Option<Tag>,
    }

    thread_local!(static ST: RefCell<St> = RefCell::new(St::default()));

    fn class() {
        thread_local!(static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
        if DONE.with(|d| d.get()) {
            return;
        }
        unsafe {
            let inst = GetModuleHandleW(None).unwrap_or_default();
            // the lines: the window's own background is the guide's colour rgb(10,132,255), at .8 (204 / 255) alpha
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: CLASS, hbrBackground: CreateSolidBrush(COLORREF(0x00FF840A)), ..Default::default() };
            RegisterClassW(&wc);
        }
        DONE.with(|d| d.set(true));
    }

    fn make(alpha: Option<u8>) -> Option<HWND> {
        class();
        let ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT;
        let h = unsafe { CreateWindowExW(ex, CLASS, w!("Guide"), WS_POPUP, 0, 0, 1, 1, None, None, GetModuleHandleW(None).ok().map(|h| h.into()), None) }.ok()?;
        if let Some(a) = alpha {
            unsafe {
                let _ = SetLayeredWindowAttributes(h, COLORREF(0), a, LWA_ALPHA);
            }
        }
        Some(h)
    }

    /// Show (or move) the guides and the tag: `gx` / `gy` = the vertical / horizontal line's place in DIPs inside the work
    /// area (None = that line hidden), `tag` = the corner's name at the pointer (DIPs inside the work area).
    pub fn show(work: (i32, i32, i32, i32), scale: f32, gx: Option<f32>, gy: Option<f32>, tag: Option<&str>, pointer: (f32, f32)) {
        let (wx, wy, ww, wh) = work;
        // a 1 CSS px line on the screen's pixels
        let thick = scale.round().max(1.0) as i32;
        ST.with(|st| {
            let mut st = st.borrow_mut();
            line(&mut st.v, gx.map(|x| (wx + (x.round() * scale).floor() as i32, wy, thick, wh)));
            line(&mut st.h, gy.map(|y| (wx, wy + (y.round() * scale).floor() as i32, ww, thick)));
            let (aw, ah) = (ww as f32 / scale, wh as f32 / scale);
            // left = round(clamp(clientX + 16, 4, W - 110)), top = round(clamp(clientY + 18, 4, H - 30))
            let tx = (pointer.0 + 16.0).clamp(4.0, (aw - 110.0).max(4.0)).round();
            let ty = (pointer.1 + 18.0).clamp(4.0, (ah - 30.0).max(4.0)).round();
            let at = (wx + (tx * scale).round() as i32, wy + (ty * scale).round() as i32);
            set_tag(&mut st.tag, tag, at, scale);
        });
    }

    /// The drag ended: lines gone at once (`display:none`), the tag fades out.
    pub fn hide() {
        ST.with(|st| {
            let mut st = st.borrow_mut();
            line(&mut st.v, None);
            line(&mut st.h, None);
            let mut t = st.tag.take();
            set_tag(&mut t, None, (0, 0), 1.0);
            st.tag = t;
        });
    }

    fn line(slot: &mut Option<HWND>, r: Option<(i32, i32, i32, i32)>) {
        match r {
            Some((x, y, w, h)) => {
                if slot.is_none() {
                    *slot = make(Some(204));
                }
                if let Some(hwnd) = *slot {
                    unsafe {
                        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
                    }
                }
            }
            None => {
                if let Some(hwnd) = slot.take() {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            }
        }
    }

    fn tag_op(t: &Tag) -> f32 {
        let p = crate::anim::EASE.ease((t.t0.elapsed().as_secs_f64() * 1000.0 / 120.0).clamp(0.0, 1.0)) as f32;
        t.from + (if t.on { 1.0 } else { 0.0 } - t.from) * p
    }

    fn set_tag(slot: &mut Option<Tag>, text: Option<&str>, at: (i32, i32), scale: f32) {
        match text {
            Some(s) => {
                if slot.is_none() {
                    let Some(hwnd) = make(None) else { return };
                    *slot = Some(Tag { hwnd, text: String::new(), on: false, t0: std::time::Instant::now(), from: 0.0, op: 0.0 });
                }
                let Some(t) = slot.as_mut() else { return };
                if !t.on {
                    t.from = tag_op(t);
                    t.on = true;
                    t.t0 = std::time::Instant::now();
                    unsafe {
                        SetTimer(Some(t.hwnd), TIMER, 16, None);
                    }
                }
                t.text = s.to_string();
                t.op = tag_op(t);
                paint_tag(t, at, scale);
            }
            None => {
                if let Some(t) = slot.as_mut() {
                    if t.on {
                        t.from = tag_op(t);
                        t.on = false;
                        t.t0 = std::time::Instant::now();
                        unsafe {
                            SetTimer(Some(t.hwnd), TIMER, 16, None);
                        }
                    }
                }
            }
        }
    }

    /// The tag's picture at screen point `at` (its box's top-left), at its opacity.
    fn paint_tag(t: &Tag, at: (i32, i32), scale: f32) {
        let g = Gfx::new(scale);
        let el = super::tag_el(&t.text, PAD, PAD);
        let laid = Laid::new(&g, El::block().w(400.0).h(80.0).child(el), 400.0, Some(80.0));
        let r = laid.nodes.get(1).map(|n| n.rect).unwrap_or((PAD, PAD, 80.0, 21.0));
        let (bw, bh) = (((r.2 + 2.0 * PAD) * scale).ceil() as i32, ((r.3 + 2.0 * PAD) * scale).ceil() as i32);
        let Some(mut surf) = crate::gfx::new_surface(bw, bh) else { return };
        g.begin(surf.canvas());
        laid.paint(&g, &Icons::new(), 0.0, 0.0, None);
        g.end();
        let px = crate::png::from_surface(&mut surf);
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: bw, biHeight: -bh, biPlanes: 1, biBitCount: 32, ..Default::default() },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            if let Ok(bmp) = CreateDIBSection(Some(mem), &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
                std::ptr::copy_nonoverlapping(px.data.as_ptr(), bits as *mut u8, px.data.len().min((bw * bh * 4) as usize));
                let old = SelectObject(mem, bmp.into());
                let pos = POINT { x: at.0 - (PAD * scale).round() as i32, y: at.1 - (PAD * scale).round() as i32 };
                let sz = SIZE { cx: bw, cy: bh };
                let blend = BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: (t.op * 255.0).round() as u8, AlphaFormat: 1 };
                let _ = UpdateLayeredWindow(t.hwnd, Some(screen), Some(&pos), Some(&sz), Some(mem), Some(&POINT::default()), COLORREF(0), Some(&blend), ULW_ALPHA);
                let _ = SetWindowPos(t.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
                SelectObject(mem, old);
                let _ = DeleteObject(bmp.into());
            }
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
        }
    }

    unsafe extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
            // the tag's .12 s fade: only its opacity changes (the picture stays)
            WM_TIMER if wp.0 == TIMER => {
                ST.with(|st| {
                    let mut st = st.borrow_mut();
                    let mut gone = false;
                    if let Some(t) = st.tag.as_mut().filter(|t| t.hwnd == h) {
                        t.op = tag_op(t);
                        let blend = BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: (t.op * 255.0).round() as u8, AlphaFormat: 1 };
                        unsafe {
                            let _ = UpdateLayeredWindow(h, None, None, None, None, None, COLORREF(0), Some(&blend), ULW_ALPHA);
                        }
                        if t.t0.elapsed().as_secs_f64() * 1000.0 >= 120.0 {
                            unsafe {
                                let _ = KillTimer(Some(h), TIMER);
                            }
                            gone = !t.on;
                        }
                    }
                    if gone {
                        if let Some(t) = st.tag.take() {
                            unsafe {
                                let _ = DestroyWindow(t.hwnd);
                            }
                        }
                    }
                });
                return LRESULT(0);
            }
            _ => {}
        }
        unsafe { DefWindowProcW(h, msg, wp, lp) }
    }
}

#[cfg(test)]
mod tests {
    use super::{corner_name, hits};

    #[test]
    fn hits_follow_the_drawings_snapping() {
        // a 96 x 34 icon on a 1920 x 1032 work area, edges at 24, snap 12
        let area = (1920.0, 1032.0);
        // near the top-left edges: both snap, lines at the edges
        assert_eq!(hits(30.0, 20.0, (96.0, 34.0), area, 24.0, 12.0), (Some((24.0, 'L')), Some((24.0, 'T'))));
        // near the right edge (1920 - 24 - 96 = 1800) and the vertical middle (1032 / 2 - 17 = 499)
        assert_eq!(hits(1795.0, 505.0, (96.0, 34.0), area, 24.0, 12.0), (Some((1896.0, 'R')), Some((516.0, 'C'))));
        // nothing within 12 px
        assert_eq!(hits(400.0, 300.0, (96.0, 34.0), area, 24.0, 12.0), (None, None));
        assert_eq!(corner_name('B', 'C'), Some("Bottom middle"));
        assert_eq!(corner_name('C', 'L'), None);
    }
}
