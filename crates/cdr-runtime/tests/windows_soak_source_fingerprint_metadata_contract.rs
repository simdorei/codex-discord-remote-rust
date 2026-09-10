#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn module_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/CodexDiscordSoak.SourceFingerprint.psm1")
}

fn fingerprint(root: &Path, runner: &Path) -> Value {
    fs::write(
        runner,
        r"param([string]$ModulePath,[string]$RepoRoot)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
Import-Module -Name $ModulePath -Force
Get-CodexSoakSourceFingerprint -RepoRoot $RepoRoot | ConvertTo-Json -Depth 8 -Compress
",
    )
    .unwrap();
    let output = Command::new("powershell.exe")
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
        .arg("-RepoRoot")
        .arg(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fingerprint failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn comparable(path: &str) -> String {
    path.strip_prefix(r"\\?\")
        .unwrap_or(path)
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn metadata_matches(value: &Value, root: &Path, total_bytes: u64) -> bool {
    value["repo_root"]
        .as_str()
        .is_some_and(|actual| comparable(actual) == comparable(&root.to_string_lossy()))
        && value["total_bytes"].as_u64() == Some(total_bytes)
}

#[test]
fn sf_metadata_01_root_and_raw_total_are_independent_and_mutation_sensitive() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    let runner = temp.path().join("invoke.ps1");
    let files: [(&str, &[u8]); 6] = [
        ("Cargo.toml", b"abc"),
        ("Cargo.lock", &[0xff, 0]),
        ("rust-toolchain.toml", b"toolchain"),
        ("crates/core/lib.rs", &[0, 1, 2, 3]),
        ("crates/core/empty.bin", b""),
        (".cargo/nested/raw.bin", &[0x80, 0x81, 0]),
    ];
    for (path, bytes) in files {
        let destination = root.join(path.replace('/', "\\"));
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, bytes).unwrap();
    }
    let expected_total = files.iter().map(|(_, bytes)| bytes.len() as u64).sum();
    let actual = fingerprint(&root, &runner);
    assert!(metadata_matches(&actual, &root, expected_total));

    let mut wrong_root = actual.clone();
    wrong_root["repo_root"] = Value::String(root.join("other").to_string_lossy().into_owned());
    assert!(!metadata_matches(&wrong_root, &root, expected_total));

    let mut wrong_total = actual;
    wrong_total["total_bytes"] = Value::from(expected_total + 1);
    assert!(!metadata_matches(&wrong_total, &root, expected_total));
}
