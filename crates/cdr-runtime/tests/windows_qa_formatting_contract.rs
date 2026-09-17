#![cfg(windows)]

#[path = "support/powershell_utf8.rs"]
mod powershell;

use std::path::Path;

fn run(variant: &str) {
    let root = tempfile::tempdir().unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = powershell::command(include_str!("fixtures/qa/formatting.ps1"))
        .env("CDR_TEST_ROOT", root.path())
        .env(
            "CDR_FORMAT_SCRIPT",
            repo.join("scripts/Test-RustFormatting.ps1"),
        )
        .env("CDR_FORMAT_VARIANT", variant)
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{variant}: {}\n{}",
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap()
    );
}

#[test]
fn all_target_roots_and_editions_survive_bounded_batches() {
    run("complete");
}

#[test]
fn native_formatter_failure_is_not_reported_as_success() {
    run("format-failure");
}

#[test]
fn incomplete_workspace_metadata_is_rejected() {
    run("metadata-incomplete");
}
