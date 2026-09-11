#![cfg(windows)]
//! Port of TRAY-1..4 and exit-code contracts, in an isolated fake process inventory.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in [
        "codex-discord-tray.ps1",
        "codex-discord-tray-runtime.ps1",
        "codex-discord-tray-restart-runtime.ps1",
    ] {
        fs::copy(repo.join(name), temp.path().join(name)).unwrap();
    }
    fs::write(
        temp.path().join(".codex_discord_rust.runtime.lock"),
        "pid=42\n",
    )
    .unwrap();
    temp
}

fn run(root: &Path, body: &str, mode: &str, mismatch: bool) -> Output {
    let prelude = r"
$ErrorActionPreference='Stop'
function Get-Process {
  param($Id)
  if ($Id -eq 42) {
    $exe=Join-Path $env:TRAY_CONTRACT_ROOT 'target\release\cdr-runtime.exe'
    if ($env:TRAY_PATH_MISMATCH -eq 'true') { $exe=Join-Path $env:TRAY_CONTRACT_ROOT 'other\cdr-runtime.exe' }
    [pscustomobject]@{Id=42;Path=$exe;StartTime=[datetime]'2026-09-01T00:00:00Z'}
  }
}
function Get-CimInstance { throw 'LEGACY_SCAN' }
";
    Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &format!("{prelude}\n{body}")])
        .env("TRAY_CONTRACT_ROOT", root)
        .env("TRAY_PATH_MISMATCH", mismatch.to_string())
        .env("CODEX_DISCORD_RUNTIME", mode)
        .output()
        .unwrap()
}

const ONCE: &str = "& (Join-Path $env:TRAY_CONTRACT_ROOT 'codex-discord-tray.ps1') -Once";
const RESTART: &str = r"
$ScriptDir=$env:TRAY_CONTRACT_ROOT
$RuntimeMode='rust'
. (Join-Path $ScriptDir 'codex-discord-tray-runtime.ps1')
. (Join-Path $ScriptDir 'codex-discord-tray-restart-runtime.ps1')
function Publish-AtomicTextFile { throw 'LEGACY_MARKER' }
function Write-LauncherLog { param($Message); Write-Output $Message }
Request-BotRestart
";

#[test]
fn default_tray_recognizes_only_the_bound_rust_process() {
    let root = fixture();
    let output = run(root.path(), ONCE, "", false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("running pid=42"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("LEGACY_SCAN"));
}

#[test]
fn mismatched_pid_path_and_missing_lock_are_not_this_bot() {
    let root = fixture();
    for missing in [false, true] {
        if missing {
            fs::remove_file(root.path().join(".codex_discord_rust.runtime.lock")).unwrap();
        }
        let output = run(root.path(), ONCE, "rust", true);
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stdout).contains("stopped"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("LEGACY_SCAN"));
    }
}

#[test]
fn saved_mode_is_honored_and_invalid_or_removed_modes_have_no_fallback() {
    let root = fixture();
    fs::write(root.path().join(".codex_discord_runtime"), "rust\n").unwrap();
    assert!(run(root.path(), ONCE, "", false).status.success());
    for mode in ["typo", "python"] {
        let output = run(root.path(), ONCE, mode, false);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("Unsupported Codex Discord runtime selection"),
            "{error}"
        );
        assert!(!error.contains("LEGACY_SCAN"));
    }
}

#[test]
fn restart_delegates_exact_identity_and_ignores_unrelated_prior_exit_codes() {
    let root = fixture();
    fs::write(root.path().join("codex-discord-rust-restart.ps1"), "param([string]$RepoRoot,[string]$ExpectedBotIdentity)\nWrite-Output \"RUST_RESTART expected=$ExpectedBotIdentity\"\n").unwrap();
    for prior in ["$null", "9"] {
        let output = run(
            root.path(),
            &format!("$global:LASTEXITCODE={prior};\n{RESTART}"),
            "",
            false,
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("RUST_RESTART expected=42|"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("LEGACY_MARKER"));
    }
}

#[test]
fn failed_exit_or_readiness_error_is_not_reported_as_restart_queued() {
    let root = fixture();
    for (body, expected) in [
        ("exit 7", "Rust restart request failed with exit code 7"),
        (
            "throw 'app-server readiness: controlled failure'",
            "app-server readiness: controlled failure",
        ),
    ] {
        fs::write(
            root.path().join("codex-discord-rust-restart.ps1"),
            format!("param([string]$RepoRoot,[string]$ExpectedBotIdentity)\n{body}\n"),
        )
        .unwrap();
        let output = run(root.path(), RESTART, "", false);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("LEGACY_MARKER"));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("tray_restart_requested"));
    }
}
