#![cfg(windows)]

#[path = "support/windows_release_checkpoint.rs"]
mod support;

use std::fs;

use support::{
    MARKER_BYTES, create_fixture, run_archive_verifier_adversarial_probe,
    run_post_verification_package_mutation_probe,
};

fn assert_post_verification_mutation_rejected(mutation: &str, expected: &str) {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));

    let (output, archive) = run_post_verification_package_mutation_probe(&fixture, mutation);

    assert!(
        !output.status.success(),
        "post-verification {mutation} mutation was accepted: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(expected),
        "{mutation} rejection did not contain {expected:?}: {stderr}"
    );
    assert!(!archive.exists(), "rejected mutation created an archive");
    assert_eq!(fs::read(&fixture.marker).unwrap(), MARKER_BYTES);
}

#[test]
fn staged_binding_rejects_rollback_source_mutated_after_verification() {
    assert_post_verification_mutation_rejected(
        "rollback",
        "Rollback source codex_discord_helper.py SHA-256 mismatch after staging",
    );
}

#[test]
fn staged_binding_rejects_artifact_mutated_after_verification() {
    assert_post_verification_mutation_rejected(
        "artifact",
        "cdr-mcp-server.exe evidence-bound payload SHA-256 mismatch after staging",
    );
}

#[test]
fn staged_binding_rejects_evidence_bytes_mutated_after_verification() {
    assert_post_verification_mutation_rejected(
        "evidence",
        "Workspace gate evidence SHA-256 mismatch after staging",
    );
}

#[test]
fn staged_binding_rejects_database_mutated_after_verification() {
    assert_post_verification_mutation_rejected(
        "database",
        "Rollback DB SHA-256 mismatch after staging",
    );
}

#[test]
fn staged_binding_rejects_workspace_file_mutated_after_verification() {
    assert_post_verification_mutation_rejected(
        "workspace",
        "Workspace payload Cargo.toml SHA-256 mismatch after staging",
    );
}

#[test]
fn archive_verifier_rejects_coherent_semantic_and_directory_tampering() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));

    let output = run_archive_verifier_adversarial_probe(&fixture);

    assert!(
        output.status.success(),
        "archive adversarial probe failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("passed=11"));
    assert_eq!(fs::read(&fixture.marker).unwrap(), MARKER_BYTES);
}
