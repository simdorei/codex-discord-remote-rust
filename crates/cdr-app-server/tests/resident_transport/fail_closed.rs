use std::time::Duration;

use cdr_app_server::{
    DeadActiveTurn, DeadGenerationSettleResult, DeadServerRequest, ResidentAppServer,
    ResidentNotificationEvent, ResidentServerRequestEvent,
};
use serde_json::json;
use tokio::time::timeout;

use super::fake_config;

async fn wait_until_transport_closes(server: &ResidentAppServer) {
    timeout(Duration::from_secs(2), async {
        loop {
            if server.lifecycle_snapshot().await.process_id.is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("transport close");
}

async fn wait_until_restart_pending(server: &ResidentAppServer) {
    timeout(Duration::from_secs(2), async {
        loop {
            if server.lifecycle_snapshot().await.restart_pending {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("resident death propagation");
}

#[tokio::test]
async fn active_turn_death_stays_fail_closed_until_exact_snapshot_is_settled() {
    let server = ResidentAppServer::start(fake_config())
        .await
        .expect("start");
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
        timeout(Duration::from_secs(2), notifications.recv())
            .await
            .expect("turn started timeout")
            .expect("turn started")
    else {
        panic!("unexpected notification gap");
    };
    assert_eq!(notification.method, "turn/started");
    wait_until_transport_closes(&server).await;
    wait_until_restart_pending(&server).await;

    assert!(matches!(
        server
            .request("test/echo", json!({}), Duration::from_secs(1), Some(1))
            .await,
        Err(cdr_app_server::AppServerError::Closed)
    ));

    assert!(
        !server
            .force_restart_if_quiescent()
            .await
            .expect("fail closed")
    );
    assert_eq!(server.generation(), 1);

    let work = server
        .dead_generation_work(1)
        .await
        .expect("generation CAS")
        .expect("dead generation work");
    assert_eq!(work.generation, 1);
    assert_eq!(work.closed_reason, "app-server stdout closed");
    assert_eq!(
        work.active_turns,
        vec![DeadActiveTurn {
            thread_id: "thread-a".to_owned(),
            turn_id: "turn-a".to_owned(),
        }]
    );
    assert!(work.server_requests.is_empty());
    assert_eq!(
        server
            .settle_dead_generation(1, &work)
            .await
            .expect("exact settlement"),
        DeadGenerationSettleResult::Settled
    );
    assert!(
        server
            .force_restart_if_quiescent()
            .await
            .expect("settled restart")
    );
    assert_eq!(server.generation(), 2);
    server.close().await.expect("close");
}

#[tokio::test]
async fn unsettled_request_death_stays_fail_closed_until_exact_snapshot_is_settled() {
    let server = ResidentAppServer::start(fake_config())
        .await
        .expect("start");
    let mut requests = server.subscribe_server_requests();
    server
        .request(
            "test/requestApprovalThenExit",
            json!({}),
            Duration::from_secs(1),
            Some(1),
        )
        .await
        .expect("request and exit");
    let ResidentServerRequestEvent::Request { request, .. } =
        timeout(Duration::from_secs(2), requests.recv())
            .await
            .expect("server request timeout")
            .expect("server request")
    else {
        panic!("unexpected request gap");
    };
    wait_until_transport_closes(&server).await;
    wait_until_restart_pending(&server).await;

    assert!(matches!(
        server
            .request("test/echo", json!({}), Duration::from_secs(1), Some(1))
            .await,
        Err(cdr_app_server::AppServerError::Closed)
    ));

    assert!(
        !server
            .force_restart_if_quiescent()
            .await
            .expect("fail closed")
    );
    assert_eq!(server.generation(), 1);

    let work = server
        .dead_generation_work(1)
        .await
        .expect("generation CAS")
        .expect("dead generation work");
    assert!(work.active_turns.is_empty());
    assert_eq!(
        work.server_requests,
        vec![DeadServerRequest {
            id: request.id.clone(),
            occurrence: request.occurrence,
            method: request.method.clone(),
            params: request.params.clone(),
        }]
    );

    let mut tampered_method = work.clone();
    tampered_method.server_requests[0].method = "item/tool/requestUserInput".to_owned();
    assert_eq!(
        server
            .settle_dead_generation(1, &tampered_method)
            .await
            .expect("tampered method"),
        DeadGenerationSettleResult::SnapshotChanged
    );
    let mut tampered_params = work.clone();
    tampered_params.server_requests[0].params["threadId"] = json!("other-thread");
    assert_eq!(
        server
            .settle_dead_generation(1, &tampered_params)
            .await
            .expect("tampered params"),
        DeadGenerationSettleResult::SnapshotChanged
    );
    assert_eq!(
        server.dead_generation_work(1).await.expect("work retained"),
        Some(work.clone())
    );
    assert_eq!(
        server
            .settle_dead_generation(1, &work)
            .await
            .expect("exact settlement"),
        DeadGenerationSettleResult::Settled
    );
    assert!(
        server
            .force_restart_if_quiescent()
            .await
            .expect("settled restart")
    );
    assert_eq!(server.generation(), 2);
    server.close().await.expect("close");
}
