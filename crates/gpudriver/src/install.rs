//! Install the DRIVER ONLY - run in the app's elevated copy (one admin prompt), never in the normal app.
//!
//! NVIDIA's package is a 7-Zip self-extractor: a small signed program with the 7z archive after it (621.01: archive at
//! byte 1 120 292, LZMA2 + BCJ2, 1295 entries). The Authenticode hash covers those bytes, so a package Windows trusts as
//! NVIDIA's carries NVIDIA's archive. Steps (the way TinyNvidiaUpdateChecker / NVCleanstall do "driver only"):
//! 1. copy the downloaded package into a new folder only SYSTEM + Administrators can write (the normal app can't swap a
//!    file in between) and check the COPY is signed by NVIDIA, for exactly the version asked for;
//! 2. read out only the driver: `Display.Driver\` (the driver + the Control Panel), `NVI2\` (the installer's engine),
//!    `setup.exe`, `setup.cfg`, `EULA.txt`, `ListDevices.txt` - not NvApp (NVIDIA app), NvTelemetry, NvContainer,
//!    ShadowPlay, FrameView, PhysX, HD Audio …;
//! 3. take the NVIDIA app's EULA / consent / privacy files out of setup.cfg's file list (they aren't there any more and
//!    setup.exe would refuse to start);
//! 4. check `setup.exe` is signed by NVIDIA, then run `setup.exe -s -noreboot -noeula` (silent, no restart, NVIDIA's
//!    settings kept - no clean install) and wait;
//! 5. delete the folder; the result is the driver Windows reports afterwards.

use crate::detect::{detect, GpuOs};
use crate::download::{check_nvidia, Signatures};
use crate::error::{io, DriverError, Result};
use crate::version::DriverVersion;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// The installer's switches: silent, no restart, no EULA page.
pub const SETUP_ARGS: [&str; 3] = ["-s", "-noreboot", "-noeula"];
/// The files at the package's top that are kept.
pub const DRIVER_FILES: [&str; 4] = ["setup.exe", "setup.cfg", "EULA.txt", "ListDevices.txt"];
/// The folders that are kept.
pub const DRIVER_FOLDERS: [&str; 2] = ["Display.Driver", "NVI2"];
/// setup.cfg's file-list lines naming the NVIDIA app's files (removed).
const MANIFEST_DROPS: [&str; 3] = ["${{EulaHtmlFile}}", "${{FunctionalConsentFile}}", "${{PrivacyPolicyFile}}"];
/// 7-Zip's archive signature.
const SEVEN_Z: [u8; 6] = [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];
/// Where in the package the archive may start (621.01: ~1.1 MB).
const SEARCH: usize = 32 * 1024 * 1024;

/// What the elevated copy needs from Windows (real: `real::RealInstallOs`; fake: records, runs nothing).
pub trait InstallOs: Send + Sync {
    /// A new, empty folder that only SYSTEM + Administrators can write.
    fn make_work_dir(&self) -> Result<PathBuf>;
    /// Run `program` with `args` in `cwd`, wait for it, return its exit code.
    fn run(&self, program: &Path, args: &[&str], cwd: &Path) -> Result<u32>;
}

/// How an install ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    /// Windows now reports the version asked for.
    Done,
    /// NVIDIA's installer said ok but Windows still reports another version: a restart finishes it.
    NeedsRestart,
}

/// Is `name` (a path inside the package, `/` or `\`) one of the driver-only parts, and safe to write under a folder?
pub fn is_driver_part(name: &str) -> bool {
    let n = name.replace('\\', "/");
    let parts: Vec<&str> = n.split('/').collect();
    if n.is_empty() || n.contains(':') || parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return false;
    }
    match parts[..] {
        [one] => DRIVER_FILES.iter().any(|f| f.eq_ignore_ascii_case(one)),
        [first, ..] => DRIVER_FOLDERS.iter().any(|f| f.eq_ignore_ascii_case(first)),
        [] => false,
    }
}

