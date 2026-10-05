#![cfg(windows)]
use std::{path::Path, process::Command};

#[path = "../../../tests/support/async_orphan_fixture.rs"]
mod fixture;

fn scenario(name: &str) {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("store.sqlite");
    drop(cdr_store::schema::open_initialized(&database).unwrap());
    fixture::dispatching(&database, "install-fixture-resident");
    fixture::pending(&database, "held-later", "thread-b", 1);
    let before = std::fs::read(&database).unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/recovery_install_fixture.ps1");
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .arg("-Repository")
        .arg(repo)
        .arg("-FixtureRoot")
        .arg(temp.path())
        .arg("-Candidate")
        .arg(env!("CARGO_BIN_EXE_cdr-runtime"))
        .arg("-Scenario")
        .arg(name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "scenario={name}\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read(database).unwrap(),
        before,
        "arming and actual compatibility/help children must not modify stored execution evidence"
    );
}

#[test]
fn reviewed_install_arms_and_reaches_one_real_native_child() {
    scenario("valid");
}
#[test]
fn identical_install_repeat_does_not_rewrite_or_launch() {
    scenario("repeat");
}
#[test]
fn matching_required_only_install_is_completed_without_clearing_intent() {
    scenario("required-only");
}
#[test]
fn matching_contract_only_install_is_completed_without_clearing_intent() {
    scenario("contract-only");
}
#[test]
fn foreign_install_intent_is_preserved_without_probe_or_launch() {
    scenario("foreign");
}
#[test]
fn install_requires_the_real_root_control_lease() {
    scenario("no-control");
}
#[test]
fn writable_but_nonexclusive_control_handle_is_not_a_lease() {
    scenario("shared-control");
}
#[test]
fn install_requires_its_owned_maintenance_seal() {
    scenario("foreign-seal");
}
#[test]
fn install_rejects_changed_reviewed_launcher_source() {
    scenario("changed-launcher");
}
#[test]
fn install_rejects_changed_environment_before_probe() {
    scenario("changed-env");
}
#[test]
fn install_rejects_an_expired_deadline_without_writes() {
    scenario("expired");
}
#[test]
fn interrupted_install_preserves_required_intent_and_blocks_native_start() {
    scenario("interrupted");
}
