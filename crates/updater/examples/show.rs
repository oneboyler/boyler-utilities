//! Read-only look at the updater's state on this PC. Uses NO network: with no argument it asks `check()` with the repo setting
//! empty (the answer is "not set up yet", produced without any request) and counts the requests an in-memory server saw.
//!   updater-show                    -> the state, offline
//!   updater-show --check owner/name -> additionally does ONE real check against GitHub (only when you type it)
//! Nothing is downloaded, installed or changed either way.

use bu_updater::{CheckResult, FakeHttp, Updater, UpdaterConfig};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let exe: PathBuf = std::env::current_exe().expect("exe path");
    let version = env!("CARGO_PKG_VERSION");
    println!("bu-updater {version}");
    println!("app exe the updater would replace : {}", exe.display());
    for s in ["part", "new", "old", "result"] {
        let mut n = exe.clone().into_os_string();
        n.push(format!(".update-{s}"));
        let p = PathBuf::from(n);
        println!("  side file .update-{s:<6}: {}", if p.exists() { "present" } else { "absent" });
    }
    println!("system temp folder                : {}", std::env::temp_dir().display());

    // 1. repo setting empty -> "not set up", no request
    let cfg = UpdaterConfig::new("", version, exe.clone());
    let fake = std::sync::Arc::new(FakeHttp::new());
    let u = Updater::new(cfg, Box::new(fake.clone()), Box::new(bu_updater::FakeInstaller::default()));
    println!("check() with an empty repo setting : {:?}  (requests made: {})", u.check().expect("check"), fake.requests().len());

    // 2. optional real check
    if let Some(i) = args.iter().position(|a| a == "--check") {
        let repo = args.get(i + 1).cloned().unwrap_or_default();
        let u = Updater::real(UpdaterConfig::new(&repo, version, exe));
        println!("real check of {repo:?} (one request):");
        match u.check() {
            Ok(CheckResult::Available(r)) => println!("  newer version {} available, file {} ({} bytes, sha256 listed: {})", r.version, r.asset.name, r.asset.size, r.asset.sha256.is_some()),
            Ok(other) => println!("  {other:?}"),
            Err(e) => println!("  error: {e}"),
        }
    }
}
