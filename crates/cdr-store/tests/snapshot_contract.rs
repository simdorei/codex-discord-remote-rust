use cdr_store::backup::snapshot;
use cdr_store::schema::{LATEST_STORE_SCHEMA_VERSION, open_initialized};
use rusqlite::{Connection, OpenFlags};

#[test]
fn online_cutover_snapshot_is_integral_and_contains_committed_wal_rows() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("discord_mirror.sqlite");
    let connection = open_initialized(&database).unwrap();
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    connection
        .execute(
            "INSERT INTO discord_processed_messages(message_id, seen_at) VALUES (42, 1.0)",
            [],
        )
        .unwrap();

    let backup = snapshot(&database).unwrap();

    assert!(backup.starts_with(temp.path().join(".codex-discord-backups")));
    assert!(
        backup
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("-cutover")
    );
    let copied = Connection::open_with_flags(&backup, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        copied
            .pragma_query_value(None, "integrity_check", |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        copied
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        LATEST_STORE_SCHEMA_VERSION
    );
    assert_eq!(
        copied
            .query_row(
                "SELECT message_id FROM discord_processed_messages",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        42
    );
}

#[test]
fn missing_cutover_source_is_a_visible_error() {
    let temp = tempfile::tempdir().unwrap();
    let error = snapshot(&temp.path().join("missing.sqlite")).unwrap_err();
    assert!(error.to_string().contains("store database was not found"));
}
