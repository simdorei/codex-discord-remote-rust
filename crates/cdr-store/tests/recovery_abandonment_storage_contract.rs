use cdr_store::{async_resolution::abandonment as storage, schema};
use rusqlite::{Connection, params};

const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, Connection) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("private.sqlite3");
    let db = schema::open_initialized(&path).unwrap();
    (directory, path, db)
}

fn seed(db: &Connection, decision: &str) {
    // Structural fixture only, not an authenticated user decision or real audit.
    db.execute(
        "INSERT INTO cdr_recovery_abandonment_proposals VALUES(?,1,1,'job','thread',30,20,40,'{}',?)",
        params![ID, "1".repeat(64)],
    ).unwrap();
    db.execute(
        "INSERT INTO cdr_recovery_abandonment_deliveries VALUES(?,1,50,?)",
        params![ID, "2".repeat(64)],
    )
    .unwrap();
    db.execute(
        "INSERT INTO cdr_recovery_abandonment_decisions VALUES(?,1,'interaction:60',60,?,'0')",
        params![ID, decision],
    )
    .unwrap();
}

fn assert_literal_drift_is_held(decision: &str) {
    let (_directory, path, db) = fixture();
    seed(&db, "abandon_only");
    db.execute(
        "INSERT INTO codex_request_cancellations VALUES('job','thread',20,30,70,3)",
        [],
    )
    .unwrap();
    let original: String = db
        .query_row(
            "SELECT sql FROM sqlite_schema
         WHERE name='cdr_recovery_abandonment_cancellation_no_delete'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(original.matches("d.decision='abandon_only'").count(), 1);
    let altered = original.replacen(
        "d.decision='abandon_only'",
        &format!("d.decision='{decision}'"),
        1,
    );
    db.execute_batch(&format!(
        "DROP TRIGGER cdr_recovery_abandonment_cancellation_no_delete; {altered}"
    ))
    .unwrap();
    // Show the actual semantic loss, then restore the structural fixture.
    db.execute_batch("SAVEPOINT probe_changed_trigger").unwrap();
    assert_eq!(
        db.execute(
            "DELETE FROM codex_request_cancellations WHERE job_id='job'",
            [],
        )
        .unwrap(),
        1
    );
    db.execute_batch("ROLLBACK TO probe_changed_trigger; RELEASE probe_changed_trigger")
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    let result = storage::check_compatibility_in(&db, 1);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let retained: String = db
        .query_row(
            "SELECT sql FROM sqlite_schema
         WHERE name='cdr_recovery_abandonment_cancellation_no_delete'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retained, altered);
    let cancellations: i64 = db
        .query_row(
            "SELECT count(*) FROM codex_request_cancellations WHERE job_id='job'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(cancellations, 1);
    assert!(
        result.is_err(),
        "changed decision literal was accepted: {decision}"
    );
}

#[test]
fn literal_internal_space_drift_is_rejected() {
    assert_literal_drift_is_held("abandon_ only");
}

#[test]
fn literal_case_drift_is_rejected() {
    assert_literal_drift_is_held("ABANDON_ONLY");
}

#[test]
fn literal_ifnotexists_text_is_not_removed() {
    assert_literal_drift_is_held("abandon_ifnotexistsonly");
}

#[test]
fn literal_sql_prefix_text_is_not_removed() {
    assert_literal_drift_is_held("abandon_IF NOT EXISTSonly");
}

#[test]
fn pristine_legacy_probe_does_not_initialize_anything() {
    let db = Connection::open_in_memory().unwrap();
    storage::check_compatibility_in(&db, 0).unwrap();
    let objects: i64 = db
        .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))
        .unwrap();
    assert_eq!(objects, 0);
}

