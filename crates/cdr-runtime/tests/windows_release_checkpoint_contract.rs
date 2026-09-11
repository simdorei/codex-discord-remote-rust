#![cfg(windows)]

#[path = "support/windows_release_checkpoint.rs"]
mod support;

use std::fs;
use std::io::ErrorKind;
use std::os::windows::fs::symlink_file;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;
use support::{
    MARKER_BYTES, assert_no_generated_checkpoint, assert_preexisting_checkpoint_targets_survive,
    checkpoint_tools, create_fixture, repo_root, run_archive_publish_failure_probe, run_checkpoint,
    run_external_output_override_probe, run_injected_bot_off_guard_probe,
    run_process_enumeration_failure_probe, sha256,
};

#[test]
fn release_checkpoint_tooling_declares_full_safety_contract() {
    let scripts = repo_root().join("scripts");
    let tooling = checkpoint_tools()
        .iter()
        .map(|tool| fs::read_to_string(scripts.join(tool)).expect("checkpoint tool"))
        .collect::<String>();
    for required in [
        "ExpectedRuntimeSha256",
        "BD1A83372D5FE95BBE27D76FE344826098CD761F02171187905E8CA9505C6813",
        "cdr-runtime.exe",
        "cdr-offline-soak.exe",
        "cdr-mcp-server.exe",
        "cdr-pro-helper.exe",
        "SHA256SUMS",
        "ARCHIVE-METADATA.json",
        "bot_disabled",
        "local_checkpoint_only",
        "immutable_release_checkpoint_still_pending_authorization",
        "Expand-Archive",
        "--duration-secs",
        "FileAttributes]::ReparsePoint",
        "Assert-CdrCheckpointSource",
        "Assert-CdrPathUnderRoot",
        "Get-CdrForbiddenProcessIdentity",
        "AllowExternalOutput",
        "external_output_override",
        "OfflineSoakEvidencePath",
        "WorkspaceGateEvidencePath",
        "final_source_local_checkpoint",
        "source_fingerprint",
        "FileShare]::Read",
        "-ArchiveStream $archiveGuard",
        "$archiveHash = $verification.ArchiveHash",
        "$sha.ComputeHash($ArchiveStream)",
        "Publish-CdrCheckpointArchive",
        "Assert-CdrCheckpointBotOff",
        "Get-CdrLegacyPythonBotProcessSnapshot",
    ] {
        assert!(
            tooling.contains(required),
            "missing checkpoint contract: {required}"
        );
    }
    let package = fs::read_to_string(scripts.join("RustMigrationCheckpoint.Package.psm1")).unwrap();
    let stage = fs::read_to_string(scripts.join("RustMigrationCheckpoint.Stage.psm1")).unwrap();
    assert!(package.contains("RustMigrationCheckpoint.Stage.psm1"));
    let guard = stage
        .find("Assert-CdrCheckpointSource $Source $RepoRoot")
        .unwrap();
    let copy = stage.find("Copy-Item -LiteralPath $Source").unwrap();
    assert!(guard < copy, "reparse guard must run before Copy-Item");
    let launcher = fs::read_to_string(scripts.join("New-RustMigrationCheckpoint.ps1")).unwrap();
    assert!(!launcher.contains("$archiveHash = Get-CdrSha256 $archivePath"));
    let archive_guard = launcher.find("$archiveGuard = [IO.File]::Open").unwrap();
    let verification = launcher
        .find("$verification = Test-CdrCheckpointArchive")
        .unwrap();
    let trusted_hash = launcher
        .find("$archiveHash = $verification.ArchiveHash")
        .unwrap();
    let evidence_write = launcher.find("$evidenceStream = [IO.File]::Open").unwrap();
    let guard_release = launcher.rfind("$archiveGuard.Dispose()").unwrap();
    assert!(archive_guard < verification);
    assert!(verification < trusted_hash);
    assert!(trusted_hash < evidence_write);
    assert!(evidence_write < guard_release);
}

#[test]
fn failed_archive_compression_never_publishes_a_partial_final_name() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join(".codex_discord_bot.disabled");
    let evidence = temp.path().join("checkpoint-evidence.json");
    fs::write(&marker, MARKER_BYTES).unwrap();

    let output = run_archive_publish_failure_probe(temp.path());

    assert!(
        output.status.success(),
        "archive publish probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!temp.path().join("final-checkpoint.zip").exists());
    assert!(!evidence.exists());
    assert_eq!(fs::read(marker).unwrap(), MARKER_BYTES);
}

