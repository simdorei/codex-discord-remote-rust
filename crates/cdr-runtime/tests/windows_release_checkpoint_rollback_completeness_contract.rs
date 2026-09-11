#![cfg(windows)]
#![allow(dead_code)]

#[path = "support/windows_release_checkpoint.rs"]
mod support;

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;
use support::{assert_no_generated_checkpoint, create_fixture, repo_root, run_checkpoint};

fn source_binding_records(shell: &str) -> Value {
    let module = repo_root().join("scripts/RustMigrationCheckpoint.SourceBinding.psm1");
    let root = repo_root();
    let command = format!(
        "Import-Module '{}'; [ordered]@{{ rollback = Get-CdrCheckpointRollbackSourceRecord -RepoRoot '{}'; powershell = Get-CdrCheckpointPowerShellSourceRecord -RepoRoot '{}' }} | ConvertTo-Json -Depth 8 -Compress",
        module.display().to_string().replace('\'', "''"),
        root.display().to_string().replace('\'', "''"),
        root.display().to_string().replace('\'', "''")
    );
    let output = Command::new(shell)
        .args(["-NoProfile", "-Command", &command])
        .output()
        .unwrap_or_else(|error| panic!("start {shell}: {error}"));
    assert!(
        output.status.success(),
        "{shell} source binding failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("source binding JSON")
}

fn assert_source_binding_rejects_non_scalar_or_string_numbers(shell: &str) {
    let command = r#"
$ErrorActionPreference = 'Stop'
Import-Module $env:CDR_SOURCE_BINDING_MODULE -Force -ErrorAction Stop
function Assert-RejectedMutation {
    param([string]$Kind, [string]$Mutation, [string]$Json)
    $record = $Json | ConvertFrom-Json
    switch ($Mutation) {
        'schema_array' { $record.schema = [object[]]@([string]$record.schema) }
        'file_count_string' { $record.file_count = [string]$record.file_count }
        'bytes_string' { $record.files[0].bytes = [string]$record.files[0].bytes }
        default { throw "unknown mutation $Mutation" }
    }
    $accepted = $false
    try {
        if ($Kind -ceq 'rollback') {
            $null = Assert-CdrCheckpointRollbackSourceRecord -Record $record -RepoRoot $env:CDR_REPO_ROOT
        } else {
            $null = Assert-CdrCheckpointPowerShellSourceRecord -Record $record -RepoRoot $env:CDR_REPO_ROOT
        }
        $accepted = $true
    } catch {}
    if ($accepted) { throw "$Kind accepted $Mutation" }
}
$rollback = Get-CdrCheckpointRollbackSourceRecord -RepoRoot $env:CDR_REPO_ROOT |
    ConvertTo-Json -Depth 8 -Compress
$powershell = Get-CdrCheckpointPowerShellSourceRecord -RepoRoot $env:CDR_REPO_ROOT |
    ConvertTo-Json -Depth 8 -Compress
foreach ($mutation in @('schema_array', 'file_count_string', 'bytes_string')) {
    Assert-RejectedMutation 'rollback' $mutation $rollback
    Assert-RejectedMutation 'powershell' $mutation $powershell
}
Write-Output 'passed'
"#;
    let output = Command::new(shell)
        .args(["-NoProfile", "-Command", command])
        .env(
            "CDR_SOURCE_BINDING_MODULE",
            repo_root().join("scripts/RustMigrationCheckpoint.SourceBinding.psm1"),
        )
        .env("CDR_REPO_ROOT", repo_root())
        .output()
        .unwrap_or_else(|error| panic!("start {shell}: {error}"));
    assert!(
        output.status.success(),
        "{shell} accepted a non-scalar or stringified number: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_same_order(left: &Value, right: &Value, record: &str) {
    let left = left[record]["files"].as_array().unwrap();
    let right = right[record]["files"].as_array().unwrap();
    assert_eq!(left.len(), right.len(), "{record} record count mismatch");
    if let Some(index) = left.iter().zip(right).position(|(a, b)| a != b) {
        panic!(
            "{record} record mismatch at {index}: {} vs {}",
            left[index]["path"], right[index]["path"]
        );
    }
    let mut ordinal = left.clone();
    ordinal.sort_by(|a, b| a["path"].as_str().unwrap().cmp(b["path"].as_str().unwrap()));
    assert_eq!(left, &ordinal, "{record} record is not ordinally sorted");
}

#[test]
fn source_binding_order_matches_windows_powershell_and_pwsh() {
    let windows_powershell = source_binding_records("powershell.exe");
    let pwsh = source_binding_records("pwsh.exe");
    assert_same_order(&windows_powershell, &pwsh, "rollback");
    assert_same_order(&windows_powershell, &pwsh, "powershell");
}

#[test]
fn source_binding_types_are_strict_in_windows_powershell_and_pwsh() {
    for shell in ["powershell.exe", "pwsh.exe"] {
        assert_source_binding_rejects_non_scalar_or_string_numbers(shell);
    }
}

#[test]
fn checkpoint_proves_the_extracted_rust_rollback_is_source_complete() {
    let temp = tempfile::tempdir().expect("create fixture tempdir");
    let fixture = create_fixture(&temp.path().join("repo"));
    let output = run_checkpoint(&fixture, None, None, None);
    assert!(
        output.status.success(),
        "checkpoint failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let result: Value = serde_json::from_slice(&output.stdout).expect("checkpoint result JSON");
    let evidence_path = PathBuf::from(result["evidence_path"].as_str().unwrap());
    let evidence: Value = serde_json::from_slice(&fs::read(evidence_path).unwrap()).unwrap();
    let rollback = &evidence["verification"]["source_rollback"];

    assert_eq!(rollback["status"], "passed");
    assert_eq!(
        rollback["scope"],
        "local_windows_bot_and_operational_tooling"
    );
    assert_eq!(rollback["powershell_watchdog_dry_run"], "passed_disabled");
    assert_eq!(rollback["powershell_source_parse"], "passed");
    assert_eq!(rollback["memory_ab_module_import_preflight"], "passed");
    assert_eq!(rollback["rust_setup_preflight"], "passed");
    assert_eq!(rollback["rust_pro_helper_preflight"], "passed");
    assert_eq!(rollback["requires_python"], false);
    assert!(
        rollback["source_file_count"]
            .as_u64()
            .is_some_and(|count| count > 1)
    );
    assert!(
        rollback["powershell_source_file_count"]
            .as_u64()
            .is_some_and(|count| count >= 7)
    );
    assert!(
        evidence["verified_inputs"]["rollback_source_file_count"]
            .as_u64()
            .is_some_and(|count| count > 1)
    );
}

#[test]
fn checkpoint_tooling_names_the_watchdog_dependencies_and_honest_boundary() {
    let scripts = support::repo_root().join("scripts");
    let tooling = support::checkpoint_tools()
        .iter()
        .map(|tool| fs::read_to_string(scripts.join(tool)).expect("checkpoint tool"))
        .collect::<String>();

    for required in [
        "codex-discord-atomic-file-runtime.ps1",
        "codex-discord-bot-headless.vbs",
        "codex-discord-memory-ab-common.ps1",
        "codex-discord-memory-ab-process.ps1",
        "codex-discord-memory-ab-report.ps1",
        "codex-discord-memory-ab-sample.ps1",
        "codex-discord-memory-ab-validation.ps1",
        "codex-discord-memory-ab-compare.ps1",
        "powershell_watchdog_dry_run",
        "powershell_source_parse",
        "memory_ab_module_import_preflight",
        "rust_pro_helper_preflight",
        "requires_python",
        "rollback_source_file_count",
    ] {
        assert!(
            tooling.contains(required),
            "missing rollback contract: {required}"
        );
    }
}

#[test]
fn checkpoint_rejects_operational_rollback_source_changed_after_the_workspace_gate() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    fs::write(
        fixture.root.join("codex-discord-helper.sh"),
        b"VALUE=false\n",
    )
    .unwrap();

    let output = run_checkpoint(&fixture, None, None, None);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Rollback source hash mismatch"));
    assert_no_generated_checkpoint(&fixture);
}

#[test]
fn checkpoint_rejects_operational_rollback_source_added_after_the_workspace_gate() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    fs::write(
        fixture.root.join("codex-discord-unverified.sh"),
        b"UNVERIFIED=true\n",
    )
    .unwrap();

    let output = run_checkpoint(&fixture, None, None, None);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Workspace gate rollback source contract failed: file count is invalid")
    );
    assert_no_generated_checkpoint(&fixture);
}
