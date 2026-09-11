#![cfg(windows)]
use std::{fs, path::Path, process::Command};

#[test]
fn rust_to_rust_interruption_requires_an_explicit_recovery_request() {
    let root = tempfile::tempdir().unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::copy(
        repo.join("codex-discord-rust-control.ps1"),
        root.path().join("codex-discord-rust-control.ps1"),
    )
    .unwrap();
    let state = "transaction_id=test-transaction\nowner_identity=999999|0\nsource_runtime=rust\ntarget_runtime=rust\nphase=source_stopped\n";
    let disabled = "kind=cutover\ntransaction_id=test-transaction\n";
    fs::write(root.path().join(".codex_discord_runtime.cutover"), state).unwrap();
    fs::write(root.path().join(".codex_discord_bot.disabled"), disabled).unwrap();
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(repo.join("codex-discord-runtime-cutover.ps1"))
        .arg("-RepoRoot")
        .arg(root.path())
        .arg("-DryRun")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("would_require_explicit_cutover_recovery")
    );
    assert_eq!(
        fs::read_to_string(root.path().join(".codex_discord_runtime.cutover")).unwrap(),
        state
    );
    assert_eq!(
        fs::read_to_string(root.path().join(".codex_discord_bot.disabled")).unwrap(),
        disabled
    );
}

#[test]
fn cutover_has_no_interpreter_or_removed_runtime_recovery_path() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = fs::read_to_string(repo.join("codex-discord-runtime-cutover.ps1")).unwrap();
    for needle in [
        "PythonRuntime",
        "PythonBackupScript",
        "Stop-Python",
        "Start-Python",
        "'rust', 'python'",
    ] {
        assert!(
            !source.contains(needle),
            "removed runtime remains: {needle}"
        );
    }
}
