#![cfg(windows)]
use std::{fs, path::Path, process::Command};

#[test]
fn custom_binary_cannot_disguise_a_different_cargo_build_destination() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("scripts")).unwrap();
    fs::copy(source.join("install.ps1"), root.path().join("install.ps1")).unwrap();
    for entry in fs::read_dir(source.join("scripts")).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().is_some_and(|ext| ext == "psm1") {
            fs::copy(
                entry.path(),
                root.path().join("scripts").join(entry.file_name()),
            )
            .unwrap();
        }
    }
    let installed = root.path().join("target/release/cdr-runtime.exe");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::write(&installed, b"running artifact sentinel").unwrap();
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.path().join("install.ps1"))
        .args(["-DryRun", "-SkipEnvFile", "-SkipCodexPlugin", "-BinaryPath"])
        .arg(root.path().join("different-build/release/cdr-runtime.exe"))
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "custom path was incorrectly accepted as the cargo output"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("CARGO_TARGET_DIR"));
    assert_eq!(fs::read(installed).unwrap(), b"running artifact sentinel");
    assert!(!root.path().join(".codex_discord_runtime").exists());
}

#[test]
fn external_build_is_installed_at_launcher_path_without_overwriting_an_existing_version() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("scripts")).unwrap();
    fs::copy(source.join("install.ps1"), root.path().join("install.ps1")).unwrap();
    for item in fs::read_dir(source.join("scripts")).unwrap() {
        let item = item.unwrap();
        if item.path().extension().is_some_and(|value| value == "psm1") {
            fs::copy(
                item.path(),
                root.path().join("scripts").join(item.file_name()),
            )
            .unwrap();
        }
    }
    let installed = root.path().join("target/release/cdr-runtime.exe");
    let run = || {
        Command::new("powershell.exe")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(root.path().join("install.ps1"))
            .args([
                "-SkipBuild",
                "-SkipEnvFile",
                "-SkipCodexPlugin",
                "-BinaryPath",
            ])
            .arg(env!("CARGO_BIN_EXE_cdr-runtime"))
            .arg("-CodexExe")
            .arg(env!("CARGO_BIN_EXE_cdr-runtime"))
            .output()
            .unwrap()
    };
    let first = run();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(
        fs::read(&installed).unwrap(),
        fs::read(env!("CARGO_BIN_EXE_cdr-runtime")).unwrap()
    );
    assert!(run().status.success(), "same build should be idempotent");
    fs::write(&installed, b"previous verified version").unwrap();
    let marker = root.path().join(".codex_discord_runtime");
    fs::write(&marker, "original selection").unwrap();
    let conflict = run();
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("verified deployment"));
    assert_eq!(fs::read(&installed).unwrap(), b"previous verified version");
    assert_eq!(fs::read_to_string(marker).unwrap(), "original selection");
    assert!(!root.path().join(".codex_discord_rust.stop").exists());
}
