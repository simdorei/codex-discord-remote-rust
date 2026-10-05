use super::*;
use tempfile::TempDir;

fn fixture() -> (TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    activate(&path, "runtime-a").unwrap();
    (temp, path)
}

fn attempt<'a>(id: &'a str, target: &'a str) -> NewAttempt<'a> {
    NewAttempt {
        runtime_id: "runtime-a",
        owner_id: "instance-a",
        generation: 1,
        attempt_id: id,
        wire_id: id,
        method: "thread/settings/update",
        target_thread_id: Some(target),
        scoped: true,
        payload: &Value::Null,
    }
}

fn completion<'a>(id: &'a str, outcome: &'a str) -> Completion<'a> {
    Completion {
        runtime_id: "runtime-a",
        owner_id: "instance-a",
        generation: 1,
        attempt_id: id,
        wire_id: id,
        outcome,
    }
}

#[test]
fn mutation_attempt_scope_exact_completion_and_duplicate_cas() {
    let (_temp, path) = fixture();
    begin(&path, &attempt("a", "A")).unwrap();
    assert!(check(&path, "runtime-a", Some("A")).is_err());
    check(&path, "runtime-a", Some("B")).unwrap();
    begin(&path, &attempt("b", "B")).unwrap();
    for bad in [
        Completion {
            owner_id: "other",
            ..completion("a", "reply_ok")
        },
        Completion {
            generation: 2,
            ..completion("a", "reply_ok")
        },
        Completion {
            wire_id: "new-occurrence",
            ..completion("a", "reply_ok")
        },
    ] {
        assert!(finish(&path, &bad).is_err());
    }
    finish(&path, &completion("b", "reply_ok")).unwrap();
    assert!(check(&path, "runtime-a", Some("A")).is_err());
    check(&path, "runtime-a", Some("B")).unwrap();
    finish(&path, &completion("a", "reply_error")).unwrap();
    assert!(finish(&path, &completion("a", "reply_ok")).is_err());
    check(&path, "runtime-a", Some("A")).unwrap();
    assert!(begin(&path, &attempt("a", "new-target")).is_err());
}

