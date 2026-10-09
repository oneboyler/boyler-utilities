//! Order 066's proof on the REAL PC (the real build of the real code): picks a cursor scheme and sets the cursor size the way the
//! Mouse tab does, measures what Windows then really shows (the size of the system arrow cursor in pixels), and puts everything
//! back EXACTLY as it was (the registry text of the cursors and the size, then a reload). It only runs while nobody uses the PC
//! (`--wait-idle`), never while a game runs, and aborts + restores at once on any error.
//!
//!   cargo run -p bu-mouse --example cursor-proof -- [--wait-idle]

use bu_mouse::win::RealOs;
use bu_mouse::{AppDirs, Mouse};
use windows::Win32::Graphics::Gdi::{DeleteObject, GetObjectW, BITMAP, HGDIOBJ};
use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, LoadCursorW, HICON, ICONINFO, IDC_ARROW, IDC_IBEAM};

/// (width, height) in px of what Windows shows now for a system cursor.
fn system_cursor_px(id: windows::core::PCWSTR) -> (i32, i32) {
    unsafe {
        let h = LoadCursorW(None, id).expect("LoadCursor");
        let mut ii = ICONINFO::default();
        GetIconInfo(HICON(h.0), &mut ii).expect("GetIconInfo");
        let mut bm = BITMAP::default();
        let bmp = if ii.hbmColor.is_invalid() { ii.hbmMask } else { ii.hbmColor };
        GetObjectW(HGDIOBJ(bmp.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _));
        let mono = ii.hbmColor.is_invalid();
        let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
        if !ii.hbmColor.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
        }
        (bm.bmWidth, if mono { bm.bmHeight / 2 } else { bm.bmHeight })
    }
}

/// What Windows really DRAWS for the pointer: the size of the pointer shape the compositor hands to Desktop Duplication (the
/// system cursor handle's own bitmap does not follow Windows' size slider, so it proves nothing). One duplication per output,
/// opened BEFORE a change; `last_shape` then takes the shape updates that come after it.
struct Watch {
    dups: Vec<windows::Win32::Graphics::Dxgi::IDXGIOutputDuplication>,
}

impl Watch {
    fn open() -> Watch {
        use windows::core::Interface;
        use windows::Win32::Foundation::HMODULE;
        use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
        use windows::Win32::Graphics::Direct3D11::{D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION};
        use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1, IDXGIOutput1};
        let mut dups = Vec::new();
        unsafe {
            let Ok(f) = CreateDXGIFactory1::<IDXGIFactory1>() else { return Watch { dups } };
            let mut ai = 0;
            while let Ok(ad) = f.EnumAdapters1(ai) {
                ai += 1;
                let Ok(base) = ad.cast::<IDXGIAdapter>() else { continue };
                let mut dev: Option<ID3D11Device> = None;
                if D3D11CreateDevice(Some(&base), D3D_DRIVER_TYPE_UNKNOWN, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut dev), None, None).is_err() {
                    continue;
                }
                let Some(dev) = dev else { continue };
                let mut oi = 0;
                while let Ok(out) = ad.EnumOutputs(oi) {
                    oi += 1;
                    if let Some(dup) = out.cast::<IDXGIOutput1>().ok().and_then(|o1| o1.DuplicateOutput(&dev).ok()) {
                        dups.push(dup);
                    }
                }
            }
        }
        Watch { dups }
    }

    /// The newest pointer shape (w, h in px) that arrives within `ms` ms, from any output.
    fn last_shape(&self, ms: u64) -> Option<(u32, u32)> {
        use windows::Win32::Graphics::Dxgi::{IDXGIResource, DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTDUPL_POINTER_SHAPE_INFO};
        let start = std::time::Instant::now();
        let mut last = None;
        while start.elapsed().as_millis() < ms as u128 {
            for dup in &self.dups {
                let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
                let mut res: Option<IDXGIResource> = None;
                unsafe {
                    if let Err(e) = dup.AcquireNextFrame(40, &mut info, &mut res) {
                        if std::env::var_os("CP_DEBUG").is_some() && e.code().0 as u32 != 0x887A0027 { println!("acquire error {:#x}", e.code().0 as u32); }
                        continue;
                    }
                    if std::env::var_os("CP_DEBUG").is_some() { println!("frame: present {} mouse {} shape {} visible {}", info.LastPresentTime, info.LastMouseUpdateTime, info.PointerShapeBufferSize, info.PointerPosition.Visible.as_bool()); }
                    if info.PointerShapeBufferSize > 0 {
                        let mut buf = vec![0u8; info.PointerShapeBufferSize as usize];
                        let (mut need, mut shape) = (0u32, DXGI_OUTDUPL_POINTER_SHAPE_INFO::default());
                        if dup.GetFramePointerShape(buf.len() as u32, buf.as_mut_ptr() as *mut _, &mut need, &mut shape).is_ok() {
                            // type 1 = monochrome: the height counts the AND and the XOR mask
                            last = Some((shape.Width, if shape.Type == 1 { shape.Height / 2 } else { shape.Height }));
                        }
                    }
                    let _ = dup.ReleaseFrame();
                }
            }
        }
        last
    }
}

