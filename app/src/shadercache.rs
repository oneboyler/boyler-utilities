//! The GPU shaders kept on disk (Order 051). Skia's Direct3D backend turns every new drawing kind into HLSL and compiles it
//! with `D3DCompile` (d3dcompiler_47.dll) - every time, in every new device: its own persistent cache stores only the HLSL
//! and compiles it again (GrD3DPipelineStateBuilder, Skia m153). Measured: the first GPU frame of an open took 305-1265 ms
//! (the CPU path: 77 ms). So the app defines `D3DCompile` itself: the linker takes a symbol defined in our own code before
//! the import library's, so Skia's calls land here. A shader compiled once is kept as a file (its source stored with it and
//! compared on load - two different shaders can never share a file), and the real compiler (loaded from System32 by full
//! path) runs only for a shader not seen before. Normal copy: `%LOCALAPPDATA%\BoylerUtilities\GPUCache` (the uninstaller removes it); a test copy:
//! `%TEMP%\BoylerUtilities-test\GPUCache` (or `BU_GPUCACHE`).
//!
//! Order 052 item 0 (the owner Oct 9: "first time double clicking to open it and it freezes my pc again" - his first open of
//! 1.0.1 compiled 122 shaders on the menu's thread, 64 in one second): (a) the compiled shaders ship INSIDE the app
//! (`app/gpu/shaders.pack`, made by tools/gpucache/make.ps1 from a run that draws every tab, popup and the capture overlay -
//! the DXBC from D3DCompile does not depend on the GPU); (b) a shader that is in neither the pack nor the disk cache is
//! never compiled on the drawing thread: the call fails at once, the shader goes to ONE background-priority worker that
//! compiles them one by one with a pause between, and the frame that needed it is drawn on the CPU path instead
//! (`take_missed`, gpu.rs / menu.rs); (c) while that worker has work (`warm` false) the menu and the overlay draw on the
//! CPU path, and the menu is woken when it is done.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

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
    /// Order 052: shaders this thread asked for that were not ready (handed to the worker) since the last `take_missed`
    static MISSED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// The compiled shaders shipped with the app (tools/gpucache/make.ps1): "BUSHPK01", a u32 count, then `count` entries
/// (sha256 of the key: 32 bytes, offset u32, length u32) sorted by the hash, then the DXBC bytes.
static PACK: &[u8] = include_bytes!("../gpu/shaders.pack");

/// The shipped shader for this key's hash.
fn pack_find(pack: &'static [u8], hash: &[u8; 32]) -> Option<&'static [u8]> {
    if pack.len() < 12 || &pack[..8] != b"BUSHPK01" {
        return None;
    }
    let n = u32::from_le_bytes(pack[8..12].try_into().ok()?) as usize;
    let ent = |i: usize| -> Option<&'static [u8]> { pack.get(12 + i * 40..12 + i * 40 + 40) };
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let e = ent(mid)?;
        match e[..32].cmp(&hash[..]) {
            std::cmp::Ordering::Less => lo = mid + 1,
            std::cmp::Ordering::Greater => hi = mid,
            std::cmp::Ordering::Equal => {
                let off = u32::from_le_bytes(e[32..36].try_into().ok()?) as usize;
                let len = u32::from_le_bytes(e[36..40].try_into().ok()?) as usize;
                return pack.get(off..off + len);
            }
        }
    }
    None
}

fn hash32(k: &[u8]) -> [u8; 32] {
    let hex = bu_updater::sha256::sha256_hex(k);
    let mut h = [0u8; 32];
    for (i, b) in h.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0);
    }
    h
}

/// One shader waiting for the worker.
struct Job {
    hash: [u8; 32],
    key: Vec<u8>,
    src: Vec<u8>,
    entry: std::ffi::CString,
    target: std::ffi::CString,
    f1: u32,
    f2: u32,
    path: Option<PathBuf>,
}

#[derive(Default)]
struct Work {
    jobs: VecDeque<Job>,
    /// queued or being compiled (never twice)
    queued: HashSet<[u8; 32]>,
    /// the worker thread runs
    running: bool,
    /// compiled by the worker in this run of the app (also when the cache folder can't be written)
    done: HashMap<[u8; 32], Vec<u8>>,
}

fn work() -> &'static Mutex<Work> {
    static W: OnceLock<Mutex<Work>> = OnceLock::new();
    W.get_or_init(|| Mutex::new(Work::default()))
}

