use std::cell::{Cell, RefCell};

use cdr_app_server::{AppServerError, RequestId, ServerRequest, ServerRequestOccurrence};
use serde_json::{Value, json};

use super::{PendingTextReplyServer, handle_pending_text_reply_with};
use crate::component_worker::ComponentWorkerError;

struct FakeServer {
    generation: Cell<u64>,
    requests: Vec<ServerRequest>,
    live_occurrence: Option<ServerRequestOccurrence>,
    reconnect_after_fetch: bool,
    reconnect_on_respond: bool,
    accepted: RefCell<Vec<(RequestId, ServerRequestOccurrence, Value, u64)>>,
}

impl PendingTextReplyServer for FakeServer {
    async fn authorize(&self, _: &ServerRequest, _: u64) -> Result<(), ComponentWorkerError> {
        Ok(()) // Parser/occurrence fixture; actor authority has real-resident integration tests.
    }

    fn generation(&self) -> u64 {
        self.generation.get()
    }

    async fn pending_server_requests(
        &self,
        _thread_id: Option<&str>,
    ) -> Result<Vec<ServerRequest>, AppServerError> {
        let snapshot = self.requests.clone();
        if self.reconnect_after_fetch {
            self.generation.set(2);
        }
        Ok(snapshot)
    }

    async fn respond(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
        result: Value,
        expected_generation: u64,
    ) -> Result<(), AppServerError> {
        if self.reconnect_on_respond {
            self.generation.set(2);
        }
        let actual = self.generation.get();
        if expected_generation != actual {
            return Err(AppServerError::GenerationMismatch {
                expected: expected_generation,
                actual,
            });
        }
        if self.live_occurrence.is_some_and(|live| occurrence != live) {
            return Err(AppServerError::StaleServerRequest { id: id.clone() });
        }
        self.accepted
            .borrow_mut()
            .push((id.clone(), occurrence, result, expected_generation));
        Ok(())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn txt_amb_01_two_approval_requests_are_ambiguous_without_submission() {
    let server = stable_server(vec![
        approval(7, "first", occurrence(0x11)),
        approval(8, "second", occurrence(0x22)),
    ]);
    let result = handle_pending_text_reply_with("thread-a", "1", &server).await;
    assert_ambiguous(&result, &server);
}

#[tokio::test(flavor = "current_thread")]
async fn txt_amb_02_mixed_approval_and_input_are_ambiguous_without_submission() {
    let server = stable_server(vec![
        approval(7, "approval", occurrence(0x11)),
        input(8, occurrence(0x22)),
    ]);
    let result = handle_pending_text_reply_with("thread-a", "1", &server).await;
    assert_ambiguous(&result, &server);
}

#[tokio::test(flavor = "current_thread")]
async fn txt_amb_03_one_eligible_request_uses_its_exact_identity_once() {
    let expected_occurrence = occurrence(0x33);
    let server = stable_server(vec![
        irrelevant(7, occurrence(0x11)),
        input(8, expected_occurrence),
        irrelevant(9, occurrence(0x22)),
    ]);
    let confirmation = handle_pending_text_reply_with("thread-a", "2", &server)
        .await
        .unwrap();
    assert_eq!(
        confirmation.as_deref(),
        Some("Codex input reply submitted.")
    );
    let accepted = server.accepted.borrow();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].0, RequestId::Integer(8));
    assert_eq!(accepted[0].1, expected_occurrence);
    assert_eq!(
        accepted[0].2,
        json!({"answers":{"mode":{"answers":["Safe"]}}})
    );
    assert_eq!(accepted[0].3, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn txt_amb_04_zero_eligible_requests_returns_none_without_submission() {
    let server = stable_server(vec![irrelevant(7, occurrence(0x11))]);
    let result = handle_pending_text_reply_with("thread-a", "ordinary text", &server)
        .await
        .unwrap();
    assert_eq!(result, None);
    assert!(server.accepted.borrow().is_empty());
}

// TXT-AMB-05: preserve generation and occurrence race protections.
#[tokio::test(flavor = "current_thread")]
async fn reconnect_during_fetch_cannot_submit_old_payload_to_reused_request_id() {
    let server = reconnect_server(true, false, occurrence(0x11));
    let result = handle_pending_text_reply_with("thread-a", "1", &server).await;
    assert!(matches!(
        result,
        Err(ComponentWorkerError::AppServer(
            AppServerError::GenerationMismatch {
                expected: 1,
                actual: 2
            }
        ))
    ));
    assert!(server.accepted.borrow().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn reconnect_before_respond_uses_the_captured_generation() {
    let server = reconnect_server(false, true, occurrence(0x11));
    let result = handle_pending_text_reply_with("thread-a", "1", &server).await;
    assert!(matches!(
        result,
        Err(ComponentWorkerError::AppServer(
            AppServerError::GenerationMismatch {
                expected: 1,
                actual: 2
            }
        ))
    ));
    assert!(server.accepted.borrow().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn stable_generation_submits_once_with_the_captured_generation() {
    let server = reconnect_server(false, false, occurrence(0x11));
    let confirmation = handle_pending_text_reply_with("thread-a", "1", &server)
        .await
        .unwrap();
    assert_eq!(
        confirmation.as_deref(),
        Some("Approval response submitted: accept")
    );
    let accepted = server.accepted.borrow();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].0, RequestId::Integer(7));
    assert_eq!(accepted[0].1, occurrence(0x11));
    assert_eq!(accepted[0].3, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn same_generation_reused_id_cannot_accept_a_stale_occurrence() {
    let server = reconnect_server(false, false, occurrence(0x22));
    let result = handle_pending_text_reply_with("thread-a", "1", &server).await;
    assert!(matches!(
        result,
        Err(ComponentWorkerError::AppServer(
            AppServerError::StaleServerRequest {
                id: RequestId::Integer(7)
            }
        ))
    ));
    assert!(server.accepted.borrow().is_empty());
}

fn assert_ambiguous(result: &Result<Option<String>, ComponentWorkerError>, server: &FakeServer) {
    assert!(
        matches!(result, Err(ComponentWorkerError::AmbiguousPendingRequest)),
        "expected AmbiguousPendingRequest, got {result:?}; accepted={:?}",
        server.accepted.borrow()
    );
    assert!(server.accepted.borrow().is_empty());
}

fn stable_server(requests: Vec<ServerRequest>) -> FakeServer {
    FakeServer {
        generation: Cell::new(1),
        live_occurrence: None,
        requests,
        reconnect_after_fetch: false,
        reconnect_on_respond: false,
        accepted: RefCell::new(Vec::new()),
    }
}

fn reconnect_server(
    reconnect_after_fetch: bool,
    reconnect_on_respond: bool,
    replacement_occurrence: ServerRequestOccurrence,
) -> FakeServer {
    let request = approval(7, "old request", occurrence(0x11));
    FakeServer {
        generation: Cell::new(1),
        requests: vec![request],
        live_occurrence: Some(replacement_occurrence),
        reconnect_after_fetch,
        reconnect_on_respond,
        accepted: RefCell::new(Vec::new()),
    }
}

fn approval(id: i64, reason: &str, occurrence: ServerRequestOccurrence) -> ServerRequest {
    ServerRequest {
        id: RequestId::Integer(id),
        occurrence,
        method: "item/commandExecution/requestApproval".into(),
        params: json!({"threadId":"thread-a","reason":reason}),
    }
}

fn input(id: i64, occurrence: ServerRequestOccurrence) -> ServerRequest {
    ServerRequest {
        id: RequestId::Integer(id),
        occurrence,
        method: "item/tool/requestUserInput".into(),
        params: json!({"threadId":"thread-a","questions":[{
            "id":"mode","options":[{"label":"Fast"},{"label":"Safe"}]
        }]}),
    }
}

fn irrelevant(id: i64, occurrence: ServerRequestOccurrence) -> ServerRequest {
    ServerRequest {
        id: RequestId::Integer(id),
        occurrence,
        method: "item/tool/requestUserInput/preview".into(),
        params: json!({"threadId":"thread-a"}),
    }
}

const fn occurrence(byte: u8) -> ServerRequestOccurrence {
    ServerRequestOccurrence::from_bytes([byte; 16])
}
