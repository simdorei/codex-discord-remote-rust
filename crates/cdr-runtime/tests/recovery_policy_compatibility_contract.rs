use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{ffi::OsString, path::Path};

async fn compatibility(path: &Path) -> Result<Value, String> {
    Box::pin(cdr_runtime::admin::run([
        OsString::from("check-recovery-compatibility"),
        OsString::from("--database"),
        path.as_os_str().to_owned(),
    ]))
    .await
    .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
}

fn sha(path: &Path) -> Vec<u8> {
    Sha256::digest(std::fs::read(path).unwrap()).to_vec()
}

#[tokio::test]
async fn compatibility_is_read_only_and_never_arms_or_authorizes_policy() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    cdr_store::schema::open_initialized(&path).unwrap();
    let before = sha(&path);
    let unarmed = compatibility(&path).await.unwrap();
    assert_eq!(unarmed["required_async_recovery_policy_format"], 1);
    assert_eq!(unarmed["supported_async_recovery_policy_format"], 1);
    assert_eq!(unarmed["reviewed_policy_installed"], false);
    assert_eq!(sha(&path), before);
    cdr_store::async_resolution::install_reviewed_policy(&path).unwrap();
    let before = sha(&path);
    let armed = compatibility(&path).await.unwrap();
    assert_eq!(armed["reviewed_policy_installed"], true);
    for key in [
        "admission_authorized",
        "recovery_authorized",
        "special_dispatch_supported",
    ] {
        assert_eq!(armed[key], false, "{key}");
    }
    assert_eq!(armed["read_only"], true);
    assert_eq!(sha(&path), before);
}

#[tokio::test]
async fn unsupported_policy_capability_is_not_downgraded_or_silently_accepted() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    cdr_store::schema::open_initialized(&path)
        .unwrap()
        .execute(
            "UPDATE cdr_runtime_capability_requirements SET format_version=2
         WHERE component='async_recovery_policy'",
            [],
        )
        .unwrap();
    let before = sha(&path);
    let error = compatibility(&path).await.unwrap_err();
    assert!(
        error.contains("async_recovery_policy") && error.contains("version 2"),
        "{error}"
    );
    assert_eq!(sha(&path), before);
}

#[tokio::test]
async fn missing_required_policy_schema_is_not_recreated_by_readonly_compatibility() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    cdr_store::schema::open_initialized(&path)
        .unwrap()
        .execute_batch("DROP TABLE cdr_async_recovery_policies;")
        .unwrap();
    let before = sha(&path);
    assert!(
        compatibility(&path)
            .await
            .unwrap_err()
            .contains("policy ledger is missing")
    );
    assert_eq!(sha(&path), before);
}