/// test proof (`gpustate`): shaders handed to the worker, compiled by it
pub static DEFERRED: AtomicU32 = AtomicU32::new(0);
pub static WORKER_COMPILED: AtomicU32 = AtomicU32::new(0);
/// What D3DCompile answers for a shader handed to the worker (E_PENDING: "the data necessary to complete this operation
/// is not yet available"); a present that saw one answers it too (gpu.rs).
pub const NOT_READY: HRESULT = HRESULT(0x8000_000Au32 as i32);
/// the pause between two compiles on the worker (ms): a few at a time, never all at once
const WORKER_PAUSE_MS: u64 = 30;

/// No shader is waiting for the worker: the GPU path may be used (gpu.rs).
pub fn warm() -> bool {
    work().lock().map(|w| w.jobs.is_empty() && w.queued.is_empty()).unwrap_or(true)
}

/// How many shaders the worker still has (the log's reason).
pub fn pending() -> usize {
    work().lock().map(|w| w.queued.len()).unwrap_or(0)
}

/// Were shaders missing (handed to the worker) since the last call on this thread? The frame drawn meanwhile lacks them.
pub fn take_missed() -> u32 {
    MISSED.with(|m| m.replace(0))
}

/// Tests (and the pack's own making, `BU_SHADERS=inline` in a test copy): compile on the calling thread as before.
fn inline() -> bool {
    cfg!(test) || matches!(crate::testmode::env("BU_SHADERS").as_deref(), Some("inline") | Some("make"))
}

/// The pack's own making (`BU_SHADERS=make`, tools/gpucache/make.ps1): the shipped pack is not used, every shader is
/// compiled (or read from the scratch cache) into the cache folder the next pack is made from.
fn making() -> bool {
    !cfg!(test) && crate::testmode::env("BU_SHADERS").as_deref() == Some("make")
}

/// Hand a shader to the worker (started if it is not running).
fn defer(job: Job) {
    let Ok(mut w) = work().lock() else { return };
    if !w.queued.insert(job.hash) {
        return;
    }
    w.jobs.push_back(job);
    DEFERRED.fetch_add(1, Ordering::Relaxed);
    if w.running {
        return;
    }
    w.running = true;
    let started = std::thread::Builder::new().name("shader compile".into()).spawn(worker).is_ok();
    if !started {
        w.running = false;
    }
}

/// The one worker: background priority (CPU and disk), one shader at a time with a pause between; wakes the menu when
/// the last one is done (it goes back to the GPU path).
fn worker() {
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
    loop {
        let job = {
            let Ok(mut w) = work().lock() else { return };
            match w.jobs.pop_front() {
                Some(j) => j,
                None => {
                    w.running = false;
                    break;
                }
            }
        };
        let bin = real().and_then(|r| unsafe {
            let mut code: *mut c_void = std::ptr::null_mut();
            let mut err: *mut c_void = std::ptr::null_mut();
            let hr = (r.compile)(job.src.as_ptr() as *const c_void, job.src.len(), PCSTR::null(), std::ptr::null(), std::ptr::null_mut(), PCSTR(job.entry.as_ptr() as *const u8), PCSTR(job.target.as_ptr() as *const u8), job.f1, job.f2, &mut code, &mut err);
            if !err.is_null() {
                release(err);
            }
            if hr.is_ok() && !code.is_null() {
                let b = blob_bytes(code).to_vec();
                release(code);
                Some(b)
            } else {
                None
            }
        });
        if let Some(bin) = &bin {
            WORKER_COMPILED.fetch_add(1, Ordering::Relaxed);
            if let Some(path) = &job.path {
                let mut k = job.key.clone();
                k.extend_from_slice(bin);
                let tmp = path.with_extension("tmp");
                if std::fs::write(&tmp, &k).is_ok() && std::fs::rename(&tmp, path).is_err() {
                    let _ = std::fs::remove_file(&tmp);
                }
            }
        }
        if let Ok(mut w) = work().lock() {
            if let Some(b) = bin {
                w.done.insert(job.hash, b);
            }
            // (a shader the compiler refused is not asked for again: Skia's own error path then, as before)
            w.queued.remove(&job.hash);
        }
        std::thread::sleep(std::time::Duration::from_millis(WORKER_PAUSE_MS));
    }
    crate::services::Waker.wake();
}

