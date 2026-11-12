use std::fmt::Write as _;
use std::fs;
use std::process::{Command, Output};

fn headless_computer_startup(raw: Option<&str>, check_config: bool) -> Output {
    let directory = tempfile::tempdir().unwrap();
    let env_path = directory.path().join("runtime.env");
    let mut environment = format!(
        "DISCORD_BOT_TOKEN=do-not-print-me\nDISCORD_ALLOWED_CHANNEL_IDS=42\nCODEX_DISCORD_MIRROR_DB={}\nCODEX_HOME={}\nCODEX_EXE={}\n",
        directory.path().join("state/mirror.sqlite").display(),
        directory.path().join("codex-home").display(),
        directory.path().join("missing-codex.exe").display(),
    );
    if let Some(raw) = raw {
        writeln!(environment, "DISCORD_HEADLESS_COMPUTER_USE={raw}").unwrap();
    }
    fs::write(&env_path, environment).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_cdr-runtime"));
    if check_config {
        command.arg("--check-config");
    }
    let output = command
        .arg("--env")
        .arg(&env_path)
        .current_dir(directory.path())
        .env_clear()
        .output()
        .unwrap();

    assert_eq!(
        fs::read_dir(directory.path()).unwrap().count(),
        1,
        "configuration validation must not create runtime state"
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("do-not-print-me"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("do-not-print-me"));
    output
}

#[test]
fn check_config_rejects_invalid_headless_computer_use_just_like_startup() {
    for raw in ["maybe", "TRUE", "yes", "2"] {
        for check_config in [false, true] {
            let output = headless_computer_startup(Some(raw), check_config);
            assert_eq!(output.status.code(), Some(1));
            assert!(output.stdout.is_empty());
            assert_eq!(
                String::from_utf8(output.stderr).unwrap().trim(),
                "ERROR: DISCORD_HEADLESS_COMPUTER_USE must be 0, 1, false, or true"
            );
        }
    }
}

#[test]
fn check_config_reports_effective_headless_computer_use_without_starting_runtime() {
    for (raw, expected) in [
        (None, cfg!(windows)),
        (Some(""), cfg!(windows)),
        (Some("   "), cfg!(windows)),
        (Some("0"), false),
        (Some(" false "), false),
    ] {
        let output = headless_computer_startup(raw, true);
        assert!(output.status.success(), "raw value: {raw:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("config_valid"));
        assert!(stdout.contains(&format!("headless_computer_use={expected}")));
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn check_config_enforces_headless_computer_use_platform_support() {
    for raw in ["1", " true "] {
        let output = headless_computer_startup(Some(raw), true);
        if cfg!(windows) {
            assert!(output.status.success());
            assert!(
                String::from_utf8(output.stdout)
                    .unwrap()
                    .contains("headless_computer_use=true")
            );
            assert!(output.stderr.is_empty());
        } else {
            assert_eq!(output.status.code(), Some(1));
            assert!(output.stdout.is_empty());
            assert_eq!(
                String::from_utf8(output.stderr).unwrap().trim(),
                "ERROR: DISCORD_HEADLESS_COMPUTER_USE requires Windows"
            );
            let startup = headless_computer_startup(Some(raw), false);
            assert_eq!(startup.status.code(), Some(1));
            assert!(
                String::from_utf8(startup.stderr)
                    .unwrap()
                    .contains("DISCORD_HEADLESS_COMPUTER_USE requires Windows")
            );
        }
    }
}

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

    assert!(source.contains(".app_server_config(startup_deadline)"));
    let config_source = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/runtime_paths.rs"),
    )
    .unwrap();
    assert!(config_source.contains("AppServerConfig::new(&self.codex_exe).with_environment"));
    assert!(config_source.contains("\"CODEX_HOME\".into()"));
    assert!(config_source.contains("self.codex_home.to_string_lossy().into_owned()"));
}
