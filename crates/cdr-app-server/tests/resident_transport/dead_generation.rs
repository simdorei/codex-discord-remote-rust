use std::sync::Arc;
use std::time::Duration;

use cdr_app_server::{
    AppServerError, DeadActiveTurn, DeadGenerationSettleResult, DeadGenerationWork,
    ResidentAppServer, ResidentNotificationEvent,
};
use serde_json::json;
use tokio::sync::watch;
use tokio::time::timeout;

use super::fake_config;

const WAIT: Duration = Duration::from_secs(3);

async fn create_active_turn_death(server: &ResidentAppServer) {
    let mut notifications = server.subscribe_notifications();
    server
        .request(
            "test/startThenExit",
            json!({}),
            Duration::from_secs(1),
            Some(1),
        )
        .await
        .expect("start and exit");
    let ResidentNotificationEvent::Notification { notification, .. } =
        timeout(WAIT, notifications.recv())
            .await
            .expect("turn notification timeout")
            .expect("turn notification")
    else {
        panic!("unexpected notification gap");
    };
    assert_eq!(notification.method, "turn/started");
    timeout(WAIT, async {
        loop {
            let lifecycle = server.lifecycle_snapshot().await;
            if lifecycle.process_id.is_none() && lifecycle.restart_pending {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dead generation propagation");
}

#[tokio::test]
async fn partial_stale_and_repeated_settlement_never_clear_new_generation_state() {
    let server = ResidentAppServer::start(fake_config())
        .await
        .expect("start");
    create_active_turn_death(&server).await;
    let work = server
        .dead_generation_work(1)
        .await
        .expect("generation one")
        .expect("dead work");

    let mut partial = work.clone();
    partial.active_turns.clear();
    assert_eq!(
        server
            .settle_dead_generation(1, &partial)
            .await
            .expect("partial settlement result"),
        DeadGenerationSettleResult::SnapshotChanged
    );
    assert_eq!(
        server
            .dead_generation_work(1)
            .await
            .expect("unchanged work"),
        Some(work.clone())
    );
    assert_eq!(
        server
            .settle_dead_generation(1, &work)
            .await
            .expect("exact settlement"),
        DeadGenerationSettleResult::Settled
    );
    assert_eq!(
        server
            .settle_dead_generation(1, &work)
            .await
            .expect("idempotent settlement"),
        DeadGenerationSettleResult::AlreadySettled
    );
    assert!(
        server
            .force_restart_if_quiescent()
            .await
            .expect("generation two restart")
    );

    server
        .request("test/notify", json!({}), Duration::from_secs(1), Some(2))
        .await
        .expect("generation two active turn");
    assert_eq!(
        server
            .settle_dead_generation(2, &work)
            .await
            .expect("cross-generation snapshot"),
        DeadGenerationSettleResult::SnapshotChanged
    );
    assert_eq!(
        server
            .active_turn_id("thread-a")
            .await
            .expect("active turn"),
        Some("turn-a".to_owned())
    );
    assert_eq!(
        server
            .settle_dead_generation(1, &work)
            .await
            .expect("late repeated settlement"),
        DeadGenerationSettleResult::AlreadySettled
    );
    assert_eq!(
        server
            .active_turn_id("thread-a")
            .await
            .expect("active turn"),
        Some("turn-a".to_owned())
    );

    let mut changed = work;
    changed.active_turns[0].turn_id = "other-turn".to_owned();
    assert!(matches!(
        server.settle_dead_generation(1, &changed).await,
        Err(AppServerError::GenerationMismatch {
            expected: 1,
            actual: 2
        })
    ));
    assert_eq!(
        server
            .active_turn_id("thread-a")
            .await
            .expect("active turn"),
        Some("turn-a".to_owned())
    );
    server.close().await.expect("close");
}

#[tokio::test]
async fn live_transport_rejects_dead_generation_settlement_without_mutation() {
    let server = ResidentAppServer::start(fake_config())
        .await
        .expect("start");
    server
        .request("test/notify", json!({}), Duration::from_secs(1), Some(1))
        .await
        .expect("live active turn");
    let claimed_dead_work = DeadGenerationWork {
        generation: 1,
        closed_reason: "not actually closed".to_owned(),
        active_turns: vec![DeadActiveTurn {
            thread_id: "thread-a".to_owned(),
            turn_id: "turn-a".to_owned(),
        }],
        server_requests: Vec::new(),
    };

    assert!(
        server
            .dead_generation_work(1)
            .await
            .expect("live query")
            .is_none()
    );
    assert_eq!(
        server
            .settle_dead_generation(1, &claimed_dead_work)
            .await
            .expect("live settlement"),
        DeadGenerationSettleResult::NotEligible
    );
    assert_eq!(
        server
            .active_turn_id("thread-a")
            .await
            .expect("active turn"),
        Some("turn-a".to_owned())
    );
    server.close().await.expect("close");
}

#[tokio::test]
async fn exact_settlement_allows_existing_supervisor_to_start_generation_two() {
    let server = Arc::new(
        ResidentAppServer::start(fake_config())
            .await
            .expect("start"),
    );
    let (shutdown, shutdown_rx) = watch::channel(false);
    let supervisor = tokio::spawn(Arc::clone(&server).run_restart_supervisor(shutdown_rx));
    create_active_turn_death(&server).await;
    let work = server
        .dead_generation_work(1)
        .await
        .expect("generation one")
        .expect("dead work");
    assert_eq!(
        server
            .settle_dead_generation(1, &work)
            .await
            .expect("settlement"),
        DeadGenerationSettleResult::Settled
    );

    timeout(WAIT, async {
        while server.generation() != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("supervisor generation two");
    assert_eq!(
        server
            .request(
                "test/echo",
                json!({"generation": 2}),
                Duration::from_secs(1),
                Some(2),
            )
            .await
            .expect("generation two echo"),
        json!({"generation": 2})
    );
    shutdown.send(true).expect("stop supervisor");
    timeout(WAIT, supervisor)
        .await
        .expect("supervisor stop timeout")
        .expect("supervisor task");
    server.close().await.expect("close");
}
