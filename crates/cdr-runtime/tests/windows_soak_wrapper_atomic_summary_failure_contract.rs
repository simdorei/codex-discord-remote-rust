#![cfg(windows)]

#[path = "support/windows_soak_wrapper.rs"]
mod wrapper;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use serde_json::{Value, json};

const OLD_SUMMARY: &[u8] = br#"{"schema":"old.summary.v1","sentinel":"byte-exact"}"#;
const STAGE_PREFIX: &str = ".codex-soak-summary-publish-";
const UNRELATED_STAGE_SENTINEL: &[u8] = b"unrelated-stage-sentinel";

fn eventual_summary_path(fixture: &wrapper::Fixture) -> PathBuf {
    let memory = fs::read_dir(&fixture.evidence)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.to_string_lossy().ends_with(".memory.jsonl"))
        .expect("memory evidence path");
    let name = memory.file_name().unwrap().to_string_lossy();
    memory.with_file_name(format!(
        "{}.summary.json",
        name.trim_end_matches(".memory.jsonl")
    ))
}

fn owned_stages(fixture: &wrapper::Fixture) -> Vec<PathBuf> {
    fs::read_dir(&fixture.evidence)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name.starts_with(STAGE_PREFIX) && name.ends_with(".tmp")
        })
        .collect()
}

