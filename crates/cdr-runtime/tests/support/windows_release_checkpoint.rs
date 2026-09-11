#![allow(dead_code)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, MutexGuard};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const MARKER_BYTES: &[u8] = b"operator_disabled\n";
static CHECKPOINT_PROCESS_TEST_LOCK: Mutex<()> = Mutex::new(());

fn checkpoint_process_test_lock() -> MutexGuard<'static, ()> {
    CHECKPOINT_PROCESS_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub struct Fixture {
    pub root: PathBuf,
    pub runtime: PathBuf,
    pub snapshot: PathBuf,
    pub marker: PathBuf,
    pub soak_evidence: PathBuf,
    pub workspace_evidence: PathBuf,
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn sha256(path: &Path) -> String {
    hex::encode_upper(Sha256::digest(
        fs::read(path).expect("read file for SHA-256"),
    ))
}

pub fn archive_entries(path: &Path) -> Vec<String> {
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            "Add-Type -AssemblyName System.IO.Compression.FileSystem; $archive=[IO.Compression.ZipFile]::OpenRead($env:CDR_ARCHIVE_PATH); try { $archive.Entries | ForEach-Object { $_.FullName } } finally { $archive.Dispose() }",
        ])
        .env("CDR_ARCHIVE_PATH", path)
        .output()
        .expect("read checkpoint archive entries");
    assert!(
        output.status.success(),
        "archive listing failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim().replace('\\', "/"))
        .filter(|line| !line.is_empty())
        .collect()
}

#[allow(clippy::too_many_lines)]
pub fn create_fixture(root: &Path) -> Fixture {
    let release = root.join("target/release");
    fs::create_dir_all(&release).expect("create fixture release directory");
    let runtime = release.join("cdr-runtime.exe");
    fs::copy(env!("CARGO_BIN_EXE_cdr-runtime"), &runtime).expect("copy runtime fixture");
    for name in ["cdr-offline-soak.exe", "cdr-mcp-server.exe"] {
        fs::copy(env!("CARGO_BIN_EXE_cdr-offline-soak"), release.join(name))
            .expect("copy offline executable fixture");
    }
    let helper = Path::new(env!("CARGO_BIN_EXE_cdr-runtime"))
        .parent()
        .unwrap()
        .join("cdr-pro-helper.exe");
    fs::copy(&helper, release.join("cdr-pro-helper.exe"))
        .expect("build the native helper first: cargo build -p cdr-pro --bin cdr-pro-helper");
    write_named_files(
        root,
        &[
            "codex-discord-runtime-cutover.ps1",
            "codex-discord-rust-watchdog.ps1",
            "codex-discord-watchdog.ps1",
            "codex-discord-atomic-file-runtime.ps1",
            "codex-discord-bot-headless.vbs",
            "codex-discord-memory-ab.ps1",
            "codex-discord-rust-restart.ps1",
            "codex-discord-rust-status.ps1",
            "codex-discord-rust-soak.ps1",
            "codex-discord-watchdog-hidden.vbs",
            "codex-discord-bot.cmd",
            "codex-discord-helper.sh",
            "install.ps1",
            "install.sh",
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "RUST_MIGRATION_HANDOFF.md",
        ],
    );
    fs::write(
        root.join("codex-discord-watchdog.ps1"),
        b"param([switch]$DryRun)\n. (Join-Path $PSScriptRoot 'codex-discord-atomic-file-runtime.ps1')\nif (-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot '.codex_discord_bot.disabled'))) { throw 'disabled marker missing' }\nWrite-Output 'disabled'\n",
    )
    .expect("write safe watchdog fixture");
    fs::write(
        root.join("codex-discord-atomic-file-runtime.ps1"),
        b"# source rollback dependency fixture\n",
    )
    .expect("write atomic helper fixture");
    fs::write(root.join("codex-discord-helper.sh"), b"VALUE=true\n")
        .expect("write operational source fixture");
    fs::create_dir_all(root.join("crates/fixture/src")).expect("create fixture Rust source");
    fs::write(
        root.join("crates/fixture/src/lib.rs"),
        b"pub fn fixture() {}\n",
    )
    .expect("write fixture Rust source");
    let scripts = root.join("scripts");
    fs::create_dir_all(&scripts).expect("create fixture scripts");
    for tool in checkpoint_tools() {
        fs::copy(repo_root().join("scripts").join(tool), scripts.join(tool))
            .expect("copy checkpoint tool");
    }
    for helper in [
        "CdrCutoverState.ps1",
        "CdrCutoverRuntime.ps1",
        "CdrCutoverCompletion.ps1",
        "CdrCutoverRecovery.ps1",
        "codex-discord-memory-ab-common.ps1",
        "codex-discord-memory-ab-process.ps1",
        "codex-discord-memory-ab-report.ps1",
        "codex-discord-memory-ab-sample.ps1",
        "codex-discord-memory-ab-validation.ps1",
        "codex-discord-memory-ab-compare.ps1",
    ] {
        fs::copy(
            repo_root().join("scripts").join(helper),
            scripts.join(helper),
        )
        .expect("copy memory A/B rollback helper");
    }
    fs::write(
        scripts.join("RustMigrationCheckpoint.QualityApprovals.json"),
        r#"{"schema":"cdr.reviewed-quality-exceptions.v1","exceptions":[]}"#,
    )
    .unwrap();
    let (soak_evidence, workspace_evidence) = write_fixture_evidence(root, &runtime, &release);
    let backups = root.join(".codex-discord-backups");
    fs::create_dir_all(&backups).expect("create fixture backup directory");
    let snapshot = backups.join("rollback.sqlite");
    fs::write(&snapshot, b"verified rollback fixture").expect("write snapshot fixture");
    let marker = root.join(".codex_discord_bot.disabled");
    fs::write(&marker, MARKER_BYTES).expect("write disabled marker");
    let git = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(root)
        .output()
        .expect("initialize fixture git repository");
    assert!(git.status.success());
    Fixture {
        root: root.to_path_buf(),
        runtime,
        snapshot,
        marker,
        soak_evidence,
        workspace_evidence,
    }
}

