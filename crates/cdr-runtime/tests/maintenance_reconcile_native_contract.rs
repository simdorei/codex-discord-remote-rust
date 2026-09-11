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
fn existing_receipt_from_different_review_manifest_is_refused() {
    run(
        "existing_receipt_from_different_review_manifest_is_refused.ps1",
        &["0"],
    );
}

#[test]
fn preflight_is_readonly_then_expired_legacy_completes_without_replay() {
    run(
        "preflight_is_readonly_then_expired_legacy_completes_without_replay.ps1",
        &["0"],
    );
}

#[test]
fn partial_cleanup_preserves_original_hash_and_resumes() {
    run(
        "partial_cleanup_preserves_original_hash_and_resumes.ps1",
        &["0"],
    );
}

#[test]
fn changed_state_source_candidate_or_child_refuses_all_cleanup() {
    run(
        "changed_state_source_candidate_or_child_refuses_all_cleanup.ps1",
        &["0", "1", "2", "3", "4", "5", "6", "7"],
    );
}

#[test]
fn wrong_manifest_digest_is_rejected_before_loading_modules() {
    run(
        "wrong_manifest_digest_is_rejected_before_loading_modules.ps1",
        &["0"],
    );
}

#[test]
fn competing_control_owner_blocks_reconcile() {
    run("competing_control_owner_blocks_reconcile.ps1", &["0"]);
}

#[test]
fn update_success_notice_reports_binary_proof_without_claiming_room_cleanup() {
    run(
        "update_success_notice_reports_binary_proof_without_claiming_room_cleanup.ps1",
        &["0"],
    );
}

#[test]
fn post_mutation_baseline_hash_is_refused_by_actual_artifact_guard() {
    run(
        "post_mutation_baseline_hash_is_refused_by_actual_artifact_guard.ps1",
        &["0"],
    );
}

#[test]
fn modified_program_pin_is_rejected() {
    run("modified_program_pin_is_rejected.ps1", &["0"]);
}

#[test]
fn unknown_failure_notification_is_recorded_and_not_resent() {
    run(
        "unknown_failure_notification_is_recorded_and_not_resent.ps1",
        &["0"],
    );
}

#[test]
fn successful_native_probe_clears_only_its_command_record() {
    run(
        "successful_native_probe_clears_only_its_command_record.ps1",
        &["0"],
    );
}

#[test]
fn nonzero_exit_is_explicit_not_success_or_silent_fallback() {
    run(
        "nonzero_exit_is_explicit_not_success_or_silent_fallback.ps1",
        &["0"],
    );
}

#[test]
fn expired_wait_preserves_child_and_prohibits_command_replay() {
    run(
        "expired_wait_preserves_child_and_prohibits_command_replay.ps1",
        &["0"],
    );
}

#[test]
fn expired_deadline_prevents_even_native_child_creation() {
    run(
        "expired_deadline_prevents_even_native_child_creation.ps1",
        &["0"],
    );
}

#[test]
fn exit_zero_and_both_readers_ready_just_after_limit_preserves_unknown() {
    run(
        "exit_zero_and_both_readers_ready_just_after_limit_preserves_unknown.ps1",
        &["0"],
    );
}
