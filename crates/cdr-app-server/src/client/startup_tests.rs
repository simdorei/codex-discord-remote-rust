use std::io::{BufRead, Write};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::time::timeout;

use super::{AppServerClient, AppServerConfig, WriteTestPause};
use crate::AppServerError;

const CHILD_ENV: &str = "CDR_STARTUP_CANCEL_HELPER_CHILD";
const INITIALIZE_ERROR_ENV: &str = "CDR_STARTUP_INITIALIZE_ERROR";
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

#[test]
#[ignore = "spawned by startup cancellation tests"]
fn startup_helper_child_process() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }

    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let message: serde_json::Value = serde_json::from_str(&line).expect("JSON-RPC request");
        let Some(id) = message.get("id") else {
            continue;
        };
        let response = if std::env::var_os(INITIALIZE_ERROR_ENV).is_some()
            && message.get("method").and_then(serde_json::Value::as_str) == Some("initialize")
        {
            serde_json::json!({
                "id": id,
                "error": {"code": -32_001, "message": "injected initialization failure"}
            })
        } else {
            serde_json::json!({"id": id, "result": {}})
        };
        serde_json::to_writer(&mut stdout, &response).expect("write JSON-RPC response");
        writeln!(stdout).expect("terminate JSON-RPC response");
        stdout.flush().expect("flush JSON-RPC response");
    }
}

#[tokio::test]
async fn abort_after_initialize_write_automatically_reaps_child() {
    let pause = Arc::new(WriteTestPause::new());
    let pause_for_start = Arc::clone(&pause);
    let (observed_tx, observed_rx) = tokio::sync::oneshot::channel();
    let start = tokio::spawn(async move {
        AppServerClient::start_observed(helper_config(), move |client| {
            *client.inner.write_pause.lock().expect("write pause lock") = Some(pause_for_start);
            assert!(observed_tx.send(client.clone()).is_ok());
        })
        .await
    });

    let client = timeout(TEST_TIMEOUT, observed_rx)
        .await
        .expect("startup observer timed out")
        .expect("startup observer dropped");
    timeout(TEST_TIMEOUT, pause.wait_until_entered())
        .await
        .expect("initialize write did not enter deterministic pause");

    start.abort();
    let Err(join_error) = start.await else {
        panic!("startup unexpectedly completed");
    };
    assert!(
        join_error.is_cancelled(),
        "startup task did not report cancellation"
    );

    let automatic = timeout(
        TEST_TIMEOUT,
        wait_until_cleanup_complete(&client, "app-server write outcome indeterminate"),
    )
    .await;
    let leaked = if automatic.is_err() {
        Some(leak_snapshot(&client).await)
    } else {
        None
    };
    if automatic.is_err() {
        timeout(TEST_TIMEOUT, client.close())
            .await
            .expect("fallback cleanup timed out")
            .expect("fallback cleanup failed");
    }
    let inner = Arc::downgrade(&client.inner);
    drop(client);
    let released = timeout(TEST_TIMEOUT, wait_until_inner_released(&inner)).await;

    assert!(
        automatic.is_ok(),
        "startup cancellation did not automatically clean up: {leaked:?}"
    );
    assert!(
        released.is_ok(),
        "startup cancellation left background tasks owning client state"
    );
}

#[tokio::test]
async fn initialization_error_preserves_cleanup_failure() {
    let mut config = helper_config();
    config
        .environment
        .insert(INITIALIZE_ERROR_ENV.to_owned(), "1".to_owned());
    let (observed_tx, observed_rx) = tokio::sync::oneshot::channel();
    let result = super::startup::start_observed_with_cleanup(
        config,
        move |client| assert!(observed_tx.send(client.clone()).is_ok()),
        |client| async move {
            client.close().await?;
            Err(AppServerError::Io(std::io::Error::other(
                "injected startup cleanup failure",
            )))
        },
    )
    .await;
    let Err(error) = result else {
        panic!("injected initialization error unexpectedly succeeded");
    };
    let client = timeout(TEST_TIMEOUT, observed_rx)
        .await
        .expect("startup observer timed out")
        .expect("startup observer dropped");

    let AppServerError::StartupCleanup { primary, cleanup } = error else {
        panic!("startup did not return a composite error: {error}");
    };
    match *primary {
        AppServerError::Remote {
            method,
            code,
            message,
            ..
        } => {
            assert_eq!(method, "initialize");
            assert_eq!(code, -32_001);
            assert_eq!(message, "injected initialization failure");
        }
        error => panic!("unexpected primary startup error: {error}"),
    }
    assert!(matches!(
        *cleanup,
        AppServerError::Io(ref source)
            if source.to_string() == "injected startup cleanup failure"
    ));
    timeout(
        TEST_TIMEOUT,
        wait_until_cleanup_complete(&client, "closed by client"),
    )
    .await
    .expect("cleanup completed but client state stayed live");
}

pub(crate) fn helper_config() -> AppServerConfig {
    let mut config = AppServerConfig::new(std::env::current_exe().expect("test executable"));
    config.arguments = vec![
        "--exact".to_owned(),
        "client::startup_tests::startup_helper_child_process".to_owned(),
        "--ignored".to_owned(),
    ];
    config
        .environment
        .insert(CHILD_ENV.to_owned(), "1".to_owned());
    config
}

pub(crate) async fn wait_until_cleanup_complete(client: &AppServerClient, expected_reason: &str) {
    loop {
        let child_cleared = client.inner.child.lock().await.is_none();
        let stdin_cleared = client.inner.stdin.lock().await.is_none();
        let closed = client.inner.closed.load(Ordering::Acquire);
        let reason = client.lifecycle_snapshot().closed_reason;
        if child_cleared && stdin_cleared && closed && reason.as_deref() == Some(expected_reason) {
            return;
        }
        tokio::task::yield_now().await;
    }
}

pub(crate) async fn wait_until_inner_released(inner: &std::sync::Weak<super::Inner>) {
    while inner.upgrade().is_some() {
        tokio::task::yield_now().await;
    }
}

pub(crate) async fn leak_snapshot(client: &AppServerClient) -> (bool, bool, bool) {
    let mut child = client.inner.child.lock().await;
    let child_running = child
        .as_mut()
        .is_some_and(|child| child.try_wait().is_ok_and(|status| status.is_none()));
    let child_present = child.is_some();
    drop(child);
    let stdin_present = client.inner.stdin.lock().await.is_some();
    (child_present, child_running, stdin_present)
}
