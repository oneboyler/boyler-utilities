//! DXGI Desktop Duplication.
//!
//! Frozen capture: one duplication per wanted monitor is opened first (each on the GPU that drives it), then every monitor's
//! frame is taken back to back — the first AcquireNextFrame after opening hands over the current desktop image at once — and
//! queued for copying; only then are the copies read into memory. So the All picture is as close to one instant as the
//! hardware allows (the spread is measured and reported). The cursor is never in the picture (Desktop Duplication delivers the
//! pointer separately and it is ignored). There is no border: this API has none.
//!
//! HDR monitors: IDXGIOutput5::DuplicateOutput1 asks for 16-bit float frames (scRGB), converted with the monitor's SDR white.
//! Windows only allows that for per-monitor-DPI-aware processes; if refused, the 8-bit DuplicateOutput is used and the frame
//! is marked [`ColorPath::HdrByWindows`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R16G16B16A16_FLOAT;
use windows::Win32::Graphics::Dxgi::{
    IDXGIOutput1, IDXGIOutput5, IDXGIOutputDuplication, IDXGIResource, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};

use super::gpu::{self, SendCell, Slot};
use super::monitors::{self, DpiScope, Output};
use super::{us, Ctx, FRAME_TIMEOUT};
use crate::error::{Error, Result};
use crate::geom::Monitor;
use crate::image::HdrToSdr;
use crate::os::{Capture, CaptureTiming, ColorPath, LiveFeed, MonitorFrame};

fn open(o: &Output) -> Result<(IDXGIOutputDuplication, Slot)> {
    let (dev, ctx) = gpu::device(Some(&o.adapter))?;
    let m = &o.monitor;
    let slot = |dev, ctx, color, hdr| Slot::new(m.number, dev, ctx, m.rotation, color, hdr);
    if m.hdr {
        let _dpi = DpiScope::per_monitor();
        if let Ok(o5) = o.output.cast::<IDXGIOutput5>() {
            if let Ok(dup) = unsafe { o5.DuplicateOutput1(&dev, 0, &[DXGI_FORMAT_R16G16B16A16_FLOAT]) } {
                return Ok((dup, slot(dev, ctx, ColorPath::HdrConverted, Some(HdrToSdr::new(m.sdr_white_nits)))));
            }
        }
    }
    let o1: IDXGIOutput1 = o.output.cast().ctx("IDXGIOutput1")?;
    let dup = unsafe { o1.DuplicateOutput(&dev) }.ctx("DuplicateOutput")?;
    let color = if m.hdr { ColorPath::HdrByWindows } else { ColorPath::Sdr };
    Ok((dup, slot(dev, ctx, color, None)))
}

/// Waits up to `timeout` for a frame; on one, hands its texture to `copy` and releases the frame.
/// `Ok(Some(true))` = a new desktop image, `Ok(Some(false))` = only the pointer moved, `Ok(None)` = timeout.
/// `always` = copy even a pointer-only frame's surface (it still holds the desktop's last image).
fn take(dup: &IDXGIOutputDuplication, timeout: Duration, always: bool, copy: impl FnOnce(&ID3D11Texture2D) -> Result<()>) -> Result<Option<bool>> {
    let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
    let mut res: Option<IDXGIResource> = None;
    match unsafe { dup.AcquireNextFrame(timeout.as_millis() as u32, &mut info, &mut res) } {
        Ok(()) => {}
        Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
        Err(e) => return Err(Error::os("AcquireNextFrame", e.code().0)),
    }
    let image_updated = info.LastPresentTime != 0;
    let r = if image_updated || always {
        res.ok_or_else(|| Error::os("AcquireNextFrame (no resource)", 0))
            .and_then(|r| r.cast::<ID3D11Texture2D>().ctx("ID3D11Texture2D"))
            .and_then(|t| copy(&t))
    } else {
        Ok(())
    };
    let _ = unsafe { dup.ReleaseFrame() };
    r.map(|()| Some(image_updated))
}

fn open_wanted(wanted: &[Monitor]) -> Result<Vec<(IDXGIOutputDuplication, Slot)>> {
    let outs = monitors::outputs()?;
    wanted
        .iter()
        .map(|w| {
            let o = outs.iter().find(|o| o.monitor.device == w.device).ok_or(Error::NoMonitor(w.number))?;
            open(o)
        })
        .collect()
}