fn write_fixture_evidence(root: &Path, runtime: &Path, release: &Path) -> (PathBuf, PathBuf) {
    let evidence = root.join("docs/rust-migration/evidence");
    fs::create_dir_all(&evidence).expect("create prior evidence directory");
    fs::write(
        evidence.join("rollback-snapshot-20260831T082055Z.json"),
        b"{\"status\":\"passed\"}\n",
    )
    .expect("write rollback evidence");
    let source = source_fingerprint_record(root);
    let powershell_source = powershell_source_record(root);
    let rollback_source = rollback_source_record(root);
    let soak_evidence = evidence.join("windows-offline-soak-final.json");
    fs::write(
        &soak_evidence,
        serde_json::to_vec(&short_soak_record(runtime, release, &source)).unwrap(),
    )
    .expect("write soak evidence");
    let workspace_evidence = evidence.join("windows-full-workspace-gate-final.json");
    fs::write(
        &workspace_evidence,
        serde_json::to_vec(&workspace_gate_record(
            runtime,
            release,
            &source,
            &powershell_source,
            &rollback_source,
        ))
        .unwrap(),
    )
    .expect("write workspace evidence");
    (soak_evidence, workspace_evidence)
}

pub fn refresh_fixture_evidence(fixture: &Fixture) {
    write_fixture_evidence(
        &fixture.root,
        &fixture.runtime,
        &fixture.root.join("target/release"),
    );
}

fn artifact_record(path: &Path) -> Value {
    json!({
        "sha256": sha256(path),
        "bytes": fs::metadata(path).unwrap().len(),
        "pe_magic": "MZ"
    })
}

fn short_soak_record(runtime: &Path, release: &Path, source: &Value) -> Value {
    json!({
        "schema_version": 1,
        "kind": "windows_offline_fake_replay_soak_smoke",
        "status": "passed_short_smoke_only",
        "operational_status": "passed",
        "harness": {
            "status": "passed", "cycles": 1,
            "queue_submitted": 1, "queue_completed": 1,
            "outbox_staged": 1, "outbox_delivered": 1,
            "mirror_send_failures": 1, "mirror_send_retries": 1,
            "exact_message_retries": 1, "duplicate_successes": 0,
            "target_stalls": 0, "queue_remaining": 0, "outbox_remaining": 0
        },
        "memory": {
            "sample_count": 2, "threshold_passed": true,
            "interpretation":
                "startup_plumbing_only_not_a_stability_or_production_memory_no_regression_result"
        },
        "artifacts": {
            "cdr_runtime": artifact_record(runtime),
            "cdr_offline_soak": artifact_record(&release.join("cdr-offline-soak.exe")),
            "cdr_mcp_server": artifact_record(&release.join("cdr-mcp-server.exe")),
            "cdr_pro_helper": artifact_record(&release.join("cdr-pro-helper.exe"))
        },
        "provenance": {
            "same_run_canonical_release_build": true, "canonical_release_harness": true,
            "source_fingerprint": source["aggregate_sha256"],
            "source_file_count": source["file_count"],
            "source_total_bytes": source["total_bytes"],
            "build_before_after_equal": true, "soak_before_after_equal": true,
            "harness_hash_expected_before_after_equal": true,
            "harness_provenance_verified": true,
            "local_summary_sha256": "A".repeat(64)
        },
        "safety": {
            "bot_disabled_before_during_after": true,
            "operator_disabled_marker_preserved": true, "child_exit_code": 0,
            "cleanup_clean": true, "discord_or_network_boundary_invoked": false,
            "secrets_in_evidence": false,
            "production_runtime_process_count_after": 0,
            "offline_soak_process_count_after": 0,
            "mcp_server_process_count_after": 0
        },
        "final_eligibility": {
            "status": "ineligible", "eligible": false,
            "expected_for_short_smoke": true
        },
        "long_run": { "completed": false, "status": "pending" }
    })
}

fn workspace_gate_record(
    runtime: &Path,
    release: &Path,
    source: &Value,
    powershell_source: &Value,
    rollback_source: &Value,
) -> Value {
    json!({
        "schema_version": 2, "kind": "windows_full_workspace_gate",
        "status": "passed", "platform": "windows",
        "source_fingerprint": source["aggregate_sha256"],
        "source_scope": {
            "schema": source["schema"], "file_count": source["file_count"],
            "total_bytes": source["total_bytes"]
        },
        "rust": {
            "cargo_test_workspace_all_targets_locked_offline_no_fail_fast": "passed_exit_0",
            "cargo_clippy_workspace_all_targets_locked_offline_deny_warnings": "passed_exit_0",
            "cargo_fmt_all_check": "passed_exit_0",
            "cargo_doc_workspace_no_deps_locked_offline_deny_warnings": "passed_exit_0",
            "cargo_build_workspace_release_locked_offline": "passed_exit_0",
            "powershell_5_1_source_fingerprint_evidence_contract": "passed",
            "powershell_7_6_source_fingerprint_evidence_contract": "passed",
            "release_checkpoint_contracts": {
                "base": 16, "evidence_integrity": 19, "rollback_completeness": 6,
                "staged_binding": 5, "archive_adversarial": 1, "failed": 0
            }
        },
        "native_tools": {
            "operations_suite": { "passed": 1, "failed": 0 },
            "pro_helper_contracts": { "passed": 1, "failed": 0 },
            "installer_contracts": { "passed": 1, "failed": 0 },
            "desktop_bridge_contracts": { "passed": 1, "failed": 0 },
            "durable_store_contracts": { "passed": 1, "failed": 0 },
            "python_unavailable_execution": {
                "required": false, "status": "not_run", "reason": "User approved current-PC verification; shared Python stays installed."
            },
            "dependency_audit": {
                "scope": "deliverable_dependencies_and_callsites", "status": "passed",
                "source_fingerprint": source["aggregate_sha256"],
                "rollback_source_sha256": rollback_record_sha256(rollback_source),
                "command": "cargo test --test python_free_repository_contract",
                "exit_code": 0, "passed": 2, "failed": 0
            },
            "process_observation": {
                "schema": "cdr.current-pc-observation.v1", "status": "completed",
                "scope": "owned_descendants_current_pc",
                "source_fingerprint": source["aggregate_sha256"],
                "rollback_source_sha256": rollback_record_sha256(rollback_source),
                "operations": current_pc_operations()
            },
            "install_wrappers_dry_run": "passed"
        },
        "quality": {
            "rust_files_checked": 1, "rust_utf8_bom_count": 0, "rust_invalid_utf8_count": 0,
            "production_rust_files_over_250_lines": 0,
            "text_scope": "rust_and_checkpoint_rollback_sources",
            "text_files_checked": rollback_source["file_count"].as_u64().unwrap() + 1,
            "text_utf8_bom_count": 0, "text_invalid_utf8_count": 0,
            "exceptions": []
        },
        "artifacts": {
            "cdr_runtime": artifact_record(runtime),
            "cdr_offline_soak": artifact_record(&release.join("cdr-offline-soak.exe")),
            "cdr_mcp_server": artifact_record(&release.join("cdr-mcp-server.exe")),
            "cdr_pro_helper": artifact_record(&release.join("cdr-pro-helper.exe")),
            "repository_target_release_matches_external_build": true
        },
        "safety": {
            "bot_disabled": true, "disabled_marker_present": true, "runtime_mode": "rust",
            "runtime_process_count_after": 0, "offline_soak_process_count_after": 0,
            "mcp_server_process_count_after": 0, "live_discord_or_network_smoke_run": false,
            "secrets_in_evidence": false
        },
        "powershell_source": powershell_source,
        "rollback_source": rollback_source,
        "live_verification": { "status": "pending", "recorded_separately": true }
    })
}

