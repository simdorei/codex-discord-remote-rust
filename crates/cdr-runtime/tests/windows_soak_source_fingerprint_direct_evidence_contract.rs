#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha256};

fn script_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts")
        .join(name)
}

fn fixture_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("Cargo.toml", b"[workspace]\n".to_vec()),
        ("Cargo.lock", vec![0xff, 0, 0x80, b'\n']),
        (
            "rust-toolchain.toml",
            b"[toolchain]\nchannel='stable'\n".to_vec(),
        ),
        (
            "crates/core/lib.rs",
            b"pub fn value() -> u8 { 7 }\n".to_vec(),
        ),
        (
            ".cargo/config.toml",
            b"[build]\nincremental=false\n".to_vec(),
        ),
    ]
}

fn create_fixture(root: &Path) -> Vec<(&'static str, Vec<u8>)> {
    let files = fixture_files();
    for (logical, bytes) in &files {
        let destination = root.join(logical.replace('/', "\\"));
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, bytes).unwrap();
    }
    files
}

fn oracle(mut files: Vec<(&str, Vec<u8>)>) -> (String, u64, Vec<(String, u64, String)>) {
    files.sort_by(|left, right| left.0.encode_utf16().cmp(right.0.encode_utf16()));
    let mut frame = b"cdr.rust-source-fingerprint.v1\0".to_vec();
    frame.extend_from_slice(&(files.len() as u64).to_be_bytes());
    let mut total = 0;
    let mut records = Vec::new();
    for (logical, bytes) in files {
        let digest = Sha256::digest(&bytes);
        frame.extend_from_slice(&(logical.len() as u64).to_be_bytes());
        frame.extend_from_slice(logical.as_bytes());
        frame.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        frame.extend_from_slice(&digest);
        total += bytes.len() as u64;
        records.push((
            logical.to_owned(),
            bytes.len() as u64,
            hex::encode_upper(digest),
        ));
    }
    (hex::encode_upper(Sha256::digest(frame)), total, records)
}

fn write_runner(path: &Path) {
    fs::write(
        path,
        r"param([string]$SourceModule,[string]$EvidenceModule,[string]$RepoRoot)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
function Test-InertCarrier {
    param([object]$Value,[string[]]$ExpectedNames)
    if ($Value -isnot [pscustomobject]) { return $false }
    $properties=@($Value.PSObject.Properties)
    if ($properties.Count -ne $ExpectedNames.Count) { return $false }
    for ($index=0; $index -lt $ExpectedNames.Count; $index++) {
        $property=$properties[$index]
        if ($property.Name -cne $ExpectedNames[$index] -or
            $property.MemberType -ne [Management.Automation.PSMemberTypes]::NoteProperty) {
            return $false
        }
    }
    return $true
}
try {
    Import-Module -Name $SourceModule -Force
    Import-Module -Name $EvidenceModule -Force
    $first=Get-CodexSoakSourceFingerprint -RepoRoot $RepoRoot
    $second=Get-CodexSoakSourceFingerprint -RepoRoot $RepoRoot
    $topNames=@('schema','repo_root','aggregate_sha256','file_count','total_bytes','files')
    $rowNames=@('path','bytes','sha256')
    $firstRowsValid=$true
    foreach ($row in @($first.files)) {
        if (-not (Test-InertCarrier -Value $row -ExpectedNames $rowNames)) {
            $firstRowsValid=$false
        }
    }
    $secondRowsValid=$true
    foreach ($row in @($second.files)) {
        if (-not (Test-InertCarrier -Value $row -ExpectedNames $rowNames)) {
            $secondRowsValid=$false
        }
    }
    [ordered]@{
        first_top_inert=Test-InertCarrier -Value $first -ExpectedNames $topNames
        second_top_inert=Test-InertCarrier -Value $second -ExpectedNames $topNames
        first_rows_inert=$firstRowsValid
        second_rows_inert=$secondRowsValid
        equal=Test-CodexSoakSourceFingerprintEqual -Expected $first -Actual $second
        schema=$first.schema
        repo_root=$first.repo_root
        aggregate_sha256=$first.aggregate_sha256
        second_aggregate_sha256=$second.aggregate_sha256
        file_count=$first.file_count
        total_bytes=$first.total_bytes
        first_files=@($first.files)
        second_files=@($second.files)
    } | ConvertTo-Json -Depth 6 -Compress
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

fn comparable(path: &str) -> String {
    path.strip_prefix(r"\\?\")
        .unwrap_or(path)
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn assert_rows(actual: &Value, expected: &[(String, u64, String)]) {
    let rows = actual.as_array().unwrap();
    assert_eq!(rows.len(), expected.len());
    for (row, (path, bytes, sha256)) in rows.iter().zip(expected) {
        assert_eq!(row["path"], path.as_str());
        assert_eq!(row["bytes"], *bytes);
        assert_eq!(row["sha256"], sha256.as_str());
    }
}

#[test]
fn direct_sf_01_generator_output_is_inert_and_directly_comparable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    let files = create_fixture(&root);
    let (expected_aggregate, expected_total, expected_files) = oracle(files.clone());
    let runner = temp.path().join("invoke-direct.ps1");
    write_runner(&runner);

    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&runner)
        .arg("-SourceModule")
        .arg(script_path("CodexDiscordSoak.SourceFingerprint.psm1"))
        .arg("-EvidenceModule")
        .arg(script_path("CodexDiscordSoak.Evidence.psm1"))
        .arg("-RepoRoot")
        .arg(&root)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "direct comparison failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: Value = serde_json::from_slice(&output.stdout).unwrap();
    for field in [
        "first_top_inert",
        "second_top_inert",
        "first_rows_inert",
        "second_rows_inert",
        "equal",
    ] {
        assert_eq!(actual[field], true, "contract field {field}");
    }
    assert_eq!(actual["schema"], "cdr.rust-source-fingerprint.v1");
    assert_eq!(actual["aggregate_sha256"], expected_aggregate);
    assert_eq!(actual["second_aggregate_sha256"], expected_aggregate);
    assert_eq!(actual["file_count"], files.len() as u64);
    assert_eq!(actual["total_bytes"], expected_total);
    assert_rows(&actual["first_files"], &expected_files);
    assert_rows(&actual["second_files"], &expected_files);
    assert_eq!(
        comparable(actual["repo_root"].as_str().unwrap()),
        comparable(&root.to_string_lossy())
    );
}

#[test]
fn direct_sf_02_generator_output_is_directly_comparable_in_powershell_7() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    create_fixture(&root);
    let runner = temp.path().join("invoke-direct-pwsh.ps1");
    write_runner(&runner);

    let output = Command::new("pwsh.exe")
        .args(["-NoProfile", "-NonInteractive", "-File"])
        .arg(&runner)
        .arg("-SourceModule")
        .arg(script_path("CodexDiscordSoak.SourceFingerprint.psm1"))
        .arg("-EvidenceModule")
        .arg(script_path("CodexDiscordSoak.Evidence.psm1"))
        .arg("-RepoRoot")
        .arg(&root)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "PowerShell 7 direct comparison failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(actual["equal"], true);
}
