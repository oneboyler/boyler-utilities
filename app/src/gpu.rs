//! Drawing on the graphics card (Order 051).
//! Skia's GPU backend (Ganesh) on Direct3D 12: one device + command queue on the adapter that drives the window's monitor,
//! one Skia context on the UI thread, and composition swap chains made on that queue, so Skia draws straight into their back
//! buffers. Why Direct3D 12 and not our old Direct3D 11 device: Skia's only Direct3D backend is Direct3D 12 (its prebuilt
//! `d3d` binaries exist for our pinned 0.153.3), and a composition swap chain made on a Direct3D 12 queue is the same
//! flip-model, frame-latency-waitable chain the compositor (and the glass) already takes - no Direct3D 11 interop layer.
//!
//! The CPU path of Orders 003-050 stays as the fallback, chosen by itself: no hardware adapter (a VM, the Basic Render
//! Driver), a remote desktop session, a device / context that fails to start, or a device lost while drawing. After a lost
//! device the GPU is tried again `RETRY_MS` later (and on the next open). The device lives only while something draws with
//! it: the menu and the capture overlay each hold an `Rc<Gpu>`; when both let go the device and every GPU resource are
//! released (closed = no GPU memory of ours). Every choice is written to the timing log (`gpu path=...`).
//!
//! Test switches (a test copy only, `testmode::env`): `BU_GPU=off` = CPU path forced, `BU_GPU=fail` = the GPU start fails
//! (the fallback without a broken driver); the test command `gpulose` removes the device (`ID3D12Device5::RemoveDevice`).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use skia_safe as sk;
use skia_safe::gpu as skg;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_REMOTESESSION};

/// After a lost device: the CPU path for this long, then the GPU again.
pub const RETRY_MS: f64 = 3000.0;

pub struct Gpu {
    pub device: ID3D12Device,
    pub queue: ID3D12CommandQueue,
    pub factory: IDXGIFactory4,
    pub name: String,
    ctx: RefCell<skg::DirectContext>,
}

thread_local! {
    /// the device in use on this thread (the menu / the overlay hold it; it goes when they both let go)
    static CUR: RefCell<Weak<Gpu>> = const { RefCell::new(Weak::new()) };
    /// a device made on tray hover, handed to the next open (dropped with the warm-up)
    static KEPT: RefCell<Option<Rc<Gpu>>> = const { RefCell::new(None) };
    /// when the last device was lost (ms) - the CPU path until RETRY_MS later
    static LOST_AT: Cell<Option<f64>> = const { Cell::new(None) };
    /// the last path written to the log (one line per change)
    static LOGGED: RefCell<String> = const { RefCell::new(String::new()) };
}

fn log_path(s: String) {
    let changed = LOGGED.with(|l| {
        let mut l = l.borrow_mut();
        let c = *l != s;
        if c {
            *l = s.clone();
        }
        c
    });
    if changed {
        crate::timing::note(&s);
    }
}

/// Why the GPU is not used right now (None = it may be).
fn cpu_reason() -> Option<String> {
    let remote = unsafe { GetSystemMetrics(SM_REMOTESESSION) } != 0;
    reason(crate::testmode::env("BU_GPU").as_deref(), remote, LOST_AT.with(|l| l.get()), crate::timing::now())
}

/// The path rule: a test switch, a remote desktop session, a lost device less than RETRY_MS ago = the CPU path.
fn reason(switch: Option<&str>, remote: bool, lost_at: Option<f64>, now: f64) -> Option<String> {
    match switch {
        Some("off") => return Some("forced (BU_GPU=off)".into()),
        Some("fail") => return Some("start failed (BU_GPU=fail, test)".into()),
        _ => {}
    }
    if remote {
        return Some("remote desktop session".into());
    }
    if lost_at.is_some_and(|t| now - t < RETRY_MS) {
        return Some("device lost, retry pending".into());
    }
    None
}

