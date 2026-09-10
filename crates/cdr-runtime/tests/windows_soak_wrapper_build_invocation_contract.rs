#![cfg(windows)]

#[path = "support/windows_soak_wrapper.rs"]
mod wrapper;

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

fn fake_cargo_without_exit(root: &Path) -> PathBuf {
    let path = root.join("fake-cargo-without-exit.ps1");
    fs::write(
        &path,
        b"param([Parameter(ValueFromRemainingArguments=$true)][string[]]$Rest)\n$null = $Rest\nreturn\n",
    )
    .unwrap();
    path
}

fn outer_runner(root: &Path) -> PathBuf {
    let path = root.join("invoke-wrapper-in-current-host.ps1");
    fs::write(
        &path,
        br"param(
    [string]$Wrapper,
    [string]$RepoRoot,
    [string]$OutputDirectory,
    [string]$HarnessPath,
    [string]$ExpectedHarnessSha256,
    [string]$CargoPath,
    [int]$StaleExitCode
)
$ErrorActionPreference = 'Stop'
$global:LASTEXITCODE = $StaleExitCode
& $Wrapper `
    -RepoRoot $RepoRoot `
    -OutputDirectory $OutputDirectory `
    -HarnessPath $HarnessPath `
    -ExpectedHarnessSha256 $ExpectedHarnessSha256 `
    -CargoPath $CargoPath `
    -DurationSeconds 1 `
    -SampleIntervalSeconds 0.1 `
    -WarmupSeconds 0 `
    -HarnessExitGraceSeconds 5 `
    -MaxSlopeBytesPerHour 1000000000000000
",
    )
    .unwrap();
    path
}

fn run_in_same_host(fixture: &wrapper::Fixture, cargo: &Path, stale_exit_code: i32) -> Output {
    let runner = outer_runner(&fixture.root);
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(runner)
        .arg("-Wrapper")
        .arg(&fixture.script)
        .arg("-RepoRoot")
        .arg(&fixture.root)
        .arg("-OutputDirectory")
        .arg(&fixture.evidence)
        .arg("-HarnessPath")
        .arg(&fixture.release_harness)
        .arg("-ExpectedHarnessSha256")
        .arg(wrapper::sha256(&fixture.release_harness))
        .arg("-CargoPath")
        .arg(cargo)
        .arg("-StaleExitCode")
        .arg(stale_exit_code.to_string())
        .env("CARGO_TARGET_DIR", &fixture.target)
        .env_remove("DISCORD_BOT_TOKEN")
        .env_remove("DISCORD_TOKEN")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    wrapper::wait_for_output(command.spawn().unwrap(), Duration::from_secs(20))
}

fn assert_build_failure(output: &Output, summary: &Value) -> String {
    assert!(
        !output.status.success(),
        "wrapper unexpectedly passed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(summary["schema"], "cdr.windows-soak.summary.v3");
    assert_eq!(summary["operational_status"], "runtime_failed");

    let provenance = &summary["source_provenance"];
    let primary = &provenance["primary_failure"];
    assert_eq!(primary["stage"], "build");
    assert_eq!(primary["code"], "build_failed");
    primary["message"]
        .as_str()
        .filter(|message| !message.is_empty())
        .expect("nonempty build failure message")
        .to_owned()
}

fn assert_post_build_boundary(summary: &Value) {
    let provenance = &summary["source_provenance"];
    assert_eq!(
        provenance["build_after"]["schema"],
        "cdr.rust-source-fingerprint.v1"
    );
    assert!(
        provenance["build_after"]["aggregate_sha256"]
            .as_str()
            .is_some_and(|hash| !hash.is_empty())
    );
    assert_eq!(provenance["build_fingerprints_equal"], true);
    assert!(provenance["soak_after"].is_null());
    assert!(provenance["soak_fingerprints_equal"].is_null());
    assert!(summary["harness_pid"].is_null());
    assert!(summary["harness_provenance"]["pid"].is_null());
    assert!(summary["child_exit_code"].is_null());
}

fn assert_missing_exit_message(message: &str) {
    assert!(
        message
            .to_ascii_lowercase()
            .contains("no exit code was reported"),
        "build failure must identify the missing exit code, got: {message}"
    );
}

#[test]
fn fake_cargo_returning_normally_does_not_reuse_stale_zero_exit_code() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    let cargo = fake_cargo_without_exit(&fixture.root);
    let output = run_in_same_host(&fixture, &cargo, 0);
    let summary = wrapper::summary(&fixture);

    let message = assert_build_failure(&output, &summary);
    assert_missing_exit_message(&message);
    assert_post_build_boundary(&summary);
    wrapper::assert_marker(&fixture);
}

#[test]
fn fake_cargo_returning_normally_does_not_reuse_stale_nonzero_exit_code() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    let cargo = fake_cargo_without_exit(&fixture.root);
    let output = run_in_same_host(&fixture, &cargo, 37);
    let summary = wrapper::summary(&fixture);

    let message = assert_build_failure(&output, &summary);
    assert_missing_exit_message(&message);
    assert!(!message.contains("37"), "stale exit code leaked: {message}");
    assert_post_build_boundary(&summary);
    wrapper::assert_marker(&fixture);
}

#[test]
fn cargo_invocation_error_is_preserved_instead_of_becoming_exit_minus_one() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    let cargo = fixture.root.join("missing-cargo-command.ps1");
    let output = run_in_same_host(&fixture, &cargo, 0);
    let summary = wrapper::summary(&fixture);

    let message = assert_build_failure(&output, &summary);
    assert!(
        message.contains("missing-cargo-command.ps1"),
        "invocation error lost the failing command path: {message}"
    );
    assert!(
        !message.contains("exit -1"),
        "synthetic exit replaced error: {message}"
    );
    assert_post_build_boundary(&summary);
    wrapper::assert_marker(&fixture);
}
