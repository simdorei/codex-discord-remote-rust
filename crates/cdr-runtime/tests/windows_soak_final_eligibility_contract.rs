#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Map, Value, json};

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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/CodexDiscordSoak.Evidence.psm1")
}

fn write_runner(path: &Path) {
    fs::write(
        path,
        r"param([string]$ModulePath,[string]$InputPath)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$utf8=[Text.UTF8Encoding]::new($false,$true)
try {
    Import-Module -Name $ModulePath -Force
    $inputRecord=[IO.File]::ReadAllText($InputPath,$utf8) | ConvertFrom-Json
    $result=Get-CodexSoakFinalEligibility `
        -OperationalStatus $inputRecord.operational_status -Checks $inputRecord.checks
    [ordered]@{
        result=$result; check_names=@($result.checks.PSObject.Properties.Name)
    } | ConvertTo-Json -Depth 8 -Compress
} catch {
    [ordered]@{
        code=$_.Exception.Data['CodexSoakFailureCode']; message=$_.Exception.Message
    } | ConvertTo-Json -Compress
    exit 23
}
",
    )
    .unwrap();
}

fn all_checks(value: bool) -> Value {
    let checks: Map<String, Value> = CHECKS
        .iter()
        .map(|name| ((*name).to_owned(), json!(value)))
        .collect();
    Value::Object(checks)
}

fn emitted_checks(operational_passed: bool, checks: &Value) -> Value {
    let mut emitted = checks.as_object().unwrap().clone();
    emitted.insert("operational_passed".into(), json!(operational_passed));
    Value::Object(emitted)
}

fn invoke(temp: &Path, operational_status: &Value, checks: &Value) -> Output {
    let runner = temp.join("invoke-eligibility.ps1");
    let input = temp.join("eligibility-input.json");
    write_runner(&runner);
    fs::write(
        &input,
        serde_json::to_vec(&json!({
            "operational_status": operational_status,
            "checks": checks
        }))
        .unwrap(),
    )
    .unwrap();
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
        .arg("-InputPath")
        .arg(input)
        .output()
        .unwrap()
}

fn success(temp: &Path, status: &str, checks: &Value) -> Value {
    let output = invoke(temp, &json!(status), checks);
    assert!(
        output.status.success(),
        "eligibility failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn assert_invalid(temp: &Path, status: &Value, checks: &Value) {
    let output = invoke(temp, status, checks);
    assert_eq!(output.status.code(), Some(23));
    let failure: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(failure["code"], "eligibility_input_invalid");
    assert!(
        failure["message"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );
}

#[test]
fn eligibility_01_all_checks_emit_the_fixed_eligible_contract() {
    let temp = tempfile::tempdir().unwrap();
    let checks = all_checks(true);
    let envelope = success(temp.path(), "passed", &checks);
    let result = &envelope["result"];
    assert_eq!(result["schema"], "cdr.windows-soak.final-eligibility.v1");
    assert_eq!(result["status"], "eligible");
    assert_eq!(result["eligible"], true);
    assert_eq!(result["reasons"], json!([]));
    assert_eq!(
        result["requirements"],
        json!({
            "minimum_duration_seconds": 86400,
            "warmup_seconds": 3600,
            "maximum_sample_interval_seconds": 60,
            "maximum_slope_bytes_per_hour": 1_048_576,
            "minimum_regression_samples": 2
        })
    );
    let expected_names: Vec<_> = std::iter::once("operational_passed")
        .chain(CHECKS)
        .collect();
    assert_eq!(envelope["check_names"], json!(expected_names));
    assert_eq!(result["checks"], emitted_checks(true, &checks));
}

#[test]
fn eligibility_02_each_false_check_is_independently_ineligible_with_exact_reason() {
    let temp = tempfile::tempdir().unwrap();
    for name in CHECKS {
        let mut checks = all_checks(true);
        checks[name] = json!(false);
        let result = success(temp.path(), "passed", &checks)["result"].clone();
        assert_eq!(result["status"], "ineligible", "{name}");
        assert_eq!(result["eligible"], false, "{name}");
        assert_eq!(result["reasons"], json!([name]), "{name}");
        assert_eq!(result["checks"], emitted_checks(true, &checks), "{name}");
    }
}

#[test]
fn eligibility_03_reasons_follow_fixed_order_and_operational_failure_leads() {
    let temp = tempfile::tempdir().unwrap();
    let mut checks = all_checks(true);
    for name in [CHECKS[18], CHECKS[3], CHECKS[10]] {
        checks[name] = json!(false);
    }
    let expected_tail = json!([CHECKS[3], CHECKS[10], CHECKS[18]]);
    let passed = success(temp.path(), "passed", &checks)["result"].clone();
    assert_eq!(passed["reasons"], expected_tail);
    assert_eq!(passed["status"], "ineligible");
    let all_false = all_checks(false);
    let passed = success(temp.path(), "passed", &all_false)["result"].clone();
    assert_eq!(passed["reasons"], json!(CHECKS));

    for operational in ["integrity_failed", "runtime_failed"] {
        let clean = success(temp.path(), operational, &all_checks(true))["result"].clone();
        assert_eq!(clean["status"], "failed");
        assert_eq!(clean["eligible"], false);
        assert_eq!(clean["reasons"], json!([operational]));
        assert_eq!(clean["checks"], emitted_checks(false, &all_checks(true)));
        for name in CHECKS {
            let mut single = all_checks(true);
            single[name] = json!(false);
            let result = success(temp.path(), operational, &single)["result"].clone();
            assert_eq!(result["status"], "failed", "{operational}/{name}");
            assert_eq!(result["eligible"], false, "{operational}/{name}");
            assert_eq!(result["checks"], emitted_checks(false, &single));
            assert_eq!(result["reasons"], json!([operational, name]));
        }
        let failed = success(temp.path(), operational, &all_false)["result"].clone();
        let reasons: Vec<_> = std::iter::once(operational).chain(CHECKS).collect();
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["reasons"], json!(reasons));
    }
}

#[test]
fn eligibility_04_successful_short_or_skip_build_style_checks_are_ineligible() {
    let temp = tempfile::tempdir().unwrap();
    let mut short = all_checks(true);
    short["minimum_duration_met"] = json!(false);
    short["harness_elapsed_reached_request"] = json!(false);
    let result = success(temp.path(), "passed", &short)["result"].clone();
    assert_eq!(result["status"], "ineligible");
    assert_eq!(
        result["reasons"],
        json!(["minimum_duration_met", "harness_elapsed_reached_request"])
    );

    let mut skip_build = all_checks(true);
    skip_build["same_run_canonical_release_build"] = json!(false);
    let result = success(temp.path(), "passed", &skip_build)["result"].clone();
    assert_eq!(result["status"], "ineligible");
    assert_eq!(
        result["reasons"],
        json!(["same_run_canonical_release_build"])
    );
}

#[test]
fn eligibility_05_malformed_status_or_check_objects_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let valid = all_checks(true);
    assert_invalid(temp.path(), &json!("unknown"), &valid);
    assert_invalid(temp.path(), &json!(7), &valid);

    let mut missing = valid.clone();
    missing.as_object_mut().unwrap().remove(CHECKS[0]);
    assert_invalid(temp.path(), &json!("passed"), &missing);
    let mut extra = valid.clone();
    extra["unexpected"] = json!(true);
    assert_invalid(temp.path(), &json!("passed"), &extra);
    for value in [json!("false"), json!(0), Value::Null] {
        let mut wrong_type = valid.clone();
        wrong_type[CHECKS[5]] = value;
        assert_invalid(temp.path(), &json!("passed"), &wrong_type);
    }
}
