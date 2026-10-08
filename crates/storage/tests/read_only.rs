//! `RealOs::read_only()` (what `storage-show` runs on) refuses every delete. Only a missing path inside this lane's
//! scratch folder is used, so even a broken guard could delete nothing outside it; emptying the bin is not called here (a
//! broken guard would empty the real one).

use bu_storage::{RealOs, StorageOs};
use std::io::ErrorKind;
use std::path::Path;

#[test]
fn read_only_layer_refuses_deletes() {
    let os = RealOs::read_only();
    let missing = Path::new(r"C:\BoylerUtilities-scratch\lane-e\read-only-test-never-created\file.tmp");
    assert!(!missing.exists());
    assert_eq!(os.remove_file(missing).unwrap_err().kind(), ErrorKind::PermissionDenied, "refused before Windows is asked");
    assert_eq!(os.remove_dir(missing.parent().unwrap()).unwrap_err().kind(), ErrorKind::PermissionDenied);
    // The app's layer reaches Windows (which answers "not found" for the same path).
    assert_eq!(RealOs::new().remove_file(missing).unwrap_err().kind(), ErrorKind::NotFound);
    // Reads still work.
    assert!(!os.drives().unwrap().is_empty());
}
