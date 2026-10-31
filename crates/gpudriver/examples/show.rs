//! Read-only: the NVIDIA card + driver Windows reports, and NVIDIA's own driver list for it.
//! `--check <package.exe>`: Windows' signature check of a downloaded package (never runs it).
//! `--extract <package.exe> <empty folder>`: read the driver-only parts out of a package (never runs anything).

#[cfg(windows)]
fn main() {
    use bu_gpudriver::real::{RealGpuOs, RealSignatures};
    use bu_gpudriver::{GpuOs, Signatures};
    let a: Vec<String> = std::env::args().collect();
    if a.len() == 3 && a[1] == "--check" {
        let p = std::path::Path::new(&a[2]);
        let t = std::time::Instant::now();
        println!("signer   : {:?}", RealSignatures.signer(p));
        println!("NVIDIA's : {:?} ({} ms)", bu_gpudriver::check_nvidia(&RealSignatures, p), t.elapsed().as_millis());
        return;
    }
    if a.len() == 4 && a[1] == "--extract" {
        let t = std::time::Instant::now();
        println!("files    : {:?} ({} s)", bu_gpudriver::install::extract_driver(std::path::Path::new(&a[2]), std::path::Path::new(&a[3])), t.elapsed().as_secs());
        return;
    }
    let os = RealGpuOs;
    for ad in os.adapters().unwrap_or_default() {
        println!("adapter  : {} | {} | {}", ad.name, ad.hardware_id, ad.driver_version);
    }
    println!("build    : {} (NVIDIA os id {})  laptop: {}", os.os_build(), bu_gpudriver::lookup::os_id(os.os_build()), os.is_laptop());
    let gpu = match bu_gpudriver::detect(&os) {
        Ok(Some(g)) => g,
        other => {
            println!("no NVIDIA card: {other:?}");
            return;
        }
    };
    println!("NVIDIA   : {}  installed {}", gpu.name, gpu.installed.map(|v| v.to_string()).unwrap_or_else(|| "unknown".into()));
    let t = std::time::Instant::now();
    match bu_gpudriver::lookup(&bu_updater::WinHttp::new("BoylerUtilities"), &os, &gpu.name) {
        Ok(list) => {
            println!("lookup   : {} drivers in {} ms", list.len(), t.elapsed().as_millis());
            for r in list.iter().take(8) {
                println!("  {}  {}  {}  {}", r.version, r.date, r.size, r.url);
            }
        }
        Err(e) => println!("lookup   : {e}"),
    }
}

#[cfg(not(windows))]
fn main() {}
