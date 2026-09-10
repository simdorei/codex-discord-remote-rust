#[path = "support/dead_generation.rs"]
mod support;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use cdr_app_server::{DeadGenerationFence, ResidentAppServer};
use cdr_runtime::dead_generation_recovery::RuntimeDeadGenerationFence;
use cdr_runtime::queue_runner::QueueCoordinator;
use cdr_store::dead_generation::{generation_is_sealed, target_is_held};
use cdr_store::queue::{NewQueueJob, enqueue, list, try_begin_attempt};
use rusqlite::Connection;
use serde_json::json;

fn fence(db: &Path) -> Arc<RuntimeDeadGenerationFence> {
    Arc::new(RuntimeDeadGenerationFence::new(db.into(), "runtime-a".into(), Some(88)).unwrap())
}

fn starting_job(db: &Path) {
    enqueue(
        db,
        NewQueueJob {
            job_id: "uncertain-job",
            target_thread_id: "held-thread",
            channel_id: 10,
            owner_user_id: Some(20),
            discord_message_id: None,
            app_server_generation: 1,
            prompt: "old prompt must never replay",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    try_begin_attempt(db, "uncertain-job", &[], 1)
        .unwrap()
        .unwrap();
}

// DG6: full protocol work is durable before the replacement can become gen2.
#[tokio::test]
async fn dead_turn_and_request_are_fenced_before_replacement_and_never_replayed() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let log = temp.path().join("methods");
    let durable = fence(&db);
    let server = Arc::new(
        ResidentAppServer::start_with_dead_generation_fence(support::config(&log), durable.clone())
            .await
            .unwrap(),
    );
    starting_job(&db);
    support::kill_by_protocol(&server, "test/die-active").await;
    let expected = server.dead_generation_work(1).await.unwrap().unwrap();
    let before = list(&db).unwrap();
    assert!(support::restart_dead(&server).await.unwrap());
    assert_eq!(server.generation(), 2);
    assert_eq!(list(&db).unwrap(), before);
    assert!(target_is_held(&db, "held-thread").unwrap());
    let saved: String = Connection::open(&db)
        .unwrap()
        .query_row(
            "SELECT snapshot_json FROM codex_dead_generation_incidents",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<cdr_app_server::DeadGenerationWork>(&saved).unwrap(),
        expected
    );
    for method in ["thread/resume", "thread/fork", "turn/start"] {
        assert!(
            server
                .request(
                    method,
                    json!({"threadId":"held-thread"}),
                    Duration::from_secs(1),
                    Some(2)
                )
                .await
                .is_err()
        );
    }
    let queue = QueueCoordinator::new(
        db.clone(),
        Arc::new(cdr_runtime::app_backend::AppServerTurnBackend::new(
            server.clone(),
        )),
    );
    assert!(
        queue
            .ensure_app_server_only_target("held-thread")
            .await
            .is_err()
    );
    queue.recover_target("held-thread").await.unwrap();
    let fresh = queue
        .submit("independent", 10, 20, None, "new request")
        .await
        .unwrap();
    assert_eq!(fresh.turn_id.as_deref(), Some("new-turn"));
    let calls = support::methods(&log);
    assert!(!calls.iter().any(|call| call.ends_with(" held-thread")));
    assert!(calls.iter().any(|call| call == "turn/start independent"));
    for notice in cdr_store::delivery::list_pending(&db).unwrap() {
        assert!(!notice.content.contains("private"));
        cdr_store::delivery::complete(&db, &notice.delivery_id).unwrap();
    }
    durable.persist(&expected).unwrap();
    assert!(cdr_store::delivery::list_pending(&db).unwrap().is_empty());
    server.close().await.unwrap();
}

// DG7: the central hook is required even when public dead_generation_work is None.
#[tokio::test]
async fn empty_dead_snapshot_captures_starting_only_job() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let server = ResidentAppServer::start_with_dead_generation_fence(
        support::config(&temp.path().join("methods")),
        fence(&db),
    )
    .await
    .unwrap();
    starting_job(&db);
    support::kill_by_protocol(&server, "test/die-empty").await;
    assert!(server.dead_generation_work(1).await.unwrap().is_none());
    assert!(support::restart_dead(&server).await.unwrap());
    assert_eq!(server.generation(), 2);
    assert!(target_is_held(&db, "held-thread").unwrap());
    assert!(generation_is_sealed(&db, 1).unwrap());
    assert_eq!(
        list(&db).unwrap()[0].state,
        cdr_store::queue::QueueJobState::Starting
    );
    // A retained idempotency snapshot from gen1 must not be mistaken for gen2.
    support::kill_by_protocol(&server, "test/die-empty").await;
    assert!(support::restart_dead(&server).await.unwrap());
    assert_eq!(server.generation(), 3);
    assert!(generation_is_sealed(&db, 2).unwrap());
    server.close().await.unwrap();
}

