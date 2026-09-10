use std::time::Duration;

use tokio::time::timeout;

use super::startup_tests::{helper_config, wait_until_cleanup_complete};
use crate::AppServerError;
use crate::transport::mark_closed;

const CLOSE_REASON: &str = "transport closed after initialized write";
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::test]
async fn close_after_initialized_write_prevents_dead_client_commit_and_reaps_child() {
    let (observed_tx, observed_rx) = tokio::sync::oneshot::channel();
    let result = timeout(
        TEST_TIMEOUT,
        super::startup::start_observed_after_initialized(
            helper_config(),
            move |client| assert!(observed_tx.send(client.clone()).is_ok()),
            |client| mark_closed(&client.inner, CLOSE_REASON),
        ),
    )
    .await
    .expect("startup close-race test timed out");
    let client = observed_rx.await.expect("startup observer dropped");

    let error = match result {
        Err(error) => error,
        Ok((returned, ())) => {
            let snapshot = returned.lifecycle_snapshot();
            timeout(TEST_TIMEOUT, returned.close())
                .await
                .expect("fallback cleanup timed out")
                .expect("fallback cleanup failed");
            panic!("startup returned a closed client after transport closure: {snapshot:?}");
        }
    };

    assert!(matches!(error, AppServerError::Closed));
    timeout(
        TEST_TIMEOUT,
        wait_until_cleanup_complete(&client, CLOSE_REASON),
    )
    .await
    .expect("startup closure did not fully reap the child");
    let snapshot = client.lifecycle_snapshot();
    assert!(!snapshot.healthy);
    assert!(!snapshot.initialized);
    assert_eq!(snapshot.generation, 0);
    assert!(snapshot.process_id.is_none());
    assert_eq!(snapshot.closed_reason.as_deref(), Some(CLOSE_REASON));
    assert_eq!(client.wait_closed().await, CLOSE_REASON);
}
