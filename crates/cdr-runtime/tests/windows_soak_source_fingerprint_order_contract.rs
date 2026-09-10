#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn module_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/CodexDiscordSoak.SourceFingerprint.psm1")
}

fn fingerprint(root: &Path, run: &Path) -> Value {
    fs::write(
        run,
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
        .arg(run)
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

#[test]
fn sf_order_02_sort_is_dotnet_ordinal_while_framed_paths_are_utf8() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    let run = temp.path().join("invoke.ps1");
    for (path, bytes) in [
        ("Cargo.toml", b"a".as_slice()),
        ("Cargo.lock", b"b".as_slice()),
        ("rust-toolchain.toml", b"c".as_slice()),
        ("crates/core/\u{e000}.rs", b"d".as_slice()),
        ("crates/core/\u{10000}.rs", b"e".as_slice()),
    ] {
        let destination = root.join(path.replace('/', "\\"));
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, bytes).unwrap();
    }
    let actual = fingerprint(&root, &run);
    assert_eq!(
        actual["aggregate_sha256"],
        "5FA7EDD753F2DC5DD06C21609761043F3514EAB4ED00E1A9AE3F8C8540AFC3E8"
    );
    let paths: Vec<_> = actual["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    let supplementary = paths
        .iter()
        .position(|path| *path == "crates/core/\u{10000}.rs")
        .unwrap();
    let private_use = paths
        .iter()
        .position(|path| *path == "crates/core/\u{e000}.rs")
        .unwrap();
    assert!(
        supplementary < private_use,
        ".NET ordinal compares UTF-16 code units"
    );
}
