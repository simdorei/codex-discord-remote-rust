use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use serde_json::json;
use tokio::sync::{Mutex as AsyncMutex, broadcast};

use super::*;
use crate::client::{ClientLifecycle, Inner};
use crate::diagnostics::BoundedDiagnostics;
use crate::state::RuntimeState;
use crate::state::ServerRequestRecordOutcome;

fn request(id: i64, token: u128) -> ServerRequest {
    ServerRequest {
        id: RequestId::Integer(id),
        occurrence: ServerRequestOccurrence::from_bytes(token.to_be_bytes()),
        method: "item/commandExecution/requestApproval".to_owned(),
        params: json!({"threadId": "thread-a", "command": format!("command-{id}")}),
    }
}

fn unwritable_client(requests: impl IntoIterator<Item = ServerRequest>) -> AppServerClient {
    let mut state = RuntimeState::starting(None);
    for request in requests {
        state
            .record_server_request(request)
            .expect("record request");
    }
    let (notifications, _) = broadcast::channel(8);
    let (server_requests, _) = broadcast::channel(8);
    AppServerClient {
        inner: Arc::new(Inner {
            child: AsyncMutex::new(None),
            closed: AtomicBool::new(false),
            diagnostics: Mutex::new(BoundedDiagnostics::default()),
            lifecycle: Arc::new(ClientLifecycle::new()),
            notifications,
            pending: Mutex::new(HashMap::new()),
            server_requests,
            state: Mutex::new(state),
            stdin: AsyncMutex::new(None),
            write_pause: Mutex::new(None),
        }),
    }
}

#[tokio::test]
async fn write_failure_consumes_claim_and_blocks_duplicate_wire_attempt() {
    let request = request(1, 1);
    let client = unwritable_client([request.clone()]);
    let first = client
        .respond(&request.id, request.occurrence, json!({}))
        .await
        .expect_err("missing writer");
    assert!(matches!(first, AppServerError::Closed));
    assert!(client.pending_server_requests(None).is_empty());
    assert!(client.has_unsettled_server_requests());

    let retry = client
        .respond(&request.id, request.occurrence, json!({}))
        .await
        .expect_err("ambiguous write must not be retried");
    assert!(matches!(
        retry,
        AppServerError::ServerRequestResponseIndeterminate { .. }
    ));
    assert!(matches!(
        client
            .respond_current(&request.id, request.occurrence, json!({}))
            .await,
        Err(AppServerError::ServerRequestResponseIndeterminate { .. })
    ));
}

#[tokio::test]
async fn cancellation_attempts_later_occurrences_and_returns_first_exact_error() {
    let first = request(1, 1);
    let second = request(2, 2);
    let client = unwritable_client([first.clone(), second.clone()]);
    let write_error = client
        .respond(&first.id, first.occurrence, json!({}))
        .await
        .expect_err("make first indeterminate");
    assert!(matches!(write_error, AppServerError::Closed));
    assert_eq!(client.pending_server_requests(None), vec![second.clone()]);

    let cancellation = client
        .cancel_pending_server_requests("thread-a")
        .await
        .expect_err("preserve first indeterminate error");
    let AppServerError::ServerRequestResponseIndeterminate { id } = cancellation else {
        panic!("unexpected cancellation error");
    };
    assert_eq!(id, first.id);
    assert!(client.pending_server_requests(None).is_empty());
    let second_status = client
        .respond_error(
            &second.id,
            second.occurrence,
            RpcErrorPayload {
                code: -32_800,
                message: "again".to_owned(),
                data: None,
            },
        )
        .await
        .expect_err("later request was attempted and became indeterminate");
    assert!(matches!(
        second_status,
        AppServerError::ServerRequestResponseIndeterminate { .. }
    ));
}

