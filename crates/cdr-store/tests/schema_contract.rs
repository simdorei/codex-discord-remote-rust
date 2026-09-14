use std::collections::BTreeSet;

use cdr_store::StoreError;
use cdr_store::schema::{LATEST_STORE_SCHEMA_VERSION, assert_integrity, open_initialized};
use rusqlite::Connection;

const OWNED_TABLES: [&str; 30] = [
    "busy_choices",
    "cdr_archive_fences",
    "cdr_async_question_inbox",
    "cdr_async_questions",
    "cdr_cleanup_fences",
    "cdr_idle_release",
    "codex_app_server_runtime",
    "codex_archive_fences",
    "codex_dead_generation_holds",
    "codex_dead_generation_incidents",
    "codex_session_mirror_events",
    "codex_session_mirror_offsets",
    "codex_turn_queue",
    "codex_delivery_outbox",
    "codex_commentary_outbox",
    "codex_delivery_receipts",
    "codex_goal_progress",
    "codex_new_first_replies",
    "codex_observed_completions",
    "codex_observed_final_answers",
    "codex_busy_control_bindings",
    "codex_prompt_intakes",
    "codex_request_cancellations",
    "discord_ingress_journal",
    "discord_ingress_owner_receipts",
    "discord_processed_messages",
    "mirror_projects",
    "mirror_threads",
    "persistent_component_claims",
    "session_mirror_details",
];

const OWNED_INDEXES: [&str; 11] = [
    "cdr_async_question_inbox_pending",
    "cdr_async_question_pending",
    "cdr_async_question_reply_job",
    "codex_cancelled_message",
    "codex_new_first_replies_pending",
    "codex_prompt_intakes_message_id",
    "codex_prompt_intakes_target_ready",
    "codex_turn_queue_message_id",
    "codex_turn_queue_target_order",
    "discord_ingress_event",
    "discord_ingress_owner",
];

fn object_names(conn: &Connection, kind: &str) -> BTreeSet<String> {
    let mut statement = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = ? AND name NOT LIKE 'sqlite_%'")
        .expect("prepare sqlite_schema query");
    statement
        .query_map([kind], |row| row.get(0))
        .expect("query sqlite_schema")
        .collect::<rusqlite::Result<_>>()
        .expect("collect sqlite_schema names")
}

fn schema_objects(conn: &Connection) -> BTreeSet<(String, String, String)> {
    let mut statement = conn
        .prepare(
            "SELECT type, name, sql FROM sqlite_schema \
             WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )
        .expect("prepare full schema query");
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("query full schema")
        .collect::<rusqlite::Result<_>>()
        .expect("collect full schema")
}

#[test]
fn s1_new_database_matches_shared_schema_and_rust_extensions() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("mirror.sqlite");
    let conn = open_initialized(&path).expect("initialize Rust store");
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("read schema version");
    assert_eq!(version, LATEST_STORE_SCHEMA_VERSION);
    assert_eq!(
        object_names(&conn, "table"),
        OWNED_TABLES.map(String::from).into()
    );
    assert_eq!(
        object_names(&conn, "index"),
        OWNED_INDEXES.map(String::from).into()
    );
    let timeout: i64 = conn
        .pragma_query_value(None, "busy_timeout", |row| row.get(0))
        .expect("read busy timeout");
    assert_eq!(timeout, 5_000);
    assert_integrity(&conn).expect("new store integrity");
}

#[test]
fn s2_existing_seventeen_table_store_preserves_foreign_objects_and_data() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("mirror.sqlite");
    let before = {
        let conn = Connection::open(&path).expect("create fixture database");
        conn.execute_batch(include_str!(
            "../../../fixtures/parity/discord_mirror_schema_v2.sql"
        ))
        .expect("load 17-table fixture");
        conn.execute(
            "INSERT INTO chatgpt_app_mirror_conversations (conversation_id, primed_at) \
             VALUES ('foreign-sentinel', 42.0)",
            [],
        )
        .expect("insert foreign sentinel");
        let objects = schema_objects(&conn);
        assert_eq!(object_names(&conn, "table").len(), 17);
        objects
    };
    let conn = open_initialized(&path).expect("open existing production-shaped store");
    let after = schema_objects(&conn);
    assert!(before.iter().all(|(kind, name, _)| {
        after
            .iter()
            .any(|(after_kind, after_name, _)| after_kind == kind && after_name == name)
    }));
    assert!(object_names(&conn, "table").contains("codex_delivery_outbox"));
    let primed_at: f64 = conn
        .query_row(
            "SELECT primed_at FROM chatgpt_app_mirror_conversations \
             WHERE conversation_id = 'foreign-sentinel'",
            [],
            |row| row.get(0),
        )
        .expect("foreign sentinel survives");
    assert!((primed_at - 42.0).abs() < f64::EPSILON);
    assert_integrity(&conn).expect("existing store integrity");
}