#[test]
fn current_schema_is_not_permission_to_apply_or_release() {
    let (_directory, path, db) = fixture();
    storage::check_compatibility_in(&db, 1).unwrap();
    assert!(storage::check_compatibility_in(&db, 0).is_err());
    assert!(storage::check_compatibility_in(&db, 2).is_err());
    let cancellations: i64 = db
        .query_row(
            "SELECT count(*) FROM codex_request_cancellations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cancellations, 0);
    assert!(
        cdr_store::async_resolution::admission_held(
            &path,
            cdr_store::async_resolution::REVIEWED_INCIDENT_THREAD,
        )
        .unwrap()
    );
}

#[test]
fn append_only_evidence_rejects_update_delete_and_replace() {
    let (_directory, _path, db) = fixture();
    db.pragma_update(None, "recursive_triggers", false).unwrap();
    seed(&db, "keep_held");
    for (table, column) in [
        ("cdr_recovery_abandonment_proposals", "id"),
        ("cdr_recovery_abandonment_deliveries", "proposal_id"),
        ("cdr_recovery_abandonment_decisions", "proposal_id"),
    ] {
        for sql in [
            format!("UPDATE {table} SET {column}={column}"),
            format!("DELETE FROM {table}"),
            format!("INSERT OR REPLACE INTO {table} SELECT * FROM {table}"),
        ] {
            assert!(db.execute(&sql, []).is_err(), "{sql}");
        }
        let count: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}

#[test]
fn alternate_unique_key_cannot_replace_original_evidence() {
    let (_directory, _path, db) = fixture();
    db.pragma_update(None, "recursive_triggers", false).unwrap();
    seed(&db, "keep_held");
    assert!(
        db.execute(
            "INSERT OR REPLACE INTO cdr_recovery_abandonment_proposals
         SELECT 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',format_version,revision,job_id,
         target_thread_id,owner_user_id,channel_id,application_id,seal_json,seal_sha256
         FROM cdr_recovery_abandonment_proposals",
            [],
        )
        .is_err()
    );
    assert!(
        db.execute(
            "INSERT OR REPLACE INTO cdr_recovery_abandonment_decisions
         SELECT 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',revision,ingress_id,interaction_id,
         decision,recorded_at_bits FROM cdr_recovery_abandonment_decisions",
            [],
        )
        .is_err()
    );
    let retained: String = db
        .query_row(
            "SELECT id FROM cdr_recovery_abandonment_proposals",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(retained, ID);
}

#[test]
fn abandoned_cancellation_cannot_be_removed_or_replaced() {
    let (_directory, _path, db) = fixture();
    seed(&db, "abandon_only");
    db.execute(
        "INSERT INTO codex_turn_queue(job_id,target_thread_id,channel_id,owner_user_id,
         discord_message_id,prompt,queued,ack_sent,state,attempt_count,baseline_turn_ids,
         created_at,updated_at,app_server_generation)
         VALUES('job','thread',20,30,70,'private fixture input',1,1,'pending',356,'[]',1,2,1)",
        [],
    )
    .unwrap();
    db.execute_batch("CREATE TEMP TABLE saved_queue AS SELECT * FROM codex_turn_queue")
        .unwrap();
    db.execute(
        "INSERT INTO codex_request_cancellations VALUES('job','thread',20,30,70,3)",
        [],
    )
    .unwrap();
    db.execute("DELETE FROM codex_turn_queue WHERE job_id='job'", [])
        .unwrap();
    for sql in [
        "UPDATE codex_request_cancellations SET channel_id=99 WHERE job_id='job'",
        "DELETE FROM codex_request_cancellations WHERE job_id='job'",
        "INSERT OR REPLACE INTO codex_request_cancellations SELECT * FROM codex_request_cancellations",
        "INSERT OR REPLACE INTO codex_request_cancellations SELECT 'other',target_thread_id,
         channel_id,owner_user_id,discord_message_id,cancelled_at FROM codex_request_cancellations",
        "INSERT INTO codex_turn_queue SELECT * FROM saved_queue",
    ] {
        assert!(db.execute(sql, []).is_err(), "{sql}");
    }
    let retained: String = db
        .query_row("SELECT job_id FROM codex_request_cancellations", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(retained, "job");
}

#[test]
fn keep_held_does_not_redefine_existing_cancellation_behavior() {
    let (_directory, _path, db) = fixture();
    seed(&db, "keep_held");
    db.execute(
        "INSERT INTO codex_request_cancellations VALUES('job','thread',20,30,70,3)",
        [],
    )
    .unwrap();
    assert_eq!(
        db.execute(
            "DELETE FROM codex_request_cancellations WHERE job_id='job'",
            []
        )
        .unwrap(),
        1
    );
}

#[test]
fn changed_or_partial_catalog_is_rejected_without_repair() {
    for fault in [
        "DROP TRIGGER cdr_recovery_abandonment_proposal_no_delete",
        "DROP TRIGGER cdr_recovery_abandonment_proposal_no_delete;
         CREATE TRIGGER cdr_recovery_abandonment_proposal_no_delete BEFORE DELETE
         ON cdr_recovery_abandonment_proposals BEGIN SELECT 1; END",
        "ALTER TABLE cdr_recovery_abandonment_proposals ADD COLUMN unsupported TEXT",
        "DROP TABLE cdr_recovery_abandonment_deliveries",
        "DROP TABLE cdr_runtime_capability_requirements",
    ] {
        let (_directory, path, db) = fixture();
        db.execute_batch(fault).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(storage::check_compatibility_in(&db, 1).is_err(), "{fault}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}

#[test]
fn orphaned_delivery_revision_is_not_compatible_evidence() {
    let (_directory, _path, db) = fixture();
    db.execute(
        "INSERT INTO cdr_recovery_abandonment_deliveries VALUES(?,2,50,?)",
        params![ID, "1".repeat(64)],
    )
    .unwrap();
    assert!(storage::check_compatibility_in(&db, 1).is_err());
}

#[test]
fn unsupported_requirement_is_preserved_without_downgrade() {
    let (_directory, path, db) = fixture();
    db.execute(
        "UPDATE cdr_runtime_capability_requirements SET format_version=2 WHERE component=?",
        [storage::COMPONENT],
    )
    .unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(storage::check_compatibility_in(&db, 1).is_err());
    assert!(storage::check_compatibility_in(&db, 2).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn ignored_capability_insert_rolls_back_the_new_schema() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("partial.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TABLE cdr_runtime_capability_requirements(component TEXT PRIMARY KEY,
         format_version INTEGER NOT NULL);
         CREATE TRIGGER reject_abandonment_requirement BEFORE INSERT
         ON cdr_runtime_capability_requirements WHEN NEW.component='recovery_abandonment'
         BEGIN SELECT RAISE(IGNORE); END;",
    )
    .unwrap();
    assert!(schema::initialize(&mut db, &path).is_err());
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name GLOB 'cdr_recovery_abandonment_*'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}
