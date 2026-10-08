//! Windows.Graphics.Capture (WGC).
//!
//! Frozen capture: one capture session per wanted monitor (GraphicsCaptureItem for its HMONITOR), all started, then each
//! monitor's first frame is taken (FrameArrived wakes the wait — no polling) and copied; then the sessions are closed.
//!
//! - Yellow border: off with `GraphicsCaptureSession.IsBorderRequired = false` after asking
//!   `GraphicsCaptureAccess.RequestAccessAsync(Borderless)` (both exist from Windows 11; on Windows 10 the property is missing
//!   and Windows always draws the border — [`border_state`] says which happened).
//! - Cursor: off with `IsCursorCaptureEnabled = false` (Windows 10 2004+).
//! - HDR monitors: the frame pool asks for `R16G16B16A16Float` (scRGB) and the frame is converted with the monitor's SDR white.
//! - WGC hands the picture already turned the way the user sees it (no rotation step).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::core::{Interface, HSTRING};
use windows::Foundation::Metadata::ApiInformation;
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureAccess, GraphicsCaptureAccessKind, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Security::Authorization::AppCapabilityAccess::AppCapabilityAccessStatus;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows_future::{AsyncOperationCompletedHandler, AsyncStatus};

use super::gpu::{self, SendCell, Slot};
use super::{us, Ctx, FRAME_TIMEOUT};
use crate::error::{Error, Result};
use crate::geom::{Monitor, Rotation};
use crate::image::HdrToSdr;
use crate::os::{Capture, CaptureTiming, ColorPath, LiveFeed, MonitorFrame};

/// What happened with the yellow capture border.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderState {
    /// IsBorderRequired = false was accepted: no border.
    Off,
    /// Windows 10: the property does not exist; Windows draws the border while capturing (a frozen capture lasts tens of ms).
    NotSupported,
    /// Borderless access was refused (status code from AppCapabilityAccessStatus); the border shows.
    Denied(i32),
}

static BORDER: OnceLock<BorderState> = OnceLock::new();

/// The border outcome of this process (asked once, on the first WGC capture). `None` before any WGC capture.
pub fn border_state() -> Option<BorderState> {
    BORDER.get().copied()
}

const SESSION: &str = "Windows.Graphics.Capture.GraphicsCaptureSession";

fn has_property(name: &str) -> bool {
    ApiInformation::IsPropertyPresent(&HSTRING::from(SESSION), &HSTRING::from(name)).unwrap_or(false)
}

/// For the proof: is WGC available, can the border be switched off (property present), can the cursor be left out.
pub fn support() -> (bool, bool, bool) {
    (GraphicsCaptureSession::IsSupported().unwrap_or(false), has_property("IsBorderRequired"), has_property("IsCursorCaptureEnabled"))
}

fn ask_borderless() -> BorderState {
    if !has_property("IsBorderRequired") {
        return BorderState::NotSupported;
    }
    let ask = || -> Result<AppCapabilityAccessStatus> {
        let op = GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless).ctx("RequestAccessAsync")?;
        let (tx, rx) = mpsc::channel();
        op.SetCompleted(&AsyncOperationCompletedHandler::new(move |_, status: AsyncStatus| {
            let _ = tx.send(status);
            Ok(())
        }))
        .ctx("SetCompleted")?;
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(s) if s == AsyncStatus::Completed => op.GetResults().ctx("RequestAccessAsync result"),
            _ => Err(Error::os("RequestAccessAsync (no answer)", 0)),
        }
    };
    match ask() {
        Ok(s) if s == AppCapabilityAccessStatus::Allowed => BorderState::Off,
        Ok(s) => BorderState::Denied(s.0),
        Err(_) => BorderState::Denied(-1),
    }
}

fn winrt_device(dev: &ID3D11Device) -> Result<IDirect3DDevice> {
    let dxgi: IDXGIDevice = dev.cast().ctx("IDXGIDevice")?;
    let insp = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }.ctx("CreateDirect3D11DeviceFromDXGIDevice")?;
    insp.cast().ctx("IDirect3DDevice")
}

fn item_for(m: &Monitor) -> Result<GraphicsCaptureItem> {
    let interop = windows_core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().ctx("IGraphicsCaptureItemInterop")?;
    unsafe { interop.CreateForMonitor(HMONITOR(m.handle as *mut _)) }.ctx("CreateForMonitor")
}

