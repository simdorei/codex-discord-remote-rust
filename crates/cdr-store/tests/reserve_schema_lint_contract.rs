//! Strict-lint cleanup must not weaken the parent-owned Reserve schema contract.
use cdr_store::reserve_policy;
use rusqlite::Connection;

fn current() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    reserve_policy::migrate_schema(&connection).unwrap();
    assert!(reserve_policy::schema_current(&connection).unwrap());
    connection
}

#[test]
fn every_usage_fence_column_is_still_required_and_repaired() {
    for column in [
        "usage_failure_state",
        "usage_failure_revision",
        "usage_failure_id",
        "usage_failure_reason",
        "usage_failure_resolution_reason",
        "usage_failure_updated_at",
    ] {
        let connection = current();
        connection
            .execute_batch(&format!(
                "ALTER TABLE codex_reserve_policy DROP COLUMN {column}"
            ))
            .unwrap();
        assert!(
            !reserve_policy::schema_current(&connection).unwrap(),
            "{column}"
        );
        reserve_policy::migrate_schema(&connection).unwrap();
        assert!(
            reserve_policy::schema_current(&connection).unwrap(),
            "{column}"
        );
    }
}

#[test]
fn repeated_migration_preserves_pending_fence_and_schema_version() {
    let connection = current();
    connection
        .execute_batch(
            "INSERT INTO codex_reserve_policy
         (thread_id,mode,state,revision,usage_failure_state,usage_failure_id,
          usage_failure_revision,usage_failure_reason)
         VALUES('thread','on','ordinary',17,'pending',9,13,'exact failure');",
        )
        .unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "schema_version", |row| row.get(0))
        .unwrap();
    for _ in 0..2 {
        reserve_policy::migrate_schema(&connection).unwrap();
        assert!(reserve_policy::schema_current(&connection).unwrap());
        let preserved: bool = connection
            .query_row(
                "SELECT COUNT(*)=1 FROM codex_reserve_policy WHERE thread_id='thread'
             AND mode='on' AND state='ordinary' AND revision=17
             AND usage_failure_state='pending' AND usage_failure_id=9
             AND usage_failure_revision=13 AND usage_failure_reason='exact failure'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(preserved);
        assert_eq!(
            connection
                .pragma_query_value(None, "schema_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            version
        );
    }
}

#[test]
fn notice_tables_and_indexes_remain_required() {
    for sql in [
        "DROP TABLE codex_reserve_start_notices",
        "DROP TABLE codex_reserve_transition_notices",
        "DROP INDEX codex_reserve_policy_recovery",
        "DROP INDEX codex_reserve_transition_notices_pending",
    ] {
        let connection = current();
        connection.execute_batch(sql).unwrap();
        assert!(
            !reserve_policy::schema_current(&connection).unwrap(),
            "{sql}"
        );
        reserve_policy::migrate_schema(&connection).unwrap();
        assert!(
            reserve_policy::schema_current(&connection).unwrap(),
            "{sql}"
        );
    }
}

#[test]
fn merged_integer_column_definition_preserves_defaults_and_affinity() {
    let connection = current();
    connection
        .execute_batch(
            "ALTER TABLE codex_reserve_policy DROP COLUMN previous_effort_present;
         ALTER TABLE codex_reserve_policy DROP COLUMN usage_failure_id;",
        )
        .unwrap();
    reserve_policy::migrate_schema(&connection).unwrap();
    connection.execute_batch(
        "INSERT INTO codex_reserve_policy(thread_id,mode,state) VALUES('legacy','auto','ordinary');"
    ).unwrap();
    let valid: bool = connection
        .query_row(
            "SELECT previous_effort_present=0 AND usage_failure_id=0
         AND typeof(previous_effort_present)='integer' AND typeof(usage_failure_id)='integer'
         FROM codex_reserve_policy WHERE thread_id='legacy'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(valid);
    assert!(reserve_policy::schema_current(&connection).unwrap());
}

#[test]
fn migration_still_propagates_database_write_errors() {
    let connection = Connection::open_in_memory().unwrap();
    connection.pragma_update(None, "query_only", true).unwrap();
    assert!(reserve_policy::migrate_schema(&connection).is_err());
    assert!(!reserve_policy::schema_current(&connection).unwrap());
    connection.pragma_update(None, "query_only", false).unwrap();
    reserve_policy::migrate_schema(&connection).unwrap();
    assert!(reserve_policy::schema_current(&connection).unwrap());
}
