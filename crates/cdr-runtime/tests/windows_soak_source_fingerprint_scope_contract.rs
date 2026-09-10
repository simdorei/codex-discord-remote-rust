#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn module_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/CodexDiscordSoak.SourceFingerprint.psm1")
}

fn runner(path: &Path) {
    fs::write(
        path,
        r"param([string]$ModulePath,[string]$RepoRoot)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
Import-Module -Name $ModulePath -Force
Get-CodexSoakSourceFingerprint -RepoRoot $RepoRoot | ConvertTo-Json -Depth 8 -Compress
",
    )
    .unwrap();
}

fn fingerprint(root: &Path, run: &Path) -> Value {
    let output: Output = Command::new("powershell.exe")
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

fn basic_repo(root: &Path) {
    for (path, bytes) in [
        ("Cargo.toml", b"workspace".as_slice()),
        ("Cargo.lock", b"lock".as_slice()),
        ("rust-toolchain.toml", b"toolchain".as_slice()),
        ("crates/core/lib.rs", b"source".as_slice()),
    ] {
        let destination = root.join(path.replace('/', "\\"));
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, bytes).unwrap();
    }
}

fn paths(value: &Value) -> Vec<&str> {
    value["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["path"].as_str().unwrap())
        .collect()
}

#[test]
fn sf_scope_01_exact_filesystem_closure_includes_all_scoped_and_excludes_outputs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    let run = temp.path().join("invoke.ps1");
    runner(&run);
    basic_repo(&root);
    let baseline = fingerprint(&root, &run);

    let pyc = root.join("crates/pkg/__pycache__/x.pyc");
    let cargo_hidden = root.join(".cargo/nested/.arbitrary.bin");
    fs::create_dir_all(pyc.parent().unwrap()).unwrap();
    fs::create_dir_all(cargo_hidden.parent().unwrap()).unwrap();
    fs::write(&pyc, [0xff, 0, 1]).unwrap();
    fs::write(&cargo_hidden, b"cargo-any-file").unwrap();
    let scoped = fingerprint(&root, &run);
    assert_ne!(baseline["aggregate_sha256"], scoped["aggregate_sha256"]);
    let scoped_paths = paths(&scoped);
    assert!(scoped_paths.contains(&"crates/pkg/__pycache__/x.pyc"));
    assert!(scoped_paths.contains(&".cargo/nested/.arbitrary.bin"));

    for (path, bytes) in [
        ("README.md", b"docs".as_slice()),
        (".git/objects/fake", b"metadata".as_slice()),
        ("target/soak/run.json", b"evidence-1".as_slice()),
    ] {
        let destination = root.join(path.replace('/', "\\"));
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, bytes).unwrap();
    }
    let excluded = fingerprint(&root, &run);
    assert_eq!(
        scoped, excluded,
        "out-of-scope outputs changed source identity"
    );
    for path in paths(&excluded) {
        assert!(!path.starts_with("target/"));
        assert!(!path.starts_with(".git/"));
        assert_ne!(path, "README.md");
    }
    fs::write(root.join("target/soak/run.json"), b"evidence-2").unwrap();
    fs::write(root.join("target/soak/new.json"), b"new output").unwrap();
    assert_eq!(excluded, fingerprint(&root, &run));

    let pyo = root.join("crates/pkg/__pycache__/x.pyo");
    fs::rename(&pyc, &pyo).unwrap();
    let renamed = fingerprint(&root, &run);
    assert_ne!(excluded["aggregate_sha256"], renamed["aggregate_sha256"]);
    assert!(paths(&renamed).contains(&"crates/pkg/__pycache__/x.pyo"));
    fs::remove_file(pyo).unwrap();
    assert_ne!(
        renamed["aggregate_sha256"],
        fingerprint(&root, &run)["aggregate_sha256"]
    );
}
