//! Fake OS layers for tests (this crate's and the menu app's): scripted adapters, signatures and an installer that
//! records what it was asked and never runs anything.

use crate::detect::{Adapter, GpuOs};
use crate::download::Signatures;
use crate::error::{DriverError, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// NVIDIA's recorded answers (Oct 30 2026): the product list (cut to a few cards), the series list, and the RTX 4090 /
/// Windows 11 driver list cut to 621.01, 617.42 and 610.88.
pub const SAMPLE_PRODUCTS: &str = include_str!("../tests/data/products.xml");
pub const SAMPLE_SERIES: &str = include_str!("../tests/data/series.xml");
pub const SAMPLE_DRIVERS: &str = include_str!("../tests/data/drivers.json");

/// A stand-in NVIDIA site in memory: the recorded lookups for an RTX 4090 on Windows 11, and every listed driver's link
/// answering a small stand-in package ("MZ" + `package_len` bytes). Never the internet.
pub fn sample_site(package_len: usize) -> bu_updater::FakeHttp {
    use crate::lookup::{drivers_url, parse_drivers, Product, PRODUCTS_URL, SERIES_URL};
    let h = bu_updater::FakeHttp::new();
    h.ok(PRODUCTS_URL, SAMPLE_PRODUCTS);
    h.ok(SERIES_URL, SAMPLE_SERIES);
    h.ok(&drivers_url(Product { psid: 127, pfid: 995 }, 135), SAMPLE_DRIVERS);
    let mut pkg = b"MZ".to_vec();
    pkg.resize(package_len.max(2), 0);
    for r in parse_drivers(SAMPLE_DRIVERS).unwrap_or_default() {
        h.ok(&r.url, pkg.clone());
    }
    h
}

/// Where a stand-in package puts its archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hide {
    /// like NVIDIA's: after the sections, before the signature (covered by the signature's hash)
    Overlay,
    /// in the signature table's padding (the table is bigger than its one certificate)
    CertPadding,
    /// inside the certificate itself (its length grown over the archive)
    InCertificate,
    /// after the signature table (the file is longer than the table says)
    AfterEnd,
}

/// A minimal 64-bit Windows program (one section) with `archive` placed as `hide` says and a stand-in certificate table
/// at the end - the shape of NVIDIA's self-extracting package.
pub fn signed_pe(archive: &[u8], hide: Hide) -> Vec<u8> {
    let put32 = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    let mut b = vec![0u8; 0x400];
    b[0..2].copy_from_slice(b"MZ");
    put32(&mut b, 0x3C, 0x40);
    b[0x40..0x44].copy_from_slice(b"PE\0\0");
    b[0x44..0x46].copy_from_slice(&0x8664u16.to_le_bytes());
    b[0x46..0x48].copy_from_slice(&1u16.to_le_bytes());
    b[0x54..0x56].copy_from_slice(&240u16.to_le_bytes());
    b[0x58..0x5A].copy_from_slice(&0x20Bu16.to_le_bytes());
    // the one section: 0x200 bytes at 0x200
    put32(&mut b, 0x148 + 16, 0x200);
    put32(&mut b, 0x148 + 20, 0x200);
    let cert = |extra: &[u8]| {
        let mut c = Vec::new();
        c.extend_from_slice(&((8 + 13 + extra.len()) as u32).to_le_bytes());
        c.extend_from_slice(&0x200u16.to_le_bytes());
        c.extend_from_slice(&2u16.to_le_bytes());
        c.extend_from_slice(b"pkcs7 bytes..");
        c.extend_from_slice(extra);
        while !c.len().is_multiple_of(8) {
            c.push(0);
        }
        c
    };
    let mut tail = Vec::new();
    match hide {
        Hide::Overlay | Hide::AfterEnd => {
            if hide == Hide::Overlay {
                b.extend_from_slice(archive);
            }
            while !b.len().is_multiple_of(8) {
                b.push(0);
            }
            if hide == Hide::AfterEnd {
                tail = archive.to_vec();
            }
            let c = cert(&[]);
            let at = b.len();
            put32(&mut b, 0xC8 + 32, at as u32);
            put32(&mut b, 0xC8 + 36, c.len() as u32);
            b.extend_from_slice(&c);
        }
        Hide::CertPadding => {
            let mut c = cert(&[]);
            c.extend_from_slice(archive);
            while !c.len().is_multiple_of(8) {
                c.push(0);
            }
            let at = b.len() as u32;
            put32(&mut b, 0xC8 + 32, at);
            put32(&mut b, 0xC8 + 36, c.len() as u32);
            b.extend_from_slice(&c);
        }
        Hide::InCertificate => {
            let c = cert(archive);
            let at = b.len() as u32;
            put32(&mut b, 0xC8 + 32, at);
            put32(&mut b, 0xC8 + 36, c.len() as u32);
            b.extend_from_slice(&c);
        }
    }
    b.extend_from_slice(&tail);
    b
}

