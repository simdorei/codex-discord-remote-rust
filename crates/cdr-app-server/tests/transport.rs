use std::sync::Arc;
use std::time::Duration;

use cdr_app_server::{
    AppServerClient, AppServerConfig, AppServerError, ResidentAppServer, ResidentNotificationEvent,
};
use serde_json::json;
use tokio::time::timeout;

fn fake_config() -> AppServerConfig {
    let mut config = AppServerConfig::new(env!("CARGO_BIN_EXE_fake_codex_app_server"));
    config.arguments.clear();
    config
}

#[tokio::test]
async fn initializes_with_experimental_api_and_round_trips_requests() {
    let client = AppServerClient::start(fake_config())
        .await
        .expect("start fake app-server");
    let snapshot = client.lifecycle_snapshot();
    assert!(snapshot.healthy);
    assert_eq!(snapshot.generation, 1);

    let result = client
        .request("test/echo", json!({"value": 42}), Duration::from_secs(1))
        .await
        .expect("echo response");
    assert_eq!(result, json!({"value": 42}));
    client.close().await.expect("close app-server");
    assert!(!client.lifecycle_snapshot().healthy);
}

#[tokio::test]
async fn routes_notifications_server_requests_and_active_turn_state() {
    let client = AppServerClient::start(fake_config())
        .await
        .expect("start fake app-server");
    let mut notifications = client.subscribe_notifications();
    let mut requests = client.subscribe_server_requests();

    client
        .request("test/notify", json!({}), Duration::from_secs(1))
        .await
        .expect("emit notification");
    let notification = notifications.recv().await.expect("turn notification");
    assert_eq!(notification.method, "turn/started");
    assert_eq!(client.active_turn_id("thread-a").as_deref(), Some("turn-a"));

    client
        .request("test/requestApproval", json!({}), Duration::from_secs(1))
        .await
        .expect("request approval");
    let request = requests.recv().await.expect("server request");
    assert_eq!(request.method, "item/commandExecution/requestApproval");
    assert_eq!(
        client.latest_approval_request("thread-a"),
        Some(request.clone())
    );
    client
        .respond(
            &request.id,
            request.occurrence,
            json!({"decision": "accept"}),
        )
        .await
        .expect("approval response");
    assert!(client.pending_server_requests(Some("thread-a")).is_empty());

    client
        .request("test/complete", json!({}), Duration::from_secs(1))
        .await
        .expect("complete turn");
    assert_eq!(client.active_turn_id("thread-a"), None);
    client.close().await.expect("close app-server");
}

#[tokio::test]
async fn bounds_diagnostics_and_surfaces_timeouts() {
    let client = AppServerClient::start(fake_config())
        .await
        .expect("start fake app-server");
    client
        .request("test/invalid", json!({}), Duration::from_secs(1))
        .await
        .expect("invalid line test response");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        client
            .diagnostic_snapshot()
            .lines
            .iter()
            .any(|line| line.contains("non-JSON"))
    );

    let error = client
        .request("test/timeout", json!({}), Duration::from_millis(20))
        .await
        .expect_err("timeout");
    assert!(matches!(error, AppServerError::Timeout { .. }));
    client.close().await.expect("close app-server");
}

#[tokio::test]
async fn quarantines_ambiguous_timeouts_and_restarts_only_when_quiescent() {
    let server = ResidentAppServer::start(fake_config())
        .await
        .expect("start resident server");
    let error = server
        .request("test/timeout", json!({}), Duration::from_millis(20), None)
        .await
        .expect_err("ambiguous timeout");
    assert!(matches!(error, AppServerError::Timeout { .. }));
    let snapshot = server.lifecycle_snapshot().await;
    assert!(snapshot.quarantined);
    assert!(snapshot.restart_pending);
    assert!(!snapshot.healthy);

    let blocked = server
        .request("test/echo", json!({}), Duration::from_secs(1), Some(1))
        .await
        .expect_err("quarantined generation");
    assert!(matches!(
        blocked,
        AppServerError::GenerationQuarantined { .. }
    ));
    assert!(server.restart_if_quiescent().await.expect("restart"));
    assert_eq!(server.lifecycle_snapshot().await.generation, 2);
    let result = server
        .request(
            "test/echo",
            json!({"ok": true}),
            Duration::from_secs(1),
            Some(2),
        )
        .await
        .expect("fresh generation request");
    assert_eq!(result, json!({"ok": true}));
    server.close().await.expect("close resident server");
}

#[tokio::test]
async fn transport_close_after_confirmed_write_is_typed_and_quarantines_generation() {
    let server = Arc::new(
        timeout(
            Duration::from_secs(10),
            ResidentAppServer::start(fake_config()),
        )
        .await
        .expect("start timeout")
        .expect("start resident server"),
    );
    let mut notifications = server.subscribe_notifications();
    let delayed = tokio::spawn({
        let server = Arc::clone(&server);
        async move {
            server
                .request(
                    "test/delayedEcho",
                    json!({"effect": "may-have-run"}),
                    Duration::from_secs(30),
                    Some(1),
                )
                .await
        }
    });

    let ResidentNotificationEvent::Notification {
        generation,
        notification,
    } = timeout(Duration::from_secs(2), notifications.recv())
        .await
        .expect("delayed-entered timeout")
        .expect("delayed-entered notification")
    else {
        panic!("unexpected notification gap");
    };
    assert_eq!(generation, 1);
    assert_eq!(notification.method, "test/delayedEntered");

    server
        .request(
            "test/startThenExit",
            json!({}),
            Duration::from_secs(2),
            Some(1),
        )
        .await
        .expect("trigger response before exit");
    let error = timeout(Duration::from_secs(2), delayed)
        .await
        .expect("delayed request completion timeout")
        .expect("delayed request task")
        .expect_err("transport closure must fail the delayed request");
    match error {
        AppServerError::TransportClosed { method, reason } => {
            assert_eq!(method, "test/delayedEcho");
            assert_eq!(reason, "app-server stdout closed");
        }
        other => panic!("expected typed transport closure, got {other:?}"),
    }

    let snapshot = server.lifecycle_snapshot().await;
    assert_eq!(snapshot.generation, 1);
    assert!(!snapshot.healthy);
    assert_eq!(snapshot.process_id, None);
    assert!(snapshot.quarantined);
    assert!(snapshot.restart_pending);
    timeout(Duration::from_secs(10), server.close())
        .await
        .expect("close timeout")
        .expect("close resident server");
}

#[tokio::test]
async fn cancels_pending_server_requests_with_standard_error() {
    let client = AppServerClient::start(fake_config())
        .await
        .expect("start fake app-server");
    client
        .request("test/requestApproval", json!({}), Duration::from_secs(1))
        .await
        .expect("approval request");
    assert_eq!(
        client
            .cancel_pending_server_requests("thread-a")
            .await
            .expect("cancel"),
        1
    );
    assert!(client.pending_server_requests(Some("thread-a")).is_empty());
    client.close().await.expect("close app-server");
}
