//! Actual finish wiring at the durable handoff and receipt cleanup boundaries.
use super::goal_handoff_tests::{make_worker, setup_running};
use super::*;
use cdr_store::{delivery_receipt, goal_progress, observed_completion, queue};
use sha2::{Digest, Sha256};

fn completion() -> TurnCompletion {
    TurnCompletion {
        thread_id: "thread".into(),
        turn_id: "T1".into(),
        status: TurnStatus::Completed,
        error_message: String::new(),
        interrupt_origin: None,
            duration_ms: None,
            usage_limit: false,
    }
}

#[tokio::test]
async fn failed_handoff_commit_does_not_enter_sender_and_retains_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let worker = make_worker(&temp).await;
    setup_running(&worker);
    let db = worker.queue.db_path();
    let c = rusqlite::Connection::open(db).unwrap();
    c.execute_batch(
        "CREATE TRIGGER fail_handoff BEFORE DELETE ON codex_observed_completions
        BEGIN SELECT RAISE(ABORT,'injected handoff failure'); END;",
    )
    .unwrap();
    let error = worker
        .finish(
            worker.server.generation(),
            i64::try_from(worker.server.generation()).unwrap(),
            &completion(),
        )
        .await
        .unwrap_err();
    worker.server.close().await.unwrap();
    assert!(error.to_string().contains("injected handoff failure"));
    assert!(goal_progress::pending(db).unwrap().is_empty());
    assert!(observed_completion::contains(db, "thread", "T1").unwrap());
    assert!(!queue::list(db).unwrap()[0].goal_waiting);
    assert_eq!(delivery_receipt::unknown_count(db).unwrap(), 0);
}

#[tokio::test]
async fn committed_receipt_cleans_progress_after_restart_without_new_attempt() {
    let temp = tempfile::tempdir().unwrap();
    let worker = make_worker(&temp).await;
    setup_running(&worker);
    let db = worker.queue.db_path();
    let key = serde_json::to_string(&(42_u64, "completion/goal-progress/v1", "6:thread;2:T1;", 0))
        .unwrap();
    let hash = hex::encode(Sha256::digest(b"[Goal progress]\nprogress"));
    delivery_receipt::begin(db, &key, &hash).unwrap();
    assert!(delivery_receipt::confirm(db, &key, "123").unwrap());
    // A crash after receipt commit but before pending row cleanup.
    worker
        .queue
        .stage_goal_progress("thread", "T1", "[Goal progress]\nprogress")
        .await
        .unwrap();
    worker.server.close().await.unwrap();
    let restarted = make_worker(&temp).await;
    restarted.recover_goal_progress().await.unwrap();
    assert!(goal_progress::pending(db).unwrap().is_empty());
    restarted
        .finish(
            restarted.server.generation(),
            i64::try_from(restarted.server.generation()).unwrap(),
            &completion(),
        )
        .await
        .unwrap();
    assert!(goal_progress::pending(db).unwrap().is_empty());
    assert_eq!(
        delivery_receipt::begin(db, &key, &hash).unwrap(),
        delivery_receipt::ReceiptState::Delivered("123".into())
    );
    restarted.server.close().await.unwrap();
}
