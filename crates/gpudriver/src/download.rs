//! Download one driver into a folder with progress, and make sure it is NVIDIA's: Windows must trust its Authenticode
//! signature and the signer must be "NVIDIA Corporation" (measured: the 621.01 package is signed CN=NVIDIA Corporation,
//! issuer DigiCert Trusted G4 Code Signing RSA4096 SHA384 2021 CA1). A file that fails any check is deleted.

use crate::error::{io, DriverError, Result};
use crate::lookup::{is_nvidia_download, Release};
use bu_updater::{Http, Request};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The signer every file we run must carry.
pub const NVIDIA_SIGNER: &str = "NVIDIA Corporation";
/// Biggest driver package accepted (621.01 is 1.04 GB).
pub const MAX_PACKAGE: u64 = 4 * 1024 * 1024 * 1024;

/// Windows' signature check (real: WinVerifyTrust; fake: a list).
pub trait Signatures: Send + Sync {
    /// The signer's name when Windows trusts the file's Authenticode signature; `Err(NotSigned)` otherwise.
    fn signer(&self, file: &Path) -> Result<String>;
}

/// Ok when `file` is trusted by Windows AND signed by NVIDIA.
pub fn check_nvidia(sig: &dyn Signatures, file: &Path) -> Result<()> {
    let who = sig.signer(file)?;
    if who == NVIDIA_SIGNER {
        Ok(())
    } else {
        Err(DriverError::NotSigned(format!("signed by {who}")))
    }
}

/// Where a version's package goes in `dir`.
pub fn package_path(dir: &Path, release: &Release) -> PathBuf {
    dir.join(format!("{}.exe", release.version))
}