/// setup.cfg without the manifest lines of the NVIDIA app's files (every other byte unchanged).
pub fn patch_setup_cfg(cfg: &str) -> String {
    let mut out = String::with_capacity(cfg.len());
    for line in cfg.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with("<file ") && MANIFEST_DROPS.iter().any(|d| t.contains(&format!("name=\"{d}\""))) {
            continue;
        }
        out.push_str(line);
    }
    out
}

/// The `version="…"` of setup.cfg's `<setup>` line.
fn cfg_version(cfg: &str) -> Option<DriverVersion> {
    let s = &cfg[cfg.find("<setup ")?..];
    let s = &s[..s.find('>')?];
    let v = &s[s.find(" version=\"")? + 10..];
    DriverVersion::parse(&v[..v.find('"')?])
}

/// A reader that shows the archive inside the package as if it started at 0.
struct Offset<R> {
    inner: R,
    /// where the archive starts in the file, its length, the position inside it
    at: u64,
    len: u64,
    pos: u64,
}

impl<R: Read> Read for Offset<R> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        // never past the archive's end (the signed part of the file)
        let room = self.len.saturating_sub(self.pos).min(b.len() as u64) as usize;
        let n = self.inner.read(&mut b[..room])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl<R: Seek> Seek for Offset<R> {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        let to = match p {
            SeekFrom::Start(s) => Some(s),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
        };
        let to = to.filter(|&t| t <= self.len).ok_or_else(|| std::io::Error::other("seek outside the archive"))?;
        // (a seek to where it already is keeps the read buffer)
        if to != self.pos {
            self.inner.seek(SeekFrom::Start(self.at + to))?;
        }
        self.pos = to;
        Ok(to)
    }
}

fn u16_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?) as u64)
}
fn u32_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?) as u64)
}

/// Where in a signed program the data after its sections may sit: (end of the last section, start of the signature).
/// The signature (Authenticode's certificate table) must be the file's end, one certificate with no padding - bytes
/// hidden in the table or after it are not covered by the signature's hash (Windows checks that padding only with
/// EnableCertPaddingCheck, off by default), so an archive there would pass as NVIDIA's (end review).
pub fn signed_overlay(head: &[u8], file_len: u64) -> Option<(u64, u64)> {
    if !head.starts_with(b"MZ") {
        return None;
    }
    let pe = u32_at(head, 0x3C)? as usize;
    if head.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let coff = pe + 4;
    let sections = u16_at(head, coff + 2)? as usize;
    let opt = coff + 20;
    let opt_size = u16_at(head, coff + 16)? as usize;
    let dirs = match u16_at(head, opt)? {
        0x10B => opt + 96,
        0x20B => opt + 112,
        _ => return None,
    };
    // data directory 4 = the certificate table (a file offset)
    let (sec_at, sec_len) = (u32_at(head, dirs + 32)?, u32_at(head, dirs + 36)?);
    if sec_at == 0 || sec_len < 8 || sec_at + sec_len != file_len {
        return None;
    }
    let mut end = 0u64;
    let table = opt + opt_size;
    for s in 0..sections {
        let e = table + s * 40;
        end = end.max(u32_at(head, e + 20)? + u32_at(head, e + 16)?);
    }
    if end == 0 || end > sec_at {
        return None;
    }
    Some((end, sec_at))
}

/// The one WIN_CERTIFICATE's length must fill the table exactly (rounded to 8, as Windows writes it).
fn one_certificate(f: &mut std::fs::File, sec_at: u64, sec_len: u64) -> bool {
    let mut b = [0u8; 4];
    f.seek(SeekFrom::Start(sec_at)).is_ok() && f.read_exact(&mut b).is_ok() && (u32::from_le_bytes(b) as u64).div_ceil(8) * 8 == sec_len
}

