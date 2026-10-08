//! Handing Skia's CPU pixels to the window (Order 003): a Direct3D 11 composition swap chain (premultiplied BGRA, flip
//! model, frame-latency waitable — the frame pacing and Windows' glass from Order 001 stay as they were); each frame the
//! finished pixels are copied into the back buffer with one `UpdateSubresource` and presented. No Direct2D any more.
//! Why this way: it is one memory copy per frame (600 x 520 x 4 = 1.2 MB), keeps the vsync-paced waitable and the
//! composition visual the glass sits under, and needs no GPU drawing code of ours.

use std::cell::RefCell;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

thread_local! {
    /// A Direct3D device made on tray hover and handed to the next open (see App::warm_up); None = make one per open.
    pub static KEPT_D3D: RefCell<Option<ID3D11Device>> = const { RefCell::new(None) };
}

pub fn new_d3d() -> Result<ID3D11Device> {
    unsafe {
        let mut d3d = None;
        let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
        let mut r = D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), flags, None, D3D11_SDK_VERSION, Some(&mut d3d), None, None);
        if r.is_err() {
            r = D3D11CreateDevice(None, D3D_DRIVER_TYPE_WARP, HMODULE::default(), flags, None, D3D11_SDK_VERSION, Some(&mut d3d), None, None);
        }
        r?;
        Ok(d3d.unwrap())
    }
}

/// The composition swap chain the menu's pixels go into.
pub struct Chain {
    pub d3d: ID3D11Device,
    ctx: ID3D11DeviceContext,
    pub swap: IDXGISwapChain2,
    pub waitable: HANDLE,
    pub w: u32,
    pub h: u32,
}

impl Chain {
    pub fn new(w: u32, h: u32) -> Result<Chain> {
        let kept = KEPT_D3D.with(|k| k.borrow_mut().take());
        let d3d: ID3D11Device = match kept {
            Some(d) if unsafe { d.GetDeviceRemovedReason() }.is_ok() => d,
            _ => new_d3d()?,
        };
        crate::timing::note("open_step d3d");
        Self::new_on(d3d, w, h)
    }

    /// A swap chain on an existing device (the window's edge mask uses the menu's device).
    pub fn new_on(d3d: ID3D11Device, w: u32, h: u32) -> Result<Chain> {
        Self::new_alpha(d3d, w, h, DXGI_ALPHA_MODE_PREMULTIPLIED)
    }

    /// `alpha` = DXGI_ALPHA_MODE_IGNORE for an opaque picture (option 3's desktop capture: Desktop Duplication's
    /// alpha is 0, which the compositor would take as see-through).
    pub fn new_alpha(d3d: ID3D11Device, w: u32, h: u32, alpha: DXGI_ALPHA_MODE) -> Result<Chain> {
        unsafe {
            let ctx = d3d.GetImmediateContext()?;
            let dxgi: IDXGIDevice = d3d.cast()?;
            let adapter = dxgi.GetAdapter()?;
            let factory: IDXGIFactory2 = adapter.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: w,
                Height: h,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: alpha,
                Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
            };
            let sc1 = factory.CreateSwapChainForComposition(&d3d, &desc, None)?;
            let swap: IDXGISwapChain2 = sc1.cast()?;
            swap.SetMaximumFrameLatency(1)?;
            let waitable = swap.GetFrameLatencyWaitableObject();
            Ok(Chain { d3d, ctx, swap, waitable, w, h })
        }
    }

    /// Copy premultiplied BGRA pixels (`w` x `h`, `pitch` bytes per row) into the back buffer and present.
    pub fn present(&self, px: &[u8], pitch: u32) -> Result<()> {
        unsafe {
            let tex: ID3D11Texture2D = self.swap.GetBuffer(0)?;
            self.ctx.UpdateSubresource(&tex, 0, None, px.as_ptr() as *const _, pitch, 0);
            self.swap.Present(1, DXGI_PRESENT(0)).ok()
        }
    }

    /// Let go of the GPU work now (the menu is closing).
    pub fn release(&self) {
        unsafe {
            self.ctx.ClearState();
            self.ctx.Flush();
        }
    }
}

impl Drop for Chain {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.waitable);
        }
    }
}
