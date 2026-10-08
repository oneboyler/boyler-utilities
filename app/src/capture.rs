//! Order 013 — the drawing's EXACT glass over the real desktop (test mode `BU_GLASS=own`, not the default; see
//! Q_013_01): Windows' Desktop Duplication hands us the desktop pixels behind the menu (the menu and its shadow window
//! are excluded from every screen capture with WDA_EXCLUDEFROMCAPTURE, so they never see themselves), and Skia blurs them
//! with the drawing's own recipe (blur 13 px as a standard deviation, saturate 170 %, brightness 1.04, mirrored edges).
//! A worker thread waits inside AcquireNextFrame (no CPU while nothing on the screen changes); when the desktop under
//! the menu changed, it copies just that rectangle and posts WM_CAPTURE to the menu window.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_CAPTURE: u32 = WM_APP + 3;

/// The latest pixels of the desktop under the menu (BGRA, opaque), in screen pixels at `rect`.
pub struct Shot {
    pub px: Vec<u8>,
    pub rect: RECT,
    pub seq: u64,
    /// when the duplication saw the change (QueryPerformanceCounter, ms) - for the lag measurement
    pub t_present: f64,
    /// frames the worker got / copied / found unchanged (cost counters for the report)
    pub frames: u64,
    pub copies: u64,
    pub same: u64,
    /// option 3: desktop frame in hand -> copied + presented, ms (sum / max / count)
    pub hand_sum: f64,
    pub hand_max: f64,
    pub hand_n: u64,
}

/// Option 3 of Q_013_01 (`BU_GLASS=gpu`): the captured rectangle never leaves the graphics card - it is copied into this
/// swap chain (on a device of its own, used only by the worker after the hand-over) and Windows' compositor blurs it.
pub struct GpuTarget {
    pub dev: ID3D11Device,
    pub swap: IDXGISwapChain2,
}
// the device and the swap chain are used only by the worker after the hand-over
unsafe impl Send for GpuTarget {}

pub struct Capture {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub shot: Arc<Mutex<Shot>>,
}

fn qpc_ms() -> f64 {
    crate::timing::now()
}

impl Capture {
    /// Start watching `rect` (screen pixels). `notify` gets WM_CAPTURE after every change.
    pub fn start(notify: HWND, rect: RECT, gpu: Option<GpuTarget>) -> Capture {
        let stop = Arc::new(AtomicBool::new(false));
        let shot = Arc::new(Mutex::new(Shot { px: Vec::new(), rect, seq: 0, t_present: 0.0, frames: 0, copies: 0, same: 0, hand_sum: 0.0, hand_max: 0.0, hand_n: 0 }));
        let (s2, sh2) = (stop.clone(), shot.clone());
        let nh = notify.0 as isize;
        let thread = std::thread::Builder::new()
            .name("bu-capture".into())
            .spawn(move || {
                if let Err(e) = worker(HWND(nh as *mut _), rect, &s2, &sh2, gpu) {
                    crate::timing::note(&format!("capture error {:08x} {}", e.code().0, e.message()));
                }
            })
            .ok();
        Capture { stop, thread, shot }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        // the worker sees this within its 100 ms wait and ends by itself; the menu does not wait for it to close
        self.stop.store(true, Ordering::SeqCst);
        drop(self.thread.take());
    }
}

fn find_output(rect: &RECT) -> Result<(IDXGIAdapter1, IDXGIOutput1, RECT)> {
    unsafe {
        let f: IDXGIFactory1 = CreateDXGIFactory1()?;
        let (cx, cy) = ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2);
        let mut i = 0;
        while let Ok(a) = f.EnumAdapters1(i) {
            let mut j = 0;
            while let Ok(o) = a.EnumOutputs(j) {
                let d = o.GetDesc()?;
                let r = d.DesktopCoordinates;
                if cx >= r.left && cx < r.right && cy >= r.top && cy < r.bottom {
                    return Ok((a, o.cast()?, r));
                }
                j += 1;
            }
            i += 1;
        }
        Err(Error::from(E_FAIL))
    }
}