fn current_pc_operations() -> Vec<Value> {
    [
        "install_wrappers",
        "setup_dry_run",
        "pro_helper_offline",
        "start_restart_contracts",
        "mcp_offline_contracts",
    ]
    .iter()
    .map(|id| {
        json!({
            "id": id, "command": format!("native fixture {id}"), "exit_code": 0,
            "status": "completed", "observer": "windows_process_start_stop_trace",
            "started_at": "2026-09-11T00:00:01Z", "ended_at": "2026-09-11T00:00:02Z",
            "observation_started_at": "2026-09-11T00:00:00Z",
            "observation_ended_at": "2026-09-11T00:00:03Z",
            "canary_observed": true, "owned_process_starts": 3,
            "command_process": {
                "observed": true, "pid": 60, "name": "fixture.exe",
                "created_at": "2026-09-11T00:00:01.1Z", "exited_at": "2026-09-11T00:00:01.9Z",
                "start_event_at": "2026-09-11T00:00:02.1Z", "stop_event_at": "2026-09-11T00:00:02.2Z"
            },
            "recognized_python_process_count": 0, "live_network_invoked": false
        })
    })
    .collect()
}

fn rollback_record_sha256(record: &Value) -> String {
    let mut frame = format!(
        "cdr.observation-rollback.v1\0{}\n",
        record["file_count"].as_u64().unwrap()
    );
    for row in record["files"].as_array().unwrap() {
        writeln!(
            frame,
            "{}\0{}\0{}\0{}",
            row["path"].as_str().unwrap(),
            row["archive_path"].as_str().unwrap(),
            row["sha256"].as_str().unwrap(),
            row["bytes"].as_u64().unwrap()
        )
        .unwrap();
    }
    hex::encode_upper(Sha256::digest(frame.as_bytes()))
}

fn source_fingerprint_record(root: &Path) -> Value {
    let module = repo_root().join("scripts/CodexDiscordSoak.SourceFingerprint.psm1");
    let command = format!(
        "Import-Module '{}'; Get-CodexSoakSourceFingerprint -RepoRoot '{}' | Select-Object schema,aggregate_sha256,file_count,total_bytes | ConvertTo-Json -Compress",
        module.display().to_string().replace('\'', "''"),
        root.display().to_string().replace('\'', "''")
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &command])
        .output()
        .expect("calculate fixture source fingerprint");
    assert!(
        output.status.success(),
        "source fingerprint failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn powershell_source_record(root: &Path) -> Value {
    let module = root.join("scripts/RustMigrationCheckpoint.SourceBinding.psm1");
    let command = format!(
        "Import-Module '{}'; Get-CdrCheckpointPowerShellSourceRecord -RepoRoot '{}' | ConvertTo-Json -Depth 5 -Compress",
        module.display().to_string().replace('\'', "''"),
        root.display().to_string().replace('\'', "''")
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &command])
        .output()
        .expect("calculate fixture PowerShell source record");
    assert!(
        output.status.success(),
        "PowerShell source record failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn rollback_source_record(root: &Path) -> Value {
    let module = root.join("scripts/RustMigrationCheckpoint.SourceBinding.psm1");
    let command = format!(
        "Import-Module '{}'; Get-CdrCheckpointRollbackSourceRecord -RepoRoot '{}' | ConvertTo-Json -Depth 6 -Compress",
        module.display().to_string().replace('\'', "''"),
        root.display().to_string().replace('\'', "''")
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &command])
        .output()
        .expect("calculate fixture rollback source record");
    assert!(
        output.status.success(),
        "rollback source record failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

pub fn run_checkpoint(
    fixture: &Fixture,
    output: Option<&Path>,
    evidence: Option<&Path>,
    snapshot: Option<&Path>,
) -> Output {
    let _process_guard = checkpoint_process_test_lock();
    let snapshot = snapshot.unwrap_or(&fixture.snapshot);
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(repo_root().join("scripts/New-RustMigrationCheckpoint.ps1"))
        .arg("-RepoRoot")
        .arg(&fixture.root);
    if let Some(path) = output {
        command.arg("-OutputDirectory").arg(path);
    }
    if let Some(path) = evidence {
        command.arg("-EvidenceOutputDirectory").arg(path);
    }
    command
        .arg("-DatabaseSnapshotPath")
        .arg(snapshot)
        .arg("-ExpectedRuntimeSha256")
        .arg(sha256(&fixture.runtime))
        .arg("-ExpectedDatabaseSha256")
        .arg(sha256(snapshot))
        .arg("-OfflineSoakEvidencePath")
        .arg(&fixture.soak_evidence)
        .arg("-WorkspaceGateEvidencePath")
        .arg(&fixture.workspace_evidence)
        .args(["-OfflineSmokeDurationSeconds", "1"])
        .output()
        .expect("run checkpoint creator")
}

pub fn run_post_verification_package_mutation_probe(
    fixture: &Fixture,
    mutation: &str,
) -> (Output, PathBuf) {
    let _process_guard = checkpoint_process_test_lock();
    let probe = fixture
        .root
        .parent()
        .expect("fixture parent")
        .join(format!("checkpoint-staged-binding-{mutation}.ps1"));
    let archive = fixture.root.join(format!(
        ".codex-discord-backups/staged-binding-{mutation}.zip"
    ));
    let staging = fixture.root.join(format!(".checkpoint-staging-{mutation}"));
    fs::write(
        &probe,
        r##"$ErrorActionPreference = 'Stop'
Import-Module $env:CDR_EVIDENCE_MODULE -Force -ErrorAction Stop
Import-Module $env:CDR_PACKAGE_MODULE -Force -ErrorAction Stop
$root = $env:CDR_FIXTURE_ROOT
$evidenceRoot = Join-Path $root 'docs\rust-migration\evidence'
$bundle = Get-CdrCheckpointEvidenceBundle `
    -RepoRoot $root `
    -RepositoryEvidenceRoot $evidenceRoot `
    -OfflineSoakEvidencePath $env:CDR_SOAK_EVIDENCE `
    -WorkspaceGateEvidencePath $env:CDR_WORKSPACE_EVIDENCE `
    -ExpectedRuntimeSha256 $env:CDR_RUNTIME_SHA256
$utf8 = [Text.UTF8Encoding]::new($false)
switch ($env:CDR_MUTATION_KIND) {
    'rollback' {
        [IO.File]::AppendAllText((Join-Path $root 'codex-discord-helper.sh'), "# changed`n", $utf8)
    }
    'artifact' {
        [IO.File]::AppendAllText((Join-Path $root 'target\release\cdr-mcp-server.exe'), 'x', $utf8)
    }
    'evidence' {
        [IO.File]::AppendAllText($env:CDR_WORKSPACE_EVIDENCE, ' ', $utf8)
    }
    'database' {
        [IO.File]::AppendAllText($env:CDR_DATABASE_SNAPSHOT, 'x', $utf8)
    }
    'workspace' {
        [IO.File]::AppendAllText((Join-Path $root 'Cargo.toml'), "# changed`n", $utf8)
    }
    default { throw "Unknown mutation probe: $env:CDR_MUTATION_KIND" }
}
$null = New-Item -ItemType Directory -Path $env:CDR_STAGING_ROOT -Force
$package = @{
    RepoRoot = $root
    StagingRoot = $env:CDR_STAGING_ROOT
    ArchivePath = $env:CDR_ARCHIVE_PATH
    DatabaseSnapshotPath = $env:CDR_DATABASE_SNAPSHOT
    ExpectedRuntimeSha256 = $env:CDR_RUNTIME_SHA256
    ExpectedDatabaseSha256 = $env:CDR_DATABASE_SHA256
    OfflineSoakSnapshot = $bundle.OfflineSoakSnapshot
    WorkspaceGateSnapshot = $bundle.WorkspaceGateSnapshot
    SourceFingerprint = $bundle.SourceFingerprint
    RollbackSource = $bundle.RollbackSource
    ExternalOutputOverride = $false
}
$null = New-CdrCheckpointPackage @package
"##,
    )
    .expect("write staged binding probe");
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&probe)
        .env(
            "CDR_EVIDENCE_MODULE",
            repo_root().join("scripts/RustMigrationCheckpoint.Evidence.psm1"),
        )
        .env(
            "CDR_PACKAGE_MODULE",
            repo_root().join("scripts/RustMigrationCheckpoint.Package.psm1"),
        )
        .env("CDR_FIXTURE_ROOT", &fixture.root)
        .env("CDR_SOAK_EVIDENCE", &fixture.soak_evidence)
        .env("CDR_WORKSPACE_EVIDENCE", &fixture.workspace_evidence)
        .env("CDR_DATABASE_SNAPSHOT", &fixture.snapshot)
        .env("CDR_RUNTIME_SHA256", sha256(&fixture.runtime))
        .env("CDR_DATABASE_SHA256", sha256(&fixture.snapshot))
        .env("CDR_MUTATION_KIND", mutation)
        .env("CDR_STAGING_ROOT", &staging)
        .env("CDR_ARCHIVE_PATH", &archive)
        .output()
        .expect("run staged binding mutation probe");
    (output, archive)
}

