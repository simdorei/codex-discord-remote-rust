use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::time::timeout;

use super::{server_from_client, test_client};
use crate::client::WriteTestPause;
use crate::{
    AppServerError, DeadGenerationFence, DeadGenerationWork, RequestId, RpcErrorPayload,
    ServerRequest, ServerRequestOccurrence,
};

#[derive(Default)]
struct AdmissionFence {
    admitted: AtomicBool,
    begins: AtomicUsize,
    finishes: AtomicUsize,
}

impl DeadGenerationFence for AdmissionFence {
    fn persist(&self, _: &DeadGenerationWork) -> Result<(), AppServerError> {
        Ok(())
    }

    fn response_authority(
        &self,
        _: (&str, u64),
        request: &ServerRequest,
    ) -> Result<Option<Value>, AppServerError> {
        Ok(Some(request.params.clone()))
    }

    fn begin_response(
        &self,
        _: (&str, u64),
        request: &ServerRequest,
        authority: &Value,
        _: &Value,
    ) -> Result<(), AppServerError> {
        assert_eq!(authority, &request.params);
        assert_eq!(self.begins.fetch_add(1, Ordering::AcqRel), 0);
        self.admitted.store(true, Ordering::Release);
        Ok(())
    }

    fn finish_response(
        &self,
        _: (&str, u64),
        _: &ServerRequest,
        _: &Value,
        _: &Value,
        outcome: &str,
    ) -> Result<(), AppServerError> {
        assert_eq!(outcome, "flushed");
        self.finishes.fetch_add(1, Ordering::AcqRel);
        Err(AppServerError::MutationHeld {
            message: "injected response completion store failure".into(),
        })
    }
}

#[derive(Clone, Copy)]
enum Reply {
    Ordinary,
    Current,
    Error,
}

async fn exercise(reply: Reply, io_failure: bool) {
    let pause = Arc::new(if io_failure {
        WriteTestPause::failing()
    } else {
        WriteTestPause::new()
    });
    pause.release();
    let client = test_client(pause);
    let request = ServerRequest {
        id: RequestId::Integer(71),
        occurrence: ServerRequestOccurrence::from_bytes([0x71; 16]),
        method: "item/tool/requestUserInput".into(),
        params: json!({"threadId":"thread-a","turnId":"original-turn","itemId":"original-item"}),
    };
    {
        let mut state = client.inner.state.lock().unwrap();
        state.record_notification(crate::Notification {
            method: "turn/started".into(),
            params: json!({"threadId":"thread-a","turn":{"id":"original-turn"}}),
        });
        state.record_server_request(request.clone()).unwrap();
    }
    let fence = Arc::new(AdmissionFence::default());
    let mut server = server_from_client(client.clone());
    server.dead_generation_fence = Some(fence.clone());
    let result = timeout(Duration::from_secs(2), async {
        match reply {
            Reply::Ordinary => {
                server
                    .respond(&request.id, request.occurrence, json!({}), 1)
                    .await
            }
            Reply::Current => {
                server
                    .respond_current(&request.id, request.occurrence, json!({}), 1)
                    .await
            }
            Reply::Error => {
                server
                    .respond_error(
                        &request.id,
                        request.occurrence,
                        RpcErrorPayload {
                            code: -32_800,
                            message: "cancelled".into(),
                            data: None,
                        },
                        1,
                    )
                    .await
            }
        }
    })
    .await
    .unwrap();
    // Observe immediately, without a lifecycle monitor or a recovery task.
    let snapshot = server.lifecycle_snapshot().await;
    let closed = client.inner.closed.load(Ordering::Acquire);
    timeout(Duration::from_secs(3), server.close())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        result,
        Err(AppServerError::MutationOutcomeUnknown { .. })
    ));
    assert_eq!(fence.begins.load(Ordering::Acquire), 1);
    assert_eq!(
        fence.finishes.load(Ordering::Acquire),
        usize::from(!io_failure)
    );
    assert!(
        fence.admitted.load(Ordering::Acquire),
        "unknown admission must remain"
    );
    assert_eq!(
        closed, io_failure,
        "only an actual I/O failure closes the transport"
    );
    assert_eq!(
        snapshot.quarantined, io_failure,
        "raw I/O must reach the resident guard"
    );
    assert_eq!(
        snapshot.restart_pending, io_failure,
        "raw I/O must request safe restart"
    );
}

#[tokio::test]
async fn ordinary_response_io_failure_preserves_immediate_quarantine() {
    exercise(Reply::Ordinary, true).await;
}

#[tokio::test]
async fn current_response_io_failure_preserves_immediate_quarantine() {
    exercise(Reply::Current, true).await;
}

#[tokio::test]
async fn error_response_io_failure_preserves_immediate_quarantine() {
    exercise(Reply::Error, true).await;
}

#[tokio::test]
async fn ordinary_response_store_failure_keeps_transport_healthy() {
    exercise(Reply::Ordinary, false).await;
}

#[tokio::test]
async fn current_response_store_failure_keeps_transport_healthy() {
    exercise(Reply::Current, false).await;
}

#[tokio::test]
async fn error_response_store_failure_keeps_transport_healthy() {
    exercise(Reply::Error, false).await;
}
