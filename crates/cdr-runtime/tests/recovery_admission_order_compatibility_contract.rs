use serde_json::Value;
use std::{ffi::OsString, path::Path};

async fn check(path: &Path) -> Result<Value, String> {
    let output = cdr_runtime::admin::run([
        OsString::from("check-recovery-compatibility"),
        OsString::from("--database"),
        path.as_os_str().to_owned(),
    ])
    .await?;
    serde_json::from_str(&output).map_err(|error| error.to_string())
}

#[tokio::test]
async fn admission_order_support_is_not_new_request_release_authority() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite");
    drop(cdr_store::schema::open_initialized(&path).unwrap());
    let before = std::fs::read(&path).unwrap();
    let result = Box::pin(check(&path)).await.unwrap();
    assert_eq!(result["required_recovery_admission_order_format"], 1);
    assert_eq!(result["supported_recovery_admission_order_format"], 1);
    assert_eq!(result["new_requests_release_supported"], false);
    assert_eq!(result["admission_authorized"], false);
    assert_eq!(result["recovery_authorized"], false);
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[tokio::test]
async fn changed_order_schema_and_capability_are_refused_without_writes() {
    for fault in [
        "DROP TRIGGER cdr_recovery_ingress_order_no_replace",
        "DROP TABLE cdr_recovery_ingress_order",
        "UPDATE cdr_runtime_capability_requirements SET format_version=2
         WHERE component='recovery_admission_order'",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private.sqlite");
        let db = cdr_store::schema::open_initialized(&path).unwrap();
        let present: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='cdr_recovery_ingress_order')",
            [], |row| row.get(0),
        ).unwrap();
        assert!(present, "R4-D3A: missing durable admission-order feature");
        db.execute_batch(fault).unwrap();
        drop(db);
        let before = std::fs::read(&path).unwrap();
        assert!(Box::pin(check(&path)).await.is_err(), "{fault}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}
