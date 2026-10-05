use cdr_store::schema::open_initialized;

#[test]
fn consent_ledger_is_installed_with_a_distinct_required_capability() {
    let directory = tempfile::tempdir().unwrap();
    let db = open_initialized(&directory.path().join("store.sqlite")).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name IN (
         'cdr_recovery_publication_proposals','cdr_recovery_publication_deliveries',
         'cdr_recovery_publication_decisions')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 3,
        "publication intent requires its own durable ledger"
    );
    let version: i64 = db
        .query_row(
            "SELECT format_version FROM cdr_runtime_capability_requirements
         WHERE component='recovery_publication_consent'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, 1);
}
