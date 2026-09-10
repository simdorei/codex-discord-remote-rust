#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use sha2::{Digest, Sha256};

fn module_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/CodexDiscordSoak.SourceFingerprint.psm1")
}

fn runner(path: &Path) {
    fs::write(
        path,
        r"param([string]$ModulePath, [string]$RepoRoot)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
try {
    Import-Module -Name $ModulePath -Force
    Get-CodexSoakSourceFingerprint -RepoRoot $RepoRoot |
        ConvertTo-Json -Depth 8 -Compress
} catch {
    [ordered]@{
        code = $_.Exception.Data['CodexSoakFailureCode']
        message = $_.Exception.Message
    } | ConvertTo-Json -Compress
    exit 23
}
",
    )
    .unwrap();
}

fn invoke(root: &Path, runner_path: &Path) -> Output {
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(runner_path)
        .arg("-ModulePath")
        .arg(module_path())
        .arg("-RepoRoot")
        .arg(root)
        .output()
        .unwrap()
}

fn fingerprint(root: &Path, runner_path: &Path) -> Value {
    let output = invoke(root, runner_path);
    assert!(
        output.status.success(),
        "fingerprint failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn fixture_files() -> Vec<(String, Vec<u8>)> {
    vec![
        (
            "Cargo.toml".into(),
            b"[workspace]\r\n# raw\0bytes\n".to_vec(),
        ),
        ("Cargo.lock".into(), vec![0xff, 0, 0x80, b'\n']),
        (
            "rust-toolchain.toml".into(),
            b"[toolchain]\nchannel='stable'\n".to_vec(),
        ),
        ("crates/.gitignore".into(), b"ignored.bin\n".to_vec()),
        ("crates/.hidden-dir/secret.bin".into(), vec![0, 1, 2, 3]),
        ("crates/.hidden.bin".into(), b"hidden".to_vec()),
        ("crates/core/Z.rs".into(), b"Z".to_vec()),
        ("crates/core/a.rs".into(), b"a".to_vec()),
        ("crates/core/ignored.bin".into(), b"gitignored".to_vec()),
        ("crates/core/\u{e4}.rs".into(), b"latin".to_vec()),
        ("crates/core/\u{03a9}.rs".into(), b"omega".to_vec()),
        (
            ".cargo/config.toml".into(),
            b"[build]\ntarget-dir='x'\n".to_vec(),
        ),
    ]
}

fn create_fixture(root: &Path, reverse: bool, include_cargo: bool) -> Vec<(String, Vec<u8>)> {
    let mut entries = fixture_files();
    if !include_cargo {
        entries.retain(|(path, _)| !path.starts_with(".cargo/"));
    }
    let order: Box<dyn Iterator<Item = &(String, Vec<u8>)>> = if reverse {
        Box::new(entries.iter().rev())
    } else {
        Box::new(entries.iter())
    };
    for (path, bytes) in order {
        let destination = root.join(path.replace('/', "\\"));
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(destination, bytes).unwrap();
    }
    let hidden_dir = root.join("crates/.hidden-dir");
    let hidden_file = root.join("crates/.hidden.bin");
    for path in [&hidden_dir, &hidden_file] {
        let status = Command::new("attrib").arg("+h").arg(path).status().unwrap();
        assert!(
            status.success(),
            "failed to mark hidden: {}",
            path.display()
        );
    }
    entries
}

fn oracle(mut entries: Vec<(String, Vec<u8>)>) -> (String, Vec<(String, u64, String)>) {
    entries.sort_by(|left, right| left.0.encode_utf16().cmp(right.0.encode_utf16()));
    let mut framed = b"cdr.rust-source-fingerprint.v1\0".to_vec();
    framed.extend_from_slice(&(entries.len() as u64).to_be_bytes());
    let records = entries
        .into_iter()
        .map(|(path, bytes)| {
            let digest = Sha256::digest(&bytes);
            framed.extend_from_slice(&(path.len() as u64).to_be_bytes());
            framed.extend_from_slice(path.as_bytes());
            framed.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
            framed.extend_from_slice(&digest);
            (path, bytes.len() as u64, hex::encode_upper(digest))
        })
        .collect();
    (hex::encode_upper(Sha256::digest(framed)), records)
}

fn assert_matches_oracle(actual: &Value, entries: Vec<(String, Vec<u8>)>, golden: &str) {
    let (expected_digest, expected_files) = oracle(entries);
    assert_eq!(
        expected_digest, golden,
        "independent oracle drifted from golden vector"
    );
    assert_eq!(actual["schema"], "cdr.rust-source-fingerprint.v1");
    assert_eq!(actual["aggregate_sha256"], expected_digest);
    assert_eq!(actual["file_count"], expected_files.len() as u64);
    let actual_files = actual["files"].as_array().unwrap();
    assert_eq!(actual_files.len(), expected_files.len());
    for (actual, (path, bytes, sha256)) in actual_files.iter().zip(expected_files) {
        assert_eq!(actual["path"], path);
        assert_eq!(actual["bytes"], bytes);
        assert_eq!(actual["sha256"], sha256);
    }
}

#[test]
fn sf_01_raw_hidden_ignored_unicode_files_match_independent_framing_oracle() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    let run = temp.path().join("invoke.ps1");
    runner(&run);
    let entries = create_fixture(&root, false, true);
    let actual = fingerprint(&root, &run);
    assert_matches_oracle(
        &actual,
        entries,
        "9C60B4078EEBC4529028EC9934744344756EFA807DA72380F17B632E5081E7D5",
    );
}

