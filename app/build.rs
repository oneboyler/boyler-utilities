//! Compiles app.rc (the Pane icons + the manifest) with the Windows SDK's rc.exe and links the result.
use std::path::{Path, PathBuf};

fn find_rc() -> Option<PathBuf> {
    let base = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut vers: Vec<PathBuf> = std::fs::read_dir(base)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("x64").join("rc.exe").exists())
        .collect();
    vers.sort();
    vers.pop().map(|p| p.join("x64").join("rc.exe"))
}

fn main() {
    // Order 039 review: the exe's own imports (dwmapi, uxtheme, dxgi …) load from System32 only, never from the exe's folder
    // (a user-writable install folder: a DLL planted next to it must not run inside the app's elevated copy).
    // 0x800 = LOAD_LIBRARY_SEARCH_SYSTEM32; explicit LoadLibrary calls are not affected.
    println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=assets/pane_dark.ico");
    println!("cargo:rerun-if-changed=assets/pane_light.ico");
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let out = out_dir.join("app.res");
    let have_icons = dir.join("assets/pane_dark.ico").exists() && dir.join("assets/pane_light.ico").exists();
    let src = if have_icons {
        dir.join("app.rc")
    } else {
        // first build (before the icons exist): only the manifest
        let p = out_dir.join("min.rc");
        let m = dir.join("app.manifest").display().to_string().replace('\\', "/");
        std::fs::write(&p, format!("1 24 \"{}\"\n", m)).unwrap();
        p
    };
    let rc = find_rc().expect("rc.exe (Windows SDK) not found");
    let st = std::process::Command::new(rc).current_dir(&dir).args(["/nologo", "/fo"]).arg(&out).arg(&src).status().expect("rc.exe failed to start");
    assert!(st.success(), "rc.exe failed");
    println!("cargo:rustc-link-arg-bins={}", out.display());
}
