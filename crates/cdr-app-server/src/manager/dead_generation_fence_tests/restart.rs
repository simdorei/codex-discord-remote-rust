use super::{RecordingFence, ResidentAppServer, helper_config};
use crate::AppServerError;
use crate::client::WriteTestPause;
use crate::manager::close_retry_tests::test_server;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::test]
async fn os_exit_before_closure_publication_cannot_skip_empty_generation_capture() {
    // This fixture retains a real process without an async stdout reader, so
    // death publication is delayed deterministically instead of by scheduling.
    let (mut server, client) = test_server();
    let fence = Arc::new(RecordingFence::default());
    server.config = helper_config();
    server.dead_generation_fence = Some(fence.clone());
    server.state.mark_timeout(1);
    {
        let mut child = client.inner.child.lock().await;
        let child = child.as_mut().unwrap();
        child.start_kill().unwrap();
        timeout(TEST_TIMEOUT, child.wait()).await.unwrap().unwrap();
        assert!(child.try_wait().unwrap().is_some());
    }
    assert!(client.lifecycle_snapshot().closed_reason.is_none());

    let unpublished = server.force_restart_if_quiescent().await;
    let unpublished_generation = server.generation();
    let unpublished_captures = fence.0.lock().unwrap().len();
    crate::transport::mark_closed(&client.inner, "delayed test process exit");
    let published = server.force_restart_if_quiescent().await;
    let published_generation = server.generation();
    server.close().await.unwrap();

    assert!(
        !unpublished.unwrap(),
        "OS-confirmed exit must not use the normal live-idle restart shortcut"
    );
    assert_eq!(unpublished_generation, 1);
    assert_eq!(unpublished_captures, 0);
    assert!(published.unwrap());
    assert_eq!(published_generation, 2);
    let captures = fence.0.lock().unwrap();
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].generation, 1);
    assert!(captures[0].is_empty());
}

#[tokio::test]
async fn fenced_idle_restart_can_retry_after_replacement_initialize_cancellation() {
    let fence = Arc::new(RecordingFence::default());
    let server = Arc::new(
        ResidentAppServer::start_with_dead_generation_fence(helper_config(), fence.clone())
            .await
            .unwrap(),
    );
    let old = server.state.current_client().unwrap();
    server.state.mark_timeout(1);
    let pause = Arc::new(WriteTestPause::new());
    let pause_for_start = pause.clone();
    let (observed_tx, observed_rx) = tokio::sync::oneshot::channel();
    let restart = tokio::spawn({
        let server = server.clone();
        async move {
            server
                .restart_if_quiescent_observed(move |client| {
                    *client.inner.write_pause.lock().unwrap() = Some(pause_for_start);
                    assert!(observed_tx.send(client.clone()).is_ok());
                })
                .await
        }
    });
    let replacement = timeout(TEST_TIMEOUT, observed_rx).await.unwrap().unwrap();
    timeout(TEST_TIMEOUT, pause.wait_until_entered())
        .await
        .unwrap();
    restart.abort();
    assert!(restart.await.unwrap_err().is_cancelled());
    assert!(old.inner.child.lock().await.is_none());
    assert!(server.state.has_replacement_cleanup());
    assert!(fence.0.lock().unwrap().is_empty());

    let retry = timeout(TEST_TIMEOUT, server.restart_if_quiescent())
        .await
        .unwrap();
    let generation = server.generation();
    let candidate_reaped = replacement.inner.child.lock().await.is_none();
    server.close().await.unwrap();

    assert!(
        retry.unwrap(),
        "intentional old-client cleanup must not wedge the replacement retry"
    );
    assert_eq!(generation, 2);
    assert!(candidate_reaped);
    assert!(fence.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn fenced_idle_restart_can_retry_after_replacement_initialize_failure() {
    let fence = Arc::new(RecordingFence::default());
    let mut server =
        ResidentAppServer::start_with_dead_generation_fence(helper_config(), fence.clone())
            .await
            .unwrap();
    server
        .config
        .environment
        .insert("CDR_STARTUP_INITIALIZE_ERROR".to_owned(), "1".to_owned());
    server.state.mark_timeout(1);
    let first = server.restart_if_quiescent().await;
    assert!(matches!(first, Err(AppServerError::Remote { .. })));
    assert_eq!(server.generation(), 1);
    server
        .config
        .environment
        .remove("CDR_STARTUP_INITIALIZE_ERROR");

    let retry = server.restart_if_quiescent().await;
    let generation = server.generation();
    server.close().await.unwrap();
    assert!(
        retry.unwrap(),
        "failed replacement startup must be retryable"
    );
    assert_eq!(generation, 2);
    assert!(fence.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn missing_child_without_cleanup_authorization_does_not_prove_exit() {
    let (mut server, client) = test_server();
    let fence = Arc::new(RecordingFence::default());
    server.dead_generation_fence = Some(fence.clone());
    let mut detached_child = client.inner.child.lock().await.take().unwrap();
    crate::transport::mark_closed(&client.inner, "test unproven process state");
    let restart = server.force_restart_if_quiescent().await;
    let generation = server.generation();
    detached_child.start_kill().unwrap();
    timeout(TEST_TIMEOUT, detached_child.wait())
        .await
        .unwrap()
        .unwrap();
    server.close().await.unwrap();

    assert!(!restart.unwrap());
    assert_eq!(generation, 1);
    assert!(fence.0.lock().unwrap().is_empty());
}
