#![cfg(windows)]

#[path = "support/windows_soak_wrapper.rs"]
mod wrapper;

use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

const ELIGIBILITY_MESSAGE: &str = "forced final eligibility failure";

fn inject_eligibility_failure(fixture: &wrapper::Fixture) {
    let path = fixture.root.join("scripts/CodexDiscordSoak.Evidence.psm1");
    let mut module = fs::read_to_string(&path).unwrap();
    module.push_str(
        r"
function Get-CodexSoakFinalEligibility {
    $failure = [InvalidOperationException]::new('forced final eligibility failure')
    $failure.Data['CodexSoakFailureCode'] = 'eligibility_input_invalid'
    throw $failure
}
Export-ModuleMember -Function 'Get-CodexSoakFinalEligibility'
",
    );
    fs::write(path, module).unwrap();
}

fn write_runner(fixture: &wrapper::Fixture) -> PathBuf {
    let path = fixture.root.join("invoke-wrapper.ps1");
    fs::write(
        &path,
        r"param(
    [string]$WrapperPath, [string]$CaughtPath, [string]$RepoRoot,
    [string]$OutputDirectory, [string]$HarnessPath,
    [string]$ExpectedHarnessSha256, [string]$CargoPath, [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$utf8 = [Text.UTF8Encoding]::new($false, $true)
try {
    $wrapperArgs = @{
        RepoRoot = $RepoRoot; OutputDirectory = $OutputDirectory
        HarnessPath = $HarnessPath; ExpectedHarnessSha256 = $ExpectedHarnessSha256
        DurationSeconds = 1; SampleIntervalSeconds = 0.1; WarmupSeconds = 0
        HarnessExitGraceSeconds = 5; MaxSlopeBytesPerHour = 1000000000000000
    }
    if ($SkipBuild) { $wrapperArgs.SkipBuild = $true }
    else { $wrapperArgs.CargoPath = $CargoPath }
    & $WrapperPath @wrapperArgs
} catch {
    $caught = [ordered]@{
        ExceptionMessage = $_.Exception.Message
        CodexSoakFailureStage = $_.Exception.Data['CodexSoakFailureStage']
        CodexSoakFailureCode = $_.Exception.Data['CodexSoakFailureCode']
        CodexSoakPrimaryFailure = $_.Exception.Data['CodexSoakPrimaryFailure']
        CodexSoakSecondaryFailures = @($_.Exception.Data['CodexSoakSecondaryFailures'])
    }
    [IO.File]::WriteAllText($CaughtPath, ($caught | ConvertTo-Json -Depth 20), $utf8)
    exit 71
}
",
    )
    .unwrap();
    path
}

fn runner_command(fixture: &wrapper::Fixture, cargo: Option<&Path>) -> Command {
    let runner = write_runner(fixture);
    let caught = fixture.root.join("caught.json");
    let harness = cargo.map_or(&fixture.debug_harness, |_| &fixture.release_harness);
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
        .arg("-WrapperPath")
        .arg(&fixture.script)
        .arg("-CaughtPath")
        .arg(caught)
        .arg("-RepoRoot")
        .arg(&fixture.root)
        .arg("-OutputDirectory")
        .arg(&fixture.evidence)
        .arg("-HarnessPath")
        .arg(harness)
        .args(["-ExpectedHarnessSha256", &wrapper::sha256(harness)])
        .env("CARGO_TARGET_DIR", &fixture.target)
        .env_remove("DISCORD_BOT_TOKEN")
        .env_remove("DISCORD_TOKEN")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cargo) = cargo {
        command.arg("-CargoPath").arg(cargo);
    } else {
        command.arg("-SkipBuild");
    }
    command
}

fn run(command: &mut Command) -> Output {
    let child = command.spawn().unwrap();
    wrapper::wait_for_output(child, Duration::from_secs(20))
}