/// The device for drawing on `mon` (the one in use, the warm one, or a new one); None = draw on the CPU (logged why).
pub fn get(mon: Option<HMONITOR>) -> Option<Rc<Gpu>> {
    if let Some(why) = cpu_reason() {
        log_path(format!("gpu path=cpu reason={why}"));
        return None;
    }
    // (the warm device is always taken out of KEPT: the menu / overlay holds it now, so it goes when they close)
    let kept = KEPT.with(|k| k.borrow_mut().take()).filter(|g| !g.lost());
    let cur = CUR.with(|c| c.borrow().upgrade()).filter(|g| !g.lost());
    let g = match cur.or(kept) {
        Some(g) => g,
        None => match Gpu::new(mon) {
            Ok(g) => Rc::new(g),
            Err(e) => {
                log_path(format!("gpu path=cpu reason=start failed {:08x} {}", e.code().0, e.message()));
                return None;
            }
        },
    };
    CUR.with(|c| *c.borrow_mut() = Rc::downgrade(&g));
    LOST_AT.with(|l| l.set(None));
    log_path(format!("gpu path=gpu adapter=\"{}\"", g.name));
    Some(g)
}

/// Tray hover: make the device now (the slow part of an open); dropped again by `cool_down`.
pub fn warm(mon: Option<HMONITOR>) {
    if KEPT.with(|k| k.borrow().is_some()) || CUR.with(|c| c.borrow().upgrade()).is_some() {
        return;
    }
    if let Some(g) = get(mon) {
        KEPT.with(|k| *k.borrow_mut() = Some(g));
    }
}

/// The menu or the overlay closed while the other still draws on the device: what it used is let go now (a device nobody
/// holds is gone already).
pub fn trim_current() {
    if let Some(g) = CUR.with(|c| c.borrow().upgrade()) {
        if !g.lost() {
            g.trim();
        }
    }
}

/// Drop the warm device (the menu was not opened).
pub fn cool_down() -> bool {
    KEPT.with(|k| k.borrow_mut().take()).is_some()
}

/// The device in use was lost: the CPU path until RETRY_MS from now.
pub fn mark_lost() {
    LOST_AT.with(|l| l.set(Some(crate::timing::now())));
    log_path("gpu path=cpu reason=device lost".into());
}

/// Is a device alive on this thread (proof: the GPU is released while the menu and the overlay are closed)?
pub fn alive() -> bool {
    CUR.with(|c| c.borrow().strong_count() > 0) || KEPT.with(|k| k.borrow().is_some())
}

