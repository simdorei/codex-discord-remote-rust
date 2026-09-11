use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn run(root: &Path, cwd: &Path, home: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cdr-runtime"));
    command
        .args(["--admin", "backup-store", "--repo-root"])
        .arg(root)
        .current_dir(cwd)
        .env_clear();
    if let Some(home) = home {
        command.env("USERPROFILE", home);
    }
    command.output().unwrap()
}

fn seed(path: &Path) {
    cdr_store::schema::open_initialized(path)
        .unwrap()
        .execute_batch(
            "CREATE TABLE preserved(value TEXT); INSERT INTO preserved VALUES('정확한 원본');",
        )
        .unwrap();
}

fn assert_backup(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let path = text.trim().strip_prefix("backup_created path=").unwrap();
    let value: String = rusqlite::Connection::open(path)
        .unwrap()
        .query_row("SELECT value FROM preserved", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "정확한 원본");
}

#[test]
fn backup_honors_configured_runtime_root_and_home_expansion_without_codex() {
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    seed(&data.path().join("discord_mirror.sqlite"));
    fs::write(
        config.path().join(".env"),
        format!("CODEX_DISCORD_ROOT={}\n", data.path().display()),
    )
    .unwrap();
    assert_backup(run(config.path(), config.path(), None));
    fs::write(config.path().join(".env"), "CODEX_DISCORD_ROOT=~\n").unwrap();
    assert_backup(run(config.path(), config.path(), Some(data.path())));
    fs::write(
        config.path().join(".env"),
        "CODEX_DISCORD_MIRROR_DB=~/discord_mirror.sqlite\n",
    )
    .unwrap();
    assert_backup(run(config.path(), config.path(), Some(data.path())));
}

#[test]
fn relative_database_uses_runtime_working_directory_not_configuration_directory() {
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    seed(&data.path().join("relative.sqlite"));
    fs::write(
        config.path().join(".env"),
        "CODEX_DISCORD_MIRROR_DB=relative.sqlite\n",
    )
    .unwrap();
    assert_backup(run(config.path(), data.path(), None));
}

#[test]
fn missing_home_for_tilde_is_visible_and_does_not_create_a_literal_tilde_directory() {
    let config = tempfile::tempdir().unwrap();
    fs::write(
        config.path().join(".env"),
        "CODEX_DISCORD_MIRROR_DB=~/mirror.sqlite\n",
    )
    .unwrap();
    let output = run(config.path(), config.path(), None);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("home"));
    assert!(!config.path().join("~").exists());
}
