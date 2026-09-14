use std::collections::HashMap;
use std::io::Read;
use std::process::Stdio;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::process::Command;
use tokio::sync::{Mutex as AsyncMutex, broadcast, watch};
use tokio::time::timeout;

use super::ResidentAppServer;
use super::admission::ResidentState;
use crate::client::{AppServerClient, ClientLifecycle, Inner, WriteTestPause};
use crate::diagnostics::BoundedDiagnostics;
use crate::state::RuntimeState;
use crate::{AppServerConfig, AppServerError};

const CHILD_ENV: &str = "CDR_WRITE_PAUSE_CHILD";

#[path = "write_cancel_tests/idle_gate.rs"]
mod idle_gate;
#[path = "write_cancel_tests/response.rs"]
mod response;
#[path = "write_cancel_tests/settings.rs"]
mod settings;

#[test]
#[ignore = "spawned by the partial-write cancellation regression"]
fn write_pause_child_process() {
    if std::env::var_os(CHILD_ENV).is_some() {
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
    }
}

fn test_client(pause: Arc<WriteTestPause>) -> AppServerClient {
    let mut command = Command::new(std::env::current_exe().expect("test executable"));
    command
        .args([
            "--exact",
            "manager::write_cancel_tests::write_pause_child_process",
            "--ignored",
        ])
        .env(CHILD_ENV, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().expect("spawn write fixture");
    let process_id = child.id();
    let stdin = child.stdin.take().expect("child stdin");
    let (notifications, _) = broadcast::channel(1);
    let (server_requests, _) = broadcast::channel(1);
    AppServerClient {
        inner: Arc::new(Inner {
            child: AsyncMutex::new(Some(child.into())),
            closed: AtomicBool::new(false),
            diagnostics: Mutex::new(BoundedDiagnostics::default()),
            lifecycle: Arc::new(ClientLifecycle::new()),
            notifications,
            pending: Mutex::new(HashMap::new()),
            server_requests,
            state: Mutex::new(RuntimeState::starting(process_id)),
            stdin: AsyncMutex::new(Some(Box::pin(stdin))),
            write_pause: Mutex::new(Some(pause)),
        }),
    }
}

fn test_server(pause: Arc<WriteTestPause>) -> ResidentAppServer {
    let client = test_client(pause);
    server_from_client(client)
}

fn server_from_client(client: AppServerClient) -> ResidentAppServer {
    let (notifications, _) = broadcast::channel(8);
    let (server_requests, _) = broadcast::channel(8);
    let (forwarder_generation, _) = watch::channel(1);
    ResidentAppServer {
        instance_id: uuid::Uuid::new_v4().to_string(),
        state: ResidentState::new(client),
        config: AppServerConfig::new(std::env::current_exe().expect("test executable")),
        restart_lock: AsyncMutex::new(()),
        notifications,
        server_requests,
        forwarder_generation,
        forwarders: Mutex::new(None),
        dead_generation_fence: None,
        target_gate: Arc::default(),
    }
}

#[tokio::test]
async fn cancellation_after_bytes_before_flush_quarantines_generation() {
    let pause = Arc::new(WriteTestPause::new());
    let server = Arc::new(test_server(Arc::clone(&pause)));
    let request = tokio::spawn({
        let server = Arc::clone(&server);
        async move {
            server
                .request(
                    "test/partialWrite",
                    json!({"sideEffect": true}),
                    Duration::from_secs(30),
                    Some(1),
                )
                .await
        }
    });
    timeout(Duration::from_secs(2), pause.wait_until_entered())
        .await
        .expect("write reached pre-flush pause");
    request.abort();
    assert!(request.await.expect_err("aborted request").is_cancelled());

    let snapshot = server.lifecycle_snapshot().await;
    let observed = (snapshot.quarantined, snapshot.restart_pending);
    timeout(Duration::from_secs(3), server.close())
        .await
        .expect("cleanup timeout")
        .expect("cleanup");

    assert_eq!(observed, (true, true));
}

#[tokio::test]
async fn io_error_after_bytes_quarantines_generation() {
    let server = test_server(Arc::new(WriteTestPause::failing()));

    let error = server
        .request(
            "test/writeError",
            json!({"sideEffect": true}),
            Duration::from_secs(2),
            Some(1),
        )
        .await
        .expect_err("injected write failure");
    let snapshot = server.lifecycle_snapshot().await;
    timeout(Duration::from_secs(3), server.close())
        .await
        .expect("cleanup timeout")
        .expect("cleanup");

    assert!(matches!(
        error,
        AppServerError::Io(ref source) if source.to_string() == "injected post-write failure"
    ));
    assert!(snapshot.quarantined);
    assert!(snapshot.restart_pending);
}

#[tokio::test]
async fn direct_client_abort_after_bytes_seals_transport() {
    let pause = Arc::new(WriteTestPause::new());
    let client = test_client(Arc::clone(&pause));
    let notification = tokio::spawn({
        let client = client.clone();
        async move { client.notify("test/partialNotify", json!({})).await }
    });
    timeout(Duration::from_secs(2), pause.wait_until_entered())
        .await
        .expect("write reached pre-flush pause");
    notification.abort();
    assert!(
        notification
            .await
            .expect_err("aborted notify")
            .is_cancelled()
    );

    let snapshot = client.lifecycle_snapshot();
    let retry = client
        .notify("test/retry", json!({}))
        .await
        .expect_err("sealed transport");
    timeout(Duration::from_secs(3), client.close())
        .await
        .expect("cleanup timeout")
        .expect("cleanup");

    assert!(!snapshot.healthy);
    assert_eq!(
        snapshot.closed_reason.as_deref(),
        Some("app-server write outcome indeterminate")
    );
    assert!(matches!(retry, AppServerError::Closed));
}

#[tokio::test]
async fn queued_writer_rechecks_closed_after_first_writer_aborts() {
    let pause = Arc::new(WriteTestPause::new());
    let client = test_client(Arc::clone(&pause));
    let first = tokio::spawn({
        let client = client.clone();
        async move { client.notify("test/first", json!({})).await }
    });
    timeout(Duration::from_secs(2), pause.wait_until_before_lock())
        .await
        .expect("first passed initial check");
    timeout(Duration::from_secs(2), pause.wait_until_entered())
        .await
        .expect("first reached pre-flush pause");
    let mut second = tokio::spawn({
        let client = client.clone();
        async move { client.notify("test/second", json!({})).await }
    });
    timeout(Duration::from_secs(2), pause.wait_until_before_lock())
        .await
        .expect("second passed initial check");

    first.abort();
    assert!(
        first
            .await
            .expect_err("aborted first writer")
            .is_cancelled()
    );
    let outcome = timeout(Duration::from_secs(2), async {
        tokio::select! {
            result = &mut second => Ok(result),
            () = pause.wait_until_entered() => Err("second write reached the pipe"),
        }
    })
    .await
    .expect("second writer resolution");
    if outcome.is_err() {
        second.abort();
        let _ = second.await;
    }
    timeout(Duration::from_secs(3), client.close())
        .await
        .expect("cleanup timeout")
        .expect("cleanup");

    let result = outcome
        .expect("second writer must not write")
        .expect("second task");
    assert!(matches!(result, Err(AppServerError::Closed)));
}
