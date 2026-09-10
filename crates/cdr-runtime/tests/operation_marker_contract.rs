use std::fs;

use cdr_runtime::operation_marker::{restart_requested, shutdown_requested};

#[test]
fn stop_and_restart_markers_request_graceful_shutdown_without_being_consumed() {
    let temp = tempfile::tempdir().unwrap();
    assert!(!shutdown_requested(temp.path()));

    let restart = temp.path().join(".codex_discord_rust.restart");
    fs::write(&restart, b"restart").unwrap();
    assert!(shutdown_requested(temp.path()));
    assert!(restart_requested(temp.path()));
    assert!(restart.is_file());
    fs::remove_file(&restart).unwrap();

    let stop = temp.path().join(".codex_discord_rust.stop");
    fs::write(&stop, b"stop").unwrap();
    assert!(shutdown_requested(temp.path()));
    assert!(!restart_requested(temp.path()));
    assert!(stop.is_file());

    fs::write(&restart, b"restart").unwrap();
    assert!(shutdown_requested(temp.path()));
    assert!(!restart_requested(temp.path()), "an explicit stop must win");
}
