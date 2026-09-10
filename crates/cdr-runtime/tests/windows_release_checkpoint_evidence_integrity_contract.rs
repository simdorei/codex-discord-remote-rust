#![cfg(windows)]
#![allow(dead_code)]

#[path = "support/windows_release_checkpoint.rs"]
mod support;

use std::fs;

use serde_json::{Value, json};
use support::{
    create_fixture, run_checkpoint, run_evidence_bundle_in_shell, run_evidence_contract_in_shell,
};

fn read_json(path: &std::path::Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn write_json(path: &std::path::Path, value: &Value) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

fn assert_rejected(output: &std::process::Output, expected: &str) {
    assert!(
        !output.status.success(),
        "unsafe checkpoint evidence was accepted: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(expected),
        "stderr did not contain {expected:?}: {stderr}"
    );
}

fn assert_contract_rejected_in_both_shells(path: &std::path::Path, kind: &str) {
    for shell in ["powershell.exe", "pwsh.exe"] {
        let output = run_evidence_contract_in_shell(path, kind, shell);
        assert_rejected(
            &output,
            &format!(
                "{} evidence contract",
                if kind == "soak" {
                    "Offline soak"
                } else {
                    "Workspace gate"
                }
            ),
        );
    }
}

fn wrap_json_in_singleton_array(path: &std::path::Path) {
    let original = fs::read(path).unwrap();
    let mut wrapped = Vec::with_capacity(original.len() + 2);
    wrapped.push(b'[');
    wrapped.extend_from_slice(&original);
    wrapped.push(b']');
    fs::write(path, wrapped).unwrap();
}

#[test]
fn evidence_integrity_01_rejects_mcp_changed_after_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    fs::copy(
        &fixture.runtime,
        fixture.root.join("target/release/cdr-mcp-server.exe"),
    )
    .unwrap();

    let output = run_checkpoint(&fixture, None, None, None);

    assert_rejected(
        &output,
        "cdr-mcp-server.exe evidence-bound payload SHA-256 mismatch after staging",
    );
}

#[test]
fn evidence_integrity_02_rejects_label_only_workspace_gate() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let current = read_json(&fixture.workspace_evidence);
    write_json(
        &fixture.workspace_evidence,
        &json!({
            "kind": "windows_full_workspace_gate",
            "status": "passed",
            "source_fingerprint": current["source_fingerprint"],
            "safety": { "bot_disabled": true }
        }),
    );

    let output = run_checkpoint(&fixture, None, None, None);

    assert_rejected(&output, "Workspace gate evidence contract");
}

#[test]
fn evidence_integrity_03_rejects_label_only_short_soak() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let current = read_json(&fixture.soak_evidence);
    write_json(
        &fixture.soak_evidence,
        &json!({
            "kind": "windows_offline_fake_replay_soak_smoke",
            "status": "passed_short_smoke_only",
            "artifacts": current["artifacts"],
            "provenance": {
                "same_run_canonical_release_build": true,
                "source_fingerprint": current["provenance"]["source_fingerprint"]
            },
            "safety": { "bot_disabled_before_during_after": true }
        }),
    );

    let output = run_checkpoint(&fixture, None, None, None);

    assert_rejected(&output, "Offline soak evidence contract");
}

#[test]
fn evidence_integrity_04_rejects_powershell_changed_after_gate() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    fs::write(
        fixture.root.join("codex-discord-rust-soak.ps1"),
        b"Write-Output 'changed after verification'\n",
    )
    .unwrap();

    let output = run_checkpoint(&fixture, None, None, None);

    assert_rejected(&output, "PowerShell source hash mismatch");
}

#[test]
fn evidence_integrity_05_rejects_failed_required_workspace_gate() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let mut evidence = read_json(&fixture.workspace_evidence);
    evidence["rust"]["cargo_clippy_workspace_all_targets_locked_offline_deny_warnings"] =
        json!("failed_exit_1");
    write_json(&fixture.workspace_evidence, &evidence);

    let output = run_checkpoint(&fixture, None, None, None);

    assert_rejected(&output, "Workspace gate evidence contract");
}

#[test]
fn evidence_integrity_06_rejects_false_short_soak_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let mut evidence = read_json(&fixture.soak_evidence);
    evidence["provenance"]["harness_provenance_verified"] = json!(false);
    write_json(&fixture.soak_evidence, &evidence);

    let output = run_checkpoint(&fixture, None, None, None);

    assert_rejected(&output, "Offline soak evidence contract");
}