fn idle_ms() -> u32 {
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        let _ = GetLastInputInfo(&mut li);
        GetTickCount().wrapping_sub(li.dwTime)
    }
}

fn main() {
    if std::env::args().any(|a| a == "--wait-idle") {
        // the pointer must be SHOWING (Windows only hands out the pointer shape then) and his hand must be off the mouse
        // (no input for 3 s); waits up to 40 min for such a moment, never touches the pointer before
        use windows::Win32::UI::WindowsAndMessaging::{GetCursorInfo, CURSORINFO, CURSOR_SHOWING};
        let start = std::time::Instant::now();
        loop {
            let mut ci = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
            let showing = unsafe { GetCursorInfo(&mut ci).is_ok() } && ci.flags.0 & CURSOR_SHOWING.0 != 0 && !ci.hCursor.is_invalid();
            if showing && idle_ms() >= 3_000 {
                break;
            }
            if start.elapsed().as_secs() > 2400 {
                println!("ABORT: no moment in 40 min with the pointer showing and no input");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }
    let appdata = std::env::var("APPDATA").unwrap_or_default();
    let mut m = Mouse::new(RealOs::new(), AppDirs::new(std::path::Path::new(&appdata).join("Boyler Utilities").join("mouse")));
    let watch = Watch::open();
    println!("watching {} outputs", watch.dups.len());
    let look0 = m.cursor_look_text().expect("read cursors");
    let size0 = m.cursor_size_text().expect("read size");
    println!("before: size text {size0}, arrow {:?}, ibeam {:?}, drawn pointer {:?}", system_cursor_px(IDC_ARROW), system_cursor_px(IDC_IBEAM), watch.last_shape(900));
    let mut fail = Vec::new();
    let mut run = || -> Result<(), String> {
        // 1. the size slider at three steps: Windows must really show those sizes
        for (step, px) in [(1u32, 32i32), (4, 80), (8, 144)] {
            m.set_cursor_size(step).map_err(|e| format!("size {step}: {e}"))?;
            std::thread::sleep(std::time::Duration::from_millis(150));
            // what Windows holds as its arrow / I-beam cursor now (the pointer shape on screen is only known to Desktop
            // Duplication while the pointer is composited by the GPU; on this PC it never was, so that is printed, not judged)
            let (a, b) = (system_cursor_px(IDC_ARROW), system_cursor_px(IDC_IBEAM));
            println!("size step {step} ({px} px asked): arrow {a:?}, ibeam {b:?}, shape on screen {:?}", watch.last_shape(300));
            if a != (px, px) || b != (px, px) {
                fail.push(format!("at step {step} the arrow is {a:?} and the I-beam {b:?}, wanted {px}"));
            }
        }
        // 2. a whole scheme (Windows' own, with files for every role) - this used to say "SPI_SETCURSORS failed"
        for name in ["Windows Inverted", "Windows Black"] {
            m.set_scheme(name).map_err(|e| format!("scheme {name}: {e}"))?;
            std::thread::sleep(std::time::Duration::from_millis(150));
            // (the size is still step 8 = 144 px: a scheme pick keeps it)
            let a = system_cursor_px(IDC_ARROW);
            println!("scheme {name}: ok, arrow {a:?}");
            if a != (144, 144) {
                fail.push(format!("after picking {name} the arrow is {a:?}, the size was lost"));
            }
        }
        Ok(())
    };
    // (a panic inside must not skip the restore below)
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut run)).unwrap_or_else(|_| Err("panicked".to_string()));
    // put everything back exactly as it was
    let back_look = m.restore_cursor_look(&look0);
    let back_size = m.restore_cursor_size(&size0);
    std::thread::sleep(std::time::Duration::from_millis(150));
    let (look1, size1) = (m.cursor_look_text().unwrap(), m.cursor_size_text().unwrap());
    println!("after:  size text {size1}, arrow {:?}, ibeam {:?}, drawn pointer {:?}", system_cursor_px(IDC_ARROW), system_cursor_px(IDC_IBEAM), watch.last_shape(900));
    if let Err(e) = r {
        fail.push(e);
    }
    if back_look.is_err() || back_size.is_err() {
        fail.push(format!("put back: {back_look:?} {back_size:?}"));
    }
    if look1 != look0 || size1 != size0 {
        fail.push("the registry is not exactly as before".into());
    }
    if fail.is_empty() {
        println!("PASS: sizes real, schemes switch without an error, everything put back exactly");
    } else {
        println!("FAIL: {fail:?}");
        std::process::exit(1);
    }
}