#[test]
fn mutation_attempt_cold_owner_never_clears_or_finishes_old_attempt() {
    let (_temp, path) = fixture();
    begin(&path, &attempt("a", "A")).unwrap();
    activate(&path, "runtime-new").unwrap();
    assert!(finish(&path, &completion("a", "reply_ok")).is_err());
    assert!(check(&path, "runtime-new", Some("A")).is_err());
    check(&path, "runtime-new", Some("B")).unwrap();
    assert!(check(&path, "runtime-a", Some("B")).is_err());
    let db = existing(&path).unwrap();
    let state: String = db
        .query_row(
            "SELECT state FROM codex_mutation_attempts WHERE attempt_id='a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "prepared");
}

#[test]
fn mutation_attempt_unknown_scope_is_global_and_never_borrows_a_target() {
    let (_temp, path) = fixture();
    begin(&path, &attempt("a", "A")).unwrap();
    assert!(
        begin(
            &path,
            &NewAttempt {
                scoped: false,
                method: "unregistered/mutate",
                ..attempt("global", "B")
            }
        )
        .is_err()
    );
    finish(&path, &completion("a", "not_sent")).unwrap();
    begin(
        &path,
        &NewAttempt {
            scoped: false,
            method: "unregistered/mutate",
            ..attempt("global", "B")
        },
    )
    .unwrap();
    for target in [Some("A"), Some("B"), Some("C"), None] {
        assert!(check(&path, "runtime-a", target).is_err());
    }
    assert!(finish(&path, &completion("global", "timeout")).is_err());
}

#[test]
fn mutation_attempt_no_eviction_at_1024_and_confirmed_history_bound_256() {
    let (_temp, path) = fixture();
    begin(&path, &attempt("unknown", "A")).unwrap();
    let db = existing(&path).unwrap();
    db.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1023)
        INSERT INTO codex_mutation_attempts(attempt_id,runtime_id,owner_id,generation,wire_id,method,
        target_thread_id,scoped,request_sha256,state,created_at,updated_at)
        SELECT 'held-'||x,'runtime-a','instance-a',1,'wire-'||x,'thread/settings/update',
        'target-'||x,1,'fixture','prepared',0,0 FROM n;").unwrap();
    assert!(begin(&path, &attempt("overflow", "B")).is_err());
    let pending: i64 = db
        .query_row(
            "SELECT count(*) FROM codex_mutation_attempts WHERE state='prepared'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1024);
    db.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<300)
        INSERT INTO codex_mutation_attempts(attempt_id,runtime_id,owner_id,generation,wire_id,method,
        target_thread_id,scoped,request_sha256,state,created_at,updated_at)
        SELECT 'done-'||x,'runtime-a','instance-a',1,'done-wire-'||x,'thread/settings/update',
        'done-target-'||x,1,'fixture','reply_ok',0,0 FROM n;").unwrap();
    finish(&path, &completion("held-1", "not_sent")).unwrap_err();
    finish(
        &path,
        &Completion {
            wire_id: "wire-1",
            ..completion("held-1", "not_sent")
        },
    )
    .unwrap();
    let confirmed: i64 = db
        .query_row(
            "SELECT count(*) FROM codex_mutation_attempts WHERE state!='prepared'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(confirmed, 256);
    assert!(check(&path, "runtime-a", Some("A")).is_err());
    let pending: i64 = db
        .query_row(
            "SELECT count(*) FROM codex_mutation_attempts WHERE state='prepared'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1023);
}

#[test]
fn mutation_attempt_insert_ignore_or_removed_claim_never_grants_dispatch() {
    for trigger in [
        "CREATE TRIGGER fail BEFORE INSERT ON codex_mutation_attempts BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER fail AFTER INSERT ON codex_mutation_attempts BEGIN DELETE FROM codex_mutation_attempts WHERE attempt_id=NEW.attempt_id; END;",
        "CREATE TRIGGER fail AFTER INSERT ON codex_mutation_attempts BEGIN UPDATE codex_mutation_runtime SET runtime_id='other'; END;",
    ] {
        let (_temp, path) = fixture();
        let db = existing(&path).unwrap();
        db.execute_batch(trigger).unwrap();
        assert!(begin(&path, &attempt("a", "A")).is_err());
        owner_is_current(&db, "runtime-a").unwrap();
        let count: i64 = db
            .query_row("SELECT count(*) FROM codex_mutation_attempts", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[test]
fn mutation_attempt_failed_completion_rolls_back_and_absent_store_fails_closed() {
    let (temp, path) = fixture();
    begin(&path, &attempt("a", "A")).unwrap();
    let db = existing(&path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER fail AFTER UPDATE ON codex_mutation_attempts
        BEGIN DELETE FROM codex_mutation_attempts WHERE attempt_id=NEW.attempt_id; END;",
    )
    .unwrap();
    assert!(finish(&path, &completion("a", "reply_ok")).is_err());
    assert!(check(&path, "runtime-a", Some("A")).is_err());
    let missing = temp.path().join("missing.sqlite");
    assert!(check(&missing, "runtime-a", Some("A")).is_err());
    assert!(!missing.exists());
    db.execute("DELETE FROM codex_mutation_runtime", [])
        .unwrap();
    assert!(check(&path, "runtime-a", Some("B")).is_err());
}

#[test]
fn mutation_attempt_concurrent_same_target_has_one_owner() {
    let (_temp, path) = fixture();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = ["one", "two"].map(|id| {
        let path = path.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            begin(&path, &attempt(id, "A")).is_ok()
        })
    });
    let successes = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .filter(|ok| *ok)
        .count();
    assert_eq!(successes, 1);
}

#[test]
fn mutation_attempt_extension_migration_preserves_prior_queue_and_payload_privacy() {
    let (_temp, path) = fixture();
    let db = existing(&path).unwrap();
    db.execute_batch(
        "CREATE TABLE fixture_marker(value TEXT);
        INSERT INTO fixture_marker VALUES('unchanged');
        DROP TABLE codex_mutation_attempts; DROP TABLE codex_mutation_runtime;",
    )
    .unwrap();
    drop(db);
    activate(&path, "runtime-a").unwrap();
    let payload = serde_json::json!({"input":"private fixture prompt"});
    begin(
        &path,
        &NewAttempt {
            payload: &payload,
            ..attempt("a", "A")
        },
    )
    .unwrap();
    let db = existing(&path).unwrap();
    assert!(schema_current(&db).unwrap());
    let value: String = db
        .query_row("SELECT value FROM fixture_marker", [], |r| r.get(0))
        .unwrap();
    assert_eq!(value, "unchanged");
    let digest: String = db
        .query_row(
            "SELECT request_sha256 FROM codex_mutation_attempts",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(digest.len(), 64);
    assert!(!digest.contains("private fixture"));
}
