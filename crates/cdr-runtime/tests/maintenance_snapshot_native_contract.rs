#![cfg(windows)]
#[path = "support/maintenance_native.rs"]
mod fixture;
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command};

fn isolated(command: &mut Command, root: &Path) {
    command.env_clear();
    for key in [
        "PATH",
        "SYSTEMROOT",
        "WINDIR",
        "TEMP",
        "TMP",
        "COMSPEC",
        "PATHEXT",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("USERPROFILE", root)
        .env("CODEX_HOME", root.join("codex-home"))
        .env("CODEX_EXE", root.join("codex.exe"))
        .env("CODEX_DISCORD_ROOT", root)
        .env(
            "CODEX_DISCORD_MIRROR_DB",
            root.join("discord_mirror.sqlite"),
        )
        .env("CODEX_STATE_DB", root.join("unused.sqlite"));
}

#[test]
fn real_candidate_and_maintenance_receipts_preserve_the_live_source() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("discord_mirror.sqlite");
    Connection::open(&source).unwrap().execute_batch(
        "CREATE TABLE synthetic_ticket(id INTEGER PRIMARY KEY, value TEXT); INSERT INTO synthetic_ticket VALUES(1,'retain me'); PRAGMA user_version=3;"
    ).unwrap();
    let before = Sha256::digest(fs::read(&source).unwrap());
    let config = root.path().join("fixture.env");
    fs::write(
        &config,
        "DISCORD_BOT_TOKEN=synthetic-not-a-real-token\nDISCORD_ALLOW_ALL_CHANNELS=1\n",
    )
    .unwrap();
    fs::write(
        root.path().join("codex.exe"),
        b"not executable; backup must not launch an app-server",
    )
    .unwrap();
    let binary = env!("CARGO_BIN_EXE_cdr-runtime");
    let mut command = Command::new(binary);
    isolated(&mut command, root.path());
    let output = command
        .arg("--backup-store")
        .arg("--env")
        .arg(&config)
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 1, "{text}");
    let backup = Path::new(text.trim().strip_prefix("backup_created path=").unwrap());
    assert_eq!(
        backup.parent().unwrap(),
        root.path().join(".codex-discord-backups")
    );
    let db = Connection::open_with_flags(backup, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        db.query_row::<String, _, _>("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        db.query_row::<i64, _, _>("PRAGMA user_version", [], |r| r.get(0))
            .unwrap(),
        3
    );
    assert_eq!(
        db.query_row::<(i64, String), _, _>("SELECT * FROM synthetic_ticket", [], |r| Ok((
            r.get(0)?,
            r.get(1)?
        )))
        .unwrap(),
        (1, "retain me".into())
    );
    assert_eq!(Sha256::digest(fs::read(&source).unwrap()), before);
    for name in [
        "unused.sqlite",
        ".codex_discord_rust.stop",
        ".codex_discord_rust.drain.identity",
    ] {
        assert!(!root.path().join(name).exists());
    }
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/maintenance");
    fixture::run_fixture_with(
        root.path(),
        &directory,
        "real_snapshot_receipts.ps1",
        "",
        |command| {
            isolated(command, root.path());
            command.env("CDR_TEST_RUNTIME_EXE", binary);
        },
    );
    assert_eq!(Sha256::digest(fs::read(source).unwrap()), before);
    assert!(!root.path().join(".codex_discord_rust.stop").exists());
    assert!(!root.path().join("unused.sqlite").exists());
    // Also exercise the default runner so both isolated harness entry points stay live.
    let failure_root = tempfile::tempdir().unwrap();
    fixture::run_fixture(
        failure_root.path(),
        &directory,
        "failure_observation_separates_liveness.ps1",
        "unknown",
    );
}