#[tokio::test]
async fn exact_success_promotes_and_broadcasts_deferred_candidate_once() {
    let old = request(1, 1);
    let candidate = ServerRequest {
        occurrence: ServerRequestOccurrence::from_bytes(2_u128.to_be_bytes()),
        params: json!({"threadId": "thread-a", "command": "new"}),
        ..old.clone()
    };
    let client = unwritable_client([old.clone()]);
    let mut events = client.subscribe_server_requests();
    let claim = ServerResponseClaim::begin(&client, &old.id, old.occurrence).expect("claim old");
    let outcome = client
        .inner
        .state
        .lock()
        .expect("runtime state lock")
        .record_server_request(candidate.clone())
        .expect("defer candidate");
    assert!(matches!(outcome, ServerRequestRecordOutcome::Deferred));
    assert!(client.pending_server_requests(None).is_empty());

    claim.resolve().expect("resolve and promote");
    assert_eq!(
        events.recv().await.expect("promoted event"),
        candidate.clone()
    );
    assert!(matches!(
        events.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    assert_eq!(client.pending_server_requests(None), vec![candidate]);
}

#[tokio::test]
async fn exact_success_does_not_promote_or_broadcast_identical_claimed_redelivery() {
    let original = request(1, 1);
    let redelivery = ServerRequest {
        occurrence: ServerRequestOccurrence::from_bytes(2_u128.to_be_bytes()),
        ..original.clone()
    };
    let client = unwritable_client([original.clone()]);
    let mut events = client.subscribe_server_requests();
    let claim =
        ServerResponseClaim::begin(&client, &original.id, original.occurrence).expect("claim");
    let outcome = client
        .inner
        .state
        .lock()
        .expect("runtime state lock")
        .record_server_request(redelivery)
        .expect("dedupe redelivery");
    assert!(matches!(outcome, ServerRequestRecordOutcome::Duplicate));

    claim.resolve().expect("resolve without promotion");
    assert!(matches!(
        events.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    assert!(client.pending_server_requests(None).is_empty());
    assert!(!client.has_unsettled_server_requests());
}

#[test]
fn failed_claim_keeps_deferred_candidate_non_actionable_and_unbroadcast() {
    let old = request(1, 1);
    let candidate = ServerRequest {
        occurrence: ServerRequestOccurrence::from_bytes(2_u128.to_be_bytes()),
        params: json!({"threadId": "thread-a", "command": "new"}),
        ..old.clone()
    };
    let client = unwritable_client([old.clone()]);
    let mut events = client.subscribe_server_requests();
    let claim = ServerResponseClaim::begin(&client, &old.id, old.occurrence).expect("claim old");
    let outcome = client
        .inner
        .state
        .lock()
        .expect("runtime state lock")
        .record_server_request(candidate)
        .expect("defer candidate");
    assert!(matches!(outcome, ServerRequestRecordOutcome::Deferred));
    drop(claim);

    assert!(client.pending_server_requests(None).is_empty());
    assert!(client.has_unsettled_server_requests());
    assert!(matches!(
        events.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn turn_finished_while_response_waits_for_writer_preserves_unsent_request() {
    let mut original = request(1, 1);
    original.params["turnId"] = json!("first");
    let client = unwritable_client([original.clone()]);
    client
        .inner
        .state
        .lock()
        .unwrap()
        .record_notification(crate::Notification {
            method: "turn/started".into(),
            params: json!({"threadId":"thread-a","turn":{"id":"first"}}),
        });
    let pause = Arc::new(crate::client::WriteTestPause::new());
    *client.inner.write_pause.lock().unwrap() = Some(pause.clone());
    let held = client.inner.stdin.lock().await;
    let running = tokio::spawn({
        let (client, original) = (client.clone(), original.clone());
        async move {
            client
                .respond_current(&original.id, original.occurrence, json!({}))
                .await
        }
    });
    pause.wait_until_before_lock().await;
    client
        .inner
        .state
        .lock()
        .unwrap()
        .record_notification(crate::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread-a","turn":{"id":"first"}}),
        });
    drop(held);
    assert!(matches!(
        running.await.unwrap(),
        Err(AppServerError::StaleServerRequest { .. })
    ));
    assert_eq!(client.pending_server_requests(None), vec![original]);
}
