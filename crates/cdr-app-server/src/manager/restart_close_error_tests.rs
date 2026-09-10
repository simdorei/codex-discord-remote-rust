use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::json;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::ResidentAppServer;
use crate::client::WriteTestPause;
use crate::client::startup_tests::{
    helper_config, wait_until_cleanup_complete, wait_until_inner_released,
};
use crate::{AppServerClient, AppServerError};

const TEST_TIMEOUT: Duration = Duration::from_secs(5);
const OLD_CLOSE_FAILURE: &str = "injected resident close failure";
const DEBT_CLOSE_FAILURE: &str = "injected replacement cleanup failure";

struct PausedRestart {
    candidate: AppServerClient,
    task: JoinHandle<Result<bool, AppServerError>>,
}

async fn pause_replacement(server: &Arc<ResidentAppServer>) -> PausedRestart {
    server.state.request_restart();
    let pause = Arc::new(WriteTestPause::new());
    let pause_for_start = Arc::clone(&pause);
    let (observed_tx, observed_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn({
        let server = Arc::clone(server);
        async move {
            server
                .restart_if_quiescent_observed(move |client| {
                    *client.inner.write_pause.lock().expect("write pause lock") =
                        Some(pause_for_start);
                    assert!(observed_tx.send(client.clone()).is_ok());
                })
                .await
        }
    });
    let candidate = timeout(TEST_TIMEOUT, observed_rx)
        .await
        .expect("replacement observer timed out")
        .expect("replacement observer dropped");
    timeout(TEST_TIMEOUT, pause.wait_until_entered())
        .await
        .expect("replacement initialize write did not enter pause");
    PausedRestart { candidate, task }
}

async fn abort_restart(task: JoinHandle<Result<bool, AppServerError>>) {
    task.abort();
    assert!(
        task.await
            .expect_err("replacement startup unexpectedly completed")
            .is_cancelled()
    );
}

fn assert_io_error(error: &AppServerError, expected: &str) {
    assert!(
        matches!(error, AppServerError::Io(source) if source.to_string() == expected),
        "unexpected restart error: {error}"
    );
}

#[tokio::test]
async fn old_close_error_preserves_sealed_generation_and_ordinary_retry_installs_generation_two() {
    let server = ResidentAppServer::start(helper_config())
        .await
        .expect("start resident server");
    let original = server.state.snapshot().client.expect("generation 1 client");
    server.state.request_restart();
    let close_calls = Arc::new(AtomicUsize::new(0));
    let replacement_starts = Arc::new(AtomicUsize::new(0));
    let calls = Arc::clone(&close_calls);
    let starts = Arc::clone(&replacement_starts);
    let expected = original.clone();

    let error = timeout(
        TEST_TIMEOUT,
        server.restart_if_quiescent_observed_with_cleanup(
            move |_| {
                starts.fetch_add(1, Ordering::SeqCst);
            },
            move |client| {
                calls.fetch_add(1, Ordering::SeqCst);
                assert!(Arc::ptr_eq(&client.inner, &expected.inner));
                async { Err(AppServerError::Io(std::io::Error::other(OLD_CLOSE_FAILURE))) }
            },
        ),
    )
    .await
    .expect("restart resolution")
    .expect_err("injected old close failure");

    assert_io_error(&error, OLD_CLOSE_FAILURE);
    let failed = server.state.snapshot();
    assert_eq!(failed.generation, 1);
    assert!(!failed.accepting);
    assert!(failed.restart_pending);
    assert!(
        failed
            .client
            .as_ref()
            .is_some_and(|client| { Arc::ptr_eq(&client.inner, &original.inner) })
    );
    assert!(!server.state.has_replacement_cleanup());
    assert_eq!(*server.forwarder_generation.borrow(), 0);
    assert!(server.forwarders.lock().expect("forwarders lock").is_none());
    assert_eq!(close_calls.load(Ordering::SeqCst), 1);
    assert_eq!(replacement_starts.load(Ordering::SeqCst), 0);
    assert!(original.inner.child.lock().await.is_some());
    let rejected = server
        .request("test/echo", json!({}), Duration::from_secs(1), Some(1))
        .await
        .expect_err("sealed generation must reject admission after close failure");
    assert!(matches!(rejected, AppServerError::Closed));
    drop(failed);

    assert!(
        timeout(TEST_TIMEOUT, server.restart_if_quiescent())
            .await
            .expect("ordinary retry timed out")
            .expect("ordinary retry failed")
    );
    assert_generation_two_healthy(&server).await;
    assert!(original.inner.child.lock().await.is_none());
    assert!(original.inner.stdin.lock().await.is_none());
    assert_eq!(original.lifecycle_snapshot().process_id, None);
    let original_inner = Arc::downgrade(&original.inner);
    drop(original);
    timeout(TEST_TIMEOUT, wait_until_inner_released(&original_inner))
        .await
        .expect("old generation remained retained after successful retry");
    server.close().await.expect("close resident server");
}

#[tokio::test]
async fn replacement_cleanup_error_retains_exact_debt_and_retry_installs_generation_two() {
    let server = Arc::new(
        ResidentAppServer::start(helper_config())
            .await
            .expect("start resident server"),
    );
    let original = server.state.snapshot().client.expect("generation 1 client");
    let PausedRestart { candidate, task } = pause_replacement(&server).await;
    let child_guard = candidate.inner.child.lock().await;
    abort_restart(task).await;
    let debt_before = server
        .state
        .replacement_cleanup()
        .expect("cancelled replacement cleanup debt");
    assert_eq!(debt_before.generation, 2);
    assert!(Arc::ptr_eq(&debt_before.client.inner, &candidate.inner));

    let close_calls = Arc::new(AtomicUsize::new(0));
    let replacement_starts = Arc::new(AtomicUsize::new(0));
    let calls = Arc::clone(&close_calls);
    let starts = Arc::clone(&replacement_starts);
    let expected = candidate.clone();
    let error = timeout(
        TEST_TIMEOUT,
        server.restart_if_quiescent_observed_with_cleanup(
            move |_| {
                starts.fetch_add(1, Ordering::SeqCst);
            },
            move |client| {
                calls.fetch_add(1, Ordering::SeqCst);
                assert!(Arc::ptr_eq(&client.inner, &expected.inner));
                async {
                    Err(AppServerError::Io(std::io::Error::other(
                        DEBT_CLOSE_FAILURE,
                    )))
                }
            },
        ),
    )
    .await
    .expect("cleanup retry resolution")
    .expect_err("injected replacement cleanup failure");

    assert_io_error(&error, DEBT_CLOSE_FAILURE);
    let failed = server.state.snapshot();
    assert_eq!(failed.generation, 1);
    assert!(!failed.accepting);
    assert!(failed.restart_pending);
    assert!(
        failed
            .client
            .as_ref()
            .is_some_and(|client| { Arc::ptr_eq(&client.inner, &original.inner) })
    );
    let debt_after = server
        .state
        .replacement_cleanup()
        .expect("failed cleanup must retain debt");
    assert_eq!(debt_after.generation, debt_before.generation);
    assert!(Arc::ptr_eq(&debt_after.client.inner, &candidate.inner));
    assert_eq!(close_calls.load(Ordering::SeqCst), 1);
    assert_eq!(replacement_starts.load(Ordering::SeqCst), 0);
    assert_eq!(*server.forwarder_generation.borrow(), 0);
    assert!(server.forwarders.lock().expect("forwarders lock").is_none());

    drop(child_guard);
    assert!(
        timeout(TEST_TIMEOUT, server.restart_if_quiescent())
            .await
            .expect("ordinary retry timed out")
            .expect("ordinary retry failed")
    );
    assert_generation_two_healthy(&server).await;
    assert!(!server.state.has_replacement_cleanup());
    timeout(
        TEST_TIMEOUT,
        wait_until_cleanup_complete(&candidate, "app-server write outcome indeterminate"),
    )
    .await
    .expect("same replacement candidate was not reconciled");
    let current = server.state.snapshot().client.expect("generation 2 client");
    assert!(!Arc::ptr_eq(&current.inner, &candidate.inner));
    let candidate_inner = Arc::downgrade(&candidate.inner);
    drop(debt_before);
    drop(debt_after);
    drop(candidate);
    timeout(TEST_TIMEOUT, wait_until_inner_released(&candidate_inner))
        .await
        .expect("failed replacement remained retained after reconciliation");
    server.close().await.expect("close resident server");
}

async fn assert_generation_two_healthy(server: &ResidentAppServer) {
    let snapshot = server.lifecycle_snapshot().await;
    assert_eq!(snapshot.generation, 2);
    assert!(snapshot.healthy);
    assert!(!snapshot.quarantined);
    assert!(!snapshot.restart_pending);
    assert!(!server.state.has_replacement_cleanup());
    assert_eq!(*server.forwarder_generation.borrow(), 2);
    assert!(server.forwarders.lock().expect("forwarders lock").is_some());
    let echo = server
        .request("test/echo", json!({}), Duration::from_secs(1), Some(2))
        .await
        .expect("generation 2 echo");
    assert_eq!(echo, json!({}));
}
