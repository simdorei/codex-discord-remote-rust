use super::{MUTATE, support};
use cdr_runtime::{app_backend::AppServerTurnBackend, queue_runner::QueueCoordinator};
use cdr_store::queue::{QueueJobState, list};
use rusqlite::Connection;
use serde_json::json;
use std::sync::Arc;
use support::{EXPIRY, WAIT, count, start};

#[tokio::test]
async fn insert_failure_or_ignore_sends_zero_bytes_without_fencing_b() {
    for instruction in ["RAISE(ABORT, 'fixture insert failure')", "RAISE(IGNORE)"] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        let log = temp.path().join("rpc.jsonl");
        let server = start(&db, &log, "runtime-a", "normal").await;
        Connection::open(&db)
            .unwrap()
            .execute_batch(&format!(
                "CREATE TRIGGER fail BEFORE INSERT ON codex_mutation_attempts
            WHEN NEW.target_thread_id='thread-a' BEGIN SELECT {instruction}; END;"
            ))
            .unwrap();
        let a = server
            .request(MUTATE, json!({"threadId":"thread-a"}), WAIT, Some(1))
            .await;
        let b = server
            .request(MUTATE, json!({"threadId":"thread-b"}), WAIT, Some(1))
            .await;
        server.close().await.unwrap();
        assert!(a.is_err());
        assert!(b.is_ok());
        assert_eq!(count(&log, MUTATE, "thread-a"), 0);
        assert_eq!(count(&log, MUTATE, "thread-b"), 1);
    }
}

#[tokio::test]
async fn unavailable_journal_blocks_mutation_but_not_observation() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let server = start(&db, &log, "runtime-a", "normal").await;
    Connection::open(&db)
        .unwrap()
        .execute("DELETE FROM codex_mutation_runtime", [])
        .unwrap();
    let a = server
        .request(MUTATE, json!({"threadId":"thread-a"}), WAIT, Some(1))
        .await;
    let read = server
        .request("thread/read", json!({"threadId":"thread-a"}), WAIT, Some(1))
        .await;
    server.close().await.unwrap();
    assert!(a.is_err());
    assert!(read.is_ok());
    assert_eq!(count(&log, MUTATE, "thread-a"), 0);
    assert_eq!(count(&log, "thread/read", "thread-a"), 1);
}

#[tokio::test]
async fn completion_commit_failure_keeps_original_starting_and_never_replays() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let server = start(&db, &log, "runtime-a", "normal").await;
    let connection = Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER fail BEFORE UPDATE ON codex_mutation_attempts
        WHEN OLD.method='turn/start' AND OLD.target_thread_id='thread-a'
        BEGIN SELECT RAISE(ABORT, 'fixture completion write failure'); END;",
        )
        .unwrap();
    let queue = QueueCoordinator::new(
        db.clone(),
        Arc::new(AppServerTurnBackend::new(server.clone())),
    );
    let _ = queue
        .submit("thread-a", 10, 20, None, "original must not replay")
        .await;
    let b = queue.submit("thread-b", 10, 20, None, "independent").await;
    connection.execute("DROP TRIGGER fail", []).unwrap();
    for _ in 0..2 {
        let _ = queue.recover_target("thread-a").await;
        let _ = queue.kick_target("thread-a").await;
    }
    let jobs = list(&db).unwrap();
    server.close().await.unwrap();
    let a = jobs
        .iter()
        .find(|j| j.target_thread_id == "thread-a")
        .unwrap();
    assert_eq!(a.state, QueueJobState::Starting);
    assert_eq!(a.attempt_count, 1);
    assert_eq!(a.prompt, "original must not replay");
    assert!(b.is_ok(), "B queue progression was lost: {b:?}");
    assert_eq!(count(&log, "turn/start", "thread-a"), 1);
    assert_eq!(count(&log, "turn/start", "thread-b"), 1);
}

#[tokio::test]
async fn confirmed_error_or_success_finishes_only_its_own_request() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let server = start(&db, &log, "runtime-a", "normal").await;
    assert!(
        server
            .request(
                MUTATE,
                json!({"threadId":"thread-a","fixture":"hold"}),
                EXPIRY,
                Some(1)
            )
            .await
            .is_err()
    );
    assert!(
        server
            .request(
                MUTATE,
                json!({"threadId":"thread-b","fixture":"remote"}),
                WAIT,
                Some(1)
            )
            .await
            .is_err()
    );
    let b = server
        .request(MUTATE, json!({"threadId":"thread-b"}), WAIT, Some(1))
        .await;
    let a = server
        .request(MUTATE, json!({"threadId":"thread-a"}), WAIT, Some(1))
        .await;
    server.close().await.unwrap();
    assert!(b.is_ok());
    assert!(a.is_err());
    let db = Connection::open(&db).unwrap();
    let held: i64 = db.query_row("SELECT count(*) FROM codex_mutation_attempts WHERE state='prepared' AND target_thread_id='thread-a'", [], |r| r.get(0)).unwrap();
    let confirmed: i64 = db.query_row("SELECT count(*) FROM codex_mutation_attempts WHERE state IN ('reply_error','reply_ok') AND target_thread_id='thread-b'", [], |r| r.get(0)).unwrap();
    assert_eq!(held, 1);
    assert_eq!(confirmed, 2);
    assert_eq!(count(&log, MUTATE, "thread-a"), 1);
}

#[tokio::test]
async fn explicit_interrupt_does_not_borrow_or_clear_the_old_attempt() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let server = start(&db, &log, "runtime-a", "normal").await;
    let _ = server
        .request(
            MUTATE,
            json!({"threadId":"thread-a","fixture":"hold"}),
            EXPIRY,
            Some(1),
        )
        .await;
    let stop = server
        .request(
            "turn/interrupt",
            json!({"threadId":"thread-a","turnId":"known-turn"}),
            WAIT,
            Some(1),
        )
        .await;
    let a = server
        .request(MUTATE, json!({"threadId":"thread-a"}), WAIT, Some(1))
        .await;
    server.close().await.unwrap();
    assert!(
        stop.is_ok(),
        "existing explicit interrupt route must remain available"
    );
    assert!(
        a.is_err(),
        "interrupt ACK is not resolution of the original mutation"
    );
    assert_eq!(count(&log, "turn/interrupt", "thread-a"), 1);
    assert_eq!(count(&log, MUTATE, "thread-a"), 1);
}
