use std::{os::windows::process::CommandExt, path::Path, process::Command};

fn run_route_matrix(mode: &str) -> serde_json::Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.join("crates/cdr-runtime/tests/fixtures/native_process_route_matrix.ps1"))
        .args(["-Mode", mode])
        .env("CDR_ROUTE_PARENT_PID", std::process::id().to_string())
        .creation_flags(0x0800_0000)
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "route JSON: {error}; stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    eprintln!("NATIVE_ROUTE_MATRIX {value}");
    assert!(
        output.status.success(),
        "route fixture failed: {value}; {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(value["diagnostic_only"], true);
    assert_eq!(value["native_gate_pass"], false);
    value
}

#[test]
fn route_matrix_gates_do_not_backfill_or_retry() {
    let value = run_route_matrix("contracts");
    let rows = value["contracts"].as_array().unwrap();
    let expected = [
        "matching_control_authorizes_once",
        "absent",
        "queue-only",
        "mismatch",
        "stale",
        "late",
        "future",
        "wrong-kind",
        "valid_lifetime",
        "invalid_exit_0",
        "invalid_exit_199",
        "invalid_exit_200",
        "invalid_exit_401",
        "callback_error",
        "cleanup_error",
        "primary_error",
        "unconfirmed_exit",
        "forced_cleanup_is_not_observation",
        "token_boolean_false",
        "token_boolean_true",
        "token_dword_false",
        "token_dword_true",
        "token_dword_nonboolean",
        "token_dword_high_bit",
        "token_null_rejected",
        "token_empty_rejected",
        "token_two_bytes_rejected",
        "token_three_bytes_rejected",
        "token_five_bytes_rejected",
        "token_boolean_two_rejected",
        "token_boolean_max_rejected",
    ];
    assert_eq!(rows.len(), expected.len());
    for (row, name) in rows.iter().zip(expected) {
        assert_eq!(row["name"], name);
        assert_eq!(row["pass"], true, "{row}");
    }
}

#[test]
fn kernel_diagnostic_revision26_preserves_evidence_and_cleanup_authority() {
    let value = run_semisync("kernel-regressions");
    let rows = value["contracts"].as_array().unwrap();
    let expected = [
        "same_schema_variable_lengths",
        "lifecycle_after_variable_lengths",
        "lifecycle_identity_qpc_preserved",
        "valid_ingress_capture",
        "genuine_schema_overflow",
        "existing_schema_survives_overflow",
        "schema_overflow_capture_inconclusive",
        "decoder_error_recorded",
        "decoder_errors_bounded",
        "decoder_failure_cleanup_separate",
        "ingress_row_bound",
        "evidence_normal",
        "evidence_empty_stdout",
        "evidence_malformed_json",
        "evidence_stderr_preserves_rows",
        "evidence_safe_false",
        "evidence_safe_null",
        "evidence_safe_string_false",
        "evidence_safe_string_true",
        "evidence_safe_missing",
        "evidence_wrong_helper_identity",
        "evidence_cleanup_missing",
        "evidence_cleanup_nonempty",
        "evidence_threads_unjoined",
        "evidence_probe_forced",
        "evidence_probe_unconfirmed",
        "evidence_window_missing",
        "evidence_frequency_zero",
        "evidence_helper_not_started",
        "evidence_comparison_failed_clean",
        "evidence_tap_unsafe",
        "queue_cleanup_not_absence_proof",
    ];
    assert_eq!(rows.len(), expected.len());
    for (row, name) in rows.iter().zip(expected) {
        assert_eq!(row["name"], name);
        assert_eq!(row["pass"], true, "{row}");
    }
}

#[test]
fn route_matrix_context_is_query_only_and_sid_free() {
    let value = run_route_matrix("context");
    let process = &value["observer_process"];
    assert_eq!(process["query_only"], true);
    assert_eq!(process["status"], "ok", "{value}");
    assert_eq!(process["user_equals_observer_process"], true);
    assert_eq!(process["logon_equals_observer_process"], true);
    assert!(process["session_id"].is_number());
    assert!(process["integrity_rid"].is_number());
    assert!(process["has_restrictions"].is_boolean());
    assert!(matches!(
        process["has_restrictions_returned_bytes"].as_u64(),
        Some(1 | 4)
    ));
    assert!(!process["privileges"].as_array().unwrap().is_empty());
    assert_eq!(value["observer_thread"]["query_only"], true);
    assert!(!value.to_string().contains("S-1-"));
}