#[test]
fn evidence_integrity_07_rejects_array_final_eligibility_status() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let mut evidence = read_json(&fixture.soak_evidence);
    evidence["final_eligibility"]["status"] = json!(["ineligible"]);
    write_json(&fixture.soak_evidence, &evidence);

    let output = run_checkpoint(&fixture, None, None, None);

    assert_rejected(&output, "Offline soak evidence contract");
}

#[test]
fn evidence_integrity_08_rejects_array_long_run_status() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let mut evidence = read_json(&fixture.soak_evidence);
    evidence["long_run"]["status"] = json!(["pending"]);
    write_json(&fixture.soak_evidence, &evidence);

    let output = run_checkpoint(&fixture, None, None, None);

    assert_rejected(&output, "Offline soak evidence contract");
}

#[test]
fn evidence_integrity_09_rejects_singleton_soak_root_array_in_both_shells() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    wrap_json_in_singleton_array(&fixture.soak_evidence);

    for shell in ["powershell.exe", "pwsh.exe"] {
        let output = run_evidence_bundle_in_shell(&fixture, shell);
        assert_rejected(&output, "root must be a JSON object");
    }
}

#[test]
fn evidence_integrity_10_rejects_singleton_workspace_root_array_in_both_shells() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    wrap_json_in_singleton_array(&fixture.workspace_evidence);

    for shell in ["powershell.exe", "pwsh.exe"] {
        let output = run_evidence_bundle_in_shell(&fixture, shell);
        assert_rejected(&output, "root must be a JSON object");
    }
}

#[test]
fn evidence_integrity_11_rejects_singleton_nested_soak_record_in_both_shells() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let mut evidence = read_json(&fixture.soak_evidence);
    evidence["harness"] = json!([evidence["harness"].clone()]);
    write_json(&fixture.soak_evidence, &evidence);

    assert_contract_rejected_in_both_shells(&fixture.soak_evidence, "soak");
}

#[test]
fn evidence_integrity_12_rejects_singleton_nested_workspace_record_in_both_shells() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let mut evidence = read_json(&fixture.workspace_evidence);
    evidence["rust"] = json!([evidence["rust"].clone()]);
    write_json(&fixture.workspace_evidence, &evidence);

    assert_contract_rejected_in_both_shells(&fixture.workspace_evidence, "workspace");
}

#[test]
fn evidence_integrity_13_rejects_nonzero_missing_or_string_soak_process_counts() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let baseline = read_json(&fixture.soak_evidence);
    for field in [
        "production_runtime_process_count_after",
        "offline_soak_process_count_after",
        "mcp_server_process_count_after",
    ] {
        let mut evidence = baseline.clone();
        evidence["safety"][field] = json!(1);
        write_json(&fixture.soak_evidence, &evidence);
        assert_contract_rejected_in_both_shells(&fixture.soak_evidence, "soak");
    }
    let mut missing = baseline.clone();
    missing["safety"]
        .as_object_mut()
        .unwrap()
        .remove("production_runtime_process_count_after");
    write_json(&fixture.soak_evidence, &missing);
    assert_contract_rejected_in_both_shells(&fixture.soak_evidence, "soak");
    let mut stringified = baseline;
    stringified["safety"]["production_runtime_process_count_after"] = json!("0");
    write_json(&fixture.soak_evidence, &stringified);
    assert_contract_rejected_in_both_shells(&fixture.soak_evidence, "soak");
}

#[test]
fn evidence_integrity_14_rejects_stale_or_malformed_checkpoint_gate_counts() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let baseline = read_json(&fixture.workspace_evidence);

    let mut missing = baseline.clone();
    missing["rust"]
        .as_object_mut()
        .unwrap()
        .remove("release_checkpoint_contracts");
    write_json(&fixture.workspace_evidence, &missing);
    assert_contract_rejected_in_both_shells(&fixture.workspace_evidence, "workspace");

    let mut failed = baseline.clone();
    failed["rust"]["release_checkpoint_contracts"]["failed"] = json!(1);
    write_json(&fixture.workspace_evidence, &failed);
    assert_contract_rejected_in_both_shells(&fixture.workspace_evidence, "workspace");

    let mut stringified = baseline.clone();
    stringified["rust"]["release_checkpoint_contracts"]["base"] = json!("16");
    write_json(&fixture.workspace_evidence, &stringified);
    assert_contract_rejected_in_both_shells(&fixture.workspace_evidence, "workspace");

    let mut array = baseline;
    let checkpoint = array["rust"]["release_checkpoint_contracts"].clone();
    array["rust"]["release_checkpoint_contracts"] = json!([checkpoint]);
    write_json(&fixture.workspace_evidence, &array);
    assert_contract_rejected_in_both_shells(&fixture.workspace_evidence, "workspace");
}