#[test]
fn injected_native_and_python_bot_identities_fail_the_bot_off_guard() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join(".codex_discord_bot.disabled");
    let archive = temp.path().join("checkpoint.zip");
    let evidence = temp.path().join("checkpoint.json");
    fs::write(&marker, MARKER_BYTES).unwrap();
    fs::write(temp.path().join("codex_discord_bot.py"), b"# fixture\n").unwrap();

    for shell in ["powershell.exe", "pwsh.exe"] {
        let output = run_injected_bot_off_guard_probe(temp.path(), shell);
        assert!(
            output.status.success(),
            "bot-off injection probe failed in {shell}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("bot_off_rejections=19"),
            "{shell}: {stdout}"
        );
        assert!(stdout.contains("bot_off_negatives=13"), "{shell}: {stdout}");
        assert!(
            stdout.contains("actual_short_alias_cases="),
            "{shell}: {stdout}"
        );
        assert!(
            stdout.contains("short_path_failure_closed=true"),
            "{shell}: {stdout}"
        );
    }
    assert!(!archive.exists());
    assert!(!evidence.exists());
    assert_eq!(fs::read(marker).unwrap(), MARKER_BYTES);
}

#[test]
fn checkpoint_tooling_is_split_and_packaged_by_responsibility() {
    let scripts = repo_root().join("scripts");
    let launcher = fs::read_to_string(scripts.join(checkpoint_tools()[0])).unwrap();
    assert!(
        launcher.lines().count() < 250,
        "checkpoint launcher is not thin"
    );
    let package = fs::read_to_string(scripts.join(checkpoint_tools()[2])).unwrap();
    let payload = fs::read_to_string(scripts.join("RustMigrationCheckpoint.Payload.psm1")).unwrap();
    for helper in checkpoint_tools() {
        let text = fs::read_to_string(scripts.join(helper)).unwrap();
        assert!(
            text.lines().count() < 250,
            "checkpoint tool is too large: {helper}"
        );
        assert!(package.contains(helper) || payload.contains(helper) || launcher.contains(helper));
    }
}

#[test]
fn checkpoint_creation_verifies_a_freshly_extracted_offline_fixture() {
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
    let archive = PathBuf::from(result["archive_path"].as_str().unwrap());
    let evidence = PathBuf::from(result["evidence_path"].as_str().unwrap());
    assert!(archive.starts_with(fixture.root.join(".codex-discord-backups")));
    assert!(evidence.starts_with(fixture.root.join("docs/rust-migration/evidence")));
    assert_eq!(fs::read(&fixture.marker).unwrap(), MARKER_BYTES);
    let proof: Value = serde_json::from_slice(&fs::read(evidence).unwrap()).unwrap();
    assert_eq!(proof["local_archive_verified"], true);
    assert_eq!(proof["checkpoint_stage"], "final_source_local_checkpoint");
    assert_eq!(proof["verification"]["forbidden_entry_count"], 0);
    assert_eq!(proof["verification"]["offline_smoke"]["status"], "passed");
    assert_eq!(proof["safety"]["external_output_override"], false);
    assert_eq!(proof["archive"]["sha256"], sha256(&archive));
    let entries = support::archive_entries(&archive);
    for required in [
        "workspace/Cargo.toml",
        "workspace/Cargo.lock",
        "workspace/rust-toolchain.toml",
    ] {
        assert!(
            entries.iter().any(|entry| entry == required),
            "missing {required}"
        );
    }
    for forbidden in [
        "workspace/RUST_MIGRATION_HANDOFF.md",
        "evidence/rollback-snapshot-20260831T082055Z.json",
    ] {
        assert!(
            entries.iter().all(|entry| entry != forbidden),
            "stale or self-referential payload was packaged: {forbidden}"
        );
    }
    assert_eq!(
        proof["verified_inputs"]["offline_soak_evidence_sha256"],
        sha256(&fixture.soak_evidence)
    );
    assert_eq!(
        proof["verified_inputs"]["workspace_gate_evidence_sha256"],
        sha256(&fixture.workspace_evidence)
    );
    assert!(
        proof["verified_inputs"]["source_fingerprint"]
            .as_str()
            .is_some_and(|value| value.len() == 64)
    );
}

#[test]
fn checkpoint_rejects_workspace_gate_evidence_for_different_source() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let mut evidence: Value =
        serde_json::from_slice(&fs::read(&fixture.workspace_evidence).unwrap()).unwrap();
    evidence["source_fingerprint"] = "0".repeat(64).into();
    // Keep the stale record internally consistent to exercise the actual-source guard.
    evidence["native_tools"]["dependency_audit"]["source_fingerprint"] = "0".repeat(64).into();
    evidence["native_tools"]["process_observation"]["source_fingerprint"] = "0".repeat(64).into();
    fs::write(
        &fixture.workspace_evidence,
        serde_json::to_vec(&evidence).unwrap(),
    )
    .unwrap();

    let output = run_checkpoint(&fixture, None, None, None);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Workspace gate evidence source fingerprint does not match")
    );
    assert_no_generated_checkpoint(&fixture);
}

