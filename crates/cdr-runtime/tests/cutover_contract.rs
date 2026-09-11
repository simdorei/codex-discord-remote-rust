#![cfg(windows)]

use std::fs;
use std::process::Command;

fn cutover_fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::copy(
        repo.join("codex-discord-rust-control.ps1"),
        temp.path().join("codex-discord-rust-control.ps1"),
    )
    .unwrap();
    temp
}

#[test]
fn cutover_dry_run_is_non_mutating_and_declares_backup_and_observation() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = repo.join("codex-discord-runtime-cutover.ps1");
    let temp = cutover_fixture();
    let env_path = temp.path().join("runtime.env");
    fs::write(
        &env_path,
        "DISCORD_BOT_TOKEN=do-not-print-me\nDISCORD_ALLOWED_CHANNEL_IDS=42\n",
    )
    .unwrap();
    for name in [
        "codex-discord-watchdog.ps1",
        "codex-discord-rust-watchdog.ps1",
    ] {
        fs::write(temp.path().join(name), "exit 0\n").unwrap();
    }
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script)
        .arg("-RepoRoot")
        .arg(temp.path())
        .args(["-BinaryPath"])
        .arg(env!("CARGO_BIN_EXE_cdr-runtime"))
        .args(["-EnvPath"])
        .arg(&env_path)
        .args(["-ObserveSeconds", "1", "-DryRun"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("would_create_verified_online_store_backup"));
    assert!(stdout.contains("would_switch_runtime target=rust observe_seconds=1"));
    assert!(!stdout.contains("do-not-print-me"));
    assert!(!temp.path().join(".codex_discord_runtime").exists());
    assert!(!temp.path().join(".codex-discord-backups").exists());
}

#[test]
fn cutover_script_requires_explicit_rust_recovery() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = fs::read_to_string(repo.join("codex-discord-runtime-cutover.ps1")).unwrap();
    assert!(text.contains("Backup-Store"));
    assert!(text.contains("Enter-CutoverMaintenance"));
    assert!(text.contains("Stop-Rust"));
    assert!(text.contains("Wait-RustHealthy"));
    let completion = fs::read_to_string(repo.join("scripts/CdrCutoverCompletion.ps1")).unwrap();
    assert!(completion.contains("automatic rollback disabled"));
    let recovery = fs::read_to_string(repo.join("scripts/CdrCutoverRecovery.ps1")).unwrap();
    assert!(recovery.contains("-Runtime rust -Recover"));
    assert!(recovery.contains("if (-not $Recover)"));
    assert!(!text.contains("rollback=starting"));
    assert!(!text.contains("Python rollback was restored"));
}

#[test]
fn interrupted_cutover_is_durable_and_dry_run_requires_explicit_source_recovery() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = repo.join("codex-discord-runtime-cutover.ps1");
    let temp = cutover_fixture();
    let env_path = temp.path().join("runtime.env");
    fs::write(
        &env_path,
        "DISCORD_BOT_TOKEN=do-not-print-me\nDISCORD_ALLOWED_CHANNEL_IDS=42\n",
    )
    .unwrap();
    for name in [
        "codex-discord-watchdog.ps1",
        "codex-discord-rust-watchdog.ps1",
    ] {
        fs::write(temp.path().join(name), "exit 0\n").unwrap();
    }
    let state_path = temp.path().join(".codex_discord_runtime.cutover");
    let disabled_path = temp.path().join(".codex_discord_bot.disabled");
    let state = concat!(
        "transaction_id=test-transaction\n",
        "owner_identity=999999|0\n",
        "source_runtime=rust\n",
        "target_runtime=rust\n",
        "phase=source_stopped\n"
    );
    let disabled = "kind=cutover\ntransaction_id=test-transaction\n";
    fs::write(&state_path, state).unwrap();
    fs::write(&disabled_path, disabled).unwrap();

    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script)
        .args(["-Runtime", "rust", "-RepoRoot"])
        .arg(temp.path())
        .args(["-BinaryPath"])
        .arg(env!("CARGO_BIN_EXE_cdr-runtime"))
        .args(["-EnvPath"])
        .arg(&env_path)
        .args(["-ObserveSeconds", "1", "-DryRun"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("would_require_explicit_cutover_recovery")
    );
    assert_eq!(fs::read_to_string(state_path).unwrap(), state);
    assert_eq!(fs::read_to_string(disabled_path).unwrap(), disabled);
}

#[test]
fn failed_rust_cutover_stays_disabled_and_never_invokes_python_watchdog() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = repo.join("codex-discord-runtime-cutover.ps1");
    let temp = cutover_fixture();
    let database = temp.path().join("mirror.sqlite");
    cdr_store::schema::open_initialized(&database).unwrap();
    let env_path = temp.path().join("runtime.env");
    fs::write(
        &env_path,
        format!(
            concat!(
                "DISCORD_BOT_TOKEN=do-not-print-me\n",
                "DISCORD_ALLOWED_CHANNEL_IDS=42\n",
                "CODEX_DISCORD_MIRROR_DB={}\n",
                "CODEX_EXE={}\n"
            ),
            database.display(),
            std::env::current_exe().unwrap().display()
        ),
    )
    .unwrap();
    let python_watchdog = temp.path().join("codex-discord-watchdog.ps1");
    fs::write(
        &python_watchdog,
        "Set-Content -LiteralPath $env:CUTOVER_PYTHON_SENTINEL -Value invoked\nexit 9\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("codex-discord-rust-watchdog.ps1"),
        "exit 0\n",
    )
    .unwrap();
    fs::write(temp.path().join(".codex_discord_runtime"), "rust\n").unwrap();
    let sentinel = temp.path().join("python-watchdog-invoked.txt");

    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script)
        .args(["-Runtime", "rust", "-RepoRoot"])
        .arg(temp.path())
        .args(["-BinaryPath"])
        .arg(env!("CARGO_BIN_EXE_cdr-runtime"))
        .args(["-EnvPath"])
        .arg(&env_path)
        .args(["-ObserveSeconds", "0"])
        .env("CUTOVER_PYTHON_SENTINEL", &sentinel)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "cutover unexpectedly passed: {stdout}"
    );
    assert!(
        stdout.contains("automatic_rollback=disabled")
            || stderr.contains("automatic rollback disabled"),
        "missing fail-closed diagnostic: stdout={stdout} stderr={stderr}"
    );
    assert!(
        !sentinel.exists(),
        "Python watchdog was invoked after a failed Rust cutover"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join(".codex_discord_runtime")).unwrap(),
        "rust\n"
    );
    assert!(temp.path().join(".codex_discord_runtime.cutover").exists());
    assert!(temp.path().join(".codex_discord_bot.disabled").exists());
}

