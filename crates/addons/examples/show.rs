//! `cargo run -p bu-addons --example addons-show` - the real state, read-only: Raw Accel's filter + driver, the pinned
//! release, and (optional path argument) whether a Raw Accel folder's uninstaller is an official one and its driver signed.

fn main() {
    #[cfg(windows)]
    {
        use bu_addons::os::AddonOs;
        use bu_addons::rawaccel::{self, PIN};
        let os = bu_addons::real::RealOs::new();
        println!("pinned release: Raw Accel {} ({} bytes = {}) {}", PIN.version, PIN.size, rawaccel::mb(PIN.size), PIN.url);
        println!("mouse UpperFilters: {:?}", bu_addons::real::mouse_upper_filters());
        println!("filter set: {}  driver running: {}", os.rawaccel_filter_set(), os.rawaccel_running());
        println!("state: {:?}", rawaccel::state(&os));
        let sys = std::path::Path::new(r"C:\Windows\System32\drivers\rawaccel.sys");
        if sys.exists() {
            println!("installed driver signature: {:?}", os.verify_signature(sys));
        }
        if let Some(dir) = std::env::args().nth(1) {
            let dir = std::path::PathBuf::from(dir);
            println!("{}: official uninstaller = {}", dir.display(), rawaccel::official_uninstaller(&dir));
            let drv = dir.join("driver").join("rawaccel.sys");
            if drv.exists() {
                println!("its driver signature: {:?}", os.verify_signature(&drv));
            }
            for f in ["installer.exe", "uninstaller.exe"] {
                if dir.join(f).exists() {
                    println!("{f} signature: {:?}", os.verify_signature(&dir.join(f)));
                }
            }
        }
    }
}