fn open_archive(package: &Path) -> Result<sevenz_rust2::ArchiveReader<Offset<std::io::BufReader<std::fs::File>>>> {
    let mut f = std::fs::File::open(package).map_err(|e| io("opening the package", e))?;
    let len = f.metadata().map_err(|e| io("reading the package", e))?.len();
    let mut head = Vec::new();
    (&mut f).take(SEARCH as u64).read_to_end(&mut head).map_err(|e| io("reading the package", e))?;
    let Some((start, sig)) = signed_overlay(&head, len) else {
        return Err(DriverError::NotAPackage("not a signed program with its data where the signature covers it".into()));
    };
    if !one_certificate(&mut f, sig, len - sig) {
        return Err(DriverError::NotAPackage("the signature block holds more than the signature".into()));
    }
    let window = &head[(start as usize).min(head.len())..(sig as usize).min(head.len())];
    for at in window.windows(SEVEN_Z.len()).enumerate().filter(|(_, w)| *w == SEVEN_Z).map(|(i, _)| start + i as u64) {
        // the whole archive (start header + packed data + its header) must end before the signature
        let i = (at - start) as usize;
        let (Some(next_at), Some(next_len)) = (window.get(i + 12..i + 20), window.get(i + 20..i + 28)) else { continue };
        let span = 32u64.checked_add(u64::from_le_bytes(next_at.try_into().unwrap_or_default())).and_then(|x| x.checked_add(u64::from_le_bytes(next_len.try_into().unwrap_or_default())));
        let Some(span) = span.filter(|&s| at + s <= sig) else { continue };
        let file = f.try_clone().map_err(|e| io("reading the package", e))?;
        let mut r = Offset { inner: std::io::BufReader::with_capacity(1 << 20, file), at, len: span, pos: 0 };
        if r.inner.seek(SeekFrom::Start(at)).is_err() {
            continue;
        }
        if let Ok(a) = sevenz_rust2::ArchiveReader::new(r, sevenz_rust2::Password::empty()) {
            return Ok(a);
        }
    }
    Err(DriverError::NotAPackage("no driver archive inside".into()))
}

/// Read the driver-only parts of `package` into `dest`. Returns how many files were written.
pub fn extract_driver(package: &Path, dest: &Path) -> Result<usize> {
    let mut a = open_archive(package)?;
    let mut n = 0usize;
    let mut failed: Option<DriverError> = None;
    let r = a.for_each_entries(|e, rd| {
        if e.is_directory() || !is_driver_part(e.name()) {
            std::io::copy(rd, &mut std::io::sink())?;
            return Ok(true);
        }
        let p = dest.join(e.name().replace('\\', "/"));
        let mut write = || -> std::io::Result<()> {
            std::fs::create_dir_all(p.parent().unwrap_or(dest))?;
            let mut o = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&p)?);
            std::io::copy(rd, &mut o)?;
            std::io::Write::flush(&mut o)
        };
        if let Err(err) = write() {
            failed = Some(io("unpacking the driver", err));
            return Ok(false);
        }
        n += 1;
        Ok(true)
    });
    if let Some(e) = failed {
        return Err(e);
    }
    r.map_err(|e| DriverError::NotAPackage(format!("unreadable archive: {e}")))?;
    Ok(n)
}