#[allow(clippy::too_many_lines)] // One cohesive embedded PowerShell adversarial fixture.
pub fn run_archive_verifier_adversarial_probe(fixture: &Fixture) -> Output {
    let _process_guard = checkpoint_process_test_lock();
    let probe_root = fixture.root.parent().expect("fixture parent");
    let probe = probe_root.join("checkpoint-archive-adversarial.ps1");
    let archive = fixture
        .root
        .join(".codex-discord-backups/archive-contract-original.zip");
    let staging = fixture.root.join(".archive-contract-stage");
    fs::write(
        &probe,
        r##"$ErrorActionPreference = 'Stop'
Import-Module $env:CDR_COMMON_MODULE -Force -ErrorAction Stop
Import-Module $env:CDR_EVIDENCE_MODULE -Force -ErrorAction Stop
Import-Module $env:CDR_PACKAGE_MODULE -Force -ErrorAction Stop
Import-Module $env:CDR_VERIFY_MODULE -Force -ErrorAction Stop
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$root = $env:CDR_FIXTURE_ROOT
$evidenceRoot = Join-Path $root 'docs\rust-migration\evidence'
$bundle = Get-CdrCheckpointEvidenceBundle `
    -RepoRoot $root -RepositoryEvidenceRoot $evidenceRoot `
    -OfflineSoakEvidencePath $env:CDR_SOAK_EVIDENCE `
    -WorkspaceGateEvidencePath $env:CDR_WORKSPACE_EVIDENCE `
    -ExpectedRuntimeSha256 $env:CDR_RUNTIME_SHA256
$null = New-Item -ItemType Directory -Path $env:CDR_STAGING_ROOT -Force
$package = New-CdrCheckpointPackage `
    -RepoRoot $root -StagingRoot $env:CDR_STAGING_ROOT `
    -ArchivePath $env:CDR_ORIGINAL_ARCHIVE `
    -DatabaseSnapshotPath $env:CDR_DATABASE_SNAPSHOT `
    -ExpectedRuntimeSha256 $env:CDR_RUNTIME_SHA256 `
    -ExpectedDatabaseSha256 $env:CDR_DATABASE_SHA256 `
    -OfflineSoakSnapshot $bundle.OfflineSoakSnapshot `
    -WorkspaceGateSnapshot $bundle.WorkspaceGateSnapshot `
    -SourceFingerprint $bundle.SourceFingerprint `
    -RollbackSource $bundle.RollbackSource
$utf8 = [Text.UTF8Encoding]::new($false, $true)

function Rewrite-ArchiveMetadata([string]$Archive, [string]$Kind) {
    $expanded = Join-Path $env:CDR_PROBE_ROOT "$Kind-expanded"
    $null = New-Item -ItemType Directory -Path $expanded
    Expand-Archive -LiteralPath $Archive -DestinationPath $expanded
    $metadataPath = Join-Path $expanded 'ARCHIVE-METADATA.json'
    $sumsPath = Join-Path $expanded 'SHA256SUMS'
    $metadata = [IO.File]::ReadAllText($metadataPath, $utf8) | ConvertFrom-Json
    switch ($Kind) {
        'coherent_payload' {
            $payloadPath = Join-Path $expanded 'workspace\Cargo.toml'
            [IO.File]::AppendAllText($payloadPath, "# coherent tamper`n", $utf8)
            $hash = Get-CdrSha256 $payloadPath
            $bytes = [long](Get-Item -LiteralPath $payloadPath).Length
            $rows = @($metadata.payload_files | Where-Object { $_.path -ceq 'workspace/Cargo.toml' })
            if ($rows.Count -ne 1) { throw 'Cargo payload row missing from adversarial fixture' }
            $rows[0].sha256 = $hash; $rows[0].bytes = $bytes
            $lines = [IO.File]::ReadAllLines($sumsPath, $utf8)
            for ($index = 0; $index -lt $lines.Count; $index++) {
                if ($lines[$index].EndsWith('  workspace/Cargo.toml', [StringComparison]::Ordinal)) {
                    $lines[$index] = "$hash  workspace/Cargo.toml"
                }
            }
            [IO.File]::WriteAllLines($sumsPath, $lines, $utf8)
        }
        'metadata_schema' { $metadata.schema_version = [string]'1' }
        'metadata_bool' { $metadata.bot_disabled = [string]'True' }
        'metadata_artifact' { $metadata.current_artifact_hashes.'cdr-runtime.exe'.sha256 = '0' * 64 }
        'metadata_evidence' { $metadata.offline_soak_evidence.sha256 = '0' * 64 }
        'control_unbound_field' { $metadata.created_at_utc = '2099-01-01T00:00:00.0000000Z' }
        'metadata_source_count' {
            $metadata.source_fingerprint.file_count = [int64]$metadata.source_fingerprint.file_count + 1
        }
        'metadata_rollback_count' {
            $metadata.source_rollback.source_file_count = [int64]$metadata.source_rollback.source_file_count + 1
        }
        default { throw "Unknown metadata rewrite: $Kind" }
    }
    [IO.File]::WriteAllText($metadataPath, ($metadata | ConvertTo-Json -Depth 30), $utf8)
    [IO.File]::Delete($Archive)
    [IO.Compression.ZipFile]::CreateFromDirectory(
        $expanded, $Archive, [IO.Compression.CompressionLevel]::Optimal, $false
    )
}

