//! `RealOs::read_only()` (what `perf-show` runs on) refuses End and priority changes. Only a PID that can't exist is
//! used, so even a broken guard could end nothing; open_file_location is not called here (a broken guard would open
//! Explorer on the owner's screen).

use bu_perf::{EndHow, PerfError, PerfOs, Priority, RealOs};

const NO_SUCH_PID: u32 = 0xFFFF_FFF0;

#[test]
fn read_only_layer_refuses_end_and_priority() {
    let os = RealOs::read_only();
    for how in [EndHow::Close, EndHow::Terminate] {
        assert!(matches!(os.end_process(NO_SUCH_PID, how), Err(PerfError::Refused(_))));
    }
    assert!(matches!(os.set_priority(NO_SUCH_PID, Priority::Low), Err(PerfError::Refused(_))));
    // Reads still work.
    assert!(!os.processes().unwrap().is_empty());
}