#[tokio::test]
async fn automatic_supervisor_cannot_race_past_empty_snapshot_starting_job() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let server = Arc::new(
        ResidentAppServer::start_with_dead_generation_fence(
            support::config(&temp.path().join("methods")),
            fence(&db),
        )
        .await
        .unwrap(),
    );
    starting_job(&db);
    let (shutdown, shutdown_rx) = tokio::sync::watch::channel(false);
    let supervisor = tokio::spawn(server.clone().run_restart_supervisor(shutdown_rx));
    let _ = server
        .request("test/die-empty", json!({}), Duration::from_secs(1), Some(1))
        .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while server.generation() != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("supervisor replaced fenced dead generation");
    shutdown.send(true).unwrap();
    supervisor.await.unwrap();
    assert!(target_is_held(&db, "held-thread").unwrap());
    assert!(generation_is_sealed(&db, 1).unwrap());
    assert_eq!(
        list(&db).unwrap()[0].state,
        cdr_store::queue::QueueJobState::Starting
    );
    server.close().await.unwrap();
}

// DG8: failed durable persistence never clears the old snapshot or advances gen.
#[tokio::test]
async fn store_failure_leaves_generation_one_unsettled_then_retry_can_replace() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let server = ResidentAppServer::start_with_dead_generation_fence(
        support::config(&temp.path().join("methods")),
        fence(&db),
    )
    .await
    .unwrap();
    starting_job(&db);
    support::kill_by_protocol(&server, "test/die-active").await;
    let expected = server.dead_generation_work(1).await.unwrap().unwrap();
    let connection = Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_fence BEFORE INSERT ON codex_delivery_outbox
        BEGIN SELECT RAISE(ABORT, 'fixture notice failure'); END;",
        )
        .unwrap();
    assert!(support::restart_dead(&server).await.is_err());
    assert_eq!(server.generation(), 1);
    assert_eq!(
        server.dead_generation_work(1).await.unwrap().unwrap(),
        expected
    );
    assert!(!target_is_held(&db, "held-thread").unwrap());
    assert!(!generation_is_sealed(&db, 1).unwrap());
    connection
        .execute_batch("DROP TRIGGER reject_fence;")
        .unwrap();
    assert!(support::restart_dead(&server).await.unwrap());
    assert_eq!(server.generation(), 2);
    server.close().await.unwrap();
}

// DG9: a still-alive quarantined app-server keeps the existing quiescent rule.
#[tokio::test]
async fn alive_quarantined_work_is_not_fenced_or_killed() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let server = ResidentAppServer::start_with_dead_generation_fence(
        support::config(&temp.path().join("methods")),
        fence(&db),
    )
    .await
    .unwrap();
    server
        .request("test/active", json!({}), Duration::from_secs(1), Some(1))
        .await
        .unwrap();
    assert!(
        server
            .request("test/hang", json!({}), Duration::from_millis(50), Some(1))
            .await
            .is_err()
    );
    assert!(server.lifecycle_snapshot().await.quarantined);
    assert!(!server.force_restart_if_quiescent().await.unwrap());
    assert_eq!(server.generation(), 1);
    assert!(!generation_is_sealed(&db, 1).unwrap());
    assert!(!target_is_held(&db, "held-thread").unwrap());
    server.close().await.unwrap();
}