function Add-DirectoryEntry([string]$Archive, [string]$Name) {
    $zip = [IO.Compression.ZipFile]::Open($Archive, [IO.Compression.ZipArchiveMode]::Update)
    try { $null = $zip.CreateEntry($Name) } finally { $zip.Dispose() }
}

function Assert-VerifierRejects([string]$Kind, [string]$Expected) {
    $archive = Join-Path $root ".codex-discord-backups\adversarial-$Kind.zip"
    Copy-Item -LiteralPath $env:CDR_ORIGINAL_ARCHIVE -Destination $archive
    switch ($Kind) {
        'directory_traversal' { Add-DirectoryEntry $archive '../escape/' }
        'directory_forbidden' { Add-DirectoryEntry $archive '.env/' }
        'directory_plain' { Add-DirectoryEntry $archive 'untrusted-empty/' }
        default { Rewrite-ArchiveMetadata $archive $Kind }
    }
    $verifyRoot = Join-Path $env:CDR_PROBE_ROOT "$Kind-verify"
    $null = New-Item -ItemType Directory -Path $verifyRoot
    $before = @(Get-CdrForbiddenArtifactProcessSnapshot)
    $marker = Assert-DisabledMarker $env:CDR_DISABLE_PATH $null
    $stream = [IO.File]::Open(
        $archive, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read
    )
    $rejection = $null
    try {
        try {
            $null = Test-CdrCheckpointArchive `
                -ArchivePath $archive -ArchiveStream $stream -VerificationRoot $verifyRoot `
                -PayloadFiles $package.PayloadFiles -ControlFiles $package.ControlFiles `
                -ExpectedMetadata $package.Metadata -OfflineSmokeDurationSeconds 1 `
                -ForbiddenProcessesBefore $before -DisablePath $env:CDR_DISABLE_PATH `
                -MarkerBefore $marker
        } catch { $rejection = $_.Exception.Message }
    } finally { $stream.Dispose() }
    if ($null -eq $rejection) { throw "Verifier accepted adversarial archive: $Kind" }
    if (-not $rejection.Contains($Expected)) {
        throw "Unexpected rejection for $Kind`: $rejection"
    }
}

$cases = [ordered]@{
    coherent_payload = 'Archive metadata differs from trusted staged payload: workspace/Cargo.toml'
    metadata_schema = 'Checkpoint metadata identity contract failed'
    metadata_bool = 'Checkpoint metadata bot_disabled must be scalar true'
    metadata_artifact = 'Checkpoint metadata artifact mismatch: cdr-runtime.exe'
    metadata_evidence = 'Checkpoint metadata evidence mismatch: offline_soak_evidence'
    control_unbound_field = 'Extracted checkpoint control differs from trusted staged control: ARCHIVE-METADATA.json'
    metadata_source_count = 'Checkpoint metadata source_fingerprint.file_count mismatch'
    metadata_rollback_count = 'Checkpoint metadata rollback contract mismatch'
    directory_traversal = 'unsafe or non-canonical'
    directory_forbidden = 'Forbidden environment file'
    directory_plain = 'Checkpoint ZIP directory entries are not allowed'
}
foreach ($entry in $cases.GetEnumerator()) {
    Assert-VerifierRejects ([string]$entry.Key) ([string]$entry.Value)
}
Write-Output "passed=$($cases.Count)"
"##,
    )
    .expect("write archive adversarial probe");
    Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&probe)
        .env(
            "CDR_COMMON_MODULE",
            repo_root().join("scripts/RustMigrationCheckpoint.Common.psm1"),
        )
        .env(
            "CDR_EVIDENCE_MODULE",
            repo_root().join("scripts/RustMigrationCheckpoint.Evidence.psm1"),
        )
        .env(
            "CDR_PACKAGE_MODULE",
            repo_root().join("scripts/RustMigrationCheckpoint.Package.psm1"),
        )
        .env(
            "CDR_VERIFY_MODULE",
            repo_root().join("scripts/RustMigrationCheckpoint.Verify.psm1"),
        )
        .env("CDR_FIXTURE_ROOT", &fixture.root)
        .env("CDR_PROBE_ROOT", probe_root)
        .env("CDR_STAGING_ROOT", &staging)
        .env("CDR_ORIGINAL_ARCHIVE", &archive)
        .env("CDR_SOAK_EVIDENCE", &fixture.soak_evidence)
        .env("CDR_WORKSPACE_EVIDENCE", &fixture.workspace_evidence)
        .env("CDR_DATABASE_SNAPSHOT", &fixture.snapshot)
        .env("CDR_DISABLE_PATH", &fixture.marker)
        .env("CDR_RUNTIME_SHA256", sha256(&fixture.runtime))
        .env("CDR_DATABASE_SHA256", sha256(&fixture.snapshot))
        .output()
        .expect("run archive verifier adversarial probe")
}

