#![cfg(windows)]
use std::{path::Path, process::Command};

#[path = "../../../tests/support/async_orphan_fixture.rs"]
mod fixture;

fn scenario(name: &str) {
    let temp = tempfile::tempdir().unwrap();
    let deployment = name.starts_with("deployment-");
    let database = temp.path().join("store.sqlite");
    let backup = temp.path().join("before-execution.sqlite");
    let evidence = if deployment {
        drop(cdr_store::schema::open_initialized(&database).unwrap());
        std::fs::copy(&database, &backup).unwrap();
        let old = std::fs::read(&backup).unwrap();
        fixture::dispatching(&database, "deployment-fixture-resident");
        fixture::pending(&database, "held-later", "thread-b", 1);
        if name == "deployment-incompatible" {
            rusqlite::Connection::open(&database)
                .unwrap()
                .execute(
                    "UPDATE cdr_runtime_capability_requirements SET format_version=2
                 WHERE component='async_recovery_policy'",
                    [],
                )
                .unwrap();
        }
        let current = std::fs::read(&database).unwrap();
        assert_ne!(
            current, old,
            "backup must predate actual stored execution evidence"
        );
        Some((current, old))
    } else {
        None
    };
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/recovery_launcher_fixture.ps1");
    let mut command = Command::new("powershell.exe");
    command
        .env_remove("PSModulePath")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .arg("-Repository")
        .arg(&repo)
        .arg("-FixtureRoot")
        .arg(temp.path())
        .arg("-Scenario")
        .arg(name);
    if deployment {
        command.env("CDR_TEST_NATIVE_PROBE", env!("CARGO_BIN_EXE_cdr-runtime"));
    } else {
        command.env_remove("CDR_TEST_NATIVE_PROBE");
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "scenario={name}\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if let Some((current, old)) = evidence {
        assert_eq!(std::fs::read(database).unwrap(), current);
        assert_eq!(std::fs::read(backup).unwrap(), old);
    }
}

#[test]
fn armed_launcher_rejects_unsupported_candidate_before_runtime_start() {
    scenario("unsupported");
}
#[test]
fn armed_launcher_rejects_changed_environment_before_runtime_start() {
    scenario("changed-env");
}
#[test]
fn armed_launcher_rejects_missing_control_lease_before_runtime_start() {
    scenario("no-control");
}
#[test]
fn armed_launcher_rejects_partial_install_before_runtime_start() {
    scenario("partial");
}
#[test]
fn armed_launcher_starts_once_with_artifacts_pinned_through_process_creation() {
    scenario("valid");
}
#[test]
fn unarmed_legacy_launcher_preserves_existing_start_behavior() {
    scenario("legacy");
}
#[test]
fn armed_launcher_rejects_probe_database_mismatch_before_runtime_start() {
    scenario("wrong-db");
}
#[test]
fn armed_launcher_rejects_expired_deadline_without_process_start() {
    scenario("expired");
}

#[test]
fn normal_start_uses_actual_optional_journal_without_uninitialized_state() {
    scenario("normal-journal");
}
#[test]
fn legacy_start_uses_actual_optional_journal_without_uninitialized_state() {
    scenario("legacy-journal");
}
#[test]
fn existing_journal_reaches_child_and_rejects_a_duplicate_start() {
    scenario("owned-journal");
}
#[test]
fn unknown_launching_journal_preserves_evidence_without_start() {
    scenario("launching-journal");
}
#[test]
fn relative_environment_is_resolved_once_against_the_child_working_directory() {
    scenario("relative-env");
}
#[test]
fn a_parent_environment_probe_cannot_authorize_a_different_child_environment() {
    scenario("wrong-relative-env");
}

#[test]
fn supported_deployment_recovery_uses_real_probe_and_preserves_current_evidence() {
    scenario("deployment-compatible");
}
#[test]
fn supported_deployment_recovery_refuses_unsupported_current_store_without_rewind() {
    scenario("deployment-incompatible");
}
#[test]
fn supported_deployment_recovery_preserves_unknown_launch_without_probe_or_child() {
    scenario("deployment-unknown");
}

#[test]
fn supported_legacy_recovery_without_version_retains_actual_launch_compatibility_checks() {
    scenario("deployment-versionless");
}

#[test]
fn supported_recovery_rejects_newer_state_without_probe_launch_or_journal_rewrite() {
    scenario("deployment-newer");
}