/// One monitor's running session.
struct Running {
    number: usize,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
}

impl Running {
    fn close(&self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

/// Copies a frame waiting in `pool` (if any) into `slot`. `Ok(true)` = one was there.
fn take_frame(pool: &Direct3D11CaptureFramePool, slot: &mut Slot) -> Result<bool> {
    let Ok(frame) = pool.TryGetNextFrame() else { return Ok(false) };
    let r = (|| -> Result<()> {
        let size = frame.ContentSize().ctx("ContentSize")?;
        let surface = frame.Surface().ctx("Surface")?;
        let access: IDirect3DDxgiInterfaceAccess = surface.cast().ctx("IDirect3DDxgiInterfaceAccess")?;
        let tex: ID3D11Texture2D = unsafe { access.GetInterface() }.ctx("GetInterface (texture)")?;
        slot.copy(&tex)?;
        slot.content = Some((size.Width.max(0) as u32, size.Height.max(0) as u32));
        Ok(())
    })();
    let _ = frame.Close();
    r.map(|()| true)
}

/// Opens a session per monitor. `on_frame(i)` runs (on a Windows thread-pool thread) whenever monitor i has a new frame.
/// `one_device`: all monitors share one GPU device (frozen capture: only the calling thread touches it); Live gives each
/// monitor its own, because their frames are copied on different threads.
fn start(wanted: &[Monitor], one_device: bool, on_frame: impl Fn(usize) + Send + Sync + Clone + 'static) -> Result<(Vec<Running>, Vec<Slot>)> {
    if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
        return Err(Error::MethodUnsupported("Windows.Graphics.Capture"));
    }
    let border = *BORDER.get_or_init(ask_borderless);
    let cursor_prop = has_property("IsCursorCaptureEnabled");
    let mut running = Vec::with_capacity(wanted.len());
    let mut slots = Vec::with_capacity(wanted.len());
    let shared = if one_device { Some(gpu::device(None)?) } else { None };
    for (i, m) in wanted.iter().enumerate() {
        let (dev, ctx) = match &shared {
            Some((d, c)) => (d.clone(), c.clone()),
            None => gpu::device(None)?,
        };
        let d3d = winrt_device(&dev)?;
        let item = item_for(m)?;
        let size = item.Size().ctx("GraphicsCaptureItem.Size")?;
        let (fmt, color, hdr) = if m.hdr {
            (DirectXPixelFormat::R16G16B16A16Float, ColorPath::HdrConverted, Some(HdrToSdr::new(m.sdr_white_nits)))
        } else {
            (DirectXPixelFormat::B8G8R8A8UIntNormalized, ColorPath::Sdr, None)
        };
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&d3d, fmt, 2, size).ctx("CreateFreeThreaded")?;
        let f = on_frame.clone();
        pool.FrameArrived(&TypedEventHandler::new(move |_, _| {
            f(i);
            Ok(())
        }))
        .ctx("FrameArrived")?;
        let session = pool.CreateCaptureSession(&item).ctx("CreateCaptureSession")?;
        if cursor_prop {
            session.SetIsCursorCaptureEnabled(false).ctx("IsCursorCaptureEnabled")?;
        }
        if border == BorderState::Off {
            session.SetIsBorderRequired(false).ctx("IsBorderRequired")?;
        }
        slots.push(Slot::new(m.number, dev, ctx, Rotation::None, color, hdr));
        running.push(Running { number: m.number, pool, session });
    }
    for r in &running {
        if let Err(e) = r.session.StartCapture() {
            running.iter().for_each(Running::close);
            return Err(Error::os("StartCapture", e.code().0));
        }
    }
    Ok((running, slots))
}