/// Removes the work folder however the install ends.
struct WorkDir(PathBuf);

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Install `version` from the downloaded `package` (driver only). Runs in the elevated copy. `gpu` re-reads the
/// installed driver afterwards.
pub fn install(package: &Path, version: DriverVersion, os: &dyn InstallOs, sig: &dyn Signatures, gpu: &dyn GpuOs) -> Result<Installed> {
    // one open file: its size checked, then copied from that same handle
    let mut src = std::fs::File::open(package).map_err(|e| io("opening the package", e))?;
    let size = src.metadata().map_err(|e| io("reading the package", e))?.len();
    if size > crate::download::MAX_PACKAGE {
        return Err(DriverError::NotAPackage(format!("{size} bytes - bigger than any driver package")));
    }
    let work = WorkDir(os.make_work_dir()?);
    let copy = work.0.join("package.exe");
    let mut dst = std::fs::File::create(&copy).map_err(|e| io("copying the package", e))?;
    std::io::copy(&mut (&mut src).take(crate::download::MAX_PACKAGE + 1), &mut dst).map_err(|e| io("copying the package", e))?;
    drop((src, dst));
    check_nvidia(sig, &copy)?;
    let driver = work.0.join("driver");
    extract_driver(&copy, &driver)?;
    let _ = std::fs::remove_file(&copy);
    let cfg_path = driver.join("setup.cfg");
    let cfg = std::fs::read_to_string(&cfg_path).map_err(|_| DriverError::NotAPackage("no setup.cfg".into()))?;
    match cfg_version(&cfg) {
        Some(v) if v == version => {}
        other => {
            let got = other.map(|v| v.to_string()).unwrap_or_else(|| "no version".into());
            return Err(DriverError::NotAPackage(format!("the package is {got}, not {version}")));
        }
    }
    std::fs::write(&cfg_path, patch_setup_cfg(&cfg)).map_err(|e| io("writing setup.cfg", e))?;
    let setup = driver.join("setup.exe");
    check_nvidia(sig, &setup)?;
    // and the installer's own engine beside it (end review: belt and braces on top of the archive check)
    for e in std::fs::read_dir(driver.join("NVI2")).map_err(|_| DriverError::NotAPackage("no NVI2 folder".into()))?.flatten() {
        let p = e.path();
        let ext = p.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if p.is_file() && (ext == "dll" || ext == "exe") {
            check_nvidia(sig, &p)?;
        }
    }
    let code = os.run(&setup, &SETUP_ARGS, &driver)?;
    drop(work);
    let now = detect(gpu).ok().flatten().and_then(|g| g.installed);
    match (now == Some(version), code) {
        (true, _) => Ok(Installed::Done),
        (false, 0) => Ok(Installed::NeedsRestart),
        (false, c) => Err(DriverError::SetupFailed(c)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::Adapter;
    use crate::fake::{scratch, FakeGpuOs, FakeInstallOs, FakeSignatures};

    const CFG: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n<setup title=\"${{ProductTitle}}\" version=\"621.01\" installerMin=\"2.1002.158.1139\">\r\n\t<manifest>\r\n\t\t<file name=\"setup.exe\" core=\"true\"/>\r\n\t\t<file name=\"setup.cfg\"/>\r\n\t\t<file name=\"eula.txt\"/>\r\n\t\t<file name=\"${{EulaHtmlFile}}\"/>\r\n\t\t<file name=\"${{FunctionalConsentFile}}\"/>\r\n\t\t<file name=\"${{PrivacyPolicyFile}}\"/>\r\n\t\t<file name=\"${{InstallerDllDirectory}}/NVI2.dll\" core=\"true\"/>\r\n\t</manifest>\r\n</setup>\r\n";

    /// A stand-in package: a signed program with a real 7z archive of driver and non-driver parts after its sections
    /// (like NVIDIA's).
    fn package(dir: &Path, cfg: &str) -> PathBuf {
        package_hidden(dir, cfg, crate::fake::Hide::Overlay)
    }

    fn package_hidden(dir: &Path, cfg: &str, hide: crate::fake::Hide) -> PathBuf {
        let mut sz = sevenz_rust2::ArchiveWriter::new(std::io::Cursor::new(Vec::new())).unwrap();
        let files: Vec<(&str, Vec<u8>)> = vec![
            ("setup.exe", b"MZ setup".to_vec()),
            ("setup.cfg", cfg.as_bytes().to_vec()),
            ("EULA.txt", b"eula".to_vec()),
            ("ListDevices.txt", b"devices".to_vec()),
            ("Display.Driver/nv_disp.inf", b"inf".to_vec()),
            ("Display.Driver/NVCPL/cpl.appx", b"control panel".to_vec()),
            ("NVI2/NVI2.dll", b"MZ nvi2".to_vec()),
            ("NvApp/NVIDIA app.exe", b"MZ app".to_vec()),
            ("NvTelemetry/telemetry.dll", b"MZ tel".to_vec()),
            ("PhysX/physx.msi", b"physx".to_vec()),
            ("HDAudio/hda.inf", b"hda".to_vec()),
            ("Overlay.exe", b"MZ overlay".to_vec()),
        ];
        for (name, data) in files {
            sz.push_archive_entry(sevenz_rust2::ArchiveEntry::new_file(name), Some(std::io::Cursor::new(data))).unwrap();
        }
        let archive = sz.finish().unwrap().into_inner();
        let p = dir.join("621.01.exe");
        std::fs::write(&p, crate::fake::signed_pe(&archive, hide)).unwrap();
        p
    }

    #[test]
    fn an_archive_where_the_signature_does_not_cover_it_is_refused() {
        use crate::fake::Hide;
        let sig = FakeSignatures::nvidia();
        for hide in [Hide::CertPadding, Hide::InCertificate, Hide::AfterEnd] {
            let pkg = package_hidden(&scratch("in-hide-dl"), CFG, hide);
            let os = FakeInstallOs::new(scratch("in-hide-work"));
            let e = install(&pkg, v("621.01"), &os, &sig, &gpu_after("32.0.16.2101")).unwrap_err();
            assert!(matches!(e, DriverError::NotAPackage(_)), "{hide:?}: {e:?}");
            assert!(os.runs().is_empty(), "{hide:?}");
            assert!(!os.work.exists());
        }
        // a program with no signature table, or not a program
        let dir = scratch("in-hide-plain");
        let mut plain = crate::fake::signed_pe(b"", Hide::Overlay);
        plain[0xC8 + 32..0xC8 + 40].fill(0);
        let p = dir.join("621.01.exe");
        std::fs::write(&p, plain).unwrap();
        assert!(matches!(extract_driver(&p, &dir.join("x")), Err(DriverError::NotAPackage(_))));
    }

    fn gpu_after(ver: &str) -> FakeGpuOs {
        FakeGpuOs::new(vec![Adapter { name: "NVIDIA GeForce RTX 4090".into(), hardware_id: r"PCI\VEN_10DE&DEV_2684".into(), driver_version: ver.into() }])
    }

    fn v(s: &str) -> DriverVersion {
        DriverVersion::parse(s).unwrap()
    }

    #[test]
    fn installs_the_driver_only_with_nvidias_signed_setup() {
        let dl = scratch("in-ok-dl");
        let pkg = package(&dl, CFG);
        let os = FakeInstallOs::new(scratch("in-ok-work"));
        let sig = FakeSignatures::nvidia();
        assert_eq!(install(&pkg, v("621.01"), &os, &sig, &gpu_after("32.0.16.2101")), Ok(Installed::Done));
        let runs = os.runs();
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert_eq!(run.args, ["-s", "-noreboot", "-noeula"]);
        assert_eq!(run.program.file_name().unwrap(), "setup.exe");
        // what setup.exe saw: the driver parts only, and setup.cfg without the NVIDIA app's three files
        assert_eq!(
            run.files,
            ["Display.Driver/NVCPL/cpl.appx", "Display.Driver/nv_disp.inf", "EULA.txt", "ListDevices.txt", "NVI2/NVI2.dll", "setup.cfg", "setup.exe"]
        );
        assert!(!run.cfg.contains("EulaHtmlFile") && !run.cfg.contains("FunctionalConsentFile") && !run.cfg.contains("PrivacyPolicyFile"));
        assert!(run.cfg.contains("<file name=\"eula.txt\"/>") && run.cfg.contains("NVI2.dll"));
        assert_eq!(
            run.cfg,
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n<setup title=\"${{ProductTitle}}\" version=\"621.01\" installerMin=\"2.1002.158.1139\">\r\n\t<manifest>\r\n\t\t<file name=\"setup.exe\" core=\"true\"/>\r\n\t\t<file name=\"setup.cfg\"/>\r\n\t\t<file name=\"eula.txt\"/>\r\n\t\t<file name=\"${{InstallerDllDirectory}}/NVI2.dll\" core=\"true\"/>\r\n\t</manifest>\r\n</setup>\r\n"
        );
        // the package copy was checked first, setup.exe right before it ran
        assert_eq!(sig.checked_names(), ["package.exe", "setup.exe", "NVI2.dll"]);
        // the work folder is gone; the download itself is the app's to remove
        assert!(!os.work.exists());
        assert!(pkg.exists());
    }

    #[test]
    fn nothing_runs_when_a_signature_is_wrong() {
        let pkg = package(&scratch("in-sig-dl"), CFG);
        for sig in [
            FakeSignatures::signed_by("Someone Else Ltd"),
            FakeSignatures::untrusted(),
            FakeSignatures::nvidia().with("setup.exe", Some("Someone Else Ltd")),
            FakeSignatures::nvidia().with("setup.exe", None),
            FakeSignatures::nvidia().with("NVI2.dll", Some("Someone Else Ltd")),
        ] {
            let os = FakeInstallOs::new(scratch("in-sig-work"));
            assert!(matches!(install(&pkg, v("621.01"), &os, &sig, &gpu_after("32.0.16.1088")), Err(DriverError::NotSigned(_))));
            assert!(os.runs().is_empty());
            assert!(!os.work.exists());
        }
    }

    #[test]
    fn a_package_of_another_version_is_refused() {
        let pkg = package(&scratch("in-ver-dl"), CFG);
        let os = FakeInstallOs::new(scratch("in-ver-work"));
        let e = install(&pkg, v("617.42"), &os, &FakeSignatures::nvidia(), &gpu_after("32.0.16.1088")).unwrap_err();
        assert_eq!(e, DriverError::NotAPackage("the package is 621.01, not 617.42".into()));
        assert!(os.runs().is_empty());
    }

    #[test]
    fn a_file_without_an_archive_is_refused() {
        let dl = scratch("in-noarc");
        let p = dl.join("621.01.exe");
        std::fs::write(&p, b"MZ just a program").unwrap();
        let os = FakeInstallOs::new(scratch("in-noarc-work"));
        assert!(matches!(install(&p, v("621.01"), &os, &FakeSignatures::nvidia(), &gpu_after("")), Err(DriverError::NotAPackage(_))));
        assert!(os.runs().is_empty() && !os.work.exists());
    }

    #[test]
    fn the_result_is_what_windows_reports_afterwards() {
        let pkg = package(&scratch("in-res-dl"), CFG);
        let sig = FakeSignatures::nvidia();
        let os = FakeInstallOs::new(scratch("in-res-w1")).exit_code(1);
        assert_eq!(install(&pkg, v("621.01"), &os, &sig, &gpu_after("32.0.16.2101")), Ok(Installed::Done));
        let os = FakeInstallOs::new(scratch("in-res-w2"));
        assert_eq!(install(&pkg, v("621.01"), &os, &sig, &gpu_after("32.0.16.1088")), Ok(Installed::NeedsRestart));
        let os = FakeInstallOs::new(scratch("in-res-w3")).exit_code(0x8000_0001);
        assert_eq!(install(&pkg, v("621.01"), &os, &sig, &gpu_after("32.0.16.1088")), Err(DriverError::SetupFailed(0x8000_0001)));
    }

    #[test]
    fn only_driver_parts_with_safe_names_are_written() {
        for ok in ["setup.exe", "setup.cfg", "eula.txt", "ListDevices.txt", "Display.Driver/nvlddmkm.sys", r"NVI2\NVI2.dll", "Display.Driver/NVCPL/x.appx"] {
            assert!(is_driver_part(ok), "{ok}");
        }
        for bad in [
            "NvApp/app.exe",
            "NvTelemetry/x.dll",
            "PhysX/x.msi",
            "Overlay.exe",
            "Display.Driver/../evil.exe",
            r"Display.Driver\..\..\evil.exe",
            "/setup.exe",
            "C:/setup.exe",
            "Display.Driver//x",
            "",
            "Display.Driver",
        ] {
            assert!(!is_driver_part(bad), "{bad}");
        }
    }
}