fn caught(fixture: &wrapper::Fixture) -> Value {
    serde_json::from_slice(&fs::read(fixture.root.join("caught.json")).unwrap()).unwrap()
}

fn assert_runner_failed(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(71),
        "runner did not catch terminal failure\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_failure_record(record: &Value, stage: &str, code: &str) {
    let object = record.as_object().expect("failure record");
    assert_eq!(object.len(), 3);
    assert_eq!(record["stage"], stage);
    assert_eq!(record["code"], code);
    assert!(
        record["message"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
}

fn assert_terminal_matches(caught: &Value, primary: &Value, secondary: &Value) {
    assert_eq!(caught["CodexSoakFailureStage"], primary["stage"]);
    assert_eq!(caught["CodexSoakFailureCode"], primary["code"]);
    assert_eq!(caught["CodexSoakPrimaryFailure"], *primary);
    assert_eq!(caught["CodexSoakSecondaryFailures"], *secondary);
    assert_eq!(caught["ExceptionMessage"], primary["message"]);
}

fn assert_written_failure(summary: &Value, primary: &Value, secondary: &Value) {
    assert_eq!(summary["schema"], "cdr.windows-soak.summary.v3");
    assert_eq!(summary["operational_status"], "runtime_failed");
    assert!(summary["final_eligibility"].is_null());
    assert_eq!(summary["source_provenance"]["primary_failure"], *primary);
    assert_eq!(
        summary["source_provenance"]["secondary_failures"],
        *secondary
    );
}

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

#[test]
fn finalization_01_eligibility_failure_is_normalized_written_and_rethrown() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    inject_eligibility_failure(&fixture);
    let output = run(&mut runner_command(&fixture, None));
    assert_runner_failed(&output);

    let primary = json!({
        "stage": "final_eligibility",
        "code": "eligibility_input_invalid",
        "message": ELIGIBILITY_MESSAGE
    });
    let secondary = json!([]);
    let summary = wrapper::summary(&fixture);
    assert_written_failure(&summary, &primary, &secondary);
    assert_terminal_matches(&caught(&fixture), &primary, &secondary);
    wrapper::assert_marker(&fixture);
}

#[test]
fn finalization_02_build_failure_remains_primary_and_eligibility_is_secondary() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    inject_eligibility_failure(&fixture);
    let cargo = wrapper::fake_cargo(&fixture, false, 31);
    let output = run(&mut runner_command(&fixture, Some(&cargo)));
    assert_runner_failed(&output);

    let primary = json!({
        "stage": "build",
        "code": "build_failed",
        "message": "Offline soak harness build failed with exit 31"
    });
    let secondary = json!([{
        "stage": "final_eligibility",
        "code": "eligibility_input_invalid",
        "message": ELIGIBILITY_MESSAGE
    }]);
    let summary = wrapper::summary(&fixture);
    assert_written_failure(&summary, &primary, &secondary);
    assert_terminal_matches(&caught(&fixture), &primary, &secondary);
    wrapper::assert_marker(&fixture);
}

#[test]
fn finalization_03_final_write_failure_is_terminal_and_publishes_no_summary() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    let child = runner_command(&fixture, None).spawn().unwrap();
    let (child, _) = wrapper::wait_for_memory(child, &fixture);
    let summary_path = eventual_summary_path(&fixture);
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(&summary_path)
        .unwrap();
    let output = wrapper::wait_for_output(child, Duration::from_secs(12));
    drop(lock);
    assert_runner_failed(&output);
    assert_eq!(fs::metadata(&summary_path).unwrap().len(), 0);

    let caught = caught(&fixture);
    let primary = &caught["CodexSoakPrimaryFailure"];
    let secondary = json!([]);
    assert_failure_record(primary, "write", "write_failed");
    assert_terminal_matches(&caught, primary, &secondary);
    let detail = primary["message"].as_str().unwrap();
    assert!(detail.contains(summary_path.file_name().unwrap().to_str().unwrap()));
    wrapper::assert_marker(&fixture);
}