fn worker(notify: HWND, want: RECT, stop: &AtomicBool, shot: &Mutex<Shot>, gpu: Option<GpuTarget>) -> Result<()> {
    unsafe {
        let t0 = qpc_ms();
        let (adapter, output, orc) = find_output(&want)?;
        let (dev, ctx): (ID3D11Device, ID3D11DeviceContext) = match &gpu {
            Some(g) => (g.dev.clone(), g.dev.GetImmediateContext()?),
            None => {
                let mut dev = None;
                let mut ctx = None;
                D3D11CreateDevice(&adapter, D3D_DRIVER_TYPE_UNKNOWN, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut dev), None, Some(&mut ctx))?;
                (dev.unwrap(), ctx.unwrap())
            }
        };
        let t1 = qpc_ms();
        // the part of the wanted rectangle on this output, in the output's own pixels
        let r = RECT { left: want.left.max(orc.left), top: want.top.max(orc.top), right: want.right.min(orc.right), bottom: want.bottom.min(orc.bottom) };
        let (w, h) = ((r.right - r.left) as u32, (r.bottom - r.top) as u32);
        let sdesc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut staging = None;
        dev.CreateTexture2D(&sdesc, None, Some(&mut staging))?;
        let staging = staging.unwrap();
        let mut dup = output.DuplicateOutput(&dev)?;
        let t2 = qpc_ms();
        crate::timing::note(&format!("capture start device {:.1} ms duplication {:.1} ms rect {} {} {} {}", t1 - t0, t2 - t1, r.left, r.top, r.right, r.bottom));
        let mut first = true;
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let mut dirty: Vec<RECT> = Vec::new();
        while !stop.load(Ordering::SeqCst) {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut res = None;
            match dup.AcquireNextFrame(100, &mut info, &mut res) {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                    // mode change, UAC / secure desktop, full-screen app: make a new duplication
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    if let Ok(d) = output.DuplicateOutput(&dev) {
                        dup = d;
                        first = true;
                    }
                    continue;
                }
                Err(e) => return Err(e),
            }
            let t_got = qpc_ms();
            shot.lock().unwrap().frames += 1;
            // only a change inside our rectangle matters (the mouse pointer alone: LastPresentTime 0)
            // the very first frame of a new duplication can be an empty (black) picture: take the first picture only
            // from a frame Windows really presented (measured: Order 013's real-screen test showed black glass)
            if first && info.LastPresentTime == 0 {
                let _ = dup.ReleaseFrame();
                continue;
            }
            let mut hit = first;
            if !first && info.LastPresentTime != 0 && info.TotalMetadataBufferSize > 0 {
                let mut need = 0u32;
                dirty.resize((info.TotalMetadataBufferSize as usize) / std::mem::size_of::<RECT>() + 1, RECT::default());
                if dup.GetFrameDirtyRects((dirty.len() * std::mem::size_of::<RECT>()) as u32, dirty.as_mut_ptr(), &mut need).is_ok() {
                    let n = need as usize / std::mem::size_of::<RECT>();
                    let (lx, ly) = (r.left - orc.left, r.top - orc.top);
                    hit = dirty[..n].iter().any(|d| d.left < lx + w as i32 && d.right > lx && d.top < ly + h as i32 && d.bottom > ly);
                } else {
                    hit = true;
                }
                // moved areas count as changes too
                let mut mneed = 0u32;
                let mut mv = vec![DXGI_OUTDUPL_MOVE_RECT::default(); 64];
                if dup.GetFrameMoveRects((mv.len() * std::mem::size_of::<DXGI_OUTDUPL_MOVE_RECT>()) as u32, mv.as_mut_ptr(), &mut mneed).is_ok() && mneed > 0 {
                    hit = true;
                }
            }
            if hit && gpu.is_some() {
                // option 3: copy on the graphics card into the glass's swap chain and show it - no pixels on the CPU
                if let (Some(res), Some(g)) = (res, &gpu) {
                    let tex: ID3D11Texture2D = res.cast()?;
                    let bx = D3D11_BOX { left: (r.left - orc.left) as u32, top: (r.top - orc.top) as u32, front: 0, right: (r.right - orc.left) as u32, bottom: (r.bottom - orc.top) as u32, back: 1 };
                    let bb: ID3D11Texture2D = g.swap.GetBuffer(0)?;
                    ctx.CopySubresourceRegion(&bb, 0, 0, 0, 0, &tex, 0, Some(&bx));
                    let _ = g.swap.Present(0, DXGI_PRESENT(0));
                    let hand = qpc_ms() - t_got;
                    let mut s = shot.lock().unwrap();
                    s.hand_sum += hand;
                    s.hand_max = s.hand_max.max(hand);
                    s.hand_n += 1;
                    s.copies += 1;
                    s.rect = r;
                    s.seq += 1;
                    s.t_present = t_got;
                    drop(s);
                    let _ = PostMessageW(Some(notify), WM_CAPTURE, WPARAM(0), LPARAM(0));
                }
                first = false;
            } else if hit {
                if let Some(res) = res {
                    let tex: ID3D11Texture2D = res.cast()?;
                    let bx = D3D11_BOX { left: (r.left - orc.left) as u32, top: (r.top - orc.top) as u32, front: 0, right: (r.right - orc.left) as u32, bottom: (r.bottom - orc.top) as u32, back: 1 };
                    ctx.CopySubresourceRegion(&staging, 0, 0, 0, 0, &tex, 0, Some(&bx));
                    let mut m = D3D11_MAPPED_SUBRESOURCE::default();
                    ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut m))?;
                    let src = std::slice::from_raw_parts(m.pData as *const u8, (m.RowPitch * h) as usize);
                    let mut changed = first;
                    for y in 0..h as usize {
                        let row = &src[y * m.RowPitch as usize..y * m.RowPitch as usize + w as usize * 4];
                        let dst = &mut buf[y * w as usize * 4..(y + 1) * w as usize * 4];
                        if !changed && row != dst {
                            changed = true;
                        }
                        if changed {
                            dst.copy_from_slice(row);
                        }
                    }
                    ctx.Unmap(&staging, 0);
                    let mut s = shot.lock().unwrap();
                    s.copies += 1;
                    if changed {
                        // opaque: the desktop has no transparency
                        let mut px = buf.clone();
                        for a in px.chunks_mut(4) {
                            a[3] = 255;
                        }
                        s.px = px;
                        s.rect = r;
                        s.seq += 1;
                        s.t_present = t_got;
                        drop(s);
                        let _ = PostMessageW(Some(notify), WM_CAPTURE, WPARAM(0), LPARAM(0));
                    } else {
                        s.same += 1;
                    }
                }
                first = false;
            }
            let _ = dup.ReleaseFrame();
            // test only: one picture of the desktop, then stop (a real-screen picture of the menu with `noexclude`)
            if !first && crate::menu::test_flag("captureonce") && shot.lock().unwrap().seq > 0 {
                break;
            }
        }
        Ok(())
    }
}

/// Keep a window out of every screen capture (also ours): WDA_EXCLUDEFROMCAPTURE (Windows 10 2004 and later).
pub fn exclude_from_capture(hwnd: HWND) -> bool {
    unsafe { SetWindowDisplayAffinity(hwnd, WINDOW_DISPLAY_AFFINITY(0x11)).is_ok() }
}