#[test]
fn s3_version_one_store_is_backed_up_before_queue_migration() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("legacy.sqlite");
    {
        let conn = Connection::open(&path).expect("create legacy database");
        conn.execute_batch(
            "CREATE TABLE codex_turn_queue (\
                job_id TEXT PRIMARY KEY, target_thread_id TEXT NOT NULL, \
                channel_id INTEGER NOT NULL, owner_user_id INTEGER, \
                discord_message_id INTEGER, prompt TEXT NOT NULL, queued INTEGER NOT NULL, \
                ack_sent INTEGER NOT NULL, state TEXT NOT NULL, attempt_count INTEGER NOT NULL, \
                turn_id TEXT, baseline_turn_ids TEXT NOT NULL, last_error TEXT NOT NULL DEFAULT '', \
                created_at REAL NOT NULL, updated_at REAL NOT NULL\
             );\
             INSERT INTO codex_turn_queue VALUES (\
                'legacy-job', 'thread-1', 7, NULL, 8, 'hello', 1, 0, 'pending', 0, \
                NULL, '[]', '', 10.0, 10.0\
             );\
             PRAGMA user_version = 1;",
        )
        .expect("build version-one fixture");
    }

    let conn = open_initialized(&path).expect("migrate legacy store");
    let generation: i64 = conn
        .query_row(
            "SELECT app_server_generation FROM codex_turn_queue WHERE job_id = 'legacy-job'",
            [],
            |row| row.get(0),
        )
        .expect("legacy row receives generation");
    assert_eq!(generation, 0);

    let backup_dir = temp.path().join(".codex-discord-backups");
    let backups: Vec<_> = std::fs::read_dir(backup_dir)
        .expect("migration backup directory")
        .collect::<std::io::Result<_>>()
        .expect("migration backup entries");
    assert_eq!(backups.len(), 1);
    assert!(
        backups[0]
            .file_name()
            .to_string_lossy()
            .contains(".v1-to-v2.")
    );
    let backup = Connection::open(backups[0].path()).expect("open migration backup");
    let legacy_count: i64 = backup
        .query_row(
            "SELECT COUNT(*) FROM codex_turn_queue WHERE job_id = 'legacy-job'",
            [],
            |row| row.get(0),
        )
        .expect("legacy backup row");
    assert_eq!(legacy_count, 1);
    let has_generation: bool = backup
        .prepare("PRAGMA table_info(codex_turn_queue)")
        .expect("prepare backup column query")
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query backup columns")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect backup columns")
        .iter()
        .any(|column| column == "app_server_generation");
    assert!(!has_generation);
}

#[test]
fn s4_newer_schema_is_rejected_without_modification() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("future.sqlite");
    {
        let conn = Connection::open(&path).expect("create future database");
        conn.execute_batch(
            "CREATE TABLE future_data (value TEXT NOT NULL);\
             INSERT INTO future_data VALUES ('keep-me');\
             PRAGMA user_version = 5;",
        )
        .expect("build future fixture");
    }

    let error = open_initialized(&path).expect_err("future schema must be rejected");
    assert!(matches!(
        error,
        StoreError::UnsupportedVersion {
            found: 5,
            supported: LATEST_STORE_SCHEMA_VERSION
        }
    ));
    let conn = Connection::open(path).expect("reopen future database");
    let value: String = conn
        .query_row("SELECT value FROM future_data", [], |row| row.get(0))
        .expect("future sentinel remains");
    assert_eq!(value, "keep-me");
}

#[test]
fn s5_failed_migration_rolls_back_and_keeps_the_pre_migration_backup() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("broken-v1.sqlite");
    {
        let conn = Connection::open(&path).expect("create broken legacy database");
        conn.execute_batch(
            "CREATE TABLE sentinel (value TEXT NOT NULL);\
             INSERT INTO sentinel VALUES ('keep-me');\
             CREATE VIEW codex_turn_queue AS SELECT 'not-a-table' AS job_id;\
             PRAGMA user_version = 1;",
        )
        .expect("build migration failure fixture");
    }
    open_initialized(&path).expect_err("altering a view must fail migration");
    let conn = Connection::open(&path).expect("reopen rolled-back database");
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("read rolled-back version");
    assert_eq!(version, 1);
    let sentinel: String = conn
        .query_row("SELECT value FROM sentinel", [], |row| row.get(0))
        .expect("sentinel survives rollback");
    assert_eq!(sentinel, "keep-me");
    let view_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'view' AND name = 'codex_turn_queue'",
            [],
            |row| row.get(0),
        )
        .expect("legacy view survives rollback");
    assert_eq!(view_count, 1);
    let backup_count = std::fs::read_dir(temp.path().join(".codex-discord-backups"))
        .expect("failed migration backup directory")
        .count();
    assert_eq!(backup_count, 1);
}