/// A frozen frame of each wanted monitor.
pub fn capture(wanted: &[Monitor]) -> Result<Capture> {
    let t0 = Instant::now();
    let (tx, rx) = mpsc::channel::<usize>();
    let tx = Arc::new(Mutex::new(tx));
    let (running, mut slots) = start(wanted, true, move |i| {
        let _ = tx.lock().map(|t| t.send(i));
    })?;
    let setup_us = us(t0);

    let t1 = Instant::now();
    let deadline = t1 + FRAME_TIMEOUT;
    let mut got = vec![false; running.len()];
    let mut arrived = Vec::new();
    let result = (|| -> Result<()> {
        while got.iter().any(|g| !g) {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok(i) if !got[i] => {
                    if take_frame(&running[i].pool, &mut slots[i])? {
                        got[i] = true;
                        arrived.push(us(t1));
                    }
                }
                Ok(_) => {}
                Err(_) => {
                    let missing = got.iter().position(|g| !g).map(|i| running[i].number).unwrap_or(0);
                    return Err(Error::NoFrame { monitor: missing, ms: FRAME_TIMEOUT.as_millis() as u32 });
                }
            }
        }
        Ok(())
    })();
    running.iter().for_each(Running::close);
    result?;
    let frames_us = us(t1);
    let spread_us = arrived.last().copied().unwrap_or(0) - arrived.first().copied().unwrap_or(0);

    let t2 = Instant::now();
    let frames = slots.iter().map(Slot::read).collect::<Result<Vec<_>>>()?;
    let readback_us = us(t2);
    Ok(Capture { frames, timing: CaptureTiming { setup_us, frames_us, spread_us, readback_us, total_us: us(t0) } })
}

struct Shared {
    slots: Vec<Mutex<SendCell<Slot>>>,
    pools: Vec<SendCell<Direct3D11CaptureFramePool>>,
    generation: Mutex<u64>,
    changed: Condvar,
    stopped: AtomicBool,
}

/// Live with WGC: FrameArrived (Windows calls it when the monitor shows a new frame) copies the frame into the monitor's slot.
pub struct WgcLive {
    shared: Arc<Shared>,
    running: Vec<SendCell<Running>>,
    seen: u64,
}

impl WgcLive {
    pub fn start(wanted: &[Monitor]) -> Result<Self> {
        let cell: Arc<OnceLock<Arc<Shared>>> = Arc::new(OnceLock::new());
        let c2 = cell.clone();
        let (running, slots) = start(wanted, false, move |i| {
            let Some(sh) = c2.get() else { return };
            if sh.stopped.load(Ordering::Relaxed) {
                return;
            }
            let ok = {
                let mut slot = sh.slots[i].lock().unwrap_or_else(|p| p.into_inner());
                take_frame(&sh.pools[i].0, &mut slot.0).unwrap_or(false)
            };
            if ok {
                let mut g = sh.generation.lock().unwrap_or_else(|p| p.into_inner());
                *g += 1;
                sh.changed.notify_all();
            }
        })?;
        let shared = Arc::new(Shared {
            slots: slots.into_iter().map(|s| Mutex::new(SendCell(s))).collect(),
            pools: running.iter().map(|r| SendCell(r.pool.clone())).collect(),
            generation: Mutex::new(0),
            changed: Condvar::new(),
            stopped: AtomicBool::new(false),
        });
        let _ = cell.set(shared.clone());
        let mut live = WgcLive { shared, running: running.into_iter().map(SendCell).collect(), seen: 0 };
        // The first frames may have arrived before `shared` was in place: take whatever is waiting, then wait for the rest.
        for (i, s) in live.shared.slots.iter().enumerate() {
            let mut slot = s.lock().unwrap_or_else(|p| p.into_inner());
            let _ = take_frame(&live.shared.pools[i].0, &mut slot.0);
        }
        let deadline = Instant::now() + FRAME_TIMEOUT;
        while live.shared.slots.iter().any(|s| s.lock().map(|s| s.0.staging.is_none()).unwrap_or(true)) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Error::NoFrame { monitor: 0, ms: FRAME_TIMEOUT.as_millis() as u32 });
            }
            live.wait(left)?;
        }
        Ok(live)
    }
}

impl LiveFeed for WgcLive {
    fn wait(&mut self, timeout: Duration) -> Result<bool> {
        let seen = self.seen;
        let g = self.shared.generation.lock().unwrap_or_else(|p| p.into_inner());
        let (g, _) = self.shared.changed.wait_timeout_while(g, timeout, |g| *g == seen).unwrap_or_else(|p| p.into_inner());
        let new = *g != seen;
        self.seen = *g;
        Ok(new)
    }

    fn latest(&mut self) -> Result<Vec<MonitorFrame>> {
        self.shared.slots.iter().map(|s| s.lock().unwrap_or_else(|p| p.into_inner()).0.read()).collect()
    }
}

impl Drop for WgcLive {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Relaxed);
        for r in &self.running {
            r.0.close();
        }
    }
}
