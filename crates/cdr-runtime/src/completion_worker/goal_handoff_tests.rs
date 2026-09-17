//! GH1: a held progress receipt must not prevent durable goal ownership handoff.
use super::*;
use cdr_store::{delivery_receipt, observed_completion, queue};
use sha2::{Digest, Sha256};

pub(super) async fn make_worker(temp: &tempfile::TempDir) -> CompletionWorker {
    let mut config = crate::test_support::native_fixture::config("goal");
    config.environment.insert(
        "GOAL_TEST_LOG".into(),
        temp.path()
            .join("goal-rpc.log")
            .to_string_lossy()
            .into_owned(),
    );
    config.environment.insert(
        "GOAL_TEST_ROLLOUT".into(),
        temp.path()
            .join("goal-rollout.jsonl")
            .to_string_lossy()
            .into_owned(),
    );
    let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
    CompletionWorker {
        queue: Arc::new(QueueCoordinator::new(
            temp.path().join("mirror.sqlite"),
            Arc::new(AppServerTurnBackend::new(Arc::clone(&server))),
        )),
        server,
        http: Arc::new(
            Client::builder()
                .token("test-token".into())
                .proxy("127.0.0.1:1".into(), true)
                .ratelimiter(None)
                .build(),
        ),
        commentary_enabled: false,
        history_read_timeout: Duration::from_secs(2),
        commentary: Mutex::new(CommentaryBuffer::default()),
        terminal_fence: terminal_fence::TerminalFence::default(),
    }
}

pub(super) fn setup_running(worker: &CompletionWorker) {
    let db = worker.queue.db_path();
    let generation = i64::try_from(worker.server.generation()).unwrap();
    queue::enqueue(
        db,
        queue::NewQueueJob {
            job_id: "job",
            target_thread_id: "thread",
            channel_id: 42,
            owner_user_id: Some(1),
            discord_message_id: None,
            app_server_generation: generation,
            prompt: "input",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    queue::begin_attempt(db, "job", &[], generation).unwrap();
    queue::mark_running(db, "job", "T1", generation).unwrap();
    observed_completion::record(
        db,
        "thread",
        "T1",
        generation,
        r#"{"threadId":"thread","turn":{"id":"T1","status":"completed"}}"#,
    )
    .unwrap();
}

async fn held_progress_still_hands_off(blocked: bool) {
    let temp = tempfile::tempdir().unwrap();
    let worker = make_worker(&temp).await;
    setup_running(&worker);
    let db = worker.queue.db_path();
    // A prior attempt's durable outcome, at the same real sender identity.
    let key = serde_json::to_string(&(42_u64, "completion/goal-progress/v1", "6:thread;2:T1;", 0))
        .unwrap();
    let hash = hex::encode(Sha256::digest(b"[Goal progress]\nprogress"));
    delivery_receipt::begin(db, &key, &hash).unwrap();
    if blocked {
        delivery_receipt::block_rejected(db, &key, "403 Missing Permissions").unwrap();
    }
    let completion = TurnCompletion {
        thread_id: "thread".into(),
        turn_id: "T1".into(),
        status: TurnStatus::Completed,
        error_message: String::new(),
        interrupt_origin: None,
        duration_ms: None,
        usage_limit: false,
    };
    let result = worker
        .finish(
            worker.server.generation(),
            i64::try_from(worker.server.generation()).unwrap(),
            &completion,
        )
        .await;
    worker.server.close().await.unwrap();
    assert!(result.is_err(), "delivery failure must remain visible");
    let error = result.unwrap_err().to_string();
    assert!(
        error.contains(if blocked {
            "requires correction"
        } else {
            "outcome unknown"
        }),
        "{error}"
    );
    assert!(
        queue::list(db).unwrap()[0].goal_waiting,
        "GH1: progress failure must not prevent goal ownership handoff"
    );
    assert!(!observed_completion::contains(db, "thread", "T1").unwrap());
    assert_eq!(
        delivery_receipt::blocked_count(db).unwrap(),
        i64::from(blocked)
    );
    assert_eq!(
        delivery_receipt::unknown_count(db).unwrap(),
        i64::from(!blocked)
    );
    let pending = cdr_store::goal_progress::pending(db).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].content, "[Goal progress]\nprogress");
    assert!(!pending[0].last_error.is_empty());
    assert!(
        cdr_store::mirror::has_event(
            db,
            &cdr_store::mirror::turn_origin_marker("thread", "T1"),
            "thread"
        )
        .unwrap()
    );
    // Reconstruct both app-server and worker; no in-memory delivery state survives.
    let restarted = make_worker(&temp).await;
    assert!(
        restarted
            .finish(
                restarted.server.generation(),
                i64::try_from(restarted.server.generation()).unwrap(),
                &completion,
            )
            .await
            .is_err()
    );
    assert!(
        restarted
            .queue
            .goal_turn_started("thread", "T2")
            .await
            .unwrap()
    );
    for _ in 0..2 {
        assert!(restarted.recover_goal_progress().await.is_err());
    }
    assert_eq!(queue::list(db).unwrap()[0].turn_id.as_deref(), Some("T2"));
    assert!(!queue::list(db).unwrap()[0].goal_waiting);
    assert_eq!(cdr_store::goal_progress::pending(db).unwrap().len(), 1);
    assert_eq!(
        delivery_receipt::blocked_count(db).unwrap(),
        i64::from(blocked)
    );
    assert_eq!(
        delivery_receipt::unknown_count(db).unwrap(),
        i64::from(!blocked)
    );
    restarted.server.close().await.unwrap();
}

#[tokio::test]
async fn blocked_progress_does_not_block_goal_handoff() {
    held_progress_still_hands_off(true).await;
}

#[tokio::test]
async fn unknown_progress_does_not_block_goal_handoff() {
    held_progress_still_hands_off(false).await;
}
