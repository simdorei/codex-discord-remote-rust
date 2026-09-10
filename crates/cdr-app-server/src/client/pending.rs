use std::sync::Weak;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::oneshot;
use tokio::time::sleep;

use super::{AdmissionPermit, Inner};
use crate::{RequestId, RpcErrorPayload};

pub(crate) enum PendingOutcome {
    Response(Result<Value, RpcErrorPayload>),
    TransportClosed { reason: String },
    Timeout,
}

pub(crate) struct PendingResponse {
    sender: oneshot::Sender<PendingOutcome>,
    _permit: AdmissionPermit,
}

pub(crate) fn insert(
    inner: &Inner,
    id: RequestId,
    pending: PendingResponse,
) -> Option<PendingResponse> {
    {
        let mut entries = inner.pending.lock().expect("pending response lock");
        entries.insert(id, pending)
    }
}

pub(crate) fn take(inner: &Inner, id: &RequestId) -> Option<PendingResponse> {
    {
        let mut entries = inner.pending.lock().expect("pending response lock");
        entries.remove(id)
    }
}

impl PendingResponse {
    pub(crate) fn new(permit: AdmissionPermit) -> (Self, oneshot::Receiver<PendingOutcome>) {
        let (sender, receiver) = oneshot::channel();
        (
            Self {
                sender,
                _permit: permit,
            },
            receiver,
        )
    }

    pub(crate) fn respond(self, result: Result<Value, RpcErrorPayload>) {
        let _ = self.sender.send(PendingOutcome::Response(result));
    }

    pub(crate) fn transport_closed(self, reason: String) {
        let _ = self.sender.send(PendingOutcome::TransportClosed { reason });
    }

    pub(crate) fn expire(self) {
        let _ = self.sender.send(PendingOutcome::Timeout);
    }
}

pub(crate) fn spawn_deadline(inner: Weak<Inner>, id: RequestId, wait: Duration) {
    tokio::spawn(async move {
        sleep(wait).await;
        let Some(inner) = inner.upgrade() else {
            return;
        };
        let pending = take(&inner, &id);
        if let Some(pending) = pending {
            pending.expire();
        }
    });
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use tokio::sync::{Mutex as AsyncMutex, broadcast};
    use tokio::time::advance;

    use super::{PendingOutcome, PendingResponse, insert, spawn_deadline};
    use crate::client::{AppServerClient, ClientLifecycle, Inner};
    use crate::diagnostics::BoundedDiagnostics;
    use crate::state::RuntimeState;
    use crate::{AppServerError, RequestId};

    fn test_client() -> AppServerClient {
        let (notifications, _) = broadcast::channel(1);
        let (server_requests, _) = broadcast::channel(1);
        AppServerClient {
            inner: Arc::new(Inner {
                child: AsyncMutex::new(None),
                closed: AtomicBool::new(false),
                diagnostics: Mutex::new(BoundedDiagnostics::default()),
                lifecycle: Arc::new(ClientLifecycle::new()),
                notifications,
                pending: Mutex::new(HashMap::new()),
                server_requests,
                state: Mutex::new(RuntimeState::starting(None)),
                stdin: AsyncMutex::new(None),
                write_pause: Mutex::new(None),
            }),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn deadline_releases_transport_lease_after_caller_is_gone() {
        let inner = test_client().inner;
        let id = RequestId::String("deadline-test".to_owned());
        let permit = inner.lifecycle.admit().expect("admit transport lease");
        let (pending, receiver) = PendingResponse::new(permit);
        drop(insert(&inner, id.clone(), pending));
        spawn_deadline(Arc::downgrade(&inner), id, Duration::from_millis(100));

        assert!(!inner.lifecycle.seal_if_quiescent(|| true));
        tokio::task::yield_now().await;
        advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await;

        assert!(
            inner
                .pending
                .lock()
                .expect("pending response lock")
                .is_empty()
        );
        assert!(matches!(receiver.await, Ok(PendingOutcome::Timeout)));
        assert!(inner.lifecycle.seal_if_quiescent(|| true));
    }

    #[tokio::test]
    async fn write_failure_removes_pending_before_transport_lease_drops() {
        let client = test_client();

        let error = client
            .request(
                "test/writeFailure",
                serde_json::json!({}),
                Duration::from_secs(1),
            )
            .await
            .expect_err("missing stdin must fail the write");

        assert!(matches!(error, AppServerError::Closed));
        assert!(
            client
                .inner
                .pending
                .try_lock()
                .expect("pending lock")
                .is_empty()
        );
        assert!(client.inner.lifecycle.seal_if_quiescent(|| true));
    }
}
