use std::{ffi::OsString, path::Path};
#[path = "async_recovery_readonly_contract/lifecycle.rs"]
mod lifecycle;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[path = "../../../tests/support/async_orphan_fixture.rs"]
mod fixture;

async fn inspect(db: &Path, extra: &[&str]) -> Result<Value, String> {
    let mut args = vec![
        OsString::from("inspect-async-recovery"),
        OsString::from("--database"),
        db.as_os_str().to_owned(),
        OsString::from("--thread-id"),
        OsString::from("thread-b"),
    ];
    args.extend(extra.iter().map(OsString::from));
    cdr_runtime::admin::run(args)
        .await
        .and_then(|text| serde_json::from_str(&text).map_err(|e| e.to_string()))
}

fn file_sha(path: &Path) -> Vec<u8> {
    Sha256::digest(std::fs::read(path).unwrap()).to_vec()
}

#[tokio::test]
async fn missing_or_unsupported_database_is_not_created_or_migrated() {
    let temp = tempfile::tempdir().unwrap();
    let absent = temp.path().join("absent.sqlite");
    assert!(inspect(&absent, &[]).await.is_err());
    assert!(!absent.exists());
    for version in [0_i64, 999] {
        let path = temp.path().join(format!("old-{version}.sqlite"));
        let db = rusqlite::Connection::open(&path).unwrap();
        db.pragma_update(None, "user_version", version).unwrap();
        drop(db);
        let before = file_sha(&path);
        assert!(
            inspect(&path, &[])
                .await
                .unwrap_err()
                .contains(&version.to_string())
        );
        assert_eq!(file_sha(&path), before);
    }
    assert!(!temp.path().join(".codex-discord-backups").exists());
}

#[tokio::test]
async fn exact_inventory_is_read_only_bounded_and_never_an_execution_authorization() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let id = fixture::dispatching(&path, "resident");
    cdr_store::queue::complete(&path, "origin").unwrap();
    fixture::pending(&path, "next", "thread-b", 2);
    let before = file_sha(&path);
    let value = inspect(&path, &["--dry-run"]).await.unwrap();
    assert_eq!(value["read_only"], true);
    assert_eq!(value["execution_authorized"], false);
    assert_eq!(value["receipt_apply_authorized"], false);
    assert_eq!(value["publication_authorized"], false);
    assert_eq!(value["requires_evidence_review"], true);
    assert_eq!(value["questions"]["rows"][0]["id"], id);
    assert_eq!(value["obligations"]["rows"][0]["origin_job_id"], "origin");
    assert_eq!(value["jobs"]["rows"][0]["job_id"], "next");
    assert_eq!(value["jobs"]["rows"][0]["attempt_count"], 0);
    let serialized = value.to_string();
    assert!(!serialized.contains("original input"));
    assert!(!serialized.contains("only this answer"));
    assert_eq!(file_sha(&path), before);
    assert!(inspect(&path, &["--apply"]).await.is_err());
    assert_eq!(file_sha(&path), before);
}

#[tokio::test]
async fn old_extension_absence_is_inventory_not_negative_execution_proof() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("legacy.sqlite");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.pragma_update(
        None,
        "user_version",
        cdr_store::schema::LATEST_STORE_SCHEMA_VERSION,
    )
    .unwrap();
    drop(db);
    let before = file_sha(&path);
    let value = inspect(&path, &[]).await.unwrap();
    assert_eq!(value["ledger_schema_present"], false);
    assert_eq!(value["execution_authorized"], false);
    assert_eq!(file_sha(&path), before);
    let db = rusqlite::Connection::open(&path).unwrap();
    let tables: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type='table'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
}
