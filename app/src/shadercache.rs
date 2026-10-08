//! The GPU shaders kept on disk (Order 051). Skia's Direct3D backend turns every new drawing kind into HLSL and compiles it
//! with `D3DCompile` (d3dcompiler_47.dll) - every time, in every new device: its own persistent cache stores only the HLSL
//! and compiles it again (GrD3DPipelineStateBuilder, Skia m153). Measured: the first GPU frame of an open took 305-1265 ms
//! (the CPU path: 77 ms). So the app defines `D3DCompile` itself: the linker takes a symbol defined in our own code before
//! the import library's, so Skia's calls land here. A shader compiled once is kept as a file (its source stored with it and
//! compared on load - two different shaders can never share a file), and the real compiler (loaded from System32 by full
//! path) runs only for a shader not seen before. Normal copy: `%LOCALAPPDATA%\BoylerUtilities\GPUCache` (the uninstaller removes it); a test copy:
//! `%TEMP%\BoylerUtilities-test\GPUCache` (or `BU_GPUCACHE`).

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::OnceLock;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};

type Compile = unsafe extern "system" fn(*const c_void, usize, PCSTR, *const c_void, *mut c_void, PCSTR, PCSTR, u32, u32, *mut *mut c_void, *mut *mut c_void) -> HRESULT;
type CreateBlob = unsafe extern "system" fn(usize, *mut *mut c_void) -> HRESULT;

/// ID3DBlob's vtable: IUnknown (3), GetBufferPointer, GetBufferSize.
#[repr(C)]
struct BlobVtbl {
    _qi: usize,
    _addref: usize,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    ptr: unsafe extern "system" fn(*mut c_void) -> *mut c_void,
    size: unsafe extern "system" fn(*mut c_void) -> usize,
}

unsafe fn blob_bytes<'a>(b: *mut c_void) -> &'a [u8] {
    let vt = &**(b as *mut *const BlobVtbl);
    std::slice::from_raw_parts((vt.ptr)(b) as *const u8, (vt.size)(b))
}

struct Real {
    compile: Compile,
    blob: CreateBlob,
}

/// The system's compiler can be loaded (else the GPU path is not used - gpu.rs).
pub fn available() -> bool {
    real().is_some()
}

fn real() -> Option<&'static Real> {
    static R: OnceLock<Option<Real>> = OnceLock::new();
    R.get_or_init(|| unsafe {
        let m = LoadLibraryExW(w!("d3dcompiler_47.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
        let c = GetProcAddress(m, s!("D3DCompile"))?;
        let b = GetProcAddress(m, s!("D3DCreateBlob"))?;
        Some(Real { compile: std::mem::transmute::<unsafe extern "system" fn() -> isize, Compile>(c), blob: std::mem::transmute::<unsafe extern "system" fn() -> isize, CreateBlob>(b) })
    })
    .as_ref()
}

/// Where the compiled shaders are kept (None = no cache: compile every time).
fn dir() -> Option<&'static PathBuf> {
    static D: OnceLock<Option<PathBuf>> = OnceLock::new();
    D.get_or_init(|| {
        let d = if cfg!(test) || crate::testmode::on() {
            match crate::testmode::env("BU_GPUCACHE") {
                Some(v) => PathBuf::from(v),
                None => std::env::temp_dir().join("BoylerUtilities-test").join("GPUCache"),
            }
        } else {
            PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("BoylerUtilities").join("GPUCache")
        };
        std::fs::create_dir_all(&d).ok()?;
        Some(d)
    })
    .as_ref()
}

/// The cache file's content: the key (everything the compiler was given) then the compiled bytes.
fn key(src: &[u8], entry: &[u8], target: &[u8], f1: u32, f2: u32) -> Vec<u8> {
    let mut k = Vec::with_capacity(src.len() + 64);
    for part in [src, entry, target] {
        k.extend_from_slice(&(part.len() as u32).to_le_bytes());
        k.extend_from_slice(part);
    }
    k.extend_from_slice(&f1.to_le_bytes());
    k.extend_from_slice(&f2.to_le_bytes());
    k
}

unsafe fn cstr<'a>(p: PCSTR) -> &'a [u8] {
    if p.is_null() {
        &[]
    } else {
        std::ffi::CStr::from_ptr(p.0 as *const std::ffi::c_char).to_bytes()
    }
}