/// A fresh, empty test folder in the system temp folder (one per name and test process).
pub fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("bu-gpudriver-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("test folder");
    d
}

/// Scripted signatures: every file has the default signer unless its file name has its own; `None` = Windows doesn't
/// trust it. Logs every file checked.
#[derive(Debug, Default)]
pub struct FakeSignatures {
    pub default: Option<String>,
    pub by_name: HashMap<String, Option<String>>,
    pub checked: Mutex<Vec<PathBuf>>,
}

impl FakeSignatures {
    pub fn nvidia() -> Self {
        Self::signed_by(crate::download::NVIDIA_SIGNER)
    }
    pub fn signed_by(who: &str) -> Self {
        FakeSignatures { default: Some(who.to_string()), ..Default::default() }
    }
    pub fn untrusted() -> Self {
        FakeSignatures::default()
    }
    /// The file called `name` (any folder) gets its own signer.
    pub fn with(mut self, name: &str, who: Option<&str>) -> Self {
        self.by_name.insert(name.to_string(), who.map(str::to_string));
        self
    }
    /// The file names checked so far, in order.
    pub fn checked_names(&self) -> Vec<String> {
        self.checked.lock().unwrap().iter().map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned()).collect()
    }
}

/// One `run` the fake installer was asked for, with what the program would have found in its folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeRun {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Every file under the folder it ran in, `/`-separated, sorted.
    pub files: Vec<String>,
    /// setup.cfg as it was at the run (empty when missing).
    pub cfg: String,
}

/// An installer that never runs anything: the work folder is a test folder, `run` records and answers `code`.
#[derive(Debug)]
pub struct FakeInstallOs {
    pub work: PathBuf,
    pub code: u32,
    pub fail_run: Option<String>,
    runs: Mutex<Vec<FakeRun>>,
}

impl FakeInstallOs {
    pub fn new(work: PathBuf) -> Self {
        let _ = std::fs::remove_dir_all(&work);
        FakeInstallOs { work, code: 0, fail_run: None, runs: Mutex::default() }
    }
    pub fn exit_code(mut self, code: u32) -> Self {
        self.code = code;
        self
    }
    pub fn runs(&self) -> Vec<FakeRun> {
        self.runs.lock().unwrap().clone()
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(root, &p, out);
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

impl crate::install::InstallOs for FakeInstallOs {
    fn make_work_dir(&self) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.work).map_err(|e| DriverError::Os(e.to_string()))?;
        Ok(self.work.clone())
    }
    fn run(&self, program: &Path, args: &[&str], cwd: &Path) -> Result<u32> {
        let mut files = Vec::new();
        walk(cwd, cwd, &mut files);
        files.sort();
        let cfg = std::fs::read_to_string(cwd.join("setup.cfg")).unwrap_or_default();
        self.runs.lock().unwrap().push(FakeRun { program: program.to_path_buf(), args: args.iter().map(|a| a.to_string()).collect(), files, cfg });
        match &self.fail_run {
            Some(e) => Err(DriverError::Os(e.clone())),
            None => Ok(self.code),
        }
    }
}

impl Signatures for FakeSignatures {
    fn signer(&self, file: &Path) -> Result<String> {
        self.checked.lock().unwrap().push(file.to_path_buf());
        if !file.is_file() {
            return Err(DriverError::NotSigned("no such file".into()));
        }
        let name = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let who = self.by_name.get(&name).cloned().unwrap_or_else(|| self.default.clone());
        who.ok_or_else(|| DriverError::NotSigned("Windows does not trust the signature".into()))
    }
}

/// Scripted display adapters.
#[derive(Debug, Clone, Default)]
pub struct FakeGpuOs {
    pub adapters: Vec<Adapter>,
    pub build: u32,
    pub laptop: bool,
    /// `Some` = `adapters()` fails with this.
    pub fail: Option<String>,
}

impl FakeGpuOs {
    /// A Windows 11 desktop with these adapters.
    pub fn new(adapters: Vec<Adapter>) -> Self {
        FakeGpuOs { adapters, build: 22631, laptop: false, fail: None }
    }
}

impl GpuOs for FakeGpuOs {
    fn adapters(&self) -> Result<Vec<Adapter>> {
        match &self.fail {
            Some(e) => Err(DriverError::Os(e.clone())),
            None => Ok(self.adapters.clone()),
        }
    }
    fn os_build(&self) -> u32 {
        self.build
    }
    fn is_laptop(&self) -> bool {
        self.laptop
    }
}
