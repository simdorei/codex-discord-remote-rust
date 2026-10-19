//! PATCH-04A: the storage seam, not a claim that runtime stop is wired yet.
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use cdr_store::{Result, StoreError, mutation_attempt};
use rusqlite::{Connection, TransactionBehavior, params};
use serde_json::json;

struct Fixture {
    _temp: tempfile::TempDir,
    path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("store.sqlite");
        mutation_attempt::activate(&path, "runtime-a").unwrap();
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE test_control_authority (
                target TEXT PRIMARY KEY, revision INTEGER NOT NULL,
                stopped INTEGER NOT NULL, origin TEXT NOT NULL);
             INSERT INTO test_control_authority VALUES('thread-a',7,0,'job-a');
             INSERT INTO test_control_authority VALUES('thread-b',3,0,'job-b');",
            )
            .unwrap();
        Self { _temp: temp, path }
    }

    fn db(&self) -> Connection {
        Connection::open(&self.path).unwrap()
    }

    fn count(&self) -> i64 {
        self.db()
            .query_row("SELECT count(*) FROM codex_mutation_attempts", [], |r| {
                r.get(0)
            })
            .unwrap()
    }
}

fn check_original(db: &Connection, target: &str, revision: i64, origin: &str) -> Result<()> {
    let allowed: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM test_control_authority
         WHERE target=? AND revision=? AND stopped=0 AND origin=?)",
        params![target, revision, origin],
        |r| r.get(0),
    )?;
    if allowed {
        Ok(())
    } else {
        Err(StoreError::Integrity("original admission revoked".into()))
    }
}

fn begin(
    path: &Path,
    target: &str,
    attempt: &str,
    check: impl Fn(&Connection) -> Result<()>,
) -> Result<()> {
    mutation_attempt::begin_checked(
        path,
        &mutation_attempt::NewAttempt {
            runtime_id: "runtime-a",
            owner_id: "resident-a",
            generation: 9,
            attempt_id: attempt,
            wire_id: "wire-original",
            method: "turn/start",
            target_thread_id: Some(target),
            scoped: true,
            payload: &json!({"threadId":target,"input":[{"text":"original input"}]}),
        },
        check,
    )
}

#[test]
fn stop_commit_before_writer_claim_rejects_original_and_allows_other_target() {
    let f = Fixture::new();
    f.db()
        .execute(
            "UPDATE test_control_authority SET revision=8,stopped=1 WHERE target='thread-a'",
            [],
        )
        .unwrap();
    let result = begin(&f.path, "thread-a", "attempt-a", |db| {
        check_original(db, "thread-a", 7, "job-a")
    });
    assert!(
        matches!(result, Err(StoreError::Integrity(ref message)) if message == "original admission revoked")
    );
    assert_eq!(f.count(), 0);
    begin(&f.path, "thread-b", "attempt-b", |db| {
        check_original(db, "thread-b", 3, "job-b")
    })
    .unwrap();
    assert_eq!(f.count(), 1);
}

