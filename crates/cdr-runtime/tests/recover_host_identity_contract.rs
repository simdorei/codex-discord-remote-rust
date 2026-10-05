#![cfg(windows)]

use std::{path::Path, process::Command};

fn check(case: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.join("scripts/Test-CdrToolsRecoveryIdentity.ps1"))
        .args(["-Case", case])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "case={case}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("recover_identity_case_passed={case}")),
    );
}

#[test]
fn recovery_preparation_identity_includes_the_exact_writer() {
    check("writer-plan");
}

#[test]
fn changed_writer_is_refused_before_full_recovery_host_effects() {
    check("writer-dispatch");
}

#[test]
fn same_writer_pid_with_new_creation_time_is_refused_before_host_effects() {
    check("writer-pid-reuse");
}

#[test]
fn unchanged_exact_hosts_and_writer_still_restart_without_claiming_tool_probe() {
    check("unchanged");
}
