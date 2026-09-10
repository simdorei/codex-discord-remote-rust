use std::fs;
use std::process::Command;

#[test]
fn check_config_uses_env_file_and_never_prints_the_token() {
    let directory = tempfile::tempdir().unwrap();
    let env_path = directory.path().join("runtime.env");
    fs::write(
        &env_path,
        "DISCORD_BOT_TOKEN=do-not-print-me\nDISCORD_ALLOWED_CHANNEL_IDS=42\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cdr-runtime"))
        .args(["--check-config", "--env"])
        .arg(&env_path)
        .env_clear()
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "backup command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("config_valid"));
    assert!(stdout.contains("channels={42}"));
    assert!(stdout.contains("token=[REDACTED]"));
    assert!(!stdout.contains("do-not-print-me"));
    assert!(
        !String::from_utf8(output.stderr)
            .unwrap()
            .contains("do-not-print-me")
    );
}

#[test]
fn missing_required_config_returns_one_with_actionable_error() {
    let directory = tempfile::tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cdr-runtime"))
        .args(["--check-config", "--env"])
        .arg(directory.path().join("missing.env"))
        .env_clear()
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("missing required environment variable: DISCORD_BOT_TOKEN"));
}

#[test]
fn unknown_cli_argument_returns_two_instead_of_silently_falling_back() {
    let output = Command::new(env!("CARGO_BIN_EXE_cdr-runtime"))
        .arg("--unknown")
        .env_clear()
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("unknown argument: --unknown")
    );
}

#[test]
fn backup_store_creates_an_integrity_checked_snapshot_without_starting_discord() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("mirror.sqlite");
    cdr_store::schema::open_initialized(&database).unwrap();
    let env_path = directory.path().join("runtime.env");
    fs::write(
        &env_path,
        format!(
            "DISCORD_BOT_TOKEN=do-not-print-me\nDISCORD_ALLOWED_CHANNEL_IDS=42\nCODEX_DISCORD_MIRROR_DB={}\nCODEX_EXE={}\n",
            database.display(),
            std::env::current_exe().unwrap().display()
        ),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cdr-runtime"))
        .args(["--backup-store", "--env"])
        .arg(env_path)
        .env_clear()
        .env(
            if cfg!(windows) { "USERPROFILE" } else { "HOME" },
            directory.path(),
        )
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "backup command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("backup_created path="));
    assert!(!stdout.contains("do-not-print-me"));
    assert_eq!(
        std::fs::read_dir(directory.path().join(".codex-discord-backups"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn resolved_codex_home_is_forwarded_to_the_app_server_process() {
    let source = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/discord_runtime.rs"),
    )
    .unwrap();

    assert!(source.contains("AppServerConfig::new(&paths.codex_exe).with_environment"));
    assert!(source.contains("\"CODEX_HOME\".to_owned()"));
    assert!(source.contains("paths.codex_home.to_string_lossy().into_owned()"));
}