/// Download `release` into `dir` (made if missing) as `<version>.exe` and check it. `progress(bytes, total)` after every
/// piece. A package already there that passes the check is used as it is (nothing downloaded again).
pub fn download(http: &dyn Http, sig: &dyn Signatures, release: &Release, dir: &Path, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<PathBuf> {
    if !is_nvidia_download(&release.url) {
        return Err(DriverError::NotNvidiaLink(release.url.clone()));
    }
    let dest = package_path(dir, release);
    if dest.is_file() {
        if check_nvidia(sig, &dest).is_ok() {
            return Ok(dest);
        }
        std::fs::remove_file(&dest).map_err(|e| io("removing an old download", e))?;
    }
    std::fs::create_dir_all(dir).map_err(|e| io("making the download folder", e))?;
    let part = dir.join(format!("{}.exe.part", release.version));
    let r = fetch(http, &release.url, &part, progress).and_then(|_| {
        std::fs::rename(&part, &dest).map_err(|e| io("saving the download", e))?;
        check_nvidia(sig, &dest)
    });
    if r.is_err() {
        let _ = std::fs::remove_file(&part);
        let _ = std::fs::remove_file(&dest);
    }
    r.map(|_| dest)
}

/// GET into `part`; the file must be a Windows program of the announced size.
fn fetch(http: &dyn Http, url: &str, part: &Path, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<()> {
    let f = std::fs::File::create(part).map_err(|e| io("writing the download", e))?;
    let mut w = Capped { inner: std::io::BufWriter::with_capacity(1 << 20, f), written: 0, head: Vec::new() };
    let resp = http.get(&Request { url, accept: "*/*" }, &mut w, progress).map_err(|e| DriverError::Network(e.to_string()))?;
    if !(200..300).contains(&resp.status) {
        return Err(DriverError::HttpStatus(resp.status));
    }
    w.inner.flush().map_err(|e| io("writing the download", e))?;
    if let Some(n) = resp.content_length.filter(|&n| n != w.written) {
        return Err(DriverError::NotAPackage(format!("{} of {n} bytes arrived", w.written)));
    }
    if !w.head.starts_with(b"MZ") {
        return Err(DriverError::NotAPackage("not a Windows program".into()));
    }
    Ok(())
}

/// Counts what is written, keeps the first bytes, refuses more than [`MAX_PACKAGE`].
struct Capped<W: Write> {
    inner: W,
    written: u64,
    head: Vec<u8>,
}

impl<W: Write> Write for Capped<W> {
    fn write(&mut self, d: &[u8]) -> std::io::Result<usize> {
        if self.written + d.len() as u64 > MAX_PACKAGE {
            return Err(std::io::Error::other("the download is bigger than any driver package"));
        }
        if self.head.len() < 2 {
            self.head.extend(d.iter().take(2 - self.head.len()));
        }
        self.inner.write_all(d)?;
        self.written += d.len() as u64;
        Ok(d.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{scratch, FakeSignatures};
    use crate::version::DriverVersion;
    use bu_updater::{FakeHttp, FakeReply};

    const URL: &str = "https://us.download.nvidia.com/Windows/621.01/621.01-desktop-win10-win11-64bit-international-dch-whql.exe";

    fn release() -> Release {
        Release { version: DriverVersion::parse("621.01").unwrap(), date: "21 Oct 2026".into(), url: URL.into(), size: "1.04 GB".into() }
    }

    fn package() -> Vec<u8> {
        let mut b = b"MZ".to_vec();
        b.extend(std::iter::repeat_n(7u8, 100_000));
        b
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        v.sort();
        v
    }

    #[test]
    fn downloads_with_progress_and_keeps_an_nvidia_signed_package() {
        let dir = scratch("dl-ok");
        let h = FakeHttp::new().with_chunk(25_000);
        h.ok(URL, package());
        let mut seen = Vec::new();
        let p = download(&h, &FakeSignatures::nvidia(), &release(), &dir, &mut |d, t| seen.push((d, t))).unwrap();
        assert_eq!(p, dir.join("621.01.exe"));
        assert_eq!(std::fs::read(&p).unwrap(), package());
        assert_eq!(seen.len(), 5);
        assert_eq!(seen.last(), Some(&(100_002, Some(100_002))));
        assert_eq!(files(&dir), ["621.01.exe"]);
    }

    #[test]
    fn a_package_not_signed_by_nvidia_is_deleted() {
        let dir = scratch("dl-unsigned");
        let h = FakeHttp::new();
        h.ok(URL, package());
        let e = download(&h, &FakeSignatures::signed_by("Someone Else Ltd"), &release(), &dir, &mut |_, _| {}).unwrap_err();
        assert_eq!(e, DriverError::NotSigned("signed by Someone Else Ltd".into()));
        let e = download(&h, &FakeSignatures::untrusted(), &release(), &dir, &mut |_, _| {}).unwrap_err();
        assert!(matches!(e, DriverError::NotSigned(_)));
        assert!(files(&dir).is_empty());
    }

    #[test]
    fn a_cut_or_wrong_download_leaves_nothing_behind() {
        let dir = scratch("dl-cut");
        let sig = FakeSignatures::nvidia();
        let h = FakeHttp::new();
        h.route(URL, FakeReply::Cut { body: package(), sent: 5000 });
        assert!(matches!(download(&h, &sig, &release(), &dir, &mut |_, _| {}), Err(DriverError::Network(_))));
        h.route(URL, FakeReply::WrongLength { body: package(), announced: 200_000 });
        assert!(matches!(download(&h, &sig, &release(), &dir, &mut |_, _| {}), Err(DriverError::NotAPackage(_))));
        h.ok(URL, b"<html>sorry</html>".to_vec());
        assert!(matches!(download(&h, &sig, &release(), &dir, &mut |_, _| {}), Err(DriverError::NotAPackage(_))));
        h.status(URL, 404);
        assert_eq!(download(&h, &sig, &release(), &dir, &mut |_, _| {}), Err(DriverError::HttpStatus(404)));
        assert!(files(&dir).is_empty());
    }

    #[test]
    fn a_link_that_is_not_nvidias_is_never_fetched() {
        let dir = scratch("dl-link");
        let h = FakeHttp::new();
        let r = Release { url: "https://evil.example/Windows/621.01/x.exe".into(), ..release() };
        assert!(matches!(download(&h, &FakeSignatures::nvidia(), &r, &dir, &mut |_, _| {}), Err(DriverError::NotNvidiaLink(_))));
        assert!(h.requests().is_empty());
    }

    #[test]
    fn a_good_package_already_there_is_not_downloaded_again() {
        let dir = scratch("dl-again");
        std::fs::write(dir.join("621.01.exe"), package()).unwrap();
        let h = FakeHttp::new();
        assert_eq!(download(&h, &FakeSignatures::nvidia(), &release(), &dir, &mut |_, _| {}).unwrap(), dir.join("621.01.exe"));
        assert!(h.requests().is_empty());
    }
}
