use std::collections::BTreeSet;

use cdr_store::schema::{assert_integrity, open_initialized};

#[test]
fn prompt_intake_extension_repairs_every_column_and_index() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("partial-prompt-intake.sqlite");
    {
        let conn = open_initialized(&path).expect("initialize Rust store");
        conn.execute_batch(
            "DROP TABLE codex_prompt_intakes;\
             CREATE TABLE codex_prompt_intakes (\
                job_id TEXT PRIMARY KEY, target_thread_id TEXT NOT NULL\
             );",
        )
        .expect("replace prompt intake with partial legacy extension");
    }

    let conn = open_initialized(&path).expect("repair partial prompt intake extension");
    let mut statement = conn
        .prepare("PRAGMA table_info(codex_prompt_intakes)")
        .expect("prepare prompt intake column query");
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query prompt intake columns")
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .expect("collect prompt intake columns");
    for required in [
        "job_id",
        "target_thread_id",
        "channel_id",
        "owner_user_id",
        "discord_message_id",
        "raw_prompt",
        "auto_queue_when_busy",
        "require_current_mirror",
        "attempt_count",
        "last_error",
        "retry_after",
        "claim_token",
        "claim_expires_at",
        "created_at",
        "updated_at",
    ] {
        assert!(
            columns.contains(required),
            "missing repaired column {required}"
        );
    }
    drop(statement);

    let indexes = sqlite_object_names(&conn, "index");
    assert!(indexes.contains("codex_prompt_intakes_message_id"));
    assert!(indexes.contains("codex_prompt_intakes_target_ready"));
    assert_integrity(&conn).expect("repaired prompt intake integrity");
}

fn sqlite_object_names(connection: &rusqlite::Connection, kind: &str) -> BTreeSet<String> {
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type = ? AND name NOT LIKE 'sqlite_%'")
        .expect("prepare sqlite_schema query");
    statement
        .query_map([kind], |row| row.get(0))
        .expect("query sqlite_schema")
        .collect::<rusqlite::Result<_>>()
        .expect("collect sqlite_schema names")
}
