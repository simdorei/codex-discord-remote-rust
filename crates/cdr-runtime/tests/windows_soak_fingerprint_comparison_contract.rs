#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

const HASH_A: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const HASH_B: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
const HASH_C: &str = "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC";

fn module_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/CodexDiscordSoak.Evidence.psm1")
}

fn write_runner(path: &Path) {
    fs::write(
        path,
        r"param([string]$ModulePath,[string]$ExpectedPath,[string]$ActualPath)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$utf8=[Text.UTF8Encoding]::new($false,$true)
try {
    Import-Module -Name $ModulePath -Force
    $expected=[IO.File]::ReadAllText($ExpectedPath,$utf8) | ConvertFrom-Json
    $actual=[IO.File]::ReadAllText($ActualPath,$utf8) | ConvertFrom-Json
    [ordered]@{
        equal=Test-CodexSoakSourceFingerprintEqual -Expected $expected -Actual $actual
    } | ConvertTo-Json -Compress
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

fn record(root: &Path) -> Value {
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

fn invoke(temp: &Path, expected: &Value, actual: &Value) -> Output {
    let runner = temp.join("invoke.ps1");
    let expected_path = temp.join("expected.json");
    let actual_path = temp.join("actual.json");
    write_runner(&runner);
    fs::write(&expected_path, serde_json::to_vec(expected).unwrap()).unwrap();
    fs::write(&actual_path, serde_json::to_vec(actual).unwrap()).unwrap();
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
        .arg("-ExpectedPath")
        .arg(expected_path)
        .arg("-ActualPath")
        .arg(actual_path)
        .output()
        .unwrap()
}

fn result(temp: &Path, expected: &Value, actual: &Value) -> bool {
    let output = invoke(temp, expected, actual);
    assert!(
        output.status.success(),
        "comparison failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["equal"] == true
}

fn assert_invalid(temp: &Path, expected: &Value, actual: &Value) {
    let output = invoke(temp, expected, actual);
    assert_eq!(output.status.code(), Some(23), "invalid record accepted");
    let failure: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(failure["code"], "fingerprint_record_invalid");
    assert!(
        failure["message"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );
}

#[test]
fn fingerprint_equal_01_compares_semantics_not_json_text_or_property_order() {
    let module = fs::read(module_path()).unwrap();
    assert!(!module.starts_with(&[0xef, 0xbb, 0xbf]));
    std::str::from_utf8(&module).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    let expected = record(&root);
    let alternate_root = root.join(".");
    let actual: Value = serde_json::from_str(&format!(
        r#"{{"files":[{{"sha256":"{HASH_B}","bytes":1,"path":"Cargo.lock"}},{{"bytes":2,"path":"crates/core/lib.rs","sha256":"{HASH_C}"}}],"total_bytes":3,"file_count":2,"aggregate_sha256":"{HASH_A}","repo_root":{},"schema":"cdr.rust-source-fingerprint.v1"}}"#,
        serde_json::to_string(&alternate_root).unwrap()
    ))
    .unwrap();
    assert!(result(temp.path(), &expected, &actual));
    for equivalent in [
        format!("{}\\", root.display()),
        root.join("unused/..").display().to_string(),
        root.display().to_string().to_ascii_uppercase(),
    ] {
        let mut normalized = expected.clone();
        normalized["repo_root"] = json!(equivalent);
        assert!(result(temp.path(), &expected, &normalized));
    }
}

#[test]
fn fingerprint_equal_02_each_complete_record_dimension_can_break_equality() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    let other_root = temp.path().join("other");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&other_root).unwrap();
    let expected = record(&root);
    let mut variants = Vec::new();

    for (pointer, value) in [
        ("/repo_root", json!(other_root)),
        ("/aggregate_sha256", json!(HASH_C)),
    ] {
        let mut changed = expected.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        variants.push(changed);
    }
    let mut added = expected.clone();
    added["files"].as_array_mut().unwrap().push(json!({
        "path": "rust-toolchain.toml", "bytes": 4, "sha256": HASH_A
    }));
    added["file_count"] = json!(3);
    added["total_bytes"] = json!(7);
    variants.push(added);

    for row in 0..2 {
        for field in ["path", "bytes", "sha256"] {
            let mut changed = expected.clone();
            changed["files"][row][field] = match field {
                "path" => json!(format!("changed/{row}.rs")),
                "bytes" => json!(changed["files"][row]["bytes"].as_u64().unwrap() + 1),
                _ => json!(HASH_A),
            };
            if field == "bytes" {
                changed["total_bytes"] = json!(4);
            }
            variants.push(changed);
        }
    }
    let mut reordered = expected.clone();
    reordered["files"].as_array_mut().unwrap().swap(0, 1);
    assert_invalid(temp.path(), &reordered, &reordered);

    for (index, changed) in variants.iter().enumerate() {
        assert!(!result(temp.path(), &expected, changed), "mutation {index}");
    }
}

#[test]
fn fingerprint_equal_03_malformed_records_fail_closed_with_a_stable_code() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    let valid = record(&root);
    let mut invalid = Vec::new();

    let mut missing = valid.clone();
    missing.as_object_mut().unwrap().remove("schema");
    invalid.push(missing);
    let mut wrong_schema = valid.clone();
    wrong_schema["schema"] = json!("cdr.rust-source-fingerprint.v2");
    invalid.push(wrong_schema.clone());
    let mut extra = valid.clone();
    extra["unexpected"] = json!(true);
    invalid.push(extra);
    let mut wrong_type = valid.clone();
    wrong_type["file_count"] = json!("2");
    invalid.push(wrong_type);
    let mut lowercase_hash = valid.clone();
    lowercase_hash["aggregate_sha256"] = json!(HASH_A.to_ascii_lowercase());
    invalid.push(lowercase_hash);
    let mut bad_count = valid.clone();
    bad_count["file_count"] = json!(1);
    invalid.push(bad_count);
    let mut bad_total = valid.clone();
    bad_total["total_bytes"] = json!(4);
    invalid.push(bad_total);
    let mut bad_row = valid.clone();
    bad_row["files"][0]
        .as_object_mut()
        .unwrap()
        .remove("sha256");
    invalid.push(bad_row);
    let mut extra_row = valid.clone();
    extra_row["files"][0]["unexpected"] = json!(true);
    invalid.push(extra_row);
    let mut missing_root = valid.clone();
    missing_root["repo_root"] = json!(temp.path().join("missing"));
    invalid.push(missing_root);
    let mut relative_root = valid.clone();
    relative_root["repo_root"] = json!(".");
    invalid.push(relative_root);
    let mut nul_root = valid.clone();
    nul_root["repo_root"] = json!(format!("{}\0", root.display()));
    invalid.push(nul_root);
    for path in [
        "../Cargo.lock",
        "crates\\core\\lib.rs",
        "C:/source.rs",
        "\0",
    ] {
        let mut bad_path = valid.clone();
        bad_path["files"][1]["path"] = json!(path);
        invalid.push(bad_path);
    }
    let mut duplicate_path = valid.clone();
    duplicate_path["files"][1]["path"] = json!("cargo.LOCK");
    invalid.push(duplicate_path);

    for malformed in &invalid {
        assert_invalid(temp.path(), &valid, malformed);
    }
    assert_invalid(temp.path(), &wrong_schema, &wrong_schema);
}