#[test]
fn sf_02_creation_order_does_not_change_records_or_aggregate() {
    let temp = tempfile::tempdir().unwrap();
    let run = temp.path().join("invoke.ps1");
    runner(&run);
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    let entries = create_fixture(&first, false, true);
    create_fixture(&second, true, true);
    let one = fingerprint(&first, &run);
    let two = fingerprint(&second, &run);
    assert_eq!(one["files"], two["files"]);
    assert_eq!(one["aggregate_sha256"], two["aggregate_sha256"]);
    assert_matches_oracle(
        &one,
        entries,
        "9C60B4078EEBC4529028EC9934744344756EFA807DA72380F17B632E5081E7D5",
    );
}

#[test]
fn sf_03_same_length_raw_byte_mutation_changes_file_and_aggregate_hashes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    let run = temp.path().join("invoke.ps1");
    runner(&run);
    let mut entries = create_fixture(&root, false, true);
    let before = fingerprint(&root, &run);
    let target = root.join("crates/core/ignored.bin");
    let replacement = b"GITIGNORED";
    assert_eq!(
        fs::metadata(&target).unwrap().len(),
        replacement.len() as u64
    );
    fs::write(&target, replacement).unwrap();
    entries
        .iter_mut()
        .find(|(path, _)| path == "crates/core/ignored.bin")
        .unwrap()
        .1 = replacement.to_vec();
    let after = fingerprint(&root, &run);
    assert_ne!(before["aggregate_sha256"], after["aggregate_sha256"]);
    assert_matches_oracle(
        &after,
        entries,
        "C58EB3A793CF928C1CA00CA91B268FDD72B2F2D3BE8B12922143917D2639026F",
    );
}

#[test]
fn sf_04_absent_and_empty_optional_cargo_have_identical_fingerprints() {
    let temp = tempfile::tempdir().unwrap();
    let run = temp.path().join("invoke.ps1");
    runner(&run);
    let root = temp.path().join("repo");
    let entries = create_fixture(&root, false, false);
    let absent = fingerprint(&root, &run);
    fs::create_dir(root.join(".cargo")).unwrap();
    let empty = fingerprint(&root, &run);
    assert_eq!(absent, empty);
    assert_matches_oracle(
        &empty,
        entries,
        "B69F8E976DDC14F911629C042E3FD3D16C93C2C73DC79F62C69BFB3DF0316E4A",
    );
}
