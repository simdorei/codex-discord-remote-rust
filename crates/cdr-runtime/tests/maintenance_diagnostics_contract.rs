#![cfg(windows)]
#[path = "support/maintenance_native.rs"]
mod fixture;
use std::path::Path;

#[test]
fn native_preflight_reason_is_persisted_without_secrets_or_stop() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/maintenance");
    for variant in ["reason", "secrets", "bounded", "unicode"] {
        let root = tempfile::tempdir().unwrap();
        fixture::run_fixture(
            root.path(),
            &directory,
            "native_failure_diagnostic.ps1",
            variant,
        );
    }
}

#[test]
fn readiness_command_budget_includes_startup_wait_and_close() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/maintenance");
    let root = tempfile::tempdir().unwrap();
    fixture::run_fixture(root.path(), &directory, "readiness_command_budget.ps1", "");
}
