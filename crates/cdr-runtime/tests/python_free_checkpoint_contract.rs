#![cfg(windows)]
use std::{path::Path, process::Command};

#[test]
fn checkpoint_source_inventory_contains_no_executable_python_or_requirements() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output=Command::new("powershell.exe").args(["-NoProfile","-Command",
        "Import-Module $env:CDR_PAYLOAD_MODULE; @(Get-CdrCheckpointRollbackSourceEntries -RepoRoot $env:CDR_SOURCE_ROOT) | ForEach-Object { $_.source_path }"])
        .env("CDR_PAYLOAD_MODULE",root.join("scripts/RustMigrationCheckpoint.Payload.psm1"))
        .env("CDR_SOURCE_ROOT",root).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let paths = String::from_utf8(output.stdout).unwrap();
    assert!(paths.lines().count() > 10);
    for path in paths.lines() {
        assert!(
            !Path::new(path)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("py"))
                && !path.to_ascii_lowercase().starts_with("requirements.")
                && path != "runtime-release.json",
            "legacy runtime payload: {path}"
        );
    }
}

#[test]
fn checkpoint_process_scope_preserves_own_external_binary_but_allows_another_repository() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = tempfile::tempdir().unwrap();
    let output=Command::new("powershell.exe").args(["-NoProfile","-Command",r"
$ErrorActionPreference='Stop'
Import-Module $env:CDR_PROCESS_MODULE
$row=[pscustomobject]@{Id=4242;Path=(Join-Path (Split-Path -Parent $env:CDR_ROOT) 'other-repo/cdr-runtime.exe');StartTime=[datetime]::UtcNow}
$query={param($name) if($name -eq 'cdr-runtime'){$row}}.GetNewClosure()
$snapshot=@(Get-CdrCheckpointNativeProcessSnapshot -RepoRoot $env:CDR_ROOT -ProcessQuery $query)
if($snapshot.Count -ne 0){throw 'another repository was treated as this bot'}
$lock=Join-Path $env:CDR_ROOT '.codex_discord_rust.runtime.lock'
[IO.File]::WriteAllText($lock,'pid=4242')
$snapshot=@(Get-CdrCheckpointNativeProcessSnapshot -RepoRoot $env:CDR_ROOT -ProcessQuery $query)
if($snapshot.Count -ne 1){throw 'own externally located runtime was ignored'}
[IO.File]::Delete($lock)
$row.Path=Join-Path $env:CDR_ROOT 'target/release/cdr-runtime.exe'
if(@(Get-CdrCheckpointNativeProcessSnapshot -RepoRoot $env:CDR_ROOT -ProcessQuery $query).Count -ne 1){throw 'own runtime was ignored'}
$row.Path=''
try {Get-CdrCheckpointNativeProcessSnapshot -RepoRoot $env:CDR_ROOT -ProcessQuery $query;throw 'unknown path accepted'}
catch {if($_.Exception.Message -notmatch 'Cannot verify executable path'){throw}}
exit 0
"])
        .env("CDR_PROCESS_MODULE",repo.join("scripts/RustMigrationCheckpoint.ProcessSafety.psm1"))
        .env("CDR_ROOT",root.path()).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
