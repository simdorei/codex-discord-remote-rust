#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Map, Value, json};

const HASH_A: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const HASH_B: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
const HASH_C: &str = "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC";
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
        r"param([string]$ModulePath,[string]$Mode,[string]$InputPath)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$utf8=[Text.UTF8Encoding]::new($false,$true)
try {
    Import-Module -Name $ModulePath -Force
    $inputRecord=[IO.File]::ReadAllText($InputPath,$utf8) | ConvertFrom-Json
    if ($Mode -eq 'fingerprint') {
        if ($inputRecord.inject_unpaired_surrogate) {
            $inputRecord.actual.files[1].path=[string][char]0xD800
        }
        $result=Test-CodexSoakSourceFingerprintEqual `
            -Expected $inputRecord.expected -Actual $inputRecord.actual
    } else {
        $result=Get-CodexSoakFinalEligibility `
            -OperationalStatus $inputRecord.operational_status -Checks $inputRecord.checks
    }
    [ordered]@{unexpected=$result} | ConvertTo-Json -Depth 8 -Compress
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

fn invoke(temp: &Path, mode: &str, input: &Value) -> Output {
    let runner = temp.join("invoke-validation.ps1");
    let input_path = temp.join("validation-input.json");
    write_runner(&runner);
    fs::write(&input_path, serde_json::to_vec(input).unwrap()).unwrap();
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
        .arg("-Mode")
        .arg(mode)
        .arg("-InputPath")
        .arg(input_path)
        .output()
        .unwrap()
}

fn assert_code(temp: &Path, mode: &str, input: &Value, expected_code: &str) {
    let output = invoke(temp, mode, input);
    assert_eq!(
        output.status.code(),
        Some(23),
        "invalid input accepted: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let failure: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(failure["code"], expected_code, "failure was: {failure}");
    assert!(
        failure["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty())
    );
}

fn fingerprint(root: &Path) -> Value {
    json!({
        "schema": "cdr.rust-source-fingerprint.v1",
        "repo_root": root,
        "aggregate_sha256": HASH_A,
        "file_count": 2,
        "total_bytes": 3,
        "files": [
            {"path": "Cargo.lock", "bytes": 1, "sha256": HASH_B},
            {"path": "crates/core/lib.rs", "bytes": 2, "sha256": HASH_C}
        ]
    })
}

fn all_checks() -> Value {
    let values: Map<String, Value> = CHECKS
        .iter()
        .map(|name| ((*name).to_owned(), json!(true)))
        .collect();
    Value::Object(values)
}

#[test]
fn evidence_input_01_null_public_fingerprint_arguments_fail_with_the_contract_code() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    let valid = fingerprint(&root);
    for input in [
        json!({"expected": null, "actual": valid}),
        json!({"expected": valid, "actual": null}),
    ] {
        assert_code(
            temp.path(),
            "fingerprint",
            &input,
            "fingerprint_record_invalid",
        );
    }
}

#[test]
fn evidence_input_02_remaining_malformed_fingerprint_branches_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    let valid = fingerprint(&root);
    let mut malformed = Vec::new();
    for (pointer, value) in [
        ("/total_bytes", json!("1")),
        ("/files", json!({})),
        ("/files/0/bytes", json!("1")),
        ("/files/0/sha256", json!("not-a-sha256")),
    ] {
        let mut changed = valid.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        malformed.push(changed);
    }
    for actual in malformed {
        assert_code(
            temp.path(),
            "fingerprint",
            &json!({"expected": valid, "actual": actual}),
            "fingerprint_record_invalid",
        );
    }
}

#[test]
fn evidence_input_03_null_public_eligibility_arguments_fail_with_the_contract_code() {
    let temp = tempfile::tempdir().unwrap();
    for input in [
        json!({"operational_status": null, "checks": all_checks()}),
        json!({"operational_status": "passed", "checks": null}),
    ] {
        assert_code(
            temp.path(),
            "eligibility",
            &input,
            "eligibility_input_invalid",
        );
    }
}

#[test]
fn evidence_input_04_non_fully_qualified_repo_roots_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    let valid = fingerprint(&root);
    let absolute = root.display().to_string();
    let invalid_roots = [absolute[2..].to_owned(), format!("{}:.", &absolute[..1])];
    for invalid_root in invalid_roots {
        let mut actual = valid.clone();
        actual["repo_root"] = json!(invalid_root);
        assert_code(
            temp.path(),
            "fingerprint",
            &json!({"expected": valid, "actual": actual}),
            "fingerprint_record_invalid",
        );
    }
}

#[test]
fn evidence_input_05_invalid_or_non_utf8_logical_paths_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    let valid = fingerprint(&root);
    for path in ["bad*name", "bad?name", "bad:name", "bad\u{1}name"] {
        let mut actual = valid.clone();
        actual["files"][0]["path"] = json!(path);
        assert_code(
            temp.path(),
            "fingerprint",
            &json!({"expected": valid, "actual": actual}),
            "fingerprint_record_invalid",
        );
    }
    assert_code(
        temp.path(),
        "fingerprint",
        &json!({
            "expected": valid,
            "actual": valid,
            "inject_unpaired_surrogate": true
        }),
        "fingerprint_record_invalid",
    );
}
