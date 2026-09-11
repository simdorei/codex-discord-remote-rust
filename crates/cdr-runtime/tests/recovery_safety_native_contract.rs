#![cfg(windows)]
#[path = "support/maintenance_native.rs"]
mod fixture;
use std::path::Path;
fn run(name: &str) {
    let root = tempfile::tempdir().unwrap();
    fixture::run_fixture(
        root.path(),
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/recovery_safety"),
        name,
        "",
    );
}

#[test]
fn exit_then_start_once_and_repeat_verifies_receipt() {
    run("test_exit_then_start_once_and_repeat_verifies_receipt.ps1");
}

#[test]
fn timeout_preserves_markers_and_never_runs_general_health_recovery() {
    run("test_timeout_preserves_markers_and_never_runs_general_health_recovery.ps1");
}

#[test]
fn foreign_live_instance_is_not_accepted_or_changed() {
    run("test_foreign_live_instance_is_not_accepted_or_changed.ps1");
}

#[test]
fn foreign_marker_is_preserved() {
    run("test_foreign_marker_is_preserved.ps1");
}

#[test]
fn create_only_publication_does_not_overwrite() {
    run("test_create_only_publication_does_not_overwrite.ps1");
}

#[test]
fn control_lock_blocks_other_owner_and_releases() {
    run("test_control_lock_blocks_other_owner_and_releases.ps1");
}

#[test]
fn wrong_restart_receipt_never_claims_completion() {
    run("test_wrong_restart_receipt_never_claims_completion.ps1");
}

#[test]
fn completion_requires_real_heartbeat_not_bootstrap() {
    run("test_completion_requires_real_heartbeat_not_bootstrap.ps1");
}

#[test]
fn new_stop_intent_prevents_false_restart_completion() {
    run("test_new_stop_intent_prevents_false_restart_completion.ps1");
}

#[test]
fn failed_start_reseals_maintenance_without_stopping_unknown_process() {
    run("test_failed_start_reseals_maintenance_without_stopping_unknown_process.ps1");
}

#[test]
fn v2_state_cannot_publish_stop_before_ack() {
    run("test_v2_state_cannot_publish_stop_before_ack.ps1");
}

#[test]
fn v2_owner_blocks_legacy_even_before_disabled() {
    run("test_v2_owner_blocks_legacy_even_before_disabled.ps1");
}
