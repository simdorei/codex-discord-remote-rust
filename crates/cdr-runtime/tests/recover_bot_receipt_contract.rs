#![cfg(windows)]

fn run_case(case: &str) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.join("scripts/Test-CdrToolsRecoveryIdentity.ps1"))
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
            .contains(&format!("recover_identity_case_passed={case}"))
    );
}

#[test]
fn missing_replacement_identity_cannot_be_reported_as_restarted() {
    run_case("receipt-missing");
}

#[test]
fn empty_replacement_identity_cannot_be_reported_as_restarted() {
    run_case("receipt-empty");
}

#[test]
fn malformed_replacement_identity_cannot_be_reported_as_restarted() {
    run_case("receipt-malformed");
}

#[test]
fn null_replacement_identity_cannot_be_reported_as_restarted() {
    run_case("receipt-null");
}

#[test]
fn array_replacement_identity_cannot_be_reported_as_restarted() {
    run_case("receipt-array");
}

#[test]
fn unchanged_bot_identity_remains_rejected_and_desktop_restored() {
    run_case("receipt-same");
}

#[test]
fn original_identity_with_trailing_lf_cannot_masquerade_as_a_replacement() {
    run_case("receipt-same-lf");
}