pub fn run_external_output_override_probe(
    fixture: &Fixture,
    output: &Path,
    evidence: &Path,
) -> Output {
    let _process_guard = checkpoint_process_test_lock();
    Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(repo_root().join("scripts/New-RustMigrationCheckpoint.ps1"))
        .arg("-RepoRoot")
        .arg(&fixture.root)
        .arg("-OutputDirectory")
        .arg(output)
        .arg("-EvidenceOutputDirectory")
        .arg(evidence)
        .arg("-AllowExternalOutput")
        .arg("-DatabaseSnapshotPath")
        .arg(&fixture.snapshot)
        .arg("-ExpectedRuntimeSha256")
        .arg("0000000000000000000000000000000000000000000000000000000000000000")
        .arg("-ExpectedDatabaseSha256")
        .arg(sha256(&fixture.snapshot))
        .arg("-OfflineSoakEvidencePath")
        .arg(&fixture.soak_evidence)
        .arg("-WorkspaceGateEvidencePath")
        .arg(&fixture.workspace_evidence)
        .output()
        .expect("run external output override probe")
}

fn run_checkpoint_with_stamp(fixture: &Fixture, stamp: &str) -> Output {
    Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(repo_root().join("scripts/New-RustMigrationCheckpoint.ps1"))
        .arg("-RepoRoot")
        .arg(&fixture.root)
        .arg("-DatabaseSnapshotPath")
        .arg(&fixture.snapshot)
        .arg("-ExpectedRuntimeSha256")
        .arg(sha256(&fixture.runtime))
        .arg("-ExpectedDatabaseSha256")
        .arg(sha256(&fixture.snapshot))
        .arg("-OfflineSoakEvidencePath")
        .arg(&fixture.soak_evidence)
        .arg("-WorkspaceGateEvidencePath")
        .arg(&fixture.workspace_evidence)
        .arg("-CheckpointStamp")
        .arg(stamp)
        .output()
        .expect("run checkpoint creator with fixed stamp")
}

pub fn assert_preexisting_checkpoint_targets_survive(fixture: &Fixture) {
    let _process_guard = checkpoint_process_test_lock();
    let stamp = "20991231T235959999Z";
    let archive = fixture.root.join(format!(
        ".codex-discord-backups/cdr-rust-local-release-checkpoint-{stamp}.zip"
    ));
    let evidence = fixture.root.join(format!(
        "docs/rust-migration/evidence/local-release-checkpoint-{stamp}.json"
    ));
    let archive_bytes = b"preexisting archive bytes\n";
    let evidence_bytes = b"preexisting evidence bytes\n";
    fs::write(&archive, archive_bytes).unwrap();
    fs::write(&evidence, evidence_bytes).unwrap();
    let hashes = (sha256(&archive), sha256(&evidence));
    let output = run_checkpoint_with_stamp(fixture, stamp);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Checkpoint archive already exists"));
    assert_eq!(fs::read(&archive).unwrap(), archive_bytes);
    assert_eq!(fs::read(&evidence).unwrap(), evidence_bytes);
    assert_eq!((sha256(&archive), sha256(&evidence)), hashes);
    fs::remove_file(&archive).unwrap();
    let output = run_checkpoint_with_stamp(fixture, stamp);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Checkpoint evidence already exists"));
    assert!(!archive.exists());
    assert_eq!(fs::read(&evidence).unwrap(), evidence_bytes);
    assert_eq!(sha256(&evidence), hashes.1);
}

pub fn run_process_enumeration_failure_probe() -> Output {
    let common = repo_root().join("scripts/RustMigrationCheckpoint.Common.psm1");
    let command = format!(
        "Import-Module '{}'; Get-CdrForbiddenProcessCandidates 'cdr-runtime' {{ throw 'enumeration denied fixture' }}",
        common.display().to_string().replace('\'', "''")
    );
    Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &command])
        .output()
        .expect("run fail-closed process enumeration probe")
}

pub fn run_evidence_bundle_in_shell(fixture: &Fixture, shell: &str) -> Output {
    let command = r"
$ErrorActionPreference = 'Stop'
Import-Module $env:CDR_EVIDENCE_MODULE -Force -ErrorAction Stop
$null = Get-CdrCheckpointEvidenceBundle `
    -RepoRoot $env:CDR_FIXTURE_ROOT `
    -RepositoryEvidenceRoot $env:CDR_EVIDENCE_ROOT `
    -OfflineSoakEvidencePath $env:CDR_SOAK_EVIDENCE `
    -WorkspaceGateEvidencePath $env:CDR_WORKSPACE_EVIDENCE `
    -ExpectedRuntimeSha256 $env:CDR_RUNTIME_SHA256
";
    Command::new(shell)
        .args(["-NoProfile", "-Command", command])
        .env(
            "CDR_EVIDENCE_MODULE",
            repo_root().join("scripts/RustMigrationCheckpoint.Evidence.psm1"),
        )
        .env("CDR_FIXTURE_ROOT", &fixture.root)
        .env(
            "CDR_EVIDENCE_ROOT",
            fixture.root.join("docs/rust-migration/evidence"),
        )
        .env("CDR_SOAK_EVIDENCE", &fixture.soak_evidence)
        .env("CDR_WORKSPACE_EVIDENCE", &fixture.workspace_evidence)
        .env("CDR_RUNTIME_SHA256", sha256(&fixture.runtime))
        .output()
        .unwrap_or_else(|error| panic!("run evidence bundle in {shell}: {error}"))
}

pub fn run_evidence_contract_in_shell(path: &Path, kind: &str, shell: &str) -> Output {
    let command = r"
$ErrorActionPreference = 'Stop'
Import-Module $env:CDR_EVIDENCE_CONTRACT_MODULE -Force -ErrorAction Stop
$utf8 = [Text.UTF8Encoding]::new($false, $true)
$record = [IO.File]::ReadAllText($env:CDR_EVIDENCE_PATH, $utf8) | ConvertFrom-Json
if ($env:CDR_EVIDENCE_KIND -ceq 'soak') {
    Assert-CdrOfflineSoakEvidenceContract $record
} else {
    Assert-CdrWorkspaceGateEvidenceContract $record
}
";
    Command::new(shell)
        .args(["-NoProfile", "-Command", command])
        .env(
            "CDR_EVIDENCE_CONTRACT_MODULE",
            repo_root().join("scripts/RustMigrationCheckpoint.EvidenceContract.psm1"),
        )
        .env("CDR_EVIDENCE_PATH", path)
        .env("CDR_EVIDENCE_KIND", kind)
        .output()
        .unwrap_or_else(|error| panic!("run {kind} evidence contract in {shell}: {error}"))
}

pub fn run_archive_publish_failure_probe(root: &Path) -> Output {
    let stage = root.join("stage");
    fs::create_dir_all(&stage).unwrap();
    fs::write(stage.join("00-readable.txt"), b"readable\n").unwrap();
    fs::write(stage.join("99-locked.txt"), b"locked\n").unwrap();
    let command = r"
$ErrorActionPreference = 'Stop'
Import-Module $env:CDR_ARCHIVE_PUBLISH_MODULE -Force -ErrorAction Stop
$lock = [IO.File]::Open(
    $env:CDR_LOCKED_SOURCE, [IO.FileMode]::Open,
    [IO.FileAccess]::Read, [IO.FileShare]::None
)
$rejection = $null
try {
    try {
        $null = Publish-CdrCheckpointArchive `
            -StagingRoot $env:CDR_STAGING_ROOT -ArchivePath $env:CDR_ARCHIVE_PATH
    } catch { $rejection = $_.Exception.Message }
} finally { $lock.Dispose() }
if ($null -eq $rejection) { throw 'Locked staging source was unexpectedly archived' }
if ([IO.File]::Exists($env:CDR_ARCHIVE_PATH)) { throw 'Partial final archive survived failure' }
$parent = [IO.Path]::GetDirectoryName($env:CDR_ARCHIVE_PATH)
$partials = @(Get-ChildItem -LiteralPath $parent -Filter '.cdr-checkpoint-*.tmp' -File)
if ($partials.Count -ne 0) { throw 'Temporary partial archive survived failure' }
Write-Output 'publish_failure_clean'
";
    Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", command])
        .env(
            "CDR_ARCHIVE_PUBLISH_MODULE",
            repo_root().join("scripts/RustMigrationCheckpoint.ArchivePublish.psm1"),
        )
        .env("CDR_STAGING_ROOT", &stage)
        .env("CDR_LOCKED_SOURCE", stage.join("99-locked.txt"))
        .env("CDR_ARCHIVE_PATH", root.join("final-checkpoint.zip"))
        .output()
        .expect("run archive publication failure probe")
}