#[test]
fn explicit_rust_recovery_preserves_state_when_preflight_is_unusable() {
    for exists in [false, true] {
        let temp = cutover_fixture();
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let binary = temp.path().join("broken-rust.cmd");
        let sentinel = temp.path().join("started");
        if exists {
            fs::write(&binary, "@echo off\r\nexit /b 91\r\n").unwrap();
        }
        fs::write(
            temp.path().join("runtime.env"),
            "DISCORD_ALLOWED_CHANNEL_IDS=42\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("codex-discord-rust-watchdog.ps1"),
            format!(
                "[IO.File]::WriteAllText('{}','started')",
                sentinel.display()
            ),
        )
        .unwrap();
        let state = "transaction_id=recover\nowner_identity=999999|0\nsource_runtime=rust\ntarget_runtime=rust\nphase=source_stopped\n";
        let disabled = "kind=cutover_recovery\ntransaction_id=recover\n";
        fs::write(temp.path().join(".codex_discord_runtime.cutover"), state).unwrap();
        fs::write(temp.path().join(".codex_discord_bot.disabled"), disabled).unwrap();
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(repo.join("codex-discord-runtime-cutover.ps1"))
            .arg("-RepoRoot")
            .arg(temp.path())
            .arg("-BinaryPath")
            .arg(&binary)
            .arg("-EnvPath")
            .arg(temp.path().join("runtime.env"))
            .arg("-Recover")
            .output()
            .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("Rust configuration preflight failed with exit code 91")
                || stderr.contains("Rust cutover prerequisite was not found"),
            "{stderr}"
        );
        assert!(!sentinel.exists());
        assert_eq!(
            fs::read_to_string(temp.path().join(".codex_discord_runtime.cutover")).unwrap(),
            state
        );
        assert!(temp.path().join(".codex_discord_bot.disabled").exists());
    }
}

#[test]
fn removed_runtime_rollback_is_rejected_before_data_or_runtime_changes() {
    for exists in [false, true] {
        let temp = cutover_fixture();
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let binary = temp.path().join("broken-rust.cmd");
        let invoked = temp.path().join("invoked");
        if exists {
            fs::write(
                &binary,
                format!(
                    "@echo off\r\n>\"{}\" echo invoked\r\nexit /b 91\r\n",
                    invoked.display()
                ),
            )
            .unwrap();
        }
        let database = temp.path().join("discord_mirror.sqlite");
        cdr_store::schema::open_initialized(&database).unwrap();
        let before = fs::read(&database).unwrap();
        fs::write(temp.path().join(".codex_discord_runtime"), "rust\n").unwrap();
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(repo.join("codex-discord-runtime-cutover.ps1"))
            .args(["-Runtime", "python", "-RepoRoot"])
            .arg(temp.path())
            .arg("-BinaryPath")
            .arg(binary)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("ValidateSet"));
        assert!(!invoked.exists());
        assert_eq!(fs::read(database).unwrap(), before);
        assert_eq!(
            fs::read_to_string(temp.path().join(".codex_discord_runtime")).unwrap(),
            "rust\n"
        );
        assert!(!temp.path().join(".codex_discord_runtime.cutover").exists());
        assert!(!temp.path().join(".codex_discord_bot.disabled").exists());
    }
}

#[test]
fn cutover_heartbeat_is_pid_bound_with_bounded_bootstrap_grace() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = fs::read_to_string(repo.join("scripts/CdrCutoverRuntime.ps1")).unwrap();

    assert!(text.contains("HeartbeatBootstrapGraceSeconds"));
    assert!(text.contains("Get-VerifiedRustHeartbeatState"));
    assert!(text.contains("heartbeat_pid_mismatch"));
    assert!(text.contains("($now - $healthySince).TotalSeconds -ge $ObserveSeconds"));
}

#[test]
fn cutover_has_a_durable_transaction_and_final_recovery_guard() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = fs::read_to_string(repo.join("codex-discord-runtime-cutover.ps1")).unwrap();

    assert!(text.contains(".codex_discord_runtime.cutover"));
    assert!(text.contains("Recover-InterruptedCutover"));
    let recovery = fs::read_to_string(repo.join("scripts/CdrCutoverState.ps1")).unwrap();
    assert!(recovery.contains("Ensure-CutoverRecoveryDisabled"));
    assert!(text.contains("source_stopped"));
    assert!(text.contains("target_starting"));
    assert!(text.contains("finally"));
}
