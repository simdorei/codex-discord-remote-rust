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
    fs::copy(
        repo.join("codex-discord-watchdog-identity-runtime.ps1"),
        temp.path()
            .join("codex-discord-watchdog-identity-runtime.ps1"),
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
fn cutover_script_requires_explicit_manual_python_rollback() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = fs::read_to_string(repo.join("codex-discord-runtime-cutover.ps1")).unwrap();
    assert!(text.contains("Backup-Store"));
    assert!(text.contains("Enter-CutoverMaintenance"));
    assert!(text.contains("Stop-Python"));
    assert!(text.contains("Wait-RustHealthy"));
    let completion = fs::read_to_string(repo.join("scripts/CdrCutoverCompletion.ps1")).unwrap();
    assert!(completion.contains("automatic rollback disabled"));
    assert!(text.contains("-Runtime $($transaction.SourceRuntime)"));
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
    fs::copy(
        repo.join("codex-discord-watchdog-identity-runtime.ps1"),
        temp.path()
            .join("codex-discord-watchdog-identity-runtime.ps1"),
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
        "source_runtime=python\n",
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
    fs::copy(
        repo.join("codex-discord-watchdog-identity-runtime.ps1"),
        temp.path()
            .join("codex-discord-watchdog-identity-runtime.ps1"),
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
    fs::write(temp.path().join(".codex_discord_runtime"), "python\n").unwrap();
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
fn explicit_python_recovery_does_not_require_or_invoke_a_rust_binary() {
    for rust_binary_exists in [false, true] {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let script = repo.join("codex-discord-runtime-cutover.ps1");
        let temp = cutover_fixture();
        let python_running = temp.path().join("python-running.txt");
        let rust_invoked = temp.path().join("rust-invoked.txt");
        let binary_path = temp.path().join("broken-rust.cmd");
        if rust_binary_exists {
            fs::write(
                &binary_path,
                format!(
                    "@echo off\r\n>\"{}\" echo invoked\r\nexit /b 91\r\n",
                    rust_invoked.display()
                ),
            )
            .unwrap();
        }
        fs::write(
            temp.path()
                .join("codex-discord-watchdog-identity-runtime.ps1"),
            concat!(
                "function Get-CodexBotProcessIdentity {\n",
                "    param([string]$BotScript, [string]$RuntimeLockPath)\n",
                "    $running = Join-Path (Split-Path -Parent $BotScript) 'python-running.txt'\n",
                "    if (Test-Path -LiteralPath $running) { return '4242|1' }\n",
                "    return ''\n",
                "}\n"
            ),
        )
        .unwrap();
        fs::write(
            temp.path().join("codex-discord-watchdog.ps1"),
            "Set-Content -LiteralPath (Join-Path $PSScriptRoot 'python-running.txt') -Value running\nexit 0\n",
        )
        .unwrap();
        let transaction = concat!(
            "transaction_id=manual-python-recovery\n",
            "owner_identity=999999|0\n",
            "source_runtime=python\n",
            "target_runtime=rust\n",
            "phase=target_starting\n"
        );
        fs::write(
            temp.path().join(".codex_discord_runtime.cutover"),
            transaction,
        )
        .unwrap();
        fs::write(
            temp.path().join(".codex_discord_bot.disabled"),
            "kind=cutover_recovery\ntransaction_id=manual-python-recovery\n",
        )
        .unwrap();
        fs::write(temp.path().join(".codex_discord_runtime"), "rust\n").unwrap();

        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(script)
            .args(["-Runtime", "python", "-RepoRoot"])
            .arg(temp.path())
            .args(["-BinaryPath"])
            .arg(&binary_path)
            .args(["-EnvPath"])
            .arg(temp.path().join("missing.env"))
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "explicit Python recovery failed: stdout={stdout} stderr={stderr}"
        );
        assert!(stdout.contains("recovery=explicit"));
        assert!(python_running.exists());
        assert!(!rust_invoked.exists(), "Rust preflight was invoked");
        assert_eq!(
            fs::read_to_string(temp.path().join(".codex_discord_runtime")).unwrap(),
            "python\n"
        );
        assert!(!temp.path().join(".codex_discord_runtime.cutover").exists());
        assert!(!temp.path().join(".codex_discord_bot.disabled").exists());
    }
}

#[test]
fn ordinary_manual_python_rollback_backs_up_without_rust_preflight() {
    let python_output = Command::new("py")
        .args(["-3", "-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(python_output.status.success());
    let python_executable = String::from_utf8(python_output.stdout)
        .unwrap()
        .trim()
        .to_owned();

    for rust_binary_exists in [false, true] {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let script = repo.join("codex-discord-runtime-cutover.ps1");
        let temp = cutover_fixture();
        let database = temp.path().join("discord_mirror.sqlite");
        cdr_store::schema::open_initialized(&database).unwrap();
        let rust_invoked = temp.path().join("rust-invoked.txt");
        let binary_path = temp.path().join("broken-rust.cmd");
        if rust_binary_exists {
            fs::write(
                &binary_path,
                format!(
                    "@echo off\r\n>\"{}\" echo invoked\r\nexit /b 91\r\n",
                    rust_invoked.display()
                ),
            )
            .unwrap();
        }
        fs::write(
            temp.path()
                .join("codex-discord-watchdog-identity-runtime.ps1"),
            concat!(
                "function Get-CodexBotProcessIdentity {\n",
                "    param([string]$BotScript, [string]$RuntimeLockPath)\n",
                "    $running = Join-Path (Split-Path -Parent $BotScript) 'python-running.txt'\n",
                "    if (Test-Path -LiteralPath $running) { return '4242|1' }\n",
                "    return ''\n",
                "}\n"
            ),
        )
        .unwrap();
        fs::write(
            temp.path().join("codex-discord-watchdog.ps1"),
            "Set-Content -LiteralPath (Join-Path $PSScriptRoot 'python-running.txt') -Value running\nexit 0\n",
        )
        .unwrap();
        fs::write(temp.path().join("codex_discord_bot.py"), "").unwrap();
        fs::write(temp.path().join(".codex_discord_runtime"), "rust\n").unwrap();

        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(script)
            .args(["-Runtime", "python", "-RepoRoot"])
            .arg(temp.path())
            .args(["-BinaryPath"])
            .arg(&binary_path)
            .args(["-EnvPath"])
            .arg(temp.path().join("missing.env"))
            .env("CODEX_DISCORD_PYTHON", &python_executable)
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "manual Python rollback failed: stdout={stdout} stderr={stderr}"
        );
        assert!(stdout.contains("manual_rollback=verified"));
        assert!(!rust_invoked.exists(), "Rust preflight was invoked");
        assert_eq!(
            fs::read_to_string(temp.path().join(".codex_discord_runtime")).unwrap(),
            "python\n"
        );
        let backup_directory = temp.path().join(".codex-discord-backups");
        let backups = fs::read_dir(&backup_directory)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(backups.len(), 1);
        let backup = rusqlite::Connection::open(backups[0].path()).unwrap();
        let integrity: String = backup
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        assert!(temp.path().join("python-running.txt").exists());
        assert!(!temp.path().join(".codex_discord_runtime.cutover").exists());
        assert!(!temp.path().join(".codex_discord_bot.disabled").exists());
    }
}

#[test]
fn cutover_heartbeat_is_pid_bound_with_bounded_bootstrap_grace() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = fs::read_to_string(repo.join("codex-discord-runtime-cutover.ps1")).unwrap();

    assert!(text.contains("HeartbeatBootstrapGraceSeconds"));
    assert!(text.contains("Get-VerifiedRustHeartbeatState"));
    assert!(text.contains("heartbeat_pid_mismatch"));
    assert!(text.contains("target_healthy"));
}

#[test]
fn cutover_has_a_durable_transaction_and_final_recovery_guard() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = fs::read_to_string(repo.join("codex-discord-runtime-cutover.ps1")).unwrap();

    assert!(text.contains(".codex_discord_runtime.cutover"));
    assert!(text.contains("Recover-InterruptedCutover"));
    assert!(text.contains("Ensure-CutoverRecoveryDisabled"));
    assert!(text.contains("source_stopped"));
    assert!(text.contains("target_starting"));
    assert!(text.contains("finally"));
}
