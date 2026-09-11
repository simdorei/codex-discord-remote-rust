#![cfg(windows)]
#[path = "support/maintenance_native.rs"]
mod fixture;
use std::path::Path;
fn run(file: &str, variants: &[&str]) {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/maintenance");
    for variant in variants {
        let root = tempfile::tempdir().unwrap();
        fixture::run_fixture(root.path(), &directory, file, variant);
    }
}

#[test]
fn receipt_is_bound_and_reloaded_from_disk() {
    run("receipt_is_bound_and_reloaded_from_disk.ps1", &["0"]);
}

#[test]
fn changed_deleted_foreign_or_wrong_source_receipt_rejected() {
    run(
        "changed_deleted_foreign_or_wrong_source_receipt_rejected.ps1",
        &["0", "1", "2", "3"],
    );
}

#[test]
fn native_snapshot_or_package_failure_never_certifies_receipt() {
    run(
        "native_snapshot_or_package_failure_never_certifies_receipt.ps1",
        &["0", "1"],
    );
}

#[test]
fn post_stop_cannot_reuse_pre_stop_snapshot() {
    run("post_stop_cannot_reuse_pre_stop_snapshot.ps1", &["0"]);
}

#[test]
fn opt_in_result_is_returned_only_after_definite_success() {
    run(
        "opt_in_result_is_returned_only_after_definite_success.ps1",
        &["0"],
    );
}

#[test]
fn nonzero_exit_never_returns_a_backup_result() {
    run("nonzero_exit_never_returns_a_backup_result.ps1", &["0"]);
}

#[test]
fn halted_reentry_preserves_original_error_and_observation() {
    run(
        "halted_reentry_preserves_original_error_and_observation.ps1",
        &["0"],
    );
}

#[test]
fn only_matching_sealed_ack_is_reported_as_matching() {
    run(
        "only_matching_sealed_ack_is_reported_as_matching.ps1",
        &["0", "1", "2", "3"],
    );
}

#[test]
fn stop_observation_requires_exact_owner_and_reports_read_failure() {
    run(
        "stop_observation_requires_exact_owner_and_reports_read_failure.ps1",
        &["0", "1", "2", "3"],
    );
}

#[test]
fn live_policy_does_not_require_or_fabricate_old_certificate() {
    run(
        "live_policy_does_not_require_or_fabricate_old_certificate.ps1",
        &["0"],
    );
}

#[test]
fn backup_failure_never_publishes_drain_or_stop() {
    run("backup_failure_never_publishes_drain_or_stop.ps1", &["0"]);
}

#[test]
fn resuming_prepared_rechecks_backup_before_drain() {
    run("resuming_prepared_rechecks_backup_before_drain.ps1", &["0"]);
}

#[test]
fn legacy_or_unknown_policy_is_not_silently_adopted() {
    run(
        "legacy_or_unknown_policy_is_not_silently_adopted.ps1",
        &["0", "1"],
    );
}

#[test]
fn shutdown_failure_cannot_resume_after_late_success() {
    run(
        "shutdown_failure_cannot_resume_after_late_success.ps1",
        &["0", "1", "2"],
    );
}

#[test]
fn same_user_aliases_pass_but_other_user_is_rejected() {
    run(
        "same_user_aliases_pass_but_other_user_is_rejected.ps1",
        &["0", "1", "2", "3"],
    );
}

#[test]
fn failure_observation_separates_liveness_and_redacts_private_details() {
    run(
        "failure_observation_separates_liveness.ps1",
        &["alive", "exited", "unknown"],
    );
}
