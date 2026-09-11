#![cfg(windows)]
use std::{
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn run_case(file: &str, name: &str) {
    let root = tempfile::tempdir().unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cutover");
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(fixtures.join("run.ps1"))
        .arg("-CaseFile")
        .arg(fixtures.join(file))
        .arg("-CaseName")
        .arg(name)
        .env("PUBLISH_ROOT", root.path())
        .env("PUBLISH_SOURCE", repo)
        .current_dir(root.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("PowerShell cutover contract timed out: {name}");
        }
        thread::sleep(Duration::from_millis(25));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{name}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cutover_does_not_delete_foreign_restart() {
    run_case(
        "maintenance_publication.ps1",
        "test_cutover_does_not_delete_foreign_restart",
    );
}

#[test]
fn cutover_does_not_delete_foreign_stop() {
    run_case(
        "maintenance_publication.ps1",
        "test_cutover_does_not_delete_foreign_stop",
    );
}

#[test]
fn cutover_marker_publication_respects_control_owner() {
    run_case(
        "maintenance_publication.ps1",
        "test_cutover_marker_publication_respects_control_owner",
    );
}

#[test]
fn worker_rechecks_runtime_lock_before_stop_publication() {
    run_case(
        "maintenance_publication.ps1",
        "test_worker_rechecks_runtime_lock_before_stop_publication",
    );
}

#[test]
fn cutover_stop_consumes_its_marker_after_confirmed_exit() {
    run_case(
        "maintenance_publication.ps1",
        "test_cutover_stop_consumes_its_marker_after_confirmed_exit",
    );
}

#[test]
fn finalization_refuses_stop_published_after_observation() {
    run_case(
        "cutover_finalization.ps1",
        "test_finalization_refuses_stop_published_after_observation",
    );
}

#[test]
fn finalization_rejects_changed_identity() {
    run_case(
        "cutover_finalization.ps1",
        "test_finalization_rejects_changed_identity",
    );
}

#[test]
fn completion_and_interrupted_completion_use_persisted_identity() {
    run_case(
        "cutover_finalization.ps1",
        "test_completion_and_interrupted_completion_use_persisted_identity",
    );
}

#[test]
fn interrupted_completion_refuses_restart() {
    run_case(
        "cutover_finalization.ps1",
        "test_interrupted_completion_refuses_restart",
    );
}

#[test]
fn competing_restart_after_observation_survives_outer_catch() {
    run_case(
        "cutover_outer_failure.ps1",
        "test_competing_restart_after_observation_survives_outer_catch",
    );
}

#[test]
fn recovered_source_completion_reports_actual_runtime() {
    run_case(
        "cutover_outer_failure.ps1",
        "test_recovered_source_completion_reports_actual_runtime",
    );
}

#[test]
fn primary_error_survives_seal_control_lock_failure() {
    run_case(
        "cutover_outer_failure.ps1",
        "test_primary_error_survives_seal_control_lock_failure",
    );
}