/// The hardware adapter driving `mon` (no cross-adapter copy for the compositor), else the first hardware adapter.
/// The Basic Render Driver / WARP never counts: Skia's CPU path is faster than a software Direct3D.
fn pick_adapter(factory: &IDXGIFactory4, mon: Option<HMONITOR>) -> Result<(IDXGIAdapter1, String)> {
    let mut first = None;
    unsafe {
        for i in 0.. {
            let Ok(a) = factory.EnumAdapters1(i) else { break };
            let d = a.GetDesc1()?;
            if (d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 || d.VendorId == 0x1414 {
                continue;
            }
            let name = String::from_utf16_lossy(&d.Description).trim_end_matches('\0').to_string();
            if let Some(m) = mon {
                for o in 0.. {
                    let Ok(out) = a.EnumOutputs(o) else { break };
                    if out.GetDesc().map(|od| od.Monitor == m).unwrap_or(false) {
                        return Ok((a, name));
                    }
                }
            }
            if first.is_none() {
                first = Some((a, name));
            }
        }
    }
    first.ok_or_else(|| Error::new(DXGI_ERROR_NOT_FOUND, "no hardware graphics adapter"))
}

impl Gpu {
    fn new(mon: Option<HMONITOR>) -> Result<Gpu> {
        let t = crate::timing::now();
        // Skia's shaders need the system's HLSL compiler (shadercache.rs): without it every GPU frame would be blank
        if !crate::shadercache::available() {
            return Err(Error::new(E_FAIL, "no d3dcompiler_47.dll"));
        }
        unsafe {
            let factory: IDXGIFactory4 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
            let (adapter, name) = pick_adapter(&factory, mon)?;
            let mut device: Option<ID3D12Device> = None;
            D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut device)?;
            let device = device.ok_or_else(|| Error::from(E_FAIL))?;
            let queue: ID3D12CommandQueue = device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC { Type: D3D12_COMMAND_LIST_TYPE_DIRECT, ..Default::default() })?;
            let bc = skg::d3d::BackendContext { adapter, device: device.clone(), queue: queue.clone(), memory_allocator: None, protected_context: skg::Protected::No };
            let ctx = skg::direct_contexts::make_d3d(&bc, None).ok_or_else(|| Error::new(E_FAIL, "Skia's Direct3D context did not start"))?;
            crate::timing::note(&format!("open_step gpu_device {:.1} ms", crate::timing::now() - t));
            Ok(Gpu { device, queue, factory, name, ctx: RefCell::new(ctx) })
        }
    }

    /// The device was removed / reset (driver update, TDR, `gpulose`) or Skia gave the context up.
    pub fn lost(&self) -> bool {
        unsafe { self.device.GetDeviceRemovedReason() }.is_err() || self.ctx.borrow_mut().abandoned()
    }

    /// A GPU surface (premultiplied BGRA, sRGB-tagged like the CPU surfaces), or None.
    pub fn surface(&self, w: i32, h: i32) -> Option<sk::Surface> {
        let ii = sk::ImageInfo::new((w.max(1), h.max(1)), sk::ColorType::BGRA8888, sk::AlphaType::Premul, Some(sk::ColorSpace::new_srgb()));
        let mut c = self.ctx.borrow_mut();
        skg::surfaces::render_target(&mut c, skg::Budgeted::Yes, &ii, None, skg::SurfaceOrigin::TopLeft, Some(&crate::gfx::surface_props()), false, None)
    }

    /// A CPU picture as a GPU texture (uploaded once; drawing it then costs no upload).
    pub fn texture(&self, img: &sk::Image) -> Option<sk::Image> {
        let mut c = self.ctx.borrow_mut();
        skg::images::texture_from_image(&mut c, img, skg::Mipmapped::No, skg::Budgeted::Yes)
    }

    /// Hand everything drawn so far to the GPU (`surf` = a swap chain's back buffer, left ready to be presented).
    pub fn flush_present(&self, surf: &mut sk::Surface) {
        let mut c = self.ctx.borrow_mut();
        c.flush_surface_with_access(surf, sk::surface::BackendSurfaceAccess::Present, &skg::FlushInfo::default());
        c.submit(None);
    }

    /// Submit and wait until the GPU has finished (before the device or a swap chain goes away).
    pub fn finish(&self) {
        self.ctx.borrow_mut().flush_submit_and_sync_cpu();
    }

    /// Let go of the GPU memory nothing holds any more (the menu closed but the overlay still draws, or the other way).
    pub fn trim(&self) {
        self.ctx.borrow_mut().purge_unlocked_resources(skg::PurgeResourceOptions::AllResources);
    }

    /// Test only (`gpulose`): remove the device the way a driver crash does.
    pub fn remove_for_test(&self) {
        if let Ok(d5) = self.device.cast::<ID3D12Device5>() {
            unsafe { d5.RemoveDevice() };
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        let mut c = self.ctx.borrow_mut();
        if !c.abandoned() {
            c.flush_submit_and_sync_cpu();
            c.free_gpu_resources();
        }
        crate::timing::note("gpu released");
    }
}

/// A composition swap chain on the GPU device's queue (premultiplied BGRA, flip model, frame-latency waitable - the same
/// kind of chain as present.rs's CPU one) with a Skia surface on each back buffer: Skia draws straight into the buffer the
/// compositor shows next - no CPU copy.
pub struct GpuChain {
    pub gpu: Rc<Gpu>,
    pub swap: IDXGISwapChain2,
    swap3: IDXGISwapChain3,
    pub waitable: HANDLE,
    pub w: u32,
    pub h: u32,
    surfs: Vec<sk::Surface>,
}