fn write_runner(fixture: &wrapper::Fixture) -> PathBuf {
    let path = fixture.root.join("invoke-wrapper.ps1");
    fs::write(
        &path,
        r"param([string]$WrapperPath,[string]$CaughtPath,[string]$RepoRoot,
    [string]$OutputDirectory,[string]$HarnessPath,[string]$ExpectedHarnessSha256,
    [long]$DurationSeconds)
$ErrorActionPreference='Stop'
$utf8=[Text.UTF8Encoding]::new($false,$true)
try {
    & $WrapperPath -RepoRoot $RepoRoot -OutputDirectory $OutputDirectory `
        -HarnessPath $HarnessPath -ExpectedHarnessSha256 $ExpectedHarnessSha256 `
        -DurationSeconds $DurationSeconds -SampleIntervalSeconds 0.1 -WarmupSeconds 0 `
        -HarnessExitGraceSeconds 5 -MaxSlopeBytesPerHour 1000000000000000 -SkipBuild
} catch {
    $caught=[ordered]@{
        message=$_.Exception.Message
        primary=$_.Exception.Data['CodexSoakPrimaryFailure']
        secondary=@($_.Exception.Data['CodexSoakSecondaryFailures'])
    }
    [IO.File]::WriteAllText($CaughtPath,($caught|ConvertTo-Json -Depth 20),$utf8)
    exit 71
}
",
    )
    .unwrap();
    path
}

fn runner_command(fixture: &wrapper::Fixture, duration: u64) -> Command {
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(write_runner(fixture))
        .arg("-WrapperPath")
        .arg(&fixture.script)
        .arg("-CaughtPath")
        .arg(fixture.root.join("caught.json"))
        .arg("-RepoRoot")
        .arg(&fixture.root)
        .arg("-OutputDirectory")
        .arg(&fixture.evidence)
        .arg("-HarnessPath")
        .arg(&fixture.debug_harness)
        .args([
            "-ExpectedHarnessSha256",
            &wrapper::sha256(&fixture.debug_harness),
        ])
        .args(["-DurationSeconds", &duration.to_string()])
        .env("CARGO_TARGET_DIR", &fixture.target)
        .env_remove("DISCORD_BOT_TOKEN")
        .env_remove("DISCORD_TOKEN")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn caught(fixture: &wrapper::Fixture) -> Value {
    serde_json::from_slice(&fs::read(fixture.root.join("caught.json")).unwrap()).unwrap()
}

fn assert_runner_failed(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(71),
        "runner failure mismatch\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("soak_passed"));
}

fn ps_literal(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

#[test]
fn w3_pub_02_unlocked_destination_collision_is_fail_closed_and_preserves_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    fs::create_dir_all(&fixture.evidence).unwrap();
    let sentinel = fixture
        .evidence
        .join(format!("{STAGE_PREFIX}unrelated.tmp"));
    fs::write(&sentinel, UNRELATED_STAGE_SENTINEL).unwrap();
    let baseline_stages = owned_stages(&fixture);
    let child = runner_command(&fixture, 3).spawn().unwrap();
    let (child, _) = wrapper::wait_for_memory(child, &fixture);
    let path = eventual_summary_path(&fixture);
    fs::write(&path, OLD_SUMMARY).unwrap();
    let output = wrapper::wait_for_output(child, Duration::from_secs(15));

    assert_runner_failed(&output);
    assert_eq!(fs::read(&path).unwrap(), OLD_SUMMARY);
    assert_eq!(owned_stages(&fixture), baseline_stages);
    assert_eq!(fs::read(&sentinel).unwrap(), UNRELATED_STAGE_SENTINEL);
    let failure = caught(&fixture);
    assert_eq!(failure["primary"]["stage"], "write");
    assert_eq!(failure["primary"]["code"], "write_failed");
    assert_eq!(failure["secondary"], json!([]));
    assert!(
        failure["primary"]["message"]
            .as_str()
            .unwrap()
            .contains(&*path.to_string_lossy())
    );
}

#[test]
fn w3_pub_03_prior_failure_stays_primary_with_exactly_one_write_secondary() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    let child = runner_command(&fixture, 30).spawn().unwrap();
    let (child, memory) = wrapper::wait_for_memory(child, &fixture);
    let path = eventual_summary_path(&fixture);
    fs::write(&path, OLD_SUMMARY).unwrap();
    let pid = u32::try_from(memory["harness_pid"].as_u64().unwrap()).unwrap();
    wrapper::terminate_tree(pid);
    let output = wrapper::wait_for_output(child, Duration::from_secs(15));

    assert_runner_failed(&output);
    assert_eq!(fs::read(&path).unwrap(), OLD_SUMMARY);
    assert!(owned_stages(&fixture).is_empty());
    let failure = caught(&fixture);
    assert_ne!(failure["primary"]["stage"], "write");
    assert_eq!(failure["secondary"].as_array().unwrap().len(), 1);
    assert_eq!(failure["secondary"][0]["stage"], "write");
    assert_eq!(failure["secondary"][0]["code"], "write_failed");
}

#[test]
fn w3_pub_04_fabricated_cleanup_errors_append_after_existing_secondary_and_write() {
    let module = wrapper::repo_root().join("scripts/CodexDiscordSoak.WrapperEvidence.psm1");
    let invoke = format!(
        r"$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
Import-Module {} -Force
$ledger=New-CodexSoakFailureLedger
$null=Add-CodexSoakFailure $ledger 'runtime' 'existing_primary' 'primary'
$null=Add-CodexSoakFailure $ledger 'cleanup' 'existing_secondary' 'existing secondary'
$publication=[IO.IOException]::new('publish')
$publication.Data['CodexSoakCleanupErrors']=[string[]]@('cleanup one','cleanup two')
$terminal=New-CodexSoakSummaryPublicationTerminalException $ledger $publication
[ordered]@{{
  primary=$terminal.Data['CodexSoakPrimaryFailure']
  secondary=@($terminal.Data['CodexSoakSecondaryFailures'])
}} | ConvertTo-Json -Depth 10 -Compress
",
        ps_literal(&module)
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &invoke])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fabricated failure failed: {output:?}"
    );
    let failure: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        failure,
        json!({
            "primary": {"stage":"runtime","code":"existing_primary","message":"primary"},
            "secondary": [
                {"stage":"cleanup","code":"existing_secondary","message":"existing secondary"},
                {"stage":"write","code":"write_failed","message":"publish"},
                {"stage":"cleanup","code":"cleanup_failed","message":"cleanup one"},
                {"stage":"cleanup","code":"cleanup_failed","message":"cleanup two"}
            ]
        })
    );
}
