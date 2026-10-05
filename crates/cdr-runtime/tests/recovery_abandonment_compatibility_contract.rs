use serde_json::Value;
use std::{ffi::OsString, path::Path};

async fn check(path: &Path) -> Result<Value, String> {
    let output = Box::pin(cdr_runtime::admin::run([
        OsString::from("check-recovery-compatibility"),
        OsString::from("--database"),
        path.as_os_str().to_owned(),
    ]))
    .await?;
    serde_json::from_str(&output).map_err(|error| error.to_string())
}

#[tokio::test]
async fn in_process_admin_rejects_literal_drift_without_writes() {
    for decision in [
        "abandon_ only",
        "ABANDON_ONLY",
        "abandon_ifnotexistsonly",
        "abandon_IF NOT EXISTSonly",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("private.sqlite3");
        let db = cdr_store::schema::open_initialized(&path).unwrap();
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
        drop(db);
        let before = std::fs::read(&path).unwrap();
        let result = Box::pin(check(&path)).await;
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(
            result.is_err(),
            "changed literal was compatible: {decision}; {result:?}"
        );
    }
}

#[tokio::test]
async fn native_admin_reports_storage_support_but_no_disposal_authority() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("private.sqlite3");
    drop(cdr_store::schema::open_initialized(&path).unwrap());
    let before = std::fs::read(&path).unwrap();
    let result = Box::pin(check(&path)).await.unwrap();
    assert_eq!(result["required_recovery_abandonment_format"], 1);
    assert_eq!(result["supported_recovery_abandonment_format"], 1);
    assert_eq!(result["abandonment_apply_supported"], false);
    assert_eq!(result["admission_authorized"], false);
    assert_eq!(result["recovery_authorized"], false);
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[tokio::test]
async fn native_admin_rejects_newer_or_malformed_storage_without_writes() {
    for fault in [
        "UPDATE cdr_runtime_capability_requirements SET format_version=2
         WHERE component='recovery_abandonment'",
        "DROP TRIGGER cdr_recovery_abandonment_decision_no_replace",
        "DROP TABLE cdr_recovery_abandonment_decisions",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("private.sqlite3");
        let db = cdr_store::schema::open_initialized(&path).unwrap();
        db.execute_batch(fault).unwrap();
        drop(db);
        let before = std::fs::read(&path).unwrap();
        assert!(Box::pin(check(&path)).await.is_err(), "{fault}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}
