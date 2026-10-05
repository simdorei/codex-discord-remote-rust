use cdr_store::schema::open_initialized;

#[test]
fn exact_disposition_ledger_is_installed_before_any_apply() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("private.sqlite3");
    let db = open_initialized(&path).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name IN (
         'cdr_recovery_abandonment_proposals','cdr_recovery_abandonment_deliveries',
         'cdr_recovery_abandonment_decisions')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 3,
        "exact no-replay disposition schema is not installed"
    );
    let version: i64 = db
        .query_row(
            "SELECT format_version FROM cdr_runtime_capability_requirements
         WHERE component='recovery_abandonment'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, 1);
}
