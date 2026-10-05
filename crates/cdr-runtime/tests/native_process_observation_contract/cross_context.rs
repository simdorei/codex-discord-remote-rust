#[path = "cross_context_bootstrap.rs"]
mod bootstrap;

fn run(mode: &str) -> serde_json::Value {
    let built = bootstrap::Built::new();
    let output = built.execute(mode, false);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "cross-context JSON {error}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    eprintln!("NATIVE_CROSS_CONTEXT {value}");
    assert!(
        output.status.success(),
        "cross-context {mode}: {:?}; stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(value["diagnostic_only"], true);
    assert_eq!(value["native_gate_pass"], false);
    value
}

#[test]
fn four_subscription_and_failure_boundaries_are_closed() {
    let value = run("contracts");
    let actual: Vec<_> = value["contracts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            assert_eq!(case["pass"], true, "{case}");
            case["name"].as_str().unwrap()
        })
        .collect();
    assert_eq!(
        actual,
        [
            "late_a_setup_blocks_pre_control",
            "four_ready_then_control",
            "each_missing_subscription",
            "each_late_subscription",
            "each_faulted_subscription",
            "deadline_999",
            "deadline_1000",
            "deadline_1001",
            "same_tick_is_not_preceding",
            "negative_window_rejected",
            "only_empty_poll_is_setup",
            "authorization_consumed_once",
            "invalid_control_does_not_authorize",
            "start_and_stop_and_exit_required",
            "late_control_does_not_reopen",
            "late_fault_invalidates_without_replay",
            "post_requires_ready_and_exact_exit",
            "post_stays_inside_window",
            "exact_envelope",
            "stale_nonce_rejected",
            "wrong_identity_rejected",
            "wrong_phase_or_expiry_rejected",
            "exact_row_matches",
            "late_row_rejected",
            "different_pid_rejected",
            "old_process_row_rejected",
            "partial_start",
            "join_throw",
            "join_false",
            "cleanup_continues_after_stdin_and_kill_faults",
            "publication_write",
            "publication_close",
            "publication_both",
            "publication_is_no_clobber",
            "all_zero_is_inconclusive",
            "separate_context_outcomes",
            "dispatch_999_once",
            "dispatch_1000_denied",
            "dispatch_1001_denied",
            "dispatch_post_5000_denied",
            "dispatch_permission_check_delay",
            "dispatch_fresh_fault_denied",
            "final_matrix_uses_full_a_window",
            "final_matrix_keeps_b_early_gate",
            "final_matrix_rejects_a_after_window",
            "final_output_retains_write_and_close",
            "cleanup_output_retains_write_and_close",
            "finalization_A_fresh",
            "finalization_A_prior",
            "finalization_B_fresh",
            "finalization_B_prior",
        ]
    );
}

#[test]
fn cross_lineage_capture_never_grants_native_readiness() {
    let value = run("capture");
    assert_eq!(value["safe_for_follow_up"], true, "{value}");
    assert_eq!(value["a"]["budget_ms"], 5000);
    assert_eq!(value["a"]["probe_cap"], 3);
    assert!(value["status"] == "COMPARISON_RECORDED" || value["status"] == "INCONCLUSIVE");
    if value["comparison_valid"] == true {
        assert_eq!(value["b_exit_confirmed"], true);
        assert_eq!(value["outcomes"]["start"]["b_sees_b_both"], true);
        assert_eq!(value["outcomes"]["stop"]["b_sees_b_both"], true);
    } else {
        assert!(value["outcomes"].as_object().unwrap().is_empty());
    }
}

#[test]
fn bootstrap_deadline_precedes_session_initialization() {
    let built = bootstrap::Built::new();
    let start = std::time::Instant::now();
    let output = built.execute("bootstrap-stall", false);
    assert_eq!(
        output.status.code(),
        Some(3),
        "independent first-Main guard must end the stalled process"
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(10));
    assert!(
        output.stdout.is_empty(),
        "no initialized session or success receipt"
    );
    let expired = built.execute("contracts", true);
    assert_eq!(expired.status.code(), Some(2));
    assert!(
        expired.stdout.is_empty(),
        "expired bootstrap must not initialize"
    );
}

#[test]
fn compiler_stall_does_not_escape_supervision() {
    bootstrap::assert_stalled_compiler_is_reaped();
}
