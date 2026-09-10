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
        r"param([string]$ModulePath,[string]$Mode,[string]$InputPath)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$utf8=[Text.UTF8Encoding]::new($false,$true)
try {
    Import-Module -Name $ModulePath -Force
    $expected=[IO.File]::ReadAllText($InputPath,$utf8) | ConvertFrom-Json
    $actual=[IO.File]::ReadAllText($InputPath,$utf8) | ConvertFrom-Json
    if ($Mode -eq 'wrapped_root') {
        $script:safeRoot=$actual.repo_root
        $wrapped=[Management.Automation.PSObject]::AsPSObject('C:.')
        $wrapped | Add-Member ScriptMethod Replace { param($old,$new) $script:safeRoot } -Force
        $expected.repo_root=$wrapped
    } elseif ($Mode -eq 'wrapped_path') {
        foreach ($record in @($expected,$actual)) {
            $wrapped=[Management.Automation.PSObject]::AsPSObject('../escape')
            $wrapped | Add-Member ScriptMethod Split { param($separator) @('Cargo.lock') } -Force
            $wrapped | Add-Member ScriptMethod Contains { param($fragment) $false } -Force
            $record.files[0].path=$wrapped
        }
    } elseif ($Mode -eq 'wrapped_rank') {
        $matrix=[object[,]]::new(1,2)
        $matrix[0,0]=$expected.files[0]; $matrix[0,1]=$expected.files[1]
        Add-Member -InputObject $matrix -MemberType ScriptProperty -Name Rank -Value { 1 } -Force
        $expected.files=$matrix
    }
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

fn invoke(temp: &Path, mode: &str, input: &Value) -> Output {
    let runner = temp.join("invoke-ets-wrapper.ps1");
    let input_path = temp.join("ets-input.json");
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

fn parse(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON ({error}); stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn ets_wrapper_01_valid_plain_record_still_compares_equal() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    let output = invoke(temp.path(), "plain", &fingerprint(&root));
    assert!(output.status.success(), "result was: {}", parse(&output));
    assert_eq!(parse(&output)["equal"], true);
}

#[test]
fn ets_wrapper_02_wrapped_strings_and_rank_spoofs_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    let input = fingerprint(&root);
    for mode in ["wrapped_root", "wrapped_path", "wrapped_rank"] {
        let output = invoke(temp.path(), mode, &input);
        assert_eq!(
            output.status.code(),
            Some(23),
            "result was: {}",
            parse(&output)
        );
        assert_eq!(parse(&output)["code"], "fingerprint_record_invalid");
    }
}
