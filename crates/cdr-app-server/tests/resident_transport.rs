use std::time::Duration;

use cdr_app_server::{
    AppServerConfig, ResidentAppServer, ResidentNotificationEvent, ResidentServerRequestEvent,
    ServerRequest,
};
use serde_json::json;
use tokio::sync::broadcast;

#[path = "resident_transport/dead_generation.rs"]
mod dead_generation;
#[path = "resident_transport/fail_closed.rs"]
mod fail_closed;
#[path = "resident_transport/races.rs"]
mod races;

fn fake_config() -> AppServerConfig {
    let mut config = AppServerConfig::new(env!("CARGO_BIN_EXE_fake_codex_app_server"));
    config.arguments.clear();
    config
}

async fn approval_for_generation(
    server: &ResidentAppServer,
    requests: &mut broadcast::Receiver<ResidentServerRequestEvent>,
    generation: u64,
) -> ServerRequest {
    server
        .request(
            "test/requestApproval",
            json!({}),
            Duration::from_secs(1),
            Some(generation),
        )
        .await
        .expect("resident request");
    let ResidentServerRequestEvent::Request {
        generation: actual,
        request,
    } = requests.recv().await.expect("request event")
    else {
        panic!("unexpected request gap");
    };
    assert_eq!(actual, generation);
    request
}

#[tokio::test]
async fn event_subscriptions_survive_restart_and_responses_are_generation_bound() {
    let server = ResidentAppServer::start(fake_config())
        .await
        .expect("start");
    let mut notifications = server.subscribe_notifications();
    let mut requests = server.subscribe_server_requests();
    server
        .request("test/notify", json!({}), Duration::from_secs(1), Some(1))
        .await
        .expect("generation one notification");
    let ResidentNotificationEvent::Notification {
        generation,
        notification,
    } = notifications.recv().await.expect("generation one event")
    else {
        panic!("unexpected notification gap");
    };
    assert_eq!(generation, 1);
    assert_eq!(notification.method, "turn/started");

    let approval = approval_for_generation(&server, &mut requests, 1).await;
    server
        .respond(
            &approval.id,
            approval.occurrence,
            json!({"decision":"accept"}),
            1,
        )
        .await
        .expect("generation-bound response");
    server
        .request("test/complete", json!({}), Duration::from_secs(1), Some(1))
        .await
        .expect("complete turn");
    loop {
        let ResidentNotificationEvent::Notification { notification, .. } =
            notifications.recv().await.expect("completion event")
        else {
            panic!("unexpected notification gap");
        };
        if notification.method == "turn/completed" {
            break;
        }
    }

    let _ = server
        .request(
            "test/timeout",
            json!({}),
            Duration::from_millis(20),
            Some(1),
        )
        .await
        .expect_err("quarantine generation one");
    assert!(server.restart_if_quiescent().await.expect("restart"));
    assert!(
        server
            .respond(&approval.id, approval.occurrence, json!({}), 1)
            .await
            .is_err()
    );

    server
        .request("test/notify", json!({}), Duration::from_secs(1), Some(2))
        .await
        .expect("generation two notification");
    let ResidentNotificationEvent::Notification { generation, .. } =
        notifications.recv().await.expect("generation two event")
    else {
        panic!("unexpected notification gap");
    };
    assert_eq!(generation, 2);
    let replacement = approval_for_generation(&server, &mut requests, 2).await;
    assert_eq!(replacement.id, approval.id);
    assert_ne!(replacement.occurrence, approval.occurrence);
    server
        .respond(&replacement.id, replacement.occurrence, json!({}), 2)
        .await
        .expect("generation two response");
    server.close().await.expect("close");
}

#[tokio::test]
async fn unsettled_server_requests_block_resident_restart_until_exact_response() {
    let server = ResidentAppServer::start(fake_config())
        .await
        .expect("start");
    let mut requests = server.subscribe_server_requests();
    server
        .request(
            "test/requestApproval",
            json!({}),
            Duration::from_secs(1),
            Some(1),
        )
        .await
        .expect("request approval");
    let ResidentServerRequestEvent::Request { request, .. } =
        requests.recv().await.expect("request event")
    else {
        panic!("unexpected request gap");
    };
    assert!(server.has_unsettled_server_requests().await.expect("state"));
    assert!(!server.force_restart_if_quiescent().await.expect("blocked"));

    server
        .respond(&request.id, request.occurrence, json!({}), 1)
        .await
        .expect("respond exact occurrence");
    assert!(!server.has_unsettled_server_requests().await.expect("state"));
    server.close().await.expect("close");
}