#[allow(clippy::too_many_lines)] // One cohesive embedded PowerShell process-safety fixture.
pub fn run_injected_bot_off_guard_probe(root: &Path, shell: &str) -> Output {
    // The legacy writer guard must still work after the real Python source is removed.
    // Short-name lookup uses a non-executable file only inside this isolated fixture.
    let legacy_root = root.join("legacy-writer-probe");
    fs::create_dir_all(&legacy_root).unwrap();
    fs::write(
        legacy_root.join("codex_discord_bot.py"),
        b"read-only identity fixture",
    )
    .unwrap();
    let command = r#"
$ErrorActionPreference = 'Stop'
Import-Module $env:CDR_PROCESS_SAFETY_MODULE -Force -ErrorAction Stop
function Assert-Rejected([object[]]$Snapshot, [string]$Label) {
    $accepted = $false
    try { Assert-CdrCheckpointBotOff $Snapshot $Label; $accepted = $true } catch {}
    if ($accepted) { throw "Bot-off guard accepted $Label" }
}
$noRows = { @() }
$knownScript = Join-Path $env:CDR_REAL_REPO_ROOT 'codex_discord_bot.py'
$actualShortPath = Get-CdrWindowsShortPath $knownScript
$native = Get-CdrCheckpointForbiddenProcessSnapshot `
    -RepoRoot $env:CDR_REAL_REPO_ROOT `
    -NativeSnapshotQuery { @('cdr-runtime|42|1') } `
    -ProcessRowsQuery $noRows
Assert-Rejected $native 'injected_native'
$positiveCases = @(
    [pscustomobject]@{ Name = 'python.exe'; CommandLine = 'python.exe codex_discord_bot.py' },
    [pscustomobject]@{ Name = 'python.exe'; CommandLine = 'python.exe ".\codex_discord_bot.py"' },
    [pscustomobject]@{ Name = 'python.exe'; CommandLine = 'python.exe ./codex_discord_bot.py' },
    [pscustomobject]@{
        Name = 'python.exe'
        CommandLine = ('python.exe "' + $knownScript + '"')
    },
    [pscustomobject]@{ Name = 'py.exe'; CommandLine = 'py.exe -3 -m codex_discord_bot' },
    [pscustomobject]@{ Name = 'pythonw.exe'; CommandLine = 'pythonw.exe -m "codex_discord_bot"' },
    [pscustomobject]@{ Name = 'python.exe'; CommandLine = 'python.exe -mcodex_discord_bot' },
    [pscustomobject]@{ Name = 'py.exe'; CommandLine = 'py.exe -3 -mcodex_discord_bot' },
    [pscustomobject]@{ Name = 'python.exe'; CommandLine = 'python.exe C:codex_discord_bot.py' },
    [pscustomobject]@{ Name = 'python3.exe'; CommandLine = 'python3.exe codex_discord_bot.py' },
    [pscustomobject]@{ Name = 'python3.12.exe'; CommandLine = 'python3.12.exe -m codex_discord_bot' },
    [pscustomobject]@{ Name = 'pyw.exe'; CommandLine = 'pyw.exe -m codex_discord_bot' },
    [pscustomobject]@{ Name = 'python.exe'; CommandLine = 'python.exe -Im codex_discord_bot' },
    [pscustomobject]@{ Name = 'python.exe'; CommandLine = 'python.exe -Imcodex_discord_bot' },
    [pscustomobject]@{ Name = 'python.exe'; CommandLine = 'python.exe codex_discord_bot.py.' },
    [pscustomobject]@{ Name = 'python.exe'; CommandLine = 'python.exe .\codex_discord_bot.py...' },
    [pscustomobject]@{ Name = 'python.exe'; CommandLine = 'python.exe "codex_discord_bot.py "' }
)
$rejections = 1
for ($index = 0; $index -lt $positiveCases.Count; $index++) {
    $case = $positiveCases[$index]
    $row = [pscustomobject]@{
        Name = $case.Name; ProcessId = [uint32](77 + $index)
        CommandLine = $case.CommandLine
        CreationDate = [datetime]'2026-09-03T00:00:00Z'
    }
    $pythonRows = { @($row) }.GetNewClosure()
    $python = Get-CdrCheckpointForbiddenProcessSnapshot `
        -RepoRoot $env:CDR_REAL_REPO_ROOT -NativeSnapshotQuery { @() } `
        -ProcessRowsQuery $pythonRows
    Assert-Rejected $python "injected_python_bot_$index"
    $rejections++
}
$fakeShortPath = 'C:\FAKE\COE85F~1.PY'
$fakeShortRow = [pscustomobject]@{
    Name = 'python.exe'; ProcessId = [uint32]150
    CommandLine = 'python.exe COE85F~1.PY'
    CreationDate = [datetime]'2026-09-03T00:00:00Z'
}
$fakeShortRows = { @($fakeShortRow) }.GetNewClosure()
$fakeShortQuery = { param($Path) $fakeShortPath }.GetNewClosure()
$fakeShort = Get-CdrCheckpointForbiddenProcessSnapshot `
    -RepoRoot $env:CDR_REAL_REPO_ROOT -NativeSnapshotQuery { @() } `
    -ProcessRowsQuery $fakeShortRows -ShortPathQuery $fakeShortQuery
Assert-Rejected $fakeShort 'injected_short_alias'
$rejections++
$actualShortCases = @()
if (-not $actualShortPath.Equals($knownScript, [StringComparison]::OrdinalIgnoreCase)) {
    $actualShortCases = @(
        ('python.exe "' + $actualShortPath + '"'),
        ('python.exe ' + [IO.Path]::GetFileName($actualShortPath))
    )
}
for ($index = 0; $index -lt $actualShortCases.Count; $index++) {
    $row = [pscustomobject]@{
        Name = 'python.exe'; ProcessId = [uint32](160 + $index)
        CommandLine = $actualShortCases[$index]
        CreationDate = [datetime]'2026-09-03T00:00:00Z'
    }
    $actualShortRows = { @($row) }.GetNewClosure()
    $snapshot = Get-CdrCheckpointForbiddenProcessSnapshot `
        -RepoRoot $env:CDR_REAL_REPO_ROOT -NativeSnapshotQuery { @() } `
        -ProcessRowsQuery $actualShortRows
    Assert-Rejected $snapshot "actual_short_alias_$index"
}
$shortPathFailureClosed = $false
try {
    $null = Get-CdrCheckpointForbiddenProcessSnapshot `
        -RepoRoot $env:CDR_REAL_REPO_ROOT -NativeSnapshotQuery { @() } `
        -ProcessRowsQuery $noRows -ShortPathQuery { throw 'alias lookup denied fixture' }
} catch {
    if (-not $_.Exception.Message.Contains('Cannot verify legacy Python bot short path')) { throw }
    $shortPathFailureClosed = $true
}
if (-not $shortPathFailureClosed) { throw 'Short-path lookup failure was accepted' }
$negativeCommands = @(
    'python.exe codex_discord_bot.py.backup',
    'python.exe not_codex_discord_bot.py',
    'python.exe .\not_codex_discord_bot.py',
    'python.exe ".\codex_discord_bot.py.backup"',
    'python.exe -m codex_discord_bot.extra',
    'python.exe --label=codex_discord_bot.py',
    'python.exe -mcodex_discord_bot.extra',
    'python.exe -mcodex_discord_bot_backup',
    'python.exe C:codex_discord_bot.py.backup',
    'python.exe -Im codex_discord_bot.extra',
    'python.exe -Imcodex_discord_bot_backup',
    'python.exe codex_discord_bot.py.backup.',
    'python.exe not_codex_discord_bot.py...'
)
$negativeRows = @(
    for ($index = 0; $index -lt $negativeCommands.Count; $index++) {
        [pscustomobject]@{
            Name = 'python.exe'; ProcessId = [uint32](177 + $index)
            CommandLine = $negativeCommands[$index]
            CreationDate = [datetime]'2026-09-03T00:00:00Z'
        }
    }
)
$negativeRowsQuery = { @($negativeRows) }.GetNewClosure()
$negative = Get-CdrCheckpointForbiddenProcessSnapshot `
    -RepoRoot $env:CDR_REAL_REPO_ROOT -NativeSnapshotQuery { @() } `
    -ProcessRowsQuery $negativeRowsQuery
Assert-CdrCheckpointBotOff $negative 'token_boundary_negatives'
Write-Output (
    "bot_off_rejections=$rejections " +
    "actual_short_alias_cases=$($actualShortCases.Count) " +
    "bot_off_negatives=$($negativeCommands.Count) short_path_failure_closed=true"
)
"#;
    Command::new(shell)
        .args(["-NoProfile", "-Command", command])
        .env(
            "CDR_PROCESS_SAFETY_MODULE",
            repo_root().join("scripts/RustMigrationCheckpoint.ProcessSafety.psm1"),
        )
        .env("CDR_PROBE_ROOT", root)
        .env("CDR_REAL_REPO_ROOT", legacy_root)
        .output()
        .unwrap_or_else(|error| panic!("run injected bot-off guard probe in {shell}: {error}"))
}

pub fn checkpoint_tools() -> [&'static str; 22] {
    [
        "New-RustMigrationCheckpoint.ps1",
        "RustMigrationCheckpoint.Common.psm1",
        "RustMigrationCheckpoint.Package.psm1",
        "RustMigrationCheckpoint.Payload.psm1",
        "RustMigrationCheckpoint.Verify.psm1",
        "RustMigrationCheckpoint.RollbackVerify.psm1",
        "RustMigrationCheckpoint.Evidence.psm1",
        "RustMigrationCheckpoint.EvidenceContract.psm1",
        "RustMigrationCheckpoint.SourceBinding.psm1",
        "RustMigrationCheckpoint.Stage.psm1",
        "RustMigrationCheckpoint.ArchiveContract.psm1",
        "RustMigrationCheckpoint.ArchivePublish.psm1",
        "RustMigrationCheckpoint.EvidenceShape.psm1",
        "RustMigrationCheckpoint.NativeEvidence.psm1",
        "RustMigrationCheckpoint.Quality.psm1",
        "RustMigrationCheckpoint.ProcessSafety.psm1",
        "CodexDiscordSoak.SourceFingerprint.psm1",
        "CodexDiscordSoak.Evidence.psm1",
        "CodexDiscordSoak.ProcessOwnership.psm1",
        "CodexDiscordSoak.Provenance.psm1",
        "CodexDiscordSoak.WrapperEvidence.psm1",
        "CodexDiscordSoak.WrapperRuntime.psm1",
    ]
}

pub fn assert_no_generated_checkpoint(fixture: &Fixture) {
    let archives = fs::read_dir(fixture.root.join(".codex-discord-backups")).unwrap();
    assert!(!archives.filter_map(Result::ok).any(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .starts_with("cdr-rust-local-release-checkpoint-")
    }));
    let evidence = fs::read_dir(fixture.root.join("docs/rust-migration/evidence")).unwrap();
    assert!(!evidence.filter_map(Result::ok).any(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .starts_with("local-release-checkpoint-")
    }));
    assert_eq!(fs::read(&fixture.marker).unwrap(), MARKER_BYTES);
}

fn write_named_files(root: &Path, paths: &[&str]) {
    for path in paths {
        fs::write(root.join(path), format!("fixture={path}\n")).expect("write fixture file");
    }
}
