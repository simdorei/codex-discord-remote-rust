use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::time::timeout;

use super::ResidentAppServer;
use crate::client::{AppServerClient, WriteTestPause, startup_tests::helper_config};
use crate::{
    AppServerError, DeadGenerationFence, DeadGenerationWork, RequestId, ServerRequest,
    ServerRequestOccurrence,
};

#[path = "dead_generation_fence_tests/restart.rs"]
mod restart;

#[derive(Default)]
struct RecordingFence(Mutex<Vec<DeadGenerationWork>>);

impl DeadGenerationFence for RecordingFence {
    fn persist(&self, snapshot: &DeadGenerationWork) -> Result<(), AppServerError> {
        self.0.lock().unwrap().push(snapshot.clone());
        Ok(())
    }
}

fn request(client: &AppServerClient) -> ServerRequest {
    let request = ServerRequest {
        id: RequestId::Integer(7),
        occurrence: ServerRequestOccurrence::from_bytes(7_u128.to_be_bytes()),
        method: "item/commandExecution/requestApproval".into(),
        params: json!({"threadId":"thread-a"}),
    };
    client
        .inner
        .state
        .lock()
        .unwrap()
        .record_server_request(request.clone())
        .unwrap();
    request
}

async fn exit_child(client: &AppServerClient) {
    {
        let mut child = client.inner.child.lock().await;
        let child = child.as_mut().unwrap();
        child.start_kill().unwrap();
        timeout(Duration::from_secs(3), child.wait())
            .await
            .unwrap()
            .unwrap();
    }
    crate::transport::mark_closed(&client.inner, "test process exited");
}

#[tokio::test]
async fn capture_waits_until_already_written_response_finishes_resolving() {
    let fence = Arc::new(RecordingFence::default());
    let server = Arc::new(
        ResidentAppServer::start_with_dead_generation_fence(helper_config(), fence.clone())
            .await
            .unwrap(),
    );
    let client = server.state.current_client().unwrap();
    let request = request(&client);
    let pause = Arc::new(WriteTestPause::for_response_resolution());
    *client.inner.write_pause.lock().unwrap() = Some(pause.clone());
    let response = tokio::spawn({
        let server = server.clone();
        async move {
            server
                .respond(&request.id, request.occurrence, json!({}), 1)
                .await
        }
    });
    timeout(Duration::from_secs(3), pause.wait_until_entered())
        .await
        .unwrap();
    exit_child(&client).await;

    let first = server.force_restart_if_quiescent().await;
    let captures_before_resolution = fence.0.lock().unwrap().len();
    pause.release();
    let resolved = timeout(Duration::from_secs(3), response)
        .await
        .unwrap()
        .unwrap();
    let retried = server.force_restart_if_quiescent().await;
    let generation = server.generation();
    server.close().await.unwrap();

    assert_eq!(
        captures_before_resolution, 0,
        "do not freeze a snapshot while an admitted response can still remove its occurrence"
    );
    assert!(!first.unwrap());
    assert!(
        resolved.is_ok(),
        "the already written response must finish exact local resolution"
    );
    assert!(retried.unwrap());
    assert_eq!(generation, 2);
    let snapshots = fence.0.lock().unwrap();
    assert_eq!(snapshots.len(), 1);
    assert!(snapshots[0].server_requests.is_empty());
}

#[tokio::test]
async fn sealed_transport_does_not_prove_child_death_or_authorize_settling_live_work() {
    let fence = Arc::new(RecordingFence::default());
    let server = Arc::new(
        ResidentAppServer::start_with_dead_generation_fence(helper_config(), fence.clone())
            .await
            .unwrap(),
    );
    let client = server.state.current_client().unwrap();
    let request = request(&client);
    let pause = Arc::new(WriteTestPause::new());
    *client.inner.write_pause.lock().unwrap() = Some(pause.clone());
    let response = tokio::spawn({
        let server = server.clone();
        async move {
            server
                .respond(&request.id, request.occurrence, json!({}), 1)
                .await
        }
    });
    timeout(Duration::from_secs(3), pause.wait_until_entered())
        .await
        .unwrap();
    response.abort();
    assert!(response.await.unwrap_err().is_cancelled());
    assert!(client.lifecycle_snapshot().closed_reason.is_some());
    assert!(
        client
            .inner
            .child
            .lock()
            .await
            .as_mut()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_none()
    );

    let first = server.force_restart_if_quiescent().await;
    let first_generation = server.generation();
    let captures_while_alive = fence.0.lock().unwrap().len();
    let alive = match client.inner.child.lock().await.as_mut() {
        Some(child) => child.try_wait().unwrap().is_none(),
        None => false,
    };
    if alive {
        exit_child(&client).await;
    }
    let after_exit = server.force_restart_if_quiescent().await;
    let final_generation = server.generation();
    server.close().await.unwrap();

    assert_eq!(
        captures_while_alive, 0,
        "transport cancellation is not evidence of process death"
    );
    assert!(
        alive,
        "the unresolved live child must not be killed by dead-generation recovery"
    );
    assert!(!first.unwrap());
    assert_eq!(first_generation, 1);
    assert!(after_exit.unwrap());
    assert_eq!(final_generation, 2);
    assert_eq!(fence.0.lock().unwrap()[0].server_requests.len(), 1);
}

#[tokio::test]
async fn live_idle_timeout_quarantine_keeps_normal_quiescent_restart() {
    let fence = Arc::new(RecordingFence::default());
    let server =
        ResidentAppServer::start_with_dead_generation_fence(helper_config(), fence.clone())
            .await
            .unwrap();
    let client = server.state.current_client().unwrap();
    server.state.mark_timeout(1);
    assert!(client.lifecycle_snapshot().closed_reason.is_none());
    assert!(
        client
            .inner
            .child
            .lock()
            .await
            .as_mut()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_none()
    );
    assert!(server.force_restart_if_quiescent().await.unwrap());
    assert_eq!(server.generation(), 2);
    assert!(fence.0.lock().unwrap().is_empty());
    server.close().await.unwrap();
}
