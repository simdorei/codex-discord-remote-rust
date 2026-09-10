use std::sync::Arc;
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

const BLOCKED_WINDOW: Duration = Duration::from_secs(2);
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

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

async fn abort(task: JoinHandle<Result<bool, AppServerError>>) {
    task.abort();
    assert!(
        task.await
            .expect_err("replacement startup unexpectedly completed")
            .is_cancelled()
    );
}

#[tokio::test]
async fn cancelled_replacement_is_reconciled_before_immediate_retry() {
    let server = Arc::new(
        ResidentAppServer::start(helper_config())
            .await
            .expect("start resident server"),
    );
    let PausedRestart { candidate, task } = pause_replacement(&server).await;
    let child_guard = candidate.inner.child.lock().await;
    abort(task).await;

    let interrupted = server.state.snapshot();
    assert_eq!(interrupted.generation, 1);
    assert!(interrupted.restart_pending);
    assert!(!interrupted.accepting);
    assert!(server.state.has_replacement_cleanup());
    let rejected = server
        .request("test/echo", json!({}), Duration::from_secs(1), Some(1))
        .await
        .expect_err("sealed generation must reject requests");
    assert!(matches!(rejected, AppServerError::Closed));

    let mut retry = tokio::spawn({
        let server = Arc::clone(&server);
        async move { server.restart_if_quiescent().await }
    });
    let premature = timeout(BLOCKED_WINDOW, &mut retry).await;
    let waited_for_candidate = premature.is_err();
    drop(child_guard);
    let restarted = match premature {
        Ok(joined) => joined.expect("retry task"),
        Err(_) => timeout(TEST_TIMEOUT, retry)
            .await
            .expect("retry timed out after candidate release")
            .expect("retry task"),
    }
    .expect("retry replacement");

    assert!(restarted);
    assert_eq!(server.generation(), 2);
    assert!(!server.state.has_replacement_cleanup());
    assert_candidate_reaped(&candidate, "app-server write outcome indeterminate").await;
    let inner = Arc::downgrade(&candidate.inner);
    drop(candidate);
    timeout(TEST_TIMEOUT, wait_until_inner_released(&inner))
        .await
        .expect("replacement background tasks retained client state");
    server.close().await.expect("close resident server");
    assert!(
        waited_for_candidate,
        "retry installed generation 2 while the cancelled candidate was still owned"
    );
}

#[tokio::test]
async fn close_reconciles_cancelled_replacement_before_returning() {
    let server = Arc::new(
        ResidentAppServer::start(helper_config())
            .await
            .expect("start resident server"),
    );
    let PausedRestart { candidate, task } = pause_replacement(&server).await;
    let child_guard = candidate.inner.child.lock().await;
    abort(task).await;

    let mut close = tokio::spawn({
        let server = Arc::clone(&server);
        async move { server.close().await }
    });
    let premature = timeout(BLOCKED_WINDOW, &mut close).await;
    let waited_for_candidate = premature.is_err();
    drop(child_guard);
    match premature {
        Ok(joined) => joined.expect("close task"),
        Err(_) => timeout(TEST_TIMEOUT, close)
            .await
            .expect("close timed out after candidate release")
            .expect("close task"),
    }
    .expect("close resident server");

    assert!(!server.state.has_replacement_cleanup());
    assert_candidate_reaped(&candidate, "app-server write outcome indeterminate").await;
    let inner = Arc::downgrade(&candidate.inner);
    drop(candidate);
    timeout(TEST_TIMEOUT, wait_until_inner_released(&inner))
        .await
        .expect("terminal close left replacement tasks alive");
    assert!(
        waited_for_candidate,
        "resident close returned before the cancelled replacement was reaped"
    );
}

async fn assert_candidate_reaped(candidate: &AppServerClient, expected_reason: &str) {
    timeout(
        TEST_TIMEOUT,
        wait_until_cleanup_complete(candidate, expected_reason),
    )
    .await
    .expect("candidate cleanup did not complete");
    assert!(candidate.inner.child.lock().await.is_none());
    assert!(candidate.inner.stdin.lock().await.is_none());
    assert_eq!(candidate.lifecycle_snapshot().process_id, None);
}
