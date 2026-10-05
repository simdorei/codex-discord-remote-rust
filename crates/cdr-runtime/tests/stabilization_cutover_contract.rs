#![cfg(windows)]
use std::{os::windows::process::CommandExt, path::Path, process::Command};

fn scenario(name: &str) {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("discord_mirror.sqlite");
    drop(cdr_store::schema::open_initialized(&database).unwrap());
    let before = std::fs::read(&database).unwrap();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script =
        repository.join("crates/cdr-runtime/tests/support/stabilization_cutover_fixture.ps1");
    let output = Command::new("powershell.exe")
        .creation_flags(0x0800_0000)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .arg("-Repository")
        .arg(&repository)
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
        "cutover fixture must not mutate or release stored execution evidence"
    );
}

#[test]
fn engine_arms_compatibility_before_any_candidate_writer() {
    scenario("engine-order");
}
#[test]
fn failed_arming_cannot_reach_readiness_cleanup_or_start() {
    scenario("engine-failure");
}
#[test]
fn real_arming_readiness_and_one_help_child_keep_database_unchanged() {
    scenario("valid");
}
#[test]
fn idempotent_arming_keeps_both_original_intent_files() {
    scenario("repeat");
}
#[test]
fn missing_arming_refuses_the_actual_readiness_dispatch() {
    scenario("missing");
}
#[test]
fn concurrent_writer_refuses_arming_before_any_intent() {
    scenario("writer");
}
#[test]
fn foreign_arming_intent_is_never_cleared_or_adopted() {
    scenario("foreign");
}
#[test]
fn changed_environment_refuses_the_candidate_writer() {
    scenario("changed-env");
}
#[test]
fn foreign_maintenance_seal_cannot_arm_the_guard() {
    scenario("foreign-seal");
}
#[test]
fn legacy_operator_modes_remain_unchanged() {
    scenario("legacy");
}
#[test]
fn actual_watchdog_and_state_inventory_include_cutover_guards() {
    scenario("wiring");
}

#[test]
fn held_preflight_preserves_normal_absolute_paths() {
    scenario("preflight-normal");
}
#[test]
fn held_preflight_accepts_database_verbatim_path() {
    scenario("preflight-database-verbatim");
}
#[test]
fn held_preflight_accepts_environment_verbatim_path() {
    scenario("preflight-environment-verbatim");
}
#[test]
fn held_preflight_accepts_both_native_verbatim_paths() {
    scenario("preflight-both-verbatim");
}
#[test]
fn held_preflight_rejects_wrong_or_malformed_paths() {
    scenario("preflight-rejected-paths");
}
#[test]
fn held_preflight_preserves_negative_authority_contract() {
    scenario("preflight-rejected-authority");
}
#[test]
fn held_preflight_rejection_prevents_engine_effects_and_preserves_hold() {
    scenario("preflight-engine-rejection");
}
#[test]
fn held_preflight_accepts_unmodified_native_readonly_json() {
    scenario("preflight-native");
}
