#![cfg(windows)]

use std::fs::{self, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
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
        r"param([string]$ModulePath, [string]$RepoRoot, [string]$Mode)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
try {
    $module = Import-Module -Name $ModulePath -Force -PassThru
    if ($Mode -eq 'collision') {
        [Threading.Thread]::CurrentThread.CurrentCulture =
            [Globalization.CultureInfo]::GetCultureInfo('tr-TR')
        & $module {
            Assert-CodexSoakSourceLogicalPaths -Paths @(
                'crates/I.rs', 'crates/i.rs'
            )
        }
        @{ unexpected = 'collision accepted' } | ConvertTo-Json -Compress
    } else {
        Get-CodexSoakSourceFingerprint -RepoRoot $RepoRoot |
            ConvertTo-Json -Depth 8 -Compress
    }
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

fn invoke(root: &Path, run: &Path, mode: &str) -> Output {
    Command::new("powershell.exe")
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
        .arg("-Mode")
        .arg(mode)
        .output()
        .unwrap()
}

fn failure(root: &Path, run: &Path, mode: &str, code: &str) -> Value {
    let output = invoke(root, run, mode);
    assert_eq!(
        output.status.code(),
        Some(23),
        "expected fail-closed exit\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["code"], code, "failure was: {value}");
    assert!(
        value["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty())
    );
    value
}

fn basic_repo(root: &Path) {
    fs::create_dir_all(root.join("crates/core")).unwrap();
    fs::write(root.join("Cargo.toml"), b"workspace").unwrap();
    fs::write(root.join("Cargo.lock"), b"lock").unwrap();
    fs::write(root.join("rust-toolchain.toml"), b"toolchain").unwrap();
    fs::write(root.join("crates/core/lib.rs"), b"source").unwrap();
}

fn create_junction(script: &Path, junction: &Path, target: &Path) {
    fs::write(script, "param([string]$Link,[string]$Target)\n$ErrorActionPreference='Stop'\nNew-Item -ItemType Junction -Path $Link -Target $Target | Out-Null\n").unwrap();
    let created = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .arg("-Link")
        .arg(junction)
        .arg("-Target")
        .arg(target)
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "junction setup failed: {}",
        String::from_utf8_lossy(&created.stderr)
    );
}

#[test]
fn sf_missing_01_each_missing_required_root_file_fails_closed() {
    for required in ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        let run = temp.path().join("invoke.ps1");
        runner(&run);
        basic_repo(&root);
        fs::remove_file(root.join(required)).unwrap();
        let evidence = failure(&root, &run, "fingerprint", "source_required_missing");
        assert!(evidence["message"].as_str().unwrap().contains(required));
    }
}

#[test]
fn sf_reparse_01_directory_junction_is_rejected_instead_of_skipped_or_followed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    let run = temp.path().join("invoke.ps1");
    runner(&run);
    basic_repo(&root);
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("escaped.rs"), b"outside").unwrap();
    let junction = root.join("crates/linked-outside");
    create_junction(&temp.path().join("junction.ps1"), &junction, &outside);
    failure(&root, &run, "fingerprint", "source_reparse_point");
    fs::remove_dir(&junction).unwrap();
}

#[test]
fn sf_collision_01_case_colliding_paths_fail_independent_of_current_culture() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    let run = temp.path().join("invoke.ps1");
    runner(&run);
    failure(&root, &run, "collision", "source_path_collision");
}

#[test]
fn sf_read_01_exclusively_locked_source_file_is_not_silently_omitted() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    let run = temp.path().join("invoke.ps1");
    runner(&run);
    basic_repo(&root);
    let locked_path = root.join("crates/core/lib.rs");
    let guard = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&locked_path)
        .unwrap();
    failure(&root, &run, "fingerprint", "source_read_failed");
    drop(guard);
}

#[test]
fn sf_reparse_02_repository_root_junction_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real-repo");
    let link = temp.path().join("linked-repo");
    let run = temp.path().join("invoke.ps1");
    runner(&run);
    basic_repo(&real);
    create_junction(&temp.path().join("root-junction.ps1"), &link, &real);
    failure(&link, &run, "fingerprint", "source_reparse_point");
    fs::remove_dir(link).unwrap();
}

#[test]
fn sf_scope_02_missing_or_nondirectory_crates_scope_fails_closed() {
    for as_file in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        let run = temp.path().join("invoke.ps1");
        runner(&run);
        basic_repo(&root);
        fs::remove_dir_all(root.join("crates")).unwrap();
        if as_file {
            fs::write(root.join("crates"), b"not a directory").unwrap();
        }
        let code = if as_file {
            "source_scope_invalid"
        } else {
            "source_scope_missing"
        };
        failure(&root, &run, "fingerprint", code);
    }
}
