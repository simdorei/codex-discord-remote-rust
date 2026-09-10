use std::time::Duration;

use cdr_app_server::{AppServerClient, AppServerConfig, AppServerError};
use serde_json::json;
use tokio::time::timeout;

fn fake_config() -> AppServerConfig {
    let mut config = AppServerConfig::new(env!("CARGO_BIN_EXE_fake_codex_app_server"));
    config.arguments.clear();
    config
}

async fn trigger_transport_close(client: &AppServerClient) -> String {
    let mut notifications = client.subscribe_notifications();
    let delayed = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request(
                    "test/delayedEcho",
                    json!({"effect": "may-have-run"}),
                    Duration::from_secs(30),
                )
                .await
        }
    });

    let notification = timeout(Duration::from_secs(2), notifications.recv())
        .await
        .expect("delayed-entered timeout")
        .expect("delayed-entered notification");
    assert_eq!(notification.method, "test/delayedEntered");
    client
        .request("test/startThenExit", json!({}), Duration::from_secs(2))
        .await
        .expect("trigger response before exit");
    let error = timeout(Duration::from_secs(2), delayed)
        .await
        .expect("delayed request completion timeout")
        .expect("delayed request task")
        .expect_err("transport closure must fail the delayed request");
    assert!(matches!(
        error,
        AppServerError::TransportClosed { ref reason, .. }
            if reason == "app-server stdout closed"
    ));
    match error {
        AppServerError::TransportClosed { reason, .. } => reason,
        other => panic!("expected transport closure, got {other:?}"),
    }
}

#[tokio::test]
async fn explicit_close_does_not_overwrite_the_first_transport_close_reason() {
    let client = AppServerClient::start(fake_config())
        .await
        .expect("start fake app-server");
    assert_eq!(
        trigger_transport_close(&client).await,
        "app-server stdout closed"
    );
    assert_eq!(
        timeout(Duration::from_secs(2), client.wait_closed())
            .await
            .expect("late waiter replay timeout"),
        "app-server stdout closed"
    );

    client.close().await.expect("cleanup closed app-server");
    client.close().await.expect("idempotent cleanup retry");
    assert_eq!(
        client.lifecycle_snapshot().closed_reason.as_deref(),
        Some("app-server stdout closed")
    );
}

#[tokio::test]
async fn intentional_close_latches_its_reason_before_transport_eof() {
    let client = AppServerClient::start(fake_config())
        .await
        .expect("start fake app-server");

    client.close().await.expect("close app-server");
    assert_eq!(
        client.lifecycle_snapshot().closed_reason.as_deref(),
        Some("closed by client")
    );
    assert_eq!(
        timeout(Duration::from_secs(2), client.wait_closed())
            .await
            .expect("intentional-close replay timeout"),
        "closed by client"
    );
}

#[tokio::test]
async fn wait_closed_is_replayable_and_a_cancelled_waiter_does_not_consume_it() {
    let client = AppServerClient::start(fake_config())
        .await
        .expect("start fake app-server");

    let cancelled = tokio::spawn({
        let client = client.clone();
        async move { client.wait_closed().await }
    });
    tokio::task::yield_now().await;
    cancelled.abort();
    assert!(
        cancelled
            .await
            .expect_err("cancelled waiter must stop")
            .is_cancelled()
    );

    let waiting_before_close = tokio::spawn({
        let client = client.clone();
        async move { client.wait_closed().await }
    });
    tokio::task::yield_now().await;
    assert_eq!(
        trigger_transport_close(&client).await,
        "app-server stdout closed"
    );
    assert_eq!(
        timeout(Duration::from_secs(2), waiting_before_close)
            .await
            .expect("pre-close waiter timeout")
            .expect("pre-close waiter task"),
        "app-server stdout closed"
    );
    assert_eq!(
        timeout(Duration::from_secs(2), client.wait_closed())
            .await
            .expect("post-close replay timeout"),
        "app-server stdout closed"
    );

    client.close().await.expect("cleanup closed app-server");
    assert_eq!(
        timeout(Duration::from_secs(2), client.wait_closed())
            .await
            .expect("post-cleanup replay timeout"),
        "app-server stdout closed"
    );
}
