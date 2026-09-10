use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::time::timeout;

use crate::AppServerError;
use crate::client::Inner;
use crate::process::{AppServerInput, AppServerProcess};
use crate::transport::mark_closed;

const GRACEFUL_CLOSE_TIMEOUT: Duration = Duration::from_millis(1_500);
const FORCED_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
const CLIENT_CLOSE_REASON: &str = "closed by client";

pub(super) async fn close(inner: &Arc<Inner>) -> Result<(), AppServerError> {
    close_using(
        inner,
        GRACEFUL_CLOSE_TIMEOUT,
        FORCED_CLOSE_TIMEOUT,
        |mut stdin| async move {
            stdin.shutdown().await?;
            Ok(())
        },
    )
    .await
}

async fn close_using<F, Fut>(
    inner: &Arc<Inner>,
    graceful_timeout: Duration,
    forced_timeout: Duration,
    shutdown: F,
) -> Result<(), AppServerError>
where
    F: FnOnce(AppServerInput) -> Fut,
    Fut: Future<Output = Result<(), AppServerError>>,
{
    inner.lifecycle.seal_for_close(CLIENT_CLOSE_REASON);
    inner.closed.store(true, Ordering::Release);

    let mut first_error = None;
    if let Some(stdin) = inner.stdin.lock().await.take()
        && let Err(error) = shutdown(stdin).await
    {
        first_error = Some(error);
    }

    let mut child_slot = inner.child.lock().await;
    if let Some(child) = child_slot.as_mut() {
        let reaped = reap_child(child, graceful_timeout, forced_timeout, &mut first_error).await;
        if reaped {
            child_slot.take();
        }
    }
    drop(child_slot);

    mark_closed(inner, CLIENT_CLOSE_REASON);
    first_error.map_or(Ok(()), Err)
}

async fn reap_child(
    child: &mut AppServerProcess,
    graceful_timeout: Duration,
    forced_timeout: Duration,
    first_error: &mut Option<AppServerError>,
) -> bool {
    match timeout(graceful_timeout, child.wait()).await {
        Ok(Ok(())) => return true,
        Ok(Err(error)) => record_first(first_error, error),
        Err(_) => {}
    }

    if let Err(error) = child.start_kill() {
        record_first(first_error, error);
    }

    match timeout(forced_timeout, child.wait()).await {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            record_first(first_error, error);
            false
        }
        Err(_) => {
            record_first(
                first_error,
                AppServerError::Timeout {
                    method: "process/exit".to_owned(),
                    timeout_ms: forced_timeout.as_millis(),
                },
            );
            false
        }
    }
}

fn record_first(first_error: &mut Option<AppServerError>, error: AppServerError) {
    if first_error.is_none() {
        *first_error = Some(error);
    }
}

#[cfg(test)]
pub(super) async fn close_with_shutdown<F, Fut>(
    inner: &Arc<Inner>,
    graceful_timeout: Duration,
    forced_timeout: Duration,
    shutdown: F,
) -> Result<(), AppServerError>
where
    F: FnOnce(AppServerInput) -> Fut,
    Fut: Future<Output = Result<(), AppServerError>>,
{
    close_using(inner, graceful_timeout, forced_timeout, shutdown).await
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::process::Stdio;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use tokio::process::Command;
    use tokio::sync::{Mutex as AsyncMutex, broadcast};

    use super::close_with_shutdown;
    use crate::AppServerError;
    use crate::client::{ClientLifecycle, Inner};
    use crate::diagnostics::BoundedDiagnostics;
    use crate::state::RuntimeState;

    const CHILD_ENV: &str = "CDR_CLOSE_HELPER_CHILD";

    #[test]
    #[ignore = "spawned by the close cleanup regression test"]
    fn close_helper_child_process() {
        if std::env::var_os(CHILD_ENV).is_some() {
            std::thread::sleep(Duration::from_secs(30));
        }
    }

    #[tokio::test]
    async fn shutdown_failure_still_kills_reaps_and_marks_closed() {
        let mut command = Command::new(std::env::current_exe().expect("current test executable"));
        command
            .args([
                "--exact",
                "control::close::tests::close_helper_child_process",
                "--ignored",
            ])
            .env(CHILD_ENV, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().expect("spawn close helper child");
        let process_id = child.id();
        let stdin = child.stdin.take().expect("child stdin");
        let (notifications, _) = broadcast::channel(1);
        let (server_requests, _) = broadcast::channel(1);
        let inner = Arc::new(Inner {
            child: AsyncMutex::new(Some(child.into())),
            closed: AtomicBool::new(false),
            diagnostics: Mutex::new(BoundedDiagnostics::default()),
            lifecycle: Arc::new(ClientLifecycle::new()),
            notifications,
            pending: Mutex::new(HashMap::new()),
            server_requests,
            state: Mutex::new(RuntimeState::starting(process_id)),
            stdin: AsyncMutex::new(Some(Box::pin(stdin))),
            write_pause: Mutex::new(None),
        });

        let error = close_with_shutdown(
            &inner,
            Duration::from_millis(25),
            Duration::from_secs(5),
            |_stdin| async {
                Err(AppServerError::Io(std::io::Error::other(
                    "injected stdin shutdown failure",
                )))
            },
        )
        .await
        .expect_err("injected shutdown failure must be returned");

        assert!(matches!(
            error,
            AppServerError::Io(ref source)
                if source.to_string() == "injected stdin shutdown failure"
        ));
        assert!(inner.child.lock().await.is_none());
        assert!(inner.stdin.lock().await.is_none());
        let snapshot = inner.state.lock().expect("runtime state lock").snapshot();
        assert!(!snapshot.healthy);
        assert_eq!(snapshot.closed_reason.as_deref(), Some("closed by client"));

        close_with_shutdown(
            &inner,
            Duration::from_millis(25),
            Duration::from_secs(5),
            |_stdin| async { Ok(()) },
        )
        .await
        .expect("idempotent cleanup retry");
    }
}