thread_local! {
    /// test proof: (hits, compiled)
    pub static STATS: std::cell::Cell<(u32, u32)> = const { std::cell::Cell::new((0, 0)) };
}

/// Skia's `D3DCompile` (the same signature as d3dcompiler's). Defines, includes and a source name are passed on to the real
/// compiler; a call with defines or an include handler is never cached (Skia passes neither).
#[no_mangle]
pub unsafe extern "system" fn D3DCompile(
    src: *const c_void,
    len: usize,
    name: PCSTR,
    defines: *const c_void,
    include: *mut c_void,
    entry: PCSTR,
    target: PCSTR,
    f1: u32,
    f2: u32,
    code: *mut *mut c_void,
    errors: *mut *mut c_void,
) -> HRESULT {
    let Some(r) = real() else {
        if !code.is_null() {
            *code = std::ptr::null_mut();
        }
        if !errors.is_null() {
            *errors = std::ptr::null_mut();
        }
        return E_FAIL;
    };
    let cacheable = defines.is_null() && include.is_null() && !src.is_null() && !code.is_null();
    let file = cacheable.then(dir).flatten().map(|d| {
        let k = key(std::slice::from_raw_parts(src as *const u8, len), cstr(entry), cstr(target), f1, f2);
        (d.join(format!("{}.dxbc", &bu_updater::sha256::sha256_hex(&k)[..32])), k)
    });
    if let Some((path, k)) = &file {
        if let Ok(data) = std::fs::read(path) {
            if data.len() > k.len() && data[..k.len()] == k[..] {
                let bin = &data[k.len()..];
                let mut b: *mut c_void = std::ptr::null_mut();
                if (r.blob)(bin.len(), &mut b).is_ok() && !b.is_null() {
                    let vt = &**(b as *mut *const BlobVtbl);
                    std::ptr::copy_nonoverlapping(bin.as_ptr(), (vt.ptr)(b) as *mut u8, bin.len());
                    *code = b;
                    if !errors.is_null() {
                        *errors = std::ptr::null_mut();
                    }
                    STATS.with(|s| s.set((s.get().0 + 1, s.get().1)));
                    return S_OK;
                }
            }
        }
    }
    let hr = (r.compile)(src, len, name, defines, include, entry, target, f1, f2, code, errors);
    STATS.with(|s| s.set((s.get().0, s.get().1 + 1)));
    if hr.is_ok() && !(*code).is_null() {
        if let Some((path, mut k)) = file {
            k.extend_from_slice(blob_bytes(*code));
            // written whole to a temp name, then renamed: a half-written file is never read
            let tmp = path.with_extension("tmp");
            if std::fs::write(&tmp, &k).is_ok() && std::fs::rename(&tmp, &path).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }
    hr
}

/// Let go of a blob (tests).
#[allow(dead_code)]
unsafe fn release(b: *mut c_void) {
    let vt = &**(b as *mut *const BlobVtbl);
    (vt.release)(b);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shader compiled once comes from the cache the next time, byte for byte the same.
    #[test]
    fn a_shader_is_compiled_once_then_read_from_the_cache() {
        if real().is_none() {
            return;
        }
        let src = format!("float4 main() : SV_Target {{ return float4(0.25, 0.5, 0.75, {}); }}\0", std::process::id() % 1000);
        let run = || unsafe {
            let mut code: *mut c_void = std::ptr::null_mut();
            let mut err: *mut c_void = std::ptr::null_mut();
            let hr = D3DCompile(src.as_ptr() as *const c_void, src.len() - 1, PCSTR::null(), std::ptr::null(), std::ptr::null_mut(), s!("main"), s!("ps_5_0"), 0, 0, &mut code, &mut err);
            assert!(hr.is_ok() && !code.is_null());
            let b = blob_bytes(code).to_vec();
            release(code);
            b
        };
        let (h0, _) = STATS.with(|s| s.get());
        let a = run();
        let b = run();
        let (h1, _) = STATS.with(|s| s.get());
        assert_eq!(a, b);
        assert!(h1 > h0, "the second compile came from the cache");
        // a cache file never matches another source (the key is stored and compared)
        assert_ne!(key(b"a", b"main", b"ps_5_0", 0, 0), key(b"b", b"main", b"ps_5_0", 0, 0));
        assert_ne!(key(b"ab", b"", b"x", 0, 0), key(b"a", b"b", b"x", 0, 0));
    }
}