#[test]
fn native_route_matrix_diagnostic_preserves_inconclusive_results() {
    let value = run_route_matrix("capture");
    assert_eq!(value["acquisition_budget_ms"], 5000);
    assert_eq!(value["positive_control_deadline_ms"], 1000);
    assert_eq!(value["safe_for_follow_up"], true, "{value}");
    assert!(value["cleanup_errors"].as_array().unwrap().is_empty());
    assert!(value["callback_errors"].as_array().unwrap().is_empty());
    assert!(value["queue_rows"].is_array());
    assert!(value["direct_rows"].is_array());
    if value["start_attempted"] == false {
        assert_eq!(value["status"], "INCONCLUSIVE");
        assert_eq!(value["comparison_valid"], false);
        assert_eq!(value["probe"]["started"], false);
        assert_eq!(value["probe"]["process_id"], 0);
    } else {
        assert_eq!(value["probe"]["started"], true);
        assert_eq!(value["probe"]["exit_confirmed"], true);
        assert!(value["positive_control"].is_object());
    }
}

fn run_semisync(mode: &str) -> serde_json::Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.join("crates/cdr-runtime/tests/fixtures/native_process_semisync.ps1"))
        .args(["-Mode", mode])
        .creation_flags(0x0800_0000)
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "semisync JSON: {error}; stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    eprintln!("NATIVE_SEMISYNC_MATRIX {value}");
    assert!(
        output.status.success(),
        "semisync diagnostic failed: {value}; {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(value["diagnostic_only"], true);
    assert_eq!(value["native_gate_pass"], false);
    value
}

#[test]
fn semisync_enumeration_only_retries_an_empty_poll() {
    let value = run_semisync("contracts");
    let rows = value["contracts"].as_array().unwrap();
    let expected = [
        "only_poll_timeout_is_retriable",
        "access_denied_is_not_timeout",
        "transport_failure_is_not_timeout",
        "success_is_not_timeout",
        "partial_start_cleanup",
        "join_throw_cleanup",
        "join_false_cleanup",
        "outer_sync_dispose_failure",
        "outer_both_dispose_failures",
        "ready_before_deadline",
        "ready_at_deadline",
        "ready_after_deadline",
        "not_ready_before_deadline",
        "negative_clock_rejected",
        "authorization_is_one_shot",
    ];
    assert_eq!(rows.len(), expected.len());
    for (row, name) in rows.iter().zip(expected) {
        assert_eq!(row["name"], name);
        assert_eq!(row["pass"], true, "{row}");
    }
}

#[test]
fn native_semisync_diagnostic_compares_async_and_blocking_routes() {
    let value = run_semisync("capture");
    assert_eq!(value["acquisition_budget_ms"], 5000);
    assert_eq!(value["safe_for_follow_up"], true, "{value}");
    assert_eq!(value["comparison_valid"], true, "{value}");
    assert_eq!(value["probes"].as_array().unwrap().len(), 2);
    for route in ["queue_rows", "direct_rows", "semisync_rows"] {
        assert!(value[route].is_array());
    }
}

#[test]
fn semisync_late_arrivals_do_not_backfill_the_five_second_window() {
    let value = run_semisync("late-arrival");
    assert_eq!(value["acquisition_budget_ms"], 5000);
    assert_eq!(value["late_observation_budget_ms"], 15000);
    assert_eq!(value["safe_for_follow_up"], true, "{value}");
    assert_eq!(value["comparison_valid"], true, "{value}");
    assert_eq!(value["probes"].as_array().unwrap().len(), 2);
    for route in ["queue_rows", "direct_rows", "semisync_rows"] {
        for row in value[route].as_array().unwrap() {
            assert!(row["ReceivedMs"].as_u64().unwrap() < 5000, "{row}");
        }
        for row in value[format!("late_{route}")].as_array().unwrap() {
            assert!(row["ReceivedMs"].as_u64().unwrap() >= 5000, "{row}");
        }
    }
    for probe in value["probes"].as_array().unwrap() {
        assert!(probe["ready_ms"].as_u64().unwrap() < 5000);
        assert!(probe["release_ms"].as_u64().unwrap() < 5000);
        assert_eq!(probe["exit_confirmed"], true);
        assert_eq!(probe["termination_requested"], false);
    }
}

#[test]
fn kernel_diagnostic_ownership_and_time_windows_are_fail_closed() {
    let value = run_semisync("kernel-contracts");
    let rows = value["contracts"].as_array().unwrap();
    assert_eq!(rows.len(), 23);
    for row in rows {
        assert_eq!(row["pass"], true, "{row}");
    }
}
