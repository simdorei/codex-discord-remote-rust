#![cfg(windows)]

fn run_case(case: &str) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.join("scripts/Test-CdrRecoveryControllerLifecycle.ps1"))
        .args(["-Case", case])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("recovery_controller_case_passed:{case}"))
    );
}

#[test]
fn duplicate_worker_keeps_exact_receipt_and_dispatches_only_once() {
    run_case("duplicate");
}

#[test]
fn unconfirmed_attempt_is_not_replayed_or_relabelled() {
    run_case("unconfirmed");
}

#[test]
fn expired_worker_request_cannot_claim_or_dispatch() {
    run_case("expired");
}

#[test]
fn changed_request_bytes_cannot_claim_or_dispatch() {
    run_case("request-change");
}

#[test]
fn changed_program_pin_cannot_claim_or_dispatch() {
    run_case("program-change");
}

#[test]
fn busy_host_gate_cannot_replay_after_it_is_released() {
    run_case("gate-busy");
}

#[test]
fn partial_dispatch_failure_is_durable_and_cannot_dispatch_twice() {
    run_case("dispatch-failure");
}

#[test]
fn receipt_rename_failure_before_effect_preserves_evidence_and_refuses_replay() {
    run_case("receipt-prewrite");
}

#[test]
fn receipt_rename_failure_after_partial_effect_preserves_evidence_and_refuses_replay() {
    run_case("receipt-partial");
}