#[test]
fn checkpoint_rejects_short_soak_evidence_for_different_release_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let mut evidence: Value =
        serde_json::from_slice(&fs::read(&fixture.soak_evidence).unwrap()).unwrap();
    evidence["artifacts"]["cdr_runtime"]["sha256"] = "0".repeat(64).into();
    fs::write(
        &fixture.soak_evidence,
        serde_json::to_vec(&evidence).unwrap(),
    )
    .unwrap();

    let output = run_checkpoint(&fixture, None, None, None);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Offline soak evidence cdr-runtime.exe SHA-256 mismatch")
    );
    assert_no_generated_checkpoint(&fixture);
}

#[test]
fn archive_output_outside_repository_backup_root_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let outside = temp.path().join("outside-archives");
    let output = run_checkpoint(&fixture, Some(&outside), None, None);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("OutputDirectory must remain under"));
    assert!(!outside.exists());
    assert_no_generated_checkpoint(&fixture);
}

#[test]
fn evidence_output_outside_repository_evidence_root_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let outside = temp.path().join("outside-evidence");
    let output = run_checkpoint(&fixture, None, Some(&outside), None);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("EvidenceOutputDirectory must remain under")
    );
    assert!(!outside.exists());
    assert_no_generated_checkpoint(&fixture);
}

#[test]
fn explicit_external_output_override_reaches_payload_validation() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let output_root = temp.path().join("operator-archive");
    let evidence_root = temp.path().join("operator-evidence");
    let output = run_external_output_override_probe(&fixture, &output_root, &evidence_root);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cdr-runtime.exe SHA-256 mismatch"),
        "{stderr}"
    );
    assert!(!stderr.contains("must remain under"), "{stderr}");
    assert_eq!(fs::read_dir(&output_root).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&evidence_root).unwrap().count(), 0);
    assert_no_generated_checkpoint(&fixture);
}

#[test]
fn database_snapshot_outside_repository_backup_root_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let outside = temp.path().join("outside.sqlite");
    fs::write(&outside, b"verified rollback fixture").unwrap();
    let output = run_checkpoint(&fixture, None, None, Some(&outside));
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("DatabaseSnapshotPath must remain under")
    );
    assert_no_generated_checkpoint(&fixture);
}

#[test]
fn reparse_point_payload_source_is_rejected_before_copy() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let source = fixture.root.join("install.sh");
    let target = fixture.root.join("install-real.sh");
    fs::remove_file(&source).unwrap();
    fs::write(&target, b"fixture=install-real.sh\n").unwrap();
    if let Err(error) = symlink_file(&target, &source) {
        assert!(error.kind() == ErrorKind::PermissionDenied || error.raw_os_error() == Some(1314));
        let common =
            fs::read_to_string(repo_root().join("scripts/RustMigrationCheckpoint.Common.psm1"))
                .unwrap();
        assert!(common.contains("FileAttributes]::ReparsePoint"));
        return;
    }
    let output = run_checkpoint(&fixture, None, None, None);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("reparse point"));
    assert_no_generated_checkpoint(&fixture);
}

#[test]
fn forbidden_process_start_time_failure_is_clear_and_closed() {
    let common = repo_root().join("scripts/RustMigrationCheckpoint.Common.psm1");
    let command = format!(
        "Import-Module '{}'; $p=[pscustomobject]@{{Id=77}}; $p | Add-Member ScriptProperty StartTime {{ throw 'access denied fixture' }}; Get-CdrForbiddenProcessIdentity $p 'cdr-runtime'",
        common.display().to_string().replace('\'', "''")
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &command])
        .output()
        .expect("run fail-closed process identity probe");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Cannot verify forbidden process StartTime"),
        "{stderr}"
    );
    assert!(
        stderr.contains("cdr-runtime") && stderr.contains("77"),
        "{stderr}"
    );
}

#[test]
fn preexisting_checkpoint_targets_survive_rejection_byte_for_byte() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    assert_preexisting_checkpoint_targets_survive(&fixture);
}

#[test]
fn forbidden_process_enumeration_failure_is_clear_and_closed() {
    let output = run_process_enumeration_failure_probe();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Cannot enumerate forbidden process"),
        "{stderr}"
    );
    assert!(stderr.contains("cdr-runtime"), "{stderr}");
}

#[test]
fn powershell_parser_accepts_all_checkpoint_tools() {
    for tool in checkpoint_tools() {
        let script = repo_root().join("scripts").join(tool);
        let command = format!(
            "$e=$null; [void][Management.Automation.Language.Parser]::ParseFile('{}',[ref]$null,[ref]$e); if ($e.Count) {{ $e | % Message; exit 1 }}",
            script.display().to_string().replace('\'', "''")
        );
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &command])
            .output()
            .expect("parse checkpoint tool");
        assert!(output.status.success(), "parse failed for {tool}");
    }
}
