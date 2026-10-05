//! Real completion worker with four original target locks held, not mock state slots.
use crate::{
    app_backend::AppServerTurnBackend, queue_runner::QueueCoordinator, soak::native_fixture,
};
use cdr_app_server::{Notification, ResidentAppServer, ResidentNotificationEvent};
use cdr_store::{delivery, mirror, queue};
use serde_json::json;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::{broadcast, watch};

use crate::test_support::completion_lane_http as http_fixture;

fn seed(db: &Path, target: &str, generation: i64) {
    queue::enqueue(
        db,
        queue::NewQueueJob {
            job_id: target,
            target_thread_id: target,
            channel_id: 42,
            owner_user_id: Some(1),
            discord_message_id: None,
            app_server_generation: generation,
            prompt: "original",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    queue::begin_attempt(db, target, &[], generation).unwrap();
    queue::mark_running(db, target, "T1", generation).unwrap();
}

fn event(generation: u64, target: &str, method: &str, status: &str) -> ResidentNotificationEvent {
    ResidentNotificationEvent::Notification {
        generation,
        notification: Notification {
            method: method.into(),
            params: json!({"threadId":target,"turn":{"id":"T1","status":status,"error":{"message":"fixture"}}}),
        },
    }
}

#[tokio::test]
async fn patch05_review_p2_four_locked_starts_do_not_block_b_local_completion() {
    let temp = tempfile::tempdir().unwrap();
    let remote = http_fixture::start([42], []).await;
    let mut config = native_fixture::config("action");
    config.environment.insert(
        "CDR_ACTION_RPC_LOG".into(),
        temp.path().join("rpc.jsonl").to_string_lossy().into_owned(),
    );
    let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
    let db = temp.path().join("mirror.sqlite");
    let queue = Arc::new(QueueCoordinator::new(
        db.clone(),
        Arc::new(AppServerTurnBackend::new(Arc::clone(&server))),
    ));
    let generation = server.generation();
    for target in ["a1", "a2", "a3", "a4", "b"] {
        seed(&db, target, i64::try_from(generation).unwrap());
    }
    let locks = ["a1", "a2", "a3", "a4"].map(|t| queue.target_lock(t).unwrap());
    let mut held = Vec::new();
    for lock in &locks {
        held.push(lock.lock().await);
    }
    let client = Arc::new(
        twilight_http::Client::builder()
            .token("fixture".into())
            .proxy(remote.address.clone(), true)
            .ratelimiter(None)
            .build(),
    );
    let (sender, receiver) = broadcast::channel(128);
    let (stop, shutdown) = watch::channel(false);
    let task = tokio::spawn(super::super::run_completion_worker(
        receiver,
        Arc::clone(&server),
        Arc::clone(&queue),
        client,
        false,
        Duration::from_secs(2),
        shutdown,
    ));
    for target in ["a1", "a2", "a3", "a4"] {
        sender
            .send(event(generation, target, "turn/started", "inProgress"))
            .unwrap();
    }
    sender
        .send(event(generation, "b", "turn/completed", "failed"))
        .unwrap();
    let stored = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if mirror::has_event(&db, &mirror::turn_origin_marker("b", "T1"), "b").unwrap()
                && delivery::list_pending(&db)
                    .unwrap()
                    .iter()
                    .any(|d| d.job_id == "b")
                && !queue::list(&db).unwrap().iter().any(|j| j.job_id == "b")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    let original = queue::list(&db).unwrap();
    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    drop(held);
    server.close().await.unwrap();
    remote.release();
    let posts = remote.posts.lock().unwrap().clone();
    assert!(
        posts
            .iter()
            .all(|p| p.channel == 42 && p.content.starts_with("Failed"))
    );
    remote.close().await;
    assert!(
        original
            .iter()
            .filter(|j| j.job_id != "b")
            .all(|j| j.turn_id.as_deref() == Some("T1") && j.attempt_count == 1)
    );
    assert!(
        stored.is_ok(),
        "B marker/outbox/queue commit must finish within5s while four original locks remain held"
    );
}

#[tokio::test]
async fn patch05_review_p2_failed_goal_waiting_is_native_with_exact_lease_proof() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = native_fixture::config("action");
    config.environment.insert(
        "CDR_ACTION_RPC_LOG".into(),
        temp.path().join("rpc.jsonl").to_string_lossy().into_owned(),
    );
    let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
    let db = temp.path().join("mirror.sqlite");
    let queue = Arc::new(QueueCoordinator::new(
        db.clone(),
        Arc::new(AppServerTurnBackend::new(Arc::clone(&server))),
    ));
    let generation = i64::try_from(server.generation()).unwrap();
    seed(&db, "goal", generation);
    assert!(queue::mark_goal_waiting(&db, "goal", "T1", generation).unwrap());
    let worker = super::super::CompletionWorker {
        server: Arc::clone(&server),
        queue: Arc::clone(&queue),
        http: Arc::new(twilight_http::Client::new("fixture".into())),
        commentary_enabled: false,
        history_read_timeout: Duration::from_secs(2),
        commentary: tokio::sync::Mutex::new(crate::commentary_stream::CommentaryBuffer::default()),
        terminal_fence: super::super::terminal_fence::TerminalFence::default(),
    };
    let budget = Arc::new(tokio::sync::Semaphore::new(super::lanes::EVENT_BYTES));
    let item = super::lanes::StateWork::Live(
        super::lanes::Envelope::charge(
            event(server.generation(), "goal", "turn/completed", "failed"),
            &budget,
        )
        .unwrap(),
    );
    assert!(
        !item.needs_native(),
        "raw failed status alone does not expose the history wait"
    );
    let (admission, native) = super::work::prepare(&worker, &item).unwrap().unwrap();
    assert!(native, "captured goal_waiting owner requires a native slot");
    assert!(queue.target_lock("goal").unwrap().try_lock().is_err());
    let super::super::AdmissionOwner::Exact(expected) = &admission.owner else {
        panic!("exact captured owner");
    };
    let mut changed = expected.as_ref().clone();
    changed.goal_waiting = false;
    assert!(
        super::super::Processing::Staged(&admission)
            .validate_owner(&changed)
            .is_err()
    );
    drop(admission);
    assert!(queue.target_lock("goal").unwrap().try_lock().is_ok());
    server.close().await.unwrap();
}
