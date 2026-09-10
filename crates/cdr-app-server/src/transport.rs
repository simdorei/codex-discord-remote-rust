use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::client::Inner;
use crate::process::AppServerOutput;
use crate::rpc::{IncomingMessage, classify};
use crate::state::{ServerRequestRecordError, ServerRequestRecordOutcome};

#[cfg(test)]
#[path = "transport/tests.rs"]
mod tests;

pub(crate) async fn drain_stdout(inner: Arc<Inner>, stdout: AppServerOutput) {
    let mut lines = BufReader::new(stdout).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => handle_stdout_line(&inner, &line),
            Ok(None) => {
                mark_closed(&inner, "app-server stdout closed");
                return;
            }
            Err(error) => {
                record_diagnostic(&inner, format!("stdout read failed: {error}"));
                mark_closed(&inner, "app-server stdout read failed");
                return;
            }
        }
    }
}

pub(crate) async fn drain_stderr(inner: Arc<Inner>, stderr: AppServerOutput) {
    let mut lines = BufReader::new(stderr).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => record_diagnostic(&inner, line),
            Ok(None) => return,
            Err(error) => {
                record_diagnostic(&inner, format!("stderr read failed: {error}"));
                return;
            }
        }
    }
}

fn handle_stdout_line(inner: &Arc<Inner>, line: &str) {
    if line.trim().is_empty() {
        return;
    }
    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(error) => {
            let preview: String = line.chars().take(200).collect();
            record_diagnostic(inner, format!("non-JSON stdout ({error}): {preview}"));
            return;
        }
    };
    let message = match classify(value) {
        Ok(message) => message,
        Err(error) => {
            record_diagnostic(inner, format!("invalid JSON-RPC message: {error}"));
            return;
        }
    };
    match message {
        IncomingMessage::ServerRequest(request) => {
            let rejected_id = request.id.clone();
            if inner
                .lifecycle
                .with_open(|| record_server_request(inner, request))
                .is_err()
            {
                record_diagnostic(
                    inner,
                    format!("server request rejected after lifecycle seal: {rejected_id:?}"),
                );
            }
        }
        IncomingMessage::Response { id, result } => {
            let rejected_id = id.clone();
            match inner
                .lifecycle
                .with_open(|| take_pending_response(inner, &id))
            {
                Ok(Some(sender)) => sender.respond(result),
                Ok(None) => {
                    record_diagnostic(inner, format!("late or unknown response id: {id:?}"));
                }
                Err(_) => {
                    record_diagnostic(
                        inner,
                        format!("response rejected after lifecycle seal: {rejected_id:?}"),
                    );
                }
            }
        }
        IncomingMessage::Notification(notification) => {
            let method = notification.method.clone();
            if inner
                .lifecycle
                .with_open(|| {
                    inner
                        .state
                        .lock()
                        .expect("runtime state lock")
                        .record_notification(notification.clone());
                    let _ = inner.notifications.send(notification);
                })
                .is_err()
            {
                record_diagnostic(
                    inner,
                    format!("notification rejected after lifecycle seal: {method}"),
                );
            }
        }
        IncomingMessage::Ignored => {
            record_diagnostic(inner, "ignored non-object app-server message".to_owned());
        }
    }
}

fn record_server_request(inner: &Arc<Inner>, request: crate::ServerRequest) {
    let recorded = inner
        .state
        .lock()
        .expect("runtime state lock")
        .record_server_request(request);
    match recorded {
        Ok(ServerRequestRecordOutcome::Broadcast(canonical)) => {
            let _ = inner.server_requests.send(canonical);
        }
        Ok(ServerRequestRecordOutcome::Duplicate | ServerRequestRecordOutcome::Deferred) => {}
        Err(ServerRequestRecordError::Conflict { id }) => record_diagnostic(
            inner,
            format!("conflicting pending server request id: {id:?}; canonical payload retained"),
        ),
        Err(ServerRequestRecordError::Saturated { id }) => record_diagnostic(
            inner,
            format!(
                "server request capacity saturated at 500 unresolved occurrences; rejected id: {id:?}"
            ),
        ),
    }
}

fn take_pending_response(
    inner: &Arc<Inner>,
    id: &crate::RequestId,
) -> Option<crate::client::PendingResponse> {
    crate::client::take_pending_response(inner, id)
}

pub(crate) fn mark_closed(inner: &Arc<Inner>, reason: &str) {
    let proposed_reason = inner.lifecycle.seal_and_resolve_close_reason(reason);
    inner.closed.store(true, Ordering::Release);
    let (canonical_reason, first_close) = {
        let mut state = inner.state.lock().expect("runtime state lock");
        state.initialized = false;
        state.process_id = None;
        if let Some(canonical_reason) = state.closed_reason.as_ref() {
            (canonical_reason.clone(), false)
        } else {
            let canonical_reason = proposed_reason;
            state.closed_reason = Some(canonical_reason.clone());
            (canonical_reason, true)
        }
    };
    if !first_close {
        return;
    }
    let pending = {
        let mut entries = inner.pending.lock().expect("pending response lock");
        std::mem::take(&mut *entries)
    };
    for (_, sender) in pending {
        sender.transport_closed(canonical_reason.clone());
    }
    inner.lifecycle.publish_closed(canonical_reason);
}

fn record_diagnostic(inner: &Arc<Inner>, line: String) {
    inner
        .diagnostics
        .lock()
        .expect("diagnostic lock")
        .push(line);
}
