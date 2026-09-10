#![cfg(windows)]
use cdr_windows_native::resources::{active_processor_count, cpu_times, disk_bytes, memory_bytes};

#[test]
fn actual_read_only_host_probes_have_explicit_units_and_errors() {
    let root = tempfile::tempdir().unwrap();
    let disk = disk_bytes(root.path()).unwrap();
    assert!(disk.caller_total > 0);
    assert!(disk.caller_available <= disk.caller_total);
    assert!(disk.volume_free > 0);
    let error = disk_bytes(&root.path().join("does-not-exist")).unwrap_err();
    assert!(error.to_string().contains("GetDiskFreeSpaceExW"));
    let memory = memory_bytes().unwrap();
    assert!(memory.total > 0);
    assert!(memory.available <= memory.total);
    assert!(active_processor_count().unwrap() > 0);
    let before = cpu_times().unwrap();
    let after = cpu_times().unwrap();
    assert!(after.idle >= before.idle);
    assert!(after.kernel >= before.kernel);
    assert!(after.user >= before.user);
}
