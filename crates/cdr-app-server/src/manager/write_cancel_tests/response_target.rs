use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
struct TargetFence {
    stopped: AtomicBool,
    checks: Mutex<Vec<Value>>,
}

impl DeadGenerationFence for TargetFence {
    fn persist(&self, _: &DeadGenerationWork) -> Result<(), AppServerError> {
        Ok(())
    }

    fn check_request(&self, _: u64, method: &str, params: &Value) -> Result<(), AppServerError> {
        if method == "server/response" {
            self.checks.lock().unwrap().push(params.clone());
            if self.stopped.load(Ordering::Acquire) && params["threadId"] == "thread-a" {
                return Err(AppServerError::MutationHeld {
                    message: "original target stopped".into(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Reply {
    Ordinary,
    Current,
    Error,
}

fn record_original(client: &crate::AppServerClient, request: &ServerRequest) {
    let mut state = client.inner.state.lock().unwrap();
    state.record_notification(crate::Notification {
        method: "turn/started".into(),
        params: json!({"threadId":request.params["threadId"],"turn":{"id":"original-turn"}}),
    });
    state.record_server_request(request.clone()).unwrap();
}

async fn exercise(reply: Reply, stopped_target: bool) {
    let pause = Arc::new(WriteTestPause::new());
    let client = test_client(Arc::clone(&pause));
    let thread = if stopped_target {
        "thread-a"
    } else {
        "thread-b"
    };
    let request = ServerRequest {
        id: RequestId::Integer(71),
        occurrence: ServerRequestOccurrence::from_bytes([0x71; 16]),
        method: "item/tool/requestUserInput".into(),
        params: json!({"threadId":thread,"turnId":"original-turn","itemId":"original-item"}),
    };
    record_original(&client, &request);
    let fence = Arc::new(TargetFence::default());
    let mut resident = server_from_client(client.clone());
    resident.dead_generation_fence = Some(fence.clone());
    let server = Arc::new(resident);
    let held_writer = client.inner.stdin.lock().await;
    let mut response = tokio::spawn({
        let (server, request) = (Arc::clone(&server), request.clone());
        async move {
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
        }
    });
    timeout(Duration::from_secs(2), pause.wait_until_before_lock())
        .await
        .unwrap();
    // Admission passed. Stop wins while this exact response is waiting for stdin.
    fence.stopped.store(true, Ordering::Release);
    drop(held_writer);
    let first = timeout(Duration::from_secs(2), async {
        tokio::select! {
            result = &mut response => Some(result),
            () = pause.wait_until_entered() => None,
        }
    })
    .await
    .unwrap();
    let wrote = first.is_none();
    let result = if let Some(result) = first {
        result.unwrap()
    } else {
        pause.release();
        timeout(Duration::from_secs(2), response)
            .await
            .unwrap()
            .unwrap()
    };
    let quarantined = server.lifecycle_snapshot().await.quarantined;
    timeout(Duration::from_secs(3), server.close())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        wrote, !stopped_target,
        "stop before final writer must prevent A bytes, not B bytes"
    );
    if stopped_target {
        assert!(matches!(result, Err(AppServerError::MutationHeld { .. })));
    } else {
        result.unwrap();
    }
    assert!(
        !quarantined,
        "a definite target rejection must not quarantine the resident"
    );
    let checks = fence.checks.lock().unwrap();
    assert_eq!(checks.len(), 2);
    assert!(
        checks.iter().all(|params| params == &request.params),
        "both boundaries need the immutable original request params"
    );
}

#[tokio::test]
async fn ordinary_response_rechecks_original_target_after_writer_wait() {
    exercise(Reply::Ordinary, true).await;
    exercise(Reply::Ordinary, false).await;
}

#[tokio::test]
async fn current_response_rechecks_original_target_after_writer_wait() {
    exercise(Reply::Current, true).await;
    exercise(Reply::Current, false).await;
}

#[tokio::test]
async fn error_response_rechecks_original_target_after_writer_wait() {
    exercise(Reply::Error, true).await;
    exercise(Reply::Error, false).await;
}
