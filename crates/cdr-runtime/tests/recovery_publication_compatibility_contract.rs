use std::{
    path::Path,
    process::{Command, Output},
};

use cdr_store::schema;
use rusqlite::Connection;
use serde_json::Value;

fn probe(path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cdr-runtime"))
        .args(["--admin", "check-recovery-compatibility", "--database"])
        .arg(path)
        .current_dir(path.parent().unwrap())
        .output()
        .unwrap()
}

fn check_read_only(path: &Path, compatible: bool) {
    let before = std::fs::read(path).unwrap();
    let output = probe(path);
    assert_eq!(
        std::fs::read(path).unwrap(),
        before,
        "probe mutated its database"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    if compatible {
        assert!(output.status.success(), "{stdout}\n{stderr}");
        let value: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(value["compatible"], true);
        assert_eq!(value["read_only"], true);
        assert_eq!(value["admission_authorized"], false);
        assert_eq!(value["recovery_authorized"], false);
        assert_eq!(value["special_dispatch_supported"], false);
    } else {
        assert!(
            !output.status.success(),
            "incomplete or unsupported ledger was compatible: {stdout}"
        );
        assert!(stderr.contains("RECOVERY_COMPATIBILITY_HOLD"), "{stderr}");
        assert!(!stdout.contains("\"compatible\":true"), "{stdout}");
    }
}

#[test]
fn missing_proposal_consumed_columns_are_held_without_schema_repair() {
    for column in [
        "seal_json",
        "seal_sha256",
        "id",
        "job_id",
        "target_thread_id",
        "owner_user_id",
        "channel_id",
        "application_id",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.sqlite");
        let db = schema::open_initialized(&path).unwrap();
        db.execute_batch(&format!(
            "ALTER TABLE cdr_recovery_publication_proposals
             RENAME COLUMN {column} TO legacy_{column};"
        ))
        .unwrap();
        let guards: i64 = db.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name IN (
             'cdr_recovery_publication_proposal_immutable','cdr_recovery_publication_proposal_no_delete',
             'cdr_recovery_publication_delivery_immutable','cdr_recovery_publication_delivery_no_delete',
             'cdr_recovery_publication_decision_immutable','cdr_recovery_publication_decision_no_delete')",
            [], |r| r.get(0)).unwrap();
        assert_eq!(guards, 6);
        drop(db);
        check_read_only(&path, false);
    }
}

#[test]
fn supported_current_and_absent_legacy_ledgers_are_readonly_compatible() {
    for legacy in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.sqlite");
        if legacy {
            Connection::open(&path)
                .unwrap()
                .pragma_update(None, "user_version", schema::LATEST_STORE_SCHEMA_VERSION)
                .unwrap();
        } else {
            drop(schema::open_initialized(&path).unwrap());
        }
        check_read_only(&path, true);
    }
}

#[test]
fn future_consent_capability_is_held_without_downgrade() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.sqlite");
    let db = schema::open_initialized(&path).unwrap();
    db.execute(
        "UPDATE cdr_runtime_capability_requirements SET format_version=2
         WHERE component='recovery_publication_consent'",
        [],
    )
    .unwrap();
    drop(db);
    check_read_only(&path, false);
}

#[test]
fn existing_delivery_and_decision_column_checks_remain_readonly() {
    for (table, column) in [
        ("cdr_recovery_publication_deliveries", "body_sha256"),
        ("cdr_recovery_publication_deliveries", "message_id"),
        ("cdr_recovery_publication_decisions", "recorded_at_bits"),
        ("cdr_recovery_publication_decisions", "ingress_id"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.sqlite");
        let db = schema::open_initialized(&path).unwrap();
        db.execute_batch(&format!(
            "ALTER TABLE {table} RENAME COLUMN {column} TO legacy_{column};"
        ))
        .unwrap();
        drop(db);
        check_read_only(&path, false);
    }
}

#[test]
fn required_but_missing_consent_table_is_not_recreated() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.sqlite");
    let db = schema::open_initialized(&path).unwrap();
    db.execute_batch("DROP TABLE cdr_recovery_publication_deliveries;")
        .unwrap();
    drop(db);
    check_read_only(&path, false);
}
