use cdr_store::{StoreError, mapping, room_cleanup, schema};
use rusqlite::Connection;

#[test]
fn archived_begin_checks_parent_before_archive_callback() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mirror.sqlite");
    mapping::upsert_thread(&path, "old", "C:/repo", "old", 21, 31, 1.0).unwrap();
    let mut called = false;
    let result = room_cleanup::archived_rejections::begin(&path, 31, "old", 99, 20.0, || {
        called = true;
        Ok("{}".into())
    });
    assert!(result.is_err());
    assert!(!called);
    assert_eq!(room_cleanup::phase(&path, 31).unwrap(), None);
    assert_eq!(
        mapping::thread_channels(&path, "old").unwrap(),
        Some((21, 31))
    );
}

#[test]
fn failed_archive_verification_rolls_back_the_close_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mirror.sqlite");
    mapping::upsert_thread(&path, "old", "C:/repo", "old", 21, 31, 1.0).unwrap();
    let result = room_cleanup::archived_rejections::begin(&path, 31, "old", 21, 20.0, || {
        Err(StoreError::Integrity("source restored".into()))
    });
    assert!(result.is_err());
    assert_eq!(room_cleanup::phase(&path, 31).unwrap(), None);
    assert_eq!(
        mapping::thread_channels(&path, "old").unwrap(),
        Some((21, 31))
    );
}

#[test]
fn reopening_restores_append_only_guards_without_changing_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mirror.sqlite");
    let connection = schema::open_initialized(&path).unwrap();
    connection.execute_batch(
        "INSERT INTO cdr_archived_cleanup_evidence VALUES ('token',31,'old','original','{}','{}','{}','{}',20);
         DROP TRIGGER cdr_archived_cleanup_evidence_no_update;
         DROP TRIGGER cdr_archived_cleanup_evidence_no_delete;
         DROP INDEX cdr_archived_cleanup_evidence_ingress;",
    ).unwrap();
    drop(connection);
    drop(schema::open_initialized(&path).unwrap());
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT ingress_id FROM cdr_archived_cleanup_evidence",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "original"
    );
    assert!(
        connection
            .execute("DELETE FROM cdr_archived_cleanup_evidence", [])
            .is_err()
    );
    assert!(
        connection
            .execute(
                "UPDATE cdr_archived_cleanup_evidence SET outcome_json='null'",
                []
            )
            .is_err()
    );
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE name='cdr_archived_cleanup_evidence_ingress'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
}
