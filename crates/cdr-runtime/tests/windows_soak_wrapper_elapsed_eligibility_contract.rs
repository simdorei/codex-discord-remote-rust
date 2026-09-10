#![cfg(windows)]

use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CHECKS: [&str; 19] = [
    "same_run_canonical_release_build",
    "canonical_release_harness",
    "build_fingerprints_equal",
    "soak_fingerprints_equal",
    "harness_provenance_verified",
    "harness_process_exit_confirmed",
    "child_exit_zero",
    "harness_contract_passed",
    "event_stream_valid",
    "minimum_duration_met",
    "harness_duration_matches_request",
    "harness_elapsed_reached_request",
    "canonical_warmup",
    "sampling_interval_not_weakened",
    "slope_limit_not_weakened",
    "regression_samples_met",
    "memory_slope_passed",
    "disabled_marker_preserved",
    "cleanup_clean",
];

fn module_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/CodexDiscordSoak.WrapperEvidence.psm1")
}

fn wrapper_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../codex-discord-rust-soak.ps1")
}

fn write_runner(path: &Path) {
    fs::write(
        path,
        r#"param([string]$ModulePath,[string]$CaseName)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
Import-Module -Name $ModulePath -Force
$provenance=[pscustomobject]@{ verified=$true; process_exit_confirmed=$true }
$runElapsed=86400.0
$harnessElapsed=[long]86400000
switch ($CaseName) {
    'short_wrapper' { $runElapsed=1.0 }
    'just_short_wrapper' { $runElapsed=86399.999 }
    'short_harness' { $harnessElapsed=[long]86399999 }
    'nan' { $runElapsed=[double]::NaN }
    'positive_infinity' { $runElapsed=[double]::PositiveInfinity }
    'negative_infinity' { $runElapsed=[double]::NegativeInfinity }
    'negative' { $runElapsed=-1.0 }
    'malformed' { $runElapsed='not-a-number' }
    'canonical' { }
    default { throw "unknown case: $CaseName" }
}
$harness=[pscustomobject]@{
    schema='cdr.offline-soak.summary.v1'; mode='offline_fake_replay'; status='passed'
    duration_secs=86400; elapsed_ms=$harnessElapsed
}
$regression=[pscustomobject]@{ Count=2 }
$checks=Get-CodexSoakEligibilityChecks `
    $true $true $true $true $provenance 0 $harness 1 86400 $runElapsed `
    3600 60.0 1048576.0 $regression $true $true $true
[ordered]@{
    names=@($checks.PSObject.Properties.Name); checks=$checks
} | ConvertTo-Json -Depth 4 -Compress
"#,
    )
    .unwrap();
}

fn invoke(case_name: &str) -> Output {
    let temp = tempfile::tempdir().unwrap();
    let runner = temp.path().join("invoke-elapsed-check.ps1");
    write_runner(&runner);
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(runner)
        .arg("-ModulePath")
        .arg(module_path())
        .arg("-CaseName")
        .arg(case_name)
        .output()
        .unwrap()
}

fn success(case_name: &str) -> Value {
    let output = invoke(case_name);
    assert!(
        output.status.success(),
        "PowerShell case {case_name} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn assert_only_elapsed_is_false(case_name: &str) {
    let envelope = success(case_name);
    assert_eq!(envelope["names"], json!(CHECKS));
    for name in CHECKS {
        assert_eq!(
            envelope["checks"][name],
            json!(name != "harness_elapsed_reached_request"),
            "unexpected {name} value for {case_name}"
        );
    }
}

#[test]
fn wrapper_clock_starts_before_launch_and_its_observation_is_wired_to_eligibility() {
    let source = fs::read_to_string(wrapper_path()).unwrap();
    let clock_start = source
        .find("$runClock = [Diagnostics.Stopwatch]::StartNew()")
        .unwrap();
    let process_start = source.find("if (-not $child.Start())").unwrap();
    let clock_stop = source
        .find("$runClock.Stop(); $runElapsedSeconds = $runClock.Elapsed.TotalSeconds")
        .unwrap();
    let eligibility = source
        .find("$checks = Get-CodexSoakEligibilityChecks")
        .unwrap();
    assert!(clock_start < process_start);
    assert!(process_start < clock_stop);
    assert!(clock_stop < eligibility);
    assert!(source[eligibility..].contains("$DurationSeconds $runElapsedSeconds $WarmupSeconds"));
}

#[test]
fn post_loop_elapsed_deadline_is_checked_before_exit_code_acceptance() {
    let source = fs::read_to_string(wrapper_path()).unwrap();
    let post_loop_wait = source.find("$child.WaitForExit();").unwrap();
    let elapsed_assignment = source
        .find("$runElapsedSeconds = $runClock.Elapsed.TotalSeconds")
        .unwrap();
    let deadline_check = source[elapsed_assignment..]
        .find("Test-CodexSoakWithinExitDeadline $runClock.Elapsed $DurationSeconds $HarnessExitGraceSeconds")
        .map(|offset| elapsed_assignment + offset)
        .unwrap();
    let exit_code_acceptance = source.find("$exitCode = $child.ExitCode").unwrap();
    let evidence_parse = source.find("$stage = 'parse'").unwrap();

    assert!(post_loop_wait < elapsed_assignment);
    assert!(elapsed_assignment < deadline_check);
    assert!(deadline_check < exit_code_acceptance);
    assert!(exit_code_acceptance < evidence_parse);
}

#[test]
fn canonical_elapsed_maps_all_nineteen_checks_true_in_fixed_order() {
    let envelope = success("canonical");
    assert_eq!(envelope["names"], json!(CHECKS));
    for name in CHECKS {
        assert_eq!(envelope["checks"][name], true, "{name}");
    }
}

#[test]
fn materially_short_wrapper_elapsed_time_fails_the_elapsed_check() {
    assert_only_elapsed_is_false("short_wrapper");
}

#[test]
fn wrapper_elapsed_just_below_the_strict_boundary_fails() {
    assert_only_elapsed_is_false("just_short_wrapper");
}

#[test]
fn harness_and_wrapper_elapsed_inputs_are_both_required() {
    assert_only_elapsed_is_false("short_harness");
}

#[test]
fn nonfinite_or_negative_wrapper_elapsed_values_fail_closed() {
    for case_name in ["nan", "positive_infinity", "negative_infinity", "negative"] {
        assert_only_elapsed_is_false(case_name);
    }
}

#[test]
fn malformed_wrapper_elapsed_value_fails_closed() {
    let output = invoke("malformed");
    assert!(!output.status.success());
}
