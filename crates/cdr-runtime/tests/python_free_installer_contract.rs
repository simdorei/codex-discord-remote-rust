#![cfg(windows)]

use std::fs;
use std::path::Path;
use std::process::Command;

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target.join(entry.file_name()));
        } else if entry
            .path()
            .extension()
            .is_some_and(|ext| ext == "ps1" || ext == "psm1")
        {
            fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
fn install_dry_run_never_prepares_python_or_changes_files() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = tempfile::tempdir().unwrap();
    fs::copy(source.join("install.ps1"), root.path().join("install.ps1")).unwrap();
    fs::copy(
        source.join("rust-toolchain.toml"),
        root.path().join("rust-toolchain.toml"),
    )
    .unwrap();
    // No legacy manifest or dependency file is present in a native fresh install.
    copy_tree(&source.join("scripts"), &root.path().join("scripts"));
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.path().join("install.ps1"))
        .args(["-DryRun", "-SkipEnvFile", "-SkipCodexPlugin"])
        .env("PYTHON_EXE", root.path().join("no-python.exe"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Dry run complete"));
    assert!(
        !text.contains("Installing Python")
            && !text.contains("pip")
            && !text.contains("Invoke-Python")
    );
    assert!(!root.path().join(".env").exists());
    assert!(!root.path().join(".python-portable").exists());
    assert!(!root.path().join(".codex_discord_runtime").exists());
}

#[test]
fn setup_wrapper_dry_run_uses_rust_without_scheduling_or_saving_secrets() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = tempfile::tempdir().unwrap();
    copy_tree(&source.join("scripts"), &root.path().join("scripts"));
    for name in [
        "setup-discord-bot.ps1",
        "codex-discord-watchdog.ps1",
        "codex-discord-watchdog-hidden.vbs",
    ] {
        fs::copy(source.join(name), root.path().join(name)).unwrap();
    }
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.path().join("setup-discord-bot.ps1"))
        .args(["-RepoRoot"])
        .arg(root.path())
        .args(["-DryRun", "-BotId", "42", "-BinaryPath"])
        .arg(env!("CARGO_BIN_EXE_cdr-runtime"))
        .env("PYTHON_EXE", root.path().join("no-python.exe"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("client_id=42"));
    assert!(text.contains("Would register scheduled task"));
    assert!(!root.path().join(".env").exists());
}

#[test]
fn setup_without_repo_root_uses_script_location_not_callers_directory() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = tempfile::tempdir().unwrap();
    let caller = tempfile::tempdir().unwrap();
    copy_tree(&source.join("scripts"), &root.path().join("scripts"));
    for name in [
        "setup-discord-bot.ps1",
        "codex-discord-watchdog.ps1",
        "codex-discord-watchdog-hidden.vbs",
    ] {
        fs::copy(source.join(name), root.path().join(name)).unwrap();
    }
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.path().join("setup-discord-bot.ps1"))
        .args(["-DryRun", "-BotId", "42", "-BinaryPath"])
        .arg(env!("CARGO_BIN_EXE_cdr-runtime"))
        .current_dir(caller.path())
        .env("PYTHON_EXE", root.path().join("no-python.exe"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let expected_target = format!(
        "Would save DISCORD_BOT_TOKEN to: {}",
        root.path().join(".env").display()
    );
    assert!(text.lines().any(|line| line == expected_target), "{text}");
    assert!(text.contains("client_id=42"));
    assert!(text.contains("Would register scheduled task"));
    assert!(!root.path().join(".env").exists());
    assert_eq!(fs::read_dir(caller.path()).unwrap().count(), 0);
}