/// A blob holding `bin` (made with the real compiler's D3DCreateBlob: Skia releases it as its own).
unsafe fn make_blob(r: &Real, bin: &[u8]) -> Option<*mut c_void> {
    let mut b: *mut c_void = std::ptr::null_mut();
    if (r.blob)(bin.len(), &mut b).is_ok() && !b.is_null() {
        let vt = &**(b as *mut *const BlobVtbl);
        std::ptr::copy_nonoverlapping(bin.as_ptr(), (vt.ptr)(b) as *mut u8, bin.len());
        Some(b)
    } else {
        None
    }
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
    if !errors.is_null() {
        *errors = std::ptr::null_mut();
    }
    if cacheable {
        let k = key(std::slice::from_raw_parts(src as *const u8, len), cstr(entry), cstr(target), f1, f2);
        let h = hash32(&k);
        // 1. shipped with the app, 2. compiled by the worker in this run, 3. the disk cache
        let found: Option<Vec<u8>> = pack_find(PACK, &h).filter(|_| !making()).map(|b| b.to_vec()).or_else(|| work().lock().ok().and_then(|w| w.done.get(&h).cloned()));
        let path = dir().map(|d| d.join(format!("{}.dxbc", &bu_updater::sha256::sha256_hex(&k)[..32])));
        let found = found.or_else(|| {
            let data = std::fs::read(path.as_ref()?).ok()?;
            (data.len() > k.len() && data[..k.len()] == k[..]).then(|| data[k.len()..].to_vec())
        });
        if let Some(bin) = found {
            if let Some(b) = make_blob(r, &bin) {
                *code = b;
                STATS.with(|s| s.set((s.get().0 + 1, s.get().1)));
                return S_OK;
            }
        }
        if !inline() {
            // never compiled here (the drawing thread): the worker does it; this frame is drawn on the CPU path
            *code = std::ptr::null_mut();
            // (Skia reads the error text of a failed compile without checking it: an access violation without one, seen)
            if !errors.is_null() {
                if let Some(e) = make_blob(r, b"compiled in the background (Boyler Utilities) ") {
                    *errors = e;
                }
            }
            MISSED.with(|m| m.set(m.get() + 1));
            defer(Job {
                hash: h,
                key: k,
                src: std::slice::from_raw_parts(src as *const u8, len).to_vec(),
                entry: std::ffi::CString::new(cstr(entry)).unwrap_or_default(),
                target: std::ffi::CString::new(cstr(target)).unwrap_or_default(),
                f1,
                f2,
                path,
            });
            return NOT_READY;
        }
        let hr = (r.compile)(src, len, name, defines, include, entry, target, f1, f2, code, errors);
        STATS.with(|s| s.set((s.get().0, s.get().1 + 1)));
        if hr.is_ok() && !(*code).is_null() {
            if let Some(path) = path {
                let mut k = k;
                k.extend_from_slice(blob_bytes(*code));
                // written whole to a temp name, then renamed: a half-written file is never read
                let tmp = path.with_extension("tmp");
                if std::fs::write(&tmp, &k).is_ok() && std::fs::rename(&tmp, &path).is_err() {
                    let _ = std::fs::remove_file(&tmp);
                }
            }
        }
        return hr;
    }
    let hr = (r.compile)(src, len, name, defines, include, entry, target, f1, f2, code, errors);
    STATS.with(|s| s.set((s.get().0, s.get().1 + 1)));
    hr
}

/// Let go of a blob.
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

    /// Order 052: the shipped pack's lookup finds every entry by its key's hash and nothing else; a broken pack finds nothing.
    #[test]
    fn the_shipped_pack_is_found_by_the_keys_hash() {
        let keys: Vec<Vec<u8>> = (0..9u8).map(|i| key(&[i; 5], b"main", b"ps_5_0", i as u32, 0)).collect();
        let mut ents: Vec<([u8; 32], Vec<u8>)> = keys.iter().enumerate().map(|(i, k)| (hash32(k), vec![i as u8; 3 + i])).collect();
        ents.sort();
        let mut pack = b"BUSHPK01".to_vec();
        pack.extend_from_slice(&(ents.len() as u32).to_le_bytes());
        let mut off = 12 + 40 * ents.len();
        for (h, b) in &ents {
            pack.extend_from_slice(h);
            pack.extend_from_slice(&(off as u32).to_le_bytes());
            pack.extend_from_slice(&(b.len() as u32).to_le_bytes());
            off += b.len();
        }
        for (_, b) in &ents {
            pack.extend_from_slice(b);
        }
        let pack: &'static [u8] = Box::leak(pack.into_boxed_slice());
        for (i, k) in keys.iter().enumerate() {
            assert_eq!(pack_find(pack, &hash32(k)), Some(&vec![i as u8; 3 + i][..]));
        }
        assert_eq!(pack_find(pack, &hash32(b"not there")), None);
        assert_eq!(pack_find(&pack[..20], &hash32(&keys[0])), None);
        assert_eq!(pack_find(b"BUSHPK00\0\0\0\0", &hash32(&keys[0])), None);
        // the shipped pack itself is well formed (or empty)
        assert!(PACK.len() >= 12 && &PACK[..8] == b"BUSHPK01");
    }
}
