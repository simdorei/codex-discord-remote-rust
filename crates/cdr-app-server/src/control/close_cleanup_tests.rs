use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::process::Command;
use tokio::sync::{Mutex as AsyncMutex, broadcast};

use crate::client::{AppServerClient, ClientLifecycle, Inner};
use crate::diagnostics::BoundedDiagnostics;
use crate::state::RuntimeState;

const CHILD_FIXTURE_ENV: &str = "CDR_CLOSE_CHILD_FIXTURE";

#[test]
#[ignore = "spawned only by close cleanup regression"]
fn close_child_fixture() {
    if std::env::var_os(CHILD_FIXTURE_ENV).is_some() {
        std::thread::sleep(Duration::from_secs(30));
    }
}

#[tokio::test]
async fn close_retries_cleanup_when_client_was_already_marked_closed() {
    let mut command = Command::new(std::env::current_exe().expect("test executable"));
    command
        .args([
            "--exact",
            "control::close_cleanup_tests::close_child_fixture",
            "--ignored",
        ])
        .env(CHILD_FIXTURE_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let child = command.spawn().expect("spawn fixture");
    let process_id = child.id();
    let (notifications, _) = broadcast::channel(8);
    let (server_requests, _) = broadcast::channel(8);
    let client = AppServerClient {
        inner: Arc::new(Inner {
            child: AsyncMutex::new(Some(child.into())),
            closed: AtomicBool::new(true),
            diagnostics: Mutex::new(BoundedDiagnostics::default()),
            lifecycle: Arc::new(ClientLifecycle::new()),
            notifications,
            pending: Mutex::new(HashMap::new()),
            server_requests,
            state: Mutex::new(RuntimeState::starting(process_id)),
            stdin: AsyncMutex::new(None),
            write_pause: Mutex::new(None),
        }),
    };

    client.close().await.expect("retry close");
    assert!(client.inner.child.lock().await.is_none());
    assert!(client.lifecycle_snapshot().process_id.is_none());
}
