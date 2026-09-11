use std::path::Path;

use cdr_pro::conversation::execute;
use serde_json::{Value, json};

const SCOPE: &str = "codex-pro-0123456789abcdef01234567";
const URL: &str = "https://chatgpt.com/c/original";
const NEXT: &str = "https://chatgpt.com/c/replacement";

fn run(path: &Path, action: &str, url: Option<&str>, lease: Option<&str>, now: i64) -> Value {
    execute(path, action, SCOPE, url, lease, now).unwrap()
}
fn owner(value: &Value) -> &str {
    value["lease_token"].as_str().unwrap()
}
fn initial(path: &Path) {
    let lease = run(path, "acquire", None, None, 100);
    run(path, "set", Some(URL), Some(owner(&lease)), 100);
}

#[test]
fn expiry_alone_does_not_revoke_owner_but_reacquisition_does() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db");
    let first = run(&path, "acquire", None, None, 100);
    assert_eq!(
        run(&path, "status", None, None, 220),
        json!({"status":"busy"})
    );
    run(&path, "set", Some(URL), Some(owner(&first)), 221);
    run(&path, "delete", None, None, 222);
    let stale = run(&path, "acquire", None, None, 300);
    let current = run(&path, "acquire", None, None, 421);
    assert!(
        execute(&path, "set", SCOPE, Some(URL), Some(owner(&stale)), 500)
            .unwrap_err()
            .contains("missing or was replaced")
    );
    assert_eq!(
        run(&path, "set", Some(URL), Some(owner(&current)), 700),
        json!({"status":"saved"})
    );
}

#[test]
fn stalled_restart_retains_owner_and_cannot_be_reacquired_by_a_contender() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db");
    initial(&path);
    let lease = run(&path, "restart", Some(URL), None, 100);
    assert_eq!(
        run(&path, "status", None, None, 221),
        json!({"status":"stalled"})
    );
    assert_eq!(
        run(&path, "acquire", None, None, 221),
        json!({"status":"stalled"})
    );
    assert_eq!(
        run(&path, "restart", Some(URL), None, 221),
        json!({"status":"stalled"})
    );
    assert_eq!(
        run(&path, "set", Some(NEXT), Some(owner(&lease)), 221),
        json!({"status":"saved"})
    );
}

#[test]
fn guarded_restore_and_release_restore_original_without_admitting_stale_writers() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db");
    initial(&path);
    let stale = run(&path, "restart", Some(URL), None, 100);
    assert_eq!(
        run(&path, "restore-stalled", Some(URL), None, 200),
        json!({"status":"busy"})
    );
    assert_eq!(
        run(&path, "restore-stalled", Some(NEXT), None, 221),
        json!({"status":"superseded","url":URL})
    );
    assert_eq!(
        run(&path, "restore-stalled", Some(URL), None, 221),
        json!({"status":"restored","url":URL})
    );
    assert!(execute(&path, "set", SCOPE, Some(NEXT), Some(owner(&stale)), 222).is_err());
    let release = run(&path, "restart", Some(URL), None, 300);
    assert_eq!(
        run(&path, "release", None, Some(owner(&release)), 500),
        json!({"status":"released"})
    );
    assert_eq!(
        run(&path, "status", None, None, 500),
        json!({"status":"found","url":URL})
    );
}

#[test]
fn equivalent_failed_urls_cannot_be_saved_as_the_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db");
    initial(&path);
    let equivalent = "https://www.chatgpt.com/c/original/?model=pro#x";
    let lease = run(&path, "restart", Some(equivalent), None, 100);
    assert!(
        execute(
            &path,
            "set",
            SCOPE,
            Some(equivalent),
            Some(owner(&lease)),
            101
        )
        .unwrap_err()
        .contains("must differ")
    );
    run(&path, "set", Some(NEXT), Some(owner(&lease)), 101);
    assert_eq!(
        run(&path, "complete-restart", Some(URL), None, 102),
        json!({"status":"superseded","url":NEXT})
    );
    assert_eq!(
        run(&path, "delete", None, None, 102),
        json!({"status":"protected","url":NEXT})
    );
    assert_eq!(
        run(&path, "restart", Some(NEXT), None, 102),
        json!({"status":"exhausted","url":NEXT})
    );
    run(&path, "complete-restart", Some(NEXT), None, 103);
    assert_eq!(
        run(&path, "restart", Some(NEXT), None, 104)["status"],
        "acquired"
    );
}

#[test]
fn legacy_schema_migration_preserves_url_and_rejects_corrupt_types() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TABLE conversations(scope TEXT PRIMARY KEY, conversation_url TEXT, lease_hash TEXT, lease_expires_at INTEGER, updated_at INTEGER NOT NULL)").unwrap();
    connection
        .execute(
            "INSERT INTO conversations VALUES(?,?,NULL,NULL,0)",
            [SCOPE, URL],
        )
        .unwrap();
    assert_eq!(
        run(&path, "acquire", None, None, 100),
        json!({"status":"found","url":URL})
    );
    for mutation in [
        "UPDATE conversations SET lease_expires_at='not-an-integer'",
        "UPDATE conversations SET lease_expires_at=NULL,conversation_url=x'1234'",
        "UPDATE conversations SET conversation_url=NULL,restart_pending=2",
    ] {
        connection.execute_batch(mutation).unwrap();
        assert!(
            execute(&path, "acquire", SCOPE, None, None, 100)
                .unwrap_err()
                .contains("store contains invalid data")
        );
    }
}

#[test]
fn wrong_scope_and_invalid_url_are_rejected_before_database_creation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db");
    assert!(execute(&path, "acquire", "invalid", None, None, 100).is_err());
    assert!(
        execute(
            &path,
            "restart",
            SCOPE,
            Some("https://example.com/c/x"),
            None,
            100
        )
        .is_err()
    );
    assert!(!path.exists());
}
