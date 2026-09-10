//! DG3: loss/failure of the first observation cannot replay the original request.
use super::goal_handoff_tests::{make_worker, setup_running};
use super::*;
use cdr_store::{delivery_receipt, observed_completion, queue};
use serde_json::json;
use sha2::{Digest, Sha256};

async fn check_initial_boundary(boundary: u8) {
    let temp = tempfile::tempdir().unwrap();
    let worker = make_worker(&temp).await;
    setup_running(&worker);
    let db = worker.queue.db_path();
    observed_completion::finish(db, "thread", "T1").unwrap();
    let event = ResidentNotificationEvent::Notification {
        generation: worker.server.generation(),
        notification: cdr_app_server::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread","turn":{"id":"T1","status":"completed"}}),
        },
    };
    match boundary {
        0 => {
            let conn = rusqlite::Connection::open(db).unwrap();
            conn.execute_batch("CREATE TRIGGER fail_initial BEFORE INSERT ON codex_observed_completions BEGIN SELECT RAISE(ABORT, 'DG3 initial journal write failed'); END;").unwrap();
            let error = worker.observe_terminal(&event).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("DG3 initial journal write failed")
            );
            assert!(observed_completion::pending(db).unwrap().is_empty());
            conn.execute_batch("DROP TRIGGER fail_initial").unwrap();
        }
        1 => {
            // Durable state at a crash before the first observation commit.
            assert!(observed_completion::pending(db).unwrap().is_empty());
        }
        2 => {
            worker.observe_terminal(&event).unwrap();
            assert!(observed_completion::contains(db, "thread", "T1").unwrap());
        }
        _ => unreachable!(),
    }
    let before = queue::list(db).unwrap().remove(0);
    assert_eq!(before.state, QueueJobState::Running);
    // Keep delivery out of scope: an existing ambiguous receipt must not be sent.
    let key = serde_json::to_string(&(42_u64, "completion/goal-progress/v1", "6:thread;2:T1;", 0))
        .unwrap();
    delivery_receipt::begin(
        db,
        &key,
        &hex::encode(Sha256::digest(b"[Goal progress]\nprogress")),
    )
    .unwrap();
    worker.server.close().await.unwrap();
    let restarted = make_worker(&temp).await;
    let result = if boundary == 2 {
        restarted.recover_observed().await
    } else {
        restarted.recover().await
    };
    assert!(result.is_err(), "held delivery must remain visible");
    let after = queue::list(db).unwrap().remove(0);
    assert_eq!(after.job_id, before.job_id);
    assert_eq!(after.turn_id.as_deref(), Some("T1"));
    assert_eq!(
        after.attempt_count, before.attempt_count,
        "DG3: no original job replay"
    );
    assert!(
        after.goal_waiting,
        "DG3: readonly terminal recovery must reach durable handoff"
    );
    assert!(observed_completion::pending(db).unwrap().is_empty());
    let pending = cdr_store::goal_progress::pending(db).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].content, "[Goal progress]\nprogress");
    assert!(!pending[0].last_error.is_empty());
    assert_eq!(delivery_receipt::unknown_count(db).unwrap(), 1);
    let rpc = std::fs::read_to_string(temp.path().join("goal-rpc.log")).unwrap();
    assert!(
        !rpc.lines()
            .any(|method| matches!(method, "turn/start" | "thread/fork" | "turn/steer")),
        "DG3: no mutating request replay on wire"
    );
    restarted.server.close().await.unwrap();
}

#[tokio::test]
async fn first_journal_write_failure_recovers_without_job_replay() {
    check_initial_boundary(0).await;
}

#[tokio::test]
async fn crash_state_before_first_commit_recovers_without_job_replay() {
    check_initial_boundary(1).await;
}

#[tokio::test]
async fn crash_state_after_first_commit_recovers_without_job_replay() {
    check_initial_boundary(2).await;
}