#[test]
fn writer_claim_serializes_stop_and_retains_exact_uncertain_attempt() {
    let f = Fixture::new();
    let stop_path = f.path.clone();
    let (inside_tx, wait_inside_tx) = mpsc::channel();
    let (stop_tried, wait_stop_tried) = mpsc::channel();
    let (writer_done, wait_writer_done) = mpsc::channel();
    let stop = thread::spawn(move || {
        wait_inside_tx.recv_timeout(Duration::from_secs(3)).unwrap();
        let mut db = Connection::open(&stop_path).unwrap();
        db.busy_timeout(Duration::from_millis(100)).unwrap();
        let error = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .err()
            .unwrap();
        assert!(
            matches!(error, rusqlite::Error::SqliteFailure(ref code, _) if code.code == rusqlite::ErrorCode::DatabaseBusy)
        );
        stop_tried.send(()).unwrap();
        wait_writer_done
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        tx.execute(
            "UPDATE test_control_authority SET revision=8,stopped=1 WHERE target='thread-a'",
            [],
        )
        .unwrap();
        let identity: (String, String, i64, String, String, String) = tx.query_row(
            "SELECT runtime_id,owner_id,generation,wire_id,method,state FROM codex_mutation_attempts WHERE attempt_id='attempt-a'",
            [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
        ).unwrap();
        tx.commit().unwrap();
        identity
    });
    let first = std::cell::Cell::new(true);
    let result = begin(&f.path, "thread-a", "attempt-a", |db| {
        assert!(
            !db.is_autocommit(),
            "authority must be checked inside the writer transaction"
        );
        if first.replace(false) {
            inside_tx.send(()).unwrap();
            wait_stop_tried
                .recv_timeout(Duration::from_secs(3))
                .unwrap();
        }
        check_original(db, "thread-a", 7, "job-a")
    });
    writer_done.send(()).unwrap();
    let identity = stop.join().unwrap();
    result.unwrap();
    assert_eq!(
        identity,
        (
            "runtime-a".into(),
            "resident-a".into(),
            9,
            "wire-original".into(),
            "turn/start".into(),
            "prepared".into()
        )
    );
    mutation_attempt::activate(&f.path, "runtime-b").unwrap();
    assert!(mutation_attempt::check(&f.path, "runtime-b", Some("thread-a")).is_err());
    assert!(mutation_attempt::check(&f.path, "runtime-b", Some("thread-b")).is_ok());
    assert_eq!(
        f.count(),
        1,
        "cold activation must not evict or replay the claim"
    );
}

#[test]
fn authority_changed_by_insert_trigger_rolls_back_claim_and_revision() {
    let f = Fixture::new();
    f.db().execute_batch(
        "CREATE TRIGGER revoke_during_claim AFTER INSERT ON codex_mutation_attempts
         BEGIN UPDATE test_control_authority SET revision=8,stopped=1 WHERE target='thread-a'; END;",
    ).unwrap();
    let result = begin(&f.path, "thread-a", "attempt-a", |db| {
        check_original(db, "thread-a", 7, "job-a")
    });
    assert!(
        matches!(result, Err(StoreError::Integrity(ref message)) if message == "original admission revoked")
    );
    assert_eq!(f.count(), 0);
    let revision: i64 = f
        .db()
        .query_row(
            "SELECT revision FROM test_control_authority WHERE target='thread-a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        revision, 7,
        "failed claim must roll back every statement in its transaction"
    );
}

#[test]
fn missing_authority_is_not_reconstructed_from_current_target() {
    let f = Fixture::new();
    f.db()
        .execute(
            "DELETE FROM test_control_authority WHERE target='thread-a'",
            [],
        )
        .unwrap();
    assert!(
        begin(&f.path, "thread-a", "attempt-a", |db| check_original(
            db, "thread-a", 7, "job-a"
        ))
        .is_err()
    );
    assert_eq!(f.count(), 0);
}

#[test]
fn database_failure_in_authority_check_cannot_leave_a_writer_claim() {
    let f = Fixture::new();
    f.db()
        .execute("DROP TABLE test_control_authority", [])
        .unwrap();
    assert!(matches!(
        begin(&f.path, "thread-a", "attempt-a", |db| check_original(
            db, "thread-a", 7, "job-a"
        )),
        Err(StoreError::Database(_))
    ));
    assert_eq!(f.count(), 0);
}

#[test]
fn second_check_failure_preserves_original_starting_queue_evidence() {
    let f = Fixture::new();
    cdr_store::queue::enqueue(
        &f.path,
        cdr_store::queue::NewQueueJob {
            job_id: "job-a",
            target_thread_id: "thread-a",
            channel_id: 99,
            owner_user_id: Some(20),
            discord_message_id: Some(123),
            app_server_generation: 9,
            prompt: "original input",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    cdr_store::queue::begin_attempt(&f.path, "job-a", &["baseline-turn".into()], 9).unwrap();
    let calls = std::cell::Cell::new(0);
    let result = begin(&f.path, "thread-a", "attempt-a", |db| {
        calls.set(calls.get() + 1);
        check_original(db, "thread-a", 7, "job-a")?;
        if calls.get() == 2 {
            return Err(StoreError::Integrity(
                "injected late custody rejection".into(),
            ));
        }
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(calls.get(), 2);
    assert_eq!(f.count(), 0);
    let original: (String, String, i64, i64, String) = f.db().query_row(
        "SELECT state,prompt,attempt_count,execution_generation,baseline_turn_ids FROM codex_turn_queue WHERE job_id='job-a'",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).unwrap();
    assert_eq!(
        original,
        (
            "starting".into(),
            "original input".into(),
            1,
            9,
            "[\"baseline-turn\"]".into()
        )
    );
}
