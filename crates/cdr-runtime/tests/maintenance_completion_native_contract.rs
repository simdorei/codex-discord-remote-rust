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
fn active_only_first_and_later_failures_survive_without_event_audit() {
    run(
        "active_only_first_and_later_failures_survive_without_event_audit.ps1",
        &["0"],
    );
}

#[test]
fn active_copy_is_separate_and_validated_even_when_active_becomes_empty() {
    run(
        "active_copy_is_separate_and_validated_even_when_active_becomes_empty.ps1",
        &["0", "1", "2", "3"],
    );
}

#[test]
fn clean_active_still_validates_independent_audit_identity_and_shape() {
    run(
        "clean_active_still_validates_independent_audit_identity_and_shape.ps1",
        &["0", "1", "2"],
    );
}

#[test]
fn confirmed_failure_receipt_survives_stale_active_sending_state() {
    run(
        "confirmed_failure_receipt_survives_stale_active_sending_state.ps1",
        &["0"],
    );
}

#[test]
fn unknown_failure_result_survives_stale_active_sending_state() {
    run(
        "unknown_failure_result_survives_stale_active_sending_state.ps1",
        &["0"],
    );
}

#[test]
fn conflicting_terminal_failure_evidence_is_never_overwritten() {
    run(
        "conflicting_terminal_failure_evidence_is_never_overwritten.ps1",
        &["0", "1", "2"],
    );
}

#[test]
fn active_delete_and_resave_failure_still_preserve_independent_audit() {
    run(
        "active_delete_and_resave_failure_still_preserve_independent_audit.ps1",
        &["0", "1", "2", "3"],
    );
}

#[test]
fn corrupted_persisted_heartbeat_proof_is_refused() {
    run("corrupted_persisted_heartbeat_proof_is_refused.ps1", &["0"]);
}

#[test]
fn receipt_write_failure_preserves_owned_seal_then_resumes() {
    run(
        "receipt_write_failure_preserves_owned_seal_then_resumes.ps1",
        &["0"],
    );
}

#[test]
fn active_delete_failure_after_unseal_resumes_despite_halted_expiry() {
    run(
        "active_delete_failure_after_unseal_resumes_despite_halted_expiry.ps1",
        &["0"],
    );
}

#[test]
fn missing_seal_without_receipt_is_not_completion() {
    run("missing_seal_without_receipt_is_not_completion.ps1", &["0"]);
}

#[test]
fn new_stop_same_owner_is_never_removed() {
    run("new_stop_same_owner_is_never_removed.ps1", &["0"]);
}

#[test]
fn dead_stale_duplicate_or_wrong_artifact_refuses_cleanup() {
    run(
        "dead_stale_duplicate_or_wrong_artifact_refuses_cleanup.ps1",
        &["0", "1", "2", "3"],
    );
}

#[test]
fn timeout_preserved_without_second_post() {
    run("timeout_preserved_without_second_post.ps1", &["0"]);
}

#[test]
fn post_succeeded_but_result_write_failed_is_not_replayed() {
    run(
        "post_succeeded_but_result_write_failed_is_not_replayed.ps1",
        &["0"],
    );
}

#[test]
fn late_notice_never_overwrites_another_completion() {
    run(
        "late_notice_never_overwrites_another_completion.ps1",
        &["0"],
    );
}

#[test]
fn two_unwritable_archives_then_cleanup_preserves_both_errors_and_unknown() {
    run(
        "two_unwritable_archives_then_cleanup_preserves_both_errors_and_unknown.ps1",
        &["0"],
    );
}

#[test]
fn first_only_evidence_is_copied_and_remains_immutable() {
    run(
        "first_only_evidence_is_copied_and_remains_immutable.ps1",
        &["0"],
    );
}

#[test]
fn no_first_record_can_be_enriched_by_the_first_real_failure() {
    run(
        "no_first_record_can_be_enriched_by_the_first_real_failure.ps1",
        &["0"],
    );
}

#[test]
fn rejected_notice_does_not_leave_healthy_runtime_locked() {
    run(
        "rejected_notice_does_not_leave_healthy_runtime_locked.ps1",
        &["0"],
    );
}

#[test]
fn actual_notification_adapter_preserves_safe_http_diagnostics() {
    run(
        "actual_notification_adapter_preserves_safe_http_diagnostics.ps1",
        &["0"],
    );
}
