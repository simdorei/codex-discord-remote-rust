use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::sync::watch;
use tokio::time::timeout;

use super::ResidentAppServer;
use crate::client::startup_tests::{
    helper_config, wait_until_cleanup_complete, wait_until_inner_released,
};
use crate::{AppServerClient, AppServerError};

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::test]
async fn explicit_replacement_startup_failure_clears_debt_and_allows_retry() {
    let server = ResidentAppServer::start(helper_config())
        .await
        .expect("start resident server");
    server.state.request_restart();
    let captured = Arc::new(Mutex::new(None::<AppServerClient>));
    let captured_by_observer = Arc::clone(&captured);

    let error = server
        .restart_if_quiescent_observed(move |client| {
            *captured_by_observer
                .lock()
                .expect("captured replacement lock") = Some(client.clone());
            crate::transport::mark_closed(&client.inner, "injected replacement startup failure");
        })
        .await
        .expect_err("closed replacement startup must fail");

    assert!(matches!(error, AppServerError::Closed));
    let failed = captured
        .lock()
        .expect("captured replacement lock")
        .take()
        .expect("replacement candidate was observed");
    let interrupted = server.state.snapshot();
    assert_eq!(interrupted.generation, 1);
    assert!(!interrupted.accepting);
    assert!(interrupted.restart_pending);
    assert_eq!(*server.forwarder_generation.borrow(), 0);
    assert!(
        server
            .forwarders
            .lock()
            .expect("resident forwarders lock")
            .is_none()
    );
    assert!(!server.state.has_replacement_cleanup());

    timeout(
        TEST_TIMEOUT,
        wait_until_cleanup_complete(&failed, "injected replacement startup failure"),
    )
    .await
    .expect("failed replacement was not reaped");
    let failed_inner = Arc::downgrade(&failed.inner);
    drop(failed);
    drop(captured);
    timeout(TEST_TIMEOUT, wait_until_inner_released(&failed_inner))
        .await
        .expect("failed replacement state remained retained");

    assert!(server.restart_if_quiescent().await.expect("retry restart"));
    assert_eq!(server.generation(), 2);
    assert!(!server.state.has_replacement_cleanup());
    let echo = server
        .request("test/echo", json!({}), Duration::from_secs(1), Some(2))
        .await
        .expect("generation 2 echo");
    assert_eq!(echo, json!({}));
    server.close().await.expect("close resident server");
}

#[tokio::test]
async fn closed_after_successful_startup_before_install_rolls_back_and_retries_generation_two() {
    const CLOSE_REASON: &str = "injected pre-install replacement failure";

    let server = ResidentAppServer::start(helper_config())
        .await
        .expect("start resident server");
    let original = server.state.snapshot().client.expect("generation 1 client");
    server.state.request_restart();
    let captured = Arc::new(Mutex::new(None::<AppServerClient>));
    let captured_by_hook = Arc::clone(&captured);
    let activation = Arc::new(Mutex::new(None::<watch::Receiver<bool>>));
    let activation_by_hook = Arc::clone(&activation);

    let result = timeout(
        TEST_TIMEOUT,
        server.restart_if_quiescent_with_before_install(move |client, forwarders| {
            assert!(
                !*forwarders.activation.borrow(),
                "replacement forwarders activated before installation"
            );
            *activation_by_hook.lock().expect("captured activation lock") =
                Some(forwarders.activation.subscribe());
            *captured_by_hook.lock().expect("captured replacement lock") = Some(client.clone());
            crate::transport::mark_closed(&client.inner, CLOSE_REASON);
        }),
    )
    .await
    .expect("pre-install replacement failure did not resolve");
    let error = match result {
        Err(error) => error,
        Ok(installed) => {
            server
                .close()
                .await
                .expect("clean up incorrectly installed replacement");
            panic!(
                "closed pre-install replacement returned Ok({installed}) instead of AppServerError::Closed"
            );
        }
    };

    assert!(matches!(error, AppServerError::Closed));
    let failed = captured
        .lock()
        .expect("captured replacement lock")
        .take()
        .expect("pre-install replacement was not captured");
    let failed_activation = activation
        .lock()
        .expect("captured activation lock")
        .take()
        .expect("pre-install activation receiver was not captured");
    assert!(
        !*failed_activation.borrow(),
        "failed replacement forwarders were activated"
    );
    assert!(
        failed_activation.has_changed().is_err(),
        "failed replacement retained an open activation sender"
    );

    assert_pre_install_failure_state(&server, &original);

    timeout(
        TEST_TIMEOUT,
        wait_until_cleanup_complete(&failed, CLOSE_REASON),
    )
    .await
    .expect("failed pre-install replacement was not reaped");
    let failed_inner = Arc::downgrade(&failed.inner);
    drop(failed_activation);
    drop(failed);
    drop(captured);
    drop(activation);
    timeout(TEST_TIMEOUT, wait_until_inner_released(&failed_inner))
        .await
        .expect("failed pre-install replacement remained retained");

    assert!(
        timeout(TEST_TIMEOUT, server.restart_if_quiescent())
            .await
            .expect("generation 2 retry timed out")
            .expect("generation 2 retry failed")
    );
    assert_eq!(server.generation(), 2);
    assert!(!server.state.has_replacement_cleanup());
    assert_eq!(*server.forwarder_generation.borrow(), 2);
    assert!(server.forwarders.lock().expect("forwarders lock").is_some());
    let echo = server
        .request("test/echo", json!({}), Duration::from_secs(1), Some(2))
        .await
        .expect("generation 2 echo");
    assert_eq!(echo, json!({}));
    server.close().await.expect("close resident server");
}

fn assert_pre_install_failure_state(server: &ResidentAppServer, original: &AppServerClient) {
    let interrupted = server.state.snapshot();
    assert_eq!(interrupted.generation, 1);
    assert!(!interrupted.accepting);
    assert!(interrupted.restart_pending);
    assert!(
        interrupted
            .client
            .as_ref()
            .is_some_and(|client| Arc::ptr_eq(&client.inner, &original.inner)),
        "failed replacement displaced the generation 1 client"
    );
    assert_eq!(*server.forwarder_generation.borrow(), 0);
    assert!(
        server
            .forwarders
            .lock()
            .expect("resident forwarders lock")
            .is_none()
    );
    assert!(!server.state.has_replacement_cleanup());
}