/// Takes frames until one carries a desktop image (the first one after opening normally does). If a frame says "only the
/// pointer moved", its surface is kept and the next frame is awaited for at most 150 ms more, then the kept surface is used.
fn first_image(dup: &IDXGIOutputDuplication, slot: &mut Slot) -> Result<()> {
    let mut deadline = Instant::now() + FRAME_TIMEOUT;
    let mut have = false;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match take(dup, left, true, |t| slot.copy(t))? {
            Some(true) => return Ok(()),
            Some(false) => {
                if !have {
                    have = true;
                    deadline = deadline.min(Instant::now() + Duration::from_millis(150));
                }
                if left.is_zero() {
                    return Ok(());
                }
            }
            None if have => return Ok(()),
            None => return Err(Error::NoFrame { monitor: slot.number, ms: FRAME_TIMEOUT.as_millis() as u32 }),
        }
    }
}

/// A frozen frame of each wanted monitor.
pub fn capture(wanted: &[Monitor]) -> Result<Capture> {
    let t0 = Instant::now();
    let mut opened = open_wanted(wanted)?;
    let setup_us = us(t0);

    let t1 = Instant::now();
    let mut arrived = Vec::with_capacity(opened.len());
    for (dup, slot) in &mut opened {
        first_image(dup, slot)?;
        arrived.push(us(t1));
    }
    let frames_us = us(t1);
    let spread_us = arrived.last().copied().unwrap_or(0) - arrived.first().copied().unwrap_or(0);

    let t2 = Instant::now();
    let frames = opened.iter().map(|(_, s)| s.read()).collect::<Result<Vec<_>>>()?;
    let readback_us = us(t2);
    Ok(Capture { frames, timing: CaptureTiming { setup_us, frames_us, spread_us, readback_us, total_us: us(t0) } })
}

struct Shared {
    slots: Vec<Mutex<SendCell<Slot>>>,
    generation: Mutex<u64>,
    changed: Condvar,
    stop: AtomicBool,
}

/// Live with Desktop Duplication: one worker thread per monitor blocks in AcquireNextFrame (Windows wakes it when the screen
/// changes; the 100 ms timeout only lets it notice the stop flag) and copies each new image into its slot. A snap reads the
/// slots; it never waits for a worker's blocking call.
pub struct DdLive {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
    seen: u64,
}

impl DdLive {
    pub fn start(wanted: &[Monitor]) -> Result<Self> {
        let mut opened = open_wanted(wanted)?;
        for (dup, slot) in &mut opened {
            first_image(dup, slot)?; // so a snap right away has a picture
        }
        let (dups, slots): (Vec<_>, Vec<_>) = opened.into_iter().unzip();
        let shared = Arc::new(Shared {
            slots: slots.into_iter().map(|s| Mutex::new(SendCell(s))).collect(),
            generation: Mutex::new(0),
            changed: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let workers = dups
            .into_iter()
            .enumerate()
            .map(|(i, dup)| {
                let sh = shared.clone();
                let dup = SendCell(dup);
                std::thread::spawn(move || worker(sh, i, dup))
            })
            .collect();
        Ok(DdLive { shared, workers, seen: 0 })
    }
}

fn worker(sh: Arc<Shared>, i: usize, dup: SendCell<IDXGIOutputDuplication>) {
    while !sh.stop.load(Ordering::Relaxed) {
        let got = take(&dup.0, Duration::from_millis(100), false, |t| {
            let mut slot = sh.slots[i].lock().unwrap_or_else(|p| p.into_inner());
            slot.0.copy(t)
        });
        match got {
            Ok(Some(true)) => {
                let mut g = sh.generation.lock().unwrap_or_else(|p| p.into_inner());
                *g += 1;
                sh.changed.notify_all();
            }
            Ok(_) => {}
            // Access lost (display mode change, secure desktop): this monitor's feed stops; its last picture stays.
            Err(_) => break,
        }
    }
}

impl LiveFeed for DdLive {
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

impl Drop for DdLive {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}