impl GpuChain {
    pub fn new(gpu: &Rc<Gpu>, w: u32, h: u32) -> Result<GpuChain> {
        unsafe {
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: w.max(1),
                Height: h.max(1),
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
            };
            let sc1 = gpu.factory.CreateSwapChainForComposition(&gpu.queue, &desc, None)?;
            let swap: IDXGISwapChain2 = sc1.cast()?;
            let swap3: IDXGISwapChain3 = sc1.cast()?;
            swap.SetMaximumFrameLatency(1)?;
            let waitable = swap.GetFrameLatencyWaitableObject();
            let mut surfs = Vec::new();
            for i in 0..desc.BufferCount {
                let res: ID3D12Resource = swap.GetBuffer(i)?;
                let info = skg::d3d::TextureResourceInfo {
                    resource: res,
                    alloc: None,
                    resource_state: D3D12_RESOURCE_STATE_PRESENT,
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    sample_count: 1,
                    level_count: 1,
                    sample_quality_pattern: 0,
                    protected: skg::Protected::No,
                };
                let brt = skg::backend_render_targets::make_d3d((desc.Width as i32, desc.Height as i32), &info);
                let mut c = gpu.ctx.borrow_mut();
                let s = skg::surfaces::wrap_backend_render_target(&mut c, &brt, skg::SurfaceOrigin::TopLeft, sk::ColorType::BGRA8888, Some(sk::ColorSpace::new_srgb()), Some(&crate::gfx::surface_props()))
                    .ok_or_else(|| Error::new(E_FAIL, "Skia could not draw into the swap chain"))?;
                surfs.push(s);
            }
            Ok(GpuChain { gpu: gpu.clone(), swap, swap3, waitable, w: desc.Width, h: desc.Height, surfs })
        }
    }

    /// The back buffer the next frame goes into (a handle to the same surface: drawing on the clone draws on it).
    pub fn back(&self) -> sk::Surface {
        let i = unsafe { self.swap3.GetCurrentBackBufferIndex() } as usize;
        self.surfs[i.min(self.surfs.len() - 1)].clone()
    }

    /// Flush the back buffer's drawing and present it (vsync-paced: the compositor's next frame).
    pub fn present(&mut self) -> Result<()> {
        let i = unsafe { self.swap3.GetCurrentBackBufferIndex() } as usize;
        let i = i.min(self.surfs.len() - 1);
        self.gpu.flush_present(&mut self.surfs[i]);
        unsafe { self.swap.Present(1, DXGI_PRESENT(0)).ok() }
    }
}

impl Drop for GpuChain {
    fn drop(&mut self) {
        // the GPU must be done with the buffers before the chain goes
        if !self.gpu.lost() {
            self.gpu.finish();
        }
        self.surfs.clear();
        unsafe {
            let _ = CloseHandle(self.waitable);
        }
    }
}

/// Did a present / draw call fail because the device went away?
pub fn is_lost_error(e: &Error) -> bool {
    let c = e.code();
    c == DXGI_ERROR_DEVICE_REMOVED || c == DXGI_ERROR_DEVICE_RESET || c == DXGI_ERROR_DEVICE_HUNG
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_rule() {
        assert!(reason(None, false, None, 0.0).is_none());
        assert!(reason(Some("off"), false, None, 0.0).unwrap().contains("forced"));
        assert!(reason(Some("fail"), false, None, 0.0).unwrap().contains("failed"));
        assert!(reason(None, true, None, 0.0).unwrap().contains("remote"));
        // a lost device: the CPU path for RETRY_MS, then the GPU again
        assert!(reason(None, false, Some(1000.0), 1000.0 + RETRY_MS - 1.0).unwrap().contains("lost"));
        assert!(reason(None, false, Some(1000.0), 1000.0 + RETRY_MS + 1.0).is_none());
    }

    /// On this PC's GPU (skipped where there is none): Skia draws on it and the pixels read back right; dropping the last
    /// holder releases the device; a removed device is seen as lost, the CPU path follows and the GPU comes back on retry.
    #[test]
    fn draws_on_the_gpu_releases_it_and_survives_a_lost_device() {
        let Some(g) = get(None) else { return };
        assert!(alive() && !g.lost());
        let mut s = g.surface(32, 16).expect("a GPU surface");
        assert!(s.recording_context().is_some(), "on the GPU");
        s.canvas().clear(sk::Color::from_argb(255, 10, 200, 30));
        let px = crate::png::from_surface(&mut s);
        assert_eq!(&px.data[0..4], &[30, 200, 10, 255], "BGRA read back");
        // the same device while something holds it
        let g2 = get(None).expect("the same device");
        assert!(Rc::ptr_eq(&g, &g2));
        drop((s, g2));
        drop(g);
        assert!(!alive(), "the device goes with its last holder");
        // a removed device
        let g = get(None).expect("a new device");
        g.remove_for_test();
        assert!(g.lost());
        mark_lost();
        assert!(get(None).is_none(), "the CPU path right after a loss");
        drop(g);
        LOST_AT.with(|l| l.set(Some(crate::timing::now() - RETRY_MS - 1.0)));
        assert!(get(None).is_some_and(|g| !g.lost()), "the GPU again after RETRY_MS");
    }
}
