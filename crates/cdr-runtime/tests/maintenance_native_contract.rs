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
fn normal_order_and_durable_mutation_then_single_launch() {
    run(
        "normal_order_and_durable_mutation_then_single_launch.ps1",
        &[""],
    );
}

#[test]
fn each_prelaunch_interruption_resumes_at_recorded_phase() {
    run(
        "each_prelaunch_interruption_resumes_at_recorded_phase.ps1",
        &[
            "armed",
            "preflight",
            "backup",
            "package",
            "install",
            "cleanup",
            "full_readiness",
        ],
    );
}

#[test]
fn unknown_notification_does_not_replay_launch_cleanup_or_post() {
    run(
        "unknown_notification_does_not_replay_launch_cleanup_or_post.ps1",
        &[""],
    );
}

#[test]
fn budget_is_persistent_across_reloaded_attempts() {
    run("budget_is_persistent_across_reloaded_attempts.ps1", &[""]);
}

#[test]
fn expired_operation_does_not_run_any_action() {
    run("expired_operation_does_not_run_any_action.ps1", &[""]);
}

#[test]
fn changed_state_owner_is_preserved() {
    run("changed_state_owner_is_preserved.ps1", &[""]);
}

#[test]
fn actual_catalog_does_not_accept_uncertified_installed_hash() {
    run(
        "actual_catalog_does_not_accept_uncertified_installed_hash.ps1",
        &[""],
    );
}

#[test]
fn old_scheduler_operation_cannot_consume_new_state_budget() {
    run(
        "old_scheduler_operation_cannot_consume_new_state_budget.ps1",
        &[""],
    );
}

#[test]
fn recorded_child_exited_is_never_reset_or_relaunched() {
    run(
        "recorded_child_exited_is_never_reset_or_relaunched.ps1",
        &[""],
    );
}

#[test]
fn unrecorded_launch_outcome_stays_unknown() {
    run("unrecorded_launch_outcome_stays_unknown.ps1", &[""]);
}

#[test]
fn actual_launch_once_then_adopts_only_recorded_child() {
    run(
        "actual_launch_once_then_adopts_only_recorded_child.ps1",
        &[""],
    );
}

#[test]
fn foreign_marker_prevents_real_launch_boundary() {
    run("foreign_marker_prevents_real_launch_boundary.ps1", &[""]);
}

#[test]
fn old_process_liveness_blocks_launch() {
    run("old_process_liveness_blocks_launch.ps1", &[""]);
}

#[test]
fn stop_requires_real_matching_ack() {
    run("stop_requires_real_matching_ack.ps1", &[""]);
}

#[test]
fn same_heartbeat_is_not_two_observations() {
    run("same_heartbeat_is_not_two_observations.ps1", &[""]);
}

#[test]
fn two_increasing_heartbeats_are_saved() {
    run("two_increasing_heartbeats_are_saved.ps1", &[""]);
}

#[test]
fn saved_success_does_not_override_stale_live_heartbeat() {
    run(
        "saved_success_does_not_override_stale_live_heartbeat.ps1",
        &[""],
    );
}

#[test]
fn new_stop_intent_cannot_be_consumed_by_completion() {
    run(
        "new_stop_intent_cannot_be_consumed_by_completion.ps1",
        &[""],
    );
}

#[test]
fn readiness_returning_after_deadline_never_calls_launch() {
    run(
        "readiness_returning_after_deadline_never_calls_launch.ps1",
        &[""],
    );
}

#[test]
fn korean_failure_evidence_uses_utf8_not_machine_default() {
    run(
        "korean_failure_evidence_uses_utf8_not_machine_default.ps1",
        &[""],
    );
}

#[test]
fn notice_file_creation_failure_does_not_retain_ownership() {
    run(
        "notice_file_creation_failure_does_not_retain_ownership.ps1",
        &[""],
    );
}

#[test]
fn notice_diagnostic_write_failure_warns_without_relocking_runtime() {
    run(
        "notice_diagnostic_write_failure_warns_without_relocking_runtime.ps1",
        &[""],
    );
}

#[test]
fn legacy_completion_also_ignores_notice_only_file_failure() {
    run(
        "legacy_completion_also_ignores_notice_only_file_failure.ps1",
        &[""],
    );
}

#[test]
fn locked_notice_journal_cannot_hold_healthy_runtime_completion() {
    run(
        "locked_notice_journal_cannot_hold_healthy_runtime_completion.ps1",
        &[""],
    );
}

#[test]
fn cleanup_failure_and_unknown_failure_post_survive_engine_reentry() {
    run(
        "cleanup_failure_and_unknown_failure_post_survive_engine_reentry.ps1",
        &[""],
    );
}

#[test]
fn failure_audit_write_guard_preserves_active_evidence_until_retry() {
    run(
        "failure_audit_write_guard_preserves_active_evidence_until_retry.ps1",
        &[""],
    );
}

#[test]
fn first_failure_is_immutable_when_latest_failure_changes() {
    run(
        "first_failure_is_immutable_when_latest_failure_changes.ps1",
        &[""],
    );
}
