use std::collections::HashMap;
use std::future::Future;
use std::io::Read;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::process::Command;
use tokio::sync::{Mutex as AsyncMutex, Semaphore, broadcast, watch};
use tokio::time::timeout;

use super::ResidentAppServer;
use super::admission::ResidentState;
use crate::client::{AppServerClient, ClientLifecycle, Inner};
use crate::diagnostics::BoundedDiagnostics;
use crate::state::RuntimeState;
use crate::{AppServerConfig, AppServerError};

const CHILD_ENV: &str = "CDR_RESIDENT_CLOSE_CHILD";

#[test]
#[ignore = "spawned by resident close retry regressions"]
fn resident_close_child_process() {
    if std::env::var_os(CHILD_ENV).is_some() {
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
    }
}

pub(super) fn test_server() -> (ResidentAppServer, AppServerClient) {
    let mut command = Command::new(std::env::current_exe().expect("test executable"));
    command
        .args([
            "--exact",
            "manager::close_retry_tests::resident_close_child_process",
            "--ignored",
        ])
        .env(CHILD_ENV, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().expect("spawn close fixture");
    let process_id = child.id();
    let stdin = child.stdin.take().expect("child stdin");
    let (client_notifications, _) = broadcast::channel(1);
    let (client_requests, _) = broadcast::channel(1);
    let client = AppServerClient {
        inner: Arc::new(Inner {
            child: AsyncMutex::new(Some(child.into())),
            closed: AtomicBool::new(false),
            diagnostics: Mutex::new(BoundedDiagnostics::default()),
            lifecycle: Arc::new(ClientLifecycle::new()),
            notifications: client_notifications,
            pending: Mutex::new(HashMap::new()),
            server_requests: client_requests,
            state: Mutex::new(RuntimeState::starting(process_id)),
            stdin: AsyncMutex::new(Some(Box::pin(stdin))),
            write_pause: Mutex::new(None),
        }),
    };
    let (notifications, _) = broadcast::channel(1);
    let (server_requests, _) = broadcast::channel(1);
    let (forwarder_generation, _) = watch::channel(1);
    let server = ResidentAppServer {
        instance_id: uuid::Uuid::new_v4().to_string(),
        state: ResidentState::new(client.clone()),
        config: AppServerConfig::new(std::env::current_exe().expect("test executable")),
        restart_lock: AsyncMutex::new(()),
        notifications,
        server_requests,
        forwarder_generation,
        forwarders: Mutex::new(None),
        dead_generation_fence: None,
        target_gate: Arc::default(),
    };
    (server, client)
}

impl ResidentAppServer {
    async fn close_with_test<F, Fut>(&self, cleanup: F) -> Result<(), AppServerError>
    where
        F: FnMut(AppServerClient) -> Fut,
        Fut: Future<Output = Result<(), AppServerError>>,
    {
        super::close::close_with(self, cleanup).await
    }
}

async fn restart_result(server: &ResidentAppServer) -> Result<bool, AppServerError> {
    timeout(Duration::from_secs(2), server.force_restart_if_quiescent())
        .await
        .expect("restart resolution")
}

#[tokio::test]
async fn cleanup_error_retains_same_client_for_resident_close_retry() {
    let (server, original) = test_server();
    let process_id = original.lifecycle_snapshot().process_id;
    let calls = Arc::new(AtomicUsize::new(0));
    let first_calls = Arc::clone(&calls);
    let error = timeout(
        Duration::from_secs(2),
        server.close_with_test(move |_client| {
            first_calls.fetch_add(1, Ordering::SeqCst);
            async {
                Err(AppServerError::Io(std::io::Error::other(
                    "injected cleanup error",
                )))
            }
        }),
    )
    .await
    .expect("first close resolution")
    .expect_err("injected cleanup error");

    let state = server.state.snapshot();
    let retained = state.client.as_ref().is_some_and(|client| {
        Arc::ptr_eq(&client.inner, &original.inner)
            && client.lifecycle_snapshot().process_id == process_id
    });
    let current_closed = matches!(server.state.current_client(), Err(AppServerError::Closed));
    let restart = restart_result(&server).await;
    let generation = server.generation();
    let retry_calls = Arc::clone(&calls);
    timeout(
        Duration::from_secs(3),
        server.close_with_test(move |client| {
            let retry_calls = Arc::clone(&retry_calls);
            async move {
                retry_calls.fetch_add(1, Ordering::SeqCst);
                client.close().await
            }
        }),
    )
    .await
    .expect("retry close timeout")
    .expect("retry close");
    let child_reaped = original.inner.child.lock().await.is_none();
    if !child_reaped {
        let _ = timeout(Duration::from_secs(3), original.close()).await;
    }

    assert!(matches!(error, AppServerError::Io(_)));
    assert!(!state.accepting);
    assert!(current_closed);
    assert!(retained);
    assert!(matches!(restart, Ok(false)));
    assert_eq!(generation, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(child_reaped);
}

#[tokio::test]
async fn cancelled_cleanup_retains_same_client_for_resident_close_retry() {
    let (server, original) = test_server();
    let server = Arc::new(server);
    let process_id = original.lifecycle_snapshot().process_id;
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(Semaphore::new(0));
    let close = tokio::spawn({
        let server = Arc::clone(&server);
        let calls = Arc::clone(&calls);
        let entered = Arc::clone(&entered);
        async move {
            server
                .close_with_test(move |client| {
                    let calls = Arc::clone(&calls);
                    let entered = Arc::clone(&entered);
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        entered.add_permits(1);
                        std::future::pending::<()>().await;
                        client.close().await
                    }
                })
                .await
        }
    });
    timeout(Duration::from_secs(2), entered.acquire())
        .await
        .expect("cleanup entered")
        .expect("cleanup barrier open")
        .forget();
    close.abort();
    assert!(close.await.expect_err("cancelled close").is_cancelled());

    let state = server.state.snapshot();
    let retained = state.client.as_ref().is_some_and(|client| {
        Arc::ptr_eq(&client.inner, &original.inner)
            && client.lifecycle_snapshot().process_id == process_id
    });
    let current_closed = matches!(server.state.current_client(), Err(AppServerError::Closed));
    let restart = restart_result(&server).await;
    let generation = server.generation();
    let retry_calls = Arc::clone(&calls);
    timeout(
        Duration::from_secs(3),
        server.close_with_test(move |client| {
            let retry_calls = Arc::clone(&retry_calls);
            async move {
                retry_calls.fetch_add(1, Ordering::SeqCst);
                client.close().await
            }
        }),
    )
    .await
    .expect("retry close timeout")
    .expect("retry close");
    let child_reaped = original.inner.child.lock().await.is_none();
    if !child_reaped {
        let _ = timeout(Duration::from_secs(3), original.close()).await;
    }

    assert!(!state.accepting);
    assert!(current_closed);
    assert!(retained);
    assert!(matches!(restart, Ok(false)));
    assert_eq!(generation, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(child_reaped);
}
